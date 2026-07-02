//! Deterministic regression for the single-barrier reflection-exponent overflow.
//!
//! In the degenerate high-|carry|/low-vol regime the drift `b = r_d − r_f` is
//! huge relative to `σ²`, so `μ = b/σ² − ½` is unbounded and the Reiner-
//! Rubinstein image weight `(H/S)^{2μ}` overflows f64. Evaluated literally the
//! image term is `∞·Φ(far tail) = ∞·0 = NaN`, so `single_barrier_price`
//! returned a non-finite value (surfaced by `payoff_bounds_fuzz`,
//! "KI must be finite"). The fix recombines the image weight and the normal
//! tail in log-space, recovering the finite limit:
//!
//!   * drift slams the spot INTO the barrier  ⇒ hit a.s. ⇒ knock-in → vanilla,
//!     knock-out → 0;
//!   * barrier so far the drift never reaches it ⇒ knock-in → 0,
//!     knock-out → vanilla.
//!
//! Both limits are asserted below so a future regression to a naive
//! "overflow ⇒ knock-in = vanilla" shortcut (wrong for the far-barrier case)
//! is caught. Inputs are all inside the fuzz generator's clamped domain
//! (spot,strike ∈ [1e-6,1e6]; vol ∈ [1e-6,2]; t ∈ [1e-9,10]; r ∈ [-1,1];
//! barrier_rel ∈ [0.01,10]) and satisfy its `σ√T ≥ 0.01` realism guard.

use celnet_exotics::{
    BarrierKind, BarrierStyle, ExoticInputs, SingleBarrier, single_barrier_price,
};
use celnet_types::{Carry, Ccy, CcyPair, OptionType, Underlying, VanillaInputs};

fn inputs(spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> ExoticInputs {
    ExoticInputs::new(
        spot,
        strike,
        vol,
        t,
        Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD)),
        Carry::FxRates { r_dom, r_for },
    )
}

fn vanilla(
    opt: OptionType,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    celnet_vanilla::price(opt, &VanillaInputs::new(spot, strike, vol, t, r_dom, r_for))
}

/// Every model-free contract the fuzz test checks for a zero-rebate single
/// barrier: both legs finite and in `[0, vanilla]`, and exact in/out parity.
fn assert_bounds(
    label: &str,
    i: &ExoticInputs,
    opt: OptionType,
    strike: f64,
    barrier: f64,
    v: f64,
) -> (f64, f64) {
    let up = barrier >= i.spot;
    let leg = |style| {
        single_barrier_price(
            i,
            SingleBarrier {
                kind: BarrierKind {
                    up,
                    style,
                    option: opt,
                },
                strike,
                barrier,
                rebate: 0.0,
            },
        )
    };
    let ki = leg(BarrierStyle::KnockIn);
    let ko = leg(BarrierStyle::KnockOut);
    assert!(
        ki.is_finite() && ko.is_finite(),
        "{label} {opt:?}: legs must be finite, ki={ki} ko={ko}"
    );
    assert!(
        ki >= -1e-12 && ko >= -1e-12,
        "{label} {opt:?}: legs must be ≥ 0, ki={ki} ko={ko}"
    );
    assert!(
        ki <= v + 1e-12 && ko <= v + 1e-12,
        "{label} {opt:?}: legs must be ≤ vanilla, ki={ki} ko={ko} v={v}"
    );
    let tol = 1e-9 * (1.0 + v.abs());
    assert!(
        (ki + ko - v).abs() <= tol,
        "{label} {opt:?}: parity ki+ko={} v={v}",
        ki + ko
    );
    (ki, ko)
}

#[test]
fn up_barrier_reflection_overflow_is_finite_and_bounded() {
    // b = 1.0, vol = 0.006 ⇒ μ ≈ 2.78e4; H = 5·S ⇒ (H/S)^{2μ} overflows.
    // σ√T = 0.006·√5 = 0.0134 ≥ 0.01. Moderate premium magnitudes so the pure
    // reflection-overflow limit is exercised without the separate large-|value|
    // `KO = vanilla − KI` parity-subtraction rounding.
    for &(s, k, h) in &[(1.0, 1.0, 5.0), (1.0, 30.0, 5.0), (1.0, 0.5, 8.0)] {
        let i = inputs(s, k, 0.006, 5.0, 1.0, 0.0);
        for opt in [OptionType::Call, OptionType::Put] {
            let v = vanilla(opt, s, k, 0.006, 5.0, 1.0, 0.0);
            assert_bounds("up-slam", &i, opt, k, h, v);
        }
    }
}

#[test]
fn down_barrier_reflection_overflow_is_finite_and_bounded() {
    // b = -2.0 ⇒ μ hugely negative; H = 0.2·S < S ⇒ (H/S)^{2μ} overflows.
    // Moderate premium magnitudes (see the up-barrier note).
    for &(s, k, h) in &[(1.0, 1.0, 0.2), (1.0, 0.03, 0.2), (1.0, 2.0, 0.1)] {
        let i = inputs(s, k, 0.006, 5.0, -1.0, 1.0);
        for opt in [OptionType::Call, OptionType::Put] {
            let v = vanilla(opt, s, k, 0.006, 5.0, -1.0, 1.0);
            assert_bounds("down-slam", &i, opt, k, h, v);
        }
    }
}

#[test]
fn drift_slam_limit_knock_in_is_the_vanilla() {
    // Enormous up-drift onto a nearby up-barrier ⇒ hit a.s. ⇒ KI → vanilla,
    // KO → 0. (b = 1.0, vol = 0.006, μ ≈ 2.78e4, H = 5·S.)
    let (s, k, h) = (1.0, 1.0, 5.0);
    let i = inputs(s, k, 0.006, 5.0, 1.0, 0.0);
    let opt = OptionType::Call;
    let v = vanilla(opt, s, k, 0.006, 5.0, 1.0, 0.0);
    let (ki, ko) = assert_bounds("slam", &i, opt, k, h, v);
    assert!(
        (ki - v).abs() <= 1e-9 * (1.0 + v.abs()),
        "slam: KI({ki}) → vanilla({v})"
    );
    assert!(ko <= 1e-9 * (1.0 + v.abs()), "slam: KO({ko}) → 0");
}

#[test]
fn far_barrier_limit_knock_in_vanishes() {
    // Up-drift but the up-barrier is astronomically far (H = 1e6·S with S = 1)
    // ⇒ the reflection weight still overflows, yet the barrier is essentially
    // never reached ⇒ KI → 0, KO → vanilla. This is the discriminator that a
    // naive "overflow ⇒ KI = vanilla" shortcut would get WRONG.
    let (s, k, h) = (1.0, 1.0, 1e6);
    let i = inputs(s, k, 0.05, 5.0, 1.0, -1.0);
    let opt = OptionType::Call;
    let v = vanilla(opt, s, k, 0.05, 5.0, 1.0, -1.0);
    let (ki, ko) = assert_bounds("far", &i, opt, k, h, v);
    assert!(ki <= 1e-6 * (1.0 + v.abs()), "far: KI({ki}) → 0");
    assert!(
        (ko - v).abs() <= 1e-9 * (1.0 + v.abs()),
        "far: KO({ko}) → vanilla({v})"
    );
}
