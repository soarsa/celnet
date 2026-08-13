---
name: hedge-wash-lp-analytics-and-ia-reorg
description: "Hedge correctness fix (wash-book honors internal policy + real LP hedge source + LP analytics attribution + exactly-one-record) and the Hedging→\"Hedging Rules\" / Risk-blotters IA reorg; both local main (abf51e2b, 213d2721), NOT deployed — UAT still runs b71767bb."
metadata: 
  node_type: memory
  type: project
  originSessionId: bd2a0685-2d52-4bdc-95d4-4e7872001f3e
---

Shipped 2026-08-10 on **local main** (NOT pushed/deployed; UAT still runs release **b71767bb**).
Three UAT observations (by ben@celnet.com login) were CONFIRMED live + root-caused before fixing:
hedges always filled on COMPOSITE (0 LP hits), Analytics LP league table all-zeros, and
**WASH_BOOK emitted external B2B market orders despite an internal (Warehouse) policy**.

**Server fix `abf51e2b`** (`crates/celnet-server/src/services/rates_book.rs` `stamp_internalise`,
`auto_hedge/engine.rs`, `aggregation.rs`, `analytics/hedge_flow.rs` NEW, `analytics/mod.rs`,
`e2e_lifecycle.rs`, `lib.rs`):
- **A (wash defect):** the below-min-edge shed (<0.5 bp edge, `internalise.rs` verdict; floor
  `default_min_edge_bps=0.5`) was UNCONDITIONAL — force-shed the whole fill externally + stamped
  SubmitMarketOrder even under an internal hold. A wash/internal book captures ~0 edge so hit it
  every time. Now the split HONORS the resolved action: below-edge + internal hold ⇒ warehouse
  (internal=fill, external=0, no B2B); below-edge + external policy ⇒ still back-to-back. There is
  **no wash/internal flag** in the model — a "wash book" is just a RiskBookDef whose policy resolves
  to Warehouse; the engine was overriding it. This makes `external_dv01>0 ⟹ graph_is_external`,
  cleanly deleting the 54b077aa contradictory-row synthesis.
- **B (always-composite):** `stamp_internalise` had hardcoded `NoLpSource` (best_fill always None) so
  every external leg backstopped to composite regardless of LP-panel mode. Now `impl LpHedgeSource
  for AggregationHub` (off `resolve_rfq_composite` per-LP RfqMemberLine bid/offer) is wired as the
  live LP panel (set via `set_lp_hedge_source` in lib.rs). LP-panel modes fill against the best
  executable LP on the required side (side by net-risk sign: reduce long⇒highest fresh bid; reduce
  short⇒lowest fresh offer) → `venue=Lp`, `lp_won=<lp_name>`, real signed slippage vs mid; composite
  only on a genuine miss (never fabricated).
- **C (analytics):** named-LP hedge fills record a won deal+notional for that LP via
  `HedgeFillFlowLog : LpFlowSource` (registered in lib.rs), so the Street-side LP league table
  populates. Composite fills NOT attributed (pseudo-venue); last-look/cover stay absent (no
  fabricated zeros).
- **Exactly one ring record per external fill:** the engine self-rings a DECISION record for external
  actions (`evaluate`→`stamp_provenance`, engine.rs ring). `stamp_internalise` now AMENDS that record
  in place by id (new `AutoHedgeEngine::amend_execution`) with realised economics rather than appending
  → no double-count in the Hedge Deals blotter / "N external" summary; trace hedge_id stays consistent.
  Guarded by `breach_fill_leaves_exactly_one_realised_ring_record` (asserts provenance().len()==1).
  Gate green: fmt, clippy -D, 733 celnet-server + 14 celnet-analytics.

**GUI IA reorg `213d2721`** (per product direction): top tab **Hedging → "Hedging Rules"** (internal
id `hedging` + `hedge·FI` gate unchanged). Hedging Rules = authoring only: Exit Policy · Thresholds ·
LP Panels · **Execution mode** (HedgeConfigControl split off the old Monitor). The live Monitor moved
to **Risk** (`RiskDashboardWorkspace`) as **Hedge flows** (`HedgeMonitor` refactored prop-driven →
self-fetching: `HedgeMonitorView` + container). The single Deals lens-toggle split into **Client
blotter** (`DealsBlotterWorkspace lens="client"`, toggle hidden) + **Hedge blotter** (`HedgeDealsView`).
New Risk tab order: Dashboard·Portfolios·Routing·Acceptance·Positions·Quotes·Client blotter·Hedge
blotter·Hedge flows. Client/Hedge blotters on `view·FI`; Hedge flows `hedge·FI`. `DealsBlotterWorkspace`
gained optional `lens?: "client"|"hedge"|"both"` (default both ⇒ BookWorkspace unchanged). Help/tour/
wizard synced. Seed hand-offs still land on Hedging Rules→Exit Policy. Build green; vitest 1993.

**NONE of the day's commits are live** — UAT runs b71767bb. To see any of it (pricing-group→hedge-rule
f1c2671f, wash/LP/analytics abf51e2b, IA reorg 213d2721) the box must be redeployed — and the deploy
itself needs the swap fix (`8819c440`, via privileged `site.yml`) applied first or it OOM-dies mid-build
(see [[uat-oom-root-cause-and-swap-fix]]).
