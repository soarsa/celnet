//! The resolved per-`(pair, tenor)` FX-options convention record.
//!
//! A [`ConventionRecord`] is the *fully resolved* set of market conventions for
//! one currency pair at one tenor (`docs/ANALYTICS-SPEC.md` §1.1). It is the
//! single object the pricing layers (`celnet-vanilla`, `celnet-surface`) consult
//! to know *how* a quote is expressed — the delta convention to invert,
//! whether the ATM is ATMF or a delta-neutral straddle, which currency the
//! premium is paid in (and therefore whether delta is premium-adjusted), the
//! expiry cut, the vol-time and accrual day-counts, and the settlement style.
//!
//! Conventions are **first-class data**, never global defaults: USDJPY, EURUSD
//! and an EM NDF differ materially, and the *same* pair switches delta
//! convention with tenor (spot for short, forward/driftless for long). Mislabelling
//! any of these silently corrupts every downstream strike, smile and hedge —
//! convention error dwarfs model error — so the record carries each field
//! explicitly.

use celnet_types::{AtmConvention, Cut, DayCount, DeltaConvention, PremiumStyle, Settlement};

/// A fully resolved FX-options convention record for one `(pair, tenor)`.
///
/// Every field is an explicit market choice (`docs/CONVENTIONS.md`); there are
/// no implied defaults. The record is `Copy`/POD so it never allocates on the
/// hot path and can be cached densely per `(pair, tenor)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConventionRecord {
    /// Delta convention used to quote and invert strikes for this `(pair, tenor)`.
    ///
    /// Short tenors use spot delta; long tenors (≳1–2Y) switch to forward
    /// (driftless) delta. Premium-adjusted iff [`Self::premium_style`] is paid
    /// in the foreign (base) currency.
    pub delta: DeltaConvention,
    /// At-the-money strike convention (ATM-forward vs delta-neutral straddle).
    ///
    /// The delta-neutral straddle is the dominant interbank ATM for G10/EM.
    pub atm: AtmConvention,
    /// Premium quotation style — which currency the premium is paid in and its
    /// units. Drives whether delta is premium-adjusted.
    pub premium_style: PremiumStyle,
    /// Expiry/fixing cut: New York 10:00 for most pairs, Tokyo 15:00 for the
    /// JPY-region/Asian business.
    pub cut: Cut,
    /// Day-count used to convert dates to vol-time year fractions (ACT/365-fixed).
    pub day_count_vol: DayCount,
    /// Day-count used for the foreign-leg money-market rate accrual.
    pub day_count_accrual_for: DayCount,
    /// Day-count used for the domestic-leg money-market rate accrual.
    pub day_count_accrual_dom: DayCount,
    /// Settlement style: physically deliverable vs cash-settled non-deliverable.
    pub settlement: Settlement,
}

impl ConventionRecord {
    /// Construct a record from its eight explicit fields.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        delta: DeltaConvention,
        atm: AtmConvention,
        premium_style: PremiumStyle,
        cut: Cut,
        day_count_vol: DayCount,
        day_count_accrual_for: DayCount,
        day_count_accrual_dom: DayCount,
        settlement: Settlement,
    ) -> Self {
        Self {
            delta,
            atm,
            premium_style,
            cut,
            day_count_vol,
            day_count_accrual_for,
            day_count_accrual_dom,
            settlement,
        }
    }

    /// Whether the configured delta convention is premium-adjusted.
    ///
    /// This must agree with `self.premium_style.is_premium_adjusted()`; the
    /// [`crate::registry`] builders guarantee that invariant (see
    /// [`Self::is_consistent`]).
    #[must_use]
    pub const fn is_delta_premium_adjusted(self) -> bool {
        matches!(
            self.delta,
            DeltaConvention::SpotPremiumAdjusted | DeltaConvention::ForwardPremiumAdjusted
        )
    }

    /// Whether the configured delta convention is a forward (driftless) delta.
    #[must_use]
    pub const fn is_delta_forward(self) -> bool {
        matches!(
            self.delta,
            DeltaConvention::ForwardUnadjusted | DeltaConvention::ForwardPremiumAdjusted
        )
    }

    /// Internal consistency: the premium-adjusted flag implied by the premium
    /// style must match the one implied by the delta convention. A record that
    /// quotes premium in the foreign currency *must* use a premium-adjusted
    /// delta, and vice versa — otherwise the hedge delta is systematically wrong.
    #[must_use]
    pub const fn is_consistent(self) -> bool {
        self.premium_style.is_premium_adjusted() == self.is_delta_premium_adjusted()
    }

    /// Whether the pair is cash-settled at a published fixing (a non-deliverable
    /// option) rather than physically delivered.
    #[must_use]
    pub const fn is_non_deliverable(self) -> bool {
        matches!(self.settlement, Settlement::NonDeliverable)
    }
}

/// A `(pair, tenor)` resolution carrying the resolved convention record plus the
/// provenance of *how* it was resolved (exact match, term-structure rule, or a
/// region/default fallback). The provenance lets callers and tests reason about
/// whether a convention was hard-known or inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedConvention {
    /// The resolved convention record.
    pub record: ConventionRecord,
    /// How the registry arrived at this record.
    pub source: ResolutionSource,
}

/// Provenance of a [`ResolvedConvention`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSource {
    /// The pair has a bespoke profile and the tenor fell on a hard-coded rule
    /// for that profile (e.g. USDJPY ≤1Y spot vs >1Y forward delta).
    PairProfile,
    /// The pair has no bespoke profile; conventions came from the currency
    /// pair's region default (e.g. a generic USD-quoted G10 pair).
    RegionDefault,
}
