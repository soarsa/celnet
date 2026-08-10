//! **Hedge-execution → street-side LP attribution** (Analytics §2.4).
//!
//! The Street-side LP league table ([`super::lp`]) grades our liquidity providers'
//! behaviour. Its panel-outcome seam ([`crate::services::quote::quote_record_to_lp_flows`])
//! only mines the CLIENT-facing RFQ panels — so an LP that never wins a client RFQ but
//! consistently fills our OUTBOUND hedges shows `Deals won 0 · Won $0`, hiding real
//! street-side flow.
//!
//! This module closes that gap: when the auto-hedge executor fills a shed against a NAMED
//! LP (`venue = Lp`), the rates booking seam records that fill here. The rollup then folds
//! this log as an [`LpFlowSource`], attributing a **WON deal + won notional** to the
//! executing LP — updating its Deals won / Won notional / Win rate.
//!
//! # Honest mapping (guardrail 2 — no fabrication)
//!
//! A hedge fill is not a competitive multi-dealer panel: we lifted the single best
//! executable LP price on the required side. So a recorded fill carries ONLY the winner's
//! outcome (`was_quoted` + `was_won` + `notional`). We do NOT synthesise `was_missed`
//! rows for the LPs we did not lift (they were not in a formal RFQ we ran), and a hedge
//! fill has no `last_look` or `cover` we can honestly attach — those stay absent, never a
//! fabricated zero. Composite-venue fills are never recorded here (COMPOSITE is a
//! pseudo-venue, not a street LP), so only real named-LP fills reach the league table.
//!
//! # Off the hot path (guardrail 11)
//!
//! The writer ([`HedgeFillFlowLog::record_fill`]) takes a short mutex on the off-core
//! booking tier (never the pinned pricing thread); the reader ([`LpFlowSource`]) folds the
//! bounded ring on the async query edge. Nothing runs on the pricing core.

use std::collections::VecDeque;
use std::sync::Mutex;

use celnet_analytics::LpFlowRecord;

use super::in_window;
use super::lp::LpFlowSource;

/// The bounded retention of recorded hedge fills — a rolling window of the most recent
/// street-side hedge executions the league table folds. Old fills age out (the analytics
/// view is a recent-activity picture, not an unbounded audit ledger — the immutable audit
/// ledger is the auto-hedge engine's provenance ring).
const CAPACITY: usize = 8192;

/// One recorded hedge execution that filled against a named street LP.
#[derive(Debug, Clone, PartialEq)]
struct HedgeFill {
    /// The LP the shed filled on (its `lp_id` on the league table).
    lp_id: String,
    /// The instrument hedged (symbol / ISIN) — context, not itself a metric.
    instrument: String,
    /// The non-negative filled notional (the hedge's external metric units) — sums into
    /// the LP's won notional.
    notional: f64,
    /// When the fill fired (epoch nanos, UTC) — for the `[from, to)` window filter.
    ts_nanos: i64,
}

/// A shared, bounded log of hedge executions that filled against named LPs, folded by the
/// Street-side LP rollup as an [`LpFlowSource`]. Written by the rates booking seam
/// ([`crate::services::rates_book::RatesPositionStore`]) off-core; read on the analytics
/// query edge.
#[derive(Debug)]
pub struct HedgeFillFlowLog {
    ring: Mutex<VecDeque<HedgeFill>>,
}

impl Default for HedgeFillFlowLog {
    fn default() -> Self {
        Self::new()
    }
}

impl HedgeFillFlowLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ring: Mutex::new(VecDeque::new()),
        }
    }

    /// Record one hedge fill against a named LP (a WON street-side deal). Bounded: the
    /// oldest fill is evicted at [`CAPACITY`]. A non-positive / non-finite notional is
    /// clamped to `0.0` (a fill with no honest notional still counts as a won deal, never a
    /// fabricated magnitude). Off the pinned pricing core — a short mutex on the booking tier.
    pub fn record_fill(
        &self,
        lp_id: impl Into<String>,
        instrument: impl Into<String>,
        notional: f64,
        ts_nanos: i64,
    ) {
        let notional = if notional.is_finite() {
            notional.max(0.0)
        } else {
            0.0
        };
        let mut g = self.ring.lock().expect("hedge flow log lock poisoned");
        if g.len() >= CAPACITY {
            g.pop_front();
        }
        g.push_back(HedgeFill {
            lp_id: lp_id.into(),
            instrument: instrument.into(),
            notional,
            ts_nanos,
        });
    }
}

#[tonic::async_trait]
impl LpFlowSource for HedgeFillFlowLog {
    /// Each recorded fill in `[from, to)` becomes one WON panel-outcome record for its LP:
    /// `was_quoted` (a win implies a quote) + `was_won` + the filled `notional`. No
    /// `was_missed` / `was_last_look_reject` / `cover_distance` — a hedge fill has no honest
    /// value for those (guardrail 2).
    async fn lp_flow_records(&self, from: Option<i64>, to: Option<i64>) -> Vec<LpFlowRecord> {
        let g = self.ring.lock().expect("hedge flow log lock poisoned");
        g.iter()
            .filter(|f| in_window(f.ts_nanos, from, to))
            .map(|f| LpFlowRecord {
                was_quoted: true,
                was_won: true,
                notional: f.notional,
                ..LpFlowRecord::blank(f.lp_id.clone(), f.instrument.clone())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_a_named_lp_fill_as_a_won_deal() {
        let log = HedgeFillFlowLog::new();
        log.record_fill("LP-SIM-01", "USSW10", 5_000.0, 1_000);
        log.record_fill("LP-SIM-03", "USSW10", 3_000.0, 2_000);

        let recs = log.lp_flow_records(None, None).await;
        assert_eq!(recs.len(), 2);
        let one = recs.iter().find(|r| r.lp_id == "LP-SIM-01").expect("lp-01");
        assert!(one.was_quoted && one.was_won, "a fill is a quoted win");
        assert!(!one.was_missed && !one.was_last_look_reject);
        assert_eq!(one.notional, 5_000.0);
        assert_eq!(one.cover_distance, None, "no cover on a hedge fill");
    }

    #[tokio::test]
    async fn window_filters_by_fire_time() {
        let log = HedgeFillFlowLog::new();
        log.record_fill("LP-SIM-01", "USSW10", 1.0, 100);
        log.record_fill("LP-SIM-01", "USSW10", 1.0, 500);
        // Half-open [200, 600): only the ts=500 fill qualifies.
        let recs = log.lp_flow_records(Some(200), Some(600)).await;
        assert_eq!(recs.len(), 1);
    }

    #[test]
    fn non_finite_notional_is_clamped_to_zero() {
        let log = HedgeFillFlowLog::new();
        log.record_fill("LP-SIM-01", "USSW10", f64::NAN, 1);
        log.record_fill("LP-SIM-01", "USSW10", -50.0, 2);
        let g = log.ring.lock().unwrap();
        assert!(g.iter().all(|f| f.notional == 0.0));
    }
}
