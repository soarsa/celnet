# Celnet — GPU-at-Scale Plan (`celnet-gpu`: functional → perf-at-scale, proven)

> Plan to take `celnet-gpu` from **functional + reconciled** (a wgpu/Metal f32
> vanilla GBM Monte-Carlo backend with a Philox counter-RNG and an f64 CPU
> reconciliation oracle — built, tested, all reconciliation gates green) to
> **perf-at-scale, proven**: large many-pair / many-tenor surface + exotic
> Monte-Carlo / PDE workloads on GPU, with a benchmark + reconciliation harness
> that measures GPU batch throughput and latency against the §1.2 budgets and
> proves the f32-GPU / f64-CPU agreement within a *derived* error bound, both
> headless in CI (Mesa Lavapipe software Vulkan) and on real Metal here.
>
> This is a **design / sequencing document**, not a record of what is built.
> §1 states the honest starting point; every build item carries a validation
> gate (§8). It is governed by `CLAUDE.md` guardrails (OSS-only, no commercial
> products, no mocks, vendor-neutral names, scale-as-requirement) and validated
> against `docs/ARCHITECTURE.md` §4 (GPU abstraction) and §1.2 (latency budgets,
> esp. the **≤ 50 ms booking-grade path-dependent exotic** budget), and
> `docs/SCALE-OUT.md` (shard-local, off-hot-path dispatch).

---

## 0. Governing constraints (what GPU is and isn't, here)

1. **GPU is off the hot path, always.** Per `docs/ARCHITECTURE.md` §3/§4 and
   `docs/SCALE-OUT.md` §1, the latency-critical vanilla price + Greeks (p50 ≤ 2 µs)
   stays on the pinned CPU core. GPU serves the **path-dependent exotic** budget
   (interactive **≤ 50 ms** for booking-grade accuracy) and **large batch**
   surface/portfolio sweeps, dispatched async from `celnet-engine`'s GPU-dispatch
   core. A cross-PCIe / cross-device hop is *disqualifying* on the per-tick path;
   it is *expected* on the per-RFQ-exotic and batch-revaluation paths.
2. **f32 on GPU, f64 on CPU — non-negotiable on this hardware.** WGSL/Metal have
   no native f64 (WGSL scalars are `f32`/`f16`/`i32`/`u32`; atomics integer-only).
   The whole reconciliation discipline exists *because* of this. Every GPU result
   is bounded against the f64 CPU oracle by a derived error bound, never asserted
   plausible.
3. **Determinism survives the GPU.** The Philox-4×32-10 counter-RNG produces
   bit-identical *integers* on CPU and GPU (exact u32 arithmetic, mirrored in
   `counter_rng.rs` ↔ `shader.wgsl`); only the int→float conversion and payoff
   arithmetic differ in precision, and that difference is the reconciliation
   bound. A QMC variate source (Sobol, §5) must preserve this: the point set is a
   deterministic function of `(dimension, index)` plus a fixed scramble seed.
4. **OSS-only datapath, CUDA first-class but not required.** The primary backend
   is `wgpu` (Metal/Vulkan/DX12) + hand-authored WGSL; Mesa Lavapipe gives a
   software-Vulkan device for GPU-less CI. A CUDA path is a *first-class CI/deploy
   target validated on Linux*, never a build dependency of the core (guardrail 7).

---

## 1. Honest starting point (built today)

From a read of `crates/celnet-gpu/src/` (`backend.rs`, `cpu.rs`, `gpu.rs`,
`counter_rng.rs`, `shader.wgsl`, `lib.rs`) and the CI lane in
`.github/workflows/ci.yml`:

| Capability | Status | Where |
|---|---|---|
| `PricingBackend` trait (scalar-generic `F: Scalar`; `simulate_paths` + `reduce_payoff` + fused `price_vanilla`) | **Built** | `backend.rs` |
| f64 `CpuBackend` reference oracle (libm transcendentals, deterministic pairwise/tree sum) | **Built** | `cpu.rs` |
| f32 `GpuBackend` over wgpu 29 (Metal/Vulkan/DX12), runtime adapter probe, transparent CPU fallback, **bounded** 30 s readback deadline | **Built** | `gpu.rs` |
| Philox-4×32-10 counter-RNG, bit-identical CPU↔WGSL, known-answer-tested (Random123 KAT vectors) | **Built** | `counter_rng.rs`, `shader.wgsl` |
| Box-Muller normals, workgroup tree reduction in shared memory (no float atomics), host pairwise finish | **Built** | `shader.wgsl`, `gpu.rs` |
| *Derived* f32↔f64 reconciliation bound (from `f32::EPSILON`, WGSL ULP budgets, realized path stats — not a fitted constant) | **Built** | `gpu.rs::derived_reconciliation_bound` |
| Reconciliation + ragged-tail + closed-form-bracket tests; asserts `is_gpu()` when an adapter exists so the shader path is genuinely exercised | **Built** | `gpu.rs` tests |
| Headless CI GPU lane on Mesa Lavapipe software Vulkan (`VK_ICD_FILENAMES=lvp_icd`, `WGPU_BACKEND=vulkan`) | **Built** | `.github/workflows/ci.yml` `gpu-lavapipe` |
| **Batched spot×vol scenario kernel** (`ScenarioPricer`/`ScenarioAxes`/`ScenarioGrid`): whole ladder priced in **one 2-D dispatch** (x = path-block, y = grid node) under common random numbers; per-node f32↔f64 reconciliation to the CPU oracle (same derived bound); ragged-tail + monotone-under-CRN + centre-node tests; **CPU oracle fallback** node-by-node when no adapter. Wired into the risk cube as `celnet_risk_cube::gpu_pv_grid` (sums notional-scaled per-position grids) with `NodeScenarioGrid::reconciles_to_analytic` vs the closed-form oracle. **Measured (M4 Metal, `celnet-bench`):** 121 nodes × 3 positions × 65 536 paths/node ~13 ms batched vs ~165 ms unbatched per-node MC (~12.7×); the exact closed form (~9.6 µs) still dominates for analytic vanillas (honest crossover — GPU MC is the scale path for path-dependent payoffs / large amortizing grids). | **Built** | `scenario.rs`, `scenario.wgsl`; `celnet-risk-cube::scenario_grid.rs`; `celnet-bench::scenario.rs` |
| **Batch closed-form vanilla kernel** (Workload A / G2 — `BatchPricer`/`BatchInstrument`): **one 1-D dispatch** prices a large batch of *independent* vanillas (varying spot/strike/vol/expiry/rates/call-put) by the **Garman-Kohlhagen closed form in f32**, one thread per instrument — a smooth, exact, embarrassingly-parallel batch with **no Monte-Carlo noise**. WGSL has no `erf`, so Φ is the **A&S 7.1.26** rational-times-Gaussian (max abs `erf` error `1.5e-7`, at the f32 floor); its f32 coefficients are **bit-identical** to the CPU oracle's (`erf_as_oracle`) and to the canonical A&S constants. Reconciliation is **split into two separately-derived bounds**: (i) GPU-f32 vs the **f64 A&S-erf oracle** within `derived_batch_bound` (pure f32 round-off, from `f32::EPSILON` and the GK condition numbers `dmax`/`scale` — *not* a fitted constant); (ii) the f64 A&S oracle vs `celnet_vanilla::price` (production `libm::erfc` Φ, gated bit-for-bit vs the **QuantLib golden** in `celnet-golden`) within `as_erf_price_bound` (A&S algorithmic error). The **three-way bracket** `GPU-f32 ~ f64-oracle ~ golden-closed-form` holds node-by-node within their sum. **Measured (M4 Metal):** n=1792-instrument batch (ITM/ATM/OTM × short/long expiry × varied vol/rates × call/put), `is_gpu=true`, **max f32 round-off rel = 3.11e-7**, max A&S-alg rel = 1.24e-7 — both inside the per-node derived bounds (f32 round-off well under the ~few×1e-6 f32 expectation). Bit-reproducible (no RNG: `to_bits`-identical across runs/backends); ragged-tail tested; **CPU oracle fallback** reconciles to golden when no adapter (headless/CI). **HONEST BOUNDARY:** in-repo this proves **correctness** (the f32↔f64↔golden reconcile) only; the M4 is integrated-GPU/unified-memory and has no f64 — the **absolute throughput / speedup** for Workload A is a *ratio* on this host and the NVIDIA absolute headline is **deferred to the CUDA deploy-gate (G8)**, never claimed here. | **Built** | `batch.rs`, `batch.wgsl` |

**Built since this plan was first written (the gap is largely closed):**

- **Multi-step path engine on GPU — Built (W8).** `path.wgsl` runs a multi-step GBM path
  (Sobol or Philox variates, Brownian-bridge ordering) so path-dependent payoffs run on GPU;
  three-way GPU-f32 ≈ CPU-f64 ≈ golden within derived f32 bounds, with a CPU↔GPU **Sobol KAT**
  (`celnet-parity/tests/{gpu_path,gpu_greeks,qmc_highdim}.rs`).
- **Batch / many-instrument dispatch — Built.** The Workload-A closed-form batch kernel
  (`batch.wgsl`, one 1-D dispatch over a large independent-vanilla batch) and the spot×vol
  scenario kernel (`scenario.wgsl`, one 2-D dispatch) — see the Built table above.
- **QMC — Built (`celnet-qmc`).** Owen-scrambled Joe-Kuo Sobol' + Brownian-bridge, consumed
  unmodified by the GPU path/greeks kernels; the integer/direction-number layer is bit-identical
  CPU↔GPU (KAT). Measured ≈37.7×/88.5× variance reduction (`celnet-parity/tests/qmc.rs`).
- **GPU throughput/latency RATIO harness — Built.** `celnet-bench` `gpu_load` (HdrHistogram
  GPU+CPU kpaths/s + gpu/cpu ratio + dispatch p50/p99/p99.9), `benches/gpu_batch.rs` +
  `baselines/gpu_batch.json`, and `gpu_gate` (slowdown-only relative gate). Measured M4 Metal
  dispatch-amortization curve 0.68×@4k → 53×@1M paths.
- **Greeks on GPU — Built (W8).** `greeks.wgsl` carries pathwise + likelihood-ratio Greek
  estimators in the path sweep, each reconciled vs the f64 CPU estimator and analytic golden
  (`celnet-parity/tests/gpu_greeks.rs`).

**Still genuinely not built (the remaining gap):**

- **No GPU PDE** (the production CN+Rannacher and HV-ADI PDEs are CPU-only in
  `celnet-exotics`; the GPU PDE path is *designed* in §4 of ARCHITECTURE).
- **No absolute GPU throughput headline.** The M4 is integrated-GPU/unified-memory and has
  **no f64**, so in-repo the GPU lane proves **correctness** (f32↔f64↔golden) + host-local
  **ratios** only. The **NVIDIA ABSOLUTE throughput / ≤ 50 ms exotic / Workload-A·B absolute
  numbers are deferred to the CUDA deploy-gate (G8)**, never claimed in-repo.

So the honest one-line state: **the GPU path is correct, reconciled, at-scale (batch/path/Greeks
on GPU + Sobol QMC), and host-ratio perf-proven; the GPU PDE and the NVIDIA absolute headline are
the remaining deploy-gated items.** The rest of this document is the design route + the deploy-gate.

---

## 2. Target workloads (what goes on GPU, and the expected speedup)

GPU wins when arithmetic intensity × parallel width amortizes the host↔device
round-trip. The CPU path already serves single-option pricing inside ~23 ns
(`celnet-bench`), so the GPU's job is the workloads where the **CPU is the
bottleneck**: deep path-dependent exotics, and wide batch revaluation.

### 2.1 Workload A — many-pair / many-tenor / many-strike vanilla & first-gen surface batch

A full-portfolio or full-surface revaluation prices `O(10⁴–10⁶)` (pair, tenor,
strike) points. Today this is a CPU sweep at ~52–105 ns/option (`celnet-bench`
`batch::*`); a 1M-point book revaluation is ~50–105 ms of CPU wall time on one
core. Closed-form GK is *cheap per point* but *embarrassingly wide*, so the win
here is **batching the whole grid into one (or few) GPU dispatches** rather than
per-point MC. This is a **kernel-fusion** workload, not a path workload:

- One GPU thread per (pair, tenor, strike) point; uniform/structured-buffer inputs
  for the per-pair surface params and per-tenor discount factors; one output buffer
  of prices (and Greeks, §6).
- **Expected speedup vs the one-core CPU sweep: ~20–100×** for a ≥ 10⁵-point grid
  on a mid-range discrete GPU, and a more modest ~5–20× on the M4 integrated GPU
  (shared-memory bandwidth bound, no PCIe transfer). The win is bandwidth- and
  occupancy-bound, not flop-bound, so it scales with grid width and saturates once
  the device is full. *This number is a hypothesis to be measured (§3), not a
  claim.* Use cases: scenario/stress grids, EOD full-book revaluation, surface
  arbitrage scans across the whole liquid universe.

### 2.2 Workload B — path-dependent exotic Monte-Carlo (the ≤ 50 ms budget)

This is the **headline** GPU workload and the reason the ≤ 50 ms booking-grade
budget exists. Single-asset, multi-step GBM (and later LSV) paths with
path-dependent payoffs: barriers (all 8 single + double-KO), touches/no-touches,
DNT, window barriers, geometric/arithmetic Asians, and autocallables.

- One GPU thread per path; `steps` time-steps per path drawing
  `normal(path, step, dim)` from the Philox stream (the address scheme already
  supports `step`/`dim` — only the CPU/shader engines ignore `steps > 1` today).
- Per-path running state machine (running min/max for barriers, running sum/product
  for Asians, knock flags for autocalls) kept in registers; one scalar payoff per
  path; the same workgroup tree-reduction + host pairwise finish already in place.
- Apply the **Broadie-Glasserman-Kou `0.5826·σ·√dt` continuity correction** (the
  CPU MC in `celnet-exotics` already implements it) in the kernel for discretely-
  monitored barriers, and a Brownian-bridge max/min refinement once QMC lands (§5).
- **Expected speedup vs the CPU MC oracle: ~30–134×** is the band reported across
  the GPU-MC option-pricing literature (≈ 30× for European QMC, up to ≈ 134× for
  barrier options on GPU; see refs). On the M4 integrated GPU expect the lower end
  of that band; on a discrete CUDA/Vulkan device the upper end. The concrete
  acceptance criterion is **not** the multiplier — it is **landing a booking-grade
  barrier/DNT price (target ≤ 1e-3 relative MC error vs the QuantLib golden) inside
  the 50 ms wall-clock budget**, which the harness (§3) measures directly.

### 2.3 Workload C — GPU PDE for low-dimensional exotics

For 1-D (single barrier/digital) and 2-D (LSV / two-factor) problems a PDE can be
*more accurate per unit time* than MC near barriers. The CPU CN+Rannacher (1-D)
and HV-ADI (2-D) in `celnet-exotics` are the oracle. GPU PDE is the **lowest-
priority / most-deferred** item (§7) because:

- Explicit / ADI schemes map to **tiled stencil kernels** using workgroup shared
  memory (good GPU fit), but
- implicit / Crank-Nicolson needs a **tridiagonal solve** per time-step, which on
  GPU means **cyclic reduction / parallel-cyclic-reduction (PCR)** — correct but a
  large, separately-validated kernel, and the 1-D grids that dominate FX-exotic
  bookings (a few hundred spatial nodes) are *already* sub-millisecond on the CPU.

So GPU PDE is built **only if** a measured workload (e.g. dense 2-D LSV barrier
grids across the book) shows the CPU ADI as the bottleneck. Expected speedup for a
large 2-D grid is ~10–40×; for the typical 1-D booking grid it is **negative**
(transfer + launch overhead dominates) — which is itself a finding to record.

### 2.4 Non-targets (stay on CPU)

- Single vanilla price + Greeks on the hot path (CPU wins by orders of magnitude).
- Surface *calibration* solvers (VV/SABR/SVI/SSVI) — iterative, small, latency-
  sensitive root-finds; CPU. (A batched *re-strike* of an already-calibrated
  surface is Workload A and *is* a GPU target.)
- Anything that must return inside the 2 µs / 150 µs hot-path budgets.

---

## 3. Benchmark + reconciliation harness design

The harness must answer two orthogonal questions, kept strictly separate (the
same discipline `celnet-bench` already applies to in-core vs wire-path):

1. **Is it correct?** GPU f32 result vs f64 CPU oracle, within a *derived* bound.
2. **Is it fast enough?** GPU batch **throughput** and **dispatch latency** vs the
   §1.2 budgets — especially the ≤ 50 ms exotic budget.

### 3.1 Where it lives

- **Correctness / reconciliation** stays as `#[test]`s in `celnet-gpu` (extending
  the existing `gpu.rs` tests) so it runs in `just check` and in the CI Lavapipe
  lane on every PR. It must be **fast** (Lavapipe is a CPU rasterizer — keep path
  counts modest in the CI reconciliation tests; the *throughput* numbers come from
  the perf harness on real hardware, not from Lavapipe).
- **Performance** is a new `celnet-bench` surface: `benches/gpu_batch.rs` (divan,
  for warm-cache micro-throughput of a fixed grid) **plus** a bounded binary
  `src/bin/gpu_load.rs` (mirroring the existing `wire_load`/`bench_gate` pattern)
  that drives realistic batch/exotic dispatches and records HdrHistogram
  percentiles + throughput into a committed baseline
  `baselines/gpu_batch.json` / `baselines/gpu_exotic.json`. A `gpu_gate` binary
  (mirroring `bench_gate`) re-measures and fails CI on a structural regression
  (`> 2×` baseline tolerance, slowdown-only — the same philosophy as the wire
  gate, since absolute GPU numbers float with the device).

### 3.2 Throughput + latency measurement (the "fast enough?" question)

- **Reuse one device context across the whole measured run.** The current
  `GpuBackend::dispatch` re-creates param/output/readback buffers per call; for a
  batch sweep this is the overhead under test, so the harness builds a persistent
  context with pre-sized, reused buffers (a build item, §7 G2) and times steady-
  state dispatch, separating **one-time init** (adapter + pipeline compile — off
  any budget) from **per-dispatch** cost (on the budget).
- **Three timing strata, reported distinctly** (do not blend them):
  1. **Kernel-only** GPU time via wgpu timestamp queries (`TIMESTAMP_QUERY`
     feature where the adapter supports it; Lavapipe and some Metal configs may
     not — degrade to wall-clock and label it). This is the device compute floor.
  2. **Dispatch wall-clock** = encode + submit + device poll + readback map +
     host reduction finish. This is what `celnet-engine`'s GPU-dispatch core
     actually waits on, so **this is the number compared to the ≤ 50 ms budget**.
  3. **End-to-end RFQ-exotic** (optional, later): drive a barrier RFQ through the
     real `celnet-server` edge → engine → GPU dispatch → reconcile → respond,
     timed client-side, the GPU analogue of the existing `wire_load`. This proves
     the ≤ 50 ms budget *as a counterparty observes it*, including framing.
- **Throughput** = paths/s (Workload B) and points/s (Workload A) at steady state;
  reported as a ratio to the one-core CPU sweep so the speedup claim is auditable
  (the durable claim is the *ratio*, not the absolute, exactly as the existing
  bench README argues).
- **Coordinated-omission-aware** HdrHistogram (reuse the `hdrhistogram` dep and
  the harness shape already proven in `celnet-bench/src/wire.rs`).

### 3.3 Reconciliation measurement (the "correct?" question)

- **Generalize the existing `derived_reconciliation_bound`** (currently per-vanilla
  in `gpu.rs`) into a reusable bound that covers multi-step paths and path-
  dependent payoffs. The error model extends naturally: per-step the f32 GBM
  increment carries `≲ (K_bm+4)·eps·σ·√dt·z_max`, accumulated over `steps` steps,
  plus the payoff/widening floor; barriers add a discrete-monitoring term bounded
  by the BGK correction's own f32 error. The bound stays **derived from
  `f32::EPSILON` and realized path statistics**, never a fitted constant — this is
  a hard project rule and the existing vanilla bound is the template.
- **Path-level reconciliation, not just price-level.** For a sample of path
  indices, compare the GPU f32 terminal (and running min/max/sum for exotics)
  against the f64 CPU oracle path-by-path (the trait already exposes
  `CpuBackend::terminal_spot`; extend with a per-path state probe). This localizes
  a divergence to a kernel bug vs. expected f32 drift.
- **Triangulation against the golden oracle.** For every exotic with a closed-form
  or PDE reference, assert GPU-MC ≈ CPU-MC ≈ `celnet-golden` (QuantLib 1.42.1)
  within `MC-noise + derived-f32-bound`. The existing vanilla test already
  brackets the GK closed form within `4·se + bound`; replicate that three-way
  bracket (GPU-MC / CPU-MC / golden) for barriers and digitals. This is the
  strongest honest statement of correctness and is the gate for declaring an exotic
  "GPU-ready".
- **Cross-backend bit-stability of the RNG layer** is already KAT-tested for
  Philox integers; add the same for Sobol direction numbers + scramble (§5) so the
  *integer* layer stays bit-identical CPU↔GPU and only the float conversion drifts.

### 3.4 Headless-CI vs real-hardware split (Lavapipe ≠ a perf oracle)

- **CI (Lavapipe software Vulkan, Linux):** runs the **correctness/reconciliation**
  suite — it genuinely exercises the wgpu→Vulkan→WGSL path and proves the kernel is
  *correct* on a real Vulkan implementation, with **no GPU present**. Lavapipe is a
  CPU rasterizer (LLVM-compiled shaders), so it is **not** a throughput oracle: the
  perf gate on Lavapipe asserts only *functional completion within a generous wall-
  clock ceiling* (it must not hang), never a speedup. Keep CI path counts small.
- **This M4 host (Metal):** runs the real-Metal correctness suite (the existing
  tests already assert `is_gpu()` here so the shader path isn't bypassed) **and** is
  where the integrated-GPU throughput/latency baseline is taken — with the honest
  caveat that Metal has **no f64**, so the M4 can only ever validate the *f32 GPU*
  side; the f64 reconciliation reference is computed on the CPU. The M4 gives a
  *unified-memory, no-PCIe* perf picture (best case for transfer, modest for raw
  flops).
- **CI-container / deployment gate (real NVIDIA, Linux):** the CUDA-via-Vulkan (or
  later cubecl-cuda) path on a real discrete GPU is where the **headline speedup**
  and the **≤ 50 ms booking-grade exotic** budget are proven for production
  hardware. This is a **deployment gate**, not a per-PR gate (no NVIDIA in dev or
  in the default CI runner). It produces the production-hardware baseline that the
  GA latency headline is allowed to cite.

| Question | M4 / Metal (dev) | CI / Lavapipe (Linux, no GPU) | Deploy gate / real NVIDIA |
|---|---|---|---|
| f32 GPU kernel **correct** (vs f64 CPU bound) | ✅ (Metal shader path) | ✅ (software Vulkan path) | ✅ |
| f64 anything **on the GPU** | ❌ (Metal has no f64) | ❌ | ✅ (Vulkan/CUDA f64, but 16–64× slower — reference only) |
| **Throughput** speedup vs CPU | partial (integrated GPU, unified memory) | ❌ (CPU rasterizer — not a perf oracle) | ✅ (the headline number) |
| **≤ 50 ms exotic** budget, prod-grade | indicative (integrated) | ❌ | ✅ (the gating proof) |
| Runs on **every PR** | dev-only (manual / `just`) | ✅ (`gpu-lavapipe` lane) | ❌ (scheduled / pre-release) |

**Honesty rule (mirrors `docs/SCALE-OUT.md` §0 and the bench README):** the GA
"GPU at scale" headline may only cite the **real-NVIDIA deploy-gate** numbers; M4
numbers are labelled "integrated GPU, unified memory"; Lavapipe is **correctness
only, never perf**. No speedup is asserted that the harness has not measured on the
hardware class being claimed.

### 3.5 Performance harness — G1 BUILT (this repo, `celnet-bench`)

The G1 perf-measurement skeleton is built over the **existing** `celnet-gpu`
`GpuBackend` (it does **not** depend on any new kernel), mirroring the proven
`wire_load` / `bench_gate` / `core_load` / `surface_rebuild` patterns:

- **`celnet-bench/src/gpu_load.rs`** + the bounded **`src/bin/gpu_load.rs`**
  binary — drives the `GpuBackend` MC pricing path over a batch-size sweep
  (`4 096 … 1 048 576` paths/dispatch), times each dispatch into a
  coordinated-omission-aware HdrHistogram, and reports per-batch **dispatch
  latency** (p50/p99/p99.9) + **instrument throughput** (priced paths/s) **and**
  the same sweep on the `CpuBackend` oracle, so it emits a **measured host-local
  GPU/CPU throughput RATIO**. It records `is_gpu` + the backend label, so a
  headless (no-adapter) run is labelled the **CPU fallback** and its throughput is
  honestly the CPU oracle's. Bounded by construction (fixed sweep × fixed dispatch
  count + the backend's bounded readback deadline).
- **`benches/gpu_batch.rs`** (divan) — the warm-cache micro-throughput companion
  (GPU and CPU items/s over the same fixed sweep).
- **`baselines/gpu_batch.json`** — the committed relative-regression baseline,
  captured on THIS M4 host (`gpu-f32:Metal:Apple M4`).
- **`src/bin/gpu_gate.rs`** (mirrors `bench_gate`) — a **slowdown-only RELATIVE /
  ratio** gate: it fails on a structural regression (throughput collapsing below
  `baseline / (1 + tol)`, default tol = 1.0 ⇒ a > 2× drop, or dispatch p99
  inflating beyond `baseline × (1 + tol)`). It is **explicitly NOT** an
  absolute-throughput assertion; on a headless CI runner (Lavapipe / none) it
  degrades to a completes-within-ceiling check.

**Measured on this M4 (Metal, real adapter — a RATIO on this host, NOT an absolute
throughput; the NVIDIA absolute headline + the ≤ 50 ms exotic stay deploy-gated and
are never claimed here):** the GPU/CPU throughput ratio rises with batch size from
~0.7× at 4 096 paths (per-dispatch fixed overhead dominates) through ~7.5× at
65 536 and ~53× at 1 048 576 paths (device saturated) — the expected dispatch-
amortization shape. This is exactly the §3.2 framing: the durable claim is the
**ratio + relative-regression signal**, not an absolute number.

---

## 4. f32-vs-f64 error bound — the derived-bound discipline (extended)

The crate already derives the vanilla bound from first principles (`gpu.rs`
`derived_reconciliation_bound`): `rel = ((K_bm+4)·σ√T·z_max + K_exp + K_payoff)·eps`
with `eps = f32::EPSILON`, WGSL transcendental ULP budgets from the WebGPU spec,
and a `16·eps` absolute floor — *not* a hand-tuned constant. Extending to scale:

- **Multi-step paths:** the per-step increment error compounds; bound the
  accumulated log-spot error as `Σ_steps` of the per-step term, i.e.
  `≲ (K_bm+4)·eps·σ·√dt·z_max·steps^{½..1}` (use the conservative linear-in-steps
  bound unless a tighter √steps random-walk argument is justified and tested).
- **Path-dependent payoffs:** running min/max accumulate the same per-step relative
  error; the barrier crossing decision is the sensitive point — bound it with the
  BGK shift's own f32 representation error and verify against the CPU oracle's
  *same* shift.
- **Batch/structured inputs:** widening per-workgroup f32 partials to f64 before the
  host pairwise finish (already done) keeps the reduction error `O(log n)`; the
  bound's `K_payoff` term already covers the widen.
- **The bound is the test oracle, not a comment.** Every new GPU kernel ships with
  its derived bound *and* the path-level + golden triangulation that confirms the
  realized error sits inside it across the parameter grid (proptest-driven over
  realistic FX ranges). If realized error exceeds the derived bound, that is a
  **kernel bug**, surfaced loudly — never a reason to loosen the constant.

---

## 5. Sobol QMC + Brownian bridge — **BUILT** (`celnet-qmc`, consumed on GPU)

> **STATUS: built.** Owen-scrambled Joe-Kuo Sobol' + principal-bisection Brownian bridge ship in
> `celnet-qmc` (`sobol.rs`/`bridge.rs`/`direction_numbers.rs`), swappable behind `VariateSource`,
> consumed unmodified by the GPU path/greeks kernels (`path.wgsl`/`greeks.wgsl`) with a CPU↔GPU
> Sobol KAT. Measured ≈37.7×/88.5× variance reduction (`celnet-parity/tests/qmc.rs`); high-dim RQMC
> convergence in `qmc_highdim.rs`. The design rationale below is retained.

Pseudo-random Philox converges at `O(N^{-1/2})`. For the smooth-ish payoffs that
dominate FX exotics, **randomized QMC (scrambled Sobol) + Brownian-bridge
dimension ordering** converges substantially faster and yields a practical error
bound from the inter-randomization variance — the single highest-leverage variance
reduction for Workload B.

### 5.1 Why it pays (grounded in the literature)

- A Sobol Brownian-bridge generator has been reported to cut the statistical error
  of a structured equity portfolio by ~3×, i.e. a **~9× speed-up at fixed MC
  error** (HPC-QuantLib). European QMC option pricing on GPU has reported ~30× over
  CPU; QMC generally outperforms plain MC, Brownian-bridge construction generally
  outperforms the standard construction, and PCA construction generally beats both
  (more accurate but less GPU-friendly — Leobacher, arXiv:1707.04293).
- **Scrambling is the key 2024-2025 refinement.** Plain (deterministic) Sobol gives
  no error estimate and can be biased; **Owen scrambling** (and the GPU-friendly
  *ART-Owen* construction, Ahmed & Pharr, *ACM TOG* 2023, `10.1145/3618307`) gives
  RQMC: asymptotically better-than-MC convergence for smooth integrands *and* a
  variance estimate from `R` independent scrambles, providing the confidence
  interval booking-grade pricing needs. Scrambled-Sobol "supercharged QMC" for
  exotic option pricing is current practice (Hok, arXiv:2210.16548).

### 5.2 Design (behind the existing `VariateSource` seam)

- **New variate source, swappable with Philox.** Implement a `SobolNormals`
  alongside `CounterNormals`, selected behind the `VariateSource` abstraction the
  architecture already names (so the path engine is agnostic). Direction numbers
  from a published OSS table (e.g. Joe-Kuo new-direction-numbers, freely licensed)
  shipped as a committed data table in `celnet-gpu`; **no commercial generator**.
- **Determinism preserved.** Sobol point `(dimension d, index i)` is a pure
  function of the direction numbers; the **scramble** is a deterministic keyed
  permutation seeded once per run (the QMC analogue of the Philox run seed). Like
  Philox, the *integer/bit* layer must be bit-identical CPU↔WGSL (KAT-tested,
  §3.3); only the int→float conversion differs.
- **GPU-resident generation.** Gray-code Sobol generation is sequential per
  dimension but **parallel across paths/dimensions**; generate on-device (one
  thread per path, looping dimensions = time-steps after the bridge reorders them),
  or precompute a tile and stream it. Scrambling is a per-coordinate XOR/permutation
  (ART-Owen is explicitly designed to be cheap and screen-space/GPU-friendly).
- **Brownian-bridge construction.** Reorder the `steps` Sobol dimensions so the
  lowest-discrepancy early dimensions set the most significant path features
  (terminal, then midpoint, then quarters …). Implement a `BrownianBridge` path
  constructor (the CPU MC has *no* bridge today — confirmed) shared by CPU oracle
  and GPU kernel so reconciliation still holds dimension-for-dimension.
- **RQMC error bound.** Replace the i.i.d. `1/√N` std-error with the variance across
  `R` independent scrambles (typically `R = 8..32`); report that as the MC error in
  `Reduction` (a new `qmc_std_error`). This is what lets the harness assert
  "booking-grade ≤ 1e-3 relative error inside 50 ms" *with* a statistically valid
  interval.

### 5.3 Validation gate

QMC must (a) reconcile f32-GPU vs f64-CPU within the derived bound *on the same
Sobol points*; (b) match `celnet-golden`/closed-form for vanillas and barriers
within the RQMC interval; (c) demonstrate, on real hardware, a **measured
variance-reduction factor ≥ 3** (≥ ~9× effective speedup at fixed error) for a
representative path-dependent exotic vs Philox at equal path count — the headline
QMC claim, measured not asserted; and (d) preserve bit-identical integer streams
CPU↔GPU (KAT).

---

## 6. Greeks on GPU (sequenced with the workloads)

Booking needs risk, not just price. Pathwise and likelihood-ratio (LR) estimators
compute Greeks **in the same path sweep** (no extra dispatch), so they ride
Workload B almost for free:

- **Pathwise** (delta, vega) for Lipschitz payoffs; **LR / Malliavin** for
  discontinuous payoffs (digitals/barriers) where pathwise fails; **mixed
  pathwise-LR** for second order (gamma, vanna, volga). Bumped/finite-difference is
  the fallback and the cross-check (the CPU side already validates Greeks vs FD).
- Reconcile every GPU Greek against the f64 CPU estimator and, where available, the
  `celnet-golden` analytic Greek — same triangulation as price (§3.3).
- This is **deferred behind a working multi-step exotic kernel** (you need paths
  before pathwise Greeks); listed here so the kernel ABI (`Reduction` → a Greeks-
  carrying reduction) is designed to carry them from the start, not retrofitted.

---

## 7. CubeCL vs staying on wgpu+WGSL (revisit, don't auto-adopt)

ARCHITECTURE §4 records the ADR: **wgpu + hand-WGSL over CubeCL**, because the
direct path ships today with a working f32 kernel + f64 oracle and no extra build-
time codegen dependency. This plan **keeps that decision** but flags a concrete
re-evaluation trigger:

- **CubeCL** (`tracel-ai/cubecl`) compiles one `#[cube]` Rust fn to CUDA / HIP /
  Metal / SPIR-V / WGSL / CPU-SIMD with comptime specialization, autotune, and
  autovectorization — exactly the "write the kernel once, target the M4 *and* the
  NVIDIA deploy gate" story. But it is **alpha** (its own docs), pulls a large
  build-time toolchain, and CUDA codegen needs the (free-but-proprietary) CUDA
  toolkit — which under guardrail 7 must be an ADR with the wgpu/Vulkan fallback
  kept first-class (the same posture the architecture already takes).
- **Decision:** stay on wgpu+WGSL for Workloads A/B (one shader, validated on
  Metal + Lavapipe + Vulkan-CUDA). **Re-open the ADR only if** (i) the multi-step
  exotic + Sobol kernels become painful to maintain as duplicated WGSL+CPU, or
  (ii) the deploy-gate measurement shows hand-WGSL leaving large headroom vs a
  CubeCL-CUDA build on real NVIDIA. Until a *measured* reason exists, no new build-
  time GPU codegen dependency enters the tree.

---

## 8. Build-vs-defer task list (each with its validation gate)

Ordered by leverage. Every item is "done" only when its gate is green under
`just check` (or, for hardware-class items, on the stated hardware). **Build now**
= next wave; **Defer** = gated on a measured bottleneck or a prerequisite.

| # | Task | Build / Defer | Validation gate |
|---|---|---|---|
| **G1** | **GPU perf harness skeleton**: persistent `GpuContext` with reused buffers; `celnet-bench` `benches/gpu_batch.rs` (divan) + `src/bin/gpu_load.rs` (HdrHistogram, bounded) + committed `baselines/gpu_*.json` + `gpu_gate` regression binary (slowdown-only, 2× tol). | **DONE** (`gpu_load.rs`/`gpu_gate.rs`/`gpu_batch.rs`) | **Built + gated.** `gpu_load` runs bounded on M4 (Metal) and Lavapipe; emits GPU+CPU throughput + gpu/cpu ratio + p50/p99/p99.9; `gpu_gate` fails on >2× baseline; CI Lavapipe lane runs it as a *completes-within-ceiling* check, not a speedup. Measured M4 dispatch-amortization curve 0.68×@4k → 53×@1M paths. |
| **G2** | **Batch many-pair/many-tenor/many-strike vanilla kernel** (Workload A): structured-buffer instrument inputs, one thread per instrument, one 1-D dispatch per sweep. | **DONE** (`batch.rs`/`batch.wgsl`) | **Built + gated.** Reconciles each instrument vs the f64 A&S-erf oracle within `derived_batch_bound` (pure f32 round-off, derived from `f32::EPSILON`/GK condition numbers) **and** vs the golden-validated `celnet_vanilla::price` within the three-way sum; measured M4: max f32-roundoff rel `3.11e-7` over a 1792-instrument ITM/ATM/OTM × expiry × vol/rate × call-put batch, `is_gpu=true`. Bit-reproducible (no RNG). **Absolute** points/s + the speedup-vs-CPU **headline stays deferred to the NVIDIA deploy gate (G8)**; in-repo proves correctness + (per the plan's honesty rule) only a ratio on this integrated GPU. |
| **G3** | **Multi-step GBM path engine on GPU** (honor `PathSpec::steps`): per-path register state, `normal(path, step, dim)` loop; CPU oracle gains the identical multi-step engine. | **DONE** (W8 — `path.wgsl`/`path.rs`) | **Built + gated.** GPU multi-step path reconciles with the CPU multi-step oracle within the derived bound; three-way GPU-f32 ≈ CPU-f64 ≈ golden (`celnet-parity/tests/gpu_path.rs`); real Metal exercised. |
| **G4** | **Path-dependent exotic payoff kernels** (Workload B): barriers (8 single + double-KO), touch/no-touch/DNT, window barrier, geometric/arithmetic Asian, BGK shift in-kernel. | **Partial / deploy-gated** | The multi-step path + Greeks kernels (G3/G6) are built; the full per-payoff exotic kernel set + the **booking-grade ≤ 1e-3 rel inside ≤ 50 ms** absolute is **deploy-gated to NVIDIA (G8)** (M4 Metal lacks f64 ⇒ in-repo proves correctness + ratios only). |
| **G5** | **Sobol QMC + Brownian-bridge variate source** behind `VariateSource`; scrambled (ART-Owen), committed Joe-Kuo direction numbers; RQMC error from `R` scrambles; CPU + GPU share the bridge. | **DONE** (`celnet-qmc`; consumed on GPU) | **Built + gated.** Integer/direction-number layer bit-identical CPU↔GPU (KAT, `gpu_path.rs`); matches golden within the RQMC interval; **measured variance reduction ≈37.7×/88.5× vs plain MC** (`celnet-parity/tests/qmc.rs`), high-dim RQMC convergence in `qmc_highdim.rs`. |
| **G6** | **GPU Greeks** (pathwise + LR/Malliavin, mixed for 2nd order) carried in the path sweep; widen `Reduction` to carry Greeks. | **DONE** (W8 — `greeks.wgsl`/`pathwise.rs`) | **Built + gated.** Each GPU pathwise/LR Greek reconciles vs the f64 CPU estimator and analytic golden within the derived bound (`celnet-parity/tests/gpu_greeks.rs`); discontinuous-payoff Greeks use LR. |
| **G7** | **End-to-end RFQ-exotic wire proof**: barrier RFQ through real `celnet-server` → engine GPU dispatch → reconcile → respond, timed client-side (GPU analogue of `wire_load`). | **Defer** (needs G4 + engine GPU-dispatch wiring) | ≤ 50 ms p99 *as a counterparty observes it* on the deploy-gate hardware, under concurrent RFS load; bounded, never hangs. |
| **G8** | **CUDA deploy-gate job**: scheduled/pre-release CI on real NVIDIA (wgpu-Vulkan first; cubecl-cuda only if §7 ADR re-opens) producing the production-hardware baseline the GA headline cites. | **Defer** (infra; gates the GA perf headline) | Produces signed baseline JSON for Workloads A/B + exotic ≤ 50 ms; reconciliation green on f32; f64-on-GPU used only as a slow reference. |
| **G9** | **GPU PDE** (tiled-stencil explicit/ADI; PCR tridiagonal for implicit) for dense 2-D LSV grids. | **Defer** (gate on measured CPU-ADI bottleneck) | Only built if a measured book-revaluation shows CPU HV-ADI as the bottleneck; then reconciles vs CPU PDE within derived bound and vs golden; honest finding recorded if 1-D booking grids are *slower* on GPU. |
| **G10** | **CubeCL re-evaluation ADR** (per §7). | **Defer** (gate on G3/G4 maintenance pain *or* G8 headroom) | An ADR (`manage_adr`) with a measured comparison; wgpu/Vulkan stays first-class regardless. |

**Cross-cutting gates on every item:** `cargo fmt` + `clippy -D warnings` +
`nextest` + `cargo-deny` (advisories/licenses/bans — any new dep, e.g. a Sobol
table crate, must pass the OSS license set); no new commercial/proprietary runtime
dep; vendor/method-neutral names (no `philox`/`sobol`/`owen` in *public API*
identifiers — provenance in doc comments only, as `counter_rng.rs` already does);
`#![forbid(unsafe_code)]` preserved; lodestar re-indexes automatically — run `detect_changes` to confirm scope — and
ARCHITECTURE §4 / ROADMAP WS-E updated after each landed item.

---

## 9. Risks & honest limits

1. **The M4 cannot prove the production speedup.** Integrated GPU + unified memory
   + no f64 means the M4 validates *f32 correctness* and gives an *indicative*
   integrated-GPU throughput; the headline speedup and the ≤ 50 ms exotic budget
   are only GA-grade on the **real-NVIDIA deploy gate** (G8). This plan never lets
   an M4 or Lavapipe number stand in for that.
2. **Lavapipe is correctness, not performance.** It is a CPU rasterizer; a "fast"
   Lavapipe number is meaningless. CI gates *functional completion within a
   ceiling*, not speed.
3. **f32 accuracy near barriers** is the sharpest numerical risk: discontinuous
   payoffs amplify f32 error exactly where booking accuracy matters most. Mitigation:
   in-kernel BGK shift, Brownian-bridge max/min refinement (G5), path-level
   reconciliation (§3.3), and the derived bound as a hard tripwire — never a tuned
   constant.
4. **Determinism under QMC scrambling** must be re-proven (G5 KAT) — a non-
   reproducible scramble would break replay/audit (`docs/ARCHITECTURE.md` §8).
5. **Kernel duplication** (WGSL ↔ CPU oracle) is the maintenance cost that could
   eventually justify CubeCL (G10); tracked, not pre-solved.
6. **Dispatch overhead can make small workloads slower on GPU** (esp. GPU PDE on
   1-D grids, G9). The harness must report *where the crossover is* so dispatch is
   only chosen above it — an architectural finding, not just a latency number.

---

## 10. Sequencing summary

**DONE (correctness-reconciled f32↔f64↔golden + host ratios):** G1 (perf harness —
`gpu_load`/`gpu_gate`), G2 (batch closed-form vanilla — `batch.rs`/`batch.wgsl`),
G3 (multi-step paths — `path.wgsl`), G5 (Sobol QMC + bridge — `celnet-qmc`, consumed
on GPU), G6 (GPU pathwise/LR Greeks — `greeks.wgsl`).
**Deferred / deploy-gated:** G4 (full per-payoff exotic kernel set + the ≤ 50 ms
absolute — NVIDIA-gated), G7 (wire-path exotic proof), G8 (NVIDIA deploy gate —
gates the GA absolute perf headline), G9 (GPU PDE), G10 (CubeCL ADR).

The throughline: **measure before claiming** (G1 first), **reconcile every result
against the f64 oracle within a derived bound**, and **stratify the claim by
hardware class** (M4 correctness/indicative, Lavapipe correctness-only, real NVIDIA
for the production headline) — the same honesty discipline `docs/SCALE-OUT.md` §0
and `celnet-bench` already enforce elsewhere.

---

## Sources

External (current as of May 2026):

- Sobol Brownian-bridge generator on GPU, ~3× error reduction ⇒ ~9× speedup at
  fixed error: [HPC-QuantLib — The Sobol Brownian Bridge Generator on a GPU](https://hpcquantlib.wordpress.com/2012/09/23/the-sobol-brownian-bridge-on-a-gpu/),
  [HPC-QuantLib — Quasi Monte-Carlo](https://hpcquantlib.wordpress.com/category/quasi-monte-carlo/).
- QMC option-pricing primer (QMC > MC; Brownian-bridge > standard; PCA > bridge):
  [Leobacher, *A short introduction to quasi-Monte Carlo option pricing*, arXiv:1707.04293](https://arxiv.org/pdf/1707.04293).
- Scrambled-Sobol "supercharged QMC" for exotic option pricing / RQMC error bounds:
  [Hok et al., *The importance of being scrambled: supercharged Quasi Monte Carlo*, arXiv:2210.16548](https://arxiv.org/abs/2210.16548).
- GPU-friendly Owen scrambling (cheap, screen-space/GPU construction):
  [Ahmed & Pharr, *ART-Owen Scrambling*, ACM TOG 2023, doi:10.1145/3618307](https://dl.acm.org/doi/10.1145/3618307).
- GPU Monte-Carlo option-pricing speedup band (~30× European QMC, up to ~134×
  barriers): [GPU option pricing (ResearchGate)](https://www.researchgate.net/publication/301459047_GPU_option_pricing),
  [Efficient Monte-Carlo options pricing on GPUs](https://www.researchgate.net/publication/220362365_Efficient_Monte_Carlo-based_options_pricing_on_graphics_processors_and_its_optimizations).
- CubeCL multi-backend Rust GPU compute (single `#[cube]` → CUDA/HIP/Metal/SPIR-V/
  WGSL/CPU-SIMD; comptime/autotune; alpha): [tracel-ai/cubecl](https://github.com/tracel-ai/cubecl),
  [cubecl-cuda](https://crates.io/crates/cubecl-cuda), [cubecl-wgpu](https://crates.io/crates/cubecl-wgpu).
- Mesa Lavapipe software-Vulkan state for headless CI (Vulkan 1.3 conformant, 1.4
  extensions; CPU rasterizer): [Current state of Lavapipe — Vulkanised 2025 (Igalia)](https://vulkan.org/user/pages/09.events/vulkanised-2025/T5-Lucas-Fryzek-Igalia.pdf),
  [Lavapipe Software Vulkan Driver (Mesa DeepWiki)](https://deepwiki.com/sailfishos-mirror/mesa/3.6.2-lavapipe-software-vulkan-driver).

Internal: `docs/ARCHITECTURE.md` §4 (GPU abstraction), §3 (concurrency/zero-alloc),
§1.2 (latency budgets incl. ≤ 50 ms exotic), §8 (determinism/golden);
`docs/SCALE-OUT.md` §0/§1 (built-vs-designed honesty, off-hot-path discipline);
`crates/celnet-gpu/src/{backend,cpu,gpu,counter_rng}.rs`, `src/shader.wgsl`;
`crates/celnet-bench/benches/README.md` (in-core vs wire-path, `bench_gate`
pattern); `.github/workflows/ci.yml` (`gpu-lavapipe` lane).
