//! Forward rate agreement (FRA) pricing and curve risk.
//!
//! A FRA is the single-period building block of the linear rates book: one
//! accrual period `[fixing, maturity]` over which one side receives a fixed
//! rate and the other the simply-compounded forward implied by the discount
//! curve. Valuation is the single-curve, convexity-free identity
//!
//! ```text
//!   PV_receive_fixed = N * ( K * tau * DF(maturity)  -  ( DF(fixing) - DF(maturity) ) )
//! ```
//!
//! The float leg value `DF(fixing) - DF(maturity)` is model-implied and
//! independent of the contractual day-count fraction `tau`; only the fixed leg
//! carries `tau`. This makes a FRA *exactly* a one-period OIS swaplet, which is
//! the structural identity used to validate this module against [`crate::ois`]
//! (`fra_pv` ≡ [`crate::ois::ois_pv`] of the equivalent single-period schedule).
//!
//! Sign convention matches [`crate::ois::ois_pv`]: a positive PV is value to the
//! **fixed-rate receiver**. The fixed-rate payer's PV is the negation.

use crate::bootstrap::{BootstrapError, OisQuote, bootstrap_ois};
use crate::curve::Curve;
use crate::daycount::AccrualBasis;
use crate::risk::ONE_BP;
use celnet_calendar::year_fraction;
use celnet_types::{DayCount, Rate, Time};
use time::Date;

/// A forward rate agreement over the accrual period `[fixing, maturity]`.
///
/// `accrual` is the contractual day-count fraction `tau` of that period (e.g.
/// ACT/360), kept separate from the curve year-fractions so any day-count
/// convention prices correctly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fra {
    /// Fixing time (period start), as a year fraction from the curve reference date.
    pub fixing: Time,
    /// Maturity time (period end), as a year fraction from the curve reference date.
    pub maturity: Time,
    /// Contractual accrual fraction `tau` of the period (day-count applied).
    pub accrual: f64,
    /// Contractual fixed rate `K`.
    pub fixed_rate: Rate,
    /// Notional `N`.
    pub notional: f64,
}

/// Errors constructing a [`Fra`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FraError {
    /// `maturity` is not strictly after `fixing`.
    NonIncreasingTimes,
    /// `accrual` is not strictly positive.
    NonPositiveAccrual,
}

impl core::fmt::Display for FraError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonIncreasingTimes => f.write_str("FRA maturity must be strictly after fixing"),
            Self::NonPositiveAccrual => f.write_str("FRA accrual fraction must be positive"),
        }
    }
}

impl core::error::Error for FraError {}

impl Fra {
    /// Construct a validated FRA.
    ///
    /// # Errors
    /// Returns [`FraError`] if `maturity <= fixing` or `accrual <= 0`.
    pub fn new(
        fixing: Time,
        maturity: Time,
        accrual: f64,
        fixed_rate: Rate,
        notional: f64,
    ) -> Result<Self, FraError> {
        if maturity.0 <= fixing.0 {
            return Err(FraError::NonIncreasingTimes);
        }
        if accrual <= 0.0 {
            return Err(FraError::NonPositiveAccrual);
        }
        Ok(Self {
            fixing,
            maturity,
            accrual,
            fixed_rate,
            notional,
        })
    }

    /// Construct a FRA from calendar dates, taking the accrual fraction `tau`
    /// from `accrual_basis` (e.g. ACT/360 or 30/360 Bond Basis).
    ///
    /// The contractual accrual `tau` is `accrual_basis.year_fraction(fixing_date,
    /// maturity_date)` — this is the only place the leg's day-count convention
    /// enters pricing. The curve coordinates `fixing` and `maturity` are measured
    /// ACT/365F from `reference`, matching the curve time axis used by
    /// [`crate::schedule`] (so a FRA and the OIS/discount curve share one axis).
    ///
    /// # Errors
    /// Returns [`FraError`] if `maturity_date <= fixing_date` (non-increasing
    /// curve times) or the resulting `tau <= 0`.
    pub fn from_dates(
        reference: Date,
        fixing_date: Date,
        maturity_date: Date,
        accrual_basis: AccrualBasis,
        fixed_rate: Rate,
        notional: f64,
    ) -> Result<Self, FraError> {
        let fixing = year_fraction(DayCount::Act365Fixed, reference, fixing_date);
        let maturity = year_fraction(DayCount::Act365Fixed, reference, maturity_date);
        let accrual = accrual_basis.year_fraction(fixing_date, maturity_date).0;
        Self::new(fixing, maturity, accrual, fixed_rate, notional)
    }
}

/// PV of the FRA to the **fixed-rate receiver**, discounted to the curve date.
///
/// `N * ( K * tau * DF(maturity) - ( DF(fixing) - DF(maturity) ) )`.
#[must_use]
pub fn fra_pv(curve: &Curve, fra: &Fra) -> f64 {
    let df_fix = curve.discount_factor(fra.fixing).0;
    let df_mat = curve.discount_factor(fra.maturity).0;
    let fixed_leg = fra.fixed_rate.0 * fra.accrual * df_mat;
    let float_leg = df_fix - df_mat;
    fra.notional * (fixed_leg - float_leg)
}

/// Par (break-even) fixed rate: the `K` that zeroes [`fra_pv`].
///
/// `( DF(fixing) / DF(maturity) - 1 ) / tau` — the simply-compounded forward for
/// the contractual day count.
#[must_use]
pub fn fra_par_rate(curve: &Curve, fra: &Fra) -> Rate {
    let df_fix = curve.discount_factor(fra.fixing).0;
    let df_mat = curve.discount_factor(fra.maturity).0;
    Rate((df_fix / df_mat - 1.0) / fra.accrual)
}

/// Analytic PV01: PV change in absolute terms for a 1bp move in the fixed rate.
///
/// `N * tau * DF(maturity) * 1bp` — the single-period fixed-leg annuity scaled to
/// one basis point, matching the convention of [`crate::risk::pv01`].
#[must_use]
pub fn fra_pv01(curve: &Curve, fra: &Fra) -> f64 {
    fra.notional * fra.accrual * curve.discount_factor(fra.maturity).0 * ONE_BP
}

/// Curve risk of a FRA priced off a curve bootstrapped from `quotes`.
///
/// All sensitivities use a symmetric (central) 1bp bump — re-bootstrap, reprice
/// up and down, halve the difference. Central differencing cancels the
/// second-order curvature term, so it is second-order accurate and the per-pillar
/// ladder sums to the parallel `dv01` to within third-order (~1e-8 relative).
///
/// - `dv01`: PV sensitivity to a parallel ±1bp shift of every quoted par rate.
/// - `key_rate`: per-pillar ladder — PV sensitivity to a ±1bp shift of each quote
///   in isolation. The ladder sums to `dv01` (Jacobian completeness).
#[derive(Clone, Debug, PartialEq)]
pub struct FraRisk {
    /// Present value at the bootstrapped curve.
    pub pv: f64,
    /// Analytic fixed-rate PV01 (`N * tau * DF * 1bp`).
    pub pv01: f64,
    /// Parallel-shift DV01.
    pub dv01: f64,
    /// Per-pillar key-rate ladder (one entry per quote, in quote order).
    pub key_rate: Vec<f64>,
}

/// Compute [`FraRisk`] for `fra` against the OIS `quotes`.
///
/// # Errors
/// Propagates any [`BootstrapError`] from re-bootstrapping the bumped curves.
pub fn fra_risk(quotes: &[OisQuote], fra: &Fra) -> Result<FraRisk, BootstrapError> {
    let base = bootstrap_ois(quotes)?;
    let pv = fra_pv(&base, fra);
    let pv01 = fra_pv01(&base, fra);

    // Re-price the FRA against a copy of `quotes` whose par rates are shifted by
    // `delta` on the pillars selected by `bump`.
    let reprice = |bump: &dyn Fn(usize) -> f64| -> Result<f64, BootstrapError> {
        let bumped: Vec<OisQuote> = quotes
            .iter()
            .enumerate()
            .map(|(i, q)| OisQuote {
                schedule: q.schedule.clone(),
                par_rate: Rate(q.par_rate.0 + bump(i)),
            })
            .collect();
        Ok(fra_pv(&bootstrap_ois(&bumped)?, fra))
    };

    let dv01 = (reprice(&|_| ONE_BP)? - reprice(&|_| -ONE_BP)?) / 2.0;

    let mut key_rate = Vec::with_capacity(quotes.len());
    for pillar in 0..quotes.len() {
        let up = reprice(&|i| if i == pillar { ONE_BP } else { 0.0 })?;
        let down = reprice(&|i| if i == pillar { -ONE_BP } else { 0.0 })?;
        key_rate.push((up - down) / 2.0);
    }

    Ok(FraRisk {
        pv,
        pv01,
        dv01,
        key_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ois::{FixedPeriod, OisSchedule, ois_par_rate, ois_pv};

    /// Upward-sloping continuously-compounded zero curve for tests.
    fn test_curve() -> Curve {
        Curve::from_zero_rates(&[
            (Time(0.5), Rate(0.030)),
            (Time(1.0), Rate(0.032)),
            (Time(2.0), Rate(0.035)),
            (Time(3.0), Rate(0.037)),
        ])
        .expect("curve")
    }

    /// The defining identity: a FRA is a one-period OIS swaplet. `fra_pv` must
    /// equal `ois_pv` of the equivalent single-period schedule to machine
    /// precision (FI-VERIFICATION-CONTRACT §FRA).
    #[test]
    fn fra_pv_equals_single_period_swaplet() {
        let curve = test_curve();
        let (t1, t2, tau, k, n) = (Time(1.0), Time(2.0), 1.0, Rate(0.033), 25_000_000.0);
        let fra = Fra::new(t1, t2, tau, k, n).expect("fra");

        let schedule = OisSchedule::new(
            t1,
            vec![FixedPeriod {
                pay: t2,
                accrual: Time(tau),
            }],
        )
        .expect("schedule");
        let swaplet = ois_pv(&curve, &schedule, k, n);

        assert!((fra_pv(&curve, &fra) - swaplet).abs() < 1e-9);
    }

    /// At the par fixed rate the FRA is worth zero, and that par rate matches the
    /// single-period OIS par rate.
    #[test]
    fn par_rate_zeroes_pv_and_matches_ois() {
        let curve = test_curve();
        let mut fra = Fra::new(Time(0.5), Time(1.0), 0.5, Rate(0.0), 1.0).expect("fra");
        let par = fra_par_rate(&curve, &fra);
        fra.fixed_rate = par;
        assert!(fra_pv(&curve, &fra).abs() < 1e-12);

        let schedule = OisSchedule::new(
            Time(0.5),
            vec![FixedPeriod {
                pay: Time(1.0),
                accrual: Time(0.5),
            }],
        )
        .expect("schedule");
        assert!((par.0 - ois_par_rate(&curve, &schedule).0).abs() < 1e-12);
    }

    /// PV is linear in the fixed rate, so the analytic PV01 equals the one-sided
    /// finite difference for a 1bp rate move.
    #[test]
    fn pv01_matches_finite_difference() {
        let curve = test_curve();
        let base = Fra::new(Time(1.0), Time(2.0), 1.0, Rate(0.030), 1.0).expect("fra");
        let mut up = base;
        up.fixed_rate = Rate(base.fixed_rate.0 + ONE_BP);
        let fd = fra_pv(&curve, &up) - fra_pv(&curve, &base);
        assert!((fd - fra_pv01(&curve, &base)).abs() < 1e-15);
    }

    /// A dated FRA prices under a selectable accrual basis. The accrual fraction
    /// `tau` is validated against the QuantLib 1.42.1 day counts for the window
    /// [16 Sep 2025, 16 Dec 2025]: ACT/360 = 91 days, 30/360 Bond Basis = 90 days.
    /// Because only the fixed leg carries `tau`, the par rate scales exactly
    /// inversely with it (par_3360 / par_act == 91/90) and the curve coordinates
    /// are basis-independent.
    #[test]
    fn fra_accrual_basis_selects_day_count_against_quantlib() {
        use crate::daycount::AccrualBasis;
        use time::{Date, Month};

        let reference = Date::from_calendar_date(2025, Month::June, 16).unwrap();
        let fixing_date = Date::from_calendar_date(2025, Month::September, 16).unwrap();
        let maturity_date = Date::from_calendar_date(2025, Month::December, 16).unwrap();
        let curve = test_curve();
        let k = Rate(0.030);
        let n = 50_000_000.0;

        let fra_act = Fra::from_dates(
            reference,
            fixing_date,
            maturity_date,
            AccrualBasis::Act360,
            k,
            n,
        )
        .expect("act/360 fra");
        let fra_3360 = Fra::from_dates(
            reference,
            fixing_date,
            maturity_date,
            AccrualBasis::Thirty360BondBasis,
            k,
            n,
        )
        .expect("30/360 fra");

        // QuantLib-oracle accrual fractions.
        assert!((fra_act.accrual - 91.0 / 360.0).abs() < 1e-15);
        assert!((fra_3360.accrual - 90.0 / 360.0).abs() < 1e-15);

        // Curve coordinates (ACT/365F from the reference) do not depend on the accrual basis.
        assert_eq!(fra_act.fixing, fra_3360.fixing);
        assert_eq!(fra_act.maturity, fra_3360.maturity);

        // Only the fixed leg carries tau ⇒ par rate scales inversely with it.
        let par_act = fra_par_rate(&curve, &fra_act).0;
        let par_3360 = fra_par_rate(&curve, &fra_3360).0;
        assert!(
            par_3360 > par_act,
            "30/360 has the smaller tau ⇒ higher par"
        );
        assert!((par_3360 / par_act - 91.0 / 90.0).abs() < 1e-12);

        // At a shared fixed rate the PV difference is exactly the fixed-leg accrual change.
        let df_mat = curve.discount_factor(fra_act.maturity).0;
        let pv_diff = fra_pv(&curve, &fra_3360) - fra_pv(&curve, &fra_act);
        let expected = n * k.0 * df_mat * (90.0 / 360.0 - 91.0 / 360.0);
        assert!(
            (pv_diff - expected).abs() < 1e-6,
            "pv_diff {pv_diff} expected {expected}"
        );
    }

    /// Key-rate ladder sums to the parallel DV01 (Jacobian completeness,
    /// FI-VERIFICATION-CONTRACT §risk).
    #[test]
    fn key_rate_ladder_sums_to_dv01() {
        let quotes = vec![
            OisQuote {
                schedule: OisSchedule::new(
                    Time(0.0),
                    vec![FixedPeriod {
                        pay: Time(1.0),
                        accrual: Time(1.0),
                    }],
                )
                .expect("schedule"),
                par_rate: Rate(0.030),
            },
            OisQuote {
                schedule: OisSchedule::new(
                    Time(0.0),
                    vec![
                        FixedPeriod {
                            pay: Time(1.0),
                            accrual: Time(1.0),
                        },
                        FixedPeriod {
                            pay: Time(2.0),
                            accrual: Time(1.0),
                        },
                    ],
                )
                .expect("schedule"),
                par_rate: Rate(0.034),
            },
        ];
        let fra = Fra::new(Time(1.0), Time(2.0), 1.0, Rate(0.033), 10_000_000.0).expect("fra");
        let risk = fra_risk(&quotes, &fra).expect("risk");

        assert_eq!(risk.key_rate.len(), 2);
        let ladder: f64 = risk.key_rate.iter().sum();
        // Central differencing makes ladder-additivity hold to third order.
        assert!((ladder - risk.dv01).abs() < 1e-6 * risk.dv01.abs());
        // The 2y pillar dominates a [1y,2y] FRA's curve risk.
        assert!(risk.key_rate[1].abs() > risk.key_rate[0].abs());
    }

    #[test]
    fn rejects_degenerate_fra() {
        assert_eq!(
            Fra::new(Time(2.0), Time(1.0), 1.0, Rate(0.03), 1.0),
            Err(FraError::NonIncreasingTimes)
        );
        assert_eq!(
            Fra::new(Time(1.0), Time(2.0), 0.0, Rate(0.03), 1.0),
            Err(FraError::NonPositiveAccrual)
        );
    }
}
