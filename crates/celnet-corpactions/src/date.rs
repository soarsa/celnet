//! A serde-clean civil date shared by the event record and the schedule.
//!
//! The crate keeps its public surface on a plain `(year, month, day)` triple — the same
//! discipline `celnet-refdata`'s `CivilYmd` uses — so every record round-trips through JSON
//! without pulling `time`'s optional serde feature, and the pure effect math converts to a
//! real [`time::Date`] only where calendar arithmetic is genuinely needed.

use serde::{Deserialize, Serialize};
use time::Date;

/// A civil (broken) date: a plain `(year, month, day)` triple.
///
/// Derives `Ord` in field order (`year`, then `month`, then `day`), which is exactly
/// chronological order, so schedules sort and compare by date without conversion. Validate a
/// triple came from a real calendar with [`CivilDate::to_date`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CivilDate {
    /// Gregorian year (e.g. 2035).
    pub year: i32,
    /// Month of year, 1..=12.
    pub month: u8,
    /// Day of month, 1..=31 (validated against the month by [`CivilDate::to_date`]).
    pub day: u8,
}

impl CivilDate {
    /// Construct a civil date from its components (unchecked; validate with [`CivilDate::to_date`]).
    #[must_use]
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }

    /// Convert to a real [`time::Date`], or `None` if the triple is not a real calendar date
    /// (e.g. 2035-02-30). The single validation gate the crate uses.
    #[must_use]
    pub fn to_date(self) -> Option<Date> {
        let month = time::Month::try_from(self.month).ok()?;
        Date::from_calendar_date(self.year, month, self.day).ok()
    }

    /// Project a real [`time::Date`] back onto a civil triple (always valid).
    #[must_use]
    pub fn from_date(date: Date) -> Self {
        Self {
            year: date.year(),
            month: u8::from(date.month()),
            day: date.day(),
        }
    }

    /// Whether this triple is a real calendar date.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.to_date().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_impossible_date() {
        assert!(!CivilDate::new(2035, 2, 30).is_valid());
        assert!(CivilDate::new(2035, 2, 28).is_valid());
    }

    #[test]
    fn orders_chronologically() {
        assert!(CivilDate::new(2035, 1, 31) < CivilDate::new(2035, 2, 1));
        assert!(CivilDate::new(2034, 12, 31) < CivilDate::new(2035, 1, 1));
    }

    #[test]
    fn round_trips_through_time_date() {
        let c = CivilDate::new(2035, 6, 15);
        let d = c.to_date().expect("real date");
        assert_eq!(CivilDate::from_date(d), c);
    }
}
