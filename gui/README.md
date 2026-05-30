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
  `TwoWayQuote`/blotter cells, `ConventionChip`/`ConventionRow` (conventions on
  the face), `GreeksStrip` (Δ Γ ν Θ + full 14-Greek expand), `Sparkline`
  (Canvas), `StatusBadge` (◉/◐/○ stream health), `LastLookRing` (depleting
  countdown), `ArbBanner`, `CommandPalette` (⌘K, fuzzy), `Panel`, `Button`.
- `viz/SurfaceMesh` (rotatable projected 3D surface, WebGPU-capability-detected,
  Canvas mesh today), `viz/SmileChart` (per-tenor smile), `viz/ramp` (perceptual
  diverging ramp — never rainbow, colourblind-safe).

### Hero screens (`src/workspaces`)
- **Stream** — the RFS streaming blotter, the **resting state**. Multiplexed
  two-ways; only changed numbers flash; per-row health is honest; **click a side
  to trade** (presents the line's short-lived `TradableToken`, surfaces a typed
  Executed / StreamReject toast).
- **Ticket** — the RFQ / click-to-trade card: analytics **and** executable in one
  card, conventions on the face, inline Solve, a visible last-look countdown
  ring, ⏎ to request / ⌘⏎ to lift.
- **Surface** — the 3D surface + per-tenor smile + broker marking grid, linked;
  edit ATM/RR/BF to reprice live with an arb guard and inline provenance.
- **Risk** — the spot×vol scenario shock grid (diverging tint, anchored "now"
  cell) with a vega ladder + cross-gamma disclosure.

### Data layer (`src/data`) — the contract seam
- `contract.ts` — a typed mirror of the **single, current** `celnet-proto`
  contract (`celnet.wire`): Instrument / Quote / Snapshot+Update / Greeks /
  Smile / Scenario, etc. One contract, no versioning (CLAUDE.md rule 9).
- `transport.ts` — the **isolated transport seam** (`CelnetTransport`): the only
  interface the app talks to. Two transports satisfy it, selected once at the app
  root by `transportConfig.ts`:
  - the deterministic in-app `mockSource.ts` (seeded PRNG tape, real GK pricing +
    14 Greeks, surface calibration, scenario repricing) — **the default**, so the
    app runs standalone with no server;
  - `wsTransport.ts` — a **live** client over the `celnet-server` WebSocket JSON
    mirror (`crates/celnet-server/src/ws`), speaking the SAME single `celnet.wire`
    contract encoded as the mirror's type-tagged snake_case JSON (`wsCodec.ts` +
    `enums.ts` mirror the server's `codec.rs` field-for-field). It multiplexes the
    RFS session and the request/response calls over one connection, tracks
    per-subscription sequence with **gap-detect → server-assisted `resync`**, and
    **auto-reconnects** with capped backoff (re-subscribing and resyncing every
    live line; pending request/response calls are failed fast across a drop).
  Select the live transport at build time with `VITE_CELNET_TRANSPORT=ws`
  (endpoint via `VITE_CELNET_WS_URL`, default `ws://127.0.0.1:8081`). Nothing else
  in the app changes; `contract.ts` is swapped for the `buf`-generated module when
  that lands so the GUI cannot drift from the wire.

## Performance discipline

The render layer inherits the platform's P99 budget: streaming updates are
coalesced into **one paint per frame** (rAF batching) so a burst tape never
blows the frame budget. The status ribbon shows a **real measured P99**
inter-frame render time, not decoration.

## Notes

- **Positions / P&L** is designed in `docs/GUI-DESIGN.md` but depends on the
  API-v2 `GetPosition`/`AttributePnl` services not yet on the wire; it is
  deliberately **not** drawn here (no fake P&L — CLAUDE.md rule 2).
- Build artifacts (`node_modules/`, `dist/`) are gitignored.
