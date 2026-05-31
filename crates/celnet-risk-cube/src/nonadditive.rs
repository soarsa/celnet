//! Non-additive measures (`docs/RISK-HIERARCHY.md` §2.5).
//!
//! Some node-level measures **cannot be summed** from child results — they must be
//! **re-derived at each node** from the node's constituent positions:
//!
//! - **VaR / Expected Shortfall** — a non-linear function of the joint P&L
//!   distribution; the VaR of a sum is not the sum of VaRs (sub-additivity, and
//!   diversification, break naive summation).
//! - **FRTB-SbM curvature (CVR)** — the worst of an up-shock and down-shock full
//!   reprice of the node net of its linear (delta) approximation; curvature is a
//!   `max`/non-linear reduction, not a sum (`docs/RISK-HIERARCHY.md` §2.5, MAR21).
//! - **Correlation-weighted vega** — the `√(wᵀ ρ w)` SbM-style aggregation across
//!   vega buckets; a quadratic form in the bucketed vegas, not their sum.
//!
//! # Method: bump-and-revalue (honest about the scale path)
//!
//! Each measure here is computed by **bump-and-revalue**: shock a risk factor,
//! re-price every constituent position with `celnet-vanilla`'s closed forms, and
//! reduce. This is **correct** and is the natural reference implementation, but it
//! is **not** the fastest at investment-banking scale: it costs O(positions ×
//! scenarios) repricings per node. `docs/RISK-HIERARCHY.md` §3.3 names the known
//! scale path — **adjoint algorithmic differentiation (AAD) + batched-GPU**
//! (`celnet-gpu`) — as the throughput lever. That optimisation is **deliberately
//! deferred**, not faked: there is no stub adjoint here. The bump-and-revalue
//! result is the *oracle* the future AAD path will be validated against, so
//! building it first is the correct order. (This mirrors `celnet-journal`'s
//! honestly-deferred compaction.)
//!
//! All shocks are **relative or absolute parameter bumps** applied to
//! [`VanillaInputs`]; repricing is deterministic (`libm`), so every measure here
//! is bit-reproducible for a fixed scenario set.

use celnet_core::math::sqrt;
use celnet_risk_normalize::PositionRisk;
use celnet_types::VanillaInputs;
use celnet_vanilla::price;

/// One scenario: a set of multiplicative/additive shocks to the pricing inputs of
/// every constituent position, used to reprice the node under stress.
///
/// Shocks are expressed as the *change applied to each position's own inputs*, so
/// a single scenario means the same economic move (e.g. "spot +1 %, vol +1 vol")
/// applied consistently across a heterogeneous book. `spot_rel` multiplies spot;
/// `vol_abs` adds to vol (in absolute vol units, so 0.01 = +1 vol point);
/// `rate_dom_abs`/`rate_for_abs` add to the two rates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scenario {
    /// Relative spot shock (0.01 = +1 %). Applied as `spot *= 1 + spot_rel`.
    pub spot_rel: f64,
    /// Absolute vol shock in vol units (0.01 = +1 vol point). `vol += vol_abs`.
    pub vol_abs: f64,
    /// Absolute domestic-rate shock. `r_dom += rate_dom_abs`.
    pub rate_dom_abs: f64,
    /// Absolute foreign-rate shock. `r_for += rate_for_abs`.
    pub rate_for_abs: f64,
}

impl Scenario {
    /// The no-op (base) scenario.
    #[must_use]
    pub const fn base() -> Self {
        Self {
            spot_rel: 0.0,
            vol_abs: 0.0,
            rate_dom_abs: 0.0,
            rate_for_abs: 0.0,
        }
    }

    /// A pure relative-spot scenario.
    #[must_use]
    pub const fn spot(spot_rel: f64) -> Self {
        Self {
            spot_rel,
            vol_abs: 0.0,
            rate_dom_abs: 0.0,
            rate_for_abs: 0.0,
        }
    }

    /// A pure absolute-vol scenario.
    #[must_use]
    pub const fn vol(vol_abs: f64) -> Self {
        Self {
            spot_rel: 0.0,
            vol_abs,
            rate_dom_abs: 0.0,
            rate_for_abs: 0.0,
        }
    }

    /// Apply this scenario's shocks to a set of pricing inputs.
    #[must_use]
    pub fn apply(&self, i: &VanillaInputs) -> VanillaInputs {
        VanillaInputs::new(
            i.spot * (1.0 + self.spot_rel),
            i.strike,
            i.vol + self.vol_abs,
            i.t,
            i.r_dom + self.rate_dom_abs,
            i.r_for + self.rate_for_abs,
        )
    }
}

/// Re-price a single position under a scenario, returning its **P&L** vs base in
/// the position's quote (domestic) currency, scaled by notional and signed by the
/// long/short direction of the notional.
///
/// `price` returns the per-unit-base domestic PV; multiplying by `notional_base`
/// gives the position value, and the P&L is `value(shocked) − value(base)`.
#[must_use]
pub fn position_pnl(pos: &PositionRisk, scenario: Scenario) -> f64 {
    let base_v = price(pos.option, &pos.inputs) * pos.notional_base;
    let shocked = scenario.apply(&pos.inputs);
    let shocked_v = price(pos.option, &shocked) * pos.notional_base;
    shocked_v - base_v
}

/// The total P&L of a node (its constituent positions) under one scenario, in the
/// **common premium currency** assumption (all positions share a quote ccy) — see
/// the note on [`historical_var_es`] for the multi-currency caveat.
#[must_use]
pub fn node_pnl(positions: &[PositionRisk], scenario: Scenario) -> f64 {
    positions.iter().map(|p| position_pnl(p, scenario)).sum()
}

/// Historical-style **VaR** and **Expected Shortfall** of a node, by full
/// bump-and-revalue over a supplied set of historical scenarios
/// (`docs/RISK-HIERARCHY.md` §2.5).
///
/// For each scenario the whole node is repriced and its P&L recorded; VaR at level
/// `alpha` is the `alpha`-quantile **loss** (a positive number = a loss), and ES
/// is the mean loss in the tail beyond VaR. This is genuinely non-additive: the
/// node's VaR reflects diversification across its positions and is *not* the sum
/// of per-position VaRs — which is exactly why it is re-derived here from
/// constituents rather than rolled up.
///
/// Determinism: the scenario set is supplied by the caller (the cube wires it to a
/// historical-return window or an IPV-pinned scenario library); repricing is
/// `libm`-deterministic, so the result is reproducible for a fixed scenario set.
///
/// **Currency caveat (honest):** P&L is summed in each position's quote currency.
/// For a single-currency node this is exact; for a multi-currency node the caller
/// must pass positions already normalized to a common numeraire (the cube does
/// this via `celnet-risk-normalize` before calling), otherwise the loss
/// distribution mixes currencies. The function does not silently convert — it sums
/// the raw P&L it is given.
///
/// Returns `(var, es)`; both are non-negative loss magnitudes. An empty scenario
/// set yields `(0.0, 0.0)`.
#[must_use]
pub fn historical_var_es(positions: &[PositionRisk], scenarios: &[Scenario], alpha: f64) -> VarEs {
    if scenarios.is_empty() {
        return VarEs { var: 0.0, es: 0.0 };
    }
    // P&L per scenario (a loss is a negative P&L).
    let mut pnl: Vec<f64> = scenarios.iter().map(|s| node_pnl(positions, *s)).collect();
    // Sort ascending: the worst losses (most negative P&L) are first.
    pnl.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = pnl.len();
    // The VaR index: the alpha-tail. floor((1-alpha)·n) worst observations are the
    // tail; the VaR is the loss at the tail boundary.
    let tail = (((1.0 - alpha) * n as f64).floor() as usize).max(1).min(n);
    // ES = mean loss over the worst `tail` observations.
    let tail_sum: f64 = pnl[..tail].iter().sum();
    let es = -(tail_sum / tail as f64);
    // VaR = the loss at the boundary observation (the best of the tail).
    let var = -pnl[tail - 1];
    VarEs {
        var: var.max(0.0),
        es: es.max(0.0),
    }
}

/// VaR / Expected-Shortfall pair (both non-negative loss magnitudes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VarEs {
    /// Value-at-Risk: the `alpha`-quantile loss.
    pub var: f64,
    /// Expected Shortfall: the mean loss in the tail beyond VaR.
    pub es: f64,
}

/// FRTB-SbM **curvature** charge of a node along the spot risk factor, by
/// up/down full reprice net of the delta (linear) approximation
/// (`docs/RISK-HIERARCHY.md` §2.5, MAR21 curvature).
///
/// Curvature captures the gamma risk the linear SbM delta charge misses. For a
/// relative spot shock `rw` (the prescribed risk weight), the curvature for each
/// direction is
/// `CVR_k = −[ V(x·(1±rw)) − V(x) ∓ rw·x·delta ]`,
/// and the node charge is `max(CVR_up, CVR_down, 0)` summed across positions.
/// Because of the `max`, curvature **cannot be summed** from child node charges —
/// it is re-derived here from the node's positions.
///
/// The shock `rw` is the FX curvature risk weight (externally supplied, never
/// compiled-in per §2.3 / §2.11). The linear (delta) term is **netted per
/// position** as `Σ (∂V/∂Sᵢ · notionalᵢ · rw · Sᵢ)`: because a relative shock `rw`
/// is a *different* absolute spot move per position when a heterogeneous book
/// spans several spot levels, the convention-exact linear term cannot use a single
/// representative spot — it is recomputed from each position's own spot/delta here.
#[must_use]
pub fn sbm_curvature_spot(positions: &[PositionRisk], rw: f64) -> f64 {
    // Reprice the node up and down by the relative shock.
    let base = node_value(positions);
    let up = node_value_shocked(positions, 1.0 + rw);
    let down = node_value_shocked(positions, 1.0 - rw);
    // The convention-exact linear term, netted per position (see the doc note).
    let linear = positions
        .iter()
        .map(|p| {
            let g = celnet_vanilla::greeks(p.option, &p.inputs);
            g.delta_spot * p.notional_base * rw * p.inputs.spot
        })
        .sum::<f64>();
    let cvr_up = -((up - base) - linear);
    let cvr_down = -((down - base) + linear);
    cvr_up.max(cvr_down).max(0.0)
}

/// Correlation-weighted vega aggregation across vega buckets — the SbM-style
/// `√(Σᵢ wᵢ² + Σᵢ≠ⱼ ρᵢⱼ wᵢ wⱼ)` quadratic form (`docs/RISK-HIERARCHY.md` §2.5).
///
/// `weighted_vegas` are the risk-weighted bucket vegas (already in a common
/// numeraire); `rho` is the symmetric inter-bucket correlation supplied per the
/// (versioned, externally-supplied) regulatory matrix — `rho(i, j)` for `i != j`.
/// The result is a single non-negative aggregate. This is a non-additive
/// reduction (a square-root of a quadratic form), so it is re-derived per node,
/// never summed from child aggregates.
///
/// A negative quadratic form (possible with a non-PSD correlation matrix under
/// the SbM low/high scenarios) is floored at zero before the square root, matching
/// the SbM alternative-specification fallback (MAR21.4).
#[must_use]
pub fn correlation_weighted_vega<F>(weighted_vegas: &[f64], rho: F) -> f64
where
    F: Fn(usize, usize) -> f64,
{
    let n = weighted_vegas.len();
    let mut acc = 0.0;
    for i in 0..n {
        acc += weighted_vegas[i] * weighted_vegas[i];
        for j in (i + 1)..n {
            acc += 2.0 * rho(i, j) * weighted_vegas[i] * weighted_vegas[j];
        }
    }
    sqrt(acc.max(0.0))
}

/// The base (unshocked) value of a node in quote ccy (Σ per-unit PV × notional).
fn node_value(positions: &[PositionRisk]) -> f64 {
    positions
        .iter()
        .map(|p| price(p.option, &p.inputs) * p.notional_base)
        .sum()
}

/// The node value after multiplying every position's spot by `spot_mult`.
fn node_value_shocked(positions: &[PositionRisk], spot_mult: f64) -> f64 {
    positions
        .iter()
        .map(|p| {
            let shocked = VanillaInputs::new(
                p.inputs.spot * spot_mult,
                p.inputs.strike,
                p.inputs.vol,
                p.inputs.t,
                p.inputs.r_dom,
                p.inputs.r_for,
            );
            price(p.option, &shocked) * p.notional_base
        })
        .sum()
}
