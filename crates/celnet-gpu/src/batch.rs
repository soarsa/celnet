//! Batch **closed-form** vanilla pricing — one GPU dispatch prices a large batch
//! of independent Garman-Kohlhagen vanilla FX options (varying spot, strike,
//! vol, expiry, rates, call/put) by the *analytic* closed form in f32, with the
//! exact f64 CPU oracle the crate already carries for headless/CI hosts.
//!
//! # What this is (GPU-AT-SCALE Workload A / G2)
//!
//! This is the closed-form batch lever `docs/GPU-AT-SCALE-PLAN.md` §2.1 names as
//! the right GPU-at-scale Workload A: a full-portfolio / full-surface revaluation
//! prices `O(10⁴–10⁶)` (pair, tenor, strike) points. Closed-form GK is *cheap
//! per point* but *embarrassingly wide*, so the win is **batching the whole grid
//! into one GPU dispatch** (one thread per instrument) rather than a per-point
//! CPU sweep or a per-point Monte-Carlo. Unlike the MC kernels
//! ([`crate::gpu`], [`crate::scenario`]) this draws **no random numbers** and has
//! **no Monte-Carlo noise** — it evaluates the exact analytic price — so the only
//! divergence from the f64 oracle is f32 round-off, reconciled node-by-node below.
//!
//! # Numeric policy and the golden chain
//!
//! WGSL/Metal have no f64, so every instrument is priced in f32. WGSL also has no
//! native `erf`/`erfc`, so the normal CDF is the Abramowitz & Stegun 7.1.26
//! rational-times-Gaussian approximation (max absolute error `1.5e-7` on `erf`,
//! at the f32 round-off floor). The reconciliation is split into two independent,
//! separately-derived bounds so neither error source hides the other:
//!
//! 1. **f32 round-off** — [`Self::price_batch`] (GPU f32, or its CPU fallback)
//!    vs [`cpu_batch_with_as_erf`] (the f64 evaluation of the *same* A&S-erf GK
//!    algorithm). This isolates pure f32 arithmetic round-off and is bounded by
//!    [`derived_batch_bound`], derived from `f32::EPSILON` and the GK condition
//!    numbers — never a fitted constant.
//! 2. **A&S algorithmic error** — [`cpu_batch_with_as_erf`] vs
//!    [`celnet_vanilla::price`] (the production f64 path, which uses `libm::erfc`
//!    and is gated bit-for-bit against the **QuantLib golden** in `celnet-golden`
//!    to ~1e-10). This is bounded by [`as_erf_price_bound`], the A&S `1.5e-7`
//!    `erf` error propagated through the price (`vega/σ`-scaled).
//!
//! The combined GPU-f32 vs golden-validated closed form is then within the sum of
//! the two — the three-way bracket `GPU-f32 ~ f64-oracle ~ golden-closed-form`
//! the plan (§3.3) requires, here for the *exact* closed form (no MC-noise term).

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::backend::PricingBackend;
use crate::gpu::{GpuBackend, GpuContextRef};

/// Workgroup size; must match `@workgroup_size` in `batch.wgsl`.
const WG_SIZE: u32 = 256;

/// Bounded readback deadline for a batch dispatch (see the MC kernels for the
/// always-terminates rationale): a wedged device fails loudly rather than hanging.
const GPU_DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Upper bound on batch size, guarding the readback allocation against a runaway
/// dispatch. 4M instruments is the top of the Workload-A range in the plan.
const MAX_BATCH: u64 = 4 * 1024 * 1024;

/// `1/√2` in f32 (the kernel's normalization constant; mirrored here so the CPU
/// oracle's A&S CDF is bit-aligned with the shader's).
const INV_SQRT_2_F32: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// One vanilla instrument to price by the closed form: a Garman-Kohlhagen input
/// set plus a call/put sign. Plain data; the eight fields are irreducible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchInstrument {
    /// Spot `S_0`.
    pub spot: f64,
    /// Strike `K`.
    pub strike: f64,
    /// Volatility `σ` (annualized, absolute).
    pub vol: f64,
    /// Time to expiry `T` in years.
    pub t: f64,
    /// Domestic continuously-compounded rate `r_dom`.
    pub r_dom: f64,
    /// Foreign continuously-compounded rate `r_for`.
    pub r_for: f64,
    /// `+1.0` call, `−1.0` put.
    pub sign: f64,
}

impl BatchInstrument {
    /// A call instrument.
    #[must_use]
    pub const fn call(spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            r_dom,
            r_for,
            sign: 1.0,
        }
    }

    /// A put instrument.
    #[must_use]
    pub const fn put(spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            r_dom,
            r_for,
            sign: -1.0,
        }
    }
}

/// GPU-side instrument layout (`#[repr(C)]` + `Pod`, f32, std430 array element).
/// Seven meaningful f32 then one pad f32 → 32 bytes, a 16-byte multiple so the
/// `array<Instrument>` stride is std430-legal on every backend.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GpuInstrument {
    spot: f32,
    strike: f32,
    vol: f32,
    t: f32,
    r_dom: f32,
    r_for: f32,
    sign: f32,
    _pad: f32,
}

impl From<&BatchInstrument> for GpuInstrument {
    fn from(b: &BatchInstrument) -> Self {
        Self {
            spot: b.spot as f32,
            strike: b.strike as f32,
            vol: b.vol as f32,
            t: b.t as f32,
            r_dom: b.r_dom as f32,
            r_for: b.r_for as f32,
            sign: b.sign as f32,
            _pad: 0.0,
        }
    }
}

/// Uniform metadata block (instrument count + std140 tail pad to 16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct BatchMeta {
    n: u32,
    _p: [u32; 3],
}

/// Prices a batch of independent vanillas by the closed form, on the GPU when an
/// adapter is present and on the exact f64 CPU oracle otherwise.
///
/// Holds a [`GpuBackend`] (adapter probe + CPU fallback) and lazily builds the
/// batch compute pipeline once. Construction is off any hot path.
pub struct BatchPricer {
    backend: GpuBackend,
    pipeline: Option<BatchPipeline>,
}

/// The batch compute pipeline, built once over the [`GpuBackend`]'s device.
struct BatchPipeline {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl core::fmt::Debug for BatchPricer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BatchPricer")
            .field("is_gpu", &self.is_gpu())
            .field("label", &self.backend.label())
            .finish()
    }
}

impl BatchPricer {
    /// Build a batch pricer, probing for a GPU adapter (falling back to the exact
    /// f64 CPU oracle when none is present, so it runs headless in CI).
    #[must_use]
    pub fn new() -> Self {
        let backend = GpuBackend::new();
        let pipeline = backend
            .gpu_context()
            .map(|ctx| BatchPipeline::build(ctx.device));
        Self { backend, pipeline }
    }

    /// `true` when a real GPU adapter is driving the batch compute path.
    #[must_use]
    pub fn is_gpu(&self) -> bool {
        self.backend.is_gpu() && self.pipeline.is_some()
    }

    /// Human-readable backend identity (mirrors [`GpuBackend::label`]).
    #[must_use]
    pub fn label(&self) -> String {
        self.backend.label()
    }

    /// Price a whole batch of vanillas in one dispatch, returning one price per
    /// instrument (row-aligned with `insts`). The GPU path is used when an
    /// adapter is present; otherwise the exact f64 CPU oracle prices the batch
    /// (the f32 view of [`cpu_batch_with_as_erf`], bit-identical to the fallback).
    ///
    /// # Panics
    ///
    /// Panics if `insts` is empty or exceeds [`MAX_BATCH`] — a caller-side
    /// configuration error surfaced loudly rather than silently truncated.
    #[must_use]
    pub fn price_batch(&self, insts: &[BatchInstrument]) -> Vec<f64> {
        assert!(
            !insts.is_empty() && insts.len() as u64 <= MAX_BATCH,
            "invalid batch size {} (must be 1..={MAX_BATCH})",
            insts.len()
        );

        match (self.backend.gpu_context(), self.pipeline.as_ref()) {
            (Some(ctx), Some(pipe)) => Self::dispatch(&ctx, pipe, insts),
            // No adapter (or pipeline build failed): the exact f64 CPU oracle,
            // narrowed to f32 per element so the contract (a batch of f32-grade
            // prices) is identical to the shader path.
            _ => cpu_batch_with_as_erf(insts)
                .into_iter()
                .map(|p| f64::from(p as f32))
                .collect(),
        }
    }

    /// Run the batch GPU compute path: one 1-D dispatch over the instruments,
    /// read back one f32 price per instrument, widened to f64 for the caller.
    fn dispatch(
        ctx: &GpuContextRef<'_>,
        pipe: &BatchPipeline,
        insts: &[BatchInstrument],
    ) -> Vec<f64> {
        let n = insts.len() as u32;
        let gpu_insts: Vec<GpuInstrument> = insts.iter().map(GpuInstrument::from).collect();

        let meta = BatchMeta { n, _p: [0; 3] };
        let meta_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-batch-meta"),
                contents: bytemuck::bytes_of(&meta),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let inst_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("celnet-batch-insts"),
                contents: bytemuck::cast_slice(&gpu_insts),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let out_bytes = (n as u64) * core::mem::size_of::<f32>() as u64;
        let out_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-batch-prices"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celnet-batch-readback"),
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("celnet-batch-bg"),
            layout: &pipe.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: meta_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: inst_buf.as_entire_binding(),
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
                label: Some("celnet-batch-encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("celnet-batch-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipe.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(n.div_ceil(WG_SIZE).max(1), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &read_buf, 0, out_bytes);
        ctx.queue.submit(Some(encoder.finish()));

        Self::map_readback(ctx, &read_buf, n as usize)
    }

    /// Map the readback buffer under a bounded deadline and return the f32 prices
    /// widened to f64 (see the MC kernels for the always-terminates rationale).
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
                        "GPU batch readback timed out after {GPU_DISPATCH_TIMEOUT:?} \
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
}

impl Default for BatchPricer {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchPipeline {
    /// Compile the batch shader and build its pipeline + bind-group layout.
    fn build(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("celnet-batch-vanilla"),
            source: wgpu::ShaderSource::Wgsl(include_str!("batch.wgsl").into()),
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
            label: Some("celnet-batch-bgl"),
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
                storage(1, true),  // instruments (read-only)
                storage(2, false), // prices (read-write)
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("celnet-batch-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("celnet-batch-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("batch_vanilla"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }
}

/// f32 error function via Abramowitz & Stegun 7.1.26, evaluated in **f64** but
/// mirroring the shader's f32 coefficient algebra exactly.
///
/// This is the CPU oracle for the GPU's `erf_as`: the shader computes the very
/// same rational-times-Gaussian in f32, so comparing the GPU price against an
/// oracle built on this `erf` (rather than `libm::erfc`) isolates pure f32
/// round-off from the A&S *algorithmic* error. The A&S error is bounded and
/// checked separately against `libm::erfc` (the production / golden Φ).
///
/// The coefficients are f32 literals exactly as in `batch.wgsl`; we evaluate in
/// f64 so this function is the *algorithm-matched, round-off-free* oracle.
#[must_use]
fn erf_as_oracle(x: f64) -> f64 {
    // The coefficients are written in their shortest decimal forms that round to
    // exactly the same f32 bit patterns the WGSL kernel parses (the full A&S
    // 7.1.26 constants 0.3275911, 0.254829592, … carry more digits than f32 can
    // hold; clippy::excessive_precision flags that, and the shortened forms below
    // are bit-identical f32 values). We then widen to f64 so this oracle runs the
    // GPU's *algorithm* with no f32 round-off — exactly what isolates the round-off
    // term in `derived_batch_bound`.
    let s = x.signum();
    let z = x.abs();
    let p = f64::from(0.327_591_1_f32);
    let a1 = f64::from(0.254_829_6_f32);
    let a2 = f64::from(-0.284_496_72_f32);
    let a3 = f64::from(1.421_413_8_f32);
    let a4 = f64::from(-1.453_152_1_f32);
    let a5 = f64::from(1.061_405_4_f32);
    let tt = 1.0 / (1.0 + p * z);
    let poly = ((((a5 * tt + a4) * tt + a3) * tt + a2) * tt + a1) * tt;
    let y = 1.0 - poly * celnet_core::math::exp(-z * z);
    s * y
}

/// Standard-normal CDF via the A&S `erf` oracle (the GPU's algorithm, in f64).
#[must_use]
fn norm_cdf_as_oracle(x: f64) -> f64 {
    0.5 * (1.0 + erf_as_oracle(x * f64::from(INV_SQRT_2_F32)))
}

/// Price one Garman-Kohlhagen vanilla in f64 using the A&S `erf` Φ — the exact
/// f64 evaluation of the algorithm the GPU runs in f32. This is the
/// reconciliation oracle for the f32 round-off bound.
#[must_use]
fn gk_price_as_oracle(b: &BatchInstrument) -> f64 {
    let sqt = celnet_core::math::sqrt(b.t);
    let vsqt = b.vol * sqt;
    let d1 = (celnet_core::math::ln(b.spot / b.strike)
        + (b.r_dom - b.r_for + 0.5 * b.vol * b.vol) * b.t)
        / vsqt;
    let d2 = d1 - vsqt;
    let df_dom = celnet_core::math::exp(-b.r_dom * b.t);
    let df_for = celnet_core::math::exp(-b.r_for * b.t);
    let s_disc = b.spot * df_for;
    let k_disc = b.strike * df_dom;
    let px = if b.sign > 0.0 {
        s_disc * norm_cdf_as_oracle(d1) - k_disc * norm_cdf_as_oracle(d2)
    } else {
        k_disc * norm_cdf_as_oracle(-d2) - s_disc * norm_cdf_as_oracle(-d1)
    };
    px.max(0.0)
}

/// f64 batch oracle using the GPU's A&S `erf` Φ — the node-by-node ground truth
/// the f32 GPU result is reconciled against under [`derived_batch_bound`].
#[must_use]
pub fn cpu_batch_with_as_erf(insts: &[BatchInstrument]) -> Vec<f64> {
    insts.iter().map(gk_price_as_oracle).collect()
}

/// **Derived** f32-vs-f64 reconciliation bound for one closed-form vanilla price.
///
/// The GPU and the [`cpu_batch_with_as_erf`] oracle evaluate the *same*
/// Garman-Kohlhagen algorithm with the *same* A&S `erf`; the GPU does it in f32,
/// the oracle in f64. So the divergence is pure f32 round-off, which we bound
/// from `f32::EPSILON` and the formula's condition numbers — never a fitted
/// constant.
///
/// # Per-price error model
///
/// Let `eps = f32::EPSILON ≈ 5.96e-8`. The price is
/// `V = φ(±1)·[S·e^{-r_f t}·Φ(±d1) − K·e^{-r_d t}·Φ(±d2)]`. The f32 evaluation
/// perturbs each sub-expression; we bound the absolute price error as the sum of
/// the dominant contributions, each a relative-error budget times the magnitude
/// of the term it scales:
///
/// * **Discount factors** `e^{-r·t}`: WGSL `exp` carries an implementation-defined
///   error the WebGPU spec bounds at `K_exp ≈ 3` ULP; `|Δ(S e^{-r_f t})| ≲
///   (K_exp+2)·eps·S·e^{-r_f t}` (the `+2` for the multiply and the `r·t` form).
/// * **`d1`/`d2`**: `d1 = [ln(S/K) + (b+½σ²)T]/(σ√T)`. The sensitive part is the
///   division by `σ√T`; with `ln`/`sqrt`/division each `≲ K_log·eps`
///   (`K_log ≈ 3` ULP for `log`, plus a few for the divide/adds) the absolute
///   error in `d1` is `|Δd1| ≲ K_d·eps·(|d1| + 1)` with `K_d ≈ 8` (the `+1`
///   covers the ATM `d1 ≈ 0` regime where the relative model degenerates).
/// * **`Φ(d)`**: `Φ' = φ(d) ≤ 1/√(2π) ≈ 0.399`, so a `d`-error propagates as
///   `|ΔΦ| ≲ φ(d)·|Δd| + K_erf·eps`, with `K_erf ≈ 4` ULP for the A&S
///   rational+`exp` evaluated in f32. Bounding `φ(d) ≤ 0.399`:
///   `|ΔΦ| ≲ 0.399·K_d·eps·(|d1|+1) + K_erf·eps`.
/// * **The S·Φ − K·Φ combination**: each product contributes its own
///   `≲ eps·(magnitude)` and the subtraction is exact-to-`eps`. Summing the two
///   `Φ`-error terms (scaled by `S·e^{-r_f t}` and `K·e^{-r_d t}`) and the
///   discount/product round-off gives the total.
///
/// Collecting terms, with `scale = max(S·e^{-r_f t}, K·e^{-r_d t})` the price
/// scale and `dmax = max(|d1|,|d2|)`:
///
/// ```text
/// |ΔV| ≲ scale · eps · [ (K_exp+4)                       // discount + product round-off
///                      + 2·(0.399·K_d·(dmax+1) + K_erf) ] // both Φ terms
///        + abs_floor
/// ```
///
/// with `abs_floor = 16·eps·scale` for the deep-OTM regime where the bracket
/// nearly cancels and the `max(·,0)` clamp engages on both sides. The constant is
/// therefore *derived* from `f32::EPSILON` and the GK condition numbers `dmax`,
/// `scale` — not a hand-tuned tolerance.
///
/// This bound covers **f32 round-off only**. The A&S `erf` *algorithmic* error
/// (vs `libm::erfc`) is a separate, larger-but-still-tiny term bounded by
/// [`as_erf_price_bound`]; the GPU-vs-golden three-way bracket uses the sum.
#[must_use]
pub fn derived_batch_bound(b: &BatchInstrument) -> f64 {
    let eps = f64::from(f32::EPSILON);
    let k_exp = 3.0;
    let k_d = 8.0;
    let k_erf = 4.0;
    let inv_sqrt_2pi = celnet_core::math::INV_SQRT_2PI; // max φ ≈ 0.399

    let sqt = celnet_core::math::sqrt(b.t);
    let vsqt = b.vol * sqt;
    let d1 = (celnet_core::math::ln(b.spot / b.strike)
        + (b.r_dom - b.r_for + 0.5 * b.vol * b.vol) * b.t)
        / vsqt;
    let d2 = d1 - vsqt;
    let dmax = d1.abs().max(d2.abs());

    let s_disc = b.spot * celnet_core::math::exp(-b.r_for * b.t);
    let k_disc = b.strike * celnet_core::math::exp(-b.r_dom * b.t);
    let scale = s_disc.max(k_disc);

    let phi_term = 2.0 * (inv_sqrt_2pi * k_d * (dmax + 1.0) + k_erf);
    let rel = (k_exp + 4.0) + phi_term;
    scale * eps * rel + 16.0 * eps * scale
}

/// **Derived** Abramowitz & Stegun 7.1.26 *algorithmic* price-error bound — the
/// gap between the A&S-`erf` Φ (what the GPU and [`cpu_batch_with_as_erf`] use)
/// and the production `libm::erfc` Φ (gated against the QuantLib golden).
///
/// A&S 7.1.26 has a stated maximum absolute error `ε_erf = 1.5e-7` on `erf(x)`
/// over all `x`. Through `Φ(x) = ½(1 + erf(x/√2))`, the absolute Φ error is
/// `½·ε_erf`. The price is `S·e^{-r_f t}·Φ(±d1) − K·e^{-r_d t}·Φ(±d2)`, so two Φ
/// terms each contribute their scale times `½·ε_erf`:
///
/// ```text
/// |ΔV_alg| ≲ (S·e^{-r_f t} + K·e^{-r_d t}) · ½ · ε_erf
/// ```
///
/// This is independent of f32: it is the algorithmic substitution error of using
/// A&S in place of `erfc`, and it dominates the f32 round-off term. The
/// GPU-f32-vs-golden bracket is [`derived_batch_bound`] + this.
#[must_use]
pub fn as_erf_price_bound(b: &BatchInstrument) -> f64 {
    const AS_ERF_MAX_ABS_ERR: f64 = 1.5e-7;
    let s_disc = b.spot * celnet_core::math::exp(-b.r_for * b.t);
    let k_disc = b.strike * celnet_core::math::exp(-b.r_dom * b.t);
    (s_disc + k_disc) * 0.5 * AS_ERF_MAX_ABS_ERR
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{OptionType, VanillaInputs};

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

    /// A deterministic batch spanning ITM/ATM/OTM × short/long expiry × varied
    /// vol/rates × call/put, sized ≥ 1k instruments. Built from a structured
    /// sweep so it covers the realistic FX parameter ranges the bound must hold
    /// over, with no randomness (fully reproducible).
    fn realistic_batch() -> Vec<BatchInstrument> {
        let spots = [0.85, 1.00, 1.20, 1.35];
        let moneyness = [0.80, 0.90, 0.95, 1.00, 1.05, 1.10, 1.25]; // K/S
        let vols = [0.06, 0.10, 0.18, 0.28];
        let expiries = [0.05, 0.25, 1.0, 2.5]; // ~2w, 3m, 1y, 2.5y
        let rates = [(0.03, 0.01), (0.005, 0.04)]; // (r_dom, r_for)
        let mut out = Vec::new();
        for &s in &spots {
            for &m in &moneyness {
                let k = s * m;
                for &vol in &vols {
                    for &t in &expiries {
                        for &(rd, rf) in &rates {
                            out.push(BatchInstrument::call(s, k, vol, t, rd, rf));
                            out.push(BatchInstrument::put(s, k, vol, t, rd, rf));
                        }
                    }
                }
            }
        }
        out
    }

    /// Gate (a) + (b): three-way batch closed-form ≈ CPU f64 (A&S-erf oracle) ≈
    /// golden-validated closed form, node-by-node over a ≥ 1k-instrument batch.
    ///
    /// * GPU-f32 vs the f64 A&S-erf oracle is within [`derived_batch_bound`]
    ///   (pure f32 round-off);
    /// * the A&S-erf oracle vs `celnet_vanilla::price` (libm::erfc Φ, gated
    ///   bit-for-bit against the QuantLib golden in `celnet-golden`) is within
    ///   [`as_erf_price_bound`] (A&S algorithmic error);
    /// * therefore GPU-f32 vs the golden-validated closed form is within the sum.
    ///
    /// On a host with an adapter the shader path is genuinely exercised
    /// (asserts `is_gpu`); on a headless host the batch is the exact CPU oracle.
    #[test]
    fn batch_reconciles_three_way() {
        let batch = realistic_batch();
        assert!(batch.len() >= 1000, "batch too small: {}", batch.len());

        let pricer = BatchPricer::new();
        if adapter_is_available() {
            assert!(
                pricer.is_gpu(),
                "a GPU adapter is present but the batch pricer fell back to CPU: {}",
                pricer.label()
            );
        }
        let gpu = pricer.price_batch(&batch);
        let oracle = cpu_batch_with_as_erf(&batch);
        assert_eq!(gpu.len(), batch.len());
        assert_eq!(oracle.len(), batch.len());

        let mut max_rel_f32 = 0.0_f64;
        let mut max_rel_alg = 0.0_f64;
        for (i, inst) in batch.iter().enumerate() {
            // (b) f32 round-off: GPU vs the algorithm-matched f64 oracle.
            let f32_bound = derived_batch_bound(inst);
            let f32_err = (gpu[i] - oracle[i]).abs();
            assert!(
                f32_err <= f32_bound,
                "f32 round-off bound exceeded at #{i} {inst:?}: \
                 err={f32_err:e} > bound={f32_bound:e} (gpu={}, oracle={})",
                gpu[i],
                oracle[i]
            );

            // A&S algorithmic error: the f64 A&S oracle vs the golden-validated
            // production closed form.
            let golden = celnet_vanilla::price(
                if inst.sign > 0.0 {
                    OptionType::Call
                } else {
                    OptionType::Put
                },
                &VanillaInputs::new(
                    inst.spot,
                    inst.strike,
                    inst.vol,
                    inst.t,
                    inst.r_dom,
                    inst.r_for,
                ),
            );
            let alg_bound = as_erf_price_bound(inst);
            let alg_err = (oracle[i] - golden).abs();
            assert!(
                alg_err <= alg_bound,
                "A&S algorithmic bound exceeded at #{i} {inst:?}: \
                 err={alg_err:e} > bound={alg_bound:e} (oracle={}, golden={golden})",
                oracle[i]
            );

            // (a) three-way: GPU vs golden within the SUM of the two derived bounds.
            let three_way = f32_bound + alg_bound;
            let gpu_golden = (gpu[i] - golden).abs();
            assert!(
                gpu_golden <= three_way,
                "three-way bracket exceeded at #{i} {inst:?}: \
                 err={gpu_golden:e} > bound={three_way:e} (gpu={}, golden={golden})",
                gpu[i]
            );

            // Track normalized (relative) errors for the reported headline.
            let scale = (inst.spot.max(inst.strike)).max(1e-6);
            max_rel_f32 = max_rel_f32.max(f32_err / scale);
            max_rel_alg = max_rel_alg.max(alg_err / scale);
        }
        // Visible in `--nocapture`; both must sit well under the f32 ~few×1e-6
        // relative-error expectation for a closed-form GK in f32.
        println!(
            "batch_reconciles_three_way: n={} is_gpu={} \
             max f32-roundoff rel={max_rel_f32:e} max A&S-alg rel={max_rel_alg:e}",
            batch.len(),
            pricer.is_gpu()
        );
        assert!(
            max_rel_f32 < 5e-6,
            "max f32 relative round-off {max_rel_f32:e} above the ~few×1e-6 f32 expectation"
        );
    }

    /// Gate (d) headless: a forced no-adapter pricer falls back to the exact CPU
    /// oracle and still reconciles to the golden-validated closed form within the
    /// A&S algorithmic bound (the f32 round-off term collapses, since the fallback
    /// computes in f64 then narrows once per element).
    #[test]
    fn forced_fallback_reconciles_to_golden() {
        // Construct a pricer guaranteed to have no pipeline (force the CPU path)
        // by dropping the pipeline even if an adapter exists.
        let pricer = BatchPricer {
            backend: GpuBackend::new(),
            pipeline: None,
        };
        assert!(!pricer.is_gpu());

        let batch = realistic_batch();
        let prices = pricer.price_batch(&batch);
        for (i, inst) in batch.iter().enumerate() {
            let golden = celnet_vanilla::price(
                if inst.sign > 0.0 {
                    OptionType::Call
                } else {
                    OptionType::Put
                },
                &VanillaInputs::new(
                    inst.spot,
                    inst.strike,
                    inst.vol,
                    inst.t,
                    inst.r_dom,
                    inst.r_for,
                ),
            );
            // Fallback is the f64 A&S oracle narrowed to f32 once: bound is the
            // A&S algorithmic error plus a single f32 narrowing ULP on the result.
            let bound = as_erf_price_bound(inst) + f64::from(f32::EPSILON) * golden.abs() + 1e-9;
            assert!(
                (prices[i] - golden).abs() <= bound,
                "fallback vs golden exceeded at #{i} {inst:?}: \
                 got {} golden {golden} bound {bound:e}",
                prices[i]
            );
        }
    }

    /// Determinism: two batch dispatches of the same instruments produce
    /// **bit-identical** prices (the closed form draws no RNG, so the result is a
    /// pure deterministic function of the inputs on any backend).
    #[test]
    fn batch_is_bit_reproducible() {
        let batch = realistic_batch();
        let pricer = BatchPricer::new();
        let a = pricer.price_batch(&batch);
        let b = pricer.price_batch(&batch);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.to_bits(), y.to_bits(), "non-deterministic batch price");
        }
    }

    /// Ragged batch size (not a multiple of WG_SIZE) still reconciles: exercises
    /// the shader's `idx >= meta.n` tail guard and the host
    /// `div_ceil(...).max(1)` group count on a real device.
    #[test]
    fn batch_honors_ragged_size() {
        let mut batch = realistic_batch();
        batch.truncate(1003); // not a multiple of WG_SIZE (256)
        assert_ne!(batch.len() % WG_SIZE as usize, 0);

        let pricer = BatchPricer::new();
        if adapter_is_available() {
            assert!(
                pricer.is_gpu(),
                "GPU present but fell back: {}",
                pricer.label()
            );
        }
        let gpu = pricer.price_batch(&batch);
        let oracle = cpu_batch_with_as_erf(&batch);
        assert_eq!(gpu.len(), batch.len());
        for (i, inst) in batch.iter().enumerate() {
            let bound = derived_batch_bound(inst);
            assert!(
                (gpu[i] - oracle[i]).abs() <= bound,
                "ragged-tail reconcile failed at #{i}"
            );
        }
    }

    /// The A&S-erf f64 oracle itself matches the production libm::erfc Φ closed
    /// form within the derived A&S algorithmic bound — proving the oracle is a
    /// faithful (golden-bracketed) reference, not a second source of error that
    /// could mask a GPU bug.
    #[test]
    fn as_erf_oracle_brackets_golden() {
        let batch = realistic_batch();
        let oracle = cpu_batch_with_as_erf(&batch);
        let mut max_abs = 0.0_f64;
        for (i, inst) in batch.iter().enumerate() {
            let golden = celnet_vanilla::price(
                if inst.sign > 0.0 {
                    OptionType::Call
                } else {
                    OptionType::Put
                },
                &VanillaInputs::new(
                    inst.spot,
                    inst.strike,
                    inst.vol,
                    inst.t,
                    inst.r_dom,
                    inst.r_for,
                ),
            );
            let err = (oracle[i] - golden).abs();
            assert!(
                err <= as_erf_price_bound(inst),
                "A&S oracle off golden at #{i}"
            );
            max_abs = max_abs.max(err);
        }
        assert!(max_abs > 0.0, "oracle suspiciously bit-identical to libm Φ");
    }

    /// An empty batch is rejected loudly.
    #[test]
    #[should_panic(expected = "invalid batch size")]
    fn empty_batch_panics() {
        let _ = BatchPricer::new().price_batch(&[]);
    }
}
