---
name: use-agent-teams-and-loop
description: "Operator preference (2026-07-01): use AGENT TEAMS (parallel Agent spawns) and /loop MORE FREQUENTLY — default to parallel fan-out for any decomposable work, keep the loop iterating with tighter cadence; fall back in-session only for synthesis/coordination/small edits."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 1e7d532e-2588-4979-97f3-496c6259eb3e
---

**Operator directive (2026-07-01):** "use agent teams and /loop more frequently."

**Why:** the work decomposes well (per-dimension/per-crate mapping, per-ADR drafting, per-finding verification, per-lane builds) and the operator values momentum + throughput + breadth of coverage. I had been doing some decomposable work serially in-session, which is slower and burns my context.

**How to apply:**
- **Default to parallel agent teams** for ANY decomposable work — mapping (one agent per subsystem/dimension), drafting (one agent per ADR/doc), verification (adversarial verifiers per finding), implementation lanes (one per disjoint crate), research (one per topic). Spawn them in ONE message so they run concurrently.
- **Keep the /loop iterating** with tighter cadence + more per-iteration fan-out; don't sit idle between agent returns — kick off the next parallel wave.
- **Fall back in-session only** for synthesis (merging agent outputs), coordination/git/landing, small edits, and operator-facing presentation.
- **Right-size** the fan-out to the task ([[token-and-context-discipline]]) — 3-6 agents for a mapping/drafting wave is typical; don't over-shard trivial work.
- Persist each agent's output to a scratchpad/doc as it returns (durable across compaction), then synthesize.

Applied first on the target-architecture program: 8 explorer agents mapped the 6 dimensions in parallel; the 4 ADRs (0013-0016) drafted as a 4-agent team. See [[planned-builds-no-blocking]], [[target-architecture-program]].
