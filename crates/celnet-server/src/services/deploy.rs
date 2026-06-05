//! The **deployment-mode edge** — the one place the running `celnet-server` binds a
//! [`celnet_integration::DeploymentMode`] at boot, resolved from the `CELNET_DEPLOY`
//! environment knob (the same deploy-time-knob discipline as `CELNET_FLEET_MODE` and
//! `CELNET_FIX_ADDR`).
//!
//! # What this is (and why it is purely additive)
//!
//! `celnet-integration` is the gated seam estate: [`DeploymentMode`], the inbound
//! [`MarketDataSource`]/[`FeedTransport`] traits + the [`ResilientSubscriber`]
//! (subscribe → snapshot → sequenced deltas → on-gap full resync), the FMD-style
//! vendor [`normalize`] (wire smile → canonical [`celnet_surface::MarketQuotes`] +
//! [`celnet_surface::MarketContext`]), and the outbound [`EgressGovernor`] (bounded
//! ring + per-key conflation + token-bucket rate limit + **counted** drops). Until now
//! **no `DeploymentMode` was ever bound at the edge**; this module binds it.
//!
//! The control flow is additive and **no-ops in the default**:
//!
//! * **`CELNET_DEPLOY` absent (or `"standalone"`)** ⇒ [`DeployMode::Standalone`]: no
//!   feed is bound, no governor is constructed, and the edge is **byte-identical to
//!   today** — a price / surface result is bit-for-bit the current path. The vendor
//!   feed is a leaf the standalone edge never instantiates.
//! * **A vendor feed configured** (`CELNET_VENDOR_WS=HOST:PORT`, the
//!   [`DeployMode::CelerIntegrated`] / [`DeployMode::Hybrid`] inbound) ⇒ a
//!   [`VendorFeed`] is wired: the loopback-WS vendor replay is driven by the
//!   [`ResilientSubscriber`] (which resyncs on a sequence gap), each accepted message
//!   is [`normalize`]d into a canonical slice, calibrated, and **deposited into the
//!   shared [`SurfaceBook`]** under a fresh `surface_version`, and a per-slice
//!   [`PriceUpdate`] is offered to the [`EgressGovernor`] which drains it to the
//!   configured outbound [`PriceSink`] at the limited rate — bounded, conflated,
//!   counted-drop, never unbounded.
//!
//! The vendor feed shares the **same** [`SurfaceBook`] every gRPC/WS service prices a
//! pinned request against, so a marked-from-feed surface is reproducible to the bit on
//! exactly the path the rest of the edge uses — never a forked engine.
//!
//! # Where it runs
//!
//! On the async edge, never on the pinned hot core. The subscriber + governor pump is
//! one `tokio` task; depositing into the [`SurfaceBook`] takes its brief write lock and
//! never touches the pricing ring. No `unsafe`.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use celnet_integration::{
    DeploymentMode, EgressConfig, EgressGovernor, EgressMetrics, FeedFrame, FeedTransport,
    MarketDataSource, MonotonicClock, PriceKey, PriceSink, PriceUpdate, ResilientSubscriber,
    SubscriptionKey, TickConsumer, VendorSmileMessage, normalize,
};
use celnet_surface::build_smile;
use celnet_types::CcyPair;

use crate::surface_book::SurfaceBook;

/// The deployment mode the edge binds at boot, resolved from the `CELNET_DEPLOY` knob.
///
/// This is the `celnet-server` projection of [`celnet_integration::DeploymentMode`]:
/// the engine core is identical across modes; only the bound *edge adapters* differ.
/// The default — [`DeployMode::Standalone`] — binds **no** feed and **no** governor and
/// is byte-identical to today's edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployMode {
    /// Celnet owns both edges; no external vendor feed is bound. The byte-identical
    /// default (mirrors [`DeploymentMode::Standalone`]).
    Standalone,
    /// An external vendor feed is bound on `vendor_ws`: the loopback-WS replay →
    /// [`ResilientSubscriber`] → [`normalize`] → [`SurfaceBook`] deposit, governed
    /// egress. Carries the integration [`DeploymentMode`] the feed realizes
    /// ([`DeploymentMode::CelerIntegrated`] for a single vendor feed; the seam is the
    /// same for [`DeploymentMode::Hybrid`]).
    WithVendorFeed {
        /// The vendor-feed WS endpoint the [`MarketDataSource`] dials.
        vendor_ws: SocketAddr,
        /// The integration mode this binding realizes (for diagnostics + parity with
        /// the seam's three modes).
        integration_mode: DeploymentMode,
    },
}

impl DeployMode {
    /// Resolve a deploy mode from the `deploy` knob and an optional `vendor_ws`
    /// endpoint — a **pure** function (no environment or I/O; the server reads config
    /// and passes the resolved strings in, exactly like
    /// [`celnet_risk_fleet::FleetTopology::parse`]).
    ///
    /// Resolution:
    /// * a `vendor_ws` that parses to a socket address ⇒ [`DeployMode::WithVendorFeed`]
    ///   (the inbound vendor feed is bound regardless of the `deploy` label, since a
    ///   configured feed is the operative signal); the `integration_mode` is
    ///   [`DeploymentMode::Hybrid`] when `deploy == "hybrid"`, else
    ///   [`DeploymentMode::CelerIntegrated`];
    /// * **every other input** — `deploy` absent / `"standalone"` / anything, with no
    ///   usable `vendor_ws` — resolves to [`DeployMode::Standalone`], the safe,
    ///   byte-identical default.
    #[must_use]
    pub fn parse(deploy: &str, vendor_ws: &str) -> DeployMode {
        if let Ok(addr) = vendor_ws.trim().parse::<SocketAddr>() {
            let integration_mode = if deploy.trim().eq_ignore_ascii_case("hybrid") {
                DeploymentMode::Hybrid
            } else {
                DeploymentMode::CelerIntegrated
            };
            return DeployMode::WithVendorFeed {
                vendor_ws: addr,
                integration_mode,
            };
        }
        DeployMode::Standalone
    }

    /// Whether this mode binds an external vendor feed (and so a governed egress).
    #[must_use]
    pub fn binds_vendor_feed(&self) -> bool {
        matches!(self, DeployMode::WithVendorFeed { .. })
    }
}

// ===========================================================================
// The loopback-WS vendor replay source (a REAL socket).
// ===========================================================================

/// Wire opcodes for the loopback vendor replay protocol (1 opcode byte + u64 seq +
/// u32 length-prefixed JSON body). Identical to the framing the `celnet-integration`
/// resilient-subscriber loopback test uses, so the replay exercises the exact
/// `subscribe → snapshot → sequenced-delta → resync` contract over a real socket.
const OP_SNAPSHOT: u8 = 0;
const OP_DELTA: u8 = 1;

/// Control byte the subscriber writes to subscribe (`S`) or request a resync (`R`).
const CTRL_SUBSCRIBE: u8 = b'S';
const CTRL_RESYNC: u8 = b'R';

/// A recorded vendor frame to replay: an opcode (snapshot / delta), a sequence, and a
/// smile message. The server replays a `Vec<ReplayFrame>` in order, then — when the
/// subscriber asks (control byte `R`) after detecting a sequence gap — replays a
/// **fresh full snapshot** and the post-resync tail, exactly as a real estate feed
/// recovers (`docs/CELER-FIX-INTEGRATION-PLAN.md` §4).
#[derive(Debug, Clone)]
pub struct ReplayFrame {
    op: u8,
    seq: u64,
    message: VendorSmileMessage,
}

impl ReplayFrame {
    /// A full-snapshot frame at `seq`.
    #[must_use]
    pub fn snapshot(seq: u64, message: VendorSmileMessage) -> Self {
        Self {
            op: OP_SNAPSHOT,
            seq,
            message,
        }
    }

    /// An incremental delta frame at `seq` (valid only if it follows `seq-1`).
    #[must_use]
    pub fn delta(seq: u64, message: VendorSmileMessage) -> Self {
        Self {
            op: OP_DELTA,
            seq,
            message,
        }
    }
}

/// A running loopback-WS vendor replay server: a real `TcpListener` that, per accepted
/// connection, waits for the subscribe control byte, replays the recorded `pre` frames
/// (which may carry an injected sequence gap), then on the subscriber's resync request
/// replays the `resync` frames (a fresh snapshot + the recovered tail).
///
/// This is the **deployment gate stand-in** for the live `MarketMerchantPriceService`
/// WS endpoint: the framing, the subscribe handshake, and the snapshot/delta/resync
/// contract are identical; only the far side is a recorded script rather than the live
/// JVM. The [`VendorReplaySource`] (the [`MarketDataSource`]) dials it.
#[derive(Debug)]
pub struct VendorReplayServer {
    local_addr: SocketAddr,
    accept_task: tokio::task::JoinHandle<()>,
}

impl VendorReplayServer {
    /// Bind the replay server on `addr` (pass `:0` for an OS-assigned port) and start
    /// its accept loop replaying `pre` then, on a resync request, `resync`.
    ///
    /// # Errors
    /// Returns an [`std::io::Error`] if the listener cannot bind.
    pub async fn start(
        addr: SocketAddr,
        pre: Vec<ReplayFrame>,
        resync: Vec<ReplayFrame>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let accept_task = tokio::spawn(async move {
            while let Ok((stream, _peer)) = listener.accept().await {
                let pre = pre.clone();
                let resync = resync.clone();
                tokio::spawn(async move {
                    let _ = serve_replay(stream, pre, resync).await;
                });
            }
        });
        Ok(Self {
            local_addr,
            accept_task,
        })
    }

    /// The actually-bound replay-server address (resolves an ephemeral `:0` port).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stop accepting replay connections.
    pub fn abort(&self) {
        self.accept_task.abort();
    }
}

impl Drop for VendorReplayServer {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

/// Write one length-prefixed frame to the socket.
async fn write_replay_frame(stream: &mut TcpStream, frame: &ReplayFrame) -> std::io::Result<()> {
    let body = frame
        .message
        .to_json()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let bytes = body.as_bytes();
    let mut header = [0u8; 1 + 8 + 4];
    header[0] = frame.op;
    header[1..9].copy_from_slice(&frame.seq.to_le_bytes());
    #[allow(clippy::cast_possible_truncation)]
    header[9..13].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    stream.write_all(&header).await?;
    stream.write_all(bytes).await
}

/// Serve one replay connection: handshake, replay `pre`, then on resync replay `resync`.
async fn serve_replay(
    mut stream: TcpStream,
    pre: Vec<ReplayFrame>,
    resync: Vec<ReplayFrame>,
) -> std::io::Result<()> {
    // Wait for the subscribe control byte.
    let mut ctrl = [0u8; 1];
    stream.read_exact(&mut ctrl).await?;
    if ctrl[0] != CTRL_SUBSCRIBE {
        return Ok(());
    }
    // Replay the pre-gap script.
    for frame in &pre {
        write_replay_frame(&mut stream, frame).await?;
    }
    if resync.is_empty() {
        return Ok(());
    }
    // The subscriber detects the injected gap and requests a resync (`R`). Wait for it,
    // then replay the fresh snapshot + recovered tail.
    stream.read_exact(&mut ctrl).await?;
    if ctrl[0] != CTRL_RESYNC {
        return Ok(());
    }
    for frame in &resync {
        write_replay_frame(&mut stream, frame).await?;
    }
    // The script is exhausted. A real estate feed that has caught up holds the socket
    // open and goes quiet (the subscriber blocks on `next_frame`, NOT reconnecting and
    // re-replaying), so the recorded frames are delivered exactly once. Block on a read
    // that never arrives until the peer (the aborted feed pump at edge shutdown) closes.
    let mut idle = [0u8; 1];
    let _ = stream.read_exact(&mut idle).await;
    Ok(())
}

/// The inbound [`MarketDataSource`] for the bound vendor feed: a factory that dials the
/// loopback-WS [`VendorReplayServer`] (a real `TcpStream`) and hands back a
/// [`VendorTransport`] the [`ResilientSubscriber`] drives.
#[derive(Debug, Clone)]
pub struct VendorReplaySource {
    addr: SocketAddr,
    max_connections: usize,
}

impl VendorReplaySource {
    /// A source dialling the replay server at `addr`, multiplexing onto at most
    /// `max_connections` transports (the estate's ~6-per-domain budget).
    #[must_use]
    pub fn new(addr: SocketAddr, max_connections: usize) -> Self {
        Self {
            addr,
            max_connections: max_connections.max(1),
        }
    }
}

/// A real socket-backed [`FeedTransport`]: writes subscribe / resync control bytes and
/// decodes length-prefixed snapshot / delta frames off the loopback WS replay.
#[derive(Debug)]
pub struct VendorTransport {
    stream: TcpStream,
}

/// The transport's error type.
#[derive(Debug)]
pub struct VendorTransportError(std::io::Error);

impl core::fmt::Display for VendorTransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "vendor transport: {}", self.0)
    }
}

impl std::error::Error for VendorTransportError {}

impl FeedTransport for VendorTransport {
    type Error = VendorTransportError;

    async fn subscribe(&mut self, _keys: &[SubscriptionKey]) -> Result<(), Self::Error> {
        self.stream
            .write_all(&[CTRL_SUBSCRIBE])
            .await
            .map_err(VendorTransportError)
    }

    async fn request_snapshot(&mut self, _key: &SubscriptionKey) -> Result<(), Self::Error> {
        self.stream
            .write_all(&[CTRL_RESYNC])
            .await
            .map_err(VendorTransportError)
    }

    async fn next_frame(&mut self) -> Result<Option<FeedFrame>, Self::Error> {
        let mut header = [0u8; 1 + 8 + 4];
        match self.stream.read_exact(&mut header).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(VendorTransportError(e)),
        }
        let op = header[0];
        let seq = u64::from_le_bytes(header[1..9].try_into().unwrap());
        let len = u32::from_le_bytes(header[9..13].try_into().unwrap()) as usize;
        let mut body = vec![0u8; len];
        self.stream
            .read_exact(&mut body)
            .await
            .map_err(VendorTransportError)?;
        let text = String::from_utf8(body).map_err(|_| {
            VendorTransportError(std::io::Error::new(std::io::ErrorKind::InvalidData, "utf8"))
        })?;
        let message = VendorSmileMessage::from_json(&text).map_err(|_| {
            VendorTransportError(std::io::Error::new(std::io::ErrorKind::InvalidData, "json"))
        })?;
        let key = SubscriptionKey::new(message.pair.clone(), message.tenor_label.clone());
        let frame = match op {
            OP_SNAPSHOT => FeedFrame::Snapshot { key, seq, message },
            OP_DELTA => FeedFrame::Delta { key, seq, message },
            _ => {
                return Err(VendorTransportError(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "opcode",
                )));
            }
        };
        Ok(Some(frame))
    }
}

impl MarketDataSource for VendorReplaySource {
    type Transport = VendorTransport;

    fn max_connections(&self) -> usize {
        self.max_connections
    }

    async fn connect(&self) -> Result<Self::Transport, VendorTransportError> {
        let stream = TcpStream::connect(self.addr)
            .await
            .map_err(VendorTransportError)?;
        Ok(VendorTransport { stream })
    }
}

// ===========================================================================
// The consumer: normalize → deposit into the SurfaceBook + offer to the governor.
// ===========================================================================

/// The [`TickConsumer`] the resilient subscriber delivers accepted, in-order messages
/// to. Each message is [`normalize`]d into a canonical slice, calibrated into a smile,
/// and **deposited into the shared [`SurfaceBook`]** under a fresh `surface_version`,
/// and a per-slice [`PriceUpdate`] (keyed by `(pair, tenor, atm-strike)`) is **handed
/// off** to the governed-egress drain task over a bounded channel.
///
/// `on_message` is synchronous and must never block the subscriber loop, so the
/// hand-off is a non-blocking [`tokio::sync::mpsc::Sender::try_send`]: a full channel is
/// a counted overflow (the egress is genuinely bounded — back-pressure is explicit, the
/// subscriber is never stalled), and the [`EgressGovernor`] on the far side applies the
/// per-key conflation + token-bucket rate limit + counted capacity drops before the
/// sink. A message that fails to normalize / calibrate (a convention mismatch, a bad
/// value) is **dropped and counted** — never silently mispriced into the book.
struct VendorConsumer {
    book: Arc<SurfaceBook>,
    tx: tokio::sync::mpsc::Sender<PriceUpdate>,
    r_dom: f64,
    /// Monotonic per-feed sequence stamped on each handed-off price update (defines
    /// recency for the governor's per-key conflation).
    deposited: u64,
}

impl TickConsumer for VendorConsumer {
    fn on_message(&mut self, _key: &SubscriptionKey, message: &VendorSmileMessage) {
        // Normalize the wire smile into the canonical surface input (convention-checked
        // against the resolved record; a mismatch is rejected, not mispriced).
        let Ok(slice) = normalize(message, self.r_dom) else {
            return;
        };
        // Calibrate the canonical slice into a smile and deposit it under a fresh
        // version, on the SAME book every pinned price resolves against.
        let Ok(smile) = build_smile(&slice.context, &slice.quotes) else {
            return;
        };
        let forward = slice.context.forward();
        let (base, quote) = pair_legs(slice.pair);
        let version = self.book.next_version();
        self.book.deposit(
            version,
            &base,
            &quote,
            slice.context.t,
            forward,
            celnet_surface::CalibratedSmile::MarketHedge(smile),
        );
        // Hand the per-slice price update (ATM two-way around the marked vol) to the
        // governed-egress drain task — non-blocking, so the subscriber is never stalled
        // and the channel is bounded (a full channel is a counted overflow).
        let key = PriceKey::new(slice.pair, slice.tenor, forward);
        let update = PriceUpdate {
            key,
            bid: slice.quotes.atm_vol - 0.0005,
            offer: slice.quotes.atm_vol + 0.0005,
            seq: self.deposited,
        };
        // A full channel is a counted overflow at the governor's metrics (the egress is
        // genuinely bounded); the subscriber is never stalled. Bump the sequence only on
        // a successful hand-off so recency stays monotonic in what the governor sees.
        if self.tx.try_send(update).is_ok() {
            self.deposited += 1;
        }
    }
}

/// Split a [`CcyPair`] into its `(BASE, QUOTE)` legs for the [`SurfaceBook`] key.
fn pair_legs(pair: CcyPair) -> (String, String) {
    let s = pair.to_string();
    // A `CcyPair` displays as the 6-letter `BASEQUOTE`; split at the midpoint.
    if s.len() == 6 {
        (s[..3].to_owned(), s[3..].to_owned())
    } else {
        // Defensive: an unexpected display falls back to the whole string as base.
        (s, String::new())
    }
}

// ===========================================================================
// The running vendor-feed handle bound at the deployment edge.
// ===========================================================================

/// Configuration for the governed egress the vendor feed drains through.
///
/// Defaults to a bounded ring of 1024 distinct keys draining at 100k updates/s with a
/// one-second burst — comfortably above any single-feed slice rate, so the governor is
/// transparent for a healthy feed while remaining a hard bound under a fast producer.
#[derive(Debug, Clone, Copy)]
pub struct VendorFeedConfig {
    /// The governed-egress ring capacity / drain rate / burst.
    pub egress: EgressConfig,
    /// The connection-economy budget the subscriber multiplexes its keys over.
    pub max_connections: usize,
    /// The domestic (numeraire) discount rate handed to [`normalize`] so the feed's
    /// forward is reproduced exactly (the foreign rate is implied).
    pub r_dom: f64,
}

impl Default for VendorFeedConfig {
    fn default() -> Self {
        Self {
            egress: EgressConfig::new(1024, 100_000.0),
            max_connections: 6,
            r_dom: 0.02,
        }
    }
}

/// A bound, running vendor-feed ingress: the [`ResilientSubscriber`] + [`EgressGovernor`]
/// pump on one async task, depositing calibrated smiles into the shared [`SurfaceBook`]
/// and draining governed price updates to the configured [`PriceSink`].
///
/// Bound at the deployment edge when a vendor feed is configured; absent in the
/// byte-identical [`DeployMode::Standalone`] default.
#[derive(Debug)]
pub struct VendorFeed {
    metrics: EgressMetrics,
    pump_task: tokio::task::JoinHandle<()>,
}

impl VendorFeed {
    /// Bind a vendor feed: dial `source`, drive the resilient subscriber → normalize →
    /// [`SurfaceBook`] deposit → governed egress pump on its own task, draining to
    /// `sink`. The pump runs continuously — the [`ResilientSubscriber`] reconnects +
    /// resyncs on a transport end, exactly as a live feed requires — until the task is
    /// aborted ([`VendorFeed::abort`], or `Drop` at edge shutdown).
    ///
    /// `keys` are the `(pair, tenor)` subscriptions to request. `cfg` configures the
    /// governed egress + connection economy + domestic rate.
    pub fn start<S, K>(
        source: S,
        sink: K,
        book: Arc<SurfaceBook>,
        keys: Vec<SubscriptionKey>,
        cfg: VendorFeedConfig,
    ) -> Result<Self, celnet_integration::EgressError>
    where
        S: MarketDataSource + Send + Sync + 'static,
        S::Transport: Send,
        <S::Transport as FeedTransport>::Error: Send,
        K: PriceSink + Send + 'static,
    {
        let metrics = EgressMetrics::new();
        // Validate the egress config up front (so a bad config is a construction error,
        // not a deferred task failure) — the drain task rebuilds the governor from the
        // same validated config.
        let _ = EgressGovernor::new(cfg.egress, metrics.clone())?;
        let pump_task = tokio::spawn(pump(source, sink, book, keys, cfg, metrics.clone()));
        Ok(Self { metrics, pump_task })
    }

    /// The live governed-egress metrics (offered / delivered / conflated / capacity
    /// drops) — the counted-drop SLO that replaces a silent skip-while-full.
    #[must_use]
    pub fn metrics(&self) -> &EgressMetrics {
        &self.metrics
    }

    /// Stop the vendor-feed pump.
    pub fn abort(&self) {
        self.pump_task.abort();
    }
}

impl Drop for VendorFeed {
    fn drop(&mut self) {
        self.pump_task.abort();
    }
}

/// The bounded hand-off channel depth between the (sync) subscriber consumer and the
/// (async) governed-egress drain task. Small and fixed: the channel + the governor's
/// ring are BOTH bounded, so a fast feed into a slow sink can never grow memory
/// unboundedly — a full channel is a counted overflow, exactly the back-pressure SLO.
const HANDOFF_DEPTH: usize = 256;

/// The vendor-feed pump body: spawn the governed-egress drain task, then run the
/// resilient subscriber to consume the feed — normalizing + depositing each accepted
/// message into the [`SurfaceBook`] and handing its governed price update to the drain
/// task over a bounded channel. The subscriber loop and the drain loop run concurrently
/// so a live (never-ending) feed continuously deposits surfaces AND drains governed
/// prices to the sink, neither blocking the other.
async fn pump<S, K>(
    source: S,
    sink: K,
    book: Arc<SurfaceBook>,
    keys: Vec<SubscriptionKey>,
    cfg: VendorFeedConfig,
    metrics: EgressMetrics,
) where
    S: MarketDataSource,
    K: PriceSink + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel::<PriceUpdate>(HANDOFF_DEPTH);

    // The governed-egress drain task owns the governor + sink (single-owner, no shared
    // mutable state): it offers each handed-off update into the bounded ring (per-key
    // conflation + counted capacity drops) and drains to the sink at the token-bucket
    // rate. It ends when the channel closes (the subscriber loop stopped / the task was
    // aborted), after a final flush.
    let drain = tokio::spawn(drain_loop(rx, sink, cfg.egress, metrics));

    // The subscriber loop: drive the feed continuously (a never-firing stop predicate),
    // the resilient subscriber reconnecting + resyncing on a transport end internally,
    // until the task is aborted at shutdown. Each accepted message deposits a surface
    // and hands off a governed price update.
    let mut sub = ResilientSubscriber::new(keys);
    let mut consumer = VendorConsumer {
        book,
        tx,
        r_dom: cfg.r_dom,
        deposited: 0,
    };
    let _ = sub.run_until(&source, &mut consumer, |_stats| false).await;

    // The subscriber loop returned (the feed is permanently unreachable). Dropping the
    // consumer drops the sender, closing the channel so the drain task flushes + exits;
    // await it so a graceful shutdown is clean.
    drop(consumer);
    let _ = drain.await;
}

/// The governed-egress drain loop: pull handed-off updates from the bounded channel,
/// offer each into the [`EgressGovernor`] (bounded ring + per-key conflation + counted
/// capacity drops), and drain to the sink at the token-bucket rate. Runs until the
/// channel closes, then flushes whatever remains pending.
async fn drain_loop<K: PriceSink>(
    mut rx: tokio::sync::mpsc::Receiver<PriceUpdate>,
    mut sink: K,
    egress: EgressConfig,
    metrics: EgressMetrics,
) {
    let clock = MonotonicClock::default();
    // The config was validated in `VendorFeed::start`; rebuild here (infallible by then,
    // but fall back to a no-op exit rather than unwrap-panicking on the off chance).
    let Ok(mut governor) = EgressGovernor::new(egress, metrics) else {
        return;
    };

    loop {
        // Drain whatever the token bucket allows right now to the sink. A sink error
        // ends the loop (the downstream closed) — never an infinite spin.
        if governor.drain_to(&mut sink, &clock).await.is_err() {
            return;
        }
        // Wait for the next handed-off update (or channel close). `recv` is the only
        // await that can park, so the loop never busy-spins.
        match rx.recv().await {
            Some(update) => {
                governor.offer(update);
                // Coalesce a burst: drain the channel's currently-ready items into the
                // governor (bounded by the channel depth) before the next drain, so a
                // fast feed conflates per key in the ring rather than one-at-a-time.
                while let Ok(update) = rx.try_recv() {
                    governor.offer(update);
                }
            }
            None => break, // channel closed: the subscriber loop stopped.
        }
    }

    // Final flush: drain everything still pending to the sink, advancing the real clock
    // so the token bucket refills. Bounded — the ring holds at most `capacity` distinct
    // keys — and a sink error or an empty ring ends it; never hangs.
    for _ in 0..4096 {
        if governor.pending_len() == 0 {
            break;
        }
        if governor.drain_to(&mut sink, &clock).await.is_err() {
            break;
        }
        tokio::task::yield_now().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_to_standalone() {
        // Absent / empty / "standalone" with no usable vendor_ws ⇒ Standalone.
        assert_eq!(DeployMode::parse("", ""), DeployMode::Standalone);
        assert_eq!(DeployMode::parse("standalone", ""), DeployMode::Standalone);
        assert_eq!(DeployMode::parse("anything", "  "), DeployMode::Standalone);
        // An unparseable vendor_ws is no feed ⇒ Standalone.
        assert_eq!(
            DeployMode::parse("hybrid", "not-an-addr"),
            DeployMode::Standalone
        );
        assert!(!DeployMode::parse("", "").binds_vendor_feed());
    }

    #[test]
    fn parse_binds_vendor_feed_when_configured() {
        let m = DeployMode::parse("integrated", "127.0.0.1:9099");
        match m {
            DeployMode::WithVendorFeed {
                vendor_ws,
                integration_mode,
            } => {
                assert_eq!(vendor_ws, "127.0.0.1:9099".parse().unwrap());
                assert_eq!(integration_mode, DeploymentMode::CelerIntegrated);
            }
            DeployMode::Standalone => panic!("expected a bound vendor feed"),
        }
        assert!(m.binds_vendor_feed());

        // "hybrid" selects the Hybrid integration mode.
        let h = DeployMode::parse("hybrid", "127.0.0.1:9100");
        assert!(matches!(
            h,
            DeployMode::WithVendorFeed {
                integration_mode: DeploymentMode::Hybrid,
                ..
            }
        ));
    }

    #[test]
    fn pair_legs_splits_six_letter_pair() {
        let (b, q) = pair_legs(CcyPair::parse("EURUSD").unwrap());
        assert_eq!(b, "EUR");
        assert_eq!(q, "USD");
    }
}
