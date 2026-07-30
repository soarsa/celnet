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

use celnet_proto::{AccrualBasis, BondInstrument, BrokenDate, PaymentFrequency, Side};

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
    /// `CouponRate(223)` was missing, unparseable, negative, or not finite.
    BadCoupon,
    /// `[TAG_COUPON_FREQUENCY]` was missing or not `1` / `2` / `4` coupons per year.
    BadCouponFrequency,
    /// `[TAG_DAY_COUNT]` was missing or not a recognised day-count mnemonic.
    BadDayCount,
    /// `MaturityDate(541)` was missing or not a valid `YYYYMMDD` civil date.
    BadMaturity,
    /// `[TAG_REDEMPTION]` was present but not a finite, strictly-positive face value.
    BadRedemption,
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
    build_rates_quote_request_with_party(hdr, p, None, enc)
}

/// Build an OIS `QuoteRequest(R)` frame as [`build_rates_quote_request`], additionally
/// naming the counterparty on whose behalf the RFQ is entered in a `NoPartyIDs(453)`
/// party block (`party_id` ⇒ `PartyID(448)`; see
/// [`crate::messages::push_originating_party`]). Passing `party_id = None` emits a
/// byte-identical frame to [`build_rates_quote_request`] — the plain builder simply
/// delegates here with `None`.
///
/// A managed SIM varies `party_id` per RFQ so the desk blotter shows realistic, varied
/// counterparty names over one FIX session (one `SenderCompID`) and counterparty-keyed
/// risk-routing rules become testable; the real gateway leaves it `None`, so the venue
/// falls back to the authenticated `TargetCompID`.
#[must_use]
pub fn build_rates_quote_request_with_party(
    hdr: &Header<'_>,
    p: &RatesQuoteRequestParams<'_>,
    party_id: Option<&[u8]>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::QuoteRequest, enc);
    enc.push(131, p.quote_req_id);
    enc.push(55, p.symbol);
    enc.push(460, b"5"); // Product = RATE
    enc.push(167, SEC_TYPE_OIS);
    if is_integer_valued(p.notional) {
        enc.push_int(38, p.notional as i64);
    } else {
        enc.push(38, format!("{}", p.notional).as_bytes());
    }
    enc.push_int(TAG_TENOR_YEARS, i64::from(p.tenor_years));
    enc.push(263, p.subscription.to_fix());
    if let Some(side) = p.side.to_fix() {
        enc.push(54, &[side]);
    }
    crate::messages::push_originating_party(enc, party_id);
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

/// Whether a value is a whole number small enough to round-trip through `i64`
/// without losing its fractional zero (so it can be emitted with `push_int`).
/// Shared by the notional (`OrderQty(38)`) and the bond redemption ([`TAG_REDEMPTION`]).
fn is_integer_valued(value: f64) -> bool {
    value.fract() == 0.0 && value.abs() < 9.007_199_254_740_992e15
}

// ===========================================================================
// Cash-bond arm
//
// A fixed-coupon cash bond is the fixed-income analogue of the OIS RFQ above:
// `SecurityType(167)=BOND` selects the bond arm, and the coupon economics ride on
// standard FIX fields (`CouponRate(223)`, `MaturityDate(541)`) plus a small set of
// private dialect tags for the fields FIX 4.4 has no core tag for (coupon frequency,
// day-count, par redemption). The decode produces the LANDED [`BondInstrument`] wire
// shape verbatim — the same numeric message the server prices through
// `price_rates`(Bond) / `celnet_bond::price_from_curve` — so a bond RFQ flows through
// the identical quote / keyed-MAC token / lift machinery as an OIS or FX line, with no
// bond-specific quote/order path. The dialect stays pricing-free: it maps bytes ⇄ the
// wire instrument only; the edge supplies the discount curve and prices.
// ===========================================================================

/// FIX `SecurityType(167)` for a fixed-coupon cash bond — the fixed-income analogue of
/// [`SEC_TYPE_OIS`]. A purpose-named, vendor-neutral dialect selector (`CLAUDE.md`
/// rule 8): FIX tolerates a private `SecurityType` value exactly as it tolerates the
/// private [`TAG_TENOR_YEARS`], and this is the canonical home of the bond selector the
/// server's FIX edge content-detects.
pub const SEC_TYPE_BOND: &[u8] = b"BOND";

/// FIX `Product(460)` value the bond builder stamps (`6` = GOVERNMENT). Decorative: the
/// decoder never reads `Product(460)` — `SecurityType(167)` selects the arm — exactly as
/// the OIS builder stamps `Product=5` (RATE) for completeness only.
pub const PRODUCT_BOND: &[u8] = b"6";

/// The dialect tag carrying the bond coupon frequency as **coupons per year**
/// (`1` / `2` / `4` ⇒ annual / semi-annual / quarterly). FIX 4.4 has no core
/// coupon-frequency tag; this is a private, user-defined dialect field (rule 8) — the
/// FI analogue of [`TAG_TENOR_YEARS`]. Maps 1:1 onto the priced [`PaymentFrequency`].
pub const TAG_COUPON_FREQUENCY: u32 = 7110;

/// The dialect tag carrying the bond accrual day-count as a mnemonic token
/// ([`DAY_COUNT_ACT_360`] / [`DAY_COUNT_ACT_365F`] / [`DAY_COUNT_30_360`]). FIX 4.4 has
/// no core day-count tag; a private dialect field. Maps 1:1 onto the priced
/// [`AccrualBasis`].
pub const TAG_DAY_COUNT: u32 = 7111;

/// The dialect tag carrying the bond par redemption / face value. Absent ⇒
/// [`DEFAULT_REDEMPTION`] (par is the near-universal convention). FIX 4.4 has no core
/// redemption tag.
pub const TAG_REDEMPTION: u32 = 7112;

/// Day-count mnemonic ([`TAG_DAY_COUNT`]): Actual/360.
pub const DAY_COUNT_ACT_360: &[u8] = b"ACT360";
/// Day-count mnemonic ([`TAG_DAY_COUNT`]): Actual/365 Fixed.
pub const DAY_COUNT_ACT_365F: &[u8] = b"ACT365F";
/// Day-count mnemonic ([`TAG_DAY_COUNT`]): 30/360 Bond Basis (the standard USD fixed
/// coupon-bond basis).
pub const DAY_COUNT_30_360: &[u8] = b"30360";

/// The par redemption a bond RFQ assumes when [`TAG_REDEMPTION`] is absent (100 = par).
pub const DEFAULT_REDEMPTION: f64 = 100.0;

/// Parameters for a cash-bond `QuoteRequest(R)` — the symmetric encode side of
/// [`decode_bond_rfq`]. An initiator (price-taker / test client) fills these and the
/// builder lays them onto the wire flat, exactly as the acceptor reads them.
#[derive(Debug, Clone)]
pub struct BondQuoteRequestParams<'a> {
    /// `QuoteReqID(131)` — the client-minted RFQ correlation id.
    pub quote_req_id: &'a [u8],
    /// `Symbol(55)` — the bond symbol, e.g. `b"US-TREASURY-5Y"`.
    pub symbol: &'a [u8],
    /// `CouponRate(223)` — the annual coupon as a **decimal** (`0.05` = 5%; `0` for a
    /// zero-coupon bond). This dialect uses the decimal convention (matching the priced
    /// [`BondInstrument::coupon_rate`]), never the percent convention some venues put on
    /// tag 223.
    pub coupon_rate: f64,
    /// The coupon payment frequency ([`TAG_COUPON_FREQUENCY`]).
    pub coupon_frequency: PaymentFrequency,
    /// The accrual day-count basis ([`TAG_DAY_COUNT`]).
    pub day_count: AccrualBasis,
    /// `MaturityDate(541)` — the final-redemption date.
    pub maturity: BrokenDate,
    /// The par redemption / face value ([`TAG_REDEMPTION`]; must be `> 0`).
    pub redemption: f64,
    /// `OrderQty(38)` — the notional (must be `> 0`).
    pub notional: f64,
    /// The directional intent: `Side(54)=1` long (buy), `2` short (sell), absent ⇒ a
    /// two-way (bid/offer) request.
    pub side: Side,
    /// One-shot RFQ vs RFS subscribe / unsubscribe (`SubscriptionRequestType(263)`).
    pub subscription: SubscriptionRequest,
}

/// Build a flat cash-bond `QuoteRequest(R)` frame from [`BondQuoteRequestParams`].
///
/// The layout mirrors what [`decode_bond_rfq`] reads: `QuoteReqID(131)`, `Symbol(55)`,
/// `Product(460)=6`, `SecurityType(167)=BOND`, `CouponRate(223)`, `OrderQty(38)`,
/// `MaturityDate(541)`, the dialect's [`TAG_COUPON_FREQUENCY`] / [`TAG_DAY_COUNT`] /
/// [`TAG_REDEMPTION`], `SubscriptionRequestType(263)`, and `Side(54)` when directional.
/// A whole-number notional / redemption is emitted as a FIX int; a fractional one is
/// re-emitted as a precise decimal so the exact value survives the wire.
#[must_use]
pub fn build_bond_quote_request(
    hdr: &Header<'_>,
    p: &BondQuoteRequestParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::QuoteRequest, enc);
    enc.push(131, p.quote_req_id);
    enc.push(55, p.symbol);
    enc.push(460, PRODUCT_BOND);
    enc.push(167, SEC_TYPE_BOND);
    enc.push(223, format!("{}", p.coupon_rate).as_bytes());
    if is_integer_valued(p.notional) {
        enc.push_int(38, p.notional as i64);
    } else {
        enc.push(38, format!("{}", p.notional).as_bytes());
    }
    enc.push(541, fmt_fix_date(&p.maturity).as_bytes());
    enc.push_int(TAG_COUPON_FREQUENCY, periods_per_year(p.coupon_frequency));
    enc.push(TAG_DAY_COUNT, day_count_token(p.day_count));
    if is_integer_valued(p.redemption) {
        enc.push_int(TAG_REDEMPTION, p.redemption as i64);
    } else {
        enc.push(TAG_REDEMPTION, format!("{}", p.redemption).as_bytes());
    }
    enc.push(263, p.subscription.to_fix());
    if let Some(side) = side_to_fix_byte(p.side) {
        enc.push(54, &[side]);
    }
    enc.finish()
}

/// A single cash-bond RFQ descriptor decoded from the FIX instrument block: the RFQ
/// envelope (correlation id, symbol, notional, subscription) plus the LANDED
/// [`BondInstrument`] wire shape the server prices verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct BondRfq {
    /// `QuoteReqID(131)` — the client RFQ correlation id.
    pub quote_req_id: Vec<u8>,
    /// `Symbol(55)` — the bond symbol.
    pub symbol: Vec<u8>,
    /// The notional (`OrderQty(38)`). Carried on the envelope, not the instrument — a
    /// [`BondInstrument`] is priced per unit face and scaled by notional outside.
    pub notional: f64,
    /// One-shot RFQ vs RFS subscribe / unsubscribe.
    pub subscription: SubscriptionRequest,
    /// The decoded bond, ready to price through `price_rates`(Bond) — coupon, frequency,
    /// day-count, maturity, redemption, and `side` (absent `Side(54)` ⇒ `SIDE_TWO_WAY`).
    pub instrument: BondInstrument,
}

/// Decode + validate an inbound cash-bond `QuoteRequest(R)` into a [`BondRfq`] carrying
/// the LANDED [`BondInstrument`] wire shape.
///
/// # Errors
///
/// Returns a [`RatesDialectError`] for any missing/invalid required field. A malformed
/// request never panics and never yields a partial descriptor.
pub fn decode_bond_rfq(frame: &FrameCursor<'_>) -> Result<BondRfq, RatesDialectError> {
    let quote_req_id = frame
        .get(131)
        .map(<[u8]>::to_vec)
        .ok_or(RatesDialectError::MissingQuoteReqId)?;
    let symbol = frame
        .get(55)
        .map(<[u8]>::to_vec)
        .ok_or(RatesDialectError::MissingSymbol)?;

    if frame.get(167) != Some(SEC_TYPE_BOND) {
        return Err(RatesDialectError::BadSecurityType);
    }

    // Coupon may be zero (a zero-coupon bond); it may never be negative or non-finite.
    let coupon_rate = frame
        .get(223)
        .and_then(parse_float)
        .filter(|c| *c >= 0.0 && c.is_finite())
        .ok_or(RatesDialectError::BadCoupon)?;
    let frequency = frame
        .get(TAG_COUPON_FREQUENCY)
        .and_then(parse_bond_frequency)
        .ok_or(RatesDialectError::BadCouponFrequency)?;
    let day_count = frame
        .get(TAG_DAY_COUNT)
        .and_then(parse_bond_day_count)
        .ok_or(RatesDialectError::BadDayCount)?;
    let maturity = frame
        .get(541)
        .and_then(parse_fix_date)
        .ok_or(RatesDialectError::BadMaturity)?;
    // Redemption defaults to par when the tag is absent; a present value must be a
    // finite, strictly-positive face.
    let redemption = match frame.get(TAG_REDEMPTION) {
        None => DEFAULT_REDEMPTION,
        Some(v) => parse_float(v)
            .filter(|r| *r > 0.0 && r.is_finite())
            .ok_or(RatesDialectError::BadRedemption)?,
    };
    let notional = frame
        .get(38)
        .and_then(parse_float)
        .filter(|n| *n > 0.0 && n.is_finite())
        .ok_or(RatesDialectError::BadNotional)?;
    // A bond's directional intent is a long/short (buy/sell), not pay/receive-fixed;
    // absent `Side(54)` ⇒ a two-way (bid/offer) request. The two-way instrument is
    // priced at magnitude by the edge (a bond's price/DV01 are side-independent), so
    // carrying `SIDE_TWO_WAY` on the decoded instrument is faithful to the request.
    let side = match frame.get(54) {
        None => Side::TwoWay,
        Some([SIDE_BUY]) => Side::Buy,
        Some([SIDE_SELL]) => Side::Sell,
        Some(_) => return Err(RatesDialectError::BadSide),
    };
    let subscription = SubscriptionRequest::from_fix(frame.get(263))?;

    Ok(BondRfq {
        quote_req_id,
        symbol,
        notional,
        subscription,
        instrument: BondInstrument {
            coupon_rate,
            coupon_frequency: frequency as i32,
            day_count: day_count as i32,
            maturity_date: Some(maturity),
            redemption,
            side: side as i32,
        },
    })
}

/// Map the coupons-per-year int carried on [`TAG_COUPON_FREQUENCY`] to a
/// [`PaymentFrequency`]; any other count is a malformed frequency.
fn parse_bond_frequency(v: &[u8]) -> Option<PaymentFrequency> {
    match parse_u32(v)? {
        1 => Some(PaymentFrequency::Annual),
        2 => Some(PaymentFrequency::SemiAnnual),
        4 => Some(PaymentFrequency::Quarterly),
        _ => None,
    }
}

/// The coupons-per-year int [`build_bond_quote_request`] stamps for a [`PaymentFrequency`].
fn periods_per_year(f: PaymentFrequency) -> i64 {
    match f {
        PaymentFrequency::Annual => 1,
        PaymentFrequency::SemiAnnual => 2,
        PaymentFrequency::Quarterly => 4,
    }
}

/// Map the day-count mnemonic carried on [`TAG_DAY_COUNT`] to an [`AccrualBasis`]; an
/// unrecognised token is a malformed day-count.
fn parse_bond_day_count(v: &[u8]) -> Option<AccrualBasis> {
    if v == DAY_COUNT_ACT_360 {
        Some(AccrualBasis::Act360)
    } else if v == DAY_COUNT_ACT_365F {
        Some(AccrualBasis::Act365Fixed)
    } else if v == DAY_COUNT_30_360 {
        Some(AccrualBasis::Thirty360BondBasis)
    } else {
        None
    }
}

/// The day-count mnemonic [`build_bond_quote_request`] stamps for an [`AccrualBasis`].
fn day_count_token(d: AccrualBasis) -> &'static [u8] {
    match d {
        AccrualBasis::Act360 => DAY_COUNT_ACT_360,
        AccrualBasis::Act365Fixed => DAY_COUNT_ACT_365F,
        AccrualBasis::Thirty360BondBasis => DAY_COUNT_30_360,
    }
}

/// Parse a FIX `MaturityDate(541)` (`YYYYMMDD`, 8 ASCII digits) into a [`BrokenDate`].
/// Basic month/day range validation only; the server's date resolution + `Bond::new`
/// apply the authoritative civil-date + `maturity > settlement` checks.
fn parse_fix_date(v: &[u8]) -> Option<BrokenDate> {
    if v.len() != 8 || !v.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let s = core::str::from_utf8(v).ok()?;
    let year: i32 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(4..6)?.parse().ok()?;
    let day: u32 = s.get(6..8)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(BrokenDate { year, month, day })
}

/// Format a [`BrokenDate`] as a FIX `MaturityDate(541)` (`YYYYMMDD`).
fn fmt_fix_date(d: &BrokenDate) -> String {
    format!("{:04}{:02}{:02}", d.year, d.month, d.day)
}

/// The `Side(54)` byte a bond side encodes as, or `None` for a two-way request.
fn side_to_fix_byte(side: Side) -> Option<u8> {
    match side {
        Side::Buy => Some(SIDE_BUY),
        Side::Sell => Some(SIDE_SELL),
        Side::TwoWay => None,
    }
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

    /// The party-aware builder with `None` is byte-identical to the plain builder (so every
    /// existing caller is unaffected), and with `Some(name)` it stamps a `PartyID(448)` the
    /// venue reads as the display counterparty — the seam a SIM rotates for varied names.
    #[test]
    fn party_id_is_optional_and_round_trips_on_the_wire() {
        let p = RatesQuoteRequestParams {
            quote_req_id: b"RFQ-1",
            symbol: b"USD-OIS",
            tenor_years: 5,
            notional: 100_000_000.0,
            side: RatesSide::PayFixed,
            subscription: SubscriptionRequest::Snapshot,
        };
        let mut e1 = FrameEncoder::new();
        let plain = build_rates_quote_request(&header(), &p, &mut e1);
        let mut e2 = FrameEncoder::new();
        let none = build_rates_quote_request_with_party(&header(), &p, None, &mut e2);
        assert_eq!(
            plain, none,
            "None party must be byte-identical to the plain builder"
        );

        let mut e3 = FrameEncoder::new();
        let named = build_rates_quote_request_with_party(
            &header(),
            &p,
            Some(b"Millennium Capital"),
            &mut e3,
        );
        let frame = FrameCursor::parse(&named).expect("named frame parses");
        assert_eq!(
            frame.get(crate::messages::TAG_PARTY_ID),
            Some(b"Millennium Capital".as_ref()),
        );
        // The RFQ payload still decodes exactly — the party block is additive.
        let rfq = decode_rates_rfq(&frame).expect("rfq still decodes");
        assert_eq!(rfq.symbol, b"USD-OIS");
        assert_eq!(rfq.tenor_years, 5);
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

    // ---- cash-bond arm ----

    fn bond_params() -> BondQuoteRequestParams<'static> {
        BondQuoteRequestParams {
            quote_req_id: b"BRFQ-1",
            symbol: b"US-TREASURY-5Y",
            coupon_rate: 0.045,
            coupon_frequency: PaymentFrequency::SemiAnnual,
            day_count: AccrualBasis::Thirty360BondBasis,
            maturity: BrokenDate {
                year: 2031,
                month: 6,
                day: 25,
            },
            redemption: 100.0,
            notional: 25_000_000.0,
            side: Side::Buy,
            subscription: SubscriptionRequest::Snapshot,
        }
    }

    fn round_trip_bond(p: &BondQuoteRequestParams<'_>) -> BondRfq {
        let mut enc = FrameEncoder::new();
        let raw = build_bond_quote_request(&header(), p, &mut enc);
        let frame = FrameCursor::parse(&raw).expect("frame parses");
        decode_bond_rfq(&frame).expect("bond rfq decodes")
    }

    #[test]
    fn bond_rfq_round_trips_field_for_field() {
        let p = bond_params();
        let rfq = round_trip_bond(&p);

        // The RFQ envelope.
        assert_eq!(rfq.quote_req_id, b"BRFQ-1");
        assert_eq!(rfq.symbol, b"US-TREASURY-5Y");
        assert_eq!(rfq.notional, 25_000_000.0);
        assert_eq!(rfq.subscription, SubscriptionRequest::Snapshot);

        // The decoded instrument is EXACTLY the intended landed `BondInstrument`.
        assert_eq!(
            rfq.instrument,
            BondInstrument {
                coupon_rate: 0.045,
                coupon_frequency: PaymentFrequency::SemiAnnual as i32,
                day_count: AccrualBasis::Thirty360BondBasis as i32,
                maturity_date: Some(BrokenDate {
                    year: 2031,
                    month: 6,
                    day: 25,
                }),
                redemption: 100.0,
                side: Side::Buy as i32,
            }
        );
    }

    #[test]
    fn bond_two_way_request_omits_side() {
        let mut p = bond_params();
        p.side = Side::Sell;
        let sell = round_trip_bond(&p);
        assert_eq!(sell.instrument.side, Side::Sell as i32);

        p.side = Side::TwoWay;
        let two_way = round_trip_bond(&p);
        assert_eq!(two_way.instrument.side, Side::TwoWay as i32);
    }

    #[test]
    fn bond_absent_redemption_defaults_to_par() {
        // Hand-build a bond QuoteRequest WITHOUT the redemption tag: it defaults to par.
        let mut enc = FrameEncoder::new();
        enc.clear();
        header().encode(MsgType::QuoteRequest, &mut enc);
        enc.push(131, b"BRFQ-2");
        enc.push(55, b"US-CORP");
        enc.push(167, SEC_TYPE_BOND);
        enc.push(223, b"0.05");
        enc.push_int(38, 10_000_000);
        enc.push(541, b"20340101");
        enc.push_int(TAG_COUPON_FREQUENCY, 2);
        enc.push(TAG_DAY_COUNT, DAY_COUNT_ACT_365F);
        let raw = enc.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        let rfq = decode_bond_rfq(&frame).expect("decodes with default redemption");
        assert_eq!(rfq.instrument.redemption, DEFAULT_REDEMPTION);
        assert_eq!(rfq.instrument.side, Side::TwoWay as i32);
        assert_eq!(rfq.instrument.day_count, AccrualBasis::Act365Fixed as i32);
    }

    #[test]
    fn zero_coupon_bond_round_trips() {
        let mut p = bond_params();
        p.coupon_rate = 0.0;
        p.coupon_frequency = PaymentFrequency::Annual;
        let rfq = round_trip_bond(&p);
        assert_eq!(rfq.instrument.coupon_rate, 0.0);
        assert_eq!(
            rfq.instrument.coupon_frequency,
            PaymentFrequency::Annual as i32
        );
    }

    #[test]
    fn fractional_coupon_and_notional_survive() {
        let mut p = bond_params();
        p.coupon_rate = 0.0375;
        p.notional = 12_345_678.5;
        p.redemption = 99.5;
        let rfq = round_trip_bond(&p);
        assert_eq!(rfq.instrument.coupon_rate, 0.0375);
        assert_eq!(rfq.notional, 12_345_678.5);
        assert_eq!(rfq.instrument.redemption, 99.5);
    }

    #[test]
    fn bond_rejects_wrong_security_type() {
        let mut enc = FrameEncoder::new();
        enc.clear();
        header().encode(MsgType::QuoteRequest, &mut enc);
        enc.push(131, b"BRFQ-X");
        enc.push(55, b"US-CORP");
        enc.push(167, SEC_TYPE_OIS); // an OIS, decoded on the bond arm
        enc.push(223, b"0.05");
        enc.push_int(38, 1_000_000);
        enc.push(541, b"20340101");
        enc.push_int(TAG_COUPON_FREQUENCY, 2);
        enc.push(TAG_DAY_COUNT, DAY_COUNT_30_360);
        let raw = enc.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(
            decode_bond_rfq(&frame),
            Err(RatesDialectError::BadSecurityType)
        );
    }

    #[test]
    fn bond_rejects_bad_frequency_day_count_and_maturity() {
        let base = |freq: &[u8], dc: &[u8], mat: &[u8]| {
            let mut enc = FrameEncoder::new();
            enc.clear();
            header().encode(MsgType::QuoteRequest, &mut enc);
            enc.push(131, b"BRFQ-3");
            enc.push(55, b"US-CORP");
            enc.push(167, SEC_TYPE_BOND);
            enc.push(223, b"0.05");
            enc.push_int(38, 1_000_000);
            enc.push(541, mat);
            enc.push(TAG_COUPON_FREQUENCY, freq);
            enc.push(TAG_DAY_COUNT, dc);
            enc.finish()
        };

        // Frequency 3 is not 1/2/4.
        let raw = base(b"3", DAY_COUNT_30_360, b"20340101");
        assert_eq!(
            decode_bond_rfq(&FrameCursor::parse(&raw).unwrap()),
            Err(RatesDialectError::BadCouponFrequency)
        );
        // An unrecognised day-count mnemonic.
        let raw = base(b"2", b"ACT_ACT", b"20340101");
        assert_eq!(
            decode_bond_rfq(&FrameCursor::parse(&raw).unwrap()),
            Err(RatesDialectError::BadDayCount)
        );
        // A malformed maturity (month 13).
        let raw = base(b"2", DAY_COUNT_30_360, b"20341301");
        assert_eq!(
            decode_bond_rfq(&FrameCursor::parse(&raw).unwrap()),
            Err(RatesDialectError::BadMaturity)
        );
    }

    #[test]
    fn bond_rejects_negative_coupon_and_non_positive_redemption() {
        let with = |coupon: &[u8], redemption: Option<&[u8]>| {
            let mut enc = FrameEncoder::new();
            enc.clear();
            header().encode(MsgType::QuoteRequest, &mut enc);
            enc.push(131, b"BRFQ-4");
            enc.push(55, b"US-CORP");
            enc.push(167, SEC_TYPE_BOND);
            enc.push(223, coupon);
            enc.push_int(38, 1_000_000);
            enc.push(541, b"20340101");
            enc.push_int(TAG_COUPON_FREQUENCY, 2);
            enc.push(TAG_DAY_COUNT, DAY_COUNT_30_360);
            if let Some(r) = redemption {
                enc.push(TAG_REDEMPTION, r);
            }
            enc.finish()
        };

        let raw = with(b"-0.01", None);
        assert_eq!(
            decode_bond_rfq(&FrameCursor::parse(&raw).unwrap()),
            Err(RatesDialectError::BadCoupon)
        );
        let raw = with(b"0.05", Some(b"0"));
        assert_eq!(
            decode_bond_rfq(&FrameCursor::parse(&raw).unwrap()),
            Err(RatesDialectError::BadRedemption)
        );
    }
}
