//! Shared in-process test harness for the Celnet gRPC edge.
//!
//! Every test starts an [`Edge`] on an ephemeral port over the engine's calibrated
//! EURUSD fixture, dials it with the generated `celnet-proto` clients, and asserts
//! against first-principles `celnet-vanilla` / `celnet-exotics` references. Every
//! body is wrapped in a hard wall-clock deadline and every network await is itself
//! bounded, so a regression surfaces as a fast failure, never an infinite hang.

// Each integration-test binary `mod common;`s this file and uses only a subset of
// the helpers, so per-binary some are unused (`dead_code`) and their `pub` is not
// reachable outside the binary (`unreachable_pub`). Both are expected for a shared
// test-support module and are allowed here.
#![allow(dead_code, unreachable_pub)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celnet_engine::testing::make_state;
use celnet_entitlements::AccessMode;
use celnet_risk_fleet::FleetTopology;
use celnet_server::{Clock, CoreLink, Edge, LpPanelConfig, SpreadModel};
use celnet_types::{CcyPair, Tenor};
use tempfile::TempDir;

/// The hard wall-clock ceiling for any single edge integration test. A correctness
/// failure must surface as a *fast* failure, never an infinite hang.
///
/// Raised from 45 s → 90 s to accommodate multi-node fleet tests (3-node + 4-node fleets) on loaded local development machines.
pub const TEST_DEADLINE: Duration = Duration::from_secs(90);

/// Bound a single network / response await so a never-arriving reply fails fast.
///
/// Raised from 5 s → 20 s to match the celnet-client STEP_DEADLINE precedent
/// under loaded-t2 contention on the single M4.
pub const STEP_DEADLINE: Duration = Duration::from_secs(20);

/// The resolved EURUSD 1Y convention record the fixture is built on.
#[must_use]
pub fn eurusd_conv() -> celnet_conventions::ConventionRecord {
    celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
}

/// Start a ready edge on an ephemeral port over the EURUSD fixture with a system
/// clock, returning the edge, its bound gRPC address, and the [`TempDir`] rooting
/// its isolated persisted config (`identity.json` / `fix-connections.json`).
///
/// The `TempDir` MUST be held for the test's full lifetime — dropping it deletes the
/// directory mid-test. Bind it (e.g. `let (edge, addr, _tmp) = …`), never `_`.
pub async fn start_ready_edge() -> (Edge, SocketAddr, TempDir) {
    start_edge_with(true, Clock::system()).await
}

/// Start an edge with an explicit ready flag and clock (so a test can drive
/// last-look expiry with a manual clock or assert not-ready rejection).
///
/// Each edge gets its OWN [`TempDir`]-rooted persisted config so parallel test edges
/// never race the one shared `identity.json` / `fix-connections.json` path. The
/// `TempDir` is returned so the caller owns it for the edge's full lifetime.
pub async fn start_edge_with(ready: bool, clock: Clock) -> (Edge, SocketAddr, TempDir) {
    let initial = make_state(1.10, eurusd_conv());
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
    if ready {
        edge.gate().mark_ready();
    }
    // The generic mechanics harness runs under the Permissive posture: these tests
    // exercise stream/surface/RFQ behaviour, not the caller-auth gate (Enforce is
    // covered by the dedicated stream-auth unit test + the live e2e). Auth/entitlement
    // tests build their own edge and assert the Enforce default explicitly.
    edge.store().set_access_mode(AccessMode::Permissive);
    let addr = edge.grpc_addr();
    (edge, addr, data_dir)
}

/// Start a ready edge whose multi-dealer RFQ panel carries `synthetic_lps`
/// deterministic synthetic demo dealers beside the native maker, with a system
/// clock. Explicit config — race-free, no process-global env mutation.
///
/// Returns the edge, its bound gRPC address, and the [`TempDir`] rooting its
/// isolated persisted config — hold the `TempDir` for the test's full lifetime.
pub async fn start_ready_panel_edge(synthetic_lps: u32) -> (Edge, SocketAddr, TempDir) {
    start_panel_edge_with(true, Clock::system(), synthetic_lps).await
}

/// Start an edge with an explicit ready flag, clock, and synthetic LP-panel
/// breadth (so a test can drive a panel row past its last-look deadline with a
/// manual clock). Uses the explicit-topology/panel boot path — no env mutation.
///
/// Each edge gets its OWN [`TempDir`]-rooted persisted config so parallel test edges
/// never race the one shared config path; the `TempDir` is returned for the caller
/// to own.
pub async fn start_panel_edge_with(
    ready: bool,
    clock: Clock,
    synthetic_lps: u32,
) -> (Edge, SocketAddr, TempDir) {
    let initial = make_state(1.10, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let data_dir = tempfile::tempdir().expect("temp data dir for the edge's persisted config");
    let edge = Edge::start_on_with_topology(
        grpc,
        ws,
        Arc::clone(&link),
        SpreadModel::default(),
        clock,
        FleetTopology::InProcess,
        LpPanelConfig { synthetic_lps },
        Some(data_dir.path()),
    )
    .await
    .expect("edge binds on an ephemeral port");
    if ready {
        edge.gate().mark_ready();
    }
    // The generic mechanics harness runs under the Permissive posture: these tests
    // exercise stream/surface/RFQ behaviour, not the caller-auth gate (Enforce is
    // covered by the dedicated stream-auth unit test + the live e2e). Auth/entitlement
    // tests build their own edge and assert the Enforce default explicitly.
    edge.store().set_access_mode(AccessMode::Permissive);
    let addr = edge.grpc_addr();
    (edge, addr, data_dir)
}

/// Log in as the default administrator over the edge's real `AuthService.Login` RPC
/// (at the gRPC `base_url`, e.g. `http://127.0.0.1:NNNN`) and return the issued
/// session token — the exact bearer a deployment threads onto a gated request.
///
/// The seed admin ([`SEED_ADMIN_EMAIL`] / [`SEED_ADMIN_PASSWORD`]) is always present:
/// every [`Edge::start`] ensures it. The returned token validates against THIS edge's
/// OWN session registry, so a request to this edge carrying it is admitted as that
/// (grant-all) user under [`AccessMode::Enforce`].
pub async fn login_seed_admin(base_url: &str) -> String {
    use celnet_proto::LoginRequest;
    use celnet_proto::auth_service_client::AuthServiceClient;
    use celnet_server::config::identity::{SEED_ADMIN_EMAIL, SEED_ADMIN_PASSWORD};

    let mut auth = tokio::time::timeout(
        STEP_DEADLINE,
        AuthServiceClient::connect(base_url.to_owned()),
    )
    .await
    .expect("auth client connects in time")
    .expect("auth client connects");
    let resp = tokio::time::timeout(
        STEP_DEADLINE,
        auth.login(LoginRequest {
            email: SEED_ADMIN_EMAIL.to_owned(),
            password: SEED_ADMIN_PASSWORD.to_owned(),
            correlation_id: Some(1),
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

/// The wire conventions used across the tests (EURUSD spot-premium-adjusted /
/// delta-neutral-straddle / foreign-percent).
#[must_use]
pub fn wire_conventions() -> celnet_proto::Conventions {
    celnet_proto::Conventions {
        delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
        atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
        premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
        cut: celnet_proto::Cut::NewYork1000 as i32,
        day_count: celnet_proto::DayCount::Act365Fixed as i32,
        settlement: celnet_proto::Settlement::Deliverable as i32,
    }
}

/// A EUR/USD currency pair on the wire.
#[must_use]
pub fn eurusd_pair() -> celnet_proto::CcyPair {
    celnet_proto::CcyPair {
        base: "EUR".to_owned(),
        quote: "USD".to_owned(),
    }
}

/// A vanilla-call instrument at an absolute strike with a 1Y expiry.
#[must_use]
pub fn vanilla_call(strike: f64) -> celnet_proto::Instrument {
    celnet_proto::Instrument {
        underlying: Some(celnet_proto::Underlying::fx(eurusd_pair())),
        tenor: Some(celnet_proto::Tenor {
            unit: celnet_proto::tenor::Unit::Years as i32,
            count: 1,
            broken_date: None,
        }),
        expiry_years: 1.0,
        quantity: Some(celnet_proto::Quantity {
            notional: 1_000_000.0,
            base_ccy: true,
        }),
        side: celnet_proto::Side::TwoWay as i32,
        solve: None,
        pricing_model: celnet_proto::PricingModel::Default as i32,
        product: Some(celnet_proto::instrument::Product::Vanilla(
            celnet_proto::Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(celnet_proto::StrikeOrDelta {
                    spec: Some(celnet_proto::strike_or_delta::Spec::Strike(strike)),
                }),
            },
        )),
        ..Default::default()
    }
}

/// The live market context the edge prices an RFQ / RFS against: the EURUSD
/// fixture's spot, ATM vol (at the forward), and rates. Tests mirror this to form
/// a first-principles reference.
#[must_use]
pub fn live_market() -> celnet_proto::MarketContext {
    let state = make_state(1.10, eurusd_conv());
    let forward = state.forward();
    let atm_vol = celnet_core::Smile::implied_vol(&state.smile, forward, forward, state.t).0;
    celnet_proto::MarketContext::fx(state.spot, atm_vol, state.r_dom, state.r_for)
}
