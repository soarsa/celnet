//! Parity row: **variance & volatility swaps reproduce independent oracles**.
//!
//! Wave 4a catalogue increment. The production pricers in `celnet-exotics`
//! ([`fair_variance`] / [`fair_volatility`]) implement, respectively, the
//! log-contract static replication of the variance-swap fair strike and the
//! Carr–Lee convexity adjustment for the volatility-swap fair strike. This file
//! gates them against **genuinely independent** oracles — no closed form is
//! checked against itself:
//!
//!  (i)   **var-swap == independent adaptive quadrature (~1e-6).** The production
//!        strip integrates the `2/K²`-weighted OTM-option strip on a *log-spaced*
//!        (`u = ln(K/F)`) Simpson grid. The oracle here integrates the **same**
//!        `2/(T·K²)·Õ(K)` integrand by a completely different scheme — recursive
//!        **adaptive Simpson** in *strike space* with a tight per-interval error
//!        tolerance — so agreement to ~1e-6 cross-validates the strip against a
//!        different discretisation/coordinate/refinement strategy.
//!
//!  (ii)  **flat-smile closed-form oracle (~1e-6).** For a *flat* implied vol `σ`
//!        the model-free fair variance is exactly `σ²` — a genuinely independent
//!        closed form (it does not pass through any quadrature), which catches a
//!        forward/discount/scaling/sign error that a quadrature-vs-quadrature
//!        check could share. Verified across several `σ` and several rate sets.
//!
//!  (iii) **vol-swap strictly below √(var-swap), widening with convexity.** By
//!        Jensen/Carr–Lee the volatility-swap fair strike `K_vol` is strictly
//!        below `√K_var` for any non-degenerate smile, and the gap grows with
//!        smile convexity (butterfly). Asserted across a sweep of smiles.
//!
//! The oracle integrand (the undiscounted OTM forward value priced off the smile
//! at vol `σ(K)`) is built here in the test from `celnet-vanilla` + the `Smile`
//! trait — re-deriving the integrand independently of the exotics module so the
//! quadrature is not the production code under another name.

use celnet_core::Smile;
use celnet_exotics::{
    VarSwapContext, VarSwapStrip, fair_variance, fair_variance_with, fair_volatility,
};
use celnet_surface::MarketHedgeSmile;
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::price as vanilla_price;
use proptest::prelude::*;

/// A flat smile re-implemented locally as a test oracle (independent of any
/// crate's `FlatSmile`) so the closed-form `σ²` leg cannot accidentally couple
/// to library state.
#[derive(Clone, Copy)]
struct LocalFlat(f64);
impl Smile for LocalFlat {
    fn implied_vol(&self, _k: f64, _f: f64, _t: f64) -> celnet_types::Vol {
        celnet_types::Vol(self.0)
    }
}

/// Undiscounted (forward-measure) OTM option value at strike `K`, priced off the
/// smile vol `σ(K)`. Re-derived here independently of `celnet-exotics`: the same
/// economic object (`E^Q[(K−S_T)^+]` / `E^Q[(S_T−K)^+]`) the production strip
/// integrates, built straight from `celnet-vanilla` + the smile.
fn otm_forward<S: Smile>(smile: &S, ctx: &VarSwapContext, k: f64) -> f64 {
    let opt = if k < ctx.forward {
        OptionType::Put
    } else {
        OptionType::Call
    };
    let sigma = smile.implied_vol(k, ctx.forward, ctx.t).0;
    let spot = ctx.forward * (-(ctx.r_dom - ctx.r_for) * ctx.t).exp();
    let inputs = VanillaInputs {
        spot,
        strike: k,
        vol: sigma,
        t: ctx.t,
        r_dom: ctx.r_dom,
        r_for: ctx.r_for,
    };
    let pv = vanilla_price(opt, &inputs);
    pv * (ctx.r_dom * ctx.t).exp()
}

/// The fair-variance integrand in **strike space**: `2/(T·K²)·Õ(K)`. Integrating
/// this over `(0, ∞)` (truncated to the OTM wings) gives `K_var` directly — the
/// same integrand the production strip evaluates, here a function of `K` (not of
/// `u = ln K/F`), so the oracle quadrature works in a different coordinate.
fn integrand<S: Smile>(smile: &S, ctx: &VarSwapContext, k: f64) -> f64 {
    2.0 / (ctx.t * k * k) * otm_forward(smile, ctx, k)
}

/// One plain (3-point) Simpson estimate over `[a, b]`.
fn simpson<S: Smile>(smile: &S, ctx: &VarSwapContext, a: f64, b: f64, fa: f64, fb: f64) -> f64 {
    let m = 0.5 * (a + b);
    let fm = integrand(smile, ctx, m);
    (b - a) / 6.0 * (fa + 4.0 * fm + fb)
}

/// Recursive **adaptive Simpson** with Richardson error control — a genuinely
/// different quadrature scheme from the production fixed-grid log-space Simpson
/// strip. Subdivides until the Richardson estimate `|S_left+S_right−S|/15 < tol`.
#[allow(clippy::too_many_arguments)]
fn adaptive<S: Smile>(
    smile: &S,
    ctx: &VarSwapContext,
    a: f64,
    b: f64,
    fa: f64,
    fb: f64,
    whole: f64,
    tol: f64,
    depth: u32,
) -> f64 {
    let m = 0.5 * (a + b);
    let fm = integrand(smile, ctx, m);
    let left = simpson(smile, ctx, a, m, fa, fm);
    let right = simpson(smile, ctx, m, b, fm, fb);
    let delta = left + right - whole;
    if depth == 0 || delta.abs() <= 15.0 * tol {
        return left + right + delta / 15.0;
    }
    adaptive(smile, ctx, a, m, fa, fm, left, tol * 0.5, depth - 1)
        + adaptive(smile, ctx, m, b, fm, fb, right, tol * 0.5, depth - 1)
}

/// Independent oracle for the fair variance: adaptive-Simpson quadrature of the
/// strike-space integrand over the OTM wings (truncated wide in σ√T units, the
/// same place the integrand is numerically zero).
fn oracle_fair_variance<S: Smile>(smile: &S, ctx: &VarSwapContext) -> f64 {
    let atm = smile.implied_vol(ctx.forward, ctx.forward, ctx.t).0;
    // Truncate wide in σ√T units — well past where the 1/K²-weighted OTM
    // integrand is numerically zero even for a smile whose far wings rise under
    // extrapolation (the test smiles cap at ~0.35 σ√T, so 24·σ√T ≈ e^{8} of
    // moneyness is deep in the dead tail) — so the oracle is itself converged.
    let span = (24.0 * atm * ctx.t.sqrt()).exp();
    let k_lo = ctx.forward / span;
    let k_hi = ctx.forward * span;
    let f = ctx.forward;

    // Adaptive tolerance ~1e-9 on each wing — three orders tighter than the 1e-6
    // parity gate, so the oracle is the reference, not the bottleneck.
    let tol = 1e-9;
    // Put wing (K_lo → F) and call wing (F → K_hi) integrated separately; the
    // integrand is continuous through F (OTM put = OTM call there).
    let mut total = 0.0;
    for (a, b) in [(k_lo, f), (f, k_hi)] {
        let fa = integrand(smile, ctx, a);
        let fb = integrand(smile, ctx, b);
        let whole = simpson(smile, ctx, a, b, fa, fb);
        total += adaptive(smile, ctx, a, b, fa, fb, whole, tol, 50);
    }
    total
}

fn ctx(spot: f64, t: f64, r_dom: f64, r_for: f64) -> VarSwapContext {
    let i = VanillaInputs::new(spot, spot, 0.10, t, r_dom, r_for);
    VarSwapContext::from_inputs(&i)
}

// (i) Production strip == independent adaptive quadrature to ~1e-6.

#[test]
fn var_swap_matches_independent_adaptive_quadrature() {
    let cases = [
        (1.30, 1.0, 0.03, 0.01),
        (1.30, 0.5, 0.02, 0.04),
        (100.0, 2.0, 0.05, 0.00),
        (0.85, 0.25, 0.01, 0.03),
    ];
    // A convex, slightly skewed FX smile at each context.
    for (spot, t, r_dom, r_for) in cases {
        let c = ctx(spot, t, r_dom, r_for);
        let f = c.forward;
        let (kp, kc) = (f / 1.12, f * 1.12);
        let smile = MarketHedgeSmile::new([kp, f, kc], [0.122, 0.10, 0.118], f, c.t);

        let production = fair_variance(&smile, &c).fair_variance;
        let oracle = oracle_fair_variance(&smile, &c);
        let diff = (production - oracle).abs();
        assert!(
            diff < 1e-6,
            "strip {production} vs adaptive-quadrature oracle {oracle} (|Δ|={diff:e}) \
             at spot={spot} t={t}"
        );
    }
}

// (ii) Flat smile ⇒ K_var == σ² exactly (independent closed form).

#[test]
fn flat_smile_fair_variance_equals_sigma_squared() {
    for sigma in [0.04_f64, 0.08, 0.15, 0.25, 0.40] {
        for (spot, t, r_dom, r_for) in [
            (1.30, 1.0, 0.03, 0.01),
            (100.0, 0.5, 0.05, 0.02),
            (0.85, 2.0, 0.00, 0.04),
        ] {
            let c = ctx(spot, t, r_dom, r_for);
            let res = fair_variance(&LocalFlat(sigma), &c);
            let diff = (res.fair_variance - sigma * sigma).abs();
            assert!(
                diff < 1e-6,
                "flat σ={sigma}: K_var={} vs σ²={} (|Δ|={diff:e}) at spot={spot} t={t}",
                res.fair_variance,
                sigma * sigma
            );
        }
    }
}

// (iii) vol-swap strictly below √(var-swap), gap widening with convexity.

#[test]
fn vol_swap_strictly_below_sqrt_var_and_widens_with_convexity() {
    let c = ctx(1.30, 1.0, 0.03, 0.01);
    let f = c.forward;
    let (kp, kc) = (f / 1.10, f * 1.10);

    // (a) Strict K_vol < √K_var, positive gap, and the Var(v) Carr–Lee driver
    //     growing monotonically with the butterfly across the whole sweep.
    let mut last_vov = -1.0;
    for wing in [0.102_f64, 0.11, 0.125, 0.145, 0.17, 0.20] {
        let smile = MarketHedgeSmile::new([kp, f, kc], [wing, 0.10, wing], f, c.t);
        let var = fair_variance(&smile, &c);
        let vol = fair_volatility(&smile, &c);
        let sqrt_var = var.fair_vol();

        assert!(
            vol.fair_vol < sqrt_var,
            "K_vol {} must be strictly below √K_var {} (wing={wing})",
            vol.fair_vol,
            sqrt_var
        );
        assert!(
            vol.convexity_correction > 0.0,
            "non-degenerate smile must have a positive convexity gap (wing={wing})"
        );
        assert!(
            vol.variance_of_variance > last_vov,
            "Var(v) must grow with butterfly: {} !> {} (wing={wing})",
            vol.variance_of_variance,
            last_vov
        );
        last_vov = vol.variance_of_variance;
    }

    // (b) The convexity gap itself widens monotonically over the realistic FX
    //     butterfly regime (the gap = Var(v)/(8·K_var^{3/2}) eventually turns over
    //     as the K_var^{3/2} normalisation overtakes at extreme convexity, so the
    //     monotone-gap claim is scoped to the FX-quoted regime).
    let mut last_gap = -1.0;
    for wing in [0.102_f64, 0.108, 0.115, 0.125] {
        let smile = MarketHedgeSmile::new([kp, f, kc], [wing, 0.10, wing], f, c.t);
        let var = fair_variance(&smile, &c);
        let vol = fair_volatility(&smile, &c);
        let gap = var.fair_vol() - vol.fair_vol;
        assert!(
            gap > last_gap,
            "Jensen gap must widen with butterfly in the FX regime: {gap:e} !> {last_gap:e} (wing={wing})"
        );
        last_gap = gap;
    }
}

// Strip convergence: the production default is on the ~1e-9 plateau — refining
// the strip further (more nodes, wider wings) moves K_var by < 1e-7, proving the
// default is converged (a self-consistency check, distinct from the oracle).

#[test]
fn production_strip_is_on_the_convergence_plateau() {
    let c = ctx(1.30, 1.0, 0.03, 0.01);
    let f = c.forward;
    let (kp, kc) = (f / 1.12, f * 1.12);
    let smile = MarketHedgeSmile::new([kp, f, kc], [0.122, 0.10, 0.118], f, c.t);

    let default = fair_variance(&smile, &c).fair_variance;
    let refined = fair_variance_with(
        &smile,
        &c,
        VarSwapStrip {
            nodes_per_leg: 16000,
            wing_std: 16.0,
        },
    )
    .fair_variance;
    assert!(
        (default - refined).abs() < 1e-7,
        "default strip not converged: {default} vs refined {refined}"
    );
}

proptest! {
    // Each case prices two heavy quadratures (the production strip and the
    // independent adaptive-Simpson oracle), so a focused randomised sweep — on
    // top of the four tight fixed cases above — is the right cost/coverage point.
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Across randomised contexts and convex smiles, the production strip and the
    /// independent adaptive-Simpson oracle agree to ~1e-6, and K_vol < √K_var.
    #[test]
    fn prop_strip_matches_oracle_and_vol_below_var(
        spot in 0.5f64..150.0,
        t in 0.1f64..3.0,
        r_dom in 0.0f64..0.06,
        r_for in 0.0f64..0.06,
        atm in 0.06f64..0.30,
        bf in 0.005f64..0.05,
    ) {
        let c = ctx(spot, t, r_dom, r_for);
        let f = c.forward;
        let (kp, kc) = (f / 1.10, f * 1.10);
        let smile = MarketHedgeSmile::new([kp, f, kc], [atm + bf, atm, atm + bf], f, c.t);

        let production = fair_variance(&smile, &c).fair_variance;
        let oracle = oracle_fair_variance(&smile, &c);
        prop_assert!(
            (production - oracle).abs() < 1e-6,
            "strip {production} vs oracle {oracle}"
        );

        let var = fair_variance(&smile, &c);
        let vol = fair_volatility(&smile, &c);
        prop_assert!(vol.fair_vol < var.fair_vol());
    }
}
