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
use tempfile::TempDir;

/// The hard wall-clock ceiling for any single client integration test. A
/// correctness failure must surface as a *fast* failure, never an infinite hang.
///
/// Sized for the real edge-boot cost (argon2 seed-admin hashing + socket bind +
/// leader wait) under a *fully-loaded* `just t2` run, where `cargo test --test '*'`
/// boots several edges concurrently on one machine — a 10s ceiling flaked
/// `desk`/`multi_dealer` around boot time (a slow boot, not a hang). Matches the
/// `lsv_workflow` local precedent; a genuine hang still fails well within it.
pub const TEST_DEADLINE: Duration = Duration::from_secs(45);

/// Bound a single network / response await so a never-arriving reply fails fast.
pub const STEP_DEADLINE: Duration = Duration::from_secs(20);

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
/// clock, returning the edge, a connected typed [`Client`], and the [`TempDir`]
/// rooting the edge's isolated persisted config — hold it for the test's lifetime.
pub async fn start_edge_and_client() -> (Edge, Client, TempDir) {
    start_edge_and_client_with(Clock::system()).await
}

/// Start a ready edge with an explicit clock (so a test can drive last-look expiry
/// deterministically), returning the edge, a connected typed [`Client`], and the
/// [`TempDir`] rooting the edge's isolated persisted config.
pub async fn start_edge_and_client_with(clock: Clock) -> (Edge, Client, TempDir) {
    let (edge, addr, data_dir) = start_ready_edge(clock).await;
    let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
        .await
        .expect("client connects in time")
        .expect("client connects");
    (edge, client, data_dir)
}

/// Start a ready edge and return it plus a typed [`Client`] **authenticated as the
/// seed admin** (via the edge's real `AuthService.Login`), carrying a
/// capability-complete session token.
///
/// The capability gates — `AcceptQuote` (`Execute·FxOptions`), the stream
/// `Subscribe`/`Modify`/`Resync`/`MarketSeriesSubscribe`/`Execute` frames, and the
/// FI desk respond/accept — resolve a capability ONLY from an authenticated session;
/// a body-asserted grant-all principal cannot self-grant one (finding #3). So a test
/// that invokes any of those under the production `Enforce` posture must log in. The
/// token only AUTHORIZES — the SDK still sends the grant-all principal and the price
/// is unchanged. Pure principal-gated tests (`request_quote` / panel DISPLAY / reject
/// / risk with `ReadAny`) keep the token-less [`start_edge_and_client`] helpers.
pub async fn start_edge_and_authed_client() -> (Edge, Client, TempDir) {
    start_edge_and_authed_client_with(Clock::system()).await
}

/// [`start_edge_and_authed_client`] with an explicit clock (so a gated test can also
/// drive last-look expiry deterministically). Returns the [`TempDir`] rooting the
/// edge's isolated persisted config — hold it for the test's lifetime so parallel
/// edges never race the shared `identity.json` / `fix-connections.json` path.
pub async fn start_edge_and_authed_client_with(clock: Clock) -> (Edge, Client, TempDir) {
    let (edge, addr, data_dir) = start_ready_edge(clock).await;
    let token = login_seed_admin(addr).await;
    let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
        .await
        .expect("client connects in time")
        .expect("client connects")
        .with_session_token(token);
    (edge, client, data_dir)
}

/// Start a ready edge on an ephemeral port over the EURUSD fixture.
///
/// Each edge roots its persisted config (`identity.json` / `fix-connections.json`)
/// in its OWN [`TempDir`] so parallel test edges never race the one shared path; the
/// `TempDir` is returned for the caller to own for the edge's full lifetime.
pub async fn start_ready_edge(clock: Clock) -> (Edge, SocketAddr, TempDir) {
    let initial = make_state(FIXTURE_SPOT, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let data_dir = tempfile::tempdir().expect("temp data dir for the edge's persisted config");
    let edge = Edge::start(
        grpc,
        Arc::clone(&link),
        SpreadModel::default(),
        clock,
        Some(data_dir.path()),
    )
    .await
    .expect("edge binds on an ephemeral port");
    mark_ready_enforcing(&edge);
    let addr = edge.grpc_addr();
    (edge, addr, data_dir)
}

/// Mark a freshly-bound in-process test edge ready under the **production
/// deny-by-default posture** (`AccessMode::Enforce`, set explicitly so a future
/// edit can't silently re-mask the boundary with Permissive).
///
/// These tests deliberately run against `Enforce` — the real production edge — so
/// they verify the trader-facing default end-to-end. Two authorization tiers apply:
///
/// * **Principal-gated (`ReadAny`)** — `RequestQuote` / `RejectQuote` / the panel
///   DISPLAY / risk aggregation: the typed SDK sends an **explicit grant-all**
///   principal when a query asserts none, so the headline read workflow is served by
///   the token-less [`start_edge_and_client`] helpers (a *genuinely* absent principal
///   is denied — covered server-side by
///   `absent_principal_denied_on_every_entitlement_gated_service`); a scoped principal
///   is honored, so the entitlement-pruning workflow exercises real pruning.
/// * **Capability-gated** — `AcceptQuote` (`Execute·FxOptions`), the stream frames
///   (`Stream`/`Execute·FxOptions`), and the FI desk respond/accept: a capability
///   resolves ONLY from an authenticated session — a grant-all body principal cannot
///   self-grant one (finding #3) — so a test invoking these must log in
///   ([`start_edge_and_authed_client`] / [`start_panel_edge_and_authed_client`]). The
///   token only authorizes; the SDK still sends the grant-all principal and the price
///   is unchanged.
///
/// (The dev demo edge runs Permissive for raw/un-principalled clients; the SDK never
/// needs it.)
fn mark_ready_enforcing(edge: &Edge) {
    edge.store().set_access_mode(AccessMode::Enforce);
    edge.gate().mark_ready();
}

/// Start a ready edge with an explicit clock AND a deterministic synthetic
/// LP-panel breadth (native maker + `synthetic_lps` labeled demo/test dealers),
/// returning the edge and a connected typed [`Client`]. Uses the explicit-panel
/// boot path ([`Edge::start_on_with_panel`]) so the multi-dealer tests never
/// mutate process-global env, matching the server's own panel-test harness.
pub async fn start_panel_edge_and_client(
    clock: Clock,
    synthetic_lps: u32,
) -> (Edge, Client, TempDir) {
    let initial = make_state(FIXTURE_SPOT, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let data_dir = tempfile::tempdir().expect("temp data dir for the edge's persisted config");
    let edge = Edge::start_on_with_panel(
        grpc,
        ws,
        Arc::clone(&link),
        SpreadModel::default(),
        clock,
        LpPanelConfig { synthetic_lps },
        Some(data_dir.path()),
    )
    .await
    .expect("edge binds on an ephemeral port");
    mark_ready_enforcing(&edge);
    let addr = edge.grpc_addr();
    let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
        .await
        .expect("client connects in time")
        .expect("client connects");
    (edge, client, data_dir)
}

/// [`start_panel_edge_and_client`] but with the returned [`Client`] **authenticated as
/// the seed admin**, for multi-dealer tests that book a panel row via the
/// capability-gated `AcceptQuote` (`Execute·FxOptions`). The panel DISPLAY
/// (`request_multi_dealer_quote`) is only principal-gated, so it is unaffected; the
/// token only authorizes the accept and does not change the booked price.
pub async fn start_panel_edge_and_authed_client(
    clock: Clock,
    synthetic_lps: u32,
) -> (Edge, Client, TempDir) {
    let initial = make_state(FIXTURE_SPOT, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let data_dir = tempfile::tempdir().expect("temp data dir for the edge's persisted config");
    let edge = Edge::start_on_with_panel(
        grpc,
        ws,
        Arc::clone(&link),
        SpreadModel::default(),
        clock,
        LpPanelConfig { synthetic_lps },
        Some(data_dir.path()),
    )
    .await
    .expect("edge binds on an ephemeral port");
    mark_ready_enforcing(&edge);
    let addr = edge.grpc_addr();
    let token = login_seed_admin(addr).await;
    let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
        .await
        .expect("client connects in time")
        .expect("client connects")
        .with_session_token(token);
    (edge, client, data_dir)
}

/// Log in as the default administrator over the edge's real `AuthService.Login` RPC
/// and return the issued session token — the exact bearer a deployment threads onto a
/// gated request. The seed admin (`admin@celnet.com` / `password`) is always present:
/// every [`Edge::start`] ensures it (seeded on first run into the gitignored
/// `identity.json`, persisted thereafter; nothing in the suite rotates its password).
/// The returned token validates against the edge's OWN session registry — the one
/// `StreamAuth` is checked against — so a stream authenticated with it is admitted as
/// that user under [`AccessMode::Enforce`].
pub async fn login_seed_admin(addr: SocketAddr) -> String {
    use celnet_proto::LoginRequest;
    use celnet_proto::auth_service_client::AuthServiceClient;

    let mut auth = tokio::time::timeout(
        STEP_DEADLINE,
        AuthServiceClient::connect(format!("http://{addr}")),
    )
    .await
    .expect("auth client connects in time")
    .expect("auth client connects");
    let resp = tokio::time::timeout(
        STEP_DEADLINE,
        auth.login(LoginRequest {
            email: "admin@celnet.com".to_owned(),
            password: "password".to_owned(),
            correlation_id: None,
        }),
    )
    .await
    .expect("login resolves in time")
    .expect("seed admin logs in")
    .into_inner();
    assert!(
        !resp.session_token.is_empty(),
        "Login mints a non-empty session token"
    );
    resp.session_token
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
