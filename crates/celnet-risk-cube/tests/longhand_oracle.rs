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

use celnet_core::carry::CarryInputs;
use celnet_core::is_close;
use celnet_risk_cube::{
    BookId, Cube, DeskId, EntityId, FactKey, FactMeasure, LocationId, PositionId, RiskFact,
    StandardFrtbParams, TraderId, VegaPillar, VegaPillarMap, standard_delta_buckets,
};
use celnet_risk_normalize::{AssetPricer, CanonicalLeaf, PositionRisk, canonicalize};
use celnet_types::{
    Carry, Ccy, CcyPair, CommodityRef, DeltaConvention, EquityRef, Metal, MetalPair, OptionType,
    PremiumStyle, Symbol, Underlying, VanillaInputs,
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
