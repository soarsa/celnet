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
pub mod xva_pricing;

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
pub use pricer::{
    ConventionSet, PriceError, Priced, PricingEngine, price_exotic_via_contract, price_instrument,
};
pub use readiness::{ReadinessGate, ServiceState};
pub use services::pricefanout::{PriceTick, pair_seed, spot_at, underlying_seed};
pub use services::quote::LpPanelConfig;
pub use spread::SpreadModel;
pub use surface_book::{MarkedCurve, PinError, SurfaceBook};
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

use config::consistency::ConsistencyPolicy;
use config::fix_connections::FixConnectionStore;
use config::identity::IdentityStore;
use services::auth::AuthEdge;
use services::consensus::{ConsensusBoot, ConsensusHandle, boot_if_strong_async};
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
    /// The optional activated consistency tier (ADR-0015): `Some` iff a `Strong`-tier
    /// book was configured and Raft was booted at start. Held so the node's serve / tick
    /// threads are torn down with the edge (its `Drop` stops them on the last `Arc`);
    /// `None` (the pure-`Local` default) means no consensus node was ever bound.
    consensus: Option<Arc<ConsensusHandle>>,
}

impl Edge {
    /// Bind the gRPC listener and start serving all four services.
    ///
    /// `grpc_addr` is the requested bind address; pass a port of `0` to let the OS
    /// assign an ephemeral port (read back via [`Edge::grpc_addr`]). The supplied
    /// [`CoreLink`] owns the running pricing core; the [`SpreadModel`] sets the
    /// maker two-way; the [`Clock`] sources edge message timestamps.
    ///
    /// `data_dir` roots the persisted operator config files (`fix-connections.json`,
    /// `identity.json`); pass `None` for the production default (the
    /// `CELNET_FIX_CONFIG` / `CELNET_IDENTITY_CONFIG` env knob, or the CWD-relative
    /// fallback — unchanged behaviour). Tests pass a per-edge temp dir so parallel
    /// edges get isolated identity/connection stores instead of racing one shared
    /// path.
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
        data_dir: Option<&std::path::Path>,
    ) -> std::io::Result<Self> {
        // The WS mirror binds on the same host as the gRPC listener with an
        // OS-assigned ephemeral port (read back via [`Edge::ws_addr`]).
        let ws_addr = SocketAddr::new(grpc_addr.ip(), 0);
        Self::start_on(grpc_addr, ws_addr, link, spread, clock, data_dir).await
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
        data_dir: Option<&std::path::Path>,
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
            data_dir,
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
        data_dir: Option<&std::path::Path>,
    ) -> std::io::Result<Self> {
        let topology = fleet_topology_from_env();
        Self::start_on_with_topology(
            grpc_addr, ws_addr, link, spread, clock, topology, panel, data_dir,
        )
        .await
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
    // The fully-explicit boot path: every deploy-time knob the env-reading entry
    // points resolve (grpc/ws addrs, spread, clock, fleet topology, LP panel, and the
    // data-dir root for the persisted identity/fix-connection stores) is passed
    // directly so a federation/multi-dealer test is race-free against process-global
    // env. The argument count is intrinsic to that "no hidden env" contract.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_on_with_topology(
        grpc_addr: SocketAddr,
        ws_addr: SocketAddr,
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
        topology: FleetTopology,
        panel: LpPanelConfig,
        data_dir: Option<&std::path::Path>,
    ) -> std::io::Result<Self> {
        let gate = Arc::new(ReadinessGate::new());
        // The single versioned marked-surface registry every service shares: the
        // surface edge deposits marks; the pricing / RFQ / RFS paths resolve a
        // pinned `surface_version` against it (§ surface_version pinning).
        let surface_book = Arc::new(SurfaceBook::new());
        // The multi-curve registry: load the persisted named curve definitions
        // (`curve-definitions.json`, path from `CELNET_CURVE_CONFIG` or the data dir),
        // install any saved set over the seeded USD-SOFR primary, wire the persistence
        // path so CRUD mutations survive restart, and materialise the seed on first run
        // so the registry file exists. The `SurfaceService` curve-CRUD verbs manage it.
        {
            let curve_path = data_dir
                .map(|d| d.join(config::curve_definitions::DEFAULT_CONFIG_PATH))
                .unwrap_or_else(config::curve_definitions::CurveDefinitionStore::config_path);
            let curve_store = config::curve_definitions::CurveDefinitionStore::load(&curve_path)
                .map_err(|e| {
                    std::io::Error::new(e.kind(), format!("curve definition config: {e}"))
                })?;
            if !curve_store.curves.is_empty() {
                surface_book.install_curve_registry(curve_store);
            }
            surface_book.set_curve_persistence(curve_path);
            surface_book.persist_curves().map_err(|e| {
                std::io::Error::new(e.kind(), format!("persist curve definitions: {e}"))
            })?;
        }
        // The single shared live position book: the RFS click-to-trade path records
        // booked vanilla lines into it, and `RiskService` aggregates the same book
        // (API-first parity: the Book/Risk views read the server's aggregate, never
        // looping positions client-side).
        let store = Arc::new(PositionStore::new());
        // Wire the shared latency/ops telemetry hub (owned by the `CoreLink`) into the
        // position store so a successful FX booking records its ack→fill→book commit
        // latency (best-order timer O3), off the pinned pricing thread.
        store.set_telemetry(Arc::clone(link.telemetry()));

        // The edge-wide aggregated-book engine hub (D3): shared by the LP ingest
        // service (which feeds it) and the stream service (which reads its
        // composite). Reconciled to the persisted enabled books once the identity
        // store is loaded (below), and again on every admin book CRUD (via the
        // AuthEdge). Bound to the edge clock so staleness decay measures quote age.
        // The firm-wide runtime **pricing kill-switch** (server-only control plane). ONE
        // shared control threaded into the aggregation ingest (inbound gate), every FIX
        // session (outbound gate), the AuthEdge (the `SetPricingControl` RPC), and the WS
        // layer (change fan-out). Seeded both-enabled here and reconciled to the operator's
        // persisted setting once the identity store is loaded (below) — so a bounce restores
        // a halt rather than silently resuming pricing.
        let pricing_control = services::pricing_control::PricingControl::new(true, true);
        let aggregation_hub = services::aggregation::AggregationHub::with_control(
            clock.clone(),
            Arc::clone(&pricing_control),
        );

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
        // The edge-wide session registry: the ONE authentication state every front
        // (gRPC + WS) and every gated edge (quote, stream, risk, fix-admin, auth)
        // shares. Created before the edges so each is built `.with_sessions(...)` /
        // `.with_session_access(...)` over it; the registry stamps issue/expiry off
        // the SAME edge clock. Process-local — empty on boot, so a restart
        // invalidates every token.
        let sessions = Arc::new(SessionRegistry::new(clock.clone()));
        // Built behind an `Arc` (via `from_arc`) so the SAME quote edge that serves RFQs
        // also backs the client-flow analytics fold as its FXO source (Analytics phase 2).
        let quote_edge = Arc::new(
            QuoteEdge::with_fleet(
                Arc::clone(&link),
                Arc::clone(&gate),
                spread,
                clock.clone(),
                Arc::clone(&surface_book),
                fleet.clone(),
                panel,
            )
            // The RFQ caller gate (item B §2) validates `session_token` against the
            // shared registry and reads the live access mode off the shared store.
            .with_session_access(Arc::clone(&sessions), Arc::clone(&store))
            // Phase 2b: the SAME aggregated-book engine hub the LP ingest / stream / auth
            // edges share, so an inbound RFQ on an instrument an admin-defined book covers
            // prices against that book's already-tiered composite (and ranks its member-LP
            // lines) instead of the synthetic-demo panel.
            .with_aggregation_hub(Arc::clone(&aggregation_hub)),
        );
        let quote = QuoteServiceServer::from_arc(Arc::clone(&quote_edge));
        // The shared LINEAR-RATES position book (created here so the stream + auth edges can
        // share it): ONE book read/written by the RiskService rates Book/List RPCs and the
        // RfqDeskService (whose AcceptDeskQuote books accepted deals into it), and read by the
        // stream edge for the per-book risk roster. Its risk router is primed below beside the
        // FX store's, so a routed rates fill buckets into its risk book from first boot.
        let rates_store = Arc::new(services::rates_book::RatesPositionStore::new());
        // Wire the SAME shared latency/ops telemetry hub (owned by the `CoreLink`) into the
        // rates book so the FI booking seams record their per-stage latency (best-order booking
        // commit → Book, risk-routing decision → RiskRoute, auto-hedge fire → HedgeFire), and
        // so the desk edge (which holds this store) can record the quote→lift accept span →
        // QuoteAccept. Off the pinned pricing thread, uniform with the FX `store.set_telemetry`.
        rates_store.set_telemetry(Arc::clone(link.telemetry()));
        let stream = StreamServiceServer::new(
            StreamEdge::with_store_and_fleet(
                Arc::clone(&link),
                Arc::clone(&gate),
                spread,
                clock.clone(),
                Arc::clone(&surface_book),
                Arc::clone(&store),
                fleet.clone(),
            )
            .with_sessions(Arc::clone(&sessions))
            .with_aggregation_hub(Arc::clone(&aggregation_hub))
            // The per-book risk stream sums the rates positions routed into each book beside
            // the FX ones and advances on a rates fill.
            .with_rates_store(Arc::clone(&rates_store)),
        );
        let surface = SurfaceServiceServer::new(
            SurfaceEdge::with_fleet(
                Arc::clone(&link),
                Arc::clone(&gate),
                clock.clone(),
                Arc::clone(&surface_book),
                fleet.clone(),
            )
            // Share the edge-wide session registry so the curve-definition CRUD verbs
            // authenticate `session_token`s against the sessions `AuthService` issues
            // (the WS mirror's SurfaceEdge is wired the same way in `ws::WsServices`).
            .with_sessions(Arc::clone(&sessions)),
        );
        // The risk edge: a distributed topology federates every RiskService RPC across
        // the SAME shared backend fleet (one pool of channels for the whole edge), so
        // it reconciles to the single-node answer over the union book (Phase 3,
        // `docs/RISK-HIERARCHY.md` §3.4). In-process ⇒ the direct single-node path. The
        // SAME connected edge backs both the gRPC server and the WS mirror, shared
        // behind an `Arc`. It shares the edge-wide `sessions` registry built above.
        // The shared dealer-quoting stores + notification broker: the `rates_store` (created
        // above, shared with the stream edge) is read/written by both the RiskService rates
        // Book/List RPCs and the RfqDeskService, so the Book workspace and the desk blotter
        // stay coherent.
        let desk_requests = Arc::new(services::desk::store::DeskRequestStore::new());
        let deals_store = Arc::new(services::desk::store::DealStore::new());
        let notify_broker = Arc::new(services::desk::notify::NotificationBroker::new());
        let risk_edge = Arc::new(
            match &fleet {
                Some(fleet) => {
                    RiskEdge::with_fleet(Arc::clone(&store), Arc::clone(&gate), Arc::clone(fleet))
                }
                None => RiskEdge::new(Arc::clone(&store), Arc::clone(&gate)),
            }
            .with_sessions(Arc::clone(&sessions))
            .with_rates_store(Arc::clone(&rates_store)),
        );
        let risk = RiskServiceServer::from_arc(Arc::clone(&risk_edge));
        // The dealer-quoting desk edge implements BOTH RfqDeskService (capture /
        // respond / accept / reads) and NotificationService (the push stream); one
        // Arc is registered under both generated service servers.
        let rfq_desk_edge = Arc::new(services::desk::RfqDeskEdge::new(
            Arc::clone(&store),
            Arc::clone(&sessions),
            Arc::clone(&gate),
            Arc::clone(&desk_requests),
            Arc::clone(&deals_store),
            Arc::clone(&rates_store),
            Arc::clone(&notify_broker),
            clock.clone(),
        ));
        let rfq_desk = celnet_proto::rfq_desk_service_server::RfqDeskServiceServer::from_arc(
            Arc::clone(&rfq_desk_edge),
        );
        let notifications =
            celnet_proto::notification_service_server::NotificationServiceServer::from_arc(
                Arc::clone(&rfq_desk_edge),
            );

        // The bond corporate-actions edge: the effective-dated, journal-backed golden
        // source (seeded once from the deterministic OSS govvie universe) + the shared
        // rates book a confirmed+applied CA realises/scales into (the SAME `book` path a
        // trade uses). Off the pinned hot core; the reads are on the `view` floor, the
        // confirm/apply writes gated on the `refdata` capability. ONE edge backs the gRPC
        // server and the WS mirror (shared behind an `Arc`), like every other service.
        let corp_actions_journal = data_dir
            .map(|d| d.join("corpactions.journal"))
            .unwrap_or_else(|| std::env::temp_dir().join("celnet-corpactions.journal"));
        let corp_store = celnet_refstore::GoldenSourceStore::open(&corp_actions_journal)
            .map_err(|e| std::io::Error::other(format!("corporate-actions golden store: {e}")))?;
        let corp_actions_edge = Arc::new(services::corpactions::CorporateActionsEdge::new(
            Arc::clone(&sessions),
            Arc::clone(&gate),
            Arc::clone(&rates_store),
            corp_store,
        ));
        // Seed the golden source once from the curated OSS govvie universe (masters + the
        // next upcoming coupon/redemption per instrument) so the schedule + CA-inbox reads
        // resolve from first boot. Idempotent; a seed failure is non-fatal (the reads just
        // return empty) — never fail the whole edge over reference-data seeding.
        if let Err(e) = corp_actions_edge.seed_from_source(
            &celnet_refstore::GovvieSource::curated(),
            celnet_corpactions::CivilDate::new(2026, 1, 1),
        ) {
            tracing::warn!(error = %e, "corporate-actions golden source seeding skipped");
        }
        let corp_actions =
            celnet_proto::corporate_actions_service_server::CorporateActionsServiceServer::from_arc(
                Arc::clone(&corp_actions_edge),
            );

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
                Arc::clone(&store),
                // The desk inbox a managed fixed-income acceptor records inbound RFQs
                // into (auto-quoted history + human-routed pending), so the GUI desk
                // shows what a FIX venue received.
                Some(Arc::clone(&rfq_desk_edge)),
                data_dir
                    .map(|d| d.join("fix-connections.json"))
                    .unwrap_or_else(FixConnectionStore::config_path),
            )
            .map_err(|e| std::io::Error::new(e.kind(), format!("FIX connection config: {e}")))?,
        );
        // Price a managed fixed-income STREAM acceptor's outbound RFS/ESP off the
        // aggregated-book composite through the connection's pricing group (design §5):
        // wire the SAME hub the gRPC/WS RFQ path prices against, so the FIX stream is
        // composite-based + tiered too (closing the deferred rates-RFS group-pricing seam).
        // Set before the enabled acceptors bind below.
        fix_registry.set_aggregation_hub(Arc::clone(&aggregation_hub));
        // Gate every managed acceptor's outbound RFQ auto-quotes + RFS/ESP streams on the
        // firm-wide kill-switch (the SAME runtime control). Set before the enabled
        // acceptors bind below; the persisted setting is applied to the shared control
        // once the identity store loads (above).
        fix_registry.set_pricing_control(Arc::clone(&pricing_control));
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
        let identity_path = data_dir
            .map(|d| d.join("identity.json"))
            .unwrap_or_else(IdentityStore::config_path);
        let mut identity_store = IdentityStore::load(&identity_path)
            .map_err(|e| std::io::Error::new(e.kind(), format!("identity config: {e}")))?;
        let admin_seeded = identity_store
            .ensure_seed_admin()
            .map_err(std::io::Error::other)?;
        if admin_seeded {
            // The seeded admin uses a well-known default password — make its
            // presence loud so an operator rotates it before any network exposure.
            tracing::warn!(
                class = celnet_observability::LogClass::Security.label(),
                email = config::identity::SEED_ADMIN_EMAIL,
                "SECURITY: default admin seeded with a well-known password — rotate it \
                 via AuthService.ResetPassword before exposing the edge to any network"
            );
        }
        // Seed a small, realistic entity/book registry on a fresh store so the rates
        // booking form has named legal-entities/accounts + books from first boot (the
        // wire keys stay numeric; this only names them). Idempotent — a no-op once any
        // entity exists.
        let registry_seeded = identity_store.ensure_seed_registry();
        // Seed a small, realistic instrument reference-data registry (a USD-SOFR
        // rates strip + sample bonds) on a fresh store so curve-building/pricing have
        // resolvable instrument definitions from first boot. Idempotent — a no-op once
        // any instrument exists.
        let instruments_seeded = identity_store.ensure_seed_instruments();
        // Additively ensure the curated government-bond universe (US Treasuries + UK
        // gilts + EUR govvies) is registered — on EVERY boot, so an already-populated
        // store gains the bonds without clobbering admin edits. This is what makes the
        // FI Aggregated Book tiles resolve real names + ISIN/CUSIP (the LP-SIM feed
        // streams the same instrument_ids) and powers the region/sub-asset-type
        // security-list download.
        let gov_bonds_seeded = identity_store.ensure_seed_government_bonds();
        // Seed a default ENABLED "Firm Warehouse" risk book + a default single-leaf routing
        // graph that targets it on a PRISTINE store, so an accepted / lifted fill routes into
        // an enabled risk book and the per-book risk dashboard shows a row out of the box
        // (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §3.1/§8.2). Idempotent — a no-op once any
        // risk book or routing graph exists. Runs BEFORE the routers are primed below so the
        // seeded graph takes effect from first boot.
        let risk_routing_seeded = identity_store.ensure_seed_risk_routing();
        // Seed a default auto-hedge / internalisation policy (a warehouse-vs-hedge exit graph +
        // a DV01 warehouse threshold on the default warehouse book) on a pristine hedge config,
        // so a booked FI fill carries a real internalise decision on the deal blotter from first
        // boot (§4/§6). Idempotent + advisory-only (nothing trades externally). Runs AFTER the
        // risk-routing seed (it binds to the default warehouse book that seed creates).
        let hedge_policy_seeded = identity_store.ensure_seed_hedge_policy();
        // Seed the default ACCEPT-ALL acceptance graph on a pristine store so the FIX
        // acceptance point evaluates a real (identity) policy from first boot — every lift
        // is accepted and books exactly as before, until a trader writes rules (the third
        // trader-configurable rule engine — `celnet-acceptance`). Idempotent.
        let acceptance_seeded = identity_store.ensure_seed_acceptance();
        if admin_seeded
            || registry_seeded
            || instruments_seeded
            || gov_bonds_seeded
            || risk_routing_seeded
            || hedge_policy_seeded
            || acceptance_seeded
        {
            identity_store
                .save(&identity_path)
                .map_err(|e| std::io::Error::new(e.kind(), format!("seed identity: {e}")))?;
        }
        // The desk-identity bridge (item B §3): populate the shared position book's
        // `Book → Desk` hierarchy from the configured desk→book membership, so a
        // logged-in trader's session narrows its risk reads to exactly the facts
        // booked into their desk's books. Empty-`books` desks are no-ops. Runs before
        // `identity_store` is moved into the `AuthEdge`.
        for d in &identity_store.desks {
            store.configure_desk(&d.id, &d.books);
        }

        // Stand up an engine for every persisted ENABLED aggregated book (D3), so a
        // GUI can subscribe to its composite and an LP feed lands into it from first
        // boot. Runs before `identity_store` is moved into the `AuthEdge`; every
        // admin book CRUD re-reconciles (see `AuthEdge::with_aggregation_hub`).
        aggregation_hub.reconcile(&identity_store);

        // Restore the operator's persisted firm-wide pricing kill-switch into the runtime
        // control (a bounce must not silently resume pricing an operator halted). Applied
        // before any FIX venue binds or LP feed lands, so the gates are correct from the
        // first tick. Both-enabled (the default) is a no-op re-seed.
        {
            let persisted = identity_store.pricing_control();
            let current = pricing_control.snapshot();
            // Only apply (and bump the version) when the persisted setting actually
            // differs from the both-enabled default — a fresh store stays at version 1.
            if persisted.outbound_enabled != current.outbound_enabled
                || persisted.inbound_enabled != current.inbound_enabled
            {
                pricing_control.set(persisted.outbound_enabled, persisted.inbound_enabled);
            }
        }

        // Prime the shared position store's risk router from the persisted firm-wide
        // routing graph (phase 4 boot reconcile, `docs/FI-RISK-ROUTING-REQUIREMENTS.md`
        // §7): every subsequent fill through `book_from_attribution` is routed to its
        // risk book from first boot. `None` (no graph persisted yet) leaves routing off —
        // fills book byte-identically to before. Runs before `identity_store` is moved
        // into the `AuthEdge`; every admin risk-book/graph write re-primes it (see
        // `AuthEdge::with_position_store` / `reconcile_risk_routing`).
        store.set_routing(identity_store.risk_routing_graph().cloned());
        // Prime the store's per-book limit view beside the graph (§8.3 enforcement), so a
        // routed fill that would breach its risk book's (or an ancestor's) hard notional cap
        // is refused from first boot. Every admin risk-book write re-primes it (see
        // `AuthEdge::reconcile_risk_routing`). No books / no caps ⇒ the gate is skipped and
        // booking is byte-identical to before.
        store.set_risk_books(
            identity_store
                .risk_books
                .iter()
                .map(crate::services::risk::store::RiskBookLimitDef::from)
                .collect(),
        );
        // Prime the store's full risk-book tree beside the limit view, so the live per-book
        // risk stream aggregates over the persisted book set from first boot. Every admin
        // risk-book write re-primes it (see `AuthEdge::reconcile_risk_routing`).
        store.set_risk_book_tree(identity_store.risk_books.clone());
        // Prime the shared LINEAR-RATES store's router + per-book limit view beside the FX
        // store, so a routed rates fill (RiskService::BookRatesPosition or the RFQ desk's
        // AcceptDeskQuote) buckets into — and is capped by — the same risk books from first
        // boot. `None`/no caps ⇒ unrouted, byte-identical. Every admin risk-book/graph write
        // re-primes it (see `AuthEdge::reconcile_risk_routing`).
        rates_store.set_routing(identity_store.risk_routing_graph().cloned());
        rates_store.set_risk_books(
            identity_store
                .risk_books
                .iter()
                .map(crate::services::risk::store::RiskBookLimitDef::from)
                .collect(),
        );
        // The shared off-core auto-hedge decision + provenance engine (Phase B). The SAME
        // `Arc` is primed into the rates store's hedge-policy snapshot (so a booked FI fill's
        // internalise decision stamps into its ring) AND handed to the `AuthEdge` (so the
        // `ListHedgeProvenance` RPC reads that same audit ring).
        let auto_hedge_engine = Arc::new(services::auto_hedge::AutoHedgeEngine::default());
        // Prime the rates store's auto-hedge / internalisation policy from the persisted config
        // (the exit graph + warehouse thresholds + engine config + known-LP set), so a booked
        // RFQ-desk / FIX-lift fill carrying a priced reference mid resolves its warehouse-vs-
        // external decision + tolerance verdict from first boot (§6). Re-primed on every admin
        // hedge write via `AuthEdge::reconcile_hedge_policy`. `None`/no threshold ⇒ no decision.
        rates_store.set_hedge_policy(Some(services::rates_book::RatesHedgePolicy {
            engine: Arc::clone(&auto_hedge_engine),
            graph: identity_store.hedge_policy_graph().cloned(),
            thresholds: identity_store.hedge_thresholds().to_vec(),
            config: identity_store.hedge_config().clone(),
            known_lps: identity_store.known_hedge_lps(),
        }));
        // Prime the rates store's incoming-quote-acceptance graph from the persisted store
        // (seeded ACCEPT-ALL on a pristine store), so the FIX acceptance point gates inbound
        // lifts from first boot. Re-primed on every admin acceptance write via
        // `AuthEdge::reconcile_acceptance`. `None` ⇒ acceptance off (every lift accepted).
        rates_store.set_acceptance(identity_store.acceptance_graph().cloned());

        // ADR-0015 §2.1: activate the configurable consistency tier — Raft **wired
        // everywhere but forced nowhere**. A `RaftNode` is booted ONLY when a
        // `Strong`-tier book / desk / tenant is configured (else zero overhead: a
        // pure-`Local` fleet is byte-identical to the single-node fast path). The
        // resolved handle is shared by the FX + rates position sinks; a `Strong` book's
        // ms-scale quorum commit runs on this async booking / state tier, NEVER on the
        // pinned pricing thread (§4.3). The `book → desk` membership is folded in first
        // so a book inherits its desk's level in the cascade. Raft peers come from the
        // dedicated transport knob (`CELNET_RAFT_PEERS`); `InProcess` (the default) boots
        // an inert single-node group. The blocking boot (socket bind + bounded leader
        // wait) runs off the reactor.
        let mut consistency = ConsistencyPolicy::from_env();
        for d in &identity_store.desks {
            for b in &d.books {
                consistency.map_book_to_desk(b.clone(), d.id.clone());
            }
        }
        let consensus =
            boot_if_strong_async(consistency, ConsensusBoot::from_env(data_dir)).await?;
        if let Some(handle) = &consensus {
            store.set_consensus(Arc::clone(handle));
            rates_store.set_consensus(Arc::clone(handle));
        }

        // The shared identity store backs both the AuthService (users/desks CRUD)
        // and the FIX registry's routing-desk directory, so a connection's routing
        // desk is validated against the SAME live desk set an admin edits. Injected
        // set-once before `start_enabled` binds acceptors, so the boot-time guard can
        // warn on an unresolved routing desk.
        let identity_arc = Arc::new(std::sync::Mutex::new(identity_store));
        fix_registry.set_desk_directory(Arc::clone(&identity_arc) as _);
        // The risk-transfer service (RISK-TRANSFER-REQUIREMENTS §11.2/§11.5): the
        // transfer registry + inbox broker + apply engine over the SHARED FX + rates
        // stores and the identity registry, wired into BOTH the `AuthService` transfer
        // RPCs and the `NotificationService` inbox push. Off the pinned pricing core
        // (guardrail 11 — the apply runs on the async booking tier the sinks run on).
        let transfer_service = Arc::new(services::risk_transfer::RiskTransferService::new(
            Arc::new(services::risk_transfer::RiskTransferRegistry::new()),
            Arc::new(services::risk_transfer::RiskTransferBroker::new()),
            Arc::new(services::transfer_apply::TransferApplier::new(
                Arc::clone(&store),
                Arc::clone(&rates_store),
            )),
            Arc::clone(&identity_arc),
        ));
        // The desk edge (already `Arc`-shared) publishes/serves the inbox push over the
        // SAME service.
        rfq_desk_edge.set_transfer_service(Arc::clone(&transfer_service));
        let auth_edge = Arc::new(
            AuthEdge::new(
                Arc::clone(&identity_arc),
                identity_path,
                Arc::clone(&sessions),
                Arc::clone(&gate),
                clock.clone(),
            )
            // Re-reconcile the aggregated-book engines after every admin book CRUD
            // (create/update/delete) so a new/edited/disabled book stands up or tears
            // down its engine immediately (D3).
            .with_aggregation_hub(Arc::clone(&aggregation_hub))
            // Re-prime the shared position store's risk router after every admin
            // risk-book/graph write (phase 4 reconcile) so routing takes effect on
            // subsequent fills immediately.
            .with_position_store(Arc::clone(&store))
            // Re-prime the shared LINEAR-RATES store's router beside the FX store, and sum its
            // routed positions into the `ListRiskBookRisk` roll-up.
            .with_rates_store(Arc::clone(&rates_store))
            // Share the SAME auto-hedge engine the rates store's internalise decision stamps
            // into, so `ListHedgeProvenance` serves that live audit ring; admin hedge writes
            // then re-prime the rates store's hedge-policy snapshot.
            .with_auto_hedge_engine(Arc::clone(&auto_hedge_engine))
            // Back the transfer RPCs (initiate / accept / reject / cancel / list) with the
            // shared transfer service.
            .with_transfer_service(Arc::clone(&transfer_service))
            // Register the cross-asset client-flow analytics sources (Analytics phase 2):
            // the FXO quote edge + the FI desk edge, each folding its OWN already-captured
            // history into FlowRecords on-query, off the hot path.
            .with_client_flow_source(
                Arc::clone(&quote_edge) as Arc<dyn services::analytics::ClientFlowSource>
            )
            .with_client_flow_source(
                Arc::clone(&rfq_desk_edge) as Arc<dyn services::analytics::ClientFlowSource>
            )
            // Register the street-side / LP liquidity analytics sources (§2.4): the FXO
            // quote edge (RFQ panel win/miss/last-look outcomes) + the aggregation hub
            // (per-LP quote-update tick tally), each folded on-query, off the hot path.
            .with_lp_flow_source(
                Arc::clone(&quote_edge) as Arc<dyn services::analytics::lp::LpFlowSource>
            )
            .with_lp_flow_source(
                Arc::clone(&aggregation_hub) as Arc<dyn services::analytics::lp::LpFlowSource>
            )
            // Back the Latency/Ops analytics RPC with the SAME telemetry hub the
            // `CoreLink` owns — the pinned core + async edges fold their per-stage
            // latency into it, and this RPC reads that store (Analytics pillar B).
            .with_telemetry(Arc::clone(link.telemetry()))
            // Drive the firm-wide pricing kill-switch (`SetPricingControl`) through the
            // SAME runtime control the aggregation ingest + FIX seams read and the WS
            // layer fans out.
            .with_pricing_control(Arc::clone(&pricing_control)),
        );
        let auth = AuthServiceServer::from_arc(Arc::clone(&auth_edge));

        // The backend LP-quote ingest (D3): a machine-to-machine feed that routes
        // pushed `LpQuote`s into the shared aggregation hub. gRPC only (no WS mirror).
        let liquidity_feed =
            celnet_proto::liquidity_feed_service_server::LiquidityFeedServiceServer::new(
                services::liquidity_feed::LiquidityFeedEdge::new(
                    Arc::clone(&aggregation_hub),
                    Arc::clone(&gate),
                ),
            );

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
                .add_service(rfq_desk)
                .add_service(notifications)
                .add_service(corp_actions)
                .add_service(liquidity_feed)
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
            // The shared linear-rates position book — wired onto the WS `StreamEdge` so the
            // WS-mirror per-book risk stream aggregates FI fills identically to gRPC.
            Arc::clone(&rates_store),
            Arc::clone(&sessions),
            Arc::clone(&risk_edge),
            Arc::clone(&fix_admin_edge),
            Arc::clone(&auth_edge),
            Arc::clone(&rfq_desk_edge),
            Arc::clone(&corp_actions_edge),
            fleet.clone(),
            panel,
            Arc::clone(&aggregation_hub),
            Arc::clone(&pricing_control),
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
                    Arc::clone(&store),
                )
                // Gate the legacy env-seeded acceptor's outbound FX auto-quotes on the
                // firm-wide kill-switch (the SAME runtime control).
                .with_pricing_control(Some(Arc::clone(&pricing_control)));
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
            consensus,
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
            // The legacy attach path serves the FX-options dialect (and still
            // content-detects an OIS request) — dedicated FI venues are stood up
            // through the managed `FixAdminService` registry, with their own kind.
            crate::config::fix_connections::AcceptorKind::Options,
            Arc::clone(&self.store),
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

    /// The activated consistency tier (ADR-0015), if one was booted (`Some` iff a
    /// `Strong`-tier book was configured at start). Exposes the leader/commit state for
    /// an ops probe or a test; `None` is the pure-`Local` default.
    #[must_use]
    pub fn consensus(&self) -> Option<&Arc<ConsensusHandle>> {
        self.consensus.as_ref()
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
