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
//! * **`StreamService`** — the multiplexed bidirectional RFS session: one channel
//!   carrying many subscriptions (each per-subscription snapshot + sequenced
//!   deltas + heartbeat + server-assisted resync), in-place
//!   [`celnet_proto::Modify`], and click-to-trade [`celnet_proto::Execute`]
//!   booking off the streamed lines ([`services::stream`]);
//! * **`SurfaceService`** — `GetSmile` / `MarkSurface` / `Scenario`
//!   ([`services::surface`]).
//!
//! # WebSocket mirror
//!
//! Alongside the gRPC server, the edge serves a **WebSocket JSON mirror** ([`ws`])
//! of the *same single, current contract* so a browser/GUI client gets exactly what
//! gRPC clients get — RFQ, the multiplexed RFS [`services::stream`] session
//! (subscribe / snapshot / sequenced-delta / heartbeat / resync + click-to-trade
//! `Execute`-by-token), and surface read/mark/scenario — serialized as type-tagged
//! JSON. It is a second *encoding*, never a contract fork: every WS frame decodes to
//! the same [`celnet_proto`] message and dispatches onto the **same** service edges
//! over the **same** [`CoreLink`] / [`SurfaceBook`], so a WS price is byte-identical
//! to the gRPC/direct one. The mirror shares the [`readiness`] gate and drain (it
//! never blocks the pinned core; a slow socket is paused, not back-pressured).
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
//! * [`surface_book`] — the versioned marked-surface registry every service shares:
//!   `MarkSurface` deposits calibrated smiles under a fresh `surface_version`; the
//!   pricing / RFQ / RFS paths pin a request to a marked surface against it.
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
pub mod surface_book;
pub mod tick;
pub mod ws;

pub use clock::Clock;
pub use core_link::{
    BarrierTopology, CoreLink, CoreLinkError, ExoticQuery, MarketSnapshot, Observable,
    ObservableQuery, SurfaceQuery, SurfaceVol,
};
pub use pricer::{ConventionSet, PriceError, Priced, price_instrument};
pub use readiness::{ReadinessGate, ServiceState};
pub use spread::SpreadModel;
pub use surface_book::{PinError, SurfaceBook};
pub use tick::TickSource;
pub use ws::{WsMirror, WsServices};

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::oneshot;

use celnet_proto::pricing_service_server::PricingServiceServer;
use celnet_proto::quote_service_server::QuoteServiceServer;
use celnet_proto::risk_service_server::RiskServiceServer;
use celnet_proto::stream_service_server::StreamServiceServer;
use celnet_proto::surface_service_server::SurfaceServiceServer;

use services::pricing::PricingEdge;
use services::quote::QuoteEdge;
use services::risk::RiskEdge;
use services::risk::store::PositionStore;
use services::stream::StreamEdge;
use services::surface::SurfaceEdge;

use celnet_risk_fleet::FleetTopology;

/// Read the fleet-risk deploy-time topology from the environment and resolve it via
/// the pure [`FleetTopology::parse`]. `CELNET_FLEET_MODE` selects the mode
/// (`"distributed"` to fan out across shards; anything else is in-process) and
/// `CELNET_FLEET_BACKENDS` carries the comma-separated backend endpoints a
/// distributed transport would dial. Both absent (or `CELNET_FLEET_MODE` anything but
/// the exact `"distributed"`) ⇒ [`FleetTopology::InProcess`], the byte-identical
/// single-node default. This is the one place the env is read; the resolved topology
/// is then threaded into every [`RiskEdge`] (gRPC + WS) at boot.
fn fleet_topology_from_env() -> FleetTopology {
    let mode = std::env::var("CELNET_FLEET_MODE").unwrap_or_default();
    let backends = std::env::var("CELNET_FLEET_BACKENDS").unwrap_or_default();
    FleetTopology::parse(&mode, &backends)
}

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
    ws_addr: SocketAddr,
    gate: Arc<ReadinessGate>,
    link: Arc<CoreLink>,
    surface_book: Arc<SurfaceBook>,
    store: Arc<PositionStore>,
    grpc_shutdown: oneshot::Sender<()>,
    grpc_task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    ws_mirror: WsMirror,
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
        // The WS mirror binds on the same host as the gRPC listener with an
        // OS-assigned ephemeral port (read back via [`Edge::ws_addr`]).
        let ws_addr = SocketAddr::new(grpc_addr.ip(), 0);
        Self::start_on(grpc_addr, ws_addr, link, spread, clock).await
    }

    /// Like [`Edge::start`], but binds the WebSocket mirror on an **explicit**
    /// `ws_addr` (a fixed port) instead of an OS-assigned ephemeral one.
    ///
    /// This is the entry point for a long-lived local demo / verification edge that
    /// an out-of-process client (the Excel add-in harness, the GUI) dials at a
    /// well-known `ws://HOST:PORT`. Pass a port of `0` in `ws_addr` to fall back to
    /// an ephemeral port (the [`Edge::start`] behaviour). The resolved address is
    /// always readable via [`Edge::ws_addr`].
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if either listener cannot bind or be wrapped
    /// for serving.
    pub async fn start_on(
        grpc_addr: SocketAddr,
        ws_addr: SocketAddr,
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
    ) -> std::io::Result<Self> {
        let gate = Arc::new(ReadinessGate::new());
        // The single versioned marked-surface registry every service shares: the
        // surface edge deposits marks; the pricing / RFQ / RFS paths resolve a
        // pinned `surface_version` against it (§ surface_version pinning).
        let surface_book = Arc::new(SurfaceBook::new());
        // The single shared live position book: the RFS click-to-trade path records
        // booked vanilla lines into it, and `RiskService` aggregates the same book
        // (API-first parity: the Book/Risk views read the server's aggregate, never
        // looping positions client-side).
        let store = Arc::new(PositionStore::new());

        // Resolve the fleet-risk deploy-time topology from the environment (the
        // deploy-time-binding precedent of `celnet-integration`'s `DeploymentMode`):
        // `CELNET_FLEET_MODE` + `CELNET_FLEET_BACKENDS` select whether firm risk
        // aggregates in-process or would fan out across physical shards. Absent ⇒
        // `InProcess` (the byte-identical single-node default). Parsing is the pure
        // `FleetTopology::parse`; reading the env (the only I/O) happens here, once,
        // at boot — never on any request path (`CLAUDE.md` rules 6/9; no proto
        // change, no `schema_version`).
        let topology = fleet_topology_from_env();

        let listener = TcpListener::bind(grpc_addr).await?;
        let bound = listener.local_addr()?;
        let (grpc_shutdown, grpc_rx) = oneshot::channel::<()>();

        let pricing = PricingServiceServer::new(PricingEdge::new(
            Arc::clone(&gate),
            Arc::clone(&surface_book),
        ));
        let quote = QuoteServiceServer::new(QuoteEdge::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock.clone(),
            Arc::clone(&surface_book),
        ));
        let stream = StreamServiceServer::new(StreamEdge::with_store(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock.clone(),
            Arc::clone(&surface_book),
            Arc::clone(&store),
        ));
        let surface = SurfaceServiceServer::new(SurfaceEdge::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            clock.clone(),
            Arc::clone(&surface_book),
        ));
        // The risk edge: for a distributed topology this connects the backend fleet
        // (one `celnet_client::Client` per endpoint) so every RiskService RPC fans
        // out across the fleet and reconciles to the single-node answer over the union
        // book (Phase 3, `docs/RISK-HIERARCHY.md` §3.4). A dial failure surfaces here,
        // at boot, as an `io::Error` rather than on the first request. The SAME
        // connected edge backs both the gRPC server and the WS mirror (one fleet of
        // backend channels, not two), shared behind an `Arc`.
        let risk_edge = Arc::new(
            RiskEdge::with_topology_connected(
                Arc::clone(&store),
                Arc::clone(&gate),
                topology.clone(),
            )
            .await
            .map_err(|s| std::io::Error::other(s.to_string()))?,
        );
        let risk = RiskServiceServer::from_arc(Arc::clone(&risk_edge));

        let incoming = tonic::transport::server::TcpIncoming::from_listener(listener, true, None)
            .map_err(std::io::Error::other)?;
        let grpc_task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(pricing)
                .add_service(quote)
                .add_service(stream)
                .add_service(surface)
                .add_service(risk)
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = grpc_rx.await;
                })
                .await
        });

        // The WebSocket JSON mirror: the SAME single contract over WS, driven by the
        // SAME shared services (one `CoreLink`, one `SurfaceBook`, one readiness gate,
        // one spread/clock) — a second encoding of one pricing path, never a fork
        // (`CLAUDE.md` rule 9). Bound on the caller-supplied `ws_addr` (a fixed port
        // for a demo edge, or `:0` for an OS-assigned ephemeral port).
        let ws_services = ws::WsServices::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock,
            Arc::clone(&surface_book),
            Arc::clone(&store),
            Arc::clone(&risk_edge),
        );
        let ws_mirror = ws::WsMirror::start(ws_addr, ws_services).await?;

        Ok(Self {
            grpc_addr: bound,
            ws_addr: ws_mirror.addr(),
            gate,
            link,
            surface_book,
            store,
            grpc_shutdown,
            grpc_task,
            ws_mirror,
        })
    }

    /// The actually-bound gRPC socket address (resolves an ephemeral `:0` port).
    #[must_use]
    pub fn grpc_addr(&self) -> SocketAddr {
        self.grpc_addr
    }

    /// The actually-bound WebSocket-mirror socket address (resolves the ephemeral
    /// port the WS JSON mirror serves the single current contract on).
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

    /// The shared versioned marked-surface registry (so a test or admin path can
    /// inspect which surface versions have been marked).
    #[must_use]
    pub fn surface_book(&self) -> &Arc<SurfaceBook> {
        &self.surface_book
    }

    /// The shared live position book the RFS path books into and `RiskService`
    /// aggregates (so a test / admin path can configure the org hierarchy + limit
    /// tree and inspect the live positions).
    #[must_use]
    pub fn store(&self) -> &Arc<PositionStore> {
        &self.store
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
        // Stop the gRPC listener and the WS-mirror accept loop. The drain above
        // already let in-flight requests (gRPC calls and WS sessions, both holding
        // an in-flight guard on the shared gate) finish; the WS accept loop is then
        // aborted so no new connections are taken (a draining instance also refuses
        // the upgrade at the readiness gate).
        let _ = self.grpc_shutdown.send(());
        let _ = self.grpc_task.await;
        self.ws_mirror.abort();
    }
}
