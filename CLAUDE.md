# CelerOption — Claude Operating Guide

State-of-the-art FX **Options** pricing platform in Rust. Ultra-low-latency, scalable,
mission-critical, hot-upgradable; integrates into the Celer trade-lifecycle estate and
front end, consumes Fenics market data, and exposes user-extensible analytics via SDKs.
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

## Running the toolchain

The shell does **not** persist env between Bash calls and rustup is not on the default
PATH. Prefix every Rust command:

```bash
source "$HOME/.cargo/env" && cargo <...>
```

Or use the **justfile** (each recipe sources the env): `just build`, `just test`,
`just lint`, `just fmt`, `just deny`, `just coverage`, `just mutants`, `just check`.

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
- `docs/COMPETITIVE-ANALYSIS.md` — Synoption / Fenics / Bloomberg critique & positioning.
- `docs/CELER-INTEGRATION.md` — integration map with Celer services + Fenics.
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

- 2026-05-30 — Foundations: git (local-only) init; Rust 1.96.0 + tooling installed;
  rust-analyzer-lsp plugin; memory bootstrapped; settings/guardrails; toolchain/config
  files (rust-toolchain, rustfmt, deny, .cargo/config, justfile). Research/design workflow
  running to produce `docs/`. **Next:** write design docs, scaffold Cargo workspace per
  ARCHITECTURE.md, stabilize interface crates, begin P0 pricing core.
