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
        // The GPU scenario kernel is the FX/metal GBM path (the only asset class with
        // a wired batched Monte-Carlo kernel today); a non-FX position lowers to no
        // FX inputs and is skipped — its batched-grid kernel is a separate, named GPU
        // workload (`docs/GPU-AT-SCALE-PLAN.md`), not faked here. The discriminant is
        // read by the leaf's own FX lowering, not by an aggregation-loop match.
        let Ok(i) = celnet_core::carry::fx_vanilla_inputs(&p.inputs) else {
            continue;
        };
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
                .filter_map(|p| {
                    // FX/metal GBM oracle grid (the GPU kernel's reference); a non-FX
                    // position has no FX lowering and is skipped (same scope as the GPU
                    // path above).
                    let base = celnet_core::carry::fx_vanilla_inputs(&p.inputs).ok()?;
                    let s = VanillaInputs::new(
                        base.spot * sm,
                        base.strike,
                        base.vol + vb,
                        base.t,
                        base.r_dom,
                        base.r_for,
                    );
                    Some(price(p.option, &s) * p.notional_base)
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
        PositionRisk::fx(
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

    /// **Asymmetric-grid indexing is row-major `i·n_vol + j` exactly**: on a 2×3
    /// grid every node PV is pinned against a direct closed-form evaluation at its
    /// own `(spot_mult, vol_bump)` — an index-arithmetic mutant (`*`→`+`, swapped
    /// strides) relocates off-diagonal nodes and is caught because all six nodes
    /// carry distinct shocks (and `n_spot ≠ n_vol`).
    #[test]
    fn asymmetric_grid_indexing_is_row_major() {
        let positions = [pos(
            OptionType::Call,
            3_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 0.5, 0.03, 0.01),
        )];
        let axes = ScenarioAxes::new(vec![0.98, 1.03], vec![-0.005, 0.0, 0.0125]);
        let grid = analytic_pv_grid(&positions, &axes);
        assert_eq!(grid.n_spot, 2);
        assert_eq!(grid.n_vol, 3);
        assert_eq!(grid.pv.len(), 6);
        for (i, &sm) in axes.spot_mult.iter().enumerate() {
            for (j, &vb) in axes.vol_bump.iter().enumerate() {
                let want: f64 = positions
                    .iter()
                    .map(|p| {
                        let b = celnet_core::carry::fx_vanilla_inputs(&p.inputs).unwrap();
                        let s = VanillaInputs::new(
                            b.spot * f64::from(sm),
                            b.strike,
                            b.vol + f64::from(vb),
                            b.t,
                            b.r_dom,
                            b.r_for,
                        );
                        price(p.option, &s) * p.notional_base
                    })
                    .sum();
                let got = grid.pv(i as u32, j as u32);
                assert!(
                    is_close(got, want, 1e-12, 1e-6),
                    "node ({i},{j}): grid {got} vs direct {want}"
                );
                // std_err is uniformly zero on the exact path, via BOTH accessors.
                assert_eq!(
                    grid.std_err(i as u32, j as u32).to_bits(),
                    0.0_f64.to_bits()
                );
            }
        }
        // All six nodes are distinct (the pin genuinely constrains the layout).
        for a in 0..grid.pv.len() {
            for b in (a + 1)..grid.pv.len() {
                assert_ne!(grid.pv[a].to_bits(), grid.pv[b].to_bits());
            }
        }
        assert!(!grid.on_gpu, "the analytic oracle is the CPU path");
    }

    /// **The reconciliation band logic is exact**: a corrupted node fails with its
    /// own coordinates and the `k·σ + abs` band; a within-band deviation passes and
    /// is reported as the worst observation; the band genuinely scales with `k`
    /// when `std_err > 0`.
    #[test]
    fn reconciliation_band_detects_and_reports_exactly() {
        let positions = [pos(
            OptionType::Put,
            2_000_000.0,
            VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.03, 0.01),
        )];
        let axes = ScenarioAxes::ladders(1, 0.01, 1, 0.005); // 3×3
        let exact = analytic_pv_grid(&positions, &axes);

        // Corrupt node (1, 2) by +0.5 on the exact path (std_err = 0, abs = 1e-6):
        // must fail AT that node with d = 0.5 and band = 1e-6.
        let mut bad = exact.clone();
        bad.pv[5] += 0.5; // flat index 5 = row 1 · n_vol 3 + col 2
        let err = bad
            .reconciles_to_analytic(&positions, &axes, 6.0, 1e-6)
            .expect_err("a 0.5 deviation must breach a 1e-6 band");
        assert_eq!((err.0, err.1), (1, 2), "must report the offending node");
        assert!(is_close(err.2, 0.5, 1e-9, 1e-12), "|Δ| = 0.5");
        assert!(is_close(err.3, 1e-6, 1e-9, 1e-18), "band = k·0 + abs");

        // A within-band deviation passes and is the reported worst. (The recovered
        // |Δ| is 5e-7 up to the rounding of `pv + 5e-7` at PV scale ~1e4, i.e. a
        // few e-12 — far above a broken-worst-tracking 0.0 and far below the band.)
        let mut close = exact.clone();
        close.pv[4] += 5e-7;
        let (worst_d, worst_se) = close
            .reconciles_to_analytic(&positions, &axes, 6.0, 1e-6)
            .expect("5e-7 < 1e-6 must reconcile");
        assert!(is_close(worst_d, 5e-7, 2e-2, 1e-11), "worst |Δ| {worst_d}");
        assert_eq!(worst_se.to_bits(), 0.0_f64.to_bits());

        // With std_err > 0 the band is k·σ + abs: a 3σ deviation passes at k = 5
        // and fails at k = 2 (the k-term is load-bearing).
        let mut mc = exact.clone();
        mc.pv[0] += 3.0;
        mc.std_err = vec![1.0; mc.pv.len()];
        assert!(
            mc.reconciles_to_analytic(&positions, &axes, 5.0, 0.1)
                .is_ok()
        );
        let err2 = mc
            .reconciles_to_analytic(&positions, &axes, 2.0, 0.1)
            .expect_err("3σ must breach a 2σ+0.1 band");
        assert_eq!((err2.0, err2.1), (0, 0));
        assert!(is_close(err2.3, 2.1, 1e-12, 1e-12), "band = 2·1 + 0.1");
    }

    /// **The batched path's std_err propagates in quadrature and the GPU flag is
    /// honest**: the per-position standard errors add as variances (two identical
    /// positions ⇒ exactly √2× the one-position σ per node), `on_gpu` mirrors the
    /// pricer, and the same seed reproduces the grid bit-for-bit.
    #[test]
    fn batched_grid_std_err_quadrature_and_determinism() {
        let one = [pos(
            OptionType::Call,
            1_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 0.5, 0.03, 0.01),
        )];
        let two = [one[0].clone(), one[0].clone()];
        let axes = ScenarioAxes::ladders(1, 0.01, 1, 0.005);
        let pricer = ScenarioPricer::new();
        let g1 = gpu_pv_grid(&pricer, &one, &axes, 1 << 12, 0xFEED);
        let g2 = gpu_pv_grid(&pricer, &two, &axes, 1 << 12, 0xFEED);
        assert_eq!(g1.on_gpu, pricer.is_gpu());
        for idx in 0..g1.pv.len() {
            // PV doubles exactly (same CRN bitstream per position).
            assert!(is_close(g2.pv[idx], 2.0 * g1.pv[idx], 1e-12, 1e-9));
            // σ adds in quadrature: √(σ² + σ²) = √2·σ.
            assert!(is_close(
                g2.std_err[idx],
                core::f64::consts::SQRT_2 * g1.std_err[idx],
                1e-12,
                1e-12
            ));
        }
        // Same seed ⇒ bit-identical grid (the documented reproducibility contract).
        let g1b = gpu_pv_grid(&pricer, &one, &axes, 1 << 12, 0xFEED);
        for idx in 0..g1.pv.len() {
            assert_eq!(g1.pv[idx].to_bits(), g1b.pv[idx].to_bits());
            assert_eq!(g1.std_err[idx].to_bits(), g1b.std_err[idx].to_bits());
        }
        // A non-FX position is skipped (no silent proxy): the grid is unchanged.
        let mut with_eq = two.to_vec();
        let u = celnet_types::Underlying::Equity(celnet_types::EquityRef::new(
            celnet_types::Symbol::new("ACME", ""),
            Ccy::USD,
        ));
        with_eq.push(PositionRisk::carry(
            u.clone(),
            OptionType::Call,
            1_000.0,
            celnet_core::carry::CarryInputs::new(
                100.0,
                105.0,
                0.20,
                1.0,
                u,
                celnet_types::Carry::CostOfCarry { r: 0.03, b: 0.01 },
            ),
        ));
        let g3 = gpu_pv_grid(&pricer, &with_eq, &axes, 1 << 12, 0xFEED);
        for idx in 0..g2.pv.len() {
            assert_eq!(g3.pv[idx].to_bits(), g2.pv[idx].to_bits());
        }
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
            .map(|p| {
                let vi = celnet_core::carry::fx_vanilla_inputs(&p.inputs).unwrap();
                price(p.option, &vi) * p.notional_base
            })
            .sum();
        assert!(
            is_close(centre, base, 1e-12, 1e-6),
            "centre grid node {centre} must equal base PV {base}"
        );
    }
}
