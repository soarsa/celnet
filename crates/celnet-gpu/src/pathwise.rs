//! Pathwise / likelihood-ratio Monte-Carlo **Greeks** on the GPU — one dispatch
//! produces the per-path Greek estimators for a single-step terminal GBM vanilla
//! under SHARED Sobol' draws, averaged by the host into delta/vega. The exact
//! f64 CPU oracle (reusing the `celnet-qmc` Sobol'/inverse-normal core verbatim)
//! reconciles it node-by-node and runs headless/CI.
//!
//! # What this is (GPU-AT-SCALE G6)
//!
//! Two estimator families share one terminal draw `Z = Φ⁻¹(u)`,
//! `S_T = S₀·exp((μ−½σ²)T + σ√T·Z)`:
//!
//! * **Pathwise** (smooth call payoff `(S_T−K)⁺` — the `max` kink is
//!   measure-zero, so pathwise differentiation is valid):
//!   `∂/∂S₀ = 1{S_T>K}·S_T/S₀`,
//!   `∂/∂σ = 1{S_T>K}·S_T·(√T·Z − σT)`.
//! * **Likelihood-ratio** (the **discontinuous** digital `1{S_T>K}` — pathwise
//!   fails because the payoff derivative is a Dirac; differentiate the *density*
//!   instead): `∂/∂S₀ (LR) = payoff·Z/(S₀·σ√T)`.
//!
//! # Reconciliation (three-way + independent FD)
//!
//! * **GPU-f32 == CPU-f64** node-by-node (same Sobol' integers ⇒ same sample;
//!   only the f32 inverse-normal/`exp` chain diverges) within
//!   [`derived_greeks_bound`] — derived from `f32::EPSILON`, never fitted.
//! * **CPU-MC pathwise == analytic Garman-Kohlhagen** delta/vega within MC error
//!   (an independent closed-form oracle).
//! * **CPU-MC == central finite-difference** on the *same* QMC price for **both**
//!   families (the independent estimator-agnostic check the parity row gates).
//!
//! # HONEST BOUNDARY (verbatim)
//!
//! M4 Metal lacks f64 ⇒ Greeks are computed in **f32**. In-repo proves
//! **CORRECTNESS + RATIOS only**; the **NVIDIA absolute throughput headline /
//! ≤50ms exotic / Workload-A/B absolutes are DEFERRED** to the CUDA deploy-gate.
//! We **never claim f64 on Metal**.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::backend::PricingBackend;
use crate::cpu::pairwise_sum;
use crate::gpu::{GpuBackend, GpuContextRef};

/// Workgroup size; must match `@workgroup_size` in `greeks.wgsl`.
const WG_SIZE: u32 = 256;

/// Estimators per path; must match `N_EST` in `greeks.wgsl`.
const N_EST: usize = 5;

/// Upper bound on the number of paths per dispatch.
const MAX_PATHS: u64 = 8 * 1024 * 1024;

/// Bounded readback deadline (always-terminates posture).
const GPU_DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// A single-step terminal GBM vanilla whose pathwise/LR Greeks are estimated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GreeksSpec {
    /// Initial spot `S₀`.
    pub spot: f64,
    /// Strike `K`.
    pub strike: f64,
    /// Volatility `σ`.
    pub vol: f64,
    /// Time to expiry `T` (years).
    pub t: f64,
    /// Domestic continuously-compounded rate `r_dom` (discounting).
    pub r_dom: f64,
    /// Foreign continuously-compounded rate `r_for`.
    pub r_for: f64,
    /// Number of Sobol' QMC paths.
    pub paths: u32,
    /// First Sobol' index (gray-code offset).
    pub sobol_base: u32,
}

impl GreeksSpec {
    /// Risk-neutral GBM drift `μ = r_dom − r_for`.
    #[must_use]
    pub fn drift(&self) -> f64 {
        self.r_dom - self.r_for
    }

    /// Discount factor `e^{−r_dom·T}`.
    #[must_use]
    pub fn discount(&self) -> f64 {
        celnet_core::math::exp(-self.r_dom * self.t)
    }
}

/// The discounted Monte-Carlo Greeks (and the price) for a [`GreeksSpec`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GreeksEstimate {
    /// Discounted call price `e^{−r_d T}·E[(S_T−K)⁺]`.
    pub call_price: f64,
    /// Pathwise delta of the call `∂price/∂S₀`.
    pub pathwise_delta: f64,
    /// Pathwise vega of the call `∂price/∂σ`.
    pub pathwise_vega: f64,
    /// Discounted digital price `e^{−r_d T}·E[1{S_T>K}]`.
    pub digital_price: f64,
    /// Likelihood-ratio delta of the digital `∂(digital price)/∂S₀`.
    pub lr_digital_delta: f64,
    /// Number of paths reduced.
    pub paths: u32,
}

/// Uniform metadata block (`GreeksMeta`): four `u32` then eight `f32` = 48 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GreeksMetaGpu {
    paths: u32,
    sobol_base: u32,
    _p0: u32,
    _p1: u32,
    ln_spot: f32,
    spot: f32,
    drift: f32,
    vol: f32,
    strike: f32,
    t_total: f32,
    sqrt_t: f32,
    _p2: f32,
}

/// Estimates pathwise/LR Greeks by quasi-Monte-Carlo, on the GPU when an adapter
/// is present and on the exact f64 CPU oracle otherwise.
pub struct PathwiseGreeksPricer {
    backend: GpuBackend,
    pipeline: Option<GreeksPipeline>,
}

struct GreeksPipeline {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl core::fmt::Debug for PathwiseGreeksPricer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PathwiseGreeksPricer")
            .field("is_gpu", &self.is_gpu())
            .field("label", &self.backend.label())
            .finish()
    }
}

impl PathwiseGreeksPricer {
    /// Build the pricer, probing for a GPU adapter (CPU fallback otherwise).
    #[must_use]
    pub fn new() -> Self {
        let backend = GpuBackend::new();
        let pipeline = backend
            .gpu_context()
            .map(|ctx| GreeksPipeline::build(ctx.device));
        Self { backend, pipeline }
    }

    /// `true` when a real GPU adapter is driving the compute path.
    #[must_use]
    pub fn is_gpu(&self) -> bool {
        self.backend.is_gpu() && self.pipeline.is_some()
    }

    /// Human-readable backend identity.
    #[must_use]
    pub fn label(&self) -> String {
        self.backend.label()
    }

    /// Per-path estimators (`N_EST` f64 per path, row-major), node-by-node — the
    /// GPU side of the reconciliation. GPU path when an adapter is present;
    /// otherwise the exact f64 CPU oracle narrowed to f32 per element.
    ///
    /// # Panics
    ///
    /// Panics if `paths` is zero or exceeds [`MAX_PATHS`].
    #[must_use]
    pub fn path_estimators(&self, spec: &GreeksSpec) -> Vec<f64> {
        assert!(
            spec.paths >= 1 && u64::from(spec.paths) <= MAX_PATHS,
            "invalid path count {} (must be 1..={MAX_PATHS})",
            spec.paths
        );
        match (self.backend.gpu_context(), self.pipeline.as_ref()) {
            (Some(ctx), Some(pipe)) => Self::dispatch(&ctx, pipe, spec),
            _ => cpu_path_estimators(spec)
                .into_iter()
                .map(|p| f64::from(p as f32))
                .collect(),
        }
    }

    /// Average the per-path estimators into the discounted [`GreeksEstimate`].
    #[must_use]
    pub fn estimate(&self, spec: &GreeksSpec) -> GreeksEstimate {
        let est = self.path_estimators(spec);
        let n = spec.paths as usize;
        let discount = spec.discount();
        let col = |c: usize| {
            let v: Vec<f64> = (0..n).map(|p| est[N_EST * p + c]).collect();
            discount * pairwise_sum(&v) / n as f64
        };
        GreeksEstimate {
            call_price: col(0),
            pathwise_delta: col(1),
            pathwise_vega: col(2),
            digital_price: col(3),
            lr_digital_delta: col(4),
            paths: spec.paths,
        }
    }

    /// Run the GPU path: upload the shared dimension-1 Sobol' direction numbers,
    /// dispatch one thread per path, read back `N_EST` f32 per path.
    fn dispatch(ctx: &GpuContextRef<'_>, pipe: &GreeksPipeline, spec: &GreeksSpec) -> Vec<f64> {
        let dir_nums = shared_dim1_direction_numbers();

        let meta = GreeksMetaGpu {
            paths: spec.paths,
            sobol_base: spec.sobol_base,
            _p0: 0,
            _p1: 0,
            ln_spot: celnet_core::math::ln(spec.spot) as f32,
            spot: spec.spot as f32,
            drift: spec.drift() as f32,
            vol: spec.vol as f32,
            strike: spec.strike as f32,
            t_total: spec.t as f32,
            sqrt_t: celnet_core::math::sqrt(spec.t) as f32,
            _p2: 0.0,
        };

        let meta_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-greeks-meta"),
                contents: bytemuck::bytes_of(&meta),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let dir_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-greeks-dirnums"),
                contents: bytemuck::cast_slice(&dir_nums),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let out_len = spec.paths as usize * N_EST;
        let out_bytes = (out_len * core::mem::size_of::<f32>()) as u64;
        let out_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-greeks-est"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-greeks-readback"),
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("celnet-greeks-bg"),
            layout: &pipe.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: meta_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: dir_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("celnet-greeks-encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("celnet-greeks-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipe.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(spec.paths.div_ceil(WG_SIZE).max(1), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &read_buf, 0, out_bytes);
        ctx.queue.submit(Some(encoder.finish()));

        map_readback(ctx, &read_buf, out_len)
    }
}

impl Default for PathwiseGreeksPricer {
    fn default() -> Self {
        Self::new()
    }
}

impl GreeksPipeline {
    fn build(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("celnet-greeks"),
            source: wgpu::ShaderSource::Wgsl(include_str!("greeks.wgsl").into()),
        });

        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("celnet-greeks-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true),  // direction numbers (read-only)
                storage(2, false), // estimators (read-write)
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("celnet-greeks-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("celnet-greeks-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("pathwise_lr_greeks"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }
}

fn map_readback(ctx: &GpuContextRef<'_>, read_buf: &wgpu::Buffer, n: usize) -> Vec<f64> {
    let slice = read_buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });

    let deadline = std::time::Instant::now() + GPU_DISPATCH_TIMEOUT;
    let map_result = loop {
        ctx.device
            .poll(wgpu::PollType::Poll)
            .expect("device poll failed (device lost?)");
        match rx.try_recv() {
            Ok(res) => break res,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "GPU greeks readback timed out after {GPU_DISPATCH_TIMEOUT:?} \
                     (device lost or queue stalled)"
                );
                std::thread::yield_now();
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("map_async callback dropped without delivering a result")
            }
        }
    };
    map_result.expect("buffer map failed");

    let out = {
        let view = slice.get_mapped_range();
        let vals: &[f32] = bytemuck::cast_slice(&view);
        debug_assert_eq!(vals.len(), n);
        vals.iter().map(|&v| f64::from(v)).collect()
    };
    read_buf.unmap();
    out
}

/// The 32 dimension-1 Sobol' direction numbers (`v_{0,k} = 2^{31−k}`, the van
/// der Corput / bit-reversal sequence) **taken from the `celnet-qmc` public
/// API** so the GPU and CPU consume identical integers for the single driving
/// dimension.
fn shared_dim1_direction_numbers() -> [u32; 32] {
    let seq = celnet_qmc::SobolSequence::new(1);
    *seq.direction_numbers(0)
}

/// **Exact f64 CPU oracle** for the per-path Greek estimators, mirroring
/// `greeks.wgsl` and reusing the `celnet-qmc` Sobol'/inverse-normal core. Same
/// Sobol' integers ⇒ same QMC sample as the GPU.
#[must_use]
pub fn cpu_path_estimators(spec: &GreeksSpec) -> Vec<f64> {
    let seq = celnet_qmc::SobolSequence::new(1);
    let ln_spot = celnet_core::math::ln(spec.spot);
    let drift = spec.drift();
    let half_var = 0.5 * spec.vol * spec.vol;
    let sqrt_t = celnet_core::math::sqrt(spec.t);
    let s = spec.vol * sqrt_t;

    let mut out = Vec::with_capacity(spec.paths as usize * N_EST);
    for p in 0..spec.paths {
        let i = u64::from(spec.sobol_base) + u64::from(p);
        let u = (f64::from(seq.point_u32(i)[0]) + 0.5) / 4_294_967_296.0;
        // Algorithm-matched (A&S-erf) inverse-normal so the GPU↔CPU reconcile is
        // pure f32 round-off; the A&S vs libm-erf inverse difference is bracketed
        // separately (parity row) and is NOT folded into the round-off bound.
        let z = crate::as_normal::inv_norm_cdf_as(u);
        let ln_st = ln_spot + (drift - half_var) * spec.t + s * z;
        let st = celnet_core::math::exp(ln_st);
        let in_money = if st > spec.strike { 1.0 } else { 0.0 };
        let call = (st - spec.strike).max(0.0);
        out.push(call);
        out.push(in_money * st / spec.spot);
        out.push(in_money * st * (sqrt_t * z - spec.vol * spec.t));
        out.push(in_money);
        out.push(in_money * z / (spec.spot * s));
    }
    out
}

/// **Derived** f32-vs-f64 **per-path** reconciliation bound for the Greek
/// estimators of a path whose driving normal is `z` (the value the CPU oracle
/// computed). Applied per estimator value. The GPU and the
/// [`cpu_path_estimators`] oracle evaluate the *same* sample (bit-identical
/// Sobol' integer, the same A&S-erf inverse-normal algorithm) in f32 vs f64; the
/// divergence is **pure f32 round-off**, bounded from `f32::EPSILON` and the
/// estimator condition numbers — never fitted.
///
/// # Per-path error model
///
/// Let `eps = f32::EPSILON ≈ 5.96e-8` and `φ(z) = (2π)^{−½}e^{−z²/2}`.
///
/// * **Inverse-normal** `z = Φ⁻¹(u)` (Acklam seed + one Halley step against the
///   A&S-erf `Φ`). The Halley correction is `e/φ` with `e = Φ(x)−p`; in the tail
///   `Φ(x) ≈ p`, so the f32 rounding of that near-cancellation (absolute `~eps`)
///   is divided by `φ(z)`, giving the dominant round-off
///   `|Δz| ≲ (K_h·eps)/φ(z) + K_inv·eps·|z|`, `K_h ≈ 2`, `K_inv ≈ 8` (the
///   rational + remaining ops). This is the genuine — and per-path-known —
///   sensitivity of the f32 inverse-normal; it is large only for tail draws.
/// * `S_T = exp(ln S₀ + (μ−½σ²)T + σ√T·z)`: `|ΔS_T| ≈ S_T·(σ√T·|Δz| + (K_exp+2)·eps)`,
///   `K_exp ≈ 3`.
///
/// Each estimator's per-path absolute bound is its derivative magnitude times
/// these errors:
///
/// * call `(S_T−K)⁺`: `|ΔS_T|`.
/// * pathwise delta `S_T/S₀`: `|ΔS_T|/S₀`.
/// * pathwise vega `S_T·(√T·z − σT)`: `|ΔS_T|·|√T·z − σT| + S_T·√T·|Δz|`.
/// * digital `1{·}`: exact away from the strike (boundary flips handled by the
///   parity row's flip-count, not folded in here).
/// * LR delta `z/(S₀·σ√T)`: `|Δz|/(S₀·σ√T) + K_op·eps·|value|`.
///
/// Returns the **max** over the five (the comparison is per estimator value),
/// plus a small absolute floor. All terms derive from `f32::EPSILON` and the
/// path's own condition numbers.
#[must_use]
pub fn derived_greeks_bound_for_z(spec: &GreeksSpec, z: f64) -> f64 {
    let eps = f64::from(f32::EPSILON);
    let k_h = 2.0;
    let k_inv = 8.0;
    let k_exp = 3.0;
    let k_op = 6.0;
    let sqrt_t = celnet_core::math::sqrt(spec.t);
    let s_std = spec.vol * sqrt_t; // std of ln S_T

    let phi = celnet_core::math::norm_pdf(z).max(1e-12);
    let dz = (k_h * eps) / phi + k_inv * eps * z.abs();

    let ln_st = celnet_core::math::ln(spec.spot)
        + (spec.drift() - 0.5 * spec.vol * spec.vol) * spec.t
        + s_std * z;
    let st = celnet_core::math::exp(ln_st);
    let d_st = st * (s_std * dz + f64::from(k_exp as u32 + 2) * eps);

    let b_call = d_st;
    let b_delta = d_st / spec.spot;
    let b_vega = d_st * (sqrt_t * z - spec.vol * spec.t).abs() + st * sqrt_t * dz;
    let lr_val = z / (spec.spot * s_std);
    let b_lr = dz / (spec.spot * s_std) + k_op * eps * lr_val.abs();

    let abs_floor = 32.0 * eps;
    b_call.max(b_delta).max(b_vega).max(b_lr) + abs_floor
}

/// A conservative, **z-independent** envelope of [`derived_greeks_bound_for_z`]
/// over a `|z| ≤ z_cap` core (the bulk of the distribution). Convenience for
/// callers that do not have the per-path `z`; the per-path form is tighter and
/// is what the node-by-node parity row uses (it knows each path's `z`).
#[must_use]
pub fn derived_greeks_bound(spec: &GreeksSpec) -> f64 {
    // Evaluate the per-path bound at a representative tail cap (z = 5): beyond
    // this the inverse-normal round-off grows like 1/φ(z), so a single number is
    // only meaningful over the core. The parity row uses the per-path form.
    derived_greeks_bound_for_z(spec, 5.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{OptionType, VanillaInputs};

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
            spot: 1.20,
            strike: 1.20,
            vol: 0.12,
            t: 1.0,
            r_dom: 0.03,
            r_for: 0.01,
            paths,
            sobol_base: 1,
        }
    }

    /// CPU↔GPU node-by-node reconciliation across all five estimators within the
    /// derived f32 bound (excluding the small set of paths within round-off of
    /// the strike, where the indicator can legitimately flip between f32 and
    /// f64 — those are bounded in count, not in per-value magnitude).
    #[test]
    fn gpu_greeks_reconcile_node_by_node() {
        let spec = demo_spec(WG_SIZE * 16);
        let pricer = PathwiseGreeksPricer::new();
        if adapter_is_available() {
            assert!(
                pricer.is_gpu(),
                "GPU present but fell back: {}",
                pricer.label()
            );
        }
        let gpu = pricer.path_estimators(&spec);
        let cpu = cpu_path_estimators(&spec);
        assert_eq!(gpu.len(), cpu.len());

        // Recover each path's driving normal z (same Sobol' integer + the same
        // A&S-erf inverse-normal the oracle uses), so the per-path DERIVED bound
        // can account for the f32 inverse-normal's 1/φ(z) tail sensitivity.
        let seq = celnet_qmc::SobolSequence::new(1);

        let mut flips = 0usize;
        for p in 0..spec.paths as usize {
            // The indicator (estimator 3) is 0/1; a mismatch means a boundary
            // flip — count it and skip the other estimators for that path.
            if (gpu[N_EST * p + 3] - cpu[N_EST * p + 3]).abs() > 0.5 {
                flips += 1;
                continue;
            }
            let i = u64::from(spec.sobol_base) + p as u64;
            let u = (f64::from(seq.point_u32(i)[0]) + 0.5) / 4_294_967_296.0;
            let z = crate::as_normal::inv_norm_cdf_as(u);
            let bound = derived_greeks_bound_for_z(&spec, z);
            for c in 0..N_EST {
                let g = gpu[N_EST * p + c];
                let cc = cpu[N_EST * p + c];
                assert!(
                    (g - cc).abs() <= bound,
                    "path {p} est {c} (z={z}): gpu {g} vs cpu {cc} exceeds bound {bound}"
                );
            }
        }
        // Boundary flips are rare (~ paths within |ΔS_T| of the strike).
        assert!(
            flips * 200 < spec.paths as usize,
            "too many strike-boundary flips: {flips}/{}",
            spec.paths
        );
    }

    /// Pathwise delta/vega average to the analytic Garman-Kohlhagen Greeks within
    /// Monte-Carlo error — an independent closed-form oracle.
    #[test]
    fn pathwise_greeks_match_analytic() {
        let spec = demo_spec(WG_SIZE * 1024);
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
        // QMC delta/vega should be within ~1% of analytic at this budget.
        assert!(
            (est.pathwise_delta - g.delta_spot).abs() < 0.02,
            "pathwise delta {} vs analytic {}",
            est.pathwise_delta,
            g.delta_spot
        );
        // celnet-vanilla vega is per 1.00 vol (absolute); compare directly.
        assert!(
            (est.pathwise_vega - g.vega).abs() < 0.02 * spec.spot,
            "pathwise vega {} vs analytic {}",
            est.pathwise_vega,
            g.vega
        );
    }
}
