---
name: agent-worktree-base-and-cross-cuts
description: "isolation:worktree bases on main not your branch; client-auth changes are a cross-cut, not parallelizable from main — gate them with the server under Enforce"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b757c952-cc98-456a-9272-5cb8b360235d
---

During the stream/WS caller-authz cross-cut I fanned out GUI/Excel client work as `isolation: worktree` agent lanes. Two things bit:

**Why:** (1) `isolation: worktree` created the agent's tree from **main's HEAD, not the checked-out feature branch**, so the lanes lacked the branch's `StreamAuth` contract + server enforcement. The GUI agent correctly **refused** to emit a GUI-only frame against a contract that (in its main-based tree) didn't exist — that would be contract drift / fake depth. (2) The client auth changes are a **cross-cut** (every client depends on the one contract + the server gate), not independent parallel tasks — splitting them off ahead of the contract is exactly what the guardrails forbid.

**How to apply:**
- `isolation: worktree` bases on the default branch (main). For branch-dependent work either do it **on the branch in the main worktree** (sequential, to avoid git-index races when an agent runs `git add -A && commit`), or manually `git worktree add <path> <new-branch-off-the-feature-branch>` and point the agent there.
- Only **toolchain-disjoint** lanes parallelize on the single M4 (one cargo + N node/TS). Two heavy cargo builds contend → serialize them. TS client work IS addable to a main-based tree (clients hand-encode the wire JSON), but still gate it **with** the server under `Enforce`, never standalone.
- A new wire **control-verb must be added to BOTH the decoder AND the router classification** — they were duplicated lists in `ws/mod.rs`; the live e2e under `Enforce` caught the `authenticate`-frame omission (it fell through to `handle_unary`, sessions stayed anonymous) that the Permissive unit test could not. Now guarded by a decoder/router lockstep test. Reinforces [[deferred-e2e-defect-reservoir]] + [[dev-posture-masks-prod-defects]].
- Salvage value: a misbased worktree's **disjoint** changes still graft cleanly onto the branch (`git -C <wt> diff HEAD -- <subtree> | git apply`). The Excel lane's 139-line change was reused this way.

Related: [[planned-builds-no-blocking]], [[api-first-client-parity]], [[w2-parallel-session-collision]].
