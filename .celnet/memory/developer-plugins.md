---
name: agent-plugins
description: Which Agent plugins are installed for this project and why; what was deliberately skipped.
metadata: 
  node_type: memory
  type: reference
  originSessionId: 4e9a6b38-74d9-4a36-9109-4d49116555b3
---

Plugin tooling decision (2026-05-31), researched via the agent-code-guide agent + the real marketplaces.

**2026-06-29 optimization (token-economy audit via `agent plugin details <p>` → projected always-on cost).** Plugins are GLOBAL (user scope, no project `enabledPlugins`) ⇒ apply uniformly across celnet + all worktrees + the celnet estate. **REPRO GAP:** because they're global-only, a teammate's `git pull` does NOT get them — plugins are NOT reproducible from the repo (unlike `.mcp.json`/agents/skills/workflows/hooks/`.lodestar`, which ARE committed). **Fix:** `agent plugin install <p>@<mkt> --scope project` writes a COMMITTED declaration (teammates get prompted to trust+install on next session). PENDING the coordinator's open merge (can't write tracked `.agents/settings.json` mid-merge). Caveat: the `developer-wiki` marketplace is a LOCAL directory (`/Users/adrian/code/developer-wiki`) → not reproducible until published to a git marketplace; the official + `buf-plugins` marketplaces are github-sourced → reproducible. The `lodestar` BINARY also isn't git-delivered (install via the one-liner); only its config (`.mcp.json`) is. LSP plugins carry 0 skills ⇒ ~0 always-on cost (keep all: rust-analyzer/typescript/jdtls/pyright). Skill-heavy plugins cost tokens every session.
- **INSTALLED `plugin-dev@official-plugins`** (~2,349 tok always-on) — 8 skills (skill/agent/command/hook-development, mcp-integration, plugin-structure/settings) + 3 agents (agent-creator, plugin-validator, skill-reviewer) + `/plugin-dev:create-plugin`. Strong fit: this repo hand-maintains 5 `celnet-*` agents, custom skills (knowledge-maintenance, verification-loop), 10 `.agents/workflows/*.js`, hooks, and the lodestar MCP wiring — plugin-dev authors/validates exactly those. (Loads next session.)
- **DISABLED `expo`** (~1,801 tok always-on, 16 skills) — zero React-Native anywhere in celnet OR the celnet JVM estate. Pure dead weight; offsets most of plugin-dev's cost. Reversible: `agent plugin enable expo`.
- **PENDING (user call):** `atlassian` (~1,069 tok, 6 Jira/Confluence skills) + `gitlab` (~0 tok, MCP only) — disable unless used for the celnet estate (celnet is GitHub/soarsa).
- **Do NOT add** semantic code-search plugins (`serena`, `lumen`, `sourcegraph`, `greptile`, etc.) — [[lodestar-first]] owns code search/understanding and [[lodestar-no-workaround]] forbids duplicating it. Optional future: `agent-md-management` (fits the heavy GUIDE.md governance). NOTE: `developer-wiki`'s blurb still cites codebase-memory-mcp, which celnet replaced with lodestar ([[lodestar-migration]]) — the wiki serves the broader estate, leave it.

**Installed this session:**
- **`protobuf@buf-plugins`** — Buf's official protobuf skill (added marketplace `buf-plugins` = `bufbuild/plugins`, Apache-2.0). A pure SKILL (no auto-running hooks → low standing cost) that auto-triggers on `**/*.proto`; covers proto design, buf CLI, gRPC/Connect, protovalidate, **schema evolution / breaking-change discipline**, lint. Strong fit for the proto-central contract (`celnet-proto/proto/celnet.proto`, the one unversioned contract — guardrail #9). **Use it** when editing/reviewing the proto (e.g. the RiskService addition): apply its naming/field-numbering/no-break checklist. References on disk: `~/.agents/plugins/marketplaces/buf-plugins/plugins/protobuf/skills/protobuf/references/`. Activates next session / on `/reload-plugins`.

**Deliberately SKIPPED (with reason):**
- `security-guidance@official-plugins` — official + useful BUT always-on: injects a reminder every `UserPromptSubmit`, LLM diff-review every `Stop`, pip-installs the agent SDK at `SessionStart`. That standing per-turn + per-stop cost conflicts with [[token-and-context-discipline]]; the built-in **`security-review`** + **`/code-review`** cover it on-demand at milestones. Revisit only if the user wants always-on security gating.
- `pr-review-toolkit`, `commit-commands` — redundant with built-in `/code-review` + the manual commit flow.
- `42crunch-api-security-testing` — commercial vendor (guardrail #7: OSS/free only).
- Trail of Bits `trailofbits/skills` — reputable but an extra third-party marketplace; built-in security-review suffices for now.

Enabled set (as of 2026-06-29): rust-analyzer-lsp, typescript-lsp, pyright-lsp, jdtls-lsp, playwright, chrome-devtools-mcp, frontend-design, developer-wiki, atlassian, gitlab, protobuf, **plugin-dev** (expo now DISABLED). Code-graph substrate is **lodestar** (replaced codebase-memory-mcp — [[lodestar-migration]]). Discover/manage via `agent plugin {list,install,disable,enable,details,marketplace}` or `/plugin`; `agent plugin details <p>` shows projected always-on token cost.
