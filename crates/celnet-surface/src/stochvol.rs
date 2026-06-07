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
//! 1. **Asymptotic Black vol** ([`StochasticVolParams::black_vol`]): the closed-form
//!    expansion — O(1) per strike, the hot-path evaluator that the [`Smile`]
//!    implementation uses in the liquid core.
//! 2. **Arbitrage-free density** ([`StochasticVolParams::risk_neutral_density`]): the
//!    effective-forward 1-D density of the 2014 refinement, used to (a) detect
//!    where the asymptotic smile turns arbitrageable and (b) reprice options by
//!    integrating the genuine density in the wings. The [`StochasticVolSmile`] surface
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
pub struct StochasticVolParams {
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

impl StochasticVolParams {
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
        let StochasticVolParams {
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
    /// This is the single-step Hagan-2014 normal approximation, so the **raw**
    /// density is only *approximately* normalised: its total mass over `(0, ∞)`
    /// is within a few percent of one, and its mean differs from the forward by a
    /// comparable amount. Used raw, that bias propagates into wing call prices and
    /// can break the martingale (forward) constraint — i.e. it is *not* by itself
    /// arbitrage-free.
    ///
    /// The wing repricing in [`StochasticVolSmile::density_implied_vol`] therefore does
    /// **not** integrate this raw density. It builds a [`WingDensity`] that
    /// renormalises to **exact unit mass** and **rescales the forward coordinate
    /// so the density mean equals `F` exactly** (the martingale constraint). A
    /// non-negative density with unit mass and mean `F` produces European call
    /// prices that are convex and monotone in `K` and lie inside the no-arbitrage
    /// bounds `[max(F−K,0), F]` — i.e. a genuinely butterfly-arbitrage-free wing
    /// (Breeden-Litzenberger), removing the systematic mass/mean bias of the raw
    /// single-step form. See [`StochasticVolParams::wing_density`].
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

    /// Build a normalised, martingale-corrected [`WingDensity`] from the raw
    /// single-step density over a fixed deterministic quadrature grid.
    ///
    /// The grid spans `(0, K_max]` with `K_max` a wide multiple of the forward
    /// (covering essentially all the mass). The raw density is sampled, then the
    /// [`WingDensity`] renormalises to exact unit mass and rescales the strike
    /// coordinate so the density mean equals the forward `F` exactly. The result
    /// is the genuinely arbitrage-free wing density the smile uses for repricing.
    #[must_use]
    pub fn wing_density(&self) -> WingDensity {
        WingDensity::from_params(self)
    }
}

/// A normalised, martingale-corrected SABR wing density.
///
/// Built from [`StochasticVolParams::risk_neutral_density`] on a fixed grid, then
/// corrected so that, in **forward (undiscounted) measure**,
///
/// ```text
///   ∫₀^∞ g(F') dF' = 1        (unit mass)
///   ∫₀^∞ F' g(F') dF' = F     (martingale / forward constraint)
/// ```
///
/// both hold **exactly** (to quadrature precision). The first correction is a
/// plain renormalisation `g/M`; the second is a multiplicative rescale of the
/// forward coordinate by `λ = F/μ` (where `μ` is the post-normalisation mean),
/// which preserves non-negativity and unit mass while moving the mean onto `F`.
/// A non-negative density satisfying both constraints prices European calls free
/// of butterfly and (forward) put-call-parity arbitrage.
#[derive(Debug, Clone)]
pub struct WingDensity {
    /// Lower grid edge (just above 0).
    lo: f64,
    /// Grid step.
    dk: f64,
    /// Normalised density samples at cell midpoints (mass `g·dk` sums to 1).
    g: Vec<f64>,
    /// Forward-coordinate rescale `λ = F/μ` applied at pricing time.
    lambda: f64,
}

impl WingDensity {
    /// Number of quadrature cells used to discretise the wing density. Fixed and
    /// deterministic (no adaptivity) so the wing repricing is reproducible.
    const CELLS: usize = 4096;

    fn from_params(p: &StochasticVolParams) -> Self {
        // A wide support: the single-step density is centred near F with spread
        // set by α√t in forward terms; 16 std-devs of headroom captures the mass.
        let f = p.forward;
        let spread = (p.alpha * sqrt(p.t)).max(0.05) * f;
        let lo = (f - 16.0 * spread).max(1e-6 * f);
        let hi = (f + 16.0 * spread).max(f * 3.0);
        let n = Self::CELLS;
        let dk = (hi - lo) / (n as f64);

        // Sample the raw density at cell midpoints.
        let mut g = Vec::with_capacity(n);
        let mut mass = 0.0;
        let mut k = lo + 0.5 * dk;
        for _ in 0..n {
            let gi = p.risk_neutral_density(k).max(0.0);
            mass += gi * dk;
            g.push(gi);
            k += dk;
        }
        // Renormalise to exact unit mass.
        debug_assert!(mass > 0.0, "wing density mass must be positive");
        let inv_mass = 1.0 / mass;
        for gi in &mut g {
            *gi *= inv_mass;
        }
        // Post-normalisation mean μ = Σ k·g·dk; rescale coordinate by λ = F/μ so
        // the corrected mean equals F exactly (martingale constraint).
        let mut mean = 0.0;
        let mut k = lo + 0.5 * dk;
        for &gi in &g {
            mean += k * gi * dk;
            k += dk;
        }
        let lambda = if mean > 0.0 { f / mean } else { 1.0 };
        Self { lo, dk, g, lambda }
    }

    /// Total probability mass (`= 1` to quadrature precision).
    #[must_use]
    pub fn total_mass(&self) -> f64 {
        let mut m = 0.0;
        for &gi in &self.g {
            m += gi * self.dk;
        }
        m
    }

    /// Density mean in the rescaled coordinate (`= F` to quadrature precision).
    #[must_use]
    pub fn mean(&self) -> f64 {
        let mut mu = 0.0;
        let mut k = self.lo + 0.5 * self.dk;
        for &gi in &self.g {
            mu += (self.lambda * k) * gi * self.dk;
            k += self.dk;
        }
        mu
    }

    /// The undiscounted forward call price `C(K) = ∫ max(λ·F' − K, 0) g(F') dF'`,
    /// using the renormalised, mean-`F` density. Arbitrage-free by construction.
    #[must_use]
    pub fn forward_call(&self, strike: f64) -> f64 {
        let mut call = 0.0;
        let mut k = self.lo + 0.5 * self.dk;
        for &gi in &self.g {
            let payoff = (self.lambda * k - strike).max(0.0);
            call += payoff * gi * self.dk;
            k += self.dk;
        }
        call
    }
}

/// A SABR smile slice that evaluates as a [`Smile`].
///
/// In the liquid core it returns the asymptotic Hagan-2002 Black vol; in the
/// wings — where the asymptotic density would turn negative — it switches to the
/// volatility *implied by the arbitrage-free density* so the surface it presents
/// is butterfly-arbitrage-free everywhere. The crossover strikes are detected
/// once at construction from where the asymptotic density changes sign.
#[derive(Debug, Clone)]
pub struct StochasticVolSmile {
    params: StochasticVolParams,
    /// Lower wing crossover: below this strike use the density-implied vol.
    wing_lo: f64,
    /// Upper wing crossover: above this strike use the density-implied vol.
    wing_hi: f64,
    /// The normalised, martingale-corrected wing density, built once. Only the
    /// wings consult it; the liquid core uses the asymptotic Black vol.
    wing_density: WingDensity,
}

impl StochasticVolSmile {
    /// Build a SABR smile from calibrated parameters, locating the wing
    /// crossovers where the asymptotic expansion's implied density goes negative
    /// and constructing the arbitrage-free wing density once up front.
    #[must_use]
    pub fn new(params: StochasticVolParams) -> Self {
        let f = params.forward;
        // Scan outward from the forward for the first strike where the
        // asymptotic-vol density turns negative; that is the wing crossover.
        let wing_lo = Self::find_density_floor(&params, f, false);
        let wing_hi = Self::find_density_floor(&params, f, true);
        let wing_density = params.wing_density();
        Self {
            params,
            wing_lo,
            wing_hi,
            wing_density,
        }
    }

    /// The underlying calibrated parameters.
    #[must_use]
    pub fn params(&self) -> StochasticVolParams {
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
    fn asymptotic_density(params: &StochasticVolParams, strike: f64, h: f64) -> f64 {
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
    fn find_density_floor(params: &StochasticVolParams, f: f64, upward: bool) -> f64 {
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

    /// Implied vol from the arbitrage-free wing density: price the undiscounted
    /// forward call against the **normalised, mean-`F`** [`WingDensity`] and
    /// re-imply the Black vol that reproduces it.
    ///
    /// Because the wing density integrates to one and has mean exactly `F`, the
    /// repriced call lies inside the no-arbitrage bounds `[max(F−K,0), F]`, so the
    /// re-implied vol is a genuine, arbitrage-free wing vol — free of the
    /// systematic mass/mean bias of the raw single-step density.
    fn density_implied_vol(&self, strike: f64) -> f64 {
        let p = &self.params;
        let call = self.wing_density.forward_call(strike);
        implied_black_vol(call, p.forward, strike, p.t).unwrap_or_else(|| p.black_vol(strike))
    }
}

impl Smile for StochasticVolSmile {
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

    fn slice() -> StochasticVolParams {
        // EURUSD-like 1Y: F = 1.10, moderate skew/curvature, β = 1 (lognormal).
        StochasticVolParams::new(0.11, 1.0, -0.20, 0.45, 1.10, 1.0)
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
        let p = StochasticVolParams::new(0.11, 1.0, -0.8, 0.45, 1.10, 1.0);
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

    /// The *raw* single-step density is non-negative everywhere (the property
    /// the asymptotic expansion lacks), but is only approximately normalised — so
    /// we assert strict non-negativity and an approximate-mass band only. The
    /// genuinely arbitrage-free object is the [`WingDensity`] (next test).
    #[test]
    fn raw_density_is_nonnegative() {
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
            "raw density must stay non-negative: min {min_g}"
        );
        // The raw form is only ~normalised — a few-percent band, NOT a tight
        // equality. This is the bias the WingDensity removes.
        assert!(
            is_close(mass, 1.0, 1e-1, 1e-1),
            "raw density mass should be order one: got {mass}"
        );
    }

    /// The corrected [`WingDensity`] integrates to **exactly** one and has mean
    /// **exactly** the forward `F` (the martingale constraint) — to quadrature
    /// precision. This is the regression that pins down the arbitrage-free fix:
    /// the raw single-step density satisfied neither tightly.
    #[test]
    fn wing_density_has_unit_mass_and_mean_forward() {
        for p in [
            slice(),
            StochasticVolParams::new(0.18, 0.7, -0.45, 0.6, 1.35, 0.5),
            StochasticVolParams::new(0.09, 1.0, 0.30, 0.30, 0.80, 2.0),
        ] {
            let wd = p.wing_density();
            assert!(
                is_close(wd.total_mass(), 1.0, 1e-10, 1e-10),
                "wing density mass must be exactly 1: got {}",
                wd.total_mass()
            );
            assert!(
                is_close(wd.mean(), p.forward, 1e-8, 1e-10),
                "wing density mean must equal forward {}: got {}",
                p.forward,
                wd.mean()
            );
        }
    }

    /// The wing call prices implied by the corrected density are arbitrage-free:
    /// inside the no-arbitrage bounds `[max(F−K,0), F]`, monotone-decreasing and
    /// convex in `K`. A biased (un-normalised, wrong-mean) density would breach
    /// these — this is the genuine arbitrage-freeness the audit demanded.
    #[test]
    fn wing_density_call_prices_are_arbitrage_free() {
        let p = StochasticVolParams::new(0.16, 0.8, -0.40, 0.7, 1.20, 0.75);
        let wd = p.wing_density();
        let f = p.forward;
        // Strikes spanning both wings.
        let ks: Vec<f64> = (1..=40)
            .map(|i| 0.4 + (2.4 - 0.4) * (i as f64) / 40.0)
            .collect();
        let mut prev_c = f64::INFINITY;
        for w in ks.windows(3) {
            let (k0, k1, k2) = (w[0], w[1], w[2]);
            let c0 = wd.forward_call(k0);
            let c1 = wd.forward_call(k1);
            let c2 = wd.forward_call(k2);
            // No-arbitrage bounds.
            assert!(
                c1 >= (f - k1).max(0.0) - 1e-9 && c1 <= f + 1e-9,
                "call({k1})={c1} must lie in [max(F-K,0), F]"
            );
            // Monotone decreasing in K.
            assert!(c1 <= prev_c + 1e-9, "call must be non-increasing in K");
            prev_c = c1;
            // Convexity (butterfly ≥ 0): c0 - 2 c1 + c2 ≥ 0.
            assert!(
                c0 - 2.0 * c1 + c2 >= -1e-7,
                "call must be convex in K (butterfly ≥ 0): {}",
                c0 - 2.0 * c1 + c2
            );
        }
    }

    /// The smile reprices its forward (ATM) through the [`Smile`] trait.
    #[test]
    fn smile_trait_evaluates() {
        let s = StochasticVolSmile::new(slice());
        let atm = s.implied_vol(1.10, 1.10, 1.0).0;
        assert!(atm > 0.0 && atm.is_finite());
    }

    /// Wing band is located: the core band straddles the forward.
    #[test]
    fn core_band_straddles_forward() {
        let s = StochasticVolSmile::new(slice());
        let (lo, hi) = s.core_band();
        assert!(
            lo < 1.10 && hi > 1.10,
            "core band [{lo},{hi}] must straddle F"
        );
    }

    /// The `params()` accessor returns exactly the calibrated slice the smile was
    /// constructed from (the read-back the surface layer uses to recover the
    /// underlying SABR parameters).
    #[test]
    fn params_accessor_round_trips() {
        let p = slice();
        let s = StochasticVolSmile::new(p);
        assert_eq!(s.params(), p);
    }

    /// The risk-neutral density is identically zero for a non-positive strike
    /// (out of the forward's positive support) — the guard clause at the top of
    /// `risk_neutral_density`.
    #[test]
    fn density_is_zero_at_non_positive_strike() {
        let p = slice();
        assert_eq!(p.risk_neutral_density(0.0), 0.0);
        assert_eq!(p.risk_neutral_density(-1.0), 0.0);
    }

    /// In the `ν → 0` (no vol-of-vol) limit the density reduces to the pure
    /// transformed-Gaussian small-ν branch: u = y, du/dy = 1, so the density is
    /// `φ(y/√t)·(1/√t)·(1/(α Kᵝ))`. INDEPENDENT ORACLE: re-derive that closed
    /// form in-test (no call back into `risk_neutral_density`) and require an
    /// exact match. This pins the small-ν branch (`nu < 1e-10`).
    #[test]
    fn small_nu_density_matches_transformed_gaussian_oracle() {
        // ν below the 1e-10 switch → small-ν branch.
        let p = StochasticVolParams::new(0.12, 1.0, -0.2, 1e-12, 1.10, 1.0);
        for &k in &[0.7, 0.95, 1.10, 1.30, 1.6] {
            let got = p.risk_neutral_density(k);
            // y(K) for β = 1: ln(K/F)/α.
            let y = (k / p.forward).ln() / p.alpha;
            let std = p.t.sqrt();
            let phi = norm_pdf(y / std) / std;
            let dy_dk = 1.0 / (p.alpha * powf(k, p.beta));
            let expect = phi * 1.0 * dy_dk; // du_dy = 1 in the small-ν branch
            assert!(
                is_close(got, expect, 0.0, 0.0),
                "small-ν density {got} at K={k} must equal the Gaussian oracle {expect} exactly"
            );
        }
    }

    /// A SABR slice whose asymptotic expansion turns arbitrageable on BOTH wings
    /// produces a finite two-sided core band, and outside it the smile switches
    /// to the arbitrage-free density-implied vol. This exercises the wing branch
    /// of `implied_vol`, `density_implied_vol`, and `find_density_floor`'s
    /// negative-density crossover. INDEPENDENT CHECKS: (1) the band is genuinely
    /// finite and two-sided (a crossover was found in each direction, not the
    /// far-wing fallback); (2) inside the band the smile equals the raw
    /// asymptotic Black vol; (3) outside the band it equals the density-implied
    /// vol — which differs from the asymptotic vol there (the wing actually
    /// switches model); (4) the density-implied wing vol is finite and positive.
    #[test]
    fn wing_switches_to_density_implied_vol_outside_the_core_band() {
        // Strong curvature, long tenor: the asymptotic density goes negative on
        // both wings within scan range, so both crossovers are finite.
        let p = StochasticVolParams::new(0.25, 1.0, 0.0, 2.5, 1.10, 2.0);
        let s = StochasticVolSmile::new(p);
        let (lo, hi) = s.core_band();
        // (1) finite two-sided band well inside the far-wing fallbacks (F*1e-3, F*1e3).
        assert!(
            lo > p.forward * 1e-3 && hi < p.forward * 1e3 && lo < p.forward && hi > p.forward,
            "expected a finite two-sided crossover band, got [{lo},{hi}]"
        );

        // (2) inside the band → asymptotic Black vol exactly.
        let k_core = p.forward;
        assert!(is_close(
            s.implied_vol(k_core, p.forward, p.t).0,
            p.black_vol(k_core),
            0.0,
            0.0
        ));

        // (3) below the lower crossover → density-implied vol, which differs from
        // the asymptotic vol there (the wing genuinely switches model).
        let k_put = lo * 0.85;
        let wing_vol = s.implied_vol(k_put, p.forward, p.t).0;
        let asym_vol = p.black_vol(k_put);
        assert!(
            wing_vol.is_finite() && wing_vol > 0.0,
            "wing vol must be sane"
        );
        assert!(
            (wing_vol - asym_vol).abs() > 1e-6,
            "below the crossover the density-implied vol {wing_vol} must differ \
             from the asymptotic vol {asym_vol}"
        );

        // (4) above the upper crossover → density-implied vol, finite/positive.
        let k_call = hi * 1.15;
        let up_vol = s.implied_vol(k_call, p.forward, p.t).0;
        assert!(
            up_vol.is_finite() && up_vol > 0.0,
            "upper wing vol must be sane"
        );
    }

    /// `implied_black_vol` is the bisection inverse of the undiscounted forward
    /// Black call. INDEPENDENT ORACLE: forward-price a call at a KNOWN vol, invert
    /// it, and require the recovered vol to round-trip; then re-price at the
    /// recovered vol and require it to reproduce the target call. Also pins the
    /// three no-arbitrage rejections (`None`): call ≤ intrinsic, call ≥ forward,
    /// and an unreachable-by-vol target above `price(hi)`.
    #[test]
    fn implied_black_vol_inverts_the_forward_call_oracle() {
        let f = 1.10_f64;
        let k = 1.25_f64;
        let t = 0.8_f64;
        let true_vol = 0.27_f64;
        // Independent forward Black call at the known vol.
        let fwd_call = |vol: f64| {
            let vsqt = vol * t.sqrt();
            let d1 = ((f / k).ln() + 0.5 * vol * vol * t) / vsqt;
            let d2 = d1 - vsqt;
            f * norm_cdf(d1) - k * norm_cdf(d2)
        };
        let target = fwd_call(true_vol);

        let recovered = implied_black_vol(target, f, k, t).expect("call is in-bounds");
        assert!(
            is_close(recovered, true_vol, 1e-6, 1e-8),
            "recovered vol {recovered} must round-trip to {true_vol}"
        );
        // Re-price at the recovered vol → reproduces the target call.
        assert!(
            is_close(fwd_call(recovered), target, 1e-9, 1e-11),
            "re-priced call {} must reproduce target {target}",
            fwd_call(recovered)
        );

        // No-arbitrage rejections (all return None):
        let intrinsic = (f - k).max(0.0); // = 0 here (OTM call)
        // call ≤ intrinsic + 1e-15.
        assert!(implied_black_vol(intrinsic, f, k, t).is_none());
        // call ≥ forward.
        assert!(implied_black_vol(f, f, k, t).is_none());
        assert!(implied_black_vol(f + 0.01, f, k, t).is_none());
        // A target between price(hi=5.0) and forward is unreachable by any vol
        // ≤ 5.0 → None via the `price(hi) < call` guard. For an ITM-ish strike,
        // price(5.0) saturates below F; pick a call just under F but above it.
        let k_itm = 0.90;
        let cap = {
            // price(hi) for the inverter's hi = 5.0
            let vol = 5.0;
            let vsqt = vol * t.sqrt();
            let d1 = ((f / k_itm).ln() + 0.5 * vol * vol * t) / vsqt;
            let d2 = d1 - vsqt;
            f * norm_cdf(d1) - k_itm * norm_cdf(d2)
        };
        // A target strictly between price(hi) and the forward is unreachable.
        let unreachable = 0.5 * (cap + f);
        if cap < f {
            assert!(
                implied_black_vol(unreachable, f, k_itm, t).is_none(),
                "a call above price(vol=5) but below F must be unreachable (None)"
            );
        }
    }

    /// When the asymptotic density never goes negative on the upward scan, the
    /// crossover finder returns the far-wing fallback (F·1000), so the smile uses
    /// the asymptotic vol throughout any practical upper strike range — the
    /// arbitrage-free-asymptotic case. This pins the no-negative fallback line in
    /// `find_density_floor`. (A mild, low-vol-of-vol slice keeps the upper
    /// asymptotic density positive across the whole 400-step scan.)
    #[test]
    fn benign_slice_pushes_upper_crossover_to_far_wing() {
        // Mild slice with small vol-of-vol: no negative density upward within scan.
        let p = StochasticVolParams::new(0.10, 1.0, -0.05, 0.10, 1.10, 1.0);
        let s = StochasticVolSmile::new(p);
        let (_lo, hi) = s.core_band();
        assert!(
            is_close(hi, p.forward * 1_000.0, 0.0, 0.0),
            "benign slice should push the upper crossover to the far-wing fallback \
             F·1000, got {hi}"
        );
        // A practical upper-wing strike is inside the (huge) band → asymptotic vol.
        let k = 1.5;
        assert!(is_close(
            s.implied_vol(k, p.forward, p.t).0,
            p.black_vol(k),
            0.0,
            0.0
        ));
    }
}
