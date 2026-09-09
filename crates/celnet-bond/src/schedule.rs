//! The cashflow schedule derived from a [`Bond`] — the shared substrate for pricing and risk.
//!
//! Building the schedule is the one place that touches the calendar: it rolls the regular coupon
//! dates back from maturity, locates the coupon period bracketing settlement, and reduces the bond
//! to a flat list of future cashflows carrying both discounting coordinates (street period-units and
//! curve year-fraction) plus the accrued interest. Pricing and risk are then pure arithmetic over
//! this list, so every measure sees exactly the same cashflows.

use celnet_rates::{AccrualBasis, Curve};
use celnet_types::Time;
use time::Date;

use crate::bond::{Bond, BondError};

/// One future cashflow of a bond, as seen from settlement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cashflow {
    /// Discounting exponent in **coupon periods** from settlement: `w + (k − 1)` for the `k`-th
    /// future coupon, where `w ∈ (0, 1]` is the fraction of the current period still remaining.
    pub period_exponent: f64,
    /// Year-fraction time from settlement on the curve's ACT/365F discount-time axis.
    pub curve_time: f64,
    /// The cash amount paid (a coupon, plus `redemption` on the final flow).
    pub amount: f64,
}

/// A bond reduced to its future cashflows and accrued interest, ready for pricing and risk.
///
/// Immutable once built. All measures — dirty/clean/curve price, yield, DV01, duration, convexity —
/// are pure functions of this value and the coupon frequency it carries.
#[derive(Clone, Debug)]
pub(crate) struct CashflowSchedule {
    /// Coupons per year (the yield compounding basis).
    freq: f64,
    /// Accrued interest from the last coupon to settlement (day-count based).
    accrued: f64,
    /// Future cashflows in ascending time order (the final flow includes the redemption).
    flows: Vec<Cashflow>,
}

impl CashflowSchedule {
    /// Reduce `bond` to its future cashflows and accrued interest.
    ///
    /// The regular coupon dates are `maturity − j·(12/f)` months for `j = 0, 1, …`
    /// (end-of-month-aware via [`celnet_calendar::add_months`]); those strictly after settlement are
    /// the future coupons, and the latest one on or before settlement is the current period's start.
    /// The regular coupon amount is `coupon_rate / f · redemption`; the buyer receives the full next
    /// coupon and compensates the seller through the accrued interest
    /// `coupon_rate · redemption · τ(last_coupon, settlement)`.
    ///
    /// # Errors
    ///
    /// Returns [`BondError::DegeneratePeriod`] if the current coupon period has a non-positive
    /// day-count length (a malformed schedule the day-count basis cannot measure).
    pub(crate) fn from_bond(bond: &Bond) -> Result<Self, BondError> {
        let freq_i = bond.frequency().per_year();
        let freq = f64::from(freq_i);
        let step_months = bond.frequency().months();
        let settlement = bond.settlement();
        let maturity = bond.maturity();
        let basis = bond.day_count();

        // Roll regular coupon dates back from maturity until one lands on/before settlement. Those
        // strictly after settlement are future coupons (descending); the first on/before settlement
        // is the current period's start (`previous_coupon`).
        let mut future_desc: Vec<Date> = Vec::new();
        let previous_coupon;
        let mut j: i32 = 0;
        loop {
            let date = celnet_calendar::add_months(maturity, -j * step_months);
            if date <= settlement {
                previous_coupon = date;
                break;
            }
            future_desc.push(date);
            j += 1;
        }
        // `maturity > settlement` (enforced by `Bond::new`) guarantees at least the maturity flow.
        debug_assert!(
            !future_desc.is_empty(),
            "maturity is always a future coupon date"
        );

        // Ascending future coupon dates; `next_coupon` is the first one after settlement.
        future_desc.reverse();
        let future = future_desc;
        let next_coupon = future[0];

        // Fraction of the current coupon period still remaining after settlement, on the accrual
        // basis: `w = τ(settlement, next) / τ(previous, next)`. On a coupon date `w = 1`.
        let period_len = year_fraction(basis, previous_coupon, next_coupon);
        if period_len <= 0.0 {
            return Err(BondError::DegeneratePeriod);
        }
        let remaining = year_fraction(basis, settlement, next_coupon);
        let w = remaining / period_len;

        let effective_redemption = bond.effective_redemption();
        let regular_coupon = bond.coupon_rate() / freq * effective_redemption;
        let accrued = bond.coupon_rate()
            * effective_redemption
            * year_fraction(basis, previous_coupon, settlement);

        let last = future.len() - 1;
        let flows: Vec<Cashflow> = future
            .iter()
            .enumerate()
            .map(|(k, &date)| {
                let amount = if k == last {
                    regular_coupon + effective_redemption
                } else {
                    regular_coupon
                };
                Cashflow {
                    period_exponent: w + k as f64,
                    curve_time: year_fraction(AccrualBasis::Act365Fixed, settlement, date),
                    amount,
                }
            })
            .collect();

        Ok(Self {
            freq,
            accrued,
            flows,
        })
    }

    /// Coupons per year — the yield compounding basis.
    #[must_use]
    pub(crate) fn freq(&self) -> f64 {
        self.freq
    }

    /// Accrued interest from the last coupon to settlement.
    #[must_use]
    pub(crate) fn accrued(&self) -> f64 {
        self.accrued
    }

    /// Dirty price at a flat periodically-compounded `yield_`: `Σ CFₖ · (1 + y/f)^(−eₖ)`.
    ///
    /// Evaluated via the Horner-like discount recurrence `Dₖ = D_{k-1} · v` where `v = (1 + y/f)⁻¹`
    /// and `D₀ = (1 + y/f)^{−w}`, reducing `N` transcendental `powf` calls to a single `powf`
    /// followed by `N - 1` fast multiplications.
    ///
    /// Returns `+∞` when `1 + y/f ≤ 0` (a yield below `−f`), so a root-find treats that region as
    /// unboundedly rich and brackets away from it rather than producing a `NaN`.
    #[must_use]
    pub(crate) fn dirty_price_at_yield(&self, yield_: f64) -> f64 {
        let base = 1.0 + yield_ / self.freq;
        if base <= 0.0 {
            return f64::INFINITY;
        }
        if self.flows.is_empty() {
            return 0.0;
        }
        let w = self.flows[0].period_exponent;
        let v = 1.0 / base;
        let mut d_k = base.powf(-w);
        let mut sum = self.flows[0].amount * d_k;
        for c in &self.flows[1..] {
            d_k *= v;
            sum += c.amount * d_k;
        }
        sum
    }

    /// `∂(dirty price)/∂y = −(1/f) · Σ eₖ · CFₖ · (1 + y/f)^(−eₖ−1)` (analytic, strictly negative).
    ///
    /// Evaluated via the same `Dₖ = D_{k-1} · v` recurrence with a single `powf` call.
    #[must_use]
    pub(crate) fn dirty_price_first_derivative(&self, yield_: f64) -> f64 {
        let base = 1.0 + yield_ / self.freq;
        if base <= 0.0 {
            return f64::NEG_INFINITY;
        }
        if self.flows.is_empty() {
            return 0.0;
        }
        let w = self.flows[0].period_exponent;
        let v = 1.0 / base;
        let mut d_k = base.powf(-w);
        let mut sum = self.flows[0].period_exponent * self.flows[0].amount * d_k;
        for c in &self.flows[1..] {
            d_k *= v;
            sum += c.period_exponent * c.amount * d_k;
        }
        -(sum * v) / self.freq
    }

    /// Compute dirty price and its first derivative in a single unified pass over cashflows.
    #[must_use]
    pub(crate) fn dirty_price_and_first_derivative(&self, yield_: f64) -> (f64, f64) {
        let base = 1.0 + yield_ / self.freq;
        if base <= 0.0 {
            return (f64::INFINITY, f64::NEG_INFINITY);
        }
        if self.flows.is_empty() {
            return (0.0, 0.0);
        }
        let w = self.flows[0].period_exponent;
        let v = 1.0 / base;
        let mut d_k = base.powf(-w);
        let mut price = self.flows[0].amount * d_k;
        let mut deriv = self.flows[0].period_exponent * self.flows[0].amount * d_k;
        for c in &self.flows[1..] {
            d_k *= v;
            price += c.amount * d_k;
            deriv += c.period_exponent * c.amount * d_k;
        }
        (price, -(deriv * v) / self.freq)
    }

    /// `∂²(dirty price)/∂y² = (1/f²) · Σ eₖ(eₖ+1) · CFₖ · (1 + y/f)^(−eₖ−2)` (analytic).
    #[must_use]
    pub(crate) fn dirty_price_second_derivative(&self, yield_: f64) -> f64 {
        let base = 1.0 + yield_ / self.freq;
        if base <= 0.0 {
            return f64::INFINITY;
        }
        if self.flows.is_empty() {
            return 0.0;
        }
        let w = self.flows[0].period_exponent;
        let v = 1.0 / base;
        let mut d_k = base.powf(-w);
        let mut sum = self.flows[0].period_exponent
            * (self.flows[0].period_exponent + 1.0)
            * self.flows[0].amount
            * d_k;
        for c in &self.flows[1..] {
            d_k *= v;
            sum += c.period_exponent * (c.period_exponent + 1.0) * c.amount * d_k;
        }
        (sum * v * v) / (self.freq * self.freq)
    }

    /// Dirty price off a discount curve: `Σ CFₖ · DF(tₖ)` at the curve's ACT/365F time axis.
    #[must_use]
    pub(crate) fn price_on_curve(&self, curve: &Curve) -> f64 {
        self.flows
            .iter()
            .map(|c| c.amount * curve.discount_factor(Time(c.curve_time)).0)
            .sum()
    }

    /// Dirty price off a discount curve with a **continuously-compounded z-spread** `z` added
    /// to every zero rate: `Σ CFₖ · DF(tₖ) · e^(−z·tₖ)`.
    ///
    /// The spread is applied on the curve's own continuous zero-rate convention, so
    /// `z = 0` reproduces [`Self::price_on_curve`] exactly.
    #[must_use]
    pub(crate) fn price_on_curve_with_spread(&self, curve: &Curve, z: f64) -> f64 {
        self.flows
            .iter()
            .map(|c| {
                c.amount * curve.discount_factor(Time(c.curve_time)).0 * (-z * c.curve_time).exp()
            })
            .sum()
    }

    /// `∂P/∂z` of [`Self::price_on_curve_with_spread`] — the **analytic** first derivative
    /// `−Σ CFₖ · tₖ · DF(tₖ) · e^(−z·tₖ)` (strictly negative for a positive-cashflow bond,
    /// which is what makes the z-spread solve single-rooted and Newton-safe).
    #[must_use]
    pub(crate) fn curve_price_spread_derivative(&self, curve: &Curve, z: f64) -> f64 {
        -self
            .flows
            .iter()
            .map(|c| {
                c.amount
                    * c.curve_time
                    * curve.discount_factor(Time(c.curve_time)).0
                    * (-z * c.curve_time).exp()
            })
            .sum::<f64>()
    }

    /// The number of future cashflows (coupons, the last carrying the redemption).
    #[cfg(test)]
    pub(crate) fn flow_count(&self) -> usize {
        self.flows.len()
    }

    /// The `curve_time`s of the future cashflows, ascending (test-only introspection).
    #[cfg(test)]
    pub(crate) fn curve_times(&self) -> Vec<f64> {
        self.flows.iter().map(|c| c.curve_time).collect()
    }

    /// The discounting exponents of the future cashflows (test-only introspection).
    #[cfg(test)]
    pub(crate) fn period_exponents(&self) -> Vec<f64> {
        self.flows.iter().map(|c| c.period_exponent).collect()
    }
}

/// Day-count year fraction on `basis` — the shared `celnet-rates` accrual convention.
fn year_fraction(basis: AccrualBasis, start: Date, end: Date) -> f64 {
    basis.year_fraction(start, end).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_rates::PaymentFrequency;
    use time::Month;

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).expect("valid date")
    }

    /// A 3y 6% semi-annual bond settling exactly on a coupon date (15 Jun 2032, matures 15 Jun 2035).
    fn on_coupon_date() -> Bond {
        Bond::new(
            d(2032, Month::June, 15),
            d(2035, Month::June, 15),
            0.06,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .expect("valid bond")
    }

    #[test]
    fn builds_regular_coupon_count() {
        let s = CashflowSchedule::from_bond(&on_coupon_date()).expect("schedule");
        // 3 years semi-annual → 6 future coupons.
        assert_eq!(s.flow_count(), 6);
    }

    #[test]
    fn on_coupon_date_has_integer_exponents_and_zero_accrued() {
        let s = CashflowSchedule::from_bond(&on_coupon_date()).expect("schedule");
        // Settlement is a coupon date ⇒ w = 1 ⇒ exponents 1, 2, …, 6 and no accrued interest.
        let exps = s.period_exponents();
        for (i, e) in exps.iter().enumerate() {
            assert!(
                (e - (i as f64 + 1.0)).abs() < 1e-12,
                "exponent {i} = {e}, want {}",
                i + 1
            );
        }
        assert!(s.accrued().abs() < 1e-12, "accrued {}", s.accrued());
    }

    #[test]
    fn mid_period_settlement_has_fractional_first_exponent_and_positive_accrued() {
        // Settle 3 months into a 6-month period (15 Sep 2032): w ≈ 0.5, accrued ≈ a quarter-year of
        // coupon on 30/360.
        let bond = Bond::new(
            d(2032, Month::September, 15),
            d(2035, Month::June, 15),
            0.06,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .expect("valid bond");
        let s = CashflowSchedule::from_bond(&bond).expect("schedule");
        let first = s.period_exponents()[0];
        // 15 Sep → 15 Dec is 90/360 = 0.25y remaining of a 0.5y period ⇒ w = 0.5.
        assert!((first - 0.5).abs() < 1e-9, "first exponent {first}");
        // Accrued for 15 Jun → 15 Sep = 90/360 = 0.25y ⇒ 0.06 · 100 · 0.25 = 1.5.
        assert!((s.accrued() - 1.5).abs() < 1e-9, "accrued {}", s.accrued());
        // The next coupon is still received in full.
        assert_eq!(s.flow_count(), 6);
    }

    #[test]
    fn curve_times_are_strictly_increasing_and_positive() {
        let s = CashflowSchedule::from_bond(&on_coupon_date()).expect("schedule");
        let ts = s.curve_times();
        assert!(ts[0] > 0.0);
        for w in ts.windows(2) {
            assert!(w[1] > w[0], "curve times not increasing: {w:?}");
        }
    }
}
