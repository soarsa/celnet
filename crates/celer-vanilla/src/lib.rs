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

#![forbid(unsafe_code)]

use celer_core::math::{ln, norm_cdf, norm_pdf, sqrt};
use celer_types::{GkInputs, Greeks, OptionType};

/// Intermediate quantities shared by price and Greeks.
struct Aux {
    d1: f64,
    d2: f64,
    sqt: f64,
    vsqt: f64,
}

#[inline]
fn aux(i: &GkInputs) -> Aux {
    let sqt = sqrt(i.t);
    let vsqt = i.vol * sqt;
    let d1 = (ln(i.spot / i.strike) + (i.r_dom - i.r_for + 0.5 * i.vol * i.vol) * i.t) / vsqt;
    let d2 = d1 - vsqt;
    Aux { d1, d2, sqt, vsqt }
}

/// Present value (domestic premium per 1 unit of base notional).
#[must_use]
pub fn price(opt: OptionType, i: &GkInputs) -> f64 {
    let a = aux(i);
    let s_disc = i.spot * i.df_for();
    let k_disc = i.strike * i.df_dom();
    match opt {
        OptionType::Call => s_disc * norm_cdf(a.d1) - k_disc * norm_cdf(a.d2),
        OptionType::Put => k_disc * norm_cdf(-a.d2) - s_disc * norm_cdf(-a.d1),
    }
}

/// Price and the full Greek set in a single pass.
///
/// See [`Greeks`] for the precise definition and units of each sensitivity.
#[must_use]
#[allow(clippy::similar_names)] // d1/d2, nd1/nd2 are the canonical option-pricing names
pub fn greeks(opt: OptionType, i: &GkInputs) -> Greeks {
    let a = aux(i);
    let (d1, d2, sqt, vsqt) = (a.d1, a.d2, a.sqt, a.vsqt);
    let (s, k, t, vol) = (i.spot, i.strike, i.t, i.vol);
    let df_dom = i.df_dom();
    let df_for = i.df_for();
    let b = i.r_dom - i.r_for; // cost of carry

    let pd1 = norm_pdf(d1);
    let nd1 = norm_cdf(d1);
    let nd2 = norm_cdf(d2);
    let nmd1 = norm_cdf(-d1);
    let nmd2 = norm_cdf(-d2);

    let s_disc = s * df_for;
    let k_disc = k * df_dom;

    let price = match opt {
        OptionType::Call => s_disc * nd1 - k_disc * nd2,
        OptionType::Put => k_disc * nmd2 - s_disc * nmd1,
    };

    let delta_spot = match opt {
        OptionType::Call => df_for * nd1,
        OptionType::Put => df_for * (nd1 - 1.0),
    };
    let delta_forward = match opt {
        OptionType::Call => nd1,
        OptionType::Put => nd1 - 1.0,
    };

    // Symmetric across call/put.
    let gamma = df_for * pd1 / (s * vsqt);
    let vega = s_disc * sqt * pd1;
    let vanna = -df_for * pd1 * d2 / vol;
    let volga = vega * d1 * d2 / vol;
    let speed = -gamma / s * (d1 / vsqt + 1.0);
    let zomma = gamma * (d1 * d2 - 1.0) / vol;

    // theta = ∂V/∂t (per year) = −∂V/∂T.
    let theta_common = -(s_disc * pd1 * vol) / (2.0 * sqt);
    let theta = match opt {
        OptionType::Call => theta_common + i.r_for * s_disc * nd1 - i.r_dom * k_disc * nd2,
        OptionType::Put => theta_common - i.r_for * s_disc * nmd1 + i.r_dom * k_disc * nmd2,
    };

    let rho_dom = match opt {
        OptionType::Call => k * t * df_dom * nd2,
        OptionType::Put => -k * t * df_dom * nmd2,
    };
    let rho_for = match opt {
        OptionType::Call => -s * t * df_for * nd1,
        OptionType::Put => s * t * df_for * nmd1,
    };

    // charm = ∂(delta_spot)/∂T. With delta_spot = e^{−r_f T}·Φ(±d1):
    //   ∂Δ/∂T = −r_f·e^{−r_f T}·Φ(d1) + e^{−r_f T}·φ(d1)·∂d1/∂T,
    //   ∂d1/∂T = b/(σ√T) − d1/(2T) + … collapses to (b/vsqt − d2/(2T))/… ; we use the
    //   standard generalized-BSM closed form and validate against finite differences.
    // ∂d1/∂T = (b + ½σ²)/(2σ√T) − ln(S/K)/(2σ·T^{3/2}); equivalently:
    let dd1_dt = b / vsqt - d1 / (2.0 * t) + 0.5 * vol / sqt;
    let charm = match opt {
        OptionType::Call => -i.r_for * df_for * nd1 + df_for * pd1 * dd1_dt,
        OptionType::Put => i.r_for * df_for * nmd1 + df_for * pd1 * dd1_dt,
    };

    // color = ∂gamma/∂T. gamma = e^{−r_f T}·φ(d1)/(S·σ·√T); differentiate w.r.t. T.
    //   = gamma·[ −r_f − 1/(2T) − d1·∂d1/∂T ].
    let color = gamma * (-i.r_for - 1.0 / (2.0 * t) - d1 * dd1_dt);

    Greeks {
        price,
        delta_spot,
        delta_forward,
        gamma,
        vega,
        theta,
        rho_dom,
        rho_for,
        vanna,
        volga,
        charm,
        speed,
        zomma,
        color,
    }
}

#[cfg(test)]
mod tests {
    use celer_core::assert_close;
    use proptest::prelude::*;

    use super::*;

    /// Black-Scholes textbook benchmark: S=K=100, T=1, r=5%, q=0, σ=20%.
    /// Reference values are the standard analytic results.
    #[test]
    fn black_scholes_reference() {
        let i = GkInputs::new(100.0, 100.0, 0.2, 1.0, 0.05, 0.0);
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
        let i = GkInputs::new(1.2345, 1.30, 0.11, 0.75, 0.03, 0.01);
        let lhs = price(OptionType::Call, &i) - price(OptionType::Put, &i);
        let rhs = i.spot * i.df_for() - i.strike * i.df_dom();
        assert_close!(lhs, rhs, 1e-12, 1e-12);
    }

    // ---- finite-difference Greek oracle ----

    fn fd1<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
        (f(x + h) - f(x - h)) / (2.0 * h)
    }

    fn with_spot(i: &GkInputs, s: f64) -> GkInputs {
        GkInputs { spot: s, ..*i }
    }
    fn with_vol(i: &GkInputs, v: f64) -> GkInputs {
        GkInputs { vol: v, ..*i }
    }
    fn with_t(i: &GkInputs, t: f64) -> GkInputs {
        GkInputs { t, ..*i }
    }
    fn with_rd(i: &GkInputs, r: f64) -> GkInputs {
        GkInputs { r_dom: r, ..*i }
    }
    fn with_rf(i: &GkInputs, r: f64) -> GkInputs {
        GkInputs { r_for: r, ..*i }
    }

    fn check_greeks(opt: OptionType, i: &GkInputs) {
        let g = greeks(opt, i);
        let p = |x: &GkInputs| price(opt, x);

        // First-order.
        let hs = 1e-4 * i.spot;
        assert_close!(
            g.delta_spot,
            fd1(|s| p(&with_spot(i, s)), i.spot, hs),
            1e-4,
            1e-7
        );
        assert_close!(g.vega, fd1(|v| p(&with_vol(i, v)), i.vol, 1e-5), 1e-4, 1e-7);
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
        let gam = |x: &GkInputs| greeks(opt, x).gamma;
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

    #[test]
    fn greeks_vs_finite_difference() {
        // A spread of regimes: ITM/OTM, low/high vol, short/long, +/- carry.
        let cases = [
            GkInputs::new(100.0, 100.0, 0.2, 1.0, 0.05, 0.0),
            GkInputs::new(1.10, 1.25, 0.09, 0.5, 0.02, 0.01),
            GkInputs::new(1.35, 1.20, 0.14, 2.0, 0.04, 0.015),
            GkInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.03),
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
            let i = GkInputs::new(s, k, vol, t, r_dom, r_for);
            let lhs = price(OptionType::Call, &i) - price(OptionType::Put, &i);
            let rhs = i.spot * i.df_for() - i.strike * i.df_dom();
            prop_assert!(celer_core::is_close(lhs, rhs, 1e-9, 1e-9));
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
            let i = GkInputs::new(s, k, vol, t, r_dom, r_for);
            let c = price(OptionType::Call, &i);
            let pp = price(OptionType::Put, &i);
            // Non-negative and bounded by the discounted underlying / strike.
            prop_assert!(c >= -1e-9 && c <= s * i.df_for() + 1e-9);
            prop_assert!(pp >= -1e-9 && pp <= k * i.df_dom() + 1e-9);
        }
    }
}
