//! Money-market cash deposit — the conventional front pillar of a curve build.
//!
//! A deposit (a.k.a. a cash or money-market deposit) lends notional from the spot
//! date to a near maturity at a single simply-compounded rate `r`. Its only
//! cashflow identity is the simple-interest discount factor over the accrual
//! `[spot, maturity]`:
//!
//! ```text
//!   DF(maturity) = DF(spot) / ( 1 + r · τ ) = 1 / ( 1 + r · τ )
//! ```
//!
//! where `τ` is the deposit's day-count year fraction (ACT/360 for USD/EUR money
//! markets). Because the deposit starts at the curve origin (spot ⇒ `DF(spot) = 1`)
//! the pillar discount factor is **closed-form** — no root-find — so a deposit
//! contributes the short-end (ON / 1W / 1M / 3M) pillars of a sequential curve
//! build directly (`FI-CURVES-SPEC.md` §5.1, the cash stub that anchors the
//! futures/FRA strip above it).
//!
//! The model par (break-even) rate the curve implies for a deposit is the inverse
//! identity `(DF(spot)/DF(maturity) − 1) / τ`; on a curve that carries the pillar
//! above it recovers `r` exactly, which is the reprice-to-par check the bootstrap
//! must satisfy. Method/paper provenance lives in prose only (GUIDE.md §8).

use crate::curve::Curve;
use crate::daycount::AccrualBasis;
use celnet_calendar::year_fraction;
use celnet_types::{DayCount, Df, Rate, Time};
use time::Date;

/// A simply-compounded money-market deposit over `[spot, maturity]`.
///
/// `accrual` is the contractual day-count fraction `τ` of the period (e.g.
/// ACT/360), kept separate from the curve year-fraction coordinate `maturity`
/// (measured ACT/365F from the spot reference) so any money-market day count
/// prices correctly — exactly as a [`crate::fra::Fra`] separates its `tau` from
/// the curve coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Deposit {
    /// Maturity time coordinate (ACT/365F from the curve reference / spot); strictly positive.
    pub maturity: Time,
    /// Contractual accrual fraction `τ` over `[spot, maturity]` on the deposit's basis; positive.
    pub accrual: f64,
    /// Quoted simply-compounded deposit rate `r`.
    pub rate: Rate,
}

/// Errors constructing a [`Deposit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepositError {
    /// `maturity` is not strictly after the curve origin (spot).
    NonPositiveMaturity,
    /// `accrual` is not strictly positive.
    NonPositiveAccrual,
}

impl core::fmt::Display for DepositError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonPositiveMaturity => {
                f.write_str("deposit maturity must be strictly after the curve origin")
            }
            Self::NonPositiveAccrual => f.write_str("deposit accrual fraction must be positive"),
        }
    }
}

impl core::error::Error for DepositError {}

impl Deposit {
    /// Construct a validated deposit on the curve time axis.
    ///
    /// # Errors
    /// Returns [`DepositError`] if `maturity <= 0` or `accrual <= 0`.
    pub fn new(maturity: Time, accrual: f64, rate: Rate) -> Result<Self, DepositError> {
        if maturity.0 <= 0.0 {
            return Err(DepositError::NonPositiveMaturity);
        }
        if accrual <= 0.0 {
            return Err(DepositError::NonPositiveAccrual);
        }
        Ok(Self {
            maturity,
            accrual,
            rate,
        })
    }

    /// Construct a deposit from calendar dates, taking the accrual fraction `τ`
    /// from `accrual_basis` (e.g. ACT/360 for a USD money-market deposit).
    ///
    /// The curve maturity coordinate is measured ACT/365F from `reference` (the
    /// spot date / curve origin), matching the curve time axis used by
    /// [`crate::schedule`]; the contractual accrual `τ` is the only place the
    /// deposit's day-count convention enters pricing.
    ///
    /// # Errors
    /// Returns [`DepositError`] if `maturity_date <= reference` (non-positive
    /// curve maturity) or the resulting `τ <= 0`.
    pub fn from_dates(
        reference: Date,
        maturity_date: Date,
        accrual_basis: AccrualBasis,
        rate: Rate,
    ) -> Result<Self, DepositError> {
        let maturity = year_fraction(DayCount::Act365Fixed, reference, maturity_date);
        let accrual = accrual_basis.year_fraction(reference, maturity_date).0;
        Self::new(maturity, accrual, rate)
    }
}

/// The discount factor a deposit pins at its maturity: `1 / (1 + r · τ)`.
///
/// This is the closed-form pillar the sequential bootstrap places without a
/// root-find. The growth factor `1 + r · τ` is strictly positive for any sensible
/// money-market quote; a non-positive growth factor is a degenerate deposit the
/// curve builder rejects.
#[must_use]
pub fn deposit_discount_factor(deposit: &Deposit) -> Df {
    Df(1.0 / (1.0 + deposit.rate.0 * deposit.accrual))
}

/// The model par (break-even) deposit rate the `curve` implies:
/// `(DF(spot)/DF(maturity) − 1) / τ`.
///
/// With the deposit starting at the curve origin (`DF(spot) = 1`) this is
/// `(1/DF(maturity) − 1) / τ`. On a curve carrying the deposit's closed-form
/// pillar it recovers the quoted `r` exactly — the reprice-to-par identity.
#[must_use]
pub fn deposit_par_rate(curve: &Curve, deposit: &Deposit) -> Rate {
    let df = curve.discount_factor(deposit.maturity).0;
    Rate((1.0 / df - 1.0) / deposit.accrual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::Curve;
    use time::Month;

    /// The closed-form pillar inverts to the quoted rate: build a one-pillar curve
    /// from the deposit's DF and confirm `deposit_par_rate` recovers `r`.
    #[test]
    fn closed_form_pillar_reprices_to_quote() {
        let deposit = Deposit::new(Time(0.25), 0.25_f64 * 365.0 / 360.0, Rate(0.0432)).expect("ok");
        let df = deposit_discount_factor(&deposit);
        // DF = 1 / (1 + r·τ) ⇒ exact closed form.
        assert!((df.0 - 1.0 / (1.0 + 0.0432 * deposit.accrual)).abs() < 1e-15);

        let curve = Curve::from_log_linear_dfs(&[(Time(0.0), Df(1.0)), (deposit.maturity, df)])
            .expect("curve");
        let repriced = deposit_par_rate(&curve, &deposit).0;
        assert!(
            (repriced - 0.0432).abs() < 1e-12,
            "deposit repriced {repriced} vs 0.0432"
        );
    }

    /// A dated deposit takes its accrual fraction from the selected basis. The
    /// window [16 Jun 2025, 16 Sep 2025] is 92 actual days, so ACT/360 = 92/360
    /// and the curve coordinate is ACT/365F = 92/365 (independent day-count
    /// oracle, matching the QuantLib 1.42.1 conventions used elsewhere).
    #[test]
    fn from_dates_takes_accrual_from_basis() {
        let reference = Date::from_calendar_date(2025, Month::June, 16).unwrap();
        let maturity = Date::from_calendar_date(2025, Month::September, 16).unwrap();
        let deposit = Deposit::from_dates(reference, maturity, AccrualBasis::Act360, Rate(0.043))
            .expect("ok");
        assert!((deposit.accrual - 92.0 / 360.0).abs() < 1e-15);
        assert!((deposit.maturity.0 - 92.0 / 365.0).abs() < 1e-15);
    }

    #[test]
    fn rejects_degenerate_deposits() {
        assert_eq!(
            Deposit::new(Time(0.0), 0.25, Rate(0.04)),
            Err(DepositError::NonPositiveMaturity)
        );
        assert_eq!(
            Deposit::new(Time(0.25), 0.0, Rate(0.04)),
            Err(DepositError::NonPositiveAccrual)
        );
    }
}
