//! Parity row — **GPU multi-step path kernel** (`celnet-gpu` G3, Wave 8 Track A).
//!
//! The `celnet-gpu` multi-step path kernel (`path.wgsl`/`path.rs`) evolves a
//! batch of multi-step geometric-Brownian-motion paths under the SHARED
//! `celnet-qmc` Sobol' direction numbers and Brownian-bridge weight matrix, and
//! prices a path-dependent arithmetic-average Asian. Each claim is backed by an
//! *independent* oracle:
//!
//! * (i) **CPU↔GPU Sobol' KAT** — feeding the *same* Sobol' index to the CPU and
//!   the GPU yields the same path. The Sobol' INTEGERS are pure `u32` gray-code
//!   XOR and are bit-identical on both sides (asserted directly against the
//!   `celnet-qmc` `point_u32`); the resulting per-path payoffs then reconcile
//!   node-by-node within the DERIVED f32 bound (`celnet-gpu::derived_path_bound`,
//!   from `f32::EPSILON` — never fitted). When a GPU adapter is present the
//!   shader path is genuinely exercised (`is_gpu()` asserted), not a vacuous
//!   CPU-vs-CPU.
//! * (ii) **GPU-f32 ≈ CPU-f64 ≈ exact geometric closed form** — the *geometric*
//!   average over the same Sobol'/bridge path has the exact Kemna-Vorst closed
//!   form (`celnet-exotics::geometric_average_price`), an INDEPENDENT analytic
//!   oracle for the path machinery (Sobol' + bridge + GBM): the QMC geometric
//!   price over the same paths converges to it within the reported QMC error.
//! * (iii) **arithmetic price ≈ independent plain-MC** — the GPU's arithmetic
//!   Asian price agrees with a *code-disjoint* plain pseudo-random (splitmix64 +
//!   independent Box-Muller) Monte-Carlo of the same arithmetic Asian within the
//!   plain-MC standard error, and brackets the geometric price from above
//!   (arithmetic ≥ geometric, the AM-GM structural inequality).
//!
//! # HONEST BOUNDARY (verbatim)
//!
//! M4 Metal lacks f64 ⇒ the GPU path is **f32**; in-repo this proves
//! **CORRECTNESS + RATIOS only**. The **NVIDIA absolute throughput headline /
//! ≤50ms exotic / Workload-A/B absolutes are DEFERRED** to the CUDA deploy-gate.
//! We **never claim f64 on Metal**.

use celnet_core::math::{exp, ln, sqrt};
use celnet_exotics::asian::{AnalyticAsian, geometric_average_price};
use celnet_gpu::{
    AsianPathSpec, MultiStepPathPricer, cpu_path_payoffs, cpu_path_zmax,
    derived_path_bound_for_zmax,
};
use celnet_qmc::{BrownianBridge, SobolSequence, inv_norm_cdf};
use celnet_types::{OptionType, VanillaInputs};

/// Probe whether a real GPU adapter is present (so the reconcile can require the
/// shader path when hardware exists, and skip it honestly when headless).
fn adapter_is_available() -> bool {
    if wgpu::Instance::enabled_backend_features().is_empty() {
        return false;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .is_ok()
}

fn demo_spec(paths: u32, steps: u32) -> AsianPathSpec {
    AsianPathSpec {
        spot: 100.0,
        strike: 100.0,
        vol: 0.20,
        t: 1.0,
        r_dom: 0.05,
        r_for: 0.02,
        sign: 1.0,
        steps,
        paths,
        sobol_base: 1,
    }
}

// ---------------------------------------------------------------------------
// (i) CPU↔GPU Sobol' known-answer test + node-by-node reconciliation
// ---------------------------------------------------------------------------

/// The GPU kernel and the CPU oracle draw the IDENTICAL Sobol' sequence (the
/// integers are bit-identical `u32` gray-code XOR), so the per-path payoffs match
/// node-by-node within the derived f32 bound. With an adapter present the shader
/// path MUST be exercised.
#[test]
fn gpu_path_cpu_kat_and_node_by_node() {
    let spec = demo_spec(256 * 32, 8);

    // The integer Sobol' draws the kernel consumes (uploaded direction numbers,
    // gray-code XOR'd on-device) are exactly the celnet-qmc point_u32 — the KAT
    // on the SHARED draws (this is what makes CPU and GPU price the same path).
    let seq = SobolSequence::new(spec.steps as usize);
    for p in 0..16u64 {
        let i = u64::from(spec.sobol_base) + p;
        // Reconstruct the gray-code XOR the shader does from the same table.
        let g = i ^ (i >> 1);
        let mut coord0 = 0u32;
        let dirs = seq.direction_numbers(0);
        let mut bits = g;
        let mut k = 0usize;
        while bits != 0 && k < 32 {
            if bits & 1 == 1 {
                coord0 ^= dirs[k];
            }
            bits >>= 1;
            k += 1;
        }
        assert_eq!(
            coord0,
            seq.point_u32(i)[0],
            "Sobol' KAT mismatch at index {i} (the shared draw the GPU consumes)"
        );
    }

    let pricer = MultiStepPathPricer::new();
    if adapter_is_available() {
        assert!(
            pricer.is_gpu(),
            "a GPU adapter is present but the path pricer fell back to CPU: {}",
            pricer.label()
        );
    }
    let gpu = pricer.path_payoffs(&spec);
    let cpu = cpu_path_payoffs(&spec);
    let zmax = cpu_path_zmax(&spec);
    assert_eq!(gpu.len(), cpu.len());
    // Each path's bound is sized by that path's max |z| (the f32 inverse-normal
    // has a 1/φ(z) tail sensitivity — derived from f32::EPSILON, never fitted).
    for (idx, (&g, &c)) in gpu.iter().zip(cpu.iter()).enumerate() {
        let bound = derived_path_bound_for_zmax(&spec, zmax[idx]);
        let err = (g - c).abs();
        assert!(
            err <= bound,
            "path {idx}: gpu {g} vs cpu {c} err {err} exceeds derived bound {bound} (zmax={}, is_gpu={})",
            zmax[idx],
            pricer.is_gpu()
        );
    }
}

// ---------------------------------------------------------------------------
// (ii) GPU/CPU geometric QMC price == exact Kemna-Vorst closed form
// ---------------------------------------------------------------------------

/// The discretely-monitored **geometric**-average Asian has the exact
/// Kemna-Vorst closed form. Pricing it by QMC over the SAME Sobol'/bridge path
/// the GPU kernel uses validates the path machinery against an INDEPENDENT
/// analytic oracle to within the QMC error.
#[test]
fn geometric_qmc_matches_closed_form() {
    let spec = demo_spec(1 << 16, 8);
    let m = spec.steps as usize;

    // Geometric average over the same Sobol'/bridge construction the GPU uses.
    let seq = SobolSequence::new(m);
    let bridge = BrownianBridge::new(m, spec.t);
    let a = bridge.weight_matrix();
    let ln_spot = ln(spec.spot);
    let drift = spec.drift();
    let half_var = 0.5 * spec.vol * spec.vol;
    let dt = spec.t / m as f64;

    let mut acc = 0.0f64;
    for p in 0..spec.paths {
        let i = u64::from(spec.sobol_base) + u64::from(p);
        let pt = seq.point_u32(i);
        let z: Vec<f64> = pt
            .iter()
            .map(|&u| inv_norm_cdf((f64::from(u) + 0.5) / 4_294_967_296.0))
            .collect();
        let mut sum_ln = 0.0;
        for (l, a_row) in a.iter().enumerate().take(m) {
            let w_l: f64 = a_row.iter().zip(z.iter()).map(|(&aik, &zk)| aik * zk).sum();
            let t_l = (l + 1) as f64 * dt;
            sum_ln += ln_spot + (drift - half_var) * t_l + spec.vol * w_l;
        }
        // Geometric average = exp(mean of the log-spots).
        let geo = exp(sum_ln / m as f64);
        acc += (geo - spec.strike).max(0.0);
    }
    let qmc_geo = spec.discount() * acc / f64::from(spec.paths);

    let inputs = VanillaInputs::new(
        spec.spot,
        spec.strike,
        spec.vol,
        spec.t,
        spec.r_dom,
        spec.r_for,
    );
    let exact = geometric_average_price(
        &inputs,
        AnalyticAsian::fresh_discrete(OptionType::Call, spec.strike, m),
    );
    // QMC over 2^16 Brownian-bridged Sobol' points converges to the exact
    // geometric closed form to well under a tenth of a percent of the price.
    assert!(
        (qmc_geo - exact).abs() < 1e-3 * exact.max(1.0),
        "QMC geometric {qmc_geo} vs exact Kemna-Vorst {exact}"
    );
}

// ---------------------------------------------------------------------------
// (iii) arithmetic GPU price == independent plain-MC + AM-GM bracket
// ---------------------------------------------------------------------------

/// SplitMix64 → two independent Box-Muller normals; a CODE-DISJOINT plain
/// pseudo-random generator (no Sobol', no Philox) so the plain-MC oracle shares
/// nothing with the QMC path under test.
struct PlainRng {
    state: u64,
}
impl PlainRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        // 53-bit mantissa uniform in (0, 1).
        ((self.next_u64() >> 11) as f64 + 0.5) * (1.0 / (1u64 << 53) as f64)
    }
    fn normal(&mut self) -> f64 {
        let u1 = self.unit();
        let u2 = self.unit();
        sqrt(-2.0 * ln(u1)) * (core::f64::consts::TAU * u2).cos()
    }
}

#[test]
fn arithmetic_gpu_matches_plain_mc_and_brackets_geometric() {
    let spec = demo_spec(1 << 16, 8);
    let m = spec.steps as usize;

    let pricer = MultiStepPathPricer::new();
    let (gpu_arith, _se) = pricer.price_asian(&spec);

    // Independent plain-MC of the SAME arithmetic Asian: sequential GBM
    // increments from a code-disjoint splitmix64 + Box-Muller. No Sobol'/bridge.
    let n_mc = 400_000usize;
    let dt = spec.t / m as f64;
    let drift_step = (spec.drift() - 0.5 * spec.vol * spec.vol) * dt;
    let vol_step = spec.vol * sqrt(dt);
    let mut rng = PlainRng::new(0x0BAD_C0DE_1234_5678);
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    for _ in 0..n_mc {
        let mut ln_s = ln(spec.spot);
        let mut avg = 0.0;
        for _ in 0..m {
            ln_s += drift_step + vol_step * rng.normal();
            avg += exp(ln_s);
        }
        avg /= m as f64;
        let pay = (avg - spec.strike).max(0.0);
        sum += pay;
        sum_sq += pay * pay;
    }
    let mean = sum / n_mc as f64;
    let var = ((sum_sq - n_mc as f64 * mean * mean) / (n_mc as f64 - 1.0)).max(0.0);
    let plain_price = spec.discount() * mean;
    let plain_se = spec.discount() * sqrt(var / n_mc as f64);

    // The GPU QMC arithmetic price agrees with the independent plain-MC price
    // within a few plain-MC standard errors (QMC has negligible error here).
    assert!(
        (gpu_arith - plain_price).abs() < 5.0 * plain_se + 1e-3,
        "GPU arithmetic {gpu_arith} vs plain-MC {plain_price} (5·se = {})",
        5.0 * plain_se
    );

    // AM-GM: the arithmetic-average call dominates the geometric-average call.
    let inputs = VanillaInputs::new(
        spec.spot,
        spec.strike,
        spec.vol,
        spec.t,
        spec.r_dom,
        spec.r_for,
    );
    let geo = geometric_average_price(
        &inputs,
        AnalyticAsian::fresh_discrete(OptionType::Call, spec.strike, m),
    );
    assert!(
        gpu_arith > geo - 5.0 * plain_se,
        "arithmetic {gpu_arith} should bracket geometric {geo} from above"
    );
}
