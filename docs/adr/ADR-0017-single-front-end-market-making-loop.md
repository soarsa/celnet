# ADR-0017 — One sell-side front-end: persona lenses over the market-making loop

- **Status:** Accepted as a **design direction** (2026-07-01). Governs the GUI/Excel
  redesign (mockup corpus `docs/gui-redesign/`, 16 surfaces `00-shell` … `15-rates-ticket`).
  Records the intended experience architecture; the current `gui/` + `excel/` remain
  authoritative until each surface lands. Honours ADR-0007 (one unversioned contract),
  ADR-0008 (multi-asset carry architecture), ADR-0010 (FI/rates onto the carry seam),
  ADR-0012 (unified gBSM kernel) and CLAUDE.md guardrails #8/#9/#10/#11.
- **Deliverable:** `single-front-end-experience`
  (`docs/acceptance/single-front-end-experience.acceptance.json`).

## Context (grounded in the code, not aspiration)

Celnet is a **sell-side market-maker pricing, distribution & administration platform**
(replacing Fenics / Synoption / ION-class terminals) — the users *price, model, mark,
contribute, administer and manage feeds*; **no order/execution/matching engine exists**
(code-verified: `search_graph OrderBook|ExecutionEngine|MatchingEngine` returns only an
unrelated fanout queue). Two structural defects motivated a full redesign
(`docs/gui-redesign/DISCOVERY.md`, `FRONTEND-INVENTORY.md`):

1. **Replication born of asset-class silos** — parallel FX-vs-FI stacks sharing ~0
   components, a blotter built 4×, FX-first state with non-FX as a clearable overlay.
2. **Capabilities the API can price but no client surfaces** — XVA, crypto strike-axis,
   feed compositing/health, model/plugin management, observability, reporting.

The backend is already largely unified: a 26-arm class-orthogonal `Instrument` oneof, a
`ProductEngine` registry (`price_instrument` → `engines::dispatch`), one versioned
`SurfaceBook`, one `SpreadModel` pricing streaming/RFQ/FIX alike, an engine-enforced
`AssetClass × Action × Desk` entitlement kernel, and the carry-seam convergence that makes
rates a peer term-structure (ADR-0010). The **one genuinely-integrated loop** today is the
streaming LP-maker chain: `handle_subscribe` → `attribution::resolve` → `record_booked_position`
→ `PositionStore::book_from_attribution` (`docs/gui-redesign/KNOWLEDGE-flow-links.md`, LIVE).

## Decision

Ship **one** front-end (GUI + Excel), a projection of the one canonical contract, built as
**persona lenses over a single closed market-making loop**:

```
FEEDS → PRICING/MODEL → CONTRIBUTION (Celnet-as-LP make) ⇄ RFQ / LP-TAKE + SALES-TRADER
      → BOOKS/POSITIONS → RISK/XVA → OPS      (and RISK feeds back into pricing/contribution SKEW)
```

Five design commitments:

1. **The loop is the spine, every surface is a lens on it.** Not 15 hard-mounted
   workspaces — a command-palette-first `entity(underlier) × task(verb)` grammar joined by a
   Desk▸Book▸Underlier scope bus, with saved *perspectives*. Cross-capability continuity:
   from any price drill to its surface, risk, XVA, or send to RFQ without changing "workspace".

2. **Persona lenses, one dataset.** Quant · trader/market-maker · sales-trader · risk-manager ·
   desk-head each get a default panel arrangement over the shared loop — same data, different lens.

3. **Per-asset-class licensing is a design primitive.** The UI mirrors the engine's
   `AssetClass × Action × Desk` capability kernel (deny-wins): licensed = present, unlicensed =
   subtly gated / upsell, **never a ticket the engine will reject**
   (`cross_asset_non_vanilla_product_is_rejected`). Tiers: FX+Metals (full) / Equity·Commodity·
   Crypto (vanilla + landing perpetual-future) / Rates-FI (curve suite).

4. **FI/rates is a first-class PEER, not a siloed app.** The asset-class selector treats
   Rates/FI as a peer everywhere; FI flows through the SAME shared surfaces as FX
   (price/model/contribute/risk/books) **plus** the FI-specific surfaces FX lacks — a curve
   construction workbench and a rates-instrument ticket. Reflects the ADR-0010 carry-seam
   convergence (curve = general case; flat FX carry = 2-curve degenerate).

5. **Honest LIVE / TARGET link convention.** Every cross-capability link and every surface is
   rendered with an explicit **LIVE** (wired end-to-end via ≥1 client) or **TARGET** (design
   shows the link; the wiring is a named gap) badge — never fake a wired link. The high-value
   TARGET legs (feeds→pricing, risk→skew, unified cross-asset book, XVA) are shown *and* drive
   the wiring roadmap (⟂ the D-xva / feed-activation cargo lanes).

Rendered in the **July-2026 SOTA design language** (`DESIGN-LANGUAGE.md`): re-founded coral
`#ff7357` + indigo in OKLCH with fixed-lightness ramps, a tight 4-step surface stack with 1px
hairline borders as the primary depth cue, a **separate colorblind-safe dataviz palette**
(Viridis sequential / diverging blue↔orange centered at 0) distinct from brand hues, modern
grotesque UI type with a real mono for every numeric (tabular figures), two-radius shape system,
Apple-Liquid-Glass vibrancy for **transient** surfaces only (never data planes), physics-based
value-flash gated on `prefers-reduced-motion`. Validated components promote into the `gui/`
Storybook as reusable primitives.

## No-drift anchoring (neurosymbolic references)

Each of the 16 mockup surfaces + the loop spine is stored as a lodestar graph-anchored
`decision` claim linking the surface → the crate/proto/fn it realizes → the workflow + persona
it serves → its honest LIVE/TARGET state (`knowledge_get`; see the `single-front-end-experience`
deliverable). The design becomes the verified, evolvable source of truth — clean code, no
legacy, no drift ([[lodestar-lifecycle-substrate]]).

## Alternatives rejected

- **FX-silo + per-class overlays (status quo).** Rejected: it is the replication defect itself
  (4× blotter, twin FX/FI risk stacks, FX-first state). The engine is already asset-class-
  parametric; the client must be one component taking `(desk, book, underlier, assetClass)`,
  not a per-class fork.
- **A separate FI/rates app or stack.** Rejected: violates the single-front-end and API-first
  parity guardrails; rates positions must roll up in the firm-wide cube and dispatch through
  `price_instrument`. FI is a peer asset class, not a second product.
- **15 distinct, persistent, hard-mounted screens (one per capability).** Rejected: it is a
  switchboard, not a workflow — it blocks cross-capability drill and cross-asset netting (the
  actual differentiators). Replaced by the composable loop + scope bus + saved perspectives.
- **Buy-side execution framing (Buy/Sell, taker axe-lift, trading-P&L/VaR-to-trade).** Rejected:
  no order engine exists; the persona is sell-side make/take/sales-trade. The prior `00-shell`
  over-indexed on taker affordances and was reframed.
- **Hiding the unwired legs / presenting the whole loop as done.** Rejected: dishonest and
  drift-prone. The LIVE/TARGET convention keeps the front-end truthful to the code and turns the
  gaps into a visible, verifiable roadmap.
- **Brand-hue quantitative heatmaps.** Rejected: coral/indigo are chrome only; quantitative
  scales use a separate colorblind-safe dataviz contract (the correct fix for the loud heatmap).

## Consequences

- The client contract collapses to ONE shared TS package consumed by GUI + Excel (kills the
  hand-mirrored `excel/src/contract` drift and the GUI's second offline pricer).
- New backend seams the design commits to (clean, unversioned): XVA wire surface (⟂ D-xva),
  vendor-feed/market-data-admin service, contribution/spread-model admin service, auto-quote
  engine, ReportingService (greenfield), observability/metrics wire surface, model-management
  RPCs, crypto strike-axis surface field.
- Each surface's LIVE/TARGET badge is a standing invariant: when its anchored code changes, the
  backing claim goes stale and must be re-verified — the design cannot silently assert a
  TARGET as LIVE.
