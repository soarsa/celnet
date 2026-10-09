//! Cash-bond relative-value analytics — yield, Z-spread, G-spread, and asset-swap spread.
//!
//! A fixed-coupon cash bond reuses the conventionful coupon schedule from `vanilla_swap.rs` (a
//! [`SwapLeg`] of accrual periods) plus a redemption. Given the curve and an **observed market
//! price**, this module computes the desk's standard RV measures (`FI-ARCHITECTURE.md` §1,
//! cash-bond analytics):
//!
//! - **Yield to maturity** — the single periodically-compounded rate that discounts the bond's own
//!   cashflows to the quoted price (street convention, compounded at the coupon frequency).
//! - **Z-spread** — the constant continuously-compounded spread added to every curve zero rate
//!   (equivalently `DF(t) · e^{−z·t}`) that reprices the bond to the quoted price.
//! - **G-spread** — the yield pick-up over the curve: the bond's yield minus the yield of the
//!   *same cashflows* valued on the curve, so it is exactly zero when the bond trades curve-fair.
//! - **Asset-swap spread** — the par-par spread: the difference between the bond's curve PV and its
//!   price, amortised over the coupon-schedule annuity (single-curve par-par form).
//!
//! ## Verifying identities (self-contained — no external oracle)
//!
//! - A bond priced at redemption with coupons at rate `c` has YTM `= c` (par-bond identity).
//! - `price_at_yield(yield_to_maturity(price)) == price` (solver round-trip).
//! - Z-spread, G-spread, and asset-swap spread all vanish when `price == bond_pv(curve)`.
//! - A bond below its curve PV has a strictly positive Z-spread and asset-swap spread.
//!
//! ## Deliberately not in this slice (no stubs — coordinated follow-ups)
//!
//! - **Mid-period settlement / accrued interest** (clean vs dirty). The bond is valued
//!   spot-starting (settlement at the curve origin), so accrued interest is zero and price == PV of
//!   future cashflows. Non-zero accrued needs the settlement-date / day-count-since-last-coupon
//!   layer, tracked in `FI-STATUS.md`.
//! - **OAS** on callable bonds (this slice is option-free, where OAS ≡ Z-spread).
//!
//! Method/paper provenance lives in prose only — never in identifiers (GUIDE.md §8).

use crate::curve::Curve;
use crate::solver::{SolverError, brent_root};
use crate::vanilla_swap::{PaymentFrequency, SwapError, SwapLeg, swap_leg_schedule};
use celnet_types::{DayCount, Rate, Time};
use time::Date;

/// Lower yield/spread bracket for the analytics root-finds (−40% — keeps `1 + y/f > 0`).
const RATE_LO: f64 = -0.4;
/// Upper yield/spread bracket for the analytics root-finds (+200%, ample for distressed levels).
const RATE_HI: f64 = 2.0;
/// Abscissa tolerance for the analytics root-finds.
const RATE_TOL: f64 = 1e-13;
/// Iteration cap for the analytics root-finds.
const MAX_ITER: usize = 200;

/// A fixed-coupon cash bond: a coupon schedule, its compounding frequency, a coupon rate, and the
/// redemption (face) repaid at maturity.
#[derive(Clone, Debug)]
pub struct CashBond {
    coupons: SwapLeg,
    frequency: PaymentFrequency,
    coupon_rate: Rate,
    redemption: f64,
}

/// Failure modes of cash-bond construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BondError {
    /// The redemption (face) is not strictly positive.
    NonPositiveRedemption,
    /// The coupon schedule could not be built.
    Schedule(SwapError),
}

impl core::fmt::Display for BondError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonPositiveRedemption => {
                f.write_str("the bond redemption must be strictly positive")
            }
            Self::Schedule(e) => write!(f, "bond coupon schedule rejected: {e}"),
        }
    }
}

impl core::error::Error for BondError {}

impl CashBond {
    /// Assemble a bond from a prepared coupon schedule.
    ///
    /// # Errors
    ///
    /// Returns [`BondError::NonPositiveRedemption`] when `redemption <= 0` or is non-finite.
    pub fn new(
        coupons: SwapLeg,
        frequency: PaymentFrequency,
        coupon_rate: Rate,
        redemption: f64,
    ) -> Result<Self, BondError> {
        if redemption <= 0.0 || !redemption.is_finite() {
            return Err(BondError::NonPositiveRedemption);
        }
        Ok(Self {
            coupons,
            frequency,
            coupon_rate,
            redemption,
        })
    }

    /// The coupon schedule.
    #[must_use]
    pub fn coupons(&self) -> &SwapLeg {
        &self.coupons
    }

    /// The coupon rate.
    #[must_use]
    pub fn coupon_rate(&self) -> Rate {
        self.coupon_rate
    }

    /// The redemption (face) repaid at maturity.
    #[must_use]
    pub fn redemption(&self) -> f64 {
        self.redemption
    }

    /// The cash coupon paid for an accrual of `accrual`: `coupon_rate · accrual · redemption`.
    fn coupon(&self, accrual: f64) -> f64 {
        self.coupon_rate.0 * accrual * self.redemption
    }
}

/// Build a spot-starting fixed-coupon bond of `years` years at `freq`, accruing on `basis`.
///
/// The coupon schedule is generated exactly as a swap fixed leg ([`swap_leg_schedule`]); `freq`
/// also sets the yield compounding basis.
///
/// # Errors
///
/// Propagates [`SwapError`] from the schedule build (wrapped) or
/// [`BondError::NonPositiveRedemption`].
pub fn fixed_coupon_bond(
    reference: Date,
    years: u32,
    freq: PaymentFrequency,
    basis: DayCount,
    coupon_rate: Rate,
    redemption: f64,
) -> Result<CashBond, BondError> {
    let coupons = swap_leg_schedule(reference, years, freq, basis).map_err(BondError::Schedule)?;
    CashBond::new(coupons, freq, coupon_rate, redemption)
}

/// Present value of the bond's cashflows on `curve`: `Σ coupon_i · DF(pay_i) + redemption · DF(T)`.
#[must_use]
pub fn bond_pv(curve: &Curve, bond: &CashBond) -> f64 {
    let coupon_pv: f64 = bond
        .coupons()
        .periods()
        .iter()
        .map(|p| bond.coupon(p.accrual.0) * curve.discount_factor(p.pay).0)
        .sum();
    coupon_pv + bond.redemption * curve.discount_factor(bond.coupons().maturity()).0
}

/// Price the bond at a flat periodically-compounded `yield_`, compounded at the coupon frequency.
///
/// Cashflow at time `t` is discounted by `(1 + y/f)^(−f·t)`, where `f` is the coupons per year.
#[must_use]
pub fn price_at_yield(bond: &CashBond, yield_: Rate) -> f64 {
    let f = f64::from(bond.frequency.per_year());
    let base = 1.0 + yield_.0 / f;
    let discount = |t: f64| base.powf(-f * t);
    let coupon_pv: f64 = bond
        .coupons()
        .periods()
        .iter()
        .map(|p| bond.coupon(p.accrual.0) * discount(p.pay.0))
        .sum();
    coupon_pv + bond.redemption * discount(bond.coupons().maturity().0)
}

/// The yield to maturity that reprices the bond to `price`.
///
/// # Errors
///
/// Returns [`SolverError`] if the yield bracket does not straddle `price` or the solve fails to
/// converge.
pub fn yield_to_maturity(bond: &CashBond, price: f64) -> Result<Rate, SolverError> {
    let residual = |y: f64| price_at_yield(bond, Rate(y)) - price;
    brent_root(residual, RATE_LO, RATE_HI, RATE_TOL, MAX_ITER).map(Rate)
}

/// The Z-spread: the constant continuously-compounded spread `z` over the curve zero rates
/// (`DF(t) · e^{−z·t}`) that reprices the bond to `price`.
///
/// # Errors
///
/// Returns [`SolverError`] if the spread bracket does not straddle `price` or the solve fails.
pub fn z_spread(curve: &Curve, bond: &CashBond, price: f64) -> Result<f64, SolverError> {
    let priced = |z: f64| {
        let shifted = |t: f64| curve.discount_factor(Time(t)).0 * (-z * t).exp();
        let coupon_pv: f64 = bond
            .coupons()
            .periods()
            .iter()
            .map(|p| bond.coupon(p.accrual.0) * shifted(p.pay.0))
            .sum();
        coupon_pv + bond.redemption * shifted(bond.coupons().maturity().0)
    };
    brent_root(|z| priced(z) - price, RATE_LO, RATE_HI, RATE_TOL, MAX_ITER)
}

/// The G-spread: the bond's yield minus the yield of the same cashflows valued on the curve.
///
/// Zero exactly when the bond trades at its curve PV; positive when it yields above the curve.
///
/// # Errors
///
/// Returns [`SolverError`] if either underlying yield solve fails.
pub fn g_spread(curve: &Curve, bond: &CashBond, price: f64) -> Result<Rate, SolverError> {
    let bond_yield = yield_to_maturity(bond, price)?.0;
    let curve_yield = yield_to_maturity(bond, bond_pv(curve, bond))?.0;
    Ok(Rate(bond_yield - curve_yield))
}

/// The par-par asset-swap spread: `(bond_pv(curve) − price)` amortised over the coupon-schedule
/// annuity `Σ accrual_i · DF(pay_i)` (scaled by the redemption).
///
/// Single-curve par-par form: the asset-swap annuity is taken on the bond's own accrual schedule.
/// Zero when the bond trades at its curve PV; positive (the bond cheapens to the curve) when it
/// trades below.
#[must_use]
pub fn asset_swap_spread(curve: &Curve, bond: &CashBond, price: f64) -> f64 {
    let annuity: f64 = bond
        .coupons()
        .periods()
        .iter()
        .map(|p| p.accrual.0 * curve.discount_factor(p.pay).0)
        .sum();
    (bond_pv(curve, bond) - price) / (bond.redemption * annuity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::{OisQuote, bootstrap_ois};
    use crate::ois::{FixedPeriod, OisSchedule};
    use time::Month;

    const FACE: f64 = 100.0;

    fn spot() -> Date {
        Date::from_calendar_date(2025, Month::June, 16).expect("valid date")
    }

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
        bootstrap_ois(&[
            quote(1, 0.0432),
            quote(2, 0.0418),
            quote(5, 0.0405),
            quote(10, 0.0415),
        ])
        .expect("bootstraps")
    }

    fn bond(coupon: f64) -> CashBond {
        fixed_coupon_bond(
            spot(),
            5,
            PaymentFrequency::SemiAnnual,
            DayCount::Act360,
            Rate(coupon),
            FACE,
        )
        .expect("bond")
    }

    #[test]
    fn yield_recovers_the_pricing_rate() {
        // Price the bond at a known flat yield, then recover that yield — the par-bond/round-trip
        // identity in its sharpest form (no day-count ambiguity).
        let b = bond(0.05);
        let y = yield_to_maturity(&b, price_at_yield(&b, Rate(0.05))).expect("ytm");
        assert!((y.0 - 0.05).abs() < 1e-10, "ytm {y:?}");
    }

    #[test]
    fn yield_round_trips_through_price() {
        let b = bond(0.04);
        let price = 98.5;
        let y = yield_to_maturity(&b, price).expect("ytm");
        assert!((price_at_yield(&b, y) - price).abs() < 1e-9);
    }

    #[test]
    fn higher_price_means_lower_yield() {
        let b = bond(0.045);
        let y_cheap = yield_to_maturity(&b, 97.0).expect("ytm");
        let y_rich = yield_to_maturity(&b, 103.0).expect("ytm");
        assert!(y_rich.0 < y_cheap.0, "{y_rich:?} !< {y_cheap:?}");
    }

    #[test]
    fn z_spread_vanishes_on_curve() {
        let c = curve();
        let b = bond(0.045);
        let z = z_spread(&c, &b, bond_pv(&c, &b)).expect("z");
        assert!(z.abs() < 1e-10, "z {z}");
    }

    #[test]
    fn z_spread_reprices_the_bond() {
        let c = curve();
        let b = bond(0.045);
        let price = 96.0;
        let z = z_spread(&c, &b, price).expect("z");
        // Re-discount the cashflows at curve+z and confirm we land on the price.
        let shifted = |t: f64| c.discount_factor(Time(t)).0 * (-z * t).exp();
        let pv: f64 = b
            .coupons()
            .periods()
            .iter()
            .map(|p| b.coupon(p.accrual.0) * shifted(p.pay.0))
            .sum::<f64>()
            + b.redemption * shifted(b.coupons().maturity().0);
        assert!((pv - price).abs() < 1e-7, "pv {pv} price {price}");
        // A bond below its curve PV trades cheap: positive Z-spread.
        assert!(z > 0.0, "expected positive z, got {z}");
    }

    #[test]
    fn g_spread_vanishes_on_curve() {
        let c = curve();
        let b = bond(0.05);
        let g = g_spread(&c, &b, bond_pv(&c, &b)).expect("g");
        assert!(g.0.abs() < 1e-10, "g {g:?}");
    }

    #[test]
    fn asset_swap_spread_vanishes_on_curve_and_signs_correctly() {
        let c = curve();
        let b = bond(0.045);
        let pv = bond_pv(&c, &b);
        assert!(asset_swap_spread(&c, &b, pv).abs() < 1e-12);
        // Below curve PV ⇒ the bond cheapens to the curve ⇒ positive asset-swap spread.
        assert!(asset_swap_spread(&c, &b, pv - 2.0) > 0.0);
        assert!(asset_swap_spread(&c, &b, pv + 2.0) < 0.0);
    }

    #[test]
    fn rejects_non_positive_redemption() {
        let coupons = swap_leg_schedule(spot(), 3, PaymentFrequency::SemiAnnual, DayCount::Act360)
            .expect("schedule");
        assert_eq!(
            CashBond::new(coupons, PaymentFrequency::SemiAnnual, Rate(0.04), 0.0).unwrap_err(),
            BondError::NonPositiveRedemption
        );
    }
}
