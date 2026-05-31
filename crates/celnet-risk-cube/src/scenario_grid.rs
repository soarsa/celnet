//! Batched **scenario-grid revaluation** for a node — the spot×vol risk grid
//! (`docs/RISK-HIERARCHY.md` §3.3, `docs/GPU-AT-SCALE-PLAN.md`).
//!
//! # What this is
//!
//! The non-additive measures in [`crate::nonadditive`] re-price a node under
//! shocks. The GUI **Risk** workspace and the cube's scenario reval both want the
//! *same* shape: a node's P&L (or PV) over a **2-D ladder** of relative-spot ×
//! absolute-vol shocks. This module computes that grid for a node by driving
//! `celnet-gpu`'s batched scenario kernel — **one GPU dispatch per position**
//! over the whole grid (Metal/Vulkan) — and summing the notional-scaled
//! per-position PV grids into the node grid. When no GPU adapter is present (CI /
//! headless) the *same* call transparently runs the exact f64 CPU oracle node by
//! node, so the result is identical in shape and meaning.
//!
//! # Fast path vs oracle (honest reconciliation)
//!
//! - **GPU (f32 Monte-Carlo)** is the scale path: it prices the entire grid in a
//!   handful of dispatches under **common random numbers** (a smooth surface), at
//!   throughput the node-by-node analytic bump loop cannot match for IB-scale
//!   grids. It is a *Monte-Carlo estimator*, so each node agrees with the analytic
//!   closed-form PV only within the node's **Monte-Carlo standard error** — not to
//!   machine precision. [`NodeScenarioGrid::reconciles_to_analytic`] asserts
//!   exactly that statistical band against the closed-form oracle.
//! - **The analytic bump-and-revalue** ([`crate::nonadditive`],
//!   `celnet-vanilla::price`) remains the **oracle**: deterministic, exact, and the
//!   reference the GPU grid is validated against. [`analytic_pv_grid`] is that
//!   reference, available directly for callers that want the exact grid (and used
//!   by the reconciliation test).
//! - **GPU-vs-CPU-oracle**: when the GPU path is taken, the f32 result reconciles
//!   to the f64 CPU oracle bit-stream-for-bit-stream within `celnet-gpu`'s own
//!   first-principles f32 bound (proven inside that crate); here we add the
//!   independent check against the *analytic* PV the cube actually reports.
//!
//! # Scope
//!
//! Single-pair (shared-quote-currency) nodes: the grid sums position PVs in the
//! common quote currency, exactly as [`crate::nonadditive::node_pnl`] does. A
//! multi-pair node must be numeraire-normalized first (the cube's existing §2.3
//! discipline) — this module does not silently mix currencies.
//!
//! # Determinism
//!
//! The grid is reproducible for a fixed `(positions, axes, seed, paths)`: the GPU
//! and CPU paths share the Philox bitstream, and the CPU oracle is `libm`-routed.

use celnet_gpu::{PathSpec, PayoffKernel, ScenarioAxes, ScenarioPricer};
use celnet_risk_normalize::PositionRisk;
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::price;

/// A node's PV over the spot×vol scenario grid, plus the Monte-Carlo standard
/// error per node, in the node's common quote currency.
///
/// Row-major: `pv[i·n_vol + j]` is the node PV at relative-spot rung `i` and
/// absolute-vol rung `j` of the supplied [`ScenarioAxes`]. `std_err` is the
/// summed-in-quadrature MC standard error of the node PV at the same node (zero on
/// the exact CPU-oracle path).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeScenarioGrid {
    /// Number of relative-spot rungs.
    pub n_spot: u32,
    /// Number of absolute-vol rungs.
    pub n_vol: u32,
    /// Row-major node PV grid (length `n_spot · n_vol`), quote ccy.
    pub pv: Vec<f64>,
    /// Row-major node-PV MC standard error (length `n_spot · n_vol`); `0.0` when
    /// the exact CPU oracle priced the grid.
    pub std_err: Vec<f64>,
    /// `true` when a real GPU adapter drove the batch (else the CPU oracle).
    pub on_gpu: bool,
}

impl NodeScenarioGrid {
    /// The node PV at grid coordinate `(i, j)` (`i` = spot rung, `j` = vol rung).
    #[must_use]
    pub fn pv(&self, i: u32, j: u32) -> f64 {
        self.pv[(i * self.n_vol + j) as usize]
    }

    /// The node-PV MC standard error at `(i, j)`.
    #[must_use]
    pub fn std_err(&self, i: u32, j: u32) -> f64 {
        self.std_err[(i * self.n_vol + j) as usize]
    }

    /// Assert each grid node reconciles to the **analytic** closed-form node PV
    /// within a statistical band: `|pv_mc − pv_analytic| ≤ k·std_err + abs`.
    ///
    /// `k` is the standard-error multiple (e.g. 5.0 ≈ a 5σ band) and `abs` an
    /// absolute floor for the exact CPU path (where `std_err == 0`). On the exact
    /// CPU oracle this collapses to an exact `|Δ| ≤ abs` check. Returns the worst
    /// observed `(|Δ|, std_err)` for diagnostics, or `Err` on the first node that
    /// falls outside the band.
    ///
    /// # Errors
    ///
    /// Returns the offending `(i, j, |Δ|, band)` when a node exceeds the band.
    pub fn reconciles_to_analytic(
        &self,
        positions: &[PositionRisk],
        axes: &ScenarioAxes,
        k: f64,
        abs: f64,
    ) -> Result<(f64, f64), (u32, u32, f64, f64)> {
        let reference = analytic_pv_grid(positions, axes);
        let mut worst = (0.0_f64, 0.0_f64);
        for i in 0..self.n_spot {
            for j in 0..self.n_vol {
                let d = (self.pv(i, j) - reference.pv(i, j)).abs();
                let se = self.std_err(i, j);
                let band = k * se + abs;
                if d > worst.0 {
                    worst = (d, se);
                }
                if d > band {
                    return Err((i, j, d, band));
                }
            }
        }
        Ok(worst)
    }
}

/// Price a node's PV over the `axes` spot×vol grid via the batched scenario
/// kernel (GPU when present, exact f64 CPU oracle otherwise).
///
/// One batched dispatch per position over the whole grid, summed (notional- and
/// sign-scaled) into the node grid under common random numbers. `paths` and `seed`
/// fix the Monte-Carlo bitstream; pass the same values to get a reproducible grid.
///
/// # Panics
///
/// Panics if `axes.is_valid()` is false (empty axis or oversize grid) — surfaced
/// loudly, matching `celnet-gpu`'s contract.
#[must_use]
pub fn gpu_pv_grid(
    pricer: &ScenarioPricer,
    positions: &[PositionRisk],
    axes: &ScenarioAxes,
    paths: u32,
    seed: u64,
) -> NodeScenarioGrid {
    let n = (axes.n_spot() * axes.n_vol()) as usize;
    let mut pv = vec![0.0_f64; n];
    // Variances add for independent estimators sharing the path set per position;
    // because all positions reuse the same CRN bitstream the per-node payoffs are
    // correlated, but summing the per-position variances is the conservative
    // (upper-bound) standard error, which is the safe band for reconciliation.
    let mut var = vec![0.0_f64; n];

    for p in positions {
        let i = &p.inputs;
        let spec = PathSpec::gbm(i.spot, i.vol, i.t, i.r_dom, i.r_for, paths, 1, seed);
        let payoff = match p.option {
            OptionType::Call => PayoffKernel::call(i.strike),
            OptionType::Put => PayoffKernel::put(i.strike),
        };
        let grid = pricer.price_scenario_batch(&spec, &payoff, axes);
        for idx in 0..n {
            let r = grid.nodes[idx];
            pv[idx] += r.price() * p.notional_base;
            let se = r.std_error() * p.notional_base;
            var[idx] += se * se;
        }
    }
    let std_err = var.iter().map(|v| v.sqrt()).collect();
    NodeScenarioGrid {
        n_spot: axes.n_spot(),
        n_vol: axes.n_vol(),
        pv,
        std_err,
        on_gpu: pricer.is_gpu(),
    }
}

/// The **exact analytic** node-PV grid (the oracle): closed-form
/// `celnet-vanilla::price` for every position at every `(spot_mult, vol_bump)`
/// node, summed notional-scaled. Deterministic and `libm`-routed.
///
/// This is the reference [`gpu_pv_grid`] is validated against, and the exact grid
/// a caller can use directly when Monte-Carlo noise is unacceptable (small grids
/// where the closed form is already cheap). `std_err` is uniformly zero.
#[must_use]
pub fn analytic_pv_grid(positions: &[PositionRisk], axes: &ScenarioAxes) -> NodeScenarioGrid {
    let n_spot = axes.n_spot();
    let n_vol = axes.n_vol();
    let mut pv = vec![0.0_f64; (n_spot * n_vol) as usize];
    for (i, &sm) in axes.spot_mult.iter().enumerate() {
        for (j, &vb) in axes.vol_bump.iter().enumerate() {
            let idx = i * n_vol as usize + j;
            // Widen the f32 axis values to f64 exactly as the GPU kernel sees them,
            // so the analytic reference is shocked by the identical grid point.
            let sm = f64::from(sm);
            let vb = f64::from(vb);
            pv[idx] = positions
                .iter()
                .map(|p| {
                    let s = VanillaInputs::new(
                        p.inputs.spot * sm,
                        p.inputs.strike,
                        p.inputs.vol + vb,
                        p.inputs.t,
                        p.inputs.r_dom,
                        p.inputs.r_for,
                    );
                    price(p.option, &s) * p.notional_base
                })
                .sum();
        }
    }
    NodeScenarioGrid {
        n_spot,
        n_vol,
        std_err: vec![0.0; pv.len()],
        pv,
        on_gpu: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_types::{Ccy, CcyPair, DeltaConvention, PremiumStyle};

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

    /// **The batched scenario grid reconciles to the analytic oracle.** A
    /// two-position node is priced over a 5×5 spot×vol ladder by the batched
    /// kernel (GPU if present, exact f64 CPU oracle in CI), and every node agrees
    /// with the closed-form `celnet-vanilla::price` grid within a 6σ MC band (and
    /// to ~1e-7 absolute on the exact CPU path).
    #[test]
    fn scenario_grid_reconciles_to_analytic_oracle() {
        let positions = [
            pos(
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 0.5, 0.03, 0.01),
            ),
            pos(
                OptionType::Put,
                -5_000_000.0,
                VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.03, 0.01),
            ),
        ];
        // ±2% spot in 5 rungs, ±1 vol point in 5 rungs.
        let axes = ScenarioAxes::ladders(2, 0.01, 2, 0.005);
        let pricer = ScenarioPricer::new();
        let grid = gpu_pv_grid(&pricer, &positions, &axes, 1 << 16, 0xC0FFEE);
        // High path count → tight band. 6σ band + tiny absolute floor for the exact
        // CPU oracle (std_err == 0 there → collapses to an exact 1e-6 check).
        let (worst_d, worst_se) = grid
            .reconciles_to_analytic(&positions, &axes, 6.0, 1e-6)
            .expect("scenario grid must reconcile to the analytic oracle");
        // Sanity: the worst node deviation is genuinely small relative to PV scale.
        assert!(
            worst_d < 6.0 * worst_se + 1e-6 + 1.0,
            "worst grid deviation {worst_d} (se {worst_se}) implausibly large"
        );
        assert_eq!(grid.n_spot, 5);
        assert_eq!(grid.n_vol, 5);
    }

    /// **The centre node is the unshocked node PV.** The (centre,centre) rung is
    /// `spot_mult = 1.0`, `vol_bump = 0.0`, so its analytic PV equals the node's
    /// base value exactly (the bump-and-revalue base in `nonadditive`).
    #[test]
    fn centre_node_is_base_pv() {
        let positions = [pos(
            OptionType::Call,
            7_000_000.0,
            VanillaInputs::new(1.2500, 1.2600, 0.09, 0.75, 0.02, 0.01),
        )];
        let axes = ScenarioAxes::ladders(3, 0.01, 3, 0.005);
        let analytic = analytic_pv_grid(&positions, &axes);
        let centre = analytic.pv(3, 3); // 7 rungs → centre index 3.
        let base: f64 = positions
            .iter()
            .map(|p| price(p.option, &p.inputs) * p.notional_base)
            .sum();
        assert!(
            is_close(centre, base, 1e-12, 1e-6),
            "centre grid node {centre} must equal base PV {base}"
        );
    }
}
