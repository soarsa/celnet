//! Cross-shard **fixed-income rates** risk fan-out — the *purely additive*
//! analogue of the options reducer in [`crate`] (`docs/RISK-HIERARCHY.md` §3.4,
//! `docs/SCALE-OUT.md` §2/§11; `docs/fixed-income/FI-ARCHITECTURE.md`).
//!
//! # What this module is
//!
//! Where the options reducer ([`crate::FleetReducer`]) threads a vega-pillar map
//! through every roll-up and carries a **non-additive re-gather** (VaR/ES,
//! FRTB-SbM curvature must be re-derived once over the union of constituents),
//! rates portfolio risk is **purely additive**: PV, PV01, DV01 and the key-rate
//! ladder of a netting set are sums of their constituents, with **no** non-linear
//! firm functional to re-derive. (The OIS par rate is curve-derived, not a
//! position measure, so it is *never* aggregated — it has no meaning at the firm
//! node and is deliberately absent here.) The measure semantics mirror
//! [`celnet_rates::OisRisk`](../../celnet_rates/risk/struct.OisRisk.html) exactly:
//! `pv` (base-curve PV), `pv01` (analytic annuity risk), `dv01` (parallel
//! calibrating-quote bump) and a per-tenor `key_rate_ladder` whose buckets each
//! map to a tradeable hedge instrument tenor (the client wire carries these as
//! `OisPillar { tenor_years, .. }`, hence [`KeyRateBucket::tenor_years`]).
//!
//! Because the only reduction is additive, the fan-out is *one* associative,
//! commutative merge — and it equals the single-node roll-up **bit-for-bit**, not
//! merely to tolerance. That exactness is the headline invariant this module
//! proves.
//!
//! # The exact-reconciliation mechanism (the crux)
//!
//! Floating-point addition is **commutative but not associative**, so a naive
//! "sum each shard's scalar, then add the shard scalars" reduction would differ
//! from the single-node sum in the last bit as soon as one currency's risk splits
//! across ≥3 shards. We make the fan-out **bit-identical** to the single-node
//! roll-up by pinning a **fixed summation order that is a pure function of the
//! facts, never of the shard layout**:
//!
//! - The partition key is `(entity, ccy)` ([`rates_partition_key_of`]), so every
//!   `(entity, ccy)` **cell** is wholly co-resident on one shard (the §2
//!   co-residency guarantee). A cell's scalar/ladder sub-sum is therefore built
//!   from the **identical** fact subsequence whether computed single-node or on
//!   its owning shard — bit-identical by construction.
//! - A [`RatesNodeAggregate`] retains its constituent cells keyed by `entity` in a
//!   [`BTreeMap`], and its net scalars / ladder are **folded over the cells in
//!   ascending-entity order** ([`RatesNodeAggregate::finalize`]). Merging two
//!   aggregates unions their (disjoint, by co-residency) cell maps and re-folds —
//!   so the result is **independent of merge order**: associative *and*
//!   commutative bit-for-bit. The fixed ascending-`(ccy, entity, tenor)` fold
//!   order is the "fixed summation order" the invariant relies on.
//!
//! Hence [`RatesFleetReducer::fan_in_additive`] over any HRW sharding equals
//! [`firm_aggregate_rates`] over the whole book **exactly** (proven by the
//! property test, over randomized portfolios and replica sets).
//!
//! # Honest scope
//!
//! This is the **reducer algebra + sharded roll-up + the proven invariant** — the
//! same in-process logical-shard simulation as [`crate`] (no sockets/RPC; the
//! physical cross-node transport is designed-only, `docs/SCALE-OUT.md` §0). The
//! server-owned RPC endpoint that drives a position store through this reducer is
//! a separate integration slice. Provenance lives in prose only; no method/vendor
//! name appears in any identifier (guardrail #8).

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use celnet_risk_cube::{BookId, EntityId};
use celnet_router::{PartitionKey, PartitionMap, ReplicaId, RouteError, TenantId};
use celnet_types::Ccy;

use crate::additive::{AdditiveAggregate, fan_in_additive_seq};

/// One bucket of a key-rate (instrument-Jacobian) ladder: the DV01 attributable
/// to a single calibrating-instrument **tenor**, in PV (settlement-currency)
/// terms.
///
/// `tenor_years` is the maturity-in-years label of the calibrating quote the
/// bucket hedges (the wire `OisPillar::tenor_years`); buckets are merged across a
/// netting set **by tenor** so each surviving bucket maps to one tradeable hedge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyRateBucket {
    /// The calibrating-instrument tenor, in whole years (the ladder's bucket key).
    pub tenor_years: u32,
    /// The DV01 of a +1bp bump of that tenor's quote, in settlement-currency PV.
    pub dv01: f64,
}

/// The aggregation key of one priced rates instrument's risk contribution: the
/// `(legal-entity, settlement-currency, book)` cell it nets into.
///
/// Mirrors the options [`celnet_risk_cube::FactKey`] dimension handles — `entity`
/// and `book` are the cube's interned [`EntityId`]/[`BookId`]; the FX `ccy-pair`
/// axis is replaced by the rates **settlement currency** [`Ccy`]. Partitioning
/// keys on `(entity, ccy)` (see [`rates_partition_key_of`]); `book` is carried for
/// netting/attribution but is deliberately **not** in the partition key, so a
/// currency's whole cross-book risk stays co-resident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RatesFactKey {
    /// The legal entity (regulatory-capital unit) the position books into.
    pub entity: EntityId,
    /// The settlement currency of the instrument's PV/risk.
    pub ccy: Ccy,
    /// The netting book the position sits in.
    pub book: BookId,
}

/// One priced rates instrument's additive risk contribution.
///
/// The measures carry [`celnet_rates::OisRisk`](../../celnet_rates/risk/struct.OisRisk.html)
/// semantics: `pv` is the base-curve present value, `pv01` the analytic annuity
/// risk, `dv01` the parallel calibrating-quote bump, and `key_rate_ladder` the
/// per-tenor instrument Jacobian (which sums, to first order, to `dv01`).
#[derive(Debug, Clone, PartialEq)]
pub struct RatesRiskFact {
    /// The `(entity, ccy, book)` cell this contribution nets into.
    pub key: RatesFactKey,
    /// Present value on the base (unbumped) curve, in settlement currency.
    pub pv: f64,
    /// Analytic PV01 (`N · annuity · 1bp`), in settlement-currency PV.
    pub pv01: f64,
    /// Parallel DV01 (every calibrating quote bumped +1bp), in settlement-currency PV.
    pub dv01: f64,
    /// Per-tenor key-rate DV01 ladder (the tradeable instrument Jacobian).
    pub key_rate_ladder: Vec<KeyRateBucket>,
}

/// The additive sub-sum of one co-resident `(entity, ccy)` cell — the atomic unit
/// of the fixed-order fold. Kept private; callers see only the finalized
/// [`RatesNodeAggregate`].
#[derive(Debug, Clone, Default, PartialEq)]
struct EntityCell {
    pv: f64,
    pv01: f64,
    dv01: f64,
    /// Per-tenor DV01, keyed ascending by tenor for a deterministic fold.
    ladder: BTreeMap<u32, f64>,
}

impl EntityCell {
    /// Fold one fact into this cell (called in fact-input order; a cell's facts
    /// are co-resident, so single-node and shard fold the identical subsequence).
    fn add_fact(&mut self, fact: &RatesRiskFact) {
        self.pv += fact.pv;
        self.pv01 += fact.pv01;
        self.dv01 += fact.dv01;
        for bucket in &fact.key_rate_ladder {
            *self.ladder.entry(bucket.tenor_years).or_default() += bucket.dv01;
        }
    }

    /// Fold another cell's sub-sums into this one (the same-entity overlap case;
    /// never exercised across shards, where entities are disjoint by co-residency).
    fn merge_from(&mut self, other: &EntityCell) {
        self.pv += other.pv;
        self.pv01 += other.pv01;
        self.dv01 += other.dv01;
        for (&tenor, &dv01) in &other.ladder {
            *self.ladder.entry(tenor).or_default() += dv01;
        }
    }
}

/// The per-currency additive roll-up of a rates netting set.
///
/// The public fields are the finalized firm-node measures; the private `cells`
/// map (keyed by `entity`) is the **fixed-order fold mechanism** that makes
/// [`merge_additive`](Self::merge_additive) associative and commutative
/// bit-for-bit (see the module-level "exact-reconciliation mechanism"). A single
/// aggregate is per **one** currency — multi-currency books roll up into a
/// [`RatesFirmRollup`] (one aggregate per currency), so currencies never
/// cross-contaminate.
#[derive(Debug, Clone, PartialEq)]
pub struct RatesNodeAggregate {
    /// The settlement currency this aggregate nets.
    pub ccy: Ccy,
    /// Net present value over the netting set, in settlement currency.
    pub net_pv: f64,
    /// Net analytic PV01.
    pub net_pv01: f64,
    /// Net parallel DV01.
    pub net_dv01: f64,
    /// The merged key-rate ladder, bucketed by tenor and sorted ascending.
    pub key_rate_ladder: Vec<KeyRateBucket>,
    /// Constituent `(entity)` cells, ascending by entity handle — the fixed fold
    /// order behind exact, shard-order-independent reconciliation.
    cells: BTreeMap<u32, EntityCell>,
}

impl RatesNodeAggregate {
    /// The **empty** aggregate for `ccy` — a netting set holding no facts (a fresh
    /// book): every measure `0`, an empty ladder, no cells. The FI counterpart to
    /// `celnet_risk_cube::NodeAggregate::empty`, and the identity of
    /// [`merge_additive`](Self::merge_additive).
    ///
    /// Exists so a caller that resolves *no* facts for a scope has an honest empty
    /// aggregate to classify against, rather than having to fabricate one or skip the
    /// limit check (a skipped check is an inert limit — the failure mode
    /// `docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §2.3 Defect 2 documents).
    #[must_use]
    pub fn empty(ccy: Ccy) -> Self {
        Self::finalize(ccy, BTreeMap::new())
    }

    /// Fold a cell map into the finalized firm-node measures by summing the cells
    /// in **ascending-entity order** (and, within the ladder, ascending tenor) —
    /// the deterministic order both the single-node and the fan-in paths share.
    #[must_use]
    fn finalize(ccy: Ccy, cells: BTreeMap<u32, EntityCell>) -> Self {
        let mut net_pv = 0.0;
        let mut net_pv01 = 0.0;
        let mut net_dv01 = 0.0;
        let mut ladder: BTreeMap<u32, f64> = BTreeMap::new();
        // BTreeMap::values iterates ascending key (entity handle): the fixed fold.
        for cell in cells.values() {
            net_pv += cell.pv;
            net_pv01 += cell.pv01;
            net_dv01 += cell.dv01;
            for (&tenor, &dv01) in &cell.ladder {
                *ladder.entry(tenor).or_default() += dv01;
            }
        }
        let key_rate_ladder = ladder
            .into_iter()
            .map(|(tenor_years, dv01)| KeyRateBucket { tenor_years, dv01 })
            .collect();
        Self {
            ccy,
            net_pv,
            net_pv01,
            net_dv01,
            key_rate_ladder,
            cells,
        }
    }

    /// **Additive merge** of two same-currency aggregates: union their (disjoint,
    /// by co-residency) cell maps and re-fold in the fixed ascending-entity order.
    ///
    /// Because the result is always re-folded over the merged cells in their fixed
    /// order, this is associative **and** commutative **bit-for-bit** — the fan-in
    /// is independent of the order shards are visited and equals the single-node
    /// roll-up exactly.
    ///
    /// The two aggregates must share a currency (the firm rollup only ever merges
    /// matching-currency nodes); a cross-currency merge is a caller bug.
    #[must_use]
    pub fn merge_additive(self, other: &Self) -> Self {
        debug_assert_eq!(
            self.ccy, other.ccy,
            "merge_additive must not net across currencies"
        );
        let mut cells = self.cells;
        for (&entity, cell) in &other.cells {
            match cells.entry(entity) {
                // Disjoint across shards (co-residency) — exact clone, bit-identical.
                Entry::Vacant(v) => {
                    v.insert(cell.clone());
                }
                // Same-entity overlap (not reached cross-shard) — sum sub-sums.
                Entry::Occupied(mut o) => o.get_mut().merge_from(cell),
            }
        }
        Self::finalize(self.ccy, cells)
    }
}

/// A firm-level rates roll-up: one [`RatesNodeAggregate`] per settlement currency,
/// in ascending-currency order (deterministic).
///
/// Per-currency partitioning is the rates analogue of the options firm node: PV is
/// currency-denominated, so summing PV across currencies is meaningless — each
/// currency nets in isolation.
#[derive(Debug, Clone, PartialEq)]
pub struct RatesFirmRollup {
    /// Per-currency aggregates, ascending by currency code.
    books: Vec<RatesNodeAggregate>,
}

/// A `[u8; 3]` ordering key for a currency (lexicographic = ISO-code order), so
/// per-currency books sort deterministically without requiring `Ccy: Ord`.
#[must_use]
fn ccy_order_key(ccy: Ccy) -> [u8; 3] {
    let b = ccy.as_str().as_bytes();
    [b[0], b[1], b[2]]
}

impl RatesFirmRollup {
    /// The empty roll-up (no currencies).
    #[must_use]
    pub fn empty() -> Self {
        Self { books: Vec::new() }
    }

    /// The per-currency aggregates, ascending by currency code.
    #[must_use]
    pub fn books(&self) -> &[RatesNodeAggregate] {
        &self.books
    }

    /// The aggregate for `ccy`, if this roll-up nets that currency.
    #[must_use]
    pub fn book(&self, ccy: Ccy) -> Option<&RatesNodeAggregate> {
        let key = ccy_order_key(ccy);
        self.books.iter().find(|b| ccy_order_key(b.ccy) == key)
    }

    /// Roll a slice of facts up directly into per-currency aggregates — grouped by
    /// `ccy`, then by `(entity)` cell, folded in the fixed order. This is the
    /// single-node ground truth ([`firm_aggregate_rates`]) and a shard's local
    /// roll-up ([`RatesLogicalShard::local_aggregate`]) — the same builder, so a
    /// shard's per-cell sub-sums are bit-identical to the single-node node's.
    #[must_use]
    fn from_facts(facts: &[RatesRiskFact]) -> Self {
        // ccy-order-key -> (ccy, entity-cells). BTreeMap pins ascending currency.
        let mut by_ccy: BTreeMap<[u8; 3], (Ccy, BTreeMap<u32, EntityCell>)> = BTreeMap::new();
        for fact in facts {
            let entry = by_ccy
                .entry(ccy_order_key(fact.key.ccy))
                .or_insert_with(|| (fact.key.ccy, BTreeMap::new()));
            entry.1.entry(fact.key.entity.0).or_default().add_fact(fact);
        }
        let books = by_ccy
            .into_values()
            .map(|(ccy, cells)| RatesNodeAggregate::finalize(ccy, cells))
            .collect();
        Self { books }
    }

    /// **Additive merge** of two firm roll-ups: merge matching-currency aggregates
    /// via [`RatesNodeAggregate::merge_additive`], carry through currencies present
    /// in only one side. Associative + commutative bit-for-bit (per-currency).
    #[must_use]
    fn merge(self, other: &Self) -> Self {
        let mut by_ccy: BTreeMap<[u8; 3], RatesNodeAggregate> = self
            .books
            .into_iter()
            .map(|b| (ccy_order_key(b.ccy), b))
            .collect();
        for book in &other.books {
            let key = ccy_order_key(book.ccy);
            let merged = match by_ccy.remove(&key) {
                Some(existing) => existing.merge_additive(book),
                None => book.clone(),
            };
            by_ccy.insert(key, merged);
        }
        Self {
            books: by_ccy.into_values().collect(),
        }
    }
}

/// The fixed-income additive aggregate joins the **one class-parametric additive
/// fan-in seam** ([`AdditiveAggregate`], the additive-side complement of the C2c
/// non-additive cube unification): `combine` is the existing fixed-order per-currency
/// [`RatesFirmRollup::merge`], so the shared [`fan_in_additive_seq`] driver folds the
/// FI net-DV01 / PV01 / key-rate ladder through the **same** path as the options
/// net-Greeks / vega ladder. Its exact-reconciliation contract is unchanged: the
/// re-fold pins the fixed ascending-`(ccy, entity, tenor)` summation order, so the
/// fan-in equals the single-node [`firm_aggregate_rates`] **bit-for-bit** under any
/// sharding (the F5 invariant).
impl AdditiveAggregate for RatesFirmRollup {
    fn combine(self, other: &Self) -> Self {
        self.merge(other)
    }
}

/// The single-node **reference** roll-up over a whole book: per-currency net
/// PV/PV01/DV01 and the tenor-bucketed key-rate ladder, computed directly from the
/// full fact list in the fixed `(ccy, entity, tenor)` summation order.
///
/// This is the ground truth the headline invariant reconciles the sharded fan-in
/// against ([`RatesFleetReducer::fan_in_additive`]).
#[must_use]
pub fn firm_aggregate_rates(facts: &[RatesRiskFact]) -> RatesFirmRollup {
    RatesFirmRollup::from_facts(facts)
}

/// One **in-process logical shard**: the slice of the firm's rates facts the HRW
/// map assigned to a single [`ReplicaId`]. Facts are held in a `Vec` in assignment
/// (input) order so a cell's fold order is deterministic.
#[derive(Debug, Clone)]
pub struct RatesLogicalShard {
    replica: ReplicaId,
    facts: Vec<RatesRiskFact>,
}

impl RatesLogicalShard {
    /// An empty shard owned by `replica`.
    #[must_use]
    pub fn new(replica: ReplicaId) -> Self {
        Self {
            replica,
            facts: Vec::new(),
        }
    }

    /// The replica this shard belongs to.
    #[must_use]
    pub const fn replica(&self) -> ReplicaId {
        self.replica
    }

    /// Read-only access to this shard's facts, in assignment order.
    #[must_use]
    pub fn facts(&self) -> &[RatesRiskFact] {
        &self.facts
    }

    /// Add one fact this shard owns.
    pub fn push(&mut self, fact: RatesRiskFact) {
        self.facts.push(fact);
    }

    /// The number of facts this shard owns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.facts.len()
    }

    /// Whether this shard holds no facts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// This shard's local additive roll-up (per currency). No pillar map: rates
    /// risk is purely additive, so a shard's roll-up needs no extra context.
    #[must_use]
    pub fn local_aggregate(&self) -> RatesFirmRollup {
        RatesFirmRollup::from_facts(&self.facts)
    }
}

/// The fan-out / fan-in **cross-shard reducer** for rates risk.
///
/// Holds the logical shards produced by [`partition_rates_facts`] in **ascending
/// [`ReplicaId`] order** (deterministic reduction order). It is the in-process
/// stand-in for the firm-risk fan-in tier (no cross-node transport — see the
/// module-level honest-scope note).
#[derive(Debug, Clone)]
pub struct RatesFleetReducer {
    /// Shards in ascending replica order.
    shards: Vec<RatesLogicalShard>,
}

impl RatesFleetReducer {
    /// The shards, in ascending replica order.
    #[must_use]
    pub fn shards(&self) -> &[RatesLogicalShard] {
        &self.shards
    }

    /// The number of non-empty logical shards.
    #[must_use]
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    /// The total fact count across all shards (== the input fact count, since the
    /// partition is a disjoint cover).
    #[must_use]
    pub fn total_facts(&self) -> usize {
        self.shards.iter().map(RatesLogicalShard::len).sum()
    }

    /// **Additive fan-out**: each shard rolls up locally, then the firm roll-up is
    /// the [`RatesFirmRollup::merge`] of every shard's roll-up, in ascending owner
    /// order. Associative + commutative bit-for-bit, so the result equals
    /// [`firm_aggregate_rates`] over the union of facts **exactly** (proven by the
    /// invariant test) — independent of the sharding.
    ///
    /// Delegates to the **one class-parametric additive fan-in driver**
    /// ([`fan_in_additive_seq`]) — the single additive-aggregation path shared with
    /// the options reducer ([`crate::FleetReducer::fan_in_additive`]); the FI
    /// aggregate's fixed-order [`RatesFirmRollup::merge`] rides through it as its
    /// [`AdditiveAggregate::combine`]. Byte-identical to the prior hand-written left
    /// fold (same `merge`, same ascending-owner order).
    #[must_use]
    pub fn fan_in_additive(&self) -> RatesFirmRollup {
        fan_in_additive_seq(
            self.shards.iter().map(RatesLogicalShard::local_aggregate),
            RatesFirmRollup::empty,
        )
    }
}

/// The partition key of a rates fact: its `(entity, ccy)` cell, mirroring the
/// options `(entity, pair)` key ([`crate::partition_key_of`]) with the settlement
/// **currency** as the primary subject and the legal entity folded in as the
/// router tenant sub-key.
///
/// The currency is the primary partition subject (all tenors/books of a currency's
/// rates book co-resident, so curve rebuild never crosses a shard — the §2
/// co-residency guarantee); the entity sub-shards it per booking entity while
/// keeping each `(entity, ccy)` cell whole.
#[must_use]
pub fn rates_partition_key_of(fact: &RatesRiskFact) -> PartitionKey {
    PartitionKey::currency(fact.key.ccy).with_tenant(TenantId(u64::from(fact.key.entity.0)))
}

/// Partition a firm's rates facts across the HRW `map` into [`RatesLogicalShard`]s,
/// one per owning [`ReplicaId`], by the `(entity, ccy)` strategy.
///
/// Each fact is routed by its natural HRW owner and pushed into that owner's shard;
/// the result is a **disjoint cover** (every fact on exactly one shard, the union
/// is the input, each `(entity, ccy)` cell co-resident). Shards are returned sorted
/// by [`ReplicaId`] for deterministic reduction. Health is ignored for assignment —
/// the aggregation tier reads each shard's owned slice regardless of live health.
///
/// # Errors
/// [`RouteError::EmptySet`] if `map`'s membership cannot route a fact (empty set).
pub fn partition_rates_facts(
    facts: &[RatesRiskFact],
    map: &PartitionMap<'_>,
) -> Result<RatesFleetReducer, RouteError> {
    let mut shards: Vec<RatesLogicalShard> = Vec::new();
    for fact in facts {
        let owner = map
            .natural_owner(rates_partition_key_of(fact))
            .ok_or(RouteError::EmptySet)?;
        let idx = match shards.iter().position(|s| s.replica() == owner) {
            Some(i) => i,
            None => {
                shards.push(RatesLogicalShard::new(owner));
                shards.len() - 1
            }
        };
        shards[idx].push(fact.clone());
    }
    // Deterministic reduction order: ascending replica id.
    shards.sort_by_key(|s| s.replica().0);
    Ok(RatesFleetReducer { shards })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_router::{PartitionMap, Replica, ReplicaSet};
    use proptest::prelude::*;

    fn ccy(code: &str) -> Ccy {
        Ccy::parse(code).expect("valid ccy")
    }

    fn bucket(tenor_years: u32, dv01: f64) -> KeyRateBucket {
        KeyRateBucket { tenor_years, dv01 }
    }

    fn fact(
        entity: u32,
        c: Ccy,
        book: u32,
        pv: f64,
        pv01: f64,
        dv01: f64,
        ladder: Vec<KeyRateBucket>,
    ) -> RatesRiskFact {
        RatesRiskFact {
            key: RatesFactKey {
                entity: EntityId(entity),
                ccy: c,
                book: BookId(book),
            },
            pv,
            pv01,
            dv01,
            key_rate_ladder: ladder,
        }
    }

    fn replicas(ids: &[u64]) -> ReplicaSet {
        ReplicaSet::new(ids.iter().map(|&i| Replica::up(ReplicaId(i))).collect()).unwrap()
    }

    /// A non-trivial multi-entity / multi-currency rates book: 3 entities × 3
    /// currencies, with **overlapping** tenor ladders (so buckets net across
    /// entities) and **disjoint** ones (so a novel tenor survives standalone).
    fn firm_book() -> Vec<RatesRiskFact> {
        let usd = ccy("USD");
        let eur = ccy("EUR");
        let gbp = ccy("GBP");
        vec![
            // Entity 1 — USD receive-fixed 5y + a USD 2y, both on the 1/2/5y grid.
            fact(
                1,
                usd,
                100,
                12_500.0,
                830.0,
                -829.4,
                vec![bucket(1, -120.0), bucket(2, -250.0), bucket(5, -459.4)],
            ),
            fact(
                1,
                usd,
                101,
                -3_200.0,
                410.0,
                -409.8,
                vec![bucket(1, -90.0), bucket(2, -319.8)],
            ),
            // Entity 2 — USD pay-fixed 10y (offsets entity 1 at the long end), and a
            // EUR book on a partly-overlapping (2/5/7y) grid.
            fact(
                2,
                usd,
                200,
                7_750.0,
                915.0,
                914.2,
                vec![bucket(2, 140.0), bucket(5, 360.0), bucket(10, 414.2)],
            ),
            fact(
                2,
                eur,
                201,
                4_100.0,
                560.0,
                -559.6,
                vec![bucket(2, -160.0), bucket(5, -210.0), bucket(7, -189.6)],
            ),
            // Entity 3 — EUR (same grid as entity 2's EUR → buckets net across
            // entities), and a GBP book on a fully disjoint (3/15y) grid.
            fact(
                3,
                eur,
                300,
                -1_900.0,
                300.0,
                299.7,
                vec![bucket(2, 80.0), bucket(5, 110.0), bucket(7, 109.7)],
            ),
            fact(
                3,
                gbp,
                301,
                6_050.0,
                720.0,
                -719.5,
                vec![bucket(3, -300.0), bucket(15, -419.5)],
            ),
        ]
    }

    /// Bit-for-bit equality of two firm roll-ups: same currencies, same per-book
    /// scalars (compared on their raw bits), same ladders bucket-for-bucket.
    fn assert_rollup_bit_equal(a: &RatesFirmRollup, b: &RatesFirmRollup) {
        assert_eq!(a.books().len(), b.books().len(), "currency count differs");
        for (x, y) in a.books().iter().zip(b.books()) {
            assert_eq!(x.ccy, y.ccy, "currency order differs");
            assert_eq!(
                x.net_pv.to_bits(),
                y.net_pv.to_bits(),
                "net_pv: {} vs {}",
                x.net_pv,
                y.net_pv
            );
            assert_eq!(x.net_pv01.to_bits(), y.net_pv01.to_bits(), "net_pv01");
            assert_eq!(x.net_dv01.to_bits(), y.net_dv01.to_bits(), "net_dv01");
            assert_eq!(
                x.key_rate_ladder.len(),
                y.key_rate_ladder.len(),
                "ladder length for {}",
                x.ccy
            );
            for (bx, by) in x.key_rate_ladder.iter().zip(&y.key_rate_ladder) {
                assert_eq!(bx.tenor_years, by.tenor_years, "ladder tenor order");
                assert_eq!(
                    bx.dv01.to_bits(),
                    by.dv01.to_bits(),
                    "ladder dv01 at {}y: {} vs {}",
                    bx.tenor_years,
                    bx.dv01,
                    by.dv01
                );
            }
        }
    }

    /// **THE HEADLINE INVARIANT — fan-out == single-node, EXACTLY.** A
    /// multi-entity, multi-currency book partitioned across a 5-replica HRW map
    /// fans in **bit-for-bit identically** to the single-node `firm_aggregate_rates`
    /// over the whole book. Genuinely fans across ≥2 shards.
    #[test]
    fn fan_out_equals_single_node_exactly() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let map = PartitionMap::new(&set);

        let single = firm_aggregate_rates(&facts);
        let reducer = partition_rates_facts(&facts, &map).unwrap();
        let fleet = reducer.fan_in_additive();

        assert!(
            reducer.shard_count() >= 2,
            "expected the book to fan across >=2 shards, got {}",
            reducer.shard_count()
        );
        assert_rollup_bit_equal(&fleet, &single);

        // Sanity: the roll-up actually nets the three currencies and overlapping
        // EUR buckets summed across entities (2y: -160 + 80 = -80).
        assert_eq!(single.books().len(), 3);
        let eur = single.book(ccy("EUR")).expect("EUR book");
        let two_y = eur
            .key_rate_ladder
            .iter()
            .find(|b| b.tenor_years == 2)
            .expect("EUR 2y bucket");
        assert!(
            (two_y.dv01 - (-80.0)).abs() < 1e-9,
            "EUR 2y dv01 {}",
            two_y.dv01
        );
    }

    /// The pre-unification hand-written left fold, reconstructed inline: seed with
    /// the first shard's local roll-up, then [`RatesFirmRollup::merge`] each
    /// subsequent shard in ascending-owner order. The reference the unified driver
    /// must reproduce bit-for-bit.
    fn hand_fold(reducer: &RatesFleetReducer) -> RatesFirmRollup {
        let mut iter = reducer.shards().iter();
        let Some(first) = iter.next() else {
            return RatesFirmRollup::empty();
        };
        let mut acc = first.local_aggregate();
        for shard in iter {
            acc = acc.merge(&shard.local_aggregate());
        }
        acc
    }

    /// **UNIFIED DRIVER — FI parity oracle (non-circular).** After folding the
    /// fixed-income roll-up through the shared class-parametric fan-in driver
    /// ([`fan_in_additive_seq`], behind [`RatesFleetReducer::fan_in_additive`]), the
    /// result is (1) **bit-for-bit** equal to the pre-unification hand-written left
    /// fold over the same shards (behavior preservation — same `merge`, same order),
    /// and (2) **bit-for-bit** equal to the INDEPENDENT single-node reference
    /// [`firm_aggregate_rates`] (a direct `from_facts` fold that never runs the
    /// sharded reducer — the F5 invariant as a non-circular oracle that the refactor
    /// is exact).
    #[test]
    fn unified_fan_in_equals_prior_hand_fold_and_single_node() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5]);
        let map = PartitionMap::new(&set);
        let reducer = partition_rates_facts(&facts, &map).unwrap();
        assert!(
            reducer.shard_count() >= 2,
            "must genuinely fan across >=2 shards, got {}",
            reducer.shard_count()
        );

        let unified = reducer.fan_in_additive();

        // (1) Behavior preservation vs the prior hand fold.
        assert_rollup_bit_equal(&unified, &hand_fold(&reducer));

        // (2) Still exact against the independent single-node reference.
        assert_rollup_bit_equal(&unified, &firm_aggregate_rates(&facts));
    }

    /// **Sharding is a disjoint cover.** Every fact lands on exactly one shard;
    /// `total_facts == input len`; shards are in ascending replica order.
    #[test]
    fn partition_is_disjoint_cover() {
        let facts = firm_book();
        let set = replicas(&[1, 2, 3, 4, 5, 6, 7]);
        let map = PartitionMap::new(&set);
        let reducer = partition_rates_facts(&facts, &map).unwrap();

        // Total preserved.
        assert_eq!(reducer.total_facts(), facts.len());

        // Ascending replica order.
        let mut prev = 0u64;
        for shard in reducer.shards() {
            assert!(shard.replica().0 >= prev, "shards not ascending by replica");
            prev = shard.replica().0;
        }

        // Each fact re-routes to exactly the shard it was placed on (disjoint).
        for fact in &facts {
            let owner = map.natural_owner(rates_partition_key_of(fact)).unwrap();
            let hosting: Vec<ReplicaId> = reducer
                .shards()
                .iter()
                .filter(|s| s.facts().contains(fact))
                .map(RatesLogicalShard::replica)
                .collect();
            assert_eq!(hosting, vec![owner], "fact must live on exactly its owner");
        }
    }

    /// **Ladder bucketing by tenor.** Same-tenor contributions sum into one bucket;
    /// distinct tenors stay separate and ascending; a novel tenor appends.
    #[test]
    fn ladder_buckets_by_tenor() {
        let usd = ccy("USD");
        let facts = vec![
            fact(
                1,
                usd,
                1,
                0.0,
                0.0,
                0.0,
                vec![bucket(2, 10.0), bucket(5, 20.0)],
            ),
            // Same entity, same tenors (2,5) → sum within the cell; plus a novel 10y.
            fact(
                1,
                usd,
                1,
                0.0,
                0.0,
                0.0,
                vec![bucket(2, 3.0), bucket(5, 7.0), bucket(10, 99.0)],
            ),
            // Different entity, overlapping 2y → nets across entities.
            fact(2, usd, 1, 0.0, 0.0, 0.0, vec![bucket(2, 1.5)]),
        ];
        let agg = firm_aggregate_rates(&facts);
        let usd_book = agg.book(usd).expect("USD book");
        let ladder = &usd_book.key_rate_ladder;

        // Tenors ascending and de-duplicated: {2, 5, 10}.
        let tenors: Vec<u32> = ladder.iter().map(|b| b.tenor_years).collect();
        assert_eq!(tenors, vec![2, 5, 10]);

        let dv01_at = |t: u32| ladder.iter().find(|b| b.tenor_years == t).unwrap().dv01;
        assert!((dv01_at(2) - (10.0 + 3.0 + 1.5)).abs() < 1e-12);
        assert!((dv01_at(5) - (20.0 + 7.0)).abs() < 1e-12);
        assert!((dv01_at(10) - 99.0).abs() < 1e-12);
    }

    /// **Per-currency isolation.** Facts of different currencies never
    /// cross-contaminate — each nets into its own book, and a single-currency
    /// subset reconciles independently of the others.
    #[test]
    fn currencies_do_not_cross_contaminate() {
        let usd = ccy("USD");
        let jpy = ccy("JPY");
        let facts = vec![
            fact(1, usd, 1, 100.0, 10.0, -9.0, vec![bucket(2, -9.0)]),
            fact(1, jpy, 1, 5_000.0, 700.0, -699.0, vec![bucket(5, -699.0)]),
        ];
        let agg = firm_aggregate_rates(&facts);
        assert_eq!(agg.books().len(), 2);

        let usd_book = agg.book(usd).unwrap();
        assert!((usd_book.net_pv - 100.0).abs() < 1e-12);
        assert_eq!(usd_book.key_rate_ladder.len(), 1);

        let jpy_book = agg.book(jpy).unwrap();
        assert!((jpy_book.net_pv - 5_000.0).abs() < 1e-12);
        assert_eq!(jpy_book.key_rate_ladder.len(), 1);

        // The USD book equals a roll-up of the USD subset alone (no JPY leakage).
        let usd_only = firm_aggregate_rates(&[facts[0].clone()]);
        assert_rollup_bit_equal(
            &RatesFirmRollup {
                books: vec![usd_book.clone()],
            },
            &usd_only,
        );
    }

    // ---- Property test: exact reconciliation over randomized portfolios ----

    /// A bounded, finite measure so summation reasoning has no NaN/inf corner.
    fn money() -> impl Strategy<Value = f64> {
        -1.0e7f64..1.0e7f64
    }

    prop_compose! {
        fn arb_bucket()(tenor in prop::sample::select(vec![1u32, 2, 3, 5, 7, 10, 15, 20]),
                        dv01 in money()) -> KeyRateBucket {
            KeyRateBucket { tenor_years: tenor, dv01 }
        }
    }

    prop_compose! {
        fn arb_fact()(
            entity in 0u32..4,
            ccy_ix in 0usize..3,
            book in 0u32..3,
            pv in money(),
            pv01 in money(),
            dv01 in money(),
            ladder in prop::collection::vec(arb_bucket(), 0..5),
        ) -> RatesRiskFact {
            let c = [Ccy::USD, Ccy::EUR, Ccy::GBP][ccy_ix];
            fact(entity, c, book, pv, pv01, dv01, ladder)
        }
    }

    proptest! {
        /// **EXACT reconciliation over random portfolios and shardings.** For any
        /// portfolio (≤24 facts over 4 entities × 3 currencies × random tenor
        /// ladders) partitioned across any replica set (2..8 replicas), the sharded
        /// `fan_in_additive` equals the single-node `firm_aggregate_rates`
        /// **bit-for-bit**. This is the headline invariant generalized: no sharding
        /// ever perturbs a single bit of the firm roll-up.
        #[test]
        fn fan_out_equals_single_node_prop(
            facts in prop::collection::vec(arb_fact(), 0..24),
            n_replicas in 2usize..8,
        ) {
            let ids: Vec<u64> = (1..=n_replicas as u64).collect();
            let set = replicas(&ids);
            let map = PartitionMap::new(&set);

            let single = firm_aggregate_rates(&facts);
            let reducer = partition_rates_facts(&facts, &map).unwrap();
            prop_assert_eq!(reducer.total_facts(), facts.len());

            let fleet = reducer.fan_in_additive();
            assert_rollup_bit_equal(&fleet, &single);
        }
    }
}
