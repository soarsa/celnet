//! Selectable per-leg / per-instrument accrual day-count basis for the linear
//! rates book.
//!
//! The curve's continuous **time axis** and option **vol-time** use the
//! foundational [`celnet_types::DayCount`] vocabulary (ACT/365F, ACT/360). A
//! traded leg's **accrual fraction** `tau`, however, can also quote on a 30/360
//! schedule (USD fixed bonds and the fixed leg of vanilla swaps), so this enum
//! is the instrument-level superset that adds 30/360 Bond Basis on top of the
//! money-market bases. Keeping the two concerns in distinct types means the
//! curve/vol time axis is never accidentally measured on a 30/360 basis.
//!
//! The actual day-count arithmetic lives once in `celnet-calendar`
//! (the shared, independently tested date engine); this type only dispatches.

use celnet_calendar::{thirty_360_bond_basis_year_fraction, year_fraction};
use celnet_types::{DayCount, Time};
use time::Date;

/// Day-count basis a linear-rates leg uses to turn an accrual period into a year
/// fraction `tau`.
///
/// A superset of the money-market subset of [`celnet_types::DayCount`], extended
/// with [`AccrualBasis::Thirty360BondBasis`] for fixed legs that quote 30/360.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccrualBasis {
    /// Actual/360 — USD/EUR money-market accrual (the linear-book default).
    Act360,
    /// Actual/365 fixed.
    Act365Fixed,
    /// 30/360 Bond Basis (ISDA 2006 §4.16(f)) — the standard USD fixed-bond/swap basis.
    Thirty360BondBasis,
}

impl AccrualBasis {
    /// Year fraction `tau` of the accrual period `[start, end]` under this basis.
    ///
    /// Delegates to `celnet-calendar` so the arithmetic is single-homed and
    /// oracle-validated there. Signed if `end < start`.
    #[must_use]
    pub fn year_fraction(self, start: Date, end: Date) -> Time {
        match self {
            Self::Act360 => year_fraction(DayCount::Act360, start, end),
            Self::Act365Fixed => year_fraction(DayCount::Act365Fixed, start, end),
            Self::Thirty360BondBasis => thirty_360_bond_basis_year_fraction(start, end),
        }
    }
}

impl From<DayCount> for AccrualBasis {
    /// Lift a curve/vol [`DayCount`] into the instrument-accrual vocabulary.
    fn from(dc: DayCount) -> Self {
        match dc {
            DayCount::Act360 => Self::Act360,
            DayCount::Act365Fixed => Self::Act365Fixed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).expect("valid date")
    }

    #[test]
    fn act_bases_agree_with_celnet_calendar() {
        let s = d(2025, Month::September, 16);
        let e = d(2025, Month::December, 16);
        assert_eq!(
            AccrualBasis::Act360.year_fraction(s, e),
            year_fraction(DayCount::Act360, s, e)
        );
        assert_eq!(
            AccrualBasis::Act365Fixed.year_fraction(s, e),
            year_fraction(DayCount::Act365Fixed, s, e)
        );
    }

    #[test]
    fn thirty_360_bond_basis_dispatches_to_calendar() {
        // FRA 3x6 window: 30/360 BondBasis is exactly 90/360 = 0.25 (QuantLib oracle),
        // distinct from ACT/360's 91/360.
        let s = d(2025, Month::September, 16);
        let e = d(2025, Month::December, 16);
        assert_eq!(
            AccrualBasis::Thirty360BondBasis.year_fraction(s, e),
            thirty_360_bond_basis_year_fraction(s, e)
        );
        assert!((AccrualBasis::Thirty360BondBasis.year_fraction(s, e).0 - 0.25).abs() < 1e-15);
        assert!(
            AccrualBasis::Act360.year_fraction(s, e).0
                > AccrualBasis::Thirty360BondBasis.year_fraction(s, e).0
        );
    }

    #[test]
    fn from_day_count_round_trips_the_money_market_bases() {
        assert_eq!(AccrualBasis::from(DayCount::Act360), AccrualBasis::Act360);
        assert_eq!(
            AccrualBasis::from(DayCount::Act365Fixed),
            AccrualBasis::Act365Fixed
        );
    }
}
