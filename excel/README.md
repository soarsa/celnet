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

There are **13** custom functions (the manifest is `src/functions/functions.json`,
the registered set is `src/functions/functions.ts`). Every one shapes a request in
the single `celnet.wire` contract and renders the server's typed result — the
add-in adds no pricing. Product pricing is **one composable polymorphic surface**:
`CELNET.INSTRUMENT` builds an opaque instrument token for ANY product family on
ANY asset class, and the four verbs (`PRICE`/`GREEKS`/`RFQ`/`SUBSCRIBE`) price it.
The former per-product function table (`CELNET.BARRIER`, `CELNET.TARF`, …) is
retired at proven wire parity (`test/instrumentPolymorphic.test.ts` asserts the
spec path emits the byte-identical frame for every retired function and every
golden-corpus family).

### The polymorphic pricing surface

| Function | Shape | Contract path |
|---|---|---|
| `=CELNET.INSTRUMENT(underlier, product, terms, [tenor], [notional])` | the opaque instrument token (a deterministic value, not an API) | — (pure shaping) |
| `=CELNET.PRICE(pairOrInstrument, [tenor], [strikeOrDelta], [callPut], [notional])` | positional vanilla: scalar mid premium (unchanged); token: the family's labelled spill (`premium`, honest `std_error` only when MC-priced, 13 Greeks, convention footer; swaps spill their fair strikes) | `request_quote` |
| `=CELNET.GREEKS(pairOrInstrument, …)` | 13×2 spill `[name, value]` + convention footer — any family/class via a token | `request_quote` (Greeks) |
| `=CELNET.RFQ(pairOrInstrument, …)` | 1×4 spill `[bid, offer, quoteId, validUntil]` + footer — any family/class via a token | `request_quote` |
| `=CELNET.SUBSCRIBE(pairOrInstrument, …)` | **streaming** live two-way; re-ticks; stale-aware — any family/class via a token | `subscribe`/`update` (multiplexed) |

**The underlier grammar** (one string, five asset classes): FX `EURUSD`/`EUR/USD`;
metal `XAUUSD`/`XAU/EUR` (a metal-X-code base leg vs a FIAT quote —
metal-vs-metal ratios are not priceable and rejected with a typed error); equity
`AAPL@XNAS:USD` (`ticker@venue:ccy` — venue present); commodity `BRENT@:USD`
(empty venue); crypto `BTC/USD`/`ETH-USDT`/`DOGEUSDT` with an optional
`:inverse`/`:linear` settlement suffix (`:inverse` = the coin-margined `1/S_T`
convention). A 3-letter/3-letter pair is FX unless its base is a metal X-code or
a known liquid coin; any other coin uses a non-3-letter leg (`FOO/USDT`) or an
explicit suffix (`FOO/USD:linear`).

**The terms range** is a named, order-free 2-column key/value range — e.g.
`("strike",1.12; "callPut","C"; "barrier",1.20; "kind","KNOCK_OUT")` — whose keys
mirror the family's parameters exactly (a missing/unknown key is a typed error
naming the key). `tenor`/`notional` may be terms instead of arguments (notional
defaults to 1). Per family (optional keys bracketed):

| `product` | terms keys |
|---|---|
| `VANILLA` | `strike` (level or delta `25dP`/`ATM`), `callPut` |
| `BARRIER` | `strike`, `callPut`, `barrier`, `[kind]`, `[side]`, `[upperBarrier]` (⇒ double), `[rebate]`, `[monitoring]`, `[model]` (`ANALYTIC`/`LSV`) |
| `WINDOWBARRIER` | `strike`, `callPut`, `barrier`, `[side]`, `[windowStart]`, `[windowEnd]`, `[mcPairs]`, `[mcSteps]`, `[mcSeed]` |
| `DIGITAL` | `strike`, `callPut`, `[style]`, `[payout]` |
| `TOUCH` | `kind` (`OT`/`NT`/`DNT`/`DOT`), `barrier`, `[rebate]`, `[upperBarrier]`, `[monitoring]` |
| `VARSWAP` / `VOLSWAP` | `[strikeVol]` |
| `ASIAN` | `strike`, `callPut`, `[averaging]`, `[observations]`, `[method]`, `[elapsedAvg]`, `[elapsedWeight]` |
| `FORWARDSTART` | `callPut`, `moneyness`, `reset` |
| `CLIQUET` | `callPut`, `moneyness`, `periods`, `[localFloor]`, `[localCap]`, `[globalFloor]`, `[globalCap]`, `[mcPairs]`, `[mcSeed]` |
| `QUANTO` | `callPut`, `strike`, `conversionVol`, `correlation`, `[payoff]` |
| `TARF` | `callPut`, `strike`, `target`, `leverage`, `fixings`, `[redemption]`, `[fixingNotional]`, `[mcPairs]`, `[mcSeed]` |
| `ACCUMULATOR` | `pivot`, `barrier`, `leverage`, `fixings`, `[monitoring]`, `[fixingNotional]`, `[mcPairs]`, `[mcSeed]` |
| `LOOKBACK` | `callPut`, `[style]`, `[monitoring]`, `[strike]` (FIXED only), `[observations]`, `[mcPairs]`, `[mcSeed]` |
| `AMERICAN` | `strike`, `callPut`, `[style]`, `[bermudanSteps]`, `[lsmPaths]`, `[lsmExerciseDates]`, `[lsmSeed]` |
| `BASKET` | `callPut`, `strike`, `[kind]`, `[mcPaths]`, `[mcReplications]`, `[mcSteps]`, `[mcSeed]`, plus repeated rows `("legs", pair, weight, spot, vol, rFor)` and `("correlations", ρ…)` |
| `FORWARD` | `rate`, `[side]` |
| `SWAP` | `rate`, `[nearSide]` |
| `NDF` | `rate`, `fixing` (e.g. `BRL.PTAX`), `[settlementCcy]`, `[side]` |

The proto product-arm names (`single_barrier`, `variance_swap`, `fx_forward`, …)
are accepted as `product` aliases, so corpus/family tokens work verbatim.

### Surface (marking & smile)

| Function | Shape | Contract path |
|---|---|---|
| `=CELNET.SURFACE(pair, tenor, [model])` | smile spill (delta pillars × vol) + arb/model/convention footer | `get_smile` |
| `=CELNET.MARKSURFACE(pair, tenor, model, atmVol, rr25, bf25, [rr10], [bf10])` | calibrate a surface under a model (VV/SABR/SVI/SSVI); spill the calibrated smile + `surface_version`/model footer | `mark_surface` (`smile_model`) |
| `=CELNET.MARK(pair, tenor, pillar, vol, [model], [comment])` | status spill `[status, version, detail]` — **two-phase, idempotent**; commits via the task pane | `mark_surface` (`smile_model`, on confirm) |

### Streaming

| Function | Shape | Contract path |
|---|---|---|
| `=CELNET.SUBSCRIBE(pairOrInstrument, …)` | **streaming** live two-way (vanilla positional or any instrument token); re-ticks; stale-aware | `subscribe`/`update` (multiplexed) |
| `=CELNET.SERIES(pair, observable, [tenor], [delta])` | **streaming** live market-observable trend (ATM/SPOT/RR/BF/FWD); re-ticks | `market_series_subscribe`/`…_point` (multiplexed) |

### Hierarchical risk & operations (`RiskService` + observability)

| Function | Shape | Contract path |
|---|---|---|
| `=CELNET.RISK(dimension, numeraire, [rates], [scope])` | hierarchical risk node grid (one row per rolled-up node) + reporting-numeraire footer — **server-side aggregation** | `aggregate_risk` |
| `=CELNET.POSITIONS([scope])` | the entitled open-position leaf grid (org placement + attribution) + count/empty footer | `list_positions` |
| `=CELNET.LIMITS(scope, numeraire, [rates])` | limit-tree utilization/RAG grid + worst-RAG / hard-breach footer | `limit_status` |
| `=CELNET.STATUS()` | live server observability spill: connection state, drain-side price latency p50/p99/p99.9, ring conflation drops, surface/correlation provenance | `heartbeat` (latest beat) |

`strikeOrDelta` accepts an absolute strike (`1.12`), a delta string (`25dP`,
`10dC`), or `ATM`/`DNS`. `callPut` is `C`/`P`. `model` accepts `VV` (market
hedge / Vanna-Volga, the default), `SABR`, `SVI` or `SSVI` (the contract
`smile_model` selector). `observable` accepts `ATM`, `SPOT`, `RR`, `BF`, `FWD`.
The optional Monte-Carlo controls on the MC-priced exotics (`mcPairs`/`mcPaths`,
`mcReplications`, `mcSteps`, `mcSeed`) are **bit-reproducible**: a fixed seed
reproduces the same path set and price (with its `std_error`) exactly; `0` selects
the server default.
`CELNET.MARK` never writes on a recalc: it **stages** under a deterministic
idempotency key and the trader confirms in the task pane (or the server four-eyes
it) — so a thousand recalcs produce at most one mark. `CELNET.MARKSURFACE` is the
direct (non-staged) "re-mark under model X" action cell — it issues `mark_surface`
on evaluation; the model the surface was calibrated under is echoed in the footer
from the server's TYPED `ArbReport.smile_model` provenance field (the authoritative
contract field, appended — no `schema_version`), never the legacy `model=` note
token. The model a surface was last marked under is read back by
`CELNET.SURFACE(pair, tenor, model)` so a model mismatch is visible, never assumed.

`CELNET.RISK` / `CELNET.POSITIONS` / `CELNET.LIMITS` are the **server-side
hierarchical-risk** surface (the `RiskService` contract). Aggregation is **owned by
the server**: the workbook never loops positions and sums — it asks `aggregate_risk`
for the rolled-up node tree over a `dimension` (`FIRM`/`TRADER`/`BOOK`/`DESK`/`PAIR`/
`LOCATION`/`ENTITY`), reads its constituents with `list_positions`, and reads
`limit_status` for RAG/utilization. Every measure is in the **reporting numeraire**
the caller names (e.g. `USD`) — aggregation runs through `celnet-risk-normalize`
server-side, so a cell is never in native premium units. Supply the per-currency
conversion rates as an optional `[ccy, rate]` range (numeraire units per 1 unit of
ccy at spot; the numeraire's own rate is implicit 1.0); a rate the book needs but you
omit fails the request loudly server-side, never a silent leg drop. The default
entitlement principal is **grant-all** (show-all-now; entitlement-ready). The
non-additive measures (VaR/ES/curvature) are presence-tracked — a blank cell when not
evaluated this cycle, never a spurious zero.

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
