# Celnet — Agent Entrypoint

Cross-agent orientation for the Celnet codebase. Every agent (Agent or any
lodestar-supported tool) starts here, then follows the pointers below. This file
is the concise entry; the full operating guide is **[GUIDE.md](GUIDE.md)**.

---

## Operating guide

Read **[GUIDE.md](GUIDE.md)** before writing any code. It contains the hard
guardrails (git policy, no mocks, vendor-neutral naming, gate tiers, scale
requirements, zero-disruption headless testing) that every agent must follow without exception.

---

## Codebase knowledge graph — lodestar

All code discovery and verified "why" comes from lodestar, never from grep/read.

**Project key:** `github.com-soarsa-celnet` (stable, git-remote-derived; identical
on every clone — no machine-specific paths anywhere).

**Health probe** (run before trusting the graph):

```sh
lodestar doctor --json
# Must report: ok:true, problems:0
# Hot symbol check: lodestar cli search_graph '{"project":"github.com-soarsa-celnet","query":"price_instrument"}'
# A sharp node-count drop => lodestar index --full <abs-path-to-repo>
```

**Always use MCP tools, not the CLI**, from within a Agent session:
- `mcp__lodestar__search_graph` / `mcp__lodestar__search_code` — find symbols
- `mcp__lodestar__get_code_snippet` — exact source by qualified name
- `mcp__lodestar__trace_path` — callers, impact, data flow
- `mcp__lodestar__evidence_pack` — token-budgeted verified bundle (~21x fewer tokens)
- `mcp__lodestar__knowledge_get` — verified "why" (invariants, decisions, rationale)
- `mcp__lodestar__knowledge_put` — author/revise a verified claim
- `mcp__lodestar__get_architecture` — layout without reading many files
- `mcp__lodestar__detect_changes` — blast radius before choosing gate scope
- `mcp__lodestar__manage_adr` — author/update Architecture Decision Records
- `mcp__lodestar__query_graph` — Cypher for structural/relationship queries

The verified knowledge log (`.lodestar/knowledge/`) is **committed to git** and
shared across all machines by set-union merge — no conflicts.

---

## Subagent fleet

Project-level subagents live in **[.agents/agents/](.agents/agents/)** and are
auto-discovered on every clone. See `.agents/agents/README.md` for the full table.

| Agent | Role |
|---|---|
| `celnet-explorer` | Read-only codebase navigation via the lodestar graph |
| `celnet-quant` | Rust pricing / numerical engine — oracle-validated |
| `celnet-gui` | React GUI + Excel add-in, live Playwright/axe verify |
| `celnet-verifier` | Adversarial read-only verification (refute-default) |
| `celnet-knowledge-curator` | Maintain lodestar verified claims + ADRs |

**No Stage-2 judge** is configured (removed by operator decision) — the deterministic
**Stage-1 constraint gate** is the primary verifier and needs no model. The
verification-loop canon (`.agents/skills/verification-loop/SKILL.md`) + the
`celnet-verifier` agent cover review where a behavioral check is genuinely needed.

---

## New-machine bootstrap

```sh
# 1. Clone
git clone git@github.com:soarsa/celnet.git
cd celnet

# 2. Install lodestar (registers MCP + skills + hooks locally)
lodestar install

# 3. Index with an ABSOLUTE path (relative '.' corrupts the db)
lodestar index "$(pwd)"

# 4. Pull shared knowledge (committed in .lodestar/knowledge/)
git pull   # events/ merge by set-union — no conflict

# 5. (Agent) trust/enable the PROJECT .mcp.json lodestar server. It carries the
#    cortex env (LODESTAR_PROPOSALS=1 — the proactive drift stream); without it Agent
#    spawns the global registration (no env) and the cortex stays dark.
#    Verify after restart:  mcp__lodestar__knowledge_config -> knowledge_log.subscribed:true

# 6. Health probe
lodestar doctor --json
```

After bootstrap the full fleet (subagents, MCP, skills, hooks) + the shared knowledge are
active. Knowledge authored by any agent on any machine travels via normal git push/pull.
Optional, per developer: `lodestar --ui=true --port=9749` opens the shared-KB dashboard.

---

## lodestar capability set (switched on — all except the Stage-2 judge)

lodestar is the single cohesive substrate — discovery → knowledge → planning → tracking →
governance → visualization — shared cross-machine via the git-committed knowledge log.

- **Discovery + graph** — auto-index (FS watcher + SessionStart/Stop hooks), all crates +
  TS/TSX/CSS/proto, call graph + Leiden clusters.
- **Verified knowledge** — claims (`invariant`/`spec:satisfies`/`design`/`ui`/`a11y`/`adr`),
  the deterministic Stage-1 gate, the closed staleness loop (self-invalidating).
- **Proactive cortex** — `LODESTAR_PROPOSALS=1` (the active→stale drift stream), derive-rules
  (hot-core zero-alloc/non-blocking), motif discovery.
- **Planning / scoping / tracking** — `spec:satisfies` + `*.acceptance.json` + Deliverable
  roll-ups (`knowledge_get deliverable=`), `detect_changes` (blast radius), `tools/planner`
  (decompose), `tools/requirements` (PM bridge: github/jira/linear/plane).
- **Governance / export** — `manage_adr`, `knowledge_export format=sarif|markdown|llms.txt|
  graphml` (CI gate + flat human view).
- **Visualization** — `lodestar --ui=true --port=9749` (Trust Map, Knowledge tab, drift inbox,
  provenance) — every developer browses the same shared KB.

Knowledge is the single source of truth for facts/decisions/status; markdown holds only
narrative (lodestar can't render prose). Updating knowledge = author/refresh a claim, not
edit prose in N places. Engine gaps tracked upstream (never worked around): **lodestar#7**
(CSS-module token chain), **lodestar#9** (TESTS edges).

---

## Parallel-session model

Independent sessions own **disjoint crates** — no merge conflicts on code.
Before starting: claim a lane in `docs/PARALLEL-SESSIONS.md`. Work in an isolated
git worktree. Touch only your subtree. The lodestar knowledge log is
content-addressed + set-union merge, so verified "why" never conflicts.

Full details: `docs/PARALLEL-SESSIONS.md` and `docs/ROADMAP.md` §4/§7.
