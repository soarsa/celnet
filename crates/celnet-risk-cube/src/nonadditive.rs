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

use celnet_core::carry::{CarryInputs, CarryPricer};
use celnet_core::math::sqrt;
use celnet_risk_normalize::PositionRisk;
use celnet_types::{Carry, Greeks, RateSensitivities};
use celnet_vanilla::adjoint_greeks;

/// One scenario: a set of multiplicative/additive shocks to the pricing inputs of
/// every constituent position, used to reprice the node under stress.
///
/// Shocks are expressed as the *change applied to each position's own inputs*, so
/// a single scenario means the same economic move (e.g. "spot +1 %, vol +1 vol")
/// applied consistently across a heterogeneous **cross-asset** book. `spot_rel`
/// multiplies spot; `vol_abs` adds to vol (in absolute vol units, so 0.01 = +1 vol
/// point); the two carry shocks bump the asset-agnostic carry coordinates `(r, b)`:
/// `discount_abs` adds to the discount rate `r`, `carry_abs` adds to the net carry
/// `b` (`F = S·e^{b·t}`).
///
/// # Cross-asset carry mapping (ADR-0008)
///
/// The shocks are stated in the **carry-neutral** `(r, b)` basis so a single scenario
/// is the *same* economic move across every asset class — and there is **no
/// match-on-underlying** here. For FX, `r = r_dom` and `b = r_dom − r_for`, so a
/// classic `Δr_dom`/`Δr_for` shock maps to `discount_abs = Δr_dom`,
/// `carry_abs = Δr_dom − Δr_for` (see [`Scenario::fx_rates`]). The shock arithmetic
/// itself lives on [`Carry`] (the legitimate two-arm carry transform), applied
/// uniformly to whichever carry the position holds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scenario {
    /// Relative spot shock (0.01 = +1 %). Applied as `spot *= 1 + spot_rel`.
    pub spot_rel: f64,
    /// Absolute vol shock in vol units (0.01 = +1 vol point). `vol += vol_abs`.
    pub vol_abs: f64,
    /// Absolute discount-rate shock `Δr` (the numeraire rate; `r_dom` for FX).
    pub discount_abs: f64,
    /// Absolute net-carry shock `Δb` (`r_dom − r_for` for FX).
    pub carry_abs: f64,
}

impl Scenario {
    /// The no-op (base) scenario.
    #[must_use]
    pub const fn base() -> Self {
        Self {
            spot_rel: 0.0,
            vol_abs: 0.0,
            discount_abs: 0.0,
            carry_abs: 0.0,
        }
    }

    /// A pure relative-spot scenario.
    #[must_use]
    pub const fn spot(spot_rel: f64) -> Self {
        Self {
            spot_rel,
            vol_abs: 0.0,
            discount_abs: 0.0,
            carry_abs: 0.0,
        }
    }

    /// A pure absolute-vol scenario.
    #[must_use]
    pub const fn vol(vol_abs: f64) -> Self {
        Self {
            spot_rel: 0.0,
            vol_abs,
            discount_abs: 0.0,
            carry_abs: 0.0,
        }
    }

    /// Construct the carry shocks from a classic FX two-rate shock `(Δr_dom, Δr_for)`:
    /// `discount_abs = Δr_dom`, `carry_abs = Δr_dom − Δr_for` (so `r = r_dom` shifts by
    /// `Δr_dom` and `b = r_dom − r_for` shifts by `Δr_dom − Δr_for`). This reproduces
    /// the pre-generalization FX scenario byte-for-byte.
    #[must_use]
    pub const fn fx_rates(spot_rel: f64, vol_abs: f64, d_r_dom: f64, d_r_for: f64) -> Self {
        Self {
            spot_rel,
            vol_abs,
            discount_abs: d_r_dom,
            carry_abs: d_r_dom - d_r_for,
        }
    }

    /// Apply this scenario's shocks to a set of carry-tagged pricing inputs, asset-
    /// class-agnostically: shock spot relatively, vol absolutely, and shift the carry
    /// coordinates `(r, b)` by `(discount_abs, carry_abs)` via [`shift_carry`]. The
    /// underlying is untouched; **no match on the underlying** appears here.
    #[must_use]
    pub fn apply(&self, i: &CarryInputs) -> CarryInputs {
        CarryInputs::new(
            i.spot * (1.0 + self.spot_rel),
            i.strike,
            i.vol + self.vol_abs,
            i.t,
            i.underlying.clone(),
            shift_carry(i.carry, self.discount_abs, self.carry_abs),
        )
    }
}

/// Shift a [`Carry`] by `(Δr, Δb)` in the asset-agnostic discount/carry basis. For
/// [`Carry::FxRates`] this reproduces `r_dom += Δr`, `r_for += Δr − Δb` (so the
/// discount rate `r = r_dom` shifts by `Δr` and the net carry `b = r_dom − r_for`
/// shifts by `Δb`); for [`Carry::CostOfCarry`] it is `r += Δr`, `b += Δb` directly.
/// The two-arm match here is the legitimate carry transform (ADR-0008-clean — a
/// carry-internal operation, not an aggregation-loop branch on the underlying).
#[must_use]
pub fn shift_carry(carry: Carry, discount_abs: f64, carry_abs: f64) -> Carry {
    match carry {
        Carry::FxRates { r_dom, r_for } => Carry::FxRates {
            r_dom: r_dom + discount_abs,
            // b = r_dom − r_for shifts by carry_abs ⇒ r_for shifts by (Δr − Δb).
            r_for: r_for + (discount_abs - carry_abs),
        },
        Carry::CostOfCarry { r, b } => Carry::CostOfCarry {
            r: r + discount_abs,
            b: b + carry_abs,
        },
    }
}

/// Re-price a single position under a scenario, returning its **P&L** vs base in
/// the position's quote (domestic) currency, scaled by notional and signed by the
/// long/short direction of the notional.
///
/// `price` returns the per-unit-base numeraire PV; multiplying by `notional_base`
/// gives the position value, and the P&L is `value(shocked) − value(base)`.
///
/// The base and shocked prices are obtained **through the agnostic
/// [`CarryPricer`] seam** — the asset's own leaf prices the position; this function
/// never matches on the underlying and never silently FX-proxies a non-FX leg. A
/// position the pricer cannot price contributes `0.0` (its leaf rejected it); the
/// caller chooses a dispatcher whose leaves cover the book (see
/// [`node_pnl`]/[`historical_var_es`], which take the dispatcher explicitly).
#[must_use]
pub fn position_pnl<P: CarryPricer>(pricer: &P, pos: &PositionRisk, scenario: Scenario) -> f64 {
    let Ok(base_unit) = pricer.price(pos.option, &pos.inputs) else {
        return 0.0;
    };
    let shocked = scenario.apply(&pos.inputs);
    let Ok(shocked_unit) = pricer.price(pos.option, &shocked) else {
        return 0.0;
    };
    (shocked_unit - base_unit) * pos.notional_base
}

/// The total P&L of a node (its constituent positions) under one scenario, in the
/// **common premium currency** assumption (all positions share a quote ccy) — see
/// the note on [`historical_var_es`] for the multi-currency caveat. Each leg is
/// repriced through the seam (`pricer`).
#[must_use]
pub fn node_pnl<P: CarryPricer>(pricer: &P, positions: &[PositionRisk], scenario: Scenario) -> f64 {
    positions
        .iter()
        .map(|p| position_pnl(pricer, p, scenario))
        .sum()
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
pub fn historical_var_es<P: CarryPricer>(
    pricer: &P,
    positions: &[PositionRisk],
    scenarios: &[Scenario],
    alpha: f64,
) -> VarEs {
    if scenarios.is_empty() {
        return VarEs { var: 0.0, es: 0.0 };
    }
    // Full bump-and-revalue P&L per scenario (a loss is a negative P&L), repriced
    // through the seam. The tail reduction is the SHARED `quantile_var_es`,
    // identical to the AAD lens.
    let mut pnl: Vec<f64> = scenarios
        .iter()
        .map(|s| node_pnl(pricer, positions, *s))
        .collect();
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
/// `vanna` is `∂²V/∂S∂σ`, and the rate sensitivities are carry-tagged
/// (`discount_rho = ∂V/∂r`, `carry_rho = ∂V/∂b`) per absolute carry-coordinate move.
/// `spot` is the position's own spot level, retained so a relative spot shock
/// `spot_rel` is turned into the absolute move `dS = spot · spot_rel` at expansion
/// time.
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
    /// `∂V/∂r` · notional (discount-rate rho; per absolute `discount_abs` move).
    /// For FX `r = r_dom`, so this is the domestic rho.
    pub discount_rho: f64,
    /// `∂V/∂b` · notional (net-carry rho; per absolute `carry_abs` move). For FX
    /// `b = r_dom − r_for`, so this is `−rho_for` (since `∂/∂b = −∂/∂r_for` at fixed
    /// `r_dom`).
    pub carry_rho: f64,
}

impl PositionSensitivity {
    /// Build a position's sensitivity profile through the agnostic seam, scaled by
    /// notional. For an FX/metal underlying this consumes **one** reverse-mode AAD
    /// sweep ([`celnet_vanilla::adjoint_greeks`]) — the price-equivalent gradient
    /// paid once per position; for every other asset class it uses the leaf's
    /// closed-form carry-tagged Greeks through `pricer.price_greeks`. The rate block
    /// is normalized to the carry basis `(discount_rho, carry_rho)` regardless of
    /// asset class, so a heterogeneous node's profiles sum element-wise.
    ///
    /// A position the pricer cannot price yields an all-zero profile (its leaf
    /// rejected it) — the same no-silent-FX-proxy contract as [`position_pnl`].
    #[must_use]
    pub fn from_position<P: CarryPricer>(pricer: &P, pos: &PositionRisk) -> Self {
        let n = pos.notional_base;
        // FX/metal: the fast reverse-mode AAD sweep (one gradient for the price of
        // one price). The discriminant read here is the FX-acceleration guard (the
        // leaf's own "is this FX?" lowering), not an aggregation-loop match.
        if let Ok(vi) = celnet_core::carry::fx_vanilla_inputs(&pos.inputs) {
            let g: Greeks = adjoint_greeks(pos.option, &vi);
            return Self {
                spot: pos.inputs.spot,
                delta_spot: g.delta_spot * n,
                gamma: g.gamma * n,
                vega: g.vega * n,
                volga: g.volga * n,
                vanna: g.vanna * n,
                // FX carry basis: r = r_dom ⇒ ∂V/∂r = rho_dom; b = r_dom − r_for ⇒
                // ∂V/∂b = −rho_for (at fixed r_dom).
                discount_rho: g.rho_dom * n,
                carry_rho: -g.rho_for * n,
            };
        }
        // Non-FX: the leaf's closed-form carry-tagged strip through the seam.
        let Ok(g) = pricer.price_greeks(pos.option, &pos.inputs) else {
            return Self::zero(pos.inputs.spot);
        };
        let (discount_rho, carry_rho) = match g.rates {
            RateSensitivities::Carry {
                discount_rho,
                carry_rho,
            } => (discount_rho, carry_rho),
            // An FX-tagged strip from a non-FX-lowering leaf cannot occur (FX lowers
            // above); normalize defensively to the carry basis.
            RateSensitivities::Fx { rho_dom, rho_for } => (rho_dom, -rho_for),
        };
        Self {
            spot: pos.inputs.spot,
            delta_spot: g.delta_spot * n,
            gamma: g.gamma * n,
            vega: g.vega * n,
            volga: g.volga * n,
            vanna: g.vanna * n,
            discount_rho: discount_rho * n,
            carry_rho: carry_rho * n,
        }
    }

    /// An all-zero profile at a given spot (an unpriceable position's contribution).
    #[must_use]
    const fn zero(spot: f64) -> Self {
        Self {
            spot,
            delta_spot: 0.0,
            gamma: 0.0,
            vega: 0.0,
            volga: 0.0,
            vanna: 0.0,
            discount_rho: 0.0,
            carry_rho: 0.0,
        }
    }

    /// The second-order Taylor estimate of this position's P&L under `scenario`,
    /// in quote ccy:
    ///
    /// ```text
    /// dV ≈ delta_spot·dS + ½·gamma·dS²
    ///    + vega·dσ + ½·volga·dσ²
    ///    + vanna·dS·dσ
    ///    + discount_rho·dr + carry_rho·db
    /// ```
    ///
    /// where `dS = spot · spot_rel` (the **absolute** spot move implied by the
    /// relative shock), `dσ = vol_abs`, `dr = discount_abs`, `db = carry_abs`. Rate
    /// sensitivity is kept first-order: rho convexity is negligible over a VaR-scale
    /// rate shock and the analytic/AAD set carries no second-order rate Greek, so
    /// adding a fake one would over-claim.
    #[must_use]
    pub fn taylor_pnl(&self, scenario: Scenario) -> f64 {
        let d_s = self.spot * scenario.spot_rel;
        let d_vol = scenario.vol_abs;
        self.delta_spot * d_s
            + 0.5 * self.gamma * d_s * d_s
            + self.vega * d_vol
            + 0.5 * self.volga * d_vol * d_vol
            + self.vanna * d_s * d_vol
            + self.discount_rho * scenario.discount_abs
            + self.carry_rho * scenario.carry_abs
    }
}

/// Compute every position's [`PositionSensitivity`] for a node through the seam,
/// one sweep/strip per position (O(positions) total) — the precompute step shared by
/// all scenarios in [`sensitivity_var_es`].
#[must_use]
pub fn node_sensitivities<P: CarryPricer>(
    pricer: &P,
    positions: &[PositionRisk],
) -> Vec<PositionSensitivity> {
    positions
        .iter()
        .map(|p| PositionSensitivity::from_position(pricer, p))
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
pub fn sensitivity_var_es<P: CarryPricer>(
    pricer: &P,
    positions: &[PositionRisk],
    scenarios: &[Scenario],
    alpha: f64,
) -> VarEs {
    if scenarios.is_empty() {
        return VarEs { var: 0.0, es: 0.0 };
    }
    // ONE sweep/strip per position through the seam, reused across every scenario.
    let sens = node_sensitivities(pricer, positions);
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
pub fn sbm_curvature_spot<P: CarryPricer>(pricer: &P, positions: &[PositionRisk], rw: f64) -> f64 {
    let (cvr_up, cvr_down) = vanilla_curvature_legs(pricer, positions, rw);
    cvr_up.max(cvr_down).max(0.0)
}

/// The vanilla node's FRTB-SbM curvature legs `(CVR_up, CVR_down)` along spot, BEFORE
/// the `max(·, ·, 0)` reduction — so a node containing both vanilla and exotic legs
/// can sum the two `(up, down)` pairs and take a single `max` over the WHOLE node
/// (curvature is non-additive in exactly this way: `max(Σ_vanilla up + Σ_exotic up,
/// Σ_vanilla down + Σ_exotic down, 0)`, not the sum of two independent `max`es).
#[must_use]
pub fn vanilla_curvature_legs<P: CarryPricer>(
    pricer: &P,
    positions: &[PositionRisk],
    rw: f64,
) -> (f64, f64) {
    // Reprice the node up and down by the relative spot shock, through the seam.
    let base = node_value(pricer, positions);
    let up = node_value_shocked(pricer, positions, 1.0 + rw);
    let down = node_value_shocked(pricer, positions, 1.0 - rw);
    // The convention-exact linear term, netted per position (see the doc note). The
    // leaf's spot delta is read through the seam (the asset's own ∂V/∂S), never an
    // FX proxy.
    let linear = positions
        .iter()
        .map(|p| {
            let delta_spot = pricer
                .price_greeks(p.option, &p.inputs)
                .map_or(0.0, |g| g.delta_spot);
            delta_spot * p.notional_base * rw * p.inputs.spot
        })
        .sum::<f64>();
    let cvr_up = -((up - base) - linear);
    let cvr_down = -((down - base) + linear);
    (cvr_up, cvr_down)
}

/// **Combined node VaR/ES** over a node's vanilla positions AND its exotic legs by
/// full bump-and-revalue (`docs/RISK-HIERARCHY.md` §2.5, exotic extension). Each
/// scenario reprices BOTH the vanilla legs (`celnet-vanilla`) and the exotic legs
/// (the real closed-form exotic pricer) and sums their P&L into one node loss, so an
/// exotic leg contributes its true tail risk — never a vanilla proxy, never excluded.
/// Reduces to [`historical_var_es`] exactly when there are no exotic legs.
#[must_use]
pub fn node_var_es_combined<P: CarryPricer>(
    pricer: &P,
    positions: &[PositionRisk],
    exotic_legs: &[crate::exotic::ExoticLeg],
    scenarios: &[Scenario],
    alpha: f64,
) -> VarEs {
    if scenarios.is_empty() {
        return VarEs { var: 0.0, es: 0.0 };
    }
    let mut pnl: Vec<f64> = scenarios
        .iter()
        .map(|s| node_pnl(pricer, positions, *s) + crate::exotic::exotic_node_pnl(exotic_legs, *s))
        .collect();
    quantile_var_es(&mut pnl, alpha)
}

/// **Combined node VaR/ES (scale path)**: the AAD Taylor lens over the vanilla legs
/// PLUS a full closed-form reprice of the exotic legs per scenario. The exotic legs
/// are repriced exactly (the closed forms are cheap and the second-order Taylor of a
/// barrier near its knock-out would be a poor approximation), so this lens stays
/// honest about the exotic tail; the vanilla legs keep the O(positions)-sweep
/// Taylor speed-up. Reduces to [`sensitivity_var_es`] exactly when there are no
/// exotic legs.
#[must_use]
pub fn node_var_es_sensitivity_combined<P: CarryPricer>(
    pricer: &P,
    positions: &[PositionRisk],
    exotic_legs: &[crate::exotic::ExoticLeg],
    scenarios: &[Scenario],
    alpha: f64,
) -> VarEs {
    if scenarios.is_empty() {
        return VarEs { var: 0.0, es: 0.0 };
    }
    let sens = node_sensitivities(pricer, positions);
    let mut pnl: Vec<f64> = scenarios
        .iter()
        .map(|s| {
            let vanilla: f64 = sens.iter().map(|p| p.taylor_pnl(*s)).sum();
            vanilla + crate::exotic::exotic_node_pnl(exotic_legs, *s)
        })
        .collect();
    quantile_var_es(&mut pnl, alpha)
}

/// **Combined node FRTB-SbM spot curvature** over vanilla + exotic legs: sum the
/// vanilla `(up, down)` legs and the exotic `(up, down)` legs, THEN take the single
/// node `max(Σ up, Σ down, 0)`. Curvature is non-additive precisely because of this
/// `max`, so the two leg classes must be combined before the reduction (taking a
/// `max` per class and summing would over-count). Reduces to [`sbm_curvature_spot`]
/// exactly when there are no exotic legs.
#[must_use]
pub fn sbm_curvature_spot_combined<P: CarryPricer>(
    pricer: &P,
    positions: &[PositionRisk],
    exotic_legs: &[crate::exotic::ExoticLeg],
    rw: f64,
) -> f64 {
    let (v_up, v_down) = vanilla_curvature_legs(pricer, positions, rw);
    let (e_up, e_down) = crate::exotic::exotic_curvature_legs(exotic_legs, rw);
    (v_up + e_up).max(v_down + e_down).max(0.0)
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

/// The base (unshocked) value of a node in quote ccy (Σ per-unit PV × notional),
/// repriced through the seam.
fn node_value<P: CarryPricer>(pricer: &P, positions: &[PositionRisk]) -> f64 {
    positions
        .iter()
        .map(|p| {
            pricer
                .price(p.option, &p.inputs)
                .map_or(0.0, |v| v * p.notional_base)
        })
        .sum()
}

/// The node value after multiplying every position's spot by `spot_mult` (carry,
/// vol and time held fixed), repriced through the seam. The shocked input keeps the
/// position's own underlying/carry — no match on the underlying.
fn node_value_shocked<P: CarryPricer>(
    pricer: &P,
    positions: &[PositionRisk],
    spot_mult: f64,
) -> f64 {
    positions
        .iter()
        .map(|p| {
            let shocked = CarryInputs::new(
                p.inputs.spot * spot_mult,
                p.inputs.strike,
                p.inputs.vol,
                p.inputs.t,
                p.inputs.underlying.clone(),
                p.inputs.carry,
            );
            pricer
                .price(p.option, &shocked)
                .map_or(0.0, |v| v * p.notional_base)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_risk_normalize::AssetPricer;
    use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn pos(opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
        PositionRisk::fx(
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
                    discount_abs: 0.0,
                    carry_abs: 0.0,
                });
            }
        }
        v
    }

    /// **The fast lens genuinely consumes `adjoint_greeks`.** The per-position
    /// sensitivity profile (FX) must equal `celnet_vanilla::adjoint_greeks` scaled by
    /// notional — bit-identical, proving the lens is built on the real reverse-mode
    /// AAD sweep (not the analytic `greeks`, not finite differences). The rate block
    /// is in the carry basis: `discount_rho = rho_dom·n`, `carry_rho = −rho_for·n`.
    #[test]
    fn sensitivity_profile_is_adjoint_greeks_scaled_by_notional() {
        let inputs = VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02);
        let notional = 7_500_000.0;
        let p = pos(OptionType::Call, notional, inputs);
        let s = PositionSensitivity::from_position(&AssetPricer, &p);
        let g = adjoint_greeks(OptionType::Call, &inputs);
        assert_eq!(s.delta_spot.to_bits(), (g.delta_spot * notional).to_bits());
        assert_eq!(s.gamma.to_bits(), (g.gamma * notional).to_bits());
        assert_eq!(s.vega.to_bits(), (g.vega * notional).to_bits());
        assert_eq!(s.volga.to_bits(), (g.volga * notional).to_bits());
        assert_eq!(s.vanna.to_bits(), (g.vanna * notional).to_bits());
        assert_eq!(s.discount_rho.to_bits(), (g.rho_dom * notional).to_bits());
        assert_eq!(s.carry_rho.to_bits(), (-g.rho_for * notional).to_bits());
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
        let oracle = historical_var_es(&AssetPricer, &node, &scen, 0.99);
        let fast = sensitivity_var_es(&AssetPricer, &node, &scen, 0.99);
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
                        discount_abs: 0.0,
                        carry_abs: 0.0,
                    });
                }
            }
            let o = historical_var_es(&AssetPricer, &node, &scen, 0.99).var;
            let f = sensitivity_var_es(&AssetPricer, &node, &scen, 0.99).var;
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
            let o = historical_var_es(&AssetPricer, &node, &scen, 0.99).var;
            let f = sensitivity_var_es(&AssetPricer, &node, &scen, 0.99).var;
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
        let a = sensitivity_var_es(&AssetPricer, &node, &scen, 0.975);
        let b = sensitivity_var_es(&AssetPricer, &node, &scen, 0.975);
        assert_eq!(a.var.to_bits(), b.var.to_bits());
        assert_eq!(a.es.to_bits(), b.es.to_bits());
    }

    /// **The carry-shock terms expand correctly.** A pure discount-rate scenario's
    /// node Taylor P&L equals `discount_rho · dr` summed over positions — first-order,
    /// as documented (no fake rate convexity). A pure carry shock expands by
    /// `carry_rho · db`.
    #[test]
    fn rate_shock_expansion_is_first_order_rho() {
        let p = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let s = PositionSensitivity::from_position(&AssetPricer, &p);
        let dr = 0.0010; // +10bp discount.
        let discount_only = Scenario {
            spot_rel: 0.0,
            vol_abs: 0.0,
            discount_abs: dr,
            carry_abs: 0.0,
        };
        assert!(is_close(
            s.taylor_pnl(discount_only),
            s.discount_rho * dr,
            1e-12,
            1e-9
        ));
        let db = 0.0007; // +7bp carry.
        let carry_only = Scenario {
            spot_rel: 0.0,
            vol_abs: 0.0,
            discount_abs: 0.0,
            carry_abs: db,
        };
        assert!(is_close(
            s.taylor_pnl(carry_only),
            s.carry_rho * db,
            1e-12,
            1e-9
        ));
    }
}
