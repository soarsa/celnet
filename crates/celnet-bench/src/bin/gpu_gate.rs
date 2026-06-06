//! `gpu_gate` — the GPU batch **relative-regression** gate (mirrors `bench_gate`).
//!
//! Re-measures the GPU batch sweep (`celnet_bench::gpu_load`) over the **existing**
//! [`celnet_gpu::GpuBackend`] and compares it to the committed baseline
//! (`crates/celnet-bench/baselines/gpu_batch.json`) at a relative tolerance,
//! failing CI on a **structural slowdown** — throughput collapsing below
//! `baseline / (1 + tol)` (default tol = 1.0 ⇒ a >2× throughput drop) or dispatch
//! p99 inflating beyond `baseline * (1 + tol)` (a >2× latency regression).
//!
//! # THE GPU HONEST BOUNDARY (reproduced verbatim; never violated)
//!
//! This is explicitly a **RELATIVE / ratio** gate, NOT an absolute-throughput
//! assertion. Absolute GPU numbers float with the device, so M4 Metal lacks f64
//! and the GPU path is f32; in-repo we prove CORRECTNESS (f32↔f64 reconcile,
//! three-way GPU-MC ~ CPU-MC ~ golden — `#[test]`s in `celnet-gpu`) and RATIOS (a
//! measured GPU/CPU speedup on THIS host). The NVIDIA absolute throughput
//! headline, the ≤ 50 ms exotic, and the Workload-A/B absolute numbers are
//! DEFERRED to the CUDA deploy-gate CI and are NEVER claimed here. On a headless CI
//! runner (Lavapipe / none) the [`celnet_gpu::GpuBackend`] falls back to the CPU
//! oracle, so this gate degrades to a **completes-within-ceiling** check, not a
//! speedup claim.
//!
//! Usage:
//!
//! ```bash
//! source "$HOME/.cargo/env" && \
//!   timeout 200 cargo run --release -p celnet-bench --bin gpu_gate
//! # custom baseline / tolerance:
//! #   gpu_gate <baseline.json> <tolerance>
//! ```
//!
//! The tolerance defaults to `1.0` (allow up to 2× the baseline) because CI
//! runners are shared, variable hosts whose absolute throughput floats run-to-run;
//! this arm catches a *structural* regression, not host jitter.

use celnet_bench::gpu_load::{GpuBatchReport, GpuLoadConfig, compare_to_baseline, measure};

const DEFAULT_BASELINE: &str = "crates/celnet-bench/baselines/gpu_batch.json";
const DEFAULT_TOLERANCE: f64 = 1.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let baseline_path = args.next().unwrap_or_else(|| DEFAULT_BASELINE.to_owned());
    let tolerance: f64 = match args.next() {
        Some(s) => s.parse().map_err(|_| "tolerance must be a float")?,
        None => DEFAULT_TOLERANCE,
    };

    let baseline_json = std::fs::read_to_string(&baseline_path).map_err(|e| {
        format!(
            "cannot read baseline {baseline_path}: {e} — refresh it with `gpu_load --ci <path>`"
        )
    })?;
    let baseline: GpuBatchReport = serde_json::from_str(&baseline_json)
        .map_err(|e| format!("baseline {baseline_path} is not a valid GpuBatchReport: {e}"))?;

    println!("== GPU batch RELATIVE-regression gate (ratio, NOT absolute throughput) ==");
    println!();
    println!(
        "  HONEST BOUNDARY: relative / ratio numbers on THIS host only. The NVIDIA absolute\n  \
         throughput headline + the ≤50ms exotic stay deploy-gated and are NEVER claimed here.\n  \
         Headless (no adapter) ⇒ CPU fallback ⇒ this is a completes-within-ceiling check."
    );
    println!();

    // Re-measure with the same CI-sized sweep the baseline was captured with.
    let measured = measure(GpuLoadConfig::ci());
    measured.print_summary();
    println!();

    println!(
        "baseline: {baseline_path}  (tolerance: +{:.0}%, slowdown-only)",
        tolerance * 100.0
    );
    println!("  baseline backend: {}", baseline.backend_label);
    println!("  measured backend: {}", measured.backend_label);
    println!();
    println!("gate comparison (measured vs committed baseline, per batch size):");
    println!(
        "  {:>10} {:>16} {:>16} {:>16}",
        "paths", "base_kpaths/s", "meas_kpaths/s", "floor_kpaths/s"
    );
    for base in &baseline.points {
        if let Some(meas) = measured.points.iter().find(|p| p.paths == base.paths) {
            let floor = base.throughput_paths_per_s / (1.0 + tolerance);
            let mark = if meas.throughput_paths_per_s < floor {
                "FAIL"
            } else {
                "ok"
            };
            println!(
                "  {:>10} {:>16.1} {:>16.1} {:>16.1}   {}",
                base.paths,
                base.throughput_paths_per_s / 1000.0,
                meas.throughput_paths_per_s / 1000.0,
                floor / 1000.0,
                mark
            );
        }
    }
    println!();

    let breaches = compare_to_baseline(&baseline, &measured, tolerance);
    if breaches.is_empty() {
        println!(
            "gpu_gate PASSED: every gated GPU batch metric within tolerance (relative / ratio — \
             throughput did not collapse, dispatch p99 did not inflate)."
        );
        Ok(())
    } else {
        eprintln!(
            "gpu_gate FAILED: {} GPU batch metric(s) regressed:",
            breaches.len()
        );
        for b in &breaches {
            eprintln!(
                "  {} = {:.1} vs bound {:.1} (baseline {:.1}, tol +{:.0}%)",
                b.metric,
                b.measured,
                b.bound,
                b.baseline,
                tolerance * 100.0
            );
        }
        std::process::exit(1);
    }
}
