//! **Street-side execution capture** — the bounded log of every outbound order we sent
//! to the street, and what came back (Analytics §2.4).
//!
//! # Why this exists (and what it replaced)
//!
//! The Street-side LP league table ([`super::lp`]) grades our liquidity providers. Its
//! original seams only ever saw two things: the CLIENT-facing RFQ panels, and — later —
//! the bare fact that an auto-hedge had filled on a named LP. That second seam recorded
//! four fields (LP, instrument, notional, time) and *only for fills*, which left the
//! desk unable to answer any of the questions it actually asks about the street:
//!
//! - Orders that **backstopped to the composite** were recorded nowhere at all, so a
//!   desk whose every hedge missed the street looked identical to a desk that never
//!   hedged. The league table's zeros were truthful and completely uninformative.
//! - The LPs we **dealt away from** were discarded at the execution seam, so `Missed`
//!   could only ever be `0` for street flow, and `Mean cover` could only ever be absent.
//! - Nothing recorded the **side, the requested-vs-filled quantity, the price, the
//!   slippage, the outcome or its reason**, so a fill and a rejection were
//!   indistinguishable after the fact.
//!
//! [`StreetOrderLog`] records **one [`StreetOrder`] per outbound attempt** — fills,
//! partials, rejections, last-look pulls, composite backstops and honest misses alike —
//! carrying the full economics and the ranked panel it competed against. The league
//! table then folds it as an [`LpFlowSource`], and the `ListStreetOrders` RPC serves the
//! blotter and the per-LP / per-product / per-tenor / per-hour breakdowns off it.
//!
//! # Honesty (guardrail 2 — no fabricated metrics)
//!
//! The mapping onto per-LP league-table records lives in the pure crate
//! ([`celnet_analytics::lp_flow_records`]) and is deliberately conservative: a composite
//! backstop credits **no** LP (COMPOSITE is a synthetic mid, not a counterparty), a
//! rejection is never counted as a last-look pull, and an LP is only marked *missed*
//! when a deal was genuinely done away from it. Fields the routing seam does not
//! measure — venue round-trip latency, order type, time-in-force — stay [`None`] rather
//! than being filled with a plausible number.
//!
//! # Off the hot path (guardrail 11)
//!
//! The writer ([`StreetOrderLog::record`]) takes a short mutex on the off-core booking
//! tier (never the pinned pricing thread); the readers fold the bounded ring on the
//! async query edge. Nothing runs on the pricing core.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_analytics::{
    BreakdownDimension, LpFlowRecord, StreetBreakdownRow, StreetOrder, fold_breakdown,
    lp_flow_records,
};

use super::in_window;
use super::lp::LpFlowSource;

/// The bounded retention of recorded street orders — a rolling window of the most recent
/// outbound executions. Old orders age out: the analytics view is a recent-activity
/// picture, not an unbounded audit ledger (the immutable audit ledger is the auto-hedge
/// engine's provenance ring, which the `parent_hedge_id` links back to).
const CAPACITY: usize = 8192;

/// The hard cap on rows one `ListStreetOrders` query may return, so a query can never
/// materialise more than a bounded page regardless of the window asked for.
pub const MAX_PAGE: usize = 2_000;

/// A filter over the street-order blotter. Every field is optional; an absent field does
/// not constrain. Empty strings are treated as absent so a blank GUI control is not a
/// filter that matches nothing.
#[derive(Debug, Clone, Default)]
pub struct StreetOrderFilter {
    /// Only orders executed against this LP id.
    pub lp_id: Option<String>,
    /// Only orders in this product family token (`ois`, `bond_future`, …).
    pub family: Option<String>,
    /// Only orders on this instrument.
    pub instrument: Option<String>,
    /// Only orders with this terminal outcome token (`filled`, `rejected`, …).
    pub outcome: Option<String>,
    /// Only orders belonging to this parent hedge decision.
    pub parent_hedge_id: Option<String>,
}

impl StreetOrderFilter {
    /// Whether `order` passes every constrained field.
    fn matches(&self, order: &StreetOrder) -> bool {
        fn eq(filter: Option<&String>, value: &str) -> bool {
            filter.is_none_or(|f| f.is_empty() || f == value)
        }
        fn eq_opt(filter: Option<&String>, value: Option<&str>) -> bool {
            filter.is_none_or(|f| f.is_empty() || value == Some(f.as_str()))
        }
        eq_opt(self.lp_id.as_ref(), order.lp_id.as_deref())
            && eq(self.family.as_ref(), &order.family)
            && eq(self.instrument.as_ref(), &order.instrument)
            && eq(self.outcome.as_ref(), order.outcome.label())
            && eq_opt(
                self.parent_hedge_id.as_ref(),
                order.parent_hedge_id.as_deref(),
            )
    }
}

/// A shared, bounded log of outbound street orders. Written by the rates booking /
/// hedge-execution seams off-core; read on the analytics query edge and folded by the
/// Street-side LP rollup as an [`LpFlowSource`].
#[derive(Debug)]
pub struct StreetOrderLog {
    ring: Mutex<VecDeque<StreetOrder>>,
    /// The monotonic source of `order_id`s. Starts at 1 so `SO-0` is never minted.
    next_id: AtomicU64,
}

impl Default for StreetOrderLog {
    fn default() -> Self {
        Self::new()
    }
}

impl StreetOrderLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ring: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Mint the next street-order id. Process-local and monotonic, so a blotter row is
    /// stably identifiable for the lifetime of the ring.
    #[must_use]
    pub fn next_order_id(&self) -> String {
        format!("SO-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Record one outbound street order. Bounded: the oldest ages out at [`CAPACITY`].
    /// Off the pinned pricing core — a short mutex on the booking tier.
    ///
    /// Non-finite / negative quantities are clamped to `0.0` (an order with no honest
    /// quantity is still a real order that went out, so it is kept rather than dropped —
    /// but it never contributes a fabricated magnitude).
    pub fn record(&self, mut order: StreetOrder) {
        let clamp = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        order.requested_qty = clamp(order.requested_qty);
        order.filled_qty = clamp(order.filled_qty);
        order.filled_price = order.filled_price.filter(|p| p.is_finite());
        order.slippage_bp = order.slippage_bp.filter(|s| s.is_finite());
        let mut g = self.ring.lock().expect("street order log lock poisoned");
        if g.len() >= CAPACITY {
            g.pop_front();
        }
        g.push_back(order);
    }

    /// The orders in `[from, to)` matching `filter`, **newest first**, capped at
    /// `limit` (itself capped at [`MAX_PAGE`]).
    #[must_use]
    pub fn orders(
        &self,
        from: Option<i64>,
        to: Option<i64>,
        filter: &StreetOrderFilter,
        limit: usize,
    ) -> Vec<StreetOrder> {
        let cap = limit.clamp(1, MAX_PAGE);
        let g = self.ring.lock().expect("street order log lock poisoned");
        g.iter()
            .rev()
            .filter(|o| in_window(o.ts_nanos, from, to))
            .filter(|o| filter.matches(o))
            .take(cap)
            .cloned()
            .collect()
    }

    /// The aggregated breakdown over the window + filter on `dimension`. Folded over the
    /// **whole** matching window (not the returned page), so a breakdown never reflects
    /// only the first page of a blotter.
    #[must_use]
    pub fn breakdown(
        &self,
        from: Option<i64>,
        to: Option<i64>,
        filter: &StreetOrderFilter,
        dimension: BreakdownDimension,
    ) -> Vec<StreetBreakdownRow> {
        let matching: Vec<StreetOrder> = {
            let g = self.ring.lock().expect("street order log lock poisoned");
            g.iter()
                .filter(|o| in_window(o.ts_nanos, from, to))
                .filter(|o| filter.matches(o))
                .cloned()
                .collect()
        };
        fold_breakdown(&matching, dimension)
    }

    /// The number of orders currently retained (diagnostics / tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.ring
            .lock()
            .expect("street order log lock poisoned")
            .len()
    }

    /// Whether the ring is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// Wire projection
// ---------------------------------------------------------------------------

/// Project one pure [`StreetOrder`] onto its wire mirror.
///
/// Every `Option` maps 1:1 onto a proto3 `optional` (absent = JSON `null` on the WS
/// wire), so a datum we did not observe stays absent all the way to the screen and is
/// rendered as an explicit marker rather than a fabricated zero (guardrail 2).
#[must_use]
pub fn order_to_wire(o: &StreetOrder) -> celnet_proto::StreetOrderDesc {
    celnet_proto::StreetOrderDesc {
        order_id: o.order_id.clone(),
        ts_nanos: o.ts_nanos,
        lp_id: o.lp_id.clone(),
        venue: o.venue.label().to_owned(),
        instrument: o.instrument.clone(),
        family: o.family.clone(),
        tenor_years: o.tenor_years,
        side: o.side.label().to_owned(),
        requested_qty: o.requested_qty,
        filled_qty: o.filled_qty,
        requested_price: o.requested_price,
        filled_price: o.filled_price,
        slippage_bp: o.slippage_bp,
        outcome: o.outcome.label().to_owned(),
        reason: o.reason.clone(),
        competitors: o
            .competitors
            .iter()
            .map(|c| celnet_proto::StreetCompetitorDesc {
                lp_id: c.lp_id.clone(),
                price: c.price,
            })
            .collect(),
        parent_hedge_id: o.parent_hedge_id.clone(),
        parent_position_id: o.parent_position_id,
        order_type: o.order_type.clone(),
        time_in_force: o.time_in_force.clone(),
        response_latency_nanos: o.response_latency_nanos,
    }
}

/// Project one pure [`StreetBreakdownRow`] onto its wire mirror.
#[must_use]
pub fn breakdown_row_to_wire(r: &StreetBreakdownRow) -> celnet_proto::StreetBreakdownRowDesc {
    celnet_proto::StreetBreakdownRowDesc {
        dimension: r.dimension.label().to_owned(),
        key: r.key.clone(),
        orders: r.orders,
        filled: r.filled,
        partially_filled: r.partially_filled,
        rejected: r.rejected,
        cancelled: r.cancelled,
        expired: r.expired,
        last_look_pulled: r.last_look_pulled,
        no_liquidity: r.no_liquidity,
        composite_backstop: r.composite_backstop,
        requested_qty: r.requested_qty,
        filled_qty: r.filled_qty,
        fill_ratio: r.fill_ratio,
        win_rate: r.win_rate,
        mean_slippage_bp: r.mean_slippage_bp,
        mean_response_latency_nanos: r.mean_response_latency_nanos,
        last_look_rate: r.last_look_rate,
        mean_cover: r.mean_cover,
        mean_competitors: r.mean_competitors,
    }
}

#[tonic::async_trait]
impl LpFlowSource for StreetOrderLog {
    /// Project each recorded order in `[from, to)` onto the per-LP league-table records
    /// it honestly implies — see [`celnet_analytics::lp_flow_records`] for the exact
    /// (deliberately conservative) mapping.
    async fn lp_flow_records(&self, from: Option<i64>, to: Option<i64>) -> Vec<LpFlowRecord> {
        let g = self.ring.lock().expect("street order log lock poisoned");
        g.iter()
            .filter(|o| in_window(o.ts_nanos, from, to))
            .flat_map(lp_flow_records)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_analytics::{StreetCompetitor, StreetOutcome, StreetSide, StreetVenue};

    fn competitor(lp: &str, price: f64) -> StreetCompetitor {
        StreetCompetitor {
            lp_id: lp.to_owned(),
            price,
        }
    }

    /// A named-LP fill against a three-deep panel.
    fn fill(id: &str, lp: &str, qty: f64, ts: i64) -> StreetOrder {
        StreetOrder {
            lp_id: Some(lp.to_owned()),
            venue: StreetVenue::NamedLp,
            family: "ois".to_owned(),
            tenor_years: Some(10.0),
            filled_qty: qty,
            filled_price: Some(0.03005),
            slippage_bp: Some(0.5),
            outcome: StreetOutcome::Filled,
            competitors: vec![
                competitor(lp, 0.03005),
                competitor("LP-B", 0.03007),
                competitor("LP-C", 0.03011),
            ],
            parent_hedge_id: Some("HDG-1".to_owned()),
            parent_position_id: Some(42),
            ..StreetOrder::new(id, ts, "USSW10", StreetSide::Buy, qty, 0.03)
        }
    }

    #[tokio::test]
    async fn a_routed_order_attributes_the_win_and_the_misses_per_lp() {
        let log = StreetOrderLog::new();
        log.record(fill("SO-1", "LP-A", 5_000.0, 1_000));

        let recs = log.lp_flow_records(None, None).await;
        assert_eq!(recs.len(), 3);
        let a = recs.iter().find(|r| r.lp_id == "LP-A").expect("LP-A");
        assert!(a.was_won && a.was_quoted);
        assert_eq!(a.notional, 5_000.0);
        let b = recs.iter().find(|r| r.lp_id == "LP-B").expect("LP-B");
        assert!(b.was_missed, "we dealt away from it — that is a real miss");
        assert!(b.cover_distance.is_some(), "the cover carries a distance");
    }

    #[tokio::test]
    async fn a_composite_backstop_is_recorded_but_credits_no_lp() {
        let log = StreetOrderLog::new();
        log.record(StreetOrder {
            venue: StreetVenue::CompositeBackstop,
            outcome: StreetOutcome::Filled,
            filled_qty: 4_000.0,
            reason: Some("no_firm_lp_price".to_owned()),
            ..StreetOrder::new("SO-2", 1_000, "USSW10", StreetSide::Sell, 4_000.0, 0.03)
        });

        // The ORDER exists — the desk can see the street was missed …
        assert_eq!(
            log.orders(None, None, &StreetOrderFilter::default(), 50)
                .len(),
            1
        );
        // … but no LP is credited for it.
        assert!(log.lp_flow_records(None, None).await.is_empty());
        let rows = log.breakdown(
            None,
            None,
            &StreetOrderFilter::default(),
            BreakdownDimension::Lp,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, celnet_analytics::COMPOSITE_KEY);
        assert_eq!(rows[0].composite_backstop, 1);
    }

    #[tokio::test]
    async fn a_partial_fill_and_a_rejection_are_distinct_outcomes() {
        let log = StreetOrderLog::new();
        log.record(StreetOrder {
            outcome: StreetOutcome::PartiallyFilled,
            requested_qty: 10_000.0,
            filled_qty: 2_500.0,
            ..fill("SO-1", "LP-A", 2_500.0, 1_000)
        });
        log.record(StreetOrder {
            outcome: StreetOutcome::Rejected,
            requested_qty: 10_000.0,
            filled_qty: 0.0,
            filled_price: None,
            slippage_bp: None,
            reason: Some("venue_rejected".to_owned()),
            ..fill("SO-2", "LP-A", 0.0, 2_000)
        });

        let rows = log.breakdown(
            None,
            None,
            &StreetOrderFilter::default(),
            BreakdownDimension::Lp,
        );
        let a = rows.iter().find(|r| r.key == "LP-A").expect("LP-A");
        assert_eq!(a.partially_filled, 1);
        assert_eq!(a.rejected, 1);
        assert_eq!(a.filled, 0, "a partial is not a full fill");
        assert_eq!(a.requested_qty, 20_000.0);
        assert_eq!(a.filled_qty, 2_500.0);
        assert_eq!(a.fill_ratio, Some(0.125));
        assert_eq!(a.win_rate, Some(0.5), "one of two orders traded");
        assert_eq!(
            a.mean_response_latency_nanos, None,
            "no measured round trip ⇒ ABSENT, never 0"
        );
    }

    #[test]
    fn the_window_and_filters_select() {
        let log = StreetOrderLog::new();
        log.record(fill("SO-1", "LP-A", 1.0, 100));
        log.record(fill("SO-2", "LP-Z", 1.0, 500));

        // Half-open [200, 600).
        let win = log.orders(Some(200), Some(600), &StreetOrderFilter::default(), 50);
        assert_eq!(win.len(), 1);
        assert_eq!(win[0].order_id, "SO-2");

        let by_lp = log.orders(
            None,
            None,
            &StreetOrderFilter {
                lp_id: Some("LP-A".to_owned()),
                ..StreetOrderFilter::default()
            },
            50,
        );
        assert_eq!(by_lp.len(), 1);
        assert_eq!(by_lp[0].lp_id.as_deref(), Some("LP-A"));

        // An EMPTY filter string does not constrain (a blank GUI control is not a
        // filter that matches nothing).
        assert_eq!(
            log.orders(
                None,
                None,
                &StreetOrderFilter {
                    lp_id: Some(String::new()),
                    ..StreetOrderFilter::default()
                },
                50,
            )
            .len(),
            2
        );
    }

    #[test]
    fn orders_come_back_newest_first_and_are_page_capped() {
        let log = StreetOrderLog::new();
        for i in 1..=5 {
            log.record(fill(&format!("SO-{i}"), "LP-A", 1.0, i64::from(i)));
        }
        let page = log.orders(None, None, &StreetOrderFilter::default(), 2);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].order_id, "SO-5", "newest first");
        assert_eq!(page[1].order_id, "SO-4");
        // A caller asking for more than the hard cap gets the cap, not the ask.
        assert!(
            log.orders(None, None, &StreetOrderFilter::default(), usize::MAX)
                .len()
                <= MAX_PAGE
        );
    }

    #[test]
    fn the_ring_is_bounded_and_ids_are_monotonic() {
        let log = StreetOrderLog::new();
        for i in 0..(CAPACITY + 16) {
            log.record(fill("SO-x", "LP-A", 1.0, i as i64));
        }
        assert_eq!(log.len(), CAPACITY);
        assert_eq!(log.next_order_id(), "SO-1");
        assert_eq!(log.next_order_id(), "SO-2");
    }

    #[test]
    fn a_non_finite_quantity_is_clamped_never_recorded_as_a_magnitude() {
        let log = StreetOrderLog::new();
        log.record(StreetOrder {
            filled_qty: f64::NAN,
            requested_qty: -50.0,
            slippage_bp: Some(f64::INFINITY),
            ..fill("SO-1", "LP-A", 0.0, 1)
        });
        let got = log.orders(None, None, &StreetOrderFilter::default(), 10);
        assert_eq!(got[0].filled_qty, 0.0);
        assert_eq!(got[0].requested_qty, 0.0);
        assert_eq!(got[0].slippage_bp, None, "a non-finite slippage is ABSENT");
    }

    #[test]
    fn the_breakdown_folds_the_whole_window_not_just_a_page() {
        let log = StreetOrderLog::new();
        for i in 1..=10 {
            log.record(fill("SO-x", "LP-A", 1.0, i64::from(i)));
        }
        let rows = log.breakdown(
            None,
            None,
            &StreetOrderFilter::default(),
            BreakdownDimension::Lp,
        );
        let a = rows.iter().find(|r| r.key == "LP-A").expect("LP-A");
        assert_eq!(a.orders, 10);
    }
}
