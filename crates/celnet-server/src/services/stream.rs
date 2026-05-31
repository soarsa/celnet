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
use crate::services::forward::route_pair;
use crate::services::pin::{PinnedVol, resolve_pinned_vol};
use crate::services::risk::federate::Fleet;
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;
use crate::tick::TickSource;

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

/// The per-tick fractional spot bump for the streaming tick source; small enough
/// to stay realistic, large enough that consecutive updates carry a real delta.
const STREAM_BUMP: f64 = 0.0005;

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
        }
    }
}

/// A live click-to-trade token stamped on a streamed line: the side and premium it
/// books and the deadline after which it is dead.
#[derive(Debug, Clone, Copy)]
struct LiveToken {
    /// BUY lifts the offer; SELL hits the bid.
    side: Side,
    /// The premium this token books (the offer for BUY, the bid for SELL).
    premium: f64,
    /// The token validity deadline (nanoseconds since the Unix epoch, UTC).
    valid_until_nanos: i64,
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
    /// Deterministic spot-tick source seeded from the subscription id.
    tick: SpotTick,
    /// Recent server messages retained for a Resync replay (seq → message).
    replay: VecDeque<(u64, ServerStreamMessage)>,
    /// The currently-live click-to-trade tokens (keyed by opaque token value). A
    /// new sequence mints fresh tokens and clears the prior ones, so a token is
    /// live only for the sequence it was stamped on.
    live_tokens: HashMap<u64, LiveToken>,
    /// Tokens already consumed by an accepted `Execute`, mapped to their validity
    /// deadline, so a replayed `Execute` (or a second click) is rejected as
    /// already-consumed rather than re-booked. **Bounded**: a consumed token only
    /// needs to block replay *within its own validity window* (after expiry the
    /// token is rejected as `Expired` regardless), so entries past their
    /// `valid_until_nanos` are evicted — the set can never grow without bound over a
    /// long session. See [`Subscription::record_consumed`].
    consumed_tokens: HashMap<u64, i64>,
    /// `Execute` idempotency: a client key → the `Executed` it booked, so an
    /// `Execute` retry carrying the same key returns the same booking.
    execute_idempotency: HashMap<String, Executed>,
}

/// A deterministic, counter-based spot perturbation for one subscription. Mirrors
/// the engine's `splitmix64` tick discipline (no wall-clock, no OS RNG) so a
/// stream is reproducible from its `(seed, base_spot)`.
struct SpotTick {
    base_spot: f64,
    seed: u64,
    counter: u64,
}

impl SpotTick {
    fn new(base_spot: f64, seed: u64) -> Self {
        Self {
            base_spot,
            seed,
            counter: 0,
        }
    }

    /// The deterministically-bumped spot for a given counter value, *without*
    /// advancing the counter. Reuses the same public-domain `splitmix64` mixer as
    /// [`crate::tick::TickSource`] so the path is bit-identical to the tick source.
    ///
    /// Centering and bumping use *separate* multiply/add (never a fused `mul_add`):
    /// FMA rounds the fused op once at a rounding not guaranteed bit-identical
    /// across targets/opt-levels, which would break the cross-target bit-stable
    /// reproduction the RFS determinism discipline promises.
    fn spot_at(&self, counter: u64) -> f64 {
        let mixed = TickSource::splitmix64(self.seed ^ counter.wrapping_mul(0x2545_F491_4F6C_DD1D));
        let u = TickSource::unit_signed(mixed);
        self.base_spot * (u * STREAM_BUMP + 1.0)
    }

    /// The next deterministically-bumped spot the upcoming live tick will use,
    /// *without* consuming it.
    fn peek_spot(&self) -> f64 {
        self.spot_at(self.counter)
    }

    /// The next deterministically-bumped spot, advancing the internal counter.
    fn next_spot(&mut self) -> f64 {
        let spot = self.spot_at(self.counter);
        self.counter = self.counter.wrapping_add(1);
        spot
    }
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

    /// Record `token` as consumed (so a replayed `Execute` is rejected as
    /// already-consumed), keyed by its validity deadline, and **evict every
    /// already-expired consumed token** in the same pass.
    ///
    /// Replay protection only has to hold *within a token's validity window*: once a
    /// token is past its `valid_until_nanos` an `Execute` presenting it is rejected
    /// as `Expired` before the consumed-set is ever consulted, so retaining expired
    /// entries buys no extra protection — they are dead weight. Pruning them on every
    /// insert bounds the set to at most the tokens minted within one validity window
    /// (a few per the millisecond-cadence tick over the 1-second window), so it can
    /// never grow without bound over an arbitrarily long session (the scale
    /// guardrail, `CLAUDE.md` rule 6).
    fn record_consumed(&mut self, token: u64, valid_until_nanos: i64, now_nanos: i64) {
        // Evict expired entries first (amortized O(1): the live window is small).
        self.consumed_tokens
            .retain(|_, &mut deadline| deadline >= now_nanos);
        self.consumed_tokens.insert(token, valid_until_nanos);
    }

    /// Is `token` a still-valid consumed token (consumed within its window, blocking
    /// a replay)? Expired consumed entries are lazily pruned and never matched here:
    /// an expired token is the `Expired` path, not the `AlreadyConsumed` path.
    fn is_consumed(&self, token: u64, now_nanos: i64) -> bool {
        self.consumed_tokens
            .get(&token)
            .is_some_and(|&deadline| deadline >= now_nanos)
    }
}

/// A cryptographically-unforgeable click-to-trade token minter.
///
/// A click-to-trade token must be impossible to forge without the server secret:
/// presenting a guessed/enumerated token would otherwise book a trade the maker never
/// quoted. The token is therefore a **keyed MAC** (a truncated `blake3` keyed hash,
/// RFC-grade keyed-hash construction) over the immutable line-binding tuple
/// `(subscription_id, sequence, side, premium_bits, valid_until_nanos)` plus a fresh
/// per-mint nonce. The 256-bit key is drawn **once at session start from the OS
/// CSPRNG** ([`getrandom`]); without it an attacker cannot produce a value that the
/// server will accept, even by enumeration of the 64-bit token space.
///
/// # Why a CSPRNG secret does not break pricing determinism
///
/// This is a **runtime, control-plane ephemeral identity** — it authenticates a click,
/// it is *never* an input to any priced number. The platform's determinism guardrail
/// (`CLAUDE.md` rule 5) governs the **pricing** path (libm, no OS RNG) so prices are
/// bit-reproducible; a token's MAC tag is not a price and is not replayed for pricing.
/// Seeding the MAC key from a CSPRNG is exactly correct here: unpredictability is the
/// security property we need, and it leaves every priced value untouched.
struct TokenMinter {
    /// The session-unique MAC key, drawn once from the OS CSPRNG at session start.
    key: [u8; 32],
    /// A fresh per-mint nonce so two tokens for the *same* binding tuple still differ
    /// (and a token reveals nothing about the key).
    nonce: AtomicU64,
}

/// The immutable binding a click-to-trade token authenticates: a token is valid only
/// for *exactly* the line it was stamped on (its subscription, sequence, side,
/// premium, and validity deadline). The MAC is computed over this tuple, so a token
/// cannot be lifted onto a different line, side, or premium.
#[derive(Debug, Clone, Copy)]
struct TokenBinding {
    subscription_id: u64,
    sequence: u64,
    side: Side,
    premium: f64,
    valid_until_nanos: i64,
}

impl TokenMinter {
    fn new() -> Self {
        let mut key = [0u8; 32];
        // OS CSPRNG. `getrandom` cannot fail on a supported platform; a failure here
        // means the OS entropy source is unavailable, which is unrecoverable for a
        // security-bearing token, so we refuse to serve forgeable tokens.
        getrandom::getrandom(&mut key)
            .expect("OS CSPRNG unavailable: cannot mint unforgeable tokens");
        Self {
            key,
            nonce: AtomicU64::new(0),
        }
    }

    /// Serialize a binding + nonce into the MAC message: a fixed-width, unambiguous
    /// big-endian encoding so distinct tuples never collide on the same message.
    fn message(binding: &TokenBinding, nonce: u64) -> [u8; 41] {
        let mut msg = [0u8; 41];
        msg[0..8].copy_from_slice(&binding.subscription_id.to_be_bytes());
        msg[8..16].copy_from_slice(&binding.sequence.to_be_bytes());
        msg[16] = binding.side as u8;
        msg[17..25].copy_from_slice(&binding.premium.to_bits().to_be_bytes());
        msg[25..33].copy_from_slice(&binding.valid_until_nanos.to_be_bytes());
        msg[33..41].copy_from_slice(&nonce.to_be_bytes());
        msg
    }

    /// The 64-bit MAC tag for a binding under a given nonce (first 8 bytes of the
    /// keyed `blake3` hash, big-endian).
    fn tag(&self, binding: &TokenBinding, nonce: u64) -> u64 {
        let mac = blake3::keyed_hash(&self.key, &Self::message(binding, nonce));
        u64::from_be_bytes(mac.as_bytes()[0..8].try_into().expect("32-byte hash"))
    }

    /// Mint an unforgeable token for a line binding (always `>= 1`). The fresh nonce
    /// makes two mints of the same binding distinct; a zero tag is re-minted so the
    /// token is always a non-zero sentinel-safe value.
    fn mint(&self, binding: &TokenBinding) -> u64 {
        loop {
            let nonce = self.nonce.fetch_add(1, Ordering::Relaxed);
            let t = self.tag(binding, nonce);
            if t != 0 {
                return t;
            }
        }
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
    // A new sequence retires the prior sequence's tokens: a click always books the
    // current streamed premium, never a stale one.
    sub.live_tokens.clear();
    let valid_until = now_nanos.saturating_add(TOKEN_VALIDITY_NANOS);
    let sub_id = sub.id.value;
    let mut out = Vec::with_capacity(2);
    // SELL hits the bid (only when the bid is a real, positive price).
    if two_way.bid > 0.0 {
        let token = minter.mint(&TokenBinding {
            subscription_id: sub_id,
            sequence: seq,
            side: Side::Sell,
            premium: two_way.bid,
            valid_until_nanos: valid_until,
        });
        sub.live_tokens.insert(
            token,
            LiveToken {
                side: Side::Sell,
                premium: two_way.bid,
                valid_until_nanos: valid_until,
            },
        );
        out.push(TradableToken {
            token,
            side: Side::Sell as i32,
            premium: two_way.bid,
            valid_until_nanos: valid_until,
        });
    }
    // BUY lifts the offer.
    let token = minter.mint(&TokenBinding {
        subscription_id: sub_id,
        sequence: seq,
        side: Side::Buy,
        premium: two_way.offer,
        valid_until_nanos: valid_until,
    });
    sub.live_tokens.insert(
        token,
        LiveToken {
            side: Side::Buy,
            premium: two_way.offer,
            valid_until_nanos: valid_until,
        },
    );
    out.push(TradableToken {
        token,
        side: Side::Buy as i32,
        premium: two_way.offer,
        valid_until_nanos: valid_until,
    });
    out
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
    let (priced, two_way) = sub.price(market, spread)?;
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
        }
    }
}

/// The routing pair of one client→server stream message, if it carries one. Only an
/// opening [`celnet_proto::Subscribe`] (its instrument's pair) or a
/// [`MarketSeriesSubscribe`] (its pair) names a pair; control frames (modify /
/// unsubscribe / resync / execute / heartbeat) do not, so a session is routed off the
/// FIRST pair-bearing message.
fn pair_of_client_message(msg: &ClientStreamMessage) -> Option<&celnet_proto::CcyPair> {
    match msg.message.as_ref()? {
        client_stream_message::Message::Subscribe(s) => s.instrument.as_ref()?.pair.as_ref(),
        client_stream_message::Message::MarketSeriesSubscribe(s) => s.pair.as_ref(),
        _ => None,
    }
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

    // The pair the line trades.
    let Some(wire_pair) = sub.instrument.pair.clone() else {
        return;
    };
    let Ok(pair) = celnet_types::CcyPair::try_from(wire_pair) else {
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
        sub.base_market.r_dom,
        sub.base_market.r_for,
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
        let market = MarketContext {
            spot: snap.spot,
            vol: snap.atm_vol,
            r_dom: snap.r_dom,
            r_for: snap.r_for,
        };
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
            tick: SpotTick::new(market.spot, id.value),
            replay: VecDeque::new(),
            live_tokens: HashMap::new(),
            consumed_tokens: HashMap::new(),
            execute_idempotency: HashMap::new(),
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
        let sub = self.subs.get_mut(&id.value).expect("checked present");
        // Re-baseline the subscription in place at the next sequence.
        sub.instrument = instrument;
        sub.conv = conv;
        sub.base_market = market;
        sub.surface_version = pinned.echo_version;
        sub.lagged = false;
        sub.tick = SpotTick::new(market.spot, id.value);
        // Modifying retires the prior structure's tradable tokens (they reference a
        // line that no longer exists). Consumed tokens stay recorded so a late
        // replayed Execute is still rejected as already-consumed.
        sub.live_tokens.clear();
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

        // Already consumed (a second click / replayed Execute on a booked token,
        // within its validity window — expired consumed entries are pruned and fall
        // through to the Expired path below).
        if sub.is_consumed(e.token, now) {
            return out_tx
                .send(Ok(reject(stream_reject::Reason::AlreadyConsumed)))
                .await
                .is_ok();
        }
        // Unknown / forged token: not a live stamped token for this subscription.
        let Some(live) = sub.live_tokens.get(&e.token).copied() else {
            return out_tx
                .send(Ok(reject(stream_reject::Reason::UnknownToken)))
                .await
                .is_ok();
        };
        // Expired: presented after its validity deadline (last-look).
        if now > live.valid_until_nanos {
            return out_tx
                .send(Ok(reject(stream_reject::Reason::Expired)))
                .await
                .is_ok();
        }

        // Book it: mark the token consumed (bounded, expiry-evicting), mint an
        // execution, record idempotency.
        sub.record_consumed(e.token, live.valid_until_nanos, now);
        sub.live_tokens.remove(&e.token);
        let execution_id = exec_id_counter.fetch_add(1, Ordering::Relaxed);
        let executed = Executed {
            subscription: Some(id),
            token: e.token,
            execution_id,
            side: live.side as i32,
            traded_premium: live.premium,
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
            record_booked_position(&store, sub, &self.spread, execution_id, live.side);
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
            // on the current live path (the spot the next live Update continues
            // from), so the client never holds an off-path baseline.
            let market = MarketContext {
                spot: sub.tick.peek_spot(),
                ..sub.base_market
            };
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
        let Some(pair) = s.pair.clone() else {
            let _ = out_tx
                .send(Err(Status::invalid_argument(
                    "market-series subscribe requires a `pair`",
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
                    pair: Some(pair),
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

    /// Drive one market tick across every live subscription. Returns `false` only
    /// when the channel has closed (client gone).
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
            let spot = sub.tick.next_spot();
            let market = MarketContext {
                spot,
                ..sub.base_market
            };
            let seq = sub.sequence + 1;
            debug_assert!(
                seq > sub.delivered,
                "live tick must advance past the last delivered sequence"
            );
            let update = match make_update(sub, seq, &market, &spread, minter, &clock) {
                Ok(m) => m,
                Err(status) => {
                    let _ = out_tx.try_send(Err(status));
                    if out_tx.is_closed() {
                        return false;
                    }
                    mark_lagged(sub, out_tx);
                    continue;
                }
            };
            if try_emit(out_tx, update.clone()) {
                sub.sequence = seq;
                sub.delivered = seq;
                sub.retain(seq, update);
                if seq % HEARTBEAT_EVERY == 0 {
                    let hb = ServerStreamMessage {
                        message: Some(server_stream_message::Message::Heartbeat(Heartbeat {
                            subscription: Some(sub.id),
                            sequence: seq,
                            epoch_nanos: clock.now_nanos(),
                        })),
                    };
                    let _ = out_tx.try_send(Ok(hb));
                }
            } else if out_tx.is_closed() {
                return false;
            } else {
                // The dropped `seq` was neither advanced nor retained, and the
                // just-minted tokens (in `live_tokens`) are about to be retired by
                // the next emitted line, so no gap, duplicate, or stale token leaks.
                mark_lagged(sub, out_tx);
            }
        }
        true
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
            pair: Some(celnet_proto::CcyPair {
                base: "EUR".to_owned(),
                quote: "USD".to_owned(),
            }),
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
            product: Some(celnet_proto::instrument::Product::Vanilla(
                celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(celnet_proto::StrikeOrDelta {
                        spec: Some(celnet_proto::strike_or_delta::Spec::Strike(strike)),
                    }),
                },
            )),
        }
    }

    /// Build a session over a calibrated EURUSD fixture, with one open subscription
    /// (baseline snapshot at sequence 1 already retained + delivered), ready to be
    /// driven by `drive_tick` and to receive `Execute`s.
    fn make_session(clock: Clock) -> Session {
        let market = MarketContext {
            spot: 1.10,
            vol: 0.10,
            r_dom: 0.02,
            r_for: 0.01,
        };
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
            tick: SpotTick::new(market.spot, 1),
            replay: VecDeque::new(),
            live_tokens: HashMap::new(),
            consumed_tokens: HashMap::new(),
            execute_idempotency: HashMap::new(),
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

    /// `peek_spot` must return exactly the value the next `next_spot` consumes.
    #[test]
    fn peek_spot_matches_the_next_consumed_spot() {
        let mut t = SpotTick::new(1.2345, 99);
        for _ in 0..32 {
            let peeked = t.peek_spot();
            assert_eq!(t.peek_spot().to_bits(), peeked.to_bits());
            assert_eq!(t.next_spot().to_bits(), peeked.to_bits());
        }
    }

    /// A fixed line binding for token-MAC tests.
    fn binding(seq: u64, side: Side, premium: f64) -> TokenBinding {
        TokenBinding {
            subscription_id: 1,
            sequence: seq,
            side,
            premium,
            valid_until_nanos: 2_000_000_000,
        }
    }

    /// Minted tokens are unguessable (not a dense 1,2,3 sequence) and distinct, even
    /// for the *same* binding (the per-mint nonce differs).
    #[test]
    fn token_minter_is_unguessable_and_distinct() {
        let minter = TokenMinter::new();
        let b = binding(1, Side::Buy, 0.012);
        let t1 = minter.mint(&b);
        let t2 = minter.mint(&b);
        assert!(t1 >= 1 && t2 >= 1);
        assert_ne!(t1, t2, "fresh nonce ⇒ distinct tokens for one binding");
        assert_ne!(t1.abs_diff(t2), 1, "tokens must not be a dense sequence");
    }

    /// The MAC is **unforgeable without the key** and **binds the whole line**: the
    /// same binding+nonce under a different key yields a different tag (so a forged
    /// token cannot be produced without the server secret), and changing any bound
    /// field (sequence / side / premium / expiry) changes the tag (so a token cannot
    /// be lifted onto another line).
    #[test]
    fn token_mac_is_key_bound_and_field_bound() {
        let minter = TokenMinter::new();
        let other = TokenMinter::new();
        let b = binding(5, Side::Buy, 0.012);
        // Same message, different key ⇒ different tag (key-bound / unforgeable).
        assert_ne!(
            minter.tag(&b, 0),
            other.tag(&b, 0),
            "a different key must yield a different MAC (forgery needs the key)"
        );
        // Field-bound: flipping any bound field changes the tag.
        assert_ne!(
            minter.tag(&b, 0),
            minter.tag(&binding(6, Side::Buy, 0.012), 0)
        );
        assert_ne!(
            minter.tag(&b, 0),
            minter.tag(&binding(5, Side::Sell, 0.012), 0)
        );
        assert_ne!(
            minter.tag(&b, 0),
            minter.tag(&binding(5, Side::Buy, 0.013), 0)
        );
        let mut expiry_shift = binding(5, Side::Buy, 0.012);
        expiry_shift.valid_until_nanos += 1;
        assert_ne!(minter.tag(&b, 0), minter.tag(&expiry_shift, 0));
    }

    /// A snapshot stamps two click-to-trade tokens (SELL@bid, BUY@offer) whose
    /// premiums match the streamed two-way, and they are registered live.
    #[tokio::test]
    async fn snapshot_stamps_live_tradable_tokens() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let session = make_session(clock);
            let sub = session.subs.get(&1).unwrap();
            assert_eq!(sub.live_tokens.len(), 2, "SELL@bid + BUY@offer");
            let buy = sub
                .live_tokens
                .values()
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
                &MarketContext {
                    spot: 1.101,
                    vol: 0.10,
                    r_dom: 0.02,
                    r_for: 0.01,
                },
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
            let prior_token = *session
                .subs
                .get(&1)
                .unwrap()
                .live_tokens
                .keys()
                .next()
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
                !session
                    .subs
                    .get(&1)
                    .unwrap()
                    .live_tokens
                    .contains_key(&prior_token),
                "modify retires the prior structure's tokens"
            );
        })
        .await
        .expect("no hang");
    }

    /// `drive_tick` advances the per-subscription sequence and stamps fresh tokens.
    #[tokio::test]
    async fn drive_tick_advances_sequence_and_remints_tokens() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let clock = Clock::manual(1_000_000_000);
            let mut session = make_session(clock);
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(64);
            assert!(session.drive_tick(&tx));
            let mut last = 1u64;
            while let Ok(msg) = rx.try_recv() {
                let msg = msg.unwrap();
                if let Some(seq) = seq_of(&msg) {
                    assert_eq!(seq, last + 1, "strictly +1");
                    last = seq;
                    assert!(
                        !snapshot_tokens_update(&msg).is_empty(),
                        "update mints tokens"
                    );
                }
            }
            assert!(last >= 2, "the tick advanced past the baseline");
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
                    &MarketContext {
                        spot: 1.10 + 0.0001 * seq as f64,
                        vol: 0.10,
                        r_dom: 0.02,
                        r_for: 0.01,
                    },
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
            let consumed = session.subs.get(&1).unwrap().consumed_tokens.len();
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
                .live_tokens
                .iter()
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
