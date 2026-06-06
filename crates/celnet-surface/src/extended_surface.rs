//! Extended surface stochastic-volatility-inspired (eSSVI) parameterization.
//!
//! Provenance (doc-only): the *extended* SSVI of Hendriks & Martini (2019),
//! "The extended SSVI volatility surface" (J. Comp. Finance), which generalises
//! the surface SVI of Gatheral & Jacquier (2014) by letting the correlation be
//! **maturity-dependent**, `ρ → ρ(θ)`, while retaining closed-form static
//! no-arbitrage (butterfly + calendar) conditions. The calendar condition we use
//! is the explicit two-slice inequality of Hendriks & Martini (2019, Prop. 3.1),
//! itself a specialisation of the general calendar-spread condition of Corbetta,
//! Cohort, Laachir & Martini (2019) / Gatheral & Jacquier (2014, Thm 4.1)
//! (`docs/ANALYTICS-SPEC.md` §3.3). "SSVI"/"eSSVI" are the established neutral
//! acronyms; identifiers carry no person names (guardrail #8).
//!
//! # The `(θ, ρ, ψ)` slice parameterization
//!
//! Each maturity slice is described by three numbers — the ATM total variance
//! `θ`, the correlation `ρ ∈ (−1, 1)`, and the **ATM skew-scale**
//! `ψ = θ·φ(θ) > 0` (`ψ` is the natural eSSVI shape variable: `∂_k w|_{k=0} = ρψ`
//! is the ATM skew, and the curvature scales with `ψ²/θ`). In these variables the
//! total implied variance is the SSVI form with `φ = ψ/θ`:
//!
//! ```text
//!   w(k) = (θ/2)·{ 1 + ρ·(ψ/θ)·k + √( ((ψ/θ)·k + ρ)² + (1 − ρ²) ) }.
//! ```
//!
//! The whole surface is a set of calibrated pillars `(θ_i, ρ_i, ψ_i)` ordered by
//! `θ`; between pillars `ρ` and `ψ` are interpolated linearly **in `θ`** (the
//! coordinate in which the calendar condition is linear), and `θ(t)` is supplied
//! by the caller's ATM total-variance term structure. SSVI is the special case
//! `ρ(θ) ≡ const` with `ψ = θ·φ(θ)` from a power-law `φ`.
//!
//! # Static no-arbitrage (closed form) — CLAIMS validated against numerics
//!
//! The two predicates below are *claims*; the `celnet-parity` `essvi` rows
//! validate them against the independent Breeden-Litzenberger density numerics
//! and pointwise calendar monotonicity (the numerics are the oracle).
//!
//! * **Butterfly (per slice)** — substituting `φ = ψ/θ` into the SSVI sufficient
//!   conditions (Gatheral-Jacquier 2014, Thm 4.2) gives, in the `(θ,ρ,ψ)`
//!   variables, `ψ·(1+|ρ|) < 4` and `(ψ²/θ)·(1+|ρ|) ≤ 4` (Hendriks-Martini 2019,
//!   §2). The first is the eSSVI g-function / large-strike bound; the second the
//!   vertex curvature bound.
//! * **Calendar (between consecutive slices)** — for `θ₁ < θ₂` the surface is
//!   calendar-arbitrage-free if `w(k,θ₂) ≥ w(k,θ₁)` for every `k`. Hendriks &
//!   Martini (2019, Prop. 3.1) reduce this to the explicit pair condition
//!   `θ₁ ≤ θ₂`, `ψ₁ ≤ ψ₂`, and `ρ₁·ψ₁ ≤ ρ₂·ψ₂` **and**
//!   `ρ₂·ψ₂ − ρ₁·ψ₁ ≤ ψ₂ − ψ₁` (equivalently `|ρ₂ψ₂ − ρ₁ψ₁| ≤ ψ₂ − ψ₁`): the ATM
//!   skews and the ψ-gap must be compatible so the two slices never cross.

use celnet_core::math::sqrt;

use crate::parametric::ParametricSlice;
use crate::parametric_surface::ParametricSurface;

/// One eSSVI maturity slice in the `(θ, ρ, ψ)` parameterization: the ATM total
/// variance `θ`, the correlation `ρ ∈ (−1, 1)`, and the ATM skew-scale
/// `ψ = θ·φ(θ) > 0`.
///
/// Internally the slice carries the **curvature `φ`** (the SSVI shape variable
/// `φ = ψ/θ`) as well, computed once at construction. Evaluating `total_variance`
/// through the stored `φ` — exactly as the SSVI [`ParametricSurface`] does — is
/// what makes the SSVI special case **byte-recoverable**: when an eSSVI slice is
/// built from an SSVI `(ρ, φ(θ))` via [`ExtendedSlice::from_curvature`], its
/// `total_variance` reproduces [`ParametricSurface::total_variance`] to bit
/// identity (no `ψ/θ` round-trip, which would lose a ULP).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtendedSlice {
    /// ATM total variance `θ > 0` (`= σ_ATM²·t`).
    pub theta: f64,
    /// Correlation `ρ ∈ (−1, 1)`.
    pub rho: f64,
    /// ATM skew-scale `ψ = θ·φ > 0`.
    pub psi: f64,
    /// The SSVI curvature `φ` of this slice (`= ψ/θ`), stored so `total_variance`
    /// evaluates the SSVI form through the identical arithmetic.
    phi: f64,
}

impl ExtendedSlice {
    /// Construct an eSSVI slice from `(θ, ρ, ψ)`, validating the parameter ranges.
    /// The curvature is `φ = ψ/θ`.
    ///
    /// # Panics
    ///
    /// Panics if `θ ≤ 0`, `|ρ| ≥ 1`, or `ψ ≤ 0`.
    #[must_use]
    pub fn new(theta: f64, rho: f64, psi: f64) -> Self {
        assert!(theta > 0.0, "eSSVI theta must be positive: {theta}");
        assert!(rho.abs() < 1.0, "eSSVI rho must lie in (-1,1): {rho}");
        assert!(psi > 0.0, "eSSVI psi must be positive: {psi}");
        Self {
            theta,
            rho,
            psi,
            phi: psi / theta,
        }
    }

    /// Construct an eSSVI slice from the SSVI shape variables `(θ, ρ, φ)` (the
    /// curvature `φ`), with `ψ = θ·φ` derived. This is the canonical SSVI→eSSVI
    /// bridge: a slice so constructed evaluates `total_variance` through the
    /// **same `φ`** the SSVI surface uses, so the SSVI special case is recovered
    /// bit-for-bit (no `ψ/θ` round-trip).
    ///
    /// # Panics
    ///
    /// Panics if `θ ≤ 0`, `|ρ| ≥ 1`, or `φ ≤ 0`.
    #[must_use]
    pub fn from_curvature(theta: f64, rho: f64, phi: f64) -> Self {
        assert!(theta > 0.0, "eSSVI theta must be positive: {theta}");
        assert!(rho.abs() < 1.0, "eSSVI rho must lie in (-1,1): {rho}");
        assert!(phi > 0.0, "eSSVI phi must be positive: {phi}");
        Self {
            theta,
            rho,
            psi: theta * phi,
            phi,
        }
    }

    /// The SSVI curvature `φ = ψ/θ` of this slice (the stored exact value).
    #[inline]
    #[must_use]
    pub fn phi(&self) -> f64 {
        self.phi
    }

    /// Total implied variance `w(k)` at log-moneyness `k`, the SSVI form with the
    /// stored curvature `φ`:
    ///
    /// ```text
    ///   w(k) = (θ/2)·{ 1 + ρ·φ·k + √( (φ·k + ρ)² + (1 − ρ²) ) }.
    /// ```
    ///
    /// This is **token-for-token** the SSVI [`ParametricSurface::total_variance`]
    /// arithmetic (with `φ` supplied rather than recomputed from `η/θ^γ`), so the
    /// SSVI special case is byte-recoverable.
    #[inline]
    #[must_use]
    pub fn total_variance(&self, k: f64) -> f64 {
        let p = self.phi;
        let pk = p * k + self.rho;
        0.5 * self.theta * (1.0 + self.rho * p * k + sqrt(pk * pk + (1.0 - self.rho * self.rho)))
    }

    /// Whether the per-slice butterfly no-arbitrage **sufficient** conditions hold
    /// in the `(θ,ρ,ψ)` variables (Hendriks-Martini 2019, §2; the SSVI
    /// Gatheral-Jacquier Thm 4.2 conditions with `φ = ψ/θ`):
    /// `ψ·(1+|ρ|) < 4` and `(ψ²/θ)·(1+|ρ|) ≤ 4`.
    #[must_use]
    pub fn is_butterfly_free(&self) -> bool {
        let one_p_abs_rho = 1.0 + self.rho.abs();
        self.psi * one_p_abs_rho < 4.0 && (self.psi * self.psi / self.theta) * one_p_abs_rho <= 4.0
    }

    /// Materialise an [`ParametricSlice`] (raw SVI) for this eSSVI slice at the
    /// given forward `f` and expiry `t`, via the exact SSVI→raw closed-form map
    /// applied with `φ = ψ/θ`. This bridges eSSVI into every [`celnet_core::Smile`]
    /// consumer and lets the arbitrage numerics re-strike the genuine curvature.
    ///
    /// # Panics
    ///
    /// Panics if `f` or `t` are non-positive.
    #[must_use]
    pub fn to_slice(&self, forward: f64, t: f64) -> ParametricSlice {
        // Reuse the SSVI→raw map: an SSVI surface with this slice's ρ and a
        // power-law η/γ that reproduces φ = ψ/θ at θ gives the identical raw slice
        // (the map depends only on (θ, ρ, φ), not on the η/γ split). Choose γ = ½
        // and η = φ·θ^{1/2} so ParametricSurface::phi(θ) == φ exactly.
        let phi = self.phi();
        let eta = phi * sqrt(self.theta);
        ParametricSurface::new(self.rho, eta, 0.5).to_slice(self.theta, forward, t)
    }

    /// Whether this slice and a *later* slice `next` (with `θ_next > θ`) are free
    /// of calendar-spread arbitrage between them, by the explicit eSSVI pair
    /// condition (Hendriks-Martini 2019, Prop. 3.1):
    ///
    /// ```text
    ///   θ ≤ θ_next,   ψ ≤ ψ_next,   and   |ρ_next·ψ_next − ρ·ψ| ≤ ψ_next − ψ.
    /// ```
    ///
    /// The third inequality couples the ATM skews (`ρψ`) to the ψ-gap so the two
    /// total-variance curves never cross in `k`. A small absolute slack absorbs
    /// floating-point noise at the boundary.
    #[must_use]
    pub fn is_calendar_free_with(&self, next: &ExtendedSlice) -> bool {
        const SLACK: f64 = 1e-12;
        if next.theta < self.theta - SLACK {
            return false;
        }
        if next.psi < self.psi - SLACK {
            return false;
        }
        let skew_gap = (next.rho * next.psi - self.rho * self.psi).abs();
        let psi_gap = next.psi - self.psi;
        skew_gap <= psi_gap + SLACK
    }
}

/// An eSSVI surface: an ordered set of calibrated `(θ_i, ρ_i, ψ_i)` pillars with
/// monotone-`θ` linear interpolation of `ρ` and `ψ`, plus the closed-form static
/// no-arbitrage predicates over the whole pillar set.
///
/// The pillars are stored sorted by `θ` (ascending). The surface evaluates a
/// total variance at any `(k, θ)` by locating the bracketing pillars and
/// interpolating `(ρ, ψ)` linearly in `θ` (constant extrapolation outside the
/// pillar range), then evaluating the eSSVI slice. `θ` itself is the surface's
/// own coordinate, supplied by the caller's ATM total-variance term structure.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtendedSurface {
    /// Pillars sorted ascending by `θ`.
    pillars: Vec<ExtendedSlice>,
}

impl ExtendedSurface {
    /// Build an eSSVI surface from calibrated pillars.
    ///
    /// # Panics
    ///
    /// Panics if `pillars` is empty or the `θ` values are not strictly increasing.
    #[must_use]
    pub fn new(pillars: Vec<ExtendedSlice>) -> Self {
        assert!(!pillars.is_empty(), "eSSVI surface needs ≥ 1 pillar");
        assert!(
            pillars.windows(2).all(|w| w[0].theta < w[1].theta),
            "eSSVI surface pillars must be strictly increasing in theta"
        );
        Self { pillars }
    }

    /// The calibrated pillars, ascending in `θ`.
    #[must_use]
    pub fn pillars(&self) -> &[ExtendedSlice] {
        &self.pillars
    }

    /// The interpolated eSSVI slice at ATM total variance `θ`: `(ρ, ψ)` are
    /// linearly interpolated in `θ` between the bracketing pillars (constant
    /// extrapolation beyond the endpoints).
    #[must_use]
    pub fn slice_at(&self, theta: f64) -> ExtendedSlice {
        let p = &self.pillars;
        if theta <= p[0].theta {
            return ExtendedSlice::new(theta.max(f64::MIN_POSITIVE), p[0].rho, p[0].psi);
        }
        let last = &p[p.len() - 1];
        if theta >= last.theta {
            return ExtendedSlice::new(theta, last.rho, last.psi);
        }
        // Locate the bracketing pillars [lo, hi] with lo.theta ≤ θ < hi.theta.
        let mut hi = 1;
        while hi < p.len() && p[hi].theta <= theta {
            hi += 1;
        }
        let lo = &p[hi - 1];
        let hi = &p[hi];
        let frac = (theta - lo.theta) / (hi.theta - lo.theta);
        let rho = lo.rho + frac * (hi.rho - lo.rho);
        let psi = lo.psi + frac * (hi.psi - lo.psi);
        ExtendedSlice::new(theta, rho, psi)
    }

    /// Total implied variance `w(k, θ)` at log-moneyness `k` and ATM total
    /// variance `θ`, evaluating the interpolated slice.
    #[must_use]
    pub fn total_variance(&self, k: f64, theta: f64) -> f64 {
        self.slice_at(theta).total_variance(k)
    }

    /// Materialise a raw [`ParametricSlice`] at `θ` for forward `f`, expiry `t`.
    ///
    /// # Panics
    ///
    /// Panics if `θ`, `f` or `t` are non-positive.
    #[must_use]
    pub fn to_slice(&self, theta: f64, forward: f64, t: f64) -> ParametricSlice {
        self.slice_at(theta).to_slice(forward, t)
    }

    /// Whether every calibrated pillar is butterfly-arbitrage-free (per-slice).
    #[must_use]
    pub fn is_butterfly_free(&self) -> bool {
        self.pillars.iter().all(ExtendedSlice::is_butterfly_free)
    }

    /// Whether every consecutive pillar pair is calendar-arbitrage-free.
    #[must_use]
    pub fn is_calendar_free(&self) -> bool {
        self.pillars
            .windows(2)
            .all(|w| w[0].is_calendar_free_with(&w[1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    /// At constant `ρ` and `ψ = θ·φ(θ)` from a power-law `φ`, the eSSVI slice's
    /// total variance equals the existing SSVI `ParametricSurface::total_variance`
    /// **bit-for-bit** across a k-grid — SSVI byte-recovered as the special case.
    #[test]
    fn ssvi_byte_recovered_at_constant_rho() {
        let rho = -0.25;
        let eta = 0.8;
        let gamma = 0.4;
        let ssvi = ParametricSurface::new(rho, eta, gamma);
        for &theta in &[0.004_f64, 0.011, 0.03, 0.07] {
            // Build from the SSVI curvature φ(θ) directly so the arithmetic is
            // token-for-token the SSVI form (no ψ/θ round-trip).
            let essvi = ExtendedSlice::from_curvature(theta, rho, ssvi.phi(theta));
            for &k in &[-0.5, -0.2, -0.05, 0.0, 0.05, 0.2, 0.5] {
                let w_ssvi = ssvi.total_variance(k, theta);
                let w_essvi = essvi.total_variance(k);
                assert_eq!(
                    w_ssvi.to_bits(),
                    w_essvi.to_bits(),
                    "byte mismatch at theta={theta}, k={k}: SSVI {w_ssvi} vs eSSVI {w_essvi}"
                );
            }
        }
    }

    /// At k = 0 the eSSVI total variance is exactly θ (the ATM property).
    #[test]
    fn atm_total_variance_is_theta() {
        let s = ExtendedSlice::new(0.011, -0.2, 0.3);
        assert!(is_close(s.total_variance(0.0), 0.011, 1e-13, 1e-14));
    }

    /// The ATM skew `∂_k w|_{k=0}` equals `ρ·ψ` (the defining property of ψ).
    #[test]
    fn atm_skew_is_rho_psi() {
        let s = ExtendedSlice::new(0.02, -0.3, 0.25);
        let h = 1e-6;
        let skew_fd = (s.total_variance(h) - s.total_variance(-h)) / (2.0 * h);
        assert!(
            is_close(skew_fd, s.rho * s.psi, 1e-6, 1e-8),
            "ATM skew {skew_fd} must equal ρψ = {}",
            s.rho * s.psi
        );
    }

    /// The eSSVI→raw materialised slice reproduces the eSSVI total variance at
    /// every log-moneyness (the SSVI→raw map is exact).
    #[test]
    fn essvi_equals_its_raw_slice() {
        let s = ExtendedSlice::new(0.011, -0.25, 0.32);
        let slice = s.to_slice(1.10, 1.0);
        for &k in &[-0.4, -0.1, 0.0, 0.05, 0.3] {
            let w_surface = s.total_variance(k);
            let strike = 1.10 * celnet_core::math::exp(k);
            let w_slice = slice.total_variance(slice.log_moneyness(strike));
            assert!(
                is_close(w_surface, w_slice, 1e-12, 1e-13),
                "k={k}: surface {w_surface} vs slice {w_slice}"
            );
        }
    }

    /// A mild slice satisfies the closed-form butterfly conditions. With θ = 0.011
    /// and ρ = −0.25 the vertex-curvature bound `ψ ≤ √(4θ/(1+|ρ|)) ≈ 0.188` is the
    /// binding one; a ψ comfortably inside it is butterfly-free.
    #[test]
    fn mild_slice_is_butterfly_free() {
        let s = ExtendedSlice::new(0.011, -0.25, 0.14);
        assert!(s.is_butterfly_free());
    }

    /// A slice pushed past the butterfly bound (huge ψ) is flagged.
    #[test]
    fn excessive_psi_is_flagged() {
        // ψ(1+|ρ|) = 5·1.25 = 6.25 ≥ 4 ⇒ violates the first condition.
        let bad = ExtendedSlice::new(0.5, -0.25, 5.0);
        assert!(!bad.is_butterfly_free());
    }

    /// Calendar: a non-decreasing-θ, compatible-skew pair passes; a crossing pair
    /// (skew gap exceeding the ψ-gap) fails.
    #[test]
    fn calendar_condition() {
        let s1 = ExtendedSlice::new(0.006, -0.2, 0.20);
        let s2 = ExtendedSlice::new(0.011, -0.2, 0.30);
        assert!(
            s1.is_calendar_free_with(&s2),
            "compatible pair is calendar-free"
        );

        // Same θ ordering and ψ ordering, but the skew gap |ρ₂ψ₂ − ρ₁ψ₁| exceeds
        // the ψ-gap ⇒ a crossing ⇒ flagged.
        let s3 = ExtendedSlice::new(0.011, 0.9, 0.32);
        assert!(
            !s1.is_calendar_free_with(&s3),
            "skew gap {} exceeds psi gap {}",
            (s3.rho * s3.psi - s1.rho * s1.psi).abs(),
            s3.psi - s1.psi
        );
    }

    /// The surface interpolates `(ρ, ψ)` linearly in θ and reproduces a pillar
    /// exactly at its own θ.
    #[test]
    fn surface_interpolates_and_reproduces_pillars() {
        // ψ values stay inside the per-θ butterfly bound √(4θ/(1+|ρ|)) and the
        // pair (ρψ, ψ) gaps keep the slices from crossing (calendar-free).
        let surf = ExtendedSurface::new(vec![
            ExtendedSlice::new(0.006, -0.20, 0.10),
            ExtendedSlice::new(0.011, -0.25, 0.14),
            ExtendedSlice::new(0.030, -0.30, 0.20),
        ]);
        // At a pillar θ the slice equals the pillar.
        let at = surf.slice_at(0.011);
        assert!(is_close(at.rho, -0.25, 1e-13, 1e-14));
        assert!(is_close(at.psi, 0.14, 1e-13, 1e-14));
        // Midway between the first two pillars, ρ/ψ are the linear midpoints.
        let mid = surf.slice_at(0.0085);
        assert!(is_close(mid.rho, -0.225, 1e-12, 1e-13));
        assert!(is_close(mid.psi, 0.12, 1e-12, 1e-13));
        assert!(surf.is_butterfly_free());
        assert!(surf.is_calendar_free());
    }
}
