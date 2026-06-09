//! Celnet canonical domain types.
//!
//! This is the **frozen interface crate** at the base of the dependency graph
//! (see `docs/ROADMAP.md` §3): every other crate depends on it, so it is kept
//! pure (`Copy`/POD, no IO, no framework deps) and changed only via a deliberate
//! interface PR. It defines the FX-options *vocabulary* — currencies, pairs,
//! tenors, option type — and the **convention enums** that make Celnet
//! convention-correct rather than convention-defaulted, plus the data-transfer
//! objects (`VanillaInputs`, `Greeks`) shared across the pricing layers.

#![forbid(unsafe_code)]

use core::fmt;

use serde::{Deserialize, Serialize};

/// Call or put.
///
/// `sign()` yields the payoff sign (`+1` call, `-1` put) used throughout the
/// Garman-Kohlhagen formulae so a single code path serves both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OptionType {
    /// Right to buy the base currency.
    Call,
    /// Right to sell the base currency.
    Put,
}

impl OptionType {
    /// Payoff sign: `+1.0` for a call, `-1.0` for a put.
    #[must_use]
    pub const fn sign(self) -> f64 {
        match self {
            OptionType::Call => 1.0,
            OptionType::Put => -1.0,
        }
    }

    /// The opposite option type.
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            OptionType::Call => OptionType::Put,
            OptionType::Put => OptionType::Call,
        }
    }
}

/// An ISO-4217-style 3-letter currency code stored as uppercase ASCII bytes.
///
/// Compact (`Copy`, 3 bytes) so it never allocates on the hot path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ccy([u8; 3]);

impl Ccy {
    /// Construct from three ASCII bytes, upper-casing letters. Returns `None`
    /// unless all three are ASCII alphabetic.
    #[must_use]
    pub const fn new(code: [u8; 3]) -> Option<Self> {
        let mut out = [0u8; 3];
        let mut i = 0;
        while i < 3 {
            let b = code[i];
            if !b.is_ascii_alphabetic() {
                return None;
            }
            out[i] = b.to_ascii_uppercase();
            i += 1;
        }
        Some(Ccy(out))
    }

    /// Parse from a 3-character string slice.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 3 {
            return None;
        }
        Ccy::new([b[0], b[1], b[2]])
    }

    /// The code as an uppercase string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // The bytes are validated ASCII-alphabetic on every constructor
        // (`new`/`parse`) and the only field is private, so this conversion is
        // infallible by construction. We `expect` rather than substitute a
        // sentinel so any future invariant break fails loudly instead of
        // silently propagating a bogus currency code.
        core::str::from_utf8(&self.0).expect("Ccy bytes are validated ASCII on construction")
    }

    // Common majors, available as compile-time constants.
    /// US dollar.
    pub const USD: Ccy = Ccy([b'U', b'S', b'D']);
    /// Euro.
    pub const EUR: Ccy = Ccy([b'E', b'U', b'R']);
    /// Japanese yen.
    pub const JPY: Ccy = Ccy([b'J', b'P', b'Y']);
    /// Pound sterling.
    pub const GBP: Ccy = Ccy([b'G', b'B', b'P']);
    /// Swiss franc.
    pub const CHF: Ccy = Ccy([b'C', b'H', b'F']);
    /// Australian dollar.
    pub const AUD: Ccy = Ccy([b'A', b'U', b'D']);
    /// Canadian dollar.
    pub const CAD: Ccy = Ccy([b'C', b'A', b'D']);
    /// New Zealand dollar.
    pub const NZD: Ccy = Ccy([b'N', b'Z', b'D']);

    // Precious metals as ISO-4217 "X"-prefixed asset codes (the metal leg of a
    // [`MetalPair`] projects to one of these — XAU/XAG/XPT/XPD vs a fiat quote).
    /// Gold (one troy ounce), ISO-4217 XAU.
    pub const XAU: Ccy = Ccy([b'X', b'A', b'U']);
    /// Silver (one troy ounce), ISO-4217 XAG.
    pub const XAG: Ccy = Ccy([b'X', b'A', b'G']);
    /// Platinum (one troy ounce), ISO-4217 XPT.
    pub const XPT: Ccy = Ccy([b'X', b'P', b'T']);
    /// Palladium (one troy ounce), ISO-4217 XPD.
    pub const XPD: Ccy = Ccy([b'X', b'P', b'D']);
}

impl fmt::Display for Ccy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An FX currency pair `BASE/QUOTE` (market convention `CCY1CCY2`).
///
/// In Celnet terminology the **base** is the foreign/asset currency (FOR, CCY1)
/// and the **quote** is the domestic/numeraire currency (DOM, CCY2). For
/// `EURUSD`: base = EUR (FOR), quote = USD (DOM); the price is USD per 1 EUR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CcyPair {
    /// Foreign / asset currency (CCY1).
    pub base: Ccy,
    /// Domestic / numeraire currency (CCY2).
    pub quote: Ccy,
}

impl CcyPair {
    /// Construct a pair from base (FOR) and quote (DOM) currencies.
    #[must_use]
    pub const fn new(base: Ccy, quote: Ccy) -> Self {
        Self { base, quote }
    }

    /// Parse the 6-letter market form, e.g. `"EURUSD"`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 6 {
            return None;
        }
        Some(Self::new(Ccy::parse(&s[0..3])?, Ccy::parse(&s[3..6])?))
    }
}

impl fmt::Display for CcyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.base, self.quote)
    }
}

/// A precious metal traded as the asset (base) leg of a metal pair.
///
/// The four LBMA/LPPM precious metals. Each projects to an ISO-4217 "X"-prefixed
/// asset code ([`Ccy::XAU`]/[`Ccy::XAG`]/[`Ccy::XPT`]/[`Ccy::XPD`]) so a
/// [`MetalPair`] overlaps the FX [`CcyPair`] encoding byte-for-byte on the metal
/// leg. The metal is *always* the base/asset; the quote is a fiat numeraire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Metal {
    /// Gold (XAU).
    Gold,
    /// Silver (XAG).
    Silver,
    /// Platinum (XPT).
    Platinum,
    /// Palladium (XPD).
    Palladium,
}

impl Metal {
    /// The ISO-4217 "X"-prefixed asset code for this metal (XAU/XAG/XPT/XPD) as
    /// the base/asset [`Ccy`].
    #[must_use]
    pub const fn ccy(self) -> Ccy {
        match self {
            Metal::Gold => Ccy::XAU,
            Metal::Silver => Ccy::XAG,
            Metal::Platinum => Ccy::XPT,
            Metal::Palladium => Ccy::XPD,
        }
    }

    /// The metal for an ISO-4217 asset code, or `None` if the code is not one of
    /// the four precious-metal codes (XAU/XAG/XPT/XPD).
    #[must_use]
    pub const fn from_ccy(c: Ccy) -> Option<Metal> {
        // `Ccy` is not `const`-comparable with `==`, so match the raw bytes.
        match c.0 {
            [b'X', b'A', b'U'] => Some(Metal::Gold),
            [b'X', b'A', b'G'] => Some(Metal::Silver),
            [b'X', b'P', b'T'] => Some(Metal::Platinum),
            [b'X', b'P', b'D'] => Some(Metal::Palladium),
            _ => None,
        }
    }
}

impl fmt::Display for Metal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.ccy().as_str())
    }
}

/// A precious-metal pair: the [`Metal`] is the base/asset leg, traded against a
/// fiat `quote` (numeraire) currency, e.g. `XAUUSD`, `XAUEUR`, `XAGJPY`.
///
/// A metal pair projects losslessly to/from an FX-shaped [`CcyPair`] whose `base`
/// is the metal's ISO-4217 asset code — this keeps the `celnet-conventions` /
/// `celnet-calendar` registries (which key on [`CcyPair`]) untouched, and makes
/// the XAU/XAG/XPT/XPD-vs-fiat projection **byte-identical** to the FX path where
/// it overlaps (the W2 metal byte-identity invariant). The asset-class *naming*
/// (lease rate vs foreign rate) is a convention/risk-layer concern; the carry
/// math is the identical generalized forward/discount producer ([`Carry`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MetalPair {
    /// The precious metal (the base/asset leg).
    pub metal: Metal,
    /// The fiat quote (numeraire) currency, e.g. `USD`, `EUR`, `JPY`.
    pub quote: Ccy,
}

impl MetalPair {
    /// Construct a metal pair from its metal (base/asset) and fiat quote leg.
    #[must_use]
    pub const fn new(metal: Metal, quote: Ccy) -> Self {
        Self { metal, quote }
    }

    /// Project this metal pair into the FX-shaped [`CcyPair`] whose `base` is the
    /// metal's ISO-4217 asset code and whose `quote` is the fiat quote leg. The
    /// inverse of [`MetalPair::from_ccy_pair`] on a metal-base pair.
    #[must_use]
    pub const fn as_ccy_pair(self) -> CcyPair {
        CcyPair::new(self.metal.ccy(), self.quote)
    }

    /// Recover a metal pair from an FX-shaped [`CcyPair`] iff its `base` leg is a
    /// precious-metal asset code (XAU/XAG/XPT/XPD); else `None`. The inverse of
    /// [`MetalPair::as_ccy_pair`].
    #[must_use]
    pub const fn from_ccy_pair(p: CcyPair) -> Option<MetalPair> {
        match Metal::from_ccy(p.base) {
            Some(metal) => Some(MetalPair::new(metal, p.quote)),
            None => None,
        }
    }
}

impl fmt::Display for MetalPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.metal, self.quote)
    }
}

/// A free-form instrument ticker — the vendor-neutral identifier for an asset
/// not named by a currency-pair / metal-pair leg structure (an equity, a
/// commodity, a digital-asset coin).
///
/// Mirrors the wire `Symbol`. The `ticker` is the trading symbol (an
/// exchange/ISIN-style code); `venue` is the optional listing venue / exchange
/// MIC, empty when the ticker is globally unambiguous. Purpose-named only — no
/// vendor product names appear.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol {
    /// The trading symbol / ticker, e.g. `AAPL`, `BRENT`, an ISIN.
    pub ticker: String,
    /// The listing venue / exchange MIC, e.g. `XNAS`; empty when unambiguous.
    pub venue: String,
}

impl Symbol {
    /// Construct a symbol from its ticker and listing venue (pass an empty venue
    /// when the ticker is globally unambiguous).
    #[must_use]
    pub fn new(ticker: impl Into<String>, venue: impl Into<String>) -> Self {
        Self {
            ticker: ticker.into(),
            venue: venue.into(),
        }
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.venue.is_empty() {
            f.write_str(&self.ticker)
        } else {
            write!(f, "{}.{}", self.ticker, self.venue)
        }
    }
}

/// An equity (single-name or index) underlying — mirrors the wire `EquityRef`.
///
/// The `symbol` names the listed instrument; `currency` is the trading /
/// settlement currency the price is quoted in. The dividend treatment is a
/// carry-layer concern ([`Carry::CostOfCarry`]), not encoded here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EquityRef {
    /// The listed equity symbol (single name or index).
    pub symbol: Symbol,
    /// The currency the equity is quoted / settled in.
    pub currency: Ccy,
}

impl EquityRef {
    /// Construct an equity reference from its symbol and quote/settlement
    /// currency.
    #[must_use]
    pub const fn new(symbol: Symbol, currency: Ccy) -> Self {
        Self { symbol, currency }
    }
}

impl fmt::Display for EquityRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.symbol, self.currency)
    }
}

/// A commodity underlying (a futures-style or spot commodity) — mirrors the wire
/// `CommodityRef`.
///
/// The `symbol` names the commodity / contract; `currency` is the quote /
/// settlement currency. The cost-of-carry (storage / convenience yield) is a
/// carry-layer concern ([`Carry::CostOfCarry`]), not encoded here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommodityRef {
    /// The commodity / contract symbol, e.g. `BRENT`.
    pub symbol: Symbol,
    /// The currency the commodity is quoted / settled in.
    pub currency: Ccy,
}

impl CommodityRef {
    /// Construct a commodity reference from its symbol and quote/settlement
    /// currency.
    #[must_use]
    pub const fn new(symbol: Symbol, currency: Ccy) -> Self {
        Self { symbol, currency }
    }
}

impl fmt::Display for CommodityRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.symbol, self.currency)
    }
}

/// A digital-asset (crypto) pair — mirrors the wire `CryptoPair`.
///
/// The `base` is the coin/asset leg (e.g. `BTC`); the `quote` is the numeraire
/// it is priced against, which may be a fiat currency (`USD`) or another coin /
/// stablecoin (`USDT`). Both legs are free-form strings — crypto tickers are not
/// constrained to the 3-letter ISO-4217 shape, so they are not [`Ccy`] newtypes.
/// The linear/inverse settlement of a coin-margined contract is carried on the
/// instrument's [`SettlementStyle`], not here (it is a contract-mechanics
/// convention, not part of the pair identity).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CryptoPair {
    /// The coin/asset (base) leg, e.g. `BTC`, `ETH`.
    pub base: String,
    /// The numeraire (quote) leg — fiat (`USD`) or coin/stablecoin (`USDT`).
    pub quote: String,
}

impl CryptoPair {
    /// Construct a crypto pair from its coin (base) leg and numeraire (quote)
    /// leg.
    #[must_use]
    pub fn new(base: impl Into<String>, quote: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            quote: quote.into(),
        }
    }
}

impl fmt::Display for CryptoPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.base, self.quote)
    }
}

/// The instrument's underlying — the asset-class discriminator that lets one
/// unversioned contract name FX, precious metals, equities, commodities and
/// digital assets (and, in later waves, further classes).
///
/// This type answers only *what is the underlying*; pricing carry and settlement
/// specifics live in [`Carry`] and the per-arm references. W1 shipped the FX arm
/// (the platform's origin asset class); W2 adds the [`Metal`] arm; the cross-asset
/// wave adds the [`EquityRef`], [`CommodityRef`] and [`CryptoPair`] arms. Further
/// arms are added by their asset-class wave as additive enum growth (one current
/// contract — no versioning, no placeholder arms).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Underlying {
    /// An FX currency pair (e.g. `EURUSD`).
    Fx(CcyPair),
    /// A precious-metal pair (metal vs fiat quote, e.g. `XAUUSD`).
    Metal(MetalPair),
    /// An equity (single-name or index) underlying.
    Equity(EquityRef),
    /// A commodity underlying.
    Commodity(CommodityRef),
    /// A digital-asset (crypto) pair.
    DigitalAsset(CryptoPair),
}

impl Underlying {
    /// The FX pair if this underlying is FX, else `None`.
    #[must_use]
    pub const fn as_fx(&self) -> Option<CcyPair> {
        match self {
            Underlying::Fx(p) => Some(*p),
            Underlying::Metal(_)
            | Underlying::Equity(_)
            | Underlying::Commodity(_)
            | Underlying::DigitalAsset(_) => None,
        }
    }

    /// The metal pair if this underlying is a precious metal, else `None`.
    #[must_use]
    pub const fn as_metal(&self) -> Option<MetalPair> {
        match self {
            Underlying::Metal(m) => Some(*m),
            Underlying::Fx(_)
            | Underlying::Equity(_)
            | Underlying::Commodity(_)
            | Underlying::DigitalAsset(_) => None,
        }
    }

    /// The equity reference if this underlying is an equity, else `None`.
    #[must_use]
    pub fn as_equity(&self) -> Option<&EquityRef> {
        match self {
            Underlying::Equity(e) => Some(e),
            Underlying::Fx(_)
            | Underlying::Metal(_)
            | Underlying::Commodity(_)
            | Underlying::DigitalAsset(_) => None,
        }
    }

    /// The commodity reference if this underlying is a commodity, else `None`.
    #[must_use]
    pub fn as_commodity(&self) -> Option<&CommodityRef> {
        match self {
            Underlying::Commodity(c) => Some(c),
            Underlying::Fx(_)
            | Underlying::Metal(_)
            | Underlying::Equity(_)
            | Underlying::DigitalAsset(_) => None,
        }
    }

    /// The crypto pair if this underlying is a digital asset, else `None`.
    #[must_use]
    pub fn as_digital_asset(&self) -> Option<&CryptoPair> {
        match self {
            Underlying::DigitalAsset(p) => Some(p),
            Underlying::Fx(_)
            | Underlying::Metal(_)
            | Underlying::Equity(_)
            | Underlying::Commodity(_) => None,
        }
    }

    /// Project the underlying to the FX-shaped [`CcyPair`] the convention /
    /// calendar registries key on, *if* the underlying is one of the leg-pair
    /// arms (FX, or a metal's metal-base projection). The cross-asset arms
    /// (equity / commodity / digital-asset) have no `CcyPair` projection — they
    /// are not keyed by the FX-pair-shaped registries — so they return `None`
    /// rather than a coerced pair. This is the bridge that keeps those registries
    /// [`CcyPair`]-keyed while the wire/identity layer is asset-tagged.
    #[must_use]
    pub const fn as_ccy_pair(&self) -> Option<CcyPair> {
        match self {
            Underlying::Fx(p) => Some(*p),
            Underlying::Metal(m) => Some(m.as_ccy_pair()),
            Underlying::Equity(_) | Underlying::Commodity(_) | Underlying::DigitalAsset(_) => None,
        }
    }
}

impl From<CcyPair> for Underlying {
    fn from(p: CcyPair) -> Self {
        Underlying::Fx(p)
    }
}

impl From<MetalPair> for Underlying {
    fn from(m: MetalPair) -> Self {
        Underlying::Metal(m)
    }
}

impl From<EquityRef> for Underlying {
    fn from(e: EquityRef) -> Self {
        Underlying::Equity(e)
    }
}

impl From<CommodityRef> for Underlying {
    fn from(c: CommodityRef) -> Self {
        Underlying::Commodity(c)
    }
}

impl From<CryptoPair> for Underlying {
    fn from(p: CryptoPair) -> Self {
        Underlying::DigitalAsset(p)
    }
}

impl fmt::Display for Underlying {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Underlying::Fx(p) => write!(f, "{p}"),
            Underlying::Metal(m) => write!(f, "{m}"),
            Underlying::Equity(e) => write!(f, "{e}"),
            Underlying::Commodity(c) => write!(f, "{c}"),
            Underlying::DigitalAsset(p) => write!(f, "{p}"),
        }
    }
}

/// A civil (Gregorian) calendar date, as a self-contained POD triple.
///
/// `celnet-types` is the dependency-graph root and stays free of the `time`
/// crate, so a *date-valued* tenor ([`Tenor::BrokenDate`]) carries its date as
/// this small `Copy`/`Hash`/serializable triple. `celnet-calendar` converts it
/// to a `time::Date` (validating it) when resolving the date chain; the wire
/// (`celnet-proto`) mirrors it as three scalar fields. A broken date is an
/// **explicit expiry date** — the odd/off-the-ladder expiry that, per
/// `docs/TRADING-UNIVERSE-SCALE.md` §2.2, carries the *majority* of real FX
/// flow — not a count of standard units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BrokenDate {
    /// Gregorian year (e.g. 2026).
    pub year: i32,
    /// Month of year, 1 (January) .. 12 (December).
    pub month: u8,
    /// Day of month, 1 .. 31 (validated against the month on resolution).
    pub day: u8,
}

impl BrokenDate {
    /// Construct a broken date from its civil components. The range of `month`
    /// and `day` is *not* validated here (no calendar knowledge at this layer);
    /// `celnet-calendar` validates the triple against the Gregorian calendar
    /// when it resolves the expiry, rejecting an impossible date rather than
    /// silently clamping it.
    #[must_use]
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }
}

/// A standard FX-options tenor.
///
/// Most tenors are measured from the **spot** date (`Weeks`/`Months`/`Years`),
/// but the short end is anchored on *today* (the trade/horizon date) and the
/// off-the-ladder cases carry an explicit date or an exchange-defined date:
///
/// - [`Tenor::Overnight`] (ON), [`Tenor::TomNext`] (TN) and [`Tenor::SpotNext`]
///   (SN) are the **pre-spot short end**, each anchored distinctly on the
///   horizon→spot chain (see `celnet-calendar`'s `expiry_for_tenor`). ON is the
///   next good business day after *today*; TN the one after that; SN the next
///   good day after *spot*.
/// - [`Tenor::Imm`] selects an **IMM expiry** (3rd Wednesday of Mar/Jun/Sep/Dec),
///   the `n`-th such date strictly after today (`n = 1` ⇒ the next one).
/// - [`Tenor::BrokenDate`] is an **explicit expiry date** (a broken/odd date) —
///   the dominant real-flow case for FX (TRADING-UNIVERSE-SCALE §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tenor {
    /// Overnight (ON): the next good business day after **today** (the horizon),
    /// i.e. a ~T+1 expiry. (This is the short end anchored on the horizon, *not*
    /// on spot — resolving it relative to spot would wrongly push ON out to the
    /// spot-next region.)
    Overnight,
    /// Tomorrow-next (TN): the good business day **after** the overnight date,
    /// i.e. the day between the overnight and the spot leg.
    TomNext,
    /// Spot-next (SN): the next good business day **after the spot date**.
    SpotNext,
    /// `n` calendar weeks from spot.
    Weeks(u16),
    /// `n` calendar months from spot (end-of-month-aware).
    Months(u16),
    /// `n` calendar years from spot (end-of-month-aware).
    Years(u16),
    /// The `n`-th IMM expiry strictly after today: the 3rd Wednesday of the next
    /// March / June / September / December cycle (`n = 1` ⇒ the next IMM date;
    /// `n = 2` ⇒ the one after, etc.). `n = 0` is invalid.
    Imm(u8),
    /// An explicit broken (odd) expiry **date**, resolved directly to that date
    /// (validated, then roll-adjusted only if it falls on a non-business day).
    BrokenDate(BrokenDate),
}

/// The smile/surface calibration model a mark or scenario is computed under.
///
/// This is the canonical, **vendor- and method-neutral** selector mirrored by
/// `celnet-proto` (wire) and consumed by `celnet-surface`'s calibration. The
/// names describe the *purpose* of each model (guardrail #8); the mathematical
/// provenance of each (the vanna-volga market-hedge interpolation, the SABR
/// stochastic-vol expansion, and the SVI / SSVI parametric families) lives in
/// doc comments only, never in identifiers. The default selection (absent on the
/// wire) preserves the current calibration behaviour ([`SmileModel::MarketHedge`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmileModel {
    /// Market-hedge interpolation off the broker pillars (the FX broker
    /// baseline; vanna-volga method). The default.
    MarketHedge,
    /// Stochastic-volatility smile with arbitrage-free wing density (SABR method).
    StochasticVol,
    /// Single parametric total-variance slice (SVI method).
    Parametric,
    /// Surface-level parametric family, closed-form arbitrage-free (SSVI method).
    ParametricSurface,
    /// Extended surface-level parametric family with maturity-dependent
    /// correlation, closed-form arbitrage-free (eSSVI method).
    ExtendedSurface,
}

impl Default for SmileModel {
    /// The default calibration is the market-hedge interpolation, preserving the
    /// behaviour of a mark/scenario request that does not select a model.
    fn default() -> Self {
        Self::MarketHedge
    }
}

/// Quoted delta convention for a `(pair, tenor)`.
///
/// FX has four because delta is taken on spot or forward, and adjusted for the
/// premium when the premium is paid in the base/foreign currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeltaConvention {
    /// Spot delta, premium-unadjusted: `e^{-r_f T} N(d1)`.
    SpotUnadjusted,
    /// Forward (driftless) delta, premium-unadjusted: `N(d1)`.
    ForwardUnadjusted,
    /// Spot delta, premium-adjusted: `e^{-r_f T} (K/F) N(d2)`.
    SpotPremiumAdjusted,
    /// Forward delta, premium-adjusted: `(K/F) N(d2)`.
    ForwardPremiumAdjusted,
}

/// At-the-money strike convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AtmConvention {
    /// ATM-forward: `K = F`.
    AtmForward,
    /// Delta-neutral straddle: strike where call delta + put delta = 0.
    DeltaNeutralStraddle,
}

/// Premium quotation style (which currency the premium is paid in, and units).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PremiumStyle {
    /// Domestic pips (quote-ccy per 1 unit of base notional).
    DomesticPips,
    /// Percentage of foreign (base) notional.
    PercentForeign,
    /// Percentage of domestic (quote) notional.
    PercentDomestic,
    /// Foreign pips.
    ForeignPips,
}

impl PremiumStyle {
    /// Whether the premium is paid in the foreign (base) currency and therefore
    /// carries FX risk, requiring a premium-adjusted delta.
    #[must_use]
    pub const fn is_premium_adjusted(self) -> bool {
        matches!(
            self,
            PremiumStyle::PercentForeign | PremiumStyle::ForeignPips
        )
    }

    /// The premium style for the **inverted pair orientation** (base↔quote
    /// swapped).
    ///
    /// The premium is always paid in the *same physical currency*, but inverting
    /// the pair swaps that currency's role between foreign (base) and domestic
    /// (quote). A premium paid in `EURUSD`'s base (EUR) is `PercentForeign`
    /// (premium-adjusted); quoting `USDEUR` makes EUR the *quote*, so the same
    /// premium is now `PercentDomestic` (premium-unadjusted). This is the
    /// orientation transform required when a covered profile is consulted for the
    /// flipped pair, otherwise the premium-adjusted flag is systematically wrong
    /// for inverted majors.
    #[must_use]
    pub const fn flip_orientation(self) -> PremiumStyle {
        match self {
            PremiumStyle::DomesticPips => PremiumStyle::ForeignPips,
            PremiumStyle::PercentForeign => PremiumStyle::PercentDomestic,
            PremiumStyle::PercentDomestic => PremiumStyle::PercentForeign,
            PremiumStyle::ForeignPips => PremiumStyle::DomesticPips,
        }
    }
}

/// Expiry cut (fixing time) convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cut {
    /// New York 10:00 — the standard interbank OTC cut for most pairs.
    NewYork1000,
    /// Tokyo 15:00 — standard for JPY-region / Asian business.
    Tokyo1500,
}

/// Day-count basis used to convert dates to year fractions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DayCount {
    /// Actual/365 fixed — typical for option vol-time.
    Act365Fixed,
    /// Actual/360 — typical money-market accrual (USD/EUR).
    Act360,
}

/// Settlement style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Settlement {
    /// Physically deliverable: both currencies exchanged on delivery.
    Deliverable,
    /// Non-deliverable option: cash-settled at a published fixing.
    NonDeliverable,
}

/// Contract settlement mechanics — how a contract's PnL is denominated and
/// margined. Mirrors the wire `SettlementStyle`.
///
/// The meaningful-zero convention (matching the other vocabulary enums): the
/// [`SettlementStyle::Linear`] style is the canonical default, so an
/// instrument that does not assert a settlement style is the ordinary
/// quote-currency-margined linear contract — byte-identical to the contract
/// before this dimension existed. [`SettlementStyle::InverseCoin`] is the
/// coin-margined (digital-asset) convention where PnL accrues in the base coin
/// against a `1/S_T` payoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SettlementStyle {
    /// Linear (quote-currency-margined): the ordinary contract whose payoff and
    /// margin are in the quote / numeraire currency. The default for every asset
    /// class shipped before the digital-asset arm.
    #[default]
    Linear,
    /// Inverse, coin-margined: the digital-asset convention whose payoff is the
    /// `1/S_T` (base-coin-denominated) form and whose margin accrues in the base
    /// coin. Meaningful only for a `DigitalAsset` underlying.
    InverseCoin,
}

/// The published reference fixing a non-deliverable FX option (NDO) or NDF
/// cash-settles against on the fixing date.
///
/// A non-deliverable option pays out the difference between its strike and a
/// **published official fixing** for the restricted currency, settled in the
/// convertible currency (USD). The fixing source is a market-data identity, not
/// a model input: it names *which* published rate the contract references (the
/// EMTA/ISDA template names each one). This enum is the vendor-neutral catalogue
/// of the fixings the convention registry resolves; the *live values* of these
/// fixings remain an estate-gated market-data feed and are never sourced from
/// this repository — only the convention identity is encoded here.
///
/// References: EMTA (Emerging Markets Trade Association) template terms and the
/// 2005/2018 ISDA FX/Currency Option definitions per-currency matrices, which
/// name the settlement-rate option for each restricted currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FixingSource {
    /// Korea — KFTC18 (the MAS/KFTC USD/KRW spot rate, ~15:30 Seoul) per the
    /// EMTA "KRW KFTC18" settlement-rate option.
    KrwKftc18,
    /// Taiwan — the Taipei Forex / TFEMA USD/TWD fixing (EMTA "TWD Taipei").
    TwdTaipei,
    /// India — the Reserve Bank of India USD/INR reference rate (EMTA "INR RBIB",
    /// the RBI 12:30 reference rate).
    InrRbiRef,
    /// Brazil — the BCB PTAX USD/BRL rate (EMTA "BRL PTAX", BRL09).
    BrlPtax,
    /// Chile — the "Dólar Observado" USD/CLP rate published by the Banco Central
    /// de Chile (EMTA "CLP Dolar Observado", CLP10).
    ClpDolarObs,
    /// Colombia — the TRM (Tasa Representativa del Mercado) USD/COP rate (EMTA
    /// "COP TRM", COP04).
    CopTrm,
}

impl FixingSource {
    /// A short, stable identifier for the fixing (the EMTA-style code), suitable
    /// for logging and reconciliation. Vendor-neutral and method-neutral.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            FixingSource::KrwKftc18 => "KRW.KFTC18",
            FixingSource::TwdTaipei => "TWD.TAIPEI",
            FixingSource::InrRbiRef => "INR.RBIB",
            FixingSource::BrlPtax => "BRL.PTAX",
            FixingSource::ClpDolarObs => "CLP.DOLAROBS",
            FixingSource::CopTrm => "COP.TRM",
        }
    }
}

/// Annualized Black volatility (absolute; `0.10` = 10 vol), a transparent `f64`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Vol(pub f64);

/// A strike (quote currency per 1 unit of base), a transparent `f64`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Strike(pub f64);

/// A continuously-compounded interest rate, a transparent `f64`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Rate(pub f64);

/// An option delta (signed; `−1..=1` for unadjusted), a transparent `f64`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Delta(pub f64);

/// A discount factor `e^{-r·t} ∈ (0, 1]`, a transparent `f64`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Df(pub f64);

/// A time/year-fraction (years), a transparent `f64`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Time(pub f64);

/// Inputs to the Garman-Kohlhagen vanilla model.
///
/// Continuous rates are used so the two rhos and the carry are unambiguous; the
/// forward and the two discount factors are derived (`forward`, `df_dom`,
/// `df_for`). Times are year fractions on the vol-time day-count.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VanillaInputs {
    /// Spot FX rate (quote per 1 unit of base).
    pub spot: f64,
    /// Strike (quote per 1 unit of base).
    pub strike: f64,
    /// Annualized volatility (absolute, e.g. `0.10` = 10 vol).
    pub vol: f64,
    /// Time to expiry in years (vol-time).
    pub t: f64,
    /// Continuously-compounded domestic (quote) interest rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) interest rate.
    pub r_for: f64,
}

impl VanillaInputs {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            r_dom,
            r_for,
        }
    }

    /// Domestic discount factor `e^{-r_dom · t}`.
    #[must_use]
    pub fn df_dom(&self) -> f64 {
        libm::exp(-self.r_dom * self.t)
    }

    /// Foreign discount factor `e^{-r_for · t}`.
    #[must_use]
    pub fn df_for(&self) -> f64 {
        libm::exp(-self.r_for * self.t)
    }

    /// Outright forward `F = S · e^{(r_dom − r_for)·t}`.
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.spot * libm::exp((self.r_dom - self.r_for) * self.t)
    }
}

/// The cost-of-carry model behind an option's forward and discounting.
///
/// FX carries two continuously-compounded rates (`r_dom`, `r_for`); other asset
/// classes carry a single discount rate `r` and a net carry `b` (equity
/// `b = r − q`; commodity `b = r − convenience`; digital-asset `b = r − funding`).
/// Both reduce to the generalized-Black-Scholes forward `F = S·e^{b·t}` and
/// discount `e^{−r·t}` — for FX, `r = r_dom` and `b = r_dom − r_for`.
///
/// [`Carry::FxRates`] preserves the *exact* FX two-rate arithmetic
/// (`forward_factor`/`discount_df` reproduce [`VanillaInputs::forward`]/
/// [`VanillaInputs::df_dom`] bit-for-bit); [`Carry::CostOfCarry`] serves the
/// other asset classes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Carry {
    /// FX two-rate carry: domestic (quote) `r_dom`, foreign (base) `r_for`.
    FxRates {
        /// Continuously-compounded domestic (quote) rate — the discount rate.
        r_dom: f64,
        /// Continuously-compounded foreign (base) rate.
        r_for: f64,
    },
    /// Generalized cost-of-carry: discount rate `r`, net carry `b`.
    CostOfCarry {
        /// Continuously-compounded discount (numeraire) rate `r`.
        r: f64,
        /// Net cost-of-carry `b` in `F = S·e^{b·t}`.
        b: f64,
    },
}

impl Carry {
    /// The discount (numeraire) rate `r` for `e^{−r·t}`. For FX this is `r_dom`.
    #[must_use]
    pub const fn discount_rate(&self) -> f64 {
        match self {
            Carry::FxRates { r_dom, .. } => *r_dom,
            Carry::CostOfCarry { r, .. } => *r,
        }
    }

    /// The net carry `b` in `F = S·e^{b·t}`. For FX this is `r_dom − r_for`.
    #[must_use]
    pub fn carry_rate(&self) -> f64 {
        match self {
            Carry::FxRates { r_dom, r_for } => r_dom - r_for,
            Carry::CostOfCarry { b, .. } => *b,
        }
    }

    /// Discount factor `e^{−r·t}`.
    #[must_use]
    pub fn discount_df(&self, t: f64) -> f64 {
        libm::exp(-self.discount_rate() * t)
    }

    /// Outright forward factor `e^{b·t}` (multiply by spot for `F`).
    #[must_use]
    pub fn forward_factor(&self, t: f64) -> f64 {
        libm::exp(self.carry_rate() * t)
    }

    /// The yield/foreign rate `q` such that discount `= e^{−r·t}`, growth `= e^{−q·t}`,
    /// and net carry `b = r − q`.
    ///
    /// For [`Carry::FxRates`] this is the **stored** `r_for`, returned *verbatim* —
    /// never reconstructed as `discount_rate() − carry_rate()` (`r_dom − (r_dom − r_for)`
    /// does not in general round-trip bit-for-bit). Reading the stored field keeps every
    /// foreign-discount site (`e^{−r_for·t}`) byte-identical to the FX two-rate form.
    /// For [`Carry::CostOfCarry`] it is `r − b`.
    #[must_use]
    pub fn yield_rate(&self) -> f64 {
        match self {
            Carry::FxRates { r_for, .. } => *r_for,
            Carry::CostOfCarry { r, b } => r - b,
        }
    }
}

/// The full FX-options Greek set produced by the vanilla engine.
///
/// All sensitivities are *raw* (per unit of the underlying quantity): vega and
/// volga are per `1.0` of absolute vol (divide by 100 for "per vol point");
/// theta is `∂V/∂t` per year (time decay; `−∂V/∂T`); rhos are per `1.0` of
/// continuously-compounded rate. Delta is reported in both spot and forward
/// (premium-unadjusted) conventions; convention-specific deltas are derived in
/// `celnet-vanilla` from the configured `DeltaConvention`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Greeks {
    /// Present value (premium) in domestic currency, per 1 unit of base notional.
    pub price: f64,
    /// Spot delta (premium-unadjusted): `∂V/∂S` scaled appropriately.
    pub delta_spot: f64,
    /// Forward delta (premium-unadjusted).
    pub delta_forward: f64,
    /// Gamma: `∂²V/∂S²`.
    pub gamma: f64,
    /// Vega: `∂V/∂σ` (per `1.0` absolute vol).
    pub vega: f64,
    /// Theta: `∂V/∂t` per year (`−∂V/∂T`).
    pub theta: f64,
    /// Rho domestic: `∂V/∂r_dom`.
    pub rho_dom: f64,
    /// Rho foreign: `∂V/∂r_for`.
    pub rho_for: f64,
    /// Vanna: `∂²V/∂S∂σ` (= `∂delta_spot/∂σ`).
    pub vanna: f64,
    /// Volga / vomma: `∂²V/∂σ²`.
    pub volga: f64,
    /// Charm: `∂(delta_spot)/∂T` (delta decay, per year).
    pub charm: f64,
    /// Speed: `∂³V/∂S³` (= `∂gamma/∂S`).
    pub speed: f64,
    /// Zomma: `∂gamma/∂σ`.
    pub zomma: f64,
    /// Color: `∂gamma/∂T`.
    pub color: f64,
}

impl Greeks {
    /// A price-only Greek set: the present value with every sensitivity zeroed.
    ///
    /// Used by products whose sensitivity strip is a distinct, larger increment
    /// not yet computed — e.g. correlated multi-asset baskets, whose Greeks are a
    /// per-leg `N × {spot, vol}` Jacobian plus cross-gammas. Returning an honest
    /// zero strip alongside the price (and the Monte-Carlo standard error)
    /// signals the deferral explicitly, rather than fabricating a
    /// single-underlying bump that would be silently wrong for a multi-asset
    /// payoff.
    #[must_use]
    pub const fn price_only(price: f64) -> Self {
        Self {
            price,
            delta_spot: 0.0,
            delta_forward: 0.0,
            gamma: 0.0,
            vega: 0.0,
            theta: 0.0,
            rho_dom: 0.0,
            rho_for: 0.0,
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        }
    }
}

/// Carry-tagged rate sensitivities — the asset-class-appropriate rate Greeks.
///
/// FX reports the two rate rhos (`rho_dom`, `rho_for`); other asset classes report
/// a discount-rho `∂V/∂r` and a carry-rho `∂V/∂b` (equity dividend-rho, commodity
/// carry-rho). The two are related for FX by `rho_dom = discount_rho + carry_rho`
/// and `rho_for = −carry_rho` (since `r = r_dom`, `b = r_dom − r_for`). The
/// [`RateSensitivities::Fx`] arm is exactly today's two flat rhos (byte-identical).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RateSensitivities {
    /// FX: `∂V/∂r_dom`, `∂V/∂r_for`.
    Fx {
        /// Rho domestic: `∂V/∂r_dom`.
        rho_dom: f64,
        /// Rho foreign: `∂V/∂r_for`.
        rho_for: f64,
    },
    /// Generalized: `∂V/∂r` (discount), `∂V/∂b` (carry).
    Carry {
        /// Discount rho: `∂V/∂r`.
        discount_rho: f64,
        /// Carry rho: `∂V/∂b` (equity dividend-rho, commodity carry-rho).
        carry_rho: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ccy_parse_and_display() {
        assert_eq!(Ccy::parse("eur").unwrap(), Ccy::EUR);
        assert_eq!(Ccy::EUR.as_str(), "EUR");
        assert!(Ccy::parse("EU1").is_none());
        assert!(Ccy::parse("EURO").is_none());
    }

    #[test]
    fn pair_parse() {
        let p = CcyPair::parse("EURUSD").unwrap();
        assert_eq!(p.base, Ccy::EUR);
        assert_eq!(p.quote, Ccy::USD);
        assert_eq!(p.to_string(), "EURUSD");
        assert!(CcyPair::parse("EURUS").is_none());
    }

    #[test]
    fn option_sign_and_flip() {
        assert_eq!(OptionType::Call.sign(), 1.0);
        assert_eq!(OptionType::Put.sign(), -1.0);
        assert_eq!(OptionType::Call.flip(), OptionType::Put);
    }

    #[test]
    fn underlying_fx_roundtrip() {
        let p = CcyPair::parse("EURUSD").unwrap();
        let u: Underlying = p.into();
        assert_eq!(u.as_fx(), Some(p));
        assert_eq!(u.as_metal(), None);
        assert_eq!(u.as_ccy_pair(), Some(p));
        assert_eq!(u.to_string(), "EURUSD");
        assert_eq!(u, Underlying::Fx(p));
    }

    #[test]
    fn metal_ccy_codes_are_iso_x_prefixed() {
        assert_eq!(Metal::Gold.ccy(), Ccy::XAU);
        assert_eq!(Metal::Silver.ccy(), Ccy::XAG);
        assert_eq!(Metal::Platinum.ccy(), Ccy::XPT);
        assert_eq!(Metal::Palladium.ccy(), Ccy::XPD);
        assert_eq!(Metal::Gold.to_string(), "XAU");
        // Round-trip every metal through its asset code.
        for m in [
            Metal::Gold,
            Metal::Silver,
            Metal::Platinum,
            Metal::Palladium,
        ] {
            assert_eq!(Metal::from_ccy(m.ccy()), Some(m));
        }
        // A fiat code is not a metal.
        assert_eq!(Metal::from_ccy(Ccy::USD), None);
        assert_eq!(Metal::from_ccy(Ccy::EUR), None);
    }

    #[test]
    fn metal_pair_projects_to_metal_base_ccy_pair() {
        let mp = MetalPair::new(Metal::Gold, Ccy::USD);
        let cp = mp.as_ccy_pair();
        assert_eq!(cp.base, Ccy::XAU);
        assert_eq!(cp.quote, Ccy::USD);
        assert_eq!(cp.to_string(), "XAUUSD");
        assert_eq!(mp.to_string(), "XAUUSD");
        // Round-trip CcyPair -> MetalPair -> CcyPair.
        assert_eq!(MetalPair::from_ccy_pair(cp), Some(mp));
        // A non-metal-base pair is not a metal pair.
        assert_eq!(
            MetalPair::from_ccy_pair(CcyPair::new(Ccy::EUR, Ccy::USD)),
            None
        );
        // A metal cross (XAUEUR, XAGJPY).
        let cross = MetalPair::new(Metal::Gold, Ccy::EUR);
        assert_eq!(cross.as_ccy_pair().to_string(), "XAUEUR");
    }

    #[test]
    fn underlying_metal_roundtrip() {
        let mp = MetalPair::new(Metal::Platinum, Ccy::USD);
        let u: Underlying = mp.into();
        assert_eq!(u.as_metal(), Some(mp));
        assert_eq!(u.as_fx(), None);
        assert_eq!(u.to_string(), "XPTUSD");
        assert_eq!(u, Underlying::Metal(mp));
        // The CcyPair projection is the metal-base pair the registries key on.
        assert_eq!(u.as_ccy_pair(), Some(CcyPair::new(Ccy::XPT, Ccy::USD)));
    }

    // The metal XAUUSD projection's forward and discount factors are byte-for-byte
    // identical to the FX path over the SAME (spot, r_dom = quote rate, r_for =
    // lease rate) inputs — the W2 metal byte-identity invariant. The metal lease
    // rate is modelled as the FX foreign rate (per `InstrumentClass::PreciousMetal`
    // and ADR-0008): identical operations, identical order, so `to_bits` matches.
    #[test]
    fn metal_carry_byte_identical_to_fx_path() {
        // (spot, quote/discount rate r_dom, lease rate r_for, t)
        for &(spot, r_dom, r_for, t) in &[
            (1850.0, 0.045, 0.012, 1.0),
            (24.5, 0.05, 0.0, 0.25),
            (980.0, -0.004, 0.031, 2.5),
        ] {
            // FX path (the reference): a plain CcyPair vanilla input.
            let fx = VanillaInputs::new(spot, 1.0, 0.1, t, r_dom, r_for);
            // Metal path: identical carry assembly via the generalized producer,
            // reached through the Underlying::Metal projection's CcyPair.
            let mp = MetalPair::new(Metal::Gold, Ccy::USD);
            let u = Underlying::Metal(mp);
            // The projected pair the registry/pricer keys on must be metal-base.
            assert_eq!(u.as_ccy_pair(), Some(CcyPair::new(Ccy::XAU, Ccy::USD)));
            let carry = Carry::FxRates { r_dom, r_for };
            assert_eq!(
                (spot * carry.forward_factor(t)).to_bits(),
                fx.forward().to_bits(),
                "metal forward must be byte-identical to the FX path"
            );
            assert_eq!(
                carry.discount_df(t).to_bits(),
                fx.df_dom().to_bits(),
                "metal discount must be byte-identical to the FX path"
            );
        }
    }

    // The FX projection of the generalized `Carry` reproduces the FX two-rate
    // forward and discount factor BIT-FOR-BIT — the W1 no-regression invariant
    // (same operations, same order, so `to_bits` must match exactly).
    #[test]
    fn carry_fxrates_byte_identical_to_vanilla_inputs() {
        for &(spot, r_dom, r_for, t) in &[
            (1.10, 0.02, 0.015, 1.0),
            (100.0, 0.05, 0.0, 0.25),
            (0.85, -0.004, 0.031, 2.5),
        ] {
            let i = VanillaInputs::new(spot, 1.0, 0.1, t, r_dom, r_for);
            let carry = Carry::FxRates { r_dom, r_for };
            assert_eq!(
                (spot * carry.forward_factor(t)).to_bits(),
                i.forward().to_bits(),
                "forward must be byte-identical via Carry::FxRates"
            );
            assert_eq!(
                carry.discount_df(t).to_bits(),
                i.df_dom().to_bits(),
                "domestic discount must be byte-identical via Carry::FxRates"
            );
            assert_eq!(carry.discount_rate(), r_dom);
            assert_eq!(carry.carry_rate(), r_dom - r_for);
            // The yield/foreign rate must be the STORED `r_for`, bit-for-bit — NOT the
            // `discount_rate() − carry_rate()` reconstruction, which would not round-trip.
            assert_eq!(
                carry.yield_rate().to_bits(),
                r_for.to_bits(),
                "yield_rate must read the stored r_for verbatim (byte-identical)"
            );
        }
    }

    #[test]
    fn yield_rate_cost_of_carry() {
        let (r, q) = (0.04, 0.03);
        let carry = Carry::CostOfCarry { r, b: r - q };
        assert_eq!(carry.yield_rate(), r - (r - q));
    }

    #[test]
    fn carry_cost_of_carry_generalized() {
        // Equity with dividend yield q ⇒ r = 0.04, b = r − q = 0.04 − 0.03.
        let (r, b, t, spot) = (0.04, 0.04 - 0.03, 1.0, 50.0);
        let carry = Carry::CostOfCarry { r, b };
        assert_eq!(carry.discount_rate(), r);
        assert_eq!(carry.carry_rate(), b);
        assert_eq!(carry.discount_df(t), libm::exp(-r * t));
        assert_eq!(spot * carry.forward_factor(t), spot * libm::exp(b * t));
    }

    #[test]
    fn rate_sensitivities_arms() {
        let fx = RateSensitivities::Fx {
            rho_dom: 0.42,
            rho_for: -0.17,
        };
        match fx {
            RateSensitivities::Fx { rho_dom, rho_for } => {
                assert_eq!(rho_dom, 0.42);
                assert_eq!(rho_for, -0.17);
            }
            RateSensitivities::Carry { .. } => panic!("expected the Fx arm"),
        }
        // The FX↔carry relation documented on the type: rho_dom = discount_rho +
        // carry_rho, rho_for = −carry_rho. (Binary-exact constants so the equality
        // is exact, not FP-approximate.)
        let (discount_rho, carry_rho) = (0.25_f64, 0.125_f64);
        let derived = RateSensitivities::Fx {
            rho_dom: discount_rho + carry_rho,
            rho_for: -carry_rho,
        };
        assert_eq!(
            derived,
            RateSensitivities::Fx {
                rho_dom: 0.375,
                rho_for: -0.125
            }
        );
    }

    #[test]
    fn gk_derived_quantities() {
        let i = VanillaInputs::new(100.0, 100.0, 0.2, 1.0, 0.05, 0.0);
        // r_for = 0 → df_for = 1, forward = spot·e^{0.05}.
        assert!((i.df_for() - 1.0).abs() < 1e-15);
        assert!((i.forward() - 100.0 * (0.05_f64).exp()).abs() < 1e-9);
    }

    #[test]
    fn premium_adjusted_flag() {
        assert!(PremiumStyle::PercentForeign.is_premium_adjusted());
        assert!(!PremiumStyle::DomesticPips.is_premium_adjusted());
    }

    #[test]
    fn fixing_source_codes_are_distinct_and_nonempty() {
        let all = [
            FixingSource::KrwKftc18,
            FixingSource::TwdTaipei,
            FixingSource::InrRbiRef,
            FixingSource::BrlPtax,
            FixingSource::ClpDolarObs,
            FixingSource::CopTrm,
        ];
        for (i, a) in all.iter().enumerate() {
            assert!(!a.code().is_empty());
            for b in &all[i + 1..] {
                assert_ne!(a.code(), b.code(), "fixing codes must be unique");
                assert_ne!(a, b);
            }
        }
    }

    /// Determinism guardrail: the derived discount/forward quantities feed the
    /// actual pricing path, so they must be **bit-identical** to `libm::exp`
    /// (the same software implementation `celnet-core::math::exp` wraps), not
    /// the platform-dependent `f64::exp` intrinsic. A regression that reroutes
    /// these through the std intrinsic would break cross-platform reproducible
    /// pricing; this asserts exact bit equality, not approximate closeness.
    #[test]
    fn derived_quantities_are_bit_identical_to_libm() {
        let i = VanillaInputs::new(123.45, 130.0, 0.18, 1.37, 0.043, 0.011);
        assert_eq!(i.df_dom().to_bits(), libm::exp(-0.043 * 1.37).to_bits());
        assert_eq!(i.df_for().to_bits(), libm::exp(-0.011 * 1.37).to_bits());
        assert_eq!(
            i.forward().to_bits(),
            (123.45 * libm::exp((0.043 - 0.011) * 1.37)).to_bits()
        );
    }
}
