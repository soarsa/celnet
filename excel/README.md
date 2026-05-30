# Celnet Excel add-in (`celnet-excel`)

An Office.js custom-functions + task-pane add-in that brings Celnet FX-options
pricing into Excel over the **single, current Celnet contract** — the
`celnet-server` WebSocket JSON mirror (`crates/celnet-server/src/ws`). Excel is an
**edge consumer**, exactly like the React GUI and the SDK: every number is the
server's libm-core value, so a cell is **bit-identical** to the GUI, the CLI, and
a `celnet-client` call. The add-in adds **no pricing logic** — it shapes requests
in the contract and renders typed results, with the resolved **convention shown
alongside every value** (the #1 cure for FX-options mismarks).

Design: `docs/EXCEL-INTEGRATION.md`. This project is **hand-rolled** (Vite +
TypeScript strict, no Yeoman / no network generator) and uses **only OSS,
permissively-licensed** dependencies (Office.js is royalty-free; no PyXLL, no
commercial add-in SDK).

## Worksheet functions (`CELNET.*`)

| Function | Shape | Contract path |
|---|---|---|
| `=CELNET.PRICE(pair, tenor, strikeOrDelta, callPut, notional)` | scalar premium (two-way mid) | `request_quote` |
| `=CELNET.GREEKS(pair, tenor, strikeOrDelta, callPut, notional)` | 13×2 spill `[name, value]` + convention footer | `request_quote` (Greeks) |
| `=CELNET.SURFACE(pair, tenor)` | smile spill (delta pillars × vol) + arb/convention footer | `get_smile` |
| `=CELNET.RFQ(pair, tenor, strikeOrDelta, callPut, notional)` | 1×4 spill `[bid, offer, quoteId, validUntil]` + footer | `request_quote` |
| `=CELNET.SUBSCRIBE(pair, tenor, strikeOrDelta, callPut, notional)` | **streaming** live two-way; re-ticks; stale-aware | `subscribe`/`update` (multiplexed) |
| `=CELNET.MARK(pair, tenor, pillar, vol, [comment])` | status spill `[status, version, detail]` — **two-phase, idempotent**; commits via the task pane | `mark_surface` (on confirm) |

`strikeOrDelta` accepts an absolute strike (`1.12`), a delta string (`25dP`,
`10dC`), or `ATM`/`DNS`. `callPut` is `C`/`P`. `CELNET.MARK` never writes on a
recalc: it **stages** under a deterministic idempotency key and the trader
confirms in the task pane (or the server four-eyes it) — so a thousand recalcs
produce at most one mark.

### Streaming, staleness, and click-to-trade

- One workbook opens **one** multiplexed `StreamSession`; identical-argument live
  cells **share one subscription** (the add-in ref-counts subscribers and tears a
  server subscription down on the last `onCanceled` — no orphans).
- Each subscription tracks the wall-clock of its last server frame (snapshot /
  update / **heartbeat**). On a missed heartbeat the cell flips to a visible
  **stale** state (`… 0.04099/0.04254 (stale)`) — it is **never** shown as live
  when the stream has silently frozen. A sequence gap triggers a server-assisted
  `resync`; a reconnect re-subscribes every live cell and resyncs from its last
  good sequence.
- Click-to-trade is a **task-pane** button bound to a live RFS token — a cell
  never trades.

## Build & test

The shell does not need the Rust toolchain for the add-in itself. Node v26 / npm
11 are assumed.

```bash
cd excel
npm install            # OSS deps only
npm run typecheck      # tsc --noEmit (strict)
npm run build          # tsc --noEmit && vite build  → dist/
npm test               # vitest: shaping / formatting / dedup / stale / ticket logic
```

`node_modules/` and `dist/` are gitignored.

### Headless end-to-end verification (the deployment-gate substitute)

There is **no Excel application in this repo**, so the actual in-Excel grid render
is a **deployment gate** you run in your own Excel (below). What is verified
**headlessly** is the *full chain through the exact code path Excel's custom
functions call* — the request shaping, the WS transport (with a node `ws` socket
instead of the browser one, via the same transport seam), and the **live
`celnet-server` WS mirror**:

```bash
# Build the server once (Rust toolchain on PATH), then run the headless e2e.
source "$HOME/.cargo/env" && cargo build -p celnet-server
cd excel && npm run verify:headless
```

It spawns the real server, drives `PRICE`/`GREEKS`/`SURFACE`/`RFQ` and a streaming
`SUBSCRIBE` over the live mirror, asserts the values, and kills the server. It is
fully timeout-bounded.

## Sideloading into Excel (deployment gate — run in your own Excel)

The add-in is served over HTTPS. For local sideloading you need a trusted dev
certificate for `https://localhost:3000`.

1. **Trust a dev cert** (one-time). Either use the OSS `office-addin-dev-certs`:
   ```bash
   npx office-addin-dev-certs install
   ```
   or your own `mkcert localhost`. Configure the Vite dev server to use it (set
   `server.https` in `vite.config.ts` to the generated key/cert), then:
   ```bash
   cd excel && npm run dev      # serves https://localhost:3000
   ```
   The manifest's URLs (`https://localhost:3000/...`) must be reachable.

2. **Point the add-in at your edge.** Before opening Excel, set the WS endpoint
   the functions dial. The default is `ws://127.0.0.1:8081`; to use another edge,
   inject a global at sideload time (e.g. add
   `<script>globalThis.CELNET_WS_ENDPOINT="ws://your-edge:PORT"</script>` to
   `dist/functions.html`/`taskpane.html`, or set it from the task pane). Run a
   `celnet-server` and note its `WS-mirror ws://HOST:PORT` line.

### Excel for Windows / Mac (desktop)

3. Put `manifest.xml` in a trusted catalog:
   - **Windows:** create a network share, add it as a *Trusted Add-in Catalog*
     in *File → Options → Trust Center → Trust Center Settings → Trusted Add-in
     Catalogs*, tick *Show in Menu*, restart Excel. Then *Insert → My Add-ins →
     Shared Folder → Celnet FX Options*.
   - **Mac:** copy `manifest.xml` to
     `~/Library/Containers/com.microsoft.Excel/Data/Documents/wef/`, restart
     Excel, then *Insert → My Add-ins → Celnet FX Options*.

4. In a cell, type `=CELNET.PRICE("EURUSD","1Y",1.12,"C",1000000)` and press
   Enter; try `=CELNET.GREEKS(...)` (spills 13 Greeks + a convention footer),
   `=CELNET.SURFACE("EURUSD","1Y")`, `=CELNET.RFQ(...)`, and
   `=CELNET.SUBSCRIBE(...)` (a live, stale-aware cell). Open the **Celnet Ticket**
   task pane (Home tab → Celnet FX Options) for RFQ + click-to-trade and the
   mark-contribution Stage → Confirm flow.

### Excel on the web

3. Open a workbook on the web, *Insert → Add-ins → Upload My Add-in*, choose
   `manifest.xml`. The same functions and task pane work unchanged (Office.js is
   identical across hosts).

## One contract, no fork

`src/contract/` is a **minimal, semantics-identical** duplicate of the GUI's
`gui/src/data/{contract,enums,wsCodec}.ts` — two TS projections of the **same**
`celnet.wire` contract (the add-in is a separate project and cannot import across
the GUI package boundary). The codec mirrors the server's
`crates/celnet-server/src/ws/codec.rs` field-for-field. There is no
`schema_version`, no Excel-specific RPC, no fork. If `celnet.proto` evolves, both
projections update identically.
