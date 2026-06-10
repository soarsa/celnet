# Client Parity Matrix

> **The api-first parity proof.** Every product and capability lives in the **one
> unversioned `celnet-proto` contract** (`crates/celnet-proto/proto/celnet.proto`;
> guardrail #9 — no `schema_version`, no N/N-1 negotiation). Each client surface —
> the **server**, the Rust **SDK** (`celnet-client`), the **CLI** (`celnet-cli`),
> the **Excel** add-in (`CELNET.*`), and the **GUI** — consumes that same contract
> and evolves in lockstep ([[api-first-client-parity]]). This matrix records, for
> every capability, that it is reachable from each surface.
>
> **Legend:** ✅ reachable from this surface · n/a structurally not applicable to
> this surface (with the honest reason) · **gap** a real, named parity gap on this
> surface (tracked in `docs/WORLD-CLASS-BACKLOG.md` — never silently claimed) ·
> **landing** implemented in an in-flight wave with gates pending (not yet
> claimable as ✅). No cell is a stub: a ✅ means the surface actually exercises
> the wire contract, not a placeholder.
>
> **Single contract.** A product is added by **appending** to the `Instrument`
> oneof / the relevant service in `celnet.proto` (never renumbering, never a
> version field); the clients then surface it. The wire types under `Instrument`
> (`crates/celnet-proto/proto/celnet.proto`, `message Instrument` / `oneof product`) are the
> authoritative product set.

## Surfaces

| Surface | Crate / path | Transport |
|---|---|---|
| **Server** | `celnet-server` | gRPC (tonic) + WebSocket mirror of the same contract |
| **SDK** | `celnet-client` | gRPC channel (`tonic`), async |
| **CLI** | `celnet-cli` | calls the SDK / prices locally via the same engine crates |
| **Excel** | `excel/` (Office.js custom functions `CELNET.*`) | the WS/gRPC-web transport in `excel/src/transport` |
| **GUI** | `gui/` (React) | WebSocket by default (`gui/src/data`), the same wire codec |

## Pricing & quoting

| Capability | Wire contract | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| Price + full Greek set | `PricingService.Price` | ✅ | ✅ `price` | ✅ `price` | ✅ `CELNET.PRICE` / `CELNET.GREEKS` | ✅ PriceTile / GreeksStrip |
| RFQ → two-way quote | `QuoteService.RequestQuote` | ✅ | ✅ `request_quote` | ✅ `rfq` / `stream` (RFS) | ✅ `CELNET.RFQ` | ✅ TwoWayQuote |
| RFQ-to-many → ranked multi-dealer panel | `QuoteService.RequestMultiDealerQuote` (`MultiDealerQuote` / `DealerQuote`; accept by `QuoteAccept.lp_id`) | ✅ (`celnet-rfq` engine + loopback LP panel; `tests/multi_dealer.rs`) | ✅ `request_multi_dealer_quote` → `MultiDealerRfq` | ✅ `rfq` | ✅ `CELNET.RFQ` ranked panel spill | ✅ DealerPanel |
| Accept / reject quote (click-to-trade) | `QuoteService.{AcceptQuote,RejectQuote}` | ✅ | ✅ `Rfq::{accept,reject}` / `MultiDealerRfq::accept_dealer` | ✅ `stream` | ✅ task-pane Trade (`acceptQuote` by `(quote_id, lp_id)`; a cell never trades — `CELNET.RFQ` spills the `quote_id` an accept echoes) | ✅ LastLookRing click-to-trade |
| Streaming prices (RFS, multiplex session) | `StreamService.StreamSession` | ✅ | ✅ `open_session` | ✅ `stream` | ✅ `CELNET.SUBSCRIBE` | ✅ StreamWorkspace |
| Market series / trend feed | `StreamService.StreamSession` (`MarketSeries*`) | ✅ | ✅ `subscribe_series` | n/a (one-shot CLI; the stream subcommand covers live ticks) | ✅ `CELNET.SERIES` | ✅ Sparkline / `useTrendSeries` |

## Surface (smile / mark / scenario)

| Capability | Wire contract | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| Get calibrated smile | `SurfaceService.GetSmile` | ✅ | ✅ `get_smile` | ✅ `surface` | ✅ `CELNET.SURFACE` | ✅ SurfaceWorkspace / SurfaceMesh |
| Mark surface (selectable model: VV / SABR / SVI / SSVI / eSSVI) | `SurfaceService.MarkSurface` (+ `SmileModel`) | ✅ | ✅ `mark_surface` / `mark_surface_with` | ✅ `surface` | ✅ `CELNET.MARK` / `CELNET.MARKSURFACE` | ✅ SurfaceWorkspace mark + model chips |
| Scenario grid (spot×vol, theta-roll, bucketed vega, cross-gamma) | `SurfaceService.Scenario` | ✅ | ✅ `scenario` / `scenario_with_model` / `scenario_with_risk[_and_model]` | ✅ `surface` | **gap** — no worksheet scenario function (the WS codec decodes `scenario` frames, `excel/src/contract/wsCodec.ts` `scenarioResultFromWire`, but no `CELNET.*` function or transport method issues one; server-side risk grids reach Excel via `CELNET.RISK`) | ✅ CubeWorkspace pivot/heatmap |

## Exotic & structured products (the catalogue)

All exotics flow through the unified `Instrument` oneof; the same instrument
message is priced by `PricingService.Price` and (where applicable) quoted/booked.
Each row is **Built** with a gated `celnet-parity` row (cited in
`docs/ANALYTICS-SPEC.md` §4).

> **Excel column — the current surface.** The 18 per-product `CELNET.*` worksheet
> functions were **retired at proven byte-identical wire parity** (commit `5800ec4`;
> proof: `excel/test/instrumentPolymorphic.test.ts`). The Excel surface is now ONE
> polymorphic set: `=CELNET.INSTRUMENT(underlier, family, terms, [tenor], [notional])`
> produces an opaque instrument token that `CELNET.PRICE` / `CELNET.GREEKS` /
> `CELNET.RFQ` / `CELNET.SUBSCRIBE` all accept, for any family on any asset class
> (`docs/EXCEL-INTEGRATION.md` §3.3). An Excel ✅ below therefore names the
> `CELNET.INSTRUMENT` **family token**, not a per-product function.

| Product | Engine module / parity row | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| Multi-leg strategy (risk reversal / straddle / strangle / seagull) | `Strategy strategy = 8` / `strategy.rs` | ✅ | ✅ strategy vocab builders | n/a (no multi-leg argv grammar — legs are built via the SDK/GUI) | **gap** — `CELNET.INSTRUMENT` has no STRATEGY family yet (open backlog `excel-instrument-strategy-family`) | ✅ TicketWorkspace leg-ladder (riskReversal/straddle/strangle/seagull specs) |
| European digital | `celnet-exotics::digital` / `exotics.rs` | ✅ | ✅ `price(Instrument)` | ✅ `exotic digital` | ✅ INSTRUMENT `"DIGITAL"` | ✅ TicketWorkspace |
| One-touch / no-touch / DNT / double-touch | `celnet-exotics::touch` / `exotics.rs` | ✅ | ✅ | ✅ `exotic one-touch` / `dnt` | ✅ INSTRUMENT `"TOUCH"` | ✅ TicketWorkspace |
| Single / double barrier (KO/KI, all 8 + double) | `celnet-exotics::barrier` / `exotics.rs` | ✅ | ✅ | ✅ `exotic barrier` | ✅ INSTRUMENT `"BARRIER"` | ✅ TicketWorkspace (single/double-barrier instruments) |
| Window / partial barrier | `celnet-exotics::lsv::price_window_barrier_pde` / `lsv.rs` | ✅ | ✅ | ✅ `exotic window-barrier` | ✅ INSTRUMENT `"WINDOWBARRIER"` | ✅ TicketWorkspace (windowBarrierInstrument) |
| Arithmetic / geometric Asian | `celnet-exotics::asian` / `asian.rs` | ✅ | ✅ | ✅ `exotic asian` | ✅ INSTRUMENT `"ASIAN"` | ✅ TicketWorkspace (asianInstrument) |
| Forward-start vanilla | `celnet-exotics::forward_start` / `forward_start.rs` | ✅ | ✅ | ✅ `exotic forward-start` | ✅ INSTRUMENT `"FORWARDSTART"` | ✅ TicketWorkspace (forwardStartInstrument) |
| Cliquet (plain / capped) | `celnet-exotics::forward_start` (cliquet legs) / `forward_start.rs` | ✅ | ✅ | ✅ `exotic cliquet` | ✅ INSTRUMENT `"CLIQUET"` | ✅ TicketWorkspace (cliquetInstrument) |
| Quanto vanilla / digital | `celnet-exotics::quanto` / `quanto.rs` | ✅ | ✅ | ✅ `exotic quanto` | ✅ INSTRUMENT `"QUANTO"` | ✅ TicketWorkspace (quantoInstrument) |
| Lookback (floating / fixed) | `celnet-exotics::lookback` / `lookback.rs` | ✅ | ✅ | ✅ `exotic lookback` | ✅ INSTRUMENT `"LOOKBACK"` | ✅ TicketWorkspace (lookbackInstrument) |
| TARF | `celnet-exotics::tarf` / `tarf.rs` | ✅ | ✅ | ✅ `exotic tarf` | ✅ INSTRUMENT `"TARF"` | ✅ TicketWorkspace (tarfInstrument) |
| Accumulator / decumulator | `celnet-exotics::accumulator` / `accumulator.rs` | ✅ | ✅ | ✅ `exotic accumulator` | ✅ INSTRUMENT `"ACCUMULATOR"` | ✅ TicketWorkspace (accumulatorInstrument) |
| Variance swap | `celnet-exotics::var_swap` / `var_vol_swap.rs` | ✅ | ✅ | ✅ `exotic var-swap` | ✅ INSTRUMENT `"VARSWAP"` | ✅ TicketWorkspace (varianceSwapInstrument) |
| Volatility swap | `celnet-exotics::vol_swap` / `var_vol_swap.rs` | ✅ | ✅ | ✅ `exotic vol-swap` | ✅ INSTRUMENT `"VOLSWAP"` | ✅ TicketWorkspace (volatilitySwapInstrument) |
| American / Bermudan (free-boundary FD or LSM-MC) | `celnet-exotics::american` / `american.rs` | ✅ | ✅ | ✅ `exotic american` | ✅ INSTRUMENT `"AMERICAN"` | ✅ TicketWorkspace (americanInstrument) |
| Correlated basket / best-of / worst-of (multi-asset MC) | `celnet-exotics::price_basket` / `multiasset.rs` | ✅ | ✅ | ✅ `basket` | ✅ INSTRUMENT `"BASKET"` | ✅ TicketWorkspace (basketInstrument) |
| LSV booking model (calibration + ADI PDE) | `celnet-exotics::lsv` / `lsv.rs` | ✅ (booking/Greeks engine) | ✅ (via the priced instrument / `--model lsv`) | ✅ `exotic … --model lsv` | n/a (Excel exposes priced products, not raw calibration) | n/a (GUI prices via the model the server selects) |

> The SDK exposes the catalogue through the single `price(Instrument)` entry point
> (and the quote/stream paths) rather than one method per product, so each ✅ in
> the SDK column is the same method carrying a different `Instrument` variant —
> exactly the "one clean contract" shape.

## Linear products & cross-asset underlyings (W2–W5)

The linear (non-option) book and the cross-asset universe ride the same one contract:
the linear payoffs are `Instrument.product` arms 26–28; a cross-asset option is the
**same** product arm priced over a non-FX `Underlying` arm (metal / equity / commodity /
digital-asset) + the generalized carry seam — there are no per-class product arms
(registry: `docs/INTERFACES.md` §"Asset-class universe, linear products, RFQ-to-many +
settlement mechanics").

| Product / underlying | Wire contract | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| Outright forward (deliverable) | `FxForward fx_forward = 26` / `linear.rs` | ✅ (`celnet-linear`) | ✅ vocab + `price_linear` example | ✅ `forward` | ✅ INSTRUMENT `"FORWARD"` | ✅ TicketWorkspace (forward spec) |
| FX swap (near + far leg) | `FxSwap fx_swap = 27` / `linear.rs` | ✅ | ✅ | ✅ `swap` | ✅ INSTRUMENT `"SWAP"` | ✅ TicketWorkspace (swap spec) |
| NDF (fixing identity + convertible-ccy settlement) | `Ndf ndf = 28` / `linear.rs` | ✅ (deliverable-pair NDF rejected) | ✅ | ✅ `ndf` | ✅ INSTRUMENT `"NDF"` | ✅ TicketWorkspace (ndf spec) |
| Metal underlying (XAU/XAG/XPT/XPD vs fiat) | `Underlying.metal = 3` / `pair_universe.rs` | ✅ | ✅ | ✅ | ✅ underlier `"XAUUSD"` (metal-vs-FIAT only) | ✅ TicketWorkspace (crossAsset spec) |
| Equity option | `Underlying.equity = 4` (+ existing product arms) / `crossasset.rs` | ✅ (`celnet-equity-vanilla`) | ✅ + `price_cross_asset` example | ✅ `price --asset equity` (three-way gate) | ✅ underlier `"AAPL@XNAS:USD"` | ✅ TicketWorkspace (crossAsset spec) |
| Commodity option | `Underlying.commodity = 5` / `crossasset.rs` | ✅ (`celnet-commodity-vanilla`) | ✅ | ✅ `price --asset commodity` | ✅ underlier `"BRENT@:USD"` | ✅ TicketWorkspace (crossAsset spec) |
| Crypto option (linear + inverse coin-margined) | `Underlying.digital_asset = 6` + `settlement_style = 29` / `crossasset.rs` | ✅ (`celnet-crypto-vanilla`) | ✅ | ✅ `price --asset crypto` | ✅ underlier `"BTC/USD[:inverse]"` | ✅ TicketWorkspace (crossAsset spec) |

> **Test-coverage caveat** (open backlog `cross-asset-client-priced-vectors-ws-e2e`): the
> GUI offline-conformance corpus and the Excel WS e2e corpus do not yet **price** the
> linear/cross-asset golden vectors client-side (`gui/test/conformance.test.ts`
> `FAMILIES_NOT_EXPOSED_BY_GUI`; `excel/e2e/corpus.ts` exclusions). Reachability above is
> the wire/ticket surface, gated by `celnet-parity/tests/{linear,crossasset}.rs`,
> `excel/test/{linearProducts,crossAssetProducts}.test.ts` and the CLI three-way gate
> (`celnet-cli/tests/conformance.rs`: CLI == server == independent oracle).

## Landing — in-flight payoff arms (wip checkpoint `c8fb02f`; gates pending)

Both rows are implemented across all five surfaces in the in-flight new-payoff-shapes
wave, with `celnet-parity/tests/{perpetual,listed_future}.rs` + golden vectors committed —
but the full-workspace gates are pending, so under this matrix's gate semantics they are
**landing**, not yet ✅.

| Product | Wire contract | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| Perpetual American option (no expiry) | `PerpetualOption perpetual_option = 30` / `perpetual.rs` | landing (`pricer.rs`) | landing (`InstrumentSpec::perpetual[_on]`) | landing (`perpetual`) | landing (INSTRUMENT `"PERPETUAL"`, tenorless) | landing (perpetual spec, `noExpiry`) |
| Listed-future option (any asset class; equity-/futures-style margining) | `ListedFutureOption listed_future_option = 31` / `listed_future.rs` | landing (`pricer.rs`) | landing | landing (`future-option`) | landing (INSTRUMENT `"FUTUREOPTION"`) | landing (listedFutureOption spec) |

## Risk & portfolio

| Capability | Wire contract | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| List positions | `RiskService.ListPositions` | ✅ | ✅ `list_positions` | ✅ `risk positions` | ✅ `CELNET.POSITIONS` | ✅ BookWorkspace |
| Aggregate risk (hierarchical roll-up) | `RiskService.AggregateRisk` | ✅ | ✅ `aggregate_risk` | ✅ `risk aggregate` | ✅ `CELNET.RISK` | ✅ RiskWorkspace |
| Drill risk (scope drill-down) | `RiskService.DrillRisk` | ✅ | ✅ `drill_risk` | ✅ `risk drill` | ✅ `CELNET.RISK` (scoped) | ✅ Book→Risk drill |
| Limit status (RAG) | `RiskService.LimitStatus` | ✅ | ✅ `limit_status` | ✅ `risk limits` | ✅ `CELNET.LIMITS` | ✅ RiskWorkspace limits panel |

## Conventions & calendar

| Capability | Source | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| Resolve (pair, tenor) conventions / spot & delivery dates | `celnet-conventions` + `celnet-calendar` (carried on every priced instrument) | ✅ (applied per request) | ✅ (`Conventions` on the RFQ / price call) | ✅ `convention` | ✅ implicit per function | ✅ ConventionChip / DatePicker (incl. ON/TN/SN/IMM + broken dates) |

## How parity is kept (the rule)

1. A capability is added **only** by appending to `celnet.proto` (one contract, no
   versioning).
2. The server implements it; `celnet-client` exposes it; the CLI, Excel functions,
   and GUI all consume the SDK / wire contract.
3. The change is proven by exercising the **same** capability across surfaces
   (e.g. the scale harness drives price + Greeks + surface + RFS + risk through the
   edge via `celnet-client`; the Excel e2e suite and the GUI vitest/Playwright
   suites exercise the same wire codec).
4. This matrix is updated in the same change, so no surface silently lags.

## SDK onboarding — runnable examples

The Rust SDK ships runnable quickstart examples under `crates/celnet-client/examples/`,
the canonical `cargo run` onboarding affordance. Each connects to a running edge
(`CELNET_GRPC_ADDR`, default `http://127.0.0.1:50551`), drives one trader workflow,
prints real priced output, and exits non-zero on a degenerate/empty response:

| Example | Workflow | Run |
|---|---|---|
| `quote_and_trade` | RFQ: request a two-way quote → BUY (lift the offer) → book | `cargo run -p celnet-client --example quote_and_trade` |
| `multi_dealer_trade` | RFQ-to-many: ranked dealer panel → book the best LP by `lp_id` | `cargo run -p celnet-client --example multi_dealer_trade` |
| `stream_blotter` | one multiplexed session → 3 streamed vanilla lines (snapshot + live ticks) | `cargo run -p celnet-client --example stream_blotter` |
| `price_exotic` | price an Asian (closed-form) and an American (FD) via the vocab builders | `cargo run -p celnet-client --example price_exotic` |
| `price_linear` | price an outright forward / FX swap / NDF off the linear book arms | `cargo run -p celnet-client --example price_linear` |
| `price_cross_asset` | price equity / commodity / crypto vanillas over the cross-asset `Underlying` | `cargo run -p celnet-client --example price_cross_asset` |

Boot the edge first with `cargo run -p celnet-server --example demo_edge`. The same
SDK code paths are gated in-process by `crates/celnet-client/tests/examples_smoke.rs`,
which boots a real edge on an ephemeral port and asserts each workflow yields a
non-empty priced result (so the examples cannot silently rot).

**Honest boundary.** "Reachable from each surface" means the capability is wired to
the one contract on that surface. Surface-specific UX depth differs by design (e.g.
the GUI prices exotics through `TicketWorkspace` and selects the server-chosen model
rather than driving raw LSV calibration; Excel exposes priced products, not the
calibration internals) — those `n/a` cells name the structural reason rather than
overclaiming a control that does not exist.
