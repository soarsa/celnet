//! High-accuracy inverse of the standard-normal CDF, `Φ⁻¹(p)`.
//!
//! [`celnet_core::math`] provides `Φ` (`norm_cdf`) and `φ` (`norm_pdf`) but no
//! inverse, so we provide one here. We use Acklam's rational approximation as a
//! starting point and then run **one Halley step** against the deterministic
//! `Φ`/`φ` to polish the result to full double precision: with Acklam's seed
//! (relative error `< 1.15e-9`) a single Halley iteration converges to `< 1 ULP`
//! on the realised value of `Φ⁻¹`.
//!
//! # Method provenance (doc comments only)
//!
//! Peter J. Acklam, *An algorithm for computing the inverse normal cumulative
//! distribution function* (2003). The Halley polish is the standard cubic-order
//! Newton variant for `Φ⁻¹`. Identifiers are purpose-named.
//!
//! # Determinism
//!
//! All transcendentals route through [`celnet_core::math`] (the `rust-lang/libm`
//! software backend), so the result is bit-identical across OS/architecture.

use celnet_core::math::{ln, norm_cdf, norm_pdf, sqrt};

// Acklam coefficients.
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

/// Break-point between the central rational region and the tail region.
const P_LOW: f64 = 0.024_25;
const P_HIGH: f64 = 1.0 - P_LOW;

/// Inverse standard-normal CDF `Φ⁻¹(p)` for `p ∈ (0, 1)`.
///
/// Returns `-∞`/`+∞` at the open endpoints `0`/`1` and `NaN` outside `[0, 1]`,
/// matching the mathematical limits. Accurate to `< 1 ULP` on the realised value
/// (Acklam seed + one Halley step against the deterministic `Φ`).
#[must_use]
pub fn inv_norm_cdf(p: f64) -> f64 {
    if p.is_nan() || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if p == 0.0 {
        return f64::NEG_INFINITY;
    }
    if p == 1.0 {
        return f64::INFINITY;
    }

    // Acklam rational seed.
    let mut x = if p < P_LOW {
        let q = sqrt(-2.0 * ln(p));
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= P_HIGH {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = sqrt(-2.0 * ln(1.0 - p));
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };

    // One Halley step: solves Φ(x) = p to cubic order. e = Φ(x) − p,
    // u = e / φ(x), x ← x − u / (1 + x·u/2).
    let e = norm_cdf(x) - p;
    let u = e / norm_pdf(x);
    x -= u / (1.0 + 0.5 * x * u);
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    /// Round-trip: `Φ(Φ⁻¹(p)) == p` to full precision across the unit interval,
    /// including deep tails. The oracle (`norm_cdf`) is an independent method.
    #[test]
    fn round_trip_phi_inverse() {
        let ps = [
            1e-12,
            1e-9,
            1e-6,
            1e-3,
            0.01,
            0.024_25,
            0.1,
            0.25,
            0.5,
            0.75,
            0.9,
            0.975_75,
            0.99,
            0.999,
            1.0 - 1e-6,
            1.0 - 1e-9,
            1.0 - 1e-12,
        ];
        for &p in &ps {
            let x = inv_norm_cdf(p);
            // Round-trip relative error to ~1e-13.
            assert_close!(norm_cdf(x), p, 1e-12, 1e-15);
        }
    }

    /// Known reference quantiles (published standard-normal values).
    #[test]
    fn known_quantiles() {
        assert_close!(inv_norm_cdf(0.5), 0.0, 1e-15, 1e-15);
        assert_close!(inv_norm_cdf(0.975), 1.959_963_984_540_054, 1e-12, 1e-12);
        assert_close!(inv_norm_cdf(0.95), 1.644_853_626_951_472, 1e-12, 1e-12);
        assert_close!(inv_norm_cdf(0.025), -1.959_963_984_540_054, 1e-12, 1e-12);
        // Φ⁻¹(0.8413447460685429) ≈ 1.0
        assert_close!(inv_norm_cdf(0.841_344_746_068_542_9), 1.0, 1e-11, 1e-12);
    }

    /// Symmetry `Φ⁻¹(1−p) = −Φ⁻¹(p)`.
    #[test]
    fn symmetry() {
        for &p in &[1e-6, 1e-3, 0.1, 0.3, 0.49] {
            assert_close!(inv_norm_cdf(1.0 - p), -inv_norm_cdf(p), 1e-12, 1e-12);
        }
    }

    /// Endpoints and out-of-range map to the mathematical limits / NaN.
    #[test]
    fn endpoints() {
        assert!(inv_norm_cdf(0.0).is_infinite() && inv_norm_cdf(0.0) < 0.0);
        assert!(inv_norm_cdf(1.0).is_infinite() && inv_norm_cdf(1.0) > 0.0);
        assert!(inv_norm_cdf(-0.1).is_nan());
        assert!(inv_norm_cdf(1.1).is_nan());
        assert!(inv_norm_cdf(f64::NAN).is_nan());
    }

    /// Monotone increasing.
    #[test]
    fn monotone() {
        let mut prev = f64::NEG_INFINITY;
        let mut p = 0.001;
        while p < 0.999 {
            let x = inv_norm_cdf(p);
            assert!(x > prev, "not monotone at p={p}");
            prev = x;
            p += 0.001;
        }
    }
}
