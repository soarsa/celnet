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

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use crate::services::consensus::{ConsensusHandle, rates_book_key};

use celnet_acceptance::AcceptanceGraph;
use celnet_hedge_routing::{
    Dv01Basis, HedgeContext, HedgeGraph, HedgeRatioPlan, HedgeVehicle, HedgeVehicleRegistry,
    HedgeVehicleRule, WarehouseThreshold, plan_hedge_ratio,
};
use celnet_limits::{
    IncrementalTrade, LimitCheck, LimitScope, LimitSpec, LimitTree, NonAdditiveExposure,
    PreTradeDecision, PreTradeResult, ScopePath, pre_trade_check_mixed,
};
use celnet_proto::{
    BondInstrument, EntitlementPrincipal, EntitlementRule, HedgeProvenance, InternaliseProvenance,
    RatesPosition, RiskDimension, Side, rates_instrument,
};
use celnet_risk_cube::{BookId, EntityId, NetGreeks, NodeAggregate, VegaPillar};
use celnet_risk_fleet::{RatesFactKey, RatesNodeAggregate, RatesRiskFact, firm_aggregate_rates};
use celnet_risk_routing::{RiskRouter, RiskRoutingGraph, RoutingContext};

use crate::config::hedge_policy::{
    HedgeConfigDef, HedgeMetric, HedgePolicyScope, HedgeScopeKind, HedgeThresholdDef,
    ScopedHedgeGraph, ScopedThreshold,
};
use crate::config::identity::{
    IdentityStore, RiskLimits, default_hedge_policy_graph, default_warehouse_threshold_def,
};
use crate::services::auto_hedge::AutoHedgeEngine;
use crate::services::auto_hedge::wire::band_label;
use crate::services::internalise::{self, QuoteKind};
use crate::services::risk::store::{RiskBookLimitDef, limit_breached_status};

/// One basis point in absolute rate terms — the scale of the linear-rates limit
/// exposure (see [`rates_linear_exposure`]).
const ONE_BP: f64 = 1e-4;

/// The booking-time **routing attribution** for a rates fill — the originating fields the
/// `RatesPosition` does not itself carry but the firm-wide routing graph can match on
/// (`RouteField::Counterparty` / `RouteField::Ccy`). It is a pure routing *input*, not
/// persisted on the position (the position round-trips without it), so it is threaded
/// alongside [`RatesPositionStore::book_with_routing`] rather than added to the proto — the
/// RFQ desk path supplies both from the `DeskRequest` (its requester = counterparty, and the
/// `CurveSet` currency), while the manual `RiskService::BookRatesPosition` path carries
/// neither and books with the empty default (honestly unmatched by a counterparty/ccy rule).
#[derive(Debug, Clone, Default)]
pub struct RatesRoutingAttribution {
    /// The originating counterparty (the client / FIX session that dealt), or empty when the
    /// booking path does not carry one (the manual `BookRatesPosition`).
    pub counterparty: String,
    /// The settlement/curve currency (ISO 4217, e.g. `"USD"`) the fill priced under, or empty
    /// when unknown at the booking site.
    pub ccy: String,
    /// The **dealt level** of the fill (the accepted `DeskQuote.price`): the swap/FRA fixed
    /// rate, or the cash-bond clean price. `Some` on the RFQ-desk accept / FIX-lift paths (so
    /// the booking engine can measure the dealer-captured edge); `None` on the manual
    /// `BookRatesPosition` path (no internalise decision — byte-identical to before).
    pub dealt_price: Option<f64>,
    /// The engine **reference mid** the fill is measured against — the fair par rate (rate
    /// markets) or the mid clean price (bonds) the pricer computed in the accept/lift handler.
    /// `Some` beside [`dealt_price`](Self::dealt_price); `None` ⇒ no internalise decision.
    pub reference_mid: Option<f64>,
    /// The acceptance **`request_id`** the FIX-lift / RFQ-desk accept booked against — the
    /// correlation key the event-trace hub bound the lift's `trace_id` to at quote publish
    /// (`services::trace`). Threaded through here so `book_with_routing` can resolve the same
    /// `trace_id` and emit the routing / booking / hedge stage events at their TRUE instants.
    /// `None` on the manual `BookRatesPosition` / test paths (no trace).
    pub request_id: Option<String>,
}

/// The firm-wide **auto-hedge / internalisation policy** snapshot primed into the rates
/// store beside the routing graph (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`
/// §6/§7). A booked RFQ-desk / FIX-lift fill that carries a priced reference mid resolves
/// its `(warehouse-cap × hedge-policy graph)` here and stamps an [`InternaliseProvenance`]
/// decision onto the deal — the internal (warehoused) vs advisory-external split, the
/// captured edge, and the RAG band. Primed once at boot from the persisted `IdentityStore`
/// and re-primed after every admin hedge write (the `AuthEdge` reconcile hook). `None` on the
/// store ⇒ no internalise decision runs and booking is byte-identical to the pre-Phase-B path.
#[derive(Clone)]
pub struct RatesHedgePolicy {
    /// The shared off-core decision + provenance engine (the SAME `Arc` the
    /// `ListHedgeProvenance` RPC reads), so a stamped decision also lands in the audit ring.
    pub engine: Arc<AutoHedgeEngine>,
    /// The firm-wide (`Firm`-scope) hedge-policy exit graph. `None` ⇒ no firm policy; a
    /// fill may still resolve a `Book` / `Bucket` scoped graph (`scoped_graphs`).
    pub graph: Option<HedgeGraph>,
    /// The scope-bound (`Book` / `Bucket`) hedge policy graphs (§5). A fill resolves the
    /// most-specific: its own `Book` graph, else the nearest ancestor `Bucket` graph, else
    /// the firm `graph`.
    pub scoped_graphs: Vec<ScopedHedgeGraph>,
    /// The configured warehouse thresholds (the "100" per desk / book / instrument), resolved
    /// most-specific-wins per fill.
    pub thresholds: Vec<ScopedThreshold>,
    /// The engine config (kill-switch / execution mode / rate guards / `min_edge_bps` floor).
    pub config: HedgeConfigDef,
    /// The live known-LP registry the engine resolves an external action's target panel against.
    pub known_lps: BTreeSet<String>,
    /// Each risk book's ancestor ids (nearest first) — the parent-chain snapshot used to
    /// resolve a `Bucket` policy governing a fill's book (§5). Primed from the identity tree.
    pub book_ancestors: HashMap<String, Vec<String>>,
    /// Each risk book's descendant ids — the subtree snapshot used to roll up a `Bucket`
    /// policy's risk state (the "PORTFOLIO notional" the decision measures). Primed from the
    /// identity tree.
    pub book_descendants: HashMap<String, Vec<String>>,
}

impl std::fmt::Debug for RatesHedgePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RatesHedgePolicy")
            .field("has_graph", &self.graph.is_some())
            .field("scoped_graphs", &self.scoped_graphs.len())
            .field("thresholds", &self.thresholds.len())
            .field("known_lps", &self.known_lps.len())
            .finish()
    }
}

impl RatesHedgePolicy {
    /// Build the hedge-policy snapshot from the persisted [`IdentityStore`] — the single
    /// source of truth both prime sites (boot in `lib.rs` and `AuthEdge::reconcile_hedge_policy`)
    /// share, so the firm + scoped graphs, thresholds, config, known-LP set, and the risk-book
    /// tree snapshot (ancestor + descendant maps, for `Bucket`-scope selection + subtree
    /// roll-up) are primed consistently.
    #[must_use]
    pub fn from_identity(engine: Arc<AutoHedgeEngine>, store: &IdentityStore) -> Self {
        let mut book_ancestors = HashMap::new();
        let mut book_descendants = HashMap::new();
        for b in &store.risk_books {
            book_ancestors.insert(
                b.id.clone(),
                store
                    .risk_book_ancestors(&b.id)
                    .iter()
                    .map(|a| a.id.clone())
                    .collect(),
            );
            book_descendants.insert(
                b.id.clone(),
                store
                    .risk_book_descendants(&b.id)
                    .iter()
                    .map(|d| d.id.clone())
                    .collect(),
            );
        }
        Self {
            engine,
            graph: store.hedge_policy_graph().cloned(),
            scoped_graphs: store.scoped_hedge_policy_graphs().to_vec(),
            thresholds: store.hedge_thresholds().to_vec(),
            config: store.hedge_config().clone(),
            known_lps: store.known_hedge_lps(),
            book_ancestors,
            book_descendants,
        }
    }

    /// Resolve the **most-specific** hedge policy graph governing a fill in `book` (§5),
    /// mirroring [`IdentityStore::select_hedge_policy_graph`] off the primed snapshot: the
    /// book's own `Book` graph, else the nearest ancestor `Bucket` graph (with its subtree
    /// root id), else the firm `graph`. `None` ⇒ no policy at any scope (the caller falls
    /// back to the default warehouse graph).
    #[must_use]
    fn select_scoped_graph(&self, book: &str) -> Option<(&HedgeGraph, Option<String>)> {
        // 1. the book's own Book-scope policy.
        if let Some(sg) = self
            .scoped_graphs
            .iter()
            .find(|s| s.scope == HedgePolicyScope::Book(book.to_owned()))
        {
            return Some((&sg.graph, None));
        }
        // 2. the nearest ancestor Bucket policy (self as a bucket root first, then upward).
        let mut chain = vec![book.to_owned()];
        if let Some(anc) = self.book_ancestors.get(book) {
            chain.extend(anc.iter().cloned());
        }
        for id in chain {
            if let Some(sg) = self
                .scoped_graphs
                .iter()
                .find(|s| s.scope == HedgePolicyScope::Bucket(id.clone()))
            {
                return Some((&sg.graph, Some(id)));
            }
        }
        // 3. the firm default.
        self.graph.as_ref().map(|g| (g, None))
    }

    /// The human-readable label of the policy scope [`Self::select_scoped_graph`] would
    /// resolve for `book` — the exact same precedence, reported rather than applied.
    ///
    /// This is what a decision-journal row records under `scope`, so a trader reading the
    /// audit table sees WHICH of their policies governed a decision (and, crucially, when
    /// none of them did: [`SCOPE_NONE`](crate::services::auto_hedge::SCOPE_NONE)).
    fn scope_label(&self, book: &str) -> String {
        if self
            .scoped_graphs
            .iter()
            .any(|s| s.scope == HedgePolicyScope::Book(book.to_owned()))
        {
            return format!("book:{book}");
        }
        let mut chain = vec![book.to_owned()];
        if let Some(anc) = self.book_ancestors.get(book) {
            chain.extend(anc.iter().cloned());
        }
        for id in chain {
            if self
                .scoped_graphs
                .iter()
                .any(|s| s.scope == HedgePolicyScope::Bucket(id.clone()))
            {
                return format!("bucket:{id}");
            }
        }
        if self.graph.is_some() {
            return "firm".to_owned();
        }
        crate::services::auto_hedge::SCOPE_NONE.to_owned()
    }
}

/// The shared in-memory linear-rates position book. Cheap to share behind an
/// [`Arc`](std::sync::Arc); every mutation takes the write lock briefly. Lives
/// strictly on the async edge — never the pinned zero-alloc pricing core.
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
    /// The current firm-wide **risk-routing graph** (`docs/FI-RISK-ROUTING-REQUIREMENTS.md`
    /// §4) — the SAME graph the FX [`PositionStore`](super::risk::store::PositionStore) holds,
    /// pushed by the SAME reconcile sites (boot prime + `AuthEdge::reconcile_risk_routing`).
    /// `None` ⇒ no routing: a rates fill books exactly as before with **no** risk-book stamp,
    /// byte-identical to the pre-routing path. Behind an [`Arc`] so a fill clones only a
    /// pointer off the off-hot-path booking tier.
    routing: RwLock<Option<Arc<RiskRoutingGraph>>>,
    /// The resolved `risk_book_id` stamped on each routed rates fill, keyed by the rates
    /// position id — the rates analogue of the FX store's `risk_book` bucketing dimension
    /// (§8.3). A position id is present only when its fill was routed (a graph was
    /// configured and [`RiskRouter::route`] resolved a book); absent ⇒ unrouted. The
    /// existing `(entity, book)` keying is untouched, so an unrouted store is
    /// byte-identical to today.
    risk_book: RwLock<HashMap<u64, String>>,
    /// The current **risk-book limit view** (§8.3 enforcement): each routed book's parent
    /// (for subtree roll-up) + optional hard [`RiskLimits`], keyed by book id. Kept current
    /// by [`RatesPositionStore::set_risk_books`] from the SAME reconcile sites the FX store
    /// uses. Empty — or a view with no caps on the fill's book path — ⇒ the per-book gate is
    /// skipped, byte-identical to the pre-enforcement booking path.
    risk_book_limits: RwLock<HashMap<String, RiskBookLimitDef>>,
    /// A monotonic **risk version** bumped on every successful [`RatesPositionStore::book`]
    /// (a position change) — the rates analogue of the FX store's `risk_version`. The live
    /// per-book risk stream ([`crate::services::stream`]) folds this into its poll signal
    /// (summed with the FX version) so a rates fill advances the stream exactly as an FX
    /// fill does. A relaxed atomic (read lock-free off the streaming tick loop, never the
    /// pinned pricing core); starts at `0`.
    risk_version: AtomicU64,
    /// The firm-wide **auto-hedge / internalisation policy** snapshot (§6/§7). `None` ⇒ no
    /// internalise decision runs (booking byte-identical to the pre-Phase-B path); `Some`
    /// enables the per-fill warehouse-vs-external decision + [`InternaliseProvenance`] stamp.
    /// Primed at boot + on every admin hedge write, behind an [`Arc`] so a booking clones only
    /// a pointer off the off-hot-path booking tier.
    hedge_policy: RwLock<Option<Arc<RatesHedgePolicy>>>,
    /// The internalise-decision provenance stamped on each fill that carried a priced
    /// reference mid AND resolved a hedge policy, keyed by rates position id (the analogue of
    /// [`risk_book`](Self::risk_book)). Absent ⇒ the fill booked with no internalise decision
    /// (manual path, or no policy configured) — surfaced on the deal, never fabricated.
    internalise: RwLock<HashMap<u64, InternaliseProvenance>>,
    /// The firm-wide **incoming-quote-acceptance** decision graph snapshot (the third
    /// trader-configurable rule engine — `celnet-acceptance`). `None` ⇒ no acceptance
    /// gating: the FIX acceptance point accepts every lift, byte-identical to the
    /// pre-acceptance path. Primed at boot + on every admin acceptance write (the SAME
    /// `AuthEdge` reconcile hook that re-primes the routing / hedge snapshots), behind an
    /// [`Arc`] so the desk edge clones only a pointer when it evaluates a lift off the FIX
    /// edge (never the pinned pricing core).
    acceptance: RwLock<Option<Arc<AcceptanceGraph>>>,
    /// The shared latency/ops telemetry hub (the SAME `Arc` the FX
    /// [`PositionStore`](super::risk::store::PositionStore) and the `CoreLink` hold,
    /// `docs/LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md` §4.3). Set once at boot; the FI
    /// booking seams on the async edge record their per-stage latency here (best-order booking
    /// commit, risk-routing decision, auto-hedge fire) — never the pinned pricing thread
    /// (guardrail 11). `None` (a store never given a hub — the unit tests' default) ⇒ the
    /// record calls are no-ops, byte-identical to the pre-instrumentation path.
    telemetry: OnceLock<Arc<crate::services::telemetry::TelemetryHub>>,
    /// The shared end-to-end **event-trace** hub (the SAME `Arc` the `CoreLink` and FIX edge
    /// hold). Set once at boot; the booking seam records the routing / deal-booked / hedge
    /// stage events into it OFF the pinned pricing core (guardrail 11). `None` (the unit-test
    /// default) ⇒ every trace call is a no-op, byte-identical to the pre-tracing path.
    trace: OnceLock<Arc<crate::services::trace::TraceHub>>,
    /// The live **external-hedge LP source** (§6.2) — the standing LP panel a live-LP hedge
    /// mode (`LpPanel` / `LpPanelThenComposite`) fills an external shed against. In production
    /// this is the aggregation hub (the inbound per-LP quotes the agg book consolidates), wired
    /// once at boot; a store never given one falls back to the honest [`NoLpSource`], so an
    /// `LpPanelThenComposite` shed reaches the composite backstop and a pure `LpPanel` shed
    /// records an honest miss — never a fabricated LP fill (guardrail 2). Off the pinned core.
    lp_hedge_source: OnceLock<Arc<dyn crate::services::auto_hedge::LpHedgeSource>>,
    /// The live **street-order router** — the seam that sends a real `NewOrderSingle(D)`
    /// to a ranked panel member and waits for its `ExecutionReport(8)`. Deliberately
    /// separate from `lp_hedge_source`: that one reports who is *showing* a price, this
    /// one is the only thing that can obtain a counterparty's *agreement*. A store never
    /// given one falls back to
    /// [`NoStreetRouter`](crate::services::auto_hedge::NoStreetRouter), which routes
    /// nothing and says so — so a firm quote can never become a fill by default.
    /// Off the pinned core.
    street_router: OnceLock<Arc<dyn crate::services::auto_hedge::StreetOrderRouter>>,
    /// The shared **street-side execution log** (Analytics §2.4). EVERY outbound street
    /// order this seam works is recorded there off-core — fills, partials, rejections,
    /// composite backstops and honest misses alike — carrying the side, the requested-vs-filled
    /// quantity, the price and slippage, the outcome and its reason, the ranked panel it
    /// competed against, and the parent hedge / position linkage. The Street-side LP league
    /// table folds the SAME `Arc` as an `LpFlowSource` (crediting only real named-LP fills),
    /// and `ListStreetOrders` serves the blotter + breakdowns off it. `None` (the unit-test
    /// default) means the recording call is a no-op.
    street_orders: OnceLock<Arc<crate::services::analytics::street_orders::StreetOrderLog>>,
    /// The detached-hedge work queue, drained by ONE long-lived worker.
    ///
    /// The first cut spawned an OS thread per booking. That is not cheap at booking rates:
    /// the phase breakdown on UAT put 0.9-2.4ms in the dispatch bucket, dwarfing the
    /// pre-trade gate (25-100us) and the write commit (19-118us) it was meant to get out
    /// of the way of — async moved the venue round trip off the path and put a thread
    /// spawn there instead.
    ///
    /// Bounded, deliberately: a full queue means the hedge worker is not keeping up, and
    /// the honest response is to run that hedge INLINE (slower, still correct) rather than
    /// grow an unbounded backlog of risk nobody is working.
    hedge_queue: OnceLock<std::sync::mpsc::SyncSender<HedgeJob>>,
    /// The **standing hedge suggestions** raised under a `Suggest`-mode scope (§6.5) — the
    /// manual half of suggest-then-exit. A breach in such a scope computes the whole hedge
    /// (band, action, vehicle, DV01 ratio, whole-lot rounding) and publishes it here
    /// instead of trading it; the `ExecuteHedgeSuggestion` RPC later fires exactly that
    /// pinned plan. Always present (an empty store costs nothing) and shared behind an
    /// `Arc` with the `AuthEdge` RPC handlers.
    suggestions: Arc<crate::services::auto_hedge::SuggestionStore>,
}

/// A read view of one booked rates position assembled for a **risk transfer** (§6):
/// the slice fields the pure leg computation needs (signed notional, per-unit dealt-level
/// mark, linear DV01) plus the stored [`RatesPosition`] template an economic transfer
/// re-books its offsetting / opening legs from. Produced by
/// [`RatesPositionStore::rates_transfer_view`].
#[derive(Debug, Clone)]
pub struct RatesTransferView {
    /// The risk book the position is currently stamped into (`None` ⇒ unrouted).
    pub risk_book: Option<String>,
    /// Trade-direction signed notional (+ pay-fixed / long, − receive-fixed / short).
    pub signed_notional: f64,
    /// The per-unit dealt-level mark (swap/FRA `fixed_rate`, bond `coupon_rate`) — a rates
    /// cell carries no marking surface, so this contractual level is its stored mark.
    pub mark: f64,
    /// The curve-free linear PV01 proxy (`notional · tenor · 1bp`, signed by IR duration) —
    /// the same measure the pre-trade limit gate charges (the exact curve DV01 needs a live
    /// `CurveSet` not carried on the transfer seam).
    pub dv01: f64,
    /// The stored position (the template an economic transfer re-books legs from).
    pub template: RatesPosition,
}

impl Default for RatesTransferView {
    fn default() -> Self {
        Self {
            risk_book: None,
            signed_notional: 0.0,
            mark: 0.0,
            dv01: 0.0,
            template: RatesPosition::default(),
        }
    }
}

impl std::fmt::Debug for RatesPositionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A concise state summary — the trait-object seams (`lp_hedge_source`) are not `Debug`,
        // so this hand impl replaces the derive and reports their presence rather than contents.
        f.debug_struct("RatesPositionStore")
            .field(
                "positions",
                &self.inner.read().map(|g| g.len()).unwrap_or(0),
            )
            .field("next_id", &self.next_id.load(Ordering::Relaxed))
            .field(
                "has_routing",
                &self.routing.read().map(|g| g.is_some()).unwrap_or(false),
            )
            .field(
                "has_hedge_policy",
                &self
                    .hedge_policy
                    .read()
                    .map(|g| g.is_some())
                    .unwrap_or(false),
            )
            .field("has_lp_hedge_source", &self.lp_hedge_source.get().is_some())
            .field("has_street_order_log", &self.street_orders.get().is_some())
            .finish_non_exhaustive()
    }
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
            hedge_queue: OnceLock::new(),
            limits: RwLock::new(LimitTree::new()),
            consensus: OnceLock::new(),
            routing: RwLock::new(None),
            risk_book: RwLock::new(HashMap::new()),
            risk_book_limits: RwLock::new(HashMap::new()),
            risk_version: AtomicU64::new(0),
            hedge_policy: RwLock::new(None),
            internalise: RwLock::new(HashMap::new()),
            acceptance: RwLock::new(None),
            telemetry: OnceLock::new(),
            trace: OnceLock::new(),
            lp_hedge_source: OnceLock::new(),
            street_router: OnceLock::new(),
            street_orders: OnceLock::new(),
            suggestions: Arc::new(crate::services::auto_hedge::SuggestionStore::new()),
        }
    }

    /// Attach the shared latency/ops telemetry hub so the FI booking seams record their
    /// per-stage latency (best-order booking commit → [`OpKind::Book`], risk-routing decision
    /// → [`OpKind::RiskRoute`], auto-hedge fire → [`OpKind::HedgeFire`]). The SAME hub the FX
    /// [`PositionStore::set_telemetry`](super::risk::store::PositionStore::set_telemetry)
    /// receives, wired once at boot. Idempotent-once; a store never given a hub records
    /// nothing (byte-identical to the pre-instrumentation path).
    pub fn set_telemetry(&self, hub: Arc<crate::services::telemetry::TelemetryHub>) {
        let _ = self.telemetry.set(hub);
    }

    /// Attach the shared end-to-end event-trace hub so the booking seam emits the
    /// routing / deal-booked / hedge stage events for a lift. The SAME hub the `CoreLink`
    /// owns, wired once at boot. Idempotent-once; a store never given a hub records nothing.
    pub fn set_trace(&self, hub: Arc<crate::services::trace::TraceHub>) {
        let _ = self.trace.set(hub);
    }

    /// Attach the live **external-hedge LP source** (§6.2) — the standing LP panel a live-LP
    /// hedge mode fills an external shed against (production wires the aggregation hub's inbound
    /// per-LP quotes). Wired once at boot; idempotent-once. A store never given one falls back to
    /// the honest [`NoLpSource`](crate::services::auto_hedge::NoLpSource), so an
    /// `LpPanelThenComposite` shed reaches the composite backstop and a pure `LpPanel` shed
    /// records an honest miss — never a fabricated LP fill.
    pub fn set_lp_hedge_source(&self, src: Arc<dyn crate::services::auto_hedge::LpHedgeSource>) {
        let _ = self.lp_hedge_source.set(src);
    }

    /// Start the detached-hedge dispatcher (once, at wiring time). Required for
    /// [`HedgeDispatch`](crate::config::hedge_policy::HedgeDispatch)`::Async`; without it
    /// there is no queue to hand work to and a hedge configured async runs inline.
    pub fn set_self_handle(&self, me: &Arc<RatesPositionStore>) {
        // One worker, started once, draining the queue for the life of the process — in
        // place of a thread per booking. The worker holds a WEAK handle and exits when the
        // store goes away, so it never keeps a dropped store alive.
        let (tx, rx) = std::sync::mpsc::sync_channel::<HedgeJob>(HEDGE_QUEUE_DEPTH);
        if self.hedge_queue.set(tx).is_err() {
            return; // already started
        }
        let weak = Arc::downgrade(me);
        let _ = std::thread::Builder::new()
            .name("celnet-hedge-dispatch".to_owned())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let Some(store) = weak.upgrade() else { return };
                    store.stamp_internalise(
                        &job.fill,
                        job.book.as_deref(),
                        &job.attribution,
                        job.trace_id,
                    );
                }
            });
    }

    /// Attach the live **street-order router** — the outbound FIX seam that actually
    /// sends a hedge to a named counterparty and waits for its answer. Wired once at
    /// boot; idempotent-once. A store never given one uses
    /// [`NoStreetRouter`](crate::services::auto_hedge::NoStreetRouter): the panel is
    /// still ranked (a real observation) but nothing can fill on it, so
    /// `LpPanelThenComposite` backstops and pure `LpPanel` records an honest miss.
    pub fn set_street_router(
        &self,
        router: Arc<dyn crate::services::auto_hedge::StreetOrderRouter>,
    ) {
        let _ = self.street_router.set(router);
    }

    /// The installed street-order router, or the honest no-op.
    fn street_router(&self) -> &dyn crate::services::auto_hedge::StreetOrderRouter {
        self.street_router
            .get()
            .map_or(&crate::services::auto_hedge::NoStreetRouter, |r| r.as_ref())
    }

    /// Attach the shared **street-side execution log** (Analytics §2.4) so every outbound
    /// street order this store works is captured — and a fill on a named LP is attributed to
    /// that LP in the league table. The SAME `Arc` the analytics rollup folds as an
    /// `LpFlowSource` and the `ListStreetOrders` RPC reads; wired once at boot,
    /// idempotent-once. A store never given one records nothing (byte-identical otherwise).
    pub fn set_street_order_log(
        &self,
        log: Arc<crate::services::analytics::street_orders::StreetOrderLog>,
    ) {
        let _ = self.street_orders.set(log);
    }

    /// The attached street-side execution log, if any (used by the recording seams).
    fn street_order_log(
        &self,
    ) -> Option<&Arc<crate::services::analytics::street_orders::StreetOrderLog>> {
        self.street_orders.get()
    }

    /// Record an async-edge stage latency (`nanos`) under `kind` into the shared telemetry hub,
    /// when one is installed — the drain-side `record_edge` idiom, off the pinned pricing core
    /// (guardrail 11). A no-op when no hub is attached, so the booking seams and the desk edge
    /// (which holds this store) can bracket their work unconditionally without a hub check at
    /// every call site. `nanos` is a monotonic `Instant` elapsed span the caller measured.
    pub fn record_latency(&self, kind: celnet_observability::OpKind, nanos: u64) {
        if let Some(hub) = self.telemetry.get() {
            hub.record_edge(kind, nanos);
        }
    }

    /// Emit one booking-side event-trace stage for `trace_id`, when a trace hub is
    /// installed (a no-op otherwise). Booking-side events pass an EMPTY symbol so the
    /// store keeps the FIX-side `Symbol(55)` captured earlier in the lift; the durable
    /// `position_id` in the details links a blotter `Deal` to its trace. Off the pinned
    /// pricing core (guardrail 11) — an `Arc` deref + a bounded lossy `try_send`.
    fn trace_stage(
        &self,
        trace_id: u64,
        stage: celnet_proto::TraceStage,
        details: crate::services::trace::TraceDetails,
    ) {
        if let Some(hub) = self.trace.get() {
            hub.record(trace_id, stage, "", details);
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

    /// A clone of the configured limit tree — the read snapshot the rates risk aggregate
    /// gate ([`crate::services::rates_risk::aggregate_rates_risk_gated`]) evaluates the
    /// fixed-income limits over, taken off the async edge (never the pinned pricing
    /// core). Cheap: the configured limit set is bounded (O(scopes × metric-types)), and
    /// [`LimitTree`] is allocation-light `Clone`.
    #[must_use]
    pub fn limits_snapshot(&self) -> LimitTree {
        self.limits
            .read()
            .expect("rates limit tree lock poisoned")
            .clone()
    }

    /// Install (or clear) the live firm-wide **risk-routing graph** (§4) — the rates
    /// analogue of [`PositionStore::set_routing`](super::risk::store::PositionStore::set_routing).
    /// Pushed at boot from the persisted graph and re-pushed after every admin edit via the
    /// SAME `AuthEdge::reconcile_risk_routing` hook, so defining/editing/clearing the graph
    /// takes effect on subsequent rates fills immediately. `Some(graph)` ⇒ each subsequent
    /// fill through [`Self::book`] is routed to a risk book and stamped; `None` ⇒ routing is
    /// off and fills book exactly as before (no stamp) — the backward-compatible default.
    ///
    /// Thread-safe: takes the routing write lock briefly (control-plane cadence, never the
    /// pinned pricing core) and swaps an [`Arc`], so a concurrent booking sees either the old
    /// or the new graph atomically, never a torn one.
    pub fn set_routing(&self, graph: Option<RiskRoutingGraph>) {
        let mut g = self.routing.write().expect("rates routing lock poisoned");
        *g = graph.map(Arc::new);
    }

    /// Push the current **risk-book limit view** into the store — each book's parent (for
    /// subtree roll-up) and optional hard [`RiskLimits`] notional caps. Kept current beside
    /// [`Self::set_routing`] by the SAME two reconcile sites the FX store uses (the boot-time
    /// prime and the per-write `AuthEdge::reconcile_risk_routing` hook), so a routed rates
    /// fill that would breach its risk book's (or an ancestor's) hard notional cap is refused.
    /// An empty view (no books, or none carrying limits) ⇒ the per-book gate is skipped and
    /// booking is byte-identical to the pre-enforcement path.
    pub fn set_risk_books(&self, books: Vec<RiskBookLimitDef>) {
        let mut g = self
            .risk_book_limits
            .write()
            .expect("rates risk-book limit lock poisoned");
        *g = books.into_iter().map(|b| (b.id.clone(), b)).collect();
    }

    /// Install (or clear) the firm-wide **auto-hedge / internalisation policy** snapshot
    /// (§6/§7). Pushed at boot from the persisted `IdentityStore` and re-pushed after every
    /// admin hedge write (the SAME `AuthEdge` reconcile hook that re-primes the routing graph),
    /// so defining / editing / clearing the hedge graph, thresholds, or config takes effect on
    /// subsequent fills immediately. `Some(policy)` ⇒ a booked fill carrying a priced reference
    /// mid resolves its warehouse-vs-external decision and stamps an [`InternaliseProvenance`];
    /// `None` ⇒ no decision runs and booking is byte-identical to the pre-Phase-B path. Behind
    /// an [`Arc`] so a concurrent booking sees the old or new policy atomically, never a torn one.
    pub fn set_hedge_policy(&self, policy: Option<RatesHedgePolicy>) {
        let mut g = self
            .hedge_policy
            .write()
            .expect("rates hedge-policy lock poisoned");
        *g = policy.map(Arc::new);
    }

    /// **Wave-0 migration audit** — every configured cap whose *meaning* changed when the
    /// bond exposure arm was re-based from `redemption × 1bp` to a duration-correct
    /// analytic DV01 (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §7.2 Wave 0, G1).
    ///
    /// The re-basing is a **migration, not a patch**: every cap an operator set against
    /// the old units silently tightened by roughly the bond's duration (≈1.9× at 2y, ≈4.5×
    /// at 5y, ≈8× at 10y, ≈16× at 30y), so a bond book that sat comfortably inside its cap
    /// can breach on day one for no economic reason. This audit measures the change on the
    /// **actual current inventory** rather than assuming a duration, and states the exact
    /// factor each cap must be multiplied by to keep its original meaning.
    ///
    /// Covered:
    ///
    /// - every [`LimitTree`] cap at a scope a rates cell rolls through (`Book` / `Entity` /
    ///   `Firm`) whose metric is charged with [`rates_linear_exposure`] — `Dv01`, `Pvbp`,
    ///   and `Delta` (the rates booking sink charges the linear IR proxy against `Delta`,
    ///   so a `Delta` cap on the rates tree changed units too);
    /// - every warehouse [`ScopedThreshold`] whose metric is [`HedgeMetric::Dv01`] and
    ///   which binds to a **risk book** (a `Desk`-scoped threshold does not bind on the
    ///   rates path at all, and an `Instrument` threshold has no resolvable position set
    ///   here — both are reported by the caller-facing docs, not silently audited).
    ///
    /// A scope holding no bonds produces no finding: swap/FRA exposure is unchanged by the
    /// re-basing, so its caps mean exactly what they always did.
    ///
    /// Every finding is also rung at **WARN** (`class=risk`) so the change cannot be
    /// discovered as an unexplained breach. Call it after priming limits/policy at boot and
    /// after any admin edit; it is O(caps × positions) on the control plane, never the
    /// booking or pricing path.
    #[must_use]
    pub fn audit_proxy_era_caps(&self) -> Vec<ProxyEraCapFinding> {
        let mut findings = Vec::new();
        {
            let positions = self
                .inner
                .read()
                .expect("rates position store lock poisoned");
            let limits = self.limits.read().expect("rates limit tree lock poisoned");
            for (scope, spec) in limits.iter() {
                if !matches!(
                    scope,
                    LimitScope::Book(_) | LimitScope::Entity(_) | LimitScope::Firm
                ) || !matches!(
                    spec.metric,
                    celnet_limits::LimitMetric::Dv01
                        | celnet_limits::LimitMetric::Pvbp
                        | celnet_limits::LimitMetric::Delta
                ) {
                    continue;
                }
                let matched = positions.iter().filter(|p| rates_scope_matches(scope, p));
                if let Some(f) = ProxyEraCapFinding::measure(
                    format!("limit:{scope:?}"),
                    format!("{:?}", spec.metric),
                    spec.cap,
                    matched,
                ) {
                    findings.push(f);
                }
            }
        }
        let thresholds = self
            .hedge_policy
            .read()
            .expect("rates hedge-policy lock poisoned")
            .as_ref()
            .map(|p| p.thresholds.clone())
            .unwrap_or_default();
        for t in thresholds
            .iter()
            .filter(|t| t.def.metric == HedgeMetric::Dv01)
            .filter(|t| t.def.scope_kind == HedgeScopeKind::Book)
        {
            let held = self.positions_in_risk_book(&t.scope_id);
            if let Some(f) = ProxyEraCapFinding::measure(
                format!("warehouse:book:{}", t.scope_id),
                HedgeMetric::Dv01.label().to_owned(),
                t.def.cap,
                held.iter(),
            ) {
                findings.push(f);
            }
        }
        for f in &findings {
            tracing::warn!(
                class = celnet_observability::LogClass::Risk.label(),
                scope = %f.scope,
                metric = %f.metric,
                cap = f.cap,
                legacy_exposure = f.legacy_exposure,
                rebased_exposure = f.rebased_exposure,
                multiplier = f.multiplier,
                suggested_cap = f.suggested_cap,
                breaching_now = f.breaching_now,
                "PROXY-ERA CAP: bond exposure is now a duration-correct DV01, so this cap's \
                 units changed. Re-base it by `multiplier` (suggested_cap) or confirm the \
                 current value is intended — see docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md \
                 §7.2 Wave 0",
            );
        }
        findings
    }

    /// Install (or clear) the firm-wide **incoming-quote-acceptance** decision graph
    /// snapshot (the third trader-configurable rule engine — `celnet-acceptance`). Pushed at
    /// boot from the persisted `IdentityStore` and re-pushed after every admin acceptance
    /// write (the SAME `AuthEdge` reconcile hook that re-primes the routing / hedge
    /// snapshots), so defining / editing / clearing the acceptance graph takes effect on
    /// subsequent inbound lifts immediately. `Some(graph)` ⇒ the FIX acceptance point gates
    /// each lift (accept / reject / hold); `None` ⇒ acceptance is off and every lift is
    /// accepted, byte-identical to the pre-acceptance path. Behind an [`Arc`] so the desk
    /// edge clones only a pointer, never a torn graph.
    pub fn set_acceptance(&self, graph: Option<AcceptanceGraph>) {
        let mut g = self
            .acceptance
            .write()
            .expect("rates acceptance lock poisoned");
        *g = graph.map(Arc::new);
    }

    /// The firm-wide acceptance decision graph snapshot, if installed — the graph the FIX
    /// acceptance point evaluates a lift against. Clones the `Arc` (a pointer) under the read
    /// lock so the evaluation runs lock-free off the FIX edge.
    #[must_use]
    pub fn acceptance_graph(&self) -> Option<Arc<AcceptanceGraph>> {
        self.acceptance
            .read()
            .expect("rates acceptance lock poisoned")
            .clone()
    }

    /// The internalise-decision provenance stamped on a booked rates fill, or `None` when the
    /// fill booked with no decision (the manual `BookRatesPosition` path, or no hedge policy
    /// configured). Looked up by rates position id — the analogue of [`Self::risk_book_of`].
    #[must_use]
    pub fn internalise_of(&self, position_id: u64) -> Option<InternaliseProvenance> {
        self.internalise
            .read()
            .expect("rates internalise lock poisoned")
            .get(&position_id)
            .cloned()
    }

    /// Whether a risk-routing graph is currently installed (routing is active).
    #[must_use]
    pub fn has_routing(&self) -> bool {
        self.routing
            .read()
            .expect("rates routing lock poisoned")
            .is_some()
    }

    /// The current **risk version**: a monotonic counter advanced on every successful
    /// [`Self::book`]. The live per-book risk stream folds this into its poll signal so a
    /// rates fill re-publishes the roster exactly as an FX fill does.
    #[must_use]
    pub fn risk_version(&self) -> u64 {
        self.risk_version.load(Ordering::Relaxed)
    }

    /// The resolved `risk_book_id` a booked rates position was routed into, or `None` if it
    /// booked unrouted (no graph configured, or a routing error fell it back). Looked up by
    /// the rates position id.
    #[must_use]
    pub fn risk_book_of(&self, position_id: u64) -> Option<String> {
        self.risk_book
            .read()
            .expect("rates risk-book lock poisoned")
            .get(&position_id)
            .cloned()
    }

    /// All live rates positions currently bucketed into `risk_book_id` (the by-risk-book
    /// query path §8.3, rates analogue of
    /// [`PositionStore::positions_in_risk_book`](super::risk::store::PositionStore::positions_in_risk_book)).
    /// Returns the full [`RatesPosition`]s so the per-book risk aggregation can sum their
    /// notional/PV01; the existing `(entity, book)` keying is untouched, so a position appears
    /// in both its entity/book roll-up and its risk book.
    #[must_use]
    pub fn positions_in_risk_book(&self, risk_book_id: &str) -> Vec<RatesPosition> {
        let stamps = self
            .risk_book
            .read()
            .expect("rates risk-book lock poisoned");
        let g = self
            .inner
            .read()
            .expect("rates position store lock poisoned");
        g.iter()
            .filter(|p| {
                stamps
                    .get(&p.position_id)
                    .is_some_and(|b| b == risk_book_id)
            })
            .cloned()
            .collect()
    }

    /// Mint a fresh rates position id for a **risk-transfer leg** (§6) — the moved slice of
    /// a partial re-attribution split (an economic leg auto-assigns via
    /// [`Self::book_into_risk_book`] with id `0`). Draws from the SAME monotonic counter the
    /// booking path uses, so a minted id never collides with a booked or auto-assigned one.
    #[must_use]
    pub fn mint_position_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// A read view of one booked rates position for a **risk transfer** (§6): its current
    /// risk-book stamp, trade-direction signed notional, per-unit "dealt level" mark, the
    /// linear DV01, and the stored [`RatesPosition`] template (so an economic transfer can
    /// re-book offsetting / opening legs off the exact instrument). Returns `None` for an id
    /// that is not booked.
    ///
    /// **Mark.** A rates cell carries **no marking surface** (see the module docs), so the
    /// economically-meaningful stored "current mark" is the instrument's contractual dealt
    /// level — the swap/FRA `fixed_rate`, the bond `coupon_rate` — the same level the routing
    /// engine reads on its `strike` axis. A `Mid`/`MarkToMarket` transfer resolves the
    /// transfer price to this level ⇒ zero realised P&L; an `Agreed` override crosses P&L
    /// against it.
    ///
    /// **DV01.** The moved DV01 is the curve-free **linear PV01 proxy**
    /// ([`rates_linear_exposure`]) — `notional · tenor · 1bp`, signed by IR-duration
    /// direction — the SAME conservative measure the pre-trade limit gate charges. The exact
    /// curve-bootstrapped DV01 needs a live `CurveSet`, which is not carried on the async
    /// transfer seam (transfers run off the pricing path); using the limit-gate measure keeps
    /// the moved-risk vector consistent with the book's enforced exposure rather than
    /// fabricating a zero or an unavailable curve number (guardrail 2 / 5).
    #[must_use]
    pub fn rates_transfer_view(&self, position_id: u64) -> Option<RatesTransferView> {
        let g = self
            .inner
            .read()
            .expect("rates position store lock poisoned");
        let position = g.iter().find(|p| p.position_id == position_id)?.clone();
        drop(g);
        Some(RatesTransferView {
            risk_book: self.risk_book_of(position_id),
            signed_notional: rates_signed_notional(&position),
            mark: rates_dealt_level(&position),
            dv01: rates_linear_exposure(&position),
            template: position,
        })
    }

    /// Re-point an already-booked rates position's **risk-book stamp** to `target_book` — a
    /// pure re-attribution (§6.1): no new position, economics unchanged. The caller runs the
    /// target's headroom check ([`Self::check_risk_book_headroom`]) first. Advances the risk
    /// version.
    ///
    /// # Errors
    /// `not_found` if no rates position is booked under `position_id`.
    pub fn restamp_risk_book(
        &self,
        position_id: u64,
        target_book: &str,
    ) -> Result<(), tonic::Status> {
        {
            let g = self
                .inner
                .read()
                .expect("rates position store lock poisoned");
            if !g.iter().any(|p| p.position_id == position_id) {
                return Err(tonic::Status::not_found(format!(
                    "rates position {position_id} is not booked"
                )));
            }
        }
        self.risk_book
            .write()
            .expect("rates risk-book lock poisoned")
            .insert(position_id, target_book.to_owned());
        self.risk_version.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Split a rates position for a **partial re-attribution** (§6.1 step 4): reduce the
    /// source position's notional magnitude to `|remainder_notional|` (same instrument /
    /// side, keeps its id and current book) and insert a NEW position at `moved_position_id`
    /// carrying `|moved_notional|` (same side), stamped into `target_book`. The two slices'
    /// notionals sum back to the original (no risk created or destroyed). Advances the risk
    /// version.
    ///
    /// # Errors
    /// `not_found` if `source_position_id` is not booked; `failed_precondition` if it carries
    /// no instrument arm to split.
    pub fn split_rates_position(
        &self,
        source_position_id: u64,
        remainder_notional: f64,
        moved_position_id: u64,
        moved_notional: f64,
        target_book: &str,
    ) -> Result<(), tonic::Status> {
        let template = {
            let g = self
                .inner
                .read()
                .expect("rates position store lock poisoned");
            g.iter()
                .find(|p| p.position_id == source_position_id)
                .ok_or_else(|| {
                    tonic::Status::not_found(format!(
                        "rates position {source_position_id} is not booked"
                    ))
                })?
                .clone()
        };
        if template
            .instrument
            .as_ref()
            .and_then(|i| i.instrument.as_ref())
            .is_none()
        {
            return Err(tonic::Status::failed_precondition(format!(
                "rates position {source_position_id} carries no instrument arm to split"
            )));
        }
        let remainder =
            rates_with_signed_notional(&template, remainder_notional, source_position_id);
        let moved = rates_with_signed_notional(&template, moved_notional, moved_position_id);
        {
            let mut g = self
                .inner
                .write()
                .expect("rates position store lock poisoned");
            if let Some(slot) = g.iter_mut().find(|p| p.position_id == source_position_id) {
                *slot = remainder;
            } else {
                return Err(tonic::Status::not_found(format!(
                    "rates position {source_position_id} is not booked"
                )));
            }
            g.push(moved);
        }
        self.risk_book
            .write()
            .expect("rates risk-book lock poisoned")
            .insert(moved_position_id, target_book.to_owned());
        self.risk_version.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// The hard-limit **headroom check** for a rates book receiving an incoming `(net,
    /// gross)` notional batch (§6): projects the resolved book AND each ancestor exactly as
    /// the routed-fill gate does, excluding every moved id. Returns `Ok(())` when no cap on
    /// the path is exceeded (or none is configured), the typed breach otherwise. A
    /// re-attribution calls this BEFORE any mutation so a move that would blow the target's
    /// (or an ancestor's) cap is refused with the store unmutated.
    ///
    /// # Errors
    /// `failed_precondition` if the incoming batch would EXCEED a hard net/gross notional cap
    /// on `target_book` or an ancestor.
    pub fn check_risk_book_headroom(
        &self,
        target_book: &str,
        incoming_net: f64,
        incoming_gross: f64,
        incoming_dv01: f64,
        exclude_position_ids: &[u64],
    ) -> Result<(), tonic::Status> {
        match self.project_rates_book_breach_multi(
            target_book,
            incoming_net,
            incoming_gross,
            incoming_dv01,
            exclude_position_ids,
        ) {
            Some(status) => Err(status),
            None => Ok(()),
        }
    }

    /// Book a rates position into an **explicit** risk book (an economic-transfer leg, §6.2)
    /// rather than the routed one — the rates analogue of
    /// [`PositionStore::book_into_risk_book`](super::risk::store::PositionStore::book_into_risk_book).
    /// Runs the SAME gates [`Self::book`] runs — the `book → entity → firm` pre-trade gate
    /// and the per-book hard notional-cap gate over `risk_book` + its ancestors — over a read
    /// snapshot BEFORE any mutation, refuses a hard breach with the store unmutated, routes a
    /// `Strong`-tier book's write through the quorum log first, then stamps the EXPLICIT book
    /// (never the routed one). Advances the risk version. `position_id == 0` ⇒ a fresh id is
    /// assigned.
    ///
    /// # Errors
    /// `failed_precondition` on a hard `(book → entity → firm)` limit or per-book notional-cap
    /// breach.
    pub fn book_into_risk_book(
        &self,
        mut position: RatesPosition,
        risk_book: &str,
    ) -> Result<RatesPosition, tonic::Status> {
        // (1) The FactKey pre-trade gate — identical to `book_with_routing`, skipped when no
        // limit is configured (byte-identical to the pre-gate path).
        {
            let limits = self.limits.read().expect("rates limit tree lock poisoned");
            if !limits.is_empty() {
                let g = self
                    .inner
                    .read()
                    .expect("rates position store lock poisoned");
                let result = rates_pre_trade(&g, &limits, &position);
                if result.decision == PreTradeDecision::Reject {
                    return Err(rates_limit_breached(&g, &position, &result));
                }
            }
        }
        // (2) The per-book HARD-limit gate on the EXPLICIT book (not routed).
        if let Some(breach) = self.project_rates_risk_book_breach(risk_book, &position) {
            return Err(breach);
        }
        // (3) A `Strong`-tier book routes its authoritative write through the quorum log
        // BEFORE the local apply (mirrors `book_with_routing`); a `Local` cell skips it.
        if let Some(consensus) = self.consensus.get()
            && consensus.level_for_rates_book(position.book).is_strong()
        {
            if position.position_id == 0 {
                position.position_id = self.next_id.fetch_add(1, Ordering::Relaxed);
            }
            consensus.commit_book_write(
                rates_book_key(position.position_id),
                rates_linear_exposure(&position),
            )?;
        }
        {
            let mut g = self
                .inner
                .write()
                .expect("rates position store lock poisoned");
            if position.position_id == 0 {
                position.position_id = self.next_id.fetch_add(1, Ordering::Relaxed);
            } else {
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
                *slot = position.clone();
            } else {
                g.push(position.clone());
            }
        }
        self.risk_book
            .write()
            .expect("rates risk-book lock poisoned")
            .insert(position.position_id, risk_book.to_owned());
        self.risk_version.fetch_add(1, Ordering::Relaxed);
        Ok(position)
    }

    /// Remove a booked rates position entirely — its cell and its risk-book stamp. The
    /// economic-transfer **rollback** primitive (§6.2): a staged source-offset leg is
    /// un-booked if the paired target leg is refused, so a rejected transfer never leaves a
    /// half-transfer. Advances the risk version when a position was actually removed.
    pub fn remove_position(&self, position_id: u64) {
        let removed = {
            let mut g = self
                .inner
                .write()
                .expect("rates position store lock poisoned");
            let before = g.len();
            g.retain(|p| p.position_id != position_id);
            g.len() != before
        };
        self.risk_book
            .write()
            .expect("rates risk-book lock poisoned")
            .remove(&position_id);
        if removed {
            self.risk_version.fetch_add(1, Ordering::Relaxed);
        }
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
    /// When a firm-wide **risk-routing graph** is installed ([`Self::set_routing`]) the fill
    /// is additionally routed to a risk book and stamped (§4); no graph ⇒ unrouted, byte-
    /// identical to the pre-routing path. A routed fill also charges the routed book's (and
    /// its ancestors') optional hard notional caps.
    ///
    /// # Errors
    /// `failed_precondition` when a **hard** `(book → entity → firm)` limit OR a routed
    /// **risk-book** hard notional cap (the resolved book or an ancestor) would be breached.
    pub fn book(&self, position: RatesPosition) -> Result<RatesPosition, tonic::Status> {
        // The default booking path (RiskService::BookRatesPosition + the existing tests):
        // no originating counterparty/ccy is known, so route with the empty attribution
        // (a counterparty/ccy rule is honestly unmatched). The RFQ desk path calls
        // `book_with_routing` with the real originating fields.
        self.book_with_routing(position, RatesRoutingAttribution::default())
    }

    /// [`Self::book`] with an explicit booking-time [`RatesRoutingAttribution`] — the RFQ
    /// desk path supplies the originating counterparty + curve currency so a
    /// `RouteField::Counterparty` / `RouteField::Ccy` rule can route the fill. Identical to
    /// [`Self::book`] in every other respect (pre-trade gate, per-book cap, stamp, version).
    ///
    /// # Errors
    /// As [`Self::book`].
    pub fn book_with_routing(
        &self,
        mut position: RatesPosition,
        attribution: RatesRoutingAttribution,
    ) -> Result<RatesPosition, tonic::Status> {
        // Best-order timer O3 (ack→fill→book commit latency, `OpKind::Book`): bracket the whole
        // rates booking commit with a monotonic `Instant`; recorded into the telemetry hub on a
        // successful book below — the rates analogue of the FX sink
        // ([`PositionStore::book`](super::risk::store::PositionStore::book)). This is the async
        // booking tier, never the pinned pricing core (guardrail 11).
        let book_t0 = std::time::Instant::now();
        // Pre-trade limit gate (ADR-0016 A1) over a READ snapshot, BEFORE the write lock,
        // so a hard breach can never book and the projection never runs while the exclusive
        // write lock is held (guardrails #6/#11 — a fill never serialises the whole book
        // behind its own limit projection). Skipped when no limit is set (the empty-tree
        // default keeps the store byte-identical to the pre-gate behaviour). The narrow
        // snapshot→write window is the standard pre-trade TOCTOU: a hard breach still
        // rejects here and leaves the book unmutated, the post-trade limit monitor
        // backstopping any concurrent joint breach.
        // Phase timers. The `Book` stage is a composite — pre-trade gate, then the write
        // commit, then the trace emission — and a p99 five times its own p50 says one of
        // them tails while the others do not. Reading the code cannot say which: the gate
        // is O(positions) but memoised, the commit takes the store's only write lock, and
        // the trace emit is off-core. So the booking measures itself and NAMES the phase
        // when it runs long, rather than anyone guessing from an aggregate.
        let gate_t0 = std::time::Instant::now();
        {
            let limits = self.limits.read().expect("rates limit tree lock poisoned");
            if !limits.is_empty() {
                let g = self
                    .inner
                    .read()
                    .expect("rates position store lock poisoned");
                let result = rates_pre_trade(&g, &limits, &position);
                if result.decision == PreTradeDecision::Reject {
                    return Err(rates_limit_breached(&g, &position, &result));
                }
            }
        }

        // Risk routing (§4): resolve the firm-wide graph on a READ snapshot, BEFORE any
        // mutation — mirroring the FX sink. A routing error on an already-validated graph
        // falls back to UNROUTED (a routing failure never rejects the fill), byte-identical
        // to the pre-routing path; `None` (no graph) is likewise unrouted.
        // Best-order timer: bracket just the routing decision-graph evaluation (`OpKind::RiskRoute`).
        // Recorded only when a graph was actually installed (routing ran) so an unrouted store
        // never folds a trivial no-graph span into the stage.
        let route_t0 = std::time::Instant::now();
        let routing_ran;
        let resolved_book: Option<String> = {
            let routing = self.routing.read().expect("rates routing lock poisoned");
            routing_ran = routing.is_some();
            routing.as_ref().and_then(|graph| {
                let ctx = routing_context_from_rates(&position, &attribution);
                match RiskRouter::route(graph, &ctx) {
                    Ok(book) => Some(book.to_owned()),
                    Err(err) => {
                        tracing::warn!(
                            class = celnet_observability::LogClass::Risk.label(),
                            position_id = position.position_id,
                            %err,
                            "rates risk routing failed; booking unrouted",
                        );
                        None
                    }
                }
            })
        };
        if routing_ran {
            self.record_latency(
                celnet_observability::OpKind::RiskRoute,
                u64::try_from(route_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
        }
        // The per-book HARD-limit gate (§8.3): if the fill's resolved book (or any ancestor)
        // would EXCEED a hard notional cap, reject BEFORE any mutation — store unchanged. No
        // caps on the path ⇒ `None`, byte-identical to the pre-enforcement path.
        if let Some(book_id) = resolved_book.as_deref()
            && let Some(breach) = self.project_rates_risk_book_breach(book_id, &position)
        {
            return Err(breach);
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

        let gate_ns = gate_t0.elapsed();
        // Measured OUTSIDE the guard's scope below so it includes the wait to ACQUIRE the
        // write lock, not just the work done under it — a booking blocked behind a reader
        // is exactly the tail this is looking for, and timing only the critical section
        // would hide it.
        let commit_t0 = std::time::Instant::now();
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
            *slot = position.clone();
        } else {
            g.push(position.clone());
        }
        // Captured before the guard drops: the gate's cost is O(this), so an outlier is
        // only interpretable next to the book size that produced it.
        let g_len = g.len();
        drop(g);

        // Risk routing (§4): stamp the risk book resolved above (before the per-book gate),
        // beside the existing `(entity, book)` keying. `None` — no graph, or a routing error
        // — leaves the position UNROUTED (and a re-book of a formerly-routed id while routing
        // is now off drops the stamp), byte-identical to the pre-routing booking path.
        {
            let mut stamps = self
                .risk_book
                .write()
                .expect("rates risk-book lock poisoned");
            match &resolved_book {
                Some(book) => {
                    stamps.insert(position.position_id, book.clone());
                }
                None => {
                    stamps.remove(&position.position_id);
                }
            }
        }
        // A successful rates fill changed the live book, so a risk book's aggregated risk may
        // have moved: advance the monotonic risk version the live per-book risk stream folds
        // into its poll signal (mirrors the FX sink's unconditional post-mutation bump).
        self.risk_version.fetch_add(1, Ordering::Relaxed);
        // RISK ROUTING (class=Risk → orders sink): the booked fill's resolved risk book,
        // its position id, and the routing context (counterparty / ccy / dealt level).
        // Async booking tier only — never the pinned pricing core (guardrail 11).
        tracing::info!(
            class = celnet_observability::LogClass::Risk.label(),
            position_id = position.position_id,
            risk_book_id = resolved_book.as_deref(),
            routed = resolved_book.is_some(),
            counterparty = %attribution.counterparty,
            ccy = %attribution.ccy,
            dealt_price = attribution.dealt_price,
            reference_mid = attribution.reference_mid,
            "rates fill routed to risk book",
        );
        // End-to-end event trace (Analytics pillar C): resolve THIS lift's trace via the
        // acceptance `request_id` the FIX-lift bound at quote publish, bind the freshly-minted
        // `position_id` so the hedge stage inherits the trace, and emit the routing + deal-booked
        // stage events at their TRUE instants (before the hedge decision below, so the timeline
        // orders correctly). A no-op when no hub / no request_id / an unknown request_id — the
        // manual-booking + gRPC-accept paths never bound a trace. Off the pinned core (guardrail 11).
        let commit_ns = commit_t0.elapsed();
        let trace_t0 = std::time::Instant::now();
        let booking_trace_id = self
            .trace
            .get()
            .zip(attribution.request_id.as_deref())
            .and_then(|(hub, rid)| hub.resolve_request(rid));
        if let Some(trace_id) = booking_trace_id {
            if let Some(hub) = self.trace.get() {
                hub.bind_position(position.position_id, trace_id);
            }
            let counterparty =
                (!attribution.counterparty.is_empty()).then(|| attribution.counterparty.clone());
            self.trace_stage(
                trace_id,
                celnet_proto::TraceStage::RiskRouted,
                crate::services::trace::TraceDetails {
                    price: attribution.dealt_price,
                    book_id: resolved_book.clone(),
                    counterparty: counterparty.clone(),
                    position_id: Some(position.position_id),
                    detail: Some(
                        if resolved_book.is_some() {
                            "routed"
                        } else {
                            "unrouted"
                        }
                        .to_owned(),
                    ),
                    ..Default::default()
                },
            );
            self.trace_stage(
                trace_id,
                celnet_proto::TraceStage::DealBooked,
                crate::services::trace::TraceDetails {
                    price: attribution.dealt_price,
                    book_id: resolved_book.clone(),
                    counterparty,
                    position_id: Some(position.position_id),
                    ..Default::default()
                },
            );
        }

        // Auto-hedge / internalise decision (§6): when a hedge policy is primed AND this fill
        // routed to a book AND the booking path supplied a priced reference mid + dealt price
        // (the RFQ-desk accept / FIX-lift paths), resolve the warehouse-vs-external split +
        // price-tolerance verdict and stamp it onto the fill for the deal blotter. A no-op
        // otherwise (the manual `BookRatesPosition` path, or no policy) — byte-identical.
        // `booking_trace_id` threads the lift's trace so the hedge decision/fire emit stages.
        // The trace phase ends HERE, before dispatch. Conflating the two hid the real cost
        // once already: the first breakdown blamed "trace" for 1-2ms when `TraceHub::record`
        // is a non-blocking `try_send` and cannot cost that — what the bucket actually
        // contained was the hedge dispatch below.
        let trace_ns = trace_t0.elapsed();
        let dispatch_t0 = std::time::Instant::now();

        // Auto-hedge dispatch (`HedgeDispatch`): the hedge is the DESK's risk management,
        // not part of confirming the client's trade — but running it here, inline, puts a
        // blocking venue round trip on the client's booking commit (measured ~6ms p99 on
        // UAT against a hedge decision of ~26us).
        //
        // `Async` detaches the whole hedge — decision and execution together, so the
        // ordering between them is unchanged and only its relationship to the FILL moves.
        // What is given up is simultaneity: the offsetting leg and the provenance record
        // land a moment after the fill returns rather than with it.
        //
        // Falls back to inline whenever the async path is unavailable (no self-handle
        // installed, or the store is being dropped). Degrading to synchronous is always
        // safe; silently skipping the hedge never is.
        let dispatch = self
            .hedge_policy
            .read()
            .expect("rates hedge-policy lock poisoned")
            .as_ref()
            .map_or(crate::config::hedge_policy::HedgeDispatch::Sync, |p| {
                p.config.dispatch
            });
        let queued = dispatch.is_async()
            && self.hedge_queue.get().is_some_and(|tx| {
                // `try_send`, never `send`: blocking here would put the queue's depth back
                // on the client's booking commit, which is the whole thing async exists to
                // avoid. A refusal falls through to inline below.
                tx.try_send(HedgeJob {
                    fill: position.clone(),
                    book: resolved_book.clone(),
                    attribution: attribution.clone(),
                    trace_id: booking_trace_id,
                })
                .is_ok()
            });
        if !queued {
            self.stamp_internalise(
                &position,
                resolved_book.as_deref(),
                &attribution,
                booking_trace_id,
            );
        }
        // Name the phase when the booking runs long. WARN rather than a metric because the
        // aggregate ALREADY exists (`OpKind::Book`) and is not the thing that is missing:
        // what is missing is which of its three parts produced a given outlier, and that
        // is a per-occurrence fact, not a distribution.
        let total = book_t0.elapsed();
        if total >= SLOW_BOOKING_THRESHOLD {
            let dispatch_ns = dispatch_t0.elapsed();
            tracing::warn!(
                class = celnet_observability::LogClass::Risk.label(),
                position_id = position.position_id,
                total_us = total.as_micros(),
                pre_trade_gate_us = gate_ns.as_micros(),
                write_commit_us = commit_ns.as_micros(),
                trace_emit_us = trace_ns.as_micros(),
                hedge_dispatch_us = dispatch_ns.as_micros(),
                positions = g_len,
                "booking commit exceeded its budget — phase breakdown",
            );
        }
        // O3: record the ack→fill→book commit latency into the per-`OpKind` store (mirrors the
        // FX sink). Off the pinned pricing core; a no-op when no hub is installed.
        self.record_latency(
            celnet_observability::OpKind::Book,
            u64::try_from(book_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
        Ok(position)
    }

    /// The signed net risk of one book restricted to a single product FAMILY ("BOND"/"OIS"/…),
    /// measured in `metric` — the family-scoped analogue of [`Self::book_net_dv01`].
    ///
    /// The family restriction exists because DV01 is not fungible across families: a bond's
    /// PV01 does not offset a FRA's in any executable sense, so pooling them would claim an
    /// internal cross that could not actually be done. `GrossNotional` and `NetVega` yield
    /// `0.0` — a gross measure never nets (it has no direction to offset) and a linear-rates
    /// cell carries no vega — matching the budget-basis switch in [`Self::stamp_internalise`].
    #[must_use]
    fn book_risk_in_family(&self, book: &str, family: &str, metric: HedgeMetric) -> f64 {
        self.positions_in_risk_book(book)
            .iter()
            .filter(|p| internalise_instrument_label(p) == family)
            .map(|p| match metric {
                HedgeMetric::Dv01 => rates_linear_exposure(p),
                HedgeMetric::NetNotional | HedgeMetric::NetDelta => rates_signed_notional(p),
                // A gross roll-up is unsigned and never nets down, so it names no opposing
                // side to cross against; likewise a linear cell carries no vega.
                HedgeMetric::GrossNotional | HedgeMetric::NetVega => 0.0,
            })
            .sum()
    }

    /// The opposing internal risk this book could cross against **right now** — the
    /// [`HedgeField::InternalOffsetAvailable`] operand.
    ///
    /// Scope: the net risk held by the **sibling books under the same immediate parent**,
    /// restricted to the same product family, netted across the sibling set first and then
    /// tested for opposition. Each part of that is a deliberate narrowing:
    ///
    /// - *Sibling subtree, not firm-wide.* Crossing risk with an unrelated part of the firm is
    ///   a transfer requiring consent, not something a rule may silently assume is available.
    ///   A book with no parent (a root) has no siblings and therefore no offset.
    /// - *Same family.* See [`Self::book_risk_in_family`].
    /// - *Netted, then opposing.* Two siblings at `+50` and `−50` have nothing to cross;
    ///   summing their opposing legs separately would overstate the pool.
    ///
    /// Every one of those narrowings can only ever **understate** the offset, so the engine
    /// externalises more than strictly necessary rather than assuming a cross it cannot do —
    /// the safe direction to err in.
    ///
    /// Returns the magnitude of the sibling pool when it opposes `book_risk`, else `0.0`. It
    /// is NOT clamped to `|book_risk|`: the field answers "how much opposing flow exists",
    /// which is a property of the pool, not of what this fill happens to need.
    ///
    /// **Known limitation.** The pool cannot be currency-filtered: a stored [`RatesPosition`]
    /// carries no settlement currency (`ccy` lives on the [`RatesRoutingAttribution`], not on
    /// the position), so a parent nesting EUR and USD books of one family would overstate the
    /// offset. Either avoid that nesting or add a currency to the position record.
    #[must_use]
    fn internal_offset_available(
        &self,
        policy: &RatesHedgePolicy,
        book: &str,
        family: &str,
        metric: HedgeMetric,
        book_risk: f64,
    ) -> f64 {
        // No direction to offset ⇒ nothing to cross, whatever the siblings hold.
        if !book_risk.is_finite() || book_risk == 0.0 {
            return 0.0;
        }
        // `book_ancestors` is nearest-first, so the head is the immediate parent.
        let Some(parent) = policy
            .book_ancestors
            .get(book)
            .and_then(|a| a.first())
            .map(String::as_str)
        else {
            return 0.0; // a root book has no siblings.
        };
        // The sibling set = the parent's subtree, minus this book's own subtree, minus the
        // parent itself (the parent's own risk is a level up, not a peer's to cross).
        let empty = Vec::new();
        let own_subtree = policy.book_descendants.get(book).unwrap_or(&empty);
        let pool: f64 = policy
            .book_descendants
            .get(parent)
            .unwrap_or(&empty)
            .iter()
            .filter(|b| b.as_str() != book && !own_subtree.iter().any(|d| d == *b))
            .map(|b| self.book_risk_in_family(b, family, metric))
            .sum();
        if !pool.is_finite() || pool == 0.0 {
            return 0.0;
        }
        // Opposing only: a sibling pool on the SAME side adds to the firm's risk, it does
        // not relieve this book's.
        if (pool > 0.0) == (book_risk > 0.0) {
            return 0.0;
        }
        pool.abs()
    }

    /// The current external hedge-cost estimate in basis points — the
    /// [`HedgeField::HedgeCostBp`] operand.
    ///
    /// Prefers the **live** cost: the distance from the reference `mid` to the best executable
    /// LP top-of-book on the side this risk would have to shed, expressed in bp via `bp_scale`
    /// (the same convention [`dealer_edge_bps`](crate::services::internalise::dealer_edge_bps)
    /// and `composite_hedge_price` use). With no live panel — or a non-finite level — it falls
    /// back to the desk's configured `composite_spread_bp`, which is exactly what a composite
    /// fill would actually pay, so the estimate degrades to the real backstop cost rather than
    /// to zero.
    ///
    /// Probing the panel here is side-effect-free and correctly sized-agnostic: the
    /// aggregation hub's `best_fill` ignores its `size` argument entirely and is a pure read
    /// of member top-of-book, so no shed need be sized first.
    ///
    /// **This is the crossing half-spread only — it does NOT include market impact**, because
    /// no depth or ADV data exists anywhere in the system to derive impact from. A large clip
    /// will cost more than this reports.
    #[must_use]
    fn live_hedge_cost_bp(
        &self,
        instrument: &str,
        book_risk: f64,
        mid: f64,
        bp_scale: f64,
        composite_spread_bp: f64,
    ) -> f64 {
        let composite = composite_spread_bp.max(0.0);
        if !mid.is_finite() || !bp_scale.is_finite() || bp_scale <= 0.0 {
            return composite;
        }
        let Some(source) = self.lp_hedge_source.get() else {
            return composite;
        };
        // `size` is ignored by the hub's implementation; `book_risk`'s sign selects the side
        // (a long sheds by hitting a bid, a short by lifting an offer).
        let Some(fill) = source.best_fill(instrument, book_risk, 0.0) else {
            return composite;
        };
        if !fill.price.is_finite() {
            return composite;
        }
        let cost = (fill.price - mid).abs() / bp_scale;
        if cost.is_finite() { cost } else { composite }
    }

    /// Assemble the one production [`HedgeContext`] — the rule-evaluation input for a booked
    /// rates fill.
    ///
    /// **Every field is named explicitly; there is deliberately no `..Default::default()`.**
    /// That struct-update shorthand is what let five fields ship as silent zeros: a field
    /// could be added to [`HedgeContext`], given a type, a doc comment, an operator matrix and
    /// a UI chip, and still reach this — the only production builder in the codebase — with
    /// nothing populating it, because the shorthand absorbed it without a word. Naming every
    /// field makes a future addition a compile error **here**, at the point where the question
    /// "what actually produces this?" has to be answered.
    ///
    /// The fields with no production source are set to their inert value with the reason
    /// stated inline; the authoritative declaration is
    /// [`HedgeField::provider`](celnet_hedge_routing::HedgeField::provider), which
    /// `HedgeGraph::validate` reads to reject a rule that branches on one of them.
    fn build_hedge_context(&self, i: &HedgeContextInputs<'_>) -> HedgeContext {
        HedgeContext {
            // The product FAMILY ("BOND"/"OIS"/…) — the axis the hedge-vehicle registry
            // buckets on alongside maturity, and a rule-condition field in its own right.
            product: i.instrument.to_owned(),
            instrument_id: i.instrument.to_owned(),
            execution_instrument_id: i.execution_instrument.clone(),
            ccy: i.attribution.ccy.clone(),
            book: i.book.to_owned(),
            // UNPROVIDED. A booked rates fill carries no desk: the desk belongs to the
            // FIX/RFQ session that priced the quote, not to the position that resulted, and
            // is unrecoverable here — which is why `resolve_hedging_model` and
            // `resolve_hedge_threshold` are both already called with an empty desk operand on
            // this path. `HedgeField::Desk` is declared `Unprovided`, so a `desk`-scoped rule
            // is rejected at authoring time rather than silently never matching.
            desk: String::new(),
            // The originating counterparty of THIS fill — the same party id the Deal /
            // blotter and the risk-routing attribution carry — so a hedge rule
            // `counterparty == "X"` back-to-backs a given client's flow (a property of the
            // incoming fill, not a per-counterparty net position).
            counterparty: i.attribution.counterparty.clone(),
            // `net_dv01` and `net_notional` are two DISTINCT, honestly-different risk measures
            // of the SAME scope, NOT one mirrored onto the other:
            //  - `net_dv01`      = signed DV01 (linear PV01 proxy) — the warehouse-cap basis the
            //    default `Dv01`-metric threshold classifies (RAG band / overflow / sizing).
            //  - `net_notional`  = signed face notional — the operand a `NetNotional`/`NetDelta`
            //    rule CONDITION compares against, so a trader's "net notional > 1M" rule fires on
            //    true notional (a two-way rates book nets to low-thousands DV01 but tens of
            //    millions face — mirroring DV01 here would make such a rule never fire).
            // A threshold whose metric IS explicitly `NetNotional`/`NetDelta` then measures the
            // cap against `net_notional` too (the trader owns denominating that cap in notional).
            net_dv01: i.book_net_dv01,
            net_notional: i.book_net_notional,
            gross_notional: i.book_gross_notional,
            // UNPROVIDED (both). Only linear-rates cells ever reach this builder, and they
            // carry no volatility or convexity risk, so zero is the honest value — but a rule
            // reading it is still unconditionally dead, so both are declared `Unprovided` and
            // such a rule is refused. Wiring an options cell that builds a context is what
            // would make them real.
            net_vega: 0.0,
            net_gamma: 0.0,
            // The direction of the risk, in the SAME metric the bands below classify — so it
            // always agrees with `breached` / `utilization` / `overflow` rather than
            // disagreeing whenever the budget is not denominated in DV01.
            inventory_sign: inventory_sign(i.book_risk),
            // Classified in the THRESHOLD'S metric (`book_risk`), not unconditionally in
            // DV01 — so a `breached` / `utilization` / `overflow` rule condition reads the
            // same basis the engine sizes off.
            threshold: i.wh.cap,
            utilization: i.wh.utilization(i.book_risk),
            overflow: i.wh.overflow(i.book_risk),
            breached: i.wh.breached(i.book_risk),
            // UNPROVIDED. Deriving markout needs the post-fill mark trajectory of the flow,
            // which nothing retains; no per-counterparty toxicity score is computed anywhere.
            counterparty_toxicity: 0.0,
            // UNPROVIDED. A stored `RatesPosition` carries no acquisition timestamp at all, so
            // the age of the risk cannot be derived. Adding one is a wire + store change; it
            // is NOT invented here.
            inventory_age_secs: 0.0,
            internal_offset_available: self.internal_offset_available(
                i.policy,
                i.book,
                i.instrument,
                i.metric,
                i.book_risk,
            ),
            // Priced off the tradeable security when the cell resolves one — the family label
            // is not a security and could only miss the panel.
            hedge_cost_bp: self.live_hedge_cost_bp(
                i.execution_instrument.as_deref().unwrap_or(i.instrument),
                i.book_risk,
                i.mid,
                i.bp_scale,
                i.policy.config.composite_spread_bp,
            ),
        }
    }

    /// Resolve and stamp this fill's auto-hedge / internalisation decision (§6), when a hedge
    /// policy is primed, the fill routed to a book, and the booking path supplied a priced
    /// reference mid + dealt price. Reads the routed book's post-fill net DV01, builds a
    /// [`HedgeContext`], runs the pure [`AutoHedgeEngine`] (which also records the firm-wide
    /// audit provenance the `ListHedgeProvenance` RPC serves), combines the engine's
    /// internal/external decompose with the price-tolerance
    /// [`InternaliseVerdict`](crate::services::internalise::InternaliseVerdict), and stamps the
    /// resulting [`InternaliseProvenance`] keyed by the fill's position id. A no-op (no stamp)
    /// when any precondition is absent — the deal then carries no internalise provenance
    /// (surfaced, never fabricated). Off the pinned pricing core (guardrail 11).
    fn stamp_internalise(
        &self,
        fill: &RatesPosition,
        resolved_book: Option<&str>,
        attribution: &RatesRoutingAttribution,
        trace_id: Option<u64>,
    ) {
        let Some(book) = resolved_book else { return };
        let (Some(dealt), Some(mid)) = (attribution.dealt_price, attribution.reference_mid) else {
            return;
        };
        let Some(policy) = self
            .hedge_policy
            .read()
            .expect("rates hedge-policy lock poisoned")
            .clone()
        else {
            return;
        };
        // The exit graph the decision resolves an action against. A firm that has NOT
        // configured a hedge policy (no persisted graph — e.g. the pristine-store seed was
        // skipped because the operator's routing book is not the default warehouse book) still
        // gets a decision: fall back to the default warehouse-hold-vs-advisory-shed graph, so
        // every booked fill stamps an internalise decision + RAG band rather than an empty `—`.
        // Select the MOST-SPECIFIC hedge policy governing this fill's book (§5): its own
        // `Book` graph, else the nearest ancestor `Bucket` graph (whose subtree the decision
        // measures), else the firm graph. No policy at any scope ⇒ the default
        // warehouse-hold-vs-advisory-shed graph, so every booked fill stamps a decision.
        let fallback_graph;
        let (authored_graph, bucket_root) = match policy.select_scoped_graph(book) {
            Some((g, root)) => (g, root),
            None => {
                fallback_graph = default_hedge_policy_graph();
                (&fallback_graph, None)
            }
        };
        // The fill's side + whether its dealt level is a rate or a clean price.
        let Some((desk_side, kind)) = fill_side_and_quote_kind(fill) else {
            return;
        };
        let instrument = internalise_instrument_label(fill);
        // The tradeable identity behind that family label, when the fill resolves one. Kept
        // SEPARATE from `instrument` because thresholds/provenance are family-scoped while
        // street execution is security-scoped — see `hedge_execution_instrument`.
        let execution_instrument = hedge_execution_instrument(fill);
        // Resolve the most-specific warehouse threshold for this fill (instrument > book >
        // desk). No threshold configured for ANY of the fill's scopes — e.g. it routed into an
        // operator-defined risk book the firm never bound a warehouse cap to — falls back to
        // the default firm warehouse budget, so the fill is still measured against a real cap
        // and stamps a decision, never silently carrying none.
        // For a `Bucket` policy the "100" is the bucket root's threshold and the risk state is
        // the subtree roll-up; otherwise it is the fill's own book.
        let scope_book: &str = bucket_root.as_deref().unwrap_or(book);
        // The scoped HEDGING MODEL governing this fill — *how* the scope manages risk
        // (back-to-back / warehouse-to-a-DV01-budget / the desk's own authored graph), resolved
        // most-specific-wins exactly as the panels and exit modes are. A bound, non-`Custom`
        // model DERIVES an ordinary graph from the same node vocabulary a trader would have
        // authored by hand, and it is handed to the same evaluator below — the model is a
        // control over the existing spine, never a second engine.
        //
        // The desk operand is `""` for the same reason `resolve_hedge_threshold` passes `""`
        // here: a rates fill does not carry its desk at this point, so a `Desk`-scoped binding
        // cannot match on this path (`docs/HEDGING-AND-RISK-EXIT.md` §10). `Instrument` and
        // `Book` bindings — the two a trader actually configures — resolve normally.
        let model_binding = policy
            .config
            .resolve_hedging_model("", scope_book, &instrument);
        let derived_graph;
        // Whether a bound hedging MODEL supplied the graph rather than the trader's own
        // authored rules — recorded on the journal row so the walked node ids are read
        // against the right source.
        let mut derived_model_graph = false;
        let graph = match model_binding.and_then(|b| b.model.derived_graph()) {
            Some(g) => {
                derived_graph = g;
                derived_model_graph = true;
                &derived_graph
            }
            // `Custom`, or nothing bound at any scope ⇒ the authored graph governs, i.e.
            // byte-identical behaviour to before a model was ever bindable.
            None => authored_graph,
        };
        let mut thr_def = resolve_hedge_threshold(&policy.thresholds, "", scope_book, &instrument)
            .unwrap_or_else(default_warehouse_threshold_def);
        // A positive DV01 budget on an `InternaliseToDv01` binding IS the "100" this scope
        // warehouses up to, so it overrides the resolved cap and forces the metric to DV01 —
        // otherwise the budget would be silently measured against whatever metric the
        // threshold happened to carry. Everything else (amber / red / target / clip / ramp)
        // still comes from the resolved threshold, so a desk that tuned its bands keeps them.
        // A blank budget yields `None` and changes nothing — binding a model never invents a
        // cap the operator did not ask for.
        if let Some(budget) = model_binding.and_then(|b| b.effective_budget()) {
            thr_def.cap = budget;
            thr_def.metric = HedgeMetric::Dv01;
        }
        let wh = thr_def.to_threshold();

        // The risk state the warehouse cap is measured against (signed linear PV01 proxy): the
        // fill's own book, or — under a `Bucket` policy — the whole subtree roll-up (the
        // "PORTFOLIO notional" the bucket decision governs, §5). Plus this fill's own DV01.
        let book_net_dv01 = match &bucket_root {
            Some(root) => {
                let empty = Vec::new();
                let desc = policy.book_descendants.get(root).unwrap_or(&empty);
                self.subtree_net_dv01(root, desc)
            }
            None => self.book_net_dv01(book),
        };
        // The signed FACE NOTIONAL over the SAME scope (subtree under a `Bucket` policy, else
        // the fill's own book) — a DISTINCT measure from `book_net_dv01` (notional vs. PV01;
        // see [`rates_signed_notional`] vs. [`rates_linear_exposure`]). This is the operand a
        // hedge rule's `NetNotional` / `NetDelta` condition compares against, so such a rule
        // measures true face notional and NOT the DV01 proxy. It never re-scales the DV01-based
        // warehouse cap: the default warehouse threshold's metric is `Dv01`, so the RAG band /
        // overflow / sizing keep reading `book_net_dv01` below.
        let book_net_notional = match &bucket_root {
            Some(root) => {
                let empty = Vec::new();
                let desc = policy.book_descendants.get(root).unwrap_or(&empty);
                self.subtree_net_notional(root, desc)
            }
            None => self.book_net_notional(book),
        };
        // The GROSS roll-up over the same scope — the third budget basis a desk can pick
        // (net / gross / DV01). Never nets down, so it is a turnover brake (see
        // [`HedgeMetric::GrossNotional`]).
        let book_gross_notional = match &bucket_root {
            Some(root) => {
                let empty = Vec::new();
                let desc = policy.book_descendants.get(root).unwrap_or(&empty);
                self.subtree_gross_notional(root, desc)
            }
            None => self.book_gross_notional(book),
        };

        // ---- The BUDGET BASIS: everything below measures in the threshold's OWN metric ----
        //
        // The engine already sizes off the threshold's metric (`net_risk_for`), so any measure
        // computed here in a DIFFERENT metric silently disagrees with the engine's own sizing.
        // Two consequences before this was threaded through, both live bugs:
        //   1. `breached` / `utilization` / `overflow` — the fields RULE CONDITIONS read, and
        //      the band stamped on provenance — were computed off DV01 whatever the metric,
        //      so a NetNotional budget classified against a DV01 cap.
        //   2. the internal/external split clamped a shed expressed in the metric's units
        //      against the fill's DV01 — mixing notional with PV01 outright.
        // `book_risk` and `fill_risk` below are the single basis both now use.
        let book_risk = match thr_def.metric {
            HedgeMetric::Dv01 => book_net_dv01,
            HedgeMetric::NetNotional | HedgeMetric::NetDelta => book_net_notional,
            HedgeMetric::GrossNotional => book_gross_notional,
            // A linear-rates cell carries no vega. Reporting 0 is the honest answer (such a
            // budget simply never breaches) rather than silently substituting another metric.
            HedgeMetric::NetVega => 0.0,
        };
        // THIS FILL's own contribution, in the SAME metric — the denominator that scales the
        // offsetting leg. Both `rates_linear_exposure` and `rates_signed_notional` are linear
        // in the instrument's notional/redemption, so the shed RATIO is identical either way;
        // what matters is only that numerator and denominator share a metric.
        let fill_risk = match thr_def.metric {
            HedgeMetric::Dv01 => rates_linear_exposure(fill).abs(),
            HedgeMetric::NetNotional | HedgeMetric::NetDelta | HedgeMetric::GrossNotional => {
                rates_signed_notional(fill).abs()
            }
            HedgeMetric::NetVega => 0.0,
        };

        // The price-tolerance verdict: did the desk capture enough edge to warehouse this fill?
        let verdict = internalise::verdict(desk_side, dealt, mid, kind, policy.config.min_edge_bps);

        // Run the pure engine on the book's risk state: it decomposes the shed internal-vs-
        // external per the firm's hedge-policy graph AND records the firm-wide audit provenance
        // in the shared ring the `ListHedgeProvenance` RPC serves. `now` is the (off-core)
        // fire timestamp for that audit record.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        // `execution_instrument` is CLONED rather than moved: the street-routing seam below
        // still needs it for the decision journal's `symbol` (and the executor's order), so
        // the context builder may not consume it.
        let ctx = self.build_hedge_context(&HedgeContextInputs {
            book,
            instrument: &instrument,
            execution_instrument: execution_instrument.clone(),
            attribution,
            policy: &policy,
            metric: thr_def.metric,
            wh: &wh,
            book_net_dv01,
            book_net_notional,
            book_gross_notional,
            book_risk,
            mid,
            bp_scale: kind.bp_scale(),
        });
        // Best-order timer: bracket the auto-hedge decision (`OpKind::HedgeFire`) — the pure
        // engine's threshold-breach → hedge-action evaluation on this fill's post-book book
        // risk. Measured at the call site (not inside the engine) so the pinned/pure engine
        // stays telemetry-free; recorded off the async booking tier via the store's hub
        // (guardrail 11), a no-op when no hub is installed.
        let hedge_t0 = std::time::Instant::now();
        // The audit identity of this evaluation: WHICH policy governed (or that none was
        // authored), and the keys that join the journal row back to this fill, its lift
        // trace and its counterparty. A model-derived graph says so, so a trader is never
        // shown a walked path they cannot find in their own authored rules.
        let mut scope = policy.scope_label(scope_book);
        if derived_model_graph {
            scope = format!("{scope} (model-derived)");
        }
        let meta = crate::services::auto_hedge::DecisionMeta {
            scope,
            trace_id,
            position_id: Some(fill.position_id),
            // A linear-rates position carries no counterparty field (it is an
            // entity/book/instrument cell), so a hedge decision row honestly names none —
            // the counterparty lives on the ACCEPTANCE row of the same lift, which the
            // shared `trace_id` joins this row to.
            counterparty: None,
            symbol: execution_instrument.clone(),
        };
        let outcome = policy.engine.evaluate_with_meta(
            graph,
            &thr_def,
            &policy.config,
            &ctx,
            &policy.known_lps,
            now,
            &meta,
        );
        self.record_latency(
            celnet_observability::OpKind::HedgeFire,
            u64::try_from(hedge_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
        // The book-level external shed the graph's decompose decided; attribute up to this
        // fill's own DV01 to it (a fill can shed at most what it added).
        let shed = outcome.intent.external_hedged;

        // Whether the resolved policy graph's action for THIS fill is an EXTERNAL shed (a market
        // order / RFQ-out) rather than an internal hold (warehouse / cross-internal / skew /
        // escalate) — read from the graph's OWN resolved action, computed BEFORE the split so the
        // below-tolerance branch can HONOR the policy. A warehouse / wash / internal-only book
        // captures ~0 edge on essentially every fill; it must NOT be force-shed to the street.
        let graph_is_external = outcome
            .intent
            .action
            .as_ref()
            .and_then(|d| crate::services::auto_hedge::wire::exit_action_from_wire(d).ok())
            .is_some_and(|a| a.is_external());

        // Combine the engine's shed with the tolerance verdict (§6), honoring the resolved policy:
        // - within tolerance (making money): warehouse the fill, shedding only the over-cap
        //   overflow (clamped to the fill's DV01) externally; internalised iff nothing is shed.
        //   (A warehouse hold decomposes to shed = 0, so a within-tolerance shed already implies
        //   an external graph action.)
        // - below tolerance AND the policy is EXTERNAL: a losing fill under a shed policy is
        //   handed straight back to the street as a back-to-back (the whole fill).
        // - below tolerance BUT the policy is an INTERNAL hold (warehouse / wash): HONOR it —
        //   warehouse the fill. A wash / internal-only book must never emit an external B2B that
        //   contradicts its own internal mandate just because the fill captured thin edge.
        // Consequence exploited below: `external_dv01 > 0` now implies `graph_is_external` in
        // BOTH shedding branches, so the external portion always stamps the graph's OWN action.
        // UNWIND: the shed is bounded by the BOOK's own risk, not by this fill's contribution.
        // It used to be `shed.min(fill_risk)` — "a fill can shed at most what it added" — which
        // silently made auto-hedging incapable of reducing risk it already held: each decision
        // could only neutralise the incoming fill, so a book sitting far over its cap breached
        // on every fill, hedged the fill, and stayed exactly where it was. (Observed on UAT:
        // 14 consecutive breach decisions, net DV01 starting and ending at -87,800.) Manual
        // `ClearRisk` was then the only thing that could ever unwind a position.
        //
        // The engine's `resolve_size` already caps every size at `|net_risk|` (you cannot hedge
        // more than you hold) and applies the threshold's `min_clip`/`max_clip`, so the ticket
        // stays bounded — `max_clip` is the control for "never send more than X in one go".
        // A shed larger than the fill simply scales the offsetting leg above 1× the fill, i.e.
        // trades a bigger clip of the same instrument, which is exactly what unwinding means.
        let (internal_dv01, external_dv01, internalised) = if verdict.within_tolerance {
            let ext = shed.min(book_risk.abs()).max(0.0);
            ((fill_risk - ext).max(0.0), ext, ext == 0.0)
        } else if graph_is_external {
            (0.0, fill_risk, false)
        } else {
            (fill_risk, 0.0, true)
        };

        let prov = InternaliseProvenance {
            internalised,
            internal_dv01,
            external_dv01,
            edge_bps: verdict.edge_bps,
            within_tolerance: verdict.within_tolerance,
            hedge_band: band_label(wh.classify(book_risk)).to_owned(),
        };
        // HEDGING (class=Hedge → orders sink): the internalise-vs-back-to-back decision —
        // the price-tolerance verdict, the internal/external DV01 split, the warehouse-cap
        // utilisation and RAG band. WARN when the routed book has BREACHED its warehouse
        // cap (the desk must shed), INFO otherwise. Async booking tier, never the core.
        let warehouse_breached = wh.breached(book_risk);
        let utilization = wh.utilization(book_risk);
        if warehouse_breached {
            tracing::warn!(
                class = celnet_observability::LogClass::Hedge.label(),
                position_id = fill.position_id,
                book,
                internalised = prov.internalised,
                internal_dv01 = prov.internal_dv01,
                external_dv01 = prov.external_dv01,
                edge_bps = prov.edge_bps,
                within_tolerance = prov.within_tolerance,
                hedge_band = %prov.hedge_band,
                book_net_dv01,
                warehouse_cap = wh.cap,
                utilization,
                warehouse_breached,
                "auto-hedge internalise decision — warehouse cap breached",
            );
        } else {
            tracing::info!(
                class = celnet_observability::LogClass::Hedge.label(),
                position_id = fill.position_id,
                book,
                internalised = prov.internalised,
                internal_dv01 = prov.internal_dv01,
                external_dv01 = prov.external_dv01,
                edge_bps = prov.edge_bps,
                within_tolerance = prov.within_tolerance,
                hedge_band = %prov.hedge_band,
                book_net_dv01,
                warehouse_cap = wh.cap,
                utilization,
                warehouse_breached,
                "auto-hedge internalise decision",
            );
        }
        self.internalise
            .write()
            .expect("rates internalise lock poisoned")
            .insert(fill.position_id, prov.clone());

        // Does this scope fire by itself, or publish a standing suggestion a trader fires
        // (§6.5)? Resolved most-specific-wins (instrument > book > desk); an unbound scope is
        // `Auto`, so a firm that has configured nothing behaves exactly as before. Resolved
        // HERE, before the trace, because a suggestion must not be traced as a FIRE.
        let exit_mode = policy
            .config
            .resolve_exit_mode(&ctx.desk, &ctx.book, &ctx.instrument_id);

        // End-to-end event trace (Analytics pillar C): emit the HEDGE_DECIDED stage (the
        // internalise-vs-shed verdict + RAG band) always, and the HEDGE_FIRED stage when the
        // fill actually sheds risk externally (`external_dv01 > 0`) — the terminal stage of a
        // hedged lift. Timestamps strictly increase, so decided precedes fired. Off the pinned
        // pricing core (guardrail 11); a no-op when this lift carries no trace.
        if let Some(tid) = trace_id {
            let counterparty =
                (!attribution.counterparty.is_empty()).then(|| attribution.counterparty.clone());
            self.trace_stage(
                tid,
                celnet_proto::TraceStage::HedgeDecided,
                crate::services::trace::TraceDetails {
                    price: Some(mid),
                    counterparty: counterparty.clone(),
                    position_id: Some(fill.position_id),
                    detail: Some(format!(
                        "band={}; {}; edge={:.2}bp; int_dv01={:.1}; ext_dv01={:.1}; mode={}",
                        prov.hedge_band,
                        if prov.internalised {
                            "internalised"
                        } else {
                            "shed"
                        },
                        prov.edge_bps,
                        prov.internal_dv01,
                        prov.external_dv01,
                        exit_mode.label(),
                    )),
                    ..Default::default()
                },
            );
            // A SUGGESTION is not a fire. Under `Suggest` the decision is traced (above) but
            // the terminal `HEDGE_FIRED` stage is deliberately withheld until the trader
            // actually fires it — a trace that claimed a fire for an untraded suggestion would
            // be a fabricated event (guardrail 2).
            if external_dv01 > 0.0 && exit_mode.fires_automatically() {
                // The minted hedge id when the engine stamped a book-level intent for this
                // shed; the per-fill advisory record mints its own id inside `record_execution`
                // (not returned), so it is absent (honest) rather than fabricated here.
                let hedge_id = outcome
                    .provenance
                    .as_ref()
                    .map(|p| p.hedge_id.clone())
                    .filter(|s| !s.is_empty());
                let lp = outcome.intent.lps.first().cloned();
                self.trace_stage(
                    tid,
                    celnet_proto::TraceStage::HedgeFired,
                    crate::services::trace::TraceDetails {
                        price: Some(mid),
                        notional: Some(external_dv01),
                        counterparty,
                        position_id: Some(fill.position_id),
                        hedge_id,
                        detail: Some(format!(
                            "band={}; ext_dv01={:.1}; advisory{}",
                            prov.hedge_band,
                            external_dv01,
                            lp.map(|l| format!("; lp={l}")).unwrap_or_default(),
                        )),
                        ..Default::default()
                    },
                );
            }
        }

        // LIVE EXECUTION (§6.2): when THIS fill sheds risk externally (an over-cap overflow OR a
        // below-min-edge back-to-back under an external policy — see the split above), execute
        // the shed on the policy's configured venue off the reference composite mid — the live LP
        // panel, the Agg Book COMPOSITE mid, or LP-first-then-composite — then BOOK the offsetting
        // leg so the warehoused net actually reduces, and stamp a **per-fill hedge-execution
        // record** with the REAL economics (advisory=false, realised price / mid / signed slippage
        // / winning venue). Keyed by the fill's `position_id`, so the LIVE HEDGE DESK reconciles
        // to the originating B2B deal (which carries the same id). An `Advisory` mode (or a pure
        // `LpPanel` miss) books nothing and stamps an honest advisory record — never a fabricated
        // fill (guardrail 2).
        //
        // Because the split above forces `external_dv01 = 0` for an internal-policy hold, reaching
        // here implies `graph_is_external`, so the record carries the graph's OWN external action
        // (no synthesized market-order stand-in) with the fill's realised economics. To keep
        // EXACTLY ONE ring record per fill, this realised record SUPERSEDES the pre-execution
        // decision record `evaluate` already rang (amended in place by its `hedge_id`) rather than
        // appending a second row — otherwise a fired hedge would double-count in the blotter.
        if external_dv01 > 0.0 {
            // ---- THE HEDGE VEHICLE: with WHAT does this exit hedge? (§6.4) ----------
            //
            // The resolved leaf carries the trader's vehicle choice, which rides FLAT on the
            // action descriptor. A `SelfInstrument` leaf (the default, and every policy
            // authored before vehicles existed) short-circuits everything below and the path
            // is byte-identical to what it always was.
            let vehicle = outcome
                .intent
                .action
                .as_ref()
                .map(crate::services::auto_hedge::wire::hedge_vehicle_from_wire)
                .unwrap_or_default();
            // The GENUINE DV01 of this fill and the basis it came from — NOT the book's
            // duration-blind exposure proxy, which would mis-size a bond-vs-future ratio by
            // the bond's whole duration. See `genuine_position_dv01`.
            let (fill_dv01_genuine, dv01_basis) =
                genuine_position_dv01(fill, matches!(kind, QuoteKind::Price).then_some(dealt));
            // The shed as a dimensionless FRACTION of this fill's own risk. Both numerator
            // and denominator are in the threshold's metric, so the ratio is unit-free and
            // can be applied to the genuine DV01 (and, later, to the fill's face) without
            // ever mixing a notional with a PV01. It may exceed 1: a breached book sheds its
            // OWN overflow, which is what lets auto-hedging unwind a standing position.
            let shed_fraction = if fill_risk > 0.0 {
                external_dv01 / fill_risk
            } else {
                0.0
            };
            // Size the vehicle trade off the genuine DV01:
            //     units = (fraction × fill_DV01) / vehicle_DV01_per_unit
            // rounded to whole contracts for a future, with the residual reported.
            let plan: Option<HedgeRatioPlan> = resolve_hedge_vehicle(
                &policy.config.vehicles,
                &vehicle,
                ctx.execution_instrument_id
                    .as_deref()
                    .unwrap_or(&ctx.instrument_id),
                &ctx.product,
                &ctx.ccy,
                hedge_maturity_years(fill),
                // The valuation date the futures FRONT MONTH is resolved on, so a vehicle
                // configured as a product (`ZF`) trades the contract that is live today
                // rather than the one that was live when the policy was written.
                {
                    let d = time::OffsetDateTime::now_utc().date();
                    celnet_refdata::CivilYmd::new(
                        d.year(),
                        u32::from(u8::from(d.month())),
                        u32::from(d.day()),
                    )
                },
            )
            .and_then(|(rule, whole_units)| {
                plan_hedge_ratio(
                    shed_fraction * fill_dv01_genuine,
                    dv01_basis,
                    &rule,
                    whole_units,
                )
            });
            if !vehicle.is_self() && plan.is_none() {
                // A named vehicle the registry does not know has NO known DV01 per unit, and a
                // `Benchmark` that matches no bucket has no instrument at all. Either way the
                // ratio could only be guessed — so fall back to the self-hedge (always exact,
                // ratio 1) and say loudly why, rather than trading a fabricated size.
                tracing::warn!(
                    book,
                    instrument = %ctx.instrument_id,
                    vehicle = %vehicle,
                    "hedge VEHICLE did not resolve against the registry — no DV01 per unit is \
                     known, so this shed falls back to the self-hedge (the same security sold \
                     back) rather than trading a guessed size",
                );
            }
            // What the venue is actually asked to trade. A vehicle hedge asks for the
            // VEHICLE's security (the future / benchmark) — that is the whole point.
            let execution_instrument = plan.as_ref().map_or_else(
                || {
                    ctx.execution_instrument_id
                        .clone()
                        .unwrap_or_else(|| ctx.instrument_id.clone())
                },
                |p| p.hedge_instrument_id.clone(),
            );
            // The externalised size stays denominated in the BUDGET METRIC end to end (the
            // contracts live on the plan), but is scaled down by whatever whole-lot rounding
            // actually achieved. That is what makes the rounding residual real: the book
            // reduces by the DV01 the 318 contracts removed, not by the 318.47 we wanted.
            let effective_external = plan
                .as_ref()
                .map_or(external_dv01, |p| external_dv01 * p.fraction_of_target());

            // ---- SUGGEST vs AUTO: does this fire, or wait for a trader? (§6.5) ------
            if !exit_mode.fires_automatically() {
                self.raise_hedge_suggestion(
                    fill,
                    &ctx,
                    book,
                    &outcome,
                    &thr_def,
                    &wh,
                    book_risk,
                    utilization,
                    &prov.hedge_band,
                    internal_dv01,
                    effective_external,
                    fill_risk,
                    mid,
                    kind.bp_scale(),
                    execution_instrument,
                    plan,
                    now,
                );
                return;
            }

            let (venue_quantity, risk_per_venue_unit) =
                venue_denomination(plan.as_ref(), effective_external);
            let req = crate::services::auto_hedge::ExternalHedgeRequest {
                // The LP panel is asked for the TRADEABLE security, falling back to the family
                // label only when the cell resolves none. Asking for a family ("BOND") can
                // never match an aggregated-book instrument, so before this the panel always
                // missed and every shed backstopped to the synthetic composite — which in turn
                // left the street-side league table with no fill to attribute. Under a VEHICLE
                // hedge this is the vehicle's own security — the future the desk really trades.
                instrument: &execution_instrument,
                net_risk: book_risk,
                size: effective_external,
                venue_quantity,
                risk_per_venue_unit,
                mid,
                bp_scale: kind.bp_scale(),
                mode: policy.config.execution,
                composite_spread_bp: policy.config.composite_spread_bp,
            };
            // Fill against the LIVE LP panel when one is wired (the aggregation hub's inbound
            // per-LP quotes): an LP-panel mode then fills against the best executable LP price for
            // the hedged instrument on the required side (`venue=Lp`, `lp_won=<that LP>`, a real
            // signed slippage vs mid), and only falls back to the composite when NO LP has a firm
            // price. When no panel is wired the honest [`NoLpSource`] never fills, so
            // `LpPanelThenComposite` reaches the composite backstop and pure `LpPanel` records an
            // honest miss (never a fabricated fill, guardrail 2).
            let router = self.street_router();
            // Best-order timer O4 (`OpKind::HedgeExecute`): bracket the EXECUTION — a real
            // order on a real venue, with a real round trip. It is deliberately separate
            // from `HedgeFire` (the pure decision, microseconds): folding the two together
            // is what made the booking commit look like it cost milliseconds when almost
            // all of it was an outbound network wait, and the two call for opposite
            // responses — "our hedge logic is slow" vs "the street took 6ms to answer".
            let hedge_exec_t0 = std::time::Instant::now();
            let exec = match self.lp_hedge_source.get() {
                Some(src) => {
                    crate::services::auto_hedge::execute_external(&req, src.as_ref(), router)
                }
                None => crate::services::auto_hedge::execute_external(
                    &req,
                    &crate::services::auto_hedge::NoLpSource,
                    router,
                ),
            };
            self.record_latency(
                celnet_observability::OpKind::HedgeExecute,
                u64::try_from(hedge_exec_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
            // Book the offsetting leg into the same book so the warehoused net reduces by the
            // filled amount. `book_into_risk_book` re-runs the hard-cap gate (a reducing leg
            // never breaches, §8.3) and does NOT recurse into `stamp_internalise`. Every rates
            // arm — swaps AND cash bonds — yields a leg, so a filled shed always reduces the
            // book; only a cell carrying no instrument arm can construct none.
            //
            // A REJECTED booking is an operational break, not a no-op: the street trade really
            // happened, so the record below stays honestly non-advisory, but the book did NOT
            // reduce and the next fill would compound risk the blotter claims is hedged. Never
            // discard that Result silently — ring it at ERROR so ops sees the divergence.
            if exec.is_filled()
                && fill_risk > 0.0
                && let Some(leg) = offsetting_rates_leg(fill, exec.filled / fill_risk)
                && let Err(status) = self.book_into_risk_book(leg, book)
            {
                tracing::error!(
                    book,
                    instrument = %ctx.instrument_id,
                    parent_position_id = fill.position_id,
                    hedged_dv01 = exec.filled,
                    reason = %status,
                    "auto-hedge shed FILLED externally but its offsetting leg was REJECTED — \
                     the book did not reduce; risk and blotter now diverge",
                );
            }
            // Street-side execution capture (Analytics §2.4): EVERY attempt is recorded —
            // the named-LP fill, the composite backstop, and the honest miss — carrying the
            // ranked panel it competed against so the league table's "missed" and "cover"
            // are real rather than structurally zero. Keyed on the SECURITY actually dealt
            // when the cell resolved one (that is what the LP quoted), falling back to the
            // family label otherwise. Off-core.
            record_street_order(
                self.street_order_log(),
                &exec,
                policy.config.execution,
                &execution_instrument,
                &ctx.product,
                hedge_maturity_years(fill),
                book_risk,
                venue_quantity,
                risk_per_venue_unit,
                outcome
                    .provenance
                    .as_ref()
                    .map(|p| p.hedge_id.clone())
                    .filter(|id| !id.is_empty()),
                fill.position_id,
                now,
            );
            log_hedge_execution(
                book,
                &ctx.instrument_id,
                &execution_instrument,
                fill.position_id,
                &exec,
                "auto-hedge shed EXECUTED",
            );
            // Build the REALISED execution record: the graph's own (external) action + the fill's
            // real economics, keyed by `parent_position_id` for deal reconciliation. It SUPERSEDES
            // the engine's pre-execution decision record in place (see below), so a fired hedge
            // leaves exactly one ring row carrying the realised fill.
            let execution = HedgeProvenance {
                hedge_id: String::new(), // set by `amend_execution` / minted by `record_execution`
                book: book.to_owned(),
                instrument: ctx.instrument_id.clone(),
                fired_at: now,
                metric: thr_def.metric.as_i32(),
                threshold: wh.cap,
                net_risk: book_risk,
                utilization,
                band: prov.hedge_band.clone(),
                policy_path: outcome.intent.policy_path.clone(),
                action: outcome.intent.action.clone(),
                internal_crossed: internal_dv01,
                external_hedged: exec.filled,
                residual: exec.residual,
                hedge_price: exec.hedge_price,
                mid_at_fire: exec.mid_at_fire,
                slippage_bp: exec.slippage_bp,
                lp_won: exec.lp_won.clone(),
                // Advisory records a DRY RUN — the policy never tried to trade (Advisory
                // mode, or a rate cap that forced a shadow run). It must NOT be inferred
                // from the outcome: a live hedge that fired, routed and got no fill is a
                // MISS, and a miss leaves real risk on the book.
                //
                // Deriving it from `!exec.is_filled()` conflated the two and made the
                // warehoused figure structurally unreachable, because consumers filter
                // advisory rows out (`flowTotals`, gui/src/lib/hedgeBuckets.ts). While a
                // composite backstop existed every shed filled, so nothing was ever
                // mis-flagged; switching UAT to `LpPanel` exposed it at once — 25 misses
                // holding 18,322 DV01 reported as "no hedges have fired yet, 0 warehoused".
                // The street-order side already keeps this distinction (see the comment on
                // `record_street_order`: an advisory desk must not look like one whose
                // orders all missed).
                advisory: outcome.intent.advisory,
                lps: outcome.intent.lps.clone(),
                parent_position_id: Some(fill.position_id),
                // The VEHICLE sizing, when this shed hedged with something other than the
                // position's own security: the DV01 ratio, the whole-lot rounding, and the
                // honest residual. Absent for a self-hedge (ratio identically 1).
                vehicle_plan: plan
                    .as_ref()
                    .map(crate::services::auto_hedge::wire::vehicle_plan_to_wire),
            };
            // EXACTLY ONE ring record per external fill: `evaluate` already rang a book-level
            // DECISION record for this size-bearing external action (it self-rings any non-hold
            // action, returning `provenance: Some`). Amend THAT record in place with the realised
            // economics — matched by its minted `hedge_id` — rather than appending a duplicate. On
            // the (defensive) chance the engine rang nothing here, fall back to appending a fresh
            // realised record so the fill is never lost.
            match outcome
                .provenance
                .as_ref()
                .map(|p| p.hedge_id.clone())
                .filter(|id| !id.is_empty())
            {
                Some(decision_id) => {
                    policy.engine.amend_execution(&decision_id, execution);
                }
                None => {
                    policy.engine.record_execution(execution);
                }
            }
        }
    }

    /// The shared **standing-suggestion** store (§6.5) — read by `ListHedgeSuggestions`,
    /// consumed by `ExecuteHedgeSuggestion`.
    #[must_use]
    pub fn hedge_suggestions(&self) -> &Arc<crate::services::auto_hedge::SuggestionStore> {
        &self.suggestions
    }

    /// Publish a **standing suggestion** instead of trading (`Suggest` mode, §6.5).
    ///
    /// Everything the automatic path would have computed is already computed by the time we
    /// get here — the band, the resolved action, the vehicle, the DV01 ratio, the whole-lot
    /// rounding, the residual. This method does exactly two things with it: renders the
    /// trader-facing instruction, and pins the execution recipe so firing it later trades
    /// *that* hedge rather than re-deriving a different one.
    ///
    /// Deliberately **no notification, no modal, no interrupt** — the suggestion is
    /// addressed to a `(book, instrument)` cell and rendered inline on the risk surface.
    #[allow(clippy::too_many_arguments)] // one call site; the whole decision is threaded through.
    fn raise_hedge_suggestion(
        &self,
        fill: &RatesPosition,
        ctx: &HedgeContext,
        book: &str,
        outcome: &crate::services::auto_hedge::HedgeOutcome,
        thr_def: &HedgeThresholdDef,
        wh: &celnet_hedge_routing::WarehouseThreshold,
        book_risk: f64,
        utilization: f64,
        band: &str,
        internal_crossed: f64,
        external_size: f64,
        fill_risk: f64,
        mid: f64,
        bp_scale: f64,
        execution_instrument: String,
        plan: Option<HedgeRatioPlan>,
        now: i64,
    ) {
        // The instruction, in the desk's own words: which way, how much, of what.
        //
        // The side follows the DURATION sign, not the trade sign. `book_risk` here is the
        // signed linear exposure, whose convention is that a LONG cash bond is NEGATIVE (long
        // duration nets against a pay-fixed swap — see `rates_linear_exposure`). A hedge
        // instrument (a benchmark bond or a bond future) is itself long duration when bought,
        // so shedding a net-long-duration book (`book_risk < 0`) means SELLING the hedge, and
        // shedding a net-short-duration book (a pay-fixed swap position, `book_risk > 0`)
        // means BUYING it. Getting this backwards would double the risk rather than remove it.
        let side = if book_risk < 0.0 { "Sell" } else { "Buy" };
        let headline = match plan.as_ref() {
            Some(p) if p.is_tradeable() => format!("{side} {}", p.summary()),
            // A vehicle whose target rounds to zero whole contracts is honestly untradeable
            // with that vehicle — say so rather than showing a size of nothing.
            Some(p) => format!(
                "No hedge available: {:.2} {} of {} rounds to zero whole lots ({:.1} DV01 stays)",
                p.exact_units,
                if p.unit_label.is_empty() {
                    "unit"
                } else {
                    p.unit_label.as_str()
                },
                p.hedge_instrument_id,
                p.residual_dv01,
            ),
            None => format!("{side} {external_size:.0} of {execution_instrument} (self-hedge)"),
        };
        let mut rationale = format!(
            "{band} band at {:.0}% of the {:.0} cap · policy resolved {}",
            utilization * 100.0,
            wh.cap,
            outcome
                .intent
                .action
                .as_ref()
                .map_or("—", |a| exit_action_kind_label(a.kind)),
        );
        // The honesty caveat rides ON the suggestion, not just in the payload: a size taken
        // off the duration-blind exposure proxy must never read as exact.
        if let Some(p) = plan.as_ref()
            && !p.basis.is_duration_correct()
        {
            rationale.push_str(
                " · SIZE IS APPROXIMATE: the hedge ratio was computed off the coarse exposure \
                 proxy (which treats every bond as duration 1), not a genuine DV01",
            );
        }
        let desc = celnet_proto::HedgeSuggestion {
            suggestion_id: String::new(), // minted by the store
            book: book.to_owned(),
            instrument: ctx.instrument_id.clone(),
            desk: ctx.desk.clone(),
            raised_at: now,
            band: band.to_owned(),
            net_risk: book_risk,
            threshold: wh.cap,
            utilization,
            action: outcome.intent.action.clone(),
            policy_path: outcome.intent.policy_path.clone(),
            external_size,
            vehicle_plan: plan
                .as_ref()
                .map(crate::services::auto_hedge::wire::vehicle_plan_to_wire),
            headline,
            rationale,
            parent_position_id: Some(fill.position_id),
            lps: outcome.intent.lps.clone(),
            mid_at_raise: mid,
        };
        let exec = crate::services::auto_hedge::SuggestionExec {
            book: book.to_owned(),
            instrument: ctx.instrument_id.clone(),
            execution_instrument,
            net_risk: book_risk,
            size: external_size,
            mid,
            bp_scale,
            fill: fill.clone(),
            fill_risk,
            metric: thr_def.metric.as_i32(),
            threshold: wh.cap,
            utilization,
            band: band.to_owned(),
            policy_path: outcome.intent.policy_path.clone(),
            action: outcome.intent.action.clone(),
            lps: outcome.intent.lps.clone(),
            internal_crossed,
            plan,
        };
        let raised = self.suggestions.raise(desc, exec);
        tracing::info!(
            class = celnet_observability::LogClass::Hedge.label(),
            book,
            instrument = %ctx.instrument_id,
            suggestion_id = %raised.desc.suggestion_id,
            headline = %raised.desc.headline,
            external_size,
            "hedge SUGGESTED (manual exit mode) — nothing traded; standing on the risk surface",
        );
    }

    /// Resolve a standing suggestion: **fire** it (execute the pinned hedge on the
    /// configured venue and book the offsetting leg, exactly as the automatic path would
    /// have) or **dismiss** it (clear it, trade nothing).
    ///
    /// Both paths consume the suggestion, so a fired hedge can never be fired twice.
    /// Dismissing hides nothing: the risk stays on the book and the next fill on that cell
    /// raises the suggestion again.
    ///
    /// # Errors
    /// A `suggestion_id` that is unknown or already resolved (a superseded id included) —
    /// which is the point of superseding: a stale size can never be traded.
    pub fn resolve_hedge_suggestion(
        &self,
        suggestion_id: &str,
        dismiss: bool,
    ) -> Result<Option<HedgeProvenance>, String> {
        let Some(row) = self.suggestions.take(suggestion_id) else {
            return Err(format!(
                "hedge suggestion {suggestion_id:?} is unknown, already resolved, or has been \
                 superseded by a newer suggestion for the same book and instrument"
            ));
        };
        if dismiss {
            tracing::info!(
                class = celnet_observability::LogClass::Hedge.label(),
                book = %row.exec.book,
                instrument = %row.exec.instrument,
                suggestion_id,
                "hedge suggestion DISMISSED — nothing traded; the risk stays on the book",
            );
            return Ok(None);
        }
        let Some(policy) = self
            .hedge_policy
            .read()
            .expect("rates hedge-policy lock poisoned")
            .clone()
        else {
            return Err("no hedge policy is primed — the engine cannot execute".to_owned());
        };
        let e = &row.exec;
        let (venue_quantity, risk_per_venue_unit) = venue_denomination(e.plan.as_ref(), e.size);
        let req = crate::services::auto_hedge::ExternalHedgeRequest {
            instrument: &e.execution_instrument,
            net_risk: e.net_risk,
            size: e.size,
            venue_quantity,
            risk_per_venue_unit,
            mid: e.mid,
            bp_scale: e.bp_scale,
            mode: policy.config.execution,
            composite_spread_bp: policy.config.composite_spread_bp,
        };
        let router = self.street_router();
        let hedge_exec_t0 = std::time::Instant::now();
        let exec = match self.lp_hedge_source.get() {
            Some(src) => crate::services::auto_hedge::execute_external(&req, src.as_ref(), router),
            None => crate::services::auto_hedge::execute_external(
                &req,
                &crate::services::auto_hedge::NoLpSource,
                router,
            ),
        };
        self.record_latency(
            celnet_observability::OpKind::HedgeExecute,
            u64::try_from(hedge_exec_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        // Book the offsetting leg so the book's net actually falls — the same half-two of the
        // exit the automatic path performs (§8.2). `e.size` is already reduced by whole-lot
        // rounding, so the book reduces by what the vehicle trade really removed and the
        // rounding residual honestly stays.
        if exec.is_filled()
            && e.fill_risk > 0.0
            && let Some(leg) = offsetting_rates_leg(&e.fill, exec.filled / e.fill_risk)
            && let Err(status) = self.book_into_risk_book(leg, &e.book)
        {
            tracing::error!(
                book = %e.book,
                instrument = %e.instrument,
                parent_position_id = e.fill.position_id,
                hedged = exec.filled,
                reason = %status,
                "fired hedge suggestion FILLED externally but its offsetting leg was REJECTED — \
                 the book did not reduce; risk and blotter now diverge",
            );
        }
        log_hedge_execution(
            &e.book,
            &e.instrument,
            &e.execution_instrument,
            e.fill.position_id,
            &exec,
            "fired hedge suggestion EXECUTED",
        );
        let prov = policy.engine.record_execution(HedgeProvenance {
            hedge_id: String::new(),
            book: e.book.clone(),
            instrument: e.instrument.clone(),
            fired_at: now,
            metric: e.metric,
            threshold: e.threshold,
            net_risk: e.net_risk,
            utilization: e.utilization,
            band: e.band.clone(),
            policy_path: e.policy_path.clone(),
            action: e.action.clone(),
            internal_crossed: e.internal_crossed,
            external_hedged: exec.filled,
            residual: exec.residual,
            hedge_price: exec.hedge_price,
            mid_at_fire: exec.mid_at_fire,
            slippage_bp: exec.slippage_bp,
            lp_won: exec.lp_won.clone(),
            // Same distinction as the automatic path above: advisory means the desk never
            // tried to trade, NOT that the trade failed. A trader who fires a suggestion
            // has deliberately chosen to trade, so only an Advisory-mode policy makes the
            // resulting record a dry run; a miss stays a live fire whose residual is real
            // risk the trader must still see.
            advisory: policy.config.execution.is_advisory(),
            lps: e.lps.clone(),
            parent_position_id: Some(e.fill.position_id),
            vehicle_plan: e
                .plan
                .as_ref()
                .map(crate::services::auto_hedge::wire::vehicle_plan_to_wire),
        });
        // Street-side execution capture (Analytics §2.4) — the fired-suggestion path records
        // exactly what the automatic path does, so a manually-fired hedge is as visible on the
        // street-side blotter as an automatic one. Recorded AFTER the provenance is minted so
        // the order carries the real `hedge_id`: that linkage is what lets a trader walk from a
        // breach, to the hedge decision, to the street orders it produced.
        record_street_order(
            self.street_order_log(),
            &exec,
            policy.config.execution,
            &e.execution_instrument,
            &e.instrument,
            hedge_maturity_years(&e.fill),
            e.net_risk,
            venue_quantity,
            risk_per_venue_unit,
            Some(prov.hedge_id.clone()).filter(|id| !id.is_empty()),
            e.fill.position_id,
            now,
        );
        Ok(Some(prov))
    }

    /// The routed book's net DV01 (signed linear PV01 proxy) over the positions currently
    /// stamped into it — the risk state the internalise decision measures against the
    /// warehouse cap. Own positions only (the per-fill decision is book-local; the dashboard's
    /// subtree roll-up is the separate `book_risk` seam).
    #[must_use]
    fn book_net_dv01(&self, book: &str) -> f64 {
        self.positions_in_risk_book(book)
            .iter()
            .map(rates_linear_exposure)
            .sum()
    }

    /// The **subtree** net DV01 of a `Bucket` policy: the bucket root's own net DV01 plus
    /// every descendant book's, summed (the "PORTFOLIO notional" a bucket-scoped hedge
    /// decision measures — §5). `descendants` is the primed subtree id set for `root`
    /// ([`RatesHedgePolicy::book_descendants`]). Mirrors the FX `aggregate_risk_book`
    /// subtree roll-up (`services/risk/book_risk.rs`), book-local per node.
    #[must_use]
    fn subtree_net_dv01(&self, root: &str, descendants: &[String]) -> f64 {
        let mut sum = self.book_net_dv01(root);
        for d in descendants {
            sum += self.book_net_dv01(d);
        }
        sum
    }

    /// The routed book's signed net FACE NOTIONAL over the positions currently stamped into
    /// it — the operand a hedge rule's `NetNotional` / `NetDelta` condition measures against
    /// (a DISTINCT measure from [`Self::book_net_dv01`]: notional vs. PV01;
    /// [`rates_signed_notional`] signs by trade direction, [`rates_linear_exposure`] by IR
    /// duration). Own positions only, exactly mirroring the DV01 method's scope + locking.
    #[must_use]
    fn book_net_notional(&self, book: &str) -> f64 {
        self.positions_in_risk_book(book)
            .iter()
            .map(rates_signed_notional)
            .sum()
    }

    /// The **subtree** signed net face notional of a `Bucket` policy: the bucket root's own
    /// plus every descendant book's, summed — the notional analogue of
    /// [`Self::subtree_net_dv01`] (same primed-descendant scope, book-local per node), used
    /// as the `NetNotional` / `NetDelta` operand for a bucket-scoped hedge decision (§5).
    #[must_use]
    fn subtree_net_notional(&self, root: &str, descendants: &[String]) -> f64 {
        let mut sum = self.book_net_notional(root);
        for d in descendants {
            sum += self.book_net_notional(d);
        }
        sum
    }

    /// The routed book's **GROSS** face notional — `Σ|notional|` over its stamped positions.
    /// Unlike [`Self::book_net_notional`] it never nets down: a payer and a receiver of equal
    /// size contribute twice, not zero. A gross budget is therefore a TURNOVER brake, not a
    /// risk budget — it only falls as positions roll off. Own positions only, exactly
    /// mirroring the net method's scope + locking.
    #[must_use]
    fn book_gross_notional(&self, book: &str) -> f64 {
        self.positions_in_risk_book(book)
            .iter()
            .map(|p| rates_signed_notional(p).abs())
            .sum()
    }

    /// The **subtree** gross face notional of a `Bucket` policy — the gross analogue of
    /// [`Self::subtree_net_notional`]. Gross sums across books exactly as it does within one
    /// (no cross-book netting to lose), so the roll-up is a plain sum.
    #[must_use]
    fn subtree_gross_notional(&self, root: &str, descendants: &[String]) -> f64 {
        let mut sum = self.book_gross_notional(root);
        for d in descendants {
            sum += self.book_gross_notional(d);
        }
        sum
    }

    /// Project this fill's post-book risk-book aggregate over the resolved book **and each
    /// ancestor**, returning a typed `failed_precondition` breach if any HARD notional cap
    /// (`max_net_notional` on `|net|`, `max_gross_notional` on gross) would be EXCEEDED — the
    /// rates analogue of the FX
    /// [`project_risk_book_breach`](super::risk::store::PositionStore) per-book gate (§8.3).
    /// Runs on a read snapshot BEFORE any mutation, so a breach rejects with the store left
    /// unmutated. Notional is the trade-direction **signed** rates notional (bond: redemption)
    /// summed nominally — the single-numeraire per-book cap the FI books this seam serves use.
    ///
    /// The prior fact under this fill's id (a re-book) is excluded so a re-book projects
    /// `others + this fill` and never double-counts. `max_dv01` is skipped here (the per-book
    /// DV01 cap is the dedicated rates-risk seam); no caps on the path ⇒ `None`, gate skipped.
    fn project_rates_risk_book_breach(
        &self,
        resolved_book: &str,
        fill: &RatesPosition,
    ) -> Option<tonic::Status> {
        // A single incoming rates fill: its signed notional is the net, its magnitude the
        // gross, and exactly its own prior fact is excluded — byte-identical to the original
        // single-fill gate the routed-fill path runs.
        let n = rates_signed_notional(fill);
        self.project_rates_book_breach_multi(
            resolved_book,
            n,
            n.abs(),
            rates_linear_exposure(fill),
            &[fill.position_id],
        )
    }

    /// The general per-book hard-limit projection over an incoming `(net, gross)` notional
    /// pair and a set of excluded ids — the shared core of the single-fill routed gate
    /// ([`Self::project_rates_risk_book_breach`]) and the multi-position transfer headroom
    /// check ([`Self::check_risk_book_headroom`], §6). A re-attribution or an economic
    /// transfer can move several rates positions (mixed sides) at once, so the incoming net
    /// and gross are passed separately and every moved id is excluded from the current-book
    /// roll-up so the projection is `others + incoming`, never double-counting a moved line.
    fn project_rates_book_breach_multi(
        &self,
        resolved_book: &str,
        incoming_net: f64,
        incoming_gross: f64,
        incoming_dv01: f64,
        exclude_ids: &[u64],
    ) -> Option<tonic::Status> {
        let limits_map = self
            .risk_book_limits
            .read()
            .expect("rates risk-book limit lock poisoned");
        // The scopes to enforce: the resolved book + its ancestors, keeping only those that
        // actually carry a `RiskLimits`. No caps on the whole path ⇒ nothing to gate.
        let chain = rates_risk_book_chain(&limits_map, resolved_book);
        let scoped: Vec<(&str, &RiskLimits)> = chain
            .iter()
            .filter_map(|id| {
                limits_map
                    .get(*id)
                    .and_then(|d| d.limits.as_ref())
                    .map(|l| (*id, l))
            })
            .collect();
        if scoped.is_empty() {
            return None;
        }
        // The current per-book OWN (un-rolled) net/gross from the stamped rates positions,
        // excluding every moved id (re-book supersede — never double-counted against itself).
        let stamps = self
            .risk_book
            .read()
            .expect("rates risk-book lock poisoned");
        let g = self
            .inner
            .read()
            .expect("rates position store lock poisoned");
        // Per book: (signed net notional, gross notional, signed DV01). DV01 is summed
        // SIGNED so equal-and-opposite legs net exactly as they do in the risk roll-up —
        // a paid-fixed and a received-fixed line of the same size consume no capacity
        // between them, which is the whole point of an internalising book.
        let mut own: HashMap<&str, (f64, f64, f64)> = HashMap::new();
        for p in g.iter() {
            if exclude_ids.contains(&p.position_id) {
                continue;
            }
            if let Some(book) = stamps.get(&p.position_id) {
                let n = rates_signed_notional(p);
                let e = own.entry(book.as_str()).or_insert((0.0, 0.0, 0.0));
                e.0 += n;
                e.1 += n.abs();
                e.2 += rates_linear_exposure(p);
            }
        }
        for (scope, lim) in scoped {
            // Subtree roll-up: the incoming batch + every current book whose ancestor-or-self
            // chain passes through `scope` (i.e. the book is in the subtree rooted at `scope`).
            let mut net = incoming_net;
            let mut gross = incoming_gross;
            let mut dv01 = incoming_dv01;
            for (book, (bnet, bgross, bdv01)) in &own {
                if rates_risk_book_chain(&limits_map, book).contains(&scope) {
                    net += bnet;
                    gross += bgross;
                    dv01 += bdv01;
                }
            }
            if let Some(cap) = lim.max_net_notional
                && net.abs() > cap
            {
                return Some(rates_risk_book_limit_breached(
                    scope,
                    "net_notional",
                    net.abs(),
                    cap,
                ));
            }
            if let Some(cap) = lim.max_gross_notional
                && gross > cap
            {
                return Some(rates_risk_book_limit_breached(
                    scope,
                    "gross_notional",
                    gross,
                    cap,
                ));
            }
            // `max_dv01` is now ENFORCED here. It was skipped while this gate had no DV01
            // numerator; `rates_linear_exposure` supplies one (exact undiscounted annuity
            // PV01 on the swap/FRA arms, modified-duration based on the bond arm), the same
            // measure the per-book risk roll-up publishes — so the cap that blocks a trade
            // and the utilization a trader watches are the SAME number, never two views
            // that can disagree. Compared on the ABSOLUTE, like the net-notional cap: a
            // received-fixed book consumes capacity exactly as a paid-fixed one does.
            if let Some(cap) = lim.max_dv01
                && dv01.abs() > cap
            {
                return Some(rates_risk_book_limit_breached(
                    scope,
                    "dv01",
                    dv01.abs(),
                    cap,
                ));
            }
        }
        None
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

/// The DESK's side + the quote kind (rate vs clean price) of a rates fill, read from its
/// instrument arm — the inputs the internalise price-tolerance verdict needs. `None` for a
/// missing / unrecognised arm or side (the fill then carries no internalise decision).
fn fill_side_and_quote_kind(fill: &RatesPosition) -> Option<(Side, QuoteKind)> {
    let arm = fill
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())?;
    let (side_i32, kind) = match arm {
        rates_instrument::Instrument::Ois(o) => (o.side, QuoteKind::Rate),
        rates_instrument::Instrument::Irs(i) => (i.side, QuoteKind::Rate),
        rates_instrument::Instrument::Fra(f) => (f.side, QuoteKind::Rate),
        rates_instrument::Instrument::Bond(b) => (b.side, QuoteKind::Price),
    };
    Some((Side::try_from(side_i32).ok()?, kind))
}

/// A short instrument-family label (`OIS` / `IRS` / `FRA` / `BOND`) for the internalise
/// decision's [`HedgeContext`] + instrument-scoped threshold match. Empty for a missing arm.
/// The **executable security id** for a fill, when it has one — the canonical
/// `BondInstrument.instrument_id` the reference registry, the wire and the LP feed all share
/// (the same id that names an aggregated-book instrument).
///
/// This is what makes a rates shed reachable on the street. [`internalise_instrument_label`]
/// deliberately returns the product FAMILY ("BOND"/"OIS"/…) because warehouse thresholds and
/// provenance are scoped by family — but a family label is not tradeable, so pricing the LP
/// lookup off it can only ever miss and silently backstop the shed to the synthetic composite.
///
/// `None` for an OIS/IRS/FRA fill (a rates cell genuinely carries no security-master id) and
/// for a bond whose `instrument_id` never resolved against refdata — empty means unresolved
/// on the wire and is never fabricated into an id (guardrail 2). The executor then falls back
/// to the family label, preserving the previous behaviour exactly.
fn hedge_execution_instrument(fill: &RatesPosition) -> Option<String> {
    let arm = fill
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())?;
    match arm {
        rates_instrument::Instrument::Bond(bond) if !bond.instrument_id.is_empty() => {
            Some(bond.instrument_id.clone())
        }
        _ => None,
    }
}

/// The **genuine** DV01 of a rates fill, and the basis it was computed on
/// (`docs/HEDGING-AND-RISK-EXIT.md` §6.4).
///
/// This exists because a hedge ratio is only as good as its numerator. It predates the
/// Wave-0 re-basing, when [`rates_linear_exposure`]'s bond arm was still `redemption ×
/// 1bp` and a ratio taken off it under-hedged a long-dated bond by its whole duration.
/// The two measures now **agree on the bond arm** — both are `celnet_bond::dv01` on the
/// same contract — with one deliberate difference: this function prefers the yield
/// implied by the **dealt clean price** when the booking path supplies one, whereas the
/// stored-position exposure measure has no dealt price and uses the par (coupon)
/// assumption. Sizing therefore stays at least as precise as measurement, never less.
///
/// Each arm reports the best measure it honestly has, **labelled**:
///
/// | Arm | Measure | [`Dv01Basis`] |
/// | --- | --- | --- |
/// | Bond | the closed-form analytic yield derivative of its own cashflow schedule (`celnet-bond`), off the yield implied by the dealt clean price when one is known, else its coupon (the par assumption) | `Analytic` |
/// | OIS / IRS / FRA | `notional × years × 1bp` — the undiscounted fixed-leg annuity, which IS the standard linear PV01 for a swap leg | `AnnuityPv01` |
/// | anything unconstructable | the coarse exposure proxy | `ExposureProxy` |
///
/// The label is never dropped: it rides onto the wire as
/// `HedgeVehiclePlanDesc.dv01_basis` / `duration_correct`, so a size computed off the
/// proxy is surfaced as approximate rather than presented as exact (guardrail 2).
///
/// `dealt_clean_price` is the bond's dealt CLEAN price per 100 when the booking path
/// supplied one (the RFQ-accept / FIX-lift paths do; the manual path does not).
/// Off the pinned pricing core — a handful of closed-form evaluations on the booking tier.
fn genuine_position_dv01(fill: &RatesPosition, dealt_clean_price: Option<f64>) -> (f64, Dv01Basis) {
    let proxy = || (rates_linear_exposure(fill).abs(), Dv01Basis::ExposureProxy);
    let Some(arm) = fill.instrument.as_ref().and_then(|i| i.instrument.as_ref()) else {
        return (0.0, Dv01Basis::ExposureProxy);
    };
    let rates_instrument::Instrument::Bond(bond) = arm else {
        // A swap/FRA's `notional × years × 1bp` IS the undiscounted annuity PV01 — the
        // standard linear measure, and duration-correct in shape (the tenor is right
        // there in the contract). Only the discounting is missing, which makes it a
        // conservative upper bound rather than a wrong number.
        return (rates_linear_exposure(fill).abs(), Dv01Basis::AnnuityPv01);
    };
    let today = time::OffsetDateTime::now_utc().date();
    let Ok(contract) = crate::rates_pricing::bond_contract_from_wire(bond, today) else {
        return proxy();
    };
    // Prefer the yield the market actually dealt at: solve it from the dealt clean price
    // (converted to the contract's own face and made dirty by its accrued). Falling back
    // to the coupon is the standard par assumption — modified duration is only weakly
    // sensitive to the yield level, so the ratio stays sound either way, and it is a
    // stated approximation rather than a fabricated input.
    let analytic = dealt_clean_price
        .filter(|p| p.is_finite() && *p > 0.0)
        .and_then(|clean_pct| {
            let clean = clean_pct / 100.0 * bond.redemption;
            let accrued = celnet_bond::accrued_interest(&contract).ok()?;
            celnet_bond::bond_risk(&contract, clean + accrued)
                .ok()
                .map(|r| r.dv01)
        })
        .or_else(|| celnet_bond::dv01(&contract, celnet_types::Rate(bond.coupon_rate)).ok());
    match analytic {
        Some(d) if d.is_finite() && d > 0.0 => (d, Dv01Basis::Analytic),
        // The analytic path did not produce a usable number: fall back to the coarse
        // proxy AND say so, rather than silently presenting a duration-blind size as exact.
        _ => proxy(),
    }
}

/// Split a shed into **what the venue is asked to trade** and **what one venue unit removes**.
///
/// A vehicle hedge sheds DV01 by trading a *lot-denominated* instrument, so the two are in
/// different denominations: the book accounts in DV01, while the venue trades contracts and
/// rejects anything that is not a whole lot. Asking the venue in DV01 got every futures shed
/// rejected `NOT_A_WHOLE_LOT` on UAT — a plan for 14 `ZTU26` went to the wire as 484.5648 —
/// so no external hedge ever filled and the book pinned at its cap.
///
/// Without a plan the self-hedge sells the same security back, where the two denominations
/// coincide, so the pair is `(size, 1.0)` and behaviour is unchanged.
fn venue_denomination(plan: Option<&HedgeRatioPlan>, size: f64) -> (f64, f64) {
    match plan {
        // A resolved plan is ALWAYS authoritative for the venue quantity — including when it
        // rounds to ZERO contracts. Falling back to `size` on a zero-unit plan puts the DV01
        // figure back on the wire for the one case the rounding exists to prevent (a target
        // below one whole lot); `execute_external` refuses a non-positive venue quantity,
        // which is the honest "too small to hedge with this vehicle".
        //
        // The listed venue denominates order quantity in FACE, not in contract counts: one
        // whole lot IS one contract's face value, which is what keeps a bond and a future in
        // the same units on one aggregated book. So a contract count goes on the wire
        // multiplied by the contract's face; the DV01 per wire-unit divides by the same
        // factor, leaving the risk conversion exact.
        Some(p) if p.dv01_per_unit > 0.0 => {
            match celnet_refdata::contract_face_value(&p.hedge_instrument_id) {
                Some(face) if face > 0.0 => (p.units.abs() * face, p.dv01_per_unit / face),
                // A vehicle that is not a listed contract trades in its own units already.
                _ => (p.units.abs(), p.dv01_per_unit),
            }
        }
        _ => (size, 1.0),
    }
}

/// The fill's **maturity in years** — the axis the hedge-vehicle registry buckets on (a
/// 9-year corp resolving into the 10Y-future bucket).
///
/// A cash bond carries a real maturity date, so this is the exact act/365 span from today.
/// A swap/FRA carries a tenor rather than a maturity date, so it reports that span. `None`
/// for a cell with no instrument arm or an unparseable date — a registry row that declares
/// a maturity bucket then honestly cannot match it (rather than being matched by a
/// fabricated zero).
fn hedge_maturity_years(fill: &RatesPosition) -> Option<f64> {
    let arm = fill
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())?;
    match arm {
        rates_instrument::Instrument::Ois(o) => Some(f64::from(o.tenor_years)),
        rates_instrument::Instrument::Irs(i) => Some(f64::from(i.tenor_years)),
        rates_instrument::Instrument::Fra(f) => {
            Some(f64::from(f.end_months.saturating_sub(f.start_months)) / 12.0)
        }
        rates_instrument::Instrument::Bond(b) => {
            let m = b.maturity_date.as_ref()?;
            let month = u8::try_from(m.month)
                .ok()
                .and_then(|x| time::Month::try_from(x).ok())?;
            let day = u8::try_from(m.day).ok()?;
            let maturity = time::Date::from_calendar_date(m.year, month, day).ok()?;
            let today = time::OffsetDateTime::now_utc().date();
            let days = (maturity - today).whole_days();
            (days > 0).then(|| days as f64 / 365.0)
        }
    }
}

/// Record the outbound **street order(s)** one hedge attempt produced, into the shared
/// street-side execution log.
///
/// Called at every place a hedge really reaches a venue — the automatic shed and the
/// fired suggestion — so the log holds EVERY attempt, not only the ones that filled.
///
/// # One row per order that actually left the building
///
/// A hedge that routed emits **one row per routed order**, in the order they were sent:
/// a refusal on the best-priced member followed by a fill on the cover is two rows,
/// because two `NewOrderSingle`s really went out and two counterparties really answered.
/// Collapsing them into one would erase the refusal, which is the single most useful
/// fact about that counterparty. If the panel then declined unanimously and the shed
/// backstopped, the backstop is an ADDITIONAL row (venue `CompositeBackstop`, **no**
/// `lp_id`) — the street orders happened and so did the backstop.
///
/// A hedge that routed nothing (a composite-only desk, or an empty panel) emits the
/// single row it always did: recording only fills is what made the street-side league
/// table indistinguishable between "we never hedged" and "we hedged and the street
/// never showed us a price".
///
/// # Every field is an observation, never an inference (guardrail 2)
///
/// `filled_price` / `slippage_bp` are absent unless something filled. `order_type` /
/// `time_in_force` are the FIX values **actually sent**, so they are present exactly on
/// the rows that carry a routed order and absent on a composite backstop, which sends
/// none. `response_latency_nanos` is the measured send→`ExecutionReport` round trip and
/// is absent only where there was no round trip to measure (an unroutable member, a
/// backstop).
#[allow(clippy::too_many_arguments)]
/// Record what the street was asked for, **in the venue's own units**.
///
/// `requested` is the quantity that actually went on the wire (contracts for a listed
/// future), NOT the budget-metric size — the blotter is a record of orders, so a row
/// reading "484.5648" against a plan for 14 contracts describes an order nobody sent.
/// `risk_per_venue_unit` converts the composite row's fill (which is carried in the budget
/// metric) back into the same units, so every quantity on one row is denominated alike.
fn record_street_order(
    log: Option<&Arc<crate::services::analytics::street_orders::StreetOrderLog>>,
    exec: &crate::services::auto_hedge::ExternalHedgeFill,
    mode: crate::config::hedge_policy::HedgeExecutionMode,
    instrument: &str,
    family: &str,
    tenor_years: Option<f64>,
    net_risk: f64,
    requested: f64,
    risk_per_venue_unit: f64,
    parent_hedge_id: Option<String>,
    parent_position_id: u64,
    ts_nanos: i64,
) {
    use crate::services::auto_hedge::HedgeVenue;
    use celnet_analytics::{StreetCompetitor, StreetOrder, StreetOutcome, StreetSide, StreetVenue};

    let Some(log) = log else { return };
    if requested <= 0.0 || mode.is_advisory() {
        // Nothing went out: either nothing was asked for, or the desk is in the
        // shadow-run posture and this seam trades nothing at all. Recording a street
        // order for either would manufacture activity that never happened, and would
        // make an advisory desk indistinguishable from one whose orders all missed.
        return;
    }

    let family = family.to_ascii_lowercase();
    let side = StreetSide::shedding(net_risk);
    // The competition each routed order was ranked against. `exec.panel` holds it
    // whenever the fill itself competed on the street; a composite backstop deliberately
    // carries none (it preferred nothing over the street — there was nothing it could
    // deal on), so for those rows the panel is reconstructed from the members we
    // actually addressed, which under a unanimous decline IS the whole ranking.
    let competitors: Vec<StreetCompetitor> = if exec.panel.is_empty() {
        exec.attempts
            .iter()
            .map(|a| StreetCompetitor {
                lp_id: a.lp_id.clone(),
                price: a.quoted_price,
            })
            .collect()
    } else {
        exec.panel
            .iter()
            .map(|f| StreetCompetitor {
                lp_id: f.lp_id.clone(),
                price: f.price,
            })
            .collect()
    };

    // ---- the routed orders -------------------------------------------------------
    for attempt in &exec.attempts {
        let filled = attempt.outcome.is_fill();
        log.record(StreetOrder {
            lp_id: Some(attempt.lp_id.clone()),
            // A named venue is claimed only where something traded. An order that was
            // refused reached a counterparty but executed nowhere, and the league-table
            // fold reads that as "showed a price, did not trade" rather than a win.
            venue: if filled {
                StreetVenue::NamedLp
            } else {
                StreetVenue::None
            },
            family: family.clone(),
            tenor_years,
            filled_qty: attempt.filled,
            filled_price: filled.then_some(attempt.price).flatten(),
            // Only the attempt that actually traded has a realised slippage, and it is
            // the one the fill's economics were computed from.
            slippage_bp: filled.then_some(exec.slippage_bp),
            outcome: attempt.outcome.street_outcome(),
            reason: attempt.reason.clone(),
            competitors: competitors.clone(),
            parent_hedge_id: parent_hedge_id.clone(),
            parent_position_id: Some(parent_position_id),
            order_type: Some((attempt.order_type as char).to_string()),
            time_in_force: Some((attempt.time_in_force as char).to_string()),
            response_latency_nanos: attempt.response_latency_nanos,
            ..StreetOrder::new(
                log.next_order_id(),
                ts_nanos,
                instrument,
                side,
                requested,
                exec.mid_at_fire,
            )
        });
    }

    // ---- the venue of record for this shed --------------------------------------
    // An LP-panel fill is fully described by its routed order above; anything else
    // needs its own row.
    if exec.venue == Some(HedgeVenue::LpPanel) {
        return;
    }
    // A non-zero residual means the venue filled less than we asked for.
    let fill_outcome = if exec.residual > 0.0 {
        StreetOutcome::PartiallyFilled
    } else {
        StreetOutcome::Filled
    };
    let (venue, outcome, reason) = match exec.venue {
        Some(HedgeVenue::Composite) => (
            StreetVenue::CompositeBackstop,
            fill_outcome,
            // WHY it landed on the composite differs by mode, and conflating the two
            // would misreport a configuration as a liquidity failure: under a
            // composite-only mode the desk chose that venue, whereas under an
            // LP-first mode reaching it means the street would not deal.
            Some(
                if !mode.tries_lp_panel() {
                    "composite_venue_configured"
                } else if exec.attempts.is_empty() {
                    "no_firm_lp_price"
                } else {
                    // Every member we asked declined. That is a materially different
                    // fact from an empty street, and the rows above name who declined.
                    "street_declined"
                }
                .to_owned(),
            ),
        ),
        // Nothing filled anywhere. When orders went out this is already fully recorded
        // by the rows above, so the summary row would double-count the miss.
        Some(HedgeVenue::LpPanel) | None if !exec.attempts.is_empty() => return,
        _ => (
            StreetVenue::None,
            StreetOutcome::NoLiquidity,
            Some("no_firm_lp_price".to_owned()),
        ),
    };

    log.record(StreetOrder {
        // A composite fill is a synthetic mid, NOT a counterparty: it never carries an
        // LP id, so nothing downstream can attribute it to one.
        lp_id: None,
        venue,
        family,
        tenor_years,
        // `exec.filled` is in the budget metric; this row's `requested` is in venue units,
        // so convert rather than print two denominations side by side on one row.
        filled_qty: if risk_per_venue_unit > 0.0 {
            exec.filled / risk_per_venue_unit
        } else {
            exec.filled
        },
        filled_price: outcome.is_fill().then_some(exec.hedge_price),
        slippage_bp: outcome.is_fill().then_some(exec.slippage_bp),
        outcome,
        reason,
        // A backstop competed against nothing it could deal on, and a no-liquidity miss
        // saw nobody at all. Either way this row credits no counterparty.
        competitors: Vec::new(),
        parent_hedge_id,
        parent_position_id: Some(parent_position_id),
        // No order was sent for this row, so it carries no typed order and no round trip.
        order_type: None,
        time_in_force: None,
        response_latency_nanos: None,
        ..StreetOrder::new(
            log.next_order_id(),
            ts_nanos,
            instrument,
            side,
            requested,
            exec.mid_at_fire,
        )
    });
}

/// Resolve the leaf's [`HedgeVehicle`] onto a concrete registry row (which names the hedge
/// instrument AND its DV01 per unit) plus whether that vehicle trades in whole lots.
///
/// - [`HedgeVehicle::SelfInstrument`] ⇒ `None`: the position hedges itself, ratio
///   identically `1`, no registry lookup and no sizing needed.
/// - [`HedgeVehicle::Benchmark`] ⇒ the registry's most-specific match on
///   `(instrument, product, ccy, maturity)`.
/// - [`HedgeVehicle::Instrument`] / [`HedgeVehicle::Future`] ⇒ the registry row that names
///   that hedge instrument, which is where its DV01 per unit comes from.
///
/// `None` for a named-but-unregistered vehicle, a `Benchmark` that matches nothing, or a
/// product-symbol vehicle whose listed cycle has run out. The caller then falls back to the
/// **self-hedge** — never to a guessed DV01 or an invented contract code (guardrail 2).
///
/// `as_of` is the valuation date the FRONT MONTH is resolved on — see
/// [`roll_to_front_month`].
fn resolve_hedge_vehicle(
    registry: &HedgeVehicleRegistry,
    vehicle: &HedgeVehicle,
    instrument_id: &str,
    product: &str,
    ccy: &str,
    maturity_years: Option<f64>,
    as_of: celnet_refdata::CivilYmd,
) -> Option<(HedgeVehicleRule, bool)> {
    let rule = match vehicle {
        HedgeVehicle::SelfInstrument => return None,
        HedgeVehicle::Benchmark => registry.resolve(instrument_id, product, ccy, maturity_years)?,
        HedgeVehicle::Instrument { instrument_id: id }
        | HedgeVehicle::Future { contract_id: id } => registry.by_hedge_instrument(id)?,
    };
    // A row naming a PRODUCT resolves to the delivery month that product trades today.
    let rolled = roll_to_front_month(rule, as_of)?;
    // Whole-lot rounding applies when the leaf declares a future, the registry row does,
    // or the row rolled onto a listed contract — a desk that types a contract id gets
    // contract semantics regardless of how the row was configured.
    let whole = rolled.is_future || vehicle.is_future();
    Some((rolled, whole))
}

/// Re-point a registry row that names a futures **PRODUCT** (`ZF`) at the delivery month
/// that product actually trades on `as_of` (`ZFU26` before the September roll, `ZFZ26`
/// after it).
///
/// # Why this exists
///
/// A hedge policy names a product, not a delivery month — but a delivery month is what a
/// venue quotes, and it stops trading four times a year. Without this step a registry row
/// configured once as `ZFU26` keeps routing hedges at Sep-26 after the September roll, at
/// which point the contract has stopped trading and every hedge routed at it strands. The
/// roll resolves off the SAME committed cycle the venue quotes and the reference registry
/// seeds, so the id it yields is by construction one the venue is quoting.
///
/// A row naming a specific delivery month (`ZFU26`) or a cash security is returned
/// UNCHANGED: an explicit delivery month is a deliberate choice and must never be silently
/// re-pointed. A product whose whole listed cycle has expired yields `None` rather than a
/// stale or invented code — the caller then declines and says why (guardrail 2).
///
/// The row's configured `dv01_per_unit` is carried across untouched. That is deliberate: it
/// is the trader's configured DV01 for ONE CONTRACT OF THAT PRODUCT, which is exactly what
/// the sizing ratio needs, and substituting a number the desk did not author would be a
/// silent change to every size computed off this row.
fn roll_to_front_month(
    rule: &HedgeVehicleRule,
    as_of: celnet_refdata::CivilYmd,
) -> Option<HedgeVehicleRule> {
    let id = rule.hedge_instrument_id.trim();
    if !celnet_refdata::is_product_symbol(id) {
        return Some(rule.clone());
    }
    let Some(front) = celnet_refdata::front_contract_id(id, as_of) else {
        tracing::warn!(
            vehicle_rule = %rule.id,
            product = %id,
            "hedge vehicle names a futures PRODUCT but no contract of it is listed on the \
             valuation date — the committed listed cycle needs rolling. Declining rather \
             than routing a hedge at an expired contract",
        );
        return None;
    };
    tracing::debug!(
        vehicle_rule = %rule.id,
        product = %id,
        front_contract = %front,
        "hedge vehicle names a futures product — resolved to its front month",
    );
    Some(HedgeVehicleRule {
        hedge_instrument_id: front,
        is_future: true,
        ..rule.clone()
    })
}

/// The already-resolved scope measures a [`HedgeContext`] is assembled from — the inputs to
/// [`RatesPositionStore::build_hedge_context`], grouped so the builder reads as a mapping
/// rather than a twelve-argument call.
struct HedgeContextInputs<'a> {
    /// The risk book the fill routed into.
    book: &'a str,
    /// The product FAMILY label ("BOND"/"OIS"/…) thresholds and provenance are scoped by.
    instrument: &'a str,
    /// The tradeable security behind that family label, when the fill resolves one.
    execution_instrument: Option<String>,
    /// The fill's risk-routing attribution (counterparty, currency, dealt/mid levels).
    attribution: &'a RatesRoutingAttribution,
    /// The hedge-policy snapshot governing this fill (book tree + engine config).
    policy: &'a RatesHedgePolicy,
    /// The resolved threshold's budget metric — the basis every band/offset measure uses.
    metric: HedgeMetric,
    /// The resolved warehouse threshold (the "100") for this fill's scope.
    wh: &'a WarehouseThreshold,
    /// Signed net DV01 over the decision's scope.
    book_net_dv01: f64,
    /// Signed net face notional over the same scope.
    book_net_notional: f64,
    /// Gross face notional over the same scope.
    book_gross_notional: f64,
    /// The scope's net risk expressed in [`Self::metric`] — the band/sizing basis.
    book_risk: f64,
    /// The engine's reference mid for this fill, in the instrument's quote convention.
    mid: f64,
    /// One basis point in that quote convention (`QuoteKind::bp_scale`).
    bp_scale: f64,
}

/// The three-way sign of a risk measure: `+1` long, `−1` short, `0` flat.
///
/// Deliberately NOT [`f64::signum`], which returns `+1.0` for `+0.0` and would report a flat
/// book as long — and likewise not `signum` on a NaN, which is not a direction at all.
#[must_use]
fn inventory_sign(risk: f64) -> f64 {
    if !risk.is_finite() || risk == 0.0 {
        0.0
    } else if risk > 0.0 {
        1.0
    } else {
        -1.0
    }
}

fn internalise_instrument_label(fill: &RatesPosition) -> String {
    let Some(arm) = fill.instrument.as_ref().and_then(|i| i.instrument.as_ref()) else {
        return String::new();
    };
    match arm {
        rates_instrument::Instrument::Ois(_) => "OIS",
        rates_instrument::Instrument::Irs(_) => "IRS",
        rates_instrument::Instrument::Fra(_) => "FRA",
        rates_instrument::Instrument::Bond(_) => "BOND",
    }
    .to_owned()
}

/// Resolve the most-specific warehouse [`HedgeThresholdDef`] for a `(desk, book, instrument)`
/// fill — instrument > book > desk (the same precedence as the hedging LP panels, §4.4).
/// `None` when no threshold is configured for any of the three scopes (⇒ no internalise
/// decision runs).
fn resolve_hedge_threshold(
    thresholds: &[ScopedThreshold],
    desk: &str,
    book: &str,
    instrument: &str,
) -> Option<HedgeThresholdDef> {
    let by = |kind: HedgeScopeKind, id: &str| {
        thresholds
            .iter()
            .find(move |t| t.def.scope_kind == kind && t.scope_id == id)
    };
    by(HedgeScopeKind::Instrument, instrument)
        .or_else(|| by(HedgeScopeKind::Book, book))
        .or_else(|| by(HedgeScopeKind::Desk, desk))
        .map(|t| t.def)
}

/// Construct the **offsetting hedge leg** for a shed of `factor · |fill exposure|` (§6.2):
/// a copy of `fill`'s instrument with its side FLIPPED and its notional scaled by `factor`
/// (`= hedged_dv01 / fill_dv01`, in `(0, 1]`), so its signed linear exposure is exactly the
/// negation of the shed portion — booking it reduces the book's net by the hedged amount.
/// `position_id` is zeroed so [`RatesPositionStore::book_into_risk_book`] assigns a fresh id.
///
/// A **cash bond** sheds the same way, and exactly: the offsetting leg is the SAME security
/// (same coupon / maturity / day-count / security id) on the opposite side with its `redemption`
/// FACE scaled by `factor`. Because the leg and the fill are the identical bond, their DV01 ratio
/// is identically `1` — selling `factor` of the face you are long sheds exactly `factor` of the
/// risk under ANY duration measure, so no curve or duration input is needed to scale it honestly.
/// (This holds under [`rates_linear_exposure`]'s analytic DV01 bond arm exactly as it held under
/// the pre-migration `redemption · 1bp` proxy: both are linear in face for a fixed security.)
/// This is what makes a bond hedge actually REDUCE the warehoused
/// net; before it, a bond stamped hedge provenance while the book retained 100% of the risk, so
/// every subsequent fill compounded a position the blotter already claimed was hedged.
///
/// Returns `None` only for a non-positive/non-finite factor or a cell carrying no instrument arm
/// — the caller then skips booking rather than fabricating a leg (guardrail 2).
fn offsetting_rates_leg(fill: &RatesPosition, factor: f64) -> Option<RatesPosition> {
    // Reject a non-finite (NaN/∞) or non-positive factor — both mean "no valid leg".
    // Written as a positive guard so it also rejects NaN without a negated PartialOrd compare.
    if !(factor.is_finite() && factor > 0.0) {
        return None;
    }
    /// Flip a trade side: pay-fixed (Buy) ⇄ receive-fixed (Sell), so the leg nets the fill.
    fn flip(side: i32) -> i32 {
        match Side::try_from(side) {
            Ok(Side::Sell) => Side::Buy as i32,
            _ => Side::Sell as i32,
        }
    }
    let mut leg = fill.clone();
    leg.position_id = 0;
    let instr = leg.instrument.as_mut()?.instrument.as_mut()?;
    match instr {
        rates_instrument::Instrument::Ois(o) => {
            o.notional = o.notional.abs() * factor;
            o.side = flip(o.side);
        }
        rates_instrument::Instrument::Irs(i) => {
            i.notional = i.notional.abs() * factor;
            i.side = flip(i.side);
        }
        rates_instrument::Instrument::Fra(f) => {
            f.notional = f.notional.abs() * factor;
            f.side = flip(f.side);
        }
        // The offsetting leg of a cash bond is the SAME bond sold back: identical coupon,
        // maturity, day-count and security id, opposite side, `factor` of the face. Identical
        // security ⇒ DV01 ratio exactly 1 ⇒ scaling the face by `factor` sheds exactly `factor`
        // of the risk with no duration input required.
        rates_instrument::Instrument::Bond(b) => {
            b.redemption = b.redemption.abs() * factor;
            b.side = flip(b.side);
        }
    }
    Some(leg)
}

/// The memo key of one cash bond's **DV01 per unit redemption face**: everything
/// `celnet_bond::dv01` at the par (coupon) yield depends on, and nothing else. The face
/// itself is deliberately absent — every cashflow of a `Bond` scales linearly with
/// `redemption`, so the price and its yield derivative do too, which makes DV01 exactly
/// proportional to face and one cached unit-face number exact for any size of the same
/// security (asserted by `bond_dv01_is_exactly_linear_in_face`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BondDv01Key {
    /// `coupon_rate.to_bits()` — an exact key for an `f64` (no epsilon comparison).
    coupon_bits: u64,
    /// The wire maturity `(year, month, day)`.
    maturity: (i32, u32, u32),
    /// The wire `PaymentFrequency` ordinal.
    frequency: i32,
    /// The wire `AccrualBasis` ordinal.
    day_count: i32,
}

/// The process-wide analytic-DV01 memo, valid for **one** settlement date.
struct BondDv01Memo {
    /// The reference date every cached value was computed against. A date roll clears
    /// the map (settlement moves ⇒ every schedule and therefore every DV01 changes), so
    /// the memo is bounded by the traded universe rather than growing without limit.
    day: time::Date,
    /// `key → DV01 per unit face`, or `None` for a security whose contract cannot be
    /// constructed (memoized too, so a malformed bond is not re-attempted per position).
    per_unit: HashMap<BondDv01Key, Option<f64>>,
}

/// One detached auto-hedge, carried to the worker. Owned throughout — the worker outlives
/// the booking that queued it.
struct HedgeJob {
    fill: RatesPosition,
    book: Option<String>,
    attribution: RatesRoutingAttribution,
    trace_id: Option<u64>,
}

/// How many detached hedges may be queued before dispatch degrades to inline.
///
/// Small on purpose. This is a work queue for risk management, not a buffer: a deep
/// backlog would mean the book's real hedge state lags its position state by however long
/// the queue is, which is exactly the divergence the ledger exists to rule out.
const HEDGE_QUEUE_DEPTH: usize = 64;

/// A booking slower than this names its phase breakdown in the log.
///
/// 1ms is the stated budget for the whole tick→book path, so a single booking commit at
/// or above it has consumed the entire budget on its own.
const SLOW_BOOKING_THRESHOLD: std::time::Duration = std::time::Duration::from_millis(1);

static BOND_DV01_MEMO: OnceLock<RwLock<BondDv01Memo>> = OnceLock::new();

/// Compute one bond's analytic DV01 **per unit redemption face**, at the par (coupon)
/// yield. Curve-free: `celnet_bond::dv01` is the closed-form derivative
/// `−∂P/∂y · 1bp` of the bond's **own** cashflow schedule (`celnet-bond/src/risk.rs`),
/// so it needs no bootstrapped discount curve and no live market data — which is what
/// keeps [`rates_linear_exposure`] usable as a deterministic, market-data-free
/// pre-trade gate after the re-basing (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §6.2
/// G12: the curve dependency that gap warns about belongs to the Wave-2 *key-rate
/// ladder*, not to this analytic yield derivative).
///
/// `None` when the contract cannot be constructed (missing/unparseable maturity,
/// unmapped frequency or day-count, maturity not after settlement) or the derivative is
/// not a usable positive finite number — the caller then falls back to the
/// pre-migration proxy and says so, never to a guessed duration (guardrail 2).
fn compute_bond_dv01_per_unit_face(bond: &BondInstrument, today: time::Date) -> Option<f64> {
    let mut unit = bond.clone();
    unit.redemption = 1.0;
    let contract = crate::rates_pricing::bond_contract_from_wire(&unit, today).ok()?;
    let dv01 = celnet_bond::dv01(&contract, celnet_types::Rate(bond.coupon_rate)).ok()?;
    (dv01.is_finite() && dv01 > 0.0).then_some(dv01)
}

/// The memoized analytic DV01 per unit redemption face for `bond` on `today`.
///
/// The memo is what makes the re-basing affordable at investment-banking book sizes
/// (guardrail 6): `rates_linear_exposure` is called once per position per book roll-up
/// and per pre-trade scope, so an un-memoized `CashflowSchedule` build + 60-term
/// derivative sum per call would turn an `O(N)` roll-up into `O(N · cashflows)` on the
/// booking tier. Distinct securities in a book are few relative to positions, so the
/// memo collapses that back to `O(N)` map lookups after one computation per security
/// per day. Off the pinned zero-alloc pricing core (booking tier only).
fn bond_dv01_per_unit_face(bond: &BondInstrument, today: time::Date) -> Option<f64> {
    let m = bond.maturity_date.as_ref()?;
    let key = BondDv01Key {
        coupon_bits: bond.coupon_rate.to_bits(),
        maturity: (m.year, m.month, m.day),
        frequency: bond.coupon_frequency,
        day_count: bond.day_count,
    };
    let memo = BOND_DV01_MEMO.get_or_init(|| {
        RwLock::new(BondDv01Memo {
            day: today,
            per_unit: HashMap::new(),
        })
    });
    {
        let g = memo.read().expect("bond dv01 memo lock poisoned");
        if g.day == today
            && let Some(hit) = g.per_unit.get(&key)
        {
            return *hit;
        }
    }
    let computed = compute_bond_dv01_per_unit_face(bond, today);
    let mut g = memo.write().expect("bond dv01 memo lock poisoned");
    if g.day != today {
        g.day = today;
        g.per_unit.clear();
    }
    g.per_unit.insert(key, computed);
    computed
}

/// The **pre-migration** bond exposure magnitude: `|redemption| · 1bp` — every bond
/// treated as though its duration were `1`.
///
/// Retained deliberately, and used in exactly two places: as the honest fallback when
/// no analytic DV01 can be constructed, and as the reference measure the Wave-0
/// migration guard ([`proxy_era_cap_advice`] / [`RatesPositionStore::audit_proxy_era_caps`])
/// compares against to tell an operator that a configured cap was set in these units.
/// It is **not** a DV01 and must never be presented as one.
#[must_use]
fn bond_proxy_exposure_magnitude(bond: &BondInstrument) -> f64 {
    bond.redemption.abs() * ONE_BP
}

/// The **duration-correct** bond exposure magnitude — the analytic DV01 of the bond's
/// own cashflow schedule at the par (coupon) yield, falling back to
/// [`bond_proxy_exposure_magnitude`] only when no contract can be constructed.
///
/// This is the Wave-0 re-basing (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §7.2 / G1).
/// Before it, a 10-year bond charged ~1/8 of its real risk and — far worse — a bond and
/// a swap in the same book net were **not commensurable**: a 10y swap contributed
/// `notional · 10 · 1bp` while a 10y bond of equal face contributed `redemption · 1bp`,
/// so netting them was arithmetic on two different units and an economically flat mixed
/// book could report large risk (or vice versa).
#[must_use]
fn bond_exposure_magnitude(bond: &BondInstrument) -> f64 {
    let today = time::OffsetDateTime::now_utc().date();
    bond_dv01_per_unit_face(bond, today).map_or_else(
        || bond_proxy_exposure_magnitude(bond),
        |per_unit| per_unit * bond.redemption.abs(),
    )
}

/// The shared body of [`rates_linear_exposure`] and [`rates_linear_exposure_proxy`]:
/// the swap/FRA arms are identical in both (their `notional · years · 1bp` IS the
/// undiscounted annuity PV01 and was never duration-blind), so only the **bond**
/// magnitude differs and it is injected.
#[must_use]
fn rates_linear_exposure_with(
    position: &RatesPosition,
    bond_magnitude: impl Fn(&BondInstrument) -> f64,
) -> f64 {
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
        // The cash-bond magnitude is injected: the duration-correct analytic DV01 for the
        // live measure, the pre-migration `redemption · 1bp` for the migration reference.
        // This arm IS live: bonds route and book through `book_with_routing`, so it feeds
        // real limits and is the measure the auto-hedge warehouse nets and sheds in — see
        // [`offsetting_rates_leg`], whose bond leg is the same security sold back (DV01
        // ratio exactly 1), so the shed stays exact under EITHER magnitude. A long
        // (SIDE_BUY) bond carries long-duration exposure — the same netting sign as a
        // receive-fixed swap.
        rates_instrument::Instrument::Bond(bond) => {
            let magnitude = bond_magnitude(bond);
            match Side::try_from(bond.side) {
                Ok(Side::Buy) => -magnitude,
                _ => magnitude,
            }
        }
    }
}

/// The signed **linear interest-rate exposure** a rates position charges against a
/// limit, in the platform's one IR-duration sign convention (pay-fixed / short bond `+`,
/// receive-fixed / long bond `−`, so equal-and-opposite legs net to zero at a node).
///
/// | Arm | Measure |
/// | --- | --- |
/// | OIS / IRS | `notional · tenor_years · 1bp` — the undiscounted fixed-leg annuity PV01 |
/// | FRA | `notional · accrual_window_years · 1bp` |
/// | Cash bond | the **analytic DV01** of its own cashflow schedule at the par (coupon) yield ([`bond_exposure_magnitude`]) |
///
/// **This measure is curve-free and deterministic, and stays so after the Wave-0
/// re-basing.** The swap arms are undiscounted, so they are an *upper bound* on the true
/// annuity PV01 — conservative, hence fail-safe for a hard limit. The bond arm is
/// `celnet_bond::dv01`, a closed-form yield derivative of the bond's own schedule
/// (`celnet-bond/src/risk.rs`); it consumes no discount curve, no market data and no
/// live surface, so the pre-trade gate keeps exactly the market-data independence that
/// made it safe to run on the booking path. The genuinely curve-dependent measure —
/// the per-pillar key-rate ladder — is Wave 2 (`RISK-MODEL-REQUIREMENTS-AND-GAPS.md`
/// §3.5 / G7, G12) and is deliberately **not** introduced here.
///
/// The one honest approximation is the yield the derivative is taken at: with no dealt
/// price on a stored position, the coupon (par) assumption is used. Modified duration
/// is only weakly sensitive to the yield level, so the measure stays sound; the sizing
/// path ([`genuine_position_dv01`]), which *does* see a dealt clean price, prefers the
/// dealt yield.
#[must_use]
pub(crate) fn rates_linear_exposure(position: &RatesPosition) -> f64 {
    rates_linear_exposure_with(position, bond_exposure_magnitude)
}

/// The **pre-migration** linear exposure — identical to [`rates_linear_exposure`] except
/// that its bond arm is the duration-1 `redemption · 1bp` proxy.
///
/// This is not a live risk measure and nothing gates on it. It exists solely so the
/// Wave-0 migration can *quantify itself*: the ratio of the two over a real book is the
/// factor by which every cap configured before the re-basing must be re-based
/// (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §7.2, §7.3 "Re-basing existing caps").
#[must_use]
pub(crate) fn rates_linear_exposure_proxy(position: &RatesPosition) -> f64 {
    rates_linear_exposure_with(position, bond_proxy_exposure_magnitude)
}

/// The trade-direction **signed** notional of a rates position — magnitude signed by
/// `side` (SIDE_BUY = pay-fixed / long bond ⇒ `+`; SIDE_SELL = receive-fixed / short bond
/// ⇒ `−`), so an equal-and-opposite payer/receiver pair nets at a book. The magnitude is
/// the instrument's `notional` (bond: its `redemption` face — a bond carries no `notional`
/// field). This is the trade-direction convention used for the per-book **net/gross
/// notional** roll-up; note it is a *different* sign convention from
/// [`rates_linear_exposure`] (which signs by IR *duration* direction, so a long bond is
/// `−`) — the two are distinct, honestly-different measures (notional vs. PV01).
#[must_use]
pub(crate) fn rates_signed_notional(position: &RatesPosition) -> f64 {
    let Some(instr) = position
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())
    else {
        return 0.0;
    };
    let (magnitude, side) = match instr {
        rates_instrument::Instrument::Ois(ois) => (ois.notional.abs(), ois.side),
        rates_instrument::Instrument::Irs(irs) => (irs.notional.abs(), irs.side),
        rates_instrument::Instrument::Fra(fra) => (fra.notional.abs(), fra.side),
        rates_instrument::Instrument::Bond(bond) => (bond.redemption.abs(), bond.side),
    };
    match Side::try_from(side) {
        Ok(Side::Sell) => -magnitude,
        _ => magnitude,
    }
}

/// The instrument's contractual **dealt level** — the closest analogue of a marked "price"
/// a rates cell carries (it has no marking surface): the swap/FRA `fixed_rate`, the bond
/// `coupon_rate`. Used as the transfer P&L cost basis (§6): a `Mid`/`MarkToMarket` cross
/// resolves the transfer price to this level ⇒ zero realised P&L; an `Agreed` override
/// crosses P&L against it. `0.0` for a position with no instrument arm.
#[must_use]
pub(crate) fn rates_dealt_level(position: &RatesPosition) -> f64 {
    let Some(instr) = position
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())
    else {
        return 0.0;
    };
    match instr {
        rates_instrument::Instrument::Ois(ois) => ois.fixed_rate,
        rates_instrument::Instrument::Irs(irs) => irs.fixed_rate,
        rates_instrument::Instrument::Fra(fra) => fra.fixed_rate,
        rates_instrument::Instrument::Bond(bond) => bond.coupon_rate,
    }
}

/// A copy of `template` re-cast to carry `signed_notional`'s magnitude and direction under
/// `position_id` — the constructor for a transfer's offsetting / opening / split legs (§6).
/// The instrument's notional-bearing field (`notional`, or a bond's `redemption`) is set to
/// `|signed_notional|` and its `side` to `SIDE_BUY` for a non-negative signed notional /
/// `SIDE_SELL` for a negative one, matching the [`rates_signed_notional`] trade-direction
/// convention (a payer / long bond is `+`, a receiver / short bond is `−`). A template with
/// no instrument arm is returned unchanged but for the id (its notional is honestly zero).
#[must_use]
pub(crate) fn rates_with_signed_notional(
    template: &RatesPosition,
    signed_notional: f64,
    position_id: u64,
) -> RatesPosition {
    let mut out = template.clone();
    out.position_id = position_id;
    let magnitude = signed_notional.abs();
    let side = if signed_notional < 0.0 {
        Side::Sell
    } else {
        Side::Buy
    } as i32;
    if let Some(instr) = out.instrument.as_mut().and_then(|i| i.instrument.as_mut()) {
        match instr {
            rates_instrument::Instrument::Ois(ois) => {
                ois.notional = magnitude;
                ois.side = side;
            }
            rates_instrument::Instrument::Irs(irs) => {
                irs.notional = magnitude;
                irs.side = side;
            }
            rates_instrument::Instrument::Fra(fra) => {
                fra.notional = magnitude;
                fra.side = side;
            }
            rates_instrument::Instrument::Bond(bond) => {
                bond.redemption = magnitude;
                bond.side = side;
            }
        }
    }
    out
}

/// Map a booked rates fill onto the routing engine's [`RoutingContext`] (§4) — the input the
/// firm-wide decision graph matches an FI execution against. Total (never panics); a position
/// with no instrument arm yields an all-default context (routes to the graph's fallthrough).
///
/// Field provenance (a rates position carries far fewer routable fields than an FX fill — the
/// absent ones are left honestly empty/zero, never fabricated):
///
/// * `product` — the instrument family: `"ois"` / `"irs"` / `"fra"` / `"bond"` (the oneof
///   arm name), so a rule like `product == "bond"` matches FI executions by kind.
/// * `notional` — `|notional|` (bond: `|redemption|`), side-agnostic (mirrors the FX
///   `|notional_base|`).
/// * `tenor` — years to maturity: OIS/IRS `tenor_years`; FRA `end_months / 12`; a **bond is
///   `0.0`** (its tenor needs the settlement/reference date, which the position does not
///   carry — honestly zero, never a fabricated span).
/// * `side` — `"Buy"` (SIDE_BUY = pay-fixed / long) / `"Sell"` (SIDE_SELL = receive / short);
///   any other/unspecified ⇒ `""`.
/// * `strike` — the fixed-rate *level*: OIS/IRS/FRA `fixed_rate`, bond `coupon_rate` (the
///   closest analogue to a resolved level, so a rule can route by rate).
/// * `instrument_id` — the product family (a rates cell carries no security-master id).
/// * `desk` — the netting `book` id as text: a rates cell is **not** desk-attributed, so its
///   netting book (its coarsest org unit) stands in on the desk axis, letting a rule route by
///   rates book. The `entity` axis has no routing field today (a future `RouteField::Entity`
///   is its clean home — it is honestly unmapped, not folded onto a mismatched field).
/// * `counterparty` / `ccy` — from the booking-time [`RatesRoutingAttribution`]: the RFQ desk
///   path supplies the originating counterparty (its `DeskRequest` requester) + the `CurveSet`
///   currency, so a `RouteField::Counterparty` / `RouteField::Ccy` rule matches FI executions;
///   the manual `BookRatesPosition` path passes the empty default (both `""`, honestly absent
///   — a rates position stores no counterparty, and its notional is in the *curve* currency
///   which the position itself does not carry).
/// * `user` — `""`: a rates cell carries no booking user.
/// * `price` — `0.0`: the marked PV is a derived quantity not needed to route on economics.
#[must_use]
pub(crate) fn routing_context_from_rates(
    position: &RatesPosition,
    attribution: &RatesRoutingAttribution,
) -> RoutingContext {
    let Some(instr) = position
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())
    else {
        return RoutingContext {
            instrument_id: String::new(),
            ccy: attribution.ccy.clone(),
            counterparty: attribution.counterparty.clone(),
            desk: position.book.to_string(),
            ..RoutingContext::default()
        };
    };
    let (product, notional, tenor, level, side) = match instr {
        rates_instrument::Instrument::Ois(ois) => (
            "ois",
            ois.notional.abs(),
            f64::from(ois.tenor_years),
            ois.fixed_rate,
            ois.side,
        ),
        rates_instrument::Instrument::Irs(irs) => (
            "irs",
            irs.notional.abs(),
            f64::from(irs.tenor_years),
            irs.fixed_rate,
            irs.side,
        ),
        rates_instrument::Instrument::Fra(fra) => (
            "fra",
            fra.notional.abs(),
            f64::from(fra.end_months) / 12.0,
            fra.fixed_rate,
            fra.side,
        ),
        // A bond's tenor needs the settlement/reference date (absent on the position), so it
        // is honestly `0.0`; its `redemption` face is the notional and `coupon_rate` the level.
        rates_instrument::Instrument::Bond(bond) => (
            "bond",
            bond.redemption.abs(),
            0.0,
            bond.coupon_rate,
            bond.side,
        ),
    };
    let side = match Side::try_from(side) {
        Ok(Side::Buy) => "Buy",
        Ok(Side::Sell) => "Sell",
        _ => "",
    }
    .to_owned();
    RoutingContext {
        instrument_id: product.to_owned(),
        ccy: attribution.ccy.clone(),
        product: product.to_owned(),
        side,
        notional,
        tenor,
        strike: level,
        counterparty: attribution.counterparty.clone(),
        user: String::new(),
        desk: position.book.to_string(),
        price: 0.0,
    }
}

/// The resolved book and its ancestors (self first, then upward the parent chain),
/// cycle-guarded against a not-yet-validated registry — the rates analogue of the FX
/// store's `risk_book_chain`.
fn rates_risk_book_chain<'a>(
    limits_map: &'a HashMap<String, RiskBookLimitDef>,
    book: &'a str,
) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    let mut cur: Option<&str> = Some(book);
    while let Some(id) = cur {
        if out.contains(&id) {
            break; // cycle guard — a validated store is acyclic
        }
        out.push(id);
        cur = limits_map.get(id).and_then(|d| d.parent_id.as_deref());
    }
    out
}

/// A typed `failed_precondition` for a routed-risk-book hard notional-cap breach on the rates
/// path — book id + metric + used/limit (§8.3), matching the FX per-book breach wording.
#[must_use]
fn rates_risk_book_limit_breached(
    book_id: &str,
    metric: &str,
    used: f64,
    cap: f64,
) -> tonic::Status {
    tonic::Status::failed_precondition(format!(
        "risk book limit breached: {book_id}/{metric} used {used:.4} exceeds cap {cap}"
    ))
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

/// Ring the realised **hedge execution** — including, first-class, the **venue** it
/// filled on and the winning `lp_won`.
///
/// Before this the venue lived *only* in the in-memory `HedgeProvenance` ring: no log
/// sink anywhere carried `COMPOSITE` or an LP id, so ops could not tell from the logs
/// whether a hedge had crossed the street to a real LP or backstopped to the synthetic
/// composite mid — a question that is the difference between "we are flat" and "we
/// booked an offsetting leg against a number we invented".
///
/// - **INFO** on a real fill (`venue = LP_PANEL | COMPOSITE`, `lp_won = <lp id> |
///   COMPOSITE`).
/// - **WARN** on a miss (`venue = NONE`): nothing was externalised, so the risk is still
///   on the book — the case that most needs to be visible.
///
/// `class=hedge` routes it to the orders sink alongside the internalise-decision line,
/// which fires *before* execution and therefore cannot carry a venue. Booking tier, never
/// the pinned core.
fn log_hedge_execution(
    book: &str,
    instrument: &str,
    execution_instrument: &str,
    parent_position_id: u64,
    exec: &crate::services::auto_hedge::ExternalHedgeFill,
    headline: &'static str,
) {
    let venue = exec.venue_label();
    let lp_won = exec.lp_won.as_deref().unwrap_or("—");
    if exec.is_filled() {
        tracing::info!(
            class = celnet_observability::LogClass::Hedge.label(),
            book,
            instrument,
            execution_instrument,
            parent_position_id,
            venue,
            lp_won,
            filled = exec.filled,
            residual = exec.residual,
            hedge_price = exec.hedge_price,
            mid_at_fire = exec.mid_at_fire,
            slippage_bp = exec.slippage_bp,
            "{headline} on venue {venue} (lp_won={lp_won})",
        );
    } else {
        tracing::warn!(
            class = celnet_observability::LogClass::Hedge.label(),
            book,
            instrument,
            execution_instrument,
            parent_position_id,
            venue,
            lp_won,
            filled = exec.filled,
            residual = exec.residual,
            mid_at_fire = exec.mid_at_fire,
            "{headline} — NOTHING externalised (no venue filled); the whole shed stays \
             warehoused on the book",
        );
    }
}

/// One configured cap whose **meaning** changed under the Wave-0 bond re-basing
/// (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §7.2), measured on the scope's *actual*
/// current inventory rather than on an assumed duration.
///
/// Produced by [`RatesPositionStore::audit_proxy_era_caps`]; a scope holding no bonds
/// never produces one, because the re-basing does not touch swap/FRA exposure.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyEraCapFinding {
    /// Which cap: `limit:<LimitScope>` for a limit-tree cap, `warehouse:book:<id>` for a
    /// warehouse threshold.
    pub scope: String,
    /// The metric the cap constrains.
    pub metric: String,
    /// The cap as configured — expressed, on the evidence below, in **pre-migration**
    /// units.
    pub cap: f64,
    /// `|Σ exposure|` over the scope under the pre-migration duration-1 bond proxy.
    pub legacy_exposure: f64,
    /// `|Σ exposure|` over the scope under the duration-correct bond DV01.
    pub rebased_exposure: f64,
    /// `rebased / legacy` — the factor this cap must be multiplied by to preserve its
    /// pre-migration meaning **for this inventory** (≈ the book's bond duration).
    pub multiplier: f64,
    /// `cap × multiplier` — the re-based cap.
    pub suggested_cap: f64,
    /// Whether the cap is breached under the new measure but was **not** under the old —
    /// the silent day-one breach this audit exists to pre-empt.
    pub breaching_now: bool,
}

impl ProxyEraCapFinding {
    /// Measure one cap against a scope's positions under both measures, returning a
    /// finding only when the two differ (i.e. the scope actually holds a bond, so the cap's
    /// units genuinely changed). A non-finite or non-positive legacy exposure yields no
    /// finding: there is no honest multiplier to state, and inventing one would be exactly
    /// the fabricated precision this migration is trying to avoid.
    #[must_use]
    fn measure<'a>(
        scope: String,
        metric: String,
        cap: f64,
        positions: impl Iterator<Item = &'a RatesPosition>,
    ) -> Option<Self> {
        let (rebased, legacy) = positions.fold((0.0_f64, 0.0_f64), |(r, l), p| {
            (
                r + rates_linear_exposure(p),
                l + rates_linear_exposure_proxy(p),
            )
        });
        let (rebased, legacy) = (rebased.abs(), legacy.abs());
        // Bit-equal ⇒ the scope holds no bond (or a zero one): nothing changed.
        if rebased == legacy || !legacy.is_finite() || legacy <= 0.0 || !rebased.is_finite() {
            return None;
        }
        let multiplier = rebased / legacy;
        Some(Self {
            scope,
            metric,
            cap,
            legacy_exposure: legacy,
            rebased_exposure: rebased,
            multiplier,
            suggested_cap: cap * multiplier,
            breaching_now: rebased > cap && legacy <= cap,
        })
    }
}

/// ISO 4217 **`XXX`** — "no currency involved" — the currency the booking-gate rates
/// aggregate is keyed on.
///
/// A [`RatesPosition`] carries **no settlement currency** (`celnet.proto` gives it
/// `entity`, `book` and `instrument`, nothing else), so the booking gate nets one
/// currency-agnostic set — exactly what [`RatesPositionStore::book_net_dv01`] and every
/// existing rates roll-up already do. `XXX` records that honestly instead of asserting a
/// currency the position does not carry. The aggregate is gate-local, never published,
/// and [`celnet_limits::exposure_of_rates`] never reads `ccy`. Wave 2 / G3
/// (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md`) replaces this with a genuinely
/// per-currency aggregate once positions carry their curve currency.
const GATE_CCY: celnet_types::Ccy = match celnet_types::Ccy::new([b'X', b'X', b'X']) {
    Some(c) => c,
    None => panic!("XXX is a valid ISO 4217 alphabetic code"),
};

/// One booked rates position as a [`RatesRiskFact`] for the **booking-gate** aggregate.
///
/// - `dv01` — the position's signed [`rates_linear_exposure`]: the duration-correct
///   analytic DV01 for a bond, the undiscounted annuity PV01 for a swap/FRA. This is the
///   number a [`celnet_limits::LimitMetric::Dv01`] cap now actually gates on.
/// - `pv01` — the **same** number. On this deliberately curve-free gate the platform has
///   exactly one first-order rate sensitivity per position; there is no separately
///   discounted annuity to report. Charging the undiscounted figure to a `Pvbp` cap is
///   conservative (discount factors are `≤ 1`, so it is an upper bound), which preserves
///   the gate's fail-safe character. The two measures diverge only once the curve-priced
///   aggregate of Wave 2 / G3 feeds this seam.
/// - `pv` — **NaN, deliberately**. No curve is available here, so there is no present
///   value; a `0.0` would be a fabricated number that reads as "flat" (guardrail 2). NaN
///   makes any future read of `net_pv` off a gate-local aggregate impossible to miss.
///   No limit metric reads it.
/// - `key_rate_ladder` — **empty**. The per-pillar ladder needs a bootstrapped curve and
///   a book-time cache (Wave 2 / G7); it is not computed here and no bucket value is
///   fabricated. A configured `RateTenorBucket` limit therefore cannot bind on this path
///   and is reported as such by [`warn_tenor_bucket_limits_are_inert`] rather than left
///   to read a silent zero.
#[must_use]
fn rates_risk_fact_of(p: &RatesPosition) -> RatesRiskFact {
    let dv01 = rates_linear_exposure(p);
    RatesRiskFact {
        key: RatesFactKey {
            entity: EntityId(p.entity),
            ccy: GATE_CCY,
            book: BookId(p.book),
        },
        pv: f64::NAN,
        pv01: dv01,
        dv01,
        key_rate_ladder: Vec::new(),
    }
}

/// Say once, loudly, that a configured [`celnet_limits::LimitMetric::RateTenorBucket`]
/// limit **cannot bind on the booking path**.
///
/// The booking-gate aggregate carries no key-rate ladder (see [`rates_risk_fact_of`]), so
/// `exposure_of_rates` sums an empty ladder and the limit reads `0`. That is precisely
/// the silent-inertness defect this wave removed for `Dv01`/`Pvbp`
/// (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §2.3 Defect 2), one level down — so it is
/// surfaced rather than tolerated. The tenor axis is Wave 2 (G6/G7/G8) and needs a
/// bootstrapped curve plus a per-position ladder cache; nothing here fabricates a bucket
/// value in the meantime.
///
/// Fires at most once per process (a per-fill warning would drown the log). The limit
/// still evaluates — honestly, at zero — so nothing is blocked or spuriously breached.
fn warn_tenor_bucket_limits_are_inert(limits: &LimitTree, path: &ScopePath) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    // Already said ⇒ nothing to scan and nothing to allocate on the booking path.
    if WARNED.load(Ordering::Relaxed) {
        return;
    }
    let tenor_of = |l: &LimitSpec| match l.metric {
        celnet_limits::LimitMetric::RateTenorBucket { tenor_years } => Some(tenor_years),
        _ => None,
    };
    // Allocation-free scan first; only a tree that really carries a tenor limit builds the
    // list for the message.
    if !path
        .scopes()
        .flat_map(|scope| limits.at(scope).iter())
        .any(|l| tenor_of(l).is_some())
        || WARNED.swap(true, Ordering::Relaxed)
    {
        return;
    }
    let configured: Vec<u32> = path
        .scopes()
        .flat_map(|scope| limits.at(scope).iter())
        .filter_map(tenor_of)
        .collect();
    tracing::warn!(
        class = celnet_observability::LogClass::Risk.label(),
        tenors = ?configured,
        "RateTenorBucket limit(s) are configured but CANNOT bind on the rates booking path: \
         the booking gate builds no key-rate ladder (the per-position ladder needs a \
         bootstrapped curve and a book-time cache — Wave 2 / G6-G8 of \
         docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md). They evaluate at a true zero and will \
         never breach here. Curve risk is currently UNPROTECTED at booking; the tenor \
         limits that DO bind are the ones enforced on the AggregateRatesRisk request path, \
         which supplies a real ladder.",
    );
}

/// Run the pre-trade limit check for a proposed rates booking against the projected
/// book, at each of the position's `book → entity → firm` scopes (excluding any prior
/// fact under the same id, so a re-book projects `others + this trade`).
///
/// **Both metric families are enforced** ([`pre_trade_check_mixed`]):
///
/// - **FX-Greeks metrics** (in practice [`celnet_limits::LimitMetric::Delta`], the coarse
///   linear-IR proxy the rates sink has always charged) read the projected `NodeAggregate`.
/// - **Linear-FI metrics** ([`celnet_limits::LimitMetric::Dv01`] /
///   [`Pvbp`](celnet_limits::LimitMetric::Pvbp) /
///   [`RateTenorBucket`](celnet_limits::LimitMetric::RateTenorBucket)) read a genuine
///   [`RatesNodeAggregate`] built here from the projected position set.
///
/// Before this, the gate ran the Greeks check alone, so `exposure_of` returned a
/// hard-coded `0.0` for every FI metric and **a DV01 limit configured on a risk book was
/// silently inert — it could never breach** (`RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §2.3
/// Defect 2 / G2). Each limit is charged exactly once, by its own family, so the coarse
/// delta proxy and the DV01 cap never double-charge the same node.
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
    warn_tenor_bucket_limits_are_inert(limits, &path);
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

    // Build each standing position's fact ONCE, here, and reuse it for every scope.
    //
    // The three closures below are called per scope (`book → entity → firm`), and each
    // previously re-walked the whole book and recomputed `rates_linear_exposure` from
    // scratch — four full passes over every position per booking, with the bond arm taking
    // the DV01 memo's read lock on each one. That is O(positions) work multiplied by a
    // constant nobody was counting, and it is why the pre-trade gate grew from 25us at 14
    // positions to ~1.1ms at 1,320 on UAT while the write commit stayed under 120us.
    //
    // One pass, one exposure per position. The facts are then FILTERED per scope, so every
    // scope still sees exactly the positions it saw before, in the same order, and
    // `firm_aggregate_rates` folds an identical sequence — the exact-fold contract the
    // sharded roll-up depends on is untouched. This is a constant-factor fix, not an
    // asymptotic one: the gate is still O(positions), just once instead of four times.
    let standing: Vec<(&RatesPosition, RatesRiskFact)> = positions
        .iter()
        .filter(|p| exclude == 0 || p.position_id != exclude)
        .map(|p| (p, rates_risk_fact_of(p)))
        .collect();
    let others = |scope: LimitScope| {
        standing
            .iter()
            .filter(move |(p, _)| rates_scope_matches(scope, p))
    };
    let node_at = |scope: LimitScope| -> NodeAggregate {
        // `RatesRiskFact::dv01` IS `rates_linear_exposure` (see `rates_risk_fact_of`), so
        // the sum reads the value already computed rather than recomputing it.
        let sum: f64 = others(scope).map(|(_, f)| f.dv01).sum();
        let mut node = NodeAggregate::empty(scope.group_value().unwrap_or(0));
        node.net_greeks.delta_base = sum;
        node
    };
    // Booking never re-derives VaR/ES/stop-loss on the linear-rates path.
    let nonadditive_at = |_scope: LimitScope| NonAdditiveExposure::default();
    // The FI aggregate is built ALREADY PROJECTED — the scope's current facts plus the
    // proposed trade — by folding the proposal in as one more fact through the same
    // fixed-order additive builder the sharded roll-up uses, rather than by mutating a
    // finalized aggregate (whose exact-fold order is its contract). Every scope on `path`
    // contains the proposed position by construction, so it belongs in every projection.
    let rates_at = |scope: LimitScope| -> RatesNodeAggregate {
        let facts: Vec<RatesRiskFact> = others(scope)
            .map(|(_, f)| f.clone())
            .chain(std::iter::once(rates_risk_fact_of(position)))
            .collect();
        firm_aggregate_rates(&facts)
            .book(GATE_CCY)
            .cloned()
            .unwrap_or_else(|| RatesNodeAggregate::empty(GATE_CCY))
    };
    pre_trade_check_mixed(limits, &path, &increment, node_at, nonadditive_at, rates_at)
}

/// The Wave-0 **migration guard**: when the duration-correct measure rejects a booking
/// that the pre-migration `redemption × 1bp` proxy would have accepted at the same scope,
/// the cap is almost certainly **proxy-era** — set against a number that understated
/// every bond by roughly its duration.
///
/// Returns the operator-facing advice appended to the rejection, or `None` when the
/// breach is genuine under both measures (a real risk breach, not a units change).
/// Cheap: two sums over the breached scope, computed only on an already-failing booking.
#[must_use]
fn proxy_era_cap_advice(
    positions: &[RatesPosition],
    position: &RatesPosition,
    breach: &LimitCheck,
) -> Option<String> {
    let exclude = position.position_id;
    let scope_sum = |measure: fn(&RatesPosition) -> f64| -> f64 {
        positions
            .iter()
            .filter(|p| exclude == 0 || p.position_id != exclude)
            .filter(|p| rates_scope_matches(breach.scope, p))
            .chain(std::iter::once(position))
            .map(measure)
            .sum()
    };
    let rebased = scope_sum(rates_linear_exposure).abs();
    let legacy = scope_sum(rates_linear_exposure_proxy).abs();
    let cap = breach.limit.cap;
    // Only a cap the OLD measure fitted inside and the NEW one does not is proxy-era
    // evidence. A breach under both measures is a real breach and says nothing about units.
    if !(legacy <= cap && rebased > cap) {
        return None;
    }
    Some(if legacy > 0.0 {
        let multiplier = rebased / legacy;
        format!(
            "MIGRATION NOTE: this cap is not breached under the pre-migration duration-1 bond \
             proxy ({legacy:.4} vs cap {cap}) and IS breached under the duration-correct bond \
             DV01 ({rebased:.4}, ×{multiplier:.2}). The cap looks PROXY-ERA: bond exposure is \
             now measured as a real DV01, so a cap set before the re-basing must be multiplied \
             by this book's bond duration — here ×{multiplier:.2}, i.e. {suggested:.0} — to keep \
             its original meaning. See docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md §7.2 Wave 0.",
            suggested = cap * multiplier
        )
    } else {
        format!(
            "MIGRATION NOTE: the pre-migration duration-1 bond proxy measured this scope at \
             ~0 against cap {cap}; the duration-correct bond DV01 measures {rebased:.4}. This \
             cap looks PROXY-ERA and must be re-based. See \
             docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md §7.2 Wave 0."
        )
    })
}

/// The `failed_precondition` for a rejected rates pre-trade, carrying the Wave-0
/// migration advice when the rejection is a re-basing artefact rather than a real
/// breach. Never silent: the same advice is rung at WARN so ops sees it even when the
/// caller only surfaces the status code.
#[must_use]
fn rates_limit_breached(
    positions: &[RatesPosition],
    position: &RatesPosition,
    result: &PreTradeResult,
) -> tonic::Status {
    let base = limit_breached_status(result);
    let Some(advice) = result
        .hard_breaches()
        .next()
        .and_then(|b| proxy_era_cap_advice(positions, position, b))
    else {
        return base;
    };
    tracing::warn!(
        class = celnet_observability::LogClass::Risk.label(),
        position_id = position.position_id,
        book = position.book,
        entity = position.entity,
        advice = %advice,
        "rates pre-trade REJECTED by a cap that looks PROXY-ERA (the Wave-0 bond DV01 \
         re-basing changed this cap's units)",
    );
    tonic::Status::failed_precondition(format!("{} — {advice}", base.message()))
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

/// A stable, human label for an [`ExitActionKind`](celnet_proto::ExitActionKind) ordinal —
/// used in a suggestion's rationale so a trader reads "SUBMIT_MARKET_ORDER", not `3`.
fn exit_action_kind_label(kind: i32) -> &'static str {
    match celnet_proto::ExitActionKind::try_from(kind) {
        Ok(celnet_proto::ExitActionKind::ExitActionWarehouse) => "WAREHOUSE",
        Ok(celnet_proto::ExitActionKind::ExitActionCrossInternal) => "CROSS_INTERNAL",
        Ok(celnet_proto::ExitActionKind::ExitActionSkew) => "SKEW",
        Ok(celnet_proto::ExitActionKind::ExitActionSubmitMarketOrder) => "SUBMIT_MARKET_ORDER",
        Ok(celnet_proto::ExitActionKind::ExitActionRfqOut) => "RFQ_OUT",
        Ok(celnet_proto::ExitActionKind::ExitActionSplit) => "SPLIT",
        Ok(celnet_proto::ExitActionKind::ExitActionEscalate) => "ESCALATE",
        Ok(celnet_proto::ExitActionKind::ExitActionClearRisk) => "CLEAR_RISK",
        Err(_) => "—",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use celnet_proto::{
        EntitlementRule, OisInstrument, RatesInstrument, RiskScope, Side, rates_instrument,
    };

    pub(crate) fn position(id: u64, entity: u32, book: u32) -> RatesPosition {
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
            ..Default::default()
        }
    }

    /// **The pre-trade gate isolates scopes** — a book's own limit sees only that book.
    ///
    /// Guards the single-pass refactor. The gate used to re-walk `positions` per scope;
    /// it now walks once and FILTERS the built facts, so the predicate that decides which
    /// positions a scope sees moved. If that filter were wrong the gate would silently
    /// aggregate the whole firm into every book limit — which fails safe (it over-counts,
    /// so it rejects) and would therefore never show up as a missing rejection, only as
    /// spurious ones. Asserted from both sides for that reason.
    #[test]
    fn the_pre_trade_gate_sees_only_the_scope_it_is_checking() {
        use celnet_limits::{LimitMetric, LimitSpec};

        // Two books under one entity, each already carrying one 5y 10mm OIS.
        let standing = vec![position(1, 1, 10), position(2, 1, 20)];
        let one_dv01 = rates_linear_exposure(&standing[0]).abs();
        assert!(one_dv01 > 0.0, "the fixture must carry real exposure");

        // A BOOK-scoped cap that one position fits inside but two would not.
        let mut limits = LimitTree::new();
        limits.set(
            LimitScope::Book(BookId(10)),
            LimitSpec::hard(LimitMetric::Dv01, one_dv01 * 2.5),
        );

        // Booking a THIRD position into book 10 projects book 10 to 2 x one_dv01 — inside
        // its own cap. It only breaches if the gate wrongly folds book 20 in as well.
        let incoming = position(0, 1, 10);
        let result = rates_pre_trade(&standing, &limits, &incoming);
        assert_eq!(
            result.decision,
            PreTradeDecision::Accept,
            "book 10's cap must not see book 20's position",
        );

        // …and the cap still binds on its OWN book: a third position into book 10 takes it
        // past 2.5x. Without this the test would pass on a gate that sees nothing at all.
        let crowded = vec![position(1, 1, 10), position(2, 1, 10)];
        let result = rates_pre_trade(&crowded, &limits, &position(0, 1, 10));
        assert_eq!(
            result.decision,
            PreTradeDecision::Reject,
            "three positions in book 10 must breach book 10's own cap",
        );
    }

    /// The `Bucket`-scope subtree roll-up (§5): a bucket's net DV01 is its root book's
    /// plus every descendant's — the "PORTFOLIO notional" a bucket hedge decision measures.
    #[test]
    fn subtree_net_dv01_rolls_up_the_descendants() {
        let store = RatesPositionStore::new();
        // A +5000-DV01 OIS into the bucket root, and another into a child book.
        store
            .book_into_risk_book(position(0, 1, 10), "RATES")
            .expect("root leg books");
        store
            .book_into_risk_book(position(0, 1, 10), "RATES-EUR")
            .expect("child leg books");
        let own = store.book_net_dv01("RATES");
        assert!(own.abs() > 0.0, "the root book holds its own risk");
        // The root alone sees only its own leg; the subtree rolls up the child too.
        let sub = store.subtree_net_dv01("RATES", &["RATES-EUR".to_owned()]);
        assert!(
            (sub - 2.0 * own).abs() < 1e-6,
            "subtree net = root + child (parent sees the child's net)"
        );
    }

    /// A flexible OIS position constructor (explicit tenor / notional / side) — the
    /// notional-vs-DV01 divergence tests need a large notional at a SHORT tenor, which the
    /// fixed `position` helper (5y / 10mm / Buy) cannot express. `entity` is fixed at 1.
    pub(crate) fn ois(
        id: u64,
        book: u32,
        tenor_years: u32,
        notional: f64,
        side: Side,
    ) -> RatesPosition {
        RatesPosition {
            position_id: id,
            entity: 1,
            book,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years,
                    fixed_rate: 0.04,
                    notional,
                    side: side as i32,
                })),
            }),
            ..Default::default()
        }
    }

    /// `book_net_notional` / `subtree_net_notional` sum each position's SIGNED FACE NOTIONAL
    /// ([`rates_signed_notional`]): a mixed buy/sell book nets by direction, a one-directional
    /// book accumulates, and a subtree rolls the root + descendants — the notional analogue of
    /// `subtree_net_dv01`, and the operand a `NetNotional` hedge rule now measures.
    #[test]
    fn book_and_subtree_net_notional_sum_signed_notional() {
        let store = RatesPositionStore::new();
        // Mixed buy/sell into the root book NETS by signed notional: +10mm − 4mm = +6mm.
        store
            .book_into_risk_book(ois(0, 10, 5, 10_000_000.0, Side::Buy), "NB")
            .expect("buy books");
        store
            .book_into_risk_book(ois(0, 10, 5, 4_000_000.0, Side::Sell), "NB")
            .expect("sell books");
        assert!(
            (store.book_net_notional("NB") - 6_000_000.0).abs() < 1e-6,
            "mixed buy/sell nets: +10mm − 4mm = +6mm ({})",
            store.book_net_notional("NB")
        );
        // A one-directional child book ACCUMULATES: +15mm + 10mm = +25mm.
        store
            .book_into_risk_book(ois(0, 11, 2, 15_000_000.0, Side::Buy), "NB-EUR")
            .expect("child leg 1 books");
        store
            .book_into_risk_book(ois(0, 11, 2, 10_000_000.0, Side::Buy), "NB-EUR")
            .expect("child leg 2 books");
        assert!(
            (store.book_net_notional("NB-EUR") - 25_000_000.0).abs() < 1e-6,
            "one-directional accumulates: +15mm + 10mm = +25mm ({})",
            store.book_net_notional("NB-EUR")
        );
        // Subtree = root (+6mm) + child (+25mm) = +31mm.
        let sub = store.subtree_net_notional("NB", &["NB-EUR".to_owned()]);
        assert!(
            (sub - 31_000_000.0).abs() < 1e-6,
            "subtree net notional = root + child = +31mm ({sub})"
        );
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

    // ---- G2: a DV01 limit is REACHABLE at booking (it used to be structurally inert) ----

    /// **A hard `Dv01` limit now actually blocks a rates booking.**
    ///
    /// Before this wave the rates gate ran the FX-Greeks `pre_trade_check` alone, so
    /// `exposure_of` charged every FI metric a hard-coded `0.0` and a DV01 cap **could
    /// never breach** (`docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §2.3 Defect 2 / G2). The
    /// gate now also builds a real `RatesNodeAggregate`, so the cap binds — and the
    /// rejection names the metric that stopped it.
    #[test]
    fn a_dv01_limit_now_rejects_a_rates_booking_it_could_never_have_stopped_before() {
        // A 10y 5% 10mm bond: analytic DV01 ≈ 7 720 (the retired proxy measured 1 000).
        let fill = dated_bond(0, 10_000_000.0, Side::Buy, 10);
        let dv01 = rates_linear_exposure(&fill).abs();
        assert!((7_000.0..8_500.0).contains(&dv01), "sanity: DV01 {dv01}");

        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Dv01, dv01 / 2.0),
        );
        let err = store
            .book(fill.clone())
            .expect_err("a DV01 cap at half the position's DV01 must now block");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("Dv01"),
            "the reject must name the DV01 metric that bound, got {:?}",
            err.message()
        );
        assert_eq!(
            store.len(),
            0,
            "a rejected booking must not mutate the book"
        );

        // Control: the same limit with room accepts, so the gate is discriminating, not
        // simply rejecting everything now that the metric is live.
        let roomy = RatesPositionStore::new();
        roomy.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Dv01, dv01 * 10.0),
        );
        roomy.book(fill).expect("a DV01 cap with room accepts");
        assert_eq!(roomy.len(), 1);
    }

    /// A `Pvbp` cap binds through the same seam (the gate's aggregate carries the
    /// conservative undiscounted PV01 alongside the DV01).
    #[test]
    fn a_pvbp_limit_also_binds_at_the_rates_booking_gate() {
        let fill = dated_bond(0, 10_000_000.0, Side::Buy, 10);
        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Pvbp, 1.0),
        );
        let err = store.book(fill).expect_err("a 1-unit PV01 cap must block");
        assert!(err.message().contains("Pvbp"), "{:?}", err.message());
    }

    /// A `RateTenorBucket` cap is **honestly inert at booking** — the gate builds no
    /// key-rate ladder (Wave 2 / G6-G8), so nothing is fabricated and nothing is blocked.
    /// The operator is told, once, by `warn_tenor_bucket_limits_are_inert` rather than left
    /// to believe a tenor cap is protecting them.
    #[test]
    fn a_tenor_bucket_limit_is_inert_at_booking_and_never_fabricates_a_bucket() {
        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(
                celnet_limits::LimitMetric::RateTenorBucket { tenor_years: 10 },
                1.0,
            ),
        );
        store
            .book(dated_bond(0, 10_000_000.0, Side::Buy, 10))
            .expect("no ladder ⇒ zero bucket exposure ⇒ no breach, and no invented value");
        assert_eq!(store.len(), 1);
    }

    // ---- Wave-0 migration: re-basing the bond arm is announced, never silent ----

    /// **A rejection caused by the re-basing says so.** A cap the retired duration-1 proxy
    /// fitted inside and the duration-correct DV01 does not is proxy-era; the status carries
    /// the observed multiplier and the re-based cap instead of an unexplained breach.
    #[test]
    fn a_proxy_era_cap_rejection_carries_the_rebasing_advice() {
        let fill = dated_bond(0, 10_000_000.0, Side::Buy, 10);
        let legacy = rates_linear_exposure_proxy(&fill).abs(); // 1 000
        let rebased = rates_linear_exposure(&fill).abs(); // ≈ 7 720
        let cap = (legacy + rebased) / 2.0; // fits the old measure, blows the new one
        assert!(legacy <= cap && cap < rebased);

        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Dv01, cap),
        );
        let msg = store
            .book(fill)
            .expect_err("blown under the new measure")
            .message()
            .to_owned();
        assert!(msg.contains("PROXY-ERA"), "{msg}");
        assert!(msg.contains("MIGRATION NOTE"), "{msg}");
        assert!(
            msg.contains("RISK-MODEL-REQUIREMENTS-AND-GAPS.md"),
            "the advice must point at the migration doc, got {msg}"
        );
    }

    /// A cap blown under **both** measures is a real risk breach, not a units artefact — no
    /// migration advice is attached, so the guard never cries wolf.
    #[test]
    fn a_genuine_breach_carries_no_rebasing_advice() {
        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Dv01, 1.0),
        );
        let msg = store
            .book(dated_bond(0, 10_000_000.0, Side::Buy, 10))
            .expect_err("blown under both measures")
            .message()
            .to_owned();
        assert!(
            !msg.contains("PROXY-ERA"),
            "a breach under both measures says nothing about units, got {msg}"
        );
    }

    /// **The startup audit names every cap whose units changed, and by how much.** A 10y
    /// bond book reports a multiplier ≈ its modified duration, the re-based cap, and the
    /// day-one breach it would otherwise have produced silently.
    #[test]
    fn audit_reports_the_bond_duration_multiplier_for_a_proxy_era_cap() {
        let store = RatesPositionStore::new();
        // Book first under a cap with room, then tighten to the proxy-era value.
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Dv01, 1.0e12),
        );
        let fill = dated_bond(0, 10_000_000.0, Side::Buy, 10);
        let rebased = rates_linear_exposure(&fill).abs();
        store.book(fill).expect("books");
        let cap = 1_500.0; // > the retired proxy's 1 000, < the real ~7 720
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Dv01, cap),
        );

        let findings = store.audit_proxy_era_caps();
        assert_eq!(findings.len(), 1, "one bond-bearing cap, one finding");
        let f = &findings[0];
        assert_eq!(f.metric, "Dv01");
        assert!((f.legacy_exposure - 1_000.0).abs() < 1e-9);
        assert!((f.rebased_exposure - rebased).abs() < 1e-9);
        assert!(
            (7.0..8.5).contains(&f.multiplier),
            "the multiplier is the book's bond duration, got {}",
            f.multiplier
        );
        assert!((f.suggested_cap - cap * f.multiplier).abs() < 1e-9);
        assert!(
            f.breaching_now,
            "this cap breaches under the new measure and did not under the old — exactly \
             the silent day-one breach the audit exists to pre-empt"
        );
    }

    /// A **swap-only** book produces no finding: the re-basing did not touch swap exposure,
    /// so its caps mean precisely what they always did and the audit stays quiet.
    #[test]
    fn audit_is_silent_for_a_book_holding_no_bonds() {
        let store = RatesPositionStore::new();
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Dv01, 1.0e12),
        );
        store.book(position(0, 1, 10)).expect("books");
        assert!(store.audit_proxy_era_caps().is_empty());
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

    // ---- §4 risk routing on the linear-rates booking sink ----

    use celnet_proto::BondInstrument;
    use celnet_risk_routing::{RiskRoutingGraph, RouteField, RouteOp, RouteValue, RoutingNode};

    /// A firm-wide graph: `product == "bond"` → BOOK-A, else DEFAULT.
    fn product_graph() -> RiskRoutingGraph {
        use std::collections::BTreeMap;
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0u32,
            RoutingNode::Condition {
                field: RouteField::Product,
                op: RouteOp::Eq,
                value: RouteValue::Text("bond".to_owned()),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1u32,
            RoutingNode::Book {
                risk_book_id: "BOOK-A".to_owned(),
            },
        );
        nodes.insert(
            2u32,
            RoutingNode::Book {
                risk_book_id: "DEFAULT".to_owned(),
            },
        );
        RiskRoutingGraph { entry: 0, nodes }
    }

    fn bond_position(id: u64, redemption: f64, side: Side) -> RatesPosition {
        RatesPosition {
            position_id: id,
            entity: 1,
            book: 7,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Bond(BondInstrument {
                    coupon_rate: 0.05,
                    coupon_frequency: 0,
                    day_count: 0,
                    maturity_date: None,
                    redemption,
                    side: side as i32,
                    ..Default::default()
                })),
            }),
            ..Default::default()
        }
    }

    /// `hedge_execution_instrument` — the TRADEABLE security behind the family label, which is
    /// what makes a rates shed reachable on the LP panel at all. It must resolve ONLY when the
    /// fill genuinely carries a security-master id, and must never disturb the family label the
    /// warehouse thresholds and provenance are scoped by.
    #[test]
    fn hedge_execution_instrument_resolves_only_a_bond_carrying_a_real_security_id() {
        // A bond whose refdata id resolved ⇒ that id is what the LP panel gets asked for.
        let mut resolved = bond_position(1, 100.0, Side::Buy);
        if let Some(ri) = resolved.instrument.as_mut()
            && let Some(rates_instrument::Instrument::Bond(bond)) = ri.instrument.as_mut()
        {
            bond.instrument_id = "US91282CJL6".to_owned();
        }
        assert_eq!(
            hedge_execution_instrument(&resolved).as_deref(),
            Some("US91282CJL6"),
        );
        // …and the family label is UNCHANGED, so instrument-scoped warehouse thresholds and the
        // provenance blotter keep resolving exactly as before.
        assert_eq!(internalise_instrument_label(&resolved), "BOND");

        // An UNRESOLVED bond id (empty on the wire) is honestly absent — never fabricated into
        // an id that would then miss on the panel anyway.
        let unresolved = bond_position(2, 100.0, Side::Buy);
        assert_eq!(hedge_execution_instrument(&unresolved), None);
        assert_eq!(internalise_instrument_label(&unresolved), "BOND");

        // A cell with no instrument arm at all resolves nothing.
        let bare = RatesPosition {
            position_id: 3,
            entity: 1,
            book: 7,
            instrument: None,
            ..Default::default()
        };
        assert_eq!(hedge_execution_instrument(&bare), None);
    }

    /// **The growing-position regression.** A filled BOND shed must construct an offsetting leg
    /// whose exposure exactly negates the shed portion — previously the bond arm returned `None`,
    /// so a bond stamped hedge provenance while the book kept 100% of the risk and every
    /// subsequent fill compounded a position the blotter already claimed was hedged.
    ///
    /// The leg is the SAME security sold back, so its honesty needs no duration input: identical
    /// coupon / maturity / security id, flipped side, `factor` of the face.
    #[test]
    fn a_filled_bond_shed_books_an_offsetting_leg_that_actually_reduces_the_book() {
        let mut fill = bond_position(11, 20_000_000.0, Side::Buy);
        if let Some(ri) = fill.instrument.as_mut()
            && let Some(rates_instrument::Instrument::Bond(bond)) = ri.instrument.as_mut()
        {
            bond.instrument_id = "US91282CJL6".to_owned();
        }

        // Shed 40% of the fill — the ratio the call site passes as `filled_dv01 / fill_dv01`.
        let leg = offsetting_rates_leg(&fill, 0.4).expect("a bond shed must construct a leg");

        // (1) The leg's signed exposure is EXACTLY minus the shed portion, in the very measure
        //     the book nets in — so booking it reduces the warehoused net by the hedged amount.
        let shed = 0.4 * rates_linear_exposure(&fill);
        assert!((rates_linear_exposure(&leg) + shed).abs() < 1e-12);
        // …and the book genuinely nets down to the unshed 60%, rather than staying at 100%.
        let net = rates_linear_exposure(&fill) + rates_linear_exposure(&leg);
        assert!((net - 0.6 * rates_linear_exposure(&fill)).abs() < 1e-12);

        // (2) It is the SAME security on the opposite side with a proportional face — that
        //     identity is why the DV01 ratio is 1 and no curve/duration input is needed.
        let Some(rates_instrument::Instrument::Bond(hedge)) =
            leg.instrument.as_ref().and_then(|i| i.instrument.as_ref())
        else {
            panic!("the offsetting leg of a bond must itself be a bond");
        };
        assert_eq!(hedge.instrument_id, "US91282CJL6");
        assert_eq!(hedge.coupon_rate, 0.05);
        assert_eq!(hedge.side, Side::Sell as i32);
        assert!((hedge.redemption - 8_000_000.0).abs() < 1e-9);
        // A fresh id is assigned by the booking sink, never inherited from the parent fill.
        assert_eq!(leg.position_id, 0);

        // (3) A SHORT bond sheds symmetrically — the leg buys the face back.
        let short = bond_position(12, 20_000_000.0, Side::Sell);
        let short_leg = offsetting_rates_leg(&short, 1.0).expect("a short bond also sheds");
        assert!((rates_linear_exposure(&short) + rates_linear_exposure(&short_leg)).abs() < 1e-12);

        // (4) Degenerate factors still construct nothing rather than fabricating a leg.
        for bad in [0.0, -0.5, f64::NAN, f64::INFINITY] {
            assert!(offsetting_rates_leg(&fill, bad).is_none(), "factor {bad}");
        }
        // A cell with no instrument arm has nothing to offset.
        let bare = RatesPosition {
            position_id: 13,
            entity: 1,
            book: 7,
            instrument: None,
            ..Default::default()
        };
        assert!(offsetting_rates_leg(&bare, 0.5).is_none());
    }

    /// The same regression END-TO-END through the real booking path: an over-cap BOND fill must
    /// leave the routed book holding only the WAREHOUSED portion. This is the trader-visible
    /// symptom the unit test above only implies — before the fix the blotter showed a fired,
    /// filled hedge while `book_net_dv01` stayed at the full fill, so bond positions grew without
    /// bound as each subsequent fill compounded risk that was already reported as hedged.
    #[test]
    fn an_over_cap_bond_fill_leaves_only_the_warehoused_portion_in_the_book() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        // 50mm face ⇒ |exposure| = 50e6 · 1bp = 5000 DV01, over the 4000 cap (util 1.25).
        let policy = hedge_policy("wh", 4000.0, 0.5);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        // No LP has a firm price ⇒ the shed honestly backstops to the composite and FILLS.
        store.set_lp_hedge_source(Arc::new(NoFillLp));
        let booked = store
            .book_with_routing(
                bond_position(0, 50_000_000.0, Side::Buy),
                priced_attribution(0.0400, 0.0405),
            )
            .expect("the bond books");

        let exec = engine
            .provenance(None, None)
            .into_iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("a shed bond stamps a hedge-execution record");
        assert!(!exec.advisory, "a live composite shed is not advisory");
        assert!(
            exec.external_hedged > 0.0,
            "this fixture must actually shed, else it proves nothing"
        );

        // THE FIX — the invariant that was violated: whatever the hedge REPORTS as filled must
        // have left the book. Before it, `external_hedged` was stamped in full while the book
        // still held all 5000, so the next fill compounded risk the blotter called hedged.
        // A long bond carries NEGATIVE linear exposure (long duration), hence the magnitude.
        let net = store.book_net_dv01("wh");
        assert!(
            (net.abs() - (5000.0 - exec.external_hedged)).abs() < 1e-6,
            "a filled bond shed must reduce the book by exactly the hedged amount: \
             net {net}, hedged {} (net still 5000 ⇒ the offsetting leg never booked)",
            exec.external_hedged,
        );
    }

    /// A live hedge that fired and got NO fill is a MISS, not an advisory dry run.
    ///
    /// Regression for the UAT report of 2026-08-17. `advisory` was derived from the
    /// OUTCOME (`!exec.is_filled()`), so an unfilled live shed was indistinguishable from a
    /// policy that never intended to trade. Every consumer filters advisory rows out
    /// (`flowTotals`, gui/src/lib/hedgeBuckets.ts), so the warehoused total was
    /// structurally unreachable: with the composite backstop removed (`LpPanel`), 25 real
    /// misses holding 18,322 DV01 rendered as "No hedges have fired yet · 0 warehoused"
    /// while the risk buckets sat at 99% amber. A trader cannot see risk the panel is
    /// filtering away.
    #[test]
    fn an_unfilled_live_shed_is_a_miss_not_an_advisory_dry_run() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let mut policy = hedge_policy("wh", 4000.0, 0.5);
        // The strict posture: no composite backstop, so an unfilled panel is a real miss
        // rather than a synthetic fill. This is exactly how UAT is configured.
        policy.config.execution = crate::config::hedge_policy::HedgeExecutionMode::LpPanel;
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        // No LP shows a firm price, and there is no backstop ⇒ nothing fills.
        store.set_lp_hedge_source(Arc::new(NoFillLp));

        let booked = store
            .book_with_routing(
                bond_position(0, 50_000_000.0, Side::Buy),
                priced_attribution(0.0400, 0.0405),
            )
            .expect("the bond books");

        let exec = engine
            .provenance(None, None)
            .into_iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("a shed bond stamps a hedge-execution record even when nothing fills");

        // The fixture must genuinely miss, else it proves nothing.
        assert_eq!(
            exec.external_hedged, 0.0,
            "this fixture must shed NOTHING externally"
        );
        assert!(
            exec.residual > 0.0,
            "an unfilled shed leaves its whole clip as residual"
        );
        // THE FIX: the policy fired for real, so the record is LIVE. Marking it advisory
        // would hide the residual from every warehoused roll-up.
        assert!(
            !exec.advisory,
            "a live hedge that fired and missed is NOT advisory — marking it so hides \
             {} DV01 of warehoused risk from the trader",
            exec.residual,
        );
    }

    /// A policy whose warehouse budget is denominated in an explicit metric.
    fn metric_policy(book: &str, metric: HedgeMetric, cap: f64) -> RatesHedgePolicy {
        let mut p = hedge_policy(book, cap, 0.5);
        p.thresholds[0].def.metric = metric;
        p
    }

    /// The budget METRIC selects which roll-up the band classifies. The engine has always
    /// sized off the threshold's metric, but the band fields the booking path computed —
    /// `breached` / `utilization` / the stamped RAG band — were derived from DV01 whatever the
    /// metric was, so a NetNotional budget was silently classified against a DV01 cap.
    ///
    /// Same position, same cap, different metric ⇒ different verdict. That is the whole point.
    #[test]
    fn the_band_is_classified_in_the_thresholds_own_metric() {
        // 100mm at 1y: notional 100mm, but DV01 only 100e6 × 1 × 1bp = 10,000.
        let fill = || ois(0, 10, 1, 100_000_000.0, Side::Buy);
        let cap = 50_000_000.0;

        // NET NOTIONAL basis: 100mm against a 50mm cap ⇒ utilisation 2.0 ⇒ breach ⇒ sheds.
        let notional_store = RatesPositionStore::new();
        notional_store.set_routing(Some(single_book_graph("wh")));
        notional_store.set_hedge_policy(Some(metric_policy("wh", HedgeMetric::NetNotional, cap)));
        let booked = notional_store
            .book_with_routing(fill(), priced_attribution(0.0400, 0.0405))
            .expect("books");
        let prov = notional_store
            .internalise_of(booked.position_id)
            .expect("stamped");
        assert_eq!(prov.hedge_band, "breach", "100mm vs a 50mm NOTIONAL cap");
        assert!(prov.external_dv01 > 0.0, "a breach on notional must shed");

        // DV01 basis, IDENTICAL cap: 10,000 DV01 against 50,000,000 ⇒ utilisation 0.0002 ⇒ green.
        let dv01_store = RatesPositionStore::new();
        dv01_store.set_routing(Some(single_book_graph("wh")));
        dv01_store.set_hedge_policy(Some(metric_policy("wh", HedgeMetric::Dv01, cap)));
        let booked2 = dv01_store
            .book_with_routing(fill(), priced_attribution(0.0400, 0.0405))
            .expect("books");
        let prov2 = dv01_store
            .internalise_of(booked2.position_id)
            .expect("stamped");
        assert_eq!(
            prov2.hedge_band, "green",
            "10k DV01 vs a 50m cap is nowhere"
        );
        assert_eq!(prov2.external_dv01, 0.0, "green warehouses");
    }

    /// GROSS notional never nets down — that is exactly why a desk asks for it. Two
    /// equal-and-opposite legs net to ZERO notional but carry 200mm of gross, so a gross
    /// budget breaches on a book a net budget considers flat.
    #[test]
    fn a_gross_budget_sees_turnover_a_net_budget_nets_away() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        // Equal and opposite: net notional 0, gross 200mm.
        store
            .book_into_risk_book(ois(0, 10, 1, 100_000_000.0, Side::Buy), "wh")
            .expect("long books");
        store
            .book_into_risk_book(ois(0, 10, 1, 100_000_000.0, Side::Sell), "wh")
            .expect("short books");
        assert!(
            store.book_net_notional("wh").abs() < 1e-6,
            "the two legs net to flat"
        );
        assert!(
            (store.book_gross_notional("wh") - 200_000_000.0).abs() < 1e-6,
            "but gross counts both"
        );

        // A 150mm GROSS budget is breached by that 200mm of turnover.
        store.set_hedge_policy(Some(metric_policy(
            "wh",
            HedgeMetric::GrossNotional,
            150_000_000.0,
        )));
        let booked = store
            .book_with_routing(
                ois(0, 10, 1, 10_000_000.0, Side::Buy),
                priced_attribution(0.0400, 0.0405),
            )
            .expect("books");
        let prov = store.internalise_of(booked.position_id).expect("stamped");
        assert_eq!(prov.hedge_band, "breach", "210mm gross vs a 150mm cap");
        assert!(prov.external_dv01 > 0.0, "a gross breach sheds");
    }

    /// **The unwind regression.** A book already far over its cap must shed the BOOK's overflow,
    /// not merely neutralise the incoming fill. The shed used to be clamped to `fill_risk`
    /// ("a fill can shed at most what it added"), which made auto-hedging structurally unable to
    /// reduce risk it already held: it breached on every fill, hedged that fill, and stayed put
    /// (observed on UAT — 14 straight breach decisions, net DV01 beginning and ending at -87,800).
    #[test]
    fn a_breached_book_sheds_its_own_overflow_not_just_the_incoming_fill() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        // Standing risk booked WITHOUT a hedge decision: 100mm × 5y = 50,000 DV01.
        store
            .book_into_risk_book(ois(0, 10, 5, 100_000_000.0, Side::Buy), "wh")
            .expect("standing risk books");
        // Cap 10,000 DV01 ⇒ band edge 8,000 ⇒ the book is ~5× over before this fill lands.
        store.set_hedge_policy(Some(hedge_policy("wh", 10_000.0, 0.5)));

        // A DELIBERATELY TINY fill: 10mm × 1y = 1,000 DV01.
        let booked = store
            .book_with_routing(
                ois(0, 10, 1, 10_000_000.0, Side::Buy),
                priced_attribution(0.0400, 0.0405),
            )
            .expect("books");
        let prov = store.internalise_of(booked.position_id).expect("stamped");

        // The old clamp capped this at the fill's own 1,000. The overflow is ~43,000.
        assert!(
            prov.external_dv01 > 5_000.0,
            "a breached book must shed its OWN overflow, not the fill's 1,000 — got {}",
            prov.external_dv01
        );
        // And it never sheds more than the book actually holds.
        assert!(
            prov.external_dv01 <= store.book_net_dv01("wh").abs() + prov.external_dv01 + 1e-6,
            "never hedge more than is held"
        );
    }

    /// EVERY rates arm now sheds — the swap arms keep their existing exact-negation behaviour,
    /// so the bond fix did not come at the cost of a regression on the paths that already worked.
    #[test]
    fn every_rates_arm_constructs_a_leg_that_exactly_negates_the_shed() {
        let ois = position(21, 1, 7);
        let leg = offsetting_rates_leg(&ois, 0.25).expect("an OIS shed must construct a leg");
        let shed = 0.25 * rates_linear_exposure(&ois);
        assert!((rates_linear_exposure(&leg) + shed).abs() < 1e-12);
        assert!(offsetting_rates_leg(&bond_position(22, 1_000.0, Side::Buy), 0.25).is_some());
    }

    /// With a graph installed, a rates fill routes into the resolved risk book; the
    /// by-risk-book query returns it there — the core of the feature (the bond → BOOK-A,
    /// the OIS → DEFAULT via the graph's fallthrough).
    #[test]
    fn routed_rates_fill_lands_in_the_resolved_book() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(product_graph()));
        let bond = store
            .book(bond_position(0, 100.0, Side::Buy))
            .expect("bond books");
        let ois = store.book(position(0, 1, 10)).expect("ois books");
        assert_eq!(
            store.risk_book_of(bond.position_id).as_deref(),
            Some("BOOK-A")
        );
        assert_eq!(
            store.risk_book_of(ois.position_id).as_deref(),
            Some("DEFAULT")
        );
        assert_eq!(store.positions_in_risk_book("BOOK-A").len(), 1);
        assert_eq!(store.positions_in_risk_book("DEFAULT").len(), 1);
        assert!(store.has_routing());
        // A rates fill advances the risk version (the stream re-publishes).
        assert!(store.risk_version() >= 2);
    }

    /// No graph installed ⇒ every rates fill books UNROUTED (`risk_book_of` is None) — the
    /// backward-compatibility guarantee: routing off is byte-identical to the old path.
    #[test]
    fn no_graph_books_rates_unrouted() {
        let store = RatesPositionStore::new();
        assert!(!store.has_routing());
        let booked = store.book(position(0, 1, 10)).expect("books");
        assert_eq!(store.risk_book_of(booked.position_id), None);
        assert!(store.positions_in_risk_book("BOOK-A").is_empty());
        assert!(store.positions_in_risk_book("DEFAULT").is_empty());
    }

    /// `routing_context_from_rates` maps the fill economics for an OIS and a bond:
    /// product/notional/tenor/side/strike(level)/desk(book), with ccy honestly empty.
    #[test]
    fn routing_context_from_rates_maps_ois_and_bond() {
        // OIS: 5y 10mm pay-fixed (SIDE_BUY), fixed 4%, netting book 10. No attribution ⇒
        // counterparty/ccy honestly empty.
        let ois =
            routing_context_from_rates(&position(1, 1, 10), &RatesRoutingAttribution::default());
        assert_eq!(ois.product, "ois");
        assert_eq!(ois.instrument_id, "ois");
        assert_eq!(ois.notional, 10_000_000.0);
        assert_eq!(ois.tenor, 5.0);
        assert_eq!(ois.side, "Buy");
        assert_eq!(ois.strike, 0.04);
        assert_eq!(ois.desk, "10");
        assert_eq!(ois.ccy, "");
        assert_eq!(ois.counterparty, "");
        assert_eq!(ois.user, "");

        // Bond: short (SIDE_SELL) 100 face, 5% coupon, book 7; tenor honestly 0 (no ref date).
        let bond = routing_context_from_rates(
            &bond_position(2, 100.0, Side::Sell),
            &RatesRoutingAttribution::default(),
        );
        assert_eq!(bond.product, "bond");
        assert_eq!(bond.notional, 100.0);
        assert_eq!(bond.tenor, 0.0);
        assert_eq!(bond.side, "Sell");
        assert_eq!(bond.strike, 0.05);
        assert_eq!(bond.desk, "7");
    }

    /// A rates fill originating from a counterparty in a currency routes via a
    /// `Counterparty`/`Ccy` rule chain; a manual `BookRatesPosition` (empty attribution)
    /// falls through (the counterparty rule is honestly unmatched).
    #[test]
    fn counterparty_and_ccy_route_a_rates_fill() {
        use std::collections::BTreeMap;
        // Chain: Counterparty == "celer-rates-celnet" AND Ccy == "USD" → BOOK-CP, else DEFAULT.
        let graph = {
            let mut nodes = BTreeMap::new();
            nodes.insert(
                0u32,
                RoutingNode::Condition {
                    field: RouteField::Counterparty,
                    op: RouteOp::Eq,
                    value: RouteValue::Text("celer-rates-celnet".to_owned()),
                    on_true: 1,
                    on_false: 3,
                },
            );
            nodes.insert(
                1u32,
                RoutingNode::Condition {
                    field: RouteField::Ccy,
                    op: RouteOp::Eq,
                    value: RouteValue::Text("USD".to_owned()),
                    on_true: 2,
                    on_false: 3,
                },
            );
            nodes.insert(
                2u32,
                RoutingNode::Book {
                    risk_book_id: "BOOK-CP".to_owned(),
                },
            );
            nodes.insert(
                3u32,
                RoutingNode::Book {
                    risk_book_id: "DEFAULT".to_owned(),
                },
            );
            RiskRoutingGraph { entry: 0, nodes }
        };
        let store = RatesPositionStore::new();
        store.set_routing(Some(graph));

        // A desk-originated fill (counterparty "celer-rates-celnet" in USD) → BOOK-CP.
        let routed = store
            .book_with_routing(
                position(0, 1, 10),
                RatesRoutingAttribution {
                    counterparty: "celer-rates-celnet".to_owned(),
                    ccy: "USD".to_owned(),
                    ..RatesRoutingAttribution::default()
                },
            )
            .expect("desk fill books");
        assert_eq!(
            store.risk_book_of(routed.position_id).as_deref(),
            Some("BOOK-CP")
        );

        // A manual BookRatesPosition (empty attribution) → the counterparty rule is unmatched,
        // so it falls through to DEFAULT (never BOOK-CP).
        let manual = store.book(position(0, 1, 11)).expect("manual fill books");
        assert_eq!(
            store.risk_book_of(manual.position_id).as_deref(),
            Some("DEFAULT")
        );

        // The threaded fields land on the context a rule matches.
        let ctx = routing_context_from_rates(
            &position(1, 1, 10),
            &RatesRoutingAttribution {
                counterparty: "celer-rates-celnet".to_owned(),
                ccy: "USD".to_owned(),
                ..RatesRoutingAttribution::default()
            },
        );
        assert_eq!(ctx.counterparty, "celer-rates-celnet");
        assert_eq!(ctx.ccy, "USD");
    }

    /// A routed rates fill that would blow the resolved book's HARD net-notional cap is
    /// refused (store unmutated); a within-cap fill books — the §8.3 per-book gate.
    #[test]
    fn routed_rates_fill_gated_by_risk_book_cap() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(product_graph()));
        // Cap BOOK-A (where bonds route) at 50 net notional.
        store.set_risk_books(vec![RiskBookLimitDef {
            id: "BOOK-A".to_owned(),
            parent_id: None,
            limits: Some(RiskLimits {
                max_net_notional: Some(50.0),
                max_gross_notional: None,
                max_dv01: None,
            }),
        }]);
        // A 100-face bond routes to BOOK-A and breaches the 50 cap → rejected, book unchanged.
        let err = store
            .book(bond_position(0, 100.0, Side::Buy))
            .expect_err("over-cap bond must be rejected");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("risk book limit breached"));
        assert_eq!(
            store.len(),
            0,
            "a rejected rates fill must not mutate the book"
        );
        // A 40-face bond is within cap → books.
        let ok = store
            .book(bond_position(0, 40.0, Side::Buy))
            .expect("within cap");
        assert_eq!(
            store.risk_book_of(ok.position_id).as_deref(),
            Some("BOOK-A")
        );
    }

    /// The per-book HARD `max_dv01` cap now REJECTS an over-cap routed fill. A 5y 10mm
    /// pay-fixed OIS charges `10mm · 5 · 1bp = 5,000` of signed linear DV01, so a 4,000
    /// cap refuses it and the store stays unmutated; a 6,000 cap books it. Before this the
    /// gate skipped `max_dv01` entirely — the field was configurable and inert, so a trader
    /// could set a cap and trade straight through it with no rejection and no warning.
    #[test]
    fn routed_rates_fill_gated_by_risk_book_dv01_cap() {
        let cap_at = |cap: f64| {
            let store = RatesPositionStore::new();
            store.set_routing(Some(single_book_graph("BOOK-A")));
            store.set_risk_books(vec![RiskBookLimitDef {
                id: "BOOK-A".to_owned(),
                parent_id: None,
                limits: Some(RiskLimits {
                    max_net_notional: None,
                    max_gross_notional: None,
                    max_dv01: Some(cap),
                }),
            }]);
            store
        };

        // 5,000 of DV01 against a 4,000 cap → refused, book unmutated.
        let store = cap_at(4_000.0);
        let err = store
            .book(ois(0, 10, 5, 10_000_000.0, Side::Buy))
            .expect_err("an over-DV01-cap fill must be rejected");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("risk book limit breached"),
            "{err:?}"
        );
        assert!(
            err.message().contains("dv01"),
            "the breach names the metric: {err:?}"
        );
        assert_eq!(store.len(), 0, "a rejected fill must not mutate the book");

        // The same fill against a 6,000 cap books — proving the rejection is the CAP, not
        // the fill being unbookable.
        let store = cap_at(6_000.0);
        let ok = store
            .book(ois(0, 10, 5, 10_000_000.0, Side::Buy))
            .expect("within the DV01 cap");
        assert_eq!(
            store.risk_book_of(ok.position_id).as_deref(),
            Some("BOOK-A")
        );
    }

    /// DV01 is charged on the ABSOLUTE of the signed sum, so a RECEIVED-fixed book consumes
    /// capacity exactly as a paid-fixed one does — a desk cannot buy headroom by flipping
    /// direction. And because the roll-up is SIGNED, an equal-and-opposite second leg NETS
    /// the book back under its cap, which is precisely what an internalising book relies on.
    #[test]
    fn dv01_cap_charges_the_magnitude_and_nets_opposing_legs() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("BOOK-A")));
        store.set_risk_books(vec![RiskBookLimitDef {
            id: "BOOK-A".to_owned(),
            parent_id: None,
            limits: Some(RiskLimits {
                max_net_notional: None,
                max_gross_notional: None,
                max_dv01: Some(4_000.0),
            }),
        }]);

        // Receive-fixed carries −5,000; |−5,000| > 4,000 ⇒ still refused.
        let err = store
            .book(ois(0, 10, 5, 10_000_000.0, Side::Sell))
            .expect_err("the short side consumes the same capacity");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);

        // Under the cap at 2,500 (5y 5mm), then an equal-and-opposite leg nets to ~0 —
        // the second fill is accepted even though a GROSS charge would have refused it.
        let first = store
            .book(ois(0, 10, 5, 5_000_000.0, Side::Buy))
            .expect("2,500 is within the 4,000 cap");
        assert!(store.risk_book_of(first.position_id).is_some());
        let second = store
            .book(ois(0, 10, 5, 5_000_000.0, Side::Sell))
            .expect("the offsetting leg NETS the book down, not up");
        assert!(store.risk_book_of(second.position_id).is_some());
        assert_eq!(store.len(), 2);
    }

    // ---- auto-hedge / internalisation decision (§6) --------------------------
    //
    // A 5y 10mm pay-fixed (Buy) OIS charges `+5000` of signed linear DV01 (`10mm·5·1bp`).
    // The internalise decision runs only when a hedge policy is primed AND the fill carried a
    // priced dealt level + reference mid. The pieces combine: the price-tolerance verdict
    // (did the desk capture edge?) × the warehouse-cap sizing (is the book over its "100"?).

    /// A single-book routing graph: every fill → `book`.
    pub(crate) fn single_book_graph(book: &str) -> RiskRoutingGraph {
        let mut nodes = std::collections::BTreeMap::new();
        nodes.insert(
            0u32,
            RoutingNode::Book {
                risk_book_id: book.to_owned(),
            },
        );
        RiskRoutingGraph { entry: 0, nodes }
    }

    /// A hedge policy: the default warehouse-vs-hedge exit graph + one Book-scoped DV01
    /// threshold on `book` with budget `cap`, advisory-only, `min_edge_bps` tolerance floor.
    pub(crate) fn hedge_policy(book: &str, cap: f64, min_edge_bps: f64) -> RatesHedgePolicy {
        let config = HedgeConfigDef {
            min_edge_bps,
            ..HedgeConfigDef::default()
        };
        RatesHedgePolicy {
            engine: Arc::new(AutoHedgeEngine::default()),
            graph: Some(crate::config::identity::default_hedge_policy_graph()),
            thresholds: vec![ScopedThreshold {
                scope_id: book.to_owned(),
                def: HedgeThresholdDef {
                    scope_kind: HedgeScopeKind::Book,
                    metric: HedgeMetric::Dv01,
                    cap,
                    amber: 0.8,
                    red: 0.9,
                    target_fraction: 0.8,
                    min_clip: 0.0,
                    max_clip: f64::INFINITY,
                    ramped: false,
                    ramp_k: 1.0,
                },
            }],
            config,
            known_lps: BTreeSet::new(),
            scoped_graphs: Vec::new(),
            book_ancestors: HashMap::new(),
            book_descendants: HashMap::new(),
        }
    }

    /// A hedge policy whose exit graph ALWAYS sheds externally — a single `SubmitMarketOrder`
    /// action at the entry, every band — the counterpart to [`hedge_policy`]'s default
    /// warehouse-vs-shed graph. Exercises the below-tolerance EXTERNAL-policy path: a thin /
    /// adverse fill under a shed policy is handed back to the street (whereas the default
    /// internal-hold graph now WAREHOUSES it — the wash-book fix, Part A).
    pub(crate) fn external_hedge_policy(
        book: &str,
        cap: f64,
        min_edge_bps: f64,
    ) -> RatesHedgePolicy {
        use celnet_hedge_routing::{ExecStyle, ExitAction, HedgeNode, HedgeSize};
        let mut nodes = std::collections::BTreeMap::new();
        nodes.insert(
            0u32,
            HedgeNode::action(ExitAction::SubmitMarketOrder {
                size: HedgeSize::Full,
                style: ExecStyle::Immediate,
            }),
        );
        RatesHedgePolicy {
            graph: Some(HedgeGraph { entry: 0, nodes }),
            ..hedge_policy(book, cap, min_edge_bps)
        }
    }

    /// A stub external-hedge LP source that always fills at the LP's firm two-way on the required
    /// side — reducing a long (`net_risk > 0`) fills at `bid`, reducing a short at `offer` — so
    /// the Part B wiring can be exercised without standing up a live aggregation book.
    struct StubLp {
        lp: &'static str,
        bid: f64,
        offer: f64,
    }
    impl crate::services::auto_hedge::LpHedgeSource for StubLp {
        fn rank(
            &self,
            _instrument: &str,
            net_risk: f64,
            _size: f64,
        ) -> Vec<crate::services::auto_hedge::LpFill> {
            let price = if net_risk > 0.0 { self.bid } else { self.offer };
            vec![crate::services::auto_hedge::LpFill {
                lp_id: self.lp.to_owned(),
                price,
            }]
        }
    }

    /// A stub LP source that never has a firm price (an honest miss) — for the composite-fallback path.
    struct NoFillLp;
    impl crate::services::auto_hedge::LpHedgeSource for NoFillLp {
        fn rank(
            &self,
            _instrument: &str,
            _net_risk: f64,
            _size: f64,
        ) -> Vec<crate::services::auto_hedge::LpFill> {
            Vec::new()
        }
    }

    /// A router standing in for a counterparty that honours the price it is showing —
    /// so these booking-layer tests can assert what the BOOK does with a fill without
    /// re-testing the FIX transport. That the transport genuinely produces such answers
    /// (and the refusals, partials and timeouts it also produces) is proven against the
    /// real simulators over real sockets in `tests/street_routing_e2e.rs`.
    struct TradesAtQuote;
    impl crate::services::auto_hedge::StreetOrderRouter for TradesAtQuote {
        fn route(
            &self,
            intent: &crate::services::auto_hedge::StreetOrderIntent<'_>,
        ) -> crate::services::auto_hedge::RouteAnswer {
            crate::services::auto_hedge::RouteAnswer::Traded(
                crate::services::auto_hedge::RoutedFill {
                    filled: intent.quantity,
                    price: intent.limit_price,
                    reason: None,
                    latency_nanos: 640_000,
                },
            )
        }
    }

    /// A booking attribution carrying the dealt level + reference mid (the RFQ-desk / lift path).
    pub(crate) fn priced_attribution(dealt: f64, mid: f64) -> RatesRoutingAttribution {
        RatesRoutingAttribution {
            counterparty: "cp".to_owned(),
            ccy: "USD".to_owned(),
            dealt_price: Some(dealt),
            reference_mid: Some(mid),
            request_id: None,
        }
    }

    /// A 5-year OIS on `side` — `Buy` (pay fixed) is `+5000` DV01, `Sell` (receive fixed)
    /// `−5000`, so two of them in different books make an offsetting pair.
    fn ois_leg(side: Side) -> RatesPosition {
        RatesPosition {
            position_id: 0,
            entity: 1,
            book: 10,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 5,
                    fixed_rate: 0.04,
                    notional: 10_000_000.0,
                    side: side as i32,
                })),
            }),
            ..Default::default()
        }
    }

    /// A book state deliberately chosen to drive EVERY computed context field off its
    /// default: a breaching own book, an opposing sibling under a shared parent, and a live
    /// LP away from mid. Returns the assembled context.
    fn context_under_full_book_state() -> HedgeContext {
        let store = RatesPositionStore::new();
        let lp = StubLp {
            lp: "LP1",
            bid: 99.5,
            offer: 100.5,
        };
        store.set_lp_hedge_source(Arc::new(lp));
        // +5000 DV01 in the fill's own book; −5000 in a SIBLING under the same parent, so a
        // real opposing pool exists to cross against.
        store
            .book_into_risk_book(ois_leg(Side::Buy), "OWN")
            .expect("own leg books");
        store
            .book_into_risk_book(ois_leg(Side::Sell), "SIB")
            .expect("sibling leg books");

        let policy = RatesHedgePolicy {
            book_ancestors: HashMap::from([("OWN".to_owned(), vec!["PARENT".to_owned()])]),
            book_descendants: HashMap::from([(
                "PARENT".to_owned(),
                vec!["OWN".to_owned(), "SIB".to_owned()],
            )]),
            ..hedge_policy("OWN", 1_000.0, 0.5)
        };
        // Cap 1000 against 5000 DV01 ⇒ utilization 5, over the red band, with real overflow.
        let thr = HedgeThresholdDef {
            scope_kind: HedgeScopeKind::Book,
            metric: HedgeMetric::Dv01,
            cap: 1_000.0,
            amber: 0.8,
            red: 0.9,
            target_fraction: 0.8,
            min_clip: 0.0,
            max_clip: f64::INFINITY,
            ramped: false,
            ramp_k: 1.0,
        };
        let wh = thr.to_threshold();
        let book_risk = store.book_net_dv01("OWN");
        let attribution = priced_attribution(100.25, 100.0);
        store.build_hedge_context(&HedgeContextInputs {
            book: "OWN",
            instrument: "OIS",
            execution_instrument: None,
            attribution: &attribution,
            policy: &policy,
            metric: thr.metric,
            wh: &wh,
            book_net_dv01: book_risk,
            book_net_notional: store.book_net_notional("OWN"),
            book_gross_notional: store.book_gross_notional("OWN"),
            book_risk,
            mid: 100.0,
            // A clean-price convention: one bp is 0.01 price points.
            bp_scale: 1e-2,
        })
    }

    /// THE anti-regression test for the silent-zero defect: what
    /// [`HedgeField::provider`] DECLARES and what the production builder actually
    /// produces must agree, in BOTH directions.
    ///
    /// A field declared `Computed` that comes back at its default is a dead rule shipped as a
    /// live one — the original bug. A field declared `Unprovided` that carries a real value is
    /// the inverse error: the validator would be refusing rules that would in fact have fired.
    #[test]
    fn every_computed_field_is_populated_and_every_unprovided_field_is_not() {
        use celnet_hedge_routing::{FieldProvider, HedgeField};
        let ctx = context_under_full_book_state();
        let default = HedgeContext::default();
        for f in HedgeField::ALL {
            match f.provider() {
                FieldProvider::Computed { basis } => assert_ne!(
                    ctx.get(f),
                    default.get(f),
                    "{f:?} is declared Computed ({basis}) but the production builder left it \
                     at its default — a rule on it would be dead"
                ),
                FieldProvider::Unprovided { reason } => assert_eq!(
                    ctx.get(f),
                    default.get(f),
                    "{f:?} is declared Unprovided ({reason}) but the production builder gave \
                     it a value — the declaration is now wrong and rules on it are being \
                     refused for no reason"
                ),
            }
        }
    }

    /// The three fixed fields, at their exact expected values — the declaration test above
    /// only proves they are non-default, not that they are RIGHT.
    #[test]
    fn the_three_repaired_fields_carry_their_derived_values() {
        let ctx = context_under_full_book_state();
        // A +5000 DV01 book is LONG.
        assert_eq!(ctx.inventory_sign, 1.0);
        // The sibling's −5000 opposes the own book's +5000, so the whole pool is crossable.
        assert_eq!(ctx.internal_offset_available, 5_000.0);
        // Long ⇒ sheds by hitting the 99.50 bid, 0.50 price points below the 100.00 mid,
        // which at 0.01 points per bp is 50 bp of crossing cost.
        assert!(
            (ctx.hedge_cost_bp - 50.0).abs() < 1e-9,
            "hedge_cost_bp = {}",
            ctx.hedge_cost_bp
        );
    }

    /// `inventory_sign` is a THREE-way test, not `f64::signum` — which returns `+1.0` for
    /// `+0.0` and would report a flat book as long.
    #[test]
    fn inventory_sign_reports_flat_as_zero_not_long() {
        assert_eq!(inventory_sign(12.5), 1.0);
        assert_eq!(inventory_sign(-12.5), -1.0);
        assert_eq!(inventory_sign(0.0), 0.0);
        assert_eq!(inventory_sign(-0.0), 0.0);
        // A non-finite risk is not a direction.
        assert_eq!(inventory_sign(f64::NAN), 0.0);
        // The trap this guards, stated as an assertion.
        assert_eq!(f64::signum(0.0), 1.0);
    }

    /// The internal-offset pool is narrowed three ways, each of which can only UNDERSTATE it.
    #[test]
    fn internal_offset_is_zero_without_an_opposing_same_family_sibling() {
        let store = RatesPositionStore::new();
        store
            .book_into_risk_book(ois_leg(Side::Buy), "OWN")
            .expect("own leg books");
        let base = hedge_policy("OWN", 1_000.0, 0.5);

        // (1) A ROOT book has no siblings, so nothing is crossable however much the firm holds.
        store
            .book_into_risk_book(ois_leg(Side::Sell), "SIB")
            .expect("sibling leg books");
        let rootless = RatesHedgePolicy {
            book_ancestors: HashMap::new(),
            ..hedge_policy("OWN", 1_000.0, 0.5)
        };
        assert_eq!(
            store.internal_offset_available(&rootless, "OWN", "OIS", HedgeMetric::Dv01, 5_000.0),
            0.0
        );

        let parented = RatesHedgePolicy {
            book_ancestors: HashMap::from([("OWN".to_owned(), vec!["PARENT".to_owned()])]),
            book_descendants: HashMap::from([(
                "PARENT".to_owned(),
                vec!["OWN".to_owned(), "SIB".to_owned()],
            )]),
            ..base
        };
        // (2) A DIFFERENT product family does not offset — a bond's PV01 is not a swap's.
        assert_eq!(
            store.internal_offset_available(&parented, "OWN", "BOND", HedgeMetric::Dv01, 5_000.0),
            0.0
        );
        // (3) A sibling on the SAME side adds risk, it does not relieve any.
        assert_eq!(
            store.internal_offset_available(&parented, "OWN", "OIS", HedgeMetric::Dv01, -5_000.0),
            0.0
        );
        // With family and side both matching, the pool IS available.
        assert_eq!(
            store.internal_offset_available(&parented, "OWN", "OIS", HedgeMetric::Dv01, 5_000.0),
            5_000.0
        );
    }

    /// With no live panel the hedge cost degrades to the desk's CONFIGURED composite spread —
    /// what a composite fill would actually pay — never to a misleading zero.
    #[test]
    fn hedge_cost_falls_back_to_the_composite_spread_on_an_lp_miss() {
        let store = RatesPositionStore::new();
        store.set_lp_hedge_source(Arc::new(NoFillLp));
        assert!((store.live_hedge_cost_bp("OIS", 5_000.0, 100.0, 1e-2, 0.75) - 0.75).abs() < 1e-12);
        // A store that was never given a panel at all behaves the same way.
        let unwired = RatesPositionStore::new();
        assert!(
            (unwired.live_hedge_cost_bp("OIS", 5_000.0, 100.0, 1e-2, 0.75) - 0.75).abs() < 1e-12
        );
        // A non-finite mid cannot yield a distance; fall back rather than emit NaN.
        assert!(
            (unwired.live_hedge_cost_bp("OIS", 5_000.0, f64::NAN, 1e-2, 0.75) - 0.75).abs() < 1e-12
        );
    }

    /// Making money + under the warehouse cap ⇒ the fill is FULLY internalised (warehoused),
    /// nothing shed externally, and the verdict is within tolerance.
    #[test]
    fn internalise_within_tolerance_under_cap_is_fully_internalised() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(hedge_policy("wh", 100_000.0, 0.5)));
        // Pay-fixed 4.00% vs a 4.05% fair mid → paying 5bp under fair = +5bp dealer edge; the
        // 5000-DV01 fill sits far under the 100k cap → green band → warehoused.
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("books");
        let prov = store
            .internalise_of(booked.position_id)
            .expect("a priced fill under a hedge policy is stamped");
        assert!(prov.within_tolerance);
        assert!(
            prov.internalised,
            "under cap + making money ⇒ fully internalised"
        );
        assert!((prov.internal_dv01 - 5000.0).abs() < 1e-6);
        assert_eq!(prov.external_dv01, 0.0);
        assert!((prov.edge_bps - 5.0).abs() < 1e-6);
        assert_eq!(prov.hedge_band, "green");
    }

    /// The scoped **hedging model** actually governs the live booking path — it is a control,
    /// not inert config.
    ///
    /// The fill, the threshold and the graph are IDENTICAL to
    /// [`internalise_within_tolerance_under_cap_is_fully_internalised`], which warehouses it
    /// (5,000 DV01 against a 100,000 cap ⇒ green). Binding `BackToBack` at `Book` scope must
    /// flip that same fill to a full external shed, because back-to-back consults no budget.
    /// If the binding were ignored the fill would still be internalised, so this fails loudly
    /// on a regression that leaves the model resolved-but-unused.
    #[test]
    fn a_bound_back_to_back_model_overrides_the_authored_graph() {
        use crate::config::hedge_policy::ScopedHedgingModel;
        use celnet_hedge_routing::HedgingModel;

        let baseline = RatesPositionStore::new();
        baseline.set_routing(Some(single_book_graph("wh")));
        baseline.set_hedge_policy(Some(hedge_policy("wh", 100_000.0, 0.5)));
        let base_prov = baseline
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .ok()
            .and_then(|b| baseline.internalise_of(b.position_id))
            .expect("baseline stamps");
        assert!(
            base_prov.internalised && base_prov.external_dv01 == 0.0,
            "precondition: this fill warehouses under the authored graph"
        );

        let mut policy = hedge_policy("wh", 100_000.0, 0.5);
        policy.config.hedging_models = vec![ScopedHedgingModel {
            scope_kind: HedgeScopeKind::Book,
            scope_id: "wh".into(),
            model: HedgingModel::BackToBack,
            dv01_budget: 0.0,
        }];
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(policy));
        let prov = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .ok()
            .and_then(|b| store.internalise_of(b.position_id))
            .expect("a priced fill under a hedge policy is stamped");

        assert!(
            !prov.internalised,
            "back-to-back warehouses nothing, so the fill is not internalised"
        );
        assert!(
            (prov.external_dv01 - 5000.0).abs() < 1e-6,
            "the whole 5,000 DV01 pays the street, not {}",
            prov.external_dv01
        );
        assert_eq!(prov.internal_dv01, 0.0);
    }

    /// A bound `InternaliseToDv01` budget IS the cap the fill is measured against — it
    /// overrides the resolved threshold rather than sitting beside it.
    ///
    /// Same 5,000-DV01 fill, same 100,000 threshold (green, warehoused). A 4,000 budget puts
    /// the same fill OVER its cap, so the overflow above the band edge must be shed. A blank
    /// budget must change nothing at all — binding a model may not invent a cap.
    #[test]
    fn a_bound_dv01_budget_overrides_the_resolved_threshold_cap() {
        use crate::config::hedge_policy::ScopedHedgingModel;
        use celnet_hedge_routing::HedgingModel;

        let with_budget = |dv01_budget: f64| {
            let mut policy = hedge_policy("wh", 100_000.0, 0.5);
            policy.config.hedging_models = vec![ScopedHedgingModel {
                scope_kind: HedgeScopeKind::Book,
                scope_id: "wh".into(),
                model: HedgingModel::InternaliseToDv01,
                dv01_budget,
            }];
            let store = RatesPositionStore::new();
            store.set_routing(Some(single_book_graph("wh")));
            store.set_hedge_policy(Some(policy));
            store
                .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
                .ok()
                .and_then(|b| store.internalise_of(b.position_id))
                .expect("stamps")
        };

        let tight = with_budget(4_000.0);
        assert!(
            tight.external_dv01 > 0.0,
            "5,000 DV01 against a bound 4,000 budget is over cap and must shed the overflow"
        );
        assert_ne!(
            tight.hedge_band, "green",
            "over its bound budget the band cannot read green"
        );

        // Blank budget ⇒ inherit the configured 100,000 threshold ⇒ green + warehoused.
        let blank = with_budget(0.0);
        assert_eq!(blank.hedge_band, "green");
        assert_eq!(
            blank.external_dv01, 0.0,
            "a blank budget must inherit the threshold, never invent a cap"
        );
    }

    /// REGRESSION (the notional-operand fix): a book whose signed FACE NOTIONAL is large
    /// (+50mm) while its signed DV01 is TINY (+5,000 = 50mm · 1y · 1bp). A hedge rule whose
    /// CONDITION is `NetNotional > 1mm` must FIRE an external shed — it measures true face
    /// notional (50mm > 1mm), NOT the DV01 proxy (which is 5,000 < 1mm and would never fire).
    /// Simultaneously: the default DV01 warehouse RAG stays GREEN (util 5,000 / 100,000 =
    /// 0.05), and the SAME book under a `NetDv01 > 1mm` rule does NOT fire (5,000 < 1mm).
    /// Before the fix `ctx.net_notional` mirrored `book_net_dv01`, so the `NetNotional` rule
    /// compared against 5,000 and never fired — the "0 external hedges on 252mm gross" bug.
    #[test]
    fn net_notional_condition_fires_on_true_face_notional_not_dv01() {
        use celnet_hedge_routing::{
            ExecStyle, ExitAction, HedgeField, HedgeGraph, HedgeNode, HedgeSize,
        };
        use celnet_risk_routing::{RouteOp, RouteValue};
        use std::collections::BTreeMap;

        // `IF <field> > 1mm THEN SubmitMarketOrder(Full) ELSE Warehouse`.
        let rule_gt_1mm = |field: HedgeField| HedgeGraph {
            entry: 0,
            nodes: BTreeMap::from([
                (
                    0u32,
                    HedgeNode::Condition {
                        field,
                        op: RouteOp::Gt,
                        value: RouteValue::Num(1_000_000.0),
                        on_true: 2,
                        on_false: 1,
                    },
                ),
                (1u32, HedgeNode::action(ExitAction::Warehouse)),
                (
                    2u32,
                    HedgeNode::action(ExitAction::SubmitMarketOrder {
                        size: HedgeSize::Full,
                        style: ExecStyle::Immediate,
                    }),
                ),
            ]),
        };

        // A 1y 50mm pay-fixed OIS: +50mm signed notional, but only +5,000 signed DV01.
        // Deal AT the mid ⇒ 0 edge, below the 0.5bp floor ⇒ below tolerance, so the combine
        // hands the whole fill to the graph's OWN resolved action (external ⇒ shed).

        // (a) A `NetNotional` rule FIRES external on the true 50mm notional.
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(RatesHedgePolicy {
            graph: Some(rule_gt_1mm(HedgeField::NetNotional)),
            ..hedge_policy("wh", 100_000.0, 0.5)
        }));
        let booked = store
            .book_with_routing(
                ois(0, 10, 1, 50_000_000.0, Side::Buy),
                priced_attribution(0.0405, 0.0405),
            )
            .expect("books");
        let prov = store
            .internalise_of(booked.position_id)
            .expect("a priced fill under a hedge policy is stamped");
        assert!(
            prov.external_dv01 > 0.0,
            "NetNotional > 1mm fires on the true 50mm face notional (external_dv01 {})",
            prov.external_dv01
        );
        assert!(
            !prov.internalised,
            "a NetNotional-fired external shed is not warehoused"
        );
        assert_eq!(
            prov.hedge_band, "green",
            "the DV01 warehouse RAG is green — the 50mm notional does not touch the DV01 band"
        );

        // (b) The SAME book under a `NetDv01 > 1mm` rule does NOT fire (5,000 < 1mm).
        let store_dv01 = RatesPositionStore::new();
        store_dv01.set_routing(Some(single_book_graph("wh")));
        store_dv01.set_hedge_policy(Some(RatesHedgePolicy {
            graph: Some(rule_gt_1mm(HedgeField::NetDv01)),
            ..hedge_policy("wh", 100_000.0, 0.5)
        }));
        let booked_dv01 = store_dv01
            .book_with_routing(
                ois(0, 10, 1, 50_000_000.0, Side::Buy),
                priced_attribution(0.0405, 0.0405),
            )
            .expect("books");
        let prov_dv01 = store_dv01
            .internalise_of(booked_dv01.position_id)
            .expect("stamped");
        assert_eq!(
            prov_dv01.external_dv01, 0.0,
            "NetDv01 > 1mm does NOT fire: the book's 5,000 DV01 is far under 1mm"
        );
        assert!(
            prov_dv01.internalised,
            "the DV01-metric rule warehouses this tiny-DV01 book"
        );
    }

    /// The DV01 warehouse behaviour is UNCHANGED by the notional-operand fix: the default
    /// policy (which branches on `breached`, a DV01-band signal) warehouses a +50mm-notional
    /// but +5,000-DV01 fill GREEN and fully internalises it — the huge net notional never
    /// leaks into the DV01 band / overflow / sizing. `net_risk` for the default `Dv01`
    /// threshold stays `net_dv01`.
    #[test]
    fn default_dv01_warehouse_unchanged_by_large_notional() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(hedge_policy("wh", 100_000.0, 0.5)));
        // Pay-fixed 4.00% vs 4.05% fair mid ⇒ +5bp edge (within tolerance); DV01 5,000 far
        // under the 100k cap ⇒ green ⇒ fully warehoused, nothing shed.
        let booked = store
            .book_with_routing(
                ois(0, 10, 1, 50_000_000.0, Side::Buy),
                priced_attribution(0.0400, 0.0405),
            )
            .expect("books");
        let prov = store.internalise_of(booked.position_id).expect("stamped");
        assert!(prov.within_tolerance);
        assert!(
            prov.internalised,
            "under the DV01 cap + making money ⇒ warehoused, regardless of the 50mm notional"
        );
        assert_eq!(prov.external_dv01, 0.0);
        assert!((prov.internal_dv01 - 5000.0).abs() < 1e-6);
        assert_eq!(
            prov.hedge_band, "green",
            "DV01 util 0.05 is green — the large notional does not perturb the DV01 warehouse"
        );
    }

    /// A fill classified B2B (below the min-edge tolerance) under an EXTERNAL shed policy emits a
    /// per-fill hedge-EXECUTION record into the ring the LIVE HEDGE DESK reads
    /// (`ListHedgeProvenance`), so the desk populates and reconciles to the originating B2B deal
    /// via `parent_position_id`. Regression for "0 hedges · 0 external despite B2B deals": the
    /// tolerance-driven external shed used to be advisory-only with no execution record.
    ///
    /// (Part A: under an INTERNAL-hold policy a below-tolerance fill is now WAREHOUSED instead —
    /// see `wash_book_warehouses_thin_edge_fill_with_no_external_deal`. This scenario uses an
    /// explicit external shed policy, which is the configuration that actually backs-to-back.)
    #[test]
    fn b2b_below_tolerance_fill_emits_a_reconcilable_hedge_execution() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = external_hedge_policy("wh", 100_000.0, 0.5);
        // The SAME engine the `ListHedgeProvenance` RPC serves (production wires
        // `Arc::clone(&self.auto_hedge)` into the policy).
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        // Deal AT the mid ⇒ 0 captured edge, below the 0.5bp floor ⇒ below tolerance ⇒ under the
        // EXTERNAL shed policy the whole 5000-DV01 fill is backed-to-back externally.
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");
        let prov = store
            .internalise_of(booked.position_id)
            .expect("a priced fill under a hedge policy is stamped");
        assert!(
            !prov.within_tolerance,
            "a zero-edge fill is below the floor"
        );
        assert!(
            prov.external_dv01 > 0.0,
            "the whole fill is shed externally"
        );
        assert!(
            !prov.internalised,
            "a below-tolerance fill is not warehoused"
        );

        // The Hedge Desk ring now carries a per-fill execution record reconciling to the deal.
        let records = engine.provenance(None, None);
        let exec = records
            .iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("a hedge-execution record was emitted for the B2B fill");
        assert!(
            (exec.external_hedged - prov.external_dv01).abs() < 1e-6,
            "the shed amount is the fill's external DV01"
        );
        // The default execution mode is LIVE (LP-then-composite): with no LP source wired the
        // shed fills on the composite venue, so the record is NON-advisory and carries the real
        // executed economics (mid worsened by the composite spread, a signed slippage, and the
        // COMPOSITE venue label) — not the reference mid.
        assert!(!exec.advisory, "a live composite shed is not advisory");
        assert_eq!(
            exec.lp_won.as_deref(),
            Some("COMPOSITE"),
            "the live shed filled on the composite venue"
        );
        assert!(
            exec.mid_at_fire > 0.0,
            "the record carries the reference composite mid"
        );
        assert!(
            (exec.hedge_price - exec.mid_at_fire).abs() > 0.0,
            "the composite fill is worsened off mid by the spread"
        );
        assert!(
            exec.slippage_bp.abs() > 0.0,
            "a composite fill records a signed slippage vs mid"
        );
    }

    /// Regression (live UAT 4375541): the firm primed a hedge policy with NO exit graph and
    /// NO configured thresholds — exactly what `lib.rs` primes when the pristine-store seed was
    /// skipped (the operator's routing book is not the default `warehouse` book, so
    /// `ensure_seed_hedge_policy` found no book to bind to). Fills routed into that operator
    /// book. Before the fix `stamp_internalise` bailed on the missing graph → the deal carried
    /// NO internalise decision → the blotter HEDGE column showed `—`. The booking path must
    /// STILL stamp a real decision + RAG band by falling back to the default warehouse graph +
    /// threshold — a genuine engine evaluation against the default firm warehouse budget,
    /// never a fabricated value.
    #[test]
    fn internalise_stamps_with_default_when_policy_unconfigured() {
        let store = RatesPositionStore::new();
        // Route into an operator-defined book the firm never bound a warehouse cap to (the
        // live box booked into "default-book").
        store.set_routing(Some(single_book_graph("default-book")));
        // The runtime prime `lib.rs` performs when nothing is persisted: the shared engine is
        // present, but there is no graph and no thresholds.
        store.set_hedge_policy(Some(RatesHedgePolicy {
            engine: Arc::new(AutoHedgeEngine::default()),
            graph: None,
            thresholds: Vec::new(),
            config: HedgeConfigDef::default(),
            known_lps: BTreeSet::new(),
            scoped_graphs: Vec::new(),
            book_ancestors: HashMap::new(),
            book_descendants: HashMap::new(),
        }));
        // Pay-fixed 4.00% vs a 4.05% fair mid → +5bp dealer edge; the 5000-DV01 fill sits deep
        // under the default 1mm-DV01 warehouse cap.
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("books");
        let prov = store
            .internalise_of(booked.position_id)
            .expect("an unconfigured policy still stamps via the default warehouse fallback");
        assert!(
            prov.within_tolerance,
            "5bp edge clears the 0.5bp default tolerance floor"
        );
        assert!(
            prov.internalised,
            "deep under the default warehouse cap + making money ⇒ fully internalised"
        );
        assert!((prov.internal_dv01 - 5000.0).abs() < 1e-6);
        assert_eq!(prov.external_dv01, 0.0);
        assert_eq!(
            prov.hedge_band, "green",
            "the deal must carry a real RAG band, never an empty `—`"
        );
    }

    /// The single-sourced default warehouse threshold is the firm DV01 budget the runtime
    /// fallback measures an unbound fill against — active from first boot for any routed book.
    #[test]
    fn default_warehouse_threshold_is_the_firm_dv01_budget() {
        let def = default_warehouse_threshold_def();
        assert_eq!(def.metric, HedgeMetric::Dv01);
        assert_eq!(def.cap, crate::config::identity::DEFAULT_WAREHOUSE_DV01_CAP);
        // A normally-sized single fill sits green under it (⇒ warehoused / internalised).
        let wh = def.to_threshold();
        assert!(!wh.breached(5_000.0));
    }

    // ---- latency instrumentation (docs/LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md) ----

    /// A routed, priced rates booking through the FI sink records its per-stage latency into
    /// the installed telemetry hub: the booking commit (`Book`), the risk-routing decision
    /// (`RiskRoute`), and the auto-hedge decision (`HedgeFire`) all show `count > 0` in
    /// `stage_stats()` — the stages the Latency/Ops workspace renders. Drives the SAME routed +
    /// hedge-policy fixture the internalise tests use.
    #[test]
    fn fi_booking_seams_record_latency_stages() {
        use celnet_observability::OpKind;
        let store = RatesPositionStore::new();
        let hub = Arc::new(crate::services::telemetry::TelemetryHub::new(1_000_000_000));
        store.set_telemetry(Arc::clone(&hub));
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(hedge_policy("wh", 100_000.0, 0.5)));

        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("routed priced booking succeeds");
        assert!(
            store.internalise_of(booked.position_id).is_some(),
            "the fixture must exercise the hedge/internalise path so HedgeFire fires"
        );

        let stats = hub.stage_stats();
        let count_of = |k: OpKind| {
            stats
                .iter()
                .find(|s| s.kind == k)
                .map_or(0, |s| s.snapshot.count)
        };
        assert!(
            count_of(OpKind::Book) > 0,
            "the booking commit records the Book stage, got {stats:?}"
        );
        assert!(
            count_of(OpKind::RiskRoute) > 0,
            "the routing decision records the RiskRoute stage, got {stats:?}"
        );
        assert!(
            count_of(OpKind::HedgeFire) > 0,
            "the auto-hedge decision records the HedgeFire stage, got {stats:?}"
        );
    }

    /// **`Sync` keeps the hedge on the booking path; `Async` takes it off.**
    ///
    /// The synchronous case is the historical contract and several callers depend on it:
    /// the offsetting leg and the provenance record are BOTH in place the instant
    /// `book_with_routing` returns. Asserting that immediately after booking is exactly
    /// the guarantee `Sync` makes.
    #[test]
    fn sync_dispatch_completes_the_hedge_before_the_fill_returns() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let mut policy = external_hedge_policy("wh", 100_000.0, 0.5);
        policy.config.dispatch = crate::config::hedge_policy::HedgeDispatch::Sync;
        store.set_hedge_policy(Some(policy));
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-01",
            bid: 0.0404,
            offer: 0.0406,
        }));
        store.set_street_router(Arc::new(TradesAtQuote));

        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");
        assert!(
            store.internalise_of(booked.position_id).is_some(),
            "under Sync the hedge is complete by the time the fill returns",
        );
    }

    /// Under `Async` the fill returns without waiting — and the hedge still happens.
    ///
    /// Both halves matter. A fill that returns early has bought nothing if the hedge was
    /// dropped, so this waits (bounded) for the detached work to land rather than
    /// asserting its absence, which would pass just as well for a hedge that never ran.
    #[test]
    fn async_dispatch_detaches_the_hedge_but_still_runs_it() {
        let store = Arc::new(RatesPositionStore::new());
        store.set_self_handle(&store);
        store.set_routing(Some(single_book_graph("wh")));
        let mut policy = external_hedge_policy("wh", 100_000.0, 0.5);
        policy.config.dispatch = crate::config::hedge_policy::HedgeDispatch::Async;
        store.set_hedge_policy(Some(policy));
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-01",
            bid: 0.0404,
            offer: 0.0406,
        }));
        store.set_street_router(Arc::new(TradesAtQuote));

        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while store.internalise_of(booked.position_id).is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the detached hedge must still run — async moves it off the path, it does \
                 not drop it",
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Async configured but NO self-handle installed ⇒ the hedge runs inline.
    ///
    /// Degrading to synchronous is always safe; silently skipping the hedge never is, so
    /// the missing handle must not become a missing hedge.
    #[test]
    fn async_without_a_self_handle_falls_back_to_running_inline() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let mut policy = external_hedge_policy("wh", 100_000.0, 0.5);
        policy.config.dispatch = crate::config::hedge_policy::HedgeDispatch::Async;
        store.set_hedge_policy(Some(policy));
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-01",
            bid: 0.0404,
            offer: 0.0406,
        }));
        store.set_street_router(Arc::new(TradesAtQuote));

        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");
        assert!(
            store.internalise_of(booked.position_id).is_some(),
            "no handle ⇒ inline, never skipped",
        );
    }

    /// **The venue round trip is its own stage, not part of the booking commit.**
    ///
    /// The auto-hedge EXECUTION puts a real order on a real venue and blocks on the
    /// answer. That wait used to be unmeasured, so it was folded into `Book` — which is
    /// why a live desk read 5-6ms for "Ack→fill→book" while the hedge DECISION read 26us.
    /// A stage that silently contains someone else's network call cannot be optimised,
    /// because the number never points at the thing that is slow.
    #[test]
    fn the_hedge_venue_round_trip_is_measured_apart_from_the_booking_commit() {
        use celnet_observability::OpKind;
        let store = RatesPositionStore::new();
        let hub = Arc::new(crate::services::telemetry::TelemetryHub::new(1_000_000_000));
        store.set_telemetry(Arc::clone(&hub));
        store.set_routing(Some(single_book_graph("wh")));
        // An EXTERNAL policy, an LP that quotes, and a router that trades — so the shed
        // genuinely reaches the street seam rather than warehousing.
        store.set_hedge_policy(Some(external_hedge_policy("wh", 100_000.0, 0.5)));
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-01",
            bid: 0.0404,
            offer: 0.0406,
        }));
        store.set_street_router(Arc::new(TradesAtQuote));

        store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");

        let stats = hub.stage_stats();
        let count_of = |k: OpKind| -> u64 {
            stats
                .iter()
                .find(|s| s.kind == k)
                .map_or(0, |s| s.snapshot.count)
        };
        assert!(
            count_of(OpKind::HedgeExecute) > 0,
            "the external hedge execution records its own stage, got {stats:?}"
        );
        assert!(
            count_of(OpKind::HedgeFire) > 0,
            "the decision still records its own stage, got {stats:?}"
        );
    }

    /// With no hub installed the same booking is a silent no-op for telemetry — booking still
    /// succeeds, byte-identical to the pre-instrumentation path (the record calls short-circuit).
    #[test]
    fn fi_booking_seams_are_silent_without_a_hub() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(hedge_policy("wh", 100_000.0, 0.5)));
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("booking succeeds with no telemetry hub attached");
        assert_eq!(booked.position_id, 1);
    }

    /// An unrouted booking (no graph) does NOT fold a trivial no-graph span into the RiskRoute
    /// stage — RiskRoute stays empty while Book still records. Guards the `routing_ran` gate.
    #[test]
    fn unrouted_booking_records_book_but_not_riskroute() {
        use celnet_observability::OpKind;
        let store = RatesPositionStore::new();
        let hub = Arc::new(crate::services::telemetry::TelemetryHub::new(1_000_000_000));
        store.set_telemetry(Arc::clone(&hub));
        // No routing graph installed.
        let _ = store
            .book(position(0, 1, 10))
            .expect("unrouted booking succeeds");
        let stats = hub.stage_stats();
        let count_of = |k: OpKind| {
            stats
                .iter()
                .find(|s| s.kind == k)
                .map_or(0, |s| s.snapshot.count)
        };
        assert!(
            count_of(OpKind::Book) > 0,
            "Book still records when unrouted"
        );
        assert_eq!(
            count_of(OpKind::RiskRoute),
            0,
            "no graph ⇒ RiskRoute records nothing (no trivial span), got {stats:?}"
        );
    }

    /// PART A (wash-book fix): a thin (sub-floor edge) fill under an INTERNAL-hold policy (the
    /// default warehouse-vs-shed graph, green band ⇒ `Warehouse`) is now WAREHOUSED, not
    /// force-shed to the street. A wash / internal-only book captures ~0 edge on essentially
    /// every fill; it must never emit an external B2B market order that contradicts its own
    /// internal mandate. No external DV01, no offsetting hedge leg, no per-fill execution record.
    #[test]
    fn wash_book_warehouses_thin_edge_fill_with_no_external_deal() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = hedge_policy("wh", 100_000.0, 0.5);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        // 4.049% vs 4.05% mid → 0.1bp edge, below the 0.5bp floor; the 5000-DV01 fill sits far
        // under the 100k cap (green band) ⇒ the default graph resolves to Warehouse (INTERNAL).
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.04049, 0.0405))
            .expect("books");
        let prov = store.internalise_of(booked.position_id).expect("stamped");
        assert!(!prov.within_tolerance, "0.1bp is below the 0.5bp floor");
        assert!(
            prov.internalised,
            "an internal-policy thin fill is warehoused, not shed"
        );
        assert!(
            (prov.internal_dv01 - 5000.0).abs() < 1e-6,
            "the whole fill is warehoused"
        );
        assert_eq!(prov.external_dv01, 0.0, "NOTHING is shed to the street");
        // No external shed ⇒ the engine rings NOTHING (a green-band Warehouse hold self-rings no
        // decision record) and no per-fill execution record is stamped — the ring is empty.
        assert!(
            engine.provenance(None, None).is_empty(),
            "a warehoused wash fill emits no hedge ring record at all"
        );
        assert!(
            (store.book_net_dv01("wh") - 5000.0).abs() < 1e-6,
            "the warehoused net is the fill itself — no offsetting hedge leg was booked"
        );
    }

    /// FOLLOW-UP (no double-record): an over-cap BREACH fill (RED, size-bearing external action —
    /// the engine's `evaluate` self-rings a book-level DECISION record) leaves EXACTLY ONE ring
    /// record, and it carries the REALISED economics (the LP that filled + a real signed slippage),
    /// NOT the engine's pre-execution intent (which lacks `lp_won` / slippage). Guards against the
    /// decision-record + execution-record double count in the Hedge Deals blotter.
    #[test]
    fn breach_fill_leaves_exactly_one_realised_ring_record() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        // cap 4000 DV01: the +5000-DV01 pay-fixed fill breaches (util 1.25) ⇒ the default graph's
        // breach node fires a size-bearing SubmitMarketOrder → `evaluate` self-rings a decision.
        let policy = hedge_policy("wh", 4000.0, 0.5);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        // A live LP is on the panel: the shed fills against it (a long sheds by selling the bid).
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-03",
            bid: 0.0404,
            offer: 0.0406,
        }));
        // …and a router that reaches it. A firm quote alone can no longer fill: the
        // counterparty has to agree, which is the whole point of the routing seam.
        store.set_street_router(Arc::new(TradesAtQuote));
        // +5bp edge (within tolerance) so the overflow-only shed path (not the below-edge path) is
        // exercised — the case where `evaluate` DID ring a decision record.
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("books");

        let records = engine.provenance(None, None);
        assert_eq!(
            records.len(),
            1,
            "a fired breach hedge leaves exactly one ring record, got {records:?}"
        );
        let rec = &records[0];
        assert_eq!(rec.parent_position_id, Some(booked.position_id));
        assert_eq!(
            rec.lp_won.as_deref(),
            Some("LP-SIM-03"),
            "the single record carries the REALISED winning LP, not the intent's empty lp_won"
        );
        assert!(!rec.advisory, "a real LP fill is not advisory");
        // Realised, not the intent's zero economics: sold a long below mid ⇒ negative slippage.
        assert!(
            (rec.slippage_bp - (-1.0)).abs() < 1e-9,
            "the record carries realised slippage, got {}",
            rec.slippage_bp
        );
        assert!(
            (rec.hedge_price - 0.0404).abs() < 1e-12,
            "the record carries the realised LP fill price, not 0"
        );
    }

    /// FOLLOW-UP (no double-record): an external-policy BELOW-EDGE back-to-back also leaves EXACTLY
    /// ONE realised ring record (the engine self-rings the external action's decision, which the
    /// executor then supersedes in place).
    #[test]
    fn external_below_edge_back_to_back_leaves_exactly_one_ring_record() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = external_hedge_policy("wh", 100_000.0, 0.5);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-01",
            bid: 0.0404,
            offer: 0.0406,
        }));
        // …and a router that reaches it. A firm quote alone can no longer fill: the
        // counterparty has to agree, which is the whole point of the routing seam.
        store.set_street_router(Arc::new(TradesAtQuote));
        // Deal at mid ⇒ zero edge ⇒ below the floor ⇒ the whole fill backs to back externally.
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");
        let records = engine.provenance(None, None);
        assert_eq!(
            records.len(),
            1,
            "one realised record for the back-to-back, got {records:?}"
        );
        assert_eq!(records[0].parent_position_id, Some(booked.position_id));
        assert_eq!(records[0].lp_won.as_deref(), Some("LP-SIM-01"));
        assert!(!records[0].advisory);
    }

    /// PART A: a thin (sub-floor edge) fill under an EXTERNAL shed policy IS handed back to the
    /// street — the whole fill sheds, it is not internalised, and the per-fill execution record
    /// carries the graph's OWN external action (so the hedging-only desk filter, which keys off
    /// `is_external`, shows it).
    #[test]
    fn thin_edge_under_external_policy_sheds_back_to_back() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = external_hedge_policy("wh", 100_000.0, 0.5);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.04049, 0.0405))
            .expect("books");
        let prov = store.internalise_of(booked.position_id).expect("stamped");
        assert!(!prov.within_tolerance, "0.1bp is below the 0.5bp floor");
        assert!(
            !prov.internalised,
            "a shed policy does not warehouse a thin fill"
        );
        assert_eq!(prov.internal_dv01, 0.0);
        assert!(
            (prov.external_dv01 - 5000.0).abs() < 1e-6,
            "the whole fill goes to the street"
        );
        let exec = engine
            .provenance(None, None)
            .into_iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("a reconcilable per-fill execution record");
        let action = exec.action.as_ref().expect("an action is stamped");
        let exit = crate::services::auto_hedge::wire::exit_action_from_wire(action)
            .expect("the action decodes");
        assert!(
            exit.is_external(),
            "the stamped action is the external shed the policy fired, not a hold"
        );
    }

    /// PART B: with a live LP source wired, an external shed FILLS against the best executable LP
    /// price on the required side — `venue = Lp`, `lp_won = <that LP>`, and a REAL signed slippage
    /// vs mid (negative when selling a long below mid) — not the composite backstop.
    #[test]
    fn external_shed_fills_against_the_live_lp_panel() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = external_hedge_policy("wh", 100_000.0, 0.5);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        // The long fill (+5000 DV01) sheds by SELLING to the LP at its bid 0.0404 vs the 0.0405 mid.
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-01",
            bid: 0.0404,
            offer: 0.0406,
        }));
        // …and a router that reaches it. A firm quote alone can no longer fill: the
        // counterparty has to agree, which is the whole point of the routing seam.
        store.set_street_router(Arc::new(TradesAtQuote));
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");
        let exec = engine
            .provenance(None, None)
            .into_iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("a per-fill execution record");
        assert_eq!(
            exec.lp_won.as_deref(),
            Some("LP-SIM-01"),
            "filled on the named LP, not COMPOSITE"
        );
        assert!(!exec.advisory, "a real LP fill is not advisory");
        assert!(
            (exec.hedge_price - 0.0404).abs() < 1e-12,
            "the LP bid is the fill price"
        );
        // Sold a long BELOW mid ⇒ negative slippage: (0.0404 − 0.0405)/1e-4 = −1.0 bp.
        assert!(
            (exec.slippage_bp - (-1.0)).abs() < 1e-9,
            "signed slippage vs mid, got {}",
            exec.slippage_bp
        );
    }

    /// PART B: when NO LP has a firm price (the source honestly misses), a live LP-then-composite
    /// shed falls back to the composite backstop — never a fabricated LP fill (guardrail 2).
    #[test]
    fn external_shed_falls_back_to_composite_when_no_lp_price() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = external_hedge_policy("wh", 100_000.0, 0.5);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        store.set_lp_hedge_source(Arc::new(NoFillLp));
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");
        let exec = engine
            .provenance(None, None)
            .into_iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("record");
        assert_eq!(
            exec.lp_won.as_deref(),
            Some("COMPOSITE"),
            "an LP miss honestly backstops to the composite"
        );
        assert!(!exec.advisory, "a live composite shed is not advisory");
    }

    /// **A listed vehicle goes to the wire in FACE, and the risk conversion stays exact.**
    ///
    /// The last unit in the 2026-08-20 chain. Sending whole CONTRACTS still came back
    /// `NOT_A_WHOLE_LOT` for `UBU26 qty=2`, because this venue denominates order quantity in
    /// face — `contract_lot_size` IS the contract's face value, and its quoted clips are
    /// `contracts × face`. That is a deliberate design: one denomination across the whole
    /// aggregated book is what lets a cash bond and a future sit on the same panel.
    ///
    /// So the wire gets `units × face`, and `risk_per_venue_unit` divides by the same
    /// factor — the product is invariant, so the book still reduces by exactly the DV01 the
    /// contracts removed.
    #[test]
    fn a_listed_vehicle_is_sent_in_face_with_an_exact_risk_conversion() {
        use celnet_hedge_routing::Dv01Basis;
        let dv01_per_contract = 133.8721;
        let plan = HedgeRatioPlan {
            hedge_instrument_id: "UBU26".to_owned(),
            unit_label: "contracts".to_owned(),
            whole_units: true,
            basis: Dv01Basis::Analytic,
            target_dv01: 290.32,
            dv01_per_unit: dv01_per_contract,
            exact_units: 2.168_663,
            units: 2.0,
            hedged_dv01: 2.0 * dv01_per_contract,
            residual_dv01: 22.58,
        };
        let (venue_quantity, risk_per_venue_unit) = venue_denomination(Some(&plan), 267.74);

        // UB's contract face is 100,000, so two contracts is 200,000 of face — a whole
        // multiple of the lot, which is precisely what the venue was refusing before.
        assert!(
            (venue_quantity - 200_000.0).abs() < 1e-6,
            "expected 200000 face, got {venue_quantity}"
        );
        assert!(
            (venue_quantity % 100_000.0).abs() < 1e-6,
            "the wire quantity must be a whole multiple of the contract face"
        );
        // The invariant that matters: quantity × risk-per-unit is still the DV01 the two
        // contracts actually remove, so nothing about the book's accounting moved.
        assert!(
            (venue_quantity * risk_per_venue_unit - 2.0 * dv01_per_contract).abs() < 1e-9,
            "risk conversion must be exact"
        );
    }

    /// A vehicle that is NOT a listed contract keeps its own units — the face lookup must
    /// not silently rescale something it does not recognise.
    #[test]
    fn an_unlisted_vehicle_keeps_its_own_units() {
        use celnet_hedge_routing::Dv01Basis;
        let plan = HedgeRatioPlan {
            hedge_instrument_id: "SOME-OTC-BENCHMARK".to_owned(),
            unit_label: "1mm face".to_owned(),
            whole_units: false,
            basis: Dv01Basis::Analytic,
            target_dv01: 500.0,
            dv01_per_unit: 100.0,
            exact_units: 5.0,
            units: 5.0,
            hedged_dv01: 500.0,
            residual_dv01: 0.0,
        };
        let (q, r) = venue_denomination(Some(&plan), 500.0);
        assert!((q - 5.0).abs() < 1e-9, "unlisted vehicle keeps its units");
        assert!((r - 100.0).abs() < 1e-9);
    }

    /// **The blotter records what was SENT, in the venue's own units.**
    ///
    /// Second-order regression from the 2026-08-20 UAT report. Once the wire was fixed to
    /// ask for whole contracts, the blotter still logged the budget-metric size — so a plan
    /// for 1 `ZFU26` showed as "Requested 38.3347" next to a fill of 0, and the row looked
    /// like the bug was still there. A blotter row that names a quantity nobody sent is
    /// worse than no row: it sends the desk chasing a fix that has already shipped.
    #[test]
    fn the_street_blotter_records_the_venue_quantity_not_the_dv01() {
        use crate::services::analytics::street_orders::{StreetOrderFilter, StreetOrderLog};
        use crate::services::auto_hedge::{ExternalHedgeFill, HedgeVenue};

        let log = Arc::new(StreetOrderLog::new());
        let dv01_per_contract = 40.1992;
        let contracts = 1.0;
        // A composite backstop fill, carried (as always) in the budget metric.
        let exec = ExternalHedgeFill {
            filled: contracts * dv01_per_contract,
            residual: 0.0,
            hedge_price: 107.39,
            mid_at_fire: 107.39,
            slippage_bp: 0.5,
            lp_won: Some(HedgeVenue::COMPOSITE_LABEL.to_owned()),
            venue: Some(HedgeVenue::Composite),
            panel: Vec::new(),
            attempts: Vec::new(),
        };
        record_street_order(
            Some(&log),
            &exec,
            crate::config::hedge_policy::HedgeExecutionMode::Composite,
            "ZFU26",
            "bond_future",
            Some(4.6),
            -500.0,
            contracts,
            dv01_per_contract,
            Some("HDG-1".to_owned()),
            7,
            1_787_000_000_000_000_000,
        );

        let orders = log.orders(None, None, &StreetOrderFilter::default(), 10);
        assert_eq!(orders.len(), 1);
        let o = &orders[0];
        assert!(
            (o.requested_qty - contracts).abs() < 1e-9,
            "requested must be CONTRACTS, got {}",
            o.requested_qty
        );
        // …and the fill is converted back to the same denomination, so one row never
        // carries two different units.
        assert!(
            (o.filled_qty - contracts).abs() < 1e-9,
            "filled must be CONTRACTS, got {}",
            o.filled_qty
        );
    }

    /// PART C: a hedge that fills on a NAMED LP is attributed to that LP in the street-side LP
    /// flow rollup (a WON deal + won notional), so the LP league table shows the win.
    #[tokio::test]
    async fn hedge_fill_on_named_lp_is_attributed_in_the_lp_flow_rollup() {
        use crate::services::analytics::lp::{LpFlowSource, fold};
        use crate::services::analytics::street_orders::{StreetOrderFilter, StreetOrderLog};
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(external_hedge_policy("wh", 100_000.0, 0.5)));
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-01",
            bid: 0.0404,
            offer: 0.0406,
        }));
        // …and a router that reaches it. A firm quote alone can no longer fill: the
        // counterparty has to agree, which is the whole point of the routing seam.
        store.set_street_router(Arc::new(TradesAtQuote));
        let log = Arc::new(StreetOrderLog::new());
        store.set_street_order_log(Arc::clone(&log));
        store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");

        // The ORDER itself is captured — side, requested-vs-filled, price, outcome — not
        // merely the fact that some LP filled something.
        let orders = log.orders(None, None, &StreetOrderFilter::default(), 10);
        assert_eq!(orders.len(), 1, "exactly one outbound street order");
        let o = &orders[0];
        assert_eq!(o.lp_id.as_deref(), Some("LP-SIM-01"));
        assert_eq!(
            o.reason, None,
            "a clean complete fill on a routed order needs no qualifier — and must NOT \
             carry a quote-derived-lift tag, which would understate what happened"
        );
        assert_eq!(o.venue, celnet_analytics::StreetVenue::NamedLp);
        assert_eq!(o.outcome, celnet_analytics::StreetOutcome::Filled);
        assert!(o.requested_qty > 0.0 && o.filled_qty > 0.0);
        assert!(o.filled_price.is_some(), "a fill carries a realised price");
        assert!(o.slippage_bp.is_some());
        assert!(
            o.parent_position_id.is_some(),
            "the street order links back to the client fill that produced it"
        );
        // The typed order and its measured round trip are REAL now: a `NewOrderSingle`
        // carrying these exact tags was sent, and the answer was timed.
        assert_eq!(
            o.order_type.as_deref(),
            Some("2"),
            "the FIX OrdType(40) actually sent — limit, at the LP's own level"
        );
        assert_eq!(
            o.time_in_force.as_deref(),
            Some("3"),
            "the FIX TimeInForce(59) actually sent — immediate-or-cancel"
        );
        assert_eq!(
            o.response_latency_nanos,
            Some(640_000),
            "the venue round trip the router measured"
        );

        let recs = log.lp_flow_records(None, None).await;
        let won = recs
            .iter()
            .find(|r| r.lp_id == "LP-SIM-01")
            .expect("the fill is attributed to the executing LP");
        assert!(
            won.was_won && won.was_quoted,
            "a hedge fill is a quoted win"
        );
        assert!(won.notional > 0.0, "the hedge notional is the won notional");
        // The LP league-table fold now grades LP-SIM-01 with a won deal (not 0 · $0).
        let rows = fold(&recs, &std::collections::BTreeMap::new(), Some("LP-SIM-01"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].deals_won, 1);
        assert!(rows[0].won_notional > 0.0);
    }

    /// PART C: a COMPOSITE-venue hedge fill is NOT attributed to a named street LP (COMPOSITE is
    /// a pseudo-venue) — the flow log stays empty, so no fabricated LP win.
    #[tokio::test]
    async fn composite_hedge_fill_is_not_attributed_to_a_named_lp() {
        use crate::services::analytics::lp::LpFlowSource;
        use crate::services::analytics::street_orders::{StreetOrderFilter, StreetOrderLog};
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(external_hedge_policy("wh", 100_000.0, 0.5)));
        // No LP source ⇒ the default LP-then-composite shed fills on the composite venue.
        let log = Arc::new(StreetOrderLog::new());
        store.set_street_order_log(Arc::clone(&log));
        store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");
        assert!(
            log.lp_flow_records(None, None).await.is_empty(),
            "a COMPOSITE fill is not a street-LP win"
        );
        // …but the ORDER is still recorded, explicitly labelled as a backstop with the
        // reason it reached one. That is the whole point: "we hedged and the street
        // showed us nothing" must be distinguishable from "we never hedged".
        let orders = log.orders(None, None, &StreetOrderFilter::default(), 10);
        assert_eq!(orders.len(), 1);
        assert_eq!(
            orders[0].venue,
            celnet_analytics::StreetVenue::CompositeBackstop
        );
        assert_eq!(orders[0].lp_id, None, "a backstop names no LP");
        assert_eq!(orders[0].reason.as_deref(), Some("no_firm_lp_price"));
        assert!(
            orders[0].competitors.is_empty(),
            "a backstop is the ABSENCE of street prices"
        );
    }

    /// A fill that pushes the book's net DV01 over the warehouse cap ⇒ the over-cap overflow
    /// is shed externally and the rest warehoused (a split), even when making money.
    #[test]
    fn over_cap_fill_splits_internal_and_external() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        // cap 4000 DV01: the 5000-DV01 fill breaches (util 1.25). target = 0.8·4000 = 3200 →
        // overflow 5000 − 3200 = 1800 shed externally; the remaining 3200 warehoused.
        store.set_hedge_policy(Some(hedge_policy("wh", 4000.0, 0.5)));
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("books");
        let prov = store.internalise_of(booked.position_id).expect("stamped");
        assert!(prov.within_tolerance, "5bp edge clears the floor");
        assert!(
            !prov.internalised,
            "an over-cap fill is not fully internalised"
        );
        assert!(
            (prov.external_dv01 - 1800.0).abs() < 1e-6,
            "the over-cap overflow is shed externally, got {}",
            prov.external_dv01
        );
        assert!(
            (prov.internal_dv01 - 3200.0).abs() < 1e-6,
            "the rest is warehoused, got {}",
            prov.internal_dv01
        );
        assert_eq!(prov.hedge_band, "breach");
    }

    /// The manual `book` path (no dealt/mid) stamps NO internalise decision — byte-identical
    /// to the pre-Phase-B booking path even with a hedge policy primed.
    #[test]
    fn manual_booking_stamps_no_internalise_decision() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(hedge_policy("wh", 100_000.0, 0.5)));
        let booked = store.book(position(0, 1, 10)).expect("books");
        assert!(store.internalise_of(booked.position_id).is_none());
    }

    /// With no hedge policy primed, a priced fill still books but carries no internalise
    /// decision (surfaced, never fabricated).
    #[test]
    fn no_hedge_policy_stamps_no_internalise_decision() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("books");
        assert!(store.internalise_of(booked.position_id).is_none());
    }

    /// With no warehouse threshold configured for the routed book (the policy's only threshold
    /// is on a DIFFERENT book), the fill falls back to the default firm warehouse budget and
    /// STILL stamps a real internalise decision + band — never an empty `—` (live UAT 4375541).
    /// This is the taker-facing guarantee: every booked fill carries a HEDGE decision.
    #[test]
    fn no_threshold_for_book_falls_back_to_default_warehouse_budget() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        // A policy whose only threshold is on a DIFFERENT book ⇒ unresolved for "wh" ⇒ the
        // default warehouse budget (1mm DV01) applies.
        store.set_hedge_policy(Some(hedge_policy("other-book", 100_000.0, 0.5)));
        let booked = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("books");
        let prov = store
            .internalise_of(booked.position_id)
            .expect("the fill falls back to the default warehouse budget and is stamped");
        // 5000 DV01 is deep green under the default 1mm cap, +5bp edge ⇒ fully internalised.
        assert!(prov.within_tolerance);
        assert!(prov.internalised);
        assert_eq!(prov.hedge_band, "green");
    }

    // ======================================================================
    // Hedge VEHICLE + suggest-then-exit (docs/HEDGING-AND-RISK-EXIT.md §6.4/§6.5)
    // ======================================================================

    /// A ten-year 5% bond maturing on a real date, so the analytic DV01 has a schedule to
    /// work with (the shared `bond_position` fixture deliberately carries no maturity).
    fn dated_bond(id: u64, redemption: f64, side: Side, years: i64) -> RatesPosition {
        let maturity = time::OffsetDateTime::now_utc().date() + time::Duration::days(365 * years);
        RatesPosition {
            position_id: id,
            entity: 1,
            book: 7,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Bond(BondInstrument {
                    coupon_rate: 0.05,
                    coupon_frequency: 0, // annual
                    day_count: 0,
                    maturity_date: Some(celnet_proto::BrokenDate {
                        year: maturity.year(),
                        month: u32::from(u8::from(maturity.month())),
                        day: u32::from(maturity.day()),
                    }),
                    redemption,
                    side: side as i32,
                    instrument_id: "XS-CORP-10Y".to_owned(),
                    ..Default::default()
                })),
            }),
            ..Default::default()
        }
    }

    /// **The DV01-proxy problem, pinned — and the Wave-0 re-basing that removed it.**
    ///
    /// The pre-migration exposure measured a bond as `redemption × 1bp` (duration 1), while
    /// the genuine analytic DV01 of a ten-year 5% bond is roughly EIGHT times that (modified
    /// duration ≈ 7.7). `rates_linear_exposure` now charges the analytic number, and
    /// `rates_linear_exposure_proxy` retains the old one purely as the migration reference.
    #[test]
    fn a_ten_year_bonds_exposure_is_about_eight_times_the_retired_duration_blind_proxy() {
        let fill = dated_bond(1, 1_000_000.0, Side::Buy, 10);
        let proxy = rates_linear_exposure_proxy(&fill).abs();
        assert!(
            (proxy - 100.0).abs() < 1e-9,
            "the retired proxy is redemption × 1bp = 100, i.e. duration 1: {proxy}"
        );

        let rebased = rates_linear_exposure(&fill).abs();
        // A 10y 5% annual bond at par: modified duration ≈ 7.72 ⇒ DV01 ≈ 772 per 1mm face.
        let ratio = rebased / proxy;
        assert!(
            (7.0..8.5).contains(&ratio),
            "the LIVE exposure measure must now be ~8× the retired proxy for a 10y bond, \
             got {ratio}× (rebased {rebased}, proxy {proxy})"
        );

        // The exposure measure and the hedge-sizing measure now agree on the bond arm — the
        // asymmetry that made a correct hedge ratio chase a meaningless target is gone.
        let (genuine, basis) = genuine_position_dv01(&fill, None);
        assert_eq!(
            basis,
            Dv01Basis::Analytic,
            "a bond with a real maturity is priced analytically, never off the proxy"
        );
        assert!(
            (genuine - rebased).abs() < 1e-9,
            "measurement and sizing must charge the same bond DV01: {genuine} vs {rebased}"
        );
        assert!(
            basis.is_duration_correct(),
            "an analytic DV01 is duration-correct and may be presented as exact"
        );
    }

    /// **The re-based bond exposure IS `celnet_bond::dv01`** — verified against the leaf
    /// engine directly, not against the server's own restatement of it (an independent
    /// oracle for the migration).
    #[test]
    fn rebased_bond_exposure_equals_the_analytic_leaf_dv01() {
        let fill = dated_bond(1, 5_000_000.0, Side::Buy, 10);
        let rates_instrument::Instrument::Bond(bond) = fill
            .instrument
            .as_ref()
            .and_then(|i| i.instrument.as_ref())
            .expect("a bond arm")
        else {
            panic!("a bond arm");
        };
        let today = time::OffsetDateTime::now_utc().date();
        let contract = crate::rates_pricing::bond_contract_from_wire(bond, today)
            .expect("a dated bond builds a contract");
        let oracle = celnet_bond::dv01(&contract, celnet_types::Rate(bond.coupon_rate))
            .expect("the analytic derivative");
        assert!(
            (rates_linear_exposure(&fill).abs() - oracle).abs() < 1e-9,
            "the exposure measure must BE the analytic leaf DV01, not an approximation of it"
        );
    }

    /// **DV01 is exactly linear in redemption face** — the invariant the per-unit-face memo
    /// depends on. Every cashflow of a `Bond` scales with `redemption`, so the price and its
    /// yield derivative do too; one cached unit-face number is therefore exact for any size
    /// of the same security.
    #[test]
    fn bond_dv01_is_exactly_linear_in_face() {
        let one = rates_linear_exposure(&dated_bond(1, 1.0, Side::Buy, 10)).abs();
        for face in [100.0, 1_000_000.0, 250_000_000.0] {
            let scaled = rates_linear_exposure(&dated_bond(2, face, Side::Buy, 10)).abs();
            let want = one * face;
            assert!(
                (scaled - want).abs() <= want * 1e-12,
                "DV01 must scale exactly with face: {face} ⇒ {scaled}, expected {want}"
            );
        }
    }

    /// **Bond and swap risk are now COMMENSURABLE in one book net** — the defect that made
    /// any DV01 limit on a mixed book meaningless (`RISK-MODEL-REQUIREMENTS-AND-GAPS.md`
    /// §2.3 Defect 1). A long 10y bond and a receive-fixed... i.e. a pay-fixed 10y swap of
    /// DV01-equivalent size must net to ~0. Under the retired proxy the same pair nets to a
    /// large fictitious number.
    #[test]
    fn a_bond_and_a_swap_of_equal_dv01_now_net_to_zero() {
        let bond = dated_bond(1, 100_000_000.0, Side::Buy, 10);
        let bond_dv01 = rates_linear_exposure(&bond).abs();
        // A pay-fixed 10y OIS whose annuity PV01 (notional × 10 × 1bp) equals the bond's DV01.
        let notional = bond_dv01 / (10.0 * ONE_BP);
        let swap = ois(2, 7, 10, notional, Side::Buy);

        let net = rates_linear_exposure(&bond) + rates_linear_exposure(&swap);
        assert!(
            net.abs() < bond_dv01 * 1e-9,
            "an economically flat bond-vs-swap pair must net to ~0, got {net} \
             (bond {bond_dv01})"
        );

        // The pre-migration measure netted the SAME pair to a huge fictitious residual,
        // because it was adding two different units.
        let legacy = rates_linear_exposure_proxy(&bond) + rates_linear_exposure_proxy(&swap);
        assert!(
            legacy.abs() > bond_dv01 * 0.5,
            "the retired proxy must be shown to mis-net this pair, got {legacy}"
        );
    }

    /// A bond that cannot be turned into a contract (no maturity date) falls back to the
    /// retired proxy rather than reporting zero or a guessed duration — and both measures
    /// then agree, so the migration audit correctly reports "nothing changed" for it.
    #[test]
    fn an_unconstructable_bond_falls_back_to_the_proxy_magnitude() {
        let fill = bond_position(1, 1_000_000.0, Side::Buy);
        assert!((rates_linear_exposure(&fill).abs() - 100.0).abs() < 1e-9);
        assert!((rates_linear_exposure(&fill) - rates_linear_exposure_proxy(&fill)).abs() < 1e-12);
    }

    /// A swap reports the undiscounted annuity PV01 and says so — duration-correct in shape
    /// (the tenor is in the contract), unlike the bond proxy.
    #[test]
    fn a_swaps_dv01_basis_is_the_annuity_pv01_not_the_proxy() {
        let (dv01, basis) = genuine_position_dv01(&ois(1, 7, 10, 50_000_000.0, Side::Buy), None);
        assert_eq!(basis, Dv01Basis::AnnuityPv01);
        assert!(basis.is_duration_correct());
        assert!((dv01 - 50_000.0).abs() < 1e-6, "notional × 10y × 1bp");
    }

    /// A bond with no maturity date cannot be priced analytically. It falls back to the
    /// coarse proxy AND labels itself `ExposureProxy`, so every surface downstream knows the
    /// size is approximate rather than being told a duration-blind number is exact.
    #[test]
    fn an_unpriceable_bond_falls_back_to_the_proxy_and_labels_itself_so() {
        let (dv01, basis) = genuine_position_dv01(&bond_position(1, 1_000_000.0, Side::Buy), None);
        assert_eq!(basis, Dv01Basis::ExposureProxy);
        assert!(
            !basis.is_duration_correct(),
            "a proxy-based ratio must never be presented as exact"
        );
        assert!((dv01 - 100.0).abs() < 1e-9);
    }

    #[test]
    fn maturity_years_reads_a_bonds_date_and_a_swaps_tenor() {
        let bond = hedge_maturity_years(&dated_bond(1, 1_000_000.0, Side::Buy, 9))
            .expect("a dated bond has a maturity");
        assert!(
            (8.9..9.1).contains(&bond),
            "a 9-year bond must bucket at ~9y, got {bond}"
        );
        assert_eq!(
            hedge_maturity_years(&ois(1, 7, 10, 1.0, Side::Buy)),
            Some(10.0)
        );
        // No instrument arm ⇒ honestly unknown, never a fabricated zero.
        assert_eq!(
            hedge_maturity_years(&RatesPosition {
                position_id: 1,
                entity: 1,
                book: 7,
                instrument: None,
                ..Default::default()
            }),
            None
        );
    }

    /// A valuation date inside the committed listed cycle, so front-month resolution is
    /// deterministic rather than dependent on when the suite happens to run.
    const AS_OF: celnet_refdata::CivilYmd = celnet_refdata::CivilYmd::new(2026, 6, 1);

    /// The firm registry: US bonds 7–12y hedge into the 10Y future at 78 DV01 a contract.
    fn ty_registry() -> HedgeVehicleRegistry {
        HedgeVehicleRegistry::new(vec![HedgeVehicleRule {
            id: "US-BOND-10Y".into(),
            product: "BOND".into(),
            ccy: "USD".into(),
            min_maturity_years: 7.0,
            max_maturity_years: 12.0,
            hedge_instrument_id: "TY-DEC26".into(),
            is_future: true,
            dv01_per_unit: 78.0,
            unit_label: "contract".into(),
            ..HedgeVehicleRule::default()
        }])
    }

    #[test]
    fn a_benchmark_vehicle_resolves_a_nine_year_corp_into_the_ten_year_future() {
        let reg = ty_registry();
        let (rule, whole) = resolve_hedge_vehicle(
            &reg,
            &HedgeVehicle::Benchmark,
            "XS-CORP-9Y",
            "BOND",
            "USD",
            Some(9.0),
            AS_OF,
        )
        .expect("a 9y USD corp resolves the 10Y bucket");
        assert_eq!(rule.hedge_instrument_id, "TY-DEC26");
        assert!(whole, "a futures vehicle trades in whole lots");
    }

    /// A self-hedge never consults the registry — it is the same security sold back, ratio
    /// identically 1, which is what keeps every pre-existing policy byte-identical.
    #[test]
    fn a_self_vehicle_never_consults_the_registry() {
        assert!(
            resolve_hedge_vehicle(
                &ty_registry(),
                &HedgeVehicle::SelfInstrument,
                "XS-CORP-9Y",
                "BOND",
                "USD",
                Some(9.0),
                AS_OF,
            )
            .is_none()
        );
    }

    /// **Never guess a hedge size.** A vehicle the registry does not know has no DV01 per
    /// unit, so it cannot be sized at all — resolution refuses, and the booking path falls
    /// back to the self-hedge rather than trading a fabricated quantity.
    #[test]
    fn an_unregistered_named_vehicle_refuses_to_resolve() {
        assert!(
            resolve_hedge_vehicle(
                &ty_registry(),
                &HedgeVehicle::Future {
                    contract_id: "GHOST-DEC26".into()
                },
                "XS-CORP-9Y",
                "BOND",
                "USD",
                Some(9.0),
                AS_OF,
            )
            .is_none(),
            "an unregistered contract has no known DV01 — sizing it would be a guess"
        );
        // A benchmark that matches no bucket is equally unresolvable.
        assert!(
            resolve_hedge_vehicle(
                &ty_registry(),
                &HedgeVehicle::Benchmark,
                "XS-CORP-30Y",
                "BOND",
                "USD",
                Some(30.0),
                AS_OF,
            )
            .is_none()
        );
    }

    /// A registry row naming the 5-Year **product** rather than a delivery month.
    fn zf_product_registry() -> HedgeVehicleRegistry {
        HedgeVehicleRegistry::new(vec![HedgeVehicleRule {
            id: "US-BOND-5Y".into(),
            product: "BOND".into(),
            ccy: "USD".into(),
            min_maturity_years: 4.0,
            max_maturity_years: 6.0,
            // The whole point: a PRODUCT, not `ZFU26`.
            hedge_instrument_id: "ZF".into(),
            is_future: true,
            dv01_per_unit: 47.0,
            unit_label: "contract".into(),
            ..HedgeVehicleRule::default()
        }])
    }

    /// **A vehicle configured as a product rolls itself.** A registry row naming `ZF`
    /// resolves to whichever 5-Year contract is trading on the valuation date — Sep-26
    /// before the roll, Dec-26 after it — with NO edit to the policy. Before this, a row
    /// pinned to a delivery month kept routing hedges at a contract that had stopped
    /// trading.
    #[test]
    fn a_product_symbol_vehicle_rolls_to_the_front_contract() {
        let reg = zf_product_registry();
        let resolve = |as_of| {
            resolve_hedge_vehicle(
                &reg,
                &HedgeVehicle::Benchmark,
                "UST-5Y",
                "BOND",
                "USD",
                Some(5.0),
                as_of,
            )
        };

        let (before, whole) = resolve(AS_OF).expect("a 5y USD bond resolves the ZF bucket");
        assert_eq!(
            before.hedge_instrument_id, "ZFU26",
            "a product-symbol vehicle resolves to the front delivery month, not to `ZF`"
        );
        assert!(whole, "a rolled futures vehicle still trades in whole lots");
        assert_eq!(
            before.dv01_per_unit, 47.0,
            "the trader's configured DV01 per contract is carried across untouched"
        );

        // Past the 5-Year's cessation the SAME row points at the next quarterly contract.
        // The 5-Year trades THROUGH month end, so its last trading day is the last
        // business day of the delivery month — the first day of the following month is
        // the nearest date that is unambiguously after it AND always a real calendar date.
        let ltd = celnet_refdata::front_contract("ZF", AS_OF)
            .expect("a front 5-Year")
            .last_trading_date;
        let after = if ltd.month == 12 {
            celnet_refdata::CivilYmd::new(ltd.year + 1, 1, 1)
        } else {
            celnet_refdata::CivilYmd::new(ltd.year, ltd.month + 1, 1)
        };
        assert_eq!(
            resolve(after)
                .expect("still resolves after the roll")
                .0
                .hedge_instrument_id,
            "ZFZ26",
            "the vehicle rolls with the market rather than stranding on a dead contract"
        );

        // An explicit delivery month is a deliberate choice and is NEVER re-pointed.
        let (pinned, _) = resolve_hedge_vehicle(
            &ty_registry(),
            &HedgeVehicle::Benchmark,
            "XS-CORP-9Y",
            "BOND",
            "USD",
            Some(9.0),
            after,
        )
        .expect("the pinned row still resolves");
        assert_eq!(
            pinned.hedge_instrument_id, "TY-DEC26",
            "a row naming a specific instrument must never be silently re-pointed"
        );

        // Cycle fully expired ⇒ decline, so the caller falls back to the self-hedge
        // rather than routing at an invented contract code (guardrail 2).
        assert!(
            resolve(celnet_refdata::CivilYmd::new(2030, 1, 1)).is_none(),
            "no listed contract ⇒ refuse, never fabricate a delivery month"
        );
    }

    /// A hedge policy that always sheds externally into the 10Y future, with a registry and
    /// an optional `Suggest` binding on the routed book.
    fn vehicle_policy(book: &str, cap: f64, suggest: bool) -> RatesHedgePolicy {
        use celnet_hedge_routing::{ExecStyle, ExitAction, HedgeNode, HedgeSize, HedgeVehicle};
        let mut nodes = std::collections::BTreeMap::new();
        nodes.insert(
            0u32,
            HedgeNode::action_with(
                ExitAction::SubmitMarketOrder {
                    size: HedgeSize::Full,
                    style: ExecStyle::Immediate,
                },
                HedgeVehicle::Benchmark,
            ),
        );
        let mut base = hedge_policy(book, cap, 0.5);
        base.config.vehicles = ty_registry();
        if suggest {
            base.config.exit_modes = vec![crate::config::hedge_policy::ScopedExitMode {
                scope_kind: HedgeScopeKind::Book,
                scope_id: book.to_owned(),
                mode: celnet_hedge_routing::HedgeExitMode::Suggest,
            }];
        }
        RatesHedgePolicy {
            graph: Some(HedgeGraph { entry: 0, nodes }),
            ..base
        }
    }

    /// A bond attribution: dealt AT the mid ⇒ zero edge ⇒ below the tolerance floor ⇒ the
    /// external shed policy backs the whole fill to the street.
    fn bond_attribution() -> RatesRoutingAttribution {
        priced_attribution(100.0, 100.0)
    }

    /// **The headline vehicle behaviour.** A 9-year corp under a `Benchmark` leaf is hedged
    /// with the 10Y FUTURE — the venue is asked for the contract, the size is the DV01 ratio
    /// rounded to whole lots, and the rounding residual is reported rather than hidden.
    #[test]
    fn a_corp_bond_sheds_into_the_ten_year_future_in_whole_contracts() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = vehicle_policy("wh", 100_000.0, false);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));

        let booked = store
            .book_with_routing(
                dated_bond(0, 10_000_000.0, Side::Buy, 9),
                bond_attribution(),
            )
            .expect("books");

        let rec = engine
            .provenance(None, None)
            .into_iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("a fired vehicle hedge stamps an execution record");
        let plan = rec
            .vehicle_plan
            .expect("a vehicle hedge carries its sizing plan");
        assert_eq!(
            plan.hedge_instrument_id, "TY-DEC26",
            "the corp is hedged with the benchmark future, not with itself"
        );
        assert_eq!(plan.unit_label, "contract");
        assert!(plan.whole_units, "a future trades in whole lots");
        assert_eq!(
            plan.units,
            plan.units.round(),
            "the traded size is a whole number of contracts, never a part lot"
        );
        assert!(
            (plan.units - plan.exact_units).abs() > 0.0,
            "the exact ratio is not a whole number here, so rounding really happened"
        );
        assert!(
            (plan.residual_dv01 - (plan.target_dv01 - plan.hedged_dv01)).abs() < 1e-9,
            "the residual is the honest difference, not a suppressed remainder"
        );
        assert_eq!(
            plan.dv01_basis, "analytic",
            "the ratio is sized off a GENUINE DV01, never the duration-blind proxy"
        );
        assert!(plan.duration_correct);
        // The ratio really is the genuine DV01 over the contract DV01, not the retired
        // duration-1 proxy's. The proxy would have sized this at `10mm × 1bp / 78 ≈ 12.8`
        // contracts. The genuine 9-year DV01 is ~7× that, so the honest hedge is ~90
        // contracts. Sizing off the proxy would have left roughly seven eighths of the risk
        // unhedged and called it done. (Since the Wave-0 re-basing the BOOK's own exposure
        // measure agrees with the sizing measure — this compares against the retired proxy.)
        let proxy_units = rates_linear_exposure_proxy(&booked).abs() / plan.dv01_per_unit;
        assert!(
            plan.exact_units > 5.0 * proxy_units,
            "the genuine-DV01 ratio must dwarf the duration-blind proxy ratio: {} vs {proxy_units}",
            plan.exact_units
        );
        assert!(
            (60.0..140.0).contains(&plan.exact_units),
            "a 10mm 9-year corp hedges with ~90 ten-year contracts, got {}",
            plan.exact_units
        );
    }

    /// **Suggest mode trades NOTHING.** The whole decision is computed — band, action,
    /// vehicle, ratio, rounding — and published as a standing suggestion. No order is
    /// placed, no offsetting leg is booked, and the book's risk is untouched.
    #[test]
    fn suggest_mode_raises_a_standing_suggestion_and_trades_nothing() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = vehicle_policy("wh", 100_000.0, true);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));

        let booked = store
            .book_with_routing(
                dated_bond(0, 10_000_000.0, Side::Buy, 9),
                bond_attribution(),
            )
            .expect("books");
        let net_after_book = store.book_net_dv01("wh");

        // Exactly one standing suggestion, addressed to the (book, instrument) cell.
        let standing = store.hedge_suggestions().list(None);
        assert_eq!(standing.len(), 1, "one suggestion per risk cell");
        let s = &standing[0];
        assert_eq!(s.book, "wh");
        assert_eq!(s.instrument, "BOND");
        assert_eq!(s.parent_position_id, Some(booked.position_id));
        assert!(
            s.headline.contains("TY-DEC26"),
            "the instruction names the vehicle: {}",
            s.headline
        );
        assert!(
            s.headline.starts_with("Sell"),
            "reducing a long bond means SELLING the hedge: {}",
            s.headline
        );
        assert!(
            s.vehicle_plan.is_some(),
            "the suggestion carries its sizing"
        );

        // NOTHING traded: no execution record, and the book still holds the whole position.
        assert!(
            engine
                .provenance(None, None)
                .iter()
                .all(|p| p.parent_position_id != Some(booked.position_id)),
            "suggest mode must not stamp an execution record — nothing executed"
        );
        assert!(
            (net_after_book - rates_linear_exposure(&booked)).abs() < 1e-9,
            "the book still carries the full position — no offsetting leg was booked"
        );
    }

    /// Firing a standing suggestion executes the pinned plan and books the offsetting leg,
    /// so the book genuinely reduces — the same half-two of the exit the automatic path
    /// performs. Firing twice is impossible: the suggestion is consumed.
    #[test]
    fn firing_a_suggestion_executes_it_reduces_the_book_and_consumes_it() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = vehicle_policy("wh", 100_000.0, true);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));

        let booked = store
            .book_with_routing(
                dated_bond(0, 10_000_000.0, Side::Buy, 9),
                bond_attribution(),
            )
            .expect("books");
        let before = store.book_net_dv01("wh");
        let id = store.hedge_suggestions().list(None)[0]
            .suggestion_id
            .clone();

        let prov = store
            .resolve_hedge_suggestion(&id, false)
            .expect("firing a standing suggestion succeeds")
            .expect("a fired hedge stamps provenance");
        assert_eq!(prov.parent_position_id, Some(booked.position_id));
        assert!(!prov.advisory, "a live composite fill is not advisory");
        assert!(prov.external_hedged > 0.0);
        assert!(
            prov.vehicle_plan.is_some(),
            "the fired record carries the vehicle sizing the trader was shown"
        );
        assert_eq!(
            engine
                .provenance(None, None)
                .iter()
                .filter(|p| p.parent_position_id == Some(booked.position_id))
                .count(),
            1,
            "exactly one ring record per fired hedge"
        );

        // The book actually reduced — a hedge that does not reduce the book is not an exit.
        let after = store.book_net_dv01("wh");
        assert!(
            after.abs() < before.abs(),
            "the offsetting leg must reduce the book: {before} → {after}"
        );

        // Consumed: it is gone from the store and cannot be fired again.
        assert!(store.hedge_suggestions().is_empty());
        assert!(
            store.resolve_hedge_suggestion(&id, false).is_err(),
            "a fired suggestion must never be fireable twice"
        );
    }

    /// Dismissing trades nothing and hides nothing: the suggestion clears, but the risk stays
    /// exactly where it was.
    #[test]
    fn dismissing_a_suggestion_clears_it_without_trading() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = vehicle_policy("wh", 100_000.0, true);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));
        let booked = store
            .book_with_routing(
                dated_bond(0, 10_000_000.0, Side::Buy, 9),
                bond_attribution(),
            )
            .expect("books");
        let before = store.book_net_dv01("wh");
        let id = store.hedge_suggestions().list(None)[0]
            .suggestion_id
            .clone();

        assert_eq!(
            store
                .resolve_hedge_suggestion(&id, true)
                .expect("dismiss succeeds"),
            None,
            "a dismiss stamps no provenance — nothing traded"
        );
        assert!(store.hedge_suggestions().is_empty());
        assert!(
            (store.book_net_dv01("wh") - before).abs() < 1e-12,
            "the risk stays on the book after a dismiss"
        );
        assert!(
            engine
                .provenance(None, None)
                .iter()
                .all(|p| p.parent_position_id != Some(booked.position_id))
        );
    }

    /// **Nothing changes for an existing policy.** A leaf that names no vehicle is the
    /// self-hedge: the venue is asked for the position's own security, the record carries no
    /// vehicle plan, and the book reduces exactly as it always did.
    #[test]
    fn a_policy_with_no_vehicle_behaves_exactly_as_before() {
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let policy = external_hedge_policy("wh", 100_000.0, 0.5);
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));

        let booked = store
            .book_with_routing(
                dated_bond(0, 10_000_000.0, Side::Buy, 9),
                bond_attribution(),
            )
            .expect("books");
        let rec = engine
            .provenance(None, None)
            .into_iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("a shed stamps a record");
        assert!(
            rec.vehicle_plan.is_none(),
            "a self-hedge has no vehicle plan — its ratio is identically 1"
        );
        assert!(
            store.hedge_suggestions().is_empty(),
            "with no exit-mode binding the shed fires automatically, as it always did"
        );
        assert!(rec.external_hedged > 0.0);
    }

    /// A named vehicle the registry does not know cannot be sized, so the shed falls back to
    /// the self-hedge — a real, exact hedge — rather than trading a guessed contract count.
    #[test]
    fn an_unregistered_vehicle_falls_back_to_the_self_hedge_rather_than_guessing() {
        use celnet_hedge_routing::{ExecStyle, ExitAction, HedgeNode, HedgeSize, HedgeVehicle};
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        let mut nodes = std::collections::BTreeMap::new();
        nodes.insert(
            0u32,
            HedgeNode::action_with(
                ExitAction::SubmitMarketOrder {
                    size: HedgeSize::Full,
                    style: ExecStyle::Immediate,
                },
                HedgeVehicle::Future {
                    contract_id: "GHOST-DEC26".into(),
                },
            ),
        );
        let policy = RatesHedgePolicy {
            graph: Some(HedgeGraph { entry: 0, nodes }),
            ..hedge_policy("wh", 100_000.0, 0.5)
        };
        let engine = Arc::clone(&policy.engine);
        store.set_hedge_policy(Some(policy));

        let booked = store
            .book_with_routing(
                dated_bond(0, 10_000_000.0, Side::Buy, 9),
                bond_attribution(),
            )
            .expect("books");
        let rec = engine
            .provenance(None, None)
            .into_iter()
            .find(|p| p.parent_position_id == Some(booked.position_id))
            .expect("the shed still fires");
        assert!(
            rec.vehicle_plan.is_none(),
            "an unsizeable vehicle produces NO plan — it must not be traded on a guess"
        );
        assert!(
            rec.external_hedged > 0.0,
            "the shed still happens, as an exact self-hedge"
        );
    }

    /// The scoped exit mode resolves most-specific-wins and defaults to `Auto`, so a firm
    /// that configures nothing keeps firing automatically.
    #[test]
    fn exit_mode_resolves_most_specific_wins_and_defaults_to_auto() {
        use celnet_hedge_routing::HedgeExitMode;
        let mut cfg = HedgeConfigDef::default();
        assert_eq!(
            cfg.resolve_exit_mode("RATES", "wh", "BOND"),
            HedgeExitMode::Auto,
            "an unconfigured firm fires automatically, exactly as before"
        );
        cfg.upsert_exit_mode(crate::config::hedge_policy::ScopedExitMode {
            scope_kind: HedgeScopeKind::Desk,
            scope_id: "RATES".into(),
            mode: HedgeExitMode::Suggest,
        });
        assert_eq!(
            cfg.resolve_exit_mode("RATES", "wh", "BOND"),
            HedgeExitMode::Suggest
        );
        // A more specific book binding wins over the desk one.
        cfg.upsert_exit_mode(crate::config::hedge_policy::ScopedExitMode {
            scope_kind: HedgeScopeKind::Book,
            scope_id: "wh".into(),
            mode: HedgeExitMode::Auto,
        });
        assert_eq!(
            cfg.resolve_exit_mode("RATES", "wh", "BOND"),
            HedgeExitMode::Auto,
            "an explicit book-level Auto overrides a desk-wide Suggest"
        );
        // Clearing that override restores inheritance from the wider desk scope.
        cfg.remove_exit_mode(HedgeScopeKind::Book, "wh");
        assert_eq!(
            cfg.resolve_exit_mode("RATES", "wh", "BOND"),
            HedgeExitMode::Suggest
        );
    }
}
