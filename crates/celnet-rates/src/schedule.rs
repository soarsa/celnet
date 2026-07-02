//! USD-SOFR OIS schedule generation from market dates — the calendar/day-count layer.
//!
//! This is the bridge between calendar dates and the numeric [`OisSchedule`] the bootstrap and
//! pricer consume: it turns `(spot date, tenor in years)` into roll-adjusted annual fixed-leg
//! periods, mapping each onto the curve's continuous year-fraction time axis. The conventions are
//! the standard USD-SOFR OIS ones (`FI-CONVENTIONS.md`): annual fixed leg, **modified-following**
//! roll on the US settlement calendar, **ACT/360** fixed-leg accrual, and the payment's
//! discount-time coordinate measured **ACT/365F** from the spot date.
//!
//! The roll and day-count primitives are reused from `celnet-calendar` (the shared, independently
//! tested date engine) — not re-derived here.

use crate::daycount::AccrualBasis;
use crate::ois::{FixedPeriod, OisSchedule, ScheduleError};
use celnet_calendar::{BusinessCalendar, CentreId, RollRule, add_months, year_fraction};
use celnet_types::{DayCount, Time};
use time::Date;

/// The US settlement calendar used for USD-SOFR schedule rolls.
#[must_use]
pub fn us_settlement_calendar() -> BusinessCalendar {
    BusinessCalendar::single(CentreId::UnitedStates)
}

/// The roll-adjusted annual period-end dates of a `years`-year schedule starting at `start`.
///
/// Period `i` ends `12·i` calendar months after `start` (end-of-month-aware), rolled
/// **modified-following** onto a US business day.
pub(crate) fn period_end_dates(cal: &BusinessCalendar, start: Date, years: u32) -> Vec<Date> {
    (1..=years)
        .map(|i| RollRule::ModifiedFollowing.adjust(cal, add_months(start, (i as i32) * 12)))
        .collect()
}

/// Build a spot-starting USD-SOFR OIS fixed-leg schedule of `years` annual periods.
///
/// `reference` is the spot (settlement) date and becomes the schedule origin (curve time 0); it is
/// normalised to the next US business day if it is not already one. Each annual period end is
/// rolled modified-following on the US calendar; the fixed-leg accrual is ACT/360 and each
/// payment's discount-time coordinate is ACT/365F from the spot date.
///
/// # Errors
///
/// Returns [`ScheduleError::Empty`] if `years` is zero.
pub fn usd_sofr_ois_schedule(reference: Date, years: u32) -> Result<OisSchedule, ScheduleError> {
    usd_ois_schedule_with_basis(reference, years, AccrualBasis::Act360)
}

/// Build a spot-starting USD OIS fixed-leg schedule whose fixed-leg accrual uses
/// a caller-selected [`AccrualBasis`] (e.g. ACT/360 or 30/360 Bond Basis).
///
/// Identical to [`usd_sofr_ois_schedule`] except the per-period accrual fraction
/// is measured under `accrual_basis`; the payment discount-time coordinate stays
/// ACT/365F from the spot date (the curve time axis), and the roll calendar is
/// unchanged. This is the seam that makes the day-count convention selectable in
/// the curve bootstrap: an [`crate::bootstrap::OisQuote`] built from this
/// schedule reprices on whatever fixed-leg basis the instrument quotes.
///
/// # Errors
///
/// Returns [`ScheduleError::Empty`] if `years` is zero.
pub fn usd_ois_schedule_with_basis(
    reference: Date,
    years: u32,
    accrual_basis: AccrualBasis,
) -> Result<OisSchedule, ScheduleError> {
    let cal = us_settlement_calendar();
    let start = RollRule::Following.adjust(&cal, reference);
    let ends = period_end_dates(&cal, start, years);
    schedule_from_ends(start, &ends, accrual_basis)
}

/// Build a spot-starting USD OIS fixed-leg schedule whose final period ends at an
/// explicit `maturity` civil date — the generalisation that supports **custom
/// (sub-/multi-year, broken-period) tenors and odd-dated ("broken date")
/// pillars**, not just whole years.
///
/// Annual fixed-leg periods roll forward in 12-month steps from the spot origin;
/// every annual roll strictly before the (modified-following adjusted) `maturity`
/// is a full period, and a final stub period ends exactly at that adjusted
/// maturity. When `maturity` is the roll-adjusted N-year date, the result is
/// **byte-identical** to [`usd_ois_schedule_with_basis`]`(reference, N, basis)`
/// (the loop emits the same N period ends), so the dated and tenor paths price a
/// whole-year pillar identically. The payment discount-time coordinate stays
/// ACT/365F from spot; the accrual uses `accrual_basis`.
///
/// # Errors
///
/// Returns [`ScheduleError::Empty`] if the adjusted `maturity` is not strictly
/// after the adjusted spot start (a zero-/negative-length pillar).
pub fn usd_ois_schedule_to_maturity(
    reference: Date,
    maturity: Date,
    accrual_basis: AccrualBasis,
) -> Result<OisSchedule, ScheduleError> {
    let cal = us_settlement_calendar();
    let start = RollRule::Following.adjust(&cal, reference);
    let end = RollRule::ModifiedFollowing.adjust(&cal, maturity);
    if end <= start {
        return Err(ScheduleError::Empty);
    }

    // Full annual rolls strictly before the final maturity, then the maturity stub.
    let mut ends = Vec::new();
    let mut step = 1i32;
    loop {
        let roll = RollRule::ModifiedFollowing.adjust(&cal, add_months(start, step * 12));
        if roll < end {
            ends.push(roll);
            step += 1;
        } else {
            break;
        }
    }
    ends.push(end);
    schedule_from_ends(start, &ends, accrual_basis)
}

/// Build a spot-starting USD OIS fixed-leg schedule of a `months`-month tenor
/// (e.g. 3, 18, 30) — the month-tenor arm of a curve pillar. The maturity is the
/// spot start advanced by `months` calendar months (end-of-month-aware) and rolled
/// modified-following; the schedule then follows the same annual-roll-plus-final-stub
/// shape as [`usd_ois_schedule_to_maturity`]. A `months` that is an exact multiple
/// of 12 yields the same schedule as the equivalent whole-year tenor.
///
/// # Errors
///
/// Returns [`ScheduleError::Empty`] if `months` is zero.
pub fn usd_ois_schedule_for_months(
    reference: Date,
    months: u32,
    accrual_basis: AccrualBasis,
) -> Result<OisSchedule, ScheduleError> {
    if months == 0 {
        return Err(ScheduleError::Empty);
    }
    let cal = us_settlement_calendar();
    let start = RollRule::Following.adjust(&cal, reference);
    let maturity = add_months(start, months as i32);
    usd_ois_schedule_to_maturity(start, maturity, accrual_basis)
}

/// Turn a spot `start` plus its roll-adjusted period-end dates into the numeric
/// [`OisSchedule`]: ACT/365F payment times from spot, `accrual_basis` accruals.
fn schedule_from_ends(
    start: Date,
    ends: &[Date],
    accrual_basis: AccrualBasis,
) -> Result<OisSchedule, ScheduleError> {
    let mut periods = Vec::with_capacity(ends.len());
    let mut prev = start;
    for &end in ends {
        let accrual = accrual_basis.year_fraction(prev, end);
        let pay = year_fraction(DayCount::Act365Fixed, start, end);
        periods.push(FixedPeriod { pay, accrual });
        prev = end;
    }
    OisSchedule::new(Time(0.0), periods)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::{OisQuote, bootstrap_ois};
    use crate::ois::ois_par_rate;
    use celnet_types::Rate;
    use time::Month;

    /// A reference spot date that is a US business day (Monday 16 Jun 2025).
    fn spot() -> Date {
        Date::from_calendar_date(2025, Month::June, 16).expect("valid date")
    }

    fn period_coords(s: &OisSchedule) -> Vec<(f64, f64)> {
        s.periods().iter().map(|p| (p.pay.0, p.accrual.0)).collect()
    }

    /// Parity oracle: the dated builder reproduces the whole-year tenor builder
    /// byte-for-byte. For each N, `usd_ois_schedule_to_maturity(spot, spot+N·12m)`
    /// must equal `usd_ois_schedule_with_basis(spot, N)` in every period's `(pay,
    /// accrual)` — so a curve pillar's `years` and equivalent dated arm price a
    /// whole-year point identically (no drift introduced by the generalisation).
    #[test]
    fn dated_builder_matches_whole_year_tenor_builder() {
        let cal = us_settlement_calendar();
        let start = RollRule::Following.adjust(&cal, spot());
        for years in 1u32..=30 {
            let by_tenor =
                usd_ois_schedule_with_basis(spot(), years, AccrualBasis::Act360).expect("tenor");
            let maturity = add_months(start, (years as i32) * 12);
            let by_date =
                usd_ois_schedule_to_maturity(spot(), maturity, AccrualBasis::Act360).expect("date");
            assert_eq!(
                period_coords(&by_tenor),
                period_coords(&by_date),
                "tenor/date schedule mismatch at {years}Y"
            );
        }
    }

    /// A 12-month tenor resolves to the same schedule as the 1-year tenor, and a
    /// 24-month to the 2-year — the month arm agrees with whole years on the grid.
    #[test]
    fn month_arm_matches_whole_year_on_grid() {
        for (months, years) in [(12u32, 1u32), (24, 2), (36, 3)] {
            let by_months =
                usd_ois_schedule_for_months(spot(), months, AccrualBasis::Act360).expect("months");
            let by_years =
                usd_ois_schedule_with_basis(spot(), years, AccrualBasis::Act360).expect("years");
            assert_eq!(period_coords(&by_months), period_coords(&by_years));
        }
    }

    /// A broken (odd-dated) maturity yields a final stub whose ACT/365F payment
    /// time equals the day-count from spot to the roll-adjusted maturity, and the
    /// schedule has one period per elapsed year plus the stub.
    #[test]
    fn broken_date_stub_pays_at_day_count() {
        // ~18 months out: 16 Jun 2025 spot → 18 Dec 2026 (a Friday business day).
        let maturity = Date::from_calendar_date(2026, Month::December, 18).expect("valid");
        let sched =
            usd_ois_schedule_to_maturity(spot(), maturity, AccrualBasis::Act360).expect("dated");
        // One full annual period (to ~Jun 2026) + the final stub to the maturity.
        assert_eq!(sched.periods().len(), 2);
        let cal = us_settlement_calendar();
        let start = RollRule::Following.adjust(&cal, spot());
        let end = RollRule::ModifiedFollowing.adjust(&cal, maturity);
        let expected_pay = year_fraction(DayCount::Act365Fixed, start, end);
        let last_pay = sched.periods().last().unwrap().pay;
        assert!((last_pay.0 - expected_pay.0).abs() < 1e-12);
    }

    #[test]
    fn builds_the_expected_number_of_annual_periods() {
        let sched = usd_sofr_ois_schedule(spot(), 5).expect("schedule");
        assert_eq!(sched.periods().len(), 5);
    }

    #[test]
    fn payment_times_are_monotone_and_about_one_year_apart() {
        let sched = usd_sofr_ois_schedule(spot(), 10).expect("schedule");
        let mut prev = 0.0;
        for (i, p) in sched.periods().iter().enumerate() {
            let years = (i + 1) as f64;
            // ACT/365F over i whole years lands within a few days of the integer.
            assert!(
                (p.pay.0 - years).abs() < 0.03,
                "period {i} pay {} not ~{years}",
                p.pay.0
            );
            assert!(p.pay.0 > prev, "payment times must increase");
            prev = p.pay.0;
        }
    }

    #[test]
    fn accruals_are_act360_year_fractions() {
        let sched = usd_sofr_ois_schedule(spot(), 5).expect("schedule");
        for p in sched.periods() {
            // One annual ACT/360 accrual is ~365/360 ≈ 1.014 (a little more across a roll).
            assert!(
                p.accrual.0 > 1.0 && p.accrual.0 < 1.05,
                "accrual {}",
                p.accrual.0
            );
        }
    }

    #[test]
    fn thirty_360_basis_is_selectable_and_distinct_from_act360() {
        let s360 = usd_sofr_ois_schedule(spot(), 5).expect("act360 schedule");
        let s3360 = usd_ois_schedule_with_basis(spot(), 5, AccrualBasis::Thirty360BondBasis)
            .expect("30/360");
        assert_eq!(s360.periods().len(), s3360.periods().len());
        for (a, b) in s360.periods().iter().zip(s3360.periods()) {
            // The payment discount-time coordinate (curve axis) is basis-independent.
            assert_eq!(a.pay, b.pay);
            // A 30/360 annual accrual sits ~1.0; ACT/360 sits ~365/360 ⇒ strictly larger.
            assert!(
                (b.accrual.0 - 1.0).abs() < 0.05,
                "30/360 accrual {}",
                b.accrual.0
            );
            assert!(
                a.accrual.0 > b.accrual.0,
                "act360 {} should exceed 30/360 {}",
                a.accrual.0,
                b.accrual.0
            );
        }
    }

    #[test]
    fn thirty_360_schedules_bootstrap_and_reprice_to_par() {
        // The selectable-basis schedule plugs into the bootstrap end-to-end.
        let quotes: Vec<OisQuote> = [(1u32, 0.0432), (2, 0.0418), (5, 0.0405)]
            .into_iter()
            .map(|(years, par)| OisQuote {
                schedule: usd_ois_schedule_with_basis(
                    spot(),
                    years,
                    AccrualBasis::Thirty360BondBasis,
                )
                .expect("schedule"),
                par_rate: Rate(par),
            })
            .collect();
        let curve = bootstrap_ois(&quotes).expect("bootstraps from 30/360 schedules");
        for q in &quotes {
            let repriced = ois_par_rate(&curve, &q.schedule).0;
            assert!(
                (repriced - q.par_rate.0).abs() < 1e-9,
                "repriced {repriced} vs {}",
                q.par_rate.0
            );
        }
    }

    #[test]
    fn every_period_end_lands_on_a_us_business_day() {
        let cal = us_settlement_calendar();
        for end in period_end_dates(&cal, spot(), 12) {
            assert!(cal.is_business_day(end), "{end} is not a US business day");
        }
    }

    #[test]
    fn schedules_from_dates_bootstrap_and_reprice_to_par() {
        // The date layer plugs into the sequential bootstrap end-to-end: build real-date
        // schedules, calibrate, and confirm each reprices to its quoted par rate.
        let quotes: Vec<OisQuote> = [(1u32, 0.0432), (2, 0.0418), (5, 0.0405), (10, 0.0415)]
            .into_iter()
            .map(|(years, par)| OisQuote {
                schedule: usd_sofr_ois_schedule(spot(), years).expect("schedule"),
                par_rate: Rate(par),
            })
            .collect();
        let curve = bootstrap_ois(&quotes).expect("bootstraps from dated schedules");
        for q in &quotes {
            let repriced = ois_par_rate(&curve, &q.schedule).0;
            assert!(
                (repriced - q.par_rate.0).abs() < 1e-9,
                "repriced {repriced} vs {}",
                q.par_rate.0
            );
        }
    }

    #[test]
    fn zero_years_is_an_empty_schedule() {
        assert_eq!(
            usd_sofr_ois_schedule(spot(), 0).unwrap_err(),
            ScheduleError::Empty
        );
    }
}
