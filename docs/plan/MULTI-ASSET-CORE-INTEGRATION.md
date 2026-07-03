# Multi-Asset Core Integration — the full rearchitecture plan

**Status:** PLAN (authored 2026-07-02 on `origin/main` a63e095, from a 5-surface lodestar
mapping sweep). Decomposed into disjoint, claimable workstreams for parallel sessions
(encoded on `coord/board`). **This is the plan of record for making CelNet a
first-class multi-asset platform** — options + fixed income (+ credit) equally integrated
across every surface.

## 0. Thesis

CelNet is no longer an options pricer with a rates side-car. Every asset class must be a
**first-class core capability** on **every surface** — one pricing+risk contract, one
dispatch, one risk cube, one wire contract, one set of clients — so a user gets a
*perfectly integrated* cross-asset experience. The central-core migration (ADR-0020: one
`Priceable`/`MarketResolver`/`RiskMeasure` contract; C1 unified dispatch; C2 unified risk
cube) delivered the **engine-room** unification. This plan finishes the job: it takes FI
from "priceable internally" to "first-class everywhere," fills the genuine product gaps
(credit, inflation, basis), completes the regulatory surface (GIRR vega/curvature, CSR,
XVA), reaches full client parity, and removes all legacy.

## 1. What is ALREADY done (do NOT re-plan these)

Verified on a63e095 (authoritative git check — earlier lodestar reports were off a *stale*
index at 86ecb46; the graph is now re-indexed):

- **One contract** — options (FX vanilla/cross-asset/23 exotics) + FI (OIS, bond) all impl
  `Priceable`/`MarketResolver`/`RiskMeasure` (ADR-0020).
- **One dispatch** — `PricingEngine` (C1), byte-identical, duplicate dispatch deleted.
- **One risk cube (C2c)** — `celnet-risk-cube/src/fi.rs` (501 LOC) + `celnet-core/src/tail.rs`
  (93 LOC): joint economic VaR/ES (options spot/vol + FI rate scenarios at one market state)
  **and** FRTB SbM GIRR-delta folded into `FrtbCapital`; one shared `tail_var_es` primitive;
  OIS/bond DV01 sign-normalized. Options path byte-identical.
- **FI risk engine** — `celnet-rates-risk` (rate-scenario VaR/ES + GIRR delta, MAR21).
- **Codebase is clean at the code layer** — no `todo!()`, no dead-code markers, no
  options-only hardcoding. The `price_{exotic,rates}_via_contract` seams are **test-only**
  (differential-harness targets; retire with the G codec swap, not before).

Reality-check on crates flagged "absent" by the stale index: `celnet-xva` (809 LOC) and
`celnet-replog` (6147 LOC) **exist** — they are *activation* tasks, not greenfield.
`celnet-credit` and `celnet-inflation` are **genuinely absent** — greenfield.

## 2. Coverage matrix (asset-class × surface, on a63e095)

✅ first-class · ◐ partial/internal-only · ✗ absent

| Surface | Options | Rates (OIS) | Rates (IRS/FRA) | Bond | Credit/CDS | Inflation |
|---|---|---|---|---|---|---|
| Price (engine) | ✅ | ✅ | ◐ analytics, not wired | ◐ via_contract | ✗ | ✗ |
| Wire RPC | ✅ `Price` | ◐ `PriceRates` | ✗ | ✗ | ✗ | ✗ |
| Streaming (WS) | ✅ | ✗ | ✗ | ✗ | ✗ | ✗ |
| RFQ (WS) | ✅ | ✗ (FIX only) | ✗ | ✗ | ✗ | ✗ |
| FIX/edge | ✅ | ✅ | ✅ | ✗ | ✗ | ✗ |
| Surface/curve query | ✅ smile | ✗ no `GetCurve` | ✗ | ✗ | ✗ | ✗ |
| SDK (Rust) | ✅ | ◐ OIS-only | ✗ | ✗ | ✗ | ✗ |
| Risk: VaR/ES | ✅ | ✅ (C2c) | ✅ | ✅ | ✗ | ✗ |
| Risk: FRTB delta | ✅ | ✅ GIRR | ✅ | ✅ | ✗ | ✗ |
| Risk: FRTB vega/curv | ✅ | ✗ | ✗ | ✗ | ✗ | ✗ |
| XVA | ◐ engine, unwired | ✗ | ✗ | ✗ | ✗ | ✗ |
| Limits | ✅ Greeks | ✗ no DV01/PVBP | ✗ | ✗ | ✗ | ✗ |
| Entitlements | ✅ Greek-level | ◐ book-cell | ◐ | ◐ | ✗ | ✗ |
| GUI | ✅ | ◐ OIS ticket | ✗ | ✗ | ✗ | ✗ |
| Excel | ✅ | ◐ `=RATES` OIS | ✗ | ✗ (no `=BOND`) | ✗ | ✗ |

The pattern is systematic: FI is *priceable internally* but not *first-class on the
surfaces*. Closing this matrix, column by column, IS the multi-asset rearchitecture.

## 3. The four pillars & workstreams

Each workstream is a `coord/board` task (id = deliverable slug). Owner hints: **BE**=backend/
server-quant, **Q**=numerical/FI-session, **GUI**=GUI-session, **SDK**=client, **DOC**=docs.
Sizes S/M/L. All numerical work validates against an **independent oracle** (QuantLib / ISDA
std model / FRTB worked examples) ≤1e-12; options paths stay byte-identical.

### PILLAR 1 — TAIL (finish the central-core migration) — BE/Q
| id | what | scope | deps | gate | size |
|---|---|---|---|---|---|
| `fi-wire-instruments` | RatesInstrument proto oneof + `price_rates` arms for VanillaIRS/FRA/Bond (analytics exist, QuantLib-validated) | celnet-proto, celnet-server/rates_pricing | — | T2 | M |
| `fi-pluggable-dispatch` | `FiProductEngine` + `dispatch_rates_live` + `ModelRegistry` FI-kind + handle_unary FI branch (mirror options `dispatch_live`) → plugin-host reaches FI | celnet-server/pricer, celnet-plugin-{api,host} | fi-wire-instruments | T2 | M |
| `be-combined-tail-risk-rpc` | expose C2c `combined_tail_risk` (one-cube tail risk) through an RPC/edge so clients can call it | celnet-proto, celnet-server, celnet-risk-cube | — | T2 | M |
| `be-girr-vega` | FRTB SbM GIRR vega charge (0 today) | celnet-rates-risk | — | T2 | M |
| `be-girr-curvature` | FRTB SbM GIRR curvature (±25bp) | celnet-rates-risk | be-girr-vega | T2 | M |
| `be-node-aggregate-fi-unification` | F3: fold additive `RatesFleetReducer` into `NodeAggregate` → one additive path | celnet-risk-fleet, celnet-server | — | T2 | M |

### PILLAR 2 — FI-AS-CORE (first-class on every surface)
**2a Wire/contract — BE**
| id | what | scope | deps | gate | size |
|---|---|---|---|---|---|
| `ws-codec-from-proto` | **G INC3+4** (land `arch/G-ws-codec-full-swap`): descriptor-driven codec + override table + differential byte-identity harness + handle_unary swap. **Blocks all new codec work.** (Correction: the `price_*_via_contract` seams are the C2 `RiskMeasure` risk seam per ADR-0017 — NOT retired here; retiring them would orphan live `FxSurfaceResolver`/`RatesOisEngine` leaves.) | celnet-server/ws, celnet-proto/build.rs | — | T2 | L |
| `unified-price-rpc` | **⚑ DECISION** collapse `Price`/`PriceRates`/`PriceXva` → one `Price(oneof Instrument)` so any asset class is first-class without a new RPC | celnet-proto, server, client, GUI, Excel | ws-codec-from-proto | T2 | L |
| `rates-stream-ws` | FI streaming on WS StreamService (`handle_series_subscribe` unimplemented non-FX) + rates fanout ring | celnet-server/{stream,pricefanout}, celnet-proto | unified-price-rpc | T2 | L |
| `rates-rfq-ws` | FI RFQ on WS QuoteService + `RatesQuoteRequest` | celnet-proto, celnet-server/ws | ws-codec-from-proto | T2 | M |
| `curve-surface-query` | `GetCurve`/`MarkCurve`/`CurveScenario` on SurfaceService | celnet-proto, celnet-server/surface | ws-codec-from-proto | T1 | M |
| `fix-bond-dialect` | bond FIX dialect (`decode_bond_rfq`) — bonds priced server-side, no FIX route | celnet-fix, celnet-server/fix | — | T1 | S |
| `multi-dealer-rates-rfq` | `MultiDealerEngine` + LP adapter rates arm | celnet-rfq | rates-rfq-ws | T1 | M |
**2b Risk/XVA/regulatory — BE/Q**
| id | what | scope | deps | gate | size |
|---|---|---|---|---|---|
| `celnet-xva-activation` | wire the EXISTING celnet-xva engine into the server: XvaService proto + handle_unary + codec + dep | celnet-xva, celnet-proto, celnet-server | celnet-credit-cds-core (hazard) | T2 | M |
| `be-xva-exposure-profile` | EPE/ENE profile on the PriceXva response (board task) | celnet-proto, celnet-server, celnet-xva | celnet-xva-activation | T2 | M |
| `be-xva-rates-exposure` | IR curve paths in `ExposureProfile.simulate` (options-only today) | celnet-xva | celnet-xva-activation | T2 | L |
| `be-limits-fi-parity` | `LimitMetric::{Dv01,Pvbp,RateTenorBucket}` + `exposure_of_rates` | celnet-limits, celnet-server | — | T1 | S |
| `be-entitlements-fi-parity` | rates Greek-level pruning (book-cell → Greek-level) | celnet-server/services/rates_book | — | T1 | S |
**2c Products — Q (FI-session; wrap, don't clobber)**
| id | what | scope | deps | gate | size |
|---|---|---|---|---|---|
| `celnet-credit-cds-core` | **NEW crate** survival curve + single-name CDS (PV/par/upfront/CS01/JTD) + FRTB CSR SbM buckets. Oracle: ISDA Standard Model (Apache-2.0) | crates/celnet-credit (new) | — | T2 | L |
| `fi-inflation-products` | **NEW crate** ZCIS/YYIS/breakeven + inflation curve. Oracle: QuantLib | crates/celnet-inflation (new) | — | T2 | M |
| `fi-basis-products` | OIS-SOFR + cross-ccy basis swaps (multi-curve bootstrap exists) | celnet-rates | — | T1 | M |
| `fi-stir-futures-product` | expose STIR futures as a priceable product (calibration-only today) | celnet-rates, celnet-server | fi-wire-instruments | T1 | S |
**2d SDK — SDK**
| id | scope | deps | gate | size |
|---|---|---|---|---|
| `sdk-fi-price-parity` (price_bond/price_irs/price_fra) | celnet-client | fi-wire-instruments | T1 | M |
| `sdk-xva-rust` (Client::price_xva) | celnet-client | celnet-xva-activation | T1 | S |
| `sdk-fi-stream` (subscribe_rates) | celnet-client | rates-stream-ws | T1 | M |
| `sdk-fi-rfq` (rfq_rates) | celnet-client | rates-rfq-ws | T1 | S |
| `sdk-fi-scenario` (scenario_rates_risk) | celnet-client | be-combined-tail-risk-rpc | T1 | S |
**2e Clients — GUI (GUI-session-owned; flagged)**
| id | scope | deps | gate | size |
|---|---|---|---|---|
| `fi-bond-ticket-gui` | gui/src/products, gui/src/data/pricing.ts | sdk-fi-price-parity | T2 | M |
| `fi-stream-gui` | gui/src, wsCodec | sdk-fi-stream / rates-stream-ws | T2 | L |
| `fi-scenario-gui` | gui/src/viz | sdk-fi-scenario | T2 | L |
| `fi-rfq-gui` | gui/src | rates-rfq-ws | T2 | L |
| `fi-xva-gui` | gui/src/data/xvaPricing.ts | celnet-xva-activation | T2 | M |
| `excel-bond-fn` `=CELNET.BOND()` | excel/src/functions | sdk-fi-price-parity | T2 | S |
| `excel-fi-stream` | excel/src/functions | sdk-fi-stream | T2 | M |
| `excel-fi-rfq` | excel/src/functions | sdk-fi-rfq | T1 | S |
| `excel-fi-mark-bond` | excel/src/functions | curve-surface-query | T1 | S |

### PILLAR 3 — POLISH — BE/Q/coordinator
| id | what | scope | gate |
|---|---|---|---|
| `exotics-fuzz-hardening` | fix 2 payoff-bounds fuzz edges (Asian AM-GM band; large-magnitude parity RELATIVE tol) | celnet-exotics/tests | T1 |
| `knowledge-multiasset-claims` | author FI-conformance + central-core + multi-asset lodestar claims (single-writer) + a multi-asset ADR | .lodestar/knowledge, docs/adr | — |
| `landing-arms-gate` | perpetual/listed-future payoff arms (wip `c8fb02f`) full-workspace gates | celnet-* | T2 |

### PILLAR 4 — LEGACY / HYGIENE — coordinator/DOC
| id | what | gate |
|---|---|---|
| `land-J-desk-authz-e2e` | merge `lane/J-desk-authz-e2e` (2 commits) under Enforce + Playwright e2e | T2 |
| `land-orchestration-agent-teams` | merge `arch/orchestration-agent-teams` (doc-only) | T1 |
| `branch-worktree-prune` | prune ~25 integrated/stale branches + ~10 stale worktrees; keep coord/board, main, ship-program | — |
| `interfaces-doc-sync` | INTERFACES.md 40→43 crates (+celnet-bond/rates-risk/risk-accel) + deferral registry refresh | — |
| `architecture-doc-sync` | ARCHITECTURE.md crate tree + layers | — |
| `roadmap-doc-sync` | ROADMAP.md WS-R (central-core FI risk) + caveats | — |
| `docs-fi-parity` | de-"options-only" ANALYTICS-SPEC/INTERFACES/ARCHITECTURE prose | — |
| `celnet-replog-activation` | wire existing Raft (6147 LOC) into server lifecycle — **configurable consistency per operator directive** | T2 |

## 4. Dependency DAG (critical path first)

```
ws-codec-from-proto ──► unified-price-rpc ──┬─► rates-stream-ws ─► sdk-fi-stream ─► {fi-stream-gui, excel-fi-stream}
   (G branch, land 1st) (⚑ DECISION)         ├─► rates-rfq-ws ─► {multi-dealer-rates-rfq, sdk-fi-rfq ─► fi-rfq-gui, excel-fi-rfq}
                                             └─► curve-surface-query ─► excel-fi-mark-bond
fi-wire-instruments ─► fi-pluggable-dispatch
        └─► sdk-fi-price-parity ─► {fi-bond-ticket-gui, excel-bond-fn}
celnet-credit-cds-core ─► celnet-xva-activation ─┬─► be-xva-exposure-profile ─► fi-xva-gui
                                                 └─► be-xva-rates-exposure ; sdk-xva-rust
be-combined-tail-risk-rpc ─► sdk-fi-scenario ─► fi-scenario-gui
INDEPENDENT (start anytime): be-girr-vega ─► be-girr-curvature ; be-node-aggregate-fi-unification ;
   be-limits-fi-parity ; be-entitlements-fi-parity ; fi-inflation-products ; fi-basis-products ;
   fix-bond-dialect ; exotics-fuzz-hardening ; celnet-replog-activation ; all doc-sync tasks ; land-* ; prune
```

## 5. How parallel sessions pick up (launch guide)

- One session runs `CELNET_ROLE=coordinator` (owns merges-to-main, the single t2, the proto
  window); all others `CELNET_ROLE=worker`. Each worker: `tools/celnet-task selector` → claims
  the best open task on a disjoint scope → builds → T1 → `done` (in_review) → coordinator
  batches the t2 + lands. Single-M4 → `cargo-lane acquire/release` around every cargo run.
- **First wave (no deps, maximal parallelism, ~8 lanes):** `ws-codec-from-proto` (unblocks the
  wire), `fi-wire-instruments`, `be-girr-vega`, `be-limits-fi-parity`, `celnet-credit-cds-core`,
  `fi-inflation-products`, `fi-basis-products`, the doc-sync + prune lanes.
- **Proto window:** `unified-price-rpc`, `celnet-xva-activation`, `be-combined-tail-risk-rpc`,
  `rates-rfq-ws`, `curve-surface-query` all touch `celnet.proto` → coordinator opens ONE proto
  window and serializes these (`celnet-task window open`).
- **GUI/Excel lanes** wait on their SDK/wire deps, owned by the GUI session.

## 6. The one open decision (⚑)

**`unified-price-rpc`** — collapse `Price`/`PriceRates`/`PriceXva` into a single
`Price(PriceRequest{ oneof instrument })`, vs. keep the per-class RPCs. Unifying is the
cleanest "perfectly integrated" contract (one verb, every asset class first-class, no new
RPC per class) and fits the no-versioned-API rule; cost is a one-time cross-cut (proto +
server + all clients) best done right after `ws-codec-from-proto` and before the streaming/
RFQ/credit wire work builds on top. Recommendation: **unify.** Operator to confirm.

## 7. Deferral backlog (tracked, lower priority — not first-wave)

From the full registry (docs/INTERFACES.md §D + ledger): exotic/cross-asset fill
auto-ingestion (R1), durable market-series store (F1), child-limits cascade solver (L1),
entitlements audit log (E1), role↔principal + user-admin GUI (E2), AAD adjoint risk (P1),
distributed cross-shard reduction (P2), auto vol-triangulation sign (P3), global allocator
(A1), SIMD `wide` (A2), Tier-1 native ABI / Tier-3 Landlock (A3/A4), GPU Sobol' (A5),
FRTB √2 liquid-ccy divisor (C2) + settlement-capital (C5, out-of-scope). Each is a future
task; none blocks multi-asset first-classness.
