# Celnet — System Architecture

> Ultra-low-latency, scalable, mission-critical, hot-upgradable FX-options pricing
> platform in Rust. Greenfield (started 2026-05-30). Toolchain pinned to **Rust 1.96.0**,
> **edition 2024**. Primary dev host: Apple M4 / Metal 4 (`aarch64-apple-darwin`); CUDA
> validated in CI/containers on Linux.

This document is the source of truth for the system architecture: guiding principles and
non-functional requirements, the multi-crate Cargo workspace, the concurrency/latency
model, the GPU compute abstraction, the zero-downtime/hot-upgrade strategy, the
user-extensibility SDK/plugin model, the data flow, and the determinism + versioned wire
protocol discipline. It is deliberately concrete and decisive — where the research surfaced
multiple options, this document picks one and states why.

---

## 1. Guiding principles & non-functional requirements

### 1.1 Principles

1. **Tail latency is the product.** We optimize p99/p99.9, not the mean. Every design
   decision is judged by whether it removes a *source of jitter* — allocations, syscalls,
   page faults, cache misses, false sharing, scheduler preemption, GC pauses (we have
   none — Rust), or synchronous logging.
2. **Two-tier split: hot core vs. async edge.** A non-async, busy-polling, core-pinned
   inner loop (market-data ingest, surface rebuild, pricing, Greeks, risk checks) is
   physically separated from a `tokio`-based async edge (gRPC/WebSocket/FIX control plane).
   The tiers are joined by wait-free SPSC ring buffers, never by locks or channels with
   contention.
3. **Determinism is a discipline, not a hope.** Identical inputs produce bit-identical
   prices and Greeks on every supported platform, so results are reproducible, auditable
   (FRTB/IPV), and replayable. This constrains float handling, the math library, and the
   plugin sandbox.
4. **Convention-correctness over speed-correctness.** FX-options analytics are dominated by
   convention errors (delta type, ATM type, broker-vs-smile butterfly), which dwarf model
   error. Conventions are first-class per-(pair, tenor) configuration, never hardcoded
   globals. A fast wrong price is worthless.
5. **Own your IP: extensibility is a first-class feature.** Desks run *their* vol models,
   exotic payoffs and calibrations inside our engine via a versioned SDK — not a vendor's
   black box. This is the central differentiator against SynOption's closed Orion and
   Fenics/kACE's packaged libraries.
6. **Integrate, don't rebuild, market data.** Fenics FXO 2.0 (and others) are *inputs*. We
   normalize their under-documented conventions on ingest and add the value: an
   arbitrage-free, continuously re-strikable surface with transparent interpolation.
7. **No mocks, no placeholders.** Only complete, reference-validated implementations.
   Numerical code is cross-validated against QuantLib and published benchmark prices.
8. **Layer for parallelism.** Crates are sliced so independent agents/sessions own disjoint
   crates with stable interfaces, eliminating merge contention (see §2.4).

### 1.2 Non-functional requirements (targets)

| Dimension | Target | Rationale / mechanism |
|---|---|---|
| **Vanilla price + full Greeks (cached surface), hot path** | p50 ≤ 2 µs, p99 ≤ 10 µs, p99.9 ≤ 25 µs per option | Zero-alloc closed-form Garman-Kohlhagen on a pinned core; no syscalls, no logging on the path. |
| **Surface rebuild on market tick** (single pair, all tenors) | p99 ≤ 150 µs | VV/SSVI recompute over pre-allocated arenas; SIMD slice math. |
| **First-generation exotic (VV) indicative** | p99 ≤ 50 µs | Closed-form BS/GK TV + VV overlay, zero-alloc. |
| **Path-dependent exotic (LSV/MC, GPU)** | interactive ≤ 50 ms for booking-grade accuracy | GPU MC (Philox/Sobol) or PDE; off the hot path, dispatched async. |
| **Streaming quote throughput** | ≥ 1M price updates/s/core sustained | Thread-per-core, lock-free fan-out, no cross-core cache traffic. |
| **Availability** | 99.99% incl. upgrades; **zero dropped connections / zero in-flight order loss on upgrade** | `SO_REUSEPORT` graceful handoff + versioned live-state transfer (§5). |
| **Determinism** | bit-identical prices across Linux/macOS/Windows and across rebuilds | `rust-lang/libm`, no FMA contraction on reproducible paths, fuel-metered Wasm plugins (§8). |
| **Plugin call budget** | per-invocation fuel cap = latency/compute SLA | Wasmtime fuel metering bounds untrusted model runtime deterministically (§6). |
| **Recovery** | warm restart with state handoff ≤ 1 s; cold start ≤ 10 s | Pre-fault + `mlock` + huge pages at boot; shadow-pod pre-warm (§5). |

> All latency NFRs are *measured*, not asserted: `HdrHistogram` in production, `criterion`/
> `divan` micro-benchmarks gated in CI against committed baselines, `iai-callgrind` for
> deterministic instruction-count regression gates.

---

## 2. Workspace layout (multi-crate Cargo workspace)

A **flat virtual-manifest workspace** (matklad/rust-analyzer style): the root `Cargo.toml`
is a virtual manifest (no root package), every crate lives under `crates/<name>/` and is
named identically to its folder (prefixes **not** stripped). Versions and lint policy are
pinned centrally via `[workspace.dependencies]` and `[workspace.lints]`; a single shared
`Cargo.lock` and `target/` dir. `cargo-deny` + `cargo-audit` gate the whole tree.

Crates are **layered by domain function, not by technical tier**. New transports (a new
gRPC service, an admin CLI) are added as *thin new crates*, not by bloating existing ones.

```
celnet/
├── Cargo.toml                 # virtual manifest: [workspace], deps, lints
├── Cargo.lock                 # committed
├── rust-toolchain.toml        # 1.96.0, edition 2024
├── deny.toml  rustfmt.toml  justfile
└── crates/
    ├── core/                  # Layer 0 — pure domain
    │   ├── celer-num          # float policy, ULP/approx compare, libm wiring, SIMD (wide) helpers
    │   ├── celer-core         # FX domain: dates/calendars, conventions, GK/forward math, Greeks — ZERO IO/framework deps
    │   ├── celer-types        # canonical DTOs (option, leg, surface point, quote, Greeks); serde
    │   └── celer-conventions  # per-(pair,tenor) convention registry: delta type, ATM type, premium ccy, cut, day-count, spot lag
    ├── models/                # Layer 1 — pricing models
    │   ├── celer-vanilla      # Garman-Kohlhagen vanilla + strike↔delta root-find (Brent/Newton, premium-adj branch-safe)
    │   ├── celer-vannavolga   # Vanna-Volga engine (Castagna-Mercurio 2nd approx) + survival-prob exotic weighting
    │   ├── celer-sabr         # SABR (Hagan 2002 expansion) + arbitrage-free PDE SABR (Hagan 2014) + shifted/normal
    │   ├── celer-svi          # SVI / SSVI (Gatheral-Jacquier 2014) arbitrage-free surface parameterization
    │   ├── celer-lsv          # Local-Stochastic-Vol: Heston backbone + Dupire leverage, particle-method calibration
    │   ├── celer-exotics      # payoff catalog: barriers/touches/DNT/digitals, window barriers, TARF/accumulator, Asian, lookback, varswap/volswap
    │   └── celer-numerics     # shared numerical engines: PDE (Crank-Nicolson+Rannacher, ADI Craig-Sneyd), MC drivers, Philox/Sobol, Brownian-bridge
    ├── surface/               # Layer 2 — vol surface construction
    │   ├── celer-surface      # canonical surface object: delta-space, broker→smile fly conversion, arbitrage checks (butterfly/calendar/vertical)
    │   └── celer-surface-build# orchestration: pick model (VV/SSVI/SABR), tenor interpolation in total-variance/business-time, event weighting
    ├── marketdata/            # Layer 3 — market-data adapters (ingest only)
    │   ├── celer-md-api       # MarketDataSource trait: stream surfaces/spot/fwd/NDF fixings; resilience contracts
    │   ├── celer-md-fenics    # Fenics FMD FXO 2.0 adapter (ATM/25d&10d RR/BF per tenor, spot/fwd pts/NDF) via FMD API or LSEG redistribution; convention normalization
    │   ├── celer-md-celer     # Celer marketdata-api feed handler (MarketMerchantPriceService WS, reconnect/resync)
    │   └── celer-md-blend     # multi-source blend/validate, staleness decay, divergence flagging
    ├── engine/                # Layer 4 — pricing engine & orchestration (the hot core)
    │   ├── celer-engine       # stateful low-latency core: book state, recompute graph, dispatch hot vs GPU, Greeks/risk
    │   ├── celer-pricer       # model-agnostic pricing facade behind a registry of PricingModel impls
    │   ├── celer-risk         # portfolio Greeks (incl. vanna/volga/charm/speed/zomma/color), vega buckets, scenario/stress, P&L attribution, IPV
    │   └── celer-rt           # latency runtime: core pinning, SPSC rings, arenas, seqlock/arc-swap publish, allocator wiring
    ├── gpu/                   # Layer 5 — GPU compute abstraction
    │   ├── celer-compute      # PricingBackend trait (simulate_paths / reduce_payoff / solve_pde); scalar-generic
    │   ├── celer-compute-cubecl # CubeCL backend (CUDA/Metal/Vulkan/WGSL/CPU-SIMD) — primary
    │   └── celer-compute-cpu  # rayon + `wide` f64 CPU fallback & reconciliation oracle
    ├── sdk/                   # Layer 6 — user-extensibility SDK / plugin host
    │   ├── celer-plugin-api   # the SDK: PricingModel/Payoff/Calibration traits + WIT world; semver-versioned product
    │   ├── celer-plugin-host  # wasmtime 45 embedding: Linker capabilities, fuel metering, deterministic config
    │   └── celer-plugin-guest # guest-side bindings + helpers for authoring Wasm plugins in Rust
    ├── transport/             # Layer 7 — service & transport
    │   ├── celer-proto        # versioned wire protocol (prost 0.14): message headers + schema-version gating
    │   ├── celer-server       # tokio async edge: tonic gRPC + WebSocket RFS streaming; control plane
    │   ├── celer-fix          # hand-rolled zero-copy FIX dialect (Celer destination / venue wire)
    │   └── celer-ipc          # rkyv 0.8 zero-copy shared-memory / UDS for in-host and upgrade handoff
    ├── integration/           # Layer 8 — Celer estate adapters
    │   ├── celer-distributor  # bridge to Celer in-proc disruptor distributor (JVM adapter or DistributorProducerChannelHandler socket)
    │   ├── celer-staticdata   # staticdata / celertech-type product-type & netting-key mapping for FX option product
    │   └── celer-lifecycle    # OMS/EMS booking, exercise/expiry/fixing/barrier lifecycle, STP execution reports
    ├── tools/                 # Layer 9 — CLI & operator tools
    │   ├── celer-cli          # operator/quant CLI: price, calibrate, dump surface, replay, stress
    │   └── celer-goldgen      # frozen QuantLib reference-table generator (version-pinned oracle)
    └── testkit/               # Layer 10 — test & bench harness (shared)
        ├── celer-testkit      # invariant assertions (put-call parity, monotonicity, no-arb), proptest strategies, fixture loaders
        └── celer-bench        # criterion/divan suites + committed baselines + iai-callgrind gates
```

### 2.1 Why this split aids performance

- **`celer-core` is dependency-free and IO-free.** The hottest math (GK, forward, Greeks,
  date logic) compiles fast, has no framework/runtime in scope, and so cannot accidentally
  allocate, lock, or call a runtime. Determinism and zero-alloc are *structurally*
  enforceable here.
- **The hot core (`engine/`) never depends on the async edge (`transport/`).** The
  dependency arrow points one way: `transport` → `engine` → `models`/`surface` → `core`.
  Nothing on the pricing path can reach `tokio`, `tonic`, or a socket. `celer-rt` owns the
  runtime primitives (pinning, SPSC rings, arenas, `seqlock`/`arc-swap` publication) so the
  latency discipline lives in one auditable crate.
- **GPU is behind a trait (`celer-compute`) with swappable backends.** The engine depends on
  the abstraction, not on CubeCL/CUDA, so the heavy/optional GPU stack never bloats the hot
  core's compile or binary and can degrade to CPU at runtime.
- **The plugin host is isolated.** `celer-plugin-host` (wasmtime) is a leaf the engine calls
  through `celer-plugin-api`; the Wasm runtime's weight and security surface never touch the
  native hot path for first-party models.
- **`celer-proto` centralizes the wire format,** so schema evolution and N/N-1 compatibility
  are gated in one place (§8) rather than smeared across services.

### 2.2 Why this split aids parallel multi-agent maintenance

- **Disjoint ownership, stable seams.** Interface crates (`celer-types`, `celer-core`,
  `celer-conventions`, `celer-compute`, `celer-plugin-api`, `celer-proto`) are stabilized
  *first* and changed only with coordination. Every other crate is a leaf an agent can own
  end-to-end: e.g. one session owns `celer-svi`, another `celer-lsv`, another
  `celer-md-fenics` — they never touch the same files, so there are no merge conflicts.
- **Compile-time fences match team fences.** Because layers only depend downward, a change
  in `celer-md-fenics` can't ripple into `celer-vanilla`; CI rebuilds only the affected
  subtree, keeping the multi-agent feedback loop short.
- **Test ownership is local.** `celer-testkit` provides shared invariants/strategies so each
  model crate carries its own property + golden tests without duplicating harness code.

### 2.3 Workspace hygiene

- `[workspace.dependencies]` pins every external version once; `[workspace.lints]` sets
  `deny(warnings)` for the mission-critical tree. One committed `Cargo.lock`.
- `cargo-deny` (advisories + licenses + bans + sources — it subsumes `cargo-audit`) and
  `cargo-vet` (explicit dependency certification) gate every PR. MSRV declared in
  `rust-version` and verified in a dedicated CI job; edition 2024 enables the MSRV-aware
  resolver.

---

## 3. Concurrency & latency model

### 3.1 Two-tier architecture

```
                 async edge (tokio 1.4x)                       hot core (no async)
   ┌───────────────────────────────────────┐        ┌──────────────────────────────────────┐
   │ tonic gRPC · WebSocket RFS · FIX I/O   │        │ pinned core 0: MD ingest + surface     │
   │ control plane · auth · admin           │        │ pinned core 1: pricer + Greeks         │
   │ (Send + Sync ecosystem)                │ rtrb   │ pinned core 2: risk/stress             │
   │                                        │ SPSC   │ pinned core 3: GPU dispatch + telemetry│
   │   submit req ─────────────────────────►│ rings ►│  busy-poll loops, zero-alloc           │
   │   ◄──────────────── publish results    │◄───────│  lock-free publish (seqlock/arc-swap)  │
   └───────────────────────────────────────┘        └──────────────────────────────────────┘
```

- **Async edge = `tokio` (latest 1.4x line)** with `tonic ~0.13` + `prost ~0.14`, `hyper`,
  WebSocket. We *default to tokio* for ecosystem maturity and the `Send + Sync` middleware
  surface. We deliberately **do not** adopt `monoio`/`glommio`: their io_uring throughput
  win does not justify losing the tonic/axum ecosystem, and io_uring carries a kernel
  security-hardening risk (some prod environments disable it).
- **Hot core = non-async busy-polling OS threads, each pinned to an isolated core** via
  `core_affinity`, with kernel `isolcpus` + `nohz_full` + `rcu_nocbs` on those cores. Async
  is reserved for the IO edge only.

### 3.2 Lock-free hot path & publication

- **Tier-to-tier and core-to-core hops use wait-free SPSC ring buffers** (`rtrb`, with
  `CachePadded` heads/tails; `ringbuffer-spsc` power-of-two/bitmask variant where modulo
  must be avoided). **Never** unbounded channels on the hot hops.
- **Read-mostly state publication** uses the right primitive per access pattern:
  - `arc-swap` — atomically swappable whole-struct read-mostly config (convention registry,
    routing tables, the live model set) for lock-free reads and hot reload.
  - `seqlock` — a single writer publishing small `Copy`/POD snapshots (a quote, a top-of-
    surface point) to many readers with no reader-side contention (readers retry on torn
    reads).
  - `left_right` — larger read-mostly maps where whole-struct swap is too coarse.
  - `crossbeam` `ArrayQueue` (bounded MPMC) and `crossbeam-epoch` only where MPMC is truly
    required.
- **Every shared hot atomic/field is wrapped in `CachePadded`** (`crossbeam-utils`) to kill
  false sharing — the single highest-leverage SPSC optimization. We pad to 128 bytes on
  Apple Silicon / adjacent-line-prefetcher x86.

### 3.3 Zero-allocation & determinism on the path

- **Pre-allocate all buffers/object pools at startup.** On the hot path: never allocate,
  never syscall, never lock, never log synchronously. `arrayvec`/`smallvec`/`heapless` and
  fixed-size types throughout; an allocation-counting guard (and `dhat` in CI) asserts zero
  allocations in the hot loop.
- **Telemetry/logging is offloaded** over a bounded queue to a non-critical core — a
  synchronous log line or lock-based metric on the path is a stealth tail-latency killer.
- **Global allocator** is set explicitly: `mimalloc` for allocation-heavy parsing/ingest
  paths (best small-alloc p99), `tikv-jemallocator` for the throughput-oriented async edge.
  Benchmarked against the real workload — the steady-state gap is small once pre-allocated.

### 3.4 SIMD & math

- **SIMD via the `wide` crate on stable Rust** for slice math (surface arrays, FIX field
  scanning, batch payoff). `std::simd` (portable_simd) is still nightly-only as of May 2026
  and we do not pin nightly for the core; `std::arch` intrinsics are the escape hatch for a
  profiled hotspot.
- **Host tuning** per Rigtorp's low-latency guide: disable deep C-states, fix CPU
  frequency/turbo, IRQ affinity off isolated cores, huge pages + `mlock` + pre-fault at boot
  (explicit `hugetlbfs`, not THP, to avoid `khugepaged` compaction stalls), NUMA-local
  allocation pinned to the same node as the thread.

---

## 4. GPU compute abstraction & CPU fallback

Exotics (LSV Monte Carlo, finite-difference PDE) are dispatched **off the hot path** to a
pluggable compute backend. The abstraction is a Rust trait, scalar-generic over the numeric
type, selected at runtime by probing for an adapter.

```rust
// crates/gpu/celer-compute
pub trait PricingBackend {
    fn simulate_paths(&self, spec: &PathSpec, variates: VariateSource) -> PathBuffer;
    fn reduce_payoff(&self, paths: &PathBuffer, payoff: &PayoffKernel) -> Reduction;
    fn solve_pde(&self, grid: &PdeGrid, scheme: PdeScheme) -> PdeSolution;
}
```

- **Primary backend: CubeCL (v0.10.x)** — one `#[cube]` Rust kernel compiles on demand to
  **CUDA** (native speed on NVIDIA), **Metal/Vulkan/WGSL** (via `wgpu`), and a **CPU-SIMD
  runtime** as the automatic no-GPU fallback. Kernels are real, type/borrow-checked,
  CPU-unit-testable Rust; autotune selects kernels at runtime, `comptime` specializes the
  IR. This single-codebase, multi-backend property is exactly what satisfies
  macOS-Metal + Windows/Linux-CUDA + container deployment.
- **Numeric policy: f32 on GPU, f64 on CPU.** Metal and WebGPU/WGSL have **no native f64**
  (WGSL scalars are only `f32`/`f16`/`i32`/`u32`; atomics are integer-only); even where
  Vulkan/CUDA support f64 it is 16–64× slower than f32. So we **standardize GPU pricing on
  f32** and run a periodic **f64 CPU reconciliation** (`celer-compute-cpu`: `rayon` +
  `wide`) as the validation oracle to bound numerical error for FX exotics. Payoff
  accumulation uses integer-atomic / tree-reduction (no float atomics on WGSL).
- **RNG: counter-based Philox-4×32-10** implemented as a `#[cube]` kernel, each variate
  seeded from `(global_seed, path_index, time_step, dimension)`. Stateless, embarrassingly
  parallel, and **bit-stable across backends** — naive LCG/per-thread RNGs are rejected.
  Box-Muller / inverse-CDF for normals.
- **QMC: Sobol** with host-precomputed direction numbers uploaded as a buffer; dimensions
  assigned to time steps via a **Brownian bridge** for path-dependent exotics. Philox (MC)
  and Sobol (QMC) are swappable variate sources behind one kernel interface.
- **PDE on GPU:** explicit/ADI as tiled stencil kernels using workgroup shared memory;
  implicit/Crank-Nicolson via cyclic-reduction / PCR tridiagonal solvers. Validated against
  the MC engine on vanillas.
- **Deployment:** Linux containers ship the **NVIDIA Container Toolkit** (CUDA/Vulkan in
  container) and bundle **Mesa Lavapipe** so GPU-less CI / headless nodes exercise the same
  `wgpu`/Vulkan path on a software device. NVIDIA driver/toolkit versions are pinned (the
  550→570 class container-Vulkan regression is a known hazard) with GPU health checks.
- **Rejected for the core today:** `rust-gpu`, `Rust-CUDA`, `cuda-oxide` (promising but
  nightly/experimental, non-integrated codegen); `candle` (tensor/ML-shaped, awkward for
  path-dependent state machines + barrier/autocall logic).

---

## 5. Zero-downtime / hot-upgrade strategy

Goal: upgrade a *stateful* low-latency engine with **zero dropped connections and zero
in-flight order/quote loss**. Kubernetes StatefulSet rolling updates do **not** give this
out of the box — we implement app-level handoff.

### 5.1 Routine in-place upgrade (non-breaking schema)

1. **`SO_REUSEPORT` socket handoff.** The new process binds the same port; the old process
   stops accepting new connections and **drains** in-flight work. eBPF `SO_REUSEPORT`
   steering (`BPF_PROG_TYPE_SK_REUSEPORT`) routes *new* connections only to the warm new
   instance. A `/readyz` + connection-drain phase gates the cutover.
2. **Live state handoff** of book state, open RFQ/quote sessions and in-flight orders over a
   Unix-domain socket / shared memory using **`rkyv` zero-copy** framing carried in
   **`celer-proto`** (versioned). `SO_REUSEPORT` alone does *not* migrate session state —
   the explicit, versioned transfer protocol is what makes the handoff safe.
3. **Hot model swap.** Pricing models (first-party native via the registry, and user Wasm
   modules) are swapped behind `arc-swap` with **no maintenance window** — a new model set
   is published atomically and lock-free readers pick it up on their next read. This is the
   direct answer to Murex-class multi-year/MXTEST upgrade pain called out in the competitive
   research.

### 5.2 Risky / schema-breaking upgrade

- **Blue-green at the orchestration layer** for schema-breaking releases, with **SHADOW-
  style shadow-instance state pre-warming** (per the 2026 SHADOW paper) for lowest-downtime
  stateful migration — a shadow is created on the target while the source still serves,
  cutting the restore phase substantially with zero message loss.

### 5.3 What makes it safe

- Versioned wire protocol (§8) with hard **N / N-1 compatibility** CI gates lets old and new
  process versions interoperate during the handoff window.
- Memory is pre-faulted + `mlock`ed + huge-paged at boot so the new instance is warm before
  cutover (no first-touch page faults on the path).

---

## 6. User-extensibility SDK / plugin model

Desks must add/override **vol models, exotic payoffs, calibrations and stress logic** and
run them *in-engine* — keeping their proprietary IP, not consuming a vendor's models.

### 6.1 Chosen approach: WebAssembly first, native only for trusted first-party

| Path | Use | Mechanism | Why |
|---|---|---|---|
| **Wasm (primary, default)** | *Any* user/third-party model or workflow | `wasmtime 45.0.0` + Component Model + WASI 0.2.x; contract defined as a **WIT world** in `celer-plugin-api` | Only Wasm gives a true **security sandbox** *and* **determinism**. Untrusted code cannot corrupt memory, crash the engine, or stall the hot path. |
| **Native trait registry (first-party, compiled-in)** | Hot first-party models shipped in the binary | `inventory` of `dyn PricingModel`, same trait shape the Wasm host implements | Fastest path, no ABI/sandbox concerns; first-party and user plugins are interchangeable behind one registry. |
| **Native dynamic `.so`/`.dylib` (trusted partners only)** | Trusted partner C++/Rust models needing full native speed | **`stabby` 72.1.x** (not `abi_stable`), code-signed + trusted-publisher allowlist | `abi_stable` is effectively unmaintained (last release Oct 2023). Native plugins have **no sandbox and no determinism** — never loaded untrusted. |

### 6.2 Determinism for plugin execution (Wasm)

- **Fuel metering** (`Config::consume_fuel(true)`), not epochs — fuel is fully deterministic
  (same input + same fuel ⇒ interrupt at the same instruction). A **per-call fuel budget is
  the plugin's latency/compute SLA**.
- **NaN canonicalization / pinned float behavior**, and **no non-deterministic host
  imports** — no wall-clock, no RNG (unless explicitly seeded), no threads — locked down at
  the `Linker`. Capability-based security: the host decides exactly which market-data /
  pricing primitives a plugin may touch.
- **Hot-path caveat:** Wasm fuel metering has meaningful overhead vs native; ultra-hot
  per-tick models compile natively via the trait registry, and Wasm is reserved for
  user-supplied / less-hot-path models. The same `PricingModel` trait shape unifies both.

### 6.3 The SDK as a product

- `celer-plugin-api` is **semver-versioned**; guest bindings are generated from the WIT
  world (`celer-plugin-guest`). A **deterministic test harness** replays fixed market-data
  snapshots through the fuel-metered sandbox and asserts **bit-identical** pricing output,
  so user models are reproducible and auditable.
- We track WASI P3 / component-model `struct`/`map` support but build today on **stable WASI
  0.2.x** (map/struct WIT support in wasmtime 45 is still experimental) to avoid depending
  on experimental WIT features in production.

---

## 7. Data flow

End-to-end: external market data → normalized surface → pricing → Greeks/risk →
distribution to the Celer estate and front end. The **hot core** (busy-polling, pinned) is
boxed; the async edge sits at the boundaries.

```
  EXTERNAL MARKET DATA                         Celnet HOT CORE (pinned, lock-free, zero-alloc)
  ───────────────────                          ────────────────────────────────────────────────────
 ┌────────────────────┐
 │ Fenics FMD FXO 2.0 │  ATM / 25d&10d RR / 25d&10d BF per tenor, spot, fwd pts, NDF fixings
 │ (API or LSEG feed) │──┐    via celer-md-fenics  (normalize Fenics conventions → canonical)
 └────────────────────┘  │
 ┌────────────────────┐  │   ┌───────────────────┐      ┌──────────────────────────┐
 │ Celer marketdata-  │  ├──►│  celer-md-blend   │ rtrb │   celer-surface(-build)  │
 │ api (WS, spot/vol) │──┘   │ blend · staleness │ SPSC │ broker→smile fly convert │
 │ MarketMerchantPrice│      │ decay · divergence│─────►│ VV / SSVI / SABR fit     │
 └────────────────────┘      │ flag              │      │ delta-space, arb checks  │
        (async edge)         └───────────────────┘      │ (butterfly/calendar/vert)│
                                                         └────────────┬─────────────┘
                                                          arc-swap publish (atomic surface)
                                                                       ▼
   convention registry ─────► ┌──────────────────────────────────────────────────────────┐
   (celer-conventions,        │                    celer-engine / celer-pricer            │
    arc-swap)                 │  PricingModel registry: native (inventory) + Wasm (fuel)  │
                              │  ┌───────────────┐   ┌──────────────────┐   GPU dispatch  │
                              │  │ celer-vanilla │   │ celer-vannavolga │   ┌───────────┐ │
   plugin host ──────────────►│  │ GK + Greeks   │   │ first-gen exotic │──►│celer-     │ │
   (celer-plugin-host,        │  └───────────────┘   └──────────────────┘   │compute    │ │
    wasmtime, sandboxed)      │  ┌───────────────────────────────────────┐  │(CubeCL):  │ │
                              │  │ celer-lsv / celer-exotics (path-dep.)  │─►│CUDA/Metal │ │
                              │  └───────────────────────────────────────┘  │/WGSL/CPU  │ │
                              └───────────────────────────┬─────────────────└───────────┘─┘
                                       seqlock publish     ▼
                              ┌──────────────────────────────────────────┐
                              │             celer-risk                    │
                              │ portfolio Greeks (Δ,Γ,vega,θ, ρ_d, ρ_f,   │
                              │ vanna,volga,charm,speed,zomma,color),     │
                              │ vega buckets · scenario/stress · PnL · IPV│
                              └───────────────────────┬───────────────────┘
                                     rtrb SPSC         ▼
  ASYNC EDGE (tokio)          ┌──────────────────────────────────────────┐
  ───────────────            │   celer-server (tonic gRPC + WS RFS)       │──►  Celer front end
                              │   celer-fix (FIX dialect)                  │──►  venues / destination
                              │   celer-distributor (in-proc disruptor)    │──►  Celer OMS/EMS/risk
                              │   celer-lifecycle (booking, STP, fixings)  │──►  positionmanager / clearing
                              └────────────────────────────────────────────┘
```

**Notes on the Celer integration edge (from the integration research):**
- The Celer distributor is an **in-process disruptor mailbox in the JVM** with
  *skip-while-full* back-pressure — a high-frequency option pricer can cause silent price
  drops, so `celer-distributor` sizes mailboxes and rate-limits, and a Rust process joins via
  a **JVM adapter or the `DistributorProducerChannelHandler` socket protocol** (decision
  deferred but scoped in `celer-distributor`).
- Adding an FX-**option** product type is a broad change touching `celertech-type`,
  staticdata, every API proto enum, positionmanager netting keys, risk exposure models, and
  the destination FIX dialect — concentrated in `celer-staticdata` + `celer-proto` +
  `celer-fix` so the blast radius is contained.
- `MarketMerchantPriceService` is WS-only (no fallback, ~6 concurrent HTTP connections per
  domain) — `celer-md-celer` is built resilient to WS disconnects with resync.

---

## 8. Determinism & versioned wire protocols

### 8.1 Determinism

- **Math library:** prefer **`rust-lang/libm`** (correctly-rounded, verified ≤ 1.0 ULP vs
  MPFR) over the system libm so transcendental results are **identical across OS/arch and
  across libm updates** — correct rounding yields exactly one answer.
- **Float policy (in `celer-num`):** IEEE-754; **forbid FMA contraction and fast-math on
  reproducibility-critical paths** (FP addition is non-associative, so reduction/parallel
  order changes low bits). Centralize all float comparison in one `assert_close` /
  ULP+relative+absolute helper (`approx`-style); **never** compare with `==` or assert on
  **NaN bit patterns** (NaN payload generation is non-deterministic). Guard against NaN/Inf
  propagation at pricing input and output.
- **Toolchain pinned** via `rust-toolchain.toml` (1.96.0, single target per artifact).
- **Plugins** are deterministic by construction via Wasm fuel + NaN canonicalization + no
  non-deterministic host imports (§6.2).
- **Reproducibility of bugs:** `proptest-regressions/` and fuzz corpora are **committed**, so
  failing seeds replay deterministically in CI and on every developer machine.

### 8.2 Validation layered on determinism

- **Golden / reference oracle:** `celer-goldgen` produces a frozen, version-pinned table of
  **QuantLib** prices + Greeks across a parameter grid (committed); the Rust engine must
  match within documented ULP/relative/absolute tolerances. Re-generated only under explicit
  review, tracking the QuantLib version (treating QuantLib as ground truth without pinning is
  a known risk).
- **Financial invariants as property tests** (`celer-testkit`, `proptest` — chosen over
  quickcheck for Strategy-based generation + superior shrinking): put-call parity, price
  monotonicity in spot/vol/maturity, **convexity in strike (butterfly ≥ 0)**, **non-negative
  risk-neutral density**, **no calendar-spread arbitrage** (non-crossing total variance),
  Greeks-vs-finite-difference consistency, intrinsic-value lower bounds. SVI/SSVI sufficient
  no-arbitrage conditions are golden-model targets.
- **Fuzzing** (`cargo-fuzz`/libFuzzer, Linux-nightly job only — not in the cross-platform
  matrix) with `Arbitrary` mapping bytes → valid-but-adversarial market inputs, for
  serialization, calibration solvers and any unsafe SIMD.
- **Snapshots** (`insta`, `CI=1` so stale snapshots **fail**) for serialized surfaces and
  risk reports; **mutation testing** (`cargo-mutants`, incremental on PR diffs, full on a
  schedule); **coverage** (`cargo-llvm-cov nextest`, fail-under on core modules, Linux only —
  Windows coverage targets are known-broken).

### 8.3 Versioned wire protocol

- **`celer-proto` (prost 0.14.x)** carries a **protocol/schema version in every message
  header**. Strict Protobuf-evolution discipline: **never reuse/renumber field tags, reserve
  removed numbers, add fields as optional, avoid lossy `oneof` edits.** Protobuf Editions
  (`minimum_required_edition`) gate incompatible descriptors.
- **N / N-1 compatibility is a hard CI gate** — this is precisely what makes the
  old↔new-process **live-state handoff** (§5) and rolling/blue-green upgrades safe. Schema
  mistakes (renumbering, lossy `oneof`, remove-without-reserve) silently corrupt state during
  mixed-version windows, so they are caught in CI, not production.
- **Two internal framings:** **`rkyv` 0.8** zero-copy for in-host IPC, shared-memory and
  upgrade-handoff (access archived bytes directly, `no_std`/`no_alloc` capable); **`tonic` +
  `prost`** for external service RPC; **hand-rolled zero-copy FIX** (`celer-fix`) for the
  venue/Celer-destination wire, with SIMD (`wide`) field scanning.

---

## Appendix A — Key version pins (as of May 2026)

| Component | Version / choice |
|---|---|
| Rust toolchain | 1.96.0, edition 2024 |
| Async runtime | tokio 1.4x; tonic ~0.13; prost ~0.14 |
| Allocators | mimalloc (ingest) / tikv-jemallocator (edge) |
| Zero-copy IPC | rkyv 0.8 |
| GPU | CubeCL v0.10.x (CUDA/Metal/Vulkan/WGSL/CPU); wgpu underneath; Mesa Lavapipe for CI |
| Plugin runtime | wasmtime 45.0.0, Component Model, WASI 0.2.x, fuel metering |
| Native plugin ABI (trusted only) | stabby 72.1.x (NOT abi_stable) |
| SIMD (stable) | `wide` crate (+ rayon); `std::simd` nightly-only, not used in core |
| Math | rust-lang/libm (correctly rounded) |
| Test/CI | proptest, cargo-fuzz, insta, criterion/divan, iai-callgrind, cargo-llvm-cov, cargo-mutants, cargo-deny (+cargo-vet), cargo-nextest |
| Reference oracle | QuantLib (version-pinned, frozen golden tables) |

## Appendix B — FX-options analytics anchored in the workspace

The analytics standard (see `docs/ANALYTICS-SPEC.md`) maps onto crates as follows:
- **Garman-Kohlhagen** priced off the forward `F = S·e^{(r_d−r_f)T}` storing two discount
  factors `(DF_d, DF_f)` separately (clean deliverable vs NDO and dual-curve) → `celer-core`,
  `celer-vanilla`.
- **Four delta conventions** (spot/forward × premium-adjusted/unadjusted) and **ATM types**
  (ATMF vs delta-neutral-straddle, premium in/out) as per-(pair, tenor) config →
  `celer-conventions`; branch-safe strike↔delta root-find (premium-adjusted delta is
  non-monotone) → `celer-vanilla`.
- **Smile from ATM + 25d/10d RR + BF**, with explicit **broker(market) → smile strangle**
  calibration → `celer-surface`.
- **Vanna-Volga** (Castagna-Mercurio) with survival-probability weighting for first-gen
  exotics → `celer-vannavolga`; **SVI/SSVI**, **SABR / arb-free PDE SABR** →
  `celer-svi`/`celer-sabr`; **LSV** (Heston + Dupire leverage, particle method) →
  `celer-lsv`.
- **Two rhos (ρ_d, ρ_f)** and the full higher-order Greek set (vanna, volga, charm, speed,
  zomma, color) + bucketed vega → `celer-risk`.
- **Calendar/date engine** (intersect both currency calendars + USD; spot/delivery by
  identical lag; modified-following + EOM; NY 10am / Tokyo 3pm cut), and **NDO/NDF cash
  settlement** at named fixings (EMTA/WMR) → `celer-core`, `celer-conventions`.
- **Numerical engines** — PDE (Crank-Nicolson + Rannacher, ADI Craig-Sneyd / Hundsdorfer-
  Verwer), MC (Andersen QE, Sobol + Brownian-bridge, Broadie-Glasserman-Kou 0.5826·σ·√dt
  barrier shift) → `celer-numerics`; variance-swap log-contract replication + volatility-swap
  convexity adjustment → `celer-exotics`.
