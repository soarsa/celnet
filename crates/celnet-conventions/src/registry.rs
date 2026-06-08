//! The per-`(pair, tenor)` convention registry.
//!
//! [`resolve`] is the single entry point: given a [`CcyPair`] and a [`Tenor`] it
//! returns the fully [`ResolvedConvention`] for that point, populated with the
//! real-world market conventions catalogued in `docs/ANALYTICS-SPEC.md` §1.2–§1.6
//! and mapped in `docs/CONVENTIONS.md`.
//!
//! # Resolution strategy
//!
//! 1. **Pair profile.** Pairs in the covered universe ([`COVERED_PAIRS`] — the
//!    full G10 + Scandi cross matrix, the EM deliverable set, the EMTA NDF panel,
//!    the precious metals XAU/XAG/XPT/XPD vs USD, and the metal crosses) carry an
//!    explicit [`PairProfile`] that encodes their premium currency, ATM style,
//!    cut, settlement, metal leg, and *term structure of the delta convention* —
//!    the headline subtlety being that several pairs use a **spot** delta for
//!    short tenors and switch to a **forward (driftless)** delta for long tenors
//!    (the "≳1–2Y" rule). The canonical worked example is **USDJPY: spot
//!    premium-adjusted ≤ 1Y, forward premium-adjusted beyond** (§1.2).
//! 2. **Region default.** A pair with no bespoke profile is resolved from its
//!    *region* — derived from the quote (numeraire) currency — so a previously
//!    unseen but well-behaved G10-style pair still resolves to a sensible,
//!    explicit convention rather than panicking or guessing per call.
//!
//! All conventions are first-class **data**, never global mutable defaults: the
//! registry is a pure function of `(pair, tenor)`.

use celnet_calendar::{centre_for, spot_lag_days};
use celnet_types::{
    AtmConvention, Ccy, CcyPair, Cut, DayCount, DeltaConvention, FixingSource, PremiumStyle,
    Settlement, Tenor,
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
        // The pre-spot short end and spot-next are all ~1 day for the
        // short/long delta classification (well inside the "short" bucket).
        Tenor::Overnight | Tenor::TomNext | Tenor::SpotNext => 1,
        Tenor::Weeks(w) => u32::from(w) * 7,
        // 365/12 ≈ 30.4167 days/month, rounded to nearest day.
        Tenor::Months(m) => (u32::from(m) * 365 + 6) / 12,
        Tenor::Years(y) => u32::from(y) * 365,
        // The next IMM quarter is ≤ ~3 months out; the n-th is ~3n months. This
        // coarse axis only drives the short/long delta split, so an IMM ≥ ~5
        // quarters is "long". Use 3 months per IMM step as the nominal horizon.
        Tenor::Imm(n) => (u32::from(n) * 3 * 365 + 6) / 12,
        // A broken date carries no standard-unit horizon at this layer; classify
        // it as "short" (≤ 1Y delta convention) — the conservative default that
        // matches the dominant near-dated broken-date flow. The exact axis a
        // broken date prices on is set by the pricer from its resolved expiry,
        // not by this coarse delta-classification heuristic.
        Tenor::BrokenDate(_) => 1,
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

/// The asset class of a covered pair — used to surface metals distinctly in the
/// [`PairMeta`] universe view without changing the wire-facing convention record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentClass {
    /// A spot/forward FX pair between two fiat currencies.
    Fiat,
    /// A precious-metal-vs-fiat pair (e.g. XAUUSD, XAGUSD, XPTUSD, XPDUSD, and
    /// the metal crosses XAUEUR/XAUJPY/…): the **metal is the base** (the
    /// asset/FOR leg), quoted against a fiat currency, settling T+2 loco-London.
    /// The metal carries no money-market interest accrual of its own; in its
    /// place the **lease rate** of the metal is modelled as the foreign (base)
    /// rate by the pricer (see [`MetalLeg`]).
    PreciousMetal,
}

/// The lease-rate-bearing metal leg of a precious-metal pair.
///
/// A precious metal does not pay a deposit rate; instead the holder of the metal
/// can lend it out and earn the **lease rate** (the bullion-market analogue of a
/// foreign deposit rate — the negative of the forward GOFO basis). For carry and
/// forward purposes the metal leg therefore behaves exactly like the foreign
/// (base) leg of an FX pair whose foreign rate is the lease rate. This struct
/// names that convention explicitly so the risk layer reports a *lease rate*, not
/// a mislabelled "foreign deposit rate", while the underlying carry arithmetic is
/// byte-identical to the FX foreign-rate path (ADR-0008 §carry).
///
/// **Only the convention is encoded here; the live lease-rate VALUE is an
/// estate-gated market-data input and is never sourced from this repository.**
/// (Sources: LBMA loco-London good-delivery / forward-rate conventions; LPPM for
/// platinum/palladium.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetalLeg {
    /// The metal that forms the base (asset/FOR) leg, as its ISO-4217 X-code
    /// (`XAU`/`XAG`/`XPT`/`XPD`).
    pub metal: Ccy,
    /// The day-count the metal lease rate accrues on. The loco-London bullion
    /// forward/lease market quotes on an **ACT/360** basis (the same money-market
    /// basis the USD leg uses), so the metal leg carries `Act360`.
    pub lease_day_count: DayCount,
    /// Whether the metal settles **loco-London** (the LBMA/LPPM clearing
    /// location). Every metal in the covered universe does; the flag is explicit
    /// so a future non-London-cleared metal venue is nameable rather than
    /// implied.
    pub loco_london: bool,
}

/// The cash-settlement terms of a non-deliverable pair (NDF/NDO): the published
/// fixing it references and the convertible currency it settles in.
///
/// A non-deliverable option pays the strike-vs-fixing difference in the
/// **settlement currency** (always the convertible leg — USD for the covered
/// USD/EM pairs) at the published [`FixingSource`]. This is convention identity,
/// not market data: the live fixing values stay estate-gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NdfTerms {
    /// The published reference fixing the contract cash-settles against.
    pub fixing: FixingSource,
    /// The currency the net cash settlement is paid in (the convertible leg).
    pub settlement_ccy: Ccy,
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
    /// Non-deliverable cash-settlement terms, present iff `settlement` is
    /// [`Settlement::NonDeliverable`]. The registry guarantees this invariant.
    ndf: Option<NdfTerms>,
    /// Asset class of the pair (fiat vs precious metal).
    instrument: InstrumentClass,
    /// The lease-rate-bearing metal leg, present iff `instrument` is
    /// [`InstrumentClass::PreciousMetal`]. Orientation-invariant: the metal is the
    /// physical base of the pair regardless of quote orientation, so a covered
    /// metal pair is only ever quoted metal-base and this rides through inversion
    /// unchanged.
    metal_leg: Option<MetalLeg>,
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
            // The fixing reference and the physical settlement currency of an NDF
            // are pair properties, invariant under quote-orientation: KRW always
            // cash-settles in USD at KFTC18 whether quoted USDKRW or KRWUSD.
            ndf: self.ndf,
            instrument: self.instrument,
            // The metal leg is the physical base of the pair (the lease-bearing
            // asset), invariant under quote orientation.
            metal_leg: self.metal_leg,
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

/// The settlement class of a covered canonical pair, carried in [`PairSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecKind {
    /// A physically-deliverable fiat (or metal) pair.
    Deliverable,
    /// A non-deliverable (NDF/NDO) pair, cash-settled in USD at the named fixing.
    NonDeliverable(FixingSource),
}

/// One row of the covered-pair universe table: the **single source of truth** for
/// both the covered-set membership ([`is_covered_key`]) and the per-pair profile
/// ([`profile_for_canonical`]). Each row is one canonical (market-quotation
/// ordering) pair and the orientation-independent market conventions it carries.
///
/// The `premium_style` is encoded for the **canonical** orientation; the inverted
/// orientation is derived by [`PairProfile::inverted`]. Accrual day-counts are
/// **not** stored here — they are derived per-currency from [`accrual_basis`] (the
/// single source of truth per currency) when the profile is built.
struct PairSpec {
    /// Canonical six-byte `BASEQUOTE` key.
    key: &'static [u8; 6],
    /// Premium quotation style in the canonical orientation.
    premium_style: PremiumStyle,
    /// Expiry/fixing cut.
    cut: Cut,
    /// Settlement class (deliverable vs non-deliverable + fixing).
    kind: SpecKind,
    /// Asset class (fiat vs precious metal).
    instrument: InstrumentClass,
}

/// The covered pair-universe table — a documented superset exceeding the
/// 75-pair interbank panel. Every convention value is the published market
/// standard (EMTA/ISDA per-currency templates for FX/NDF; LBMA/LPPM for metals),
/// encoded in the canonical orientation.
///
/// # The covered classes
///
/// * **Fiat G10 + Scandi matrix** (the C(10,2) = 45 canonical crosses over
///   {USD, EUR, JPY, GBP, CHF, AUD, CAD, NZD, NOK, SEK}). The seven USD majors
///   carry their bespoke per-pair premium style (the §1.2/§1.3 documented
///   standard, e.g. EURUSD premium in EUR vs GBPUSD premium in USD); every other
///   cross carries the **standard interbank cross convention** — DNS ATM, premium
///   paid in the **quote (DOM)** currency (→ unadjusted, `DomesticPips`), NY cut
///   (Tokyo when the quote is JPY), deliverable, T+2. Liquidity of the thinner
///   Scandi crosses varies, but the *convention* is the uniform market-standard
///   cross convention — this table claims convention correctness, not liquidity.
/// * **EM deliverable** (Gregorian-calendar EM): USDMXN, USDZAR, EURMXN, EURZAR,
///   GBPZAR. USD-base → premium in USD (FOR) premium-adjusted; EUR/GBP-base →
///   premium in the EM quote ccy (DOM) unadjusted (cross convention). NY cut, T+2.
/// * **EM non-deliverable (NDF/NDO)**: USDKRW, USDTWD, USDINR, USDBRL, USDCLP,
///   USDCOP — cash-settled in USD at the published EMTA fixing; premium in USD
///   (FOR) premium-adjusted; Tokyo cut for the Asian fixings, NY for the LatAm.
/// * **Precious metals vs USD**: XAUUSD, XAGUSD, XPTUSD, XPDUSD — metal as base
///   (asset/FOR), USD quote, premium in USD (DOM) unadjusted, DNS, NY cut, T+2
///   loco-London.
/// * **Metal crosses**: gold/silver/platinum/palladium vs a fiat quote (e.g.
///   XAUEUR, XAUJPY, XAGEUR, XPTEUR) — same metal-as-base loco-London convention
///   with the **premium in the fiat quote** (DOM) unadjusted; NY cut (Tokyo when
///   the quote is JPY), T+2. The cross is the loco-London metal leg quoted against
///   the fiat, i.e. metal/USD × USD/quote.
const COVERED_PAIRS: &[PairSpec] = &[
    // ===================================================================
    // Fiat: the seven USD majors with their bespoke documented premium style.
    // ===================================================================
    // EURUSD: premium in EUR (FOR) → premium-adjusted; NY cut.
    fiat(b"EURUSD", PremiumStyle::PercentForeign, Cut::NewYork1000),
    // USDJPY: USD is FOR → premium in the foreign ccy → premium-adjusted; Tokyo
    // cut (the §1.2 spot-≤1Y / forward->1Y worked example).
    fiat(b"USDJPY", PremiumStyle::PercentForeign, Cut::Tokyo1500),
    // GBPUSD: premium in USD = DOM → unadjusted (textbook). NY cut.
    fiat(b"GBPUSD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    // AUDUSD: premium in USD = DOM → unadjusted. NY cut.
    fiat(b"AUDUSD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    // USDCHF: premium in USD = FOR → premium-adjusted. NY cut.
    fiat(b"USDCHF", PremiumStyle::PercentForeign, Cut::NewYork1000),
    // USDCAD: premium in USD = FOR → premium-adjusted. NY cut. (T+1 spot lag is
    // the calendar layer's responsibility.)
    fiat(b"USDCAD", PremiumStyle::PercentForeign, Cut::NewYork1000),
    // NZDUSD: premium in USD = DOM → unadjusted. NY cut.
    fiat(b"NZDUSD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    // ===================================================================
    // Fiat G10 crosses (no USD leg): standard interbank cross convention —
    // premium in the quote (DOM) currency → unadjusted (DomesticPips); NY cut,
    // Tokyo when the quote is JPY. The canonical orientation follows the standard
    // base-currency precedence (EUR>GBP>AUD>NZD>USD>CAD>CHF>NOK>SEK>JPY).
    // ===================================================================
    fiat(b"EURGBP", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"EURJPY", PremiumStyle::DomesticPips, Cut::Tokyo1500),
    fiat(b"EURCHF", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"EURAUD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"EURCAD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"EURNZD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"GBPJPY", PremiumStyle::DomesticPips, Cut::Tokyo1500),
    fiat(b"GBPCHF", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"GBPAUD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"GBPCAD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"GBPNZD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"AUDJPY", PremiumStyle::DomesticPips, Cut::Tokyo1500),
    fiat(b"AUDCHF", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"AUDCAD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"AUDNZD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"NZDJPY", PremiumStyle::DomesticPips, Cut::Tokyo1500),
    fiat(b"NZDCHF", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"NZDCAD", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"CADJPY", PremiumStyle::DomesticPips, Cut::Tokyo1500),
    fiat(b"CADCHF", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"CHFJPY", PremiumStyle::DomesticPips, Cut::Tokyo1500),
    // ===================================================================
    // Scandi (NOK, SEK) — quoted vs USD/EUR/GBP and as the NOKSEK cross. USD-base
    // Scandi carries the documented USD-FOR premium-adjusted; the EUR/GBP-base and
    // NOKSEK crosses use the cross convention (premium in DOM).
    // ===================================================================
    fiat(b"USDNOK", PremiumStyle::PercentForeign, Cut::NewYork1000),
    fiat(b"USDSEK", PremiumStyle::PercentForeign, Cut::NewYork1000),
    fiat(b"EURNOK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"EURSEK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"GBPNOK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"GBPSEK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"AUDNOK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"AUDSEK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"NZDNOK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"NZDSEK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"CADNOK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"CADSEK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"CHFNOK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"CHFSEK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"NOKSEK", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"NOKJPY", PremiumStyle::DomesticPips, Cut::Tokyo1500),
    fiat(b"SEKJPY", PremiumStyle::DomesticPips, Cut::Tokyo1500),
    // ===================================================================
    // EM deliverable (Gregorian-calendar EM: MXN, ZAR). USD-base → premium in USD
    // (FOR) premium-adjusted; EUR/GBP-base → premium in the EM quote (DOM)
    // unadjusted (cross convention). NY cut, T+2.
    // ===================================================================
    fiat(b"USDMXN", PremiumStyle::PercentForeign, Cut::NewYork1000),
    fiat(b"USDZAR", PremiumStyle::PercentForeign, Cut::NewYork1000),
    fiat(b"EURMXN", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"EURZAR", PremiumStyle::DomesticPips, Cut::NewYork1000),
    fiat(b"GBPZAR", PremiumStyle::DomesticPips, Cut::NewYork1000),
    // ===================================================================
    // EM non-deliverable (NDF/NDO), cash-settled in USD at the published fixing.
    // Premium in USD = FOR (base) → premium-adjusted (PercentForeign). Cut follows
    // the fixing region: Tokyo for the Asian fixings, NY for the LatAm.
    // ===================================================================
    ndf(b"USDKRW", FixingSource::KrwKftc18, Cut::Tokyo1500),
    ndf(b"USDTWD", FixingSource::TwdTaipei, Cut::Tokyo1500),
    ndf(b"USDINR", FixingSource::InrRbiRef, Cut::Tokyo1500),
    ndf(b"USDBRL", FixingSource::BrlPtax, Cut::NewYork1000),
    ndf(b"USDCLP", FixingSource::ClpDolarObs, Cut::NewYork1000),
    ndf(b"USDCOP", FixingSource::CopTrm, Cut::NewYork1000),
    // ===================================================================
    // Precious metals vs USD: metal as base (asset/FOR), USD quote, premium in
    // USD = DOM (quote) → unadjusted (the bullion-market standard quotes the
    // premium in USD terms). DNS ATM, NY cut, deliverable loco-London, T+2.
    // ===================================================================
    metal(b"XAUUSD"),
    metal(b"XAGUSD"),
    metal(b"XPTUSD"),
    metal(b"XPDUSD"),
    // ===================================================================
    // Metal crosses: metal as base vs a fiat quote — premium in the fiat quote
    // (DOM) unadjusted; NY cut (Tokyo when the quote is JPY), T+2 loco-London.
    // The cross is the loco-London metal leg quoted against the fiat
    // (metal/USD × USD/quote).
    // ===================================================================
    metal_cross(b"XAUEUR", Cut::NewYork1000),
    metal_cross(b"XAUJPY", Cut::Tokyo1500),
    metal_cross(b"XAUGBP", Cut::NewYork1000),
    metal_cross(b"XAUCHF", Cut::NewYork1000),
    metal_cross(b"XAUAUD", Cut::NewYork1000),
    metal_cross(b"XAUCAD", Cut::NewYork1000),
    metal_cross(b"XAUNZD", Cut::NewYork1000),
    metal_cross(b"XAUNOK", Cut::NewYork1000),
    metal_cross(b"XAUSEK", Cut::NewYork1000),
    metal_cross(b"XAGEUR", Cut::NewYork1000),
    metal_cross(b"XAGJPY", Cut::Tokyo1500),
    metal_cross(b"XAGGBP", Cut::NewYork1000),
    metal_cross(b"XAGCHF", Cut::NewYork1000),
    metal_cross(b"XPTEUR", Cut::NewYork1000),
    metal_cross(b"XPTJPY", Cut::Tokyo1500),
    metal_cross(b"XPDEUR", Cut::NewYork1000),
];

/// Build a deliverable fiat [`PairSpec`].
const fn fiat(key: &'static [u8; 6], premium_style: PremiumStyle, cut: Cut) -> PairSpec {
    PairSpec {
        key,
        premium_style,
        cut,
        kind: SpecKind::Deliverable,
        instrument: InstrumentClass::Fiat,
    }
}

/// Build a non-deliverable (NDF/NDO) [`PairSpec`] (always USD-cash-settled,
/// premium in USD = FOR → premium-adjusted).
const fn ndf(key: &'static [u8; 6], fixing: FixingSource, cut: Cut) -> PairSpec {
    PairSpec {
        key,
        premium_style: PremiumStyle::PercentForeign,
        cut,
        kind: SpecKind::NonDeliverable(fixing),
        instrument: InstrumentClass::Fiat,
    }
}

/// Build a precious-metal-vs-USD [`PairSpec`] (premium in USD = DOM → unadjusted).
const fn metal(key: &'static [u8; 6]) -> PairSpec {
    PairSpec {
        key,
        premium_style: PremiumStyle::DomesticPips,
        cut: Cut::NewYork1000,
        kind: SpecKind::Deliverable,
        instrument: InstrumentClass::PreciousMetal,
    }
}

/// Build a metal-cross [`PairSpec`] (metal base vs a fiat quote, premium in the
/// fiat quote = DOM → unadjusted).
const fn metal_cross(key: &'static [u8; 6], cut: Cut) -> PairSpec {
    PairSpec {
        key,
        premium_style: PremiumStyle::DomesticPips,
        cut,
        kind: SpecKind::Deliverable,
        instrument: InstrumentClass::PreciousMetal,
    }
}

/// The covered [`PairSpec`] for a canonical key, or `None` if not covered.
#[must_use]
fn spec_for(k: [u8; 6]) -> Option<&'static PairSpec> {
    COVERED_PAIRS.iter().find(|s| *s.key == k)
}

/// Whether a six-byte key is one of the covered canonical pairs.
///
/// The covered set is the [`COVERED_PAIRS`] table (the documented >75-pair
/// superset). This is the EXACT set; nothing outside it carries a bespoke profile
/// (such pairs fall through to the region default).
#[must_use]
fn is_covered_key(k: [u8; 6]) -> bool {
    spec_for(k).is_some()
}

/// The profile for a canonical covered key. The key is guaranteed covered by the
/// caller, so the catch-all is unreachable in practice and falls back to the
/// EURUSD-style G10 profile.
///
/// The money-market accrual legs are derived from [`accrual_basis`] keyed on the
/// canonical pair's FOR/DOM currencies — the single source of truth per currency
/// — so a covered pair and an uncovered cross of the same currency get the same
/// accrual day-count (e.g. AUD is ACT/365 in both covered AUDUSD and an
/// uncovered AUD cross), rather than each profile hard-coding its own. The metal
/// leg accrues on the bullion lease basis (ACT/360) when the base is a metal.
#[must_use]
fn profile_for_canonical(k: [u8; 6]) -> PairProfile {
    // FOR (base) is bytes 0..3, DOM (quote) is bytes 3..6 of the canonical key.
    let accrual_for = Ccy::new([k[0], k[1], k[2]]).map_or(DayCount::Act360, accrual_basis);
    let accrual_dom = Ccy::new([k[3], k[4], k[5]]).map_or(DayCount::Act360, accrual_basis);
    let Some(spec) = spec_for(k) else {
        // Unreachable: caller guarantees the key is covered. Fall back to a
        // EURUSD-style G10 profile rather than panic.
        return PairProfile {
            atm: AtmConvention::DeltaNeutralStraddle,
            premium_style: PremiumStyle::PercentForeign,
            cut: Cut::NewYork1000,
            day_count_accrual_for: accrual_for,
            day_count_accrual_dom: accrual_dom,
            settlement: Settlement::Deliverable,
            ndf: None,
            instrument: InstrumentClass::Fiat,
            metal_leg: None,
        };
    };
    let (settlement, ndf_terms) = match spec.kind {
        SpecKind::Deliverable => (Settlement::Deliverable, None),
        SpecKind::NonDeliverable(fixing) => (
            Settlement::NonDeliverable,
            Some(NdfTerms {
                fixing,
                settlement_ccy: Ccy::USD,
            }),
        ),
    };
    // A precious metal accrues no deposit; its lease rate rides the foreign (base)
    // leg on the loco-London ACT/360 bullion basis (the value is ENV; only the
    // convention is encoded). The metal is the canonical base.
    let metal_leg = match spec.instrument {
        InstrumentClass::PreciousMetal => Ccy::new([k[0], k[1], k[2]]).map(|metal| MetalLeg {
            metal,
            lease_day_count: DayCount::Act360,
            loco_london: true,
        }),
        InstrumentClass::Fiat => None,
    };
    PairProfile {
        atm: AtmConvention::DeltaNeutralStraddle,
        premium_style: spec.premium_style,
        cut: spec.cut,
        // For a precious metal the base (metal) leg carries the lease basis
        // (ACT/360); otherwise the per-currency accrual basis applies.
        day_count_accrual_for: match spec.instrument {
            InstrumentClass::PreciousMetal => DayCount::Act360,
            InstrumentClass::Fiat => accrual_for,
        },
        day_count_accrual_dom: accrual_dom,
        settlement,
        ndf: ndf_terms,
        instrument: spec.instrument,
        metal_leg,
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

/// The full **pair-universe** view of a covered pair: the resolved convention
/// fields the wire record carries, plus the universe-level metadata that does not
/// belong on the per-`(pair, tenor)` wire record — the spot lag, the physical
/// premium currency, the asset class, and (for NDFs) the cash-settlement terms.
///
/// This is the read model behind the broadened pair-universe registry. It is
/// resolved in the **queried orientation** (so the premium currency and the
/// premium-adjusted flag are correct for `USDKRW` vs `KRWUSD`), and is purely
/// additive: it consults the same [`PairProfile`] the wire [`resolve`] uses, so
/// the two can never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairMeta {
    /// The pair, in the queried orientation.
    pub pair: CcyPair,
    /// Spot lag in business days (T+2 default; T+1 for the USDCAD/TRY/RUB/PHP
    /// set), from `celnet-calendar` — the single source of truth for the lag.
    pub spot_lag_days: u32,
    /// ATM strike convention.
    pub atm: AtmConvention,
    /// Premium quotation style in the queried orientation.
    pub premium_style: PremiumStyle,
    /// The physical currency the premium is paid in (FOR/base for the
    /// foreign-premium styles, DOM/quote for the domestic-premium styles).
    pub premium_ccy: Ccy,
    /// Whether the (short-tenor) delta is premium-adjusted — equivalently,
    /// whether `premium_style` is a foreign-premium style.
    pub premium_adjusted: bool,
    /// Expiry/fixing cut.
    pub cut: Cut,
    /// Settlement style (deliverable vs non-deliverable).
    pub settlement: Settlement,
    /// Cash-settlement terms, present iff `settlement` is non-deliverable.
    pub ndf: Option<NdfTerms>,
    /// Asset class (fiat vs precious metal).
    pub instrument: InstrumentClass,
    /// The lease-rate-bearing metal leg, present iff `instrument` is
    /// [`InstrumentClass::PreciousMetal`]. Names the loco-London metal carry
    /// convention (the lease rate rides the foreign/base leg) without sourcing the
    /// live lease-rate value.
    pub metal_leg: Option<MetalLeg>,
}

impl PairMeta {
    /// Whether the pair is a non-deliverable (NDF/NDO) pair.
    #[must_use]
    pub const fn is_non_deliverable(self) -> bool {
        matches!(self.settlement, Settlement::NonDeliverable)
    }

    /// Whether the pair is a precious-metal pair (metal as the base leg).
    #[must_use]
    pub const fn is_precious_metal(self) -> bool {
        matches!(self.instrument, InstrumentClass::PreciousMetal)
    }

    /// Structural self-consistency of the universe entry: the NDF terms are
    /// present **iff** the pair is non-deliverable, the cash-settlement currency
    /// (when present) is one of the pair's two legs, the premium currency is one
    /// of the pair's legs and agrees with the premium-adjusted flag, and the spot
    /// lag is the canonical T+1 or T+2. Returns `false` on any contradiction.
    #[must_use]
    pub fn is_self_consistent(self) -> bool {
        // NDF terms present iff non-deliverable.
        if self.is_non_deliverable() != self.ndf.is_some() {
            return false;
        }
        // The settlement currency is one of the pair's legs (the convertible one).
        if let Some(terms) = self.ndf
            && terms.settlement_ccy != self.pair.base
            && terms.settlement_ccy != self.pair.quote
        {
            return false;
        }
        // The premium currency is one of the pair's legs.
        if self.premium_ccy != self.pair.base && self.premium_ccy != self.pair.quote {
            return false;
        }
        // The premium-adjusted flag agrees with the premium style and with the
        // premium currency being the FOR (base) leg.
        if self.premium_adjusted != self.premium_style.is_premium_adjusted() {
            return false;
        }
        if self.premium_adjusted != (self.premium_ccy == self.pair.base) {
            return false;
        }
        // Metal leg present iff precious metal; when present, the metal is one of
        // the pair's legs (the canonical base — but the orientation-inverted view
        // makes it the quote) and settles loco-London.
        if self.is_precious_metal() != self.metal_leg.is_some() {
            return false;
        }
        if let Some(leg) = self.metal_leg
            && ((leg.metal != self.pair.base && leg.metal != self.pair.quote) || !leg.loco_london)
        {
            return false;
        }
        // Spot lag is canonical.
        matches!(self.spot_lag_days, 1 | 2)
    }
}

/// The physical premium currency for a `(pair, premium_style)`: the FOR (base)
/// leg for the foreign-premium styles, the DOM (quote) leg otherwise.
#[must_use]
fn premium_ccy_of(pair: CcyPair, premium_style: PremiumStyle) -> Ccy {
    if premium_style.is_premium_adjusted() {
        pair.base
    } else {
        pair.quote
    }
}

/// Resolve the full [`PairMeta`] universe view for a covered pair, in the queried
/// orientation, or `None` if the pair has no bespoke profile.
///
/// Like [`resolve`], this consults the bespoke [`PairProfile`] (orientation-
/// transformed for the inverted ordering), so the premium currency and the
/// premium-adjusted flag are correct for both `USDKRW` and `KRWUSD`. The spot lag
/// comes from `celnet-calendar` so there is one source of truth.
#[must_use]
pub fn pair_meta(pair: CcyPair) -> Option<PairMeta> {
    let profile = pair_profile(pair)?;
    let premium_ccy = premium_ccy_of(pair, profile.premium_style);
    Some(PairMeta {
        pair,
        spot_lag_days: spot_lag_days(pair),
        atm: profile.atm,
        premium_style: profile.premium_style,
        premium_ccy,
        premium_adjusted: profile.premium_style.is_premium_adjusted(),
        cut: profile.cut,
        settlement: profile.settlement,
        ndf: profile.ndf,
        instrument: profile.instrument,
        metal_leg: profile.metal_leg,
    })
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
        // AUD must accrue ACT/365 whether the pair is covered as a major (AUDUSD)
        // or as a now-covered cross (AUDJPY, a bespoke G10 cross), since accrual
        // is a currency property, not a pair property.
        let covered = resolve(pair("AUDUSD"), Tenor::Months(3)).record;
        let cross = resolve(pair("AUDJPY"), Tenor::Months(3));
        // AUD is the FOR leg in both.
        assert_eq!(covered.day_count_accrual_for, DayCount::Act365Fixed);
        assert_eq!(cross.record.day_count_accrual_for, DayCount::Act365Fixed);

        // A genuinely uncovered cross (a currency outside the panel) still derives
        // its accrual per-currency via the region default — AUD stays ACT/365.
        let uncovered = resolve(pair("AUDPLN"), Tenor::Months(3));
        assert_eq!(uncovered.source, ResolutionSource::RegionDefault);
        assert_eq!(
            uncovered.record.day_count_accrual_for,
            DayCount::Act365Fixed
        );
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

    #[test]
    fn em_deliverable_crosses_are_usd_premium_adjusted_t2() {
        for p in ["USDMXN", "USDZAR", "USDNOK", "USDSEK"] {
            let m = pair_meta(pair(p)).unwrap_or_else(|| panic!("{p} covered"));
            assert_eq!(m.settlement, Settlement::Deliverable, "{p}");
            assert!(m.ndf.is_none(), "{p}");
            assert_eq!(m.premium_ccy, Ccy::USD, "{p} premium in USD (FOR)");
            assert!(m.premium_adjusted, "{p}");
            assert_eq!(m.spot_lag_days, 2, "{p} is T+2");
            assert_eq!(m.cut, Cut::NewYork1000, "{p}");
            assert!(m.is_self_consistent(), "{p}");
            assert_eq!(m.instrument, InstrumentClass::Fiat, "{p}");
        }
    }

    #[test]
    fn ndf_pairs_carry_fixing_and_usd_settlement() {
        let cases = [
            ("USDKRW", FixingSource::KrwKftc18, Cut::Tokyo1500),
            ("USDTWD", FixingSource::TwdTaipei, Cut::Tokyo1500),
            ("USDINR", FixingSource::InrRbiRef, Cut::Tokyo1500),
            ("USDBRL", FixingSource::BrlPtax, Cut::NewYork1000),
            ("USDCLP", FixingSource::ClpDolarObs, Cut::NewYork1000),
            ("USDCOP", FixingSource::CopTrm, Cut::NewYork1000),
        ];
        for (p, fixing, cut) in cases {
            let m = pair_meta(pair(p)).unwrap_or_else(|| panic!("{p} covered"));
            assert!(m.is_non_deliverable(), "{p}");
            let terms = m.ndf.unwrap_or_else(|| panic!("{p} has NDF terms"));
            assert_eq!(terms.fixing, fixing, "{p} fixing");
            assert_eq!(terms.settlement_ccy, Ccy::USD, "{p} settles USD");
            assert_eq!(m.cut, cut, "{p} cut");
            assert!(m.premium_adjusted, "{p} USD-FOR premium-adjusted");
            assert!(m.is_self_consistent(), "{p}");
            // The wire record agrees on non-deliverability.
            assert!(
                resolve(pair(p), Tenor::Months(3))
                    .record
                    .is_non_deliverable()
            );
        }
    }

    #[test]
    fn precious_metals_have_metal_base_usd_premium() {
        for (p, metal) in [("XAUUSD", Ccy::new(*b"XAU")), ("XAGUSD", Ccy::new(*b"XAG"))] {
            let cp = pair(p);
            let m = pair_meta(cp).unwrap_or_else(|| panic!("{p} covered"));
            assert!(m.is_precious_metal(), "{p}");
            // Metal is the base (asset/FOR) leg.
            assert_eq!(cp.base, metal.unwrap(), "{p} base is the metal");
            assert_eq!(cp.quote, Ccy::USD, "{p} quote is USD");
            // Premium in USD = quote → unadjusted.
            assert_eq!(m.premium_ccy, Ccy::USD, "{p} premium in USD (DOM)");
            assert!(!m.premium_adjusted, "{p}");
            assert_eq!(m.settlement, Settlement::Deliverable, "{p}");
            assert!(m.ndf.is_none(), "{p}");
            assert_eq!(m.spot_lag_days, 2, "{p} loco-London T+2");
            assert!(m.is_self_consistent(), "{p}");
        }
    }

    #[test]
    fn ndf_meta_is_orientation_invariant_in_fixing_and_settlement() {
        // Quoting KRWUSD must still cash-settle in USD at KFTC18 — the fixing and
        // settlement currency are physical pair properties.
        let direct = pair_meta(pair("USDKRW")).unwrap();
        let flipped = pair_meta(pair("KRWUSD")).unwrap();
        assert!(direct.is_non_deliverable() && flipped.is_non_deliverable());
        assert_eq!(direct.ndf.unwrap().fixing, flipped.ndf.unwrap().fixing);
        assert_eq!(
            direct.ndf.unwrap().settlement_ccy,
            flipped.ndf.unwrap().settlement_ccy
        );
        assert_eq!(direct.ndf.unwrap().settlement_ccy, Ccy::USD);
        // Both orientations are individually self-consistent.
        assert!(direct.is_self_consistent() && flipped.is_self_consistent());
        // The premium-adjusted flag flips with orientation (USD is FOR in USDKRW,
        // DOM in KRWUSD).
        assert!(direct.premium_adjusted);
        assert!(!flipped.premium_adjusted);
    }

    #[test]
    fn every_covered_pair_meta_is_self_consistent() {
        // Iterate the WHOLE covered table (the single source of truth) — every
        // canonical pair and its orientation-inverted view must be self-consistent
        // and agree with the wire record on settlement.
        for spec in COVERED_PAIRS {
            let cp = CcyPair::new(
                Ccy::new([spec.key[0], spec.key[1], spec.key[2]]).unwrap(),
                Ccy::new([spec.key[3], spec.key[4], spec.key[5]]).unwrap(),
            );
            let p = cp.to_string();
            for oriented in [cp, CcyPair::new(cp.quote, cp.base)] {
                let m = pair_meta(oriented).unwrap_or_else(|| panic!("{oriented} covered"));
                assert!(m.is_self_consistent(), "{oriented} inconsistent: {m:?}");
            }
            // The wire record's settlement matches the meta's (canonical view).
            let m = pair_meta(cp).unwrap();
            let rec = resolve(cp, Tenor::Months(3)).record;
            assert_eq!(rec.settlement, m.settlement, "{p} wire/meta settlement");
            assert_eq!(rec.is_non_deliverable(), m.is_non_deliverable(), "{p}");
        }
        // A genuinely uncovered pair (a currency outside the panel) has no meta.
        assert!(pair_meta(pair("EURPLN")).is_none());
        assert!(pair_meta(pair("USDTRY")).is_none());
        // The panel exceeds the documented 75-pair interbank universe.
        assert!(
            COVERED_PAIRS.len() > 75,
            "covered panel must exceed 75 pairs, got {}",
            COVERED_PAIRS.len()
        );
    }

    #[test]
    fn covered_table_has_no_duplicate_keys() {
        // The table is the single source of truth; a duplicate canonical key would
        // make `spec_for`/`is_covered_key` ambiguous and silently shadow a row.
        for (i, a) in COVERED_PAIRS.iter().enumerate() {
            for b in &COVERED_PAIRS[i + 1..] {
                assert_ne!(a.key, b.key, "duplicate canonical key {:?}", a.key);
            }
        }
    }

    #[test]
    fn precious_metals_carry_lease_bearing_metal_leg() {
        // Every metal (vs USD and the metal crosses) carries an explicit
        // lease-rate-bearing metal leg on the loco-London ACT/360 basis, with the
        // metal as the canonical base.
        for spec in COVERED_PAIRS
            .iter()
            .filter(|s| matches!(s.instrument, InstrumentClass::PreciousMetal))
        {
            let cp = CcyPair::new(
                Ccy::new([spec.key[0], spec.key[1], spec.key[2]]).unwrap(),
                Ccy::new([spec.key[3], spec.key[4], spec.key[5]]).unwrap(),
            );
            let m = pair_meta(cp).unwrap();
            assert!(m.is_precious_metal(), "{cp}");
            let leg = m
                .metal_leg
                .unwrap_or_else(|| panic!("{cp} has a metal leg"));
            assert_eq!(leg.metal, cp.base, "{cp} metal is the base");
            assert_eq!(leg.lease_day_count, DayCount::Act360, "{cp} lease ACT/360");
            assert!(leg.loco_london, "{cp} loco-London");
            // The metal base accrues on the lease (ACT/360) basis on the wire
            // record's foreign leg.
            let rec = resolve(cp, Tenor::Months(3)).record;
            assert_eq!(
                rec.day_count_accrual_for,
                DayCount::Act360,
                "{cp} lease leg"
            );
        }
    }
}
