//! Interest-rate futures — STIR (short-term rate) convexity and bond-future delivery analytics.
//!
//! Two distinct families (`FI-ARCHITECTURE.md` §1, futures), both built on the curve and the cash
//! bond already in this crate:
//!
//! ## STIR futures
//!
//! A short-term-rate future settles on a reference rate fixed over `[T1, T2]`. Because the future
//! is margined daily, its rate exceeds the curve's forward fixing by a **convexity adjustment**. We
//! apply the closed-form deterministic adjustment of a one-factor Gaussian (constant-volatility,
//! normal) short-rate model:
//!
//! ```text
//! futures_rate = forward_simple(T1, T2) + ½ · σ² · T1 · T2
//! futures_price = 100 · (1 − futures_rate)
//! ```
//!
//! `σ` is the absolute (normal) annual volatility of the short rate, supplied by the caller — the
//! deterministic convexity choice recorded in the curves spec (`FI-CURVES-SPEC.md`, Q11). A
//! stochastic-vol calibration is a later slice; the seam is the single `σ` input. The adjustment is
//! non-negative, vanishes as `σ → 0` (the future then prices off the pure forward), and grows with
//! `σ` and with time-to-fixing.
//!
//! ## Bond futures
//!
//! A bond future delivers one of a basket of deliverable bonds against an invoice of
//! `futures_price · conversion_factor`. The **conversion factor** is the deliverable's price per
//! unit face at the contract's notional yield (reusing [`price_at_yield`]); the **gross basis**,
//! **implied repo rate**, and **cheapest-to-deliver** selection follow from it.
//!
//! ## Deliberately not in this slice (no stubs — coordinated follow-ups)
//!
//! - **Delivery-window accrued interest and intervening coupons** in the implied-repo cashflows.
//!   The deliverables are spot-settled (accrued = 0) and the horizon carries no coupon, so the
//!   implied repo is the clean carry to delivery; the dated settlement layer is tracked in
//!   `FI-STATUS.md`.
//! - **Stochastic / term-structure-of-vol convexity** (single constant `σ` here).
//!
//! Method/paper provenance lives in prose only — never in identifiers (GUIDE.md §8).

use crate::bond::{CashBond, price_at_yield};
use crate::curve::Curve;
use celnet_types::{Rate, Time};

/// Failure modes of futures construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FutureError {
    /// The reference-rate fixing window is not strictly increasing and non-negative.
    InvalidFixingWindow,
    /// A supplied volatility or price input is negative.
    NegativeInput,
}

impl core::fmt::Display for FutureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidFixingWindow => f.write_str("the fixing window must satisfy 0 <= T1 < T2"),
            Self::NegativeInput => f.write_str("volatility and price inputs must be non-negative"),
        }
    }
}

impl core::error::Error for FutureError {}

/// A short-term-rate future referencing the simple rate fixed over `[fixing_start, fixing_end]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StirFuture {
    fixing_start: Time,
    fixing_end: Time,
}

impl StirFuture {
    /// Construct a STIR future over `[fixing_start, fixing_end]`.
    ///
    /// # Errors
    ///
    /// Returns [`FutureError::InvalidFixingWindow`] unless `0 <= fixing_start < fixing_end`.
    pub fn new(fixing_start: Time, fixing_end: Time) -> Result<Self, FutureError> {
        if !(fixing_start.0 >= 0.0 && fixing_end.0 > fixing_start.0) {
            return Err(FutureError::InvalidFixingWindow);
        }
        Ok(Self {
            fixing_start,
            fixing_end,
        })
    }

    /// The fixing window start.
    #[must_use]
    pub fn fixing_start(&self) -> Time {
        self.fixing_start
    }

    /// The fixing window end.
    #[must_use]
    pub fn fixing_end(&self) -> Time {
        self.fixing_end
    }
}

/// The curve's simple forward fixing over the future's window.
#[must_use]
pub fn stir_forward_rate(curve: &Curve, future: &StirFuture) -> Rate {
    curve.forward_rate_simple(future.fixing_start(), future.fixing_end())
}

/// The deterministic convexity adjustment `½ · σ² · T1 · T2` (one-factor Gaussian short rate).
///
/// Non-negative; zero when `vol == 0`; increasing in `vol` and in time-to-fixing.
#[must_use]
pub fn convexity_adjustment(future: &StirFuture, vol: f64) -> f64 {
    0.5 * vol * vol * future.fixing_start().0 * future.fixing_end().0
}

/// The futures-implied rate: forward fixing plus the convexity adjustment.
#[must_use]
pub fn stir_futures_rate(curve: &Curve, future: &StirFuture, vol: f64) -> Rate {
    Rate(stir_forward_rate(curve, future).0 + convexity_adjustment(future, vol))
}

/// The quoted futures price `100 · (1 − futures_rate)`.
#[must_use]
pub fn stir_futures_price(curve: &Curve, future: &StirFuture, vol: f64) -> f64 {
    100.0 * (1.0 - stir_futures_rate(curve, future, vol).0)
}

/// The conversion factor: the deliverable's price per unit face at the contract's notional yield.
///
/// Equals 1 when the deliverable's coupon equals the notional yield (the par-bond identity).
#[must_use]
pub fn conversion_factor(deliverable: &CashBond, notional_yield: Rate) -> f64 {
    price_at_yield(deliverable, notional_yield) / deliverable.redemption()
}

/// One basket entry for a bond future: its observed clean price and conversion factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Deliverable {
    /// The deliverable's observed clean price (per unit face, same scale as the futures price).
    pub clean_price: f64,
    /// The deliverable's conversion factor against the contract's notional yield.
    pub conversion_factor: f64,
}

/// The gross basis `clean_price − futures_price · conversion_factor`.
///
/// Zero when the deliverable trades exactly at its converted futures value.
#[must_use]
pub fn gross_basis(deliverable: &Deliverable, futures_price: f64) -> f64 {
    deliverable.clean_price - futures_price * deliverable.conversion_factor
}

/// The annualised implied repo rate of delivering `deliverable` into the future.
///
/// Buy the bond at its clean price today, deliver at the invoice `futures_price · conversion_factor`
/// after `years_to_delivery`; the implied repo is the annualised return of that carry (spot-settled,
/// no intervening coupon — see the module deferral note).
#[must_use]
pub fn implied_repo_rate(
    deliverable: &Deliverable,
    futures_price: f64,
    years_to_delivery: f64,
) -> f64 {
    let invoice = futures_price * deliverable.conversion_factor;
    (invoice - deliverable.clean_price) / deliverable.clean_price / years_to_delivery
}

/// The index of the cheapest-to-deliver basket entry: the one with the **highest implied repo rate**.
///
/// Returns `None` for an empty basket.
#[must_use]
pub fn cheapest_to_deliver(
    basket: &[Deliverable],
    futures_price: f64,
    years_to_delivery: f64,
) -> Option<usize> {
    basket
        .iter()
        .enumerate()
        .map(|(i, d)| (i, implied_repo_rate(d, futures_price, years_to_delivery)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::{OisQuote, bootstrap_ois};
    use crate::ois::{FixedPeriod, OisSchedule};
    use crate::vanilla_swap::{LegPeriod, PaymentFrequency, SwapLeg};

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

    fn curve() -> Curve {
        bootstrap_ois(&[quote(1, 0.0432), quote(2, 0.0418), quote(5, 0.0405)]).expect("bootstraps")
    }

    fn future() -> StirFuture {
        StirFuture::new(Time(1.0), Time(1.25)).expect("future")
    }

    #[test]
    fn zero_vol_future_prices_off_the_forward() {
        let c = curve();
        let f = future();
        assert!(convexity_adjustment(&f, 0.0).abs() < 1e-15);
        assert!((stir_futures_rate(&c, &f, 0.0).0 - stir_forward_rate(&c, &f).0).abs() < 1e-15);
    }

    #[test]
    fn convexity_is_positive_and_increases_with_vol() {
        let f = future();
        let lo = convexity_adjustment(&f, 0.008);
        let hi = convexity_adjustment(&f, 0.012);
        assert!(lo > 0.0 && hi > lo, "lo {lo} hi {hi}");
        // The futures rate sits above the forward by exactly the convexity adjustment.
        let c = curve();
        let raised = stir_futures_rate(&c, &f, 0.012).0 - stir_forward_rate(&c, &f).0;
        assert!((raised - hi).abs() < 1e-15);
    }

    #[test]
    fn futures_price_is_hundred_minus_rate() {
        let c = curve();
        let f = future();
        let rate = stir_futures_rate(&c, &f, 0.01).0;
        assert!((stir_futures_price(&c, &f, 0.01) - 100.0 * (1.0 - rate)).abs() < 1e-12);
    }

    #[test]
    fn rejects_degenerate_fixing_window() {
        assert_eq!(
            StirFuture::new(Time(1.0), Time(1.0)).unwrap_err(),
            FutureError::InvalidFixingWindow
        );
        assert_eq!(
            StirFuture::new(Time(-0.1), Time(0.5)).unwrap_err(),
            FutureError::InvalidFixingWindow
        );
    }

    #[test]
    fn conversion_factor_is_one_at_the_notional_coupon() {
        // The par-bond identity (CF == 1 when coupon == notional yield) is exact only when each
        // period's accrual is 1/f and pay times are k/f. Build that idealised semi-annual schedule
        // directly so the identity holds to machine precision; real ACT/360+roll schedules deviate
        // by the day-count/roll basis (a CF a little off 1), which is correct, not a bug.
        let notional = Rate(0.06);
        let periods: Vec<LegPeriod> = (1..=14)
            .map(|k| LegPeriod {
                accrual_start: Time(f64::from(k - 1) * 0.5),
                pay: Time(f64::from(k) * 0.5),
                accrual: Time(0.5),
            })
            .collect();
        let leg = SwapLeg::new(Time(0.0), periods).expect("idealised leg");
        let b = CashBond::new(leg, PaymentFrequency::SemiAnnual, notional, 100.0).expect("bond");
        assert!((conversion_factor(&b, notional) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn gross_basis_vanishes_at_the_converted_price() {
        let d = Deliverable {
            clean_price: 98.0,
            conversion_factor: 0.9,
        };
        let futures_price = 98.0 / 0.9;
        assert!(gross_basis(&d, futures_price).abs() < 1e-12);
    }

    #[test]
    fn implied_repo_is_positive_when_invoice_exceeds_cost() {
        let d = Deliverable {
            clean_price: 99.0,
            conversion_factor: 1.0,
        };
        // Invoice 100 > cost 99 over a quarter ⇒ positive annualised repo.
        let irr = implied_repo_rate(&d, 100.0, 0.25);
        assert!(irr > 0.0, "irr {irr}");
        assert!((irr - (1.0 / 99.0) / 0.25).abs() < 1e-12);
    }

    #[test]
    fn cheapest_to_deliver_picks_the_highest_implied_repo() {
        let basket = [
            Deliverable {
                clean_price: 99.5,
                conversion_factor: 1.0,
            },
            Deliverable {
                // Cheaper relative to its converted invoice ⇒ higher implied repo ⇒ the CTD.
                clean_price: 98.0,
                conversion_factor: 1.0,
            },
        ];
        assert_eq!(cheapest_to_deliver(&basket, 100.0, 0.25), Some(1));
        assert_eq!(cheapest_to_deliver(&[], 100.0, 0.25), None);
    }
}
