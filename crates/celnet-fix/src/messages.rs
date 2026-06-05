//! Typed borrowed views and owned builders for the FX-options dialect messages
//! (`QuoteRequest`, `Quote`, `MassQuote`, `NewOrderSingle`, `NewOrderMultileg`,
//! `QuoteCancel`, `ExecutionReport`) and the session admin messages.
//!
//! Views borrow over a [`FrameCursor`] — zero-copy, no per-message allocation.
//! Builders write into a reusable [`FrameEncoder`] and produce the complete
//! on-wire bytes with a correct `BodyLength`/`CheckSum`. The header (`MsgType`,
//! `SenderCompID`, `TargetCompID`, `MsgSeqNum`, `SendingTime`) is laid out by
//! [`Header`] so the session layer can stamp sequence numbers uniformly.

use crate::dictionary::MsgType;
use crate::framing::{FrameCursor, FrameEncoder};

/// The standard FIX header fields every outbound message carries. Laid out
/// immediately after `MsgType(35)` by every builder so the session FSM can
/// stamp the sequence number and timestamp consistently.
#[derive(Debug, Clone, Copy)]
pub struct Header<'a> {
    /// `SenderCompID(49)`.
    pub sender: &'a [u8],
    /// `TargetCompID(56)`.
    pub target: &'a [u8],
    /// `MsgSeqNum(34)`.
    pub seq_num: u64,
    /// `SendingTime(52)` (UTC timestamp bytes; supplied by the caller's clock).
    pub sending_time: &'a [u8],
}

impl Header<'_> {
    /// Write `35`, `49`, `56`, `34`, `52` into the encoder in canonical order.
    pub fn encode(&self, mt: MsgType, enc: &mut FrameEncoder) {
        enc.push(35, mt.as_bytes());
        enc.push(49, self.sender);
        enc.push(56, self.target);
        enc.push_int(34, self.seq_num as i64);
        enc.push(52, self.sending_time);
    }
}

// ---------------------------------------------------------------------------
// Session admin builders
// ---------------------------------------------------------------------------

/// Build a `Logon(A)` frame with `EncryptMethod(98)=0` and the given heartbeat
/// interval and optional sequence reset.
#[must_use]
pub fn build_logon(
    hdr: &Header<'_>,
    heart_bt_int: u32,
    reset_seq: bool,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::Logon, enc);
    enc.push_int(98, 0);
    enc.push_int(108, i64::from(heart_bt_int));
    if reset_seq {
        enc.push(141, b"Y");
    }
    enc.finish()
}

/// Build a `Logout(5)` frame with an optional `Text(58)` reason.
#[must_use]
pub fn build_logout(hdr: &Header<'_>, text: Option<&[u8]>, enc: &mut FrameEncoder) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::Logout, enc);
    if let Some(t) = text {
        enc.push(58, t);
    }
    enc.finish()
}

/// Build a `Heartbeat(0)` frame, echoing `TestReqID(112)` when responding to a
/// `TestRequest`.
#[must_use]
pub fn build_heartbeat(
    hdr: &Header<'_>,
    test_req_id: Option<&[u8]>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::Heartbeat, enc);
    if let Some(id) = test_req_id {
        enc.push(112, id);
    }
    enc.finish()
}

/// Build a `TestRequest(1)` frame with a `TestReqID(112)`.
#[must_use]
pub fn build_test_request(hdr: &Header<'_>, test_req_id: &[u8], enc: &mut FrameEncoder) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::TestRequest, enc);
    enc.push(112, test_req_id);
    enc.finish()
}

/// Build a `ResendRequest(2)` for the half-open range `[begin, end]` (`end=0`
/// means "until infinity", per FIX).
#[must_use]
pub fn build_resend_request(
    hdr: &Header<'_>,
    begin: u64,
    end: u64,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::ResendRequest, enc);
    enc.push_int(7, begin as i64);
    enc.push_int(16, end as i64);
    enc.finish()
}

/// Build a gap-fill `SequenceReset(4)` advancing the sequence to `new_seq_no`.
/// `GapFillFlag(123)=Y` marks it as a gap fill (not a hard reset).
#[must_use]
pub fn build_sequence_reset(
    hdr: &Header<'_>,
    new_seq_no: u64,
    gap_fill: bool,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::SequenceReset, enc);
    if gap_fill {
        enc.push(123, b"Y");
    }
    enc.push_int(36, new_seq_no as i64);
    enc.finish()
}

// ---------------------------------------------------------------------------
// Application message views
// ---------------------------------------------------------------------------

/// A borrowed view over a `QuoteRequest(R)` frame.
#[derive(Debug, Clone, Copy)]
pub struct QuoteRequestView<'a> {
    frame: FrameCursor<'a>,
}

impl<'a> QuoteRequestView<'a> {
    /// Wrap a frame already confirmed to be a `QuoteRequest`.
    #[must_use]
    pub fn new(frame: FrameCursor<'a>) -> Self {
        Self { frame }
    }

    /// `QuoteReqID(131)`.
    #[must_use]
    pub fn quote_req_id(&self) -> Option<&'a [u8]> {
        self.frame.get(131)
    }

    /// `QuoteType(537)` — indicative vs tradeable.
    #[must_use]
    pub fn quote_type(&self) -> Option<&'a [u8]> {
        self.frame.get(537)
    }

    /// Whether this is a multi-leg request (`NoLegs(555)` present).
    #[must_use]
    pub fn is_multileg(&self) -> bool {
        self.frame.get(555).is_some()
    }

    /// The underlying frame, for dialect decoding.
    #[must_use]
    pub fn frame(&self) -> &FrameCursor<'a> {
        &self.frame
    }
}

/// Dialect tag carrying the **round-trip-exact** bid premium (full f64 precision),
/// alongside the human-readable 8-dp `BidPx(132)`. FX-option premia are quoted to
/// pips on the standard price tags, but the maker's *booked* premium is the exact
/// model value; this user-defined field carries that exact value so a counterparty (or
/// an audit) can reconcile the fill to the engine/golden price to the last bit. A FIX
/// receiver that does not understand the tag simply ignores it (unknown tags are
/// tolerated); it never alters the standard price fields.
pub const TAG_BID_EXACT: u32 = 7011;
/// Dialect tag carrying the round-trip-exact offer premium (see [`TAG_BID_EXACT`]).
pub const TAG_OFFER_EXACT: u32 = 7012;
/// Dialect tag carrying the round-trip-exact `LastPx` fill premium on an
/// `ExecutionReport(8)` (see [`TAG_BID_EXACT`]).
pub const TAG_LAST_PX_EXACT: u32 = 7013;

/// Build a `Quote(S)` reply: a two-sided price with a `QuoteID(117)` carrying
/// last-look identity and a `ValidUntilTime(62)`.
#[derive(Debug, Clone, Copy)]
pub struct QuoteParams<'a> {
    /// Echoed `QuoteReqID(131)`.
    pub quote_req_id: &'a [u8],
    /// Minted `QuoteID(117)` (last-look / idempotency anchor).
    pub quote_id: &'a [u8],
    /// `Symbol(55)`.
    pub symbol: &'a [u8],
    /// Bid price (`BidPx(132)`).
    pub bid_px: f64,
    /// Offer price (`OfferPx(133)`).
    pub offer_px: f64,
    /// Quote size (`BidSize(134)`/`OfferSize(135)`).
    pub size: f64,
    /// `ValidUntilTime(62)` bytes (UTC timestamp).
    pub valid_until: &'a [u8],
}

/// Render a price as a **round-trip-exact** decimal (Rust's `{}` f64 formatting emits
/// the shortest decimal that parses back to the identical bits), so the exact model
/// premium survives the wire with no precision loss. Used only for the dialect's
/// exact-premium provenance tags, never for the standard pip-resolution price fields.
fn push_exact(enc: &mut FrameEncoder, tag: u32, v: f64) {
    if v.is_finite() {
        enc.push(tag, format!("{v}").as_bytes());
    }
}

/// Encode a fixed-precision decimal (8 dp) for a price/size field. FX-option
/// premia are fine at 8 dp; this avoids `format!`/locale and is deterministic.
fn push_decimal(enc: &mut FrameEncoder, tag: u32, v: f64) {
    // Render with a fixed 8-decimal layout via integer scaling.
    let neg = v.is_sign_negative();
    let scaled = (v.abs() * 1e8).round() as u128;
    let int_part = scaled / 100_000_000;
    let frac_part = scaled % 100_000_000;
    let mut s = Vec::with_capacity(24);
    if neg && scaled != 0 {
        s.push(b'-');
    }
    let mut tmp = itoa(int_part);
    s.append(&mut tmp);
    s.push(b'.');
    // 8 zero-padded fractional digits.
    let mut frac = [b'0'; 8];
    let mut f = frac_part;
    let mut idx = 8;
    while f > 0 && idx > 0 {
        idx -= 1;
        frac[idx] = b'0' + (f % 10) as u8;
        f /= 10;
    }
    s.extend_from_slice(&frac);
    enc.push(tag, &s);
}

/// Decimal digits of a `u128` as bytes (no leading zeros, `0` for zero).
fn itoa(mut v: u128) -> Vec<u8> {
    if v == 0 {
        return vec![b'0'];
    }
    let mut buf = Vec::with_capacity(40);
    while v > 0 {
        buf.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
    buf.reverse();
    buf
}

/// Build a `Quote(S)` frame from [`QuoteParams`].
#[must_use]
pub fn build_quote(hdr: &Header<'_>, p: &QuoteParams<'_>, enc: &mut FrameEncoder) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::Quote, enc);
    enc.push(131, p.quote_req_id);
    enc.push(117, p.quote_id);
    enc.push(55, p.symbol);
    push_decimal(enc, 132, p.bid_px);
    push_decimal(enc, 133, p.offer_px);
    push_decimal(enc, 134, p.size);
    push_decimal(enc, 135, p.size);
    enc.push(62, p.valid_until);
    // Exact-premium provenance: the booked two-way to full f64 precision, so a fill
    // reconciles to the engine/golden price to the last bit (the standard 132/133 stay
    // pip-resolution).
    push_exact(enc, TAG_BID_EXACT, p.bid_px);
    push_exact(enc, TAG_OFFER_EXACT, p.offer_px);
    enc.finish()
}

/// A borrowed view over a `Quote(S)` frame.
#[derive(Debug, Clone, Copy)]
pub struct QuoteView<'a> {
    frame: FrameCursor<'a>,
}

impl<'a> QuoteView<'a> {
    /// Wrap a frame confirmed to be a `Quote`.
    #[must_use]
    pub fn new(frame: FrameCursor<'a>) -> Self {
        Self { frame }
    }

    /// `QuoteID(117)`.
    #[must_use]
    pub fn quote_id(&self) -> Option<&'a [u8]> {
        self.frame.get(117)
    }

    /// `Symbol(55)`.
    #[must_use]
    pub fn symbol(&self) -> Option<&'a [u8]> {
        self.frame.get(55)
    }

    /// `BidPx(132)` as bytes.
    #[must_use]
    pub fn bid_px(&self) -> Option<&'a [u8]> {
        self.frame.get(132)
    }

    /// `OfferPx(133)` as bytes.
    #[must_use]
    pub fn offer_px(&self) -> Option<&'a [u8]> {
        self.frame.get(133)
    }

    /// Parsed bid price.
    #[must_use]
    pub fn bid(&self) -> Option<f64> {
        self.bid_px().and_then(crate::dialect_fx::parse_float)
    }

    /// Parsed offer price.
    #[must_use]
    pub fn offer(&self) -> Option<f64> {
        self.offer_px().and_then(crate::dialect_fx::parse_float)
    }

    /// The round-trip-exact bid premium ([`TAG_BID_EXACT`]), if the maker stamped it
    /// (full f64 precision, for last-bit reconciliation to the engine/golden price).
    #[must_use]
    pub fn bid_exact(&self) -> Option<f64> {
        self.frame
            .get(TAG_BID_EXACT)
            .and_then(crate::dialect_fx::parse_float)
    }

    /// The round-trip-exact offer premium ([`TAG_OFFER_EXACT`]), if the maker stamped
    /// it (full f64 precision).
    #[must_use]
    pub fn offer_exact(&self) -> Option<f64> {
        self.frame
            .get(TAG_OFFER_EXACT)
            .and_then(crate::dialect_fx::parse_float)
    }
}

/// One entry of a `MassQuote(i)` — a symbol with a two-sided price. The builder
/// emits a simplified single-`QuoteSet` mass quote sufficient for the FX-option
/// RFS stream (a flat run of `QuoteEntry`s under one `QuoteID`).
#[derive(Debug, Clone, Copy)]
pub struct MassQuoteEntry<'a> {
    /// `Symbol(55)` for this entry.
    pub symbol: &'a [u8],
    /// Bid price.
    pub bid_px: f64,
    /// Offer price.
    pub offer_px: f64,
}

/// Build a `MassQuote(i)` frame carrying a list of two-sided entries under one
/// `QuoteID(117)`.
#[must_use]
pub fn build_mass_quote(
    hdr: &Header<'_>,
    quote_id: &[u8],
    entries: &[MassQuoteEntry<'_>],
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::MassQuote, enc);
    enc.push(117, quote_id);
    for e in entries {
        enc.push(55, e.symbol);
        push_decimal(enc, 132, e.bid_px);
        push_decimal(enc, 133, e.offer_px);
    }
    enc.finish()
}

/// A borrowed view over a `NewOrderSingle(D)` frame (executes against a quote).
#[derive(Debug, Clone, Copy)]
pub struct NewOrderSingleView<'a> {
    frame: FrameCursor<'a>,
}

impl<'a> NewOrderSingleView<'a> {
    /// Wrap a frame confirmed to be a `NewOrderSingle`.
    #[must_use]
    pub fn new(frame: FrameCursor<'a>) -> Self {
        Self { frame }
    }

    /// `ClOrdID(11)`.
    #[must_use]
    pub fn cl_ord_id(&self) -> Option<&'a [u8]> {
        self.frame.get(11)
    }

    /// `QuoteID(117)` the order is executing against (last-look anchor).
    #[must_use]
    pub fn quote_id(&self) -> Option<&'a [u8]> {
        self.frame.get(117)
    }

    /// `Side(54)`.
    #[must_use]
    pub fn side(&self) -> Option<u8> {
        self.frame.get(54).and_then(|v| v.first().copied())
    }

    /// `OrderQty(38)`.
    #[must_use]
    pub fn order_qty(&self) -> Option<f64> {
        self.frame.get(38).and_then(crate::dialect_fx::parse_float)
    }

    /// The underlying frame, for dialect decoding of the instrument block.
    #[must_use]
    pub fn frame(&self) -> &FrameCursor<'a> {
        &self.frame
    }
}

/// Build a `NewOrderSingle(D)` executing against a live `QuoteID`.
#[derive(Debug, Clone, Copy)]
pub struct NewOrderParams<'a> {
    /// `ClOrdID(11)`.
    pub cl_ord_id: &'a [u8],
    /// `QuoteID(117)` being lifted.
    pub quote_id: &'a [u8],
    /// `Symbol(55)`.
    pub symbol: &'a [u8],
    /// `Side(54)` byte.
    pub side: u8,
    /// `OrderQty(38)`.
    pub qty: f64,
    /// `TransactTime(60)` bytes.
    pub transact_time: &'a [u8],
}

/// Build a `NewOrderSingle(D)` frame from [`NewOrderParams`].
#[must_use]
pub fn build_new_order_single(
    hdr: &Header<'_>,
    p: &NewOrderParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::NewOrderSingle, enc);
    enc.push(11, p.cl_ord_id);
    enc.push(117, p.quote_id);
    enc.push(55, p.symbol);
    enc.push(54, &[p.side]);
    push_decimal(enc, 38, p.qty);
    enc.push(40, b"D"); // OrdType = previously quoted
    enc.push(60, p.transact_time);
    enc.finish()
}

/// `ExecType(150)` / `OrdStatus(39)`: filled.
pub const EXEC_FILLED: u8 = b'F';
/// `ExecType(150)` / `OrdStatus(39)`: rejected.
pub const EXEC_REJECTED: u8 = b'8';

/// Build an `ExecutionReport(8)` for a single fill (or rejection).
#[derive(Debug, Clone, Copy)]
pub struct ExecReportParams<'a> {
    /// `OrderID(37)` minted by the venue.
    pub order_id: &'a [u8],
    /// `ExecID(17)`.
    pub exec_id: &'a [u8],
    /// Echoed `ClOrdID(11)`.
    pub cl_ord_id: &'a [u8],
    /// `ExecType(150)` byte.
    pub exec_type: u8,
    /// `OrdStatus(39)` byte.
    pub ord_status: u8,
    /// `Symbol(55)`.
    pub symbol: &'a [u8],
    /// `Side(54)`.
    pub side: u8,
    /// `LastQty(32)` filled.
    pub last_qty: f64,
    /// `LastPx(31)` fill price (the locked quote premium).
    pub last_px: f64,
    /// Optional `MultiLegReportingType(442)` for package fills.
    pub multileg_type: Option<i64>,
    /// Optional `Text(58)` (rejection reason).
    pub text: Option<&'a [u8]>,
}

/// Build an `ExecutionReport(8)` frame from [`ExecReportParams`].
#[must_use]
pub fn build_execution_report(
    hdr: &Header<'_>,
    p: &ExecReportParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::ExecutionReport, enc);
    enc.push(37, p.order_id);
    enc.push(17, p.exec_id);
    enc.push(11, p.cl_ord_id);
    enc.push(150, &[p.exec_type]);
    enc.push(39, &[p.ord_status]);
    enc.push(55, p.symbol);
    enc.push(54, &[p.side]);
    push_decimal(enc, 32, p.last_qty);
    push_decimal(enc, 31, p.last_px);
    // Exact-premium provenance: the locked fill premium to full f64 precision, so the
    // booked fill reconciles to the engine/golden price to the last bit (the standard
    // `LastPx(31)` stays pip-resolution). Emitted on every report; a reject's
    // `last_px` is zero and reconciles trivially.
    push_exact(enc, TAG_LAST_PX_EXACT, p.last_px);
    if let Some(t) = p.multileg_type {
        enc.push_int(442, t);
    }
    if let Some(t) = p.text {
        enc.push(58, t);
    }
    enc.finish()
}

/// A borrowed view over an `ExecutionReport(8)` frame.
#[derive(Debug, Clone, Copy)]
pub struct ExecReportView<'a> {
    frame: FrameCursor<'a>,
}

impl<'a> ExecReportView<'a> {
    /// Wrap a frame confirmed to be an `ExecutionReport`.
    #[must_use]
    pub fn new(frame: FrameCursor<'a>) -> Self {
        Self { frame }
    }

    /// `ExecType(150)` byte.
    #[must_use]
    pub fn exec_type(&self) -> Option<u8> {
        self.frame.get(150).and_then(|v| v.first().copied())
    }

    /// `OrdStatus(39)` byte.
    #[must_use]
    pub fn ord_status(&self) -> Option<u8> {
        self.frame.get(39).and_then(|v| v.first().copied())
    }

    /// `LastPx(31)` parsed.
    #[must_use]
    pub fn last_px(&self) -> Option<f64> {
        self.frame.get(31).and_then(crate::dialect_fx::parse_float)
    }

    /// The round-trip-exact fill premium ([`TAG_LAST_PX_EXACT`]), if the maker stamped
    /// it (full f64 precision, for last-bit reconciliation to the engine/golden price).
    #[must_use]
    pub fn last_px_exact(&self) -> Option<f64> {
        self.frame
            .get(TAG_LAST_PX_EXACT)
            .and_then(crate::dialect_fx::parse_float)
    }

    /// Echoed `ClOrdID(11)`.
    #[must_use]
    pub fn cl_ord_id(&self) -> Option<&'a [u8]> {
        self.frame.get(11)
    }
}

/// Build a `QuoteCancel(Z)` cancelling all quotes under a `QuoteID(117)`.
#[must_use]
pub fn build_quote_cancel(hdr: &Header<'_>, quote_id: &[u8], enc: &mut FrameEncoder) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::QuoteCancel, enc);
    enc.push(117, quote_id);
    enc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr() -> Header<'static> {
        Header {
            sender: b"CELNET",
            target: b"CPARTY",
            seq_num: 7,
            sending_time: b"20260530-12:00:00.000",
        }
    }

    #[test]
    fn quote_roundtrip() {
        let mut enc = FrameEncoder::new();
        let p = QuoteParams {
            quote_req_id: b"REQ1",
            quote_id: b"Q-42",
            symbol: b"EURUSD",
            bid_px: 0.0123_4567,
            offer_px: 0.0124_5678,
            size: 1_000_000.0,
            valid_until: b"20260530-12:00:05.000",
        };
        let raw = build_quote(&hdr(), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).unwrap();
        let v = QuoteView::new(frame);
        assert_eq!(v.quote_id(), Some(&b"Q-42"[..]));
        assert!((v.bid().unwrap() - 0.0123_4567).abs() < 1e-9);
        assert!((v.offer().unwrap() - 0.0124_5678).abs() < 1e-9);
    }

    #[test]
    fn decimal_encoding_is_exact_and_deterministic() {
        let mut enc = FrameEncoder::new();
        let p = QuoteParams {
            quote_req_id: b"R",
            quote_id: b"Q",
            symbol: b"EURUSD",
            bid_px: 1.0950,
            offer_px: 1.0951,
            size: 1.0,
            valid_until: b"20260530-12:00:05.000",
        };
        let raw = build_quote(&hdr(), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(frame.get(132), Some(&b"1.09500000"[..]));
        assert_eq!(frame.get(133), Some(&b"1.09510000"[..]));
    }

    #[test]
    fn exec_report_roundtrip() {
        let mut enc = FrameEncoder::new();
        let p = ExecReportParams {
            order_id: b"O-1",
            exec_id: b"E-1",
            cl_ord_id: b"C-1",
            exec_type: EXEC_FILLED,
            ord_status: EXEC_FILLED,
            symbol: b"EURUSD",
            side: b'1',
            last_qty: 1_000_000.0,
            last_px: 0.0123_0000,
            multileg_type: None,
            text: None,
        };
        let raw = build_execution_report(&hdr(), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).unwrap();
        let v = ExecReportView::new(frame);
        assert_eq!(v.exec_type(), Some(EXEC_FILLED));
        assert!((v.last_px().unwrap() - 0.0123).abs() < 1e-9);
        assert_eq!(v.cl_ord_id(), Some(&b"C-1"[..]));
    }
}
