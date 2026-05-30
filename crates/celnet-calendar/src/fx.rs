//! The FX date engine: currency→centre mapping, spot-lag rules, and the
//! horizon→spot / tenor→expiry / expiry→delivery date chain.
//!
//! This is the public face of the crate consumed by `celnet-conventions`. It
//! resolves a [`CcyPair`] to a joint [`BusinessCalendar`] (intersecting both
//! legs' centres, plus USD for cross-via-USD pairs), applies the pair's
//! **spot lag** (T+2 default; T+1 for USDCAD/USDTRY/USDRUB/USDPHP, see
//! `docs/ANALYTICS-SPEC.md` §1.5), expands standard [`Tenor`]s onto adjusted
//! expiry dates with modified-following + end-of-month, and derives the
//! delivery/settlement date from any expiry by the **same** spot-lag rule.

use celnet_types::{Ccy, CcyPair, Tenor};
use time::Date;

use crate::calendar::BusinessCalendar;
use crate::holiday::CentreId;
use crate::roll::{
    RollRule, add_months, add_weeks, add_years, is_last_business_day_of_month,
    last_business_day_of_month,
};

/// Map a currency to its principal settlement centre.
///
/// Returns `None` for currencies outside the WS-A coverage set (the eight G10
/// majors). Callers needing an exotic leg extend this map before pricing — the
/// engine never silently substitutes a wrong calendar.
#[must_use]
pub fn centre_for(ccy: Ccy) -> Option<CentreId> {
    Some(match ccy {
        Ccy::USD => CentreId::UnitedStates,
        Ccy::EUR => CentreId::Target2,
        Ccy::GBP => CentreId::UnitedKingdom,
        Ccy::JPY => CentreId::Japan,
        Ccy::CHF => CentreId::Switzerland,
        Ccy::AUD => CentreId::Australia,
        Ccy::CAD => CentreId::Canada,
        Ccy::NZD => CentreId::NewZealand,
        _ => return None,
    })
}

/// Spot lag in business days for a pair (horizon→spot and expiry→delivery).
///
/// The market default is **T+2**; a small set of USD pairs settle **T+1**:
/// USDCAD, USDTRY, USDRUB, USDPHP (same-region / North-American convention).
/// The rule is symmetric in the two legs (`USDCAD` == `CADUSD`).
#[must_use]
pub fn spot_lag_days(pair: CcyPair) -> u32 {
    if is_t_plus_one_pair(pair) { 1 } else { 2 }
}

/// Whether `pair` settles T+1 (either ordering of the two legs).
#[must_use]
pub fn is_t_plus_one_pair(pair: CcyPair) -> bool {
    let (a, b) = (pair.base, pair.quote);
    let usd = Ccy::USD;
    let other = if a == usd {
        Some(b)
    } else if b == usd {
        Some(a)
    } else {
        None
    };
    match other {
        Some(c) => c == Ccy::CAD || c == ccy("TRY") || c == ccy("RUB") || c == ccy("PHP"),
        None => false,
    }
}

/// Compile a 3-letter code into a [`Ccy`] (panics on an invalid literal — only
/// used with hard-coded constants here).
fn ccy(code: &str) -> Ccy {
    Ccy::parse(code).expect("valid currency literal")
}

/// The joint business-day calendar for a pair: both legs' centres, plus USD for
/// cross-via-USD pairs (neither leg is USD).
///
/// For majors-vs-USD (e.g. EURUSD) the result is the two-centre intersection;
/// for crosses (e.g. EURJPY) USD is added because the standard market settles
/// the cross through USD legs and the USD calendar therefore gates good days.
///
/// # Panics
/// Panics if either leg is outside the covered currency set (see
/// [`centre_for`]).
#[must_use]
pub fn calendar_for(pair: CcyPair) -> BusinessCalendar {
    let base = centre_for(pair.base).expect("base currency has a settlement centre");
    let quote = centre_for(pair.quote).expect("quote currency has a settlement centre");
    let usd_centre = CentreId::UnitedStates;
    let is_cross = pair.base != Ccy::USD && pair.quote != Ccy::USD;
    if is_cross {
        BusinessCalendar::with_centres([base, quote, usd_centre])
    } else {
        BusinessCalendar::with_centres([base, quote])
    }
}

/// The spot date for a pair given the `horizon` (trade/today) date.
///
/// Adds the pair's spot lag in **good business days** of the joint calendar.
/// The horizon itself need not be a business day; counting begins from the next
/// good day, matching market practice.
#[must_use]
pub fn spot_date(pair: CcyPair, horizon: Date) -> Date {
    let cal = calendar_for(pair);
    cal.add_business_days(horizon, spot_lag_days(pair))
}

/// The delivery/settlement date for a given `expiry`, by the **same** spot-lag
/// rule — delivery is the spot date relative to expiry.
#[must_use]
pub fn delivery_date(pair: CcyPair, expiry: Date) -> Date {
    let cal = calendar_for(pair);
    cal.add_business_days(expiry, spot_lag_days(pair))
}

/// Expand a standard [`Tenor`] into its adjusted **expiry** date, measured from
/// the `spot` date with modified-following and end-of-month rules.
///
/// - [`Tenor::Overnight`] → next business day after `spot` (no EOM/MF month
///   logic; ON is purely the following good day).
/// - Week tenors add calendar weeks, then modified-following.
/// - Month/year tenors add calendar months/years (end-of-month-day-clamped),
///   then apply the **end-of-month rule**: if `spot` is the last business day of
///   its month, the result is the last business day of the target month;
///   otherwise modified-following.
///
/// All dates use the pair's joint calendar.
#[must_use]
pub fn expiry_for_tenor(pair: CcyPair, spot: Date, tenor: Tenor) -> Date {
    let cal = calendar_for(pair);
    match tenor {
        Tenor::Overnight => cal.next_business_day(spot),
        Tenor::Weeks(n) => {
            let cand = add_weeks(spot, i32::from(n));
            RollRule::ModifiedFollowing.adjust(&cal, cand)
        }
        Tenor::Months(n) => roll_period(&cal, spot, add_months(spot, i32::from(n))),
        Tenor::Years(n) => roll_period(&cal, spot, add_years(spot, i32::from(n))),
    }
}

/// Apply the end-of-month / modified-following choice for a month/year period.
fn roll_period(cal: &BusinessCalendar, spot: Date, candidate: Date) -> Date {
    if is_last_business_day_of_month(cal, spot) {
        last_business_day_of_month(cal, candidate)
    } else {
        RollRule::ModifiedFollowing.adjust(cal, candidate)
    }
}

/// The full date schedule for a `(pair, tenor)` priced as of `horizon`:
/// `(spot, expiry, delivery)`.
///
/// This is the one-call entry point conventions code uses: it composes
/// [`spot_date`], [`expiry_for_tenor`] and [`delivery_date`] so the spot-lag and
/// roll rules are applied consistently and exactly once each.
#[must_use]
pub fn schedule(pair: CcyPair, horizon: Date, tenor: Tenor) -> FxSchedule {
    let spot = spot_date(pair, horizon);
    let expiry = expiry_for_tenor(pair, spot, tenor);
    let delivery = delivery_date(pair, expiry);
    FxSchedule {
        horizon,
        spot,
        expiry,
        delivery,
    }
}

/// The resolved FX date chain for a quoted `(pair, tenor)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FxSchedule {
    /// Trade / valuation date the schedule was computed as of.
    pub horizon: Date,
    /// Spot value date (`horizon` + spot lag).
    pub spot: Date,
    /// Option expiry / fixing date.
    pub expiry: Date,
    /// Delivery / settlement date (`expiry` + spot lag).
    pub delivery: Date,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).unwrap()
    }

    fn pair(s: &str) -> CcyPair {
        CcyPair::parse(s).unwrap()
    }

    #[test]
    fn spot_lag_defaults_and_exceptions() {
        assert_eq!(spot_lag_days(pair("EURUSD")), 2);
        assert_eq!(spot_lag_days(pair("GBPJPY")), 2);
        assert_eq!(spot_lag_days(pair("USDCAD")), 1);
        assert_eq!(spot_lag_days(pair("CADUSD")), 1); // symmetric
        // USDTRY/USDRUB/USDPHP are T+1 even though not in the covered centre set
        // for full calendars; the lag rule is currency-pair-level.
        assert_eq!(spot_lag_days(pair("USDTRY")), 1);
        assert_eq!(spot_lag_days(pair("USDPHP")), 1);
    }

    #[test]
    fn eurusd_spot_t_plus_2() {
        // Mon 3 Jun 2024 + 2 business days = Wed 5 Jun 2024 (no holidays).
        let s = spot_date(pair("EURUSD"), d(2024, Month::June, 3));
        assert_eq!(s, d(2024, Month::June, 5));
    }

    #[test]
    fn usdcad_spot_t_plus_1() {
        // Mon 3 Jun 2024 + 1 business day = Tue 4 Jun 2024.
        let s = spot_date(pair("USDCAD"), d(2024, Month::June, 3));
        assert_eq!(s, d(2024, Month::June, 4));
    }

    #[test]
    fn spot_skips_us_holiday_in_eurusd() {
        // Horizon Tue 2 Jul 2024. EURUSD T+2 must skip Thu 4 Jul (US holiday).
        // Wed 3 Jul (good), then Thu 4 Jul skipped, Fri 5 Jul → spot.
        let s = spot_date(pair("EURUSD"), d(2024, Month::July, 2));
        assert_eq!(s, d(2024, Month::July, 5));
    }

    #[test]
    fn delivery_never_before_spot() {
        // For any tenor the delivery (expiry + lag) must be >= spot (horizon + lag).
        let p = pair("EURUSD");
        let horizon = d(2024, Month::June, 3);
        let sch = schedule(p, horizon, Tenor::Months(1));
        assert!(sch.spot >= sch.horizon);
        assert!(sch.expiry >= sch.spot);
        assert!(sch.delivery >= sch.expiry);
        assert!(sch.delivery >= sch.spot);
    }

    #[test]
    fn one_month_expiry_and_delivery() {
        // EURUSD, spot Wed 5 Jun 2024, 1M → 5 Jul 2024 is a Friday & business day.
        let p = pair("EURUSD");
        let spot = d(2024, Month::June, 5);
        let exp = expiry_for_tenor(p, spot, Tenor::Months(1));
        assert_eq!(exp, d(2024, Month::July, 5));
        // Delivery = expiry + 2 business days = Tue 9 Jul (Fri 5 → Mon 8 → Tue 9).
        let del = delivery_date(p, exp);
        assert_eq!(del, d(2024, Month::July, 9));
    }

    #[test]
    fn end_of_month_rule_pins_month_end() {
        // GBPUSD. Choose a spot that is the last business day of its month, then
        // 1M must land on the last business day of the next month.
        let p = pair("GBPUSD");
        let cal = calendar_for(p);
        // Last business day of Aug 2024 is Fri 30 Aug.
        let spot = d(2024, Month::August, 30);
        assert!(is_last_business_day_of_month(&cal, spot));
        let exp = expiry_for_tenor(p, spot, Tenor::Months(1));
        // Last business day of Sep 2024 is Mon 30 Sep.
        assert_eq!(exp, d(2024, Month::September, 30));
        assert!(is_last_business_day_of_month(&cal, exp));
    }

    #[test]
    fn overnight_is_next_business_day() {
        let p = pair("EURUSD");
        // Fri 7 Jun 2024 → ON = Mon 10 Jun.
        assert_eq!(
            expiry_for_tenor(p, d(2024, Month::June, 7), Tenor::Overnight),
            d(2024, Month::June, 10)
        );
    }

    #[test]
    fn cross_pair_includes_usd_calendar() {
        // EURJPY is a cross → calendar must include USD; 4 Jul 2024 (US holiday)
        // is not a good day even though TARGET2 and Tokyo are open.
        let cal = calendar_for(pair("EURJPY"));
        assert_eq!(cal.centres().count(), 3);
        assert!(!cal.is_business_day(d(2024, Month::July, 4)));
    }

    #[test]
    fn week_tenor_modified_following() {
        let p = pair("EURUSD");
        // Spot Wed 5 Jun 2024 + 1W = Wed 12 Jun (business) → unchanged.
        assert_eq!(
            expiry_for_tenor(p, d(2024, Month::June, 5), Tenor::Weeks(1)),
            d(2024, Month::June, 12)
        );
    }
}
