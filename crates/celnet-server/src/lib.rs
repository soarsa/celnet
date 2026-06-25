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
pub mod config;
pub mod core_link;
pub mod lsv_pricer;
pub mod pricer;
pub mod rates_pricing;
pub mod readiness;
pub mod services;
pub mod spread;
pub mod surface_book;
pub mod tick;
pub mod ws;

pub use clock::Clock;
// The edge's entitlements trust-boundary posture, re-exported on the server facade
// because `store().set_access_mode` takes it: any consumer configuring the edge's
// access mode (a dev/test edge flipping to `Permissive`) needs the type without
// reaching into `celnet-entitlements` directly. Production boot never flips it —
// the construction default is `AccessMode::Enforce` (deny-by-default).
pub use celnet_entitlements::AccessMode;
pub use core_link::{
    BarrierTopology, CoreLink, CoreLinkError, ExoticQuery, MarketSnapshot, Observable,
    ObservableQuery, SurfaceQuery, SurfaceVol,
};
pub use pricer::{ConventionSet, PriceError, Priced, price_instrument};
pub use readiness::{ReadinessGate, ServiceState};
pub use services::pricefanout::{PriceTick, pair_seed, spot_at};
pub use services::quote::LpPanelConfig;
pub use spread::SpreadModel;
pub use surface_book::{PinError, SurfaceBook};
pub use tick::TickSource;
pub use ws::{WsMirror, WsServices};

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::oneshot;

use celnet_proto::auth_service_server::AuthServiceServer;
use celnet_proto::fix_admin_service_server::FixAdminServiceServer;
use celnet_proto::pricing_service_server::PricingServiceServer;
use celnet_proto::quote_service_server::QuoteServiceServer;
use celnet_proto::risk_service_server::RiskServiceServer;
use celnet_proto::stream_service_server::StreamServiceServer;
use celnet_proto::surface_service_server::SurfaceServiceServer;

use config::fix_connections::FixConnectionStore;
use config::identity::IdentityStore;
use services::auth::AuthEdge;
use services::fix::{FixAcceptor, FixContext};
use services::fix_admin::FixAdminEdge;
use services::fix_monitor::FixMonitor;
use services::fix_registry::FixAcceptorRegistry;
use services::sessions::SessionRegistry;

/// The synthetic connection id the legacy env-seeded (`CELNET_FIX_ADDR`) / test-
/// attached acceptor tags its captured frames with in the monitor — it is not a
/// managed `FixConnectionDef`, so it has no persisted id of its own.
const LEGACY_FIX_CONNECTION_ID: &str = "env-default";
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

/// Read the optional live-FIX-acceptor bind address from `CELNET_FIX_ADDR`.
///
/// Absent (or unparseable) ⇒ `None` — no FIX listener is started and the edge is
/// byte-identical to today. Present and a valid `HOST:PORT` ⇒ `Some(addr)` and a FIX
/// 4.4 acceptor binds there at boot. This is the one place the env is read for the FIX
/// edge (the same deploy-time-knob discipline as `CELNET_FLEET_MODE`).
fn fix_addr_from_env() -> Option<SocketAddr> {
    std::env::var("CELNET_FIX_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
}

/// Read the deployment-mode knob from the environment and resolve it via the pure
/// [`DeployMode::parse`]. `CELNET_DEPLOY` selects the mode label (`"hybrid"` ⇒ the
/// Hybrid integration mode, else CelerIntegrated when a feed is configured) and
/// `CELNET_VENDOR_WS` carries the vendor-feed WS endpoint a [`MarketDataSource`] would
/// dial. Both absent (or no usable `CELNET_VENDOR_WS`) ⇒ [`DeployMode::Standalone`], the
/// **byte-identical** default in which no feed and no governor is bound. This is the one
/// place the env is read for the deployment edge (the same discipline as
/// `CELNET_FLEET_MODE` / `CELNET_FIX_ADDR`).
fn deploy_mode_from_env() -> services::deploy::DeployMode {
    let deploy = std::env::var("CELNET_DEPLOY").unwrap_or_default();
    let vendor_ws = std::env::var("CELNET_VENDOR_WS").unwrap_or_default();
    services::deploy::DeployMode::parse(&deploy, &vendor_ws)
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
    /// The optional live FIX 4.4 acceptor edge: `Some` iff `CELNET_FIX_ADDR` was set
    /// (or an explicit address was supplied), bound on that address and serving the
    /// RFQ→Quote→lift→ExecutionReport lifecycle over the same pricing + click-to-trade
    /// token path the gRPC/RFS edges use. Absent ⇒ the edge is byte-identical to today.
    fix_acceptor: Option<FixAcceptor>,
    /// The managed inbound FIX-acceptor registry: the persisted set of operator-defined
    /// acceptor connections (`fix-connections.json`), loaded at boot with the enabled
    /// ones bound, and mutated at runtime via `FixAdminService` (create/update/enable/
    /// delete, each persisted). Independent of the legacy single `fix_acceptor` env seed
    /// above. Its acceptors are stopped on [`Edge::shutdown`].
    fix_registry: Arc<FixAcceptorRegistry>,
    /// The shared FIX session-traffic capture sink behind the monitor screen
    /// (`FixAdminService.ListMessages`). Every managed acceptor and the legacy env
    /// seed record their frames here; retained as a bounded ring buffer.
    fix_monitor: Arc<FixMonitor>,
    /// The optional bound vendor-feed ingress: `Some` iff `CELNET_VENDOR_WS` named a
    /// reachable WS endpoint at boot (the [`DeployMode::WithVendorFeed`] inbound). It
    /// drives the resilient subscriber → normalize → [`SurfaceBook`] deposit → governed
    /// egress pump, sharing the SAME marked-surface registry every gRPC/WS service
    /// prices a pinned request against. Absent (the `Standalone` default) ⇒ no feed and
    /// no governor are bound and the edge is byte-identical to today.
    vendor_feed: Option<services::deploy::VendorFeed>,
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
        // Resolve the deploy-time knobs from the environment, then delegate.
        // Reading the env (the only I/O) happens here, once, at boot: the
        // fleet-risk topology and the synthetic demo/test LP-panel breadth
        // (`CELNET_DEMO_LPS`; absent ⇒ 0 ⇒ the byte-identical single-dealer edge).
        Self::start_on_with_panel(
            grpc_addr,
            ws_addr,
            link,
            spread,
            clock,
            LpPanelConfig::from_env(),
        )
        .await
    }

    /// Like [`Edge::start_on`], but with an **explicit** synthetic demo/test
    /// LP-panel breadth ([`LpPanelConfig`]) instead of reading `CELNET_DEMO_LPS`
    /// from the environment (the fleet topology is still resolved from the
    /// environment, exactly as [`Edge::start_on`]).
    ///
    /// This is the boot path for the local demo edge, which defaults to a ≥3-LP
    /// panel (env-overridable) so live e2e suites exercise the multi-dealer path.
    ///
    /// # Errors
    /// Returns an [`std::io::Error`] if either listener cannot bind, or if a
    /// distributed backend endpoint cannot be dialled.
    pub async fn start_on_with_panel(
        grpc_addr: SocketAddr,
        ws_addr: SocketAddr,
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
        panel: LpPanelConfig,
    ) -> std::io::Result<Self> {
        let topology = fleet_topology_from_env();
        Self::start_on_with_topology(grpc_addr, ws_addr, link, spread, clock, topology, panel).await
    }

    /// Like [`Edge::start_on`], but binds the edge under an **explicit**
    /// [`FleetTopology`] and synthetic LP-panel breadth instead of reading
    /// `CELNET_FLEET_MODE` / `CELNET_FLEET_BACKENDS` / `CELNET_DEMO_LPS` from the
    /// environment.
    ///
    /// This is the race-free entry point for a federation / multi-dealer test that
    /// boots edges on ephemeral ports without mutating process-global env (the env
    /// path is [`Edge::start_on`]). The semantics are otherwise identical: a
    /// distributed topology connects the shared backend [`Fleet`] once at boot (so
    /// the unary pricing/quote/surface services forward by owned pair and the risk
    /// edge federates), and a dial failure surfaces here as an `io::Error`.
    ///
    /// # Errors
    /// Returns an [`std::io::Error`] if either listener cannot bind, or if a
    /// distributed backend endpoint cannot be dialled.
    pub async fn start_on_with_topology(
        grpc_addr: SocketAddr,
        ws_addr: SocketAddr,
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
        topology: FleetTopology,
        panel: LpPanelConfig,
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

        // Connect the backend fleet ONCE for a distributed topology (one
        // `celnet_client::Client` per endpoint, sharing its HTTP/2 channel) and share
        // the SAME `Fleet` across every edge — the unary pricing/quote/surface services
        // (owned-pair forwarding, `docs/SCALE-OUT.md` §3), the stream relay, AND the
        // risk federation (`docs/RISK-HIERARCHY.md` §3.4). In-process ⇒ `None` (every
        // edge serves locally, byte-identical to the single-node default). A dial
        // failure surfaces here, at boot, as an `io::Error` rather than on first
        // request (the channels are established eagerly).
        let fleet: Option<Arc<services::risk::federate::Fleet>> = match &topology {
            FleetTopology::InProcess => None,
            FleetTopology::Distributed { endpoints } => Some(Arc::new(
                services::risk::federate::Fleet::connect(endpoints)
                    .await
                    .map_err(|s| std::io::Error::other(s.to_string()))?,
            )),
        };

        let listener = TcpListener::bind(grpc_addr).await?;
        let bound = listener.local_addr()?;
        let (grpc_shutdown, grpc_rx) = oneshot::channel::<()>();

        let pricing = PricingServiceServer::new(PricingEdge::with_fleet(
            Arc::clone(&gate),
            Arc::clone(&surface_book),
            fleet.clone(),
        ));
        let quote = QuoteServiceServer::new(QuoteEdge::with_fleet(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock.clone(),
            Arc::clone(&surface_book),
            fleet.clone(),
            panel,
        ));
        let stream = StreamServiceServer::new(StreamEdge::with_store_and_fleet(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock.clone(),
            Arc::clone(&surface_book),
            Arc::clone(&store),
            fleet.clone(),
        ));
        let surface = SurfaceServiceServer::new(SurfaceEdge::with_fleet(
            Arc::clone(&link),
            Arc::clone(&gate),
            clock.clone(),
            Arc::clone(&surface_book),
            fleet.clone(),
        ));
        // The risk edge: a distributed topology federates every RiskService RPC across
        // the SAME shared backend fleet (one pool of channels for the whole edge), so
        // it reconciles to the single-node answer over the union book (Phase 3,
        // `docs/RISK-HIERARCHY.md` §3.4). In-process ⇒ the direct single-node path. The
        // SAME connected edge backs both the gRPC server and the WS mirror, shared
        // behind an `Arc`.
        // The edge-wide session registry: the ONE authentication state every front
        // (gRPC + WS) and every gated edge (risk, fix-admin, auth) shares. Created
        // before the edges so each is built `.with_sessions(...)` over it; the
        // registry stamps issue/expiry off the SAME edge clock. Process-local —
        // empty on boot, so a restart invalidates every token.
        let sessions = Arc::new(SessionRegistry::new(clock.clone()));
        let risk_edge = Arc::new(
            match &fleet {
                Some(fleet) => {
                    RiskEdge::with_fleet(Arc::clone(&store), Arc::clone(&gate), Arc::clone(fleet))
                }
                None => RiskEdge::new(Arc::clone(&store), Arc::clone(&gate)),
            }
            .with_sessions(Arc::clone(&sessions)),
        );
        let risk = RiskServiceServer::from_arc(Arc::clone(&risk_edge));

        // The managed inbound FIX-acceptor registry: the persisted set of acceptor
        // connections (`fix-connections.json`, path from `CELNET_FIX_CONFIG`) the
        // operator defines via `FixAdminService`. It shares the SAME pricing core,
        // marked-surface registry, spread and edge clock every other edge uses — a
        // managed FIX RFQ prices through the identical path. Loaded here (a corrupt
        // config fails boot loudly); the enabled acceptors are bound below once the
        // listeners are up. Its entitlement gate reads the same `PositionStore`
        // access mode `RiskService` does, so gRPC and the WS mirror enforce one policy.
        // The shared session-traffic capture sink behind the monitor screen: every
        // managed acceptor (and the legacy env seed) records its inbound/outbound
        // frames here, and `FixAdminService.ListMessages` serves a cursored tail.
        let fix_monitor = Arc::new(FixMonitor::new());
        let fix_registry = Arc::new(
            FixAcceptorRegistry::load(
                Arc::clone(&link),
                spread,
                clock.clone(),
                Arc::clone(&surface_book),
                Arc::clone(&fix_monitor),
                FixConnectionStore::config_path(),
            )
            .map_err(|e| std::io::Error::new(e.kind(), format!("FIX connection config: {e}")))?,
        );
        // ONE admin edge backs both the gRPC server and the WS mirror (shared behind an
        // `Arc`), so the two fronts manage the SAME registry through one entitlement
        // boundary — exactly the single-edge sharing the risk service uses.
        let fix_admin_edge = Arc::new(
            FixAdminEdge::new(
                Arc::clone(&fix_registry),
                Arc::clone(&gate),
                Arc::clone(&store),
                Arc::clone(&fix_monitor),
            )
            .with_sessions(Arc::clone(&sessions)),
        );
        let fix_admin = FixAdminServiceServer::from_arc(Arc::clone(&fix_admin_edge));

        // The persisted operator identity (users + desks, `identity.json` / the
        // `CELNET_IDENTITY_CONFIG` knob): loaded here (a corrupt file fails boot
        // loudly), with the default administrator (`admin@celnet.com` / `password`)
        // seeded and persisted on first run so a fresh edge is always administrable.
        // ONE `AuthEdge` backs both the gRPC server and the WS mirror (shared behind
        // an `Arc`); it mints and validates the server-enforced session tokens every
        // administrative call carries, against the SAME session registry (process-
        // local, emptied on restart). The registry stamps issue/expiry off the SAME
        // edge clock every other service uses.
        let identity_path = IdentityStore::config_path();
        let mut identity_store = IdentityStore::load(&identity_path)
            .map_err(|e| std::io::Error::new(e.kind(), format!("identity config: {e}")))?;
        if identity_store
            .ensure_seed_admin()
            .map_err(std::io::Error::other)?
        {
            // The seeded admin uses a well-known default password — make its
            // presence loud so an operator rotates it before any network exposure.
            tracing::warn!(
                class = celnet_observability::LogClass::Security.label(),
                email = config::identity::SEED_ADMIN_EMAIL,
                "SECURITY: default admin seeded with a well-known password — rotate it \
                 via AuthService.ResetPassword before exposing the edge to any network"
            );
            identity_store
                .save(&identity_path)
                .map_err(|e| std::io::Error::new(e.kind(), format!("seed identity: {e}")))?;
        }
        let auth_edge = Arc::new(AuthEdge::new(
            Arc::new(std::sync::Mutex::new(identity_store)),
            identity_path,
            Arc::clone(&sessions),
            Arc::clone(&gate),
            clock.clone(),
        ));
        let auth = AuthServiceServer::from_arc(Arc::clone(&auth_edge));

        let incoming = tonic::transport::server::TcpIncoming::from_listener(listener, true, None)
            .map_err(std::io::Error::other)?;
        let grpc_task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(pricing)
                .add_service(quote)
                .add_service(stream)
                .add_service(surface)
                .add_service(risk)
                .add_service(fix_admin)
                .add_service(auth)
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
        // Clone the edge clock for the FIX acceptor before `clock` is moved into the
        // WS mirror's services (one clock source shared across every edge).
        let fix_clock = clock.clone();
        let ws_services = ws::WsServices::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock,
            Arc::clone(&surface_book),
            Arc::clone(&store),
            Arc::clone(&risk_edge),
            Arc::clone(&fix_admin_edge),
            Arc::clone(&auth_edge),
            fleet.clone(),
            panel,
        );
        let ws_mirror = ws::WsMirror::start(ws_addr, ws_services).await?;

        // Bind every persisted **enabled** managed acceptor now the edge's listeners
        // are up (a per-connection bind failure is logged and skipped so one bad
        // address can't block the rest). This is the "load on startup when saved"
        // half of the managed-FIX feature; new acceptors created at runtime via
        // `FixAdminService` bind immediately and persist for the next boot.
        fix_registry.start_enabled().await;

        // The optional live FIX 4.4 acceptor edge: bound only when `CELNET_FIX_ADDR`
        // names an address (the deploy-time knob, mirroring `CELNET_FLEET_MODE`). It
        // shares the SAME pricing core, marked-surface registry, spread, and edge clock
        // as the gRPC/WS edges — a FIX RFQ prices and books through the identical paths,
        // never a forked engine. Absent ⇒ no listener and the edge is byte-identical to
        // today. A bad/unbindable address surfaces here, at boot, as an `io::Error`.
        let fix_acceptor = match fix_addr_from_env() {
            Some(addr) => {
                let ctx = FixContext::new(
                    Arc::clone(&link),
                    spread,
                    fix_clock,
                    Arc::clone(&surface_book),
                    Arc::clone(&fix_monitor),
                    LEGACY_FIX_CONNECTION_ID.to_owned(),
                );
                Some(FixAcceptor::start(addr, ctx).await?)
            }
            None => None,
        };

        // The optional vendor-feed ingress: bound only when `CELNET_DEPLOY` /
        // `CELNET_VENDOR_WS` resolve to a `WithVendorFeed` mode (the deploy-time knob,
        // mirroring `CELNET_FLEET_MODE` / `CELNET_FIX_ADDR`). It dials the vendor WS
        // endpoint, drives the resilient subscriber → normalize → deposit into the SAME
        // shared `SurfaceBook`, and drains governed price updates to its own standalone
        // distributor sink. Absent ⇒ `Standalone`: no feed, no governor, byte-identical
        // to today.
        let vendor_feed = match deploy_mode_from_env() {
            services::deploy::DeployMode::Standalone => None,
            services::deploy::DeployMode::WithVendorFeed { vendor_ws, .. } => {
                let source = services::deploy::VendorReplaySource::new(
                    vendor_ws,
                    services::deploy::VendorFeedConfig::default().max_connections,
                );
                // The seeded default subscription (the demo fixture's EURUSD 1Y slice);
                // a real deployment configures its universe here.
                let keys = vec![celnet_integration::SubscriptionKey::new("EURUSD", "1Y")];
                let sink = celnet_integration::StandaloneSink::new();
                Some(
                    services::deploy::VendorFeed::start(
                        source,
                        sink,
                        Arc::clone(&surface_book),
                        keys,
                        services::deploy::VendorFeedConfig::default(),
                    )
                    .map_err(std::io::Error::other)?,
                )
            }
        };

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
            fix_acceptor,
            fix_registry,
            fix_monitor,
            vendor_feed,
        })
    }

    /// The actually-bound gRPC socket address (resolves an ephemeral `:0` port).
    #[must_use]
    pub fn grpc_addr(&self) -> SocketAddr {
        self.grpc_addr
    }

    /// The actually-bound live FIX 4.4 acceptor address, if one is running (`Some` iff
    /// `CELNET_FIX_ADDR` was set at boot, or a FIX acceptor was attached via
    /// [`Edge::attach_fix_acceptor`]). Resolves an ephemeral `:0` port.
    #[must_use]
    pub fn fix_addr(&self) -> Option<SocketAddr> {
        self.fix_acceptor.as_ref().map(FixAcceptor::local_addr)
    }

    /// Attach (bind + start) a live FIX 4.4 acceptor on `addr` against this edge's
    /// shared pricing core + marked-surface registry, with explicit `spread` / `clock`
    /// and the pre-agreed `(sender, counterparty)` CompIDs.
    ///
    /// This is the **race-free** entry point for a test that binds an ephemeral FIX
    /// port and dials it with a real `celnet-fix` initiator, without mutating
    /// process-global `CELNET_FIX_ADDR`. The acceptor shares the SAME `CoreLink`,
    /// `SurfaceBook`, spread, and clock as the gRPC/WS edges — a FIX RFQ prices and
    /// books through the identical paths.
    ///
    /// # Errors
    /// Returns an [`std::io::Error`] if the FIX listener cannot bind on `addr`.
    pub async fn attach_fix_acceptor(
        &mut self,
        addr: SocketAddr,
        spread: SpreadModel,
        clock: Clock,
        sender: Vec<u8>,
        counterparty: Vec<u8>,
    ) -> std::io::Result<SocketAddr> {
        let ctx = FixContext::with_comp_ids(
            Arc::clone(&self.link),
            spread,
            clock,
            Arc::clone(&self.surface_book),
            sender,
            counterparty,
            Arc::clone(&self.fix_monitor),
            LEGACY_FIX_CONNECTION_ID.to_owned(),
        );
        let acceptor = FixAcceptor::start(addr, ctx).await?;
        let bound = acceptor.local_addr();
        if let Some(prev) = self.fix_acceptor.replace(acceptor) {
            prev.abort();
        }
        Ok(bound)
    }

    /// The actually-bound WebSocket-mirror socket address (resolves the ephemeral
    /// port the WS JSON mirror serves the single current contract on).
    #[must_use]
    pub fn ws_addr(&self) -> SocketAddr {
        self.ws_addr
    }

    /// The bound vendor-feed ingress, if one is running (`Some` iff
    /// `CELNET_VENDOR_WS` resolved a `WithVendorFeed` mode at boot, or a feed was
    /// attached via [`Edge::attach_vendor_feed`]). Exposes the governed-egress metrics
    /// (offered / delivered / conflated / capacity drops) for an ops scraper / a test.
    #[must_use]
    pub fn vendor_feed(&self) -> Option<&services::deploy::VendorFeed> {
        self.vendor_feed.as_ref()
    }

    /// Attach (bind + start) a vendor-feed ingress dialling `source`, draining governed
    /// price updates to `sink`, depositing calibrated smiles into this edge's shared
    /// [`SurfaceBook`] under fresh surface versions. `keys` are the `(pair, tenor)`
    /// subscriptions; `cfg` configures the governed egress + connection economy.
    ///
    /// This is the **race-free** entry point for a test that boots a vendor replay
    /// server on an ephemeral port and binds a feed against it without mutating
    /// process-global `CELNET_VENDOR_WS`. The feed shares the SAME `SurfaceBook` the
    /// gRPC/WS price paths pin against — a marked-from-feed surface is reproducible to
    /// the bit on the same path the rest of the edge uses.
    pub fn attach_vendor_feed<S, K>(
        &mut self,
        source: S,
        sink: K,
        keys: Vec<celnet_integration::SubscriptionKey>,
        cfg: services::deploy::VendorFeedConfig,
    ) -> Result<(), celnet_integration::EgressError>
    where
        S: celnet_integration::MarketDataSource + Send + Sync + 'static,
        S::Transport: Send,
        <S::Transport as celnet_integration::FeedTransport>::Error: Send,
        K: celnet_integration::PriceSink + Send + 'static,
    {
        let feed = services::deploy::VendorFeed::start(
            source,
            sink,
            Arc::clone(&self.surface_book),
            keys,
            cfg,
        )?;
        if let Some(prev) = self.vendor_feed.replace(feed) {
            prev.abort();
        }
        Ok(())
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
        if let Some(fix) = self.fix_acceptor {
            fix.abort();
        }
        // Stop every managed acceptor (in-flight sessions run to their own close).
        self.fix_registry.abort_all().await;
        if let Some(feed) = self.vendor_feed {
            feed.abort();
        }
    }
}
