//! `bench_gate` — the CI bench-regression gate. Two arms, both must pass:
//!
//! 1. **In-core ABSOLUTE §1.2 budget gate.** Runs the low-jitter, pinned,
//!    priority-elevated in-core measurement (`celnet_bench::core_load`) of
//!    `celnet_vanilla::greeks` (price + full Greek set) and asserts the
//!    **absolute** `docs/ARCHITECTURE.md` §1.2 ceilings — **p50 ≤ 2 µs,
//!    p99 ≤ 10 µs, p99.9 ≤ 25 µs** — failing if any *measured* percentile exceeds
//!    its committed budget number. This is the contract gate: it asserts the real
//!    budget, not a relative drift.
//!
//! 2. **Wire-path RELATIVE regression gate.** Runs a short, bounded wire-path
//!    load against a real in-process [`celnet_server::Edge`], then compares the
//!    measured RFQ round-trip percentiles to the committed baseline
//!    (`crates/celnet-bench/baselines/wire_path.json`) at a relative tolerance,
//!    failing if any gated percentile regresses beyond `baseline * (1 + tol)`.
//!    (The end-to-end loopback round-trip legitimately exceeds the *in-core* §1.2
//!    budget because it additionally pays the full gRPC/HTTP-2 framing + async⇄
//!    core hop; §1.2 is the in-core compute budget, gated in arm 1.)
//!
//! The process exits non-zero (failing CI) if **either** arm breaches.
//!
//! Usage:
//!
//! ```bash
//! source "$HOME/.cargo/env" && \
//!   timeout 200 cargo run --release -p celnet-bench --bin bench_gate
//! # custom wire baseline / tolerance:
//! #   bench_gate <baseline.json> <tolerance>
//! ```
//!
//! The wire tolerance defaults to `1.0` (allow up to 2× the baseline) because CI
//! runners are shared, variable-core hosts whose absolute latency floats
//! run-to-run; that arm catches a *structural* regression, not host jitter. The
//! §1.2 *absolute* arm has no such tolerance — it asserts the committed contract
//! number directly (it passes with ~hundreds-of-× margin, so jitter is a
//! non-issue). The committed baselines are measured on the M4 dev host and
//! recorded under `baselines/` + `benches/README.md`.

use std::time::Duration;

use celnet_bench::core_load::{CoreLoadConfig, measure};
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

    // ---------------------------------------------------------------------
    // Arm 1: in-core ABSOLUTE §1.2 budget gate (p50 ≤ 2µs / p99 ≤ 10µs /
    // p99.9 ≤ 25µs). The measurement is synchronous, pinned, bounded; run it on
    // a dedicated thread so it pins/elevates its own thread without touching the
    // tokio runtime that drives the wire arm below.
    // ---------------------------------------------------------------------
    println!("== arm 1: in-core ABSOLUTE §1.2 budget gate ==");
    println!();
    let core_report = std::thread::Builder::new()
        .name("core_load_gate".to_owned())
        .spawn(|| measure(CoreLoadConfig::default()))
        .map_err(|e| format!("could not spawn the in-core measurement thread: {e}"))?
        .join()
        .map_err(|_| "the in-core measurement thread panicked")?;
    core_report.print_summary();
    println!();
    let core_breaches = core_report.budget_breaches();
    if core_breaches.is_empty() {
        println!(
            "arm 1 PASSED: every §1.2 absolute budget met (p50 ≤ 2µs, p99 ≤ 10µs, p99.9 ≤ 25µs)."
        );
    } else {
        eprintln!(
            "arm 1 FAILED: {} §1.2 absolute budget(s) breached:",
            core_breaches.len()
        );
        for b in &core_breaches {
            eprintln!(
                "  {} = {:.3}µs ({} ns) > §1.2 ceiling {:.3}µs ({} ns)",
                b.metric,
                b.measured_ns as f64 / 1000.0,
                b.measured_ns,
                b.ceiling_ns as f64 / 1000.0,
                b.ceiling_ns,
            );
        }
    }
    println!();
    println!("== arm 2: wire-path RELATIVE regression gate ==");
    println!();

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
        println!("arm 2 PASSED: every gated wire-path percentile within tolerance.");
    } else {
        eprintln!(
            "arm 2 FAILED: {} wire-path percentile(s) regressed:",
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
    }

    // Both arms must pass.
    println!();
    if core_breaches.is_empty() && breaches.is_empty() {
        println!(
            "bench gate PASSED: in-core §1.2 absolute budgets met AND wire-path within tolerance."
        );
        Ok(())
    } else {
        eprintln!(
            "bench gate FAILED: {} in-core §1.2 breach(es), {} wire-path regression(s).",
            core_breaches.len(),
            breaches.len()
        );
        std::process::exit(1);
    }
}
