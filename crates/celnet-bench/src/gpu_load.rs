//! GPU batch **throughput + dispatch-latency** load harness (Wave 5, G1).
//!
//! This is the GPU sibling of [`crate::core_load`] / [`crate::wire`]: it drives
//! the **existing** [`celnet_gpu::GpuBackend`] Monte-Carlo pricing path under a
//! bounded batch-size sweep, times each dispatch into a coordinated-omission-aware
//! [`hdrhistogram::Histogram`], and reports per-batch **dispatch latency**
//! (p50/p99/p99.9) and **instrument throughput** (priced paths/second). It
//! serializes a [`GpuBatchReport`] which is committed as the relative-regression
//! baseline and re-measured by the `gpu_gate` binary.
//!
//! # THE GPU HONEST BOUNDARY (reproduced verbatim; never violated)
//!
//! M4 Metal lacks f64, so the GPU path is f32; in-repo we prove CORRECTNESS
//! (f32↔f64 reconcile, three-way GPU-MC ~ CPU-MC ~ golden — those proofs live as
//! `#[test]`s in `celnet-gpu`, not here) and RATIOS (a measured GPU/CPU speedup on
//! THIS host) — a RATIO and a relative-regression signal, NOT an absolute
//! throughput. The NVIDIA absolute throughput headline, the ≤ 50 ms exotic, and
//! the Workload-A/B absolute numbers are DEFERRED to the CUDA deploy-gate CI and
//! are NEVER claimed from this repo. Everything runs HEADLESS (the [`GpuBackend`]
//! falls back to the [`celnet_gpu::CpuBackend`] when no adapter is present) so CI
//! (Lavapipe / none) is a *completes-within-ceiling* check, not a speedup
//! assertion.
//!
//! Concretely, this harness:
//!
//! * measures the [`GpuBackend`]'s per-dispatch latency + path throughput across a
//!   batch-size sweep, **and** the [`celnet_gpu::CpuBackend`] oracle on the same
//!   sweep, so it can report a **measured GPU/CPU throughput RATIO on this host**;
//! * records whether a real adapter drove the run ([`GpuBatchReport::is_gpu`]) and
//!   the backend label, so a headless/fallback run is **labelled as the CPU
//!   fallback** and its "throughput" is honestly the CPU oracle's, not a GPU claim;
//! * is the basis for the slowdown-only [`compare_to_baseline`] gate — a RELATIVE
//!   ratio gate against the committed baseline (fail on a structural regression,
//!   e.g. throughput collapsing below `baseline / (1 + tol)` or dispatch p99
//!   inflating beyond `baseline * (1 + tol)`), **never** an absolute-throughput
//!   assertion.
//!
//! # Boundedness
//!
//! Every batch is a fixed path count and a fixed dispatch count, with no blocking
//! I/O on the measured path beyond the backend's own bounded readback deadline
//! (the [`GpuBackend`] caps its device poll, so a wedged device fails loudly
//! rather than hanging). The whole sweep is therefore bounded by construction;
//! callers additionally run it under a shell timeout per repo hygiene.

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

use celnet_gpu::{CpuBackend, GpuBackend, PathSpec, PayoffKernel, PricingBackend};

/// The committed batch-size sweep: paths priced per dispatch.
///
/// Spans a small dispatch (where per-dispatch fixed overhead dominates) up to a
/// large batch (where the device/CPU is saturated and steady-state throughput is
/// meaningful). Powers of two keep the workgroup tiling clean. Kept modest at the
/// top end so a headless CPU-fallback run (Lavapipe / no adapter) still completes
/// quickly — the *throughput* number floats with the device and is only ever
/// compared **relatively**.
pub const SWEEP_PATHS: [u32; 5] = [4_096, 16_384, 65_536, 262_144, 1_048_576];

/// The representative single-asset GBM vanilla the sweep prices.
///
/// An at-the-money EUR/USD-style call (spot `1.10`, vol `9.5%`, 6-month expiry,
/// modest positive carry) — the same regime [`crate::representative_inputs`]
/// uses for the in-core gate, so the GPU and in-core benches price a coherent
/// workload. The path count is overridden per sweep step.
fn representative_spec(paths: u32) -> PathSpec {
    // spot, vol, t, r_dom, r_for, paths, steps, seed.
    PathSpec::gbm(1.10, 0.095, 0.5, 0.025, 0.015, paths, 1, 0x_C0FF_EE51)
}

/// The representative payoff: an at-the-money call struck on spot.
fn representative_payoff() -> PayoffKernel {
    PayoffKernel::call(1.10)
}

/// The measured dispatch-latency distribution for one batch size, nanoseconds.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DispatchLatency {
    /// Minimum observed dispatch, nanoseconds (warm floor).
    pub min_ns: u64,
    /// 50th percentile dispatch, nanoseconds.
    pub p50_ns: u64,
    /// 99th percentile dispatch, nanoseconds.
    pub p99_ns: u64,
    /// 99.9th percentile dispatch, nanoseconds.
    pub p999_ns: u64,
    /// 99.99th percentile dispatch, nanoseconds.
    pub p9999_ns: u64,
    /// Maximum observed dispatch, nanoseconds.
    pub max_ns: u64,
}

/// One batch-size point of the sweep: the measured GPU-path numbers plus the CPU
/// oracle throughput on the identical batch, so a host-local ratio can be derived.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPoint {
    /// Monte-Carlo paths priced per dispatch (the "instrument batch" size).
    pub paths: u32,
    /// GPU-path dispatch-latency distribution (per-dispatch wall-clock).
    pub latency: DispatchLatency,
    /// GPU-path instrument throughput: priced paths/second over the timed phase.
    /// On a headless/fallback run this is the **CPU oracle's** throughput, honestly
    /// (see [`GpuBatchReport::is_gpu`]).
    pub throughput_paths_per_s: f64,
    /// The [`celnet_gpu::CpuBackend`] f64-oracle throughput on the identical batch,
    /// paths/second — the denominator of the host-local GPU/CPU ratio.
    pub cpu_throughput_paths_per_s: f64,
    /// Measured **host-local** GPU/CPU throughput ratio (`gpu / cpu`). A RATIO on
    /// THIS host, NOT an absolute throughput. When the GPU path is the CPU
    /// fallback this is ~1.0 by construction (same backend) — labelled as such.
    pub gpu_over_cpu_ratio: f64,
}

/// The full result of a GPU batch load run — serialized as the committed baseline
/// and re-emitted on every run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuBatchReport {
    /// Workload identifier.
    pub workload: String,
    /// Whether a real GPU adapter drove the run. **`false` ⇒ the numbers are the
    /// CPU fallback's**, and the report is honestly a completes-within-ceiling /
    /// CPU-baseline run, not a GPU-throughput claim.
    pub is_gpu: bool,
    /// The backend label (e.g. `"gpu-f32:Metal"` or `"cpu-f64 (fallback)"`).
    pub backend_label: String,
    /// Host label (target triple + pointer width).
    pub host: String,
    /// Warmup dispatches discarded before timing (per batch).
    pub warmup_dispatches: u64,
    /// Timed dispatches recorded into each batch's histogram.
    pub timed_dispatches: u64,
    /// The per-batch-size measured points (ascending in `paths`).
    pub points: Vec<BatchPoint>,
}

impl GpuBatchReport {
    /// Print a human-readable summary, with the honest GPU/fallback labelling and
    /// the explicit "ratio, not absolute" framing on every throughput figure.
    pub fn print_summary(&self) {
        let us = |ns: u64| ns as f64 / 1000.0;
        println!("== celnet GPU batch throughput + dispatch-latency load harness ==");
        println!("  workload: {}", self.workload);
        println!("  host: {}", self.host);
        println!("  backend: {}", self.backend_label);
        if self.is_gpu {
            println!(
                "  device: REAL GPU adapter — throughput is a measured GPU number on THIS host\n  \
                 (a RATIO / relative-regression signal, NOT an absolute throughput; the NVIDIA\n  \
                 absolute headline + the ≤50ms exotic stay deploy-gated, never claimed here)."
            );
        } else {
            println!(
                "  device: NO GPU adapter — HEADLESS CPU FALLBACK. The throughput below is the\n  \
                 f64 CPU oracle's, labelled as such. This run is a COMPLETES-WITHIN-CEILING /\n  \
                 CPU-baseline check, NOT a GPU speedup claim (gpu/cpu ratio ≈ 1.0 by construction)."
            );
        }
        println!(
            "  dispatches: {} timed (after {} warmup discarded) per batch size",
            self.timed_dispatches, self.warmup_dispatches
        );
        println!();
        println!(
            "  {:>10} {:>14} {:>14} {:>9} {:>10} {:>10} {:>10}",
            "paths", "gpu_kpaths/s", "cpu_kpaths/s", "gpu/cpu", "disp_p50", "disp_p99", "disp_p999"
        );
        for p in &self.points {
            println!(
                "  {:>10} {:>14.1} {:>14.1} {:>8.2}x {:>9.3}µs {:>9.3}µs {:>9.3}µs",
                p.paths,
                p.throughput_paths_per_s / 1000.0,
                p.cpu_throughput_paths_per_s / 1000.0,
                p.gpu_over_cpu_ratio,
                us(p.latency.p50_ns),
                us(p.latency.p99_ns),
                us(p.latency.p999_ns),
            );
        }
        println!();
        if !self.is_gpu {
            println!(
                "  (headless: gpu/cpu ≈ 1.0 is EXPECTED — the GPU path IS the CPU fallback here.)"
            );
        }
    }
}

/// Tunables for the GPU batch load measurement. Defaults are the published-proof
/// sizing; [`GpuLoadConfig::ci`] is the smaller, gate-sized run.
#[derive(Debug, Clone)]
pub struct GpuLoadConfig {
    /// The batch sizes (paths per dispatch) to sweep.
    pub sweep_paths: Vec<u32>,
    /// Warmup dispatches discarded before timing (device/pipeline warm, page faults).
    pub warmup_dispatches: u64,
    /// Timed dispatches recorded into each batch's histogram.
    pub timed_dispatches: u64,
    /// Core index to pin the measuring thread to (best-effort jitter reduction).
    pub pin_core: usize,
}

impl Default for GpuLoadConfig {
    fn default() -> Self {
        Self {
            sweep_paths: SWEEP_PATHS.to_vec(),
            // A handful of warmup dispatches compiles the pipeline (one-off, off any
            // budget) and faults the readback buffers before timing.
            warmup_dispatches: 8,
            // Enough dispatches for a stable p99 without making the largest-batch
            // CPU-fallback run slow: 256 dispatches × up-to-1M paths is bounded.
            timed_dispatches: 256,
            pin_core: 0,
        }
    }
}

impl GpuLoadConfig {
    /// A smaller, faster sizing for the CI gate and the in-crate unit test (still
    /// enough dispatches for a sane percentile), exercising the exact path.
    #[must_use]
    pub fn ci() -> Self {
        Self {
            // Trim the top of the sweep so a headless CPU-fallback CI runner finishes
            // quickly; the gate is relative, so fewer points still catch a regression.
            sweep_paths: vec![4_096, 16_384, 65_536, 262_144],
            warmup_dispatches: 4,
            timed_dispatches: 64,
            pin_core: 0,
        }
    }

    /// The tiny sizing the in-crate unit test uses (fast, still real dispatches).
    #[must_use]
    pub fn quick() -> Self {
        Self {
            sweep_paths: vec![1_024, 4_096],
            warmup_dispatches: 2,
            timed_dispatches: 16,
            pin_core: 0,
        }
    }
}

/// Best-effort: pin the current thread to `core_index`. Returns whether it stuck.
///
/// The same affordance as [`crate::core_load`]'s pinning — a jitter reduction,
/// never a correctness requirement.
fn pin_current_thread(core_index: usize) -> bool {
    match core_affinity::get_core_ids() {
        Some(ids) => match ids.get(core_index) {
            Some(&id) => core_affinity::set_for_current(id),
            None => false,
        },
        None => false,
    }
}

/// Measure the steady-state per-dispatch throughput of a backend on one batch.
///
/// Returns `(timed_seconds, paths_per_second)`. Pure measurement of the backend's
/// `price_vanilla` (simulate + reduce), with the spec/payoff built outside the
/// timed phase so the figure is dispatch cost, not fixture allocation.
fn measure_throughput<B: PricingBackend>(
    backend: &B,
    spec: &PathSpec,
    payoff: &PayoffKernel,
    warmup: u64,
    timed: u64,
) -> (f64, f64) {
    let mut acc = 0.0_f64;
    for _ in 0..warmup {
        acc += backend.price_vanilla(spec, payoff).price();
    }
    std::hint::black_box(acc);

    let phase_start = std::time::Instant::now();
    let mut acc2 = 0.0_f64;
    for _ in 0..timed {
        let r = backend.price_vanilla(std::hint::black_box(spec), std::hint::black_box(payoff));
        acc2 += r.price();
    }
    let elapsed = phase_start.elapsed().as_secs_f64();
    std::hint::black_box(acc2);

    let total_paths = (timed as f64) * f64::from(spec.paths);
    let pps = if elapsed > 0.0 {
        total_paths / elapsed
    } else {
        0.0
    };
    (elapsed, pps)
}

/// Measure one batch size on the GPU path: per-dispatch latency into a CO-aware
/// histogram plus the achieved path throughput, and the CPU-oracle throughput on
/// the identical batch for the host-local ratio.
fn measure_batch(
    gpu: &GpuBackend,
    cpu: &CpuBackend,
    paths: u32,
    config: &GpuLoadConfig,
) -> BatchPoint {
    let spec = representative_spec(paths);
    let payoff = representative_payoff();

    // 100ns..100s window at 3 sig figs: resolves the body finely while capturing
    // any pathological stall (a large CPU-fallback batch can be many ms).
    let mut hist: Histogram<u64> =
        Histogram::new_with_bounds(1, 100_000_000_000, 3).expect("valid histogram bounds");

    // --- Warmup (discarded): compile pipeline / fault buffers --------------
    let mut acc = 0.0_f64;
    for _ in 0..config.warmup_dispatches {
        acc += gpu.price_vanilla(&spec, &payoff).price();
    }
    std::hint::black_box(acc);

    // --- Timed phase: open-loop, time each dispatch, record immediately ----
    let mut acc2 = 0.0_f64;
    let phase_start = std::time::Instant::now();
    for _ in 0..config.timed_dispatches {
        let t0 = std::time::Instant::now();
        let r = gpu.price_vanilla(std::hint::black_box(&spec), std::hint::black_box(&payoff));
        let elapsed = t0.elapsed().as_nanos();
        acc2 += r.price();
        let v = u64::try_from(elapsed).unwrap_or(u64::MAX).max(1);
        hist.saturating_record(v);
    }
    let phase_elapsed = phase_start.elapsed().as_secs_f64();
    std::hint::black_box(acc2);

    // Coordinated-omission back-fill against the measured median, so a stalled
    // dispatch can only make the tail MORE pessimistic.
    let expected_interval = hist.value_at_quantile(0.50).max(1);
    let corrected = hist.clone_correct(expected_interval);

    let latency = DispatchLatency {
        min_ns: corrected.min(),
        p50_ns: corrected.value_at_quantile(0.50),
        p99_ns: corrected.value_at_quantile(0.99),
        p999_ns: corrected.value_at_quantile(0.999),
        p9999_ns: corrected.value_at_quantile(0.9999),
        max_ns: corrected.max(),
    };

    let total_paths = (config.timed_dispatches as f64) * f64::from(paths);
    let throughput_paths_per_s = if phase_elapsed > 0.0 {
        total_paths / phase_elapsed
    } else {
        0.0
    };

    // CPU oracle on the identical batch — the ratio denominator (host-local).
    let (_cpu_s, cpu_throughput_paths_per_s) = measure_throughput(
        cpu,
        &spec,
        &payoff,
        config.warmup_dispatches,
        config.timed_dispatches,
    );

    let gpu_over_cpu_ratio = if cpu_throughput_paths_per_s > 0.0 {
        throughput_paths_per_s / cpu_throughput_paths_per_s
    } else {
        0.0
    };

    BatchPoint {
        paths,
        latency,
        throughput_paths_per_s,
        cpu_throughput_paths_per_s,
        gpu_over_cpu_ratio,
    }
}

/// Run the GPU batch load sweep and return the populated [`GpuBatchReport`].
///
/// Builds the [`GpuBackend`] once (probing for an adapter; falling back to the CPU
/// oracle headless), then measures every batch size in [`GpuLoadConfig::sweep_paths`].
/// The returned report is *not* a pass/fail by itself — the `gpu_gate` binary
/// compares it to the committed baseline via [`compare_to_baseline`].
#[must_use]
pub fn measure(config: GpuLoadConfig) -> GpuBatchReport {
    let _pinned = pin_current_thread(config.pin_core);
    let _elevated = crate::sched::request_elevated_priority();

    let gpu = GpuBackend::new();
    let cpu = CpuBackend::new();
    let is_gpu = gpu.is_gpu();
    let backend_label = gpu.label();

    let points = config
        .sweep_paths
        .iter()
        .map(|&paths| measure_batch(&gpu, &cpu, paths, &config))
        .collect();

    GpuBatchReport {
        workload: "gpu_batch_vanilla_mc_throughput".to_owned(),
        is_gpu,
        backend_label,
        host: crate::core_load::host_label(),
        warmup_dispatches: config.warmup_dispatches,
        timed_dispatches: config.timed_dispatches,
        points,
    }
}

/// One breached GPU-batch metric, for human-readable reporting and the gate.
#[derive(Debug, Clone)]
pub struct GpuBreach {
    /// The metric name (e.g. `"batch[65536].throughput"` / `"batch[65536].disp_p99"`).
    pub metric: String,
    /// The committed baseline value (paths/s for throughput, ns for latency).
    pub baseline: f64,
    /// The measured value this run.
    pub measured: f64,
    /// The ceiling/floor the measured value had to respect.
    pub bound: f64,
}

/// Compare a measured [`GpuBatchReport`] against a committed baseline at a relative
/// `tolerance` — a **slowdown-only RELATIVE / ratio gate**, NOT an absolute claim.
///
/// For each batch size present in **both** reports it flags a regression when:
///
/// * **throughput collapses** below `baseline_throughput / (1 + tolerance)` (e.g.
///   with `tolerance = 1.0`, throughput dropping below half the committed baseline
///   — i.e. a >2× slowdown), or
/// * **dispatch p99 inflates** beyond `baseline_p99 * (1 + tolerance)` (a >2×
///   dispatch-latency regression).
///
/// This is the same philosophy as the wire / fleet gates: absolute GPU numbers
/// float with the device, so the gate catches a *structural* regression, never
/// asserting an absolute throughput. On a headless runner where both baseline and
/// measurement are the CPU fallback, it is a completes-within-ceiling check.
#[must_use]
pub fn compare_to_baseline(
    baseline: &GpuBatchReport,
    measured: &GpuBatchReport,
    tolerance: f64,
) -> Vec<GpuBreach> {
    let mut breaches = Vec::new();
    for base in &baseline.points {
        let Some(meas) = measured.points.iter().find(|p| p.paths == base.paths) else {
            continue;
        };

        // Throughput floor (slowdown-only): a regression is throughput FALLING.
        let throughput_floor = base.throughput_paths_per_s / (1.0 + tolerance);
        if base.throughput_paths_per_s > 0.0 && meas.throughput_paths_per_s < throughput_floor {
            breaches.push(GpuBreach {
                metric: format!("batch[{}].throughput", base.paths),
                baseline: base.throughput_paths_per_s,
                measured: meas.throughput_paths_per_s,
                bound: throughput_floor,
            });
        }

        // Dispatch-p99 ceiling (slowdown-only): a regression is latency RISING.
        let p99_ceiling = base.latency.p99_ns as f64 * (1.0 + tolerance);
        if (meas.latency.p99_ns as f64) > p99_ceiling {
            breaches.push(GpuBreach {
                metric: format!("batch[{}].disp_p99", base.paths),
                baseline: base.latency.p99_ns as f64,
                measured: meas.latency.p99_ns as f64,
                bound: p99_ceiling,
            });
        }
    }
    breaches
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measurement path runs end-to-end on a tiny sizing, populates a monotone
    /// dispatch distribution and positive throughput for every batch, and records
    /// the backend identity honestly. (No absolute-throughput assertion — that
    /// would be an overclaim; the gate is purely relative.)
    #[test]
    fn gpu_load_measures_a_real_sweep() {
        let report = measure(GpuLoadConfig::quick());

        assert_eq!(report.points.len(), 2, "both quick batch sizes measured");
        assert_eq!(report.points[0].paths, 1_024);
        assert_eq!(report.points[1].paths, 4_096);
        // The backend label is non-empty and the is_gpu flag is self-consistent
        // with the "(fallback)" marker (honest headless labelling).
        assert!(!report.backend_label.is_empty());
        if !report.is_gpu {
            assert!(
                report.backend_label.contains("fallback"),
                "a non-GPU run must be labelled a fallback: {}",
                report.backend_label
            );
        }

        for p in &report.points {
            // Distribution is sane and monotone.
            assert!(p.latency.min_ns >= 1, "min ≥ 1ns");
            assert!(p.latency.p50_ns >= p.latency.min_ns, "p50 ≥ min");
            assert!(p.latency.p99_ns >= p.latency.p50_ns, "p99 ≥ p50");
            assert!(p.latency.p999_ns >= p.latency.p99_ns, "p99.9 ≥ p99");
            assert!(p.latency.max_ns >= p.latency.p999_ns, "max ≥ p99.9");
            // Real work produced positive throughput on both paths.
            assert!(
                p.throughput_paths_per_s > 0.0,
                "positive GPU-path throughput"
            );
            assert!(
                p.cpu_throughput_paths_per_s > 0.0,
                "positive CPU-oracle throughput"
            );
            assert!(p.gpu_over_cpu_ratio > 0.0, "positive ratio");
            // Headless: the GPU path IS the CPU fallback, so the ratio is ~1.0.
            if !report.is_gpu {
                assert!(
                    (0.2..5.0).contains(&p.gpu_over_cpu_ratio),
                    "headless gpu/cpu ratio should be O(1) (same backend): {}",
                    p.gpu_over_cpu_ratio
                );
            }
        }
    }

    /// `compare_to_baseline` flags exactly the regressions: a throughput collapse
    /// and a dispatch-p99 inflation beyond tolerance, and nothing within tolerance.
    #[test]
    fn compare_flags_exactly_the_regressions() {
        let point = |paths: u32, tput: f64, p99: u64| BatchPoint {
            paths,
            latency: DispatchLatency {
                min_ns: 1_000,
                p50_ns: 2_000,
                p99_ns: p99,
                p999_ns: p99 + 1_000,
                p9999_ns: p99 + 2_000,
                max_ns: p99 + 3_000,
            },
            throughput_paths_per_s: tput,
            cpu_throughput_paths_per_s: tput,
            gpu_over_cpu_ratio: 1.0,
        };
        let report = |pts: Vec<BatchPoint>| GpuBatchReport {
            workload: "t".to_owned(),
            is_gpu: false,
            backend_label: "cpu-f64 (fallback)".to_owned(),
            host: "t".to_owned(),
            warmup_dispatches: 0,
            timed_dispatches: 0,
            points: pts,
        };

        let base = report(vec![point(4_096, 1_000_000.0, 5_000)]);

        // Within tolerance (tol = 1.0 ⇒ allow 2× slower / half throughput): ok.
        let ok = report(vec![point(4_096, 600_000.0, 9_000)]);
        assert!(compare_to_baseline(&base, &ok, 1.0).is_empty());

        // Throughput collapses to a quarter (< half) AND p99 triples (> 2×): two breaches.
        let bad = report(vec![point(4_096, 250_000.0, 15_000)]);
        let breaches = compare_to_baseline(&base, &bad, 1.0);
        assert_eq!(breaches.len(), 2, "{breaches:?}");
        assert!(breaches.iter().any(|b| b.metric.contains("throughput")));
        assert!(breaches.iter().any(|b| b.metric.contains("disp_p99")));
    }

    /// A batch size absent from the measured run is simply skipped (no panic, no
    /// false breach) — the gate only compares the intersection of batch sizes.
    #[test]
    fn compare_skips_missing_batch_sizes() {
        let point = BatchPoint {
            paths: 4_096,
            latency: DispatchLatency {
                min_ns: 1_000,
                p50_ns: 2_000,
                p99_ns: 5_000,
                p999_ns: 6_000,
                p9999_ns: 7_000,
                max_ns: 8_000,
            },
            throughput_paths_per_s: 1_000_000.0,
            cpu_throughput_paths_per_s: 1_000_000.0,
            gpu_over_cpu_ratio: 1.0,
        };
        let mk = |pts: Vec<BatchPoint>| GpuBatchReport {
            workload: "t".to_owned(),
            is_gpu: false,
            backend_label: "cpu-f64 (fallback)".to_owned(),
            host: "t".to_owned(),
            warmup_dispatches: 0,
            timed_dispatches: 0,
            points: pts,
        };
        let base = mk(vec![point]);
        let empty = mk(vec![]);
        assert!(compare_to_baseline(&base, &empty, 1.0).is_empty());
    }
}
