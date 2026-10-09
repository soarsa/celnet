---
name: arch-program-status
description: Main-line architecture program (items A–L) is mostly LANDED on origin/main; only G/D/J + orchestration tail remain — the GUIDE.md resume anchor is stale
metadata: 
  node_type: memory
  type: project
  originSessionId: 5ff9525a-81fa-41e7-a4f8-745f35d76a80
---

The **GUIDE.md resume anchor is STALE** (it predates a large team push). As of
`origin/main` HEAD **`7878048`** (verified 2026-06-30, after fast-forwarding local main
which was 38 commits behind), the architecture program **items A, C, E, F, H, L are all
DONE and landed** on origin/main (merge `682eefd→0da3eeb`, "arch/ship-program", full t2
**19/19 GREEN**). Concretely:

- **C** (`price_instrument` god-fn → `ProductEngine` registry) — DONE: `ProductEngine`
  lives in `crates/celnet-server/src/pricer/engines.rs` (≈24 engine unit-structs);
  `price_instrument` is now ~104 lines (decode→guard→dispatch). Do NOT "start item C".
- **E** risk-cube via `ExoticLegPricer` (exotics/gpu deps inverted) — DONE.
- **H** single error → `tonic::Status` taxonomy — DONE.
- **L** dev-dep back-edges + `typecheck:test`/`:e2e` wired into t2 — DONE.
- **A** carry-seam-to-edge + **F** carry→sensitivity mapper — DONE (earlier).

**Still outstanding** (per the newest ledger "▶ NEXT" on current main):
- **G** — WS-codec-from-proto (generate/derive the WS codec from the proto vs hand-maintained).
- **D** — 4 dormant crates.
- **J** — restrictive-principal e2e.
- Build the ratified **cross-session orchestration system** + write `docs/acceptance/*.json` +
  final integration-verify.

**Before doing any of G/D/J: re-read the CURRENT main ledger head** — origin/main is a
fast-moving "treadmill" ([[dev-posture-masks-prod-defects]] context), so re-verify status
and re-gate. Each remaining item is a cross-cut needing full t2 + live GUI/Excel e2e under
`CELNET_ACCESS_MODE=enforce` before landing. This program is on **main**, a DIFFERENT
workstream from the FI branch ([[fi-platform-branch-program]]) which must never merge to main.
