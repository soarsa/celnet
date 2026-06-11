//! Shared in-process harness for the `celnet-client` trader-workflow tests.
//!
//! Each test starts a real [`celnet_server::Edge`] on an ephemeral port over the
//! engine's calibrated EURUSD fixture, dials it with the typed [`celnet_client`]
//! SDK, and asserts the client result equals a first-principles `celnet-vanilla` /
//! `celnet-surface` / `celnet-exotics` reference — the same fixture and the same
//! references the server's own integration tests use, so the SDK is validated
//! end-to-end against the underlying analytics, not merely "looks plausible".
//!
//! Every test body is wrapped in a hard wall-clock deadline and every network /
//! stream await is itself bounded, so a regression surfaces as a fast failure,
//! never an infinite hang (test-hygiene mandate).

// Each integration-test binary `mod common;`s this file and uses only a subset of
// the helpers, so per-binary some are unused. Expected for shared test support.
#![allow(dead_code, unreachable_pub)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celnet_client::{
    Client, Conventions, InstrumentSpec, MarketContext, Quantity, Side, StrikeSpec,
};
use celnet_engine::testing::make_state;
use celnet_server::{AccessMode, Clock, CoreLink, Edge, LpPanelConfig, SpreadModel};
use celnet_types::{CcyPair, OptionType, Tenor};

/// The hard wall-clock ceiling for any single client integration test. A
/// correctness failure must surface as a *fast* failure, never an infinite hang.
pub const TEST_DEADLINE: Duration = Duration::from_secs(10);

/// Bound a single network / response await so a never-arriving reply fails fast.
pub const STEP_DEADLINE: Duration = Duration::from_secs(5);

/// The EURUSD spot the fixture is built on.
pub const FIXTURE_SPOT: f64 = 1.10;

/// The resolved EURUSD 1Y convention record the fixture is built on.
#[must_use]
pub fn eurusd_conv() -> celnet_conventions::ConventionRecord {
    celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
}

/// The EURUSD currency pair.
#[must_use]
pub fn eurusd() -> CcyPair {
    CcyPair::parse("EURUSD").unwrap()
}

/// Start a ready edge on an ephemeral port over the EURUSD fixture with a system
/// clock, returning the edge and a connected typed [`Client`].
pub async fn start_edge_and_client() -> (Edge, Client) {
    start_edge_and_client_with(Clock::system()).await
}

/// Start a ready edge with an explicit clock (so a test can drive last-look expiry
/// deterministically), returning the edge and a connected typed [`Client`].
pub async fn start_edge_and_client_with(clock: Clock) -> (Edge, Client) {
    let (edge, addr) = start_ready_edge(clock).await;
    let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
        .await
        .expect("client connects in time")
        .expect("client connects");
    (edge, client)
}

/// Start a ready edge on an ephemeral port over the EURUSD fixture.
pub async fn start_ready_edge(clock: Clock) -> (Edge, SocketAddr) {
    let initial = make_state(FIXTURE_SPOT, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start(grpc, Arc::clone(&link), SpreadModel::default(), clock)
        .await
        .expect("edge binds on an ephemeral port");
    mark_ready_enforcing(&edge);
    let addr = edge.grpc_addr();
    (edge, addr)
}

/// Mark a freshly-bound in-process test edge ready under the **production
/// deny-by-default posture** (`AccessMode::Enforce`, set explicitly so a future
/// edit can't silently re-mask the boundary with Permissive).
///
/// These tests deliberately run against `Enforce` — the real production edge — so
/// they verify the trader-facing default end-to-end: the typed SDK sends an
/// **explicit grant-all** principal when a query asserts none, so the headline
/// risk workflow is served, while a *genuinely* absent principal would be denied
/// (covered server-side by `absent_principal_denied_on_every_entitlement_gated_service`).
/// A scoped principal is honored, so the entitlement-pruning workflow exercises
/// real pruning. (The dev demo edge runs Permissive for raw/un-principalled
/// clients; the SDK never needs it.)
fn mark_ready_enforcing(edge: &Edge) {
    edge.store().set_access_mode(AccessMode::Enforce);
    edge.gate().mark_ready();
}

/// Start a ready edge with an explicit clock AND a deterministic synthetic
/// LP-panel breadth (native maker + `synthetic_lps` labeled demo/test dealers),
/// returning the edge and a connected typed [`Client`]. Uses the explicit-panel
/// boot path ([`Edge::start_on_with_panel`]) so the multi-dealer tests never
/// mutate process-global env, matching the server's own panel-test harness.
pub async fn start_panel_edge_and_client(clock: Clock, synthetic_lps: u32) -> (Edge, Client) {
    let initial = make_state(FIXTURE_SPOT, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start_on_with_panel(
        grpc,
        ws,
        Arc::clone(&link),
        SpreadModel::default(),
        clock,
        LpPanelConfig { synthetic_lps },
    )
    .await
    .expect("edge binds on an ephemeral port");
    mark_ready_enforcing(&edge);
    let addr = edge.grpc_addr();
    let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
        .await
        .expect("client connects in time")
        .expect("client connects");
    (edge, client)
}

/// The wire/typed conventions used across the tests — EURUSD spot-unadjusted /
/// ATM-forward / domestic-pips, matching the server harness's `wire_conventions`.
#[must_use]
pub fn conventions() -> Conventions {
    Conventions::major_default()
}

/// The live market context the edge prices an RFQ / RFS / scenario against: the
/// EURUSD fixture's spot, ATM vol (at the forward), and rates. Tests mirror this to
/// form a first-principles reference — identical to the server harness's
/// `live_market`.
#[must_use]
pub fn live_market() -> MarketContext {
    let state = make_state(FIXTURE_SPOT, eurusd_conv());
    let forward = state.forward();
    let atm_vol = celnet_core::Smile::implied_vol(&state.smile, forward, forward, state.t).0;
    MarketContext {
        spot: state.spot,
        vol: atm_vol,
        r_dom: state.r_dom,
        r_for: state.r_for,
    }
}

/// A vanilla EURUSD call at an absolute strike with a 1Y expiry and 1mm EUR
/// notional (the typed SDK form of the server harness's `vanilla_call`).
#[must_use]
pub fn vanilla_call(strike: f64) -> InstrumentSpec {
    InstrumentSpec::vanilla(
        eurusd(),
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        OptionType::Call,
        StrikeSpec::Absolute(strike),
    )
}
