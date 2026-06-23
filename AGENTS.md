# Celnet — Agent Entrypoint

Cross-agent orientation for the Celnet codebase. Every agent (Claude Code or any
lodestar-supported tool) starts here, then follows the pointers below. This file
is the concise entry; the full operating guide is **[CLAUDE.md](CLAUDE.md)**.

---

## Operating guide

Read **[CLAUDE.md](CLAUDE.md)** before writing any code. It contains the hard
guardrails (git policy, no mocks, vendor-neutral naming, gate tiers, scale
requirements) that every agent must follow without exception.

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

**Always use MCP tools, not the CLI**, from within a Claude Code session:
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

Project-level subagents live in **[.claude/agents/](.claude/agents/)** and are
auto-discovered on every clone. See `.claude/agents/README.md` for the full table.

| Agent | Role |
|---|---|
| `celnet-explorer` | Read-only codebase navigation via the lodestar graph |
| `celnet-quant` | Rust pricing / numerical engine — oracle-validated |
| `celnet-gui` | React GUI + Excel add-in, live Playwright/axe verify |
| `celnet-verifier` | Adversarial read-only verification (refute-default) |
| `celnet-knowledge-curator` | Maintain lodestar verified claims + ADRs |

The **judge driver** for Stage-2 knowledge review targets `celnet-verifier` as the
isolated adversarial subagent. See `tools/judge/judge-claude-subagent.sh` and the
verification loop canon at `.claude/skills/verification-loop/SKILL.md`.

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

# 5. Health probe
lodestar doctor --json
```

After bootstrap the full fleet (subagents, MCP, skills, hooks) is active.
Knowledge authored by any agent on any machine travels via normal git push/pull.

---

## Parallel-session model

Independent sessions own **disjoint crates** — no merge conflicts on code.
Before starting: claim a lane in `docs/PARALLEL-SESSIONS.md`. Work in an isolated
git worktree. Touch only your subtree. The lodestar knowledge log is
content-addressed + set-union merge, so verified "why" never conflicts.

Full details: `docs/PARALLEL-SESSIONS.md` and `docs/ROADMAP.md` §4/§7.
