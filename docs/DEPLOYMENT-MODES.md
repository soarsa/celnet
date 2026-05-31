# Celnet — Deployment Modes (Standalone / Celer-Integrated / Hybrid)

> **Purpose.** Celnet is the FX-**options** pricing platform. The parent **Celer** estate
> (`celertech-*`, ~113 repos indexed in codebase-memory) has *no option product type today*
> and prices FX **spot/forward** through a JVM, disruptor-based price/order path. This document
> specifies the **three deployment topologies** in which Celnet ships, and for each: which Celer
> services Celnet **replaces**, **consumes**, and **publishes-to**; the **adapter** each mode
> needs (`MarketDataSource` / `PriceSink` / order+exec path); how Celnet remains the **system of
> record (SoR) for FX-options pricing** in all three; the **process/JVM topology** (what runs
> where); and the **migration path** Standalone → Hybrid → Integrated.
>
> **Sibling docs.** `docs/CELER-INTEGRATION.md` is the integration *map*; `docs/CELER-FIX-
> INTEGRATION-PLAN.md` is the engineering *plan* (crates, ADRs, build-vs-defer). This doc is the
> *topology* layer that selects among them. It does not change any guardrail.
>
> **Evidence convention.** Facts traced through the codebase-memory graph or the `~/wiki/`
> narrative wikis are tagged **[graph]** / **[wiki]** with the symbol/file. Items I could **not**
> runtime-trace (the estate uses an in-proc JVM disruptor + Protobuf + FIX, so cross-service
> edges are invisible to static tooling — confirmed: `codebase-memory` finds zero cross-service
> edges) are tagged **[inferred — live-staging gate]** and must be verified in a staging tenant
> before being relied upon. I never fake the far side to claim completeness (guardrail #2).

---

## 0. Grounding: what the Celer price/order path actually is

Reconstructed from the graph + wikis (`~/wiki/celertech-marketmerchant`, `-marketdata`,
`-destination`, `-positionmanager`, and `~/wiki/celertech-devops/concepts/trade-lifecycle`):

**Price path** [wiki + graph]:
external venues → **FIX in** → `celertech-marketdata` `MarketDataManager.subscribe()` [graph:
`com.celertech.marketdata.connectivity.MarketDataManager.subscribe`, with venue destinations
`BloombergSAPIMarketDataDestination`, `CurrenexMarketDataDestination`, FastMatch, HotSpot under
`MarketDataDestinationManager`] → `celertech-marketmerchant`
`NonRingBufferBasedMarketMerchantSession` (the quote-assembly DAG) → publish via the in-proc
`celertech-distributor` disruptor → frontend `MarketMerchantPriceServiceClient` (**WS-only, no
fallback**) [wiki].

**Order path** [wiki, with inferred hops]:
frontend `OrderServiceClient` (gRPC-over-WebSocket) → `celertech-orderrouting`
(`OrderManagerImpl.onCreateOrderRequest`) → pre-trade `celertech-risk`
(`RiskCheckFxOrderRequestHandler`) **[inferred hop]** → `celertech-destination`
(`ConfigurableFixOrderRoutingDestination` + 20+ venue adapters; the FIX edge is
`AcceptorFixEngineAdapter` / `InitiatorFixEngineAdapter` / `FixEngineAdapter.sendFixMessage`
[graph]) → **FIX out** to LP/venue → `ExecutionReport (35=8)` back → `celertech-clearing`
**[inferred]** → `celertech-positionmanager`
(`NetPositionManager.handleTransactionDownstreamEvent` [graph]) → `NetPositionDisseminator` → WS
push to frontend.

**Three load-bearing facts that constrain every mode:**

1. **The distributor is an in-process JVM disruptor mailbox** (in `celertech-baseserver`) with
   **silent skip-while-full back-pressure** [wiki + graph]. `marketmerchant` publishes assembled
   prices via `NonRingBufferBasedMarketMerchantSession.publishPriceEventOrSkipWhileFull()`
   [graph: `…session.NonRingBufferBasedMarketMerchantSession.publishPriceEventOrSkipWhileFull`,
   lines 552–594]. A Rust process **cannot natively join a JVM disruptor**; any
   Integrated/Hybrid publish-to-distributor needs a JVM sidecar or a reverse-engineered socket
   client. See §6 (marketmerchant findings) for the exact conflation semantics.
2. **There is no `FX_OPTION` product type anywhere in the estate** [wiki]. `ProductType` is a
   **netting-key component** in `positionmanager` and an enum across every `*-api` proto. Adding
   it is a broad, append-only, cross-cutting change — concentrated and feature-flagged per
   `docs/CELER-FIX-INTEGRATION-PLAN.md` §3.
3. **Four lifecycle hops are inferred, not traced** (orderrouting→risk, risk→destination,
   destination→clearing, clearing→positionmanager). Every mode element that crosses one is
   **[inferred — live-staging gate]**.

---

## 1. The three adapter seams (identical traits across all modes)

Celnet defines **three internal trait seams** so the *engine never changes* between modes —
only which adapter is bound at deploy time changes. This is the whole point of the design: the
pinned zero-alloc hot core (`celnet-engine`) is mode-agnostic; modes are an **edge** (async,
`celnet-server` / `celnet-integration`) concern.

| Seam | Trait (celnet-side) | Standalone impl | Hybrid impl | Integrated impl |
|---|---|---|---|---|
| **`MarketDataSource`** (ingress: spot, fwd pts, NDF fixings, **vol surface** ATM/RR/BF, curves) | `celnet-integration::MarketDataSource` | direct external vol/FX feed adapter (FMD-FXO-style WS/stream client) + synthetic/file curves | external vol feed **+** a *read* subscription to Celer `marketdata`/`MarketMerchantPriceService` for spot/fwd | Celer `marketdata` distributor subscription (via JVM sidecar) as primary; external vol feed where the estate has no vol |
| **`PriceSink`** (egress: option prices/quotes, full Greek set, surface, risk/PnL) | `celnet-server` native edge (gRPC-over-WS + FIX acceptor) to **Celnet's own clients** | native edge **+** Celer egress for the subset of consumers that opt in | Celer egress (`marketmerchant` quote stream + distributor) as primary; native edge retained for direct/desk clients |
| **order+exec path** (RFQ/RFS → quote → order → ExecutionReport; delta-hedge initiator) | `celnet-fix` acceptor+initiator to Celnet's own/counterparty FIX | `celnet-fix` acceptor for option RFQ; hedge via `celnet-fix` initiator to venues directly | option order routed through Celer `orderrouting`→`destination`→`clearing`→`positionmanager` with `FX_OPTION` product type |

These three traits are the **only** things that vary by mode. The `DistributorEgress` trait and
`EgressGovernor` (bounded conflating governor, `docs/CELER-FIX-INTEGRATION-PLAN.md` §2.3) are the
concrete `PriceSink` adapter used whenever the sink is the Celer distributor — in Hybrid and
Integrated, never in Standalone.

### 1.1 Fleet topology — an orthogonal deploy-time knob (same bind-at-deploy pattern)

Independent of which Celer mode is selected, Celnet's **horizontal-scale topology** is the same
kind of deploy-time-bound seam (`docs/SCALE-OUT.md` §0). A single config knob —
`celnet_risk_fleet::FleetTopology`, resolved at `Edge` boot from `CELNET_FLEET_MODE`
(`in-process` | `distributed`) + `CELNET_FLEET_BACKENDS` — selects:

| Topology | When | Behavior |
|---|---|---|
| **`InProcess`** (**default**) | single node holds the book (laptop, single-tenant, tests, and the common production case) | one process; the RiskService aggregates locally; **byte-identical to the single-node edge** — zero overhead, zero config. |
| **`Distributed { endpoints }`** | the book exceeds one process / horizontal scale-out is wanted | the **stateless edge** is a *client of the same `RiskService` it serves*, federating across N backend `celnet-server` processes over real gRPC: **federates** risk (additive wire-merge + non-additive constituent re-gather, reconciled `fan-out == single-node`) and **forwards** owned-pair Pricing/Quote/Surface by health-aware HRW route; `unavailable` on an unreachable slice. |

The engine, the wire contract, and every client are **mode-agnostic** (a client cannot tell which
topology served it) — exactly the §1 invariant, one layer up. In-process is the safe default;
out-of-process is opt-in for deployments where a single process is not enough. Cross-**DC**
transport hardening, the replicated log, and hot-standby pre-warm remain the deferred tier
(`docs/SCALE-OUT.md` §12).

---

## 2. Mode A — STANDALONE

**Definition.** Celnet runs as a self-contained FX-options pricing/quoting platform with **no
Celer estate dependency**. It is the full SoR: pricing, surface construction, the quote venue,
and execution capture for options it quotes.

### Replaces / Consumes / Publishes-to

| Verb | Celer service | Standalone behavior |
|---|---|---|
| **Replaces** | `marketmerchant` (quote assembly), `marketdata` (as the option-pricing feed hub), the distributor fan-out, `destination` (FIX edge), the frontend price/order WS edge | Celnet performs all of these *for options* itself: `celnet-server` is the WS/gRPC + FIX edge; `celnet-engine` + `celnet-surface` are the pricing/surface engine. |
| **Consumes** | *none of Celer* | Only **external** market data via `MarketDataSource`: a vol-surface feed (ATM + 25Δ/10Δ RR/BF, spot, fwd points, NDF fixings) normalized by `celnet-integration::normalize` into the canonical surface; curves from a file/external curve adapter. |
| **Publishes-to** | *none of Celer* | Celnet's own SDK clients / GUI / counterparty FIX peers, via the native `PriceSink` (gRPC-over-WS multiplex `StreamSession` + `celnet-fix` acceptor). |

### Adapters bound

- `MarketDataSource` = **external feed adapter only** (REAL-buildable here against a real loopback
  WS server replaying recorded vendor frames → `VendorSmileMessage` → existing `pipeline`).
- `PriceSink` = **native Celnet edge** (`celnet-server` `StreamService.StreamSession`, click-to-trade,
  `surface_book`/`surface_version`).
- order+exec = **`celnet-fix` acceptor** (RFQ/RFS → Quote/MassQuote → NewOrderSingle/Multileg →
  ExecutionReport, last-look via QuoteID) + **`celnet-fix` initiator** for delta-hedge to venues.

### Process / JVM topology

- **Pure Rust. No JVM.** Processes: `celnet-server` (edge), the `celnet-engine` core (pinned,
  zero-alloc), `celnet-plugin-host` (Tier-0/Tier-2), optional `celnet-gpu` workers. Horizontal
  scale-out per `docs/SCALE-OUT.md`. No `celertech-baseserver`, no distributor, no sidecar.

### SoR statement

Celnet is **trivially** the SoR for FX-options pricing — it is the only pricer present. The
surface registry (`surface_book` with monotonic `surface_version`) is the canonical mark
store; every price/quote/Greek echoes `surface_version` + `correlation_id` for auditability.

**Status:** fully buildable + provable **here, now** — no live-staging gate. This is the GA
target of the current crate set (per the implementation ledger).

---

## 3. Mode C — CELER-INTEGRATED

**Definition.** Celnet is the FX-options **brain embedded inside the Celer trade lifecycle**:
options flow through the *same* `marketmerchant` quote stream, `orderrouting`→`destination`→
`clearing`→`positionmanager` order path, `risk` checks, and the React webtrader that the estate
uses for spot/forward — but every option *price, Greek, and surface* originates in Celnet.

### Replaces / Consumes / Publishes-to

| Verb | Celer service | Integrated behavior |
|---|---|---|
| **Replaces** | the **pricing logic** for options *inside* `marketmerchant` (Celnet is the option price source feeding the assembly DAG); any legacy/absent option analytics in `risk` | Celnet does **not** replace the *service*; it replaces the *option-pricing function* the service would otherwise lack. `marketmerchant` keeps owning quote-assembly/credit/business-rule DAG; Celnet supplies the option price events it assembles. |
| **Consumes** | `marketdata` (spot/fwd via distributor, through the JVM sidecar) [inferred — vol capability gate], `staticdata` (option reference data once `FX_OPTION` exists) [live-gate], a curve service (DF_d/DF_f) [live-gate] | `MarketDataSource` = distributor subscription (sidecar) for spot/fwd + external vol feed where Celer has no vol surface. |
| **Publishes-to** | `marketmerchant` (option price events → assembly → `publishPriceEventOrSkipWhileFull` fan-out), `risk` (Greeks/exposure for the option arm), `positionmanager` (booking events under a new option netting key), `destination` (FIX dialect for option exec), the React webtrader (option ticket + surface/Greeks panels) | `PriceSink` = `DistributorEgress` (JVM sidecar + `EgressGovernor`). order+exec = the estate path with `FX_OPTION` plumbed end-to-end. |

### Adapters bound

- `MarketDataSource` = **distributor ingress** (sidecar) + external vol feed.
- `PriceSink` = **`DistributorEgress` (JVM sidecar)** fronted by the **`EgressGovernor`** (bounded
  ring + per-`(pair,tenor,strike)` conflation + token bucket + **counted** drops) so a µs pricer
  never triggers `marketmerchant`'s silent `publishPriceEventOrSkipWhileFull` skip (§6).
- order+exec = the estate path: option `NewOrderSingle/Multileg` → `orderrouting` →
  `risk` **[inferred]** → `destination` (the `celnet-fix::dialect_fx` tag set is the *reference
  spec* `destination` implements) → venue FIX → `ExecutionReport` → `clearing` **[inferred]** →
  `positionmanager` (new option netting key, §4).

### Process / JVM topology

- **Rust engine + JVM estate side-by-side**, bridged by the **JVM distributor sidecar** (the only
  protocol-correct way for Rust to touch the in-proc disruptor; native socket client is a deferred
  benchmark-gated optimization — `docs/CELER-FIX-INTEGRATION-PLAN.md` §2.2). Topology: `celnet-engine`
  + `celnet-server` (Rust) ↔ **distributor sidecar** (thin JVM embedding the real `baseserver`
  distributor client, exposing a UDS/TCP length-prefixed Protobuf seam) ↔ `celertech-baseserver` /
  `marketmerchant` / `marketdata` (JVM) ↔ `orderrouting`/`risk`/`destination`/`clearing`/
  `positionmanager` (JVM). Per-tenant enablement via `celnet-client` overlays [live-gate].

### SoR statement

**Celnet remains the SoR for FX-options pricing even inside Celer.** The estate services *carry*
and *assemble* Celnet's option prices but never *recompute* them: `marketmerchant`'s DAG applies
credit/tiering/business rules on top of a Celnet-originated option price event; `risk` consumes
Celnet-exported Greeks; `positionmanager` books what Celnet quoted. Every option price event
carries Celnet's `surface_version` + `correlation_id` so the estate's records are attributable
back to the exact Celnet surface that produced them. The **option mark/surface registry lives in
Celnet**, not in the estate.

**Status:** the Celnet-side seams (governor, dialect spec, proto projections, sidecar client
trait) are **REAL-buildable here**; everything crossing into a JVM process — sidecar handshake +
mailbox calibration, `FX_OPTION` enum across `*-api`/`staticdata`/`celertech-type`, the netting
key + risk exposure edits, the 4 inferred hops, live `MarketMerchantPriceService` + feed
entitlement, tenant overlays — is **[inferred / live-staging gate]** (D1–D10 in the FIX plan).

---

## 4. Option netting key — concrete grounding (Integrated/Hybrid booking)

The positionmanager netting key is **traced** [graph]:
`NetPositionManager.buildKey` (lines 1345–1370) reads from the `GeneralLedgerTransactionEvent` +
`TransactionLeg` and calls `netPositionKeyFactory.create(account, securityId, securityCode,
assetType, productType, settlementType, settlementDate, trader)` — i.e. the live netting identity
is **(account, securityId, securityCode, assetType, ProductType, settlementType, settlementDate,
trader)**, with key variants `NonSettlementDateProductTypeNetPositionKey` and
`SettlementTypeNetPositionKey` selected by a factory [graph].

For options this is **insufficient** — `securityId`/`securityCode` must encode the option's full
identity. Celnet's reference position-key logic nets options by **(account, ProductType=FX_OPTION,
pair, strike, expiry, call/put, exerciseStyle, settlementType, settlementDate, trader)**: strike
+ expiry + put/call + style are part of the option's identity, unlike a linear spot/forward leg.
The Celnet-side key + delta-equivalent/vega exposure math is unit-provable here (FIX plan B11);
wiring it into `positionmanager`'s `netPositionKeyFactory` (a new key variant gated on
`ProductType==FX_OPTION`, leaving the spot/fwd key path byte-unchanged) is **[live-gate D4]**.

---

## 5. Mode B — HYBRID (the migration midpoint and a first-class steady state)

**Definition.** Celnet is the **option SoR and quote venue on its own native edge**, *and*
selectively bridges into Celer for the consumers that need estate integration — typically:
**consume** Celer spot/forward for surface inputs, **publish** indicative option prices into
`marketmerchant` for desk visibility, while keeping **firm RFS execution and booking on Celnet's
own FIX edge** until the estate's `FX_OPTION` plumbing is proven.

### Replaces / Consumes / Publishes-to

| Verb | Celer service | Hybrid behavior |
|---|---|---|
| **Replaces** | the option quote venue + execution + surface (kept on Celnet's native edge) | Celnet's `celnet-server` + `celnet-fix` stay the firm-quote/exec path; the estate is *not* in the critical execution loop yet. |
| **Consumes** | `marketdata` / `MarketMerchantPriceService` (WS-only) for **spot/forward** [wiki — resilient WS subscriber: subscribe→snapshot→sequenced deltas→on-gap full-resync, ≤6 conn/domain multiplex], external vol feed for the **surface** | `MarketDataSource` = external vol feed (primary for vol) + Celer spot/fwd read subscription. |
| **Publishes-to** | `marketmerchant` (**indicative** option prices only, rate-limited + conflated via `EgressGovernor`), the React webtrader (read-only surface/Greeks panels) [live-gate] | `PriceSink` = native edge (firm) **+** `DistributorEgress` (indicative, governed). Booking stays Celnet-internal until D3/D4 land. |

### Adapters bound

- `MarketDataSource` = external vol feed **+** resilient Celer WS subscriber (REAL-buildable here
  against a real loopback WS server replaying recorded frames; live endpoint is a gate).
- `PriceSink` = **native edge (firm/tradeable)** + **governed `DistributorEgress` (indicative)**.
- order+exec = **`celnet-fix` acceptor/initiator** (firm RFS + hedge stay on Celnet's edge); the
  estate order path is **not** yet wired (that is the Hybrid→Integrated step).

### Process / JVM topology

- **Rust edge live + selective JVM sidecar for ingress/indicative-egress only.** The sidecar is
  present (for `marketdata` distributor ingress and indicative egress to `marketmerchant`) but the
  estate is *out of the firm-execution loop*. This is exactly the topology where the bounded
  conflating governor earns its keep: indicative option prices are throttled to the measured
  distributor drain rate so they never cause `skip-while-full` drops, while firm quotes bypass the
  JVM entirely on the native edge.

### SoR statement

Celnet is unambiguously the option SoR: it *originates* every price, *owns* the surface registry,
and *executes + books firm trades itself*. Celer sees a **read-only, indicative projection** of
Celnet's option prices for desk visibility — never the authoritative mark. The migration to
Integrated only *moves where firm execution/booking happen*; it never moves *who computes the
option price*.

**Status:** Celnet-side fully **REAL-buildable here**; the Celer read subscription + indicative
publish are **[live-gate D8/D6]**.

---

## 6. marketmerchant — key findings (the throughput/back-pressure bottleneck)

`celertech-marketmerchant` is the largest estate service (60,164 graph nodes / 203,079 edges) and
the **logic + throughput bottleneck of the price path** [wiki index]. Concrete findings that
shape Celnet's `PriceSink`:

1. **`NonRingBufferBasedMarketMerchantSession` is the quote-streaming session** [graph], holding a
   `DistributorProducerClientHelper` and publishing assembled price events to the in-proc
   distributor. It runs a **disruptor ring buffer** (`celerDisruptor.publishEvent(...)`) plus a
   secondary **non-ring-buffer** path for reference (non-executable) prices.
2. **`publishPriceEventOrSkipWhileFull` (lines 552–594) is the back-pressure seam** [graph]. The
   traced logic is precise and important: when the non-ring buffer is full it logs at most once a
   minute ("may need to increase the ring buffer size"); if the call comes *from the non-ring-
   buffer thread itself* it **drops the event** (`cleanConflatingMsg` + `return`) to avoid a
   self-deadlock; when `skipPriceForNonRingBufferWhenFull` is set, a **reference price is skipped**
   (conflated away) rather than blocking the *executable* pricing ring buffer — "the price for the
   non ring buffer should be a non-executed reference price … skip … to avoid blocking the price
   into the executable pricing ring buffer." Otherwise the event is put back into the ring buffer.
   **Takeaway:** the estate already *prioritizes executable prices and silently conflates/drops
   reference prices under load* — the drop is by design and **unobserved by the producer**.
3. **`cleanConflatingMsg` (596–621) + `markConflatedMessage` (522–525)** show marketmerchant does
   **per-message conflation** internally. Celnet's `EgressGovernor` must therefore conflate
   *before* the sidecar (newest-per-`(pair,tenor,strike)`) so the bytes that reach
   `publishPriceEventOrSkipWhileFull` are already the freshest — turning the estate's *silent*
   skip into Celnet's *counted, observable* conflation upstream.
4. **Two distinct egress channels exist:** the **price** path
   (`NonRingBufferBasedMarketMerchantSession` → distributor → `MarketMerchantPriceService` WS) and
   a separate **quote-notification** path (`PeerQuoteNotificationManager.notifyUsers`, 135–146
   [graph]) for peer/RFQ quote dissemination. Celnet's indicative-price egress targets the former;
   firm-quote/RFQ responses (if ever bridged) map to the latter.
5. **`MarketMerchantPriceService` is WS-only with no fallback** and a ~6-concurrent-connection-per-
   domain semaphore [wiki/CELER-INTEGRATION] — so the Celnet ingress subscriber (Hybrid/Integrated)
   must multiplex many pairs over ≤6 sockets and resync on gap.

**Net effect on Celnet:** Celnet's egress to marketmerchant must (a) conflate newest-per-key
*before* the JVM boundary, (b) rate-limit to the **measured** ring-buffer drain rate
(calibration is **[live-gate D1]** — the safe ring size / drain rate is not knowable from source),
and (c) **count every drop** (HdrHistogram + metric) so the estate's silent skip becomes an
observable Celnet SLO. The governor design in `docs/CELER-FIX-INTEGRATION-PLAN.md` §2.3 implements
exactly this and is provable here against a real rate-limited socket sink.

---

## 7. Migration path: Standalone → Hybrid → Integrated

The seam design makes migration a **sequence of adapter swaps**, never an engine rewrite. Each
step is independently shippable and reversible (flip the adapter back).

```
STANDALONE                         HYBRID                              INTEGRATED
─────────                          ──────                              ──────────
MarketDataSource = ext feed   →    + Celer spot/fwd WS read sub   →    Celer marketdata (sidecar)
                                                                        primary; ext vol where
                                                                        estate lacks vol
PriceSink = native edge       →    native (firm) + governed       →    governed DistributorEgress
                                   DistributorEgress (indicative)      primary; native edge for
                                                                        direct/desk clients
order+exec = celnet-fix       →    celnet-fix (firm + hedge);     →    estate path: orderrouting→
acceptor+initiator                 estate out of exec loop             risk→destination→clearing→
                                                                        positionmanager w/ FX_OPTION
JVM: none                     →    sidecar (ingress + indicative) →    sidecar (full price + order
                                                                        bridge)
```

**Step S→H (consume + project):**
1. Stand up the JVM distributor sidecar **read-only** (ingress) + the resilient WS subscriber to
   `marketdata`/`MarketMerchantPriceService`; verify resync/connection-economy in staging
   **[live-gate D8]**.
2. Bind the **governed `DistributorEgress` (indicative only)** alongside the native firm edge;
   calibrate the drain rate against the real ring buffer **[live-gate D1]**; prove zero
   `skip-while-full` under the calibrated budget. Firm execution/booking stay on Celnet. Reversible.

**Step H→I (book + execute in the estate):**
3. Land `FX_OPTION` append-only across `celertech-type`/`staticdata`/every `*-api` proto enum
   (wire-safe, no renumber) **[live-gate D3]**; add the option netting-key variant (§4) +
   `risk` option exposure arm **[live-gate D4]**; implement the `destination` FX-options FIX
   dialect from `celnet-fix::dialect_fx` as spec + venue certification **[live-gate D5]**; add the
   `marketmerchant` DAG option-quote branch **[live-gate D6]**.
4. Validate the **4 inferred hops** (orderrouting→risk, risk→destination, destination→clearing,
   clearing→positionmanager) with real staging traces **[live-gate D7]** — never inferred.
5. Switch the order+exec adapter from `celnet-fix` to the estate path; switch `PriceSink` primary
   from native to governed-distributor (native edge retained for direct/desk clients). Per-tenant
   via `celnet-client` overlays **[live-gate D9]**; webtrader option ticket + panels **[live-gate
   D10]**.

**Invariant across all steps — Celnet stays the option pricing SoR.** No migration step ever
moves *option price/surface computation* out of Celnet. Migration only changes *where prices are
carried, executed, and booked*. The `surface_version` + `correlation_id` stamped on every output
guarantees that whatever the estate records is attributable to the exact Celnet surface — in all
three modes.

---

## 8. Open questions (carry forward to live-staging; mirror of CELER-INTEGRATION §5)

All marked **[inferred — live-staging gate]**: (1) distributor sidecar handshake + back-pressure
contract; (2) marketmerchant ring-buffer depth + drain rate calibration (the safe `EgressGovernor`
budget); (3) the 4 inferred lifecycle hops; (4) whether `marketdata-api` delivers vol (not just
spot) or external vol feed is the sole vol source; (5) external vol feed convention spec
(delta/ATM/day-count/cut); (6) webtrader transport (gRPC-web vs raw WS) + auth + the 6-conn
semaphore impact on a streaming surface+Greeks feed; (7) `MarketMerchantPriceService` reconnect /
sequence-gap / option-as-first-class-price semantics; (8) completeness of the `FX_OPTION`
proto/netting/exposure/dialect touch-list; (9) `celnet-client` tenant-overlay ownership;
(10) NDF/NDO fixing sources + settlement lags expected by `destination`/`clearing`;
(11) exact distributor `notifyUsers` channel names for option ingress/egress;
(12) curve service supplying DF_d/DF_f and its message type.
