# Fixed-Income Excel Integration — Review & Requirements

**Status:** analysis only (no code change). 2026-06-29.
**Question answered:** what is required to ship a **Fixed-Income** counterpart to the
existing FX-options Excel add-in — surfaced as a **tab under the Fixed Income
domain** — mirroring the FX custom-function surface, the task-pane affordances, the
sign-in/capability gating, and the GUI rail launcher.

**Siblings:**
[`docs/EXCEL-INTEGRATION.md`](../../clients/EXCEL-INTEGRATION.md) (the FX add-in design),
[`docs/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md`](CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md),
[`docs/FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`](FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md).

> **Naming guardrail (GUIDE.md §8).** Every proposed function/identifier is
> purpose-named and vendor-neutral, under the `CELNET.*` worksheet namespace.

---

## 0. Executive summary

### Proposed FI custom-function set (mirroring the FX surface)

| Function | Purpose | Wire RPC | Capability | Status |
|---|---|---|---|---|
| `CELNET.RATES` | Price an OIS / swap / FRA, spill PV + first-order risk | `price_rates` | `price · fixed_income` | **Exists** (OIS only) |
| `CELNET.DV01` | First-order rate risk (pv01/dv01) for a swap | `price_rates` (projection) | `price · fixed_income` | Buildable now (no new RPC) |
| `CELNET.KEYRATE` | Key-rate DV01 ladder for a swap | `price_rates` (projection) | `price · fixed_income` | Buildable now (no new RPC) |
| `CELNET.RATESRISK` | Server-rolled rates risk cube (DV01 by node) | `aggregate_rates_risk` | `view · fixed_income` | RPC on WS; **needs Excel client method** |
| `CELNET.RATESPOSITIONS` | List entitled rates positions (the Book) | `list_rates_positions` | `view · fixed_income` | RPC on WS; **needs Excel client method** |
| `CELNET.RATESBOOK` | Book a rates position | `book_rates_position` | `book · fixed_income` | RPC on WS; **needs Excel client method** |
| `CELNET.DEALS` | List FI deal blotter (RFQ/IOI fills) | `list_deals` | `view · fixed_income` | RPC on WS; **needs Excel client method** |
| `CELNET.RATESRFQ` | FI dealer-quoting (submit/respond/accept) | `submit/respond/accept_desk_quote` | `rfq_respond`/`ioi_respond`/`execute · fixed_income` | RPC on WS; **better as task-pane affordance** |
| `CELNET.CURVE` | Pull discount/forward/zero curve by pillar | **none** | `view · fixed_income` | **GAP — needs new wire RPC** |
| `CELNET.RATESSUBSCRIBE` | Stream live rates pricing | **none** | `stream · fixed_income` | **GAP — no rates streaming on the wire** |

### Top gaps / RPCs that must be built first

1. **No curve-read RPC.** `PricingService` exposes only `Price` and `PriceRates`
   (`celnet.proto:2992`); there is **no `GetCurve`**. The discount/forward curve is
   bootstrapped *inside* `price_rates` from the caller's par pillars and never
   returned as discount factors / zero rates / forwards. `CELNET.CURVE` therefore
   needs a **new wire RPC** (or a curve block added to `RatesPriceResponse`).
2. **No rates streaming.** The `StreamService` RFS session subscribes FX/cross-asset
   `Instrument`s only (`connection.ts` `subscribe` → `instrumentToWire`); there is
   **no rates subscription frame**. `CELNET.RATESSUBSCRIBE` needs a new streaming
   path end-to-end.
3. **Wire instrument coverage is OIS-only.** `RatesInstrument` is a one-arm `oneof`
   (`ois` only — `celnet.proto:3951`). FRA pricing exists in the `celnet-rates`
   engine (commit `89b0588`) and cash-bond analytics exist in `bond.rs`, but
   **neither is on the wire**, so `CELNET.RATES` cannot price a FRA or a bond yet
   (see the curves/bond gap-analysis docs). Cash-bond pricing is the largest gap.
4. **Excel transport lacks the FI client methods.** The mirror already routes nine
   FI frame types (`ws/mod.rs`), but `excel/src/transport/connection.ts` implements
   only `priceRates` — the rates-risk / book / positions / deals / desk-RFQ /
   notification methods are **not yet on the Excel `Connection`** (a thin add; no
   server work).

**Net:** the read/price/book/deal-blotter FI functions are **all reachable today
over the existing WS mirror** and need only Excel-side client methods + functions +
codecs. `CELNET.CURVE` and `CELNET.RATESSUBSCRIBE` are blocked on **new wire RPCs**.
Cash-bond and FRA pricing are blocked on **wire instrument arms** (tracked in the
sibling docs).

---

## A. Current state

### A.1 The existing Excel function surface (`excel/src/functions/functions.ts`)

The add-in exposes **13 registered custom functions** (`registerAll`,
`functions.ts:940`), all under the `CELNET` namespace (declared in the manifest,
`Celnet.Functions.Namespace`). The surface is polymorphic: one `INSTRUMENT` token
spec drives the pricing verbs across all asset classes.

| Function | Kind | FX / FI | Wire call (via `getConnection()`) | Cell gate (`denyIfUngated`) |
|---|---|---|---|---|
| `INSTRUMENT` | builder (value) | shared grammar (FX/metal/equity/commodity/crypto) | none (pure encode) | none |
| `PRICE` | price | FX + cross-asset | `requestQuote` | `price` |
| `GREEKS` | price | FX + cross-asset | `requestQuote` | `greeks` |
| `RFQ` | quote | FX + cross-asset | `requestQuote` / `requestMultiDealerQuote` | `rfq_cell` |
| `SUBSCRIBE` | stream | FX + cross-asset | RFS `subscribe` (stream registry) | `subscribe` |
| `SURFACE` | surface read | FX | `getSmile` | none |
| `MARKSURFACE` | surface calibrate | FX | `markSurface` | `marksurface` |
| `SERIES` | stream | FX | RFS `subscribeSeries` | `series` |
| `MARK` | surface contribute | FX | `stageMark` (two-phase) | `mark` |
| `RISK` | risk cube | FX/cross-asset | `aggregateRisk` | none (server enforces) |
| `POSITIONS` | book read | FX/cross-asset | `listPositions` | none |
| `LIMITS` | limits | FX/cross-asset | `limitStatus` | none |
| `STATUS` | server health | n/a | `latestHeartbeat` (passive) | none |
| **`CELNET.RATES`** | **FI price** | **fixed income** | **`priceRates`** | **`rates` → `price · fixed_income`** |

**The lone FI function — `CELNET.RATES`** (`functions.ts:401-419`):
- **Signature:** `RATES(curve, referenceDate, tenor, fixedRate, direction, notional, [currency])`.
- **Inputs:** a 2-column `[tenorYears, parRate]` par-OIS pillar range; a reference
  (spot-anchor) date; the OIS tenor in whole years; the fixed-leg rate (decimal);
  `"PAY_FIXED"`/`"RECEIVE_FIXED"`; notional; optional ISO-4217 currency (default USD).
- **Shaping:** `shapeRatesCurve` + `shapeOisInstrument` (`functions/shaping.ts`).
- **Wire:** `Connection.priceRates(curveSet, instrument)` → frame `price_rates`,
  reply `rates_price_response` (`connection.ts:826-839`), decoded by
  `ratesPricingResultFromWire` (`contract/wsCodec.ts`).
- **Output:** a labelled `(4 + pillars)×2` spill — `pv`, `par_rate`, `pv01`, `dv01`,
  then a `kr_dv01[<tenor>Y]` key-rate DV01 ladder (one row per pillar; the ladder
  sums to `dv01`). All measures in the curve currency, carrying the `direction` sign.
- **Gate:** `denyIfUngated("rates")` → capability `price · fixed_income`
  (`contract/access.ts:279-287`).

The add-in carries **no rates math**: the calibrated pillars + OIS terms go to the
live `celnet-rates` engine, which bootstraps the discount/forward curve and returns
the authoritative PV + risk — a cell is bit-identical to the GUI/SDK/CLI.

### A.2 Add-in architecture

- **Shared runtime.** `manifest.xml` declares `SharedRuntime` (one long-lived
  runtime hosting BOTH the task pane and the custom functions —
  `Celnet.Taskpane.Url` is the `<Runtime>`, `<Script>`, and `<Page>`). A sign-in in
  the pane therefore gates the cells too.
- **Transport.** `excel/src/transport/connection.ts` — one WS connection IS one
  multiplexed RFS session AND the request/response channel; ported from the GUI's
  `wsTransport.ts` over the one contract. Carries auto-reconnect, gap-detect/resync,
  a per-subscription staleness monitor, and the `Authenticate`-first frame
  (`authenticateFrame`) with a `session_token` + grant-all-default `principal`.
- **Contract codec.** `excel/src/contract/` — `contract.ts` (typed model incl.
  `RatesCurveSet` / `OisInstrument` / `RatesPricingResult`), `wsCodec.ts`
  (snake_case JSON ⇄ typed), `riskCodec.ts`, `authCodec.ts`, `instrumentCodec.ts`,
  `enums.ts`, and **`access.ts`** (the user-permission layer).
- **Sign-in + capability gating** (`contract/access.ts` + `transport/session.ts`):
  - One capability = one `CapabilityAction × CapabilityAsset`. Assets are
    `fx_options` | `fixed_income` (`access.ts:35`). Actions: `view`, `price`,
    `quote_respond`, `rfq_respond`, `ioi_respond`, `stream`, `execute`, `book`,
    `administer` (`access.ts:23-32`).
  - `can(caps, action, asset)` (`access.ts:103`) tests membership against the
    server-resolved effective set delivered on `AuthService.Login`
    (`LoginResult.capabilities`).
  - `ENTRY_POINTS` (`access.ts:206`) is the affordance → capability bridge: each
    task-pane control and each `CELNET.*` cell declares its gating capability.
  - `denyIfUngated(id)` (`functions.ts:117`) is the cell-side gate: a SIGNED-IN
    caller lacking the capability gets `#CELNET_DENIED! <reason>`; an ANONYMOUS
    caller stays permissive (the server still enforces every request).
  - `UserSession` (`transport/session.ts`, shared singleton via
    `runtime.getSession()`) drives `login`/`logout` and answers
    `isSignedIn()` / `canEntry(id)` / `entryDenialReason(id)`.
- **Task pane** (`excel/src/taskpane/taskpane.ts`) is **entirely FX-centric today**:
  an asset-class selector (FX/metal/equity/commodity/crypto), the polymorphic
  instrument builder (`instrumentBuilder.ts`), the live touch tiles, the ranked
  **multi-dealer RFQ panel** (`dealerPanel.ts`) with click-to-trade booking, and the
  **Contribute** (mark) panel — plus sign-in. There is **no FI surface in the pane**.

### A.3 How the launcher is surfaced in the GUI rail

`gui/src/lib/commands.ts` is the single rail/command registry. `RAIL` declares one
entry per workspace, each tagged with a `domain` (`fx-options` | `fixed-income` |
`administration`). The Excel launcher is currently:

```
{ id: "excel", glyph: "▦", label: "Excel", domain: "administration" }   // commands.ts:115
```

It is in the `administration` domain but **deliberately not admin-only** (absent
from `ADMIN_ONLY_WORKSPACES`, `commands.ts:156`), so `workspaceAccessible` returns
`true` for any signed-in user (`commands.ts:189-194`). Nav-gating
(`domainAccessible`, `commands.ts:171`) hides whole domain tabs a user can't see:
`fixed-income` is gated on `view · fixed_income`. The task notes the `excel` item is
being moved to `fx-options`; the FI Excel tab is the **sibling rail item under
`fixed-income`**.

---

## B. Required FI function surface

Each proposed function below lists inputs, output (cell/array shape), the wire RPC
it maps to, and the gating capability. RPC availability is stated against today's
WS mirror (`crates/celnet-server/src/ws/mod.rs`).

### B.1 `CELNET.RATES` — price a linear-rates instrument (EXISTS; extend)
- **Today:** OIS only (§A.1). `RatesInstrument` oneof = `ois` only
  (`celnet.proto:3951`).
- **Extend when wired:** add FRA and vanilla-swap arms (engine math exists;
  blocked on the wire `oneof` — see the curves review §B). Keep the same spill
  geometry; add arm-specific terms via a key/value range (mirroring `INSTRUMENT`).
- **RPC:** `price_rates` (mirror `ws/mod.rs:717`). **Capability:** `price · fixed_income`.

### B.2 `CELNET.DV01` / `CELNET.KEYRATE` — rates risk projections (BUILDABLE NOW)
- **Inputs:** identical to `CELNET.RATES` (curve + instrument).
- **Output:** `CELNET.DV01` → a 2-row spill `[pv01, dv01]`; `CELNET.KEYRATE` → the
  `kr_dv01[<tenor>Y]` ladder (one row per pillar) + a `dv01` total footer.
- **RPC:** none new — both are **projections of the existing `price_rates` result**
  (`RatesPricingResult` already carries `pv01`, `dv01`, the key-rate ladder). They
  are ergonomic single-purpose views of the same call `CELNET.RATES` makes.
- **Capability:** `price · fixed_income` (same as `RATES`).

### B.3 `CELNET.RATESRISK` — server-rolled rates risk cube (RPC ON WS)
- **Inputs:** roll-up dimension (e.g. DESK/BOOK/TRADER), optional scope, numeraire.
- **Output:** a node grid (header + one row per rolled-up node + footer) — the FI
  analogue of `CELNET.RISK`.
- **RPC:** `aggregate_rates_risk` (mirror `ws/mod.rs:798`). **Needs an Excel client
  method** (`Connection.aggregateRatesRisk`) + codec; the FX `RISK` is the template.
- **Capability:** `view · fixed_income` (read; server enforces — mirror FX `RISK`,
  which is client-ungated).

### B.4 `CELNET.RATESPOSITIONS` / `CELNET.RATESBOOK` — the rates Book (RPC ON WS)
- **`CELNET.RATESPOSITIONS`** — list entitled rates positions. Output: a leaf grid
  (header + one row per `RatesPosition` + count/empty footer). **RPC:**
  `list_rates_positions` (mirror `ws/mod.rs:831`). **Capability:** `view · fixed_income`.
- **`CELNET.RATESBOOK`** — book a rates position. **RPC:** `book_rates_position`
  (mirror `ws/mod.rs:823`). **Capability:** `book · fixed_income`. Because booking is
  a state change, prefer a **two-phase, task-pane-confirmed** action like
  `CELNET.MARK` (stage in the cell, commit in the pane) rather than a silent recalc
  write (`EXCEL-INTEGRATION.md §3.3 "a recalc never silently writes"`).

### B.5 `CELNET.DEALS` — FI deal blotter (RPC ON WS)
- **Inputs:** optional scope/filter. **Output:** a deal grid (one row per `Deal`).
- **RPC:** `list_deals` (mirror `ws/mod.rs:872`). **Needs an Excel client method.**
- **Capability:** `view · fixed_income`.
- **Note:** the `Deal` model is rates-derivative shaped (OIS/swap/FRA), not
  cash-bond — see the bond gap-analysis. A bond-capable blotter is a separate build.

### B.6 `CELNET.RATESRFQ` — FI dealer quoting (RPC ON WS; prefer task pane)
- **RPCs:** `submit_desk_request`, `respond_desk_request`, `accept_desk_quote`,
  `list_desk_requests` (mirror `ws/mod.rs:840-869`).
- **Capabilities:** submit/respond → `rfq_respond` / `ioi_respond · fixed_income`;
  accept (lift) → `execute · fixed_income`.
- **Recommendation:** the desk-quoting flow is **stateful and multi-step** (inbox →
  quote → lift) and maps poorly to stateless cell recalcs. Surface it as a
  **task-pane affordance** (the FI analogue of the FX multi-dealer panel), with at
  most a read-only `CELNET.RATESRFQ` listing cell over `list_desk_requests`.

### B.7 `CELNET.CURVE` — pull a curve (GAP — NEW RPC REQUIRED)
- **Desired inputs:** par pillars + reference date (+ currency); optional query
  tenors/dates. **Desired output:** a grid of discount factor / zero rate / forward
  per pillar or per queried tenor.
- **Blocker:** **no curve-read RPC exists.** `PricingService` = `Price` + `PriceRates`
  only; the bootstrapped curve lives inside `price_rates` and is never returned as a
  curve object. Options: (a) add a `GetCurve` RPC; (b) add a curve block (DF/zero/fwd
  by pillar) to `RatesPriceResponse`. **Capability:** `view · fixed_income`.
- See the curves review §A for the related broken-date / custom-tenor pillar gap
  (the wire `OisPillar.tenor_years` is whole-years-only).

### B.8 `CELNET.RATESSUBSCRIBE` — stream live rates (GAP — NEW STREAMING PATH)
- **Blocker:** the RFS session subscribes FX/cross-asset `Instrument`s only; there
  is **no rates subscription frame** (no `subscribe_rates` / `rates_stream` in
  `ws/`). `CELNET.RATESSUBSCRIBE` needs a new streaming RPC + a rates stream
  registry mirroring `streamRegistry.ts`. **Capability:** `stream · fixed_income`.
- Be honest in the doc/UX: until built, FI has **no live streaming** — only
  request/response pricing.

---

## C. Tab / pane surfacing

### C.1 GUI rail launcher (mirror the FX `excel` item)

Add a sibling rail entry under the `fixed-income` domain — e.g.:

```
{ id: "ratesexcel", glyph: "▦", label: "Rates Excel", domain: "fixed-income" }
```

Consequences via `commands.ts` (no new gating code needed):
- `domainOf("ratesexcel") = "fixed-income"`; `workspaceAccessible` →
  `domainAccessible("fixed-income")` → `can("view", "fixed_income")`
  (`commands.ts:171-194`). So a user without FI view never sees the FI Excel item —
  correct and consistent with the FX-side move of `excel` to `fx-options`.
- It is **not** added to `ADMIN_ONLY_WORKSPACES`, so any FI-entitled user reaches it.
- Its `⌘N` chord is derived automatically from `RAIL` order (`railChord`), and the
  command-palette entry + cheatsheet are projections of the registry — one row adds
  the whole binding.
- The single-source command-registry invariant (`test/commands.test.ts`) and the
  rail/AppContext redirect all pick it up for free.

### C.2 Task-pane organization (FX vs FI)

Two viable shapes; recommend **(a)** for clarity and capability-fit:

- **(a) Capability-adaptive sections in one pane.** Keep one task pane (one shared
  runtime, one connection, one sign-in) and render **FX-options** and **Fixed-Income**
  sections, each shown/enabled by the signed-in user's capabilities (`can(...,
  "fx_options")` vs `can(..., "fixed_income")`). The FI section hosts the rates
  ticket (OIS/swap/FRA terms), a rates risk read-out, the **rates desk-quoting
  panel** (the FI analogue of the multi-dealer panel — submit/respond/accept), and a
  two-phase **Book** action. This reuses the existing sign-in, transport, and gating
  chassis verbatim.
- **(b) A separate FI task pane** (a second `ShowTaskpane` action + page). Cleaner
  separation but duplicates the runtime/boot and splits the one connection's session
  UX across two panes — not recommended given the shared-runtime model.

Either way, respect the layers already built: the **server enforces** every RPC; the
pane only narrows affordances for a signed-in user via `ENTRY_POINTS` + `can()`;
anonymous stays permissive.

---

## D. Manifest / registration & build

`excel/manifest.xml` and `excel/src/functions/functions.json` (the Office.js
custom-function metadata) need:

1. **Register the new functions.** Add `@customfunction` JSDoc + a
   `cf.associate(...)` line in `registerAll` (`functions.ts:940`) for each FI
   function, and the matching metadata entry in `functions.json`. Namespace stays
   `CELNET` (the manifest `Celnet.Functions.Namespace` shortstring) — FI functions
   are `CELNET.RATES`, `CELNET.CURVE`, `CELNET.DV01`, … (no new namespace).
2. **(Optional) a ribbon group/tab.** Today the manifest has one `Celnet.Group`
   ("Celnet FX Options") with one button opening the (FX) ticket pane. To surface FI
   distinctly, either add a second `Control` (button) in the same group that opens
   the FI section, or add a second `Group` ("Celnet Fixed Income"). Custom functions
   themselves need no ribbon entry — they register via the `CustomFunctions`
   extension point regardless.
3. **Cosmetic/accuracy updates.** The manifest `DisplayName`/`Description`/group
   label say "FX Options" and the function list in the comment + `GetStarted`
   description omit the rates functions — update these so the add-in name and the
   advertised function list reflect the combined FX + FI surface.
4. **Function metadata.** Mark `CELNET.RATESSUBSCRIBE` (if/when built) `@streaming`
   like `SUBSCRIBE`/`SERIES`; the rest are request/response (return a `SpillMatrix`).

No new manifest *capabilities* are required — the add-in is `ReadWriteDocument`
already and the WS endpoint is resolved at runtime (`runtime.ts resolveEndpoint`).

---

## E. Permissioning

The capability algebra is `Action × AssetClass`, with `fixed_income` already a
first-class asset (`access.ts:35`). Every proposed FI function maps cleanly onto an
`ENTRY_POINTS` row (`access.ts:206`) with `asset: "fixed_income"`:

| Function | `EntryPointId` (new) | action | asset | surface | gate |
|---|---|---|---|---|---|
| `CELNET.RATES` | `rates` (exists) | `price` | `fixed_income` | cell | `denyIfUngated` |
| `CELNET.DV01` | `dv01` | `price` | `fixed_income` | cell | `denyIfUngated` |
| `CELNET.KEYRATE` | `keyrate` | `price` | `fixed_income` | cell | `denyIfUngated` |
| `CELNET.RATESRISK` | `rates_risk` | `view` | `fixed_income` | cell | (read; server enforces) |
| `CELNET.RATESPOSITIONS` | `rates_positions` | `view` | `fixed_income` | cell | (read) |
| `CELNET.RATESBOOK` | `rates_book` | `book` | `fixed_income` | taskpane | `denyIfUngated` (+ 2-phase) |
| `CELNET.DEALS` | `deals` | `view` | `fixed_income` | cell | (read) |
| `CELNET.RATESRFQ` (submit/respond) | `rates_rfq` | `rfq_respond` / `ioi_respond` | `fixed_income` | taskpane | `denyIfUngated` |
| `CELNET.RATESRFQ` (accept) | `rates_accept` | `execute` | `fixed_income` | taskpane | `denyIfUngated` |
| `CELNET.CURVE` | `curve` | `view` | `fixed_income` | cell | (read) |
| `CELNET.RATESSUBSCRIBE` | `rates_subscribe` | `stream` | `fixed_income` | cell | `denyIfUngated` |

**The existing gating extends cleanly:**
- `denyIfUngated(id)` (`functions.ts:117`) and `entryDenialReason` are
  asset-agnostic; a new `ENTRY_POINTS` row with `asset: "fixed_income"` produces the
  honest `#CELNET_DENIED! Your permissions don't allow …` sentence
  (`capabilityDenialTitle` → `ASSET_ADJECTIVE.fixed_income = "fixed-income"`,
  `access.ts:112`) — byte-identical to the GUI's voice.
- Read functions (risk/positions/deals/curve) follow the FX precedent of being
  **client-ungated** (the FX `RISK`/`POSITIONS`/`LIMITS` are not wrapped in
  `denyIfUngated`); the server enforces `view · fixed_income`. Price/book/stream/
  desk-action functions ARE client-gated, matching the FX price/mark/subscribe set.
- The taskpane desk-quoting affordances reuse the `requiresSignIn: true` posture of
  the FX `rfq`/`book`/`contribute` entry points (disabled-with-prompt while
  anonymous).

No change to the capability model, the wire `CapabilityDesc`, or the server
entitlement algebra is required — only new `ENTRY_POINTS` rows.

---

## F. Gaps & recommendations

### F.1 What exists vs what must be built

**Reachable today (WS mirror already routes these — Excel needs only client
methods + functions + codecs, no server work):**
- `price_rates` (already wired into `CELNET.RATES`)
- `aggregate_rates_risk`, `book_rates_position`, `list_rates_positions`
- `submit_desk_request`, `respond_desk_request`, `accept_desk_quote`,
  `list_desk_requests`, `list_deals`
- `stream_notifications` (FI desk push — could drive a task-pane notification
  center, mirroring the GUI's NotificationCenter)

**Must be built on the wire first (blocked):**
- **`GetCurve` (or a curve block on `RatesPriceResponse`)** for `CELNET.CURVE`.
- **A rates streaming RPC + stream registry** for `CELNET.RATESSUBSCRIBE`.
- **FRA / vanilla-swap wire arms** on `RatesInstrument` to extend `CELNET.RATES`
  beyond OIS (engine math exists; see curves review §B).
- **Cash-bond pricing path** (`BondInstrument` on the wire + accrued/settlement +
  credit-spread source) — the largest gap; out of scope for a first FI Excel slice
  (see the bond gap-analysis and curves review §D.2). Do **not** promise a
  `CELNET.BOND` price function until that path lands.

### F.2 Honest non-coverage to state in the UX/docs
- **No live FI streaming** until a rates stream RPC exists — FI is request/response
  only at first.
- **No curve export** until `GetCurve` exists.
- **OIS-only pricing** until FRA/swap/bond arms are on the wire.
- The FI `Deal`/blotter is **rates-derivative shaped, not cash-bond** (no ISIN /
  clean-dirty / yield / spread / accrued / settlement) — see the bond gap-analysis.

### F.3 Sequenced plan

1. **Move the FX `excel` rail item to `fx-options`** (in flight) and **add the FI
   `ratesexcel` rail item under `fixed-income`** (`commands.ts`) — gating is free.
2. **Add the FI client methods to `excel/src/transport/connection.ts`** for the nine
   already-mirrored FI frame types + their codecs in `contract/`. No server change.
3. **Ship the no-new-RPC FI cell functions:** `CELNET.RATES` extensions (terms-range
   form), `CELNET.DV01`, `CELNET.KEYRATE` (projections of `price_rates`),
   `CELNET.RATESRISK`, `CELNET.RATESPOSITIONS`, `CELNET.DEALS` — each with a new
   `ENTRY_POINTS` row + `denyIfUngated`/read posture + `functions.json` metadata.
4. **Add the FI task-pane section** (capability-adaptive, option C.2(a)): rates
   ticket, rates risk read-out, the **desk-quoting panel** (submit/respond/accept
   over the desk RPCs), a **two-phase Book** action, and (optionally) a notification
   center over `stream_notifications`.
5. **Update the manifest** (name/description/group label + function list +
   registrations) to reflect the combined FX + FI surface.
6. **Then, gated on new wire work:** `GetCurve` → `CELNET.CURVE`; a rates stream RPC
   → `CELNET.RATESSUBSCRIBE`; FRA/swap arms → `CELNET.RATES` extension; cash-bond
   path → a future `CELNET.BOND`.

Each step is gated (T1/T2) and live-e2e-verified under `Enforce` wherever it touches
gui/excel/wire, with FX cells byte-identical and the FI functions validated against
the GUI/SDK/CLI for the same inputs (the one-contract parity rule).

---

## Appendix — files & symbols cited

- **Excel functions:** `excel/src/functions/functions.ts` (`RATES` 401-419;
  `denyIfUngated` 117; `registerAll` 940), `excel/src/functions/shaping.ts`
  (`shapeRatesCurve`/`shapeOisInstrument`/`formatRatesSpill`),
  `excel/src/functions/runtime.ts` (`getConnection`/`getSession`),
  `excel/src/functions/functions.json` (CF metadata).
- **Excel contract/transport:** `excel/src/contract/access.ts`
  (`Capability`/`can`/`ENTRY_POINTS`/`capabilityDenialTitle`),
  `excel/src/transport/connection.ts` (`priceRates` 826; `requestQuote`,
  `aggregateRisk`, `listPositions`, `login`/`logout`; `Authenticate` frame),
  `excel/src/transport/session.ts` (`UserSession`),
  `excel/src/contract/contract.ts` (`RatesCurveSet`/`OisInstrument`/`RatesPricingResult`),
  `excel/src/contract/wsCodec.ts` (`ratesPricingResultFromWire`),
  `excel/src/taskpane/taskpane.ts` (FX-centric pane), `excel/manifest.xml`.
- **GUI rail:** `gui/src/lib/commands.ts` (`RAIL` 91-116; `domainAccessible` 171;
  `workspaceAccessible` 189; `ADMIN_ONLY_WORKSPACES` 156).
- **Wire / server:** `crates/celnet-proto/proto/celnet.proto`
  (`PricingService` 2992; `RiskService` 3035 incl. `AggregateRatesRisk` 3056 /
  `BookRatesPosition` 3061 / `ListRatesPositions` 3064; `RfqDeskService` 4462;
  `NotificationService` 4472; `RatesInstrument` 3951 = OIS-only; `CurveSet` 3917;
  `RatesPricingResult` 3963; `RatesPosition` 4024; `Deal` 4348),
  `crates/celnet-server/src/ws/mod.rs` (mirror handlers: `price_rates` 717,
  `aggregate_rates_risk` 798, `book_rates_position` 823, `list_rates_positions` 831,
  `submit/respond/accept_desk` 840-861, `list_desk_requests` 864, `list_deals` 872,
  `stream_notifications` 576).
- **SDK:** `crates/celnet-client/src/rates.rs` (`UsdSofrCurve`/`Ois` builders),
  `Client::price_rates` (lib.rs).
- **Grounding gap docs:** `docs/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md`,
  `docs/FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`, `docs/EXCEL-INTEGRATION.md`.
</content>
</invoke>
