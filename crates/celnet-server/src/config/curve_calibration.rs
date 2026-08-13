//! Resolution of reference-data instrument **definitions** into
//! [`celnet_rates::CalibrationInstrument`]s ready for [`celnet_rates::bootstrap_curve`].
//!
//! The reference-data registry ([`super::reference_data`]) stores instruments as
//! convention *blocks* keyed by string labels (day-counts, roll conventions, calendar centres,
//! payment frequencies) plus tenor / broken-date tokens. The bootstrap, by contrast, consumes
//! *resolved* instruments whose schedules already live on the curve's continuous ACT/365F time
//! axis. This module is the bridge: it resolves the labels to the engine enums (via the existing
//! [`super::reference_data`] label helpers), turns tenor / broken-date tokens into
//! calendar-adjusted dates, and assembles each family's schedule on the curve's time axis using
//! the **same** conventions the live OIS handler ([`crate::rates_pricing`]) and
//! [`celnet_rates::schedule`] use (ACT/365F time coordinates from the spot origin; period ends
//! rolled by the instrument's business-day convention).
//!
//! ## Curve origin (Time 0)
//!
//! Every instrument in one calibration set shares a single discount origin — the curve's **spot**
//! date. Each instrument's spot is `value_date` advanced by its own `spot_lag_days` business days
//! on its settlement calendar; a coherent set therefore uses one consistent spot lag + calendar so
//! all instruments anchor at the same Time 0 (the spot-starting single-curve convention the
//! sequential bootstrap requires). Listed STIR futures carry no settlement lag — their fixing
//! windows are absolute calendar windows measured from `value_date`.
//!
//! ## Scope
//!
//! A [`super::reference_data::InstrumentFamily::Bond`] is a cash instrument, not a curve pillar;
//! it is rejected with a typed error. Period generation honours the regular (`none`) roll
//! convention; end-of-month / IMM swap-period generation is out of this increment's scope and is
//! rejected explicitly rather than silently mis-generated.

use celnet_calendar::{BusinessCalendar, CentreId, RollRule, add_months, year_fraction};
use celnet_rates::{
    AccrualBasis, CalibrationInstrument, Deposit, DepositError, FixedPeriod, Fra, FraError,
    FutureError, LegPeriod, OisQuote, OisSchedule, PaymentFrequency, ScheduleError, StirFuture,
    StirFuturesQuote, SwapError, SwapLeg, VanillaIrsQuote,
};
use celnet_types::{DayCount, Rate, Time};
use time::{Date, Duration, Month};

use super::reference_data::{
    DepositDef, FraDef, InstrumentDef, InstrumentFamily, OisDef, StirFutureDef, VanillaIrsDef,
    accrual_basis_from_label, centre_from_label, payment_frequency_from_label,
    roll_rule_from_label,
};

/// A typed failure of reference-data → calibration-instrument resolution.
///
/// Every variant is a *static* (input-shape) failure: an unknown label, an unresolvable tenor, a
/// non-calibratable family, or a schedule/instrument constructor rejecting the resolved
/// coordinates. None is a numeric (bootstrap) failure — that happens downstream in
/// [`celnet_rates::bootstrap_curve`].
#[derive(Debug, Clone, PartialEq)]
pub enum CurveCalibrationError {
    /// The instrument's family is not a curve-calibration instrument (e.g. a cash bond).
    NotCalibratable {
        /// The offending instrument's registry id.
        instrument_id: String,
        /// The family kind label (e.g. `bond`).
        family: &'static str,
    },
    /// A tenor / broken-date token could not be resolved to a calendar date.
    UnresolvableTenor(String),
    /// A convention label did not map to any engine enum.
    UnknownConvention {
        /// The reference-data field whose label was unknown.
        field: &'static str,
        /// The unrecognised label value.
        label: String,
    },
    /// A period-generation roll convention this increment does not implement (`eom` / `imm`).
    UnsupportedRollConvention(String),
    /// An instrument listed no settlement-calendar centres.
    NoCalendars {
        /// The offending instrument's registry id.
        instrument_id: String,
    },
    /// A settlement-calendar centre label did not map to a known centre.
    UnknownCalendar(String),
    /// A calibration set mixed pricing currencies (the bootstrap is single-currency).
    CurrencyMismatch {
        /// The first currency seen in the set.
        expected: String,
        /// The conflicting currency.
        found: String,
    },
    /// A money-market deposit could not be constructed from its resolved coordinates.
    Deposit(DepositError),
    /// A forward rate agreement could not be constructed from its resolved coordinates.
    Fra(FraError),
    /// A STIR future could not be constructed from its resolved fixing window.
    Future(FutureError),
    /// An OIS fixed-leg schedule could not be constructed from its resolved coordinates.
    Schedule(ScheduleError),
    /// A swap leg could not be constructed from its resolved coordinates.
    SwapLeg(SwapError),
}

impl core::fmt::Display for CurveCalibrationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotCalibratable {
                instrument_id,
                family,
            } => write!(
                f,
                "instrument `{instrument_id}` is a {family}, which is not a curve-calibration instrument"
            ),
            Self::UnresolvableTenor(t) => write!(f, "unresolvable tenor token `{t}`"),
            Self::UnknownConvention { field, label } => {
                write!(f, "unknown {field} convention label `{label}`")
            }
            Self::UnsupportedRollConvention(r) => write!(
                f,
                "period-generation roll convention `{r}` is not supported (use `none`)"
            ),
            Self::NoCalendars { instrument_id } => {
                write!(
                    f,
                    "instrument `{instrument_id}` lists no settlement calendars"
                )
            }
            Self::UnknownCalendar(c) => write!(f, "unknown settlement-calendar centre `{c}`"),
            Self::CurrencyMismatch { expected, found } => write!(
                f,
                "calibration set mixes currencies `{expected}` and `{found}`"
            ),
            Self::Deposit(e) => write!(f, "deposit: {e}"),
            Self::Fra(e) => write!(f, "fra: {e}"),
            Self::Future(e) => write!(f, "future: {e}"),
            Self::Schedule(e) => write!(f, "schedule: {e}"),
            Self::SwapLeg(e) => write!(f, "swap leg: {e}"),
        }
    }
}

impl std::error::Error for CurveCalibrationError {}

impl From<DepositError> for CurveCalibrationError {
    fn from(e: DepositError) -> Self {
        Self::Deposit(e)
    }
}
impl From<FraError> for CurveCalibrationError {
    fn from(e: FraError) -> Self {
        Self::Fra(e)
    }
}
impl From<FutureError> for CurveCalibrationError {
    fn from(e: FutureError) -> Self {
        Self::Future(e)
    }
}
impl From<ScheduleError> for CurveCalibrationError {
    fn from(e: ScheduleError) -> Self {
        Self::Schedule(e)
    }
}
impl From<SwapError> for CurveCalibrationError {
    fn from(e: SwapError) -> Self {
        Self::SwapLeg(e)
    }
}

/// Resolve a single reference-data definition + observed `quote` into a calibration instrument.
///
/// `quote` is the instrument's market observable in decimal rate terms: the deposit/FRA/par-swap
/// fixed rate, or a STIR future's `(100 − price) / 100` futures rate. `value_date` is the curve's
/// trade date; the instrument's spot (Time 0) is `value_date + spot_lag_days` business days.
///
/// # Errors
///
/// Returns a [`CurveCalibrationError`] if the family is not a curve pillar (a bond), a convention
/// label or tenor token is unresolvable, or the resolved coordinates are rejected by the
/// instrument/schedule constructor.
pub fn calibration_instrument(
    def: &InstrumentDef,
    quote: f64,
    value_date: Date,
) -> Result<CalibrationInstrument, CurveCalibrationError> {
    match &def.definition {
        InstrumentFamily::Deposit(d) => {
            deposit_instrument(d, quote, value_date, &def.instrument_id)
        }
        InstrumentFamily::Fra(d) => fra_instrument(d, quote, value_date, &def.instrument_id),
        InstrumentFamily::StirFuture(d) => {
            stir_future_instrument(d, quote, value_date, &def.instrument_id)
        }
        InstrumentFamily::Ois(d) => ois_instrument(d, quote, value_date, &def.instrument_id),
        InstrumentFamily::VanillaIrs(d) => {
            vanilla_irs_instrument(d, quote, value_date, &def.instrument_id)
        }
        // A listed bond future is a hedge vehicle, not a curve pillar: it is priced
        // off the curve (through its deliverable), not used to build one.
        InstrumentFamily::BondFuture(_) | InstrumentFamily::Bond(_) => {
            Err(CurveCalibrationError::NotCalibratable {
                instrument_id: def.instrument_id.clone(),
                family: def.definition.kind(),
            })
        }
    }
}

/// Resolve a set of `(definition, quote)` pairs into a calibration ladder ready for
/// [`celnet_rates::bootstrap_curve`].
///
/// The set must be single-currency (the bootstrap builds one discount curve). Maturity ordering
/// and strict monotonicity are enforced downstream by the bootstrap; this function preserves the
/// caller's order.
///
/// # Errors
///
/// Returns [`CurveCalibrationError::CurrencyMismatch`] if the set mixes currencies, or propagates
/// the first per-instrument resolution failure.
pub fn calibration_set(
    defs: &[(&InstrumentDef, f64)],
    value_date: Date,
) -> Result<Vec<CalibrationInstrument>, CurveCalibrationError> {
    let mut expected: Option<&str> = None;
    for (def, _) in defs {
        match expected {
            None => expected = Some(&def.currency),
            Some(c) if c.eq_ignore_ascii_case(&def.currency) => {}
            Some(c) => {
                return Err(CurveCalibrationError::CurrencyMismatch {
                    expected: c.to_string(),
                    found: def.currency.clone(),
                });
            }
        }
    }
    defs.iter()
        .map(|(def, quote)| calibration_instrument(def, *quote, value_date))
        .collect()
}

/// Resolve a standalone date-anchored pillar into a synthetic money-market cash deposit.
///
/// The deposit runs from the curve's `reference_date` (Time 0) to `maturity_date`, pinning the
/// closed-form pillar `DF = 1/(1 + r·τ)` with ACT/360 accrual (the USD money-market default) and
/// `quote` as its simple rate. This is the same front-pillar the bootstrap places for a registry
/// deposit, so a curve can be pinned to an arbitrary date (a turn, an IMM, a meeting date) without
/// a registry instrument maturing there. Calibration is scale-invariant, so the notional is implicit.
///
/// # Errors
///
/// Returns [`CurveCalibrationError::Deposit`] if `maturity_date <= reference_date` (a non-positive
/// curve maturity the deposit constructor rejects).
pub fn date_pillar_instrument(
    reference_date: Date,
    maturity_date: Date,
    quote: f64,
) -> Result<CalibrationInstrument, CurveCalibrationError> {
    let deposit = Deposit::from_dates(
        reference_date,
        maturity_date,
        AccrualBasis::Act360,
        Rate(quote),
    )?;
    Ok(CalibrationInstrument::Deposit(deposit))
}

// ---------------------------------------------------------------------------------------------
// Convention / calendar / tenor resolution helpers.
// ---------------------------------------------------------------------------------------------

/// Build a settlement calendar from the instrument's centre labels.
fn build_calendar(
    labels: &[String],
    instrument_id: &str,
) -> Result<BusinessCalendar, CurveCalibrationError> {
    if labels.is_empty() {
        return Err(CurveCalibrationError::NoCalendars {
            instrument_id: instrument_id.to_string(),
        });
    }
    let mut centres: Vec<CentreId> = Vec::with_capacity(labels.len());
    for label in labels {
        let centre = centre_from_label(label)
            .ok_or_else(|| CurveCalibrationError::UnknownCalendar(label.clone()))?;
        centres.push(centre);
    }
    Ok(BusinessCalendar::with_centres(centres))
}

/// Map an accrual-basis label, attributing the field on failure.
fn accrual_basis(label: &str, field: &'static str) -> Result<AccrualBasis, CurveCalibrationError> {
    accrual_basis_from_label(label).ok_or_else(|| CurveCalibrationError::UnknownConvention {
        field,
        label: label.to_string(),
    })
}

/// Map a business-day / roll-rule label.
fn roll_rule(label: &str) -> Result<RollRule, CurveCalibrationError> {
    roll_rule_from_label(label).ok_or_else(|| CurveCalibrationError::UnknownConvention {
        field: "business_day_convention",
        label: label.to_string(),
    })
}

/// Map a payment-frequency label, attributing the field on failure.
fn payment_frequency(
    label: &str,
    field: &'static str,
) -> Result<PaymentFrequency, CurveCalibrationError> {
    payment_frequency_from_label(label).ok_or_else(|| CurveCalibrationError::UnknownConvention {
        field,
        label: label.to_string(),
    })
}

/// Resolve a tenor / broken-date token to a calendar-adjusted date.
///
/// Supported forms (case-insensitive):
/// - `ON` / `TN` / `SN` — 1 / 2 / 1 business days after the spot (`value_date + spot_lag`);
/// - `<n>D` / `<n>W` / `<n>M` / `<n>Y` — calendar offset from the spot, then `roll`-adjusted;
/// - an ISO `YYYY-MM-DD` broken date — parsed, then `roll`-adjusted (the curves slice-A path).
///
/// # Errors
///
/// Returns [`CurveCalibrationError::UnresolvableTenor`] for any token that matches none of these.
fn tenor_to_date(
    token: &str,
    value_date: Date,
    spot_lag_days: u32,
    roll: RollRule,
    cal: &BusinessCalendar,
) -> Result<Date, CurveCalibrationError> {
    let spot = cal.add_business_days(value_date, spot_lag_days);
    let upper = token.trim().to_ascii_uppercase();

    // Broken ISO date (contains a `-` separator).
    if upper.contains('-') {
        return parse_iso_date(&upper)
            .map(|d| roll.adjust(cal, d))
            .ok_or_else(|| CurveCalibrationError::UnresolvableTenor(token.to_string()));
    }

    // Spot-relative overnight tokens (already land on business days).
    match upper.as_str() {
        "ON" | "SN" => return Ok(roll.adjust(cal, cal.add_business_days(spot, 1))),
        "TN" => return Ok(roll.adjust(cal, cal.add_business_days(spot, 2))),
        _ => {}
    }

    // `<n><unit>` numeric tenor.
    let (num_str, unit) = upper.split_at(upper.len().saturating_sub(1));
    let n: i32 = num_str
        .parse()
        .map_err(|_| CurveCalibrationError::UnresolvableTenor(token.to_string()))?;
    let target = match unit {
        "D" => spot + Duration::days(i64::from(n)),
        "W" => spot + Duration::weeks(i64::from(n)),
        "M" => add_months(spot, n),
        "Y" => add_months(spot, n * 12),
        _ => return Err(CurveCalibrationError::UnresolvableTenor(token.to_string())),
    };
    Ok(roll.adjust(cal, target))
}

/// Parse an ISO `YYYY-MM-DD` token to a civil date, or `None` if it is not one.
fn parse_iso_date(token: &str) -> Option<Date> {
    let mut parts = token.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let month = Month::try_from(month).ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

/// The curve-axis (ACT/365F) time coordinate of `date` measured from the spot origin.
fn date_to_time(spot: Date, date: Date) -> Time {
    year_fraction(DayCount::Act365Fixed, spot, date)
}

// ---------------------------------------------------------------------------------------------
// Per-family resolution.
// ---------------------------------------------------------------------------------------------

/// Resolve a money-market cash deposit (closed-form front pillar `1/(1 + r·τ)`).
fn deposit_instrument(
    def: &DepositDef,
    quote: f64,
    value_date: Date,
    instrument_id: &str,
) -> Result<CalibrationInstrument, CurveCalibrationError> {
    let cal = build_calendar(&def.calendars, instrument_id)?;
    let roll = roll_rule(&def.business_day_convention)?;
    let basis = accrual_basis(&def.day_count, "day_count")?;
    let spot = cal.add_business_days(value_date, def.spot_lag_days);
    let maturity = tenor_to_date(&def.tenor, value_date, def.spot_lag_days, roll, &cal)?;
    let deposit = Deposit::from_dates(spot, maturity, basis, Rate(quote))?;
    Ok(CalibrationInstrument::Deposit(deposit))
}

/// Resolve a single-period forward rate agreement (notional defaults to 1; calibration is
/// scale-invariant).
fn fra_instrument(
    def: &FraDef,
    quote: f64,
    value_date: Date,
    instrument_id: &str,
) -> Result<CalibrationInstrument, CurveCalibrationError> {
    let cal = build_calendar(&def.calendars, instrument_id)?;
    let roll = roll_rule(&def.business_day_convention)?;
    let basis = accrual_basis(&def.accrual_day_count, "accrual_day_count")?;
    let fixing = tenor_to_date(&def.start_tenor, value_date, def.spot_lag_days, roll, &cal)?;
    let maturity = tenor_to_date(&def.end_tenor, value_date, def.spot_lag_days, roll, &cal)?;
    let spot = cal.add_business_days(value_date, def.spot_lag_days);
    let fra = Fra::from_dates(spot, fixing, maturity, basis, Rate(quote), 1.0)?;
    Ok(CalibrationInstrument::Fra(fra))
}

/// Resolve a listed STIR future (closed-form forward pillar; convexity volatility passed through).
///
/// A listed future carries no settlement lag — its fixing window is an absolute calendar window
/// measured from `value_date`. `contract_size` is not needed for rate calibration, and `day_count`
/// does not enter the (ACT/365F) curve-axis coordinates.
fn stir_future_instrument(
    def: &StirFutureDef,
    quote: f64,
    value_date: Date,
    instrument_id: &str,
) -> Result<CalibrationInstrument, CurveCalibrationError> {
    let cal = build_calendar(&def.calendars, instrument_id)?;
    let roll = RollRule::ModifiedFollowing;
    let start = tenor_to_date(&def.reference_start, value_date, 0, roll, &cal)?;
    let end = tenor_to_date(&def.reference_end, value_date, 0, roll, &cal)?;
    let future = StirFuture::new(
        date_to_time(value_date, start),
        date_to_time(value_date, end),
    )?;
    Ok(CalibrationInstrument::StirFuture(StirFuturesQuote {
        future,
        futures_rate: Rate(quote),
        convexity_vol: def.convexity_vol,
    }))
}

/// Resolve a spot-starting overnight-index swap (par fixed rate root-solved).
///
/// Only the fixed-leg schedule (frequency + day-count) enters the par-rate calibration; the
/// compounded-overnight float leg is implied by the curve, so `float_day_count` and `index` are
/// not required to form the [`OisQuote`].
fn ois_instrument(
    def: &OisDef,
    quote: f64,
    value_date: Date,
    instrument_id: &str,
) -> Result<CalibrationInstrument, CurveCalibrationError> {
    let cal = build_calendar(&def.calendars, instrument_id)?;
    let roll = roll_rule(&def.business_day_convention)?;
    let basis = accrual_basis(&def.fixed_day_count, "fixed_day_count")?;
    let freq = payment_frequency(&def.fixed_frequency, "fixed_frequency")?;
    let spot = cal.add_business_days(value_date, def.spot_lag_days);
    let maturity = tenor_to_date(&def.tenor, value_date, def.spot_lag_days, roll, &cal)?;
    let schedule = fixed_ois_schedule(spot, maturity, freq, basis, roll, &cal)?;
    Ok(CalibrationInstrument::Ois(OisQuote {
        schedule,
        par_rate: Rate(quote),
    }))
}

/// Resolve a coterminal vanilla fixed-vs-float IRS (par fixed rate root-solved).
fn vanilla_irs_instrument(
    def: &VanillaIrsDef,
    quote: f64,
    value_date: Date,
    instrument_id: &str,
) -> Result<CalibrationInstrument, CurveCalibrationError> {
    let roll_conv = def.roll_convention.trim().to_ascii_lowercase();
    if !roll_conv.is_empty() && roll_conv != "none" {
        return Err(CurveCalibrationError::UnsupportedRollConvention(
            def.roll_convention.clone(),
        ));
    }
    let cal = build_calendar(&def.calendars, instrument_id)?;
    let roll = roll_rule(&def.business_day_convention)?;
    let fixed_basis = accrual_basis(&def.fixed_day_count, "fixed_day_count")?;
    let fixed_freq = payment_frequency(&def.fixed_frequency, "fixed_frequency")?;
    let float_basis = accrual_basis(&def.float_day_count, "float_day_count")?;
    let float_freq = payment_frequency(&def.float_frequency, "float_frequency")?;
    let spot = cal.add_business_days(value_date, def.spot_lag_days);
    let maturity = tenor_to_date(&def.tenor, value_date, def.spot_lag_days, roll, &cal)?;
    let fixed_leg = swap_leg(spot, maturity, fixed_freq, fixed_basis, roll, &cal)?;
    let float_leg = swap_leg(spot, maturity, float_freq, float_basis, roll, &cal)?;
    Ok(CalibrationInstrument::VanillaIrs(VanillaIrsQuote {
        fixed_leg,
        float_leg,
        par_rate: Rate(quote),
    }))
}

// ---------------------------------------------------------------------------------------------
// Generic schedule generation on the curve time axis.
//
// Both builders mirror `celnet_rates::schedule` / `vanilla_swap::swap_leg_schedule`: full
// `freq`-spaced periods roll by the instrument's business-day convention up to (but strictly
// before) the maturity, then a final maturity stub; accrual fractions use the leg's day-count,
// while the accrual-start / payment time coordinates are ACT/365F from the spot origin.
// ---------------------------------------------------------------------------------------------

/// Compute the rolled period-end dates: full `freq`-spaced rolls strictly before `maturity`,
/// then the maturity stub.
fn period_ends(
    spot: Date,
    maturity: Date,
    freq: PaymentFrequency,
    roll: RollRule,
    cal: &BusinessCalendar,
) -> Vec<Date> {
    let mut ends = Vec::new();
    let mut step = 1i32;
    loop {
        let end = roll.adjust(cal, add_months(spot, step * freq.months()));
        if end < maturity {
            ends.push(end);
            step += 1;
        } else {
            break;
        }
    }
    ends.push(maturity);
    ends
}

/// Build an OIS fixed-leg schedule honouring `freq` and the fixed-leg `basis`.
fn fixed_ois_schedule(
    spot: Date,
    maturity: Date,
    freq: PaymentFrequency,
    basis: AccrualBasis,
    roll: RollRule,
    cal: &BusinessCalendar,
) -> Result<OisSchedule, CurveCalibrationError> {
    let ends = period_ends(spot, maturity, freq, roll, cal);
    let mut periods = Vec::with_capacity(ends.len());
    let mut prev = spot;
    for end in ends {
        periods.push(FixedPeriod {
            pay: date_to_time(spot, end),
            accrual: basis.year_fraction(prev, end),
        });
        prev = end;
    }
    Ok(OisSchedule::new(Time(0.0), periods)?)
}

/// Build a swap leg honouring `freq` and the leg's `basis`, coterminal at `maturity`.
fn swap_leg(
    spot: Date,
    maturity: Date,
    freq: PaymentFrequency,
    basis: AccrualBasis,
    roll: RollRule,
    cal: &BusinessCalendar,
) -> Result<SwapLeg, CurveCalibrationError> {
    let ends = period_ends(spot, maturity, freq, roll, cal);
    let mut periods = Vec::with_capacity(ends.len());
    let mut prev = spot;
    for end in ends {
        periods.push(LegPeriod {
            accrual_start: date_to_time(spot, prev),
            pay: date_to_time(spot, end),
            accrual: basis.year_fraction(prev, end),
        });
        prev = end;
    }
    Ok(SwapLeg::new(Time(0.0), periods)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_rates::{
        Curve, bootstrap_curve, deposit_discount_factor, deposit_par_rate, fixed_annuity,
        float_leg_value, fra_par_rate, implied_forward_rate, ois_par_rate,
    };

    /// A US business day used as the curve trade/spot date (Monday 16 Jun 2025).
    fn value_date() -> Date {
        Date::from_calendar_date(2025, Month::June, 16).expect("valid date")
    }

    fn instrument(id: &str, family: InstrumentFamily) -> InstrumentDef {
        InstrumentDef {
            sub_asset_type: String::new(),
            region: String::new(),
            instrument_id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            currency: "USD".to_string(),
            external_ids: Vec::new(),
            definition: family,
        }
    }

    fn deposit_def(id: &str, tenor: &str) -> InstrumentDef {
        instrument(
            id,
            InstrumentFamily::Deposit(DepositDef {
                index: "sofr".to_string(),
                tenor: tenor.to_string(),
                day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                spot_lag_days: 0,
            }),
        )
    }

    fn future_def(id: &str, start: &str, end: &str) -> InstrumentDef {
        instrument(
            id,
            InstrumentFamily::StirFuture(StirFutureDef {
                contract_code: id.to_string(),
                reference_start: start.to_string(),
                reference_end: end.to_string(),
                day_count: "act_360".to_string(),
                calendars: vec!["united_states".to_string()],
                convexity_vol: 0.0075,
                contract_size: 2_500_000.0,
            }),
        )
    }

    fn fra_def(id: &str, start: &str, end: &str) -> InstrumentDef {
        instrument(
            id,
            InstrumentFamily::Fra(FraDef {
                float_index: "sofr".to_string(),
                start_tenor: start.to_string(),
                end_tenor: end.to_string(),
                accrual_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                spot_lag_days: 0,
            }),
        )
    }

    fn ois_def(id: &str, tenor: &str) -> InstrumentDef {
        instrument(
            id,
            InstrumentFamily::Ois(OisDef {
                tenor: tenor.to_string(),
                index: "sofr".to_string(),
                fixed_frequency: "annual".to_string(),
                fixed_day_count: "act_360".to_string(),
                float_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                spot_lag_days: 0,
            }),
        )
    }

    fn irs_def(id: &str, tenor: &str) -> InstrumentDef {
        instrument(
            id,
            InstrumentFamily::VanillaIrs(VanillaIrsDef {
                tenor: tenor.to_string(),
                fixed_frequency: "semi_annual".to_string(),
                fixed_day_count: "act_360".to_string(),
                float_index: "sofr".to_string(),
                float_frequency: "quarterly".to_string(),
                float_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                roll_convention: "none".to_string(),
                spot_lag_days: 0,
            }),
        )
    }

    /// The acceptance gate: a realistic USD mixed ladder resolved from reference-data definitions
    /// bootstraps, and every calibrating instrument reprices to its own quote (model par within
    /// 1e-8) with pillar discount factors consistent within 1e-10 — the inc.1 gate, but driven
    /// through the reference-data resolver.
    #[test]
    fn reference_data_ladder_bootstraps_and_reprices_every_instrument_to_par() {
        let deposits = [
            (deposit_def("usd-depo-1w", "1W"), 0.0431),
            (deposit_def("usd-depo-1m", "1M"), 0.0432),
            (deposit_def("usd-depo-3m", "3M"), 0.0433),
        ];
        let future = (future_def("usd-sr3-1", "3M", "6M"), 0.0436);
        let fra = (fra_def("usd-fra-6x9", "6M", "9M"), 0.0442);
        let oises = [
            (ois_def("usd-ois-1y", "1Y"), 0.0420),
            (ois_def("usd-ois-2y", "2Y"), 0.0418),
        ];
        let irses = [
            (irs_def("usd-irs-5y", "5Y"), 0.0415),
            (irs_def("usd-irs-10y", "10Y"), 0.0425),
        ];

        let mut defs: Vec<(&InstrumentDef, f64)> = Vec::new();
        for (d, q) in &deposits {
            defs.push((d, *q));
        }
        defs.push((&future.0, future.1));
        defs.push((&fra.0, fra.1));
        for (d, q) in &oises {
            defs.push((d, *q));
        }
        for (d, q) in &irses {
            defs.push((d, *q));
        }

        let ladder = calibration_set(&defs, value_date()).expect("resolves");
        let curve: Curve = bootstrap_curve(&ladder).expect("bootstraps");

        let mut worst_rate = 0.0_f64;
        let mut worst_df = 0.0_f64;
        for inst in &ladder {
            match inst {
                CalibrationInstrument::Deposit(d) => {
                    worst_rate = worst_rate.max((deposit_par_rate(&curve, d).0 - d.rate.0).abs());
                    worst_df = worst_df.max(
                        (curve.discount_factor(d.maturity).0 - deposit_discount_factor(d).0).abs(),
                    );
                }
                CalibrationInstrument::StirFuture(q) => {
                    let model = curve
                        .forward_rate_simple(q.future.fixing_start(), q.future.fixing_end())
                        .0;
                    worst_rate = worst_rate.max((model - implied_forward_rate(q).0).abs());
                }
                CalibrationInstrument::Fra(f) => {
                    worst_rate = worst_rate.max((fra_par_rate(&curve, f).0 - f.fixed_rate.0).abs());
                }
                CalibrationInstrument::Ois(q) => {
                    worst_rate =
                        worst_rate.max((ois_par_rate(&curve, &q.schedule).0 - q.par_rate.0).abs());
                }
                CalibrationInstrument::VanillaIrs(q) => {
                    let model =
                        float_leg_value(&curve, &q.float_leg) / fixed_annuity(&curve, &q.fixed_leg);
                    worst_rate = worst_rate.max((model - q.par_rate.0).abs());
                }
            }
        }

        assert!(
            worst_rate < 1e-8,
            "worst reprice-to-par residual {worst_rate:e}"
        );
        assert!(worst_df < 1e-10, "worst deposit DF residual {worst_df:e}");
    }

    #[test]
    fn bond_id_is_rejected_as_non_calibratable() {
        use super::super::reference_data::{BondDef, CivilDate};
        let bond = instrument(
            "usd-bond-1",
            InstrumentFamily::Bond(BondDef {
                issuer: "US Treasury".to_string(),
                coupon_rate: 0.04,
                coupon_type: "fixed".to_string(),
                coupon_frequency: "semi_annual".to_string(),
                day_count: "thirty_360_bond_basis".to_string(),
                issue_date: None,
                dated_date: None,
                first_coupon_date: None,
                maturity_date: CivilDate {
                    year: 2035,
                    month: 6,
                    day: 16,
                },
                redemption: 100.0,
                calendars: vec!["united_states".to_string()],
            }),
        );
        let err = calibration_instrument(&bond, 0.04, value_date()).expect_err("bond rejected");
        assert_eq!(
            err,
            CurveCalibrationError::NotCalibratable {
                instrument_id: "usd-bond-1".to_string(),
                family: "bond",
            }
        );
    }

    #[test]
    fn tenor_tokens_resolve_to_calendar_adjusted_dates() {
        let cal = BusinessCalendar::with_centres([CentreId::UnitedStates]);
        let vd = value_date(); // Monday 16 Jun 2025
        let roll = RollRule::ModifiedFollowing;
        let d = |y, m, day| Date::from_calendar_date(y, m, day).expect("date");

        // Overnight: next business day after spot.
        assert_eq!(
            tenor_to_date("ON", vd, 0, roll, &cal).expect("ON"),
            d(2025, Month::June, 17)
        );
        // Calendar-week / month offsets, rolled to a business day.
        assert_eq!(
            tenor_to_date("1W", vd, 0, roll, &cal).expect("1W"),
            d(2025, Month::June, 23)
        );
        assert_eq!(
            tenor_to_date("1M", vd, 0, roll, &cal).expect("1M"),
            d(2025, Month::July, 16)
        );
        assert_eq!(
            tenor_to_date("3M", vd, 0, roll, &cal).expect("3M"),
            d(2025, Month::September, 16)
        );

        // Broken ISO date that is already a business day round-trips unchanged.
        let broken = tenor_to_date("2027-03-15", vd, 0, roll, &cal).expect("broken date");
        assert!(broken >= d(2027, Month::March, 15));
        assert!((broken - d(2027, Month::March, 15)).whole_days() <= 3);

        // A spot lag advances the anchor by business days: a lagged tenor resolves strictly later.
        let no_lag = tenor_to_date("1M", vd, 0, roll, &cal).expect("1M");
        let lagged = tenor_to_date("1M", vd, 2, roll, &cal).expect("1M+lag");
        assert!(lagged > no_lag, "spot lag must advance the resolved date");

        // An unparseable token is a typed error.
        assert!(matches!(
            tenor_to_date("wibble", vd, 0, roll, &cal),
            Err(CurveCalibrationError::UnresolvableTenor(_))
        ));
    }

    #[test]
    fn date_pillar_resolves_to_a_deposit_that_reprices_to_par() {
        let vd = value_date(); // 2025-06-16
        let maturity = Date::from_calendar_date(2026, Month::December, 31).expect("date");
        let quote = 0.0415;
        let inst = date_pillar_instrument(vd, maturity, quote).expect("resolves");
        let curve: Curve =
            bootstrap_curve(core::slice::from_ref(&inst)).expect("single-pillar bootstrap");
        match &inst {
            CalibrationInstrument::Deposit(d) => {
                assert!(
                    (deposit_par_rate(&curve, d).0 - quote).abs() < 1e-8,
                    "date pillar deposit must reprice to its quote"
                );
                assert!(
                    (curve.discount_factor(d.maturity).0 - deposit_discount_factor(d).0).abs()
                        < 1e-10
                );
            }
            _ => panic!("a date pillar must resolve to a deposit"),
        }
    }

    #[test]
    fn date_pillar_with_non_positive_maturity_is_rejected() {
        let vd = value_date();
        // A maturity on (or before) the reference date is a non-positive curve maturity.
        let err =
            date_pillar_instrument(vd, vd, 0.04).expect_err("non-positive maturity is rejected");
        assert!(matches!(err, CurveCalibrationError::Deposit(_)));
    }
}
