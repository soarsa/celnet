//! **Street-side / LP liquidity analytics** rollup (server side — the LP-keyed
//! analogue of the client-flow rollup, `docs/ANALYTICS-REQUIREMENTS.md` §2.4).
//!
//! Where [`super`]'s client-flow fold grades *our clients'* flow, this fold grades
//! *our liquidity providers'* street-side behaviour: per-LP **tick rate**,
//! **deals won** (+ won notional), **missed deals** (on the panel but lost),
//! **last-look rejects**, **win-rate**, and mean **cover distance**. It owns no
//! market model and re-prices nothing — each *source* maps its own already-captured
//! history into neutral [`LpFlowRecord`](celnet_analytics::LpFlowRecord)s and this
//! module folds the union into per-LP
//! [`LpFlowMetrics`](celnet_analytics::LpFlowMetrics).
//!
//! # The three real data seams (guardrail 2 — no fabrication)
//!
//! - **Tick rate** — the aggregation hub's bounded per-LP ingest tally
//!   ([`AggregationHub::lp_tick_counts`](crate::services::aggregation::AggregationHub::lp_tick_counts)),
//!   merged into the fold via [`celnet_analytics::merge_tick_counts`]. The sink only
//!   keeps the latest quote per venue, so the count is recorded at the ingest seam
//!   (a bounded off-core counter, documented there), never streamed per update.
//! - **Won / missed** — the RFQ panel rows pinned on each `QuoteRecord`
//!   (`dealers` + `booked_lp_id`): every panel dealer was quoted, the booked line
//!   won, the rest missed on a booked RFQ.
//! - **Last-look rejects** — the multi-dealer engine's
//!   [`RankedPanel::last_look_rejected`](celnet_rfq::RankedPanel) set, pinned onto
//!   the `QuoteRecord` at panel time.
//!
//! # Off the hot path (guardrail 11)
//!
//! Collection is fold-on-query: a source's [`LpFlowSource::lp_flow_records`] reads
//! its existing history store on the async query edge (never a price call), and the
//! tick tally is a lock-guarded snapshot of a counter the off-core feed edge
//! maintains. Nothing runs on the pinned pricing thread.

use std::collections::BTreeMap;
use std::sync::Arc;

use celnet_analytics::{LpFlowMetrics, LpFlowRecord, group_by_lp, merge_tick_counts};
use celnet_proto::LpFlowMetricsDesc;

/// A provider of already-captured street-side LP activity the rollup folds over.
/// Object-safe (via `tonic::async_trait`) so the server holds a heterogeneous
/// `Vec<Arc<dyn LpFlowSource>>` — the RFQ quote edge (panel outcomes) + the
/// aggregation hub (tick tally).
#[tonic::async_trait]
pub trait LpFlowSource: Send + Sync {
    /// This source's panel-outcome [`LpFlowRecord`]s whose event time falls in
    /// `[from, to)` (each bound optional/open). Called on-query, off the hot path.
    /// A pure tick source returns an empty vec (its data arrives via
    /// [`Self::tick_counts`]).
    async fn lp_flow_records(&self, from: Option<i64>, to: Option<i64>) -> Vec<LpFlowRecord>;

    /// This source's bounded per-LP quote-update tick tally (`lp_id` → count).
    /// Defaults to empty — only the aggregation hub (the tick source) overrides it.
    /// A monotonic lifetime count (not window-filtered): the ingest seam records a
    /// count, not per-tick timestamps, so it cannot be sliced by time — documented
    /// at the counter.
    fn tick_counts(&self) -> BTreeMap<String, u64> {
        BTreeMap::new()
    }
}

/// Collect + union every source's window-filtered panel records **and** sum their
/// tick tallies (async — awaits each source's store). Kept separate from
/// [`fold`] so the fold stays a pure, synchronously-testable function.
pub async fn collect(
    sources: &[Arc<dyn LpFlowSource>],
    from: Option<i64>,
    to: Option<i64>,
) -> (Vec<LpFlowRecord>, BTreeMap<String, u64>) {
    let mut records: Vec<LpFlowRecord> = Vec::new();
    let mut ticks: BTreeMap<String, u64> = BTreeMap::new();
    for source in sources {
        records.extend(source.lp_flow_records(from, to).await);
        for (lp, n) in source.tick_counts() {
            *ticks.entry(lp).or_insert(0) += n;
        }
    }
    (records, ticks)
}

/// Fold a record union + tick tally into per-LP metrics, optionally filtered to a
/// single `lp_id`. **Pure** and deterministic (`BTreeMap` key order) — the whole
/// numerical surface lives in the oracle-tested crate; this only merges the tick
/// tally and applies the optional filter. An empty/`None` filter returns every LP.
#[must_use]
pub fn fold(
    records: &[LpFlowRecord],
    ticks: &BTreeMap<String, u64>,
    lp_filter: Option<&str>,
) -> Vec<LpFlowMetricsDesc> {
    let mut metrics = group_by_lp(records);
    merge_tick_counts(&mut metrics, ticks);
    let filter = lp_filter.filter(|f| !f.is_empty());
    metrics
        .values()
        .filter(|m| filter.is_none_or(|f| m.lp_id == f))
        .map(metrics_to_wire)
        .collect()
}

/// Project one pure [`LpFlowMetrics`] onto its wire mirror. `Option<f64>` ⇒ proto3
/// `optional double` (absent = JSON null on the WS wire — the divide-by-zero guard).
#[must_use]
pub fn metrics_to_wire(m: &LpFlowMetrics) -> LpFlowMetricsDesc {
    LpFlowMetricsDesc {
        lp_id: m.lp_id.clone(),
        tick_count: m.tick_count,
        quote_count: m.quote_count,
        deals_won: m.deals_won,
        won_notional: m.won_notional,
        missed: m.missed,
        last_look_rejects: m.last_look_rejects,
        win_rate: m.win_rate,
        mean_cover: m.mean_cover,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A won panel row for `lp` (a win implies it was quoted).
    fn won(lp: &str, notional: f64) -> LpFlowRecord {
        LpFlowRecord {
            was_quoted: true,
            was_won: true,
            notional,
            ..LpFlowRecord::blank(lp, "EURUSD")
        }
    }

    /// A quoted-but-lost panel row.
    fn missed(lp: &str) -> LpFlowRecord {
        LpFlowRecord {
            was_quoted: true,
            was_missed: true,
            ..LpFlowRecord::blank(lp, "EURUSD")
        }
    }

    /// A last-look rejection (the LP responded, then its quote lapsed at ranking).
    fn last_look(lp: &str) -> LpFlowRecord {
        LpFlowRecord {
            was_quoted: true,
            was_last_look_reject: true,
            ..LpFlowRecord::blank(lp, "EURUSD")
        }
    }

    #[test]
    fn fold_merges_ticks_and_grades_lps() {
        // A known set of LP outcomes folds to the expected metrics (validated
        // against the same crate fold the oracle pins): a WINNER LP_A (won 2 of 2 ⇒
        // win_rate 1), a FISHER-panel LP_B (quoted 3, won 0 ⇒ misses, win_rate 0), a
        // LAST-LOOK-REJECTER LP_C (2 firm quotes both reneged), and a pure tick
        // stream LP_Z (7 ticks, no panel). Plus 40 ingest ticks on LP_A.
        let recs = vec![
            won("LP_A", 5_000_000.0),
            won("LP_A", 5_000_000.0),
            missed("LP_B"),
            missed("LP_B"),
            missed("LP_B"),
            last_look("LP_C"),
            last_look("LP_C"),
        ];
        let mut ticks: BTreeMap<String, u64> = BTreeMap::new();
        ticks.insert("LP_A".to_owned(), 40);
        ticks.insert("LP_Z".to_owned(), 7);

        let rows = fold(&recs, &ticks, None);
        // Ordered by lp_id: LP_A, LP_B, LP_C, LP_Z.
        assert_eq!(rows.len(), 4);
        // Winner: high win-rate + deals + notional.
        assert_eq!(rows[0].lp_id, "LP_A");
        assert_eq!(rows[0].deals_won, 2);
        assert_eq!(rows[0].won_notional, 10_000_000.0);
        assert_eq!(rows[0].tick_count, 40);
        assert_eq!(rows[0].win_rate, Some(1.0));
        // Fisher-panel: on the panel 3× but never won ⇒ win_rate 0, no ticks.
        assert_eq!(rows[1].lp_id, "LP_B");
        assert_eq!(rows[1].missed, 3);
        assert_eq!(rows[1].win_rate, Some(0.0));
        assert_eq!(rows[1].tick_count, 0);
        // Last-look rejecter: 2 firm quotes, both reneged ⇒ rejects counted.
        assert_eq!(rows[2].lp_id, "LP_C");
        assert_eq!(rows[2].last_look_rejects, 2);
        assert_eq!(rows[2].quote_count, 2);
        assert_eq!(rows[2].deals_won, 0);
        assert_eq!(rows[2].win_rate, Some(0.0));
        // Tick-only stream: no quotes, win_rate ABSENT (None → JSON null).
        assert_eq!(rows[3].lp_id, "LP_Z");
        assert_eq!(rows[3].tick_count, 7);
        assert_eq!(rows[3].quote_count, 0);
        assert_eq!(rows[3].win_rate, None);
    }

    #[test]
    fn fold_lp_filter_selects_one() {
        let recs = vec![won("LP_A", 1_000_000.0), missed("LP_B")];
        let ticks = BTreeMap::new();
        let only_b = fold(&recs, &ticks, Some("LP_B"));
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].lp_id, "LP_B");
        // Empty filter string ⇒ no filter (all rows).
        assert_eq!(fold(&recs, &ticks, Some("")).len(), 2);
    }
}
