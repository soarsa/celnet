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
trader-shaped. The current contract models loops 1–4 directly; loop 5 (position/P&L) is
designed but not yet on the wire (§7).

---

## 2. One contract, two transports

- **gRPC (primary, implemented):** `tonic` over the `celnet-proto` types — the low-latency
  programmatic path the modern market-maker wants. This is what `celnet-server` serves and
  what `celnet-client` consumes; it is exercised by the real-like trader-workflow tests.
- **WebSocket mirror (designed):** a firewall-friendly transport that serializes the *same*
  stream messages as a tagged-JSON enum (`Snapshot`/`Update`/`Heartbeat`/`StreamEnd`), so a
  browser/light client gets identical snapshot+delta+resync semantics. To avoid drift, the
  WS payloads must be generated from the same Rust types as gRPC — *one contract, two
  transports*. The JSON-over-WS transport is **not yet wired**; gRPC is the shipped path.

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
           | Touch   // one-touch / no-touch / double-no-touch
```

Supporting messages: `Quantity` (notional + which leg-ccy), `Solve` (solve strike or premium
so a leg/structure is zero-cost), `StrikeOrDelta` (`oneof spec` — quote by strike or by
delta in the configured convention), `Conventions` and `MarketContext` carried so a price is
fully self-describing, `FixingSchedule` for path/fixing structures, `Greeks`, `TwoWayPrice`.

**Deferred (do not claim as built):** `Tarf` and `Accumulator` variants. The underlying
`celnet-exotics` engine prices the components, but the wire model for fixing-schedule +
target-redemption + gearing is a deliberate later coordinated interface change (a thin
under-specified message here would be a placeholder, banned by rule 2).

---

## 4. Services (the implemented contract)

| Service · RPC | Purpose |
|---|---|
| `PricingService.Price(PriceRequest) → PriceResponse` | One-shot price + full Greeks for an `Instrument` in a `MarketContext`. |
| `QuoteService.RequestQuote(QuoteRequest) → Quote` | RFQ: client-supplied `idempotency_key`, `Instrument`, side(s) (omit ⇒ two-way), quantity → server `quote_id`, `TwoWayPrice`, full Greeks, `valid_until` last-look deadline, resolved `Conventions`. |
| `QuoteService.AcceptQuote(QuoteAccept) → Execution` | Accept within the last-look window → booked `Execution`. |
| `QuoteService.RejectQuote(QuoteReject) → RejectAck` | Decline a live quote. |
| `StreamService.Stream(stream ClientStreamMessage) → stream ServerStreamMessage` | RFS bidi keyed on a client `SubscriptionId`. |
| `SurfaceService.GetSmile(GetSmileRequest) → Smile` | Smile on a delta axis with ATM/25Δ&10Δ RR/BF and an `ArbReport`. |
| `SurfaceService.MarkSurface(MarkSurfaceRequest) → MarkSurfaceResponse` | Calibrate from a `BrokerQuoteSet` + `Conventions` → a marked surface version. |
| `SurfaceService.Scenario(ScenarioRequest) → ScenarioResponse` | `ShockAxis` grid (spot/vol/rate, absolute or relative) → `ScenarioPoint`s. |

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
new subscriptions while blue-green-draining. *(A `Modify` message is designed but not yet in
the contract; clients re-`Subscribe` to change a structure.)*

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
- `get_smile(...)`, `mark_surface(...)`, `scenario(...)`.
- Ergonomic builders: `InstrumentSpec::vanilla/strategy/unit`, `Conventions::major_default()
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

- **Position / P&L** — `GetPosition(book)` and `AttributePnl(book, from_mark, to_mark)`
  (delta/gamma/vega/theta/vanna/volga decomposition). The Greeks and engine exist; the
  service is not yet in `celnet-proto`.
- **`Tarf` / `Accumulator`** instrument variants (§3).
- **WebSocket JSON-mirror transport** (§2).
- **`Modify`** RFS message (§4).

These are tracked as the API-evolution-v2 wave; they are listed here so the contract's scope
is not over-read.

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
