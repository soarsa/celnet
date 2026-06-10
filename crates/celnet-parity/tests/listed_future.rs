//! Parity rows — the **option on a listed future** (proto arm 31,
//! `Instrument.product.listed_future_option`) priced by the
//! `celnet-commodity-vanilla` Black-76 engine on the carry seam (`b = 0` on a
//! future) under BOTH premium-margining conventions reproduces genuinely
//! independent oracles.
//!
//! Four gates, none of which re-runs the engine as its own check:
//!
//!   (i)   **Hand-pinned literals, INDEPENDENTLY recomputed**: the published
//!         Black-76 worked examples (Haug 2007 2nd ed. §1.2.2 call `1.7011`;
//!         Hull's futures-option worked example call `1.12`) plus an ITM point,
//!         each re-derived offline with **CPython 3** IEEE-754 doubles and
//!         `Φ(x) = ½(1 + math.erf(x/√2))` — a float route disjoint from the
//!         engine's `libm::erfc`-based `norm_cdf` (the recomputation steps are
//!         spelled out per literal below).
//!   (ii)  **Futures-style undiscounted parity** `C − P = F − K` (model-free:
//!         the daily margin sweep holds the parity portfolio at zero financing):
//!         `to_bits`-exact at the ATM anchors, where both sides are exactly
//!         representable (`F = spot·e^{0·t}` is exact and `K = F`, so both sides
//!         are exactly `0.0` — verified bitwise; `libm` is deterministic, so the
//!         anchors are stable), and at `1e-12` across the off-ATM grid (there
//!         the two sides are DIFFERENT float rounding routes — a Φ-weighted
//!         bracket difference vs a plain subtraction — measured ≤ 128 ulps
//!         apart, so bit equality off-ATM is not a property of correctly-rounded
//!         arithmetic; `1e-12` sits ~100× above the observed rounding noise and
//!         ~10 orders below any genuine pricing defect).
//!   (iii) **Equity-style == df · futures-style, bitwise**: the engine computes
//!         the discounted (equity-style) price as `df·(undiscounted bracket)` in
//!         the IDENTICAL operation order as the futures-style closed form, so
//!         the identity holds to the last bit on every grid point — a structural
//!         `to_bits` gate (the documented reproducibility carve-out), covering
//!         the margining dispatcher too.
//!   (iv)  **Engine vs the independent `libm::erf` golden oracle** under both
//!         margining conventions (`celnet_golden::oracle::black76_price` /
//!         `black76_undiscounted_price`): the oracle takes the forward directly,
//!         so it is reached without re-forming the engine's
//!         `spot · forward_factor(t)` product.

use celnet_commodity_vanilla::{
    CommodityInputs, Margining, futures_style_price, price, price_with_margining,
};
use celnet_core::assert_close;
use celnet_golden::oracle::{self, Cp};
use celnet_types::{Carry, OptionType};

fn cp(opt: OptionType) -> Cp {
    match opt {
        OptionType::Call => Cp::Call,
        OptionType::Put => Cp::Put,
    }
}

/// (i) Published worked examples + an ITM point, every full-precision literal
/// INDEPENDENTLY recomputed offline (CPython 3, IEEE-754, `math.erf` route).
#[test]
fn pinned_published_and_offline_recomputed_literals() {
    // --- Haug (2007) 2nd ed., §1.2.2, the Black-76 worked example -----------
    // F = K = 19, t = 0.75, r = 0.10, σ = 0.28; published call = 1.7011 (4 dp).
    // CPython recomputation (Φ via math.erf — disjoint from the engine's erfc):
    //   σ√t = 0.24248711305964282
    //   d1  = (ln 1 + ½·0.28²·0.75)/σ√t = 0.12124355652982143,  d2 = −d1
    //   Φ(d1) = 0.5482509372784995,  Φ(d2) = 0.4517490627215005
    //   df  = e^{−0.075} = 0.9277434863285529
    //   C_eq  = df·19·(Φ(d1) − Φ(d2)) = 1.701050725236268   → Haug's 1.7011
    //   C_fut = 19·(Φ(d1) − Φ(d2))    = 1.8335356165829815  (undiscounted)
    //   ATM forward ⇒ P = C under both conventions (C − P = df·(F−K) = 0 and
    //   C − P = F − K = 0 respectively).
    let haug = CommodityInputs::on_future(19.0, 19.0, 0.28, 0.75, 0.10);
    let c_eq = price(OptionType::Call, &haug);
    assert_close!(c_eq, 1.7011, 5e-4, 5e-4); // the published 4-dp value
    assert_close!(c_eq, 1.701_050_725_236_268, 1e-12, 1e-12);
    assert_close!(
        price(OptionType::Put, &haug),
        1.701_050_725_236_268,
        1e-12,
        1e-12
    );
    assert_close!(
        futures_style_price(OptionType::Call, &haug),
        1.833_535_616_582_981_5,
        1e-12,
        1e-12
    );
    assert_close!(
        futures_style_price(OptionType::Put, &haug),
        1.833_535_616_582_981_5,
        1e-12,
        1e-12
    );

    // --- Hull's futures-option worked example -------------------------------
    // (Options, Futures, and Other Derivatives): F = K = 20, σ = 0.25,
    // r = 0.09, t = 4/12; published call = 1.12 (2 dp). CPython recomputation
    // (same erf route): C_eq = 1.1166414565589438, C_fut = 1.1506482517116225.
    let hull = CommodityInputs::on_future(20.0, 20.0, 0.25, 0.333_333_333_333_333_3, 0.09);
    let hull_call = price(OptionType::Call, &hull);
    assert_close!(hull_call, 1.12, 5e-3, 5e-3); // the published 2-dp value
    assert_close!(hull_call, 1.116_641_456_558_943_8, 1e-12, 1e-12);
    assert_close!(
        futures_style_price(OptionType::Call, &hull),
        1.150_648_251_711_622_5,
        1e-12,
        1e-12
    );

    // --- ITM point (a shared scale/sign error cannot hide at the money) -----
    // F = 45, K = 40, t = 0.5, r = 0.04, σ = 0.35. CPython recomputation:
    //   σ√t = 0.24748737341529164
    //   d1  = (ln 1.125 + ½·0.35²·0.5)/σ√t = 0.5996590194011638
    //   d2  = d1 − σ√t                     = 0.3521716459858722
    //   Φ(d1)  = 0.7256332475037105,   Φ(d2)  = 0.6376452301181443
    //   Φ(−d1) = 0.2743667524962895,   Φ(−d2) = 0.36235476988185567
    //   df  = e^{−0.02} = 0.9801986733067553
    //   C_eq  = df·(45·Φ(d1) − 40·Φ(d2))   = 7.0061532488809934
    //   P_eq  = df·(40·Φ(−d2) − 45·Φ(−d1)) = 2.1051598823472166
    //   C_fut = 45·Φ(d1) − 40·Φ(d2)        = 7.147686932941198
    //   P_fut = 40·Φ(−d2) − 45·Φ(−d1)      = 2.1476869329411983
    let itm = CommodityInputs::on_future(45.0, 40.0, 0.35, 0.5, 0.04);
    assert_close!(
        price(OptionType::Call, &itm),
        7.006_153_248_880_993_4,
        1e-12,
        1e-12
    );
    assert_close!(
        price(OptionType::Put, &itm),
        2.105_159_882_347_216_6,
        1e-12,
        1e-12
    );
    assert_close!(
        futures_style_price(OptionType::Call, &itm),
        7.147_686_932_941_198,
        1e-12,
        1e-12
    );
    assert_close!(
        futures_style_price(OptionType::Put, &itm),
        2.147_686_932_941_198_3,
        1e-12,
        1e-12
    );
}

/// (ii) Futures-style undiscounted parity `C − P = F − K`: bitwise at the ATM
/// anchors (both sides exactly `0.0`), `1e-12` across the off-ATM grid.
#[test]
fn futures_style_parity_is_undiscounted() {
    // ATM anchors: with carry b = 0 the forward factor is e^{0·t} = 1.0 exactly,
    // so F = spot exactly and F − K = 0.0 exactly; C − P lands on exactly 0.0
    // too (verified bitwise — deterministic libm makes the anchors stable).
    for &(f, vol, t, r) in &[
        (19.0, 0.28, 0.75, 0.10),
        (100.0, 0.20, 1.0, 0.05),
        (20.0, 0.25, 0.333_333_333_333_333_3, 0.09),
    ] {
        let i = CommodityInputs::on_future(f, f, vol, t, r);
        let c = futures_style_price(OptionType::Call, &i);
        let p = futures_style_price(OptionType::Put, &i);
        let rhs = i.forward() - f;
        assert_eq!(
            rhs.to_bits(),
            0.0f64.to_bits(),
            "F − K must be exact at ATM"
        );
        assert_eq!(
            (c - p).to_bits(),
            rhs.to_bits(),
            "undiscounted parity must be exact at the ATM anchor F={f}"
        );
    }
    // Off-ATM grid: the two sides are different float rounding routes (measured
    // ≤ 128 ulps apart), so the model-free law is gated at 1e-12 — ~100× the
    // observed rounding noise, ~10 orders below any genuine defect.
    for &(f, k, vol, t, r) in &[
        (45.0, 40.0, 0.35, 0.5, 0.04),
        (50.0, 55.0, 0.30, 1.0, 0.05),
        (150.0, 140.0, 0.18, 0.25, 0.045),
        (2000.0, 2100.0, 0.16, 1.5, 0.038),
    ] {
        let i = CommodityInputs::on_future(f, k, vol, t, r);
        let c = futures_style_price(OptionType::Call, &i);
        let p = futures_style_price(OptionType::Put, &i);
        assert_close!(c - p, i.forward() - k, 1e-12, 1e-12);
    }
}

/// (iii) Equity-style == df · futures-style to the last bit on every grid point
/// (including b ≠ 0 spot-representation carries), and the margining dispatcher
/// is bitwise-faithful to both closed forms.
#[test]
fn equity_style_is_discounted_futures_style_bitwise() {
    let cases = [
        // (spot, strike, vol, t, r, b)
        (19.0, 19.0, 0.28, 0.75, 0.10, 0.0),
        (45.0, 40.0, 0.35, 0.5, 0.04, 0.0),
        (100.0, 90.0, 0.22, 0.5, 0.03, 0.01),
        (62.0, 70.0, 0.40, 0.5, 0.03, -0.06),
        (2000.0, 2100.0, 0.16, 1.5, 0.038, 0.0),
    ];
    for &(s, k, vol, t, r, b) in &cases {
        let i = CommodityInputs::new(s, k, vol, t, Carry::CostOfCarry { r, b });
        let df = i.discount_df();
        for opt in [OptionType::Call, OptionType::Put] {
            let equity = price(opt, &i);
            let undiscounted = futures_style_price(opt, &i);
            assert_eq!(
                equity.to_bits(),
                (df * undiscounted).to_bits(),
                "equity-style must be df · futures-style bitwise: {opt:?} F={s} K={k}"
            );
            assert_eq!(
                price_with_margining(opt, Margining::EquityStyle, &i).to_bits(),
                equity.to_bits()
            );
            assert_eq!(
                price_with_margining(opt, Margining::FuturesStyle, &i).to_bits(),
                undiscounted.to_bits()
            );
        }
    }
}

/// (iv) Engine vs the independent `libm::erf` golden oracle under both premium
/// margining conventions. The listed-future representation feeds the oracle the
/// forward DIRECTLY (the input spot IS `F`, carry `b = 0`), so the oracle route
/// never re-forms the engine's `spot · forward_factor(t)` product.
#[test]
fn engine_matches_independent_erf_oracle_under_both_marginings() {
    let cases = [
        // (future, strike, vol, t, r)
        (19.0, 19.0, 0.28, 0.75, 0.10),
        (45.0, 40.0, 0.35, 0.5, 0.04),
        (50.0, 55.0, 0.30, 1.0, 0.05),
        (20.0, 20.0, 0.25, 0.333_333_333_333_333_3, 0.09),
        (150.0, 140.0, 0.18, 0.25, 0.045),
        (2000.0, 2100.0, 0.16, 1.5, 0.038),
    ];
    for &(f, k, vol, t, r) in &cases {
        let i = CommodityInputs::on_future(f, k, vol, t, r);
        for opt in [OptionType::Call, OptionType::Put] {
            assert_close!(
                price(opt, &i),
                oracle::black76_price(cp(opt), f, k, vol, t, r),
                1e-12,
                1e-12
            );
            assert_close!(
                futures_style_price(opt, &i),
                oracle::black76_undiscounted_price(cp(opt), f, k, vol, t),
                1e-12,
                1e-12
            );
        }
    }
}
