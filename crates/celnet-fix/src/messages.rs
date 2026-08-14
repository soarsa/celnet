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
// Counterparty (party) identification — the `NoPartyIDs(453)` block
// ---------------------------------------------------------------------------

/// FIX `NoPartyIDs(453)` — the repeating-group count for the party block.
pub const TAG_NO_PARTY_IDS: u32 = 453;
/// FIX `PartyID(448)` — the identifier of the party named by a `NoPartyIDs(453)`
/// entry. The venue reads it as the DISPLAY counterparty on whose behalf an RFQ was
/// entered (see [`push_originating_party`]), falling back to the session
/// `TargetCompID` when absent.
pub const TAG_PARTY_ID: u32 = 448;
/// FIX `PartyIDSource(447)` — how to interpret the [`TAG_PARTY_ID`] value.
pub const TAG_PARTY_ID_SOURCE: u32 = 447;
/// FIX `PartyRole(452)` — the role the named party plays in the message.
pub const TAG_PARTY_ROLE: u32 = 452;
/// `PartyIDSource(447)='D'` — a proprietary / custom party-code source (a free-form
/// counterparty label rather than a registered LEI/BIC).
pub const PARTY_ID_SOURCE_PROPRIETARY: &[u8] = b"D";
/// `PartyRole(452)=3` — ClientID: the client on whose behalf the RFQ/order is entered.
pub const PARTY_ROLE_CLIENT_ID: i64 = 3;

/// Emit a minimal single-entry `NoPartyIDs(453)` party block naming the counterparty on
/// whose behalf an RFQ is entered — `453=1`, `PartyID(448)=<id>`,
/// `PartyIDSource(447)='D'`, `PartyRole(452)=3` (ClientID).
///
/// A **no-op** when `party_id` is `None` or empty, so a builder that passes `None` emits
/// byte-identical output to one with no party block at all (backward-compatible: every
/// existing caller is unchanged). The acceptor reads `PartyID(448)` as the display
/// counterparty for the blotter, falling back to the authenticated session
/// `TargetCompID` when the block is absent — so the real gateway path (which names no
/// party) still shows `counterparty == CompID`, while a SIM can vary the label per RFQ
/// independent of the single transport CompID.
pub fn push_originating_party(enc: &mut FrameEncoder, party_id: Option<&[u8]>) {
    let Some(id) = party_id.filter(|id| !id.is_empty()) else {
        return;
    };
    enc.push_int(TAG_NO_PARTY_IDS, 1);
    enc.push(TAG_PARTY_ID, id);
    enc.push(TAG_PARTY_ID_SOURCE, PARTY_ID_SOURCE_PROPRIETARY);
    enc.push_int(TAG_PARTY_ROLE, PARTY_ROLE_CLIENT_ID);
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

/// Build a `NewOrderSingle(D)` lifting a streamed market-data top-of-book by `Symbol(55)`
/// — no `QuoteID(117)`, since a `MarketDataSnapshotFullRefresh(W)` carries none. The venue
/// resolves the current liftable token for the symbol (see the market-data lift path). The
/// `Price(44)` is the top-of-book level being hit (Bid on a sell / Offer on a buy).
#[derive(Debug, Clone, Copy)]
pub struct MarketOrderParams<'a> {
    /// `ClOrdID(11)`.
    pub cl_ord_id: &'a [u8],
    /// `Symbol(55)` being lifted.
    pub symbol: &'a [u8],
    /// The streamed `QuoteID(117)` the `35=W` advertised for this top-of-book — the primary
    /// lift handle. Empty ⇒ omitted (a pure by-`Symbol(55)` lift, still accepted by the venue).
    pub quote_id: &'a [u8],
    /// `SecurityType(167)` echoing the streamed instrument's product arm (e.g. `BOND`). Empty
    /// ⇒ omitted.
    pub security_type: &'a [u8],
    /// `Side(54)` byte.
    pub side: u8,
    /// `OrderQty(38)`.
    pub qty: f64,
    /// `Price(44)` — the streamed top-of-book level being hit. Omitted from the
    /// frame for a pure market order ([`ord_type::MARKET`]), which carries no price.
    pub price: f64,
    /// `OrdType(40)` — [`ord_type::PREVIOUSLY_QUOTED`] for a lift of a streamed
    /// level (the price the taker was shown is the price it is entitled to),
    /// [`ord_type::LIMIT`] for a priced order, [`ord_type::MARKET`] for an unpriced
    /// one.
    pub ord_type: u8,
    /// `TimeInForce(59)` — see [`time_in_force`]. `None` omits the tag entirely,
    /// leaving the venue's own default in force, which is what a lift of a streamed
    /// level has always meant.
    pub tif: Option<u8>,
    /// `TransactTime(60)` bytes.
    pub transact_time: &'a [u8],
}

/// Build a `NewOrderSingle(D)` frame from [`MarketOrderParams`] (a market-data lift by
/// symbol — carries `Price(44)`, omits `QuoteID(117)`).
#[must_use]
pub fn build_new_order_by_symbol(
    hdr: &Header<'_>,
    p: &MarketOrderParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::NewOrderSingle, enc);
    enc.push(11, p.cl_ord_id);
    // The streamed QuoteID handle (the primary lift key); omitted for a pure by-symbol lift.
    if !p.quote_id.is_empty() {
        enc.push(117, p.quote_id);
    }
    enc.push(55, p.symbol);
    // Echo the streamed instrument's SecurityType so the lift is a complete FI order.
    if !p.security_type.is_empty() {
        enc.push(167, p.security_type);
    }
    enc.push(54, &[p.side]);
    push_decimal(enc, 38, p.qty);
    // A market order carries no price by definition; every other type does.
    if p.ord_type != ord_type::MARKET {
        push_decimal(enc, 44, p.price);
    }
    enc.push(40, &[p.ord_type]);
    if let Some(tif) = p.tif {
        enc.push(59, &[tif]);
    }
    enc.push(60, p.transact_time);
    enc.finish()
}

/// A borrowed view over a `NewOrderSingle(D)` frame — the maker side of the lift.
///
/// The venue must be able to tell a MALFORMED order from a well-formed one it
/// declines, so every accessor is fallible and nothing is defaulted: an absent or
/// unparseable field reads as `None` and the caller decides what that means, rather
/// than silently becoming a market order, a buy, or a zero quantity.
#[derive(Debug, Clone, Copy)]
pub struct NewOrderView<'a> {
    frame: FrameCursor<'a>,
}

impl<'a> NewOrderView<'a> {
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

    /// `Symbol(55)`.
    #[must_use]
    pub fn symbol(&self) -> Option<&'a [u8]> {
        self.frame.get(55)
    }

    /// `QuoteID(117)`, when the taker lifted a specific streamed handle.
    #[must_use]
    pub fn quote_id(&self) -> Option<&'a [u8]> {
        self.frame.get(117)
    }

    /// `Side(54)` byte.
    #[must_use]
    pub fn side(&self) -> Option<u8> {
        self.frame.get(54).and_then(|v| v.first().copied())
    }

    /// `OrderQty(38)` parsed.
    #[must_use]
    pub fn order_qty(&self) -> Option<f64> {
        self.frame.get(38).and_then(crate::dialect_fx::parse_float)
    }

    /// `Price(44)` parsed (absent on a market order).
    #[must_use]
    pub fn price(&self) -> Option<f64> {
        self.frame.get(44).and_then(crate::dialect_fx::parse_float)
    }

    /// `OrdType(40)` byte.
    #[must_use]
    pub fn ord_type(&self) -> Option<u8> {
        self.frame.get(40).and_then(|v| v.first().copied())
    }

    /// `TimeInForce(59)` byte — `None` when the taker did not state one.
    #[must_use]
    pub fn time_in_force(&self) -> Option<u8> {
        self.frame.get(59).and_then(|v| v.first().copied())
    }
}

/// `ExecType(150)` / `OrdStatus(39)`: filled.
pub const EXEC_FILLED: u8 = b'F';
/// `ExecType(150)` / `OrdStatus(39)`: rejected.
pub const EXEC_REJECTED: u8 = b'8';
/// `OrdStatus(39)`: partially filled — some of the order traded and the rest is no
/// longer working (an IOC's cancelled remainder). Paired with an
/// [`EXEC_FILLED`] `ExecType(150)`, which reports the trade that just happened,
/// while `OrdStatus` reports where the ORDER now stands.
pub const ORD_STATUS_PARTIALLY_FILLED: u8 = b'1';
/// `OrdStatus(39)`: cancelled. The order is done and nothing traded — the terminal
/// state of an IOC that found no eligible liquidity, as distinct from a rejection
/// (which means the venue would not accept the order at all).
pub const ORD_STATUS_CANCELED: u8 = b'4';
/// `ExecType(150)`: cancelled.
pub const EXEC_CANCELED: u8 = b'4';

/// `TimeInForce(59)` — the standard encoding. Declared here so a venue names the
/// value it is honouring or declining instead of comparing raw bytes at the call
/// site. The venue-side semantics live with the matching core that implements them.
pub mod time_in_force {
    /// `0` — Day.
    pub const DAY: u8 = b'0';
    /// `1` — Good Till Cancel.
    pub const GOOD_TILL_CANCEL: u8 = b'1';
    /// `2` — At the Opening.
    pub const AT_THE_OPENING: u8 = b'2';
    /// `3` — Immediate Or Cancel.
    pub const IMMEDIATE_OR_CANCEL: u8 = b'3';
    /// `4` — Fill Or Kill.
    pub const FILL_OR_KILL: u8 = b'4';
    /// `5` — Good Till Crossing.
    pub const GOOD_TILL_CROSSING: u8 = b'5';
    /// `6` — Good Till Date.
    pub const GOOD_TILL_DATE: u8 = b'6';
    /// `7` — At the Close.
    pub const AT_THE_CLOSE: u8 = b'7';
}

/// `OrdType(40)` — the standard encoding for the order types this estate trades.
pub mod ord_type {
    /// `1` — Market.
    pub const MARKET: u8 = b'1';
    /// `2` — Limit.
    pub const LIMIT: u8 = b'2';
    /// `D` — Previously Quoted (a lift of a streamed level).
    pub const PREVIOUSLY_QUOTED: u8 = b'D';
}

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

    /// `LastQty(32)` parsed — the quantity that actually traded on this report.
    ///
    /// Distinct from the order's `OrderQty(38)`: a partially-filled report carries the
    /// traded part here and reports the rest via `OrdStatus(39)` + `Text(58)`. A taker
    /// that read only `OrdStatus` could not tell a 10% fill from a 90% one.
    #[must_use]
    pub fn last_qty(&self) -> Option<f64> {
        self.frame.get(32).and_then(crate::dialect_fx::parse_float)
    }

    /// `OrderID(37)` — the venue's own identity for the order.
    #[must_use]
    pub fn order_id(&self) -> Option<&'a [u8]> {
        self.frame.get(37)
    }

    /// `Text(58)` — the venue's reason. Absent on a clean complete fill, which needs
    /// no explanation; present (leading with a machine-readable code) on every
    /// rejection, cancel and partial.
    #[must_use]
    pub fn text(&self) -> Option<&'a [u8]> {
        self.frame.get(58)
    }

    /// Echoed `ClOrdID(11)`.
    #[must_use]
    pub fn cl_ord_id(&self) -> Option<&'a [u8]> {
        self.frame.get(11)
    }
}

// ---------------------------------------------------------------------------
// Market-data streaming (35=V MarketDataRequest / 35=W MarketDataSnapshotFullRefresh)
//
// The fixed-income **streaming** venue speaks Market Data, not Quote: a taker
// SUBSCRIBES to instruments with a `MarketDataRequest(V)` and the venue pushes a
// `MarketDataSnapshotFullRefresh(W)` per subscribed instrument on each stream tick,
// carrying a top-of-book Bid/Offer under the `NoMDEntries(268)` group. A lift is a
// plain `NewOrderSingle(D)` naming the `Symbol(55)` (no `QuoteID` — the taker hits the
// last snapshot it saw), answered by the same `ExecutionReport(8)` the RFQ lift uses.
// ---------------------------------------------------------------------------

/// `MDReqID(262)` — the taker-minted market-data request correlation id, echoed on
/// every `MarketDataSnapshotFullRefresh(W)` the subscription produces.
pub const TAG_MD_REQ_ID: u32 = 262;
/// `SubscriptionRequestType(263)` — subscribe (snapshot+updates) / unsubscribe intent.
pub const TAG_SUBSCRIPTION_REQUEST_TYPE: u32 = 263;
/// `MarketDepth(264)` — book depth requested (`1` = top-of-book).
pub const TAG_MARKET_DEPTH: u32 = 264;
/// `NoMDEntryTypes(267)` — the count of requested entry types (Bid/Offer).
pub const TAG_NO_MD_ENTRY_TYPES: u32 = 267;
/// `MDEntryType(269)` — the entry side: [`MD_ENTRY_BID`] / [`MD_ENTRY_OFFER`].
pub const TAG_MD_ENTRY_TYPE: u32 = 269;
/// `NoMDEntries(268)` — the count of entries carried in a snapshot.
pub const TAG_NO_MD_ENTRIES: u32 = 268;
/// `MDEntryPx(270)` — the price of a market-data entry.
pub const TAG_MD_ENTRY_PX: u32 = 270;
/// `MDEntrySize(271)` — the size of a market-data entry.
pub const TAG_MD_ENTRY_SIZE: u32 = 271;
/// `NoRelatedSym(146)` — the count of instruments in a `MarketDataRequest`.
pub const TAG_NO_RELATED_SYM: u32 = 146;

/// `MDEntryType(269)` value: Bid.
pub const MD_ENTRY_BID: u8 = b'0';
/// `MDEntryType(269)` value: Offer.
pub const MD_ENTRY_OFFER: u8 = b'1';

/// `SubscriptionRequestType(263)`: snapshot only (a one-shot).
pub const SUBSCRIPTION_SNAPSHOT: &[u8] = b"0";
/// `SubscriptionRequestType(263)`: snapshot + updates (an ESP subscribe).
pub const SUBSCRIPTION_SNAPSHOT_UPDATES: &[u8] = b"1";
/// `SubscriptionRequestType(263)`: disable a previous snapshot (an ESP unsubscribe).
pub const SUBSCRIPTION_DISABLE: &[u8] = b"2";
/// `MarketDepth(264)`: top-of-book (the venue streams a single best bid + best offer).
pub const MARKET_DEPTH_TOP_OF_BOOK: i64 = 1;

/// Open a `MarketDataRequest(V)` body: after the caller has written the header, push the
/// request envelope — `MDReqID(262)`, `SubscriptionRequestType(263)`, `MarketDepth(264)`,
/// the two-sided `NoMDEntryTypes(267)` group (Bid + Offer), and `NoRelatedSym(146)=1`.
/// The caller then pushes the single instrument block (the dialect's `Symbol(55)` + terms)
/// and calls [`FrameEncoder::finish`]. Kept asset-agnostic here (the instrument block is a
/// dialect concern), so any product family can ride the one market-data envelope.
pub fn push_md_request_envelope(
    enc: &mut FrameEncoder,
    md_req_id: &[u8],
    subscription: &[u8],
    depth: i64,
) {
    enc.push(TAG_MD_REQ_ID, md_req_id);
    enc.push(TAG_SUBSCRIPTION_REQUEST_TYPE, subscription);
    enc.push_int(TAG_MARKET_DEPTH, depth);
    // Request BOTH sides of the book (Bid then Offer).
    enc.push_int(TAG_NO_MD_ENTRY_TYPES, 2);
    enc.push(TAG_MD_ENTRY_TYPE, &[MD_ENTRY_BID]);
    enc.push(TAG_MD_ENTRY_TYPE, &[MD_ENTRY_OFFER]);
    // A single instrument per request (NoRelatedSym=1); the instrument block follows.
    enc.push_int(TAG_NO_RELATED_SYM, 1);
}

/// A borrowed view over an inbound `MarketDataRequest(V)` frame.
#[derive(Debug, Clone, Copy)]
pub struct MarketDataRequestView<'a> {
    frame: FrameCursor<'a>,
}

impl<'a> MarketDataRequestView<'a> {
    /// Wrap a frame confirmed to be a `MarketDataRequest`.
    #[must_use]
    pub fn new(frame: FrameCursor<'a>) -> Self {
        Self { frame }
    }

    /// `MDReqID(262)`.
    #[must_use]
    pub fn md_req_id(&self) -> Option<&'a [u8]> {
        self.frame.get(TAG_MD_REQ_ID)
    }

    /// `SubscriptionRequestType(263)` byte.
    #[must_use]
    pub fn subscription_type(&self) -> Option<u8> {
        self.frame
            .get(TAG_SUBSCRIPTION_REQUEST_TYPE)
            .and_then(|v| v.first().copied())
    }

    /// `Symbol(55)` (the single subscribed instrument).
    #[must_use]
    pub fn symbol(&self) -> Option<&'a [u8]> {
        self.frame.get(55)
    }

    /// The underlying frame, for dialect decoding of the instrument block.
    #[must_use]
    pub fn frame(&self) -> &FrameCursor<'a> {
        &self.frame
    }
}

/// Build a `MarketDataSnapshotFullRefresh(W)`: a top-of-book snapshot for one instrument.
#[derive(Debug, Clone, Copy)]
pub struct MarketDataSnapshotParams<'a> {
    /// Echoed `MDReqID(262)` of the subscription this snapshot serves.
    pub md_req_id: &'a [u8],
    /// `Symbol(55)`.
    pub symbol: &'a [u8],
    /// The liftable `QuoteID(117)` naming this streamed top-of-book — the client-visible
    /// handle for the SAME keyed-MAC two-way token the venue will honour on a lift. A
    /// `NewOrderSingle(D)` echoes it (plus `Side(54)` to pick the leg); the venue also still
    /// accepts a by-`Symbol(55)` lift for compatibility.
    pub quote_id: &'a [u8],
    /// Best bid (`MDEntryType=0`, `MDEntryPx(270)`).
    pub bid_px: f64,
    /// Best offer (`MDEntryType=1`, `MDEntryPx(270)`).
    pub offer_px: f64,
    /// Size on both entries (`MDEntrySize(271)`).
    pub size: f64,
}

/// Build a `MarketDataSnapshotFullRefresh(W)` frame from [`MarketDataSnapshotParams`].
/// Two entries under `NoMDEntries(268)=2`: Bid then Offer, each with px + size.
#[must_use]
pub fn build_market_data_snapshot(
    hdr: &Header<'_>,
    p: &MarketDataSnapshotParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::MarketDataSnapshotFullRefresh, enc);
    enc.push(TAG_MD_REQ_ID, p.md_req_id);
    enc.push(55, p.symbol);
    // The liftable QuoteID naming this top-of-book (a client echoes it on the lift).
    enc.push(117, p.quote_id);
    enc.push_int(TAG_NO_MD_ENTRIES, 2);
    // Bid entry.
    enc.push(TAG_MD_ENTRY_TYPE, &[MD_ENTRY_BID]);
    push_decimal(enc, TAG_MD_ENTRY_PX, p.bid_px);
    push_decimal(enc, TAG_MD_ENTRY_SIZE, p.size);
    // Offer entry.
    enc.push(TAG_MD_ENTRY_TYPE, &[MD_ENTRY_OFFER]);
    push_decimal(enc, TAG_MD_ENTRY_PX, p.offer_px);
    push_decimal(enc, TAG_MD_ENTRY_SIZE, p.size);
    enc.finish()
}

/// A borrowed view over a `MarketDataSnapshotFullRefresh(W)` frame: extracts the echoed
/// `MDReqID(262)`, the `Symbol(55)`, and the top-of-book Bid/Offer + sizes by walking the
/// `NoMDEntries(268)` group in wire order (delimiter `MDEntryType(269)`).
#[derive(Debug, Clone, Copy)]
pub struct MarketDataSnapshotView<'a> {
    frame: FrameCursor<'a>,
}

impl<'a> MarketDataSnapshotView<'a> {
    /// Wrap a frame confirmed to be a `MarketDataSnapshotFullRefresh`.
    #[must_use]
    pub fn new(frame: FrameCursor<'a>) -> Self {
        Self { frame }
    }

    /// `MDReqID(262)`.
    #[must_use]
    pub fn md_req_id(&self) -> Option<&'a [u8]> {
        self.frame.get(TAG_MD_REQ_ID)
    }

    /// `Symbol(55)`.
    #[must_use]
    pub fn symbol(&self) -> Option<&'a [u8]> {
        self.frame.get(55)
    }

    /// The liftable `QuoteID(117)` this snapshot advertised (a client echoes it on the lift).
    #[must_use]
    pub fn quote_id(&self) -> Option<&'a [u8]> {
        self.frame.get(117)
    }

    /// The top-of-book `(bid_px, offer_px, bid_size, offer_size)` extracted from the
    /// `NoMDEntries(268)` group. Each entry starts with `MDEntryType(269)` (`0`=Bid,
    /// `1`=Offer); the following `MDEntryPx(270)` / `MDEntrySize(271)` bind to it. A side
    /// absent from the snapshot returns `None` for its px/size.
    #[must_use]
    pub fn top_of_book(&self) -> MdTopOfBook {
        let mut tob = MdTopOfBook::default();
        let mut current: Option<u8> = None;
        for field in self.frame.fields() {
            match field.tag {
                TAG_MD_ENTRY_TYPE => current = field.value.first().copied(),
                TAG_MD_ENTRY_PX => {
                    let px = crate::dialect_fx::parse_float(field.value);
                    match current {
                        Some(MD_ENTRY_BID) => tob.bid_px = px,
                        Some(MD_ENTRY_OFFER) => tob.offer_px = px,
                        _ => {}
                    }
                }
                TAG_MD_ENTRY_SIZE => {
                    let sz = crate::dialect_fx::parse_float(field.value);
                    match current {
                        Some(MD_ENTRY_BID) => tob.bid_size = sz,
                        Some(MD_ENTRY_OFFER) => tob.offer_size = sz,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        tob
    }
}

/// The top-of-book a [`MarketDataSnapshotView`] extracts: best bid/offer + their sizes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MdTopOfBook {
    /// Best bid price (`MDEntryType=0`).
    pub bid_px: Option<f64>,
    /// Best offer price (`MDEntryType=1`).
    pub offer_px: Option<f64>,
    /// Bid size.
    pub bid_size: Option<f64>,
    /// Offer size.
    pub offer_size: Option<f64>,
}

/// Build a `QuoteRequestReject(AG)` declining an RFQ that could not be priced —
/// the desk declined, the request expired/withdrew, or the gateway could not
/// reach the desk. Carries the echoed `QuoteReqID(131)`, the `Symbol(55)`, a
/// `QuoteRequestRejectReason(658)` code, and a free-text `Text(58)` reason.
#[must_use]
pub fn build_quote_request_reject(
    hdr: &Header<'_>,
    quote_req_id: &[u8],
    symbol: &[u8],
    reason_code: i64,
    text: &[u8],
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::QuoteRequestReject, enc);
    enc.push(131, quote_req_id);
    if !symbol.is_empty() {
        enc.push(55, symbol);
    }
    enc.push_int(658, reason_code);
    if !text.is_empty() {
        enc.push(58, text);
    }
    enc.finish()
}

/// A borrowed view over a `QuoteRequestReject(AG)` frame.
#[derive(Debug, Clone, Copy)]
pub struct QuoteRequestRejectView<'a> {
    frame: FrameCursor<'a>,
}

impl<'a> QuoteRequestRejectView<'a> {
    /// Wrap a frame confirmed to be a `QuoteRequestReject`.
    #[must_use]
    pub fn new(frame: FrameCursor<'a>) -> Self {
        Self { frame }
    }

    /// Echoed `QuoteReqID(131)`.
    #[must_use]
    pub fn quote_req_id(&self) -> Option<&'a [u8]> {
        self.frame.get(131)
    }

    /// `QuoteRequestRejectReason(658)` as bytes.
    #[must_use]
    pub fn reject_reason(&self) -> Option<&'a [u8]> {
        self.frame.get(658)
    }

    /// `Text(58)` free-text reason.
    #[must_use]
    pub fn text(&self) -> Option<&'a [u8]> {
        self.frame.get(58)
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
    fn market_data_snapshot_roundtrips_and_validates() {
        let mut enc = FrameEncoder::new();
        let p = MarketDataSnapshotParams {
            md_req_id: b"MDR-1",
            symbol: b"US-912828-5Y",
            quote_id: b"Q-42",
            bid_px: 99.4567_8901,
            offer_px: 99.6543_2109,
            size: 5_000_000.0,
        };
        let raw = build_market_data_snapshot(&hdr(), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).expect("W frame parses (BodyLength + CheckSum ok)");
        // The dictionary accepts it (35=W required tags present, MD entry types valid).
        assert_eq!(
            crate::dictionary::validate(&frame),
            Ok(MsgType::MarketDataSnapshotFullRefresh)
        );
        let v = MarketDataSnapshotView::new(frame);
        assert_eq!(v.md_req_id(), Some(&b"MDR-1"[..]));
        assert_eq!(v.symbol(), Some(&b"US-912828-5Y"[..]));
        assert_eq!(v.quote_id(), Some(&b"Q-42"[..]));
        let tob = v.top_of_book();
        assert!((tob.bid_px.unwrap() - 99.4567_8901).abs() < 1e-6);
        assert!((tob.offer_px.unwrap() - 99.6543_2109).abs() < 1e-6);
        assert!((tob.bid_size.unwrap() - 5_000_000.0).abs() < 1e-3);
        assert!((tob.offer_size.unwrap() - 5_000_000.0).abs() < 1e-3);
    }

    #[test]
    fn market_data_request_envelope_roundtrips_and_validates() {
        // Build a MarketDataRequest(V) envelope + a single (bond-style) instrument block.
        let mut enc = FrameEncoder::new();
        enc.clear();
        hdr().encode(MsgType::MarketDataRequest, &mut enc);
        push_md_request_envelope(
            &mut enc,
            b"MDR-2",
            SUBSCRIPTION_SNAPSHOT_UPDATES,
            MARKET_DEPTH_TOP_OF_BOOK,
        );
        enc.push(55, b"US-912828-5Y");
        enc.push(167, b"BOND");
        let raw = enc.finish();
        let frame = FrameCursor::parse(&raw).expect("V frame parses");
        assert_eq!(
            crate::dictionary::validate(&frame),
            Ok(MsgType::MarketDataRequest)
        );
        let v = MarketDataRequestView::new(frame);
        assert_eq!(v.md_req_id(), Some(&b"MDR-2"[..]));
        assert_eq!(v.subscription_type(), Some(b'1')); // subscribe (snapshot+updates)
        assert_eq!(v.symbol(), Some(&b"US-912828-5Y"[..]));
    }

    #[test]
    fn market_order_by_symbol_omits_quote_id() {
        let mut enc = FrameEncoder::new();
        let p = MarketOrderParams {
            cl_ord_id: b"C-1",
            symbol: b"US-912828-5Y",
            quote_id: b"",
            security_type: b"",
            side: crate::dialect_fx::SIDE_BUY,
            qty: 1_000_000.0,
            price: 99.65,
            // Preserved verbatim: a lift of a streamed level IS a
            // previously-quoted order, and it stated no TimeInForce(59)
            // before this field existed. Byte-identical frame.
            ord_type: ord_type::PREVIOUSLY_QUOTED,
            tif: None,
            transact_time: b"20260530-12:00:01.000",
        };
        let raw = build_new_order_by_symbol(&hdr(), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).expect("D frame parses");
        assert_eq!(
            crate::dictionary::validate(&frame),
            Ok(MsgType::NewOrderSingle)
        );
        // No QuoteID(117) on a bare by-symbol lift — the venue resolves by Symbol.
        assert_eq!(frame.get(117), None);
        assert_eq!(frame.get(167), None);
        let v = NewOrderSingleView::new(frame);
        assert_eq!(v.quote_id(), None);
        assert_eq!(v.side(), Some(crate::dialect_fx::SIDE_BUY));
        assert!(
            (frame
                .get(44)
                .and_then(crate::dialect_fx::parse_float)
                .unwrap()
                - 99.65)
                .abs()
                < 1e-6
        );
    }

    /// An ESP lift echoes the streamed `QuoteID(117)`, the `Symbol(55)`, and the
    /// `SecurityType(167)` — a complete, venue-resolvable fixed-income order.
    #[test]
    fn market_order_by_symbol_carries_quote_id_and_security_type() {
        let mut enc = FrameEncoder::new();
        let p = MarketOrderParams {
            cl_ord_id: b"C-2",
            symbol: b"912828XY7",
            quote_id: b"7734901234",
            security_type: crate::dialect_rates::SEC_TYPE_BOND,
            side: crate::dialect_fx::SIDE_SELL,
            qty: 5_000_000.0,
            price: 99.40,
            // Preserved verbatim: a lift of a streamed level IS a
            // previously-quoted order, and it stated no TimeInForce(59)
            // before this field existed. Byte-identical frame.
            ord_type: ord_type::PREVIOUSLY_QUOTED,
            tif: None,
            transact_time: b"20260530-12:00:01.000",
        };
        let raw = build_new_order_by_symbol(&hdr(), &p, &mut enc);
        let frame = FrameCursor::parse(&raw).expect("D frame parses");
        assert_eq!(
            crate::dictionary::validate(&frame),
            Ok(MsgType::NewOrderSingle)
        );
        assert_eq!(frame.get(117), Some(&b"7734901234"[..]));
        assert_eq!(frame.get(55), Some(&b"912828XY7"[..]));
        assert_eq!(frame.get(167), Some(crate::dialect_rates::SEC_TYPE_BOND));
        let v = NewOrderSingleView::new(frame);
        assert_eq!(v.quote_id(), Some(&b"7734901234"[..]));
        assert_eq!(v.side(), Some(crate::dialect_fx::SIDE_SELL));
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
