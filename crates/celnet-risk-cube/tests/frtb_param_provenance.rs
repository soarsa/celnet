//! FRTB-SA **parameter provenance** gate (W6 rigor §3.3): every risk weight /
//! correlation / curvature parameter exposed by `frtb_params` is pinned against a
//! literal **hand-typed from the published Basel FRTB-SA text** (the BCBS MAR21
//! paragraph cited per constant) — never computed from another in-repo constant.
//! This re-applies the 0.75ρ circular-oracle lesson to the *whole* parameter
//! surface, and pins the bucket/correlation **assignment** functions
//! (`standard_delta_buckets` / `standard_vega_buckets` / `standard_curvature_buckets`)
//! against hand-built leaf-direct expectations, so a mutant anywhere on the
//! regulatory-params surface changes a pinned value.

use celnet_core::is_close;
use celnet_risk_cube::NodeAggregate;
use celnet_risk_cube::{
    ExoticKind, ExoticLeg, NetGreeks, RiskBucket, StandardFrtbParams, VegaLadder,
    curvature_legs as frtb_curvature_legs, standard_curvature_buckets, standard_delta_buckets,
    standard_vega_buckets,
};
use celnet_risk_normalize::{
    AssetPricer, CanonicalGreeks, CanonicalLeaf, PositionRisk, canonicalize,
};
use celnet_types::{
    Ccy, CcyPair, DeltaConvention, DigitalKind, EquityRef, OptionType, PremiumStyle, Symbol,
    Underlying, VanillaInputs,
};

// The hand-coded, code-disjoint closed-form exotic pricer the cube re-prices its
// digital legs through (the cube does not depend on `celnet-exotics`). Single-homed
// in `src/test_support.rs` and `#[path]`-included (one definition across all tests).
#[path = "../src/test_support.rs"]
pub mod test_support;
use test_support::DigitalTestPricer;

fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}
fn usdjpy() -> CcyPair {
    CcyPair::new(Ccy::USD, Ccy::JPY)
}

/// A hand-built FX canonical leaf carrying ONLY a delta (the delta-bucket input).
fn delta_leaf(pair: CcyPair, spot: f64, delta_base: f64) -> CanonicalLeaf {
    CanonicalLeaf {
        underlying: Underlying::Fx(pair),
        spot,
        greeks: CanonicalGreeks {
            delta_base,
            gamma: 0.0,
            vega: 0.0,
            theta: 0.0,
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        },
        premium_quote: 0.0,
        vega_premium_ccy: pair.quote,
        quoted_was_premium_adjusted: false,
    }
}

/// An empty node carrying only the supplied leaves (what `standard_delta_buckets`
/// reads).
fn node_with_leaves(leaves: Vec<CanonicalLeaf>) -> NodeAggregate {
    NodeAggregate {
        group: 0,
        net_greeks: NetGreeks::zero(),
        vega_ladder: VegaLadder::new(),
        positions: Vec::new(),
        exotic_legs: Vec::new(),
        leaves,
    }
}

/// Pack a 3-letter ISO code by hand (the documented bucket-id packing), from
/// hand-typed byte values — independent of the production `ccy_id`.
fn hand_ccy_id(code: &str) -> u32 {
    let b = code.as_bytes();
    (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2])
}

/// Every cited constant on `StandardFrtbParams`, pinned to a value hand-typed from
/// the MAR21 paragraph (never read from another in-repo constant).
///
/// `clippy::approx_constant` is allowed for the same reason as the in-module gate:
/// the √2 literal is deliberately hand-typed from MAR21.88's "divide by √2" so the
/// oracle stays independent of every in-repo constant including `f64::consts`.
#[test]
#[allow(clippy::approx_constant)]
fn every_cited_param_matches_hand_typed_paragraph_value() {
    // MAR21.88: the FX delta risk weight is 15%. Hand-typed 0.15.
    assert_eq!(
        StandardFrtbParams::FX_DELTA_RW.to_bits(),
        0.15_f64.to_bits()
    );
    assert_eq!(
        StandardFrtbParams::new().fx_delta_rw().to_bits(),
        0.15_f64.to_bits()
    );
    // `Default` is the standard (non-liquid, full-weight) parameter set.
    assert_eq!(
        StandardFrtbParams::default().fx_delta_rw().to_bits(),
        0.15_f64.to_bits()
    );

    // MAR21.88 relief: the RW may be divided by √2 for regulator-specified liquid
    // pairs. Hand-typed √2 = 1.4142135623730951; 0.15/√2 = 0.10606601717798212.
    let sqrt2: f64 = 1.414_213_562_373_095_1;
    let want_liquid: f64 = 0.15 / sqrt2;
    assert!((want_liquid - 0.106_066_017_177_982_12).abs() <= 1e-17);
    assert!((StandardFrtbParams::fx_delta_rw_liquid() - want_liquid).abs() <= 1e-15);
    assert!((StandardFrtbParams::liquid_fx().fx_delta_rw() - want_liquid).abs() <= 1e-15);
    // The relief genuinely reduces the weight (the divide is a divide).
    assert!(StandardFrtbParams::fx_delta_rw_liquid() < 0.15);

    // MAR21.94: the simplified vega sigma RW is 0.55. Hand-typed.
    assert_eq!(
        StandardFrtbParams::FX_VEGA_RW_SIGMA.to_bits(),
        0.55_f64.to_bits()
    );
    // MAR21.93 (liquidity-horizon table): LH_FX = 40. Hand-typed.
    assert_eq!(
        StandardFrtbParams::FX_LIQUIDITY_HORIZON.to_bits(),
        40.0_f64.to_bits()
    );
    // MAR21.93/.94: RW_vega = min(0.55·√(40/10), 1.0) = min(1.10, 1.0) = 1.0, i.e.
    // the cap BINDS (the uncapped product 0.55·√4 = 1.10 > 1). Both facts pinned.
    let uncapped: f64 = 0.55 * (40.0_f64 / 10.0).sqrt();
    assert!(uncapped > 1.0, "the MAR21.94 cap must genuinely bind");
    assert_eq!(
        StandardFrtbParams::new().fx_vega_rw().to_bits(),
        1.0_f64.to_bits()
    );
    assert_eq!(
        StandardFrtbParams::liquid_fx().fx_vega_rw().to_bits(),
        1.0_f64.to_bits(),
        "the vega RW carries no liquid-pair relief (MAR21.88 relief is delta-only)"
    );

    // MAR21.94: vega maturity-correlation decay α = 0.01. Hand-typed.
    assert_eq!(
        StandardFrtbParams::VEGA_CORR_ALPHA.to_bits(),
        0.01_f64.to_bits()
    );
    assert_eq!(
        StandardFrtbParams::new().vega_corr_alpha().to_bits(),
        0.01_f64.to_bits()
    );

    // MAR21.98: the curvature shift RW is the largest delta RW of the bucket's
    // factors — for FX's single spot factor, the (relief-adjusted) delta RW.
    assert_eq!(
        StandardFrtbParams::new().fx_curvature_rw().to_bits(),
        0.15_f64.to_bits()
    );
    assert!((StandardFrtbParams::liquid_fx().fx_curvature_rw() - want_liquid).abs() <= 1e-15);
}

/// The MAR21.94 maturity kernel `exp(−α·|t_k−t_l|/min)` re-derived with hand-typed
/// α at several vertex pairs, including symmetry, the degenerate `min ≤ 0` limit,
/// and the exact-equal-maturity identity.
#[test]
fn vega_maturity_kernel_rederived_from_cited_formula() {
    let p = StandardFrtbParams::new();
    let alpha: f64 = 0.01; // MAR21.94, hand-typed.

    // ρ(t,t) = exp(0) = 1 exactly.
    assert_eq!(p.vega_maturity_corr(0.5, 0.5).to_bits(), 1.0_f64.to_bits());
    // Hand re-derivation at several pairs (libm/exp shared math is fine: the
    // *kernel arguments* are the gated arithmetic).
    for &(a, b) in &[(1.0_f64, 2.0_f64), (0.25, 5.0), (0.5, 0.75), (3.0, 10.0)] {
        let want = (-alpha * (a - b).abs() / a.min(b)).exp();
        assert!(
            (p.vega_maturity_corr(a, b) - want).abs() <= 1e-15,
            "kernel({a},{b})"
        );
        // Symmetric in its arguments.
        assert_eq!(
            p.vega_maturity_corr(a, b).to_bits(),
            p.vega_maturity_corr(b, a).to_bits()
        );
    }
    // Degenerate maturities collapse to the perfectly-correlated limit.
    assert_eq!(p.vega_maturity_corr(0.0, 1.0).to_bits(), 1.0_f64.to_bits());
    assert_eq!(p.vega_maturity_corr(-1.0, 2.0).to_bits(), 1.0_f64.to_bits());
    assert_eq!(p.vega_maturity_corr(0.0, 0.0).to_bits(), 1.0_f64.to_bits());
    // Monotone decay with the gap (a genuinely-decaying kernel, not constant 1).
    assert!(p.vega_maturity_corr(0.25, 5.0) < p.vega_maturity_corr(1.0, 2.0));
    assert!(p.vega_maturity_corr(1.0, 2.0) < 1.0);
}

/// `standard_delta_buckets` nets the per-currency legs and weights them by the
/// cited RW — pinned against a fully hand-built expectation on exact-binary spots
/// and deltas (every sum/product below is float-exact, so the asserts are
/// bit-level). A leaf with no fiat-pair projection (equity) contributes nothing.
///
/// `clippy::approx_constant` allowed for the same anti-circular reason as above:
/// the √2 literal is hand-typed from MAR21.88, independent of every in-repo (and
/// standard-library) constant.
#[test]
#[allow(clippy::approx_constant)]
fn delta_buckets_match_hand_netting_and_cited_weight() {
    // Exact-binary spots/deltas so the netting arithmetic is bit-exact.
    let leaves = vec![
        delta_leaf(eurusd(), 1.25, 5.0),
        delta_leaf(eurusd(), 1.25, -2.0),
        delta_leaf(usdjpy(), 160.0, 3.0),
        // An equity leaf (no CcyPair projection) must be skipped entirely.
        CanonicalLeaf {
            underlying: Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD)),
            spot: 100.0,
            greeks: CanonicalGreeks {
                delta_base: 7.0,
                gamma: 0.0,
                vega: 0.0,
                theta: 0.0,
                vanna: 0.0,
                volga: 0.0,
                charm: 0.0,
                speed: 0.0,
                zomma: 0.0,
                color: 0.0,
            },
            premium_quote: 0.0,
            vega_premium_ccy: Ccy::USD,
            quoted_was_premium_adjusted: false,
        },
    ];
    let node = node_with_leaves(leaves);
    let buckets = standard_delta_buckets(&node, &StandardFrtbParams::new());

    // Hand netting, first-seen order:
    //   EUR: +5.0 − 2.0                       = 3.0
    //   USD: −5.0·1.25 + 2.0·1.25 + 3.0       = −0.75
    //   JPY: −3.0·160.0                       = −480.0
    let rw: f64 = 0.15; // MAR21.88 hand-typed.
    let want: [(u32, f64); 3] = [
        (hand_ccy_id("EUR"), rw * 3.0),
        (hand_ccy_id("USD"), rw * -0.75),
        (hand_ccy_id("JPY"), rw * -480.0),
    ];
    assert_eq!(buckets.len(), 3, "equity leaf must not create a bucket");
    for (b, (id, ws)) in buckets.iter().zip(want) {
        assert_eq!(b.id, id);
        assert_eq!(b.ws.len(), 1, "FX delta is a single-factor bucket");
        assert_eq!(b.ws[0].to_bits(), ws.to_bits());
        assert_eq!(
            b.rho_intra.to_bits(),
            0.0_f64.to_bits(),
            "single factor ⇒ degenerate intra-bucket ρ"
        );
    }

    // The liquid-pair set applies the √2-relieved weight to the same nets.
    let liquid = standard_delta_buckets(&node, &StandardFrtbParams::liquid_fx());
    let sqrt2: f64 = 1.414_213_562_373_095_1; // hand-typed
    assert!((liquid[0].ws[0] - (0.15 / sqrt2) * 3.0).abs() <= 1e-15);
}

/// `standard_vega_buckets` groups vega vertices by premium currency, nets vertices
/// at the same maturity, sorts by maturity, applies the cited vega RW, and folds an
/// exotic leg's vega into its quote-ccy bucket — all pinned against leaf-direct
/// (`celnet_vanilla::greeks`) hand sums.
#[test]
fn vega_buckets_match_hand_grouping_and_netting() {
    let fx = |pair: CcyPair, opt, n, vi| {
        PositionRisk::fx(
            pair,
            opt,
            n,
            vi,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    };
    let vi_1y_a = VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02);
    let vi_1y_b = VanillaInputs::new(1.10, 1.08, 0.11, 1.0, 0.04, 0.02);
    let vi_6m = VanillaInputs::new(1.10, 1.11, 0.10, 0.5, 0.04, 0.02);
    let vi_jpy = VanillaInputs::new(160.0, 158.0, 0.11, 0.25, 0.01, 0.05);
    let positions = vec![
        fx(eurusd(), OptionType::Call, 10_000_000.0, vi_1y_a),
        fx(eurusd(), OptionType::Put, -4_000_000.0, vi_1y_b), // nets into the 1Y USD vertex
        fx(eurusd(), OptionType::Call, 6_000_000.0, vi_6m),
        fx(usdjpy(), OptionType::Put, 8_000_000.0, vi_jpy),
    ];
    // An exotic digital (GBPUSD, t = 0.75) joins the USD bucket via its quote ccy.
    let leg = ExoticLeg::new(
        CcyPair::new(Ccy::GBP, Ccy::USD),
        ExoticKind::Digital(DigitalKind::cash(OptionType::Call)),
        5_000_000.0,
        VanillaInputs::new(1.30, 1.32, 0.12, 0.75, 0.03, 0.01),
    );
    let node = NodeAggregate {
        group: 0,
        net_greeks: NetGreeks::zero(),
        vega_ladder: VegaLadder::new(),
        positions,
        exotic_legs: vec![leg],
        leaves: Vec::new(),
    };
    let rw = StandardFrtbParams::new();
    let (buckets, mats) = standard_vega_buckets(&AssetPricer, &DigitalTestPricer, &node, &rw);

    // Hand expectation. RW_vega = 1.0 (MAR21.93/.94, gated above); vegas leaf-direct.
    let w: f64 = 1.0;
    let v_1y = w * celnet_vanilla::greeks(OptionType::Call, &vi_1y_a).vega * 10_000_000.0
        + w * celnet_vanilla::greeks(OptionType::Put, &vi_1y_b).vega * -4_000_000.0;
    let v_6m = w * celnet_vanilla::greeks(OptionType::Call, &vi_6m).vega * 6_000_000.0;
    let v_jpy = w * celnet_vanilla::greeks(OptionType::Put, &vi_jpy).vega * 8_000_000.0;
    let v_dig = w * leg.canonical_leaf(&DigitalTestPricer).greeks.vega;

    // Two buckets, first-seen order: USD (EURUSD premium ccy), then JPY.
    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0].id, hand_ccy_id("USD"));
    assert_eq!(buckets[1].id, hand_ccy_id("JPY"));
    // USD vertices sorted by maturity: 0.5, 0.75 (digital), 1.0 (netted pair).
    assert_eq!(mats[0].len(), 3);
    assert_eq!(mats[0][0].to_bits(), 0.5_f64.to_bits());
    assert_eq!(mats[0][1].to_bits(), 0.75_f64.to_bits());
    assert_eq!(mats[0][2].to_bits(), 1.0_f64.to_bits());
    assert!(is_close(buckets[0].ws[0], v_6m, 1e-12, 1e-9));
    assert!(is_close(buckets[0].ws[1], v_dig, 1e-12, 1e-9));
    assert!(is_close(buckets[0].ws[2], v_1y, 1e-12, 1e-9));
    // JPY bucket: the single USDJPY vertex at t = 0.25.
    assert_eq!(mats[1].len(), 1);
    assert_eq!(mats[1][0].to_bits(), 0.25_f64.to_bits());
    assert!(is_close(buckets[1].ws[0], v_jpy, 1e-12, 1e-9));
    // The buckets carry no representative intra ρ (the exact kernel is the closure).
    assert_eq!(buckets[0].rho_intra.to_bits(), 0.0_f64.to_bits());
    // The netting genuinely netted (the 1Y vertex is NOT either single leg).
    let v_leg_a = w * celnet_vanilla::greeks(OptionType::Call, &vi_1y_a).vega * 10_000_000.0;
    assert!((buckets[0].ws[2] - v_leg_a).abs() > 1.0);
}

/// The same-maturity netting tolerance is `|Δt| < 1e-12` **strict**: two vertices
/// whose maturity gap is exactly 1e-12 stay distinct (the exact-boundary pin that
/// kills the `<`→`<=` boundary mutant; `2e-12 − 1e-12` is float-exact).
#[test]
fn vega_vertex_netting_boundary_is_strict() {
    let fx = |t: f64, n: f64| {
        PositionRisk::fx(
            eurusd(),
            OptionType::Call,
            n,
            VanillaInputs::new(1.10, 1.12, 0.10, t, 0.04, 0.02),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    };
    // Maturity gap exactly 1e-12 (2e-12 = 2·1e-12 in binary, so the gap is exact).
    let node = NodeAggregate {
        group: 0,
        net_greeks: NetGreeks::zero(),
        vega_ladder: VegaLadder::new(),
        positions: vec![fx(1e-12, 1_000_000.0), fx(2e-12, 1_000_000.0)],
        exotic_legs: Vec::new(),
        leaves: Vec::new(),
    };
    let (buckets, mats) = standard_vega_buckets(
        &AssetPricer,
        &DigitalTestPricer,
        &node,
        &StandardFrtbParams::new(),
    );
    assert_eq!(buckets.len(), 1);
    assert_eq!(
        mats[0].len(),
        2,
        "a gap of exactly 1e-12 must NOT net (strict <)"
    );
    // And a zero gap genuinely nets into one vertex.
    let node2 = NodeAggregate {
        group: 0,
        net_greeks: NetGreeks::zero(),
        vega_ladder: VegaLadder::new(),
        positions: vec![fx(0.5, 1_000_000.0), fx(0.5, 2_000_000.0)],
        exotic_legs: Vec::new(),
        leaves: Vec::new(),
    };
    let (b2, m2) = standard_vega_buckets(
        &AssetPricer,
        &DigitalTestPricer,
        &node2,
        &StandardFrtbParams::new(),
    );
    assert_eq!(m2[0].len(), 1, "identical maturities must net");
    assert_eq!(b2[0].ws.len(), 1);
}

/// `standard_curvature_buckets` re-prices through the seam with the MAR21.98
/// (relief-adjusted) curvature shift and passes the bucket id through — pinned
/// against the leg arithmetic computed directly with the hand-typed shift.
///
/// `clippy::approx_constant` allowed for the same anti-circular reason as above:
/// the √2 literal is hand-typed from MAR21.88, independent of every in-repo (and
/// standard-library) constant.
#[test]
#[allow(clippy::approx_constant)]
fn curvature_buckets_use_cited_shift_and_pass_id_through() {
    let p = PositionRisk::fx(
        eurusd(),
        OptionType::Call,
        10_000_000.0,
        VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    );
    let mut node = node_with_leaves(vec![canonicalize(&p).unwrap()]);
    node.positions = vec![p.clone()];

    // LIQUID params: the shift must be the √2-relieved RW (≠ 0.15), so a mutant
    // that drops the relief is caught here.
    let params = StandardFrtbParams::liquid_fx();
    let got = standard_curvature_buckets(&AssetPricer, &node, 77, &params);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, 77, "bucket id must pass through unchanged");
    assert_eq!(got[0].rho_intra.to_bits(), 0.0_f64.to_bits());

    let sqrt2: f64 = 1.414_213_562_373_095_1; // hand-typed (MAR21.88)
    let shift = 0.15 / sqrt2;
    let (want_up, want_down) = frtb_curvature_legs(&AssetPricer, std::slice::from_ref(&p), shift);
    assert!(is_close(got[0].cvr_up, want_up, 1e-12, 1e-9));
    assert!(is_close(got[0].cvr_down, want_down, 1e-12, 1e-9));
    // The relieved shift genuinely differs from the full-weight legs (vacuity guard).
    let (full_up, _) = frtb_curvature_legs(&AssetPricer, std::slice::from_ref(&p), 0.15);
    assert!((got[0].cvr_up - full_up).abs() > 1e-9);

    // RiskBucket::new wiring sanity used across this surface.
    let rb = RiskBucket::new(9, vec![1.5, -0.5], 0.25);
    assert_eq!(rb.id, 9);
    assert_eq!(rb.ws, vec![1.5, -0.5]);
    assert_eq!(rb.rho_intra.to_bits(), 0.25_f64.to_bits());
}
