//! `bench_gate` — the CI bench-regression gate. Four arms, all must pass:
//!
//! 1. **In-core ABSOLUTE §1.2 budget gate.** Runs the low-jitter, pinned,
//!    priority-elevated in-core measurement (`celnet_bench::core_load`) of
//!    `celnet_vanilla::greeks` (price + full Greek set) and asserts the
//!    **absolute** `docs/ARCHITECTURE.md` §1.2 ceilings — **p50 ≤ 2 µs,
//!    p99 ≤ 10 µs, p99.9 ≤ 25 µs** — failing if any *measured* percentile exceeds
//!    its committed budget number. This is the contract gate: it asserts the real
//!    budget, not a relative drift.
//!
//! 1b. **Surface-rebuild ABSOLUTE §1.2 budget gate.** Runs the same low-jitter,
//!    pinned, priority-elevated discipline on the per-pair-all-tenors surface
//!    recompute (`celnet_bench::surface_rebuild`) for BOTH the Vanna-Volga and
//!    SSVI models, and asserts the **absolute** §1.2 ceiling for that workload —
//!    **surface rebuild (single pair, all tenors) p99 ≤ 150 µs** — failing if
//!    either model's measured recompute p99 exceeds it. Like arm 1 this asserts
//!    the committed contract number directly (it passes with multiple-× margin).
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
//! 3. **Fleet §11 SLO RELATIVE regression gate (LOOPBACK).** Re-measures the four
//!    fleet SLOs (`docs/SCALE-OUT.md` §11) that can be honestly measured without a
//!    real NIC — cross-shard federation overhead, publish→snapshot lag, conflation
//!    correctness under a stalled consumer, and many-sub fan-out spread — over the
//!    REAL fleet primitives on loopback, and compares to the committed LOOPBACK
//!    baseline (`crates/celnet-bench/baselines/fleet_slo.json`) at the same relative
//!    tolerance. This gates routing/conflation/fan-out ARITHMETIC + relative
//!    regression — it is **NOT** a claim about the absolute §11 wire-latency SLOs
//!    under a real NIC (those stay deploy-gated; see `docs/SCALE-OUT.md` §0/§11).
//!
//! The process exits non-zero (failing CI) if **any** arm breaches.
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
use celnet_bench::fleet_slo::{
    FleetSloConfig, FleetSloReport, compare_to_baseline as fleet_compare, measure as fleet_measure,
};
use celnet_bench::surface_rebuild::{SurfaceLoadConfig, measure as surface_measure};
use celnet_bench::wire::{LoadConfig, WireReport, compare_to_baseline, run_load, start_ready_edge};

const DEFAULT_BASELINE: &str = "crates/celnet-bench/baselines/wire_path.json";
const DEFAULT_TOLERANCE: f64 = 1.0;
const FLEET_BASELINE: &str = "crates/celnet-bench/baselines/fleet_slo.json";

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

    // ---------------------------------------------------------------------
    // Arm 1b: surface-rebuild ABSOLUTE §1.2 budget gate (per-pair-all-tenors
    // recompute p99 ≤ 150µs, for both the VV and SSVI models). Synchronous,
    // pinned, bounded — run it on a dedicated thread for the same reason as arm 1.
    // ---------------------------------------------------------------------
    println!("== arm 1b: surface-rebuild ABSOLUTE §1.2 budget gate ==");
    println!();
    let surface_report = std::thread::Builder::new()
        .name("surface_rebuild_gate".to_owned())
        .spawn(|| surface_measure(SurfaceLoadConfig::default()))
        .map_err(|e| format!("could not spawn the surface-rebuild measurement thread: {e}"))?
        .join()
        .map_err(|_| "the surface-rebuild measurement thread panicked")?;
    surface_report.print_summary();
    println!();
    let surface_breaches = surface_report.budget_breaches();
    if surface_breaches.is_empty() {
        println!(
            "arm 1b PASSED: every model's per-pair-all-tenors surface-rebuild p99 ≤ 150µs (§1.2)."
        );
    } else {
        eprintln!(
            "arm 1b FAILED: {} §1.2 surface-rebuild budget(s) breached:",
            surface_breaches.len()
        );
        for b in &surface_breaches {
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

    // ---------------------------------------------------------------------
    // Arm 3: fleet §11 SLO RELATIVE regression gate (LOOPBACK).
    //
    // Re-measures the four fleet-SLO arms (cross-shard federation overhead,
    // publish→snapshot lag, conflation correctness, many-sub fan-out spread) over
    // the REAL fleet primitives on loopback, and compares to the committed LOOPBACK
    // baseline (`fleet_slo.json`) at the SAME relative tolerance. This is a
    // routing/conflation/fan-out-ARITHMETIC + relative-regression gate — NOT a
    // claim about the absolute §11 wire-latency SLOs under a real NIC (those stay
    // deploy-gated; see docs/SCALE-OUT.md §0/§11). A >tolerance slowdown of any
    // gated metric fails CI.
    // ---------------------------------------------------------------------
    println!();
    println!("== arm 3: fleet §11 SLO RELATIVE regression gate (LOOPBACK) ==");
    println!();
    println!(
        "  HONEST BOUNDARY: loopback numbers — routing/conflation/fan-out arithmetic +\n  \
         relative regression + the architectural invariant. NOT the absolute §11 wire\n  \
         SLOs under a real NIC (deploy-gated)."
    );
    println!();
    let fleet_baseline_json = std::fs::read_to_string(FLEET_BASELINE).map_err(|e| {
        format!(
            "cannot read fleet baseline {FLEET_BASELINE}: {e} — refresh it with `fleet_slo --ci`"
        )
    })?;
    let fleet_baseline: FleetSloReport =
        serde_json::from_str(&fleet_baseline_json).map_err(|e| {
            format!("fleet baseline {FLEET_BASELINE} is not a valid FleetSloReport: {e}")
        })?;

    let fleet_config = FleetSloConfig::ci();
    let fleet_cap = fleet_config.wall_clock_cap + Duration::from_secs(40);
    let fleet_measured = tokio::time::timeout(fleet_cap, fleet_measure(fleet_config))
        .await
        .map_err(|_| "fleet SLO arm exceeded its wall-clock cap")?
        .map_err(std::io::Error::other)?;
    fleet_measured.print_summary();
    println!();
    println!(
        "fleet baseline: {FLEET_BASELINE}  (tolerance: +{:.0}%)",
        tolerance * 100.0
    );
    let fleet_breaches = fleet_compare(&fleet_baseline, &fleet_measured, tolerance);
    if fleet_breaches.is_empty() {
        println!("arm 3 PASSED: every gated fleet-SLO metric within tolerance (loopback).");
    } else {
        eprintln!(
            "arm 3 FAILED: {} fleet-SLO metric(s) regressed:",
            fleet_breaches.len()
        );
        for b in &fleet_breaches {
            eprintln!(
                "  {} = {:.3} > ceiling {:.3} (baseline {:.3} + {:.0}%)",
                b.metric,
                b.measured,
                b.ceiling,
                b.baseline,
                tolerance * 100.0
            );
        }
    }

    // All four arms must pass.
    println!();
    if core_breaches.is_empty()
        && surface_breaches.is_empty()
        && breaches.is_empty()
        && fleet_breaches.is_empty()
    {
        println!(
            "bench gate PASSED: in-core §1.2 absolute budgets met, surface-rebuild §1.2 absolute \
             budget met, wire-path within tolerance, AND fleet §11 loopback SLOs within tolerance."
        );
        Ok(())
    } else {
        eprintln!(
            "bench gate FAILED: {} in-core §1.2 breach(es), {} surface-rebuild §1.2 breach(es), \
             {} wire-path regression(s), {} fleet-SLO regression(s).",
            core_breaches.len(),
            surface_breaches.len(),
            breaches.len(),
            fleet_breaches.len()
        );
        std::process::exit(1);
    }
}
