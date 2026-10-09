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
6. **Greenfield.** The repo root contains only `.git`. Everything below is created from scratch; P0 establishes the skeleton.

---

## 1. Workspace Layout (64 crates)

```
celnet/
├─ Cargo.toml                      # [workspace] virtual manifest; workspace.dependencies + workspace.lints
├─ Cargo.lock                      # committed
├─ rust-toolchain.toml             # pinned stable (1.96.0); pinned nightly only for fuzz job
├─ deny.toml                       # cargo-deny: advisories+licenses+bans+sources
├─ CLAUDE.md                       # the LEDGER & operational guardrails
├─ docs/                           # documentation corpus & reference libraries
├─ excel/                          # Excel Custom Functions Add-in
├─ gui/                            # React high-density trading studio cockpit
├─ python/                         # Python high-performance SDK & dynamic runtime
└─ crates/                         # 64 crates (Layer 0–9, flat virtual manifest)
   ├─ celnet-types/  celnet-core/  celnet-proto/  celnet-plugin-api/  celnet-plugin-host/
   ├─ celnet-conventions/  celnet-calendar/  celnet-vanilla/  celnet-equity-vanilla/
   ├─ celnet-commodity-vanilla/  celnet-crypto-vanilla/  celnet-linear/  celnet-surface/
   ├─ celnet-exotics/  celnet-heston/  celnet-qmc/  celnet-gpu/  celnet-risk-accel/
   ├─ celnet-rates/  celnet-bond/  celnet-rates-risk/  celnet-rates-exotics/  celnet-refdata/
   ├─ celnet-refstore/  celnet-corpactions/  celnet-margin/  celnet-xva/  celnet-algo/
   ├─ celnet-engine/  celnet-journal/  celnet-replog/  celnet-fanout/  celnet-shm/  celnet-sbe/
   ├─ celnet-exchange-codecs/  celnet-fix/  celnet-risk-normalize/  celnet-risk-cube/  celnet-risk-fleet/
   ├─ celnet-router/  celnet-limits/  celnet-entitlements/  celnet-risk-routing/  celnet-risk-transfer/
   ├─ celnet-hedge-routing/  celnet-acceptance/  celnet-tiering/  celnet-aggregation/  celnet-rfq/
   ├─ celnet-upgrade/  celnet-license/  celnet-server/  celnet-cli/  celnet-client/  celnet-c-api/
   ├─ celnet-observability/  celnet-golden/  celnet-parity/  celnet-analytics/  celnet-lp-sim/
   ├─ celnet-cme-sim/  celnet-testkit/  celnet-integration/  celnet-bench/
```

> The workspace has evolved through cross-asset, fixed-income/rates, liquidity composite,
> execution algorithms, ULL messaging/SBE/SHM, Multi-Raft consensus, and dynamic capability
> licensing into **64 crates** (detailed in `docs/ARCHITECTURE.md` §2 and `docs/INTERFACES.md`).

**Dependency direction (must never invert):**
`celnet-types` ← `celnet-core` ← {leaf domain crates} ← `celnet-engine` ← {`celnet-server`, `celnet-cli`}.
`celnet-proto` depends only on `celnet-types`. `celnet-plugin-api` depends only on `celnet-types` (+ `celnet-core` traits).
No cyclic dependencies. Single unversioned contract across all surfaces (Rule 9).

---

## 2. Phases & Exit Criteria

| Phase | Theme | Exit Criteria (all must hold; CI-green) |
|---|---|---|
| **P0** | Foundations & interface freeze | Virtual workspace builds; `celnet-types`, `celnet-core` traits, `celnet-proto` v0, `celnet-plugin-api` v0 are **semver-tagged & frozen**; CI matrix (ubuntu/macos/windows × {stable, MSRV}) on `cargo-nextest` green; `clippy -D warnings`, `cargo fmt --check`, `cargo-deny`, coverage (Linux) wired; `CLAUDE.md` ledger + `docs/INTERFACES.md` live; float-compare helper (`assert_close`, ULP+rel+abs) shipped in `celnet-core`. |
| **P1** | Vanilla core | `celnet-conventions` + `celnet-calendar` complete; `celnet-vanilla` prices GK call/put off forward F=S·e^{(r_d−r_f)T} with separate DF_d/DF_f; full first/second/higher Greeks (two rhos, vanna, volga, charm, speed, zomma, color); strike↔delta solver respecting all 4 delta conventions; **validated against Reiswich-Wystup (2010) & Clark worked numbers** within documented tolerances. |
| **P2** | Vol surface | `celnet-surface` builds delta-space smile from ATM/25d/10d RR/BF; **broker→smile strangle calibration** implemented; VV (Castagna-Mercurio 2nd approx), SABR (Hagan + arbitrage-free PDE), SVI/SSVI (Gatheral-Jacquier) selectable; arbitrage gates (butterfly density ≥0, calendar total-variance monotone, vertical) pass as tests; total-variance/business-time tenor interpolation. |
| **P3** | Exotics + GPU | `celnet-exotics` two-tier (fast VV for 1st-gen with survival-probability weighting; LSV [Heston+Dupire leverage, particle calibration] for booking/2nd-gen); PDE (Crank-Nicolson+Rannacher, HV-ADI) + MC (Andersen QE, Philox + BGK barrier correction + control variates; **Sobol'+Brownian-bridge Built in `celnet-qmc`**); `celnet-gpu` **wgpu/WGSL** path (Metal/Vulkan/DX12/GLES) with f32 + CPU f64 reconciliation, Philox RNG. (Built: + lookbacks, forward-start/cliquet, TARF/accumulator, quanto, variance/vol swaps, arithmetic Asian — gated `celnet-parity` rows.) |
| **P4** | Engine, SDK, integration | `celnet-engine` hot path (non-async, core-pinned, zero-alloc, SPSC rtrb); zero-downtime SO_REUSEPORT handoff + single-current-contract state transfer; `celnet-plugin-api` SDK contract + the tiered `celnet-plugin-host` (Tier-0 native + Tier-2 wasmi fuel-metered sandbox; wasmtime rejected for advisories); `celnet-integration` consumes CelNet marketdata + FMD-style surface feed; `celnet-server`/`celnet-client` expose the gRPC contract (WS mirror designed). |
| **GA** | Hardening & release | Full numerical golden suite vs QuantLib **1.42.1** pinned oracle; property + fuzz + mutation gates met (≥90% coverage core modules; kill-rate gate); p99/p99.9 latency budgets met & regression-gated; **blue-green single-version state-handoff** CI gate green (no mixed-version window); supply-chain (cargo-deny) clean; deployment (Lavapipe CI fallback) verified; docs complete. |

---

## 3. Stabilize-Interfaces-First Sequencing (the unblock plan)

**Gate G0 — Interface Freeze (end of P0).** Until G0 passes, *only the Interface Work-Stream runs*. All other sessions are blocked. G0 deliverables:

1. **`celnet-types`** — POD, `Copy`, no IO: `Ccy`, `CcyPair` (FOR=base/CCY1, DOM=quote/CCY2), `Tenor`, `Money`, discount factors `Df`, `Vol`, `Delta`, `Strike`, scalar policy (`f64` CPU canonical / `f32` GPU), enums for `DeltaConvention {SpotUnadj, FwdUnadj, SpotPremAdj, FwdPremAdj}`, `AtmConvention {Atmf, Dns}`, `PremiumStyle {DomPips, For%, Dom%, ForPips}`, `Cut {NY1000, Tokyo1500}`, `DayCount {Act365, Act360}`, `Settlement {Deliverable, Ndo}`. **Frozen first — everything depends on it.**
2. **`celnet-core` traits** — `PricingModel`, `VolModel`/`SmileModel`, `PricingBackend { simulate_paths, reduce_payoff, solve_pde }`, `Calendar`, `CalibrationTarget`, plus the `assert_close` ULP/rel/abs float helper and the scalar abstraction. Trait *shapes* must match what both the trait-object registry and the wasm host implement (so first-party and user plugins are interchangeable).
3. **`celnet-proto`** — single current wire contract (prost 0.13 / tonic 0.12); **no** `schema_version`, no version negotiation (CLAUDE.md rule 9). Evolution discipline (`docs/INTERFACES.md`): change the contract **and every dependent in one PR**; no old reader ever decodes a new message, so no reserve/renumber ceremony.
4. **`celnet-plugin-api`** — the SDK trait surface + WIT `world` definition for wasm plugins; semver-tagged as a *product*.

**Rule:** Any later change to a frozen interface crate is a **dedicated interface PR**: edit the interface crate **and every dependent in the same change**, update `docs/INTERFACES.md`, re-index the memory graph, and announce in `CLAUDE.md`. There is no N/N-1 compat gate (single current contract — rule 9). Implementation sessions rebase, never edit interface crates ad hoc.

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
- **Deliverables:** **two-tier** — (1) fast VV for 1st-gen (one-touch, no-touch, DNT, single/double barriers, European digitals) with survival-probability/first-exit-time weighting and `[0,notional]` clamps; (2) **LSV** (Heston backbone + Dupire leverage L²=σ_Dupire²/E[v|S], **particle-method** calibration, exposed mixing weight η) for booking/Greeks/2nd-gen (window/partial barriers, TARFs/accumulators, Asians, lookbacks, forward-starts). PDE: Crank-Nicolson + **Rannacher** start-up, ADI Craig-Sneyd / Hundsdorfer-Verwer for 2D LSV, log-spot grids, barrier-aligned nodes. MC: **Andersen QE** for CIR variance (full-truncation when Feller violated), **Philox** counter-based RNG, geometric-Asian/vanilla control variates, **BGK 0.5826·σ·√dt** barrier correction. **Sobol' QMC + Brownian-bridge dimension ordering are Built (CPU-first) in the `celnet-qmc` crate** (Joe-Kuo Sobol' + Owen scramble + principal-bisection bridge; parity row `celnet-parity/tests/qmc.rs`). Variance swap via log-contract (1/K² strip); vol swap with explicit convexity adjustment. **STATUS — all Built:** the full two-tier engine ships: 1st-gen VV touches/barriers/digitals (`celnet-exotics::{touch,barrier,digital}`), LSV calibration + ADI PDE (`celnet-exotics::{lsv,leverage,particle,adi,stochvol}`), TARF/accumulator (`celnet-exotics::{tarf,accumulator}`), Asians (`asian`), lookbacks (`lookback`), forward-start/cliquet (`forward_start`), quanto (`quanto`), variance/vol swaps (`var_swap`/`vol_swap`) — each a gated `celnet-parity` row (`exotics.rs`, `lsv.rs`, `structured.rs`, `asian.rs`, `forward_start.rs`, `var_vol_swap.rs`).
- **Depends on:** G2 (needs arbitrage-free smile for Dupire); coordinates with WS-E via the `PricingBackend` trait (frozen at G0).
- **Gates:** cross-method validation (VV vs LSV-PDE vs LSV-MC) within tolerance; barrier/touch reconciled to broker DNT/one-touch as SV-mixing target; geometric-Asian closed-form vs MC control variate; explicit TARF gap-risk stress test; Rannacher-vs-naive oscillation regression on digitals.

### WS-E · GPU Acceleration
- **Owns:** `celnet-gpu`.
- **Deliverables:** a **wgpu 29 + WGSL** backend (Metal/Vulkan/DX12/GLES) implementing `PricingBackend`; **f32 on GPU, f64 CPU reconciliation** path; **Philox-4x32-10** counter-based RNG (WGSL kernel) seeded from (global_seed, path_index, step, dim); FD stencil kernels (tiled shared-memory; PCR/cyclic-reduction tridiagonal for implicit); runtime adapter probing → CPU fallback when no GPU; integer-atomic / tree-reduction payoff accumulation (no float atomics in WGSL). **Sobol' QMC + Brownian-bridge dim assignment are Built (CPU-first) in `celnet-qmc`** with the integer Sobol/scramble core exposed verbatim for on-device reuse; the GPU on-device QMC kernel itself remains the documented next GPU increment.
- **Depends on:** G0 (`PricingBackend` trait + scalar policy). Can run **in parallel with WS-D** because both target the same frozen trait; integrate at G3.
- **Gates:** GPU f32 vs CPU f64 reconciliation within documented error bound on vanillas; Philox bit-stability across backends; Lavapipe software-Vulkan CI path exercises the same wgpu code; pinned NVIDIA driver/toolkit container smoke test. *(Sobol' convergence gate now Built (CPU-first) — measured ≥3× variance reduction in `celnet-parity/tests/qmc.rs`; the on-device Sobol' kernel gate lands with the GPU QMC increment.)*

### WS-F · Engine & Hot Upgrade
- **Owns:** `celnet-engine`.
- **Deliverables:** two-tier architecture — non-async, **core-pinned** (`core_affinity` + isolcpus/nohz_full/rcu_nocbs) busy-poll hot path for pricing/risk; **zero-alloc** (pre-allocated pools, arrayvec/smallvec/heapless); `rtrb` SPSC ring buffers between hot path and async edge (not channels); `arc-swap` for hot-reloadable reference data, `seqlock` for single-writer price snapshots, `CachePadded` on shared atomics; default global allocator (tuned allocator deferred, benchmark-gated); **zero-downtime upgrade** via SO_REUSEPORT graceful socket handoff + drain + single-current-contract live-state transfer (rkyv over UDS/shared memory); full-book Greeks + spot/vol stress recompute.
- **Depends on:** G1 (vanilla), G3 ideally for full wiring; can scaffold hot-path + handoff against trait stubs after G0.
- **Gates:** HdrHistogram p99/p99.9 budgets met; zero-alloc assertion in hot loop (counting-allocator guard in tests); **blue-green single-version state-handoff** test (no dropped/corrupted state across cutover; no mixed-version window); no synchronous logging on hot path (telemetry over bounded queue to non-critical core).

### WS-G · Plugin Host & SDK
- **Owns:** `celnet-plugin-host`; co-owns `celnet-plugin-api` *contract evolution only* via interface PRs.
- **Status: DONE.** Tiered host built behind the frozen `celnet-plugin-api` contract; all four
  gates green; the wasmtime blocker is closed. See `docs/PLUGIN-HOST-ALT.md` (ADR + evaluation).
- **Deliverables (shipped):** **`wasmi 1.0.9`** (pure-Rust, fuel-metered interpreter — replaces
  the originally-planned wasmtime, which is blocked by open 2026 RustSec advisories) embedding
  **core Wasm modules** (not the Component Model — wasmi CM is WIP); a no-WASI capability
  `Linker` exposing **only** the explicit libm `celnet_core::math` pricing primitives (zero
  ambient authority — no clock/RNG/threads/fs/net); **deterministic execution** —
  `Config::consume_fuel(true)` with a per-call `FuelBudget` as the compute SLA (exhaustion ⇒
  typed `HostError::FuelExhausted`, never a hang), NaN-canonicalization on every boundary value,
  a host-controlled `(ptr,len)` core-module ABI marshalling the Copy-POD `VanillaInputs`→`Greeks`
  records; a unified `ModelRegistry` routing **Tier-0 native** (`dyn PricingModel`) and **Tier-2
  wasm** models identically via the tier-blind `HostModel` seam; a deterministic **replay
  harness** (fixed snapshot → `to_bits`-identical price/Greeks across runs, cross-platform via
  the libm requirement). The Tier-1 trusted-partner native `.so`/`.dylib` path (via `stabby`
  72.x behind code-signing + allowlist; abi_stable is unmaintained — not used) and the optional
  Tier-3 Landlock/seccomp Linux ring are designed in `docs/PLUGIN-HOST-ALT.md` §4 and not yet
  wired.
- **Depends on:** G0 (`celnet-plugin-api` WIT world + traits).
- **Gates (all passing):** capability-denial (a module importing anything outside the granted
  set fails to load); fuel-exhaustion bounded-runtime (an infinite-loop guest traps within
  budget, proven under a watchdog timeout — never hangs); replay determinism (bit-identical
  `to_bits` across runs); Tier-0 == Tier-2 interchangeability (a native and a wasm twin of one
  trivial pricer route through one registry and agree to the bit).

### WS-H · CelNet Estate Integration & Feeds
- **Owns:** `celnet-integration`.
- **Deliverables:** adapter to the in-process JVM **distributor** (bounded disruptor mailbox, skip-while-full) via JVM adapter or `DistributorProducerChannelHandler` socket protocol — **decision recorded as ADR** before build; sized mailboxes + rate-limiting so a high-frequency option pricer cannot cause silent price drops; resilient `MarketMerchantPriceService` WS consumer (handle disconnects, respect ~6 concurrent HTTP conns/domain semaphore); vol-data feed handler (verify marketdata-api can supply vol surface/Greeks input or provision additional feed); **FMD FXO 2.0-style surface adapter** normalizing vendor ATM/25d&10d RR/BF + spot/fwd/NDF into the canonical surface object; new-`OptionProduct`-type touch-list tracked as a cross-cutting ADR (celnet-type, staticdata, api proto enums, positionmanager netting keys, risk exposure models, destination FIX dialect).
- **Depends on:** G0 (`celnet-proto`), WS-C (surface object) for the FMD adapter.
- **Gates:** back-pressure test (mailbox full → bounded, no corruption); WS-reconnect resilience test; FMD-feed normalization round-trip snapshot; cross-service dependency map maintained **manually** (lodestar finds zero cross-service edges for in-proc distributor/Protobuf/FIX) and verified against Spring config + distributor `notifyUsers` names.

### WS-I · Edge Binaries
- **Owns:** `celnet-server`, `celnet-cli`.
- **Deliverables:** thin tokio async edge — `tonic` ~0.13 + `prost` ~0.13 gRPC, WebSocket streaming RFS, admin CLI/diagnostics; connects to hot path only via `rtrb`; `/readyz` + connection-drain for LB/eBPF reuseport steering during upgrade.
- **Depends on:** G3 + WS-F engine surface.
- **Gates:** integration smoke (price round-trip over gRPC + WS); drain/readiness behavior under simulated rolling restart.

### WS-T · Test/CI/Determinism Backbone (cross-cutting, low-conflict)
- **Owns:** CI workflow files, `deny.toml`, `supply-chain/`, the QuantLib golden-table generator harness, fuzz targets directory, mutation/coverage config. *Edits config + test-only files; does not edit other streams' `src/`.*
- **Deliverables:** layered pyramid — golden vs **QuantLib 1.42.1 (version-pinned) frozen tables** in `celnet-golden` (the implemented numerical oracle; Reiswich-Wystup / Clark are the convention references checked manually, not test fixtures), invariant tests, **proptest** (Strategy generators for valid market inputs; commit `proptest-regressions/`), **cargo-fuzz** (Arbitrary → adversarial inputs; Linux nightly job only; commit corpus/crashes), `insta` snapshots (CI=1 fails stale), criterion/divan perf gates with committed baselines; `cargo-llvm-cov nextest` fail-under (≥90% core); `cargo-mutants` (incremental on PR diff, full scheduled on main); `cargo-deny` + `cargo-vet` per PR; MSRV-pinned job.
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

## 6. Cross-Cutting ADRs (record before building, in `mcp__lodestar__manage_adr` and `docs/INTERFACES.md`)

1. **Async runtime split** — tokio edge (default, for tonic/hyper ecosystem) vs non-async core-pinned hot path; monoio/glommio explicitly rejected for the engine core (Send/Sync ecosystem loss, maintenance/io_uring security risk) unless a measured IO-bound workload justifies a prototype.
2. **Allocator choice** — mimalloc (small-alloc tail latency) vs tikv-jemallocator (multi-thread throughput); benchmark on real workload, gap small in steady state.
3. **Distributor integration mechanism** — JVM adapter vs `DistributorProducerChannelHandler` socket protocol; back-pressure sizing.
4. **GPU numeric policy** — f32 GPU canonical + f64 CPU reconciliation; no f64-on-GPU promise (Metal/WGSL lack native f64).
5. **Plugin trust tiers** — wasm (untrusted, default, sandboxed+fuel) vs trusted-first-party native (stabby + signing); abi_stable banned.
6. **New `OptionProduct` type rollout** — enumerate the full touch-list (celnet-type, staticdata, proto enums, positionmanager netting keys, risk exposure, FIX dialect) as a coordinated breaking change.

---

## 7. Per-Session Operating Protocol

Every Claude session, on every pickup, follows this loop:

### 7.1 Pick up work
1. `git pull`; read `CLAUDE.md` (the ledger) and `docs/INTERFACES.md`.
2. Verify **G0 is green**. If not, and you are not WS-0, **stop** — only interface work proceeds pre-G0.
3. Choose an **unclaimed** work-stream from the ledger whose dependency gate is green. Claim it by editing the ledger (see §7.3) and committing/pushing that claim *first* (atomic claim to prevent two sessions taking the same stream).
4. Confirm your stream's crates are **disjoint** from every other `IN-PROGRESS` claim. If overlap, pick another stream.

### 7.2 Do work
5. Work **only inside your owned crate(s)**. Never edit a frozen interface crate directly — if you need an interface change, open a dedicated **interface PR** (edit the interface crate + every dependent in one change, update `docs/INTERFACES.md`, re-index the memory graph, announce in ledger) and pause dependent work until it lands. No N/N-1 compat gate (single current contract — rule 9).
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
Also append to a **decision log** section any ADRs, interface bumps (with semver + tag), and gate transitions (G0→G1 etc.). lodestar auto-indexes; run `detect_changes` to confirm scope after structural changes so the manual cross-service map stays accurate.

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

---

## 9. Hierarchical Risk Aggregation (new backlog — 2026-05-31)

> **New workstream added 2026-05-31.** Design spec: `docs/RISK-HIERARCHY.md`. Celnet today
> prices and risks at the **single-instrument** level (`celnet-vanilla` 14-Greek set;
> `celnet-engine` `BucketedRisk` = vega by `tenor×delta`, cross-gamma, theta-roll). The layer
> **above** a single position — a firm-wide, convention-normalized, multi-dimensional risk cube
> that rolls position risk up the org hierarchy (trader→book→desk→ccy-pair→booking-location→legal-
> entity→firm), nets it correctly across conventions and numeraires, enforces limits at every node,
> scopes by entitlements, and streams to the trader/risk UI at sub-second cadence — does **not yet
> exist**. This section is the backlog for that layer. It **appends** to the roadmap and does not
> renumber or alter §0–§8.
>
> **Honesty flags (carried from `docs/RISK-HIERARCHY.md`):** (a) the firm-hierarchical *scale*
> claim depends on **AAD / batched-GPU**, not bump-and-revalue, for the non-additive measures
> (VaR/ES, FRTB curvature, correlation-weighted vega) — this is the top technical risk; (b) the
> `celnet-router` HRW/fleet scale-out tier is **designed only, not built** (`docs/SCALE-OUT.md`),
> so the distributed-aggregation tasks below target a not-yet-shipped substrate; (c) the hot core
> stays unchanged — aggregation runs **off-core** over the existing bounded SPSC offload seam.

### WS-R · Hierarchical Risk Aggregation, Limits & Entitlements

- **Owns (new crates):** `celnet-risk-cube`, `celnet-risk-normalize`, `celnet-limits`,
  `celnet-entitlements`. Co-owns (interface PRs only): the risk dimension/hierarchy value types in
  `celnet-types`; the streaming-risk messages in `celnet-proto`; the risk-explorer/limit-dashboard
  views in `gui/` (coordinated with the GUI workstream — **do not edit `gui/` from this stream
  without coordination**).
- **Depends on:** G3 (needs `celnet-engine` `BucketedRisk` leaves + `celnet-surface` for scenario
  re-pricing). Aggregation/limits/entitlements crates can scaffold against trait stubs after G1.
- **Dependency direction:** new crates sit **above** `celnet-engine`, depend on `celnet-types` +
  `celnet-core` (+ `celnet-surface` for scenario reval), and feed `celnet-server`'s streaming edge.
  Never invert toward the hot core.

#### Tasks

1. **Risk dimension/hierarchy model in `celnet-types`** — POD value types `PositionId`, `TraderId`,
   `BookId`, `DeskId`, `LocationId`, `EntityId`, `ValueDate`, `TradingSession`, a `DimensionId` enum,
   and parent-pointer hierarchies (`Book→Desk`, `Location→Entity→Firm`); the immutable `RiskFact`
   leaf (dimension keys + canonical measures + `surface_version`). *Crate: `celnet-types` (interface
   PR). Dep: G0 (extends a frozen crate — coordinated change).*
2. **Convention-normalized risk netting** — `celnet-risk-normalize`: re-derive every position's risk
   into one canonical internal convention (proposed: spot-unadjusted, premium-excluded, premium as a
   separate line) before aggregation; pure deterministic transform over `celnet-vanilla` outputs,
   reusing the strike↔delta machinery. *Crate: `celnet-risk-normalize`. Dep: G1 (`celnet-vanilla`).*
3. **Common-numeraire conversion** — resolve delta into a per-currency exposure vector + reporting-
   ccy spot conversion; normalize vega P&L from per-position premium-ccy to reporting ccy (couples
   to task 2 — vega needs the same premium-ccy normalization as delta). *Crate: `celnet-risk-
   normalize`. Dep: task 2.*
4. **Cross-pair delta triangulation + signed cross-pair correlation matrix** — net delta at the
   currency-node level (USD legs of EURUSD/EURJPY cancel); store a **signed** cross-pair correlation
   matrix with an explicit **quote-convention tag** so the vol-triangulation sign is resolved at
   netting time (never a global minus). Roll up **vanna/volga** via the same matrix. *Crate:
   `celnet-risk-normalize` + `celnet-risk-cube`. Dep: task 3.*
5. **Risk-aggregation engine (the OLAP cube)** — `celnet-risk-cube`: immutable fact store; **incremental
   additive roll-up** (a trade touches O(depth) ancestor sums); **per-node re-derivation** of
   non-additive measures (VaR/ES, FRTB curvature, correlation-weighted vega); group-by/reduce along
   any dimension subset at any level; drill-down reconciles to constituents. *Crate: `celnet-risk-
   cube`. Dep: tasks 1, 4.*
6. **Recompute-trigger + AAD/batched-GPU reval** — non-additive measures recompute on a delta-driven
   + throttled trigger (not every tick); scenario/VaR/curvature reval via **adjoint AD and/or batched
   GPU** (`celnet-gpu`, Philox, f32-GPU/f64-CPU reconciled) — the throughput requirement that makes
   the firm-hierarchical claim real. *Crate: `celnet-risk-cube` + `celnet-gpu`. Dep: task 5; G3.*
7. **FRTB-SA sensitivities export** — map cube nodes to FRTB SbM delta/vega/curvature per risk-class→
   bucket→factor; run the inter-bucket reduction under the **three correlation scenarios** (×0.75/1.0/
   1.25, take max); RRAO (1.0 % exotic / 0.1 % other); treat reg weights/pillars (SIMM v2.8, FRTB
   vertices) as **versioned external data, never compiled-in**. DRC noted immaterial for vanilla FX.
   *Crate: `celnet-risk-cube`. Dep: task 5.*
8. **Entitlements service** — `celnet-entitlements`: role grants scoped to dimension subtrees;
   **server-side pruning before aggregation** (no aggregate leakage); information barriers as deny
   rules; separation-of-duties scopes; every decision audited via `celnet-observability`. *Crate:
   `celnet-entitlements`. Dep: task 5.*
9. **Limits & breach service** — `celnet-limits`: limit tree cascading board→entity→desk→book→trader;
   types (Greek incl. vanna/volga, bucketed-vega/pin-risk, concentration, VaR/ES, scenario, stop-loss);
   soft vs hard; utilization; **pre/at-trade multi-node check** on the µs additive-Greek path; breach/
   escalation (suspend/hedge/block, four-eyes). *Crate: `celnet-limits`. Dep: task 5; G3.*
10. **Streaming risk wire contract** — extend `celnet-proto` with the multiplexed streaming-risk
    subscription (per-node Greeks/bucketed-vega/limit-utilization deltas + snapshots), entitlement-
    scoped; `surface_version`-pinned (live vs IPV-official lens). *Crate: `celnet-proto` (interface
    PR) + `celnet-server`. Dep: tasks 5, 8; WS-I.*
11. **Real-time aggregation over scale-out** — partition facts by `(legal_entity, ccy_pair)` over the
    `celnet-router` HRW map; shard-local roll-up + cross-shard reducer (combine additive measures;
    re-derive/gather non-additive at firm level); IPV/joint-portfolio off the hot shard. *Crate:
    `celnet-risk-cube` + `celnet-router`. Dep: task 5; **`celnet-router` is designed-only — blocked
    until the fleet tier is built**.*
12. **Point-in-time / official-vs-live lens** — `surface_version`-stamped facts aggregated under the
    live, IPV-official, or any historical surface; deterministic-replay "what did the desk see at
    14:32" reconstruction; PLA (Spearman > 0.80 & KS < 0.09 — **not** the removed 2016 ratio tests)
    and RFET/NMRF hooks. *Crate: `celnet-risk-cube` + `celnet-observability`. Dep: task 5.*
13. **GUI risk-explorer / limits-dashboard** — virtualized server-side-row-model tree grid (O(log n)
    streaming sort), vega-by-tenor×delta + signed cross-pair correlation heatmaps (perceptual/colorblind-
    safe ramps), Greeks blotter with vanna/volga ladder, RAG limit dashboard, scenario tornado/what-if,
    P&L-explain waterfall (higher-order vanna/volga + residual), session-aware + point-in-time replay.
    *Area: `gui/` (coordinate with the GUI workstream — do not edit `gui/` unilaterally). Dep: tasks
    10, 12.*
14. **Celnet-specific risk latency budget + CI gate** — publish and gate Celnet's **own** incremental
    pre-trade Greek/utilization-check budget (low-single-digit µs additive path; scenario reval on the
    trigger cadence) — do **not** borrow the B2BITS FIX-stack ~4 µs figure; aim to be first to publish
    a portfolio-risk-roll-up latency/throughput number. *Crate: `celnet-bench` + WS-T. Dep: tasks 5, 9.*

### Out of scope (explicitly, for honesty — future backlog, not silently dropped)

- **XVA / SA-CVA** sensitivities (own delta/vega buckets distinct from market-risk SbM).
- **FX settlement / Herstatt / CLS PvP** risk as a capital line (the `value_date` axis is carried;
  the settlement-capital treatment is deferred).
- **DRC** beyond noting it is immaterial for vanilla FX (no issuer-default leg).

> Add a `WS-R` row to the **Work-Stream Ledger** in `CLAUDE.md` when this stream is claimed; keep
> `docs/RISK-HIERARCHY.md` and `docs/INTERFACES.md` in sync as the dimension model and streaming-risk
> contract land.

---

## 10. Experience Architecture & Trading-Universe Scale (new backlog — 2026-05-31)

> **The product-experience design corpus.** A trader-grounded, IB-scale critique & redesign of the
> whole front-end and the capabilities behind it, produced 2026-05-31:
> - `docs/EXPERIENCE-ARCHITECTURE.md` — **the authoritative, reconciled phased backlog** (Phase 0/1/2)
>   and the unified IX: navigation = **Scope (toolbar) × View (rail) × Analytics (inspector)** over one
>   position-fact cube; entitlement-aware drill-down (show-all now, entitlement-ready); Book↔Risk =
>   same cube at two zooms; analytics selection via the plugin `ModelRegistry`; the `TrendMode` spec.
> - `docs/TRADING-UNIVERSE-SCALE.md` — pair universe & liquidity tiers, the continuous expiry axis
>   (broken/IMM/event dates), the vol **cube**, who-is-trading/attribution, scale architecture, scale UX.
> - `docs/SURFACE-WORKFLOW.md` — the optimal marking workflow; **editable delta×tenor grid is primary,
>   3-D mesh demoted**; working-vs-official + publish/version; broken-date/event pricing.
>
> **API-first client parity (governing rule).** Every capability below lives in the **one canonical API**
> (`celnet-proto` + `celnet-server`); the GUI uses the *same* API as any client, and a feature is "done"
> only when **API + SDK (`celnet-client`) + Excel (`CELNET.*`) + GUI + `docs/INTERFACES.md`** are all
> consistent. No GUI-only computation of a capability the API doesn't expose (e.g. the Book view's
> current client-side aggregation must become a server-side aggregate-risk API). See the
> `api-first-client-parity` auto-memory.
>
> **Phase 0 (implementable on today's data)** — surface mismark fix (transport the edit; Re-mark ≠
> Publish), wire the real calendar-arb gate (kill hardcoded `calendarArbitrageFree:true`), editable
> ATM/RR/BF handles, labelled "Premium" `TrendMode` (stop the cross-structure blend), Book→Risk drill,
> `ScopeContext` showing-all breadcrumb, analytics/model selector where the engine already supports it,
> Σ/Mid column clarity, surface-chart fixes, ticket-rate fix, stop workspace remount, ON/TN/SN labels.
> **Phase 1** = new feeds/contracts (market-series for non-Premium trends; tenor/IMM/event/broken-date
> model incl. the **ON-resolves-as-SN** fix in `celnet-calendar`; consensus surface; attribution
> identity; surface contract evolutions; pair-universe registry). **Phase 2** = scale/infra
> (virtualised blotter + server-side aggregation; vol-cube store + dirty recalibration; cross-fleet
> fan-out; `celnet-risk-cube`/`-normalize`/`-limits`/`-entitlements`; AAD/GPU Greeks; reg-data feed).
>
> **Honest headline gaps** (full list in `docs/EXPERIENCE-ARCHITECTURE.md` §9): the firm-scale
> aggregation backend and scale-out tier are **proposal/designed-only**; the hierarchical-scale claim
> hinges on **AAD/batched-GPU** replacing bump-and-revalue; Celnet must publish its **own** roll-up
> latency budget (the ~4µs figure is FIX-stack, not options-reval). The buildable-now seam (`ScopeContext`,
> Book→Risk drill) degrades honestly to show-all over seeded positions in native units.
>
> Interface-crate changes (`celnet-types`/`celnet-proto`/`celnet-conventions`) in Phase 1 must follow the
> parallel-session interface-crate coordination discipline (§4 / §7), not be changed unilaterally.

---

## 11. Corporate-Action Dynamic Pricing & Inventory-Skew Reconciliation (new backlog — 2026-08-12)

> **Two externally-supplied specifications reconciled against as-built code.** Both turned out to
> overlap shipped Celnet features substantially, so both produced **gap analyses, not greenfield
> designs** — writing a fresh spec for a built feature creates a second, drifting source of truth
> (guardrail 10). Every "already exists" claim in both docs is `file:line`-cited from code that was
> read, not inferred.
> - [`docs/archive/audits/CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md`](archive/audits/CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md) — **stages 1–3 of the source pipeline and 3 of
>   its 4 event families are already built** (`celnet-corpactions` CAEV model + hand-verified effect
>   math; `celnet-refstore` journal-backed bitemporal golden store with the vendor-neutral
>   `CorpActionSource` port; `CorporateActionsService` + GUI workspace). **7 real gaps**, headed by
>   **G1: an applied corporate action never reaches the live pricer** —
>   `gov_bond_to_instrument_def` (`config/reference_data.rs:859-895`) still reads the static
>   `GovBondSpec`, so the "dynamic bond pricing" half of the spec is entirely absent. Also: no
>   YTW/YTC anchor switch (and no static call/put schedule to switch onto), no pool factor in the
>   pricing crates, no consent-fee event shape, no valuation of an exchange target, no
>   per-instrument quote lock (only the firm-wide two-boolean `PricingControl`), no ex-date-aware
>   accrued. Phases **CA-P1…CA-P7**. Amends [`docs/fixed-income/BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md`](fixed-income/BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md)
>   (whose "No code yet" status and P1–P5 phase list are now corrected in place).
> - [`docs/hedging/INVENTORY-SKEW-ENGINE-GAP-ANALYSIS.md`](hedging/INVENTORY-SKEW-ENGINE-GAP-ANALYSIS.md) — **4 of 5 components, both operational modes and
>   half a guardrail are already live**: `celnet-tiering`'s `FeaturePipeline` *is* the spec's layer
>   between core pricing and distribution, applied on ESP (`aggregation.rs:1334`) **and** RFQ
>   (`:1193`); `InventorySkew` is Mode A over live `InventorySource` inventory; `PricingFeature::Axe`
>   with `AxeSide::{Buy,Sell}` is Mode B; `SpreadUnit::YieldBps` covers the yield-space formulation.
>   **5 real gaps**, headed by **S1: the anti-arbitrage cap is static, not the spec's dynamic
>   "never exceed half the bid-ask"** — a mis-set `s_max > h` silently permits a **through-mid**
>   quote (the quote never crosses itself, since `offer − bid = 2h` is skew-invariant, so this is
>   silent). Also: skew keys on raw inventory rather than limit utilisation, no per-issuer
>   aggregation, no dedicated/per-instrument axe, no explicit post-fill fade. Phases **SK-P1…SK-P6**.
>
> **Two ADRs required before building.** (1) *Skew-band authority* — whether
> `celnet-hedge-routing`'s RAG band owns the skew decision with `celnet-tiering` executing it, or
> the two stay independent. Today they read the same live inventory **independently** (the hedge
> engine has a `SKEW` action; the tiering skew runs continuously and is not gated by any band), so
> a desk running both **is** running two uncoordinated responses to one signal — a live
> configuration hazard, documented in the skew doc §5. (2) *YTW as valuation anchor* — whether the
> anchor auto-switches or YTW is reported alongside YTM; this changes what a trader sees on a live
> quote and must not be decided silently. SK-P1 additionally needs an ADR because it changes live
> outbound prices for any group configured with `s_max > h`.
>
> **Guardrail conflicts in the source specs — resolved in the docs, never copied.** The CA spec
> names Bloomberg CACS / Refinitiv Event Streams as *the* feed (guardrails 7+8 — resolved: the
> purpose-named `CorpActionSource` port already exists with the open `GovvieSource` first-class; a
> licensed feed is a customer-wired adapter, vendor names in integration prose only); keys its
> quote lock on CUSIP (guardrail 7 — CUSIP *data* is licensed; key on internal `instrument_id`);
> implies a synchronous ingest→pricer chain (guardrail 11 — ingest stays on the async edge, the
> pricer reads a resolved versioned definition). The skew spec names Bloomberg/Tradeweb/MarketAxess
> as distribution channels (same posture) and a standalone "Skew Engine" component (guardrail 10 —
> **extend `celnet-tiering`; a parallel engine must be rejected**). **Neither spec engages guardrail
> 9** (no versioned API proposed) **or guardrail 2** (no mock mandated). One source requirement is
> **rejected on correctness grounds**: the CA spec's "reset accrued interest to zero on ex-date" is
> not market-correct — several conventions trade with *negative* accrued in the ex-dividend period,
> so the requirement is "ex-date aware", validated against QuantLib (guardrail 5), not "reset to
> zero".
>
> **Recommended build order across both docs:** **SK-P1** (dynamic anti-arb cap — smallest change,
> real money risk, prevents silent through-mid quotes on a live outbound path) → **CA-P1** (wire the
> applied CA into the pricer — makes the already-built corpactions/refstore stack actually matter,
> and retires an overclaiming server doc comment that currently asserts this works) → **SK-P2**
> (utilisation-driven skew — closes the Mode-A gap and the double-count hazard in one move rather
> than adding a second uncoordinated skew). Two items are explicitly **measure-before-build**
> (SK-P6 post-fill fade, given the existing 5 ms repricing loop) or **candidate out-of-scope**
> (CA-P7 component valuation, which needs a cash-equity instrument type and price source that do
> not exist).
>
> ### Production Remediation Status (September 2026 — BUILT & VERIFIED)
> All recommended build-order items and essential operational controls are **fully implemented and verified**:
> - **SK-P1 (Dynamic Anti-Arb Skew Cap)**: Built in `celnet-tiering` with $|s| \le \min(s_{\max}, \lambda \cdot h)$.
>   Verified in `tests/tiering.rs` and `tests/feature_pipeline.rs`. Eliminates through-mid quotes.
> - **SK-P2 (Limit-Utilization Inventory Skew)**: Built in `celnet-tiering` with $u = q / \text{limit\_cap}$
>   and power damping, resolving the auto-hedging double-count hazard.
> - **CA-G6 (Lock-Free Quote Lock)**: Built in `celnet-server::PricingControl` using `arc_swap::ArcSwap`
>   for zero-alloc, wait-free loads on the pricing tick loop, wired to FIX/RFS quote emission and streaming.
> - **CA-P1/CA-P2 (Pool Factor & Corporate Actions)**: Built in `celnet-bond` (`Bond.pool_factor`,
>   `CashflowSchedule` scaling) and wired to `BondDef` in `reference_data.rs` and `aggregation.rs`.
> - **RFS Futures Streaming**: Built in `celnet-fix` with `SEC_TYPE_FUT` and `PRODUCT_FUT`, wired to
>   `celnet-server` RFS lines and `NewOrderSingle` execution.
> - **Notification SDK Parity**: Built in `celnet-client` and `celnet-cli` with wire tag 7
>   `MANUAL_INTERVENTION_REQUIRED` and exhaustive pattern matching.
>
> See `docs/architecture/CELNET-PRODUCTION-GRADE-ARCHITECTURE-REVIEW-AND-IMPLEMENTATION.md` and
> `docs/CELNET-PRODUCTION-GRADE-ARCHITECTURE-REVIEW-AND-IMPLEMENTATION.html`.

---

*End of roadmap. Source of truth for live status is `CLAUDE.md` (the ledger); source of truth for contracts is `docs/INTERFACES.md`; FX convention spec is `docs/CONVENTIONS.md`.*