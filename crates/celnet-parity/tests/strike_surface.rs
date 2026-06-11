//! Parity row (CRYPTO-SURFACE-LEAF-SPEC §5.4): the **strike-axis smile leaf** —
//! exchange-chain (strike-gridded) quote calibration onto the asset-class-
//! neutral total-variance core, the quote basis digital-asset venues actually
//! publish (incumbent FX-options stacks ingest only delta-pillar broker
//! quotes). House standard: verified **over a domain, not a point** — random
//! arbitrage-free raw-SVI ground truths, the fitter sees only `(K_i, σ_i)`.
//!
//! Per case: an 11-point strike grid `k ∈ ±0.6` generated from an **in-test
//! re-typed** reference formula (never `ParametricSlice::total_variance`) →
//! `fit_strike_slice` → the fit must (i) reproduce the quotes to ≤ 1e-4
//! absolute vol (0.01 vol point — the sanctioned floor, never loosened),
//! (ii) be butterfly-free in both the closed-form Durrleman and the
//! **independent** Breeden–Litzenberger finite-difference notions, and
//! (iii) respect the dimensionless Lee wing bound.

use celnet_core::math::norm_cdf;
use celnet_surface::{StrikeQuote, StrikeQuoteSlice, StrikeSliceContext, fit_strike_slice};
use celnet_types::Carry;
use proptest::prelude::*;

/// Raw-SVI total variance, re-typed from the published equation (Gatheral
/// 2004; Gatheral & Jacquier 2014 eq. 3.1) — independent of production code.
fn svi_w_reference(k: f64, a: f64, b: f64, rho: f64, m: f64, sigma: f64) -> f64 {
    a + b * (rho * (k - m) + ((k - m) * (k - m) + sigma * sigma).sqrt())
}

/// Undiscounted Black forward call, re-typed from the published formula — the
/// independent density oracle.
fn black_call(forward: f64, strike: f64, vol: f64, t: f64) -> f64 {
    let st = vol * t.sqrt();
    let d1 = ((forward / strike).ln() + 0.5 * st * st) / st;
    let d2 = d1 - st;
    forward * norm_cdf(d1) - strike * norm_cdf(d2)
}

proptest! {
    // 256 randomized arbitrage-free truths — "over a domain, not a point".
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Domain: the spec §5.4 ranges, filtered to the arbitrage-free subset
    /// (Lee bound, positive variance floor, and a strictly positive Durrleman
    /// density-factor scan — the b/ρ ranges alone do not exclude every
    /// butterfly-violating corner, so the truth itself is certified arb-free
    /// before asking the fitter to reproduce it; a sanctioned domain pin, the
    /// reproduction tolerance is NEVER loosened below 1e-4).
    #[test]
    fn strike_fit_reproduces_random_arbitrage_free_truths(
        a in 1e-4..0.05_f64,
        b in 0.01..0.5_f64,
        rho in -0.8..0.8_f64,
        m in -0.2..0.2_f64,
        sigma in 0.05..0.5_f64,
        forward in 100.0..1e5_f64,
        t in 0.02..1.0_f64,
    ) {
        // Arbitrage-free-domain pins on the truth.
        prop_assume!(b * (1.0 + rho.abs()) <= 2.0);
        prop_assume!(a + b * sigma * (1.0 - rho * rho).sqrt() > 0.0);
        let truth_g_min = celnet_surface::ParametricSlice::new(a, b, rho, m, sigma, forward, t)
            .min_butterfly_density_factor(2.0, 4096);
        prop_assume!(truth_g_min >= 1e-6);

        // Spot chosen so the carry-seam forward is exactly `forward`
        // (zero net carry: e⁰ = 1 exactly).
        let ctx = StrikeSliceContext::new(forward, Carry::CostOfCarry { r: 0.0, b: 0.0 }, t);

        // The 11-point grid the fitter sees: strikes + vols only.
        let quotes = StrikeQuoteSlice::new(
            (0..11)
                .map(|i| {
                    let k = -0.6 + 1.2 * f64::from(i) / 10.0;
                    let w = svi_w_reference(k, a, b, rho, m, sigma);
                    StrikeQuote { strike: forward * k.exp(), vol: (w / t).sqrt() }
                })
                .collect(),
        );

        let fit = fit_strike_slice(&ctx, &quotes).unwrap();

        // (i) Reproduction: ≤ 0.01 vol point on-grid.
        prop_assert!(
            fit.max_vol_error <= 1e-4,
            "max vol error {} for truth (a={a}, b={b}, rho={rho}, m={m}, sigma={sigma}, t={t})",
            fit.max_vol_error
        );

        // (iii) The dimensionless Lee wing bound.
        prop_assert!(fit.slice.satisfies_wing_bound());

        // (ii-a) Closed-form Durrleman butterfly-freeness.
        prop_assert!(
            fit.slice.is_butterfly_free(0.6, 1e-6),
            "min g = {}",
            fit.slice.min_butterfly_density_factor(0.6, 4096)
        );

        // (ii-b) Independent Breeden–Litzenberger density across the grid span.
        let h = 1e-3 * forward;
        for j in 0..=40 {
            let k = -0.6 + 1.2 * f64::from(j) / 40.0;
            let strike = forward * k.exp();
            let c = |kk: f64| black_call(forward, kk, fit.slice.vol_at(kk), t);
            let density = (c(strike - h) - 2.0 * c(strike) + c(strike + h)) / (h * h);
            prop_assert!(
                density >= -1e-6,
                "FD density {density} < 0 at k={k} for truth (a={a}, b={b}, rho={rho}, m={m}, sigma={sigma}, t={t})"
            );
        }
    }
}
