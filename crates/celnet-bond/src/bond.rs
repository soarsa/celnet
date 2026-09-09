//! The decoupled bond definition and the crate's error type.
//!
//! [`Bond`] is a self-contained analytics input — deliberately **not** the server's reference-data
//! `BondDef` — so this leaf compiles and is validated in isolation (ADR-0018's one-way dependency).

use celnet_rates::{AccrualBasis, PaymentFrequency};
use time::Date;

/// A fixed-coupon cash bond, described at a settlement date.
///
/// The coupon schedule is the regular set of month-step dates rolled back from `maturity` at the
/// coupon `frequency` (see [`crate::schedule`]); the bond pays `coupon_rate / f · redemption` at each
/// coupon and repays `redemption` at maturity. `coupon_rate` and the day count are the *accrual*
/// conventions used for accrued interest; the yield compounding basis is the coupon `frequency`.
///
/// Construct via [`Bond::new`], which validates the inputs; the fields are then immutable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bond {
    settlement: Date,
    maturity: Date,
    coupon_rate: f64,
    frequency: PaymentFrequency,
    day_count: AccrualBasis,
    redemption: f64,
    pool_factor: f64,
}

/// Failure modes of bond construction and analytics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BondError {
    /// `maturity` is not strictly after `settlement`.
    MaturityNotAfterSettlement,
    /// The redemption (face) is not strictly positive and finite.
    NonPositiveRedemption,
    /// The coupon rate is not finite.
    NonFiniteCouponRate,
    /// A coupon period collapsed to a non-positive day-count length (a malformed schedule).
    DegeneratePeriod,
    /// A yield-to-maturity solve was asked for a non-positive or non-finite target price.
    NonPositivePrice,
    /// The target price lies outside the solvable yield range (no bracketing yield exists).
    YieldOutOfRange,
    /// The yield solve did not converge within the iteration cap.
    YieldDidNotConverge,
    /// The pool factor is not finite or not in `(0, 1]`.
    InvalidPoolFactor,
}

impl core::fmt::Display for BondError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            Self::MaturityNotAfterSettlement => "bond maturity must be strictly after settlement",
            Self::NonPositiveRedemption => {
                "the bond redemption must be strictly positive and finite"
            }
            Self::NonFiniteCouponRate => "the bond coupon rate must be finite",
            Self::DegeneratePeriod => "a coupon period has a non-positive day-count length",
            Self::NonPositivePrice => "yield-to-maturity requires a strictly positive target price",
            Self::YieldOutOfRange => {
                "no yield in the solvable range reprices the bond to that price"
            }
            Self::YieldDidNotConverge => {
                "the yield solve did not converge within the iteration cap"
            }
            Self::InvalidPoolFactor => "the pool factor must be finite and in (0, 1]",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for BondError {}

impl Bond {
    /// Assemble a bond from its settlement date, maturity, coupon, and redemption.
    ///
    /// # Errors
    ///
    /// - [`BondError::MaturityNotAfterSettlement`] if `maturity <= settlement`.
    /// - [`BondError::NonPositiveRedemption`] if `redemption <= 0` or is non-finite.
    /// - [`BondError::NonFiniteCouponRate`] if `coupon_rate` is non-finite. (A zero or negative
    ///   coupon is permitted: zero models a zero-coupon bond, negative a below-zero fixed leg.)
    pub fn new(
        settlement: Date,
        maturity: Date,
        coupon_rate: f64,
        frequency: PaymentFrequency,
        day_count: AccrualBasis,
        redemption: f64,
    ) -> Result<Self, BondError> {
        if maturity <= settlement {
            return Err(BondError::MaturityNotAfterSettlement);
        }
        if redemption <= 0.0 || !redemption.is_finite() {
            return Err(BondError::NonPositiveRedemption);
        }
        if !coupon_rate.is_finite() {
            return Err(BondError::NonFiniteCouponRate);
        }
        Ok(Self {
            settlement,
            maturity,
            coupon_rate,
            frequency,
            day_count,
            redemption,
            pool_factor: 1.0,
        })
    }

    /// Assemble a bond with an explicit `pool_factor` (e.g. following a pro-rata corporate action).
    ///
    /// # Errors
    /// Returns [`BondError::InvalidPoolFactor`] if `pool_factor ∉ (0, 1]` or is non-finite.
    pub fn with_pool_factor(mut self, pool_factor: f64) -> Result<Self, BondError> {
        if !(pool_factor.is_finite() && pool_factor > 0.0 && pool_factor <= 1.0) {
            return Err(BondError::InvalidPoolFactor);
        }
        self.pool_factor = pool_factor;
        Ok(self)
    }

    /// The pool factor of the bond, defaulting to `1.0`.
    #[must_use]
    pub fn pool_factor(&self) -> f64 {
        self.pool_factor
    }

    /// The effective redemption amount taking the pool factor into account: `redemption * pool_factor`.
    #[must_use]
    pub fn effective_redemption(&self) -> f64 {
        self.redemption * self.pool_factor
    }

    /// The settlement date (the valuation date and the curve reference date).
    #[must_use]
    pub fn settlement(&self) -> Date {
        self.settlement
    }

    /// The maturity date, on which `redemption` is repaid alongside the final coupon.
    #[must_use]
    pub fn maturity(&self) -> Date {
        self.maturity
    }

    /// The annualised coupon rate (a fraction, e.g. `0.06` for a 6% coupon).
    #[must_use]
    pub fn coupon_rate(&self) -> f64 {
        self.coupon_rate
    }

    /// The coupon payment frequency, which also sets the yield compounding basis.
    #[must_use]
    pub fn frequency(&self) -> PaymentFrequency {
        self.frequency
    }

    /// The day-count (accrual) basis used for accrued interest.
    #[must_use]
    pub fn day_count(&self) -> AccrualBasis {
        self.day_count
    }

    /// The redemption (face) repaid at maturity.
    #[must_use]
    pub fn redemption(&self) -> f64 {
        self.redemption
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).expect("valid date")
    }

    #[test]
    fn rejects_maturity_on_or_before_settlement() {
        assert_eq!(
            Bond::new(
                d(2032, Month::June, 15),
                d(2032, Month::June, 15),
                0.05,
                PaymentFrequency::SemiAnnual,
                AccrualBasis::Thirty360BondBasis,
                100.0,
            )
            .unwrap_err(),
            BondError::MaturityNotAfterSettlement
        );
    }

    #[test]
    fn rejects_non_positive_redemption() {
        assert_eq!(
            Bond::new(
                d(2025, Month::June, 16),
                d(2030, Month::June, 16),
                0.05,
                PaymentFrequency::SemiAnnual,
                AccrualBasis::Act365Fixed,
                0.0,
            )
            .unwrap_err(),
            BondError::NonPositiveRedemption
        );
    }

    #[test]
    fn rejects_non_finite_coupon() {
        assert_eq!(
            Bond::new(
                d(2025, Month::June, 16),
                d(2030, Month::June, 16),
                f64::NAN,
                PaymentFrequency::SemiAnnual,
                AccrualBasis::Act365Fixed,
                100.0,
            )
            .unwrap_err(),
            BondError::NonFiniteCouponRate
        );
    }

    #[test]
    fn accepts_zero_coupon() {
        let b = Bond::new(
            d(2025, Month::June, 16),
            d(2030, Month::June, 16),
            0.0,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Act365Fixed,
            100.0,
        );
        assert!(b.is_ok());
    }

    #[test]
    fn pool_factor_validation_and_scaling() {
        let b = Bond::new(
            d(2025, Month::June, 16),
            d(2030, Month::June, 16),
            0.05,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Act365Fixed,
            100.0,
        )
        .expect("valid bond");
        assert_eq!(b.pool_factor(), 1.0);
        assert_eq!(b.effective_redemption(), 100.0);

        // Rejection of invalid factors
        assert_eq!(
            b.with_pool_factor(0.0).unwrap_err(),
            BondError::InvalidPoolFactor
        );
        assert_eq!(
            b.with_pool_factor(-0.5).unwrap_err(),
            BondError::InvalidPoolFactor
        );
        assert_eq!(
            b.with_pool_factor(1.05).unwrap_err(),
            BondError::InvalidPoolFactor
        );
        assert_eq!(
            b.with_pool_factor(f64::NAN).unwrap_err(),
            BondError::InvalidPoolFactor
        );

        // Valid factor scaling
        let b_partial = b.with_pool_factor(0.70).expect("valid pool factor");
        assert_eq!(b_partial.pool_factor(), 0.70);
        assert_eq!(b_partial.effective_redemption(), 70.0);

        let p_full = crate::price::dirty_price(&b, celnet_types::Rate(0.05)).expect("price");
        let p_part = crate::price::dirty_price(&b_partial, celnet_types::Rate(0.05)).expect("price");
        assert!((p_part - p_full * 0.70).abs() < 1e-10);
    }
}
