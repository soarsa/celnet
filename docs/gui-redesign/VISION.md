# Celnet Experience — Unified Cross-Asset Vision (north-star)

> Operator directives (2026-06-30): fully review + redesign the entire experience so it is clean,
> non-replicated, well-planned, and SOTA per our design system. **Out-style and out-function
> Synoption and Spectraxe** (incl. Spectraxe's axe-based contribution). Enable ALL capabilities
> intuitively in one **fully-reasoned cohesive vision**. A **single architecture** brings fixed
> income and every asset class into a consistent, clean model. Backed by a **perfect, single,
> evolved, intuitive API that enables cross-asset opportunities**, in the cleanest state-of-the-art
> trader experience **as of June 2026**. Our lodestar knowledge enables evolution, clean code,
> **no legacy, no drift**.

This is the spec the HTML mockups (first deliverable) implement, and the design that gets anchored
into lodestar as neurosymbolic references so it stays the living source of truth. It is a *draft*:
Phase-2 competitor/SOTA research (in flight) refines §6/§9; the invariants in §1–§3 are stable.

---

## 0.5 PERSONA & POSITIONING — CORRECTION (operator, 2026-07-01)

Celnet's front-ends (GUI + Excel) are **NOT a buy-side trading/execution surface.** The users
**price, administer, manage market-data feeds, and contribute/distribute prices** — this is a
**sell-side market-maker pricing & distribution platform positioned to replace Fenics and
Synoption**, not a taker's trading blotter. This retunes everything below (§1–§9 hold, but retargeted):

- **Personas:** a pricing/quant; a market-data & FEED operator; a price-CONTRIBUTION/distribution
  manager; a platform admin. Not a trader clicking buy/sell.
- **Remove execute-to-trade affordances** — no Buy/Sell, no taker "lift-the-axe", no trading-P&L
  blotter / VaR-to-trade. The prior `00-shell` mock over-indexed on these and is being re-framed.
- **The real pillars** (some were under-covered):
  - **Pricing workbench** — price instruments/structures, greeks, scenario/what-if, as a pricing tool.
  - **Vol surface** — mark / calibrate / version / PUBLISH (5 smile models, arb checks).
  - **Feed management (inbound)** — vendor MD sources, FIX/MD sessions, symbology/mapping,
    compositing/arbitration, staleness, feed health. **A core Fenics pillar — was missing.**
  - **Price contribution / distribution (outbound)** — what the desk streams/quotes to which
    clients/venues, **tiering, spread/skew rules, publication controls, contribution health**. The
    "axe" surface **flips to the SELL side**: the desk's OWN contributed axes/skew + inbound-RFQ
    **auto-quoting (maker)** — NOT lifting others' axes. **Core Synoption/Fenics pillar — was inverted.**
  - **Administration** — users, roles, entitlements (deny-wins), FIX connectivity, config, monitoring.
- **Still valid, retargeted:** cross-asset unification, the Celer design system, the command spine,
  and XVA-as-a-pricing-analytic. Competitive frame is **Fenics/Synoption (sell-side pricing +
  market-data + distribution)** — NOT SpectrAxe/OptAxe execution venues. Ground-truth re-inventory in
  flight; §7 mockup set + §3 are being reworked to the pricing/feed/contribution/admin jobs.

## 0.6 DESIGN MANDATE — raise the bar (operator, 2026-07-01)

Do **NOT** treat today's GUI as good or as the target — reimagine to SOTA. This governs all below.

- **All asset classes, integrated but separately licensed.** One experience; each class is
  **distinctly permissioned + commercially licensed**. The UI integrates what a firm holds and
  cleanly gates/omits the rest — *integrated feel, modular license*. Entitlement is a design
  primitive (present/gated/upsell), not an afterthought.
- **Investment-banking scale** for risk and data — millions of instruments/positions, real-time
  risk aggregation; density handled with discipline (virtualized, progressive, GPU where it earns
  it), **never cluttered**.
- **SOTA pricing & modelling are hero capabilities** — model choice/calibration (LSV/Heston/SABR/
  SSVI/eSSVI), surface marking, scenario — surfaced intuitively, never buried.
- **Ruthless minimalism** — integrated, intuitive, clean, **visually stunning**. No verbose
  descriptions, no unnecessary functionality. Every element earns its place; high signal, low chrome.

## 0.7 CAPABILITY SCOPE — curated; all must make sense (operator, 2026-07-01)

The integrated, per-class-licensed experience serves these coherent jobs — nothing superfluous:
- **Pricing & modelling** — price any instrument/structure; choose & calibrate models (LSV/Heston/
  SABR/SSVI/eSSVI); greeks; scenario / what-if. (hero)
- **Vol surface lifecycle** — mark / calibrate / version / publish; arbitrage checks.
- **Market-data & FIX (inbound)** — vendor feeds, FIX sessions, symbology/mapping, compositing/
  arbitration, staleness, feed health.
- **Contribution & distribution (outbound)** — stream/quote to client tiers & venues; spread/skew
  rules; publication controls; contribution health; **maker auto-quoting** of inbound RFQ. FIX out.
- **Risk at IB scale** — real-time cross-asset risk aggregation across desks/books; scenario; limits.
- **Desks & books** — org hierarchy; scope pricing/risk/contribution/reporting by **desk/book**; a
  first-class entitlement + aggregation boundary.
- **Permissioning & licensing** — roles/capabilities (deny-wins) **+ per-asset-class commercial
  license** (present / gated / upsell) as a design primitive.
- **Reporting** — valuation / risk / activity / contribution / feed & regulatory; scheduled + ad-hoc.
- **Administration & ops** — users, FIX connectivity, config, monitoring, telemetry (p50/p99).

Everything is **scoped by desk/book** and **gated by entitlement + license**. If a capability doesn't
serve one of these jobs, it doesn't ship. Delivery process: **loop + multi-agent**, with **adversarial
critique of the visuals AND end-to-end intuitivity** each round (visual agents render/critique; a UX
agent walks flows) — build → critique → fix until clean.

## 0.8 FULL REDESIGN v2 (operator, 2026-07-01) — July-2026 SOTA, integrated, multi-persona

- **The integrated market-making LOOP is the design spine** (the "link" between capabilities):
  `FEEDS → PRICING/MODEL → CONTRIBUTION (Celnet-as-LP make) ⇄ RFQ/LP-TAKE + SALES-TRADER →
  BOOKS/POSITIONS → RISK/XVA → OPS`, and **RISK feeds back into pricing/contribution skew** — one
  CLOSED loop, not distinct tools. Every surface is a lens on this loop.
- **Celnet is an LP that also TAKES prices from LPs, with SALES-TRADER on top:** *make* (price →
  contribute/stream), *take* (RFQ-panel aggregate other LPs → hedge/best-ex/composite), *sales-trade*
  (respond to client RFQs, tiers, client analytics) — in one flow.
- **Multi-persona = lenses on the loop:** quant · trader/market-maker · sales-trader · risk manager ·
  desk head. Each gets a default PERSPECTIVE (panel arrangement) over the shared loop; same data,
  different lens.
- **Asset-class → quant-detail selection must be explicit + guided:** asset class (FX/FI/Crypto/…) →
  underlier → product/structure → quant details (model · surface/curve · conventions · calibration) →
  price. Full FX + FI + Crypto depth.
- **Visual language upgraded SIGNIFICANTLY** to feel July-2026 SOTA (evolve color/type/depth/motion,
  keep Celer identity but modernize; the pricing tiles + charts must become genuinely state-of-the-art).
- **Interactive:** HTML mockups must be CLICKABLE prototypes; then promote validated components into
  the `gui/` **Storybook** as real, reusable, evolvable components.
- **Risk / XVA / books/positions** must be a stunning INTEGRATED experience (part of the loop), not
  separated screens.
- **Method:** comprehensively capture the ENTIRE-codebase knowledge via lodestar FIRST, then fully
  redesign; run as a **/loop** with the adversarial critique gate (verify against real competitor
  visuals). Charts are the bar.

## 1. Core thesis — one evolved API, thin parametric clients

The defect today is **replication born of asset-class silos**: two parallel FX-vs-FI stacks sharing
~0 components, a blotter built 4×, FX-first state with non-FX as a clearable overlay, and
capabilities the API can price but no client surfaces (XVA, crypto strike-axis, 21/24 cross-asset
exotic arms). See `docs/gui-redesign/DISCOVERY.md`.

The cure is a **single asset-class-parametric contract** as the one source of truth, consumed
*identically* by GUI, Excel, and SDK ([[api-first-client-parity]]):

- **One `Underlier`** — every tradable (FX pair, equity, commodity, crypto spot/perp/inverse, rates
  entity) is a peer instance of one type carrying its `AssetClass` + carry/settlement descriptor.
  Not "FX plus overlays."
- **One `Instrument`** — the 26-arm payoff `oneof` is class-orthogonal; a barrier is a barrier
  whether the underlier is EURUSD or BTCUSD, gated only by what the engine can price.
- **One analytics vocabulary** — `Price`, `Greeks` (the 14-field struct with a class-appropriate
  two-rho pair via `RateSensitivities`), `Surface`, `Scenario/Risk`, `RFQ`, `Stream`, `XVA`,
  `Blotter` — each a single verb parameterized by class, never re-specified per silo.

The clients never compute a capability the API doesn't expose (`docs/ROADMAP.md:409`), and never
re-implement one the API already owns. The GUI is a **projection** of the contract.

## 2. The unified architecture (three layers, one contract)

```
        ┌─────────────────────────── ONE canonical API (celnet.proto + server) ──────────────────────────┐
        │  Underlier · Instrument(26 arms) · Price/Greeks · Surface(5 smiles) · Scenario/Risk-cube ·       │
        │  RFQ(multi-dealer + AXES) · Stream · XVA(CVA/DVA/FVA) · Blotter · Auth/Entitlements               │
        └───────────────────────────────────────────────┬───────────────────────────────────────────────┘
                                    one shared, generated wire contract (no hand-mirrored drift)
        ┌───────────────────────────────────────────────┴───────────────────────────────────────────────┐
        │  Client contract/transport — a SINGLE shared TS package consumed by GUI + Excel (kills the       │
        │  hand-rolled excel/src/contract mirror and the GUI's second offline pricer as a divergence risk) │
        └───────────────────────────────────────────────┬───────────────────────────────────────────────┘
        ┌───────────────────────────────────────────────┴───────────────────────────────────────────────┐
        │  Asset-class-PARAMETRIC UI components — generalize the one clean pattern (GreeksStrip): every    │
        │  surface (Ticket, Structure, DealerPanel/Axes, Surface, Risk, XVA, Blotter, Stream) is ONE       │
        │  component taking (underlier, class) — never a per-class fork. Primitive/feature boundary added. │
        └───────────────────────────────────────────────────────────────────────────────────────────────┘
```

## 3. Information architecture — cohesive flow, not 15 silos

Replace the 3-domain × 15-persistent-workspace switchboard with a **composable drill-down grammar**
around a single spine:

**Universal Underlier Picker → Ticket → (Structure) → Price → RFQ/Axes → Risk/Scenario/XVA → Blotter**

- **One entry, class-agnostic.** A universal command palette + underlier picker where FX, equity,
  commodity, crypto, and rates are *peers*. Pick `BTC-28JUN` or `EURUSD` or `SOFR 5y` the same way.
- **The Universal Ticket** — one registry-driven ticket that adapts its InputBlock and available
  structures to the underlier's class (already partly true via `PRODUCT_REGISTRY`); rates FRA/IRS/
  bond become ticket products, not a separate stack.
- **Cross-capability continuity** — from any price you drill to its surface, its risk, its XVA, or
  send it to RFQ, without changing "workspace." Panels compose; state flows.
- **One blotter, one risk, one XVA** — asset-class columns/filters, not four implementations.
- **Perspectives** replace workspaces: a saved arrangement of panels (the good part of today's
  saved-views), not 15 hard-mounted routes.

## 4. Cross-asset opportunities (the differentiator)

A single contract + single blotter/risk make genuinely cross-asset workflows first-class — things
FX-only Synoption/Spectraxe structurally cannot do:

- **Cross-asset portfolio risk / scenario / XVA** in one grid (FX + crypto + equity + rates netted).
- **Cross-asset RFQ & axes** — request or watch axes across classes in one panel; relative-value
  and switch trades expressed natively.
- **Unified vol** — one surface tool across FX/metal/crypto/equity (surfacing the priced-but-hidden
  crypto strike-axis).
- **One position → many actions** — hedge, restructure, roll, XVA-price, or re-RFQ from a blotter row.

## 5. The 11 workflows → unified surfaces (from DISCOVERY §Workflows)

Price vanilla (all classes) · Structure multi-leg/exotic · Multi-dealer **RFQ + Axes** (unify FX &
rates) · Stream two-way quotes (extend cross-asset) · Vol surface + cube (5 smiles; add crypto axis)
· Blotter/positions (one class-parametric) · Portfolio risk + scenario P&L (unify) · **Portfolio
XVA** (new surface) · FI quoting/booking/curve (folded into the spine) · Admin/entitlements/FIX ·
Excel parity (shared contract). Each maps to a composable panel, not a silo.

## 6. Out-Synoption, out-Spectraxe — the axe model (refine with research)

- **Out-function:** breadth (all classes + exotics + rates + XVA in one flow) and cohesion
  (cross-capability drill, cross-asset netting) that single-purpose FX RFQ tools lack.
- **Out-style:** June-2026 SOTA — the mature OKLCH/Celer token system, MacOS-HIG vibrancy /
  "Liquid-Glass" depth, tabular-num density, disciplined live-price flash, keyboard-first.
- **Axe-based contribution (beat Spectraxe):** dealers contribute inventory-driven axes (directional
  interest + skew); buy-side sees live *axed* streaming prices ranked in the DealerPanel, can
  auto-RFQ/trade against them, with last-look. Generalize axes **across asset classes**, not FX-only
  — an axe board that spans the book. *(Exact interaction model pending the Spectraxe research pass.)*

## 7. API evolution targets (the deltas the vision requires)

The contract is already largely unified (26-arm oneof, generalized `RateSensitivities`, `CostOfCarry`
seam). To fully realize the vision, evolve — cleanly, no versioning ([[api-naming-and-evolution]]):

1. **XVA wire surface** — proto message + `handle_unary` arm (synergy with the **D-xva** cargo lane;
   coordinate the proto window).
2. **Cross-asset exotic coverage** — extend `price_cross_asset` beyond 3/24 arms where the carry seam
   allows (or surface honest capability gates where it doesn't).
3. **Crypto strike-axis surface** — the missing `quote_basis`/`StrikeQuoteSet` field + reachability.
4. **Axe-distribution messages** — dealer axe contribution + client axe board (new).
5. **Streaming for rates/cross-asset** — no FX-only streaming privilege.

## 8. Design language

Reuse, don't rebuild: `gui/design-tokens.json` (73 DTCG tokens) → `gui/src/design/tokens.css` OKLCH
custom props (brand coral `#ff7357`, accent indigo), CSS Modules, three orthogonal `data-*` axes
(appearance/contrast/density), aurora-wash, Liquid-Glass materials. Add the **primitive/feature
boundary** the flat components dir lacks. Anaheim for headings only; tabular nums for all numerics.

## 9. No-drift by construction (lodestar neurosymbolic anchoring)

Every IA decision, each mockup, and each unified-surface contract is stored as a lodestar
graph-anchored knowledge claim (`decision`/`spec:satisfies`) + an ADR, linking the symbolic artifact
(HTML mockup / component / proto message) to the capability and workflow it realizes. The design
becomes the verified, evolvable source of truth — clean code, no legacy, no drift
([[lodestar-lifecycle-substrate]]).

---

### Open questions for the research pass to settle
- Spectraxe's exact axe-board interaction + last-look UX to beat.
- Best unified multi-asset IA precedent (tiling vs. spine-with-drilldown vs. command-driven).
- Whether cross-asset structures (multi-class legs) are in scope for v1 mockups or a later evolution.
