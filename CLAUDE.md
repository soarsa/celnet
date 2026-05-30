# Celnet — Claude Operating Guide

State-of-the-art FX **Options** pricing platform in Rust. Ultra-low-latency, scalable,
mission-critical, hot-upgradable; integrates into the Celer trade-lifecycle estate and
front end, consumes external vendor FX-options market-data feeds, and exposes
user-extensible analytics via SDKs.
Greenfield, started 30 May 2026.

## Hard guardrails (non-negotiable)

1. **Git is local-only.** Never `git push`, never add a remote. A deny rule in
   `.claude/settings.json` enforces it. Commit locally freely.
2. **No mocks, no placeholders, no `todo!()`.** Only 100% complete, state-of-the-art
   implementations. If scope can't be finished, narrow it — never fake depth. Split large
   implementations across files/crates instead of abbreviating.
3. **codebase-memory-mcp first** for code discovery (`search_graph`, `trace_path`,
   `get_code_snippet`, `query_graph`, `get_architecture`, `detect_changes`, `manage_adr`);
   fall back to Grep/Read only for non-code text. The graph **auto-indexes** (git post-commit
   hook + a Stop hook in `.claude/settings.json`, both running `codebase-memory-mcp cli
   index_repository … mode:fast`), so its scope always covers new files — no manual
   re-indexing needed. Use `detect_changes` to scope builds/tests; keep ADRs current via
   `manage_adr`. Saves tokens, stays exact, never forgets.
4. **LSP for code intel.** Use the LSP tool (rust-analyzer) for goToDefinition,
   findReferences, hover, document/workspace symbols, call hierarchy — not guesswork.
5. **Every change passes the gates** before it's "done": `just check` (fmt, clippy -D
   warnings, nextest, cargo-deny). Numerical code is validated against references
   (QuantLib / published prices), never merely asserted plausible.
6. **Scale & performance are requirements, not afterthoughts.** Every component must scale
   to **investment-banking-sized portfolios** and stream prices to **high-performance
   counterparties** at the latency/throughput budgets in `docs/ARCHITECTURE.md` §1.2.
   Design for horizontal scale-out and many-instrument/many-tenor batch from day one; pick
   algorithms and data structures accordingly. **Leverage the latest academic research** to
   optimize and scale wherever it helps — cite the paper/method in code comments and in
   `docs/`.
7. **No commercial products — anywhere.** Use only open-source, permissively-licensed
   (MIT/Apache-2.0/BSD/etc.) software and free, academically-grounded methods. No paid
   libraries, no proprietary commercial SDKs/solvers/data products (e.g. no Intel MKL,
   no commercial market-data terminals as runtime deps). QuantLib (open-source) as the
   golden oracle is fine. Any unavoidable proprietary-but-free toolkit (e.g. the CUDA
   toolkit behind CubeCL) is recorded as an ADR with the open fallback (wgpu/Vulkan) kept
   first-class. `cargo-deny` license policy enforces the OSS license set.
8. **Naming is `celnet`-logical & vendor-neutral.** Every product artifact (crate, module,
   type, trait, fn) is named for its **purpose** under the `celnet-` namespace. **No**
   commercial-product / competitor / vendor names (Bloomberg, Fenics, Synoption, Murex,
   Numerix, QuantLib, MKL, …) and **no** person/paper/framework names in API identifiers —
   e.g. inputs are `VanillaInputs`, not `GkInputs`. Mathematical-method provenance may appear
   in doc comments only, never in names. (The parent firm **Celer** and its `celertech`
   estate are our own systems; their names are fine in integration/docs context, never in
   core product identifiers.)
9. **No versioned APIs.** We have no external users — there is exactly **one clean, current
   contract**. No `schema_version`, no N/N-1 negotiation, no back-compat shims. Evolve and
   refactor freely; upgrades deploy a single uniform version (no mixed-version window).
10. **Always refactor to cleanest; zero legacy.** Continuously delete dead code, keep files
    in the correct crate/dir, and keep **all** docs/guides/references in sync as code evolves
    — no stale or duplicate references anywhere. After any structural change,
    re-`index_repository` so the codebase-memory graph always covers the full scope.
11. **Trader-centric API, zero-cost observability, scale-out aware.** Ship evolving client
    SDK(s) and design the API by exercising **real-like trader/GUI/API-user workflows** as
    tests; evolve the single current contract toward the cleanest ergonomics. Instrument for
    mission-critical ops (structured logging, tracing, metrics, HdrHistogram p50/p99/p99.9)
    **without diminishing performance** — the pinned zero-alloc hot core stays
    log/lock/alloc-free; telemetry offloads over a bounded queue. Continuously assess
    horizontal/distributed scale-out vs the latency/throughput budgets (`docs/SCALE-OUT.md`).

## Running the toolchain

The shell does **not** persist env between Bash calls and rustup is not on the default
PATH. Prefix every Rust command:

```bash
source "$HOME/.cargo/env" && cargo <...>
```

Or use the **justfile** (each recipe sources the env): `just build`, `just test`,
`just lint`, `just fmt`, `just deny`, `just coverage`, `just mutants`, `just check`.

**Build incrementally — gate only what changed.** Per iteration, verify the modified
crate(s) only: `just check-crate <crate>` (one crate) or `just check-changed` (all crates
touched in the working tree). These use `cargo … -p <crate>`, so unchanged crates are
neither recompiled (sccache + cargo incremental) nor re-tested. Reserve the full-workspace
`just check` for the **cross-crate integration gate before committing a milestone**. The
crate split exists precisely so a change rebuilds/tests a minimal subtree — keep crates
small and dependencies pointing one way (see `docs/INTERFACES.md`). Use `detect_changes`
(codebase-memory) to see a diff's blast radius before choosing the gate scope.

Toolchain pinned to **1.96.0** via `rust-toolchain.toml`. Edition **2024**.
Installed tooling: cargo-nextest, cargo-deny, cargo-audit, cargo-llvm-cov, cargo-mutants,
cargo-machete, just, sccache (build cache, wired via `.cargo/config.toml`).

## Environment

Apple M4 / Metal 4, `aarch64-apple-darwin`. No local NVIDIA — the CUDA GPU path is
validated in CI/containers on Linux. **GPU strategy:** `wgpu` (Metal/Vulkan/DX12) baseline
+ optional CUDA backend + CPU-SIMD fallback (note: Metal lacks f64).

## Knowledge base (source of truth for design)

`docs/` holds the design corpus, produced and maintained by the research/design workflow:

- `docs/ARCHITECTURE.md` — system architecture, crate layout, concurrency/latency model,
  GPU abstraction, hot-upgrade strategy, SDK/plugin model, data flow.
- `docs/ANALYTICS-SPEC.md` — the market-standard FX-options analytics to implement.
- `docs/COMPETITIVE-ANALYSIS.md` — competitor critique & positioning (analysis doc only).
- `docs/CELER-INTEGRATION.md` — integration map with the Celer estate + vendor feeds.
- `docs/INTERFACES.md` — frozen-interface registry (the current contracts).
- `docs/CONVENTIONS.md` — FX convention spec mapped to the `celnet-types` enums.
- `docs/ROADMAP.md` — phased plan + crate-ownership workstreams for parallel sessions.

Cross-session durable facts/decisions live in the auto-memory at
`~/.claude/projects/-Users-adrian-code-celeroption/memory/` (index: `MEMORY.md`).

## Parallel multi-session model

Independent Claude sessions own **disjoint crates** → no merge conflicts. Shared interface
crates (core types, traits, wire schemas) are stabilized **first**, then changed only with
coordination. A session: (1) reads this file + `docs/ROADMAP.md` + the ledger below,
(2) claims a workstream, (3) builds it to passing gates, (4) updates the ledger + memory.

## Implementation ledger

> Append-only status log. Newest first. One line per meaningful unit of progress.

- 2026-05-31 — **Live demo + Excel + GUI all REAL (no mocks); building durable journal.**
  `celnet-fix` (real FIX 4.4 engine, acceptor+initiator, dialect, loopback-tested) +
  `celnet-integration` egress governor/ingress/deployment-mode seam + `celnet-router`
  (fleet HRW partition map). Excel add-in (`excel/`, Office.js `CELNET.*`) + `gui/`
  (React/WebGPU trader UI) both verified end-to-end against a LIVE seeded server
  (`cargo run -p celnet-server --example demo_edge`). GUI flipped to **live WS by default**
  (mock demoted to `?mock`), stuck-resync + click-to-trade fixed, Risk shows real
  cross-gamma/theta-roll/vega. Headless e2e PASS (PRICE==server, MARK→version pin,
  forged-token reject). **Persistence audit (this session):** recovery = deterministic replay;
  durable today = `celnet-fix` FileStore + `celnet-observability` lossless audit (committer
  seam); blue-green handoff = in-memory; integrated-mode trade/position durability = the Celer
  estate; **gap = a standalone durable event-log/WAL (SCALE-OUT §8 "designed only")** → now
  being built as `celnet-journal` (task #37). See memory [[session-state-2026-05-31]] for the
  running services + how to resume.
- 2026-05-30 — **GA sign-off (rev 2).** All open-gap streams closed: plugin-host (wasmi) +
  trader GUI built; API-v2 optimized (multiplex session, click-to-trade keyed-MAC token,
  book-shaped risk, surface_version — no versioning). 21 crates + `gui/`, **555 tests green**,
  full `just check` terminating. `docs/GA-READINESS.md`: **GO** for the pricing-platform GA with
  one honest gating caveat — end-to-end latency-under-load proof + CI bench gate before the
  wire-latency headline is GA-grade; fleet layer, GPU-at-scale, live Celer/FIX, WS-mirror, and
  TARF/quanto breadth are de-risked post-GA execution.
- 2026-05-30 — **WS-G plugin host built — wasmtime blocker CLOSED.** `celnet-plugin-host` is a
  tiered host behind the frozen `celnet-plugin-api` contract: a unified `ModelRegistry` routes
  **Tier-0 native** (`dyn PricingModel` via the tier-blind `HostModel` seam) and **Tier-2 wasm**
  models identically. Tier-2 is the deterministic sandbox on **wasmi 1.0.9** (pure-Rust,
  fuel-metered, advisory-clean — replaces wasmtime): `Config::consume_fuel`, a no-WASI capability
  `Linker` exposing ONLY the libm `celnet_core::math` primitives (zero ambient authority),
  per-call `FuelBudget` SLA (exhaustion ⇒ typed `HostError::FuelExhausted`, never a hang),
  boundary NaN-canonicalization, and a host-controlled `(ptr,len)` core-module ABI marshalling
  `VanillaInputs`→`Greeks`. Deterministic **replay** harness asserts `to_bits` identity across
  runs. 14 tests green (all four WS-G gates: capability-denial, fuel-exhaustion bounded under a
  watchdog, replay bit-identity, Tier-0==Tier-2 interchangeability via WAT fixtures). `cargo fmt`
  + `clippy -D warnings` + `nextest` + `cargo-deny` (advisories/bans/licenses) all green. Docs
  synced (ARCHITECTURE §6, ROADMAP WS-G, CAPABILITIES-VS-COMPETITION, `wit/celnet.wit` header →
  wasmi/core-modules). No `unsafe`. **Next:** Tier-1 `stabby` signed-`.so` + Tier-3 Landlock ring
  (designed in `PLUGIN-HOST-ALT.md`), GA sign-off (#15).
- 2026-05-30 — **GA-push critique "needs-work" findings closed (client/server/engine + GUI doc).**
  (1) Client RFS reconnect-liveness bug fixed: `reconnect_session` + session-close now drain the
  click-to-trade waiter table via `fail_all_waiters`, resolving every pending `execute` with a
  typed `ClientError::Reconnected`/`StreamClosed` (no more infinite await across a blue-green
  cutover); regression tests are timeout-bounded. (2) Server `consumed_tokens` bounded:
  `HashMap<token, valid_until>` with expiry eviction on insert (`record_consumed`/`is_consumed`) —
  replay protection still holds *within* the validity window; bounded-growth test added. (3)
  Forgeable token minter replaced by a **keyed-MAC** `TokenMinter` (`blake3` keyed hash over the
  line-binding tuple under a 256-bit OS-CSPRNG secret drawn once at session start — runtime
  control-plane identity, NOT a pricing input, so pricing determinism is untouched); key-bound +
  field-bound MAC tests added. (4) Engine flaky tests fixed: zero-alloc concurrent-publish proof
  made deterministic (reclamation proven in a separate single-thread armed micro-window; racy
  `deallocs>0` sub-assert removed, zero-alloc guarantee UNWEAKENED); seqlock/arc-swap probes
  wall-clock-capped (`SPIN_CAP`); new `.config/nextest.toml` serializes the global-allocator/
  spin-sensitive engine tests (`engine-serial` group) + slow-timeout. Engine suite now ~0.9 s in
  isolation AND `--workspace`, stable across repeated runs. (5) `docs/GUI-DESIGN.md` §2/§4.2/§8/§10
  updated: `StreamService.StreamSession` (multiplex) is the contract and click-to-trade is
  *implemented* (Positions/P&L `GetPosition`/`AttributePnl` remains the only honest API-v2 gap).
  `blake3`/`getrandom` vetted clean by cargo-deny (advisories/licenses/bans ok). `just check`
  fully green.
- 2026-05-30 — **API-v2 stage 2/3: celnet-server on the optimized celnet-proto.** Server
  caught up to the multiplex/click-to-trade/book-risk/surface-version contract. New
  `surface_book` (versioned marked-surface registry: `MarkSurface` deposits calibrated
  smiles under a fresh `surface_version`; the pricing/RFQ/RFS paths pin against it via the
  shared `services::pin` resolver — unknown version ⇒ `failed_precondition`, never silent
  live fallback). `stream.rs` rewritten to the multiplex `StreamSession` driver: one session,
  many subscriptions, per-sub sequence/snapshot/delta/resync, in-place `Modify`, and
  click-to-trade — unguessable `splitmix64`-minted `TradableToken`s (SELL@bid/BUY@offer) with
  `valid_until` last-look + `Execute` idempotency, rejecting stale/forged/already-consumed.
  Scenario now book-shaped: theta-roll (`FACTOR_TIME`) axis rolling expiry per node, bucketed
  vega per (tenor,delta) pillar, cross-gamma 2-D stencil. Pricing/Quote echo
  `correlation_id`+`surface_version`. celnet-client updated to the new contract.
  **59 server tests + 22 client tests pass (suite < 0.1 s); clippy -D clean, fmt clean.**
- 2026-05-30 — **Full-implementation audit → remediation → GA-evidence (verified).** 19-lane
  read-only audit (98 findings: 7 blockers/30 majors/43 minors/18 gaps) → layered remediation
  resolving ALL blockers+majors (libm determinism, seqlock UB, method/person-name purge,
  honesty/doc fixes, broker→smile calibration + DegenerateQuote guard, holiday/convention
  fixes, idempotency/resync) → re-audit verdict production-grade. Then GA-evidence:
  `celnet-parity` (15 capability rows gated vs incumbents), CI matrix + nightly fuzz,
  mutation kill-rate 78→88.4% on vanilla, 96% core coverage, true 13-Greek count reconciled.
  **509 tests, `just check` green & terminating.** Remaining to GA: API-v2 (#20), scale-out
  validation (#19), plugin-host (#10 — since DONE on wasmi, see top of ledger), GA sign-off (#15).

- 2026-05-30 — **G3 reached (wide 4-lane wave).** `celnet-exotics` (digitals/touches/DNT/
  all-8 barriers + survival-weighted VV overlay; PDE Crank-Nicolson+Rannacher & Philox MC,
  PDE≈MC≈analytic cross-validated), `celnet-gpu` (PricingBackend over wgpu/Metal + f64 CPU
  oracle, Philox bit-stable, f32↔f64 reconciled), `celnet-engine` (core-pinned zero-alloc hot
  path: rtrb SPSC, arc-swap/seqlock, blue-green handoff; audited seqlock unsafe),
  `celnet-golden` (QuantLib 1.42.1 frozen tables; vanilla + all-8 barriers + both digital
  styles gated to ~1e-10/last-bit — independent oracle). 284 tests, full `just check` green;
  adversarial verdict production-grade. Auto-index live (post-commit + Stop hooks). **Next
  (wide wave):** LSV booking model, vendor/multi-source integration, streaming edge
  (server/cli), competitive parity matrix as executable tests, WS-T hardening.
- 2026-05-30 — **G2 reached.** `celnet-surface` (VV + broker→smile + SABR/SVI/SSVI +
  arb-free term structure) + `celnet-bench` (measured: vanilla price 8.85ns, +14 Greeks
  ~19ns, 64-strike batch ~6.75µs). Adversarial review caught + fixed a person-named public
  fn + overstated docs + a SABR sign error.
- 2026-05-30 — **G0 + G1 reached (3 parallel lanes).** `celnet-calendar` (43 tests),
  `celnet-conventions`, `celnet-vanilla` strike↔delta solver + 4 delta conventions + ATM/DNS,
  `celnet-testkit` (shared invariants/strategies), `celnet-proto` (single unversioned wire
  contract, protox build), `celnet-plugin-api` (SDK traits + WIT + validated example). 130
  tests, full `just check` green. Adversarial review caught + fixed a sign-inverted charm in
  the SDK example (added full-Greek FD gate). Incremental build recipes (`check-crate`,
  `check-changed`) + lane infra (central dep registry, skeletons). **Next (parallel lanes):**
  G2 `celnet-surface` (highest-leverage Synoption gap), latency-bench harness + QuantLib
  golden oracle (WS-T), then exotics ∥ gpu ∥ plugin-host ∥ integration.
- 2026-05-30 — **P0/P1 vertical slice green.** Flat workspace live; `celnet-types` (frozen
  vocab + convention enums + DTOs), `celnet-core` (libm math, deterministic `assert_close`,
  `Smile` trait), `celnet-vanilla` (Garman-Kohlhagen + full 14-Greek set). 17 tests pass:
  BS textbook benchmark, put-call parity (512 proptest cases), finite-difference validation
  of every Greek. `just check` fully green (fmt/clippy-D/nextest/deny). **Next:** complete
  G0 (`celnet-proto`, `celnet-plugin-api`), then fan out post-G0 workstreams via a workflow.
- 2026-05-30 — Design corpus written to `docs/` (ARCHITECTURE, ANALYTICS-SPEC,
  COMPETITIVE-ANALYSIS, CELER-INTEGRATION, ROADMAP + `_research/`). Product renamed
  CelerOption → **Celnet**.
- 2026-05-30 — Foundations: git (local-only) init; Rust 1.96.0 + tooling; rust-analyzer-lsp
  plugin; memory bootstrapped; settings/guardrails; toolchain/config files.

## Work-stream ledger

> Live ownership for parallel sessions (see `docs/ROADMAP.md` §4/§7). Claim a row by
> editing it before starting. One owner per row at a time; crates are disjoint.

| Stream | Crates (owned) | Status | Owner | Dep gate | Notes |
|--------|----------------|--------|-------|----------|-------|
| WS-0 | celnet-types, celnet-core, celnet-proto, celnet-plugin-api | **DONE** | — | — | G0 complete; all four frozen. Wire contract unversioned (ADR-0007). |
| WS-A | celnet-conventions, celnet-calendar | **DONE** | — | G0 | calendar (43 tests) + convention registry green & validated. |
| WS-B | celnet-vanilla | **DONE** | — | G0 | GK + 14 Greeks + 4 delta conventions + ATM/DNS + strike↔delta solver. G1 reached. |
| WS-C | celnet-surface | **DONE** | — | G1 | VV/SABR/SVI/SSVI + broker→smile + arb-free term structure. G2 reached. |
| WS-D | celnet-exotics | **DONE (1st-gen)** | — | G2 | digitals/touches/DNT/barriers + PDE/MC, QuantLib-gated. LSV booking model pending (task #16). |
| WS-E | celnet-gpu | **DONE (core)** | — | G0 | PricingBackend wgpu/Metal + CPU oracle, Philox, f32↔f64 reconciled. Sobol QMC = enhancement. |
| WS-F | celnet-engine | **DONE (hot path)** | — | G1/G3 | core-pinned zero-alloc rt + blue-green handoff. Full edge wiring = WS-I. |
| WS-G | celnet-plugin-host | **DONE** | — | G0 | Tiered host: Tier-0 native registry + Tier-2 **wasmi 1.0.9** fuel-metered no-WASI sandbox + replay harness, behind frozen `celnet-plugin-api`. 14 tests, all 4 gates green; deny clean. wasmtime blocker CLOSED. Tier-1 stabby `.so` + Tier-3 Landlock ring designed, not yet wired. |
| WS-H | celnet-integration | UNBLOCKED | — | G0/WS-C | **next** — vendor feed normalization + multi-source surface aggregation/divergence. |
| WS-I | celnet-server, celnet-cli | UNBLOCKED (G3) | — | G3 | **next** — streaming gRPC/WS edge + admin CLI (tokio/tonic vetted). |
| WS-T | CI/test/deny/golden/bench | PARTIAL | — | G0 | testkit + QuantLib golden + latency bench DONE; pending: fuzz/mutation/coverage gates, CI matrix, executable parity matrix (#14). |
