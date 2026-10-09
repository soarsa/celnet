---
name: dev-environment
description: "Celnet toolchain, hardware, GPU strategy, and how to invoke cargo."
metadata: 
  node_type: memory
  type: project
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

Dev box: **Apple M4, Metal 4**, `aarch64-apple-darwin`, macOS (Darwin 25.5). No local NVIDIA/CUDA — the CUDA GPU path is validated in CI/containers on Linux, not on this machine.

Toolchain (installed 30 May 2026): **Rust 1.96.0 stable** via rustup, with clippy, rustfmt, rust-src, llvm-tools. Extra tooling via cargo-binstall: cargo-nextest, cargo-deny, cargo-audit, cargo-llvm-cov, cargo-mutants, cargo-machete, just, sccache.

**Invoking cargo:** the shell does NOT persist env between Bash calls, and rustup isn't on the default PATH. Every cargo/rustc command must start by sourcing the env, e.g. `source "$HOME/.cargo/env" && cargo build`. (Codified in GUIDE.md.)

**GPU strategy:** cross-platform compute via `wgpu` (Metal/Vulkan/DX12) as the baseline, optional CUDA backend (cudarc) for NVIDIA hosts, with a CPU-SIMD fallback. Metal lacks f64 — handle precision per-backend. See [[celnet-mission]].

**Incremental, crate-scoped builds (user directive 30 May 2026):** organize so a cycle only builds/tests the crates that changed. Per iteration use `just check-crate <crate>` or `just check-changed` (git-diff → `cargo … -p <crate>` for changed crates only); unchanged crates aren't recompiled (sccache + cargo incremental) or re-tested. Reserve the full-workspace `just check` for the milestone integration gate before committing. Keep crates small with one-way deps so the rebuild subtree stays minimal; use `detect_changes` to see a diff's blast radius. Parallel "lane" agents each build with `-p <their-crate>`. See [[parallel-session-model]], [[api-naming-and-evolution]].

**Golden-oracle toolchain:** QuantLib (open-source) is the numerical reference for validation. Installed in an isolated venv at `~/.celnet-goldenv` (system python is PEP-668 externally-managed — do NOT `pip install` into it; use the venv). Run the oracle via `~/.celnet-goldenv/bin/python`. Verified 30 May 2026: QuantLib 1.42.1 `GarmanKohlagenProcess` + `AnalyticEuropeanEngine` reproduces Celnet's GK benchmark (S=K=100,T=1,r_dom=5%,r_for=0,σ=20% → call 10.4505835721855) to ~1e-14. The golden-table generator (WS-T) lives in-repo and shells to this python to emit frozen reference CSVs. See [[no-commercial-products]] (QuantLib OSS = allowed oracle).

**Code intelligence / plugins:** the `rust-analyzer-lsp@official-plugins` plugin is installed — use the **LSP tool** (goToDefinition, findReferences, hover, documentSymbol, incomingCalls/outgoingCalls) for exact, deterministic code intel on `.rs` files. Toolchain is pinned via `rust-toolchain.toml` (1.96.0). Quality gates run via `just` recipes (fmt, clippy, nextest, deny, audit, llvm-cov, mutants). Prefer LSP + lodestar over grep ([[lodestar-first]]).
