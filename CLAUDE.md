# Celnet — Claude Operating Guide

State-of-the-art FX **Options** pricing platform in Rust. Ultra-low-latency, scalable,
mission-critical, hot-upgradable; integrates into the Celer trade-lifecycle estate and
front end, consumes external vendor FX-options market-data feeds, and exposes
user-extensible analytics via SDKs.
Greenfield, started 30 May 2026.

## Hard guardrails (non-negotiable)

1. **Git: local-first; one sanctioned remote.** Commit locally freely. Pushing is
   permitted **only** to `github.com/soarsa/celnet` (the `origin` remote, owner
   `soarsa`) — authorized 2026-06-05. Do **not** add any other remote or push
   elsewhere. (The previous local-only deny rules in `.claude/settings.json` were
   removed for this explicit authorization.)
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
coordination. A session: (1) reads this file + `docs/ROADMAP.md` + the implementation
ledger (`docs/IMPLEMENTATION-LEDGER.md`; newest entry mirrored below), (2) claims a
workstream, (3) builds it to passing gates, (4) updates the ledger + memory.

## Implementation ledger

> Append-only status log. **Full history → [`docs/IMPLEMENTATION-LEDGER.md`](docs/IMPLEMENTATION-LEDGER.md)** (newest first). Only the newest entry is
> mirrored below as a resume anchor — append new entries to that file (top of list)
> and replace the anchor here with the new newest entry.

**Current state (newest entry — full history in the ledger file):**

- 2026-06-08 — **W1 — MULTI-ASSET CORE LANDED + GUI foundation merged (commits `ba0fc03` W1 core,
  `6143409` gw-foundation merge; pushed).** The keystone wave: generalized the three FX-only Layer-0
  seams to a cross-asset vocabulary IN PLACE (one unversioned contract, FX byte-identical) per **ADR-0008**
  (identity / carry-as-forward-discount-producer / asset-class-agnostic payoff). Built as a gated workflow
  (trait → contract → GUI∥Excel∥plugin → adversarial-verify) + my independent re-gate. **celnet-core:** the
  carry-producing-market seam (`CarryInputs`/`CarryPricer`/`CarryGreeks` + typed `CarryPriceError`).
  **celnet-vanilla:** `FxPricer` delegates to the UNCHANGED GK arithmetic, to_bits-gated vs the golden grid.
  **celnet-proto:** `Underlying` (oneof fx) replaces `CcyPair`; MarketContext/VanillaInputs →
  {discount_rate, CarryModel}; Greeks rhos → `RateSensitivities`; convert FX round-trip to_bits-identical +
  product×underlying validity guard. **celnet-server:** routes by Underlying + a **no-silent-fallback carry
  guard** at the price-path head (the verifier-flagged blocker: a generalized carry was silently read as FX
  r_for=0; now a typed error). **plugin-api/-host:** PricingModel+WIT+wasmi ABI generalized; a NEW
  equity-dividend (CostOfCarry) plugin gate reconciles to an INDEPENDENT generalized-BSM oracle 1e-12 (no
  circular oracle). **GUI (gw-foundation, parallel worktree):** GW0 design-system + accessible role=grid
  `<DataGrid>` + GW1 single breadcrumb scope nav, deleting 7 redundant pair affordances. **Re-gated:** `just
  check` literal "All gates passed." (1343 tests, was 1306); conformance 120/120; GUI 425; Excel e2e 81; FX
  byte-identity gated by to_bits. **▶ All downstream fan-out lanes now OPEN** (`docs/PARALLEL-SESSIONS.md`):
  W2/W3/W4/W5 + GW2 — claimable by parallel service-mesh sessions on worktrees off `main`. Also done this
  arc: the **capabilities document** rebuilt comprehensive + multi-asset + SOTA + audience-targeted
  (Traders/Quants/Tech/Leadership), 30 visuals embedded, 0 hedges (`docs/celnet-capabilities.html`; PDF =
  open backlog `deliverable/capabilities-pdf`). Follow-up (not W1): the `celnet-surface` FX→neutral split
  (only when a non-FX surface leaf lands — W3 carries its own crypto surface leaf).



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
