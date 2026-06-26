//! Overnight-index-swap (OIS) pricing on a self-discounting curve.
//!
//! For a collateralised OIS whose discount and projection curve are the same (the USD-SOFR
//! self-discounting case, `FI-CURVES-SPEC.md` §6.4), the compounded-overnight float leg telescopes
//! to an exact discount-factor identity: per unit notional its present value over `[t0, tN]` is
//! `DF(t0) − DF(tN)`. The fixed leg is the annuity `Σ δ_i · DF(pay_i)`. Hence the par (fair fixed)
//! rate is `(DF(t0) − DF(tN)) / annuity` — the identity the sequential bootstrap inverts pillar by
//! pillar (`build::bootstrap_ois`).
//!
//! Everything here is in continuous year-fraction time; turning `(effective date, tenor, USD-OIS
//! convention)` into an [`OisSchedule`] is the calendar/day-count layer of a later slice.

use crate::curve::Curve;
use celnet_types::{Rate, Time};

/// Absolute tolerance for treating a schedule time as the curve origin.
const TIME_TOL: f64 = 1e-12;

/// One accrual period of the OIS fixed leg.
#[derive(Clone, Copy, Debug)]
pub struct FixedPeriod {
    /// Payment time (period end) in curve year-fraction coordinates.
    pub pay: Time,
    /// Year-fraction accrual for the period (e.g. ACT/360); strictly positive.
    pub accrual: Time,
}

/// The schedule of an OIS: its effective start and the ordered fixed-leg accrual periods.
///
/// The periods must be strictly increasing in payment time and start after the effective date;
/// the final period's payment time is the swap maturity.
#[derive(Clone, Debug)]
pub struct OisSchedule {
    start: Time,
    periods: Vec<FixedPeriod>,
}

/// Construction error for an [`OisSchedule`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// No fixed-leg periods were supplied.
    Empty,
    /// The first payment time was not strictly after the effective start.
    StartNotBeforeFirstPay,
    /// Payment times were not strictly increasing.
    NonIncreasingPayTimes,
    /// A period had a non-positive year-fraction accrual.
    NonPositiveAccrual,
}

impl core::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let m = match self {
            Self::Empty => "OIS schedule has no fixed-leg periods",
            Self::StartNotBeforeFirstPay => {
                "first payment must be strictly after the effective start"
            }
            Self::NonIncreasingPayTimes => "payment times must be strictly increasing",
            Self::NonPositiveAccrual => "period accruals must be strictly positive",
        };
        f.write_str(m)
    }
}

impl core::error::Error for ScheduleError {}

impl OisSchedule {
    /// Build a schedule from an effective start and its ordered fixed-leg periods.
    ///
    /// # Errors
    ///
    /// Returns a [`ScheduleError`] if there are no periods, the first payment is not after the
    /// start, payment times are not strictly increasing, or an accrual is non-positive.
    pub fn new(start: Time, periods: Vec<FixedPeriod>) -> Result<Self, ScheduleError> {
        if periods.is_empty() {
            return Err(ScheduleError::Empty);
        }
        let mut prev = start.0;
        for (i, p) in periods.iter().enumerate() {
            if p.accrual.0 <= 0.0 {
                return Err(ScheduleError::NonPositiveAccrual);
            }
            if p.pay.0 <= prev {
                return Err(if i == 0 {
                    ScheduleError::StartNotBeforeFirstPay
                } else {
                    ScheduleError::NonIncreasingPayTimes
                });
            }
            prev = p.pay.0;
        }
        Ok(Self { start, periods })
    }

    /// The effective (start) time of the swap.
    #[must_use]
    pub fn start(&self) -> Time {
        self.start
    }

    /// The swap maturity — the final period's payment time.
    #[must_use]
    pub fn maturity(&self) -> Time {
        self.periods[self.periods.len() - 1].pay
    }

    /// The fixed-leg accrual periods, in payment-time order.
    #[must_use]
    pub fn periods(&self) -> &[FixedPeriod] {
        &self.periods
    }
}

/// The fixed-leg annuity `A = Σ δ_i · DF(pay_i)` (per unit notional).
#[must_use]
pub fn ois_annuity(curve: &Curve, schedule: &OisSchedule) -> f64 {
    schedule
        .periods()
        .iter()
        .map(|p| p.accrual.0 * curve.discount_factor(p.pay).0)
        .sum()
}

/// The par (fair fixed) rate `K* = (DF(start) − DF(maturity)) / annuity`.
#[must_use]
pub fn ois_par_rate(curve: &Curve, schedule: &OisSchedule) -> Rate {
    let df_start = curve.discount_factor(schedule.start()).0;
    let df_maturity = curve.discount_factor(schedule.maturity()).0;
    Rate((df_start - df_maturity) / ois_annuity(curve, schedule))
}

/// Present value of **receiving fixed** at `fixed_rate` on `notional`:
/// `notional · (K·A − (DF(start) − DF(maturity)))`.
///
/// Positive when the fixed rate exceeds the par rate; pay-fixed is the negation. Returned in the
/// discount curve's settlement currency, scaled by `notional`.
#[must_use]
pub fn ois_pv(curve: &Curve, schedule: &OisSchedule, fixed_rate: Rate, notional: f64) -> f64 {
    let annuity = ois_annuity(curve, schedule);
    let df_start = curve.discount_factor(schedule.start()).0;
    let df_maturity = curve.discount_factor(schedule.maturity()).0;
    notional * (fixed_rate.0 * annuity - (df_start - df_maturity))
}

/// True when the schedule starts at the curve origin (a spot-starting swap on a spot-referenced
/// curve) — the case the sequential bootstrap calibrates.
#[must_use]
pub(crate) fn is_spot_starting(schedule: &OisSchedule) -> bool {
    schedule.start().0.abs() <= TIME_TOL
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An annual fixed-leg schedule covering `[0, n]` with unit-year accruals.
    fn annual(n: usize) -> OisSchedule {
        let periods = (1..=n)
            .map(|i| FixedPeriod {
                pay: Time(i as f64),
                accrual: Time(1.0),
            })
            .collect();
        OisSchedule::new(Time(0.0), periods).expect("valid annual schedule")
    }

    #[test]
    fn par_rate_matches_single_period_identity() {
        // One annual period: par = (1 - DF(1)) / DF(1).
        let curve = Curve::from_zero_rates(&[(Time(1.0), Rate(0.04))]).unwrap();
        let sched = annual(1);
        let df1 = curve.discount_factor(Time(1.0)).0;
        let expected = (1.0 - df1) / df1;
        assert!((ois_par_rate(&curve, &sched).0 - expected).abs() < 1e-12);
    }

    #[test]
    fn pv_is_zero_at_the_par_rate() {
        let curve =
            Curve::from_zero_rates(&[(Time(1.0), Rate(0.040)), (Time(5.0), Rate(0.043))]).unwrap();
        let sched = annual(5);
        let par = ois_par_rate(&curve, &sched);
        assert!(ois_pv(&curve, &sched, par, 100.0).abs() < 1e-10);
    }

    #[test]
    fn receive_fixed_pv_rises_above_par() {
        let curve = Curve::from_zero_rates(&[(Time(3.0), Rate(0.04))]).unwrap();
        let sched = annual(3);
        let par = ois_par_rate(&curve, &sched);
        let above = ois_pv(&curve, &sched, Rate(par.0 + 0.001), 100.0);
        assert!(
            above > 0.0,
            "receiving an above-par fixed rate must be a positive PV"
        );
    }

    #[test]
    fn rejects_malformed_schedules() {
        assert_eq!(
            OisSchedule::new(Time(0.0), vec![]).unwrap_err(),
            ScheduleError::Empty
        );
        assert_eq!(
            OisSchedule::new(
                Time(1.0),
                vec![FixedPeriod {
                    pay: Time(0.5),
                    accrual: Time(0.5)
                }]
            )
            .unwrap_err(),
            ScheduleError::StartNotBeforeFirstPay
        );
        assert_eq!(
            OisSchedule::new(
                Time(0.0),
                vec![
                    FixedPeriod {
                        pay: Time(2.0),
                        accrual: Time(1.0)
                    },
                    FixedPeriod {
                        pay: Time(1.0),
                        accrual: Time(1.0)
                    },
                ],
            )
            .unwrap_err(),
            ScheduleError::NonIncreasingPayTimes
        );
        assert_eq!(
            OisSchedule::new(
                Time(0.0),
                vec![FixedPeriod {
                    pay: Time(1.0),
                    accrual: Time(0.0)
                }]
            )
            .unwrap_err(),
            ScheduleError::NonPositiveAccrual
        );
    }

    #[test]
    fn spot_starting_predicate() {
        assert!(is_spot_starting(&annual(2)));
        let forward = OisSchedule::new(
            Time(0.5),
            vec![FixedPeriod {
                pay: Time(1.5),
                accrual: Time(1.0),
            }],
        )
        .unwrap();
        assert!(!is_spot_starting(&forward));
    }
}
