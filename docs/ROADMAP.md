# Celnet Implementation Roadmap

> A parallel-development roadmap for a Rust-native, SDK-extensible, microsecond-class FX options pricing, vol-surface, exotics, and risk engine, designed for **multiple concurrent Claude sessions** working on **disjoint crates** with **deterministic, non-conflicting** progress.

> **Binding-rules override (2026-05-30):** crate prefix is **`celnet-`**; **no versioned APIs**
> (single current contract; zero-downtime upgrades use blue-green/full cutover, not N/N-1);
> API identifiers are vendor/research-neutral and purpose-named. Any "versioned wire protocol",
> "semver-tagged", or "N↔N-1" wording below is superseded by these rules.

---

## 0. Governing Principles

1. **Interfaces before implementations.** No work-stream implements behavior until the interface crates it depends on (`celnet-types`, `celnet-core` traits, `celnet-proto`, `celnet-plugin-api`) are *frozen* (semver-tagged, CI-green). This is the single rule that prevents merge conflicts and rework.
2. **One crate, one owner, one session at a time.** Each work-stream owns a disjoint set of crates. Two sessions never edit the same `Cargo.toml` member directory concurrently. Cross-crate changes go through interface PRs reviewed against the frozen contract.
3. **Flat virtual workspace, layered by domain function** (per matklad / rust-analyzer style), *not* by technical tier. Root is a `[workspace]` virtual manifest; crate folders named identically to crate names; members under `crates/`.
4. **Determinism is a feature.** Float comparisons use ULP/relative tolerances (never `==`); prefer `rust-lang/libm` (correctly-rounded) for transcendentals; forbid FMA contraction / fast-math on reproducibility-critical paths; never assert on NaN bit patterns. GPU standardizes on **f32**, with **f64 CPU** reconciliation.
5. **Tail latency (p99/p99.9), not mean,** is the latency KPI. Hot path is non-async, core-pinned, zero-alloc.
6. **Greenfield.** `/Users/adrian/code/celeroption` contains only `.git`. Everything below is created from scratch; P0 establishes the skeleton.

---

## 1. Workspace Layout (created in P0, then frozen-by-section)

```
celnet/
├─ Cargo.toml                      # [workspace] virtual manifest; workspace.dependencies + workspace.lints
├─ Cargo.lock                      # committed
├─ rust-toolchain.toml             # pinned stable; pinned nightly only for fuzz job
├─ deny.toml                       # cargo-deny: advisories+licenses+bans+sources
├─ supply-chain/                   # cargo-vet config + audits
├─ CLAUDE.md                       # the LEDGER (see §7)
├─ docs/
│  ├─ ROADMAP.md                   # this file
│  ├─ INTERFACES.md                # frozen-interface registry + semver tags
│  └─ CONVENTIONS.md               # FX convention spec (delta/ATM/premium/cut/daycount)
└─ crates/
   # ── SHARED INTERFACE LAYER (freeze FIRST — see §3) ──
   ├─ celnet-types/                 # POD value types: Ccy, CcyPair, Tenor, Money, Rate, Vol, Delta, scalar policy
   ├─ celnet-core/                  # pure-domain TRAITS + math primitives (zero IO, zero framework deps)
   ├─ celnet-proto/                 # versioned wire protocol (prost 0.14.x); message header w/ schema version
   ├─ celnet-plugin-api/            # SDK: PricingModel/VolModel traits + WIT world; semver product
   # ── ENGINE / IMPLEMENTATION LAYER ──
   ├─ celnet-conventions/           # FX conventions config: per-(pair,tenor) delta/ATM/premium/cut/daycount
   ├─ celnet-calendar/              # holiday calendars, spot lag, delivery, modified-following, EOM
   ├─ celnet-vanilla/               # Garman-Kohlhagen, full Greek set, strike<->delta solver
   ├─ celnet-surface/               # vol-surface construction: VV, SABR, SVI/SSVI, broker->smile fly
   ├─ celnet-exotics/               # barriers/touches/DNT/digitals (VV), LSV, PDE + MC engines
   ├─ celnet-gpu/                   # CubeCL backend(s) + CPU fallback; PricingBackend impls
   ├─ celnet-engine/                # stateful low-latency service; hot path; hot-upgrade handoff
   ├─ celnet-plugin-host/           # wasmtime 45 embedding; fuel metering; capability linker
   ├─ celnet-integration/           # Celer estate adapters (distributor, Protobuf, FIX) + marketdata feed
   ├─ celnet-server/                # thin binary: gRPC/WebSocket edge (tokio + tonic)
   └─ celnet-cli/                   # thin binary: admin/diagnostics
```

**Dependency direction (must never invert):**
`celnet-types` ← `celnet-core` ← {`celnet-conventions`, `celnet-calendar`, `celnet-vanilla`, `celnet-surface`, `celnet-exotics`, `celnet-gpu`} ← `celnet-engine` ← {`celnet-server`, `celnet-cli`}.
`celnet-proto` and `celnet-plugin-api` depend only on `celnet-types` (+ `celnet-core` traits for the plugin API). `celnet-plugin-host` depends on `celnet-plugin-api`. `celnet-integration` depends on `celnet-proto` + `celnet-core`.

---

## 2. Phases & Exit Criteria

| Phase | Theme | Exit Criteria (all must hold; CI-green) |
|---|---|---|
| **P0** | Foundations & interface freeze | Virtual workspace builds; `celnet-types`, `celnet-core` traits, `celnet-proto` v0, `celnet-plugin-api` v0 are **semver-tagged & frozen**; CI matrix (ubuntu/macos/windows × {stable, MSRV}) on `cargo-nextest` green; `clippy -D warnings`, `cargo fmt --check`, `cargo-deny`, coverage (Linux) wired; `CLAUDE.md` ledger + `docs/INTERFACES.md` live; float-compare helper (`assert_close`, ULP+rel+abs) shipped in `celnet-core`. |
| **P1** | Vanilla core | `celnet-conventions` + `celnet-calendar` complete; `celnet-vanilla` prices GK call/put off forward F=S·e^{(r_d−r_f)T} with separate DF_d/DF_f; full first/second/higher Greeks (two rhos, vanna, volga, charm, speed, zomma, color); strike↔delta solver respecting all 4 delta conventions; **validated against Reiswich-Wystup (2010) & Clark worked numbers** within documented tolerances. |
| **P2** | Vol surface | `celnet-surface` builds delta-space smile from ATM/25d/10d RR/BF; **broker→smile strangle calibration** implemented; VV (Castagna-Mercurio 2nd approx), SABR (Hagan + arbitrage-free PDE), SVI/SSVI (Gatheral-Jacquier) selectable; arbitrage gates (butterfly density ≥0, calendar total-variance monotone, vertical) pass as tests; total-variance/business-time tenor interpolation. |
| **P3** | Exotics + GPU | `celnet-exotics` two-tier (fast VV for 1st-gen with survival-probability weighting; LSV [Heston+Dupire leverage, particle calibration] for booking/2nd-gen); PDE (Crank-Nicolson+Rannacher, ADI Craig-Sneyd) + MC (Andersen QE, Sobol+Brownian-bridge, BGK barrier correction); `celnet-gpu` CubeCL path (CUDA/Metal/Vulkan/WGSL) with f32 + CPU f64 reconciliation, Philox RNG. |
| **P4** | Engine, SDK, integration | `celnet-engine` hot path (non-async, core-pinned, zero-alloc, SPSC rtrb); zero-downtime SO_REUSEPORT handoff + versioned state transfer; `celnet-plugin-host` wasmtime 45 fuel-metered sandbox; `celnet-plugin-api` SDK shippable with deterministic replay harness; `celnet-integration` consumes Celer distributor/marketdata + FMD-style surface feed; `celnet-server`/`celnet-cli` expose gRPC/WS. |
| **GA** | Hardening & release | Full numerical golden suite vs QuantLib pinned oracle; property + fuzz + mutation gates met (≥90% coverage core modules; kill-rate gate); p99/p99.9 latency budgets met & regression-gated; hot-upgrade N↔N-1 wire-compat CI gate green; supply-chain (cargo-deny + cargo-vet baseline) clean; deployment (NVIDIA container toolkit + Lavapipe CI fallback) verified; docs complete. |

---

## 3. Stabilize-Interfaces-First Sequencing (the unblock plan)

**Gate G0 — Interface Freeze (end of P0).** Until G0 passes, *only the Interface Work-Stream runs*. All other sessions are blocked. G0 deliverables:

1. **`celnet-types`** — POD, `Copy`, no IO: `Ccy`, `CcyPair` (FOR=base/CCY1, DOM=quote/CCY2), `Tenor`, `Money`, discount factors `Df`, `Vol`, `Delta`, `Strike`, scalar policy (`f64` CPU canonical / `f32` GPU), enums for `DeltaConvention {SpotUnadj, FwdUnadj, SpotPremAdj, FwdPremAdj}`, `AtmConvention {Atmf, Dns}`, `PremiumStyle {DomPips, For%, Dom%, ForPips}`, `Cut {NY1000, Tokyo1500}`, `DayCount {Act365, Act360}`, `Settlement {Deliverable, Ndo}`. **Frozen first — everything depends on it.**
2. **`celnet-core` traits** — `PricingModel`, `VolModel`/`SmileModel`, `PricingBackend { simulate_paths, reduce_payoff, solve_pde }`, `Calendar`, `CalibrationTarget`, plus the `assert_close` ULP/rel/abs float helper and the scalar abstraction. Trait *shapes* must match what both the trait-object registry and the wasm host implement (so first-party and user plugins are interchangeable).
3. **`celnet-proto`** — message envelope with explicit `schema_version`; prost 0.14.x; evolution discipline documented in `docs/INTERFACES.md` (never renumber tags, reserve removed tags, additive optional fields). N/N-1 compat test scaffold.
4. **`celnet-plugin-api`** — the SDK trait surface + WIT `world` definition for wasm plugins; semver-tagged as a *product*.

**Rule:** Any later change to a frozen interface crate is a **dedicated interface PR**: bump semver, update `docs/INTERFACES.md`, run the N/N-1 compat gate, and announce in `CLAUDE.md`. Implementation sessions rebase, never edit interface crates ad hoc.

**Gate G1 (end of P1):** `celnet-vanilla` + `celnet-conventions` + `celnet-calendar` stable → unblocks `celnet-surface`.
**Gate G2 (end of P2):** `celnet-surface` stable → unblocks `celnet-exotics` (needs arbitrage-free smile as upstream input for Dupire local vol).
**Gate G3 (end of P3):** `celnet-exotics` + `celnet-gpu` stable → unblocks full `celnet-engine` pricing wiring.

---

## 4. Work-Stream Decomposition (disjoint crate ownership)

> Each row is owned by **one session at a time**. "Depends on" lists hard gates. A stream may start as soon as its dependency gate is green.

### WS-0 · Interface & Platform (runs first, alone, until G0)
- **Owns:** `celnet-types`, `celnet-core` (traits + float helper), `celnet-proto`, `celnet-plugin-api`; root `Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `supply-chain/`, CI workflows, `docs/INTERFACES.md`, `docs/CONVENTIONS.md`, `CLAUDE.md` scaffold.
- **Deliverables:** virtual workspace; `[workspace.dependencies]` + `[workspace.lints]`; frozen interface crates; CI matrix on `cargo-nextest` + clippy/fmt/deny/coverage; MSRV job (edition 2024, MSRV-aware resolver); `assert_close` helper; proptest-regressions/ + fuzz corpora directories committed.
- **Depends on:** nothing.
- **Gates:** workspace builds; interface crates semver-tagged; CI green on all 3 OSes × {stable, MSRV}; `cargo-deny` + `cargo-audit` clean; coverage job (Linux) wired. **= Gate G0.**

### WS-A · FX Conventions & Calendar
- **Owns:** `celnet-conventions`, `celnet-calendar`.
- **Deliverables:** per-(pair,tenor) convention config as **first-class data, not global defaults** (delta type, ATM type, premium ccy + pips/%, spot lag, cut, day count, settlement); calendar engine intersecting **both** ccy calendars (+ USD for cross-via-USD); horizon→spot and expiry→delivery by identical lag rules; modified-following + EOM; T+2 default with T+1 exceptions (USDCAD/USDTRY/USDRUB/USDPHP); NY 10:00 / Tokyo 15:00 cut per pair; ACT/365 vol-time vs ACT/360-etc accrual kept distinct; NDO fixing-to-settlement lag.
- **Depends on:** G0.
- **Gates:** property tests on date arithmetic (delivery never before spot; EOM correctness); golden tables of spot/delivery dates for a pair set; snapshot (`insta`) tests of resolved convention sets.

### WS-B · Vanilla Pricing & Greeks
- **Owns:** `celnet-vanilla`.
- **Deliverables:** GK core off forward F=S·e^{(r_d−r_f)T} storing DF_d/DF_f separately; `d1/d2` per spec; full Greeks incl. **two rhos** (rho-domestic, rho-foreign), vega (per vol point/100), gamma, theta, vanna, volga/vomma, charm, speed, zomma, color, bucketed vega ladder; robust strike↔delta root-finder (Brent/Newton) per configured delta convention, with **guarded bracketing for non-monotone premium-adjusted call delta**; DNS strike sign correct under each convention (F·e^{+½σ²T} unadjusted vs F·e^{−½σ²T} premium-adjusted).
- **Depends on:** G0; WS-A (consumes `celnet-conventions`/`celnet-calendar`).
- **Gates:** **golden tests vs Reiswich-Wystup (2010) & Clark** per delta/ATM convention; put-call parity property test (within tolerance); Greeks vs finite-difference bumps; intrinsic-value lower bounds; ULP/rel tolerances throughout. **= Gate G1.**

### WS-C · Vol Surface
- **Owns:** `celnet-surface`.
- **Deliverables:** delta-space smile from ATM(DNS)/RR_25/BF_25 (+10d); **broker(market) strangle → smile strangle iterative calibration** (explicit, never arithmetic-average); pluggable smile models — VV (Castagna-Mercurio incl. 2nd approx), SABR (Hagan expansion + arbitrage-free PDE SABR for wings/low-vol; shifted/normal only if forwards→0), SVI raw + SSVI (closed-form butterfly+calendar no-arb); tenor interpolation in **total variance / business time** with weekend/holiday/event weighting; arbitrage repair + cross-checks; multi-source blend hook (client marks + vendor feed) flagging divergence.
- **Depends on:** G1.
- **Gates:** reprice input ATM/RR/BF **exactly** (OVML-style); density ≥0 (butterfly), monotone total variance (calendar), call-price monotone in strike — all as property/invariant tests; VV cross-checked against SSVI in wings; broker-vs-smile-fly regression test on a high-RR/EM pair. **= Gate G2.**

### WS-D · Exotics & Numerical Engines
- **Owns:** `celnet-exotics`.
- **Deliverables:** **two-tier** — (1) fast VV for 1st-gen (one-touch, no-touch, DNT, single/double barriers, European digitals) with survival-probability/first-exit-time weighting and `[0,notional]` clamps; (2) **LSV** (Heston backbone + Dupire leverage L²=σ_Dupire²/E[v|S], **particle-method** calibration, exposed mixing weight η) for booking/Greeks/2nd-gen (window/partial barriers, TARFs/accumulators, Asians, lookbacks, forward-starts). PDE: Crank-Nicolson + **Rannacher** start-up, ADI Craig-Sneyd / Hundsdorfer-Verwer for 2D LSV, log-spot grids, barrier-aligned nodes. MC: **Andersen QE** for CIR variance (full-truncation when Feller violated), Sobol QMC + Brownian-bridge dimension ordering, geometric-Asian/vanilla control variates, **BGK 0.5826·σ·√dt** + Brownian-bridge barrier corrections. Variance swap via log-contract (1/K² strip); vol swap with explicit convexity adjustment.
- **Depends on:** G2 (needs arbitrage-free smile for Dupire); coordinates with WS-E via the `PricingBackend` trait (frozen at G0).
- **Gates:** cross-method validation (VV vs LSV-PDE vs LSV-MC) within tolerance; barrier/touch reconciled to broker DNT/one-touch as SV-mixing target; geometric-Asian closed-form vs MC control variate; explicit TARF gap-risk stress test; Rannacher-vs-naive oscillation regression on digitals.

### WS-E · GPU Acceleration
- **Owns:** `celnet-gpu`.
- **Deliverables:** `CubeClBackend` (v0.10.x, features cuda/wgpu/hip/cpu) implementing `PricingBackend`; **f32 on GPU, f64 CPU reconciliation** path (rayon + `wide`); **Philox-4x32-10** counter-based RNG (`#[cube]`) seeded from (global_seed, path_index, step, dim); Sobol QMC (host-precomputed direction numbers, Brownian-bridge dim assignment); FD stencil kernels (tiled shared-memory; PCR/cyclic-reduction tridiagonal for implicit); runtime adapter probing → CPU fallback when no GPU; integer-atomic / tree-reduction payoff accumulation (no float atomics in WGSL).
- **Depends on:** G0 (`PricingBackend` trait + scalar policy). Can run **in parallel with WS-D** because both target the same frozen trait; integrate at G3.
- **Gates:** GPU f32 vs CPU f64 reconciliation within documented error bound on vanillas; Philox bit-stability across backends; Sobol convergence vs MC; Lavapipe software-Vulkan CI path exercises the same wgpu code; pinned NVIDIA driver/toolkit container smoke test.

### WS-F · Engine & Hot Upgrade
- **Owns:** `celnet-engine`.
- **Deliverables:** two-tier architecture — non-async, **core-pinned** (`core_affinity` + isolcpus/nohz_full/rcu_nocbs) busy-poll hot path for pricing/risk; **zero-alloc** (pre-allocated pools, arrayvec/smallvec/heapless); `rtrb` SPSC ring buffers between hot path and async edge (not channels); `arc-swap` for hot-reloadable reference data, `seqlock` for single-writer price snapshots, `CachePadded` on shared atomics; mimalloc/tikv-jemallocator global allocator (benchmarked); **zero-downtime upgrade** via SO_REUSEPORT graceful socket handoff + drain + versioned live-state transfer over `celnet-proto` (UDS/shared memory); full-book Greeks + spot/vol stress recompute.
- **Depends on:** G1 (vanilla), G3 ideally for full wiring; can scaffold hot-path + handoff against trait stubs after G0.
- **Gates:** HdrHistogram p99/p99.9 budgets met; zero-alloc assertion in hot loop (dhat/counting-allocator guard in tests); **N↔N-1 wire-compat** handoff test (no dropped/corrupted state across versions); no synchronous logging on hot path (telemetry over bounded queue to non-critical core).

### WS-G · Plugin Host & SDK
- **Owns:** `celnet-plugin-host`; co-owns `celnet-plugin-api` *contract evolution only* via interface PRs.
- **Deliverables:** wasmtime **45.0.0** + Component Model + WASI 0.2.x embedding; capability-based `Linker` exposing only explicit market-data/pricing primitives; **deterministic execution** — `Config::consume_fuel(true)` with per-call fuel budget as compute SLA, NaN canonicalization, no wall-clock/RNG/threads imports unless seeded; trait-object registry for compiled-in first-party models (same trait shape as wasm host → interchangeable); deterministic replay harness (fixed market-data snapshot → bit-identical pricing output). Native `.so`/`.dylib` path **only** via `stabby` 72.1.x behind code-signing + allowlist (abi_stable is unmaintained — not used).
- **Depends on:** G0 (`celnet-plugin-api` WIT world + traits).
- **Gates:** sandbox escape test (capability denial); fuel-exhaustion bounded-runtime test; replay determinism (bit-identical) gate; first-party-vs-wasm interchangeability test through one registry.

### WS-H · Celer Estate Integration & Feeds
- **Owns:** `celnet-integration`.
- **Deliverables:** adapter to the in-process JVM **distributor** (bounded disruptor mailbox, skip-while-full) via JVM adapter or `DistributorProducerChannelHandler` socket protocol — **decision recorded as ADR** before build; sized mailboxes + rate-limiting so a high-frequency option pricer cannot cause silent price drops; resilient `MarketMerchantPriceService` WS consumer (handle disconnects, respect ~6 concurrent HTTP conns/domain semaphore); vol-data feed handler (verify marketdata-api can supply vol surface/Greeks input or provision additional feed); **FMD FXO 2.0-style surface adapter** normalizing vendor ATM/25d&10d RR/BF + spot/fwd/NDF into the canonical surface object; new-`OptionProduct`-type touch-list tracked as a cross-cutting ADR (celertech-type, staticdata, api proto enums, positionmanager netting keys, risk exposure models, destination FIX dialect).
- **Depends on:** G0 (`celnet-proto`), WS-C (surface object) for the FMD adapter.
- **Gates:** back-pressure test (mailbox full → bounded, no corruption); WS-reconnect resilience test; FMD-feed normalization round-trip snapshot; cross-service dependency map maintained **manually** (codebase-memory finds zero cross-service edges for in-proc distributor/Protobuf/FIX) and verified against Spring config + distributor `notifyUsers` names.

### WS-I · Edge Binaries
- **Owns:** `celnet-server`, `celnet-cli`.
- **Deliverables:** thin tokio async edge — `tonic` ~0.13 + `prost` ~0.13 gRPC, WebSocket streaming RFS, admin CLI/diagnostics; connects to hot path only via `rtrb`; `/readyz` + connection-drain for LB/eBPF reuseport steering during upgrade.
- **Depends on:** G3 + WS-F engine surface.
- **Gates:** integration smoke (price round-trip over gRPC + WS); drain/readiness behavior under simulated rolling restart.

### WS-T · Test/CI/Determinism Backbone (cross-cutting, low-conflict)
- **Owns:** CI workflow files, `deny.toml`, `supply-chain/`, the QuantLib golden-table generator harness, fuzz targets directory, mutation/coverage config. *Edits config + test-only files; does not edit other streams' `src/`.*
- **Deliverables:** layered pyramid — golden vs **QuantLib (version-pinned, e.g. PyQL 1.40) frozen tables**, invariant tests, **proptest** (Strategy generators for valid market inputs; commit `proptest-regressions/`), **cargo-fuzz** (Arbitrary → adversarial inputs; Linux nightly job only; commit corpus/crashes), `insta` snapshots (CI=1 fails stale), criterion/divan perf gates with committed baselines; `cargo-llvm-cov nextest` fail-under (≥90% core); `cargo-mutants` (incremental on PR diff, full scheduled on main); `cargo-deny` + `cargo-vet` per PR; MSRV-pinned job.
- **Depends on:** G0; then continuously supports all streams.
- **Gates:** every other stream's gates are *defined and enforced here*; benchmark gate lenient off protected branches, strict on main with per-key tolerances; Windows excluded from coverage gate (known-broken); fuzz isolated to Linux nightly.

---

## 5. Dependency / Parallelism Map

```
                         WS-0 (G0 freeze) ── blocks everyone until green
                                  │
        ┌──────────────┬──────────┼───────────────┬───────────────┬──────────────┐
        ▼              ▼          ▼                ▼               ▼              ▼
      WS-A          WS-E        WS-G             WS-T          WS-F(scaffold)  WS-H(distributor ADR,
   (conventions/  (GPU, on     (plugin host,  (CI/test,       (hot path +      feeds; FMD adapter
    calendar)      PricingBackend) on plugin-api) continuous)  handoff stubs)   waits on WS-C)
        │
        ▼ G1
      WS-B (vanilla) ──► G1 unblocks ──► WS-C (surface) ──► G2 ──► WS-D (exotics)
                                                                      │
                                              WS-D + WS-E ──► G3 ──► WS-F (full engine) ──► WS-I (edge)
```

**Maximally parallel after G0:** WS-A, WS-E, WS-G, WS-T, WS-H(ADR phase), WS-F(scaffold) all proceed concurrently on disjoint crates. WS-B starts immediately after G0 (depends only on WS-A's crate *interfaces*, which are part of the conventions/calendar trait surface — coordinate so `celnet-conventions`/`celnet-calendar` public APIs land early in P1).

---

## 6. Cross-Cutting ADRs (record before building, in `mcp__codebase-memory-mcp__manage_adr` and `docs/INTERFACES.md`)

1. **Async runtime split** — tokio edge (default, for tonic/hyper ecosystem) vs non-async core-pinned hot path; monoio/glommio explicitly rejected for the engine core (Send/Sync ecosystem loss, maintenance/io_uring security risk) unless a measured IO-bound workload justifies a prototype.
2. **Allocator choice** — mimalloc (small-alloc tail latency) vs tikv-jemallocator (multi-thread throughput); benchmark on real workload, gap small in steady state.
3. **Distributor integration mechanism** — JVM adapter vs `DistributorProducerChannelHandler` socket protocol; back-pressure sizing.
4. **GPU numeric policy** — f32 GPU canonical + f64 CPU reconciliation; no f64-on-GPU promise (Metal/WGSL lack native f64).
5. **Plugin trust tiers** — wasm (untrusted, default, sandboxed+fuel) vs trusted-first-party native (stabby + signing); abi_stable banned.
6. **New `OptionProduct` type rollout** — enumerate the full touch-list (celertech-type, staticdata, proto enums, positionmanager netting keys, risk exposure, FIX dialect) as a coordinated breaking change.

---

## 7. Per-Session Operating Protocol

Every Claude session, on every pickup, follows this loop:

### 7.1 Pick up work
1. `git pull`; read `CLAUDE.md` (the ledger) and `docs/INTERFACES.md`.
2. Verify **G0 is green**. If not, and you are not WS-0, **stop** — only interface work proceeds pre-G0.
3. Choose an **unclaimed** work-stream from the ledger whose dependency gate is green. Claim it by editing the ledger (see §7.3) and committing/pushing that claim *first* (atomic claim to prevent two sessions taking the same stream).
4. Confirm your stream's crates are **disjoint** from every other `IN-PROGRESS` claim. If overlap, pick another stream.

### 7.2 Do work
5. Work **only inside your owned crate(s)**. Never edit a frozen interface crate directly — if you need an interface change, open a dedicated **interface PR** (bump semver, update `docs/INTERFACES.md`, run N/N-1 compat gate, announce in ledger) and pause dependent work until it lands.
6. Consume frozen interfaces by their semver-tagged version. Rebase onto interface updates; do not fork them.
7. Keep determinism rules: `assert_close` for floats, `rust-lang/libm`, no FMA on reproducible paths, no NaN-bit assertions, GPU=f32.

### 7.3 Update the ledger (`CLAUDE.md`)
The ledger is the single source of truth for parallel coordination. Maintain a table:

```
## Work-Stream Ledger
| Stream | Crates (owned) | Status | Session/Branch | Depends-on gate | Last gate run | Notes |
|--------|----------------|--------|----------------|-----------------|---------------|-------|
| WS-A   | celnet-conventions, celnet-calendar | IN-PROGRESS | sess-3 / ws-a-cal | G0 | 2026-05-30 green | EOM logic done |
| ...    | ... | UNCLAIMED / IN-PROGRESS / BLOCKED / DONE | ... | ... | ... | ... |
```
Also append to a **decision log** section any ADRs, interface bumps (with semver + tag), and gate transitions (G0→G1 etc.). Update `index_repository` / codebase-memory after structural changes so the manual cross-service map stays accurate.

### 7.4 Test gates before "done" (must all pass locally, then in CI)
A stream is **DONE** only when, on its owned crates:
- `cargo fmt --check` and `cargo clippy -- -D warnings` clean.
- `cargo nextest run` green on Linux/macOS/Windows × {stable, MSRV}.
- The stream's **specific gates** (§4) pass — golden vs QuantLib/Reiswich-Wystup/Clark within ULP/rel tolerances, invariant + proptest properties, `insta` snapshots reviewed (no blind accept; CI=1), criterion/divan baselines within tolerance.
- `cargo-llvm-cov nextest` ≥90% on core pricing modules (Linux); `cargo-mutants` incremental kill-rate gate on the diff.
- `cargo-deny` (+ `cargo-vet` if deps added) clean; `Cargo.lock` committed.
- Latency-touching streams: HdrHistogram p99/p99.9 within budget, zero-alloc hot-loop assertion, regression-gated.
- Commit `proptest-regressions/` and any new fuzz corpus/crash artifacts so failing seeds replay deterministically everywhere.
- Update the ledger row to `DONE` with the last green gate timestamp, and announce any gate transition (e.g. "G1 reached — WS-C unblocked").

### 7.5 Hand-off / merge discipline
8. Open one PR per stream increment; PRs touch only owned crates + shared config you own (CI/test files belong to WS-T). Cross-crate edits = interface PR (rare, reviewed against the frozen contract).
9. After merge, push, re-pull, update the ledger, and either continue the same stream or release the claim.

---

## 8. Determinism & Reproducibility Checklist (applies to all streams, enforced by WS-T)

- Pin one toolchain via `rust-toolchain.toml`; nightly only for the isolated Linux fuzz job (cargo-fuzz needs nightly + C++ toolchain, no Windows).
- Prefer `rust-lang/libm` (correctly-rounded ≤1.0 ULP vs MPFR) over system libm for cross-platform identical transcendentals.
- Forbid FMA contraction / fast-math where reproducibility matters; test both contracted and non-contracted where relevant.
- All float assertions via `assert_close` (rel + abs + ULP). Never `==`. Never assert on NaN payloads; guard NaN/Inf in pricing inputs/outputs.
- GPU = f32 canonical with f64 CPU reconciliation bound; integer-atomic / tree-reduction payoff accumulation; Philox counter-based RNG seeded from (seed, path, step, dim) for bit-stable cross-backend results.
- QuantLib treated as a **version-pinned** oracle; golden tables regenerated only under explicit review with version recorded.
- `edition = 2024`, MSRV declared in `Cargo.toml` `rust-version`, MSRV-aware resolver, dedicated MSRV CI job; MSRV bump = minor version per API guidelines.

---

*End of roadmap. Source of truth for live status is `CLAUDE.md` (the ledger); source of truth for contracts is `docs/INTERFACES.md`; FX convention spec is `docs/CONVENTIONS.md`.*