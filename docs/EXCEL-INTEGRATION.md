# Celnet — Excel Integration Design

> Status: design (2026-05-30). Owner: WS-X (Excel edge), depends on WS-0 (frozen
> contract: `celnet-proto`), WS-H (`celnet-integration` normalize/audit discipline),
> WS-I (`celnet-server` gRPC + WebSocket mirror), and the typed `celnet-client` shapes.
>
> **Position in the estate.** Excel is an *edge consumer*, exactly like the React/WebGPU
> trader GUI and the typed SDK. It rides the **ONE current contract** in
> `crates/celnet-proto/proto/celnet.proto` (the five gRPC services: `PricingService`,
> `QuoteService`, `StreamService`, `SurfaceService`, `RiskService`) — no fork, no
> `schema_version`, no Excel-specific RPCs
> (guardrail #9). It never touches the pinned zero-alloc hot core (guardrail #11): all
> compute happens server-side; the workbook only renders results that already exist for the
> GUI and SDK. Determinism is inherited end-to-end — the server runs the libm core, so a
> price in `=CELNET.PRICE(...)` is **bit-identical** to the same trade in the GUI, the CLI,
> and a `celnet-client` call (guardrail: determinism).
>
> **Naming & licensing.** Every artifact is `celnet`-logical and vendor/research-neutral
> (guardrail #8): `CELNET.*` worksheet functions, `celnet-excel` (web add-in) and
> `celnet-xll` (Rust XLL) crates. **OSS-only** (guardrail #7): the cross-platform path uses
> Microsoft's open, royalty-free **Office.js** (`Office-Add-in` runtime, no SDK licence,
> no PyXLL, no commercial XLL toolkit); the optional native path is a pure-Rust C-ABI
> `cdylib` using only permissive crates. No commercial market-data add-in is a dependency
> anywhere.

---

## 1. Workflows we serve → concrete Excel surfaces

FX-options desks live in Excel for two reasons: **read** (pull live marks/Greeks/surface
into a sheet that already contains their own structuring math) and **write** (contribute a
mark, a manual vol, or a structured ticket back into the desk's books). We map each desk
workflow to the cleanest native Excel surface, so the add-in *feels* like Excel rather than
a bolted-on terminal.

| # | Desk workflow (read/write) | Excel surface | Contract RPC reused |
|---|---|---|---|
| W1 | Live price + two-way for one structure on a blotter row | **Streaming custom function** `=CELNET.SUBSCRIBE(...)` (one cell, auto-updates) | `StreamService.StreamSession` (snapshot+delta) |
| W2 | Full Greek vector for a position | **Dynamic-array spill** `=CELNET.GREEKS(...)` → labelled 13-Greek column/row | `PricingService.Price` (`Greeks`) |
| W3 | Marked smile / vol grid for a pair-tenor (or a surface cube) | **Dynamic-array spill** `=CELNET.SURFACE(...)` → strikes × pillars grid | `SurfaceService.GetSmile` |
| W4 | RFQ for a bespoke structure, two-way + token | `=CELNET.RFQ(...)` spill (bid/offer/token/valid-until) + **task-pane ticket** | `QuoteService.RequestQuote` |
| W5 | Watch many structures (a book) ticking live | `=CELNET.SUBSCRIBE(...)` streaming, one session multiplexed across all cells | `StreamService.StreamSession` (multiplex) |
| W6 | Click-to-trade off a live streamed price | **Task-pane** "Trade" button bound to the cell's live `TradableToken` | `StreamService` `Execute`/`Executed` |
| W7 | Risk roll-up / book / limits | Dynamic-array spill `=CELNET.RISK(...)` / `=CELNET.POSITIONS(...)` / `=CELNET.LIMITS(...)` | `RiskService.{AggregateRisk,ListPositions,LimitStatus}` |
| W8 | **Contribute a mark / manual vol** back to the marked surface, choosing the calibration model | `=CELNET.MARK(...)` write-function (optional `smileModel`) + task-pane "Contribute" confirm | `SurfaceService.MarkSurface` (optional `SmileModel`) |
| W9 | Convention/audit transparency for any cell | per-cell **convention/model footer** + task-pane provenance panel + `=CELNET.STATUS()` | echoed `Conventions` + `surface_version` (+ `model=` note) on every response |
| W10 | Live market-series spark/history for a pair (ATM-vol/spot/RR/BF/forward) | **Streaming** `=CELNET.SERIES(...)` (one cell or a spill of recent points) | `StreamService.StreamSession` (`MarketSeriesSubscribe`/`…Point`) |
| W11 | Who quoted / holds / won a streamed or RFQ line | attribution columns on the `CELNET.SUBSCRIBE`/`CELNET.RFQ` footer | `AttributionRecord` echoed on `Snapshot`/`Quote`/`Executed`/`Execution` |

The deliberate split: **read** is dominated by streaming/spill functions (zero clicks, the
sheet stays alive); **write** always goes through a task-pane confirmation step (W4/W6/W8)
because contribution and trading are entitlement-gated, audited, and (optionally) four-eyed
— never a silent side effect of a recalculation.

---

## 2. Architecture — two paths, one contract

```
                          ┌─────────────────────────────────────────────────────┐
                          │  celnet-server (WS-I)  — edge, NOT the hot core       │
                          │   gRPC (tonic)  +  WebSocket mirror (ws/codec.rs)     │
                          │   surface_book · keyed-MAC TradableToken · audit sink │
                          └───────────────▲──────────────────────▲───────────────┘
                  same proto, same        │ gRPC-web / WS         │ gRPC (HTTP/2)
                  typed shapes, no fork    │                       │
        ┌──────────────────────────────────┴──┐        ┌───────────┴──────────────────┐
        │  PATH A — celnet-excel (Office.js)   │        │  PATH B — celnet-xll (Rust)   │
        │  cross-platform: Win/Mac/Web/iPad    │        │  Windows native, ultra-low-lat│
        │  streaming custom funcs + task pane  │        │  C-ABI cdylib XLL, tokio+tonic│
        │  TS shapes generated from celnet.proto│        │  reuses celnet-client crate   │
        └──────────────────────────────────────┘        └───────────────────────────────┘
```

### Path A (primary): `celnet-excel` — Office.js streaming custom functions + task pane

- **What it is.** An Office Web Add-in (manifest + JS custom-function runtime + task-pane
  SPA), implemented at the top-level **`excel/`** directory (hand-rolled Vite + TypeScript
  strict; outside the cargo workspace). It is the cross-platform path: identical on Excel
  for Windows, Mac, the web, and iPad, because Office.js is the only Excel surface that runs
  everywhere. Its TS contract layer (`excel/src/contract/`) is a minimal, semantics-identical
  projection of `gui/src/data/{contract,enums,wsCodec}.ts` — one `celnet.wire` contract, no
  fork — and the WS transport (`excel/src/transport/`) speaks the same tagged-JSON the
  `crates/celnet-server/src/ws` mirror defines. See `excel/README.md` for the function
  surface, the headless e2e (`npm run verify:headless`, asserted against a live server), and
  the Excel sideload guide.
- **Transport.** The add-in talks to `celnet-server` over the **WebSocket mirror**
  (`crates/celnet-server/src/ws/`) for streaming (W1/W5/W6) and **gRPC-web** (unary, behind
  the same tonic server) for request/response (W2/W3/W4/W7/W8). Both carry the *same* proto
  messages — the WS codec frames the identical `ClientStreamMessage`/`ServerStreamMessage`,
  and gRPC-web carries the identical unary messages. There is no Excel-specific endpoint.
- **Typed shapes, one contract, no fork.** TS interfaces for `VanillaInputs`, `Greeks`,
  `Quote`, `Snapshot`, `Update`, `Smile`, `Conventions`, `TradableToken`, … live in
  `excel/src/contract/` as a **minimal, semantics-identical projection of
  `gui/src/data/{contract,enums,wsCodec}.ts`** (the add-in is a separate Vite project outside
  the GUI package and the cargo workspace, so it cannot import across that boundary; rather
  than fork the wire semantics it duplicates the *definitions* unchanged — see
  `excel/src/contract/contract.ts`). These mirror the `celnet-client` Rust shapes and the GUI
  one-for-one, so the workbook, the GUI, and the SDK share one vocabulary; the headless e2e
  round-trips fixtures through the WS codec against a live `celnet-server` so the projection
  cannot drift from the contract (guardrail #9/#10).
- **Why primary.** Zero desktop install footprint (centrally deployed manifest), the same
  code as the React GUI's data layer, and the broadest reach. It is the default for the
  whole desk.

### Path B (optional): `celnet-xll` — OSS Rust XLL for ultra-low-latency Windows desks

- **What it is.** A new workspace crate `celnet-xll` (with thin helper crate
  `celnet-excel-abi` if the C-ABI marshalling grows) compiled as a **C-ABI `cdylib`** →
  `celnet-xll.xll`, loaded by Excel's native XLL add-in mechanism on Windows. Pure Rust,
  permissive deps only (e.g. the MIT-licensed `xladd`/`xlcall`-style binding shim,
  `tonic`/`tokio` for transport) — **no PyXLL, no commercial XLL SDK** (guardrail #7).
- **Why.** Native XLL functions execute in-process with the lowest possible cell-update
  latency and the tightest RTD topic fan-out — the right tool for a high-frequency Windows
  trading desk that cannot tolerate the JS bridge's overhead.
- **Same contract, reused client.** `celnet-xll` depends directly on the **`celnet-client`**
  crate: it opens a `StreamSession` and issues `price`/`get_smile`/`request_quote` exactly
  like any other SDK consumer. It speaks the identical proto over gRPC. No second contract,
  no divergence; the XLL is just another `celnet-client` embedding behind Excel's RTD/async
  function API.
- **Both paths are first-class and interchangeable.** A cell returns the same value on
  either path because both resolve to the same server, the same surface_version, the same
  libm core.

### Where these sit

Both the **shipped Path-A add-in (top-level `excel/`, a standalone Vite/TypeScript project
outside the cargo workspace)** and the **designed-only Path-B `celnet-xll` crate** are **edge
consumers**, dependency arrows pointing *into* `celnet-client` / `celnet-proto` /
the WS-mirror contract only (one-way, per `docs/INTERFACES.md`). Neither is a dependency of the
engine, the core, or any pricing crate. The hot core stays log/lock/alloc-free; Excel adds
load only as another network client of the edge (guardrail #11).

---

## 3. Function surface (`CELNET.*`)

All functions are `celnet`-logical and vendor-neutral. Signatures are shown in the Office.js
metadata sense; the XLL exports the same names and argument order. Every result-bearing
function attaches **convention transparency** (see §3.4). The **27** shipped functions
(`excel/src/functions/functions.json`; see also `excel/README.md`) are grouped below into
request/response reads (§3.1), streaming reads (§3.2), the exotic/structured catalogue (§3.3),
risk/book reads (§3.4), and the write / observability surface (§3.5). All compute is
server-side; a cell renders, it never prices.

### 3.1 Read — request/response (dynamic-array spill)

```
=CELNET.PRICE(pair, tenor, strikeOrDelta, callPut, notional, [conv], [surfaceVersion])
    → scalar premium (two-way mid, in the convention's premium ccy/style)

=CELNET.GREEKS(pair, tenor, strikeOrDelta, callPut, notional, [conv], [surfaceVersion])
    → 13×2 vertical spill: [GreekName, Value] for the full Greek vector
      (delta, gamma, vega, theta, rho_dom, rho_for, vanna, volga, charm,
       speed, zomma, vomma/ultima, … exactly the celnet-vanilla 13-Greek set)
       + a convention footer

=CELNET.SURFACE(pair, tenor, [model], [surfaceVersion])
    → smile spill for the (pair, tenor): delta pillars × vol, with an arb-status /
      model / convention footer. Backed by SurfaceService.GetSmile.

=CELNET.MARKSURFACE(pair, tenor, model, atmVol, rr25, bf25, [rr10], [bf10])
    → calibrate & deposit a surface under a chosen smile model (VV/SABR/SVI/SSVI),
      spilling the calibrated smile + a surface_version / model footer
      (SurfaceService.MarkSurface with the SmileModel selector). This is the
      single-cell calibrate-and-pin function; CELNET.MARK (§3.5) is the two-phase
      manual-vol contribution.

=CELNET.RFQ(pair, tenor, strikeOrDelta, callPut, notional, [conv])
    → 1×4 spill [bid, offer, quoteId, validUntil] + convention footer
      (QuoteService.RequestQuote). The task-pane ticket resolves the quote for
      accept/click-to-trade (W4/W6).
```

`strikeOrDelta` accepts either an absolute strike or a delta-string (e.g. `"25dP"`,
`"ATM"`, `"DNS"`) and resolves through the `StrikeOrDelta` contract message — the same
strike↔delta solver the GUI uses. `callPut` is `"C"`/`"P"`. `conv` is an optional override
object/range; omitted ⇒ the canonical convention for the `(pair, tenor)` is resolved
server-side (the desk almost never overrides it). `model ∈ {VV, SABR, SVI, SSVI, eSSVI}`
(the five shipped smile families; `SurfaceService` echoes the model in the footer).

`tenor` uses the same shorthand the CLI/GUI/SDK speak (one `Tenor` contract message): `ON`,
`TN`, `SN`, `<n>W`/`<n>M`/`<n>Y`, `<n>IMM` (the `n`-th 3rd-Wednesday IMM date), or a
`YYYY-MM-DD` broken date. The pre-spot short end (ON/TN/SN) is anchored on **today**, not spot
(the ON-resolves-as-SN bug is fixed; see `docs/CONVENTIONS.md` / `ANALYTICS-SPEC.md` §1.5).

> **Scenario / what-if** is not a standalone `CELNET.*` function: the shock-grid path
> (`SurfaceService.Scenario`) is exercised by the GUI CubeWorkspace and the SDK
> `scenario*` helpers; the Excel surface composes the same view from `CELNET.PRICE` /
> `CELNET.GREEKS` re-evaluated over a strike/shock ladder and from `CELNET.RISK` (§3.4).

### 3.2 Read — streaming (`@streaming`, RTD-equivalent)

```
=CELNET.SUBSCRIBE(pair, tenor, strikeOrDelta, callPut, notional, [conv])
    → streaming live two-way for the structure; re-ticks on every Update;
      stale-aware; multiplexed and coalesced across identical-arg cells
      (StreamService.StreamSession snapshot+delta). This is the blotter RFS cell;
      the task-pane Trade button resolves its live TradableToken for click-to-trade (W6),
      and the AttributionRecord seats (quotedBy/heldBy) ride the snapshot (W11).

=CELNET.SERIES(pair, observable, [tenor], [delta], [throttleMs], [historyLimit])
    → streaming live market-observable trend for the GUI's TrendMode (W10), over one
      StreamSession: observable ∈ {ATM, SPOT, RR, BF, FWD}; tenor required for
      ATM/RR/BF/FWD, delta required for RR/BF. A scalar cell streams the latest value;
      a spill returns the recent [epoch, value] points (snapshot + appended points).
      Every value is derived server-side on the core thread from the live MarketState
      (never a fabricated proxy); an underivable point is skipped, a lagging cell drops a
      conflatable point — the contract matches the GUI/SDK exactly (API-first parity).
```

**Streaming behaviour.** Office.js streaming functions use `@streaming` /
`StreamingInvocation`; the add-in opens **one** `StreamService.StreamSession` per workbook
and *multiplexes* every live cell as a subscription on that single session (the contract is
built for exactly this — "watching hundreds of structures uses a single session, not one
stream per structure"). Identical-argument cells share one subscription (Excel coalesces
streams with equal arguments; the add-in reference-counts subscribers). On the XLL path the
same mapping uses Excel's RTD topic model: one topic per `(instrument, conv)` key, the XLL
acting as the RTD server fed by the `StreamSession`. Snapshot → first value; Update → cell
refresh; Resync → silent re-snapshot (no flicker); StreamEnd → see §5.

### 3.3 Read — the exotic / structured / multi-asset catalogue (dynamic-array spill)

Every product on the `Instrument` `oneof` (`docs/API-CLIENTS.md` §3) has a dedicated read
function — the same vocabulary as the SDK builders and the CLI `exotic` subcommands, so a cell
prices exactly the structure the GUI TicketWorkspace prices. Each returns the premium / PV, the
13-Greek vector where it exists in closed form, and a convention footer; the **Monte-Carlo
products carry a `std_error` line in the spill** (never "machine precision" — these are MC
estimates).

```
=CELNET.BARRIER(pair, tenor, strikeOrDelta, callPut, notional, barrier(s), kind, …)
    → single OR double barrier (selected by params): premium + 13 Greeks + footer
=CELNET.WINDOWBARRIER(pair, tenor, …, windowStart, windowEnd, …)
    → window (partial-time) KO barrier under PRICING_MODEL_LOCAL_STOCH_VOL (LSV PDE):
      premium + (std_error) + footer
=CELNET.DIGITAL(pair, tenor, strikeOrDelta, callPut, notional, style)
    → digital (binary) option: premium + 13 Greeks + footer
=CELNET.TOUCH(pair, tenor, level(s), kind)
    → one-/no-/double-no-/double-one-touch: premium + Greeks + footer
=CELNET.ASIAN(pair, tenor, strike, callPut, notional, fixings…)
    → arithmetic-average-rate Asian: spill [premium, PV] + 13 Greeks + footer
=CELNET.FORWARDSTART(pair, tenor, resetT, moneyness, callPut, notional)
    → forward-start vanilla: spill [premium, PV] + 13 Greeks + footer
=CELNET.CLIQUET(pair, tenor, resets…, [cap], [floor])
    → cliquet / ratchet: spill [premium, PV], (std_error if clamped/MC), 13 Greeks + footer
=CELNET.QUANTO(pair, tenor, strikeOrDelta, callPut, notional, quantoFx)
    → quanto vanilla / digital: spill [premium, PV] + 13 Greeks + footer
=CELNET.TARF(pair, tenor, fixings…, target, gearing, …)
    → target-redemption forward (Monte-Carlo): spill [premium, PV], [std_error], 13 Greeks + footer
=CELNET.ACCUMULATOR(pair, tenor, fixings…, pivot, barrier, …)
    → accumulator (Monte-Carlo): spill [premium, PV], [std_error], 13 Greeks + footer
=CELNET.LOOKBACK(pair, tenor, kind, callPut, notional, [fixings])
    → lookback (floating/fixed): spill [premium, PV], (std_error if discrete/MC), 13 Greeks + footer
=CELNET.AMERICAN(pair, tenor, strike, callPut, notional, [lsmPaths])
    → American / Bermudan early-exercise: premium + (std_error if LSM/MC) + 13 Greeks + footer
=CELNET.VARSWAP(pair, tenor, …)
    → variance swap: spill [fair_variance, K_var] / [fair_vol, √K_var] + convention footer
=CELNET.VOLSWAP(pair, tenor, …)
    → volatility swap: spill [fair_vol, K_vol] (convexity-adjusted) + convention footer
=CELNET.BASKET(legs…, weights, correlation, callPut, kind)
    → correlated multi-asset basket / best-of / worst-of: premium + MC std_error + footer
```

These map one-to-one onto the `Instrument` `oneof` arms and the parity rows in
`docs/CLIENT-PARITY-MATRIX.md`. (LSV is exposed only as a *priced product* — e.g. the window
barrier — never as raw calibration, exactly as the parity matrix records for the GUI/Excel.)

### 3.4 Read — risk, book & observability (dynamic-array spill)

```
=CELNET.RISK(dimension, numeraire, [rates], [scope])
    → server-side hierarchical risk aggregate over the org cube: one row per rolled-up
      node + a reporting-numeraire footer (RiskService.AggregateRisk / DrillRisk by scope)
=CELNET.POSITIONS([scope])
    → the entitled open-position leaf grid (org placement + attribution) + a count/empty
      footer (RiskService.ListPositions)
=CELNET.LIMITS(scope, numeraire, [rates])
    → limit-tree utilization + RAG grid + a worst-RAG / hard-breach footer
      (RiskService.LimitStatus)
=CELNET.STATUS()
    → live server observability spill: connection state, drain-side price latency
      (p50/p99/p99.9), ring conflation drops, and surface/correlation provenance
      (from the live heartbeat — the same telemetry the ops view reads)
```

`CELNET.RISK` / `CELNET.POSITIONS` / `CELNET.LIMITS` are the **server-side aggregation**
functions: a workbook never loops positions and sums — it asks `RiskService` for the rolled-up
node tree, scope- and principal-pruned, in the reporting numeraire (the same contract the GUI
Book/Risk views and the CLI `risk` subcommands consume). `scope` is an org `DIM:VALUE`; no
scope ⇒ the grant-all (show-all-now) default.

### 3.5 Write — contribution / trade

```
=CELNET.MARK(pair, tenor, pillar, vol, [model], [comment])
    → stages a manual vol/mark contribution; returns a status spill
      [status, surfaceVersionAfter, detail]  — NEVER auto-fires on recalc (see below)
      model ∈ {VV (default), SABR, SVI, SSVI, eSSVI}; selects the calibration family
      (ANALYTICS-SPEC §3.4a), tagged on the deposited surface version and echoed back
      (provenance from Smile.arbitrage.note `model=<family>`).
      (For the one-shot calibrate-and-pin path, use CELNET.MARKSURFACE in §3.1.)

(task-pane only) Trade        → Execute against a live RFS TradableToken (W6)
(task-pane only) Contribute   → confirm/stage a CELNET.MARK batch (W8)
```

**Critical write semantics.** A worksheet function that mutates server state on every
recalculation is a footgun. `CELNET.MARK` is therefore **two-phase and idempotent**: the
function *stages* a contribution keyed by a deterministic idempotency key
(`hash(pair,tenor,pillar,vol,conv,sessionEpoch)` — reusing the `celnet-client`
`idempotency` discipline) and renders a `PENDING` status; the actual
`SurfaceService.MarkSurface` write is committed only when the trader confirms in the
task-pane "Contribute" panel. A recalc with unchanged args re-stages under the
same key → the server dedupes → **no double-mark** (guardrail: determinism + idempotency).
Trading (`Execute`) is *only* ever a task-pane button, never a function — a cell never trades.

### 3.6 Convention-on-every-cell transparency

Every read function attaches the resolved `Conventions` (delta type, ATM convention,
premium-adjusted flag, day-count, spot/forward basis) and the `surface_version` that
produced the number. Surfaces:

- **Cell comment / spill footer.** Every result-bearing function (`CELNET.GREEKS`,
  `CELNET.SURFACE`, `CELNET.MARKSURFACE`, the whole exotic catalogue, …) carries a footer row
  with the resolved convention, the `surface_version`, the **smile model** the pinned surface
  was marked with (read from the `Smile.arbitrage.note` `model=<family>` provenance,
  ANALYTICS-SPEC §3.4a), and the timestamp — e.g.
  `[conv: 25Δ premium-adj, ATM=DNS, ACT/365F | surface v#1284 | model=SSVI | t=12:04:07.114Z]`.
  The streaming RFS/series footers additionally carry the `AttributionRecord` seats
  (`quotedBy`/`heldBy`/`won`/`lp_count`, W11) — the same who's-trading chain the GUI shows.
- **Task-pane provenance panel.** Selecting a Celnet cell shows the full
  convention + surface_version + smile model + upstream source lineage (W9).
- **`CELNET.STATUS`.** The live observability spill (§3.4) surfaces connection/latency/drop
  and surface/correlation provenance for the workbook's session as a whole.

This is a direct out-intuit of incumbents, which print a number with **no** convention
context — the desk's #1 source of FX-options mismarks is silent delta/ATM/premium-adjust
convention mismatch.

---

## 4. Contribution controls (write-path discipline)

The write-path reuses the server-side discipline already built for `celnet-integration` and
`celnet-server`, so Excel inherits the same guarantees as every other ingest channel.

1. **Entitlement.** Each session authenticates (the add-in carries the desk SSO token; the
   server maps it to a contributor identity). `CELNET.MARK`/`Execute` are rejected with a
   typed `permission_denied` unless the identity is entitled for that `(pair, book)`. Read
   functions degrade to entitled scope; an un-entitled cell returns a typed error value, not
   a wrong number.
2. **Convention-checked validation on ingest.** A contributed mark is normalized through the
   **same convention cross-check** as a vendor feed
   (`celnet-integration/src/normalize.rs`): the contribution's declared delta/ATM/premium
   convention is checked against the canonical convention Celnet resolves for the
   `(pair, tenor)`; a material mismatch is **rejected** (`NormalizeError`) rather than
   silently corrupting the surface. The trader sees the declared-vs-resolved diagnostic in
   the task pane. No mark enters `surface_book` un-normalized.
3. **Idempotency.** Deterministic idempotency keys (§3.3) flow through to the server's
   existing dedupe so a retried/recalculated contribution commits at most once.
4. **Audit (lossless).** Every contribution and every click-to-trade is recorded through the
   server's **lossless audit sink** (`celnet-observability/src/audit.rs`): who, what
   (pair/tenor/pillar/vol or token), declared vs resolved convention, idempotency key,
   resulting `surface_version`, timestamp. `CELNET.MARK` returns the `auditId` so the row is
   traceable from the workbook.
5. **Optional four-eyes.** For configured pairs/desks, a contribution stages as
   `PENDING_APPROVAL`; a second entitled identity approves it in the task pane (or via the
   GUI) before it is published to a new `surface_version`. The maker cannot self-approve.
   This is a server-side policy on the one contract — Excel simply renders the pending state.

---

## 5. Reliability guarantees (determinism, no-stale-stream, no #N/A-storm, non-blocking grid)

- **Determinism / bit-identity.** All math is server-side on the libm core; Excel never
  recomputes a price. `=CELNET.PRICE(...)` equals the GUI, CLI, and `celnet-client` value to
  the last bit for the same inputs + `surface_version`. Pinning `surfaceVersion` makes a
  workbook reproducible across a re-open.
- **No stale stream.** Streamed cells carry an implicit liveness contract:
  - The session sends/receives **Heartbeats**; on a missed heartbeat the add-in marks
    affected cells **stale** (a visible `…` / dimmed state) rather than showing a frozen
    last-good number as if live.
  - **Resync** triggers a silent re-snapshot — the cell jumps to truth, never to a guess.
  - **StreamEnd** (instrument retired, entitlement lost, server cutover) resolves the cell to
    a typed terminal value, and any pending click-to-trade waiter is failed with
    `Reconnected`/`StreamClosed` (the exact `celnet-client` RFS reconnect-liveness fix — no
    cell waits forever across a blue-green cutover).
  - On reconnect the add-in **re-subscribes** every live cell on a fresh session and
    re-pins the `surface_version`; an unknown version returns `failed_precondition`, never a
    silent fallback to live (mirrors the server `surface_book` pin resolver).
- **No #N/A-storm.** Transient transport errors do **not** spray `#N/A` across a blotter:
  - A cell holds its last-good value in a **stale** visual state during a brief outage; it
    only surfaces a typed error value (`#CELNET_STALE!`, `#CELNET_DENIED!`,
    `#CELNET_VERSION!`) when the condition is durable, with the reason readable via
    `=CELNET.STATUS(cellRef)`.
  - Backoff + jitter on reconnect; a single session reconnect re-feeds all cells at once, so
    a thousand-row blotter recovers in one event, not a thousand independent failures.
- **Non-blocking grid.** Every function is **async/streaming** — no synchronous network call
  ever runs on Excel's calc thread. On Path A this is inherent to Office.js custom functions
  (promise/streaming); on Path B the XLL uses Excel's async UDF / RTD model so the calc
  thread never blocks on I/O. The grid stays responsive under a full book of live cells.
- **Cleanup.** Streaming `onCanceled` tears down the subscription (decrement ref-count;
  unsubscribe when zero) so a deleted cell stops its server stream — no orphaned subscriptions
  (Office.js best practice).

---

## 6. Competitor comparison

| Capability | Bloomberg (BLP / BQL in Excel) | Fenics (FX-options add-in) | Refinitiv (Eikon/Workspace Excel) | **Celnet Excel** |
|---|---|---|---|---|
| Live streaming into cells | RTD/`BDP` w/ realtime; COM, Windows-centric | Add-in streaming, desktop-bound | RTD via desktop add-in | **Office.js streaming + optional native XLL; Win/Mac/Web/iPad** |
| Cross-platform (Mac/Web/iPad) | Effectively Windows/desktop | Windows desktop | Windows desktop | **Yes — one Office.js add-in everywhere** |
| Pricing identical to the firm's pricer | Bloomberg's models, not yours | Fenics models | Refinitiv models | **Same libm core as GUI/SDK — bit-identical to the desk's own marks** |
| Convention transparency per cell | None inline | Limited | None inline | **Convention + surface_version on every cell + `CELNET.CONV`/`PROVENANCE`** |
| Contribute marks back from Excel | No (read-mostly) | Limited, opaque | No | **`CELNET.MARK` write-path, convention-validated, audited, four-eyes** |
| Click-to-trade off a live cell | Limited / separate ticket | Via Fenics UI, not cells | No | **Live RFS token → task-pane Trade, idempotent (`Execute`/`Executed`)** |
| Data-pull quotas / metering | Daily/monthly download caps | Licence-tiered | Licence-tiered | **No vendor quota — your own server, your own data** |
| Determinism / reproducibility | Vendor black box | Vendor black box | Vendor black box | **Pin `surface_version` ⇒ reproducible to the bit** |
| Stale-stream honesty | Can show last value as live | Varies | Varies | **Explicit stale state + heartbeat + resync; never frozen-as-live** |
| Licensing | Commercial terminal | Commercial | Commercial | **OSS-only add-in (Office.js / pure-Rust XLL)** |

**Concrete wins.** (1) *Same number everywhere* — the cell, the GUI, the SDK, and the
trade-lifecycle book agree to the bit, because they share one server and one core; no
"why does Excel disagree with the pricer" reconciliation. (2) *Convention transparency*
kills the desk's top mismark source. (3) *True write-path* — Excel becomes a first-class
contribution and click-to-trade surface, audited and four-eyed, not a read-only data tap.
(4) *Cross-platform* from a single add-in. (5) *No vendor quota / no commercial licence*.

---

## 7. Phased build plan & validation gates

> **Status (records the original plan; reconciled to what shipped).** Path A
> (`excel/`, Office.js) is **shipped**: phases X0–X3 and X5 are realized in the top-level
> `excel/` project — the **27** `CELNET.*` functions of §3, the task-pane ticket/contribution
> flow, the headless e2e (`cd excel && npm run verify:headless`, asserted against a live
> `celnet-server`), and the vitest suites under `excel/test/`. Two refinements vs the original
> plan below: the scaffold lives at top-level **`excel/`** (a standalone Vite project outside
> the cargo workspace), and the TS contract is a **hand-maintained projection** of the GUI's
> `gui/src/data` (`excel/src/contract/`) rather than `ts-proto`-generated shapes — the
> headless e2e against the live WS codec is what guarantees it cannot drift (§2). Phase X4
> (native `celnet-xll`) remains **designed-only** — no `celnet-xll` crate has been built.

Each phase distinguishes the **in-repo build/test gate** (what is buildable and testable
headless in CI today) from the **deployment gate** (what requires an installed Excel host,
validated manually / in a containerized Office runner — explicitly *out* of the per-commit
`just check`).

### Phase X0 — Contract seam & generated shapes
- **Build.** Add `addins/celnet-excel/` scaffold; wire `celnet.proto` → TS via `ts-proto`
  (MIT) into `src/gen/`; add `celnet-xll` crate skeleton depending on `celnet-client`.
- **In-repo gate.** TS codegen is a CI step; a Rust test asserts the `celnet-xll`↔`celnet-client`
  shapes compile against the current proto. `cargo-deny` confirms every new dep is permissive.
  A JS unit test round-trips a `Snapshot`/`Update`/`Greeks` fixture through the generated
  decoders against a `celnet-proto`-encoded golden — proving Excel sees the same bytes as the SDK.
- **Deployment gate.** None.

### Phase X1 — Read functions over the WS/contract seam
- **Build.** Implement `CELNET.PRICE`, `CELNET.GREEKS`, `CELNET.SURFACE` (request/response,
  dynamic-array spill) on gRPC-web; a thin transport adapter over `celnet-server`'s WS mirror.
- **In-repo gate.** **Headless add-in test** — run the custom-function module under the
  `custom-functions-runtime` in Node (jsdom/`office-addin-mock`, MIT) against an in-process
  `celnet-server`; assert `CELNET.PRICE` returns the bit-identical value to a direct
  `celnet-client.price(...)` call (the determinism gate), and `CELNET.GREEKS` spills the
  13-Greek vector in the contract order with the convention footer. No Excel needed.
- **Deployment gate.** Manual sideload in Excel (Win + Mac + Web) confirms spill geometry
  and the convention footer render.

### Phase X2 — Streaming functions & multiplex session
- **Build.** `CELNET.PRICE.LIVE`, `CELNET.RFS`/`CELNET.SUBSCRIBE` over one multiplexed
  `StreamSession`; subscription ref-counting; heartbeat/resync/staleness state machine;
  `onCanceled` teardown.
- **In-repo gate.** Headless test drives a scripted server (snapshot→updates→resync→
  heartbeat-gap→reconnect) and asserts: identical-arg cells share one subscription; a
  heartbeat gap flips cells to *stale* (not frozen-as-live); resync re-snapshots without an
  `#N/A` storm; StreamEnd resolves pending waiters with a typed terminal value. Timeout-bounded.
- **Deployment gate.** Sideload: a 200-row live blotter stays responsive (non-blocking grid)
  and recovers in one event on a forced server cutover.

### Phase X3 — Task-pane ticket, click-to-trade, contribution
- **Build.** Task-pane SPA (reusing the GUI's data layer): RFQ ticket (`CELNET.RFQ` →
  `RequestQuote`), Trade button (`Execute` against a live token), Contribute panel
  (`CELNET.MARK` staging → `MarkSurface` commit) with entitlement, convention validation,
  idempotency, audit, optional four-eyes.
- **In-repo gate.** Headless: a contribution with a mismatched declared convention is
  rejected via the `celnet-integration` normalize cross-check; an idempotent re-stage commits
  once; the audit sink records who/what/convention/surface_version/auditId; an un-entitled
  identity is `permission_denied`; four-eyes requires a distinct approver. All asserted
  against the in-process server, no Excel host.
- **Deployment gate.** Sideload: end-to-end mark + four-eyes approval visible in the GUI and
  the trade-lifecycle book; click-to-trade executes against a live RFS token.

### Phase X4 — Native XLL path (optional, Windows)
- **Build.** `celnet-xll` C-ABI `cdylib` exporting the same `CELNET.*` surface over RTD/async
  UDFs, embedding `celnet-client`.
- **In-repo gate.** Rust tests on `celnet-xll`'s pure logic (ABI marshalling, RTD topic
  keying, the `celnet-client` session wiring) against an in-process server; assert the XLL
  path returns values bit-identical to Path A and the SDK. `cargo-deny` OSS check. Built and
  tested on Linux/Mac in CI as a library; the `.xll` host load is Windows-only.
- **Deployment gate.** Windows-host load + latency validation vs the Path-A baseline; confirm
  RTD fan-out under a high-frequency book.

### Phase X5 — Hardening & parity
- **Build.** Wire the Excel paths into the executable parity matrix (`celnet-parity`) as
  capability rows (streaming, convention transparency, write-path, click-to-trade) gated vs
  the incumbent baseline. Reconnect/backoff fuzz; #N/A-storm regression suite.
- **In-repo gate.** Parity rows green in CI; fuzz/regression in the nightly suite;
  `just check` for `celnet-xll` and the TS lint/test for the add-in.
- **Deployment gate.** Cross-host smoke matrix (Win/Mac/Web/iPad) before a desk rollout; the
  installed-Excel latency headline is a deployment-grade claim, not a per-commit gate.

---

### Guardrail compliance summary
- **OSS-only (#7):** Office.js (royalty-free), `ts-proto`/`protobuf-es`, pure-Rust XLL shim,
  `tokio`/`tonic` — all permissive; **no PyXLL, no commercial add-in SDK, no vendor data dep**;
  every new dep passes `cargo-deny`.
- **Vendor-neutral naming (#8):** `CELNET.*` functions, `celnet-excel` / `celnet-xll` crates;
  no person/vendor/method names in any identifier.
- **Single current contract (#9):** rides `celnet.proto`'s existing services; TS/Rust shapes
  generated from it; no `schema_version`, no Excel fork, no negotiation.
- **Determinism:** server-side libm core; Excel renders, never computes; bit-identical to
  GUI/SDK; `surface_version` pin = reproducibility.
- **Edge-only / zero hot-core cost (#11):** Excel is a network client of the edge server like
  the GUI; the pinned zero-alloc hot core is untouched.
