---
name: token-and-context-discipline
description: "How to run workflows token-optimally while never losing context: externalize durable state to disk; pass seams by file-reference not paste; right-size fan-out."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 4e9a6b38-74d9-4a36-9109-4d49116555b3
---

Operate token-optimally **without ever losing context**. Two halves:

**Never lose context — externalize durable state to disk (this is the safety net for any summarization/clear):**
- Keep `CLAUDE.md` ledger (newest-first, one line per milestone) + the auto-memory (`MEMORY.md` + files) + `docs/` corpus current at **every milestone boundary**. The next context window must be able to fully reconstruct from: ledger + memory + `git log` + the lodestar graph/knowledge — never from chat history alone.
- Record in-flight workflows (id, scope, "what to do on completion") in the CURRENT active ▶ program memory so a post-clear session resumes losslessly.
- After each verified milestone: commit, update ledger + memory, re-index (post-commit hook). A clear is then safe.

**Token optimization inside dynamic workflows (the big spenders — a single run hit ~1.8M tokens):**
- **File-based seam handoff**: a foundation/contract agent should WRITE its contract-delta/seam report to a scratch file (session dir or a docs scratch), and downstream agents READ it — instead of threading the full multi-KB text through every later prompt (which multiplies across parallel lanes). Pass a path, not a paste.
- **Reference, don't embed**: point agents at doc paths (`docs/…`) + the ledger + lodestar tools, rather than pasting corpora into prompts.
- **Right-size fan-out**: `parallel` only for genuinely independent work; prefer `pipeline` (no barriers); don't spawn agents for trivial edits. Disjoint file ownership per lane.
- **Per-agent context is already fresh** — subagents don't inherit the orchestrator's history, so workflows inherently "clear context" per task; lean on that.
- **Verify by running gates** (cheap, authoritative) rather than re-deriving; I independently run `just check` / `npm run build` / e2e for ground truth.
- **Background + notification**, never poll a running workflow. Don't launch a redundant/overlapping workflow — and never two tree-building workflows on the live tree at once.
- In my own replies: be concise; relay conclusions, not file dumps (delegate broad reads to agents/Explore).

**Memory hygiene — keep the auto-memory LEAN, meaningful, and lodestar-based (operator, 2026-07-01):**
- **lodestar is the BASIS for durable "why".** Invariants / decisions / rationale / ADRs → lodestar claims (`knowledge_put`) + `manage_adr`: graph-anchored, self-invalidating (goes stale when code changes), and queried **on-demand** (`knowledge_get` / `evidence_pack`) so it costs **zero standing context** and is always current. This holds the BULK of durable knowledge — do NOT copy it into memory files.
- **auto-memory is a thin OPERATIONAL layer** for only what lodestar can't anchor to code: cross-session handoffs, in-flight state, orchestration pointers, durable user preferences/feedback, external refs. If a fact is about *the code's why*, it belongs in lodestar, not here.
- **`MEMORY.md` is loaded into EVERY session** — it is the one always-on context cost, so it is the bloat surface. Keep it a terse one-line index; a memory earns a line only if current + operational/preference/pointer. Don't restate CLAUDE.md's hard guardrails (also always-loaded) — link to it.
- **Prune on completion.** When work lands / a handoff is consumed / a plan is superseded, ARCHIVE the memory (`git mv` its file to `.celnet/memory/_archive/`, drop its `MEMORY.md` line) — don't let handoffs/snapshots accrete. Archiving is lossless (the mirror keeps `_archive/`); the loaded set stays lean. `celnet-memory-sync.sh snapshot` excludes `_archive/`.
- **Precise `description:` fields** drive relevance-matched recall — vague/overlapping descriptions cause over-recall (noise); specific ones surface only what's relevant.
- **Durability ≠ load**: `.celnet/memory/` (mirror + `_archive/`) preserves everything for restore/sharing; the live dir + `MEMORY.md` stay the curated CURRENT set.
