//! Hazard-rate survival curve.
//!
//! A reduced-form (intensity) credit model: default arrives as the first jump of
//! a Cox process with deterministic, piecewise-constant intensity (hazard rate)
//! `λ(t)`. The risk-neutral survival probability to time `t` is
//!
//! ```text
//! S(t) = exp(−∫₀ᵗ λ(u) du).
//! ```
//!
//! With a flat hazard `λ` this collapses to `S(t) = exp(−λ·t)`; the marginal
//! default probability over an interval `(a, b]` is `S(a) − S(b)`, the increment
//! used by the CVA/DVA aggregation. Provenance: standard reduced-form credit
//! modelling (Lando 1998; Brigo-Mercurio 2006, ch. 21–22) — purpose-named here.

/// A piecewise-constant hazard-rate term structure giving survival probabilities.
///
/// `pillars[i]` are strictly increasing knot times (years); `hazards[i]` is the
/// constant hazard rate applied on the segment ending at `pillars[i]` (i.e. on
/// `(pillars[i−1], pillars[i]]`, with `pillars[−1] = 0`). Beyond the last pillar
/// the final hazard is held flat (forward-extrapolation), the standard market
/// convention.
#[derive(Debug, Clone, PartialEq)]
pub struct SurvivalCurve {
    pillars: Vec<f64>,
    hazards: Vec<f64>,
}

impl SurvivalCurve {
    /// A **flat** hazard-rate curve `S(t) = exp(−λ·t)`.
    ///
    /// `lambda` must be finite and non-negative. Panics otherwise (a negative
    /// hazard would make survival increase with time — a modelling error, never a
    /// valid input).
    #[must_use]
    pub fn flat(lambda: f64) -> Self {
        assert!(
            lambda.is_finite() && lambda >= 0.0,
            "hazard rate must be finite and non-negative, got {lambda}"
        );
        // A single pillar at +∞ in effect: represent as one segment held flat.
        Self {
            pillars: vec![f64::INFINITY],
            hazards: vec![lambda],
        }
    }

    /// A **piecewise-constant** hazard curve from strictly-increasing pillar times
    /// and per-segment hazard rates (`pillars.len() == hazards.len()`, both
    /// non-empty). Each `hazards[i]` applies on `(pillars[i−1], pillars[i]]`.
    ///
    /// Panics on shape mismatch, non-increasing pillars, or negative/non-finite
    /// hazards — all modelling errors, never valid inputs.
    #[must_use]
    pub fn piecewise(pillars: Vec<f64>, hazards: Vec<f64>) -> Self {
        assert!(
            !pillars.is_empty() && pillars.len() == hazards.len(),
            "pillars and hazards must be non-empty and equal length"
        );
        let mut prev = 0.0;
        for (&p, &h) in pillars.iter().zip(&hazards) {
            assert!(
                p > prev,
                "pillar times must be strictly increasing and positive"
            );
            assert!(
                h.is_finite() && h >= 0.0,
                "hazard rate must be finite and non-negative, got {h}"
            );
            prev = p;
        }
        Self { pillars, hazards }
    }

    /// The cumulative hazard `∫₀ᵗ λ(u) du` for `t ≥ 0`.
    #[must_use]
    pub fn cumulative_hazard(&self, t: f64) -> f64 {
        assert!(t >= 0.0 && t.is_finite(), "time must be finite and ≥ 0");
        let mut acc = 0.0;
        let mut lo = 0.0;
        for (&p, &h) in self.pillars.iter().zip(&self.hazards) {
            let hi = p.min(t);
            if hi > lo {
                acc += h * (hi - lo);
            }
            lo = p;
            if t <= p {
                return acc;
            }
        }
        // Beyond the last finite pillar: hold the final hazard flat.
        let last_pillar = *self.pillars.last().expect("non-empty by construction");
        if last_pillar.is_finite() && t > last_pillar {
            let last_hazard = *self.hazards.last().expect("non-empty by construction");
            acc += last_hazard * (t - last_pillar);
        }
        acc
    }

    /// Survival probability `S(t) = exp(−∫₀ᵗ λ)`.
    #[must_use]
    pub fn survival(&self, t: f64) -> f64 {
        libm::exp(-self.cumulative_hazard(t))
    }

    /// Marginal (interval) default probability `S(a) − S(b)` over `(a, b]`,
    /// `0 ≤ a ≤ b`. Always non-negative because `S` is non-increasing.
    #[must_use]
    pub fn marginal_default(&self, a: f64, b: f64) -> f64 {
        assert!(a <= b, "interval must satisfy a ≤ b");
        self.survival(a) - self.survival(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_curve_matches_exp() {
        let c = SurvivalCurve::flat(0.03);
        for &t in &[0.0, 0.5, 1.0, 2.5, 5.0] {
            let want = libm::exp(-0.03 * t);
            assert!((c.survival(t) - want).abs() < 1e-15, "t={t}");
        }
    }

    #[test]
    fn zero_hazard_never_defaults() {
        let c = SurvivalCurve::flat(0.0);
        assert!((c.survival(10.0) - 1.0).abs() < 1e-15);
        assert!(c.marginal_default(0.0, 10.0).abs() < 1e-15);
    }

    #[test]
    fn survival_is_monotone_nonincreasing_in_time() {
        let c = SurvivalCurve::flat(0.05);
        let mut prev = 1.0;
        for k in 0..=40 {
            let t = k as f64 * 0.25;
            let s = c.survival(t);
            assert!(s <= prev + 1e-15);
            prev = s;
        }
    }

    #[test]
    fn piecewise_continuous_at_pillars() {
        // λ=0.02 on (0,1], λ=0.06 on (1,3].
        let c = SurvivalCurve::piecewise(vec![1.0, 3.0], vec![0.02, 0.06]);
        // S(1) = e^{-0.02}; S(3) = e^{-(0.02·1 + 0.06·2)} = e^{-0.14}.
        assert!((c.survival(1.0) - libm::exp(-0.02)).abs() < 1e-15);
        assert!((c.survival(3.0) - libm::exp(-0.14)).abs() < 1e-15);
        // Mid-segment: S(2) = e^{-(0.02 + 0.06)} = e^{-0.08}.
        assert!((c.survival(2.0) - libm::exp(-0.08)).abs() < 1e-15);
    }

    #[test]
    fn piecewise_extrapolates_flat_past_last_pillar() {
        let c = SurvivalCurve::piecewise(vec![1.0], vec![0.04]);
        // Past pillar 1 the 0.04 hazard is held: S(2) = e^{-0.08}.
        assert!((c.survival(2.0) - libm::exp(-0.08)).abs() < 1e-15);
    }
}
