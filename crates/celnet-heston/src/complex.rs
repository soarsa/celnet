//! A minimal, dependency-free complex-number type for the Heston characteristic
//! function.
//!
//! The Heston (1993) characteristic function is an analytic function of a complex
//! argument; evaluating it requires complex `exp`, `ln`, `sqrt`, division and the
//! four arithmetic operations. We implement exactly those here over [`f64`] real
//! and imaginary parts, routing the underlying transcendentals through
//! [`celnet_core::math`] / [`libm`] so the result is **bit-identical across
//! platform and architecture** — the same determinism guarantee the rest of the
//! pricing stack relies on. (Pulling in an external `num-complex` crate would
//! add a dependency for a handful of operations and would not give the
//! cross-platform-deterministic transcendental guarantee.)

use celnet_core::math::{exp, ln, sqrt};

/// A complex number `re + i·im`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Complex {
    /// Real part.
    pub(crate) re: f64,
    /// Imaginary part.
    pub(crate) im: f64,
}

impl Complex {
    /// The complex number `re + i·im`.
    #[inline]
    #[must_use]
    pub(crate) const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    /// A purely real complex number `x + 0i`.
    #[inline]
    #[must_use]
    pub(crate) const fn real(x: f64) -> Self {
        Self { re: x, im: 0.0 }
    }

    /// Complex addition.
    #[inline]
    #[must_use]
    pub(crate) fn add(self, o: Self) -> Self {
        Self::new(self.re + o.re, self.im + o.im)
    }

    /// Complex subtraction.
    #[inline]
    #[must_use]
    pub(crate) fn sub(self, o: Self) -> Self {
        Self::new(self.re - o.re, self.im - o.im)
    }

    /// Complex multiplication.
    #[inline]
    #[must_use]
    pub(crate) fn mul(self, o: Self) -> Self {
        Self::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }

    /// Complex division `self / o`. Uses Smith's scaled algorithm to avoid
    /// intermediate overflow/underflow when one component dominates.
    #[inline]
    #[must_use]
    pub(crate) fn div(self, o: Self) -> Self {
        if o.re.abs() >= o.im.abs() {
            let r = o.im / o.re;
            let den = o.re + o.im * r;
            Self::new((self.re + self.im * r) / den, (self.im - self.re * r) / den)
        } else {
            let r = o.re / o.im;
            let den = o.re * r + o.im;
            Self::new((self.re * r + self.im) / den, (self.im * r - self.re) / den)
        }
    }

    /// Scale by a real factor.
    #[inline]
    #[must_use]
    pub(crate) fn scale(self, k: f64) -> Self {
        Self::new(self.re * k, self.im * k)
    }

    /// Modulus `|z| = √(re² + im²)`.
    #[inline]
    #[must_use]
    pub(crate) fn abs(self) -> f64 {
        // hypot avoids overflow when one component is large.
        libm::hypot(self.re, self.im)
    }

    /// Argument (principal value) `atan2(im, re) ∈ (−π, π]`.
    #[inline]
    #[must_use]
    pub(crate) fn arg(self) -> f64 {
        libm::atan2(self.im, self.re)
    }

    /// Complex exponential `e^z = e^{re}·(cos im + i·sin im)`.
    #[inline]
    #[must_use]
    pub(crate) fn exp(self) -> Self {
        let m = exp(self.re);
        Self::new(m * libm::cos(self.im), m * libm::sin(self.im))
    }

    /// Principal complex logarithm `ln z = ln|z| + i·arg z`.
    #[inline]
    #[must_use]
    pub(crate) fn ln(self) -> Self {
        Self::new(ln(self.abs()), self.arg())
    }

    /// Principal complex square root.
    ///
    /// Computed from the real modulus/argument so the branch cut is the standard
    /// negative real axis; `√z` lands in the right half-plane (re ≥ 0), the
    /// principal branch used throughout the Heston characteristic function.
    #[inline]
    #[must_use]
    pub(crate) fn sqrt(self) -> Self {
        let m = sqrt(self.abs());
        let half = 0.5 * self.arg();
        Self::new(m * libm::cos(half), m * libm::sin(half))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn close(a: Complex, b: Complex, tol: f64) {
        assert_close!(a.re, b.re, tol, tol);
        assert_close!(a.im, b.im, tol, tol);
    }

    #[test]
    fn arithmetic_matches_hand_computation() {
        let a = Complex::new(1.0, 2.0);
        let b = Complex::new(3.0, -1.0);
        close(a.add(b), Complex::new(4.0, 1.0), 1e-15);
        close(a.sub(b), Complex::new(-2.0, 3.0), 1e-15);
        // (1+2i)(3-1i) = 3 -1i +6i -2i² = 3 +5i +2 = 5 +5i
        close(a.mul(b), Complex::new(5.0, 5.0), 1e-15);
    }

    #[test]
    fn division_is_inverse_of_multiplication() {
        let a = Complex::new(0.7, -1.3);
        let b = Complex::new(-2.1, 0.4);
        close(a.mul(b).div(b), a, 1e-13);
        // Exercise both branches of Smith's algorithm.
        let big_re = Complex::new(1e6, 1.0);
        let big_im = Complex::new(1.0, 1e6);
        close(a.mul(big_re).div(big_re), a, 1e-9);
        close(a.mul(big_im).div(big_im), a, 1e-9);
    }

    #[test]
    fn exp_ln_sqrt_against_known_identities() {
        // e^{iπ} = -1.
        close(
            Complex::new(0.0, core::f64::consts::PI).exp(),
            Complex::real(-1.0),
            1e-12,
        );
        // ln(e^z) = z for z in the principal strip.
        let z = Complex::new(0.3, 1.1);
        close(z.exp().ln(), z, 1e-12);
        // (√z)² = z.
        let w = Complex::new(-0.4, 2.0);
        close(w.sqrt().mul(w.sqrt()), w, 1e-12);
        // √(-1) = i (principal branch).
        close(Complex::real(-1.0).sqrt(), Complex::new(0.0, 1.0), 1e-12);
    }
}
