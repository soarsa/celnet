# Excel add-in — local bring-up & trader workbook (session handover)

> Live demo state for the Celnet Excel add-in + the branded trader workbook, on
> Excel-for-Mac. The **1.0-RC is cut + tagged** (`origin/main`, tag `1.0-RC`,
> gated code `a0817d6`); this doc covers the *local Excel demo* built on top of it.
> All changes below are **uncommitted** (working tree) — decide what to land.

## TL;DR root cause (the whole saga)

**TWO distinct root causes, both of the same "node passes, WKWebView fails" family:**

1. **Delivery (`#NAME?`).** Excel-for-Mac's **custom-functions runtime is a stripped
   WKWebView that cannot evaluate the Vite *dev* server's output** (raw `.ts` ES
   modules + `/@vite/client` HMR). The CF runtime silently failed, so `registerAll()`
   never ran — the 13 `CELNET.*` *names* registered (from cached `functions.json`)
   but had **no implementation** → `#NAME?`. **Fix = serve the COMPILED bundle, not
   the dev server.** (The wss connection, the manifest, the namespace were red
   herrings — see the `excel-addin-runtime-delivery` memory.)

2. **Return shape ("add-in error", 2026-06-12).** Once registered, every cell that
   called a *spilling* verb (`PRICE` on a token, `RFQ`, `SURFACE`, `RISK`,
   `POSITIONS`, `LIMITS`, the var/vol-swap spills, …) threw an opaque **"add-in
   error"**, while scalar `PRICE`, `GREEKS`, `STATUS` and `MARK` worked. Cause:
   **Office.js custom functions require a RECTANGULAR 2-D return** (all rows the same
   column count); the spill formatters returned **ragged** matrices — a wide data row
   followed by a single-cell convention footer. `verify:headless` (node) inspects the
   JS value directly and never round-trips it through the host serializer, so it
   stayed green. **Fix = every `format*Spill` funnels through `rectangular()`**
   (`src/functions/shaping.ts`), right-padding short rows with empty cells; guarded by
   a unit test (`test/shaping.test.ts`, `test/rfqPanel.test.ts`).

## The local stack — how to (re)start it

Run from the repo root. (Currently all four are UP.)

1. **Edge** (all services, production posture):
   `source "$HOME/.cargo/env" && CELNET_ACCESS_MODE=enforce CELNET_DEMO_LPS=3 cargo run -q -p celnet-server --example demo_edge`
   → gRPC `127.0.0.1:50551` + WS mirror `ws://127.0.0.1:8081`.
2. **wss bridge** (the HTTPS add-in can't dial insecure `ws://` — mixed content):
   `node excel/tools/wss-bridge.mjs` → `wss://localhost:8443` (TLS via the
   office-addin-dev-certs cert, **dual-stack** so `localhost`/`::1` resolves) →
   `ws://127.0.0.1:8081`.
3. **Excel add-in — COMPILED static bundle (NOT `npm run dev`):**
   `npm --prefix excel run build` then `node excel/tools/serve-dist.mjs`
   → serves `excel/dist` over `https://localhost:3000` (dual-stack). Verify it's the
   bundle, not dev: `curl -sk https://localhost:3000/taskpane.html | grep -c '/@vite/client'` must be **0**, and it must reference `assets/functions-*.js`.
4. **GUI** (works fine, http origin so no wss needed): `npm --prefix gui run dev`
   → `http://localhost:5174` (live to `ws://127.0.0.1:8081`; status ribbon reads
   `live ws://…`).

## Sideload + clean re-register (Excel-for-Mac)

- Manifest (shared runtime, valid): `excel/manifest.xml` → copy to
  `~/Library/Containers/com.microsoft.Excel/Data/Documents/wef/celnet-manifest.xml`.
- Dev cert (one-time, already trusted here): `npx office-addin-dev-certs install`.
- **Cache clear MUST be done with Excel QUIT** (else it won't take):
  ```
  rm -rf ~/Library/Containers/com.microsoft.Excel/Data/Library/Caches/* \
         ~/Library/Containers/com.microsoft.Excel/Data/Library/Application\ Support/Microsoft/Office/16.0/Wef/* 2>/dev/null
  ```
- Reopen Excel → **Insert → My Add-ins → Celnet FX Options** (re-add) → click the
  **Home-tab "Celnet" ribbon button** once to warm the shared runtime →
  `=CELNET.STATUS()` should resolve `connection: UP`.

## What changed (uncommitted) — and what's product-worthy

**Product fixes (worth landing):**
- `excel/manifest.xml` — (a) fixed a **schema-invalid** over-length `GetStarted`
  string (was silently breaking the CF extension point); (b) converted to a
  **shared runtime** (`<Runtimes lifetime="long">`, `SharedRuntime` requirement,
  CF Script/Page → the task-pane page, `<FunctionFile>`) — the reliable Mac pattern.
- `excel/src/functions/functions.ts` — gate `registerAll()` on `Office.onReady`
  (was an eager top-level call that races/silently no-ops before `CustomFunctions`
  exists). **Validated: headless e2e PASS.**
- `excel/src/taskpane.html` — shared page now loads `functions.ts` (registers) +
  `taskpane.ts` (UI).
- **Gate gap to close:** the Excel **manifest schema is not validated in CI/T2**,
  and the **`tsc`/unit web suites were added to T2 this session** but the manifest
  validate is not — add `office-addin-manifest validate` to the t2 recipe.

**Dev-only (DON'T ship to product as-is — localhost hardcodes):**
- the `wss://…:8443` endpoint override injected in `taskpane.html`/`functions.html`
  (production uses a deploy gateway, not :8443);
- `excel/tools/wss-bridge.mjs`, `excel/tools/serve-dist.mjs` (dev bring-up tools);
- `excel/tools/build_trader_workbook.py` (the workbook generator).

## Trader workbook — `~/Desktop/Celnet-Trader.xlsx` (multi-asset refresh)

- Generator: `excel/tools/build_trader_workbook.py` (venv:
  `/tmp/celnet-xlsx-venv/bin/python excel/tools/build_trader_workbook.py [out.xlsx]`;
  venv has `xlsxwriter`+`pillow`+`openpyxl`). Designed competitor-first (Bloomberg OVDV
  bump-and-watch, Murex one-screen term-sheets) → a SOTA multi-asset demo.
- **Dark theme** (navy canvas, coral #ff7357 / indigo #6b6bf5 accents,
  wordmark, 6px coral cap-rail, build-stamp). It tells **one
  trading day** across **10 sheets** (tab strip = the day): Cover & Legend ·
  Market & Vol (live observables + editable ATM/RR/BF smile marking → MARKSURFACE +
  the vol term-structure chart + SURFACE VV-vs-SABR) · **FX Majors** (the full 24-arm
  flow — vanilla/RR/straddle/strangle, USDJPY barrier/touch/digital, GBPUSD
  TARF/accumulator, var/vol-swap) · **FX EM & NDF** (USDBRL/USDKRW NDFs) · **Metals**
  (first-class full-arm: XAU seagull, XAG one-touch, XAU TARF + SABR surface) ·
  **Equity** (vanilla/perpetual/listed-future-option + the labelled **capability-wall**
  demo) · **Commodity** (Brent/WTI leaves) · **Crypto** (BTC LINEAR vs INVERSE_COIN
  side-by-side + ETH perpetual) · **Cross-Asset RV** (gold-vs-USD vega, BTC-vs-AAPL) ·
  **Risk Cockpit** (server-aggregated RISK cube + POSITIONS + LIMITS RAG in one
  numeraire). The desk sheets share one `desk_sheet` factory (column-stable scenario
  ladder: label | product | editable terms-range | INSTRUMENT token | INDEX-extracted
  scalar premium/Δ/ν | one featured full PRICE+RFQ spill per desk). The capability
  matrix is enforced in ONE place + shown as a feature.
- **VERIFIED (validation harness, kept tooling):** `excel/tools/wb_extract.py`
  resolves every `INSTRUMENT` cell's terms-range; `excel/scripts/wbShape.ts` runs the
  REAL `shapeSpecInstrument` over them → **33/34 build, 1 = the intentional Equity
  capability-wall BARRIER**. Structural: **only the 13 registered functions, no
  `_xlfn.`**, `<calcPr fullCalcOnLoad="1">` (auto-recompute on open), all cross-asset
  cells leaf-only (vanilla/perpetual/future-option) except the wall. Layout is
  collision-free (the dense ladder uses non-spilling `=INDEX(CELNET.PRICE(tok),1,2)`
  scalars; full spills get dedicated roomy blocks). It prices the moment the add-in's
  shared runtime is live (the families are conformance-proven; cross-asset live-probed).
  Regenerate + re-validate: `… build_trader_workbook.py /tmp/wb.xlsx && … wb_extract.py
  /tmp/wb.xlsx && (cd excel && node --import tsx scripts/wbShape.ts)`.

## Pending (next step)

The **rectangular-spill fix is built and being served** (`functions-*.js` rebuilt;
serve-dist sends `Cache-Control: no-store`, so Excel refetches on reload). The add-in
was confirmed *loaded + connected* (a WebKit process held ESTABLISHED sockets to the
`:8443` bridge) — so this is no longer a load/registration problem. To pick up the
fixed bundle the running shared runtime must reload:

1. **Close the workbook and quit Excel** (the shared runtime is already warmed with
   the *old* chunk; quitting forces a clean re-fetch). Manifest `<Version>` was
   bumped to `1.0.1.0` to help Excel treat it as updated.
2. Reopen Excel → re-open `Celnet-Trader.xlsx` → click the **Celnet** ribbon button
   once to warm the shared runtime.
3. Force a full re-evaluation of the cached error cells: **Find & Replace (⌘⇧H)** →
   Find `CELNET.` → Replace `CELNET.` → **Replace All** (re-enters every formula).

Every `CELNET.*` cell should now price. If anything still errors, capture the runtime
page's WKWebView console (right-click the task pane → *Inspect Element*) — the only
host-side observability — and read the thrown error verbatim.
