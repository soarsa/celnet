---
name: deferred-e2e-defect-reservoir
description: A deferred live-e2e suite is a defect reservoir — the GW1 GUI layout break survived 3 waves of green vitest. Gui-touching gates MUST run the live Playwright e2e.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 022c5024-0efd-40df-b066-eb5f16656b52
---

The live GUI rendered **broken from GW1 through 3 subsequent GUI waves** (2026-06): GW1 deleted
PairStrip but `Shell.module.css`'s `.main` grid kept the pre-GW1 four-row layout, so the workspace
canvas auto-placed into a **0-height row** in every real browser. **567 green vitest runs never saw
it** (jsdom doesn't lay out). It was caught only when the GW2-deferred Playwright suite was finally
run — first-ever run: 7 failures, ALL genuine product defects (plus axe critical/serious), zero
stale selectors. Fixed in `b28ba63` (gate: vitest 567 · live e2e 13/13 · 0 serious/critical axe).

**Why:** unit/jsdom suites validate logic, not layout/focus/contrast/keyboard-reachability. A
deferred e2e suite silently accumulates exactly those defects while everything stays "green".

**How to apply (mesh directive, both sessions, 2026-06-10):** any gate touching `gui/` runs the
**live Playwright e2e + axe** (it is warm + fast: ~23.5s, edge pre-built — `npm ci` first; the
self-referential `gui/node_modules` symlink was untracked, worktrees just `npm ci`). Never defer a
new e2e suite past the wave that creates it. This includes seemingly non-GUI lanes whose changes
touch gui mirrors/tests (e.g. ADR-0008 Wave S renaming `gui/src/data/contract.ts`).

Related: [[mesh-coordinator-resume]], [[cargo-gate-environment-pitfalls]].
