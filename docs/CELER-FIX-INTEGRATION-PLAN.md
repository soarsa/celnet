# Celnet — Celer + FIX Integration Build Plan

> **No mocks (binding).** Nothing in the Celnet product is mocked or stubbed: the FIX engine
> (acceptor AND initiator), the dialect, the session FSM, the distributor egress governor, and
> the market-data ingress are complete, gate-checked implementations. Tests drive them against
> REAL protocol peers — our own initiator vs our own acceptor over a loopback socket; a real
> rate-limited socket sink; a real loopback WS server replaying recorded frames — never against
> fakes of our own functionality. The ONLY thing not done in this environment is connecting to
> the live deployed Celer JVM processes (separate `celertech-*` repos, not running here): that is
> a deployment/credentials gate, not a mock — the same contract test re-runs unchanged against
> the real far side in staging.

> Status: **buildable plan** for the next integration wave (WS-H/WS-I follow-on). Supersedes
> nothing in `docs/CELER-INTEGRATION.md` (the integration *map*) — this is the *engineering
> plan* that turns that map into crates, ADRs, and a build-vs-defer task list with a validation
> gate per item.
>
> **Cross-checked against the live Celer estate wikis** under `~/wiki/` (celertech-destination,
> -orderrouting, -marketmerchant, -marketdata, -positionmanager, -risk, -devops). Facts drawn
> from those wikis are tagged **[wiki]**; items the wiki itself marks *inferred* (not runtime-
> traced) are tagged **[inferred]** and gate on live-staging verification. Everything else is a
> Celnet-side design decision we own.
>
> **The live-estate boundary is honest and explicit (see §7).** Everything in §1–§5 marked
> *REAL-buildable-here* can be built and proven *here, now*, against a real loopback FIX peer (our own acceptor+initiator over a socket, no stub) and
> a real rate-limited socket sink, with no access to a running Celer estate. Everything marked
> *LIVE-GATED* cannot be finished without a staging tenant and is deferred behind a deployment
> gate — we build the seam and the contract test, not a fake of the far side.

---

## 0. What the estate actually is (grounding facts from the wikis)

The Celer trade lifecycle, reconstructed from `~/wiki/celertech-devops/concepts/trade-lifecycle`:

**Order path** [wiki]: frontend `OrderServiceClient` (gRPC-over-WebSocket, `nice-grpc-web`,
Netty `/stream`, `authorization-token` metadata) → `celertech-orderrouting`
(`CreateFxOrderRequestHandler` → `OrderManagerImpl.onCreateOrderRequest`, ~4000 LOC, the filter
chain) → pre-trade `celertech-risk` (`RiskCheckFxOrderRequestHandler.handleCepEvent` →
`RiskCheckManager`) **[inferred hop]** → `celertech-destination`
(`ConfigurableFixOrderRoutingDestination` + 20+ venue adapters) → **FIX out** to LP/venue →
`ExecutionReport` (FIX `35=8`) back → `celertech-clearing`
(`CreateTradeCaptureReportRequestHandler`) **[inferred]** → `celertech-positionmanager`
(`NetPositionManager.handleTransactionDownstreamEvent`, netting key **(ProductType,
SettlementDate, SettlementType)**) → `NetPositionDisseminator` → WS push to frontend.

**Price path** [wiki]: external venues → **FIX in** → `celertech-marketdata`
(`MarketDataManager`, BBG SAPI / Currenex / FastMatch / HotSpot) → `celertech-marketmerchant`
(`NonRingBufferBasedMarketMerchantSession`, the quote-assembly DAG,
`publishPriceEventOrSkipWhileFull()`) **[inferred]** → `celertech-marketwarehouse` → in-proc
`celertech-distributor` (`DistributorProducerClientHelper.notifyUsers(name, proto)`) → frontend
`MarketMerchantPriceServiceClient` (**WS-only, no fallback**).

**Three load-bearing constraints that shape every decision below:**

1. **The distributor is an in-process JVM disruptor mailbox** living in `celertech-baseserver`
   [wiki]. A Rust process cannot natively join a JVM disruptor. The publish call is literally
   `publishPriceEventOrSkipWhileFull()` — **back-pressure is silent skip-while-full** [wiki].
   A microsecond pricer overruns it trivially. This is the single highest-risk seam and gets a
   dedicated ADR (§2).
2. **There is no option product type anywhere in the estate** [wiki]. `ProductType` is a netting
   key in positionmanager and an enum across every `*-api` proto. Adding `FX_OPTION` is a
   broad, cross-cutting change (§3) — we concentrate it to contain blast radius.
3. **Cross-service edges are invisible to tooling** [wiki] (Kafka/FIX/Protobuf/in-proc bus, not
   HTTP/gRPC) and **four hops are inferred, not traced** (orderrouting→risk, risk→destination,
   destination→clearing, clearing→positionmanager). Every plan item that crosses one of those
   hops is LIVE-GATED.

---

## 1. FIX dialect + the `celnet-fix` crate

### 1.1 Why a FIX crate at all, and where it sits

`destination` and `marketdata` are the estate's FIX edges [wiki]; they speak **FIX 4.2 / 4.4 /
5.0** to 20+ venues. For FX options, Celnet's role is *not* to replace `destination` — it is to
(a) define the **FX-options FIX dialect** the estate currently lacks (no option product type =
no option FIX mapping), and (b) be able to act as a **FIX counterparty** in two modes:

- **Acceptor (quote venue):** Celnet receives `QuoteRequest` (RFQ) / `QuoteRequest` with
  `QuoteType=Tradeable` (RFS), streams `Quote` / `MassQuote`, and accepts `NewOrderSingle` /
  `NewOrderMultileg` against a live quote, replying with `ExecutionReport`. This is the
  client-facing pricing edge (electronic-RFQ desk).
- **Initiator (price taker / hedge):** Celnet sends orders out for delta hedging via the same
  dialect `destination` will eventually map.

`celnet-fix` is a **leaf crate** consumed only by `celnet-server` (the async edge), never by
`celnet-engine`. **Status: this crate is now built** — `crates/celnet-fix` (zero-copy FIX 4.4
framing, FIXT/4.4 session FSM, FX-options dialect, acceptor + initiator), loopback-tested and
wired into `celnet-server`; `docs/ARCHITECTURE.md` §2 records it as built. (The wave this plan
describes has landed for the *here-buildable* scope; live-venue wiring stays LIVE-GATED per §7.)
Dependency arrow stays one-way: `celnet-server → celnet-fix → celnet-proto/celnet-types`; nothing
on the pinned hot path touches a socket or a parser.

### 1.2 The FX-options FIX dialect (concrete tag set)

FX vanilla option, single leg, in `QuoteRequest (35=R)` / `Quote (35=S)` /
`NewOrderSingle (35=D)` / `ExecutionReport (35=8)`. Instrument block:

| Tag | Field | FX-option value | Notes |
|---|---|---|---|
| 55 | Symbol | `EUR/USD` | pair; canonicalized via `celnet-conventions` |
| 460 | Product | `4` (CURRENCY) | |
| 461 | CFICode | `OXXXXX` family | `O`=option; encodes call/put + style + cash/phys |
| 167 | SecurityType | `FXVO` | FX vanilla option (vs `FXNO` NDO, `FXSO`/`OPT`) |
| 201 | PutOrCall | `0`=put `1`=call | mapped to/from `celnet-types` option type |
| 202 | StrikePrice | e.g. `1.0950` | |
| 947 | StrikeCurrency | `USD` | with 15=Currency disambiguates premium/strike ccy |
| 1194 | ExerciseStyle | `0`=European `1`=American | FX vanilla = European |
| 541 | MaturityDate | `20270615` | expiry |
| 1079 | MaturityTime / cut | NY1000/TOK1500 | option cut — maps to `celnet-conventions` cut |
| 64 | SettlDate | delivery date | T+2 spot-lag from expiry |
| 1: 15/120 | Currency / SettlCurrency | premium currency | premium-in-foreign vs domestic |
| 38 / 152 | OrderQty / CashOrderQty | notional (per leg ccy) | which side of the pair the notional is in |
| 132/133 / 134/135 | Bid/OfferPx, Bid/OfferSize | quote prices | price unit per §1.3 |
| 537 | QuoteType | `0`=Indicative `1`=Tradeable | indicative-RFQ vs firm-RFS |
| 117/693 | QuoteID / QuoteRespID | quote identity | last-look / idempotency anchor |
| 60 | TransactTime | | |

**Price unit:** FX-option premium can be quoted in **% of notional**, **pips of the cross**, or
**vol**. The dialect carries `PriceType` and a Celnet extension on the vol-quoted path
(`QuoteRequest` may request a vol quote that Celnet fills from the surface). The unit is part of
the **dialect contract**, validated against `celnet-conventions` on every inbound/outbound
message — *convention error dwarfs model error*, the same principle `celnet-integration`'s
normalize layer already enforces.

**Multi-leg strategies** (straddle, strangle, risk-reversal, fly, calendar, seagull) use the
multileg messages: `QuoteRequest` with the **QuotReqLegsGrp** repeating group and execution via
**`NewOrderMultileg (35=AB)`** + **`MultilegOrderCancelReplace`**. Per-leg block:

| Tag | Leg field | Use |
|---|---|---|
| 600 | LegSymbol | per-leg pair (same for FX strategies) |
| 608 | LegCFICode / 609 LegSecurityType | per-leg option descriptor |
| 612 | LegStrikePrice | per-leg strike |
| 624 | LegSide | `1`=buy `2`=sell |
| 623 | LegRatioQty | ratio (RR = +1/−1, fly = +1/−2/+1) |
| 556 | LegCurrency | leg notional currency |
| 654 | LegRefID | ties leg to its quote/exec |

Celnet maps a multileg request onto a **strategy descriptor** in `celnet-types` (already the home
of the option vocabulary), prices each leg off one consistent surface snapshot, and returns a
**net package price** plus optional per-leg breakdown. `ExecutionReport` fans back one report per
leg with a shared `MultiLegReportingType`.

> **NDO / NDF** (non-deliverable): `SecurityType=FXNO`, plus a settlement-fixing block
> (`SettlMethod`, fixing source, fixing date). Already modelled on the ingest side by
> `celnet-integration` (NDF fixing in `VendorSmileMessage`); the dialect surfaces it.

### 1.3 OSS Rust FIX approach — **decision: hand-rolled zero-copy framing + a generated
### dialect, NOT an off-the-shelf engine**

Vetted OSS Rust FIX crates (all license-checked — `cargo-deny` must pass, guardrail #7):

| Crate | License | FIX versions | Role | Verdict for Celnet |
|---|---|---|---|---|
| **fefix / FerrumFIX** | MIT **OR** Apache-2.0 ✅ | 4.2/4.4/5.0SP2; tagvalue + JSON + FAST + SBE; `Dictionary` codegen | parser/codec + dictionary | **Use the codec ideas / dictionary model**, but it is self-described *"wildly unstable, refrain from using in production prior to 1.0"*; last release 0.7.0 (Oct 2021). Not a runtime dependency we can stake the edge on. |
| **hotfix** | MIT ✅ | 4.4 only, **initiator (buy-side) only**, tokio + tokio-rustls, in-mem/file/Mongo stores | session engine | Good session-layer reference; **initiator-only** disqualifies it as our acceptor (quote venue), and the Mongo store pulls deps we don't want. |
| **forgefix** | (verify on crates.io) | **4.2 only**, buy-side, *used in production* | session engine | Real production pedigree but **4.2-only & initiator-only** — too narrow for our acceptor + 4.4 dialect. |
| fix-rs | MIT OR Apache-2.0 ✅ | older | — | unmaintained. |

**Decision (ADR-grade, see §1.4):** **hand-roll a minimal zero-copy FIX layer in `celnet-fix`**,
borrowing fefix's *dictionary-driven, tag-value-slice* design (and citing it in doc comments),
rather than depending on any single OSS engine as a runtime dep. Rationale:

1. **No OSS Rust engine is simultaneously 4.4, acceptor-capable, production-grade, and
   dependency-clean.** Each candidate fails at least one. Adopting one means forking it anyway.
2. **Our needs are narrow:** one dialect (FX options), two session roles, a small message set
   (R/S/MassQuote/D/AB/8/quote-cancel/heartbeat/resend). A focused hand-rolled codec is *smaller*
   and easier to fuzz to last-byte than bending a general engine.
3. **Zero-copy is a hard requirement on the edge.** Parse a FIX frame as `&[u8]`, validate the
   `SOH`-delimited tag=value run, expose typed accessors over borrowed slices — no per-message
   allocation, consistent with `docs/ARCHITECTURE.md` §3 latency discipline. fefix's tagvalue
   model shows exactly this; we implement that shape directly.
4. **Determinism & auditability:** a hand-rolled codec in one crate is fully under our
   fmt/clippy-D/fuzz/mutation gates, with no transitive surprise deps to vet each upgrade.

`celnet-fix` module layout:

```
celnet-fix/
  framing.rs    # SOH framing, BodyLength(9)/CheckSum(10) validate, zero-copy frame cursor
  dictionary.rs # the FX-options dialect: tag → type, required/conditional groups (table-driven)
  session.rs    # FIXT/4.4 session: logon/logout, heartbeat(0)/testreq(1)/resendreq(2)/
                #   seqreset(4), gap-fill, store trait (in-mem + file; NO Mongo)
  acceptor.rs   # quote-venue role: R→S/MassQuote, D/AB→8, last-look via QuoteID validity
  initiator.rs  # price-taker/hedge role
  messages.rs   # typed views: QuoteRequest, Quote, MassQuote, NewOrderSingle,
                #   NewOrderMultileg, ExecutionReport (borrowed accessors + owned builders)
  dialect_fx.rs # the §1.2 tag mapping ↔ celnet-types option/strategy descriptors + conventions
```

`celnet-fix` depends on: `celnet-types`, `celnet-conventions`, `celnet-proto` (for the
internal request/response shape it bridges to the engine), `tokio` (async I/O, edge-only).
No `unsafe` (matches `celnet-integration`'s `#![forbid(unsafe_code)]`).

**Validation gate (§1, all REAL-buildable-here):**
- Codec round-trips every dialect message bit-identically (proptest over generated frames).
- BodyLength/CheckSum, required-field, and group-cardinality validation rejects malformed
  frames without panic (fuzz target, nightly, like the existing fuzz gate).
- Session FSM: logon → heartbeat → gap detected → resend request → gap-fill, proven against a
  scripted peer (the §7 **real loopback FIX peer (our own acceptor+initiator over a socket, no stub)**).
- A `QuoteRequest`(vol) → `Quote` round-trip reprices the **same** premium the `celnet-engine`
  surface produces (cross-checked vs `celnet-golden`/QuantLib tables) — the dialect carries the
  pricer faithfully, convention-correct.
- Multileg RR/straddle: package price == sum of per-leg prices off one surface snapshot.

### 1.4 ADR-0xx — *Hand-rolled `celnet-fix` over an OSS FIX engine*

Record via `manage_adr` (codebase-memory): **Decision** as §1.3. **Status** accepted.
**Consequences:** we own the session FSM and the dialect dictionary (more code, fully gated);
we avoid an unmaintained/initiator-only/4.2-only runtime dep and a Mongo transitive; the dialect
is the single current contract (no FIX-version negotiation beyond the wire 4.4 framing — guardrail
#9). **Open fallback:** if acceptor session complexity balloons, vendor in fefix's `session`
module (MIT/Apache, compatible) rather than a closed engine.

---

## 2. The distributor adapter — ADR-grade decision (the highest-risk seam)

### 2.1 The problem, precisely

The distributor is an **in-proc JVM disruptor** in `baseserver` [wiki]; `marketmerchant`
publishes via `publishPriceEventOrSkipWhileFull()` — **bounded mailbox, silent skip on full**
[wiki]. Celnet is a Rust process that streams at ≥1M updates/s/core (`ARCHITECTURE.md` §1.2) and
prices vanillas at p50 ≤ 2 µs. **An unthrottled Celnet pricer will silently saturate the JVM
mailbox and drop prices with no error** — the failure mode `docs/CELER-INTEGRATION.md` §0
flagged.

### 2.2 Two connectivity options (the choice `CELER-INTEGRATION.md` §0 said must be made first)

- **(A) JVM sidecar adapter.** A thin JVM process embeds the real `baseserver` distributor client
  and exposes a local socket (UDS/TCP) to Celnet. Celnet speaks a trivial length-prefixed
  Protobuf framing to the sidecar; the sidecar does the real `notifyUsers`. **Pro:** uses the
  *actual* disruptor client → guaranteed protocol-correct, back-pressure semantics inherited
  for free. **Con:** a JVM hop in the egress path (extra latency + an extra process to operate);
  but egress is the async edge, not the hot core, so the µs budget is unaffected.
- **(B) Native `DistributorProducerChannelHandler` socket client in Rust.** Reimplement the
  distributor socket/handshake protocol in `celnet-fix`'s sibling (a `celnet-distributor`
  module inside `celnet-integration`, or a small new crate). **Pro:** no JVM, lowest latency.
  **Con:** we must reverse-engineer and *track* an undocumented in-house wire protocol and its
  back-pressure contract — high risk, LIVE-GATED, brittle across estate upgrades.

**Decision (ADR-0xx, accepted): start with (A) the JVM sidecar adapter; keep (B) as a
benchmark-gated optimization behind the same internal seam.** Reasons: the sidecar is the only
option buildable *correctly* without reverse-engineering, it inherits the real skip-while-full
contract, and the egress hop is off the µs hot path. The seam (`DistributorEgress` trait, §2.3)
is identical for both, so swapping to (B) later is a leaf change with no caller impact.

### 2.3 The bounded-mailbox + rate-limit shim (this is the part we build NOW, proven against real loopback peers)

Regardless of A/B, Celnet inserts its **own bounded, rate-limited egress stage** *before* the
distributor so a µs pricer can never cause a silent drop downstream:

```
celnet-engine (price events, rtrb SPSC, zero-alloc)
   → [celnet-integration::distributor] EgressGovernor:
        • bounded ring (rtrb, fixed capacity) — explicit, observable
        • conflation: coalesce stale updates per (pair,tenor,strike) key — newest wins
        • token-bucket rate limiter sized to the measured distributor drain rate
        • on local-ring-full: COUNTED drop + HdrHistogram + metric (NEVER silent)
   → DistributorEgress trait
        • impl A: UDS/TCP framed proto → JVM sidecar → notifyUsers
        • impl B: native DistributorProducerChannelHandler socket
```

Key properties:
- **Conflation, not just throttling.** FX quotes are *replaceable* — a stale price for a
  `(pair,tenor,delta)` pillar should be dropped in favour of the latest. Conflation makes the
  rate limiter lossless *in information* even when lossy in messages.
- **Drops are explicit and counted.** Unlike `skip-while-full`, Celnet's stage emits a metric +
  histogram on every conflation/drop. We turn a silent JVM failure into an observable Celnet SLO.
- **Rate budget is a config, calibrated to the distributor's measured drain rate** — tenant
  overlay (`celnet-client`), not baked in.
- **Zero-cost on the hot core.** The governor lives on the async edge; the engine just pushes to
  an `rtrb` ring (`ARCHITECTURE.md` §3 — telemetry/egress offloads over a bounded queue, hot core
  stays alloc/lock/log-free).

### 2.4 ADR-0xx — *Distributor egress: JVM sidecar + Celnet-side bounded conflating governor*

`manage_adr`: **Decision** = §2.2 (A now, B deferred) + §2.3 (the governor). **Status** accepted.
**Consequences:** an operated JVM sidecar; a Celnet egress stage that converts silent
skip-while-full into observable, counted conflation; calibration of mailbox depth + drain rate is
LIVE-GATED (open question 2 in `CELER-INTEGRATION.md`).

**Validation gate:**
- **REAL-buildable-here:** the `EgressGovernor` (bounded ring + conflation + token bucket +
  counted drops) is fully buildable + testable here against a **real rate-limited socket sink** (a sink
  that drains at a configurable rate and asserts it never receives more than its capacity, and
  that conflation keeps only newest-per-key). Property test: pricer at 1M/s into a 10k/s sink →
  zero unbounded growth, newest price always delivered, drop count == produced − delivered.
- **LIVE-GATED:** sidecar handshake against the real `baseserver` distributor; calibration of the
  real drain rate; proof of no skip-while-full at the JVM mailbox under the calibrated budget.

---

## 3. The `FX_OPTION` product-type touch-list (blast-radius containment)

Adding an option product type touches the whole estate [wiki]. We **concentrate** the change to
one new enum value + one new message shape per service, behind a feature flag / tenant overlay so
existing FX-spot/forward flows are byte-for-byte unchanged until a tenant opts in (guardrail:
non-disruptive, `CELER-INTEGRATION.md` §4 phasing).

| Service / artifact | Change | Containment | Gate |
|---|---|---|---|
| `celertech-type` / `staticdata` | add `FX_OPTION` product + option reference data (strike, expiry, cut, style, settlement) | one new enum value + one reference record type; spot/fwd untouched | LIVE-GATED (estate repo change) |
| **every `*-api` proto enum** carrying `ProductType` | add `FX_OPTION = <next>` | append-only enum value (no renumber → wire-safe); single value, not a new message family | LIVE-GATED |
| `positionmanager` netting key `(ProductType, SettlementDate, SettlementType)` | options net by **(ProductType, pair, strike, expiry, call/put, SettlementType)** — strike+expiry are part of the option's identity, unlike spot | new key *only* when `ProductType==FX_OPTION`; existing spot key path unchanged | LIVE-GATED; REAL-here key-logic proof here |
| `risk` exposure model | option exposure = delta-equivalent notional + vega/gamma buckets, not linear notional | new exposure calculator branch keyed on `FX_OPTION`; `RiskCheckFxOrderRequestHandler` gains an option arm | LIVE-GATED; REAL-here: Celnet exports the Greeks risk needs |
| `destination` FIX dialect | map `FX_OPTION` order → §1.2 FIX (FXVO/multileg) | new dialect mapping module; reuses Celnet's `celnet-fix::dialect_fx` as the **reference spec** | partly REAL-here (dialect lives in `celnet-fix`), wiring LIVE-GATED |
| `marketmerchant` quote assembly DAG | accept option quote events for assembly/publish | new price-event subtype; DAG branch | LIVE-GATED |
| frontend webtrader | option ticket + surface/Greeks panels | additive screens | LIVE-GATED |

**What Celnet owns and can build now:** the **canonical option product descriptor** in
`celnet-types` (already exists — the option vocabulary, strategy descriptors), the **FIX dialect
mapping** (`celnet-fix`, §1), and a **proto bridge** in `celnet-proto`/`celnet-server` that
*projects* a Celnet option + Greeks into the shape each estate proto will need — so that when the
estate enums gain `FX_OPTION`, the mapping is a thin, already-tested adapter, not new design.

> **Honesty note:** the enum/netting/exposure/dialect changes are **edits to Celer estate repos**
> we do **not** modify here (this plan only writes the Celnet-side seam + the contract tests).
> The touch-list is the coordination checklist devops/estate owners execute in a staging tenant.

---

## 4. Market-data ingress wiring

`celnet-integration` already implements the **ingest→normalize→blend→canonical-surface** pipeline
(vendor ATM/RR/BF smile messages → `celnet_surface::MarketQuotes`/`MarketContext`, with
convention cross-check, staleness decay, divergence gating). This wave **wires it to the live
estate ingress**, behind the same fault-tolerant design it already has.

| Source | Wiring | State |
|---|---|---|
| **External vol feed** (FMD FXO 2.0-style: ATM + 25Δ/10Δ RR/BF, spot, fwd pts, NDF fixings) | already normalized by `celnet-integration::normalize`; add the live **WS/stream client** that decodes the feed body into `VendorSmileMessage` and feeds `pipeline_messages` | adapter REAL-buildable-here; live feed creds LIVE-GATED |
| **`MarketMerchantPriceService`** (WS-only, no fallback, ~6 conn/domain semaphore) [wiki] | a resilient WS subscriber in `celnet-server` edge: subscribe spot/forward, **reconnect + resubscribe + sequence-gap resync**, connection-economy (multiplex many pairs over few sockets to respect the 6-conn semaphore) | client REAL-buildable-here against a real loopback WS server (replays recorded frames, exercises the real subscriber); live endpoint LIVE-GATED |
| **`marketdata` / `marketdata-api`** (spot today; vol capability unconfirmed) [wiki] | passive distributor subscriber via the §2 sidecar (ingress direction) | LIVE-GATED |
| **curve service** (DF_d/DF_f) | feeds `r_dom` into `celnet-integration::pipeline` (already a parameter) | LIVE-GATED source; REAL-here with synthetic curves |

**Resync contract (build now, proven against real loopback peers):** the WS subscriber implements
*subscribe → snapshot → sequenced deltas → on-gap full-resync*, mirroring the multiplex resync
the `celnet-server` `StreamSession` already does internally (per the ledger: per-sub
sequence/snapshot/delta/resync). On reconnect it drains/fails in-flight waiters and re-pins to the
latest `surface_version` (the server already has `surface_book` + `services::pin`). The
divergence/staleness layer in `celnet-integration` already degrades a quiet or disconnected feed
gracefully — exactly the WS-no-fallback reality.

**Validation gate:**
- **REAL-buildable-here:** real loopback WS server (replays recorded frames, exercises the real subscriber) replays a recorded snapshot+delta+gap stream; assert
  the subscriber resubscribes, resyncs on the injected gap, and never delivers out-of-order or
  stale-past-decay quotes; assert ≤6 sockets/domain under N pairs (connection multiplexing).
  Feed adapter: recorded feed bodies → `VendorSmileMessage` → existing `pipeline` builds a
  surface (this path already has tests in `celnet-integration`).
- **LIVE-GATED:** real `MarketMerchantPriceService` endpoint, real feed entitlement, confirmation
  that vol (not just spot) is deliverable, real distributor ingress subscription.

---

## 5. What is publishable to the estate (egress recap)

Celnet publishes, via the §2 governor: indicative + firm option prices/quotes, the full 13-Greek
set (delta per convention, gamma, vega ladder, theta, rho-dom + rho-for, vanna, volga, charm,
speed, zomma, color), the arbitrage-free constructed surface, and risk/exposure/PnL inputs. The
egress message shapes are the `celnet-proto` projections of §3. All egress is **rate-limited +
conflated + counted** (§2.3). Indicative-only first, firm/tradeable after the §1 acceptor +
last-look path is proven in a staging tenant.

---

## 6. Build-vs-defer task list (each with its validation gate)

### BUILD NOW — provable here against a REAL-here estate

| # | Task | Crate | Validation gate |
|---|---|---|---|
| B1 | `celnet-fix` zero-copy framing + checksum/bodylen + fuzz | celnet-fix | round-trip proptest + nightly fuzz, no-panic on malformed |
| B2 | FX-options dialect dictionary (§1.2) + multileg (§1.2) | celnet-fix | every message validates; group cardinality enforced |
| B3 | `celnet-fix` session FSM (logon/heart/resend/gapfill), acceptor + initiator | celnet-fix | scripted-peer test: logon→gap→resend→fill |
| B4 | dialect ↔ `celnet-types` option/strategy mapping + convention cross-check | celnet-fix | QuoteRequest(vol)→Quote reprices engine/golden price; convention mismatch rejected |
| B5 | **Real loopback FIX peer (our own acceptor+initiator, no stub)** (test harness): RFQ/RFS client + venue that drives B1–B4 | celnet-testkit | drives a full RFQ→Quote→Order→ExecReport loop end-to-end |
| B6 | `EgressGovernor`: bounded ring + conflation + token bucket + counted drops | celnet-integration | 1M/s→10k/s sink: bounded, newest-per-key delivered, drops counted |
| B7 | **Real rate-limited socket sink** sink + `DistributorEgress` trait (impl A seam) | celnet-integration/testkit | sink never over-capacity; conflation correctness |
| B8 | Resilient WS market-data subscriber (reconnect/resub/resync, ≤6 conn multiplex) | celnet-server | loopback-WS gap-injection: resync, ordering, conn-economy |
| B9 | Live-feed adapter: feed body → `VendorSmileMessage` → existing pipeline | celnet-integration | recorded bodies build a surface (extends existing tests) |
| B10 | `celnet-proto` egress projections of the option + Greeks (the §3 estate shapes) | celnet-proto/server | round-trips; matches the §3 touch-list field-for-field |
| B11 | Option netting-key + delta-equiv/vega exposure logic (Celnet-side reference impl) | celnet-types/integration | unit-proven key + exposure math vs golden |
| B12 | ADRs: hand-rolled FIX (§1.4), distributor egress (§2.4), product-type plan (§3) | — (manage_adr) | recorded in codebase-memory; docs synced |

### DEFER — LIVE-GATED (cannot be finished without a running Celer staging tenant)

| # | Task | Blocked on | Deployment gate to lift |
|---|---|---|---|
| D1 | JVM distributor sidecar handshake + calibrate mailbox depth/drain rate | real `baseserver` distributor | sidecar connects; no skip-while-full under calibrated budget |
| D2 | Native `DistributorProducerChannelHandler` client (impl B, optimization) | reverse-engineered socket protocol | bench beats A; same governor seam |
| D3 | `FX_OPTION` enum across every `*-api` proto + `celertech-type`/`staticdata` | estate repo changes (devops-owned) | append-only enum; spot/fwd flows byte-unchanged |
| D4 | `positionmanager` option netting key + `risk` option exposure model | estate repo changes | staging: option position nets correctly, risk gate passes/rejects |
| D5 | `destination` FX-options FIX dialect wiring (uses B2 as spec) | estate repo + venue FIX certs | venue cert: RFQ/order/exec round-trips to a real LP venue |
| D6 | `marketmerchant` DAG option-quote branch | estate repo change | option quotes assemble + publish in staging |
| D7 | Validate the 4 **inferred** hops (or→risk, risk→dest, dest→clearing, clearing→pm) | running staging traces | each hop observed in a staging trade, not inferred |
| D8 | Live `MarketMerchantPriceService` + feed entitlement + vol-capability confirmation | endpoint + creds + entitlement | live spot/vol stream resilient across a forced disconnect |
| D9 | `celnet-client` tenant overlay ownership + per-tenant option enablement | devops | overlay gates option product on/off per tenant |
| D10 | React webtrader option ticket + surface/Greeks panels | frontend repo (gRPC/WS) | desk places an RFQ and sees live Greeks in staging |

---

## 7. The live-estate boundary (honest statement)

**Buildable + testable here, now, with no Celer access:** the entire `celnet-fix` crate (codec,
dialect, session, acceptor/initiator), the egress governor, the resilient WS subscriber, the feed
adapter, and the proto projections — **all driven by two test doubles we build ourselves:**

1. **Real loopback FIX peer (our own acceptor+initiator, no stub)** (B5, in `celnet-testkit`): a scripted FIX peer that issues
   QuoteRequests/RFS, accepts Quotes, sends NewOrderSingle/Multileg, and asserts ExecutionReports
   — exercising the full §1 dialect and session FSM over a loopback socket. This is a *real* FIX
   peer (our own), not a stub of pricing — it proves the wire contract end-to-end.
2. **Real rate-limited socket sink** (B7): a configurable-drain-rate sink behind the `DistributorEgress`
   trait that asserts capacity is never exceeded and conflation keeps newest-per-key — proving the
   governor *here*, while the real `notifyUsers` stays behind the same seam.

**Not finishable without a live staging estate (D1–D10):** anything that crosses into a Celer
process — the JVM distributor handshake and its real mailbox calibration, the `*-api` proto enum +
`positionmanager`/`risk`/`destination` edits (estate repos we do not touch from here), the four
**inferred** lifecycle hops, the live `MarketMerchantPriceService` endpoint + feed entitlement,
and tenant-overlay ownership. For each, **we build the Celnet-side seam and its contract test now**
and the deployment gate is *"the same contract test, re-run against the real far side in a staging
tenant, passes."* We never fake the far side to claim completion (guardrail #2).

The honest critical path to a *live* option RFQ in staging is therefore: **B1–B12 (here) →
D3+D4+D5 (estate enum/netting/risk/dialect edits, devops-coordinated) → D1+D8 (live transports) →
D7 (verify the inferred hops) → D9 (tenant enablement) → D10 (frontend).** Phases D3–D10 are
estate-side execution gated on a staging environment, exactly the boundary `CELER-INTEGRATION.md`
§5 enumerates as open questions.
