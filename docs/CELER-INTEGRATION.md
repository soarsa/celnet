# Celnet — Celer Integration Map

> Status: design document for a greenfield Rust service (`/Users/adrian/code/celeroption`). All Celer-side service names, hops, transports and constraints below are drawn from the `celnet-integration` research findings. Items the research flagged as **inferred** (not runtime-traced) or otherwise unconfirmed are called out explicitly and collected in the Open Questions section. Do not treat them as verified until checked against the real services.

---

## 0. Integration constraints (must-read before designing either side)

These are hard facts from the integration research that shape every decision below:

- **The distributor is an in-process JVM disruptor mailbox.** It runs inside the JVM `baseserver`. A Rust process **cannot natively join it**. Celnet must either (a) run a JVM adapter/sidecar, or (b) speak the distributor socket protocol via `DistributorProducerChannelHandler`. **This choice must be made before any code is committed** — it determines the entire ingress/egress transport.
- **The distributor mailbox is bounded with skip-while-full back-pressure.** A high-frequency option pricer can cause **silent price drops**. Mailboxes must be sized and the pricer rate-limited.
- **Cross-service edges are invisible to automated tooling.** `codebase-memory-mcp` finds zero cross-service edges because the estate uses the in-proc distributor + Protobuf + FIX, not HTTP/gRPC. The dependency map must be maintained **manually** and verified against Spring config and distributor `notifyUsers` names.
- **Several hops are inferred, not traced.** `orderrouting → risk`, `risk → destination`, `destination → clearing`, `clearing → positionmanager` are inferred from API deps and handler signatures — validate before relying on them.
- **`MarketMerchantPriceService` is WS-only with no fallback** and has a ~6 concurrent HTTP connection semaphore per domain. Any new option price stream must be resilient to WS disconnects.
- **No option product type exists in the estate today.** Adding one touches `celertech-type`, `staticdata`, every API proto enum, `positionmanager` netting keys, `risk` exposure models, and `destination` FIX dialect mappings. This is a broad, cross-cutting change.
- **Config is layered at deploy time via `celnet-client` tenant overlays**, not baked into artifacts; the `celnet-client` subgroup is not in the default repo pull, so config wiring may be invisible locally. Confirm tenant overlay ownership with devops.
- **FX options need a vol surface + Greeks inputs that spot-only marketdata may not supply.** Verify `marketdata-api` can deliver vol data or plan an additional feed handler (this is where Fenics FXO 2.0 comes in — see below).

---

## 1. Market-data INGRESS

### 1.1 What Celnet needs to consume

For Garman-Kohlhagen vanilla pricing and the LSV/VV exotics engine, Celnet requires, per currency pair and tenor:

| Input | Purpose | Likely source |
|---|---|---|
| Spot mid/bid/ask | GK spot leg, forward construction | Celer `marketdata` (spot-only today) |
| Forward points / outrights | Forward `F = S·e^{(r_d−r_f)T}`; deliverable vs NDF | Celer `marketdata` (verify), Fenics FX pricing set |
| NDF fixings | NDO/NDF cash-settlement pricing | Fenics (350+ pairs incl. NDF), verify Celer coverage |
| Vol surface quotes: **ATM, 25Δ & 10Δ RR, 25Δ & 10Δ BF** | Smile construction (delta space) | **Fenics FMD FXO 2.0** (primary), verify `marketdata-api` |
| Deposit / discount curves (DF_d, DF_f) | Two-rate discounting, dual-curve | Celer staticdata/curve service (verify) |

> **Critical gap:** the research explicitly warns spot-only marketdata may not deliver vol data. **Fenics Market Data FXO 2.0** is the designated external vol-surface source — it publishes ATM + 25Δ/10Δ RR + BF wing quotes per tenor per pair (300+ pairs, 27 metals) in standard FX-options market convention. Celnet builds an **FMD adapter** that normalizes Fenics' (under-documented) delta/ATM conventions into Celnet's canonical surface object on ingest.

### 1.2 Transports

- **Internal Celer bus:** the in-process JVM **distributor** (disruptor mailbox). Rust ingress requires a JVM adapter or the `DistributorProducerChannelHandler` socket protocol — decision pending (§0).
- **`MarketMerchantPriceService`:** **WebSocket-only**, no fallback, ~6 concurrent HTTP connections per domain. Spot/forward price streaming likely arrives here — the consumer must reconnect/resubscribe resiliently.
- **Protobuf** message bodies and **FIX** are the estate's wire formats (not HTTP/gRPC internally).
- **Fenics FMD:** external feed — API/streaming/snapshot, or via LSEG/Refinitiv redistribution (per the Fenics findings). Ingested by the FMD adapter, not the distributor.

### 1.3 Named ingress dependencies

- `marketdata` / `marketdata-api` — spot (confirmed spot-only), verify forward/NDF/vol capability.
- `MarketMerchantPriceService` — WS price streaming.
- `staticdata` — instrument/curve reference data; will need new option product reference (§0).
- `celertech-type` — shared type definitions; needs an option product type added.
- Distributor (`baseserver`, in-process) + `DistributorProducerChannelHandler`.
- Fenics FMD FXO 2.0 (external).

---

## 2. Pricing / Quote / Risk EGRESS

### 2.1 What Celnet publishes

- **Indicative & firm prices/quotes:** vanilla (GK) + exotics (VV fast tier, LSV booking tier).
- **Full FX Greek set:** delta (per convention), gamma, vega (bucketed/ladder), theta, **rho-domestic AND rho-foreign**, vanna, volga, charm, speed, zomma, color.
- **Constructed vol surface:** dense, arbitrage-free, continuously re-strikable (for re-pricing and frontend display).
- **Risk / exposure / P&L attribution** for downstream consumers.

### 2.2 Where it publishes (estate consumers)

| Consumer | What it receives | Transport / notes |
|---|---|---|
| `marketmerchant` (quote assembly) | Prices/quotes for assembly into the quote stream | Via distributor / `MarketMerchantPriceService` (WS). Rate-limit to avoid skip-while-full drops. |
| `orderrouting` | Tradeable option quotes / order flow | Distributor; hop `orderrouting → risk` is **inferred** — verify. |
| `risk` | Greeks + exposure for the option product | New option exposure model required in `risk`. Hop `risk → destination` **inferred**. |
| `positionmanager` | Position/booking events; needs new netting keys for option product | New netting keys = broad change (§0). Hop `clearing → positionmanager` **inferred**. |
| `destination` | FIX dialect mappings for option execution | New FIX dialect mappings required. |
| `clearing` | Post-trade/clearing | Hop `destination → clearing` **inferred**. |
| Distribution layer | Fan-out of prices/quotes | Bounded distributor mailbox; back-pressure risk. |

### 2.3 Egress to the React webtrader frontend

- The frontend is **React**. Per the estate pattern and Celer's WS-centric `MarketMerchantPriceService`, the frontend consumes prices/quotes over **WebSocket** (gRPC-web/WS — confirm the exact transport against the real webtrader; the estate internals are Protobuf/FIX, so the browser edge is most likely WS carrying Protobuf or a JSON projection).
- Celnet should expose the constructed surface, live Greeks, and scenario/stress outputs to the webtrader so a desk can build bespoke pre-trade screens, structured-product builders and real-time stress dashboards (the differentiator vs SynOption's fixed screens).
- **Constraint:** the WS path has no fallback and a ~6-connection-per-domain semaphore — design the frontend feed for disconnect resilience and connection economy.

### 2.4 Message types Celnet must speak

- **Protobuf** request/response and event messages — must extend **every API proto enum** to carry the new option product type (§0).
- **FIX** for execution via `destination` — new option dialect mappings.
- Distributor `notifyUsers` channel names — must be added/verified manually (no automated discovery).
- rkyv/internal Celnet wire formats stay internal to the Rust engine; only the Celer-facing boundary uses Protobuf/FIX/distributor.

---

## 3. Named services, repos & message types (summary catalog)

| Name | Role for Celnet | Change required |
|---|---|---|
| `baseserver` (distributor host, JVM) | In-proc bus host; ingress/egress gateway | JVM adapter or socket-protocol client |
| `DistributorProducerChannelHandler` | Distributor socket protocol entrypoint for non-JVM producers | Implement if not using JVM adapter |
| `marketdata` / `marketdata-api` | Spot (today); verify forward/NDF/vol | Possibly new vol feed handler |
| `MarketMerchantPriceService` | WS price streaming (in & out) | Resilient WS client; rate-limit |
| `marketmerchant` | Quote assembly | Accept option quotes |
| `orderrouting` | Order/quote routing | Verify hop to `risk` |
| `risk` | Exposure/Greeks | New option exposure model |
| `positionmanager` | Positions/netting | New netting keys |
| `destination` | FIX execution | New FIX dialect mappings |
| `clearing` | Post-trade | Verify hops |
| `staticdata` | Reference data/curves | New option reference data |
| `celertech-type` | Shared types | New option product type |
| `celnet-client` | Tenant config overlays (deploy-time) | Confirm overlay ownership w/ devops |
| API proto definitions | Protobuf enums/messages | Add option product type to **every** enum |
| Fenics FMD FXO 2.0 (external) | Vol surface + spot/fwd/NDF source | Build FMD ingest adapter |

---

## 4. Phased integration plan (non-disruptive)

The estate has **no option product type today**, so the strategy is to add Celnet as a net-new, opt-in producer/consumer that does not alter existing spot/FX flows until each stage is proven.

**Phase 0 — Decide the bus boundary & map the estate (no code in the hot path).**
- Decide JVM adapter vs `DistributorProducerChannelHandler` socket protocol for distributor connectivity.
- Manually build and review the cross-service dependency map (Spring config + distributor `notifyUsers` names); validate the four **inferred** hops.
- Confirm `marketdata-api` vol capability vs Fenics dependency; confirm `celnet-client` tenant overlay ownership.

**Phase 1 — Ingress, read-only.**
- Build the **Fenics FMD adapter** (ATM/RR/BF wings, spot, forward points, NDF fixings → canonical surface) and, if available, a Celer vol feed handler.
- Consume Celer spot/forward via the WS `MarketMerchantPriceService` and/or distributor as a **passive subscriber**. No publishing yet. Verify resilience to WS disconnects.

**Phase 2 — Pricing engine offline / shadow.**
- Run GK + VV/LSV pricing producing prices, full Greeks, and the constructed arbitrage-free surface **internally only**. Cross-validate against QuantLib golden tables. No estate egress.

**Phase 3 — Egress as a new, isolated product type.**
- Add the option product type to `celertech-type`, `staticdata`, **every API proto enum**, `risk` exposure models, `positionmanager` netting keys, and `destination` FIX dialect mappings — behind feature flags / tenant overlays so existing product flows are untouched.
- Publish prices/quotes/Greeks to `marketmerchant` and the webtrader over WS, **rate-limited** to respect the bounded distributor mailbox (avoid silent drops). Initially indicative-only.

**Phase 4 — Order routing, risk, booking, clearing.**
- Enable `orderrouting → risk → destination → clearing → positionmanager`, validating each (currently **inferred**) hop in a staging tenant before production.
- Turn on FIX execution via `destination`.

**Phase 5 — Frontend & desk tooling.**
- Wire the React webtrader to the live surface, Greeks, and scenario/stress feeds over WS; ship bespoke pre-trade/structuring/stress screens.

Throughout: tenant-overlay config (`celnet-client`) gates rollout per client so no existing service behavior changes until the option product is explicitly enabled.

---

## 5. Open questions to verify against the real services

1. **Distributor connectivity:** JVM adapter or `DistributorProducerChannelHandler` socket protocol — which, and what is the supported handshake/back-pressure contract?
2. **Mailbox sizing & rate limits:** what mailbox depth and producer rate keep a high-frequency option pricer below the skip-while-full threshold?
3. **Inferred hops:** confirm `orderrouting → risk`, `risk → destination`, `destination → clearing`, `clearing → positionmanager` against runtime traces, not just handler signatures/API deps.
4. **Vol data source:** can `marketdata-api` deliver vol surface (RR/BF/ATM) and forward/NDF data, or is Fenics FMD FXO 2.0 the sole vol source? Plan the additional feed handler accordingly.
5. **Fenics conventions:** what exact delta convention (spot vs forward, premium-adjusted vs unadjusted), ATM type (DNS vs forward), day-count and cut does FMD FXO 2.0 use? (Findings note FMD has no published convention spec.)
6. **Webtrader transport:** does the React webtrader consume gRPC-web, raw WS, or WS-carrying-Protobuf, and what is the auth/session model? How does the ~6-connection-per-domain semaphore affect a streaming surface + Greeks feed?
7. **`MarketMerchantPriceService` reconnection semantics:** subscription replay on reconnect, sequence/gap handling, and whether option quotes are a first-class price type there.
8. **Option product type plumbing:** the full list of API proto enums, `positionmanager` netting keys, `risk` exposure models, and `destination` FIX dialects that must change — confirm completeness.
9. **`celnet-client` tenant overlays:** who owns them, are they in scope for the default repo pull, and how is per-tenant option enablement gated?
10. **NDF/NDO settlement:** which fixing sources (EMTA/WMR/central-bank) and fixing-to-settlement lags does the estate already model, and where do `destination`/`clearing` expect them?
11. **`notifyUsers` channel names:** enumerate the exact distributor channel names for spot/forward in (ingress) and quotes/Greeks out (egress).
12. **Curve source:** which service supplies DF_d/DF_f discount curves, and in what message type?