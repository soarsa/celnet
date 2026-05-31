//! Batched **scenario** pricing — one call prices an entire spot×vol shock grid
//! of a single vanilla, vectorised over the grid in one GPU dispatch (with the
//! exact f64 CPU fallback the crate already carries for headless/CI hosts).
//!
//! # What this is for
//!
//! This is the kernel behind two hot, fan-out-shaped workloads:
//!
//! * the **risk-cube bump-and-revalue** scenarios (a spot ladder × vol ladder
//!   re-price of every position), and
//! * the **GUI Risk grid** (the spot×vol scenario matrix a trader scrubs).
//!
//! Both ask the same question — *price this vanilla at every node of a
//! `n_spot × n_vol` grid of shocks* — and both want it priced **node-by-node
//! under common random numbers** so the resulting surface is smooth (a clean
//! finite-difference Greek / scenario ladder) rather than dominated by
//! independent Monte-Carlo noise. The batched kernel guarantees exactly that:
//! every node reuses the same Philox normal `Z_p` for path `p`, so the only
//! thing that varies across the grid is the deterministic GBM mapping of `Z_p`.
//!
//! # Grid convention
//!
//! A node `(i, j)` shocks the base inputs by
//!
//! * `spot' = base_spot · spot_mult[i]` — a **multiplicative** spot shock
//!   (`1.0` = unshocked; `0.99`/`1.01` = ∓1% spot ladder rungs), and
//! * `vol' = base_vol + vol_bump[j]` — an **additive** vol bump
//!   (`0.0` = unshocked; `±0.0025` = ∓25 bp vega ladder rungs).
//!
//! These are the market-standard ladder parametrisations (spot in percent,
//! vol in absolute vol points), matching `docs/RISK-HIERARCHY.md` §3.3 and the
//! GUI Risk grid axes. Drift `μ = r_dom − r_for` and the discount are held at
//! the base level: a spot/vol scenario re-prices the option at a shocked market
//! state, not a shocked rate environment (rate ladders are a separate axis).
//!
//! # Layout of the result
//!
//! [`ScenarioGrid`] is a flat **row-major** (`node = i·n_vol + j`) array of
//! [`Reduction`]s — the same per-node aggregate the single-node path returns —
//! so a caller reads `grid.node(i, j).price()` for the discounted scenario PV,
//! `.std_error()` for the per-node Monte-Carlo error, etc. The discount is the
//! base discount, identical for every node.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::backend::{PathSpec, PayoffKernel, PricingBackend, Reduction};
use crate::counter_rng::CounterNormals;
use crate::cpu::pairwise_sum;
use crate::gpu::GpuBackend;

/// Workgroup size along the path axis; must match `@workgroup_size` in
/// `scenario.wgsl` and the single-node `shader.wgsl`.
const WG_SIZE: u32 = 256;

/// Bounded readback deadline for a batched dispatch (see the single-node kernel
/// for the rationale): a wedged device fails loudly rather than hanging,
/// preserving the platform's always-terminates posture. A batched grid is more
/// work than a single node, so the budget is proportionally generous.
const GPU_DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Upper bound on grid size, guarding the readback allocation and the
/// `n_spot · n_vol · x_blocks` slot arithmetic against overflow / runaway
/// dispatch. A 4096×4096 grid is already vastly larger than any risk ladder
/// (typical grids are ~21×21); this is a sanity rail, not a product limit.
const MAX_GRID_NODES: u64 = 4096 * 4096;

/// A scenario shock grid: the multiplicative spot rungs and additive vol rungs
/// that define the `n_spot × n_vol` matrix of nodes to price.
///
/// Construct directly from the two axes, or with [`ScenarioAxes::ladders`] for
/// the common symmetric percent/vol-point ladders centred on the unshocked
/// point. The axes are plain data and carry no state.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioAxes {
    /// Multiplicative spot shocks (`1.0` = unshocked). One per grid row.
    pub spot_mult: Vec<f32>,
    /// Additive vol bumps in absolute vol points (`0.0` = unshocked). One per
    /// grid column.
    pub vol_bump: Vec<f32>,
}

impl ScenarioAxes {
    /// Build axes directly from explicit shock vectors.
    ///
    /// Both axes must be non-empty; an empty axis yields a degenerate
    /// zero-column/row grid that no caller wants, so it is rejected at
    /// construction via [`Self::is_valid`] (checked by [`ScenarioPricer`]).
    #[must_use]
    pub fn new(spot_mult: Vec<f32>, vol_bump: Vec<f32>) -> Self {
        Self {
            spot_mult,
            vol_bump,
        }
    }

    /// Symmetric ladders centred on the unshocked point.
    ///
    /// Produces `2·spot_rungs + 1` spot multipliers
    /// `{1 − spot_rungs·spot_step, …, 1, …, 1 + spot_rungs·spot_step}` and
    /// `2·vol_rungs + 1` vol bumps `{−vol_rungs·vol_step, …, 0, …,
    /// +vol_rungs·vol_step}` — the canonical centred FD/scenario ladders. The
    /// centre node `(spot_rungs, vol_rungs)` is the unshocked base price.
    #[must_use]
    pub fn ladders(spot_rungs: u32, spot_step: f64, vol_rungs: u32, vol_step: f64) -> Self {
        let spot_mult = (0..=2 * spot_rungs)
            .map(|i| (1.0 + (f64::from(i) - f64::from(spot_rungs)) * spot_step) as f32)
            .collect();
        let vol_bump = (0..=2 * vol_rungs)
            .map(|j| ((f64::from(j) - f64::from(vol_rungs)) * vol_step) as f32)
            .collect();
        Self {
            spot_mult,
            vol_bump,
        }
    }

    /// Number of spot rows.
    #[must_use]
    pub fn n_spot(&self) -> u32 {
        self.spot_mult.len() as u32
    }

    /// Number of vol columns.
    #[must_use]
    pub fn n_vol(&self) -> u32 {
        self.vol_bump.len() as u32
    }

    /// Total node count `n_spot · n_vol`.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.n_spot() * self.n_vol()
    }

    /// Both axes are non-empty and the grid is within [`MAX_GRID_NODES`].
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.spot_mult.is_empty()
            && !self.vol_bump.is_empty()
            && u64::from(self.n_spot()) * u64::from(self.n_vol()) <= MAX_GRID_NODES
    }
}

/// A priced scenario grid: a flat row-major (`node = i·n_vol + j`) array of
/// per-node [`Reduction`]s.
///
/// Each entry is the same aggregate the single-node pricer returns, so the full
/// `Reduction` API (`price`, `mean`, `std_error`) is available per node.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioGrid {
    /// Number of spot rows.
    pub n_spot: u32,
    /// Number of vol columns.
    pub n_vol: u32,
    /// Row-major per-node reductions, length `n_spot · n_vol`.
    pub nodes: Vec<Reduction>,
}

impl ScenarioGrid {
    /// Borrow the reduction at grid coordinate `(i, j)` (`i` = spot row, `j` =
    /// vol column). Panics on out-of-range coordinates — a programming error,
    /// not a runtime condition.
    #[must_use]
    pub fn node(&self, i: u32, j: u32) -> &Reduction {
        assert!(
            i < self.n_spot && j < self.n_vol,
            "scenario node ({i},{j}) out of {}x{} grid",
            self.n_spot,
            self.n_vol
        );
        &self.nodes[(i * self.n_vol + j) as usize]
    }
}

/// Uniform parameter block for the batched scenario shader. `#[repr(C)]` + `Pod`
/// so `bytemuck` casts it to bytes with the exact WGSL `struct Params` layout
/// (five f32 then sign, then seven u32 — five f32 + sign f32 = six f32, then
/// seven u32). Kept tightly packed and 16-byte-tail-aligned by the trailing u32
/// run.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ScenarioParams {
    base_spot: f32,
    base_vol: f32,
    mu_drift: f32,
    t: f32,
    sqrt_t: f32,
    strike: f32,
    sign: f32,
    paths: u32,
    n_spot: u32,
    n_vol: u32,
    x_blocks: u32,
    seed_lo: u32,
    seed_hi: u32,
    // Pad to a 16-byte multiple (13 fields × 4 = 52 → pad 3 × u32 = 64 bytes)
    // so the uniform buffer's std140-ish tail alignment is satisfied on every
    // backend without relying on driver-specific tail padding.
    _pad: [u32; 3],
}

/// Prices a [`ScenarioAxes`] grid for a base vanilla, on the GPU when an adapter
/// is present and on the exact f64 CPU oracle otherwise.
///
/// Holds a [`GpuBackend`] (which itself owns the adapter probe + CPU fallback)
/// and lazily builds the batched compute pipeline the first time a GPU dispatch
/// is needed. Construction is off any hot path.
pub struct ScenarioPricer {
    backend: GpuBackend,
    pipeline: Option<ScenarioPipeline>,
}

/// The batched-scenario compute pipeline, built once over the [`GpuBackend`]'s
/// device. Separate from the single-node pipeline the backend owns so neither
/// kernel perturbs the other.
struct ScenarioPipeline {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl core::fmt::Debug for ScenarioPricer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScenarioPricer")
            .field("is_gpu", &self.backend.is_gpu())
            .field("label", &self.backend.label())
            .finish()
    }
}

impl ScenarioPricer {
    /// Build a scenario pricer, probing for a GPU adapter (falling back to the
    /// CPU oracle when none is present, so it runs headless in CI).
    #[must_use]
    pub fn new() -> Self {
        let backend = GpuBackend::new();
        let pipeline = backend
            .gpu_context()
            .map(|ctx| ScenarioPipeline::build(ctx.device));
        Self { backend, pipeline }
    }

    /// `true` when a real GPU adapter is driving the batched compute path.
    #[must_use]
    pub fn is_gpu(&self) -> bool {
        self.backend.is_gpu() && self.pipeline.is_some()
    }

    /// Human-readable backend identity (mirrors [`GpuBackend::label`]).
    #[must_use]
    pub fn label(&self) -> String {
        self.backend.label()
    }

    /// Price an entire `n_spot × n_vol` scenario grid of `base` under `payoff`
    /// in one batched dispatch, returning a row-major [`ScenarioGrid`].
    ///
    /// All nodes share `base.seed`/`base.paths`, so the grid is priced under
    /// **common random numbers** (a smooth risk surface). The GPU path is used
    /// when an adapter is present; otherwise the exact f64 CPU oracle prices the
    /// grid node-by-node (bit-identical to looping the single-node CPU backend).
    ///
    /// # Panics
    ///
    /// Panics if `axes.is_valid()` is false (empty axis or a grid exceeding
    /// [`MAX_GRID_NODES`]) — a caller-side configuration error surfaced loudly
    /// rather than silently producing a degenerate grid.
    #[must_use]
    pub fn price_scenario_batch(
        &self,
        base: &PathSpec,
        payoff: &PayoffKernel,
        axes: &ScenarioAxes,
    ) -> ScenarioGrid {
        assert!(
            axes.is_valid(),
            "invalid scenario axes: spot={} vol={} (empty axis or grid > {MAX_GRID_NODES} nodes)",
            axes.spot_mult.len(),
            axes.vol_bump.len()
        );

        match (self.backend.gpu_context(), self.pipeline.as_ref()) {
            (Some(ctx), Some(pipe)) => Self::dispatch(&ctx, pipe, base, payoff, axes),
            // No adapter (or pipeline build failed): exact f64 CPU oracle.
            _ => Self::cpu_grid(base, payoff, axes),
        }
    }

    /// Exact f64 CPU oracle: price every node node-by-node with the single-node
    /// CPU backend, applying the same shocks the shader applies. Bit-identical
    /// to what a caller would get looping [`CpuBackend::price_vanilla`] over the
    /// shocked specs, and the reconciliation ground truth for the GPU grid.
    fn cpu_grid(base: &PathSpec, payoff: &PayoffKernel, axes: &ScenarioAxes) -> ScenarioGrid {
        // Simulate the shared base normals once: every node reuses Z_p for path
        // p (common random numbers), so we draw the stream a single time and
        // remap it per node rather than re-drawing per node.
        let rng = CounterNormals::new(base.seed);
        let normals: Vec<f64> = (0..base.paths).map(|p| rng.normal(p, 0, 0)).collect();
        let discount = celnet_core::math::exp(-base.r_dom * base.t);
        let sqrt_t = celnet_core::math::sqrt(base.t);

        let mut nodes = Vec::with_capacity(axes.nodes() as usize);
        for &sm in &axes.spot_mult {
            let spot = base.spot * f64::from(sm);
            let ln_spot = celnet_core::math::ln(spot);
            for &vb in &axes.vol_bump {
                let vol = base.vol + f64::from(vb);
                let half_var = 0.5 * vol * vol;
                let vol_sqrt_t = vol * sqrt_t;
                let mu_term = (base.drift - half_var) * base.t;

                // Reduce the payoff over the shared normals under this node's GBM.
                let pay: Vec<f64> = normals
                    .iter()
                    .map(|&z| {
                        let ln_st = ln_spot + mu_term + vol_sqrt_t * z;
                        payoff.evaluate(celnet_core::math::exp(ln_st))
                    })
                    .collect();
                let paysq: Vec<f64> = pay.iter().map(|&x| x * x).collect();
                nodes.push(Reduction {
                    paths: base.paths,
                    sum: pairwise_sum(&pay),
                    sum_sq: pairwise_sum(&paysq),
                    discount,
                });
            }
        }
        ScenarioGrid {
            n_spot: axes.n_spot(),
            n_vol: axes.n_vol(),
            nodes,
        }
    }

    /// Run the batched GPU compute path: one dispatch over (path-block × node),
    /// then finish each node's reduction across path-blocks on the host with the
    /// deterministic pairwise sum.
    fn dispatch(
        ctx: &crate::gpu::GpuContextRef<'_>,
        pipe: &ScenarioPipeline,
        base: &PathSpec,
        payoff: &PayoffKernel,
        axes: &ScenarioAxes,
    ) -> ScenarioGrid {
        let x_blocks = base.paths.div_ceil(WG_SIZE).max(1);
        let n_spot = axes.n_spot();
        let n_vol = axes.n_vol();
        let total_nodes = n_spot * n_vol;

        let params = ScenarioParams {
            base_spot: base.spot as f32,
            base_vol: base.vol as f32,
            mu_drift: base.drift as f32,
            t: base.t as f32,
            sqrt_t: celnet_core::math::sqrt(base.t) as f32,
            strike: payoff.strike as f32,
            sign: payoff.sign as f32,
            paths: base.paths,
            n_spot,
            n_vol,
            x_blocks,
            seed_lo: base.seed as u32,
            seed_hi: (base.seed >> 32) as u32,
            _pad: [0; 3],
        };

        let param_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-scenario-params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let spot_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-scenario-spot-mult"),
                contents: bytemuck::cast_slice(&axes.spot_mult),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let vol_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-scenario-vol-bump"),
                contents: bytemuck::cast_slice(&axes.vol_bump),
                usage: wgpu::BufferUsages::STORAGE,
            });

        // Two f32 partials per (node, x-block).
        let out_len = (2 * total_nodes * x_blocks) as usize;
        let out_bytes = (out_len * core::mem::size_of::<f32>()) as u64;

        let out_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-scenario-group-sums"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-scenario-readback"),
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("celnet-scenario-bg"),
            layout: &pipe.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: param_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: spot_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: vol_buf.as_entire_binding(),
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
                label: Some("celnet-scenario-encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("celnet-scenario-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipe.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            // x = path-blocks, y = nodes: the whole grid in one dispatch.
            pass.dispatch_workgroups(x_blocks, total_nodes, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &read_buf, 0, out_bytes);
        ctx.queue.submit(Some(encoder.finish()));

        let partials = Self::map_readback(ctx, &read_buf, out_len);

        // Finish each node's reduction across its x-blocks with the same
        // deterministic pairwise sum the CPU oracle uses. Node n's partials
        // occupy [n*x_blocks*2, (n+1)*x_blocks*2) interleaved (sum, sum_sq).
        let discount = celnet_core::math::exp(-base.r_dom * base.t);
        let xb = x_blocks as usize;
        let mut nodes = Vec::with_capacity(total_nodes as usize);
        for n in 0..total_nodes as usize {
            let base_off = n * xb * 2;
            let sums: Vec<f64> = (0..xb)
                .map(|b| f64::from(partials[base_off + b * 2]))
                .collect();
            let sqs: Vec<f64> = (0..xb)
                .map(|b| f64::from(partials[base_off + b * 2 + 1]))
                .collect();
            nodes.push(Reduction {
                paths: base.paths,
                sum: pairwise_sum(&sums),
                sum_sq: pairwise_sum(&sqs),
                discount,
            });
        }
        ScenarioGrid {
            n_spot,
            n_vol,
            nodes,
        }
    }

    /// Map the readback buffer under a bounded deadline and return the f32
    /// partials (see the single-node kernel for the always-terminates rationale).
    fn map_readback(
        ctx: &crate::gpu::GpuContextRef<'_>,
        read_buf: &wgpu::Buffer,
        out_len: usize,
    ) -> Vec<f32> {
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
                        "GPU scenario readback timed out after {GPU_DISPATCH_TIMEOUT:?} \
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
            let partials: &[f32] = bytemuck::cast_slice(&view);
            debug_assert_eq!(partials.len(), out_len);
            partials.to_vec()
        };
        read_buf.unmap();
        out
    }
}

impl ScenarioPipeline {
    /// Compile the batched-scenario shader and build its pipeline + bind-group
    /// layout over `device`.
    fn build(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("celnet-mc-scenario"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scenario.wgsl").into()),
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
            label: Some("celnet-scenario-bgl"),
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
                storage(1, true),  // spot_mult (read-only)
                storage(2, true),  // vol_bump (read-only)
                storage(3, false), // group_sums (read-write)
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("celnet-scenario-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("celnet-scenario-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("mc_scenario"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }
}

impl Default for ScenarioPricer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::CpuBackend;

    /// Probe whether a GPU adapter is available (mirrors the backend probe), so a
    /// reconciliation test can hard-require the shader path on real hardware and
    /// adapt on headless hosts.
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

    /// Per-node f32-vs-f64 reconciliation bound, identical in form to the
    /// single-node kernel's derived bound (`crate::gpu`): the GPU and CPU price
    /// the *same realized sample* (shared Philox integers), so the divergence is
    /// purely f32 vs f64 transcendental/arithmetic round-off and inherits the
    /// node's relative price scale rather than averaging down like independent
    /// MC noise. Derived from `f32::EPSILON` and the node's path statistics.
    fn node_bound(spot: f64, vol: f64, t: f64, drift: f64, strike: f64, discount: f64) -> f64 {
        let eps = f64::from(f32::EPSILON);
        let k_bm = 8.0;
        let k_exp = 3.0;
        let k_payoff = 4.0;
        let z_max = 6.0;
        let sigma_sqrt_t = vol * celnet_core::math::sqrt(t);
        let rel = ((k_bm + 4.0) * sigma_sqrt_t * z_max + k_exp + k_payoff) * eps;
        let expected_st = spot * celnet_core::math::exp(drift * t);
        let scale = expected_st.max(strike);
        let abs_floor = 16.0 * eps;
        discount * (rel * scale + abs_floor)
    }

    /// `ScenarioAxes::ladders` produces centred, unshocked-at-centre ladders.
    #[test]
    fn ladders_are_centred() {
        let axes = ScenarioAxes::ladders(2, 0.01, 3, 0.0025);
        assert_eq!(axes.n_spot(), 5);
        assert_eq!(axes.n_vol(), 7);
        // Centre is unshocked: spot_mult==1, vol_bump==0.
        celnet_core::assert_close!(f64::from(axes.spot_mult[2]), 1.0, 1e-7, 1e-7);
        celnet_core::assert_close!(f64::from(axes.vol_bump[3]), 0.0, 1e-7, 1e-7);
        // Symmetric extremes.
        celnet_core::assert_close!(f64::from(axes.spot_mult[0]), 0.98, 1e-6, 1e-6);
        celnet_core::assert_close!(f64::from(axes.spot_mult[4]), 1.02, 1e-6, 1e-6);
        assert!(axes.is_valid());
    }

    /// The CPU oracle grid's centre node (unshocked) reproduces the single-node
    /// CPU backend exactly — the batched path is a faithful generalisation, not
    /// a separate model.
    #[test]
    fn cpu_centre_node_matches_single_node_backend() {
        let base = PathSpec::gbm(1.20, 0.11, 0.75, 0.03, 0.01, 200_000, 1, 0x5EED_0042);
        let payoff = PayoffKernel::call(1.25);
        let axes = ScenarioAxes::ladders(2, 0.01, 2, 0.0025);

        let grid = ScenarioPricer::cpu_grid(&base, &payoff, &axes);
        let single = CpuBackend.price_vanilla(&base, &payoff);

        // Centre node (i=2,j=2) is the unshocked base.
        let centre = grid.node(2, 2);
        celnet_core::assert_close!(centre.price(), single.price(), 1e-15, 1e-15);
        celnet_core::assert_close!(centre.sum, single.sum, 1e-15, 1e-15);
    }

    /// A shocked CPU-oracle node matches an independently constructed single-node
    /// backend run on the explicitly-shocked spec under the SAME seed — confirming
    /// the common-random-numbers remap is exactly a shocked re-price.
    #[test]
    fn cpu_shocked_node_matches_explicit_shocked_spec() {
        let base = PathSpec::gbm(1.20, 0.11, 0.75, 0.03, 0.01, 200_000, 1, 0x5EED_0042);
        let payoff = PayoffKernel::call(1.25);
        let axes = ScenarioAxes::new(vec![0.97, 1.0, 1.03], vec![-0.005, 0.0, 0.01]);

        let grid = ScenarioPricer::cpu_grid(&base, &payoff, &axes);

        // Node (0, 2): spot ×0.97, vol +0.01. Build the equivalent explicit spec
        // using the SAME f32-widened shocks the grid stores (the axes hold f32),
        // so the comparison is bit-identical rather than merely algebraically
        // equal: any divergence beyond f64 round-off would be a remap bug.
        let sm = f64::from(axes.spot_mult[0]); // 0.97 as stored f32, widened
        let vb = f64::from(axes.vol_bump[2]); // 0.01 as stored f32, widened
        let r_for = base.r_dom - base.drift;
        let shocked = PathSpec::gbm(
            base.spot * sm,
            base.vol + vb,
            base.t,
            base.r_dom,
            r_for,
            base.paths,
            1,
            base.seed,
        );
        let explicit = CpuBackend.price_vanilla(&shocked, &payoff);
        let node = grid.node(0, 2);
        // Algebraically the same shocked re-price; differs only by f64 operation
        // grouping (the grid pre-folds `(μ−½σ²)T`; the single-node path folds it
        // inline), bounded well under 1e-12 relative on a ~0.02 price.
        celnet_core::assert_close!(node.price(), explicit.price(), 1e-12, 1e-15);
    }

    /// Common random numbers across the grid make the surface smooth and
    /// monotone in spot for a call (higher spot ⇒ higher call price), with no
    /// Monte-Carlo noise crossings between adjacent rungs.
    #[test]
    fn grid_is_monotone_in_spot_under_crn() {
        let base = PathSpec::gbm(1.20, 0.11, 0.75, 0.03, 0.01, 200_000, 1, 0x5EED_0099);
        let payoff = PayoffKernel::call(1.20);
        let axes = ScenarioAxes::ladders(5, 0.01, 0, 0.0);

        let grid = ScenarioPricer::cpu_grid(&base, &payoff, &axes);
        // Single vol column (j=0); call price strictly increases with spot.
        for i in 1..grid.n_spot {
            let lo = grid.node(i - 1, 0).price();
            let hi = grid.node(i, 0).price();
            assert!(
                hi > lo,
                "call price not monotone in spot under CRN: node {} = {lo}, node {} = {hi}",
                i - 1,
                i
            );
        }
    }

    /// End-to-end reconciliation: the batched GPU (f32) grid matches the f64 CPU
    /// oracle grid node-by-node within the per-node derived f32 bound. On a host
    /// with an adapter the shader path is genuinely exercised (asserts is_gpu);
    /// on a headless host both sides are the exact CPU oracle and match to ~1e-15.
    #[test]
    fn gpu_grid_reconciles_with_cpu_oracle() {
        let base = PathSpec::gbm(1.20, 0.11, 0.75, 0.03, 0.01, WG_SIZE * 256, 1, 0x5EED_CAFE);
        let payoff = PayoffKernel::call(1.25);
        let axes = ScenarioAxes::ladders(3, 0.01, 3, 0.0025); // 7×7 = 49 nodes

        let pricer = ScenarioPricer::new();
        if adapter_is_available() {
            assert!(
                pricer.is_gpu(),
                "a GPU adapter is present but the scenario pricer fell back to CPU: {}",
                pricer.label()
            );
        }
        let gpu_grid = pricer.price_scenario_batch(&base, &payoff, &axes);
        let cpu_grid = ScenarioPricer::cpu_grid(&base, &payoff, &axes);

        assert_eq!(gpu_grid.nodes.len(), cpu_grid.nodes.len());
        assert_eq!(gpu_grid.nodes.len(), axes.nodes() as usize);

        for i in 0..axes.n_spot() {
            for j in 0..axes.n_vol() {
                let g = gpu_grid.node(i, j);
                let c = cpu_grid.node(i, j);
                let spot = base.spot * f64::from(axes.spot_mult[i as usize]);
                let vol = base.vol + f64::from(axes.vol_bump[j as usize]);
                let bound = node_bound(spot, vol, base.t, base.drift, payoff.strike, c.discount);
                celnet_core::assert_close!(g.price(), c.price(), 0.0, bound);
            }
        }
    }

    /// Ragged path count (not a multiple of WG_SIZE) still reconciles: exercises
    /// the shader's `path < params.paths` tail guard and the host
    /// `div_ceil(...).max(1)` x-block count per node on a real device.
    #[test]
    fn gpu_grid_honors_ragged_path_count() {
        let paths: u32 = 300_007;
        assert_ne!(paths % WG_SIZE, 0, "test must use a ragged count");
        let base = PathSpec::gbm(1.30, 0.13, 1.0, 0.02, 0.015, paths, 1, 0x0DD_C0FF);
        let payoff = PayoffKernel::put(1.25);
        let axes = ScenarioAxes::ladders(2, 0.015, 2, 0.005); // 5×5 = 25 nodes

        let pricer = ScenarioPricer::new();
        if adapter_is_available() {
            assert!(
                pricer.is_gpu(),
                "GPU present but fell back: {}",
                pricer.label()
            );
        }
        let gpu_grid = pricer.price_scenario_batch(&base, &payoff, &axes);
        let cpu_grid = ScenarioPricer::cpu_grid(&base, &payoff, &axes);

        for i in 0..axes.n_spot() {
            for j in 0..axes.n_vol() {
                let g = gpu_grid.node(i, j);
                let c = cpu_grid.node(i, j);
                assert_eq!(g.paths, paths);
                let spot = base.spot * f64::from(axes.spot_mult[i as usize]);
                let vol = base.vol + f64::from(axes.vol_bump[j as usize]);
                let bound = node_bound(spot, vol, base.t, base.drift, payoff.strike, c.discount);
                celnet_core::assert_close!(g.price(), c.price(), 0.0, bound);
            }
        }
    }

    /// The CPU oracle grid is fully reproducible (deterministic Philox + libm):
    /// two runs are bit-identical.
    #[test]
    fn cpu_grid_is_reproducible() {
        let base = PathSpec::gbm(1.20, 0.10, 0.5, 0.03, 0.01, 50_000, 1, 0xABCD_1234);
        let payoff = PayoffKernel::call(1.22);
        let axes = ScenarioAxes::ladders(2, 0.01, 2, 0.002);
        let a = ScenarioPricer::cpu_grid(&base, &payoff, &axes);
        let b = ScenarioPricer::cpu_grid(&base, &payoff, &axes);
        assert_eq!(a, b);
    }

    /// A 1×1 grid at the unshocked point degenerates to the single-node price.
    #[test]
    fn singleton_grid_is_single_price() {
        let base = PathSpec::gbm(1.20, 0.11, 0.75, 0.03, 0.01, 100_000, 1, 0x5EED_0007);
        let payoff = PayoffKernel::call(1.25);
        let axes = ScenarioAxes::new(vec![1.0], vec![0.0]);
        assert_eq!(axes.nodes(), 1);

        let pricer = ScenarioPricer::new();
        let grid = pricer.price_scenario_batch(&base, &payoff, &axes);
        let single = CpuBackend.price_vanilla(&base, &payoff);
        let bound = node_bound(
            base.spot,
            base.vol,
            base.t,
            base.drift,
            payoff.strike,
            grid.node(0, 0).discount,
        );
        celnet_core::assert_close!(grid.node(0, 0).price(), single.price(), 0.0, bound);
    }

    /// An invalid (empty) axis is rejected loudly.
    #[test]
    #[should_panic(expected = "invalid scenario axes")]
    fn empty_axis_panics() {
        let base = PathSpec::gbm(1.20, 0.11, 0.75, 0.03, 0.01, 1024, 1, 1);
        let payoff = PayoffKernel::call(1.25);
        let axes = ScenarioAxes::new(vec![], vec![0.0]);
        let _ = ScenarioPricer::new().price_scenario_batch(&base, &payoff, &axes);
    }
}
