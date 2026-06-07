//! The OLAP dimension model (`docs/RISK-HIERARCHY.md` §2.1).
//!
//! The firm's risk is modelled as a **cube**: an immutable position-level fact
//! table, with a set of **independent** dimensions (trader, book, desk,
//! currency-pair, booking-location, legal-entity) each carrying a
//! **parent-pointer hierarchy**. A roll-up is "group the facts by a dimension's
//! ancestor at level L, then reduce." The key structural point from §2.1 is that
//! desk, currency-pair, booking-location and legal-entity are **orthogonal axes,
//! not one nesting** — a single position is simultaneously an EM-vol-desk fact,
//! an EURTRY fact, a London-entity fact, and a UK-location fact. Modelling them as
//! one tree would lose roll-up paths, so each is its own dimension.
//!
//! # Dimension keys are cube-internal, not wire contract
//!
//! The organizational identifiers ([`TraderId`], [`BookId`], [`DeskId`],
//! [`LocationId`], [`EntityId`], [`PositionId`]) are interned `u32` handles owned
//! by this crate. They are deliberately **not** in `celnet-types`/`celnet-proto`:
//! they are aggregation-layer keys, and `celnet-proto` carries the wire-level
//! `BookId`/`Owner` attribution identity that the server maps onto these interned
//! handles. Keeping them here lets the cube evolve its hierarchy model without
//! touching the frozen wire contract (guardrail #9: one clean current contract).

use celnet_types::CcyPair;

/// The set of independent dimensions a [`RiskFact`] is keyed by
/// (`docs/RISK-HIERARCHY.md` §2.1). These are **orthogonal** roll-up axes.
///
/// `Trader`/`Book`/`Desk` form the front-office org chain; `Location`/`Entity`
/// the booking/regulatory chain; `CcyPair` the underlying axis. The `Firm` apex
/// is implicit — every dimension rolls up into the single firm node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DimensionId {
    /// The trader who owns the position.
    Trader,
    /// The book the position sits in.
    Book,
    /// The desk the book belongs to (a regulatory unit under FRTB).
    Desk,
    /// The currency pair (the underlying axis — orthogonal to the org axes).
    CcyPair,
    /// The booking location (follow-the-sun / country).
    Location,
    /// The legal entity (the regulatory-capital unit).
    Entity,
}

macro_rules! dim_key {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl $name {
            /// The raw interned handle.
            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0
            }
        }

        impl From<u32> for $name {
            fn from(v: u32) -> Self {
                Self(v)
            }
        }
    };
}

dim_key!(
    /// Interned handle for a single position/trade (the leaf identity).
    PositionId
);
dim_key!(
    /// Interned handle for a trader.
    TraderId
);
dim_key!(
    /// Interned handle for a book.
    BookId
);
dim_key!(
    /// Interned handle for a desk.
    DeskId
);
dim_key!(
    /// Interned handle for a booking location.
    LocationId
);
dim_key!(
    /// Interned handle for a legal entity.
    EntityId
);

/// The org-chart placement of a position across all independent dimensions
/// (`docs/RISK-HIERARCHY.md` §2.1). This is the **foreign-key tuple** into the
/// dimension hierarchies — the part of a [`RiskFact`] that decides which nodes a
/// leaf rolls up into.
///
/// All axes are carried explicitly so a group-by can select any subset at any
/// level. The parent chains (`Book → Desk`, `Location → Entity → Firm`) live in
/// [`Hierarchy`]; this struct holds only the finest-level keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FactKey {
    /// The trader who owns the position.
    pub trader: TraderId,
    /// The book the position sits in.
    pub book: BookId,
    /// The desk the book belongs to.
    pub desk: DeskId,
    /// The currency pair underlying the position.
    pub ccy_pair: CcyPair,
    /// The booking location.
    pub location: LocationId,
    /// The legal entity.
    pub entity: EntityId,
}

impl FactKey {
    /// The key value at a given dimension, as a `u64` discriminant suitable for
    /// grouping. For [`DimensionId::CcyPair`] the two 3-letter codes are packed
    /// into the low 48 bits; the org keys widen their `u32` handle.
    ///
    /// Used by the cube's group-by to bucket facts by one dimension without
    /// allocating a per-dimension key type.
    #[must_use]
    pub fn group_value(&self, dim: DimensionId) -> u64 {
        match dim {
            DimensionId::Trader => u64::from(self.trader.0),
            DimensionId::Book => u64::from(self.book.0),
            DimensionId::Desk => u64::from(self.desk.0),
            DimensionId::Location => u64::from(self.location.0),
            DimensionId::Entity => u64::from(self.entity.0),
            DimensionId::CcyPair => {
                let b = self.ccy_pair.base.as_str().as_bytes();
                let q = self.ccy_pair.quote.as_str().as_bytes();
                let mut v: u64 = 0;
                for &x in b.iter().chain(q.iter()) {
                    v = (v << 8) | u64::from(x);
                }
                v
            }
        }
    }
}

/// A measure attached to a [`RiskFact`]: the convention-free, numeraire-resolvable
/// canonical leaf (from `celnet-risk-normalize`), plus the *re-derivation inputs*
/// the non-additive reducers need.
///
/// The additive part is the [`CanonicalLeaf`](celnet_risk_normalize::CanonicalLeaf)
/// — it sums directly, **regardless of whether the leaf came from a vanilla or an
/// exotic leg** (the canonicalization unifies them at the leaf level). The
/// non-additive reducers (`docs/RISK-HIERARCHY.md` §2.5) cannot be summed; they must
/// be **re-derived from the underlying instrument** under shocks, and a vanilla and
/// an exotic re-price through different pricers — so the fact records which one it is:
///
/// - **vanilla fact** — `exotic == None`. The re-derivation source is `position`,
///   the original [`PositionRisk`](celnet_risk_normalize::PositionRisk) re-priced
///   under shocks via `celnet-vanilla`.
/// - **exotic fact** — `exotic == Some(leg)`. The re-derivation source is the
///   [`ExoticLeg`](crate::exotic::ExoticLeg), re-priced under shocks via the real
///   closed-form exotic pricer (a barrier's gamma sign flip, a digital's pin risk —
///   never a vanilla proxy). The `leaf` is the exotic's REAL Greek set, so an exotic
///   contributes correctly to the additive roll-up. `position` still carries the
///   exotic's **underlying-vanilla metadata** (same pair/option/notional/inputs) so
///   the vega-pillar bucketing has a `(tenor × delta)` to map onto; it is **never
///   priced** for an exotic fact (the exotic pricer is used instead).
///
/// This split is what lets a booked exotic stop being silently excluded from
/// firm/desk/book risk: its Greeks roll up additively, and its non-additive
/// contribution re-prices the true exotic payoff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FactMeasure {
    /// The convention-free canonical leaf (the additive measure carrier; the real
    /// exotic Greeks for an exotic fact).
    pub leaf: celnet_risk_normalize::CanonicalLeaf,
    /// The originating vanilla position (the re-derivation source for a **vanilla**
    /// fact; underlying-vanilla bucketing metadata for an **exotic** fact, never
    /// priced when `exotic` is `Some`).
    pub position: celnet_risk_normalize::PositionRisk,
    /// The exotic re-derivation source when this fact is an exotic leg (`None` for a
    /// vanilla fact). When `Some`, the non-additive reducers re-price THIS leg, not
    /// `position`.
    pub exotic: Option<crate::exotic::ExoticLeg>,
}

/// One immutable row of the fact table: a position's org placement + its risk
/// measure, stamped with the surface version that produced it
/// (`docs/RISK-HIERARCHY.md` §2.1).
///
/// Facts are append-only and never mutated in place — a re-valuation produces a
/// *new* fact for the same `position` under a new `surface_version`, so a node
/// total is always reconcilable to the exact facts that built it (§2.5). The
/// cube's `replace` keeps one current fact per position by id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskFact {
    /// The position identity (the leaf key; one current fact per id in the cube).
    pub position_id: PositionId,
    /// The org placement across the independent dimensions.
    pub key: FactKey,
    /// The risk measure (canonical leaf + re-derivation source).
    pub measure: FactMeasure,
    /// Which marked surface produced this fact (live / official-IPV / historical).
    /// Carried so a node can be aggregated under any single surface lens (§2.7).
    pub surface_version: u64,
}

/// The parent-pointer hierarchies for the org dimensions
/// (`docs/RISK-HIERARCHY.md` §2.1): `Book → Desk` and `Location → Entity`. A
/// roll-up that groups by `Desk` resolves each fact's `book`'s parent here.
///
/// `Trader`, `CcyPair`, `Desk` and `Entity` are themselves already roll-up
/// levels; only the two genuine parent chains (book→desk, location→entity) need a
/// stored pointer, the rest being direct keys on [`FactKey`]. Unmapped children
/// fall back to their own key widened — an unconfigured book is its own desk
/// group rather than silently merging into desk 0.
#[derive(Debug, Clone, Default)]
pub struct Hierarchy {
    book_to_desk: Vec<(BookId, DeskId)>,
    location_to_entity: Vec<(LocationId, EntityId)>,
}

impl Hierarchy {
    /// An empty hierarchy (every child resolves to itself).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `book` belongs to `desk`.
    pub fn set_book_desk(&mut self, book: BookId, desk: DeskId) {
        upsert(&mut self.book_to_desk, book, desk);
    }

    /// Record that `location` belongs to `entity`.
    pub fn set_location_entity(&mut self, location: LocationId, entity: EntityId) {
        upsert(&mut self.location_to_entity, location, entity);
    }

    /// The desk a book belongs to, or `None` if unconfigured.
    #[must_use]
    pub fn desk_of(&self, book: BookId) -> Option<DeskId> {
        self.book_to_desk
            .iter()
            .find(|(b, _)| *b == book)
            .map(|(_, d)| *d)
    }

    /// The entity a location belongs to, or `None` if unconfigured.
    #[must_use]
    pub fn entity_of(&self, location: LocationId) -> Option<EntityId> {
        self.location_to_entity
            .iter()
            .find(|(l, _)| *l == location)
            .map(|(_, e)| *e)
    }
}

/// Insert-or-replace a `(key, value)` pair in a small association vector.
fn upsert<K: PartialEq, V>(v: &mut Vec<(K, V)>, key: K, value: V) {
    if let Some(slot) = v.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = value;
    } else {
        v.push((key, value));
    }
}
