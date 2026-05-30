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

/// The approximate number of calendar days in a tenor, used to apply the
/// short-vs-long delta term-structure rule on a single common axis so that
/// week-expressed and month/year-expressed tenors are classified consistently.
///
/// Weeks are 7 days each; months are approximated at 365/12 days and years at
/// 365 days (the same ACT/365 basis vol-time uses). This is a coarse axis used
/// only for the short/long delta split, not for date arithmetic (that lives in
/// `celnet-calendar`).
#[must_use]
fn tenor_days(tenor: Tenor) -> u32 {
    match tenor {
        Tenor::Overnight => 1,
        Tenor::Weeks(w) => u32::from(w) * 7,
        // 365/12 ≈ 30.4167 days/month, rounded to nearest day.
        Tenor::Months(m) => (u32::from(m) * 365 + 6) / 12,
        Tenor::Years(y) => u32::from(y) * 365,
    }
}

/// Whether a tenor is "long" for delta-convention purposes: strictly beyond one
/// year. At and below 1Y is "short". This is the §1.2 threshold ("long tenors >
/// ~1–2Y use forward/driftless delta"); we cut at 1Y, which is the conservative,
/// widely-quoted boundary and matches the USDJPY worked example ("USDJPY <= 1Y
/// uses spot premium-adjusted delta").
///
/// The comparison is on an approximate-day axis ([`tenor_days`]) so that a
/// long tenor expressed in weeks (e.g. `Weeks(60)` ≈ 13.8 months) is correctly
/// classified as long rather than being mis-bucketed by an ad-hoc weeks→months
/// integer heuristic. Exactly 12 months / 1 year (= 365 days) is short; 366+
/// days is long.
#[must_use]
fn is_long_tenor(tenor: Tenor) -> bool {
    tenor_days(tenor) > 365
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
    /// Re-express this profile (written for the canonical market-quotation
    /// orientation) in the **inverted orientation** (base↔quote swapped).
    ///
    /// The premium is paid in the same physical currency, but that currency's
    /// role flips between foreign (base) and domestic (quote), so the premium
    /// style — and hence the premium-adjusted flag that drives the delta — flips
    /// too. The two money-market accrual legs (FOR/DOM) likewise swap. ATM, cut
    /// and settlement are pair properties, invariant under orientation. Failing
    /// to apply this transform mislabels premium-adjustment for inverted majors
    /// (e.g. `CADUSD` vs `USDCAD`), the exact "convention error dwarfs model
    /// error" failure this crate exists to prevent.
    #[must_use]
    fn inverted(self) -> PairProfile {
        PairProfile {
            atm: self.atm,
            premium_style: self.premium_style.flip_orientation(),
            cut: self.cut,
            day_count_accrual_for: self.day_count_accrual_dom,
            day_count_accrual_dom: self.day_count_accrual_for,
            settlement: self.settlement,
        }
    }

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

/// Money-market accrual basis for a currency's rate leg — the **single source of
/// truth** per currency.
///
/// Accrual day-count is a property of the currency, not of whether a pair
/// happens to carry a bespoke profile, so both [`PairProfile`] construction and
/// the region default derive their accrual legs from here. The sterling bloc
/// (GBP) and the Antipodean dollars (AUD, NZD) accrue ACT/365-fixed; the
/// remaining majors accrue ACT/360. This is the conventional money-market
/// day-count, not the vol-time basis.
#[must_use]
fn accrual_basis(ccy: Ccy) -> DayCount {
    if ccy == Ccy::GBP || ccy == Ccy::AUD || ccy == Ccy::NZD {
        DayCount::Act365Fixed
    } else {
        DayCount::Act360
    }
}

/// The bespoke profile for a covered pair, if one exists, **expressed in the
/// queried orientation**.
///
/// The covered set is the G10 majors named in `docs/ANALYTICS-SPEC.md` plus the
/// USDKRW non-deliverable example. Each entry encodes that pair's real OTC
/// market practice in its canonical market-quotation ordering. Lookups are tried
/// in both leg orderings so `CADUSD` resolves from the `USDCAD` profile — but
/// when the queried pair is the *inverted* orientation, the profile is
/// transformed via [`PairProfile::inverted`] so premium-adjustment, the delta,
/// and the accrual legs are correct for the caller's orientation rather than
/// copied verbatim from the canonical one.
#[must_use]
fn pair_profile(pair: CcyPair) -> Option<PairProfile> {
    let resolution = canonicalize(pair)?;
    let canonical = profile_for_canonical(resolution.key);
    Some(if resolution.flipped {
        canonical.inverted()
    } else {
        canonical
    })
}

/// The result of resolving a queried pair to a covered canonical key.
struct Canonical {
    /// The canonical (market-quotation ordering) six-byte key.
    key: [u8; 6],
    /// Whether the queried pair was the *inverted* orientation of `key`.
    flipped: bool,
}

/// The canonical key for a covered pair plus whether the queried pair was the
/// inverted orientation, or `None` if the pair is not in the covered set.
#[must_use]
fn canonicalize(pair: CcyPair) -> Option<Canonical> {
    let direct = key(pair);
    if is_covered_key(direct) {
        return Some(Canonical {
            key: direct,
            flipped: false,
        });
    }
    let flipped = key(CcyPair::new(pair.quote, pair.base));
    if is_covered_key(flipped) {
        return Some(Canonical {
            key: flipped,
            flipped: true,
        });
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
///
/// The money-market accrual legs are derived from [`accrual_basis`] keyed on the
/// canonical pair's FOR/DOM currencies — the single source of truth per currency
/// — so a covered pair and an uncovered cross of the same currency get the same
/// accrual day-count (e.g. AUD is ACT/365 in both covered AUDUSD and an
/// uncovered AUD cross), rather than each profile hard-coding its own.
#[must_use]
fn profile_for_canonical(k: [u8; 6]) -> PairProfile {
    // FOR (base) is bytes 0..3, DOM (quote) is bytes 3..6 of the canonical key.
    let accrual_for = Ccy::new([k[0], k[1], k[2]]).map_or(DayCount::Act360, accrual_basis);
    let accrual_dom = Ccy::new([k[3], k[4], k[5]]).map_or(DayCount::Act360, accrual_basis);
    // Each arm fixes the orientation-independent pair properties (ATM, premium
    // style, cut, settlement) and takes its accrual legs from `accrual_basis`
    // (keyed on the canonical pair's currencies) so the per-currency day-count
    // is the single source of truth.
    match &k {
        // EURUSD: premium in EUR (FOR) → premium-adjusted; DNS ATM; NY cut;
        // physically deliverable. (§1.2, §1.3)
        b"EURUSD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::Deliverable,
        },
        // USDJPY: premium in USD; for USDJPY USD is the FOR (base) leg, so the
        // premium is in the foreign ccy → premium-adjusted. Tokyo cut for the
        // JPY-region business; spot delta ≤1Y, forward >1Y (the §1.2 worked
        // example). Deliverable.
        b"USDJPY" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::Tokyo1500,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::Deliverable,
        },
        // GBPUSD: premium in USD = DOM (quote) → NOT premium-adjusted (the
        // textbook unadjusted-delta major). DNS ATM, NY cut. Deliverable. (§1.2)
        b"GBPUSD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::DomesticPips,
            cut: Cut::NewYork1000,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::Deliverable,
        },
        // AUDUSD: premium in USD = DOM → unadjusted. DNS ATM, NY cut.
        b"AUDUSD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::DomesticPips,
            cut: Cut::NewYork1000,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::Deliverable,
        },
        // USDCHF: premium in USD = FOR (base) → premium-adjusted. DNS, NY cut.
        // Deliverable.
        b"USDCHF" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::Deliverable,
        },
        // USDCAD: premium in USD = FOR (base) → premium-adjusted. DNS, NY cut.
        // T+1 spot lag is handled by the calendar layer, not the convention
        // record. Deliverable.
        b"USDCAD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::Deliverable,
        },
        // NZDUSD: premium in USD = DOM → unadjusted. DNS, NY cut. Deliverable.
        b"NZDUSD" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::DomesticPips,
            cut: Cut::NewYork1000,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::Deliverable,
        },
        // USDKRW: the non-deliverable example. NDOs cash-settle in USD at a
        // published fixing (KFTC18). Premium in USD = FOR (base) → premium-
        // adjusted. DNS ATM, Tokyo cut (Asian fixing region). Non-deliverable.
        // (§1.6)
        b"USDKRW" => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::Tokyo1500,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::NonDeliverable,
        },
        // Unreachable: caller guarantees the key is covered. Fall back to a
        // EURUSD-style G10 profile rather than panic.
        _ => PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
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
    fn tenor_day_count() {
        assert_eq!(tenor_days(Tenor::Overnight), 1);
        assert_eq!(tenor_days(Tenor::Weeks(4)), 28);
        assert_eq!(tenor_days(Tenor::Weeks(10)), 70);
        assert_eq!(tenor_days(Tenor::Months(6)), 183); // (6·365+6)/12
        assert_eq!(tenor_days(Tenor::Months(12)), 365);
        assert_eq!(tenor_days(Tenor::Years(2)), 730);
    }

    #[test]
    fn long_tenor_threshold_is_one_year() {
        assert!(!is_long_tenor(Tenor::Months(12)));
        assert!(!is_long_tenor(Tenor::Years(1)));
        assert!(is_long_tenor(Tenor::Months(18)));
        assert!(is_long_tenor(Tenor::Years(2)));
    }

    #[test]
    fn flipped_legs_resolve_to_orientation_correct_profile() {
        // Both orientations are covered, but the records must NOT be identical:
        // the inverted orientation re-expresses premium/delta/accrual correctly.
        assert!(has_pair_profile(pair("USDCAD")));
        assert!(has_pair_profile(pair("CADUSD")));

        let canon = resolve(pair("USDCAD"), Tenor::Months(3)).record;
        let inv = resolve(pair("CADUSD"), Tenor::Months(3)).record;

        // Canonical USDCAD: premium in USD (FOR/base) → premium-adjusted.
        assert_eq!(canon.premium_style, PremiumStyle::PercentForeign);
        assert!(canon.is_delta_premium_adjusted());
        assert!(canon.is_consistent());

        // Inverted CADUSD: the same USD premium is now in the QUOTE leg →
        // premium-unadjusted; the flag and delta must flip accordingly.
        assert_eq!(inv.premium_style, PremiumStyle::PercentDomestic);
        assert!(!inv.is_delta_premium_adjusted());
        assert!(inv.is_consistent());

        // The records differ precisely because of the orientation transform.
        assert_ne!(canon, inv);

        // Orientation-invariant pair properties are preserved.
        assert_eq!(canon.atm, inv.atm);
        assert_eq!(canon.cut, inv.cut);
        assert_eq!(canon.settlement, inv.settlement);
    }

    #[test]
    fn inverted_orientation_swaps_accrual_legs() {
        // AUDUSD canonical: FOR=AUD (Act365), DOM=USD (Act360).
        let canon = resolve(pair("AUDUSD"), Tenor::Months(6)).record;
        assert_eq!(canon.day_count_accrual_for, DayCount::Act365Fixed);
        assert_eq!(canon.day_count_accrual_dom, DayCount::Act360);

        // USDAUD inverted: legs swap so FOR=USD (Act360), DOM=AUD (Act365).
        let inv = resolve(pair("USDAUD"), Tenor::Months(6)).record;
        assert_eq!(inv.day_count_accrual_for, DayCount::Act360);
        assert_eq!(inv.day_count_accrual_dom, DayCount::Act365Fixed);
    }

    #[test]
    fn accrual_basis_is_single_source_of_truth() {
        // AUD must accrue ACT/365 whether the pair is covered (AUDUSD) or an
        // uncovered cross (AUDJPY via region default), since accrual is a
        // currency property, not a pair property.
        let covered = resolve(pair("AUDUSD"), Tenor::Months(3)).record;
        let cross = resolve(pair("AUDJPY"), Tenor::Months(3));
        assert_eq!(cross.source, ResolutionSource::RegionDefault);
        // AUD is the FOR leg in both.
        assert_eq!(covered.day_count_accrual_for, DayCount::Act365Fixed);
        assert_eq!(cross.record.day_count_accrual_for, DayCount::Act365Fixed);
    }

    #[test]
    fn long_week_tenor_is_classified_long() {
        // Weeks(60) ≈ 420 days > 1Y → must use the forward (long) delta, not be
        // mis-bucketed as short by an ad-hoc weeks→months heuristic.
        assert!(is_long_tenor(Tenor::Weeks(60)));
        // A short week tenor stays short.
        assert!(!is_long_tenor(Tenor::Weeks(4)));
        // 52 weeks (= 364 days) is at/under 1Y → short; 53 weeks is long.
        assert!(!is_long_tenor(Tenor::Weeks(52)));
        assert!(is_long_tenor(Tenor::Weeks(53)));

        // End-to-end through resolve: a long week tenor on a covered premium-
        // adjusted pair yields the FORWARD premium-adjusted delta.
        let rec = resolve(pair("USDJPY"), Tenor::Weeks(60)).record;
        assert!(rec.is_delta_forward());
        assert!(rec.is_delta_premium_adjusted());
    }
}
