//! The absolute-dated cashflow schedule a corporate-action event transforms.
//!
//! This is the vendor-neutral, serde-clean, **absolute-dated** view of a bond's future cashflows —
//! the object the pure effect functions rewrite. It deliberately mirrors, but does not depend on,
//! `celnet-bond`'s settlement-relative private `CashflowSchedule`: the CA layer needs calendar dates
//! (a call date, a sinking date, a pay date) to locate and reshape flows, and needs the result to be
//! serialized into the golden-source `InstrumentMaster`. Amounts are expressed **per 100 units of
//! original face**, so the schedule's present value is directly the value of a 100-face holding and a
//! pro-rata event scales every remaining flow by one factor (§6.1, §8).

use serde::{Deserialize, Serialize};

use crate::date::CivilDate;

/// One dated cashflow of a bond, in per-100-original-face cash.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScheduleFlow {
    /// The payment date.
    pub date: CivilDate,
    /// Coupon / interest cash on this date, per 100 face.
    pub coupon: f64,
    /// Principal / redemption cash on this date, per 100 face (`0.0` except a redemption flow).
    pub principal: f64,
}

impl ScheduleFlow {
    /// The total cash on this date (coupon + principal).
    #[must_use]
    pub fn total(&self) -> f64 {
        self.coupon + self.principal
    }
}

/// Failure modes of schedule construction and event application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaError {
    /// The flows are not in strictly ascending date order.
    NonAscendingSchedule,
    /// A flow carries a non-finite or negative amount.
    InvalidFlowAmount,
    /// A flow carries a date that is not a real calendar date.
    InvalidDate,
    /// The pool factor is not finite or not in `(0, 1]`.
    InvalidPoolFactor,
    /// A partial event's `redeemed_fraction` is not finite or not in `[0, 1]`.
    InvalidFraction,
    /// A cash price / coupon term is not finite.
    InvalidTerm,
    /// An exchange / conversion event carries no target instrument or a non-positive ratio.
    InvalidExchange,
    /// A redemption event found no cashflow on its effective date to realise.
    NoFlowOnEffectiveDate,
}

impl core::fmt::Display for CaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            Self::NonAscendingSchedule => "schedule flows must be in strictly ascending date order",
            Self::InvalidFlowAmount => "a schedule flow amount is non-finite or negative",
            Self::InvalidDate => "a schedule flow carries an impossible calendar date",
            Self::InvalidPoolFactor => "the pool factor must be finite and in (0, 1]",
            Self::InvalidFraction => "the redeemed fraction must be finite and in [0, 1]",
            Self::InvalidTerm => "a corporate-action cash term must be finite",
            Self::InvalidExchange => {
                "an exchange event needs a target instrument and positive ratio"
            }
            Self::NoFlowOnEffectiveDate => {
                "a redemption event found no cashflow on its effective date"
            }
        };
        f.write_str(msg)
    }
}

impl core::error::Error for CaError {}

/// A bond's future cashflows plus its current outstanding pool factor.
///
/// Immutable once built (via [`BondSchedule::new`] or [`BondSchedule::fixed_coupon`]); every effect
/// function returns a *new* schedule rather than mutating (guardrail: immutable transforms). The
/// `pool_factor` is the current outstanding nominal as a fraction of original (`1.0` = full); a
/// pro-rata event multiplies it and scales every remaining flow by the same factor, so the two stay
/// consistent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BondSchedule {
    flows: Vec<ScheduleFlow>,
    pool_factor: f64,
}

impl BondSchedule {
    /// Assemble a schedule from ascending, validated flows at full outstanding (`pool_factor = 1`).
    ///
    /// # Errors
    /// [`CaError::NonAscendingSchedule`] if dates are not strictly ascending; [`CaError::InvalidDate`]
    /// for an impossible date; [`CaError::InvalidFlowAmount`] for a non-finite/negative amount.
    pub fn new(flows: Vec<ScheduleFlow>) -> Result<Self, CaError> {
        Self::with_pool_factor(flows, 1.0)
    }

    /// Assemble a schedule at an explicit `pool_factor` (used when reconstructing a partially
    /// redeemed instrument from the store).
    ///
    /// # Errors
    /// As [`BondSchedule::new`], plus [`CaError::InvalidPoolFactor`] if `pool_factor ∉ (0, 1]`.
    pub fn with_pool_factor(flows: Vec<ScheduleFlow>, pool_factor: f64) -> Result<Self, CaError> {
        if !(pool_factor.is_finite() && pool_factor > 0.0 && pool_factor <= 1.0) {
            return Err(CaError::InvalidPoolFactor);
        }
        for w in flows.windows(2) {
            if w[1].date <= w[0].date {
                return Err(CaError::NonAscendingSchedule);
            }
        }
        for flow in &flows {
            if !flow.date.is_valid() {
                return Err(CaError::InvalidDate);
            }
            if !(flow.coupon.is_finite() && flow.principal.is_finite())
                || flow.coupon < 0.0
                || flow.principal < 0.0
            {
                return Err(CaError::InvalidFlowAmount);
            }
        }
        Ok(Self { flows, pool_factor })
    }

    /// Generate a regular fixed-coupon bullet schedule by rolling coupon dates back from `maturity`
    /// at `coupons_per_year`, end-of-month-aware (the same primitive `celnet-bond` uses). Only dates
    /// strictly after `first_accrual` are emitted. The final flow carries `redemption` principal.
    ///
    /// This is what a deterministic govvie source calls to *derive* a schedule from issuance terms
    /// (§7.2) rather than expecting a fed schedule.
    ///
    /// # Errors
    /// [`CaError::InvalidTerm`] for a non-finite coupon/redemption, a non-positive frequency, or a
    /// maturity on/before `first_accrual`.
    pub fn fixed_coupon(
        first_accrual: CivilDate,
        maturity: CivilDate,
        coupon_rate: f64,
        coupons_per_year: u32,
        redemption: f64,
    ) -> Result<Self, CaError> {
        if !(coupon_rate.is_finite() && redemption.is_finite() && redemption > 0.0)
            || coupons_per_year == 0
        {
            return Err(CaError::InvalidTerm);
        }
        let (Some(start), Some(mat)) = (first_accrual.to_date(), maturity.to_date()) else {
            return Err(CaError::InvalidDate);
        };
        if mat <= start {
            return Err(CaError::InvalidTerm);
        }
        let step_months = i32::try_from(12 / coupons_per_year).map_err(|_| CaError::InvalidTerm)?;
        let coupon_cash = coupon_rate / f64::from(coupons_per_year) * redemption;

        // Roll back from maturity to the first date strictly after `start`, then reverse to ascending.
        let mut dates_desc = Vec::new();
        let mut j = 0i32;
        loop {
            let d = celnet_calendar::add_months(mat, -j * step_months);
            if d <= start {
                break;
            }
            dates_desc.push(d);
            j += 1;
        }
        dates_desc.reverse();
        let last = dates_desc.len().saturating_sub(1);
        let flows = dates_desc
            .into_iter()
            .enumerate()
            .map(|(k, d)| ScheduleFlow {
                date: CivilDate::from_date(d),
                coupon: coupon_cash,
                principal: if k == last { redemption } else { 0.0 },
            })
            .collect();
        Self::new(flows)
    }

    /// The future cashflows in ascending date order.
    #[must_use]
    pub fn flows(&self) -> &[ScheduleFlow] {
        &self.flows
    }

    /// The current outstanding nominal as a fraction of original (`1.0` = full).
    #[must_use]
    pub fn pool_factor(&self) -> f64 {
        self.pool_factor
    }

    /// The number of remaining cashflows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.flows.len()
    }

    /// Whether the schedule has no remaining cashflows (a fully redeemed instrument).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }

    /// The final (maturity) flow, if any.
    #[must_use]
    pub fn last_flow(&self) -> Option<ScheduleFlow> {
        self.flows.last().copied()
    }

    /// Present value of the remaining schedule at a flat continuously-compounded `rate`, discounting
    /// each flow on the ACT/365F year-fraction from `as_of`. Flows on/before `as_of` are treated as
    /// already settled (excluded). Deterministic — used by the oracle's PV-consistency invariant and
    /// available to callers that want a quick schedule check.
    #[must_use]
    pub fn present_value(&self, as_of: CivilDate, rate: f64) -> f64 {
        let Some(base) = as_of.to_date() else {
            return f64::NAN;
        };
        self.flows
            .iter()
            .filter_map(|flow| {
                let d = flow.date.to_date()?;
                if d <= base {
                    return None;
                }
                let t = f64::from((d - base).whole_days() as i32) / 365.0;
                Some(flow.total() * (-rate * t).exp())
            })
            .sum()
    }

    /// Return a new schedule with every remaining flow's coupon and principal scaled by `factor` and
    /// the pool factor multiplied by it — the pro-rata transform a partial event makes. Internal; the
    /// public entry point is [`crate::effect::apply_event`].
    pub(crate) fn scaled(&self, factor: f64) -> Self {
        Self {
            flows: self
                .flows
                .iter()
                .map(|f| ScheduleFlow {
                    date: f.date,
                    coupon: f.coupon * factor,
                    principal: f.principal * factor,
                })
                .collect(),
            pool_factor: self.pool_factor * factor,
        }
    }

    /// Return a new schedule dropping every coupon-only flow on/before `through` (an INTR settle).
    pub(crate) fn without_coupons_through(&self, through: CivilDate) -> Self {
        Self {
            flows: self
                .flows
                .iter()
                .copied()
                .filter(|f| !(f.date <= through && f.principal == 0.0))
                .collect(),
            pool_factor: self.pool_factor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_coupon_rolls_semiannual_schedule() {
        // 2y semi-annual 4% bond, dated 2033-06-15, matures 2035-06-15 → 4 flows.
        let s = BondSchedule::fixed_coupon(
            CivilDate::new(2033, 6, 15),
            CivilDate::new(2035, 6, 15),
            0.04,
            2,
            100.0,
        )
        .expect("schedule");
        assert_eq!(s.len(), 4);
        // Each coupon = 4% / 2 * 100 = 2.0; final flow adds 100 principal.
        assert!((s.flows()[0].coupon - 2.0).abs() < 1e-12);
        assert!((s.last_flow().unwrap().principal - 100.0).abs() < 1e-12);
        assert!((s.last_flow().unwrap().total() - 102.0).abs() < 1e-12);
    }

    #[test]
    fn rejects_non_ascending() {
        let flows = vec![
            ScheduleFlow {
                date: CivilDate::new(2035, 6, 15),
                coupon: 2.0,
                principal: 0.0,
            },
            ScheduleFlow {
                date: CivilDate::new(2035, 6, 15),
                coupon: 2.0,
                principal: 100.0,
            },
        ];
        assert_eq!(
            BondSchedule::new(flows).unwrap_err(),
            CaError::NonAscendingSchedule
        );
    }

    #[test]
    fn present_value_excludes_settled_flows_and_discounts() {
        let s = BondSchedule::fixed_coupon(
            CivilDate::new(2034, 6, 15),
            CivilDate::new(2035, 6, 15),
            0.04,
            1,
            100.0,
        )
        .expect("schedule");
        // Single flow at maturity 2035-06-15 = 104.0; from 2034-06-15 that is 365 days = 1.0y.
        let pv = s.present_value(CivilDate::new(2034, 6, 15), 0.05);
        assert!((pv - 104.0 * (-0.05_f64).exp()).abs() < 1e-9, "pv {pv}");
        // As-of the maturity date, nothing remains.
        assert_eq!(s.present_value(CivilDate::new(2035, 6, 15), 0.05), 0.0);
    }
}
