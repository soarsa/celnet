//! Surface stochastic-volatility-inspired (SSVI) parameterization.
//!
//! Provenance (doc-only): the surface SVI of Gatheral & Jacquier (2014), the
//! single-surface form parametrised by the ATM total-variance term structure
//! `θ_t`, a constant correlation `ρ`, and a curvature function `φ(θ)`
//! (`docs/ANALYTICS-SPEC.md` §3.3). It carries **explicit closed-form sufficient
//! conditions** for the absence of butterfly and calendar-spread arbitrage across
//! the whole surface — the preferred globally-arbitrage-free production surface
//! for local-vol / exotics stripping. "SSVI" is the established neutral acronym;
//! identifiers carry no person names.
//!
//! # Parameterization
//!
//! In log-moneyness `k` the total implied variance is
//!
//! ```text
//!   w(k, θ) = (θ/2)·{ 1 + ρ·φ(θ)·k + √( (φ(θ)·k + ρ)² + (1 − ρ²) ) },
//! ```
//!
//! where `θ = θ_t` is the ATM total variance at the slice's maturity and `φ` is
//! the curvature function. We use the **power-law** curvature
//! `φ(θ) = η / θ^γ` (with `γ ∈ (0, ½]`), the standard production choice.
//!
//! # Static no-arbitrage (closed form)
//!
//! * **Butterfly:** sufficient conditions (Gatheral-Jacquier 2014, Thm 4.2):
//!   `θ·φ(θ)·(1 + |ρ|) < 4` and `θ·φ(θ)²·(1 + |ρ|) ≤ 4`.
//! * **Calendar:** `θ_t` must be non-decreasing in `t` and the curvature must
//!   satisfy `∂_θ(θ·φ(θ)) ≥ 0` with `φ` non-increasing — automatically met by the
//!   power-law `φ` for `γ ∈ (0, ½]` when `θ_t` is non-decreasing.

use celnet_core::math::sqrt;

use crate::mathx::powf;
use crate::svi::SviSlice;

/// The SSVI surface: a constant correlation, a power-law curvature
/// `φ(θ) = η/θ^γ`, and an ATM total-variance term structure provided by the
/// caller (the [`crate::termstructure`] layer supplies `θ(t)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SsviSurface {
    /// Constant correlation `ρ ∈ (−1, 1)`.
    pub rho: f64,
    /// Curvature scale `η > 0`.
    pub eta: f64,
    /// Curvature decay exponent `γ ∈ (0, ½]`.
    pub gamma: f64,
}

impl SsviSurface {
    /// Construct an SSVI surface, validating the parameter ranges.
    ///
    /// # Panics
    ///
    /// Panics if `|ρ| ≥ 1`, `η ≤ 0`, or `γ ∉ (0, ½]`.
    #[must_use]
    pub fn new(rho: f64, eta: f64, gamma: f64) -> Self {
        assert!(rho.abs() < 1.0, "SSVI rho must lie in (-1,1): {rho}");
        assert!(eta > 0.0, "SSVI eta must be positive: {eta}");
        assert!(
            gamma > 0.0 && gamma <= 0.5,
            "SSVI gamma must lie in (0, 1/2]: {gamma}"
        );
        Self { rho, eta, gamma }
    }

    /// The curvature `φ(θ) = η / θ^γ`.
    #[inline]
    #[must_use]
    pub fn phi(&self, theta: f64) -> f64 {
        self.eta / powf(theta, self.gamma)
    }

    /// Total implied variance `w(k, θ)` at log-moneyness `k` and ATM total
    /// variance `θ`.
    #[must_use]
    pub fn total_variance(&self, k: f64, theta: f64) -> f64 {
        let p = self.phi(theta);
        let pk = p * k + self.rho;
        0.5 * theta * (1.0 + self.rho * p * k + sqrt(pk * pk + (1.0 - self.rho * self.rho)))
    }

    /// Whether the butterfly no-arbitrage **sufficient** conditions hold at ATM
    /// total variance `θ` (Gatheral-Jacquier 2014, Thm 4.2):
    /// `θ·φ·(1+|ρ|) < 4` and `θ·φ²·(1+|ρ|) ≤ 4`.
    #[must_use]
    pub fn is_butterfly_free(&self, theta: f64) -> bool {
        let p = self.phi(theta);
        let one_p_abs_rho = 1.0 + self.rho.abs();
        theta * p * one_p_abs_rho < 4.0 && theta * p * p * one_p_abs_rho <= 4.0
    }

    /// Whether the calendar no-arbitrage condition holds between two maturities
    /// with ATM total variances `θ1 ≤ θ2` (`t1 < t2`): with the power-law `φ` and
    /// `γ ∈ (0, ½]`, calendar no-arbitrage reduces to **`θ` non-decreasing** plus
    /// the quantity `θ·φ(θ)` non-decreasing in `θ` — which the power-law gives for
    /// `γ ≤ 1`. We check both numerically for robustness.
    #[must_use]
    pub fn is_calendar_free(&self, theta1: f64, theta2: f64) -> bool {
        if theta2 < theta1 - 1e-14 {
            return false;
        }
        let q1 = theta1 * self.phi(theta1);
        let q2 = theta2 * self.phi(theta2);
        q2 >= q1 - 1e-12
    }

    /// Materialise an [`SviSlice`] (raw parameterization) for one maturity from
    /// this surface, given the slice's ATM total variance `θ`, forward `f` and
    /// expiry `t`. This is the bridge that lets the surface form be consumed by
    /// any [`celnet_core::Smile`] consumer through the per-slice raw model, and
    /// lets the cross-check tests compare SSVI against the other models on equal
    /// footing.
    ///
    /// The raw parameters are obtained from the SSVI→raw closed-form map
    /// (Gatheral-Jacquier 2014, §4): with `p = φ(θ)`,
    ///
    /// ```text
    ///   a = (θ/2)(1 − ρ²),   b = (θ/2)·p,   m = −ρ/p,   σ = √(1 − ρ²)/p,
    /// ```
    ///
    /// and the SSVI `ρ` carries straight through.
    ///
    /// # Panics
    ///
    /// Panics if `θ`, `f` or `t` are non-positive.
    #[must_use]
    pub fn to_slice(&self, theta: f64, forward: f64, t: f64) -> SviSlice {
        assert!(
            theta > 0.0 && forward > 0.0 && t > 0.0,
            "SSVI slice needs positive theta/forward/t"
        );
        let p = self.phi(theta);
        let one_m_rho2 = 1.0 - self.rho * self.rho;
        let a = 0.5 * theta * one_m_rho2;
        let b = 0.5 * theta * p;
        let m = -self.rho / p;
        let sigma = sqrt(one_m_rho2) / p;
        SviSlice::new(a, b, self.rho, m, sigma, forward, t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    fn surface() -> SsviSurface {
        SsviSurface::new(-0.25, 0.8, 0.4)
    }

    /// The SSVI total variance and the raw-slice total variance agree at every
    /// log-moneyness (the SSVI→raw map is exact).
    #[test]
    fn ssvi_equals_its_raw_slice() {
        let s = surface();
        let theta = 0.011; // ATM total variance ≈ (10.5%)² · 1Y.
        let slice = s.to_slice(theta, 1.10, 1.0);
        for &k in &[-0.4, -0.1, 0.0, 0.05, 0.3] {
            let w_surface = s.total_variance(k, theta);
            let w_slice = slice.total_variance(k);
            assert!(
                is_close(w_surface, w_slice, 1e-12, 1e-13),
                "k={k}: surface {w_surface} vs slice {w_slice}"
            );
        }
    }

    /// At k = 0 the total variance is exactly θ (the defining ATM property).
    #[test]
    fn atm_total_variance_is_theta() {
        let s = surface();
        let theta = 0.011;
        assert!(is_close(s.total_variance(0.0, theta), theta, 1e-13, 1e-14));
    }

    /// A well-behaved surface satisfies the closed-form butterfly conditions, and
    /// the materialised raw slice is butterfly-arbitrage-free under the density
    /// factor check too — the two no-arbitrage notions agree.
    #[test]
    fn closed_form_butterfly_matches_density_factor() {
        let s = surface();
        let theta = 0.011;
        assert!(
            s.is_butterfly_free(theta),
            "closed-form butterfly must hold"
        );
        let slice = s.to_slice(theta, 1.10, 1.0);
        assert!(
            slice.is_butterfly_free(2.0, 1e-8),
            "density factor g must be ≥ 0 when the closed-form condition holds: {}",
            slice.min_butterfly_density_factor(2.0, 4096)
        );
    }

    /// Calendar no-arbitrage: a non-decreasing θ passes, a decreasing one fails.
    #[test]
    fn calendar_condition() {
        let s = surface();
        assert!(
            s.is_calendar_free(0.006, 0.011),
            "increasing θ is calendar-free"
        );
        assert!(
            !s.is_calendar_free(0.011, 0.006),
            "decreasing θ has calendar arb"
        );
    }

    /// A surface pushed past the butterfly bound (huge η at a long maturity) is
    /// flagged: `θ·φ·(1+|ρ|) ≥ 4` breaks the closed-form sufficient condition.
    #[test]
    fn excessive_curvature_is_flagged() {
        let bad = SsviSurface::new(-0.25, 20.0, 0.1);
        // At a long maturity (large θ) the bound θ·φ·(1+|ρ|) < 4 is violated.
        assert!(!bad.is_butterfly_free(0.5));
        // And the materialised raw slice indeed has negative density factor g.
        let slice = bad.to_slice(0.5, 1.10, 5.0);
        assert!(
            !slice.is_butterfly_free(2.0, 1e-6),
            "raw slice should show butterfly arbitrage: min g = {}",
            slice.min_butterfly_density_factor(2.0, 4096)
        );
    }
}
