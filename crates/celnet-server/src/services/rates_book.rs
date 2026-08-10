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
use celnet_hedge_routing::{HedgeContext, HedgeGraph};
use celnet_limits::{
    IncrementalTrade, LimitScope, LimitSpec, LimitTree, NonAdditiveExposure, PreTradeDecision,
    PreTradeResult, ScopePath, pre_trade_check,
};
use celnet_proto::{
    EntitlementPrincipal, EntitlementRule, HedgeProvenance, InternaliseProvenance, RatesPosition,
    RiskDimension, Side, rates_instrument,
};
use celnet_risk_cube::{BookId, EntityId, NetGreeks, NodeAggregate, VegaPillar};
use celnet_risk_routing::{RiskRouter, RiskRoutingGraph, RoutingContext};

use crate::config::hedge_policy::{
    HedgeConfigDef, HedgePolicyScope, HedgeScopeKind, HedgeThresholdDef, ScopedHedgeGraph,
    ScopedThreshold,
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
    /// The shared **hedge-execution → street-side LP flow log** (Analytics §2.4). A hedge that
    /// FILLED on a named LP records a WON deal + won notional there, off-core, so the Street-side
    /// LP league table attributes the fill to the executing LP (never to the COMPOSITE
    /// pseudo-venue). The SAME `Arc` the analytics rollup folds as an `LpFlowSource`; `None` (the
    /// unit-test default) means the attribution call is a no-op.
    hedge_flow: OnceLock<Arc<crate::services::analytics::hedge_flow::HedgeFillFlowLog>>,
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
            .field("has_hedge_flow_log", &self.hedge_flow.get().is_some())
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
            hedge_flow: OnceLock::new(),
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

    /// Attach the shared **hedge-execution → street-side LP flow log** (Analytics §2.4) so a
    /// hedge that fills on a named LP is attributed to that LP in the LP league table. The SAME
    /// `Arc` the analytics rollup folds as an `LpFlowSource`; wired once at boot, idempotent-once.
    /// A store never given one simply records no hedge-LP attribution (byte-identical otherwise).
    pub fn set_hedge_flow_log(
        &self,
        log: Arc<crate::services::analytics::hedge_flow::HedgeFillFlowLog>,
    ) {
        let _ = self.hedge_flow.set(log);
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
        exclude_position_ids: &[u64],
    ) -> Result<(), tonic::Status> {
        match self.project_rates_book_breach_multi(
            target_book,
            incoming_net,
            incoming_gross,
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
                    return Err(limit_breached_status(&result));
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
        self.stamp_internalise(
            &position,
            resolved_book.as_deref(),
            &attribution,
            booking_trace_id,
        );
        // O3: record the ack→fill→book commit latency into the per-`OpKind` store (mirrors the
        // FX sink). Off the pinned pricing core; a no-op when no hub is installed.
        self.record_latency(
            celnet_observability::OpKind::Book,
            u64::try_from(book_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
        Ok(position)
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
        let (graph, bucket_root) = match policy.select_scoped_graph(book) {
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
        // Resolve the most-specific warehouse threshold for this fill (instrument > book >
        // desk). No threshold configured for ANY of the fill's scopes — e.g. it routed into an
        // operator-defined risk book the firm never bound a warehouse cap to — falls back to
        // the default firm warehouse budget, so the fill is still measured against a real cap
        // and stamps a decision, never silently carrying none.
        // For a `Bucket` policy the "100" is the bucket root's threshold and the risk state is
        // the subtree roll-up; otherwise it is the fill's own book.
        let scope_book: &str = bucket_root.as_deref().unwrap_or(book);
        let thr_def = resolve_hedge_threshold(&policy.thresholds, "", scope_book, &instrument)
            .unwrap_or_else(default_warehouse_threshold_def);
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
        let fill_dv01 = rates_linear_exposure(fill).abs();

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
        let ctx = HedgeContext {
            instrument_id: instrument,
            ccy: attribution.ccy.clone(),
            book: book.to_owned(),
            // The originating counterparty of THIS fill — the same party id the Deal /
            // blotter and the risk-routing attribution carry — so a hedge rule
            // `counterparty == "X"` back-to-backs a given client's flow (a property of the
            // incoming fill, not a per-counterparty net position).
            counterparty: attribution.counterparty.clone(),
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
            net_dv01: book_net_dv01,
            net_notional: book_net_notional,
            breached: wh.breached(book_net_dv01),
            threshold: wh.cap,
            utilization: wh.utilization(book_net_dv01),
            overflow: wh.overflow(book_net_dv01),
            internal_offset_available: 0.0,
            ..HedgeContext::default()
        };
        // Best-order timer: bracket the auto-hedge decision (`OpKind::HedgeFire`) — the pure
        // engine's threshold-breach → hedge-action evaluation on this fill's post-book book
        // risk. Measured at the call site (not inside the engine) so the pinned/pure engine
        // stays telemetry-free; recorded off the async booking tier via the store's hub
        // (guardrail 11), a no-op when no hub is installed.
        let hedge_t0 = std::time::Instant::now();
        let outcome = policy.engine.evaluate(
            graph,
            &thr_def,
            &policy.config,
            &ctx,
            &policy.known_lps,
            now,
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
        let (internal_dv01, external_dv01, internalised) = if verdict.within_tolerance {
            let ext = shed.min(fill_dv01).max(0.0);
            (fill_dv01 - ext, ext, ext == 0.0)
        } else if graph_is_external {
            (0.0, fill_dv01, false)
        } else {
            (fill_dv01, 0.0, true)
        };

        let prov = InternaliseProvenance {
            internalised,
            internal_dv01,
            external_dv01,
            edge_bps: verdict.edge_bps,
            within_tolerance: verdict.within_tolerance,
            hedge_band: band_label(wh.classify(book_net_dv01)).to_owned(),
        };
        // HEDGING (class=Hedge → orders sink): the internalise-vs-back-to-back decision —
        // the price-tolerance verdict, the internal/external DV01 split, the warehouse-cap
        // utilisation and RAG band. WARN when the routed book has BREACHED its warehouse
        // cap (the desk must shed), INFO otherwise. Async booking tier, never the core.
        let warehouse_breached = wh.breached(book_net_dv01);
        let utilization = wh.utilization(book_net_dv01);
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
                        "band={}; {}; edge={:.2}bp; int_dv01={:.1}; ext_dv01={:.1}",
                        prov.hedge_band,
                        if prov.internalised {
                            "internalised"
                        } else {
                            "shed"
                        },
                        prov.edge_bps,
                        prov.internal_dv01,
                        prov.external_dv01,
                    )),
                    ..Default::default()
                },
            );
            if external_dv01 > 0.0 {
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
            let req = crate::services::auto_hedge::ExternalHedgeRequest {
                instrument: &ctx.instrument_id,
                net_risk: book_net_dv01,
                size: external_dv01,
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
            let exec = match self.lp_hedge_source.get() {
                Some(src) => crate::services::auto_hedge::execute_external(&req, src.as_ref()),
                None => crate::services::auto_hedge::execute_external(
                    &req,
                    &crate::services::auto_hedge::NoLpSource,
                ),
            };
            // Book the offsetting leg into the same book so the warehoused net reduces by the
            // filled amount. `book_into_risk_book` re-runs the hard-cap gate (a reducing leg
            // never breaches, §8.3) and does NOT recurse into `stamp_internalise`. Best-effort:
            // a leg we cannot construct (e.g. a bond's coarse linear proxy) is skipped, never
            // faked — the record below is still stamped with the real executed economics.
            if exec.is_filled()
                && fill_dv01 > 0.0
                && let Some(leg) = offsetting_rates_leg(fill, exec.filled / fill_dv01)
            {
                let _ = self.book_into_risk_book(leg, book);
            }
            // Street-side LP attribution (Analytics §2.4): a hedge that FILLED on a NAMED LP
            // records a WON deal + won notional for that LP into the shared flow log the LP
            // league table folds, off-core. A composite-venue fill is deliberately NOT attributed
            // to a named LP (COMPOSITE is a pseudo-venue, not a street LP); no last-look / cover
            // is honestly known for a hedge fill, so those stay absent (guardrail 2).
            if exec.venue == Some(crate::services::auto_hedge::HedgeVenue::LpPanel)
                && let Some(lp) = exec.lp_won.as_deref()
                && let Some(log) = self.hedge_flow.get()
            {
                log.record_fill(lp, ctx.instrument_id.clone(), exec.filled, now);
            }
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
                net_risk: book_net_dv01,
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
                // A real venue fill is NOT advisory; a miss / advisory-mode is.
                advisory: !exec.is_filled(),
                lps: outcome.intent.lps.clone(),
                parent_position_id: Some(fill.position_id),
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
        self.project_rates_book_breach_multi(resolved_book, n, n.abs(), &[fill.position_id])
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
        let mut own: HashMap<&str, (f64, f64)> = HashMap::new();
        for p in g.iter() {
            if exclude_ids.contains(&p.position_id) {
                continue;
            }
            if let Some(book) = stamps.get(&p.position_id) {
                let n = rates_signed_notional(p);
                let e = own.entry(book.as_str()).or_insert((0.0, 0.0));
                e.0 += n;
                e.1 += n.abs();
            }
        }
        for (scope, lim) in scoped {
            // Subtree roll-up: the incoming batch + every current book whose ancestor-or-self
            // chain passes through `scope` (i.e. the book is in the subtree rooted at `scope`).
            let mut net = incoming_net;
            let mut gross = incoming_gross;
            for (book, (bnet, bgross)) in &own {
                if rates_risk_book_chain(&limits_map, book).contains(&scope) {
                    net += bnet;
                    gross += bgross;
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
            // `max_dv01` intentionally skipped — the per-book DV01 cap is the dedicated
            // rates-risk seam, not this notional-cap gate.
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
/// Construct the **offsetting hedge leg** for a shed of `factor · |fill exposure|` (§6.2):
/// a copy of `fill`'s instrument with its side FLIPPED and its notional scaled by `factor`
/// (`= hedged_dv01 / fill_dv01`, in `(0, 1]`), so its signed linear exposure is exactly the
/// negation of the shed portion — booking it reduces the book's net by the hedged amount.
/// `position_id` is zeroed so [`RatesPositionStore::book_into_risk_book`] assigns a fresh id.
///
/// Returns `None` for a non-positive factor or a bond leg (whose curve-DV01 scaling is not
/// modelled by this linear proxy) — the caller then skips booking rather than fabricating a
/// leg (guardrail 2).
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
        // A cash bond's linear-proxy exposure is not side/notional-scaled here — skip the
        // physical leg (the executed economics are still stamped on the provenance record).
        _ => return None,
    }
    Some(leg)
}

#[must_use]
pub(crate) fn rates_linear_exposure(position: &RatesPosition) -> f64 {
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
        }
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
        }
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

    // ---- auto-hedge / internalisation decision (§6) --------------------------
    //
    // A 5y 10mm pay-fixed (Buy) OIS charges `+5000` of signed linear DV01 (`10mm·5·1bp`).
    // The internalise decision runs only when a hedge policy is primed AND the fill carried a
    // priced dealt level + reference mid. The pieces combine: the price-tolerance verdict
    // (did the desk capture edge?) × the warehouse-cap sizing (is the book over its "100"?).

    use crate::config::hedge_policy::HedgeMetric;

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
            HedgeNode::Action {
                exit: ExitAction::SubmitMarketOrder {
                    size: HedgeSize::Full,
                    style: ExecStyle::Immediate,
                },
            },
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
        fn best_fill(
            &self,
            _instrument: &str,
            net_risk: f64,
            _size: f64,
        ) -> Option<crate::services::auto_hedge::LpFill> {
            let price = if net_risk > 0.0 { self.bid } else { self.offer };
            Some(crate::services::auto_hedge::LpFill {
                lp_id: self.lp.to_owned(),
                price,
            })
        }
    }

    /// A stub LP source that never has a firm price (an honest miss) — for the composite-fallback path.
    struct NoFillLp;
    impl crate::services::auto_hedge::LpHedgeSource for NoFillLp {
        fn best_fill(
            &self,
            _instrument: &str,
            _net_risk: f64,
            _size: f64,
        ) -> Option<crate::services::auto_hedge::LpFill> {
            None
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
                (
                    1u32,
                    HedgeNode::Action {
                        exit: ExitAction::Warehouse,
                    },
                ),
                (
                    2u32,
                    HedgeNode::Action {
                        exit: ExitAction::SubmitMarketOrder {
                            size: HedgeSize::Full,
                            style: ExecStyle::Immediate,
                        },
                    },
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

    /// PART C: a hedge that fills on a NAMED LP is attributed to that LP in the street-side LP
    /// flow rollup (a WON deal + won notional), so the LP league table shows the win.
    #[tokio::test]
    async fn hedge_fill_on_named_lp_is_attributed_in_the_lp_flow_rollup() {
        use crate::services::analytics::hedge_flow::HedgeFillFlowLog;
        use crate::services::analytics::lp::{LpFlowSource, fold};
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(external_hedge_policy("wh", 100_000.0, 0.5)));
        store.set_lp_hedge_source(Arc::new(StubLp {
            lp: "LP-SIM-01",
            bid: 0.0404,
            offer: 0.0406,
        }));
        let log = Arc::new(HedgeFillFlowLog::new());
        store.set_hedge_flow_log(Arc::clone(&log));
        store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");

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
        use crate::services::analytics::hedge_flow::HedgeFillFlowLog;
        use crate::services::analytics::lp::LpFlowSource;
        let store = RatesPositionStore::new();
        store.set_routing(Some(single_book_graph("wh")));
        store.set_hedge_policy(Some(external_hedge_policy("wh", 100_000.0, 0.5)));
        // No LP source ⇒ the default LP-then-composite shed fills on the composite venue.
        let log = Arc::new(HedgeFillFlowLog::new());
        store.set_hedge_flow_log(Arc::clone(&log));
        store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0405, 0.0405))
            .expect("books");
        assert!(
            log.lp_flow_records(None, None).await.is_empty(),
            "a COMPOSITE fill is not a street-LP win"
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
}
