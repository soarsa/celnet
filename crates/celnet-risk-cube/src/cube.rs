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
    fn empty(group: u64) -> Self {
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
    agg.leaves.push(*leaf);
    match fact.measure.exotic {
        // An exotic fact re-prices through the real closed-form exotic pricer.
        Some(leg) => agg.exotic_legs.push(leg),
        // A vanilla fact re-prices through celnet-vanilla.
        None => agg.positions.push(fact.measure.position),
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
    pub fn node_var_es(node: &NodeAggregate, scenarios: &[Scenario], alpha: f64) -> VarEs {
        node_var_es_combined(&node.positions, &node.exotic_legs, scenarios, alpha)
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
    pub fn node_var_es_sensitivity(
        node: &NodeAggregate,
        scenarios: &[Scenario],
        alpha: f64,
    ) -> VarEs {
        node_var_es_sensitivity_combined(&node.positions, &node.exotic_legs, scenarios, alpha)
    }

    /// **Non-additive**: FRTB-SbM spot curvature of a node by up/down full reprice
    /// net of the linear delta term (`docs/RISK-HIERARCHY.md` §2.5).
    #[must_use]
    pub fn node_curvature_spot(node: &NodeAggregate, rw: f64) -> f64 {
        sbm_curvature_spot_combined(&node.positions, &node.exotic_legs, rw)
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
