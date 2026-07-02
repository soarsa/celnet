---
name: mesh-compute-contention
description: "Cross-session compute/process collision on the shared M4 — session-B's global `pkill -f nextest` + a saturating `just check` kill the coordinator's gates and time out conformance at 120s. How to coexist."
metadata: 
  node_type: memory
  type: project
  originSessionId: 022c5024-0efd-40df-b066-eb5f16656b52
---

When a concurrent session (session-B) runs its critical-path `just check` in the
shared repo on the single M4, two collisions hit the coordinator's parallel lane:

1. **Global `pkill -f nextest`** — session-B's cleanup loop runs
   `pkill -9 -f "cargo nextest"` / `pkill -9 -f nextest`, which matches by command
   string across ALL processes on the machine. It kills the coordinator's
   `nextest`-based gates too (seen as exit 144 / `Signalled(15)`), including
   `cargo mutants --test-tool nextest` (mutants drives nextest). **Workarounds:**
   run tests with plain `cargo test` (no "nextest" in the command), use
   `cargo mutants --test-tool cargo`, or run the prebuilt test binary directly.

2. **Compute saturation** — a workspace-wide `just check` (clippy across ~40
   crates + nextest) plus the coordinator's own heavy build saturates the M4. A
   normally-fast test (`celnet-cli::conformance cli_prices_match_the_golden_corpus`)
   then **TIMES OUT at the 120s nextest cap** and fails session-B's gate
   (`CLEAN_EXIT=100`) — a contention artifact, not a logic bug. The coordinator's
   thrashing was a contributor.

**Coexistence rule (binding):** the coordinator must keep ≤1 LIGHT lane concurrent
with session-B's critical path. `cargo check`/`clippy` are fine (light, dodge the
pkill). Heavy RUNS — `cargo test` execution, `cargo mutants`, loom — must WAIT for
a SUSTAINED-quiet window (≈3 min of no `clippy-driver|rustc|cargo nextest` from any
session; poll `pgrep`). Compile-verify now, gate the heavy runs in the quiet window,
then FF-merge. Never co-run mutation with session-B's `just check`.

Related: [[mesh-coordinator-resume]], [[w2-parallel-session-collision]],
[[token-and-context-discipline]].
