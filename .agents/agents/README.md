# Celnet Agent subagents

Project-level subagents, committed to git so **every developer on every machine** (macOS or
Windows) gets the same optimally-configured fleet — and parallel work shares state instead of
clobbering it. They are discovered automatically from `.agents/agents/` (project scope, higher
precedence than personal `~/.agents/agents/`). Invoke via the Agent/Task tool or let Agent
auto-delegate from each agent's `description`.

## The fleet

| Agent | Role | Tools posture | Model |
|---|---|---|---|
| `celnet-explorer` | Read-only codebase navigation via the lodestar graph | read-only (`disallowedTools: Edit, Write, NotebookEdit`) | sonnet |
| `celnet-quant` | Rust pricing / numerical engine build, oracle-validated | full (inherits all) | opus |
| `celnet-gui` | React GUI + Excel add-in, live Playwright/axe/DevTools verify | full (inherits all) | inherit |
| `celnet-verifier` | Adversarial, read-only verification of a change/claim | read-only | opus |
| `celnet-knowledge-curator` | Maintain lodestar verified claims + ADRs (the "why") | full (inherits all) | inherit |

## Shared conventions baked into every agent

- **lodestar first** for code discovery (project key `github.com-soarsa-celnet`); Grep/Read only for
  non-code text; LSP (rust-analyzer) for exact Rust intel. Headless: `lodestar cli <tool> '{...}'`.
  Health-probe with `lodestar doctor --json`; always re-index with an **absolute** path.
- **Leverage our MCPs/plugins**: lodestar (graph + verified knowledge), rust-analyzer LSP, Playwright
  + axe + Chrome DevTools (GUI), protobuf (proto). Tools are inherited (or denylisted for read-only
  agents) rather than allow-listed, so the fleet stays robust as plugins evolve.
- **Hard rules**: no mocks/placeholders/`todo!()`; vendor-neutral celnet-named identifiers; gate only
  what changed (`just t0/t1`, full `just t2` at a milestone); validate numbers against an independent
  oracle (no circular-oracle).
- **Parallel without clobbering**: claim a lane on `docs/PARALLEL-SESSIONS.md`, work in an isolated git
  worktree, touch only your subtree, never commit another lane's files. The lodestar knowledge log is
  content-addressed + set-union merge, so shared "why" never conflicts across machines.

A new developer gets the whole fleet by cloning + running `lodestar install` (registers the lodestar
MCP + skills + hooks locally). See `docs/plan/LODESTAR-MIGRATION.md`.
