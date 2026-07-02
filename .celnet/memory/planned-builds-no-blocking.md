---
name: planned-builds-no-blocking
description: "Operator directive (2026-06-10) — never fully block on another session's long-running task; build only what changed, plan builds across sessions, share gate results instead of re-running."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 022c5024-0efd-40df-b066-eb5f16656b52
---

Operator directive to the mesh (2026-06-10): **"we need to not block on long running tasks, and
build only when needed across the sessions with planning."**

**Why:** the §4.1 full-stop turn-taking (one session gates, the other sits idle for the whole
multi-hour window) wastes wall-clock in both directions; and the lanes were over-building —
each lane re-ran a full-workspace gate that the other session had just run on overlapping code.

**How to apply:**
1. **No full-stop blocking.** During another session's long gate, keep running with a BOUNDED
   slice (one task, `--jobs 2-3`, leave most cores to the gate owner). Full-yield only for the
   most starvation-sensitive moments (their announced critical seconds), not whole windows.
2. **Build only what changed.** Per iteration: `just check-crate <crate>` / `check-changed` /
   `cargo … -p` / file-scoped `cargo mutants --file`. NEVER a workspace build/clippy inside a
   lane step. (This is CLAUDE.md's own incremental rule — enforce it in workflow agent prompts.)
3. **One milestone gate per merge window, planned.** The full `just check`-equivalent runs ONCE
   per merge window, by whoever merges LAST into it; the result is posted to §6 (a gate ledger:
   commit hash + exits + section counts) and the other session TRUSTS it instead of re-running.
4. **Plan build windows in §6 ahead of time** (who gates what, when) rather than reactive
   pause/resume ping-pong.

Related: [[mesh-compute-contention]], [[cargo-gate-environment-pitfalls]], [[token-and-context-discipline]].
