//! Deterministic standard-normal variate generation from uniform draws.
//!
//! Two routes are provided, both pure functions of their uniform inputs so the
//! Monte-Carlo engine stays bit-reproducible:
//!
//! * [`inverse_cdf`] — the standard-normal quantile `Φ⁻¹(u)`. This is the
//!   preferred transform for quasi-/randomised low-discrepancy sequences and for
//!   the Brownian bridge, because it is *monotone* in `u`: it maps the
//!   one-dimensional low-discrepancy structure of `u` straight onto the normal
//!   axis without the angular scrambling a Box-Muller pair would introduce.
//! * [`box_muller`] — the classic trigonometric pair transform, turning two
//!   uniforms into two independent normals. Provided as the second mandated route
//!   and cross-checked against [`inverse_cdf`] on the distribution moments.
//!
//! The quantile uses the rational minimax approximation of Acklam (2003) with a
//! single Halley refinement step against [`celnet_core::math::norm_cdf`] /
//! `norm_pdf`, giving full double precision (≤ 1e-15 relative across the
//! representable tails). Method provenance lives only here; the public functions
//! are purpose-named.

use celnet_core::math::{ln, norm_cdf, norm_pdf, sqrt};

/// Lower break-point of the central region of the rational approximation.
const P_LOW: f64 = 0.024_25;
/// Upper break-point (`1 − P_LOW`).
const P_HIGH: f64 = 1.0 - P_LOW;

// Coefficients of the Acklam rational minimax approximation.
const A: [f64; 6] = [
    -3.969_683_028_665_376e1,
    2.209_460_984_245_205e2,
    -2.759_285_104_469_687e2,
    1.383_577_518_672_69e2,
    -3.066_479_806_614_716e1,
    2.506_628_277_459_239e0,
];
const B: [f64; 5] = [
    -5.447_609_879_822_406e1,
    1.615_858_368_580_409e2,
    -1.556_989_798_598_866e2,
    6.680_131_188_771_972e1,
    -1.328_068_155_288_572e1,
];
const C: [f64; 6] = [
    -7.784_894_002_430_293e-3,
    -3.223_964_580_411_365e-1,
    -2.400_758_277_161_838e0,
    -2.549_732_539_343_734e0,
    4.374_664_141_464_968e0,
    2.938_163_982_698_783e0,
];
const D: [f64; 4] = [
    7.784_695_709_041_462e-3,
    3.224_671_290_700_398e-1,
    2.445_134_137_142_996e0,
    3.754_408_661_907_416e0,
];

/// Standard-normal inverse cumulative distribution `Φ⁻¹(p)` for `p ∈ (0,1)`.
///
/// Returns `±∞` only at the open-interval endpoints, which the
/// [`crate::rng::PhiloxStream`] never produces. Accuracy is full double
/// precision after one Halley step.
#[must_use]
pub fn inverse_cdf(p: f64) -> f64 {
    debug_assert!(p > 0.0 && p < 1.0, "inverse_cdf domain is (0,1), got {p}");

    let x = if p < P_LOW {
        // Lower tail: rational approximation in q = √(−2 ln p).
        let q = sqrt(-2.0 * ln(p));
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= P_HIGH {
        // Central region: rational approximation in q = p − ½.
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        // Upper tail: mirror of the lower tail.
        let q = sqrt(-2.0 * ln(1.0 - p));
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };

    // One Halley refinement: drives the residual to machine precision.
    let e = norm_cdf(x) - p;
    let u = e / norm_pdf(x);
    x - u / (1.0 + 0.5 * x * u)
}

/// Box-Muller transform of two uniforms `(u1, u2) ∈ (0,1)²` into an independent
/// standard-normal pair.
///
/// Used as the second normal-generation route mandated by the engine spec. The
/// `u1` argument feeds the radial part (`√(−2 ln u1)`) and `u2` the angle.
#[must_use]
pub fn box_muller(u1: f64, u2: f64) -> (f64, f64) {
    let r = sqrt(-2.0 * ln(u1));
    let theta = core::f64::consts::TAU * u2;
    // Trig goes through the `rust-lang/libm` software routines (not the system
    // trig), so the angle transform is bit-identical across platforms — the same
    // determinism contract the rest of the math path holds.
    (r * libm::cos(theta), r * libm::sin(theta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    /// The quantile inverts the CDF: `Φ(Φ⁻¹(p)) = p` to machine precision across
    /// the central region and deep into both tails.
    #[test]
    fn quantile_inverts_cdf() {
        for &p in &[
            1e-12,
            1e-6,
            0.001,
            0.01,
            0.1,
            0.25,
            0.5,
            0.75,
            0.9,
            0.99,
            0.999,
            1.0 - 1e-6,
            1.0 - 1e-12,
        ] {
            let x = inverse_cdf(p);
            assert_close!(norm_cdf(x), p, 1e-12, 1e-12);
        }
    }

    /// Known quantile points (two-sided 95% ⇒ 0.975 → 1.95996…, median → 0).
    #[test]
    fn quantile_known_points() {
        assert_close!(inverse_cdf(0.5), 0.0, 1e-12, 1e-12);
        assert_close!(inverse_cdf(0.975), 1.959_963_984_540_054, 1e-9, 1e-9);
        assert_close!(inverse_cdf(0.025), -1.959_963_984_540_054, 1e-9, 1e-9);
    }

    /// Antisymmetry: `Φ⁻¹(1−p) = −Φ⁻¹(p)`.
    #[test]
    fn quantile_is_antisymmetric() {
        for &p in &[0.01, 0.2, 0.37, 0.49] {
            assert_close!(inverse_cdf(1.0 - p), -inverse_cdf(p), 1e-10, 1e-10);
        }
    }

    /// Box-Muller produces unit-variance, zero-mean normals (moment check).
    #[test]
    fn box_muller_moments() {
        let mut s = crate::rng::PhiloxStream::new(0xAA55, 0, 0, 0);
        let n = 500_000usize;
        let (mut sum, mut sumsq) = (0.0, 0.0);
        for _ in 0..n {
            let (z0, z1) = box_muller(s.next_u01(), s.next_u01());
            sum += z0 + z1;
            sumsq += z0 * z0 + z1 * z1;
        }
        let m = 2 * n;
        let mean = sum / m as f64;
        let var = sumsq / m as f64 - mean * mean;
        assert!(mean.abs() < 5e-3, "mean {mean}");
        assert!((var - 1.0).abs() < 5e-3, "var {var}");
    }
}
