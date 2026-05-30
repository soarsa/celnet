//! Business-day-roll conventions for adjusting a candidate (calendar) date onto
//! a good business day.
//!
//! After a tenor is added to the spot date the result is a *calendar* date that
//! may fall on a weekend or holiday; it is moved onto a business day by a roll
//! rule (see `docs/ANALYTICS-SPEC.md` §1.5). FX uses **modified following**:
//! roll forward to the next business day, but if that crosses into the next
//! month, roll *backward* to the last business day of the original month
//! instead. The **end-of-month** rule additionally pins month-end tenors: if the
//! spot date is the last business day of its month, the tenor date is the last
//! business day of the target month.

use time::{Date, Month};

use crate::calendar::BusinessCalendar;

/// How a candidate date is adjusted onto a business day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollRule {
    /// No adjustment — used internally; rarely correct for settlement.
    Unadjusted,
    /// Next business day on or after the candidate.
    Following,
    /// Previous business day on or before the candidate.
    Preceding,
    /// [`RollRule::Following`], unless it changes the month, in which case
    /// [`RollRule::Preceding`]. The FX market standard.
    ModifiedFollowing,
}

impl RollRule {
    /// Adjust `date` onto a business day in `cal` per this rule.
    #[must_use]
    pub fn adjust(self, cal: &BusinessCalendar, date: Date) -> Date {
        match self {
            RollRule::Unadjusted => date,
            RollRule::Following => following(cal, date),
            RollRule::Preceding => preceding(cal, date),
            RollRule::ModifiedFollowing => {
                let fwd = following(cal, date);
                if fwd.month() != date.month() {
                    preceding(cal, date)
                } else {
                    fwd
                }
            }
        }
    }
}

/// Next business day on or after `date`.
#[must_use]
pub fn following(cal: &BusinessCalendar, date: Date) -> Date {
    if cal.is_business_day(date) {
        date
    } else {
        cal.next_business_day(date)
    }
}

/// Previous business day on or before `date`.
#[must_use]
pub fn preceding(cal: &BusinessCalendar, date: Date) -> Date {
    if cal.is_business_day(date) {
        date
    } else {
        cal.prev_business_day(date)
    }
}

/// The last business day of `date`'s month in `cal`.
#[must_use]
pub fn last_business_day_of_month(cal: &BusinessCalendar, date: Date) -> Date {
    let days = date.month().length(date.year());
    let month_end =
        Date::from_calendar_date(date.year(), date.month(), days).expect("valid month-end date");
    preceding(cal, month_end)
}

/// Whether `date` is the last business day of its month in `cal`.
#[must_use]
pub fn is_last_business_day_of_month(cal: &BusinessCalendar, date: Date) -> bool {
    cal.is_business_day(date) && last_business_day_of_month(cal, date) == date
}

/// Add `months` calendar months to `date`, clamping the day to the target
/// month's length (so 31 Jan + 1M → 28/29 Feb). This is the conventional
/// "end-of-month-aware" month addition before any business-day roll.
#[must_use]
pub fn add_months(date: Date, months: i32) -> Date {
    let total = date.year() * 12 + (month_index(date.month()) as i32) + months;
    let year = total.div_euclid(12);
    let month0 = total.rem_euclid(12);
    let month = month_from_index(month0 as u8);
    let max_day = month.length(year);
    let day = date.day().min(max_day);
    Date::from_calendar_date(year, month, day).expect("valid shifted date")
}

/// Add `weeks` calendar weeks to `date`.
#[must_use]
pub fn add_weeks(date: Date, weeks: i32) -> Date {
    date + time::Duration::weeks(i64::from(weeks))
}

/// Add `years` calendar years (`= 12·years` months, end-of-month-aware).
#[must_use]
pub fn add_years(date: Date, years: i32) -> Date {
    add_months(date, years * 12)
}

fn month_index(m: Month) -> u8 {
    m as u8 - 1 // January → 0
}

fn month_from_index(i: u8) -> Month {
    Month::try_from(i + 1).expect("month index in range")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holiday::CentreId;

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).unwrap()
    }

    #[test]
    fn modified_following_rolls_back_at_month_end() {
        let cal = BusinessCalendar::single(CentreId::UnitedKingdom);
        // 30 Nov 2024 is a Saturday. Following → Mon 2 Dec (next month) →
        // modified-following rolls back to Fri 29 Nov.
        let cand = d(2024, Month::November, 30);
        assert_eq!(
            RollRule::Following.adjust(&cal, cand),
            d(2024, Month::December, 2)
        );
        assert_eq!(
            RollRule::ModifiedFollowing.adjust(&cal, cand),
            d(2024, Month::November, 29)
        );
    }

    #[test]
    fn modified_following_stays_within_month() {
        let cal = BusinessCalendar::single(CentreId::UnitedKingdom);
        // 15 Jun 2024 is a Saturday → Mon 17 Jun, same month, no rollback.
        let cand = d(2024, Month::June, 15);
        assert_eq!(
            RollRule::ModifiedFollowing.adjust(&cal, cand),
            d(2024, Month::June, 17)
        );
    }

    #[test]
    fn add_months_clamps_day() {
        // 31 Jan + 1M → Feb has 29 days in 2024 (leap).
        assert_eq!(
            add_months(d(2024, Month::January, 31), 1),
            d(2024, Month::February, 29)
        );
        // 31 Jan + 1M → Feb has 28 days in 2023.
        assert_eq!(
            add_months(d(2023, Month::January, 31), 1),
            d(2023, Month::February, 28)
        );
        // 31 Mar + 1M → 30 Apr.
        assert_eq!(
            add_months(d(2024, Month::March, 31), 1),
            d(2024, Month::April, 30)
        );
    }

    #[test]
    fn add_months_crosses_year_boundary() {
        assert_eq!(
            add_months(d(2024, Month::November, 15), 3),
            d(2025, Month::February, 15)
        );
        assert_eq!(
            add_months(d(2024, Month::February, 15), -3),
            d(2023, Month::November, 15)
        );
    }

    #[test]
    fn last_business_day_of_month_skips_weekend() {
        let cal = BusinessCalendar::single(CentreId::UnitedKingdom);
        // Aug 2024 ends Sat 31 → last business day Fri 30 Aug.
        assert_eq!(
            last_business_day_of_month(&cal, d(2024, Month::August, 10)),
            d(2024, Month::August, 30)
        );
        assert!(is_last_business_day_of_month(
            &cal,
            d(2024, Month::August, 30)
        ));
        assert!(!is_last_business_day_of_month(
            &cal,
            d(2024, Month::August, 29)
        ));
    }
}
