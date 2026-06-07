//! The server-side risk **aggregation engine** — the pure core of
//! [`RiskService`](super::RiskEdge).
//!
//! Given a [`StoreSnapshot`](super::store::StoreSnapshot) of the live book, an
//! entitlement [`Principal`], a reporting numeraire [`WireResolver`], a pillar grid
//! and the non-additive shock inputs, this builds the wire node tree:
//!
//! 1. **Prune** the fact stream by the principal **before** any roll-up
//!    ([`celnet_entitlements::EntitlementFilter::entitled_cube`]) — no aggregate
//!    leakage (RH §2.6).
//! 2. **Group** the entitled cube by the requested dimension (or `firm_aggregate`
//!    for the apex) — additive `NetGreeks` + a vega ladder per node.
//! 3. **Collapse** each node into the reporting numeraire
//!    ([`NodeAggregate::numeraire_view`]) — delta vector → scalar, premium and vega
//!    through the premium currency (RH §2.3).
//! 4. **Re-derive** the non-additive measures (VaR / ES / curvature) per node by
//!    bump-and-revalue over numeraire-scaled positions ([`Cube::node_var_es`] /
//!    [`Cube::node_curvature_spot`]) — never summed from children (RH §2.5).
//!
//! # Numeraire-correct vega ladder
//!
//! The cube's [`VegaLadder`](celnet_risk_cube::VegaLadder) sums *raw* vega in each
//! leaf's premium currency, so a cross-currency ladder must convert per pillar
//! through the premium currency (the §2.2/§2.3 coupling). This module therefore
//! builds the wire ladder by grouping each node's leaves into pillars and running
//! the [`Numeraire`](celnet_risk_normalize::Numeraire) collapse **per pillar**, so a
//! pillar's reported vega is in the reporting numeraire — matching `vega_numeraire`
//! when summed across pillars of a single premium currency.
//!
//! # Numeraire-correct non-additive measures
//!
//! [`historical_var_es`](celnet_risk_cube::historical_var_es) sums each position's
//! P&L in its own quote currency; for a cross-currency node that would mix
//! currencies. Per that function's documented contract ("pass positions already
//! normalized to a common numeraire"), this module scales each position's
//! `notional_base` by its quote→numeraire spot rate before the VaR/curvature call —
//! `position_pnl = price · notional_base` and `price` is quote-ccy-per-unit-base, so
//! scaling the notional by the quote→numeraire rate expresses the P&L in the
//! reporting numeraire exactly. The scaled positions are used **only** for the
//! non-additive re-derivation; the additive measures use the real positions.

use std::collections::BTreeMap;

use celnet_entitlements::{EntitlementFilter, Principal};
use celnet_proto::{RiskNode as WireRiskNode, VegaLadderBucket, VegaPillar as WireVegaPillar};
use celnet_risk_cube::{
    Cube, DimensionId, Hierarchy, NodeAggregate, RiskFact, Scenario, VegaPillar, VegaPillarMap,
};
use celnet_risk_normalize::{CanonicalLeaf, Numeraire, NumeraireError, PositionRisk, SpotResolver};
use tonic::Status;

use super::convert::{self, WireResolver, additive_to_wire, nonadditive_to_wire, risk_node_shell};

/// The non-additive shock configuration for an aggregation cycle.
#[derive(Debug, Clone, Default)]
pub struct NonAdditiveConfig {
    /// The relative spot-shock steps to bump-and-revalue VaR/ES over. Empty ⇒
    /// VaR/ES are not evaluated (absent in the response — never a spurious zero).
    pub spot_shocks: Vec<f64>,
    /// The VaR/ES confidence level; `0.0` ⇒ default `0.99` when shocks are present.
    pub var_alpha: f64,
    /// The FRTB-SbM spot curvature risk weight; `0.0` ⇒ curvature not evaluated.
    pub curvature_risk_weight: f64,
}

impl NonAdditiveConfig {
    /// Whether VaR/ES are to be evaluated this cycle.
    fn evaluates_var(&self) -> bool {
        !self.spot_shocks.is_empty()
    }

    /// The effective confidence level (defaulting `0.0` to `0.99`).
    fn alpha(&self) -> f64 {
        if self.var_alpha > 0.0 {
            self.var_alpha
        } else {
            0.99
        }
    }

    /// Whether the FRTB curvature charge is to be evaluated this cycle.
    fn evaluates_curvature(&self) -> bool {
        self.curvature_risk_weight > 0.0
    }
}

/// The vega-pillar grid for an aggregation cycle: either a caller-supplied
/// regulatory/internal grid (the `VegaPillarMap` external-data contract), or, when
/// empty, a server-derived grid that buckets each leaf by its tenor-in-days at a
/// single default delta pillar (derived from the live positions, never compiled-in).
pub struct PillarGrid {
    /// The supplied tenor pillars (in days), sorted; empty ⇒ derive per leaf.
    tenors: Vec<u32>,
    /// The single delta pillar a derived grid buckets onto (the live default).
    default_delta_bp: i32,
    /// Whether a grid was explicitly supplied (snap to the nearest tenor) vs
    /// derived (bucket by the leaf's own rounded tenor).
    supplied: bool,
}

/// The default delta pillar a server-derived grid buckets onto (0.50Δ = 5000bp) —
/// the at-the-money working vertex, used only when the caller pins no grid.
const DEFAULT_DELTA_BP: i32 = 5000;

impl PillarGrid {
    /// Build a grid from the request's pillars (empty ⇒ a derived grid).
    fn new(pillars: &[WireVegaPillar]) -> Self {
        if pillars.is_empty() {
            return Self {
                tenors: Vec::new(),
                default_delta_bp: DEFAULT_DELTA_BP,
                supplied: false,
            };
        }
        let mut tenors: Vec<u32> = pillars.iter().map(|p| p.tenor_days).collect();
        tenors.sort_unstable();
        tenors.dedup();
        // The grid's delta pillar is taken from the first supplied pillar (the
        // caller's grid is along one delta vertex per tenor in the FX-vega cut).
        let default_delta_bp = pillars.first().map_or(DEFAULT_DELTA_BP, |p| p.delta_bp);
        Self {
            tenors,
            default_delta_bp,
            supplied: true,
        }
    }
}

/// The server-derived default pillar grid (no caller-supplied vertices): buckets
/// each leaf by its own rounded tenor at the default delta pillar. Used by the
/// limit path's node aggregation when no explicit grid is in play.
#[must_use]
pub fn default_grid() -> PillarGrid {
    PillarGrid::new(&[])
}

impl VegaPillarMap for PillarGrid {
    fn pillar_of(&self, _leaf: &CanonicalLeaf, position: &PositionRisk) -> VegaPillar {
        let days = (position.inputs.t * 365.0).round() as u32;
        if !self.supplied {
            // Derived grid: bucket by the leaf's own rounded tenor.
            return VegaPillar::new(days, self.default_delta_bp);
        }
        // Supplied grid: snap to the nearest configured tenor pillar.
        let snapped = self
            .tenors
            .iter()
            .copied()
            .min_by_key(|t| t.abs_diff(days))
            .unwrap_or(days);
        VegaPillar::new(snapped, self.default_delta_bp)
    }
}

/// Build the entitled cube for an aggregation cycle: prune the snapshot facts by the
/// principal **before** ingestion (no aggregate leakage), wired to the snapshot's
/// hierarchy.
#[must_use]
pub fn entitled_cube(facts: &[RiskFact], principal: &Principal, hierarchy: &Hierarchy) -> Cube {
    let filter = EntitlementFilter::new(principal, hierarchy);
    filter.entitled_cube(facts.iter().copied())
}

/// Roll the entitled cube up over a dimension (or the firm apex when `dim` is
/// `None`) and assemble the wire node tree, each node carrying its numeraire-collapsed
/// additive measures, its (optionally) re-derived non-additive measures, and its
/// position count.
///
/// # Errors
/// `failed_precondition` if a node's numeraire collapse hits a missing/invalid rate.
pub fn aggregate_nodes(
    cube: &Cube,
    dim: Option<DimensionId>,
    resolver: &WireResolver,
    pillars: &[WireVegaPillar],
    nonadditive: &NonAdditiveConfig,
) -> Result<Vec<WireRiskNode>, Status> {
    let grid = PillarGrid::new(pillars);
    let nodes: Vec<NodeAggregate> = match dim {
        None => vec![cube.firm_aggregate(&grid)],
        Some(d) => cube.group_by(d, &grid),
    };
    let dim_wire = dim.map_or(
        celnet_proto::RiskDimension::Firm as i32,
        convert::dimension_to_wire,
    );

    let mut out = Vec::with_capacity(nodes.len());
    for node in &nodes {
        out.push(node_to_wire(node, dim_wire, resolver, &grid, nonadditive)?);
    }
    Ok(out)
}

/// Assemble one wire [`RiskNode`](WireRiskNode) from a cube node aggregate.
///
/// # Errors
/// `failed_precondition` if the numeraire collapse fails (missing/invalid rate).
pub fn node_to_wire(
    node: &NodeAggregate,
    dimension: i32,
    resolver: &WireResolver,
    grid: &PillarGrid,
    nonadditive: &NonAdditiveConfig,
) -> Result<WireRiskNode, Status> {
    // Additive: the numeraire-collapsed delta vector / premium / vega.
    let numeraire = node
        .numeraire_view(resolver)
        .map_err(convert::numeraire_status)?;
    let vega_ladder = numeraire_vega_ladder(node, resolver, grid)?;
    let additive = additive_to_wire(&numeraire, &node.net_greeks, vega_ladder);

    // Non-additive: re-derived per node over numeraire-scaled positions.
    let na = nonadditive_measures(node, resolver, nonadditive)?;
    let nonadditive_wire = nonadditive_to_wire(na.var, na.es, na.var_alpha, na.curvature_spot);

    let mut shell = risk_node_shell(dimension, node.group, convert::node_position_count(node));
    shell.additive = Some(additive);
    shell.nonadditive = Some(nonadditive_wire);
    Ok(shell)
}

/// Build the node's vega ladder in the reporting numeraire: group the node's
/// leaves into pillars, then run the per-pillar [`Numeraire`] collapse so each
/// pillar's reported vega converts through the premium currency (the §2.2/§2.3
/// coupling) — summing the pillars equals the node's `vega_numeraire`.
fn numeraire_vega_ladder(
    node: &NodeAggregate,
    resolver: &WireResolver,
    grid: &PillarGrid,
) -> Result<Vec<VegaLadderBucket>, Status> {
    let pillars = numeraire_pillar_vega(node, resolver, grid)?;
    Ok(pillars
        .into_iter()
        .map(|(pillar, vega)| VegaLadderBucket {
            pillar: Some(convert::pillar_to_wire(pillar)),
            vega,
        })
        .collect())
}

/// The node's `(pillar, numeraire-vega)` pairs: group the `(leaf, position)` pairs
/// into pillars by the grid (which reads each position's tenor) and run the
/// per-pillar [`Numeraire`] collapse so each pillar's vega converts through the
/// premium currency. Shared by the wire ladder and the limit-path ladder so both
/// bucket and convert identically.
///
/// # Errors
/// `failed_precondition` for a missing/invalid premium-currency rate.
pub fn numeraire_pillar_vega(
    node: &NodeAggregate,
    resolver: &WireResolver,
    grid: &PillarGrid,
) -> Result<Vec<(VegaPillar, f64)>, Status> {
    // Group leaves by pillar, preserving insertion order with a stable key.
    let mut by_pillar: Vec<(VegaPillar, Vec<CanonicalLeaf>)> = Vec::new();
    for (leaf, position) in node.leaves.iter().zip(node.positions.iter()) {
        let pillar = grid.pillar_of(leaf, position);
        if let Some(slot) = by_pillar.iter_mut().find(|(p, _)| *p == pillar) {
            slot.1.push(*leaf);
        } else {
            by_pillar.push((pillar, vec![*leaf]));
        }
    }
    let mut out = Vec::with_capacity(by_pillar.len());
    for (pillar, leaves) in by_pillar {
        let view = Numeraire::from_leaves(&leaves, resolver).map_err(convert::numeraire_status)?;
        out.push((pillar, view.vega_numeraire));
    }
    Ok(out)
}

/// The re-derived non-additive measures of a node, each presence-tracked (absent ⇒
/// not evaluated this cycle, never a spurious zero).
#[derive(Debug, Clone, Copy, Default)]
struct NodeNonAdditive {
    var: Option<f64>,
    es: Option<f64>,
    var_alpha: Option<f64>,
    curvature_spot: Option<f64>,
}

/// Re-derive the node's non-additive measures over numeraire-scaled positions, each
/// presence-tracked. VaR/ES are evaluated only when spot shocks are supplied;
/// curvature only when a risk weight is supplied — otherwise the measure is absent
/// (never a spurious zero, §2.5).
///
/// # Errors
/// `failed_precondition` if a position's quote currency has no numeraire rate (the
/// non-additive P&L cannot be expressed in the numeraire without it).
fn nonadditive_measures(
    node: &NodeAggregate,
    resolver: &WireResolver,
    cfg: &NonAdditiveConfig,
) -> Result<NodeNonAdditive, Status> {
    if !cfg.evaluates_var() && !cfg.evaluates_curvature() {
        return Ok(NodeNonAdditive::default());
    }
    // Scale each position's notional by its quote→numeraire rate so the re-priced
    // P&L is expressed in the reporting numeraire (the documented common-numeraire
    // contract of `historical_var_es`). This scaled set is used ONLY for the
    // non-additive re-derivation.
    let scaled = scale_positions_to_numeraire(node, resolver)?;
    // Exotic legs are scaled into the reporting numeraire the same way (their P&L is
    // in quote ccy too), so a booked exotic contributes its true tail to the
    // node's non-additive VaR/ES + curvature, never silently dropped.
    let scaled_exotics = scale_exotic_legs_to_numeraire(node, resolver)?;
    // Re-derive over a transient cube node holding the scaled positions, reusing
    // the cube's bump-and-revalue reducers (the oracle path, RH §2.5).
    let scaled_node = NodeAggregate {
        group: node.group,
        net_greeks: node.net_greeks,
        vega_ladder: node.vega_ladder.clone(),
        positions: scaled,
        exotic_legs: scaled_exotics,
        leaves: node.leaves.clone(),
    };

    let (var, es, var_alpha) = if cfg.evaluates_var() {
        let scenarios: Vec<Scenario> = cfg.spot_shocks.iter().map(|s| Scenario::spot(*s)).collect();
        let ve = Cube::node_var_es(&scaled_node, &scenarios, cfg.alpha());
        (Some(ve.var), Some(ve.es), Some(cfg.alpha()))
    } else {
        (None, None, None)
    };
    let curvature_spot = if cfg.evaluates_curvature() {
        Some(Cube::node_curvature_spot(
            &scaled_node,
            cfg.curvature_risk_weight,
        ))
    } else {
        None
    };
    Ok(NodeNonAdditive {
        var,
        es,
        var_alpha,
        curvature_spot,
    })
}

/// Scale each of a node's positions so that repricing it yields P&L in the reporting
/// numeraire: multiply `notional_base` by the position's quote→numeraire spot rate
/// (`price` is quote-ccy-per-unit-base, so the scaled value/P&L is in numeraire).
///
/// # Errors
/// `failed_precondition` if a position's quote currency has no numeraire rate.
fn scale_positions_to_numeraire(
    node: &NodeAggregate,
    resolver: &WireResolver,
) -> Result<Vec<PositionRisk>, Status> {
    let mut out = Vec::with_capacity(node.positions.len());
    for p in &node.positions {
        let rate = resolver
            .rate_into_numeraire(p.pair.quote)
            .ok_or_else(|| convert::numeraire_status(NumeraireError::MissingRate(p.pair.quote)))?;
        if !rate.is_finite() || rate <= 0.0 {
            return Err(convert::numeraire_status(NumeraireError::InvalidRate(
                p.pair.quote,
            )));
        }
        out.push(PositionRisk::new(
            p.pair,
            p.option,
            p.notional_base * rate,
            p.inputs,
            p.quoted_delta,
            p.premium_style,
        ));
    }
    Ok(out)
}

/// Scale a node's exotic legs into the reporting numeraire: each leg's P&L is in its
/// quote currency, so its notional is multiplied by the quote→numeraire rate exactly
/// like a vanilla position (the canonical-numeraire contract). Empty when the node
/// holds no exotic legs.
fn scale_exotic_legs_to_numeraire(
    node: &NodeAggregate,
    resolver: &WireResolver,
) -> Result<Vec<celnet_risk_cube::ExoticLeg>, Status> {
    let mut out = Vec::with_capacity(node.exotic_legs.len());
    for l in &node.exotic_legs {
        let rate = resolver
            .rate_into_numeraire(l.pair.quote)
            .ok_or_else(|| convert::numeraire_status(NumeraireError::MissingRate(l.pair.quote)))?;
        if !rate.is_finite() || rate <= 0.0 {
            return Err(convert::numeraire_status(NumeraireError::InvalidRate(
                l.pair.quote,
            )));
        }
        out.push(celnet_risk_cube::ExoticLeg::new(
            l.pair,
            l.kind,
            l.notional * rate,
            l.inputs,
        ));
    }
    Ok(out)
}

/// List the entitled facts for a scope, in a stable order (by `position_id`), so a
/// `ListPositions` / `DrillRisk` leaf report is deterministic.
#[must_use]
pub fn entitled_facts_for_scope(
    facts: &[RiskFact],
    principal: &Principal,
    hierarchy: &Hierarchy,
    scope: Option<(DimensionId, u64)>,
) -> Vec<RiskFact> {
    let filter = EntitlementFilter::new(principal, hierarchy);
    let mut admitted: Vec<RiskFact> = facts
        .iter()
        .copied()
        .filter(|f| filter.admits(f))
        .filter(|f| match scope {
            None => true,
            Some((dim, value)) => resolved_group_value(hierarchy, f, dim) == value,
        })
        .collect();
    admitted.sort_by_key(|f: &RiskFact| f.position_id.0);
    admitted
}

/// Resolve a fact's group value along `dim` with the cube's parent-pointer
/// resolution (Desk ← book's parent, Entity ← location's parent), so a scope filter
/// matches exactly the facts that roll into the corresponding node.
#[must_use]
pub fn resolved_group_value(hierarchy: &Hierarchy, fact: &RiskFact, dim: DimensionId) -> u64 {
    match dim {
        DimensionId::Desk => hierarchy
            .desk_of(fact.key.book)
            .map_or_else(|| fact.key.group_value(dim), |d| u64::from(d.raw())),
        DimensionId::Entity => hierarchy
            .entity_of(fact.key.location)
            .map_or_else(|| fact.key.group_value(dim), |e| u64::from(e.raw())),
        other => fact.key.group_value(other),
    }
}

/// Build a transient cube over an explicit fact set (for tests / the explicit-fact
/// path). Deduplicates by `position_id` via `BTreeMap` then upserts in id order.
#[must_use]
pub fn cube_from_facts(facts: &[RiskFact], hierarchy: Hierarchy) -> Cube {
    let mut dedup: BTreeMap<u32, RiskFact> = BTreeMap::new();
    for f in facts {
        dedup.insert(f.position_id.0, *f);
    }
    let mut cube = Cube::with_hierarchy(hierarchy);
    for (_, f) in dedup {
        cube.upsert(f);
    }
    cube
}

/// Build a [`RiskFact`] for tests from raw economics + an org placement.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn test_fact(
    id: u32,
    trader: u32,
    book: u32,
    desk: u32,
    loc: u32,
    ent: u32,
    pair: celnet_types::CcyPair,
    option: celnet_types::OptionType,
    notional: f64,
    inputs: celnet_types::VanillaInputs,
) -> RiskFact {
    use celnet_risk_cube::{
        BookId, DeskId, EntityId, FactKey, FactMeasure, LocationId, PositionId, TraderId,
    };
    use celnet_risk_normalize::canonicalize;
    use celnet_types::{DeltaConvention, PremiumStyle};
    let position = PositionRisk::new(
        pair,
        option,
        notional,
        inputs,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    );
    RiskFact {
        position_id: PositionId(id),
        key: FactKey {
            trader: TraderId(trader),
            book: BookId(book),
            desk: DeskId(desk),
            ccy_pair: pair,
            location: LocationId(loc),
            entity: EntityId(ent),
        },
        measure: FactMeasure {
            leaf: canonicalize(&position),
            position,
            exotic: None,
        },
        surface_version: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{NumeraireRate, ReportingNumeraire};
    use celnet_risk_normalize::canonicalize;
    use celnet_types::{Ccy, CcyPair, OptionType, VanillaInputs};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }
    fn usdjpy() -> CcyPair {
        CcyPair::new(Ccy::USD, Ccy::JPY)
    }

    fn usd_numeraire() -> WireResolver {
        WireResolver::new(&ReportingNumeraire {
            numeraire: "USD".to_owned(),
            rates: vec![
                NumeraireRate {
                    ccy: "EUR".to_owned(),
                    rate: 1.10,
                },
                NumeraireRate {
                    ccy: "JPY".to_owned(),
                    rate: 1.0 / 156.0,
                },
            ],
        })
        .unwrap()
    }

    /// Additive roll-up consistency: the firm node's `delta_numeraire` equals the
    /// sum of the per-book nodes' `delta_numeraire` (additive measures sum exactly).
    #[test]
    fn additive_rollup_sum_of_children_equals_parent() {
        let facts = vec![
            test_fact(
                1,
                1,
                1,
                1,
                1,
                1,
                eurusd(),
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
            test_fact(
                2,
                1,
                1,
                1,
                1,
                1,
                eurusd(),
                OptionType::Put,
                5_000_000.0,
                VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.04, 0.02),
            ),
            test_fact(
                3,
                2,
                2,
                1,
                1,
                1,
                eurusd(),
                OptionType::Call,
                7_000_000.0,
                VanillaInputs::new(1.10, 1.15, 0.09, 1.0, 0.04, 0.02),
            ),
        ];
        let cube = cube_from_facts(&facts, Hierarchy::new());
        let resolver = usd_numeraire();
        let cfg = NonAdditiveConfig::default();

        let firm = aggregate_nodes(&cube, None, &resolver, &[], &cfg).unwrap();
        assert_eq!(firm.len(), 1);
        let firm_delta = firm[0].additive.as_ref().unwrap().delta_numeraire;

        let by_book =
            aggregate_nodes(&cube, Some(DimensionId::Book), &resolver, &[], &cfg).unwrap();
        assert_eq!(by_book.len(), 2);
        let sum_books: f64 = by_book
            .iter()
            .map(|n| n.additive.as_ref().unwrap().delta_numeraire)
            .sum();
        assert!(
            (firm_delta - sum_books).abs() < 1e-6,
            "firm delta {firm_delta} must equal Σ book deltas {sum_books}"
        );
        // Position counts roll up too.
        let firm_count = firm[0].position_count;
        let sum_counts: u32 = by_book.iter().map(|n| n.position_count).sum();
        assert_eq!(firm_count, 3);
        assert_eq!(sum_counts, 3);
    }

    /// Numeraire conversion correctness: a USD-numeraire firm node's
    /// `delta_numeraire` equals collapsing the netted delta vector at the same
    /// rates (the §2.3 round-trip), and the cross-pair USD leg nets correctly.
    #[test]
    fn numeraire_conversion_matches_vector_collapse() {
        let p1 = test_fact(
            1,
            1,
            1,
            1,
            1,
            1,
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p2 = test_fact(
            2,
            2,
            2,
            1,
            1,
            1,
            usdjpy(),
            OptionType::Put,
            8_000_000.0,
            VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
        );
        let cube = cube_from_facts(&[p1, p2], Hierarchy::new());
        let resolver = usd_numeraire();
        let firm =
            aggregate_nodes(&cube, None, &resolver, &[], &NonAdditiveConfig::default()).unwrap();
        let add = firm[0].additive.as_ref().unwrap();

        // Collapse the reported delta vector independently and compare to the scalar.
        let mut recomputed = 0.0;
        for leg in &add.delta_vector {
            let c = Ccy::parse(&leg.ccy).unwrap();
            recomputed += leg.amount * resolver.rate_into_numeraire(c).unwrap();
        }
        assert!(
            (recomputed - add.delta_numeraire).abs() < 1e-3,
            "vector collapse {recomputed} != scalar {}",
            add.delta_numeraire
        );

        // The USD leg = EURUSD funding leg (−delta·spot) + USDJPY base hedge.
        let l1 = canonicalize(&p1.measure.position);
        let l2 = canonicalize(&p2.measure.position);
        let expected_usd = -l1.greeks.delta_base * l1.spot + l2.greeks.delta_base;
        let usd_leg = add
            .delta_vector
            .iter()
            .find(|l| l.ccy == "USD")
            .map_or(0.0, |l| l.amount);
        assert!(
            (usd_leg - expected_usd).abs() < 1e-3,
            "USD leg {usd_leg} != expected {expected_usd}"
        );
    }

    /// The vega ladder sums (across pillars of one premium currency) to the node's
    /// `vega_numeraire` — the per-pillar numeraire collapse is consistent.
    #[test]
    fn vega_ladder_sums_to_vega_numeraire() {
        let facts = vec![
            test_fact(
                1,
                1,
                1,
                1,
                1,
                1,
                eurusd(),
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
            test_fact(
                2,
                1,
                1,
                1,
                1,
                1,
                eurusd(),
                OptionType::Put,
                5_000_000.0,
                VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.04, 0.02),
            ),
        ];
        let cube = cube_from_facts(&facts, Hierarchy::new());
        let resolver = usd_numeraire();
        let firm =
            aggregate_nodes(&cube, None, &resolver, &[], &NonAdditiveConfig::default()).unwrap();
        let add = firm[0].additive.as_ref().unwrap();
        let ladder_total: f64 = add.vega_ladder.iter().map(|b| b.vega).sum();
        assert!(
            (ladder_total - add.vega_numeraire).abs() < 1e-6,
            "ladder total {ladder_total} != vega_numeraire {}",
            add.vega_numeraire
        );
        // Two distinct tenors → two pillars in the derived grid.
        assert_eq!(add.vega_ladder.len(), 2);
    }

    /// Non-additive VaR diversifies: a long+short offsetting book has ~zero VaR,
    /// far below the sum of the legs' standalone VaRs — proving it is re-derived,
    /// not summed (§2.5). And it is absent when no shocks are supplied.
    #[test]
    fn nonadditive_var_is_diversified_and_presence_tracked() {
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01);
        let long = test_fact(
            1,
            1,
            1,
            1,
            1,
            1,
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            inputs,
        );
        let short = test_fact(
            2,
            1,
            1,
            1,
            1,
            1,
            eurusd(),
            OptionType::Call,
            -10_000_000.0,
            inputs,
        );
        let resolver = usd_numeraire();
        let shocks: Vec<f64> = (-10..=10)
            .filter(|i| *i != 0)
            .map(|i| f64::from(i) * 0.005)
            .collect();
        let cfg = NonAdditiveConfig {
            spot_shocks: shocks.clone(),
            var_alpha: 0.99,
            curvature_risk_weight: 0.0,
        };

        let combined = aggregate_nodes(
            &cube_from_facts(&[long, short], Hierarchy::new()),
            None,
            &resolver,
            &[],
            &cfg,
        )
        .unwrap();
        let var_combined = combined[0].nonadditive.as_ref().unwrap().var.unwrap();

        let long_only = aggregate_nodes(
            &cube_from_facts(&[long], Hierarchy::new()),
            None,
            &resolver,
            &[],
            &cfg,
        )
        .unwrap();
        let var_long = long_only[0].nonadditive.as_ref().unwrap().var.unwrap();

        assert!(var_long > 0.0);
        assert!(
            var_combined < 1e-6 * var_long,
            "offsetting book VaR {var_combined} should be ~0, not ~{var_long}"
        );

        // With no shocks, the non-additive measures are absent (never spurious 0).
        let none = aggregate_nodes(
            &cube_from_facts(&[long], Hierarchy::new()),
            None,
            &resolver,
            &[],
            &NonAdditiveConfig::default(),
        )
        .unwrap();
        let na = none[0].nonadditive.as_ref().unwrap();
        assert!(na.var.is_none() && na.es.is_none() && na.curvature_spot.is_none());
    }

    /// A missing reporting-numeraire rate fails the request loudly (no silent leg
    /// drop) — the §2.3 fail-loud contract surfaced as `failed_precondition`.
    #[test]
    fn missing_rate_fails_loudly() {
        let p = test_fact(
            1,
            1,
            1,
            1,
            1,
            1,
            usdjpy(),
            OptionType::Put,
            8_000_000.0,
            VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
        );
        // EUR numeraire, but only a USD rate supplied → JPY leg has no rate.
        let resolver = WireResolver::new(&ReportingNumeraire {
            numeraire: "EUR".to_owned(),
            rates: vec![NumeraireRate {
                ccy: "USD".to_owned(),
                rate: 0.92,
            }],
        })
        .unwrap();
        let err = aggregate_nodes(
            &cube_from_facts(&[p], Hierarchy::new()),
            None,
            &resolver,
            &[],
            &NonAdditiveConfig::default(),
        )
        .unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }
}
