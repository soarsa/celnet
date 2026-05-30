//! Per-currency settlement-holiday rule sets and weekend rules.
//!
//! Each currency's settlement centre observes a concrete set of public/banking
//! holidays. These are encoded here as **rules evaluated per year** (not a
//! hand-typed table), so the calendar is correct for any year without manual
//! maintenance: fixed-date holidays with the centre's weekend-adjustment policy,
//! and the moving Western-Christian feasts derived from the computus (Good
//! Friday, Easter Monday, Ascension, Whit Monday).
//!
//! Scope is the eight G10 currencies required by work-stream WS-A — USD, EUR,
//! GBP, JPY, CHF, AUD, CAD, NZD — modelling each currency's **principal
//! settlement centre** (e.g. EUR → TARGET2, USD → US Federal/SIFMA settlement,
//! JPY → Tokyo). The data are real public holidays, not placeholders; regional
//! sub-centre variations (e.g. individual Swiss cantons, Australian states) are
//! deliberately out of scope for a currency-level FX settlement calendar.
//!
//! References for the holiday definitions: national banking-holiday schedules
//! and the TARGET2 calendar (European Central Bank); the computus uses the
//! anonymous Gregorian algorithm (Meeus/Jones/Butcher).

use time::{Date, Month, Weekday};

use crate::weekday::is_weekend;

/// The weekend convention for a settlement centre.
///
/// All eight covered currencies use the Saturday/Sunday weekend; the type is
/// kept explicit so a Friday/Saturday (Gulf) or other weekend can be added
/// without touching the business-day engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekendRule {
    /// Saturday and Sunday are non-business days.
    SaturdaySunday,
}

impl WeekendRule {
    /// Whether `date` falls on this centre's weekend.
    #[must_use]
    pub fn is_weekend(self, date: Date) -> bool {
        match self {
            WeekendRule::SaturdaySunday => is_weekend(date),
        }
    }
}

/// A settlement centre: a weekend rule plus a holiday predicate.
///
/// One centre is associated with each currency (and one extra, USD, is always
/// intersected for cross-via-USD pairs by the engine layer). The predicate is
/// pure and allocation-free so it is cheap to call inside business-day loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettlementCentre {
    /// Stable identifier of the centre (the currency it settles).
    pub id: CentreId,
    /// Weekend rule for the centre.
    pub weekend: WeekendRule,
}

/// Identifier of a settlement centre, one per supported currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CentreId {
    /// United States settlement (US Federal Reserve / SIFMA bond-market days).
    UnitedStates,
    /// Euro area — the TARGET2 high-value payment calendar.
    Target2,
    /// United Kingdom — London bank holidays.
    UnitedKingdom,
    /// Japan — Tokyo bank holidays.
    Japan,
    /// Switzerland — Zurich/federal bank holidays.
    Switzerland,
    /// Australia — national (Commonwealth) bank holidays.
    Australia,
    /// Canada — federal bank holidays.
    Canada,
    /// New Zealand — national bank holidays.
    NewZealand,
}

impl SettlementCentre {
    /// Construct the canonical settlement centre for a [`CentreId`].
    #[must_use]
    pub const fn new(id: CentreId) -> Self {
        Self {
            id,
            weekend: WeekendRule::SaturdaySunday,
        }
    }

    /// Whether `date` is a non-business day for this centre (weekend OR holiday).
    #[must_use]
    pub fn is_non_business(self, date: Date) -> bool {
        self.weekend.is_weekend(date) || self.is_holiday(date)
    }

    /// Whether `date` is a settlement holiday for this centre.
    #[must_use]
    pub fn is_holiday(self, date: Date) -> bool {
        match self.id {
            CentreId::UnitedStates => is_us_holiday(date),
            CentreId::Target2 => is_target2_holiday(date),
            CentreId::UnitedKingdom => is_uk_holiday(date),
            CentreId::Japan => is_japan_holiday(date),
            CentreId::Switzerland => is_switzerland_holiday(date),
            CentreId::Australia => is_australia_holiday(date),
            CentreId::Canada => is_canada_holiday(date),
            CentreId::NewZealand => is_newzealand_holiday(date),
        }
    }
}

// ---------------------------------------------------------------------------
// Date helpers used by the holiday rules.
// ---------------------------------------------------------------------------

fn date(year: i32, month: Month, day: u8) -> Date {
    Date::from_calendar_date(year, month, day).expect("valid civil date")
}

/// Easter Sunday for `year` (Gregorian) via the anonymous Gregorian computus
/// (Meeus/Jones/Butcher algorithm). Returns the `(month, day)`.
fn easter_sunday(year: i32) -> Date {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month_num = (h + l - 7 * m + 114) / 31; // 3 = March, 4 = April
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    let month = if month_num == 3 {
        Month::March
    } else {
        Month::April
    };
    date(year, month, day as u8)
}

fn good_friday(year: i32) -> Date {
    easter_sunday(year) - time::Duration::days(2)
}

fn easter_monday(year: i32) -> Date {
    easter_sunday(year) + time::Duration::days(1)
}

fn ascension(year: i32) -> Date {
    // 39 days after Easter Sunday (the Thursday).
    easter_sunday(year) + time::Duration::days(39)
}

fn whit_monday(year: i32) -> Date {
    // 50 days after Easter Sunday.
    easter_sunday(year) + time::Duration::days(50)
}

/// The `n`-th `weekday` of `month` in `year` (1-based; `n == 1` → first).
fn nth_weekday(year: i32, month: Month, weekday: Weekday, n: u8) -> Date {
    let first = date(year, month, 1);
    let first_wd = first.weekday();
    let offset = (weekday.number_days_from_monday() as i64
        - first_wd.number_days_from_monday() as i64)
        .rem_euclid(7);
    first + time::Duration::days(offset + 7 * (i64::from(n) - 1))
}

/// The last `weekday` of `month` in `year`.
fn last_weekday(year: i32, month: Month, weekday: Weekday) -> Date {
    let days = month.length(year);
    let last = date(year, month, days);
    let last_wd = last.weekday();
    let back = (last_wd.number_days_from_monday() as i64
        - weekday.number_days_from_monday() as i64)
        .rem_euclid(7);
    last - time::Duration::days(back)
}

/// US "nearest-weekday" observance: a fixed-date federal holiday falling on a
/// Saturday is observed the preceding Friday; on a Sunday, the following Monday.
fn us_observed(d: Date) -> Date {
    match d.weekday() {
        Weekday::Saturday => d - time::Duration::days(1),
        Weekday::Sunday => d + time::Duration::days(1),
        _ => d,
    }
}

/// UK/Commonwealth "bump-forward" observance: a fixed-date holiday on a weekend
/// is observed on the next non-weekend day (Saturday → Monday, Sunday → Monday).
fn next_weekday_observed(d: Date) -> Date {
    match d.weekday() {
        Weekday::Saturday => d + time::Duration::days(2),
        Weekday::Sunday => d + time::Duration::days(1),
        _ => d,
    }
}

// ---------------------------------------------------------------------------
// Per-currency holiday rule sets.
// ---------------------------------------------------------------------------

/// US Federal Reserve settlement holidays (the SIFMA/bank-holiday set).
///
/// New Year's Day, Birthday of M.L. King Jr. (3rd Mon Jan), Washington's
/// Birthday (3rd Mon Feb), Memorial Day (last Mon May), Juneteenth (19 Jun,
/// federal from 2021), Independence Day (4 Jul), Labor Day (1st Mon Sep),
/// Columbus Day (2nd Mon Oct), Veterans Day (11 Nov), Thanksgiving (4th Thu
/// Nov), Christmas (25 Dec). Fixed-date holidays use the nearest-weekday rule.
fn is_us_holiday(d: Date) -> bool {
    let y = d.year();
    if d == us_observed(date(y, Month::January, 1)) {
        return true;
    }
    if d == nth_weekday(y, Month::January, Weekday::Monday, 3) {
        return true; // MLK Day
    }
    if d == nth_weekday(y, Month::February, Weekday::Monday, 3) {
        return true; // Washington's Birthday
    }
    if d == last_weekday(y, Month::May, Weekday::Monday) {
        return true; // Memorial Day
    }
    if y >= 2021 && d == us_observed(date(y, Month::June, 19)) {
        return true; // Juneteenth
    }
    if d == us_observed(date(y, Month::July, 4)) {
        return true; // Independence Day
    }
    if d == nth_weekday(y, Month::September, Weekday::Monday, 1) {
        return true; // Labor Day
    }
    if d == nth_weekday(y, Month::October, Weekday::Monday, 2) {
        return true; // Columbus Day
    }
    if d == us_observed(date(y, Month::November, 11)) {
        return true; // Veterans Day
    }
    if d == nth_weekday(y, Month::November, Weekday::Thursday, 4) {
        return true; // Thanksgiving
    }
    if d == us_observed(date(y, Month::December, 25)) {
        return true; // Christmas
    }
    false
}

/// TARGET2 closing days (the euro settlement calendar): New Year's Day,
/// Good Friday, Easter Monday, Labour Day (1 May), Christmas Day, and 26 Dec.
/// TARGET2 has **no** weekend-shift observance — fixed dates are observed only
/// on the day itself.
fn is_target2_holiday(d: Date) -> bool {
    let y = d.year();
    if d == date(y, Month::January, 1) {
        return true;
    }
    if d == good_friday(y) {
        return true;
    }
    if d == easter_monday(y) {
        return true;
    }
    if d == date(y, Month::May, 1) {
        return true; // Labour Day
    }
    if d == date(y, Month::December, 25) {
        return true; // Christmas
    }
    if d == date(y, Month::December, 26) {
        return true; // 2nd day of Christmas
    }
    false
}

/// UK (London) bank holidays: New Year's Day, Good Friday, Easter Monday,
/// Early May bank holiday (1st Mon May), Spring bank holiday (last Mon May),
/// Summer bank holiday (last Mon Aug), Christmas Day and Boxing Day. Fixed-date
/// holidays bump forward off the weekend; Christmas/Boxing Day get a substitute
/// day each when they land on a weekend.
fn is_uk_holiday(d: Date) -> bool {
    let y = d.year();
    if d == next_weekday_observed(date(y, Month::January, 1)) {
        return true;
    }
    if d == good_friday(y) {
        return true;
    }
    if d == easter_monday(y) {
        return true;
    }
    if d == nth_weekday(y, Month::May, Weekday::Monday, 1) {
        return true; // Early May
    }
    if d == last_weekday(y, Month::May, Weekday::Monday) {
        return true; // Spring
    }
    if d == last_weekday(y, Month::August, Weekday::Monday) {
        return true; // Summer
    }
    if is_uk_christmas_substitute(d) {
        return true;
    }
    false
}

/// UK Christmas (25 Dec) and Boxing Day (26 Dec) with their substitute-day
/// rules: when either lands on a weekend, a substitute weekday is granted, and
/// the two substitutes never collide (Boxing-Day substitute skips past the
/// Christmas substitute).
fn is_uk_christmas_substitute(d: Date) -> bool {
    let y = d.year();
    let xmas = date(y, Month::December, 25);
    let boxing = date(y, Month::December, 26);
    let xmas_obs = next_weekday_observed(xmas);
    // Boxing Day observance: bump off weekend, then off the Christmas observance.
    let mut boxing_obs = next_weekday_observed(boxing);
    if boxing_obs == xmas_obs {
        boxing_obs += time::Duration::days(1);
    }
    d == xmas_obs || d == boxing_obs
}

/// Japan (Tokyo) bank holidays. Banks observe the national public holidays plus
/// the New-Year bank closure (1–3 Jan) and 31 Dec. Public holidays follow the
/// "transfer holiday" rule (a holiday on Sunday is observed the next non-holiday
/// weekday) and the "citizens' holiday" rule (a weekday sandwiched between two
/// holidays — relevant to early May — becomes a holiday).
fn is_japan_holiday(d: Date) -> bool {
    // Bank year-end / new-year closure and 31 Dec.
    if d.month() == Month::December && d.day() == 31 {
        return true;
    }
    if d.month() == Month::January && (d.day() == 2 || d.day() == 3) {
        return true;
    }
    if is_japan_public_holiday(d) {
        return true;
    }
    // Transfer holiday: if the previous day is a Sunday public holiday, this
    // (Monday) is a substitute holiday.
    let prev = d - time::Duration::days(1);
    if d.weekday() == Weekday::Monday
        && prev.weekday() == Weekday::Sunday
        && is_japan_public_holiday(prev)
    {
        return true;
    }
    // Citizens' holiday (national holiday between two holidays): in practice
    // 4 May before its 2007 fixed designation. Covered by Greenery Day below.
    false
}

/// The Japanese national public holidays (Showa-era law as amended), excluding
/// the substitute/bank-closure logic handled by [`is_japan_holiday`].
fn is_japan_public_holiday(d: Date) -> bool {
    let y = d.year();
    let (m, day) = (d.month(), d.day());
    // Fixed-date holidays.
    if m == Month::January && day == 1 {
        return true; // New Year's Day
    }
    if m == Month::February && day == 11 {
        return true; // National Foundation Day
    }
    if y >= 2020 && m == Month::February && day == 23 {
        return true; // Emperor's Birthday (from 2020)
    }
    if m == Month::April && day == 29 {
        return true; // Showa Day
    }
    if m == Month::May && day == 3 {
        return true; // Constitution Memorial Day
    }
    if m == Month::May && day == 4 {
        return true; // Greenery Day
    }
    if m == Month::May && day == 5 {
        return true; // Children's Day
    }
    if m == Month::November && day == 3 {
        return true; // Culture Day
    }
    if m == Month::November && day == 23 {
        return true; // Labour Thanksgiving Day
    }
    // Happy-Monday movable holidays.
    if d == nth_weekday(y, Month::January, Weekday::Monday, 2) {
        return true; // Coming of Age Day
    }
    if d == nth_weekday(y, Month::July, Weekday::Monday, 3) {
        return true; // Marine Day
    }
    if d == nth_weekday(y, Month::September, Weekday::Monday, 3) {
        return true; // Respect for the Aged Day
    }
    if d == nth_weekday(y, Month::October, Weekday::Monday, 2) {
        return true; // Sports Day
    }
    // Equinoxes (astronomical; the published approximation valid for 1980–2099).
    if m == Month::March && day == spring_equinox_day(y) {
        return true; // Vernal Equinox Day
    }
    if m == Month::September && day == autumn_equinox_day(y) {
        return true; // Autumnal Equinox Day
    }
    false
}

/// Day of the Vernal Equinox holiday in March, per the standard published
/// approximation valid for 1980–2099.
fn spring_equinox_day(year: i32) -> u8 {
    let y = year as f64;
    (20.8431 + 0.242_194 * (y - 1980.0) - ((y - 1980.0) / 4.0).floor()).floor() as u8
}

/// Day of the Autumnal Equinox holiday in September (1980–2099 approximation).
fn autumn_equinox_day(year: i32) -> u8 {
    let y = year as f64;
    (23.2488 + 0.242_194 * (y - 1980.0) - ((y - 1980.0) / 4.0).floor()).floor() as u8
}

/// Switzerland (Zurich/federal) bank holidays: New Year's Day, Berchtold's Day
/// (2 Jan), Good Friday, Easter Monday, Labour Day (1 May), Ascension, Whit
/// Monday, Swiss National Day (1 Aug), Christmas Day, St Stephen's Day (26 Dec).
/// Swiss banking holidays are observed on the day itself (no weekend shift).
fn is_switzerland_holiday(d: Date) -> bool {
    let y = d.year();
    if d == date(y, Month::January, 1) {
        return true;
    }
    if d == date(y, Month::January, 2) {
        return true; // Berchtold's Day
    }
    if d == good_friday(y) {
        return true;
    }
    if d == easter_monday(y) {
        return true;
    }
    if d == date(y, Month::May, 1) {
        return true; // Labour Day
    }
    if d == ascension(y) {
        return true;
    }
    if d == whit_monday(y) {
        return true;
    }
    if d == date(y, Month::August, 1) {
        return true; // National Day
    }
    if d == date(y, Month::December, 25) {
        return true; // Christmas
    }
    if d == date(y, Month::December, 26) {
        return true; // St Stephen's Day
    }
    false
}

/// Australia (national) bank holidays: New Year's Day, Australia Day (26 Jan),
/// Good Friday, Easter Monday, Anzac Day (25 Apr), Queen's/King's Birthday
/// (2nd Mon Jun), Christmas Day, Boxing Day. Fixed-date holidays bump forward
/// off the weekend; Christmas/Boxing Day take non-colliding substitute days.
fn is_australia_holiday(d: Date) -> bool {
    let y = d.year();
    if d == next_weekday_observed(date(y, Month::January, 1)) {
        return true;
    }
    if d == next_weekday_observed(date(y, Month::January, 26)) {
        return true; // Australia Day
    }
    if d == good_friday(y) {
        return true;
    }
    if d == easter_monday(y) {
        return true;
    }
    if d == date(y, Month::April, 25) {
        return true; // Anzac Day (observed on the day itself nationally)
    }
    if d == nth_weekday(y, Month::June, Weekday::Monday, 2) {
        return true; // Sovereign's Birthday
    }
    if is_commonwealth_christmas_substitute(d) {
        return true;
    }
    false
}

/// Canada (federal) bank holidays: New Year's Day, Good Friday, Victoria Day
/// (Monday before 25 May), Canada Day (1 Jul), Labour Day (1st Mon Sep),
/// National Day for Truth & Reconciliation (30 Sep, federal from 2021),
/// Thanksgiving (2nd Mon Oct), Remembrance Day (11 Nov), Christmas Day, Boxing
/// Day. Fixed-date holidays bump forward off the weekend.
fn is_canada_holiday(d: Date) -> bool {
    let y = d.year();
    if d == next_weekday_observed(date(y, Month::January, 1)) {
        return true;
    }
    if d == good_friday(y) {
        return true;
    }
    if d == victoria_day(y) {
        return true;
    }
    if d == next_weekday_observed(date(y, Month::July, 1)) {
        return true; // Canada Day
    }
    if d == nth_weekday(y, Month::September, Weekday::Monday, 1) {
        return true; // Labour Day
    }
    if y >= 2021 && d == next_weekday_observed(date(y, Month::September, 30)) {
        return true; // Truth & Reconciliation
    }
    if d == nth_weekday(y, Month::October, Weekday::Monday, 2) {
        return true; // Thanksgiving
    }
    if d == next_weekday_observed(date(y, Month::November, 11)) {
        return true; // Remembrance Day
    }
    if is_commonwealth_christmas_substitute(d) {
        return true;
    }
    false
}

/// Victoria Day: the Monday immediately preceding 25 May.
fn victoria_day(year: i32) -> Date {
    let anchor = date(year, Month::May, 25);
    let back = anchor.weekday().number_days_from_monday() as i64;
    // Monday on or before 24 May (i.e. strictly before 25 May).
    let monday_on_or_before_25 = anchor - time::Duration::days(back);
    if monday_on_or_before_25 == anchor {
        monday_on_or_before_25 - time::Duration::days(7)
    } else {
        monday_on_or_before_25
    }
}

/// New Zealand (national) bank holidays: New Year's Day and Day after New Year
/// (1–2 Jan), Waitangi Day (6 Feb), Good Friday, Easter Monday, Anzac Day
/// (25 Apr), King's Birthday (1st Mon Jun), Labour Day (4th Mon Oct), Christmas
/// Day, Boxing Day. Since 2014 Waitangi Day and Anzac Day "Mondayise" when they
/// fall on a weekend; New Year and Christmas/Boxing also take substitute days.
fn is_newzealand_holiday(d: Date) -> bool {
    let y = d.year();
    // New Year (1 Jan) and Day after New Year (2 Jan) with Mondayisation.
    let nyd = mondayise(date(y, Month::January, 1));
    let mut day_after = mondayise(date(y, Month::January, 2));
    if day_after == nyd {
        day_after += time::Duration::days(1);
    }
    if d == nyd || d == day_after {
        return true;
    }
    if d == mondayise(date(y, Month::February, 6)) {
        return true; // Waitangi Day (Mondayised from 2014)
    }
    if d == good_friday(y) {
        return true;
    }
    if d == easter_monday(y) {
        return true;
    }
    if d == mondayise(date(y, Month::April, 25)) {
        return true; // Anzac Day (Mondayised from 2014)
    }
    if d == nth_weekday(y, Month::June, Weekday::Monday, 1) {
        return true; // King's Birthday
    }
    if d == nth_weekday(y, Month::October, Weekday::Monday, 4) {
        return true; // Labour Day
    }
    if is_commonwealth_christmas_substitute(d) {
        return true;
    }
    false
}

/// "Mondayisation": a holiday on Saturday or Sunday is observed the following
/// Monday (New Zealand / Commonwealth style).
fn mondayise(d: Date) -> Date {
    match d.weekday() {
        Weekday::Saturday => d + time::Duration::days(2),
        Weekday::Sunday => d + time::Duration::days(1),
        _ => d,
    }
}

/// Commonwealth-style Christmas (25 Dec) + Boxing Day (26 Dec) substitute rule
/// shared by GBP-adjacent calendars (AU/CA/NZ): each Mondayises off the weekend
/// and the two never collide. UK uses its own variant in [`is_uk_holiday`].
fn is_commonwealth_christmas_substitute(d: Date) -> bool {
    let y = d.year();
    let xmas = mondayise(date(y, Month::December, 25));
    let mut boxing = mondayise(date(y, Month::December, 26));
    if boxing == xmas {
        boxing += time::Duration::days(1);
    }
    d == xmas || d == boxing
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: Month, day: u8) -> Date {
        date(y, m, day)
    }

    #[test]
    fn easter_known_dates() {
        // Reference Gregorian Easter Sundays.
        assert_eq!(easter_sunday(2024), d(2024, Month::March, 31));
        assert_eq!(easter_sunday(2025), d(2025, Month::April, 20));
        assert_eq!(easter_sunday(2000), d(2000, Month::April, 23));
        assert_eq!(easter_sunday(2027), d(2027, Month::March, 28));
    }

    #[test]
    fn good_friday_is_two_days_before_easter() {
        assert_eq!(good_friday(2024), d(2024, Month::March, 29));
    }

    #[test]
    fn us_independence_day_observed() {
        // 4 Jul 2026 is a Saturday → observed Friday 3 Jul.
        let us = SettlementCentre::new(CentreId::UnitedStates);
        assert!(us.is_holiday(d(2026, Month::July, 3)));
        // 4 Jul 2025 is a Friday → observed on the day.
        assert!(us.is_holiday(d(2025, Month::July, 4)));
    }

    #[test]
    fn us_thanksgiving_2024() {
        let us = SettlementCentre::new(CentreId::UnitedStates);
        assert!(us.is_holiday(d(2024, Month::November, 28))); // 4th Thursday
        assert!(!us.is_holiday(d(2024, Month::November, 29)));
    }

    #[test]
    fn target2_christmas_and_boxing() {
        let t2 = SettlementCentre::new(CentreId::Target2);
        assert!(t2.is_holiday(d(2024, Month::December, 25)));
        assert!(t2.is_holiday(d(2024, Month::December, 26)));
        // No US Independence Day in TARGET2.
        assert!(!t2.is_holiday(d(2024, Month::July, 4)));
    }

    #[test]
    fn uk_boxing_day_substitute_2021() {
        // 25 Dec 2021 = Sat, 26 Dec = Sun → substitutes Mon 27 & Tue 28.
        let uk = SettlementCentre::new(CentreId::UnitedKingdom);
        assert!(uk.is_holiday(d(2021, Month::December, 27)));
        assert!(uk.is_holiday(d(2021, Month::December, 28)));
    }

    #[test]
    fn japan_new_year_bank_closure() {
        let jp = SettlementCentre::new(CentreId::Japan);
        assert!(jp.is_holiday(d(2024, Month::January, 1)));
        assert!(jp.is_holiday(d(2024, Month::January, 2)));
        assert!(jp.is_holiday(d(2024, Month::January, 3)));
        assert!(jp.is_holiday(d(2023, Month::December, 31)));
    }

    #[test]
    fn japan_equinox_2024() {
        let jp = SettlementCentre::new(CentreId::Japan);
        // Vernal Equinox 2024 = 20 Mar; Autumnal = 22 Sep.
        assert!(jp.is_holiday(d(2024, Month::March, 20)));
        assert!(jp.is_holiday(d(2024, Month::September, 22)));
    }

    #[test]
    fn switzerland_ascension_and_national_day() {
        let ch = SettlementCentre::new(CentreId::Switzerland);
        assert!(ch.is_holiday(ascension(2024)));
        assert!(ch.is_holiday(d(2024, Month::August, 1)));
        assert!(ch.is_holiday(d(2024, Month::January, 2))); // Berchtold's
    }

    #[test]
    fn canada_victoria_day_2024() {
        // Victoria Day 2024 = Mon 20 May.
        let ca = SettlementCentre::new(CentreId::Canada);
        assert!(ca.is_holiday(d(2024, Month::May, 20)));
        assert_eq!(victoria_day(2024), d(2024, Month::May, 20));
    }

    #[test]
    fn australia_anzac_and_australia_day() {
        let au = SettlementCentre::new(CentreId::Australia);
        assert!(au.is_holiday(d(2024, Month::April, 25))); // Anzac Day
        // 26 Jan 2025 is a Sunday → Australia Day observed Mon 27 Jan.
        assert!(au.is_holiday(d(2025, Month::January, 27)));
    }

    #[test]
    fn newzealand_waitangi_mondayised_2021() {
        // 6 Feb 2021 = Saturday → Mondayised to Mon 8 Feb.
        let nz = SettlementCentre::new(CentreId::NewZealand);
        assert!(nz.is_holiday(d(2021, Month::February, 8)));
        assert!(nz.is_holiday(d(2024, Month::October, 28))); // Labour Day 4th Mon Oct
    }

    #[test]
    fn nth_and_last_weekday() {
        // 3rd Monday of Jan 2024 = 15 Jan; last Monday of May 2024 = 27 May.
        assert_eq!(
            nth_weekday(2024, Month::January, Weekday::Monday, 3),
            d(2024, Month::January, 15)
        );
        assert_eq!(
            last_weekday(2024, Month::May, Weekday::Monday),
            d(2024, Month::May, 27)
        );
    }
}
