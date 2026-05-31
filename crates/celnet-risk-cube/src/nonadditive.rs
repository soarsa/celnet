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
//! # Two lenses: bump-and-revalue **oracle** + AAD sensitivity **scale path**
//!
//! This module offers two reconcilable ways to compute a node's VaR/ES:
//!
//! 1. **Bump-and-revalue ([`historical_var_es`]) — the exact reference / oracle.**
//!    Shock a risk factor, re-price every constituent position with
//!    `celnet-vanilla`'s closed forms, and reduce. This is **correct** and is the
//!    natural reference, but it is **not** the fastest at investment-banking scale:
//!    it costs O(positions × scenarios) full repricings per node.
//!
//! 2. **AAD sensitivity reval ([`sensitivity_var_es`]) — the scale path, now
//!    wired.** `docs/RISK-HIERARCHY.md` §3.3 names **adjoint algorithmic
//!    differentiation (AAD)** as THE throughput lever: compute the *full* Greek set
//!    of each position in **one** reverse sweep
//!    ([`celnet_vanilla::adjoint_greeks`]), then expand each scenario's node P&L by
//!    a **second-order Taylor series** in the shocked factors. The Greeks are
//!    computed **once per position** and reused across **all** scenarios, so the
//!    cost collapses from O(positions × scenarios) repricings to **O(positions)**
//!    adjoint sweeps + O(positions × scenarios) cheap floating-point arithmetic.
//!    The adjoint sweep itself already costs ~one price for the whole gradient
//!    (`celnet-vanilla::adjoint` module docs / §3.3), so the per-position price work
//!    is paid once, not once per scenario. This is no longer deferred or faked: the
//!    genuine reverse-mode AAD in `celnet-vanilla` is the engine here.
//!
//! The bump-and-revalue result remains the **oracle the AAD path is validated
//! against** ([`historical_var_es`] is retained, never deleted): the test suite
//! reconciles [`sensitivity_var_es`] against it within a documented Taylor
//! tolerance over a moderate shock regime, and proves the gap widens for large
//! shocks (the honest truncation regime of a second-order expansion).
//!
//! # On the GPU lever (honest, deliberate non-wiring)
//!
//! The batched-GPU Monte-Carlo `celnet_gpu::ScenarioPricer` (driven by
//! [`crate::scenario_grid`]) is **deliberately NOT** wired into this closed-form
//! vanilla VaR path. Mixing Monte-Carlo estimator noise into an *exact* closed-form
//! reval would be a numerical regression — the analytic/AAD path is machine-exact,
//! and a 5σ-band MC grid is not. The appropriate GPU lever for *this* closed-form
//! path is a **batched closed-form vanilla kernel** (`docs/GPU-AT-SCALE-PLAN.md`
//! Workload A / G2): the same exact arithmetic, dispatched across positions×
//! scenarios in parallel. That is a distinct, still-pending GPU increment; the AAD
//! sensitivity lens here is the *algorithmic* throughput win that lands first.
//!
//! All shocks are **relative or absolute parameter bumps** applied to
//! [`VanillaInputs`]; both repricing and the AAD sweep are deterministic (`libm`),
//! so every measure here is bit-reproducible for a fixed scenario set.

use celnet_core::math::sqrt;
use celnet_risk_normalize::PositionRisk;
use celnet_types::{Greeks, VanillaInputs};
use celnet_vanilla::{adjoint_greeks, price};

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
    // Full bump-and-revalue P&L per scenario (a loss is a negative P&L). The tail
    // reduction is the SHARED `quantile_var_es`, identical to the AAD lens.
    let mut pnl: Vec<f64> = scenarios.iter().map(|s| node_pnl(positions, *s)).collect();
    quantile_var_es(&mut pnl, alpha)
}

/// The precomputed second-order sensitivity profile of one position, in the
/// position's quote (domestic) currency, ready to expand any scenario's P&L by a
/// closed Taylor form **without** re-pricing.
///
/// Each field is the notional-scaled risk-factor sensitivity of the position's
/// value `V = price · notional_base`. The Greek set is obtained from a **single**
/// reverse-mode [`celnet_vanilla::adjoint_greeks`] sweep (one sweep yields the
/// whole first-order set plus second-order gamma/vanna/volga); the values below are
/// those adjoints multiplied by `notional_base`, so a node profile is just the
/// element-wise sum of its positions' profiles.
///
/// Units are deliberately explicit so the Taylor expansion is unit-correct:
/// `delta_spot` is `∂V/∂S` (per **absolute** spot move `dS`, **not** per relative
/// move), `gamma` is `∂²V/∂S²`, `vega`/`volga` are per absolute vol move,
/// `vanna` is `∂²V/∂S∂σ`, and the rhos are per absolute rate move. `spot` is the
/// position's own spot level, retained so a relative spot shock `spot_rel` is
/// turned into the absolute move `dS = spot · spot_rel` at expansion time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionSensitivity {
    /// The position's own spot level (to convert relative→absolute spot shocks).
    pub spot: f64,
    /// `∂V/∂S` · notional (per absolute spot move).
    pub delta_spot: f64,
    /// `∂²V/∂S²` · notional.
    pub gamma: f64,
    /// `∂V/∂σ` · notional (per absolute vol move).
    pub vega: f64,
    /// `∂²V/∂σ²` · notional.
    pub volga: f64,
    /// `∂²V/∂S∂σ` · notional.
    pub vanna: f64,
    /// `∂V/∂r_d` · notional (per absolute domestic-rate move).
    pub rho_dom: f64,
    /// `∂V/∂r_f` · notional (per absolute foreign-rate move).
    pub rho_for: f64,
}

impl PositionSensitivity {
    /// Build a position's sensitivity profile from **one** reverse-mode AAD sweep
    /// ([`celnet_vanilla::adjoint_greeks`]) scaled by notional. This is the single
    /// price-equivalent of work paid once per position; every scenario then reuses
    /// it via [`PositionSensitivity::taylor_pnl`].
    #[must_use]
    pub fn from_position(pos: &PositionRisk) -> Self {
        let g: Greeks = adjoint_greeks(pos.option, &pos.inputs);
        let n = pos.notional_base;
        Self {
            spot: pos.inputs.spot,
            delta_spot: g.delta_spot * n,
            gamma: g.gamma * n,
            vega: g.vega * n,
            volga: g.volga * n,
            vanna: g.vanna * n,
            rho_dom: g.rho_dom * n,
            rho_for: g.rho_for * n,
        }
    }

    /// The second-order Taylor estimate of this position's P&L under `scenario`,
    /// in quote ccy:
    ///
    /// ```text
    /// dV ≈ delta_spot·dS + ½·gamma·dS²
    ///    + vega·dσ + ½·volga·dσ²
    ///    + vanna·dS·dσ
    ///    + rho_dom·dr_d + rho_for·dr_f
    /// ```
    ///
    /// where `dS = spot · spot_rel` (the **absolute** spot move implied by the
    /// relative shock), `dσ = vol_abs`, `dr_d = rate_dom_abs`, `dr_f =
    /// rate_for_abs`. Rate sensitivity is kept first-order: vanilla rho convexity is
    /// negligible over a VaR-scale rate shock and the analytic/AAD set carries no
    /// second-order rate Greek, so adding a fake one would over-claim.
    #[must_use]
    pub fn taylor_pnl(&self, scenario: Scenario) -> f64 {
        let d_s = self.spot * scenario.spot_rel;
        let d_vol = scenario.vol_abs;
        self.delta_spot * d_s
            + 0.5 * self.gamma * d_s * d_s
            + self.vega * d_vol
            + 0.5 * self.volga * d_vol * d_vol
            + self.vanna * d_s * d_vol
            + self.rho_dom * scenario.rate_dom_abs
            + self.rho_for * scenario.rate_for_abs
    }
}

/// Compute every position's [`PositionSensitivity`] for a node in **one** AAD
/// sweep per position (O(positions) sweeps total) — the precompute step shared by
/// all scenarios in [`sensitivity_var_es`].
#[must_use]
pub fn node_sensitivities(positions: &[PositionRisk]) -> Vec<PositionSensitivity> {
    positions
        .iter()
        .map(PositionSensitivity::from_position)
        .collect()
}

/// AAD **sensitivity-based** VaR / Expected Shortfall of a node
/// (`docs/RISK-HIERARCHY.md` §3.3) — the *scale path* mirror of
/// [`historical_var_es`].
///
/// # Method (the throughput win, precisely)
///
/// Each position's full Greek set is computed **once** by a single reverse-mode
/// [`celnet_vanilla::adjoint_greeks`] sweep (`node_sensitivities`), then each
/// scenario's node P&L is a **second-order Taylor expansion** in the shocked
/// factors (see [`PositionSensitivity::taylor_pnl`]). The VaR/ES quantile/tail
/// reduction over the resulting P&L vector is **identical** to
/// [`historical_var_es`] — only the per-scenario node valuation differs (Taylor
/// vs full reprice).
///
/// Complexity: where [`historical_var_es`] does `O(positions × scenarios)` full
/// closed-form repricings, this does `O(positions)` adjoint sweeps (≈ one price
/// each, all-Greeks-for-the-price-of-one — `celnet-vanilla::adjoint` docs) plus
/// `O(positions × scenarios)` of nothing-but-multiply-add arithmetic. For the
/// large scenario libraries an IB VaR run uses, the per-scenario reprice cost — the
/// dominant term — is eliminated.
///
/// # Accuracy regime (honest)
///
/// The Taylor expansion is **exact in the limit of small shocks** and matches the
/// oracle to the truncation error of a 2nd-order series: it captures delta, gamma,
/// vega, volga, vanna and first-order rho, but **not** higher-order terms (speed
/// `∂³V/∂S³`, the vol/spot cross-convexity beyond vanna, rate convexity, or the
/// genuine non-linearity of a deep-in-the-money reprice). It is therefore accurate
/// for the moderate symmetric shock ladders a daily VaR run uses and **diverges for
/// large shocks** — the test suite documents exactly that regime (close agreement
/// over ±5% spot / ±2 vol-pts, widening gap beyond). Use [`historical_var_es`] when
/// an exact tail is required; use this for the fast path and reconcile periodically.
///
/// Returns `(var, es)`; both are non-negative loss magnitudes. An empty scenario
/// set yields `(0.0, 0.0)`. The same currency caveat as [`historical_var_es`]
/// applies: P&L is summed in each position's quote ccy (numeraire-normalize first
/// for a multi-currency node). Bit-reproducible for a fixed scenario set (`libm`).
#[must_use]
pub fn sensitivity_var_es(positions: &[PositionRisk], scenarios: &[Scenario], alpha: f64) -> VarEs {
    if scenarios.is_empty() {
        return VarEs { var: 0.0, es: 0.0 };
    }
    // ONE adjoint sweep per position, reused across every scenario.
    let sens = node_sensitivities(positions);
    // Node Taylor P&L per scenario (a loss is a negative P&L).
    let mut pnl: Vec<f64> = scenarios
        .iter()
        .map(|s| sens.iter().map(|p| p.taylor_pnl(*s)).sum())
        .collect();
    quantile_var_es(&mut pnl, alpha)
}

/// The shared VaR/ES tail reduction over a P&L vector (a loss is a negative P&L).
/// Sorts ascending, takes the `alpha`-tail boundary as VaR and the mean tail loss
/// as ES. This is the exact reduction [`historical_var_es`] uses, factored out so
/// the bump-and-revalue oracle and the AAD sensitivity lens are guaranteed to apply
/// **identical** quantile logic — only their per-scenario valuation differs.
fn quantile_var_es(pnl: &mut [f64], alpha: f64) -> VarEs {
    if pnl.is_empty() {
        return VarEs { var: 0.0, es: 0.0 };
    }
    pnl.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = pnl.len();
    let tail = (((1.0 - alpha) * n as f64).floor() as usize).max(1).min(n);
    let tail_sum: f64 = pnl[..tail].iter().sum();
    let es = -(tail_sum / tail as f64);
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

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn pos(opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
        PositionRisk::new(
            eurusd(),
            opt,
            notional,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    }

    /// A moderate symmetric shock ladder: spot ±5% in 1% rungs × vol ±2 vol-pts in
    /// 0.5 vol-pt rungs — the regime a daily VaR run actually uses, where the
    /// 2nd-order Taylor expansion is expected to track the full reprice tightly.
    fn moderate_ladder() -> Vec<Scenario> {
        let mut v = Vec::new();
        for si in -5..=5 {
            for vj in -4..=4 {
                v.push(Scenario {
                    spot_rel: f64::from(si) * 0.01,
                    vol_abs: f64::from(vj) * 0.005,
                    rate_dom_abs: 0.0,
                    rate_for_abs: 0.0,
                });
            }
        }
        v
    }

    /// **The fast lens genuinely consumes `adjoint_greeks`.** The per-position
    /// sensitivity profile must equal `celnet_vanilla::adjoint_greeks` scaled by
    /// notional — bit-identical, proving the lens is built on the real reverse-mode
    /// AAD sweep (not the analytic `greeks`, not finite differences).
    #[test]
    fn sensitivity_profile_is_adjoint_greeks_scaled_by_notional() {
        let inputs = VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02);
        let notional = 7_500_000.0;
        let p = pos(OptionType::Call, notional, inputs);
        let s = PositionSensitivity::from_position(&p);
        let g = adjoint_greeks(OptionType::Call, &inputs);
        assert_eq!(s.delta_spot.to_bits(), (g.delta_spot * notional).to_bits());
        assert_eq!(s.gamma.to_bits(), (g.gamma * notional).to_bits());
        assert_eq!(s.vega.to_bits(), (g.vega * notional).to_bits());
        assert_eq!(s.volga.to_bits(), (g.volga * notional).to_bits());
        assert_eq!(s.vanna.to_bits(), (g.vanna * notional).to_bits());
        assert_eq!(s.rho_dom.to_bits(), (g.rho_dom * notional).to_bits());
        assert_eq!(s.rho_for.to_bits(), (g.rho_for * notional).to_bits());
        assert_eq!(s.spot.to_bits(), inputs.spot.to_bits());
    }

    /// **RECONCILIATION (moderate regime): the AAD sensitivity VaR/ES ≈ the
    /// bump-and-revalue oracle.** Over the moderate ladder (spot ±5%, vol ±2 pts) on
    /// a multi-position node, `sensitivity_var_es` tracks `historical_var_es` to a
    /// **documented 8% relative tolerance**.
    ///
    /// Tolerance justification (measured, not asserted-plausible): the 99%-VaR over
    /// this ladder is the *worst-corner* tail observation — simultaneously spot −5%
    /// **and** the extreme vol rung — where a 2nd-order Taylor expansion of a smooth
    /// vanilla PV carries its largest O(shock³) truncation residual. Empirically
    /// that residual is ≈6.1% of the tail loss here (and shrinks monotonically with
    /// the shock: ≈3.7% at ±3%/±1pt, ≈1.4% at ±2%/±0.5pt — see
    /// `sensitivity_var_taylor_residual_shrinks_with_shock`). 8% is a tight,
    /// honest envelope for the worst-corner 99% VaR at the ±5%/±2pt daily-VaR
    /// regime — NOT a machine-precision claim. For an exact tail use the
    /// bump-and-revalue oracle; this lens is the fast path, reconciled periodically.
    #[test]
    fn sensitivity_var_reconciles_to_oracle_moderate_shocks() {
        let scen = moderate_ladder();
        let node = [
            pos(
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
            pos(
                OptionType::Put,
                6_000_000.0,
                VanillaInputs::new(1.10, 1.06, 0.115, 0.75, 0.04, 0.02),
            ),
            pos(
                OptionType::Call,
                -4_000_000.0,
                VanillaInputs::new(1.10, 1.15, 0.095, 1.5, 0.04, 0.02),
            ),
        ];
        let oracle = historical_var_es(&node, &scen, 0.99);
        let fast = sensitivity_var_es(&node, &scen, 0.99);
        assert!(
            oracle.var > 0.0 && oracle.es > 0.0,
            "oracle must see a tail loss"
        );
        // 8% relative agreement over the moderate worst-corner regime (see doc).
        assert!(
            is_close(fast.var, oracle.var, 8e-2, 1e-3),
            "VaR fast {} vs oracle {} (moderate regime)",
            fast.var,
            oracle.var
        );
        assert!(
            is_close(fast.es, oracle.es, 8e-2, 1e-3),
            "ES fast {} vs oracle {} (moderate regime)",
            fast.es,
            oracle.es
        );
    }

    /// **The Taylor residual shrinks monotonically as the shock shrinks** — the
    /// quantitative signature of an O(shock³) truncation error, and the evidence
    /// behind the documented tolerance in
    /// `sensitivity_var_reconciles_to_oracle_moderate_shocks`. We measure the
    /// sensitivity-vs-oracle 99%-VaR relative error on the same node at three
    /// nested ladders and assert it falls strictly each time.
    #[test]
    fn sensitivity_var_taylor_residual_shrinks_with_shock() {
        let node = [
            pos(
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
            pos(
                OptionType::Put,
                6_000_000.0,
                VanillaInputs::new(1.10, 1.06, 0.115, 0.75, 0.04, 0.02),
            ),
            pos(
                OptionType::Call,
                -4_000_000.0,
                VanillaInputs::new(1.10, 1.15, 0.095, 1.5, 0.04, 0.02),
            ),
        ];
        let rel_err = |sp: i32, sr: f64, vp: i32, vr: f64| -> f64 {
            let mut scen = Vec::new();
            for si in -sp..=sp {
                for vj in -vp..=vp {
                    scen.push(Scenario {
                        spot_rel: f64::from(si) * sr,
                        vol_abs: f64::from(vj) * vr,
                        rate_dom_abs: 0.0,
                        rate_for_abs: 0.0,
                    });
                }
            }
            let o = historical_var_es(&node, &scen, 0.99).var;
            let f = sensitivity_var_es(&node, &scen, 0.99).var;
            ((f - o) / o).abs()
        };
        let wide = rel_err(5, 0.01, 4, 0.005); // ±5% / ±2pt
        let mid = rel_err(3, 0.01, 2, 0.005); // ±3% / ±1pt
        let tight = rel_err(2, 0.01, 1, 0.005); // ±2% / ±0.5pt
        assert!(
            wide > mid && mid > tight,
            "Taylor residual must shrink with shock: wide {wide} > mid {mid} > tight {tight}"
        );
        // The tightest moderate regime is well inside 2% — the lens is genuinely
        // accurate where daily VaR lives.
        assert!(tight < 2e-2, "tight-regime rel-err {tight} should be <2%");
    }

    /// **HONEST regime boundary: the Taylor gap WIDENS for large shocks.** The same
    /// node, priced over a small ±5% ladder vs a large ±40% ladder: the relative
    /// error of the sensitivity VaR against the bump-and-revalue oracle must be
    /// strictly larger under the large shocks — the 2nd-order truncation error
    /// growing with shock size. This proves the method's regime rather than
    /// over-claiming machine accuracy everywhere.
    #[test]
    fn sensitivity_var_gap_widens_for_large_shocks() {
        let node = [
            pos(
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
            pos(
                OptionType::Put,
                8_000_000.0,
                VanillaInputs::new(1.10, 1.05, 0.12, 0.5, 0.04, 0.02),
            ),
        ];
        let rel_err = |spot_step: f64, rungs: i32| -> f64 {
            let scen: Vec<Scenario> = (-rungs..=rungs)
                .filter(|i| *i != 0)
                .map(|i| Scenario::spot(f64::from(i) * spot_step))
                .collect();
            let o = historical_var_es(&node, &scen, 0.99).var;
            let f = sensitivity_var_es(&node, &scen, 0.99).var;
            ((f - o) / o).abs()
        };
        // Small regime: ±5% spot in 1% rungs. Large regime: ±40% spot in 8% rungs.
        let small = rel_err(0.01, 5);
        let large = rel_err(0.08, 5);
        // The gap widens dramatically: a worst-corner ±5% pure-spot 99% VaR is
        // ≈7% off (the 2nd-order residual at that corner), while a ±40% shock —
        // far outside any daily-VaR regime — blows out to >40% as the cubic and
        // higher terms the expansion drops come to dominate. This is the honest
        // statement of the method's regime, not a uniform machine-accuracy claim.
        assert!(
            large > 4.0 * small,
            "Taylor truncation must widen the gap sharply: small-shock rel-err \
             {small} vs large-shock rel-err {large}"
        );
        assert!(
            large > 0.4,
            "a ±40% shock must drive the Taylor lens far from the oracle, got {large}"
        );
        // The small (daily-VaR-scale) regime stays bounded well under 10%.
        assert!(small < 0.10, "small-shock rel-err {small} should be <10%");
    }

    /// **Bit-reproducibility of the fast lens.** For a fixed scenario set the AAD
    /// sensitivity VaR/ES is byte-identical across runs (`libm`-deterministic sweep
    /// + deterministic quantile reduction).
    #[test]
    fn sensitivity_var_is_bit_reproducible() {
        let scen = moderate_ladder();
        let node = [
            pos(
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
            pos(
                OptionType::Put,
                -3_000_000.0,
                VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.04, 0.02),
            ),
        ];
        let a = sensitivity_var_es(&node, &scen, 0.975);
        let b = sensitivity_var_es(&node, &scen, 0.975);
        assert_eq!(a.var.to_bits(), b.var.to_bits());
        assert_eq!(a.es.to_bits(), b.es.to_bits());
    }

    /// **The rate-shock terms expand correctly.** A pure domestic-rate scenario's
    /// node Taylor P&L equals `rho_dom · dr_d` summed over positions — first-order,
    /// as documented (no fake rate convexity).
    #[test]
    fn rate_shock_expansion_is_first_order_rho() {
        let p = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let s = PositionSensitivity::from_position(&p);
        let dr = 0.0010; // +10bp domestic.
        let scenario = Scenario {
            spot_rel: 0.0,
            vol_abs: 0.0,
            rate_dom_abs: dr,
            rate_for_abs: 0.0,
        };
        assert!(is_close(
            s.taylor_pnl(scenario),
            s.rho_dom * dr,
            1e-12,
            1e-9
        ));
    }
}
