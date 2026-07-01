# ADR-0013 — Acceleration & extensibility wiring

- **Status:** Proposed / Accepted as a **design direction** (2026-07-01). **NOT yet
  implemented.** Records the intended wiring of the built-but-dormant GPU acceleration and
  the closed plugin surface into the live pricing/risk path. The CPU-analytic path remains
  authoritative until each GPU lane passes its ≤1e-12-vs-CPU-oracle gate. Realizes dimension
  **D5** of `docs/ARCHITECTURE-TARGET.md` (§1 "wire the GPU, open the plugin surface",
  §5 P1(b)/P4) and honours ADR-0007 (one unversioned contract), ADR-0008 (carry seam),
  ADR-0012 (unified gBSM kernel), and CLAUDE.md guardrails #2/#6/#7/#8/#9/#10/#11.
- **Aligns with:** the "connect the islands" theme — the biggest architectural win here is
  **integration**, not new capability (`docs/ARCHITECTURE-TARGET.md` §0). Latency embargoes
  live in ADR-0016; the generated-codec / cross-asset-client work lives in ADR-0014; this ADR
  owns acceleration + extensibility + declarative config.

## Context (grounded in the code)

- **Five validated GPU pipelines, zero live callers.** `celnet-gpu` implements, to a high
  standard, five `wgpu`/WGSL pipelines behind the `PricingBackend` trait
  (`crates/celnet-gpu/src/backend.rs:175`, `type Scalar` at `:177` — `f64` on CPU, `f32` on
  GPU):
  - `GpuBackend::dispatch` — Monte-Carlo vanilla over a `PathSpec`/`PayoffKernel`
    (`crates/celnet-gpu/src/gpu.rs:209`, struct `:70`);
  - `BatchPricer::price_batch` — **closed-form** batch across instruments
    (`crates/celnet-gpu/src/batch.rs:210`, struct `:156`; the CPU-parity path
    `cpu_batch_with_as_erf` at `:481`);
  - `ScenarioPricer::price_scenario_batch` — a spot×vol grid in **one dispatch**
    (`crates/celnet-gpu/src/scenario.rs:269`, struct `:209`);
  - `PathwiseGreeksPricer` — Sobol-QMC pathwise LR delta/vega
    (`crates/celnet-gpu/src/pathwise.rs:128`);
  - `MultiStepPathPricer` — Asian / lookback path payoffs (`crates/celnet-gpu/src/path.rs`).

  Metal-f64 is handled correctly (f32 math, **f64 reduction**); the CPU-SIMD fallback is
  first-class and **bit-reproducible** via the counter-based Philox RNG
  (`crates/celnet-gpu/src/counter_rng.rs`). But **every** live path is CPU: `price_instrument`
  (`crates/celnet-server/src/pricer.rs:495`) → `engines::dispatch`
  (`crates/celnet-server/src/pricer/engines.rs:1442`, 25 arms) is CPU-only; the risk-cube
  bump-and-revalue (`crates/celnet-risk-cube/src/nonadditive.rs`, `VarEs` at `:504`) is CPU;
  the streaming Greeks service (`crates/celnet-server/src/services/stream.rs:365` →
  `price_instrument`) is CPU. `trace_path` inbound on every GPU entry point resolves to
  **zero non-test callers**. The GPU is a parallel capability **island**.

- **The code already flags the correct GPU lever — and the numerical trap.**
  `crates/celnet-risk-cube/src/nonadditive.rs:44-56` carries an honest design note: the MC
  `ScenarioPricer` is *deliberately not* wired into the **exact closed-form** vanilla VaR
  path, because "mixing Monte-Carlo estimator noise into an *exact* closed-form reval would
  be a numerical regression"; the appropriate GPU lever for that path is a **batched
  closed-form kernel** (same exact arithmetic dispatched across positions×scenarios —
  `docs/GPU-AT-SCALE-PLAN.md` Workload A/G2). This constrains decision (2) below: the
  repricer backend must be **paradigm-matched** to the node.

- **Plugin dispatch reaches exactly one arm.** The tiered plugin host (native Tier-0
  `register_native` zero-overhead + wasmi Tier-2 fuel-metered sandbox `register_wasm`,
  `StoreLimits`, five NaN-canonical host math fns) is frozen behind the WIT contract
  (`crates/celnet-plugin-api/wit/celnet.wit`): `price` (`:182`), `price-and-greeks` (`:186`),
  `implied-vol` (`:195`), `check-no-arbitrage` (`:201`), `calibrate` (`:211`). But the server
  only consults the `ModelRegistry` on the **Vanilla FX/metal arm**: `engines::dispatch`
  (`engines.rs:1442`) routes `P::Vanilla(v)` through `ctx.plugin_models`
  (`engines.rs:1457-1469`, field declared at `:79`); **every other product arm bypasses
  plugins entirely**, and `calibrate` / `check-no-arbitrage` are defined in the WIT but have
  **no dispatch path** — they are unreachable.

- **Config is env-var proliferation; GPU is absent from it.** Deployment resolves from
  `CELNET_DEPLOY` → `DeployMode` (`crates/celnet-server/src/services/deploy.rs:65`); access
  from `CELNET_ACCESS_MODE` (`crates/celnet-server/src/services/access.rs`, default
  `Enforce`); fleet membership from a commented-out, undocumented `CELNET_FLEET_BACKENDS`.
  There is **no declarative, typed pricing config** — MC path counts, GPU-vs-CPU selection,
  plugin-wasm load lists, and feed wiring are all hardcoded or env-var-driven. GPU is absent
  from the deploy config entirely.

- **The CUDA/CubeCL claim is doc-only.** `docs/ARCHITECTURE.md:347-350` ("ADR — wgpu over
  CubeCL") states "The CUDA backend is kept **first-class** as a Linux/CI target"; `:310-311`
  records considering CubeCL and adopting `wgpu`+WGSL. But **no `cubecl`/CUDA dependency
  exists in any crate `Cargo.toml`** — the implemented backends are `wgpu` (Metal/Vulkan/DX12)
  + CPU-SIMD only. The doc overstates the code: a first-class CUDA target is claimed and not
  delivered.

- **Why this is low blast radius.** ADR-0008's carry seam + ADR-0012's one gBSM kernel already
  route every asset class through one `price_greeks`, so a `GpuInstrument` carrying
  `CostOfCarry` prices a *mixed-asset* portfolio in one dispatch with no new RPC/proto shape
  (`docs/ARCHITECTURE-TARGET.md` §2.2). GPU wiring is additive at the dispatch boundary; it
  does not touch the contract, the hot core, or the leaf math.

## Decision

Wire the acceleration island into the live path and open the plugin surface, along seven
concrete seams. **Design-first** — this ADR fixes the seams and invariants; each lane lands
behind its own ≤1e-12 gate.

1. **GPU into the live path by BATCH SIZE (not by asset class).** `price_instrument` /
   `engines::dispatch` gains a batch-size branch: **small batches (≤~16 instruments) stay on
   the CPU analytic path** (dispatch latency dominates a GPU round-trip; the single-tick hot
   path is *never* a GPU caller — see the invariant), while **large batches route to the
   `celnet-gpu` `BatchPricer` in one dispatch**. The threshold is a tuned constant in the
   declarative config (7), not hardcoded. This is the "coalesced AoS, one command buffer"
   SOTA pattern (§3).

2. **`ScenarioPricer` becomes the `celnet-risk-cube` repricer backend — paradigm-matched.**
   The risk cube's O(Npos×Nspot×Nvol) bump-and-revalue (`nonadditive.rs`) is the
   portfolio-scale throughput bottleneck and the single biggest win. The repricer backend
   becomes GPU-batched — collapsing the per-scenario reprice loop into **one GPU command
   buffer** — but **the GPU pricer is selected by the node's valuation paradigm**, honouring
   the code's own note (`nonadditive.rs:44-56`): a node priced by an **exact closed form** is
   revalued by a **batched closed-form scenario kernel** (`BatchPricer`-family, exact
   arithmetic), while **MC / exotic** nodes use `ScenarioPricer`'s MC grid. MC estimator noise
   is **never** mixed into an exact closed-form reval.

3. **GPU `PathwiseGreeksPricer` for the streaming Greeks *batch/fan-out* service.** The
   Sobol-QMC pathwise LR pipeline backs the streaming-Greeks service where it fans a *batch*
   of subscriptions (`services/stream.rs`), **not** the single-tick per-subscription hot path
   (that stays CPU, invariant below).

4. **A `CrossAssetBatchPricer` over the carry seam.** A `GpuInstrument` carries
   `CostOfCarry` (`b`, `r`) so `BatchPricer`/`ScenarioPricer` price a **mixed-asset**
   portfolio (FX + equity + commodity + crypto-linear) in one dispatch through the unified
   gBSM kernel (ADR-0012) — zero new RPC/proto/message (`Instrument.oneof` is already
   universal).

5. **Open plugin dispatch to EVERY `ProductEngine` arm + a `calibrate` callback.** Route the
   `ModelRegistry` lookup through **all** dispatch arms of `engines::dispatch`, not just
   `P::Vanilla` — every product arm may resolve a user model before the built-in engine, with
   graceful fallback to the built-in form. Wire the **`calibrate`** and **`check-no-arbitrage`**
   WIT functions (`celnet.wit:201`,`:211`) to a real dispatch path so user smile models are
   calibratable and arbitrage-checkable, not merely defined.

6. **A typed declarative `PlatformConfig` (`celnet.toml`).** Replace the env-var
   proliferation with one typed, boot-time config: GPU-vs-CPU selection + the batch-size
   threshold (1), MC path counts, plugin-wasm load list (5), fleet membership (superseding
   the commented `CELNET_FLEET_BACKENDS`), and feed wiring. Env vars remain accepted as
   overrides for ops, but the typed config is the source of truth. No `schema_version`
   (guardrail #9).

7. **Resolve the CUDA/CubeCL doc-vs-code mismatch.** Either **implement** a CUDA backend
   (via CubeCL's single-`#[cube]`-kernel → CUDA/Metal/CPU path, validated in NVIDIA CI/Linux
   containers — recorded as its own ADR with `wgpu`/Vulkan kept first-class per guardrail #7),
   **or drop the "first-class CUDA target" claim** from `docs/ARCHITECTURE.md`. The doc must
   not overstate the code. Recommended near-term: **drop the claim**, keep the `wgpu` +
   CPU-SIMD backends as the delivered set, and reintroduce CUDA only when a measured NVIDIA
   throughput need justifies the dependency.

### Keep — the parts that are already right

- **Metal-f64 strategy:** f32 math + **f64 reduction** — retained unchanged (Metal lacks f64;
  Vulkan/CUDA f64 is 16–64× slower — `docs/ARCHITECTURE.md:316`).
- **CPU-SIMD fallback first-class and bit-reproducible** (Philox counter RNG) — every GPU
  lane keeps a tested CPU-equivalent path; the platform runs correctly with **no GPU present**.
- **The one contract and the carry seam** — no proto/RPC change; acceleration and plugins are
  additive at the dispatch boundary.

## SOTA basis

- **GPU quant batch pricing** — the throughput win is **coalesced access + one command
  buffer**: pack the batch Array-of-Structs contiguously, dispatch one kernel across
  positions×scenarios, reduce on-device (f32 math, f64 reduction on the reduction stage to
  hold precision). This is the standard GPU derivatives-batch pattern and matches the existing
  `BatchPricer`/`ScenarioPricer` shape; the risk-cube O(Npos×Nspot×Nvol) grid collapses to a
  single dispatch (`docs/GPU-AT-SCALE-PLAN.md` Workload A/G2).
- **Batched exact arithmetic ≠ Monte-Carlo grid** — for closed-form nodes the correct GPU
  lever is the same exact closed form dispatched in parallel, not an MC estimator; this is why
  decision (2) is paradigm-matched (the code's own `nonadditive.rs:44-56` rationale).
- **wgpu over CubeCL** — recorded in `docs/ARCHITECTURE.md:347`; retained. CubeCL's
  single-kernel-multi-backend story stays the sanctioned route **if** CUDA is implemented
  (guardrail #7: the proprietary-but-free CUDA toolkit behind an ADR, `wgpu`/Vulkan first-class).

## Consequences

- **GPU wiring is largely a payoff of already-built pipelines** — the work is *connecting* the
  island (batch-size branch, repricer backend swap, config plumbing), not new numerics.
- **Plugin surface becomes uniform** — every product arm can host a user model; `calibrate` /
  `check-no-arbitrage` become reachable capabilities, closing the WIT-defined-but-unreachable
  gap.
- **Config becomes typed and declarative** — one `celnet.toml` replaces scattered env vars;
  GPU/MC/plugin/fleet/feed knobs are discoverable and validated at boot.
- **Docs stop overstating the code** — the CUDA claim is either delivered or removed (DRY /
  guardrail #10; the no-stale-docs rule).

### Invariants (non-negotiable, gated every phase)

- **GPU results validated vs the CPU f64 oracle + the QuantLib golden vectors at ≤1e-12.** No
  GPU lane lands without its ≤1e-12 parity gate against the independent oracle (never a
  self-referential regen — CLAUDE.md #5, ADR-0012 §4). f32 GPU math is validated to converge
  to the f64 reduction within the tolerance.
- **The single-tick streaming hot path stays CPU.** GPU is the **BATCH / portfolio tier only**
  (large-batch pricing, risk-cube reprice, streaming *fan-out* batch). The per-tick pricing
  path is never a GPU caller — a GPU round-trip blows the vanilla p50≤2µs / p99≤10µs budget
  (`docs/ARCHITECTURE.md` §1.2; ADR-0016).
- **No GPU on the pinned pricing thread.** The zero-alloc, lock-free hot core
  (`PricingCore::drain`) never touches `wgpu`; the GPU dispatch lives entirely on the async
  edge / batch tier, off the pinned OS thread (ADR-0016 hot-core embargo).
- **Exact closed-form nodes are never revalued through the MC grid** (decision 2) — no MC
  estimator noise contaminates an exact reval; this is what keeps the ≤1e-12 risk-cube
  invariant honest.
- **Every GPU lane keeps a bit-reproducible CPU-SIMD equivalent** — the platform is fully
  functional with no GPU device (correctness carried by the CPU oracle, determinism by Philox).

## Alternatives rejected

- **Leave GPU as a benchmark-only island** — rejected: the five pipelines are validated and
  the risk-cube is the portfolio-scale bottleneck; keeping them unwired forfeits the single
  biggest throughput win for IB-sized portfolios (guardrail #6) and lets the CPU/GPU parallel
  path-payoff impls drift as a maintenance liability (DRY, guardrail #10).
- **Force GPU on the single-tick hot path** — rejected: a GPU command-buffer round-trip is
  ~orders of magnitude over the vanilla p50≤2µs budget and would put `wgpu` on the pinned
  zero-alloc thread. GPU is the batch tier; the hot path is CPU analytic (the batch-size branch
  exists precisely to keep small/latency-critical work on the CPU).
- **Wire `ScenarioPricer`'s MC grid as the universal repricer** — rejected: it injects MC
  estimator noise into exact closed-form nodes (a numerical regression the code already
  refuses, `nonadditive.rs:44-56`). The repricer must be paradigm-matched (closed-form kernel
  for closed-form nodes, MC grid for MC/exotic nodes).
- **Keep env-var config; add GPU as more env vars** — rejected: extends the proliferation the
  target explicitly removes; a typed declarative config is discoverable, validated, and the
  single source of truth (guardrail #11, #10).
- **Keep the first-class-CUDA doc claim without implementing it** — rejected: the doc must not
  overstate the code (guardrail #10). Implement it behind an ADR **or** drop the claim.

## Supporting verified claims (lodestar knowledge layer)

To author graph-anchored, lifecycle **draft** (proposed direction; promotion to `active`
awaits implementation + a cross-family Stage-2 review):

- **(decision)** — GPU wires into the live path by **batch size**: small (≤~16) → CPU analytic
  `engines::dispatch`; large → `celnet-gpu::BatchPricer` in one dispatch. Anchors:
  `price_instrument`, `engines::dispatch`, `BatchPricer::price_batch`, `PricingBackend`.
- **(decision)** — the `celnet-risk-cube` repricer backend becomes GPU-batched and
  **paradigm-matched**: closed-form nodes → batched closed-form kernel, MC/exotic nodes →
  `ScenarioPricer` MC grid; the biggest portfolio-scale throughput win. Anchors:
  `celnet_risk_cube::nonadditive`, `VarEs`, `ScenarioPricer::price_scenario_batch`,
  `BatchPricer`.
- **(decision)** — plugin dispatch opens to **every** `ProductEngine` arm (not only
  `P::Vanilla`) and the `calibrate` / `check-no-arbitrage` WIT functions gain a reachable
  dispatch path. Anchors: `engines::dispatch`, `ModelRegistry`, `celnet.wit` (`calibrate`,
  `check-no-arbitrage`).
- **(decision)** — a typed declarative `PlatformConfig` (`celnet.toml`) replaces env-var
  proliferation for GPU/MC-paths/plugin-wasm/fleet/feed; env vars remain overrides. Anchors:
  `DeployMode`, `access` mode resolution.
- **(invariant)** — GPU is the BATCH/portfolio tier only; the single-tick streaming hot path
  and the pinned pricing thread stay CPU, never a `wgpu` caller. Anchors: `PricingCore::drain`,
  `price_instrument`, `GpuBackend::dispatch`.
- **(invariant)** — every GPU result is validated vs the CPU f64 oracle + QuantLib golden
  vectors at ≤1e-12, with a bit-reproducible CPU-SIMD (Philox) equivalent kept first-class.
  Anchors: `BatchPricer::cpu_batch_with_as_erf`, `PricingBackend`, `counter_rng`.
- **(decision)** — resolve the CUDA/CubeCL doc-vs-code mismatch: implement via CubeCL behind an
  ADR (wgpu/Vulkan first-class) **or** drop the "first-class CUDA target" claim. Anchor:
  `docs/ARCHITECTURE.md` "ADR — wgpu over CubeCL".
