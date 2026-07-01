//! Bond pricing — accrued interest, dirty/clean price from a flat yield, and price from a curve.
//!
//! The fractional between-coupon period is handled explicitly: a cashflow `k` whole periods after
//! the next coupon (which is itself `w` of a period away, `w ∈ (0, 1]`) is discounted by
//! `(1 + y/f)^(−(w + k))`. Ignoring `w` — pricing as if settlement were the last coupon date — is the
//! classic textbook error this module deliberately avoids (handled in the cashflow-schedule builder).

use celnet_rates::Curve;
use celnet_types::Rate;

use crate::bond::{Bond, BondError};
use crate::schedule::CashflowSchedule;

/// Accrued interest from the last coupon date to settlement: `coupon_rate · redemption · τ`, where
/// `τ` is the day-count fraction from the previous coupon to settlement on the bond's accrual basis.
///
/// Zero when settlement falls on a coupon date.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn accrued_interest(bond: &Bond) -> Result<f64, BondError> {
    Ok(CashflowSchedule::from_bond(bond)?.accrued())
}

/// The dirty (full, invoice) price at a flat periodically-compounded `yield_`, compounded at the
/// coupon frequency: `Σ CFₖ · (1 + y/f)^(−(w + k − 1))` including the redemption in the final flow.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn dirty_price(bond: &Bond, yield_: Rate) -> Result<f64, BondError> {
    Ok(CashflowSchedule::from_bond(bond)?.dirty_price_at_yield(yield_.0))
}

/// The clean (quoted) price at a flat `yield_`: `dirty − accrued`.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn clean_price(bond: &Bond, yield_: Rate) -> Result<f64, BondError> {
    let s = CashflowSchedule::from_bond(bond)?;
    Ok(s.dirty_price_at_yield(yield_.0) - s.accrued())
}

/// The dirty price off a discount curve: `Σ CFₖ · DF(tₖ)`, discounting each cashflow at the curve's
/// ACT/365F year-fraction time from settlement (the curve reference date).
///
/// The curve carries no credit spread here; discounting off an issuer/spread curve is the sibling
/// `celnet-credit` leaf's job. Together with [`clean_price`], the clean curve price is
/// `price_from_curve − accrued_interest`.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn price_from_curve(bond: &Bond, curve: &Curve) -> Result<f64, BondError> {
    Ok(CashflowSchedule::from_bond(bond)?.price_on_curve(curve))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_rates::{AccrualBasis, PaymentFrequency};
    use celnet_types::{Df, Time};
    use time::{Date, Month};

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).expect("valid date")
    }

    /// A 3y 6% semi-annual 30/360 bond settling on a coupon date (the clean textbook case).
    fn bond_6pct_3y() -> Bond {
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

    /// Closed-form price of a level-coupon bond priced on a coupon date, via the annuity identity
    /// `P = c · (1 − v^N)/i + F · v^N`, `v = 1/(1 + i)`, `i = y/f`, `c = coupon_rate/f · F`. This is
    /// an INDEPENDENT formula (geometric-series closed form) from the engine's explicit per-cashflow
    /// summation, so agreement validates the engine rather than restating it.
    fn annuity_price(coupon_rate: f64, face: f64, freq: f64, periods: u32, y: f64) -> f64 {
        let i = y / freq;
        let v = 1.0 / (1.0 + i);
        let vn = v.powi(periods as i32);
        let c = coupon_rate / freq * face;
        c * (1.0 - vn) / i + face * vn
    }

    #[test]
    fn dirty_matches_closed_form_annuity() {
        // 6% semi, 3y (6 periods), priced to yield 8%.
        let bond = bond_6pct_3y();
        let engine = dirty_price(&bond, Rate(0.08)).expect("price");
        let closed = annuity_price(0.06, 100.0, 2.0, 6, 0.08);
        assert!(
            (engine - closed).abs() < 1e-9,
            "engine {engine} vs closed-form {closed}"
        );
    }

    #[test]
    fn matches_published_street_value() {
        // Published street value: a 6% semi-annual bond, 3 years to maturity, yielding 8% trades at
        // ≈ 94.76 (standard textbook coupon-bond example; the 8% discount rate on a 6% coupon puts it
        // below par). We validate the engine to that published precision independently of the
        // closed-form check above.
        let bond = bond_6pct_3y();
        let price = dirty_price(&bond, Rate(0.08)).expect("price");
        assert!((price - 94.76).abs() < 5e-3, "price {price}");
    }

    #[test]
    fn par_bond_prices_at_redemption_on_coupon_date() {
        // Coupon == yield on a coupon date ⇒ price == redemption exactly (par identity).
        let bond = bond_6pct_3y();
        let price = dirty_price(&bond, Rate(0.06)).expect("price");
        assert!((price - 100.0).abs() < 1e-10, "par price {price}");
    }

    #[test]
    fn premium_and_discount_bracket_par() {
        let bond = bond_6pct_3y();
        // Yield below coupon ⇒ premium; above ⇒ discount.
        assert!(dirty_price(&bond, Rate(0.04)).expect("p") > 100.0);
        assert!(dirty_price(&bond, Rate(0.08)).expect("p") < 100.0);
    }

    #[test]
    fn clean_equals_dirty_minus_accrued() {
        // Mid-period settlement: clean and dirty differ by exactly the accrued interest.
        let bond = Bond::new(
            d(2032, Month::September, 15),
            d(2035, Month::June, 15),
            0.06,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .expect("valid bond");
        let dirty = dirty_price(&bond, Rate(0.05)).expect("dirty");
        let clean = clean_price(&bond, Rate(0.05)).expect("clean");
        let accrued = accrued_interest(&bond).expect("accrued");
        assert!(accrued > 0.0, "expected positive accrued, got {accrued}");
        assert!((dirty - clean - accrued).abs() < 1e-12);
    }

    #[test]
    fn curve_price_equals_yield_price_when_curve_encodes_the_yield() {
        // Build a discount curve whose DFs at the bond's own cashflow times are exactly the flat-yield
        // discount factors. Then `price_from_curve` must reproduce `dirty_price` to machine precision:
        // this validates the curve-pricing machinery exactly (independently of any flat-curve
        // approximation), since the curve reproduces its pillar DFs.
        let bond = bond_6pct_3y();
        let y = 0.047_f64;
        let s = CashflowSchedule::from_bond(&bond).expect("schedule");
        let base = 1.0 + y / 2.0;
        let mut pillars: Vec<(Time, Df)> = vec![(Time(0.0), Df(1.0))];
        for (t, e) in s.curve_times().into_iter().zip(s.period_exponents()) {
            pillars.push((Time(t), Df(base.powf(-e))));
        }
        let curve = Curve::from_log_linear_dfs(&pillars).expect("valid curve");
        let curve_price = price_from_curve(&bond, &curve).expect("curve price");
        let yield_price = dirty_price(&bond, Rate(y)).expect("yield price");
        assert!(
            (curve_price - yield_price).abs() < 1e-9,
            "curve {curve_price} vs yield {yield_price}"
        );
    }

    #[test]
    fn flat_curve_matches_equivalent_periodic_yield() {
        // The task's curve-consistency check: on a genuinely FLAT continuously-compounded curve at
        // rate z, the curve price ≈ the flat-yield dirty price at the equivalent periodic yield
        // y = f·(e^{z/f} − 1). The residual is the day-count basis difference (curve time is ACT/365F
        // from settlement, ≠ the idealised k/f coupon spacing), so this is an approximate identity.
        let bond = bond_6pct_3y();
        let z = 0.05_f64; // continuously compounded, flat
        let curve = Curve::from_zero_rates(&[
            (Time(1.0), Rate(z)),
            (Time(2.0), Rate(z)),
            (Time(3.0), Rate(z)),
        ])
        .expect("flat curve");
        let curve_price = price_from_curve(&bond, &curve).expect("curve price");
        let y_equiv = 2.0 * ((z / 2.0).exp() - 1.0);
        let yield_price = dirty_price(&bond, Rate(y_equiv)).expect("yield price");
        assert!(
            (curve_price - yield_price).abs() < 5e-2,
            "curve {curve_price} vs equiv-yield {yield_price} (day-count residual)"
        );
    }

    #[test]
    fn zero_coupon_prices_as_a_single_discounted_redemption() {
        // A zero-coupon bond on a coupon date: price = 100 · (1 + y/f)^(−N).
        let bond = Bond::new(
            d(2030, Month::June, 15),
            d(2035, Month::June, 15),
            0.0,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .expect("valid bond");
        let price = dirty_price(&bond, Rate(0.05)).expect("price");
        let expected = 100.0 * (1.0 + 0.05 / 2.0f64).powi(-10);
        assert!((price - expected).abs() < 1e-10, "zero price {price}");
    }
}
