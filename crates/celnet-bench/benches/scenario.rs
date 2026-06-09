//! Risk-path acceleration bench: **scenario-grid revaluation paths**
//! (`docs/RISK-HIERARCHY.md` §3.3, `docs/GPU-AT-SCALE-PLAN.md`).
//!
//! The Risk workspace / cube scenario reval prices a node's PV over a 2-D
//! spot×vol shock ladder. Three paths produce the same grid, benched here so the
//! crossover is **measured honestly, not assumed**:
//!
//! - **`scenario_grid_gpu`** — the **batched** kernel
//!   (`celnet_risk_cube::gpu_pv_grid`): ONE GPU dispatch per position over the
//!   whole grid (Metal/Vulkan), under common random numbers. On a headless host it
//!   transparently runs the f64 CPU oracle.
//! - **`scenario_grid_gpu_unbatched`** — the same Monte-Carlo estimate but
//!   priced **one node at a time** (a `price_vanilla` call per grid node per
//!   position). This is the like-for-like MC baseline the batching replaces, so
//!   `gpu / gpu_unbatched` is the genuine **batching win within the MC regime**
//!   (one dispatch + readback vs `nodes × positions` of them).
//! - **`scenario_grid_cpu_node_by_node`** — the exact closed-form
//!   (`celnet_risk_cube::analytic_pv_grid`). For *vanilla* payoffs the closed form
//!   needs no paths, so it dominates both MC paths at this scale; it is the
//!   honest datapoint that **MC/GPU is the scale path only for path-dependent
//!   (no-closed-form) payoffs or grids large enough to amortize MC**, not for
//!   analytic vanillas. It is also the cube's exact bump-and-revalue oracle.
//!
//! All three build the identical `node × grid` shape over one single-pair node.
//! `divan` reports item throughput in grid-nodes/s via the input counter.

use celnet_gpu::{
    GpuBackend, PathSpec, PayoffKernel, PricingBackend, ScenarioAxes, ScenarioPricer,
};
use celnet_risk_cube::{analytic_pv_grid, gpu_pv_grid};
use celnet_risk_normalize::PositionRisk;
use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};
use divan::{Bencher, black_box, counter::ItemsCount};

fn main() {
    divan::main();
}

/// Monte-Carlo paths per node for the GPU batch (production-representative).
const PATHS: u32 = 1 << 16;
/// Fixed seed → reproducible bitstream.
const SEED: u64 = 0xBEEF_F00D;

/// A representative single-pair node: a small EURUSD straddle-ish book.
fn node() -> Vec<PositionRisk> {
    let pair = CcyPair::new(Ccy::EUR, Ccy::USD);
    let mk = |opt, notional, strike, vol| {
        PositionRisk::fx(
            pair,
            opt,
            notional,
            VanillaInputs::new(1.10, strike, vol, 0.5, 0.03, 0.01),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    };
    vec![
        mk(OptionType::Call, 10_000_000.0, 1.12, 0.10),
        mk(OptionType::Put, -5_000_000.0, 1.08, 0.11),
        mk(OptionType::Call, 7_000_000.0, 1.15, 0.09),
    ]
}

/// A `11 × 11` spot×vol ladder (±5 % spot, ±5 vol points): 121 nodes.
fn axes() -> ScenarioAxes {
    ScenarioAxes::ladders(5, 0.01, 5, 0.01)
}

/// GPU-batched scenario grid (or CPU-fallback batch where no adapter is present).
#[divan::bench]
fn scenario_grid_gpu(bencher: Bencher) {
    let positions = node();
    let axes = axes();
    let pricer = ScenarioPricer::new();
    let nodes = (axes.n_spot() * axes.n_vol()) as usize;
    bencher.counter(ItemsCount::new(nodes)).bench_local(|| {
        gpu_pv_grid(
            &pricer,
            black_box(&positions),
            black_box(&axes),
            PATHS,
            SEED,
        )
    });
}

/// Unbatched Monte-Carlo: the SAME estimate priced one grid node at a time (one
/// `price_vanilla` dispatch per node per position). The like-for-like MC baseline
/// the batched kernel replaces — `gpu / this` is the batching win within the MC
/// regime (one dispatch+readback vs `nodes × positions` of them).
#[divan::bench]
fn scenario_grid_gpu_unbatched(bencher: Bencher) {
    let positions = node();
    let axes = axes();
    let backend = GpuBackend::new();
    let nodes = (axes.n_spot() * axes.n_vol()) as usize;
    bencher.counter(ItemsCount::new(nodes)).bench_local(|| {
        let mut acc = 0.0f64;
        for &sm in &axes.spot_mult {
            for &vb in &axes.vol_bump {
                let sm = f64::from(sm);
                let vb = f64::from(vb);
                for p in &positions {
                    let i = &p.inputs;
                    // The FX two-rate view of the carry-tagged inputs (these benches
                    // are FX-only; byte-identical to the prior VanillaInputs rates).
                    let v =
                        celnet_core::carry::fx_vanilla_inputs(i).expect("FX scenario bench inputs");
                    let spec = PathSpec::gbm(
                        i.spot * sm,
                        i.vol + vb,
                        i.t,
                        v.r_dom,
                        v.r_for,
                        PATHS,
                        1,
                        SEED,
                    );
                    let payoff = match p.option {
                        OptionType::Call => PayoffKernel::call(i.strike),
                        OptionType::Put => PayoffKernel::put(i.strike),
                    };
                    acc += backend.price_vanilla(&spec, &payoff).price() * p.notional_base;
                }
            }
        }
        black_box(acc)
    });
}

/// CPU node-by-node exact closed-form scenario grid (the bump-and-revalue oracle).
#[divan::bench]
fn scenario_grid_cpu_node_by_node(bencher: Bencher) {
    let positions = node();
    let axes = axes();
    let nodes = (axes.n_spot() * axes.n_vol()) as usize;
    bencher
        .counter(ItemsCount::new(nodes))
        .bench_local(|| analytic_pv_grid(black_box(&positions), black_box(&axes)));
}
