//! [`GpuBackend`] — a wgpu (Metal/Vulkan/DX12) f32 Monte-Carlo compute path with
//! a transparent CPU fallback.
//!
//! At construction the backend probes for a GPU adapter via wgpu. If one is found
//! it compiles [`shader.wgsl`](../src/shader.wgsl) and runs the vanilla GBM
//! Monte-Carlo on-device in f32, drawing from the *same* counter-based Philox
//! stream as [`crate::CpuBackend`]; only the integer→float conversion and the
//! payoff arithmetic differ (f32 vs f64), which is the documented reconciliation
//! bound. If **no adapter is available** (headless CI, no GPU), the backend holds
//! a [`CpuBackend`] and transparently produces the f64 reference result, so the
//! same code path runs everywhere.
//!
//! Payoff accumulation never uses float atomics (WGSL atomics are integer-only):
//! each workgroup tree-reduces in shared memory and writes one partial sum; the
//! host finishes with the deterministic pairwise sum from [`crate::cpu`].

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::backend::{PathSpec, PayoffKernel, PricingBackend, Reduction};
use crate::cpu::{CpuBackend, pairwise_sum};

/// Workgroup size; must match `@workgroup_size` in `shader.wgsl`.
const WG_SIZE: u32 = 256;

/// Uniform parameter block uploaded to the shader. `#[repr(C)]` + `Pod` so
/// `bytemuck` can cast it to bytes with the exact WGSL `struct Params` layout
/// (five f32 then three u32 — 32 bytes, naturally 16-byte aligned).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GpuParams {
    ln_spot: f32,
    mu_term: f32,
    vol_sqrt_t: f32,
    strike: f32,
    sign: f32,
    paths: u32,
    seed_lo: u32,
    seed_hi: u32,
}

/// Live wgpu device state for the GPU path.
struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    backend_name: String,
}

/// Cross-platform GPU Monte-Carlo backend (f32) with a CPU fallback oracle.
///
/// Either runs on a real GPU adapter or delegates to the embedded
/// [`CpuBackend`]; [`Self::is_gpu`] reports which.
pub struct GpuBackend {
    context: Option<GpuContext>,
    fallback: CpuBackend,
}

impl core::fmt::Debug for GpuBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuBackend")
            .field("is_gpu", &self.is_gpu())
            .field(
                "adapter",
                &self.context.as_ref().map(|c| c.backend_name.as_str()),
            )
            .finish()
    }
}

impl GpuBackend {
    /// Probe for a GPU adapter and build the backend, falling back to the CPU
    /// oracle when no adapter is available (so it runs headless in CI).
    ///
    /// Blocks on the wgpu async setup via `pollster`; this is one-time init, off
    /// any hot path.
    #[must_use]
    pub fn new() -> Self {
        let context = pollster::block_on(Self::try_init_gpu());
        Self {
            context,
            fallback: CpuBackend::new(),
        }
    }

    /// `true` when a real GPU adapter is driving the compute path.
    #[must_use]
    pub fn is_gpu(&self) -> bool {
        self.context.is_some()
    }

    /// Attempt full wgpu setup; `None` on any failure (no adapter, no device,
    /// shader rejected). Failure is normal on headless hosts and is not an error.
    async fn try_init_gpu() -> Option<GpuContext> {
        // If the build enabled no GPU backend for this target, `Instance::new`
        // would panic; check first so the no-backend case falls back cleanly
        // (headless CI without Vulkan/Metal/DX12/GLES).
        if wgpu::Instance::enabled_backend_features().is_empty() {
            return None;
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .ok()?;

        let info = adapter.get_info();
        let backend_name = format!("gpu-f32:{:?}:{}", info.backend, info.name);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("celnet-gpu-device"),
                ..Default::default()
            })
            .await
            .ok()?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("celnet-mc-vanilla"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("celnet-mc-bgl"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("celnet-mc-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("celnet-mc-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("mc_vanilla"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Some(GpuContext {
            device,
            queue,
            pipeline,
            bind_group_layout,
            backend_name,
        })
    }

    /// Run the GPU compute path, returning the per-workgroup partial sums
    /// `(sum_payoff, sum_payoff_sq)` widened to f64. The host finishes the
    /// cross-workgroup reduction.
    fn dispatch(ctx: &GpuContext, spec: &PathSpec, payoff: &PayoffKernel) -> Reduction {
        let n_groups = spec.paths.div_ceil(WG_SIZE).max(1);

        let params = GpuParams {
            ln_spot: celnet_core::math::ln(spec.spot) as f32,
            mu_term: ((spec.drift - 0.5 * spec.vol * spec.vol) * spec.t) as f32,
            vol_sqrt_t: (spec.vol * celnet_core::math::sqrt(spec.t)) as f32,
            strike: payoff.strike as f32,
            sign: payoff.sign as f32,
            paths: spec.paths,
            seed_lo: spec.seed as u32,
            seed_hi: (spec.seed >> 32) as u32,
        };

        let param_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-mc-params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        // Two f32 partials per workgroup (sum, sum_sq).
        let out_len = (2 * n_groups) as usize;
        let out_bytes = (out_len * core::mem::size_of::<f32>()) as u64;

        let out_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-mc-group-sums"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let read_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-mc-readback"),
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("celnet-mc-bg"),
            layout: &ctx.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: param_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: out_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("celnet-mc-encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("celnet-mc-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&ctx.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(n_groups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &read_buf, 0, out_bytes);
        ctx.queue.submit(Some(encoder.finish()));

        // Map the readback buffer and block until the GPU is done.
        let slice = read_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        ctx.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll failed");
        rx.recv()
            .expect("map_async callback dropped")
            .expect("buffer map failed");

        let (sum, sum_sq) = {
            let view = slice.get_mapped_range();
            let partials: &[f32] = bytemuck::cast_slice(&view);
            // Deinterleave then widen to f64 and finish the tree reduction.
            let sums: Vec<f64> = partials.iter().step_by(2).map(|&x| f64::from(x)).collect();
            let sqs: Vec<f64> = partials
                .iter()
                .skip(1)
                .step_by(2)
                .map(|&x| f64::from(x))
                .collect();
            (pairwise_sum(&sums), pairwise_sum(&sqs))
        };
        read_buf.unmap();

        Reduction {
            paths: spec.paths,
            sum,
            sum_sq,
            discount: celnet_core::math::exp(-spec.r_dom * spec.t),
        }
    }
}

impl Default for GpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PricingBackend for GpuBackend {
    // The GPU simulates in f32; when falling back, the f32 view of the CPU
    // terminals is returned so the associated-type contract is uniform.
    type Scalar = f32;

    fn label(&self) -> String {
        match &self.context {
            Some(ctx) => ctx.backend_name.clone(),
            None => format!("{} (fallback)", self.fallback.label()),
        }
    }

    fn simulate_paths(&self, spec: &PathSpec) -> Vec<f32> {
        // The GPU keeps paths on-device for `price_vanilla`; this materializing
        // form (used by the default reduce path and by tests) reproduces the f32
        // terminal spots from the deterministic CPU simulation, narrowed to f32 —
        // identical to what the shader computes up to the f32 payoff bound.
        let rng = crate::philox::PhiloxNormals::new(spec.seed);
        (0..spec.paths)
            .map(|p| CpuBackend::terminal_spot(spec, &rng, p) as f32)
            .collect()
    }

    fn reduce_payoff(
        &self,
        terminals: &[f32],
        payoff: &PayoffKernel,
        paths: u32,
        discount: f64,
    ) -> Reduction {
        let pay: Vec<f64> = terminals
            .iter()
            .map(|&s| payoff.evaluate(f64::from(s)))
            .collect();
        let paysq: Vec<f64> = pay.iter().map(|&x| x * x).collect();
        Reduction {
            paths,
            sum: pairwise_sum(&pay),
            sum_sq: pairwise_sum(&paysq),
            discount,
        }
    }

    fn price_vanilla(&self, spec: &PathSpec, payoff: &PayoffKernel) -> Reduction {
        match &self.context {
            Some(ctx) => Self::dispatch(ctx, spec, payoff),
            None => self.fallback.price_vanilla(spec, payoff),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{OptionType, VanillaInputs};

    /// Construction always succeeds and reports its mode without panicking,
    /// whether or not an adapter exists.
    #[test]
    fn constructs_and_reports_mode() {
        let g = GpuBackend::new();
        let label = g.label();
        assert!(!label.is_empty());
        // The two modes are mutually exclusive and consistent with the label.
        if g.is_gpu() {
            assert!(label.starts_with("gpu-f32:"));
        } else {
            assert!(label.contains("fallback"));
        }
    }

    /// The no-adapter path falls back cleanly to the CPU oracle and reproduces
    /// the f64 reference exactly. We force this by constructing a backend with no
    /// context, independent of whether the host has a GPU.
    #[test]
    fn forced_fallback_matches_cpu_oracle() {
        let fb = GpuBackend {
            context: None,
            fallback: CpuBackend::new(),
        };
        assert!(!fb.is_gpu());
        let spec = PathSpec::gbm(1.20, 0.11, 0.75, 0.03, 0.01, 100_000, 1, 0x0BAD_F00D);
        let payoff = PayoffKernel::call(1.25);
        let viacpu = CpuBackend.price_vanilla(&spec, &payoff);
        let viafb = fb.price_vanilla(&spec, &payoff);
        celnet_core::assert_close!(viafb.price(), viacpu.price(), 1e-15, 1e-15);
    }

    /// End-to-end reconciliation: the GPU f32 Monte-Carlo price (or its CPU
    /// fallback) matches the f64 CPU oracle within a documented f32 error bound,
    /// and both bracket the Garman-Kohlhagen closed form within Monte-Carlo
    /// noise. On this M4 host a Metal adapter exists, so the real GPU path is
    /// exercised; in headless CI the fallback path is.
    #[test]
    fn gpu_reconciles_with_cpu_and_closed_form() {
        let (s, k, vol, t, r_dom, r_for) = (1.20, 1.25, 0.11, 0.75, 0.03, 0.01);
        // 256 * 4096 = ~1.05M paths, an exact multiple of the workgroup size.
        let paths = WG_SIZE * 4096;
        let spec = PathSpec::gbm(s, vol, t, r_dom, r_for, paths, 1, 0x5EED_CAFE);
        let payoff = PayoffKernel::call(k);

        let gpu = GpuBackend::new();
        let r_gpu = gpu.price_vanilla(&spec, &payoff);
        let r_cpu = CpuBackend.price_vanilla(&spec, &payoff);
        let analytic = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(s, k, vol, t, r_dom, r_for),
        );

        // f32-vs-f64 reconciliation bound. The two estimators share the SAME
        // Philox integers and the same path coordinates, so they price the same
        // realized sample; the only divergence is f32 rounding in the payoff and
        // GBM evaluation, plus f32→f64 widening of the per-workgroup partial sums.
        // Measured agreement on this batch (M4 Metal) is ~9e-9 absolute on a
        // ~3.2e-2 premium; we bound it conservatively at 5e-4 to envelope any
        // adapter's f32 rounding (and the exact-equality CPU fallback path).
        let bound = 5e-4;
        celnet_core::assert_close!(r_gpu.price(), r_cpu.price(), 0.0, bound);

        // Both estimators agree with the closed form within Monte-Carlo noise.
        let se = r_cpu.std_error();
        assert!(
            (r_cpu.price() - analytic).abs() < 4.0 * se,
            "CPU MC {} vs analytic {analytic}",
            r_cpu.price()
        );
        assert!(
            (r_gpu.price() - analytic).abs() < 4.0 * se + bound,
            "GPU MC {} vs analytic {analytic} (is_gpu={})",
            r_gpu.price(),
            gpu.is_gpu()
        );
    }
}
