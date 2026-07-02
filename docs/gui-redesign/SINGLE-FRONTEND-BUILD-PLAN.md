# Single Front-End Build Plan — the entirety of Celnet

**One integrated front-end over one integrated core.** Not a per-asset-class set of apps, not FX-with-an-FI-peer. Every asset class (FX · Metals · Equity · Commodity · Crypto · **Rates/FI**) is a **license-gated instance of one architecture**. FI/rates is **migrated into** the unified core, not stood beside it.

Built against the **target architecture** (ADR-0010 term-structure unification, central-core one-`Priceable` contract, ADR-0014 generated wire) — not today's FX-centric shape. Every capability carries an honest status tier (below). Lodestar-verified 2026-07-01 (three explorer passes + direct graph checks).

## 1. Ground truth — three honesty tiers

**Tier A — LIVE & client-exposed** (build/redesign the UX only):
pricing (vanilla + full exotics catalogue + cross-asset `price_cross_asset`) · vol surface + arb gate + `SurfaceBook` publish · RFQ / LP-take / desk (`MultiDealerEngine`) · risk / VaR / ES / **FRTB-SbM** · books / positions / PnL attribution · streaming (most cross-client-exposed) · FIX admin *(GUI; Excel gap)* · pre-trade limits · auth / RBAC capability-matrix · OIS pricing + `AggregateRatesRisk` + FI reference-data.

**Tier B — WIRED, NO CLIENT** (backend + wire ready → *build the front-end now, highest ROI*):
- **XVA** — `PricingService.PriceXva` → `PricingEdge.price_xva` (pricing.rs:175) → `celnet_xva::compute_xva` (cva.rs:94) via QMC `ExposureProfile.simulate` (exposure.rs:93); WS arm `price_xva_request_from_json` (ws/codec.rs:1472); oracle-validated. **Zero clients consume it.** ← *D-xva is DONE; prior "deferred" note was stale.*
- **FI risk SDK/CLI parity** — `AggregateRatesRisk`/`BookRatesPosition`/`ListRatesPositions` are GUI-only; ADR-0014's generated wire closes this.

**Tier C — CONTRACT GAP** (no RPC exists → *backend greenfield first, then UI*):
- **Contribution / LP-make / tiering** — no RPC models outbound market-making. (`BrokerQuoteSet` is *inbound* broker-quote ingestion for calibration, not LP-make.) The loop's "make" leg + the mockups' contribution book are a **TARGET**.
- **Feed-management runtime API** — blend/divergence logic exists but is deploy-time config; no runtime admin RPC/UI.
- **Reporting service** — none; reporting is scattered ad-hoc formatting.
- **Plugin-management service** — no `PluginService` among the proto services; extensibility is SDK/host-only.
- **Ops/latency client dashboard** — server-side telemetry only, not surfaced.

**FI/rates specifics:** OIS pricing/risk LIVE; **IRS/FRA/STIR-future/bond engines built but unwired** (no `RatesInstrument` arm); **`celnet-bond` fully built with ZERO dependents**; multi-curve / turns / Jacobian orphaned; rates reachable to the client only via a separate `dialect_rates` FIX channel.

## 2. Two structural risks to resolve first

1. **Branch divergence.** `gui/experience-redesign` forked before main gained FI reference-data + the curve pillar-editor (main `CurveWorkspace.tsx` = 1146 lines with `PillarEditorMode`/`InstrumentReferenceMode` + `ReferenceDataWorkspace`; this branch = 417 lines) and WS-keepalive/reconnect fixes. Main lacks this branch's 7-chart wiring + Shell/rail/license rework. **Neither has the other's work.**
2. **FI is siloed, not integrated.** Today: 6 FI-only workspaces, 4 disjoint blotters, no shared components — the *opposite* of the target. `celnet-risk-cube::DimensionId` has no `Rates` variant.

## 3. Target the front-end binds to
- **One canonical API** — one `Priceable`/`MarketResolver`/`RiskMeasure`; ADR-0014 generated wire from the proto descriptor; **retire the client-visible `dialect_rates` split**.
- **`DiscountCurve` substrate** (ADR-0010) — Carry = degenerate curve; rates `Curve` = general case; **FX byte-identity preserved** (`to_bits` equality — non-negotiable).
- **Licensing is the only per-class differentiator** — the capability matrix (Action × Class × Desk); 3-state gating (present / gated-upsell / hidden).
- **The market-making loop is the experience spine** — `feed → pricing/model → contribution ⇄ RFQ/LP-take + sales-trader → books → risk/XVA → ops` (risk → skew), with honest LIVE/TARGET badges on every leg.

## 4. Phased build

**Phase 0 — Reconcile + anchor** (no new features)
- Graft main's ReferenceData + curve pillar-editor + WS-keepalive into the redesign branch; land the viz/redesign into main. One `gui/`.
- Merge/re-point so `docs/gui-redesign/` is lodestar-indexed (currently invisible to the graph).
- Register target-arch ADRs + capture the plan as lodestar claims/deliverable (§6).

**Phase 1 — Build Tier-B surfaces** (backend ready, fastest value)
- **XVA workspace**: wire `XvaExposureFan` + CVA/DVA/FVA + EPE/ENE to `PriceXva`. Closes the loop's risk/XVA leg.
- FI risk on SDK/CLI via the generated wire.

**Phase 2 — Migrate FI into the integrated core** (the heart)
- Backend (central-core Phase B + ADR-0010): `celnet-rates` OIS + `celnet-bond` implement `Priceable`; `RatesInstrument` gains additive `Irs`/`Fra`/`StirFuture`/`Bond` arms; `DiscountCurve` hoisted to `celnet-core`; risk-cube gains rates buckets; rates GIRR into FRTB.
- Front-end: **collapse the 6 FI-only workspaces into the shared pricing / surface / risk / books workflows**, parameterized by asset class + license. A rates ticket = the pricing workflow bound to a rates instrument under a rates entitlement. Add multi-curve (discount+projection), turns/meeting jumps, Jacobian, cashflow schedules.
- ADR-0016 curve-bucketed DV01/tenor pre-trade limits → booking must handle `BookOutcome::LimitBreached`.

**Phase 3 — Build Tier-C capabilities** (backend greenfield → UI)
- Contribution/LP-make/tiering engine + RPC → contribution console (loop "make" leg).
- Feed-management runtime API → feed console. · Reporting service → report builder. · Plugin-management service → models/plugins console. · Ops/latency dashboard.

**Phase 4 — Unify + parity**
- One cross-asset book (collapse the 4 blotter silos). · Excel parity (FIX-admin, XVA). · Close the loop wiring (inventory→skew, LP-take-hedge→book, risk/XVA push).

## 5. Capability → shared component → API → status → phase

| Capability | Shared component | API | Tier | Phase |
|---|---|---|---|---|
| Price (all classes incl. rates) | `TicketWorkspace` (class-parametric) | `PricingService.Price` / `price_cross_asset` | A | 0/2 |
| Vol surface + publish | `SurfaceWorkspace` | `SurfaceService.*` + `SurfaceBook` | A | 0 |
| Structure/strategies | `StructureGallery`+`PayoffDiagram` | `Strategy`/`StrategyKind` | A | 0 |
| RFQ / LP-take | `DealerPanel` | `QuoteService.RequestMultiDealerQuote` | A | 0 |
| Risk / FRTB (+ rates GIRR) | `RiskWorkspace` (one cube) | `RiskService.*` | A→ | 2 |
| **XVA** | **new `XvaWorkspace` + `XvaExposureFan`** | `PricingService.PriceXva` | **B** | **1** |
| Books / PnL (unified) | one `BookWorkspace` | `ListPositions`/attribution | A→ | 4 |
| Curve / rates | shared surface/curve workbench | `PriceRates` + generated wire | A→ | 2 |
| Streaming | `StreamWorkspace` | `StreamService.StreamSession` | A | 0 |
| FIX admin | `ConnectionsWorkspace` (+Excel) | `FixAdminService` | A | 0/4 |
| Admin / license | `AdminWorkspace`+capability matrix | `AuthService` (28 RPCs) | A | 0 |
| **Contribution/LP-make** | new contribution console | **none — build RPC** | **C** | **3** |
| **Feed management** | new feed console | **none — build RPC** | **C** | **3** |
| **Reporting** | new report builder | **none — build service** | **C** | **3** |
| **Models/plugins** | new plugins console | **none — build service** | **C** | **3** |
| Ops/latency | new ops dashboard | telemetry surface | C | 3 |

## 6. Knowledge capture (lodestar — cohesive across sessions & GitHub)
Per the sharing model: capture as ADRs + anchored claims + a deliverable roll-up, then sync→commit→push so it flows to every dev + session.
- Register **ADR-0013..0016 + ADR-0010** into the graph (`manage_adr`); today they're branch docs only.
- **`single-front-end` deliverable**: author `spec:satisfies` claims for its 4 assertions; add child requirements per phase.
- Anchored **`decision`/`invariant` claims**: the loop spine + its 7 gap legs; the 3-state license model (`commands.ts::railState`); the LIVE/TARGET honesty map; **FI-must-migrate-into-shared-surfaces** (tied to ADR-0010 layers 2/3); the **XVA-live correction** (stale-memory fix); the **branch-divergence reconciliation** decision.
- 10 missing mockup-surface decision claims (6/16 exist).

## 7. Guardrails
One canonical API (no versioning). Coordinator owns merges + `t2`; single-M4 cargo serialized; numerical vs independent oracles ≤1e-12; **FX byte-identity preserved through the curve trait**. No mocks/placeholders. lodestar-first; raise upstream (no workarounds).
