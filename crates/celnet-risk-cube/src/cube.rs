//! The single-node OLAP fact store + group-by/reduce engine
//! (`docs/RISK-HIERARCHY.md` §2.1/§2.5/§3.2).
//!
//! [`Cube`] holds the immutable `RiskFact` table (one current fact per position),
//! the org [`Hierarchy`], and serves two operations:
//!
//! 1. **Additive roll-up** ([`Cube::group_by`]) — group facts by any single
//!    dimension at any level and sum their canonical Greeks + vega ladder. This is
//!    the cheap, associative OLAP reduction (§2.5).
//! 2. **Non-additive re-derivation** ([`Cube::node_var_es`],
//!    [`Cube::node_var_es_sensitivity`], [`Cube::node_curvature_spot`],
//!    [`Cube::node_correlation_weighted_vega`]) — re-derive the non-additive
//!    reducers over a node's constituent positions (§2.5). These are NOT summed
//!    from children. VaR/ES has two lenses: the exact bump-and-revalue oracle and
//!    the AAD sensitivity scale path (§3.3).
//!
//! Roll-up and drill-down operate over the **same** facts, so a node total is
//! always reconcilable to its constituents (§2.5) — `group_by` returns the leaf
//! facts per group alongside the aggregate, so a drill-down is the same query at a
//! finer dimension.
//!
//! # Single-node scope (honest)
//!
//! This is a **single-node** cube: it aggregates the facts it holds. Distributed
//! **cross-shard reduction** (`docs/RISK-HIERARCHY.md` §3.4) — partition by
//! `(legal_entity, ccy_pair)` over the designed-only `celnet-router` HRW map, with
//! a cross-shard reducer combining additive measures directly and re-deriving
//! non-additive firm measures from shard contributions — is **out of scope**:
//! `celnet-router`'s fleet tier is designed-only per `docs/SCALE-OUT.md` §8. The
//! **shard-merge seam** is [`Cube::merge_additive`], which sums another node's
//! additive aggregate into one of this node's groups; a cross-shard reducer would
//! call it with shard-level aggregates. The non-additive measures cannot merge
//! that way (they need constituent facts), which is precisely the §3.4 caveat that
//! a firm-level VaR/curvature run gathers facts rather than summing shard results.

use celnet_core::ExoticLegPricer;
use celnet_core::carry::CarryPricer;
use celnet_risk_normalize::{CanonicalLeaf, Numeraire, NumeraireError, PositionRisk, SpotResolver};

use crate::additive::{NetGreeks, VegaLadder, VegaPillar};
use crate::dimension::{DimensionId, FactKey, Hierarchy, PositionId, RiskFact};
use crate::exotic::ExoticLeg;
use crate::nonadditive::{
    Scenario, VarEs, correlation_weighted_vega, node_var_es_combined,
    node_var_es_sensitivity_combined, sbm_curvature_spot_combined,
};

/// A mapping from a leaf to its `(tenor × delta)` vega pillar — supplied by the
/// caller so the cube buckets vega onto the desired regulatory or internal pillar
/// grid (the grid is versioned, externally-supplied data per §2.3/§2.11, never
/// compiled in).
pub trait VegaPillarMap {
    /// The vega pillar a leaf's `(time, delta)` maps onto.
    fn pillar_of(&self, leaf: &CanonicalLeaf, position: &PositionRisk) -> VegaPillar;
}

/// The additive aggregate of a node: summed canonical Greeks + the vega ladder,
/// plus the set of constituent facts that produced it (so a node is always
/// reconcilable to its leaves — the §2.5 drill-down invariant).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeAggregate {
    /// The group's key value along the grouping dimension (see
    /// [`FactKey::group_value`]).
    pub group: u64,
    /// The summed canonical Greek set (additive).
    pub net_greeks: NetGreeks,
    /// The vega bucketed by `(tenor × delta)` pillar (additive).
    pub vega_ladder: VegaLadder,
    /// The constituent **vanilla** positions of this node (the re-derivation source
    /// for non-additive measures over vanilla legs, and the drill-down target).
    pub positions: Vec<PositionRisk>,
    /// The constituent **exotic** legs of this node (the re-derivation source for
    /// non-additive measures over exotic legs — re-priced through the real
    /// closed-form exotic pricer, never as a vanilla proxy). Empty for a vanilla-only
    /// node, so existing vanilla behaviour is byte-identical.
    pub exotic_legs: Vec<ExoticLeg>,
    /// The canonical leaves of this node (for a numeraire view / reconciliation).
    /// Carries BOTH vanilla and exotic leaves — an exotic's leaf is its real Greek
    /// set, so the additive roll-up and numeraire view include exotics.
    pub leaves: Vec<CanonicalLeaf>,
}

impl NodeAggregate {
    /// An empty aggregate for `group` — all Greeks zero, no constituents. The public
    /// seam a pre-trade limit check (`celnet-limits`) uses to synthesize the current
    /// node at a scope that has no facts yet (a fresh book) or a scope built outside
    /// the FX cube (the linear-rates roll-up in `celnet-server`'s rates sink), before
    /// the proposed trade's increment is added.
    #[must_use]
    pub fn empty(group: u64) -> Self {
        Self {
            group,
            net_greeks: NetGreeks::zero(),
            vega_ladder: VegaLadder::new(),
            positions: Vec::new(),
            exotic_legs: Vec::new(),
            leaves: Vec::new(),
        }
    }

    /// Express this node's delta-by-ccy-leg, premium and (premium-ccy-correct)
    /// vega in a chosen reporting numeraire — delegating to `celnet-risk-normalize`
    /// (§2.3). This is how a cross-pair node collapses its base-ccy deltas into one
    /// scalar.
    ///
    /// # Errors
    /// [`NumeraireError`] if any currency leg / premium ccy / vega ccy lacks a
    /// conversion rate, or a supplied rate is invalid — the conversion fails
    /// loudly rather than dropping a leg.
    pub fn numeraire_view<R: SpotResolver>(
        &self,
        resolver: &R,
    ) -> Result<Numeraire, NumeraireError> {
        Numeraire::from_leaves(&self.leaves, resolver)
    }

    /// Merge another additive aggregate into this one (the **cross-shard merge
    /// seam**, §3.4). Sums net Greeks and vega ladders and concatenates the
    /// constituent facts. A distributed cross-shard reducer calls this with
    /// shard-level aggregates; non-additive measures must be re-derived from the
    /// merged `positions`, never merged this way.
    pub fn merge_additive(&mut self, other: &NodeAggregate) {
        self.net_greeks = self.net_greeks + other.net_greeks;
        self.vega_ladder.merge(&other.vega_ladder);
        self.positions.extend_from_slice(&other.positions);
        self.exotic_legs.extend_from_slice(&other.exotic_legs);
        self.leaves.extend_from_slice(&other.leaves);
    }
}

/// Accumulate one fact into a node aggregate, routing the additive measures (the
/// canonical leaf's Greeks + vega ladder — uniform for vanilla and exotic, since the
/// leaf already carries the real exotic Greeks) and the **non-additive re-derivation
/// source** (a vanilla fact's `position`, an exotic fact's `exotic` leg). The
/// vega-pillar `(tenor × delta)` is mapped from the fact's `position` metadata, which
/// for an exotic fact is its honest underlying-vanilla bucketing tuple.
fn accumulate_fact<P: VegaPillarMap>(agg: &mut NodeAggregate, fact: &RiskFact, pillars: &P) {
    let leaf = &fact.measure.leaf;
    agg.net_greeks.add_leaf(leaf);
    agg.vega_ladder.add(
        pillars.pillar_of(leaf, &fact.measure.position),
        leaf.greeks.vega,
    );
    agg.leaves.push(leaf.clone());
    match fact.measure.exotic {
        // An exotic fact re-prices through the real closed-form exotic pricer.
        Some(leg) => agg.exotic_legs.push(leg),
        // A vanilla fact re-prices through its asset's leaf (the seam).
        None => agg.positions.push(fact.measure.position.clone()),
    }
}

/// The single-node risk cube: the fact table + hierarchy + group-by/reduce engine.
#[derive(Debug, Clone, Default)]
pub struct Cube {
    facts: Vec<RiskFact>,
    hierarchy: Hierarchy,
}

impl Cube {
    /// An empty cube with an empty hierarchy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty cube wired to a configured org hierarchy.
    #[must_use]
    pub fn with_hierarchy(hierarchy: Hierarchy) -> Self {
        Self {
            facts: Vec::new(),
            hierarchy,
        }
    }

    /// Mutable access to the org hierarchy (to configure book→desk / location→
    /// entity parent pointers).
    pub fn hierarchy_mut(&mut self) -> &mut Hierarchy {
        &mut self.hierarchy
    }

    /// Insert or replace the current fact for a position (append-only semantics
    /// with one live fact per `position_id`; a re-valuation supersedes the prior
    /// fact for the same id).
    pub fn upsert(&mut self, fact: RiskFact) {
        if let Some(slot) = self
            .facts
            .iter_mut()
            .find(|f| f.position_id == fact.position_id)
        {
            *slot = fact;
        } else {
            self.facts.push(fact);
        }
    }

    /// The current fact for a position, if present.
    #[must_use]
    pub fn fact(&self, id: PositionId) -> Option<&RiskFact> {
        self.facts.iter().find(|f| f.position_id == id)
    }

    /// The number of live facts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.facts.len()
    }

    /// Whether the cube holds no facts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// The group value of a fact along `dim`, resolving the parent hierarchy for
    /// the genuine parent chains (a `Desk` group resolves each fact's book→desk;
    /// an `Entity` group resolves location→entity). For dimensions that are direct
    /// keys this is just [`FactKey::group_value`].
    fn group_value(&self, key: &FactKey, dim: DimensionId) -> u64 {
        match dim {
            // Desk may be carried directly on the key OR resolved from the book's
            // parent pointer; prefer the configured parent so a re-org reflows.
            DimensionId::Desk => self
                .hierarchy
                .desk_of(key.book)
                .map_or_else(|| key.group_value(dim), |d| u64::from(d.0)),
            DimensionId::Entity => self
                .hierarchy
                .entity_of(key.location)
                .map_or_else(|| key.group_value(dim), |e| u64::from(e.0)),
            other => key.group_value(other),
        }
    }

    /// **Additive roll-up**: group the facts by `dim` (at its hierarchy level) and
    /// sum the canonical Greeks + vega ladder per group, retaining the constituent
    /// facts for drill-down (`docs/RISK-HIERARCHY.md` §2.5).
    ///
    /// `pillars` maps each leaf onto its `(tenor × delta)` vega bucket. Groups are
    /// returned in first-seen order; the result is deterministic for a fixed fact
    /// insertion order and pillar map.
    pub fn group_by<P: VegaPillarMap>(&self, dim: DimensionId, pillars: &P) -> Vec<NodeAggregate> {
        let mut out: Vec<NodeAggregate> = Vec::new();
        for fact in &self.facts {
            let g = self.group_value(&fact.key, dim);
            let idx = match out.iter().position(|a| a.group == g) {
                Some(i) => i,
                None => {
                    out.push(NodeAggregate::empty(g));
                    out.len() - 1
                }
            };
            let agg = &mut out[idx];
            accumulate_fact(agg, fact, pillars);
        }
        out
    }

    /// The whole-cube (firm) additive aggregate — group-by with a single group.
    pub fn firm_aggregate<P: VegaPillarMap>(&self, pillars: &P) -> NodeAggregate {
        let mut agg = NodeAggregate::empty(0);
        for fact in &self.facts {
            accumulate_fact(&mut agg, fact, pillars);
        }
        agg
    }

    /// **Non-additive**: VaR/ES of a node's constituent positions by full
    /// **bump-and-revalue** — the exact reference / oracle (`docs/RISK-HIERARCHY.md`
    /// §2.5). Re-derived, not summed. O(positions × scenarios) repricings.
    #[must_use]
    pub fn node_var_es<P: CarryPricer>(
        pricer: &P,
        exotic_pricer: &dyn ExoticLegPricer,
        node: &NodeAggregate,
        scenarios: &[Scenario],
        alpha: f64,
    ) -> VarEs {
        node_var_es_combined(
            pricer,
            exotic_pricer,
            &node.positions,
            &node.exotic_legs,
            scenarios,
            alpha,
        )
    }

    /// **Non-additive (scale path)**: VaR/ES of a node by the **AAD sensitivity
    /// lens** (`docs/RISK-HIERARCHY.md` §3.3). Each position's full Greek set is
    /// computed in ONE reverse-mode `celnet_vanilla::adjoint_greeks` sweep, then
    /// every scenario's node P&L is a second-order Taylor expansion — O(positions)
    /// sweeps + O(positions × scenarios) cheap arithmetic, vs [`Cube::node_var_es`]'s
    /// O(positions × scenarios) full repricings.
    ///
    /// Accurate to the truncation error of a 2nd-order series over moderate VaR-scale
    /// shocks; reconcile against [`Cube::node_var_es`] (the oracle) for the exact
    /// tail. See [`crate::nonadditive::sensitivity_var_es`] for the precise regime.
    #[must_use]
    pub fn node_var_es_sensitivity<P: CarryPricer>(
        pricer: &P,
        exotic_pricer: &dyn ExoticLegPricer,
        node: &NodeAggregate,
        scenarios: &[Scenario],
        alpha: f64,
    ) -> VarEs {
        node_var_es_sensitivity_combined(
            pricer,
            exotic_pricer,
            &node.positions,
            &node.exotic_legs,
            scenarios,
            alpha,
        )
    }

    /// **Non-additive**: FRTB-SbM spot curvature of a node by up/down full reprice
    /// net of the linear delta term (`docs/RISK-HIERARCHY.md` §2.5).
    #[must_use]
    pub fn node_curvature_spot<P: CarryPricer>(
        pricer: &P,
        exotic_pricer: &dyn ExoticLegPricer,
        node: &NodeAggregate,
        rw: f64,
    ) -> f64 {
        sbm_curvature_spot_combined(
            pricer,
            exotic_pricer,
            &node.positions,
            &node.exotic_legs,
            rw,
        )
    }

    /// **Non-additive**: the correlation-weighted vega aggregate over a node's
    /// vega-ladder buckets (`docs/RISK-HIERARCHY.md` §2.5). `weighted` maps a
    /// pillar's vega to its risk-weighted value; `rho` is the symmetric
    /// inter-pillar correlation (versioned external data).
    #[must_use]
    pub fn node_correlation_weighted_vega<W, R>(node: &NodeAggregate, weighted: W, rho: R) -> f64
    where
        W: Fn(VegaPillar, f64) -> f64,
        R: Fn(usize, usize) -> f64,
    {
        let weighted_vegas: Vec<f64> = node
            .vega_ladder
            .pillars()
            .map(|(p, v)| weighted(p, v))
            .collect();
        correlation_weighted_vega(&weighted_vegas, rho)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::additive::VegaPillar;
    use crate::dimension::{BookId, DeskId, EntityId, FactKey, LocationId, TraderId};
    use crate::test_support::DigitalTestPricer;
    use celnet_risk_normalize::{AssetPricer, CanonicalGreeks, canonicalize};
    use celnet_types::{
        Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
    };
    use celnet_types::{DigitalKind, ExoticKind};

    struct OnePillar;
    impl VegaPillarMap for OnePillar {
        fn pillar_of(&self, _leaf: &CanonicalLeaf, _pos: &PositionRisk) -> VegaPillar {
            VegaPillar::new(365, 5000)
        }
    }

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn pos(n: f64, vi: VanillaInputs) -> PositionRisk {
        PositionRisk::fx(
            eurusd(),
            OptionType::Call,
            n,
            vi,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    }

    fn fact(id: u32, keys: (u32, u32, u32, u32, u32), p: &PositionRisk) -> RiskFact {
        let (trader, book, desk, location, entity) = keys;
        RiskFact {
            position_id: crate::dimension::PositionId(id),
            key: FactKey {
                trader: TraderId(trader),
                book: BookId(book),
                desk: DeskId(desk),
                underlying: p.underlying.clone(),
                location: LocationId(location),
                entity: EntityId(entity),
            },
            measure: crate::dimension::FactMeasure {
                leaf: canonicalize(p).unwrap(),
                position: p.clone(),
                exotic: None,
            },
            surface_version: 1,
        }
    }

    /// A node aggregate with distinct dyadic values in EVERY additive line and
    /// non-empty constituent vectors, for the merge pins.
    fn hand_node(group: u64, m: f64, pillar: VegaPillar) -> NodeAggregate {
        let leaf = CanonicalLeaf {
            underlying: Underlying::Fx(eurusd()),
            spot: 1.25,
            greeks: CanonicalGreeks {
                delta_base: 1.0 * m,
                gamma: 2.0 * m,
                vega: 4.0 * m,
                theta: 8.0 * m,
                vanna: 16.0 * m,
                volga: 32.0 * m,
                charm: 64.0 * m,
                speed: 128.0 * m,
                zomma: 256.0 * m,
                color: 512.0 * m,
            },
            premium_quote: 1024.0 * m,
            vega_premium_ccy: Ccy::USD,
            quoted_was_premium_adjusted: false,
        };
        let mut agg = NodeAggregate::empty(group);
        agg.net_greeks.add_leaf(&leaf);
        agg.vega_ladder.add(pillar, 4.0 * m);
        agg.leaves.push(leaf);
        agg.positions.push(pos(
            m,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        ));
        agg
    }

    /// `merge_additive` sums EVERY `NetGreeks` field and the ladder element-wise,
    /// and concatenates positions/exotic legs/leaves (self first) — the §3.4
    /// shard-merge seam, pinned bit-exactly per field.
    #[test]
    fn merge_additive_sums_every_field_and_concatenates() {
        let p1y = VegaPillar::new(365, 5000);
        let p6m = VegaPillar::new(183, 5000);
        let mut a = hand_node(1, 1.0, p1y);
        let mut b = hand_node(2, 0.25, p6m);
        b.vega_ladder.add(p1y, 0.5); // overlap to prove element-wise netting
        a.merge_additive(&b);

        let got = [
            a.net_greeks.delta_base,
            a.net_greeks.gamma,
            a.net_greeks.vega,
            a.net_greeks.theta,
            a.net_greeks.vanna,
            a.net_greeks.volga,
            a.net_greeks.charm,
            a.net_greeks.speed,
            a.net_greeks.zomma,
            a.net_greeks.color,
            a.net_greeks.premium_quote,
        ];
        let want: [f64; 11] = [
            1.25, 2.5, 5.0, 10.0, 20.0, 40.0, 80.0, 160.0, 320.0, 640.0, 1280.0,
        ];
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            assert_eq!(g.to_bits(), w.to_bits(), "merged field #{i}");
        }
        assert_eq!(a.vega_ladder.vega_in(p1y).to_bits(), 4.5_f64.to_bits());
        assert_eq!(a.vega_ladder.vega_in(p6m).to_bits(), 1.0_f64.to_bits());
        assert_eq!(a.positions.len(), 2);
        assert_eq!(a.leaves.len(), 2);
        // Self-first concatenation order (drill-down provenance).
        assert_eq!(a.positions[0].notional_base.to_bits(), 1.0_f64.to_bits());
        assert_eq!(a.positions[1].notional_base.to_bits(), 0.25_f64.to_bits());
        // The group key of the receiving node is untouched by a merge.
        assert_eq!(a.group, 1);
    }

    /// `upsert` keeps ONE live fact per position id; `fact()` retrieves by id;
    /// `len`/`is_empty` track the live set.
    #[test]
    fn upsert_fact_len_and_lookup() {
        let mut cube = Cube::new();
        assert!(cube.is_empty());
        assert_eq!(cube.len(), 0);
        assert!(cube.fact(crate::dimension::PositionId(1)).is_none());

        let p1 = pos(1.0, VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02));
        let p2 = pos(2.0, VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.04, 0.02));
        cube.upsert(fact(1, (1, 1, 1, 1, 1), &p1));
        cube.upsert(fact(2, (2, 2, 2, 2, 2), &p2));
        assert!(!cube.is_empty());
        assert_eq!(cube.len(), 2);
        // Supersede id 1: the same id must REPLACE, not duplicate, and lookup must
        // return the NEW fact (not first-inserted).
        let p1b = pos(4.0, VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02));
        cube.upsert(fact(1, (1, 1, 1, 1, 1), &p1b));
        assert_eq!(cube.len(), 2);
        let f = cube.fact(crate::dimension::PositionId(1)).unwrap();
        assert_eq!(
            f.measure.position.notional_base.to_bits(),
            4.0_f64.to_bits()
        );
        // And id 2 still resolves to ITS fact (the find matches on id, not always-first).
        let f2 = cube.fact(crate::dimension::PositionId(2)).unwrap();
        assert_eq!(
            f2.measure.position.notional_base.to_bits(),
            2.0_f64.to_bits()
        );
    }

    /// `group_by` buckets by the dimension's group key in FIRST-SEEN order with the
    /// group value carried on the node, and an exotic fact routes its leg to
    /// `exotic_legs` (the real-exotic re-derivation source) while a vanilla fact
    /// routes to `positions`.
    #[test]
    fn group_by_orders_first_seen_and_routes_exotic_legs() {
        let mut cube = Cube::new();
        let p1 = pos(1.0, VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02));
        let p2 = pos(2.0, VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.04, 0.02));
        cube.upsert(fact(1, (9, 1, 1, 1, 1), &p1)); // trader 9 first
        cube.upsert(fact(2, (3, 1, 1, 1, 1), &p2)); // trader 3 second
        // An exotic digital fact under trader 9.
        let leg = crate::exotic::ExoticLeg::new(
            eurusd(),
            ExoticKind::Digital(DigitalKind::cash(OptionType::Call)),
            5.0,
            VanillaInputs::new(1.10, 1.11, 0.10, 0.25, 0.04, 0.02),
        );
        let meta = pos(5.0, leg.inputs);
        let mut ef = fact(3, (9, 1, 1, 1, 1), &meta);
        ef.measure.leaf = leg.canonical_leaf(&DigitalTestPricer);
        ef.measure.exotic = Some(leg);
        cube.upsert(ef);

        let by_trader = cube.group_by(crate::dimension::DimensionId::Trader, &OnePillar);
        assert_eq!(by_trader.len(), 2);
        assert_eq!(by_trader[0].group, 9, "first-seen group order");
        assert_eq!(by_trader[1].group, 3);
        // Trader 9 holds one vanilla position AND one exotic leg, three leaves total
        // across both nodes.
        assert_eq!(by_trader[0].positions.len(), 1);
        assert_eq!(by_trader[0].exotic_legs.len(), 1);
        assert_eq!(by_trader[0].leaves.len(), 2);
        assert_eq!(by_trader[1].positions.len(), 1);
        assert_eq!(by_trader[1].exotic_legs.len(), 0);
        // The exotic's REAL Greek leaf contributes to the additive roll-up.
        let want_vega = canonicalize(&p1).unwrap().greeks.vega
            + by_trader[0].exotic_legs[0]
                .canonical_leaf(&DigitalTestPricer)
                .greeks
                .vega;
        assert!(
            celnet_core::is_close(by_trader[0].net_greeks.vega, want_vega, 1e-12, 1e-9),
            "exotic leaf vega must be in the node roll-up"
        );
    }

    /// The Entity dimension resolves location→entity through the configured parent
    /// pointer, and falls back to the fact's own entity key when unconfigured (an
    /// unmapped location is its own regulatory unit, never silently entity 0).
    #[test]
    fn entity_rollup_resolves_parent_with_fallback() {
        let mut h = Hierarchy::new();
        h.set_location_entity(LocationId(4), EntityId(99));
        let mut cube = Cube::with_hierarchy(h);
        let p1 = pos(1.0, VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02));
        let p2 = pos(2.0, VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.04, 0.02));
        // Fact 1: location 4 → configured entity 99 (its own entity key 7 is stale).
        cube.upsert(fact(1, (1, 1, 1, 4, 7), &p1));
        // Fact 2: location 5 unconfigured → falls back to its own entity key 8.
        cube.upsert(fact(2, (2, 2, 2, 5, 8), &p2));
        let by_entity = cube.group_by(crate::dimension::DimensionId::Entity, &OnePillar);
        let groups: Vec<u64> = by_entity.iter().map(|n| n.group).collect();
        assert_eq!(groups, vec![99, 8]);
        // hierarchy_mut genuinely exposes the live hierarchy: re-point and re-group.
        cube.hierarchy_mut()
            .set_location_entity(LocationId(5), EntityId(99));
        let regrouped = cube.group_by(crate::dimension::DimensionId::Entity, &OnePillar);
        assert_eq!(regrouped.len(), 1);
        assert_eq!(regrouped[0].group, 99);
    }

    /// `node_correlation_weighted_vega` applies the caller's pillar weighting to
    /// each ladder bucket IN ORDER before the quadratic form — pinned against the
    /// hand-computed `√(Σw² + 2ρ w_i w_j)` on distinct per-pillar weights.
    #[test]
    fn correlation_weighted_vega_applies_pillar_weights() {
        let mut node = NodeAggregate::empty(0);
        node.vega_ladder.add(VegaPillar::new(365, 5000), 3.0);
        node.vega_ladder.add(VegaPillar::new(183, 5000), 4.0);
        // Weight = vega × (tenor/365), so the two buckets get DISTINCT weights and
        // a pillar/vega confusion changes the value.
        let weighted = |p: VegaPillar, v: f64| v * (f64::from(p.tenor_days) / 365.0);
        let w1 = 3.0_f64; // 3.0 × 365/365
        let w2 = 4.0 * (183.0 / 365.0);
        let rho = 0.5;
        let want = (w1 * w1 + w2 * w2 + 2.0 * rho * w1 * w2).sqrt();
        let got = Cube::node_correlation_weighted_vega(&node, weighted, |_, _| rho);
        assert!(celnet_core::is_close(got, want, 1e-12, 1e-12));
        // Reconciles to the free function on the same weighted inputs.
        let free = crate::nonadditive::correlation_weighted_vega(&[w1, w2], |_, _| rho);
        assert_eq!(got.to_bits(), free.to_bits());
    }

    /// The non-additive delegates pass the node's BOTH constituent classes through:
    /// with no exotic legs they reduce exactly to the free functions over
    /// `node.positions`.
    #[test]
    fn node_var_es_delegates_reduce_to_free_functions() {
        let mut node = NodeAggregate::empty(0);
        let p = pos(
            1_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01),
        );
        node.positions.push(p.clone());
        let scen: Vec<Scenario> = (-5..=5)
            .filter(|i| *i != 0)
            .map(|i| Scenario::spot(f64::from(i) * 0.01))
            .collect();
        let oracle = Cube::node_var_es(&AssetPricer, &DigitalTestPricer, &node, &scen, 0.9);
        let free = crate::nonadditive::historical_var_es(
            &AssetPricer,
            std::slice::from_ref(&p),
            &scen,
            0.9,
        );
        assert_eq!(oracle.var.to_bits(), free.var.to_bits());
        assert_eq!(oracle.es.to_bits(), free.es.to_bits());
        let fast =
            Cube::node_var_es_sensitivity(&AssetPricer, &DigitalTestPricer, &node, &scen, 0.9);
        let free_fast = crate::nonadditive::sensitivity_var_es(
            &AssetPricer,
            std::slice::from_ref(&p),
            &scen,
            0.9,
        );
        assert_eq!(fast.var.to_bits(), free_fast.var.to_bits());
        assert_eq!(fast.es.to_bits(), free_fast.es.to_bits());
        let cvr = Cube::node_curvature_spot(&AssetPricer, &DigitalTestPricer, &node, 0.15);
        let free_cvr =
            crate::nonadditive::sbm_curvature_spot(&AssetPricer, std::slice::from_ref(&p), 0.15);
        assert_eq!(cvr.to_bits(), free_cvr.to_bits());
    }
}
