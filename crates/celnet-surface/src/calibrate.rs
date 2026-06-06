//! Model-selected smile calibration from broker market quotes.
//!
//! The market-hedge baseline ([`crate::build_smile`]) is the FX broker default,
//! but the platform also offers three richer smile **models** the desk can select
//! per mark: a stochastic-volatility smile (the SABR method), a parametric
//! total-variance slice (the SVI method), and the surface-level parametric family
//! (the SSVI method). This module is the single entry point that, given a
//! [`SmileModel`] selection, calibrates the corresponding model to the **same
//! market anchors** the market-hedge calibration uses and returns a unified
//! [`CalibratedSmile`] implementing [`celnet_core::Smile`].
//!
//! # Common anchors (no model-specific market data)
//!
//! Every model is calibrated to one set of market anchors derived from the broker
//! quotes via the existing convention-aware Vanna-Volga pillar calibration
//! ([`crate::calibrate_pillar`]): the at-the-money-forward volatility plus the
//! `25Δ` (and, when the broker set carries it, `10Δ`) smile wing volatilities at
//! their convention strikes. Working from the *calibrated* wing vols (not the raw
//! broker RR/BF) means the alternative models reproduce the same wings the
//! market-hedge baseline reprices — so a desk can switch models and still see the
//! market it quoted. The fit is performed in **(log-moneyness, total variance)**
//! space `(k = ln(K/F), w = σ²·t)`, the natural coordinate for SVI/SSVI and the
//! one in which calendar no-arbitrage is linear.
//!
//! # Determinism
//!
//! All arithmetic routes through [`celnet_core::math`] (libm) and the fits are
//! fixed-iteration Gauss-Newton / damped-Newton solves with deterministic
//! initialisation, so a calibration is bit-reproducible across platforms (the
//! `CLAUDE.md` determinism guardrail). No OS RNG, no wall-clock, no global state.
//!
//! # Method provenance (doc-only)
//!
//! SABR expansion: Hagan, Kumar, Lesniewski & Woodward (2002). Raw-SVI and its
//! no-arbitrage conditions: Gatheral (2004); Gatheral & Jacquier (2014). SSVI
//! surface form and its closed-form arbitrage conditions: Gatheral & Jacquier
//! (2014, §4). Vanna-Volga anchoring: Castagna & Mercurio (2007). All names are
//! provenance only; every public identifier is purpose-named.

use celnet_core::Smile;
use celnet_core::math::{ln, sqrt};
use celnet_types::Vol;

use crate::market_hedge::MarketHedgeSmile;
use crate::parametric::ParametricSlice;
use crate::parametric_surface::ParametricSurface;
use crate::quotes::{MarketContext, MarketQuotes};
use crate::stochvol::{StochasticVolParams, StochasticVolSmile};
use crate::strangle::{CalibrationError, calibrate_pillar};
use crate::surface::SmileModel;

/// A calibrated smile of the selected model, presented through one type so the
/// engine/edge can hold "a calibrated smile" without knowing which family backs
/// it. Every variant implements [`celnet_core::Smile`], so evaluation is the same
/// trait call regardless of the model.
#[derive(Debug, Clone)]
pub enum CalibratedSmile {
    /// The market-hedge (Vanna-Volga) baseline smile.
    MarketHedge(MarketHedgeSmile),
    /// A stochastic-volatility (SABR) smile fitted to the anchors.
    StochasticVol(StochasticVolSmile),
    /// A parametric total-variance (SVI) slice fitted to the anchors.
    Parametric(ParametricSlice),
    /// A surface-level parametric (SSVI) slice fitted to the anchors. Carried as a
    /// materialised raw slice at the calibrated maturity (the SSVI→raw closed-form
    /// map is exact, so this is the same smile the surface form evaluates).
    ParametricSurface(ParametricSlice),
    /// An extended surface-level parametric (eSSVI) slice fitted to the anchors.
    /// Carried as a materialised raw slice at the calibrated maturity (the
    /// eSSVI→raw closed-form map is exact, so this is the same smile the extended
    /// surface form evaluates).
    ExtendedSurface(ParametricSlice),
}

impl CalibratedSmile {
    /// The smile-model family this calibration belongs to (for provenance / audit
    /// and to echo the model actually used back to the caller).
    #[must_use]
    pub fn model(&self) -> SmileModel {
        match self {
            CalibratedSmile::MarketHedge(_) => SmileModel::MarketHedge,
            CalibratedSmile::StochasticVol(_) => SmileModel::StochasticVol,
            CalibratedSmile::Parametric(_) => SmileModel::Parametric,
            CalibratedSmile::ParametricSurface(_) => SmileModel::ParametricSurface,
            CalibratedSmile::ExtendedSurface(_) => SmileModel::ExtendedSurface,
        }
    }

    /// The outright forward this smile is anchored on.
    #[must_use]
    pub fn forward(&self) -> f64 {
        match self {
            CalibratedSmile::MarketHedge(s) => s.forward(),
            CalibratedSmile::StochasticVol(s) => s.params().forward,
            CalibratedSmile::Parametric(s)
            | CalibratedSmile::ParametricSurface(s)
            | CalibratedSmile::ExtendedSurface(s) => s.forward,
        }
    }
}

impl Smile for CalibratedSmile {
    fn implied_vol(&self, strike: f64, forward: f64, t: f64) -> Vol {
        match self {
            CalibratedSmile::MarketHedge(s) => s.implied_vol(strike, forward, t),
            CalibratedSmile::StochasticVol(s) => s.implied_vol(strike, forward, t),
            CalibratedSmile::Parametric(s)
            | CalibratedSmile::ParametricSurface(s)
            | CalibratedSmile::ExtendedSurface(s) => s.implied_vol(strike, forward, t),
        }
    }
}

/// One market anchor the alternative models are fitted to: a log-moneyness and the
/// total implied variance observed there.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    /// Log-moneyness `k = ln(K/F)`.
    k: f64,
    /// Total implied variance `w = σ²·t`.
    w: f64,
}

/// Derive the market anchors `(k, w)` for the alternative-model fits from the
/// broker quotes, via the convention-aware Vanna-Volga pillar calibration.
///
/// Always yields three anchors (put `25Δ` wing, ATM, call `25Δ` wing); when the
/// broker set carries a `10Δ` pillar two more are appended (the outer wings), so
/// the parametric fits see the convexity the wider wings imply.
fn anchors(ctx: &MarketContext, quotes: &MarketQuotes) -> Result<Vec<Anchor>, CalibrationError> {
    let atm_vol = quotes.atm_vol;
    let forward = ctx.forward();
    let t = ctx.t;

    // The inner (25Δ) calibrated wings.
    let inner = calibrate_pillar(ctx, atm_vol, quotes.inner)?;

    let mut out = vec![
        Anchor {
            k: ln(inner.put_strike / forward),
            w: inner.put_vol * inner.put_vol * t,
        },
        // ATM-forward anchor: k = 0 by construction (strike == forward).
        Anchor {
            k: 0.0,
            w: atm_vol * atm_vol * t,
        },
        Anchor {
            k: ln(inner.call_strike / forward),
            w: inner.call_vol * inner.call_vol * t,
        },
    ];

    if let Some(outer_quote) = quotes.outer {
        let outer = calibrate_pillar(ctx, atm_vol, outer_quote)?;
        out.push(Anchor {
            k: ln(outer.put_strike / forward),
            w: outer.put_vol * outer.put_vol * t,
        });
        out.push(Anchor {
            k: ln(outer.call_strike / forward),
            w: outer.call_vol * outer.call_vol * t,
        });
    }

    Ok(out)
}

/// Calibrate a smile of the selected `model` from broker quotes against a market
/// context.
///
/// `SmileModel::MarketHedge` returns the Vanna-Volga baseline unchanged
/// ([`crate::build_smile`]); the other three models are fitted, in total-variance
/// space, to the same Vanna-Volga-calibrated anchors so they reproduce the market
/// the baseline reprices.
///
/// # Errors
///
/// Returns [`CalibrationError`] if the underlying pillar calibration fails or the
/// model fit cannot reach a well-posed (positive-variance, in-range) slice.
pub fn build_model_smile(
    model: SmileModel,
    ctx: &MarketContext,
    quotes: &MarketQuotes,
) -> Result<CalibratedSmile, CalibrationError> {
    match model {
        SmileModel::MarketHedge => Ok(CalibratedSmile::MarketHedge(crate::build_smile(
            ctx, quotes,
        )?)),
        SmileModel::StochasticVol => {
            let smile = fit_sabr(ctx, quotes)?;
            Ok(CalibratedSmile::StochasticVol(smile))
        }
        SmileModel::Parametric => {
            let slice = fit_svi(ctx, quotes)?;
            Ok(CalibratedSmile::Parametric(slice))
        }
        SmileModel::ParametricSurface => {
            let slice = fit_ssvi(ctx, quotes)?;
            Ok(CalibratedSmile::ParametricSurface(slice))
        }
        SmileModel::ExtendedSurface => {
            let slice = fit_essvi(ctx, quotes)?;
            Ok(CalibratedSmile::ExtendedSurface(slice))
        }
    }
}

// ===========================================================================
// SABR (StochasticVol) — fix the lognormal backbone β = 1 (FX convention), then
// fit (α, ρ, ν) to the ATM vol + the wing vols by a small damped Gauss-Newton on
// the lognormal-vol residuals. β = 1 is the standard FX choice (Clark 2011): the
// FX forward is lognormal, so the unit backbone reproduces the at-forward vol with
// α ≈ σ_ATM and lets ρ/ν carry the skew/convexity.
// ===========================================================================

/// FX lognormal SABR backbone exponent.
const SABR_BETA: f64 = 1.0;
/// Maximum Gauss-Newton iterations for the SABR / SVI fits.
const FIT_ITERS: usize = 60;
/// Finite-difference step for the parameter Jacobians.
const FIT_FD_H: f64 = 1e-6;

/// Fit a SABR slice to the anchors. With β = 1 the at-forward vol is `α·B_atm`
/// with `B_atm → 1` for small `t`, so `α₀ = σ_ATM` is an excellent initialiser;
/// `ρ` is seeded from the wing skew and `ν` from the convexity.
fn fit_sabr(
    ctx: &MarketContext,
    quotes: &MarketQuotes,
) -> Result<StochasticVolSmile, CalibrationError> {
    let pts = anchors(ctx, quotes)?;
    let forward = ctx.forward();
    let t = ctx.t;
    let atm_vol = quotes.atm_vol;

    // Black vol at each anchor strike implied by the total-variance anchor.
    let targets: Vec<(f64, f64)> = pts
        .iter()
        .map(|a| {
            let strike = forward * celnet_core::math::exp(a.k);
            let vol = sqrt(a.w / t);
            (strike, vol)
        })
        .collect();

    // Seed (α, ρ, ν): α from ATM, ρ from the 25Δ skew sign, ν from convexity.
    let inner = calibrate_pillar(ctx, atm_vol, quotes.inner)?;
    let rr = inner.call_vol - inner.put_vol;
    let bf = 0.5 * (inner.call_vol + inner.put_vol) - atm_vol;
    let mut alpha = atm_vol.max(1e-4);
    // ρ ∝ skew; clamp well inside (−1, 1).
    let mut rho = clamp((rr / atm_vol.max(1e-3)) * 2.0, -0.9, 0.9);
    // ν from the convexity (butterfly); strictly positive.
    let mut nu = (bf.abs() / atm_vol.max(1e-3) * 8.0).clamp(0.05, 3.0);

    let model = |alpha: f64, rho: f64, nu: f64, strike: f64| -> f64 {
        // Guard the parameter domain so the evaluation never panics mid-fit.
        let a = alpha.max(1e-8);
        let r = clamp(rho, -0.999, 0.999);
        let v = nu.max(0.0);
        StochasticVolParams::new(a, SABR_BETA, r, v, forward, t).black_vol(strike)
    };

    let residuals = |alpha: f64, rho: f64, nu: f64| -> Vec<f64> {
        targets
            .iter()
            .map(|&(strike, vol)| model(alpha, rho, nu, strike) - vol)
            .collect()
    };

    gauss_newton_3(
        &mut alpha,
        &mut rho,
        &mut nu,
        residuals,
        (1e-8, f64::INFINITY),
        (-0.999, 0.999),
        (0.0, 10.0),
    );

    let params = StochasticVolParams::new(
        alpha.max(1e-8),
        SABR_BETA,
        clamp(rho, -0.999, 0.999),
        nu.max(0.0),
        forward,
        t,
    );
    Ok(StochasticVolSmile::new(params))
}

// ===========================================================================
// SVI (Parametric) — fit the five raw parameters (a, b, ρ, m, σ) to the
// total-variance anchors. We pin the level so the ATM anchor is matched exactly
// (a = w_atm − b·(ρ·(−m) + √(m²+σ²)) after the others are solved), and fit
// (b, ρ, m, σ) by damped Gauss-Newton on the total-variance residuals, projecting
// each step back into the SVI no-arbitrage box (b ≥ 0, |ρ| < 1, σ > 0, and the
// large-wing slope bound b(1+|ρ|) ≤ 2 of Lee/Gatheral-Jacquier).
// ===========================================================================

/// Fit a raw-SVI slice to the total-variance anchors.
fn fit_svi(
    ctx: &MarketContext,
    quotes: &MarketQuotes,
) -> Result<ParametricSlice, CalibrationError> {
    let pts = anchors(ctx, quotes)?;
    let forward = ctx.forward();
    let t = ctx.t;
    let atm_vol = quotes.atm_vol;
    let w_atm = atm_vol * atm_vol * t;

    // Seed from the skew/convexity of the anchors.
    let inner = calibrate_pillar(ctx, atm_vol, quotes.inner)?;
    let rr = inner.call_vol - inner.put_vol;
    let bf = 0.5 * (inner.call_vol + inner.put_vol) - atm_vol;

    let mut b = (bf.abs() / atm_vol.max(1e-3) * 0.5 + 0.02).clamp(1e-3, 0.9);
    let mut rho = clamp(
        rr.signum() * (rr.abs() / atm_vol.max(1e-3)).min(0.9),
        -0.9,
        0.9,
    );
    let mut m = clamp(-rho * 0.1, -0.5, 0.5);
    let mut sigma = 0.1_f64.max(atm_vol);

    // Given (b, ρ, m, σ), the level `a` is pinned so the ATM anchor (k = 0) is
    // reproduced exactly: w(0) = a + b·(ρ·(0−m) + √((0−m)²+σ²)) = w_atm.
    let level = |b: f64, rho: f64, m: f64, sigma: f64| -> f64 {
        let d0 = -m;
        w_atm - b * (rho * d0 + sqrt(d0 * d0 + sigma * sigma))
    };

    let w_of = |b: f64, rho: f64, m: f64, sigma: f64, k: f64| -> f64 {
        let a = level(b, rho, m, sigma);
        let d = k - m;
        a + b * (rho * d + sqrt(d * d + sigma * sigma))
    };

    let residuals = |b: f64, rho: f64, m: f64, sigma: f64| -> Vec<f64> {
        pts.iter()
            .map(|a| w_of(b, rho, m, sigma, a.k) - a.w)
            .collect()
    };

    gauss_newton_4(
        &mut b,
        &mut rho,
        &mut m,
        &mut sigma,
        residuals,
        |b, rho, m, sigma| project_svi(b, rho, m, sigma, w_atm),
    );

    // Final projection + level, then build through the validating constructor.
    let (b, rho, m, sigma) = project_svi(b, rho, m, sigma, w_atm);
    let a = level(b, rho, m, sigma);
    // A degenerate fit can leave the minimum total variance negative; the
    // validating constructor rejects it as a degenerate quote rather than panic.
    let w_min = a + b * sigma * sqrt(1.0 - rho * rho);
    if !(a.is_finite() && b.is_finite() && m.is_finite() && sigma.is_finite())
        || w_min < -1e-12
        || sigma <= 0.0
    {
        return Err(CalibrationError::DegenerateQuote);
    }
    Ok(ParametricSlice::new(a, b, rho, m, sigma, forward, t))
}

/// Project an SVI parameter step back into the admissible no-arbitrage box: `b` in
/// `[1e-4, 2/(1+|ρ|)]` (the large-wing slope bound), `|ρ| ≤ 0.999`, `σ` in
/// `[1e-3, 5]`, `m` in `[−2, 2]`, and a non-negative minimum total variance.
fn project_svi(b: f64, rho: f64, m: f64, sigma: f64, w_atm: f64) -> (f64, f64, f64, f64) {
    let rho = clamp(rho, -0.999, 0.999);
    let sigma = clamp(sigma, 1e-3, 5.0);
    let m = clamp(m, -2.0, 2.0);
    // Lee/Gatheral-Jacquier large-wing bound: b·(1+|ρ|) ≤ 2.
    let b_cap = 2.0 / (1.0 + rho.abs());
    let mut b = clamp(b, 1e-4, b_cap);
    // Keep the minimum total variance non-negative (a + b·σ·√(1−ρ²) ≥ 0 with the
    // ATM-pinned level): reduce b if it would push the floor negative.
    let d0 = -m;
    let level = w_atm - b * (rho * d0 + sqrt(d0 * d0 + sigma * sigma));
    let w_min = level + b * sigma * sqrt(1.0 - rho * rho);
    if w_min < 0.0 {
        // Shrink b toward a value that keeps w_min ≥ 0 (linear in b; one solve).
        let g = (rho * d0 + sqrt(d0 * d0 + sigma * sigma)) - sigma * sqrt(1.0 - rho * rho);
        if g > 1e-12 {
            b = (w_atm / g * 0.999).clamp(1e-4, b_cap);
        }
    }
    (b, rho, m, sigma)
}

// ===========================================================================
// SSVI (ParametricSurface) — the ATM total variance θ is pinned to the ATM anchor
// (w_atm), and the two free shape parameters (ρ, φ) are fitted to the wing anchors.
// We parameterise φ directly (a single maturity carries one θ, so the η/γ split is
// not identifiable from one slice — we fit the effective φ and report η = φ·θ^γ at
// a canonical γ). The fitted slice is materialised via the exact SSVI→raw map.
// ===========================================================================

/// Canonical SSVI curvature-decay exponent for a single-maturity fit. With one
/// slice the power-law decay is not identifiable, so we fix `γ` at the midpoint of
/// its admissible range `(0, ½]` and fit `(ρ, φ)`; `η = φ·θ^γ` then reports an
/// equivalent surface-form parameter. The slice the fit produces is independent of
/// this choice (it is fixed by `θ`, `ρ`, `φ`).
const SSVI_GAMMA: f64 = 0.5;

/// Fit an SSVI slice (materialised as a raw slice) to the anchors, pinning θ to the
/// ATM total variance and fitting `(ρ, φ)` to the wings.
fn fit_ssvi(
    ctx: &MarketContext,
    quotes: &MarketQuotes,
) -> Result<ParametricSlice, CalibrationError> {
    let pts = anchors(ctx, quotes)?;
    let forward = ctx.forward();
    let t = ctx.t;
    let atm_vol = quotes.atm_vol;
    let theta = atm_vol * atm_vol * t;
    if theta <= 0.0 {
        return Err(CalibrationError::DegenerateQuote);
    }

    // SSVI total variance with θ pinned and φ a free shape parameter:
    //   w(k) = (θ/2)·(1 + ρ·φ·k + √((φ·k + ρ)² + (1 − ρ²))).
    let w_of = |rho: f64, phi: f64, k: f64| -> f64 {
        let pk = phi * k + rho;
        0.5 * theta * (1.0 + rho * phi * k + sqrt(pk * pk + (1.0 - rho * rho)))
    };

    let inner = calibrate_pillar(ctx, atm_vol, quotes.inner)?;
    let rr = inner.call_vol - inner.put_vol;
    let bf = 0.5 * (inner.call_vol + inner.put_vol) - atm_vol;

    let mut rho = clamp(
        rr.signum() * (rr.abs() / atm_vol.max(1e-3)).min(0.9),
        -0.9,
        0.9,
    );
    let mut phi = (bf.abs() / atm_vol.max(1e-3) * 10.0 + 1.0).clamp(0.1, 20.0);

    let residuals = |rho: f64, phi: f64| -> Vec<f64> {
        pts.iter().map(|a| w_of(rho, phi, a.k) - a.w).collect()
    };

    gauss_newton_2(
        &mut rho,
        &mut phi,
        residuals,
        (-0.999, 0.999),
        |rho, phi| {
            // SSVI butterfly sufficient conditions (Gatheral-Jacquier Thm 4.2):
            //   θ·φ·(1+|ρ|) < 4  and  θ·φ²·(1+|ρ|) ≤ 4.
            // Project φ to satisfy both (the tighter of the two caps).
            let rho = clamp(rho, -0.999, 0.999);
            let one_p = 1.0 + rho.abs();
            let cap1 = 4.0 / (theta * one_p) * 0.999;
            let cap2 = sqrt(4.0 / (theta * one_p)) * 0.999;
            let phi = clamp(phi, 1e-3, cap1.min(cap2));
            (rho, phi)
        },
    );

    // Final projection.
    let rho = clamp(rho, -0.999, 0.999);
    let one_p = 1.0 + rho.abs();
    let cap1 = 4.0 / (theta * one_p) * 0.999;
    let cap2 = sqrt(4.0 / (theta * one_p)) * 0.999;
    let phi = clamp(phi, 1e-3, cap1.min(cap2));

    // η = φ·θ^γ at the canonical γ reproduces this φ via ParametricSurface::phi.
    let eta = phi * crate::mathx::powf(theta, SSVI_GAMMA);
    if !(eta.is_finite() && eta > 0.0) {
        return Err(CalibrationError::DegenerateQuote);
    }
    let surface = ParametricSurface::new(rho, eta, SSVI_GAMMA);
    Ok(surface.to_slice(theta, forward, t))
}

// ===========================================================================
// eSSVI (ExtendedSurface) — the extended SSVI in the (θ, ρ, ψ) variables, with
// the ATM total variance θ pinned to the ATM anchor and the two free shape
// parameters (ρ, ψ) fitted to the wing anchors. ψ = θ·φ is the ATM skew-scale;
// for a single calibrated maturity eSSVI's per-slice butterfly domain is exactly
// the SSVI domain re-expressed in (θ,ρ,ψ): ψ(1+|ρ|) < 4 and (ψ²/θ)(1+|ρ|) ≤ 4.
// The fitted slice is materialised via the exact eSSVI→raw map. (The
// maturity-dependence of ρ — the genuine eSSVI generalisation — lives across
// slices in `ExtendedSurface`; one mark request calibrates one slice.)
// ===========================================================================

/// Fit an eSSVI slice (materialised as a raw slice) to the anchors, pinning θ to
/// the ATM total variance and fitting `(ρ, ψ)` to the wings, projecting each step
/// into the eSSVI per-slice butterfly domain.
fn fit_essvi(
    ctx: &MarketContext,
    quotes: &MarketQuotes,
) -> Result<ParametricSlice, CalibrationError> {
    let pts = anchors(ctx, quotes)?;
    let forward = ctx.forward();
    let t = ctx.t;
    let atm_vol = quotes.atm_vol;
    let theta = atm_vol * atm_vol * t;
    if theta <= 0.0 {
        return Err(CalibrationError::DegenerateQuote);
    }

    // eSSVI total variance with θ pinned and (ρ, ψ) the free shape parameters
    // (φ = ψ/θ):
    //   w(k) = (θ/2)·(1 + ρ·(ψ/θ)·k + √(((ψ/θ)·k + ρ)² + (1 − ρ²))).
    let w_of = |rho: f64, psi: f64, k: f64| -> f64 {
        let p = psi / theta;
        let pk = p * k + rho;
        0.5 * theta * (1.0 + rho * p * k + sqrt(pk * pk + (1.0 - rho * rho)))
    };

    let inner = calibrate_pillar(ctx, atm_vol, quotes.inner)?;
    let rr = inner.call_vol - inner.put_vol;
    let bf = 0.5 * (inner.call_vol + inner.put_vol) - atm_vol;

    let mut rho = clamp(
        rr.signum() * (rr.abs() / atm_vol.max(1e-3)).min(0.9),
        -0.9,
        0.9,
    );
    // ψ seeds off the convexity; ATM skew is ρ·ψ, so scale modestly.
    let mut psi = (bf.abs() / atm_vol.max(1e-3) * theta * 10.0 + 0.1).clamp(1e-3, 3.0);

    let residuals = |rho: f64, psi: f64| -> Vec<f64> {
        pts.iter().map(|a| w_of(rho, psi, a.k) - a.w).collect()
    };

    // The eSSVI per-slice butterfly domain in (θ, ρ, ψ):
    //   ψ·(1+|ρ|) < 4  and  (ψ²/θ)·(1+|ρ|) ≤ 4 (project ψ to the tighter cap).
    let project = |rho: f64, psi: f64| {
        let rho = clamp(rho, -0.999, 0.999);
        let one_p = 1.0 + rho.abs();
        let cap1 = 4.0 / one_p * 0.999;
        let cap2 = sqrt(4.0 * theta / one_p) * 0.999;
        let psi = clamp(psi, 1e-3, cap1.min(cap2));
        (rho, psi)
    };

    gauss_newton_2(&mut rho, &mut psi, residuals, (-0.999, 0.999), project);

    let (rho, psi) = project(rho, psi);
    if !(psi.is_finite() && psi > 0.0 && rho.is_finite()) {
        return Err(CalibrationError::DegenerateQuote);
    }
    // Materialise via the exact eSSVI→raw map (φ = ψ/θ).
    let slice = crate::extended_surface::ExtendedSlice::new(theta, rho, psi);
    Ok(slice.to_slice(forward, t))
}

// ===========================================================================
// Deterministic damped Gauss-Newton solvers (libm only).
//
// Each solver minimises the sum of squared residuals over a fixed parameter set
// by repeatedly forming the finite-difference Jacobian Jᵀ, solving the normal
// equations (JᵀJ + λI)·δ = −Jᵀr with a small Levenberg damping λ, projecting the
// step into the admissible box, and accepting it only if it reduces the residual
// norm (otherwise the damping is increased). The iteration count is fixed so the
// solve is bit-reproducible; the residual-decrease guard makes it robust.
// ===========================================================================

/// Clamp a value into `[lo, hi]` (deterministic; libm-free).
#[inline]
fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

/// Sum of squares of a residual vector.
fn sumsq(r: &[f64]) -> f64 {
    r.iter().map(|x| x * x).sum()
}

/// A 2-parameter damped Gauss-Newton solve with a projection.
fn gauss_newton_2<R, P>(p0: &mut f64, p1: &mut f64, residuals: R, _b0: (f64, f64), project: P)
where
    R: Fn(f64, f64) -> Vec<f64>,
    P: Fn(f64, f64) -> (f64, f64),
{
    let mut lambda = 1e-3;
    let mut r = residuals(*p0, *p1);
    let mut cost = sumsq(&r);
    for _ in 0..FIT_ITERS {
        let n = r.len();
        // Jacobian columns by forward differences.
        let r0 = residuals(*p0 + FIT_FD_H, *p1);
        let r1 = residuals(*p0, *p1 + FIT_FD_H);
        let mut jtj = [[0.0_f64; 2]; 2];
        let mut jtr = [0.0_f64; 2];
        for i in 0..n {
            let j0 = (r0[i] - r[i]) / FIT_FD_H;
            let j1 = (r1[i] - r[i]) / FIT_FD_H;
            jtj[0][0] += j0 * j0;
            jtj[0][1] += j0 * j1;
            jtj[1][0] += j1 * j0;
            jtj[1][1] += j1 * j1;
            jtr[0] += j0 * r[i];
            jtr[1] += j1 * r[i];
        }
        // (JᵀJ + λI) δ = −Jᵀr.
        let a = jtj[0][0] + lambda;
        let d = jtj[1][1] + lambda;
        let bc = jtj[0][1];
        let det = a * d - bc * bc;
        if det.abs() < 1e-300 {
            break;
        }
        let dx0 = -(d * jtr[0] - bc * jtr[1]) / det;
        let dx1 = -(a * jtr[1] - bc * jtr[0]) / det;
        let (np0, np1) = project(*p0 + dx0, *p1 + dx1);
        let nr = residuals(np0, np1);
        let ncost = sumsq(&nr);
        if ncost < cost {
            *p0 = np0;
            *p1 = np1;
            r = nr;
            cost = ncost;
            lambda = (lambda * 0.5).max(1e-9);
        } else {
            lambda *= 4.0;
            if lambda > 1e12 {
                break;
            }
        }
    }
}

/// A 3-parameter damped Gauss-Newton solve with per-parameter box clamps.
fn gauss_newton_3<R>(
    p0: &mut f64,
    p1: &mut f64,
    p2: &mut f64,
    residuals: R,
    b0: (f64, f64),
    b1: (f64, f64),
    b2: (f64, f64),
) where
    R: Fn(f64, f64, f64) -> Vec<f64>,
{
    let project = |a: f64, b: f64, c: f64| {
        (
            clamp(a, b0.0, b0.1),
            clamp(b, b1.0, b1.1),
            clamp(c, b2.0, b2.1),
        )
    };
    let mut lambda = 1e-3;
    let mut r = residuals(*p0, *p1, *p2);
    let mut cost = sumsq(&r);
    for _ in 0..FIT_ITERS {
        let n = r.len();
        let ra = residuals(*p0 + FIT_FD_H, *p1, *p2);
        let rb = residuals(*p0, *p1 + FIT_FD_H, *p2);
        let rc = residuals(*p0, *p1, *p2 + FIT_FD_H);
        let mut jtj = [[0.0_f64; 3]; 3];
        let mut jtr = [0.0_f64; 3];
        for i in 0..n {
            let j = [
                (ra[i] - r[i]) / FIT_FD_H,
                (rb[i] - r[i]) / FIT_FD_H,
                (rc[i] - r[i]) / FIT_FD_H,
            ];
            for a in 0..3 {
                jtr[a] += j[a] * r[i];
                for b in 0..3 {
                    jtj[a][b] += j[a] * j[b];
                }
            }
        }
        for (a, row) in jtj.iter_mut().enumerate() {
            row[a] += lambda;
        }
        let Some(delta) = solve3(&jtj, &[-jtr[0], -jtr[1], -jtr[2]]) else {
            break;
        };
        let (np0, np1, np2) = project(*p0 + delta[0], *p1 + delta[1], *p2 + delta[2]);
        let nr = residuals(np0, np1, np2);
        let ncost = sumsq(&nr);
        if ncost < cost {
            *p0 = np0;
            *p1 = np1;
            *p2 = np2;
            r = nr;
            cost = ncost;
            lambda = (lambda * 0.5).max(1e-9);
        } else {
            lambda *= 4.0;
            if lambda > 1e12 {
                break;
            }
        }
    }
}

/// A 4-parameter damped Gauss-Newton solve with a custom projection.
fn gauss_newton_4<R, P>(
    p0: &mut f64,
    p1: &mut f64,
    p2: &mut f64,
    p3: &mut f64,
    residuals: R,
    project: P,
) where
    R: Fn(f64, f64, f64, f64) -> Vec<f64>,
    P: Fn(f64, f64, f64, f64) -> (f64, f64, f64, f64),
{
    let mut lambda = 1e-3;
    let mut r = residuals(*p0, *p1, *p2, *p3);
    let mut cost = sumsq(&r);
    for _ in 0..FIT_ITERS {
        let n = r.len();
        let rcols = [
            residuals(*p0 + FIT_FD_H, *p1, *p2, *p3),
            residuals(*p0, *p1 + FIT_FD_H, *p2, *p3),
            residuals(*p0, *p1, *p2 + FIT_FD_H, *p3),
            residuals(*p0, *p1, *p2, *p3 + FIT_FD_H),
        ];
        let mut jtj = [[0.0_f64; 4]; 4];
        let mut jtr = [0.0_f64; 4];
        for i in 0..n {
            let mut j = [0.0_f64; 4];
            for (c, col) in rcols.iter().enumerate() {
                j[c] = (col[i] - r[i]) / FIT_FD_H;
            }
            for a in 0..4 {
                jtr[a] += j[a] * r[i];
                for b in 0..4 {
                    jtj[a][b] += j[a] * j[b];
                }
            }
        }
        for (a, row) in jtj.iter_mut().enumerate() {
            row[a] += lambda;
        }
        let Some(delta) = solve4(&jtj, &[-jtr[0], -jtr[1], -jtr[2], -jtr[3]]) else {
            break;
        };
        let (np0, np1, np2, np3) = project(
            *p0 + delta[0],
            *p1 + delta[1],
            *p2 + delta[2],
            *p3 + delta[3],
        );
        let nr = residuals(np0, np1, np2, np3);
        let ncost = sumsq(&nr);
        if ncost < cost {
            *p0 = np0;
            *p1 = np1;
            *p2 = np2;
            *p3 = np3;
            r = nr;
            cost = ncost;
            lambda = (lambda * 0.5).max(1e-9);
        } else {
            lambda *= 4.0;
            if lambda > 1e12 {
                break;
            }
        }
    }
}

/// Solve a 3×3 linear system by Gaussian elimination with partial pivoting.
fn solve3(a: &[[f64; 3]; 3], b: &[f64; 3]) -> Option<[f64; 3]> {
    let mut m = [
        [a[0][0], a[0][1], a[0][2], b[0]],
        [a[1][0], a[1][1], a[1][2], b[1]],
        [a[2][0], a[2][1], a[2][2], b[2]],
    ];
    gaussian_eliminate::<3, 4>(&mut m)
}

/// Solve a 4×4 linear system by Gaussian elimination with partial pivoting.
fn solve4(a: &[[f64; 4]; 4], b: &[f64; 4]) -> Option<[f64; 4]> {
    let mut m = [[0.0_f64; 5]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        row[..4].copy_from_slice(&a[i]);
        row[4] = b[i];
    }
    gaussian_eliminate::<4, 5>(&mut m)
}

/// Gaussian elimination with partial pivoting on an augmented `N×C` matrix
/// (`C == N + 1`), returning the `N` solution components or `None` for a singular
/// system. Const-generic so the 3- and 4-parameter normal equations share one
/// numerically-careful implementation (the elimination uses a copied pivot row so
/// the row updates are disjoint borrows — no aliasing, no index-loop lint).
fn gaussian_eliminate<const N: usize, const C: usize>(m: &mut [[f64; C]; N]) -> Option<[f64; N]> {
    for col in 0..N {
        // Partial pivot: swap in the row with the largest magnitude in this column.
        let mut piv = col;
        for row in (col + 1)..N {
            if m[row][col].abs() > m[piv][col].abs() {
                piv = row;
            }
        }
        m.swap(col, piv);
        if m[col][col].abs() < 1e-300 {
            return None;
        }
        let pivot_row = m[col];
        let denom = pivot_row[col];
        for row in m.iter_mut().skip(col + 1) {
            let f = row[col] / denom;
            for (rk, &pk) in row.iter_mut().zip(pivot_row.iter()).skip(col) {
                *rk -= f * pk;
            }
        }
    }
    // Back-substitution.
    let mut x = [0.0_f64; N];
    for col in (0..N).rev() {
        let mut s = m[col][N];
        for (k, &xk) in x.iter().enumerate().skip(col + 1) {
            s -= m[col][k] * xk;
        }
        x[col] = s / m[col][col];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_conventions::resolve;
    use celnet_core::is_close;
    use celnet_types::{CcyPair, Tenor};

    fn ctx(spot: f64, t: f64) -> MarketContext {
        let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
        MarketContext::new(spot, 0.02, 0.01, t, conv)
    }

    /// The ATM-forward vol is reproduced by every model (each fit pins or matches
    /// the ATM anchor), so a desk switching models still sees its ATM mark.
    #[test]
    fn every_model_reprices_atm() {
        let c = ctx(1.10, 1.0);
        let q = MarketQuotes::three_point(0.10, -0.005, 0.0025);
        let f = c.forward();
        for model in [
            SmileModel::MarketHedge,
            SmileModel::StochasticVol,
            SmileModel::Parametric,
            SmileModel::ParametricSurface,
            SmileModel::ExtendedSurface,
        ] {
            let s = build_model_smile(model, &c, &q).unwrap();
            assert_eq!(s.model(), model);
            let atm = s.implied_vol(f, f, c.t).0;
            assert!(
                is_close(atm, 0.10, 5e-3, 1e-4),
                "{model:?}: ATM vol {atm} must reproduce the 0.10 mark"
            );
        }
    }

    /// Model selection changes the calibrated smile: at the wings the SABR / SVI /
    /// SSVI fits differ measurably from the market-hedge baseline (they are
    /// genuinely different models, not the same numbers relabelled).
    #[test]
    fn model_selection_changes_the_wings() {
        let c = ctx(1.10, 1.0);
        // A pronounced skew so the models' wing extrapolations diverge.
        let q = MarketQuotes::five_point(0.11, -0.02, 0.006, -0.035, 0.012);
        let f = c.forward();

        let baseline = build_model_smile(SmileModel::MarketHedge, &c, &q).unwrap();
        // A deep wing strike beyond the calibrated pillars.
        let k = f * 0.80;
        let base_vol = baseline.implied_vol(k, f, c.t).0;

        for model in [
            SmileModel::StochasticVol,
            SmileModel::Parametric,
            SmileModel::ParametricSurface,
            SmileModel::ExtendedSurface,
        ] {
            let s = build_model_smile(model, &c, &q).unwrap();
            let v = s.implied_vol(k, f, c.t).0;
            assert!(
                v.is_finite() && v > 0.0,
                "{model:?}: finite positive wing vol"
            );
            assert!(
                (v - base_vol).abs() > 1e-4,
                "{model:?}: wing vol {v} must differ from baseline {base_vol}"
            );
        }
    }

    /// The fits reproduce the calibrated 25Δ skew direction: a negative risk
    /// reversal (put-skewed) yields a higher put-wing vol than call-wing vol in
    /// every fitted model.
    #[test]
    fn fitted_models_reproduce_skew_direction() {
        let c = ctx(1.30, 0.5);
        let q = MarketQuotes::three_point(0.12, -0.015, 0.003);
        let f = c.forward();
        let k_put = f * 0.95;
        let k_call = f * 1.05;
        for model in [
            SmileModel::StochasticVol,
            SmileModel::Parametric,
            SmileModel::ParametricSurface,
            SmileModel::ExtendedSurface,
        ] {
            let s = build_model_smile(model, &c, &q).unwrap();
            let put = s.implied_vol(k_put, f, c.t).0;
            let call = s.implied_vol(k_call, f, c.t).0;
            assert!(
                put > call,
                "{model:?}: negative RR ⇒ put-skew (put {put} > call {call})"
            );
        }
    }

    /// The SVI / SSVI fits land butterfly-arbitrage-free (the projection keeps the
    /// slice inside the no-arbitrage box).
    #[test]
    fn fitted_parametric_slices_are_butterfly_free() {
        let c = ctx(1.10, 1.0);
        let q = MarketQuotes::three_point(0.10, -0.006, 0.0025);
        for model in [
            SmileModel::Parametric,
            SmileModel::ParametricSurface,
            SmileModel::ExtendedSurface,
        ] {
            let s = build_model_smile(model, &c, &q).unwrap();
            let slice = match s {
                CalibratedSmile::Parametric(p)
                | CalibratedSmile::ParametricSurface(p)
                | CalibratedSmile::ExtendedSurface(p) => p,
                _ => unreachable!(),
            };
            assert!(
                slice.is_butterfly_free(0.6, 1e-6),
                "{model:?} fitted slice must be butterfly-free"
            );
        }
    }

    /// Calibration is deterministic: the same inputs yield a bit-identical fit.
    #[test]
    fn calibration_is_bit_reproducible() {
        let c = ctx(1.10, 1.0);
        let q = MarketQuotes::three_point(0.10, -0.005, 0.0025);
        for model in [
            SmileModel::StochasticVol,
            SmileModel::Parametric,
            SmileModel::ParametricSurface,
            SmileModel::ExtendedSurface,
        ] {
            let a = build_model_smile(model, &c, &q).unwrap();
            let b = build_model_smile(model, &c, &q).unwrap();
            let f = c.forward();
            for kk in [0.85, 0.95, 1.0, 1.05, 1.15] {
                let strike = f * kk;
                assert_eq!(
                    a.implied_vol(strike, f, c.t).0.to_bits(),
                    b.implied_vol(strike, f, c.t).0.to_bits(),
                    "{model:?}: fit must be bit-identical across runs"
                );
            }
        }
    }
}
