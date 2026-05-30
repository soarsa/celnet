//! The request-for-stream (RFS) subscription: a typed async [`Stream`] of a
//! baseline snapshot followed by sequenced deltas, with automatic gap detection,
//! server-assisted resync, and reconnect handled inside the SDK.
//!
//! A caller opens a [`Subscription`] for one [`InstrumentSpec`] and then `.next()`s
//! a stream of [`StreamEvent`]s — never touching the bidirectional gRPC channel,
//! the per-subscription sequence numbers, or the control protocol. The SDK:
//!
//! * sends the [`celnet_proto::Subscribe`] and surfaces the first
//!   [`celnet_proto::Snapshot`] as [`StreamEvent::Snapshot`], establishing the
//!   baseline sequence;
//! * tracks the monotonic per-subscription sequence and surfaces each in-order
//!   [`celnet_proto::Update`] as [`StreamEvent::Tick`];
//! * **detects a gap** (a received sequence beyond `last + 1`) and automatically
//!   sends a [`celnet_proto::Resync`] with the last good sequence, emitting
//!   [`StreamEvent::GapDetected`] then resuming from the replayed messages /
//!   fresh snapshot the server returns;
//! * on a [`celnet_proto::StreamEnd`] with reason `LAGGED` automatically resyncs,
//!   and with reason `DRAINING` (a blue-green cutover) automatically re-dials a new
//!   stream and re-subscribes, transparent to the caller, emitting
//!   [`StreamEvent::Reconnected`];
//! * surfaces [`celnet_proto::Heartbeat`]s as [`StreamEvent::Heartbeat`] so a
//!   caller can monitor liveness, and forwards a clean unsubscribe end as
//!   [`StreamEvent::Closed`].
//!
//! The subscription owns a background driver task joined to a bounded channel, so
//! polling the [`Stream`] never blocks on the network and a slow caller applies
//! natural back-pressure without stalling the SDK's resync bookkeeping.

use std::pin::Pin;
use std::task::{Context, Poll};

use celnet_proto::stream_service_client::StreamServiceClient;
use celnet_proto::{
    ClientStreamMessage, Conventions as WireConventions, Instrument, Resync, ServerStreamMessage,
    Subscribe, SubscriptionId, client_stream_message, server_stream_message, stream_end,
};
use celnet_types::Greeks;
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;
use tonic::transport::Channel;

use crate::error::{ClientError, ClientResult};
use crate::vocab::{Conventions, InstrumentSpec, TwoWay};

/// The depth of the SDK→caller event channel. A caller momentarily slower than the
/// stream buffers up to this many events before exerting back-pressure on the
/// driver's `recv` loop (which is itself fed by the gRPC flow-controlled channel).
const EVENT_CHANNEL_DEPTH: usize = 1024;

/// The depth of the caller→server control channel feeding the bidirectional
/// stream's outbound half (subscribe / resync). Small: control traffic is sparse.
const CONTROL_CHANNEL_DEPTH: usize = 16;

/// A streamed snapshot or tick line: the two-way market, the Greek set, the vol,
/// and the resolved strike, all at one sequence point. The typed payload shared by
/// [`StreamEvent::Snapshot`] and [`StreamEvent::Tick`].
#[derive(Debug, Clone, Copy)]
pub struct StreamLine {
    /// The per-subscription monotonic sequence number of this line.
    pub sequence: u64,
    /// The current two-way market.
    pub price: TwoWay,
    /// The full Greek set at this sequence point.
    pub greeks: Greeks,
    /// The instrument vol (absolute) at this sequence point.
    pub vol: f64,
    /// Message time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

/// One event surfaced by a [`Subscription`] stream. The SDK collapses the wire
/// snapshot/update/heartbeat/stream-end protocol and the resync/reconnect
/// machinery into this small, caller-facing vocabulary.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// The baseline snapshot establishing (or re-establishing) the sequence — the
    /// state a caller applies whole before consuming ticks.
    Snapshot {
        /// The snapshot line (price + greeks + vol at the snapshot sequence).
        line: StreamLine,
        /// The strike the subscription resolved to (for a delta/solve key).
        resolved_strike: f64,
        /// The conventions the streamed prices are expressed under.
        conventions: Conventions,
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
    /// The server drained for a blue-green cutover and the SDK transparently
    /// re-dialed a fresh stream and re-subscribed. A fresh
    /// [`StreamEvent::Snapshot`] follows.
    Reconnected,
}

/// A live RFS subscription: a typed async stream of [`StreamEvent`]s for one
/// instrument, with gap detection, resync, and reconnect handled inside the SDK.
///
/// Construct via [`crate::Client::subscribe`]. Drop the subscription (or the
/// driver's channel closing) tears the underlying gRPC stream down.
#[derive(Debug)]
pub struct Subscription {
    rx: mpsc::Receiver<ClientResult<StreamEvent>>,
    _driver: tokio::task::JoinHandle<()>,
}

impl Subscription {
    /// Open a subscription. Dials a bidirectional stream over `channel`, sends the
    /// `Subscribe`, and spawns the driver that pumps wire messages into typed
    /// events, performing gap-detection / resync / reconnect.
    ///
    /// # Errors
    ///
    /// [`ClientError`] if the initial stream cannot be opened.
    pub(crate) async fn open(
        channel: Channel,
        sub_id: u64,
        instrument: InstrumentSpec,
        conventions: Conventions,
    ) -> ClientResult<Self> {
        let wire_instrument = instrument.to_wire();
        let wire_conv = conventions.to_wire();

        // The bounded SDK→caller event channel.
        let (event_tx, event_rx) = mpsc::channel::<ClientResult<StreamEvent>>(EVENT_CHANNEL_DEPTH);

        // Open the first stream eagerly so a connection failure surfaces from
        // `open` (not from the first `.next()`), matching a caller's mental model.
        let initial = open_stream(channel.clone(), sub_id, &wire_instrument, &wire_conv).await?;

        let driver = tokio::spawn(drive(
            channel,
            sub_id,
            wire_instrument,
            wire_conv,
            initial,
            event_tx,
        ));

        Ok(Self {
            rx: event_rx,
            _driver: driver,
        })
    }

    /// Await the next stream event, or `None` once the subscription has ended (a
    /// clean unsubscribe, an unrecoverable stream end, or a dropped channel).
    ///
    /// # Errors
    ///
    /// Yields a [`ClientError`] event if a wire message could not be decoded or the
    /// server returned a status mid-stream.
    pub async fn next_event(&mut self) -> Option<ClientResult<StreamEvent>> {
        self.rx.recv().await
    }
}

impl Stream for Subscription {
    type Item = ClientResult<StreamEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// An open bidirectional stream: the inbound server-message stream plus the
/// outbound control sender (held alive so the server keeps the session open).
struct OpenStream {
    inbound: tonic::Streaming<ServerStreamMessage>,
    control: mpsc::Sender<ClientStreamMessage>,
}

/// Dial a fresh bidirectional stream and send the initial `Subscribe`.
async fn open_stream(
    channel: Channel,
    sub_id: u64,
    instrument: &Instrument,
    conv: &WireConventions,
) -> ClientResult<OpenStream> {
    let mut client = StreamServiceClient::new(channel);
    let (control_tx, control_rx) = mpsc::channel::<ClientStreamMessage>(CONTROL_CHANNEL_DEPTH);

    // Seed the outbound stream with the Subscribe before handing it to tonic, so
    // the very first client frame opens the subscription.
    let subscribe = ClientStreamMessage {
        message: Some(client_stream_message::Message::Subscribe(Subscribe {
            subscription: Some(SubscriptionId { value: sub_id }),
            instrument: Some(instrument.clone()),
            conventions: Some(*conv),
            throttle_nanos: 0,
        })),
    };
    control_tx
        .send(subscribe)
        .await
        .map_err(|_| ClientError::StreamClosed)?;

    let outbound = tokio_stream_from(control_rx);
    let response = client.stream(outbound).await?;
    Ok(OpenStream {
        inbound: response.into_inner(),
        control: control_tx,
    })
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

/// The subscription driver: pump server messages into typed events, detect gaps
/// and resync, and reconnect on a drain. Runs until the caller drops the
/// subscription (event channel closed) or the subscription ends.
async fn drive(
    channel: Channel,
    sub_id: u64,
    instrument: Instrument,
    conv: WireConventions,
    mut stream: OpenStream,
    event_tx: mpsc::Sender<ClientResult<StreamEvent>>,
) {
    // The last in-order sequence the SDK has applied. 0 = no baseline yet.
    let mut last_seq: u64 = 0;

    loop {
        let next = stream.inbound.next().await;
        match next {
            Some(Ok(msg)) => {
                let Some(payload) = msg.message else {
                    continue; // empty frame: ignore.
                };
                match handle_server_message(
                    payload,
                    &mut last_seq,
                    &stream.control,
                    sub_id,
                    &event_tx,
                )
                .await
                {
                    Flow::Continue => {}
                    Flow::CallerGone | Flow::Closed => return,
                    Flow::Reconnect => {
                        // Drain (blue-green cutover): re-dial a fresh stream and
                        // re-subscribe, then keep going from a fresh baseline.
                        match reconnect(&channel, sub_id, &instrument, &conv).await {
                            Ok(fresh) => {
                                stream = fresh;
                                last_seq = 0;
                                if event_tx.send(Ok(StreamEvent::Reconnected)).await.is_err() {
                                    return;
                                }
                            }
                            Err(e) => {
                                let _ = event_tx.send(Err(e)).await;
                                return;
                            }
                        }
                    }
                }
            }
            Some(Err(status)) => {
                let _ = event_tx.send(Err(ClientError::from(status))).await;
                return;
            }
            None => {
                // Stream closed by the server without a StreamEnd marker.
                return;
            }
        }
    }
}

/// Is `observed` beyond the next expected sequence given the `last_good` applied?
///
/// A gap exists once a baseline is established (`last_good != 0`) and the observed
/// sequence skips at least one number (`observed > last_good + 1`). This is the
/// single predicate that triggers an automatic resync; isolating it keeps the
/// resync trigger pure and unit-testable.
fn is_gap(last_good: u64, observed: u64) -> bool {
    last_good != 0 && observed > last_good + 1
}

/// Is `observed` a stale / duplicate sequence (≤ the last applied), to be dropped
/// idempotently? Only meaningful once a baseline is established.
fn is_stale(last_good: u64, observed: u64) -> bool {
    last_good != 0 && observed <= last_good
}

/// Internal control-flow signal from handling one server message.
enum Flow {
    /// Keep pumping the current stream.
    Continue,
    /// The caller dropped the subscription; stop.
    CallerGone,
    /// The subscription ended cleanly (unsubscribe / expiry); stop.
    Closed,
    /// The server is draining for a cutover; re-dial and re-subscribe.
    Reconnect,
}

/// Handle one decoded server stream message, advancing the sequence, emitting the
/// typed event, and triggering resync on a detected gap.
async fn handle_server_message(
    payload: server_stream_message::Message,
    last_seq: &mut u64,
    control: &mpsc::Sender<ClientStreamMessage>,
    sub_id: u64,
    event_tx: &mpsc::Sender<ClientResult<StreamEvent>>,
) -> Flow {
    match payload {
        server_stream_message::Message::Snapshot(s) => {
            let line = match decode_line(s.sequence, &s.price, &s.greeks, s.vol, s.epoch_nanos) {
                Ok(l) => l,
                Err(e) => {
                    return emit(event_tx, Err(e)).await;
                }
            };
            let conventions = match s
                .conventions
                .as_ref()
                .ok_or(ClientError::MissingField("Snapshot.conventions"))
                .and_then(Conventions::from_wire)
            {
                Ok(c) => c,
                Err(e) => return emit(event_tx, Err(e)).await,
            };
            // A snapshot (baseline or post-resync) re-establishes the sequence.
            *last_seq = s.sequence;
            emit(
                event_tx,
                Ok(StreamEvent::Snapshot {
                    line,
                    resolved_strike: s.resolved_strike,
                    conventions,
                }),
            )
            .await
        }
        server_stream_message::Message::Update(u) => {
            // Gap detection: an update must advance the sequence by exactly one.
            if is_gap(*last_seq, u.sequence) {
                let gap = StreamEvent::GapDetected {
                    last_good: *last_seq,
                    observed: u.sequence,
                };
                if matches!(emit(event_tx, Ok(gap)).await, Flow::CallerGone) {
                    return Flow::CallerGone;
                }
                // Ask the server to replay from the last good sequence.
                if send_resync(control, sub_id, *last_seq).await.is_err() {
                    return Flow::Closed;
                }
                // Do not apply this out-of-order update; wait for the replay /
                // fresh snapshot to re-establish the sequence.
                return Flow::Continue;
            }
            // A stale or duplicate update (≤ last_seq) is ignored idempotently.
            if is_stale(*last_seq, u.sequence) {
                return Flow::Continue;
            }
            let line = match decode_line(u.sequence, &u.price, &u.greeks, u.vol, u.epoch_nanos) {
                Ok(l) => l,
                Err(e) => return emit(event_tx, Err(e)).await,
            };
            *last_seq = u.sequence;
            emit(event_tx, Ok(StreamEvent::Tick(line))).await
        }
        server_stream_message::Message::Heartbeat(hb) => {
            // A heartbeat carries the current sequence; if it reveals a gap (the
            // stream advanced past us without our seeing the updates), resync.
            if is_gap(*last_seq, hb.sequence) {
                let gap = StreamEvent::GapDetected {
                    last_good: *last_seq,
                    observed: hb.sequence,
                };
                if matches!(emit(event_tx, Ok(gap)).await, Flow::CallerGone) {
                    return Flow::CallerGone;
                }
                if send_resync(control, sub_id, *last_seq).await.is_err() {
                    return Flow::Closed;
                }
                return Flow::Continue;
            }
            emit(
                event_tx,
                Ok(StreamEvent::Heartbeat {
                    sequence: hb.sequence,
                    epoch_nanos: hb.epoch_nanos,
                }),
            )
            .await
        }
        server_stream_message::Message::StreamEnd(end) => {
            match stream_end::Reason::try_from(end.reason) {
                Ok(stream_end::Reason::Lagged) => {
                    // We were dropped for lagging: resync from our last good seq to
                    // recover, staying on the same stream.
                    let gap = StreamEvent::GapDetected {
                        last_good: *last_seq,
                        observed: *last_seq,
                    };
                    if matches!(emit(event_tx, Ok(gap)).await, Flow::CallerGone) {
                        return Flow::CallerGone;
                    }
                    if send_resync(control, sub_id, *last_seq).await.is_err() {
                        return Flow::Closed;
                    }
                    Flow::Continue
                }
                Ok(stream_end::Reason::Draining) => Flow::Reconnect,
                Ok(stream_end::Reason::Unsubscribed | stream_end::Reason::Expired) => Flow::Closed,
                Err(_) => Flow::Closed,
            }
        }
    }
}

/// Send the typed event to the caller; map a closed channel to `CallerGone`.
async fn emit(tx: &mpsc::Sender<ClientResult<StreamEvent>>, ev: ClientResult<StreamEvent>) -> Flow {
    if tx.send(ev).await.is_err() {
        Flow::CallerGone
    } else {
        Flow::Continue
    }
}

/// Send a `Resync(last_sequence)` control message on the stream's outbound half.
async fn send_resync(
    control: &mpsc::Sender<ClientStreamMessage>,
    sub_id: u64,
    last_sequence: u64,
) -> Result<(), ()> {
    let msg = ClientStreamMessage {
        message: Some(client_stream_message::Message::Resync(Resync {
            subscription: Some(SubscriptionId { value: sub_id }),
            last_sequence,
        })),
    };
    control.send(msg).await.map_err(|_| ())
}

/// Re-dial a fresh bidirectional stream and re-subscribe (after a drain cutover).
async fn reconnect(
    channel: &Channel,
    sub_id: u64,
    instrument: &Instrument,
    conv: &WireConventions,
) -> ClientResult<OpenStream> {
    open_stream(channel.clone(), sub_id, instrument, conv).await
}

/// Decode a snapshot/update line's price + greeks into a typed [`StreamLine`].
fn decode_line(
    sequence: u64,
    price: &Option<celnet_proto::TwoWayPrice>,
    greeks: &Option<celnet_proto::Greeks>,
    vol: f64,
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
    Ok(StreamLine {
        sequence,
        price,
        greeks,
        vol,
        epoch_nanos,
    })
}

#[cfg(test)]
mod tests {
    use super::{is_gap, is_stale};

    #[test]
    fn no_gap_before_a_baseline_is_established() {
        // last_good == 0 means "no baseline yet"; nothing is a gap or stale.
        assert!(!is_gap(0, 5));
        assert!(!is_stale(0, 5));
    }

    #[test]
    fn in_order_advance_is_neither_gap_nor_stale() {
        assert!(!is_gap(7, 8), "seq+1 is in-order");
        assert!(!is_stale(7, 8));
    }

    #[test]
    fn a_skipped_sequence_is_a_gap() {
        assert!(is_gap(7, 9), "skipping 8 is a gap");
        assert!(is_gap(7, 100));
        assert!(!is_stale(7, 9));
    }

    #[test]
    fn a_repeated_or_old_sequence_is_stale_not_a_gap() {
        assert!(is_stale(7, 7), "a duplicate is stale");
        assert!(is_stale(7, 3), "an old replay tail is stale");
        assert!(!is_gap(7, 7));
        assert!(!is_gap(7, 3));
    }
}
