//! Bond yield risk — DV01, Macaulay and modified duration, and convexity.
//!
//! All four come from the same analytic price derivatives (`celnet-bond` never bump-and-reprices for
//! these first- and second-order sensitivities, which are closed-form):
//!
//! ```text
//! P    = dirty_price(y)
//! P'   = ∂P/∂y  = −(1/f) Σ eₖ CFₖ (1 + y/f)^(−eₖ−1)          (strictly negative)
//! P''  = ∂²P/∂y² = (1/f²) Σ eₖ(eₖ+1) CFₖ (1 + y/f)^(−eₖ−2)
//!
//! modified duration D_mod = −P'/P                             (years)
//! Macaulay duration D_mac = D_mod · (1 + y/f)                 (years)
//! convexity         C     =  P''/P                            (years²)
//! DV01                    = −P' · 1bp                         (price move per +1bp of yield)
//! ```
//!
//! DV01 is quoted per the bond's own `redemption` face (so a `redemption = 100` bond gives DV01 per
//! 100 face); scale by position size externally.

use celnet_types::Rate;

use crate::bond::{Bond, BondError};
use crate::schedule::CashflowSchedule;

/// One basis point, the DV01 yield shift.
const ONE_BP: f64 = 1e-4;

/// DV01: the dirty-price change for a `+1bp` parallel shift in the yield, `−∂P/∂y · 1bp` (positive).
///
/// Analytic (first derivative), not bump-and-reprice; equal to `modified_duration · P · 1bp`.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn dv01(bond: &Bond, yield_: Rate) -> Result<f64, BondError> {
    let s = CashflowSchedule::from_bond(bond)?;
    Ok(-s.dirty_price_first_derivative(yield_.0) * ONE_BP)
}

/// Modified duration (years): `−(1/P) · ∂P/∂y`, the fractional dirty-price sensitivity per unit yield.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn modified_duration(bond: &Bond, yield_: Rate) -> Result<f64, BondError> {
    let s = CashflowSchedule::from_bond(bond)?;
    let price = s.dirty_price_at_yield(yield_.0);
    Ok(-s.dirty_price_first_derivative(yield_.0) / price)
}

/// Macaulay duration (years): the cashflow-time-weighted average, `modified_duration · (1 + y/f)`.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn macaulay_duration(bond: &Bond, yield_: Rate) -> Result<f64, BondError> {
    let s = CashflowSchedule::from_bond(bond)?;
    let price = s.dirty_price_at_yield(yield_.0);
    let modified = -s.dirty_price_first_derivative(yield_.0) / price;
    Ok(modified * (1.0 + yield_.0 / s.freq()))
}

/// Convexity (years²): `(1/P) · ∂²P/∂y²`, the second-order dirty-price sensitivity.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn convexity(bond: &Bond, yield_: Rate) -> Result<f64, BondError> {
    let s = CashflowSchedule::from_bond(bond)?;
    let price = s.dirty_price_at_yield(yield_.0);
    Ok(s.dirty_price_second_derivative(yield_.0) / price)
}

/// A one-shot risk report for a bond quoted at a market dirty price.
///
/// Produced by [`bond_risk`], which solves the yield to maturity once and derives every measure from
/// it, so the whole set is internally consistent (all off the same yield).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BondRisk {
    /// Yield to maturity implied by the market dirty price.
    pub yield_to_maturity: Rate,
    /// Dirty (full) price — equal to the input market price to solver tolerance.
    pub dirty_price: f64,
    /// Clean (quoted) price: `dirty − accrued`.
    pub clean_price: f64,
    /// Accrued interest from the last coupon to settlement.
    pub accrued_interest: f64,
    /// DV01: dirty-price move per `+1bp` of yield.
    pub dv01: f64,
    /// Macaulay duration (years).
    pub macaulay_duration: f64,
    /// Modified duration (years).
    pub modified_duration: f64,
    /// Convexity (years²).
    pub convexity: f64,
}

/// Solve the yield to maturity from `market_dirty_price` and report the full risk set off that yield.
///
/// # Errors
///
/// Propagates [`BondError`] from the yield solve or schedule construction (e.g.
/// [`BondError::NonPositivePrice`], [`BondError::YieldDidNotConverge`]).
pub fn bond_risk(bond: &Bond, market_dirty_price: f64) -> Result<BondRisk, BondError> {
    let ytm = crate::yield_solve::yield_to_maturity(bond, market_dirty_price)?;
    let s = CashflowSchedule::from_bond(bond)?;
    let y = ytm.0;
    let price = s.dirty_price_at_yield(y);
    let first = s.dirty_price_first_derivative(y);
    let second = s.dirty_price_second_derivative(y);
    let modified = -first / price;
    Ok(BondRisk {
        yield_to_maturity: ytm,
        dirty_price: price,
        clean_price: price - s.accrued(),
        accrued_interest: s.accrued(),
        dv01: -first * ONE_BP,
        macaulay_duration: modified * (1.0 + y / s.freq()),
        modified_duration: modified,
        convexity: second / price,
    })
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

    /// The 3y 6% semi-annual bond on a coupon date (yield 8%).
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

    /// High-accuracy central finite differences of the analytically-priced dirty price. This is an
    /// INDEPENDENT numerical method (numerical differentiation) versus the closed-form analytic
    /// derivatives the risk functions use, so agreement validates those formulas.
    fn fd_first_and_second(bond: &Bond, y: f64) -> (f64, f64) {
        let h = 1e-6;
        let p_up = dirty_price(bond, Rate(y + h)).expect("p+");
        let p_dn = dirty_price(bond, Rate(y - h)).expect("p-");
        let p_0 = dirty_price(bond, Rate(y)).expect("p0");
        let first = (p_up - p_dn) / (2.0 * h);
        let second = (p_up - 2.0 * p_0 + p_dn) / (h * h);
        (first, second)
    }

    #[test]
    fn dv01_matches_finite_difference() {
        let b = bond_6pct_3y();
        let y = 0.08;
        let (first_fd, _) = fd_first_and_second(&b, y);
        let expected = -first_fd * ONE_BP;
        let got = dv01(&b, Rate(y)).expect("dv01");
        assert!((got - expected).abs() < 1e-8, "dv01 {got} vs fd {expected}");
    }

    #[test]
    fn modified_duration_matches_finite_difference() {
        let b = bond_6pct_3y();
        let y = 0.08;
        let price = dirty_price(&b, Rate(y)).expect("price");
        let (first_fd, _) = fd_first_and_second(&b, y);
        let expected = -first_fd / price;
        let got = modified_duration(&b, Rate(y)).expect("dmod");
        assert!((got - expected).abs() < 1e-7, "dmod {got} vs fd {expected}");
    }

    #[test]
    fn convexity_matches_finite_difference() {
        let b = bond_6pct_3y();
        let y = 0.08;
        let price = dirty_price(&b, Rate(y)).expect("price");
        let (_, second_fd) = fd_first_and_second(&b, y);
        let expected = second_fd / price;
        let got = convexity(&b, Rate(y)).expect("cvx");
        // Second finite differences are noisier; a relative 1e-4 is ample and still catches formula
        // errors (which are O(1) wrong, not O(1e-4)).
        assert!(
            (got - expected).abs() < 1e-4 * expected.abs(),
            "convexity {got} vs fd {expected}"
        );
    }

    #[test]
    fn matches_hand_derived_duration_and_convexity() {
        // Independently hand-derived closed-form measures for the 6% semi-annual 3y bond at an 8%
        // yield (a standard textbook coupon-bond example; dirty price 94.75786):
        //
        //   Macaulay  D_mac = Σ (k/2)·CF_k·v^k / P                 = 2.78306 years   (v = 1/1.04)
        //   Modified  D_mod = D_mac / 1.04                          = 2.67602 years
        //   Convexity C     = (1/4)·Σ k(k+1)·CF_k·v^{k+2} / P       = 8.77787 years²
        //
        // These are derived from the weighted-cashflow definitions (an independent formula), not from
        // the engine's analytic derivatives, and agree with the published ≈2.78 / ≈2.68 street values.
        let b = bond_6pct_3y();
        let y = Rate(0.08);
        let dmac = macaulay_duration(&b, y).expect("dmac");
        let dmod = modified_duration(&b, y).expect("dmod");
        let cvx = convexity(&b, y).expect("cvx");
        assert!((dmac - 2.78306).abs() < 1e-4, "macaulay {dmac}");
        assert!((dmod - 2.67602).abs() < 1e-4, "modified {dmod}");
        assert!((cvx - 8.77787).abs() < 1e-3, "convexity {cvx}");
    }

    #[test]
    fn dv01_equals_modified_duration_times_price_times_bp() {
        let b = bond_6pct_3y();
        let y = Rate(0.055);
        let price = dirty_price(&b, y).expect("price");
        let dmod = modified_duration(&b, y).expect("dmod");
        let dv = dv01(&b, y).expect("dv01");
        assert!((dv - dmod * price * ONE_BP).abs() < 1e-12);
    }

    #[test]
    fn zero_coupon_macaulay_equals_maturity() {
        // A zero-coupon bond's Macaulay duration equals its time to maturity (years).
        let b = Bond::new(
            d(2032, Month::June, 15),
            d(2037, Month::June, 15),
            0.0,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .expect("valid bond");
        let dmac = macaulay_duration(&b, Rate(0.06)).expect("dmac");
        // 5 years, on a coupon date, integer exponents ⇒ exactly 5.0.
        assert!((dmac - 5.0).abs() < 1e-9, "macaulay {dmac}");
    }

    #[test]
    fn bond_risk_rolls_up_consistently() {
        let b = bond_6pct_3y();
        // Price the bond at a known yield, then confirm the roll-up recovers it and agrees with the
        // individual measures.
        let market = dirty_price(&b, Rate(0.072)).expect("price");
        let r = bond_risk(&b, market).expect("risk");
        assert!((r.yield_to_maturity.0 - 0.072).abs() < 1e-10);
        assert!((r.dirty_price - market).abs() < 1e-8);
        assert!((r.dv01 - dv01(&b, r.yield_to_maturity).expect("dv01")).abs() < 1e-12);
        assert!(
            (r.macaulay_duration - macaulay_duration(&b, r.yield_to_maturity).expect("dmac")).abs()
                < 1e-12
        );
        assert!((r.convexity - convexity(&b, r.yield_to_maturity).expect("cvx")).abs() < 1e-12);
        // On a coupon date there is no accrued, so clean == dirty.
        assert!(r.accrued_interest.abs() < 1e-12);
        assert!((r.clean_price - r.dirty_price).abs() < 1e-12);
    }
}
