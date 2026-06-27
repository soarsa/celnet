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
3. **lodestar first** for code discovery (`search_graph`, `trace_path`, `get_code_snippet`,
   `query_graph`, `get_architecture`, `detect_changes`, `manage_adr`) **and the verified
   "why" layer** (`knowledge_get`/`knowledge_put`, `evidence_pack`); fall back to Grep/Read
   only for non-code text. The graph **auto-indexes** (lodestar's native filesystem watcher
   + `SessionStart`/`Stop` `lodestar index` hooks in `.claude/settings.json`), so its scope
   always covers new files. The stable project key is **`github.com-soarsa-celnet`** (pinned
   in `.lodestar/project-id`, git-remote-derived → identical on every clone). **Health probe
   before trusting it:** `lodestar doctor --json` must report `ok:true`/`problems:0` and the
   hot symbol `price_instrument` must resolve (graph ≈18k nodes); a sharp drop ⇒
   `lodestar index --full .` (~2s, deterministic & byte-identical). Use `detect_changes` to
   scope builds/tests; keep ADRs current via `manage_adr`. The structural graph
   (`~/.cache/lodestar/`) is machine-local & regenerable; the verified-knowledge log
   (`.lodestar/knowledge/`) is git-committed and shared across machines. Saves tokens, stays
   exact, self-invalidates.
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
    — no stale or duplicate references anywhere. The lodestar graph re-indexes
    automatically (native filesystem watcher), so its scope always covers structural changes.
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

**Tiered gates — gate only what changed** (`docs/PARALLEL-SESSIONS.md` §4.2 is the law):

- **T0** (per-edit, seconds): `just t0 <crate>` = `cargo check -p`. Iterate on T0 only.
- **T1** (per-lane-batch): `just t1 [<crate>…]` — ONE invocation settles the accumulated
  batch (a single multi-`-p` cargo test + scoped clippy `-D warnings` + fmt; no args ⇒
  crates changed since the last green T1). **Accumulate edits; never gate per-fix.**
- **T2** (landing only): `just t2` — the full `check` gate set + the live GUI/Excel e2e,
  ONCE per push milestone, never per fix-iteration.

T1/T2 run through the **resumable runner** (`tools/gate-runner.sh`, ledger
`.gate-ledger.jsonl`): each step's PASS/FAIL + literal output line + REAL exit code is
journaled keyed on HEAD + a dirty-tree hash, so a killed/spend-walled gate resumes from
the last green step. `check`/`check-crate`/`check-changed` remain valid at their tier.
The crate split exists precisely so a change rebuilds/tests a minimal subtree — keep
crates small and dependencies pointing one way (see `docs/INTERFACES.md`). Use
`detect_changes` (lodestar) to see a diff's blast radius before choosing scope.

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

Cross-session durable facts/decisions live in the Claude Code per-project auto-memory
(`~/.claude/projects/<repo-path-slug>/memory/`, index `MEMORY.md`) — the slug is derived
from each machine's local checkout path, so it is per-developer, not shared.

## Parallel multi-session model

Independent Claude sessions own **disjoint crates** → no merge conflicts. Shared interface
crates (core types, traits, wire schemas) are stabilized **first**, then changed only with
coordination. A session: (1) reads this file + `docs/ROADMAP.md` + the implementation
ledger (`docs/IMPLEMENTATION-LEDGER.md`; newest entry mirrored below), (2) claims a
workstream, (3) builds it to passing gates, (4) updates the ledger + memory.

## Implementation ledger & live ownership

All status lives in disk files, not here (keeps per-session context small —
[[token-and-context-discipline]]):

- **Full append-only history** → [`docs/IMPLEMENTATION-LEDGER.md`](docs/IMPLEMENTATION-LEDGER.md) (newest first). Append new entries at its top.
- **Live parallel-session lane board** (claims/owners/branches) → [`docs/PARALLEL-SESSIONS.md`](docs/PARALLEL-SESSIONS.md). Claim a lane there before starting.
- **Phase plan + crate-workstream map** → [`docs/ROADMAP.md`](docs/ROADMAP.md) §4/§7.

**Resume anchor — one line only; replace in place each milestone, never grow it:**

- 2026-06-27 — **Stream/WS caller-authz cross-cut LANDED on `main` (`8e0ee48`, pushed)** — the #2 architecture-determination finding remediated. Closes the verified headline security hole (`docs/SECURITY-AUTHZ-FINDING.md`): an unauthenticated `StreamSession`/WS session could subscribe + click-to-trade with no caller check. Additive `StreamAuth` Authenticate frame pins a `ResolvedCaller`; ONE `authorize_caller(ReadAny)` gate before subscribe/modify/execute/resync on **both** gRPC (`handle_client_message`) and the WS mirror (`decode_stream_control`+`is_stream_control`); SDK/CLI/GUI/Excel authenticate-first (grant-all default = risk's `principal_or_grant_all`). Live-validated under `AccessMode::Enforce`: GUI **165/165** (incl. click-to-trade streaming) + Excel **122/122** + new Enforce unit/integration tests; a live-e2e-caught WS-routing bug (authenticate fell through to `handle_unary`) fixed with a decoder/router lockstep test ([[deferred-e2e-defect-reservoir]], [[agent-worktree-base-and-cross-cuts]]). 8-commit branch merged `--no-ff`. **Follow-up = task #20:** quote-accept→requester-principal binding + risk `principal∩desk_scope` (`docs/SECURITY-AUTHZ-FINDING.md` §2/§3, lower severity). Prior milestone: 1.0-RC `a0817d6` T2 16/16 (R1–R8, 24 arms × 5 clients). Detail → ledger top.
