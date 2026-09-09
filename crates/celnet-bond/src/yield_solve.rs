//! Yield to maturity by safeguarded Newton–Raphson.
//!
//! The dirty price is strictly decreasing in yield, so `g(y) = dirty_price(y) − market` has a single
//! root. We solve it with Newton's method using the **analytic** price derivative, wrapped in a
//! bisection safeguard: each step takes the Newton update only if it stays inside the current
//! bracket and is reducing the residual fast enough, otherwise it bisects. This guarantees
//! convergence from any start — a poor seed cannot make the iterate diverge or leave the bracket
//! (the classic "Newton-with-bisection-fallback" scheme).

use celnet_types::Rate;

use crate::bond::{Bond, BondError};
use crate::schedule::CashflowSchedule;

/// Iteration cap for the safeguarded Newton solve.
const MAX_ITER: usize = 100;
/// Convergence tolerance on the price residual (absolute, in price units).
const PRICE_TOL: f64 = 1e-12;
/// Convergence tolerance on the yield step.
const YIELD_TOL: f64 = 1e-14;
/// Acceptance tolerance on the final residual (guards against a stalled iteration reported as solved).
const RESIDUAL_ACCEPT: f64 = 1e-8;
/// The conventional bond price quote scale (per-100 face). Prices at or below this
/// keep the exact absolute [`PRICE_TOL`] / [`RESIDUAL_ACCEPT`]; a larger redemption
/// face widens the price gates proportionally so the solve stays f64-representable.
const PRICE_QUOTE_REFERENCE: f64 = 100.0;
/// Cap on bracket expansion doublings when searching for an upper yield bound.
const MAX_EXPANSIONS: usize = 64;

/// The yield to maturity that reprices `bond` to `market_dirty_price` (a dirty/full price).
///
/// Solves `dirty_price(bond, y) = market_dirty_price` by safeguarded Newton–Raphson: the seed is the
/// current-yield estimate `annual_coupon / price`, and a bisection fallback keeps the iterate inside
/// a validated `[low, high]` bracket so a bad start cannot diverge. Converges to a price residual
/// below `1e-12`.
///
/// # Errors
///
/// - [`BondError::NonPositivePrice`] if `market_dirty_price` is non-positive or non-finite.
/// - [`BondError::YieldOutOfRange`] if no yield in the solvable range `(−f, 10⁶]` reprices the bond
///   to the target (e.g. an impossibly small target price).
/// - [`BondError::YieldDidNotConverge`] if the iteration cap is reached without acceptance.
/// - Propagates [`BondError`] from schedule construction.
pub fn yield_to_maturity(bond: &Bond, market_dirty_price: f64) -> Result<Rate, BondError> {
    if !market_dirty_price.is_finite() || market_dirty_price <= 0.0 {
        return Err(BondError::NonPositivePrice);
    }
    let schedule = CashflowSchedule::from_bond(bond)?;
    let freq = schedule.freq();

    // g(y) = price(y) − market is strictly decreasing; g(low_yield) > 0, g(high_yield) < 0.
    let residual = |y: f64| schedule.dirty_price_at_yield(y) - market_dirty_price;

    // The price residual `g` lives in the market's price units, so its acceptance
    // tolerances must be RELATIVE to that scale: a par-100 quote and a 25,000,000
    // face are the same trade at ~1e5× the price magnitude, and an absolute 1e-8
    // residual floor is unreachable once the price exceeds ~1e6 (f64 loses the last
    // digits). We scale `PRICE_TOL` / `RESIDUAL_ACCEPT` by `price / 100` floored at 1,
    // so a conventional per-100 quote (and anything below par) keeps the EXACT original
    // absolute precision — existing bond prices stay byte-identical — while a large
    // redemption face widens the gate just enough to converge. The yield-step break
    // (`YIELD_TOL`) is already scale-free, so this only touches the price gates.
    let price_scale = (market_dirty_price / PRICE_QUOTE_REFERENCE).max(1.0);
    let price_tol = PRICE_TOL * price_scale;
    let residual_accept = RESIDUAL_ACCEPT * price_scale;

    // Lower yield bound keeps 1 + y/f > 0; the price there is enormous, so g(low) > 0 for any
    // sensible target. If it is not, the target price is larger than the bond's maximum value.
    let low_yield = -freq + 1e-6;
    if residual(low_yield) <= 0.0 {
        return Err(BondError::YieldOutOfRange);
    }

    // Expand an upper bound until the residual turns negative (price falls below the target).
    let annual_coupon = bond.coupon_rate() * bond.redemption();
    let seed = (annual_coupon / market_dirty_price).clamp(low_yield + 1e-3, 1.0);
    let mut high_yield = seed.max(0.05) * 2.0;
    let mut expansions = 0;
    while residual(high_yield) > 0.0 {
        high_yield *= 2.0;
        expansions += 1;
        if expansions > MAX_EXPANSIONS || high_yield > 1e6 {
            return Err(BondError::YieldOutOfRange);
        }
    }

    // Orient the bracket so `neg` has g < 0 (high yield) and `pos` has g > 0 (low yield).
    let mut neg = high_yield;
    let mut pos = low_yield;
    let mut y = seed.clamp(low_yield, high_yield);
    let mut step_prev = (high_yield - low_yield).abs();
    let mut step = step_prev;
    let (p0, dg0) = schedule.dirty_price_and_first_derivative(y);
    let mut g = p0 - market_dirty_price;
    let mut dg = dg0;

    for _ in 0..MAX_ITER {
        // Bisect when the Newton iterate would leave the bracket, or is not halving the step.
        let newton_out_of_range = ((y - neg) * dg - g) * ((y - pos) * dg - g) > 0.0;
        let newton_too_slow = (2.0 * g).abs() > (step_prev * dg).abs();
        if newton_out_of_range || newton_too_slow {
            step_prev = step;
            step = 0.5 * (neg - pos);
            y = pos + step;
        } else {
            step_prev = step;
            step = g / dg; // Newton step (dg < 0)
            y -= step;
        }
        if step.abs() < YIELD_TOL {
            break;
        }
        let (p, deriv) = schedule.dirty_price_and_first_derivative(y);
        g = p - market_dirty_price;
        dg = deriv;
        // Maintain the bracket around the root.
        if g > 0.0 {
            pos = y;
        } else {
            neg = y;
        }
        if g.abs() < price_tol {
            break;
        }
    }

    if residual(y).abs() > residual_accept {
        return Err(BondError::YieldDidNotConverge);
    }
    Ok(Rate(y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::price::dirty_price;
    use celnet_rates::{AccrualBasis, PaymentFrequency};
    use time::{Date, Month};

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).expect("valid date")
    }

    fn bond(coupon: f64, years: i32, freq: PaymentFrequency, basis: AccrualBasis) -> Bond {
        Bond::new(
            d(2032, Month::June, 15),
            d(2032 + years, Month::June, 15),
            coupon,
            freq,
            basis,
            100.0,
        )
        .expect("valid bond")
    }

    #[test]
    fn round_trips_price_to_yield_to_price() {
        // The core identity: ytm(dirty_price(y)) == y across a range of bonds and yields.
        for &coupon in &[0.0, 0.025, 0.06, 0.09] {
            for &years in &[2, 5, 10] {
                for &y in &[0.01, 0.035, 0.05, 0.075, 0.12] {
                    let b = bond(
                        coupon,
                        years,
                        PaymentFrequency::SemiAnnual,
                        AccrualBasis::Thirty360BondBasis,
                    );
                    let price = dirty_price(&b, Rate(y)).expect("price");
                    let solved = yield_to_maturity(&b, price).expect("ytm");
                    assert!(
                        (solved.0 - y).abs() < 1e-10,
                        "coupon {coupon} years {years} y {y} → solved {}",
                        solved.0
                    );
                }
            }
        }
    }

    #[test]
    fn par_bond_yields_its_coupon() {
        // A bond priced at redemption on a coupon date has YTM == coupon (par identity).
        for &coupon in &[0.03, 0.05, 0.075] {
            let b = bond(
                coupon,
                7,
                PaymentFrequency::SemiAnnual,
                AccrualBasis::Thirty360BondBasis,
            );
            let y = yield_to_maturity(&b, 100.0).expect("ytm");
            assert!(
                (y.0 - coupon).abs() < 1e-10,
                "coupon {coupon} → ytm {}",
                y.0
            );
        }
    }

    #[test]
    fn converges_at_a_large_redemption_face() {
        // The price residual lives in price units, so its acceptance tolerance must be
        // relative to the price scale: a 25,000,000 face is the same trade as a par-100
        // quote at 1e5× the magnitude. With an absolute residual floor this solve
        // stalled (YieldDidNotConverge); with the price-scaled tolerance it converges to
        // the same yield the par-100 bond gives — the face only scales the price.
        for &face in &[100.0, 1_000_000.0, 25_000_000.0, 500_000_000.0] {
            let b = Bond::new(
                d(2032, Month::June, 15),
                d(2039, Month::June, 15),
                0.05,
                PaymentFrequency::SemiAnnual,
                AccrualBasis::Thirty360BondBasis,
                face,
            )
            .expect("valid bond");
            // Priced at par-for-its-face (redemption on a coupon date) ⇒ YTM == coupon.
            let y = yield_to_maturity(&b, face).expect("ytm at large face");
            assert!(
                (y.0 - 0.05).abs() < 1e-10,
                "face {face} → ytm {} (expected the 5% coupon)",
                y.0
            );
        }
    }

    #[test]
    fn converges_from_a_deliberately_bad_seed_region() {
        // A deep-discount, high-yield long bond stresses the safeguard: the current-yield seed is far
        // from the true yield, so the bisection fallback must engage without diverging.
        let b = bond(
            0.02,
            30,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
        );
        let price = dirty_price(&b, Rate(0.11)).expect("price");
        let y = yield_to_maturity(&b, price).expect("ytm");
        assert!((y.0 - 0.11).abs() < 1e-10, "ytm {}", y.0);
    }

    #[test]
    fn higher_price_gives_lower_yield() {
        let b = bond(
            0.045,
            5,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
        );
        let y_rich = yield_to_maturity(&b, 105.0).expect("ytm");
        let y_cheap = yield_to_maturity(&b, 95.0).expect("ytm");
        assert!(y_rich.0 < y_cheap.0, "{} !< {}", y_rich.0, y_cheap.0);
    }

    #[test]
    fn rejects_non_positive_target_price() {
        let b = bond(
            0.05,
            5,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
        );
        assert_eq!(
            yield_to_maturity(&b, 0.0).unwrap_err(),
            BondError::NonPositivePrice
        );
        assert_eq!(
            yield_to_maturity(&b, -5.0).unwrap_err(),
            BondError::NonPositivePrice
        );
    }

    #[test]
    fn rejects_unreachable_price() {
        // A vanishingly small target price would require a yield beyond the solvable cap (the price
        // stays above it for every yield up to 10⁶), so no bracketing yield exists.
        let b = bond(
            0.05,
            5,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
        );
        assert_eq!(
            yield_to_maturity(&b, 1.0e-9).unwrap_err(),
            BondError::YieldOutOfRange
        );
    }
}
