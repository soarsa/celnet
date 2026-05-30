//! `celnet-server` binary entry point (work-stream WS-I).
//!
//! A deliberately **thin** binary: all logic lives in the library
//! ([`celnet_server`]). It builds a bootstrap market state, starts the async
//! [`Edge`] (the four gRPC services of the `celnet-proto` contract) over the
//! configured bind address, marks the readiness gate ready, and runs until
//! `SIGINT`/`Ctrl-C`, at which point it performs a graceful blue-green drain
//! (`docs/ARCHITECTURE.md` §5).
//!
//! The bind address is read from the environment so the same binary serves any
//! deployment without recompilation:
//!
//! * `CELNET_GRPC_ADDR` — gRPC bind address (default `127.0.0.1:50051`).
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
    let grpc_addr = addr_from_env("CELNET_GRPC_ADDR", "127.0.0.1:50051");

    // Bootstrap market state: the engine's calibrated EURUSD 1Y fixture. A live
    // deployment republishes real state through the same `CoreLink`.
    let conv =
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    let initial = make_state(1.10, conv);

    // Start the pinned pricing core + async bridge (unpinned here; an operator
    // sets affinity via the deployment's isolated-core list).
    let link = CoreLink::start(initial, None);

    let edge = Edge::start(
        grpc_addr,
        std::sync::Arc::clone(&link),
        SpreadModel::default(),
        Clock::system(),
    )
    .await?;
    // The core is warm: open the `/readyz` gate.
    edge.gate().mark_ready();

    eprintln!("celnet-server ready — gRPC {}", edge.grpc_addr());

    // Run until Ctrl-C, then drain gracefully for a zero-loss cutover.
    tokio::signal::ctrl_c().await?;
    eprintln!("celnet-server draining for graceful shutdown…");
    edge.shutdown(Duration::from_secs(30)).await;
    link.stop();
    Ok(())
}
