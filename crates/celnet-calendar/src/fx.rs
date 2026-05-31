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

use celnet_types::{BrokenDate, Ccy, CcyPair, Tenor};
use time::{Date, Month, Weekday};

use crate::calendar::BusinessCalendar;
use crate::holiday::CentreId;
use crate::roll::{
    RollRule, add_months, add_weeks, add_years, is_last_business_day_of_month,
    last_business_day_of_month,
};

/// An error resolving a [`Tenor`] to a concrete expiry date.
///
/// The standard ladder (ON/TN/SN/Weeks/Months/Years) and [`Tenor::Imm`] with a
/// positive index always resolve; only the two *input-bearing* cases can fail,
/// and they fail loudly rather than silently substituting a wrong date
/// (guardrail #2): an [`Tenor::Imm`] with a zero ordinal, and a
/// [`Tenor::BrokenDate`] whose civil triple is not a real Gregorian date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenorError {
    /// A [`Tenor::Imm`] was given the invalid ordinal `0` (IMM indices are 1-based).
    ImmOrdinalZero,
    /// A [`Tenor::BrokenDate`] carried a civil triple that is not a valid
    /// Gregorian date (e.g. month 13, or 31 February).
    InvalidBrokenDate(BrokenDate),
}

impl core::fmt::Display for TenorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TenorError::ImmOrdinalZero => {
                write!(f, "IMM ordinal must be >= 1 (Tenor::Imm(0) is invalid)")
            }
            TenorError::InvalidBrokenDate(b) => write!(
                f,
                "broken date {:04}-{:02}-{:02} is not a valid Gregorian date",
                b.year, b.month, b.day
            ),
        }
    }
}

impl std::error::Error for TenorError {}

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

/// Resolve a [`Tenor`] to its adjusted **expiry** date.
///
/// The two anchors differ by tenor class, which is *why* the resolver takes both
/// the `horizon` (today / trade date) and the `spot` date:
///
/// - **Pre-spot short end**, anchored on the **horizon**:
///   - [`Tenor::Overnight`] (ON) → the next good business day after `horizon`
///     (a ~T+1 expiry). This corrects the prior bug where ON was resolved
///     relative to `spot` and therefore landed in the spot-next region (~T+3).
///   - [`Tenor::TomNext`] (TN) → the good business day after the ON date.
///   - [`Tenor::SpotNext`] (SN) → the next good business day after `spot`.
/// - **Standard ladder**, anchored on the **spot** date:
///   - Week tenors add calendar weeks, then modified-following.
///   - Month/year tenors add calendar months/years (end-of-month-day-clamped),
///     then apply the **end-of-month rule**: if `spot` is the last business day
///     of its month, the result is the last business day of the target month;
///     otherwise modified-following.
/// - **Exchange / explicit dates**:
///   - [`Tenor::Imm`] → the `n`-th IMM expiry strictly after `horizon` (3rd
///     Wednesday of the Mar/Jun/Sep/Dec cycle), modified-following onto a good
///     day. `n = 0` is rejected with [`TenorError::ImmOrdinalZero`].
///   - [`Tenor::BrokenDate`] → the explicit date, modified-following onto a good
///     day. An impossible civil triple is rejected with
///     [`TenorError::InvalidBrokenDate`].
///
/// All dates use the pair's joint calendar.
///
/// # Errors
/// Returns [`TenorError`] only for the two input-bearing cases: a zero IMM
/// ordinal, or a broken date that is not a real Gregorian date. Every standard
/// ladder / short-end tenor resolves infallibly.
pub fn expiry_for_tenor(
    pair: CcyPair,
    horizon: Date,
    spot: Date,
    tenor: Tenor,
) -> Result<Date, TenorError> {
    let cal = calendar_for(pair);
    Ok(match tenor {
        // Pre-spot short end — anchored on the horizon, not spot.
        Tenor::Overnight => overnight_expiry(&cal, horizon),
        Tenor::TomNext => cal.next_business_day(overnight_expiry(&cal, horizon)),
        Tenor::SpotNext => cal.next_business_day(spot),
        // Standard ladder — anchored on spot.
        Tenor::Weeks(n) => {
            let cand = add_weeks(spot, i32::from(n));
            RollRule::ModifiedFollowing.adjust(&cal, cand)
        }
        Tenor::Months(n) => roll_period(&cal, spot, add_months(spot, i32::from(n))),
        Tenor::Years(n) => roll_period(&cal, spot, add_years(spot, i32::from(n))),
        // Exchange-defined IMM date.
        Tenor::Imm(n) => {
            if n == 0 {
                return Err(TenorError::ImmOrdinalZero);
            }
            let raw = imm_date(horizon, n);
            RollRule::ModifiedFollowing.adjust(&cal, raw)
        }
        // Explicit broken (odd) expiry date.
        Tenor::BrokenDate(b) => {
            let raw = broken_date_to_civil(b)?;
            RollRule::ModifiedFollowing.adjust(&cal, raw)
        }
    })
}

/// The overnight (ON) expiry for a `horizon`: the next good business day after
/// today. Shared by ON and TN so the two stay consistent.
fn overnight_expiry(cal: &BusinessCalendar, horizon: Date) -> Date {
    cal.next_business_day(horizon)
}

/// Validate a [`BrokenDate`] civil triple into a `time::Date`.
fn broken_date_to_civil(b: BrokenDate) -> Result<Date, TenorError> {
    let month = u8_to_month(b.month).ok_or(TenorError::InvalidBrokenDate(b))?;
    Date::from_calendar_date(b.year, month, b.day).map_err(|_| TenorError::InvalidBrokenDate(b))
}

/// Map a 1-based month number to a [`time::Month`], or `None` if out of range.
fn u8_to_month(m: u8) -> Option<Month> {
    Month::try_from(m).ok()
}

/// The `n`-th IMM expiry (1-based) strictly after `horizon`: the 3rd Wednesday
/// of the March / June / September / December cycle.
///
/// IMM third-Wednesday convention per CME (`docs/TRADING-UNIVERSE-SCALE.md`
/// §2.2). This returns the **raw** civil date; the caller applies the pair's
/// business-day roll so a holiday on the 3rd Wednesday shifts onto a good day.
#[must_use]
pub fn imm_date(horizon: Date, n: u8) -> Date {
    debug_assert!(n >= 1, "IMM ordinal is 1-based");
    let mut found = 0u8;
    // Start scanning from the horizon's own quarter month and walk forward in
    // 3-month steps through the Mar/Jun/Sep/Dec cycle.
    let mut year = horizon.year();
    let mut month = first_imm_month_on_or_after(horizon.month());
    loop {
        let candidate = third_wednesday(year, month);
        if candidate > horizon {
            found += 1;
            if found == n {
                return candidate;
            }
        }
        // Advance to the next IMM month in the cycle.
        (year, month) = next_imm_month(year, month);
    }
}

/// The first IMM cycle month (Mar/Jun/Sep/Dec) at or after the given month.
fn first_imm_month_on_or_after(m: Month) -> Month {
    match m as u8 {
        1..=3 => Month::March,
        4..=6 => Month::June,
        7..=9 => Month::September,
        _ => Month::December,
    }
}

/// The next IMM cycle month after `(year, month)`, rolling the year at December.
fn next_imm_month(year: i32, month: Month) -> (i32, Month) {
    match month {
        Month::March => (year, Month::June),
        Month::June => (year, Month::September),
        Month::September => (year, Month::December),
        _ => (year + 1, Month::March),
    }
}

/// The 3rd Wednesday of a given month/year.
fn third_wednesday(year: i32, month: Month) -> Date {
    let first = Date::from_calendar_date(year, month, 1).expect("first of month is valid");
    // Days to the first Wednesday (0..=6), then add two more weeks.
    let offset = (Weekday::Wednesday.number_days_from_monday() as i64
        - first.weekday().number_days_from_monday() as i64)
        .rem_euclid(7);
    first + time::Duration::days(offset + 14)
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
/// `(spot, expiry, delivery)` plus the vol-time accrual anchor.
///
/// This is the one-call entry point conventions code uses: it composes
/// [`spot_date`], [`expiry_for_tenor`] and [`delivery_date`] so the spot-lag and
/// roll rules are applied consistently and exactly once each.
///
/// # Errors
/// Propagates [`TenorError`] from [`expiry_for_tenor`] for the input-bearing
/// tenor cases (zero IMM ordinal, invalid broken date).
pub fn schedule(pair: CcyPair, horizon: Date, tenor: Tenor) -> Result<FxSchedule, TenorError> {
    let spot = spot_date(pair, horizon);
    let expiry = expiry_for_tenor(pair, horizon, spot, tenor)?;
    let delivery = delivery_date(pair, expiry);
    // Vol-time accrues from spot for the standard ladder, but from the horizon
    // (today) for the pre-spot short end whose expiry lands *before* spot — so
    // ON/TN never produce a negative or zero vol-time.
    let vol_anchor = match tenor {
        Tenor::Overnight | Tenor::TomNext => horizon,
        _ => spot,
    };
    Ok(FxSchedule {
        horizon,
        spot,
        expiry,
        delivery,
        vol_anchor,
    })
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
    /// The date vol-time accrues **from** for this tenor: `spot` for the
    /// standard ladder (`Weeks`/`Months`/`Years`/`SpotNext`/`Imm`/`BrokenDate`),
    /// `horizon` (today) for the pre-spot `Overnight`/`TomNext` short end whose
    /// expiry precedes spot. Vol year-fraction is `vol_anchor → expiry`.
    pub vol_anchor: Date,
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

    /// Resolve a tenor's expiry from a known horizon, asserting it never errors
    /// (every test here uses a resolvable tenor).
    fn exp(p: CcyPair, horizon: Date, tenor: Tenor) -> Date {
        let spot = spot_date(p, horizon);
        expiry_for_tenor(p, horizon, spot, tenor).expect("tenor resolves")
    }

    #[test]
    fn delivery_never_before_spot() {
        // For any tenor the delivery (expiry + lag) must be >= spot (horizon + lag).
        let p = pair("EURUSD");
        let horizon = d(2024, Month::June, 3);
        let sch = schedule(p, horizon, Tenor::Months(1)).unwrap();
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
        let exp = expiry_for_tenor(p, d(2024, Month::June, 3), spot, Tenor::Months(1)).unwrap();
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
        // Horizon two days before spot keeps the EOM rule keyed off the spot date.
        let exp = expiry_for_tenor(p, d(2024, Month::August, 28), spot, Tenor::Months(1)).unwrap();
        // Last business day of Sep 2024 is Mon 30 Sep.
        assert_eq!(exp, d(2024, Month::September, 30));
        assert!(is_last_business_day_of_month(&cal, exp));
    }

    #[test]
    fn overnight_is_next_business_day_after_today_not_spot() {
        // The fixed ON bug: ON is the next good day after the *horizon*, NOT a
        // spot-relative date. Horizon Wed 5 Jun 2024 → ON = Thu 6 Jun (T+1),
        // whereas spot is Fri 7 Jun (T+2): ON must precede spot.
        let p = pair("EURUSD");
        let horizon = d(2024, Month::June, 5);
        let s = spot_date(p, horizon);
        assert_eq!(s, d(2024, Month::June, 7)); // spot is T+2
        let on = exp(p, horizon, Tenor::Overnight);
        assert_eq!(on, d(2024, Month::June, 6)); // ON is T+1, before spot
        assert!(
            on < s,
            "ON must be before spot, not in the spot-next region"
        );
    }

    #[test]
    fn overnight_skips_weekend_from_today() {
        // Fri 7 Jun 2024 horizon → ON = Mon 10 Jun (skip the weekend).
        let p = pair("EURUSD");
        assert_eq!(
            exp(p, d(2024, Month::June, 7), Tenor::Overnight),
            d(2024, Month::June, 10)
        );
    }

    #[test]
    fn tom_next_is_day_after_overnight() {
        // Horizon Wed 5 Jun 2024: ON = Thu 6 Jun, TN = Fri 7 Jun.
        let p = pair("EURUSD");
        let horizon = d(2024, Month::June, 5);
        assert_eq!(exp(p, horizon, Tenor::Overnight), d(2024, Month::June, 6));
        assert_eq!(exp(p, horizon, Tenor::TomNext), d(2024, Month::June, 7));
    }

    #[test]
    fn spot_next_is_day_after_spot() {
        // Horizon Wed 5 Jun 2024: spot = Fri 7 Jun, SN = Mon 10 Jun (skip weekend).
        let p = pair("EURUSD");
        let horizon = d(2024, Month::June, 5);
        let s = spot_date(p, horizon);
        assert_eq!(s, d(2024, Month::June, 7));
        let sn = exp(p, horizon, Tenor::SpotNext);
        assert_eq!(sn, d(2024, Month::June, 10));
        assert!(sn > s, "SN must follow spot");
    }

    #[test]
    fn short_end_ordering_on_tn_sn_distinct_and_monotone() {
        // ON < TN < spot < SN, each distinct (the prior bug collapsed ON onto SN).
        let p = pair("EURUSD");
        let horizon = d(2024, Month::June, 5);
        let on = exp(p, horizon, Tenor::Overnight);
        let tn = exp(p, horizon, Tenor::TomNext);
        let spot = spot_date(p, horizon);
        let sn = exp(p, horizon, Tenor::SpotNext);
        assert!(on < tn, "ON < TN");
        assert!(tn <= spot, "TN <= spot");
        assert!(spot < sn, "spot < SN");
        assert_ne!(
            on, sn,
            "ON must not collapse onto SN (regression of the fixed bug)"
        );
    }

    #[test]
    fn imm_resolves_third_wednesday_of_quarter_cycle() {
        // 2024 IMM third Wednesdays (CME): 20 Mar, 19 Jun, 18 Sep, 18 Dec. The
        // EURUSD joint calendar includes the US holidays, so the Jun IMM rolls
        // off Juneteenth (Wed 19 Jun 2024) onto Thu 20 Jun (modified-following).
        let p = pair("EURUSD");
        assert_eq!(
            exp(p, d(2024, Month::January, 2), Tenor::Imm(1)),
            d(2024, Month::March, 20)
        );
        // 19 Jun 2024 is Juneteenth (US) → rolls to Thu 20 Jun.
        assert_eq!(
            exp(p, d(2024, Month::January, 2), Tenor::Imm(2)),
            d(2024, Month::June, 20)
        );
        // 3rd = Wed 18 Sep, 4th = Wed 18 Dec (both good business days).
        assert_eq!(
            exp(p, d(2024, Month::January, 2), Tenor::Imm(3)),
            d(2024, Month::September, 18)
        );
        assert_eq!(
            exp(p, d(2024, Month::January, 2), Tenor::Imm(4)),
            d(2024, Month::December, 18)
        );
    }

    #[test]
    fn imm_skips_an_imm_in_the_current_month_if_already_passed() {
        // Horizon Wed 20 Mar 2024 is itself the Mar IMM; "strictly after" ⇒ the
        // next IMM is the Jun cycle date (rolled off Juneteenth to Thu 20 Jun).
        let p = pair("EURUSD");
        assert_eq!(
            exp(p, d(2024, Month::March, 20), Tenor::Imm(1)),
            d(2024, Month::June, 20)
        );
    }

    #[test]
    fn imm_raw_third_wednesday_helper() {
        // Validate the raw (pre-roll) helper against published CME third-Wednesday
        // dates: Mar 2024 = Wed 20; Jun 2024 = Wed 19 (before the Juneteenth
        // roll); Mar 2025 = Wed 19; Dec 2025 = Wed 17.
        assert_eq!(
            third_wednesday(2024, Month::March),
            d(2024, Month::March, 20)
        );
        assert_eq!(third_wednesday(2024, Month::June), d(2024, Month::June, 19));
        assert_eq!(
            third_wednesday(2025, Month::March),
            d(2025, Month::March, 19)
        );
        assert_eq!(
            third_wednesday(2025, Month::December),
            d(2025, Month::December, 17)
        );
    }

    #[test]
    fn imm_ordinal_zero_is_rejected() {
        let p = pair("EURUSD");
        let h = d(2024, Month::January, 2);
        let s = spot_date(p, h);
        assert_eq!(
            expiry_for_tenor(p, h, s, Tenor::Imm(0)),
            Err(TenorError::ImmOrdinalZero)
        );
    }

    #[test]
    fn broken_date_resolves_explicit_expiry() {
        // An odd date that is a good business day resolves to itself.
        let p = pair("EURUSD");
        let bd = BrokenDate::new(2024, 7, 17); // Wed 17 Jul 2024, a business day
        assert_eq!(
            exp(p, d(2024, Month::June, 5), Tenor::BrokenDate(bd)),
            d(2024, Month::July, 17)
        );
    }

    #[test]
    fn broken_date_rolls_off_a_weekend() {
        // Sat 13 Jul 2024 → modified-following → Mon 15 Jul 2024.
        let p = pair("EURUSD");
        let bd = BrokenDate::new(2024, 7, 13);
        assert_eq!(
            exp(p, d(2024, Month::June, 5), Tenor::BrokenDate(bd)),
            d(2024, Month::July, 15)
        );
    }

    #[test]
    fn invalid_broken_date_is_rejected() {
        let p = pair("EURUSD");
        let h = d(2024, Month::June, 5);
        let s = spot_date(p, h);
        let bad = BrokenDate::new(2024, 2, 31); // 31 February — impossible
        assert_eq!(
            expiry_for_tenor(p, h, s, Tenor::BrokenDate(bad)),
            Err(TenorError::InvalidBrokenDate(bad))
        );
        let bad_month = BrokenDate::new(2024, 13, 1);
        assert_eq!(
            expiry_for_tenor(p, h, s, Tenor::BrokenDate(bad_month)),
            Err(TenorError::InvalidBrokenDate(bad_month))
        );
    }

    #[test]
    fn short_end_vol_anchor_is_horizon_not_spot() {
        // ON/TN must accrue vol-time from the horizon (today), else spot→expiry
        // would be negative. SN/standard ladder anchor on spot.
        let p = pair("EURUSD");
        let horizon = d(2024, Month::June, 5);
        let on = schedule(p, horizon, Tenor::Overnight).unwrap();
        assert_eq!(on.vol_anchor, horizon);
        assert!(on.expiry > on.vol_anchor, "horizon→ON expiry is positive");
        let tn = schedule(p, horizon, Tenor::TomNext).unwrap();
        assert_eq!(tn.vol_anchor, horizon);
        let sn = schedule(p, horizon, Tenor::SpotNext).unwrap();
        assert_eq!(sn.vol_anchor, sn.spot);
        let m1 = schedule(p, horizon, Tenor::Months(1)).unwrap();
        assert_eq!(m1.vol_anchor, m1.spot);
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
        // Spot Wed 5 Jun 2024 + 1W = Wed 12 Jun (business) → unchanged. Horizon
        // chosen so spot lands on 5 Jun.
        assert_eq!(
            exp(p, d(2024, Month::June, 3), Tenor::Weeks(1)),
            d(2024, Month::June, 12)
        );
    }
}
