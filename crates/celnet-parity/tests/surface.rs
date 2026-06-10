//! Parity rows 6–9: the **auditable arbitrage-free surface** — the genuine
//! IPV/FRTB differentiator over Fenics's ML-fill (which gives "no guarantee")
//! and the closed/heuristic surfaces of OVML/SynOption
//! (`docs/CAPABILITIES-VS-COMPETITION.md` §"Volatility surface"). Celnet's claim
//! is three static/dynamic no-arbitrage laws plus a user-*selectable* smile
//! family (Vanna-Volga, SABR, SVI and SSVI), and we prove each as a **property
//! test** sweeping randomized arbitrage-free parameters, not a single happy-path
//! fixture:
//!
//!  6. butterfly density ≥ 0 (Breeden-Litzenberger risk-neutral density);
//!  7. vertical (forward call) monotone non-increasing in strike;
//!  8. total variance monotone non-decreasing in business time (calendar);
//!  9. VV and SSVI agree in the wings within tolerance, AND each of the four
//!     smile families (VV / SABR / SVI / SSVI) is independently selectable and
//!     usable through the unified surface — the cross-family consistency and the
//!     user choice that no incumbent exposes
//!     (`each_smile_family_is_selectable`).

use celnet_conventions::resolve;
use celnet_core::Smile;
use celnet_surface::{
    DeltaPillar, MarketContext, MarketQuotes, ParametricSlice, ParametricSurface, SmileModel,
    StochasticVolParams, StochasticVolSmile, TenorPillar, VolSurface, build_smile, check_slice,
    implied_density,
};
use celnet_types::{CcyPair, OptionType, Tenor};
use proptest::prelude::*;

/// A 1Y EURUSD-style context for the broker→VV smile under test.
fn eurusd_1y() -> MarketContext {
    let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    MarketContext::new(
        1.10,
        celnet_types::Carry::FxRates {
            r_dom: 0.02,
            r_for: 0.01,
        },
        1.0,
        conv,
    )
}

/// A log-spaced strike grid straddling the forward for the static checks.
fn strike_grid(forward: f64) -> Vec<f64> {
    let mut g = Vec::new();
    let mut k = 0.6 * forward;
    while k <= 1.6 * forward {
        g.push(k);
        k += 0.02 * forward;
    }
    g
}

proptest! {
    // 256 randomized arbitrage-free SSVI surfaces — the auditor's "over a
    // domain, not a point" standard. Strategy ranges keep the Gatheral-Jacquier
    // sufficient butterfly/calendar conditions satisfiable; the test then proves
    // the *implied-density* and *call-monotonicity* laws hold pointwise.
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Row 6 — butterfly density ≥ 0 across the strike grid for every sampled
    /// arbitrage-free SSVI slice. A negative density is a butterfly arbitrage;
    /// Fenics's ML fill carries no such guarantee.
    #[test]
    fn butterfly_density_nonnegative(
        rho in -0.7f64..0.7,
        eta in 0.2f64..1.5,
        gamma in 0.2f64..0.5,
        theta in 0.01f64..0.09,
    ) {
        let surf = ParametricSurface::new(rho, eta, gamma);
        prop_assume!(surf.is_butterfly_free(theta));
        let forward = 1.10;
        let t = 1.0;
        let slice = surf.to_slice(theta, forward, t);
        let grid = strike_grid(forward);
        let h = 0.004 * forward;
        for &k in &grid {
            let dens = implied_density(&slice, k, forward, t, h);
            prop_assert!(
                dens >= -1e-6,
                "negative implied density {dens} at K={k} (rho={rho}, eta={eta}, gamma={gamma}, theta={theta})"
            );
        }
    }

    /// Row 7 — the undiscounted forward call price is monotone non-increasing in
    /// strike (vertical-spread no-arbitrage) for every sampled slice. Surfaced by
    /// the consolidated [`check_slice`] report (`max_vertical_increase ≤ tol`).
    #[test]
    fn vertical_monotone_in_strike(
        rho in -0.7f64..0.7,
        eta in 0.2f64..1.5,
        gamma in 0.2f64..0.5,
        theta in 0.01f64..0.09,
    ) {
        let surf = ParametricSurface::new(rho, eta, gamma);
        prop_assume!(surf.is_butterfly_free(theta));
        let forward = 1.10;
        let t = 1.0;
        let slice = surf.to_slice(theta, forward, t);
        let grid = strike_grid(forward);
        let report = check_slice(&slice, &grid, forward, t, 0.004 * forward);
        prop_assert!(
            report.is_arbitrage_free(1e-6),
            "slice not arbitrage-free: {report:?} (rho={rho}, eta={eta}, gamma={gamma}, theta={theta})"
        );
    }

    /// Row 8 — total variance is monotone non-decreasing in business time
    /// (calendar no-arbitrage), proven **at a fixed strike `K`** across two
    /// maturity slices with **different forwards** — the real calendar-arbitrage
    /// condition (an undiscounted call at fixed strike is non-decreasing in
    /// maturity), not a fixed-log-moneyness check on a single surface.
    ///
    /// The previous formulation derived the long slice from the short one by
    /// adding `dθ` and compared at a *fixed log-moneyness*, which made the
    /// inequality `w₂ ≥ w₁` hold by construction (the SSVI total variance is
    /// monotone in `θ` at fixed `k`) — vacuous. The fix changes the comparison
    /// coordinate to a **fixed strike**:
    ///
    ///  * the calendar leg of SSVI's static no-arbitrage guarantee is that a
    ///    *single* surface (shared `ρ, η, γ`, power-law `φ`) with a
    ///    non-decreasing ATM total-variance term structure `θ(t)` is
    ///    calendar-arbitrage-free across **all** maturities (Gatheral-Jacquier
    ///    2014). We use the production [`ParametricSurface::is_calendar_free`]
    ///    gate to certify the `(θ₁, θ₂)` pair;
    ///  * but we evaluate at a fixed **strike** `K`, and the two maturities carry
    ///    *independent* forwards `F₁ ≠ F₂` (independent carry over `t₁ < t₂`), so
    ///    the same `K` maps to *different* log-moneyness `ln(K/F₁) ≠ ln(K/F₂)` on
    ///    the two slices. The inequality is therefore **not** automatic — a
    ///    surface whose certified calendar gate failed to imply fixed-strike
    ///    monotonicity would be caught here.
    #[test]
    fn calendar_total_variance_monotone(
        rho in -0.7f64..0.7,
        eta in 0.2f64..1.2,
        gamma in 0.3f64..0.5,
        atm_vol1 in 0.08f64..0.16,
        vol_ramp in 0.0f64..0.10,
        carry1 in -0.04f64..0.04,
        carry2 in -0.04f64..0.04,
    ) {
        // A single SSVI surface (shared shape) — the object whose calendar leg is
        // guaranteed arbitrage-free when θ(t) is non-decreasing.
        let surf = ParametricSurface::new(rho, eta, gamma);

        // Two maturities with INDEPENDENT forwards (independent carry).
        let (t1, t2) = (0.5_f64, 1.5_f64);
        let spot = 1.10_f64;
        let f1 = spot * (carry1 * t1).exp();
        let f2 = spot * (carry2 * t2).exp();

        // A non-decreasing ATM total-variance term structure θ(t) = σ(t)²·t.
        let atm_vol2 = atm_vol1 + vol_ramp;
        let theta1 = atm_vol1 * atm_vol1 * t1;
        let theta2 = atm_vol2 * atm_vol2 * t2;

        prop_assume!(surf.is_butterfly_free(theta1) && surf.is_butterfly_free(theta2));
        // Production calendar gate must certify the (θ₁, θ₂) ordering.
        prop_assert!(
            surf.is_calendar_free(theta1, theta2),
            "calendar gate must certify θ1={theta1} ≤ θ2={theta2}"
        );

        // Fixed-STRIKE grid straddling both (different) forwards. Each maturity
        // maps the strike to its own log-moneyness via its own forward.
        let grid = strike_grid(0.5 * (f1 + f2));
        for &k in &grid {
            let w1 = surf.total_variance((k / f1).ln(), theta1); // at ln(K/F₁)
            let w2 = surf.total_variance((k / f2).ln(), theta2); // at ln(K/F₂)
            prop_assert!(
                w2 >= w1 - 1e-9,
                "calendar arbitrage at FIXED strike K={k}: \
                 w(t1)={w1} > w(t2)={w2} (θ1={theta1} θ2={theta2}, F1={f1} F2={f2})"
            );
        }
    }
}

/// Row 9 — VV and SSVI agree in the wings within tolerance. We build a
/// Vanna-Volga smile from broker quotes and fit an SSVI slice anchored to the
/// same ATM total variance, then require the two implied vols to agree at the
/// 25Δ and 10Δ wing strikes to within a desk-meaningful tolerance. This is the
/// cross-family consistency that justifies letting the user *choose* the
/// parameterization — a choice no incumbent exposes.
#[test]
fn vannavolga_and_ssvi_agree_in_wings() {
    let ctx = eurusd_1y();
    let forward = ctx.forward();
    let t = ctx.t;
    // A realistic five-point EURUSD slice (ATM 10.5, mild RR, small BF).
    let quotes = MarketQuotes::five_point(0.105, 0.010, 0.0030, 0.018, 0.0090);
    let vv = build_smile(&ctx, &quotes).expect("VV smile builds");

    let atm_vv = vv.implied_vol(forward, forward, t).0;
    let theta = atm_vv * atm_vv * t;

    // Fit SSVI curvature/skew to the VV smile at the two wings (a light,
    // deterministic 2-D grid search; OSS only, no solver dependency). The aim is
    // a cross-family *agreement* check, so the fit just minimises the summed
    // squared wing-vol error over a coarse parameter grid.
    let kc25 = ctx
        .strike_at_delta(OptionType::Call, DeltaPillar::TWENTY_FIVE, atm_vv)
        .unwrap();
    let kp25 = ctx
        .strike_at_delta(OptionType::Put, DeltaPillar::TWENTY_FIVE, atm_vv)
        .unwrap();
    let kc10 = ctx
        .strike_at_delta(OptionType::Call, DeltaPillar::TEN, atm_vv)
        .unwrap();
    let kp10 = ctx
        .strike_at_delta(OptionType::Put, DeltaPillar::TEN, atm_vv)
        .unwrap();
    let wings = [kp10, kp25, kc25, kc10];
    let vv_wing: Vec<f64> = wings
        .iter()
        .map(|&k| vv.implied_vol(k, forward, t).0)
        .collect();

    let mut best = (f64::INFINITY, ParametricSurface::new(0.0, 0.5, 0.4));
    // Coarse arbitrage-free grid.
    let mut rho = -0.6;
    while rho <= 0.6 {
        let mut eta = 0.2;
        while eta <= 1.4 {
            let mut gamma = 0.25;
            while gamma <= 0.5 {
                let surf = ParametricSurface::new(rho, eta, gamma);
                if surf.is_butterfly_free(theta) {
                    let slice = surf.to_slice(theta, forward, t);
                    let err: f64 = wings
                        .iter()
                        .zip(&vv_wing)
                        .map(|(&k, &target)| {
                            let v = slice.implied_vol(k, forward, t).0;
                            (v - target) * (v - target)
                        })
                        .sum();
                    if err < best.0 {
                        best = (err, surf);
                    }
                }
                gamma += 0.05;
            }
            eta += 0.1;
        }
        rho += 0.1;
    }

    let slice = best.1.to_slice(theta, forward, t);
    // Agreement to within 60 bps of vol at every wing — comfortably inside the
    // bid/ask of an OTM EURUSD wing, proving the two families price the same
    // economics. (VV is a local 3-point construction, SSVI a global 3-parameter
    // form, so exact agreement is not expected; *desk-meaningful* agreement is
    // the claim.)
    for (&k, &target) in wings.iter().zip(&vv_wing) {
        let ssvi = slice.implied_vol(k, forward, t).0;
        assert!(
            (ssvi - target).abs() <= 0.006,
            "VV vs SSVI wing disagreement at K={k}: VV={target}, SSVI={ssvi} \
             (|diff|={})",
            (ssvi - target).abs()
        );
    }
}

/// Row 9 (selectability) — **each** of the four smile families the platform
/// exposes can be *selected* and evaluated through the unified [`VolSurface`]:
/// market-hedge (vanna-volga), stochastic-vol (SABR), parametric slice (SVI raw)
/// and the parametric surface (SSVI). The capability claim is "user-selectable
/// VV/SABR/SVI/SSVI"; the wing-agreement test above only exercises VV-vs-SSVI,
/// so this test additionally exercises **SABR and SVI** end-to-end. For each
/// family we build a slice, wrap it in a `VolSurface` tagged with the matching
/// [`SmileModel`], and require the surface to (a) report the selected model and
/// (b) produce finite, positive Black vols across a strike grid straddling the
/// forward — i.e. the selection actually drives a working, re-strikable surface,
/// not merely an enum tag. No incumbent exposes the choice of family at all.
#[test]
fn each_smile_family_is_selectable() {
    let ctx = eurusd_1y();
    let forward = ctx.forward();
    let t = ctx.t;
    let grid = strike_grid(forward);

    // Sanity helper: a surface must report its model and price a positive,
    // finite, plausibly-bounded vol at every strike on the grid.
    fn assert_usable<S: Smile + Clone>(
        surf: &VolSurface<S>,
        expect: SmileModel,
        grid: &[f64],
        t: f64,
    ) {
        assert_eq!(
            surf.model(),
            expect,
            "surface must report the selected family"
        );
        for &k in grid {
            let v = surf.implied_vol(k, t);
            assert!(
                v.is_finite() && v > 0.0 && v < 5.0,
                "{expect:?} produced an implausible vol {v} at K={k}"
            );
        }
    }

    // 1. Market-hedge (vanna-volga): the broker-calibrated baseline smile.
    let quotes = MarketQuotes::five_point(0.105, 0.010, 0.0030, 0.018, 0.0090);
    let vv = build_smile(&ctx, &quotes).expect("VV smile builds");
    let vv_surface = VolSurface::new(
        SmileModel::MarketHedge,
        vec![TenorPillar::new(vv, forward, t)],
    );
    assert_usable(&vv_surface, SmileModel::MarketHedge, &grid, t);

    // 2. Stochastic-vol (SABR): a calibrated four-parameter slice with the
    //    arbitrage-free wing density.
    let sabr_params = StochasticVolParams::new(0.105, 1.0, -0.15, 0.55, forward, t);
    let sabr = StochasticVolSmile::new(sabr_params);
    let sabr_surface = VolSurface::new(
        SmileModel::StochasticVol,
        vec![TenorPillar::new(sabr, forward, t)],
    );
    assert_usable(&sabr_surface, SmileModel::StochasticVol, &grid, t);

    // 3. Parametric slice (SVI raw): the raw total-variance parameterization,
    //    materialised here directly so the SVI family is exercised on its own
    //    (not only via the SSVI→raw map). Parameters chosen butterfly-free.
    let svi = ParametricSlice::new(0.010, 0.040, -0.20, 0.0, 0.20, forward, t);
    let svi_surface = VolSurface::new(
        SmileModel::Parametric,
        vec![TenorPillar::new(svi, forward, t)],
    );
    assert_usable(&svi_surface, SmileModel::Parametric, &grid, t);

    // 4. Parametric surface (SSVI): the closed-form arbitrage-free surface form.
    let theta = 0.105 * 0.105 * t;
    let ssvi_slice = ParametricSurface::new(-0.20, 0.8, 0.4).to_slice(theta, forward, t);
    let ssvi_surface = VolSurface::new(
        SmileModel::ParametricSurface,
        vec![TenorPillar::new(ssvi_slice, forward, t)],
    );
    assert_usable(&ssvi_surface, SmileModel::ParametricSurface, &grid, t);
}
