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
   `get_code_snippet`, `query_graph`, `get_architecture`); fall back to Grep/Read only for
   non-code text. Re-`index_repository` as the workspace grows. Saves tokens, stays exact.
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
| WS-C | celnet-surface | UNBLOCKED (G1) | — | G1 | **next, highest-leverage** — VV/SABR/SVI/SSVI, broker→smile, arb-free. |
| WS-D | celnet-exotics | BLOCKED | — | G2 | |
| WS-E | celnet-gpu | UNCLAIMED | — | G0 | runs parallel to WS-D on PricingBackend |
| WS-F | celnet-engine | UNCLAIMED | — | G1/G3 | |
| WS-G | celnet-plugin-host | UNCLAIMED | — | G0 | api ready; host (wasmtime) can start now |
| WS-H | celnet-integration | UNCLAIMED | — | G0/WS-C | |
| WS-I | celnet-server, celnet-cli | BLOCKED | — | G3 | |
| WS-T | CI/test/deny/golden-gen/bench | PARTIAL | — | G0 | testkit DONE; pending: QuantLib golden oracle, latency bench, fuzz/mutation/coverage, CI matrix |
