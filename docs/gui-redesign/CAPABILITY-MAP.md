# Celnet Capability Map — the design's coverage source of truth

From the all-crates lodestar inventory (40 crates, graph `github.com-soarsa-celnet`). This is what the
sell-side front-end must be able to reach. Tags: **LIVE** (end-to-end via ≥1 client) · **LANDING**
(code+gates, workspace gate pending) · **DEFERRED** (built, not wired) · **NO-SURFACE** (library/
server-internal, no client reachability). The design SURFACES the hidden ones and GATES by license.

## A. Per-asset-class licensing tiers — ENGINE-ENFORCED (the licensing primitive)
The server hard-rejects out-of-tier products (`cross_asset_non_vanilla_product_is_rejected`,
`pricer.rs:697`). The UI must mirror this — present/gate/upsell per tier, never offer a ticket the
engine rejects.

| Class | Vanilla | Exotics (18) | LSV / 5-family surface | Linear | Notes |
|---|:-:|:-:|:-:|:-:|---|
| **FX** | ✅ GK 13-Greek | ✅ full | ✅ full | ✅ fwd/swap/NDF | reference/complete class |
| **Metals** (XAU/XAG/XPT/XPD) | ✅ | ✅ full (rides FX path) | ✅ full | ✅ | treated as FX |
| **Equity** | ✅ gen-BSM +div | ❌ rejected | ❌ | n/a | +LANDING perpetual/future |
| **Commodity** | ✅ Black-76 | ❌ rejected | ❌ | n/a | +LANDING perpetual/future |
| **Crypto** | ✅ linear + inverse 1/S_T | ❌ rejected | ❌ **no marked surface** (flat vol) | ✅ settle-style | +LANDING perpetual/future |
| **Rates/FI** | n/a (linear curve) | n/a | n/a | ✅ FRA/IRS/STIR/bonds | separate `RatesInstrument` leaf, own `RatesService` |

**License model:** the `AssetClass` axis already exists in `celnet-entitlements::capability`. Product
tiers to sell separately: **FX+Metals (full)**, **Equity / Commodity / Crypto (vanilla + landing
perpetual/future)**, **Rates/FI (curve suite)**. Integrated experience; modular license.

## B. Bucket coverage (condensed)
1. **Pricing & modelling** — vanilla all-class LIVE; full exotics + LSV LIVE (FX/metal only); rates
   LIVE; perpetual/listed-future LANDING; scenario grid LIVE (Excel gap). Models: GK/genBSM/Black76/
   crypto/LSV LIVE; **Heston NO-SURFACE**; **GPU batch/MC/greeks NO-SURFACE**; plugin/house-model
   prices LIVE but **model-management NO-SURFACE**.
2. **Vol surface lifecycle** — mark/calibrate/version/publish + VV/SABR/SVI/SSVI/eSSVI + arb gates
   LIVE (all 5 clients); Dupire LIVE (FX/metal); **crypto strike-axis surface DEFERRED**.
3. **Market-data & FIX (inbound)** — multi-source aggregation/staleness/`divergence_report`,
   `DeploymentMode`, FIX 4.4 acceptor LIVE (algorithm; live vendor values deploy-gated); symbology/
   compositing LIVE algorithm; **no per-source health/weights/divergence admin view (NO-SURFACE)**.
4. **Contribution & distribution (outbound)** — SPMC fanout ring, RFS multiplex StreamSession +
   TrendMode series, two-way RFQ + click-to-trade, multi-dealer RFQ-to-many (ranked, last-look),
   **maker auto-quote desk** (`RfqDeskEdge`, `QuotingWorkspace`), conflating `EgressGovernor`, FIX
   out (initiator), attribution chain (quoted_by/held_by/won/lp_count) — all LIVE.
5. **Risk @ IB scale** — risk-cube OLAP additive roll-up + vega ladder + drill LIVE; non-additive
   VaR/ES + FRTB-SbM curvature + corr-weighted vega LIVE (bump-revalue; **AAD/GPU throughput lever
   NO-SURFACE/not built**); numeraire normalize LIVE; **cross-shard fleet = algebra LIVE, transport
   DEFERRED (single-node only)**; rates risk `AggregateRatesRisk` LIVE.
6. **Desks & books** — org hierarchy Trader→Book→Desk→Location→Entity + Underlying (`RiskDimension`
   enum) LIVE; vanilla position store auto-books LIVE; FI rates book LIVE; deal blotter LIVE;
   **exotic/cross-asset live auto-booking NO-SURFACE (data model done, no RPC path calls
   `upsert_exotic`)**.
7. **Permissioning & licensing** — deny-wins pre-agg entitlement pruning + Action×AssetClass×Desk
   capability kernel + per-user overlay + FI-desk gating LIVE; **limit-cascade constraint solver
   NO-SURFACE/not built**.
8. **Reporting** — ❌ **NONE exists** (no `ReportingService`, no export). CLI pretty-printers +
   `ArbReport`/`ExecutionReport` are not reports. Regulatory report = named backlog (POST-GA E-7),
   "deepest work". → **GREENFIELD: design as first-class, flag new backend.**
9. **Admin & ops** — FIX connection config + per-user capability overlay + `ConnectionsWorkspace`
   LIVE; telemetry HdrHistogram p50/p99/p99.9 + `/livez//readyz//healthz` LIVE internally but
   **no operator dashboard (NO-SURFACE)**; OTLP DEFERRED; journal LIVE; **Raft/HA (`celnet-replog`)
   DEFERRED**; fleet failover transport designed-only; blue-green hot-upgrade designed.
10. **Plugin/extensibility & SDKs** — `celnet-plugin-api` frozen + Tier-0 native + Tier-2 wasmi
    sandbox LIVE and wired to the vanilla dispatch path; **plugin/model management UX NO-SURFACE**;
    Rust SDK + CLI + Excel + GUI LIVE.

## C. NO-CLIENT-SURFACE priority list (the "out-function" fuel — expose these)
1. **XVA** (CVA/DVA/FVA, EPE/ENE) — built+gated, no wire. Highest-value gap. (⟂ D-xva cargo lane.)
2. **Plugin/model management** — host is live-wired; no install/list/version/retire UI.
3. **Observability dashboard** — full p50/p99/p99.9 + metrics server-side, nothing surfaces them.
4. **Feed compositing/health** — divergence/staleness/blend built; no per-source admin view.
5. **Exotic/cross-asset live position auto-booking** — data model done; no RPC path books it.
6. **Crypto strike-axis vol surface** — deferred fast-follow; crypto prices off flat vol today.
7. **GPU speed lever** — wgpu batch/scenario/MC validated; zero production dispatch.
8. **Risk AAD/GPU throughput** + **cross-shard transport** — named, not built (IB-scale levers).
9. **Limit cascade constraint solver** — named-deferred (admin config path).
10. **Reporting** (valuation/risk/activity/regulatory) — none; greenfield.
11. **Raft/HA status** — dormant; no admin surface plan.

## D. Doc-hygiene issues found (flag to the docs session — not this lane)
- `docs/INTERFACES.md` ~L975–1025 has an unresolved `diff3` conflict marker (`|||||||  7878048`) in the
  "WS codec from proto descriptor" section — stale merge artifact.
- `docs/CELNET-CAPABILITIES.md` says "34 one-way-acyclic crates" — stale; live count is **40**.

## E. Design implications
- The **licensing primitive** is real and engine-backed — build present/gated/upsell straight from
  the `AssetClass × Action × Desk` capability kernel; tiers = FX+Metals / Equity-Commodity-Crypto /
  Rates.
- The **hero surfaces** (pricing/modelling, surface, contribution/distribution, risk) are LIVE and
  rich — the redesign's job is to make them intuitive + integrated + IB-scale-clean, and to SURFACE
  the §C hidden capabilities that competitors don't have (XVA, model mgmt, feed health, observability).
- **Desks/books** is the scoping spine (real `RiskDimension` hierarchy) — everything scopes to it.
- **Reporting** is genuinely new — design it, mark the backend as to-build.
