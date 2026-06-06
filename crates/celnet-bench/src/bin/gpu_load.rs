//! `gpu_load` — the GPU batch throughput + dispatch-latency load harness binary.
//!
//! Drives the **existing** [`celnet_gpu::GpuBackend`] Monte-Carlo pricing path
//! over a bounded batch-size sweep, times each dispatch into a
//! coordinated-omission-aware HdrHistogram, and prints per-batch dispatch latency
//! (p50/p99/p99.9) + instrument throughput (priced paths/s) + the host-local
//! GPU/CPU throughput RATIO. Optionally writes the committed
//! `baselines/gpu_batch.json` snapshot for the `gpu_gate` regression gate.
//!
//! # THE GPU HONEST BOUNDARY (reproduced verbatim; never violated)
//!
//! M4 Metal lacks f64, so the GPU path is f32; in-repo we prove CORRECTNESS
//! (f32↔f64 reconcile, three-way GPU-MC ~ CPU-MC ~ golden — those proofs live as
//! `#[test]`s in `celnet-gpu`) and RATIOS (a measured GPU/CPU speedup on THIS
//! host) — a RATIO and a relative-regression signal, NOT an absolute throughput.
//! The NVIDIA absolute throughput headline, the ≤ 50 ms exotic, and the
//! Workload-A/B absolute numbers are DEFERRED to the CUDA deploy-gate CI and are
//! NEVER claimed from this repo. This binary runs HEADLESS — when no adapter is
//! present the [`celnet_gpu::GpuBackend`] falls back to the CPU oracle and the run
//! is labelled a CPU-fallback / completes-within-ceiling check, not a speedup
//! claim.
//!
//! Usage:
//!
//! ```bash
//! source "$HOME/.cargo/env" && \
//!   cargo run --release -p celnet-bench --bin gpu_load
//! # refresh the committed relative-regression baseline (CI-sized sweep):
//! source "$HOME/.cargo/env" && \
//!   cargo run --release -p celnet-bench --bin gpu_load -- --ci \
//!     crates/celnet-bench/baselines/gpu_batch.json
//! ```
//!
//! Bounded by construction: a fixed batch sweep × fixed dispatch count, and the
//! backend's own bounded readback deadline ⇒ it always terminates (run under a
//! shell timeout regardless, per repo hygiene).

use celnet_bench::gpu_load::{GpuLoadConfig, measure};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Args: an optional `--ci` flag (smaller, gate-sized sweep) and an optional
    // output path to write the JSON snapshot baseline to.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let ci = args.first().map(String::as_str) == Some("--ci");
    if ci {
        args.remove(0);
    }
    let out_path = args.into_iter().next();

    let config = if ci {
        GpuLoadConfig::ci()
    } else {
        GpuLoadConfig::default()
    };

    let report = measure(config);
    report.print_summary();
    println!();

    if let Some(path) = &out_path {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(path, format!("{json}\n"))?;
        println!("wrote GPU batch baseline snapshot to {path}");
        println!();
    }

    // This binary is a measurement/report, not a pass/fail gate (that is `gpu_gate`,
    // a RELATIVE regression gate). It exits 0 once the bounded sweep completes —
    // honestly a completes-within-ceiling run on a headless host.
    println!(
        "gpu_load complete (RATIO / relative numbers only — the absolute GPU/NVIDIA throughput \
         headline stays deploy-gated, never claimed here)."
    );
    Ok(())
}
