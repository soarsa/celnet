//! GPU batch warm-cache micro-throughput benchmarks (divan).
//!
//! The bounded `gpu_load` binary is the steady-state load harness + the committed
//! relative-regression baseline; this divan bench is its warm-cache micro-throughput
//! companion: it times the **existing** [`celnet_gpu::GpuBackend`] pricing a fixed
//! Monte-Carlo batch over a small batch-size sweep, with `input_counter` set to the
//! path count so divan reports **items/s (paths/s)** directly.
//!
//! # THE GPU HONEST BOUNDARY (reproduced verbatim; never violated)
//!
//! M4 Metal lacks f64, so the GPU path is f32; in-repo we prove CORRECTNESS
//! (f32↔f64 reconcile, three-way GPU-MC ~ CPU-MC ~ golden — `#[test]`s in
//! `celnet-gpu`) and RATIOS (a measured GPU/CPU speedup on THIS host) — a RATIO and
//! a relative-regression signal, NOT an absolute throughput. The NVIDIA absolute
//! throughput headline, the ≤ 50 ms exotic, and the Workload-A/B absolute numbers
//! are DEFERRED to the CUDA deploy-gate CI and are NEVER claimed from this repo.
//! This bench runs HEADLESS — with no adapter the [`celnet_gpu::GpuBackend`] falls
//! back to the CPU oracle, so the divan items/s is then the CPU oracle's, and the
//! number is a relative-regression signal only, never an absolute GPU claim.
//!
//! Benches are NOT nextest targets, so this does not affect `just check` / nextest.

use celnet_gpu::{CpuBackend, GpuBackend, PathSpec, PayoffKernel, PricingBackend};
use divan::{Bencher, black_box, counter::ItemsCount};

fn main() {
    divan::main();
}

/// The fixed batch-size sweep divan parametrizes the throughput bench over.
const BATCH_PATHS: [u32; 4] = [4_096, 16_384, 65_536, 262_144];

/// Build the representative at-the-money EUR/USD-style GBM spec for `paths`.
fn spec(paths: u32) -> PathSpec {
    PathSpec::gbm(1.10, 0.095, 0.5, 0.025, 0.015, paths, 1, 0x_C0FF_EE51)
}

fn payoff() -> PayoffKernel {
    PayoffKernel::call(1.10)
}

/// GPU-path (existing backend; CPU fallback headless) warm-cache batch throughput.
///
/// divan's `input_counter` is the path count, so the reported items/s is priced
/// paths/s — a RATIO / relative-regression signal on this host, never an absolute
/// GPU throughput claim (see the honest boundary above).
#[divan::bench(args = BATCH_PATHS)]
fn gpu_batch_throughput(bencher: Bencher, paths: u32) {
    let backend = GpuBackend::new();
    let s = spec(paths);
    let p = payoff();
    bencher
        .counter(ItemsCount::new(paths as usize))
        .bench_local(|| {
            let r = backend.price_vanilla(black_box(&s), black_box(&p));
            black_box(r.price())
        });
}

/// The f64 CPU-oracle warm-cache batch throughput on the identical batch — the
/// denominator of the host-local GPU/CPU ratio the `gpu_load` harness reports.
#[divan::bench(args = BATCH_PATHS)]
fn cpu_batch_throughput(bencher: Bencher, paths: u32) {
    let backend = CpuBackend::new();
    let s = spec(paths);
    let p = payoff();
    bencher
        .counter(ItemsCount::new(paths as usize))
        .bench_local(|| {
            let r = backend.price_vanilla(black_box(&s), black_box(&p));
            black_box(r.price())
        });
}
