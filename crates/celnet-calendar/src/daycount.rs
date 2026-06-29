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

/// Whole **30/360 Bond Basis** day count from `start` to `end` (signed).
///
/// Bond Basis (a.k.a. "30/360", "360/360") per the ISDA 2006 Definitions
/// §4.16(f): every month is treated as 30 days and every year as 360. Two
/// end-of-month roll rules collapse a 31st onto the 30th:
///
/// - if the start day is 31 it becomes 30;
/// - if the end day is 31 **and** the (possibly adjusted) start day is 30 it
///   becomes 30.
///
/// No February-end special-casing is applied — that omission is exactly what
/// distinguishes plain Bond Basis from the `30E/360 ISDA` family. Validated to
/// the day against QuantLib 1.42 `Thirty360(Thirty360::BondBasis)` across the
/// end-of-month, 31st-roll, and (leap) February edge cases (see tests). The
/// result is signed if `end < start`, matching [`actual_days`].
#[must_use]
pub fn thirty_360_bond_basis_days(start: Date, end: Date) -> i64 {
    let mut d1 = i64::from(start.day());
    let mut d2 = i64::from(end.day());
    let m1 = i64::from(u8::from(start.month()));
    let m2 = i64::from(u8::from(end.month()));
    let y1 = i64::from(start.year());
    let y2 = i64::from(end.year());

    if d1 == 31 {
        d1 = 30;
    }
    if d2 == 31 && d1 == 30 {
        d2 = 30;
    }

    360 * (y2 - y1) + 30 * (m2 - m1) + (d2 - d1)
}

/// Year fraction under 30/360 Bond Basis: [`thirty_360_bond_basis_days`] / 360.
///
/// The fixed-leg accrual basis for standard USD bonds and the fixed leg of
/// vanilla swaps. Signed if `end < start`.
#[must_use]
pub fn thirty_360_bond_basis_year_fraction(start: Date, end: Date) -> Time {
    Time(thirty_360_bond_basis_days(start, end) as f64 / 360.0)
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

    /// 30/360 Bond Basis day counts validated **to the day** against the
    /// independent golden oracle QuantLib 1.42.1
    /// `Thirty360(Thirty360.BondBasis).dayCount(d1, d2)`. Each row exercises one
    /// of the convention's edge rules; the expected integers are QuantLib output,
    /// not a re-run of this engine.
    #[test]
    fn thirty_360_bond_basis_matches_quantlib() {
        // (start, end, expected_days, rule exercised)
        let cases: [(Date, Date, i64, &str); 11] = [
            (
                d(2007, Month::January, 15),
                d(2007, Month::February, 15),
                30,
                "plain month",
            ),
            (
                d(2007, Month::January, 31),
                d(2007, Month::March, 31),
                60,
                "d1=31→30, d2=31&d1=30→30",
            ),
            (
                d(2007, Month::January, 15),
                d(2007, Month::March, 31),
                76,
                "d2=31 but d1≠30 ⇒ kept",
            ),
            (
                d(2006, Month::December, 31),
                d(2007, Month::January, 31),
                30,
                "31→31 across year",
            ),
            (
                d(2006, Month::August, 31),
                d(2006, Month::November, 30),
                90,
                "d1=31→30",
            ),
            (
                d(2006, Month::February, 28),
                d(2007, Month::February, 28),
                360,
                "last-Feb, no rule",
            ),
            (
                d(2008, Month::February, 29),
                d(2009, Month::February, 28),
                359,
                "leap last-Feb ⇒ 359",
            ),
            (
                d(2007, Month::January, 31),
                d(2007, Month::February, 28),
                28,
                "Feb-end not bumped",
            ),
            (
                d(2006, Month::February, 28),
                d(2006, Month::August, 31),
                183,
                "d2=31, d1=28 ⇒ kept",
            ),
            (
                d(2025, Month::January, 1),
                d(2026, Month::January, 1),
                360,
                "one calendar year",
            ),
            (
                d(2025, Month::September, 16),
                d(2025, Month::December, 16),
                90,
                "FRA 3x6 schedule",
            ),
        ];
        for (start, end, expected_days, rule) in cases {
            let days = thirty_360_bond_basis_days(start, end);
            assert_eq!(days, expected_days, "{start} → {end} ({rule})");
            let yf = thirty_360_bond_basis_year_fraction(start, end);
            assert_close!(yf.0, expected_days as f64 / 360.0);
        }
    }

    #[test]
    fn thirty_360_bond_basis_is_signed_when_reversed() {
        let fwd =
            thirty_360_bond_basis_days(d(2025, Month::January, 1), d(2026, Month::January, 1));
        let rev =
            thirty_360_bond_basis_days(d(2026, Month::January, 1), d(2025, Month::January, 1));
        assert_eq!(fwd, 360);
        assert_eq!(rev, -360);
    }
}
