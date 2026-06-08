//! The market-series feed over the multiplexed RFS session: a typed,
//! self-sequencing stream of one labelled market observable (the GUI's TrendMode
//! contract — ATM vol, spot, a delta-wing risk reversal / butterfly, or the
//! outright forward), opened on the *same* [`crate::StreamSession`] a price
//! subscription rides.
//!
//! # One session, price + series multiplexed
//!
//! A caller opens a [`MarketSeries`] via [`crate::StreamSession::subscribe_series`]
//! over the session's one connection, in the same subscription-id space as a price
//! [`crate::Subscription`] — a trend tile and its live blotter share one HTTP/2
//! stream. The session driver demultiplexes the server's
//! [`celnet_proto::MarketSeriesSnapshot`] / [`celnet_proto::MarketSeriesPoint`]
//! frames by their [`celnet_proto::SubscriptionId`] to the owning series, exactly
//! as it routes price frames.
//!
//! # What the caller works with
//!
//! A [`MarketSeries`] is a typed async [`futures_util::Stream`] of [`SeriesEvent`]s:
//! a first [`SeriesEvent::Snapshot`] seeds the recent history (oldest → newest) and
//! the observable's identity for labelling, then [`SeriesEvent::Point`]s append the
//! live observed values as the maker's state ticks. Each value is the server's
//! `celnet-core`-deterministic observation of the live surface/market — never
//! fabricated client-side. The wire `oneof`, the sequence numbers, and the
//! observable/delta encoding are handled inside the SDK.

use std::pin::Pin;
use std::task::{Context, Poll};

use celnet_proto::{MarketObservable as WireObservable, MarketSeriesPoint, MarketSeriesSnapshot};
use futures_util::Stream;
use tokio::sync::mpsc;

use crate::error::{ClientError, ClientResult};

/// Which labelled market observable a [`MarketSeries`] streams — the typed form of
/// the wire [`celnet_proto::MarketObservable`]. Each is a unit-bearing quantity
/// (vol for the vol observables, a rate for spot/forward), not an abstract index.
///
/// The wing observables ([`Observable::RiskReversal`] / [`Observable::Butterfly`])
/// carry the signed delta wing they are measured at; the others do not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Observable {
    /// At-the-money volatility for the `(pair, tenor)` — absolute vol (`0.10` = 10v).
    AtmVol,
    /// Spot FX rate (quote per 1 base) — a tenor-independent series.
    Spot,
    /// Risk reversal (call vol − put vol) at the given signed delta wing — vol.
    RiskReversal {
        /// The signed delta wing (e.g. `0.25` or `0.10`).
        delta: f64,
    },
    /// Butterfly (broker fly) at the given signed delta wing — vol.
    Butterfly {
        /// The signed delta wing (e.g. `0.25` or `0.10`).
        delta: f64,
    },
    /// Outright forward FX rate for the `(pair, tenor)` — quote per 1 base.
    Forward,
}

impl Observable {
    /// The wire observable tag.
    pub(crate) fn wire_tag(self) -> i32 {
        let w = match self {
            Observable::AtmVol => WireObservable::AtmVol,
            Observable::Spot => WireObservable::Spot,
            Observable::RiskReversal { .. } => WireObservable::RiskReversal,
            Observable::Butterfly { .. } => WireObservable::Butterfly,
            Observable::Forward => WireObservable::Forward,
        };
        w as i32
    }

    /// The signed delta wing the server requires for a wing observable, or `None`
    /// for a non-wing observable (which must not carry one).
    pub(crate) fn wing_delta(self) -> Option<f64> {
        match self {
            Observable::RiskReversal { delta } | Observable::Butterfly { delta } => Some(delta),
            Observable::AtmVol | Observable::Spot | Observable::Forward => None,
        }
    }

    /// Decode a wire observable tag (the snapshot echoes one for labelling) into the
    /// typed form. A wing observable's `delta` is not echoed on the snapshot, so it
    /// is reported as `0.0` (the caller already knows the wing it subscribed).
    pub(crate) fn from_wire_tag(tag: i32) -> ClientResult<Self> {
        use celnet_proto::convert::WireError;
        Ok(match WireObservable::try_from(tag) {
            Ok(WireObservable::AtmVol) => Observable::AtmVol,
            Ok(WireObservable::Spot) => Observable::Spot,
            Ok(WireObservable::Forward) => Observable::Forward,
            Ok(WireObservable::RiskReversal) => Observable::RiskReversal { delta: 0.0 },
            Ok(WireObservable::Butterfly) => Observable::Butterfly { delta: 0.0 },
            Err(_) => {
                return Err(WireError::UnknownEnum {
                    kind: "MarketObservable",
                    tag,
                }
                .into());
            }
        })
    }
}

/// One observed point in a market series: a timestamped value of the observable at
/// a sequence point. The typed form of the wire [`celnet_proto::MarketSeriesPoint`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesPoint {
    /// The monotonic per-series sequence number of this point.
    pub sequence: u64,
    /// The observed value, in the observable's natural unit (vol for ATM/RR/BF; a
    /// rate for spot/forward).
    pub value: f64,
    /// Observation time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

impl SeriesPoint {
    pub(crate) fn from_wire(w: &MarketSeriesPoint) -> Self {
        Self {
            sequence: w.sequence,
            value: w.value,
            epoch_nanos: w.epoch_nanos,
        }
    }
}

/// One event surfaced by a [`MarketSeries`] stream: the opening baseline, or a live
/// appended point.
#[derive(Debug, Clone)]
pub enum SeriesEvent {
    /// The opening baseline: the recent history (oldest → newest) plus the
    /// observable's echoed identity, so a trend tile can label itself and seed its
    /// sparkline before live points arrive.
    Snapshot {
        /// The observable this series streams (echoed for labelling). For a wing
        /// observable the `delta` is `0.0` (the snapshot does not echo it); the
        /// caller knows the wing it subscribed.
        observable: Observable,
        /// The starting sequence number subsequent points advance from.
        sequence: u64,
        /// The recent history (oldest → newest), at most the requested
        /// `history_limit` points.
        points: Vec<SeriesPoint>,
        /// Snapshot time, nanoseconds since the Unix epoch (UTC).
        epoch_nanos: i64,
    },
    /// A live appended observation advancing the series.
    Point(SeriesPoint),
}

/// The driver-side state for one open market series: the typed-event channel to the
/// caller. Held in the session's series registry, keyed by the series'
/// subscription id.
#[derive(Debug)]
pub(crate) struct SeriesState {
    pub(crate) event_tx: mpsc::Sender<ClientResult<SeriesEvent>>,
}

/// Decode a wire [`MarketSeriesSnapshot`] into a typed [`SeriesEvent::Snapshot`].
pub(crate) fn decode_snapshot(s: &MarketSeriesSnapshot) -> ClientResult<SeriesEvent> {
    let observable = Observable::from_wire_tag(s.observable)?;
    let points = s.points.iter().map(SeriesPoint::from_wire).collect();
    Ok(SeriesEvent::Snapshot {
        observable,
        sequence: s.sequence,
        points,
        epoch_nanos: s.epoch_nanos,
    })
}

/// A live market-series subscription on a [`crate::StreamSession`]: a typed async
/// stream of [`SeriesEvent`]s for one observable, multiplexed over the session's one
/// connection.
///
/// Construct via [`crate::StreamSession::subscribe_series`]. Dropping the series
/// leaves the session (and its other subscriptions) running; the driver stops
/// routing to a dropped series' closed channel. Call
/// [`MarketSeries::unsubscribe`] to tear it down server-side explicitly.
#[derive(Debug)]
pub struct MarketSeries {
    pub(crate) sub_id: u64,
    pub(crate) rx: mpsc::Receiver<ClientResult<SeriesEvent>>,
    pub(crate) control: mpsc::Sender<celnet_proto::ClientStreamMessage>,
    pub(crate) observable: Observable,
}

impl MarketSeries {
    /// The per-session subscription id this series is keyed on.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.sub_id
    }

    /// The observable this series streams (with its wing delta, for a wing
    /// observable — known from the subscribe request, unlike the snapshot echo).
    #[must_use]
    pub fn observable(&self) -> Observable {
        self.observable
    }

    /// Await the next series event, or `None` once the series has ended (a clean
    /// unsubscribe or a dropped session).
    pub async fn next_event(&mut self) -> Option<ClientResult<SeriesEvent>> {
        self.rx.recv().await
    }

    /// Tear this series down server-side (the rest of the session is unaffected).
    /// After it, the stream ends; the caller may drop the handle.
    ///
    /// # Errors
    ///
    /// [`ClientError::StreamClosed`] if the session's control channel has closed.
    pub async fn unsubscribe(&self) -> ClientResult<()> {
        use celnet_proto::{
            ClientStreamMessage, MarketSeriesUnsubscribe, SubscriptionId, client_stream_message,
        };
        let msg = ClientStreamMessage {
            message: Some(client_stream_message::Message::MarketSeriesUnsubscribe(
                MarketSeriesUnsubscribe {
                    subscription: Some(SubscriptionId { value: self.sub_id }),
                },
            )),
        };
        self.control
            .send(msg)
            .await
            .map_err(|_| ClientError::StreamClosed)
    }
}

impl Stream for MarketSeries {
    type Item = ClientResult<SeriesEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observable_wire_round_trip_and_wing_delta_rules() {
        // Non-wing observables carry no delta and round-trip their identity.
        for obs in [Observable::AtmVol, Observable::Spot, Observable::Forward] {
            assert!(obs.wing_delta().is_none(), "{obs:?} carries no wing delta");
            let decoded = Observable::from_wire_tag(obs.wire_tag()).expect("known tag");
            assert_eq!(decoded, obs, "{obs:?} round-trips its identity");
        }

        // Wing observables carry their delta on the request; the snapshot echo
        // does not carry it (decodes to 0.0), but the family is preserved.
        let rr = Observable::RiskReversal { delta: 0.25 };
        assert_eq!(rr.wing_delta(), Some(0.25));
        assert!(matches!(
            Observable::from_wire_tag(rr.wire_tag()).unwrap(),
            Observable::RiskReversal { .. }
        ));
        let bf = Observable::Butterfly { delta: 0.10 };
        assert_eq!(bf.wing_delta(), Some(0.10));
        assert!(matches!(
            Observable::from_wire_tag(bf.wire_tag()).unwrap(),
            Observable::Butterfly { .. }
        ));
    }

    #[test]
    fn unknown_observable_tag_is_a_wire_error() {
        assert!(matches!(
            Observable::from_wire_tag(99),
            Err(ClientError::Wire(_))
        ));
    }

    #[test]
    fn decode_snapshot_seeds_history_oldest_to_newest() {
        let snap = MarketSeriesSnapshot {
            subscription: Some(celnet_proto::SubscriptionId { value: 3 }),
            sequence: 1,
            underlying: None,
            observable: WireObservable::AtmVol as i32,
            points: vec![
                MarketSeriesPoint {
                    subscription: Some(celnet_proto::SubscriptionId { value: 3 }),
                    sequence: 1,
                    value: 0.105,
                    epoch_nanos: 10,
                },
                MarketSeriesPoint {
                    subscription: Some(celnet_proto::SubscriptionId { value: 3 }),
                    sequence: 2,
                    value: 0.106,
                    epoch_nanos: 20,
                },
            ],
            epoch_nanos: 20,
        };
        match decode_snapshot(&snap).expect("decodes") {
            SeriesEvent::Snapshot {
                observable,
                sequence,
                points,
                ..
            } => {
                assert_eq!(observable, Observable::AtmVol);
                assert_eq!(sequence, 1);
                assert_eq!(points.len(), 2);
                assert_eq!(points[0].value, 0.105);
                assert_eq!(points[1].sequence, 2);
            }
            other => panic!("expected a Snapshot, got {other:?}"),
        }
    }
}
