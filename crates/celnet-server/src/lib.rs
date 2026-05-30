//! Celnet service edge — the tokio async front (gRPC via `tonic` + WebSocket RFS
//! streaming via `tokio-tungstenite`) that exposes the core-pinned
//! [`celnet_engine`] pricing core to the Celer estate and front end
//! (work-stream WS-I, `docs/ARCHITECTURE.md` §3, §5).
//!
//! # Two-tier model: async edge ⇄ pinned hot core
//!
//! The dependency arrow points one way: this edge depends on the engine, never
//! the reverse, and the async runtime never touches the pricing hot path. The
//! edge and the core are joined **only** by the engine's wait-free SPSC rings
//! ([`celnet_engine::RequestRing`] / [`celnet_engine::ResponseRing`]); the edge
//! never blocks the core and the core never blocks the edge.
//!
//! Because the rings are single-producer / single-consumer but the gRPC and
//! WebSocket surfaces are massively concurrent, the edge funnels every concurrent
//! request through one ring-owning *submitter* and routes each `Copy`
//! [`celnet_engine::PriceResponse`] back to its waiter through a `request_id`
//! correlation map. This keeps the ring contract intact (exactly one producer,
//! one consumer) while serving any number of async callers — the
//! [`core_link::CoreLink`] abstraction.
//!
//! # Modules
//!
//! * [`core_link`] — the async⇄core bridge: a dedicated busy-poll pricing thread
//!   driving the engine, fed/drained over the SPSC rings, with `request_id`
//!   correlation so concurrent async callers each get their own response.
//! * [`readiness`] — the `/readyz`-style blue-green lifecycle gate and the
//!   in-flight counter that a graceful connection drain watches to zero (§5).
//! * [`grpc`] — the `tonic` gRPC service implementation over the generated
//!   [`proto`] stubs (vanilla price, surface vol, single-barrier exotic, and the
//!   readiness probe).
//! * [`ws`] — the WebSocket RFS streaming endpoint: a tick source drives the core
//!   and serialized price/Greek updates are pushed to every subscriber.
//! * [`tick`] — a deterministic, seeded tick source that republishes market state
//!   to drive the RFS stream without an external feed.
//! * [`proto`] — the generated gRPC message + service stubs (see `build.rs`).
//!
//! # Determinism
//!
//! Pricing routes entirely through the engine (and thus `celnet_core::math`);
//! the edge adds no floating-point logic of its own. The tick source is
//! counter-based and seeded, never wall-clock-driven, so a replay is bit-exact.

#![forbid(unsafe_code)]

pub mod core_link;
pub mod grpc;
pub mod readiness;
pub mod tick;
pub mod ws;

/// The generated gRPC message and service stubs for the service edge.
///
/// `build.rs` compiles `proto/edge.proto` with the pure-Rust `protox` compiler
/// and `tonic-build`, emitting this module (proto `package celnet.edge`). It is
/// surfaced at a stable path so callers import `celnet_server::proto::...`.
pub mod proto {
    #![allow(missing_docs)] // generated code; documented at the .proto source.
    tonic::include_proto!("celnet.edge");
}

pub use core_link::{BarrierTopology, CoreLink, CoreLinkError, ExoticQuery, SurfaceQuery};
pub use readiness::{ReadinessGate, ServiceState};
pub use tick::TickSource;

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// A fully-wired, running service edge: the gRPC server, the WebSocket RFS
/// streaming server, the pricing-core bridge, and the readiness gate.
///
/// Construct with [`Edge::start`], which binds both listeners on
/// caller-supplied (typically ephemeral) addresses and spawns the serving
/// tasks. The handle exposes the actually-bound addresses (so tests can dial an
/// OS-assigned port), the readiness gate (to drive blue-green transitions), and
/// a [`Edge::shutdown`] that performs a graceful drain.
#[derive(Debug)]
pub struct Edge {
    grpc_addr: SocketAddr,
    ws_addr: SocketAddr,
    gate: Arc<ReadinessGate>,
    link: Arc<CoreLink>,
    grpc_shutdown: oneshot::Sender<()>,
    ws_shutdown: oneshot::Sender<()>,
    grpc_task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    ws_task: tokio::task::JoinHandle<()>,
}

impl Edge {
    /// Bind the gRPC and WebSocket listeners and start serving.
    ///
    /// `grpc_addr` / `ws_addr` are the requested bind addresses; pass a port of
    /// `0` to let the OS assign an ephemeral port (the bound address is then read
    /// back via [`Edge::grpc_addr`] / [`Edge::ws_addr`]). The supplied
    /// [`CoreLink`] owns the running pricing core; the supplied [`TickSource`]
    /// drives the RFS stream.
    ///
    /// The edge starts in [`ServiceState::Starting`]; call
    /// [`ReadinessGate::mark_ready`] once the core is warm to begin accepting
    /// traffic at the `/readyz` gate.
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if either listener cannot bind or be wrapped
    /// for serving.
    pub async fn start(
        grpc_addr: SocketAddr,
        ws_addr: SocketAddr,
        link: Arc<CoreLink>,
        tick: TickSource,
    ) -> std::io::Result<Self> {
        let gate = Arc::new(ReadinessGate::new());

        // --- gRPC listener (tonic over an incoming TCP stream) ---------------
        let grpc_listener = TcpListener::bind(grpc_addr).await?;
        let grpc_bound = grpc_listener.local_addr()?;
        let (grpc_shutdown, grpc_rx) = oneshot::channel::<()>();
        let svc = grpc::PricingEdgeService::new(Arc::clone(&link), Arc::clone(&gate));
        let grpc_server = proto::pricing_edge_server::PricingEdgeServer::new(svc);
        let incoming =
            tonic::transport::server::TcpIncoming::from_listener(grpc_listener, true, None)
                .map_err(std::io::Error::other)?;
        let grpc_task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(grpc_server)
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = grpc_rx.await;
                })
                .await
        });

        // --- WebSocket RFS listener -----------------------------------------
        let ws_listener = TcpListener::bind(ws_addr).await?;
        let ws_bound = ws_listener.local_addr()?;
        let (ws_shutdown, ws_rx) = oneshot::channel::<()>();
        let ws_server = ws::RfsServer::new(Arc::clone(&link), Arc::clone(&gate), tick);
        let ws_task = tokio::spawn(async move {
            ws_server.serve(ws_listener, ws_rx).await;
        });

        Ok(Self {
            grpc_addr: grpc_bound,
            ws_addr: ws_bound,
            gate,
            link,
            grpc_shutdown,
            ws_shutdown,
            grpc_task,
            ws_task,
        })
    }

    /// The actually-bound gRPC socket address (resolves an ephemeral `:0` port).
    #[must_use]
    pub fn grpc_addr(&self) -> SocketAddr {
        self.grpc_addr
    }

    /// The actually-bound WebSocket socket address.
    #[must_use]
    pub fn ws_addr(&self) -> SocketAddr {
        self.ws_addr
    }

    /// The shared readiness gate driving the `/readyz` probe and drain state.
    #[must_use]
    pub fn gate(&self) -> &Arc<ReadinessGate> {
        &self.gate
    }

    /// The shared pricing-core bridge.
    #[must_use]
    pub fn link(&self) -> &Arc<CoreLink> {
        &self.link
    }

    /// Gracefully drain and stop the edge for a blue-green cutover (§5).
    ///
    /// Transitions the readiness gate to [`ServiceState::Draining`] (so the
    /// `/readyz` probe immediately reports not-ready and the orchestrator steers
    /// new connections away), waits for in-flight requests to fall to zero (up to
    /// `drain_timeout`), then signals both serving tasks to stop accepting and
    /// awaits their completion. No in-flight request is dropped within the
    /// timeout.
    pub async fn shutdown(self, drain_timeout: std::time::Duration) {
        // Flip to draining: new readiness checks fail, in-flight work continues.
        self.gate.begin_drain();
        // Wait for in-flight work to quiesce (bounded).
        self.gate.await_drained(drain_timeout).await;
        // Stop accepting; the servers finish their in-flight connections.
        let _ = self.grpc_shutdown.send(());
        let _ = self.ws_shutdown.send(());
        let _ = self.grpc_task.await;
        let _ = self.ws_task.await;
    }
}
