# Celnet — Frozen Interface Registry

The contracts that parallel work-streams depend on. There is exactly **one clean, current
contract** — no versioned APIs, no back-compat shims (we have no external users). Changing
anything here means editing the interface crate **and every dependent in the same change**,
updating this file, and announcing it in the `CLAUDE.md` ledger. Within a parallel-build
window the interface crates are treated as **stable** so streams don't churn; a deliberate
interface change coordinates all affected crates at once (see `docs/ROADMAP.md` §3).

## The 28-crate workspace

The implemented flat workspace is **28 crates** (`ls crates`):

```
celnet-types  celnet-core  celnet-conventions  celnet-calendar  celnet-vanilla
celnet-surface  celnet-exotics  celnet-gpu  celnet-engine  celnet-integration
celnet-server  celnet-cli  celnet-client  celnet-proto  celnet-plugin-api
celnet-plugin-host  celnet-observability  celnet-golden  celnet-testkit  celnet-bench
celnet-router  celnet-fix  celnet-journal  celnet-parity  celnet-risk-normalize
celnet-risk-cube  celnet-limits  celnet-entitlements
```

Several domains the early design split across many crates were **consolidated**:
`celnet-surface` holds VV/SABR/SVI/SSVI + arbitrage gates + term structure;
`celnet-exotics` holds LSV + PDE + MC + the shared numerics; `celnet-golden` is the
QuantLib oracle/generator; `celnet-observability` owns telemetry rings/histograms;
`celnet-gpu` is the wgpu backend + f64 CPU reconciliation. `celnet-plugin-host` is **built**:
the tiered host (Tier-0 native registry + Tier-2 **wasmi** fuel-metered sandbox + replay
harness) behind the frozen `celnet-plugin-api` contract — wasmtime was rejected for open
RustSec advisories (see `docs/PLUGIN-HOST-ALT.md`).

## Dependency direction (must never invert)

```
celnet-types  ←  celnet-core  ←  { celnet-conventions, celnet-calendar, celnet-vanilla,
                                   celnet-surface, celnet-exotics, celnet-gpu }  ←  celnet-engine
                                   ←  { celnet-server, celnet-cli }
celnet-proto      →  depends only on celnet-types
celnet-plugin-api →  depends only on celnet-types (+ celnet-core traits)
celnet-client     →  depends on celnet-proto (typed SDK over tonic)
celnet-observability →  telemetry seam; celnet-engine stays free of its deps
celnet-integration   →  Celer estate + vendor MD adapters, over celnet-surface/-types
celnet-risk-normalize →  pure leaf transform over celnet-vanilla/-core/-types (no IO);
                         the convention/numeraire boundary the risk cube sits on
celnet-risk-cube      →  single-node OLAP cube over celnet-risk-normalize (+ -vanilla
                         for bump-and-revalue, -core, -types); no IO/market-data
celnet-entitlements   →  pure pre-aggregation pruning predicate over celnet-risk-cube
                         (FactKey/Hierarchy/DimensionId) + -types; no IO; sits beside the
                         cube, upstream of its group-by
celnet-limits         →  pure limit framework over celnet-risk-cube (NodeAggregate /
                         dimension keys / Hierarchy) + celnet-risk-normalize (CanonicalLeaf)
                         + -types; no IO; sits above the cube, downstream of its group-by
celnet-golden, celnet-testkit, celnet-bench  →  test/validation/bench only
```

## Status

| Crate | Version | Frozen? | Surface |
|-------|---------|---------|---------|
| `celnet-types` | 0.0.0 | **freeze-candidate** | `OptionType`, `Ccy`, `CcyPair`, `Tenor` (now `Overnight`/`TomNext`/`SpotNext`/`Weeks`/`Months`/`Years`/`Imm(u8)`/`BrokenDate(BrokenDate)`), `BrokenDate{year:i32,month:u8,day:u8}`, `SmileModel` (`MarketHedge`/`StochasticVol`/`Parametric`/`ParametricSurface`, `Default=MarketHedge`); newtypes `Vol`/`Strike`/`Rate`/`Delta`/`Df`/`Time`; convention enums `DeltaConvention`/`AtmConvention`/`PremiumStyle`/`Cut`/`DayCount`/`Settlement`; DTOs `VanillaInputs`, `Greeks`. POD/`Copy`, `serde`. (No `time` dep — a broken date is the POD triple.) |
| `celnet-core` | 0.0.0 | **freeze-candidate** | `math` (`norm_cdf`, `norm_pdf`, `exp`/`ln`/`sqrt` via `libm`); `is_close` + `assert_close!` (ULP/rel/abs); trait `Smile` + `FlatSmile`. Zero IO. |
| `celnet-proto` | 0.0.0 | **freeze-candidate** | single current wire contract (`prost 0.13` / `tonic 0.12`); `celnet.proto` services `PricingService`/`QuoteService`/`StreamService`/`SurfaceService`; `Instrument` oneof; **no** version field / negotiation. Phase-1 additions: `Tenor` short-end/IMM/`BrokenDate` units; `SmileModel` enum on `MarkSurfaceRequest`/`ScenarioRequest`; market-series feed (`MarketObservable`, `MarketSeriesSubscribe`/`Unsubscribe`/`Point`/`Snapshot`) multiplexed on `StreamSession`; attribution identity (`BookId`/`Owner`/`AttributionRecord`) on the quote/trade lifecycle. See §"Phase-1 contract extensions". |
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
  …PARAMETRIC=2, …PARAMETRIC_SURFACE=3 }`; `MarkSurfaceRequest.smile_model` (field 4) and
  `ScenarioRequest.smile_model` (field 7), both `optional` — absent ⇒ server default
  calibration. **Server status:** `MarkSurface` **honours** the selection — it routes to the
  matching `celnet-surface` calibrator (market-hedge VV baseline / fitted SABR / SVI / SSVI via
  `celnet_surface::build_model_smile`), deposits the model-tagged `CalibratedSmile` under the
  surface version (so a pinned RFQ/RFS re-prices against the exact marked model), and echoes the
  model used in the `Smile.arbitrage.note` (`model=<family>`) as provenance. The default reproduces
  the market-hedge baseline byte-for-byte. `Scenario` validates the selector but reprices each node
  off the supplied flat `base_market.vol`; with no broker skew a neutral smile collapses to the same
  flat slice, so the shock grid is model-invariant by construction (a model-dependent scenario marks
  a skewed surface first via `MarkSurface` + a pinned `surface_version`).

### Market-series feed (the TrendMode feed) — `celnet-proto`, multiplexed on `StreamSession`

- **`enum MarketObservable { …ATM_VOL=0, …SPOT=1, …RISK_REVERSAL=2, …BUTTERFLY=3, …FORWARD=4 }`**
  — labelled, unit-bearing observables (no abstract index).
- **`MarketSeriesSubscribe`** `{ SubscriptionId subscription; CcyPair pair; MarketObservable
  observable; optional Tenor tenor; optional double delta; uint64 throttle_nanos; uint32
  history_limit }` (tenor required for ATM_VOL/RR/BF/FORWARD; delta required for RR/BF).
  **`MarketSeriesUnsubscribe`** `{ SubscriptionId subscription }`.
- **`MarketSeriesPoint`** `{ SubscriptionId subscription; uint64 sequence; double value; int64
  epoch_nanos }`. **`MarketSeriesSnapshot`** `{ subscription; sequence; CcyPair pair;
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
