//! A parameterizable **backend node** for the out-of-process scale harness — a real
//! standalone OS process.
//!
//! Each backend is an ordinary single-node (`InProcess`) [`Edge`]: the four
//! `celnet-proto` gRPC services + the WS mirror over the EURUSD fixture market, with
//! its own pinned pricing core and surface book. It seeds its [`PositionStore`] with
//! the **disjoint HRW slice** of the shared master book it owns under the fleet's
//! replica list, so the union of every backend's slice is exactly the whole book and
//! a federating front edge reconciles to a single-node oracle.
//!
//! It is spawned by the `scale_harness` driver via `std::process::Command`; only the
//! front edge (`scale_edge`) runs in `Distributed` mode and forwards/federates across
//! these backends.
//!
//! Environment (the driver sets all three per child):
//!   * `CELNET_GRPC_ADDR` — this backend's gRPC bind address (a fixed localhost port).
//!   * `CELNET_REPLICA_ID` — this backend's replica id (`1..=n`, its slice key).
//!   * `CELNET_FLEET_SIZE` — the total replica count `n` the slice is computed against.
//!
//! On readiness it prints, to stdout, a single machine-parsable line the driver
//! waits for:
//!   `BACKEND replica=<id>/<n> grpc=<addr> seeded=<count> READY`
//! then runs until `SIGINT`/`SIGTERM` (the driver kills it on scale-down / teardown).

// The shared master-book fixture, defined once and `#[path]`-included by both this
// backend and the driver harness so there is exactly one book + one partition.
#[path = "scale_book.rs"]
mod scale_book;

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

fn env_u64(key: &str) -> u64 {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("{key} must be set by the scale harness"))
        .parse()
        .unwrap_or_else(|_| panic!("{key} is not a valid integer"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let grpc_addr = env_addr("CELNET_GRPC_ADDR");
    let replica = env_u64("CELNET_REPLICA_ID");
    let fleet_size = env_u64("CELNET_FLEET_SIZE");

    // The EURUSD fixture market — every backend shares it, so a forwarded price and a
    // direct-to-owner price for any pair see the same backend market (deterministic).
    let conv =
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    let initial = make_state(1.10, conv);
    let link = CoreLink::start(initial, None);

    // A single-node edge (its own fleet topology is in-process — it serves its local
    // slice directly; only the front edge is distributed).
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

    // Seed exactly this replica's disjoint HRW slice of the shared master book.
    let seeded = scale_book::seed_slice(edge.store(), replica, fleet_size);

    // The driver waits for this exact line on stdout to learn the bound address.
    println!(
        "BACKEND replica={replica}/{fleet_size} grpc={} seeded={seeded} READY",
        edge.grpc_addr()
    );

    // Run until the driver signals shutdown (SIGINT on teardown, SIGTERM/kill on
    // scale-down), then drain gracefully.
    tokio::signal::ctrl_c().await?;
    edge.shutdown(Duration::from_secs(2)).await;
    link.stop();
    Ok(())
}
