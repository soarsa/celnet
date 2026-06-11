//! Stochastic-volatility-inspired (SVI) raw slice parameterization.
//!
//! Provenance (doc-only): the raw SVI total-variance slice of Gatheral (2004),
//! with the static no-butterfly-arbitrage condition of Gatheral & Jacquier
//! (2014) (`docs/ANALYTICS-SPEC.md` §3.3–§3.4). "SVI" is used purely as the
//! established neutral technical acronym; identifiers carry no person names.
//!
//! # Raw parameterization
//!
//! In log-moneyness `k = ln(K/F)` the **total implied variance** of one expiry
//! slice is
//!
//! ```text
//!   w(k) = a + b·[ ρ(k − m) + √((k − m)² + σ²) ],
//! ```
//!
//! with five parameters: `a` (overall level), `b ≥ 0` (wing slope / angle),
//! `ρ ∈ (−1, 1)` (skew / asymmetry), `m` (horizontal shift) and `σ > 0`
//! (smoothness near the vertex). The implied Black volatility at `k` is
//! `√(w(k)/t)`. The linear wings are consistent with Lee's moment formula.
//!
//! # Static no-arbitrage
//!
//! A single slice is **butterfly-arbitrage-free** iff its density factor
//! `g(k) ≥ 0` for all `k` (the Durrleman g-function of Gatheral-Jacquier 2014,
//! eq. 2.1), where `g` is built from `w`, `w'`, `w''`.
//! [`ParametricSlice::min_butterfly_density_factor`] samples it.
//!
//! [`ParametricSlice::new`] validates the parameter **ranges** (`b ≥ 0`, `|ρ| < 1`,
//! `σ > 0`, positive forward/expiry) and that the minimum total variance is
//! non-negative (`a + b·σ·√(1−ρ²) ≥ 0`). The classic necessary large-strike
//! bound `b·(1+|ρ|) ≤ 2` is checked separately by
//! [`ParametricSlice::satisfies_wing_bound`] rather than at construction, so callers can
//! build a slice and inspect both no-arbitrage notions explicitly. Calendar
//! (cross-slice) no-arbitrage lives in [`crate::termstructure`] /
//! [`crate::parametric_surface`].
//!
//! ## The large-strike wing bound (and why there is no `1/t`)
//!
//! `w(k)` above is **total implied variance** `σ²(k)·t`, *not* variance per unit
//! time. Roger Lee's moment formula (Lee 2004, "The moment formula for implied
//! volatility at extreme strikes") bounds the asymptotic slope of total implied
//! variance — `lim sup_{k→±∞} w(k)/|k| ≤ 2` — and that bound is **dimensionless
//! and maturity-independent** (the `t` is already inside `w`). For raw SVI the
//! right-wing slope of `w` is `b(1+ρ)` and the left-wing slope is `b(1−ρ)`, so
//! Lee's bound is exactly `b(1+ρ) ≤ 2` and `b(1−ρ) ≤ 2`, i.e. `b(1+|ρ|) ≤ 2`
//! (Gatheral & Jacquier 2014, §3.1, the necessary large-strike condition). A
//! spurious `1/t` factor would make the threshold tighter for short maturities
//! and looser for long ones — a maturity dependence the moment formula does not
//! have — which is why [`ParametricSlice::satisfies_wing_bound`] carries **no** `t`.

use celnet_core::Smile;
use celnet_core::math::{ln, sqrt};
use celnet_types::Vol;

/// One raw-SVI expiry slice: the five parameters plus the forward/expiry the
/// slice is anchored at (so it can map strike ↔ log-moneyness and total variance
/// ↔ Black vol).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParametricSlice {
    /// Level parameter `a` (vertical offset of total variance).
    pub a: f64,
    /// Wing-slope parameter `b ≥ 0`.
    pub b: f64,
    /// Skew parameter `ρ ∈ (−1, 1)`.
    pub rho: f64,
    /// Horizontal shift `m`.
    pub m: f64,
    /// Vertex-smoothness `σ > 0`.
    pub sigma: f64,
    /// Outright forward the slice is anchored at.
    pub forward: f64,
    /// Time to expiry (years).
    pub t: f64,
}

impl ParametricSlice {
    /// Construct a raw-SVI slice, validating the parameter ranges.
    ///
    /// # Panics
    ///
    /// Panics if `b < 0`, `|ρ| ≥ 1`, `σ ≤ 0`, `forward`/`t` non-positive, or if
    /// the level can produce negative total variance (`a + b·σ·√(1−ρ²) < 0`,
    /// the minimum of `w`) — a slice with negative variance is not a valid smile.
    #[must_use]
    pub fn new(a: f64, b: f64, rho: f64, m: f64, sigma: f64, forward: f64, t: f64) -> Self {
        assert!(b >= 0.0, "SVI b must be non-negative: {b}");
        assert!(rho.abs() < 1.0, "SVI rho must lie in (-1,1): {rho}");
        assert!(sigma > 0.0, "SVI sigma must be positive: {sigma}");
        assert!(
            forward > 0.0 && t > 0.0,
            "SVI forward and t must be positive: F={forward}, t={t}"
        );
        let w_min = a + b * sigma * sqrt(1.0 - rho * rho);
        assert!(
            w_min >= -1e-12,
            "SVI minimum total variance must be non-negative: w_min={w_min}"
        );
        Self {
            a,
            b,
            rho,
            m,
            sigma,
            forward,
            t,
        }
    }

    /// Log-moneyness `k = ln(K/F)` of a strike.
    #[inline]
    #[must_use]
    pub fn log_moneyness(&self, strike: f64) -> f64 {
        ln(strike / self.forward)
    }

    /// Total implied variance `w(k)` at log-moneyness `k`.
    #[inline]
    #[must_use]
    pub fn total_variance(&self, k: f64) -> f64 {
        let d = k - self.m;
        self.a + self.b * (self.rho * d + sqrt(d * d + self.sigma * self.sigma))
    }

    /// First derivative `w'(k)`.
    #[inline]
    #[must_use]
    pub fn d_total_variance(&self, k: f64) -> f64 {
        let d = k - self.m;
        self.b * (self.rho + d / sqrt(d * d + self.sigma * self.sigma))
    }

    /// Second derivative `w''(k)`.
    #[inline]
    #[must_use]
    pub fn d2_total_variance(&self, k: f64) -> f64 {
        let d = k - self.m;
        let r = sqrt(d * d + self.sigma * self.sigma);
        self.b * self.sigma * self.sigma / (r * r * r)
    }

    /// The butterfly-arbitrage **density factor** `g(k)` (the Durrleman
    /// g-function of Gatheral-Jacquier 2014, eq. 2.1). The slice is
    /// butterfly-arbitrage-free iff `g(k) ≥ 0` ∀ k:
    ///
    /// ```text
    ///   g(k) = (1 − k·w'/(2w))² − (w'²/4)·(1/w + ¼) + w''/2.
    /// ```
    #[must_use]
    pub fn butterfly_density_factor(&self, k: f64) -> f64 {
        let w = self.total_variance(k);
        let wp = self.d_total_variance(k);
        let wpp = self.d2_total_variance(k);
        let term1 = {
            let inner = 1.0 - k * wp / (2.0 * w);
            inner * inner
        };
        let term2 = (wp * wp / 4.0) * (1.0 / w + 0.25);
        term1 - term2 + wpp / 2.0
    }

    /// The minimum of the butterfly density factor `g(k)` (Durrleman g-function)
    /// over a dense log-moneyness grid spanning `±span` around the vertex `m`. A
    /// non-negative minimum (within tolerance) certifies butterfly no-arbitrage of
    /// the slice.
    #[must_use]
    pub fn min_butterfly_density_factor(&self, span: f64, samples: usize) -> f64 {
        assert!(samples >= 3, "need ≥ 3 samples");
        let lo = self.m - span;
        let hi = self.m + span;
        let mut min_g = f64::INFINITY;
        for i in 0..samples {
            let k = lo + (hi - lo) * (i as f64) / ((samples - 1) as f64);
            min_g = min_g.min(self.butterfly_density_factor(k));
        }
        min_g
    }

    /// Whether the slice satisfies the necessary large-strike wing bound
    /// `b·(1+|ρ|) ≤ 2`: a violation guarantees butterfly arbitrage in the wings.
    ///
    /// This is Roger Lee's moment formula applied to the raw-SVI asymptotic
    /// slopes (Gatheral & Jacquier 2014, §3.1): the slope of **total implied
    /// variance** `w(k)` as `k → +∞` is `b(1+ρ)` and as `k → −∞` is `b(1−ρ)`, and
    /// Lee's formula caps each at `2`. The bound is **dimensionless** — `w`
    /// already contains the maturity `t`, so there is no `1/t` factor (a `1/t`
    /// would wrongly make the threshold maturity-dependent). This is a *necessary*
    /// (not sufficient) condition, complementary to the full density-factor scan
    /// in [`Self::min_butterfly_density_factor`].
    #[must_use]
    pub fn satisfies_wing_bound(&self) -> bool {
        self.b * (1.0 + self.rho.abs()) <= 2.0
    }

    /// Whether the slice is butterfly-arbitrage-free to tolerance `tol` over the
    /// `±span` log-moneyness window.
    #[must_use]
    pub fn is_butterfly_free(&self, span: f64, tol: f64) -> bool {
        self.min_butterfly_density_factor(span, 4096) >= -tol
    }

    /// Black implied volatility at strike `K`: `σ(K) = √(w(k)/t)`.
    #[inline]
    #[must_use]
    pub fn vol_at(&self, strike: f64) -> f64 {
        let k = self.log_moneyness(strike);
        sqrt(self.total_variance(k) / self.t)
    }
}

impl Smile for ParametricSlice {
    fn implied_vol(&self, strike: f64, _forward: f64, _t: f64) -> Vol {
        // The slice is anchored at its calibrated forward/expiry; the trait
        // forward/t are accepted for interface uniformity (an SVI slice carries
        // its own forward, unlike the sticky-delta vanna-volga smile).
        Vol(self.vol_at(strike))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    /// A representative skewed-but-arbitrage-free slice (1Y, ATM ≈ 10 vol).
    fn slice() -> ParametricSlice {
        // w_atm ≈ a + b·σ ≈ 0.01 ⇒ σ_atm ≈ 10%.
        ParametricSlice::new(0.008, 0.04, -0.3, 0.0, 0.10, 1.0, 1.0)
    }

    /// Total variance reproduces a chosen ATM vol at k = 0 (within the model).
    #[test]
    fn vol_at_forward_is_consistent() {
        let s = slice();
        let v = s.vol_at(s.forward);
        assert!(v > 0.0 && v.is_finite());
        // w(0) = a + b(ρ·(−m) + √(m²+σ²)) with m = 0 ⇒ a + b·σ.
        let w0 = s.a + s.b * s.sigma;
        assert!(is_close(v, sqrt(w0 / s.t), 1e-12, 1e-14));
    }

    /// The analytic derivatives match central finite differences.
    #[test]
    fn derivatives_match_finite_difference() {
        let s = slice();
        let k = 0.15;
        let h = 1e-5;
        let wp_fd = (s.total_variance(k + h) - s.total_variance(k - h)) / (2.0 * h);
        let wpp_fd = (s.total_variance(k + h) - 2.0 * s.total_variance(k)
            + s.total_variance(k - h))
            / (h * h);
        assert!(is_close(s.d_total_variance(k), wp_fd, 1e-6, 1e-7));
        assert!(is_close(s.d2_total_variance(k), wpp_fd, 1e-4, 1e-6));
    }

    /// A mild slice is butterfly-arbitrage-free (density factor g ≥ 0).
    #[test]
    fn mild_slice_is_butterfly_free() {
        let s = slice();
        assert!(
            s.is_butterfly_free(2.0, 1e-9),
            "min g = {}",
            s.min_butterfly_density_factor(2.0, 4096)
        );
        // The mild slice also respects the necessary large-strike wing bound.
        assert!(s.satisfies_wing_bound());
    }

    /// A pathological slice (huge b, extreme rho, tiny sigma) violates the
    /// butterfly condition — the detector fires.
    #[test]
    fn pathological_slice_has_arbitrage() {
        let bad = ParametricSlice::new(0.005, 0.9, -0.95, 0.0, 0.02, 1.0, 1.0);
        assert!(
            !bad.is_butterfly_free(2.0, 1e-6),
            "min g = {}",
            bad.min_butterfly_density_factor(2.0, 4096)
        );
    }

    /// The large-strike wing bound is the dimensionless Lee/Gatheral-Jacquier
    /// `b(1+|ρ|) ≤ 2`, NOT a maturity-dependent `b(1+|ρ|) ≤ 4/t`. This test pins
    /// both the correct *constant* (2, not 4) and the *absence of any `t`
    /// dependence* — the precise regression the audit flagged.
    #[test]
    fn wing_bound_is_dimensionless_and_uses_constant_two() {
        // A slice with b(1+|ρ|) = 1.5·(1+0.0) = 1.5: under the bound (passes).
        // The OLD buggy `4/t` at t = 1 would also pass — so to separate the two
        // we need a slope strictly between the correct (2) and the old (4)
        // thresholds.
        let slope = 3.0; // b(1+|ρ|) = 3: > 2 (correct ⇒ FAIL), < 4 (old ⇒ pass).
        let b = slope; // ρ = 0 ⇒ b(1+|ρ|) = b.

        // At t = 1 the OLD bound 4/t = 4 ≥ 3 would (wrongly) pass; correct ⇒ fail.
        let s1 = ParametricSlice::new(0.30, b, 0.0, 0.0, 0.10, 1.0, 1.0);
        assert!(
            !s1.satisfies_wing_bound(),
            "slope {slope} exceeds the dimensionless bound 2 and must FAIL \
             regardless of maturity"
        );

        // Maturity-independence: the SAME parameters at a very different t give
        // the SAME verdict. The OLD `4/t` would FLIP the verdict (4/0.01 = 400 at
        // short t ⇒ pass; 4/100 = 0.04 at long t ⇒ fail), so this pins out any t.
        let s_short = ParametricSlice::new(0.30, b, 0.0, 0.0, 0.10, 1.0, 0.01);
        let s_long = ParametricSlice::new(0.30, b, 0.0, 0.0, 0.10, 1.0, 100.0);
        assert_eq!(
            s1.satisfies_wing_bound(),
            s_short.satisfies_wing_bound(),
            "wing bound verdict must not depend on maturity (short t)"
        );
        assert_eq!(
            s1.satisfies_wing_bound(),
            s_long.satisfies_wing_bound(),
            "wing bound verdict must not depend on maturity (long t)"
        );

        // A slope just below 2 passes at every maturity (right at the Lee cap).
        let ok = ParametricSlice::new(0.30, 1.9, 0.0, 0.0, 0.10, 1.0, 0.01);
        assert!(ok.satisfies_wing_bound(), "slope 1.9 < 2 must pass");
        let ok_long = ParametricSlice::new(0.30, 1.9, 0.0, 0.0, 0.10, 1.0, 50.0);
        assert!(
            ok_long.satisfies_wing_bound(),
            "slope 1.9 < 2 must pass at long t"
        );
    }

    /// The total variance, its two derivatives, and the butterfly density
    /// factor match raw in-test recomputations of the published closed forms on
    /// a slice with every parameter active (`m ≠ 0`, skewed). The derivative
    /// oracles are the analytic forms, not finite differences, so every
    /// operator mutant — including ones below FD tolerance — is killed.
    #[test]
    fn slice_forms_match_raw_recomputation() {
        let s = ParametricSlice::new(0.008, 0.04, -0.3, 0.05, 0.10, 1.10, 1.0);
        for &k in &[-0.5_f64, -0.12, 0.0, 0.05, 0.21, 0.6] {
            let d = k - s.m;
            let r = (d * d + s.sigma * s.sigma).sqrt();
            let w = s.a + s.b * (s.rho * d + r);
            let wp = s.b * (s.rho + d / r);
            let wpp = s.b * s.sigma * s.sigma / (r * r * r);
            assert!(is_close(s.total_variance(k), w, 1e-15, 1e-16), "w({k})");
            assert!(is_close(s.d_total_variance(k), wp, 1e-15, 1e-16), "w'({k})");
            assert!(
                is_close(s.d2_total_variance(k), wpp, 1e-15, 1e-16),
                "w''({k})"
            );
            // Durrleman density factor from the (independently pinned) w, w', w''.
            let term1 = {
                let inner = 1.0 - k * wp / (2.0 * w);
                inner * inner
            };
            let g = term1 - (wp * wp / 4.0) * (1.0 / w + 0.25) + wpp / 2.0;
            assert!(
                is_close(s.butterfly_density_factor(k), g, 1e-14, 1e-16),
                "g({k}): got {}, want {g}",
                s.butterfly_density_factor(k)
            );
        }
        // log_moneyness and vol_at: k = ln(K/F), σ = √(w(k)/t).
        let strike = 1.23;
        let k = (strike / s.forward).ln();
        assert!(is_close(s.log_moneyness(strike), k, 1e-15, 1e-16));
        assert!(is_close(
            s.vol_at(strike),
            (s.total_variance(k) / s.t).sqrt(),
            1e-15,
            1e-16
        ));
    }

    /// The density-factor scan reproduces an in-test scan with the identical
    /// grid (`±span` around the vertex `m`, `samples` equispaced points) — pins
    /// the grid arithmetic and the min-fold of `min_butterfly_density_factor`.
    #[test]
    fn density_scan_matches_in_test_grid() {
        let s = ParametricSlice::new(0.005, 0.30, -0.6, 0.08, 0.05, 1.0, 1.0);
        let (span, samples) = (1.5_f64, 33_usize);
        let mut want = f64::INFINITY;
        for i in 0..samples {
            let k = (s.m - span) + 2.0 * span * (i as f64) / ((samples - 1) as f64);
            want = want.min(s.butterfly_density_factor(k));
        }
        let got = s.min_butterfly_density_factor(span, samples);
        assert!(
            is_close(got, want, 1e-15, 1e-16),
            "scan min {got} must reproduce the in-test grid min {want}"
        );
    }

    /// The wing bound is inclusive at exactly the cap: `b(1+|ρ|) = 2` passes
    /// (Lee's bound is `≤ 2`), and an infinitesimally larger slope fails — the
    /// exact-boundary drive for the `≤` comparison.
    #[test]
    fn wing_bound_boundary_is_inclusive() {
        let at_cap = ParametricSlice::new(0.30, 2.0, 0.0, 0.0, 0.10, 1.0, 1.0);
        assert!(at_cap.satisfies_wing_bound(), "slope exactly 2 must pass");
        let just_over = ParametricSlice::new(0.30, 2.0 + 1e-12, 0.0, 0.0, 0.10, 1.0, 1.0);
        assert!(!just_over.satisfies_wing_bound(), "slope 2+ε must fail");
    }

    /// Negative-rho slice is downward-skewed: put-wing vol exceeds call-wing vol.
    #[test]
    fn negative_rho_is_downward_skewed() {
        let s = slice();
        let put_wing = s.vol_at(s.forward * 0.85);
        let call_wing = s.vol_at(s.forward * 1.15);
        assert!(put_wing > call_wing);
    }
}
