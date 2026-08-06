//! `celnet-server` binary entry point (work-stream WS-I).
//!
//! A deliberately **thin** binary: all logic lives in the library
//! ([`celnet_server`]). It builds a bootstrap market state, starts the async
//! [`Edge`] (the four gRPC services of the `celnet-proto` contract **plus** the
//! WebSocket JSON mirror of that same single contract — same services, same
//! pricing path, a second encoding for browsers/GUIs) over the configured bind
//! address, marks the readiness gate ready, and runs until `SIGINT`/`Ctrl-C`, at
//! which point it performs a graceful blue-green drain (`docs/ARCHITECTURE.md` §5).
//!
//! The bind addresses are read from the environment so the same binary serves any
//! deployment without recompilation:
//!
//! * `CELNET_GRPC_ADDR` — gRPC bind address (default `127.0.0.1:50051`).
//! * `CELNET_WS_ADDR` — WebSocket-mirror bind address. When **unset**, the mirror
//!   binds an OS-assigned ephemeral port on the gRPC host (the historical
//!   behaviour, fine for local demos). Set it to a fixed `HOST:PORT` so a reverse
//!   proxy (e.g. the HAProxy edge that fronts `app.uat.celnet.co.uk`) can target a
//!   stable backend — see `deploy/`.
//!
//! The bootstrap market state is the engine's calibrated EURUSD fixture; in a
//! full deployment a market-data adapter republishes live state through the same
//! [`celnet_server::CoreLink`] the binary exposes, with no restart.

use std::net::SocketAddr;
use std::time::Duration;

use celnet_engine::testing::make_state;
use celnet_server::{Clock, CoreLink, Edge, SpreadModel};
use celnet_types::{CcyPair, Tenor};

/// Parse a socket address from an env var, falling back to `default`.
fn addr_from_env(key: &str, default: &str) -> SocketAddr {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| default.parse().expect("valid default socket address"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Install the structured (line-delimited JSON) tracing subscriber BEFORE any
    // other work so every edge event — auth logins, RFQ quotes, the seed-admin
    // security warning, entitlement decisions — reaches stdout. `LogConfig`
    // defaults to INFO and honours `RUST_LOG` (the deploy runs `RUST_LOG=info`),
    // so INFO events emit by default.
    //
    // We install the MULTI-SINK subscriber: a LEAN combined stdout sink PLUS four
    // daily-rolling JSON file sinks fanned out by event `class` —
    // `orders.log` (order/risk/hedge/amend/transfer), `executions.log`
    // (execution), `pricing.log` (pricing) and `security.log` (the per-decision
    // entitlement + trade-lifecycle audit, the highest-volume class). The combined
    // stdout sink EXCLUDES those four routed classes, so celnetctl's `.out.log`
    // carries only the residue (startup/lifecycle, faults, degraded, unclassed)
    // and is readable again instead of ~99% entitlement audit. Sinks land under
    // `CELNET_LOG_DIR` (default `./logs`; the deploy points it at `celnet_log_dir`,
    // i.e. `/opt/celnet/shared/logs`), with a per-sink retention cap of
    // `CELNET_LOG_MAX_FILES` rolled files (default 14; `logrotate` is a
    // belt-and-braces backstop). The non-blocking `WorkerGuard`s MUST outlive the
    // process — bind them to `_log_guards` so they live until shutdown (a dropped
    // guard silently stops that file writer).
    //
    // If the split install fails (unwritable dir, or a global subscriber already
    // set by a wrapping harness) we fall back to the combined stdout-only
    // subscriber so structured logging is never lost, and carry on.
    let log_dir = std::env::var("CELNET_LOG_DIR").unwrap_or_else(|_| "./logs".to_string());
    let max_files = std::env::var("CELNET_LOG_MAX_FILES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(14);
    let log_cfg = celnet_observability::LogConfig {
        dir: Some(std::path::PathBuf::from(&log_dir)),
        max_files: Some(max_files),
        ..celnet_observability::LogConfig::default()
    };
    let _log_guards: Vec<celnet_observability::WorkerGuard> =
        match celnet_observability::install_split(&log_cfg) {
            Ok(guards) => {
                eprintln!(
                    "celnet-server: split log sinks under {log_dir}/ \
                     (orders.log, executions.log, pricing.log, security.log); \
                     lean combined → stdout"
                );
                guards
            }
            Err(e) => {
                eprintln!(
                    "celnet-server: split log sinks unavailable ({e}); \
                     falling back to combined stdout-only logging"
                );
                if let Err(e2) = celnet_observability::init_json_subscriber(
                    &celnet_observability::LogConfig::default(),
                ) {
                    eprintln!(
                        "celnet-server: structured logging subscriber not installed \
                         ({e2}); continuing"
                    );
                }
                Vec::new()
            }
        };

    let grpc_addr = addr_from_env("CELNET_GRPC_ADDR", "127.0.0.1:50051");
    // WS mirror: a fixed port when `CELNET_WS_ADDR` is set (so a reverse proxy can
    // target it), else `{grpc_ip}:0` — an OS-assigned ephemeral port, preserving the
    // historical default exactly (`start_on` treats port 0 as "ephemeral").
    let ws_addr = addr_from_env("CELNET_WS_ADDR", &format!("{}:0", grpc_addr.ip()));

    // Bootstrap market state: the engine's calibrated EURUSD 1Y fixture. A live
    // deployment republishes real state through the same `CoreLink`.
    let conv =
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    let initial = make_state(1.10, conv);

    // Start the pinned pricing core + async bridge (unpinned here; an operator
    // sets affinity via the deployment's isolated-core list).
    let link = CoreLink::start(initial, None);

    let edge = Edge::start_on(
        grpc_addr,
        ws_addr,
        std::sync::Arc::clone(&link),
        SpreadModel::default(),
        Clock::system(),
        // Production: the env-var-or-CWD default config locations (no per-process
        // data dir override — tests pass `Some(tempdir)` for isolated stores).
        None,
    )
    .await?;
    // The core is warm: open the `/readyz` gate.
    edge.gate().mark_ready();

    eprintln!(
        "celnet-server ready — gRPC {} | WS-mirror ws://{}",
        edge.grpc_addr(),
        edge.ws_addr()
    );

    // Run until Ctrl-C, then drain gracefully for a zero-loss cutover.
    tokio::signal::ctrl_c().await?;
    eprintln!("celnet-server draining for graceful shutdown…");
    edge.shutdown(Duration::from_secs(30)).await;
    link.stop();
    Ok(())
}
