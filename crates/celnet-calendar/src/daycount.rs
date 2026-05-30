//! Year-fraction computation by [`DayCount`] basis.
//!
//! Two bases matter for FX options and they must be kept distinct (see
//! `docs/ANALYTICS-SPEC.md` §1.5): **vol-time** uses ACT/365-fixed (calendar
//! days divided by a fixed 365), whereas **money-market interest accrual** uses
//! each currency's basis — ACT/360 for USD/EUR, ACT/365 for GBP/AUD. Conflating
//! the two is a classic mispricing source, so this module exposes a single
//! pure function over the frozen [`DayCount`] vocabulary and never guesses a
//! default.

use celnet_types::{DayCount, Time};
use time::Date;

/// Whole calendar days from `start` to `end` (`end - start`), signed.
///
/// Uses civil-date arithmetic from the `time` crate, so it is leap-year and
/// month-length correct without any floating point.
#[must_use]
pub fn actual_days(start: Date, end: Date) -> i64 {
    (end - start).whole_days()
}

/// Year fraction between two dates under the given [`DayCount`] basis.
///
/// - [`DayCount::Act365Fixed`] → `actual_days / 365` (vol-time / GBP-AUD accrual).
/// - [`DayCount::Act360`] → `actual_days / 360` (USD/EUR money-market accrual).
///
/// The result is signed if `end < start`, which lets callers detect
/// non-causal date pairs without a separate branch.
#[must_use]
pub fn year_fraction(basis: DayCount, start: Date, end: Date) -> Time {
    let days = actual_days(start, end) as f64;
    let denom = match basis {
        DayCount::Act365Fixed => 365.0,
        DayCount::Act360 => 360.0,
    };
    Time(days / denom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;
    use time::Month;

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).unwrap()
    }

    #[test]
    fn act365_one_year_non_leap() {
        // 2023 is a non-leap year: 365 days → exactly 1.0.
        let yf = year_fraction(
            DayCount::Act365Fixed,
            d(2023, Month::January, 1),
            d(2024, Month::January, 1),
        );
        assert_close!(yf.0, 1.0);
    }

    #[test]
    fn act365_leap_year_exceeds_one() {
        // 2024 is a leap year: 366 days / 365 > 1.0 under ACT/365-fixed.
        let yf = year_fraction(
            DayCount::Act365Fixed,
            d(2024, Month::January, 1),
            d(2025, Month::January, 1),
        );
        assert_close!(yf.0, 366.0 / 365.0);
    }

    #[test]
    fn act360_one_year() {
        let yf = year_fraction(
            DayCount::Act360,
            d(2023, Month::January, 1),
            d(2024, Month::January, 1),
        );
        assert_close!(yf.0, 365.0 / 360.0);
    }

    #[test]
    fn signed_when_reversed() {
        let yf = year_fraction(
            DayCount::Act365Fixed,
            d(2024, Month::January, 1),
            d(2023, Month::January, 1),
        );
        assert_close!(yf.0, -1.0);
    }

    #[test]
    fn vol_time_and_accrual_differ() {
        let s = d(2024, Month::March, 1);
        let e = d(2024, Month::June, 1);
        let vol = year_fraction(DayCount::Act365Fixed, s, e);
        let acc = year_fraction(DayCount::Act360, s, e);
        // Same numerator, different denominator → ACT/360 fraction is larger.
        assert!(acc.0 > vol.0);
    }
}
