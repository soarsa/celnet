//! Celnet single-node hierarchical risk **cube** (`docs/RISK-HIERARCHY.md`
//! §2.1/§2.5/§3.2, `docs/EXPERIENCE-ARCHITECTURE.md` P2-5).
//!
//! # What this crate is
//!
//! An OLAP-style risk-aggregation engine that sits **above** the per-position
//! canonical leaf (`celnet-risk-normalize`, §2.2/§2.3) and **below** the limits /
//! entitlements / server-edge layers. It holds an immutable position-level fact
//! table keyed by the firm's independent organizational dimensions
//! (position → trader → book → desk → ccy-pair → booking-location → legal-entity →
//! firm), and answers the canonical OLAP question: *group the facts by any
//! dimension at any level, and reduce them into a node measure.*
//!
//! The crucial FX-correctness split (`docs/RISK-HIERARCHY.md` §2.5):
//!
//! - **Additive measures** — net canonical Greeks (delta-base, gamma, vega, theta,
//!   vanna, volga, charm, speed, zomma, color) and **vega bucketed by
//!   `(tenor × delta)`** — roll up by **summation of the canonical
//!   (convention-normalized) leaves**. Because the leaves are already
//!   convention-free and the sum is associative/commutative, a roll-up is cheap
//!   and a new fact touches O(depth) ancestor sums.
//! - **Non-additive measures** — **VaR / Expected Shortfall**, **FRTB-SbM
//!   curvature**, and **correlation-weighted vega** — are **NOT** summed from
//!   child results. They are **re-derived per node** over the node's constituent
//!   positions. VaR/ES offers two reconcilable lenses: full **bump-and-revalue**
//!   (the exact oracle, [`Cube::node_var_es`]) and the **AAD sensitivity** scale
//!   path ([`Cube::node_var_es_sensitivity`]) — one reverse-mode
//!   `celnet_vanilla::adjoint_greeks` sweep per position expanded by a second-order
//!   Taylor series across all scenarios.
//!
//! Roll-up and drill-down run over the **same** immutable facts, so a node total
//! is always reconcilable to its constituents (§2.5) — [`NodeAggregate`] carries
//! its constituent positions/leaves alongside the aggregate.
//!
//! # Honest scope & deferred optimisations
//!
//! - **AAD is wired as the scale path; bump-and-revalue is retained as the
//!   oracle.** `docs/RISK-HIERARCHY.md` §3.3 names **adjoint algorithmic
//!   differentiation (AAD)** as the throughput lever, and it is now live:
//!   [`crate::nonadditive::sensitivity_var_es`] computes each position's full Greek
//!   set in ONE genuine reverse-mode `celnet_vanilla::adjoint_greeks` sweep
//!   (O(positions) sweeps) and expands every scenario's P&L by a second-order
//!   Taylor series (O(positions × scenarios) cheap arithmetic), versus the oracle's
//!   O(positions × scenarios) full repricings. The bump-and-revalue
//!   [`crate::nonadditive::historical_var_es`] is **retained, never deleted**, as
//!   the exact reference the AAD lens is reconciled against (within a documented
//!   Taylor tolerance over a moderate shock regime; the gap widens for large shocks
//!   — the honest 2nd-order truncation regime). **The batched-GPU Monte-Carlo
//!   `celnet_gpu::ScenarioPricer` is deliberately NOT wired into this closed-form
//!   VaR path** (mixing MC estimator noise into an exact closed-form reval would be
//!   a numerical regression); the right GPU lever here is a batched *closed-form*
//!   vanilla kernel (`docs/GPU-AT-SCALE-PLAN.md` Workload A / G2), the distinct next
//!   GPU increment.
//! - **Single-node only.** This cube aggregates the facts it holds. Distributed
//!   **cross-shard reduction** (§3.4) over the **designed-only** `celnet-router`
//!   HRW partition map is out of scope. The **shard-merge seam** is
//!   [`NodeAggregate::merge_additive`] (additive aggregates merge directly;
//!   non-additive firm measures must re-derive from gathered constituent facts —
//!   exactly the §3.4 caveat).
//! - **Regulatory weights/pillars are external data**, never compiled-in
//!   (§2.3/§2.11): the `(tenor × delta)` pillar grid, FX curvature risk weight,
//!   and inter-bucket correlations are all caller-supplied (a [`VegaPillarMap`], a
//!   risk-weight argument, a `rho` closure), so SIMM/FRTB recalibrations are data,
//!   not a recompile.
//! - **Convention & numeraire live one layer down.** This crate sums *canonical*
//!   leaves and delegates reporting-numeraire collapse to `celnet-risk-normalize`
//!   ([`NodeAggregate::numeraire_view`]); it owns **no** convention logic and
//!   **fetches no market data** (rates arrive via a `SpotResolver`).
//! - **Cross-pair vol-triangulation sign** (§2.4) is *not* resolved inside the
//!   additive ladder: the ladder only sums vegas sharing a pillar **and** premium
//!   currency; a signed cross-pair correlation aggregation is the
//!   [`Cube::node_correlation_weighted_vega`] reducer's `rho` argument, supplied by
//!   the caller with the explicit quote-convention sign.
//!
//! # Determinism
//!
//! Every reducer is pure and `libm`-routed; for a fixed fact set, scenario set,
//! pillar map and correlation function the results are bit-reproducible.
//!
//! # Provenance
//!
//! The cube/star-schema framing and additive-vs-non-additive split follow
//! `docs/RISK-HIERARCHY.md` §2.1/§2.5; FRTB-SbM curvature and the
//! √(quadratic-form) vega aggregation follow BCBS MAR21 (cited in the design doc).
//! Provenance is in doc comments only; no method/person/vendor name appears in any
//! identifier (guardrail #8).

#![forbid(unsafe_code)]

pub mod additive;
pub mod cube;
pub mod dimension;
pub mod exotic;
pub mod frtb;
pub mod frtb_params;
pub mod nonadditive;
pub mod scenario_grid;

pub use additive::{NetGreeks, VegaLadder, VegaPillar};
pub use cube::{Cube, NodeAggregate, VegaPillarMap};
pub use dimension::{
    BookId, DeskId, DimensionId, EntityId, FactKey, FactMeasure, Hierarchy, LocationId, PositionId,
    RiskFact, TraderId,
};
pub use exotic::{ExoticKind, ExoticLeg, exotic_curvature_legs, exotic_node_pnl};
pub use frtb::{
    CorrelationScenario, CurvatureBucket, FrtbCapital, ResidualInstrument, ResidualKind,
    RiskBucket, SbmCharge, SbmParams, assemble_capital, curvature_class, curvature_legs,
    delta_vega_class, fx_default_risk_charge, node_curvature_bucket, quadratic_form,
    residual_addon,
};
pub use frtb_params::{
    StandardFrtbParams, standard_curvature_buckets, standard_delta_buckets, standard_vega_buckets,
};
pub use nonadditive::{
    PositionSensitivity, Scenario, VarEs, correlation_weighted_vega, historical_var_es, node_pnl,
    node_sensitivities, node_var_es_combined, node_var_es_sensitivity_combined, position_pnl,
    sbm_curvature_spot, sbm_curvature_spot_combined, sensitivity_var_es, shift_carry,
    vanilla_curvature_legs,
};
pub use scenario_grid::{NodeScenarioGrid, analytic_pv_grid, gpu_pv_grid};

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_risk_normalize::{AssetPricer, PositionRisk, StaticSpotResolver, canonicalize};
    use celnet_types::{
        Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
    };

    /// Canonicalize a position through the default seam, unwrapping (FX positions
    /// always price). Keeps the existing tests one-line-changed.
    fn canon(p: &PositionRisk) -> celnet_risk_normalize::CanonicalLeaf {
        canonicalize(p).unwrap()
    }

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }
    fn usdjpy() -> CcyPair {
        CcyPair::new(Ccy::USD, Ccy::JPY)
    }

    /// A pillar map that buckets by the position's tenor-in-days (rounded) and a
    /// fixed 0.50Δ pillar — enough to exercise ladder summation deterministically.
    struct DaysPillar;
    impl VegaPillarMap for DaysPillar {
        fn pillar_of(
            &self,
            _leaf: &celnet_risk_normalize::CanonicalLeaf,
            position: &PositionRisk,
        ) -> VegaPillar {
            let days = (position.inputs.t * 365.0).round() as u32;
            VegaPillar::new(days, 5000)
        }
    }

    fn pos(pair: CcyPair, opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
        PositionRisk::fx(
            pair,
            opt,
            notional,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    }

    fn fact(
        id: u32,
        trader: u32,
        book: u32,
        desk: u32,
        loc: u32,
        ent: u32,
        position: &PositionRisk,
    ) -> RiskFact {
        RiskFact {
            position_id: PositionId(id),
            key: FactKey {
                trader: TraderId(trader),
                book: BookId(book),
                desk: DeskId(desk),
                underlying: position.underlying.clone(),
                location: LocationId(loc),
                entity: EntityId(ent),
            },
            measure: FactMeasure {
                leaf: canon(position),
                position: position.clone(),
                exotic: None,
            },
            surface_version: 1,
        }
    }

    /// **Additive roll-up correctness**: the sum of children equals the parent.
    /// Group by book → each book's net delta equals the sum of its facts' canonical
    /// deltas; the firm net delta equals the sum across all books. A drill-down
    /// (the constituent facts) reconciles exactly to the rolled-up number.
    #[test]
    fn additive_rollup_sum_of_children_equals_parent() {
        let mut cube = Cube::new();
        // Two books on EURUSD; book 1 has two positions, book 2 has one.
        let p1 = pos(
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p2 = pos(
            eurusd(),
            OptionType::Put,
            5_000_000.0,
            VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.04, 0.02),
        );
        let p3 = pos(
            eurusd(),
            OptionType::Call,
            7_000_000.0,
            VanillaInputs::new(1.10, 1.15, 0.09, 1.0, 0.04, 0.02),
        );
        cube.upsert(fact(1, 1, 1, 1, 1, 1, &p1));
        cube.upsert(fact(2, 1, 1, 1, 1, 1, &p2));
        cube.upsert(fact(3, 2, 2, 1, 1, 1, &p3));

        let by_book = cube.group_by(DimensionId::Book, &DaysPillar);
        assert_eq!(by_book.len(), 2);

        // Book 1's net delta = canonical(p1) + canonical(p2).
        let want_b1 = canon(&p1).greeks.delta_base + canon(&p2).greeks.delta_base;
        let b1 = by_book.iter().find(|a| a.group == 1).unwrap();
        assert!(is_close(b1.net_greeks.delta_base, want_b1, 1e-12, 1e-6));
        // Drill-down reconciliation: re-summing the node's retained leaves gives
        // exactly the node aggregate (the §2.5 reconcilable invariant).
        let resum: f64 = b1.leaves.iter().map(|l| l.greeks.delta_base).sum();
        assert!(is_close(resum, b1.net_greeks.delta_base, 0.0, 1e-9));
        assert_eq!(b1.positions.len(), 2);

        // Firm = sum across both books.
        let firm = cube.firm_aggregate(&DaysPillar);
        let want_firm = canon(&p1).greeks.delta_base
            + canon(&p2).greeks.delta_base
            + canon(&p3).greeks.delta_base;
        assert!(is_close(firm.net_greeks.delta_base, want_firm, 1e-12, 1e-6));
        // And firm == Σ book-level aggregates (the roll-up is associative).
        let sum_books: f64 = by_book.iter().map(|a| a.net_greeks.delta_base).sum();
        assert!(is_close(firm.net_greeks.delta_base, sum_books, 0.0, 1e-9));
    }

    /// The vega **ladder** is additive: two facts with the same tenor pillar net
    /// their vega in that bucket; a different tenor lands in a separate bucket.
    #[test]
    fn vega_ladder_buckets_are_additive() {
        let mut cube = Cube::new();
        let p1 = pos(
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p2 = pos(
            eurusd(),
            OptionType::Put,
            5_000_000.0,
            VanillaInputs::new(1.10, 1.08, 0.11, 1.0, 0.04, 0.02),
        );
        let p3 = pos(
            eurusd(),
            OptionType::Call,
            8_000_000.0,
            VanillaInputs::new(1.10, 1.11, 0.10, 0.5, 0.04, 0.02),
        );
        cube.upsert(fact(1, 1, 1, 1, 1, 1, &p1));
        cube.upsert(fact(2, 1, 1, 1, 1, 1, &p2));
        cube.upsert(fact(3, 1, 1, 1, 1, 1, &p3));

        let firm = cube.firm_aggregate(&DaysPillar);
        let pillar_1y = VegaPillar::new(365, 5000);
        let pillar_6m = VegaPillar::new(183, 5000);
        // The 1Y pillar nets p1 + p2; the 6M pillar holds p3.
        let want_1y = canon(&p1).greeks.vega + canon(&p2).greeks.vega;
        let want_6m = canon(&p3).greeks.vega;
        assert!(is_close(
            firm.vega_ladder.vega_in(pillar_1y),
            want_1y,
            1e-12,
            1e-6
        ));
        assert!(is_close(
            firm.vega_ladder.vega_in(pillar_6m),
            want_6m,
            1e-12,
            1e-6
        ));
        // Ladder total == net_greeks.vega (same currency here, all EURUSD/USD).
        assert!(is_close(
            firm.vega_ladder.total(),
            firm.net_greeks.vega,
            1e-12,
            1e-6
        ));
    }

    /// **Parent-pointer hierarchy**: grouping by Desk resolves each book's
    /// configured desk. Two books mapped to the same desk roll into one desk node.
    #[test]
    fn desk_rollup_uses_book_parent_pointer() {
        let mut h = Hierarchy::new();
        h.set_book_desk(BookId(10), DeskId(99)); // book 10 → G10 desk 99
        h.set_book_desk(BookId(11), DeskId(99)); // book 11 → same desk
        let mut cube = Cube::with_hierarchy(h);
        let p1 = pos(
            eurusd(),
            OptionType::Call,
            4_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p2 = pos(
            eurusd(),
            OptionType::Call,
            6_000_000.0,
            VanillaInputs::new(1.10, 1.13, 0.10, 1.0, 0.04, 0.02),
        );
        // The fact's own desk key is deliberately wrong (0); the parent pointer wins.
        cube.upsert(fact(1, 1, 10, 0, 1, 1, &p1));
        cube.upsert(fact(2, 2, 11, 0, 1, 1, &p2));
        let by_desk = cube.group_by(DimensionId::Desk, &DaysPillar);
        assert_eq!(by_desk.len(), 1, "both books must roll into desk 99");
        let desk = &by_desk[0];
        assert_eq!(desk.group, 99);
        let want = canon(&p1).greeks.delta_base + canon(&p2).greeks.delta_base;
        assert!(is_close(desk.net_greeks.delta_base, want, 1e-12, 1e-6));
    }

    /// **Non-additive VaR is NOT the sum of child VaRs.** A long call and a short
    /// call of equal size on the same underlying have offsetting P&L, so the
    /// combined VaR is far below the sum of the two standalone VaRs — proving the
    /// node measure must be re-derived, not summed.
    #[test]
    fn var_is_non_additive_diversification() {
        // Symmetric spot scenarios.
        let scen: Vec<Scenario> = (-10..=10)
            .filter(|i| *i != 0)
            .map(|i| Scenario::spot(f64::from(i) * 0.005))
            .collect();
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01);
        let long = pos(eurusd(), OptionType::Call, 10_000_000.0, inputs);
        let short = pos(eurusd(), OptionType::Call, -10_000_000.0, inputs);

        let var_long =
            historical_var_es(&AssetPricer, std::slice::from_ref(&long), &scen, 0.99).var;
        let var_short =
            historical_var_es(&AssetPricer, std::slice::from_ref(&short), &scen, 0.99).var;
        let var_combined = historical_var_es(&AssetPricer, &[long, short], &scen, 0.99).var;

        assert!(var_long > 0.0 && var_short > 0.0);
        // A perfectly offsetting book has ~zero VaR, far below the sum of legs.
        assert!(
            var_combined < 1e-6 * var_long,
            "offsetting book VaR {var_combined} should be ~0, not {} (sum of legs)",
            var_long + var_short
        );
    }

    /// **The AAD sensitivity lens reconciles to the oracle THROUGH the cube API.**
    /// A multi-position firm node's `node_var_es_sensitivity` (one adjoint sweep per
    /// position, Taylor-expanded) matches the bump-and-revalue `node_var_es` oracle
    /// to the documented worst-corner Taylor tolerance (8% relative at ±5%/±2pt; see
    /// `nonadditive::sensitivity_var_reconciles_to_oracle_moderate_shocks`) over a
    /// moderate ladder — proving the wired lens, not just the free function.
    #[test]
    fn cube_sensitivity_var_reconciles_to_oracle() {
        let mut cube = Cube::new();
        cube.upsert(fact(
            1,
            1,
            1,
            1,
            1,
            1,
            &pos(
                eurusd(),
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
        ));
        cube.upsert(fact(
            2,
            1,
            1,
            1,
            1,
            1,
            &pos(
                eurusd(),
                OptionType::Put,
                6_000_000.0,
                VanillaInputs::new(1.10, 1.06, 0.115, 0.75, 0.04, 0.02),
            ),
        ));
        cube.upsert(fact(
            3,
            1,
            1,
            1,
            1,
            1,
            &pos(
                eurusd(),
                OptionType::Call,
                -4_000_000.0,
                VanillaInputs::new(1.10, 1.15, 0.095, 1.5, 0.04, 0.02),
            ),
        ));
        let node = cube.firm_aggregate(&DaysPillar);
        // Moderate ladder: spot ±5% × vol ±2 vol-pts.
        let mut scen = Vec::new();
        for si in -5..=5 {
            for vj in -4..=4 {
                scen.push(Scenario {
                    spot_rel: f64::from(si) * 0.01,
                    vol_abs: f64::from(vj) * 0.005,
                    discount_abs: 0.0,
                    carry_abs: 0.0,
                });
            }
        }
        let oracle = Cube::node_var_es(&AssetPricer, &node, &scen, 0.99);
        let fast = Cube::node_var_es_sensitivity(&AssetPricer, &node, &scen, 0.99);
        assert!(oracle.var > 0.0);
        assert!(
            is_close(fast.var, oracle.var, 8e-2, 1e-3),
            "cube AAD VaR {} vs oracle {}",
            fast.var,
            oracle.var
        );
        assert!(is_close(fast.es, oracle.es, 8e-2, 1e-3));
    }

    /// **VaR/ES correctness on a known distribution.** A single long call repriced
    /// over a symmetric spot grid: the 90% VaR must equal the (negated) 10th-worst
    /// percentile P&L computed independently here, and ES ≥ VaR.
    #[test]
    fn var_es_matches_independent_quantile() {
        let scen: Vec<Scenario> = (-50..=50)
            .map(|i| Scenario::spot(f64::from(i) * 0.001))
            .collect();
        let inputs = VanillaInputs::new(1.2500, 1.2600, 0.09, 0.75, 0.02, 0.01);
        let p = pos(
            CcyPair::new(Ccy::GBP, Ccy::USD),
            OptionType::Call,
            5_000_000.0,
            inputs,
        );

        // Independent reference: compute the P&L vector, sort, pick the quantile.
        let mut pnl: Vec<f64> = scen
            .iter()
            .map(|s| position_pnl(&AssetPricer, &p, *s))
            .collect();
        pnl.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = pnl.len();
        let alpha = 0.90;
        let tail = (((1.0 - alpha) * n as f64).floor() as usize).max(1);
        let ref_var = -pnl[tail - 1];
        let ref_es = -(pnl[..tail].iter().sum::<f64>() / tail as f64);

        let got = historical_var_es(&AssetPricer, &[p], &scen, alpha);
        assert!(
            is_close(got.var, ref_var, 1e-12, 1e-6),
            "VaR {} vs ref {ref_var}",
            got.var
        );
        assert!(
            is_close(got.es, ref_es, 1e-12, 1e-6),
            "ES {} vs ref {ref_es}",
            got.es
        );
        // ES is at least as severe as VaR.
        assert!(got.es >= got.var - 1e-9);
    }

    /// **FRTB-SbM curvature charges short-gamma, not long-gamma.** Per MAR21 the
    /// curvature charge is `max(CVR⁺, CVR⁻, 0)`: a *long*-gamma (long-option) book
    /// has a convexity *gain* on both up/down shocks, so both CVR legs are negative
    /// and the charge floors at **zero** (curvature never rewards long gamma). A
    /// *short* straddle is short gamma → positive curvature charge. We assert both
    /// directions and cross-check the arithmetic against an independent reprice.
    #[test]
    fn curvature_charges_short_gamma_only() {
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01);
        // Spot curvature risk weight (externally supplied; representative FX value).
        let rw = 0.20;

        // Long straddle → long gamma → zero curvature charge.
        let long_node = {
            let mut c = Cube::new();
            c.upsert(fact(
                1,
                1,
                1,
                1,
                1,
                1,
                &pos(eurusd(), OptionType::Call, 10_000_000.0, inputs),
            ));
            c.upsert(fact(
                2,
                1,
                1,
                1,
                1,
                1,
                &pos(eurusd(), OptionType::Put, 10_000_000.0, inputs),
            ));
            c.firm_aggregate(&DaysPillar)
        };
        assert_eq!(
            Cube::node_curvature_spot(&AssetPricer, &long_node, rw),
            0.0,
            "long-gamma book must have zero curvature charge"
        );

        // Short straddle → short gamma → positive curvature charge.
        let call = pos(eurusd(), OptionType::Call, -10_000_000.0, inputs);
        let put = pos(eurusd(), OptionType::Put, -10_000_000.0, inputs);
        let short_node = {
            let mut c = Cube::new();
            c.upsert(fact(1, 1, 1, 1, 1, 1, &call));
            c.upsert(fact(2, 1, 1, 1, 1, 1, &put));
            c.firm_aggregate(&DaysPillar)
        };
        let cvr = Cube::node_curvature_spot(&AssetPricer, &short_node, rw);
        assert!(
            cvr > 0.0,
            "short straddle must have positive curvature charge, got {cvr}"
        );

        // Cross-check the math against an independent up/down reprice net of delta,
        // calling the FX leaf directly on the lowered inputs (the oracle path).
        let positions = [call, put];
        let vi = |p: &PositionRisk| celnet_core::carry::fx_vanilla_inputs(&p.inputs).unwrap();
        let base: f64 = positions
            .iter()
            .map(|p| celnet_vanilla::price(p.option, &vi(p)) * p.notional_base)
            .sum();
        let reprice = |mult: f64| -> f64 {
            positions
                .iter()
                .map(|p| {
                    let b = vi(p);
                    let s =
                        VanillaInputs::new(b.spot * mult, b.strike, b.vol, b.t, b.r_dom, b.r_for);
                    celnet_vanilla::price(p.option, &s) * p.notional_base
                })
                .sum()
        };
        let linear: f64 = positions
            .iter()
            .map(|p| {
                celnet_vanilla::greeks(p.option, &vi(p)).delta_spot
                    * p.notional_base
                    * rw
                    * p.inputs.spot
            })
            .sum();
        let cvr_up = -((reprice(1.0 + rw) - base) - linear);
        let cvr_down = -((reprice(1.0 - rw) - base) + linear);
        let ref_cvr = cvr_up.max(cvr_down).max(0.0);
        assert!(
            is_close(cvr, ref_cvr, 1e-9, 1e-3),
            "curvature {cvr} vs ref {ref_cvr}"
        );
    }

    /// **Correlation-weighted vega** reduces to the plain Euclidean norm at ρ=0 and
    /// to a straight sum at ρ=1; it is non-additive (≠ Σ weighted vegas) in between.
    #[test]
    fn correlation_weighted_vega_bounds() {
        let w = [3.0, 4.0]; // weighted bucket vegas.
        // ρ = 0 → √(9+16) = 5.
        let zero = correlation_weighted_vega(&w, |_, _| 0.0);
        assert!(is_close(zero, 5.0, 1e-12, 1e-9));
        // ρ = 1 → √((3+4)²) = 7 (perfectly correlated → straight sum).
        let one = correlation_weighted_vega(&w, |_, _| 1.0);
        assert!(is_close(one, 7.0, 1e-12, 1e-9));
        // 0 < ρ < 1 lies strictly between, and is NOT the sum (non-additive).
        let half = correlation_weighted_vega(&w, |_, _| 0.5);
        assert!(half > zero && half < one);
        // A non-PSD (negative) form floors at zero, never NaN.
        let floored = correlation_weighted_vega(&[1.0, 1.0], |_, _| -1.0);
        assert_eq!(floored, 0.0);
    }

    /// **Cross-pair numeraire view through the cube.** A two-pair firm node
    /// collapses its base-ccy deltas into a single USD scalar via
    /// `celnet-risk-normalize`, and the delta vector nets the shared USD legs.
    #[test]
    fn firm_numeraire_view_collapses_cross_pair() {
        let mut cube = Cube::new();
        let p1 = pos(
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p2 = pos(
            usdjpy(),
            OptionType::Put,
            8_000_000.0,
            VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
        );
        cube.upsert(fact(1, 1, 1, 1, 1, 1, &p1));
        cube.upsert(fact(2, 2, 2, 1, 1, 1, &p2));
        let firm = cube.firm_aggregate(&DaysPillar);
        let usd = StaticSpotResolver::new(Ccy::USD, &[(Ccy::EUR, 1.10), (Ccy::JPY, 1.0 / 156.0)]);
        let view = firm.numeraire_view(&usd).unwrap();
        assert_eq!(view.numeraire, Ccy::USD);
        // Collapsing the netted delta vector equals the reported scalar.
        let recomputed = view.delta_vector.in_numeraire(&usd).unwrap();
        assert!(is_close(recomputed, view.delta_numeraire, 1e-9, 1e-3));
        // The USD leg is EURUSD's funding leg + USDJPY's base hedge (§2.3 netting).
        let usd_leg = view.delta_vector.amount_in(Ccy::USD);
        let l1 = canon(&p1);
        let l2 = canon(&p2);
        let expected_usd = -l1.greeks.delta_base * l1.spot + l2.greeks.delta_base;
        assert!(is_close(usd_leg, expected_usd, 1e-9, 1e-3));
    }

    /// **Cross-shard merge seam.** Merging two single-shard additive aggregates
    /// equals aggregating the union in one cube — the §3.4 additive shard-merge is
    /// associative; non-additive measures (re-derived from the merged positions)
    /// are unaffected.
    #[test]
    fn shard_merge_is_associative_for_additive() {
        let p1 = pos(
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p2 = pos(
            eurusd(),
            OptionType::Put,
            5_000_000.0,
            VanillaInputs::new(1.10, 1.08, 0.11, 1.0, 0.04, 0.02),
        );
        // Shard A holds p1, shard B holds p2.
        let mut a = Cube::new();
        a.upsert(fact(1, 1, 1, 1, 1, 1, &p1));
        let mut b = Cube::new();
        b.upsert(fact(2, 1, 1, 1, 1, 1, &p2));
        let mut merged = a.firm_aggregate(&DaysPillar);
        merged.merge_additive(&b.firm_aggregate(&DaysPillar));

        // Reference: one cube with both facts.
        let mut whole = Cube::new();
        whole.upsert(fact(1, 1, 1, 1, 1, 1, &p1));
        whole.upsert(fact(2, 1, 1, 1, 1, 1, &p2));
        let ref_agg = whole.firm_aggregate(&DaysPillar);

        assert!(is_close(
            merged.net_greeks.delta_base,
            ref_agg.net_greeks.delta_base,
            0.0,
            1e-9
        ));
        assert!(is_close(
            merged.vega_ladder.total(),
            ref_agg.vega_ladder.total(),
            0.0,
            1e-9
        ));
        assert_eq!(merged.positions.len(), ref_agg.positions.len());
    }

    /// **Upsert keeps one current fact per position** (append-only with supersede):
    /// re-valuing position 1 replaces its fact, it does not double-count.
    #[test]
    fn upsert_supersedes_not_duplicates() {
        let mut cube = Cube::new();
        let p_old = pos(
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p_new = pos(
            eurusd(),
            OptionType::Call,
            20_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        cube.upsert(fact(1, 1, 1, 1, 1, 1, &p_old));
        cube.upsert(fact(1, 1, 1, 1, 1, 1, &p_new));
        assert_eq!(cube.len(), 1);
        let firm = cube.firm_aggregate(&DaysPillar);
        // Greeks scale linearly in notional → the new (2×) fact is the only one.
        let want = canon(&p_new).greeks.delta_base;
        assert!(is_close(firm.net_greeks.delta_base, want, 1e-12, 1e-6));
    }

    /// **A non-FX (equity) fact rolls up through the cube by its OWN leaf delta.** A
    /// firm built from a single equity position has `net_greeks.delta_base` equal to
    /// the canonical leaf's delta — proving the cross-asset fact flows through the
    /// generalized fact table and seam (the `Underlying` axis carries the equity arm,
    /// the roll-up sums the equity leaf's own Greeks, never an FX proxy).
    #[test]
    fn equity_fact_rolls_up_by_its_own_leaf() {
        use celnet_core::carry::CarryInputs;
        use celnet_types::{Carry, EquityRef, Symbol};
        let u = Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD));
        let ci = CarryInputs::new(
            100.0,
            105.0,
            0.20,
            1.0,
            u.clone(),
            Carry::CostOfCarry { r: 0.03, b: 0.01 },
        );
        let eq = PositionRisk::carry(u, OptionType::Call, 1_000.0, ci);
        let mut cube = Cube::new();
        cube.upsert(fact(1, 1, 1, 1, 1, 1, &eq));
        // The fact's underlying axis is the equity arm.
        let by_under = cube.group_by(DimensionId::Underlying, &DaysPillar);
        assert_eq!(by_under.len(), 1);
        let firm = cube.firm_aggregate(&DaysPillar);
        let want = canon(&eq).greeks.delta_base;
        assert!(is_close(firm.net_greeks.delta_base, want, 1e-12, 1e-9));
        // And it is the equity leaf's delta, not zero / not FX.
        assert!(firm.net_greeks.delta_base.abs() > 1.0);
    }
}
