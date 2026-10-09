---
name: multiasset-experience-program
description: "Multi-asset GUI+Excel trader-experience program: design synthesis + an ordered increment plan; what's shipped, deferred, and remaining."
metadata: 
  node_type: memory
  type: project
  originSessionId: be8a831f-3890-4505-9580-acb9cab49dbd
---

The GUI + Excel UX program to surface the FULL multi-asset capability (the server +
Excel CELL functions already cover all 5 asset classes × products; the **clients'
trader-facing UX was FX-first**). Started 2026-06-12. Design synthesized by a dynamic
workflow (`multiasset-experience-design`); the plan = 8 ordered, independently-gateable
increments; **push checkpoints at logical intervals**, every gui-touching push runs the
full vitest + build + **live Playwright e2e/axe** (per [[deferred-e2e-defect-reservoir]]).

The keystone is `gui/src/products/capability.ts` — the executable (asset-class ×
product-arm) matrix mirroring `docs/EXCEL-INTEGRATION.md §3.3.1` (FX/METAL price all 24
arms; equity/commodity/crypto price only vanilla+perpetual+listed-future-option), whose
dim REASON is the SAME sentence the server `price_cross_asset` + the Excel build-time
guard reject with — the matrix made visible, one source ported to Excel later.

**Shipped + pushed (origin/main):**
- `4874b32` — increments 1-3: capability.ts + `assetClass` on `ProductBuildCtx` (from
  `app.underlier.assetClass`) + asset-class-aware `StructureGallery` (FX/metal full
  catalogue; cross-asset classes dim the FX/metal-only families with the reason, keyboard
  skips them, auto-reselect when the selection becomes unpriceable).
- `6700df0` — increment 5 (partial): `GreeksStrip` class-correct rho relabel (equity =
  dividend-yield rho, crypto = funding rho; wire fields unchanged, labels only).
- `aab309a` — increments 6+7+8: the WHOLE Excel task-pane uplift, driven by a dynamic
  workflow (4 disjoint modules built in parallel — `taskpane/capability.ts`,
  `instrumentBuilder.ts`, `dealerPanel.ts`, `celnet-tokens.css` — then an Opus integration
  agent rewired `taskpane.{html,ts,css}`). The pane went from hardcoded FX-vanilla +
  single dealer + raw hex + zero motion → class-aware builder (asset-class → underlier →
  priceable-arm-with-dimming → terms) + ranked multi-dealer panel + shared CelNet tokens +
  "alive" motion (quote-flash, pinwheel spin/heartbeat, pulse, last-look ring). Gates:
  excel tsc + 429 vitest + vite build + verify:headless. Established pattern: workflow
  BUILDS disjoint modules / integrates, coordinator GATES (tsc+vitest+build+verify:headless;
  no live Excel here) + pushes.

- `97dddba` — increment 4: equity/commodity/crypto PERPETUAL + LISTED_FUTURE_OPTION
  reachable in the GUI ticket (Opus workflow agent, approach A). `perpetual.tsx` +
  `listedFutureOption.tsx` made asset-class-agnostic via ONE additive overlay seam —
  `crossAssetOverlayFor(ctx)` (crossAsset.tsx) resolves the active underlier's `Underlying`
  (+ crypto `settlementStyle`) only for a true cross-asset underlier (FX/metal byte-identical),
  threaded via a new optional `ctx.underlier` (off `app.underlier`) + `withCrossAssetOverlay`
  (seed.ts); `applicableClasses` widened to `ALL_ASSET_CLASSES` so the gallery shows distinct
  selectable cards. +17 round-trip tests. Resolved the deferred AppContext-groundwork blocker
  (added `ActiveUnderlier.settlementStyle`).

**PROGRAM COMPLETE — all 8 increments shipped + pushed.** (ConventionRow was already
class-correct via server-resolved conventions; GUI quote-flash motion already existed via
PriceTile; RiskWorkspace shock-axis relabel left as a low-value future polish.) Pattern that
worked: dynamic workflow DESIGNS (judged synthesis) / BUILDS disjoint modules in parallel /
INTEGRATES via one coherent agent; coordinator GATES (gui: tsc-b+vitest+build+playwright-e2e;
excel: tsc+vitest+build+verify:headless) + pushes at each checkpoint. Optimal model per task
(Opus design/integration/architecture; Sonnet mechanical ports + CSS). GOTCHA: a stale
`vite preview` on the e2e PREVIEW_PORT (left by a prior e2e run) silently hangs the next
Playwright run (0 output → timeout); kill the port holder before re-running.

Full design + the increment table: workflow run `wf_3a8dc366-aaa` result (transcript dir
under the session's subagents/workflows). See [[excel-addin-runtime-delivery]] for the
Excel-side rectangular-spill + cross-asset-guard work this builds on.
