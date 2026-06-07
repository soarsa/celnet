//! **Algorithm-matched** standard-normal CDF and its inverse — the f64
//! evaluation of the *exact algorithm the WGSL path/Greeks kernels run in f32*.
//!
//! WGSL has no native `erf`, so the kernels evaluate `Φ` via the Abramowitz &
//! Stegun 7.1.26 rational-times-Gaussian approximation (max `erf` error
//! `1.5e-7`), and the inverse-normal `Φ⁻¹` via an Acklam rational seed polished
//! by **one Halley step against that A&S `Φ`**. The production `celnet-qmc`
//! inverse-normal instead polishes against the `libm::erf` `Φ` (exact).
//!
//! Using *these* f64-but-A&S-algorithm functions in the CPU oracle makes the
//! CPU↔GPU reconciliation isolate **pure f32 round-off** (small, bounded from
//! `f32::EPSILON`) — the A&S *algorithmic* error vs the `libm` inverse is a
//! SEPARATE, independently-derived bracket the parity rows pin. This is the same
//! two-bound discipline `batch.rs` uses for the closed-form kernel.
//!
//! # Method provenance (doc comments only)
//!
//! Abramowitz & Stegun, *Handbook of Mathematical Functions* (1964) §7.1.26;
//! Peter J. Acklam, *An algorithm for computing the inverse normal cumulative
//! distribution function* (2003); Halley's cubic-order root polish. Identifiers
//! are purpose-named.

/// `Φ(x)` via the A&S 7.1.26 `erf`, in f64 but with the f32 coefficient
/// bit-patterns the shader parses (mirrors the WGSL `erf_as`/`norm_cdf`).
#[must_use]
pub(crate) fn norm_cdf_as(x: f64) -> f64 {
    let inv_sqrt_2 = f64::from(std::f32::consts::FRAC_1_SQRT_2);
    let v = x * inv_sqrt_2;
    let s = v.signum();
    let z = v.abs();
    // f32 bit-pattern coefficients (shortest decimals that round to the shader's
    // f32 constants), widened to f64 — see batch.rs::erf_as_oracle.
    let p = f64::from(0.327_591_1_f32);
    let a1 = f64::from(0.254_829_6_f32);
    let a2 = f64::from(-0.284_496_72_f32);
    let a3 = f64::from(1.421_413_8_f32);
    let a4 = f64::from(-1.453_152_1_f32);
    let a5 = f64::from(1.061_405_4_f32);
    let tt = 1.0 / (1.0 + p * z);
    let poly = ((((a5 * tt + a4) * tt + a3) * tt + a2) * tt + a1) * tt;
    let erf = s * (1.0 - poly * celnet_core::math::exp(-z * z));
    0.5 * (1.0 + erf)
}

/// `Φ⁻¹(p)` using the A&S-erf `Φ` in its Halley polish — the f64 evaluation of
/// the GPU's `inv_norm_cdf` algorithm (Acklam seed + one Halley step against
/// [`norm_cdf_as`]).
#[must_use]
pub(crate) fn inv_norm_cdf_as(p: f64) -> f64 {
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
    const P_LOW: f64 = 0.024_25;
    const P_HIGH: f64 = 1.0 - 0.024_25;
    let ln = celnet_core::math::ln;
    let sqrt = celnet_core::math::sqrt;

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
    // One Halley step against the A&S-erf Φ (φ is the exact closed-form pdf, as
    // in the kernel).
    let e = norm_cdf_as(x) - p;
    let phi = celnet_core::math::norm_pdf(x);
    let u = e / phi;
    x -= u / (1.0 + 0.5 * x * u);
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_qmc::inv_norm_cdf;

    /// The A&S-erf inverse-normal brackets the production (`libm`-erf) inverse to
    /// the **A&S algorithmic error** `δ_z ≈ δ_Φ/φ(z)` (with δ_Φ = ½·1.5e-7) —
    /// the separate bracket that justifies isolating pure f32 round-off in the
    /// GPU↔CPU reconcile. This is derived, not fitted: the A&S 7.1.26 erf max
    /// error is the published `1.5e-7`.
    #[test]
    fn as_inverse_brackets_libm_inverse() {
        let d_phi = 0.5 * 1.5e-7;
        for &u in &[0.05, 0.2, 0.4, 0.5, 0.6, 0.8, 0.95, 0.99] {
            let z_as = inv_norm_cdf_as(u);
            let z_libm = inv_norm_cdf(u);
            let phi = celnet_core::math::norm_pdf(z_libm).max(1e-12);
            // δ_z = δ_Φ / φ(z), plus a small Halley-residual slack.
            let bound = d_phi / phi + 1e-8;
            assert!(
                (z_as - z_libm).abs() <= bound,
                "u={u}: A&S inv {z_as} vs libm inv {z_libm} exceeds A&S bound {bound}"
            );
        }
    }
}
