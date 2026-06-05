//! `core_load` — the in-core **per-option** latency truth-gate binary.
//!
//! Runs a low-jitter, thread-pinned, priority-elevated measurement of
//! `celnet_vanilla::greeks` (price + the full 13-Greek set) under sustained,
//! varied-input injection, times **each** call into a coordinated-omission-aware
//! HdrHistogram, prints the full tail (p50/p99/p99.9/p99.99/max) and single-core
//! throughput, and **asserts the absolute `docs/ARCHITECTURE.md` §1.2 budgets**:
//!
//! * p50  ≤ 2 µs
//! * p99  ≤ 10 µs
//! * p99.9 ≤ 25 µs
//!
//! It exits non-zero on any breach (failing CI). These budgets pass with large
//! margin on the M4 dev host because the in-core op is ~tens of nanoseconds — the
//! measurement is engineered (pinning + priority + warmup + 10M samples) so the
//! *measured* tail reflects the core's intrinsic cost, not OS jitter.
//!
//! Usage:
//!
//! ```bash
//! source "$HOME/.cargo/env" && \
//!   cargo run --release -p celnet-bench --bin core_load
//! # refresh the committed §1.2 snapshot baseline:
//! source "$HOME/.cargo/env" && \
//!   cargo run --release -p celnet-bench --bin core_load -- \
//!     crates/celnet-bench/baselines/core_path.json
//! # smaller, faster sizing (still asserts §1.2):
//! source "$HOME/.cargo/env" && \
//!   cargo run --release -p celnet-bench --bin core_load -- --quick
//! ```
//!
//! Bounded by construction: a fixed sample count, no blocking I/O on the hot
//! path, so it always terminates (run under a shell timeout regardless, per repo
//! test hygiene).

use celnet_bench::core_load::{CoreLoadConfig, measure};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Args: an optional `--quick` flag (smaller sample budget for a fast local
    // run / CI smoke), and an optional path to write the JSON snapshot to.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.first().map(String::as_str) == Some("--quick");
    if quick {
        args.remove(0);
    }
    let out_path = args.into_iter().next();

    let config = if quick {
        CoreLoadConfig::quick()
    } else {
        CoreLoadConfig::default()
    };

    let report = measure(config);
    report.print_summary();
    println!();

    if let Some(path) = &out_path {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(path, format!("{json}\n"))?;
        println!("wrote §1.2 in-core snapshot to {path}");
        println!();
    }

    // Assert the §1.2 ABSOLUTE budgets. Any breach exits non-zero (fails CI).
    let breaches = report.budget_breaches();
    if breaches.is_empty() {
        println!(
            "§1.2 in-core budget gate PASSED: p50 ≤ 2µs, p99 ≤ 10µs, p99.9 ≤ 25µs — all met with margin."
        );
        Ok(())
    } else {
        eprintln!(
            "§1.2 in-core budget gate FAILED: {} percentile(s) over the committed ceiling:",
            breaches.len()
        );
        for b in &breaches {
            eprintln!(
                "  {} = {:.3}µs ({} ns) > §1.2 ceiling {:.3}µs ({} ns)",
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
