# Current Front-End Coverage & Gaps (sell-side lens)

From the front-end re-inventory (gui/ + excel/, lodestar-cited). Complements the backend
`CAPABILITY-MAP.md`. Ratings: **Full** / **Thin** / **Absent**. This tells us what to KEEP, what to
FIX, and where the greenfield "out-function" wins are.

## Coverage by bucket
| Bucket | GUI | Excel | Verdict |
|---|---|---|---|
| **Pricing workbench** (28 product families, 14 Greeks class-aware, implied vol) | Full | Full | KEEP — strong. `TicketWorkspace` (1042L) + `GreeksStrip` |
| **Scenario / what-if** (5-factor shock, vega ladder, cross-gamma, theta-roll, pin-compare) | Full | **Absent** (no `CELNET.SCENARIO`) | KEEP GUI; close Excel gap |
| **Vol-surface lifecycle** (mark=publish, versioned, 5 smiles, **blocking arb gate**, dirty badge, downstream-wired to pricing/FIX/quote/stream) | Full | Full (`MARKSURFACE` write) | KEEP — the publish-gate is the TEMPLATE to reuse |
| **Feed management (inbound)** | FIX-session admin only | Absent | **BIG GAP** — no source registry/symbology/health; blend+divergence math built but UNWIRED |
| **Contribution/distribution (outbound)** | Viewer + click-to-deal only | Absent | **BIG GAP** — engine (fanout/`BroadcastRing`, `SpreadModel`) exists; no console/tiering/skew/publish-on-off |
| **RFQ maker (inbound response)** | Full (`QuotingWorkspace`, manual) | Thin | KEEP; **add auto-quote** (absent) |
| **LP hedging (taker toward interbank)** | Full (`DealerPanel`, distinct from client contribution) | Thin | KEEP — this is the desk hedging, NOT client contribution |
| **Risk & books** (aggregate/limits/drill, org hierarchy) | Full | Full | KEEP; unify FX+rates (twin stacks today) |
| **Admin & entitlements** (users/roles/capabilities/desks, deny-wins) | Full | server-enforced only | KEEP |
| **FIX connectivity** (`ConnectionsWorkspace`, acceptor CRUD + session monitor) | Full | Absent | KEEP |
| **Ops / monitoring** (latency p50/p99, alerts, stream stats) | **Absent** | Absent | **GAP** — instrumented server-side, surfaced nowhere |
| **Reporting** | Absent | Absent | **GREENFIELD** (backend also absent) |

## Trading-surface reconciliation (persona confirmation)
**No order/execution/matching engine exists** (`search_graph OrderBook|ExecutionEngine|MatchingEngine`
= only an unrelated fanout queue). `StreamWorkspace`=price viewer+click-to-deal · `DealerPanel`=
interbank taker/hedging panel · `BookWorkspace`=risk aggregation · `DealsBlotter`=position-for-risk
byproduct of an RFQ-accept · `RatesBook`=OIS position/book view. "Booking" verbs are RFQ-quote-accept
semantics. → **Sell-side pricing/distribution/admin persona is code-verified.**

## Fenics/Synoption replacement gaps (the out-function opportunities)
1. **Feed-management depth** — multi-vendor source registry + symbology + visible arbitration/blend +
   per-instrument freshness/tick-rate health. (Math exists in `celnet-integration`, unwired.)
2. **Contribution console** — make `SpreadModel` administrable: per-client-tier spread/skew, per-instrument
   publish on/off, contribution health. (Symmetric to the vol-surface publish gate.)
3. **Auto-quoting** — algo/auto-quote for high-volume inbound RFQ flow.
4. **Ops/feed/stream visibility** — a real ops surface over the existing HdrHistogram instrumentation.
5. **Excel parity** — `CELNET.SCENARIO`; entitlement/feed/ops visibility.
6. **Reporting** — valuation/risk/activity/regulatory (backend greenfield too).

## Persona-correct workflow set (what the front-end must serve)
Exists today: price a structure for a client (all classes) · run a scenario/what-if (GUI) · mark+
calibrate+publish a vol surface (arb-gated) · respond to an inbound client RFQ/IOI (manual) · source a
competitive hedge from an LP panel (taker, for risk) · aggregate+monitor book/position risk vs limits ·
administer users/roles/capabilities/desks · administer inbound FIX acceptors.
**Missing (design these):** configure+monitor an inbound vendor feed (symbology/compositing/health) ·
define a contribution/skew/spread profile per client tier + turn contribution on/off · auto-quote an
inbound RFQ · monitor ops health (latency/alerts/stream stats) · reporting.
