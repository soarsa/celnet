//! `bench_gate` — the CI bench-regression gate for the wire-path latency proof.
//!
//! Runs a short, bounded wire-path load against a real in-process
//! [`celnet_server::Edge`], then compares the measured RFQ round-trip
//! percentiles to the committed baseline
//! (`crates/celnet-bench/baselines/wire_path.json`) at a relative tolerance.
//! Exits non-zero (failing CI) if any gated percentile regresses beyond
//! `baseline * (1 + tolerance)`.
//!
//! Usage:
//!
//! ```bash
//! source "$HOME/.cargo/env" && \
//!   timeout 200 cargo run --release -p celnet-bench --bin bench_gate
//! # custom baseline / tolerance:
//! #   bench_gate <baseline.json> <tolerance>
//! ```
//!
//! Tolerance defaults to `1.0` (allow up to 2× the baseline) because CI runners
//! are shared, variable-core hosts whose absolute latency floats run-to-run; the
//! gate is tuned to catch a *real* regression (an order-of-magnitude tail blow-up
//! or a structural slowdown), not host jitter. The committed baseline is
//! measured on the M4 dev host and recorded in `benches/README.md`.

use std::time::Duration;

use celnet_bench::wire::{LoadConfig, WireReport, compare_to_baseline, run_load, start_ready_edge};

const DEFAULT_BASELINE: &str = "crates/celnet-bench/baselines/wire_path.json";
const DEFAULT_TOLERANCE: f64 = 1.0;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let baseline_path = args.next().unwrap_or_else(|| DEFAULT_BASELINE.to_owned());
    let tolerance: f64 = match args.next() {
        Some(s) => s.parse().map_err(|_| "tolerance must be a float")?,
        None => DEFAULT_TOLERANCE,
    };

    let baseline_json = std::fs::read_to_string(&baseline_path).map_err(|e| {
        format!("cannot read baseline {baseline_path}: {e} — refresh it with the wire_load binary")
    })?;
    let baseline: WireReport = serde_json::from_str(&baseline_json)
        .map_err(|e| format!("baseline {baseline_path} is not a valid WireReport: {e}"))?;

    // The gate uses the default (CI-sized) load: a stable tail estimate that
    // finishes in a couple of seconds, hard-capped well under the CI step timeout.
    let config = LoadConfig::default();
    let outer_cap = config.wall_clock_cap + Duration::from_secs(20);

    let measured = tokio::time::timeout(outer_cap, async {
        let (edge, addr) = start_ready_edge().await?;
        let report = run_load(addr, config)
            .await
            .map_err(std::io::Error::other)?;
        edge.shutdown(Duration::from_secs(5)).await;
        Ok::<_, std::io::Error>(report)
    })
    .await
    .map_err(|_| "bench_gate exceeded its outer wall-clock cap (edge unresponsive)")??;

    println!(
        "baseline: {baseline_path}  (tolerance: +{:.0}%)",
        tolerance * 100.0
    );
    println!();
    measured.print_summary();
    println!();
    println!("gate comparison (measured vs committed baseline):");
    println!(
        "  {:<8} {:>12} {:>12} {:>12}",
        "metric", "baseline_us", "measured_us", "ceiling_us"
    );
    for (name, base, meas) in [
        ("p50", baseline.rfq.p50_us, measured.rfq.p50_us),
        ("p99", baseline.rfq.p99_us, measured.rfq.p99_us),
        ("p99.9", baseline.rfq.p999_us, measured.rfq.p999_us),
        ("p99.99", baseline.rfq.p9999_us, measured.rfq.p9999_us),
    ] {
        let ceiling = base * (1.0 + tolerance);
        let mark = if meas > ceiling { "FAIL" } else { "ok" };
        println!("  {name:<8} {base:>12.2} {meas:>12.2} {ceiling:>12.2}   {mark}");
    }
    println!();

    let breaches = compare_to_baseline(&baseline, &measured, tolerance);
    if breaches.is_empty() {
        println!("bench-regression gate PASSED: every gated percentile within tolerance.");
        Ok(())
    } else {
        eprintln!(
            "bench-regression gate FAILED: {} percentile(s) regressed:",
            breaches.len()
        );
        for b in &breaches {
            eprintln!(
                "  {} = {:.2}us > ceiling {:.2}us (baseline {:.2}us + {:.0}%)",
                b.metric,
                b.measured_us,
                b.ceiling_us,
                b.baseline_us,
                tolerance * 100.0
            );
        }
        std::process::exit(1);
    }
}
