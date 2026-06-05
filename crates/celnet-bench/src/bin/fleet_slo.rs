//! `fleet_slo` — the fleet §11 SLO **truth-bench** binary (LOOPBACK / in-process).
//!
//! Measures the four fleet SLOs from `docs/SCALE-OUT.md` §11 that can be honestly
//! measured **without a real NIC**, each with a coordinated-omission-aware
//! HdrHistogram over the **real** fleet primitives:
//!
//! * (a) cross-shard **federation overhead** — federated `AggregateRisk` (a
//!   `Distributed{[backend]}` edge hop) vs a direct single-backend call, real
//!   loopback gRPC on both legs;
//! * (b) **publish→snapshot** visibility lag over the engine's `arc-swap`
//!   `StateHandle`;
//! * (c) **conflation correctness** under a fully-stalled bounded-`mpsc` consumer
//!   (the `stream.rs` `try_send` last-value-wins primitive): producer throughput +
//!   p99 unchanged, queue depth bounded;
//! * (d) many-sub **fan-out** delivery-window spread at N subscribers.
//!
//! # HONEST BOUNDARY
//!
//! These are **loopback** numbers. They prove routing / conflation / fan-out
//! ARITHMETIC + relative regression + the architectural invariant. They are **NOT**
//! the absolute §11 wire-latency SLOs under a real NIC — those stay deploy-gated
//! (`docs/SCALE-OUT.md` §0/§11). Do not present any number here as a *met* absolute
//! wire SLO.
//!
//! Bounded by construction: every arm is sample-bounded and the whole run is
//! wall-clock-capped, so it always terminates (run under a shell `timeout`
//! regardless, per repo test hygiene):
//!
//! ```bash
//! source "$HOME/.cargo/env" && \
//!   timeout 200 cargo run --release -p celnet-bench --bin fleet_slo
//! # refresh the committed LOOPBACK baseline (CI-sized workload):
//! source "$HOME/.cargo/env" && \
//!   timeout 200 cargo run --release -p celnet-bench --bin fleet_slo -- --ci \
//!     crates/celnet-bench/baselines/fleet_slo.json
//! ```

use std::time::Duration;

use celnet_bench::fleet_slo::{FleetSloConfig, measure};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Args: an optional `--ci` flag selecting the lighter gate-sized workload (so
    // the committed baseline matches what `bench_gate` measures), and an optional
    // path to write the JSON report to (baseline refresh).
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let ci = args.first().map(String::as_str) == Some("--ci");
    if ci {
        args.remove(0);
    }
    let out_path = args.into_iter().next();

    let config = if ci {
        FleetSloConfig::ci()
    } else {
        FleetSloConfig::default()
    };
    // A hard outer deadline so the whole process always terminates even if a gRPC
    // backend wedges (belt-and-braces over the per-arm sample bounds).
    let outer_cap = config.wall_clock_cap + Duration::from_secs(30);

    let report = tokio::time::timeout(outer_cap, measure(config))
        .await
        .map_err(|_| "fleet_slo exceeded its outer wall-clock cap (a backend wedged)")?
        .map_err(std::io::Error::other)?;

    report.print_summary();

    if let Some(path) = out_path {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(&path, format!("{json}\n"))?;
        println!("\nwrote LOOPBACK fleet-SLO report to {path}");
    }

    Ok(())
}
