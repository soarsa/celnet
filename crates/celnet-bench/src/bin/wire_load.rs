//! `wire_load` — the published end-to-end wire-path latency-under-load proof.
//!
//! Spins the real [`celnet_server::Edge`] in-process on an ephemeral loopback
//! port, drives sustained RFS streaming + concurrent RFQ load over the gRPC
//! wire, prints the client-observed RFQ round-trip histogram
//! (p50/p99/p99.9/p99.99 + throughput), and — when given a path argument —
//! writes the measured [`celnet_bench::wire::WireReport`] as JSON (used to
//! refresh the committed CI baseline).
//!
//! Bounded by construction: the load phase stops at a request budget *or* a hard
//! wall-clock cap, whichever comes first, then drains and shuts the edge down.
//! Run it under a shell timeout regardless (GUIDE.md test hygiene):
//!
//! ```bash
//! source "$HOME/.cargo/env" && \
//!   timeout 200 cargo run --release -p celnet-bench --bin wire_load
//! # refresh the committed baseline:
//! source "$HOME/.cargo/env" && \
//!   timeout 200 cargo run --release -p celnet-bench --bin wire_load -- \
//!     crates/celnet-bench/baselines/wire_path.json
//! ```

use std::time::Duration;

use celnet_bench::wire::{LoadConfig, run_load, start_ready_edge};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Args: an optional `--ci` flag selecting the lighter gate-sized workload
    // (so the committed CI baseline matches exactly what `bench_gate` measures),
    // and an optional path to write the JSON report to (baseline refresh).
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let ci = args.first().map(String::as_str) == Some("--ci");
    if ci {
        args.remove(0);
    }
    let out_path = args.into_iter().next();

    // A hard outer deadline so the whole process always terminates even if the
    // edge wedges: this is the load run's own self-termination, belt-and-braces.
    let config = if ci {
        LoadConfig::default()
    } else {
        LoadConfig::published_proof()
    };
    let outer_cap = config.wall_clock_cap + Duration::from_secs(30);

    let report = tokio::time::timeout(outer_cap, async {
        let (edge, addr) = start_ready_edge().await?;
        let report = run_load(addr, config)
            .await
            .map_err(std::io::Error::other)?;
        edge.shutdown(Duration::from_secs(5)).await;
        Ok::<_, std::io::Error>(report)
    })
    .await
    .map_err(|_| "wire_load exceeded its outer wall-clock cap (edge unresponsive)")??;

    report.print_summary();

    if let Some(path) = out_path {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(&path, format!("{json}\n"))?;
        println!("\nwrote report to {path}");
    }

    Ok(())
}
