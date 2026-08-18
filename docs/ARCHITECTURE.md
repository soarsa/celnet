# Celnet — System Architecture

> **Currency:** the §2 crate tree was re-verified against `ls crates` on **2026-08-18** and
> enumerates all **55** crates. Prose outside §2 predates that sweep — treat an undated claim
> as unverified, and see [`README.md`](README.md) for how this document relates to
> `ARCHITECTURE-TARGET.md` (the intended state) and `ARCHITECTURE-DETERMINATION.md` (how that
> target was chosen). This one is the **as-built** record.

> Ultra-low-latency, scalable, mission-critical, hot-upgradable FX-options pricing
> platform in Rust. Greenfield (started 2026-05-30). Toolchain pinned to **Rust 1.96.0**,
> **edition 2024**. Primary dev host: Apple M4 / Metal 4 (`aarch64-apple-darwin`); CUDA
> validated in CI/containers on Linux.

> **Binding-rules override (2026-05-30):** where this document predates the current product
> rules it is superseded by `CLAUDE.md` guardrails: crate prefix is **`celnet-`** (not
> `celer-`); API identifiers are vendor/research-neutral and purpose-named; **there are no
> versioned APIs** — the wire contract is single-and-current and zero-downtime upgrades use
> **blue-green / full cutover** (no `schema_version` / N–N-1). Passages below mentioning a
> "versioned wire protocol" or `celer-*` crates are scrubbed as those areas are built.

This document is the source of truth for the system architecture: guiding principles and
non-functional requirements, the multi-crate Cargo workspace, the concurrency/latency
model, the GPU compute abstraction, the zero-downtime/hot-upgrade strategy, the
user-extensibility SDK/plugin model, the data flow, and the determinism + single-current-wire-contract
discipline. It is deliberately concrete and decisive — where the research surfaced
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
| **Plugin call budget** | per-invocation fuel cap = latency/compute SLA | wasmi fuel metering bounds untrusted model runtime deterministically (§6). |
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

The **implemented** workspace is **55 crates** (flat under `crates/`; verified by
`ls crates | wc -l`). The early design sketched a finer split that was then partly
**consolidated** into cohesive crates (so the early provenance-named draft crates do not
exist), while the scale-out, risk-hierarchy, durability, cross-asset/linear, RFQ-to-many,
fixed-income/rates and verification waves added new disjoint leaf crates. This block
reflects the real tree (`ls crates`):

```
celnet/
├── Cargo.toml                 # virtual manifest: [workspace], deps, lints
├── Cargo.lock                 # committed
├── rust-toolchain.toml        # 1.96.0, edition 2024
├── deny.toml  rustfmt.toml  justfile
└── crates/
    # ── Layer 0: shared interface / pure domain ──
    ├── celnet-types           # POD DTOs + convention enums (Delta/ATM/Premium/Cut/DayCount/Settlement); serde; ZERO IO
    ├── celnet-core            # math (libm-routed exp/ln/sqrt, norm_cdf/pdf), is_close/assert_close, Smile trait; ZERO IO
    ├── celnet-proto           # single current wire contract (prost 0.13 / tonic 0.12); NO version field
    ├── celnet-plugin-api      # SDK: PricingModel/PricingBackend traits + WIT world
    # ── Layer 1: conventions, calendar, vanilla (FX + cross-asset + linear) ──
    ├── celnet-conventions     # per-(pair,tenor) convention registry: delta/ATM/premium/cut/day-count/spot-lag
    ├── celnet-calendar        # holiday calendars, spot lag, delivery, modified-following, EOM
    ├── celnet-vanilla         # Garman-Kohlhagen + full 13-Greek set + branch-safe strike↔delta solver
    ├── celnet-equity-vanilla  # generalized-BSM vanilla on the agnostic carry seam (dividend-yield carry); over celnet-core/-types only
    ├── celnet-commodity-vanilla # Black-76 / undiscounted-forward vanilla on the carry seam; over celnet-core/-types only
    ├── celnet-crypto-vanilla  # linear funding-carry vanilla + inverse coin-margined 1/S_T payoff on the carry seam; over celnet-core/-types only
    ├── celnet-linear          # linear (non-option) FX/metal book leaf: outright forward / FX swap / NDF over the carry seam; over celnet-core/-types only
    # ── Layer 2: surface (VV/SABR/SVI/SSVI/eSSVI consolidated here) ──
    ├── celnet-surface         # delta-space smile, broker→smile fly calibration, VV/SABR/SVI/SSVI/eSSVI, Dupire local vol,
    │                          #   arbitrage gates (butterfly/calendar/vertical), term-structure interpolation
    # ── Layer 3: exotics + standalone models + QMC ──
    ├── celnet-exotics         # digitals/touches/DNT/barriers + double-KO + window barrier; VV overlay; CN+Rannacher PDE;
    │                          #   Philox MC + BGK + control variates; geo/arith Asian; var/vol swap; forward-start/cliquet;
    │                          #   TARF/accumulator/quanto/lookback; American/Bermudan (PSOR/LSM); correlated multi-asset basket;
    │                          #   LSV (Andersen-QE variance + Dupire/particle leverage + HV-ADI)
    ├── celnet-heston          # standalone Heston (1993) European vanilla via two independent CF transforms (Carr-Madan + Fang-Oosterlee COS)
    ├── celnet-qmc             # scrambled Joe-Kuo Sobol + Owen-style nested scramble + Brownian-bridge path construction (CPU-first; GPU-reusable)
    # ── Layer 5: GPU ──
    ├── celnet-gpu             # PricingBackend trait + wgpu (Metal/Vulkan/DX12) f32 backend + f64 CPU reconciliation; Philox WGSL;
    │                          #   path/greeks/batch/scenario kernels (closed-form batch + MC path + pathwise/LR Greeks)
    ├── celnet-risk-accel      # GPU screening lens behind the risk cube's ScenarioReprice seam (keeps wgpu out of the cube).
    │                          #   NOTE: currently has ZERO reverse dependencies — celnet-gpu is therefore unreachable from
    │                          #   the server binary. Wiring or retiring it is ADR-0013's open decision.
    # ── Layer 4: engine (hot core) ──
    ├── celnet-engine          # core-pinned zero-alloc hot path; rtrb SPSC; arc-swap/seqlock; blue-green state handoff; durable-book journal
    # ── Layer 5b: durability + replication + fan-out ──
    ├── celnet-journal         # fsync'd append-only sequence-ordered log + deterministic crash recovery (CRC + torn-tail truncation + compaction)
    ├── celnet-replog          # leader-replicated, deterministic-replay log over real loopback sockets: quorum commit, hot-standby failover, full Raft
    ├── celnet-fanout          # lock-free SPMC broadcast ring (every consumer sees every item in order; slow consumers conflate with counted skips)
    # ── Layer 6: risk hierarchy ──
    ├── celnet-risk-normalize  # convention canonicalization + common-numeraire conversion → convention-free canonical risk leaf
    ├── celnet-risk-cube       # single-node hierarchical OLAP risk cube: additive roll-up + non-additive (VaR/ES, FRTB curvature) bump-and-revalue
    ├── celnet-risk-fleet      # cross-shard risk fan-out ALGEBRA over the celnet-router HRW map; fan-out == single-node (transport designed-only)
    ├── celnet-router          # fleet router — shard-by-pair/tenant HRW partition map, stateless replica routing, hot-standby failover
    ├── celnet-limits          # hierarchical limit tree (greek/vega/VaR/concentration/tenor/stop-loss) + RAG + pre/post-trade breach checks
    ├── celnet-entitlements    # principal role-grants + deny rules (information barriers); pre-aggregation pruning predicate (deny-by-default; grant-all only as an explicit, audited assertion)
    ├── celnet-xva             # XVA engine — EPE/ENE exposure profiles + CVA/DVA/FVA over a hazard-rate survival curve (synthetic netting sets)
    ├── celnet-risk-routing    # pure decision-graph engine routing an accepted fill to a risk book (fields/ops/leaves; cycle-checked)
    ├── celnet-risk-transfer   # manual movement of existing risk between books: validation + offsetting-leg / realised-P&L computation
    ├── celnet-hedge-routing   # decision graph resolving a book's risk state to an EXIT action; banded warehouse sizing; vehicle DV01 ratios
    ├── celnet-acceptance      # trader-configurable incoming-quote acceptance rule engine (production, consumed by celnet-server)
    # ── Layer 7: plugin host ──
    ├── celnet-plugin-host     # tiered host: Tier-0 native registry + Tier-2 wasmi fuel-metered sandbox + deterministic replay harness
    # ── Layer 6b: fixed-income / rates (multi-curve term-structure subsystem) ──
    ├── celnet-bond            # settlement-aware cash bond: dirty/clean price, accrued interest, yield-to-maturity, DV01, convexity
    ├── celnet-rates-risk      # bump-and-revalue rate VaR/ES for linear FI + FRTB GIRR delta + the signed key-rate ladder
    ├── celnet-refdata         # pure curated universe: government bonds + CBOT Treasury/STIR futures, ISIN- and date-validated from a committed JSON snapshot
    ├── celnet-refstore        # effective-dated golden-source instrument / corporate-action store (announce→confirm→apply); durability delegated to celnet-journal
    ├── celnet-corpactions     # pure CAEV/CAMV corporate-action model + deterministic effect math (no IO, clock or rng)
    ├── celnet-rates           # FI rates leaf: OIS/SOFR multi-curve bootstrap (log-linear-DF Curve) + FRA/IRS(vanilla-swap)/STIR-futures/cash-bond PV + par-rate/PV01/DV01 + key-rate ladder + Brent solver; over celnet-types/-calendar only (no IO)
    # ── Layer 8: integration + RFQ ──
    ├── celnet-integration     # Celer estate + vendor FX-options MD adapters; multi-source aggregation + divergence detection; egress governor; DeploymentMode {CelerIntegrated/Hybrid/Standalone/ExternalFeedOnly}
    ├── celnet-fix             # FIX engine — zero-copy framing, FIXT/4.4 session, FX-options dialect, acceptor + initiator
    ├── celnet-aggregation     # consolidates N venue top-of-books into one BBO/mid/confidence (median consensus + MAD outlier + staleness decay)
    ├── celnet-tiering         # margins/skews an outbound two-way from a composite mid (flat markup / inventory skew / scaled-smoothed spread)
    ├── celnet-rfq             # multi-dealer RFQ-to-many engine: concurrent fan-out, best-bid/offer ranking, deterministic tie-break, last-look; over celnet-proto/-fix/-types
    # ── Layer 9: edge & clients ──
    ├── celnet-server          # tokio async edge: tonic gRPC (Pricing/Quote/Stream/Surface/Risk/Rates/RFQ-desk/Notification/Auth); FIX acceptor; vendor-feed attach + fleet federation; capability-gated control plane
    ├── celnet-cli             # operator/quant CLI: price, surface, exotic, convention, risk, stream
    ├── celnet-client          # typed async Rust SDK over the wire contract (RFQ/RFS/surface/scenario/risk + exotic vocab builders)
    # ── Observability & validation ──
    ├── celnet-observability   # POD telemetry rings, drain threads, HdrHistogram, metrics, audit, error taxonomy
    ├── celnet-golden          # frozen QuantLib 1.42.1 reference tables (oracle + generator)
    ├── celnet-parity          # executable competitive-parity matrix — each capability claim vs incumbents backed by a gated test
    ├── celnet-analytics       # pure deterministic fold over client-flow / LP / street-order records into P&L-attribution metrics
    ├── celnet-lp-sim          # deterministic synthetic FI liquidity-provider fleet implementing the VenueFeed seam
    ├── celnet-cme-sim         # listed-futures venue built ON TOP of celnet-lp-sim's engine (not a fork — it reuses it)
    ├── celnet-testkit         # invariant assertions, proptest strategies, fixture loaders
    └── celnet-bench           # divan + HdrHistogram latency/throughput suites + committed baselines (core_load/bench_gate/gpu_load/gpu_gate)
```

> **Built:** `celnet-plugin-host` — the tiered plugin host (Tier-0 native registry + Tier-2
> **wasmi** fuel-metered sandbox; wasmtime was rejected for open 2026 RustSec advisories, see
> §6 and `docs/PLUGIN-HOST-ALT.md`). `celnet-fix` is built (zero-copy FIX 4.4 engine,
> acceptor + initiator, wired into `celnet-server`). The remaining provenance-named draft
> crates the early design sketched (`celnet-num`, `celnet-vannavolga`, `celnet-sabr`,
> `celnet-svi`, `celnet-lsv`, `celnet-numerics`, `celnet-pricer`, `celnet-rt`,
> `celnet-compute*`, `celnet-md-*`, `celnet-distributor`, `celnet-staticdata`,
> `celnet-lifecycle`, `celnet-goldgen`, `celnet-surface-build`, `celnet-plugin-guest`,
> `celnet-ipc`) were **consolidated** into the crates above and do **not** exist as separate
> crates. The native Tier-1 `.so` ABI (`stabby`) and Tier-3 Landlock/seccomp ring remain
> designed-only by intent (`docs/PLUGIN-HOST-ALT.md`).

### 2.1 Why this split aids performance

- **`celnet-core` is dependency-free and IO-free.** The hottest math (GK, forward, Greeks,
  date logic) compiles fast, has no framework/runtime in scope, and so cannot accidentally
  allocate, lock, or call a runtime. Determinism and zero-alloc are *structurally*
  enforceable here.
- **The hot core (`celnet-engine`) never depends on the async edge.** The dependency arrow
  points one way: `celnet-server`/`celnet-cli` → `celnet-engine` → models/surface → `celnet-core`.
  Nothing on the pricing path can reach `tokio`, `tonic`, or a socket. `celnet-engine` owns the
  runtime primitives (pinning, SPSC rings, arenas, `seqlock`/`arc-swap` publication) so the
  latency discipline lives in one auditable crate.
- **GPU is behind a trait (`PricingBackend` in `celnet-gpu`) with swappable backends.** The
  engine depends on the abstraction, not on `wgpu`/CUDA, so the heavy/optional GPU stack never
  bloats the hot core's compile or binary and can degrade to CPU at runtime.
- **The plugin host is isolated.** The Wasm host (`celnet-plugin-host`, **built**) is a leaf
  the engine calls through `celnet-plugin-api`; the Wasm runtime's weight and security surface
  never touch the native hot path for first-party models.
- **`celnet-proto` centralizes the single current wire format** in one place (§8) rather than
  smeared across services — and there is exactly one version, so there is no compatibility
  matrix to gate.

### 2.2 Why this split aids parallel multi-agent maintenance

- **Disjoint ownership, stable seams.** Interface crates (`celnet-types`, `celnet-core`,
  `celnet-plugin-api`, `celnet-proto`) are stabilized *first* and changed only with
  coordination. Every other crate is a leaf an agent can own end-to-end: e.g. one session owns
  `celnet-surface`, another `celnet-exotics`, another `celnet-integration` — they never touch
  the same files, so there are no merge conflicts.
- **Compile-time fences match team fences.** Because layers only depend downward, a change
  in `celnet-integration` can't ripple into `celnet-vanilla`; CI rebuilds only the affected
  subtree, keeping the multi-agent feedback loop short.
- **Test ownership is local.** `celnet-testkit` provides shared invariants/strategies so each
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

- **Async edge = `tokio` (1.4x line)** with `tonic 0.12` + `prost 0.13` (the pinned versions
  in the workspace manifest), `hyper`. We *default to tokio* for ecosystem maturity and the `Send + Sync` middleware
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
- **Global allocator (design intent, not yet wired).** The hot core is structurally
  zero-alloc (pre-allocated pools, proven by the counting-allocator guard), so the global
  allocator only matters on the async edge/ingest. The build currently uses the **default
  system allocator**; swapping in a tuned allocator (`mimalloc` for small-alloc p99 on
  ingest, `tikv-jemallocator` for the throughput edge) is a deferred, benchmark-gated change —
  the steady-state gap is small once buffers are pre-allocated, so it is not yet a dependency.

### 3.4 SIMD & math

- **SIMD policy.** Batch slice math (surface arrays, batch payoff) is the planned SIMD
  surface. `std::simd` (portable_simd) is still nightly-only as of May 2026 and we do not pin
  nightly for the core; the `wide` crate is the intended stable-Rust escape hatch and
  `std::arch` intrinsics for a profiled hotspot. **Today the batch paths are scalar
  autovectorized** (the bench shows ~52 ns/option amortized); `wide` is **not yet a
  dependency** — it is added only when a profiled batch hotspot justifies it, keeping
  determinism (no FMA contraction on reproducible paths) intact.
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
// crates/celnet-gpu
pub trait PricingBackend {
    fn simulate_paths(&self, spec: &PathSpec, variates: VariateSource) -> PathBuffer;
    fn reduce_payoff(&self, paths: &PathBuffer, payoff: &PayoffKernel) -> Reduction;
    fn solve_pde(&self, grid: &PdeGrid, scheme: PdeScheme) -> PdeSolution;
}
```

- **Implemented backend: `wgpu` 29 + hand-written WGSL.** `celnet-gpu` targets
  **Metal / Vulkan / GLES / DX12** through `wgpu` with a hand-authored `shader.wgsl` and a
  `philox.rs` host driver; a CPU path provides the no-GPU fallback. (We considered **CubeCL**
  for its single-`#[cube]`-kernel → CUDA/Metal/CPU compilation, but adopted `wgpu`+WGSL
  directly; this is an ADR-grade decision — see the note below. No `cubecl` dependency exists
  anywhere in the tree.)
- **Numeric policy: f32 on GPU, f64 on CPU.** Metal and WebGPU/WGSL have **no native f64**
  (WGSL scalars are only `f32`/`f16`/`i32`/`u32`; atomics are integer-only); even where
  Vulkan/CUDA support f64 it is 16–64× slower than f32. So we **standardize GPU pricing on
  f32** and run an **f64 CPU reconciliation** oracle to bound numerical error for FX exotics.
  Payoff accumulation uses integer-atomic / tree-reduction (no float atomics on WGSL).
- **RNG: counter-based Philox-4×32-10** implemented as a WGSL kernel (`philox.rs`/`shader.wgsl`),
  each variate seeded from `(global_seed, path_index, time_step, dimension)`. Stateless,
  embarrassingly parallel, and **bit-stable across backends** — naive LCG/per-thread RNGs are
  rejected. Box-Muller / inverse-CDF for normals.
- **QMC: Sobol + Brownian-bridge — built (`celnet-qmc`).** A scrambled Joe-Kuo Sobol'
  direction-number sequence (embedded BSD-3-Clause `new-joe-kuo-6.21201` numbers, dims 2..=300)
  with **Owen-style nested scramble** (unbiased RQMC) and **principal-bisection Brownian-bridge**
  dimension ordering is the variance-optimal variate source for path-dependent exotics, swappable
  with Philox behind the `VariateSource` interface (`crates/celnet-qmc/src/{sobol.rs,bridge.rs,
  direction_numbers.rs,normal.rs}`). Measured variance reduction vs fair plain MC: ≈37.7×
  (geometric Asian) / ≈88.5× (European), gated in `celnet-parity/tests/qmc.rs`. The direction
  numbers are exposed for GPU reuse (`celnet-gpu` path/greeks kernels), and `celnet-xva` /
  `celnet-exotics` consume the Sobol+bridge generator. The CPU MC engine additionally implements
  the **BGK 0.5826·σ·√dt** barrier shift and control variates.
- **PDE on GPU:** explicit/ADI as tiled stencil kernels using workgroup shared memory;
  implicit/Crank-Nicolson via cyclic-reduction / PCR tridiagonal solvers — the production
  CN+Rannacher and HV-ADI PDEs run on the CPU in `celnet-exotics` today; **the GPU PDE path
  remains designed-only** (the GPU compute that *is* built is the closed-form batch kernel
  `batch.wgsl`, the MC path kernel `path.wgsl`, the pathwise/LR Greeks kernel `greeks.wgsl`,
  and the scenario-grid kernel `scenario.wgsl`). Validated against the MC engine on vanillas.
- **Deployment:** Linux containers can bundle **Mesa Lavapipe** so GPU-less CI / headless
  nodes exercise the same `wgpu`/Vulkan path on a software device. A CUDA path via Vulkan is
  validated in CI/containers on Linux; the Apple-M4 dev host runs Metal (no native f64), which
  is exactly why the f32-GPU / f64-CPU reconciliation oracle exists.
- **Rejected / deferred for the core today:** `rust-gpu`, `Rust-CUDA`, `cuda-oxide` (promising
  but nightly/experimental, non-integrated codegen); `candle` (tensor/ML-shaped, awkward for
  path-dependent state machines + barrier/autocall logic).

> **ADR — wgpu over CubeCL.** The implemented GPU backend is `wgpu` 29 + WGSL, not CubeCL.
> CubeCL's single-kernel-multi-backend story is attractive but the direct `wgpu`+WGSL path
> ships today with a working f32 kernel + f64 CPU oracle and no extra build-time codegen
> dependency. The CUDA backend is kept first-class as a Linux/CI target; the open fallback
> (`wgpu`/Vulkan) is the primary, satisfying CLAUDE.md guardrail 7.

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
   Unix-domain socket / shared memory using **`rkyv` zero-copy** framing. `SO_REUSEPORT`
   alone does *not* migrate session state — the explicit state-transfer protocol is what
   makes the handoff safe. Because there is a **single current contract** (no versioning,
   rule 9), both old and new processes speak the **same** schema during the brief drain
   window; the handoff is intra-version, not an N/N-1 negotiation.
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

- The **single current wire contract** (§8) means there is exactly one schema in flight: the
  drained old process and the warm new process serialize the **same** `celnet-proto` and
  `rkyv` state framing, so the handoff window has no mixed-version interop to get wrong. The
  whole fleet converges to one uniform version (no N/N-1 negotiation — rule 9); cross-fleet
  rolling upgrade is a router-coordinated one-shard-at-a-time cutover (`docs/SCALE-OUT.md` §7).
- Memory is pre-faulted + `mlock`ed + huge-paged at boot so the new instance is warm before
  cutover (no first-touch page faults on the path).

---

## 6. User-extensibility SDK / plugin model

Desks must add/override **vol models, exotic payoffs, calibrations and stress logic** and
run them *in-engine* — keeping their proprietary IP, not consuming a vendor's models.

### 6.1 Chosen approach: a tiered host — WebAssembly (wasmi) first, native only for trusted first-party

The untrusted-plugin runtime is **wasmi**, the pure-Rust, fuel-metered interpreter — **not
wasmtime**, which carries open 2026 RustSec advisories (including an out-of-sandbox-load class)
and is blocked by guardrail 7 + the `cargo-deny` gate. The full decision, candidate evaluation
and ADR are in `docs/PLUGIN-HOST-ALT.md`; the `celnet-plugin-api` contract (Rust traits + the
`wit/celnet.wit` world) was deliberately runtime-agnostic and is **unchanged** — only the host
runtime differs. wasmi hosts **core Wasm modules** (its Component Model is WIP), so the host
lowers the frozen POD records onto a host-controlled core-module `(ptr,len)` ABI.

| Path | Use | Mechanism | Why |
|---|---|---|---|
| **Wasm — Tier 2 (primary, default for untrusted)** | *Any* user/third-party model or workflow | **`wasmi 1.0.9`** (pure-Rust interpreter), no-WASI capability `Linker`, `Config::consume_fuel`; contract mirrored as a **WIT world** in `celnet-plugin-api`, lowered to a core-module ABI | Only Wasm gives a true **security sandbox** *and* **determinism**. wasmi is advisory-clean, audited, and a pure interpreter (no JIT/native-codegen attack surface). Untrusted code cannot corrupt memory, crash the engine, or stall the hot path. |
| **Native trait registry — Tier 0 (first-party, compiled-in)** | Hot first-party models shipped in the binary | `inventory`/explicit registration of `dyn PricingModel`, same trait shape the Wasm host implements; adapted to the host's tier-blind `HostModel` seam | Fastest path, no ABI/sandbox concerns; first-party and user plugins are interchangeable behind **one** `ModelRegistry`. |
| **Native dynamic `.so`/`.dylib` — Tier 1 (trusted partners only)** | Trusted partner C++/Rust models needing full native speed | **`stabby` 72.x** (not `abi_stable`), code-signed + trusted-publisher allowlist | `abi_stable` is effectively unmaintained. Native plugins have **no sandbox and no determinism** — never loaded untrusted. |
| **OS-process ring — Tier 3 (optional, Linux)** | Highest-distrust multi-tenant prod | Child process under Landlock + seccomp-bpf around Tier 2 | Kernel-enforced second ring; config-gated, Linux-only, never load-bearing for correctness. |

### 6.2 Determinism for plugin execution (wasmi Tier 2)

- **Fuel metering** (`Config::consume_fuel(true)`), not wall-clock epochs — fuel is fully
  deterministic (same input + same fuel ⇒ interrupt at the same instruction). A **per-call
  fuel budget is the plugin's latency/compute SLA**; exhaustion is a typed
  `HostError::FuelExhausted` (a bounded trap), never a hang or panic.
- **NaN canonicalization at every boundary value** (host↔guest, both directions) and **no
  non-deterministic host imports** — no wall-clock, no RNG, no threads, no filesystem, no
  network — locked down at the `Linker` (no WASI is linked, so a guest has **zero ambient
  authority**). Capability-based security: the host grants exactly one audited primitive set
  (the libm `celnet_core::math` transcendentals), and any other import fails to link.
- **Hot-path caveat:** a pure interpreter is ~10–50× slower than a JIT; ultra-hot per-tick
  models compile natively via the Tier-0 registry, and wasmi is reserved for user-supplied /
  less-hot-path (per-quote, calibration) models. The same `PricingModel`/`HostModel` shape
  unifies both.

### 6.3 The SDK as a product

- `celnet-plugin-api` (the WIT world + `PricingModel`/`SmileModel`/`Calibration` traits) is
  **built and frozen**. `celnet-plugin-host` is now **built**: the tiered host with the Tier-0
  native registry and the Tier-2 wasmi sandbox behind one `ModelRegistry`, plus the
  deterministic **replay harness** that replays a fixed market snapshot through a wasm model and
  asserts **bit-identical** output across runs (`to_bits` equality). The four WS-G gates pass:
  capability-denial, fuel-exhaustion bounded, replay bit-identity, and Tier-0 == Tier-2
  interchangeability. (See `docs/CAPABILITIES-VS-COMPETITION.md` — this closes the top
  GA-blocking gap.)
- Cross-platform bit-identity additionally requires libm transcendentals (the contract mandates
  this and the host enforces it by exposing `celnet_core::math` as the guest's math imports), so
  a model is reproducible on the aarch64-apple-darwin dev box and the Linux CI runner alike.
- We build today on **core Wasm modules**, not the Component Model (wasmi's CM is WIP); the WIT
  world is kept lift-ready for the day wasmi ships an advisory-clean Component Model.

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
 │ (API or LSEG feed) │──┐    via celnet-integration  (normalize vendor conventions → canonical)
 └────────────────────┘  │
 ┌────────────────────┐  │   ┌───────────────────┐      ┌──────────────────────────┐
 │ Celer marketdata-  │  ├──►│  celnet-integration│ rtrb │   celnet-surface          │
 │ api (WS, spot/vol) │──┘   │ aggregate ·staleness│ SPSC │ broker→smile fly convert │
 │ MarketMerchantPrice│      │ decay · divergence│─────►│ VV / SSVI / SABR / SVI    │
 └────────────────────┘      │ detection         │      │ delta-space, arb checks  │
        (async edge)         └───────────────────┘      │ (butterfly/calendar/vert)│
                                                         └────────────┬─────────────┘
                                                          arc-swap publish (atomic surface)
                                                                       ▼
   convention registry ─────► ┌──────────────────────────────────────────────────────────┐
   (celnet-conventions,        │                         celnet-engine                       │
    arc-swap)                 │  ModelRegistry: native (inventory) + wasmi Tier-2 sandbox   │
                              │  ┌───────────────┐   ┌──────────────────┐   GPU dispatch  │
                              │  │ celnet-vanilla │   │ celnet-surface VV │   ┌───────────┐ │
   celnet-plugin-host ───────►│  │ GK + 13 Greeks│   │ first-gen exotic │──►│celnet-gpu  │ │
   (Tier-0 native +          │  └───────────────┘   └──────────────────┘   │(wgpu/WGSL):│ │
    Tier-2 wasmi)            │  ┌───────────────────────────────────────┐  │Metal/Vulkan│ │
                              │  │ celnet-exotics (LSV/PDE/MC, path-dep.)   │─►│/DX12 + CPU │ │
                              │  │ + celnet-qmc (Sobol+bridge variates)     │  │reconcile;  │ │
                              │  └───────────────────────────────────────┘  │path/greeks/│ │
                              │                                              │batch kernels│ │
                              └───────────────────────────┬─────────────────└───────────┘─┘
                                       seqlock publish     ▼
                              ┌──────────────────────────────────────────┐
                              │      portfolio Greeks / risk (engine)      │
                              │ Δ,Γ,vega,θ, ρ_d, ρ_f, vanna,volga,charm,  │
                              │ speed,zomma,color · scenario/stress · PnL │
                              │ celnet-risk-{normalize,cube,fleet,limits}  │
                              └───────────────────────┬───────────────────┘
                                     rtrb SPSC         ▼
  ASYNC EDGE (tokio)          ┌──────────────────────────────────────────┐
  ───────────────            │   celnet-server (tonic gRPC + WS mirror)    │──►  Celer front end
                              │   celnet-client (typed Rust SDK)            │──►  programmatic MMs
                              │   celnet-integration (Celer estate bridge)  │──►  Celer OMS/EMS/risk
                              │   celnet-fix (FIX 4.4 acceptor + initiator) │──►  venues / clearing
                              └────────────────────────────────────────────┘
```

**Notes on the Celer integration edge (from the integration research).** *Market-data
normalization, multi-source aggregation and the egress governor live in the single
`celnet-integration` crate (the early design's `celnet-distributor` / `celnet-staticdata` /
`celnet-md-*` split was consolidated); the **FIX 4.4 engine is built as `celnet-fix`** (zero-copy
framing, FIXT/4.4 session, FX-options dialect, acceptor + initiator) and wired into
`celnet-server` as a live acceptor; live JVM-estate lifecycle wiring remains deploy/estate-gated.*
- The Celer distributor is an **in-process disruptor mailbox in the JVM** with
  *skip-while-full* back-pressure — a high-frequency option pricer can cause silent price
  drops, so `celnet-integration` sizes mailboxes and rate-limits, and a Rust process joins via
  a **JVM adapter or the `DistributorProducerChannelHandler` socket protocol** (decision
  deferred but scoped in `celnet-integration`).
- Adding an FX-**option** product type is a broad change touching `celertech-type`,
  staticdata, every API proto enum, positionmanager netting keys, risk exposure models, and
  the destination FIX dialect — concentrated in `celnet-integration` + `celnet-proto` so the
  blast radius is contained.
- `MarketMerchantPriceService` is WS-only (no fallback, ~6 concurrent HTTP connections per
  domain) — the `celnet-integration` Celer feed handler is built resilient to WS disconnects with resync.

---

## 8. Determinism & the single current wire contract

### 8.1 Determinism

- **Math library:** prefer **`rust-lang/libm`** (correctly-rounded, verified ≤ 1.0 ULP vs
  MPFR) over the system libm so transcendental results are **identical across OS/arch and
  across libm updates** — correct rounding yields exactly one answer.
- **Float policy (in `celnet-core`):** IEEE-754; **forbid FMA contraction and fast-math on
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

- **Golden / reference oracle:** `celnet-golden` holds frozen, version-pinned tables of
  **QuantLib 1.42.1** prices + Greeks across a parameter grid (committed); the Rust engine must
  match within documented ULP/relative/absolute tolerances. Re-generated only under explicit
  review, tracking the QuantLib version (treating QuantLib as ground truth without pinning is
  a known risk).
- **Financial invariants as property tests** (`celnet-testkit`, `proptest` — chosen over
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

### 8.3 Single current wire contract

- **`celnet-proto` (prost 0.13 / tonic 0.12)** is **one clean, current contract** — there is
  **no** `schema_version`, no header version field, no negotiation, and no N/N-1
  compatibility gate (CLAUDE.md rule 9: we have no external users and never run a
  mixed-version window). Evolve and refactor the contract freely; an upgrade deploys a single
  uniform version across the fleet (blue-green full cutover, §5; cross-fleet rolling cutover,
  `docs/SCALE-OUT.md` §7).
- **Evolution discipline is editing, not versioning.** Changing the contract means editing
  `celnet-proto` **and every dependent in the same change** (the `Instrument` oneof ripples
  into `celnet-vanilla`/`celnet-exotics`/`celnet-server`/`celnet-client`), updating
  `docs/INTERFACES.md`, and re-indexing the memory graph. No reserve-removed-tag / never-
  renumber ceremony is required because no old reader must ever decode a new message.
- **Two framings, one version:** **`rkyv`** zero-copy for in-host IPC, shared-memory and the
  upgrade state handoff (access archived bytes directly); **`tonic` + `prost`** for external
  service RPC. The hand-rolled zero-copy FIX 4.4 dialect for the venue/Celer-destination wire
  is **built as `celnet-fix`** (acceptor + initiator, loopback-tested, wired into
  `celnet-server`); a separate `celnet-ipc` crate was not needed (the rkyv handoff lives in
  `celnet-engine`).

---

## Appendix A — Key version pins (as of May 2026)

Pins below reflect the **workspace manifest** (`Cargo.toml`), not aspiration.

| Component | Version / choice |
|---|---|
| Rust toolchain | 1.96.0, edition 2024 |
| Async runtime / RPC | tokio 1.4x; **tonic 0.12; prost 0.13** |
| Allocator | **default system allocator** (mimalloc/jemalloc deferred, benchmark-gated — §3.3) |
| Zero-copy IPC / handoff | rkyv (in-host state transfer) |
| GPU | **wgpu 29** (Metal/Vulkan/GLES/DX12) + hand-written WGSL + Philox; f64 CPU reconciliation; **no CubeCL**; Mesa Lavapipe for CI |
| Lock-free / publication | rtrb 0.3 (SPSC), arc-swap 1, seqlock, crossbeam-utils `CachePadded` |
| Plugin runtime | **`wasmi 1.0.9`** (pure-Rust, fuel-metered interpreter) for the untrusted Tier-2 sandbox — wasmtime rejected for open 2026 RustSec advisories. `celnet-plugin-host` (tiered host + replay harness) is **built**; contract (`celnet-plugin-api` WIT + traits) is frozen. Test fixtures: `wat 1` (WAT→wasm, dev-only) |
| Native plugin ABI | Tier 1 trusted-partner path would use `stabby` 72.x (NOT abi_stable); not yet wired |
| SIMD | scalar autovectorized today; `wide` deferred (§3.4); `std::simd` nightly-only, not used |
| Math | rust-lang/libm (correctly rounded) via `celnet-core::math` |
| Bench / metrics | divan; metrics 0.24; hdrhistogram 7; tracing/tracing-subscriber |
| Test/CI | proptest, cargo-fuzz, insta, divan, cargo-llvm-cov, cargo-mutants, cargo-deny, cargo-nextest |
| Reference oracle | **QuantLib 1.42.1** (version-pinned, frozen golden tables in `celnet-golden`) |

## Appendix B — FX-options analytics anchored in the workspace

The analytics standard (see `docs/ANALYTICS-SPEC.md`) maps onto crates as follows:
- **Garman-Kohlhagen** priced off the forward `F = S·e^{(r_d−r_f)T}` storing two discount
  factors `(DF_d, DF_f)` separately (clean deliverable vs NDO and dual-curve) → `celnet-core`,
  `celnet-vanilla`.
- **Four delta conventions** (spot/forward × premium-adjusted/unadjusted) and **ATM types**
  (ATMF vs delta-neutral-straddle, premium in/out) as per-(pair, tenor) config →
  `celnet-conventions`; branch-safe strike↔delta root-find (premium-adjusted delta is
  non-monotone) → `celnet-vanilla`.
- **Smile from ATM + 25d/10d RR + BF**, with explicit **broker(market) → smile strangle**
  calibration → `celnet-surface`.
- **Vanna-Volga** (Castagna-Mercurio) with survival-probability weighting for first-gen
  exotics, **SVI/SSVI**, and **SABR / arb-free density wing** are all consolidated in
  **`celnet-surface`**; **LSV** (Andersen-QE variance backbone + Dupire/particle leverage) is
  in **`celnet-exotics`**.
- **Two rhos (ρ_d, ρ_f)** and the full higher-order Greek set (vanna, volga, charm, speed,
  zomma, color) → `celnet-vanilla` (the **13-Greek** struct in `celnet-types::Greeks`).
- **Calendar/date engine** (intersect both currency calendars + USD; spot/delivery by
  identical lag; modified-following + EOM; NY 10am / Tokyo 3pm cut) → **`celnet-calendar`**;
  conventions + NDF settlement config → `celnet-conventions`/`celnet-types`.
- **Numerical engines — `celnet-exotics` + `celnet-qmc`:** PDE (Crank-Nicolson + Rannacher;
  Hundsdorfer-Verwer ADI for 2-D LSV), MC (Andersen QE variance, **counter-based Philox** RNG,
  Broadie-Glasserman-Kou 0.5826·σ·√dt barrier shift, control variates, geometric/arithmetic
  Asian). **Sobol' QMC + Owen scramble + Brownian-bridge path construction are built in
  `celnet-qmc`** (≈37.7×/88.5× measured variance reduction; see §4) and reused by
  `celnet-exotics`, `celnet-xva` and the `celnet-gpu` path/greeks kernels.
