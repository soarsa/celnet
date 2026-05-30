//! Deterministic, cross-platform math primitives.
//!
//! Transcendentals route through [`libm`] (the `rust-lang/libm` software
//! implementation, correctly-rounded to ≤ 1.0 ULP vs MPFR) rather than the
//! system libm, so results are **bit-identical across OS and architecture** and
//! across libm updates — a precondition for reproducible, auditable pricing.

/// `1/√(2π)` — the standard-normal PDF normalization constant.
pub const INV_SQRT_2PI: f64 = 0.398_942_280_401_432_677_939_946_059_934_4;

/// `1/√2`.
pub const INV_SQRT_2: f64 = core::f64::consts::FRAC_1_SQRT_2;

/// Natural exponential, via [`libm`].
#[inline]
#[must_use]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

/// Natural logarithm, via [`libm`].
#[inline]
#[must_use]
pub fn ln(x: f64) -> f64 {
    libm::log(x)
}

/// Square root, via [`libm`].
#[inline]
#[must_use]
pub fn sqrt(x: f64) -> f64 {
    libm::sqrt(x)
}

/// Standard-normal probability density function `φ(x)`.
#[inline]
#[must_use]
pub fn norm_pdf(x: f64) -> f64 {
    INV_SQRT_2PI * exp(-0.5 * x * x)
}

/// Standard-normal cumulative distribution function `Φ(x)`.
///
/// Computed as `½·erfc(−x/√2)` using [`libm::erfc`], which is numerically
/// stable in both tails (no catastrophic cancellation for large `|x|`).
#[inline]
#[must_use]
pub fn norm_cdf(x: f64) -> f64 {
    0.5 * libm::erfc(-x * INV_SQRT_2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_close;

    #[test]
    fn norm_cdf_known_points() {
        assert_close!(norm_cdf(0.0), 0.5, 1e-15, 1e-15);
        // Φ(1.96) ≈ 0.975 (two-sided 95%).
        assert_close!(norm_cdf(1.959_963_984_540_054), 0.975, 1e-12, 1e-12);
        assert_close!(norm_cdf(-1.0), 1.0 - norm_cdf(1.0), 1e-15, 1e-15);
    }

    #[test]
    fn norm_cdf_tails_are_stable() {
        // No underflow-to-garbage; monotone and bounded.
        assert!(norm_cdf(-40.0) >= 0.0);
        assert!(norm_cdf(40.0) <= 1.0);
        assert!(norm_cdf(-8.0) < norm_cdf(-7.0));
    }

    #[test]
    fn norm_pdf_peak() {
        assert_close!(norm_pdf(0.0), INV_SQRT_2PI, 1e-15, 1e-15);
        assert_close!(norm_pdf(1.0), norm_pdf(-1.0), 1e-15, 1e-15);
    }
}
