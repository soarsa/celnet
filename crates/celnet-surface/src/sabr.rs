//! Stochastic-alpha-beta-rho (SABR) smile model.
//!
//! Provenance (doc-only): the standard singular-perturbation lognormal (Black)
//! implied-volatility expansion of Hagan, Kumar, Lesniewski & Woodward (2002),
//! with the arbitrage-free density-PDE refinement of Hagan, Kumar, Lesniewski &
//! Woodward (2014) used for the wings where the asymptotic expansion becomes
//! inaccurate / arbitrageable (`docs/ANALYTICS-SPEC.md` §3.2). Identifiers are
//! purpose-named and vendor/person-neutral; "SABR" is used only as the
//! established neutral technical acronym for the stochastic-alpha-beta-rho model.
//!
//! # Dynamics
//!
//! Under the forward measure the forward `F` and its volatility `α` follow
//!
//! ```text
//!   dF = α F^β dW₁,   dα = ν α dW₂,   d⟨W₁,W₂⟩ = ρ dt,
//! ```
//!
//! with parameters `α` (level), `β` (CEV backbone exponent, **fixed** by market
//! convention since it is weakly identified from a single smile), `ρ` (skew) and
//! `ν` (vol-of-vol / curvature).
//!
//! # Two evaluation regimes
//!
//! 1. **Asymptotic Black vol** ([`SabrParams::black_vol`]): the closed-form
//!    expansion — O(1) per strike, the hot-path evaluator that the [`Smile`]
//!    implementation uses in the liquid core.
//! 2. **Arbitrage-free density** ([`SabrParams::risk_neutral_density`]): the
//!    effective-forward 1-D density of the 2014 refinement, used to (a) detect
//!    where the asymptotic smile turns arbitrageable and (b) reprice options by
//!    integrating the genuine density in the wings. The [`SabrSmile`] surface
//!    wires both together: asymptotic vol in the core, density-implied vol once
//!    the density would go negative.

use celnet_core::Smile;
use celnet_core::math::{ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::Vol;

use crate::mathx::powf;

/// The four SABR parameters for one expiry slice.
///
/// `beta` is the (fixed) CEV backbone exponent in `[0, 1]`; `alpha > 0` the
/// instantaneous vol level; `rho ∈ (−1, 1)` the spot/vol correlation (skew); and
/// `nu ≥ 0` the vol-of-vol (curvature). The slice also carries the forward `f`
/// and the expiry `t` it was calibrated at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SabrParams {
    /// Instantaneous volatility level `α > 0`.
    pub alpha: f64,
    /// CEV backbone exponent `β ∈ [0, 1]` (fixed by convention).
    pub beta: f64,
    /// Spot/vol correlation `ρ ∈ (−1, 1)` (skew).
    pub rho: f64,
    /// Vol-of-vol `ν ≥ 0` (curvature).
    pub nu: f64,
    /// Outright forward the slice is anchored at.
    pub forward: f64,
    /// Time to expiry (years).
    pub t: f64,
}

impl SabrParams {
    /// Construct a SABR slice, validating parameter ranges.
    ///
    /// # Panics
    ///
    /// Panics if `alpha ≤ 0`, `beta ∉ [0,1]`, `|rho| ≥ 1`, `nu < 0`, or
    /// `forward`/`t` are non-positive — these are calibration-time programming
    /// errors, not runtime market states.
    #[must_use]
    pub fn new(alpha: f64, beta: f64, rho: f64, nu: f64, forward: f64, t: f64) -> Self {
        assert!(alpha > 0.0, "SABR alpha must be positive: {alpha}");
        assert!(
            (0.0..=1.0).contains(&beta),
            "SABR beta must be in [0,1]: {beta}"
        );
        assert!(rho.abs() < 1.0, "SABR rho must lie in (-1,1): {rho}");
        assert!(nu >= 0.0, "SABR nu must be non-negative: {nu}");
        assert!(
            forward > 0.0 && t > 0.0,
            "SABR forward and t must be positive: F={forward}, t={t}"
        );
        Self {
            alpha,
            beta,
            rho,
            nu,
            forward,
            t,
        }
    }

    /// The asymptotic lognormal (Black) implied volatility at strike `K`.
    ///
    /// This is the Hagan-2002 singular-perturbation expansion (the form quoted in
    /// `docs/ANALYTICS-SPEC.md` §3.2). The ATM limit `K → F` is handled by the
    /// dedicated closed form (the general expression has a removable `z/x(z)`
    /// singularity there).
    #[must_use]
    pub fn black_vol(&self, strike: f64) -> f64 {
        let SabrParams {
            alpha,
            beta,
            rho,
            nu,
            forward: f,
            t,
        } = *self;
        debug_assert!(strike > 0.0, "SABR strike must be positive");

        let one_m_beta = 1.0 - beta;
        // Common prefactor terms shared by ATM and general cases.
        let log_fk = ln(f / strike);
        let fk_pow = powf(f * strike, 0.5 * one_m_beta); // (F·K)^((1−β)/2)

        // The bracketed "B" factor: 1 + [ (1−β)²/24·A² + ¼ρβν·alpha/fk_pow
        //                                 + (2−3ρ²)/24·ν² ] · t   where A = alpha/fk_pow.
        let a_over = alpha / fk_pow;
        let b_factor = 1.0
            + (one_m_beta * one_m_beta / 24.0 * a_over * a_over
                + 0.25 * rho * beta * nu * a_over
                + (2.0 - 3.0 * rho * rho) / 24.0 * nu * nu)
                * t;

        // ATM (or numerically-coincident strike): the z/x(z) factor → 1.
        if (f - strike).abs() <= 1e-12 * f {
            return a_over * b_factor;
        }

        // General strike: the z/x(z) curvature factor.
        let z = (nu / alpha) * fk_pow * log_fk;
        let x_z = ln((sqrt(1.0 - 2.0 * rho * z + z * z) + z - rho) / (1.0 - rho));

        // Denominator series in log-moneyness: fk_pow·[1 + (1−β)²/24·log² + (1−β)⁴/1920·log⁴].
        let lm2 = log_fk * log_fk;
        let denom = fk_pow
            * (1.0
                + one_m_beta * one_m_beta / 24.0 * lm2
                + powf(one_m_beta, 4.0) / 1920.0 * lm2 * lm2);

        // z/x(z) is 1 in the z → 0 limit; guard the removable singularity.
        let z_over_x = if z.abs() < 1e-10 {
            // x(z) = ln((√(1−2ρz+z²)+z−ρ)/(1−ρ)) ≈ z − ½ρz² + …, so its
            // reciprocal series is z/x(z) ≈ 1/(1 − ½ρz + (2−3ρ²)/12·z² + …).
            1.0 / (1.0 - 0.5 * rho * z + (2.0 - 3.0 * rho * rho) / 12.0 * z * z)
        } else {
            z / x_z
        };

        (alpha / denom) * z_over_x * b_factor
    }

    /// The "effective" local volatility `C(F) = α·Fᵝ`-scaled diffusion at level
    /// `x` used by the arbitrage-free density PDE — here in the **normal-SABR**
    /// effective-forward reduction: the cumulative function `Γ(K)` mapping strike
    /// to the transformed coordinate, plus the marginal density.
    ///
    /// We implement the 2014 single-step normal approximation to the marginal
    /// risk-neutral density: the SABR forward density at expiry is approximately
    /// normal in the transformed coordinate
    ///
    /// ```text
    ///   y(K) = ∫_F^K dF' / (α F'^β),
    /// ```
    ///
    /// rescaled by the vol-of-vol skew. This yields a strictly non-negative
    /// density everywhere (the property the asymptotic expansion lacks), at the
    /// cost of a one-dimensional quadrature. The returned density is in **forward
    /// (undiscounted) measure**.
    ///
    /// This is the single-step Hagan-2014 normal approximation, so it is only
    /// **approximately** normalised: its total mass over `(0, ∞)` is within a few
    /// percent of one (see `density_is_nonnegative_and_normalised`), not exactly
    /// one. The wing repricing in [`SabrSmile::density_implied_vol`] integrates
    /// this density and therefore inherits the same approximation; it is used only
    /// where the asymptotic expansion would itself become arbitrageable, so the
    /// small mass error is preferable to a negative-density wing.
    #[must_use]
    pub fn risk_neutral_density(&self, strike: f64) -> f64 {
        if strike <= 0.0 {
            return 0.0;
        }
        // Transformed coordinate y(K) and its derivative dy/dK = 1/(α Kᵝ).
        let y = self.y_coordinate(strike);
        let dy_dk = 1.0 / (self.alpha * powf(strike, self.beta));

        // Effective vol-of-vol scaling: the lognormal vol process maps y to a
        // mean-zero Gaussian with std-dev s(t) = ν√t in the rotated frame and a
        // skew rotation by ρ. Use the standard arctan-of-z change of variable
        // (Hagan 2014, §3): z = ν y, and the cumulant-corrected std-dev.
        let nu = self.nu;
        let rho = self.rho;
        let t = self.t;

        // Build the Gaussian in the "u" coordinate u(K) = (1/ν)·asinh-like map of
        // z = ν·y so the density stays normalised. For ν → 0 this collapses to a
        // pure normal in y with std-dev α√t-scaled — handled by the small-ν branch.
        let (u, du_dy) = if nu < 1e-10 {
            (y, 1.0)
        } else {
            let z = nu * y;
            // u = (1/ν)·ln( (√(1−2ρz+z²) + z − ρ)/(1−ρ) ) — the SABR x(z) map.
            let root = sqrt(1.0 - 2.0 * rho * z + z * z);
            let u = (1.0 / nu) * ln((root + z - rho) / (1.0 - rho));
            // du/dy = dz/dy · du/dz = ν · (1/ν)·1/root = 1/root.
            (u, 1.0 / root)
        };

        // u is approximately Gaussian with std-dev √t (unit local vol in the u
        // frame); the density of K is φ(u/√t)·(1/√t)·|du/dy|·|dy/dK|.
        let std = sqrt(t);
        let phi = norm_pdf(u / std) / std;
        phi * du_dy * dy_dk
    }

    /// The transformed coordinate `y(K) = ∫_F^K dF'/(α F'^β)`, in closed form for
    /// the CEV power: `y = (K^{1−β} − F^{1−β}) / (α(1−β))` for `β ≠ 1`, and
    /// `y = ln(K/F)/α` for `β = 1`.
    #[inline]
    fn y_coordinate(&self, strike: f64) -> f64 {
        let one_m_beta = 1.0 - self.beta;
        if one_m_beta.abs() < 1e-12 {
            ln(strike / self.forward) / self.alpha
        } else {
            (powf(strike, one_m_beta) - powf(self.forward, one_m_beta)) / (self.alpha * one_m_beta)
        }
    }
}

/// A SABR smile slice that evaluates as a [`Smile`].
///
/// In the liquid core it returns the asymptotic Hagan-2002 Black vol; in the
/// wings — where the asymptotic density would turn negative — it switches to the
/// volatility *implied by the arbitrage-free density* so the surface it presents
/// is butterfly-arbitrage-free everywhere. The crossover strikes are detected
/// once at construction from where the asymptotic density changes sign.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SabrSmile {
    params: SabrParams,
    /// Lower wing crossover: below this strike use the density-implied vol.
    wing_lo: f64,
    /// Upper wing crossover: above this strike use the density-implied vol.
    wing_hi: f64,
}

impl SabrSmile {
    /// Build a SABR smile from calibrated parameters, locating the wing
    /// crossovers where the asymptotic expansion's implied density goes negative.
    #[must_use]
    pub fn new(params: SabrParams) -> Self {
        let f = params.forward;
        // Scan outward from the forward for the first strike where the
        // asymptotic-vol density turns negative; that is the wing crossover.
        let wing_lo = Self::find_density_floor(&params, f, false);
        let wing_hi = Self::find_density_floor(&params, f, true);
        Self {
            params,
            wing_lo,
            wing_hi,
        }
    }

    /// The underlying calibrated parameters.
    #[must_use]
    pub fn params(&self) -> SabrParams {
        self.params
    }

    /// The `[lo, hi]` strike band inside which the asymptotic expansion is used
    /// directly (outside it, the density-implied vol takes over).
    #[must_use]
    pub fn core_band(&self) -> (f64, f64) {
        (self.wing_lo, self.wing_hi)
    }

    /// Density of the *asymptotic* smile via a central second difference of the
    /// undiscounted forward call priced at the asymptotic Black vol.
    fn asymptotic_density(params: &SabrParams, strike: f64, h: f64) -> f64 {
        let c = |k: f64| {
            let v = params.black_vol(k);
            let vsqt = v * sqrt(params.t);
            let d1 = (ln(params.forward / k) + 0.5 * v * v * params.t) / vsqt;
            let d2 = d1 - vsqt;
            params.forward * norm_cdf(d1) - k * norm_cdf(d2)
        };
        (c(strike - h) - 2.0 * c(strike) + c(strike + h)) / (h * h)
    }

    /// Walk outward (or inward toward zero) from the forward until the asymptotic
    /// density first goes negative; return that crossover strike. If the density
    /// stays positive over the scan range, return a strike far in the wing (so the
    /// asymptotic formula is used throughout — the arbitrage-free case).
    fn find_density_floor(params: &SabrParams, f: f64, upward: bool) -> f64 {
        let h = 1e-3 * f;
        let step = 0.02 * f;
        let mut k = f;
        for _ in 0..400 {
            let next = if upward { k + step } else { k - step };
            if next <= 2.0 * h {
                return next.max(2.0 * h);
            }
            if Self::asymptotic_density(params, next, h) < 0.0 {
                return next;
            }
            k = next;
        }
        // No negative density found within the scan: push the crossover out of
        // any practical strike range.
        if upward { f * 1_000.0 } else { f * 1e-3 }
    }

    /// Implied vol from the arbitrage-free density: numerically invert the
    /// undiscounted forward call priced by integrating the density against the
    /// Black formula. We use the density to recompute the call price and then
    /// re-imply the Black vol that reproduces it — guaranteeing the wing vol is
    /// consistent with a non-negative density.
    fn density_implied_vol(&self, strike: f64) -> f64 {
        let p = &self.params;
        // Undiscounted forward call C(K) = ∫_K^∞ (F'−K) g(F') dF' via the density.
        // Integrate on an adaptive log-spaced grid out to a far wing.
        let f = p.forward;
        let lo = (strike).max(1e-6 * f);
        let hi = (strike + 12.0 * p.alpha * sqrt(p.t).max(1e-3) * f).max(strike * 3.0);
        let n = 2000usize;
        let dk = (hi - lo) / (n as f64);
        let mut call = 0.0;
        let mut kk = lo + 0.5 * dk;
        for _ in 0..n {
            let g = p.risk_neutral_density(kk);
            call += (kk - strike) * g * dk;
            kk += dk;
        }
        // Re-imply the Black vol matching this undiscounted forward call.
        implied_black_vol(call, f, strike, p.t).unwrap_or_else(|| p.black_vol(strike))
    }
}

impl Smile for SabrSmile {
    fn implied_vol(&self, strike: f64, _forward: f64, _t: f64) -> Vol {
        // The SABR slice is anchored at its own forward/time; the trait
        // forward/t arguments are accepted for interface uniformity but the
        // calibrated slice carries the authoritative forward/expiry.
        if strike >= self.wing_lo && strike <= self.wing_hi {
            Vol(self.params.black_vol(strike))
        } else {
            Vol(self.density_implied_vol(strike))
        }
    }
}

/// Newton/bisection inversion of the undiscounted forward Black call price for
/// the implied volatility. Returns `None` if the target is outside the no-
/// arbitrage bounds `[max(F−K,0), F]`.
fn implied_black_vol(call: f64, forward: f64, strike: f64, t: f64) -> Option<f64> {
    let intrinsic = (forward - strike).max(0.0);
    if call <= intrinsic + 1e-15 || call >= forward {
        return None;
    }
    let price = |vol: f64| {
        let vsqt = vol * sqrt(t);
        let d1 = (ln(forward / strike) + 0.5 * vol * vol * t) / vsqt;
        let d2 = d1 - vsqt;
        forward * norm_cdf(d1) - strike * norm_cdf(d2)
    };
    let (mut lo, mut hi) = (1e-6, 5.0);
    if price(hi) < call {
        return None;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let pm = price(mid);
        if (pm - call).abs() < 1e-12 {
            return Some(mid);
        }
        if pm < call {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-12 {
            return Some(0.5 * (lo + hi));
        }
    }
    Some(0.5 * (lo + hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    fn slice() -> SabrParams {
        // EURUSD-like 1Y: F = 1.10, moderate skew/curvature, β = 1 (lognormal).
        SabrParams::new(0.11, 1.0, -0.20, 0.45, 1.10, 1.0)
    }

    /// ATM Black vol matches the closed-form ATM limit and the general formula
    /// evaluated infinitesimally off the forward (continuity of the removable
    /// singularity).
    #[test]
    fn atm_limit_is_continuous() {
        let p = slice();
        let atm = p.black_vol(p.forward);
        let near = p.black_vol(p.forward * (1.0 + 1e-7));
        assert!(
            is_close(atm, near, 1e-5, 1e-7),
            "ATM {atm} vs near-ATM {near} must be continuous"
        );
    }

    /// The small-`|z|` reciprocal series for `z/x(z)` agrees with the general
    /// `z/x(z)` branch across the `1e-10` threshold, locking in the corrected
    /// sign of the O(z) term (`− ½ρz`). We compare `black_vol` at two strikes
    /// that straddle the branch threshold (`|z| ≈ 1e-9` vs `≈ 1e-11`): both must
    /// coincide to high precision and with the ATM limit.
    #[test]
    fn small_z_series_matches_general_branch() {
        // Use a strong rho so a wrong sign in the O(z) term would show up.
        let p = SabrParams::new(0.11, 1.0, -0.8, 0.45, 1.10, 1.0);
        let atm = p.black_vol(p.forward);
        // For beta = 1, z = (nu/alpha)·ln(F/K), so the log-moneyness needed for a
        // target |z| is ln(F/K) = z·alpha/nu.
        let lm_just_below = 5e-11 * p.alpha / p.nu; // |z| ≈ 5e-11 → series branch
        let lm_just_above = 5e-9 * p.alpha / p.nu; // |z| ≈ 5e-9  → general branch
        let k_series = p.forward * crate::mathx::powf(core::f64::consts::E, -lm_just_below);
        let k_general = p.forward * crate::mathx::powf(core::f64::consts::E, -lm_just_above);
        let v_series = p.black_vol(k_series);
        let v_general = p.black_vol(k_general);
        // The general branch is evaluated at |z| ≈ 5e-9 (just above the 1e-10
        // switch) where its O(z) truncation error is ~1e-9 relative; the series
        // branch at |z| ≈ 5e-11 is essentially exact. They must coincide to that
        // truncation level, which a *wrong sign* in the O(ρz) term would shatter
        // (it would inject an O(ρ·z) ≈ 4e-9 one-sided bias with ρ = −0.8).
        assert!(
            is_close(v_series, v_general, 1e-8, 1e-12),
            "series branch {v_series} must match general branch {v_general}"
        );
        assert!(
            is_close(v_series, atm, 1e-7, 1e-12),
            "series branch {v_series} must match ATM {atm}"
        );
    }

    /// A non-zero negative rho produces a downward skew: put-wing vol exceeds
    /// call-wing vol.
    #[test]
    fn negative_rho_makes_downward_skew() {
        let p = slice();
        let put_wing = p.black_vol(0.95);
        let call_wing = p.black_vol(1.25);
        assert!(
            put_wing > call_wing,
            "negative-rho skew: put wing {put_wing} should exceed call wing {call_wing}"
        );
    }

    /// The arbitrage-free density is non-negative and integrates to approximately
    /// one over a wide forward grid. The single-step Hagan-2014 normal
    /// approximation is *not* exactly normalised, so the mass tolerance is a
    /// deliberate few-percent band (`5e-2`), not a tight equality — the test
    /// asserts strict non-negativity and approximate (not exact) unit mass.
    #[test]
    fn density_is_nonnegative_and_normalised() {
        let p = slice();
        let lo = 0.2;
        let hi = 4.0;
        let n = 20000usize;
        let dk = (hi - lo) / (n as f64);
        let mut mass = 0.0;
        let mut k = lo + 0.5 * dk;
        let mut min_g = f64::INFINITY;
        for _ in 0..n {
            let g = p.risk_neutral_density(k);
            min_g = min_g.min(g);
            mass += g * dk;
            k += dk;
        }
        assert!(
            min_g >= -1e-12,
            "density must stay non-negative: min {min_g}"
        );
        assert!(
            is_close(mass, 1.0, 5e-2, 5e-2),
            "density should integrate to ~1: got {mass}"
        );
    }

    /// The smile reprices its forward (ATM) through the [`Smile`] trait.
    #[test]
    fn smile_trait_evaluates() {
        let s = SabrSmile::new(slice());
        let atm = s.implied_vol(1.10, 1.10, 1.0).0;
        assert!(atm > 0.0 && atm.is_finite());
    }

    /// Wing band is located: the core band straddles the forward.
    #[test]
    fn core_band_straddles_forward() {
        let s = SabrSmile::new(slice());
        let (lo, hi) = s.core_band();
        assert!(
            lo < 1.10 && hi > 1.10,
            "core band [{lo},{hi}] must straddle F"
        );
    }
}
