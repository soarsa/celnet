//! The per-`(pair, tenor)` convention registry.
//!
//! [`resolve`] is the single entry point: given a [`CcyPair`] and a [`Tenor`] it
//! returns the fully [`ResolvedConvention`] for that point, populated with the
//! real-world market conventions catalogued in `docs/ANALYTICS-SPEC.md` §1.2–§1.6
//! and mapped in `docs/CONVENTIONS.md`.
//!
//! # Resolution strategy
//!
//! 1. **Pair profile.** Pairs with a bespoke market practice (the G10 majors and
//!    the covered NDF example) carry an explicit [`PairProfile`] that encodes
//!    their premium currency, ATM style, cut, and *term structure of the delta
//!    convention* — the headline subtlety being that several pairs use a **spot**
//!    delta for short tenors and switch to a **forward (driftless)** delta for
//!    long tenors (the "≳1–2Y" rule). The canonical worked example is **USDJPY:
//!    spot premium-adjusted ≤ 1Y, forward premium-adjusted beyond** (§1.2).
//! 2. **Region default.** A pair with no bespoke profile is resolved from its
//!    *region* — derived from the quote (numeraire) currency — so a previously
//!    unseen but well-behaved G10-style pair still resolves to a sensible,
//!    explicit convention rather than panicking or guessing per call.
//!
//! All conventions are first-class **data**, never global mutable defaults: the
//! registry is a pure function of `(pair, tenor)`.

use celnet_calendar::centre_for;
use celnet_types::{
    AtmConvention, Ccy, CcyPair, Cut, DayCount, DeltaConvention, PremiumStyle, Settlement, Tenor,
};

use crate::record::{ConventionRecord, ResolutionSource, ResolvedConvention};

/// The number of whole months in a tenor, used to apply the short-vs-long delta
/// term-structure rule. Overnight and weeks are short by construction; years are
/// multiplied out.
#[must_use]
fn tenor_months(tenor: Tenor) -> u32 {
    match tenor {
        Tenor::Overnight => 0,
        // A week is < 1 month; floor to 0 so anything sub-monthly is "short".
        Tenor::Weeks(w) => u32::from(w) / 5,
        Tenor::Months(m) => u32::from(m),
        Tenor::Years(y) => u32::from(y) * 12,
    }
}

/// Whether a tenor is "long" for delta-convention purposes: strictly beyond one
/// year (`> 12M`). At and below 1Y is "short". This is the §1.2 threshold
/// ("long tenors > ~1–2Y use forward/driftless delta"); we cut at 1Y, which is
/// the conservative, widely-quoted boundary and matches the USDJPY worked
/// example ("USDJPY <= 1Y uses spot premium-adjusted delta").
#[must_use]
fn is_long_tenor(tenor: Tenor) -> bool {
    tenor_months(tenor) > 12
}

/// Whether a delta convention is the premium-adjusted variant of its family.
#[must_use]
const fn premium_adjusted_of(forward: bool, premium_adjusted: bool) -> DeltaConvention {
    match (forward, premium_adjusted) {
        (false, false) => DeltaConvention::SpotUnadjusted,
        (false, true) => DeltaConvention::SpotPremiumAdjusted,
        (true, false) => DeltaConvention::ForwardUnadjusted,
        (true, true) => DeltaConvention::ForwardPremiumAdjusted,
    }
}

/// A bespoke market-practice profile for one currency pair.
///
/// The profile is tenor-independent *except* for the delta convention, which is
/// reconstructed per tenor from `premium_adjusted` plus the short/long split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PairProfile {
    /// ATM strike convention for the pair.
    atm: AtmConvention,
    /// Premium quotation style; its `is_premium_adjusted()` drives the delta.
    premium_style: PremiumStyle,
    /// Expiry/fixing cut.
    cut: Cut,
    /// Foreign-leg money-market accrual basis.
    day_count_accrual_for: DayCount,
    /// Domestic-leg money-market accrual basis.
    day_count_accrual_dom: DayCount,
    /// Settlement style (deliverable vs non-deliverable).
    settlement: Settlement,
}

impl PairProfile {
    /// Build the resolved record for this profile at a given tenor, applying the
    /// short(spot)/long(forward) delta term-structure rule. Premium-adjustment
    /// is fixed by the premium style and is invariant across tenors.
    fn record_at(self, tenor: Tenor) -> ConventionRecord {
        let premium_adjusted = self.premium_style.is_premium_adjusted();
        let delta = premium_adjusted_of(is_long_tenor(tenor), premium_adjusted);
        ConventionRecord::new(
            delta,
            self.atm,
            self.premium_style,
            self.cut,
            // Vol-time is always ACT/365-fixed (§1.5).
            DayCount::Act365Fixed,
            self.day_count_accrual_for,
            self.day_count_accrual_dom,
            self.settlement,
        )
    }
}

/// Money-market accrual basis for a currency's domestic-rate leg.
///
/// Most majors accrue ACT/360; the sterling bloc (GBP) accrues ACT/365-fixed.
/// This is the conventional money-market day-count, not the vol-time basis.
#[must_use]
fn accrual_basis(ccy: Ccy) -> DayCount {
    if ccy == Ccy::GBP {
        DayCount::Act365Fixed
    } else {
        DayCount::Act360
    }
}

/// The bespoke profile for a covered pair, if one exists.
///
/// The covered set is the G10 majors named in `docs/ANALYTICS-SPEC.md` plus the
/// USDKRW non-deliverable example. Each entry encodes that pair's real OTC
/// market practice. Lookups are tried in both leg orderings so `CADUSD`
/// resolves to the `USDCAD` profile.
#[must_use]
fn pair_profile(pair: CcyPair) -> Option<PairProfile> {
    let canonical = canonicalize(pair)?;
    Some(profile_for_canonical(canonical))
}

/// The canonical six-letter key for a covered pair (market-quotation ordering),
/// or `None` if the pair is not in the covered set.
#[must_use]
fn canonicalize(pair: CcyPair) -> Option<[u8; 6]> {
    let direct = key(pair);
    if is_covered_key(direct) {
        return Some(direct);
    }
    let flipped = key(CcyPair::new(pair.quote, pair.base));
    if is_covered_key(flipped) {
        return Some(flipped);
    }
    None
}

/// Pack a pair into its six ASCII bytes `BASEQUOTE`.
#[must_use]
fn key(pair: CcyPair) -> [u8; 6] {
    let b = pair.base.as_str().as_bytes();
    let q = pair.quote.as_str().as_bytes();
    [b[0], b[1], b[2], q[0], q[1], q[2]]
}

/// Whether a six-byte key is one of the covered canonical pairs.
#[must_use]
fn is_covered_key(k: [u8; 6]) -> bool {
    matches!(
        &k,
        b"EURUSD"
            | b"USDJPY"
            | b"GBPUSD"
            | b"AUDUSD"
            | b"USDCHF"
            | b"USDCAD"
            | b"NZDUSD"
            | b"USDKRW"
    )
}

/// The profile for a canonical covered key. The key is guaranteed covered by the
/// caller, so the catch-all is unreachable in practice and falls back to the
/// EURUSD-style G10 profile.
#[must_use]
fn profile_for_canonical(k: [u8; 6]) -> PairProfile {
    match &k {
        // EURUSD: premium in EUR (FOR) → premium-adjusted; DNS ATM; NY cut;
        // EUR & USD both accrue ACT/360; physically deliverable. (§1.2, §1.3)
        b"EURUSD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: DayCount::Act360, // EUR
            day_count_accrual_dom: DayCount::Act360, // USD
            settlement: Settlement::Deliverable,
        },
        // USDJPY: premium in USD; for USDJPY USD is the FOR (base) leg, so the
        // premium is in the foreign ccy → premium-adjusted. Tokyo cut for the
        // JPY-region business; spot delta ≤1Y, forward >1Y (the §1.2 worked
        // example). JPY accrues ACT/360; USD ACT/360. Deliverable.
        b"USDJPY" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::Tokyo1500,
            day_count_accrual_for: DayCount::Act360, // USD
            day_count_accrual_dom: DayCount::Act360, // JPY
            settlement: Settlement::Deliverable,
        },
        // GBPUSD: premium in USD = DOM (quote) → NOT premium-adjusted (the
        // textbook unadjusted-delta major). DNS ATM, NY cut. GBP accrues
        // ACT/365-fixed; USD ACT/360. Deliverable. (§1.2)
        b"GBPUSD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::DomesticPips,
            cut: Cut::NewYork1000,
            day_count_accrual_for: DayCount::Act365Fixed, // GBP
            day_count_accrual_dom: DayCount::Act360,      // USD
            settlement: Settlement::Deliverable,
        },
        // AUDUSD: premium in USD = DOM → unadjusted. DNS ATM, NY cut. AUD & USD
        // ACT/365 and ACT/360 respectively (AUD money market is ACT/365).
        b"AUDUSD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::DomesticPips,
            cut: Cut::NewYork1000,
            day_count_accrual_for: DayCount::Act365Fixed, // AUD
            day_count_accrual_dom: DayCount::Act360,      // USD
            settlement: Settlement::Deliverable,
        },
        // USDCHF: premium in USD = FOR (base) → premium-adjusted. DNS, NY cut.
        // USD & CHF ACT/360. Deliverable.
        b"USDCHF" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: DayCount::Act360, // USD
            day_count_accrual_dom: DayCount::Act360, // CHF
            settlement: Settlement::Deliverable,
        },
        // USDCAD: premium in USD = FOR (base) → premium-adjusted. DNS, NY cut.
        // T+1 spot lag is handled by the calendar layer, not the convention
        // record. USD & CAD ACT/360. Deliverable.
        b"USDCAD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: DayCount::Act360, // USD
            day_count_accrual_dom: DayCount::Act360, // CAD
            settlement: Settlement::Deliverable,
        },
        // NZDUSD: premium in USD = DOM → unadjusted. DNS, NY cut. NZD ACT/365,
        // USD ACT/360. Deliverable.
        b"NZDUSD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::DomesticPips,
            cut: Cut::NewYork1000,
            day_count_accrual_for: DayCount::Act365Fixed, // NZD
            day_count_accrual_dom: DayCount::Act360,      // USD
            settlement: Settlement::Deliverable,
        },
        // USDKRW: the non-deliverable example. NDOs cash-settle in USD at a
        // published fixing (KFTC18). Premium in USD = FOR (base) → premium-
        // adjusted. DNS ATM, Tokyo cut (Asian fixing region). USD & KRW ACT/360.
        // Non-deliverable. (§1.6)
        b"USDKRW" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::Tokyo1500,
            day_count_accrual_for: DayCount::Act360, // USD
            day_count_accrual_dom: DayCount::Act360, // KRW
            settlement: Settlement::NonDeliverable,
        },
        // Unreachable: caller guarantees the key is covered. Fall back to a
        // EURUSD-style G10 profile rather than panic.
        _ => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: DayCount::Act360,
            day_count_accrual_dom: DayCount::Act360,
            settlement: Settlement::Deliverable,
        },
    }
}

/// Geographic/quotation region of a pair, used to pick a sensible default for
/// pairs that lack a bespoke profile. The region is derived from the quote
/// (numeraire) currency: JPY-quoted and KRW-quoted business uses the Tokyo cut,
/// everything else the New York cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    /// New York 10:00 region: USD/EUR/GBP/CHF/CAD-numeraire G10 business.
    NewYork,
    /// Tokyo 15:00 region: JPY-numeraire and Asian business.
    Tokyo,
}

/// The region of a pair from its quote (numeraire) currency.
#[must_use]
fn region_of(pair: CcyPair) -> Region {
    if pair.quote == Ccy::JPY {
        Region::Tokyo
    } else {
        Region::NewYork
    }
}

/// Build a region-default record for a pair with no bespoke profile.
///
/// Defaults follow the dominant G10 OTC practice: delta-neutral-straddle ATM,
/// premium in the foreign (base) currency → premium-adjusted delta with the
/// short/long term-structure split, ACT/365-fixed vol-time, money-market accrual
/// per currency, deliverable settlement, and the region's cut.
#[must_use]
fn region_default(pair: CcyPair, tenor: Tenor) -> ConventionRecord {
    let cut = match region_of(pair) {
        Region::NewYork => Cut::NewYork1000,
        Region::Tokyo => Cut::Tokyo1500,
    };
    let premium_style = PremiumStyle::PercentForeign;
    let delta = premium_adjusted_of(is_long_tenor(tenor), premium_style.is_premium_adjusted());
    ConventionRecord::new(
        delta,
        AtmConvention::DeltaNeutralStraddle,
        premium_style,
        cut,
        DayCount::Act365Fixed,
        accrual_basis(pair.base),
        accrual_basis(pair.quote),
        Settlement::Deliverable,
    )
}

/// Resolve the full convention record for a `(pair, tenor)`.
///
/// Tries the bespoke [`PairProfile`] for the pair first; falls back to the
/// region default otherwise. The returned [`ResolvedConvention`] records which
/// path was taken. This is a pure, allocation-free function suitable for the
/// hot path and for dense per-`(pair, tenor)` precomputation over an
/// investment-banking-sized matrix.
#[must_use]
pub fn resolve(pair: CcyPair, tenor: Tenor) -> ResolvedConvention {
    if let Some(profile) = pair_profile(pair) {
        ResolvedConvention {
            record: profile.record_at(tenor),
            source: ResolutionSource::PairProfile,
        }
    } else {
        ResolvedConvention {
            record: region_default(pair, tenor),
            source: ResolutionSource::RegionDefault,
        }
    }
}

/// Whether both legs of a pair map to a known settlement centre in the calendar
/// layer (`celnet-calendar::centre_for`). Conventions can still be *resolved* for
/// uncovered legs via the region default, but date/cut resolution requires both
/// legs to have a centre; this predicate lets callers gate on that.
#[must_use]
pub fn has_calendar_support(pair: CcyPair) -> bool {
    centre_for(pair.base).is_some() && centre_for(pair.quote).is_some()
}

/// Whether the pair has a bespoke (non-default) convention profile.
#[must_use]
pub fn has_pair_profile(pair: CcyPair) -> bool {
    pair_profile(pair).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(s: &str) -> CcyPair {
        CcyPair::parse(s).unwrap()
    }

    #[test]
    fn tenor_month_count() {
        assert_eq!(tenor_months(Tenor::Overnight), 0);
        assert_eq!(tenor_months(Tenor::Weeks(4)), 0); // sub-monthly
        assert_eq!(tenor_months(Tenor::Weeks(10)), 2);
        assert_eq!(tenor_months(Tenor::Months(6)), 6);
        assert_eq!(tenor_months(Tenor::Years(2)), 24);
    }

    #[test]
    fn long_tenor_threshold_is_one_year() {
        assert!(!is_long_tenor(Tenor::Months(12)));
        assert!(!is_long_tenor(Tenor::Years(1)));
        assert!(is_long_tenor(Tenor::Months(18)));
        assert!(is_long_tenor(Tenor::Years(2)));
    }

    #[test]
    fn flipped_legs_resolve_to_same_profile() {
        assert!(has_pair_profile(pair("USDCAD")));
        assert!(has_pair_profile(pair("CADUSD")));
        assert_eq!(
            resolve(pair("USDCAD"), Tenor::Months(3)).record,
            resolve(pair("CADUSD"), Tenor::Months(3)).record
        );
    }
}
