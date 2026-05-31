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
//! # Honest scope: vanilla leaves only
//!
//! The risk cube aggregates **vanilla** option leaves (`celnet-risk-normalize`'s
//! `canonicalize` re-derives a vanilla `PositionRisk` via `celnet-vanilla`). A booked
//! exotic (barrier / digital / touch) has no canonical-vanilla leaf, so it is **not**
//! recorded as a risk fact — recording one would fake a vanilla risk it does not
//! have (guardrail #2). The store records the vanilla legs of the live book; exotic
//! aggregation is a named, deferred extension (it would need the cube to grow an
//! exotic-leaf measure), not a stub here.

use std::collections::HashMap;
use std::sync::RwLock;

use celnet_proto::{AttributionRecord, BookId, Owner, owner};
use celnet_risk_cube::{
    BookId as CubeBookId, DeskId, EntityId, FactKey, FactMeasure, Hierarchy, LocationId,
    PositionId, RiskFact, TraderId,
};
use celnet_risk_normalize::{PositionRisk, canonicalize};
use celnet_types::{CcyPair, DeltaConvention, OptionType, PremiumStyle};

use celnet_limits::LimitTree;

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

/// The shared live position book + org hierarchy + attribution interner + limit
/// tree the risk service aggregates over. Cheap to share behind an [`Arc`]; every
/// mutation takes the write lock briefly (the cube runs off the hot path, so the
/// lock is never contended by the pricing core).
///
/// [`Arc`]: std::sync::Arc
#[derive(Debug)]
pub struct PositionStore {
    inner: RwLock<StoreInner>,
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
}

impl Default for PositionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionStore {
    /// An empty store: no positions, an empty hierarchy, an empty limit tree.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(StoreInner::default()),
        }
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
            }),
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

    /// Configure a limit at a hierarchy scope (admin / setup path).
    pub fn set_limit(&self, scope: celnet_limits::LimitScope, spec: celnet_limits::LimitSpec) {
        let mut g = self.inner.write().expect("position store lock poisoned");
        g.limits.set(scope, spec);
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
        let position = PositionRisk::new(
            booked.pair,
            booked.option,
            booked.notional_base,
            booked.inputs,
            booked.quoted_delta,
            booked.premium_style,
        );
        let fact = RiskFact {
            position_id: PositionId(handle),
            key,
            measure: FactMeasure {
                leaf: canonicalize(&position),
                position,
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
    /// `invalid_argument` if `booked.position_id` does not fit a `u32` cube handle.
    pub fn book_from_attribution(
        &self,
        booked: BookedPosition,
        attribution: &AttributionRecord,
    ) -> Result<(), tonic::Status> {
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
            ccy_pair: booked.pair,
            location: LocationId(location_h),
            // Entity resolves from the location's parent pointer (0 ⇒ resolve).
            entity: EntityId(0),
        };
        self.upsert(booked, key, Some(attribution.clone()))
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
}
