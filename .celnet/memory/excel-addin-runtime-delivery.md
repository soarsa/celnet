---
name: excel-addin-runtime-delivery
description: "Excel-for-Mac custom functions need a COMPILED bundle (not the Vite dev server's raw .ts/HMR) + a shared runtime + Office.onReady-gated registration; the CF runtime fails silently. AND every spilled 2-D return MUST be rectangular or the cell throws an opaque 'add-in error'."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ce643053-e1eb-472e-820d-f79a68761a05
---

Getting the Celnet Office.js add-in's `CELNET.*` custom functions to work in
Excel-for-Mac took a long debugging chain. The ACTUAL root cause, found only by a
multi-agent diagnostic workflow after many wrong turns:

**Excel-for-Mac's custom-functions runtime is a stripped WKWebView that cannot
evaluate the Vite *dev* server's output** — raw `.ts` ES modules + the
`/@vite/client` HMR client. The task pane (a full WebView) ran it fine, so it
*looked* loaded, but the CF runtime silently aborted before `CustomFunctions.associate`
ran. The 13 function NAMES registered from cached `functions.json` (Excel's Wef
registry even showed them) but had NO implementation → `#NAME?` then `#VALUE!`/blank/
"add-in error". **Serve the COMPILED bundle (`vite build` → a static HTTPS server),
not `npm run dev`.**

**Why:** a green metadata registration (names known) is NOT a working function
(implementation associated). Same family as [[dev-posture-masks-prod-defects]] and
[[deferred-e2e-defect-reservoir]] — the `verify:headless` e2e passed because it runs
in **node** (which evaluates the dev .ts graph fine); the failure was webview-only,
so no headless test caught it.

**How to apply (Excel-for-Mac Office.js add-ins):**
1. Serve a **compiled** bundle to Excel, never the dev server. `npm run build` →
   serve `dist` statically over HTTPS with the office-addin-dev-certs cert. Confirm
   `curl …/taskpane.html` has NO `/@vite/client` and references `assets/*.js`.
2. Use a **shared runtime** (`<Runtimes lifetime="long">`, `SharedRuntime` set, CF
   Script/Page → the task-pane page) — task pane + functions in ONE runtime is the
   reliable Mac pattern vs a flaky JS-only runtime.
3. Gate `registerAll()` (the `CustomFunctions.associate` calls) on `Office.onReady`,
   not bare module top-level — top-level races and silently no-ops if `CustomFunctions`
   isn't defined yet.
4. **Custom functions use the plain `=NAMESPACE.FN()` form** — NOT the `_xlfn.`
   prefix (that's for built-ins). An externally-generated workbook (xlsxwriter etc.)
   must write `CELNET.PRICE(...)`, no prefix.
5. An HTTPS add-in cannot dial insecure `ws://` (mixed content) → bridge `wss://`
   (dual-stack — `localhost` resolves to `::1`) → the plain-`ws` edge.
6. Validate the manifest (`office-addin-manifest validate`) — a schema-invalid bit
   (e.g. an over-length LongString) silently breaks the CF extension point. This is
   NOT in CI/T2 — a gate gap to close.
7. Cache clear must be done with Excel **quit** (Wef + Library/Caches), then re-add.
8. **Every spilled 2-D return MUST be RECTANGULAR** (all rows the same column
   count). A ragged matrix — a wide data row + a single-cell footer — throws an
   opaque **"add-in error"** in the host (registration is fine; the name resolves;
   the *return* fails the serializer). Scalar/already-rectangular verbs (scalar
   `PRICE`, `GREEKS`, `STATUS`, `MARK`) work, so it looks product-specific. node /
   `verify:headless` inspects the JS value directly and never round-trips it through
   the host serializer → stays green (same trap as #1, and as
   [[dev-posture-masks-prod-defects]] / [[deferred-e2e-defect-reservoir]]). Fix:
   funnel every `format*Spill` through a `rectangular()` padder; gate it with a unit
   test asserting equal row widths (2026-06-12, `excel/src/functions/shaping.ts`).

Diagnosing which layer: a WebKit process holding ESTABLISHED sockets to the wss
bridge ⇒ the add-in IS loaded + connected (so not #1/registration) ⇒ suspect the
return shape (#8). Excel's `~/Library/Containers/com.microsoft.Excel/` is
TCC-protected — the agent shell gets `Operation not permitted`, so cache reads/clears
and sideload are USER steps.

Full local bring-up + the trader workbook: `docs/EXCEL-ADDIN-LOCAL-BRINGUP.md`.
