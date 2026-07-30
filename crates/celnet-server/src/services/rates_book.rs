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

use std::collections::HashMap;
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
use celnet_risk_routing::{RiskRouter, RiskRoutingGraph, RoutingContext};

use crate::config::identity::RiskLimits;
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
}

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
        let resolved_book: Option<String> = {
            let routing = self.routing.read().expect("rates routing lock poisoned");
            routing.as_ref().and_then(|graph| {
                let ctx = routing_context_from_rates(&position, &attribution);
                match RiskRouter::route(graph, &ctx) {
                    Ok(book) => Some(book.to_owned()),
                    Err(err) => {
                        tracing::warn!(
                            position_id = position.position_id,
                            %err,
                            "rates risk routing failed; booking unrouted",
                        );
                        None
                    }
                }
            })
        };
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
            *slot = position;
        } else {
            g.push(position);
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
        Ok(position)
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
        let fill_notional = rates_signed_notional(fill);
        let exclude = fill.position_id;
        // The current per-book OWN (un-rolled) net/gross from the stamped rates positions,
        // excluding this fill's own prior fact (re-book supersede — never double-counted).
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
            if exclude != 0 && p.position_id == exclude {
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
            // Subtree roll-up: this fill + every current book whose ancestor-or-self chain
            // passes through `scope` (i.e. the book is in the subtree rooted at `scope`).
            let mut net = fill_notional;
            let mut gross = fill_notional.abs();
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
}
