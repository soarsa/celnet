//! Multi-step quasi-Monte-Carlo **path** pricing — one GPU dispatch evolves a
//! large batch of multi-step geometric-Brownian-motion paths under SHARED Sobol'
//! draws and a Brownian-bridge construction, and prices a path-dependent
//! arithmetic-average Asian option. The exact f64 CPU oracle (reusing the
//! `celnet-qmc` Sobol'/bridge/inverse-normal core verbatim) reconciles it
//! node-by-node and runs headless/CI.
//!
//! # What this is (GPU-AT-SCALE G3)
//!
//! [`crate::gpu`] prices a single-step *terminal* (vanilla) Monte-Carlo; this is
//! the **multi-step path** lever (G3): a path-dependent product whose value
//! depends on the whole trajectory, priced by quasi-Monte-Carlo. The kernel
//! consumes the **same Sobol' direction numbers and the same Brownian-bridge
//! weight matrix** as the CPU `celnet-qmc` engine, so feeding the same Sobol'
//! index to CPU and GPU yields the same path — a **CPU↔GPU known-answer test**.
//! The Sobol' *integers* are pure `u32` gray-code XOR and are bit-identical on
//! both sides; the only divergence is the f32 inverse-normal/`exp` chain vs the
//! CPU f64, bounded node-by-node by [`derived_path_bound`] (derived from
//! `f32::EPSILON` and the path condition numbers — never a fitted constant).
//!
//! # HONEST BOUNDARY (verbatim)
//!
//! M4 Metal lacks f64 ⇒ the path math is **f32 by design**. In-repo this proves
//! **CORRECTNESS** (GPU-f32 == CPU-f64 within the derived bound, both == the
//! geometric-Asian closed form / `celnet-golden` chain within QMC error) and
//! **RATIOS only**. The **NVIDIA absolute throughput headline / ≤50ms exotic /
//! Workload-A/B absolute numbers are DEFERRED** to the CUDA deploy-gate. We
//! **never claim f64 on Metal**.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::backend::PricingBackend;
use crate::cpu::pairwise_sum;
use crate::gpu::{GpuBackend, GpuContextRef};

/// Workgroup size; must match `@workgroup_size` in `path.wgsl`.
const WG_SIZE: u32 = 256;

/// Maximum path step count; must match `MAX_STEPS` in `path.wgsl` (the kernel's
/// fixed-size local `z` array). Path-dependent FX products price on coarse
/// monitoring grids, so 64 steps is ample; a larger request fails loudly.
pub const MAX_STEPS: usize = 64;

/// Upper bound on the number of paths per dispatch, guarding the readback
/// allocation against a runaway request.
const MAX_PATHS: u64 = 8 * 1024 * 1024;

/// Bounded readback deadline (see the MC kernels for the always-terminates
/// rationale): a wedged device fails loudly rather than hanging.
const GPU_DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Specification of a multi-step arithmetic-average Asian Monte-Carlo batch.
///
/// The asset path is `S(t_l) = S₀·exp((μ−½σ²)·t_l + σ·W(t_l))` on the uniform
/// monitoring grid `t_l = l·T/m`, `l = 1..=m`; the payoff is
/// `max(±(Ā − K), 0)` on the arithmetic average `Ā = (1/m)·Σ S(t_l)`. The drift
/// is the Garman-Kohlhagen risk-neutral `μ = r_dom − r_for`; the present value
/// discounts by `e^{−r_dom·T}`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AsianPathSpec {
    /// Initial spot `S₀`.
    pub spot: f64,
    /// Strike `K` (on the average).
    pub strike: f64,
    /// Volatility `σ`.
    pub vol: f64,
    /// Time to expiry `T` (years).
    pub t: f64,
    /// Domestic continuously-compounded rate `r_dom` (discounting).
    pub r_dom: f64,
    /// Foreign continuously-compounded rate `r_for`.
    pub r_for: f64,
    /// `+1.0` call, `−1.0` put on the average.
    pub sign: f64,
    /// Number of monitoring steps `m` (`1..=`[`MAX_STEPS`]).
    pub steps: u32,
    /// Number of Sobol' QMC paths in the batch.
    pub paths: u32,
    /// First Sobol' index (gray-code offset). Skipping index 0 (whose point is
    /// the all-zero corner) is the standard QMC practice; the caller controls it.
    pub sobol_base: u32,
}

impl AsianPathSpec {
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

/// Uniform metadata block uploaded to `path.wgsl` (`PathMeta`): four `u32` then
/// eight `f32` = 48 bytes, a 16-byte multiple (std140-legal as a uniform).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PathMeta {
    paths: u32,
    steps: u32,
    sobol_base: u32,
    _p0: u32,
    ln_spot: f32,
    drift: f32,
    vol: f32,
    strike: f32,
    sign: f32,
    r_dom: f32,
    t_total: f32,
    _p1: f32,
}

/// Prices a multi-step Asian by quasi-Monte-Carlo, on the GPU when an adapter is
/// present and on the exact f64 CPU oracle otherwise (headless/CI). Holds a
/// [`GpuBackend`] (adapter probe + CPU fallback) and lazily builds the pipeline.
pub struct MultiStepPathPricer {
    backend: GpuBackend,
    pipeline: Option<PathPipeline>,
}

struct PathPipeline {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl core::fmt::Debug for MultiStepPathPricer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MultiStepPathPricer")
            .field("is_gpu", &self.is_gpu())
            .field("label", &self.backend.label())
            .finish()
    }
}

impl MultiStepPathPricer {
    /// Build a path pricer, probing for a GPU adapter (falling back to the exact
    /// f64 CPU oracle when none is present, so it runs headless in CI).
    #[must_use]
    pub fn new() -> Self {
        let backend = GpuBackend::new();
        let pipeline = backend
            .gpu_context()
            .map(|ctx| PathPipeline::build(ctx.device));
        Self { backend, pipeline }
    }

    /// `true` when a real GPU adapter is driving the path compute path.
    #[must_use]
    pub fn is_gpu(&self) -> bool {
        self.backend.is_gpu() && self.pipeline.is_some()
    }

    /// Human-readable backend identity (mirrors [`GpuBackend::label`]).
    #[must_use]
    pub fn label(&self) -> String {
        self.backend.label()
    }

    /// Per-path undiscounted Asian payoffs (one f32 per Sobol' point, widened to
    /// f64), row-aligned with Sobol' index `sobol_base + p`. Node-by-node — this
    /// is the GPU side of the CPU↔GPU reconciliation. The GPU path is used when
    /// an adapter is present; otherwise the exact f64 CPU oracle (narrowed to f32
    /// per path so the contract matches the shader).
    ///
    /// # Panics
    ///
    /// Panics if `paths` is zero / exceeds [`MAX_PATHS`] or `steps` is zero /
    /// exceeds [`MAX_STEPS`].
    #[must_use]
    pub fn path_payoffs(&self, spec: &AsianPathSpec) -> Vec<f64> {
        assert!(
            spec.paths >= 1 && u64::from(spec.paths) <= MAX_PATHS,
            "invalid path count {} (must be 1..={MAX_PATHS})",
            spec.paths
        );
        assert!(
            spec.steps >= 1 && spec.steps as usize <= MAX_STEPS,
            "invalid step count {} (must be 1..={MAX_STEPS})",
            spec.steps
        );

        match (self.backend.gpu_context(), self.pipeline.as_ref()) {
            (Some(ctx), Some(pipe)) => Self::dispatch(&ctx, pipe, spec),
            _ => cpu_path_payoffs(spec)
                .into_iter()
                .map(|p| f64::from(p as f32))
                .collect(),
        }
    }

    /// Price the Asian: discounted mean of the per-path payoffs, with the
    /// Monte-Carlo standard error of the price estimate.
    #[must_use]
    pub fn price_asian(&self, spec: &AsianPathSpec) -> (f64, f64) {
        let payoffs = self.path_payoffs(spec);
        let n = payoffs.len();
        let sum = pairwise_sum(&payoffs);
        let sq: Vec<f64> = payoffs.iter().map(|&x| x * x).collect();
        let sum_sq = pairwise_sum(&sq);
        let discount = spec.discount();
        let mean = sum / n as f64;
        let price = discount * mean;
        let std_error = if n >= 2 {
            let var = ((sum_sq - n as f64 * mean * mean) / (n as f64 - 1.0)).max(0.0);
            discount * celnet_core::math::sqrt(var / n as f64)
        } else {
            0.0
        };
        (price, std_error)
    }

    /// Run the GPU path: upload the shared Sobol' direction numbers + the
    /// Brownian-bridge weight matrix, dispatch one thread per path, read back one
    /// f32 payoff per path (widened to f64).
    fn dispatch(ctx: &GpuContextRef<'_>, pipe: &PathPipeline, spec: &AsianPathSpec) -> Vec<f64> {
        let (dir_nums, bridge_a) = shared_qmc_tables(spec);

        let meta = PathMeta {
            paths: spec.paths,
            steps: spec.steps,
            sobol_base: spec.sobol_base,
            _p0: 0,
            ln_spot: celnet_core::math::ln(spec.spot) as f32,
            drift: spec.drift() as f32,
            vol: spec.vol as f32,
            strike: spec.strike as f32,
            sign: spec.sign as f32,
            r_dom: spec.r_dom as f32,
            t_total: spec.t as f32,
            _p1: 0.0,
        };

        let meta_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-path-meta"),
                contents: bytemuck::bytes_of(&meta),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let dir_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-path-dirnums"),
                contents: bytemuck::cast_slice(&dir_nums),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let a_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-path-bridge-a"),
                contents: bytemuck::cast_slice(&bridge_a),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let out_len = spec.paths as usize;
        let out_bytes = (out_len * core::mem::size_of::<f32>()) as u64;
        let out_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-path-payoffs"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-path-readback"),
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("celnet-path-bg"),
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
                    resource: a_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: out_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("celnet-path-encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("celnet-path-pass"),
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

impl Default for MultiStepPathPricer {
    fn default() -> Self {
        Self::new()
    }
}

impl PathPipeline {
    fn build(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("celnet-path-asian"),
            source: wgpu::ShaderSource::Wgsl(include_str!("path.wgsl").into()),
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
            label: Some("celnet-path-bgl"),
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
                storage(2, true),  // bridge weight matrix (read-only)
                storage(3, false), // per-path payoffs (read-write)
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("celnet-path-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("celnet-path-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("path_asian"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }
}

/// Map the readback buffer under a bounded deadline; return f32 widened to f64.
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
                    "GPU path readback timed out after {GPU_DISPATCH_TIMEOUT:?} \
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
        let prices: &[f32] = bytemuck::cast_slice(&view);
        debug_assert_eq!(prices.len(), n);
        prices.iter().map(|&p| f64::from(p)).collect()
    };
    read_buf.unmap();
    out
}

/// Build the shared QMC tables for a spec: the Sobol' direction numbers
/// (`steps × 32` u32, row-major by dimension) and the Brownian-bridge weight
/// matrix `A` (`steps × steps` f32, row-major) — **both taken verbatim from the
/// `celnet-qmc` public API** so the GPU and CPU use identical tables.
fn shared_qmc_tables(spec: &AsianPathSpec) -> (Vec<u32>, Vec<f32>) {
    let m = spec.steps as usize;
    let seq = celnet_qmc::SobolSequence::new(m);
    let mut dir_nums = Vec::with_capacity(m * 32);
    for j in 0..m {
        dir_nums.extend_from_slice(seq.direction_numbers(j));
    }
    let bridge = celnet_qmc::BrownianBridge::new(m, spec.t);
    let a = bridge.weight_matrix();
    let mut a_flat = Vec::with_capacity(m * m);
    for row in &a {
        for &v in row {
            a_flat.push(v as f32);
        }
    }
    (dir_nums, a_flat)
}

/// **Exact f64 CPU oracle** for the per-path Asian payoffs, reusing the
/// `celnet-qmc` Sobol'/bridge/inverse-normal core verbatim. This is the ground
/// truth the f32 GPU result reconciles against node-by-node, and it is also the
/// f64 reference for the geometric-Asian closed-form bracket. The Sobol'
/// integers it consumes are identical (`point_u32`) to the GPU's `dir_nums`
/// gray-code XOR, so the two price the *same* QMC sample.
#[must_use]
pub fn cpu_path_payoffs(spec: &AsianPathSpec) -> Vec<f64> {
    let m = spec.steps as usize;
    let seq = celnet_qmc::SobolSequence::new(m);
    let bridge = celnet_qmc::BrownianBridge::new(m, spec.t);
    let a = bridge.weight_matrix();
    let ln_spot = celnet_core::math::ln(spec.spot);
    let drift = spec.drift();
    let half_var = 0.5 * spec.vol * spec.vol;
    let dt = spec.t / m as f64;

    (0..spec.paths)
        .map(|p| {
            let i = u64::from(spec.sobol_base) + u64::from(p);
            let pt = seq.point_u32(i);
            // u32 -> open-unit f64 (matches the shader's (u+0.5)*2^-32), then the
            // ALGORITHM-MATCHED (A&S-erf) inverse-normal — the f64 evaluation of
            // the kernel's exact algorithm — so the GPU↔CPU reconcile isolates
            // pure f32 round-off; the A&S-vs-libm inverse difference is bracketed
            // separately (as_normal::tests / the parity row), not folded in here.
            let z: Vec<f64> = pt
                .iter()
                .map(|&u| crate::as_normal::inv_norm_cdf_as((f64::from(u) + 0.5) / 4_294_967_296.0))
                .collect();
            // W(t_l) = sum_k A[l][k] z[k]; arithmetic-average payoff.
            let mut acc = 0.0;
            for (l, a_row) in a.iter().enumerate().take(m) {
                let w_l: f64 = a_row.iter().zip(z.iter()).map(|(&aik, &zk)| aik * zk).sum();
                let t_l = (l + 1) as f64 * dt;
                acc += celnet_core::math::exp(ln_spot + (drift - half_var) * t_l + spec.vol * w_l);
            }
            let avg = acc / m as f64;
            (spec.sign * (avg - spec.strike)).max(0.0)
        })
        .collect()
}

/// The per-path **max |z|** across the `m` Sobol' draws (the A&S-erf
/// inverse-normal of the same integers the GPU consumes). Sizes the per-path
/// reconciliation bound [`derived_path_bound_for_zmax`], which tracks the f32
/// inverse-normal's `1/φ(z)` tail sensitivity.
#[must_use]
pub fn cpu_path_zmax(spec: &AsianPathSpec) -> Vec<f64> {
    let m = spec.steps as usize;
    let seq = celnet_qmc::SobolSequence::new(m);
    (0..spec.paths)
        .map(|p| {
            let i = u64::from(spec.sobol_base) + u64::from(p);
            seq.point_u32(i)
                .iter()
                .map(|&u| {
                    crate::as_normal::inv_norm_cdf_as((f64::from(u) + 0.5) / 4_294_967_296.0).abs()
                })
                .fold(0.0_f64, f64::max)
        })
        .collect()
}

/// **Derived** f32-vs-f64 per-path reconciliation bound for the Asian payoff.
///
/// The GPU and the [`cpu_path_payoffs`] oracle evaluate the *same* QMC path
/// (bit-identical Sobol' integers, the same bridge matrix `A`, the same
/// inverse-normal + GBM algebra); the GPU does it in f32, the oracle in f64. So
/// the divergence is pure f32 round-off, bounded from `f32::EPSILON` and the
/// path's condition numbers — never a fitted constant.
///
/// # Per-path error model
///
/// Let `eps = f32::EPSILON ≈ 5.96e-8` and `φ(z) = (2π)^{−½}e^{−z²/2}`. For each
/// step the inverse-normal `z = Φ⁻¹(u)` (Acklam seed + one Halley step against
/// the A&S-erf `Φ`) carries `|Δz| ≲ (K_h·eps)/φ(z) + K_inv·eps·|z|` — the
/// dominant term being the f32 rounding of the near-cancellation `Φ(x)−p`
/// divided by `φ(z)` in the Halley correction (`K_h ≈ 2`, `K_inv ≈ 8`). The
/// bridge sum `W = Σ A·z` (`m` fused mul-adds) and `S = exp(ln S₀ + (μ−½σ²)t +
/// σW)` give `|ΔS| ≈ S·(σ·(|Δz|_max + m·eps·z_max) + (K_exp+2)·eps)`,
/// `K_exp ≈ 3`. Averaging `m` such `S` terms and the payoff `max`/subtract add a
/// few more `eps` relative. Because both estimators price the *same* sample the
/// per-path errors are common (they do not average down like MC noise); the
/// per-path bound is sized by **that path's max |z|** across its `m` steps
/// (`z_path`), so it tracks the genuine f32 inverse-normal tail sensitivity
/// rather than a single worst case — see [`derived_path_bound_for_zmax`].
///
/// [`derived_path_bound`] is the z-independent convenience envelope at a `z = 5`
/// core cap; the node-by-node parity row uses the per-path
/// [`derived_path_bound_for_zmax`] (it knows each path's max |z|).
#[must_use]
pub fn derived_path_bound_for_zmax(spec: &AsianPathSpec, z_path: f64) -> f64 {
    let eps = f64::from(f32::EPSILON);
    let k_h = 2.0; // Halley near-cancellation rounding.
    let k_inv = 8.0; // inverse-normal rational + remaining ops.
    let k_exp = 3.0; // exp.
    let k_payoff = 6.0; // bridge sum, average, subtract, max, widen.
    let m = f64::from(spec.steps);
    let z = z_path.abs().max(1.0);
    let phi = celnet_core::math::norm_pdf(z).max(1e-12);
    let dz = (k_h * eps) / phi + k_inv * eps * z;
    // S_T relative round-off (the same |Δz| applies to each step's W via A).
    let rel = spec.vol * (dz + m * eps * z) + f64::from(k_exp as u32) * eps + k_payoff * eps;
    let expected_st = spec.spot * celnet_core::math::exp(spec.drift() * spec.t);
    let scale = expected_st.max(spec.strike);
    let abs_floor = 16.0 * eps;
    rel * scale + abs_floor
}

/// Convenience z-independent envelope of [`derived_path_bound_for_zmax`] at a
/// `z = 5` core cap. The per-path form is tighter and is what the node-by-node
/// parity row uses.
#[must_use]
pub fn derived_path_bound(spec: &AsianPathSpec) -> f64 {
    derived_path_bound_for_zmax(spec, 5.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Probe (independently of the pricer) whether a real GPU adapter exists, so
    /// the reconciliation can require the shader path when hardware is present.
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
            spot: 1.20,
            strike: 1.25,
            vol: 0.11,
            t: 0.75,
            r_dom: 0.03,
            r_for: 0.01,
            sign: 1.0,
            steps,
            paths,
            sobol_base: 1,
        }
    }

    /// CPU↔GPU node-by-node reconciliation: the GPU f32 per-path payoffs match
    /// the exact f64 CPU oracle (same Sobol' integers, same bridge) within the
    /// derived f32 bound — path by path, not just the aggregate. On a host with
    /// an adapter the shader path MUST be exercised (not a vacuous CPU-vs-CPU).
    #[test]
    fn gpu_path_reconciles_node_by_node() {
        let spec = demo_spec(WG_SIZE * 16, 8);
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
        for (i, (&g, &c)) in gpu.iter().zip(cpu.iter()).enumerate() {
            let bound = derived_path_bound_for_zmax(&spec, zmax[i]);
            assert!(
                (g - c).abs() <= bound,
                "path {i}: gpu {g} vs cpu {c} exceeds derived bound {bound} (zmax={}, is_gpu={})",
                zmax[i],
                pricer.is_gpu()
            );
        }
    }

    /// The discounted QMC price converges to the geometric-Asian closed form?
    /// No — arithmetic ≥ geometric — but it must bracket the geometric price
    /// from above and agree with a large independent plain-MC arithmetic price
    /// within MC error. Here we sanity-check the price is positive, below the
    /// spot, and reproducible.
    #[test]
    fn price_is_reproducible_and_sane() {
        let spec = demo_spec(WG_SIZE * 32, 8);
        let pricer = MultiStepPathPricer::new();
        let (p1, se) = pricer.price_asian(&spec);
        let (p2, _) = pricer.price_asian(&spec);
        assert!(p1 > 0.0 && p1 < spec.spot, "price out of range: {p1}");
        assert!(se >= 0.0);
        // QMC is deterministic for a fixed base/scramble-free draw ⇒ bit-stable.
        assert_eq!(p1.to_bits(), p2.to_bits());
    }
}
