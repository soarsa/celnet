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

/// Square root, via IEEE-754 hardware instruction (`fsqrt` / `sqrtsd`).
#[inline]
#[must_use]
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
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

    /// Deep-tail accuracy is load-bearing for far-OTM vanillas, digitals and
    /// barrier touch probabilities, so we validate against published reference
    /// values of `Φ` rather than merely asserting bounds. Reference values are
    /// from standard normal-distribution tables / high-precision computation:
    ///   Φ(-1)  = 0.158_655_253_931_457_05
    ///   Φ(-5)  = 2.866_515_718_791_939e-7
    ///   Φ(-10) = 7.619_853_024_160_525e-24
    #[test]
    fn norm_cdf_deep_tail_matches_reference() {
        assert_close!(norm_cdf(-1.0), 0.158_655_253_931_457_05, 1e-15, 1e-18);
        assert_close!(norm_cdf(-5.0), 2.866_515_718_791_939e-7, 1e-12, 0.0);
        assert_close!(norm_cdf(-10.0), 7.619_853_024_160_525e-24, 1e-10, 0.0);
    }

    /// Tail symmetry `Φ(-x) = 1 − Φ(x)` must hold to high precision; a
    /// regression that lost the tail-stable `erfc` routing and reintroduced
    /// catastrophic cancellation would break this for large `x`. Also assert
    /// the moderately-deep tail does not underflow to exactly zero where a
    /// positive value is expected (used in deep-OTM digital/barrier pricing).
    #[test]
    fn norm_cdf_tail_symmetry_and_no_underflow() {
        // The symmetry identity Φ(-x) = 1 - Φ(x) can only be checked where
        // `1 - Φ(x)` is still representable: beyond x ≈ 8, Φ(x) rounds to 1.0
        // and the RHS underflows to 0 (precisely the catastrophic cancellation
        // the tail-stable `erfc` form in `norm_cdf` avoids on the LHS). We test
        // the identity at the largest x where the RHS is non-trivial; the ~1e-9
        // relative tolerance absorbs the RHS cancellation while still catching a
        // regression that lost the erfc routing.
        for x in [3.0_f64, 4.0, 5.0] {
            assert_close!(norm_cdf(-x), 1.0 - norm_cdf(x), 1e-9, 1e-300);
        }
        // The direct (erfc) path stays strictly positive and accurate deep in
        // the tail (reference values from a high-precision erfc), so a
        // regression that lost the erfc routing and reintroduced cancellation is
        // caught. The 1e-9 relative tolerance absorbs benign cross-erfc ULP
        // differences while remaining far tighter than any cancellation error.
        assert_close!(norm_cdf(-15.0), 3.670_966_199_312_858e-51, 1e-9, 0.0);
        assert_close!(norm_cdf(-20.0), 2.753_624_118_606_331e-89, 1e-9, 0.0);
        assert!(norm_cdf(-37.0) > 0.0); // ≈ 5.7e-300, still strictly > 0
    }

    #[test]
    fn norm_pdf_peak() {
        assert_close!(norm_pdf(0.0), INV_SQRT_2PI, 1e-15, 1e-15);
        assert_close!(norm_pdf(1.0), norm_pdf(-1.0), 1e-15, 1e-15);
    }
}
