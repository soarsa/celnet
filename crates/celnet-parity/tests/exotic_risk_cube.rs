//! Parity row (COMPLETION-PROGRAM W11 track A): **exotic legs roll up into the risk
//! cube and survive the distributed fan-out**.
//!
//! Before this wave the hierarchical risk estate aggregated **vanilla** legs only —
//! a booked exotic (barrier, digital) contributed *zero* to firm/desk/book risk and
//! was silently excluded. This row gates the fix against **independent** oracles:
//!
//! - **Gate A — fan-out == single-node, INCLUDING exotic legs.** A firm book that
//!   mixes vanilla and exotic legs, partitioned across an HRW fleet and reduced via
//!   the cross-shard algebra, equals the single-node `Cube` firm aggregate: additive
//!   Greeks to `~1e-12`, and the re-gathered non-additive VaR/ES / curvature to
//!   `~1e-9` (the exotic legs are NOT dropped on either path).
//! - **Gate B — the exotic's Greek contribution == an INDEPENDENT re-derivation.**
//!   The cube's net-Greek delta of (book-with-exotic − book-without-exotic) equals a
//!   from-scratch finite difference of the closed-form `single_barrier_price` coded
//!   here in the test (the cube is never asked to check itself), and the digital
//!   leg's leaf delta/gamma/vega equal the closed-form digital Greeks.
//! - **Gate C — non-exclusion.** A portfolio WITH an exotic leg has a STRICTLY
//!   different firm roll-up (net Greeks, premium, VaR) than the SAME portfolio
//!   without it — a silently-excluded exotic would make the two identical.
//!
//! Every exotic here is a **deterministic closed form** (Reiner-Rubinstein barrier,
//! European digital), so the comparison is machine-exact with no Monte-Carlo caveat.

use celnet_core::carry::CarryInputs;
use celnet_core::is_close;
use celnet_exotics::{
    BarrierKind, BarrierStyle, DigitalKind, SingleBarrier, digital_greeks, single_barrier_price,
};
use celnet_risk_cube::{
    BookId, Cube, DeskId, EntityId, ExoticKind, ExoticLeg, FactKey, FactMeasure, LocationId,
    PositionId, RiskFact, Scenario, TraderId, VegaPillar, VegaPillarMap,
};
use celnet_risk_fleet::{fan_out_aggregate, partition_facts};
use celnet_risk_normalize::{AssetPricer, CanonicalLeaf, PositionRisk, canonicalize};
use celnet_router::{Replica, ReplicaId, ReplicaSet};
use celnet_types::{
    Carry, Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
};

fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}

fn gbpusd() -> CcyPair {
    CcyPair::new(Ccy::GBP, Ccy::USD)
}

fn usdjpy() -> CcyPair {
    CcyPair::new(Ccy::USD, Ccy::JPY)
}

/// A pillar map bucketing by tenor-in-days at a fixed 0.50Δ pillar (the same shape
/// the cube/fleet test suites use), so the vega ladder is deterministic.
struct DaysPillar;
impl VegaPillarMap for DaysPillar {
    fn pillar_of(&self, _leaf: &CanonicalLeaf, position: &PositionRisk) -> VegaPillar {
        let days = (position.inputs.t * 365.0).round() as u32;
        VegaPillar::new(days, 5000)
    }
}

fn vanilla(pair: CcyPair, opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
    PositionRisk::fx(
        pair,
        opt,
        notional,
        inputs,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    )
}

/// The org placement for a fact: `org` sets trader/book/location/entity to the same
/// handle (these tests only need the `(entity, pair)` partition key to vary, so one
/// handle per org-cell keeps the fact builders to a couple of arguments — no
/// clippy-`too_many_arguments` dodge).
fn org_key(org: u32, pair: CcyPair) -> FactKey {
    FactKey {
        trader: TraderId(org),
        book: BookId(org),
        desk: DeskId(0),
        underlying: Underlying::Fx(pair),
        location: LocationId(org),
        entity: EntityId(org),
    }
}

fn vanilla_fact(id: u32, org: u32, position: PositionRisk) -> RiskFact {
    RiskFact {
        position_id: PositionId(id),
        key: org_key(
            org,
            position
                .underlying
                .as_ccy_pair()
                .expect("vanilla parity facts are FX"),
        ),
        measure: FactMeasure {
            leaf: canonicalize(&position).unwrap(),
            position,
            exotic: None,
        },
        surface_version: 1,
    }
}

fn exotic_fact(id: u32, org: u32, option: OptionType, leg: ExoticLeg) -> RiskFact {
    // The underlying-vanilla bucketing metadata (never priced; the exotic pricer is).
    let position = PositionRisk::fx(
        leg.pair,
        option,
        leg.notional,
        leg.inputs,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    );
    RiskFact {
        position_id: PositionId(id),
        key: org_key(org, leg.pair),
        measure: FactMeasure {
            leaf: leg.canonical_leaf(),
            position,
            exotic: Some(leg),
        },
        surface_version: 1,
    }
}

fn up_and_out_call() -> SingleBarrier {
    SingleBarrier {
        kind: BarrierKind {
            up: true,
            style: BarrierStyle::KnockOut,
            option: OptionType::Call,
        },
        strike: 1.10,
        barrier: 1.28,
        rebate: 0.0,
    }
}

fn down_and_in_put() -> SingleBarrier {
    SingleBarrier {
        kind: BarrierKind {
            up: false,
            style: BarrierStyle::KnockIn,
            option: OptionType::Put,
        },
        strike: 1.30,
        barrier: 1.20,
        rebate: 0.0,
    }
}

/// A firm book mixing **vanilla** legs and **exotic** legs across two entities and
/// two pairs (so the HRW partition genuinely fans them across shards and a naive
/// sum-of-shard-VaRs would be wrong).
fn mixed_firm_book() -> Vec<RiskFact> {
    let barrier_inputs = VanillaInputs::new(1.10, 1.10, 0.105, 1.0, 0.04, 0.02);
    let digital_inputs = VanillaInputs::new(1.30, 1.32, 0.12, 0.75, 0.03, 0.01);
    vec![
        // Org-cell 1, EURUSD: a vanilla call + a vanilla put + an up-and-out barrier.
        vanilla_fact(
            1,
            1,
            vanilla(
                eurusd(),
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
        ),
        vanilla_fact(
            2,
            1,
            vanilla(
                eurusd(),
                OptionType::Put,
                -5_000_000.0,
                VanillaInputs::new(1.10, 1.06, 0.115, 0.5, 0.04, 0.02),
            ),
        ),
        exotic_fact(
            3,
            1,
            OptionType::Call,
            ExoticLeg::new(
                eurusd(),
                ExoticKind::SingleBarrier(up_and_out_call()),
                8_000_000.0,
                barrier_inputs,
            ),
        ),
        // Org-cell 2, GBPUSD: a vanilla call + a down-and-in barrier put + a digital.
        vanilla_fact(
            4,
            2,
            vanilla(
                gbpusd(),
                OptionType::Call,
                6_000_000.0,
                VanillaInputs::new(1.30, 1.31, 0.09, 1.0, 0.045, 0.005),
            ),
        ),
        exotic_fact(
            5,
            2,
            OptionType::Put,
            ExoticLeg::new(
                gbpusd(),
                ExoticKind::SingleBarrier(down_and_in_put()),
                -4_000_000.0,
                VanillaInputs::new(1.30, 1.30, 0.11, 1.0, 0.045, 0.005),
            ),
        ),
        exotic_fact(
            6,
            2,
            OptionType::Call,
            ExoticLeg::new(
                gbpusd(),
                ExoticKind::Digital(DigitalKind::cash(OptionType::Call)),
                5_000_000.0,
                digital_inputs,
            ),
        ),
        // Org-cell 3, USDJPY: a vanilla put (a third (entity, pair) cell so the HRW
        // partition genuinely fans across multiple shards, never co-locating all).
        vanilla_fact(
            7,
            3,
            vanilla(
                usdjpy(),
                OptionType::Put,
                -7_000_000.0,
                VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
            ),
        ),
    ]
}

fn single_node(facts: &[RiskFact]) -> Cube {
    let mut cube = Cube::new();
    for f in facts {
        cube.upsert(f.clone());
    }
    cube
}

fn replicas(ids: &[u64]) -> ReplicaSet {
    ReplicaSet::new(ids.iter().map(|&i| Replica::up(ReplicaId(i))).collect()).unwrap()
}

/// A symmetric spot×vol scenario ladder (the §2.5 VaR re-derivation grid).
fn ladder() -> Vec<Scenario> {
    let mut v = Vec::new();
    for si in -8..=8 {
        for vj in -3..=3 {
            v.push(Scenario {
                spot_rel: f64::from(si) * 0.01,
                vol_abs: f64::from(vj) * 0.005,
                discount_abs: 0.0,
                carry_abs: 0.0,
            });
        }
    }
    v
}

/// **Gate A:** the cross-shard fan-out equals the single-node firm aggregate over a
/// book that INCLUDES exotic legs — additive Greeks to `~1e-12`, and non-additive
/// VaR/ES and curvature to `~1e-9`. If the exotic legs were dropped on either path
/// the two would not reconcile.
#[test]
fn fan_out_equals_single_node_including_exotics() {
    let facts = mixed_firm_book();
    let scen = ladder();
    let alpha = 0.99;
    let rw = 0.20;

    // Single-node reference.
    let cube = single_node(&facts);
    let firm = cube.firm_aggregate(&DaysPillar);
    let s_var = Cube::node_var_es(&AssetPricer, &firm, &scen, alpha);
    let s_cvr = Cube::node_curvature_spot(&AssetPricer, &firm, rw);

    // The book genuinely contains exotic legs (else the test would be vacuous).
    assert_eq!(
        firm.exotic_legs.len(),
        3,
        "three exotic legs must be present"
    );
    assert_eq!(firm.positions.len(), 4, "four vanilla legs must be present");

    // Distributed fan-out across a 3-replica HRW fleet (this id set provably spreads
    // the three (entity, pair) cells across multiple shards — see the assertion).
    let reps = replicas(&[11, 22, 33]);
    let agg = fan_out_aggregate(&facts, &reps, &DaysPillar, &scen, alpha, rw).unwrap();
    assert!(
        agg.shard_count >= 2,
        "facts must fan across multiple shards"
    );

    // Additive Greeks reconcile to ~1e-12 (associative merge over the same leaves).
    assert!(is_close(
        agg.firm.net_greeks.delta_base,
        firm.net_greeks.delta_base,
        1e-12,
        1e-6
    ));
    assert!(is_close(
        agg.firm.net_greeks.gamma,
        firm.net_greeks.gamma,
        1e-12,
        1e-6
    ));
    assert!(is_close(
        agg.firm.net_greeks.vega,
        firm.net_greeks.vega,
        1e-12,
        1e-6
    ));
    assert!(is_close(
        agg.firm.net_greeks.premium_quote,
        firm.net_greeks.premium_quote,
        1e-12,
        1e-6
    ));
    assert!(is_close(
        agg.firm.vega_ladder.total(),
        firm.vega_ladder.total(),
        1e-12,
        1e-6
    ));

    // The gathered union carries the exotic legs (not silently dropped on the wire
    // re-gather path).
    assert_eq!(agg.firm.exotic_legs.len(), firm.exotic_legs.len());
    assert_eq!(agg.firm.positions.len(), firm.positions.len());

    // Non-additive measures reconcile to ~1e-9 (same constituent multiset → same
    // oracle, residual only FP summation order).
    assert!(s_var.var > 0.0, "the firm must carry real VaR");
    assert!(
        is_close(agg.var_es.var, s_var.var, 1e-9, 1e-3),
        "fan-out VaR {} vs single-node {}",
        agg.var_es.var,
        s_var.var
    );
    assert!(is_close(agg.var_es.es, s_var.es, 1e-9, 1e-3));
    assert!(
        is_close(agg.curvature_spot, s_cvr, 1e-9, 1e-3),
        "fan-out curvature {} vs single-node {}",
        agg.curvature_spot,
        s_cvr
    );
}

/// **Gate B (barrier):** the exotic leg's contribution to the firm net Greeks equals
/// an INDEPENDENT from-scratch finite difference of the closed-form barrier price
/// (coded here, never calling the cube's own Greek path). We isolate the
/// contribution as (firm-with-exotic − firm-without-exotic) and match it to the
/// in-test FD × notional.
#[test]
fn barrier_contribution_matches_independent_fd() {
    let spec = up_and_out_call();
    let inputs = VanillaInputs::new(1.10, 1.10, 0.105, 1.0, 0.04, 0.02);
    let n = 8_000_000.0;

    // A baseline vanilla-only book.
    let base_facts = vec![vanilla_fact(
        1,
        1,
        vanilla(
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        ),
    )];
    // The same book PLUS the barrier exotic.
    let mut with_exotic = base_facts.clone();
    with_exotic.push(exotic_fact(
        2,
        1,
        OptionType::Call,
        ExoticLeg::new(eurusd(), ExoticKind::SingleBarrier(spec), n, inputs),
    ));

    let base = single_node(&base_facts).firm_aggregate(&DaysPillar);
    let plus = single_node(&with_exotic).firm_aggregate(&DaysPillar);

    let contrib_delta = plus.net_greeks.delta_base - base.net_greeks.delta_base;
    let contrib_gamma = plus.net_greeks.gamma - base.net_greeks.gamma;
    let contrib_vega = plus.net_greeks.vega - base.net_greeks.vega;
    let contrib_premium = plus.net_greeks.premium_quote - base.net_greeks.premium_quote;

    // Independent central FD of the closed-form barrier price (no cube code).
    let pr = |s: f64, vol: f64| {
        single_barrier_price(
            &(&VanillaInputs::new(s, inputs.strike, vol, inputs.t, inputs.r_dom, inputs.r_for))
                .into(),
            spec,
        )
    };
    let s = inputs.spot;
    let v = inputs.vol;
    let ds = (s * 1e-4).max(1e-7);
    let dv = 1e-4;
    let p0 = pr(s, v);
    let ref_delta = (pr(s + ds, v) - pr(s - ds, v)) / (2.0 * ds) * n;
    let ref_gamma = (pr(s + ds, v) - 2.0 * p0 + pr(s - ds, v)) / (ds * ds) * n;
    let ref_vega = (pr(s, v + dv) - pr(s, v - dv)) / (2.0 * dv) * n;
    let ref_premium = p0 * n;

    assert!(
        is_close(contrib_delta, ref_delta, 1e-9, 1e-2),
        "barrier delta contribution {contrib_delta} vs independent FD {ref_delta}"
    );
    assert!(is_close(contrib_gamma, ref_gamma, 1e-9, 1.0));
    assert!(is_close(contrib_vega, ref_vega, 1e-9, 1e-2));
    assert!(is_close(contrib_premium, ref_premium, 1e-9, 1e-3));
    // Sanity: the contribution is genuinely nonzero (the exotic is not absorbed).
    assert!(contrib_premium.abs() > 1.0);
}

/// **Gate B (digital):** the digital leg's leaf delta/gamma/vega contribution equals
/// the closed-form `celnet-exotics` digital Greeks × notional (the published exact
/// values, an oracle disjoint from the cube's own accumulation).
#[test]
fn digital_contribution_matches_closed_form_greeks() {
    let kind = DigitalKind::cash(OptionType::Call);
    let inputs = VanillaInputs::new(1.30, 1.32, 0.12, 0.75, 0.03, 0.01);
    let n = 5_000_000.0;

    let with_exotic = vec![exotic_fact(
        1,
        1,
        OptionType::Call,
        ExoticLeg::new(gbpusd(), ExoticKind::Digital(kind), n, inputs),
    )];
    let firm = single_node(&with_exotic).firm_aggregate(&DaysPillar);

    // Independent oracle: the closed-form digital Greeks (no cube code).
    let dg = digital_greeks(kind, &(&inputs).into());
    assert!(is_close(
        firm.net_greeks.delta_base,
        dg.delta * n,
        1e-12,
        1e-3
    ));
    assert!(is_close(firm.net_greeks.gamma, dg.gamma * n, 1e-12, 1e-3));
    assert!(is_close(firm.net_greeks.vega, dg.vega * n, 1e-12, 1e-3));
}

/// **Gate C — non-exclusion:** a portfolio WITH an exotic leg has a STRICTLY
/// different firm roll-up (net Greeks, premium, AND VaR) than the SAME portfolio
/// without it. A silently-excluded exotic would make the two byte-identical — this
/// is the regression the whole wave exists to prevent.
#[test]
fn exotic_leg_strictly_changes_the_rollup() {
    let scen = ladder();
    let alpha = 0.99;

    let without: Vec<RiskFact> = mixed_firm_book()
        .into_iter()
        .filter(|f| f.measure.exotic.is_none())
        .collect();
    let with_all = mixed_firm_book();

    assert_eq!(
        without.len(),
        4,
        "the vanilla-only book has the 4 vanilla legs"
    );
    assert_eq!(with_all.len(), 7, "the full book adds the 3 exotic legs");

    let firm_without = single_node(&without).firm_aggregate(&DaysPillar);
    let firm_with = single_node(&with_all).firm_aggregate(&DaysPillar);

    // The exotic legs move the additive roll-up (premium + at least delta/vega).
    assert!(
        (firm_with.net_greeks.premium_quote - firm_without.net_greeks.premium_quote).abs() > 1.0,
        "premium must change when exotics are included"
    );
    assert!(
        (firm_with.net_greeks.delta_base - firm_without.net_greeks.delta_base).abs() > 1.0,
        "net delta must change when exotics are included"
    );
    assert!(
        (firm_with.net_greeks.vega - firm_without.net_greeks.vega).abs() > 1.0,
        "net vega must change when exotics are included"
    );

    // And the non-additive tail changes too (the exotic legs carry real VaR).
    let var_without = Cube::node_var_es(&AssetPricer, &firm_without, &scen, alpha).var;
    let var_with = Cube::node_var_es(&AssetPricer, &firm_with, &scen, alpha).var;
    assert!(var_without > 0.0 && var_with > 0.0);
    assert!(
        (var_with - var_without).abs() > 1e-6 * var_without,
        "firm VaR must change when exotic legs are included: {var_without} vs {var_with}"
    );
}

/// **The exotic non-additive path is genuinely the exotic payoff, not a vanilla
/// proxy.** A long up-and-out call's firm VaR is driven by the knock-out (value
/// destroyed on an up-move toward the barrier), so a book of ONLY that exotic leg
/// has a tail dominated by up-scenarios — distinct from the equivalent long vanilla
/// call, whose tail is the down-scenarios. This proves the cube re-prices the real
/// exotic under shocks (not a vanilla stand-in) through its non-additive reducer.
#[test]
fn knock_out_var_is_exotic_not_vanilla() {
    let spec = up_and_out_call();
    let inputs = VanillaInputs::new(1.10, 1.10, 0.105, 1.0, 0.04, 0.02);
    let n = 8_000_000.0;
    // A WIDE spot ladder reaching the 1.28 up-and-out barrier (spot 1.10 → +20% =
    // 1.32 > 1.28), so the knock-out value-collapse is in range and genuinely
    // dominates the long leg's loss tail — the whole point of this exotic-vs-vanilla
    // contrast (the default ±8% ladder never reaches the barrier).
    let scen: Vec<Scenario> = (-20..=20)
        .map(|i| Scenario::spot(f64::from(i) * 0.01))
        .collect();

    // Exotic-only firm.
    let exotic_only = vec![exotic_fact(
        1,
        1,
        OptionType::Call,
        ExoticLeg::new(eurusd(), ExoticKind::SingleBarrier(spec), n, inputs),
    )];
    let firm_exotic = single_node(&exotic_only).firm_aggregate(&DaysPillar);
    let var_exotic = Cube::node_var_es(&AssetPricer, &firm_exotic, &scen, 0.99).var;

    // Vanilla-only firm of the underlying call (same strike/inputs/notional).
    let vanilla_only = vec![vanilla_fact(
        1,
        1,
        vanilla(eurusd(), OptionType::Call, n, inputs),
    )];
    let firm_vanilla = single_node(&vanilla_only).firm_aggregate(&DaysPillar);
    let var_vanilla = Cube::node_var_es(&AssetPricer, &firm_vanilla, &scen, 0.99).var;

    assert!(var_exotic > 0.0 && var_vanilla > 0.0);
    // The two VaRs are materially different — the exotic payoff is not the vanilla's.
    assert!(
        (var_exotic - var_vanilla).abs() > 1e-3 * var_vanilla.max(var_exotic),
        "exotic VaR {var_exotic} must differ from vanilla VaR {var_vanilla}"
    );
    // Independent check: the worst single-scenario loss of the long up-and-out call
    // is an UP scenario (toward the knock-out), the opposite of a long vanilla call.
    // Lift the FX VanillaInputs to the agnostic carry basis the scenario shocks act
    // on (the FX two-rate carry — byte-identical to the prior VanillaInputs shock).
    let carry_inputs = CarryInputs::new(
        inputs.spot,
        inputs.strike,
        inputs.vol,
        inputs.t,
        Underlying::Fx(eurusd()),
        Carry::FxRates {
            r_dom: inputs.r_dom,
            r_for: inputs.r_for,
        },
    );
    let exotic_pnls: Vec<(f64, f64)> = scen
        .iter()
        .map(|s| {
            let shocked = s.apply(&carry_inputs);
            let pnl = (single_barrier_price(&shocked.into(), spec)
                - single_barrier_price(&(&inputs).into(), spec))
                * n;
            (s.spot_rel, pnl)
        })
        .collect();
    let worst = exotic_pnls
        .iter()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .unwrap();
    assert!(
        worst.0 > 0.0,
        "the long up-and-out call's worst loss should be an UP move, got spot_rel {}",
        worst.0
    );
}

/// **Exotic-only fan-out reconciles too** (no vanilla leg to mask a dropped exotic):
/// a fleet of purely exotic legs reconciles to the single-node firm aggregate,
/// additive `~1e-12` and non-additive `~1e-9`.
#[test]
fn exotic_only_fan_out_reconciles() {
    let facts: Vec<RiskFact> = mixed_firm_book()
        .into_iter()
        .filter(|f| f.measure.exotic.is_some())
        .collect();
    assert_eq!(facts.len(), 3);
    let scen = ladder();
    let alpha = 0.975;
    let rw = 0.18;

    let firm = single_node(&facts).firm_aggregate(&DaysPillar);
    assert!(firm.positions.is_empty(), "no vanilla legs in this book");
    assert_eq!(firm.exotic_legs.len(), 3);
    let s_var = Cube::node_var_es(&AssetPricer, &firm, &scen, alpha);
    let s_cvr = Cube::node_curvature_spot(&AssetPricer, &firm, rw);

    let reps = replicas(&[7, 14, 21]);
    // Sanity that partitioning routes all three.
    let reducer = partition_facts(&facts, &reps).unwrap();
    assert!(reducer.shard_count() >= 2);

    let agg = fan_out_aggregate(&facts, &reps, &DaysPillar, &scen, alpha, rw).unwrap();
    assert!(is_close(
        agg.firm.net_greeks.premium_quote,
        firm.net_greeks.premium_quote,
        1e-12,
        1e-6
    ));
    assert_eq!(agg.firm.exotic_legs.len(), 3);
    assert!(is_close(agg.var_es.var, s_var.var, 1e-9, 1e-3));
    assert!(is_close(agg.curvature_spot, s_cvr, 1e-9, 1e-3));
}
