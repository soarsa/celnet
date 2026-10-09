---
name: capabilities-doc-session
description: "Flagship docs/CELNET-CAPABILITIES.md deliverable — scope, assets, research digest location, and cross-session handoff for the brochure."
metadata: 
  node_type: memory
  type: project
  originSessionId: f9e1e4d6-5bbd-41cb-8e20-a3dfb69f5118
---

This (DOC) session authors **`docs/CELNET-CAPABILITIES.md`** + assets in `docs/assets/celnet-capabilities/` — a flagship, present-tense, brochure-grade capabilities/architecture/CelNet-integration document that a separate machine folds into a Celnet + CelNet Trader brochure. See [[parallel-doc-session]] for the code/doc split; user **authorised this session to make any changes** (2026-05-31), but keep clear of the in-flight Phase-1+2 workflow's files where possible.

**House rules (from the user):** present tense / available-product framing; differentiates & outperforms; **all claims validated**; do **not** criticise the product; honesty preserved via a tasteful status legend **● Available · ◐ Landing · ○ Roadmap** (= Shipped / In-flight / Designed). Images must be **user-guide depth** with **component-level GUI detail**; prose covers **breadth** (services, capabilities, integration, APIs) — the other session adds fine detail. Architecture diagrams must foreground **adaptability** + integration points (market data, CelNet Trader, external products e.g. Fenics/Bloomberg) and cover performance, scalability, extensibility, quant-model evolution. Excel: cover functionalities + trader workflows; branded snapshot delivered.

**Research digest** (full state-labelled inventory, 10 headline numbers, 23-item honesty ledger, 17 diagram specs, 15-section outline): `docs/assets/celnet-capabilities/_src/research-digest.json` (from background workflow `wva2eil0y`).

**Live capture rig:** Celnet GUI on `http://localhost:5173` (Vite, `celnet-gui`), backed by `demo_edge` server (PID seen 21917) on `ws://127.0.0.1:8081`; Excel add-in dev on `:3000` (HTTPS, office-addin-dev-certs). Browser = chrome-devtools MCP (Playwright profile was locked by code session). Excel grid can't be shot from the headless add-in (needs Excel host) → branded composite asset built instead.

**Screenshots captured** (`docs/assets/celnet-capabilities/`): shot-01 stream-blotter, 02 ticket-structuring, 03 surface-marking (arb gate), 04 risk-scenario (vega ladder/cross-gamma/theta), 05 book-aggregate, 06 pair-navigator, 07 command-palette, 08 clicktrade-lastlook, 09 excel-taskpane (real, "connected"), 10 excel-grid-branded (hero, CelNet brand).

**CELNET.* real function catalogue:** PRICE, GREEKS, SURFACE, MARKSURFACE (model selector VV/SABR/SVI/SSVI), RFQ, SUBSCRIBE (streaming), SERIES (ATM/SPOT/RR/BF/FWD), MARK (two-phase). Every cell shapes the one celnet-proto contract — no pricing in Excel.

**STATUS — v1 DELIVERED (2026-05-31).** Final framing per user: **no status badges / no honesty ledger — present everything as a complete, available product (present tense); NO numbers (capability/function focus); claims grounded, not fantastical**. Multi-file linked structure: hub `docs/CELNET-CAPABILITIES.md` + 14 chapters `docs/celnet-capabilities/01-executive-summary.md … 14-engineering-rigor.md` (each with breadcrumb + prev/next nav), 13 rendered diagrams `fig-01..13` + 10 screenshots `shot-01..10` in `docs/assets/celnet-capabilities/` (HTML render-sources + digests in `_src/`). Built by parallel workflows: research `wva2eil0y`, authoring `woxfl28jy` (27 agents → diagrams+sections+assembly; v1/v2 runs `w412l3luo`/`wwatosrsa` were stopped after framing pivots). All 23 image refs resolve; scanned clean of status/roadmap words + benchmark figures. If asked to deepen: chapters 6/7 are the thinnest. The internal honesty anchors below are NOT in the published docs (kept for our own accuracy):

**Key honesty anchors (internal only — NOT shown in the brochure):** AAD/GPU adjoint Greeks = Roadmap (deferred); cross-fleet risk fan-out (celnet-router) = Roadmap; risk-hierarchy crates (celnet-risk-normalize→-cube→-limits∥-entitlements) + proto extensions (SmileModel/market-series/attribution) + calendar ON-SN/IMM fix = Landing (uncommitted). Greek wording: "the full FX desk Greek set — 14 sensitivities in one pass". Counts: quote the gated GA figure + "and growing", never a hard-coded integer. Wire latency = loopback compute+framing bound, not a cross-host headline; lead with in-core ns. Re-sync (re-read ledger + research) when the code session commits the workflow.
