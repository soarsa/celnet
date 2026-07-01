//! The **federating front edge** for the out-of-process scale harness — a real
//! standalone OS process running `celnet-server` in `Distributed` mode.
//!
//! This is the stateless router/aggregator tier. It holds **no** book of its own: it
//! reads the backend list from the environment (`CELNET_FLEET_MODE=distributed` +
//! `CELNET_FLEET_BACKENDS=<comma-separated http urls>`, resolved by the same
//! `Edge::start` env path the production binary uses), dials each backend once at
//! boot, and then:
//!   * forwards every unary `Price` / `MarkSurface` / `GetSmile` / `Scenario` /
//!     `RequestQuote` / `AcceptQuote` to the backend that OWNS the request's pair
//!     (HRW owned-pair forwarding, `docs/SCALE-OUT.md` §3);
//!   * relays the multiplexed RFS `StreamSession` (subscribe / ticks / click-to-trade)
//!     to the owning backend;
//!   * federates every `RiskService` RPC across all backends and reconciles the
//!     gathered union to the single-node answer (`docs/RISK-HIERARCHY.md` §3.4).
//!
//! Because the edge is stateless, it **scales by being (re)started with a new backend
//! list** — the honest "stateless router scales for free" model: there is no edge-side
//! state to migrate, so scale up/down is a fresh process pointed at the new fleet. The
//! driver does exactly that.
//!
//! Environment:
//!   * `CELNET_GRPC_ADDR`     — the front edge's gRPC bind address (clients dial this).
//!   * `CELNET_FLEET_MODE`    — `distributed` (set by the driver).
//!   * `CELNET_FLEET_BACKENDS`— comma-separated backend `http://host:port` urls.
//!
//! On readiness it prints, to stdout, a single machine-parsable line the driver waits
//! for:
//!   `EDGE grpc=<addr> backends=<count> READY`
//! then runs until `SIGINT`/`SIGTERM`.

use std::net::SocketAddr;
use std::time::Duration;

use celnet_engine::testing::make_state;
use celnet_server::{Clock, CoreLink, Edge, SpreadModel};
use celnet_types::{CcyPair, Tenor};

fn env_addr(key: &str) -> SocketAddr {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("{key} must be set by the scale harness"))
        .parse()
        .unwrap_or_else(|_| panic!("{key} is not a valid socket address"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let grpc_addr = env_addr("CELNET_GRPC_ADDR");

    // The edge has no book of its own; it still needs a (never-priced-against locally
    // in distributed mode) core link to satisfy the edge constructor. A distributed
    // edge forwards every pricing request to the owning backend.
    let conv =
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    let initial = make_state(1.10, conv);
    let link = CoreLink::start(initial, None);

    // `Edge::start` reads CELNET_FLEET_MODE / CELNET_FLEET_BACKENDS from the env and
    // dials the backend fleet ONCE at boot (a dial failure surfaces here, not on first
    // request). The WS mirror binds an ephemeral port alongside the gRPC server.
    let edge = Edge::start(
        grpc_addr,
        std::sync::Arc::clone(&link),
        SpreadModel::default(),
        Clock::system(),
        // Production default config locations (env-var-or-CWD).
        None,
    )
    .await?;
    edge.gate().mark_ready();

    let backends = std::env::var("CELNET_FLEET_BACKENDS")
        .ok()
        .map(|s| s.split(',').filter(|p| !p.trim().is_empty()).count())
        .unwrap_or(0);

    println!("EDGE grpc={} backends={backends} READY", edge.grpc_addr());

    tokio::signal::ctrl_c().await?;
    edge.shutdown(Duration::from_secs(2)).await;
    link.stop();
    Ok(())
}
