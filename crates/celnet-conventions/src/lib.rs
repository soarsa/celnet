//! Celnet per-`(pair, tenor)` FX-options convention registry (work-stream WS-A).
//!
//! # What this crate is
//!
//! FX-options conventions — which delta is quoted, what "ATM" means, the
//! currency the premium is paid in, the expiry cut, the day-counts, and the
//! settlement style — are **first-class per-`(pair, tenor)` data, never global
//! defaults** (`docs/ANALYTICS-SPEC.md` §1.1, `docs/CONVENTIONS.md`). USDJPY,
//! EURUSD and an EM NDF differ materially, and the *same* pair changes delta
//! convention with tenor. Convention error dwarfs model error, so this crate
//! makes every choice explicit and resolvable.
//!
//! # API
//!
//! - [`ConventionRecord`] — the fully resolved set of conventions for one
//!   `(pair, tenor)`.
//! - [`ResolvedConvention`] / [`ResolutionSource`] — a record plus the
//!   provenance of how it was resolved (bespoke pair profile vs region default).
//! - [`resolve`] — the registry entry point: a pure function of `(pair, tenor)`.
//! - [`has_pair_profile`] / [`has_calendar_support`] — capability predicates.
//!
//! # Coverage
//!
//! Bespoke profiles encode real OTC market practice for the G10 majors
//! (EURUSD, USDJPY, GBPUSD, AUDUSD, USDCHF, USDCAD, NZDUSD) and a
//! non-deliverable example (USDKRW). Pairs without a bespoke profile resolve to
//! an explicit region default keyed off the quote (numeraire) currency. The
//! headline term-structure rule — **spot delta for short tenors, forward
//! (driftless) delta beyond 1Y** — is applied uniformly (USDJPY ≤1Y spot
//! premium-adjusted, >1Y forward premium-adjusted, is the worked example).
//!
//! # Integration with `celnet-calendar`
//!
//! Date/cut resolution is delegated to `celnet-calendar`: this crate re-exports
//! [`schedule`] (horizon→spot→expiry→delivery) and adds [`vol_year_fraction`]
//! for the ACT/365-fixed vol-time on the record's day-count, so a caller
//! resolves *conventions and dates together* from one place. The convention
//! record names the day-count; the calendar computes the year fraction on it.

#![forbid(unsafe_code)]

mod record;
mod registry;

pub use record::{ConventionRecord, ResolutionSource, ResolvedConvention};
pub use registry::{has_calendar_support, has_pair_profile, resolve};

use celnet_calendar::{FxSchedule, schedule as calendar_schedule, year_fraction};
use celnet_types::{CcyPair, Tenor};
use time::Date;

/// The FX date chain (horizon→spot→expiry→delivery) for a `(pair, tenor)`,
/// delegated to `celnet-calendar`. Re-exported here so the convention layer is
/// the one-stop resolver for both conventions and dates.
#[must_use]
pub fn schedule(pair: CcyPair, horizon: Date, tenor: Tenor) -> FxSchedule {
    calendar_schedule(pair, horizon, tenor)
}

/// Vol-time year fraction (spot→expiry) for a `(pair, tenor)` on the resolved
/// convention's vol day-count.
///
/// Combines [`resolve`] (to read `day_count_vol`, always ACT/365-fixed for FX
/// vol-time) with [`schedule`] (for the spot and expiry dates) and
/// `celnet-calendar`'s exact day-count. This is the `T` fed to the
/// Garman-Kohlhagen model for this `(pair, tenor)`.
#[must_use]
pub fn vol_year_fraction(pair: CcyPair, horizon: Date, tenor: Tenor) -> f64 {
    let resolved = resolve(pair, tenor);
    let sch = schedule(pair, horizon, tenor);
    year_fraction(resolved.record.day_count_vol, sch.spot, sch.expiry).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{AtmConvention, Cut, DayCount, DeltaConvention, PremiumStyle, Settlement};

    fn pair(s: &str) -> CcyPair {
        CcyPair::parse(s).unwrap()
    }

    /// Local mirror of the "long tenor" rule (> 12 months) for cross-checking
    /// the resolved spot/forward axis without reaching into registry internals.
    fn expect_forward(t: Tenor) -> bool {
        let months = match t {
            Tenor::Overnight => 0,
            Tenor::Weeks(w) => u32::from(w) / 5,
            Tenor::Months(m) => u32::from(m),
            Tenor::Years(y) => u32::from(y) * 12,
        };
        months > 12
    }

    #[test]
    fn eurusd_three_month_is_spot_premium_adjusted_dns_ny() {
        let r = resolve(pair("EURUSD"), Tenor::Months(3)).record;
        assert_eq!(r.delta, DeltaConvention::SpotPremiumAdjusted);
        assert_eq!(r.atm, AtmConvention::DeltaNeutralStraddle);
        assert_eq!(r.premium_style, PremiumStyle::PercentForeign);
        assert_eq!(r.cut, Cut::NewYork1000);
        assert_eq!(r.day_count_vol, DayCount::Act365Fixed);
        assert_eq!(r.settlement, Settlement::Deliverable);
        assert!(r.is_delta_premium_adjusted());
        assert!(!r.is_delta_forward());
        assert!(r.is_consistent());
    }

    #[test]
    fn usdjpy_term_structure_spot_le_1y_forward_beyond() {
        // ≤ 1Y → spot premium-adjusted (the §1.2 worked example).
        let short = resolve(pair("USDJPY"), Tenor::Months(6)).record;
        assert_eq!(short.delta, DeltaConvention::SpotPremiumAdjusted);
        assert_eq!(short.cut, Cut::Tokyo1500);

        let one_year = resolve(pair("USDJPY"), Tenor::Years(1)).record;
        assert_eq!(one_year.delta, DeltaConvention::SpotPremiumAdjusted);

        // > 1Y → forward premium-adjusted.
        let long = resolve(pair("USDJPY"), Tenor::Years(2)).record;
        assert_eq!(long.delta, DeltaConvention::ForwardPremiumAdjusted);
        assert!(long.is_delta_forward());
        assert!(long.is_delta_premium_adjusted());
        // Premium-adjustment is tenor-invariant; only the spot/forward axis moves.
        assert_eq!(
            short.is_delta_premium_adjusted(),
            long.is_delta_premium_adjusted()
        );
        assert_eq!(short.cut, long.cut);
        assert!(short.is_consistent() && long.is_consistent());
    }

    #[test]
    fn gbpusd_is_domestic_premium_unadjusted() {
        let r = resolve(pair("GBPUSD"), Tenor::Months(3)).record;
        assert_eq!(r.delta, DeltaConvention::SpotUnadjusted);
        assert_eq!(r.premium_style, PremiumStyle::DomesticPips);
        assert!(!r.is_delta_premium_adjusted());
        assert!(r.is_consistent());
        // GBP money-market accrual is ACT/365-fixed, not ACT/360.
        assert_eq!(r.day_count_accrual_for, DayCount::Act365Fixed);
        // Beyond 1Y it stays unadjusted but goes forward.
        let long = resolve(pair("GBPUSD"), Tenor::Years(2)).record;
        assert_eq!(long.delta, DeltaConvention::ForwardUnadjusted);
    }

    #[test]
    fn nzdusd_and_audusd_are_domestic_unadjusted() {
        for p in ["NZDUSD", "AUDUSD"] {
            let r = resolve(pair(p), Tenor::Months(1)).record;
            assert_eq!(r.delta, DeltaConvention::SpotUnadjusted, "{p}");
            assert_eq!(r.premium_style, PremiumStyle::DomesticPips, "{p}");
            assert_eq!(r.day_count_accrual_for, DayCount::Act365Fixed, "{p}");
            assert!(r.is_consistent());
        }
    }

    #[test]
    fn usdchf_and_usdcad_are_foreign_premium_adjusted() {
        for p in ["USDCHF", "USDCAD"] {
            let r = resolve(pair(p), Tenor::Months(3)).record;
            assert_eq!(r.delta, DeltaConvention::SpotPremiumAdjusted, "{p}");
            assert_eq!(r.premium_style, PremiumStyle::PercentForeign, "{p}");
            assert_eq!(r.settlement, Settlement::Deliverable, "{p}");
            assert!(r.is_consistent());
        }
    }

    #[test]
    fn usdkrw_is_non_deliverable_tokyo_cut() {
        let resolved = resolve(pair("USDKRW"), Tenor::Months(3));
        assert_eq!(resolved.source, ResolutionSource::PairProfile);
        let r = resolved.record;
        assert_eq!(r.settlement, Settlement::NonDeliverable);
        assert!(r.is_non_deliverable());
        assert_eq!(r.cut, Cut::Tokyo1500);
        assert_eq!(r.premium_style, PremiumStyle::PercentForeign);
        assert!(r.is_delta_premium_adjusted());
        assert!(r.is_consistent());
    }

    #[test]
    fn region_fallback_for_uncovered_pair() {
        // EURGBP has no bespoke profile → region default (NY, quote = GBP).
        let resolved = resolve(pair("EURGBP"), Tenor::Months(3));
        assert_eq!(resolved.source, ResolutionSource::RegionDefault);
        assert_eq!(resolved.record.cut, Cut::NewYork1000);
        assert!(resolved.record.is_consistent());

        // A JPY-quoted uncovered pair (EURJPY) → Tokyo region default.
        let jpy = resolve(pair("EURJPY"), Tenor::Months(3));
        assert_eq!(jpy.source, ResolutionSource::RegionDefault);
        assert_eq!(jpy.record.cut, Cut::Tokyo1500);
        assert!(jpy.record.is_consistent());
    }

    #[test]
    fn every_resolved_record_is_self_consistent_across_pairs_and_tenors() {
        let pairs = [
            "EURUSD", "USDJPY", "GBPUSD", "AUDUSD", "USDCHF", "USDCAD", "NZDUSD", "USDKRW",
            "EURGBP", "EURJPY", "GBPJPY",
        ];
        let tenors = [
            Tenor::Overnight,
            Tenor::Weeks(2),
            Tenor::Months(1),
            Tenor::Months(6),
            Tenor::Years(1),
            Tenor::Years(2),
            Tenor::Years(5),
        ];
        for p in pairs {
            let cp = pair(p);
            for t in tenors {
                let r = resolve(cp, t).record;
                assert!(r.is_consistent(), "{p} {t:?} inconsistent");
                assert_eq!(r.is_delta_forward(), expect_forward(t), "{p} {t:?} axis");
                // Vol-time day-count is always ACT/365-fixed.
                assert_eq!(r.day_count_vol, DayCount::Act365Fixed, "{p} {t:?}");
            }
        }
    }

    #[test]
    fn calendar_support_predicate() {
        assert!(has_calendar_support(pair("EURUSD")));
        assert!(has_calendar_support(pair("USDJPY")));
        // KRW has no settlement centre in the calendar layer.
        assert!(!has_calendar_support(pair("USDKRW")));
    }

    #[test]
    fn vol_year_fraction_uses_act365_and_matches_calendar() {
        let horizon = Date::from_calendar_date(2024, time::Month::March, 1).unwrap();
        let cp = pair("EURUSD");
        let t = vol_year_fraction(cp, horizon, Tenor::Months(3));
        let sch = schedule(cp, horizon, Tenor::Months(3));
        let expected = year_fraction(DayCount::Act365Fixed, sch.spot, sch.expiry).0;
        celnet_core::assert_close!(t, expected);
        assert!(t > 0.2 && t < 0.3, "3M vol-time ~0.25, got {t}");
    }
}
