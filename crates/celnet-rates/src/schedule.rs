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

    let mut periods = Vec::with_capacity(ends.len());
    let mut prev = start;
    for end in ends {
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
