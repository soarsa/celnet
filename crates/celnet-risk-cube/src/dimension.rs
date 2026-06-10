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

use celnet_types::Underlying;

/// The set of independent dimensions a [`RiskFact`] is keyed by
/// (`docs/RISK-HIERARCHY.md` §2.1). These are **orthogonal** roll-up axes.
///
/// `Trader`/`Book`/`Desk` form the front-office org chain; `Location`/`Entity`
/// the booking/regulatory chain; `Underlying` the underlying axis. The `Firm` apex
/// is implicit — every dimension rolls up into the single firm node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DimensionId {
    /// The trader who owns the position.
    Trader,
    /// The book the position sits in.
    Book,
    /// The desk the book belongs to (a regulatory unit under FRTB).
    Desk,
    /// The underlying asset (the underlying axis — orthogonal to the org axes).
    /// Cross-asset: FX pairs, metals, equities, commodities and digital assets all
    /// share this axis; the group value packs the underlying's
    /// [`Underlying::as_ccy_pair`] projection where it has one (FX/metals — keeping
    /// FX group identity byte-identical) and falls back to the arm's stable hash
    /// otherwise.
    Underlying,
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FactKey {
    /// The trader who owns the position.
    pub trader: TraderId,
    /// The book the position sits in.
    pub book: BookId,
    /// The desk the book belongs to.
    pub desk: DeskId,
    /// The underlying asset of the position (cross-asset: FX pair, metal, equity,
    /// commodity or digital asset).
    pub underlying: Underlying,
    /// The booking location.
    pub location: LocationId,
    /// The legal entity.
    pub entity: EntityId,
}

impl FactKey {
    /// The key value at a given dimension, as a `u64` discriminant suitable for
    /// grouping. For [`DimensionId::Underlying`] an FX/metal pair packs its two
    /// 3-letter codes into the low 48 bits (so FX group identity is **byte-identical**
    /// to the pre-generalization `CcyPair` packing); a cross-asset arm with no
    /// [`CcyPair`](celnet_types::CcyPair) projection hashes its `Underlying` into a
    /// stable `u64`. The org keys widen their `u32` handle.
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
            DimensionId::Underlying => underlying_group_value(&self.underlying),
        }
    }
}

/// Pack an [`Underlying`] into a stable `u64` group key. An FX/metal pair packs its
/// two 3-letter codes into the low 48 bits (the original `CcyPair` packing — FX
/// group identity unchanged); any other arm hashes the full underlying (whose
/// `Hash`/`Eq` is the canonical identity) into a `u64`, with the top bit set so it
/// can never collide with a 48-bit packed pair.
#[must_use]
fn underlying_group_value(u: &Underlying) -> u64 {
    if let Some(pair) = u.as_ccy_pair() {
        let b = pair.base.as_str().as_bytes();
        let q = pair.quote.as_str().as_bytes();
        let mut v: u64 = 0;
        for &x in b.iter().chain(q.iter()) {
            v = (v << 8) | u64::from(x);
        }
        return v;
    }
    use core::hash::{Hash, Hasher};
    let mut h = FnvHasher::new();
    u.hash(&mut h);
    // Set the top bit so a hashed cross-asset key is in a disjoint range from any
    // 48-bit packed FX/metal pair (which never sets bits ≥ 48).
    h.finish() | (1u64 << 63)
}

/// A tiny deterministic FNV-1a hasher: the group key must be **reproducible** across
/// runs/processes, so the cube cannot use the std `RandomState` (per-process seeded).
/// FNV-1a is the standard small deterministic byte hash; collisions across distinct
/// underlyings only conflate two groups (never corrupts a within-group sum), and the
/// 64-bit space makes that vanishingly unlikely for a book's underlying universe.
struct FnvHasher(u64);

impl FnvHasher {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    fn new() -> Self {
        Self(Self::OFFSET)
    }
}

impl core::hash::Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{Ccy, CcyPair, CommodityRef, EquityRef, Metal, MetalPair, Symbol};

    fn key() -> FactKey {
        FactKey {
            trader: TraderId(11),
            book: BookId(22),
            desk: DeskId(33),
            underlying: Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD)),
            location: LocationId(44),
            entity: EntityId(55),
        }
    }

    /// Each org dimension reads ITS OWN key (all five distinct, so any arm swap in
    /// the `group_value` match is caught).
    #[test]
    fn org_dimension_group_values_are_their_own_keys() {
        let k = key();
        assert_eq!(k.group_value(DimensionId::Trader), 11);
        assert_eq!(k.group_value(DimensionId::Book), 22);
        assert_eq!(k.group_value(DimensionId::Desk), 33);
        assert_eq!(k.group_value(DimensionId::Location), 44);
        assert_eq!(k.group_value(DimensionId::Entity), 55);
    }

    /// The FX/metal pair group key is the documented 48-bit two-code byte packing —
    /// pinned against hand-typed ASCII byte values (`'E'=0x45, 'U'=0x55, 'R'=0x52,
    /// 'S'=0x53, 'D'=0x44, 'J'=0x4A, 'P'=0x50, 'Y'=0x59, 'X'=0x58, 'A'=0x41`), so
    /// the shift/or fold and its byte order are all load-bearing. No packed pair
    /// ever sets bits ≥ 48.
    #[test]
    fn fx_pair_group_value_is_the_documented_48bit_packing() {
        let eurusd = key().group_value(DimensionId::Underlying);
        assert_eq!(eurusd, 0x4555_5255_5344, "EURUSD = E U R U S D bytes");
        let mut k = key();
        k.underlying = Underlying::Fx(CcyPair::new(Ccy::USD, Ccy::JPY));
        assert_eq!(
            k.group_value(DimensionId::Underlying),
            0x5553_444A_5059,
            "USDJPY = U S D J P Y bytes"
        );
        // A metal pair packs through the SAME projection (XAUUSD byte-identical to
        // its pre-generalization CcyPair packing).
        k.underlying = Underlying::Metal(MetalPair::new(Metal::Gold, Ccy::USD));
        assert_eq!(
            k.group_value(DimensionId::Underlying),
            0x5841_5555_5344,
            "XAUUSD = X A U U S D bytes"
        );
        assert!(eurusd < (1u64 << 48), "packed pairs never reach bit 48");
    }

    /// A cross-asset (no-pair) underlying groups by the deterministic FNV-1a hash
    /// with the top bit forced — frozen-bits pins (the house `fx_byte_identity`
    /// pattern: captured once from the unmutated build) make the hash constants,
    /// the fold arithmetic and the top-bit force all load-bearing; the disjointness
    /// and same-key-same-group contracts are asserted structurally.
    #[test]
    fn cross_asset_group_value_is_stable_and_disjoint() {
        let mut k = key();
        k.underlying = Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD));
        let acme = k.group_value(DimensionId::Underlying);
        let mut k2 = key();
        k2.underlying = Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD));
        // Deterministic: the same underlying always lands in the same group.
        assert_eq!(acme, k2.group_value(DimensionId::Underlying));
        // Disjoint from every 48-bit packed pair: the top bit is forced.
        assert!(acme & (1u64 << 63) != 0, "cross-asset keys must set bit 63");
        // A different symbol is a different group (the hash genuinely hashes).
        k2.underlying = Underlying::Equity(EquityRef::new(Symbol::new("OTHR", ""), Ccy::USD));
        let othr = k2.group_value(DimensionId::Underlying);
        assert_ne!(acme, othr);
        // Frozen-bits pins (captured from the unmutated FNV-1a fold; a change to
        // the offset/prime/xor/multiply or the top-bit force shifts these).
        let mut k3 = key();
        k3.underlying =
            Underlying::Commodity(CommodityRef::new(Symbol::new("BRENT", ""), Ccy::USD));
        let brent = k3.group_value(DimensionId::Underlying);
        assert_eq!(acme, ACME_USD_GROUP, "ACME/USD frozen group key");
        assert_eq!(brent, BRENT_USD_GROUP, "BRENT/USD frozen group key");
        assert_ne!(acme, brent);
    }

    /// Frozen group keys for the cross-asset hash pins above (captured once from
    /// the unmutated build on this toolchain; deterministic by construction).
    const ACME_USD_GROUP: u64 = 0xD095_3180_2EA3_7AB8;
    const BRENT_USD_GROUP: u64 = 0x8CB6_F3CC_498F_509E;

    /// Parent pointers resolve, supersede on re-set, and answer `None` when
    /// unconfigured — for BOTH chains (book→desk, location→entity).
    #[test]
    fn hierarchy_parent_pointers_resolve_and_supersede() {
        let mut h = Hierarchy::new();
        assert_eq!(h.desk_of(BookId(1)), None);
        assert_eq!(h.entity_of(LocationId(1)), None);

        h.set_book_desk(BookId(1), DeskId(10));
        h.set_book_desk(BookId(2), DeskId(20));
        assert_eq!(h.desk_of(BookId(1)), Some(DeskId(10)));
        assert_eq!(h.desk_of(BookId(2)), Some(DeskId(20)));
        // Re-setting an existing book REPLACES (one live mapping per key — a
        // push-instead-of-replace mutant would keep answering 10 via first-match).
        h.set_book_desk(BookId(1), DeskId(11));
        assert_eq!(h.desk_of(BookId(1)), Some(DeskId(11)));
        assert_eq!(
            h.desk_of(BookId(2)),
            Some(DeskId(20)),
            "other keys untouched"
        );
        assert_eq!(h.desk_of(BookId(3)), None);

        h.set_location_entity(LocationId(7), EntityId(70));
        h.set_location_entity(LocationId(8), EntityId(80));
        assert_eq!(h.entity_of(LocationId(7)), Some(EntityId(70)));
        assert_eq!(h.entity_of(LocationId(8)), Some(EntityId(80)));
        h.set_location_entity(LocationId(8), EntityId(81));
        assert_eq!(h.entity_of(LocationId(8)), Some(EntityId(81)));
        assert_eq!(h.entity_of(LocationId(9)), None);
    }

    /// The interned-handle plumbing: `raw()` and `From<u32>` are the identity on
    /// the handle for every key type.
    #[test]
    fn interned_handles_round_trip() {
        assert_eq!(PositionId::from(7).raw(), 7);
        assert_eq!(TraderId::from(8).raw(), 8);
        assert_eq!(BookId::from(9).raw(), 9);
        assert_eq!(DeskId::from(10).raw(), 10);
        assert_eq!(LocationId::from(11).raw(), 11);
        assert_eq!(EntityId::from(12).raw(), 12);
    }
}
