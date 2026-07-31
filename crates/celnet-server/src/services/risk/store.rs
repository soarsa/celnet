//! The server-side **live position book** the risk cube aggregates over.
//!
//! [`RiskService`](super::RiskEdge) does not invent positions: it reads the desk's
//! open book — the **same** booked lines the RFS click-to-trade path
//! ([`crate::services::stream`]) produces and the GUI Book view used to loop over
//! client-side. This module is that shared store.
//!
//! # Why a store, not the engine `BookState`
//!
//! The pinned hot core's [`celnet_engine::BookState`] is deliberately lean (an id,
//! option type, strike, notional) — it is the zero-alloc repricing book, not a risk
//! warehouse. Hierarchical risk needs the *full* position fact: the pair, the whole
//! [`VanillaInputs`](celnet_types::VanillaInputs) set, the quoted conventions, the
//! marking surface version, **and** the organizational attribution
//! (`trader → book → desk → ccy-pair → location → entity`). Carrying all of that on
//! the hot-path book would bloat the zero-alloc core for data it never prices with,
//! so the risk fact lives here, on the async edge, strictly off the pricing path
//! (`docs/RISK-HIERARCHY.md` §3.5: the cube runs on the recompute cadence, never the
//! µs hot loop).
//!
//! # What it holds
//!
//! * the immutable [`RiskFact`](celnet_risk_cube::RiskFact) table (one current fact
//!   per `position_id`, the cube's `upsert` supersede semantics);
//! * the org [`Hierarchy`](celnet_risk_cube::Hierarchy) (`Book → Desk`,
//!   `Location → Entity` parent pointers);
//! * a string→`u32` **interner** so a wire `AttributionRecord` (book/seat strings)
//!   maps onto the cube's interned dimension handles deterministically, and a
//!   reverse map so [`ListPositions`](super::RiskEdge) can reconstruct the
//!   attribution chain it reports;
//! * the [`LimitTree`](celnet_limits::LimitTree) configured at hierarchy scopes.
//!
//! # Scope: vanilla AND exotic legs
//!
//! The risk cube aggregates **vanilla** option leaves (`celnet-risk-normalize`'s
//! `canonicalize` re-derives a vanilla `PositionRisk` via `celnet-vanilla`) **and**,
//! since the cube grew an exotic-leaf measure (`celnet_risk_cube::exotic`),
//! closed-form **exotic** legs (single barrier / European digital). A booked exotic
//! is recorded via [`PositionStore::upsert_exotic`] as a [`RiskFact`] carrying its
//! REAL exotic Greeks (additive) and the [`ExoticLeg`](celnet_risk_cube::ExoticLeg)
//! re-derivation source (non-additive) — so it rolls into firm/desk/book risk
//! instead of being silently excluded. The Greeks are the true exotic
//! sensitivities, not a vanilla proxy (guardrail #2).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use crate::services::consensus::{ConsensusHandle, fx_book_key};

use celnet_entitlements::AccessMode;
use celnet_proto::{AttributionRecord, BookId, Owner, owner};
use celnet_risk_cube::{
    BookId as CubeBookId, DeskId, EntityId, FactKey, FactMeasure, Hierarchy, LocationId,
    NodeAggregate, PositionId, RiskFact, TraderId, VegaPillarMap,
};
use celnet_risk_normalize::{CanonicalLeaf, PositionRisk, canonicalize};
use celnet_types::{Carry, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

use celnet_limits::{
    IncrementalTrade, LimitScope, LimitTree, NonAdditiveExposure, PreTradeDecision, PreTradeResult,
    ScopePath, pre_trade_check,
};
use celnet_risk_routing::{RiskRouter, RiskRoutingGraph, RoutingContext};

use crate::config::identity::{RiskBookDef, RiskLimits};

use super::aggregate::{cube_from_facts, default_grid};

/// A deterministic string→`u32` interner: distinct strings get distinct,
/// monotonically-increasing handles starting at `1` (handle `0` is reserved as the
/// "resolve from parent / unknown" sentinel the wire `OrgKey` uses). The reverse
/// map recovers the original string for a handle.
#[derive(Debug, Default, Clone)]
struct Interner {
    forward: HashMap<String, u32>,
    reverse: Vec<String>,
}

impl Interner {
    /// The handle for `s`, interning it on first sight. Never returns `0`.
    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&h) = self.forward.get(s) {
            return h;
        }
        // Handles start at 1; `reverse[h-1]` is the string for handle `h`.
        let h = u32::try_from(self.reverse.len() + 1).expect("interner handle space exhausted");
        self.forward.insert(s.to_owned(), h);
        self.reverse.push(s.to_owned());
        h
    }

    /// The string for a handle, or `None` for the `0` sentinel / an unknown handle.
    /// Exercised by the interner round-trip test; the public reverse map (carried on
    /// [`StoreSnapshot`]) is the production name-reporting path.
    #[cfg(test)]
    fn resolve(&self, handle: u32) -> Option<&str> {
        if handle == 0 {
            return None;
        }
        self.reverse.get((handle - 1) as usize).map(String::as_str)
    }
}

/// One booked position as captured by the live book, before it is canonicalized
/// into a [`RiskFact`]. This is the edge-side analogue of a `celnet-proto`
/// `RiskPosition`: the raw economics + org placement, never a convention-baked
/// Greek (the canonical leaf is re-derived on insert).
#[derive(Debug, Clone, Copy)]
pub struct BookedPosition {
    /// The position identity (one current fact per id; a re-book supersedes).
    pub position_id: u64,
    /// The currency pair (BASE/QUOTE).
    pub pair: CcyPair,
    /// Call or put on the base currency.
    pub option: OptionType,
    /// Signed base-currency notional (positive = long the option).
    pub notional_base: f64,
    /// The pricing inputs the position was marked under.
    pub inputs: celnet_types::VanillaInputs,
    /// The delta convention the position was quoted under (provenance).
    pub quoted_delta: DeltaConvention,
    /// The premium style the position was quoted under (provenance).
    pub premium_style: PremiumStyle,
    /// The marked-surface version that produced this fact.
    pub surface_version: u64,
}

/// One booked **exotic** position as captured by the live book, before it is recorded
/// as a [`RiskFact`]. Carries the closed-form exotic leg
/// ([`ExoticLeg`](celnet_risk_cube::ExoticLeg)) plus the metadata the canonical leaf
/// and bucketing need.
#[derive(Debug, Clone, Copy)]
pub struct BookedExotic {
    /// The position identity (one current fact per id; a re-book supersedes).
    pub position_id: u64,
    /// The closed-form exotic leg (kind + spec + inputs + signed notional + pair).
    pub leg: celnet_risk_cube::ExoticLeg,
    /// The underlying option type (for the bucketing metadata).
    pub option: OptionType,
    /// The delta convention quoted under (provenance / bucketing metadata).
    pub quoted_delta: DeltaConvention,
    /// The premium style quoted under (provenance / bucketing metadata).
    pub premium_style: PremiumStyle,
    /// The marked-surface version that produced this fact.
    pub surface_version: u64,
}

/// A read view of one booked FX position assembled for a **risk transfer** (§6): the
/// slice fields the pure leg computation needs (signed notional, per-unit mark, canonical
/// greeks) plus the reconstructed [`BookedPosition`] an economic transfer re-books its
/// offsetting / opening legs from. Produced by [`PositionStore::fx_transfer_view`].
#[derive(Debug, Clone)]
pub struct FxTransferView {
    /// The risk book the position is currently stamped into (`None` ⇒ unrouted).
    pub risk_book: Option<String>,
    /// Signed base-currency notional (+ long, − short).
    pub signed_notional: f64,
    /// The per-unit "current mark": the marked option premium (PV) in the quote/numeraire
    /// currency per unit of base notional (`leaf.premium_quote / notional_base`) — the fair
    /// value the store carries at the fact's `surface_version`. A transfer at `Mid`/`MTM`
    /// resolves to this ⇒ zero realised P&L; an `Agreed` override crosses P&L against it.
    pub mark: f64,
    /// Canonical, notional-scaled spot delta (base-leg hedge amount).
    pub delta: f64,
    /// Canonical, notional-scaled gamma.
    pub gamma: f64,
    /// Canonical, notional-scaled vega (premium-currency terms).
    pub vega: f64,
    /// Canonical, notional-scaled theta (per year).
    pub theta: f64,
    /// The reconstructed booking (instrument + inputs + conventions + surface version) —
    /// the template an economic transfer re-books offsetting / opening legs from.
    pub booked: BookedPosition,
}

/// A compact, server-side projection of a [`RiskBookDef`](crate::config::identity::RiskBookDef)
/// carrying only what the booking sink's per-book limit gate needs: the book id, its parent
/// (for ancestor roll-up), and its optional hard [`RiskLimits`]. Pushed into the store by
/// the reconcile hooks ([`PositionStore::set_risk_books`]) whenever a risk book or the graph
/// is created / edited / deleted, so the store's limit view stays current without pulling the
/// full config type onto the booking path.
#[derive(Debug, Clone)]
pub struct RiskBookLimitDef {
    /// The risk book id (matches the `risk_book_id` the routing graph resolves to).
    pub id: String,
    /// The parent book id for subtree roll-up, or `None` for a top-level book.
    pub parent_id: Option<String>,
    /// The book's optional hard notional caps. `None` ⇒ the book carries no caps.
    pub limits: Option<RiskLimits>,
}

impl From<&RiskBookDef> for RiskBookLimitDef {
    fn from(def: &RiskBookDef) -> Self {
        Self {
            id: def.id.clone(),
            parent_id: def.parent_id.clone(),
            // `RiskLimits` is `Copy`, so the caps are copied, not shared.
            limits: def.limits,
        }
    }
}

/// The shared live position book + org hierarchy + attribution interner + limit
/// tree the risk service aggregates over. Cheap to share behind an [`Arc`]; every
/// mutation takes the write lock briefly (the cube runs off the hot path, so the
/// lock is never contended by the pricing core).
///
/// [`Arc`]: std::sync::Arc
#[derive(Debug)]
pub struct PositionStore {
    inner: RwLock<StoreInner>,
    /// The entitlements **trust-boundary mode** guarding this book
    /// ([`crate::services::access`]): `false` ⇒ [`AccessMode::Enforce`] (the
    /// deny-by-default production posture and the construction default),
    /// `true` ⇒ the explicit [`AccessMode::Permissive`] dev-mode (set only by
    /// the demo edge, loudly). Lives on the shared store — not per edge — so
    /// the gRPC server, the WS mirror and a federating frontend all read one
    /// coherent policy, lock-free per request.
    permissive_access: AtomicBool,
    /// The optional activated consistency tier (ADR-0015 §2.1). `Some` only when a
    /// `Strong`-tier book is configured and Raft was booted at edge start. A booking
    /// whose book resolves to [`ConsistencyLevel`](crate::config::consistency::ConsistencyLevel)`::Strong`
    /// routes its authoritative write through the quorum log here **before** the local
    /// apply; a `Local` book (the default) never touches it, so the fast path stays
    /// byte-identical. Set once at boot and read on the async booking tier only — never
    /// the pinned pricing thread (§4.3).
    consensus: OnceLock<Arc<ConsensusHandle>>,
    /// The shared latency/ops telemetry hub (best-order timer O3 — ack→fill→book
    /// commit latency, `docs/LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md` §4.3).
    /// Set once at boot; a successful [`Self::book`] records its commit latency here,
    /// on the async booking tier, never the pinned pricing thread (guardrail 11).
    telemetry: OnceLock<Arc<crate::services::telemetry::TelemetryHub>>,
    /// A monotonic **risk version** bumped on every change that can alter a risk book's
    /// aggregated risk: a successful routed [`Self::book`] (a position change) and an
    /// admin edit of the risk-book tree ([`Self::set_risk_book_tree`]). It is the signal
    /// the live per-book risk stream ([`crate::services::stream`]) polls to decide "risk
    /// changed, re-publish" — the risk analogue of the aggregation hub's per-book
    /// composite version. A relaxed atomic (read lock-free off the streaming tick loop,
    /// never the pinned pricing core); starts at `0`.
    risk_version: AtomicU64,
    /// A monotonic **transfer id source** for the freshly-minted position handles a risk
    /// transfer (§6) needs — the moved slice of a partial re-attribution split and the
    /// offsetting / opening legs of an economic transfer. [`Self::mint_position_id`] hands
    /// out ids strictly above the current max live handle, so a minted id never collides
    /// with a booked position. The FX book (unlike the rates book) does not otherwise assign
    /// ids — client fills carry their own — so this counter exists solely for transfer legs.
    /// A relaxed atomic on the async booking tier; starts at `0`.
    next_transfer_id: AtomicU64,
}

#[derive(Debug, Default)]
struct StoreInner {
    /// The immutable fact table (one current fact per `position_id`).
    facts: Vec<RiskFact>,
    /// The org hierarchy parent pointers, shared with the entitled cube.
    hierarchy: Hierarchy,
    /// The attribution-name interner (handle ↔ string), so a booked line's
    /// book/seat strings map to cube handles and `ListPositions` can reconstruct
    /// the attribution chain it reports.
    interner: Interner,
    /// Per-(position) reconstruction provenance: the attribution chain a booked
    /// line was recorded under, so `ListPositions` reports the real chain rather
    /// than a synthesized one. Keyed by the cube's interned `u32` position handle.
    attribution: HashMap<u32, AttributionRecord>,
    /// The wire (business) `u64` position id for each cube `u32` handle, so a
    /// `ListPositions` / `DrillRisk` leaf reports the exact id the client booked
    /// under (the cube handle is a `u32` internal key; the wire id is `u64`).
    wire_ids: HashMap<u32, u64>,
    /// The limit tree configured at hierarchy scopes (caps + RAG bands).
    limits: LimitTree,
    /// The current firm-wide **risk-routing graph** (`docs/FI-RISK-ROUTING-REQUIREMENTS.md`
    /// §4), if configured. `None` ⇒ no routing: a fill books exactly as before with
    /// **no** risk-book stamp — byte-identical to the pre-routing path. Held behind an
    /// [`Arc`] so a fill clones only a pointer (never the graph) on the off-hot-path
    /// booking tier; swapped wholesale by [`PositionStore::set_routing`] when an admin
    /// defines/edits/clears the graph.
    routing: Option<Arc<RiskRoutingGraph>>,
    /// The resolved `risk_book_id` stamped on each booked position at book time, keyed
    /// by the cube `u32` position handle — the new **risk-book bucketing dimension**
    /// beside the attribution keying (§8.3). A handle is present here only when its fill
    /// was routed (a graph was configured and [`RiskRouter::route`] resolved a book); an
    /// absent handle ⇒ unrouted (`None`). The attribution/hierarchy keying is untouched,
    /// so an unrouted store is byte-identical to today.
    risk_book: HashMap<u32, String>,
    /// The current **risk-book limit view** (§8.3 enforcement): each routed book's parent
    /// (for subtree roll-up) + optional hard [`RiskLimits`], keyed by book id. Kept current
    /// by [`PositionStore::set_risk_books`] from the reconcile hooks. Empty — or a view with
    /// no caps on the fill's book path — ⇒ the per-book gate is skipped, byte-identical to
    /// the pre-enforcement booking path.
    risk_book_limits: HashMap<String, RiskBookLimitDef>,
    /// The full risk-book tree (id, name, parent, enabled, limits …) in registry order —
    /// the authoritative set of books the live per-book risk stream aggregates over. Kept
    /// current beside [`Self::risk_book_limits`] by [`Self::set_risk_book_tree`] from BOTH
    /// reconcile sites (boot prime + the per-write auth reconcile hook), so the streamed
    /// roster tracks admin edits. Held ordered (not a map) so the streamed roster is in the
    /// same registry order the polled `ListRiskBookRisk` RPC returns. Empty ⇒ no books; the
    /// stream publishes an empty roster (never a fabricated one).
    risk_book_tree: Vec<RiskBookDef>,
}

impl Default for PositionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionStore {
    /// An empty store: no positions, an empty hierarchy, an empty limit tree —
    /// and the deny-by-default [`AccessMode::Enforce`] trust-boundary mode.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(StoreInner::default()),
            permissive_access: AtomicBool::new(false),
            consensus: OnceLock::new(),
            telemetry: OnceLock::new(),
            risk_version: AtomicU64::new(0),
            next_transfer_id: AtomicU64::new(0),
        }
    }

    /// Attach the shared latency/ops telemetry hub so a successful [`Self::book`]
    /// records its ack→fill→book commit latency (best-order timer O3). Idempotent-once;
    /// a store never given a hub simply records nothing.
    pub fn set_telemetry(&self, hub: Arc<crate::services::telemetry::TelemetryHub>) {
        let _ = self.telemetry.set(hub);
    }

    /// Attach the activated consistency tier (ADR-0015 §2.1). Called once at edge boot
    /// (`Edge::start_on_with_topology`) after Raft is booted, when at least one book is
    /// configured `Strong`. Idempotent-once (a second call is ignored). A store never
    /// given a handle — the pure-`Local` default — is byte-identical to today.
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

    /// The entitlements trust-boundary [`AccessMode`] guarding this book — read
    /// per request by every entitlement-gated RPC (a single relaxed atomic
    /// load; the decision itself runs on the async edge, never the pinned
    /// pricing core).
    #[must_use]
    pub fn access_mode(&self) -> AccessMode {
        if self.permissive_access.load(Ordering::Relaxed) {
            AccessMode::Permissive
        } else {
            AccessMode::Enforce
        }
    }

    /// Set the entitlements trust-boundary [`AccessMode`]. The construction
    /// default is [`AccessMode::Enforce`] (deny-by-default); only an explicit
    /// dev affordance — the demo edge, with a loud startup banner, or a test —
    /// ever flips this to [`AccessMode::Permissive`]. The production boot
    /// (`src/main.rs` / `Edge::start*`) never calls this.
    pub fn set_access_mode(&self, mode: AccessMode) {
        self.permissive_access
            .store(mode.is_permissive(), Ordering::Relaxed);
    }

    /// A fresh store that **inherits this store's firm configuration** — the org
    /// [`Hierarchy`] parent pointers, the name interner, and the [`LimitTree`] — but
    /// holds **no facts**. Used by the distributed risk federation to stage a gathered
    /// union of positions against the firm-consistent hierarchy + limit policy so a
    /// re-derivation / limit check over the union resolves org groups and limit scopes
    /// exactly as the single-node oracle would (the interner is copied so the same org
    /// names resolve to the same handles, and the staged facts arrive with explicit
    /// firm-consistent handles from the wire `OrgKey`).
    #[must_use]
    pub fn fork_config(&self) -> Self {
        let g = self.inner.read().expect("position store lock poisoned");
        Self {
            inner: RwLock::new(StoreInner {
                facts: Vec::new(),
                hierarchy: g.hierarchy.clone(),
                interner: g.interner.clone(),
                attribution: HashMap::new(),
                wire_ids: HashMap::new(),
                limits: g.limits.clone(),
                // A staged federation-union view re-derives risk over a gathered union;
                // it never books new fills, so it carries no routing config and no
                // risk-book stamps (both default-empty — never inherited).
                routing: None,
                risk_book: HashMap::new(),
                // A staged federation-union view never books new fills, so it enforces no
                // per-book caps (default-empty — never inherited).
                risk_book_limits: HashMap::new(),
                // A staged federation-union view never streams per-book risk, so it carries
                // no risk-book tree (default-empty — never inherited).
                risk_book_tree: Vec::new(),
            }),
            // Carried for coherence; a staged store only ever backs the
            // post-boundary `*_impl` internals, which no longer re-authorize.
            permissive_access: AtomicBool::new(self.permissive_access.load(Ordering::Relaxed)),
            // A staged federation-union view is transient and never the must-order
            // writer, so it never replicates (ADR-0015 §4.3 single-writer discipline).
            consensus: OnceLock::new(),
            // A staged view books no fills, so it carries no latency telemetry hub.
            telemetry: OnceLock::new(),
            // Transient staging view: it books no fills and streams no risk, so its risk
            // version is inert (starts at 0, never polled by a stream).
            risk_version: AtomicU64::new(0),
            // A staged federation-union view books no transfer legs, so its transfer id
            // source is inert (starts at 0, never minted from).
            next_transfer_id: AtomicU64::new(0),
        }
    }

    /// Configure a `Book → Desk` parent pointer (admin / setup path). Both handles
    /// are caller-interned (see [`Self::intern`]).
    pub fn set_book_desk(&self, book: u32, desk: u32) {
        let mut g = self.inner.write().expect("position store lock poisoned");
        g.hierarchy.set_book_desk(CubeBookId(book), DeskId(desk));
    }

    /// Configure a `Location → Entity` parent pointer (admin / setup path).
    pub fn set_location_entity(&self, location: u32, entity: u32) {
        let mut g = self.inner.write().expect("position store lock poisoned");
        g.hierarchy
            .set_location_entity(LocationId(location), EntityId(entity));
    }

    /// Intern an org name (book/trader/desk/…) to its stable `u32` handle. Used by
    /// the booking path and by tests to wire org placements by name.
    pub fn intern(&self, name: &str) -> u32 {
        let mut g = self.inner.write().expect("position store lock poisoned");
        g.interner.intern(name)
    }

    /// **The desk-identity bridge** (item B §3): give an identity desk slug a
    /// canonical numeric [`DeskId`] and declare which books it owns, populating the
    /// `Book → Desk` hierarchy at boot.
    ///
    /// Interns `desk_slug` to its stable handle (the canonical numeric desk id), and
    /// for each book name interns it to a [`CubeBookId`] and sets its parent desk —
    /// so a fact booked into one of those books resolves up to this desk exactly as
    /// the cube's roll-up and the entitlement filter do (`docs/RISK-HIERARCHY.md`
    /// §2.6). Interning is idempotent: the same slug/book names always resolve to the
    /// same handles within a run, so the live attribution path's lazy book interning
    /// (`book_from_attribution`) and this boot-time configuration agree on the same
    /// numeric ids. Called once per [`config::identity::DeskDef`](crate::config::identity::DeskDef)
    /// at edge boot; the empty-`books` desk is a no-op (it owns no books yet).
    pub fn configure_desk(&self, desk_slug: &str, books: &[String]) {
        let desk_handle = self.intern(desk_slug);
        for book in books {
            let book_handle = self.intern(book);
            self.set_book_desk(book_handle, desk_handle);
        }
    }

    /// Configure a limit at a hierarchy scope (admin / setup path).
    pub fn set_limit(&self, scope: celnet_limits::LimitScope, spec: celnet_limits::LimitSpec) {
        let mut g = self.inner.write().expect("position store lock poisoned");
        g.limits.set(scope, spec);
    }

    /// Install (or clear) the live firm-wide **risk-routing graph** (§4). Called at boot
    /// from the persisted [`IdentityStore::risk_routing_graph`](crate::config::identity::IdentityStore::risk_routing_graph)
    /// and re-pushed after every admin edit via the auth reconcile hook
    /// (`AuthEdge::reconcile_risk_routing`), so defining/editing/clearing the graph takes
    /// effect on subsequent fills immediately.
    ///
    /// `Some(graph)` ⇒ each subsequent fill through [`Self::book_from_attribution`] is
    /// routed to a risk book and stamped; `None` ⇒ routing is off and fills book exactly
    /// as before (no risk-book stamp) — the backward-compatible default. The graph is
    /// expected to already be [`validated`](RiskRoutingGraph::validate) against the
    /// current risk-book registry by the config layer; the router itself never panics on
    /// an unvalidated graph (it falls back to unrouted — see [`Self::book_from_attribution`]).
    ///
    /// Thread-safe: takes the store write lock briefly (control-plane cadence, never the
    /// pinned pricing core) and swaps an [`Arc`], so a concurrent booking sees either the
    /// old or the new graph atomically, never a torn one.
    pub fn set_routing(&self, graph: Option<RiskRoutingGraph>) {
        let mut g = self.inner.write().expect("position store lock poisoned");
        g.routing = graph.map(Arc::new);
    }

    /// Push the current **risk-book limit view** into the store — each book's parent (for
    /// subtree roll-up) and optional hard [`RiskLimits`] notional caps. Kept current beside
    /// [`Self::set_routing`] by BOTH reconcile sites (the boot-time prime in
    /// `celnet-server/src/lib.rs` and the per-write `AuthEdge::reconcile_risk_routing` hook),
    /// so a book create / update / delete or a graph edit refreshes the caps the per-book
    /// booking gate ([`Self::book`]) enforces. An empty view (no books, or none carrying
    /// limits) ⇒ the per-book gate is skipped and booking is byte-identical to the
    /// pre-enforcement path.
    ///
    /// Thread-safe: takes the store write lock briefly (control-plane cadence, never the
    /// pinned pricing core).
    pub fn set_risk_books(&self, books: Vec<RiskBookLimitDef>) {
        let mut g = self.inner.write().expect("position store lock poisoned");
        g.risk_book_limits = books.into_iter().map(|b| (b.id.clone(), b)).collect();
    }

    /// Push the current **risk-book tree** (the full [`RiskBookDef`] set, in registry
    /// order) into the store — the authoritative set of books the live per-book risk
    /// stream aggregates over. Called at the SAME two reconcile sites [`Self::set_risk_books`]
    /// is (the boot-time prime in `celnet-server/src/lib.rs` and the per-write
    /// `AuthEdge::reconcile_risk_routing` hook), so an admin create/update/delete/enable of a
    /// book re-publishes the streamed roster. Advances the monotonic risk version so a live
    /// subscriber re-aggregates on its next tick — a book-tree edit is a risk-presentation
    /// change even with no new fill.
    ///
    /// Thread-safe: takes the store write lock briefly (control-plane cadence, never the
    /// pinned pricing core).
    pub fn set_risk_book_tree(&self, books: Vec<RiskBookDef>) {
        let mut g = self.inner.write().expect("position store lock poisoned");
        g.risk_book_tree = books;
        drop(g);
        self.risk_version.fetch_add(1, Ordering::Relaxed);
    }

    /// The current risk-book tree (the full [`RiskBookDef`] set, in registry order) — the
    /// book set the live per-book risk stream rolls up over the live facts. Cloned out under
    /// the read lock so the aggregation runs lock-free.
    #[must_use]
    pub fn risk_book_tree(&self) -> Vec<RiskBookDef> {
        self.inner
            .read()
            .expect("position store lock poisoned")
            .risk_book_tree
            .clone()
    }

    /// The current **risk version**: a monotonic counter advanced on every change that can
    /// alter a risk book's aggregated risk — a successful routed [`Self::book`] and an admin
    /// edit of the risk-book tree ([`Self::set_risk_book_tree`]). The live per-book risk
    /// stream polls this (a lock-free relaxed load) to decide when to re-aggregate and
    /// re-publish, exactly as the FX/rates lines poll their fan-out and the aggregated-book
    /// line polls the hub's composite version.
    #[must_use]
    pub fn risk_version(&self) -> u64 {
        self.risk_version.load(Ordering::Relaxed)
    }

    /// Whether a risk-routing graph is currently installed (routing is active).
    #[must_use]
    pub fn has_routing(&self) -> bool {
        self.inner
            .read()
            .expect("position store lock poisoned")
            .routing
            .is_some()
    }

    /// The resolved `risk_book_id` a booked position was routed into at book time, or
    /// `None` if the position was booked unrouted (no graph configured, or a routing
    /// error fell it back to the unrouted path). Looked up by the wire (business) `u64`
    /// position id.
    #[must_use]
    pub fn risk_book_of(&self, position_id: u64) -> Option<String> {
        let handle = u32::try_from(position_id).ok()?;
        let g = self.inner.read().expect("position store lock poisoned");
        g.risk_book.get(&handle).cloned()
    }

    /// All live position facts currently bucketed into `risk_book_id` (the new by-risk-book
    /// query path §8.3). Returns the full [`RiskFact`]s so a later aggregation phase can
    /// roll greeks/notional up the book tree; the existing attribution keying is untouched,
    /// so a position appears in **both** its attribution roll-up and its risk book.
    #[must_use]
    pub fn positions_in_risk_book(&self, risk_book_id: &str) -> Vec<RiskFact> {
        let g = self.inner.read().expect("position store lock poisoned");
        g.facts
            .iter()
            .filter(|f| {
                g.risk_book
                    .get(&f.position_id.0)
                    .is_some_and(|b| b == risk_book_id)
            })
            .cloned()
            .collect()
    }

    /// A read view of one booked FX position for a **risk transfer** (§6): its current
    /// risk-book stamp, signed base notional, per-unit mark, canonical greeks, and the
    /// reconstructed [`BookedPosition`] (so an economic transfer can re-book offsetting /
    /// opening legs from the exact instrument/inputs). Returns `None` for an id that is
    /// not booked, overflows the `u32` handle space, or is not an FX position (a non-FX
    /// carry cannot be lifted back into FX [`VanillaInputs`]).
    ///
    /// **Mark.** The per-unit "current mark" is the position's marked option premium (its
    /// PV) in the quote/numeraire currency per unit of base notional —
    /// `leaf.premium_quote / notional_base` — the fair value the store already carries at
    /// the fact's `surface_version` (never re-priced here, off the hot core). This is the
    /// economically correct cost basis the transfer P&L crystallises against: a
    /// `Mid`/`MarkToMarket` transfer resolves the transfer price to this same mark ⇒ zero
    /// realised P&L (a fair internal cross), while an `Agreed` override crosses
    /// `moved · (agreed − mark)`. A zero-notional line marks at `0.0` (no per-unit basis).
    #[must_use]
    pub fn fx_transfer_view(&self, position_id: u64) -> Option<FxTransferView> {
        let handle = u32::try_from(position_id).ok()?;
        let g = self.inner.read().expect("position store lock poisoned");
        let fact = g.facts.iter().find(|f| f.position_id.0 == handle)?;
        let booked = booked_from_fact(fact, position_id)?;
        let notional = fact.measure.position.notional_base;
        let greeks = &fact.measure.leaf.greeks;
        let mark = if notional != 0.0 {
            fact.measure.leaf.premium_quote / notional
        } else {
            0.0
        };
        Some(FxTransferView {
            risk_book: g.risk_book.get(&handle).cloned(),
            signed_notional: notional,
            mark,
            // Canonical, notional-scaled greeks straight off the marked leaf (the same
            // per-position sensitivities the risk cube aggregates). DV01 is absent for FX
            // vanilla (mirrors the per-book gate skipping `max_dv01`) — the transfer's
            // rates path carries DV01 instead.
            delta: greeks.delta_base,
            gamma: greeks.gamma,
            vega: greeks.vega,
            theta: greeks.theta,
            booked,
        })
    }

    /// Mint a fresh position id for a **risk-transfer leg** (§6) — the moved slice of a
    /// partial re-attribution split or an economic transfer's offsetting / opening leg. The
    /// id is strictly above both the current max live handle and every previously-minted
    /// transfer id, so it never collides with a booked position (the FX book does not
    /// otherwise assign ids — client fills carry their own). Monotonic and lock-free-ish
    /// (one read snapshot for the max handle + a CAS on the counter).
    #[must_use]
    pub fn mint_position_id(&self) -> u64 {
        let max_handle = {
            let g = self.inner.read().expect("position store lock poisoned");
            g.facts.iter().map(|f| f.position_id.0).max().unwrap_or(0)
        };
        let base = u64::from(max_handle) + 1;
        let mut cur = self.next_transfer_id.load(Ordering::Relaxed);
        loop {
            let next = cur.max(base);
            match self.next_transfer_id.compare_exchange(
                cur,
                next + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return next,
                Err(observed) => cur = observed,
            }
        }
    }

    /// Re-point an already-booked FX position's **risk-book stamp** to `target_book` — a
    /// pure re-attribution (§6.1): no new fact, no re-pricing, economics unchanged. The
    /// caller runs the target's hard-limit headroom check ([`Self::check_risk_book_headroom`])
    /// BEFORE calling this, so the move only re-labels the routing dimension. Advances the
    /// risk version (the per-book stream re-aggregates on its next tick).
    ///
    /// # Errors
    /// `invalid_argument` if the id overflows the `u32` handle space; `not_found` if no
    /// position is booked under it.
    pub fn restamp_risk_book(
        &self,
        position_id: u64,
        target_book: &str,
    ) -> Result<(), tonic::Status> {
        let handle = u32::try_from(position_id).map_err(|_| {
            tonic::Status::invalid_argument(format!(
                "position_id {position_id} exceeds the u32 cube handle space"
            ))
        })?;
        let mut g = self.inner.write().expect("position store lock poisoned");
        if !g.facts.iter().any(|f| f.position_id.0 == handle) {
            return Err(tonic::Status::not_found(format!(
                "position {position_id} is not booked"
            )));
        }
        g.risk_book.insert(handle, target_book.to_owned());
        drop(g);
        self.risk_version.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Split an FX position for a **partial re-attribution** (§6.1 step 4): reduce the
    /// source fact to `remainder_notional` (same instrument/inputs/marks, keeps its
    /// original id and current book) and insert a NEW fact at `moved_position_id` carrying
    /// `moved_notional`, stamped into `target_book`. Both slices are canonicalised through
    /// the SAME `canonical_vanilla_fact` path a booking uses, so their greeks/marks are
    /// re-derived identically — and because greeks are linear in notional, the two slices'
    /// risk sums back to the original (no risk created or destroyed). Advances the risk
    /// version.
    ///
    /// # Errors
    /// `invalid_argument` if either id overflows `u32` or the position is not priceable;
    /// `not_found` if `source_position_id` is not booked.
    pub fn split_position(
        &self,
        source_position_id: u64,
        remainder_notional: f64,
        moved_position_id: u64,
        moved_notional: f64,
        target_book: &str,
    ) -> Result<(), tonic::Status> {
        let source_handle = u32::try_from(source_position_id).map_err(|_| {
            tonic::Status::invalid_argument(format!(
                "position_id {source_position_id} exceeds the u32 cube handle space"
            ))
        })?;
        // Reconstruct the source instrument, then derive the two split slices off it.
        let template = {
            let g = self.inner.read().expect("position store lock poisoned");
            let fact = g
                .facts
                .iter()
                .find(|f| f.position_id.0 == source_handle)
                .ok_or_else(|| {
                    tonic::Status::not_found(format!("position {source_position_id} is not booked"))
                })?;
            booked_from_fact(fact, source_position_id).ok_or_else(|| {
                tonic::Status::failed_precondition(format!(
                    "position {source_position_id} is not an FX position and cannot be split"
                ))
            })?
        };
        let remainder = BookedPosition {
            notional_base: remainder_notional,
            ..template
        };
        let moved = BookedPosition {
            position_id: moved_position_id,
            notional_base: moved_notional,
            ..template
        };
        // Canonicalise both off-lock (pure); the key is resolved from the source fact's
        // existing org placement so the remainder keeps the same attribution chain.
        let (rem_position, _rem_leaf, rem_fact);
        let (mv_position, _mv_leaf, mv_fact);
        let (source_key, source_attr) = {
            let g = self.inner.read().expect("position store lock poisoned");
            let fact = g
                .facts
                .iter()
                .find(|f| f.position_id.0 == source_handle)
                .ok_or_else(|| {
                    tonic::Status::not_found(format!("position {source_position_id} is not booked"))
                })?;
            (fact.key.clone(), g.attribution.get(&source_handle).cloned())
        };
        let moved_handle = u32::try_from(moved_position_id).map_err(|_| {
            tonic::Status::invalid_argument(format!(
                "position_id {moved_position_id} exceeds the u32 cube handle space"
            ))
        })?;
        (rem_position, _rem_leaf, rem_fact) =
            canonical_vanilla_fact(&remainder, source_key.clone())?;
        (mv_position, _mv_leaf, mv_fact) = canonical_vanilla_fact(&moved, source_key)?;
        let _ = (&rem_position, &mv_position); // canonicalisation validates priceability.
        let mut g = self.inner.write().expect("position store lock poisoned");
        // Replace the source fact in place with the reduced remainder (same handle, same
        // book stamp, same attribution — unchanged organisationally).
        if let Some(slot) = g
            .facts
            .iter_mut()
            .find(|f| f.position_id.0 == source_handle)
        {
            *slot = rem_fact;
        } else {
            return Err(tonic::Status::not_found(format!(
                "position {source_position_id} is not booked"
            )));
        }
        // Insert the moved slice as a fresh fact stamped into the target book.
        g.facts.push(mv_fact);
        g.wire_ids.insert(moved_handle, moved_position_id);
        g.risk_book.insert(moved_handle, target_book.to_owned());
        if let Some(attr) = source_attr {
            g.attribution.insert(moved_handle, attr);
        }
        drop(g);
        self.risk_version.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// The hard-limit **headroom check** for a book receiving an incoming `(net, gross)`
    /// notional batch (§6): projects the resolved book AND each ancestor exactly as the
    /// routed-fill gate ([`project_risk_book_breach`]) does, excluding every moved source
    /// handle so a within-book move never double-counts a line against itself. Returns
    /// `Ok(())` when no cap on the path is exceeded (or none is configured — byte-identical
    /// to the pre-enforcement path), and the typed `failed_precondition` breach otherwise.
    /// A re-attribution calls this over a read snapshot BEFORE any mutation so a move that
    /// would blow the target's (or an ancestor's) cap is refused with the store unmutated.
    ///
    /// # Errors
    /// `failed_precondition` if the incoming batch would EXCEED a hard net/gross notional
    /// cap on `target_book` or one of its ancestors.
    pub fn check_risk_book_headroom(
        &self,
        target_book: &str,
        incoming_net: f64,
        incoming_gross: f64,
        exclude_position_ids: &[u64],
    ) -> Result<(), tonic::Status> {
        let excl: Vec<u32> = exclude_position_ids
            .iter()
            .filter_map(|&id| u32::try_from(id).ok())
            .collect();
        let g = self.inner.read().expect("position store lock poisoned");
        match project_risk_book_breach_multi(&g, target_book, incoming_net, incoming_gross, &excl) {
            Some(status) => Err(status),
            None => Ok(()),
        }
    }

    /// Book an FX position into an **explicit** risk book (an economic-transfer leg, §6.2)
    /// rather than the routed one. This is the transfer's booking primitive: the routed
    /// sink ([`Self::book`]) stamps whatever the routing graph resolves, but a transfer must
    /// land its offsetting / opening legs in NAMED books (source / target). It runs the
    /// SAME gates the routed sink runs — the FactKey Greeks/notional pre-trade gate and the
    /// per-book hard notional-cap gate over `risk_book` + its ancestors — over a read
    /// snapshot BEFORE any mutation, refuses a hard breach with the store unmutated, and
    /// (for a `Strong`-tier book) routes the authoritative write through the quorum log
    /// before the local apply, exactly as [`Self::book`] does. Never bypasses limits or the
    /// consensus path. The org FactKey is resolved from `attribution` the way
    /// [`Self::book_from_attribution`] resolves it. Advances the risk version.
    ///
    /// # Errors
    /// `invalid_argument` if the id overflows `u32` or the position is not priceable;
    /// `failed_precondition` on a hard FactKey-limit or per-book notional-cap breach.
    pub fn book_into_risk_book(
        &self,
        booked: BookedPosition,
        attribution: &AttributionRecord,
        risk_book: &str,
    ) -> Result<(), tonic::Status> {
        // Resolve the org placement from the attribution holder seat, mirroring
        // `book_from_attribution` exactly (same interning, same DEFAULT-LOCATION unit).
        let holder = attribution
            .held_by
            .as_ref()
            .or(attribution.quoted_by.as_ref());
        let (book_name, trader_name) = holder
            .map(|b| (b.book.as_str().to_owned(), seat_name(b)))
            .unwrap_or_else(|| ("UNATTRIBUTED".to_owned(), "unattributed".to_owned()));
        let (book_h, trader_h, location_h) = {
            let mut g = self.inner.write().expect("position store lock poisoned");
            let book_h = g.interner.intern(&book_name);
            let trader_h = g.interner.intern(&trader_name);
            let location_h = g.interner.intern("DEFAULT-LOCATION");
            (book_h, trader_h, location_h)
        };
        let key = FactKey {
            trader: TraderId(trader_h),
            book: CubeBookId(book_h),
            desk: DeskId(0),
            underlying: celnet_types::Underlying::Fx(booked.pair),
            location: LocationId(location_h),
            entity: EntityId(0),
        };
        let (position, leaf, fact) = canonical_vanilla_fact(&booked, key.clone())?;
        let handle = fact.position_id.0;
        // Both gates on a single READ snapshot, BEFORE the write lock (a hard breach never
        // books; the O(#facts) projections never run under the exclusive lock).
        {
            let g = self.inner.read().expect("position store lock poisoned");
            if !g.limits.is_empty() {
                let result = project_pre_trade(
                    &g.facts,
                    &g.limits,
                    &g.hierarchy,
                    &position,
                    &leaf,
                    key,
                    handle,
                );
                if result.decision == PreTradeDecision::Reject {
                    return Err(limit_breached_status(&result));
                }
            }
            if let Some(breach) =
                project_risk_book_breach(&g, risk_book, booked.notional_base, handle)
            {
                return Err(breach);
            }
        }
        // A `Strong`-tier book routes its authoritative write through the quorum log BEFORE
        // the local apply — keyed on the org book name, exactly as `book` does (a `Local`
        // book, the default, skips this and stays byte-identical).
        if let Some(consensus) = self.consensus.get()
            && consensus.level_for_book(&book_name).is_strong()
        {
            consensus.commit_book_write(fx_book_key(booked.position_id), booked.notional_base)?;
        }
        let mut g = self.inner.write().expect("position store lock poisoned");
        if let Some(slot) = g
            .facts
            .iter_mut()
            .find(|f| f.position_id == fact.position_id)
        {
            *slot = fact;
        } else {
            g.facts.push(fact);
        }
        g.wire_ids.insert(handle, booked.position_id);
        g.risk_book.insert(handle, risk_book.to_owned());
        g.attribution.insert(handle, attribution.clone());
        drop(g);
        self.risk_version.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Remove a booked FX position entirely — its fact, risk-book stamp, wire id, and
    /// attribution. The economic-transfer **rollback** primitive (§6.2): a staged
    /// source-offset leg is un-booked if the paired target leg is refused, so a rejected
    /// transfer never leaves a half-transfer. Advances the risk version when a position was
    /// actually removed; a no-op (no bump) for an unknown id.
    pub fn remove_position(&self, position_id: u64) {
        let Ok(handle) = u32::try_from(position_id) else {
            return;
        };
        let removed = {
            let mut g = self.inner.write().expect("position store lock poisoned");
            let before = g.facts.len();
            g.facts.retain(|f| f.position_id.0 != handle);
            let removed = g.facts.len() != before;
            g.risk_book.remove(&handle);
            g.wire_ids.remove(&handle);
            g.attribution.remove(&handle);
            removed
        };
        if removed {
            self.risk_version.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a booked position into the live book under an explicit
    /// [`FactKey`] org placement (the already-interned handles) and an optional
    /// attribution chain reported back by `ListPositions`. The canonical leaf is
    /// re-derived here via `celnet-risk-normalize::canonicalize`, so the store
    /// holds a convention-free fact the cube sums directly.
    ///
    /// One current fact per `position_id` (a re-book supersedes the prior fact —
    /// the cube's `upsert` semantics), so a re-valued position never double-counts.
    ///
    /// The wire id is a `u64`; the cube interns a position under a `u32` handle, so
    /// the id must fit `u32` (a desk book is far under 4 billion live lines). An id
    /// over `u32::MAX` is rejected rather than silently truncated.
    ///
    /// # Errors
    /// `invalid_argument` if `booked.position_id` does not fit a `u32` cube handle.
    pub fn upsert(
        &self,
        booked: BookedPosition,
        key: FactKey,
        attribution: Option<AttributionRecord>,
    ) -> Result<(), tonic::Status> {
        let handle = u32::try_from(booked.position_id).map_err(|_| {
            tonic::Status::invalid_argument(format!(
                "position_id {} exceeds the u32 cube handle space",
                booked.position_id
            ))
        })?;
        let position = PositionRisk::fx(
            booked.pair,
            booked.option,
            booked.notional_base,
            booked.inputs,
            booked.quoted_delta,
            booked.premium_style,
        );
        let leaf = canonicalize(&position).map_err(|e| {
            tonic::Status::invalid_argument(format!("booked position is not priceable: {e}"))
        })?;
        let fact = RiskFact {
            position_id: PositionId(handle),
            key,
            measure: FactMeasure {
                leaf,
                position,
                // The live RFS / click-to-trade book records vanilla legs here; a
                // booked exotic is recorded via `upsert_exotic` (its `exotic` field
                // is `Some`). This vanilla path leaves it `None`.
                exotic: None,
            },
            surface_version: booked.surface_version,
        };
        let mut g = self.inner.write().expect("position store lock poisoned");
        if let Some(slot) = g
            .facts
            .iter_mut()
            .find(|f| f.position_id == fact.position_id)
        {
            *slot = fact;
        } else {
            g.facts.push(fact);
        }
        g.wire_ids.insert(handle, booked.position_id);
        match attribution {
            Some(a) => {
                g.attribution.insert(handle, a);
            }
            None => {
                g.attribution.remove(&handle);
            }
        }
        Ok(())
    }

    /// Record a booked **exotic** position into the live book under an explicit
    /// [`FactKey`] org placement and an optional attribution chain. The exotic's REAL
    /// Greeks (additive leaf) and its [`ExoticLeg`](celnet_risk_cube::ExoticLeg)
    /// re-derivation source (non-additive) are produced here, so the booked exotic
    /// rolls up into firm/desk/book risk exactly like a vanilla leg — never silently
    /// excluded.
    ///
    /// The fact's `position` carries the exotic's honest underlying-vanilla bucketing
    /// metadata (same pair/option/notional/inputs) so the vega ladder has a
    /// `(tenor × delta)` to map onto; the leaf and all re-pricing use the exotic.
    ///
    /// One current fact per `position_id` (a re-book supersedes the prior fact).
    ///
    /// # Errors
    /// `invalid_argument` if `position_id` does not fit a `u32` cube handle.
    pub fn upsert_exotic(
        &self,
        booked: BookedExotic,
        key: FactKey,
        attribution: Option<AttributionRecord>,
    ) -> Result<(), tonic::Status> {
        let handle = u32::try_from(booked.position_id).map_err(|_| {
            tonic::Status::invalid_argument(format!(
                "position_id {} exceeds the u32 cube handle space",
                booked.position_id
            ))
        })?;
        let leg = booked.leg;
        // The underlying-vanilla metadata for vega-pillar bucketing (never priced for
        // an exotic fact — the cube routes exotic facts through the exotic pricer).
        let position = PositionRisk::fx(
            leg.pair,
            booked.option,
            leg.notional,
            leg.inputs,
            booked.quoted_delta,
            booked.premium_style,
        );
        let fact = RiskFact {
            position_id: PositionId(handle),
            key,
            measure: FactMeasure {
                // The REAL exotic Greek set (additive roll-up carrier), priced
                // through the injected `celnet-exotics` seam.
                leaf: leg.canonical_leaf(&super::exotic_pricer::ExoticEngine),
                position,
                exotic: Some(leg),
            },
            surface_version: booked.surface_version,
        };
        let mut g = self.inner.write().expect("position store lock poisoned");
        if let Some(slot) = g
            .facts
            .iter_mut()
            .find(|f| f.position_id == fact.position_id)
        {
            *slot = fact;
        } else {
            g.facts.push(fact);
        }
        g.wire_ids.insert(handle, booked.position_id);
        match attribution {
            Some(a) => {
                g.attribution.insert(handle, a);
            }
            None => {
                g.attribution.remove(&handle);
            }
        }
        Ok(())
    }

    /// Record a booked position whose org placement is resolved **from its
    /// attribution chain** — the live click-to-trade path. The holder seat
    /// (`held_by`, or `quoted_by` when the auto-pricer warehouses the line) names
    /// the book and trader; the book/owner strings are interned to handles. The
    /// currency pair drives the `ccy_pair` axis. `location`/`entity` default to a
    /// single configured booking location (handle `1`) unless the hierarchy maps it
    /// onward — honest: the live RFS line carries no location/entity, so they are
    /// the default booking unit until an admin path sets them, never faked per-line.
    ///
    /// # Errors
    /// `invalid_argument` if `booked.position_id` does not fit a `u32` cube handle;
    /// `failed_precondition` if a **hard** pre-trade limit would be breached (the
    /// booking is refused and store state is left unmutated — see [`Self::book`]).
    pub fn book_from_attribution(
        &self,
        booked: BookedPosition,
        attribution: &AttributionRecord,
    ) -> Result<PreTradeDecision, tonic::Status> {
        let holder = attribution
            .held_by
            .as_ref()
            .or(attribution.quoted_by.as_ref());
        let (book_name, trader_name) = holder
            .map(|b| (b.book.as_str().to_owned(), seat_name(b)))
            .unwrap_or_else(|| ("UNATTRIBUTED".to_owned(), "unattributed".to_owned()));

        let (book_h, trader_h, location_h) = {
            let mut g = self.inner.write().expect("position store lock poisoned");
            let book_h = g.interner.intern(&book_name);
            let trader_h = g.interner.intern(&trader_name);
            // A single default booking location until an admin path configures
            // per-line locations (the live RFS line carries none).
            let location_h = g.interner.intern("DEFAULT-LOCATION");
            (book_h, trader_h, location_h)
        };

        let key = FactKey {
            trader: TraderId(trader_h),
            book: CubeBookId(book_h),
            // Desk resolves from the book's parent pointer (0 ⇒ resolve).
            desk: DeskId(0),
            underlying: celnet_types::Underlying::Fx(booked.pair),
            location: LocationId(location_h),
            // Entity resolves from the location's parent pointer (0 ⇒ resolve).
            entity: EntityId(0),
        };
        self.book(booked, key, Some(attribution.clone()))
    }

    /// **The pre-trade limit gate + booking sink** (ADR-0016 A1): the single
    /// convergence point every FX booking front-end funnels through
    /// ([`crate::services::stream`]'s click-to-trade fill via
    /// [`Self::book_from_attribution`], and the check-only consultations from
    /// `QuoteService::AcceptQuote` / the FIX acceptor via [`Self::evaluate_pre_trade`]).
    ///
    /// Runs [`pre_trade_check`](celnet_limits::pre_trade_check) over the **projected**
    /// book (the current facts at every scope on the position's roll-up path *plus*
    /// this trade's incremental canonical leaf) **before** any state mutation:
    ///
    /// * a [`PreTradeDecision::Reject`] (a **hard** limit would be breached) refuses
    ///   the booking with a typed `failed_precondition` `LimitBreached` status and
    ///   leaves the store **unmutated** — the load-bearing risk-control invariant;
    /// * [`PreTradeDecision::Warn`] (a **soft** limit) and [`PreTradeDecision::Accept`]
    ///   both book, the `Warn` carrying the amber/red RAG for the audit trail.
    ///
    /// The projection runs over a **read** snapshot and the write lock is then taken
    /// only for the O(1) mutation — the exclusive lock is never held across the
    /// O(#facts) cube build, so one fill never serialises every other booking behind
    /// its own limit projection (guardrails #6/#11: the sink scales to an IB-sized hot
    /// book). The narrow window between the snapshot check and the write is the standard
    /// pre-trade TOCTOU: a hard breach still rejects **before** any mutation (the
    /// load-bearing invariant holds), and two fills that individually clear a near-full
    /// hard cap but jointly cross it are caught by the post-trade limit monitor
    /// ([`super::RiskEdge`]'s `LimitStatus` RAG), exactly as production pre-trade gates
    /// resolve throughput vs. strict serialisation.
    ///
    /// # Errors
    /// `invalid_argument` if `booked.position_id` does not fit a `u32` cube handle or
    /// the position is not priceable; `failed_precondition` on a hard FactKey-limit breach
    /// OR a routed **risk-book** hard notional-cap breach (the resolved book or an ancestor).
    pub fn book(
        &self,
        booked: BookedPosition,
        key: FactKey,
        attribution: Option<AttributionRecord>,
    ) -> Result<PreTradeDecision, tonic::Status> {
        // Best-order timer O3 (ack→fill→book commit latency): bracket the whole
        // booking commit with a monotonic `Instant`; recorded into the telemetry hub
        // on a successful book below. This is the async booking tier (guardrail 11).
        let book_t0 = std::time::Instant::now();
        // Canonicalize off-lock (pure, convention-free). The key is cloned into the fact;
        // the original drives the pre-trade scope resolution below.
        let (position, leaf, fact) = canonical_vanilla_fact(&booked, key.clone())?;
        let handle = fact.position_id.0;

        // Pre-trade gates over a single READ snapshot, BEFORE acquiring the write lock, so a
        // hard breach can never book AND the O(#facts) projections never run while the
        // exclusive write lock is held (guardrails #6/#11 — a fill never serialises every
        // other booking behind its own projection). Two gates run here, both on the SAME
        // snapshot and both leaving the store unmutated on a reject:
        //   1. the FactKey Greeks/notional pre-trade gate (`celnet_limits`), unchanged; then
        //   2. risk routing is resolved (`RiskRouter::route`) and, if the resolved book or an
        //      ancestor carries a hard `RiskLimits` notional cap, the per-book limit gate.
        // Both must pass to book; either rejects with a typed `failed_precondition` before any
        // mutation. A deployment with no FactKey limits AND no risk-book limits builds no
        // projection and is byte-identical to the pre-gate booking path.
        let (decision, resolved_book): (PreTradeDecision, Option<String>) = {
            let g = self.inner.read().expect("position store lock poisoned");
            // (1) The existing FactKey pre-trade gate — unchanged.
            let decision = if g.limits.is_empty() {
                PreTradeDecision::Accept
            } else {
                let result = project_pre_trade(
                    &g.facts,
                    &g.limits,
                    &g.hierarchy,
                    &position,
                    &leaf,
                    key,
                    handle,
                );
                if result.decision == PreTradeDecision::Reject {
                    return Err(limit_breached_status(&result));
                }
                result.decision
            };
            // (2a) Resolve risk routing on the SAME snapshot, BEFORE the mutation. A routing
            // error on an already-validated graph falls back to UNROUTED (a routing failure
            // never rejects the fill), byte-identical to the pre-routing path.
            let resolved = g.routing.as_ref().and_then(|graph| {
                let ctx = routing_context_from(&booked, attribution.as_ref());
                match RiskRouter::route(graph, &ctx) {
                    Ok(book) => Some(book.to_owned()),
                    Err(err) => {
                        tracing::warn!(
                            position_id = booked.position_id,
                            %err,
                            "risk routing failed; booking unrouted",
                        );
                        None
                    }
                }
            });
            // (2b) The per-book HARD-limit gate: if the fill's resolved book (or any ancestor)
            // would EXCEED a hard notional cap, reject BEFORE any mutation — store unchanged.
            if let Some(book_id) = resolved.as_deref()
                && let Some(breach) =
                    project_risk_book_breach(&g, book_id, booked.notional_base, handle)
            {
                return Err(breach);
            }
            (decision, resolved)
        };
        // ADR-0015 §2.1: a book configured `Strong` routes its authoritative write
        // through the Raft quorum log BEFORE the local apply — linearizable,
        // quorum-replicated, zero-data-loss. The level is resolved once, off the hot
        // path, from the book name the booking carries (its attribution). A `Local` book
        // (the default) skips this entirely, so the fast path below stays byte-identical.
        // The must-order state replicated is the position's signed economic size
        // (`notional_base`) — the derived mark is regenerable per §4.3 and deliberately
        // NOT quorum-logged. A quorum that cannot commit refuses the booking (the store
        // is left unmutated) rather than degrading a `Strong` book to un-replicated
        // durability. This runs on the async booking tier, never the pricing thread.
        if let Some(consensus) = self.consensus.get() {
            let book_name = attribution
                .as_ref()
                .and_then(|a| a.held_by.as_ref().or(a.quoted_by.as_ref()))
                .map(|b| b.book.as_str())
                .unwrap_or_default();
            if consensus.level_for_book(book_name).is_strong() {
                consensus
                    .commit_book_write(fx_book_key(booked.position_id), booked.notional_base)?;
            }
        }

        // Within limit (accept) or soft warn: take the write lock only for the mutation —
        // the same insert `upsert` does. The read guard above is already released.
        let mut g = self.inner.write().expect("position store lock poisoned");
        if let Some(slot) = g
            .facts
            .iter_mut()
            .find(|f| f.position_id == fact.position_id)
        {
            *slot = fact;
        } else {
            g.facts.push(fact);
        }
        g.wire_ids.insert(handle, booked.position_id);
        // Risk routing (§4): stamp the risk book resolved on the read snapshot above (before
        // the per-book limit gate), beside the attribution keying (§8.3). `None` — no graph,
        // or a routing error on an already-validated graph — leaves the position UNROUTED
        // (and a re-book of a formerly-routed id while routing is now off drops the stamp),
        // byte-identical to the pre-routing booking path.
        match &resolved_book {
            Some(book) => {
                g.risk_book.insert(handle, book.clone());
            }
            None => {
                g.risk_book.remove(&handle);
            }
        }
        match attribution {
            Some(a) => {
                g.attribution.insert(handle, a);
            }
            None => {
                g.attribution.remove(&handle);
            }
        }
        drop(g);
        // A routed fill changed the live book, so a risk book's aggregated risk may have
        // moved: advance the monotonic risk version the live per-book risk stream polls
        // (the risk analogue of the aggregation hub's per-book composite version bump).
        // A relaxed bump after the mutation is visible; a subscriber re-aggregates on the
        // next tick when it observes the newer version.
        self.risk_version.fetch_add(1, Ordering::Relaxed);
        // O3: record the ack→fill→book commit latency into the per-`OpKind` store.
        if let Some(hub) = self.telemetry.get() {
            hub.record_edge(
                celnet_observability::OpKind::Book,
                u64::try_from(book_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
        }
        Ok(decision)
    }

    /// **Check-only pre-trade limit gate** for a booking whose position artifact lives
    /// off the FX risk book (`QuoteService::AcceptQuote`'s quote-store execution, the
    /// FIX acceptor's `ExecutionReport` fill): consult the **same** limit tree +
    /// current-book aggregation [`Self::book`] uses, without recording a risk fact. A
    /// hard breach must refuse the front-end's own booking; a clean/soft path proceeds.
    ///
    /// These front-ends never recorded a risk fact (they book into the quote store /
    /// emit a FIX fill, not the FX warehouse), so there is no org attribution to honour:
    /// the check is placed at the org-unattributed scopes the position **does** carry —
    /// the currency pair and the firm apex ([`unattributed_fx_key`]) — never a
    /// fabricated desk/book/trader the booking did not record. The current book at those
    /// scopes is the shared warehouse's live aggregate (the lines the RFS click-to-trade
    /// path booked), so a firm/pair cap already at its limit rejects the accept.
    ///
    /// # Errors
    /// `invalid_argument` if the position is not priceable / the id overflows `u32`.
    pub fn evaluate_pre_trade(
        &self,
        booked: &BookedPosition,
    ) -> Result<PreTradeResult, tonic::Status> {
        let key = unattributed_fx_key(booked.pair);
        let g = self.inner.read().expect("position store lock poisoned");
        // No limits configured ⇒ nothing to gate: accept without canonicalizing, so a
        // deployment that has set no caps is byte-identical to the pre-gate booking path
        // (and the check adds no new failure mode on the accept/lift path).
        if g.limits.is_empty() {
            return Ok(PreTradeResult {
                decision: PreTradeDecision::Accept,
                checks: Vec::new(),
            });
        }
        let (position, leaf, fact) = canonical_vanilla_fact(booked, key.clone())?;
        let handle = fact.position_id.0;
        Ok(project_pre_trade(
            &g.facts,
            &g.limits,
            &g.hierarchy,
            &position,
            &leaf,
            key,
            handle,
        ))
    }

    /// **Check-only pre-trade limit gate for a class-parametric position** (ADR-0021
    /// uniform-asset-class): the cross-asset analogue of [`Self::evaluate_pre_trade`],
    /// for a booking whose risk artifact lives off the warehouse and is a normalized
    /// [`PositionRisk`] of **any** asset class — an FX/metal pair, or an
    /// equity/commodity/digital-asset (linear) cost-of-carry position. It consults the
    /// **same** limit tree + current-book aggregation the FX path does, without
    /// recording a risk fact.
    ///
    /// The position is placed at the org-unattributed scopes it actually carries — its
    /// `underlying` axis and the firm apex ([`unattributed_key`]) — never a fabricated
    /// desk/book/trader. The canonical leaf is re-derived through the position's own
    /// asset-class leaf ([`canonicalize`]), so a cross-asset delta charges the shared
    /// firm/per-underlying caps exactly as an FX delta does — the pre-trade gate sees
    /// the position's real class-correct exposure, never a silent zero (guardrail #2).
    /// The check-only handle is `0` (excluded from no prior fact — the store has none
    /// for this off-warehouse position), matching the FX check-only path.
    ///
    /// # Errors
    /// `invalid_argument` if the position is not priceable through its leaf.
    pub fn evaluate_pre_trade_position(
        &self,
        position: &PositionRisk,
    ) -> Result<PreTradeResult, tonic::Status> {
        let key = unattributed_key(position.underlying.clone());
        let g = self.inner.read().expect("position store lock poisoned");
        // No limits configured ⇒ nothing to gate: accept without canonicalizing, so a
        // deployment that has set no caps is byte-identical to the pre-gate booking path
        // (mirrors the FX `evaluate_pre_trade` short-circuit exactly).
        if g.limits.is_empty() {
            return Ok(PreTradeResult {
                decision: PreTradeDecision::Accept,
                checks: Vec::new(),
            });
        }
        let (position, leaf, fact) = canonical_fact(position, 0, 0, key.clone())?;
        let handle = fact.position_id.0;
        Ok(project_pre_trade(
            &g.facts,
            &g.limits,
            &g.hierarchy,
            &position,
            &leaf,
            key,
            handle,
        ))
    }

    /// A read-only snapshot of the store for one aggregation cycle: the current
    /// facts, the hierarchy, and the per-position attribution provenance. Taken
    /// under the read lock and returned owned so the aggregation runs lock-free.
    #[must_use]
    pub fn snapshot(&self) -> StoreSnapshot {
        let g = self.inner.read().expect("position store lock poisoned");
        StoreSnapshot {
            facts: g.facts.clone(),
            hierarchy: g.hierarchy.clone(),
            attribution: g.attribution.clone(),
            wire_ids: g.wire_ids.clone(),
            limits: g.limits.clone(),
            reverse: g.interner.reverse.clone(),
        }
    }

    /// The number of live positions (booked vanilla legs).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .expect("position store lock poisoned")
            .facts
            .len()
    }

    /// Whether the live book is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An owned, point-in-time view of the store used to run one aggregation cycle
/// lock-free off the hot path.
#[derive(Debug, Clone)]
pub struct StoreSnapshot {
    /// The current fact table.
    pub facts: Vec<RiskFact>,
    /// The org hierarchy parent pointers.
    pub hierarchy: Hierarchy,
    /// Per-position attribution provenance (keyed by the cube `u32` position handle).
    pub attribution: HashMap<u32, AttributionRecord>,
    /// The wire (business) `u64` id for each cube `u32` position handle.
    pub wire_ids: HashMap<u32, u64>,
    /// The configured limit tree.
    pub limits: LimitTree,
    /// The interner reverse map (handle `h` ↔ `reverse[h-1]`), so a fact's org
    /// handles can be reported back as names if needed.
    pub reverse: Vec<String>,
}

impl StoreSnapshot {
    /// The attribution chain recorded for a cube position handle, if any.
    #[must_use]
    pub fn attribution_of(&self, handle: u32) -> Option<&AttributionRecord> {
        self.attribution.get(&handle)
    }

    /// The wire (business) `u64` id for a cube position handle (falling back to the
    /// handle widened, which only happens for a handle with no recorded wire id).
    #[must_use]
    pub fn wire_id_of(&self, handle: u32) -> u64 {
        self.wire_ids
            .get(&handle)
            .copied()
            .unwrap_or(u64::from(handle))
    }
}

/// The seat name of a book id: the human trader id or the auto-pricer id, or a
/// stable placeholder when the seat is unset (so interning is total).
fn seat_name(book: &BookId) -> String {
    match book.owner.as_ref().and_then(|o: &Owner| o.seat.as_ref()) {
        Some(owner::Seat::Trader(t)) => t.clone(),
        Some(owner::Seat::AutoPricer(p)) => p.clone(),
        None => "unknown-seat".to_owned(),
    }
}

/// Map a booked FX-vanilla fill + its attribution onto the routing engine's
/// [`RoutingContext`] (§4) — the input the firm-wide decision graph matches against.
///
/// The economic fields (ccy/pair, product, side, notional, strike, tenor) come off the
/// [`BookedPosition`]; `user` off the attribution holder seat. `counterparty` and `desk`
/// are NOT carried on the live click-to-trade attribution record (a fill names only its
/// holder seat's book + trader), so they default to empty until the FI booking path
/// threads them — never faked. `price` is left `0.0`: the marked premium is a derived
/// quantity not needed to route on trade economics. Total (never panics).
fn routing_context_from(
    booked: &BookedPosition,
    attribution: Option<&AttributionRecord>,
) -> RoutingContext {
    let product = match booked.option {
        OptionType::Call => "call",
        OptionType::Put => "put",
    }
    .to_owned();
    let user = attribution
        .and_then(|a| a.held_by.as_ref().or(a.quoted_by.as_ref()))
        .map(seat_name)
        .unwrap_or_default();
    RoutingContext {
        instrument_id: booked.pair.to_string(),
        ccy: booked.pair.to_string(),
        product,
        side: if booked.notional_base >= 0.0 {
            "Buy"
        } else {
            "Sell"
        }
        .to_owned(),
        notional: booked.notional_base.abs(),
        tenor: booked.inputs.t,
        strike: booked.inputs.strike,
        counterparty: String::new(),
        user,
        desk: String::new(),
        price: 0.0,
    }
}

/// The org-**unattributed** [`FactKey`] for a booking that lives OFF the risk
/// warehouse — a `QuoteService::AcceptQuote` execution (kept in the quote store) or a
/// FIX acceptor `ExecutionReport` fill (emitted on the wire). Its trader / book / desk
/// / location / entity handles are all `0` (the "unknown / resolve-from-parent"
/// sentinel), so the only scopes [`ScopePath::resolve`] yields a *configurable* limit
/// at are the position's `underlying` axis and the firm apex (always appended). The
/// gate therefore enforces exactly the **firm-wide** and **per-underlying** caps that
/// apply regardless of desk attribution — it never invents a desk/book/trader the
/// accept never recorded (guardrail #2: derive from the position, never fabricate).
///
/// Class-parametric over the underlying (ADR-0021 uniform-asset-class): the same
/// unattributed key serves an FX pair, a metal, or a cross-asset (equity/commodity/
/// digital-asset) underlying — the cube's [`DimensionId::Underlying`] group value packs
/// each class, so a per-underlying cap covers any class without a schema change. FX is
/// [`unattributed_fx_key`].
fn unattributed_key(underlying: celnet_types::Underlying) -> FactKey {
    FactKey {
        trader: TraderId(0),
        book: CubeBookId(0),
        desk: DeskId(0),
        underlying,
        location: LocationId(0),
        entity: EntityId(0),
    }
}

/// The org-unattributed [`FactKey`] for an FX booking off the risk warehouse — the
/// FX-ergonomic form of [`unattributed_key`] (byte-identical to the former inline
/// key), used by the FX-vanilla [`PositionStore::evaluate_pre_trade`] path.
fn unattributed_fx_key(pair: CcyPair) -> FactKey {
    unattributed_key(celnet_types::Underlying::Fx(pair))
}

/// Canonicalize a booked vanilla line into its convention-free `(position, leaf,
/// fact)` — the pure, lock-free step shared by [`PositionStore::book`] and
/// [`PositionStore::evaluate_pre_trade`]. The leaf drives the pre-trade increment;
/// the fact is the store record inserted on a clean/soft decision.
/// Reconstruct a [`BookedPosition`] from a stored FX [`RiskFact`] — the inverse of the
/// `canonicalize` on the booking path — so a risk transfer can re-book offsetting / opening
/// legs or split slices off the exact instrument/inputs/conventions the position was booked
/// under. Returns `None` for a non-FX fact: only an FX two-rate carry
/// ([`Carry::FxRates`]) lifts back into [`VanillaInputs`]; a generalized cost-of-carry
/// position (equity/commodity/digital-asset) is not FX-transferable through this path.
fn booked_from_fact(fact: &RiskFact, wire_id: u64) -> Option<BookedPosition> {
    let p = &fact.measure.position;
    let pair = p.underlying.as_ccy_pair()?;
    let Carry::FxRates { r_dom, r_for } = p.inputs.carry else {
        return None;
    };
    let inputs = VanillaInputs::new(
        p.inputs.spot,
        p.inputs.strike,
        p.inputs.vol,
        p.inputs.t,
        r_dom,
        r_for,
    );
    Some(BookedPosition {
        position_id: wire_id,
        pair,
        option: p.option,
        notional_base: p.notional_base,
        inputs,
        // FX positions always carry both conventions; the defaults are unreachable for an
        // FX fact and exist only to keep the reconstruction total.
        quoted_delta: p.quoted_delta.unwrap_or(DeltaConvention::SpotUnadjusted),
        premium_style: p.premium_style.unwrap_or(PremiumStyle::DomesticPips),
        surface_version: fact.surface_version,
    })
}

fn canonical_vanilla_fact(
    booked: &BookedPosition,
    key: FactKey,
) -> Result<(PositionRisk, CanonicalLeaf, RiskFact), tonic::Status> {
    let position = PositionRisk::fx(
        booked.pair,
        booked.option,
        booked.notional_base,
        booked.inputs,
        booked.quoted_delta,
        booked.premium_style,
    );
    canonical_fact(&position, booked.position_id, booked.surface_version, key)
}

/// Canonicalize an already-built [`PositionRisk`] of **any** asset class into its
/// convention-free `(position, leaf, fact)` — the class-parametric core shared by
/// [`canonical_vanilla_fact`] (FX) and the cross-asset check-only pre-trade
/// ([`PositionStore::evaluate_pre_trade_position`]). The underlying discriminant lives
/// entirely inside the leaf pricer [`canonicalize`] dispatches to (ADR-0008); this
/// step is asset-class agnostic — an equity/commodity/digital-asset position produces
/// its own leaf greeks, never an FX proxy (guardrail #2).
fn canonical_fact(
    position: &PositionRisk,
    position_id: u64,
    surface_version: u64,
    key: FactKey,
) -> Result<(PositionRisk, CanonicalLeaf, RiskFact), tonic::Status> {
    let handle = u32::try_from(position_id).map_err(|_| {
        tonic::Status::invalid_argument(format!(
            "position_id {position_id} exceeds the u32 cube handle space"
        ))
    })?;
    let leaf = canonicalize(position).map_err(|e| {
        tonic::Status::invalid_argument(format!("booked position is not priceable: {e}"))
    })?;
    let fact = RiskFact {
        position_id: PositionId(handle),
        key,
        measure: FactMeasure {
            leaf: leaf.clone(),
            position: position.clone(),
            exotic: None,
        },
        surface_version,
    };
    Ok((position.clone(), leaf, fact))
}

/// Run the pre-trade check for a proposed vanilla booking against the **projected**
/// book: the current facts (excluding any prior fact under the same `exclude_handle`,
/// so a re-book projects `others + this trade` rather than double-counting) at every
/// scope on the position's roll-up path, plus this trade's incremental canonical leaf.
/// The current node aggregate at each scope is built from the cube exactly as the
/// post-trade `LimitStatus` read does (`cube_from_facts` → `firm_aggregate` /
/// `group_by`), so pre-trade and post-trade agree on the same node units.
fn project_pre_trade(
    facts: &[RiskFact],
    limits: &LimitTree,
    hierarchy: &Hierarchy,
    position: &PositionRisk,
    leaf: &CanonicalLeaf,
    key: FactKey,
    exclude_handle: u32,
) -> PreTradeResult {
    let current: Vec<RiskFact> = facts
        .iter()
        .filter(|f| f.position_id.0 != exclude_handle)
        .cloned()
        .collect();
    let grid = default_grid();
    let cube = cube_from_facts(&current, hierarchy.clone());
    let path = ScopePath::resolve(&key, hierarchy);
    let vega_pillar = grid.pillar_of(leaf, position);
    let increment = IncrementalTrade::from_leaf(leaf, vega_pillar);
    let node_at = |scope: LimitScope| -> NodeAggregate {
        match scope.dimension() {
            None => cube.firm_aggregate(&grid),
            Some(d) => cube
                .group_by(d, &grid)
                .into_iter()
                .find(|n| Some(n.group) == scope.group_value())
                .unwrap_or_else(|| NodeAggregate::empty(scope.group_value().unwrap_or(0))),
        }
    };
    // Booking never re-derives VaR/ES/stop-loss (no shock grid on the booking path);
    // the non-additive metrics read `0` and so never spuriously reject a booking.
    let nonadditive_at = |_scope: LimitScope| NonAdditiveExposure::default();
    pre_trade_check(limits, &path, &increment, node_at, nonadditive_at)
}

/// A `failed_precondition` status carrying the first hard breach behind a pre-trade
/// [`PreTradeDecision::Reject`] — the uniform `LimitBreached` wire reason every FX
/// booking front-end surfaces (guardrail #11 cross-client parity). Shared by the FX
/// sink and the rates sink ([`crate::services::rates_book`]).
#[must_use]
pub(crate) fn limit_breached_status(res: &PreTradeResult) -> tonic::Status {
    tonic::Status::failed_precondition(limit_breach_message(res))
}

/// The human-readable `LimitBreached` reason for the first hard breach on a rejected
/// pre-trade path (`<scope>/<metric>` + utilization + cap), reused verbatim as the
/// FIX `ExecutionReport` `Text(58)` and the gRPC `Status` message.
#[must_use]
pub(crate) fn limit_breach_message(res: &PreTradeResult) -> String {
    match res.hard_breaches().next() {
        Some(c) => format!(
            "limit breached: {:?}/{:?} at {:.4}x of cap {} ({:?})",
            c.scope, c.limit.metric, c.utilization.ratio, c.limit.cap, c.utilization.status
        ),
        None => "limit breached".to_owned(),
    }
}

/// Project this fill's post-book risk-book aggregate over the resolved book **and each
/// ancestor**, returning a typed `failed_precondition` breach if any HARD notional cap
/// (`max_net_notional` on `|net|`, `max_gross_notional` on gross) would be EXCEEDED — the
/// per-book enforcement (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §8.3). Runs on the SAME
/// read snapshot the routing used, BEFORE any mutation, so a breach rejects with the store
/// left unmutated (the load-bearing risk-control invariant, mirroring the FactKey gate).
///
/// **Roll-up.** A scope's aggregate is its whole subtree: this fill (always within the
/// scope, since every checked scope is the resolved book or one of its ancestors) plus every
/// current position whose stamped risk book is the scope itself or a descendant of it. The
/// prior fact under `exclude_handle` (a re-book of the same id) is excluded so a re-book
/// projects `others + this fill` and never double-counts (mirrors [`project_pre_trade`]).
///
/// `max_dv01` is skipped: DV01 is not available for FX vanilla (the known rates seam, §5.3),
/// so a DV01 cap has no numerator to gate on here.
///
/// Returns `None` — gate skipped, byte-identical to the pre-enforcement path — when no book
/// on the fill's ancestor path carries a `RiskLimits`.
fn project_risk_book_breach(
    inner: &StoreInner,
    resolved_book: &str,
    fill_notional: f64,
    exclude_handle: u32,
) -> Option<tonic::Status> {
    // A single incoming fill: its signed notional is the net, its magnitude the gross, and
    // exactly its own prior fact is excluded (re-book supersede) — byte-identical to the
    // original single-position gate the routed-fill path in [`PositionStore::book`] runs.
    project_risk_book_breach_multi(
        inner,
        resolved_book,
        fill_notional,
        fill_notional.abs(),
        &[exclude_handle],
    )
}

/// The general per-book hard-limit projection over an incoming `(net, gross)` notional
/// pair and a set of excluded handles — the shared core of the single-fill routed gate
/// ([`project_risk_book_breach`]) and the multi-position transfer headroom check
/// ([`PositionStore::check_risk_book_headroom`], §6). A re-attribution or an economic
/// transfer can move several positions with mixed sides at once, so the incoming net and
/// gross are passed separately (for a mixed-sign batch `gross ≠ |net|`) and every moved
/// source handle is excluded from the current-book roll-up so the projection is
/// `others + incoming`, never double-counting a moved line against itself.
fn project_risk_book_breach_multi(
    inner: &StoreInner,
    resolved_book: &str,
    incoming_net: f64,
    incoming_gross: f64,
    exclude_handles: &[u32],
) -> Option<tonic::Status> {
    let limits_map = &inner.risk_book_limits;
    // The scopes to enforce: the resolved book + its ancestors, keeping only those that
    // actually carry a `RiskLimits`. No caps on the whole path ⇒ nothing to gate.
    let chain = risk_book_chain(limits_map, resolved_book);
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
    // The current per-book OWN (un-rolled) net/gross from the stamped facts, excluding this
    // fill's own prior fact (re-book supersede — never double-counted against itself).
    let mut own: HashMap<&str, (f64, f64)> = HashMap::new();
    for f in &inner.facts {
        if exclude_handles.contains(&f.position_id.0) {
            continue;
        }
        if let Some(book) = inner.risk_book.get(&f.position_id.0) {
            let n = f.measure.position.notional_base;
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
            if risk_book_chain_contains(limits_map, book, scope) {
                net += bnet;
                gross += bgross;
            }
        }
        if let Some(cap) = lim.max_net_notional
            && net.abs() > cap
        {
            return Some(risk_book_limit_breached(
                scope,
                "net_notional",
                net.abs(),
                cap,
            ));
        }
        if let Some(cap) = lim.max_gross_notional
            && gross > cap
        {
            return Some(risk_book_limit_breached(
                scope,
                "gross_notional",
                gross,
                cap,
            ));
        }
        // `max_dv01` intentionally skipped — no DV01 numerator for FX vanilla.
    }
    None
}

/// The resolved book and its ancestors (self first, then upward the parent chain),
/// cycle-guarded against a not-yet-validated registry.
fn risk_book_chain<'a>(
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

/// Whether `scope` lies on `book`'s ancestor-or-self chain — equivalently, whether `book`
/// is in the subtree rooted at `scope`.
fn risk_book_chain_contains(
    limits_map: &HashMap<String, RiskBookLimitDef>,
    book: &str,
    scope: &str,
) -> bool {
    risk_book_chain(limits_map, book).contains(&scope)
}

/// A typed `failed_precondition` for a routed-risk-book hard notional-cap breach — book id +
/// metric + used/limit (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §8.3). Distinct from the
/// FactKey [`limit_breached_status`] so a client can tell a per-book cap breach apart.
#[must_use]
fn risk_book_limit_breached(book_id: &str, metric: &str, used: f64, cap: f64) -> tonic::Status {
    tonic::Status::failed_precondition(format!(
        "risk book limit breached: {book_id}/{metric} used {used:.4} exceeds cap {cap}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{Ccy, VanillaInputs};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn booked(id: u64, notional: f64) -> BookedPosition {
        BookedPosition {
            position_id: id,
            pair: eurusd(),
            option: OptionType::Call,
            notional_base: notional,
            inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            quoted_delta: DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
            surface_version: 1,
        }
    }

    fn attribution(book: &str, trader: &str) -> AttributionRecord {
        AttributionRecord {
            quoted_by: Some(BookId {
                book: "AUTO-MM".to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::AutoPricer("celnet-auto-pricer".to_owned())),
                }),
            }),
            held_by: Some(BookId {
                book: book.to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::Trader(trader.to_owned())),
                }),
            }),
            won: Some(true),
            lp_count: Some(3),
        }
    }

    /// A firm-wide graph: notional > 50m → BOOK-A, else DEFAULT (§4).
    fn notional_graph() -> RiskRoutingGraph {
        use celnet_risk_routing::{RouteField, RouteOp, RouteValue, RoutingNode};
        use std::collections::BTreeMap;
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0u32,
            RoutingNode::Condition {
                field: RouteField::Notional,
                op: RouteOp::Gt,
                value: RouteValue::Num(50_000_000.0),
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

    /// With a graph installed, a fill routes into the resolved risk book; the by-risk-book
    /// query returns it there — the core of the feature.
    #[test]
    fn a_routed_fill_lands_in_the_resolved_risk_book() {
        let store = PositionStore::new();
        store.set_routing(Some(notional_graph()));
        store
            .book_from_attribution(booked(1, 60_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .expect("books");
        store
            .book_from_attribution(booked(2, 10_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .expect("books");
        assert_eq!(store.risk_book_of(1).as_deref(), Some("BOOK-A"));
        assert_eq!(store.risk_book_of(2).as_deref(), Some("DEFAULT"));
        assert_eq!(store.positions_in_risk_book("BOOK-A").len(), 1);
        assert_eq!(store.positions_in_risk_book("DEFAULT").len(), 1);
    }

    /// No graph installed ⇒ every fill books UNROUTED (`risk_book_of` is None) — the
    /// backward-compatibility guarantee: routing off is byte-identical to the old path.
    #[test]
    fn no_graph_books_unrouted() {
        let store = PositionStore::new();
        assert!(!store.has_routing());
        store
            .book_from_attribution(booked(1, 60_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .expect("books");
        assert_eq!(store.risk_book_of(1), None);
        assert!(store.positions_in_risk_book("BOOK-A").is_empty());
    }

    /// `routing_context_from` maps the fill economics: side from notional sign, |notional|,
    /// product, strike/tenor from the inputs.
    #[test]
    fn routing_context_maps_fill_economics() {
        let long = routing_context_from(&booked(1, 5_000_000.0), None);
        assert_eq!(long.side, "Buy");
        assert_eq!(long.notional, 5_000_000.0);
        assert_eq!(long.product, "call");
        assert_eq!(long.strike, 1.12);
        assert_eq!(long.tenor, 1.0);
        assert_eq!(long.ccy, eurusd().to_string());
        let short = routing_context_from(&booked(2, -3_000_000.0), None);
        assert_eq!(short.side, "Sell");
        assert_eq!(short.notional, 3_000_000.0);
    }

    /// The interner is deterministic and never returns 0; distinct strings get
    /// distinct handles and a repeat returns the same handle.
    #[test]
    fn interner_is_deterministic_and_nonzero() {
        let mut i = Interner::default();
        let a = i.intern("EM-VOL-1");
        let b = i.intern("G10-1");
        let a2 = i.intern("EM-VOL-1");
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
        assert_eq!(a, a2);
        assert_eq!(i.resolve(a), Some("EM-VOL-1"));
        assert_eq!(i.resolve(0), None);
    }

    /// Booking from an attribution chain interns the holder book/seat onto the
    /// cube handles and records one fact; a re-book of the same id supersedes.
    #[test]
    fn book_from_attribution_supersedes_by_id() {
        let store = PositionStore::new();
        store
            .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .unwrap();
        store
            .book_from_attribution(booked(2, 5_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .unwrap();
        assert_eq!(store.len(), 2);
        // Re-book position 1 at a new notional — supersede, not duplicate.
        store
            .book_from_attribution(booked(1, 20_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .unwrap();
        assert_eq!(store.len(), 2);

        let snap = store.snapshot();
        // The two positions share one interned book handle (same holder book).
        let books: std::collections::HashSet<u32> =
            snap.facts.iter().map(|f| f.key.book.0).collect();
        assert_eq!(books.len(), 1, "same holder book → one book handle");
        // Attribution provenance is retained for ListPositions.
        assert!(snap.attribution_of(1).is_some());
        assert_eq!(
            snap.attribution_of(1)
                .unwrap()
                .held_by
                .as_ref()
                .unwrap()
                .book,
            "EM-VOL-1"
        );
    }

    /// Two different holder books get two book handles and roll into one desk when
    /// the hierarchy maps them to the same desk.
    #[test]
    fn distinct_books_one_desk_via_hierarchy() {
        let store = PositionStore::new();
        store
            .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "a"))
            .unwrap();
        store
            .book_from_attribution(booked(2, 7_000_000.0), &attribution("EM-VOL-2", "b"))
            .unwrap();
        let b1 = store.intern("EM-VOL-1");
        let b2 = store.intern("EM-VOL-2");
        let desk = store.intern("EM-VOL-DESK");
        store.set_book_desk(b1, desk);
        store.set_book_desk(b2, desk);

        let snap = store.snapshot();
        assert_eq!(snap.facts.len(), 2);
        assert_eq!(snap.hierarchy.desk_of(CubeBookId(b1)), Some(DeskId(desk)));
        assert_eq!(snap.hierarchy.desk_of(CubeBookId(b2)), Some(DeskId(desk)));
    }

    /// A booked **exotic** is recorded as a risk fact carrying its real exotic Greeks
    /// (additive leaf) and its `ExoticLeg` re-derivation source — so it appears in the
    /// roll-up alongside vanilla legs (W11-A: exotics no longer silently excluded).
    #[test]
    fn upsert_exotic_records_a_fact_with_exotic_leg() {
        use celnet_exotics::{BarrierKind, BarrierStyle, SingleBarrier};
        use celnet_risk_cube::{ExoticKind, ExoticLeg};

        let store = PositionStore::new();
        let spec = SingleBarrier {
            kind: BarrierKind {
                up: true,
                style: BarrierStyle::KnockOut,
                option: OptionType::Call,
            },
            strike: 1.10,
            barrier: 1.25,
            rebate: 0.0,
        };
        let leg = ExoticLeg::new(
            eurusd(),
            ExoticKind::SingleBarrier(spec),
            8_000_000.0,
            celnet_types::VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let key = FactKey {
            trader: TraderId(1),
            book: CubeBookId(1),
            desk: DeskId(0),
            underlying: celnet_types::Underlying::Fx(eurusd()),
            location: LocationId(1),
            entity: EntityId(1),
        };
        store
            .upsert_exotic(
                BookedExotic {
                    position_id: 42,
                    leg,
                    option: OptionType::Call,
                    quoted_delta: DeltaConvention::SpotUnadjusted,
                    premium_style: PremiumStyle::DomesticPips,
                    surface_version: 1,
                },
                key,
                None,
            )
            .unwrap();

        let snap = store.snapshot();
        assert_eq!(snap.facts.len(), 1);
        let fact = &snap.facts[0];
        assert!(
            fact.measure.exotic.is_some(),
            "the booked exotic must carry an ExoticLeg re-derivation source"
        );
        // The leaf is the REAL exotic Greek set (premium == closed-form barrier price).
        let want = celnet_exotics::single_barrier_price(&(&leg.inputs).into(), spec) * 8_000_000.0;
        assert!((fact.measure.leaf.premium_quote - want).abs() <= 1e-6 * (1.0 + want.abs()));
    }

    // ---- ADR-0016 A1 pre-trade limit gate at the FX position sink ----
    //
    // `book_from_attribution` is the exact function the RFS click-to-trade front-end
    // (`TokenLedger::try_book` → `record_booked_position`) funnels every booked vanilla
    // fill through, so gating it gates that front-end by construction.

    use celnet_limits::{LimitMetric, LimitScope, LimitSpec};

    /// FRONT-END 1 (clicktrade `TokenLedger::try_book`): a hard firm-wide Delta cap that
    /// the proposed booking would blow **rejects** the book with a `failed_precondition`
    /// `LimitBreached` status, and the store is left **unmutated** (the load-bearing
    /// risk-control invariant — a hard-limit-blown lift books nothing).
    #[test]
    fn clicktrade_sink_rejects_a_hard_limit_blown_book() {
        let store = PositionStore::new();
        // A firm Delta cap of 1 base unit: any real option delta (a 10mm EURUSD call is
        // millions of base delta) blows it hard.
        store.set_limit(LimitScope::Firm, LimitSpec::hard(LimitMetric::Delta, 1.0));

        let err = store
            .book_from_attribution(booked(0, 10_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .expect_err("a hard-limit-blown book must be rejected");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("limit breached"),
            "the reject carries the uniform LimitBreached reason, got {:?}",
            err.message()
        );
        assert_eq!(store.len(), 0, "a rejected book must not mutate the store");
    }

    /// A **within-limit** book still succeeds (an accept decision) and records the fact —
    /// the gate must not refuse a legitimate booking.
    #[test]
    fn within_limit_book_succeeds() {
        let store = PositionStore::new();
        // A generous firm Delta cap no single 10mm line approaches.
        store.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(LimitMetric::Delta, 1.0e12),
        );

        let decision = store
            .book_from_attribution(booked(0, 10_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .expect("a within-limit book succeeds");
        assert_eq!(decision, PreTradeDecision::Accept);
        assert_eq!(store.len(), 1, "a within-limit book records the fact");
    }

    /// A **soft** breach books (the position is recorded) but returns a `Warn` decision —
    /// soft limits early-warn, they never block.
    #[test]
    fn soft_breach_books_and_warns() {
        let store = PositionStore::new();
        // A soft Delta cap of 1 base unit is breached by any real line, but soft ⇒ warn.
        store.set_limit(LimitScope::Firm, LimitSpec::soft(LimitMetric::Delta, 1.0));

        let decision = store
            .book_from_attribution(booked(0, 10_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .expect("a soft breach books (warn, never block)");
        assert_eq!(decision, PreTradeDecision::Warn);
        assert_eq!(store.len(), 1, "a soft-warned book is still recorded");
    }

    /// The check-only `evaluate_pre_trade` (the path `QuoteService::AcceptQuote` and the
    /// FIX acceptor consult) rejects a hard-limit-blown line **without** recording a
    /// fact — it is a consult, not a booking.
    #[test]
    fn evaluate_pre_trade_rejects_without_recording() {
        let store = PositionStore::new();
        store.set_limit(LimitScope::Firm, LimitSpec::hard(LimitMetric::Delta, 1.0));

        let result = store
            .evaluate_pre_trade(&booked(0, 10_000_000.0))
            .expect("canonicalization succeeds");
        assert_eq!(result.decision, PreTradeDecision::Reject);
        assert_eq!(store.len(), 0, "a check-only evaluation records nothing");

        // With no limit configured the consult is a no-op Accept (byte-identical path).
        let clean = PositionStore::new();
        assert_eq!(
            clean
                .evaluate_pre_trade(&booked(0, 10_000_000.0))
                .expect("no-limit consult")
                .decision,
            PreTradeDecision::Accept
        );
    }

    // ---- §8.3 per-book RiskLimits enforcement at the routed booking sink ----

    use celnet_risk_routing::RiskRoutingGraph;

    /// A risk-book limit-view row with net/gross caps (DV01 always absent — the FX seam).
    fn limit_def(
        id: &str,
        parent: Option<&str>,
        net: Option<f64>,
        gross: Option<f64>,
    ) -> RiskBookLimitDef {
        RiskBookLimitDef {
            id: id.to_owned(),
            parent_id: parent.map(str::to_owned),
            limits: Some(RiskLimits {
                max_net_notional: net,
                max_gross_notional: gross,
                max_dv01: None,
            }),
        }
    }

    /// A firm graph splitting on notional into two SIBLING child books (both under a common
    /// parent in the limit view): `> 50m → CHILD-A`, else `→ CHILD-B`.
    fn child_split_graph() -> RiskRoutingGraph {
        use celnet_risk_routing::{RouteField, RouteOp, RouteValue, RoutingNode};
        use std::collections::BTreeMap;
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0u32,
            RoutingNode::Condition {
                field: RouteField::Notional,
                op: RouteOp::Gt,
                value: RouteValue::Num(50_000_000.0),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1u32,
            RoutingNode::Book {
                risk_book_id: "CHILD-A".to_owned(),
            },
        );
        nodes.insert(
            2u32,
            RoutingNode::Book {
                risk_book_id: "CHILD-B".to_owned(),
            },
        );
        RiskRoutingGraph { entry: 0, nodes }
    }

    /// A routed fill whose book's HARD gross cap would be exceeded is REJECTED before any
    /// mutation: the first fill books, the second (which would blow the cap) is refused with
    /// a typed `failed_precondition` and the store is left unmutated (no stamp for it).
    #[test]
    fn routed_fill_rejected_when_book_gross_cap_exceeded() {
        let store = PositionStore::new();
        store.set_routing(Some(notional_graph()));
        store.set_risk_books(vec![
            limit_def("BOOK-A", None, None, Some(100_000_000.0)),
            RiskBookLimitDef {
                id: "DEFAULT".to_owned(),
                parent_id: None,
                limits: None,
            },
        ]);

        // First 60m routes to BOOK-A and books (gross 60m <= 100m).
        store
            .book_from_attribution(booked(1, 60_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .expect("first fill books");
        assert_eq!(store.risk_book_of(1).as_deref(), Some("BOOK-A"));
        assert_eq!(store.len(), 1);

        // Second 60m would take BOOK-A gross to 120m > 100m ⇒ REJECT, store unmutated.
        let err = store
            .book_from_attribution(booked(2, 60_000_000.0), &attribution("EM-VOL-1", "jdoe"))
            .expect_err("the gross cap is blown");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("risk book limit breached")
                && err.message().contains("gross_notional"),
            "typed per-book breach, got {:?}",
            err.message()
        );
        assert_eq!(store.len(), 1, "a rejected fill must not mutate the store");
        assert_eq!(store.risk_book_of(2), None, "rejected fill leaves no stamp");
        assert_eq!(store.positions_in_risk_book("BOOK-A").len(), 1);
    }

    /// A PARENT cap breached by the SUM of two sibling child-book fills rejects the second at
    /// the parent scope, even though each child is individually under (no child carries a
    /// cap) — the ancestor subtree roll-up.
    #[test]
    fn routed_fill_rejected_at_ancestor_scope_rollup() {
        let store = PositionStore::new();
        store.set_routing(Some(child_split_graph()));
        store.set_risk_books(vec![
            limit_def("PARENT", None, None, Some(90_000_000.0)),
            RiskBookLimitDef {
                id: "CHILD-A".to_owned(),
                parent_id: Some("PARENT".to_owned()),
                limits: None,
            },
            RiskBookLimitDef {
                id: "CHILD-B".to_owned(),
                parent_id: Some("PARENT".to_owned()),
                limits: None,
            },
        ]);

        // 60m (>50m) → CHILD-A: parent subtree 60m <= 90m ⇒ books.
        store
            .book_from_attribution(booked(1, 60_000_000.0), &attribution("D", "t"))
            .expect("child-a fill books");
        assert_eq!(store.risk_book_of(1).as_deref(), Some("CHILD-A"));

        // 40m (<=50m) → CHILD-B: parent subtree 60m + 40m = 100m > 90m ⇒ REJECT at PARENT.
        let err = store
            .book_from_attribution(booked(2, 40_000_000.0), &attribution("D", "t"))
            .expect_err("the parent roll-up cap is blown");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("PARENT/gross_notional"),
            "rejected at the parent scope, got {:?}",
            err.message()
        );
        assert_eq!(store.len(), 1, "a rejected fill must not mutate the store");
        assert_eq!(store.risk_book_of(2), None);
    }

    /// Net and gross caps gate independently: a long+short pair within the net cap but over
    /// the gross cap rejects on GROSS; two same-sign fills over the net cap but under a
    /// generous gross cap reject on NET.
    #[test]
    fn net_and_gross_caps_gate_independently() {
        // GROSS: +100m then -80m ⇒ net 20m (ok), gross 180m > 150m ⇒ reject on gross.
        let store = PositionStore::new();
        store.set_routing(Some(notional_graph()));
        store.set_risk_books(vec![limit_def(
            "BOOK-A",
            None,
            Some(150_000_000.0),
            Some(150_000_000.0),
        )]);
        store
            .book_from_attribution(booked(1, 100_000_000.0), &attribution("D", "t"))
            .expect("long books");
        let err = store
            .book_from_attribution(booked(2, -80_000_000.0), &attribution("D", "t"))
            .expect_err("gross cap blown");
        assert!(
            err.message().contains("gross_notional"),
            "rejected on gross, got {:?}",
            err.message()
        );
        assert_eq!(store.len(), 1);

        // NET: +100m then +80m ⇒ net 180m > 150m (gross 180m under the 1bn gross cap) ⇒
        // reject on net.
        let store2 = PositionStore::new();
        store2.set_routing(Some(notional_graph()));
        store2.set_risk_books(vec![limit_def(
            "BOOK-A",
            None,
            Some(150_000_000.0),
            Some(1_000_000_000.0),
        )]);
        store2
            .book_from_attribution(booked(1, 100_000_000.0), &attribution("D", "t"))
            .expect("first long books");
        let err2 = store2
            .book_from_attribution(booked(2, 80_000_000.0), &attribution("D", "t"))
            .expect_err("net cap blown");
        assert!(
            err2.message().contains("net_notional"),
            "rejected on net, got {:?}",
            err2.message()
        );
        assert_eq!(store2.len(), 1);
    }

    /// Backward-compat: routing on but NO book limits (empty view, or a view whose resolved
    /// book carries `None` caps) ⇒ every fill books, byte-identical to the pre-enforcement
    /// path.
    #[test]
    fn routing_without_book_limits_books_unchanged() {
        // No limit view at all: the per-book gate is skipped.
        let store = PositionStore::new();
        store.set_routing(Some(notional_graph()));
        store
            .book_from_attribution(booked(1, 60_000_000.0), &attribution("D", "t"))
            .expect("books");
        store
            .book_from_attribution(booked(2, 60_000_000.0), &attribution("D", "t"))
            .expect("books");
        assert_eq!(store.len(), 2);
        assert_eq!(store.positions_in_risk_book("BOOK-A").len(), 2);

        // A limit view present but the resolved book carries `None` caps ⇒ still skipped.
        let store2 = PositionStore::new();
        store2.set_routing(Some(notional_graph()));
        store2.set_risk_books(vec![RiskBookLimitDef {
            id: "BOOK-A".to_owned(),
            parent_id: None,
            limits: None,
        }]);
        store2
            .book_from_attribution(booked(1, 90_000_000.0), &attribution("D", "t"))
            .expect("books");
        store2
            .book_from_attribution(booked(2, 90_000_000.0), &attribution("D", "t"))
            .expect("books");
        assert_eq!(store2.len(), 2);
    }
}
