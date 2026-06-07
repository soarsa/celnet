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
> this surface (with the honest reason). No cell is a stub: a ✅ means the surface
> actually exercises the wire contract, not a placeholder.
>
> **Single contract.** A product is added by **appending** to the `Instrument`
> oneof / the relevant service in `celnet.proto` (never renumbering, never a
> version field); the clients then surface it. The wire types under `Instrument`
> (`crates/celnet-proto/proto/celnet.proto:859`) are the authoritative product set.

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
| RFQ → two-way quote | `QuoteService.RequestQuote` | ✅ | ✅ `request_quote` | ✅ `stream` (RFS) | ✅ `CELNET.RFQ` | ✅ TwoWayQuote |
| Accept / reject quote (click-to-trade) | `QuoteService.{AcceptQuote,RejectQuote}` | ✅ | ✅ `Rfq::{accept,reject}` | ✅ `stream` | ✅ `CELNET.RFQ` accept | ✅ LastLookRing click-to-trade |
| Streaming prices (RFS, multiplex session) | `StreamService.StreamSession` | ✅ | ✅ `open_session` | ✅ `stream` | ✅ `CELNET.SUBSCRIBE` | ✅ StreamWorkspace |
| Market series / trend feed | `StreamService.StreamSession` (`MarketSeries*`) | ✅ | ✅ `subscribe_series` | n/a (one-shot CLI; the stream subcommand covers live ticks) | ✅ `CELNET.SERIES` | ✅ Sparkline / `useTrendSeries` |

## Surface (smile / mark / scenario)

| Capability | Wire contract | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| Get calibrated smile | `SurfaceService.GetSmile` | ✅ | ✅ `get_smile` | ✅ `surface` | ✅ `CELNET.SURFACE` | ✅ SurfaceWorkspace / SurfaceMesh |
| Mark surface (selectable model: VV / SABR / SVI / SSVI / eSSVI) | `SurfaceService.MarkSurface` (+ `SmileModel`) | ✅ | ✅ `mark_surface` / `mark_surface_with` | ✅ `surface` | ✅ `CELNET.MARK` / `CELNET.MARKSURFACE` | ✅ SurfaceWorkspace mark + model chips |
| Scenario grid (spot×vol, theta-roll, bucketed vega, cross-gamma) | `SurfaceService.Scenario` | ✅ | ✅ `scenario` / `scenario_with_model` / `scenario_with_risk[_and_model]` | ✅ `surface` | ✅ `CELNET.SURFACE` scenario | ✅ CubeWorkspace pivot/heatmap |

## Exotic & structured products (the catalogue)

All exotics flow through the unified `Instrument` oneof; the same instrument
message is priced by `PricingService.Price` and (where applicable) quoted/booked.
Each row is **Built** with a gated `celnet-parity` row (cited in
`docs/ANALYTICS-SPEC.md` §4).

| Product | Engine module / parity row | Server | SDK | CLI | Excel | GUI |
|---|---|---|---|---|---|---|
| European digital | `celnet-exotics::digital` / `exotics.rs` | ✅ | ✅ `price(Instrument)` | ✅ `exotic digital` | ✅ `CELNET.DIGITAL` | ✅ TicketWorkspace |
| One-touch / no-touch / DNT / double-touch | `celnet-exotics::touch` / `exotics.rs` | ✅ | ✅ | ✅ `exotic one-touch` / `dnt` | ✅ `CELNET.TOUCH` | ✅ TicketWorkspace |
| Single / double barrier (KO/KI, all 8 + double) | `celnet-exotics::barrier` / `exotics.rs` | ✅ | ✅ | ✅ `exotic barrier` | ✅ `CELNET.BARRIER` | ✅ TicketWorkspace (single/double-barrier instruments) |
| Window / partial barrier | `celnet-exotics::lsv::price_window_barrier_pde` / `lsv.rs` | ✅ | ✅ | ✅ `exotic window-barrier` | ✅ `CELNET.WINDOWBARRIER` | ✅ TicketWorkspace (windowBarrierInstrument) |
| Arithmetic / geometric Asian | `celnet-exotics::asian` / `asian.rs` | ✅ | ✅ | ✅ `exotic asian` | ✅ `CELNET.ASIAN` | ✅ TicketWorkspace (asianInstrument) |
| Forward-start vanilla | `celnet-exotics::forward_start` / `forward_start.rs` | ✅ | ✅ | ✅ `exotic forward-start` | ✅ `CELNET.FORWARDSTART` | ✅ TicketWorkspace (forwardStartInstrument) |
| Cliquet (plain / capped) | `celnet-exotics::forward_start` (cliquet legs) / `forward_start.rs` | ✅ | ✅ | ✅ `exotic cliquet` | ✅ `CELNET.CLIQUET` | ✅ TicketWorkspace (cliquetInstrument) |
| Quanto vanilla / digital | `celnet-exotics::quanto` / `structured.rs` (row 16) | ✅ | ✅ | ✅ `exotic quanto` | ✅ `CELNET.QUANTO` | ✅ TicketWorkspace (quantoInstrument) |
| Lookback (floating / fixed) | `celnet-exotics::lookback` / `structured.rs` (row 17) | ✅ | ✅ | ✅ `exotic lookback` | ✅ `CELNET.LOOKBACK` | ✅ TicketWorkspace (lookbackInstrument) |
| TARF | `celnet-exotics::tarf` / `structured.rs` (row 18) | ✅ | ✅ | ✅ `exotic tarf` | ✅ `CELNET.TARF` | ✅ TicketWorkspace (tarfInstrument) |
| Accumulator / decumulator | `celnet-exotics::accumulator` / `structured.rs` (row 19) | ✅ | ✅ | ✅ `exotic accumulator` | ✅ `CELNET.ACCUMULATOR` | ✅ TicketWorkspace (accumulatorInstrument) |
| Variance swap | `celnet-exotics::var_swap` / `var_vol_swap.rs` | ✅ | ✅ | ✅ `exotic var-swap` | ✅ `CELNET.VARSWAP` | ✅ TicketWorkspace (varianceSwapInstrument) |
| Volatility swap | `celnet-exotics::vol_swap` / `var_vol_swap.rs` | ✅ | ✅ | ✅ `exotic vol-swap` | ✅ `CELNET.VOLSWAP` | ✅ TicketWorkspace (volatilitySwapInstrument) |
| LSV booking model (calibration + ADI PDE) | `celnet-exotics::lsv` / `lsv.rs` | ✅ (booking/Greeks engine) | ✅ (via the priced instrument / `--model lsv`) | ✅ `exotic … --model lsv` | n/a (Excel exposes priced products, not raw calibration) | n/a (GUI prices via the model the server selects) |

> The SDK exposes the catalogue through the single `price(Instrument)` entry point
> (and the quote/stream paths) rather than one method per product, so each ✅ in
> the SDK column is the same method carrying a different `Instrument` variant —
> exactly the "one clean contract" shape.

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

**Honest boundary.** "Reachable from each surface" means the capability is wired to
the one contract on that surface. Surface-specific UX depth differs by design (e.g.
the GUI prices exotics through `TicketWorkspace` and selects the server-chosen model
rather than driving raw LSV calibration; Excel exposes priced products, not the
calibration internals) — those `n/a` cells name the structural reason rather than
overclaiming a control that does not exist.
