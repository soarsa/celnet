//! `surface_rebuild` — the per-pair surface-rebuild latency truth-gate binary.
//!
//! Runs a low-jitter, thread-pinned, priority-elevated measurement of a **full
//! single-pair-all-tenors surface rebuild** (calibrating every standard-tenor
//! smile slice from representative broker quotes and assembling the term
//! structure) for BOTH the Vanna-Volga baseline and the SSVI model, times each
//! full rebuild into a coordinated-omission-aware HdrHistogram, prints the full
//! tail (p50/p99/p99.9/p99.99/max) and single-core rebuild throughput, and
//! **asserts the absolute `docs/ARCHITECTURE.md` §1.2 budget**:
//!
//! * surface rebuild (single pair, all tenors) p99 ≤ 150 µs
//!
//! It exits non-zero on any breach (failing CI). The gate is asserted against
//! **each** model, so it only passes if the costlier SSVI rebuild also fits the
//! budget — the headline is honest. The measurement is engineered (pinning +
//! priority + warmup + ≥ 200k samples) so the *measured* tail reflects the
//! rebuild's intrinsic compute, not OS jitter.
//!
//! Usage:
//!
//! ```bash
//! source "$HOME/.cargo/env" && \
//!   cargo run --release -p celnet-bench --bin surface_rebuild
//! # refresh the committed §1.2 snapshot baseline:
//! source "$HOME/.cargo/env" && \
//!   cargo run --release -p celnet-bench --bin surface_rebuild -- \
//!     crates/celnet-bench/baselines/surface_rebuild.json
//! # smaller, faster sizing (still asserts §1.2):
//! source "$HOME/.cargo/env" && \
//!   cargo run --release -p celnet-bench --bin surface_rebuild -- --quick
//! ```
//!
//! Bounded by construction: a fixed sample count, no blocking I/O on the path, so
//! it always terminates (run under a shell timeout regardless, per repo hygiene).

use celnet_bench::surface_rebuild::{SurfaceLoadConfig, measure};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.first().map(String::as_str) == Some("--quick");
    if quick {
        args.remove(0);
    }
    let out_path = args.into_iter().next();

    let config = if quick {
        SurfaceLoadConfig::quick()
    } else {
        SurfaceLoadConfig::default()
    };

    let report = measure(config);
    report.print_summary();
    println!();

    if let Some(path) = &out_path {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(path, format!("{json}\n"))?;
        println!("wrote §1.2 surface-rebuild snapshot to {path}");
        println!();
    }

    // Assert the §1.2 ABSOLUTE budget. Any breach exits non-zero (fails CI).
    let breaches = report.budget_breaches();
    if breaches.is_empty() {
        println!(
            "§1.2 surface-rebuild budget gate PASSED: every model's all-tenors rebuild p99 ≤ 150µs — met with margin."
        );
        Ok(())
    } else {
        eprintln!(
            "§1.2 surface-rebuild budget gate FAILED: {} model(s) over the committed 150µs ceiling:",
            breaches.len()
        );
        for b in &breaches {
            eprintln!(
                "  {} {} = {:.3}µs ({} ns) > §1.2 ceiling {:.3}µs ({} ns)",
                b.model,
                b.metric,
                b.measured_ns as f64 / 1000.0,
                b.measured_ns,
                b.ceiling_ns as f64 / 1000.0,
                b.ceiling_ns,
            );
        }
        std::process::exit(1);
    }
}
