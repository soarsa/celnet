//! Parity rows — the **perpetual (no-expiry) American vanilla** (proto arm 30,
//! `Instrument.product.perpetual_option`) priced by
//! [`celnet_exotics::perpetual_price`] on the agnostic carry seam reproduces
//! genuinely independent oracles.
//!
//! Three gates, none of which re-runs the engine as its own check (the
//! FRTB-0.75ρ circular-oracle lesson):
//!
//!   (i)   **Code-disjoint closed-form oracle**
//!         ([`celnet_golden::oracle::perpetual_american_price`]): the
//!         characteristic root is found by expanding-bracket **bisection of
//!         `ψ(y) = ½σ²·y·(y−1) + b·y − r` in its product form** and the value is
//!         completed with `libm::pow` — sharing neither the engine's
//!         standard-form discriminant + cancellation-free root pairing nor its
//!         `exp(y·ln x)` seam power route. `celnet-golden` does not depend on
//!         `celnet-exotics`' perpetual module, so the two routes share no code.
//!   (ii)  **Finite-maturity American-FD `T → ∞` sandwich**: the projected-SOR
//!         free-boundary finite difference ([`celnet_exotics::american_fd`] — a
//!         genuinely code-disjoint early-exercise pricer, PDE grid vs closed
//!         form) at `t = 50y` and `t = 100y` must bracket the perpetual from
//!         below, monotonically, and converge toward it: an American option with
//!         more time is worth more, and the perpetual is the supremum.
//!   (iii) **The `b ≥ r` law** (structural, model-free): holding the asset never
//!         costs carry relative to discounting, so the perpetual call is never
//!         exercised and equals the spot **exactly** — asserted `to_bits` (the
//!         documented exact-arm carve-out from "no float `==`"), independently
//!         on the engine and on the golden oracle.

use celnet_core::assert_close;
use celnet_exotics::{
    AmericanGrid, AmericanOption, ExerciseStyle, PerpetualInputs, american_fd,
    perpetual_exercise_boundary, perpetual_price,
};
use celnet_golden::oracle::{self, Cp};
use celnet_types::{Carry, OptionType, VanillaInputs};

fn cp(opt: OptionType) -> Cp {
    match opt {
        OptionType::Call => Cp::Call,
        OptionType::Put => Cp::Put,
    }
}

/// (i) Engine vs the independent bisection-+-`libm::pow` oracle across
/// continuation, exercised and degenerate regimes — including the `σ → 0`
/// stiffness regime where `|b|/σ² → ∞` (the engine's cancellation-free pairing
/// is load-bearing there; the oracle's bisection is immune by construction, so
/// agreement kills a shared-route cancellation bug).
#[test]
fn perpetual_engine_matches_independent_bisection_oracle() {
    let cases = [
        // (spot, strike, vol, r, b)
        (100.0, 100.0, 0.30, 0.08, 0.04),  // call/put continuation
        (100.0, 110.0, 0.25, 0.06, 0.02),  // ITM put continuation
        (1.30, 1.25, 0.10, 0.05, 0.01),    // FX-scale levels
        (80.0, 95.0, 0.40, 0.07, -0.03),   // negative carry
        (50.0, 45.0, 0.15, 0.03, 0.0),     // zero carry
        (100.0, 100.0, 1e-3, 0.08, 0.04),  // σ→0 regime: stable pairing matters
        (90.0, 100.0, 0.20, 0.0, 0.05),    // r = 0, b > ½σ²: active put boundary
        (100.0, 90.0, 0.20, 0.0, 0.01),    // r = 0, b ≤ ½σ²: put = K, call = S
        (1.40, 1.10, 0.10, 0.05, -0.02),   // call beyond the boundary: intrinsic
        (100.0, 150.0, 0.10, 0.04, 0.035), // put beyond the boundary: intrinsic
    ];
    for &(s, k, vol, r, b) in &cases {
        let i = PerpetualInputs::new(s, k, vol, Carry::CostOfCarry { r, b });
        for opt in [OptionType::Call, OptionType::Put] {
            let engine = perpetual_price(opt, &i);
            let reference = oracle::perpetual_american_price(cp(opt), s, k, vol, r, b);
            assert_close!(engine, reference, 1e-10, 1e-12);
        }
    }
}

/// (i) The FX two-rate carry arm maps onto the oracle's `(r, b)` exactly as the
/// seam defines it (`r = r_dom`, `b = r_dom − r_for`) — the wire shape every
/// arm-30 FX instrument prices through.
#[test]
fn perpetual_fx_carry_matches_oracle_two_rate_mapping() {
    let cases = [
        // (spot, strike, vol, r_dom, r_for)
        (1.30, 1.25, 0.10, 0.05, 0.01),
        (1.10, 1.15, 0.105, 0.03, 0.015),
        (100.0, 150.0, 0.10, 0.04, 0.005),
    ];
    for &(s, k, vol, r_dom, r_for) in &cases {
        let i = PerpetualInputs::new(s, k, vol, Carry::FxRates { r_dom, r_for });
        for opt in [OptionType::Call, OptionType::Put] {
            let engine = perpetual_price(opt, &i);
            let reference =
                oracle::perpetual_american_price(cp(opt), s, k, vol, r_dom, r_dom - r_for);
            assert_close!(engine, reference, 1e-10, 1e-12);
        }
    }
}

/// (ii) The American-FD `T → ∞` sandwich. At `S = K = 100`, `σ = 0.20`,
/// `r = 0.04`, `b = 0` (FX `r_dom = r_for`) the characteristic quadratic factors
/// exactly — `ψ(y) = 0.02·y² − 0.02·y − 0.04 = 0.02·(y − 2)·(y + 1)`, so
/// `y₁ = 2`, `y₂ = −1` — and the perpetual closed form is derived BY HAND (never
/// read back from the engine):
///
/// ```text
/// call: S*  = K·2/(2−1) = 200,  V = (200−100)·(100/200)²    = 25
/// put:  S** = K·(−1)/(−2) = 50, V = (100−50)·(100/50)^{−1}  = 25
/// ```
///
/// The PSOR free-boundary finite difference at `t = 50y` and `t = 100y` must
/// satisfy `FD(50) < FD(100) < perpetual` (more exercise time is worth strictly
/// more; the perpetual is the supremum) and genuinely converge. Measured on the
/// default grid: gaps `2.7e-1` (50y) → `1.7e-2` (100y), i.e. ~250× and ~17× the
/// grid error, so the strict inequalities cannot be satisfied by grid noise.
#[test]
fn american_fd_long_maturity_sandwich_brackets_perpetual() {
    let (s, k, vol, r) = (100.0, 100.0, 0.20, 0.04);
    let perp_in = PerpetualInputs::new(s, k, vol, Carry::FxRates { r_dom: r, r_for: r });
    for opt in [OptionType::Call, OptionType::Put] {
        let perp = perpetual_price(opt, &perp_in);
        // The hand-derived factored value (independent of every pricer here).
        assert_close!(perp, 25.0, 1e-12, 1e-12);

        let fd = |t: f64| {
            let i = VanillaInputs::new(s, k, vol, t, r, r);
            american_fd(
                &(&i).into(),
                &AmericanOption {
                    option: opt,
                    strike: k,
                    style: ExerciseStyle::American,
                },
                AmericanGrid::default(),
            )
        };
        let fd50 = fd(50.0);
        let fd100 = fd(100.0);
        assert!(
            fd50 < fd100,
            "{opt:?}: American FD must be monotone in maturity: FD(50y) {fd50} >= FD(100y) {fd100}"
        );
        assert!(
            fd100 < perp,
            "{opt:?}: the perpetual is the supremum: FD(100y) {fd100} >= perpetual {perp}"
        );
        // Genuine convergence: the residual T-gap shrinks by far more than grid
        // noise (measured ratio ≈ 0.06; the 0.25 bound leaves 4× headroom while
        // still rejecting a stalled or wrong-limit solver).
        assert!(
            perp - fd100 < 0.25 * (perp - fd50),
            "{opt:?}: FD not converging to the perpetual: gaps {} (50y) vs {} (100y)",
            perp - fd50,
            perp - fd100
        );
        // And by 100y the FD value has essentially reached the perpetual
        // (measured 0.07% of the value; bound 0.2%).
        assert!(
            perp - fd100 < 2e-3 * perp,
            "{opt:?}: FD(100y) {fd100} too far below the perpetual {perp}"
        );
    }
}

/// (iii) The `b ≥ r` law: the perpetual call is never exercised and equals the
/// spot EXACTLY — bitwise on the engine, bitwise-agreeing on the independently
/// coded golden oracle (the same financial law encoded twice, disjointly), with
/// an infinite exercise boundary.
#[test]
fn carry_dominated_perpetual_call_is_spot_exactly() {
    for &(r, b) in &[(0.05, 0.05), (0.03, 0.06), (0.0, 0.0), (0.02, 0.025)] {
        let i = PerpetualInputs::new(123.45, 100.0, 0.2, Carry::CostOfCarry { r, b });
        let engine = perpetual_price(OptionType::Call, &i);
        assert_eq!(engine.to_bits(), 123.45f64.to_bits(), "r={r} b={b}");
        assert_eq!(
            oracle::perpetual_american_price(Cp::Call, 123.45, 100.0, 0.2, r, b).to_bits(),
            engine.to_bits(),
            "oracle disagrees on the b >= r law at r={r} b={b}"
        );
        assert!(perpetual_exercise_boundary(OptionType::Call, &i).is_infinite());
    }
    // The FX form of the law: r_for ≤ 0 ⇔ b = r_dom − r_for ≥ r_dom.
    let fx = PerpetualInputs::new(
        1.25,
        1.10,
        0.10,
        Carry::FxRates {
            r_dom: 0.02,
            r_for: -0.005,
        },
    );
    assert_eq!(
        perpetual_price(OptionType::Call, &fx).to_bits(),
        1.25f64.to_bits()
    );
}
