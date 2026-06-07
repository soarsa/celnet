//! Parity row — **GPU pathwise / likelihood-ratio Greeks** (`celnet-gpu` G6,
//! Wave 8 Track B).
//!
//! The `celnet-gpu` Greeks kernel (`greeks.wgsl`/`pathwise.rs`) produces the
//! per-path Monte-Carlo Greek estimators for a single-step terminal GBM vanilla
//! under the SHARED `celnet-qmc` Sobol' draws: **pathwise** delta/vega for the
//! smooth call payoff, and a **likelihood-ratio** delta for the *discontinuous*
//! digital payoff (where pathwise fails). Each claim is backed by an
//! *independent* oracle:
//!
//! * (i) **CPU↔GPU node-by-node** — the GPU f32 per-path estimators match the
//!   exact f64 CPU oracle (same Sobol' integer, same A&S-erf inverse-normal
//!   algorithm) within the DERIVED per-path f32 bound
//!   (`celnet-gpu::derived_greeks_bound_for_z`, from `f32::EPSILON` — never
//!   fitted), excluding the bounded count of strike-boundary indicator flips.
//!   With an adapter present the shader path is genuinely exercised
//!   (`is_gpu()` asserted).
//! * (ii) **pathwise delta/vega ≈ analytic Garman-Kohlhagen** — the discounted
//!   pathwise estimators average to the `celnet-vanilla` closed-form delta/vega
//!   within Monte-Carlo error (an INDEPENDENT analytic oracle).
//! * (iii) **central finite-difference cross-check** — BOTH families are checked
//!   against an independent central FD on the *same* QMC price (common random
//!   numbers, so the FD has no MC-noise crossing): the pathwise delta vs
//!   `(V(S+h) − V(S−h))/2h`, the pathwise vega vs `(V(σ+h) − V(σ−h))/2h`, and the
//!   LR digital delta vs the FD of the digital price. FD is estimator-agnostic,
//!   so agreeing with it validates the pathwise AND LR estimators independently
//!   of the analytic formula.
//!
//! # HONEST BOUNDARY (verbatim)
//!
//! M4 Metal lacks f64 ⇒ the Greeks are computed in **f32**; in-repo this proves
//! **CORRECTNESS + RATIOS only**. The **NVIDIA absolute throughput headline /
//! ≤50ms exotic / Workload-A/B absolutes are DEFERRED** to the CUDA deploy-gate.
//! We **never claim f64 on Metal**.

use celnet_core::math::sqrt;
use celnet_gpu::{
    GreeksSpec, PathwiseGreeksPricer, cpu_path_estimators, derived_greeks_bound_for_z,
};
use celnet_types::{OptionType, VanillaInputs};

const N_EST: usize = 5;

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

fn demo_spec(paths: u32) -> GreeksSpec {
    GreeksSpec {
        spot: 100.0,
        strike: 100.0,
        vol: 0.20,
        t: 1.0,
        r_dom: 0.05,
        r_for: 0.02,
        paths,
        sobol_base: 1,
    }
}

// ---------------------------------------------------------------------------
// (i) CPU↔GPU node-by-node reconciliation (per-path derived f32 bound)
// ---------------------------------------------------------------------------

#[test]
fn gpu_greeks_node_by_node() {
    let spec = demo_spec(256 * 32);
    let pricer = PathwiseGreeksPricer::new();
    if adapter_is_available() {
        assert!(
            pricer.is_gpu(),
            "a GPU adapter is present but the Greeks pricer fell back to CPU: {}",
            pricer.label()
        );
    }
    let gpu = pricer.path_estimators(&spec);
    let cpu = cpu_path_estimators(&spec);
    assert_eq!(gpu.len(), cpu.len());

    // The per-path derived bound is sized by that path's driving normal z (the
    // f32 inverse-normal has a 1/φ(z) tail sensitivity — derived, not fit). For
    // in-money paths z is recovered exactly from the LR estimator the oracle
    // already produced (= z/(S0·σ√T)); OTM paths have all-zero estimators, so the
    // z=0 (tightest) bound trivially covers the 0-vs-0 comparison.
    let mut flips = 0usize;
    for p in 0..spec.paths as usize {
        // Estimator 3 is the 0/1 indicator; a mismatch is a strike-boundary flip.
        if (gpu[N_EST * p + 3] - cpu[N_EST * p + 3]).abs() > 0.5 {
            flips += 1;
            continue;
        }
        let z = cpu[N_EST * p + 4] * (spec.spot * spec.vol * sqrt(spec.t));
        let bound = derived_greeks_bound_for_z(&spec, z);
        for c in 0..N_EST {
            let g = gpu[N_EST * p + c];
            let cc = cpu[N_EST * p + c];
            assert!(
                (g - cc).abs() <= bound,
                "path {p} est {c} (z={z}): gpu {g} vs cpu {cc} exceeds derived bound {bound} (is_gpu={})",
                pricer.is_gpu()
            );
        }
    }
    // Strike-boundary flips are rare (~ paths within |ΔS_T| of the strike).
    assert!(
        flips * 100 < spec.paths as usize,
        "too many strike-boundary indicator flips: {flips}/{}",
        spec.paths
    );
}

// ---------------------------------------------------------------------------
// (ii) pathwise delta/vega == analytic Garman-Kohlhagen (independent oracle)
// ---------------------------------------------------------------------------

#[test]
fn pathwise_greeks_match_analytic() {
    let spec = demo_spec(1 << 20);
    let pricer = PathwiseGreeksPricer::new();
    let est = pricer.estimate(&spec);
    let inputs = VanillaInputs::new(
        spec.spot,
        spec.strike,
        spec.vol,
        spec.t,
        spec.r_dom,
        spec.r_for,
    );
    let g = celnet_vanilla::greeks(OptionType::Call, &inputs);

    // delta ~ 0.5–0.6 here; vega ~ 0.39·S. Tolerances are MC-error scale at 2^20.
    assert!(
        (est.pathwise_delta - g.delta_spot).abs() < 5e-3,
        "pathwise delta {} vs analytic {}",
        est.pathwise_delta,
        g.delta_spot
    );
    assert!(
        (est.pathwise_vega - g.vega).abs() < 5e-3 * spec.spot,
        "pathwise vega {} vs analytic {}",
        est.pathwise_vega,
        g.vega
    );
}

// ---------------------------------------------------------------------------
// (iii) central finite-difference cross-check (estimator-agnostic)
// ---------------------------------------------------------------------------

/// Discounted QMC call price and digital price for a bumped spec, over the SAME
/// Sobol' draws (common random numbers ⇒ the FD has no MC-noise crossing). The
/// CPU oracle is used (exact f64) so the FD reference is independent of the GPU.
fn cpu_prices(spec: &GreeksSpec) -> (f64, f64) {
    let est = cpu_path_estimators(spec);
    let n = spec.paths as usize;
    let discount = spec.discount();
    let mut call = 0.0;
    let mut digital = 0.0;
    for p in 0..n {
        call += est[N_EST * p];
        digital += est[N_EST * p + 3];
    }
    (discount * call / n as f64, discount * digital / n as f64)
}

#[test]
fn greeks_match_central_finite_difference() {
    let spec = demo_spec(1 << 18);
    let pricer = PathwiseGreeksPricer::new();
    let est = pricer.estimate(&spec);

    // Central FD in spot (relative bump) under common random numbers.
    let h_s = 1e-3 * spec.spot;
    let up_s = GreeksSpec {
        spot: spec.spot + h_s,
        ..spec
    };
    let dn_s = GreeksSpec {
        spot: spec.spot - h_s,
        ..spec
    };
    let (call_up, dig_up) = cpu_prices(&up_s);
    let (call_dn, dig_dn) = cpu_prices(&dn_s);
    let fd_call_delta = (call_up - call_dn) / (2.0 * h_s);
    let fd_digital_delta = (dig_up - dig_dn) / (2.0 * h_s);

    // Central FD in vol.
    let h_v = 1e-3;
    let up_v = GreeksSpec {
        vol: spec.vol + h_v,
        ..spec
    };
    let dn_v = GreeksSpec {
        vol: spec.vol - h_v,
        ..spec
    };
    let (call_up_v, _) = cpu_prices(&up_v);
    let (call_dn_v, _) = cpu_prices(&dn_v);
    let fd_call_vega = (call_up_v - call_dn_v) / (2.0 * h_v);

    // Pathwise delta/vega vs FD on the smooth call (CRN FD is tight here).
    assert!(
        (est.pathwise_delta - fd_call_delta).abs() < 2e-3,
        "pathwise delta {} vs CRN-FD {}",
        est.pathwise_delta,
        fd_call_delta
    );
    assert!(
        (est.pathwise_vega - fd_call_vega).abs() < 2e-3 * spec.spot,
        "pathwise vega {} vs CRN-FD {}",
        est.pathwise_vega,
        fd_call_vega
    );
    // LR delta of the DISCONTINUOUS digital vs the CRN-FD of the digital price —
    // the independent check that the likelihood-ratio estimator is correct where
    // pathwise cannot apply. The digital FD is noisier (a step payoff), so a
    // looser-but-honest band; both estimate ∂(digital price)/∂S0.
    assert!(
        (est.lr_digital_delta - fd_digital_delta).abs() < 5e-3,
        "LR digital delta {} vs CRN-FD {}",
        est.lr_digital_delta,
        fd_digital_delta
    );
    // Sanity: the digital delta is the (positive) density-at-strike scale.
    assert!(
        est.lr_digital_delta > 0.0,
        "digital delta should be positive"
    );
}
