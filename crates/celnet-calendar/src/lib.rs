//! Celnet FX business-day calendars, spot-lag, delivery, and day-count year
//! fractions (work-stream WS-A).
//!
//! # What this crate computes
//!
//! FX-options pricing is convention-correct only if the **dates** are correct
//! (see `docs/ANALYTICS-SPEC.md` §1.5). This crate is the date engine:
//!
//! - **Holiday rule sets** ([`holiday`]) for the eight covered settlement
//!   centres — USD, EUR (TARGET2), GBP, JPY, CHF, AUD, CAD, NZD — encoded as
//!   per-year rules (fixed dates with each centre's weekend-observance policy,
//!   plus the moving Western-Christian feasts from the computus). Real public
//!   holidays, not placeholders.
//! - **Joint business-day calendars** ([`calendar`]) that **intersect** the two
//!   legs' centres (plus USD for cross-via-USD pairs): a date is good only if
//!   every member centre is open.
//! - **Date-roll conventions** ([`roll`]): modified-following with the
//!   end-of-month rule, plus end-of-month-aware month/year addition.
//! - **Day-count year fractions** ([`daycount`]): ACT/365-fixed vol-time kept
//!   distinct from ACT/360 money-market accrual.
//! - **The FX date chain** ([`fx`]): spot-lag (T+2 default; T+1 for
//!   USDCAD/USDTRY/USDRUB/USDPHP), horizon→spot, tenor→expiry, and
//!   expiry→delivery by the identical spot-lag rule.
//!
//! # Determinism
//!
//! All date arithmetic uses the `time` crate's exact civil-date types. Civil
//! date logic is integer-only **except** for two bounded, documented uses of
//! floating point: (1) [`daycount`] divides an exact integer day count by a
//! fixed denominator, and (2) the Japanese vernal/autumnal equinox holidays use
//! the standard published polynomial approximation, valid for years **1980–2099**
//! (see [`holiday`]); outside that window the equinox day is not guaranteed and
//! callers must restrict expiries accordingly. Both float paths are bit-identical
//! across platforms (no transcendentals), so determinism holds. Tests compare via
//! `celnet_core::assert_close!`. The API is purpose-named and vendor-neutral, and
//! is the date layer consumed by `celnet-conventions`.

#![forbid(unsafe_code)]

pub mod calendar;
pub mod daycount;
pub mod fx;
pub mod holiday;
pub mod roll;
mod weekday;

// Curated re-exports: the names a downstream convention layer reaches for.
pub use calendar::{BusinessCalendar, MAX_CENTRES};
pub use daycount::{actual_days, year_fraction};
pub use fx::{
    FxSchedule, calendar_for, centre_for, delivery_date, expiry_for_tenor, is_t_plus_one_pair,
    schedule, spot_date, spot_lag_days,
};
pub use holiday::{CentreId, SettlementCentre, WeekendRule};
pub use roll::{
    RollRule, add_months, add_weeks, add_years, is_last_business_day_of_month,
    last_business_day_of_month,
};
pub use weekday::is_weekend;

#[cfg(test)]
mod proptests {
    //! Cross-module property tests over the public surface: the financial and
    //! calendrical invariants that must hold for *every* date and tenor.

    use celnet_types::{CcyPair, Tenor};
    use proptest::prelude::*;
    use time::{Date, Duration};

    use crate::{
        calendar_for, delivery_date, expiry_for_tenor, schedule, spot_date, spot_lag_days,
    };

    /// Covered pairs whose every leg resolves to a settlement centre.
    fn covered_pairs() -> Vec<CcyPair> {
        [
            "EURUSD", "GBPUSD", "USDJPY", "USDCHF", "AUDUSD", "USDCAD", "NZDUSD", "EURJPY",
            "EURGBP", "GBPJPY",
        ]
        .iter()
        .map(|s| CcyPair::parse(s).unwrap())
        .collect()
    }

    prop_compose! {
        /// A horizon date drawn from a wide range (covers leap years, year-ends).
        fn arb_date()(days in -3650i64..3650i64) -> Date {
            Date::from_calendar_date(2024, time::Month::January, 1).unwrap() + Duration::days(days)
        }
    }

    fn arb_tenor() -> impl Strategy<Value = Tenor> {
        prop_oneof![
            Just(Tenor::Overnight),
            (1u16..8).prop_map(Tenor::Weeks),
            (1u16..25).prop_map(Tenor::Months),
            (1u16..6).prop_map(Tenor::Years),
        ]
    }

    fn arb_pair() -> impl Strategy<Value = CcyPair> {
        let pairs = covered_pairs();
        (0..pairs.len()).prop_map(move |i| pairs[i])
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Spot is on or after the horizon, and is always a good business day.
        #[test]
        fn spot_is_good_and_not_before_horizon(p in arb_pair(), h in arb_date()) {
            let s = spot_date(p, h);
            prop_assert!(s >= h);
            prop_assert!(calendar_for(p).is_business_day(s));
        }

        /// Delivery is on or after expiry, and is a good business day; expiry is
        /// on or after spot. Hence delivery never precedes spot.
        #[test]
        fn date_chain_is_monotone_and_good(p in arb_pair(), h in arb_date(), t in arb_tenor()) {
            let sch = schedule(p, h, t);
            let cal = calendar_for(p);
            prop_assert!(sch.spot >= sch.horizon);
            prop_assert!(sch.expiry >= sch.spot);
            prop_assert!(sch.delivery >= sch.expiry);
            prop_assert!(sch.delivery >= sch.spot);
            prop_assert!(cal.is_business_day(sch.spot));
            prop_assert!(cal.is_business_day(sch.expiry));
            prop_assert!(cal.is_business_day(sch.delivery));
        }

        /// Delivery is exactly `spot_lag` business days after expiry — the same
        /// rule that maps horizon→spot.
        #[test]
        fn delivery_uses_same_lag_as_spot(p in arb_pair(), h in arb_date(), t in arb_tenor()) {
            let sch = schedule(p, h, t);
            let cal = calendar_for(p);
            let recomputed = cal.add_business_days(sch.expiry, spot_lag_days(p));
            prop_assert_eq!(delivery_date(p, sch.expiry), recomputed);
            prop_assert_eq!(sch.delivery, recomputed);
        }

        /// Spot-lag is symmetric in the leg ordering.
        #[test]
        fn spot_lag_symmetric(p in arb_pair()) {
            let flipped = CcyPair::new(p.quote, p.base);
            prop_assert_eq!(spot_lag_days(p), spot_lag_days(flipped));
        }

        /// Expiry for any tenor lands on a good business day.
        #[test]
        fn expiry_is_business_day(p in arb_pair(), h in arb_date(), t in arb_tenor()) {
            let spot = spot_date(p, h);
            let exp = expiry_for_tenor(p, spot, t);
            prop_assert!(calendar_for(p).is_business_day(exp));
            prop_assert!(exp >= spot);
        }
    }
}
