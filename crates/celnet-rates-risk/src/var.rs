//! Scenario P&L aggregation into Value-at-Risk and Expected Shortfall.
//!
//! The pipeline is a scenario/historical **bump-and-revalue** VaR: reprice the FI book on the base
//! curve and on every shocked curve, take each scenario's portfolio P&L (`shocked − base`), and
//! reduce the P&L distribution to VaR/ES at a confidence level. This is genuinely non-additive —
//! the book VaR reflects offsets across positions and is *not* the sum of per-position VaRs.

use crate::curve_shock::{RatePillars, RateShock};
use crate::error::RateRiskError;
use crate::position::FiPosition;
use celnet_rates::Curve;

/// A Value-at-Risk / Expected-Shortfall pair, both non-negative loss magnitudes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateVarEs {
    /// Value-at-Risk: the `alpha`-quantile loss (a positive number is a loss).
    pub var: f64,
    /// Expected Shortfall: the mean loss in the tail at or beyond the VaR quantile.
    pub es: f64,
}

/// The full rate-scenario risk result for a book: the base PV, the scenario-ordered P&L
/// distribution, and its VaR/ES reduction.
///
/// `scenario_pnl` is **row-aligned with the input shock set** (not sorted), so Phase C2c can fold
/// it into the cube's cross-risk-factor non-additive P&L per scenario; the VaR/ES is reduced from a
/// sorted copy, leaving `scenario_pnl` in scenario order.
#[derive(Clone, Debug, PartialEq)]
pub struct RateRiskReport {
    /// The book present value on the base (un-shocked) curve.
    pub base_pv: f64,
    /// Per-scenario portfolio P&L (`shocked_pv − base_pv`), aligned with the input shocks.
    pub scenario_pnl: Vec<f64>,
    /// The VaR / Expected-Shortfall reduction of `scenario_pnl` at the requested level.
    pub var_es: RateVarEs,
}

/// The shared VaR/ES tail reduction over a P&L slice (a loss is a negative P&L).
///
/// Sorts ascending, takes the `alpha`-tail boundary loss as VaR and the mean of the tail losses as
/// ES; both are floored at zero. The tail count is `⌊(1 − alpha)·n⌋`, floored at one and capped at
/// `n`, so at least the single worst scenario always contributes.
///
/// This reduction is intentionally **byte-identical in convention** to `celnet-risk-cube`'s private
/// `quantile_var_es` (same sort, same tail index `⌊(1−α)n⌋.max(1).min(n)`, same VaR/ES sign and
/// zero-floor), so Phase C2c can unify the two into a single shared primitive when it wires FI into
/// the cube. It is re-homed here (not imported) only because the cube's function is not public and
/// C2a must not modify the cube's non-additive path.
#[must_use]
pub fn rate_var_es(pnl: &mut [f64], alpha: f64) -> RateVarEs {
    if pnl.is_empty() {
        return RateVarEs { var: 0.0, es: 0.0 };
    }
    pnl.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = pnl.len();
    let tail = (((1.0 - alpha) * n as f64).floor() as usize).max(1).min(n);
    let tail_sum: f64 = pnl[..tail].iter().sum();
    let es = -(tail_sum / tail as f64);
    let var = -pnl[tail - 1];
    RateVarEs {
        var: var.max(0.0),
        es: es.max(0.0),
    }
}

/// The book present value on `curve`, summed across positions through the wrapped pricing.
fn portfolio_pv(positions: &[FiPosition], curve: &Curve) -> Result<f64, RateRiskError> {
    let mut pv = 0.0;
    for p in positions {
        pv += p.pv_on_curve(curve)?;
    }
    Ok(pv)
}

/// The per-scenario portfolio P&L distribution (`shocked_pv − base_pv`), row-aligned with `shocks`.
///
/// # Errors
///
/// Propagates [`RateRiskError`] from building any shocked curve or repricing any position.
pub fn scenario_pnls(
    positions: &[FiPosition],
    base: &RatePillars,
    shocks: &[RateShock],
) -> Result<Vec<f64>, RateRiskError> {
    let base_pv = portfolio_pv(positions, base.base_curve())?;
    let mut pnl = Vec::with_capacity(shocks.len());
    for shock in shocks {
        let shocked = base.shocked_curve(shock)?;
        pnl.push(portfolio_pv(positions, &shocked)? - base_pv);
    }
    Ok(pnl)
}

/// End-to-end rate-scenario VaR/ES for a book: reprice under every shock, then reduce.
///
/// Returns the [`RateRiskReport`] (base PV, scenario-ordered P&L, and the VaR/ES at `alpha`). The
/// P&L vector is preserved in scenario order for downstream combination; the VaR/ES is reduced from
/// a sorted copy. Deterministic for a fixed `(positions, base, shocks, alpha)` (no RNG; the only
/// transcendental is the curve's `exp`, via `libm`-consistent `f64`).
///
/// # Errors
///
/// Propagates [`RateRiskError`] from building any shocked curve or repricing any position.
pub fn rate_scenario_var_es(
    positions: &[FiPosition],
    base: &RatePillars,
    shocks: &[RateShock],
    alpha: f64,
) -> Result<RateRiskReport, RateRiskError> {
    let scenario_pnl = scenario_pnls(positions, base, shocks)?;
    let mut sorted = scenario_pnl.clone();
    let var_es = rate_var_es(&mut sorted, alpha);
    let base_pv = portfolio_pv(positions, base.base_curve())?;
    Ok(RateRiskReport {
        base_pv,
        scenario_pnl,
        var_es,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pnl_is_zero() {
        assert_eq!(rate_var_es(&mut [], 0.99), RateVarEs { var: 0.0, es: 0.0 });
    }

    #[test]
    fn var_es_on_a_known_distribution() {
        // A hand-specified P&L set (losses negative). n = 8, alpha = 0.75 ⇒ (1−α)=0.25 is exactly
        // representable, so tail = ⌊0.25·8⌋ = 2 with no float-boundary ambiguity. The two worst are
        // −100 and −80: VaR = 80 (the tail-boundary loss, the 2nd-worst), ES = mean(100, 80) = 90.
        let mut pnl = vec![-100.0, 20.0, -30.0, 10.0, -80.0, 5.0, 40.0, -60.0];
        let got = rate_var_es(&mut pnl, 0.75);
        assert!((got.var - 80.0).abs() < 1e-12, "var {}", got.var);
        assert!((got.es - 90.0).abs() < 1e-12, "es {}", got.es);
    }

    #[test]
    fn var_es_single_tail_element() {
        // alpha = 0.875 ⇒ (1−α)=0.125 exact, n = 8 ⇒ tail = ⌊1.0⌋ = 1: VaR = ES = the single worst.
        let mut pnl = vec![-100.0, 20.0, -30.0, 10.0, -80.0, 5.0, 40.0, -60.0];
        let got = rate_var_es(&mut pnl, 0.875);
        assert!((got.var - 100.0).abs() < 1e-12);
        assert!((got.es - 100.0).abs() < 1e-12);
    }

    #[test]
    fn all_gains_floors_to_zero() {
        // No losses in the tail ⇒ VaR/ES floored at zero (a profitable tail is not a loss).
        let mut pnl = vec![10.0, 20.0, 5.0, 40.0];
        let got = rate_var_es(&mut pnl, 0.75);
        assert_eq!(got, RateVarEs { var: 0.0, es: 0.0 });
    }
}
