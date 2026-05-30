//! Celnet service edge — the tokio async gRPC front that exposes the core-pinned
//! [`celnet_engine`] pricing core to the Celer estate and front end over the
//! single, current, unversioned [`celnet_proto`] wire contract
//! (`docs/ARCHITECTURE.md` §3, §5; `docs/API-CLIENTS.md`).
//!
//! # Two-tier model: async edge ⇄ pinned hot core
//!
//! The dependency arrow points one way: this edge depends on the engine, never
//! the reverse, and the async runtime never touches the pricing hot path. The
//! edge and the core are joined **only** by the engine's wait-free SPSC rings
//! ([`celnet_engine::RequestRing`] / [`celnet_engine::ResponseRing`]) via the
//! [`core_link::CoreLink`] bridge; the edge never blocks the core and the core
//! never blocks the edge.
//!
//! # Services
//!
//! The edge hosts the four generated `tonic` services of the contract on one gRPC
//! server (see [`services`]):
//!
//! * **`PricingService`** — one-shot instrument pricing ([`services::pricing`]);
//! * **`QuoteService`** — the RFQ lifecycle (request → quote → accept → execution)
//!   with client idempotency and a last-look validity window ([`services::quote`]);
//! * **`StreamService`** — the bidirectional RFS subscription (per-instrument
//!   snapshot + sequenced deltas + heartbeat + server-assisted resync)
//!   ([`services::stream`]);
//! * **`SurfaceService`** — `GetSmile` / `MarkSurface` / `Scenario`
//!   ([`services::surface`]).
//!
//! # Modules
//!
//! * [`core_link`] — the async⇄core bridge: a dedicated busy-poll pricing thread
//!   driving the engine, fed/drained over the SPSC rings, with `request_id`
//!   correlation so concurrent async callers each get their own response, plus a
//!   control plane for market-state reads, surface and exotic queries.
//! * [`readiness`] — the `/readyz`-style blue-green lifecycle gate and the
//!   in-flight counter that a graceful connection drain watches to zero (§5).
//! * [`pricer`] — the deterministic instrument→Greeks analytics router shared by
//!   every service (vanilla / strategy / barrier / digital / touch).
//! * [`spread`] — the maker two-way bid/offer model around a mid price.
//! * [`clock`] — the edge wall-clock for message timestamping (outside pricing).
//! * [`tick`] — a deterministic, seeded tick source for the RFS stream.
//!
//! # Determinism
//!
//! Pricing routes entirely through [`pricer`] (and thus `celnet_core::math`); the
//! edge adds no floating-point logic of its own beyond convention-aware strike
//! resolution and finite-difference exotic Greeks (both deterministic). Stream
//! ticks are counter-based and seeded, never wall-clock-driven, so a replay is
//! bit-exact. Only message *timestamps* read the wall-clock, at the edge, never on
//! the pricing path.

#![forbid(unsafe_code)]

pub mod clock;
pub mod core_link;
pub mod pricer;
pub mod readiness;
pub mod services;
pub mod spread;
pub mod tick;

pub use clock::Clock;
pub use core_link::{
    BarrierTopology, CoreLink, CoreLinkError, ExoticQuery, MarketSnapshot, SurfaceQuery, SurfaceVol,
};
pub use pricer::{ConventionSet, PriceError, Priced, price_instrument};
pub use readiness::{ReadinessGate, ServiceState};
pub use spread::SpreadModel;
pub use tick::TickSource;

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::oneshot;

use celnet_proto::pricing_service_server::PricingServiceServer;
use celnet_proto::quote_service_server::QuoteServiceServer;
use celnet_proto::stream_service_server::StreamServiceServer;
use celnet_proto::surface_service_server::SurfaceServiceServer;

use services::pricing::PricingEdge;
use services::quote::QuoteEdge;
use services::stream::StreamEdge;
use services::surface::SurfaceEdge;

/// A fully-wired, running service edge: the gRPC server (all four services), the
/// pricing-core bridge, the maker spread model, the edge clock, and the readiness
/// gate.
///
/// Construct with [`Edge::start`], which binds the gRPC listener on a
/// caller-supplied (typically ephemeral) address and spawns the serving task. The
/// handle exposes the actually-bound address (so tests can dial an OS-assigned
/// port), the readiness gate (to drive blue-green transitions), and a
/// [`Edge::shutdown`] that performs a graceful drain.
#[derive(Debug)]
pub struct Edge {
    grpc_addr: SocketAddr,
    gate: Arc<ReadinessGate>,
    link: Arc<CoreLink>,
    grpc_shutdown: oneshot::Sender<()>,
    grpc_task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl Edge {
    /// Bind the gRPC listener and start serving all four services.
    ///
    /// `grpc_addr` is the requested bind address; pass a port of `0` to let the OS
    /// assign an ephemeral port (read back via [`Edge::grpc_addr`]). The supplied
    /// [`CoreLink`] owns the running pricing core; the [`SpreadModel`] sets the
    /// maker two-way; the [`Clock`] sources edge message timestamps.
    ///
    /// The edge starts in [`ServiceState::Starting`]; call
    /// [`ReadinessGate::mark_ready`] once the core is warm to begin accepting
    /// traffic at the `/readyz` gate.
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if the listener cannot bind or be wrapped for
    /// serving.
    pub async fn start(
        grpc_addr: SocketAddr,
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
    ) -> std::io::Result<Self> {
        let gate = Arc::new(ReadinessGate::new());

        let listener = TcpListener::bind(grpc_addr).await?;
        let bound = listener.local_addr()?;
        let (grpc_shutdown, grpc_rx) = oneshot::channel::<()>();

        let pricing = PricingServiceServer::new(PricingEdge::new(Arc::clone(&gate)));
        let quote = QuoteServiceServer::new(QuoteEdge::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock.clone(),
        ));
        let stream = StreamServiceServer::new(StreamEdge::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock.clone(),
        ));
        let surface = SurfaceServiceServer::new(SurfaceEdge::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            clock,
        ));

        let incoming = tonic::transport::server::TcpIncoming::from_listener(listener, true, None)
            .map_err(std::io::Error::other)?;
        let grpc_task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(pricing)
                .add_service(quote)
                .add_service(stream)
                .add_service(surface)
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = grpc_rx.await;
                })
                .await
        });

        Ok(Self {
            grpc_addr: bound,
            gate,
            link,
            grpc_shutdown,
            grpc_task,
        })
    }

    /// The actually-bound gRPC socket address (resolves an ephemeral `:0` port).
    #[must_use]
    pub fn grpc_addr(&self) -> SocketAddr {
        self.grpc_addr
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
    /// Transitions the readiness gate to [`ServiceState::Draining`] (so `/readyz`
    /// immediately reports not-ready and the orchestrator steers new connections
    /// away), waits for in-flight requests to fall to zero (up to `drain_timeout`),
    /// then signals the serving task to stop accepting and awaits its completion.
    /// No in-flight request is dropped within the timeout.
    pub async fn shutdown(self, drain_timeout: std::time::Duration) {
        self.gate.begin_drain();
        self.gate.await_drained(drain_timeout).await;
        let _ = self.grpc_shutdown.send(());
        let _ = self.grpc_task.await;
    }
}
