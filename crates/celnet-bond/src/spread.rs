//! Credit-spread measures: the **z-spread** implied by a market price, and the CS01 /
//! spread duration that fall out of it.
//!
//! # Why the spread is solved, not stored
//!
//! A DV01-hedged corporate bond displays as flat while still carrying its full credit
//! exposure: the rates hedge removes `∂P/∂y`, and nothing removes `∂P/∂z`. Reporting that
//! exposure needs a credit spread per issue, and the platform has no reference-data field
//! carrying one — deliberately, because there is no permissibly-licensed source to populate
//! it from (guardrail 7) and a hand-entered figure is stale the moment it is typed.
//!
//! So the spread is **derived from the price the book already marks with**: the z-spread is
//! the single constant `z` which, added to every continuously-compounded zero rate on the
//! risk-free curve, reprices the bond to its observed market price. That makes CS01
//! self-consistent with the mark by construction, needs no new data, and moves with the
//! market on every tick.
//!
//! ```text
//! solve   P_market = Σ CFₖ · DF(tₖ) · e^(−z·tₖ)     for z
//! then    CS01     = −∂P/∂z · 1bp                    (analytic, positive)
//! ```
//!
//! # Method
//!
//! `P(z)` is strictly decreasing in `z` for a positive-cashflow bond, so `g(z) = P(z) −
//! market` has a single root. It is solved by the same **safeguarded Newton–Raphson** scheme
//! [`crate::yield_solve`] uses for yield — analytic derivative, bisection fallback whenever a
//! Newton step would leave the bracket or stall — which converges from any seed. CS01 and
//! spread duration are then analytic first derivatives, never bump-and-reprice, matching how
//! [`crate::risk::dv01`] is defined against yield.

use celnet_rates::Curve;
use celnet_types::Rate;

use crate::bond::{Bond, BondError};
use crate::schedule::CashflowSchedule;

/// One basis point — the CS01 spread shift.
const ONE_BP: f64 = 1e-4;
/// Iteration cap for the safeguarded Newton solve.
const MAX_ITER: usize = 100;
/// Convergence tolerance on the price residual (absolute, in price units).
const PRICE_TOL: f64 = 1e-12;
/// Convergence tolerance on the spread step.
const SPREAD_TOL: f64 = 1e-14;
/// Acceptance tolerance on the final residual (guards a stalled iteration reported as solved).
const RESIDUAL_ACCEPT: f64 = 1e-8;
/// The conventional bond price quote scale (per-100 face); a larger redemption face widens
/// the absolute price gates proportionally so the solve stays f64-representable.
const PRICE_QUOTE_REFERENCE: f64 = 100.0;
/// Lower bracket bound: a deeply NEGATIVE z-spread is meaningful (a bond richer than the
/// risk-free curve — on-the-run Treasuries routinely trade there), so the bracket must admit it.
const Z_MIN: f64 = -1.0;
/// The initial upper bracket bound (100bp), doubled outwards until it brackets the price.
/// Must be strictly positive — the expansion is multiplicative.
const Z_SEED: f64 = 0.01;
/// Cap on bracket expansion doublings when searching for an upper spread bound.
const MAX_EXPANSIONS: usize = 64;

/// The **z-spread** (continuously compounded, as a decimal — `0.0125` is 125bp) that reprices
/// `bond` off `curve` to `market_dirty_price`.
///
/// `market_dirty_price` is a **dirty/full** price on the same face scale as the bond's
/// redemption, matching [`crate::price::price_from_curve`]. Pass a clean market quote plus
/// [`crate::price::accrued_interest`].
///
/// # Errors
///
/// - [`BondError`] from schedule construction.
/// - [`BondError::NonPositiveSpreadPrice`] for a non-positive / non-finite target price.
/// - [`BondError::SpreadOutOfRange`] when no spread in `[Z_MIN, ∞)` brackets the price
///   (typically a price above the bond's undiscounted cashflow sum).
/// - [`BondError::SpreadDidNotConverge`] when the iteration exhausts its cap — never a
///   silently returned wrong root.
pub fn z_spread(bond: &Bond, curve: &Curve, market_dirty_price: f64) -> Result<Rate, BondError> {
    if !market_dirty_price.is_finite() || market_dirty_price <= 0.0 {
        return Err(BondError::NonPositiveSpreadPrice);
    }
    let s = CashflowSchedule::from_bond(bond)?;
    let price_at = |z: f64| s.price_on_curve_with_spread(curve, z);
    let residual_at = |z: f64| price_at(z) - market_dirty_price;

    // Scale the absolute tolerances to the quote scale actually in use.
    let scale = (market_dirty_price / PRICE_QUOTE_REFERENCE).max(1.0);
    let price_tol = PRICE_TOL * scale;
    let residual_accept = RESIDUAL_ACCEPT * scale;

    // Bracket: P is decreasing in z, so lo carries the HIGHER price.
    let mut lo = Z_MIN;
    let mut r_lo = residual_at(lo);
    if r_lo < 0.0 {
        // Even the richest admissible spread cannot reach this price.
        return Err(BondError::SpreadOutOfRange);
    }
    // Seed the upper bound STRICTLY POSITIVE: the expansion below doubles, and a seed of
    // zero doubles to zero forever — the bracket search would exhaust its cap and report a
    // reachable price as out of range.
    let mut hi = Z_SEED;
    let mut r_hi = residual_at(hi);
    let mut expansions = 0;
    while r_hi > 0.0 {
        expansions += 1;
        if expansions > MAX_EXPANSIONS {
            return Err(BondError::SpreadOutOfRange);
        }
        lo = hi;
        r_lo = r_hi;
        hi *= 2.0;
        r_hi = residual_at(hi);
    }

    // Safeguarded Newton: take the analytic step only while it stays in the bracket.
    let mut z = if r_lo.abs() < r_hi.abs() { lo } else { hi };
    for _ in 0..MAX_ITER {
        let r = residual_at(z);
        if r.abs() <= price_tol {
            return Ok(Rate(z));
        }
        if r > 0.0 {
            lo = z;
        } else {
            hi = z;
        }
        let d = s.curve_price_spread_derivative(curve, z);
        let step = if d.abs() > 0.0 { r / d } else { f64::NAN };
        let next = z - step;
        let z_next = if next.is_finite() && next > lo && next < hi {
            next
        } else {
            0.5 * (lo + hi)
        };
        if (z_next - z).abs() <= SPREAD_TOL {
            z = z_next;
            break;
        }
        z = z_next;
    }
    if residual_at(z).abs() <= residual_accept {
        Ok(Rate(z))
    } else {
        Err(BondError::SpreadDidNotConverge)
    }
}

/// **CS01**: the dirty-price change for a `+1bp` parallel shift in the credit spread,
/// `−∂P/∂z · 1bp` (positive for a positive-cashflow bond).
///
/// Analytic first derivative, not bump-and-reprice — the same definition
/// [`crate::risk::dv01`] uses against yield, so the two sensitivities are directly
/// comparable on one risk row.
///
/// # Errors
///
/// Propagates [`BondError`] from schedule construction.
pub fn cs01(bond: &Bond, curve: &Curve, z: Rate) -> Result<f64, BondError> {
    let s = CashflowSchedule::from_bond(bond)?;
    Ok(-s.curve_price_spread_derivative(curve, z.0) * ONE_BP)
}

/// **Spread duration** (years): `−(1/P) · ∂P/∂z`, the fractional dirty-price sensitivity per
/// unit of credit spread — the credit analogue of modified duration.
///
/// # Errors
///
/// - [`BondError`] from schedule construction.
/// - [`BondError::NonPositiveSpreadPrice`] when the spread-discounted price is not strictly
///   positive (nothing to divide by).
pub fn spread_duration(bond: &Bond, curve: &Curve, z: Rate) -> Result<f64, BondError> {
    let s = CashflowSchedule::from_bond(bond)?;
    let price = s.price_on_curve_with_spread(curve, z.0);
    if !(price.is_finite() && price > 0.0) {
        return Err(BondError::NonPositiveSpreadPrice);
    }
    Ok(-s.curve_price_spread_derivative(curve, z.0) / price)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::price::price_from_curve;
    use celnet_rates::{AccrualBasis, PaymentFrequency};
    use celnet_types::{Df, Time};
    use time::{Date, Month};

    fn d(y: i32, m: Month, day: u8) -> Date {
        Date::from_calendar_date(y, m, day).expect("valid date")
    }

    /// A 5y 4% semi-annual ACT/365F bond settling on a coupon date.
    fn bond_4pct_5y() -> Bond {
        Bond::new(
            d(2030, Month::June, 15),
            d(2035, Month::June, 15),
            0.04,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Act365Fixed,
            100.0,
        )
        .expect("valid bond")
    }

    /// A zero-coupon bond — the case with an exact closed-form CS01.
    fn zero_coupon_7y() -> Bond {
        Bond::new(
            d(2030, Month::June, 15),
            d(2037, Month::June, 15),
            0.0,
            PaymentFrequency::Annual,
            AccrualBasis::Act365Fixed,
            100.0,
        )
        .expect("valid bond")
    }

    /// A FLAT continuously-compounded discount curve at `r`: `DF(t) = e^(−r·t)`.
    ///
    /// Built from explicit discount factors rather than zero rates so the test does not
    /// depend on the curve's own rate→DF convention agreeing with the spread's.
    fn flat_curve(r: f64) -> celnet_rates::Curve {
        let pillars: Vec<(Time, Df)> = (0..=40)
            .map(|i| {
                let t = f64::from(i) * 0.5;
                (Time(t), Df((-r * t).exp()))
            })
            .collect();
        celnet_rates::Curve::from_log_linear_dfs(&pillars).expect("valid flat curve")
    }

    /// **The independent check.** On a flat curve, adding a constant continuous spread `z` to
    /// every zero rate is *definitionally* the same as discounting on a flat curve at `r + z`.
    ///
    /// So: price the bond off a flat `r + z` curve (never touching the spread code), then ask
    /// the solver for the spread that reprices it off the flat `r` curve. It must return `z`.
    /// This validates the solve against a construction, not against its own forward function.
    #[test]
    fn z_spread_recovers_the_shift_between_two_flat_curves() {
        let bond = bond_4pct_5y();
        let r = 0.035;
        for z_true in [0.0025, 0.0100, 0.0375, 0.0900] {
            let priced_on_shifted =
                price_from_curve(&bond, &flat_curve(r + z_true)).expect("shifted-curve price");
            let solved = z_spread(&bond, &flat_curve(r), priced_on_shifted).expect("solves");
            assert!(
                (solved.0 - z_true).abs() < 1e-10,
                "z_spread recovered {} for a true shift of {z_true}",
                solved.0
            );
        }
    }

    /// A bond trading RICH to the curve has a NEGATIVE z-spread — on-the-run Treasuries do
    /// this routinely, and a solver bracketed at zero would refuse the price outright.
    #[test]
    fn a_rich_bond_solves_to_a_negative_spread() {
        let bond = bond_4pct_5y();
        let r = 0.035;
        let rich = price_from_curve(&bond, &flat_curve(r - 0.0040)).expect("rich price");
        let solved = z_spread(&bond, &flat_curve(r), rich).expect("solves");
        assert!(
            (solved.0 + 0.0040).abs() < 1e-10,
            "expected −40bp, got {}",
            solved.0
        );
    }

    /// The curve price itself implies a zero spread — the degenerate identity.
    #[test]
    fn the_curve_price_implies_a_zero_spread() {
        let bond = bond_4pct_5y();
        let curve = flat_curve(0.035);
        let at_curve = price_from_curve(&bond, &curve).expect("curve price");
        let solved = z_spread(&bond, &curve, at_curve).expect("solves");
        assert!(solved.0.abs() < 1e-12, "expected 0, got {}", solved.0);
    }

    /// The solve works on a SLOPED curve too — the flat-curve identity above is a special
    /// case, so this pins that nothing in the solver assumes a flat term structure.
    #[test]
    fn z_spread_round_trips_on_a_sloped_curve() {
        let bond = bond_4pct_5y();
        // An upward-sloping curve: 2% at the front, rising to 5% at 20y.
        let pillars: Vec<(Time, Df)> = (0..=40)
            .map(|i| {
                let t = f64::from(i) * 0.5;
                let r = 0.02 + 0.03 * (t / 20.0);
                (Time(t), Df((-r * t).exp()))
            })
            .collect();
        let curve = celnet_rates::Curve::from_log_linear_dfs(&pillars).expect("sloped curve");
        let s = CashflowSchedule::from_bond(&bond).expect("schedule");
        for z_true in [-0.0020, 0.0015, 0.0250] {
            let target = s.price_on_curve_with_spread(&curve, z_true);
            let solved = z_spread(&bond, &curve, target).expect("solves");
            assert!(
                (solved.0 - z_true).abs() < 1e-10,
                "sloped-curve round trip: got {} for {z_true}",
                solved.0
            );
        }
    }

    /// **CS01 against a closed form.** For a zero-coupon bond the whole price is one flow at
    /// `T`, so `P(z) = F·DF(T)·e^(−zT)` and `−∂P/∂z = T·P` exactly. CS01 must therefore be
    /// `T · P · 1bp` — an independent formula, not the engine's own derivative.
    #[test]
    fn zero_coupon_cs01_matches_the_closed_form() {
        let bond = zero_coupon_7y();
        let curve = flat_curve(0.03);
        let z = Rate(0.0150);
        let s = CashflowSchedule::from_bond(&bond).expect("schedule");
        let price = s.price_on_curve_with_spread(&curve, z.0);
        // ACT/365F time from settlement to maturity.
        let days = (bond.maturity() - bond.settlement()).whole_days();
        let t = f64::from(i32::try_from(days).expect("a bond tenor fits i32 days")) / 365.0;
        let expected = t * price * 1e-4;
        let got = cs01(&bond, &curve, z).expect("cs01");
        assert!(
            (got - expected).abs() < 1e-12,
            "closed form {expected}, engine {got}"
        );
    }

    /// The analytic CS01 agrees with bump-and-reprice to the order of the bump — this is what
    /// makes the analytic derivative trustworthy rather than merely fast.
    #[test]
    fn analytic_cs01_agrees_with_bump_and_reprice() {
        let bond = bond_4pct_5y();
        let curve = flat_curve(0.035);
        let z = Rate(0.0125);
        let s = CashflowSchedule::from_bond(&bond).expect("schedule");
        // A CENTRAL difference, deliberately: the one-sided bump differs from the true
        // derivative by the O(h²·∂²P/∂z²) convexity term (~1e-5 here, i.e. 25% of a
        // hundredth of a basis point), which would force a tolerance loose enough to hide a
        // genuinely wrong derivative. The central difference cancels that term.
        let h = 1e-6;
        let central = (s.price_on_curve_with_spread(&curve, z.0 - h)
            - s.price_on_curve_with_spread(&curve, z.0 + h))
            / (2.0 * h)
            * 1e-4;
        let analytic = cs01(&bond, &curve, z).expect("cs01");
        assert!(
            (analytic - central).abs() < 1e-11,
            "analytic {analytic} vs central difference {central}"
        );
        assert!(
            analytic > 0.0,
            "CS01 is positive for a positive-cashflow bond"
        );
    }

    /// Spread duration is CS01 normalised by price — the two must agree by definition.
    #[test]
    fn spread_duration_and_cs01_are_consistent() {
        let bond = bond_4pct_5y();
        let curve = flat_curve(0.035);
        let z = Rate(0.0125);
        let s = CashflowSchedule::from_bond(&bond).expect("schedule");
        let price = s.price_on_curve_with_spread(&curve, z.0);
        let sd = spread_duration(&bond, &curve, z).expect("spread duration");
        let got = cs01(&bond, &curve, z).expect("cs01");
        assert!(
            (sd * price * 1e-4 - got).abs() < 1e-12,
            "duration·P·1bp {} vs cs01 {got}",
            sd * price * 1e-4
        );
        // A 5y 4% bond's spread duration sits just under its maturity.
        assert!((3.5..5.0).contains(&sd), "implausible spread duration {sd}");
    }

    /// A price no spread can reach is refused, not silently mis-solved. The undiscounted
    /// cashflow sum is the supremum of `P(z)` as `z → −∞`… but the bracket floor is `Z_MIN`,
    /// so anything above the price at `Z_MIN` is out of range.
    #[test]
    fn an_unreachable_price_is_refused() {
        let bond = bond_4pct_5y();
        let curve = flat_curve(0.035);
        let s = CashflowSchedule::from_bond(&bond).expect("schedule");
        let unreachable = s.price_on_curve_with_spread(&curve, Z_MIN) * 1.01;
        assert_eq!(
            z_spread(&bond, &curve, unreachable).unwrap_err(),
            BondError::SpreadOutOfRange
        );
    }

    /// A non-positive or non-finite target price is rejected up front.
    #[test]
    fn a_non_positive_price_is_rejected() {
        let bond = bond_4pct_5y();
        let curve = flat_curve(0.035);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                z_spread(&bond, &curve, bad).unwrap_err(),
                BondError::NonPositiveSpreadPrice,
                "price {bad} must be refused"
            );
        }
    }
}
