//! The **longhand recomputation oracle** (W5-A §4): a maximally explicit,
//! **code-disjoint** recomputation of a small mixed-asset firm aggregate + its FRTB
//! charge, written so it shares **no production helper** with the cube / normalize /
//! frtb code. This is the *independent* oracle the cross-asset seam is gated against.
//!
//! The central W5-A correctness claim it proves: the seam dispatch returns exactly the
//! asset's **own** leaf Greeks (FX via `celnet-vanilla`, metal via the FX two-rate
//! path, equity via `celnet-equity-vanilla`) — never a silent FX proxy for a non-FX
//! position. The production path routes through `AssetPricer`/`canonicalize`; this
//! oracle calls the leaf crates' public `price`/`greeks` **directly** and hand-sums
//! with a plain loop. They agree because the *math* is the same, reached two
//! code-disjoint ways.

use celnet_core::carry::{CarryGreeks, CarryInputs, CarryPriceError, CarryPricer};
use celnet_core::is_close;
use celnet_risk_cube::{
    BookId, Cube, DeskId, DimensionId, EntityId, ExoticKind, ExoticLeg, FactKey, FactMeasure,
    LocationId, PositionId, PositionSensitivity, RiskFact, Scenario, StandardFrtbParams, TraderId,
    VegaPillar, VegaPillarMap, historical_var_es, node_sensitivities, node_var_es_combined,
    node_var_es_sensitivity_combined, sbm_curvature_spot, sbm_curvature_spot_combined,
    sensitivity_var_es, standard_delta_buckets, vanilla_curvature_legs,
};
use celnet_risk_normalize::{AssetPricer, CanonicalLeaf, PositionRisk, canonicalize};
use celnet_types::{
    Carry, Ccy, CcyPair, CommodityRef, DeltaConvention, EquityRef, Metal, MetalPair, OptionType,
    PremiumStyle, RateSensitivities, Symbol, Underlying, VanillaInputs,
};

/// A trivial pillar map (single tenor/delta vertex) — the oracle exercises the Greek
/// sums, not the pillar grid.
struct OnePillar;
impl VegaPillarMap for OnePillar {
    fn pillar_of(&self, _leaf: &CanonicalLeaf, _pos: &PositionRisk) -> VegaPillar {
        VegaPillar::new(365, 5000)
    }
}

// ----- the hand-built mixed 3-asset book -----

fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}

/// FX EURUSD call.
fn fx_pos() -> (PositionRisk, OptionType, f64, VanillaInputs) {
    let vi = VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02);
    let n = 10_000_000.0;
    (
        PositionRisk::fx(
            eurusd(),
            OptionType::Call,
            n,
            vi,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ),
        OptionType::Call,
        n,
        vi,
    )
}

/// XAUUSD (metal) put — priced through the FX two-rate path (the metal lease rate is
/// the foreign rate), so its leaf-direct oracle is the FX leaf on the lowered inputs.
fn metal_pos() -> (PositionRisk, OptionType, f64, VanillaInputs) {
    let vi = VanillaInputs::new(2_000.0, 1_950.0, 0.15, 1.0, 0.05, 0.01);
    let n = -3_000.0;
    let u = Underlying::Metal(MetalPair::new(Metal::Gold, Ccy::USD));
    let ci = CarryInputs::new(
        vi.spot,
        vi.strike,
        vi.vol,
        vi.t,
        u.clone(),
        Carry::FxRates {
            r_dom: vi.r_dom,
            r_for: vi.r_for,
        },
    );
    (
        PositionRisk::carry(u, OptionType::Put, n, ci),
        OptionType::Put,
        n,
        vi,
    )
}

/// Equity ACME call (dividend-paying), generalized-BSM via `celnet-equity-vanilla`.
fn equity_pos() -> (
    PositionRisk,
    OptionType,
    f64,
    celnet_equity_vanilla::EquityInputs,
) {
    let (r, b) = (0.03, 0.01); // ⇒ q = r − b = 0.02
    let n = 1_000.0;
    let u = Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD));
    let ci = CarryInputs::new(
        100.0,
        105.0,
        0.20,
        1.0,
        u.clone(),
        Carry::CostOfCarry { r, b },
    );
    // The leaf-direct input (the equity lowering: q = r − b, repo = 0).
    let ei = celnet_equity_vanilla::EquityInputs::new(100.0, 105.0, 0.20, 1.0, r, r - b, 0.0);
    (
        PositionRisk::carry(u, OptionType::Call, n, ci),
        OptionType::Call,
        n,
        ei,
    )
}

fn fact(id: u32, position: &PositionRisk) -> RiskFact {
    RiskFact {
        position_id: PositionId(id),
        key: FactKey {
            trader: TraderId(id),
            book: BookId(id),
            desk: DeskId(1),
            underlying: position.underlying.clone(),
            location: LocationId(1),
            entity: EntityId(1),
        },
        measure: FactMeasure {
            leaf: canonicalize(position).unwrap(),
            position: position.clone(),
            exotic: None,
        },
        surface_version: 1,
    }
}

/// G8: the cube's `firm_aggregate(&AssetPricer, …).net_greeks` equals the **hand
/// leaf-direct sum** of the three asset classes' own Greeks to 1e-12 — proving the
/// seam returns each asset's OWN sensitivities, not an FX proxy for the metal/equity.
#[test]
fn longhand_additive_oracle() {
    let (fx, fx_opt, fx_n, fx_vi) = fx_pos();
    let (metal, metal_opt, metal_n, metal_vi) = metal_pos();
    let (eq, eq_opt, eq_n, eq_ei) = equity_pos();

    // Production path: build the cube, firm-aggregate through the seam.
    let mut cube = Cube::new();
    cube.upsert(fact(1, &fx));
    cube.upsert(fact(2, &metal));
    cube.upsert(fact(3, &eq));
    let firm = cube.firm_aggregate(&OnePillar);

    // --- Independent hand sum (code-disjoint: calls the LEAF crates directly) ---
    // FX leaf (celnet-vanilla) on the FX inputs.
    let g_fx = celnet_vanilla::greeks(fx_opt, &fx_vi);
    // Metal leaf = the SAME FX two-rate arithmetic on the metal's lowered inputs.
    let g_metal = celnet_vanilla::greeks(metal_opt, &metal_vi);
    // Equity leaf (celnet-equity-vanilla) on the equity inputs.
    let g_eq = celnet_equity_vanilla::greeks(eq_opt, &eq_ei);

    // FX/metal canonical delta is the convention-pinned spot-unadjusted delta; for FX
    // that equals delta_spot by construction. Sum the per-asset Greeks × notional with
    // a plain loop.
    let want_delta =
        celnet_vanilla::convention_delta(DeltaConvention::SpotUnadjusted, fx_opt, &fx_vi) * fx_n
            + celnet_vanilla::convention_delta(
                DeltaConvention::SpotUnadjusted,
                metal_opt,
                &metal_vi,
            ) * metal_n
            + g_eq.delta_spot * eq_n;
    let want_vega = g_fx.vega * fx_n + g_metal.vega * metal_n + g_eq.vega * eq_n;
    let want_gamma = g_fx.gamma * fx_n + g_metal.gamma * metal_n + g_eq.gamma * eq_n;
    let want_theta = g_fx.theta * fx_n + g_metal.theta * metal_n + g_eq.theta * eq_n;
    let want_premium = g_fx.price * fx_n + g_metal.price * metal_n + g_eq.price * eq_n;

    assert!(
        is_close(firm.net_greeks.delta_base, want_delta, 1e-12, 1e-6),
        "delta: cube {} vs hand {want_delta}",
        firm.net_greeks.delta_base
    );
    assert!(is_close(firm.net_greeks.vega, want_vega, 1e-12, 1e-6));
    assert!(is_close(firm.net_greeks.gamma, want_gamma, 1e-12, 1e-9));
    assert!(is_close(firm.net_greeks.theta, want_theta, 1e-12, 1e-6));
    assert!(is_close(
        firm.net_greeks.premium_quote,
        want_premium,
        1e-12,
        1e-6
    ));

    // The equity leg is GENUINELY the equity leaf's delta (not the FX leaf's): a
    // direct FX-leaf "proxy" on the equity inputs would differ. Prove the seam did NOT
    // proxy by checking the equity leaf delta differs from an FX-leaf misprice.
    let eq_fx_proxy = celnet_vanilla::greeks(
        eq_opt,
        &VanillaInputs::new(100.0, 105.0, 0.20, 1.0, 0.03, 0.0),
    )
    .delta_spot;
    assert!(
        (g_eq.delta_spot - eq_fx_proxy).abs() > 1e-6,
        "the equity leaf delta must differ from an FX proxy (else the test is vacuous)"
    );
}

/// G9: the production FRTB **delta** SbM total over the mixed book equals a fully
/// **hand-written** SbM algebra (risk weights hand-typed from MAR21.88, the
/// within/cross-bucket quadratic forms written out with explicit loops). They agree
/// because the cited paragraph is the shared truth, not an in-repo constant.
#[test]
fn longhand_frtb_oracle() {
    let (fx, ..) = fx_pos();
    let (metal, ..) = metal_pos();
    let (eq, ..) = equity_pos();

    let mut cube = Cube::new();
    cube.upsert(fact(1, &fx));
    cube.upsert(fact(2, &metal));
    cube.upsert(fact(3, &eq));
    let firm = cube.firm_aggregate(&OnePillar);

    // Production delta buckets with the cited regulatory RW.
    let rw = StandardFrtbParams::new();
    let buckets = standard_delta_buckets(&firm, &rw);

    // --- Independent longhand SbM delta charge ---
    // MAR21.88: FX delta RW = 0.15 (hand-typed). Each currency leg is its own bucket
    // with a single (degenerate) factor, so K_b = |WS_b| and the cross-bucket γ is
    // structurally absent (FX delta), giving K = √(Σ_b WS_b²) — a plain Euclidean norm
    // of the per-currency weighted net deltas.
    let rw_fx: f64 = 0.15; // MAR21.88 verbatim.
    // Re-net the per-currency exposure by hand from the canonical leaves.
    let mut legs: Vec<(Ccy, f64)> = Vec::new();
    let net = |c: Ccy, a: f64, legs: &mut Vec<(Ccy, f64)>| {
        if let Some(s) = legs.iter_mut().find(|(k, _)| *k == c) {
            s.1 += a;
        } else {
            legs.push((c, a));
        }
    };
    for leaf in &firm.leaves {
        // Mirror the production bucketing: a leg with no fiat-pair projection (the
        // equity asset-unit leg) is skipped (its asset-unit bucketing is the deferred
        // cross-asset numeraire-collapse case); its quote funding leg has no pair
        // either, so the equity contributes no fiat delta bucket here. FX/metal legs
        // net by currency.
        let Some(pair) = leaf.underlying.as_ccy_pair() else {
            continue;
        };
        net(pair.base, leaf.greeks.delta_base, &mut legs);
        net(pair.quote, -leaf.greeks.delta_base * leaf.spot, &mut legs);
    }
    // K = √(Σ_b (RW·s_b)²) — single-factor degenerate buckets, no cross-γ.
    let want_k: f64 = legs
        .iter()
        .map(|(_, s)| (rw_fx * s) * (rw_fx * s))
        .sum::<f64>()
        .sqrt();

    // Production: assemble a single-class delta SbM charge with the structurally-absent
    // FX cross-bucket γ ≡ 0; the three-scenario max of a γ=0 class is K each scenario.
    let params = celnet_risk_cube::SbmParams::new(buckets, |_, _| 0.0);
    let prod = celnet_risk_cube::delta_vega_class(&params);
    assert!(
        is_close(prod.capital(), want_k, 1e-12, 1e-6),
        "production delta SbM {} vs longhand {want_k}",
        prod.capital()
    );

    // Anti-circularity: the production RW equals the hand-typed paragraph value (and
    // the relieved value), catching any future drift of the production constant.
    assert!(is_close(rw.fx_delta_rw(), 0.15, 0.0, 1e-15));
    assert!(is_close(
        StandardFrtbParams::fx_delta_rw_liquid(),
        0.15 / 2.0_f64.sqrt(),
        0.0,
        1e-15
    ));
}

/// G10: `curvature_legs` (through the seam) equals a leaf-direct CVR± reprice to 1e-9
/// for the FX leg AND the equity leg — proving curvature re-prices the **correct**
/// leaf through the seam (not an FX proxy for the equity).
#[test]
fn longhand_curvature_oracle() {
    let rw: f64 = 0.15; // MAR21.98 / MAR21.88 curvature shift.

    // --- FX leg ---
    let (fx, fx_opt, fx_n, fx_vi) = fx_pos();
    let (cvr_up, cvr_down) =
        celnet_risk_cube::curvature_legs(&AssetPricer, std::slice::from_ref(&fx), rw);
    // Independent FX leaf reprice net of delta.
    let base = celnet_vanilla::price(fx_opt, &fx_vi) * fx_n;
    let up = celnet_vanilla::price(
        fx_opt,
        &VanillaInputs::new(
            fx_vi.spot * (1.0 + rw),
            fx_vi.strike,
            fx_vi.vol,
            fx_vi.t,
            fx_vi.r_dom,
            fx_vi.r_for,
        ),
    ) * fx_n;
    let down = celnet_vanilla::price(
        fx_opt,
        &VanillaInputs::new(
            fx_vi.spot * (1.0 - rw),
            fx_vi.strike,
            fx_vi.vol,
            fx_vi.t,
            fx_vi.r_dom,
            fx_vi.r_for,
        ),
    ) * fx_n;
    let linear = celnet_vanilla::greeks(fx_opt, &fx_vi).delta_spot * fx_n * rw * fx_vi.spot;
    let want_up = -((up - base) - linear);
    let want_down = -((down - base) + linear);
    assert!(
        is_close(cvr_up, want_up, 1e-9, 1e-3),
        "FX CVR+ {cvr_up} vs {want_up}"
    );
    assert!(is_close(cvr_down, want_down, 1e-9, 1e-3));

    // --- Equity leg (through the SAME seam fn; must reprice the equity leaf) ---
    let (eq, eq_opt, eq_n, eq_ei) = equity_pos();
    let (e_up, e_down) =
        celnet_risk_cube::curvature_legs(&AssetPricer, std::slice::from_ref(&eq), rw);
    let shift_eq = |mult: f64| {
        celnet_equity_vanilla::EquityInputs::new(
            eq_ei.spot * mult,
            eq_ei.strike,
            eq_ei.vol,
            eq_ei.t,
            eq_ei.r,
            eq_ei.q,
            eq_ei.repo,
        )
    };
    let eq_base = celnet_equity_vanilla::price(eq_opt, &eq_ei) * eq_n;
    let eq_up = celnet_equity_vanilla::price(eq_opt, &shift_eq(1.0 + rw)) * eq_n;
    let eq_down = celnet_equity_vanilla::price(eq_opt, &shift_eq(1.0 - rw)) * eq_n;
    let eq_linear =
        celnet_equity_vanilla::greeks(eq_opt, &eq_ei).delta_spot * eq_n * rw * eq_ei.spot;
    let want_e_up = -((eq_up - eq_base) - eq_linear);
    let want_e_down = -((eq_down - eq_base) + eq_linear);
    assert!(
        is_close(e_up, want_e_up, 1e-9, 1e-3),
        "equity CVR+ via seam {e_up} vs leaf-direct {want_e_up}"
    );
    assert!(is_close(e_down, want_e_down, 1e-9, 1e-3));

    // Vacuity guard: the equity CVR differs from what an FX proxy would give.
    let fx_proxy_up = {
        let b = celnet_vanilla::price(
            eq_opt,
            &VanillaInputs::new(100.0, 105.0, 0.20, 1.0, 0.03, 0.0),
        ) * eq_n;
        let u = celnet_vanilla::price(
            eq_opt,
            &VanillaInputs::new(100.0 * (1.0 + rw), 105.0, 0.20, 1.0, 0.03, 0.0),
        ) * eq_n;
        let l = celnet_vanilla::greeks(
            eq_opt,
            &VanillaInputs::new(100.0, 105.0, 0.20, 1.0, 0.03, 0.0),
        )
        .delta_spot
            * eq_n
            * rw
            * 100.0;
        -((u - b) - l)
    };
    let _ = (&eq, OnePillar); // keep the imports honest.
    assert!(
        (e_up - fx_proxy_up).abs() > 1e-9,
        "equity curvature must differ from an FX proxy (else vacuous)"
    );

    // A used-import touch for CommodityRef (the manifest names it; a commodity arm
    // reuses the equity path's generic cost-of-carry through the CommodityLeaf).
    let _ = CommodityRef::new(Symbol::new("BRENT", ""), Ccy::USD);
}

// ===========================================================================
// W6 rigor §3.3 pre-kill oracles: naive-double-loop / partition-conservation /
// quantile-boundary / Taylor-expansion / curvature-two-revaluation pins.
// ===========================================================================

/// A pillar map that varies with the position tenor (so the ladder genuinely
/// buckets) — calendar-days label + a fixed 0.50Δ pillar.
struct DaysPillar;
impl VegaPillarMap for DaysPillar {
    fn pillar_of(&self, _leaf: &CanonicalLeaf, pos: &PositionRisk) -> VegaPillar {
        VegaPillar::new((pos.inputs.t * 365.0).round() as u32, 5000)
    }
}

/// A fact with explicit org keys on every dimension (for the partition tests).
fn keyed_fact(id: u32, keys: (u32, u32, u32, u32, u32), position: &PositionRisk) -> RiskFact {
    let (trader, book, desk, location, entity) = keys;
    RiskFact {
        position_id: PositionId(id),
        key: FactKey {
            trader: TraderId(trader),
            book: BookId(book),
            desk: DeskId(desk),
            underlying: position.underlying.clone(),
            location: LocationId(location),
            entity: EntityId(entity),
        },
        measure: FactMeasure {
            leaf: canonicalize(position).unwrap(),
            position: position.clone(),
            exotic: None,
        },
        surface_version: 1,
    }
}

/// An exotic fact: the leaf is the leg's REAL exotic Greek set; the position is the
/// underlying-vanilla bucketing metadata (never priced for an exotic fact).
fn exotic_fact(id: u32, keys: (u32, u32, u32, u32, u32), leg: ExoticLeg) -> RiskFact {
    let (trader, book, desk, location, entity) = keys;
    let meta = PositionRisk::fx(
        leg.pair,
        OptionType::Call,
        leg.notional,
        leg.inputs,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    );
    RiskFact {
        position_id: PositionId(id),
        key: FactKey {
            trader: TraderId(trader),
            book: BookId(book),
            desk: DeskId(desk),
            underlying: meta.underlying.clone(),
            location: LocationId(location),
            entity: EntityId(entity),
        },
        measure: FactMeasure {
            leaf: leg.canonical_leaf(),
            position: meta,
            exotic: Some(leg),
        },
        surface_version: 1,
    }
}

/// The mixed book used by the aggregation oracles: 4 vanilla facts (2 FX pairs +
/// metal + equity) and 1 exotic digital fact, spread across ≥2 distinct keys on
/// EVERY dimension.
fn mixed_book() -> Vec<RiskFact> {
    let fx2 = PositionRisk::fx(
        CcyPair::new(Ccy::USD, Ccy::JPY),
        OptionType::Put,
        8_000_000.0,
        VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    );
    let leg = ExoticLeg::new(
        eurusd(),
        ExoticKind::Digital(celnet_exotics::DigitalKind::cash(OptionType::Call)),
        5_000_000.0,
        VanillaInputs::new(1.10, 1.11, 0.10, 0.25, 0.04, 0.02),
    );
    vec![
        keyed_fact(1, (1, 1, 1, 1, 1), &fx_pos().0),
        keyed_fact(2, (2, 2, 1, 1, 1), &fx2),
        keyed_fact(3, (1, 1, 2, 2, 2), &metal_pos().0),
        keyed_fact(4, (2, 2, 2, 2, 2), &equity_pos().0),
        exotic_fact(5, (3, 3, 3, 1, 2), leg),
    ]
}

/// **Naive-double-loop oracle**: the firm aggregate's EVERY `NetGreeks` field and
/// every vega-ladder bucket equal an independent plain-loop sum over the facts'
/// canonical leaves (vanilla AND exotic), and the fact routing splits vanilla
/// positions from exotic legs exactly.
#[test]
fn firm_aggregate_equals_naive_double_loop() {
    let facts = mixed_book();
    let mut cube = Cube::new();
    for f in &facts {
        cube.upsert(f.clone());
    }
    let firm = cube.firm_aggregate(&DaysPillar);

    // Independent naive sums (plain loops over the same immutable leaves).
    let mut want = [0.0_f64; 11];
    let mut ladder: Vec<(VegaPillar, f64)> = Vec::new();
    for f in &facts {
        let g = &f.measure.leaf.greeks;
        for (slot, v) in want.iter_mut().zip([
            g.delta_base,
            g.gamma,
            g.vega,
            g.theta,
            g.vanna,
            g.volga,
            g.charm,
            g.speed,
            g.zomma,
            g.color,
            f.measure.leaf.premium_quote,
        ]) {
            *slot += v;
        }
        let p = VegaPillar::new((f.measure.position.inputs.t * 365.0).round() as u32, 5000);
        match ladder.iter_mut().find(|(q, _)| *q == p) {
            Some(slot) => slot.1 += g.vega,
            None => ladder.push((p, g.vega)),
        }
    }
    let got = [
        firm.net_greeks.delta_base,
        firm.net_greeks.gamma,
        firm.net_greeks.vega,
        firm.net_greeks.theta,
        firm.net_greeks.vanna,
        firm.net_greeks.volga,
        firm.net_greeks.charm,
        firm.net_greeks.speed,
        firm.net_greeks.zomma,
        firm.net_greeks.color,
        firm.net_greeks.premium_quote,
    ];
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            is_close(*g, w, 1e-12, 1e-9),
            "NetGreeks field #{i}: cube {g} vs naive {w}"
        );
        // Vacuity guard: every summed field is genuinely non-zero on this book.
        assert!(w.abs() > 0.0, "field #{i} oracle must not be vacuous");
    }
    // The ladder: per-pillar and total (float order identical here — plain loop).
    assert!(!ladder.is_empty());
    for (p, v) in &ladder {
        assert!(
            is_close(firm.vega_ladder.vega_in(*p), *v, 1e-12, 1e-9),
            "pillar {p:?}: cube {} vs naive {v}",
            firm.vega_ladder.vega_in(*p)
        );
    }
    let naive_total: f64 = ladder.iter().map(|(_, v)| *v).sum();
    assert!(is_close(firm.vega_ladder.total(), naive_total, 1e-12, 1e-9));
    assert_eq!(firm.vega_ladder.pillars().count(), ladder.len());

    // Routing: 4 vanilla positions, 1 exotic leg, 5 leaves.
    assert_eq!(firm.positions.len(), 4);
    assert_eq!(firm.exotic_legs.len(), 1);
    assert_eq!(firm.leaves.len(), 5);
}

/// **Partition conservation**: for EVERY dimension, the group-by nodes partition
/// the facts — each `NetGreeks` field, the ladder total and the constituent counts
/// sum back to the firm aggregate, and the book genuinely spans ≥2 groups per
/// dimension.
#[test]
fn group_by_partitions_conserve_the_firm_total() {
    let facts = mixed_book();
    let mut cube = Cube::new();
    for f in &facts {
        cube.upsert(f.clone());
    }
    let firm = cube.firm_aggregate(&DaysPillar);
    let field = |n: &celnet_risk_cube::NetGreeks| {
        [
            n.delta_base,
            n.gamma,
            n.vega,
            n.theta,
            n.vanna,
            n.volga,
            n.charm,
            n.speed,
            n.zomma,
            n.color,
            n.premium_quote,
        ]
    };
    for dim in [
        DimensionId::Trader,
        DimensionId::Book,
        DimensionId::Desk,
        DimensionId::Underlying,
        DimensionId::Location,
        DimensionId::Entity,
    ] {
        let nodes = cube.group_by(dim, &DaysPillar);
        assert!(nodes.len() >= 2, "{dim:?} must split into ≥2 groups");
        // Distinct group keys (no group folded into another).
        for (i, a) in nodes.iter().enumerate() {
            for b in &nodes[i + 1..] {
                assert_ne!(a.group, b.group, "{dim:?} group keys must be distinct");
            }
        }
        let firm_f = field(&firm.net_greeks);
        let mut sum_f = [0.0_f64; 11];
        let mut sum_ladder = 0.0_f64;
        let (mut n_pos, mut n_ex, mut n_leaves) = (0usize, 0usize, 0usize);
        for n in &nodes {
            for (s, v) in sum_f.iter_mut().zip(field(&n.net_greeks)) {
                *s += v;
            }
            sum_ladder += n.vega_ladder.total();
            n_pos += n.positions.len();
            n_ex += n.exotic_legs.len();
            n_leaves += n.leaves.len();
        }
        for (i, (s, f)) in sum_f.iter().zip(firm_f).enumerate() {
            assert!(
                is_close(*s, f, 1e-12, 1e-9),
                "{dim:?} field #{i}: Σgroups {s} vs firm {f}"
            );
        }
        assert!(is_close(sum_ladder, firm.vega_ladder.total(), 1e-12, 1e-9));
        assert_eq!(n_pos, firm.positions.len());
        assert_eq!(n_ex, firm.exotic_legs.len());
        assert_eq!(n_leaves, firm.leaves.len());
    }
}

/// A pricer whose PV is **exactly the spot** (a linear cash-asset leaf): P&L under a
/// relative spot shock `s` is exactly `notional·spot·s`, dyadic-exact for dyadic
/// inputs — so the VaR/ES quantile reduction can be pinned **bit-for-bit** against a
/// hand-derived tail. Implements the same public seam the production dispatcher does.
struct LinearPricer;
impl CarryPricer for LinearPricer {
    fn price(&self, _opt: OptionType, i: &CarryInputs) -> Result<f64, CarryPriceError> {
        Ok(i.spot)
    }
    fn price_greeks(
        &self,
        _opt: OptionType,
        i: &CarryInputs,
    ) -> Result<CarryGreeks, CarryPriceError> {
        Ok(CarryGreeks {
            price: i.spot,
            delta_spot: 1.0,
            delta_forward: 1.0,
            gamma: 0.0,
            vega: 0.0,
            theta: 0.0,
            rates: RateSensitivities::Carry {
                discount_rho: 0.0,
                carry_rho: 0.0,
            },
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        })
    }
}

/// A unit linear position on a non-FX underlying (so BOTH the bump-and-revalue and
/// the sensitivity lens route through `LinearPricer`, not the FX adjoint fast path).
fn linear_unit_position() -> PositionRisk {
    let u = Underlying::Equity(EquityRef::new(Symbol::new("CASH", ""), Ccy::USD));
    PositionRisk::carry(
        u.clone(),
        OptionType::Call,
        1.0,
        CarryInputs::new(1.0, 1.0, 0.1, 1.0, u, Carry::CostOfCarry { r: 0.0, b: 0.0 }),
    )
}

/// **Quantile/tail boundary, exact**: hand-built dyadic P&L vectors where the tail
/// index lands exactly on a rank boundary and exactly on a tie; expected VaR/ES are
/// re-derived in-test by sort + tail-mean and asserted **bit-for-bit** through both
/// the bump-and-revalue oracle and the AAD sensitivity lens (which share the
/// reduction). Kills the `floor`/`max(1)`/`min(n)`/index/`max(0.0)` mutants that
/// are otherwise measure-zero.
#[test]
fn var_es_quantile_boundary_is_exact() {
    let p = linear_unit_position();
    // Deliberately unsorted dyadic shocks; P&L == shock exactly (spot = n = 1).
    let shocks = [
        0.375, -0.5, 0.1875, -0.0625, 0.3125, 0.0625, -0.25, 0.25, -0.125, 0.125,
    ];
    let scen: Vec<Scenario> = shocks.iter().map(|&s| Scenario::spot(s)).collect();
    // Sorted: [-0.5, -0.25, -0.125, -0.0625, 0.0625, 0.125, 0.1875, 0.25, 0.3125, 0.375]

    let check = |alpha: f64, want_var: f64, want_es: f64| {
        let o = historical_var_es(&LinearPricer, std::slice::from_ref(&p), &scen, alpha);
        assert_eq!(o.var.to_bits(), want_var.to_bits(), "VaR at α={alpha}");
        assert_eq!(o.es.to_bits(), want_es.to_bits(), "ES at α={alpha}");
        // The sensitivity lens shares the identical reduction; the linear position's
        // Taylor P&L is the same exact dyadic vector ⇒ bit-identical results.
        let f = sensitivity_var_es(&LinearPricer, std::slice::from_ref(&p), &scen, alpha);
        assert_eq!(f.var.to_bits(), want_var.to_bits());
        assert_eq!(f.es.to_bits(), want_es.to_bits());
    };
    // α=0.90: (1−α)·10 = 0.999… (binary 1−0.9 < 0.1) ⇒ floor 0 ⇒ the `max(1)`
    // floor bites ⇒ tail = 1 ⇒ VaR = ES = 0.5 (kills the max(1) arm).
    check(0.90, 0.5, 0.5);
    // α=0.75 (dyadic): (1−α)·n = 2.5 ⇒ floor ⇒ tail = 2 ⇒ VaR = 0.25,
    // ES = (0.5+0.25)/2 = 0.375 (a ceil/round mutant takes tail 3 ⇒ VaR 0.125 ≠).
    check(0.75, 0.25, 0.375);
    // α=0.5 (dyadic): (1−α)·n = 5.0 EXACTLY ⇒ tail = 5 lands on the integer
    // boundary ⇒ the rank-5 P&L is the gain 0.0625 ⇒ VaR floors at 0 while
    // ES = −(−0.9375+0.0625)/5 = 0.175 stays a genuine loss.
    check(0.5, 0.0, 0.175);
    // α=0: tail = n; the best P&L is a gain and the mean P&L is a gain ⇒ both
    // floors bite: VaR = ES = 0 exactly.
    check(0.0, 0.0, 0.0);

    // Exact TIE spanning the boundary rank: three equal worst losses, tail = 2.
    let tie_shocks = [
        0.25, -0.25, 0.375, -0.25, 0.3125, -0.25, 0.1875, -0.125, 0.0625, 0.125,
    ];
    let tie: Vec<Scenario> = tie_shocks.iter().map(|&s| Scenario::spot(s)).collect();
    let o = historical_var_es(&LinearPricer, std::slice::from_ref(&p), &tie, 0.75);
    assert_eq!(o.var.to_bits(), 0.25_f64.to_bits());
    assert_eq!(o.es.to_bits(), 0.25_f64.to_bits());

    // A single scenario: tail = max(floor(0.01·1), 1) = 1 (kills the `.max(1)` arm).
    let one = [Scenario::spot(-0.125)];
    let o1 = historical_var_es(&LinearPricer, std::slice::from_ref(&p), &one, 0.99);
    assert_eq!(o1.var.to_bits(), 0.125_f64.to_bits());
    assert_eq!(o1.es.to_bits(), 0.125_f64.to_bits());

    // Empty scenario sets are exactly (0, 0) through every public entry point.
    let empty: [Scenario; 0] = [];
    for ve in [
        historical_var_es(&LinearPricer, std::slice::from_ref(&p), &empty, 0.99),
        sensitivity_var_es(&LinearPricer, std::slice::from_ref(&p), &empty, 0.99),
        node_var_es_combined(&LinearPricer, std::slice::from_ref(&p), &[], &empty, 0.99),
        node_var_es_sensitivity_combined(
            &LinearPricer,
            std::slice::from_ref(&p),
            &[],
            &empty,
            0.99,
        ),
    ] {
        assert_eq!(ve.var.to_bits(), 0.0_f64.to_bits());
        assert_eq!(ve.es.to_bits(), 0.0_f64.to_bits());
    }
}

/// **Taylor expansion, exact**: `PositionSensitivity::taylor_pnl` equals the
/// hand-typed `Δ·dS + ½Γ·dS² + ν·dσ + ½volga·dσ² + vanna·dS·dσ + ρ_r·dr + ρ_b·db`
/// literal re-derivation, bit-for-bit on dyadic inputs with a distinct prime weight
/// per term (any single-term mutation shifts the exact sum).
#[test]
fn taylor_pnl_matches_hand_expansion() {
    let s = PositionSensitivity {
        spot: 2.0,
        delta_spot: 3.0,
        gamma: 5.0,
        vega: 7.0,
        volga: 11.0,
        vanna: 13.0,
        discount_rho: 17.0,
        carry_rho: 19.0,
    };
    let sc = Scenario {
        spot_rel: 0.25,
        vol_abs: 0.5,
        discount_abs: 0.125,
        carry_abs: 0.0625,
    };
    // Hand expansion (every product/sum below is dyadic-exact):
    //   dS = 2.0·0.25 = 0.5, dσ = 0.5
    //   3·0.5 + 0.5·5·0.5² + 7·0.5 + 0.5·11·0.5² + 13·0.5·0.5 + 17·0.125 + 19·0.0625
    // = 1.5   + 0.625      + 3.5   + 1.375       + 3.25       + 2.125    + 1.1875
    // = 13.5625
    let d_s: f64 = 2.0 * 0.25;
    let d_v: f64 = 0.5;
    let want = 3.0 * d_s
        + 0.5 * 5.0 * d_s * d_s
        + 7.0 * d_v
        + 0.5 * 11.0 * d_v * d_v
        + 13.0 * d_s * d_v
        + 17.0 * 0.125
        + 19.0 * 0.0625;
    assert_eq!(want.to_bits(), 13.5625_f64.to_bits());
    assert_eq!(s.taylor_pnl(sc).to_bits(), want.to_bits());

    // Term isolation: each single-factor scenario reproduces its own term exactly
    // (so a cross-term confusion cannot cancel in the combined pin).
    assert_eq!(
        s.taylor_pnl(Scenario::spot(0.25)).to_bits(),
        (3.0_f64 * 0.5 + 0.5 * 5.0 * 0.25).to_bits()
    );
    assert_eq!(
        s.taylor_pnl(Scenario::vol(0.5)).to_bits(),
        (7.0_f64 * 0.5 + 0.5 * 11.0 * 0.25).to_bits()
    );
}

/// **Curvature legs = two revaluations**: `vanilla_curvature_legs` (and the
/// `sbm_curvature_spot` floor and the vanilla+exotic combination) match a direct
/// in-test up/down re-pricing through `celnet-vanilla` net of the analytic linear
/// term — the MAR21 CVR± arithmetic re-derived longhand.
#[test]
fn curvature_legs_match_two_revaluations() {
    let rw: f64 = 0.15;
    let vi = VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01);
    let call = PositionRisk::fx(
        eurusd(),
        OptionType::Call,
        -10_000_000.0,
        vi,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    );
    let put = PositionRisk::fx(
        eurusd(),
        OptionType::Put,
        -10_000_000.0,
        vi,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    );
    let positions = [call, put];

    // Longhand: two full revaluations through the FX leaf, net of the linear term.
    let reprice = |mult: f64| -> f64 {
        positions
            .iter()
            .map(|p| {
                let s =
                    VanillaInputs::new(vi.spot * mult, vi.strike, vi.vol, vi.t, vi.r_dom, vi.r_for);
                celnet_vanilla::price(p.option, &s) * p.notional_base
            })
            .sum()
    };
    let base = reprice(1.0);
    let linear: f64 = positions
        .iter()
        .map(|p| celnet_vanilla::greeks(p.option, &vi).delta_spot * p.notional_base * rw * vi.spot)
        .sum();
    let want_up = -((reprice(1.0 + rw) - base) - linear);
    let want_down = -((reprice(1.0 - rw) - base) + linear);

    let (up, down) = vanilla_curvature_legs(&AssetPricer, &positions, rw);
    assert!(is_close(up, want_up, 1e-9, 1e-3), "CVR+ {up} vs {want_up}");
    assert!(
        is_close(down, want_down, 1e-9, 1e-3),
        "CVR− {down} vs {want_down}"
    );
    // A short straddle is short gamma: both legs are genuine losses (positive CVR).
    assert!(want_up > 0.0 && want_down > 0.0);

    // The floored node charge is exactly max(up, down, 0).
    let charge = sbm_curvature_spot(&AssetPricer, &positions, rw);
    assert!(is_close(
        charge,
        want_up.max(want_down).max(0.0),
        1e-12,
        1e-9
    ));
    // And a LONG straddle floors at exactly zero (both CVR legs negative).
    let long = [
        PositionRisk::fx(
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            vi,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ),
        PositionRisk::fx(
            eurusd(),
            OptionType::Put,
            10_000_000.0,
            vi,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ),
    ];
    assert_eq!(
        sbm_curvature_spot(&AssetPricer, &long, rw).to_bits(),
        0.0_f64.to_bits()
    );

    // Combined vanilla+exotic: legs are summed BEFORE the single max (the
    // non-additivity the MAR21 reduction prescribes).
    let leg = ExoticLeg::new(
        eurusd(),
        ExoticKind::Digital(celnet_exotics::DigitalKind::cash(OptionType::Call)),
        -20_000_000.0,
        VanillaInputs::new(1.10, 1.11, 0.10, 0.25, 0.04, 0.02),
    );
    let (e_up, e_down) = leg.curvature_legs(rw);
    let want_combined = (want_up + e_up).max(want_down + e_down).max(0.0);
    let got_combined =
        sbm_curvature_spot_combined(&AssetPricer, &positions, std::slice::from_ref(&leg), rw);
    assert!(is_close(got_combined, want_combined, 1e-9, 1e-3));
    // The exotic genuinely moves the node charge (vacuity guard) and the combined
    // charge is NOT the sum of two independent maxes when directions disagree.
    assert!((got_combined - charge).abs() > 1e-3);

    // UP-DOMINANT combined regime. The fixture above resolves its max on the
    // DOWN side (the short near-the-money digital is linear-term dominated:
    // CVR⁻ ≫ CVR⁺), so the UP sum `Σ CVR⁺` never decides the reduction there —
    // a corrupted up-leg combination (`+` → `−`/`×`) would survive it. Mirror
    // the digital LONG (+50M): its CVR⁺ is strongly positive (the +15% shock
    // loss net of the large positive delta hedge) and CVR⁻ strongly negative,
    // so the UP side wins the max outright — ASSERTED below, not assumed, so
    // the fixture cannot silently rot back into the down regime.
    let long_digital = ExoticLeg::new(
        eurusd(),
        ExoticKind::Digital(celnet_exotics::DigitalKind::cash(OptionType::Call)),
        50_000_000.0,
        VanillaInputs::new(1.10, 1.11, 0.10, 0.25, 0.04, 0.02),
    );
    let (e2_up, e2_down) = long_digital.curvature_legs(rw);
    let up_sum = up + e2_up;
    let down_sum = down + e2_down;
    // Fixture-regime guards (vacuity): the exotic up contribution is material
    // and the up sum wins by a wide margin, so a corrupted up sum MUST move
    // the reported node charge.
    assert!(e2_up.abs() > 1.0e5, "exotic CVR+ must be material: {e2_up}");
    assert!(
        up_sum > down_sum.max(0.0) + 1.0e5,
        "the UP sum must win outright: up {up_sum} vs down {down_sum}"
    );
    let got_up = sbm_curvature_spot_combined(
        &AssetPricer,
        &positions,
        std::slice::from_ref(&long_digital),
        rw,
    );
    // Bit-exact: the production reduction and this recomputation evaluate the
    // identical float ops on identical leg values (same pub leg functions).
    assert_eq!(got_up.to_bits(), up_sum.to_bits());
}

/// **Combined VaR/ES folds the exotic tail in**: the vanilla+exotic node VaR equals
/// a hand-derived sort+tail over per-scenario `(vanilla P&L + exotic P&L)`, and the
/// exotic genuinely changes the tail vs the vanilla-only node.
#[test]
fn combined_var_es_includes_exotic_tail() {
    let p = fx_pos().0;
    let leg = ExoticLeg::new(
        eurusd(),
        ExoticKind::Digital(celnet_exotics::DigitalKind::cash(OptionType::Put)),
        25_000_000.0,
        VanillaInputs::new(1.10, 1.09, 0.10, 0.5, 0.04, 0.02),
    );
    let scen: Vec<Scenario> = (-8..=8)
        .filter(|i| *i != 0)
        .map(|i| Scenario::spot(f64::from(i) * 0.01))
        .collect();
    let alpha = 0.90;

    // Hand tail: per-scenario combined P&L, plain sort, floor-tail, mean.
    let mut pnl: Vec<f64> = scen
        .iter()
        .map(|s| {
            celnet_risk_cube::position_pnl(&AssetPricer, &p, *s)
                + celnet_risk_cube::exotic_node_pnl(std::slice::from_ref(&leg), *s)
        })
        .collect();
    pnl.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let tail = (((1.0 - alpha) * pnl.len() as f64).floor() as usize).max(1);
    let want_var = (-pnl[tail - 1]).max(0.0);
    let want_es = (-(pnl[..tail].iter().sum::<f64>() / tail as f64)).max(0.0);

    let got = node_var_es_combined(
        &AssetPricer,
        std::slice::from_ref(&p),
        std::slice::from_ref(&leg),
        &scen,
        alpha,
    );
    assert!(is_close(got.var, want_var, 1e-12, 1e-6));
    assert!(is_close(got.es, want_es, 1e-12, 1e-6));
    // Exotic contribution is real: dropping the legs changes the tail.
    let vanilla_only =
        node_var_es_combined(&AssetPricer, std::slice::from_ref(&p), &[], &scen, alpha);
    assert!((got.var - vanilla_only.var).abs() > 1.0);
    // No-exotic reduction contract: combined([], legs=[]) == historical exactly.
    let hist = historical_var_es(&AssetPricer, std::slice::from_ref(&p), &scen, alpha);
    assert_eq!(vanilla_only.var.to_bits(), hist.var.to_bits());
    assert_eq!(vanilla_only.es.to_bits(), hist.es.to_bits());

    // The sensitivity-combined lens: Taylor vanilla + EXACT exotic reprice, same
    // reduction — re-derived by hand from the public pieces.
    let sens = node_sensitivities(&AssetPricer, std::slice::from_ref(&p));
    let mut tpnl: Vec<f64> = scen
        .iter()
        .map(|s| {
            sens.iter().map(|q| q.taylor_pnl(*s)).sum::<f64>()
                + celnet_risk_cube::exotic_node_pnl(std::slice::from_ref(&leg), *s)
        })
        .collect();
    tpnl.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let want_tvar = (-tpnl[tail - 1]).max(0.0);
    let got_t = node_var_es_sensitivity_combined(
        &AssetPricer,
        std::slice::from_ref(&p),
        std::slice::from_ref(&leg),
        &scen,
        alpha,
    );
    assert!(is_close(got_t.var, want_tvar, 1e-12, 1e-6));
    // And it reduces exactly to `sensitivity_var_es` with no exotic legs.
    let sens_only =
        node_var_es_sensitivity_combined(&AssetPricer, std::slice::from_ref(&p), &[], &scen, alpha);
    let sens_free = sensitivity_var_es(&AssetPricer, std::slice::from_ref(&p), &scen, alpha);
    assert_eq!(sens_only.var.to_bits(), sens_free.var.to_bits());
    assert_eq!(sens_only.es.to_bits(), sens_free.es.to_bits());
}
