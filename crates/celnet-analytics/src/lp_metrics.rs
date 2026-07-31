//! The per-LP rollup: a pure fold of `&LpFlowRecord`s into [`LpFlowMetrics`].
//!
//! Every field is a deterministic function of the input records — no clock, no
//! rng, no I/O — so the whole rollup is oracle-testable against a hand-computed
//! truth table (guardrail 5). Divide-by-zero is guarded: a ratio with a zero
//! denominator is reported as `None`, never `NaN`/`inf`.

use std::collections::BTreeMap;

use crate::lp_record::LpFlowRecord;

/// Per-LP street-side liquidity metrics — the output of the fold.
///
/// Counts are exact; the `win_rate` / `mean_cover` ratios are `None` when their
/// denominator is zero. `won_notional` is the summed magnitude of the deals the LP
/// won.
#[derive(Debug, Clone, PartialEq)]
pub struct LpFlowMetrics {
    /// The LP this metric block belongs to (the rollup key).
    pub lp_id: String,

    // ── activity ────────────────────────────────────────────────────────────
    /// Number of quote-update ticks observed from this LP (its update frequency
    /// over the folded window — the tick-rate numerator).
    pub tick_count: u64,
    /// Number of panel responses from this LP (Σ `was_quoted`). Doubles as the
    /// **response-presence** count and the `win_rate` denominator.
    pub quote_count: u64,

    // ── competition outcome ─────────────────────────────────────────────────
    /// Deals this LP won (Σ `was_won`).
    pub deals_won: u64,
    /// Summed notional magnitude of the deals this LP won.
    pub won_notional: f64,
    /// Panel appearances where the LP was quoted but a deal went elsewhere
    /// (Σ `was_missed`).
    pub missed: u64,
    /// Times this LP's quote was rejected on last-look (Σ `was_last_look_reject`).
    pub last_look_rejects: u64,

    // ── derived ratios (divide-by-zero guarded) ─────────────────────────────
    /// Win-rate = `deals_won / quote_count`. `None` when the LP made no quotes.
    /// In `[0, 1]` for a well-formed input (a win implies a quote).
    pub win_rate: Option<f64>,
    /// Mean of the present `cover_distance` inputs (how far this LP was from the
    /// winner when it was the cover). `None` when the LP was never the cover.
    pub mean_cover: Option<f64>,
}

impl LpFlowMetrics {
    /// A tick-only metrics block: an LP for which the only retained datum is its
    /// ingest tick count (it appeared on no panel in the window). Every
    /// competition field is zero/`None` — an honest "ticked but no deals seen",
    /// never a fabricated outcome.
    #[must_use]
    pub fn tick_only(lp_id: impl Into<String>, tick_count: u64) -> Self {
        Self {
            lp_id: lp_id.into(),
            tick_count,
            quote_count: 0,
            deals_won: 0,
            won_notional: 0.0,
            missed: 0,
            last_look_rejects: 0,
            win_rate: None,
            mean_cover: None,
        }
    }
}

/// Single-pass accumulator over the records for one LP.
#[derive(Debug, Clone, Copy, Default)]
struct Acc {
    tick_count: u64,
    quote_count: u64,
    deals_won: u64,
    won_notional: f64,
    missed: u64,
    last_look_rejects: u64,
    cover_sum: f64,
    cover_n: u64,
}

impl Acc {
    fn ingest(&mut self, r: &LpFlowRecord) {
        if r.tick {
            self.tick_count += 1;
        }
        if r.was_quoted {
            self.quote_count += 1;
        }
        if r.was_won {
            self.deals_won += 1;
            self.won_notional += r.notional;
        }
        if r.was_missed {
            self.missed += 1;
        }
        if r.was_last_look_reject {
            self.last_look_rejects += 1;
        }
        if let Some(cd) = r.cover_distance {
            self.cover_sum += cd;
            self.cover_n += 1;
        }
    }

    fn finish(self, lp_id: String) -> LpFlowMetrics {
        let win_rate =
            (self.quote_count > 0).then(|| self.deals_won as f64 / self.quote_count as f64);
        let mean_cover = (self.cover_n > 0).then(|| self.cover_sum / self.cover_n as f64);
        LpFlowMetrics {
            lp_id,
            tick_count: self.tick_count,
            quote_count: self.quote_count,
            deals_won: self.deals_won,
            won_notional: self.won_notional,
            missed: self.missed,
            last_look_rejects: self.last_look_rejects,
            win_rate,
            mean_cover,
        }
    }
}

/// Fold an iterator of records (assumed to belong to one `lp_id`) into an
/// [`LpFlowMetrics`]. Pure and O(records) with bounded memory.
pub fn lp_metrics_from<'a, I>(lp_id: impl Into<String>, records: I) -> LpFlowMetrics
where
    I: IntoIterator<Item = &'a LpFlowRecord>,
{
    let mut acc = Acc::default();
    for r in records {
        acc.ingest(r);
    }
    acc.finish(lp_id.into())
}

/// Fold a bounded per-LP tick tally — the server's off-core aggregation-ingest
/// counter (§ street-liquidity, `docs/ANALYTICS-REQUIREMENTS.md` §2.4) — into an
/// existing metrics map.
///
/// The aggregation hub keeps only a **monotonic count** of pushes per LP (it never
/// retains every tick — the sink overwrites the latest quote per venue), so tick
/// rate cannot be streamed as one record per update at IB scale. This merge adds
/// each LP's counted ticks onto its `tick_count`, creating a
/// [`LpFlowMetrics::tick_only`] row for an LP that had ticks but appeared on no
/// panel in the window. Pure and deterministic (`BTreeMap` key order) — the same
/// numerical surface as [`lp_metrics_from`], oracle-tested against a hand tally.
pub fn merge_tick_counts(
    metrics: &mut BTreeMap<String, LpFlowMetrics>,
    ticks: &BTreeMap<String, u64>,
) {
    for (lp_id, &count) in ticks {
        metrics
            .entry(lp_id.clone())
            .and_modify(|m| m.tick_count += count)
            .or_insert_with(|| LpFlowMetrics::tick_only(lp_id.clone(), count));
    }
}
