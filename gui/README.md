# Celnet Trader GUI

The FX-options trader front end for Celnet — an Apple/macOS-grade single-window
workspace that **out-intuits SynOption Optimus** and every incumbent. Built per
[`docs/GUI-DESIGN.md`](../docs/GUI-DESIGN.md) on the **Celnet Aurora** design
system.

This directory is intentionally **outside** the Cargo workspace: it is a
standalone web app and does not touch the Rust build.

## Stack

- **Vite 6 + React 19 + TypeScript** (strict, `exactOptionalPropertyTypes`,
  `noUncheckedIndexedAccess` — the full strict surface).
- **Bespoke design-token system** (no heavy UI kit): semantic OKLCH tokens, a
  1.20 type scale, materials/elevation, purposeful motion — first-class light
  **and** dark mode plus an Increased-Contrast appearance.
- **Canvas 2D + WebGPU-ready** bespoke visualizations (vol surface, per-tenor
  smile, sparklines, diverging-ramp risk heatmap). No chart library.
- Zero runtime UI dependencies beyond React — every component is hand-rolled on
  the token layer.

## Run

```bash
cd gui
npm install          # one-time
npm run dev          # Vite dev server  (http://localhost:5173)
npm run build        # tsc -b + Vite production build → dist/
npm run preview      # serve the production build (http://localhost:4173)
npm run typecheck    # strict tsc --noEmit, no build
```

Requires Node ≥ 22 (developed on Node 26 / npm 11).

## What's here

### Design system (`src/design`, `src/components`, `src/viz`)
- `design/tokens.css` — the Aurora token contract (type, OKLCH semantic color
  for Dark/Light/Increased-Contrast, spacing, radius, elevation/materials,
  motion). `design/appearance.ts` toggles appearance via a `data-` attribute on
  `<html>`; the cascade does the rest.
- Core components: `PriceTile` (tabular, flash-on-change with decay + glyph),
  `TwoWayQuote`/blotter cells, `ConventionChip` (conventions on the face),
  `GreeksStrip` (Δ Γ ν Θ + full 13-Greek expand), `Sparkline` (Canvas),
  `StatusBadge` (◉/◐/○ stream health), `LastLookRing` (depleting countdown),
  `ArbBanner`, `CommandPalette` (⌘K, fuzzy), `UniverseNavigator` (⌘B — the
  command-palette-style pair browser over the full instrument universe, grouped
  Majors/Crosses/EM with favourites), `PairMenu`/`PairStrip` (pair switcher +
  watchlist), `ScopeBreadcrumb` (the firm→…→leaf risk scope path), `DatePicker`
  (tenor incl. ON/TN/SN/IMM or an arbitrary broken date), `CelnetMark` (the
  geometric brand mark), `ShortcutsOverlay`, `Panel`, `Button`.
- `viz/SurfaceMesh` (rotatable projected 3D surface, WebGPU-capability-detected,
  Canvas mesh today), `viz/SmileChart` (per-tenor smile), `viz/ramp` (perceptual
  diverging ramp — never rainbow, colourblind-safe).

### Hero screens (`src/workspaces`)

The single-window Shell (`src/app/Shell.tsx`) mounts a persistent left rail of
**five** workspaces (⌘1–⌘5) plus a sixth view reachable as a toggle, all driven
by one shared app context (the rail, command palette, and pair switcher re-target
the same state):

- **Ticket** (⌘1) — the RFQ / click-to-trade card: analytics **and** executable
  in one card, conventions on the face, inline Solve, a visible last-look
  countdown ring, ⏎ to request / ⌘⏎ to lift. It builds the full `Product` oneof —
  not just vanilla but multi-leg and the exotic catalogue (single/double barriers,
  Asian, American/Bermudan, TARF, …) — and prices each through the live transport.
- **Stream** (⌘2) — the RFS streaming blotter, the **resting state**. Multiplexed
  two-ways; only changed numbers flash; per-row health is honest; **click a side
  to trade** (presents the line's short-lived `TradableToken`, surfaces a typed
  Executed / StreamReject toast). Virtualised (windowed) so it scales to thousands
  of rows; group/collapse by pair/tenor with real aggregates.
- **Surface** (⌘3) — two linked views via an in-workspace toggle: **mark** (the 3D
  surface + per-tenor smile + broker marking grid — edit ATM/RR/BF to reprice live
  with an arb guard and inline provenance) and **cube** (the pair×tenor×delta
  vol-cube pivot/heatmap built from the server's calibrated smiles, with a
  cell→smile drill; no client-side vol math).
- **Risk** (⌘4) — the spot×vol scenario shock grid (diverging tint, anchored "now"
  cell) with a vega ladder + cross-gamma disclosure.
- **Book** (⌘5) — the firm-scale **hierarchical risk** view, computed
  **server-side** over the `RiskService` contract: `aggregate_risk` rolls up the
  org cube along the active Scope's dimension into one reporting numeraire, and a
  `drill_risk` opens a node's largest contributing position in Risk (Book and Risk
  are the same cube at two zooms). It never loops positions and sums client-side.

The **UniverseNavigator** (⌘B) is the command-palette-style pair browser over the
full instrument universe (grouped Majors / Crosses / EM, favourites,
keyboard-first).

### Data layer (`src/data`) — the contract seam
- `contract.ts` — a typed mirror of the **single, current** `celnet-proto`
  contract (`celnet.wire`): Instrument / Quote / Snapshot+Update / Greeks /
  Smile / Scenario, etc. One contract, no versioning (CLAUDE.md rule 9).
- `transport.ts` — the **isolated transport seam** (`CelnetTransport`): the only
  interface the app talks to. Two transports satisfy it, selected once at the app
  root by `transportConfig.ts`:
  - `wsTransport.ts` — a **live** client over the `celnet-server` WebSocket JSON
    mirror (`crates/celnet-server/src/ws`) — **the default**, so every number on
    every screen is the server's (the GUI adds no pricing). It speaks the SAME
    single `celnet.wire` contract encoded as the mirror's type-tagged snake_case
    JSON (`wsCodec.ts` + `enums.ts` mirror the server's `codec.rs` field-for-field),
    multiplexes the RFS session and the request/response calls over one connection,
    tracks per-subscription sequence with **gap-detect → server-assisted `resync`**,
    matches correlation-less replies (`smile` / `mark_surface_response` / `scenario`)
    to their FIFO waiter, parses/serializes 64-bit identities (tradable `token`,
    nanos, ids) **losslessly** so click-to-trade `Execute`-by-token round-trips
    exactly, and **auto-reconnects** with capped backoff (re-subscribing and
    resyncing every live line; pending request/response calls fail fast on a drop);
  - the deterministic in-app `mockSource.ts` (seeded PRNG tape, real GK pricing +
    13 Greeks, surface calibration, scenario repricing) — an explicit **offline
    opt-in** for a no-server demo / design review.
  The default is live `ws://127.0.0.1:8081`; override the endpoint with the URL
  param `?ws=ws://host:port` or build-time env `VITE_CELNET_WS_URL`. Force the
  offline mock with the URL flag `?mock` or `VITE_CELNET_TRANSPORT=mock` — the
  status ribbon reads `live ws://…` by default and `mock/replay` only when opted in.
  Nothing else in the app changes; `contract.ts` is swapped for the `buf`-generated
  module when that lands so the GUI cannot drift from the wire.
- Supporting data modules: `riskView.ts` (shapes the `RiskService`
  aggregate/drill/limit responses for the Book/Risk workspaces), `cube.ts` (the
  vol-cube pivot model over the server's marked smiles), and the offline-mock
  numerics (`pricing.ts` GK + 13 Greeks, `surface.ts` calibration, `rng.ts`/
  `seed.ts` the seeded PRNG tape) that back `mockSource.ts` only.

## Performance discipline

The render layer inherits the platform's P99 budget: streaming updates are
coalesced into **one paint per frame** (rAF batching) so a burst tape never
blows the frame budget. The status ribbon shows a **real measured P99**
inter-frame render time, not decoration.

## Notes

- **Positions & hierarchical risk are live** in the **Book** workspace over the
  shipped `RiskService` contract (`list_positions` / `aggregate_risk` /
  `drill_risk` / `limit_status`) — the firm cube is rolled up server-side and
  drawn directly. What remains off-wire by design is **live per-trade P&L
  attribution** (the designed `GetPosition` / `AttributePnl` calls in
  `docs/GUI-DESIGN.md`): no realised/unrealised P&L column is drawn because that
  number is not yet on the wire (no fake P&L — CLAUDE.md rule 2). The attribution
  chain (owner/book/desk) *is* on the wire and is shown.
- Build artifacts (`node_modules/`, `dist/`) are gitignored.
