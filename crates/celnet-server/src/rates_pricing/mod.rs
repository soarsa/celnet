//! Linear-rates pricing for the `PricingService::PriceRates` edge RPC.
//!
//! This module is the faithful, allocation-light bridge from the wire contract
//! ([`celnet_proto::RatesPriceRequest`]) to the [`celnet_rates`] term-structure
//! engine and back ([`celnet_proto::RatesPricingResult`]). It is deliberately a
//! **pure function** — no IO, no live-market read — because the caller supplies
//! the market explicitly as a [`celnet_proto::CurveSet`] (exactly as `Price`
//! takes an explicit `MarketContext`). That keeps the gRPC handler in
//! [`crate::services::pricing`] a thin `Status`-wrapping shell and lets the whole
//! conversion + numeric path be unit-tested without a server.
//!
//! ## What it does
//!
//! 1. Validates the curve set (USD only for the P0 arm; non-empty, strictly
//!    increasing pillar tenors) and rebuilds each pillar's spot-starting
//!    USD-SOFR schedule from the curve reference date, so the wire only carries
//!    the economic `(tenor, par_rate)` quote.
//! 2. Bootstraps the self-discounting curve (sequential, acyclic) and prices the
//!    requested [`celnet_proto::OisInstrument`] with [`celnet_rates::ois_risk`],
//!    which returns the **receive-fixed** PV / PV01 / DV01 / key-rate ladder.
//! 3. Applies the client `side` sign (`SIDE_SELL` = receive fixed = `+1`;
//!    `SIDE_BUY` = pay fixed = `-1`) to every signed measure, so a payer and a
//!    receiver of the same swap report equal-and-opposite risk. The par rate is
//!    side-independent and never sign-flipped.
//!
//! Every failure mode is a typed [`RatesPriceError`]; the handler maps it to a
//! `tonic::Status` (invalid-argument for malformed input, internal for a
//! bootstrap failure on otherwise-valid input).

use celnet_bond::{Bond, BondError, bond_risk, price_from_curve};
use celnet_calendar::{RollRule, add_months};
use celnet_proto::{
    AccrualBasis as WireAccrualBasis, BondInstrument, BrokenDate, CurveSet,
    DayCount as WireDayCount, FraInstrument, OisInstrument, OisPillar,
    PaymentFrequency as WirePaymentFrequency, PillarTenor, RatesInstrument, RatesPriceRequest,
    RatesPricingResult, Side, VanillaIrsInstrument, pillar_tenor, rates_instrument,
};
use celnet_rates::{
    AccrualBasis, BootstrapError, Curve, Fra, FraError, OisQuote, OisSchedule, PaymentFrequency,
    ScheduleError, SwapError, VanillaSwap, bootstrap_ois, fra_par_rate, fra_risk, ois_par_rate,
    ois_risk, swap_leg_schedule, swap_par_rate, swap_risk, us_settlement_calendar,
    usd_ois_schedule_for_months, usd_ois_schedule_to_maturity, usd_sofr_ois_schedule,
};
use celnet_types::{DayCount, Rate};
use time::{Date, Month};

mod contract;
pub use contract::{BondPriced, price_bond_via_contract, price_rates_via_contract};

mod engines;
pub(crate) use engines::dispatch_rates_live;
pub use engines::install_fi_house_models;

pub mod stream_spread;

/// The ISO 4217 code of the only currency the P0 rates arm supports.
const SUPPORTED_CURRENCY: &str = "USD";

/// A typed failure of [`price_rates`]. Malformed-input variants map to
/// `Status::invalid_argument`; [`RatesPriceError::Bootstrap`] (a numeric failure
/// on otherwise-valid input) maps to `Status::internal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RatesPriceError {
    /// The request carried no `curve_set`.
    MissingCurveSet,
    /// The curve set carried no `reference_date`.
    MissingReferenceDate,
    /// The `reference_date` is not a real calendar date.
    InvalidReferenceDate,
    /// The curve currency is not the supported P0 currency (USD).
    UnsupportedCurrency(String),
    /// The curve set carried no OIS pillars.
    EmptyPillars,
    /// A pillar (curve or instrument) had a zero tenor; tenors must be `>= 1`.
    ZeroTenor,
    /// A pillar carried no `tenor` (the `PillarTenor` oneof was unset).
    MissingPillarTenor,
    /// A pillar's `maturity_date` arm was not a real calendar date.
    InvalidPillarDate,
    /// The pillar maturities were not strictly increasing (duplicate or out of order).
    NonIncreasingPillars,
    /// The request carried no `instrument`, or an unset oneof arm.
    MissingInstrument,
    /// The instrument notional was not strictly positive.
    NonPositiveNotional,
    /// `SIDE_TWO_WAY` (or an unknown side code) was supplied for an outright price.
    InvalidSide,
    /// Building a spot-starting USD-SOFR schedule failed.
    Schedule(ScheduleError),
    /// Bootstrapping the discount curve failed (a numeric failure).
    Bootstrap(BootstrapError),
    /// A leg payment-frequency enum was not a known value.
    InvalidFrequency,
    /// A leg day-count enum was not a supported value.
    InvalidDayCount,
    /// An accrual-basis enum was not a known value.
    InvalidAccrualBasis,
    /// A FRA window had `end_months <= start_months` (non-increasing).
    NonIncreasingFraWindow,
    /// A cash bond carried no `maturity_date`.
    MissingBondMaturity,
    /// A cash bond's `maturity_date` is not a real calendar date.
    InvalidBondMaturity,
    /// Constructing the vanilla swap or its leg schedules failed (malformed input).
    Swap(SwapError),
    /// Constructing the FRA failed (malformed input).
    FraSetup(FraError),
    /// A cash-bond construction, curve-pricing, or yield solve failed.
    Bond(BondError),
    /// A registered fixed-income house model could not price the instrument (the
    /// pluggable-dispatch path only; the native path never yields this). Carries a
    /// static, provenance-neutral reason.
    Model(&'static str),
}

impl core::fmt::Display for RatesPriceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingCurveSet => f.write_str("missing `curve_set`"),
            Self::MissingReferenceDate => f.write_str("missing `curve_set.reference_date`"),
            Self::InvalidReferenceDate => {
                f.write_str("`reference_date` is not a real calendar date")
            }
            Self::UnsupportedCurrency(c) => {
                write!(
                    f,
                    "unsupported currency `{c}` (only USD is priced in the P0 arm)"
                )
            }
            Self::EmptyPillars => f.write_str("`curve_set.ois_pillars` is empty"),
            Self::ZeroTenor => f.write_str("pillar tenor must be >= 1"),
            Self::MissingPillarTenor => f.write_str("a pillar carried no `tenor`"),
            Self::InvalidPillarDate => {
                f.write_str("a pillar `maturity_date` is not a real calendar date")
            }
            Self::NonIncreasingPillars => {
                f.write_str("`ois_pillars` maturities must be strictly increasing")
            }
            Self::MissingInstrument => f.write_str("missing `instrument`"),
            Self::NonPositiveNotional => f.write_str("notional must be > 0"),
            Self::InvalidSide => {
                f.write_str("side must be SIDE_BUY (pay fixed) or SIDE_SELL (receive fixed)")
            }
            Self::Schedule(e) => write!(f, "schedule: {e}"),
            Self::Bootstrap(e) => write!(f, "bootstrap: {e}"),
            Self::InvalidFrequency => f.write_str("payment frequency is not a known value"),
            Self::InvalidDayCount => f.write_str("day count is not a supported value"),
            Self::InvalidAccrualBasis => f.write_str("accrual basis is not a known value"),
            Self::NonIncreasingFraWindow => {
                f.write_str("FRA `end_months` must be strictly greater than `start_months`")
            }
            Self::MissingBondMaturity => f.write_str("missing bond `maturity_date`"),
            Self::InvalidBondMaturity => {
                f.write_str("bond `maturity_date` is not a real calendar date")
            }
            Self::Swap(e) => write!(f, "swap: {e}"),
            Self::FraSetup(e) => write!(f, "fra: {e}"),
            Self::Bond(e) => write!(f, "bond: {e}"),
            Self::Model(w) => write!(f, "registered rates model: {w}"),
        }
    }
}

impl std::error::Error for RatesPriceError {}

impl From<ScheduleError> for RatesPriceError {
    fn from(e: ScheduleError) -> Self {
        Self::Schedule(e)
    }
}

impl From<BootstrapError> for RatesPriceError {
    fn from(e: BootstrapError) -> Self {
        Self::Bootstrap(e)
    }
}

impl From<SwapError> for RatesPriceError {
    fn from(e: SwapError) -> Self {
        Self::Swap(e)
    }
}

impl From<FraError> for RatesPriceError {
    fn from(e: FraError) -> Self {
        Self::FraSetup(e)
    }
}

impl From<BondError> for RatesPriceError {
    fn from(e: BondError) -> Self {
        Self::Bond(e)
    }
}

/// Parse a wire [`BrokenDate`] to a real [`time::Date`], or `None` if it is not a
/// real civil date. Callers map `None` to the context-appropriate error
/// (reference date vs. pillar maturity).
fn parse_broken_date(d: &BrokenDate) -> Option<Date> {
    let month = u8::try_from(d.month)
        .ok()
        .and_then(|m| Month::try_from(m).ok())?;
    let day = u8::try_from(d.day).ok()?;
    Date::from_calendar_date(d.year, month, day).ok()
}

/// Resolve the `CurveSet` reference (spot-anchor) date, or fail.
fn resolve_date(d: &BrokenDate) -> Result<Date, RatesPriceError> {
    parse_broken_date(d).ok_or(RatesPriceError::InvalidReferenceDate)
}

/// A whole-year [`PillarTenor`] — the canonical liquid-grid arm. Convenience for
/// the static ladder and any caller building a regular-tenor curve.
#[must_use]
pub fn years_pillar(years: u32) -> PillarTenor {
    PillarTenor {
        point: Some(pillar_tenor::Point::Years(years)),
    }
}

/// Build the spot-starting USD-SOFR schedule that prices one curve pillar from its
/// [`PillarTenor`] arm: a whole-year tenor, a month tenor, or an explicit
/// broken-date maturity. All arms use the ACT/360 fixed-leg accrual that the
/// par-OIS quote convention assumes.
fn pillar_schedule(tenor: &PillarTenor, reference: Date) -> Result<OisSchedule, RatesPriceError> {
    match tenor
        .point
        .as_ref()
        .ok_or(RatesPriceError::MissingPillarTenor)?
    {
        pillar_tenor::Point::Years(0) | pillar_tenor::Point::Months(0) => {
            Err(RatesPriceError::ZeroTenor)
        }
        pillar_tenor::Point::Years(years) => Ok(usd_sofr_ois_schedule(reference, *years)?),
        pillar_tenor::Point::Months(months) => Ok(usd_ois_schedule_for_months(
            reference,
            *months,
            AccrualBasis::Act360,
        )?),
        pillar_tenor::Point::MaturityDate(d) => {
            let maturity = parse_broken_date(d).ok_or(RatesPriceError::InvalidPillarDate)?;
            Ok(usd_ois_schedule_to_maturity(
                reference,
                maturity,
                AccrualBasis::Act360,
            )?)
        }
    }
}

/// The receive-fixed sign for a client `side`: `SIDE_SELL` receives fixed (the
/// engine's native convention, `+1`); `SIDE_BUY` pays fixed (`-1`).
fn side_sign(side: Side) -> Result<f64, RatesPriceError> {
    match side {
        Side::Sell => Ok(1.0),
        Side::Buy => Ok(-1.0),
        Side::TwoWay => Err(RatesPriceError::InvalidSide),
    }
}

/// Rebuild the calibrating OIS quotes from a wire [`CurveSet`]: one spot-starting
/// USD-SOFR schedule per pillar, paired with its observed par rate. Validates the
/// currency, non-emptiness, and strictly-increasing tenors.
fn build_quotes(curve: &CurveSet, reference: Date) -> Result<Vec<OisQuote>, RatesPriceError> {
    if !curve.currency.eq_ignore_ascii_case(SUPPORTED_CURRENCY) {
        return Err(RatesPriceError::UnsupportedCurrency(curve.currency.clone()));
    }
    if curve.ois_pillars.is_empty() {
        return Err(RatesPriceError::EmptyPillars);
    }

    let mut quotes = Vec::with_capacity(curve.ois_pillars.len());
    // Order pillars by their final ACT/365F payment time from spot — strictly
    // increasing iff the resolved maturities are, regardless of which arm
    // (years / months / broken date) located each pillar.
    let mut prev_pay = 0.0_f64;
    for pillar in &curve.ois_pillars {
        let tenor = pillar
            .tenor
            .as_ref()
            .ok_or(RatesPriceError::MissingPillarTenor)?;
        let schedule = pillar_schedule(tenor, reference)?;
        let last_pay = schedule
            .periods()
            .last()
            .map(|p| p.pay.0)
            .ok_or(RatesPriceError::ZeroTenor)?;
        if last_pay <= prev_pay {
            return Err(RatesPriceError::NonIncreasingPillars);
        }
        prev_pay = last_pay;
        quotes.push(OisQuote {
            schedule,
            par_rate: Rate(pillar.par_rate),
        });
    }
    Ok(quotes)
}

/// The resolved + bootstrapped market for a wire [`CurveSet`]: the reference date,
/// the calibrating OIS quotes, and the bootstrapped self-discounting discount
/// [`Curve`]. This is the EXACT seam [`price_rates`] builds internally
/// (`resolve_date` → `build_quotes` → `bootstrap_ois`), exposed for the
/// `SurfaceService` curve-query verbs (`GetCurve` / `MarkCurve` / `CurveScenario`)
/// to WRAP rather than fork — so a queried curve is bit-identical to the one a
/// `price_rates` prices against for the same market inputs.
pub(crate) struct ResolvedCurveSet {
    /// The calibrating OIS quotes (one spot-starting USD-SOFR schedule per pillar);
    /// each quote's resolved maturity + par rate is the source of the echoed
    /// [`celnet_proto::CurveParPillar`] set.
    pub(crate) quotes: Vec<OisQuote>,
    /// The bootstrapped self-discounting discount curve.
    pub(crate) curve: Curve,
}

/// Resolve and bootstrap a wire [`CurveSet`] into a [`ResolvedCurveSet`], reusing
/// the same validation + schedule + bootstrap path [`price_rates`] uses.
///
/// # Errors
///
/// Any [`RatesPriceError`] the shared resolve/bootstrap path raises (missing/invalid
/// reference date, unsupported currency, empty/non-increasing pillars, or a
/// [`RatesPriceError::Bootstrap`] numeric failure).
pub(crate) fn resolve_curve_set(curve: &CurveSet) -> Result<ResolvedCurveSet, RatesPriceError> {
    let reference_date = curve
        .reference_date
        .as_ref()
        .ok_or(RatesPriceError::MissingReferenceDate)?;
    let reference = resolve_date(reference_date)?;
    let quotes = build_quotes(curve, reference)?;
    let curve = bootstrap_ois(&quotes)?;
    Ok(ResolvedCurveSet { quotes, curve })
}

/// Price a single [`OisInstrument`] against the calibrating `quotes`, returning
/// the side-signed [`RatesPricingResult`].
fn price_ois(
    ois: &OisInstrument,
    quotes: &[OisQuote],
    reference: Date,
) -> Result<RatesPricingResult, RatesPriceError> {
    if ois.tenor_years == 0 {
        return Err(RatesPriceError::ZeroTenor);
    }
    if ois.notional <= 0.0 {
        return Err(RatesPriceError::NonPositiveNotional);
    }
    let side = Side::try_from(ois.side).map_err(|_| RatesPriceError::InvalidSide)?;
    let sign = side_sign(side)?;

    let schedule = usd_sofr_ois_schedule(reference, ois.tenor_years)?;

    // Receive-fixed risk from the engine, then sign to the client's perspective.
    let risk = ois_risk(quotes, &schedule, Rate(ois.fixed_rate), ois.notional)?;
    // Par (fair fixed) rate is side-independent — bootstrap once for it.
    let base = bootstrap_ois(quotes)?;
    let par = ois_par_rate(&base, &schedule);

    Ok(RatesPricingResult {
        pv: sign * risk.pv,
        par_rate: par.0,
        pv01: sign * risk.pv01,
        dv01: sign * risk.dv01,
        key_rate_ladder: risk.key_rate.iter().map(|k| sign * k).collect(),
        // The two-way + size are an additive *streaming* enrichment: proto3-zero
        // here (the indicative one-shot / RFQ-priced core), populated only on the
        // streaming edge by `stream_spread`. See `celnet.proto` RatesPricingResult.
        ..Default::default()
    })
}

/// The **long/short** sign for a cash-bond `side`: SIDE_BUY is a bought (long)
/// position (`+1`, the native long dirty price and yield DV01); SIDE_SELL is a
/// short (`-1`). SIDE_TWO_WAY is rejected for an outright price. (This differs from
/// the swap/FRA [`side_sign`], where SIDE_SELL is the engine-native receive-fixed
/// `+1` — a bond has no fixed/float legs, only a long/short direction.)
fn bond_side_sign(side: Side) -> Result<f64, RatesPriceError> {
    match side {
        Side::Buy => Ok(1.0),
        Side::Sell => Ok(-1.0),
        Side::TwoWay => Err(RatesPriceError::InvalidSide),
    }
}

/// Map a wire [`WirePaymentFrequency`] enum number to the engine
/// [`PaymentFrequency`]. An out-of-range number is a malformed wire.
fn map_frequency(w: i32) -> Result<PaymentFrequency, RatesPriceError> {
    match WirePaymentFrequency::try_from(w).map_err(|_| RatesPriceError::InvalidFrequency)? {
        WirePaymentFrequency::Annual => Ok(PaymentFrequency::Annual),
        WirePaymentFrequency::SemiAnnual => Ok(PaymentFrequency::SemiAnnual),
        WirePaymentFrequency::Quarterly => Ok(PaymentFrequency::Quarterly),
    }
}

/// Map a wire [`WireDayCount`] enum number to the curve/leg [`DayCount`]. An
/// out-of-range number is a malformed wire.
fn map_day_count(w: i32) -> Result<DayCount, RatesPriceError> {
    match WireDayCount::try_from(w).map_err(|_| RatesPriceError::InvalidDayCount)? {
        WireDayCount::Act365Fixed => Ok(DayCount::Act365Fixed),
        WireDayCount::Act360 => Ok(DayCount::Act360),
    }
}

/// Map a wire [`WireAccrualBasis`] enum number to the leg-accrual [`AccrualBasis`]
/// (the 30/360-carrying superset). An out-of-range number is a malformed wire.
fn map_accrual_basis(w: i32) -> Result<AccrualBasis, RatesPriceError> {
    match WireAccrualBasis::try_from(w).map_err(|_| RatesPriceError::InvalidAccrualBasis)? {
        WireAccrualBasis::Act360 => Ok(AccrualBasis::Act360),
        WireAccrualBasis::Act365Fixed => Ok(AccrualBasis::Act365Fixed),
        WireAccrualBasis::Thirty360BondBasis => Ok(AccrualBasis::Thirty360BondBasis),
    }
}

/// Price a single [`VanillaIrsInstrument`] against the calibrating `quotes`,
/// returning the side-signed [`RatesPricingResult`].
///
/// Wraps [`celnet_rates::swap_leg_schedule`] (one spot-starting leg per side at its
/// own frequency / day-count), [`celnet_rates::VanillaSwap`], and
/// [`celnet_rates::swap_risk`] (receive-fixed PV / PV01 / DV01 / key-rate ladder,
/// then side-signed) plus [`celnet_rates::swap_par_rate`] (the side-independent fair
/// fixed rate). The engine bodies are wrapped verbatim — never reimplemented.
fn price_irs(
    irs: &VanillaIrsInstrument,
    quotes: &[OisQuote],
    reference: Date,
) -> Result<RatesPricingResult, RatesPriceError> {
    if irs.tenor_years == 0 {
        return Err(RatesPriceError::ZeroTenor);
    }
    if irs.notional <= 0.0 {
        return Err(RatesPriceError::NonPositiveNotional);
    }
    let side = Side::try_from(irs.side).map_err(|_| RatesPriceError::InvalidSide)?;
    let sign = side_sign(side)?;

    let fixed_freq = map_frequency(irs.fixed_frequency)?;
    let float_freq = map_frequency(irs.float_frequency)?;
    let fixed_dc = map_day_count(irs.fixed_day_count)?;
    let float_dc = map_day_count(irs.float_day_count)?;

    // Each leg's spot-starting schedule (the schedule builder normalises `reference`
    // to the next US business day internally — the same curve time-0 as the OIS arm).
    let fixed_leg = swap_leg_schedule(reference, irs.tenor_years, fixed_freq, fixed_dc)?;
    let float_leg = swap_leg_schedule(reference, irs.tenor_years, float_freq, float_dc)?;
    let swap = VanillaSwap::new(fixed_leg, float_leg, Rate(irs.fixed_rate), irs.notional)?;

    // Receive-fixed risk from the engine, then sign to the client's perspective.
    let risk = swap_risk(quotes, &swap)?;
    // Par (fair fixed) rate is side-independent — bootstrap once for it.
    let base = bootstrap_ois(quotes)?;
    let par = swap_par_rate(&base, &swap);

    Ok(RatesPricingResult {
        pv: sign * risk.pv,
        par_rate: par.0,
        pv01: sign * risk.pv01,
        dv01: sign * risk.dv01,
        key_rate_ladder: risk.key_rate.iter().map(|k| sign * k).collect(),
        // The two-way + size are an additive *streaming* enrichment: proto3-zero
        // here (the indicative one-shot / RFQ-priced core), populated only on the
        // streaming edge by `stream_spread`. See `celnet.proto` RatesPricingResult.
        ..Default::default()
    })
}

/// Price a single [`FraInstrument`] against the calibrating `quotes`, returning the
/// side-signed [`RatesPricingResult`].
///
/// Wraps [`celnet_rates::Fra::from_dates`] (the single accrual window, its
/// roll-adjusted fixing/maturity dates rebuilt from the curve reference date),
/// [`celnet_rates::fra_risk`] (receive-fixed PV / PV01 / DV01 / key-rate ladder,
/// then side-signed) and [`celnet_rates::fra_par_rate`] (the side-independent
/// break-even rate). The engine bodies are wrapped verbatim.
fn price_fra(
    fra: &FraInstrument,
    quotes: &[OisQuote],
    reference: Date,
) -> Result<RatesPricingResult, RatesPriceError> {
    if fra.notional <= 0.0 {
        return Err(RatesPriceError::NonPositiveNotional);
    }
    if fra.end_months <= fra.start_months {
        return Err(RatesPriceError::NonIncreasingFraWindow);
    }
    let side = Side::try_from(fra.side).map_err(|_| RatesPriceError::InvalidSide)?;
    let sign = side_sign(side)?;
    let basis = map_accrual_basis(fra.accrual_basis)?;

    // The window's roll-adjusted dates, on the same axis as the OIS/IRS curve: the
    // spot start is the reference rolled to the next US business day (curve time 0),
    // each window end rolled modified-following; the FRA curve coordinates are then
    // measured ACT/365F from that spot (`Fra::from_dates`).
    let cal = us_settlement_calendar();
    let start = RollRule::Following.adjust(&cal, reference);
    let fixing_date =
        RollRule::ModifiedFollowing.adjust(&cal, add_months(start, fra.start_months as i32));
    let maturity_date =
        RollRule::ModifiedFollowing.adjust(&cal, add_months(start, fra.end_months as i32));
    let fra_contract = Fra::from_dates(
        start,
        fixing_date,
        maturity_date,
        basis,
        Rate(fra.fixed_rate),
        fra.notional,
    )?;

    let risk = fra_risk(quotes, &fra_contract)?;
    let base = bootstrap_ois(quotes)?;
    let par = fra_par_rate(&base, &fra_contract);

    Ok(RatesPricingResult {
        pv: sign * risk.pv,
        par_rate: par.0,
        pv01: sign * risk.pv01,
        dv01: sign * risk.dv01,
        key_rate_ladder: risk.key_rate.iter().map(|k| sign * k).collect(),
        // The two-way + size are an additive *streaming* enrichment: proto3-zero
        // here (the indicative one-shot / RFQ-priced core), populated only on the
        // streaming edge by `stream_spread`. See `celnet.proto` RatesPricingResult.
        ..Default::default()
    })
}

/// Price a single [`BondInstrument`] off the calibrated curve, returning the
/// side-signed [`RatesPricingResult`].
///
/// Wraps [`celnet_bond::price_from_curve`] (the dirty price discounted off the
/// bootstrapped OIS curve — the identical body [`contract::BondEngine`] carries as
/// the risk-leaf seam) and [`celnet_bond::bond_risk`] (the yield-risk set implied by
/// that price). The bond settles on the curve reference (spot-anchor) date. The wire
/// result maps: `pv = dirty price`, `par_rate = yield to maturity` (the bond's
/// break-even yield, side-independent), and `pv01 = dv01 = the yield DV01`
/// (a fixed-coupon bond's single rate sensitivity; PV01 and DV01 coincide, exactly
/// as [`contract::BondEngine::risk`] reports). The per-pillar `key_rate_ladder` is
/// empty: the wrapped [`bond_risk`] is a closed-form **yield-space** sensitivity, so
/// it carries no per-calibrating-pillar decomposition (a curve-space key-rate ladder
/// would require re-pricing bumped curves — new risk math this arm does not
/// reimplement). Every measure is side-signed (SIDE_BUY long, SIDE_SELL short).
fn price_bond_instrument(
    bond: &BondInstrument,
    quotes: &[OisQuote],
    reference: Date,
) -> Result<RatesPricingResult, RatesPriceError> {
    let side = Side::try_from(bond.side).map_err(|_| RatesPriceError::InvalidSide)?;
    let sign = bond_side_sign(side)?;
    let bond_contract = bond_contract_from_wire(bond, reference)?;

    let curve = bootstrap_ois(quotes)?;
    let dirty_price = price_from_curve(&bond_contract, &curve)?;
    let risk = bond_risk(&bond_contract, dirty_price)?;

    Ok(RatesPricingResult {
        pv: sign * dirty_price,
        par_rate: risk.yield_to_maturity.0,
        pv01: sign * risk.dv01,
        dv01: sign * risk.dv01,
        key_rate_ladder: Vec::new(),
        // Streaming-only two-way (see the swap arms above / `stream_spread`).
        ..Default::default()
    })
}

/// Build the [`celnet_bond::Bond`] contract from the wire [`BondInstrument`] and the
/// curve reference date — the side-agnostic economics shared by [`price_bond_instrument`]
/// (which applies the `side` sign after) and [`quote_bond`] (which prices the magnitude).
///
/// Settlement = the curve time-0 date (reference rolled to the next US business day), so
/// the bond's cashflow-time axis coincides with the discount curve's. `Bond::new`
/// validates `maturity > settlement` and `redemption > 0`.
fn bond_contract_from_wire(
    bond: &BondInstrument,
    reference: Date,
) -> Result<Bond, RatesPriceError> {
    let frequency = map_frequency(bond.coupon_frequency)?;
    let day_count = map_accrual_basis(bond.day_count)?;
    let maturity = bond
        .maturity_date
        .as_ref()
        .ok_or(RatesPriceError::MissingBondMaturity)?;
    let maturity_date = parse_broken_date(maturity).ok_or(RatesPriceError::InvalidBondMaturity)?;
    let cal = us_settlement_calendar();
    let settlement = RollRule::Following.adjust(&cal, reference);
    Ok(Bond::new(
        settlement,
        maturity_date,
        bond.coupon_rate,
        frequency,
        day_count,
        bond.redemption,
    )?)
}

/// Price the [`RatesPriceRequest`]'s instrument against its curve set.
///
/// Pure and IO-free: the market is the request-supplied [`CurveSet`]. Returns the
/// side-signed [`RatesPricingResult`]; every failure is a typed [`RatesPriceError`].
///
/// # Errors
///
/// Returns [`RatesPriceError`] for a missing/invalid curve set, an unsupported
/// currency, malformed pillars, a missing/invalid instrument, or a numeric
/// schedule/bootstrap failure.
pub fn price_rates(req: &RatesPriceRequest) -> Result<RatesPricingResult, RatesPriceError> {
    let (quotes, reference) = build_rates_market(req)?;
    match rates_instrument_of(req)? {
        rates_instrument::Instrument::Ois(ois) => price_ois(ois, &quotes, reference),
        rates_instrument::Instrument::Irs(irs) => price_irs(irs, &quotes, reference),
        rates_instrument::Instrument::Fra(fra) => price_fra(fra, &quotes, reference),
        rates_instrument::Instrument::Bond(bond) => price_bond_instrument(bond, &quotes, reference),
    }
}

/// Resolve the request's curve set into the calibrating OIS `quotes` + the curve
/// reference date — the shared front-half of both the native [`price_rates`] and
/// the pluggable [`engines::dispatch_rates`] path, so the two never drift.
///
/// # Errors
///
/// Returns [`RatesPriceError`] for a missing/invalid curve set, an unsupported
/// currency, or malformed pillars.
pub(super) fn build_rates_market(
    req: &RatesPriceRequest,
) -> Result<(Vec<OisQuote>, Date), RatesPriceError> {
    let curve = req
        .curve_set
        .as_ref()
        .ok_or(RatesPriceError::MissingCurveSet)?;
    let reference_date = curve
        .reference_date
        .as_ref()
        .ok_or(RatesPriceError::MissingReferenceDate)?;
    let reference = resolve_date(reference_date)?;
    let quotes = build_quotes(curve, reference)?;
    Ok((quotes, reference))
}

/// Borrow the request's decoded rates-instrument oneof arm.
///
/// # Errors
///
/// Returns [`RatesPriceError::MissingInstrument`] if the request carried no
/// instrument or an unset oneof arm.
pub(super) fn rates_instrument_of(
    req: &RatesPriceRequest,
) -> Result<&rates_instrument::Instrument, RatesPriceError> {
    req.instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())
        .ok_or(RatesPriceError::MissingInstrument)
}

/// The P0 static USD-SOFR par-OIS pillar ladder `(tenor_years, par_rate)`.
///
/// This is the market the FIX rates edge quotes against until a live SOFR feed is
/// wired (operator question Q6 — test-environment data-provider access — is
/// deferred). It is a *real* calibrating market, not a stub: it bootstraps a real
/// self-discounting curve and produces real par rates and risk. When the feed
/// lands it simply replaces this table; nothing else changes.
const P0_USD_SOFR_PILLARS: &[(u32, f64)] = &[
    (1, 0.0432),
    (2, 0.0418),
    (3, 0.0409),
    (5, 0.0405),
    (7, 0.0408),
    (10, 0.0415),
    (15, 0.0421),
    (20, 0.0424),
    (30, 0.0423),
];

/// The reference (spot-anchor) date of the P0 static curve.
const P0_REFERENCE: BrokenDate = BrokenDate {
    year: 2026,
    month: 6,
    day: 25,
};

/// The P0 static USD-SOFR [`CurveSet`] the FIX rates edge prices against (see
/// [`P0_USD_SOFR_PILLARS`]). A real calibrating market pending a live SOFR feed.
#[must_use]
pub fn default_usd_sofr_curve_set() -> CurveSet {
    CurveSet {
        currency: SUPPORTED_CURRENCY.to_string(),
        reference_date: Some(P0_REFERENCE),
        ois_pillars: P0_USD_SOFR_PILLARS
            .iter()
            .map(|&(years, par_rate)| OisPillar {
                tenor: Some(years_pillar(years)),
                par_rate,
            })
            .collect(),
    }
}

/// The fair (par) fixed rate of a spot-starting USD-SOFR OIS of `tenor_years`,
/// bootstrapped from `curve` — the number the FIX rates edge centres a two-way
/// RFQ market on. Side- and notional-independent.
///
/// # Errors
///
/// Returns [`RatesPriceError`] for an invalid curve, an unsupported currency, or a
/// numeric schedule/bootstrap failure.
pub fn par_rate_for(curve: &CurveSet, tenor_years: u32) -> Result<f64, RatesPriceError> {
    if tenor_years == 0 {
        return Err(RatesPriceError::ZeroTenor);
    }
    let reference_date = curve
        .reference_date
        .as_ref()
        .ok_or(RatesPriceError::MissingReferenceDate)?;
    let reference = resolve_date(reference_date)?;
    let quotes = build_quotes(curve, reference)?;
    let schedule = usd_sofr_ois_schedule(reference, tenor_years)?;
    let base = bootstrap_ois(&quotes)?;
    Ok(ois_par_rate(&base, &schedule).0)
}

/// The two-way-quoting view of a cash bond the FIX/RFS edge centres a market on: the
/// dirty price discounted off the curve, the clean (quoted) price and accrued interest,
/// the yield DV01, and the yield to maturity — all as **side-independent magnitudes**
/// (a bond's price and DV01 do not depend on long/short; only the PV sign does, and the
/// edge quotes the price two-way around the magnitude, exactly as it quotes an OIS
/// two-way around the side-independent par rate).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BondQuote {
    /// The dirty (full) price discounted off the curve — [`celnet_bond::price_from_curve`].
    pub dirty_price: f64,
    /// The clean (quoted) price: `dirty − accrued`. The market-convention quote level.
    pub clean_price: f64,
    /// Accrued interest from the last coupon to settlement.
    pub accrued_interest: f64,
    /// DV01: the dirty-price move per +1bp of yield (a positive magnitude).
    pub dv01: f64,
    /// Yield to maturity implied by the dirty price (the bond's break-even yield).
    pub yield_to_maturity: f64,
}

/// Price a cash [`BondInstrument`] off a [`CurveSet`] for the quoting edge, returning the
/// full [`BondQuote`] (clean/dirty price, accrued, DV01, YTM) as side-independent
/// magnitudes.
///
/// Byte-identical to [`price_rates`]'s `Bond` arm on the shared numbers: it calls the
/// **identical** [`bond_contract_from_wire`] / [`bootstrap_ois`] / [`price_from_curve`] /
/// [`bond_risk`] bodies (gated by [`tests::quote_bond_matches_price_rates`]). It differs
/// only in that it exposes the clean price + accrued the FIX two-way market is struck on
/// (which the scalar [`RatesPricingResult`] does not carry) and never applies a `side`
/// sign — so a two-way (`SIDE_TWO_WAY`) request, which [`price_rates`] rejects, is
/// priced at magnitude. This is the cash-bond analogue of [`par_rate_for`].
///
/// # Errors
///
/// Returns [`RatesPriceError`] for a missing/invalid curve set, malformed bond
/// economics, an invalid maturity, or a numeric schedule/bootstrap/yield-solve failure.
pub fn quote_bond(bond: &BondInstrument, curve: &CurveSet) -> Result<BondQuote, RatesPriceError> {
    let reference_date = curve
        .reference_date
        .as_ref()
        .ok_or(RatesPriceError::MissingReferenceDate)?;
    let reference = resolve_date(reference_date)?;
    let quotes = build_quotes(curve, reference)?;
    let bond_contract = bond_contract_from_wire(bond, reference)?;
    let discount = bootstrap_ois(&quotes)?;
    let dirty_price = price_from_curve(&bond_contract, &discount)?;
    let risk = bond_risk(&bond_contract, dirty_price)?;
    Ok(BondQuote {
        dirty_price,
        clean_price: risk.clean_price,
        accrued_interest: risk.accrued_interest,
        dv01: risk.dv01,
        yield_to_maturity: risk.yield_to_maturity.0,
    })
}

/// Derive the streamed two-way (bid/offer) + available size for a priced rates
/// line, dispatching on the instrument arm: a cash bond is quoted around its clean
/// price (from [`quote_bond`]), every swap/FRA arm around the par rate the priced
/// [`RatesPricingResult`] already carries. The half-spread and size come from the
/// duration-scaled [`stream_spread`] model — see its module doc for the convention.
/// This is the single choke point the streaming edge ([`crate::services::stream`])
/// calls to enrich an indicative line into a tradeable one; the one-shot `PriceRates`
/// RPC never calls it, so its result stays proto3-zero on the two-way fields.
///
/// # Errors
///
/// Returns [`RatesPriceError`] for a missing instrument arm, or (bond only) any
/// clean-price re-derivation failure from [`quote_bond`].
pub fn stream_two_way(
    instrument: &RatesInstrument,
    curve: &CurveSet,
    result: &RatesPricingResult,
) -> Result<stream_spread::StreamTwoWay, RatesPriceError> {
    let arm = instrument
        .instrument
        .as_ref()
        .ok_or(RatesPriceError::MissingInstrument)?;
    let two_way = match arm {
        rates_instrument::Instrument::Bond(bond) => {
            let clean = quote_bond(bond, curve)?.clean_price;
            // The bond's face (redemption) is the size unit; the two-way is a
            // clean-price market.
            stream_spread::two_way_price(clean, result, bond.redemption)
        }
        rates_instrument::Instrument::Ois(ois) => {
            stream_spread::two_way_rate(result.par_rate, result, ois.notional)
        }
        rates_instrument::Instrument::Irs(irs) => {
            stream_spread::two_way_rate(result.par_rate, result, irs.notional)
        }
        rates_instrument::Instrument::Fra(fra) => {
            stream_spread::two_way_rate(result.par_rate, result, fra.notional)
        }
    };
    Ok(two_way)
}

/// The maker half-spread for a fixed-income two-way **rate** market, in absolute
/// rate (`0.00005` = 0.5bp each side ⇒ a 1bp-wide market). The single-homed
/// constant the FIX RFQ line ([`crate::services::fix`]) and the WS/gRPC
/// `RequestRatesQuote` taker two-way ([`quote_rates_two_way`]) both centre an
/// OIS/IRS/FRA market on. This is the *indicative* (flat) RFQ floor; the live
/// **streaming** line widens it with the line's modified duration via
/// [`stream_spread`] — of which this constant is the zero-duration floor, so the
/// RFQ line and the streamed line are one consistent family.
pub const RATES_RFQ_HALF_SPREAD: f64 = 0.000_05;

/// The maker half-spread for a fixed-income two-way **clean-price** market, in
/// price points per 100 face (`0.05` = 5 cents each side ⇒ a 10-cent-wide market).
/// The bond analogue of [`RATES_RFQ_HALF_SPREAD`] (which is in rate), single-homed
/// for the FIX bond line and the WS/gRPC bond RFQ two-way alike.
pub const BOND_RFQ_HALF_SPREAD: f64 = 0.05;

/// A tradeable two-way FI RFQ line plus the full priced risk — the return of
/// [`quote_rates_two_way`], the taker-side analogue of the FIX maker `PricedLine`.
#[derive(Debug, Clone, PartialEq)]
pub struct RatesTwoWay {
    /// The two-way bid struck around the side-independent fair level (a rate for
    /// OIS/IRS/FRA, a clean price for a cash bond).
    pub bid: f64,
    /// The two-way offer.
    pub offer: f64,
    /// The full linear-rates risk (PV, par rate, PV01, DV01, key-rate ladder) of
    /// the position at the requested side.
    pub result: RatesPricingResult,
}

/// The directional pricing side the RFQ `side` resolves to for the RISK measures.
/// A firm `SIDE_BUY` / `SIDE_SELL` is honoured verbatim; a `SIDE_TWO_WAY` request
/// (which [`price_rates`] rejects) reports the canonical magnitude — receive-fixed
/// (`SIDE_SELL`, the engine-native `+1`) for a swap/FRA, long (`SIDE_BUY`, the
/// native `+1`) for a cash bond — so the two-way always carries a well-defined
/// risk sign while the price itself stays struck around the side-independent fair
/// level (the par rate / clean price, neither of which depends on side).
fn resolve_pricing_side(side: Side, is_bond: bool) -> Side {
    match side {
        Side::Buy => Side::Buy,
        Side::Sell => Side::Sell,
        Side::TwoWay => {
            if is_bond {
                Side::Buy
            } else {
                Side::Sell
            }
        }
    }
}

/// Override the set instrument arm's `side` (the RFQ envelope side governs the
/// risk sign, not any side carried on the wire instrument arm).
fn override_arm_side(instrument: &mut RatesInstrument, side: Side) -> Result<(), RatesPriceError> {
    match instrument
        .instrument
        .as_mut()
        .ok_or(RatesPriceError::MissingInstrument)?
    {
        rates_instrument::Instrument::Ois(o) => o.side = side as i32,
        rates_instrument::Instrument::Irs(i) => i.side = side as i32,
        rates_instrument::Instrument::Fra(f) => f.side = side as i32,
        rates_instrument::Instrument::Bond(b) => b.side = side as i32,
    }
    Ok(())
}

/// Price a linear-rates instrument to a tradeable two-way RFQ line — the taker-side
/// FI quote the WS/gRPC `QuoteService.RequestRatesQuote` returns, generalising the
/// FIX maker `rates_line` / `bond_line` clean-price machinery across ALL four
/// instrument arms.
///
/// The two-way is struck around the **side-independent fair level** — the par
/// (fair fixed) rate for an OIS/IRS/FRA (the LANDED [`price_rates`] `par_rate`),
/// the clean price for a cash bond (the LANDED [`quote_bond`] `clean_price`) — split
/// [`RATES_RFQ_HALF_SPREAD`] / [`BOND_RFQ_HALF_SPREAD`] either side, exactly as the
/// FIX edge centres its markets. The returned [`RatesTwoWay::result`] is the FULL
/// [`price_rates`] risk (PV / PV01 / DV01 / key-rate ladder) signed to the RFQ
/// `side` (see [`resolve_pricing_side`]), so the caller sees the same authoritative
/// engine numbers the outright `PriceRates` RPC returns — one FI pricing path, no
/// second implementation (gated by `tests::rates_two_way_mid_matches_engine`).
///
/// # Errors
///
/// Returns [`RatesPriceError`] for a missing/invalid curve set or instrument,
/// malformed economics, or a numeric schedule/bootstrap/yield-solve failure.
pub fn quote_rates_two_way(
    curve_set: &CurveSet,
    instrument: &RatesInstrument,
    side: Side,
) -> Result<RatesTwoWay, RatesPriceError> {
    let arm = instrument
        .instrument
        .as_ref()
        .ok_or(RatesPriceError::MissingInstrument)?;
    let is_bond = matches!(arm, rates_instrument::Instrument::Bond(_));
    let pricing_side = resolve_pricing_side(side, is_bond);

    // The full risk at the resolved directional side — the RFQ envelope side
    // governs the sign, so the wire instrument arm's own `side` is overridden.
    let mut priced = *instrument;
    override_arm_side(&mut priced, pricing_side)?;
    let result = price_rates(&RatesPriceRequest {
        request_id: 0,
        curve_set: Some(curve_set.clone()),
        instrument: Some(priced),
        correlation_id: None,
    })?;

    // The two-way struck around the side-independent fair level: the par rate for a
    // swap/FRA, the clean price for a cash bond (the FIX `rates_line` / `bond_line`
    // discipline, generalised — one `celnet-fix` `two_way_rates` helper, single-homed).
    let (bid, offer) = match arm {
        rates_instrument::Instrument::Bond(bond) => {
            let clean = quote_bond(bond, curve_set)?.clean_price;
            celnet_fix::dialect_rates::two_way_rates(clean, BOND_RFQ_HALF_SPREAD)
        }
        _ => celnet_fix::dialect_rates::two_way_rates(result.par_rate, RATES_RFQ_HALF_SPREAD),
    };

    Ok(RatesTwoWay { bid, offer, result })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{CurveSet, OisInstrument, OisPillar, RatesInstrument};

    /// 2026-06-25 reference, a representative USD-SOFR par-OIS ladder.
    fn reference() -> BrokenDate {
        BrokenDate {
            year: 2026,
            month: 6,
            day: 25,
        }
    }

    fn pillar(years: u32, par_rate: f64) -> OisPillar {
        OisPillar {
            tenor: Some(years_pillar(years)),
            par_rate,
        }
    }

    fn pillars() -> Vec<OisPillar> {
        vec![
            pillar(1, 0.0420),
            pillar(2, 0.0410),
            pillar(5, 0.0405),
            pillar(10, 0.0415),
        ]
    }

    fn curve_set() -> CurveSet {
        CurveSet {
            currency: "USD".to_string(),
            reference_date: Some(reference()),
            ois_pillars: pillars(),
        }
    }

    fn request(tenor: u32, fixed_rate: f64, notional: f64, side: Side) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: tenor,
                    fixed_rate,
                    notional,
                    side: side as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    #[test]
    fn server_path_reproduces_engine_exactly() {
        // The wire conversion must be byte-identical to a direct engine call:
        // price a 5y receive-fixed OIS @ 4.00% on 100mm both ways and compare.
        let req = request(5, 0.04, 100_000_000.0, Side::Sell);
        let got = price_rates(&req).unwrap();

        let reference = resolve_date(&reference()).unwrap();
        let quotes = build_quotes(&curve_set(), reference).unwrap();
        let schedule = usd_sofr_ois_schedule(reference, 5).unwrap();
        let risk = ois_risk(&quotes, &schedule, Rate(0.04), 100_000_000.0).unwrap();

        assert_eq!(got.pv, risk.pv);
        assert_eq!(got.pv01, risk.pv01);
        assert_eq!(got.dv01, risk.dv01);
        assert_eq!(got.key_rate_ladder, risk.key_rate);
        assert_eq!(got.key_rate_ladder.len(), pillars().len());
    }

    #[test]
    fn par_rate_priced_at_par_gives_zero_pv() {
        // Pricing at the engine's own par rate must yield ~0 PV (the defining
        // identity of a par swap).
        let probe = price_rates(&request(5, 0.04, 100_000_000.0, Side::Sell)).unwrap();
        let at_par = price_rates(&request(5, probe.par_rate, 100_000_000.0, Side::Sell)).unwrap();
        assert!(at_par.pv.abs() < 1e-6, "par PV not zero: {}", at_par.pv);
    }

    #[test]
    fn payer_is_exactly_opposite_receiver() {
        let recv = price_rates(&request(7, 0.041, 50_000_000.0, Side::Sell)).unwrap();
        let pay = price_rates(&request(7, 0.041, 50_000_000.0, Side::Buy)).unwrap();
        assert_eq!(pay.pv, -recv.pv);
        assert_eq!(pay.pv01, -recv.pv01);
        assert_eq!(pay.dv01, -recv.dv01);
        assert_eq!(pay.par_rate, recv.par_rate); // par is side-independent
        for (p, r) in pay.key_rate_ladder.iter().zip(&recv.key_rate_ladder) {
            assert_eq!(*p, -r);
        }
    }

    #[test]
    fn key_rate_ladder_sums_to_dv01() {
        let res = price_rates(&request(7, 0.041, 100_000_000.0, Side::Sell)).unwrap();
        let summed: f64 = res.key_rate_ladder.iter().sum();
        // First-order additive; residual is curve cross-gamma (~0.04%).
        assert!(
            (summed - res.dv01).abs() / res.dv01.abs() < 5e-3,
            "ladder sum {summed} vs dv01 {}",
            res.dv01
        );
    }

    #[test]
    fn rejects_non_usd_currency() {
        let mut req = request(5, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().currency = "EUR".to_string();
        assert_eq!(
            price_rates(&req),
            Err(RatesPriceError::UnsupportedCurrency("EUR".to_string()))
        );
    }

    #[test]
    fn rejects_two_way_side() {
        let req = request(5, 0.04, 100.0, Side::TwoWay);
        assert_eq!(price_rates(&req), Err(RatesPriceError::InvalidSide));
    }

    #[test]
    fn rejects_non_positive_notional() {
        let req = request(5, 0.04, 0.0, Side::Sell);
        assert_eq!(price_rates(&req), Err(RatesPriceError::NonPositiveNotional));
    }

    #[test]
    fn rejects_non_increasing_pillars() {
        let mut req = request(5, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().ois_pillars = vec![pillar(2, 0.041), pillar(2, 0.041)];
        assert_eq!(
            price_rates(&req),
            Err(RatesPriceError::NonIncreasingPillars)
        );
    }

    #[test]
    fn month_tenor_of_whole_year_matches_year_pillar() {
        // A 24-month pillar resolves to the same schedule (hence the same bootstrap
        // and par rate) as the 2-year pillar — the dated/tenor parity guarantee.
        let mut req = request(2, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().ois_pillars = vec![pillar(1, 0.0420), pillar(2, 0.0410)];
        let by_years = price_rates(&req).expect("years price");

        req.curve_set.as_mut().unwrap().ois_pillars = vec![
            OisPillar {
                tenor: Some(PillarTenor {
                    point: Some(pillar_tenor::Point::Months(12)),
                }),
                par_rate: 0.0420,
            },
            OisPillar {
                tenor: Some(PillarTenor {
                    point: Some(pillar_tenor::Point::Months(24)),
                }),
                par_rate: 0.0410,
            },
        ];
        let by_months = price_rates(&req).expect("months price");
        assert_eq!(by_years.par_rate, by_months.par_rate);
        assert_eq!(by_years.pv, by_months.pv);
    }

    #[test]
    fn broken_date_pillar_prices() {
        // An explicit broken-date maturity (≈18M) calibrates and prices without
        // error, exercising the dated arm end to end.
        let mut req = request(1, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().ois_pillars = vec![
            pillar(1, 0.0420),
            OisPillar {
                tenor: Some(PillarTenor {
                    point: Some(pillar_tenor::Point::MaturityDate(BrokenDate {
                        year: 2027,
                        month: 12,
                        day: 27,
                    })),
                }),
                par_rate: 0.0412,
            },
        ];
        let priced = price_rates(&req).expect("broken-date price");
        assert!(priced.par_rate.is_finite() && priced.par_rate > 0.0);
    }

    #[test]
    fn rejects_missing_curve_set() {
        let mut req = request(5, 0.04, 100.0, Side::Sell);
        req.curve_set = None;
        assert_eq!(price_rates(&req), Err(RatesPriceError::MissingCurveSet));
    }

    #[test]
    fn rejects_invalid_reference_date() {
        let mut req = request(5, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().reference_date = Some(BrokenDate {
            year: 2026,
            month: 13,
            day: 1,
        });
        assert_eq!(
            price_rates(&req),
            Err(RatesPriceError::InvalidReferenceDate)
        );
    }

    #[test]
    fn default_curve_reprices_its_own_pillar() {
        // The bootstrap reprices each calibrating pillar at par, so the par rate
        // of a fresh schedule at a pillar tenor returns the quoted pillar rate.
        let curve = default_usd_sofr_curve_set();
        let par5 = par_rate_for(&curve, 5).unwrap();
        assert!((par5 - 0.0405).abs() < 1e-6, "5y par {par5}");
    }

    #[test]
    fn par_rate_for_agrees_with_price_rates() {
        let curve = default_usd_sofr_curve_set();
        let par = par_rate_for(&curve, 7).unwrap();
        let req = RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 7,
                    fixed_rate: 0.0,
                    notional: 1.0,
                    side: Side::Sell as i32,
                })),
            }),
            correlation_id: None,
        };
        assert_eq!(price_rates(&req).unwrap().par_rate, par);
    }

    // --- IRS / FRA / cash-bond wire arms ------------------------------------

    fn irs_request(tenor: u32, fixed_rate: f64, notional: f64, side: Side) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Irs(VanillaIrsInstrument {
                    tenor_years: tenor,
                    fixed_rate,
                    notional,
                    side: side as i32,
                    fixed_frequency: WirePaymentFrequency::SemiAnnual as i32,
                    fixed_day_count: WireDayCount::Act360 as i32,
                    float_frequency: WirePaymentFrequency::Quarterly as i32,
                    float_day_count: WireDayCount::Act360 as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    fn fra_request(
        start_months: u32,
        end_months: u32,
        fixed_rate: f64,
        notional: f64,
        side: Side,
    ) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Fra(FraInstrument {
                    start_months,
                    end_months,
                    fixed_rate,
                    notional,
                    side: side as i32,
                    accrual_basis: WireAccrualBasis::Act360 as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    fn bond_request(
        coupon_rate: f64,
        maturity: (i32, u32, u32),
        redemption: f64,
        side: Side,
    ) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Bond(BondInstrument {
                    coupon_rate,
                    coupon_frequency: WirePaymentFrequency::SemiAnnual as i32,
                    day_count: WireAccrualBasis::Thirty360BondBasis as i32,
                    maturity_date: Some(BrokenDate {
                        year: maturity.0,
                        month: maturity.1,
                        day: maturity.2,
                    }),
                    redemption,
                    side: side as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    /// IRS ORACLE — the defining par-swap identity: at the fair (par) fixed rate a
    /// vanilla swap is worth exactly zero. Price the IRS to read its `par_rate`, then
    /// re-price AT that rate on unit notional; |PV| must be <= 1e-12 (independent of
    /// the pricing implementation — it is a mathematical identity `N(K*·A − F) = 0`).
    #[test]
    fn irs_priced_at_par_rate_has_zero_pv() {
        let probe = price_rates(&irs_request(5, 0.04, 1.0, Side::Sell)).unwrap();
        let at_par = price_rates(&irs_request(5, probe.par_rate, 1.0, Side::Sell)).unwrap();
        assert!(
            at_par.pv.abs() <= 1e-12,
            "par IRS PV not zero: {}",
            at_par.pv
        );
    }

    /// The IRS payer is exactly the opposite of the receiver; par is side-independent.
    #[test]
    fn irs_payer_is_exactly_opposite_receiver() {
        let recv = price_rates(&irs_request(7, 0.041, 50_000_000.0, Side::Sell)).unwrap();
        let pay = price_rates(&irs_request(7, 0.041, 50_000_000.0, Side::Buy)).unwrap();
        assert_eq!(pay.pv, -recv.pv);
        assert_eq!(pay.pv01, -recv.pv01);
        assert_eq!(pay.dv01, -recv.dv01);
        assert_eq!(pay.par_rate, recv.par_rate);
    }

    /// FRA ORACLE — a FRA is exactly a one-period OIS swaplet. Price a 3x6 FRA (unit
    /// notional, receive-fixed) through the wire, then independently rebuild the
    /// identical single accrual window and price it with the INDEPENDENT `ois_pv`
    /// code path (a distinct implementation in `celnet_rates::ois`). |Δ| <= 1e-12.
    #[test]
    fn fra_pv_matches_independent_single_period_swaplet() {
        use celnet_rates::{FixedPeriod, OisSchedule, ois_pv};
        use celnet_types::Time;

        let wire = price_rates(&fra_request(3, 6, 0.033, 1.0, Side::Sell)).unwrap();

        let reference = resolve_date(&reference()).unwrap();
        let quotes = build_quotes(&curve_set(), reference).unwrap();
        let base = bootstrap_ois(&quotes).unwrap();

        // Rebuild the exact FRA the dispatch built (identical rolled dates + axis).
        let cal = us_settlement_calendar();
        let start = RollRule::Following.adjust(&cal, reference);
        let fixing = RollRule::ModifiedFollowing.adjust(&cal, add_months(start, 3));
        let maturity = RollRule::ModifiedFollowing.adjust(&cal, add_months(start, 6));
        let fra = Fra::from_dates(
            start,
            fixing,
            maturity,
            AccrualBasis::Act360,
            Rate(0.033),
            1.0,
        )
        .unwrap();

        // The equivalent single-period OIS schedule on the same absolute curve times.
        let swaplet = OisSchedule::new(
            fra.fixing,
            vec![FixedPeriod {
                pay: fra.maturity,
                accrual: Time(fra.accrual),
            }],
        )
        .unwrap();
        let independent = ois_pv(&base, &swaplet, Rate(0.033), 1.0);

        assert!(
            (wire.pv - independent).abs() <= 1e-12,
            "FRA wire pv {} vs independent swaplet {}",
            wire.pv,
            independent
        );
    }

    /// BOND ORACLE — an independent discount-factor-space reprice. On a flat
    /// continuously-compounded curve `DF(t) = exp(-z·t)` exactly, so we can reprice
    /// the wrapped `price_from_curve` dirty price and the analytic `bond_risk` DV01
    /// from first principles (re-derived here, never re-running the engine): the
    /// dirty price equals `Σ CF_k · exp(-z·t_k)`, and the yield DV01 equals the
    /// closed-form `(1/f) Σ e_k CF_k (1+y/f)^(−e_k−1) · 1bp`. Both <= 1e-12.
    #[test]
    fn bond_pv_and_dv01_match_independent_df_space_reprice() {
        use celnet_calendar::year_fraction;
        use celnet_rates::Curve;
        use celnet_types::Time;

        // A clean 3y 6% semi-annual bond settling ON a coupon anniversary (no stub,
        // zero accrued, whole periods) — six semi-annual coupons on the 15th.
        let settlement = Date::from_calendar_date(2032, Month::June, 15).unwrap();
        let maturity = Date::from_calendar_date(2035, Month::June, 15).unwrap();
        let bond = Bond::new(
            settlement,
            maturity,
            0.06,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .unwrap();

        let z = 0.05_f64;
        let curve = Curve::from_zero_rates(&[
            (Time(1.0), Rate(z)),
            (Time(2.0), Rate(z)),
            (Time(3.0), Rate(z)),
        ])
        .unwrap();

        let dirty = price_from_curve(&bond, &curve).unwrap();
        let risk = bond_risk(&bond, dirty).unwrap();

        // Independent DF-space PV: discount each cashflow at exp(-z·t), t = ACT/365F
        // from settlement to the k-th semi-annual coupon date (month-step, on the
        // 15th so back-from-maturity == forward-from-settlement for this clean bond).
        let coupon = 0.06 / 2.0 * 100.0;
        let mut expected_pv = 0.0_f64;
        for k in 1..=6 {
            let cpn_date = add_months(settlement, 6 * k);
            let t = year_fraction(DayCount::Act365Fixed, settlement, cpn_date).0;
            let cf = coupon + if k == 6 { 100.0 } else { 0.0 };
            expected_pv += cf * (-z * t).exp();
        }
        assert!(
            (dirty - expected_pv).abs() <= 1e-12,
            "bond dirty price {dirty} vs DF-space reprice {expected_pv}"
        );

        // Independent analytic yield DV01: -P'(y)·1bp, P'(y) closed-form; on a coupon
        // date the k-th cashflow's period exponent e_k = k (w = 1).
        let y = risk.yield_to_maturity.0;
        let f = 2.0_f64;
        let mut neg_p_prime = 0.0_f64;
        for k in 1..=6 {
            let e = f64::from(k);
            let cf = coupon + if k == 6 { 100.0 } else { 0.0 };
            neg_p_prime += (e / f) * cf * (1.0 + y / f).powf(-e - 1.0);
        }
        let expected_dv01 = neg_p_prime * 1e-4;
        assert!(
            (risk.dv01 - expected_dv01).abs() <= 1e-12,
            "bond dv01 {} vs analytic {}",
            risk.dv01,
            expected_dv01
        );
    }

    /// The bond wire arm reproduces the wrapped engine exactly: `price_rates` (Bond
    /// arm, long) is `to_bits`-identical to `price_from_curve` + `bond_risk` on the
    /// same OIS-bootstrapped curve — the dispatch adds nothing. The short side is the
    /// exact negation of the long PV / PV01 / DV01; the yield (par) is side-independent.
    #[test]
    fn bond_wire_reproduces_engine_exactly() {
        let reference = resolve_date(&reference()).unwrap();
        let quotes = build_quotes(&curve_set(), reference).unwrap();
        let curve = bootstrap_ois(&quotes).unwrap();
        let cal = us_settlement_calendar();
        let settlement = RollRule::Following.adjust(&cal, reference);
        let maturity = Date::from_calendar_date(2031, Month::June, 15).unwrap();
        let bond = Bond::new(
            settlement,
            maturity,
            0.05,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .unwrap();
        let dirty = price_from_curve(&bond, &curve).unwrap();
        let risk = bond_risk(&bond, dirty).unwrap();

        let long = price_rates(&bond_request(0.05, (2031, 6, 15), 100.0, Side::Buy)).unwrap();
        assert_eq!(long.pv.to_bits(), dirty.to_bits());
        assert_eq!(long.pv01.to_bits(), risk.dv01.to_bits());
        assert_eq!(long.dv01.to_bits(), risk.dv01.to_bits());
        assert_eq!(long.par_rate.to_bits(), risk.yield_to_maturity.0.to_bits());
        assert!(long.key_rate_ladder.is_empty());

        let short = price_rates(&bond_request(0.05, (2031, 6, 15), 100.0, Side::Sell)).unwrap();
        assert_eq!(short.pv, -long.pv);
        assert_eq!(short.pv01, -long.pv01);
        assert_eq!(short.dv01, -long.dv01);
        assert_eq!(short.par_rate, long.par_rate); // yield is side-independent
    }

    /// [`quote_bond`] (the FIX-edge quoting view) is `to_bits`-identical to the landed
    /// [`price_rates`] `Bond` arm on every shared number: its `dirty_price` equals the
    /// long PV, its `dv01` equals the long DV01, and its `yield_to_maturity` equals the
    /// par (yield). It additionally exposes `clean = dirty − accrued` (the internal
    /// consistency of the wrapped `bond_risk`) and prices a two-way request at magnitude
    /// where [`price_rates`] rejects `SIDE_TWO_WAY`.
    #[test]
    fn quote_bond_matches_price_rates() {
        let bond = BondInstrument {
            coupon_rate: 0.05,
            coupon_frequency: WirePaymentFrequency::SemiAnnual as i32,
            day_count: WireAccrualBasis::Thirty360BondBasis as i32,
            maturity_date: Some(BrokenDate {
                year: 2031,
                month: 6,
                day: 15,
            }),
            redemption: 100.0,
            side: Side::TwoWay as i32, // a two-way request price_rates would reject
        };

        let quote = quote_bond(&bond, &curve_set()).expect("two-way bond quotes at magnitude");
        let long = price_rates(&bond_request(0.05, (2031, 6, 15), 100.0, Side::Buy)).unwrap();

        assert_eq!(quote.dirty_price.to_bits(), long.pv.to_bits());
        assert_eq!(quote.dv01.to_bits(), long.dv01.to_bits());
        assert_eq!(quote.yield_to_maturity.to_bits(), long.par_rate.to_bits());
        assert!(
            (quote.clean_price + quote.accrued_interest - quote.dirty_price).abs() <= 1e-12,
            "clean {} + accrued {} != dirty {}",
            quote.clean_price,
            quote.accrued_interest,
            quote.dirty_price
        );
        assert!(quote.dv01 > 0.0, "a long-magnitude bond DV01 is positive");
    }

    /// The IRS / FRA key-rate ladders carry one entry per calibrating pillar and sum
    /// to the parallel DV01 to first order (the additive Jacobian completeness the
    /// wrapped `swap_risk` / `fra_risk` guarantee, passed through the wire unchanged).
    #[test]
    fn irs_and_fra_key_rate_ladders_are_pillar_shaped() {
        let irs = price_rates(&irs_request(7, 0.041, 100_000_000.0, Side::Sell)).unwrap();
        assert_eq!(irs.key_rate_ladder.len(), pillars().len());
        let irs_sum: f64 = irs.key_rate_ladder.iter().sum();
        assert!((irs_sum - irs.dv01).abs() / irs.dv01.abs() < 5e-3);

        let fra = price_rates(&fra_request(3, 6, 0.033, 25_000_000.0, Side::Sell)).unwrap();
        assert_eq!(fra.key_rate_ladder.len(), pillars().len());
        let fra_sum: f64 = fra.key_rate_ladder.iter().sum();
        assert!((fra_sum - fra.dv01).abs() / fra.dv01.abs() < 5e-3);
    }

    /// Malformed new-arm inputs are typed errors, never a silent default: a
    /// non-increasing FRA window, a zero-tenor IRS, a two-way side, and a bond with
    /// no maturity each surface their dedicated [`RatesPriceError`].
    #[test]
    fn new_arms_reject_malformed_input() {
        assert_eq!(
            price_rates(&fra_request(6, 3, 0.03, 1.0, Side::Sell)),
            Err(RatesPriceError::NonIncreasingFraWindow)
        );
        assert_eq!(
            price_rates(&irs_request(0, 0.04, 1.0, Side::Sell)),
            Err(RatesPriceError::ZeroTenor)
        );
        assert_eq!(
            price_rates(&irs_request(5, 0.04, 1.0, Side::TwoWay)),
            Err(RatesPriceError::InvalidSide)
        );
        let mut no_maturity = bond_request(0.05, (2031, 6, 15), 100.0, Side::Buy);
        if let Some(rates_instrument::Instrument::Bond(b)) = no_maturity
            .instrument
            .as_mut()
            .and_then(|i| i.instrument.as_mut())
        {
            b.maturity_date = None;
        }
        assert_eq!(
            price_rates(&no_maturity),
            Err(RatesPriceError::MissingBondMaturity)
        );
    }
}
