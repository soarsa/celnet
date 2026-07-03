//! The multiplexed request-for-stream (RFS) session: ONE bidirectional gRPC
//! connection carrying many typed [`Subscription`]s plus click-to-trade execution,
//! with automatic gap detection, server-assisted resync, and reconnect handled
//! inside the SDK.
//!
//! # One session, many subscriptions
//!
//! A caller opens a [`StreamSession`] (via [`crate::Client::open_session`]) and then
//! [`StreamSession::subscribe`]s any number of instruments over it. Every
//! subscription rides the *same* HTTP/2 stream — a blotter watching hundreds of
//! structures uses a single connection, not one gRPC stream per line — and the
//! session's single background driver demultiplexes the server's
//! snapshot/update/heartbeat/stream-end/executed/stream-reject frames by their
//! [`celnet_proto::SubscriptionId`] to the owning [`Subscription`]. Each
//! subscription is itself a typed async [`Stream`] of [`StreamEvent`]s; the caller
//! never touches the wire `oneof`, the per-subscription sequence numbers, the
//! control protocol, or the click-to-trade tokens.
//!
//! The session driver:
//!
//! * sends a [`celnet_proto::Subscribe`] per [`StreamSession::subscribe`] and
//!   surfaces that subscription's first [`celnet_proto::Snapshot`] as
//!   [`StreamEvent::Snapshot`], establishing its baseline sequence;
//! * tracks each subscription's monotonic sequence and surfaces every in-order
//!   [`celnet_proto::Update`] as [`StreamEvent::Tick`];
//! * **detects a per-subscription gap** (a received sequence beyond `last + 1`) and
//!   automatically sends a [`celnet_proto::Resync`] for *that* subscription with its
//!   last good sequence, emitting [`StreamEvent::GapDetected`] then resuming from the
//!   replayed messages / fresh snapshot;
//! * on a `LAGGED` [`celnet_proto::StreamEnd`] for a subscription automatically
//!   resyncs it on the same session, emitting a distinct [`StreamEvent::Lagged`]
//!   (a server-side drop, kept separate from a client-detected gap), and on a
//!   `DRAINING` end (a blue-green cutover, which drains the whole connection)
//!   transparently re-dials a fresh session and **re-subscribes every live
//!   subscription**, emitting [`StreamEvent::Reconnected`] on each;
//! * surfaces [`celnet_proto::Heartbeat`]s as [`StreamEvent::Heartbeat`];
//! * routes a click-to-trade [`celnet_proto::Executed`] / [`celnet_proto::StreamReject`]
//!   back to the [`Subscription::execute`] call that issued the `Execute`, by the
//!   per-execute correlation id the SDK mints and tracks.
//!
//! # One control relay, transparent reconnect
//!
//! Every subscription handle and every [`Subscription::execute`] sends control
//! frames (subscribe / resync / execute) on the session's single shared control
//! channel, whose receiver the **driver** owns. The driver forwards each frame to
//! the *currently live* outbound stream. On a `DRAINING` cutover the driver re-dials
//! a fresh stream and swaps the live outbound sender — so a caller's in-flight handle
//! keeps working across the reconnect with no re-wiring on the caller's side.
//!
//! # Click-to-trade off the stream (token handled for the caller)
//!
//! A streamed [`StreamLine`] carries the server's short-lived [`TradableLine`]
//! tokens (one to SELL at the bid, one to BUY at the offer).
//! [`Subscription::execute`] takes the *line the trader clicked* and the side, finds
//! the matching token, mints an idempotency key + a correlation id, sends the
//! [`celnet_proto::Execute`] on the session control channel, and awaits the typed
//! [`ExecuteOutcome`] — the opaque `tradable_token` never appears in caller code. An
//! `Execute` whose token has aged past its validity window is rejected by the maker
//! (last-look), surfaced as [`ExecuteOutcome::Rejected`] with a typed
//! [`RejectReason`].

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use celnet_proto::stream_service_client::StreamServiceClient;
use celnet_proto::{
    ClientStreamMessage, Conventions as WireConventions, EntitlementPrincipal, Execute, Instrument,
    MarketSeriesSubscribe, Resync, ServerStreamMessage, StreamAuth, Subscribe, SubscriptionId,
    TradableToken, client_stream_message, server_stream_message, stream_end, stream_reject,
};
use celnet_types::{CcyPair, Greeks, Tenor};
use futures_util::{Stream, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tonic::transport::Channel;

use crate::error::{ClientError, ClientResult};
use crate::idempotency::KeyMinter;
use crate::series::{MarketSeries, Observable, SeriesEvent, SeriesState, decode_snapshot};
use crate::vocab::{Attribution, Conventions, InstrumentSpec, Side, TwoWay};

/// The depth of each per-subscription SDK→caller event channel. A caller
/// momentarily slower than its subscription buffers up to this many events before
/// exerting back-pressure on the session driver (which is itself fed by the gRPC
/// flow-controlled channel).
const EVENT_CHANNEL_DEPTH: usize = 1024;

/// The depth of the caller→driver control channel (subscribe / resync / execute) and
/// of the driver→stream outbound relay. One pair for the whole multiplexed session,
/// sized for many concurrent subscriptions' control + click-to-trade traffic.
const CONTROL_CHANNEL_DEPTH: usize = 256;

/// The credential the SDK presents to **authenticate the stream session** before
/// the first subscribe. Built by [`crate::Client::open_session`] from the client's
/// [`session_token`](crate::Client::with_session_token) and entitlement
/// [`principal`](crate::Client::with_principal), and rendered to the wire
/// [`StreamAuth`] the SDK sends as the very first control frame (and re-sends first
/// on a drain-cutover reconnect).
///
/// The principal defaults to the audited **explicit grant-all** every client asserts
/// (`risk::principal_or_grant_all`) — identical to the gated risk requests — so an
/// authenticated frame is always emitted and the stream is admitted on the
/// production deny-by-default edge ([`celnet_server::AccessMode::Enforce`]) without
/// the SDK ever relying on the server granting an absent caller. A real
/// `AuthService.Login`-issued `session_token` authenticates the stream as that user.
#[derive(Debug, Clone)]
pub(crate) struct SessionAuth {
    /// The `AuthService.Login`-issued bearer token, when the client attached one.
    /// Sent verbatim; the server validates it against its session registry.
    token: Option<String>,
    /// The asserted entitlement principal. `None` ⇒ the explicit grant-all default.
    principal: Option<crate::risk::Entitlements>,
}

impl SessionAuth {
    /// The credential carrying `token` (a real Login bearer, when set) and
    /// `principal` (the asserted entitlements, else the grant-all default).
    pub(crate) fn new(token: Option<String>, principal: Option<crate::risk::Entitlements>) -> Self {
        Self { token, principal }
    }

    /// Render the wire [`StreamAuth`]: the bearer token (if any) plus the asserted
    /// principal, defaulting to the explicit grant-all every client asserts so the
    /// frame always authenticates the session under `Enforce` — exactly as
    /// `crate::risk::principal_or_grant_all` does on the gated risk requests.
    fn to_wire(&self) -> StreamAuth {
        let principal: EntitlementPrincipal =
            crate::risk::principal_or_grant_all(self.principal.as_ref());
        StreamAuth {
            session_token: self.token.clone(),
            principal: Some(principal),
        }
    }

    /// The opening `Authenticate` control frame — sent FIRST on every session stream
    /// (eager open and reconnect) so the server pins the caller before any subscribe.
    fn frame(&self) -> ClientStreamMessage {
        ClientStreamMessage {
            message: Some(client_stream_message::Message::Authenticate(self.to_wire())),
        }
    }
}

/// One short-lived click-to-trade token stamped on a streamed line: the side it
/// books, the premium it books at, and the validity deadline after which it is
/// dead. The typed form of the wire [`TradableToken`] — the opaque token value is
/// retained so the SDK can present it on an [`Subscription::execute`], but a caller
/// never reads it directly.
#[derive(Debug, Clone, Copy)]
pub struct TradableLine {
    /// The side this token executes: [`Side::Buy`] lifts the offer, [`Side::Sell`]
    /// hits the bid.
    pub side: Side,
    /// The premium this token books (the bid for a SELL, the offer for a BUY), in
    /// the stream's premium-style units.
    pub premium: f64,
    /// Token validity deadline, nanoseconds since the Unix epoch (UTC). An execute
    /// after this instant is rejected as expired (last-look).
    pub valid_until_nanos: i64,
    /// The opaque server-minted token. Handled by the SDK on
    /// [`Subscription::execute`]; not for caller interpretation.
    pub(crate) token: u64,
}

/// A streamed snapshot or tick line: the two-way market, the Greek set, the vol,
/// the resolved strike, the click-to-trade tokens, and the surface version, all at
/// one sequence point. The typed payload shared by [`StreamEvent::Snapshot`] and
/// [`StreamEvent::Tick`].
#[derive(Debug, Clone)]
pub struct StreamLine {
    /// The per-subscription monotonic sequence number of this line.
    pub sequence: u64,
    /// The current two-way market.
    pub price: TwoWay,
    /// The full Greek set at this sequence point.
    pub greeks: Greeks,
    /// The instrument vol (absolute) at this sequence point.
    pub vol: f64,
    /// The marked-surface version these prices were computed against, if the
    /// subscription pinned one or the maker echoed the live mark. `None` ⇒ no
    /// version reported.
    pub surface_version: Option<u64>,
    /// The click-to-trade tokens for this line (one to SELL at the bid, one to BUY
    /// at the offer). A click on either side executes that side's stamped premium;
    /// pass the line and side to [`Subscription::execute`]. Empty ⇒ indicative-only.
    pub tradable: Vec<TradableLine>,
    /// Message time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

impl StreamLine {
    /// The live click-to-trade token for `side` on this line, if one is stamped.
    /// [`Subscription::execute`] uses this to present the right token for the click.
    #[must_use]
    pub fn token_for(&self, side: Side) -> Option<&TradableLine> {
        self.tradable.iter().find(|t| t.side == side)
    }
}

/// One event surfaced by a [`Subscription`] stream. The SDK collapses the wire
/// snapshot/update/heartbeat/stream-end protocol and the resync/reconnect machinery
/// into this small, caller-facing vocabulary.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// The baseline snapshot establishing (or re-establishing) the sequence — the
    /// state a caller applies whole before consuming ticks.
    Snapshot {
        /// The snapshot line (price + greeks + vol + tradable tokens at the
        /// snapshot sequence).
        line: StreamLine,
        /// The strike the subscription resolved to (for a delta/solve key).
        resolved_strike: f64,
        /// The conventions the streamed prices are expressed under.
        conventions: Conventions,
        /// Echo of the opening correlation id, if one was supplied — a blotter joins
        /// this line to the subscribe request that opened it.
        correlation_id: Option<u64>,
        /// The who's-trading attribution chain the server resolved for this streamed
        /// line, if any: who quoted it (the maker) and the requesting seat. Echoed
        /// from the opening `subscribe_attributed`, so a blotter's attribution
        /// column has its identity. Present iff the server attributed the line.
        attribution: Option<Attribution>,
    },
    /// An in-order sequenced delta advancing the subscription.
    Tick(StreamLine),
    /// A liveness heartbeat at the given sequence (no price change).
    Heartbeat {
        /// The per-subscription sequence the heartbeat mirrors.
        sequence: u64,
        /// Send time, nanoseconds since the Unix epoch (UTC).
        epoch_nanos: i64,
    },
    /// The SDK detected a sequence gap and is auto-resyncing; the next
    /// [`StreamEvent::Snapshot`] or replayed [`StreamEvent::Tick`]s re-establish a
    /// known-good baseline. Carries the gap boundary for observability.
    GapDetected {
        /// The last in-order sequence the SDK had applied before the gap.
        last_good: u64,
        /// The out-of-order sequence that revealed the gap.
        observed: u64,
    },
    /// The server dropped this subscription for lagging (a `LAGGED`
    /// [`celnet_proto::StreamEnd`]) and the SDK is auto-resyncing on the **same**
    /// session from `last_good`. Distinct from [`StreamEvent::GapDetected`]: no
    /// client-side sequence gap was observed — the server shed a slow consumer — so
    /// lag and gap telemetry stay separable.
    Lagged {
        /// The last in-order sequence the SDK had applied when it was dropped, and
        /// the sequence the resync requests replay from.
        last_good: u64,
    },
    /// The server drained for a blue-green cutover and the SDK transparently
    /// re-dialed a fresh session and re-subscribed *every* live subscription. A
    /// fresh [`StreamEvent::Snapshot`] follows on each subscription.
    Reconnected,
}

/// The typed outcome of a click-to-trade [`Subscription::execute`].
#[derive(Debug, Clone)]
pub enum ExecuteOutcome {
    /// The maker booked the click at the stamped premium.
    Booked(ClickExecution),
    /// The maker declined the click (last-look): the token expired, was unknown /
    /// forged, or had already been consumed.
    Rejected {
        /// Why the maker declined.
        reason: RejectReason,
    },
}

/// A booking confirmation for a successful click-to-trade — the typed form of the
/// wire [`celnet_proto::Executed`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClickExecution {
    /// The server-assigned booking id.
    pub execution_id: u64,
    /// The side actually traded ([`Side::Buy`] lifted the offer, [`Side::Sell`] hit
    /// the bid).
    pub side: Side,
    /// The premium booked (the clicked token's stamped premium), in the stream's
    /// premium-style units.
    pub traded_premium: f64,
    /// Booking time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

/// Why a click-to-trade [`Execute`] was declined — the typed form of the wire
/// [`celnet_proto::stream_reject::Reason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// The token's validity window had passed when the execute arrived (last-look).
    Expired,
    /// The token matched no live stamped line for this subscription (forged /
    /// stale / from a retired sequence).
    UnknownToken,
    /// The token was already consumed by a prior accepted execute.
    AlreadyConsumed,
}

impl RejectReason {
    fn from_wire(w: stream_reject::Reason) -> Self {
        match w {
            stream_reject::Reason::Expired => RejectReason::Expired,
            stream_reject::Reason::UnknownToken => RejectReason::UnknownToken,
            stream_reject::Reason::AlreadyConsumed => RejectReason::AlreadyConsumed,
        }
    }
}

/// The shared per-session execute-routing table: a pending click's correlation id →
/// the one-shot that delivers its [`ExecuteOutcome`]. The driver completes a pending
/// entry when the matching `Executed` / `StreamReject` arrives; the
/// [`Subscription::execute`] future awaits it.
type ExecuteWaiters = Arc<Mutex<HashMap<u64, oneshot::Sender<ClientResult<ExecuteOutcome>>>>>;

/// The driver's subscription registry, shared with [`StreamSession::subscribe`] and
/// the demultiplexing driver.
type Registry = Arc<Mutex<HashMap<u64, SubState>>>;

/// The driver's market-series registry, shared with
/// [`StreamSession::subscribe_series`] and the demultiplexing driver. Keyed by the
/// series' subscription id (same id space as price subscriptions, never colliding —
/// both mint from the session's one `next_sub_id`).
type SeriesRegistry = Arc<Mutex<HashMap<u64, SeriesState>>>;

/// A multiplexed RFS session: one bidirectional gRPC connection over which any
/// number of typed [`Subscription`]s and their click-to-trade executes are carried.
///
/// Construct via [`crate::Client::open_session`]. Dropping the session (and all its
/// subscriptions) tears the underlying gRPC stream down. The session is `Send` and
/// its [`StreamSession::subscribe`] can be called concurrently.
#[derive(Debug)]
pub struct StreamSession {
    inner: Arc<SessionInner>,
    _driver: tokio::task::JoinHandle<()>,
}

/// The session's shared state: the outbound control channel (held by every
/// subscription handle and by `execute`, drained by the driver relay), the
/// subscription registry, the per-execute waiter table, the idempotency-key minter,
/// and the monotonic id sources.
#[derive(Debug)]
struct SessionInner {
    control: mpsc::Sender<ClientStreamMessage>,
    registry: Registry,
    series: SeriesRegistry,
    waiters: ExecuteWaiters,
    keys: KeyMinter,
    next_sub_id: AtomicU64,
    next_correlation: AtomicU64,
}

impl StreamSession {
    /// Open a multiplexed session over `channel`. Dials the bidirectional stream and
    /// spawns the demultiplexing driver. No subscriptions exist yet; call
    /// [`StreamSession::subscribe`].
    ///
    /// # Errors
    ///
    /// [`ClientError`] if the session stream cannot be opened.
    pub(crate) async fn open(
        channel: Channel,
        keys: KeyMinter,
        auth: SessionAuth,
    ) -> ClientResult<Self> {
        // The caller→driver control channel: handles + execute send here; the driver
        // drains it and relays each frame to the live outbound stream.
        let (control_tx, control_rx) = mpsc::channel::<ClientStreamMessage>(CONTROL_CHANNEL_DEPTH);
        // The driver→stream outbound relay feeding the first stream's outbound half.
        let (relay_tx, relay_rx) = mpsc::channel::<ClientStreamMessage>(CONTROL_CHANNEL_DEPTH);

        let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
        let series: SeriesRegistry = Arc::new(Mutex::new(HashMap::new()));
        let waiters: ExecuteWaiters = Arc::new(Mutex::new(HashMap::new()));

        // Authenticate FIRST: push the `Authenticate` frame onto the outbound relay
        // before the stream is dialed, so it is the very head of the outbound
        // sequence tonic reads — the server pins the caller from this frame before
        // any subscribe / execute, which the production `Enforce` edge requires.
        // (The buffered relay is empty here, so this never blocks.)
        if relay_tx.send(auth.frame()).await.is_err() {
            return Err(ClientError::StreamClosed);
        }

        // Dial the first stream eagerly so a connection failure surfaces here, not on
        // the first subscribe.
        let inbound = open_session_stream(channel.clone(), relay_rx).await?;

        let driver = tokio::spawn(drive_session(DriverCtx {
            channel,
            control_rx,
            relay_tx,
            inbound,
            registry: Arc::clone(&registry),
            series: Arc::clone(&series),
            waiters: Arc::clone(&waiters),
            auth,
        }));

        let inner = Arc::new(SessionInner {
            control: control_tx,
            registry,
            series,
            waiters,
            keys,
            next_sub_id: AtomicU64::new(1),
            next_correlation: AtomicU64::new(1),
        });

        Ok(Self {
            inner,
            _driver: driver,
        })
    }

    /// Open a subscription on this session for `instrument`, returning a typed
    /// [`Subscription`] stream of [`StreamEvent`]s multiplexed over the session's one
    /// connection. The subscription id is minted per session.
    ///
    /// `correlation_id` (optional) is echoed on the subscription's snapshot so a
    /// blotter can join the opened line to its request; `surface_version` (optional)
    /// pins the stream to a specific marked surface (the version a `mark_surface`
    /// returned) for reproducible streamed prices.
    ///
    /// This is the unattributed form; use [`StreamSession::subscribe_attributed`] to
    /// declare the requesting book/seat so click-to-trade fills are attributable.
    ///
    /// # Errors
    ///
    /// [`ClientError::StreamClosed`] if the session's control channel has closed
    /// (the connection is gone).
    pub async fn subscribe(
        &self,
        instrument: InstrumentSpec,
        conventions: Conventions,
        correlation_id: Option<u64>,
        surface_version: Option<u64>,
    ) -> ClientResult<Subscription> {
        self.subscribe_inner(
            instrument,
            conventions,
            correlation_id,
            surface_version,
            None,
        )
        .await
    }

    /// Open a subscription (as [`StreamSession::subscribe`]) declaring the requesting
    /// book/seat in `attribution`. The server resolves the who's-trading chain
    /// (the maker that prices the line, the requesting seat once it trades) and
    /// echoes it on the subscription's snapshot and any click-to-trade fill, so a
    /// blotter's attribution column has its identity (API-first parity with the
    /// RFQ [`crate::Rfq::with_attribution`]).
    ///
    /// # Errors
    ///
    /// [`ClientError::StreamClosed`] if the session's control channel has closed.
    pub async fn subscribe_attributed(
        &self,
        instrument: InstrumentSpec,
        conventions: Conventions,
        correlation_id: Option<u64>,
        surface_version: Option<u64>,
        attribution: Attribution,
    ) -> ClientResult<Subscription> {
        self.subscribe_inner(
            instrument,
            conventions,
            correlation_id,
            surface_version,
            Some(attribution),
        )
        .await
    }

    async fn subscribe_inner(
        &self,
        instrument: InstrumentSpec,
        conventions: Conventions,
        correlation_id: Option<u64>,
        surface_version: Option<u64>,
        attribution: Option<Attribution>,
    ) -> ClientResult<Subscription> {
        let sub_id = self.inner.next_sub_id.fetch_add(1, Ordering::Relaxed);
        let wire_instrument = instrument.to_wire();
        let wire_conv = conventions.to_wire();
        let wire_attribution = attribution.as_ref().map(Attribution::to_wire);

        // The per-subscription SDK→caller event channel.
        let (event_tx, event_rx) = mpsc::channel::<ClientResult<StreamEvent>>(EVENT_CHANNEL_DEPTH);

        // Register this subscription with the driver (so demuxed frames + reconnect
        // re-subscribes can reach it) before sending the Subscribe, so no frame races
        // ahead of the registration.
        self.inner
            .registry
            .lock()
            .expect("registry poisoned")
            .insert(
                sub_id,
                SubState::new(
                    event_tx,
                    wire_instrument.clone(),
                    wire_conv,
                    correlation_id,
                    surface_version,
                    wire_attribution.clone(),
                ),
            );

        let subscribe = ClientStreamMessage {
            message: Some(client_stream_message::Message::Subscribe(Subscribe {
                subscription: Some(SubscriptionId { value: sub_id }),
                instrument: Some(wire_instrument),
                conventions: Some(wire_conv),
                throttle_nanos: 0,
                correlation_id,
                surface_version,
                // The requesting book/seat declared via `subscribe_attributed`;
                // absent ⇒ unattributed. The server resolves + echoes the chain.
                attribution: wire_attribution,
            })),
        };
        if self.inner.control.send(subscribe).await.is_err() {
            self.inner
                .registry
                .lock()
                .expect("registry poisoned")
                .remove(&sub_id);
            return Err(ClientError::StreamClosed);
        }

        Ok(Subscription {
            sub_id,
            rx: event_rx,
            inner: Arc::clone(&self.inner),
            conventions,
        })
    }

    /// Open a market-series subscription on this session: a typed async
    /// [`MarketSeries`] stream of one labelled observable (the TrendMode contract)
    /// over the session's *same* connection, in the same subscription-id space as a
    /// price subscription. The server seeds the series with an opening
    /// [`SeriesEvent::Snapshot`] (recent history + the observable's identity) then
    /// appends live [`SeriesEvent::Point`]s as its state ticks, conflated to
    /// `throttle_nanos` (a client hint; `0` = no throttling). `history_limit` bounds
    /// the opening snapshot's history (`0` ⇒ the server default window).
    ///
    /// `pair` names the currency pair; for a tenor-dependent observable (ATM vol /
    /// RR / BF / forward) pass the pillar `tenor`, and for a wing observable
    /// ([`Observable::RiskReversal`] / [`Observable::Butterfly`]) the wing delta
    /// rides on the [`Observable`] itself.
    ///
    /// # Errors
    ///
    /// [`ClientError::StreamClosed`] if the session's control channel has closed; a
    /// server-side validation failure (e.g. a degenerate wing) surfaces as a
    /// [`ClientError::Status`] event on the returned stream.
    pub async fn subscribe_series(
        &self,
        pair: CcyPair,
        observable: Observable,
        tenor: Option<Tenor>,
        throttle_nanos: u64,
        history_limit: u32,
    ) -> ClientResult<MarketSeries> {
        let sub_id = self.inner.next_sub_id.fetch_add(1, Ordering::Relaxed);

        // The per-series SDK→caller event channel.
        let (event_tx, event_rx) = mpsc::channel::<ClientResult<SeriesEvent>>(EVENT_CHANNEL_DEPTH);

        // Register before sending, so a snapshot/point can never race the
        // registration.
        self.inner
            .series
            .lock()
            .expect("series registry poisoned")
            .insert(sub_id, SeriesState { event_tx });

        let subscribe = ClientStreamMessage {
            message: Some(client_stream_message::Message::MarketSeriesSubscribe(
                MarketSeriesSubscribe {
                    subscription: Some(SubscriptionId { value: sub_id }),
                    underlying: Some(celnet_proto::Underlying::fx(celnet_proto::CcyPair::from(
                        pair,
                    ))),
                    observable: observable.wire_tag(),
                    tenor: tenor.map(celnet_proto::Tenor::from),
                    delta: observable.wing_delta(),
                    throttle_nanos,
                    history_limit,
                },
            )),
        };
        if self.inner.control.send(subscribe).await.is_err() {
            self.inner
                .series
                .lock()
                .expect("series registry poisoned")
                .remove(&sub_id);
            return Err(ClientError::StreamClosed);
        }

        Ok(MarketSeries {
            sub_id,
            rx: event_rx,
            control: self.inner.control.clone(),
            observable,
        })
    }
}

/// A live RFS subscription on a [`StreamSession`]: a typed async stream of
/// [`StreamEvent`]s for one instrument, with gap detection, resync, and reconnect
/// handled inside the SDK, and click-to-trade off the streamed lines.
///
/// Construct via [`StreamSession::subscribe`]. Dropping the subscription leaves the
/// session (and its other subscriptions) running; the driver stops routing to a
/// dropped subscription's closed channel.
#[derive(Debug)]
pub struct Subscription {
    sub_id: u64,
    rx: mpsc::Receiver<ClientResult<StreamEvent>>,
    inner: Arc<SessionInner>,
    conventions: Conventions,
}

impl Subscription {
    /// The per-session subscription id this handle is keyed on.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.sub_id
    }

    /// The conventions this subscription's streamed prices are expressed under.
    #[must_use]
    pub fn conventions(&self) -> Conventions {
        self.conventions
    }

    /// Await the next stream event, or `None` once the subscription has ended (a
    /// clean unsubscribe, an unrecoverable stream end, or a dropped session).
    ///
    /// # Errors
    ///
    /// Yields a [`ClientError`] event if a wire message could not be decoded or the
    /// server returned a status mid-stream.
    pub async fn next_event(&mut self) -> Option<ClientResult<StreamEvent>> {
        self.rx.recv().await
    }

    /// Click-to-trade: book *exactly* the streamed `line` on `side` by presenting the
    /// matching click-to-trade token the maker stamped on it — no separate RFQ
    /// round-trip. The SDK finds the token for the side, mints an idempotency key + a
    /// correlation id, sends the [`Execute`] on the session's shared control channel,
    /// and awaits the maker's typed [`ExecuteOutcome`]. The opaque token is handled
    /// for the caller and never appears in this API.
    ///
    /// A click on a line whose token has aged past its validity window returns
    /// [`ExecuteOutcome::Rejected`] with [`RejectReason::Expired`] (last-look),
    /// exactly as the RFQ accept deadline behaves.
    ///
    /// # Errors
    ///
    /// [`ClientError::MissingField`] if `line` carries no token for `side`
    /// (an indicative-only line), or [`ClientError::StreamClosed`] if the session's
    /// connection has closed before the outcome arrives.
    pub async fn execute(&self, line: &StreamLine, side: Side) -> ClientResult<ExecuteOutcome> {
        let token = line
            .token_for(side)
            .ok_or(ClientError::MissingField("StreamLine.tradable[side]"))?
            .token;
        let correlation_id = self.inner.next_correlation.fetch_add(1, Ordering::Relaxed);
        let idempotency_key = self.inner.keys.next_key();

        // Register the waiter BEFORE sending, so the driver can never complete the
        // outcome before the receiver exists.
        let (otx, orx) = oneshot::channel::<ClientResult<ExecuteOutcome>>();
        self.inner
            .waiters
            .lock()
            .expect("waiters mutex poisoned")
            .insert(correlation_id, otx);

        let execute = ClientStreamMessage {
            message: Some(client_stream_message::Message::Execute(Execute {
                subscription: Some(SubscriptionId { value: self.sub_id }),
                token,
                idempotency_key,
                correlation_id: Some(correlation_id),
            })),
        };
        if self.inner.control.send(execute).await.is_err() {
            self.inner
                .waiters
                .lock()
                .expect("waiters mutex poisoned")
                .remove(&correlation_id);
            return Err(ClientError::StreamClosed);
        }

        // Await the demuxed outcome. A delivered `Err` (the driver failed this
        // pending click because the session re-dialed or closed) surfaces promptly;
        // a *dropped* one-shot (the driver task itself ended) means the stream
        // closed.
        match orx.await {
            Ok(outcome) => outcome,
            Err(_) => Err(ClientError::StreamClosed),
        }
    }
}

impl Stream for Subscription {
    type Item = ClientResult<StreamEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

// ===========================================================================
// session driver — demultiplex server frames by SubscriptionId; route executes
// ===========================================================================

/// The per-subscription state the driver owns: the channel to the caller, the wire
/// instrument/conventions/correlation/surface-version for re-subscribe on a
/// drain-cutover reconnect, and the last in-order sequence applied (for gap
/// detection and resync).
#[derive(Debug)]
struct SubState {
    event_tx: mpsc::Sender<ClientResult<StreamEvent>>,
    instrument: Instrument,
    conv: WireConventions,
    correlation_id: Option<u64>,
    surface_version: Option<u64>,
    /// The requesting attribution to re-send verbatim on a drain-cutover
    /// re-subscribe, so the reconnected line keeps its who's-trading identity.
    attribution: Option<celnet_proto::AttributionRecord>,
    /// The last in-order sequence applied. 0 = no baseline yet.
    last_seq: u64,
}

impl SubState {
    fn new(
        event_tx: mpsc::Sender<ClientResult<StreamEvent>>,
        instrument: Instrument,
        conv: WireConventions,
        correlation_id: Option<u64>,
        surface_version: Option<u64>,
        attribution: Option<celnet_proto::AttributionRecord>,
    ) -> Self {
        Self {
            event_tx,
            instrument,
            conv,
            correlation_id,
            surface_version,
            attribution,
            last_seq: 0,
        }
    }
}

/// The context owned by the session driver task.
struct DriverCtx {
    channel: Channel,
    /// The caller→driver control channel (drained here).
    control_rx: mpsc::Receiver<ClientStreamMessage>,
    /// The driver→live-stream outbound relay. Swapped on a drain-cutover reconnect.
    relay_tx: mpsc::Sender<ClientStreamMessage>,
    inbound: tonic::Streaming<ServerStreamMessage>,
    registry: Registry,
    series: SeriesRegistry,
    waiters: ExecuteWaiters,
    /// The session credential, re-sent FIRST on a drain-cutover reconnect so the
    /// freshly-dialed (anonymous) stream is re-authenticated before re-subscribe.
    auth: SessionAuth,
}

/// Dial a fresh bidirectional session stream over `channel`, consuming `outbound_rx`
/// as its outbound half.
async fn open_session_stream(
    channel: Channel,
    outbound_rx: mpsc::Receiver<ClientStreamMessage>,
) -> ClientResult<tonic::Streaming<ServerStreamMessage>> {
    let mut client = StreamServiceClient::new(channel);
    let outbound = tokio_stream_from(outbound_rx);
    let response = client.stream_session(outbound).await?;
    Ok(response.into_inner())
}

/// Adapt a bounded mpsc receiver into the `Stream` of client messages tonic's
/// bidirectional RPC consumes as its outbound half.
fn tokio_stream_from(
    rx: mpsc::Receiver<ClientStreamMessage>,
) -> impl Stream<Item = ClientStreamMessage> {
    futures_util::stream::unfold(
        rx,
        |mut rx| async move { rx.recv().await.map(|msg| (msg, rx)) },
    )
}

/// The session driver: relay caller control frames to the live outbound stream, read
/// server frames and demultiplex them by [`SubscriptionId`] to the owning
/// subscription (gap-detect + resync), route click-to-trade outcomes to the waiting
/// `execute`, and on a drain-cutover re-dial the session + re-subscribe every live
/// subscription. Runs until the inbound stream closes or the caller drops the
/// session (control channel closed and no subscriptions remain reachable).
async fn drive_session(mut ctx: DriverCtx) {
    loop {
        tokio::select! {
            // ---- caller → live stream: relay control frames -----------------
            control = ctx.control_rx.recv() => {
                match control {
                    Some(frame) => {
                        // If the relay is full/closed mid-cutover, drop the frame; a
                        // resync/execute will be re-driven by the protocol or the
                        // caller surfaces StreamClosed.
                        let _ = ctx.relay_tx.send(frame).await;
                    }
                    None => return, // session dropped: every handle gone.
                }
            }
            // ---- live stream → caller: demultiplex server frames ------------
            next = ctx.inbound.next() => {
                match next {
                    Some(Ok(msg)) => {
                        let Some(payload) = msg.message else { continue; };
                        if let SessionFlow::Reconnect = handle_frame(
                            payload,
                            &ctx.registry,
                            &ctx.series,
                            &ctx.relay_tx,
                            &ctx.waiters,
                        )
                        .await
                            && !reconnect_session(&mut ctx).await
                        {
                            return;
                        }
                    }
                    Some(Err(status)) => {
                        broadcast_error(&ctx.registry, &status);
                        broadcast_series_error(&ctx.series, &status);
                        // Fail any in-flight click-to-trade so its `execute` future
                        // resolves with the transport status instead of hanging.
                        fail_all_waiters(&ctx.waiters, &ClientError::from(status));
                        return;
                    }
                    None => {
                        // Server closed the session: resolve any pending click
                        // promptly rather than leaving it to the one-shot drop.
                        fail_all_waiters(&ctx.waiters, &ClientError::StreamClosed);
                        return;
                    }
                }
            }
        }
    }
}

/// Control-flow signal from handling one server frame at the session level.
enum SessionFlow {
    /// Keep pumping the current session.
    Continue,
    /// The whole connection is draining (a subscription saw `DRAINING`); re-dial the
    /// session and re-subscribe every live subscription.
    Reconnect,
}

/// Handle one decoded server frame: route it to the owning subscription (or, for a
/// click-to-trade result, to the waiting `execute`).
async fn handle_frame(
    payload: server_stream_message::Message,
    registry: &Registry,
    series: &SeriesRegistry,
    relay: &mpsc::Sender<ClientStreamMessage>,
    waiters: &ExecuteWaiters,
) -> SessionFlow {
    match payload {
        server_stream_message::Message::Snapshot(s) => {
            let Some(sub_id) = s.subscription.map(|i| i.value) else {
                return SessionFlow::Continue;
            };
            let emit = {
                let mut reg = registry.lock().expect("registry poisoned");
                reg.get_mut(&sub_id).map(|sub| snapshot_emit(sub, s))
            };
            if let Some(emit) = emit {
                deliver(registry, sub_id, emit).await;
            }
            SessionFlow::Continue
        }
        server_stream_message::Message::Update(u) => {
            let Some(sub_id) = u.subscription.map(|i| i.value) else {
                return SessionFlow::Continue;
            };
            handle_update(registry, relay, sub_id, u).await;
            SessionFlow::Continue
        }
        server_stream_message::Message::Heartbeat(hb) => {
            let Some(sub_id) = hb.subscription.map(|i| i.value) else {
                return SessionFlow::Continue;
            };
            handle_heartbeat(registry, relay, sub_id, hb).await;
            SessionFlow::Continue
        }
        server_stream_message::Message::StreamEnd(end) => {
            let Some(sub_id) = end.subscription.map(|i| i.value) else {
                return SessionFlow::Continue;
            };
            handle_stream_end(registry, relay, sub_id, end.reason).await
        }
        server_stream_message::Message::Executed(e) => {
            complete_execute(
                waiters,
                e.correlation_id,
                ExecuteOutcome::Booked(ClickExecution {
                    execution_id: e.execution_id,
                    side: decode_side(e.side),
                    traded_premium: e.traded_premium,
                    epoch_nanos: e.epoch_nanos,
                }),
            );
            SessionFlow::Continue
        }
        server_stream_message::Message::StreamReject(r) => {
            let reason = stream_reject::Reason::try_from(r.reason)
                .map(RejectReason::from_wire)
                .unwrap_or(RejectReason::UnknownToken);
            complete_execute(
                waiters,
                r.correlation_id,
                ExecuteOutcome::Rejected { reason },
            );
            SessionFlow::Continue
        }
        // Market-series feed frames (the TrendMode time-series): route by the
        // series' SubscriptionId to the owning `MarketSeries` (a separate registry
        // from price subscriptions; ids never collide). An unsolicited frame (no
        // matching series) is dropped, not mis-routed.
        server_stream_message::Message::MarketSeriesSnapshot(s) => {
            let Some(sub_id) = s.subscription.map(|i| i.value) else {
                return SessionFlow::Continue;
            };
            deliver_series(series, sub_id, decode_snapshot(&s)).await;
            SessionFlow::Continue
        }
        server_stream_message::Message::MarketSeriesPoint(p) => {
            let Some(sub_id) = p.subscription.map(|i| i.value) else {
                return SessionFlow::Continue;
            };
            deliver_series(
                series,
                sub_id,
                Ok(SeriesEvent::Point(crate::series::SeriesPoint::from_wire(
                    &p,
                ))),
            )
            .await;
            SessionFlow::Continue
        }
        // Fixed-income streaming frames (RatesStreamSnapshot / RatesStreamUpdate)
        // are consumed by the dedicated FI-stream client surface (sdk-fi-stream). A
        // price/series session that never opened a rates line does not receive
        // these; an unsolicited FI frame is dropped, not mis-routed — never faked
        // into an FX price line.
        server_stream_message::Message::RatesStreamSnapshot(_)
        | server_stream_message::Message::RatesStreamUpdate(_) => SessionFlow::Continue,
    }
}

/// Send one event to a market series' caller channel. If the caller dropped the
/// series (channel closed), forget it so the driver stops routing to it.
async fn deliver_series(series: &SeriesRegistry, sub_id: u64, event: ClientResult<SeriesEvent>) {
    let tx = {
        let reg = series.lock().expect("series registry poisoned");
        reg.get(&sub_id).map(|s| s.event_tx.clone())
    };
    if let Some(tx) = tx
        && tx.send(event).await.is_err()
    {
        series
            .lock()
            .expect("series registry poisoned")
            .remove(&sub_id);
    }
}

/// Surface a session-level transport error to every live market series.
fn broadcast_series_error(series: &SeriesRegistry, status: &tonic::Status) {
    let reg = series.lock().expect("series registry poisoned");
    for s in reg.values() {
        let _ = s.event_tx.try_send(Err(ClientError::from(status.clone())));
    }
}

/// Compute the snapshot emit for a subscription, advancing its baseline sequence.
fn snapshot_emit(sub: &mut SubState, s: celnet_proto::Snapshot) -> ClientResult<StreamEvent> {
    let line = decode_line(
        s.sequence,
        &s.price,
        &s.greeks,
        s.vol,
        s.surface_version,
        &s.tradable,
        s.epoch_nanos,
    )?;
    let conventions = s
        .conventions
        .as_ref()
        .ok_or(ClientError::MissingField("Snapshot.conventions"))
        .and_then(Conventions::from_wire)?;
    let attribution = s
        .attribution
        .as_ref()
        .map(Attribution::from_wire)
        .transpose()?;
    sub.last_seq = s.sequence;
    Ok(StreamEvent::Snapshot {
        line,
        resolved_strike: s.resolved_strike,
        conventions,
        correlation_id: s.correlation_id,
        attribution,
    })
}

/// Decode a wire [`Side`] tag, defaulting an out-of-range tag to BUY (the maker only
/// ever stamps BUY/SELL on a token; TWO_WAY is never an executed side).
fn decode_side(tag: i32) -> Side {
    match celnet_proto::Side::try_from(tag) {
        Ok(s) => Side::from_wire(s),
        Err(_) => Side::Buy,
    }
}

/// Handle an [`Update`] for one subscription: gap-detect, resync, or emit the tick.
async fn handle_update(
    registry: &Registry,
    relay: &mpsc::Sender<ClientStreamMessage>,
    sub_id: u64,
    u: celnet_proto::Update,
) {
    enum Action {
        Gap { last_good: u64, observed: u64 },
        Stale,
        Tick(StreamLine),
        Decode(ClientError),
        Missing,
    }
    let action = {
        let mut reg = registry.lock().expect("registry poisoned");
        match reg.get_mut(&sub_id) {
            None => Action::Missing,
            Some(sub) => {
                if is_gap(sub.last_seq, u.sequence) {
                    Action::Gap {
                        last_good: sub.last_seq,
                        observed: u.sequence,
                    }
                } else if is_stale(sub.last_seq, u.sequence) {
                    Action::Stale
                } else {
                    match decode_line(
                        u.sequence,
                        &u.price,
                        &u.greeks,
                        u.vol,
                        u.surface_version,
                        &u.tradable,
                        u.epoch_nanos,
                    ) {
                        Ok(line) => {
                            sub.last_seq = u.sequence;
                            Action::Tick(line)
                        }
                        Err(e) => Action::Decode(e),
                    }
                }
            }
        }
    };

    match action {
        Action::Missing | Action::Stale => {}
        Action::Gap {
            last_good,
            observed,
        } => {
            deliver(
                registry,
                sub_id,
                Ok(StreamEvent::GapDetected {
                    last_good,
                    observed,
                }),
            )
            .await;
            let _ = send_resync(relay, sub_id, last_good).await;
        }
        Action::Tick(line) => deliver(registry, sub_id, Ok(StreamEvent::Tick(line))).await,
        Action::Decode(e) => deliver(registry, sub_id, Err(e)).await,
    }
}

/// Handle a [`Heartbeat`] for one subscription: resync if it reveals a gap, else
/// surface it.
async fn handle_heartbeat(
    registry: &Registry,
    relay: &mpsc::Sender<ClientStreamMessage>,
    sub_id: u64,
    hb: celnet_proto::Heartbeat,
) {
    let gap = {
        let reg = registry.lock().expect("registry poisoned");
        reg.get(&sub_id)
            .map(|sub| is_gap(sub.last_seq, hb.sequence).then_some(sub.last_seq))
    };
    match gap {
        None => {}
        Some(Some(last_good)) => {
            deliver(
                registry,
                sub_id,
                Ok(StreamEvent::GapDetected {
                    last_good,
                    observed: hb.sequence,
                }),
            )
            .await;
            let _ = send_resync(relay, sub_id, last_good).await;
        }
        Some(None) => {
            deliver(
                registry,
                sub_id,
                Ok(StreamEvent::Heartbeat {
                    sequence: hb.sequence,
                    epoch_nanos: hb.epoch_nanos,
                }),
            )
            .await;
        }
    }
}

/// Handle a [`celnet_proto::StreamEnd`] for one subscription, returning the
/// session-level flow (only a `DRAINING` end triggers a whole-session reconnect).
async fn handle_stream_end(
    registry: &Registry,
    relay: &mpsc::Sender<ClientStreamMessage>,
    sub_id: u64,
    reason: i32,
) -> SessionFlow {
    match stream_end::Reason::try_from(reason) {
        Ok(stream_end::Reason::Lagged) => {
            let last_good = {
                let reg = registry.lock().expect("registry poisoned");
                reg.get(&sub_id).map(|s| s.last_seq)
            };
            if let Some(last_good) = last_good {
                deliver(registry, sub_id, Ok(StreamEvent::Lagged { last_good })).await;
                let _ = send_resync(relay, sub_id, last_good).await;
            }
            SessionFlow::Continue
        }
        Ok(stream_end::Reason::Draining) => SessionFlow::Reconnect,
        Ok(stream_end::Reason::Unsubscribed | stream_end::Reason::Expired) | Err(_) => {
            // Clean teardown / expiry of this one subscription: drop its channel so
            // the caller's stream ends with `None`.
            registry.lock().expect("registry poisoned").remove(&sub_id);
            SessionFlow::Continue
        }
    }
}

/// Send one event to a subscription's caller channel. If the caller dropped the
/// subscription (channel closed), forget it so the driver stops routing to it.
async fn deliver(registry: &Registry, sub_id: u64, event: ClientResult<StreamEvent>) {
    // Clone the sender out from under the lock so the send await never holds it.
    let tx = {
        let reg = registry.lock().expect("registry poisoned");
        reg.get(&sub_id).map(|s| s.event_tx.clone())
    };
    if let Some(tx) = tx
        && tx.send(event).await.is_err()
    {
        registry.lock().expect("registry poisoned").remove(&sub_id);
    }
}

/// Surface a session-level transport error to every live subscription.
fn broadcast_error(registry: &Registry, status: &tonic::Status) {
    let reg = registry.lock().expect("registry poisoned");
    for sub in reg.values() {
        let _ = sub
            .event_tx
            .try_send(Err(ClientError::from(status.clone())));
    }
}

/// Complete a pending click-to-trade `execute` keyed by its correlation id with the
/// typed outcome. A missing waiter (no correlation, or the caller already dropped)
/// is a no-op.
fn complete_execute(
    waiters: &ExecuteWaiters,
    correlation_id: Option<u64>,
    outcome: ExecuteOutcome,
) {
    let Some(id) = correlation_id else {
        return;
    };
    let waiter = waiters.lock().expect("waiters mutex poisoned").remove(&id);
    if let Some(tx) = waiter {
        let _ = tx.send(Ok(outcome));
    }
}

/// Fail *every* pending click-to-trade waiter with `err`, draining the table. Called
/// when the session re-dials (a `DRAINING` blue-green cutover) or closes: an in-flight
/// click was bound to a token on the prior connection that can no longer be booked, so
/// each pending `execute` future is resolved promptly with a typed [`ClientError`]
/// instead of being left to hang forever. `err` is cloned per waiter via [`clone_err`].
fn fail_all_waiters(waiters: &ExecuteWaiters, err: &ClientError) {
    let pending: Vec<oneshot::Sender<ClientResult<ExecuteOutcome>>> = {
        let mut table = waiters.lock().expect("waiters mutex poisoned");
        table.drain().map(|(_, tx)| tx).collect()
    };
    for tx in pending {
        let _ = tx.send(Err(clone_err(err)));
    }
}

/// Send a `Resync(last_sequence)` for one subscription on the live outbound relay.
async fn send_resync(
    relay: &mpsc::Sender<ClientStreamMessage>,
    sub_id: u64,
    last_sequence: u64,
) -> Result<(), ()> {
    let msg = ClientStreamMessage {
        message: Some(client_stream_message::Message::Resync(Resync {
            subscription: Some(SubscriptionId { value: sub_id }),
            last_sequence,
        })),
    };
    relay.send(msg).await.map_err(|_| ())
}

/// Re-dial a fresh session stream after a `DRAINING` cutover and re-subscribe every
/// live subscription, emitting [`StreamEvent::Reconnected`] on each. Swaps the
/// driver's live outbound relay to the new stream so the caller's shared control
/// sender keeps reaching the live connection. Returns `false` if the re-dial failed
/// (the driver then ends and every subscription's stream closes).
async fn reconnect_session(ctx: &mut DriverCtx) -> bool {
    // Any click-to-trade in flight was bound to a token minted on the *prior*
    // session; that token dies with the connection, so its `Execute`/result can
    // never round-trip on the fresh stream. Fail every pending waiter promptly with
    // a typed `Reconnected` error before re-dialing — otherwise a caller awaiting an
    // execution would hang forever across the cutover. The caller re-clicks the
    // current line once the post-cutover snapshot arrives.
    fail_all_waiters(&ctx.waiters, &ClientError::Reconnected);

    let (relay_tx, relay_rx) = mpsc::channel::<ClientStreamMessage>(CONTROL_CHANNEL_DEPTH);
    let inbound = match open_session_stream(ctx.channel.clone(), relay_rx).await {
        Ok(s) => s,
        Err(e) => {
            let reg = ctx.registry.lock().expect("registry poisoned");
            for sub in reg.values() {
                let _ = sub.event_tx.try_send(Err(clone_err(&e)));
            }
            return false;
        }
    };
    ctx.inbound = inbound;
    ctx.relay_tx = relay_tx;

    // Re-authenticate FIRST on the fresh stream: the re-dialed session is anonymous
    // server-side, so the `Authenticate` frame must lead — before any re-subscribe —
    // exactly as on the eager open, or the production `Enforce` edge would reject the
    // re-subscribes `unauthenticated`.
    if ctx.relay_tx.send(ctx.auth.frame()).await.is_err() {
        return false;
    }

    // Re-subscribe every live subscription on the fresh stream (resetting its
    // baseline) and surface a Reconnected event on each.
    let resubs: Vec<(u64, ClientStreamMessage)> = {
        let mut reg = ctx.registry.lock().expect("registry poisoned");
        reg.iter_mut()
            .map(|(&id, sub)| {
                sub.last_seq = 0; // a fresh snapshot re-establishes the baseline.
                let msg = ClientStreamMessage {
                    message: Some(client_stream_message::Message::Subscribe(Subscribe {
                        subscription: Some(SubscriptionId { value: id }),
                        instrument: Some(sub.instrument.clone()),
                        conventions: Some(sub.conv),
                        throttle_nanos: 0,
                        correlation_id: sub.correlation_id,
                        surface_version: sub.surface_version,
                        attribution: sub.attribution.clone(),
                    })),
                };
                (id, msg)
            })
            .collect()
    };
    for (id, msg) in resubs {
        if ctx.relay_tx.send(msg).await.is_err() {
            return false;
        }
        deliver(&ctx.registry, id, Ok(StreamEvent::Reconnected)).await;
    }
    true
}

/// Clone a [`ClientError`] enough to broadcast it (transport/status errors clone via
/// their string form; structural variants clone directly).
fn clone_err(e: &ClientError) -> ClientError {
    match e {
        ClientError::StreamClosed => ClientError::StreamClosed,
        ClientError::Reconnected => ClientError::Reconnected,
        ClientError::MissingField(f) => ClientError::MissingField(f),
        ClientError::InvalidEndpoint(s) => ClientError::InvalidEndpoint(s.clone()),
        other => ClientError::Status(Box::new(tonic::Status::unavailable(other.to_string()))),
    }
}

/// Is `observed` beyond the next expected sequence given the `last_good` applied?
///
/// A gap exists once a baseline is established (`last_good != 0`) and the observed
/// sequence skips at least one number (`observed > last_good + 1`).
fn is_gap(last_good: u64, observed: u64) -> bool {
    last_good != 0 && observed > last_good + 1
}

/// Is `observed` a stale / duplicate sequence (≤ the last applied), to be dropped
/// idempotently? Only meaningful once a baseline is established.
fn is_stale(last_good: u64, observed: u64) -> bool {
    last_good != 0 && observed <= last_good
}

/// Decode a snapshot/update line's price + greeks + tradable tokens into a typed
/// [`StreamLine`].
fn decode_line(
    sequence: u64,
    price: &Option<celnet_proto::TwoWayPrice>,
    greeks: &Option<celnet_proto::Greeks>,
    vol: f64,
    surface_version: Option<u64>,
    tradable: &[TradableToken],
    epoch_nanos: i64,
) -> ClientResult<StreamLine> {
    let price = price
        .as_ref()
        .map(TwoWay::from_wire)
        .ok_or(ClientError::MissingField("StreamLine.price"))?;
    let greeks = greeks
        .as_ref()
        .map(crate::vocab::greeks_from_wire)
        .ok_or(ClientError::MissingField("StreamLine.greeks"))?;
    let tradable = tradable
        .iter()
        .map(|t| TradableLine {
            side: decode_side(t.side),
            premium: t.premium,
            valid_until_nanos: t.valid_until_nanos,
            token: t.token,
        })
        .collect();
    Ok(StreamLine {
        sequence,
        price,
        greeks,
        vol,
        surface_version,
        tradable,
        epoch_nanos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::{EntitlementScope, Entitlements, Scope as ClientScope};

    const SUB_ID: u64 = 7;

    /// The opening `Authenticate` frame carries the client's session token verbatim
    /// AND an explicit grant-all principal when none was asserted — so the server
    /// pins an authenticated caller and the production `Enforce` edge admits the
    /// session (parity with the gated risk requests' grant-all default).
    #[test]
    fn session_auth_frame_carries_token_and_defaults_to_grant_all() {
        let auth = SessionAuth::new(Some("login-bearer-xyz".to_owned()), None);
        match auth.frame().message.expect("a payload") {
            client_stream_message::Message::Authenticate(a) => {
                assert_eq!(
                    a.session_token.as_deref(),
                    Some("login-bearer-xyz"),
                    "the Login-issued token is sent verbatim"
                );
                let principal = a
                    .principal
                    .expect("an explicit principal is always asserted");
                assert!(
                    principal.grant_all,
                    "no asserted principal ⇒ the explicit grant-all default (Enforce-admitted)"
                );
                assert!(principal.grants.is_empty() && principal.denies.is_empty());
            }
            other => panic!("expected Authenticate, got {other:?}"),
        }
    }

    /// With NO session token, the frame is still emitted — an anonymous-token frame
    /// asserting the grant-all principal, which is exactly how the risk path is
    /// admitted under `Enforce`. A scoped principal rides through unchanged.
    #[test]
    fn session_auth_frame_without_token_asserts_the_scoped_principal() {
        let scoped = Entitlements::scoped().grant(EntitlementScope::covering(ClientScope::firm()));
        let auth = SessionAuth::new(None, Some(scoped));
        match auth.frame().message.expect("a payload") {
            client_stream_message::Message::Authenticate(a) => {
                assert!(a.session_token.is_none(), "no token ⇒ anonymous bearer");
                let principal = a.principal.expect("the asserted principal rides the frame");
                assert!(
                    !principal.grant_all,
                    "a scoped principal is sent as-is, not coerced to grant-all"
                );
                assert_eq!(principal.grants.len(), 1, "the scoped grant is carried");
            }
            other => panic!("expected Authenticate, got {other:?}"),
        }
    }

    fn registry_with_sub(last_seq: u64) -> (Registry, mpsc::Receiver<ClientResult<StreamEvent>>) {
        let (tx, rx) = mpsc::channel(16);
        let mut state = SubState::new(
            tx,
            Instrument::default(),
            WireConventions::default(),
            None,
            None,
            None,
        );
        state.last_seq = last_seq;
        let mut map = HashMap::new();
        map.insert(SUB_ID, state);
        (Arc::new(Mutex::new(map)), rx)
    }

    #[test]
    fn no_gap_before_a_baseline_is_established() {
        assert!(!is_gap(0, 5));
        assert!(!is_stale(0, 5));
    }

    #[test]
    fn in_order_advance_is_neither_gap_nor_stale() {
        assert!(!is_gap(7, 8));
        assert!(!is_stale(7, 8));
    }

    #[test]
    fn a_skipped_sequence_is_a_gap() {
        assert!(is_gap(7, 9));
        assert!(is_gap(7, 100));
        assert!(!is_stale(7, 9));
    }

    #[test]
    fn a_repeated_or_old_sequence_is_stale_not_a_gap() {
        assert!(is_stale(7, 7));
        assert!(is_stale(7, 3));
        assert!(!is_gap(7, 7));
        assert!(!is_gap(7, 3));
    }

    /// A `LAGGED` StreamEnd surfaces a distinct [`StreamEvent::Lagged`] carrying
    /// `last_good` and emits a `Resync` from that sequence — never a degenerate
    /// `GapDetected{last_good == observed}`.
    #[tokio::test]
    async fn lagged_stream_end_emits_distinct_lagged_event_and_resyncs() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (registry, mut rx) = registry_with_sub(42);
            let (relay_tx, mut relay_rx) = mpsc::channel(8);

            let flow = handle_stream_end(
                &registry,
                &relay_tx,
                SUB_ID,
                stream_end::Reason::Lagged as i32,
            )
            .await;
            assert!(
                matches!(flow, SessionFlow::Continue),
                "lag resyncs in place"
            );

            match rx.recv().await.expect("an event").expect("ok event") {
                StreamEvent::Lagged { last_good } => assert_eq!(last_good, 42),
                other => panic!("expected Lagged, got {other:?}"),
            }
            let ctrl = relay_rx.recv().await.expect("a control message");
            match ctrl.message.expect("a payload") {
                client_stream_message::Message::Resync(r) => {
                    assert_eq!(r.subscription.expect("sub id").value, SUB_ID);
                    assert_eq!(r.last_sequence, 42);
                }
                other => panic!("expected Resync, got {other:?}"),
            }
        })
        .await
        .expect("test must not hang");
    }

    /// A `DRAINING` StreamEnd signals a whole-session reconnect and emits no
    /// lag/gap event for the subscription on that path.
    #[tokio::test]
    async fn draining_stream_end_signals_reconnect() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (registry, mut rx) = registry_with_sub(10);
            let (relay_tx, _relay_rx) = mpsc::channel(8);

            let flow = handle_stream_end(
                &registry,
                &relay_tx,
                SUB_ID,
                stream_end::Reason::Draining as i32,
            )
            .await;
            assert!(matches!(flow, SessionFlow::Reconnect));
            assert!(rx.try_recv().is_err(), "drain emits no lag/gap event here");
        })
        .await
        .expect("test must not hang");
    }

    /// Liveness regression: a click-to-trade pending its outcome must NOT hang when
    /// the session re-dials (a `DRAINING` blue-green cutover) mid-flight. The
    /// reconnect path drains the waiter table via `fail_all_waiters`, so the awaiting
    /// `execute` future resolves *promptly* with a typed [`ClientError::Reconnected`]
    /// — never an infinite await on a token that died with the prior connection.
    #[tokio::test]
    async fn pending_execute_fails_promptly_when_session_reconnects() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let waiters: ExecuteWaiters = Arc::new(Mutex::new(HashMap::new()));
            // A pending click-to-trade, registered exactly as `Subscription::execute`
            // does before it awaits its one-shot.
            let (otx, orx) = oneshot::channel::<ClientResult<ExecuteOutcome>>();
            waiters.lock().unwrap().insert(42, otx);

            // The session re-dials mid-flight (the action `reconnect_session` takes
            // before touching the stream).
            fail_all_waiters(&waiters, &ClientError::Reconnected);

            // The await resolves promptly with a typed error — the `execute` future's
            // exact path (`Ok(outcome) => outcome`, here an `Err`).
            let outcome = match orx.await {
                Ok(o) => o,
                Err(_) => Err(ClientError::StreamClosed),
            };
            assert!(
                matches!(outcome, Err(ClientError::Reconnected)),
                "a reconnect-failed click must surface Reconnected, got {outcome:?}"
            );
            // The table is drained: no stale waiter leaks across the cutover.
            assert!(waiters.lock().unwrap().is_empty(), "waiter table drained");
        })
        .await
        .expect("a reconnecting click-to-trade must never hang");
    }

    /// A session close (server closed / transport error) also fails every pending
    /// click promptly with a typed error, so no `execute` await hangs on teardown.
    #[tokio::test]
    async fn pending_execute_fails_promptly_when_session_closes() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let waiters: ExecuteWaiters = Arc::new(Mutex::new(HashMap::new()));
            let (otx, orx) = oneshot::channel::<ClientResult<ExecuteOutcome>>();
            waiters.lock().unwrap().insert(7, otx);

            fail_all_waiters(&waiters, &ClientError::StreamClosed);

            let outcome = orx
                .await
                .expect("the driver delivered an error, not a drop");
            assert!(matches!(outcome, Err(ClientError::StreamClosed)));
        })
        .await
        .expect("a closing click-to-trade must never hang");
    }

    /// A click-to-trade outcome is routed to the waiting `execute` by correlation id;
    /// an unknown correlation is a harmless no-op.
    #[test]
    fn complete_execute_routes_by_correlation() {
        let waiters: ExecuteWaiters = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        waiters.lock().unwrap().insert(99, tx);

        // Unknown correlation: no-op (does not complete ours).
        complete_execute(
            &waiters,
            Some(7),
            ExecuteOutcome::Rejected {
                reason: RejectReason::Expired,
            },
        );
        assert!(waiters.lock().unwrap().contains_key(&99));

        complete_execute(
            &waiters,
            Some(99),
            ExecuteOutcome::Booked(ClickExecution {
                execution_id: 5,
                side: Side::Buy,
                traded_premium: 0.012,
                epoch_nanos: 1,
            }),
        );
        let outcome = rx
            .blocking_recv()
            .expect("outcome delivered")
            .expect("a booked outcome, not an error");
        match outcome {
            ExecuteOutcome::Booked(e) => assert_eq!(e.execution_id, 5),
            other => panic!("expected Booked, got {other:?}"),
        }
    }
}
