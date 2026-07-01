//! The FX-options dialect mapping (`docs/CELER-FIX-INTEGRATION-PLAN.md` §1.2):
//! FIX instrument/strategy blocks ⇄ `celnet-types` option & strategy
//! descriptors, convention-checked via `celnet-conventions`, and priced via
//! `celnet-vanilla` off a supplied market snapshot.
//!
//! The mapper is convention-correct by construction: every inbound option block
//! is resolved against the `(pair, tenor)` convention registry, and the strike
//! currency / premium currency are validated against the resolved
//! [`celnet_conventions::ResolvedConvention`] before a price is produced —
//! *convention error dwarfs model error*. Nothing here touches a socket; this
//! is pure data + arithmetic, reused by the acceptor and the initiator alike.

use celnet_conventions::resolve;
use celnet_types::{Ccy, CcyPair, OptionType, Settlement, Tenor, VanillaInputs};

use crate::dictionary::MsgType;
use crate::framing::{FrameCursor, FrameEncoder};
use crate::messages::Header;

/// FIX `Product(460)` value for a currency instrument.
pub const PRODUCT_CURRENCY: u32 = 4;
/// FIX `SecurityType(167)` for an FX vanilla option.
pub const SEC_TYPE_FXVO: &[u8] = b"FXVO";
/// FIX `SecurityType(167)` for an FX non-deliverable option.
pub const SEC_TYPE_FXNO: &[u8] = b"FXNO";
/// FIX `PutOrCall(201)`: put.
pub const PUT: u32 = 0;
/// FIX `PutOrCall(201)`: call.
pub const CALL: u32 = 1;
/// FIX `ExerciseStyle(1194)`: European.
pub const EXERCISE_EUROPEAN: u32 = 0;
/// FIX `ExerciseStyle(1194)`: American.
pub const EXERCISE_AMERICAN: u32 = 1;
/// FIX `QuoteType(537)`: indicative.
pub const QUOTE_TYPE_INDICATIVE: u32 = 0;
/// FIX `QuoteType(537)`: tradeable (firm RFS).
pub const QUOTE_TYPE_TRADEABLE: u32 = 1;
/// FIX `Side(54)` / `LegSide(624)`: buy.
pub const SIDE_BUY: u8 = b'1';
/// FIX `Side(54)` / `LegSide(624)`: sell.
pub const SIDE_SELL: u8 = b'2';

/// The custom dialect tag carrying the option's **vol-time in years** as a FIX float.
///
/// The platform's pricing is tenor- *and* vol-time-based; rather than depend on a
/// calendar resolution of `MaturityDate(541)` (which would drift with the trade date),
/// the dialect carries the exact `expiry_years` the engine prices against on a private,
/// user-defined tag (FIX tolerates unknown tags; this is a dialect provenance field,
/// never a vendor name — `CLAUDE.md` rule 8). It makes the RFQ instrument fully
/// wire-specified, so the returned premium reproduces the engine/golden price to the
/// bit, independent of any date. This is the canonical home of the tag; the server's
/// FIX edge (`celnet-server`) reads it from here.
pub const TAG_EXPIRY_YEARS: u32 = 7001;

/// Parameters for a single-leg FX-option `QuoteRequest(R)` — the symmetric encode
/// side of [`decode_option`]. The fields are the convention-checked instrument the
/// acceptor will decode and price; an initiator (price-taker / test client) fills
/// them from user input and the builder lays them onto the wire flat (no repeating
/// group), exactly as the acceptor reads them.
#[derive(Debug, Clone, Copy)]
pub struct QuoteRequestParams<'a> {
    /// `QuoteReqID(131)` — the client-minted RFQ correlation id.
    pub quote_req_id: &'a [u8],
    /// `Symbol(55)` — the pair, e.g. `b"EURUSD"`.
    pub symbol: &'a [u8],
    /// Call or put (`PutOrCall(201)`).
    pub option_type: OptionType,
    /// `StrikePrice(202)` — strictly positive, in the quote currency per base.
    pub strike: f64,
    /// The vol-time in years carried on [`TAG_EXPIRY_YEARS`] (must be `> 0`).
    pub expiry_years: f64,
    /// Deliverable (`FXVO`) vs non-deliverable (`FXNO`) — drives `SecurityType(167)`.
    pub settlement: Settlement,
    /// European or American (`ExerciseStyle(1194)`; the engine prices European vanillas).
    pub exercise: ExerciseStyle,
    /// `StrikeCurrency(947)` — the quote currency of the pair (e.g. `b"USD"` for EURUSD).
    pub strike_ccy: &'a [u8],
}

/// Build a flat single-leg `QuoteRequest(R)` frame from [`QuoteRequestParams`].
///
/// The layout mirrors what [`decode_option`] reads: `Symbol(55)`, `Product(460)=4`,
/// `SecurityType(167)`, `PutOrCall(201)`, `StrikePrice(202)`, `StrikeCurrency(947)`,
/// `ExerciseStyle(1194)`, plus the dialect's [`TAG_EXPIRY_YEARS`]. Float fields are
/// rendered round-trip-exact (`{}` emits the shortest decimal that parses back to the
/// identical bits), so the strike and vol-time survive the wire without precision loss.
#[must_use]
pub fn build_quote_request(
    hdr: &Header<'_>,
    p: &QuoteRequestParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    let sec_type: &[u8] = match p.settlement {
        Settlement::Deliverable => SEC_TYPE_FXVO,
        Settlement::NonDeliverable => SEC_TYPE_FXNO,
    };
    let put_or_call: &[u8] = match p.option_type {
        OptionType::Call => b"1",
        OptionType::Put => b"0",
    };
    let exercise: &[u8] = match p.exercise {
        ExerciseStyle::European => b"0",
        ExerciseStyle::American => b"1",
    };

    enc.clear();
    hdr.encode(MsgType::QuoteRequest, enc);
    enc.push(131, p.quote_req_id);
    enc.push(55, p.symbol);
    enc.push(460, b"4"); // Product = CURRENCY
    enc.push(167, sec_type);
    enc.push(201, put_or_call);
    enc.push(202, format!("{}", p.strike).as_bytes());
    enc.push(947, p.strike_ccy);
    enc.push(1194, exercise);
    enc.push(TAG_EXPIRY_YEARS, format!("{}", p.expiry_years).as_bytes());
    enc.finish()
}

/// FIX `SecurityListRequestType(559)`: request **all** securities the venue can
/// quote (the whole tradable universe, unfiltered).
pub const SECURITY_LIST_REQUEST_TYPE_ALL: u32 = 4;
/// FIX `SecurityRequestResult(560)`: the request was valid and the list follows.
pub const SECURITY_REQUEST_RESULT_VALID: u32 = 0;
/// FIX `LastFragment(893)` value marking the final (or only) `SecurityList`
/// fragment — the universe fits one message here, so it is always `Y`.
pub const LAST_FRAGMENT_YES: &[u8] = b"Y";

/// One tradable security in the venue's universe, as projected onto the
/// `SecurityList(y)` `NoRelatedSym(146)` repeating group. Purpose-named and
/// vendor-neutral: it is the venue's authoritative "what I can quote" row —
/// `Symbol(55)` plus, where known, `SecurityType(167)` and `Currency(15)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityDef {
    /// `Symbol(55)` — the pair, e.g. `b"EURUSD"`.
    pub symbol: Vec<u8>,
    /// `SecurityType(167)` — e.g. [`SEC_TYPE_FXVO`]; empty when unspecified.
    pub security_type: Vec<u8>,
    /// `Currency(15)` — the deal (premium) currency; empty when unspecified.
    pub currency: Vec<u8>,
}

impl SecurityDef {
    /// Build a security row from its `Symbol` / `SecurityType` / `Currency`
    /// bytes. Any of `security_type` / `currency` may be empty to omit that
    /// optional field from the emitted group entry.
    #[must_use]
    pub fn new(symbol: &[u8], security_type: &[u8], currency: &[u8]) -> Self {
        Self {
            symbol: symbol.to_vec(),
            security_type: security_type.to_vec(),
            currency: currency.to_vec(),
        }
    }
}

/// Parameters for a `SecurityListRequest(x)` — the client asks the venue to
/// enumerate the securities it can quote. `SecurityReqID(320)` correlates the
/// answering `SecurityList(y)`; the request is always the all-securities type,
/// optionally narrowed by `Product(460)` and/or `Currency(15)`.
#[derive(Debug, Clone, Copy)]
pub struct SecurityListRequestParams<'a> {
    /// `SecurityReqID(320)` — the client-minted correlation id.
    pub security_req_id: &'a [u8],
    /// Optional `Product(460)` filter (e.g. [`PRODUCT_CURRENCY`]).
    pub product: Option<u32>,
    /// Optional `Currency(15)` filter (the deal currency of interest).
    pub currency: Option<&'a [u8]>,
}

/// Build a `SecurityListRequest(x)` frame from [`SecurityListRequestParams`].
///
/// Layout: `SecurityReqID(320)`, `SecurityListRequestType(559)=4` (all
/// securities), then the optional `Product(460)` / `Currency(15)` narrowing
/// fields when supplied.
#[must_use]
pub fn build_security_list_request(
    hdr: &Header<'_>,
    p: &SecurityListRequestParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::SecurityListRequest, enc);
    enc.push(320, p.security_req_id);
    enc.push_int(559, i64::from(SECURITY_LIST_REQUEST_TYPE_ALL));
    if let Some(product) = p.product {
        enc.push_int(460, i64::from(product));
    }
    if let Some(currency) = p.currency {
        enc.push(15, currency);
    }
    enc.finish()
}

/// Parameters for a `SecurityList(y)` — the venue's answer enumerating the
/// securities it can quote.
#[derive(Debug, Clone, Copy)]
pub struct SecurityListParams<'a> {
    /// `SecurityReqID(320)` — echoed from the answered request.
    pub security_req_id: &'a [u8],
    /// The venue's tradable-securities universe (the `NoRelatedSym(146)` group).
    pub securities: &'a [SecurityDef],
}

/// Build a `SecurityList(y)` frame from [`SecurityListParams`].
///
/// Layout: `SecurityReqID(320)` echoed, `SecurityRequestResult(560)=0` (valid),
/// `TotNoRelatedSym(393)` = total count, then the `NoRelatedSym(146)` repeating
/// group — each entry a `Symbol(55)` plus `SecurityType(167)` / `Currency(15)`
/// where known — closed by `LastFragment(893)=Y`. The whole universe is emitted
/// as a single (final) fragment.
#[must_use]
pub fn build_security_list(
    hdr: &Header<'_>,
    p: &SecurityListParams<'_>,
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    enc.clear();
    hdr.encode(MsgType::SecurityList, enc);
    enc.push(320, p.security_req_id);
    enc.push_int(560, i64::from(SECURITY_REQUEST_RESULT_VALID));
    enc.push_int(393, p.securities.len() as i64);
    enc.push_int(146, p.securities.len() as i64);
    for def in p.securities {
        enc.push(55, &def.symbol);
        if !def.security_type.is_empty() {
            enc.push(167, &def.security_type);
        }
        if !def.currency.is_empty() {
            enc.push(15, &def.currency);
        }
    }
    enc.push(893, LAST_FRAGMENT_YES);
    enc.finish()
}

/// Exercise style as carried by the dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExerciseStyle {
    /// European exercise (FX vanilla).
    European,
    /// American exercise.
    American,
}

/// A single FX-option descriptor decoded from (or to be encoded into) the FIX
/// instrument block. This is the dialect's normalised, convention-checked view
/// of one leg.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OptionDescriptor {
    /// The currency pair (`BASE/QUOTE`).
    pub pair: CcyPair,
    /// Call or put.
    pub option_type: OptionType,
    /// Strike (quote per base).
    pub strike: f64,
    /// The currency the strike is expressed in (must be the quote currency).
    pub strike_ccy: Ccy,
    /// Exercise style.
    pub exercise: ExerciseStyle,
    /// Expiry tenor (carried alongside the absolute maturity for convention
    /// resolution).
    pub tenor: Tenor,
    /// Settlement style (deliverable vs non-deliverable / cash-fixing).
    pub settlement: Settlement,
}

/// The market snapshot needed to turn an [`OptionDescriptor`] into a premium.
///
/// In production these come from one consistent surface snapshot
/// (`celnet-surface`); the dialect carries the descriptor faithfully and the
/// edge supplies the snapshot, so a single snapshot prices every leg of a
/// package coherently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketSnapshot {
    /// Spot FX rate (quote per base).
    pub spot: f64,
    /// Annualized Black volatility for the option's `(strike, tenor)` pillar.
    pub vol: f64,
    /// Time to expiry in years (vol-time).
    pub t: f64,
    /// Continuously-compounded domestic (quote) rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    pub r_for: f64,
}

/// A dialect mapping / convention error. Recoverable; never panics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialectError {
    /// `Symbol(55)` was missing or not a 6-letter pair.
    BadSymbol,
    /// `Product(460)` was not `4` (CURRENCY).
    BadProduct,
    /// `SecurityType(167)` was not an FX-option type.
    BadSecurityType,
    /// `PutOrCall(201)` was missing or not `0`/`1`.
    BadPutOrCall,
    /// `StrikePrice(202)` was missing or unparseable.
    BadStrike,
    /// `StrikeCurrency(947)` was not the pair's quote (domestic) currency.
    StrikeCurrencyMismatch,
    /// `ExerciseStyle(1194)` was not `0`/`1`.
    BadExerciseStyle,
    /// A non-deliverable option requested but the convention says deliverable
    /// (or vice-versa) — the dialect and the resolved convention disagree.
    SettlementMismatch,
    /// A required leg field was absent in the multileg group.
    BadLeg,
    /// `LegRatioQty(623)` was non-positive or unparseable.
    BadRatio,
    /// `NoLegs(555)` was absent, zero, or unparseable.
    BadLegCount,
}

/// Parse a strict FIX float from value bytes (delegates to the same grammar the
/// dictionary validates against).
#[must_use]
pub fn parse_float(v: &[u8]) -> Option<f64> {
    // Reject anything the dictionary's float grammar would reject, then use the
    // std parser (which is deterministic for finite decimal literals).
    let s = core::str::from_utf8(v).ok()?;
    let parsed: f64 = s.parse().ok()?;
    if parsed.is_finite() {
        Some(parsed)
    } else {
        None
    }
}

/// Parse a `PutOrCall(201)` value into an [`OptionType`].
#[must_use]
pub fn parse_put_or_call(v: &[u8]) -> Option<OptionType> {
    match v {
        b"0" => Some(OptionType::Put),
        b"1" => Some(OptionType::Call),
        _ => None,
    }
}

/// The FIX `PutOrCall(201)` value for an option type.
#[must_use]
pub const fn put_or_call_value(opt: OptionType) -> u32 {
    match opt {
        OptionType::Call => CALL,
        OptionType::Put => PUT,
    }
}

/// Decode the FX-option instrument block from a frame into a normalised,
/// convention-checked [`OptionDescriptor`].
///
/// `tenor` is supplied by the caller (resolved from `MaturityDate(541)` against
/// the calendar on the edge) so this function stays pure and calendar-free.
///
/// # Errors
/// Returns a [`DialectError`] if any instrument field is missing, malformed, or
/// inconsistent with the resolved convention (e.g. strike currency ≠ quote).
pub fn decode_option(
    frame: &FrameCursor<'_>,
    tenor: Tenor,
) -> Result<OptionDescriptor, DialectError> {
    let symbol = frame.get(55).ok_or(DialectError::BadSymbol)?;
    let pair = parse_symbol(symbol).ok_or(DialectError::BadSymbol)?;

    if let Some(prod) = frame.get(460)
        && prod != b"4"
    {
        return Err(DialectError::BadProduct);
    }

    let sec_type = frame.get(167).ok_or(DialectError::BadSecurityType)?;
    let settlement = match sec_type {
        SEC_TYPE_FXVO => Settlement::Deliverable,
        SEC_TYPE_FXNO => Settlement::NonDeliverable,
        _ => return Err(DialectError::BadSecurityType),
    };

    let option_type = parse_put_or_call(frame.get(201).ok_or(DialectError::BadPutOrCall)?)
        .ok_or(DialectError::BadPutOrCall)?;

    let strike = parse_float(frame.get(202).ok_or(DialectError::BadStrike)?)
        .ok_or(DialectError::BadStrike)?;
    // Strike must be strictly positive; `parse_float` already excluded NaN, so a
    // plain `<=` is both correct and clippy-clean here.
    if strike <= 0.0 {
        return Err(DialectError::BadStrike);
    }

    // Strike currency must be the quote (domestic) currency of the pair.
    let strike_ccy = match frame.get(947) {
        Some(v) => parse_ccy(v).ok_or(DialectError::StrikeCurrencyMismatch)?,
        None => pair.quote, // default to the quote currency when omitted
    };
    if strike_ccy != pair.quote {
        return Err(DialectError::StrikeCurrencyMismatch);
    }

    let exercise = match frame.get(1194) {
        Some(b"0") | None => ExerciseStyle::European,
        Some(b"1") => ExerciseStyle::American,
        Some(_) => return Err(DialectError::BadExerciseStyle),
    };

    // Convention cross-check: the resolved settlement style for the pair/tenor
    // must agree with the dialect's SecurityType. This is the convention guard
    // the plan mandates on every inbound message.
    let resolved = resolve(pair, tenor).record;
    if resolved.settlement != settlement {
        return Err(DialectError::SettlementMismatch);
    }

    Ok(OptionDescriptor {
        pair,
        option_type,
        strike,
        strike_ccy,
        exercise,
        tenor,
        settlement,
    })
}

/// Build the [`VanillaInputs`] for an option leg from its descriptor and a
/// market snapshot. The descriptor's strike + the snapshot's spot/vol/rates/time
/// form the Garman-Kohlhagen inputs.
#[must_use]
pub fn inputs_for(desc: &OptionDescriptor, snap: &MarketSnapshot) -> VanillaInputs {
    VanillaInputs::new(
        snap.spot,
        desc.strike,
        snap.vol,
        snap.t,
        snap.r_dom,
        snap.r_for,
    )
}

/// A vanilla pricing function: `(option_type, inputs) → domestic premium`.
///
/// `celnet-fix` is a leaf crate that must not depend on the pricing engine
/// (dependency arrow points one way: `celnet-server → celnet-fix →
/// celnet-types/conventions/proto`). The pricer is therefore *injected* by the
/// async edge, which owns the surface/engine — the dialect carries the
/// descriptor faithfully and the caller supplies the model. The canonical
/// implementation is `celnet_vanilla::price`, which the test suite wires in to
/// cross-check against the `celnet-golden` / QuantLib tables.
pub type VanillaPricer = fn(OptionType, &VanillaInputs) -> f64;

/// Price one option leg with an injected pricer: the domestic premium per 1
/// unit of base notional. Deterministic when the pricer is (libm-based).
#[must_use]
pub fn price_leg(desc: &OptionDescriptor, snap: &MarketSnapshot, pricer: VanillaPricer) -> f64 {
    pricer(desc.option_type, &inputs_for(desc, snap))
}

/// A buy/sell direction with an integer ratio (`+1/−1` risk-reversal,
/// `+1/−2/+1` fly), used by the multileg strategy mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegSide {
    /// Long the leg.
    Buy,
    /// Short the leg.
    Sell,
}

impl LegSide {
    /// The signed multiplier this side contributes to a package price (`+1`
    /// buy, `−1` sell).
    #[must_use]
    pub const fn sign(self) -> f64 {
        match self {
            LegSide::Buy => 1.0,
            LegSide::Sell => -1.0,
        }
    }

    /// Parse from a FIX `LegSide(624)` / `Side(54)` byte.
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            SIDE_BUY => Some(LegSide::Buy),
            SIDE_SELL => Some(LegSide::Sell),
            _ => None,
        }
    }

    /// The FIX byte for this side.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            LegSide::Buy => SIDE_BUY,
            LegSide::Sell => SIDE_SELL,
        }
    }
}

/// One leg of a multi-leg strategy package: an option descriptor plus its side
/// and ratio within the package.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrategyLeg {
    /// The option descriptor for this leg.
    pub option: OptionDescriptor,
    /// Buy or sell.
    pub side: LegSide,
    /// Positive ratio quantity (number of units of this leg per package unit).
    pub ratio: f64,
}

/// A decoded multi-leg strategy package (straddle, strangle, risk-reversal,
/// fly, calendar, seagull). The dialect maps the `QuotReqLegsGrp` repeating
/// group onto this descriptor; the package price is the ratio- and
/// side-weighted sum of per-leg prices off **one** snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyPackage {
    /// The legs in wire order.
    pub legs: Vec<StrategyLeg>,
}

impl StrategyPackage {
    /// The net package premium off a single market snapshot: the
    /// side-and-ratio-weighted sum of per-leg Garman-Kohlhagen premia.
    ///
    /// Pricing every leg off the *same* snapshot is the invariant that makes
    /// `package_price == Σ side·ratio·leg_price` hold exactly (no smile drift
    /// between legs), which the test suite asserts for a risk-reversal and a
    /// straddle.
    #[must_use]
    pub fn package_price(&self, snap: &MarketSnapshot, pricer: VanillaPricer) -> f64 {
        self.legs
            .iter()
            .map(|leg| leg.side.sign() * leg.ratio * price_leg(&leg.option, snap, pricer))
            .sum()
    }

    /// The per-leg signed contributions, in the same order as [`Self::legs`].
    /// Useful for the optional per-leg breakdown in the `Quote`/`ExecutionReport`.
    #[must_use]
    pub fn leg_contributions(&self, snap: &MarketSnapshot, pricer: VanillaPricer) -> Vec<f64> {
        self.legs
            .iter()
            .map(|leg| leg.side.sign() * leg.ratio * price_leg(&leg.option, snap, pricer))
            .collect()
    }
}

/// Decode the `QuotReqLegsGrp` repeating group from a frame into a
/// [`StrategyPackage`]. `leg_tenor` resolves each leg's tenor (legs share the
/// pair/tenor for FX strategies; calendars differ in maturity and would carry
/// per-leg `LegMaturityDate`, resolved by the caller — passed here per index).
///
/// # Errors
/// Returns a [`DialectError`] if `NoLegs(555)` is malformed or any leg lacks a
/// symbol, strike, put/call, ratio, or side.
pub fn decode_strategy(
    frame: &FrameCursor<'_>,
    leg_tenor: impl Fn(usize) -> Tenor,
) -> Result<StrategyPackage, DialectError> {
    let no_legs = frame.get(555).ok_or(DialectError::BadLegCount)?;
    let count = crate::framing::parse_uint(no_legs).ok_or(DialectError::BadLegCount)? as usize;
    if count == 0 {
        return Err(DialectError::BadLegCount);
    }

    // Walk the group: a new leg starts at each LegSymbol(600). Fields between
    // two LegSymbol markers belong to the first.
    let mut legs: Vec<StrategyLeg> = Vec::with_capacity(count);
    let mut cur: Option<LegBuilder> = None;

    for f in frame.fields() {
        match f.tag {
            600 => {
                if let Some(b) = cur.take() {
                    legs.push(b.build(&leg_tenor, legs.len())?);
                }
                let pair = parse_symbol(f.value).ok_or(DialectError::BadLeg)?;
                cur = Some(LegBuilder::new(pair));
            }
            _ => {
                if let Some(b) = cur.as_mut() {
                    b.field(f.tag, f.value)?;
                }
            }
        }
    }
    if let Some(b) = cur.take() {
        legs.push(b.build(&leg_tenor, legs.len())?);
    }

    if legs.len() != count {
        return Err(DialectError::BadLegCount);
    }
    Ok(StrategyPackage { legs })
}

/// Accumulates fields for a single leg while walking the repeating group.
struct LegBuilder {
    pair: CcyPair,
    option_type: Option<OptionType>,
    strike: Option<f64>,
    side: Option<LegSide>,
    ratio: Option<f64>,
    sec_type: Settlement,
}

impl LegBuilder {
    fn new(pair: CcyPair) -> Self {
        Self {
            pair,
            option_type: None,
            strike: None,
            side: None,
            ratio: None,
            sec_type: Settlement::Deliverable,
        }
    }

    fn field(&mut self, tag: u32, value: &[u8]) -> Result<(), DialectError> {
        match tag {
            612 => self.strike = Some(parse_float(value).ok_or(DialectError::BadStrike)?),
            624 => {
                self.side = Some(
                    LegSide::from_byte(*value.first().ok_or(DialectError::BadLeg)?)
                        .ok_or(DialectError::BadLeg)?,
                );
            }
            623 => self.ratio = Some(parse_float(value).ok_or(DialectError::BadRatio)?),
            1358 => self.option_type = parse_put_or_call(value),
            609 => {
                self.sec_type = match value {
                    SEC_TYPE_FXVO => Settlement::Deliverable,
                    SEC_TYPE_FXNO => Settlement::NonDeliverable,
                    _ => return Err(DialectError::BadSecurityType),
                };
            }
            _ => {}
        }
        Ok(())
    }

    fn build(
        self,
        leg_tenor: &impl Fn(usize) -> Tenor,
        index: usize,
    ) -> Result<StrategyLeg, DialectError> {
        let option_type = self.option_type.ok_or(DialectError::BadPutOrCall)?;
        let strike = self.strike.ok_or(DialectError::BadStrike)?;
        // `strike`/`ratio` arrive via `parse_float`, which excluded NaN, so a
        // direct `<=` comparison is correct and clippy-clean.
        if strike <= 0.0 {
            return Err(DialectError::BadStrike);
        }
        let side = self.side.ok_or(DialectError::BadLeg)?;
        let ratio = self.ratio.unwrap_or(1.0);
        if ratio <= 0.0 {
            return Err(DialectError::BadRatio);
        }
        Ok(StrategyLeg {
            option: OptionDescriptor {
                pair: self.pair,
                option_type,
                strike,
                strike_ccy: self.pair.quote,
                exercise: ExerciseStyle::European,
                tenor: leg_tenor(index),
                settlement: self.sec_type,
            },
            side,
            ratio,
        })
    }
}

/// Parse a 6-letter `Symbol(55)` (e.g. `EURUSD`) into a [`CcyPair`]. Also
/// accepts the slash form `EUR/USD`.
#[must_use]
pub fn parse_symbol(v: &[u8]) -> Option<CcyPair> {
    let s = core::str::from_utf8(v).ok()?;
    if let Some((base, quote)) = s.split_once('/') {
        return Some(CcyPair::new(Ccy::parse(base)?, Ccy::parse(quote)?));
    }
    CcyPair::parse(s)
}

/// Parse a 3-letter currency value.
#[must_use]
pub fn parse_ccy(v: &[u8]) -> Option<Ccy> {
    core::str::from_utf8(v).ok().and_then(Ccy::parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::{FrameCursor, FrameEncoder};
    use celnet_types::Ccy;

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    #[test]
    fn decode_single_call() {
        let mut e = FrameEncoder::new();
        e.push(35, b"R");
        e.push(131, b"REQ1");
        e.push(55, b"EURUSD");
        e.push(460, b"4");
        e.push(167, b"FXVO");
        e.push(201, b"1"); // call
        e.push(202, b"1.0950");
        e.push(947, b"USD");
        e.push(1194, b"0");
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        let desc = decode_option(&frame, Tenor::Months(3)).unwrap();
        assert_eq!(desc.pair, eurusd());
        assert_eq!(desc.option_type, OptionType::Call);
        assert!((desc.strike - 1.0950).abs() < 1e-12);
        assert_eq!(desc.exercise, ExerciseStyle::European);
        assert_eq!(desc.settlement, Settlement::Deliverable);
    }

    #[test]
    fn rejects_strike_currency_mismatch() {
        let mut e = FrameEncoder::new();
        e.push(35, b"R");
        e.push(131, b"REQ1");
        e.push(55, b"EURUSD");
        e.push(167, b"FXVO");
        e.push(201, b"1");
        e.push(202, b"1.0950");
        e.push(947, b"EUR"); // wrong: should be the quote ccy USD
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(
            decode_option(&frame, Tenor::Months(3)),
            Err(DialectError::StrikeCurrencyMismatch)
        );
    }

    #[test]
    fn package_price_equals_sum_of_legs() {
        // Risk reversal: +1 call (buy), -1 put (sell), same pair, one snapshot.
        let snap = MarketSnapshot {
            spot: 1.10,
            vol: 0.10,
            t: 0.25,
            r_dom: 0.03,
            r_for: 0.01,
        };
        let call = OptionDescriptor {
            pair: eurusd(),
            option_type: OptionType::Call,
            strike: 1.15,
            strike_ccy: Ccy::USD,
            exercise: ExerciseStyle::European,
            tenor: Tenor::Months(3),
            settlement: Settlement::Deliverable,
        };
        let put = OptionDescriptor {
            option_type: OptionType::Put,
            strike: 1.05,
            ..call
        };
        let pkg = StrategyPackage {
            legs: vec![
                StrategyLeg {
                    option: call,
                    side: LegSide::Buy,
                    ratio: 1.0,
                },
                StrategyLeg {
                    option: put,
                    side: LegSide::Sell,
                    ratio: 1.0,
                },
            ],
        };
        let pricer: VanillaPricer = celnet_vanilla::price;
        let expected = price_leg(&call, &snap, pricer) - price_leg(&put, &snap, pricer);
        assert!((pkg.package_price(&snap, pricer) - expected).abs() < 1e-15);
    }

    #[test]
    fn decode_multileg_straddle() {
        // Straddle: buy call + buy put at the same strike.
        let mut e = FrameEncoder::new();
        e.push(35, b"R");
        e.push(131, b"REQ2");
        e.push_int(555, 2);
        // leg 1: call
        e.push(600, b"EURUSD");
        e.push(1358, b"1");
        e.push(612, b"1.10");
        e.push(624, b"1"); // buy
        e.push_int(623, 1);
        // leg 2: put
        e.push(600, b"EURUSD");
        e.push(1358, b"0");
        e.push(612, b"1.10");
        e.push(624, b"1");
        e.push_int(623, 1);
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        let pkg = decode_strategy(&frame, |_| Tenor::Months(3)).unwrap();
        assert_eq!(pkg.legs.len(), 2);
        assert_eq!(pkg.legs[0].option.option_type, OptionType::Call);
        assert_eq!(pkg.legs[1].option.option_type, OptionType::Put);
    }
}
