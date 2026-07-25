---
name: rates-position-store-book-plan
description: Approved plan for the rates position store + GUI rates Book workspace (next build after compact)
metadata: 
  node_type: memory
  type: project
  originSessionId: 3759f335-a305-4b8e-b3aa-727dea5a173c
---

**STATUS: DELIVERED 2026-06-27 (`25293f8`, deployed UAT).** Shipped as part of the
FI dealer-quoting slice — `RatesPositionStore` + `RiskService.BookRatesPosition`/
`ListRatesPositions` (+WS mirrors) + GUI `RatesBookWorkspace` ("ratesbook", Fixed
Income). The accept-quote flow also books a `RatesPosition`. See [[fi-dealer-quoting-shipped]].

Approved 2026-06-27 (user picked options via AskUserQuestion). Build on branch
`fixedincom_risk_ui`. This is the last remaining §4.2 FI workspace (Book).

**Write path decision: a new `BookRatesPosition` RPC.** Rates has no FIX dialect
(options positions arrive via FIX → `record_booked_position` → in-memory
`PositionStore`); rates positions are currently inline-only. So add an explicit
booking RPC the GUI/Excel calls. FIX rates ingestion is deferred to a later phase.

**Four pieces (mirror the options precedents):**
1. `RatesPositionStore` — in-memory, `RwLock` (NOT disk-persisted; the options
   `PositionStore` at `crates/celnet-server/src/services/risk/store.rs` is in-memory
   too). Methods `upsert`/`snapshot`/`list`. New module under
   `crates/celnet-server/src/services/rates/` (or rates_risk).
2. `BookRatesPosition(BookRatesPositionRequest) -> BookRatesPositionResponse` RPC
   (proto additive; assigns a position_id, upserts into the store) + WS-mirror
   (ws/codec.rs + ws/mod.rs dispatch arm) — mirror how `AggregateRatesRisk` was
   mirrored in commit `bad2625`.
3. `ListRatesPositions(scope) -> RatesPosition[]` RPC + WS-mirror — mirror the
   options `ListPositions` (`risk/mod.rs:692`, `wsTransport.ts` listPositions).
   Entitlement-prune like options (deny-by-default; resolve_caller + authorize_caller).
4. GUI `RatesBookWorkspace.tsx` (id e.g. `ratesbook`) under the **Fixed Income**
   tab — book a position + list the store via the two new transport methods;
   mirror `BookWorkspace.tsx`. Register in the 4 nav sites (Shell WORKSPACE_VIEW,
   commands RAIL+WorkspaceId+domain "fixed-income", savedViews, AppContext) and
   add a `domainOf` entry. Also add the in-app `MockTransport` real implementations.

**Reuse existing types:** `RatesPosition` (proto + gui contract.ts: position_id,
entity, book, instrument: OisInstrument) already exists from `AggregateRatesRisk`.

**Gates:** server `cargo check/test/clippy -p celnet-server`; GUI **`npm run build`**
(tsc -b, the real gate) + vitest — see [[gui-gate-uses-production-build]]. Deploy
via `deploy/celnet-deploy.sh -t uat release` (option 2) once gated.
