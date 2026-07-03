//! The server-side **linear-rates position book** the dealer-quoting feature books
//! into and the `RiskService` Book/List reads serve.
//!
//! # Why a dedicated store (not the FX [`PositionStore`])
//!
//! The FX [`PositionStore`](super::risk::store::PositionStore) warehouses
//! convention-baked vanilla/exotic [`RiskFact`](celnet_risk_cube::RiskFact)s under a
//! full org [`Hierarchy`](celnet_risk_cube::Hierarchy) and an
//! [`Underlying`](celnet_types::Underlying) axis. A linear-rates position
//! ([`RatesPosition`]) is a far leaner fact: a `(entity, book)` cell plus a
//! `RatesInstrument` priced against a request-supplied `CurveSet`. It has **no**
//! `Underlying`, no Book→Desk hierarchy, and no marking surface — so forcing it
//! through the FX fact key (which *requires* an `Underlying`) would be dishonest.
//! This module is the rates analogue of that store: in-memory, [`RwLock`]-guarded,
//! RAII-clean, with a deterministic monotonic id counter, and an entitlement
//! predicate scoped to the two org axes a rates cell actually carries.
//!
//! The same store instance is shared (behind an [`Arc`](std::sync::Arc)) by the
//! `RiskService` edge (which books via `BookRatesPosition` and reads via
//! `ListRatesPositions`) and the [`RfqDeskService`](super::desk) edge (whose
//! `AcceptDeskQuote` books the dealt position here), so the desk blotter and the
//! Book workspace read one coherent book.

// `tonic::Status` is the platform-standard edge error (the pre-trade `LimitBreached`
// reject rides it, uniform with every service edge); a large `Err` variant is the
// house convention (see `services::quote` / `services::desk`), not boxed per call.
#![allow(clippy::result_large_err)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use crate::services::consensus::{ConsensusHandle, rates_book_key};

use celnet_limits::{
    IncrementalTrade, LimitScope, LimitSpec, LimitTree, NonAdditiveExposure, PreTradeDecision,
    PreTradeResult, ScopePath, pre_trade_check,
};
use celnet_proto::{
    EntitlementPrincipal, EntitlementRule, RatesPosition, RiskDimension, Side, rates_instrument,
};
use celnet_risk_cube::{BookId, EntityId, NetGreeks, NodeAggregate, VegaPillar};

use crate::services::risk::store::limit_breached_status;

/// One basis point in absolute rate terms — the scale of the linear-rates limit
/// exposure (see [`rates_linear_exposure`]).
const ONE_BP: f64 = 1e-4;

/// The shared in-memory linear-rates position book. Cheap to share behind an
/// [`Arc`](std::sync::Arc); every mutation takes the write lock briefly. Lives
/// strictly on the async edge — never the pinned zero-alloc pricing core.
#[derive(Debug)]
pub struct RatesPositionStore {
    inner: RwLock<Vec<RatesPosition>>,
    /// The monotonic id source: the next server-assigned `position_id`. Starts at
    /// `1` (id `0` is the wire "assign me a fresh id" sentinel), so booking is
    /// deterministic and test-reproducible (no wall-clock / randomness).
    next_id: AtomicU64,
    /// The pre-trade limit tree configured at the org scopes a linear-rates cell
    /// carries (`book → entity → firm`; ADR-0016 A1). Empty by default — an empty
    /// tree makes [`RatesPositionStore::book`] byte-identical to the pre-gate store
    /// (every booking accepts) until an admin path sets a cap.
    limits: RwLock<LimitTree>,
    /// The optional activated consistency tier (ADR-0015 §2.1), shared with the FX
    /// [`PositionStore`](super::risk::store::PositionStore) (one booted node backs both
    /// books, keyed into disjoint ranges by [`rates_book_key`]). A rates cell whose
    /// numeric book id resolves to `Strong` routes its authoritative write through the
    /// quorum log **before** the local apply; a `Local` cell (the default) never touches
    /// it, so the fast path stays byte-identical. Off the pinned pricing thread (§4.3).
    consensus: OnceLock<Arc<ConsensusHandle>>,
}

impl Default for RatesPositionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RatesPositionStore {
    /// An empty rates book with the id counter primed at `1` and no limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
            limits: RwLock::new(LimitTree::new()),
            consensus: OnceLock::new(),
        }
    }

    /// Attach the activated consistency tier (ADR-0015 §2.1) — the SAME handle the FX
    /// [`PositionStore`](super::risk::store::PositionStore) holds. Called once at edge
    /// boot; a store never given a handle (the pure-`Local` default) is byte-identical to
    /// today. Idempotent-once.
    pub fn set_consensus(&self, handle: Arc<ConsensusHandle>) {
        let _ = self.consensus.set(handle);
    }

    /// The builder form of [`Self::set_consensus`] for a store constructed inline.
    #[must_use]
    pub fn with_consensus(self, handle: Arc<ConsensusHandle>) -> Self {
        let _ = self.consensus.set(handle);
        self
    }

    /// The attached consistency tier, if any.
    #[must_use]
    pub fn consensus(&self) -> Option<&Arc<ConsensusHandle>> {
        self.consensus.get()
    }

    /// Configure a limit at a linear-rates org scope (admin / setup path). A rates
    /// cell rolls up through `book → entity → firm`, so a cap is set at
    /// [`LimitScope::Book`] / [`LimitScope::Entity`] / [`LimitScope::Firm`]; a cap at
    /// any other scope is never consulted by [`Self::book`] (a rates cell is not
    /// trader/desk/ccy-pair/location-attributed).
    pub fn set_limit(&self, scope: LimitScope, spec: LimitSpec) {
        let mut g = self.limits.write().expect("rates limit tree lock poisoned");
        g.set(scope, spec);
    }

    /// **The pre-trade limit gate + booking sink** for linear-rates positions
    /// (ADR-0016 A1): the single convergence point both rates booking front-ends
    /// funnel through — `RiskService::BookRatesPosition`
    /// ([`crate::services::risk`]) and `RfqDeskService::AcceptDeskQuote`
    /// ([`crate::services::desk`]). Runs [`pre_trade_check`] over the position's
    /// `book → entity → firm` roll-up path **before** mutating store state; a hard
    /// breach refuses the booking with a `failed_precondition` `LimitBreached` status
    /// and leaves the book unmutated, uniform with the FX sink
    /// ([`crate::services::risk::store::PositionStore::book`]).
    ///
    /// `position.position_id == 0` ⇒ the server assigns a fresh monotonic id (a new
    /// booking). A non-zero id **upserts** (supersedes) the current fact for that id
    /// — one current fact per `position_id`, mirroring the FX store's `upsert`
    /// supersede semantics — so a re-book never double-counts (and the pre-trade
    /// projection excludes the superseded fact).
    ///
    /// # Errors
    /// `failed_precondition` when a **hard** limit would be breached.
    pub fn book(&self, mut position: RatesPosition) -> Result<RatesPosition, tonic::Status> {
        // Pre-trade limit gate (ADR-0016 A1) over a READ snapshot, BEFORE the write lock,
        // so a hard breach can never book and the projection never runs while the exclusive
        // write lock is held (guardrails #6/#11 — a fill never serialises the whole book
        // behind its own limit projection). Skipped when no limit is set (the empty-tree
        // default keeps the store byte-identical to the pre-gate behaviour). The narrow
        // snapshot→write window is the standard pre-trade TOCTOU: a hard breach still
        // rejects here and leaves the book unmutated, the post-trade limit monitor
        // backstopping any concurrent joint breach.
        {
            let limits = self.limits.read().expect("rates limit tree lock poisoned");
            if !limits.is_empty() {
                let g = self
                    .inner
                    .read()
                    .expect("rates position store lock poisoned");
                let result = rates_pre_trade(&g, &limits, &position);
                if result.decision == PreTradeDecision::Reject {
                    return Err(limit_breached_status(&result));
                }
            }
        }

        // ADR-0015 §2.1: a rates cell whose numeric book id is configured `Strong`
        // routes its authoritative write through the Raft quorum log BEFORE the local
        // apply. Resolved once, off the hot path, from the cell's book id (the only
        // identifier a linear-rates cell carries). The id is assigned up front for a
        // Strong write so the replicated key is stable; the must-order state replicated
        // is the position's signed linear PV01 (`rates_linear_exposure`) — the derived
        // mark is regenerable (§4.3) and not quorum-logged. A `Local` cell (the default)
        // skips this, keeping today's under-lock id assignment and byte-identical path.
        if let Some(consensus) = self.consensus.get() {
            let level = consensus.level_for_rates_book(position.book);
            if level.is_strong() {
                if position.position_id == 0 {
                    position.position_id = self.next_id.fetch_add(1, Ordering::Relaxed);
                }
                consensus.commit_book_write(
                    rates_book_key(position.position_id),
                    rates_linear_exposure(&position),
                )?;
            }
        }

        let mut g = self
            .inner
            .write()
            .expect("rates position store lock poisoned");
        if position.position_id == 0 {
            position.position_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        } else {
            // Keep the counter ahead of any explicitly-booked id so a later
            // auto-assigned id never collides with a client-chosen one.
            let mut cur = self.next_id.load(Ordering::Relaxed);
            while position.position_id >= cur {
                match self.next_id.compare_exchange(
                    cur,
                    position.position_id + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(observed) => cur = observed,
                }
            }
        }
        if let Some(slot) = g.iter_mut().find(|p| p.position_id == position.position_id) {
            *slot = position;
        } else {
            g.push(position);
        }
        Ok(position)
    }

    /// A deterministic snapshot of the whole book, ascending by `position_id`.
    #[must_use]
    pub fn snapshot(&self) -> Vec<RatesPosition> {
        let g = self
            .inner
            .read()
            .expect("rates position store lock poisoned");
        let mut out = g.clone();
        out.sort_by_key(|p| p.position_id);
        out
    }

    /// The number of booked rates positions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .expect("rates position store lock poisoned")
            .len()
    }

    /// Whether the rates book is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The signed **linear interest-rate exposure** a rates position charges against a
/// limit: a conservative undiscounted PV01 (`notional · tenor_years · 1bp`) — the
/// linear IR delta of the linear-rates line — signed by direction (pay-fixed `+`,
/// receive-fixed `−`, so a payer and a receiver of equal size net to zero at a node).
///
/// This is a deliberately curve-free, deterministic P1 exposure: the discount factors
/// are `≤ 1`, so the undiscounted PV01 is an **upper bound** on the true annuity PV01
/// — conservative (fail-safe) for a hard limit. The exact curve-bootstrapped DV01 and
/// the tenor-bucketed IR limit (`LimitScope::Tenor` / a dedicated `LimitMetric::Dv01`)
/// are the ADR-0016 **A3** breadth wave (P2); this A1 gate consults the linear delta at
/// the `book → entity → firm` scopes, which is the reachability A3 depends on.
#[must_use]
fn rates_linear_exposure(position: &RatesPosition) -> f64 {
    let Some(instr) = position
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())
    else {
        return 0.0;
    };
    match instr {
        rates_instrument::Instrument::Ois(ois) => {
            let magnitude = ois.notional.abs() * f64::from(ois.tenor_years) * ONE_BP;
            match Side::try_from(ois.side) {
                // Receive-fixed is the opposite IR sign to pay-fixed, so equal-and-
                // opposite legs at one node net (mirrors the FX `delta_base` netting).
                Ok(Side::Sell) => -magnitude,
                _ => magnitude,
            }
        }
        // A vanilla IRS carries the same first-order linear-delta proxy as an OIS
        // (notional × rate-sensitive years × 1bp); receive-fixed (SIDE_SELL) nets
        // opposite pay-fixed, as for the OIS.
        rates_instrument::Instrument::Irs(irs) => {
            let magnitude = irs.notional.abs() * f64::from(irs.tenor_years) * ONE_BP;
            match Side::try_from(irs.side) {
                Ok(Side::Sell) => -magnitude,
                _ => magnitude,
            }
        }
        // A FRA's rate-sensitive span is its single accrual window; the receive-fixed
        // (SIDE_SELL) leg nets opposite the pay-fixed leg.
        rates_instrument::Instrument::Fra(fra) => {
            let window_years = f64::from(fra.end_months.saturating_sub(fra.start_months)) / 12.0;
            let magnitude = fra.notional.abs() * window_years * ONE_BP;
            match Side::try_from(fra.side) {
                Ok(Side::Sell) => -magnitude,
                _ => magnitude,
            }
        }
        // A cash bond's precise curve DV01 needs the discount curve, which this
        // curve-free pre-trade proxy does not carry; the per-1bp face redemption is a
        // coarse linear-exposure proxy pending the bond booking path (not reachable
        // through the current OIS-only desk booking, so it never feeds a live limit
        // today). A long (SIDE_BUY) bond carries long-duration exposure — the same
        // netting sign as a receive-fixed swap.
        rates_instrument::Instrument::Bond(bond) => {
            let magnitude = bond.redemption.abs() * ONE_BP;
            match Side::try_from(bond.side) {
                Ok(Side::Buy) => -magnitude,
                _ => magnitude,
            }
        }
    }
}

/// Whether an org `scope` covers a linear-rates cell — the rates analogue of the FX
/// cube's group match, over the two org axes a rates cell carries (`book`, `entity`)
/// plus the firm apex. A scope on any other dimension never matches (a rates cell is
/// not trader/desk/ccy-pair/location-attributed).
#[must_use]
fn rates_scope_matches(scope: LimitScope, p: &RatesPosition) -> bool {
    match scope {
        LimitScope::Firm => true,
        LimitScope::Book(b) => b.raw() == p.book,
        LimitScope::Entity(e) => e.raw() == p.entity,
        _ => false,
    }
}

/// Run the pre-trade limit check for a proposed rates booking against the projected
/// book: the current linear exposure at each of the position's `book → entity → firm`
/// scopes (excluding any prior fact under the same id so a re-book projects
/// `others + this trade`) plus this trade's incremental linear IR delta.
#[must_use]
fn rates_pre_trade(
    positions: &[RatesPosition],
    limits: &LimitTree,
    position: &RatesPosition,
) -> PreTradeResult {
    let path = ScopePath::from_scopes(vec![
        LimitScope::Book(BookId(position.book)),
        LimitScope::Entity(EntityId(position.entity)),
        LimitScope::Firm,
    ]);
    let mut greeks = NetGreeks::zero();
    greeks.delta_base = rates_linear_exposure(position);
    // A linear-rates line carries no vega; the increment's vega is 0, so the pillar is
    // inert (adding 0 vega to any bucket is a no-op).
    let increment = IncrementalTrade {
        greeks,
        vega_pillar: VegaPillar::new(0, 0),
        vega: 0.0,
    };
    let exclude = position.position_id;
    let node_at = |scope: LimitScope| -> NodeAggregate {
        let sum: f64 = positions
            .iter()
            .filter(|p| exclude == 0 || p.position_id != exclude)
            .filter(|p| rates_scope_matches(scope, p))
            .map(rates_linear_exposure)
            .sum();
        let mut node = NodeAggregate::empty(scope.group_value().unwrap_or(0));
        node.net_greeks.delta_base = sum;
        node
    };
    // Booking never re-derives VaR/ES/stop-loss on the linear-rates path.
    let nonadditive_at = |_scope: LimitScope| NonAdditiveExposure::default();
    pre_trade_check(limits, &path, &increment, node_at, nonadditive_at)
}

/// Whether a rates `(entity, book)` cell is admitted by an asserted entitlement
/// principal — the post-boundary pruning predicate `ListRatesPositions` applies,
/// mirroring the FX `ListPositions` deny-by-default semantics over the **flat**
/// org axes a rates cell carries (no Book→Desk hierarchy, no underlying):
///
/// * an **absent** principal ⇒ grant-all (the post-boundary convention
///   [`convert::principal_of`](super::risk::convert::principal_of) uses: the access
///   boundary already mode-gated the absence; here it means "see everything");
/// * **deny wins**: any deny rule covering the cell denies it;
/// * `grant_all` ⇒ admitted (after deny);
/// * otherwise admitted iff some grant rule covers the cell.
#[must_use]
pub fn admits_rates_cell(principal: Option<&EntitlementPrincipal>, entity: u32, book: u32) -> bool {
    let Some(p) = principal else {
        return true;
    };
    if p.denies.iter().any(|r| rule_covers_cell(r, entity, book)) {
        return false;
    }
    if p.grant_all {
        return true;
    }
    p.grants.iter().any(|r| rule_covers_cell(r, entity, book))
}

/// Whether one entitlement rule covers a rates `(entity, book)` cell. A rule covers
/// the cell iff **every** scope it carries is on a dimension the flat rates cell
/// models (`ENTITY`, `BOOK`, or the `FIRM` apex) and matches. A scope on any other
/// org dimension (`TRADER` / `DESK` / `LOCATION` / `UNDERLYING`) cannot match a
/// rates cell — which is not trader/desk/location/underlying-attributed — so the
/// rule does not cover it. An empty rule (no scopes) is a firm-wide grant: it covers
/// every cell (`all` over an empty iterator is `true`).
fn rule_covers_cell(rule: &EntitlementRule, entity: u32, book: u32) -> bool {
    rule.scopes
        .iter()
        .all(|s| match RiskDimension::try_from(s.dimension) {
            Ok(RiskDimension::Firm) => true,
            Ok(RiskDimension::Entity) => s.value == u64::from(entity),
            Ok(RiskDimension::Book) => s.value == u64::from(book),
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{
        EntitlementRule, OisInstrument, RatesInstrument, RiskScope, Side, rates_instrument,
    };

    fn position(id: u64, entity: u32, book: u32) -> RatesPosition {
        RatesPosition {
            position_id: id,
            entity,
            book,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 5,
                    fixed_rate: 0.04,
                    notional: 10_000_000.0,
                    side: Side::Buy as i32,
                })),
            }),
        }
    }

    /// A zero-id booking is assigned a fresh monotonic id; a snapshot reports the
    /// book ascending by id.
    #[test]
    fn booking_assigns_monotonic_ids() {
        let store = RatesPositionStore::new();
        let a = store.book(position(0, 1, 10)).unwrap();
        let b = store.book(position(0, 1, 11)).unwrap();
        assert_eq!(a.position_id, 1);
        assert_eq!(b.position_id, 2);
        assert_eq!(store.len(), 2);
        let snap = store.snapshot();
        assert_eq!(
            snap.iter().map(|p| p.position_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    /// A non-zero id upserts (supersedes) the current fact for that id — one current
    /// fact per id, never a duplicate.
    #[test]
    fn rebooking_an_id_supersedes() {
        let store = RatesPositionStore::new();
        let first = store.book(position(0, 1, 10)).unwrap();
        assert_eq!(first.position_id, 1);
        // Re-book id 1 in a different book — supersede, not duplicate.
        let again = store.book(position(1, 1, 99)).unwrap();
        assert_eq!(again.position_id, 1);
        assert_eq!(store.len(), 1);
        assert_eq!(store.snapshot()[0].book, 99);
    }

    /// An explicit id keeps the auto-id counter ahead, so no later auto-assigned id
    /// collides with a client-chosen one.
    #[test]
    fn explicit_id_advances_the_counter() {
        let store = RatesPositionStore::new();
        let _ = store.book(position(50, 1, 10)).unwrap();
        let next = store.book(position(0, 1, 11)).unwrap();
        assert_eq!(next.position_id, 51);
    }

    // ---- ADR-0016 A1 pre-trade limit gate at the rates position sink ----
    //
    // `RatesPositionStore::book` is the exact function BOTH rates booking front-ends
    // funnel through — `RiskService::BookRatesPosition` and
    // `RfqDeskService::AcceptDeskQuote` — so gating it gates both by construction. A 5y
    // 10mm OIS charges `10mm · 5 · 1bp = 5000` of linear IR exposure against a Delta cap.

    /// The rates sink **rejects** a hard-limit-blown OIS booking with a
    /// `failed_precondition` `LimitBreached` status and leaves the book unmutated.
    #[test]
    fn rates_sink_rejects_a_hard_limit_blown_book() {
        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Delta, 1.0),
        );

        let err = store
            .book(position(0, 1, 10))
            .expect_err("a hard-limit-blown rates book must be rejected");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("limit breached"),
            "the reject carries the uniform LimitBreached reason, got {:?}",
            err.message()
        );
        assert_eq!(
            store.len(),
            0,
            "a rejected rates book must not mutate the book"
        );
    }

    /// A within-limit rates booking still succeeds and records the position.
    #[test]
    fn rates_within_limit_book_succeeds() {
        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Delta, 1.0e12),
        );
        let booked = store
            .book(position(0, 1, 10))
            .expect("within-limit rates book");
        assert_eq!(booked.position_id, 1);
        assert_eq!(store.len(), 1);
    }

    /// A soft rates limit breach books (never blocks) — soft limits only early-warn.
    #[test]
    fn rates_soft_breach_books() {
        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::soft(celnet_limits::LimitMetric::Delta, 1.0),
        );
        let booked = store
            .book(position(0, 1, 10))
            .expect("a soft rates breach books (warn, never block)");
        assert_eq!(booked.position_id, 1);
        assert_eq!(store.len(), 1);
    }

    /// Absent principal ⇒ grant-all; a grant on `BOOK=10` admits only that book; a
    /// deny wins over a grant.
    #[test]
    fn entitlement_prunes_by_book_cell() {
        assert!(admits_rates_cell(None, 1, 10));

        let grant_book_10 = EntitlementPrincipal {
            grant_all: false,
            grants: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Book as i32,
                    value: 10,
                }],
            }],
            denies: vec![],
        };
        assert!(admits_rates_cell(Some(&grant_book_10), 1, 10));
        assert!(!admits_rates_cell(Some(&grant_book_10), 1, 11));

        let grant_all_deny_book_11 = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Book as i32,
                    value: 11,
                }],
            }],
        };
        assert!(admits_rates_cell(Some(&grant_all_deny_book_11), 1, 10));
        assert!(!admits_rates_cell(Some(&grant_all_deny_book_11), 1, 11));
    }

    /// A grant on a dimension a flat rates cell does NOT model (e.g. DESK) cannot
    /// match it — the rate cell is not desk-attributed.
    #[test]
    fn grant_on_unmodelled_dimension_does_not_match() {
        let grant_desk = EntitlementPrincipal {
            grant_all: false,
            grants: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Desk as i32,
                    value: 3,
                }],
            }],
            denies: vec![],
        };
        assert!(!admits_rates_cell(Some(&grant_desk), 1, 10));
    }
}
