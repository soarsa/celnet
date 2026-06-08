# Celnet — GUI Experience Design (the world-class redesign)

> The single governing GUI-experience design that takes the React/WebGPU trader GUI
> (`gui/src/*`, over the ONE `celnet-proto` contract) from "structurally ahead of the
> FX-options field" to **best-in-class across every axis** — clean, intuitive, fully
> cohesive (one product, zero bolted-on feel), covering **ALL** capabilities the engine
> already ships, and seamed for the **multi-asset-class** scope (`MASTER-EVOLUTION-PROGRAM.md`
> §1) — out-functioning **SynOption/Optimus, Bloomberg OVML, Fenics Direct/kACE, 360T
> Bridge/EMS, TradingView, and the multi-asset OMS/EMS frontier (Quod/LSEG TORA)**.
>
> **Method.** This is the chief-GUI-architect synthesis of four deep read-only research /
> critique lenses (COMPETITOR-GUI deep research; TRADING-UX + DESIGN-SYSTEM SOTA;
> CURRENT-CELNET-GUI critique; INFORMATION-ARCHITECTURE + experience design). Every
> redesign action is grounded in a cited competitor/SOTA basis **and** verified against the
> read code. It is the experience-layer companion to `EXPERIENCE-ARCHITECTURE.md` (the IA
> contract) and `MASTER-EVOLUTION-PROGRAM.md` (the cross-asset program); it does not
> duplicate them — it operationalises their GUI commitments into dependency-ordered,
> fully-verified evolution waves that fold into `WORLD-CLASS-BACKLOG.md` and the convergence
> loop (§6 of the master program).
>
> **Honesty contract (CLAUDE.md rules 2/5).** Tags as in `EXPERIENCE-ARCHITECTURE.md`:
> **[built]** verified in code this session; **[proposal]** design against a partly-built
> substrate; **[gap]** needs a feed/crate that does not exist; **[ENV]** deploy-bound, never
> claimed in-repo. No fabricated numbers, ever. Empty-states say "—"; provenance is always
> on the face. The lived honest-data discipline (`EXPERIENCE-ARCHITECTURE.md` §1.4) is a
> load-bearing asset and is **systematised**, never lost, by this redesign.

---

## 1. Design thesis

Celnet already combines three things no FX-options incumbent does: a **streaming-first RFS
blotter as the resting state** (`StreamWorkspace.tsx`) rather than the RFQ-per-click cadence
of SynOption/360T/OVML; a **single analytics-AND-executable Ticket** carrying the full 21-
structure catalogue with a live two-way + 14-Greek face and same-Instrument promotion into
blotter/risk (`TicketWorkspace.tsx`); and **own-the-calibration-engine surface provenance**
(`SurfaceWorkspace.tsx` shows the typed model family + arb report per mark) that LP-
aggregating front-ends (Fenics/kACE, 360T) structurally cannot show. On top sits a real
position-fact-cube IA, an OKLCH semantic design system, a ⌘K spine, a dependency-free
virtualiser, and a measured-P99 status ribbon. That is a genuine lead — but it is not yet
*one cohesive world-class product*, and large parts of the shipped engine are invisible.

**The thesis: out-function every competitor by being the ONLY platform that unifies, in one
keyboard-first cohesive cube, the entire derivatives lifecycle across an expanding multi-
asset universe — with calibration provenance and honest data on the face at every level.**

We win each competitor on its own ground *and* on the seam between grounds:

- **vs SynOption/Optimus** — match the regulated multi-dealer RFQ + portfolio/lifecycle
  module, then exceed it by making lifecycle a *drill of the same cube* (not a bolt-on
  tool), keeping streaming-first as the resting state with RFQ-to-selected-LPs as a
  one-key escalation, and rendering the competing-quote **ladder** SynOption only counts.
- **vs Fenics/kACE** — match 300+ pair / 27-metal breadth and multi-source blending, then
  exceed by exposing *which source/calibration produced each mark* (transparent vs opaque
  composite) and showing broker-BF vs calibrated smile-strangle where they diverge.
- **vs 360T Bridge/EMS** — match deep RFS customisation + strategy templates + per-LP
  routing, then exceed by making a saved view a *shareable URL-encoded (scope × view ×
  analytics)* state — reproducible application states, not just per-user chrome.
- **vs Bloomberg OVML/TOMS** — match multi-leg structuring + chat-embedded quick-pricing
  tails + TOMS lifecycle, then exceed by keeping structuring ON the same card that streams,
  trades, risks, and books — and by being the inverse of "famously unintuitive": ⌘K spine,
  progressive disclosure, measured time-to-first-quote.
- **vs TradingView** — match the measured design-as-product gains (progressive disclosure,
  dual by-strike/by-expiry chains), then exceed by making every chain cell a *live two-way
  you can hit* that *builds structures*, not a static quote display.
- **vs the multi-asset frontier (Quod/LSEG TORA)** — match "one data model, no screen-
  switching" via the position-fact cube, then exceed by carrying FX-options-grade depth
  (full smile provenance, exotics, vanna-volga risk) as the per-asset analytics while the
  cube unifies risk *across* assets. Depth AND unification — TORA has unification without
  options depth; SynOption has options depth without unification.

The three non-negotiables that make it feel like **one product**: (1) a single
**Scope × View × Analytics** navigation grammar realised as shared primitives, not five
bespoke chrome dialects; (2) an **asset-class-agnostic substrate** so the multi-asset future
is a data/registry change, not a rewrite; (3) **honesty + provenance + keyboard-first**
extracted into enforced shared components so cohesion *scales* as scope expands.

---

## 2. Target Information Architecture

The IA is the `EXPERIENCE-ARCHITECTURE.md` §2 three-axis model — **finished, generalised to
multi-asset, and made shared**. Today only ~half is built; the redesign completes it.

### 2.1 The substrate: an asset-class-agnostic position-fact cube
Generalise the leaf of the cube from `ccy-pair` to **`(asset-class, underlier)`**, FX being
the first fully-built class (mirrors `MASTER-EVOLUTION-PROGRAM.md` §2 `Underlying`). In the
GUI this means replacing the single global `app.pairCtx`/`setPair(CcyPair)`
(`AppContext.tsx:88,267`) with `app.market`/`setUnderlier(Underlier)` where `Underlier` is a
tagged union (`{class:'FX',pair} | {class:'Metal',…} | {class:'Crypto',…} | …`), and the FX-
only universe taxonomy (`lib/universe.ts` major/cross/em) with a **pluggable per-asset-class
taxonomy provider**. Carry the asset-class dimension explicitly now even while it resolves
to FX-only — the exact "carry the context now" discipline already used for entitlement scope
(`EXPERIENCE-ARCHITECTURE.md` §1.3). This is the load-bearing change; without it the rail,
navigator, Ticket, and Cube are all `CcyPair`-shaped and the multi-asset scope is bolted-on.

### 2.2 The three orthogonal axes (each owned by exactly one surface)
- **Axis A — VIEW (left rail): identity + verbs.** Single Celer logo (rail header, the only
  logo). Workspaces as a small fixed list, now **eight first-class views**: Ticket · Stream ·
  Surface · **Cube** · Risk · **Lifecycle** · Book · **Universe** (⌘1..n, data-driven not
  hardcoded `⌘1-5` as today at `Shell.tsx:59`). Cube and Lifecycle are *promoted* (today
  Cube is a sub-toggle inside Surface; Lifecycle does not exist). Glyph table is one-glyph-
  one-meaning: **Book → ▤** (frees Σ for sum/ladder exclusively — fixes the documented-but-
  unfixed `Shell.tsx:41` Σ collision), Cube → ▦, Lifecycle → a distinct timeline glyph.
- **Axis B — SCOPE (toolbar): what slice of the firm.** The **single** scope control — a
  breadcrumb-driven group-by picker over `asset-class → firm → legal-entity → location →
  desk → trader → book → underlier` that drills **both up and down** and *pins* a dimension
  value (today `ScopeBreadcrumb` only drills up and `groupBy` is hardwired `'none'` at
  `AppContext.tsx:289`). The four redundant pair affordances (PairMenu + "Pairs" button +
  ⌘K pair list + PairStrip + UniverseNavigator overlay, all in `Shell.tsx` TitleBar) collapse
  into ONE scope affordance whose terminal case is underlier selection; the Universe
  navigator becomes the **leaf drill** of the breadcrumb (a first-class view, not an
  overlay). The toolbar also carries the **global lenses** every cube query is parameterised
  by: surface lens (live / official-IPV-pinned / historical `surface_version`), session /
  follow-the-sun cut, point-in-time **replay**, and reporting **numeraire**.
- **Axis C — ANALYTICS (a per-workspace inspector strip): with what.** One shared
  `<InspectorStrip>` docked in every Panel header (today analytics config is ad-hoc per
  lane: Risk has its own AxisBar, Surface inline model chips, Stream footer trend chips,
  Book has none). Declarative segments — `model | measures/greeks | axes | trend | columns |
  density | saved-view` — each lane populates only the relevant ones, with the provenance
  line always rendered. The **model segment is registry-driven** off the plugin
  `ModelRegistry`, so Tier-0/Tier-2 SDK models appear with zero special-casing
  (`EXPERIENCE-ARCHITECTURE.md` §5).

### 2.3 The cohesive shell
A **composable layout layer** above the rail: the persistent-mount pane model
(`Shell.tsx:227`) gains tiling/split (Stream | Risk side-by-side), tear-off-to-window for
multi-monitor (BroadcastChannel-synced AppContext), a **density** axis (compact/comfortable
via a `data-density` attribute alongside the existing `data-appearance`/`data-contrast` at
`appearance.ts:40`), a collapsible rail, and breakpoint adaptation so the fixed
`72px 1fr` / `overflow:hidden` shell (`Shell.module.css`) no longer clips. **Saved views**
make any `(scope, view-arrangement, analytics)` triple a named, bookmarkable, shareable,
URL-encoded state — realising the `EXPERIENCE-ARCHITECTURE.md` §1.2 promise. All grids
(Stream, Book, Cube, Risk) render through ONE accessible, virtualised, groupable
`<DataGrid>`/`<CubeGrid>` primitive (role=grid + roving-tabindex + column virtualisation +
tick-coalescing to a frame budget), so the cube model reads identically end-to-end and the
current `StreamWorkspace.tsx:699` ARIA-grid opt-out is *reversed*, not worked around.

---

## 3. Per-area redesign (each grounded in research)

| # | Area | Redesign | Research basis |
|---|------|----------|----------------|
| 1 | **Navigation / Scope** | Finish the three-axis model: single breadcrumb scope control (drill up+down, pin dimensions, group-by) absorbing the 4 redundant pair affordances; Universe + Cube + Lifecycle promoted to first-class rail views; data-driven ⌘N (not hardcoded ⌘1-5). | EXPERIENCE-ARCHITECTURE §2 (half-built); IA lens P0 (single global pairCtx + half-built nav); Fenics/Caplin configurable workspaces. |
| 2 | **Ticket / Structuring** | Decompose the 3606-line monolith into a **ProductSpec registry** (`gui/src/products/*`, one module per family: `InputBlock` + `toInstrument` + defaults + allowed models + asset-class), a grouped+searchable structure **gallery** (not a flat 21-item `<select>`), a live payoff-at-expiry mini-chart, intent templates ("zero-cost collar"), composable multi-leg leg-builder with a running net-premium/net-Greek strip, and progressive disclosure (raw MC/PDE estimator controls behind "Advanced"). | UX-SOTA P1 (3606-line monolith, no composable system); CRITIQUE P0 (registry, multi-asset scaling wall); IA P1; OVML strategy canvas + 360T templates; TradingView progressive disclosure (+26% setup). |
| 3 | **Stream / RFS blotter** | Migrate onto `<DataGrid>` (column virtualisation + tick-coalescing to frame budget + ARIA grid + roving-tabindex); per-cell freshness/age cue + aria-live announcements; per-row LP-competition depth (stack each LP two-way via `quotedBy`/AttributionRecord); explicit "RFQ selected" one-key escalation; Stream-row → Ticket (load) and → Risk (analyse) actions completing the bidirectional same-Instrument web. | UX-SOTA P0 (virtualization fatal at scale; ARIA opt-out; staleness/live-region); COMPETITOR P1 (per-LP routing 360T/SynOption); CRITIQUE P2 (one-directional selection). |
| 4 | **Surface / marking** | Re-layout per spec: editable delta×tenor **marking grid PRIMARY**; ATM/RR/BF **term-structure curves** second; **3D mesh demoted** to a collapsible QC tab (real WGSL pipeline behind the `SurfaceMesh.tsx:11` seam); eSSVI skew-term-structure inspector (ρ(θ)/ψ by tenor) when EXTENDED_SURFACE active; surface **role** chip (working/official/IPV); consensus/history Δ-ghost overlay with honest empty-state until the composite feed lands; broker-BF vs calibrated smile-strangle side-by-side on divergence. | SURFACE-WORKFLOW §4 + EXPERIENCE-ARCHITECTURE §6 (mesh demotion, term curves); COMPETITOR P1 (Fenics/Totem composite, opaque); CRITIQUE P1 (eSSVI invisible, no role/compare). |
| 5 | **Cube (vol heatmap)** | Promote to a first-class workspace (own glyph + ⌘N), respecting the Scope breadcrumb (region / G10-vs-EM / liquidity-tier / deliverable-vs-NDF / metals groupings); full documented 19-pair universe + registry seam surfaced (NDF "cash-settled @ fixing" + metals settlement chips); GPU-instanced cell grid on the SurfaceMesh WGSL substrate; per-mark source/calibration provenance on each cell. | COMPETITOR P0 (Fenics/kACE breadth + multi-source, opaque composite) + P1 (Cube buried); UX-SOTA P1 (WebGPU instanced heatmap); IA P1; EXPERIENCE-ARCHITECTURE §9.8. |
| 6 | **Risk + Capital** | 3D P&L scenario surface (reuse SurfaceMesh substrate) with user-editable shock ranges/steps (not fixed AXIS_SPECS) + a time-slider animating theta-decay; vanna/volga ladder (today only vega); **surface the hidden engine catalogue**: VaR/ES metric tabs (request `varAlpha`/`varSpotShocks` the server honours), FRTB-SA SbM capital + RRAO panel, XVA/CVA disclosure, additive(instant)/non-additive(recompute) cadence badge. | COMPETITOR P1 (kACE 3D scenario); CRITIQUE P0 (XVA/FRTB/VaR-ES hidden) + P1 (no vanna/volga ladder, no inspector); EXPERIENCE-ARCHITECTURE §5/§9.3. |
| 7 | **Book** | Migrate to virtualised `<CubeGrid>` (firm scale); choosable group-by dimension driven from the SAME toolbar scope path (today derived, not chosen); FRTB-SA capital + VaR/ES columns; Book → Risk drill (built) preserved; one consistent table grammar shared with Stream/Cube. | CRITIQUE P0 (scope can't drill down; capital hidden); IA P1 (unify Stream/Book/Risk on one CubeGrid); RISK-HIERARCHY §6. |
| 8 | **Lifecycle (NEW)** | A 6th-domain workspace: expiry/fixing/barrier calendar timeline + per-position event queue (upcoming fixings, **live barrier-proximity** off the streaming spot vs the Instrument barrier, exercise/expiry decisions), as a **drill of the same RiskService positions cube** — actionable in-place at the keyboard-first cadence. | COMPETITOR P0 (SynOption Optimus portfolio/lifecycle module — the single largest coverage gap; Celnet prices but cannot manage TARF/accumulator/Asian/barrier post-trade); Bloomberg TOMS. |
| 9 | **RFQ competing-quote ladder** | An RFQ mode (alongside streaming-first) rendering the stack of LP quotes ranked best-first, each with maker attribution + validity ring + click-to-hit; the "LPs in competition" count becomes ladder depth. GUI seam built now; per-LP wire payload coordinated with `celnet-proto` ([gap] backend dep; design lands visible). | COMPETITOR P1 + CRITIQUE P1 (SynOption Optimus core multi-dealer workflow; Celnet flattens to one two-way); MASTER-EVOLUTION-PROGRAM §4 W4 multi-dealer RFQ. |
| 10 | **Design system** | Density-mode token axis (compact/comfortable) + polished light/high-contrast for IPV/print; shared primitives (`<DataGrid>`, `<CubeGrid>`, `<InspectorStrip>`, `<LegBuilder>`/`<FieldGroup>`, `<Provenance>`, `<EmptyValue>`/`<StdError>`, `<PriceTile>`, `<LastLookRing>`); a documented, versioned token+component contract (`DESIGN-SYSTEM.md`, Style-Dictionary-able source-of-truth) so cohesion scales and SDK/Excel/native clients can later share tokens. | UX-SOTA P0 (density axis) + P2/P3 (saved-views, inspector strip, documented kit); SAS multidimensional tokens; design-token architecture consensus. |
| 11 | **Keyboard grammar** | Promote `lib/shortcuts.ts` to the actual binding registry the Shell reads (today bindings are hardcoded in the Shell handler); stable command-ID→keybinding map; ⌘K as the universal index of EVERY command; real ⌘P scope/underlier switcher distinct from ⌘K; per-view contextual bindings registered on focus; `?` cheatsheet generated FROM the registry (always accurate); collision-free as views grow. | IA P2 (ad-hoc, capped ⌘1-5, ⌘P aliases ⌘K, shortcuts.ts only documents); EXPERIENCE-ARCHITECTURE §1.6. |
| 12 | **Multi-asset seam** | Asset-class dimension threaded through AppContext (`Underlier` union), navigators (asset-class → market → instrument), the Ticket structure gallery (grouped by asset class), the Cube (asset-class-agnostic axes), and the universe taxonomy (pluggable provider) — even while FX is the only populated class, so crypto/metals/equity/rates arrive as a data change. | IA P0 + CRITIQUE P1 (entire GUI FX-pair-hardcoded); MASTER-EVOLUTION-PROGRAM §1/§2/§4 (Underlying, multi-asset clients). |
| 13 | **Honest-data systematisation** | Extract `<Provenance>` / `<EmptyValue>` / `<StdError>` primitives so every product block, inspector strip, and multi-asset view inherits the empty-state-says-"—" + provenance-on-face discipline automatically; make it a checklist item in the ProductSpec registry contract. | CRITIQUE P2 (turn the cultural strength into a structural one); EXPERIENCE-ARCHITECTURE §1.4. |

---

## 4. GUI evolution waves (dependency-ordered)

Each wave: disjoint where possible; grounded in a deep-research basis; surfaces named
capabilities (full + multi-asset scope); **fully verified** — vitest unit/component +
Playwright real-edge e2e (the `gui/e2e/demoEdge.ts` boots a real `celnet-server` edge) + axe
a11y (`@axe-core/playwright`, already wired in `e2e/a11y.e2e.ts`) + cross-client parity
(the same capability reachable from SDK/Excel/CLI, gated against the golden-vector corpus per
`MASTER-EVOLUTION-PROGRAM.md` §4). Waves fold into `WORLD-CLASS-BACKLOG.md` and are driven by
the §6 convergence loop. **GW1 is the load-bearing foundation; later waves depend on it for
cohesion.** GW6 (multi-asset GUI) is gated on the master program's W1 core-contract wave.

- **GW0 — Design-system + a11y foundation.** Density token axis; shared primitives
  (`<Provenance>`/`<EmptyValue>`/`<StdError>`, `<InspectorStrip>` shell, `<DataGrid>` with
  role=grid + roving-tabindex + column virtualisation + tick-coalescing); `DESIGN-SYSTEM.md`.
  *Surfaces:* the cohesion + scale + accessibility substrate every later wave composes from.
  *Verify:* vitest for the virtualiser/grid keyboard model + token cascade; axe must pass
  with role=grid claimed (reversing the opt-out); Playwright keyboard cell-nav e2e.
- **GW1 — Three-axis navigation + cohesive shell.** Single breadcrumb scope control (drill
  up+down, pin, group-by, global lenses) absorbing the 4 redundant pair affordances;
  data-driven rail; Σ→▤ glyph fix; `lib/shortcuts.ts` as the real registry; composable
  layout layer (tiling/tear-off/collapsible rail/breakpoints) + saved-views (URL+localStorage).
  *Surfaces:* the "one product" feel; reproducible shareable application states (exceeds 360T).
  *Verify:* vitest scope-state/saved-view serialisation; Playwright e2e drill-down across
  Stream/Book/Risk over one path + recall a saved view + ⌘K command index; axe on new chrome.
- **GW2 — Ticket → ProductSpec registry + structuring gallery.** Decompose the 3606-line
  monolith into `gui/src/products/*`; grouped searchable gallery; payoff mini-chart; intent
  templates; multi-leg leg-builder + net-Greek strip; progressive disclosure; honest-data
  primitives baked into the registry contract.
  *Surfaces:* clean scalable structuring for the full 21-structure catalogue + the multi-
  asset future (a new product = a registry entry). *Verify:* vitest per-ProductSpec
  `toInstrument` round-trip; Playwright e2e price every family through the gallery vs the
  golden vectors; cross-client parity (each priced family reachable from SDK/Excel/CLI); axe.
- **GW3 — Stream blotter at IB scale.** `<DataGrid>` migration (column virtualisation + tick-
  coalescing); per-cell freshness + aria-live; LP-competition depth; RFQ-selected escalation;
  Stream-row → Ticket/Risk. *Surfaces:* IB-cardinality streaming, accessible, with multi-
  dealer competition visible. *Verify:* vitest tick-coalescing-to-frame-budget + conflation
  accounting; Playwright e2e high-rate stream stays within the measured render-P99 + cell
  keyboard-nav + click-to-trade still hits; axe; parity (stream reachable from SDK/CLI).
- **GW4 — Surface + Cube promotion.** Marking grid PRIMARY + term-structure curves + mesh
  demoted (real WGSL); eSSVI skew inspector; role chip + consensus Δ-ghost (honest empty);
  broker-BF vs smile-strangle; Cube promoted to first-class + GPU-instanced + full 19-pair
  universe + per-mark provenance. *Surfaces:* best-in-class vol marking + whole-universe scan
  that EXCEEDS Fenics/kACE on provenance. *Verify:* vitest grid edit/commit-revert + ramp;
  Playwright e2e mark → version-pin → drill cell → Surface; axe; parity (mark surface == SDK/
  Excel `CELNET.MARKSURFACE` model arg + golden vectors).
- **GW5 — Risk/Capital catalogue + Lifecycle (NEW).** 3D scenario surface + editable shocks +
  time-slider + vanna/volga ladder; VaR/ES + FRTB-SA capital + XVA/CVA panels + cadence
  badge; Book on `<CubeGrid>` with chosen group-by + capital columns; **Lifecycle workspace**
  (expiry/fixing/barrier calendar + live barrier-proximity + in-place exercise/expiry as a
  cube drill). *Surfaces:* the hidden engine catalogue (FRTB/XVA/VaR-ES/AAD cadence) + the
  full post-trade lifecycle SynOption's module targets. *Verify:* vitest scenario-grid +
  honest-empty ladders + lifecycle event ordering; Playwright e2e VaR/ES + SbM capital
  request returns + barrier-proximity ribbon fires off live spot; axe; parity (risk
  aggregate/drill/limits == SDK/Excel/CLI; lifecycle == RiskService positions).
- **GW6 — Multi-asset GUI (gated on master W1).** `Underlier` union through AppContext;
  asset-class-aware Universe navigator (AssetClass → class-native buckets); structure gallery
  + Cube grouped by asset class; surface-family switch (FX RR/BF vs strike/moneyness) bound
  to the underlying's class; RFQ competing-quote ladder visible. *Surfaces:* the cross-asset
  reach (crypto/metals/equity) that exceeds SynOption's asset-class coverage — as a data/
  registry change, not a rewrite. *Verify:* vitest taxonomy provider + Underlier routing;
  Playwright e2e price ≥1 crypto vanilla + ≥1 metal through the GUI vs golden vectors; axe;
  cross-client parity row per new (asset-class, product) before "done".
- **GW7 — Convergence rounds (recurring).** Feed the GUI/UX critique lens (master loop §6
  lens 6) until two consecutive dry rounds; dedup against `WORLD-CLASS-BACKLOG.md`.

---

## 5. Cohesion guarantees (no legacy / no bolted-on feel)

1. **One navigation grammar** — every workspace reads the same Scope path, the same
   `<InspectorStrip>` segments, the same `<DataGrid>` keyboard model. Learn it once.
2. **One substrate** — every number is a reduction over the same position-fact cube; Book/
   Risk/Lifecycle/Cube are the *same cube at different zooms* (reconciliation guaranteed
   server-side). Selection is the same Instrument everywhere, bidirectionally.
3. **One design system** — documented, versioned, density/appearance/contrast-aware; new
   asset classes and SDK analytics compose from the same primitives and appear native.
4. **Zero legacy** (CLAUDE.md rules 9/10) — the monolith Ticket, the redundant pair
   affordances, the ad-hoc per-lane analytics chrome, and the ARIA opt-out are *deleted and
   replaced*, not paralleled. One clean current contract, GUI included.
5. **Honest by construction** — provenance/empty-state/std-error are shared primitives and a
   registry-contract checklist item, so honesty is enforced structurally, not by convention.

---

## 6. Verification contract (every wave)

- **vitest** (`gui/test/*`, jsdom): unit/component over real modules — registry round-trips,
  grid keyboard model, virtualiser/tick-coalescing, scope/saved-view serialisation, honest
  empty-states.
- **Playwright real-edge e2e** (`gui/e2e/*`, boots a real `celnet-server` via `demoEdge.ts`):
  the actual workflows — price/mark/stream/risk/lifecycle/RFQ — asserting GUI == server
  (==golden to 1e-12 where numeric), version-pin, forged-token reject.
- **axe a11y** (`@axe-core/playwright`, `e2e/a11y.e2e.ts`): zero violations on every new
  surface, *with* role=grid claimed (the opt-out reversed by the real grid pattern).
- **cross-client parity**: each capability reachable from SDK / Excel / CLI and gated against
  the language-neutral golden-vector corpus; `CLIENT-PARITY-MATRIX.md` generated FROM the
  passing harness (master §4). A capability is "done" only with a parity row per client.
- **Push milestone** (master §7): full `just check` prints literal `All gates passed.`; the
  5-client conformance harness green against a fresh edge; all suites pass (Rust nextest +
  GUI vitest + GUI Playwright real-edge e2e + Excel real-edge e2e); docs reconciled +
  codebase-memory re-indexed.

---

## 7. Honest boundary (in-repo vs deploy/ENV)

In-repo, the GUI proves: workflows, keyboard model, accessibility, virtualisation/coalescing
behaviour, honest-empty/provenance discipline, GUI==server numeric parity, and a host-local
render-P99 ratio. **[ENV/deploy-bound, never claimed in-repo]:** live multi-dealer LP-panel /
venue connectivity & regulated (MAS-RMO) status, live crypto/metal fixing VALUES, cross-host
wire p99, live JVM Celer estate lifecycle. The RFQ ladder (GW6/§3.9) and the consensus/
history overlay (GW4/§3.4) render their seams + honest empty-states in-repo; the live feed
VALUES behind them are ENV. This is the existing honest-boundary discipline
(`MASTER-EVOLUTION-PROGRAM.md` §0), unchanged.
