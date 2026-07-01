//! Garman-Kohlhagen (1983) vanilla FX-option pricing and Greeks.
//!
//! The Garman-Kohlhagen model is Black-Scholes-Merton specialized to FX, with
//! the foreign interest rate `r_f` acting as a continuous dividend yield on the
//! foreign-currency asset. Everything is expressed off the outright forward
//! `F = S·e^{(r_d − r_f)·t}` with the domestic and foreign discount factors kept
//! separate (clean dual-curve / NDO extension later).
//!
//! ```text
//! d1 = [ln(S/K) + (r_d − r_f + ½σ²)·t] / (σ·√t),   d2 = d1 − σ·√t
//! Call = S·e^{−r_f t}·Φ(d1) − K·e^{−r_d t}·Φ(d2)
//! Put  = K·e^{−r_d t}·Φ(−d2) − S·e^{−r_f t}·Φ(−d1)
//! ```
//!
//! The full FX desk Greek set is produced in one pass: spot & forward delta,
//! gamma, vega, theta, **both rhos** (domestic and foreign — FX has two),
//! vanna, volga/vomma, charm, speed, zomma and color. Every Greek is
//! cross-validated against central finite differences in the test suite, and
//! prices are validated against the Black-Scholes textbook benchmark.

//! ## Convention-aware strike↔delta machinery
//!
//! On top of the pricer this crate exposes the FX-desk convention layer that
//! turns a quoted *delta* into a *strike* and back, in any of the four
//! [`celnet_types::DeltaConvention`] variants, plus the premium re-expression
//! and the ATM-strike rules. See the [`delta`], [`premium`], [`atm`] and
//! [`solver`] modules. These consume [`celnet_types::VanillaInputs`] and the
//! convention enums and are validated against the standard FX-smile literature
//! (Clark 2011; Reiswich & Wystup 2010; Wystup 2017) in the test suite.

#![forbid(unsafe_code)]

use celnet_core::{carry_greeks_to_greeks, gbsm_carry_greeks, gbsm_carry_price};
use celnet_types::{Greeks, OptionType, VanillaInputs};

pub mod adjoint;
pub mod atm;
pub mod delta;
pub mod premium;
pub mod pricer;
pub mod solver;

pub use adjoint::adjoint_greeks;
pub use atm::{atm_strike, atm_strike_from_inputs};
pub use delta::{delta as convention_delta, delta_d_strike, premium_adjusted_call_delta_max};
pub use premium::{premium, premium_from_domestic_pips};
pub use pricer::FxPricer;
pub use solver::{DeltaSolveError, strike_from_delta};

/// Present value (domestic premium per 1 unit of base notional).
///
/// The FX (Garman-Kohlhagen) core is the generalized-BSM with net carry
/// `b = r_dom − r_for` and discount `r = r_dom`; it delegates to the one canonical
/// forward-space kernel ([`gbsm_carry_price`]). The former in-crate spot-space GK
/// form is replaced by the unified kernel (ADR-0012 — the sub-1e-12 forward-vs-spot
/// rounding change is accepted; the QuantLib golden grid holds at its documented
/// tolerances and every independent oracle at ≤1e-12).
#[must_use]
pub fn price(opt: OptionType, i: &VanillaInputs) -> f64 {
    gbsm_carry_price(
        opt,
        i.r_dom - i.r_for,
        i.r_dom,
        i.spot,
        i.strike,
        i.vol,
        i.t,
    )
}

/// Price and the full Greek set in a single pass.
///
/// See [`Greeks`] for the precise definition and units of each sensitivity.
///
/// The FX core gBSM math is produced by the one canonical forward-space kernel
/// ([`gbsm_carry_greeks`]) with `b = r_dom − r_for`, `r = r_dom`; the FX leaf then
/// projects the carry-tagged rate block back to its **two-rate `rho_dom`/`rho_for`
/// output basis** via [`carry_greeks_to_greeks`] (`rho_dom = discount_rho +
/// carry_rho`, `rho_for = −carry_rho`). Only the core math unifies — the FX output
/// contract (two rhos) is unchanged (ADR-0012).
#[must_use]
pub fn greeks(opt: OptionType, i: &VanillaInputs) -> Greeks {
    let cg = gbsm_carry_greeks(
        opt,
        i.r_dom - i.r_for,
        i.r_dom,
        i.spot,
        i.strike,
        i.vol,
        i.t,
    );
    carry_greeks_to_greeks(&cg)
}

#[cfg(test)]
mod tests {
    use celnet_core::assert_close;
    use proptest::prelude::*;

    use super::*;

    /// Textbook vanilla benchmark: S=K=100, T=1, r_dom=5%, r_for=0, σ=20%.
    /// Reference values are the standard closed-form analytic results.
    #[test]
    fn vanilla_reference_price() {
        let i = VanillaInputs::new(100.0, 100.0, 0.2, 1.0, 0.05, 0.0);
        assert_close!(
            price(OptionType::Call, &i),
            10.450_583_572_185_565,
            1e-10,
            1e-9
        );
        assert_close!(
            price(OptionType::Put, &i),
            5.573_526_022_256_971,
            1e-10,
            1e-9
        );
    }

    /// Put-call parity must hold exactly (to tolerance) for all valid inputs:
    /// C − P = S·e^{−r_f t} − K·e^{−r_d t}.
    #[test]
    fn put_call_parity_point() {
        let i = VanillaInputs::new(1.2345, 1.30, 0.11, 0.75, 0.03, 0.01);
        let lhs = price(OptionType::Call, &i) - price(OptionType::Put, &i);
        let rhs = i.spot * i.df_for() - i.strike * i.df_dom();
        assert_close!(lhs, rhs, 1e-12, 1e-12);
    }

    // ---- finite-difference Greek oracle ----

    fn fd1<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
        (f(x + h) - f(x - h)) / (2.0 * h)
    }

    fn with_spot(i: &VanillaInputs, s: f64) -> VanillaInputs {
        VanillaInputs { spot: s, ..*i }
    }
    fn with_vol(i: &VanillaInputs, v: f64) -> VanillaInputs {
        VanillaInputs { vol: v, ..*i }
    }
    fn with_t(i: &VanillaInputs, t: f64) -> VanillaInputs {
        VanillaInputs { t, ..*i }
    }
    fn with_rd(i: &VanillaInputs, r: f64) -> VanillaInputs {
        VanillaInputs { r_dom: r, ..*i }
    }
    fn with_rf(i: &VanillaInputs, r: f64) -> VanillaInputs {
        VanillaInputs { r_for: r, ..*i }
    }

    fn check_greeks(opt: OptionType, i: &VanillaInputs) {
        let g = greeks(opt, i);
        let p = |x: &VanillaInputs| price(opt, x);

        // First-order.
        let hs = 1e-4 * i.spot;
        assert_close!(
            g.delta_spot,
            fd1(|s| p(&with_spot(i, s)), i.spot, hs),
            1e-4,
            1e-7
        );
        assert_close!(g.vega, fd1(|v| p(&with_vol(i, v)), i.vol, 1e-5), 1e-4, 1e-7);

        // delta_forward = ∂V_fwd/∂F: difference the UNDISCOUNTED forward value
        // V_fwd = price·e^{r_d T} in the forward (bumped through spot via the
        // carry F = S·e^{(r_d−r_f)T}), then divide by the carry to convert
        // ∂/∂S → ∂/∂F. An in-crate FD gate for the forward delta (the parity-crate
        // FD check does not run under `cargo mutants -p celnet-vanilla`).
        let carry = celnet_core::math::exp((i.r_dom - i.r_for) * i.t);
        let edomt = celnet_core::math::exp(i.r_dom * i.t);
        let dvfwd_ds = fd1(|s| p(&with_spot(i, s)) * edomt, i.spot, hs);
        assert_close!(g.delta_forward, dvfwd_ds / carry, 1e-4, 1e-7);

        // theta = −∂V/∂T
        assert_close!(g.theta, -fd1(|t| p(&with_t(i, t)), i.t, 1e-5), 5e-4, 1e-6);
        assert_close!(
            g.rho_dom,
            fd1(|r| p(&with_rd(i, r)), i.r_dom, 1e-6),
            1e-4,
            1e-7
        );
        assert_close!(
            g.rho_for,
            fd1(|r| p(&with_rf(i, r)), i.r_for, 1e-6),
            1e-4,
            1e-7
        );

        // Second-order via differencing the relevant first-order Greek.
        let ds = |s: f64| greeks(opt, &with_spot(i, s)).delta_spot;
        let gam = |x: &VanillaInputs| greeks(opt, x).gamma;
        assert_close!(g.gamma, fd1(ds, i.spot, hs), 1e-3, 1e-6);
        assert_close!(
            g.vanna,
            fd1(|v| greeks(opt, &with_vol(i, v)).delta_spot, i.vol, 1e-5),
            1e-3,
            1e-6
        );
        assert_close!(
            g.volga,
            fd1(|v| greeks(opt, &with_vol(i, v)).vega, i.vol, 1e-5),
            1e-3,
            1e-6
        );
        assert_close!(
            g.charm,
            fd1(|t| greeks(opt, &with_t(i, t)).delta_spot, i.t, 1e-5),
            1e-3,
            1e-6
        );
        assert_close!(
            g.speed,
            fd1(|s| gam(&with_spot(i, s)), i.spot, hs),
            1e-2,
            1e-5
        );
        assert_close!(
            g.zomma,
            fd1(|v| gam(&with_vol(i, v)), i.vol, 1e-5),
            1e-2,
            1e-5
        );
        assert_close!(g.color, fd1(|t| gam(&with_t(i, t)), i.t, 1e-5), 1e-2, 1e-5);
    }

    /// `greeks(opt, i).price` MUST be the **bit-identical** value returned by
    /// `price(opt, i)`: the Greek pass recomputes the present value from the same
    /// shared `aux`/discount factors, and the contract is that this is exactly the
    /// standalone pricer's output — not merely close. This is a *reproducibility /
    /// bit-identity* check (the documented carve-out from the "no float `==`"
    /// rule, asserted via `to_bits()`), and it kills the mutation survivors where
    /// the greeks-pass price recomputation is perturbed (a wrong sign or dropped
    /// discount term in the `greeks` price branch leaves the FD Greek checks
    /// passing but breaks this exact tie).
    #[test]
    fn greeks_price_is_bit_identical_to_price() {
        let cases = [
            VanillaInputs::new(100.0, 100.0, 0.2, 1.0, 0.05, 0.0),
            VanillaInputs::new(1.10, 1.25, 0.09, 0.5, 0.02, 0.01),
            VanillaInputs::new(1.35, 1.20, 0.14, 2.0, 0.04, 0.015),
            VanillaInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.03),
            VanillaInputs::new(0.80, 0.95, 0.45, 3.0, -0.01, 0.06),
        ];
        for i in &cases {
            for opt in [OptionType::Call, OptionType::Put] {
                let standalone = price(opt, i);
                let from_greeks = greeks(opt, i).price;
                assert_eq!(
                    standalone.to_bits(),
                    from_greeks.to_bits(),
                    "greeks().price must be bit-identical to price(): {opt:?} {i:?} \
                     standalone={standalone} from_greeks={from_greeks}"
                );
            }
        }
    }

    #[test]
    fn greeks_vs_finite_difference() {
        // A spread of regimes: ITM/OTM, low/high vol, short/long, +/- carry.
        let cases = [
            VanillaInputs::new(100.0, 100.0, 0.2, 1.0, 0.05, 0.0),
            VanillaInputs::new(1.10, 1.25, 0.09, 0.5, 0.02, 0.01),
            VanillaInputs::new(1.35, 1.20, 0.14, 2.0, 0.04, 0.015),
            VanillaInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.03),
        ];
        for i in &cases {
            check_greeks(OptionType::Call, i);
            check_greeks(OptionType::Put, i);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn put_call_parity_property(
            s in 0.5f64..200.0,
            k in 0.5f64..200.0,
            vol in 0.02f64..0.8,
            t in 0.02f64..3.0,
            r_dom in -0.02f64..0.10,
            r_for in -0.02f64..0.10,
        ) {
            let i = VanillaInputs::new(s, k, vol, t, r_dom, r_for);
            let lhs = price(OptionType::Call, &i) - price(OptionType::Put, &i);
            let rhs = i.spot * i.df_for() - i.strike * i.df_dom();
            prop_assert!(celnet_core::is_close(lhs, rhs, 1e-9, 1e-9));
        }

        #[test]
        fn price_bounds(
            s in 0.5f64..200.0,
            k in 0.5f64..200.0,
            vol in 0.02f64..0.8,
            t in 0.02f64..3.0,
            r_dom in 0.0f64..0.10,
            r_for in 0.0f64..0.10,
        ) {
            let i = VanillaInputs::new(s, k, vol, t, r_dom, r_for);
            let c = price(OptionType::Call, &i);
            let pp = price(OptionType::Put, &i);
            // Non-negative and bounded by the discounted underlying / strike.
            prop_assert!(c >= -1e-9 && c <= s * i.df_for() + 1e-9);
            prop_assert!(pp >= -1e-9 && pp <= k * i.df_dom() + 1e-9);
        }
    }
}
