//! The fixed-income (linear-rates) live-price feed over the multiplexed RFS
//! session: a typed, self-sequencing stream of a rates instrument's PV + first-order
//! risk (PV01 / DV01 / key-rate ladder) as the pricing curve deterministically
//! evolves, opened on the *same* [`crate::StreamSession`] an FX price
//! [`crate::Subscription`] and a [`crate::MarketSeries`] ride.
//!
//! # One session, FX price + market-series + FI rates multiplexed
//!
//! A caller opens a [`RatesSubscription`] via
//! [`crate::StreamSession::subscribe_rates`] (or the one-line
//! [`crate::Client::subscribe_rates`]) over the session's one connection, in the
//! same subscription-id space as a price [`crate::Subscription`] and a market
//! [`crate::MarketSeries`]. The session driver demultiplexes the server's
//! [`celnet_proto::RatesStreamSnapshot`] / [`celnet_proto::RatesStreamUpdate`]
//! frames by their [`celnet_proto::SubscriptionId`] to the owning rates line,
//! exactly as it routes FX price and market-series frames — the FI feed rides the
//! ONE `StreamService` bidi + the ONE server-side `PriceFanout`, never a bespoke
//! parallel FI stack (ADR-0021: one uniform, asset-class-agnostic streaming seam).
//!
//! # What the caller works with
//!
//! A [`RatesSubscription`] is a typed async [`futures_util::Stream`] of
//! [`RatesStreamEvent`]s: a first [`RatesStreamEvent::Snapshot`] carries the baseline
//! priced result at the subscribed curve (`curve_shift == 0`, faithful to
//! [`crate::Client::price_rates`] et al.), then [`RatesStreamEvent::Update`]s carry
//! the line re-priced against the baseline curve shifted by the tick's
//! `curve_shift`. The wire `oneof`, the per-subscription sequence numbers, and the
//! instrument-arm encoding are handled inside the SDK.
//!
//! The rates feed is **indicative** (PV + first-order risk); rates click-to-trade
//! books through the RFQ/desk path, so — unlike the FX price line — no tradable
//! token rides the stream and there is no click-to-trade off a [`RatesLine`].
//!
//! # Sequencing without a resync
//!
//! Each [`celnet_proto::RatesStreamUpdate`] carries a *complete* re-price (not an
//! incremental delta), and the server retains no per-line replay buffer for a rates
//! line (a rates gap re-subscribes; there is no dealable token to retire). A dropped
//! intermediate tick therefore loses no state — the next frame is a full, current
//! reprice — so the SDK surfaces rates frames in arrival order without the
//! gap-detect/`Resync` machinery the FX price line uses (which the server serves for
//! FX subscriptions only). The monotonic `sequence` is carried on every [`RatesLine`]
//! for the caller's own observability.

use std::pin::Pin;
use std::task::{Context, Poll};

use celnet_proto::{
    ClientStreamMessage, RatesInstrument, RatesStreamSnapshot, RatesStreamUpdate, SubscriptionId,
    Unsubscribe, client_stream_message,
};
use futures_util::Stream;
use tokio::sync::mpsc;

use crate::error::{ClientError, ClientResult};
use crate::rates::{BondSpec, FraSpec, IrsSpec, Ois, RatesPriced};

/// Seals [`RatesInstrumentSpec`] so only this crate's linear-rates specs implement
/// it — the streamable arms are exactly those the SDK can encode to a wire
/// [`RatesInstrument`], never an arbitrary external type.
mod sealed {
    pub trait Sealed {}
}

/// A linear-rates instrument spec that can open a streaming line — the OIS / vanilla
/// IRS / FRA / cash-bond arms of the one `RatesInstrument` contract. Implemented for
/// [`Ois`], [`IrsSpec`], [`FraSpec`], and [`BondSpec`]; sealed, so
/// [`crate::StreamSession::subscribe_rates`] streams exactly the arms the SDK prices
/// (the streaming analogue of [`crate::Client::price_rates`] /
/// [`crate::Client::price_irs`] / [`crate::Client::price_fra`] /
/// [`crate::Client::price_bond`]).
pub trait RatesInstrumentSpec: sealed::Sealed {
    /// Encode this spec to its wire [`RatesInstrument`] arm — the SAME encoder the
    /// request/response `price_*` methods use, so a streamed baseline is faithful to
    /// the priced result.
    #[doc(hidden)]
    fn to_rates_instrument(&self) -> RatesInstrument;
}

impl sealed::Sealed for Ois {}
impl RatesInstrumentSpec for Ois {
    fn to_rates_instrument(&self) -> RatesInstrument {
        (*self).to_wire()
    }
}

impl sealed::Sealed for IrsSpec {}
impl RatesInstrumentSpec for IrsSpec {
    fn to_rates_instrument(&self) -> RatesInstrument {
        (*self).to_wire()
    }
}

impl sealed::Sealed for FraSpec {}
impl RatesInstrumentSpec for FraSpec {
    fn to_rates_instrument(&self) -> RatesInstrument {
        (*self).to_wire()
    }
}

impl sealed::Sealed for BondSpec {}
impl RatesInstrumentSpec for BondSpec {
    fn to_rates_instrument(&self) -> RatesInstrument {
        (*self).to_wire()
    }
}

/// One priced line of a streamed fixed-income subscription at a sequence point: the
/// PV + first-order risk ([`RatesPriced`]) re-priced against the subscribed baseline
/// curve shifted by `curve_shift`. The typed payload shared by
/// [`RatesStreamEvent::Snapshot`] and [`RatesStreamEvent::Update`].
#[derive(Debug, Clone, PartialEq)]
pub struct RatesLine {
    /// The monotonic per-subscription sequence number of this line (the baseline
    /// snapshot is `1`; updates advance from there).
    pub sequence: u64,
    /// The priced PV + par rate + PV01 / DV01 / key-rate ladder at this line, in the
    /// curve currency and side-signed — identical in shape to the request/response
    /// [`crate::Client::price_rates`] result.
    pub priced: RatesPriced,
    /// The parallel curve shift (decimal, added to every calibrating pillar par rate)
    /// this line was re-priced at, relative to the subscribed baseline curve. `0` on
    /// the baseline snapshot; a signed shift on each update.
    pub curve_shift: f64,
    /// Line time, nanoseconds since the Unix epoch (UTC).
    pub epoch_nanos: i64,
}

/// One event surfaced by a [`RatesSubscription`] stream: the opening baseline, or a
/// live re-priced update.
#[derive(Debug, Clone, PartialEq)]
pub enum RatesStreamEvent {
    /// The baseline priced result at the subscribed curve (`curve_shift == 0`) — the
    /// state a caller applies whole before consuming updates. Its [`RatesPriced`]
    /// equals `price_rates(instrument, curve)` for the subscribed instrument/curve
    /// (the server prices the baseline through the identical landed path).
    Snapshot {
        /// The baseline line (sequence `1`, `curve_shift == 0`).
        line: RatesLine,
        /// Echo of the opening `subscribe_rates` correlation id, if one was supplied
        /// — a blotter joins this line to the request that opened it. Absent ⇒ none.
        correlation_id: Option<u64>,
    },
    /// A sequenced re-price advancing the subscription: the line re-priced against the
    /// baseline curve shifted by [`RatesLine::curve_shift`].
    Update(RatesLine),
}

/// The driver-side state for one open rates line: the typed-event channel to the
/// caller. Held in the session's rates registry, keyed by the line's subscription id
/// (the same id space as FX price + market-series subscriptions; ids never collide).
#[derive(Debug)]
pub(crate) struct RatesLineState {
    pub(crate) event_tx: mpsc::Sender<ClientResult<RatesStreamEvent>>,
}

/// Decode a wire [`RatesStreamSnapshot`] into a typed [`RatesStreamEvent::Snapshot`].
/// A snapshot missing its priced `result` is a typed error (never a fabricated
/// zero-price baseline).
pub(crate) fn decode_rates_snapshot(s: &RatesStreamSnapshot) -> ClientResult<RatesStreamEvent> {
    let result = s
        .result
        .clone()
        .ok_or(ClientError::MissingField("RatesStreamSnapshot.result"))?;
    Ok(RatesStreamEvent::Snapshot {
        line: RatesLine {
            sequence: s.sequence,
            priced: RatesPriced::from_wire(result),
            curve_shift: s.curve_shift,
            epoch_nanos: s.epoch_nanos,
        },
        correlation_id: s.correlation_id,
    })
}

/// Decode a wire [`RatesStreamUpdate`] into a typed [`RatesLine`]. An update missing
/// its priced `result` is a typed error (never a fabricated tick).
pub(crate) fn decode_rates_update(u: &RatesStreamUpdate) -> ClientResult<RatesLine> {
    let result = u
        .result
        .clone()
        .ok_or(ClientError::MissingField("RatesStreamUpdate.result"))?;
    Ok(RatesLine {
        sequence: u.sequence,
        priced: RatesPriced::from_wire(result),
        curve_shift: u.curve_shift,
        epoch_nanos: u.epoch_nanos,
    })
}

/// A live fixed-income streaming subscription on a [`crate::StreamSession`]: a typed
/// async stream of [`RatesStreamEvent`]s for one linear-rates instrument, multiplexed
/// over the session's one connection.
///
/// Construct via [`crate::StreamSession::subscribe_rates`] or the one-line
/// [`crate::Client::subscribe_rates`]. Dropping the subscription leaves the session
/// (and its other subscriptions) running; the driver stops routing to a dropped
/// line's closed channel. Call [`RatesSubscription::unsubscribe`] to tear it down
/// server-side explicitly (the rest of the session is unaffected).
#[derive(Debug)]
pub struct RatesSubscription {
    pub(crate) sub_id: u64,
    pub(crate) rx: mpsc::Receiver<ClientResult<RatesStreamEvent>>,
    /// A clone of the session's shared control sender: sends the teardown
    /// `Unsubscribe`, and — crucially — its liveness keeps the session's
    /// demultiplexing driver alive after the opening [`crate::StreamSession`] is
    /// dropped (exactly as a [`crate::MarketSeries`] outlives its session), so a
    /// caller can hold a rates line without pinning the session handle.
    pub(crate) control: mpsc::Sender<ClientStreamMessage>,
}

impl RatesSubscription {
    /// The per-session subscription id this rates line is keyed on.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.sub_id
    }

    /// Await the next stream event, or `None` once the line has ended (a clean
    /// unsubscribe, a server-side stream end, or a dropped/closed session).
    ///
    /// # Errors
    ///
    /// Yields a [`ClientError`] event if a wire frame could not be decoded or the
    /// server returned a status mid-stream.
    pub async fn next_event(&mut self) -> Option<ClientResult<RatesStreamEvent>> {
        self.rx.recv().await
    }

    /// Tear this rates line down server-side (the rest of the session is unaffected).
    /// After it, the stream ends; the caller may drop the handle.
    ///
    /// # Errors
    ///
    /// [`ClientError::StreamClosed`] if the session's control channel has closed.
    pub async fn unsubscribe(&self) -> ClientResult<()> {
        let msg = ClientStreamMessage {
            message: Some(client_stream_message::Message::Unsubscribe(Unsubscribe {
                subscription: Some(SubscriptionId { value: self.sub_id }),
            })),
        };
        self.control
            .send(msg)
            .await
            .map_err(|_| ClientError::StreamClosed)
    }
}

impl Stream for RatesSubscription {
    type Item = ClientResult<RatesStreamEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{RatesPricingResult, rates_instrument};

    fn sample_result() -> RatesPricingResult {
        RatesPricingResult {
            pv: -1234.5,
            par_rate: 0.0412,
            pv01: 480.0,
            dv01: -455.0,
            key_rate_ladder: vec![-100.0, -155.0, -200.0],
            ..Default::default()
        }
    }

    #[test]
    fn each_arm_encodes_its_own_rates_instrument() {
        // Every sealed arm encodes to exactly its own oneof variant — the streaming
        // analogue of the request/response `price_*` arms.
        assert!(matches!(
            Ois::receive_fixed(5, 0.04).to_rates_instrument().instrument,
            Some(rates_instrument::Instrument::Ois(_))
        ));
        assert!(matches!(
            IrsSpec::receive_fixed(5, 0.04)
                .to_rates_instrument()
                .instrument,
            Some(rates_instrument::Instrument::Irs(_))
        ));
        assert!(matches!(
            FraSpec::receive_fixed(3, 6, 0.033)
                .to_rates_instrument()
                .instrument,
            Some(rates_instrument::Instrument::Fra(_))
        ));
        assert!(matches!(
            BondSpec::long(0.06, crate::rates::CivilDate::new(2035, 6, 15))
                .to_rates_instrument()
                .instrument,
            Some(rates_instrument::Instrument::Bond(_))
        ));
    }

    #[test]
    fn snapshot_decodes_the_baseline_line() {
        let snap = RatesStreamSnapshot {
            subscription: Some(SubscriptionId { value: 3 }),
            sequence: 1,
            result: Some(sample_result()),
            curve_shift: 0.0,
            correlation_id: Some(9),
            epoch_nanos: 42,
        };
        match decode_rates_snapshot(&snap).expect("decodes") {
            RatesStreamEvent::Snapshot {
                line,
                correlation_id,
            } => {
                assert_eq!(line.sequence, 1);
                assert_eq!(line.curve_shift.to_bits(), 0.0f64.to_bits());
                assert_eq!(line.priced.pv, -1234.5);
                assert_eq!(line.priced.dv01, -455.0);
                assert_eq!(line.priced.key_rate_ladder, vec![-100.0, -155.0, -200.0]);
                assert_eq!(correlation_id, Some(9));
            }
            other => panic!("expected a Snapshot, got {other:?}"),
        }
    }

    #[test]
    fn update_decodes_the_shifted_line() {
        let upd = RatesStreamUpdate {
            subscription: Some(SubscriptionId { value: 3 }),
            sequence: 7,
            result: Some(sample_result()),
            curve_shift: 5.0e-5,
            epoch_nanos: 84,
        };
        let line = decode_rates_update(&upd).expect("decodes");
        assert_eq!(line.sequence, 7);
        assert_eq!(line.curve_shift, 5.0e-5);
        assert_eq!(line.priced.par_rate, 0.0412);
    }

    #[test]
    fn a_snapshot_without_a_result_is_a_typed_error() {
        let snap = RatesStreamSnapshot {
            subscription: Some(SubscriptionId { value: 3 }),
            sequence: 1,
            result: None,
            curve_shift: 0.0,
            correlation_id: None,
            epoch_nanos: 0,
        };
        assert!(matches!(
            decode_rates_snapshot(&snap),
            Err(ClientError::MissingField("RatesStreamSnapshot.result"))
        ));
    }

    #[test]
    fn an_update_without_a_result_is_a_typed_error() {
        let upd = RatesStreamUpdate {
            subscription: Some(SubscriptionId { value: 3 }),
            sequence: 2,
            result: None,
            curve_shift: 1.0e-4,
            epoch_nanos: 0,
        };
        assert!(matches!(
            decode_rates_update(&upd),
            Err(ClientError::MissingField("RatesStreamUpdate.result"))
        ));
    }
}
