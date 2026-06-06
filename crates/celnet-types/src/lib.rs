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
