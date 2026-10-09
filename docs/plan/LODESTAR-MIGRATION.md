# Lodestar Migration & Distributed-Agent Knowledge Program

**Status:** ✅ **COMPLETE** (started 2026-06-22; migration finished, `codebase-memory-mcp`
(CBM) fully decommissioned 2026-06-30). Owner: this session.

> **Migration DONE — CBM decommissioned.** lodestar is the single code-graph +
> verified-knowledge substrate. The former `codebase-memory-mcp` (CBM) has been retired
> with **zero legacy**: its global MCP registration was removed on **2026-06-30** and its
> server is now disconnected. The phases below are kept as the historical record of how the
> migration was carried out; language describing CBM as "registered"/"active" is historical
> (past-tense) — CBM is no longer live anywhere.

**Goal:** Make **lodestar** the single code-graph + verified-knowledge substrate for the
CelNet estate, replacing `codebase-memory-mcp` (CBM) with **zero legacy**; auto-index;
store all shareable knowledge in the remote git (`github.com/soarsa/celnet`) so
distributed AI coding agents (Agent) on **Windows + macOS** converge; optionally
share in realtime on a LAN via `lodestar-syncd`.

This file is the gated plan required by the "full reorg" decision — the crate-layout /
repo-restructure steps do **not** run inline; they run in a dedicated T2 window,
coordinated on `docs/PARALLEL-SESSIONS.md`.

## Decisions (2026-06-22)

1. **Estate scope** — the 10 celnet-service wikis migrate **onto lodestar in place**
   (each repo is its own lodestar project; re-point its structural layer off CBM, clean
   references, keep its knowledge with it). lodestar is the shared engine across all;
   CelNet stays a clean standalone product — it does **not** absorb other repos' content.
2. **Restructure depth** — **full reorg incl. crate layout**, executed via this plan +
   a dedicated gate window (not inline).
3. **Legacy docs** — durable invariants/decisions/rationale → **lodestar claims + ADRs**;
   long-form narrative/planning → **single-homed in the git repo** (the shared state
   lodestar indexes); out-of-date material → **reconciled or pruned** (git keeps history).

## Architecture (what lodestar holds, and what it does not)

| Knowledge class | Home | Shared how |
|---|---|---|
| Code structure (graph: nodes/edges) | `.lodestar/graph.db*` | **regenerable, git-ignored**; each machine re-indexes; deterministic → all converge |
| Verified "why" (invariants, decisions, rationale) | lodestar **claims** (`knowledge_put`) + ADRs (`manage_adr`) → `.lodestar/knowledge/` | **git-committed**; content-addressed, merge-conflict-free; self-invalidates on code change |
| Narrative / planning (design docs, ledger, wiki prose) | in-repo markdown under `docs/` | **git-committed** (markdown is not a lodestar primitive) |
| Per-machine config | `.agents/settings.local.json`, caches | **git-ignored** |

Realtime: **git is the truth** for disconnected machines (commit `.lodestar/knowledge/` →
pull/push). **`lodestar-syncd`** is an opt-in LAN convenience (mDNS, PSK+AEAD) layered on
top for co-located developers; killing it reverts to the git-only flow.

## Phases

### P0 — Install + index  ✅ DONE
- `lodestar 0.8.0` installed at `~/.local/bin/lodestar`; MCP registered (coexisted with CBM
  during bake-in); skills `lodestar` + `knowledge-maintenance` installed; hooks swapped to
  `lodestar-discovery-augment` + `lodestar-session-reminder` (advisory).
- Full index of CelNet ran → established a trustworthy node count (CBM's graph was
  degraded at the time: 2,229 nodes vs the ≈17k baseline).

### P1 — Foundation (auto-index + git-shared knowledge)
- Auto-index: lodestar's native FS watcher (live) **+** a portable git `post-commit` that
  calls `lodestar index` (deterministic, commit-time). Retarget the project Stop-hook too.
- `.gitignore`: `.lodestar/*.db*`, `.lodestar/graph.db.zst`, `.lodestar/*.tmp`, keep
  `.lodestar/knowledge/` + `.lodestar/project-id` tracked.
- `.gitattributes`: force **LF** on sources + `.lodestar/knowledge/**` — required for
  lodestar's byte-identical determinism across Windows/macOS.
- Prove lodestar serves CelNet (`search_graph price_instrument`, `get_architecture`).

### P2 — CBM decommission (zero legacy)  ✅ DONE (2026-06-30)
- Removed `codebase-memory-mcp` from `~/.agents/.mcp.json` and `~/.agents.json` (global MCP
  registration removed 2026-06-30; server disconnected).
- Deleted orphaned hook scripts `~/.agents/hooks/cbm-*`.
- Removed `.codebase-memory/` (migrated `adr.md` content first → ADRs/claims).
- Scrubbed CBM references: `GUIDE.md` (guardrail #3 + toolchain), `.agents/workflows/AGENT-PREAMBLE.md`,
  `docs/**`, the `codebase-memory` skill, auto-memory (`cbm-mcp-first`,
  `codebase-graph-health`, `dev-environment`, …) → rewritten to lodestar `doctor`-based health.

### P3 — Knowledge consolidation (CelNet)
- Fold the `~/wiki/celnet` narrative into `docs/` (single home), de-duplicating.
- Author durable invariants/decisions as lodestar **claims** anchored to symbols; mirror
  ADRs via `manage_adr`. Run the staleness/constraint gate to flag drifted content.
- Prune superseded planning per Decision 3.

### P4 — Repo restructure (GATED — dedicated T2 window)
- Declutter root: loose `*.png` → `docs/assets/`; delete transient junk (`cur-errors.txt`,
  `mutants.out.old`); ensure `target/` is ignored only.
- Add **`AGENTS.md`** — the cross-agent entrypoint (12 agents incl. Agent) pointing at
  lodestar + the operating guide; slim `GUIDE.md` to defer to it.
- Optional crate **domain grouping** under `crates/<group>/` (core / engines / risk /
  services / clients / infra). Mechanical but touches 39 `Cargo.toml` `path` deps + workspace
  glob + justfile crate lists + docs — **one atomic commit, full T2 gate**, no parallel lane
  mid-flight.
- Move TS clients (`gui/`, `excel/`) under a `clients/` umbrella (optional).

### P5 — Cross-platform + distributed bootstrap
- Hooks portable: `.sh` (mac/linux) + `.ps1` (Windows); rely on lodestar's own
  installer-managed hooks where possible (cross-platform); Git-for-Windows ships bash so the
  `post-commit` works there too.
- Commit `.agents/` (agents, skills, workflows, shared `settings.json`); `settings.local.json`
  stays ignored. New-machine bootstrap doc (install lodestar → `lodestar install` →
  `lodestar index` → pull knowledge), Windows + macOS.
- Document `lodestar-syncd` opt-in for LAN realtime.

### P6 — Estate sweep (10 celnet wikis → lodestar in place)
- Per repo: re-point structural layer off CBM, clean CBM references, confirm lodestar indexes.
- Highly parallelizable (disjoint repos) — candidate for a multi-agent workflow (opt-in).

### P7 — Verify, commit, push, record
- Bake-in verify lodestar across a real discovery/workflow; then commit + push knowledge +
  config to `soarsa/celnet`. Update auto-memory + the GUIDE.md resume anchor.

## Risk & rollback
- Full pre-change backup: `~/.agents/backups/lodestar-migration-20260622-202652/`
  (`.agent.json`, both `.mcp.json`, user+project `settings.json`, `post-commit`, hooks,
  `AGENT-PREAMBLE.md`).
- CBM was kept registered until P2 (bake-in) for an instant revert by restoring the backup;
  P2 completed on 2026-06-30 and CBM is now fully decommissioned (registration removed, server
  disconnected).
- Crate reorg gated behind a full T2 gate + a quiet parallel-session window.

## Cross-platform notes
- **LF normalization is mandatory** (`.gitattributes`) or lodestar's byte-identical graph +
  content-addressed knowledge events diverge between Windows (CRLF) and macOS.
- macOS binary is ad-hoc signed (installer clears quarantine); Windows uses `install.ps1`.
- `just` is cross-platform; verify recipes are pwsh-safe or pinned to a portable shell.
- **Always index with an ABSOLUTE path.** `lodestar index .` (relative) records `root_path="."`
  and corrupts the projects table (lodestar then auto-deletes the db; re-index required). All
  hooks pass a runtime-computed absolute root (`"$PWD"` in Agent hooks where cwd = repo root;
  `git rev-parse --show-toplevel` in the git hook) — portable, never a hardcoded home path.
