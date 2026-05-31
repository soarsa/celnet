<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Celer Trader & Estate Integration</sub>

# 12. Celer Trader & Estate Integration

Celnet is the **FX-options pricing system-of-record** inside the Celer trade lifecycle. Where the broader Celer estate routes, risks, clears and books FX flow, Celnet owns the option mathematics — price, the full Greek set, the marked surface, and option risk — and injects them into both the price path and the order path through a small set of clean seam traits. The result is a front-to-back FX-options capability that slots into a running Celer estate without a parallel pricing stack and without a rewrite.

![The Celer trade lifecycle with Celnet as the FX-options pricing system-of-record](../assets/celnet-capabilities/fig-09-celer-lifecycle.png)
*Figure 9 — Celnet in the Celer trade lifecycle. Celnet supplies price, Greeks and surface into the price path and option risk into the risk/position path; the surrounding Celer services own routing, credit, execution, clearing and the net-position truth source.*

### 12.1 Where Celnet plugs in

The Celer estate runs two complementary flows, and Celnet contributes to each at exactly the point where option intelligence is needed.

**Price path.** Venue quotes arrive over FIX, are normalized by `celertech-marketdata`, assembled by `celertech-marketmerchant`, fanned out by the distributor over WebSocket, and rendered to the trader in **Celer Trader** — the `celertech-orderrouting-webtrader-react` FX front-end. Celnet enriches this path with FX-option price, the full desk Greek set, and the live calibrated surface, so every option line the trader sees carries Celnet's marks.

**Order path.** From Celer Trader an order flows through `celertech-orderrouting` (the order-lifecycle hub), `celertech-risk` (pre-trade credit and limit checks), `celertech-destination` (FIX-out to many venues), `celertech-clearing`, and into `celertech-positionmanager`, the net-position and P&L truth source. Celnet feeds option risk — sensitivities and scenario shape — into the risk and position legs so credit checks and the position book reflect true option exposure rather than a notional approximation.

| Celer stage | Role | Celnet contribution |
|---|---|---|
| Market-data ingest & quote assembly | Normalize and merchant venue quotes | FX-option price, Greeks, calibrated surface into the quote |
| Distribution → Celer Trader | Stream prices to the React front-end | Option marks on every line, surface-version pinned |
| Order-routing hub | Order lifecycle | Option valuation and risk on the working order |
| Pre-trade risk | Credit & limit gate | Option risk sensitivities for accurate exposure |
| Position manager | Net position & P&L truth | Option Greeks and marked-surface valuation |

### 12.2 The integration edge

Celnet meets the estate through a small, well-defined set of connectors, every one a real, shipping component.

**FIX 4.4 engine.** A real acceptor-plus-initiator FIX engine with an FX-options dialect carries quotes inbound and orders/executions outbound, speaking the same protocol the rest of the estate and its venues already use.

**Conflating egress governor.** Outbound price distribution passes through an egress governor that conflates to the latest value, paces with a token bucket, and counts what it drops — so a fast core never overwhelms a slower consumer, and the conflation is observable rather than silent.

**Resilient market-data subscriber.** A resilient WebSocket subscriber consumes upstream prices with reconnect and resynchronization built in, so a transient feed interruption heals automatically instead of stalling the desk.

**Vendor-feed normalization.** A normalization, blend and divergence layer maps heterogeneous upstream quotes into Celnet's canonical observables, blends multiple sources, and surfaces divergence between them — the foundation for both quote assembly and surface marking.

### 12.3 Three deployment modes, one codebase

Integration depth is a configuration choice, not a fork. Three seam traits — the **market-data source**, the **price sink**, and the **order + execution adapter** — are swapped to move Celnet along a spectrum from fully self-contained to fully estate-native. Migration between modes is a reversible adapter swap, never a rewrite.

![The three Celnet deployment modes and the adapter seams that distinguish them](../assets/celnet-capabilities/fig-10-deployment-modes.png)
*Figure 10 — Standalone, Hybrid and CelerIntegrated deployment modes. The same engine and the same contract sit behind three adapter configurations on the market-data / price-sink / order-and-execution seams.*

| Mode | Market-data source | Price & order seams | Use |
|---|---|---|---|
| **Standalone** | Self-contained | Self-contained | Celnet runs as an independent FX-options pricing and risk platform |
| **Hybrid** | Estate or external feeds | Mixed estate / self-contained | Estate market data with Celnet-owned pricing and distribution |
| **CelerIntegrated** | Celer distributor | Full estate price/order path | Celnet as the estate's FX-options pricing system-of-record |

In **CelerIntegrated** mode Celnet performs a JVM distributor sidecar handshake, takes mailbox calibration, and honours live quote-feed entitlement — becoming a first-class member of the running estate. Because each mode is just a different binding of the same seam traits, a desk can start Standalone, prove the platform, and migrate deeper into the estate at its own pace.

### 12.4 Adaptable by design

The same market-data seam that ingests the Celer distributor ingests external products and feeds. Celnet's normalization layer is built to adapt to the venues and terminals a desk already runs — for example **Fenics**, **Bloomberg**, **Refinitiv** and **EBS** — bringing their quotes into the canonical observable set that drives quote assembly, surface marking and live trend series. New sources are added as adapters behind the seam, so onboarding a feed is an integration, not a re-engineering of the pricing core.

The throughline across all of this is the platform's single principle: one clean contract, consumed identically everywhere. Whether Celnet runs Standalone or wired into the full Celer lifecycle, the price the order path sees, the risk the position manager books, and the number on the trader's screen are the same value from the same source.

---
<sub>[← The Trader GUI](11-trader-gui.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Competitive Positioning →](13-competitive-positioning.md)</sub>
