//! Readiness / graceful-drain integration tests over the new gRPC contract.
//!
//! Asserts the blue-green lifecycle the cutover relies on: a not-ready edge
//! refuses pricing with `UNAVAILABLE`, the gate transitions
//! `STARTING → READY → DRAINING`, and `shutdown` drains in-flight work to zero.

mod common;

use std::time::Duration;

use celnet_proto::pricing_service_client::PricingServiceClient;
use celnet_proto::{PriceRequest, PriceResponse};
use celnet_server::{Clock, ServiceState};

use common::{
    STEP_DEADLINE, TEST_DEADLINE, live_market, start_edge_with, start_ready_edge, vanilla_call,
    wire_conventions,
};

/// A `Price` against a not-ready edge is rejected with `UNAVAILABLE` (the
/// `/readyz` contract): new traffic only flows once warm. Once marked ready the
/// same request succeeds and reprices the instrument.
#[tokio::test]
async fn pricing_rejected_until_ready_then_succeeds() {
    tokio::time::timeout(TEST_DEADLINE, async {
        // Deliberately NOT ready.
        let (edge, addr) = start_edge_with(false, Clock::system()).await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            PricingServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let m = live_market();
        let req = || PriceRequest {
            request_id: 1,
            instrument: Some(vanilla_call(1.10)),
            market: Some(m),
            conventions: Some(wire_conventions()),
            correlation_id: None,
            surface_version: None,
        };

        let status = tokio::time::timeout(STEP_DEADLINE, client.price(req()))
            .await
            .expect("price returns in time")
            .expect_err("a not-ready edge must reject pricing");
        assert_eq!(status.code(), tonic::Code::Unavailable);

        // Mark ready, then the same request succeeds.
        edge.gate().mark_ready();
        let resp: PriceResponse = tokio::time::timeout(STEP_DEADLINE, client.price(req()))
            .await
            .expect("price returns in time")
            .expect("price succeeds once ready")
            .into_inner();
        assert_eq!(resp.request_id, 1);
        let greeks = resp.greeks.expect("greeks present");
        assert!(greeks.price > 0.0, "an ATM-ish call has positive premium");

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// The readiness gate transitions `STARTING → READY → DRAINING` and a drain is
/// terminal (never returns to ready).
#[tokio::test]
async fn readiness_gate_transitions() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _addr) = start_edge_with(false, Clock::system()).await;
        let gate = edge.gate();

        assert_eq!(gate.state(), ServiceState::Starting);
        assert!(!gate.is_ready());

        assert!(gate.mark_ready());
        assert_eq!(gate.state(), ServiceState::Ready);
        assert!(gate.is_ready());

        gate.begin_drain();
        assert_eq!(gate.state(), ServiceState::Draining);
        assert!(!gate.is_ready());
        assert!(
            !gate.mark_ready(),
            "a drained instance never returns to ready"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// `shutdown` holds the cutover open until in-flight work drains to zero.
#[tokio::test]
async fn shutdown_drains_in_flight_to_zero() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _addr) = start_ready_edge().await;
        let gate = std::sync::Arc::clone(edge.gate());

        // Simulate an in-flight request spanning the drain window.
        let guard = gate.enter();
        assert_eq!(gate.in_flight(), 1);
        gate.begin_drain();
        assert!(
            !gate.await_drained(Duration::from_millis(30)).await,
            "must not report drained while a request is in flight"
        );
        drop(guard);
        assert!(
            gate.await_drained(Duration::from_secs(2)).await,
            "drain completes once in-flight reaches zero"
        );
        assert_eq!(gate.in_flight(), 0);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
