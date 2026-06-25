//! The fixed-income (linear-rates) dialect mapping: FIX instrument blocks ⇄ the
//! Celnet rates RFQ / RFS / order vocabulary, for the USD-SOFR OIS P0 arm.
//!
//! This is the rates analogue of [`crate::dialect_fx`]: pure data + arithmetic,
//! reused by the acceptor (decode an inbound `QuoteRequest(R)` into a [`RatesRfq`]
//! and reply a `Quote(S)`) and the initiator (encode an RFQ / RFS subscribe) alike.
//! Nothing here touches a socket or prices anything — the edge supplies the curve
//! and prices via `celnet-rates`, exactly as the FX edge supplies a surface
//! snapshot. Keeping the dialect pricing-free keeps `celnet-fix` a thin wire leaf
//! and the single rates pricing path (the engine) authoritative.
//!
//! ## Wire shape
//!
//! An OIS RFQ is fully wire-specified and date-independent: `SecurityType(167)`
//! selects the rates instrument, the dialect's [`TAG_TENOR_YEARS`] carries the
//! whole-year tenor (so the priced par rate reproduces the engine to the bit,
//! independent of any trade date — the same provenance discipline as the FX
//! dialect's expiry tag), `OrderQty(38)` carries the notional, `Side(54)` selects
//! pay-fixed / receive-fixed (absent ⇒ a two-way request), and
//! `SubscriptionRequestType(263)` distinguishes a one-shot RFQ from an RFS
//! subscribe / unsubscribe. An order (`NewOrderSingle(D)`) references the live
//! `QuoteID(117)` exactly as the FX path, so the maker's last-look token ledger
//! books a rates lift with no rates-specific order decode.

use crate::dialect_fx::{SIDE_BUY, SIDE_SELL, parse_float};
use crate::dictionary::MsgType;
use crate::framing::{FrameCursor, FrameEncoder};
use crate::messages::Header;

/// FIX `Product(460)` value for a rate instrument.
pub const PRODUCT_RATE: u32 = 5;
/// FIX `SecurityType(167)` for an overnight-indexed swap.
pub const SEC_TYPE_OIS: &[u8] = b"OIS";
/// FIX `SubscriptionRequestType(263)`: snapshot (a one-shot RFQ).
pub const SUB_REQ_SNAPSHOT: &[u8] = b"0";
/// FIX `SubscriptionRequestType(263)`: snapshot + updates (an RFS subscribe).
pub const SUB_REQ_SUBSCRIBE: &[u8] = b"1";
/// FIX `SubscriptionRequestType(263)`: disable previous snapshot (an RFS unsubscribe).
pub const SUB_REQ_UNSUBSCRIBE: &[u8] = b"2";

/// The custom dialect tag carrying the OIS tenor in **whole years** as a FIX int.
///
/// As with the FX dialect's expiry tag, the platform prices off a tenor rather
/// than a calendar resolution of `MaturityDate(541)` (which would drift with the
/// trade date). Carrying the integer tenor on a private, user-defined tag (FIX
/// tolerates unknown tags; this is a dialect provenance field, never a vendor
/// name — `CLAUDE.md` rule 8) makes the RFQ fully wire-specified, so the returned
/// par rate reproduces the engine/golden value independent of any date. This is
/// the canonical home of the tag; the server's FIX edge reads it from here.
pub const TAG_TENOR_YEARS: u32 = 7101;

/// The client's directional intent on the fixed leg of an OIS RFQ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatesSide {
    /// `Side(54)=1` — the client pays fixed (payer swap; long the floating leg).
    PayFixed,
    /// `Side(54)=2` — the client receives fixed (receiver swap).
    ReceiveFixed,
    /// `Side(54)` absent — the client requests a two-way (bid/offer) market.
    TwoWay,
}

impl RatesSide {
    /// Map the raw `Side(54)` value (absent ⇒ two-way request) to a [`RatesSide`].
    ///
    /// # Errors
    ///
    /// Returns [`RatesDialectError::BadSide`] for a present-but-unrecognised side.
    pub fn from_fix(side: Option<&[u8]>) -> Result<Self, RatesDialectError> {
        match side {
            None => Ok(Self::TwoWay),
            Some([SIDE_BUY]) => Ok(Self::PayFixed),
            Some([SIDE_SELL]) => Ok(Self::ReceiveFixed),
            Some(_) => Err(RatesDialectError::BadSide),
        }
    }

    /// The `Side(54)` byte this side encodes as, or `None` for a two-way request.
    #[must_use]
    pub fn to_fix(self) -> Option<u8> {
        match self {
            Self::PayFixed => Some(SIDE_BUY),
            Self::ReceiveFixed => Some(SIDE_SELL),
            Self::TwoWay => None,
        }
    }
}

/// The `SubscriptionRequestType(263)` intent: a one-shot RFQ vs an RFS stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionRequest {
    /// `263=0` (or absent) — a one-shot snapshot RFQ.
    Snapshot,
    /// `263=1` — subscribe to a streaming RFS (snapshot + updates).
    Subscribe,
    /// `263=2` — unsubscribe from a previously requested RFS.
    Unsubscribe,
}

impl SubscriptionRequest {
    /// Map the raw `SubscriptionRequestType(263)` value (absent ⇒ snapshot).
    ///
    /// # Errors
    ///
    /// Returns [`RatesDialectError::BadSubscription`] for an unrecognised value.
    pub fn from_fix(v: Option<&[u8]>) -> Result<Self, RatesDialectError> {
        match v {
            None => Ok(Self::Snapshot),
            Some(SUB_REQ_SNAPSHOT) => Ok(Self::Snapshot),
            Some(SUB_REQ_SUBSCRIBE) => Ok(Self::Subscribe),
            Some(SUB_REQ_UNSUBSCRIBE) => Ok(Self::Unsubscribe),
            Some(_) => Err(RatesDialectError::BadSubscription),
        }
    }

    /// The `SubscriptionRequestType(263)` byte string this intent encodes as.
    #[must_use]
    pub fn to_fix(self) -> &'static [u8] {
        match self {
            Self::Snapshot => SUB_REQ_SNAPSHOT,
            Self::Subscribe => SUB_REQ_SUBSCRIBE,
            Self::Unsubscribe => SUB_REQ_UNSUBSCRIBE,
        }
    }
}

/// A rates dialect mapping error. Recoverable; never panics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatesDialectError {
    /// `QuoteReqID(131)` was missing.
    MissingQuoteReqId,
    /// `Symbol(55)` was missing.
    MissingSymbol,
    /// `SecurityType(167)` was missing or not a rates instrument type.
    BadSecurityType,
    /// `[TAG_TENOR_YEARS]` was missing, unparseable, or `< 1`.
    BadTenor,
    /// `OrderQty(38)` (notional) was missing, unparseable, or not `> 0`.
    BadNotional,
    /// `Side(54)` was present but not `1` (pay fixed) or `2` (receive fixed).
    BadSide,
    /// `SubscriptionRequestType(263)` was present but not `0` / `1` / `2`.
    BadSubscription,
}

/// Parameters for an OIS `QuoteRequest(R)` — the symmetric encode side of
/// [`decode_rates_rfq`]. An initiator (price-taker / test client) fills these and
/// the builder lays them onto the wire flat, exactly as the acceptor reads them.
#[derive(Debug, Clone, Copy)]
pub struct RatesQuoteRequestParams<'a> {
    /// `QuoteReqID(131)` — the client-minted RFQ correlation id.
    pub quote_req_id: &'a [u8],
    /// `Symbol(55)` — the curve symbol, e.g. `b"USD-OIS"`.
    pub symbol: &'a [u8],
    /// The OIS tenor in whole years (`[TAG_TENOR_YEARS]`; must be `>= 1`).
    pub tenor_years: u32,
    /// `OrderQty(38)` — the notional (must be `> 0`).
    pub notional: f64,
    /// The directional intent (absent `Side(54)` ⇒ two-way request).
    pub side: RatesSide,
    /// One-shot RFQ vs RFS subscribe / unsubscribe (`SubscriptionRequestType(263)`).
    pub subscription: SubscriptionRequest,
}

/// Build a flat OIS `QuoteRequest(R)` frame from [`RatesQuoteRequestParams`].
///
/// The layout mirrors what [`decode_rates_rfq`] reads: `QuoteReqID(131)`,
/// `Symbol(55)`, `Product(460)=5`, `SecurityType(167)=OIS`, `OrderQty(38)`,
/// the dialect's [`TAG_TENOR_YEARS`], `SubscriptionRequestType(263)`, and
/// `Side(54)` when the request is directional. A whole-number notional is
/// emitted as a FIX int; a fractional notional is re-emitted as a precise decimal
/// so the exact value survives the wire (tag 38 is read as a float on decode).
#[must_use]
pub fn build_rates_quote_request(
    hdr: &Header<'_>,
    p: &RatesQuoteRequestParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::QuoteRequest, enc);
    enc.push(131, p.quote_req_id);
    enc.push(55, p.symbol);
    enc.push(460, b"5"); // Product = RATE
    enc.push(167, SEC_TYPE_OIS);
    if is_integer_notional(p.notional) {
        enc.push_int(38, p.notional as i64);
    } else {
        enc.push(38, format!("{}", p.notional).as_bytes());
    }
    enc.push_int(TAG_TENOR_YEARS, i64::from(p.tenor_years));
    enc.push(263, p.subscription.to_fix());
    if let Some(side) = p.side.to_fix() {
        enc.push(54, &[side]);
    }
    enc.finish()
}

/// A single OIS RFQ descriptor decoded from the FIX instrument block — the
/// dialect's normalised view of one inbound rates request.
#[derive(Debug, Clone, PartialEq)]
pub struct RatesRfq {
    /// `QuoteReqID(131)` — the client RFQ correlation id.
    pub quote_req_id: Vec<u8>,
    /// `Symbol(55)` — the curve symbol.
    pub symbol: Vec<u8>,
    /// The OIS tenor in whole years.
    pub tenor_years: u32,
    /// The notional (`OrderQty(38)`).
    pub notional: f64,
    /// The directional intent.
    pub side: RatesSide,
    /// One-shot RFQ vs RFS subscribe / unsubscribe.
    pub subscription: SubscriptionRequest,
}

/// Decode + validate an inbound OIS `QuoteRequest(R)` into a [`RatesRfq`].
///
/// # Errors
///
/// Returns a [`RatesDialectError`] for any missing/invalid required field. A
/// malformed request never panics and never yields a partial descriptor.
pub fn decode_rates_rfq(frame: &FrameCursor<'_>) -> Result<RatesRfq, RatesDialectError> {
    let quote_req_id = frame
        .get(131)
        .map(<[u8]>::to_vec)
        .ok_or(RatesDialectError::MissingQuoteReqId)?;
    let symbol = frame
        .get(55)
        .map(<[u8]>::to_vec)
        .ok_or(RatesDialectError::MissingSymbol)?;

    match frame.get(167) {
        Some(SEC_TYPE_OIS) => {}
        _ => return Err(RatesDialectError::BadSecurityType),
    }

    let tenor_years = frame
        .get(TAG_TENOR_YEARS)
        .and_then(parse_u32)
        .filter(|t| *t >= 1)
        .ok_or(RatesDialectError::BadTenor)?;

    let notional = frame
        .get(38)
        .and_then(parse_float)
        .filter(|n| *n > 0.0 && n.is_finite())
        .ok_or(RatesDialectError::BadNotional)?;

    let side = RatesSide::from_fix(frame.get(54))?;
    let subscription = SubscriptionRequest::from_fix(frame.get(263))?;

    Ok(RatesRfq {
        quote_req_id,
        symbol,
        tenor_years,
        notional,
        side,
        subscription,
    })
}

/// Split a fair (par) rate into a two-way bid/offer market around it: the client
/// receives fixed at the `bid` and pays fixed at the `offer`, each a `half_spread`
/// (in absolute rate, e.g. `0.0001` = 1bp) either side of par. Returns
/// `(bid, offer)` with `bid <= offer`.
#[must_use]
pub fn two_way_rates(par_rate: f64, half_spread: f64) -> (f64, f64) {
    let h = half_spread.abs();
    (par_rate - h, par_rate + h)
}

/// Parse a strict non-negative FIX integer from value bytes.
fn parse_u32(v: &[u8]) -> Option<u32> {
    let s = core::str::from_utf8(v).ok()?;
    s.parse().ok()
}

/// Whether a notional is a whole number small enough to round-trip through `i64`
/// without losing its fractional zero (so it can be emitted with `push_int`).
fn is_integer_notional(notional: f64) -> bool {
    notional.fract() == 0.0 && notional.abs() < 9.007_199_254_740_992e15
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::Header;

    fn header() -> Header<'static> {
        Header {
            sender: b"CELNET",
            target: b"CELNET-CPTY",
            seq_num: 7,
            sending_time: b"20260625-12:00:00.000",
        }
    }

    fn round_trip(params: &RatesQuoteRequestParams<'_>) -> RatesRfq {
        let mut enc = FrameEncoder::new();
        let raw = build_rates_quote_request(&header(), params, &mut enc);
        let frame = FrameCursor::parse(&raw).expect("frame parses");
        decode_rates_rfq(&frame).expect("rfq decodes")
    }

    #[test]
    fn one_way_pay_fixed_round_trips() {
        let p = RatesQuoteRequestParams {
            quote_req_id: b"RFQ-1",
            symbol: b"USD-OIS",
            tenor_years: 5,
            notional: 100_000_000.0,
            side: RatesSide::PayFixed,
            subscription: SubscriptionRequest::Snapshot,
        };
        let rfq = round_trip(&p);
        assert_eq!(rfq.quote_req_id, b"RFQ-1");
        assert_eq!(rfq.symbol, b"USD-OIS");
        assert_eq!(rfq.tenor_years, 5);
        assert_eq!(rfq.notional, 100_000_000.0);
        assert_eq!(rfq.side, RatesSide::PayFixed);
        assert_eq!(rfq.subscription, SubscriptionRequest::Snapshot);
    }

    #[test]
    fn two_way_request_omits_side() {
        let p = RatesQuoteRequestParams {
            quote_req_id: b"RFQ-2",
            symbol: b"USD-OIS",
            tenor_years: 10,
            notional: 250_000_000.0,
            side: RatesSide::TwoWay,
            subscription: SubscriptionRequest::Snapshot,
        };
        let rfq = round_trip(&p);
        assert_eq!(rfq.side, RatesSide::TwoWay);
        assert_eq!(rfq.tenor_years, 10);
    }

    #[test]
    fn rfs_subscribe_round_trips() {
        let p = RatesQuoteRequestParams {
            quote_req_id: b"RFS-1",
            symbol: b"USD-OIS",
            tenor_years: 2,
            notional: 50_000_000.0,
            side: RatesSide::ReceiveFixed,
            subscription: SubscriptionRequest::Subscribe,
        };
        let rfq = round_trip(&p);
        assert_eq!(rfq.subscription, SubscriptionRequest::Subscribe);
        assert_eq!(rfq.side, RatesSide::ReceiveFixed);
    }

    #[test]
    fn fractional_notional_survives() {
        let p = RatesQuoteRequestParams {
            quote_req_id: b"RFQ-3",
            symbol: b"USD-OIS",
            tenor_years: 3,
            notional: 12_345_678.5,
            side: RatesSide::PayFixed,
            subscription: SubscriptionRequest::Snapshot,
        };
        let rfq = round_trip(&p);
        assert_eq!(rfq.notional, 12_345_678.5);
    }

    #[test]
    fn rejects_wrong_security_type() {
        // Hand-build a QuoteRequest carrying SecurityType=FXVO (an FX option).
        let mut enc = FrameEncoder::new();
        enc.clear();
        header().encode(MsgType::QuoteRequest, &mut enc);
        enc.push(131, b"RFQ-X");
        enc.push(55, b"USD-OIS");
        enc.push(167, b"FXVO");
        enc.push_int(TAG_TENOR_YEARS, 5);
        enc.push_int(38, 100);
        let raw = enc.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(
            decode_rates_rfq(&frame),
            Err(RatesDialectError::BadSecurityType)
        );
    }

    #[test]
    fn rejects_zero_tenor() {
        let p = RatesQuoteRequestParams {
            quote_req_id: b"RFQ-4",
            symbol: b"USD-OIS",
            tenor_years: 0,
            notional: 100.0,
            side: RatesSide::PayFixed,
            subscription: SubscriptionRequest::Snapshot,
        };
        let mut enc = FrameEncoder::new();
        let raw = build_rates_quote_request(&header(), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(decode_rates_rfq(&frame), Err(RatesDialectError::BadTenor));
    }

    #[test]
    fn rejects_non_positive_notional() {
        let mut enc = FrameEncoder::new();
        enc.clear();
        header().encode(MsgType::QuoteRequest, &mut enc);
        enc.push(131, b"RFQ-5");
        enc.push(55, b"USD-OIS");
        enc.push(167, SEC_TYPE_OIS);
        enc.push_int(TAG_TENOR_YEARS, 5);
        enc.push_int(38, 0);
        let raw = enc.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(
            decode_rates_rfq(&frame),
            Err(RatesDialectError::BadNotional)
        );
    }

    #[test]
    fn rejects_bad_side() {
        let mut enc = FrameEncoder::new();
        enc.clear();
        header().encode(MsgType::QuoteRequest, &mut enc);
        enc.push(131, b"RFQ-6");
        enc.push(55, b"USD-OIS");
        enc.push(167, SEC_TYPE_OIS);
        enc.push_int(TAG_TENOR_YEARS, 5);
        enc.push_int(38, 100);
        enc.push(54, b"9"); // not 1 or 2
        let raw = enc.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(decode_rates_rfq(&frame), Err(RatesDialectError::BadSide));
    }

    #[test]
    fn two_way_rates_brackets_par() {
        let (bid, offer) = two_way_rates(0.0405, 0.0001);
        assert!((bid - 0.0404).abs() < 1e-12);
        assert!((offer - 0.0406).abs() < 1e-12);
        assert!(bid <= offer);
    }
}
