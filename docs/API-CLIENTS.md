# Celnet — Trader-Centric API & Client SDK

> Design doc for the single current Celnet wire contract (`celnet-proto`), the async edge
> (`celnet-server`), and the typed Rust client SDK (`celnet-client`). It states what is
> *implemented and tested* today and what is *designed/deferred*, grounded in the actual
> `celnet.proto` and `celnet-client` surface (CLAUDE.md rule 10 — docs match code). There
> is exactly **one** current contract: no version field, no negotiation (rule 9).

---

## 1. What FX-options desks actually do (and what the API must model)

Desks operate around five workflow loops:

1. **RFQ** — discrete request → two-way quote → accept (last-look) → booking, including
   multi-leg strategies (risk reversal, strangle, straddle, seagull) quoted as one request.
2. **RFS** — subscribe-once, continuous two-way streaming prices+Greeks for a *defined*
   structure, with a bounded tradable window (Digital Vega Medusa streams competing RFS
   prices for up to ~5 minutes).
3. **Vol-surface marking** — calibrate at the main mark, re-mark intraday on material moves
   and central-bank events, working in broker conventions (ATM / 25Δ&10Δ RR&BF per tenor),
   with transparency into *why* each smile point is what it is.
4. **Risk / scenario / what-if** — spot/vol/rate shock grids on every pricing update.
5. **Position & P&L** with risk-factor attribution between marks.

The earlier broadcast WebSocket edge (one hard-coded reference strike fanned to all
subscribers, an echoed `uint64` correlation id) was a latency/lifecycle skeleton but not
trader-shaped. The current contract models loops 1–4 directly; loop 5 (position/P&L) is on the wire too —
the position book and hierarchical risk roll-up are served as `RiskService`
(`ListPositions`/`AggregateRisk`/`DrillRisk`/`LimitStatus`, §4), with only the per-mark
**P&L-explain** decomposition still designed-only (§7).

---

## 2. One contract, two transports

- **gRPC (primary, implemented):** `tonic` over the `celnet-proto` types — the low-latency
  programmatic path the modern market-maker wants. This is what `celnet-server` serves and
  what `celnet-client` consumes; it is exercised by the real-like trader-workflow tests.
- **WebSocket mirror:** a firewall-friendly transport that serializes the *same*
  stream messages as a tagged-JSON enum (`Snapshot`/`Update`/`Heartbeat`/`StreamEnd`), so a
  browser/light client gets identical snapshot+delta+resync semantics. The WS payloads are
  encoded/decoded from the same Rust types as gRPC — *one contract, two transports*. The JSON
  codec covers the Phase-1 additions too (broken-date/attribution/smile-model and the inbound
  `market_series_subscribe`/`market_series_unsubscribe` + outbound snapshot/point frames), so
  the GUI streams the market-series feed over the identical contract. gRPC remains the
  low-latency programmatic path; the browser GUI uses the WS mirror.

Message bodies mirror `celnet-types` one-to-one, and every identifier is purpose-named and
vendor-neutral (`VanillaInputs`, not `GkInputs`).

---

## 3. The Instrument model (one vocabulary for every workflow)

`Instrument` is a `oneof product` so every service speaks one vocabulary. **Implemented**
variants (in `celnet.proto`):

```
Instrument = Vanilla
           | Strategy(repeated Leg{ratio, side, vanilla})
           | SingleBarrier
           | DoubleBarrier
           | Digital
           | Touch            // one-touch / no-touch / double-no-touch / double-one-touch
           | VarianceSwap     // fair-variance-strike replication (K_var)
           | VolatilitySwap   // convexity-adjusted fair-vol strike (K_vol)
           | AsianOption      // fixed-strike arithmetic-average-rate (Curran / Turnbull-Wakeman)
           | ForwardStart     // strike resets at t1 to m·S(reset) (Rubinstein dual-carry)
           | Cliquet          // ratchet strip: plain closed-form, or clamped Monte-Carlo
           | Quanto           // vanilla / cash-or-nothing digital, settlement-ccy converted
           | Tarf             // target-redemption forward: geared fixing strip + KO target (MC)
           | Accumulator      // periodic pivot accumulation + up-and-out barrier (MC)
           | Lookback         // floating / fixed strike (closed-form or discrete MC)
           | WindowBarrier    // knock-out active only inside a calendar window (LSV PDE)
           | AmericanOption   // American / Bermudan early-exercise (free-boundary FD or LSM-MC)
           | BasketOption     // correlated multi-asset basket / best-of / worst-of (MC)
```

The wire field numbers are append-only (no renumber, no `schema_version`): `vanilla=7 …
touch=12`, then `variance_swap=13`, `volatility_swap=14`, `asian_option=15`,
`forward_start=16`, `cliquet=17`, `quanto=18`, `tarf=19`, `accumulator=20`, `lookback=21`,
`window_barrier=23`, `american=24`, `basket=25` (field `22` is the non-product
`pricing_model` selector — product tags never get renumbered, so a gap is left rather than
reused). This is the full **18-arm** product `oneof` (`crates/celnet-proto/proto/celnet.proto`
`oneof product`). Variance/vol swaps echo the fair strike in the priced `resolved_strike` (and
the server's `vol` field carries `√K_var` for the variance swap); the arithmetic Asian,
forward-start, plain cliquet, quanto vanilla/digital, continuous lookback and the
free-boundary-FD American carry a genuine discounted option price with the full FD Greek set.
The **Monte-Carlo products** — the clamped (locally-capped / -floored or globally-bounded)
cliquet, the TARF, the accumulator, the discrete lookback, the LSM American (`lsm_paths > 0`),
and the correlated basket — report an honest **price standard error** on the append-only
`PriceResponse.price_std_error` (`=7`, presence-tracked — absent for the closed-form
products), surfaced as `PricedLine.price_std_error` (SDK) and a `std_error` line (CLI); these
prices are MC estimates carrying that stderr, never "machine precision". SDK builders:
`InstrumentSpec::variance_swap`, `::volatility_swap`, `::asian_option(.., AsianTerms)`,
`::forward_start(.., ForwardStartTerms)`, `::cliquet(.., CliquetTerms)`,
`::quanto(.., QuantoTerms)`, `::tarf(..)`, `::accumulator(..)`, `::lookback(..)`,
`::window_barrier(..)`, `::american(..)`, `::basket(..)`
(`crates/celnet-client/src/vocab.rs`); CLI: `exotic var-swap`, `exotic vol-swap`,
`exotic asian`, `exotic forward-start`, `exotic cliquet`, `exotic quanto`, `exotic tarf`,
`exotic accumulator`, `exotic lookback`, `exotic window-barrier`, `exotic american`, and the
top-level `basket` subcommand (`crates/celnet-cli/src/cli.rs`).

Supporting messages: `Quantity` (notional + which leg-ccy), `Solve` (solve strike or premium
so a leg/structure is zero-cost), `StrikeOrDelta` (`oneof spec` — quote by strike or by
delta in the configured convention), `Conventions` and `MarketContext` carried so a price is
fully self-describing, `FixingSchedule` for path/fixing structures, `Greeks`, `TwoWayPrice`.

**Now shipped (previously deferred here):** the `Tarf` (target-redemption forward, field 19)
and `Accumulator` (field 20) variants — both with their full fixing-schedule +
target-redemption / pivot-accumulation wire models — are now on the contract and priced by
Monte-Carlo with a `price_std_error`, alongside `Lookback` (21), `WindowBarrier` (23, LSV
PDE), `AmericanOption` (24, free-boundary FD or LSM-MC) and `BasketOption` (25, correlated
multi-asset MC). The product `oneof` is complete against the in-repo `celnet-exotics`
catalogue; there are no instrument variants the engine prices that the wire cannot carry.

---

## 4. Services (the implemented contract)

| Service · RPC | Purpose |
|---|---|
| `PricingService.Price(PriceRequest) → PriceResponse` | One-shot price + full Greeks for an `Instrument` in a `MarketContext`. |
| `QuoteService.RequestQuote(QuoteRequest) → Quote` | RFQ: client-supplied `idempotency_key`, `Instrument`, side(s) (omit ⇒ two-way), quantity → server `quote_id`, `TwoWayPrice`, full Greeks, `valid_until` last-look deadline, resolved `Conventions`. |
| `QuoteService.AcceptQuote(QuoteAccept) → Execution` | Accept within the last-look window → booked `Execution`. |
| `QuoteService.RejectQuote(QuoteReject) → RejectAck` | Decline a live quote. |
| `StreamService.StreamSession(stream ClientStreamMessage) → stream ServerStreamMessage` | Multiplexed RFS bidi session carrying many subscriptions, each keyed on a client `SubscriptionId`. |
| `SurfaceService.GetSmile(GetSmileRequest) → Smile` | Smile on a delta axis with ATM/25Δ&10Δ RR/BF and an `ArbReport`. |
| `SurfaceService.MarkSurface(MarkSurfaceRequest) → MarkSurfaceResponse` | Calibrate from a `BrokerQuoteSet` + `Conventions`, with an optional `SmileModel` selector → a marked surface version. |
| `SurfaceService.Scenario(ScenarioRequest) → ScenarioResponse` | `ShockAxis` grid (spot/vol/rate, absolute or relative) → `ScenarioPoint`s. |
| `RiskService.ListPositions(ListPositionsRequest) → ListPositionsResponse` | The entitled open position book (each `RiskPosition` with its `OrgKey` + attribution), scope- and principal-pruned. |
| `RiskService.AggregateRisk(AggregateRiskRequest) → AggregateRiskResponse` | Server-side hierarchical roll-up over an org `RiskDimension` into a `RiskNode` tree (netted additive + re-derived non-additive measures) in a reporting numeraire. |
| `RiskService.DrillRisk(DrillRiskRequest) → DrillRiskResponse` | Drill one node into child sub-nodes at a finer dimension and/or its contributing positions (Book→Risk). |
| `RiskService.LimitStatus(LimitStatusRequest) → LimitStatusResponse` | Limit tree + per-limit utilization/RAG for a scope node, with the `hard_breach` escalation flag. |

### Risk — server-side hierarchical aggregation (implemented)

`RiskService` is the wire face of the single-node risk estate (`celnet-risk-cube` /
`celnet-risk-normalize` / `celnet-limits` / `celnet-entitlements`). **Aggregation is owned by the
server** — a client never loops positions and sums; it asks `AggregateRisk` for the rolled-up node
tree, `DrillRisk` to expand a node, `ListPositions` for the leaves, and `LimitStatus` for utilization.
The server prunes by the entitlement `principal` (default **grant-all**, show-all-now) **before**
roll-up so a node total never leaks an invisible subtree, groups by the org `RiskDimension`, sums the
additive measures + re-derives the non-additive ones (VaR/ES, FRTB curvature) per node by
bump-and-revalue, and collapses everything into a real `ReportingNumeraire` via `celnet-risk-normalize`
(resolving the GUI's prior "native premium units" caveat at the source). Served over **both** gRPC
(`risk_service_client`) and the WS mirror (`aggregate_risk`/`list_positions`/`drill_risk`/`limit_status`
frames) off the **same** shared live `PositionStore` the RFS click-to-trade path books vanilla fills
into. Honest scope: only vanilla fills become risk facts (an exotic has no canonical-vanilla leaf); a
`CCY_PAIR` limit scope is rejected loudly (a bare `u64` cannot reconstruct the pair). Full field-level
contract in `docs/INTERFACES.md` §"Phase-2 contract: `RiskService`".

### Smile-model selection on `MarkSurface` (implemented)

`MarkSurfaceRequest.smile_model` is an `optional SmileModel`
(`MARKET_HEDGE`/`STOCHASTIC_VOL`/`PARAMETRIC`/`PARAMETRIC_SURFACE`); absent ⇒ the market-hedge
(vanna-volga) default, reproduced byte-for-byte. The server routes the choice to the matching
`celnet-surface` calibrator (VV baseline / fitted SABR / SVI / SSVI — provenance in
`docs/ANALYTICS-SPEC.md` §3.4a; identifiers stay vendor/method-neutral), deposits the
**model-tagged** calibrated smile under the returned `surface_version` so a pinned RFQ/RFS
re-prices against the exact marked model, and **echoes the model used in `Smile.arbitrage.note`**
(`model=<family>`) as the honest provenance channel (the frozen contract has no dedicated echo
field). `ScenarioRequest.smile_model` (field 7) is validated identically; with no broker skew the
neutral smile collapses to the supplied flat `base_market.vol`, so the shock grid is
model-invariant unless a skewed surface is marked first and pinned via `surface_version`.

### Market-series feed — the TrendMode feed (implemented, multiplexed on `StreamSession`)

A labelled, unit-bearing time-series multiplexed on the same `StreamService.StreamSession` as RFS,
so a GUI/SDK/Excel client streams ATM-vol/spot/RR/BF/forward history over **one** session. Client
arms: `ClientStreamMessage.market_series_subscribe=7` / `market_series_unsubscribe=8`; server arms:
`ServerStreamMessage.market_series_snapshot=7` / `market_series_point=8`.

- `MarketSeriesSubscribe { SubscriptionId subscription; CcyPair pair; MarketObservable observable;
  optional Tenor tenor; optional double delta; uint64 throttle_nanos; uint32 history_limit }` —
  `MarketObservable ∈ { ATM_VOL, SPOT, RISK_REVERSAL, BUTTERFLY, FORWARD }`; `tenor` required for
  ATM_VOL/RR/BF/FORWARD, `delta` required for RR/BF.
- `MarketSeriesSnapshot { subscription; sequence; pair; observable; repeated MarketSeriesPoint;
  epoch_nanos }` then strictly-sequenced `MarketSeriesPoint { subscription; sequence; double value;
  int64 epoch_nanos }`s conflated to `throttle_nanos`.

**Server behaviour:** every value is derived **on the core thread from the live `MarketState`** the
hot path prices against (`CoreLink::observe`) — SPOT/FORWARD/ATM_VOL read directly; RR/BF invert the
live convention to the signed delta-wing strikes and read the **live smile** there. Never a
fabricated or recomputed proxy. A momentarily-underivable observable **skips** a point; a lagging
consumer **drops** a (conflatable) point rather than back-pressuring. The opening snapshot carries a
single live observation — there is **no durable observation store**, so the server does not backfill
fabricated history (a durable store that lets the snapshot replay a real recent window is a
documented future enhancement, not faked). This is the feed that unlocks every non-Premium GUI
`TrendMode` (`docs/EXPERIENCE-ARCHITECTURE.md` §7).

### Attribution identity — who's-trading (implemented, RFQ + RFS)

`optional AttributionRecord attribution` rides the whole quote/trade lifecycle:
`QuoteRequest`(6)→`Quote`(11)→`Execution`(7) and `Subscribe`(7)→`Snapshot`(12)→`Executed`(8).
`AttributionRecord { BookId quoted_by; optional BookId held_by; optional bool won; optional uint32
lp_count }`, `BookId { string book; Owner owner }`, `Owner { oneof seat { string trader; string
auto_pricer } }` — a human seat OR an automated pricer, uniformly.

**Server behaviour:** the shared resolver (`services::attribution`) makes every edge-quoted line
`quoted_by` the **maker auto-pricer** (`Owner::auto_pricer "celnet-auto-pricer"`, book `AUTO-MM`) —
engine-quoted flow is never anonymous — and a client-supplied requesting seat (its request
`quoted_by`) becomes `held_by`, carrying `won`/`lp_count` through verbatim. RFQ and RFS attribute
identically (API-first parity). The risk roll-up that *keys* on `AttributionRecord` (mapping it onto
the `celnet-risk-cube` org `OrgKey` dimensions) is **served** by `RiskService` — `celnet-server` maps
each booked fill's `AttributionRecord` book/seat onto the cube's interned `u32` org handles (§"Risk"
below and `docs/INTERFACES.md` §"Phase-2 contract: `RiskService`").

### RFQ idempotency (implemented)

The `idempotency_key` makes a retried `RequestQuote` safe: a second request with the same
key returns the **same** booked `Quote` (same `quote_id`, same prices) rather than
re-pricing or issuing a new id. Each issued quote stamps a publication time and a
`valid_until` last-look deadline (default 5 s, the typical OTC last-look window); an
`AcceptQuote` after `valid_until` is rejected. (Idempotency-key lifetime across a blue-green
cutover is the durable-store concern noted in §8.)

### RFS streaming (implemented)

`ClientStreamMessage = { Subscribe(SubscriptionId, Instrument, two-way) | Unsubscribe |
Resync(SubscriptionId, last_seq) | Heartbeat }`; `ServerStreamMessage = {
Snapshot(SubscriptionId, seq, two-way price+Greeks+vol) | Update(seq, changed fields) |
Heartbeat(seq) | StreamEnd(reason) }`. Per-subscription monotonic sequence + snapshot +
incremental `Update` + server-assisted `Resync` from `last_seq` fixes the gap-detection and
resync semantics the broadcast design lacked, and the draining/readiness behaviour refuses
new subscriptions while blue-green-draining. The multiplexed `StreamSession` driver carries
many subscriptions on one session (RFS, click-to-trade, and the market-series feed above) with
in-place `Modify` to change a live structure without re-subscribing.

---

## 5. The `celnet-client` SDK

A typed async Rust SDK over `tonic` + `tokio`, re-exporting the proto types so callers never
touch raw tonic. Implemented surface:

- `CelnetClient::connect(endpoint)` / `with_channel(channel)`.
- `price(instrument, ctx) → Greeks/price`.
- `request_quote(instrument, conventions) → Rfq`; `Rfq::request() → Quote`; `Rfq::accept(&quote,
  side) → Execution`; `Rfq::reject(...)`; `Rfq::idempotency_key()` (auto-generated, overridable).
- `subscribe(...) → RFS stream` with `next_event() → StreamEvent`, internally managing the
  `SubscriptionId`, applying snapshot+updates into a current two-way state, auto-heartbeat,
  and reconnect + `Resync` from `last_seq` on disconnect.
- `get_smile(...)`, `mark_surface(...)` / `mark_surface_with(...)` (optional `SmileModel`
  selector → `surface_version`), `scenario(...)` / `scenario_with_model(...)` /
  `scenario_with_risk(...)` / `scenario_with_risk_and_model(...)`.
- `list_positions(...)`, `aggregate_risk(...)`, `drill_risk(...)`, `limit_status(...)` — the
  typed `RiskService` surface (the SDK builders `Scope`, `Entitlements`, `Numeraire`,
  `AggregateQuery`, `DrillQuery`, `LimitQuery` live in `crates/celnet-client/src/risk.rs`).
- `open_session() → StreamSession` with `subscribe(...)` / `subscribe_attributed(...)` for RFS
  and **`subscribe_series(...)` for the typed market-series (TrendMode) feed** — yielding a
  `MarketSeries` stream with `next_event() → SeriesEvent` (`crates/celnet-client/src/series.rs`).
- New request fields (`smile_model`, `attribution`) default to `None` when unset, preserving the
  pre-Phase-1 call sites unchanged.
- Ergonomic builders: `InstrumentSpec::vanilla/strategy/unit`, the full exotic builder set
  (`single_barrier`/`double_barrier`/`digital`/`touch`/`variance_swap`/`volatility_swap`/
  `asian_option`/`forward_start`/`cliquet`/`quanto`/`tarf`/`accumulator`/`lookback`/
  `window_barrier`/`american`/`basket`), `Conventions::major_default()
  .with_delta/.with_atm/.with_premium`, `Quantity::base`, `StrikeSpec`, `BrokerQuoteSet::
  three_point/five_point`, `Smile::vol_at_delta/atm_vol/is_arbitrage_free`,
  `ShockAxis::relative/absolute`.

The SDK is validated by real-like end-to-end workflow integration tests that drive each loop
against a live edge; ergonomic friction in those tests is treated as an API-design defect to
fix in the proto, not worked around in the test ("evolve API by use").

---

## 6. Why this out-designs the incumbents

| Vendor | API shape | Gap Celnet's contract exploits |
|---|---|---|
| SynOption (Optimus) | FIX + UI + thin STP, seconds-scale RFQ | No typed microsecond streaming contract; closed Orion models (no SDK) |
| Fenics (FMD FXO 2.0 / kACE) | Best-in-class *data feed* (300+ pairs); kACE is a separate desktop tool | Data, not an interactive pricing API; under-documented conventions; no push/streaming, no callable arb-free surface object |
| Bloomberg (OVML/BVOL/MARS) | BLPAPI ticker/feed; MARS Python | Ticker/feed-oriented, not message/streaming-quote-oriented; closed models, seat-priced |
| 360T (Bridge) / Digital Vega (Medusa) | Venue/aggregation FIX networks | Strong workflow networks but outsource pricing to bank LPs; not an extensible quant engine |

Celnet's differentiator: a single typed gRPC contract carrying **conventions on every
message** (the transparency Fenics/Bloomberg lack), an RFQ lifecycle with caller idempotency,
per-subscription RFS with resync, and a callable arb-free surface object.

---

## 7. Designed but not yet on the wire

- **Position / P&L attribution** — a Greek-decomposed `AttributePnl(book, from_mark, to_mark)`
  (delta/gamma/vega/theta/vanna/volga explain) is not yet in `celnet-proto`. **Note this is no longer
  the whole risk lane:** the hierarchical-risk pipeline (canonical-leaf → cube → numeraire →
  limits/entitlements) **is** on the wire and **served** as `RiskService`
  (`ListPositions`/`AggregateRisk`/`DrillRisk`/`LimitStatus` — see §"Risk" and `docs/INTERFACES.md`),
  built on `celnet-risk-cube` / `-normalize` / `-limits` / `-entitlements`, with the **attribution
  identity** that keys the roll-up already on the wire (§4). What remains designed-only here is the
  per-mark **P&L-explain** decomposition (a distinct attribution report over two surface marks); the
  position book itself is queryable today via `ListPositions`.
- **`SmileModel`-dependent `Scenario`** — `Scenario` validates the selector but reprices off
  the supplied flat `base_market.vol`; a model-dependent shock grid needs a skewed surface
  marked first via `MarkSurface` + a pinned `surface_version` (the flat-base limitation is
  documented honestly, not stubbed).
- **Durable market-series history** — the market-series snapshot (§4) carries one live
  observation; a durable observation store that replays a real recent window is a future
  enhancement (the server does not fabricate backfill).

These are tracked as the continuing API-evolution wave; they are listed here so the contract's
scope is not over-read. **Now shipped (previously listed here):** the **market-series feed**
(§4) and its **typed `subscribe_series` SDK helper** (§5), **smile-model selection** on
`MarkSurface` (§4), **attribution identity** across the RFQ/RFS lifecycle (§4), in-place RFS
**`Modify`** (§4), the **WebSocket JSON-mirror** for these frames (§2), and the full exotic /
structured / multi-asset product catalogue on the `Instrument` `oneof` — including the
**`Tarf`** and **`Accumulator`** variants once deferred here (§3).

---

## 8. Risks

1. **Idempotency across cutover** — the dedupe store must survive a blue-green cutover or a
   retry during upgrade could double-book; v1 keeps RFQ quotes self-describing and bounded by
   `valid_until`, and the audit ring (see `docs/OBSERVABILITY.md` §6) is lossless.
2. **Interface-freeze blast radius** — the `Instrument` oneof ripples into
   `celnet-vanilla`/`celnet-exotics` call sites; changes are sequenced as a single
   coordinated interface edit + re-index per `docs/INTERFACES.md`.
3. **Per-subscription streaming cost** — at IB-scale fan-out this needs instrument-dedup,
   throttle hints and delta conflation (see `docs/SCALE-OUT.md` §5), designed in from day one.
4. **Two-way / last-look / quote-expiry correctness** — exactly the convention traps Celnet
   claims to win on; tested against worked examples, not asserted plausible (rule 5).
5. **WS/gRPC drift** — the WS mirror, when built, must serialize the same Rust types so the
   "one contract, two transports" claim holds.

---

*Sources: `docs/_research/api-obs-scale.json` (trader-API topic);
`crates/celnet-proto/proto/celnet.proto`; `crates/celnet-client/src`;
`crates/celnet-server/src/services`; `docs/COMPETITIVE-ANALYSIS.md`. Implemented vs deferred
split reconciled against the real contract.*
