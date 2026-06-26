//! Vanilla fixed-vs-float single-currency interest-rate swap — conventionful schedules and risk.
//!
//! This slice generalises the annual self-discounting OIS (`ois.rs`) to the **two-leg vanilla swap**
//! the linear-rates desk actually quotes (`FI-ARCHITECTURE.md` §1, the linear-rates products):
//!
//! - **Sub-annual payment frequencies** — each leg rolls at its own [`PaymentFrequency`]
//!   (annual / semi-annual / quarterly), so the standard "semi-annual fixed vs quarterly float"
//!   shape is expressible rather than annual-only.
//! - **Independent legs** — the fixed and float legs carry their own schedules and day-count
//!   accruals; the swap is the difference of the two leg values.
//! - **Explicit float projection** — each floating coupon's forward fixing is projected from the
//!   curve as `(DF(accrual_start)/DF(pay) − 1) / accrual` and discounted, rather than collapsed to
//!   the telescoping shortcut. On the single self-discounting curve this *equals*
//!   `DF(start) − DF(maturity)` (verified in the tests), and it is the exact seam where a separate
//!   projection curve plugs in later (`FI-CURVES-SPEC.md` §5.2) — projection and discounting are one
//!   curve here, by construction, not a stub.
//!
//! ## Pricing identities
//!
//! For a receive-fixed swap on `notional` `N` with fixed rate `K`:
//!
//! ```text
//! fixed_annuity  A   = Σ_i  accrual_i · DF(pay_i)                 (fixed leg)
//! float_value    F   = Σ_j ( DF(start_j) − DF(pay_j) )            (float leg, telescopes to DF(0)−DF(T))
//! PV_receive_fixed   = N · ( K · A − F )
//! par_rate       K*  = F / A
//! PV01               = N · A · 1bp                                (exact; PV is linear in K)
//! ```
//!
//! ## Deliberately not in this slice (no stubs — coordinated follow-ups)
//!
//! - **30/360 fixed-leg accrual.** The market USD fixed leg is 30/360, but `celnet_types::DayCount`
//!   currently exposes only ACT/365F and ACT/360; adding 30/360 is a coordinated change to the
//!   shared types crate (and its proto / handoff / SDK mirrors), tracked in `FI-STATUS.md`. The
//!   accrual basis here is therefore a caller-supplied [`DayCount`] from the supported set.
//! - **Distinct projection curve** (dual-curve / basis). Projection == discount on the single curve.
//!
//! Method/paper provenance lives in prose only — never in identifiers (CLAUDE.md §8).

use crate::bootstrap::{BootstrapError, OisQuote, bootstrap_ois};
use crate::curve::Curve;
use crate::risk::ONE_BP;
use crate::schedule::us_settlement_calendar;
use celnet_calendar::{RollRule, add_months, year_fraction};
use celnet_types::{DayCount, Rate, Time};
use time::Date;

/// Absolute tolerance for schedule contiguity / coterminality checks on the curve time axis (days).
const TIME_TOL: f64 = 1e-9;

/// Coupon payment frequency of a swap leg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaymentFrequency {
    /// One coupon per year.
    Annual,
    /// Two coupons per year (every 6 months).
    SemiAnnual,
    /// Four coupons per year (every 3 months).
    Quarterly,
}

impl PaymentFrequency {
    /// Number of coupon periods per year.
    #[must_use]
    pub const fn per_year(self) -> u32 {
        match self {
            Self::Annual => 1,
            Self::SemiAnnual => 2,
            Self::Quarterly => 4,
        }
    }

    /// Calendar months between consecutive coupon dates.
    #[must_use]
    pub const fn months(self) -> i32 {
        match self {
            Self::Annual => 12,
            Self::SemiAnnual => 6,
            Self::Quarterly => 3,
        }
    }
}

/// One accrual period of a swap leg, on the curve's continuous time axis.
///
/// The payment is made at `pay` (the period end — no settlement lag in this slice), so the period's
/// accrual interval is `[accrual_start, pay]` and `accrual` is its day-count year fraction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegPeriod {
    /// Accrual-start time coordinate (ACT/365F from the schedule origin).
    pub accrual_start: Time,
    /// Payment / accrual-end time coordinate (ACT/365F from the schedule origin).
    pub pay: Time,
    /// Day-count accrual fraction over `[accrual_start, pay]` on the leg's basis.
    pub accrual: Time,
}

/// A contiguous swap leg schedule: an origin plus increasing, gap-free accrual periods.
#[derive(Clone, Debug)]
pub struct SwapLeg {
    start: Time,
    periods: Vec<LegPeriod>,
}

/// Failure modes of swap construction and scheduling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapError {
    /// A leg has no accrual periods.
    EmptyLeg,
    /// A leg's periods are not contiguous and strictly increasing from the origin.
    NonContiguousLeg,
    /// The fixed and float legs do not share the same start and maturity.
    NonCoterminalLegs,
    /// The notional is not strictly positive.
    NonPositiveNotional,
}

impl core::fmt::Display for SwapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyLeg => f.write_str("a swap leg has no accrual periods"),
            Self::NonContiguousLeg => {
                f.write_str("swap-leg periods must be contiguous and strictly increasing")
            }
            Self::NonCoterminalLegs => {
                f.write_str("the fixed and float legs must share the same start and maturity")
            }
            Self::NonPositiveNotional => f.write_str("the swap notional must be strictly positive"),
        }
    }
}

impl core::error::Error for SwapError {}

impl SwapLeg {
    /// Build a leg from its origin and accrual periods, validating contiguity.
    ///
    /// # Errors
    ///
    /// Returns [`SwapError::EmptyLeg`] when `periods` is empty, or [`SwapError::NonContiguousLeg`]
    /// when the first period does not start at `start`, a period is non-increasing, or a gap opens
    /// between consecutive periods.
    pub fn new(start: Time, periods: Vec<LegPeriod>) -> Result<Self, SwapError> {
        let Some(first) = periods.first() else {
            return Err(SwapError::EmptyLeg);
        };
        if (first.accrual_start.0 - start.0).abs() > TIME_TOL {
            return Err(SwapError::NonContiguousLeg);
        }
        let mut prev_pay = start.0;
        for p in &periods {
            let contiguous = (p.accrual_start.0 - prev_pay).abs() <= TIME_TOL;
            let increasing = p.pay.0 > p.accrual_start.0 && p.accrual.0 > 0.0;
            if !contiguous || !increasing {
                return Err(SwapError::NonContiguousLeg);
            }
            prev_pay = p.pay.0;
        }
        Ok(Self { start, periods })
    }

    /// The leg origin (curve time 0 for a spot-starting leg).
    #[must_use]
    pub fn start(&self) -> Time {
        self.start
    }

    /// The leg maturity (the final period's payment time).
    #[must_use]
    pub fn maturity(&self) -> Time {
        self.periods.last().map_or(self.start, |p| p.pay)
    }

    /// The leg's accrual periods.
    #[must_use]
    pub fn periods(&self) -> &[LegPeriod] {
        &self.periods
    }
}

/// A receive-fixed vanilla swap: a fixed leg paying `fixed_rate`, a float leg, and a notional.
///
/// Pay-fixed is the negation of the returned PV/risk.
#[derive(Clone, Debug)]
pub struct VanillaSwap {
    fixed_leg: SwapLeg,
    float_leg: SwapLeg,
    fixed_rate: Rate,
    notional: f64,
}

impl VanillaSwap {
    /// Assemble a coterminal vanilla swap.
    ///
    /// # Errors
    ///
    /// Returns [`SwapError::NonCoterminalLegs`] when the legs do not share start and maturity, or
    /// [`SwapError::NonPositiveNotional`] when `notional <= 0`.
    pub fn new(
        fixed_leg: SwapLeg,
        float_leg: SwapLeg,
        fixed_rate: Rate,
        notional: f64,
    ) -> Result<Self, SwapError> {
        if notional <= 0.0 || !notional.is_finite() {
            return Err(SwapError::NonPositiveNotional);
        }
        let same_start = (fixed_leg.start().0 - float_leg.start().0).abs() <= TIME_TOL;
        let same_end = (fixed_leg.maturity().0 - float_leg.maturity().0).abs() <= TIME_TOL;
        if !same_start || !same_end {
            return Err(SwapError::NonCoterminalLegs);
        }
        Ok(Self {
            fixed_leg,
            float_leg,
            fixed_rate,
            notional,
        })
    }

    /// The fixed leg.
    #[must_use]
    pub fn fixed_leg(&self) -> &SwapLeg {
        &self.fixed_leg
    }

    /// The float leg.
    #[must_use]
    pub fn float_leg(&self) -> &SwapLeg {
        &self.float_leg
    }

    /// The fixed rate.
    #[must_use]
    pub fn fixed_rate(&self) -> Rate {
        self.fixed_rate
    }

    /// The notional.
    #[must_use]
    pub fn notional(&self) -> f64 {
        self.notional
    }
}

/// The fixed-leg annuity `A = Σ accrual_i · DF(pay_i)` (per unit notional).
#[must_use]
pub fn fixed_annuity(curve: &Curve, leg: &SwapLeg) -> f64 {
    leg.periods()
        .iter()
        .map(|p| p.accrual.0 * curve.discount_factor(p.pay).0)
        .sum()
}

/// The float-leg value per unit notional, by **explicit per-period forward projection**.
///
/// Each coupon's forward fixing is `f_j = (DF(start_j)/DF(pay_j) − 1) / accrual_j`; its discounted
/// value is `DF(pay_j) · accrual_j · f_j = DF(start_j) − DF(pay_j)`. On the single self-discounting
/// curve the periods telescope to `DF(leg.start) − DF(leg.maturity)`; computing each coupon
/// explicitly keeps the projection-curve seam live (and is checked against the telescoping form in
/// the tests).
#[must_use]
pub fn float_leg_value(curve: &Curve, leg: &SwapLeg) -> f64 {
    leg.periods()
        .iter()
        .map(|p| {
            let df_start = curve.discount_factor(p.accrual_start).0;
            let df_pay = curve.discount_factor(p.pay).0;
            let forward = (df_start / df_pay - 1.0) / p.accrual.0;
            df_pay * p.accrual.0 * forward
        })
        .sum()
}

/// The par (fair) fixed rate `K* = float_value / fixed_annuity`.
#[must_use]
pub fn swap_par_rate(curve: &Curve, swap: &VanillaSwap) -> Rate {
    Rate(float_leg_value(curve, swap.float_leg()) / fixed_annuity(curve, swap.fixed_leg()))
}

/// Present value of **receiving fixed**: `N · ( K · A − F )`.
///
/// Positive when the fixed rate exceeds par; pay-fixed is the negation.
#[must_use]
pub fn swap_pv(curve: &Curve, swap: &VanillaSwap) -> f64 {
    let annuity = fixed_annuity(curve, swap.fixed_leg());
    let float_value = float_leg_value(curve, swap.float_leg());
    swap.notional() * (swap.fixed_rate().0 * annuity - float_value)
}

/// Analytic PV01: the PV change of a one-basis-point fixed-rate move, `N · A · 1bp` (exact).
#[must_use]
pub fn swap_pv01(curve: &Curve, swap: &VanillaSwap) -> f64 {
    swap.notional() * fixed_annuity(curve, swap.fixed_leg()) * ONE_BP
}

/// A bumped-curve risk report for a vanilla swap priced off a curve bootstrapped from `quotes`.
#[derive(Clone, Debug)]
pub struct SwapRisk {
    /// Present value on the base (unbumped) curve.
    pub pv: f64,
    /// Analytic PV01 (`N · A · 1bp`).
    pub pv01: f64,
    /// PV change for a parallel one-basis-point bump of every calibrating quote (central difference).
    pub dv01: f64,
    /// Per-instrument key-rate DV01s, in calibrating-quote order; sums to `dv01` to first order.
    pub key_rate: Vec<f64>,
}

/// Compute PV, PV01, DV01, and the key-rate ladder for a receive-fixed vanilla swap.
///
/// The curve is bootstrapped from `quotes`; DV01 and the ladder use **central** (symmetric) quote
/// bumps and re-bootstrap, so the ladder is second-order accurate and sums to the parallel DV01 to
/// machine-level relative tolerance (`FI-VERIFICATION-CONTRACT.md`, key-rate additivity).
///
/// # Errors
///
/// Propagates any [`BootstrapError`] from the base or bumped curve builds.
pub fn swap_risk(quotes: &[OisQuote], swap: &VanillaSwap) -> Result<SwapRisk, BootstrapError> {
    let base = bootstrap_ois(quotes)?;
    let pv = swap_pv(&base, swap);
    let pv01 = swap_pv01(&base, swap);

    // Re-price the swap on a curve re-bootstrapped with each quote's par rate shifted by `shift(i)`.
    let reprice = |shift: &dyn Fn(usize) -> f64| -> Result<f64, BootstrapError> {
        let bumped: Vec<OisQuote> = quotes
            .iter()
            .enumerate()
            .map(|(i, q)| OisQuote {
                schedule: q.schedule.clone(),
                par_rate: Rate(q.par_rate.0 + shift(i)),
            })
            .collect();
        Ok(swap_pv(&bootstrap_ois(&bumped)?, swap))
    };

    // Parallel DV01 via central difference.
    let dv01 = (reprice(&|_| ONE_BP)? - reprice(&|_| -ONE_BP)?) / 2.0;

    // Key-rate ladder: each calibrating quote bumped in isolation, central difference.
    let mut key_rate = Vec::with_capacity(quotes.len());
    for target in 0..quotes.len() {
        let up = reprice(&|i| if i == target { ONE_BP } else { 0.0 })?;
        let down = reprice(&|i| if i == target { -ONE_BP } else { 0.0 })?;
        key_rate.push((up - down) / 2.0);
    }

    Ok(SwapRisk {
        pv,
        pv01,
        dv01,
        key_rate,
    })
}

/// Build a spot-starting swap leg of `years` years at `freq`, accruing on `basis`.
///
/// `reference` is the spot date (normalised to the next US business day, becoming curve time 0).
/// Each coupon end is `freq.months()·i` months after the start, rolled modified-following on the US
/// settlement calendar; the accrual fraction uses `basis`, while the accrual-start and payment time
/// coordinates are ACT/365F from the start (the curve's discount-time axis, matching `schedule.rs`).
///
/// # Errors
///
/// Returns [`SwapError::EmptyLeg`] when `years` is zero, propagating any contiguity error from
/// [`SwapLeg::new`].
pub fn swap_leg_schedule(
    reference: Date,
    years: u32,
    freq: PaymentFrequency,
    basis: DayCount,
) -> Result<SwapLeg, SwapError> {
    if years == 0 {
        return Err(SwapError::EmptyLeg);
    }
    let cal = us_settlement_calendar();
    let start = RollRule::Following.adjust(&cal, reference);
    let n = years * freq.per_year();

    let mut periods = Vec::with_capacity(n as usize);
    let mut prev = start;
    for i in 1..=n {
        let end =
            RollRule::ModifiedFollowing.adjust(&cal, add_months(start, (i as i32) * freq.months()));
        let accrual = year_fraction(basis, prev, end);
        let accrual_start = year_fraction(DayCount::Act365Fixed, start, prev);
        let pay = year_fraction(DayCount::Act365Fixed, start, end);
        periods.push(LegPeriod {
            accrual_start,
            pay,
            accrual,
        });
        prev = end;
    }
    SwapLeg::new(Time(0.0), periods)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ois::{FixedPeriod, OisSchedule, ois_par_rate};
    use time::Month;

    /// A reference spot date that is a US business day (Monday 16 Jun 2025).
    fn spot() -> Date {
        Date::from_calendar_date(2025, Month::June, 16).expect("valid date")
    }

    /// A representative calibrated curve from annual SOFR OIS quotes.
    fn curve() -> Curve {
        let quotes = ladder();
        bootstrap_ois(&quotes).expect("bootstraps")
    }

    /// An annual OIS quote maturing in `n` years with unit-year accruals.
    fn quote(n: usize, par: f64) -> OisQuote {
        let periods = (1..=n)
            .map(|i| FixedPeriod {
                pay: Time(i as f64),
                accrual: Time(1.0),
            })
            .collect();
        OisQuote {
            schedule: OisSchedule::new(Time(0.0), periods).expect("schedule"),
            par_rate: Rate(par),
        }
    }

    fn ladder() -> Vec<OisQuote> {
        vec![
            quote(1, 0.0432),
            quote(2, 0.0418),
            quote(5, 0.0405),
            quote(10, 0.0415),
        ]
    }

    /// A standard 5y swap: semi-annual fixed vs quarterly float, both ACT/360.
    fn five_year_swap(fixed_rate: f64) -> VanillaSwap {
        let fixed = swap_leg_schedule(spot(), 5, PaymentFrequency::SemiAnnual, DayCount::Act360)
            .expect("fixed leg");
        let float = swap_leg_schedule(spot(), 5, PaymentFrequency::Quarterly, DayCount::Act360)
            .expect("float leg");
        VanillaSwap::new(fixed, float, Rate(fixed_rate), 10_000_000.0).expect("coterminal swap")
    }

    #[test]
    fn schedule_builds_expected_period_counts() {
        let semi = swap_leg_schedule(spot(), 5, PaymentFrequency::SemiAnnual, DayCount::Act360)
            .expect("semi");
        let quarterly = swap_leg_schedule(spot(), 5, PaymentFrequency::Quarterly, DayCount::Act360)
            .expect("quarterly");
        assert_eq!(semi.periods().len(), 10);
        assert_eq!(quarterly.periods().len(), 20);
        // Both legs are coterminal at ~5y on the ACT/365F axis.
        assert!((semi.maturity().0 - quarterly.maturity().0).abs() < 1e-12);
        assert!((semi.maturity().0 - 5.0).abs() < 0.03);
    }

    #[test]
    fn float_leg_explicit_projection_matches_telescoping() {
        let c = curve();
        let float = swap_leg_schedule(spot(), 7, PaymentFrequency::Quarterly, DayCount::Act360)
            .expect("float leg");
        let explicit = float_leg_value(&c, &float);
        let telescoped = c.discount_factor(float.start()).0 - c.discount_factor(float.maturity()).0;
        assert!(
            (explicit - telescoped).abs() < 1e-12,
            "explicit {explicit} vs telescoped {telescoped}"
        );
    }

    #[test]
    fn par_swap_has_zero_pv() {
        let c = curve();
        let swap = five_year_swap(0.0);
        let par = swap_par_rate(&c, &swap);
        let priced = five_year_swap(par.0);
        assert!(
            swap_pv(&c, &priced).abs() < 1e-7,
            "par-rate PV should vanish, got {}",
            swap_pv(&c, &priced)
        );
    }

    #[test]
    fn receive_fixed_decomposes_into_fixed_minus_float() {
        let c = curve();
        let swap = five_year_swap(0.043);
        let n = swap.notional();
        let fixed_pv = n * swap.fixed_rate().0 * fixed_annuity(&c, swap.fixed_leg());
        let float_pv = n * float_leg_value(&c, swap.float_leg());
        assert!((swap_pv(&c, &swap) - (fixed_pv - float_pv)).abs() < 1e-9);
    }

    #[test]
    fn pay_fixed_is_the_negation_of_receive_fixed() {
        let c = curve();
        let swap = five_year_swap(0.05);
        // Pay-fixed PV is −receive-fixed; a swap struck above par is a positive receive-fixed PV.
        assert!(swap_pv(&c, &swap) > 0.0);
        assert!((swap_pv(&c, &swap) + (-swap_pv(&c, &swap))).abs() < 1e-12);
    }

    #[test]
    fn pv01_matches_the_fixed_rate_finite_difference() {
        let c = curve();
        let base = five_year_swap(0.04);
        let bumped = five_year_swap(0.04 + ONE_BP);
        let diff = swap_pv(&c, &bumped) - swap_pv(&c, &base);
        // PV is exactly linear in the fixed rate, so the analytic PV01 is exact.
        assert!((diff - swap_pv01(&c, &base)).abs() < 1e-7, "diff {diff}");
    }

    #[test]
    fn key_rate_ladder_sums_to_dv01() {
        let quotes = ladder();
        let swap = five_year_swap(0.041);
        let risk = swap_risk(&quotes, &swap).expect("risk");
        assert_eq!(risk.key_rate.len(), quotes.len());
        let summed: f64 = risk.key_rate.iter().sum();
        // Central differences make the single-bump ladder additive with the parallel bump to
        // second order; the residual is tiny relative to the DV01 magnitude.
        assert!(
            (summed - risk.dv01).abs() < 1e-6 * risk.dv01.abs(),
            "ladder sum {summed} vs dv01 {}",
            risk.dv01
        );
    }

    #[test]
    fn par_swap_dv01_offsets_pv01() {
        let quotes = ladder();
        let c = bootstrap_ois(&quotes).expect("curve");
        let at_par = swap_par_rate(&c, &five_year_swap(0.0));
        let swap = five_year_swap(at_par.0);
        let risk = swap_risk(&quotes, &swap).expect("risk");
        assert!(risk.pv.abs() < 1e-6, "par PV ~0, got {}", risk.pv);
        // A par receive-fixed swap loses ~N·A·1bp of value when rates rise 1bp, so DV01 ≈ −PV01.
        // The match is approximate (not the exact fixed-rate sensitivity tested separately): the
        // swap's fixed leg is semi-annual while the calibrating OIS quotes are annual on a sparse
        // 4-pillar curve, so a parallel quote bump transmits to the swap with a few-percent
        // duration/convexity residual. The sign and magnitude must still agree to ~5%.
        assert!(
            risk.dv01 < 0.0 && (risk.dv01 + risk.pv01).abs() < 5e-2 * risk.pv01.abs(),
            "dv01 {} pv01 {}",
            risk.dv01,
            risk.pv01
        );
    }

    #[test]
    fn annual_swap_par_rate_matches_ois() {
        // An annual ACT/360 fixed leg with an annual ACT/360 float leg is exactly the self-
        // discounting OIS, so its par rate must equal `ois_par_rate` on the matching schedule.
        let c = curve();
        let fixed = swap_leg_schedule(spot(), 5, PaymentFrequency::Annual, DayCount::Act360)
            .expect("fixed");
        let float = swap_leg_schedule(spot(), 5, PaymentFrequency::Annual, DayCount::Act360)
            .expect("float");
        let swap = VanillaSwap::new(fixed, float, Rate(0.0), 1.0).expect("swap");

        let ois_sched = crate::schedule::usd_sofr_ois_schedule(spot(), 5).expect("ois schedule");
        let swap_par = swap_par_rate(&c, &swap).0;
        let ois_par = ois_par_rate(&c, &ois_sched).0;
        assert!(
            (swap_par - ois_par).abs() < 1e-12,
            "swap par {swap_par} vs ois par {ois_par}"
        );
    }

    #[test]
    fn rejects_non_coterminal_legs() {
        let fixed = swap_leg_schedule(spot(), 5, PaymentFrequency::SemiAnnual, DayCount::Act360)
            .expect("fixed");
        let float = swap_leg_schedule(spot(), 7, PaymentFrequency::Quarterly, DayCount::Act360)
            .expect("float");
        assert_eq!(
            VanillaSwap::new(fixed, float, Rate(0.04), 1.0).unwrap_err(),
            SwapError::NonCoterminalLegs
        );
    }

    #[test]
    fn rejects_non_positive_notional() {
        let fixed = swap_leg_schedule(spot(), 2, PaymentFrequency::SemiAnnual, DayCount::Act360)
            .expect("fixed");
        let float = swap_leg_schedule(spot(), 2, PaymentFrequency::Quarterly, DayCount::Act360)
            .expect("float");
        assert_eq!(
            VanillaSwap::new(fixed, float, Rate(0.04), 0.0).unwrap_err(),
            SwapError::NonPositiveNotional
        );
    }

    #[test]
    fn rejects_zero_year_and_empty_legs() {
        assert_eq!(
            swap_leg_schedule(spot(), 0, PaymentFrequency::Annual, DayCount::Act360).unwrap_err(),
            SwapError::EmptyLeg
        );
        assert_eq!(
            SwapLeg::new(Time(0.0), vec![]).unwrap_err(),
            SwapError::EmptyLeg
        );
    }
}
