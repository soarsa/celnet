---
name: w2-parallel-session-collision
description: W2 multi-asset wave — two sessions shared ONE working tree and both built it; the worktree-per-session fix; preserved cross-check branches.
metadata: 
  node_type: memory
  type: project
  originSessionId: 1456ef44-c6a6-4dcd-b86f-3ee99371c4fa
---

On 2026-06-08, driving W2 (multi-asset linear + metals) as coordinator, a collision
surfaced: **two Claude sessions were operating in the SAME working tree**
(`/Users/adrian/code/celeroption`) and BOTH implemented W2.

- I (coordinator) committed the **W2 contract freeze** locally as `325cfab` (proto
  `FxForward=26`/`FxSwap=27`/`Ndf=28` + `Underlying.metal=3` + `Metal`/`FixingSource`;
  types `Metal`/`MetalPair`; FX byte-identical; 291/291). Then ran a dynamic Workflow that
  built + **adversarially-verified** two disjoint lanes in isolated worktrees:
  `celnet-linear` (forward/swap/NDF, commit `7a40ba0`, 20 tests, independent discount-bond
  oracle) and conventions/calendar **>75-pair + XPT/XPD + metal crosses** (commit `28157b6`,
  98 tests, Hinnant rata-die oracle).
- Meanwhile the **parallel session** (nominally docs/capabilities — see
  [[parallel-doc-session]]) expanded into code and built a COMPLETE server-side W2 **in the
  shared tree, on top of my freeze**: its own `celnet-linear` + server `price_linear_instrument`
  dispatch + `LinearValidity` product×underlying matrix + `ws/codec.rs` mirror +
  `celnet-parity/tests/linear.rs` + root/registry deps. It was caught **mid-gate** (live
  `clippy`/`nextest` procs) about to commit. The 5-client surfacing + golden vectors +
  `verification-coverage 21/21` + docs were still OPEN on its side.

**Root cause:** two sessions, one working tree + one git index ⇒ any `add`/`commit`/`checkout`
from one corrupts the other; and the lane board was claimed AFTER building, not before, so both
took W2 despite the board assigning me the proto window.

**The fix (orchestration rule):** **one session per git worktree — never share the main tree.**
Claim the board lane (commit-only-that-file, push) BEFORE building. Index-mutating git ops in a
shared tree are unsafe; only ref-ops (`git branch`/`git push <ref>`) + reads are collision-safe.

**Nothing lost** — my verified work is preserved on origin as cross-check branches:
`crosscheck/w2-contract-freeze` (`325cfab`), `crosscheck/w2-a-linear-verified` (`7a40ba0`),
`crosscheck/w2-b-breadth-verified` (`28157b6`). These are a second independent implementation to
DIFF against the parallel session's W2 for correctness. Links: [[api-first-client-parity]],
[[token-and-context-discipline]], [[parallel-session-model]].
