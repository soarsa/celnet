//! Volatility-swap fair strike by the **convexity (Jensen) adjustment** to the
//! variance-swap strike.
//!
//! A volatility swap pays realised *volatility* (`√` of realised variance), not
//! variance. Because `√·` is strictly concave, Jensen's inequality makes the fair
//! volatility strike strictly **below** the square root of the fair variance:
//!
//! ```text
//!   K_vol  =  √K_var  −  convexity_correction ,      convexity_correction > 0 .
//! ```
//!
//! The correction is the second-order term of `E[√v]` expanded around its mean
//! `E[v] = K_var` (`v` = realised annualised variance):
//!
//! ```text
//!   E[√v] ≈ √(E v) − ⅛ · Var(v) / (E v)^{3/2} ,
//! ```
//!
//! so
//!
//! ```text
//!                       Var(v)
//!   convexity_correction = ────────────── ,
//!                       8 · K_var^{3/2}
//! ```
//!
//! the classic variance-of-variance (vol-of-vol) Jensen gap. For any
//! **non-degenerate** smile `Var(v) > 0`, hence `K_vol < √K_var` strictly; for a
//! flat smile `Var(v) = 0` and the bound collapses to equality (`K_vol = σ`).
//!
//! # Estimating the variance of realised variance from the smile
//!
//! The replication strip already prices the whole risk-neutral terminal
//! distribution of `S_T` through the OTM option continuum. The variance of the
//! *realised* variance is dominated by the dispersion of the smile-implied
//! instantaneous variance across the states the underlying can reach, weighted by
//! the same `1/K²` log-contract measure that defines `K_var`. We therefore build
//! `Var(v)` as the log-contract-weighted **second central moment** of the
//! per-strike implied variance `σ(K)²` about its weighted mean (which is exactly
//! `K_var`):
//!
//! ```text
//!   Var(v) = ⟨ (σ(K)² − K_var)² ⟩_w ,
//! ```
//!
//! with the weight `w(K) ∝ 1/K²` (normalised over the strip). A flat smile has
//! `σ(K)² ≡ K_var` so `Var(v) = 0` exactly; a convex (positive-butterfly) smile
//! spreads `σ(K)²` and lifts `Var(v)` monotonically with the butterfly — the
//! Carr–Lee direction.
//!
//! # Method provenance (doc comments only)
//!
//! Concavity/convexity adjustment from variance to volatility swaps and the
//! second-order Jensen expansion: Carr & Lee (2008, 2009, *Robust Replication of
//! Volatility Derivatives*); Brockhaus & Long (2000, the `Var(v)/(8·K_var^{3/2})`
//! convexity term); Demeterfi, Derman, Kamal & Zou (1999). All identifiers here
//! are purpose-named and vendor/research-neutral; provenance lives only in docs.

use crate::var_swap::{VarSwapContext, VarSwapResult, VarSwapStrip, fair_variance_with};
use celnet_core::Smile;
use celnet_core::math::exp;

/// The fair volatility-swap strike `K_vol`, the variance-swap strike it is
/// adjusted down from, and the convexity (Jensen) gap between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolSwapResult {
    /// Fair volatility strike `K_vol = √K_var − convexity_correction`.
    pub fair_vol: f64,
    /// The fair variance strike `K_var` of the companion variance swap.
    pub fair_variance: f64,
    /// `√K_var` — the (always larger) naïve variance-swap vol equivalent.
    pub sqrt_fair_variance: f64,
    /// The (non-negative) convexity correction subtracted off `√K_var`.
    pub convexity_correction: f64,
    /// The variance of realised variance `Var(v)` driving the correction.
    pub variance_of_variance: f64,
}

/// Log-contract-weighted second central moment of the per-strike implied
/// variance about `K_var` — the `Var(v)` that drives the convexity correction.
///
/// Mirrors the strip geometry of [`fair_variance_with`]: a uniform `u = ln(K/F)`
/// grid, weight `∝ 1/K²` (i.e. `∝ 1/K` per `du` after the `dK = K du` Jacobian),
/// normalised over the grid so it is a genuine (weighted) variance. The grid
/// reaches the same effective wing the adaptive strip integrates over (eight
/// reference block widths), so the moment is consistent with `K_var`.
fn variance_of_variance<S: Smile>(
    smile: &S,
    ctx: &VarSwapContext,
    strip: VarSwapStrip,
    k_var: f64,
) -> f64 {
    let atm = smile.implied_vol(ctx.forward, ctx.forward, ctx.t).0;
    // Match the strip's effective reach (`max_u = 8·block_width`) and keep its
    // uniform `u`-step `h = block_width / nodes_per_leg`.
    let block_width = strip.wing_std * atm * ctx.t.sqrt();
    let u_max = block_width * 8.0;
    let h = block_width / strip.nodes_per_leg as f64;
    let n = {
        let raw = ((2.0 * u_max) / h).round() as usize;
        if raw < 2 {
            2
        } else if raw.is_multiple_of(2) {
            raw
        } else {
            raw + 1
        }
    };
    let h = (2.0 * u_max) / n as f64;
    let f = ctx.forward;

    // Composite-Simpson weighting over u ∈ [−u_max, u_max] of both the weight
    // density w(u) ∝ 1/K (the log-contract 1/K² times the dK = K du Jacobian)
    // and w(u)·(σ(K)²−K_var)². Normalising the second by the first yields the
    // weighted second central moment regardless of the (cancelling) constants.
    let simpson_w = |j: usize| -> f64 {
        if j == 0 || j == n {
            1.0
        } else if j.is_multiple_of(2) {
            2.0
        } else {
            4.0
        }
    };

    let mut norm = 0.0;
    let mut moment = 0.0;
    for j in 0..=n {
        let u = -u_max + j as f64 * h;
        let k = f * exp(u);
        let w = simpson_w(j) / k; // 1/K log-contract density on the u-grid
        let var_k = {
            let s = smile.implied_vol(k, f, ctx.t).0;
            s * s
        };
        let d = var_k - k_var;
        norm += w;
        moment += w * d * d;
    }
    if norm > 0.0 { moment / norm } else { 0.0 }
}

/// Fair volatility-swap strike `K_vol` over the supplied [`Smile`], with an
/// explicit strip discretisation.
///
/// Computes the companion variance-swap strike `K_var` (log-contract
/// replication), the variance of realised variance `Var(v)` (smile dispersion),
/// and the second-order convexity adjustment
/// `K_vol = √K_var − Var(v)/(8·K_var^{3/2})`.
///
/// By construction `K_vol ≤ √K_var`, with strict inequality for any
/// non-degenerate (`Var(v) > 0`) smile.
#[must_use]
pub fn fair_volatility_with<S: Smile>(
    smile: &S,
    ctx: &VarSwapContext,
    strip: VarSwapStrip,
) -> VolSwapResult {
    let var_res: VarSwapResult = fair_variance_with(smile, ctx, strip);
    let k_var = var_res.fair_variance;
    let sqrt_var = k_var.max(0.0).sqrt();

    let vov = variance_of_variance(smile, ctx, strip, k_var);
    // Second-order Jensen term ⅛·Var(v)/K_var^{3/2}; guarded for K_var → 0.
    let correction = if k_var > 0.0 {
        vov / (8.0 * k_var * sqrt_var)
    } else {
        0.0
    };

    VolSwapResult {
        fair_vol: sqrt_var - correction,
        fair_variance: k_var,
        sqrt_fair_variance: sqrt_var,
        convexity_correction: correction,
        variance_of_variance: vov,
    }
}

/// Fair volatility-swap strike `K_vol` with the production-default strip
/// ([`VarSwapStrip::default`]).
#[must_use]
pub fn fair_volatility<S: Smile>(smile: &S, ctx: &VarSwapContext) -> VolSwapResult {
    fair_volatility_with(smile, ctx, VarSwapStrip::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::var_swap::fair_variance;
    use celnet_core::FlatSmile;
    use celnet_surface::MarketHedgeSmile;
    use celnet_types::VanillaInputs;

    fn ctx() -> VarSwapContext {
        let i = VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01);
        VarSwapContext::from_inputs(&(&i).into())
    }

    #[test]
    fn flat_smile_has_zero_convexity_gap() {
        // A flat smile is degenerate (Var(v)=0): K_vol = √K_var = σ exactly.
        let c = ctx();
        let res = fair_volatility(&FlatSmile::new(0.20), &c);
        celnet_core::assert_close!(res.variance_of_variance, 0.0, 1e-12, 1e-12);
        celnet_core::assert_close!(res.convexity_correction, 0.0, 1e-12, 1e-12);
        celnet_core::assert_close!(res.fair_vol, 0.20, 1e-6, 1e-7);
    }

    #[test]
    fn convex_smile_is_strictly_below_sqrt_var() {
        let c = ctx();
        let f = c.forward;
        let (kp, kc) = (f / 1.10, f * 1.10);
        let smile = MarketHedgeSmile::new([kp, f, kc], [0.13, 0.10, 0.13], f, c.t);
        let vol = fair_volatility(&smile, &c);
        let var = fair_variance(&smile, &c);
        assert!(vol.variance_of_variance > 0.0);
        assert!(vol.convexity_correction > 0.0);
        assert!(
            vol.fair_vol < var.fair_vol(),
            "K_vol {} must be < √K_var {}",
            vol.fair_vol,
            var.fair_vol()
        );
    }

    #[test]
    fn variance_of_variance_grows_monotonically_with_butterfly() {
        // The genuine Carr–Lee driver is the variance of realised variance: a
        // wider butterfly (more smile convexity) spreads σ(K)² across states and
        // must raise Var(v) monotonically. (The *gap* √K_var − K_vol =
        // Var(v)/(8·K_var^{3/2}) peaks at moderate convexity then declines as the
        // K_var^{3/2} normalisation overtakes — see `gap_widens_over_fx_regime`.)
        let c = ctx();
        let f = c.forward;
        let (kp, kc) = (f / 1.10, f * 1.10);
        let mut last_vov = -1.0;
        for wing in [0.105_f64, 0.12, 0.14, 0.17, 0.20] {
            let smile = MarketHedgeSmile::new([kp, f, kc], [wing, 0.10, wing], f, c.t);
            let res = fair_volatility(&smile, &c);
            assert!(
                res.variance_of_variance > last_vov,
                "Var(v) must grow with butterfly: {} !> {}",
                res.variance_of_variance,
                last_vov
            );
            last_vov = res.variance_of_variance;
        }
    }

    #[test]
    fn gap_widens_over_fx_regime() {
        // Over the realistic FX butterfly regime (wings up to ~+3 vol over ATM)
        // the convexity gap itself grows monotonically with smile convexity.
        let c = ctx();
        let f = c.forward;
        let (kp, kc) = (f / 1.10, f * 1.10);
        let mut last_gap = -1.0;
        for wing in [0.102_f64, 0.108, 0.115, 0.125] {
            let smile = MarketHedgeSmile::new([kp, f, kc], [wing, 0.10, wing], f, c.t);
            let res = fair_volatility(&smile, &c);
            assert!(
                res.convexity_correction > last_gap,
                "gap must grow with butterfly in the FX regime: {} !> {}",
                res.convexity_correction,
                last_gap
            );
            last_gap = res.convexity_correction;
        }
    }
}
