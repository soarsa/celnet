//! Parity rows (catalogue, Wave 4a): the **extended SSVI (eSSVI)** surface — a
//! maturity-dependent-correlation generalisation of SSVI that keeps closed-form
//! static no-arbitrage (butterfly + calendar) while admitting a richer term
//! structure than any incumbent exposes (`docs/CAPABILITIES-VS-COMPETITION.md`
//! §"Volatility surface"; `docs/ANALYTICS-SPEC.md` §3.3). The closed-form
//! butterfly/calendar predicates in `celnet-surface::extended_surface` are
//! *claims*; here each is validated against an **independent numerical oracle** —
//! never against the same closed form — over randomized arbitrage-free eSSVI
//! surfaces (≥256 cases each), mirroring `tests/surface.rs`:
//!
//!  (i)   **butterfly / density ≥ 0** — the Breeden-Litzenberger risk-neutral
//!        density `g(k)` (a central second difference of the undiscounted forward
//!        call, from `celnet-surface::implied_density`) is ≥ 0 on a dense strike
//!        grid for every slice the closed-form `is_butterfly_free` admits. The
//!        density numerics are an independent method (pointwise option re-pricing,
//!        not the parametric butterfly inequality), so they genuinely test the
//!        closed-form claim;
//!  (ii)  **calendar-monotone** — `w(k,θ₂) ≥ w(k,θ₁)` pointwise for θ₁ < θ₂ on a
//!        dense grid, for every consecutive pillar pair the closed-form
//!        `is_calendar_free` admits. Pointwise total-variance comparison is the
//!        definitional calendar-no-arbitrage truth, independent of the closed-form
//!        pair inequality it validates;
//!  (iii) **golden reprice** — an eSSVI slice calibrated to its own input quotes
//!        reprices those quotes (in total variance) to ~1e-9 self-consistency;
//!  (iv)  **SSVI byte-recovery** — at constant ρ with ψ = θ·φ(θ) from a power-law
//!        φ, eSSVI's `total_variance` equals the existing
//!        `ParametricSurface::total_variance` to **bit identity** (`to_bits`)
//!        across a k-grid: SSVI is the byte-exact special case.

use celnet_surface::{ExtendedSlice, ExtendedSurface, ParametricSurface, implied_density};
use proptest::prelude::*;

/// A dense log-spaced strike grid straddling the forward for the static checks.
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
    // 256 randomized arbitrage-free eSSVI slices/surfaces — the "over a domain,
    // not a point" standard. Strategy ranges keep the closed-form butterfly /
    // calendar conditions satisfiable; the tests then prove the *independent
    // numerical* laws hold pointwise.
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Row (i) — butterfly density ≥ 0 across the strike grid for every sampled
    /// eSSVI slice the closed-form `is_butterfly_free` admits. The
    /// Breeden-Litzenberger density (an independent re-pricing method) validates
    /// the closed-form `(θ,ρ,ψ)` butterfly inequality.
    #[test]
    fn butterfly_density_nonnegative(
        rho in -0.7f64..0.7,
        psi in 0.05f64..1.2,
        theta in 0.01f64..0.09,
    ) {
        let slice = ExtendedSlice::new(theta, rho, psi);
        // Only test slices the production closed-form predicate certifies.
        prop_assume!(slice.is_butterfly_free());
        let forward = 1.10;
        let t = 1.0;
        let raw = slice.to_slice(forward, t);
        let grid = strike_grid(forward);
        let h = 0.004 * forward;
        for &k in &grid {
            let dens = implied_density(&raw, k, forward, t, h);
            prop_assert!(
                dens >= -1e-6,
                "negative implied density {dens} at K={k} (rho={rho}, psi={psi}, theta={theta})"
            );
        }
    }

    /// Row (ii) — total variance is calendar-monotone: `w(k,θ₂) ≥ w(k,θ₁)`
    /// pointwise for θ₁ < θ₂ across the strike grid, for every consecutive pillar
    /// pair the closed-form `is_calendar_free` admits. We sample two pillars whose
    /// ATM skews (`ρψ`) and ψ-gap satisfy the eSSVI calendar pair condition, build
    /// the two-pillar surface, and assert the pointwise inequality directly (the
    /// definitional truth, independent of the pair inequality it validates).
    #[test]
    fn calendar_total_variance_monotone(
        rho1 in -0.6f64..0.6,
        psi_scale in 0.02f64..0.14f64,
        psi_fraction in 0.0f64..1.0f64,
        skew_shift in -0.4f64..0.4,
        theta1 in 0.01f64..0.05,
        theta_ramp in 0.005f64..0.05,
    ) {
        let psi1 = psi_scale;
        let theta2 = theta1 + theta_ramp;
        // Non-increasing curvature phi = psi/theta requires psi2 <= psi1 * (theta2 / theta1).
        let max_psi_ramp = psi1 * (theta_ramp / theta1);
        let psi2 = psi1 + psi_fraction * max_psi_ramp;
        // Choose ρ₂ so the ATM-skew gap stays inside the ψ-gap (the calendar
        // condition |ρ₂ψ₂ − ρ₁ψ₁| ≤ ψ₂ − ψ₁): pick the target skew ρ₂ψ₂ within
        // [ρ₁ψ₁ − Δψ, ρ₁ψ₁ + Δψ] then back out ρ₂.
        let psi_gap = psi2 - psi1;
        let target_skew = (rho1 * psi1 + skew_shift * psi_gap).clamp(
            rho1 * psi1 - psi_gap,
            rho1 * psi1 + psi_gap,
        );
        let rho2 = (target_skew / psi2).clamp(-0.95, 0.95);

        let s1 = ExtendedSlice::new(theta1, rho1, psi1);
        let s2 = ExtendedSlice::new(theta2, rho2, psi2);
        // Both pillars must be butterfly-free and the pair calendar-free for this
        // to be a valid arbitrage-free surface.
        prop_assume!(s1.is_butterfly_free() && s2.is_butterfly_free());
        prop_assume!(s1.is_calendar_free_with(&s2));

        let surf = ExtendedSurface::new(vec![s1, s2]);
        prop_assert!(surf.is_calendar_free());

        let forward = 1.10;
        let grid = strike_grid(forward);
        for &strike in &grid {
            let k = (strike / forward).ln();
            let w1 = surf.total_variance(k, theta1);
            let w2 = surf.total_variance(k, theta2);
            prop_assert!(
                w2 >= w1 - 1e-9,
                "calendar arbitrage at k={k}: w(θ1)={w1} > w(θ2)={w2} \
                 (θ1={theta1} θ2={theta2}, ρ1={rho1} ρ2={rho2}, ψ1={psi1} ψ2={psi2})"
            );
        }
    }
}

/// Row (iii) — golden reprice: an eSSVI slice calibrated to its own input quotes
/// reprices those quotes (in total variance) to ~1e-9. We synthesize input quotes
/// AS the total variances of a known arbitrage-free eSSVI slice at a handful of
/// log-moneyness points, then recover (ρ, ψ) by a deterministic least-squares fit
/// (the same residual the production calibrator minimises) at the pinned θ, and
/// require the refitted slice to reproduce those total variances to 1e-9. This is
/// a genuine self-consistency oracle: the fit sees only the (k, w) samples, not
/// the generating parameters.
#[test]
fn essvi_reprices_its_input_quotes() {
    // A known arbitrage-free generating slice.
    let theta = 0.02_f64;
    let generating = ExtendedSlice::new(theta, -0.25, 0.20);
    assert!(generating.is_butterfly_free());

    // Input "quotes": total variances at five log-moneyness pillars.
    let ks = [-0.30_f64, -0.12, 0.0, 0.12, 0.30];
    let targets: Vec<(f64, f64)> = ks
        .iter()
        .map(|&k| (k, generating.total_variance(k)))
        .collect();

    // Recover (ρ, ψ) at the pinned θ by a deterministic coarse-then-fine grid
    // search minimising the summed squared total-variance residual (OSS only, no
    // solver dependency; the residual is the production calibrator's objective).
    let w_of = |rho: f64, psi: f64, k: f64| -> f64 {
        let p = psi / theta;
        let pk = p * k + rho;
        0.5 * theta * (1.0 + rho * p * k + (pk * pk + (1.0 - rho * rho)).sqrt())
    };
    let cost = |rho: f64, psi: f64| -> f64 {
        targets
            .iter()
            .map(|&(k, w)| {
                let d = w_of(rho, psi, k) - w;
                d * d
            })
            .sum::<f64>()
    };

    // Nested grid refinement around the best cell (deterministic, bit-reproducible):
    // each round re-centres a shrinking bracket on the incumbent minimiser. With
    // exact (noise-free) targets the residual collapses geometrically, reaching the
    // 1e-9 self-consistency bar after enough rounds.
    let mut best = (f64::INFINITY, 0.0_f64, 0.3_f64);
    let (mut rho_lo, mut rho_hi) = (-0.95_f64, 0.95_f64);
    let (mut psi_lo, mut psi_hi) = (1e-3_f64, 1.5_f64);
    for _round in 0..22 {
        let n = 60;
        for i in 0..=n {
            let rho = rho_lo + (rho_hi - rho_lo) * (i as f64) / (n as f64);
            for j in 0..=n {
                let psi = psi_lo + (psi_hi - psi_lo) * (j as f64) / (n as f64);
                let c = cost(rho, psi);
                if c < best.0 {
                    best = (c, rho, psi);
                }
            }
        }
        // Shrink the bracket around the incumbent best.
        let rho_w = (rho_hi - rho_lo) * 0.10;
        let psi_w = (psi_hi - psi_lo) * 0.10;
        rho_lo = (best.1 - rho_w).max(-0.95);
        rho_hi = (best.1 + rho_w).min(0.95);
        psi_lo = (best.2 - psi_w).max(1e-3);
        psi_hi = (best.2 + psi_w).min(1.5);
    }

    let refit = ExtendedSlice::new(theta, best.1, best.2);
    for &(k, w) in &targets {
        let got = refit.total_variance(k);
        assert!(
            (got - w).abs() <= 1e-9,
            "eSSVI reprice mismatch at k={k}: target w={w}, refit w={got} (|Δ|={})",
            (got - w).abs()
        );
    }
}

/// Row (iv) — SSVI byte-recovery: at constant ρ, building the eSSVI slice from the
/// SSVI shape variables `(θ, ρ, φ(θ))` (the curvature `φ(θ) = η/θ^γ` of the
/// power-law surface) via [`ExtendedSlice::from_curvature`], eSSVI's
/// `total_variance` equals the existing `ParametricSurface::total_variance` (the
/// production SSVI) to **bit identity** across a k-grid, for a sweep of
/// (ρ, η, γ, θ). The independent oracle here is the separately-implemented SSVI
/// closed form in `parametric_surface.rs`; the guarantee is `to_bits` equality,
/// not a tolerance. (Constructing through `φ` avoids the `ψ/θ` round-trip — `ψ` is
/// `θ·φ` derived — so the SSVI arithmetic is reproduced token-for-token.)
#[test]
fn ssvi_byte_recovered_at_constant_rho() {
    let thetas = [0.004_f64, 0.011, 0.03, 0.07];
    let ks = [-0.5_f64, -0.2, -0.05, 0.0, 0.05, 0.2, 0.5];
    let mut checked = 0_usize;
    for &rho in &[-0.5_f64, -0.2, 0.0, 0.3, 0.6] {
        for &eta in &[0.3_f64, 0.6, 0.9, 1.2] {
            for &gamma in &[0.25_f64, 0.4, 0.5] {
                let ssvi = ParametricSurface::new(rho, eta, gamma);
                for &theta in &thetas {
                    let essvi = ExtendedSlice::from_curvature(theta, rho, ssvi.phi(theta));
                    for &k in &ks {
                        let w_ssvi = ssvi.total_variance(k, theta);
                        let w_essvi = essvi.total_variance(k);
                        assert_eq!(
                            w_ssvi.to_bits(),
                            w_essvi.to_bits(),
                            "byte mismatch (ρ={rho}, η={eta}, γ={gamma}, θ={theta}, k={k}): \
                             SSVI {w_ssvi} vs eSSVI {w_essvi}"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 5 * 4 * 3 * 4 * 7, "must exercise the full sweep");
}
