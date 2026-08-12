//! Celnet **limits / entitlements** layer (`docs/RISK-HIERARCHY.md` §5,
//! `docs/EXPERIENCE-ARCHITECTURE.md` P2-7).
//!
//! # What this crate is
//!
//! The pre/post-trade **limit framework** that sits **above** the
//! [`celnet_risk_cube`] aggregation engine. The cube answers *"what is the netted /
//! re-derived risk at this node?"*; this crate answers *"is that within the limits
//! set at this node, and what happens if a proposed trade pushes it over?"*.
//!
//! Limits cascade **down the same hierarchy** as risk (trader → book → desk →
//! ccy-pair → location → entity → firm, RH §5.2) and are checked **at multiple
//! nodes simultaneously**: a single trade consumes limit at its trader, book, desk,
//! ccy-pair, and entity nodes at once. That is the operational reason the cube must
//! aggregate in real time — and why this crate addresses limits by a
//! [`LimitScope`] that maps 1:1 onto a cube group, so the same `group_by` /
//! `firm_aggregate` results drive both the risk display and the limit check.
//!
//! The three layers:
//!
//! 1. **Taxonomy & thresholds** ([`limit`]) — the industry-standard limit set
//!    (greek / bucketed-vega / tenor-bucket / concentration / VaR-ES / stop-loss,
//!    RH §5.1), each **soft** (warn) or **hard** (block), with **utilization** and
//!    **RAG status** as first-class node measures (RH §5.2).
//! 2. **The limit tree** ([`tree`]) — limits configured at any hierarchy scope, and
//!    the [`ScopePath`] a position is simultaneously constrained on (resolved
//!    through the cube's `book→desk` / `location→entity` parent pointers).
//! 3. **The checks** ([`check`]) — exposure extraction off a cube node, **pre-trade**
//!    projection + accept/warn/reject, and **post-trade** continuous monitoring with
//!    breach detection + escalation status (RH §5.3).
//!
//! # Additive vs non-additive (RH §2.5/§5.3)
//!
//! The crate inherits the cube's correctness split. **Additive** limits (greeks,
//! bucketed/tenor vega, concentration) read the node's already-summed
//! `NetGreeks`/`VegaLadder` in O(1), so the pre-trade additive-Greek path stays in
//! the µs-class budget (RH §3.5). **Non-additive** limits (VaR / ES / stop-loss)
//! consume a loss number **re-derived per node** by the cube's bump-and-revalue
//! reducers (RH §2.5), evaluated on the slower recompute-trigger cadence (RH §3.3);
//! the check layer takes that number as input ([`NonAdditiveExposure`]) and never
//! summed-child-VaRs (which would be wrong — VaR diversifies).
//!
//! # Scope & honesty
//!
//! - **Pure & deterministic, no IO / no market data.** The limit tree holds only
//!   the constraints; exposures are read from caller-supplied cube nodes; every
//!   classification is a pure ratio comparison. For a fixed tree + node set the
//!   result is bit-reproducible.
//! - **The crate evaluates limits; it does not own the cascade *constraint
//!   solver*.** RH §5.2's "child limits constrained by parents" (a desk's limit
//!   bounding the sum of its books' limits) is a **configuration-time** validation
//!   that belongs to the admin/entitlements path; this crate stores and evaluates
//!   whatever tree it is given. That cascade-consistency check is the named,
//!   **deferred** companion (it would live in `celnet-entitlements` / the admin
//!   workflow, P2-8) — not faked here.
//! - **Limit thresholds are external data**, never compiled-in: caps and the
//!   amber/red warning bands (RH §5.2's illustrative 80 %/90 %) are
//!   [`LimitSpec`] fields, so a re-calibration is data, not a recompile.
//! - **Entitlements vs limits.** This crate is the *limits* half of the
//!   limits/entitlements layer; the entitlement pre-aggregation predicate (EA §3,
//!   P2-8) is the sibling `celnet-entitlements` crate. A [`LimitScope`] is exactly
//!   the granularity an entitlement grant is expressed over, so the two compose
//!   cleanly.
//!
//! # Provenance
//!
//! The limit taxonomy, soft/hard cascade and pre/post-trade + escalation workflow
//! follow `docs/RISK-HIERARCHY.md` §5 (which cites the MiFID II / SEC 15c3-5
//! pre-trade mandate and the Murex/Calypso escalation workflows). Provenance is in
//! doc comments only; no method/person/vendor name appears in any identifier
//! (guardrail #8).

#![forbid(unsafe_code)]

pub mod check;
pub mod limit;
pub mod tree;

pub use check::{
    EscalationStatus, IncrementalTrade, LimitCheck, NonAdditiveExposure, PreTradeDecision,
    PreTradeResult, ScopeMonitor, check_scope, check_scope_rates, exposure_of, exposure_of_rates,
    post_trade_check, post_trade_check_rates, pre_trade_check, pre_trade_check_mixed,
};
pub use limit::{ConcentrationMetric, Enforcement, LimitMetric, LimitSpec, RagStatus, Utilization};
pub use tree::{LimitScope, LimitTree, ScopePath};

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_cube::{
        BookId, Cube, DeskId, DimensionId, EntityId, FactKey, FactMeasure, Hierarchy, LocationId,
        NetGreeks, NodeAggregate, PositionId, RiskFact, Scenario, TraderId, VegaLadder, VegaPillar,
        VegaPillarMap, test_support::DigitalTestPricer,
    };
    use celnet_risk_normalize::{AssetPricer, CanonicalLeaf, PositionRisk, canonicalize};
    use celnet_types::{
        Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
    };

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    /// A pillar map bucketing by integer-day tenor and a fixed 0.50Δ pillar.
    struct DaysPillar;
    impl VegaPillarMap for DaysPillar {
        fn pillar_of(&self, _leaf: &CanonicalLeaf, position: &PositionRisk) -> VegaPillar {
            VegaPillar::new((position.inputs.t * 365.0).round() as u32, 5000)
        }
    }

    fn pos(opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
        PositionRisk::fx(
            eurusd(),
            opt,
            notional,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    }

    fn fact(id: u32, trader: u32, book: u32, desk: u32, position: PositionRisk) -> RiskFact {
        RiskFact {
            position_id: PositionId(id),
            key: FactKey {
                trader: TraderId(trader),
                book: BookId(book),
                desk: DeskId(desk),
                underlying: position.underlying.clone(),
                location: LocationId(1),
                entity: EntityId(1),
            },
            measure: FactMeasure {
                leaf: canonicalize(&position).unwrap(),
                position,
                exotic: None,
            },
            surface_version: 1,
        }
    }

    /// An empty node aggregate (all fields public) — a scope with no current risk.
    fn empty_node() -> NodeAggregate {
        NodeAggregate {
            group: 0,
            net_greeks: NetGreeks::zero(),
            vega_ladder: VegaLadder::new(),
            positions: Vec::new(),
            leaves: Vec::new(),
            exotic_legs: Vec::new(),
        }
    }

    fn one_book_cube(positions: &[(u32, PositionRisk)]) -> Cube {
        let mut cube = Cube::new();
        for (id, p) in positions {
            cube.upsert(fact(*id, 1, 1, 1, p.clone()));
        }
        cube
    }

    // ---- limit::utilization / RAG math --------------------------------------

    /// **Utilization is `|exposure| / cap`, and RAG bands classify it exactly.**
    #[test]
    fn utilization_ratio_and_rag_bands() {
        let lim = LimitSpec::hard(LimitMetric::Delta, 10_000_000.0); // 80/90 bands.
        // 5mm of a 10mm cap → 0.5 → green.
        let u = lim.classify(5_000_000.0);
        assert_eq!(u.ratio, 0.5);
        assert_eq!(u.status, RagStatus::Green);
        assert_eq!(u.headroom(), 5_000_000.0);
        // Sign-agnostic: -8.5mm → 0.85 → amber (≥0.80, <0.90).
        assert_eq!(lim.classify(-8_500_000.0).status, RagStatus::Amber);
        // 9.5mm → 0.95 → red (≥0.90, ≤1.0).
        assert_eq!(lim.classify(9_500_000.0).status, RagStatus::Red);
        // 10mm exactly → 1.0 → still red, not a breach (at the cap, not over).
        assert_eq!(lim.classify(10_000_000.0).status, RagStatus::Red);
        // 10.0000001mm → breach.
        let over = lim.classify(10_000_001.0);
        assert_eq!(over.status, RagStatus::Breach);
        assert!(over.status.is_breach());
        assert!(over.headroom() < 0.0);
    }

    /// A **non-positive cap** is never silently unbounded: any exposure is a breach.
    #[test]
    fn zero_cap_is_a_breach_not_unbounded() {
        let lim = LimitSpec::hard(LimitMetric::Vega, 0.0);
        assert_eq!(lim.classify(0.0).status, RagStatus::Green); // no exposure, no breach.
        let u = lim.classify(1.0);
        assert!(u.ratio.is_infinite());
        assert_eq!(u.status, RagStatus::Breach);
    }

    /// **Custom warning bands** are honoured and clamped to a valid ordering.
    #[test]
    fn custom_bands_clamp_and_apply() {
        let lim = LimitSpec::soft(LimitMetric::Vega, 100.0).with_bands(0.5, 0.7);
        assert_eq!(lim.classify(60.0).status, RagStatus::Amber); // 0.60 ≥ 0.50.
        assert_eq!(lim.classify(80.0).status, RagStatus::Red); // 0.80 ≥ 0.70.
        // An inverted band (red < amber) is clamped so red ≥ amber, never inverting.
        let bad = LimitSpec::hard(LimitMetric::Delta, 10.0).with_bands(0.9, 0.1);
        assert!(bad.red >= bad.amber);
    }

    // ---- check::exposure_of off a cube node ---------------------------------

    /// **Exposure extraction reads the right additive measure off a node.** A delta
    /// limit sees `net_greeks.delta_base`; a vega-bucket limit sees the matching
    /// ladder bucket; a tenor-vega limit sums all delta pillars in that tenor.
    #[test]
    fn exposure_extraction_matches_node_aggregate() {
        let p1 = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p2 = pos(
            OptionType::Put,
            5_000_000.0,
            VanillaInputs::new(1.10, 1.08, 0.11, 1.0, 0.04, 0.02),
        );
        let cube = one_book_cube(&[(1, p1.clone()), (2, p2.clone())]);
        let node = cube.firm_aggregate(&DaysPillar);
        let na = NonAdditiveExposure::default();

        let want_delta = canonicalize(&p1).unwrap().greeks.delta_base
            + canonicalize(&p2).unwrap().greeks.delta_base;
        assert_eq!(exposure_of(&node, LimitMetric::Delta, &na), want_delta);

        let pillar_1y = VegaPillar::new(365, 5000);
        let want_vega =
            canonicalize(&p1).unwrap().greeks.vega + canonicalize(&p2).unwrap().greeks.vega;
        assert_eq!(
            exposure_of(&node, LimitMetric::VegaBucket(pillar_1y), &na),
            want_vega
        );
        // Both positions are 1Y → the tenor cut equals the bucket here.
        assert_eq!(
            exposure_of(&node, LimitMetric::TenorVega { tenor_days: 365 }, &na),
            want_vega
        );
    }

    /// **Concentration uses gross (un-netted) magnitude.** A long+short pair that
    /// nets to ~0 delta still has a large gross delta concentration — the
    /// concentration metric charges it where the net greek would not.
    #[test]
    fn concentration_is_gross_not_net() {
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01);
        let long = pos(OptionType::Call, 10_000_000.0, inputs);
        let short = pos(OptionType::Call, -10_000_000.0, inputs);
        let cube = one_book_cube(&[(1, long), (2, short)]);
        let node = cube.firm_aggregate(&DaysPillar);
        let na = NonAdditiveExposure::default();

        // Net delta ≈ 0 (offsetting legs).
        let net = exposure_of(&node, LimitMetric::Delta, &na).abs();
        // Gross concentration ≈ 2× one leg's |delta|.
        let gross = exposure_of(
            &node,
            LimitMetric::Concentration(ConcentrationMetric::Delta),
            &na,
        );
        assert!(net < 1e-6, "net delta should net to ~0, got {net}");
        assert!(
            gross > 1e6,
            "gross concentration should be large, got {gross}"
        );
    }

    /// **Non-additive VaR exposure** is read from the supplied loss number (and is
    /// `0` when not evaluated this cycle — never spuriously breaching).
    #[test]
    fn nonadditive_var_exposure_from_reducer() {
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01);
        let cube = one_book_cube(&[(1, pos(OptionType::Call, 10_000_000.0, inputs))]);
        let node = cube.firm_aggregate(&DaysPillar);
        let scen: Vec<Scenario> = (-10..=10)
            .filter(|i| *i != 0)
            .map(|i| Scenario::spot(f64::from(i) * 0.005))
            .collect();
        let na = NonAdditiveExposure::from_scenarios(
            &AssetPricer,
            &DigitalTestPricer,
            &node,
            &scen,
            0.99,
        );
        let var = exposure_of(&node, LimitMetric::Var, &na);
        assert!(var > 0.0);
        assert_eq!(var, na.var.unwrap());
        // Unevaluated VaR reads 0 → can't breach.
        let empty = NonAdditiveExposure::default();
        assert_eq!(exposure_of(&node, LimitMetric::Var, &empty), 0.0);
    }

    // ---- tree + scope path --------------------------------------------------

    /// **The limit tree stores per-scope limits and supersedes by metric.** Setting
    /// a desk delta cap twice keeps one current limit; a different metric coexists.
    #[test]
    fn tree_set_supersedes_by_metric() {
        let mut tree = LimitTree::new();
        let desk = LimitScope::Desk(DeskId(7));
        tree.set(desk, LimitSpec::hard(LimitMetric::Delta, 10_000_000.0));
        tree.set(desk, LimitSpec::hard(LimitMetric::Vega, 500_000.0));
        tree.set(desk, LimitSpec::hard(LimitMetric::Delta, 20_000_000.0)); // supersede.
        let limits = tree.at(desk);
        assert_eq!(limits.len(), 2, "delta superseded, vega coexists");
        let delta = limits
            .iter()
            .find(|l| l.metric == LimitMetric::Delta)
            .unwrap();
        assert_eq!(delta.cap, 20_000_000.0);
        assert_eq!(tree.iter().count(), 2);
    }

    /// **A position's scope path resolves desk/entity through the hierarchy parent
    /// pointers**, so a limit set at the desk is on the path even when the fact's
    /// own desk key is unset.
    #[test]
    fn scope_path_resolves_desk_via_parent_pointer() {
        let mut h = Hierarchy::new();
        h.set_book_desk(BookId(10), DeskId(99));
        let key = FactKey {
            trader: TraderId(1),
            book: BookId(10),
            desk: DeskId(0), // deliberately unset; the parent pointer wins.
            underlying: Underlying::Fx(eurusd()),
            location: LocationId(1),
            entity: EntityId(1),
        };
        let path = ScopePath::resolve(&key, &h);
        let scopes: Vec<_> = path.scopes().collect();
        assert!(scopes.contains(&LimitScope::Desk(DeskId(99))));
        assert!(scopes.contains(&LimitScope::Trader(TraderId(1))));
        assert_eq!(scopes.last(), Some(&LimitScope::Firm));
    }

    // ---- pre-trade -----------------------------------------------------------

    /// **Pre-trade hard breach rejects.** A book sits just under a delta cap; a
    /// proposed trade whose incremental delta pushes it over the cap is **rejected**
    /// by the pre-trade check, and the offending hard breach is reported.
    #[test]
    fn pre_trade_hard_breach_rejects() {
        // Current book: a long call with ~+delta.
        let current_pos = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let cube = one_book_cube(&[(1, current_pos)]);
        let book_node = cube.group_by(DimensionId::Book, &DaysPillar)[0].clone();
        let current_delta = book_node.net_greeks.delta_base;

        // A delta cap set just above current exposure, so any further +delta breaches.
        let cap = current_delta.abs() + 1_000_000.0;
        let mut tree = LimitTree::new();
        tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::hard(LimitMetric::Delta, cap),
        );

        // The proposed trade: another long call adding +delta beyond the headroom.
        let proposed = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let leaf = canonicalize(&proposed).unwrap();
        let incremental = IncrementalTrade::from_leaf(&leaf, VegaPillar::new(365, 5000));

        let path = ScopePath::resolve(
            &FactKey {
                trader: TraderId(1),
                book: BookId(1),
                desk: DeskId(1),
                underlying: Underlying::Fx(eurusd()),
                location: LocationId(1),
                entity: EntityId(1),
            },
            &Hierarchy::new(),
        );

        let result = pre_trade_check(
            &tree,
            &path,
            &incremental,
            |scope| {
                if scope == LimitScope::Book(BookId(1)) {
                    book_node.clone()
                } else {
                    empty_node()
                }
            },
            |_| NonAdditiveExposure::default(),
        );
        assert_eq!(result.decision, PreTradeDecision::Reject);
        assert!(!result.allowed());
        assert_eq!(result.hard_breaches().count(), 1);
        let breach = result.hard_breaches().next().unwrap();
        assert_eq!(breach.scope, LimitScope::Book(BookId(1)));
        assert_eq!(breach.limit.metric, LimitMetric::Delta);
    }

    /// **Pre-trade within headroom accepts; a soft over-cap warns (does not block).**
    #[test]
    fn pre_trade_accept_and_soft_warn() {
        let current_pos = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let cube = one_book_cube(&[(1, current_pos)]);
        let book_node = cube.group_by(DimensionId::Book, &DaysPillar)[0].clone();
        let current_delta = book_node.net_greeks.delta_base.abs();

        let proposed = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let leaf = canonicalize(&proposed).unwrap();
        let incr_delta = leaf.greeks.delta_base.abs();
        let incremental = IncrementalTrade::from_leaf(&leaf, VegaPillar::new(365, 5000));
        let path = ScopePath::resolve(
            &FactKey {
                trader: TraderId(1),
                book: BookId(1),
                desk: DeskId(1),
                underlying: Underlying::Fx(eurusd()),
                location: LocationId(1),
                entity: EntityId(1),
            },
            &Hierarchy::new(),
        );

        // (a) A generous cap → accept.
        let mut accept_tree = LimitTree::new();
        accept_tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::hard(LimitMetric::Delta, (current_delta + incr_delta) * 10.0),
        );
        let node_at = |scope: LimitScope| {
            if scope == LimitScope::Book(BookId(1)) {
                book_node.clone()
            } else {
                empty_node()
            }
        };
        let accept = pre_trade_check(&accept_tree, &path, &incremental, node_at, |_| {
            NonAdditiveExposure::default()
        });
        assert_eq!(accept.decision, PreTradeDecision::Accept);
        assert!(accept.allowed());

        // (b) A *soft* cap the trade exceeds → warn, but still allowed.
        let mut soft_tree = LimitTree::new();
        soft_tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::soft(LimitMetric::Delta, current_delta), // incremental pushes over.
        );
        let warn = pre_trade_check(&soft_tree, &path, &incremental, node_at, |_| {
            NonAdditiveExposure::default()
        });
        assert_eq!(warn.decision, PreTradeDecision::Warn);
        assert!(warn.allowed(), "a soft breach warns but never blocks");
    }

    // ---- post-trade + escalation --------------------------------------------

    /// **Post-trade monitoring detects a breach and drives escalation (RAG).** A
    /// booked node over a hard cap reports `HardBreach` and a red/breach RAG; a node
    /// within band is `Clear`/green.
    #[test]
    fn post_trade_breach_detection_and_escalation() {
        let p = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let cube = one_book_cube(&[(1, p)]);
        let node = cube.group_by(DimensionId::Book, &DaysPillar)[0].clone();
        let exposure = node.net_greeks.delta_base.abs();
        let na = NonAdditiveExposure::default();

        // Hard limit below current exposure → breach + hard escalation.
        let mut tree = LimitTree::new();
        tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::hard(LimitMetric::Delta, exposure * 0.5),
        );
        let mon = post_trade_check(&tree, LimitScope::Book(BookId(1)), &node, &na);
        assert_eq!(mon.worst, RagStatus::Breach);
        assert_eq!(mon.escalation, EscalationStatus::HardBreach);
        assert_eq!(mon.checks.len(), 1);

        // Same exposure under a soft limit → soft breach escalation, still surfaced.
        let mut soft_tree = LimitTree::new();
        soft_tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::soft(LimitMetric::Delta, exposure * 0.5),
        );
        let soft = post_trade_check(&soft_tree, LimitScope::Book(BookId(1)), &node, &na);
        assert_eq!(soft.escalation, EscalationStatus::SoftBreach);

        // A generous cap → clear / green.
        let mut clear_tree = LimitTree::new();
        clear_tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::hard(LimitMetric::Delta, exposure * 10.0),
        );
        let clear = post_trade_check(&clear_tree, LimitScope::Book(BookId(1)), &node, &na);
        assert_eq!(clear.worst, RagStatus::Green);
        assert_eq!(clear.escalation, EscalationStatus::Clear);
    }

    // ---- fixed-income (FI) limit parity -------------------------------------

    /// A hand-specified linear-FI risk fact — its DV01 / PV01 / per-tenor ladder are
    /// chosen here, so the roll-up is KNOWN independently of any pricing engine (the
    /// non-circular oracle for the extraction test).
    fn fi_fact(
        entity: u32,
        book: u32,
        pv01: f64,
        dv01: f64,
        ladder: &[(u32, f64)],
    ) -> celnet_risk_fleet::RatesRiskFact {
        celnet_risk_fleet::RatesRiskFact {
            key: celnet_risk_fleet::RatesFactKey {
                entity: EntityId(entity),
                ccy: Ccy::USD,
                book: BookId(book),
            },
            pv: 0.0,
            pv01,
            dv01,
            key_rate_ladder: ladder
                .iter()
                .map(|&(tenor_years, dv01)| celnet_risk_fleet::KeyRateBucket { tenor_years, dv01 })
                .collect(),
        }
    }

    /// Roll hand-specified facts up into the single-currency rates aggregate.
    fn fi_agg(
        facts: Vec<celnet_risk_fleet::RatesRiskFact>,
    ) -> celnet_risk_fleet::RatesNodeAggregate {
        celnet_risk_fleet::firm_aggregate_rates(&facts)
            .books()
            .first()
            .expect("one USD currency book")
            .clone()
    }

    /// **`exposure_of_rates` reads the netted DV01 / PV01 / per-tenor bucket off the
    /// rates aggregate, matching an independent hand roll-up of the constituent facts.**
    #[test]
    fn fi_exposure_matches_independent_hand_rollup() {
        let a = fi_fact(1, 10, -1200.0, -1000.0, &[(2, -300.0), (5, -700.0)]);
        let b = fi_fact(2, 20, -800.0, -600.0, &[(5, -250.0), (10, -350.0)]);
        // Independent hand roll-up (the fixed ascending-entity, ascending-tenor fold).
        let want_dv01 = -1000.0 + -600.0;
        let want_pv01 = -1200.0 + -800.0;
        let want_2y = -300.0;
        let want_5y = -700.0 + -250.0;
        let want_10y = -350.0;
        let agg = fi_agg(vec![a, b]);

        assert_eq!(exposure_of_rates(&agg, LimitMetric::Dv01), want_dv01);
        assert_eq!(exposure_of_rates(&agg, LimitMetric::Pvbp), want_pv01);
        assert_eq!(
            exposure_of_rates(&agg, LimitMetric::RateTenorBucket { tenor_years: 2 }),
            want_2y
        );
        assert_eq!(
            exposure_of_rates(&agg, LimitMetric::RateTenorBucket { tenor_years: 5 }),
            want_5y
        );
        assert_eq!(
            exposure_of_rates(&agg, LimitMetric::RateTenorBucket { tenor_years: 10 }),
            want_10y
        );
        // A tenor with no bucket reads 0 — no exposure, never a spurious breach.
        assert_eq!(
            exposure_of_rates(&agg, LimitMetric::RateTenorBucket { tenor_years: 30 }),
            0.0
        );
    }

    /// **A hard DV01 limit hard-breaches an over-limit FI book and clears an under one**,
    /// the breach threshold set from the KNOWN (hand-specified) net DV01 — not from
    /// re-running any engine.
    #[test]
    fn fi_dv01_limit_breach_and_pass() {
        // net_dv01 = -5000 (known); the magnitude the caps straddle is |5000|.
        let agg = fi_agg(vec![fi_fact(1, 10, -6000.0, -5000.0, &[(5, -5000.0)])]);

        let mut breach = LimitTree::new();
        breach.set(LimitScope::Firm, LimitSpec::hard(LimitMetric::Dv01, 4000.0));
        let mon = post_trade_check_rates(&breach, LimitScope::Firm, &agg);
        assert_eq!(mon.escalation, EscalationStatus::HardBreach);
        assert_eq!(mon.worst, RagStatus::Breach);

        let mut ok = LimitTree::new();
        ok.set(
            LimitScope::Firm,
            LimitSpec::hard(LimitMetric::Dv01, 10_000.0),
        );
        let clear = post_trade_check_rates(&ok, LimitScope::Firm, &agg);
        assert_eq!(clear.escalation, EscalationStatus::Clear);
        assert_eq!(clear.worst, RagStatus::Green); // 5000 / 10000 = 0.5 → green.
    }

    /// **A per-tenor bucket limit catches a single-tenor curve concentration** even when
    /// the parallel DV01 nets small — and a bucket cap only ever charges its own tenor.
    #[test]
    fn fi_tenor_bucket_limit_breach() {
        // Net DV01 = -900 (small), but the 5y bucket is -4900 (concentrated).
        let agg = fi_agg(vec![fi_fact(
            1,
            10,
            -1000.0,
            -900.0,
            &[(2, 4000.0), (5, -4900.0)],
        )]);

        let mut tree = LimitTree::new();
        tree.set(
            LimitScope::Firm,
            LimitSpec::hard(LimitMetric::RateTenorBucket { tenor_years: 5 }, 3000.0),
        );
        let mon = post_trade_check_rates(&tree, LimitScope::Firm, &agg);
        assert_eq!(mon.escalation, EscalationStatus::HardBreach);
        // A 5y-only cap evaluates exactly one check — the uncapped 2y bucket is untouched.
        assert_eq!(mon.checks.len(), 1);
        assert_eq!(
            mon.checks[0].limit.metric,
            LimitMetric::RateTenorBucket { tenor_years: 5 }
        );
    }

    // ---- the mixed-family pre-trade gate (G2: FI limits reachable at booking) ----

    /// **THE REGRESSION TEST for the inert-DV01 defect.**
    ///
    /// A hard `Dv01` limit that is over its cap on the projected book:
    ///
    /// - under the FX-Greeks-only [`pre_trade_check`] it **ACCEPTS** — `exposure_of`
    ///   returns a hard-coded `0.0` for every FI metric on a Greeks node, so the limit is
    ///   structurally incapable of breaching (the defect,
    ///   `docs/RISK-MODEL-REQUIREMENTS-AND-GAPS.md` §2.3 Defect 2 / G2);
    /// - under [`pre_trade_check_mixed`] it **REJECTS**, charged against the real
    ///   `RatesNodeAggregate` DV01.
    #[test]
    fn dv01_limit_breaches_under_the_mixed_gate_and_was_inert_before() {
        // A book whose projected net DV01 is a KNOWN -5000; the hard cap is 4000.
        let agg = fi_agg(vec![fi_fact(1, 10, -6000.0, -5000.0, &[(5, -5000.0)])]);
        let mut tree = LimitTree::new();
        tree.set(LimitScope::Firm, LimitSpec::hard(LimitMetric::Dv01, 4000.0));
        let path = ScopePath::from_scopes(vec![LimitScope::Firm]);
        let increment = IncrementalTrade {
            greeks: NetGreeks::zero(),
            vega_pillar: VegaPillar::new(0, 0),
            vega: 0.0,
        };

        // BEFORE: the Greeks-only gate cannot see the DV01 at all.
        let before = pre_trade_check(
            &tree,
            &path,
            &increment,
            |_| empty_node(),
            |_| NonAdditiveExposure::default(),
        );
        assert_eq!(
            before.decision,
            PreTradeDecision::Accept,
            "the FX-Greeks-only gate charges a DV01 limit 0.0 — it can never breach"
        );
        assert_eq!(before.checks[0].utilization.ratio, 0.0);

        // AFTER: the mixed gate charges the real aggregate DV01 and blocks.
        let after = pre_trade_check_mixed(
            &tree,
            &path,
            &increment,
            |_| empty_node(),
            |_| NonAdditiveExposure::default(),
            |_| agg.clone(),
        );
        assert_eq!(after.decision, PreTradeDecision::Reject);
        let breach = after.hard_breaches().next().expect("a hard DV01 breach");
        assert_eq!(breach.limit.metric, LimitMetric::Dv01);
        assert!((breach.utilization.ratio - 5000.0 / 4000.0).abs() < 1e-12);
    }

    /// **A `Pvbp` limit is equally reachable** through the mixed gate, read off the
    /// aggregate's netted analytic PV01 rather than the parallel DV01.
    #[test]
    fn pvbp_limit_breaches_under_the_mixed_gate() {
        let agg = fi_agg(vec![fi_fact(1, 10, -6000.0, -1.0, &[(5, -1.0)])]);
        let mut tree = LimitTree::new();
        tree.set(LimitScope::Firm, LimitSpec::hard(LimitMetric::Pvbp, 5000.0));
        let path = ScopePath::from_scopes(vec![LimitScope::Firm]);
        let increment = IncrementalTrade {
            greeks: NetGreeks::zero(),
            vega_pillar: VegaPillar::new(0, 0),
            vega: 0.0,
        };
        let res = pre_trade_check_mixed(
            &tree,
            &path,
            &increment,
            |_| empty_node(),
            |_| NonAdditiveExposure::default(),
            |_| agg.clone(),
        );
        assert_eq!(res.decision, PreTradeDecision::Reject);
        assert_eq!(
            res.hard_breaches().next().expect("breach").limit.metric,
            LimitMetric::Pvbp
        );
    }

    /// **Each limit is charged exactly once, by its own family** — a tree carrying both a
    /// Greeks `Delta` proxy limit and a `Dv01` cap evaluates two checks, each against its
    /// own node, with no double-charging and no cross-contamination.
    #[test]
    fn mixed_gate_charges_each_family_against_its_own_node() {
        let agg = fi_agg(vec![fi_fact(1, 10, -100.0, -900.0, &[(5, -900.0)])]);
        let mut tree = LimitTree::new();
        tree.set(
            LimitScope::Firm,
            LimitSpec::hard(LimitMetric::Delta, 10_000.0),
        );
        tree.set(
            LimitScope::Firm,
            LimitSpec::hard(LimitMetric::Dv01, 10_000.0),
        );
        let path = ScopePath::from_scopes(vec![LimitScope::Firm]);
        // The proposed trade adds +2000 of linear delta on the Greeks side.
        let mut greeks = NetGreeks::zero();
        greeks.delta_base = 2000.0;
        let increment = IncrementalTrade {
            greeks,
            vega_pillar: VegaPillar::new(0, 0),
            vega: 0.0,
        };
        let res = pre_trade_check_mixed(
            &tree,
            &path,
            &increment,
            |_| {
                let mut n = empty_node();
                n.net_greeks.delta_base = 3000.0;
                n
            },
            |_| NonAdditiveExposure::default(),
            |_| agg.clone(),
        );
        assert_eq!(res.decision, PreTradeDecision::Accept);
        assert_eq!(
            res.checks.len(),
            2,
            "two limits, two checks — charged once each"
        );
        let delta = res
            .checks
            .iter()
            .find(|c| c.limit.metric == LimitMetric::Delta)
            .expect("delta check");
        let dv01 = res
            .checks
            .iter()
            .find(|c| c.limit.metric == LimitMetric::Dv01)
            .expect("dv01 check");
        // Delta: (3000 current + 2000 incremental) / 10000; DV01: |−900| / 10000.
        assert!((delta.utilization.ratio - 0.5).abs() < 1e-12);
        assert!((dv01.utilization.ratio - 0.09).abs() < 1e-12);
    }

    /// **The rates aggregate is not built unless an FI limit actually needs it** — a tree
    /// carrying only Greeks limits never pays for the FI path.
    #[test]
    fn mixed_gate_skips_the_rates_aggregate_when_no_fi_limit_is_configured() {
        let mut tree = LimitTree::new();
        tree.set(
            LimitScope::Firm,
            LimitSpec::hard(LimitMetric::Delta, 10_000.0),
        );
        let path = ScopePath::from_scopes(vec![LimitScope::Firm]);
        let increment = IncrementalTrade {
            greeks: NetGreeks::zero(),
            vega_pillar: VegaPillar::new(0, 0),
            vega: 0.0,
        };
        let mut built = 0_u32;
        let res = pre_trade_check_mixed(
            &tree,
            &path,
            &increment,
            |_| empty_node(),
            |_| NonAdditiveExposure::default(),
            |_| {
                built += 1;
                fi_agg(vec![fi_fact(1, 10, 0.0, 0.0, &[])])
            },
        );
        assert_eq!(res.decision, PreTradeDecision::Accept);
        assert_eq!(built, 0, "no FI limit ⇒ no rates aggregate built");
    }

    /// **The two metric families are inert on each other's node**, so one limit tree may
    /// carry both without cross-charging, and the rates check evaluates only FI limits.
    #[test]
    fn fi_and_fx_metrics_are_inert_across_families() {
        let agg = fi_agg(vec![fi_fact(1, 10, -100.0, -80.0, &[(5, -80.0)])]);
        let na = NonAdditiveExposure::default();

        // An FX-Greeks metric has no exposure on a rates node.
        assert_eq!(exposure_of_rates(&agg, LimitMetric::Delta), 0.0);
        assert_eq!(exposure_of_rates(&agg, LimitMetric::Vega), 0.0);
        // An FI metric has no exposure on an FX-Greeks node (exposure_of stays inert).
        assert_eq!(exposure_of(&empty_node(), LimitMetric::Dv01, &na), 0.0);
        assert_eq!(
            exposure_of(
                &empty_node(),
                LimitMetric::RateTenorBucket { tenor_years: 5 },
                &na
            ),
            0.0
        );

        // check_scope_rates evaluates ONLY the FI limits at a scope — a stray FX (delta
        // proxy) limit in the same tree is skipped, so the families never double-charge.
        let mut tree = LimitTree::new();
        tree.set(LimitScope::Firm, LimitSpec::hard(LimitMetric::Delta, 1.0));
        tree.set(LimitScope::Firm, LimitSpec::hard(LimitMetric::Dv01, 1.0e12));
        let checks = check_scope_rates(&tree, LimitScope::Firm, &agg);
        assert_eq!(
            checks.len(),
            1,
            "only the FI limit is evaluated on the rates aggregate"
        );
        assert_eq!(checks[0].limit.metric, LimitMetric::Dv01);

        assert!(LimitMetric::Dv01.is_fixed_income());
        assert!(LimitMetric::Pvbp.is_fixed_income());
        assert!(LimitMetric::RateTenorBucket { tenor_years: 5 }.is_fixed_income());
        assert!(!LimitMetric::Delta.is_fixed_income());
        assert!(!LimitMetric::TenorVega { tenor_days: 365 }.is_fixed_income());
    }
}
