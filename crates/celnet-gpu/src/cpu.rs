//! [`CpuBackend`] — the pure-Rust f64 Monte-Carlo reference and validation oracle.
//!
//! This backend is the *ground truth* against which the f32 GPU path is
//! reconciled. It draws standard normals from the counter-based
//! [`crate::philox::PhiloxNormals`] stream, evolves a single-asset
//! geometric-Brownian-motion to expiry, and reduces the vanilla payoff with a
//! **deterministic pairwise (tree) reduction** — never a single running float
//! accumulator — so floating-point non-associativity cannot make the result
//! depend on path order or chunking. All transcendentals route through
//! [`celnet_core::math`] (`rust-lang/libm`), so the f64 stream is bit-identical
//! across OS and architecture.

use crate::backend::{PathSpec, PayoffKernel, PricingBackend, Reduction};
use crate::philox::PhiloxNormals;

/// Pure-Rust f64 Monte-Carlo backend; the reconciliation oracle.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuBackend;

impl CpuBackend {
    /// Construct the CPU backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Terminal spot of path `p` under the spec's GBM, drawing the path's normal
    /// from the Philox stream. Exposed so the GPU reconciliation test can compare
    /// the f64 and f32 terminal spots path-by-path, not just the aggregate price.
    #[inline]
    #[must_use]
    pub fn terminal_spot(spec: &PathSpec, rng: &PhiloxNormals, p: u32) -> f64 {
        // One exact GBM step to expiry: ln S_T = ln S_0 + (μ − ½σ²)T + σ√T·Z.
        let half_var = 0.5 * spec.vol * spec.vol;
        let sqrt_t = celnet_core::math::sqrt(spec.t);
        let z = rng.normal(p, 0, 0);
        let ln_st = celnet_core::math::ln(spec.spot)
            + (spec.drift - half_var) * spec.t
            + spec.vol * sqrt_t * z;
        celnet_core::math::exp(ln_st)
    }
}

/// Deterministic pairwise (tree) sum of a slice: recursively halves the range and
/// adds the two partial sums, fixing the accumulation tree independent of length.
///
/// This is the f64 analogue of the integer/tree reduction the WGSL shader uses;
/// it removes the order-dependence of a naive left-fold and improves accuracy
/// (error grows as `O(log n)` rather than `O(n)`).
#[must_use]
pub(crate) fn pairwise_sum(xs: &[f64]) -> f64 {
    // Base case small enough to be exact-enough and cache-friendly.
    const BLOCK: usize = 64;
    if xs.len() <= BLOCK {
        let mut acc = 0.0;
        for &x in xs {
            acc += x;
        }
        return acc;
    }
    let mid = xs.len() / 2;
    pairwise_sum(&xs[..mid]) + pairwise_sum(&xs[mid..])
}

impl PricingBackend for CpuBackend {
    type Scalar = f64;

    fn label(&self) -> String {
        "cpu-f64".to_string()
    }

    fn simulate_paths(&self, spec: &PathSpec) -> Vec<f64> {
        let rng = PhiloxNormals::new(spec.seed);
        (0..spec.paths)
            .map(|p| Self::terminal_spot(spec, &rng, p))
            .collect()
    }

    fn reduce_payoff(
        &self,
        terminals: &[f64],
        payoff: &PayoffKernel,
        paths: u32,
        discount: f64,
    ) -> Reduction {
        let pay: Vec<f64> = terminals.iter().map(|&s| payoff.evaluate(s)).collect();
        let paysq: Vec<f64> = pay.iter().map(|&x| x * x).collect();
        Reduction {
            paths,
            sum: pairwise_sum(&pay),
            sum_sq: pairwise_sum(&paysq),
            discount,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;
    use celnet_types::{OptionType, VanillaInputs};

    /// The pairwise sum agrees with a high-precision serial sum and is invariant
    /// to where the slice is split.
    #[test]
    fn pairwise_sum_is_order_stable() {
        let xs: Vec<f64> = (0..10_000).map(|i| (f64::from(i) * 0.001).sin()).collect();
        let serial: f64 = xs.iter().sum();
        let tree = pairwise_sum(&xs);
        assert_close!(tree, serial, 1e-9, 1e-9);
        // Splitting the input and summing the halves reproduces the whole.
        let (a, b) = xs.split_at(3333);
        assert_close!(
            pairwise_sum(&xs),
            pairwise_sum(a) + pairwise_sum(b),
            0.0,
            1e-9
        );
    }

    /// Simulation is fully reproducible: two runs of the same spec are identical
    /// bit-for-bit (deterministic Philox + libm).
    #[test]
    fn simulation_is_reproducible() {
        let spec = PathSpec::gbm(1.20, 0.10, 0.5, 0.03, 0.01, 50_000, 1, 0xABCD_1234);
        let a = CpuBackend.simulate_paths(&spec);
        let b = CpuBackend.simulate_paths(&spec);
        assert_eq!(a, b);
    }

    /// The CPU Monte-Carlo price converges to the Garman-Kohlhagen closed form.
    /// With ~4M paths the standard error is small enough to bind tightly, and the
    /// estimate must sit within a few standard errors of the analytic price.
    #[test]
    fn mc_converges_to_closed_form() {
        let (s, k, vol, t, r_dom, r_for) = (1.20, 1.25, 0.11, 0.75, 0.03, 0.01);
        let spec = PathSpec::gbm(s, vol, t, r_dom, r_for, 4_000_000, 1, 0x5EED_0001);
        let payoff = PayoffKernel::call(k);
        let r = CpuBackend.price_vanilla(&spec, &payoff);

        let analytic = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(s, k, vol, t, r_dom, r_for),
        );
        let se = r.std_error();
        assert!(se > 0.0 && se < 1e-3, "unexpected std error: {se}");
        // Within 4 standard errors of the analytic price.
        assert!(
            (r.price() - analytic).abs() < 4.0 * se,
            "MC {} vs analytic {analytic} (4·se = {})",
            r.price(),
            4.0 * se
        );
    }

    /// A put as well, to exercise the other payoff sign.
    #[test]
    fn mc_put_converges_to_closed_form() {
        let (s, k, vol, t, r_dom, r_for) = (1.30, 1.25, 0.13, 1.0, 0.02, 0.015);
        let spec = PathSpec::gbm(s, vol, t, r_dom, r_for, 4_000_000, 1, 0x5EED_0002);
        let payoff = PayoffKernel::put(k);
        let r = CpuBackend.price_vanilla(&spec, &payoff);
        let analytic = celnet_vanilla::price(
            OptionType::Put,
            &VanillaInputs::new(s, k, vol, t, r_dom, r_for),
        );
        let se = r.std_error();
        assert!(
            (r.price() - analytic).abs() < 4.0 * se,
            "MC {} vs analytic {analytic}",
            r.price()
        );
    }
}
