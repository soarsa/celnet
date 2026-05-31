//! Celnet cross-platform GPU compute — a [`PricingBackend`] over wgpu
//! (Metal/Vulkan/DX12) with a deterministic CPU fallback (work-stream WS-E).
//!
//! # What this crate is
//!
//! A self-contained Monte-Carlo pricing backend for single-asset
//! geometric-Brownian-motion vanilla FX options, with two interchangeable
//! implementations behind one [`PricingBackend`] trait:
//!
//! * [`CpuBackend`] — a pure-Rust **f64** reference. It draws from a
//!   counter-based **Philox-4×32-10** RNG (seeded by `(global_seed, path, step,
//!   dim)`, bit-stable and order-independent), evolves GBM to expiry, and reduces
//!   the payoff with a deterministic pairwise (tree) sum. This is the validation
//!   oracle.
//! * [`GpuBackend`] — a **wgpu** compute path (WGSL) running the *same* Philox RNG
//!   in **f32** and an on-device GBM Monte-Carlo. It is selected at runtime by
//!   probing for an adapter and **falls back to [`CpuBackend`] when none is
//!   available**, so it runs headless in CI.
//!
//! # Numeric policy (see `docs/ARCHITECTURE.md` §4)
//!
//! WGSL/Metal have no f64, so GPU pricing standardizes on **f32**; a periodic
//! **f64 CPU reconciliation** bounds the error. The Philox round arithmetic is
//! exact `u32` on both sides, so the random *integers* are bit-identical; only
//! the integer→float conversion and the payoff arithmetic differ in precision.
//! Payoff accumulation uses an **integer/tree reduction** — never a float atomic
//! (WGSL atomics are integer-only) — so the reduction order is fixed and the
//! result reproducible.
//!
//! # Determinism
//!
//! All CPU transcendentals route through [`celnet_core::math`]
//! (`rust-lang/libm`), so the f64 stream is bit-identical across OS and
//! architecture. The RNG is counter-based and seeded, so any `(path, step, dim)`
//! variate is reproducible independent of evaluation order or backend.

#![forbid(unsafe_code)]

pub mod backend;
pub mod counter_rng;
pub mod cpu;
pub mod gpu;
pub mod scenario;

pub use backend::{PathSpec, PayoffKernel, PricingBackend, Reduction};
pub use counter_rng::{CounterAddress, CounterNormals};
pub use cpu::CpuBackend;
pub use gpu::GpuBackend;
pub use scenario::{ScenarioAxes, ScenarioGrid, ScenarioPricer};

/// The simulation precision a [`PricingBackend`] computes in.
///
/// Implemented for [`f64`] (the CPU oracle) and [`f32`] (the GPU path). The
/// bounds are the minimum needed to materialize, copy and (in tests) compare a
/// path buffer; the trait deliberately carries no arithmetic so it cannot leak a
/// non-deterministic float operation into a generic context.
pub trait Scalar: Copy + core::fmt::Debug + bytemuck::Pod + Send + Sync + 'static {
    /// Widen the scalar to f64 for f64-space reconciliation and reporting.
    fn to_f64(self) -> f64;
}

impl Scalar for f64 {
    #[inline]
    fn to_f64(self) -> f64 {
        self
    }
}

impl Scalar for f32 {
    #[inline]
    fn to_f64(self) -> f64 {
        f64::from(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two backends share one trait and one Philox stream; this cross-checks
    /// at the crate boundary that the public surface composes.
    #[test]
    fn public_surface_composes() {
        let spec = PathSpec::gbm(1.0, 0.1, 1.0, 0.02, 0.01, 1024, 1, 7);
        let payoff = PayoffKernel::call(1.0);

        let cpu = CpuBackend::new();
        let r_cpu = cpu.price_vanilla(&spec, &payoff);
        assert!(r_cpu.price() > 0.0);
        assert_eq!(cpu.label(), "cpu-f64");

        let gpu = GpuBackend::new();
        let r_gpu = gpu.price_vanilla(&spec, &payoff);
        assert!(r_gpu.price() > 0.0);

        // Scalar widening is the identity / lossless cast.
        assert_eq!(Scalar::to_f64(2.5_f64), 2.5);
        assert_eq!(Scalar::to_f64(0.5_f32), 0.5);
    }
}
