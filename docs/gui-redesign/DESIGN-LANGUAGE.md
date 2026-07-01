# Celnet Design Language — July-2026 SOTA (the visual upgrade spec)

From `RESEARCH-sota-design-2026.md` (cited). Governs the celer.css rewrite + every screen + the
Storybook components. Principle: **premium restraint** — high contrast, generous rhythm, monochrome
base + one rationed accent, sharp grotesque type, density via a tight elevation stack (not cramming).

## 1. Color
- **Re-found coral `#ff7357` + indigo in OKLCH**; generate fixed-lightness ramps (perceptually even
  states, one source for light/dark). Keep the Celer identity; modernize execution.
- **Tight 4-step surface stack** (canvas → 3 elevated), small lightness increments; **1px low-alpha
  (5–8% white) hairline borders are the PRIMARY depth cue**, shadows minimal/for floats only.
- **Ration the accent:** coral = the ONE primary action / live highlight per view; indigo =
  structural/secondary/links; everything else neutral gray. (Current screens over-use coral.)
- **Separate dataviz color contract (NOT brand hues):** sequential = Viridis-family; diverging =
  blue↔orange / RdBu **centered at 0** for P&L/scenario/RR; colorblind-safe. Brand hues are for
  chrome only, never quantitative scales. (This is the correct fix for the loud heatmap.)
- Semantic bid/offer stay green/red (conventional); value-flash uses coral↔teal, not red↔green.

## 2. Type
- **UI: a modern grotesque with tabular figures** — Space Grotesk / Host Grotesk (headings, labels).
- **Numerics: a real mono** — JetBrains Mono / Geist Mono / Berkeley — every price, Greek, ID, %;
  **tabular lining figures** everywhere numbers align.
- **Brand mark only: Anaheim** (wordmark/display) — not general UI.
- Tighten the type scale so hierarchy reads (display/title/headline distinct, not 16↔18 mush).

## 3. Shape / material / motion
- **Two-radius system:** 4–6px controls, 10–12px cards.
- **Glass/vibrancy (Apple Liquid-Glass-inspired) for TRANSIENT surfaces only** — command palette,
  RFQ modal, popovers — **never** data grids/planes.
- **Motion:** physics-based, purposeful; trading micro-interactions = value-flash + number-roll on
  tick; all gated on `prefers-reduced-motion`.

## 4. Interaction
- Persona **"lenses"** (Trader / Structurer / Quant-Risk / Sales / Ops) + **progressive-disclosure**
  drill-down; **linked-view brushing** across surface ↔ smile ↔ blotter ↔ scenario.
- Explicit **asset-class → underlier → product → quant-details** selection, adapting per class.

## 5. Chart SOTA targets (ours are currently basic — this is the bar)
- **Vol surface** — WebGL/three.js shaded terrain mesh (Plotly `surface` as quick start), Viridis
  elevation, rotate/zoom, **three linked slices** (skew / smile / term), implied-vs-model diff,
  arbitrage-violation cells flagged.
- **Smile** — IV vs log-moneyness/delta, market marks + fitted curve + **bid/ask band**, ATM & 25Δ/10Δ
  RR/BF markers, expiry-family overlay.
- **Payoff** — bold net line + **dotted MTM/today curve** + light dashed per-leg, shaded profit/loss,
  strike kinks, **auto-named strategy**, instant re-render.
- **Scenario heatmap** — spot×vol grid, **diverging blue-orange centered at 0**, click-to-drill to
  contributing trades; optional 3D companion.
- **XVA exposure** — EE / EPE / **PFE(95%)** fan + mirrored ENE, stacked CVA/DVA/FVA/MVA/KVA bar,
  drill portfolio→counterparty→trade; 3D PFE quantile surface.
- **Blotter** — virtualized (AG-Grid-class, 100k+ rows), tabular mono, per-cell value-flash,
  conditional heat shading, inline sparklines.

## 6. Charting libraries — DECISION (operator, 2026-07-01)
**Adopt open-source SOTA chart libraries** for the real `gui/` chart components, **overturning the gui
house rule 7 ("no chart library")** for viz specifically. Rationale: the operator chose richer, genuinely
interactive SOTA charts (rotate/zoom/brush/3D) over hand-rolled SVG. All chosen libs are
**OSS (MIT/Apache-2.0/BSD)** → compliant with the no-commercial-products guardrail; only the internal
rule-7 house rule is superseded (formal ADR to file in `docs/adr/`). Bundle/perf: lazy-load the heavy
3D/point-cloud libs per-surface; keep the zero-alloc hot core untouched (charts are client-only).
- **three.js** — the true interactive 3D vol surface (WebGL; the one place 3D earns it).
- **visx / D3** — bespoke 2D: payoff, XVA exposure fan, key-rate ladder, smile, curve.
- **ECharts** — heatmaps / scenario surface / big-data grids.
- **Lightweight Charts** — streaming 2D price/series.
- **AG-Grid (community, MIT)** — the virtualized blotter (or keep the existing WAI-ARIA `DataGrid` if it
  meets the density bar — evaluate before adding).
Every chart consumes the `--seq-*`/`--div-*` dataviz palette (NOT brand hues), stays deterministic/
testable (seeded data in stories), and honors `prefers-reduced-motion`.

## 7. Delivery
Mockups become **interactive/clickable** prototypes approximating these; validated components are then
promoted into the `gui/` **Storybook** as reusable, evolvable primitives (Button/Panel/DataGrid/
QuoteTile/SurfaceChart/SmileChart/PayoffChart/ScenarioHeatmap/XvaProfile/Blotter/CommandPalette),
each with the token contract above. The mockup `celer.css` is the staging ground for the token system
that lands in `gui/design-tokens.json` + `gui/src/design/tokens.css`.
