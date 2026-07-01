//! Celnet risk-cube **reprice acceleration** — ADR-0013 Lane 2.
//!
//! # What this crate is
//!
//! The GPU-backed implementation of the [`celnet_risk_cube::ScenarioReprice`] seam:
//! it **wires** the already-built, oracle-validated `celnet-gpu` closed-form batch
//! pipeline ([`celnet_gpu::BatchPricer`]) into the risk cube's non-additive
//! bump-and-revalue reprice loop (`celnet_risk_cube::nonadditive`,
//! `O(Npos × Nscen)` — the portfolio-scale throughput bottleneck). It **does not
//! replicate** the GPU numerics; it maps the cube's carry-tagged positions onto the
//! batch kernel and reduces the result through the cube's own reduction.
//!
//! # Why this is a separate crate (the dependency-direction decision)
//!
//! `celnet-risk-cube` deliberately depends on **neither** `celnet-gpu` nor any
//! heavy pricer (arch-program item E; `docs/INTERFACES.md` one-way edges; the gate
//! `cargo tree -e normal -p celnet-risk-cube` lists no `celnet-gpu`): the lean OLAP
//! crate injects heavy backends through seams instead of pulling the `wgpu` stack
//! into every downstream that builds it. `risk-cube → gpu` is *acyclic*, but adding
//! that edge would violate item E and bloat the cube, so — exactly as ADR-0013 /
//! P1-LANES sanction ("put the wiring one layer up") — the GPU-consuming backend
//! lives here: **`celnet-risk-accel → {celnet-risk-cube, celnet-gpu}` is a clean
//! one-way edge** (neither depends on this crate). The cube exposes only the lean
//! seam; this crate provides the accelerator.
//!
//! # The exact-reval invariant, and where the GPU actually runs
//!
//! The cube's VaR/ES/curvature is a **machine-exact** measure: its oracle prices
//! every leg with `celnet-vanilla`'s `libm::erfc` closed form in f64, gated
//! bit-for-bit against the QuantLib golden. The `celnet-gpu` batch kernel is **f32**
//! (Metal has no `f64`) with an **Abramowitz-&-Stegun `erf`** — bounded by
//! [`celnet_gpu::derived_batch_bound`] (f32 round-off, ~few×1e-6) +
//! [`celnet_gpu::as_erf_price_bound`] (A&S algorithmic, ~1e-7 vs `libm::erfc`).
//! Those are a **hard floor far above 1e-12**, so routing the *exact* VaR through
//! the GPU batch would inject ~1e-7 approximation noise into a machine-exact reval —
//! the **same** regression `celnet_risk_cube::nonadditive` refuses for Monte-Carlo
//! estimator noise. Therefore, honouring the seam's precision contract:
//!
//! * **Exact path** ([`GpuBatchReprice`] as a [`ScenarioReprice`]): always the
//!   CPU-f64 [`celnet_risk_cube::SerialReprice`] oracle — **≤1e-12**, byte-identical
//!   to the cube's serial reprice. On an f32 device (this Apple-M4/Metal host, and
//!   headless CI) it **degrades to CPU cleanly**; an f64-native device would earn an
//!   on-device exact kernel only behind the same ≤1e-12 parity gate.
//! * **Screening path** ([`GpuBatchReprice::screening_node_pnls`] /
//!   [`GpuBatchReprice::screening_var_es`]): the genuine GPU dispatch — a large,
//!   all-FX node's `base + shocked` grid packed into **one** `BatchPricer` command
//!   buffer (the "coalesced AoS" SOTA pattern; `docs/GPU-AT-SCALE-PLAN.md` Workload
//!   A/G2), for the fast **approximate** portfolio VaR a pre-trade / what-if lens
//!   wants. It is reconciled to the CPU-f64 oracle within the **derived**
//!   [`screening_node_pnl_bound`] — never claimed at ≤1e-12.
//!
//! A **batch-size threshold** ([`RepriceConfig::gpu_min_batch`], ADR-0013
//! decision 1/6 — config-driven, not hardcoded in the loop) keeps a small reprice on
//! the CPU so it never pays a GPU round-trip; only a batch that clears the threshold
//! (and is all-FX, with an adapter present) dispatches to the GPU.
//!
//! Nothing here touches the per-tick `price_instrument` path or the pinned zero-alloc
//! pricing thread (ADR-0013 hot-core embargo): this is the risk / batch tier only.

#![forbid(unsafe_code)]

use celnet_core::{CarryInputs, CarryPricer, fx_vanilla_inputs};
use celnet_gpu::{BatchInstrument, BatchPricer, as_erf_price_bound, derived_batch_bound};
use celnet_risk_cube::{Scenario, ScenarioReprice, SerialReprice, VarEs, historical_var_es_via};
use celnet_risk_normalize::PositionRisk;
use celnet_types::OptionType;

/// Typed knobs for the reprice accelerator (ADR-0013 decision 6: a declarative,
/// boot-time selection, never a hardcoded constant on the hot loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepriceConfig {
    /// The minimum **flattened** batch size (`Npos × (Nscen + 1)`, i.e. the base
    /// prices plus every shocked reprice) below which the screening path stays on
    /// the CPU — a GPU command-buffer round-trip only pays off once the batch is
    /// wide enough to amortize the dispatch. Tunable per deployment/device.
    pub gpu_min_batch: usize,
}

impl Default for RepriceConfig {
    fn default() -> Self {
        // ~1k flattened instruments: comfortably past the dispatch break-even for a
        // GK closed form on a discrete GPU while still routing genuinely small
        // (single-desk / few-scenario) repices to the CPU. A tuned default, not a
        // hard rule — every field is overridable.
        Self {
            gpu_min_batch: 1024,
        }
    }
}

/// The GPU-backed reprice backend for `celnet-risk-cube`'s non-additive loop.
///
/// Holds the CPU-f64 oracle (`&P: CarryPricer`, the exact path + clean fallback),
/// the `celnet-gpu` [`BatchPricer`] (adapter probe + its own CPU fallback), and the
/// [`RepriceConfig`]. As a [`ScenarioReprice`] it serves the **exact ≤1e-12 path**;
/// [`Self::screening_node_pnls`] / [`Self::screening_var_es`] are the f32 GPU
/// screening lens. See the crate docs for the precision split.
pub struct GpuBatchReprice<'p, P: CarryPricer> {
    cpu: &'p P,
    gpu: BatchPricer,
    config: RepriceConfig,
}

impl<'p, P: CarryPricer> GpuBatchReprice<'p, P> {
    /// Build a reprice backend over the exact CPU pricer, probing for a GPU adapter
    /// (falling back to the CPU batch path when none is present, so it runs
    /// headless). Construction is off any hot path.
    #[must_use]
    pub fn new(cpu: &'p P) -> Self {
        Self {
            cpu,
            gpu: BatchPricer::new(),
            config: RepriceConfig::default(),
        }
    }

    /// Build with an explicit [`RepriceConfig`] (e.g. a tuned or test threshold).
    #[must_use]
    pub fn with_config(cpu: &'p P, config: RepriceConfig) -> Self {
        Self {
            cpu,
            gpu: BatchPricer::new(),
            config,
        }
    }

    /// `true` when a real GPU adapter is driving the batch closed-form pipeline
    /// (mirrors [`BatchPricer::is_gpu`]). Even when `true`, the **exact** path stays
    /// on the CPU f64 oracle (an f32 GPU cannot certify ≤1e-12); the adapter drives
    /// the **screening** lens.
    #[must_use]
    pub fn is_gpu(&self) -> bool {
        self.gpu.is_gpu()
    }

    /// Human-readable backend identity (mirrors [`BatchPricer::label`]).
    #[must_use]
    pub fn label(&self) -> String {
        self.gpu.label()
    }

    /// The active configuration.
    #[must_use]
    pub const fn config(&self) -> RepriceConfig {
        self.config
    }

    /// Whether a screening reprice of this exact shape routes to the GPU: an adapter
    /// is present **and** every position lowers to the FX two-rate GK batch **and**
    /// the flattened batch clears [`RepriceConfig::gpu_min_batch`]. When `false` the
    /// screening path is the CPU-f64 oracle (byte-identical to the exact path).
    #[must_use]
    pub fn would_screen_on_gpu(&self, positions: &[PositionRisk], scenarios: &[Scenario]) -> bool {
        if positions.is_empty() || scenarios.is_empty() {
            return false;
        }
        let all_fx = positions
            .iter()
            .all(|p| fx_batch_instrument(p.option, &p.inputs).is_some());
        let flat = positions
            .len()
            .saturating_mul(scenarios.len().saturating_add(1));
        all_fx && self.is_gpu() && flat >= self.config.gpu_min_batch
    }

    /// Per-scenario node P&L via the **GPU screening lens** (f32 closed form) when
    /// [`Self::would_screen_on_gpu`], otherwise the exact CPU-f64 oracle.
    ///
    /// This is the genuine `celnet-gpu` dispatch: the node's `base` prices (one per
    /// position) and its `Npos × Nscen` shocked reprices are packed into **one**
    /// `BatchPricer` command buffer, then reduced to a per-scenario node P&L. The
    /// result is **f32 grade** — reconciled to the CPU-f64 oracle within
    /// [`screening_node_pnl_bound`], NOT the ≤1e-12 exact contract. Use the exact
    /// path ([`ScenarioReprice::node_pnls`]) when an exact tail is required.
    #[must_use]
    pub fn screening_node_pnls(
        &self,
        positions: &[PositionRisk],
        scenarios: &[Scenario],
    ) -> Vec<f64> {
        if !self.would_screen_on_gpu(positions, scenarios) {
            // Degrade to the exact CPU-f64 oracle — small batch, no adapter, or a
            // cross-asset node the FX GK batch does not lower. Byte-identical to the
            // cube's serial reprice.
            return SerialReprice(self.cpu).node_pnls(positions, scenarios);
        }
        self.gpu_batch_node_pnls(positions, scenarios)
    }

    /// The **screening** VaR/ES — the cube's exact quantile/tail reduction over the
    /// f32 screening node P&L (via [`historical_var_es_via`]). A fast *approximate*
    /// portfolio VaR (reconciled within [`screening_node_pnl_bound`]); the exact
    /// tail is [`historical_var_es_via`] over `self` (the [`ScenarioReprice`] path).
    #[must_use]
    pub fn screening_var_es(
        &self,
        positions: &[PositionRisk],
        scenarios: &[Scenario],
        alpha: f64,
    ) -> VarEs {
        historical_var_es_via(&ScreeningView(self), positions, scenarios, alpha)
    }

    /// One-dispatch GPU reduction: `[base_0..base_{P-1}, shocked_{s0}..shocked_{sN}]`
    /// priced by the batch closed form, then node P&L per scenario. Callers guarantee
    /// (via [`Self::would_screen_on_gpu`]) that every position lowers to FX and the
    /// batch is non-empty.
    fn gpu_batch_node_pnls(&self, positions: &[PositionRisk], scenarios: &[Scenario]) -> Vec<f64> {
        let np = positions.len();
        let mut insts = Vec::with_capacity(np.saturating_mul(scenarios.len().saturating_add(1)));
        // Base block: one instrument per position (the unshocked reprice).
        for p in positions {
            insts.push(
                fx_batch_instrument(p.option, &p.inputs)
                    .expect("all-FX guaranteed by would_screen_on_gpu"),
            );
        }
        // Shocked block: scenario-major, position-minor (row-aligned with the reduce).
        for s in scenarios {
            for p in positions {
                let shocked = s.apply(&p.inputs);
                insts.push(
                    fx_batch_instrument(p.option, &shocked)
                        .expect("all-FX guaranteed by would_screen_on_gpu"),
                );
            }
        }
        let prices = self.gpu.price_batch(&insts);
        let (base, shocked_all) = prices.split_at(np);
        (0..scenarios.len())
            .map(|si| {
                let row = &shocked_all[si * np..(si + 1) * np];
                row.iter()
                    .zip(base)
                    .zip(positions)
                    .map(|((shocked_px, base_px), p)| (shocked_px - base_px) * p.notional_base)
                    .sum::<f64>()
            })
            .collect()
    }
}

/// **Exact ≤1e-12 path.** The GPU backend serves the cube's authoritative reprice
/// through the exact CPU-f64 oracle: the f32/A&S GPU batch cannot certify the
/// `libm::erfc` f64 ≤1e-12 contract, so it is never used on the exact reval (see the
/// crate docs — the same refusal the cube applies to MC estimator noise). This is
/// byte-identical to [`celnet_risk_cube::SerialReprice`].
impl<P: CarryPricer> ScenarioReprice for GpuBatchReprice<'_, P> {
    fn node_pnls(&self, positions: &[PositionRisk], scenarios: &[Scenario]) -> Vec<f64> {
        SerialReprice(self.cpu).node_pnls(positions, scenarios)
    }
}

/// A [`ScenarioReprice`] view whose `node_pnls` is the **screening** lens — lets the
/// screening VaR reuse the cube's exact quantile reduction via
/// [`historical_var_es_via`] without duplicating the tail logic.
struct ScreeningView<'a, 'p, P: CarryPricer>(&'a GpuBatchReprice<'p, P>);

impl<P: CarryPricer> ScenarioReprice for ScreeningView<'_, '_, P> {
    fn node_pnls(&self, positions: &[PositionRisk], scenarios: &[Scenario]) -> Vec<f64> {
        self.0.screening_node_pnls(positions, scenarios)
    }
}

/// Map a carry-tagged pricing input to a `celnet-gpu` [`BatchInstrument`], for the
/// FX two-rate (Garman-Kohlhagen) lowering the batch closed form implements.
///
/// FX and precious-metal underlyings under [`celnet_types::Carry::FxRates`] lower
/// directly (a field copy of `(r_dom, r_for)`); the cross-asset cost-of-carry arms
/// (equity / commodity / digital-asset) are declined (`None`) so the screening lens
/// only ever runs over inputs whose exact oracle is the golden-gated
/// `celnet-vanilla` closed form — never a silent FX-proxy of another asset class.
fn fx_batch_instrument(opt: OptionType, ci: &CarryInputs) -> Option<BatchInstrument> {
    let vi = fx_vanilla_inputs(ci).ok()?;
    Some(match opt {
        OptionType::Call => {
            BatchInstrument::call(vi.spot, vi.strike, vi.vol, vi.t, vi.r_dom, vi.r_for)
        }
        OptionType::Put => {
            BatchInstrument::put(vi.spot, vi.strike, vi.vol, vi.t, vi.r_dom, vi.r_for)
        }
    })
}

/// The **derived** per-scenario reconciliation bound for the GPU screening node P&L
/// against the exact CPU-f64 oracle: the sum over positions of
/// `|notional| × (base-price bound + shocked-price bound)`, where each price bound is
/// [`celnet_gpu::derived_batch_bound`] (f32 round-off) +
/// [`celnet_gpu::as_erf_price_bound`] (A&S-`erf` algorithmic error vs the
/// golden-gated `celnet-vanilla` closed form). Every term is derived from
/// `f32::EPSILON`, the GK condition numbers, and the A&S `1.5e-7` `erf` bound — never
/// a fitted tolerance. A cross-asset position (not FX-lowerable) contributes `0`,
/// since the screening lens routes such a node to the exact CPU path.
#[must_use]
pub fn screening_node_pnl_bound(positions: &[PositionRisk], scenario: Scenario) -> f64 {
    positions
        .iter()
        .map(|p| {
            let (Some(base), Some(shocked)) = (
                fx_batch_instrument(p.option, &p.inputs),
                fx_batch_instrument(p.option, &scenario.apply(&p.inputs)),
            ) else {
                return 0.0;
            };
            let base_bound = derived_batch_bound(&base) + as_erf_price_bound(&base);
            let shocked_bound = derived_batch_bound(&shocked) + as_erf_price_bound(&shocked);
            p.notional_base.abs() * (base_bound + shocked_bound)
        })
        .sum::<f64>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_cube::{historical_var_es, node_pnl};
    use celnet_risk_normalize::AssetPricer;
    use celnet_types::{
        Carry, Ccy, CcyPair, DeltaConvention, EquityRef, OptionType, PremiumStyle, Symbol,
        Underlying, VanillaInputs,
    };

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

    /// A representative multi-position FX node (long/short mix, varied strike / vol /
    /// expiry) — the shape a desk-level VaR reprices.
    fn fx_node() -> Vec<PositionRisk> {
        vec![
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
            pos(
                OptionType::Put,
                -3_000_000.0,
                VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.03, 0.01),
            ),
        ]
    }

    /// A wide 2-D shock ladder (spot ±20% × vol ±2 vol-pts) — `41 × 9 = 369`
    /// scenarios, so a 4-position node flattens to `4 × 370 = 1480` instruments and
    /// clears the default GPU threshold (a genuine at-scale dispatch).
    fn wide_ladder() -> Vec<Scenario> {
        let mut v = Vec::new();
        for si in -20..=20 {
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

    /// The exact oracle's per-scenario node P&L vector (the ≤1e-12 ground truth).
    fn oracle_pnls(node: &[PositionRisk], scen: &[Scenario]) -> Vec<f64> {
        scen.iter()
            .map(|s| node_pnl(&AssetPricer, node, *s))
            .collect()
    }

    /// **Exact path ≤1e-12 (byte-identical).** The GPU-backed backend's exact
    /// [`ScenarioReprice`] node P&L and its reduced VaR/ES equal the CPU-f64 oracle
    /// bit-for-bit — the f32 GPU is (correctly) never on the exact reval, so the
    /// authoritative VaR is unchanged. On this Metal host `is_gpu()` may be `true`,
    /// yet the exact path is still the CPU oracle (the crate's precision contract).
    #[test]
    fn exact_path_matches_cpu_oracle_bit_for_bit() {
        let node = fx_node();
        let scen = wide_ladder();
        let bp = GpuBatchReprice::new(&AssetPricer);

        let via = bp.node_pnls(&node, &scen);
        let oracle = oracle_pnls(&node, &scen);
        assert_eq!(via.len(), oracle.len());
        for (x, y) in via.iter().zip(&oracle) {
            assert_eq!(
                x.to_bits(),
                y.to_bits(),
                "exact reprice must be byte-identical"
            );
        }
        for alpha in [0.975, 0.99] {
            let exact = historical_var_es_via(&bp, &node, &scen, alpha);
            let direct = historical_var_es(&AssetPricer, &node, &scen, alpha);
            assert_eq!(exact.var.to_bits(), direct.var.to_bits());
            assert_eq!(exact.es.to_bits(), direct.es.to_bits());
        }
        println!(
            "exact_path_matches_cpu_oracle_bit_for_bit: is_gpu={} label={}",
            bp.is_gpu(),
            bp.label()
        );
    }

    /// **Screening path reconciles to the CPU-f64 oracle within the DERIVED bound.**
    /// On a large all-FX node the screening lens dispatches the batch closed form
    /// (genuinely on the GPU when an adapter is present — asserted) and its
    /// per-scenario node P&L sits within [`screening_node_pnl_bound`] of the exact
    /// oracle. NO `to_bits` pin on the GPU result (f32 grade); the CPU flat-carry
    /// oracle is the byte-identity reference. The derived bound (f32 round-off + A&S
    /// `erf`) is the honest reconciliation — not a fitted tolerance, and not ≤1e-12.
    #[test]
    fn screening_reconciles_within_derived_bound() {
        let node = fx_node();
        let scen = wide_ladder();
        let bp = GpuBatchReprice::new(&AssetPricer);

        // On a host with an adapter the batch dispatch is genuinely exercised.
        if bp.is_gpu() {
            assert!(
                bp.would_screen_on_gpu(&node, &scen),
                "GPU present but the large all-FX node did not route to the GPU"
            );
        }

        let screening = bp.screening_node_pnls(&node, &scen);
        let oracle = oracle_pnls(&node, &scen);
        assert_eq!(screening.len(), oracle.len());

        let mut max_abs = 0.0_f64;
        let mut max_rel = 0.0_f64;
        for (si, s) in scen.iter().enumerate() {
            let bound = screening_node_pnl_bound(&node, *s);
            let err = (screening[si] - oracle[si]).abs();
            assert!(
                err <= bound,
                "screening node P&L off oracle beyond the derived bound at scenario #{si} \
                 ({s:?}): err={err:e} > bound={bound:e} (screening={}, oracle={})",
                screening[si],
                oracle[si]
            );
            max_abs = max_abs.max(err);
            if oracle[si].abs() > 1.0 {
                max_rel = max_rel.max(err / oracle[si].abs());
            }
        }
        println!(
            "screening_reconciles_within_derived_bound: n={} is_gpu={} max_abs_err={max_abs:e} \
             max_rel_err={max_rel:e}",
            scen.len(),
            bp.is_gpu()
        );
    }

    /// **Fallback verified: degrades to CPU cleanly and matches ≤1e-12.** Forcing the
    /// CPU path (an unreachable threshold — the same branch a headless / no-f64 host
    /// takes) makes the screening node P&L **byte-identical** to the exact oracle,
    /// and `would_screen_on_gpu` is `false`. This is the "wiring degrades to CPU
    /// cleanly when GPU/f64 is unavailable — the fallback runs and matches" gate.
    #[test]
    fn forced_cpu_fallback_is_exact() {
        let node = fx_node();
        let scen = wide_ladder();
        let bp = GpuBatchReprice::with_config(
            &AssetPricer,
            RepriceConfig {
                gpu_min_batch: usize::MAX,
            },
        );
        assert!(
            !bp.would_screen_on_gpu(&node, &scen),
            "an unreachable threshold must force the CPU path"
        );
        let cpu = bp.screening_node_pnls(&node, &scen);
        let oracle = oracle_pnls(&node, &scen);
        assert_eq!(cpu.len(), oracle.len());
        for (x, y) in cpu.iter().zip(&oracle) {
            assert_eq!(
                x.to_bits(),
                y.to_bits(),
                "CPU fallback must be byte-identical"
            );
        }
        // The screening VaR over the forced-CPU path is exactly the oracle VaR.
        let screen_var = bp.screening_var_es(&node, &scen, 0.99);
        let oracle_var = historical_var_es(&AssetPricer, &node, &scen, 0.99);
        assert_eq!(screen_var.var.to_bits(), oracle_var.var.to_bits());
        assert_eq!(screen_var.es.to_bits(), oracle_var.es.to_bits());
    }

    /// **A cross-asset (non-FX) node degrades to the exact CPU path.** An equity
    /// position does not lower to the FX GK batch, so `would_screen_on_gpu` is
    /// `false` even on a GPU host, and the screening node P&L is byte-identical to
    /// the exact oracle — the screening lens never silently FX-proxies another asset
    /// class (the no-silent-proxy contract, carried from the seam).
    #[test]
    fn cross_asset_node_degrades_to_exact() {
        let u = Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD));
        let eq = PositionRisk::carry(
            u.clone(),
            OptionType::Call,
            1_000.0,
            CarryInputs::new(
                100.0,
                105.0,
                0.20,
                1.0,
                u,
                Carry::CostOfCarry { r: 0.03, b: 0.01 },
            ),
        );
        let node = vec![eq];
        let scen = wide_ladder();
        let bp = GpuBatchReprice::new(&AssetPricer);
        assert!(
            !bp.would_screen_on_gpu(&node, &scen),
            "a non-FX node must never route to the FX GK batch"
        );
        let screening = bp.screening_node_pnls(&node, &scen);
        let oracle = oracle_pnls(&node, &scen);
        for (x, y) in screening.iter().zip(&oracle) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
        // The derived screening bound for a non-FX node is exactly zero (it never
        // hits the GPU), matching the byte-identical CPU result.
        assert_eq!(screening_node_pnl_bound(&node, scen[10]), 0.0);
    }

    /// **The threshold governs GPU routing.** Below `gpu_min_batch` the reprice stays
    /// on the CPU (no dispatch overhead); at/above it — with an adapter and an all-FX
    /// node — it routes to the GPU. Verified purely through the routing predicate so
    /// the assertion holds identically on a GPU host and a headless runner (where the
    /// `is_gpu()` term makes both sides CPU).
    #[test]
    fn threshold_selects_backend() {
        let node = fx_node(); // 4 positions
        let small = vec![Scenario::spot(0.01), Scenario::spot(-0.01)]; // flat = 4×3 = 12
        let large = wide_ladder(); // flat = 4×370 = 1480

        let bp = GpuBatchReprice::with_config(&AssetPricer, RepriceConfig { gpu_min_batch: 64 });
        // Small batch never uses the GPU regardless of device.
        assert!(!bp.would_screen_on_gpu(&node, &small));
        // Large batch routes to the GPU iff an adapter is present.
        assert_eq!(bp.would_screen_on_gpu(&node, &large), bp.is_gpu());

        // Both batches produce a correct result (the small one via CPU, exact).
        let small_pnls = bp.screening_node_pnls(&node, &small);
        for (x, y) in small_pnls.iter().zip(&oracle_pnls(&node, &small)) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }
}
