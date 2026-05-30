//! The RFS (request-for-stream) service: a bidirectional gRPC stream carrying
//! per-subscription `Snapshot` + sequenced `Update` deltas, `Heartbeat`s, and a
//! server-assisted `Resync` replay.
//!
//! A counterparty opens one stream per client session and multiplexes any number
//! of [`celnet_proto::Subscribe`] subscriptions over it. For each subscription the
//! service:
//!
//! 1. sends a [`celnet_proto::Snapshot`] (sequence 1) — the full baseline state of
//!    the subscribed [`celnet_proto::Instrument`] (two-way price, Greeks, vol);
//! 2. drives a **deterministic** market tick loop that reprices *that*
//!    subscription's instrument and emits sequenced [`celnet_proto::Update`] deltas
//!    (sequence 2, 3, …), interleaved with periodic [`celnet_proto::Heartbeat`]s;
//! 3. answers a client [`celnet_proto::Resync`] by replaying a fresh
//!    [`celnet_proto::Snapshot`] at the current sequence so a client that detected
//!    a gap resumes from a known-good baseline;
//! 4. tears the subscription down on [`celnet_proto::Unsubscribe`] (or stream
//!    close), emitting a [`celnet_proto::StreamEnd`].
//!
//! # Per-subscriber back-pressure, never block the core
//!
//! Each subscription owns a **bounded** server→client channel. The market tick is
//! the same deterministic, counter-based [`crate::tick::TickSource`] the engine
//! uses elsewhere, so a stream is bit-exactly reproducible from its seed; the per
//! subscription instrument is repriced on the async edge against the ticked market
//! (the pinned hot core is never blocked by a slow streaming consumer). If a
//! subscriber lags past its channel depth the subscription is dropped with a
//! [`celnet_proto::StreamEnd`] (`LAGGED`) rather than back-pressuring the driver —
//! one slow consumer never stalls the others.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use celnet_proto::stream_service_server::StreamService;
use celnet_proto::{
    ClientStreamMessage, Conventions, Heartbeat, Instrument, MarketContext, ServerStreamMessage,
    Snapshot, StreamEnd, SubscriptionId, Update, client_stream_message, server_stream_message,
    stream_end,
};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tonic::{Request, Response, Status, Streaming};

use crate::clock::Clock;
use crate::core_link::CoreLink;
use crate::pricer::{ConventionSet, Priced, price_instrument};
use crate::readiness::ReadinessGate;
use crate::spread::SpreadModel;
use crate::tick::TickSource;

use super::stream_rx::ReceiverStream;

/// The bounded depth of each per-subscriber server→client channel. A subscriber
/// lagging past this many messages is dropped (LAGGED) rather than stalling the
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

/// The RFS streaming service over the [`CoreLink`] and readiness gate.
#[derive(Debug)]
pub struct StreamEdge {
    link: Arc<CoreLink>,
    gate: Arc<ReadinessGate>,
    spread: SpreadModel,
    clock: Clock,
}

impl StreamEdge {
    /// Construct the RFS service.
    #[must_use]
    pub fn new(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
    ) -> Self {
        Self {
            link,
            gate,
            spread,
            clock,
        }
    }
}

/// The per-subscription server-side state: the priced instrument, its conventions,
/// the monotonic sequence, the deterministic tick source, and the replay buffer.
struct Subscription {
    id: SubscriptionId,
    instrument: Instrument,
    conv: ConventionSet,
    /// The base market the subscription was opened against (its tick source bumps
    /// spot around this).
    base_market: MarketContext,
    /// The monotonic per-subscription sequence number last emitted.
    sequence: u64,
    /// The highest sequence actually handed to the client's channel (delivered,
    /// not dropped). A Resync never replays at or below this — every sequence is
    /// delivered to the client at most once across a resync, so the stream stays
    /// strictly increasing, gap-free, and duplicate-free.
    delivered: u64,
    /// Whether this subscription is in the recoverable **lagged** state: its
    /// channel filled, so live emission is paused (the retained buffer is kept
    /// intact) until a [`celnet_proto::Resync`] re-establishes it. The
    /// subscription is *not* removed, so a later Resync can recover it.
    lagged: bool,
    /// Whether the `LAGGED` [`StreamEnd`] marker has been successfully delivered
    /// to the client. The marker is retried on later ticks until it lands (the
    /// channel that filled has by then drained a slot), so a lagged subscriber is
    /// always told to resync and can never be silently stranded.
    lag_notified: bool,
    /// Deterministic spot-tick source seeded from the subscription id.
    tick: SpotTick,
    /// Recent server messages retained for a Resync replay (seq → message).
    replay: VecDeque<(u64, ServerStreamMessage)>,
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
    /// *without* consuming it. Used by a fresh resync snapshot so its baseline is
    /// priced on the same spot the immediately-following [`Update`] continues from.
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
    ) -> Result<(Priced, celnet_proto::TwoWayPrice), Status> {
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

/// Encode the wire conventions for a subscription, mapping a decode error to an
/// `invalid_argument` status.
fn decode_conv(w: &Conventions) -> Result<ConventionSet, Status> {
    ConventionSet::decode(w).map_err(|e| Status::invalid_argument(e.to_string()))
}

/// Build a [`Snapshot`] message for a subscription at `seq` priced against
/// `market`.
fn make_snapshot(
    sub: &Subscription,
    seq: u64,
    market: &MarketContext,
    spread: &SpreadModel,
    clock: &Clock,
) -> Result<ServerStreamMessage, Status> {
    let (priced, two_way) = sub.price(market, spread)?;
    Ok(ServerStreamMessage {
        message: Some(server_stream_message::Message::Snapshot(Snapshot {
            subscription: Some(sub.id),
            sequence: seq,
            price: Some(two_way),
            greeks: Some(priced.greeks.into()),
            vol: priced.vol,
            conventions: Some(conv_to_wire(&sub.conv)),
            resolved_strike: priced.resolved_strike,
            epoch_nanos: clock.now_nanos(),
        })),
    })
}

/// Build an [`Update`] message for a subscription at `seq` priced against
/// `market`.
fn make_update(
    sub: &Subscription,
    seq: u64,
    market: &MarketContext,
    spread: &SpreadModel,
    clock: &Clock,
) -> Result<ServerStreamMessage, Status> {
    let (priced, two_way) = sub.price(market, spread)?;
    Ok(ServerStreamMessage {
        message: Some(server_stream_message::Message::Update(Update {
            subscription: Some(sub.id),
            sequence: seq,
            price: Some(two_way),
            greeks: Some(priced.greeks.into()),
            vol: priced.vol,
            epoch_nanos: clock.now_nanos(),
        })),
    })
}

/// Re-encode a decoded [`ConventionSet`] back to the wire form for echoing in a
/// snapshot.
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
    type StreamStream = ReceiverStream<Result<ServerStreamMessage, Status>>;

    async fn stream(
        &self,
        request: Request<Streaming<ClientStreamMessage>>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        // The readiness gate gates *new* streams: a draining instance refuses to
        // open a session so new traffic steers to the warm replacement.
        let guard = self.gate.enter();
        if !self.gate.is_ready() {
            return Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ));
        }

        let mut inbound = request.into_inner();
        let (out_tx, out_rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(CHANNEL_DEPTH);
        let link = Arc::clone(&self.link);
        let spread = self.spread;
        let clock = self.clock.clone();

        // The session driver: owns every subscription multiplexed on this stream,
        // reads client control messages, and emits server messages. One task per
        // session keeps the per-subscription state single-owner (no locks) and
        // tears everything down when the stream closes.
        tokio::spawn(async move {
            let _guard = guard; // held for the session lifetime (drain barrier).
            let mut subs: HashMap<u64, Subscription> = HashMap::new();
            let mut ticker = tokio::time::interval(TICK_INTERVAL);
            // Skip the immediate first tick so the loop parks until either a
            // control message or the first real interval.
            ticker.tick().await;

            loop {
                tokio::select! {
                    // ---- client → server control ------------------------------
                    incoming = inbound.next() => {
                        match incoming {
                            Some(Ok(msg)) => {
                                if !handle_client_message(
                                    msg, &mut subs, &link, &spread, &clock, &out_tx,
                                ).await {
                                    break; // a fatal send failure: tear down.
                                }
                            }
                            Some(Err(_)) | None => break, // client closed / errored.
                        }
                    }
                    // ---- deterministic market tick → updates ------------------
                    _ = ticker.tick() => {
                        if subs.is_empty() {
                            continue;
                        }
                        if !drive_tick(&mut subs, &spread, &clock, &out_tx) {
                            break; // channel closed: client gone.
                        }
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(out_rx)))
    }
}

/// Handle one client→server control message, mutating the subscription set and
/// emitting server messages. Returns `false` only on a fatal channel close.
async fn handle_client_message(
    msg: ClientStreamMessage,
    subs: &mut HashMap<u64, Subscription>,
    link: &Arc<CoreLink>,
    spread: &SpreadModel,
    clock: &Clock,
    out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
) -> bool {
    let Some(message) = msg.message else {
        return true; // empty control frame: ignore.
    };
    match message {
        client_stream_message::Message::Subscribe(s) => {
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
                            "subscribe missing/!invalid conventions",
                        )))
                        .await;
                    return true;
                }
            };
            // Read the maker's live market as the subscription baseline.
            let market = match link.market_snapshot().await {
                Ok(snap) => MarketContext {
                    spot: snap.spot,
                    vol: snap.atm_vol,
                    r_dom: snap.r_dom,
                    r_for: snap.r_for,
                },
                Err(e) => {
                    let _ = out_tx.send(Err(Status::unavailable(e.to_string()))).await;
                    return true;
                }
            };
            let mut sub = Subscription {
                id,
                instrument,
                conv,
                base_market: market,
                sequence: 0,
                delivered: 0,
                lagged: false,
                lag_notified: false,
                tick: SpotTick::new(market.spot, id.value),
                replay: VecDeque::new(),
            };
            // First message is the baseline snapshot at sequence 1.
            sub.sequence = 1;
            let snap = match make_snapshot(&sub, 1, &market, spread, clock) {
                Ok(m) => m,
                Err(status) => {
                    let _ = out_tx.send(Err(status)).await;
                    return true;
                }
            };
            sub.retain(1, snap);
            // Use a blocking send for the snapshot so the baseline is never
            // dropped (it must arrive before any delta).
            if out_tx.send(Ok(snap)).await.is_err() {
                return false;
            }
            sub.delivered = 1;
            subs.insert(id.value, sub);
            true
        }
        client_stream_message::Message::Unsubscribe(u) => {
            if let Some(id) = u.subscription
                && subs.remove(&id.value).is_some()
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
            let Some(sub) = subs.get_mut(&id.value) else {
                return true;
            };
            handle_resync(sub, r.last_sequence, spread, clock, out_tx).await
        }
        client_stream_message::Message::Heartbeat(_) => {
            // A client heartbeat is liveness only; the server's own heartbeats are
            // emitted by the tick loop. Nothing to do.
            true
        }
    }
}

/// Handle a client [`celnet_proto::Resync`] for one subscription: re-establish it
/// (clearing any recoverable lagged state) and replay each retained message
/// *exactly once*, strictly after the client's last good sequence.
///
/// The replay window is `seq > last_sequence`, so a sequence the client already
/// applied is never re-delivered (no duplicate) and none is skipped (no gap); the
/// live tick then resumes from `sequence + 1`, beyond everything just delivered.
/// If the gap predates the retained buffer, a fresh snapshot baseline is sent
/// instead. Returns `false` only when the channel has closed (client gone).
async fn handle_resync(
    sub: &mut Subscription,
    last_sequence: u64,
    spread: &SpreadModel,
    clock: &Clock,
    out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
) -> bool {
    // A Resync re-establishes a subscription that may have been paused in the
    // recoverable lagged state: clear the flag so live ticks resume.
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
            .map(|(_, m)| *m)
            .collect();
        for m in to_replay {
            if out_tx.send(Ok(m)).await.is_err() {
                return false;
            }
        }
        // Everything up to the buffer tail is now delivered; the live tick resumes
        // from `sequence + 1`, never re-emitting a replayed seq.
        sub.delivered = sub.sequence;
    } else {
        let seq = sub.sequence + 1;
        sub.sequence = seq;
        // The gap predates the retained buffer, so we send a fresh snapshot
        // baseline. It MUST be priced on the current live path, not the fixed
        // subscription-open spot (`base_market`): the immediately-following live
        // `drive_tick` will price its `Update` at `sub.tick.next_spot()`, so we
        // peek that very spot here and price the snapshot at it. Otherwise the
        // client would briefly hold an off-path baseline before the next Update
        // snapped it onto the resumed deterministic path.
        let market = MarketContext {
            spot: sub.tick.peek_spot(),
            ..sub.base_market
        };
        let snap = match make_snapshot(sub, seq, &market, spread, clock) {
            Ok(m) => m,
            Err(status) => {
                let _ = out_tx.send(Err(status)).await;
                return true;
            }
        };
        sub.retain(seq, snap);
        if out_tx.send(Ok(snap)).await.is_err() {
            return false;
        }
        sub.delivered = seq;
    }
    true
}

/// Drive one market tick across every live subscription: reprice each against its
/// deterministically-bumped market and emit a sequenced [`Update`] (and a periodic
/// [`Heartbeat`]).
///
/// A subscriber whose bounded channel fills is **not** torn down: it is moved into
/// the recoverable *lagged* state (live emission paused, retained buffer kept) and
/// a single [`StreamEnd`] (`LAGGED`) is emitted to prompt a [`celnet_proto::Resync`].
/// Because the subscription stays registered, a later Resync re-establishes it from
/// the retained buffer with no sequence break. While lagged, the subscription is
/// skipped (no new sequences accrue), so the recovered stream stays gap-free and
/// duplicate-free. Returns `false` only when the channel has closed (client gone).
fn drive_tick(
    subs: &mut HashMap<u64, Subscription>,
    spread: &SpreadModel,
    clock: &Clock,
    out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>,
) -> bool {
    for sub in subs.values_mut() {
        // A lagged subscription is paused pending a Resync: do not emit (and do
        // not accrue sequences), so its retained buffer recovers it intact. Keep
        // retrying the LAGGED marker until it lands so the client always resyncs.
        if sub.lagged {
            if !sub.lag_notified && out_tx.is_closed() {
                return false;
            }
            notify_lagged(sub, out_tx);
            continue;
        }
        // Tick the subscription's own deterministic market.
        let spot = sub.tick.next_spot();
        let market = MarketContext {
            spot,
            ..sub.base_market
        };
        let seq = sub.sequence + 1;
        // Invariant: live emission is strictly monotonic and never re-delivers a
        // sequence already handed to the client (whether live or via a replay).
        debug_assert!(
            seq > sub.delivered,
            "live tick must advance past the last delivered sequence"
        );
        let update = match make_update(sub, seq, &market, spread, clock) {
            Ok(m) => m,
            Err(status) => {
                // A pricing error pauses just this subscription (recoverable).
                let _ = out_tx.try_send(Err(status));
                if out_tx.is_closed() {
                    return false;
                }
                mark_lagged(sub, out_tx);
                continue;
            }
        };
        if try_emit(out_tx, update) {
            sub.sequence = seq;
            sub.delivered = seq;
            sub.retain(seq, update);
            // Periodic heartbeat mirroring the current sequence.
            if seq % HEARTBEAT_EVERY == 0 {
                let hb = ServerStreamMessage {
                    message: Some(server_stream_message::Message::Heartbeat(Heartbeat {
                        subscription: Some(sub.id),
                        sequence: seq,
                        epoch_nanos: clock.now_nanos(),
                    })),
                };
                // A full channel here just skips this heartbeat; the next update
                // carries the sequence forward anyway.
                let _ = out_tx.try_send(Ok(hb));
            }
        } else if out_tx.is_closed() {
            return false;
        } else {
            // Bounded channel momentarily full ⇒ this subscriber lagged. Pause it
            // (keeping it registered + its buffer intact) so a Resync recovers it.
            // The dropped `seq` was neither advanced nor retained, so no gap or
            // duplicate is introduced.
            mark_lagged(sub, out_tx);
        }
    }
    true
}

/// Move a subscription into the recoverable lagged state. The subscription stays
/// registered so a later Resync re-establishes it from the retained buffer (it is
/// *not* removed); the `LAGGED` marker is then (re)attempted via [`notify_lagged`].
fn mark_lagged(sub: &mut Subscription, out_tx: &mpsc::Sender<Result<ServerStreamMessage, Status>>) {
    if sub.lagged {
        return; // already paused.
    }
    sub.lagged = true;
    sub.lag_notified = false;
    notify_lagged(sub, out_tx);
}

/// Attempt to deliver the `LAGGED` [`StreamEnd`] marker prompting the client to
/// [`celnet_proto::Resync`]. Idempotent: once the marker lands, `lag_notified` is
/// set and further calls are no-ops. The marker is retried (across ticks) because
/// the channel that filled has by then drained a slot, so the client is always
/// told to resync and never silently stranded.
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

    /// The wire conventions used to build a test subscription (mirrors the
    /// integration harness): spot-unadjusted / ATM-forward / domestic-pips.
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

    /// A vanilla EURUSD 1Y call at an absolute strike.
    fn vanilla_call(strike: f64) -> Instrument {
        Instrument {
            pair: Some(celnet_proto::CcyPair {
                base: "EUR".to_owned(),
                quote: "USD".to_owned(),
            }),
            tenor: Some(celnet_proto::Tenor {
                unit: celnet_proto::tenor::Unit::Years as i32,
                count: 1,
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

    /// Build a fresh subscription with its baseline snapshot at sequence 1 already
    /// retained + delivered, ready to be driven by [`drive_tick`].
    fn make_test_sub(id: u64, spread: &SpreadModel, clock: &Clock) -> Subscription {
        let market = MarketContext {
            spot: 1.10,
            vol: 0.10,
            r_dom: 0.02,
            r_for: 0.01,
        };
        let conv = ConventionSet::decode(&wire_conv()).expect("conventions decode");
        let mut sub = Subscription {
            id: SubscriptionId { value: id },
            instrument: vanilla_call(1.10),
            conv,
            base_market: market,
            sequence: 1,
            delivered: 1,
            lagged: false,
            lag_notified: false,
            tick: SpotTick::new(market.spot, id),
            replay: VecDeque::new(),
        };
        let snap = make_snapshot(&sub, 1, &market, spread, clock).expect("baseline snapshot");
        sub.retain(1, snap);
        sub
    }

    /// Extract the **advancing** sequence carried by a snapshot or update. A
    /// heartbeat only *mirrors* the current sequence (it does not advance it), so
    /// it is excluded from the strict-monotonic stream checks; a stream-end
    /// carries none.
    fn seq_of(msg: &ServerStreamMessage) -> Option<u64> {
        match msg.message.as_ref()? {
            server_stream_message::Message::Snapshot(s) => Some(s.sequence),
            server_stream_message::Message::Update(u) => Some(u.sequence),
            server_stream_message::Message::Heartbeat(_)
            | server_stream_message::Message::StreamEnd(_) => None,
        }
    }

    /// Extract the priced `(two_way, vol)` a snapshot or update carries (for the
    /// on-path baseline check). Heartbeats / stream-ends carry no price.
    fn price_of(msg: &ServerStreamMessage) -> Option<(celnet_proto::TwoWayPrice, f64)> {
        match msg.message.as_ref()? {
            server_stream_message::Message::Snapshot(s) => Some((s.price?, s.vol)),
            server_stream_message::Message::Update(u) => Some((u.price?, u.vol)),
            server_stream_message::Message::Heartbeat(_)
            | server_stream_message::Message::StreamEnd(_) => None,
        }
    }

    fn is_lagged_end(msg: &ServerStreamMessage) -> bool {
        matches!(
            msg.message.as_ref(),
            Some(server_stream_message::Message::StreamEnd(e))
                if e.reason == stream_end::Reason::Lagged as i32
        )
    }

    /// Regression for the RFS lagged-recovery bug: when a subscriber lags,
    /// `drive_tick` must **not** remove its subscription (which would make a later
    /// Resync unrecoverable). The subscription stays registered in the recoverable
    /// lagged state, a single LAGGED marker is emitted, and a subsequent Resync
    /// re-establishes its stream from the retained buffer and resumes correctly
    /// sequenced updates — with no sequence break, no gap, and no duplicate.
    #[tokio::test]
    async fn lagged_subscriber_recovers_via_resync_without_sequence_break() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let spread = SpreadModel::default();
            let clock = Clock::system();
            // A tiny channel so the subscriber lags after a few un-drained ticks.
            let depth = 4usize;
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(depth);

            let mut subs: HashMap<u64, Subscription> = HashMap::new();
            subs.insert(1, make_test_sub(1, &spread, &clock));

            // Drive ticks without draining until the subscriber is marked lagged.
            // (Bounded: the channel fills within `depth + a few` ticks.)
            let mut lag_ticks = 0;
            while !subs.get(&1).expect("sub present").lagged {
                assert!(drive_tick(&mut subs, &spread, &clock, &tx));
                lag_ticks += 1;
                assert!(lag_ticks < 100, "the subscriber must lag once the channel fills");
            }

            // Bug 1: the lagged subscription is NOT removed — it is still present so
            // a Resync can recover it.
            assert!(
                subs.contains_key(&1),
                "a lagged subscriber must remain registered for recovery"
            );

            // The client drains everything the channel buffered, tracking the
            // highest in-order sequence it actually applied (its `last_good`).
            let mut last_good = 0u64;
            let mut saw_lagged_marker = false;
            // Keep ticking (which retries the LAGGED marker) and draining until the
            // LAGGED marker has been observed and the channel is empty.
            loop {
                while let Ok(msg) = rx.try_recv() {
                    let msg = msg.expect("no error frame");
                    if is_lagged_end(&msg) {
                        saw_lagged_marker = true;
                    } else if let Some(seq) = seq_of(&msg) {
                        assert!(
                            seq == last_good + 1 || last_good == 0,
                            "delivered sequences before the lag are gap-free: {seq} after {last_good}"
                        );
                        last_good = last_good.max(seq);
                    }
                }
                if saw_lagged_marker {
                    break;
                }
                // Re-tick so the retried LAGGED marker can land now space is free.
                assert!(drive_tick(&mut subs, &spread, &clock, &tx));
            }
            assert!(saw_lagged_marker, "a LAGGED StreamEnd prompted the client to resync");

            // The client resyncs from its last good sequence.
            let resumed = handle_resync(
                subs.get_mut(&1).expect("sub still present"),
                last_good,
                &spread,
                &clock,
                &tx,
            )
            .await;
            assert!(resumed, "resync succeeds while the channel has room");
            assert!(
                !subs.get(&1).expect("sub present").lagged,
                "resync clears the recoverable lagged state"
            );

            // Collect the replayed messages, then drive a few more live ticks, and
            // assert the whole post-resync stream is strictly increasing, gap-free,
            // and duplicate-free starting at last_good + 1.
            let mut post: Vec<u64> = Vec::new();
            let drain_into = |rx: &mut mpsc::Receiver<Result<ServerStreamMessage, Status>>,
                              post: &mut Vec<u64>| {
                while let Ok(msg) = rx.try_recv() {
                    let msg = msg.expect("no error frame");
                    if let Some(seq) = seq_of(&msg) {
                        post.push(seq);
                    }
                }
            };
            drain_into(&mut rx, &mut post);
            for _ in 0..3 {
                assert!(drive_tick(&mut subs, &spread, &clock, &tx));
                drain_into(&mut rx, &mut post);
            }

            assert!(!post.is_empty(), "resync + live ticks delivered messages");
            // Strictly increasing (so: no duplicate and no gap), starting exactly
            // one past the client's last good sequence.
            assert_eq!(
                post[0],
                last_good + 1,
                "resume resumes exactly at last_good + 1 (no gap, no duplicate)"
            );
            for w in post.windows(2) {
                assert_eq!(
                    w[1],
                    w[0] + 1,
                    "post-resync sequence stream is strictly +1 (gap-free, duplicate-free): {post:?}"
                );
            }
        })
        .await
        .expect("test must not hang");
    }

    /// Regression for the resync-duplicate bug: a resync must deliver each sequence
    /// **exactly once**. Drive a clean stream, drain part of it (so the client's
    /// last_good lags the server), resync, and assert the union of pre-resync and
    /// post-resync sequences the client applies is strictly increasing with no
    /// duplicate and no gap.
    #[tokio::test]
    async fn resync_delivers_each_sequence_exactly_once() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let spread = SpreadModel::default();
            let clock = Clock::system();
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(256);
            let mut subs: HashMap<u64, Subscription> = HashMap::new();
            subs.insert(1, make_test_sub(1, &spread, &clock));

            // Drive several ticks (well within the channel depth, so none drop).
            for _ in 0..6 {
                assert!(drive_tick(&mut subs, &spread, &clock, &tx));
            }

            // The client applies only up to sequence 3 (simulating it lagged behind
            // or detected a gap there) and resyncs from there. It already holds the
            // baseline snapshot (sequence 1), delivered out-of-band on subscribe.
            let mut applied: Vec<u64> = vec![1];
            let mut buffered: Vec<u64> = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                if let Some(seq) = seq_of(&msg.expect("no error frame")) {
                    buffered.push(seq);
                }
            }
            // Client applies the first run up to and including 3 in order.
            let last_good = 3u64;
            for &seq in &buffered {
                if seq <= last_good {
                    applied.push(seq);
                }
            }

            let resumed = handle_resync(
                subs.get_mut(&1).expect("sub present"),
                last_good,
                &spread,
                &clock,
                &tx,
            )
            .await;
            assert!(resumed);

            // Drain the replay, then a few more live ticks.
            let mut post: Vec<u64> = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                if let Some(seq) = seq_of(&msg.expect("no error frame")) {
                    post.push(seq);
                }
            }
            for _ in 0..3 {
                assert!(drive_tick(&mut subs, &spread, &clock, &tx));
                while let Ok(msg) = rx.try_recv() {
                    if let Some(seq) = seq_of(&msg.expect("no error frame")) {
                        post.push(seq);
                    }
                }
            }

            // The client applies the replayed/live tail (everything > last_good),
            // deduping via its own stale filter (seq <= last_good ignored).
            for &seq in &post {
                if seq > last_good {
                    applied.push(seq);
                }
            }

            // The full applied stream is strictly increasing 1,2,3,…: no duplicate,
            // no gap — each sequence delivered exactly once across the resync.
            assert_eq!(
                applied.first().copied(),
                Some(1),
                "applied stream starts at 1"
            );
            for w in applied.windows(2) {
                assert_eq!(
                    w[1],
                    w[0] + 1,
                    "applied stream is gap-free and duplicate-free across resync: {applied:?}"
                );
            }
        })
        .await
        .expect("test must not hang");
    }

    /// `peek_spot` must return exactly the value the next `next_spot` consumes (and
    /// not advance the counter), so a fresh resync snapshot can be priced on the
    /// very spot the immediately-following live tick continues from.
    #[test]
    fn peek_spot_matches_the_next_consumed_spot() {
        let mut t = SpotTick::new(1.2345, 99);
        for _ in 0..32 {
            let peeked = t.peek_spot();
            // Peeking is idempotent and does not advance.
            assert_eq!(
                t.peek_spot().to_bits(),
                peeked.to_bits(),
                "peek must not advance the counter"
            );
            let consumed = t.next_spot();
            assert_eq!(
                consumed.to_bits(),
                peeked.to_bits(),
                "the consumed spot must be bit-identical to the peeked spot"
            );
        }
    }

    /// Regression for the resync stale-baseline bug: when the client's gap predates
    /// the retained buffer, `handle_resync` sends a **fresh snapshot**. That
    /// snapshot must be priced on the *current live path* (the spot the next live
    /// `Update` will continue from) — NOT the fixed subscription-open `base_market`.
    /// We assert the fresh-snapshot price/vol is bit-identical to the next live
    /// update's, so the consumer never holds an off-path baseline. With the old
    /// (base_market) baseline the snapshot would price at the open spot while the
    /// next update priced at a bumped spot, so the two would differ.
    #[tokio::test]
    async fn fresh_resync_snapshot_is_priced_on_the_live_path() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let spread = SpreadModel::default();
            let clock = Clock::system();
            let (tx, mut rx) = mpsc::channel::<Result<ServerStreamMessage, Status>>(256);
            let mut subs: HashMap<u64, Subscription> = HashMap::new();
            subs.insert(1, make_test_sub(1, &spread, &clock));

            // Advance the live path several ticks so the tick counter is non-zero
            // (the open spot is no longer on-path).
            for _ in 0..5 {
                assert!(drive_tick(&mut subs, &spread, &clock, &tx));
            }
            // Drain everything buffered so far.
            while rx.try_recv().is_ok() {}

            // Resync with a last_sequence FAR before the retained buffer front so the
            // replay branch cannot satisfy it and the fresh-snapshot branch is taken.
            // (last_sequence = 0; the buffer front seq is >= 1, so 0+1 < front only
            // once seqs advanced — the baseline at seq 1 was retained, so to force
            // the fresh branch we evict it: REPLAY_DEPTH is large, so instead we use
            // the explicit invariant that front > last_sequence + 1 cannot hold for
            // last_sequence well below front. Here front == 1, so request a resync
            // that cannot replay by clearing the buffer to simulate eviction.)
            {
                let sub = subs.get_mut(&1).expect("sub present");
                sub.replay.clear(); // emulate a gap older than the retained buffer.
            }

            let resumed = handle_resync(
                subs.get_mut(&1).expect("sub present"),
                0,
                &spread,
                &clock,
                &tx,
            )
            .await;
            assert!(resumed, "resync succeeds while the channel has room");

            // The fresh snapshot just emitted.
            let snap_msg = rx
                .try_recv()
                .expect("a fresh snapshot was emitted")
                .expect("no error frame");
            let (snap_price, snap_vol) =
                price_of(&snap_msg).expect("the fresh resync message is a priced snapshot");
            assert!(
                matches!(
                    snap_msg.message.as_ref(),
                    Some(server_stream_message::Message::Snapshot(_))
                ),
                "the before-buffer recovery sends a Snapshot baseline"
            );

            // The very next live tick's update.
            assert!(drive_tick(&mut subs, &spread, &clock, &tx));
            let next_update = loop {
                let m = rx
                    .try_recv()
                    .expect("the next live update was emitted")
                    .expect("no error frame");
                if matches!(
                    m.message.as_ref(),
                    Some(server_stream_message::Message::Update(_))
                ) {
                    break m;
                }
            };
            let (next_price, next_vol) =
                price_of(&next_update).expect("the next live message is a priced update");

            // The baseline is on the resumed live path: bit-identical price + vol.
            assert_eq!(
                snap_price.bid.to_bits(),
                next_price.bid.to_bits(),
                "fresh snapshot bid must match the next live update (on-path baseline)"
            );
            assert_eq!(
                snap_price.offer.to_bits(),
                next_price.offer.to_bits(),
                "fresh snapshot offer must match the next live update (on-path baseline)"
            );
            assert_eq!(
                snap_vol.to_bits(),
                next_vol.to_bits(),
                "fresh snapshot vol must match the next live update"
            );
        })
        .await
        .expect("test must not hang");
    }
}
