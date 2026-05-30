//! The joint business-day calendar: an **intersection** of one or more
//! settlement centres, plus business-day navigation and date-roll conventions.
//!
//! FX settlement requires *both* legs' centres to be open (see
//! `docs/ANALYTICS-SPEC.md` §1.5): a date is a good business day for a pair only
//! if it is a business day in every relevant centre. Cross-via-USD pairs add the
//! US centre as a third member. The intersection is modelled as a fixed-capacity
//! set of [`SettlementCentre`] so the calendar is `Copy`, allocation-free and
//! cheap to thread through the hot path.

use time::Date;

use crate::holiday::{CentreId, SettlementCentre};

/// Maximum number of settlement centres intersected in one calendar.
///
/// The largest case is a cross-via-USD pair: base centre + quote centre + USD,
/// i.e. three. The fixed bound keeps [`BusinessCalendar`] `Copy` and on the
/// stack; constructing with more centres than this panics in `with_centres`.
pub const MAX_CENTRES: usize = 4;

/// A joint business-day calendar over up to [`MAX_CENTRES`] settlement centres.
///
/// A date is a business day iff it is a business day in **all** member centres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusinessCalendar {
    centres: [Option<SettlementCentre>; MAX_CENTRES],
}

impl BusinessCalendar {
    /// Build a calendar from an iterator of centre ids, de-duplicating them.
    ///
    /// # Panics
    /// Panics if more than [`MAX_CENTRES`] **distinct** centres are supplied.
    #[must_use]
    pub fn with_centres(ids: impl IntoIterator<Item = CentreId>) -> Self {
        let mut centres: [Option<SettlementCentre>; MAX_CENTRES] = [None; MAX_CENTRES];
        let mut len = 0usize;
        for id in ids {
            if centres[..len].iter().any(|c| c.is_some_and(|c| c.id == id)) {
                continue; // de-dup
            }
            assert!(len < MAX_CENTRES, "BusinessCalendar exceeds MAX_CENTRES");
            centres[len] = Some(SettlementCentre::new(id));
            len += 1;
        }
        Self { centres }
    }

    /// A single-centre calendar.
    #[must_use]
    pub fn single(id: CentreId) -> Self {
        Self::with_centres([id])
    }

    /// Iterator over the distinct member centres.
    pub fn centres(&self) -> impl Iterator<Item = SettlementCentre> + '_ {
        self.centres.iter().filter_map(|c| *c)
    }

    /// Whether `date` is a good business day in **every** member centre.
    #[must_use]
    pub fn is_business_day(&self, date: Date) -> bool {
        self.centres().all(|c| !c.is_non_business(date))
    }

    /// Whether `date` is a non-business day in **any** member centre.
    #[must_use]
    pub fn is_holiday_or_weekend(&self, date: Date) -> bool {
        !self.is_business_day(date)
    }

    /// The next business day strictly after `date`.
    #[must_use]
    pub fn next_business_day(&self, date: Date) -> Date {
        let mut d = date + time::Duration::days(1);
        while !self.is_business_day(d) {
            d += time::Duration::days(1);
        }
        d
    }

    /// The previous business day strictly before `date`.
    #[must_use]
    pub fn prev_business_day(&self, date: Date) -> Date {
        let mut d = date - time::Duration::days(1);
        while !self.is_business_day(d) {
            d -= time::Duration::days(1);
        }
        d
    }

    /// Advance `n` business days forward (`n >= 0`) from `date`.
    ///
    /// `n == 0` returns `date` unchanged even if it is a non-business day
    /// (callers requiring a good start date roll it first).
    #[must_use]
    pub fn add_business_days(&self, date: Date, n: u32) -> Date {
        let mut d = date;
        for _ in 0..n {
            d = self.next_business_day(d);
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).unwrap()
    }

    #[test]
    fn intersection_requires_all_open() {
        // EURUSD: TARGET2 ∩ US. 4 Jul 2024 (US Independence Day, Thu) is a US
        // holiday → not a joint business day, though TARGET2 is open.
        let cal = BusinessCalendar::with_centres([CentreId::Target2, CentreId::UnitedStates]);
        assert!(!cal.is_business_day(d(2024, Month::July, 4)));
        // 1 May 2024 (Labour Day, Wed) is a TARGET2 holiday → not joint.
        assert!(!cal.is_business_day(d(2024, Month::May, 1)));
        // 2 May 2024 (Thu) both open.
        assert!(cal.is_business_day(d(2024, Month::May, 2)));
    }

    #[test]
    fn dedup_centres() {
        let cal = BusinessCalendar::with_centres([
            CentreId::UnitedStates,
            CentreId::UnitedStates,
            CentreId::Target2,
        ]);
        assert_eq!(cal.centres().count(), 2);
    }

    #[test]
    fn next_and_prev_skip_weekend() {
        let cal = BusinessCalendar::single(CentreId::UnitedStates);
        // Fri 31 May 2024 → next business day Mon 3 Jun.
        assert_eq!(
            cal.next_business_day(d(2024, Month::May, 31)),
            d(2024, Month::June, 3)
        );
        // Mon 3 Jun → prev business day Fri 31 May.
        assert_eq!(
            cal.prev_business_day(d(2024, Month::June, 3)),
            d(2024, Month::May, 31)
        );
    }

    #[test]
    fn add_business_days_skips_holiday() {
        let cal = BusinessCalendar::single(CentreId::UnitedStates);
        // From Wed 3 Jul 2024, +1 business day skips Thu 4 Jul (Independence Day)
        // → Fri 5 Jul.
        assert_eq!(
            cal.add_business_days(d(2024, Month::July, 3), 1),
            d(2024, Month::July, 5)
        );
    }
}
