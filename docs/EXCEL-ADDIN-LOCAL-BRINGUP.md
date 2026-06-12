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

Run from `/Users/adrian/code/celeroption`. (Currently all four are UP.)

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
- `excel/tools/build_trader_workbook.py` (the workbook generator);
- `celer-logo.png` (rasterized brand mark, root — move under assets or gitignore).

## Trader workbook — `~/Desktop/Celnet-Trader.xlsx`

- Generator: `excel/tools/build_trader_workbook.py` (run with the venv:
  `/tmp/celnet-xlsx-venv/bin/python excel/tools/build_trader_workbook.py`; venv has
  `xlsxwriter`+`pillow`). Logo at `/tmp/celer-logo.png`.
- **Dark Celer theme** (navy canvas, coral/indigo accents, pinwheel + wordmark,
  Anaheim, build-stamp). 5 sheets: Market & Vol (marking + **date-based vol term-
  structure chart**) · **Axes** (multi-pair contribute/receive board) · Structuring
  (PRICE/GREEKS + polymorphic `INSTRUMENT`→PRICE for exotics) · Trading (RFQ +
  SUBSCRIBE) · Risk & Book.
- **VERIFIED:** structural — uses **only the 13 registered functions**, **no
  `_xlfn.` prefix** (Office custom fns resolve as plain `CELNET.X`; the `_xlfn.`
  detour was wrong); functional — `npm --prefix excel run verify:headless` PASS
  (every `CELNET.*` returns real values vs a live edge). It will price the moment
  the add-in's shared runtime is live.

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
