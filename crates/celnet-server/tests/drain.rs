//! Readiness / graceful-drain state-transition integration test (§5).
//!
//! Asserts the blue-green lifecycle the cutover relies on, end to end through the
//! gRPC readiness probe:
//!
//! ```text
//!   STARTING ──mark_ready──▶ READY ──begin_drain──▶ DRAINING
//! ```
//!
//! and that the in-flight counter the drain barrier watches reaches zero, so a
//! `shutdown` completes without dropping in-flight work.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celnet_engine::testing::make_state;
use celnet_server::proto;
use celnet_server::proto::pricing_edge_client::PricingEdgeClient;
use celnet_server::{CoreLink, Edge, ServiceState, TickSource};
use celnet_types::{CcyPair, Tenor};

fn eurusd_conv() -> celnet_conventions::ConventionRecord {
    celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
}

async fn start_edge() -> (Edge, Arc<CoreLink>) {
    let conv = eurusd_conv();
    let initial = make_state(1.10, conv);
    let link = CoreLink::start(initial.clone(), None);
    let tick = TickSource::new(initial, 1, 0.0);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start(grpc, ws, Arc::clone(&link), tick)
        .await
        .expect("edge binds");
    (edge, link)
}

/// The hard wall-clock ceiling for any single drain/readiness integration test:
/// a regression must fail fast, never hang the suite.
const TEST_DEADLINE: Duration = Duration::from_secs(10);

/// Bound a single network/probe `.await` so a never-arriving reply fails fast.
const STEP_DEADLINE: Duration = Duration::from_secs(5);

#[tokio::test]
async fn readiness_gate_transitions_starting_ready_draining() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _link) = start_edge().await;
        let gate = edge.gate();

        // STARTING: not ready.
        assert_eq!(gate.state(), ServiceState::Starting);
        assert!(!gate.is_ready());

        // → READY.
        assert!(gate.mark_ready());
        assert_eq!(gate.state(), ServiceState::Ready);
        assert!(gate.is_ready());

        // → DRAINING (terminal; never returns to ready).
        gate.begin_drain();
        assert_eq!(gate.state(), ServiceState::Draining);
        assert!(!gate.is_ready());
        assert!(!gate.mark_ready());
        assert_eq!(gate.state(), ServiceState::Draining);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

#[tokio::test]
async fn readiness_probe_reflects_transitions_over_grpc() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _link) = start_edge().await;
        edge.gate().mark_ready();

        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            PricingEdgeClient::connect(format!("http://{}", edge.grpc_addr())),
        )
        .await
        .expect("client connects within the step deadline")
        .expect("client connects");

        // Ready over the wire.
        let probe =
            tokio::time::timeout(STEP_DEADLINE, client.readiness(proto::ReadinessRequest {}))
                .await
                .expect("probe returns within the step deadline")
                .expect("probe answers")
                .into_inner();
        assert_eq!(probe.state, proto::ServiceState::Ready as i32);
        assert!(probe.ready);
        assert_eq!(probe.in_flight, 0);

        // Begin draining; the probe must immediately report not-ready / draining even
        // though the server keeps serving the probe RPC itself.
        edge.gate().begin_drain();
        let probe =
            tokio::time::timeout(STEP_DEADLINE, client.readiness(proto::ReadinessRequest {}))
                .await
                .expect("probe returns within the step deadline")
                .expect("probe still answers while draining")
                .into_inner();
        assert_eq!(probe.state, proto::ServiceState::Draining as i32);
        assert!(!probe.ready);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

#[tokio::test]
async fn shutdown_drains_in_flight_to_zero() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _link) = start_edge().await;
        let gate = Arc::clone(edge.gate());
        gate.mark_ready();

        // Simulate an in-flight request spanning the drain window.
        let in_flight_guard = gate.enter();
        assert_eq!(gate.in_flight(), 1);

        // Begin the drain and confirm the gate will not declare drained while work is
        // outstanding.
        gate.begin_drain();
        assert!(
            !gate.await_drained(Duration::from_millis(30)).await,
            "must not report drained while a request is in flight"
        );

        // Release the in-flight slot; the drain barrier must then clear promptly.
        drop(in_flight_guard);
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
