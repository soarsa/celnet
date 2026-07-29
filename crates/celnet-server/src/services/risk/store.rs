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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use crate::services::consensus::{ConsensusHandle, fx_book_key};

use celnet_entitlements::AccessMode;
use celnet_proto::{AttributionRecord, BookId, Owner, owner};
use celnet_risk_cube::{
    BookId as CubeBookId, DeskId, EntityId, FactKey, FactMeasure, Hierarchy, LocationId,
    NodeAggregate, PositionId, RiskFact, TraderId, VegaPillarMap,
};
use celnet_risk_normalize::{CanonicalLeaf, PositionRisk, canonicalize};
use celnet_types::{CcyPair, DeltaConvention, OptionType, PremiumStyle};

use celnet_limits::{
    IncrementalTrade, LimitScope, LimitTree, NonAdditiveExposure, PreTradeDecision, PreTradeResult,
    ScopePath, pre_trade_check,
};
use celnet_risk_routing::{RiskRoutingGraph, RiskRouter, RoutingContext};

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
        }
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
            }),
            // Carried for coherence; a staged store only ever backs the
            // post-boundary `*_impl` internals, which no longer re-authorize.
            permissive_access: AtomicBool::new(self.permissive_access.load(Ordering::Relaxed)),
            // A staged federation-union view is transient and never the must-order
            // writer, so it never replicates (ADR-0015 §4.3 single-writer discipline).
            consensus: OnceLock::new(),
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
    /// the position is not priceable; `failed_precondition` on a hard-limit breach.
    pub fn book(
        &self,
        booked: BookedPosition,
        key: FactKey,
        attribution: Option<AttributionRecord>,
    ) -> Result<PreTradeDecision, tonic::Status> {
        // Canonicalize off-lock (pure, convention-free). The key is cloned into the fact;
        // the original drives the pre-trade scope resolution below.
        let (position, leaf, fact) = canonical_vanilla_fact(&booked, key.clone())?;
        let handle = fact.position_id.0;

        // Pre-trade gate over a READ snapshot, BEFORE acquiring the write lock, so a hard
        // breach can never book AND the O(#facts) cube projection never runs while the
        // exclusive write lock is held (guardrails #6/#11 — a fill never serialises every
        // other booking behind its own limit projection). Skipped when no limit is
        // configured (the empty-tree default keeps the sink byte-identical to the pre-gate
        // booking path and builds no cube on a fill when the desk has set no caps).
        let decision = {
            let g = self.inner.read().expect("position store lock poisoned");
            if g.limits.is_empty() {
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
            }
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
        match attribution {
            Some(a) => {
                g.attribution.insert(handle, a);
            }
            None => {
                g.attribution.remove(&handle);
            }
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
}
