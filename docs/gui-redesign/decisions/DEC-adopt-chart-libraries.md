# Decision — adopt OSS chart libraries (overturn gui rule-7 for viz)

Status: Accepted (operator, 2026-07-01). To promote to `docs/adr/` at landing. See DESIGN-LANGUAGE §6.

## Context
The gui house **rule 7 = "no chart library"** (everything hand-rolled SVG/Canvas). The July-2026 SOTA
design language (`DESIGN-LANGUAGE.md §5/§6`) targets genuinely interactive charts (rotate/zoom/brush,
true 3D). The operator chose the SOTA-library path.

## Decision
**Adopt open-source SOTA chart libraries for `gui/` viz, overturning rule-7 for charts specifically:**
- **three.js** — the true interactive 3D vol surface (WebGL).
- **visx / D3** — bespoke 2D: payoff, XVA exposure fan, key-rate ladder, smile, curve.
- **ECharts** — heatmaps / scenario surface / big-data grids.
- **lightweight-charts** — streaming 2D series.
All are **MIT/Apache-2.0/BSD** → compliant with the no-commercial-products guardrail; only the internal
rule-7 house rule is superseded.

## Consequences
- Charts consume the `--seq-*`/`--div-*` dataviz palette (never brand hues), stay deterministic/testable
  (seeded story data), honor `prefers-reduced-motion`.
- **Lazy-load** three.js/ECharts per-surface (bundle >500KB) — dynamic `import()`.
- The zero-alloc hot core is untouched (charts are client-only).
- Landed (foundation `029dd61`, components `cbe319e`): 7 gated + render-verified components.
