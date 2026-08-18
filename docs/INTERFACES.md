# Celnet — Frozen Interface Registry

The contracts that parallel work-streams depend on. There is exactly **one clean, current
contract** — no versioned APIs, no back-compat shims (we have no external users). Changing
anything here means editing the interface crate **and every dependent in the same change**,
updating this file, and announcing it in the `CLAUDE.md` ledger. Within a parallel-build
window the interface crates are treated as **stable** so streams don't churn; a deliberate
interface change coordinates all affected crates at once (see `docs/ROADMAP.md` §3).

## The 40-crate workspace

The implemented flat workspace is **40 crates** (`ls crates`):

```
celnet-types  celnet-core  celnet-conventions  celnet-calendar  celnet-vanilla
celnet-surface  celnet-exotics  celnet-heston  celnet-qmc  celnet-gpu
celnet-engine  celnet-journal  celnet-replog  celnet-fanout  celnet-integration
celnet-server  celnet-cli  celnet-client  celnet-proto  celnet-plugin-api
celnet-plugin-host  celnet-observability  celnet-golden  celnet-testkit  celnet-bench
celnet-router  celnet-fix  celnet-parity  celnet-risk-normalize  celnet-risk-cube
celnet-risk-fleet  celnet-limits  celnet-entitlements  celnet-xva  celnet-linear
celnet-equity-vanilla  celnet-commodity-vanilla  celnet-crypto-vanilla  celnet-rfq
celnet-rates
```

Several domains the early design split across many crates were **consolidated**:
`celnet-surface` holds VV/SABR/SVI/SSVI/eSSVI + arbitrage gates + term structure;
`celnet-exotics` holds LSV + PDE + MC + American/Bermudan + correlated basket + the shared
numerics; `celnet-golden` is the QuantLib oracle/generator; `celnet-observability` owns
telemetry rings/histograms; `celnet-gpu` is the wgpu backend + f64 CPU reconciliation. Later
scale/correctness/catalogue waves added disjoint leaf crates: `celnet-heston` (standalone
Heston European vanilla via two independent CF transforms), `celnet-qmc` (scrambled Joe-Kuo
Sobol' + Owen scramble + Brownian-bridge), `celnet-replog` (leader-replicated deterministic-
replay log over loopback sockets + full Raft) and `celnet-fanout` (lock-free SPMC broadcast
ring) under the engine, `celnet-risk-fleet` (cross-shard risk fan-out algebra over the
`celnet-router` HRW map), and `celnet-xva` (internal-only EPE/ENE + CVA/DVA/FVA over synthetic
netting sets — no wire surface). The cross-asset/linear/RFQ wave (W2–W5) added five more leaf
crates: `celnet-linear` (outright forward / FX swap / NDF over the asset-class-agnostic carry
seam), `celnet-equity-vanilla` (generalized-BSM, dividend-yield carry), `celnet-commodity-vanilla`
(futures-style undiscounted-forward pricing), `celnet-crypto-vanilla` (linear funding-carry +
inverse coin-margined `1/S_T` payoff), and `celnet-rfq` (the multi-dealer RFQ-to-many engine:
concurrent fan-out, ranking, deterministic tie-break, last-look). The fixed-income/rates wave
added `celnet-rates` (OIS/SOFR multi-curve bootstrap + FRA/IRS/STIR-futures/cash-bond PV +
curve risk, now wired into the server's `RatesService` + FI dealer desk — see §"Fixed-income /
rates subsystem"). `celnet-plugin-host` is **built**: the tiered host (Tier-0
native registry + Tier-2 **wasmi** fuel-metered sandbox + replay harness) behind the frozen
`celnet-plugin-api` contract — wasmtime was rejected for open RustSec advisories (see
`docs/PLUGIN-HOST-ALT.md`).

## Dependency direction (must never invert)

```
celnet-types  ←  celnet-core  ←  { celnet-conventions, celnet-calendar, celnet-vanilla,
                                   celnet-surface, celnet-exotics, celnet-heston, celnet-qmc,
                                   celnet-gpu }  ←  celnet-engine  ←  { celnet-server, celnet-cli }
celnet-proto      →  depends only on celnet-types
celnet-plugin-api →  depends only on celnet-types (+ celnet-core traits)
celnet-client     →  depends on celnet-proto (typed SDK over tonic)
celnet-observability →  telemetry seam; celnet-engine stays free of its deps
celnet-heston     →  standalone Heston European vanilla over celnet-core/-types/libm (no IO)
celnet-qmc        →  Sobol'/scramble/Brownian-bridge variate source over celnet-core (no IO);
                     consumed by celnet-exotics/-xva and the celnet-gpu path/greeks kernels
celnet-journal    →  dependency-free fsync'd append-only log + crash recovery; under celnet-engine
celnet-replog     →  leader-replicated deterministic-replay log over loopback sockets (→ celnet-journal)
celnet-fanout     →  lock-free SPMC broadcast ring (reuses crossbeam-utils CachePadded only)
celnet-integration   →  Celer estate + vendor MD adapters, over celnet-surface/-types
celnet-risk-normalize →  pure leaf transform (no IO) over celnet-core/-types + the vanilla
                         tier — celnet-{vanilla,equity-vanilla,commodity-vanilla,crypto-vanilla}
                         (grew cross-asset reach to canonicalize mixed-asset risk);
                         the convention/numeraire boundary the risk cube sits on
celnet-rates          →  WIRED fixed-income/rates leaf: OIS/SOFR multi-curve bootstrap (log-linear-DF
                         `Curve`) + FRA / IRS (`vanilla_swap`) / STIR-futures / cash-bond PV +
                         par-rate / PV01 / DV01 + a key-rate-DV01 ladder + Brent solver, over
                         celnet-types/-calendar only (no IO). **Now consumed by `celnet-server`**
                         (`rates_pricing.rs` → `price_ois`/`par_rate_for`/`build_quotes` call
                         `bootstrap_ois`/`ois_par_rate`/`ois_risk`), exposed via the `RatesService`
                         RPCs (`PriceRates`/`AggregateRatesRisk`/`BookRatesPosition`/`ListRatesPositions`)
                         and the FI dealer-quoting desk (`RfqDeskEdge` + `NotificationService`). See
                         §"Fixed-income / rates subsystem"; convergence direction = ADR-0010.
celnet-risk-cube      →  single-node OLAP cube over celnet-risk-normalize (+ -vanilla
                         for bump-and-revalue, -core, -types); no IO/market-data
celnet-risk-fleet     →  cross-shard risk fan-out ALGEBRA over celnet-risk-cube + celnet-router
                         (HRW map) + -types; fan-out == single-node (transport designed-only)
celnet-router         →  shard-by-pair/tenant HRW partition map over celnet-types; no IO
celnet-entitlements   →  pure pre-aggregation pruning predicate over celnet-risk-cube
                         (FactKey/Hierarchy/DimensionId) + -types; no IO; sits beside the
                         cube, upstream of its group-by
celnet-limits         →  pure limit framework over celnet-risk-cube (NodeAggregate /
                         dimension keys / Hierarchy) + celnet-risk-normalize (CanonicalLeaf)
                         + -types; no IO; sits above the cube, downstream of its group-by
celnet-xva            →  internal-only XVA engine (EPE/ENE + CVA/DVA/FVA) over a hazard-rate
                         survival curve + the celnet-qmc Sobol/bridge variates; synthetic
                         netting sets, no wire surface
celnet-linear         →  linear (non-option) FX book leaf: outright forward / FX swap / NDF
                         over celnet-core/-types (no IO); consumed by celnet-server/-cli
celnet-equity-vanilla / celnet-commodity-vanilla / celnet-crypto-vanilla
                      →  cross-asset vanilla leaves on the generalized carry seam, each over
                         celnet-core/-types only (no IO); consumed by the server pricer
celnet-rfq            →  multi-dealer RFQ-to-many engine (fan-out, ranking, tie-break,
                         last-look) over celnet-proto + celnet-fix + celnet-types
celnet-golden, celnet-testkit, celnet-bench  →  test/validation/bench only
```

## Status

| Crate | Version | Frozen? | Surface |
|-------|---------|---------|---------|
| `celnet-types` | 0.0.0 | **freeze-candidate** | `OptionType`, `Ccy`, `CcyPair`, `Tenor` (now `Overnight`/`TomNext`/`SpotNext`/`Weeks`/`Months`/`Years`/`Imm(u8)`/`BrokenDate(BrokenDate)`), `BrokenDate{year:i32,month:u8,day:u8}`, `SmileModel` (`MarketHedge`/`StochasticVol`/`Parametric`/`ParametricSurface`/`ExtendedSurface`, `Default=MarketHedge`; the five mirror VV / SABR / SVI / SSVI / eSSVI); newtypes `Vol`/`Strike`/`Rate`/`Delta`/`Df`/`Time`; convention enums `DeltaConvention`/`AtmConvention`/`PremiumStyle`/`Cut`/`DayCount`/`Settlement`; DTOs `VanillaInputs`, `Greeks`. POD/`Copy`, `serde`. (No `time` dep — a broken date is the POD triple.) |
| `celnet-core` | 0.0.0 | **freeze-candidate** | `math` (`norm_cdf`, `norm_pdf`, `exp`/`ln`/`sqrt` via `libm`); `is_close` + `assert_close!` (ULP/rel/abs); trait `Smile` + `FlatSmile`. Zero IO. |
| `celnet-proto` | 0.0.0 | **freeze-candidate** | single current wire contract (`prost 0.13` / `tonic 0.12`); `celnet.proto` services `PricingService`/`QuoteService`/`StreamService`/`SurfaceService`; `Instrument` oneof; **no** version field / negotiation. Phase-1 additions: `Tenor` short-end/IMM/`BrokenDate` units; `SmileModel` enum on `MarkSurfaceRequest`/`ScenarioRequest`; market-series feed (`MarketObservable`, `MarketSeriesSubscribe`/`Unsubscribe`/`Point`/`Snapshot`) multiplexed on `StreamSession`; attribution identity (`BookId`/`Owner`/`AttributionRecord`) on the quote/trade lifecycle. Phase-2 addition: `RiskService` (`ListPositions`/`AggregateRisk`/`DrillRisk`/`LimitStatus`) — server-side hierarchical risk over the org cube. api-first catalogue (W1–W6) additions: `Instrument.product` exotic arms `variance_swap=13`…`basket=25`, supporting enums (`AsianMethod`/`QuantoPayoff`/`TarfRedemption`/`AccumulatorMonitoring`/`LookbackStyle`/`BasketKind`/`ExerciseStyle`), `PricingModel pricing_model=22` booking selector, `SMILE_MODEL_EXTENDED_SURFACE=4`, `PriceResponse.price_std_error=7`/`Quote.price_std_error=12`, `ArbReport.smile_model=5` typed provenance. Cross-asset/linear/RFQ (W1–W5) additions: `Underlying` asset-class oneof + the agnostic `CarryModel` seam, linear arms `fx_forward=26`/`fx_swap=27`/`ndf=28`, `SettlementStyle settlement_style=29`, multi-dealer RFQ (`RequestMultiDealerQuote`/`MultiDealerQuote`/`DealerQuote`/`QuoteAccept.lp_id`); landing payoff arms `perpetual_option=30`/`listed_future_option=31` (in-flight). See §"Phase-1 contract extensions", §"Phase-2 contract: `RiskService`", §"Exotic catalogue + booking-model + provenance", and §"Asset-class universe, linear products, RFQ-to-many + settlement mechanics". |
| `celnet-plugin-api` | 0.0.0 | **freeze-candidate** | SDK traits (`PricingModel`/`PricingBackend`) + WIT world. |

> `celnet-proto` and `celnet-plugin-api` are built — **Gate G0 is reached** (consistent with
> `docs/CAPABILITIES-VS-COMPETITION.md`). The native trait-registry (Tier 0) and the **wasmi**
> Wasm host (Tier 2) both implement the identical `PricingModel` contract so first-party and
> user plugins are interchangeable behind one registry; the host (`celnet-plugin-host`) is
> **built** (wasmtime rejected for open RustSec advisories — wasmi is the advisory-clean
> replacement, see `docs/PLUGIN-HOST-ALT.md`).

## Determinism rules baked into the interfaces

- All float comparison via `celnet_core::assert_close!` / `is_close` (ULP + rel + abs). Never `==`; never assert on NaN payloads.
- Transcendentals via `rust-lang/libm` (correctly-rounded) for bit-identical cross-platform results.
- GPU numerics standardize on **f32** with an **f64 CPU reconciliation** oracle.
- Scalar policy: `f64` is the CPU canonical type; convention/units encoded in types, not comments.

## Phase-1 contract extensions (EXPERIENCE-ARCHITECTURE §8 Foundation)

> One unversioned contract evolved in place (guardrail #9); every dependent crate updated in
> the same change. All new request/response fields are **presence-tracked** (`optional`) and
> **default to current behaviour** when absent. Mathematical-method provenance is in doc
> comments only; identifiers are purpose-named and vendor/method-neutral (guardrail #8).

### Tenor model overhaul — `celnet-types` + `celnet-calendar` (fixes ON-resolves-as-SN)

- **`celnet_types::Tenor`** gained `TomNext` (TN), `SpotNext` (SN), `Imm(u8)` (the `n`-th IMM,
  1-based) and `BrokenDate(BrokenDate)` (an explicit civil expiry date). `Overnight`/`Weeks`/
  `Months`/`Years` are unchanged variants; **`Overnight` semantics are fixed** (see below).
- **`celnet_types::BrokenDate { year: i32, month: u8, day: u8 }`** — a POD `Copy`/`Hash`/`serde`
  date triple (keeps `celnet-types` free of the `time` crate); `BrokenDate::new(y,m,d)`.
- **Calendar resolution (`celnet-calendar::fx`)** — signatures and fixed semantics:
  - `expiry_for_tenor(pair, horizon, spot, tenor) -> Result<Date, TenorError>` — **was**
    `expiry_for_tenor(pair, spot, tenor) -> Date`. It now takes `horizon` so the **pre-spot
    short end is anchored on today, not spot**. Fixed semantics: **ON** = next good business
    day after `horizon` (~T+1) — *previously wrongly resolved relative to `spot` (≈T+3, the
    SN region); that bug is closed*. **TN** = the good day after ON. **SN** = next good day
    after `spot`. Standard ladder (Weeks/Months/Years) still anchored on `spot` with
    modified-following + end-of-month. **IMM** = `n`-th 3rd-Wednesday of the Mar/Jun/Sep/Dec
    cycle strictly after `horizon`, then modified-following. **BrokenDate** = the explicit
    date, modified-following onto a good day.
  - `schedule(pair, horizon, tenor) -> Result<FxSchedule, TenorError>` (**was** infallible).
    `FxSchedule` gained `vol_anchor: Date` — the date vol-time accrues **from** (`horizon` for
    ON/TN, `spot` otherwise), so the short end never yields a non-positive vol-time.
  - `imm_date(horizon, n) -> Date` (new public helper) — the raw `n`-th IMM third-Wednesday
    (pre-roll). `TenorError { ImmOrdinalZero, InvalidBrokenDate(BrokenDate) }` (new) — the two
    input-bearing failure cases; the standard ladder/short-end/positive-IMM always resolve.
  - `celnet_conventions::schedule`/`vol_year_fraction` are now `Result<_, TenorError>` and
    `vol_year_fraction` accrues over `vol_anchor → expiry`.
- **Wire mirror (`celnet-proto`):** `Tenor.Unit` gained `UNIT_TOM_NEXT=4`, `UNIT_SPOT_NEXT=5`,
  `UNIT_IMM=6`, `UNIT_BROKEN_DATE=7` (existing `OVERNIGHT/WEEKS/MONTHS/YEARS` keep tags 0–3);
  `Tenor` gained `optional BrokenDate broken_date = 3`; new `message BrokenDate { int32 year;
  uint32 month; uint32 day }`. `count` carries weeks/months/years **or** the IMM ordinal. The
  `From`/`TryFrom` round-trip is total; a `UNIT_BROKEN_DATE` with no `broken_date` is a decode
  error (`WireError::MissingField`).
- **CLI shorthand (`celnet-cli`):** `TN`/`SN`/`<n>IMM`/`YYYY-MM-DD` now parse/format alongside
  `ON`/`<n>W`/`<n>M`/`<n>Y`.

### Smile-model selector — `celnet-types::SmileModel` + `celnet-proto`

- **`celnet_types::SmileModel { MarketHedge, StochasticVol, Parametric, ParametricSurface }`**
  (`Default = MarketHedge`). Mirrors `celnet_surface::SmileModel` exactly (vendor-neutral
  names; the vanna-volga / SABR / SVI / SSVI provenance is documented, not named).
- **Wire (`celnet-proto`):** `enum SmileModel { SMILE_MODEL_MARKET_HEDGE=0, …STOCHASTIC_VOL=1,
  …PARAMETRIC=2, …PARAMETRIC_SURFACE=3, …EXTENDED_SURFACE=4 }` (value **4 = eSSVI**, appended; no
  renumber, no `schema_version`); `MarkSurfaceRequest.smile_model` (field 4) and
  `ScenarioRequest.smile_model` (field 7), both `optional` — absent ⇒ server default
  calibration. The typed-provenance echo `ArbReport.smile_model` (field 5) also carries this enum
  (see the catalogue registry below). **Server status:** `MarkSurface` **honours** the selection — it
  routes to the matching `celnet-surface` calibrator (market-hedge VV baseline / fitted SABR / SVI /
  SSVI / eSSVI via `celnet_surface::build_model_smile`), deposits the model-tagged `CalibratedSmile`
  under the surface version (so a pinned RFQ/RFS re-prices against the exact marked model), and echoes
  the model used in `ArbReport.smile_model` (typed) plus the `Smile.arbitrage.note` token as
  provenance. The default reproduces
  the market-hedge baseline byte-for-byte. `Scenario` validates the selector but reprices each node
  off the supplied flat `base_market.vol`; with no broker skew a neutral smile collapses to the same
  flat slice, so the shock grid is model-invariant by construction (a model-dependent scenario marks
  a skewed surface first via `MarkSurface` + a pinned `surface_version`).

### Market-series feed (the TrendMode feed) — `celnet-proto`, multiplexed on `StreamSession`

- **`enum MarketObservable { …ATM_VOL=0, …SPOT=1, …RISK_REVERSAL=2, …BUTTERFLY=3, …FORWARD=4 }`**
  — labelled, unit-bearing observables (no abstract index).
- **`MarketSeriesSubscribe`** `{ SubscriptionId subscription; Underlying underlying;
  MarketObservable observable; optional Tenor tenor; optional double delta; uint64
  throttle_nanos; uint32 history_limit }` (tenor required for ATM_VOL/RR/BF/FORWARD; delta
  required for RR/BF). **`MarketSeriesUnsubscribe`** `{ SubscriptionId subscription }`.
- **`MarketSeriesPoint`** `{ SubscriptionId subscription; uint64 sequence; double value; int64
  epoch_nanos }`. **`MarketSeriesSnapshot`** `{ subscription; sequence; Underlying underlying;
  MarketObservable observable; repeated MarketSeriesPoint points; epoch_nanos }`.
- Wired as new `oneof` arms: `ClientStreamMessage.market_series_subscribe=7` /
  `market_series_unsubscribe=8`; `ServerStreamMessage.market_series_snapshot=7` /
  `market_series_point=8`. **Server status:** the feed is **served**. A
  `MarketSeriesSubscribe` opens a real subscription on the multiplexed `StreamSession`: the server
  sends a `MarketSeriesSnapshot` seeded with one freshly **observed** point, then appends
  `MarketSeriesPoint`s (strictly sequenced, conflated to `throttle_nanos`) as the live market state
  ticks. Each value is derived **on the core thread from the live `MarketState`** the hot path
  prices against (`CoreLink::observe`): SPOT/FORWARD/ATM_VOL read directly; RR/BF invert the live
  convention to the signed delta-wing strikes and read the live smile there — never a fabricated or
  recomputed proxy. A momentarily-underivable observable skips a point rather than inventing one; a
  lagging consumer drops a point (observables are conflatable) rather than back-pressuring the
  driver. The opening snapshot carries a single live observation only — the platform retains no
  durable observation store, so it does **not** backfill fabricated history (a durable store that
  lets the snapshot replay a real recent window is a documented future enhancement, not faked).

### Attribution identity (who's-trading) — `celnet-proto`

- **`message Owner { oneof seat { string trader; string auto_pricer } }`** — a human seat OR an
  automated pricer, uniformly. **`message BookId { string book; Owner owner }`**.
- **`message AttributionRecord { BookId quoted_by; optional BookId held_by; optional bool won;
  optional uint32 lp_count }`** — who quoted / holds / won, plus LP-in-competition count.
- Attached `optional AttributionRecord attribution` to `QuoteRequest`(6), `Quote`(11),
  `Execution`(7), `Subscribe`(7), `Snapshot`(12), `Executed`(8). **Server status:** the server
  **emits** the who's-trading chain (shared resolver `services::attribution`): every edge-quoted
  line is `quoted_by` the **maker auto-pricer** (`Owner::auto_pricer "celnet-auto-pricer"`, book
  `AUTO-MM`) — so engine-quoted flow is never anonymous — and a client-supplied requesting seat
  (its request `quoted_by`) becomes the `held_by`, carrying `won`/`lp_count` through verbatim. The
  chain flows `QuoteRequest`→`Quote`→`Execution` and `Subscribe`→`Snapshot`→click-to-trade `Executed`
  identically (API-first parity). The risk roll-up that *keys* on it is a later phase.

### Downstream crates touched to keep the contract compiling

`celnet-types`, `celnet-proto` (proto + `convert.rs`), `celnet-calendar`, `celnet-conventions`
(fallible `schedule`/`vol_year_fraction`, `vol_anchor`), `celnet-cli` (tenor shorthand),
`celnet-integration` (`vol_time` now `Result`; nominal-year-fraction arms for the new tenors),
`celnet-server` (stream/quote/surface services honour smile-model selection, serve the
market-series feed off the live state, and emit maker-auto-pricer attribution; the WS JSON codec
both decodes inbound and **emits outbound** attribution on `quote`/`execution`/`snapshot`/`executed`
frames — camelCase `quotedBy`/`heldBy`/`owner.{trader,autoPricer}`/`won`/`lpCount` — so the WS
mirror has full parity with the gRPC path, plus broken-date/smile-model/market-series),
`celnet-client` (typed SDK surface — see below), `celnet-bench` (test fixtures). All
gated green (fmt, clippy `-D warnings`, 768 nextest, cargo-deny).

**`celnet-client` SDK surface (Phase-1, consumes the contract — API-first parity with GUI/Excel).**
Every new capability is callable by an external client over the same contract:
- **Smile-model selection.** `Calibration` (re-exported `celnet_types::SmileModel`) on
  `Client::mark_surface_with` / `Client::scenario_with_model` /
  `Client::scenario_with_risk_and_model` (the no-model methods default to market-hedge). The
  model the server used is read off `ArbReport::note` (`model=<family>`), the provenance channel;
  `MarkedSurface::surface_version` is the pin handle.
- **Market-series feed (TrendMode).** `StreamSession::subscribe_series(pair, Observable, tenor?,
  throttle_nanos, history_limit) -> MarketSeries`, a typed async `Stream<SeriesEvent>`
  (`Snapshot { observable, points, … }` then live `Point`s), multiplexed on the SAME session as
  price subscriptions (separate id-keyed registry; ids never collide). `Observable` =
  `AtmVol | Spot | RiskReversal { delta } | Butterfly { delta } | Forward` (the wing delta rides
  the variant). `MarketSeries::unsubscribe()` tears it down. Values are the server's live
  observations, surfaced verbatim.
- **Attribution.** `Attribution { quoted_by: BookId, held_by, won, lp_count }`, `BookId { book,
  owner: Seat }`, `Seat::{Trader, AutoPricer}`. Set the requesting seat with
  `Rfq::with_attribution` (RFQ) / `StreamSession::subscribe_attributed` (RFS); read the
  server-resolved chain off `Quote::attribution`, `Execution::attribution`, and the streamed
  `StreamEvent::Snapshot { attribution, … }`. (`Execution` is now `Clone` not `Copy`, since it
  carries the chain.) Loopback contract tests in `tests/phase1_capabilities.rs` prove each call
  round-trips against a live in-process edge.

## Phase-2: `celnet-risk-normalize` — convention + numeraire boundary (RH §2.2/§2.3, EA P2-6)

The single-node, pure, deterministic (`libm`) library that turns per-book risk in heterogeneous
conventions into a **convention-free canonical leaf**, then expresses delta-by-ccy-leg and vega in a
chosen **reporting numeraire**. This is the boundary the (future) `celnet-risk-cube` sits on: above
this crate, a roll-up is a plain group-by + sum. It depends only on `celnet-vanilla` + `celnet-core`
+ `celnet-types`; **no IO, no market-data dependency** (conversion rates come from a caller-supplied
`SpotResolver`). It does **not** aggregate, and does **not** resolve cross-pair vol-triangulation sign
(§2.4) — that is the cube's non-additive reducer, deliberately out of scope here.

Public surface (`celnet_risk_normalize`):

- **Canonicalization (§2.2)** — `PositionRisk { pair, option, notional_base, inputs: VanillaInputs,
  quoted_delta: DeltaConvention, premium_style: PremiumStyle }` → `canonicalize(&PositionRisk) ->
  CanonicalLeaf`. The canonical convention is **spot-unadjusted delta, premium excluded**, re-derived
  from the pricing inputs (not the quoted number) so it is convention-independent. `CanonicalLeaf {
  pair, spot, greeks: CanonicalGreeks, premium_quote, vega_premium_ccy: Ccy,
  quoted_was_premium_adjusted }`; `CanonicalGreeks` carries `delta_base` (spot-unadjusted ×
  notional, a base-ccy hedge amount) + gamma/vega/theta/vanna/volga/charm/speed/zomma/color × notional.
  `CanonicalLeaf::premium_base_delta_adjustment()` reconstructs the premium-adjusted-delta offset
  (`premium_quote / spot` when base-paid, else 0).
- **Common-numeraire conversion (§2.3)** — `SpotResolver` trait (`numeraire()`,
  `rate_into_numeraire(ccy)`) + `StaticSpotResolver` table impl. `CurrencyExposure` is the
  per-currency netted delta vector (`add`, `add_leaf_delta` → `+delta_base` base leg / `−delta_base·spot`
  quote leg, `legs`, `amount_in`, `in_numeraire`, `CAP = 32`). `MonetaryAmount { ccy, amount }` with
  `in_numeraire`. `Numeraire::from_leaves(&[CanonicalLeaf], &resolver) -> Result<Numeraire,
  NumeraireError>` produces `{ numeraire, delta_vector, delta_numeraire, premium_numeraire,
  vega_numeraire }` — vega is converted through each leaf's **premium** currency (§2.2/§2.3 coupling),
  not the pair base. `NumeraireError::{MissingRate(Ccy), InvalidRate(Ccy)}` — a missing/invalid cross
  fails loudly, never silently dropping a leg.

Validated against worked examples (EURUSD-call positive base delta vs USDJPY-put negative; USD legs
netting across EURUSD+USDJPY at the currency node; premium-adjusted vs unadjusted canonicalizing
identically; vega converting JPY→USD through the premium ccy; the single-pair spot-delta hedge being
self-funding). 10 tests; `just check-crate celnet-risk-normalize` green (fmt, clippy `-D warnings`,
nextest, cargo-deny).

## Phase-2: `celnet-risk-cube` — single-node hierarchical risk cube (RH §2.1/§2.5/§3.2, EA P2-5)

The OLAP-style risk-aggregation engine sitting **above** the canonical leaf (`celnet-risk-normalize`)
and **below** the limits/entitlements/server-edge layers. It holds an immutable position-level fact
table keyed by the firm's **independent** organizational dimensions and answers the canonical OLAP
question: *group facts by any dimension at any level, reduce into a node measure.* Depends only on
`celnet-risk-normalize` + `celnet-vanilla` (bump-and-revalue) + `celnet-core` + `celnet-types`; **no
IO, no market-data** (rates arrive via the normalize crate's `SpotResolver`).

The load-bearing correctness split (RH §2.5):

- **Additive measures** roll up by **summation of canonical (convention-normalized) leaves**: net
  Greeks (`NetGreeks`: delta-base/gamma/vega/theta/vanna/volga/charm/speed/zomma/color + premium)
  and **vega bucketed by `(tenor × delta)`** (`VegaLadder`/`VegaPillar`). Associative/commutative, so
  a new fact touches O(depth) ancestor sums.
- **Non-additive measures** are **re-derived per node by bump-and-revalue** over the node's
  constituent positions, **never summed from children**: `historical_var_es` (VaR/ES — diversifying,
  ≠ Σ child VaRs), `sbm_curvature_spot` (FRTB-SbM MAR21 spot curvature — `max(CVR⁺,CVR⁻,0)`, charges
  short-gamma), `correlation_weighted_vega` (√ of the SbM `wᵀρw` quadratic form).

Public surface (`celnet_risk_cube`) for the limits/entitlements layer and the server/GUI:

- **Dimension model** (`dimension`) — interned org keys `PositionId`/`TraderId`/`BookId`/`DeskId`/
  `LocationId`/`EntityId` (cube-internal `u32` handles, **not** in the wire contract — the server maps
  `celnet-proto`'s `BookId`/`Owner` attribution onto them); `FactKey` (the independent-dimension FK
  tuple incl. `CcyPair`); `RiskFact { position_id, key, measure: FactMeasure{leaf, position},
  surface_version }` (immutable; one current fact per id; `surface_version`-stamped for the live/
  official-IPV/historical lens, RH §2.7); `Hierarchy` parent-pointer chains (`set_book_desk`,
  `set_location_entity`).
- **Cube** (`cube`) — `Cube::{new, with_hierarchy, hierarchy_mut, upsert, fact, len, is_empty}`;
  additive roll-up `group_by(DimensionId, &impl VegaPillarMap) -> Vec<NodeAggregate>` (+
  `firm_aggregate`), where `NodeAggregate { group, net_greeks, vega_ladder, positions, leaves }`
  retains constituents so a node is always reconcilable to its leaves (drill-down invariant) and
  `numeraire_view(&R: SpotResolver)` collapses cross-pair base-ccy deltas via `celnet-risk-normalize`;
  non-additive `Cube::{node_var_es, node_curvature_spot, node_correlation_weighted_vega}`. The
  `(tenor×delta)` pillar grid, FX curvature risk weight, and inter-bucket `rho` are all caller-supplied
  (regulatory data is **versioned external input**, never compiled-in, RH §2.3/§2.11).

Honest scope (documented in module docs, not faked): **bump-and-revalue is the reference oracle, not
the scale path** — AAD adjoint + batched-GPU (RH §3.3) is the named, **deferred** throughput lever (no
stub adjoint exists). **Single-node only** — distributed cross-shard reduction over the designed-only
`celnet-router` HRW map (RH §3.4) is out of scope; the additive **shard-merge seam** is
`NodeAggregate::merge_additive` (non-additive firm measures must re-derive from gathered facts). It
owns **no** convention logic (delegated down) and resolves no cross-pair vol-triangulation sign inside
the ladder (that is the caller-supplied signed `rho`, RH §2.4).

Validated: additive roll-up `Σ children == parent` and drill-down reconciliation; vega-ladder bucket
additivity; desk roll-up via book→desk parent pointer; VaR non-additivity (offsetting book ≈ 0 VaR)
and VaR/ES vs an independent quantile; FRTB curvature charging short-gamma only, cross-checked against
an independent reprice; correlation-weighted vega bounded by ρ=0 (Euclidean) and ρ=1 (sum), non-PSD
floored at 0; cross-pair numeraire collapse; shard-merge associativity; upsert supersede (no
double-count). 10 tests; `just check-crate celnet-risk-cube` green (fmt, clippy `-D warnings`,
nextest, cargo-deny).

## Phase-2: `celnet-limits` — limit framework / entitlements layer (RH §5, EA P2-7)

The pre/post-trade **limit framework** sitting **above** the `celnet-risk-cube` aggregation engine
(downstream of its group-by). The cube answers *"what is the netted/re-derived risk at this node?"*;
this crate answers *"is that within the limits set at this node, and what happens if a proposed trade
pushes it over?"*. Depends only on `celnet-risk-cube` (`NodeAggregate` / interned dimension keys /
`Hierarchy` / `Scenario`) + `celnet-risk-normalize` (`CanonicalLeaf` for the incremental-trade
projection) + `celnet-types`; **no IO, no market data** — pure and deterministic. Limits address by a
`LimitScope` that maps 1:1 onto a cube group, so the same `group_by`/`firm_aggregate` results drive
both the risk display and the limit check.

Public surface (`celnet_limits`) for the server/GUI to consume:

- **Taxonomy & thresholds** (`limit`) — `LimitMetric` (the RH §5.1 industry-standard set:
  `Delta`/`Gamma`/`Vega`/`Vanna`/`Volga`, `VegaBucket(VegaPillar)`, `TenorVega{tenor_days}`,
  `Concentration(ConcentrationMetric)`, `Var`/`ExpectedShortfall`, `StopLoss`); `LimitSpec`
  (`metric`, non-negative `cap`, amber/red warning bands, `Enforcement::{Soft,Hard}`) with
  `hard`/`soft` constructors (RH §5.2's illustrative 80 %/90 % default bands) + `with_bands` (clamped
  to a valid `0≤amber≤red≤1` ordering); first-class `utilization`/`classify` → `Utilization {ratio,
  status, headroom}` and the RAG `RagStatus::{Green,Amber,Red,Breach}` (severity-ordered).
- **The limit tree** (`tree`) — `LimitScope` (`Firm`/`Trader`/`Book`/`Desk`/`CcyPair`/`Location`/
  `Entity`; `dimension()`/`group_value()` map onto a cube group); `LimitTree::{set (supersede by
  `(scope,metric)`), at, has, len, is_empty, iter}`; `ScopePath::resolve(&FactKey, &Hierarchy)` — a
  position's simultaneous limit scopes (trader→book→desk→ccy-pair→location→entity→firm), resolving
  desk/entity through the cube's `book→desk`/`location→entity` parent pointers.
- **The checks** (`check`) — `exposure_of(node, metric, &NonAdditiveExposure)` reads the additive
  measure off a node (greeks/bucketed-vega/tenor-vega/gross-concentration) or the supplied
  non-additive loss (`Var`/`ES`/`StopLoss`, `0` when un-evaluated → never spuriously breaching);
  `NonAdditiveExposure::{from_scenarios (cube bump-and-revalue), with_stop_loss}`; `check_scope`;
  pre-trade `pre_trade_check(tree, &ScopePath, &IncrementalTrade, node_at, nonadditive_at)` →
  `PreTradeResult {decision: PreTradeDecision::{Accept,Warn,Reject}, checks}` (hard breach at any
  node rejects, soft breach warns, clean accepts; `IncrementalTrade::from_leaf` projects a proposed
  canonical leaf onto each path node before classifying); post-trade `post_trade_check` →
  `ScopeMonitor {worst: RagStatus, escalation: EscalationStatus::{Clear,SoftBreach,HardBreach},
  checks}`.

Honest scope (documented in module docs, not faked): the crate **evaluates** limits; it does **not**
own the RH §5.2 cascade *constraint solver* ("child limits constrained by parents" — a
configuration-time validation that belongs to the admin/`celnet-entitlements` path, P2-8, named and
deferred, not stubbed). Non-additive limits **consume** a per-node loss re-derived by the cube's
bump-and-revalue reducers (RH §2.5) on the recompute cadence (RH §3.3); the additive-Greek pre-trade
path is O(1) off the node aggregate, inside the µs-class budget (RH §3.5). Limit caps + warning bands
are external data (`LimitSpec` fields), never compiled-in.

Validated: utilization ratio + RAG band classification (green/amber/red/breach, sign-agnostic; at-cap
is red not breach; over-cap is breach); non-positive cap is a breach, never silently unbounded; custom
bands honoured + clamped; exposure extraction matching the node aggregate (delta/vega-bucket/tenor
cut); gross **concentration** ≫ netted greek for an offsetting book; non-additive VaR exposure from the
cube reducer (and `0` when un-evaluated); limit-tree supersede-by-metric; scope-path desk resolution
via parent pointer; pre-trade **hard breach rejects** (offending breach reported) + accept-within-
headroom + **soft breach warns (never blocks)**; post-trade breach detection + RAG/escalation
(hard/soft/clear). 11 tests; `just check-crate celnet-limits` green (fmt, clippy `-D warnings`,
nextest) + `cargo deny check` clean (no new deps — only internal crates).

## Phase-2: `celnet-entitlements` — server-side pre-aggregation pruning (RH §2.6/§4, EA P2-8/§3)

The "who sees what" layer beside the cube. A **principal** carries grant rules (the dimension
subtrees it may read) and deny rules (information barriers / Chinese walls); the **pruning predicate**
admits only the facts the principal may see and is applied **before** any roll-up, so a node total can
never leak the **magnitude** of a subtree the principal cannot see (the *aggregate-leakage* hazard,
RH §2.6). Pure & deterministic over integer dimension keys; **no IO, no float math, no `unsafe`**.
Depends only on `celnet-risk-cube` (`FactKey`/`Hierarchy`/`DimensionId`/`RiskFact`/`Cube`) +
`celnet-types` (`CcyPair`).

**Decision:** `admitted ⇔ (grant-all ∨ some grant covers) ∧ no deny covers` — **deny wins** over grant
(a hard cut even inside a granted subtree).

**Grant-all is the default & migration anchor.** `Principal::default() == Principal::grant_all()`
admits every fact, mirroring the GUI's `ScopeContext.principal = "grant-all"` (EA §3). The server flows
the fact stream through the filter **now** while the predicate is the identity (entitled cube ==
unfiltered cube), so swapping in a real scoped principal later changes **only which facts the predicate
admits** — zero rework at any aggregation call site, in the GUI, or in the wire contract (the P2-8
guarantee).

Public surface (`celnet_entitlements`):

- **`Scope { dimension: DimensionId, value: u64 }`** — pins one cube dimension to a subtree value. The
  `value` **is** the cube's group discriminant (`FactKey::group_value`), and `Scope::covers` resolves
  the `Book → Desk` / `Location → Entity` parent chains through the **same** `Hierarchy` the cube rolls
  up with — so a scope covers exactly the facts that roll into the matching node (one key space, no
  drift). Dimensions are orthogonal: a one-axis scope leaves every other axis wide.
- **`Rule`** — a **conjunction** of scopes (`Rule::on(dim, v).and(dim2, v2)` = "desk 99 AND entity
  London"); `Rule::firm()` (empty) covers everything.
- **`Principal::{grant_all, scoped, grant(Rule), deny(Rule), admits(&Hierarchy, &FactKey), is_grant_all,
  grants, denies}`** — `scoped()` is deny-by-default (sees nothing until granted, the §4
  separation-of-duties posture).
- **`EntitlementFilter::{new(&Principal, &Hierarchy), admits(&RiskFact), prune(&[RiskFact]) ->
  Vec<RiskFact>, entitled_cube(IntoIterator<Item=RiskFact>) -> Cube}`** — `entitled_cube` is the
  leak-free aggregation entry point: it builds a `Cube` over **only** admitted facts wired to the same
  hierarchy, so every subsequent `group_by` / `firm_aggregate` / VaR / curvature run is automatically
  leak-free at every node and drill level. `prune`/`admits` are the lower-level predicate for callers
  that own their fact store or gate an incremental upsert.

**Composition with the cube's group-by:** `fact stream → EntitlementFilter::entitled_cube(principal,
hierarchy, facts) → Cube (admitted only) → group_by / firm_aggregate / node_var_es / …`. Pruning is
strictly **upstream** of the cube's reduce; the cube is unchanged. This is the API-first seam the GUI's
`ScopeContext` already mirrors and `celnet-limits` checks against (a limit is evaluated over the
entitlement-pruned `NodeAggregate`, never the raw firm node).

Honest scope (module docs, not faked): this crate owns the **model + predicate** only. The **audit** of
each decision (RH §4: log via `celnet-observability`) is the server's at the call site — the crate is
pure and returns the decision, it does not log. Role↔principal assignment + the user-admin GUI half of
P2-8 live at the server/GUI edge above this crate.

Validated: grant-all sees everything (entitled cube == unfiltered, exact fact set + order); default
principal is grant-all; a scoped principal is **pruned to its subtree before aggregation** with the
firm total it sees equal to only-its-desk's facts and provably ≠ the true firm total (**no
aggregate-leakage**), and drill-down (group-by Book) yields only its books; one-axis grant keeps other
axes wide (orthogonality); conjunctive rule intersects axes; **deny wins** over grant-all (walled book
excluded from the total); scoped-with-no-grants admits nothing; single-fact `admits` matches `prune`.
8 tests; `just check-crate celnet-entitlements` green (fmt, clippy `-D warnings`, nextest). Adds **no**
external dependencies (only internal path crates), so it introduces no license/advisory surface for
cargo-deny.

## Phase-2 contract: `RiskService` — server-side hierarchical risk (RH §2.1/§2.3/§2.5/§5, EA P2-5..8)

The wire face of the single-node risk estate (`celnet-risk-cube` aggregation, `celnet-risk-normalize`
common-numeraire conversion, `celnet-limits` utilization/RAG, `celnet-entitlements` pre-aggregation
pruning). **Aggregation is owned by the server**: a client (GUI/SDK/Excel) never loops positions and
sums — it lists positions, asks for a rolled-up node tree over an org dimension, drills a node to its
constituents, and reads limit utilization, all behind the one `celnet-proto` contract. This closes the
**client-side aggregation parity violation** (the GUI Book view looping `transport.scenario` per
position). Added to `crates/celnet-proto/proto/celnet.proto`; **no** `celnet-types` change was needed
(the cube's org-dimension identifiers are interned `u32` handles, deliberately not in `celnet-types`,
RH §2.1 — the server maps a trade's wire `AttributionRecord` book/owner onto them).

### `RiskService` RPCs (one current contract, no versioning)

- **`ListPositions(ListPositionsRequest) → ListPositionsResponse`** — list the open positions the cube
  aggregates (the desk's book), each carrying its `AttributionRecord` so the hierarchy keys on real
  attribution. Optionally scoped + entitlement-pruned.
- **`AggregateRisk(AggregateRiskRequest) → AggregateRiskResponse`** — prune the fact stream by the
  `principal` **before** roll-up (no aggregate leakage), group by `dimension`, sum the additive
  measures + re-derive the non-additive ones per node, and convert everything into the reporting
  `numeraire`. Response = the rolled-up node tree.
- **`DrillRisk(DrillRiskRequest) → DrillRiskResponse`** — drill one node into child sub-nodes at a finer
  dimension and/or its contributing positions (the Book→Risk drill), entitlement-pruned.
- **`LimitStatus(LimitStatusRequest) → LimitStatusResponse`** — the limit tree + per-limit
  utilization/RAG for a scope node, with the `hard_breach` escalation flag.

### Messages, fields, types, defaults

**`RiskDimension`** (enum, mirrors `celnet_risk_cube::DimensionId` + the firm apex; ORTHOGONAL axes):
`FIRM=0` (apex; the cube `firm_aggregate`, default), `TRADER=1`, `BOOK=2`, `DESK=3`, `UNDERLYING=4`
(the asset axis; for FX the currency pair), `LOCATION=5`, `ENTITY=6`.

**`OrgKey`** (mirrors `celnet_risk_cube::FactKey`): `trader:u32`, `book:u32`, `desk:u32` (0 ⇒ resolve
from the book's `Book→Desk` parent pointer), `ccy_pair:CcyPair`, `location:u32`, `entity:u32` (0 ⇒
resolve from `Location→Entity`). Identifiers are the cube's interned dimension handles.

**`RiskPosition`** (mirrors a `RiskFact` + the originating `PositionRisk`): `position_id:u64`,
`org:OrgKey`, `option_type:OptionType`, `notional_base:double` (signed; + = long), `inputs:VanillaInputs`
(the canonical convention-free leaf is re-derived server-side via `canonicalize`, never sent),
`quoted_delta:DeltaConvention` + `premium_style:PremiumStyle` (provenance), `surface_version:u64`,
`attribution:AttributionRecord` (optional).

**`RiskScope`** (the `(dimension,value)` key space the cube groups by and an entitlement scope covers):
`dimension:RiskDimension`, `value:u64` (`FactKey::group_value`; for Desk/Entity the resolved ancestor
handle; ignored for `FIRM`).

**Entitlement principal** (mirrors `celnet_entitlements`): `EntitlementRule { scopes:repeated RiskScope }`
(conjunction; empty ⇒ covers everything = firm root); `EntitlementPrincipal { grant_all:bool,
grants:repeated EntitlementRule, denies:repeated EntitlementRule }`. **Default = grant-all**: a request
that **omits** the `optional EntitlementPrincipal` is treated by the server as grant-all (the GUI
`ScopeContext.principal = "grant-all"` show-all-now posture). A present principal with `grant_all=false`
and no grants is deny-by-default (sees nothing). `denies` apply to any principal (deny wins).

**Reporting numeraire** (mirrors a `SpotResolver`): `NumeraireRate { ccy:string, rate:double }` (units
of numeraire per 1 unit of `ccy` at spot; finite, >0); `ReportingNumeraire { numeraire:string,
rates:repeated NumeraireRate }` (the numeraire's own rate is implicitly 1.0; a missing rate fails the
request loudly — no silent leg drop).

**Vega pillar** (mirrors `celnet_risk_cube::VegaPillar`): `VegaPillar { tenor_days:u32, delta_bp:i32 }`
(0.25Δ → 2500). The pillar grid is **external data** (the `VegaPillarMap` contract) — supplied on the
request, never compiled in. `VegaLadderBucket { pillar:VegaPillar, vega:double }` (reporting numeraire).

**Node measures.** `CcyExposureLeg { ccy:string, amount:double }` (one signed leg of the netted delta
vector). `AdditiveRisk` (mirrors `NetGreeks` summed + the `Numeraire` collapse): `delta_numeraire`,
`delta_vector:repeated CcyExposureLeg`, `gamma`, `vega_numeraire` (converted through each leaf's PREMIUM
currency — the §2.2/§2.3 coupling), `theta`, `vanna`, `volga`, `charm`, `speed`, `zomma`, `color`,
`premium_numeraire`, `vega_ladder:repeated VegaLadderBucket`. `NonAdditiveRisk` (RE-DERIVED per node,
never summed): `var`, `es`, `var_alpha`, `curvature_spot` — all `optional double` (absent ⇒ not
evaluated this cycle, never a spurious zero). `RiskNode { dimension, group:u64, additive:AdditiveRisk,
nonadditive:NonAdditiveRisk, position_count:u32 }`.

**Requests/responses.**
`ListPositionsRequest { scope?:RiskScope, principal?:EntitlementPrincipal, correlation_id?:u64 }` →
`ListPositionsResponse { positions:repeated RiskPosition, correlation_id?:u64 }`.
`AggregateRiskRequest { dimension:RiskDimension, numeraire:ReportingNumeraire,
principal?:EntitlementPrincipal, scope?:RiskScope, vega_pillars:repeated VegaPillar (empty ⇒ server
default grid), var_spot_shocks:repeated double (empty ⇒ no VaR/ES), var_alpha:double (0 ⇒ 0.99 when
shocks present), curvature_risk_weight:double (0 ⇒ no curvature), correlation_id?:u64 }` →
`AggregateRiskResponse { dimension, numeraire:string, nodes:repeated RiskNode, correlation_id?:u64 }`.
`DrillRiskRequest { node:RiskScope, child_dimension:RiskDimension, numeraire:ReportingNumeraire,
principal?:EntitlementPrincipal, vega_pillars:repeated VegaPillar, include_children:bool,
include_positions:bool, correlation_id?:u64 }` → `DrillRiskResponse { node:RiskScope,
children:repeated RiskNode, positions:repeated RiskPosition, correlation_id?:u64 }`.
`LimitStatusRequest { scope:RiskScope, numeraire:ReportingNumeraire, principal?:EntitlementPrincipal,
vega_pillars:repeated VegaPillar, var_spot_shocks:repeated double, var_alpha:double, correlation_id?:u64
}` → `LimitStatusResponse { scope:RiskScope, limits:repeated LimitUtilization, worst:RagStatus,
hard_breach:bool, correlation_id?:u64 }`.

**Limits** (mirror `celnet_limits`): `LimitMetricKind` (DELTA=0, GAMMA, VEGA, VANNA, VOLGA, VEGA_BUCKET
(payload `vega_pillar`), TENOR_VEGA (payload `tenor_days`), CONCENTRATION_DELTA, CONCENTRATION_VEGA, VAR,
EXPECTED_SHORTFALL, STOP_LOSS); `RagStatus` (GREEN=0, AMBER, RED, BREACH — ordered by severity);
`Enforcement` (SOFT=0, HARD); `LimitUtilization { metric:LimitMetricKind, vega_pillar:VegaPillar,
tenor_days:u32, cap:double, exposure:double, ratio:double, status:RagStatus, enforcement:Enforcement,
headroom:double }`.

### WS JSON codec keys (gui/excel are WS clients)

The WS mirror is the **same contract, second encoding** (rule 9): every JSON object maps the proto
message field-for-field by the proto **snake_case** field name; every enum rides by its canonical proto
**enum number**; `optional` (presence-tracked) fields are `null`/absent when `None`. The request `type`
discriminator tags (the `{"type": "...", ...}` frame) and their reply tags are:

| Request `type` | maps to RPC | reply `type` |
|---|---|---|
| `list_positions` | `RiskService.ListPositions` | `list_positions_response` |
| `aggregate_risk` | `RiskService.AggregateRisk` | `aggregate_risk_response` |
| `drill_risk` | `RiskService.DrillRisk` | `drill_risk_response` |
| `limit_status` | `RiskService.LimitStatus` | `limit_status_response` |

JSON object keys (symmetric in/out, by proto field name): `dimension`, `group`, `value`, `scope`,
`node`, `child_dimension`, `principal` (`{grant_all, grants:[{scopes:[{dimension,value}]}], denies:[…]}`),
`numeraire` (`{numeraire, rates:[{ccy, rate}]}`), `vega_pillars`/`vega_pillar`/`pillar`
(`{tenor_days, delta_bp}`), `var_spot_shocks`, `var_alpha`, `curvature_risk_weight`, `positions`/
`position` (`{position_id, org:{trader,book,desk,underlying,location,entity}, option_type, notional_base,
inputs:{spot,strike,vol,t,discount_rate,carry}, quoted_delta, premium_style, surface_version, attribution}`),
`org`, `inputs`, `additive` (`{delta_numeraire, delta_vector:[{ccy,amount}], gamma, vega_numeraire,
theta, vanna, volga, charm, speed, zomma, color, premium_numeraire, vega_ladder:[{pillar,vega}]}`),
`nonadditive` (`{var, es, var_alpha, curvature_spot}` — `null` when absent), `position_count`,
`include_children`, `include_positions`, `limits` (`{metric, vega_pillar, tenor_days, cap, exposure,
ratio, status, enforcement, headroom}`), `worst`, `hard_breach`, `correlation_id`. `attribution` uses
the existing camelCase chain (`quotedBy`/`heldBy`/`won`/`lpCount`, `book`/`owner`/`trader`/`autoPricer`)
already in the WS codec. **The WS codec functions + dispatch arms are wired** (Server phase, DONE): the
codec lives in `celnet-server/src/ws/codec.rs` (the four `*_request_from_json` / `*_response_to_json`
fns) and the four dispatch arms in `celnet-server/src/ws/mod.rs`, both dispatching onto the same
`RiskEdge` impl the gRPC server hosts. `RiskPosition` is encoded outbound only (the server builds it from
the live book); there is no inbound `risk_position_from_json` (a client sends scope/principal/numeraire,
not positions) — no dead decoder.

### How the Server + Clients call it

- **Server** (`celnet-server`, Server phase — **DONE**): `services::risk` implements
  `risk_service_server::RiskService` (`RiskEdge`) over a shared `services::risk::store::PositionStore` —
  the live book the RFS click-to-trade path records each booked **vanilla** fill into
  (`StreamEdge::with_store` → `record_booked_position` → `store.book_from_attribution`, mapping the fill's
  `AttributionRecord` book/seat onto interned `OrgKey` `u32` handles). Each RPC takes a lock-free store
  snapshot (off the hot path), applies `EntitlementFilter::entitled_cube(principal, hierarchy, facts)`
  **before** any roll-up, `group_by(dimension)` / `firm_aggregate` for additive, `node_var_es` /
  `node_curvature_spot` over numeraire-scaled positions for non-additive, collapses to the reporting
  numeraire via `NodeAggregate::numeraire_view(SpotResolver)` (per-pillar for the vega ladder), and
  `check_scope` for `LimitStatus`. Wired into both the gRPC server (`RiskServiceServer`, added in
  `Edge::start_on`) and the WS mirror (four dispatch arms in `ws::handle_unary` over one `RiskEdge`), so
  GUI/Excel reach the identical path. Honest scope (reconciled to built reality): the **RFS
  click-to-trade auto-book** path (`record_booked_position`) and the wire-`RiskPosition` conversion
  (`risk::convert::position_to_fact`, `exotic: None`) auto-record **vanilla FX** fills only. The
  **cube/fleet aggregation itself is NOT vanilla-only**: `celnet_risk_cube::FactMeasure` carries
  `exotic: Option<ExoticLeg>` and is keyed on a cross-asset `Underlying`, so an **exotic** leg
  (`PositionStore::upsert_exotic` → real `ExoticLeg` Greeks as the additive carrier + exotic
  bump-and-revalue for non-additive VaR/curvature) and a **cross-asset vanilla** leg both roll up in
  the firm node and through `celnet-risk-fleet::fan_out_aggregate` (proven exotic-only by
  `celnet-parity/tests/exotic_risk_cube.rs::exotic_only_fan_out_reconciles`). The remaining gap is the
  **automatic ingestion of an exotic/cross-asset fill into the live store** (no production RPC/RFS path
  wires `upsert_exotic` yet — an exotic must be staged explicitly); the algebra and data model already
  cover it, never faked. A `LimitStatus` request scoped on
  `CCY_PAIR` is **rejected loudly** (a bare `u64` `RiskScope.value` cannot reconstruct the
  `base`/`quote` pair the limit tree keys on — `invalid_argument`, never a silent mis-scope); and the
  cross-pair / firm non-additive VaR/ES uses notional-scaled positions to express P&L in the common
  numeraire (the `historical_var_es` documented contract).
- **GUI** (Book view, client lane — **DONE**): the client-side
  `transport.scenario`-per-position loop is **deleted** (`gui/src/data/portfolioRisk.ts` removed).
  `gui/src/workspaces/BookWorkspace.tsx` now issues a single `aggregate_risk` call (group-by the
  dimension `gui/src/data/riskView.ts::dimensionForScope` derives from the toolbar Scope), a
  `limit_status` call for the scope's limit tree (a real Limits RAG panel with an honest empty-state),
  and a `drill_risk` (`include_positions`) for the Book→Risk drill. The reporting numeraire (USD) is
  assembled from the live watched-pair spots (`reportingNumeraire`), so the aggregate is in **real common
  units** via `celnet-risk-normalize` server-side — the "native premium units" caveat is **resolved**
  (the Book copy now names the reporting numeraire + shows a per-currency delta-vector breakdown; no
  stale disclaimer). Scope drives group-by + entitlement principal (grant-all today ⇒ principal omitted,
  server applies grant-all). The offline mock transport implements the same four RPCs by genuinely
  aggregating its deterministic seed book into a single firm node (no fabricated org hierarchy / limits).
  Typed messages + WS codec + transport methods live in `gui/src/data/{contract,enums,wsCodec,transport,
  wsTransport,mockSource}.ts` — no `celnet-types`/proto change (the Contract phase froze those).
- **SDK** (`celnet-client`, client lane — **DONE**): the four `risk_service_client` calls are exposed as
  ergonomic typed `Client` methods (`list_positions` / `aggregate_risk` / `drill_risk` / `limit_status`)
  with a proto-free risk vocab in `crates/celnet-client/src/risk.rs`; the omit-principal-is-grant-all
  convention is honoured. Validated by `tests/risk_workflow.rs` (7 tests) driving an in-process edge over
  gRPC — additive roll-up oracle (`firm == Σ book`), VaR diversification + presence-tracking, entitlement
  pruning before aggregation, drill reconciliation, limit breach/headroom.
- **Excel** (`CELNET.*`, client lane — **DONE**): `=CELNET.RISK(dimension, numeraire, [rates], [scope])`
  / `=CELNET.POSITIONS([scope])` / `=CELNET.LIMITS(scope, numeraire, [rates])` over the WS
  `aggregate_risk` / `list_positions` / `limit_status` frames; the WS codec lives in
  `excel/src/contract/riskCodec.ts`. Validated by 16 unit tests + e2e Check H (books a real
  click-to-trade position, then asserts the FIRM apex roll-up equals the Σ over BOOK nodes in USD,
  server-aggregated).

**Build status (this contract).** The `celnet-proto` contract is **frozen and complete**; the
`celnet-server` `RiskService` impl (gRPC + WS mirror, wired to the shared live `PositionStore`) is
**DONE and gated green**, and **all three client lanes (GUI / SDK / Excel) are now DONE** — full
API-first parity reached: the GUI Book, the `celnet-client` SDK, and the Excel `CELNET.*` functions all
consume the identical server-owned aggregation, with no client-side position-loop-and-sum anywhere and
no contract change. Default principal = **grant-all** everywhere (entitlement-ready, show-all-now; a
request omitting a principal is grant-all). Validated by the proto round-trip, the server suite (62 lib
+ 92 integration tests), the SDK risk-workflow suite (7 tests), the Excel suite (75 tests) + e2e Check H,
the GUI `npm run build`, and full `just check` green (791 tests).

---

## Exotic catalogue + booking-model + provenance — `celnet-proto` `Instrument` extensions (api-first program W1–W6)

The api-first completion program additively extended the **one** `Instrument` oneof and its
supporting enums/fields so the full built `celnet-exotics` / `celnet-surface` catalogue is reachable
from the wire and from all five clients (server pricer + SDK + CLI + Excel + GUI). **No
`schema_version`, no renumber** — every tag is appended; an absent product/field reproduces the
pre-extension contract byte-for-byte. The frozen registry of additions:

### `Instrument.product` oneof arms (each: proto field number → server pricer → parity row)

| Product | oneof arm (field #) | Server pricer (`celnet-server/src/pricer.rs`) | Independent parity/gate |
|---|---|---|---|
| Variance swap | `VarianceSwap variance_swap = 13` | `Product::VarianceSwap` → `celnet-exotics` log-contract 1/K² replication | `celnet-parity/tests/var_vol_swap.rs` (flat-σ `K_var==σ²`; independent adaptive-Simpson strip) |
| Volatility swap | `VolatilitySwap volatility_swap = 14` | `Product::VolatilitySwap` → Carr-Lee convexity adjustment | `celnet-parity/tests/var_vol_swap.rs` (`K_vol<√K_var` widening) |
| Asian (arithmetic) | `AsianOption asian_option = 15` | `Product::AsianOption` → Turnbull-Wakeman / Curran (uses `AsianMethod`) | `celnet-parity/tests/asian.rs` (closed-form limits; Curran vs MC stderr; TW band) |
| Forward-start vanilla | `ForwardStart forward_start = 16` | `Product::ForwardStart` → Rubinstein (1990) dual-carry reset | `celnet-parity/tests/forward_start.rs` (t1→0 → GK ~1e-9; vs two-leg GBM MC) |
| Cliquet / ratchet | `Cliquet cliquet = 17` | `Product::Cliquet` → Σ forward-start legs (plain) / clamped MC | `celnet-parity/tests/forward_start.rs` (plain == Σ legs ~1e-10; capped MC) |
| Quanto | `Quanto quanto = 18` | `Product::Quanto` → vanilla/digital closed form (uses `QuantoPayoff`) | `celnet-parity/tests/structured.rs` (ρ=0 → vanilla; CF == MC) |
| TARF | `Tarf tarf = 19` | `Product::Tarf` → geared-fixing MC (uses `TarfRedemption`; emits `price_std_error`) | `celnet-parity/tests/structured.rs` (gap-risk premium signed; server==exotics MC) |
| Accumulator | `Accumulator accumulator = 20` | `Product::Accumulator` → pivot-accumulation MC (uses `AccumulatorMonitoring`; emits `price_std_error`) | `celnet-parity/tests/structured.rs` (continuous KO > discrete) |
| Lookback | `Lookback lookback = 21` | `Product::Lookback` → Goldman-Sosin-Gatto / Conze-Viswanathan closed form, discrete MC (uses `LookbackStyle`/`LookbackMonitoring`) | `celnet-parity/tests/structured.rs` (CF==MC; dominates vanilla) |
| Window barrier | `WindowBarrier window_barrier = 23` | `Product::WindowBarrier` → LSV engine only (no closed form; requires `PRICING_MODEL_LOCAL_STOCH_VOL`) | `celnet-parity/tests/lsv.rs` (LSV oracle: ξ=0→Dupire limit; PDE≈MC) |
| American / Bermudan | `AmericanOption american = 24` | `Product::American` → projected-SOR free-boundary FD (default) / Longstaff-Schwartz LSM when `lsm_paths>0` (uses `ExerciseStyle`; emits `price_std_error` for LSM) | `celnet-exotics/src/american.rs` (American≥European; no-foreign-rate==European; PSOR-FD==LSM-MC within stderr) |
| Basket / best-of / worst-of | `BasketOption basket = 25` | `Product::Basket` → Cholesky-correlated multi-asset GBM MC over scrambled-Sobol/Brownian-bridge (uses `BasketKind`; emits `price_std_error`) | `celnet-parity/tests/basket.rs` (Lévy two-asset band; worst≤single≤best sandwich; ρ→1 collapse) |

(The pre-program arms `vanilla=7`, `strategy=8`, `single_barrier=9`, `double_barrier=10`,
`digital=11`, `touch=12` are unchanged; W4 added their ergonomic client ctors only, no contract
change.)

### Supporting enums (`celnet-proto`, all appended)

`AsianMethod` (Asian averaging family), `QuantoPayoff` (vanilla/digital), `TarfRedemption`
(full-gain/capped-gain), `AccumulatorMonitoring` (discrete/continuous), `LookbackStyle`
(floating/fixed) + `LookbackMonitoring`, `BasketKind` (basket/best-of/worst-of), `ExerciseStyle`
(American/Bermudan/European). Each maps 1:1 to a `celnet-exotics` enum in the server pricer (named
imports `… as Ex…`).

### Booking-model selector — `Instrument.pricing_model = 22`

`enum PricingModel { PRICING_MODEL_DEFAULT=0, PRICING_MODEL_LOCAL_STOCH_VOL=1 }`. Travels on the
`Instrument` so it reaches price/quote/stream/scenario uniformly. **`DEFAULT` (or absent) is
byte-identical** to the contract before this field existed (`to_bits`-gated). `LOCAL_STOCH_VOL`
routes the supported products (vanilla, single-barrier KO, window-barrier) through the real
`celnet-exotics::LsvModel`; an unsupported product returns a clear `invalid_argument` (no silent
fallback). Parity: `celnet-parity/tests/lsv.rs`.

### MC-honesty std-error fields

- **`PriceResponse.price_std_error = 7`** (`optional double`) and **`Quote.price_std_error = 12`**
  (`optional double`) — the Monte-Carlo products (clamped cliquet, TARF, accumulator, discrete
  lookback, American-LSM, basket) emit a genuine standard error on **both** the gRPC price path and
  the WS quote path (the GUI/Excel transport). Gated by a `quote_to_json` unit test + SDK e2e
  asserting an MC product's quote carries stderr while a closed-form product does not.

### Typed smile provenance — `ArbReport.smile_model = 5`

`ArbReport.smile_model` (field 5, `SmileModel`) carries the **typed** calibrated-family identity,
retiring the `model=<family>` regex previously parsed out of `Smile.arbitrage.note` across GUI /
Excel / SDK. Values per the `SmileModel` enum above, including `SMILE_MODEL_EXTENDED_SURFACE=4`
(eSSVI). Parity/round-trip in `celnet-parity/tests/essvi.rs` + the client codec tests.

## Asset-class universe, linear products, RFQ-to-many + settlement mechanics — `celnet-proto` registry (W1–W5 cross-asset waves + the in-flight payoff arms)

The cross-asset waves (W1 carry seam, W2 linear + metals, W3 crypto, W4 RFQ-to-many, W5
equity/commodity) extended the **same one contract** additively. The frozen registry —
every symbol cited at its `crates/celnet-proto/proto/celnet.proto` line so nothing is
present-but-unregistered:

### The asset-class discriminator + agnostic carry seam (W1/W2/W3/W5)

There are **no per-class product arms**: a cross-asset option is the *existing* product
arm priced over a non-FX `Underlying` arm + the generalized carry (ADR-0008).

| Symbol | proto | Notes |
|---|---|---|
| `message Underlying` | `celnet.proto:390` | `oneof ref`: `CcyPair fx = 1` (:394), `MetalPair metal = 3` (:398), `EquityRef equity = 4` (:402), `CommodityRef commodity = 5` (:406), `CryptoPair digital_asset = 6` (:410); `settlement_ccy = 2` (:415) |
| `enum Metal` | `celnet.proto:151` | the four LBMA/LPPM metals (XAU/XAG/XPT/XPD); projects byte-identically onto the metal-base `CcyPair` the convention/calendar registries key on |
| `message MetalPair` | `celnet.proto:423` | `Metal metal = 1`, `string quote = 2` — metal-vs-FIAT only |
| `message Symbol` | `celnet.proto:437` | `ticker = 1`, `venue = 2` — the vendor-neutral instrument symbol; reused by `EquityRef`/`CommodityRef` and `ListedFutureOption.future_symbol` |
| `message EquityRef` | `celnet.proto:449` | `Symbol symbol = 1`, `currency = 2` |
| `message CommodityRef` | `celnet.proto:461` | `Symbol symbol = 1`, `currency = 2` |
| `message CryptoPair` | `celnet.proto:475` | `base = 1`, `quote = 2` (UTF-8 tickers); linear/inverse mechanics live on `Instrument.settlement_style`, not the pair identity |
| `message FxRates` | `celnet.proto:487` | `r_for = 1` — the FX two-rate carry arm (`discount_rate` carries `r_dom` out of band) |
| `message CostOfCarry` | `celnet.proto:496` | `b = 1` — generalized net carry `F = S·e^{b·t}` (equity dividend yield / commodity storage-convenience / crypto funding) |
| `message CarryModel` | `celnet.proto:505` | `oneof model`: `FxRates fx = 1` / `CostOfCarry generalized = 2`; attached at `MarketContext.carry = 4` (:649) and `VanillaInputs.carry = 6` (:670) |
| `message RateSensitivities` | `celnet.proto:519` | `oneof sensitivities`: `FxRho fx = 1` (`rho_dom`/`rho_for`) / `CarryRho carry = 2` (`discount_rho`/`carry_rho`); attached at `Greeks.rate_sensitivities = 15` (:734); the flat `rho_dom`/`rho_for` field numbers 7/8 are `reserved` (:716–717) |

Cross-asset dispatch: `celnet-server/src/pricer.rs` routes a non-FX underlying through the
generalized-carry path (`pricer.rs:1705` ff.) onto the leaf engines (`celnet-equity-vanilla` /
`celnet-commodity-vanilla` / `celnet-crypto-vanilla`). Gates: golden vector families
`vectors/{equity_option,commodity_option,crypto_option}.json`, independent parity
`celnet-parity/tests/crossasset.rs`, and the CLI three-way gate
(`celnet-cli/tests/conformance.rs`: CLI == server == independent oracle per class).

### Contract settlement mechanics

- **`enum SettlementStyle`** (`celnet.proto:120`): `SETTLEMENT_STYLE_LINEAR = 0`
  (meaningful-zero default — byte-identical to the pre-field contract) /
  `SETTLEMENT_STYLE_INVERSE_COIN = 1` (the coin-margined `1/S_T` digital-asset convention).
  Carried on **`Instrument.settlement_style = 29`** (`celnet.proto:1266`), travelling on the
  instrument like `pricing_model` so it reaches price/quote/stream/scenario uniformly.
- **`enum Margining`** (`celnet.proto:138`): `MARGINING_EQUITY_STYLE = 0` (premium-upfront,
  discounted; meaningful-zero default) / `MARGINING_FUTURES_STYLE = 1` (daily-margined,
  undiscounted). Carried on `ListedFutureOption.margining = 6` (`celnet.proto:1432`; a
  landing arm — see below).
- **`enum FixingSource`** (`celnet.proto:167`): the six EMTA/ISDA per-currency NDF
  settlement-rate identities (KRW KFTC18 / TWD TAIPEI / INR RBI / BRL PTAX / CLP DÓLAR OBS /
  COP TRM) — convention **identity only**; live fixing VALUES remain an estate-gated feed,
  never sourced in-repo. Carried on `Ndf.fixing = 4` (`celnet.proto:1382`).

### Linear (non-option) `Instrument.product` arms — built + gated

| Product | oneof arm (proto line) | Server pricer | Independent gate |
|---|---|---|---|
| Outright forward (deliverable FX/metal) | `FxForward fx_forward = 26` (`celnet.proto:1317`; message :1344) | `pricer.rs:1597` → `celnet-linear` closed-form DCF (exact; no std-error) | `celnet-parity/tests/linear.rs`; golden `vectors/fx_forward.json` |
| FX swap (near + far leg, opposite sides) | `FxSwap fx_swap = 27` (`celnet.proto:1320`; message :1358) | `pricer.rs:1598` → independent leg-PV sum | `celnet-parity/tests/linear.rs`; `vectors/fx_swap.json` |
| NDF (non-deliverable; settles in the convertible ccy) | `Ndf ndf = 28` (`celnet.proto:1324`; message :1373; `fixing = 4` :1382, `settlement_ccy = 5` :1386) | `pricer.rs:1599`; an NDF on a deliverable pair is rejected `INVALID_ARGUMENT` | `celnet-parity/tests/linear.rs`; `vectors/ndf.json` |

### Landing `Instrument.product` arms — in-flight new-payoff-shapes wave (committed at wip checkpoint `c8fb02f`; full-workspace gates pending)

| Product | oneof arm (proto line) | Server pricer | Gate (landing) |
|---|---|---|---|
| Perpetual American option (no expiry; `Instrument.expiry_years` MUST be 0 — `convert::validate_perpetual_terms`) | `PerpetualOption perpetual_option = 30` (`celnet.proto:1329`; message :1396) | `pricer.rs:1604` (FX two-rate carry) + the cross-asset carry path (:1724) | `celnet-parity/tests/perpetual.rs`; `vectors/perpetual_option.json` |
| Listed-future option, any asset class (`future_expiry_years >= expiry_years > 0` — `convert::validate_listed_future_terms`) | `ListedFutureOption listed_future_option = 31` (`celnet.proto:1334`; message :1414; `margining = 6` :1432) | `pricer.rs:1612` | `celnet-parity/tests/listed_future.rs`; `vectors/listed_future_option.json` |

### RFQ-to-many (multi-dealer panel) — built + gated (`celnet-rfq`, W4)

| Symbol | proto | Notes |
|---|---|---|
| `rpc QuoteService.RequestMultiDealerQuote(QuoteRequest) returns (MultiDealerQuote)` | `celnet.proto:2897` | idempotent on the request's `idempotency_key`; the SAME `QuoteRequest` — no new request type |
| `message DealerQuote` | `celnet.proto:1561` | `lp_id = 1` (:1563), `TwoWayPrice price = 2`, `Greeks greeks = 3`, `resolved_strike = 4`, `valid_until_nanos = 5` (per-dealer last-look), `optional AttributionRecord attribution = 6`, `optional double price_std_error = 7` |
| `message MultiDealerQuote` | `celnet.proto:1589` | `quote_id = 1` (the aggregate request id), `idempotency_key = 2`, `repeated DealerQuote dealers = 3` (ordered best-first, :1596), `best_bid_lp_id = 4` (:1599) / `best_offer_lp_id = 5` (:1602), `Conventions conventions = 6`, `epoch_nanos = 7`, `optional correlation_id = 8`, `optional surface_version = 9` |
| `QuoteAccept.lp_id = 4` | `celnet.proto:1630` | selects the winning `DealerQuote.lp_id`; empty selects the single-dealer quote — byte-identical to a pre-field accept |

Engine: `celnet-rfq` (concurrent fan-out, ranking, deterministic tie-break, last-look)
behind the server edge; gated by `celnet-server/tests/multi_dealer.rs` + `tests/rfq.rs`
(≥3 loopback LPs, external-winner booking, WS mirror). Client reach (commit `1ebd742`):
SDK `Client::request_multi_dealer_quote` → `MultiDealerRfq` (+ the `multi_dealer_trade`
example), CLI `rfq`, Excel `CELNET.RFQ` (ranked panel spill; the task-pane accept books by
`(quote_id, lp_id)`), GUI `DealerPanel`.

---

## Fixed-income / rates subsystem — `celnet-rates` + the rates server services

The fixed-income/rates wave built a **multi-curve term-structure** leaf and **wired it into the
server**, retiring the earlier "deferred / built-but-unwired" status. `celnet-rates` depends only on
`celnet-types` + `celnet-calendar` (no IO). The convergence direction onto the carry seam is recorded
in **ADR-0010** (a *design direction*, not yet implemented — the flat-carry FX path stays
authoritative; `celnet-rates` stands side-by-side with the carry core today).

### `celnet-rates` public surface (verified against the graph)

- **Curve / term structure** (`curve.rs`) — `Curve` (log-linear-DF interpolation), `Node`,
  `Interpolation`; `bootstrap_ois`, `ln_df`, `discount_factor`, `zero_rate`, `segment_forward`. A
  bootstrapped discount/forward term structure (`Carry` is its degenerate flat single-point case — the
  ADR-0010 insight). Year-end turn adjustments live in `turns.rs` (`TurnJump`); the root-finder in
  `solver.rs` (Brent).
- **Bootstrap** (`bootstrap.rs`) — `OisQuote`, `BootstrapError`, `bootstrap_ois` (curve from OIS quotes).
- **Linear products** — FRA (`fra.rs`: `Fra`/`FraRisk`, `fra_pv`/`fra_pv01`/`fra_par_rate`/`fra_risk`);
  OIS (`ois.rs`: `OisSchedule`, `ois_pv`/`ois_annuity`/`ois_par_rate`); IRS / vanilla swap
  (`vanilla_swap.rs`: `VanillaSwap`/`SwapLeg`/`SwapRisk`/`PaymentFrequency`,
  `fixed_annuity`/`swap_pv`/`swap_par_rate`); STIR futures (`futures.rs`: `StirFuture`/`Deliverable`,
  `stir_futures_price`/`stir_futures_rate`/`stir_forward_rate`); cash bonds (`bond.rs`: `CashBond`,
  `bond_pv`/`price_at_yield`/`yield_to_maturity`/`z_spread`/`g_spread`/`asset_swap_spread`).
- **Curve risk** (`risk.rs`) — `OisRisk`, `ois_risk`: parallel DV01 **and** a key-rate-DV01 ladder by
  finite difference (re-bootstrap-and-bump).

### Server wiring (the rates services + the FI dealer desk)

- **`celnet-server::rates_pricing`** consumes the leaf: `price_ois` / `par_rate_for` / `build_quotes`
  call `bootstrap_ois` / `ois_par_rate` / `ois_risk` (11 CALLS edges from `rates_pricing.rs` into
  `celnet-rates`). This is the live in-edge that closes the old "zero runtime in-edges" status.
- **Wire (`celnet-proto`):** `RatesService` RPCs — `PriceRates`, `AggregateRatesRisk`,
  `BookRatesPosition`, `ListRatesPositions` — with the `RatesInstrument` / `OisInstrument` / `CurveSet`
  message arms (`RiskService.AggregateRatesRisk` is, per ADR-0010, a typed projection over the same
  cube, not a permanent silo). The rates book store is `celnet-server::services::rates_book`
  (`RatesPositionStore`); the rates-risk fact path is `services::rates_risk` (with its own
  `RatesFleetReducer` in `celnet-risk-fleet::rates`).
- **FI dealer-quoting desk + `NotificationService`:** the dealer desk lives in
  `celnet-server::services::desk` (`RfqDeskEdge`). `NotificationService.StreamNotifications` (proto)
  is implemented by `RfqDeskEdge::stream_notifications` with a WS mirror
  (`ws::mod::handle_subscribe_notifications`) and GUI/mock clients (`streamNotifications`) — wired
  end-to-end, not proto-only. Desk actions are capability-gated (see §"Permissions / administration").

## Permissions / administration — the action-capability kernel (`celnet-entitlements::capability`)

A just-merged permissions/administration program adds a **capability kernel** on top of the
pre-aggregation entitlement predicate. It lives in `celnet-entitlements::capability` (pure, no IO).

- **Action × AssetClass × desk, deny-wins.** `Action` is a 9-variant enum (`View`, `Price`,
  `QuoteRespond`, `RfqRespond`, `IoiRespond`, `Stream`, `Execute`, `Book`, `Administer`); `AssetClass`
  (with `label`/`from_label`) is the asset axis; the desk dimension comes from
  `celnet-risk-cube::DimensionId::Desk`. A `Capability` (`Capability::new`, `action` field) is granted
  or denied in a `CapabilitySet` (`grant`/`deny`). The decision is **deny-wins**:
  `deny_wins_over_grant` / `deny_wins_over_grant_all` (`capability.rs`) and the information-barrier
  variant `deny_wins_over_grant_information_barrier` (`lib.rs`) — a `deny(Rule::on(Book,b))` overlay
  excises that book's facts even from a firm-wide `grant_all`.
- **Per-user capability overlay — persisted + over the wire.** Config: `UserDef.capability_overlay`
  (`celnet-server::config::identity`) parses per-user grants/denies. Session: snapshotted via
  `sessions::from_user_snapshots_capability_overlay` (the overlay widens/narrows the role bundle).
  Persistence: `AuthEdge::persist_and_commit`, projected with `overlay_to_wire`/`user_to_wire`. Wire:
  the proto `GetUserCapabilities` / `SetUserCapabilities` RPCs.
- **GUI capability-matrix admin editor.** The GUI consumes `GetUserCapabilities`/`SetUserCapabilities`
  to render and edit the per-user Action × AssetClass × desk matrix (API-first — the same contract the
  server enforces).
- **FI-desk authz gating.** `BookRatesPosition` requires the `Book · FixedIncome` capability:
  `RiskEdge::book_rates_position` is gated (test
  `risk::mod::book_rates_position_requires_book_capability` — an unauthenticated `grant_all` body
  principal is **denied under Enforce**, the production posture).

> The pre-aggregation `EntitlementFilter` predicate (the §"Phase-2 `celnet-entitlements`" section above)
> is the dimension-subtree pruning layer; the capability kernel here is the orthogonal *which-Action-on-
> which-AssetClass-at-which-desk* gate. Both are deny-wins and both compose **upstream** of any roll-up.

## Streamed carry seam (item A) — `celnet-server` per-underlying fan-out + the streamed `RateSensitivities` arm

- **Per-underlying fan-out, deterministically seeded.** `celnet-server::services::pricefanout`:
  `underlying_seed(&Underlying) -> u64` derives a deterministic seed from an `Underlying` identity
  key; `producer_loop` keeps a per-`Underlying` `PairProducer` (its own `BroadcastRing<PriceTick>`),
  so each distinct underlying fans out independently. FX is byte-identical to the prior pair-seed
  (`underlying_seed_fx_is_byte_identical_to_pair_seed`).
- **`RateSensitivities` on the streamed Snapshot/Update.** `pricer::streamed_rate_sensitivities`
  emits the asset-class-correct arm — `RateSensitivities::Fx { rho_dom, rho_for }` for an FX
  underlying, `RateSensitivities::Carry { discount_rho, carry_rho }` for a cross-asset underlying —
  and `services::stream::streamed_wire_greeks` threads it onto the `greeks` of every
  `ServerStreamMessage::Snapshot` / `Update` (`cross_asset_subscribe_streams_the_carry_arm_through_the_session`).

## Carry → sensitivity mapper (item F) — single-source in `celnet-types`

The carry↔flat-rho bijection and the variant→arm selector are single-sourced in **`celnet-types`**
(where `Carry` and `RateSensitivities` are defined — orphan-rule placement, not `celnet-core`):

- **`celnet_types::RateSensitivities`** — enum with `Fx { rho_dom, rho_for }` and
  `Carry { discount_rho, carry_rho }`. `RateSensitivities::flat_rhos(&self) -> (f64, f64)` projects
  BOTH arms to a common flat `(rho_dom_eff, rho_for_eff)` pair: `Fx` passes through; `Carry` returns
  `(discount_rho + carry_rho, -carry_rho)`. A proto mirror (`celnet-proto::helpers`) carries the same
  method on the decoded type.
- **`celnet_types::Carry::rate_sensitivities(&self, a, b) -> RateSensitivities`** — the variant→arm
  selector: a `Carry::FxRates` carry tags the scalar Greek pair `(a,b)` as `RateSensitivities::Fx`, a
  `Carry::CostOfCarry` carry tags it as `RateSensitivities::Carry`. Adding a third `Carry` arm is a
  one-edit change here. This collapsed the previously-duplicated server flatten/wrap sites; FX is
  byte-identical (parity cross-asset corpus).

## Dormant-crate activation (item D) — plugin-host into the engine path; xva/replog deferred

The arch program's three remaining built-but-unreachable leaves are resolved here: the plugin host
is **wired into the live pricing path**, and `celnet-xva` / `celnet-replog` are **explicitly deferred**
with a dated activation plan (no crate is left silently dormant). `celnet-rates` was already wired
(see the "Fixed-income / rates subsystem" section above) and is untouched.

### `celnet-plugin-host` → `ProductEngine` dispatch — ACTIVE (2026-06-30)

Now that architecture item C landed the `ProductEngine` registry, the plugin host's
`celnet_plugin_host::registry::ModelRegistry` is reachable from the server's analytic dispatch, so a
**registered house pricing model serves the vanilla arm through the same decode → guard → dispatch
seam as a native arm** — the dispatch→`ModelRegistry` edge the deliverable targets.

- **The seam (`celnet-server::pricer::engines`).** `EngineCtx` carries
  `plugin_models: Option<&ModelRegistry>`. `engines::dispatch`'s `Vanilla` arm consults it: a
  registered model is resolved by the non-allocating hot-path resolver
  `ModelRegistry::active_pricing_model()` (the first `ModelKind::Pricing` handle, insertion-order
  authoritative — added to `celnet-plugin-host` so the engine never calls the allocating
  discovery-trait `of_kind`), then priced by `PluginModelEngine` (a `ProductEngine`). It builds the
  generalized carry-tagged `celnet_core::CarryInputs` for the **FX two-rate arm** — exactly the
  `(spot, strike, vol, t, r_dom, r_for)` the native `VanillaEngine` prices off — calls
  `HostModel::price_and_greeks`, and maps the carry-tagged `CarryGreeks` back through the single
  source of the carry↔flat-rho bijection (`pricer::carry_greeks_to_greeks`).
- **Byte-identity preserved.** Only the `Vanilla` arm consults the registry, and only when a model
  is registered AND the instrument carries a decodable FX/metal underlying; the other arms stay pure
  static dispatch, and absent any installed model every arm is the verbatim native path. The native
  FX/analytic corpus is unchanged bit-for-bit (the parity/determinism gate). The reference
  `FlatSmilePricer` (whose FX arm reproduces Garman-Kohlhagen) is proven to agree with the native
  strip through the seam — Tier-0 interchangeability — by the `engines` `plugin_dispatch_tests`.
- **Live wiring (per-worker).** The registry is **per-worker** by design (`ModelRegistry` is `!Sync`
  — a model handle, especially a sandboxed Tier-2 one, is owned by one worker), so it is threaded
  through thread-local worker state, not a shared global. `pricer::price_instrument` dispatches via
  `engines::dispatch_live`, which reads the calling worker's installed registry. A server activates
  house models by calling the public `celnet_server::pricer::install_house_models(registry)` once per
  pricing worker — for the async edge, from the tokio runtime's `on_thread_start` hook. With no
  models installed (the default), the path is the verbatim native dispatch.
- **Hot path.** The native fast path stays allocation/lock/log-free: when no registry is installed,
  dispatch is the static `match` and the only added work is a thread-local read + a `None` check. The
  plugin route's input clone (decoding the underlying) happens only on the opt-in, model-registered
  path, never on the native FX hot path.

### `celnet-xva` and `celnet-replog` — **ACTIVATED** (verified 2026-08-17)

Both were deferred on 2026-06-30 for want of a clean RPC seam. **Both have since landed** — this
section is retained as the activation record rather than deleted, because the *activation plans*
below are what was actually built. Verified against `origin/main` on 2026-08-17:

- **`celnet-xva`** (`compute_xva(&XvaInputs) -> XvaResult` — CVA/DVA/FVA over a survival-curve grid,
  `cva.rs`). Activation requires a new `XvaService` proto (request/response messages for the
  hazard/LGD/EPE-ENE/funding inputs), a `handle_unary` `"xva"` arm + WS-mirror codec, and the
  `celnet-xva` server dep. It is a **portfolio-level post-trade analytic over an exposure profile**,
  not a per-instrument price arm, so it does not belong on the `ProductEngine` dispatch seam.
  **DONE:** `rpc PriceXva` on `PricingService` (`celnet.proto:3110`), the server edge
  (`celnet-server/src/xva_pricing.rs`), the `"price_xva"` WS arm (`ws/mod.rs:883`) and the
  `celnet-xva` server dep (`Cargo.toml:57`) are all present, gated against the crate's
  closed-form CVA/DVA oracle rows.
- **`celnet-replog`** (`RaftNode` — a deterministic replicated log over `std::net`, `election.rs`).
  Activation is a **resilience/HA cross-cut**, not a pricing or wire-contract change: it backs the
  durable book/journal replication path and needs a server-lifecycle owner (boot/propose/compact/
  shutdown wired into the book-store commit path and the node-lifecycle supervisor), which spans the
  server bootstrap and `celnet-journal` integration outside item D's scope. *Activation plan:* land
  it with the HA/replication workstream — boot a `RaftNode` per server instance, route
  `RatesPositionStore` / book commits through `RaftNode::propose`, and gate with the crate's
  partition/election proptests under a multi-node harness.

## WS codec from proto descriptor (item G) — `ws-codec-from-proto`

**Goal (ADR-0009).** Drive the WebSocket JSON codec (`celnet-server::ws`) from the proto
descriptor rather than hand-maintaining it, so it can no longer silently drift from
`celnet.proto`, while staying **wire byte-identical** across the cross-client conformance corpus.

**Landed (this lane, byte-identical, descriptor-derived).**
- **`celnet-proto/build.rs` now emits a generated `wire_contract` module** from the **same**
  `FileDescriptorSet` protox already produces for the message/service codegen (no second parse, no
  new toolchain — `prost-types` walks the descriptor). Surfaced as `celnet_proto::wire_contract`:
  - `STREAM_CONTROL_VERBS: &[&str]` — the `ClientStreamMessage` oneof arm names (the WS `type`
    discriminators the router forwards to the RFS session driver).
  - `WIRE_RPCS: &[WireRpc]` — every service method as `{service, method, request, response}`
    (the descriptor-side source of truth for unary-verb coverage).
  - `MESSAGES`/`ENUMS` — the full message (incl. nested) + enum name surface.
  Generation is deterministic (descriptor declaration order) ⇒ the emitted file is byte-identical
  on every build (determinism discipline).
- **`celnet-server::ws::mod::is_stream_control` is now sourced from `STREAM_CONTROL_VERBS`** (was a
  hand-maintained `matches!` literal). The router's stream-control classification — the routing in
  `dispatch` — is therefore generated from the schema: a new `ClientStreamMessage` oneof arm is
  classified automatically with no hand edit (the `authenticate`-omission class of bug is now
  structurally impossible). A no-regression test asserts the generated set is **byte-identical** to
  the frozen wire verb set; the lockstep test drives the generated set so the matching
  `decode_stream_control` arm is forced to keep up (a proto arm added without its decoder arm fails
  the gate).

**Status 2026-08-17 — SUBSTANTIALLY LANDED, not deferred.** The unary swap is complete for
**110 of 112 dispatch arms** (measured: 220 `generated_codec::` call sites against 14
`codec::` in `handle_unary`). Only `list_hedge_suggestions` and `execute_hedge_suggestion`
remain hand-coded, plus the framing helpers `codec::tagged` / `codec::error_frame` and the
streaming frames. Two residuals remain open: `generated_codec.rs` is **hand-maintained
despite its name** (there is no `build.rs` in `celnet-server`; only
`celnet-proto/build.rs` generates anything), and `codec.rs`'s `diff_support` oracle module
is `pub` rather than feature-gated, so it ships in the production binary.

The original deferral rationale is retained below because it still explains **why a naive
descriptor projection cannot be used**, and therefore why the curated override table exists: the WS JSON codec is a hand-curated *client*
contract that intentionally diverges from a naive descriptor projection in ways the descriptor alone
does not encode, so a mechanical regeneration would change bytes on the wire (a regression), not
merely the source of the codec:
1. **Verb naming differs per service.** `PricingService.PriceRates` → `"price_rates"` (method-name
   snake_case) but `FixAdminService.ListConnections` → `"list_fix_connections"` (request-message
   snake_case). No single descriptor rule reproduces both; a per-service mapping is required.
2. **FX/legacy projections.** `Underlying` encodes to the legacy `{base,quote}` `pair` key (not the
   `underlying` oneof); `MarketContext` encodes to `r_dom`/`r_for` accessors (not the generalized
   `{discount_rate,carry}` fields); `Greeks` emits the flat `rho_dom`/`rho_for` projection beside
   the `rate_sensitivities` oneof. These keep the unchanged GUI/Excel FX contract.
3. **Non-uniform key casing.** A few keys are camelCase (`brokenDate`), not the proto snake_case.
4. **Bespoke oneof tagging** (`tagged()`, `strike`/`delta`, the `underlying` arms).

**Activation plan.** Extend `wire_contract` to emit per-message **field tables** (proto name → JSON
key, type, label, oneof group) plus a curated **override table** capturing (1)–(4); generate the
mechanical message codecs from the descriptor and keep the override arms as hand functions; gate with
a **differential byte-identity harness** (`generated_encode(m) == hand_encode(m)` byte-for-byte over
the cross-client corpus) before swapping `handle_unary` onto the generated path. The
`ws-codec-from-proto` acceptance roll-up stays **non-active** (forward) until that swap lands and the
differential harness is green; the descriptor-derived routing + manifest above are the first
byte-identical increment of it.

---

**Cross-check (re-verified 2026-06-10 against the working tree).** Every arm/enum/field in
the catalogue registry and the asset-class/linear/RFQ registry above is present in
`crates/celnet-proto/proto/celnet.proto` at the cited line, dispatched in
`crates/celnet-server/src/pricer.rs`, reachable per `docs/CLIENT-PARITY-MATRIX.md`, and
backed by the cited gate. The `Instrument.product` oneof is fully registered: arms 7–21 and
23–28 built + gated, arms 30/31 **landing** (in-flight wave); the `Instrument` scalar selectors
`pricing_model = 22` and `settlement_style = 29` are registered, as are the `Underlying`
asset-class arms, the carry-seam messages, the settlement enums and the `QuoteService`
RFQ-to-many symbols. No proto symbol is present-but-unregistered.

**Build status.** All W1–W6 catalogue extensions are **DONE and gated green** end-to-end (server
pricer + SDK `InstrumentSpec`/vocab builders + CLI `exotic …` + Excel + GUI ticket), validated by
the cited `celnet-parity` rows, the server suite, the SDK e2e suites, the GUI/Excel build+test
suites, and full `just check` ("All gates passed."). The W2–W5 linear / cross-asset / RFQ-to-many
extensions are likewise gated (`linear.rs` / `crossasset.rs` / `multi_dealer.rs` + the client
suites). The Excel surface all of this rides is the **polymorphic** `CELNET.INSTRUMENT` token +
`PRICE`/`GREEKS`/`RFQ`/`SUBSCRIBE` verbs — the 18 per-product worksheet functions were retired at
proven byte-identical wire parity (commit `5800ec4`; see `docs/EXCEL-INTEGRATION.md` §3.3 and
`docs/CLIENT-PARITY-MATRIX.md`). Arms 30/31 are **landing** (in-flight new-payoff-shapes wave;
committed at wip checkpoint `c8fb02f`, full-workspace gates pending — not yet claimable as done).
One unversioned contract; no `schema_version`.
