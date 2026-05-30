//! Weekend predicate shared across the holiday rule sets and the business-day
//! engine. Kept tiny and pure so it inlines on the hot path.

use time::{Date, Weekday};

/// Whether `date` falls on a Saturday or Sunday (the weekend for every
/// currency centre Celnet currently covers).
#[must_use]
pub fn is_weekend(date: Date) -> bool {
    matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    #[test]
    fn weekend_detection() {
        // 2024-06-01 is a Saturday, 2024-06-02 a Sunday, 2024-06-03 a Monday.
        assert!(is_weekend(
            Date::from_calendar_date(2024, Month::June, 1).unwrap()
        ));
        assert!(is_weekend(
            Date::from_calendar_date(2024, Month::June, 2).unwrap()
        ));
        assert!(!is_weekend(
            Date::from_calendar_date(2024, Month::June, 3).unwrap()
        ));
    }
}
