//! The multiplexed RFS (request-for-stream) session: ONE bidirectional gRPC
//! channel carrying many concurrent subscriptions, each keyed by its own
//! [`celnet_proto::SubscriptionId`], plus click-to-trade execution off the streamed
//! lines.
//!
//! A counterparty opens a single [`celnet_proto::stream_service_server::StreamService::stream_session`]
//! channel and multiplexes any number of subscriptions over it. For each
//! subscription the session driver:
//!
//! 1. sends a [`celnet_proto::Snapshot`] (sequence 1) — the full baseline state of
//!    the subscribed [`celnet_proto::Instrument`] (two-way price, Greeks, vol),
//!    stamped with click-to-trade [`celnet_proto::TradableToken`]s (one to SELL at
//!    the bid, one to BUY at the offer) and the pinned `surface_version`;
//! 2. drives a **deterministic** market tick loop that reprices *that*
//!    subscription's instrument and emits sequenced [`celnet_proto::Update`] deltas
//!    (sequence 2, 3, …) — each minting fresh tradable tokens and retiring the
//!    prior ones — interleaved with periodic [`celnet_proto::Heartbeat`]s;
//! 3. answers a [`celnet_proto::Modify`] by re-baselining the subscription in place
//!    (new instrument / conventions / throttle / surface pin) with a fresh
//!    [`celnet_proto::Snapshot`] at the next sequence;
//! 4. answers a [`celnet_proto::Resync`] by replaying the retained messages after
//!    the client's last good sequence (or a fresh snapshot when the gap predates
//!    the buffer);
//! 5. books a click-to-trade [`celnet_proto::Execute`] that presents a live token
//!    (within its `valid_until_nanos`, unconsumed, known) with an
//!    [`celnet_proto::Executed`], or declines a stale / forged / already-consumed
//!    token with a [`celnet_proto::StreamReject`];
//! 6. tears the subscription down on [`celnet_proto::Unsubscribe`] (or stream
//!    close), emitting a [`celnet_proto::StreamEnd`].
//!
//! # Click-to-trade tokens (last-look off the stream)
//!
//! Each streamed line stamps two short-lived [`celnet_proto::TradableToken`]s
//! binding `(subscription, sequence, side, premium)` to a bounded validity window.
//! An [`celnet_proto::Execute`] presenting a token within its window books exactly
//! the stamped premium — no separate RFQ round-trip, no price-it-then-re-request
//! race. After `valid_until_nanos` the token is dead; a presented expired / unknown
//! / already-consumed token is rejected (last-look, identical to the RFQ deadline).
//! Tokens are **cryptographically unforgeable** — each is a truncated keyed MAC (a
//! `blake3` keyed hash) over the line-binding tuple, under a 256-bit key drawn once
//! from the OS CSPRNG at session start (see [`TokenMinter`]) — so a token cannot be
//! forged or enumerated without the server secret. (This MAC key is a runtime
//! control-plane identity, **not** a pricing input, so seeding it from a CSPRNG does
//! not touch the platform's deterministic-pricing guarantee.) A new sequence mints
//! new tokens and retires the prior ones, so a click always books the *current*
//! streamed premium.
//!
//! # Per-subscriber back-pressure, never block the core
//!
//! Each session owns a **bounded** server→client channel. The market tick is the
//! same deterministic, counter-based [`crate::tick::TickSource`] discipline the
//! engine uses elsewhere, so a stream is bit-exactly reproducible from its seed;
//! the per-subscription instrument is repriced on the async edge against the ticked
//! market (the pinned hot core is never blocked by a slow streaming consumer). If a
//! subscriber lags past its channel depth the subscription is paused in a
//! recoverable lagged state (a single LAGGED [`celnet_proto::StreamEnd`] prompts a
//! [`celnet_proto::Resync`]) rather than back-pressuring the driver — one slow
//! consumer never stalls the others.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use celnet_fanout::Consumer;
use celnet_observability::LatencyRecorder;
use celnet_proto::stream_service_server::StreamService;
use celnet_proto::{
    ClientStreamMessage, Conventions, Execute, Executed, Heartbeat, Instrument, MarketContext,
    MarketObservable, MarketSeriesPoint, MarketSeriesSnapshot, MarketSeriesSubscribe,
    ServerStreamMessage, Side, Snapshot, StreamEnd, StreamReject, SubscriptionId, TradableToken,
    TwoWayPrice, Update, client_stream_message, server_stream_message, stream_end, stream_reject,
};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tonic::{Request, Response, Status, Streaming};

use crate::clock::Clock;
use crate::core_link::{CoreLink, Observable, ObservableQuery};
use crate::pricer::{ConventionSet, Priced, price_instrument};
use crate::readiness::ReadinessGate;
use crate::services::clicktrade::{
    BookOutcome, TokenLedger, TokenMinter, TwoWayLine, mint_two_way,
};
use crate::services::forward::route_pair;
use crate::services::pin::{PinnedVol, resolve_pinned_vol};
use crate::services::pricefanout::{PriceFanout, PriceTick};
use crate::services::risk::federate::Fleet;
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

use super::stream_rx::ReceiverStream;

/// The bounded depth of each per-session server→client channel. A subscriber
/// lagging past this many messages is paused (LAGGED) rather than stalling the
/// driver. 256 mirrors the engine's broadcast depth.
const CHANNEL_DEPTH: usize = 256;

/// The number of recent server messages retained per subscription for a Resync
/// replay. A Resync replays from the requested sequence forward, so this bounds
/// how far back a gap can be recovered before a fresh snapshot is needed.
const REPLAY_DEPTH: usize = 512;

/// The wall-clock interval between deterministic market ticks driving each
/// subscription's updates. Small but nonzero so a test observes several updates
/// quickly while the loop stays cooperative and parks the runtime between ticks.
const TICK_INTERVAL: Duration = Duration::from_millis(5);

/// Emit a heartbeat every this many ticks (so the stream proves liveness even if
/// the repriced line is momentarily unchanged).
const HEARTBEAT_EVERY: u64 = 8;

/// How long a streamed line's click-to-trade tokens stay valid for execution, in
/// nanoseconds (1 second — a tight last-look window appropriate for a live stream
/// where a new sequence mints fresh tokens every few milliseconds).
const TOKEN_VALIDITY_NANOS: i64 = 1_000_000_000;

/// One live market-series (TrendMode) subscription: the observable it streams off
/// the live market state, its conflation throttle, and the monotonic per-series
/// sequence. The series shares the price-stream [`SubscriptionId`] space but is
/// otherwise independent (it carries no tradable tokens — a market observable is
/// information, not a dealable line).
///
/// The opening [`MarketSeriesSnapshot`] seeds a series from a single, freshly
/// **observed** point off the live state; the platform retains no historical
/// time-series store, so the feed does not (and must not) backfill fabricated
/// history — every subsequent point is a genuine live observation appended by the
/// tick loop. (A durable observation store that lets the snapshot replay a real
/// recent window is a future enhancement, deliberately not faked here.)
struct MarketSeries {
    id: SubscriptionId,
    /// The observable derived from the live state on each sample.
    observable: Observable,
    /// The minimum nanoseconds between appended points (client conflation hint);
    /// `0` ⇒ a point every session tick.
    throttle_nanos: i64,
    /// The monotonic per-series sequence number last emitted.
    sequence: u64,
    /// The epoch-nanos timestamp of the last appended point (for throttling).
    last_emit_nanos: i64,
}

/// The RFS streaming service over the [`CoreLink`], readiness gate, and the shared
/// versioned marked-surface registry.
#[derive(Debug)]
pub struct StreamEdge {
    link: Arc<CoreLink>,
    gate: Arc<ReadinessGate>,
    spread: SpreadModel,
    clock: Clock,
    surface_book: Arc<SurfaceBook>,
    /// Monotonic generator for server-assigned click-to-trade execution ids.
    next_execution_id: Arc<AtomicU64>,
    /// The shared live position book a click-to-trade fill records its booked vanilla
    /// position into, so `RiskService` aggregates the same lines this stream trades
    /// (API-first: the Book/Risk views read the server's aggregate of this book).
    /// `None` when the edge is run without a risk store (e.g. an isolated stream
    /// test); booking is then a no-op, never a fake.
    store: Option<Arc<crate::services::risk::store::PositionStore>>,
    /// The connected backend fleet for owned-pair forwarding; `None` ⇒ in-process
    /// (stream locally). Reuses the SAME `Fleet` the risk federation connects. In
    /// distributed mode a `StreamSession` is **pinned to the owner of its first
    /// subscription** and the bidi stream is relayed to that backend (see
    /// [`StreamEdge::stream_session`]).
    fleet: Option<Arc<Fleet>>,
    /// The shared per-pair price-tick fan-out: ONE producer per pair drives the
    /// deterministic spot path into a `celnet-fanout` SPMC ring; every session
    /// subscribed to that pair drains an independent consumer (the 1-producer →
    /// N-consumers scale improvement that replaced the old per-subscription
    /// counter-based spot ticker). Shared across all sessions on this edge (and
    /// the WS mirror, which builds its session driver from the same `StreamEdge`).
    fanout: Arc<PriceFanout>,
}

impl StreamEdge {
    /// Construct the RFS service without a risk position store (click-to-trade fills
    /// are not recorded for risk aggregation).
    #[must_use]
    pub fn new(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
    ) -> Self {
        Self {
            link,
            gate,
            spread,
            clock,
            surface_book,
            next_execution_id: Arc::new(AtomicU64::new(1)),
            store: None,
            fleet: None,
            fanout: PriceFanout::start(),
        }
    }

    /// Construct the RFS service wired to the shared live position book, so every
    /// click-to-trade fill of a vanilla line records a booked position the
    /// `RiskService` aggregates (the live book behind the Book/Risk views).
    #[must_use]
    pub fn with_store(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        store: Arc<crate::services::risk::store::PositionStore>,
    ) -> Self {
        Self {
            link,
            gate,
            spread,
            clock,
            surface_book,
            next_execution_id: Arc::new(AtomicU64::new(1)),
            store: Some(store),
            fleet: None,
            fanout: PriceFanout::start(),
        }
    }

    /// Construct the RFS service wired to the shared live position book **and** an
    /// optional connected backend [`Fleet`]: `Some(fleet)` ⇒ distributed (pin a
    /// session to the owner of its first subscription and relay the bidi stream to
    /// that backend); `None` ⇒ in-process, exactly [`StreamEdge::with_store`].
    #[must_use]
    pub fn with_store_and_fleet(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        store: Arc<crate::services::risk::store::PositionStore>,
        fleet: Option<Arc<Fleet>>,
    ) -> Self {
        Self {
            link,
            gate,
            spread,
            clock,
            surface_book,
            next_execution_id: Arc::new(AtomicU64::new(1)),
            store: Some(store),
            fleet,
            fanout: PriceFanout::start(),
        }
    }
}

/// The per-subscription server-side state.
struct Subscription {
    id: SubscriptionId,
    instrument: Instrument,
    conv: ConventionSet,
    /// The base market the subscription was opened against (its tick source bumps
    /// spot around this). Carries the pinned-surface vol when a `surface_version`
    /// is pinned, so streamed prices reproduce the marked surface.
    base_market: MarketContext,
    /// The surface version echoed on this subscription's snapshots/updates: the
    /// pinned version if one was requested, else `None` (live mark).
    surface_version: Option<u64>,
    /// The opening `Subscribe.correlation_id`, echoed on the first snapshot.
    correlation_id: Option<u64>,
    /// The opening `Subscribe.attribution` (book/seat the line is shown to),
    /// echoed on snapshots and resolved onto click-to-trade fills so the
    /// who's-trading dimension is preserved. `None` ⇒ unattributed.
    attribution: Option<celnet_proto::AttributionRecord>,
    /// The monotonic per-subscription sequence number last emitted.
    sequence: u64,
    /// The highest sequence actually handed to the client's channel (delivered,
    /// not dropped). A Resync never replays at or below this.
    delivered: u64,
    /// Whether this subscription is in the recoverable **lagged** state.
    lagged: bool,
    /// Whether the `LAGGED` [`StreamEnd`] marker has been delivered to the client.
    lag_notified: bool,
    /// This subscription's independent consumer on its **pair's** shared
    /// `celnet-fanout` price-tick ring. `drive_tick` drains it (non-blocking) to
    /// learn the next per-pair market and emit one `Update` per drained tick. The
    /// ring is the 1-producer-per-pair → N-consumers fan-out that replaced the old
    /// per-subscription counter-based spot ticker.
    tick: Consumer<PriceTick>,
    /// The most recently streamed market for this subscription (the last tick
    /// delivered as an `Update`, or the opening baseline before any tick). A
    /// server-assisted Resync whose gap predates the replay buffer re-baselines on
    /// this on-path market so the client never holds an off-path baseline.
    last_market: MarketContext,
    /// Recent server messages retained for a Resync replay (seq → message).
    replay: VecDeque<(u64, ServerStreamMessage)>,
    /// The click-to-trade token ledger (live + bounded-consumed sets): the SAME
    /// keyed-MAC last-look / replay discipline the FIX acceptor uses, shared via
    /// [`crate::services::clicktrade`]. A new sequence mints fresh tokens and retires
    /// the prior ones; a consumed token blocks replay within its window.
    tokens: TokenLedger,
    /// `Execute` idempotency: a client key → the `Executed` it booked, so an
    /// `Execute` retry carrying the same key returns the same booking.
    execute_idempotency: HashMap<String, Executed>,
    /// Drain-side HdrHistogram of the server-side price-compute latency (ns) for
    /// this subscription's updates — surfaced as the p50/p99/p99.9 on the
    /// heartbeat. Lives on the streaming edge, NEVER the pinned hot core (see
    /// [`make_update`]).
    latency: LatencyRecorder,
}

impl Subscription {
    /// Price the instrument at the current `market`, returning the priced line and
    /// the two-way market under the spread model.
    fn price(
        &self,
        market: &MarketContext,
        spread: &SpreadModel,
    ) -> Result<(Priced, TwoWayPrice), Status> {
        let priced = price_instrument(&self.instrument, market, &self.conv)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let two_way = spread.two_way(priced.greeks.price, &priced.greeks);
        Ok((priced, two_way))
    }

    /// Push a server message into the replay buffer, evicting the oldest beyond
    /// [`REPLAY_DEPTH`].
    fn retain(&mut self, seq: u64, msg: ServerStreamMessage) {
        if self.replay.len() == REPLAY_DEPTH {
            self.replay.pop_front();
        }
        self.replay.push_back((seq, msg));
    }
}

/// Re-encode a decoded [`ConventionSet`] back to the wire form for echoing.
fn conv_to_wire(c: &ConventionSet) -> Conventions {
    Conventions {
        delta_convention: celnet_proto::DeltaConvention::from(c.delta) as i32,
        atm_convention: celnet_proto::AtmConvention::from(c.atm) as i32,
        premium_style: celnet_proto::PremiumStyle::from(c.premium) as i32,
        cut: celnet_proto::Cut::from(c.cut) as i32,
        day_count: celnet_proto::DayCount::from(c.day_count) as i32,
        settlement: celnet_proto::Settlement::from(c.settlement) as i32,
    }
}

/// Decode the wire conventions, mapping a decode error to `invalid_argument`.
fn decode_conv(w: &Conventions) -> Result<ConventionSet, Status> {
    ConventionSet::decode(w).map_err(|e| Status::invalid_argument(e.to_string()))
}

/// Decode a wire [`MarketObservable`] tag plus its optional delta into the
/// core-link [`Observable`], enforcing the contract's presence rules: the wing
/// observables (RR / BF) require a delta; the others (ATM vol / spot / forward) do
/// not take one. A degenerate (non-positive, ≥ 1) delta wing is rejected.
fn decode_observable(tag: i32, delta: Option<f64>) -> Result<Observable, Status> {
    let wire = MarketObservable::try_from(tag)
        .map_err(|_| Status::invalid_argument(format!("unknown MarketObservable tag {tag}")))?;
    match wire {
        MarketObservable::AtmVol => Ok(Observable::AtmVol),
        MarketObservable::Spot => Ok(Observable::Spot),
        MarketObservable::Forward => Ok(Observable::Forward),
        MarketObservable::RiskReversal | MarketObservable::Butterfly => {
            let d = delta.ok_or_else(|| {
                Status::invalid_argument("a wing observable (RR/BF) requires a `delta`")
            })?;
            if !(d.abs() > 0.0 && d.abs() < 1.0) {
                return Err(Status::invalid_argument(
                    "wing `delta` must be a magnitude in (0, 1)",
                ));
            }
            Ok(if wire == MarketObservable::RiskReversal {
                Observable::RiskReversal { delta: d }
            } else {
                Observable::Butterfly { delta: d }
            })
        }
    }
}

/// Mint the two click-to-trade tokens for a two-way line (SELL@bid, BUY@offer),
/// register them as the subscription's live tokens (retiring any prior ones), and
/// return the wire [`TradableToken`]s to stamp on the message. A bid floored at
/// zero mints no SELL token (there is nothing to hit), so a degenerate line is
/// indicative-only on that side.
fn mint_tokens(
    sub: &mut Subscription,
    minter: &TokenMinter,
    seq: u64,
    two_way: &TwoWayPrice,
    now_nanos: i64,
) -> Vec<TradableToken> {
    // Delegate to the SHARED keyed-MAC mint path (the same one the FIX acceptor
    // uses), then map the minted tokens to the RFS `TradableToken` wire form.
    mint_two_way(
        &mut sub.tokens,
        minter,
        TwoWayLine {
            line_id: sub.id.value,
            sequence: seq,
            bid: two_way.bid,
            offer: two_way.offer,
        },
        now_nanos,
        TOKEN_VALIDITY_NANOS,
    )
    .into_iter()
    .map(|m| TradableToken {
        token: m.token,
        side: m.side as i32,
        premium: m.premium,
        valid_until_nanos: m.valid_until_nanos,
    })
    .collect()
}

/// Build a [`Snapshot`] message for a subscription at `seq` priced against
/// `market`, minting fresh click-to-trade tokens and echoing the pinned surface
/// version + correlation id.
fn make_snapshot(
    sub: &mut Subscription,
    seq: u64,
    market: &MarketContext,
    spread: &SpreadModel,
    minter: &TokenMinter,
    clock: &Clock,
) -> Result<ServerStreamMessage, Status> {
    let (priced, two_way) = sub.price(market, spread)?;
    let now = clock.now_nanos();
    let tradable = mint_tokens(sub, minter, seq, &two_way, now);
    Ok(ServerStreamMessage {
        message: Some(server_stream_message::Message::Snapshot(Snapshot {
            subscription: Some(sub.id),
            sequence: seq,
            price: Some(two_way),
            greeks: Some(priced.greeks.into()),
            vol: priced.vol,
            conventions: Some(conv_to_wire(&sub.conv)),
            resolved_strike: priced.resolved_strike,
            tradable,
            surface_version: sub.surface_version,
            correlation_id: sub.correlation_id,
            epoch_nanos: now,
            attribution: sub.attribution.clone(),
        })),
    })
}

/// Build an [`Update`] message for a subscription at `seq` priced against `market`,
/// minting fresh click-to-trade tokens and echoing the pinned surface version.
fn make_update(
    sub: &mut Subscription,
    seq: u64,
    market: &MarketContext,
    spread: &SpreadModel,
    minter: &TokenMinter,
    clock: &Clock,
) -> Result<ServerStreamMessage, Status> {
    // Time the server-side price compute on the DRAIN path (the streaming edge),
    // never the pinned zero-alloc hot core — `Instant` + the HdrHistogram record
    // both live here on the already-non-critical update path, so the price+Greek
    // loop itself is untouched. This is the honest per-subscription p50/p99/p99.9
    // surfaced on the heartbeat.
    let t0 = std::time::Instant::now();
    let (priced, two_way) = sub.price(market, spread)?;
    let compute_ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
    sub.latency.record_ns(compute_ns);
    let now = clock.now_nanos();
    let tradable = mint_tokens(sub, minter, seq, &two_way, now);
    Ok(ServerStreamMessage {
        message: Some(server_stream_message::Message::Update(Update {
            subscription: Some(sub.id),
            sequence: seq,
            price: Some(two_way),
            greeks: Some(priced.greeks.into()),
            vol: priced.vol,
            tradable,
            surface_version: sub.surface_version,
            epoch_nanos: now,
        })),
    })
}

/// Try to send a server message to the subscriber, returning `false` if the
/// bounded channel is full (the subscriber lagged) or closed.
fn try_emit(
    tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
    msg: ServerStreamMessage,
) -> bool {
    matches!(tx.try_send(Ok(msg)), Ok(()))
}

#[tonic::async_trait]
impl StreamService for StreamEdge {
    type StreamSessionStream = ReceiverStream<Result<ServerStreamMessage, Status>>;

    async fn stream_session(
        &self,
        request: Request<Streaming<ClientStreamMessage>>,
    ) -> Result<Response<Self::StreamSessionStream>, Status> {
        // The readiness gate gates *new* sessions: a draining instance refuses to
        // open a session so new traffic steers to the warm replacement.
        let guard = self.gate.enter();
        if !self.gate.is_ready() {
            return Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ));
        }

        let inbound = request.into_inner();
        let (out_tx, out_rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(CHANNEL_DEPTH);

        // Distributed: this session is RELAYED to the backend that owns the pair of
        // its FIRST subscription (the §3 owned-pair forwarding tier). We pin the
        // whole session to that one owner and relay both directions verbatim. A
        // single multiplexed session subscribing to pairs owned by DIFFERENT backends
        // is honestly **deferred**: the cross-owner case would need one upstream
        // session per owner fanned back over the single client session, which is a
        // genuine refinement — here we pin to the first owner and the backend itself
        // answers a foreign-pair subscribe (it serves whatever it is asked; the
        // partition is a routing convenience, not a hard backend filter), so nothing
        // is faked. See module docs / `docs/SCALE-OUT.md` §3.
        if let Some(fleet) = self.fleet.clone() {
            tokio::spawn(async move {
                let _guard = guard; // held for the session lifetime (drain barrier).
                relay_session(fleet, inbound, out_tx).await;
            });
            return Ok(Response::new(ReceiverStream::new(out_rx)));
        }

        // The session driver: owns every subscription multiplexed on this session,
        // reads client control + click-to-trade messages, and emits server
        // messages. One task per session keeps the per-subscription state
        // single-owner (no locks) and tears everything down when the stream closes.
        // The same driver backs the WebSocket mirror (`crate::ws`): a session is
        // transport-agnostic — it only needs an inbound `ClientStreamMessage` stream
        // and an outbound channel — so gRPC and WS share one pricing/streaming path
        // and one contract.
        let driver = self.session_driver();
        tokio::spawn(async move {
            let _guard = guard; // held for the session lifetime (drain barrier).
            // Adapt the tonic `Streaming` (yields `Result`) to the driver's
            // infallible stream: a transport error closes the inbound side, exactly
            // as a closed stream does.
            let inbound = inbound.filter_map(|r| async move { r.ok() });
            tokio::pin!(inbound);
            run_session(driver, inbound, out_tx).await;
        });

        Ok(Response::new(ReceiverStream::new(out_rx)))
    }
}

impl StreamEdge {
    /// Build a fresh single-session driver bound to the shared core/spread/clock/
    /// surface registry. One [`Session`] per opened channel keeps the
    /// per-subscription state single-owner (no locks); every session shares the same
    /// [`CoreLink`] (and thus the same pinned pricing core) and the same execution-id
    /// counter, so gRPC and the WebSocket mirror book against one path.
    #[must_use]
    pub(crate) fn session_driver(&self) -> Session {
        Session {
            subs: HashMap::new(),
            series: HashMap::new(),
            link: Arc::clone(&self.link),
            spread: self.spread,
            clock: self.clock.clone(),
            surface_book: Arc::clone(&self.surface_book),
            minter: TokenMinter::new(),
            next_execution_id: Arc::clone(&self.next_execution_id),
            store: self.store.clone(),
            fanout: Arc::clone(&self.fanout),
        }
    }
}

/// The routing pair of one client→server stream message, if it carries one. Only an
/// opening [`celnet_proto::Subscribe`] (its instrument's pair) or a
/// [`MarketSeriesSubscribe`] (its pair) names a pair; control frames (modify /
/// unsubscribe / resync / execute / heartbeat) do not, so a session is routed off the
/// FIRST pair-bearing message.
fn pair_of_client_message(msg: &ClientStreamMessage) -> Option<&celnet_proto::CcyPair> {
    let underlying = match msg.message.as_ref()? {
        client_stream_message::Message::Subscribe(s) => {
            s.instrument.as_ref()?.underlying.as_ref()?
        }
        client_stream_message::Message::MarketSeriesSubscribe(s) => s.underlying.as_ref()?,
        _ => return None,
    };
    underlying.as_fx()
}

/// **Relay one client `StreamSession` to its owning backend** (the distributed
/// owned-pair forwarding path). The session is **pinned to the owner of its first
/// subscription**: this reads inbound client messages until the first carries a pair,
/// routes that pair through the fleet's HRW [`PartitionMap`](celnet_router::PartitionMap)
/// to the owning backend, opens ONE upstream `StreamSession` to that backend, replays
/// the buffered messages, and relays both directions verbatim until either side ends.
///
/// An unreachable owner ⇒ the client receives a single [`Status::unavailable`] error
/// and the session ends (never a silent mis-route, never a faked local stream).
///
/// Cross-owner multiplexing within one session is the **honestly-deferred refinement**
/// (see [`StreamService::stream_session`]): every subscription on this session is
/// relayed to the first subscription's owner. The backend serves whatever pair it is
/// asked (the partition is a routing convenience), so a foreign-pair subscription on a
/// pinned session is answered correctly by that backend — it is just not load-balanced
/// to its own natural owner. No behaviour is faked.
async fn relay_session<S>(
    fleet: Arc<Fleet>,
    mut inbound: S,
    out_tx: mpsc::Sender<Result<ServerStreamMessage, Status>>,
) where
    S: futures_util::Stream<Item = Result<ClientStreamMessage, Status>> + Send + Unpin + 'static,
{
    // 1. Read inbound messages, buffering until the first pair-bearing one, so we can
    //    learn which backend owns this session before connecting upstream.
    let mut buffered: Vec<ClientStreamMessage> = Vec::new();
    let mut owner_channel = None;
    while let Some(item) = inbound.next().await {
        let msg = match item {
            Ok(m) => m,
            Err(_) => return, // client/transport closed before any subscription.
        };
        if let Some(wire_pair) = pair_of_client_message(&msg) {
            let routed = route_pair(Some(wire_pair))
                .and_then(|pair| fleet.owner_of_pair(pair).map(|(_r, c)| c.channel()));
            match routed {
                Ok(channel) => owner_channel = Some(channel),
                Err(status) => {
                    let _ = out_tx.send(Err(status)).await;
                    return;
                }
            }
            buffered.push(msg);
            break;
        }
        buffered.push(msg);
    }
    let Some(channel) = owner_channel else {
        // The inbound stream ended before any pair-bearing subscription: nothing to
        // route. End cleanly (the client opened a session and never subscribed).
        return;
    };

    // 2. Open ONE upstream StreamSession to the owning backend. The upstream client
    //    side is a channel we push the buffered + remaining client messages onto.
    let (up_tx, up_rx) = mpsc::channel::<ClientStreamMessage>(CHANNEL_DEPTH);
    let mut svc = celnet_proto::stream_service_client::StreamServiceClient::new(channel);
    let up_stream = ReceiverStream::new(up_rx);
    let mut upstream = match svc.stream_session(up_stream).await {
        Ok(resp) => resp.into_inner(),
        Err(status) => {
            let _ = out_tx.send(Err(status)).await;
            return;
        }
    };

    // Replay the buffered client messages to the backend (the first subscription that
    // pinned the owner, plus any pair-less control frames seen before it).
    for msg in buffered {
        if up_tx.send(msg).await.is_err() {
            return; // upstream closed.
        }
    }

    // 3. Relay both directions: client→backend and backend→client, until either ends.
    //    Two cooperating tasks share the upstream halves; this task drives the
    //    server→client leg and joins the client→server pump.
    let pump = tokio::spawn(async move {
        while let Some(item) = inbound.next().await {
            match item {
                Ok(m) => {
                    if up_tx.send(m).await.is_err() {
                        break;
                    }
                }
                Err(_) => break, // client/transport closed.
            }
        }
        // Dropping `up_tx` closes the upstream client side, ending the backend session.
    });

    while let Some(item) = upstream.next().await {
        // Forward the backend's server messages (and any status error) verbatim. A
        // closed client channel (consumer gone) ends the relay.
        if out_tx.send(item).await.is_err() {
            break;
        }
    }
    // The backend stream ended (or the client went away): tear down the pump.
    pump.abort();
}

/// Drive one multiplexed RFS session to completion over a transport-agnostic
/// inbound [`ClientStreamMessage`] stream and an outbound server-message channel.
///
/// This is the single session loop shared by the gRPC `StreamSession` handler and
/// the WebSocket mirror ([`crate::ws`]): it interleaves client control + click-to-
/// trade messages with the deterministic market tick, emitting `Snapshot` /
/// sequenced `Update` / `Heartbeat` / `Executed` / `StreamReject` / `StreamEnd`
/// frames. It returns when the inbound stream ends (client close / transport error)
/// or the outbound channel closes (consumer gone), tearing the session down.
pub(crate) async fn run_session<S>(
    mut session: Session,
    mut inbound: S,
    out_tx: mpsc::Sender<Result<ServerStreamMessage, Status>>,
) where
    S: futures_util::Stream<Item = ClientStreamMessage> + Unpin,
{
    let mut ticker = tokio::time::interval(TICK_INTERVAL);
    // Skip the immediate first tick so the loop parks until either a control
    // message or the first real interval.
    ticker.tick().await;

    loop {
        tokio::select! {
            // ---- client → server control + click-to-trade ----------
            incoming = inbound.next() => {
                match incoming {
                    Some(msg) => {
                        if !session.handle_client_message(msg, &out_tx).await {
                            break; // a fatal send failure: tear down.
                        }
                    }
                    None => break, // client closed / errored.
                }
            }
            // ---- deterministic market tick → updates ---------------
            _ = ticker.tick() => {
                // Price-stream updates (deterministic, sync, off the live read path).
                if !session.subs.is_empty() && !session.drive_tick(&out_tx) {
                    break; // channel closed: client gone.
                }
                // Market-series (TrendMode) points sampled off the *live* market
                // state — an async read per due series, conflated by the client
                // throttle, off the pinned hot core.
                if !session.series.is_empty() && !session.drive_market_series(&out_tx).await {
                    break; // channel closed: client gone.
                }
            }
        }
    }
}

/// Record a click-to-trade fill of a **vanilla** line into the shared live position
/// book, so `RiskService` aggregates the same lines this stream trades.
///
/// The booked fact is built from the subscription's instrument + the market it was
/// shown against: pair, expiry, the resolved strike + vol (re-priced off the
/// subscription's base market), the quoted conventions, the surface version, and the
/// signed base notional (BUY = long, SELL = short). A non-vanilla product, a missing
/// pair, or an unparseable convention is **not** recorded (honest scope — never a
/// faked vanilla risk leaf). Best-effort: a failure to record never fails the trade.
fn record_booked_position(
    store: &crate::services::risk::store::PositionStore,
    sub: &Subscription,
    spread: &SpreadModel,
    execution_id: u64,
    side: Side,
) {
    use celnet_proto::instrument::Product;

    // Only vanilla lines have a canonical-vanilla risk leaf.
    let Some(Product::Vanilla(vanilla)) = sub.instrument.product.as_ref() else {
        return;
    };
    let Ok(option) = celnet_proto::OptionType::try_from(vanilla.option_type) else {
        return;
    };
    let option = celnet_types::OptionType::from(option);

    // The pair the line trades (the FX arm of the instrument's underlying).
    let Some(wire_underlying) = sub.instrument.underlying.as_ref() else {
        return;
    };
    let Ok(pair) =
        celnet_proto::convert::validate_fx_underlying(wire_underlying).map(|u| u.as_fx())
    else {
        return;
    };
    let Some(pair) = pair else {
        return;
    };

    // Resolve the strike + vol the line is marked at by pricing off the base market
    // (the same deterministic pricer the stream shows the line with).
    let Ok((priced, _two_way)) = sub.price(&sub.base_market, spread) else {
        return;
    };
    let inputs = celnet_types::VanillaInputs::new(
        sub.base_market.spot,
        priced.resolved_strike,
        priced.vol,
        sub.instrument.expiry_years,
        sub.base_market.r_dom(),
        sub.base_market.r_for(),
    );

    // The signed base-currency notional: BUY = long (+), SELL = short (−). A
    // quote-ccy notional is converted to a base-equivalent at spot (the line's hedge
    // is in base-ccy units; the cube's delta_base is a base amount).
    let quantity = sub.instrument.quantity.as_ref();
    let abs_base = quantity.map_or(0.0, |q| {
        if q.base_ccy {
            q.notional
        } else if sub.base_market.spot != 0.0 {
            q.notional / sub.base_market.spot
        } else {
            0.0
        }
    });
    let signed = match side {
        Side::Sell => -abs_base,
        _ => abs_base,
    };
    if signed == 0.0 {
        return; // nothing to book (no notional).
    }

    let booked = crate::services::risk::store::BookedPosition {
        position_id: execution_id,
        pair,
        option,
        notional_base: signed,
        inputs,
        quoted_delta: sub.conv.delta,
        premium_style: sub.conv.premium,
        surface_version: sub.surface_version.unwrap_or(0),
    };
    let _ = store.book_from_attribution(booked, &sub.attribution.clone().unwrap_or_default());
}

/// The single-owner per-session state and the operations over it.
pub(crate) struct Session {
    subs: HashMap<u64, Subscription>,
    /// The live market-series (TrendMode) subscriptions multiplexed on this
    /// session, keyed by their `SubscriptionId` (in the same id space as price
    /// streams). Sampled off the live market state on each tick, conflated per the
    /// client throttle.
    series: HashMap<u64, MarketSeries>,
    link: Arc<CoreLink>,
    spread: SpreadModel,
    clock: Clock,
    surface_book: Arc<SurfaceBook>,
    minter: TokenMinter,
    next_execution_id: Arc<AtomicU64>,
    /// The shared live position book a click-to-trade fill records into (`None` ⇒
    /// no risk store wired; booking is a no-op).
    store: Option<Arc<crate::services::risk::store::PositionStore>>,
    /// The shared per-pair price-tick fan-out (see [`StreamEdge::fanout`]). On
    /// subscribe the session draws a [`PriceTick`] consumer for the subscription's
    /// pair; `drive_tick` drains it to emit per-subscription updates.
    fanout: Arc<PriceFanout>,
}

impl Session {
    /// Read the maker's live market as a subscription baseline, then resolve any
    /// pinned `surface_version` into the effective market + echoed version.
    async fn baseline_market(
        &self,
        instrument: &Instrument,
        surface_version: Option<u64>,
    ) -> Result<PinnedVol, Status> {
        let snap = self
            .link
            .market_snapshot()
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;
        let market = MarketContext::fx(snap.spot, snap.atm_vol, snap.r_dom, snap.r_for);
        resolve_pinned_vol(&self.surface_book, surface_version, instrument, &market)
    }

    /// Handle one client→server message, mutating the subscription set and emitting
    /// server messages. Returns `false` only on a fatal channel close.
    async fn handle_client_message(
        &mut self,
        msg: ClientStreamMessage,
        out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
    ) -> bool {
        let Some(message) = msg.message else {
            return true; // empty control frame: ignore.
        };
        match message {
            client_stream_message::Message::Subscribe(s) => self.handle_subscribe(s, out_tx).await,
            client_stream_message::Message::Modify(m) => self.handle_modify(m, out_tx).await,
            client_stream_message::Message::Unsubscribe(u) => {
                if let Some(id) = u.subscription
                    && self.subs.remove(&id.value).is_some()
                {
                    let end = ServerStreamMessage {
                        message: Some(server_stream_message::Message::StreamEnd(StreamEnd {
                            subscription: Some(id),
                            reason: stream_end::Reason::Unsubscribed as i32,
                        })),
                    };
                    let _ = out_tx.send(Ok(end)).await;
                }
                true
            }
            client_stream_message::Message::Resync(r) => {
                let Some(id) = r.subscription else {
                    return true;
                };
                if !self.subs.contains_key(&id.value) {
                    return true;
                }
                self.handle_resync(id.value, r.last_sequence, out_tx).await
            }
            client_stream_message::Message::Execute(e) => self.handle_execute(e, out_tx).await,
            client_stream_message::Message::Heartbeat(_) => {
                // A client heartbeat is liveness only; the server's own heartbeats
                // are emitted by the tick loop. Nothing to do.
                true
            }
            client_stream_message::Message::MarketSeriesSubscribe(s) => {
                self.handle_series_subscribe(s, out_tx).await
            }
            client_stream_message::Message::MarketSeriesUnsubscribe(u) => {
                if let Some(id) = u.subscription {
                    self.series.remove(&id.value);
                }
                true
            }
        }
    }

    /// Open a new subscription: read the baseline market (honouring any pinned
    /// surface version), send the baseline snapshot at sequence 1.
    async fn handle_subscribe(
        &mut self,
        s: celnet_proto::Subscribe,
        out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
    ) -> bool {
        let Some(id) = s.subscription else {
            return true;
        };
        let Some(instrument) = s.instrument else {
            return true;
        };
        let conv = match s.conventions.as_ref().map(decode_conv) {
            Some(Ok(c)) => c,
            _ => {
                let _ = out_tx
                    .send(Err(Status::invalid_argument(
                        "subscribe missing/invalid conventions",
                    )))
                    .await;
                return true;
            }
        };
        let pinned = match self.baseline_market(&instrument, s.surface_version).await {
            Ok(p) => p,
            Err(status) => {
                let _ = out_tx.send(Err(status)).await;
                return true;
            }
        };
        let market = pinned.market;
        // Subscribe to the instrument's pair on the shared per-pair price-tick ring
        // (the 1-producer → N-consumers fan-out). The pair's producer is created
        // lazily on first subscribe, seeded from this baseline market. An instrument
        // without a pair cannot be fanned out — a hard subscribe error (better than
        // opening a line that can never tick), never a fabricated stream.
        let tick = match instrument.underlying.as_ref().and_then(|u| u.as_fx()) {
            Some(wire_pair) => match self.fanout.subscribe(wire_pair, market) {
                Some(consumer) => consumer,
                None => {
                    let _ = out_tx
                        .send(Err(Status::unavailable(
                            "price fan-out unavailable (edge draining)",
                        )))
                        .await;
                    return true;
                }
            },
            None => {
                let _ = out_tx
                    .send(Err(Status::invalid_argument(
                        "subscribe instrument requires a `pair`",
                    )))
                    .await;
                return true;
            }
        };
        let mut sub = Subscription {
            id,
            instrument,
            conv,
            base_market: market,
            surface_version: pinned.echo_version,
            correlation_id: s.correlation_id,
            // Stamp the who's-trading chain: the maker auto-pricer quotes the
            // streamed line; the client's requesting seat (when supplied) holds a
            // click-to-trade fill. So a streamed line and its fill are attributable.
            attribution: Some(super::attribution::resolve(s.attribution.as_ref())),
            sequence: 1,
            delivered: 0,
            lagged: false,
            lag_notified: false,
            tick,
            last_market: market,
            replay: VecDeque::new(),
            tokens: TokenLedger::new(),
            execute_idempotency: HashMap::new(),
            latency: LatencyRecorder::new(),
        };
        let snap = match make_snapshot(
            &mut sub,
            1,
            &market,
            &self.spread,
            &self.minter,
            &self.clock,
        ) {
            Ok(m) => m,
            Err(status) => {
                let _ = out_tx.send(Err(status)).await;
                return true;
            }
        };
        sub.retain(1, snap.clone());
        // Blocking send for the snapshot so the baseline is never dropped.
        if out_tx.send(Ok(snap)).await.is_err() {
            return false;
        }
        sub.delivered = 1;
        self.subs.insert(id.value, sub);
        true
    }

    /// Modify a live subscription in place: replace its instrument / conventions /
    /// throttle / surface pin and re-baseline with a fresh snapshot at the next
    /// sequence (so the client rebaselines exactly as on Subscribe / Resync). The
    /// prior sequence's tokens are retired by the fresh snapshot's mint.
    async fn handle_modify(
        &mut self,
        m: celnet_proto::Modify,
        out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
    ) -> bool {
        let Some(id) = m.subscription else {
            return true;
        };
        if !self.subs.contains_key(&id.value) {
            // A modify on an unknown subscription is a client error, surfaced and
            // ignored (the session and its other subscriptions are unaffected).
            let _ = out_tx
                .send(Err(Status::not_found(format!(
                    "modify on unknown subscription {}",
                    id.value
                ))))
                .await;
            return true;
        }
        let Some(instrument) = m.instrument else {
            return true;
        };
        let conv = match m.conventions.as_ref().map(decode_conv) {
            Some(Ok(c)) => c,
            _ => {
                let _ = out_tx
                    .send(Err(Status::invalid_argument(
                        "modify missing/invalid conventions",
                    )))
                    .await;
                return true;
            }
        };
        let pinned = match self.baseline_market(&instrument, m.surface_version).await {
            Ok(p) => p,
            Err(status) => {
                let _ = out_tx.send(Err(status)).await;
                return true;
            }
        };
        let market = pinned.market;
        // Re-subscribe to the (possibly new) pair's ring before taking the mutable
        // subscription borrow (avoids a double borrow of `self`). The modified
        // structure may name a different pair, so the line follows that pair's tick
        // stream from now on.
        let tick = match instrument.underlying.as_ref().and_then(|u| u.as_fx()) {
            Some(wire_pair) => match self.fanout.subscribe(wire_pair, market) {
                Some(consumer) => consumer,
                None => {
                    let _ = out_tx
                        .send(Err(Status::unavailable(
                            "price fan-out unavailable (edge draining)",
                        )))
                        .await;
                    return true;
                }
            },
            None => {
                let _ = out_tx
                    .send(Err(Status::invalid_argument(
                        "modify instrument requires a `pair`",
                    )))
                    .await;
                return true;
            }
        };
        let sub = self.subs.get_mut(&id.value).expect("checked present");
        // Re-baseline the subscription in place at the next sequence.
        sub.instrument = instrument;
        sub.conv = conv;
        sub.base_market = market;
        sub.surface_version = pinned.echo_version;
        sub.lagged = false;
        sub.tick = tick;
        sub.last_market = market;
        // Modifying retires the prior structure's tradable tokens (they reference a
        // line that no longer exists). Consumed tokens stay recorded so a late
        // replayed Execute is still rejected as already-consumed.
        sub.tokens.clear_live();
        let seq = sub.sequence + 1;
        sub.sequence = seq;
        let snap = match make_snapshot(sub, seq, &market, &self.spread, &self.minter, &self.clock) {
            Ok(msg) => msg,
            Err(status) => {
                let _ = out_tx.send(Err(status)).await;
                return true;
            }
        };
        sub.retain(seq, snap.clone());
        if out_tx.send(Ok(snap)).await.is_err() {
            return false;
        }
        sub.delivered = seq;
        true
    }

    /// Handle a click-to-trade [`Execute`]: validate the token is live (within its
    /// `valid_until_nanos`, known, unconsumed) and book an [`Executed`], or decline
    /// with a [`StreamReject`] (expired / unknown / already-consumed). Idempotent on
    /// the client key.
    async fn handle_execute(
        &mut self,
        e: Execute,
        out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
    ) -> bool {
        let Some(id) = e.subscription else {
            return true;
        };
        let now = self.clock.now_nanos();
        let exec_id_counter = Arc::clone(&self.next_execution_id);
        let Some(sub) = self.subs.get_mut(&id.value) else {
            // An Execute on an unknown subscription is declined as unknown-token.
            let reject = ServerStreamMessage {
                message: Some(server_stream_message::Message::StreamReject(StreamReject {
                    subscription: Some(id),
                    token: e.token,
                    reason: stream_reject::Reason::UnknownToken as i32,
                    correlation_id: e.correlation_id,
                    epoch_nanos: now,
                })),
            };
            return out_tx.send(Ok(reject)).await.is_ok();
        };

        // Idempotency: an Execute retry carrying a key already booked returns the
        // same Executed rather than double-booking.
        if !e.idempotency_key.is_empty()
            && let Some(prev) = sub.execute_idempotency.get(&e.idempotency_key)
        {
            let msg = ServerStreamMessage {
                message: Some(server_stream_message::Message::Executed(prev.clone())),
            };
            return out_tx.send(Ok(msg)).await.is_ok();
        }

        // Resolve the token. A reject never books.
        let reject = |reason: stream_reject::Reason| ServerStreamMessage {
            message: Some(server_stream_message::Message::StreamReject(StreamReject {
                subscription: Some(id),
                token: e.token,
                reason: reason as i32,
                correlation_id: e.correlation_id,
                epoch_nanos: now,
            })),
        };

        // Present the token to the SHARED last-look ledger (the same try_book the FIX
        // acceptor lift uses): already-consumed (replay) → unknown/forged → expired →
        // book. On a successful book the token is marked consumed (bounded,
        // expiry-evicting) and retired so a second lift rejects as already-consumed.
        let (side, premium) = match sub.tokens.try_book(e.token, now) {
            BookOutcome::AlreadyConsumed => {
                return out_tx
                    .send(Ok(reject(stream_reject::Reason::AlreadyConsumed)))
                    .await
                    .is_ok();
            }
            BookOutcome::UnknownToken => {
                return out_tx
                    .send(Ok(reject(stream_reject::Reason::UnknownToken)))
                    .await
                    .is_ok();
            }
            BookOutcome::Expired => {
                return out_tx
                    .send(Ok(reject(stream_reject::Reason::Expired)))
                    .await
                    .is_ok();
            }
            BookOutcome::Booked { side, premium } => (side, premium),
        };

        let execution_id = exec_id_counter.fetch_add(1, Ordering::Relaxed);
        let executed = Executed {
            subscription: Some(id),
            token: e.token,
            execution_id,
            side: side as i32,
            traded_premium: premium,
            correlation_id: e.correlation_id,
            epoch_nanos: now,
            // Resolve the fill's attribution from the subscription's chain so a
            // click-to-trade fill feeds the who's-trading roll-up.
            attribution: sub.attribution.clone(),
        };
        if !e.idempotency_key.is_empty() {
            sub.execute_idempotency
                .insert(e.idempotency_key.clone(), executed.clone());
        }

        // Record the booked vanilla position into the shared live book so
        // `RiskService` aggregates exactly the lines this stream trades (API-first
        // parity: the Book/Risk views read the server's aggregate of this book). A
        // non-vanilla fill has no canonical-vanilla risk leaf, so it is not recorded
        // (honest scope — never a faked vanilla risk). Booking is best-effort: a
        // failure to record a risk fact must never fail the trade itself.
        if let Some(store) = self.store.clone() {
            record_booked_position(&store, sub, &self.spread, execution_id, side);
        }

        let msg = ServerStreamMessage {
            message: Some(server_stream_message::Message::Executed(executed)),
        };
        out_tx.send(Ok(msg)).await.is_ok()
    }

    /// Handle a client [`celnet_proto::Resync`] for one subscription: re-establish
    /// it (clearing any recoverable lagged state) and replay each retained message
    /// *exactly once*, strictly after the client's last good sequence.
    async fn handle_resync(
        &mut self,
        sub_id: u64,
        last_sequence: u64,
        out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
    ) -> bool {
        let spread = self.spread;
        let clock = self.clock.clone();
        // Borrow the subscription; the minter is borrowed immutably alongside.
        let minter = &self.minter;
        let sub = self.subs.get_mut(&sub_id).expect("checked present");
        sub.lagged = false;

        let have_replay = sub
            .replay
            .front()
            .map(|(seq, _)| *seq <= last_sequence + 1)
            .unwrap_or(false);
        if have_replay {
            let to_replay: Vec<ServerStreamMessage> = sub
                .replay
                .iter()
                .filter(|(seq, _)| *seq > last_sequence)
                .map(|(_, m)| m.clone())
                .collect();
            for m in to_replay {
                if out_tx.send(Ok(m)).await.is_err() {
                    return false;
                }
            }
            sub.delivered = sub.sequence;
        } else {
            let seq = sub.sequence + 1;
            sub.sequence = seq;
            // The gap predates the retained buffer, so send a fresh snapshot priced
            // on the last on-path market this subscription streamed (the most recent
            // per-pair tick it delivered), so the client never holds an off-path
            // baseline. Subsequent live updates continue from the next ring tick.
            let market = sub.last_market;
            let snap = match make_snapshot(sub, seq, &market, &spread, minter, &clock) {
                Ok(m) => m,
                Err(status) => {
                    let _ = out_tx.send(Err(status)).await;
                    return true;
                }
            };
            sub.retain(seq, snap.clone());
            if out_tx.send(Ok(snap)).await.is_err() {
                return false;
            }
            sub.delivered = seq;
        }
        true
    }

    /// Open a market-series (TrendMode) subscription: validate the observable +
    /// wing/tenor presence rules, read the opening value off the *live* market
    /// state, and send a [`MarketSeriesSnapshot`] seeding the series (sequence 1)
    /// with a single freshly-observed point. Subsequent points are appended by the
    /// tick loop ([`Session::drive_market_series`]) as the live state moves.
    async fn handle_series_subscribe(
        &mut self,
        s: MarketSeriesSubscribe,
        out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
    ) -> bool {
        let Some(id) = s.subscription else {
            return true;
        };
        let Some(underlying) = s.underlying.clone() else {
            let _ = out_tx
                .send(Err(Status::invalid_argument(
                    "market-series subscribe requires an `underlying`",
                )))
                .await;
            return true;
        };
        let observable = match decode_observable(s.observable, s.delta) {
            Ok(o) => o,
            Err(status) => {
                let _ = out_tx.send(Err(status)).await;
                return true;
            }
        };
        // Read the opening value off the live state. A `None` (degenerate wing) is a
        // hard subscribe error — better than opening a series that can never emit.
        let value = match self.link.observe(ObservableQuery { observable }).await {
            Ok(Some(v)) => v,
            Ok(None) => {
                let _ = out_tx
                    .send(Err(Status::failed_precondition(
                        "market observable not derivable from the live state (degenerate wing)",
                    )))
                    .await;
                return true;
            }
            Err(e) => {
                let _ = out_tx.send(Err(Status::unavailable(e.to_string()))).await;
                return true;
            }
        };
        let now = self.clock.now_nanos();
        let first = MarketSeriesPoint {
            subscription: Some(id),
            sequence: 1,
            value,
            epoch_nanos: now,
        };
        let series = MarketSeries {
            id,
            observable,
            throttle_nanos: s.throttle_nanos as i64,
            sequence: 1,
            last_emit_nanos: now,
        };
        let snapshot = ServerStreamMessage {
            message: Some(server_stream_message::Message::MarketSeriesSnapshot(
                MarketSeriesSnapshot {
                    subscription: Some(id),
                    sequence: 1,
                    underlying: Some(underlying),
                    observable: s.observable,
                    points: vec![first],
                    epoch_nanos: now,
                },
            )),
        };
        // Blocking send so the baseline snapshot is never dropped.
        if out_tx.send(Ok(snapshot)).await.is_err() {
            return false;
        }
        self.series.insert(id.value, series);
        true
    }

    /// Sample every live market series off the current live market state and append
    /// a fresh [`MarketSeriesPoint`] to each that is due (its conflation throttle has
    /// elapsed). Returns `false` only when the channel has closed (client gone).
    ///
    /// The value is derived on the core thread from the same live state the hot
    /// path prices against (never fabricated): one `observe` read per due series.
    /// A lagging consumer drops a point via `try_send` (the next due sample carries
    /// the then-current value), so a slow series never back-pressures the driver.
    async fn drive_market_series(
        &mut self,
        out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
    ) -> bool {
        if self.series.is_empty() {
            return true;
        }
        let now = self.clock.now_nanos();
        for series in self.series.values_mut() {
            // Conflate: only sample when the throttle window has elapsed.
            if series.throttle_nanos > 0 && now - series.last_emit_nanos < series.throttle_nanos {
                continue;
            }
            let value = match self
                .link
                .observe(ObservableQuery {
                    observable: series.observable,
                })
                .await
            {
                Ok(Some(v)) => v,
                // A momentarily-underivable observable simply skips this sample
                // rather than emitting a fabricated or stale point.
                Ok(None) => continue,
                Err(_) => return false, // core gone.
            };
            let seq = series.sequence + 1;
            let point = MarketSeriesPoint {
                subscription: Some(series.id),
                sequence: seq,
                value,
                epoch_nanos: now,
            };
            let msg = ServerStreamMessage {
                message: Some(server_stream_message::Message::MarketSeriesPoint(point)),
            };
            match out_tx.try_send(Ok(msg)) {
                Ok(()) => {
                    series.sequence = seq;
                    series.last_emit_nanos = now;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Lagged: drop this point (a market observable is conflatable;
                    // the next due sample carries the then-current value). Never
                    // back-pressure the driver.
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return false,
            }
        }
        true
    }

    /// Drive a market-tick pass across every live subscription. Each subscription
    /// drains its **per-pair price-tick ring** (the shared `celnet-fanout` SPMC
    /// ring, the 1-producer → N-consumers fan-out) of every tick available this
    /// cadence, **conflating to the latest** and emitting **at most one** sequenced
    /// `Update` per pass. Returns `false` only when the outbound channel has closed.
    ///
    /// **Conflation hint / throttle (preserved).** The previous per-subscription
    /// ticker advanced exactly one synthetic spot per cadence pass and emitted one
    /// `Update`; the edge's one-`Update`-per-pass cadence is a deliberate client
    /// conflation throttle (a live RFS line shows the *current* two-way, not a
    /// backlog of stale intermediates, and a token lives a full cadence so a click
    /// can land). The ring preserves that exactly: it delivers the per-pair ticks
    /// **in order, never torn, never duplicated**, but the driver **conflates the
    /// pass to the most recent tick** (the producer typically advances faster than
    /// the cadence) and emits a single `Update` carrying it. When a consumer falls
    /// more than the ring's capacity behind, the ring's own latest-value conflation
    /// (with exact `received + skipped == produced` accounting) also engages — so a
    /// momentarily-slow session always converges on the latest market, the
    /// FX-streaming-correct "a stale quote is worse than a skipped one" policy.
    fn drive_tick(&mut self, out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>) -> bool {
        let spread = self.spread;
        let clock = self.clock.clone();
        let minter = &self.minter;
        for sub in self.subs.values_mut() {
            if sub.lagged {
                if !sub.lag_notified && out_tx.is_closed() {
                    return false;
                }
                notify_lagged(sub, out_tx);
                continue;
            }
            // Conflate the pass to the LATEST available tick: drain the ring (each
            // `try_recv` is in-order; the ring conflates internally if we were
            // lapped) and keep only the most recent market. Emit at most one Update.
            let mut latest: Option<MarketContext> = None;
            while let Ok(tick) = sub.tick.try_recv() {
                latest = Some(tick.market);
            }
            if let Some(market) = latest {
                match drive_one_update(sub, market, &spread, minter, &clock, out_tx) {
                    UpdateOutcome::Emitted | UpdateOutcome::Lagged => {}
                    UpdateOutcome::Closed => return false,
                }
            }
        }
        true
    }
}

/// The outcome of attempting to emit one `Update` to a subscriber.
enum UpdateOutcome {
    /// The update was delivered; the per-subscription sequence advanced.
    Emitted,
    /// The outbound channel is closed (client gone) — tear the session down.
    Closed,
    /// The subscriber lagged (outbound channel full / a price error); it is parked
    /// in the recoverable lagged state awaiting a Resync.
    Lagged,
}

/// Emit one sequenced `Update` for `sub` priced against `market` (one drained
/// per-pair tick), stamping fresh click-to-trade tokens and interleaving a
/// periodic `Heartbeat`. Advances the per-subscription sequence and retains the
/// message for Resync replay only on a successful emit.
fn drive_one_update(
    sub: &mut Subscription,
    market: MarketContext,
    spread: &SpreadModel,
    minter: &TokenMinter,
    clock: &Clock,
    out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
) -> UpdateOutcome {
    let seq = sub.sequence + 1;
    debug_assert!(
        seq > sub.delivered,
        "live tick must advance past the last delivered sequence"
    );
    let update = match make_update(sub, seq, &market, spread, minter, clock) {
        Ok(m) => m,
        Err(status) => {
            let _ = out_tx.try_send(Err(status));
            if out_tx.is_closed() {
                return UpdateOutcome::Closed;
            }
            mark_lagged(sub, out_tx);
            return UpdateOutcome::Lagged;
        }
    };
    if try_emit(out_tx, update.clone()) {
        sub.sequence = seq;
        sub.delivered = seq;
        sub.last_market = market;
        sub.retain(seq, update);
        if seq.is_multiple_of(HEARTBEAT_EVERY) {
            let hb = ServerStreamMessage {
                message: Some(server_stream_message::Message::Heartbeat(heartbeat_for(
                    sub, seq, clock,
                ))),
            };
            let _ = out_tx.try_send(Ok(hb));
        }
        UpdateOutcome::Emitted
    } else if out_tx.is_closed() {
        UpdateOutcome::Closed
    } else {
        // The dropped `seq` was neither advanced nor retained, and the
        // just-minted tokens (in `live_tokens`) are about to be retired by
        // the next emitted line, so no gap, duplicate, or stale token leaks.
        mark_lagged(sub, out_tx);
        UpdateOutcome::Lagged
    }
}

/// Build the observability-bearing [`Heartbeat`] for a subscription: liveness
/// (subscription + sequence + send time) plus the additive server observability —
/// the exact `celnet-fanout` ring conflation-drop count (`skipped()`), the
/// drain-side price-compute p50/p99/p99.9 (ns), and the surface-version /
/// correlation-id provenance echo. Every figure is a real measurement taken off
/// the streaming edge; the pinned hot core is never touched.
fn heartbeat_for(sub: &Subscription, seq: u64, clock: &Clock) -> Heartbeat {
    Heartbeat {
        subscription: Some(sub.id),
        sequence: seq,
        epoch_nanos: clock.now_nanos(),
        // The exact number of ticks this subscription's ring conflated (dropped)
        // since it was opened — the `received + skipped == produced` skip count.
        conflation_drops: sub.tick.skipped(),
        // Drain-side price-compute latency percentiles (0 until first timed).
        server_price_p50_nanos: sub.latency.p50_ns(),
        server_price_p99_nanos: sub.latency.p99_ns(),
        server_price_p999_nanos: sub.latency.p999_ns(),
        // Provenance echo: pinned surface version (0 ⇒ live mark) + correlation id.
        surface_version: sub.surface_version.unwrap_or(0),
        correlation_id: sub.correlation_id.unwrap_or(0),
    }
}

/// Move a subscription into the recoverable lagged state. The subscription stays
/// registered so a later Resync re-establishes it from the retained buffer.
fn mark_lagged(sub: &mut Subscription, out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>) {
    if sub.lagged {
        return;
    }
    sub.lagged = true;
    sub.lag_notified = false;
    notify_lagged(sub, out_tx);
}

/// Attempt to deliver the `LAGGED` [`StreamEnd`] marker prompting the client to
/// [`celnet_proto::Resync`]. Idempotent: once the marker lands, `lag_notified` is
/// set and further calls are no-ops.
fn notify_lagged(
    sub: &mut Subscription,
    out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
) {
    if sub.lag_notified {
        return;
    }
    let end = ServerStreamMessage {
        message: Some(server_stream_message::Message::StreamEnd(StreamEnd {
            subscription: Some(sub.id),
            reason: stream_end::Reason::Lagged as i32,
        })),
    };
    if out_tx.try_send(Ok(end)).is_ok() {
        sub.lag_notified = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_conv() -> Conventions {
        Conventions {
            delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
            atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
            premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
            cut: celnet_proto::Cut::NewYork1000 as i32,
            day_count: celnet_proto::DayCount::Act365Fixed as i32,
            settlement: celnet_proto::Settlement::Deliverable as i32,
        }
    }

    fn vanilla_call(strike: f64) -> Instrument {
        Instrument {
            underlying: Some(celnet_proto::Underlying::fx(celnet_proto::CcyPair {
                base: "EUR".to_owned(),
                quote: "USD".to_owned(),
            })),
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

    /// Build a session over a calibrated EURUSD fixture, with one open subscription
    /// (baseline snapshot at sequence 1 already retained + delivered), ready to be
    /// driven by `drive_tick` and to receive `Execute`s.
    fn make_session(clock: Clock) -> Session {
        let market = MarketContext::fx(1.10, 0.10, 0.02, 0.01);
        let conv = ConventionSet::decode(&wire_conv()).expect("conventions decode");
        let link = CoreLink::start(
            celnet_engine::testing::make_state(
                1.10,
                celnet_conventions::resolve(
                    celnet_types::CcyPair::parse("EURUSD").unwrap(),
                    celnet_types::Tenor::Years(1),
                )
                .record,
            ),
            None,
        );
        let fanout = PriceFanout::start();
        let tick = fanout
            .subscribe(
                &celnet_proto::CcyPair {
                    base: "EUR".to_owned(),
                    quote: "USD".to_owned(),
                },
                market,
            )
            .expect("the EURUSD price-tick ring");
        let mut session = Session {
            subs: HashMap::new(),
            series: HashMap::new(),
            link,
            spread: SpreadModel::default(),
            clock: clock.clone(),
            surface_book: Arc::new(SurfaceBook::new()),
            minter: TokenMinter::new(),
            next_execution_id: Arc::new(AtomicU64::new(1)),
            store: None,
            fanout,
        };
        let mut sub = Subscription {
            id: SubscriptionId { value: 1 },
            instrument: vanilla_call(1.10),
            conv,
            base_market: market,
            surface_version: None,
            correlation_id: None,
            attribution: None,
            sequence: 1,
            delivered: 1,
            lagged: false,
            lag_notified: false,
            tick,
            last_market: market,
            replay: VecDeque::new(),
            tokens: TokenLedger::new(),
            execute_idempotency: HashMap::new(),
            latency: LatencyRecorder::new(),
        };
        let snap = make_snapshot(
            &mut sub,
            1,
            &market,
            &session.spread,
            &session.minter,
            &clock,
        )
        .unwrap();
        sub.retain(1, snap);
        session.subs.insert(1, sub);
        session
    }

    fn seq_of(msg: &ServerStreamMessage) -> Option<u64> {
        match msg.message.as_ref()? {
            server_stream_message::Message::Snapshot(s) => Some(s.sequence),
            server_stream_message::Message::Update(u) => Some(u.sequence),
            _ => None,
        }
    }

    fn snapshot_tokens(msg: &ServerStreamMessage) -> Vec<TradableToken> {
        match msg.message.as_ref() {
            Some(server_stream_message::Message::Snapshot(s)) => s.tradable.clone(),
            _ => Vec::new(),
        }
    }

    /// The heartbeat's `conflation_drops` reflects the EXACT number of price ticks
    /// the subscription's `celnet-fanout` ring conflated (dropped) under
    /// back-pressure — the real `received + skipped == produced` skip accounting,
    /// not a fabricated metric. We drive a genuine over-publish into a bounded ring,
    /// drain once (so the lapped consumer conflates forward and counts the gap), then
    /// assert the heartbeat carries precisely that skip count.
    #[tokio::test]
    async fn heartbeat_reports_real_ring_conflation_drops() {
        tokio::time::timeout(Duration::from_secs(10), async {
            use celnet_fanout::BroadcastRing;

            let clock = Clock::manual(1_000_000_000);
            let mut session = make_session(clock.clone());
            let market = MarketContext::fx(1.10, 0.10, 0.02, 0.01);

            // A bounded ring with a tiny capacity, its own producer + the consumer the
            // subscription will drain. (A power-of-two capacity is required.)
            let cap = 8usize;
            let ring = BroadcastRing::<PriceTick>::new(cap);
            let consumer = ring.consumer();
            let mut producer = ring.into_producer();

            // Install this consumer on the live subscription, replacing the session's.
            {
                let sub = session.subs.get_mut(&1).unwrap();
                sub.tick = consumer;
                assert_eq!(sub.tick.skipped(), 0, "fresh consumer has skipped nothing");
            }

            // Over-publish FAR past capacity WITHOUT draining: the producer laps the
            // consumer many times over (genuine back-pressure on the bounded ring).
            let produced = 100u64;
            for i in 0..produced {
                let tick = PriceTick {
                    market: market.with_spot(1.10 + f64::from(u32::try_from(i).unwrap()) * 1e-6),
                    tick_seq: i,
                };
                producer.publish(tick);
            }

            // Drive the subscription once: `drive_tick` drains the ring (conflating
            // forward to the newest live tick) and emits one Update, then — because
            // this happens to be a heartbeat sequence or not — we read the skip count.
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(64);
            assert!(session.drive_tick(&tx), "drive_tick succeeds");

            let sub = session.subs.get(&1).unwrap();
            let skipped = sub.tick.skipped();
            let received = sub.tick.received();
            // The ring conflated: the consumer saw far fewer than were produced, and
            // the skip count is the exact gap (received + skipped == produced-observed).
            assert!(
                skipped > 0,
                "the lapped consumer must have counted real skips"
            );
            assert!(
                received >= 1,
                "the consumer delivered at least the newest live tick"
            );
            assert!(
                received + skipped <= produced,
                "received + skipped is bounded by produced (exact ring accounting)"
            );

            // The heartbeat built for this subscription carries EXACTLY that skip count
            // — the wire field equals the ring's own accounting, no fabrication.
            let hb = heartbeat_for(sub, sub.sequence, &clock);
            assert_eq!(
                hb.conflation_drops, skipped,
                "heartbeat.conflation_drops must equal the ring's exact skip count"
            );
            // Provenance echo: unpinned ⇒ surface_version 0, no correlation ⇒ 0.
            assert_eq!(hb.surface_version, 0);
            assert_eq!(hb.correlation_id, 0);

            // Drain any emitted updates so the channel does not back up.
            while rx.try_recv().is_ok() {}
        })
        .await
        .expect("no hang");
    }

    /// The heartbeat surfaces a real, non-zero server-side price-compute latency once
    /// updates have been timed, and echoes the pinned surface version + correlation id
    /// provenance. The latency comes from the drain-side HdrHistogram fed in
    /// `make_update` — an honest measurement off the streaming edge.
    #[tokio::test]
    async fn heartbeat_reports_real_price_latency_and_provenance_echo() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let mut session = make_session(clock.clone());
            // Pin a surface version + correlation id so the echo is non-trivial.
            {
                let sub = session.subs.get_mut(&1).unwrap();
                sub.surface_version = Some(42);
                sub.correlation_id = Some(0xABCD);
            }

            // Time several updates so the per-subscription histogram is populated.
            let market = MarketContext::fx(1.10, 0.10, 0.02, 0.01);
            let spread = SpreadModel::default();
            let minter = TokenMinter::new();
            {
                let sub = session.subs.get_mut(&1).unwrap();
                for seq in 2..=40u64 {
                    let _ = make_update(sub, seq, &market, &spread, &minter, &clock)
                        .expect("update prices");
                }
                assert!(
                    sub.latency.count() >= 1,
                    "the drain-side histogram recorded real compute samples"
                );
            }

            let sub = session.subs.get(&1).unwrap();
            let hb = heartbeat_for(sub, sub.sequence, &clock);
            // Latency percentiles are real, monotone, and non-zero (compute is timed).
            assert!(
                hb.server_price_p50_nanos > 0,
                "a timed compute yields a non-zero p50"
            );
            assert!(hb.server_price_p50_nanos <= hb.server_price_p99_nanos);
            assert!(hb.server_price_p99_nanos <= hb.server_price_p999_nanos);
            // Provenance echo carries the pinned version + correlation id.
            assert_eq!(hb.surface_version, 42);
            assert_eq!(hb.correlation_id, 0xABCD);
        })
        .await
        .expect("no hang");
    }

    /// A snapshot stamps two click-to-trade tokens (SELL@bid, BUY@offer) whose
    /// premiums match the streamed two-way, and they are registered live. (The
    /// keyed-MAC unforgeability / field-binding invariants are proven in
    /// [`crate::services::clicktrade`], the single source of truth for the token.)
    #[tokio::test]
    async fn snapshot_stamps_live_tradable_tokens() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let session = make_session(clock);
            let sub = session.subs.get(&1).unwrap();
            assert_eq!(sub.tokens.live_len(), 2, "SELL@bid + BUY@offer");
            let buy = sub
                .tokens
                .live_tokens()
                .map(|(_, t)| t)
                .find(|t| t.side == Side::Buy)
                .expect("a BUY token");
            assert!(buy.premium > 0.0, "buy books the offer");
        })
        .await
        .expect("no hang");
    }

    /// A click-to-trade Execute on a live token books an Executed at the stamped
    /// premium; a forged token, an expired token, and a second click are each
    /// rejected with the right reason.
    #[tokio::test]
    async fn execute_books_then_rejects_forged_expired_consumed() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let mut session = make_session(clock.clone());
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(64);

            // Grab a live BUY token from the snapshot.
            let snap = session
                .subs
                .get(&1)
                .unwrap()
                .replay
                .front()
                .unwrap()
                .1
                .clone();
            let tokens = snapshot_tokens(&snap);
            let buy = tokens
                .iter()
                .find(|t| t.side == Side::Buy as i32)
                .expect("a BUY token");

            // Forged token → UnknownToken.
            assert!(
                session
                    .handle_execute(
                        Execute {
                            subscription: Some(SubscriptionId { value: 1 }),
                            token: 0xDEAD_BEEF,
                            idempotency_key: "x1".to_owned(),
                            correlation_id: Some(7),
                        },
                        &tx,
                    )
                    .await
            );
            let m = rx.try_recv().unwrap().unwrap();
            assert!(matches!(
                m.message,
                Some(server_stream_message::Message::StreamReject(r))
                    if r.reason == stream_reject::Reason::UnknownToken as i32
            ));

            // The genuine BUY token books an Executed at the stamped premium.
            assert!(
                session
                    .handle_execute(
                        Execute {
                            subscription: Some(SubscriptionId { value: 1 }),
                            token: buy.token,
                            idempotency_key: "click-1".to_owned(),
                            correlation_id: Some(7),
                        },
                        &tx,
                    )
                    .await
            );
            let booked = match rx.try_recv().unwrap().unwrap().message {
                Some(server_stream_message::Message::Executed(e)) => e,
                other => panic!("expected Executed, got {other:?}"),
            };
            assert_eq!(booked.side, Side::Buy as i32);
            assert_eq!(booked.traded_premium.to_bits(), buy.premium.to_bits());
            assert_eq!(booked.correlation_id, Some(7));

            // Idempotent retry (same key) returns the SAME Executed (no double book).
            assert!(
                session
                    .handle_execute(
                        Execute {
                            subscription: Some(SubscriptionId { value: 1 }),
                            token: buy.token,
                            idempotency_key: "click-1".to_owned(),
                            correlation_id: Some(7),
                        },
                        &tx,
                    )
                    .await
            );
            let retry = match rx.try_recv().unwrap().unwrap().message {
                Some(server_stream_message::Message::Executed(e)) => e,
                other => panic!("expected idempotent Executed, got {other:?}"),
            };
            assert_eq!(retry.execution_id, booked.execution_id);

            // A second click on the consumed token with a DIFFERENT key → AlreadyConsumed.
            assert!(
                session
                    .handle_execute(
                        Execute {
                            subscription: Some(SubscriptionId { value: 1 }),
                            token: buy.token,
                            idempotency_key: "click-2".to_owned(),
                            correlation_id: None,
                        },
                        &tx,
                    )
                    .await
            );
            let m = rx.try_recv().unwrap().unwrap();
            assert!(matches!(
                m.message,
                Some(server_stream_message::Message::StreamReject(r))
                    if r.reason == stream_reject::Reason::AlreadyConsumed as i32
            ));

            // Mint a fresh line, then advance the clock past the token window: an
            // Execute on the (now expired) token is rejected as Expired.
            let upd = make_update(
                session.subs.get_mut(&1).unwrap(),
                2,
                &MarketContext::fx(1.101, 0.10, 0.02, 0.01),
                &session.spread,
                &session.minter,
                &clock,
            )
            .unwrap();
            let fresh = snapshot_tokens_update(&upd);
            let fresh_buy = fresh.iter().find(|t| t.side == Side::Buy as i32).unwrap();
            clock.advance(TOKEN_VALIDITY_NANOS + 1);
            assert!(
                session
                    .handle_execute(
                        Execute {
                            subscription: Some(SubscriptionId { value: 1 }),
                            token: fresh_buy.token,
                            idempotency_key: "late".to_owned(),
                            correlation_id: None,
                        },
                        &tx,
                    )
                    .await
            );
            let m = rx.try_recv().unwrap().unwrap();
            assert!(matches!(
                m.message,
                Some(server_stream_message::Message::StreamReject(r))
                    if r.reason == stream_reject::Reason::Expired as i32
            ));
        })
        .await
        .expect("no hang");
    }

    fn snapshot_tokens_update(msg: &ServerStreamMessage) -> Vec<TradableToken> {
        match msg.message.as_ref() {
            Some(server_stream_message::Message::Update(u)) => u.tradable.clone(),
            _ => Vec::new(),
        }
    }

    /// A Modify re-baselines the subscription in place with a fresh Snapshot at the
    /// next sequence carrying the new structure, retiring the prior tokens.
    #[tokio::test]
    async fn modify_rebaselines_in_place_at_next_sequence() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let mut session = make_session(clock);
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(64);
            // The open subscription is at sequence 1.
            let prior_token = session
                .subs
                .get(&1)
                .unwrap()
                .tokens
                .any_live_token()
                .unwrap();

            assert!(
                session
                    .handle_modify(
                        celnet_proto::Modify {
                            subscription: Some(SubscriptionId { value: 1 }),
                            instrument: Some(vanilla_call(1.20)),
                            conventions: Some(wire_conv()),
                            throttle_nanos: 0,
                            surface_version: None,
                        },
                        &tx,
                    )
                    .await
            );
            let msg = rx.try_recv().unwrap().unwrap();
            let snap = match msg.message {
                Some(server_stream_message::Message::Snapshot(s)) => s,
                other => panic!("modify must answer with a fresh Snapshot, got {other:?}"),
            };
            assert_eq!(snap.sequence, 2, "modify re-baselines at the next sequence");
            // The prior token is retired (no longer live for the new structure).
            assert!(
                !session.subs.get(&1).unwrap().tokens.is_live(prior_token),
                "modify retires the prior structure's tokens"
            );
        })
        .await
        .expect("no hang");
    }

    /// `drive_tick` drains the per-pair price-tick ring and emits one sequenced
    /// `Update` per drained tick, advancing the per-subscription sequence strictly
    /// by one and stamping fresh tokens. (The producer thread publishes ticks
    /// asynchronously; the test drives repeatedly, deadline-bounded, until it has
    /// observed ≥2 updates past the baseline.)
    #[tokio::test]
    async fn drive_tick_advances_sequence_and_remints_tokens() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let mut session = make_session(clock);
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(256);
            let mut last = 1u64;
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            while last < 3 && std::time::Instant::now() < deadline {
                assert!(session.drive_tick(&tx));
                while let Ok(msg) = rx.try_recv() {
                    let msg = msg.unwrap();
                    if let Some(seq) = seq_of(&msg) {
                        assert_eq!(seq, last + 1, "strictly +1 (no gap, no duplicate)");
                        last = seq;
                        assert!(
                            !snapshot_tokens_update(&msg).is_empty(),
                            "update mints tokens"
                        );
                    }
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            assert!(
                last >= 3,
                "the tick advanced past the baseline (saw ≥2 updates)"
            );
        })
        .await
        .expect("no hang");
    }

    /// Scale-guardrail regression: `consumed_tokens` stays **bounded** over a long
    /// session as tokens expire. We book many click-to-trades, each consuming a fresh
    /// short-lived token, advancing the clock past each token's validity window
    /// between books. The consumed set must NOT grow with the number of books — only
    /// entries still inside their validity window are retained — proving the set is
    /// bounded by the (small) number of tokens minted within one validity window, not
    /// by total session lifetime.
    #[tokio::test]
    async fn consumed_tokens_stay_bounded_as_tokens_expire() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let mut session = make_session(clock.clone());
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(64);

            // Book a long sequence of clicks; each mints + consumes a fresh token,
            // and we step the clock past the validity window before the next book so
            // the prior consumed entry is expired (and must be evicted).
            let mut seq = 1u64;
            for _ in 0..500 {
                seq += 1;
                let upd = make_update(
                    session.subs.get_mut(&1).unwrap(),
                    seq,
                    &MarketContext::fx(1.10 + 0.0001 * seq as f64, 0.10, 0.02, 0.01),
                    &session.spread,
                    &session.minter,
                    &clock,
                )
                .unwrap();
                let token = snapshot_tokens_update(&upd)
                    .iter()
                    .find(|t| t.side == Side::Buy as i32)
                    .unwrap()
                    .token;
                assert!(
                    session
                        .handle_execute(
                            Execute {
                                subscription: Some(SubscriptionId { value: 1 }),
                                token,
                                idempotency_key: String::new(),
                                correlation_id: None,
                            },
                            &tx,
                        )
                        .await
                );
                // Drain whatever the book emitted (Executed / reject) to keep the
                // bounded channel clear.
                while rx.try_recv().is_ok() {}
                // Advance past this token's validity window so it expires before the
                // next book records its consumed entry.
                clock.advance(TOKEN_VALIDITY_NANOS + 1);
            }

            // After 500 books spread across 500 disjoint validity windows, the
            // consumed set holds at most the few tokens minted within ONE window —
            // never ~500. Generously bound it to a small constant.
            let consumed = session.subs.get(&1).unwrap().tokens.consumed_len();
            assert!(
                consumed <= 4,
                "consumed_tokens must stay bounded as tokens expire; held {consumed}"
            );
        })
        .await
        .expect("no hang");
    }

    /// A consumed token still inside its validity window blocks a replay
    /// (`AlreadyConsumed`); replay protection holds *within* the window even though
    /// expired entries are evicted.
    #[tokio::test]
    async fn consumed_replay_blocked_within_validity_window() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let mut session = make_session(clock.clone());
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(64);

            let token = session
                .subs
                .get(&1)
                .unwrap()
                .tokens
                .live_tokens()
                .find(|(_, t)| t.side == Side::Buy)
                .map(|(k, _)| *k)
                .unwrap();

            // First book succeeds.
            assert!(
                session
                    .handle_execute(
                        Execute {
                            subscription: Some(SubscriptionId { value: 1 }),
                            token,
                            idempotency_key: String::new(),
                            correlation_id: None,
                        },
                        &tx,
                    )
                    .await
            );
            let _ = rx.try_recv();

            // A replay *within the window* (clock barely advanced) is AlreadyConsumed.
            clock.advance(TOKEN_VALIDITY_NANOS / 2);
            assert!(
                session
                    .handle_execute(
                        Execute {
                            subscription: Some(SubscriptionId { value: 1 }),
                            token,
                            idempotency_key: String::new(),
                            correlation_id: None,
                        },
                        &tx,
                    )
                    .await
            );
            let m = rx.try_recv().unwrap().unwrap();
            assert!(matches!(
                m.message,
                Some(server_stream_message::Message::StreamReject(r))
                    if r.reason == stream_reject::Reason::AlreadyConsumed as i32
            ));
        })
        .await
        .expect("no hang");
    }

    /// **The live book wiring**: a click-to-trade fill on a vanilla line records the
    /// booked position into the shared `PositionStore` (the same book `RiskService`
    /// aggregates), keyed on the subscription's attribution and signed by the fill
    /// side. This is the end-to-end proof that the GUI Book/Risk views read the
    /// server's aggregate of the *actual traded book* (API-first parity).
    #[tokio::test]
    async fn click_to_trade_records_position_into_the_risk_book() {
        use crate::services::risk::store::PositionStore;

        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let store = Arc::new(PositionStore::new());
            let mut session = make_session(clock.clone());
            session.store = Some(Arc::clone(&store));
            // Attribute the subscription to a holder seat so the booked position
            // keys on a real book/trader (the who's-trading roll-up).
            session.subs.get_mut(&1).unwrap().attribution = Some(celnet_proto::AttributionRecord {
                quoted_by: Some(celnet_proto::BookId {
                    book: "AUTO-MM".to_owned(),
                    owner: Some(celnet_proto::Owner {
                        seat: Some(celnet_proto::owner::Seat::AutoPricer(
                            "celnet-auto-pricer".to_owned(),
                        )),
                    }),
                }),
                held_by: Some(celnet_proto::BookId {
                    book: "EM-VOL-1".to_owned(),
                    owner: Some(celnet_proto::Owner {
                        seat: Some(celnet_proto::owner::Seat::Trader("jdoe".to_owned())),
                    }),
                }),
                won: Some(true),
                lp_count: Some(2),
            });

            let (tx, mut _rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(64);
            let snap = session
                .subs
                .get(&1)
                .unwrap()
                .replay
                .front()
                .unwrap()
                .1
                .clone();
            let tokens = snapshot_tokens(&snap);
            // A SELL token → short the line (negative notional).
            let sell = tokens
                .iter()
                .find(|t| t.side == Side::Sell as i32)
                .expect("a SELL token");

            assert!(
                session
                    .handle_execute(
                        Execute {
                            subscription: Some(SubscriptionId { value: 1 }),
                            token: sell.token,
                            idempotency_key: "click-book".to_owned(),
                            correlation_id: None,
                        },
                        &tx,
                    )
                    .await
            );

            // The store now holds exactly one booked position.
            assert_eq!(store.len(), 1, "the fill recorded one risk position");
            let snap = store.snapshot();
            let fact = &snap.facts[0];
            // Short fill → negative base notional (the 1mm base-ccy quantity, SELL).
            assert!(
                fact.measure.position.notional_base < 0.0,
                "a SELL fill is a short position (negative notional)"
            );
            assert_eq!(
                fact.measure.position.pair,
                celnet_types::CcyPair::new(celnet_types::Ccy::EUR, celnet_types::Ccy::USD)
            );
            // The attribution chain (holder book) round-trips for the roll-up.
            let attr = snap
                .attribution_of(fact.position_id.0)
                .expect("attribution recorded");
            assert_eq!(attr.held_by.as_ref().unwrap().book, "EM-VOL-1");
        })
        .await
        .expect("no hang");
    }
}
