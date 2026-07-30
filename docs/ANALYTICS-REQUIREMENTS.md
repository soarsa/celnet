# Analytics — Requirements / Design

> Status: **requirements + design for review** (2026-07-30). No code yet. Grounds a NEW
> cross-cutting **Analytics** capability on already-captured Celnet data and the existing
> `celnet-observability` primitives (file:line cited below, current as of this doc). Follows
> the shipped **pricing-groups** / **aggregated-book** patterns (registry/store + proto/WS
> RPC + a dedicated GUI workspace) as the template. External research is cited with URLs.

## 1. Concept & goals

Two analytics **pillars**, plus a new GUI **domain tab**:

- **A — Trading & pricing performance analytics (TCA-style).** Per-trade / per-desk /
  per-book / per-counterparty / per-instrument analytics on *what we priced and traded*:
  spread & margin capture, price improvement, fill quality / hit-ratio / win-rate,
  **mark-outs** (post-trade price drift), slippage vs arrival/mid, RFQ response times &
  win/loss, tiering / pricing-group effectiveness, PnL attribution, volumes. Built on
  Celnet's **already-captured** trade data — above all the **`PricingProvenance` waterfall**
  (raw→constructed→tiered→outbound + margin/skew + features) already stamped onto
  Quote→Execution→Deal.
- **B — Infrastructure / code-latency analytics.** *How long the code took*: `price_instrument`,
  the rules engine (`RiskRouter::route`), aggregation/consolidation, the tiering pipeline,
  RFQ/quote, and booking — as **p50/p99/p99.9** latency histograms plus distributed **span
  traces** (which stage of a request took how long). Built on the *already-built*
  `celnet-observability` `LatencyRecorder` (HdrHistogram + coordinated-omission correction).
- **C — GUI "Analytics" domain tab** next to Administration, permission-gated per the
  capability model, with charts/dashboards for both pillars.

### Design tenets (locked by guardrails)
- **Collection lives OFF the pinned zero-alloc hot core (guardrail 11).** The hot core only
  pushes POD samples through the existing lossy SPSC telemetry ring; all histogram/rollup/span
  work runs on the drain/edge side, exactly where `celnet-observability` already places it
  (`crates/celnet-observability/src/audit.rs:1-30`). No new alloc/lock/log on the hot path.
- **OSS-only, MIT/Apache-2.0/BSD runtime deps (guardrail 7).** Every *linked* dependency is
  MIT/Apache/BSD. AGPL (Grafana ≥ v8) is only ever an **arm's-length, user-operated** export
  target; CDDL (`inferno`) is a dev/CI tool only; **BestX is a reference model, never a dep.**
  Full verdict table in §6.
- **Vendor-neutral, purpose-named (guardrail 8).** New crates are **`celnet-telemetry`**
  (pillar B latency/traces) and **`celnet-analytics`** (pillar A TCA rollups) — named for
  purpose, no vendor/person/paper names in identifiers.
- **One current contract, no versioning (guardrail 9).**
- **Scale to IB-sized portfolios & HFT counterparties (guardrail 6).** Rollups are O(events)
  incremental, bounded memory (HdrHistogram is fixed-size); the query API paginates.

## 2. Part 1 — external research & reference solutions (cited)

This capability deliberately reproduces the *concepts and outputs* of best-in-class tooling
with first-party OSS math, rather than depending on any of it. Per-tool licensing verdict in
§6; the concepts we adopt:

### 2.1 Distributed tracing / code latency
- **OpenTelemetry (OTel).** A trace is a tree of **spans** (start/end, status, attributes,
  events) sharing a `traceId`/`parentSpanId`; **context propagation** stitches spans across
  boundaries; **OTLP** (protobuf over gRPC/HTTP) is the export wire format; **semantic
  conventions** standardise attribute names. Rust: `opentelemetry` / `opentelemetry_sdk` /
  `opentelemetry-otlp` (traces = *Beta*, metrics/logs *Stable*). All **Apache-2.0**.
  ([OTLP spec](https://opentelemetry.io/docs/specs/otlp/),
  [opentelemetry-rust](https://github.com/open-telemetry/opentelemetry-rust))
- **`tracing` + `tracing-opentelemetry` (Rust).** `tokio-rs/tracing` is the de-facto Rust
  span-instrumentation framework, backend-decoupled via a `Subscriber`; the bridge layer
  converts `tracing` spans → OTel spans. **Caveat for us:** `tracing` is *not* truly
  zero-cost even when a span is filtered out — there is a per-callsite runtime check, so the
  pinned hot core stays instrumented only at coarse boundaries (or compile-gated).
  ([tracing](https://github.com/tokio-rs/tracing),
  [fastrace overhead note](https://fast.github.io/blog/fastrace-a-modern-approach-to-distributed-tracing-in-rust/))
  Both **MIT**.
- **Zipkin / Jaeger.** Zipkin's v2 JSON span model (`POST /api/v2/spans`) is a widely-spoken
  interop format; **Jaeger v2 is built on the OTel Collector and is OTLP-native** end-to-end.
  Both **Apache-2.0**, and both are backends we would *operate*, not link.
  ([Jaeger v2 on OTel](https://www.cncf.io/blog/2024/11/12/jaeger-v2-released-opentelemetry-in-the-core/))
- **Tail-latency: HdrHistogram + coordinated omission.** HdrHistogram records across a huge
  dynamic range in fixed memory and reports **p50/p99/p99.9/p99.99** accurately (vs averaging
  the tail away). **Coordinated omission** (Gil Tene): a stalled system stops issuing
  requests during the stall, silently omitting the very samples that should show the stall,
  flattering p99 — corrected by recording with an *expected inter-arrival interval* which
  back-fills the missed window. **This is exactly what Celnet's `LatencyRecorder` already
  implements** (§4.1). `hdrhistogram` crate = **MIT OR Apache-2.0**.
  ([HdrHistogram_rust](https://github.com/HdrHistogram/HdrHistogram_rust),
  [On Coordinated Omission](https://www.scylladb.com/2021/04/22/on-coordinated-omission/))
- **Flame graphs.** Sampled-stack visualisation (x-axis = proportion of samples, width = CPU
  cost). In-product profiling via **`pprof-rs`** (Apache-2.0; signal-safe, pre-allocated,
  emits flamegraph SVG + Google pprof protobuf). **`inferno`** (the Rust flamegraph renderer)
  is **CDDL-1.0** → dev/CI only, never a linked runtime dep.
  ([Flame Graphs](https://www.brendangregg.com/flamegraphs.html),
  [pprof-rs](https://github.com/tikv/pprof-rs), [inferno](https://github.com/jonhoo/inferno))
- **What is genuinely low-overhead on a hot path.** *Head sampling* (decide at span creation,
  before doing work) is the right default for a latency-critical service. *Tail sampling*
  (keep slow/error traces) gives better signal but costs throughput (~14% peak) and must run
  in the **collector, off-box** — never in the hot path. **In-process HdrHistogram aggregation
  beats per-span export** for latency SLOs: aggregate percentiles locally, emit summaries
  periodically, and reserve span export for sampled/slow requests.
  ([The Benefit of Hindsight (tail sampling)](https://arxiv.org/pdf/2202.05769),
  [OTel sampling](https://uptrace.dev/opentelemetry/sampling))

### 2.2 Metrics + charting
- **Prometheus.** Pull/remote-write time-series; counter / gauge / histogram / summary, plus
  **native (exponential) histograms** — auto-scaled unbounded buckets ideal for latency
  distributions. **Apache-2.0**; Rust exporters (`metrics-exporter-prometheus` MIT/Apache,
  `rust-prometheus` Apache-2.0) are clean runtime deps.
  ([native histograms](https://prometheus.io/docs/specs/native_histograms/))
- **Grafana — the licensing pivot.** Grafana was **Apache-2.0 through v7**, then relicensed
  to **AGPLv3 from v8 (20 Apr 2021)** along with Loki and Tempo. **Implication for us:**
  *exporting our metrics TO a user-operated Grafana over the network is fine* — Grafana is a
  separate process; we do not link, bundle, modify, or redistribute it, so no AGPL obligation
  attaches to Celnet. **We must never fork / bundle / embed / modify Grafana inside the
  product** (that would trigger AGPL source-disclosure and is barred by guardrail 7 anyway).
  ([Grafana AGPL relicensing](https://grafana.com/blog/grafana-loki-tempo-relicensing-to-agplv3/),
  [Grafana licensing](https://grafana.com/licensing/))
- **Export vs build-native — our decision (§6.3).** **Export** ops/SRE observability
  (latency, throughput, error rates) to Prometheus + a *user-run* Grafana — standard tooling,
  zero AGPL entanglement. **Build native** in-GUI charts only for the *trader-facing* analytics
  that belong in the trading workflow (TCA, provenance waterfalls, RFQ win/loss), using the
  MIT/Apache React/TS chart libs **already in `gui/package.json`** (visx, echarts,
  lightweight-charts — §5).

### 2.3 Trading analytics / TCA
- **BestX — commercial REFERENCE MODEL only.** An FX/rates TCA product (State Street /
  GlobalLink since 2018). **Never a dependency.** Worth mirroring as a *spec*: the
  **Expected-Cost** benchmark ("what this trade *should* have cost", bps spread-to-mid,
  conditioned on size/time/liquidity), post-trade decomposition into **spread cost / market
  impact / signalling risk**, and **hit-ratio normalised by RFQ panel size** ("RFQ Par" — 30%
  on a 2-LP panel ≠ 30% on an 8-LP panel).
  ([State Street acquires BestX](https://www.thetradenews.com/state-street-acquires-fx-tca-startup-bestx/),
  [RFQ Par](https://thefullfx.com/bestx-rfq-par-introduces-a-new-way-to-look-at-hit-ratios/))
- **General TCA methodology (our first-party build).**
  - **Mark-out** — post-trade drift of mid vs execution price at a *curve of horizons*
    (+100ms/+1s/+5s/+30s/+1m/+5m). As a market-maker (which we are on the pricing/FI side),
    this grades flow toxicity: toxic flow goes unprofitable in ms–s, benign flow stays
    profitable longer. ([QuestDB markout](https://questdb.com/docs/cookbook/sql/finance/markout/))
  - **Slippage vs arrival/mid** — execution price − arrival/decision mid (timing + drift cost).
  - **Implementation shortfall** — total cost vs the decision-point "paper" price, decomposed
    into spread / impact / timing / opportunity cost (arrival slippage with a sign flip).
    ([MillTech TCA](https://milltech.com/resources/glossary/transaction-cost-analysis-tca))
  - **Effective spread / spread capture** — realised spread earned vs quoted; the
    market-maker's core P&L lens. ([LMAX TCA whitepaper](https://www.lmax.com/documents/LMAXExchange-FX-TCA-Transaction-Cost-Analysis-Whitepaper.pdf))
  - **Price reversion** — retrace after the trade; high reversion ⇒ our own impact/adverse
    selection.
  - **Benchmarks** — arrival, TWAP/VWAP, mid.
  - **Win-rate / hit-ratio + RFQ win/loss + response / hold / reject-time**, interpreted
    relative to **panel size** and last-look behaviour.
    ([last look](https://en.wikipedia.org/wiki/Last_look_(foreign_exchange)))
- **FIX-based trade analytics** reconstruct the lifecycle from `QuoteRequest`/`Quote`/
  `ExecutionReport` timestamps (`TransactTime`, `SendingTime`), `LastPx`/`AvgPx`, qty, side —
  which Celnet's own quote/execution records already carry (§3).
- **Open-source references (not deps).** **`tcapy`** (cuemacro, **Apache-2.0**) — OSS FX-spot
  TCA (slippage, impact, mid/arrival/TWAP/VWAP benchmarks) — a *methodology reference / porting
  source*. **Almgren–Chriss** optimal-execution / impact model — a *published academic method*
  we may implement first-party (guardrail 6: cite the method).
  ([tcapy](https://github.com/cuemacro/tcapy))

## 3. Part 2 — Celnet grounding: the data & seams we build ON (don't reinvent)

### 3.1 Observability primitives already built — `celnet-observability`
The pillar-B histogram machinery **already exists and is validated**; the gap is *wiring* it
around specific paths and *exposing* snapshots.

- **`LatencyRecorder`** — `crates/celnet-observability/src/latency.rs:1-354`: HdrHistogram
  wrapper with **coordinated-omission correction** (Gil Tene, cited in the module doc).
  `percentile_ns` (`:100`), `p50_ns` (`:106`), `p99_ns` (`:112`), `p999_ns`, `p9999_ns`,
  `max/min/mean_ns`, `snapshot() -> LatencySnapshot` (`:150`, a `Copy` POD).
  `REPORTED_PERCENTILES = [50, 99, 99.9, 99.99, 100]` (`:26`). **`LatencyByKind`** (`:200+`) —
  one recorder per `OpKind` in a fixed `OpKind::COUNT` array (no hashing on the drain path),
  `record_ns(kind, nanos)`, `snapshots()`.
- **`metrics_facade.rs`** — `crates/celnet-observability/src/metrics_facade.rs:1-238`: thin
  facade over the `metrics` crate; canonical keys (`:17-30`) `celnet.op.latency.ns` (hist),
  `celnet.ops.total`, `celnet.telemetry.drained/dropped.total`, `celnet.engine.inflight`
  (gauge), `celnet.quotes.streamed.total`, `celnet.audit.committed.total`. `record_op(kind,
  class, latency_ns)` (`:39`) increments the counter AND records the histogram. Vendor-neutral —
  install any `metrics::Recorder` (Prometheus/OTLP) behind it.
- **`audit.rs`** — `crates/celnet-observability/src/audit.rs:1-336`: the **bounded lossy vs
  unbounded lossless** offload split, doc'd explicitly — telemetry (`channel.rs`) is *lossy by
  design* (drop on full ring, never block the hot core); audit is *lossless*
  (`audit_channel()` `:229`, monotonic sequence, committed on a non-critical task). **This is
  the natural, never-drop TCA event stream.**
- **`logging.rs`** — `crates/celnet-observability/src/logging.rs:1-372`: `LogClass` taxonomy;
  `build_json_subscriber`/`init_json_subscriber` (`:142-174`) structured line-JSON via
  `tracing_subscriber::fmt().json()`; **`AuditRecord`/`AuditStage`** (`:200-224`)
  `quote_requested → quote_issued → quote_accepted/rejected → trade_booked/amended/cancelled`.
- **Server init** — `crates/celnet-server/src/main.rs:49-51`: `init_json_subscriber` installed
  first in `main()`, honouring `RUST_LOG` (deploy runs `RUST_LOG=info`).
- **`channel.rs` / `record.rs`** — the lossy SPSC ring the hot core pushes `HotSample`s into;
  `OpKind` / `ErrorClass` / `HotSample` POD types.

### 3.2 Seams to instrument (pillar B) — signatures & call sites
Each is a synchronous, non-hot-core function called from the async edge — i.e. exactly where
`record_op`/`LatencyByKind` already belong. Wrapping each call site with a `LatencyByKind`
recorder keyed by a new `OpKind` is **additive and zero-cost-when-unused** (guardrail 11).

| Seam | File:line | Signature (as found) |
|---|---|---|
| Pricer | `crates/celnet-server/src/pricer.rs` | `fn price_instrument(instrument, market: &WireMarketContext, conv) -> Result<Priced, PriceError>` — in-degree **65** |
| Risk routing | `crates/celnet-risk-routing/src/router.rs` | `RiskRouter::route(graph, ctx) -> Result<&str, RouteError>`; called from `PositionStore::book` (`store.rs:807`) |
| Aggregation | `crates/celnet-aggregation/src/consolidate.rs` + driver `AggregationHub` (`crates/celnet-server/src/services/aggregation.rs`) | `ConsolidatedBook::from_quotes`; provenance stamp `provenance_from_priced` (`aggregation.rs:666-695`) |
| Tiering | `crates/celnet-tiering/src/feature_pipeline.rs` | `FeaturePipeline::run(raw: TwoWay, ctx) -> PricedResult` (`PricedResult` at `:32-49`) |
| ESP stream | `crates/celnet-server/src/services/stream.rs` | `StreamEdge` (calls `price_instrument` directly, CALLS-edge confirmed) |
| RFQ/quote | `crates/celnet-server/src/services/quote.rs` | `QuoteEdge::request_quote` (`:897`), `accept_quote` (`:1443`); desk twin `RfqDeskEdge::accept_desk_quote` (`services/desk/mod.rs:784`) |
| Booking | `crates/celnet-server/src/services/risk/store.rs:757-890` | `PositionStore::book(booked, key, attribution) -> Result<PreTradeDecision, Status>` — pre-trade gate → route → commit |

No `LatencyRecorder` is *currently wired* on these specific paths — they are the instrumentation
targets, not yet instrumented.

### 3.3 Trade/pricing data already captured (pillar A) — the goldmine
- **`Execution`** — `crates/celnet-proto/proto/celnet.proto:1731-1753`: `execution_id`,
  `quote_id`, `side`, `traded_premium`, `instrument`, `epoch_nanos`, `attribution`, and
  **`pricing_provenance`** (`:1750`, "copied verbatim from the accepted Quote").
- **`Quote`** — `celnet.proto:1579-1619`: `quote_id`, `price: TwoWayPrice`, `greeks`,
  `resolved_strike`, **`epoch_nanos`** (publication) + **`valid_until_nanos`** (last-look
  deadline) — the two timestamps for RFQ response-time / win-loss, `attribution`,
  `price_std_error`, **`pricing_provenance`** (`:1616`).
- **`BookedPosition`** — `crates/celnet-server/src/services/risk/store.rs:110-127`.
- **`QuoteRecord`** (server-internal) — `crates/celnet-server/src/services/quote.rs:266-301`:
  wraps `Quote` + `execution` + **`dealers: Vec<DealerQuote>`** (multi-dealer panel rows) +
  **`booked_lp_id`** + `requester` + `pre_trade` — the full RFQ panel plus which line won.
- **`PricingProvenance` — the pillar-A dataset** — `celnet.proto:4426-4459` (doc'd `:4419` for
  analytics: "realized markout = executed price vs `raw_mid`, decomposable per feature/group/
  mode"). Fields: `pricing_group_id`, `mode (ESP|RFQ)`, `raw_bid/mid/offer` (consolidated LP
  composite, pre-feature), `constructed_bid/offer` (after mid-shift), `tiered_bid/offer` (after
  tiering margin), `outbound_bid/offer` (after closing guardrail — what was sent),
  `applied_margin`, `applied_skew`, `features: repeated FeatureKind`. Rust mirror **`PricedResult`**
  (`feature_pipeline.rs:32-49`). Stamped: computed `provenance_from_priced`
  (`aggregation.rs:666`), copied onto booking (`quote.rs:2276` proves the invariant), on the
  wire via `pricing_provenance_to_json` (`ws/codec.rs:1175`).
- **Blotter / event-log storage:** **`DealStore`** —
  `crates/celnet-server/src/services/desk/store.rs:116-120` (`RwLock<Vec<Deal>>` + monotonic
  `deal-{n}`, newest-first); `DeskRequestStore` (RFQ/IOI requests); `NotificationBroker`
  (`services/desk/notify.rs`, a push event source for streaming analytics updates); and the
  lossless **audit log** (§3.1) as the canonical TCA event stream.

### 3.4 Permission model
- **`Action` × `AssetClass`** — `crates/celnet-entitlements/src/capability.rs:1-361`.
  `Action` (`:47-72`): `View, Price, QuoteRespond, RfqRespond, IoiRespond, Stream, Execute,
  Book, Simulate, Administer` (+ `Action::ALL` `:76`, `label`/`from_label` `:92`).
  `AssetClass` (`:119`): `FxOptions, FixedIncome`. `Capability{action, asset}` (`:151`).
  **`CapabilitySet`** (`:170-249`) — `grant_all` + `grants`/`denies` BTreeSets; deny-by-default,
  deny-wins; `.allows(cap)` is the decision predicate (`:226`).
- **`AuthEdge`** — `crates/celnet-server/src/services/auth.rs`: `require_admin(token) ->
  Principal`; `require_capability(token, cap) -> ...` (else `Status::permission_denied`).
  Existing gated-RPC pattern (mirrored from `accept_quote`): resolve caller → `authorize_caller`
  → `Capability::new(Action::Execute, AssetClass::FxOptions)`.
- **Trader-visible precedent:** Tiering is gated on `quote_respond·fixed_income` (ordinary
  traders hold it, NOT admin) — `gui/src/app/Shell.tsx:100-102`; contrast the
  `ADMIN_ONLY_WORKSPACES` set (`connections/admin/permissions/pricinggroups/refdata`).

### 3.5 GUI tab pattern & charting
- **Registry** — `gui/src/lib/commands.ts`: `WorkspaceId` union (`:43-62`); **`RAIL`** rows
  (`:106-148`) `{ id, glyph, label, assets }`; **`ADMIN_ONLY_WORKSPACES`** set (`:198-204`);
  gating predicates `workspaceAssets`/`workspaceAccessible`/`railState` (`:150-163`).
- **Mounting** — `gui/src/app/Shell.tsx`: **`WORKSPACE_VIEW`** map (`:88-135`) id → component;
  each workspace is its own `gui/src/workspaces/*Workspace.tsx` file.
- **Charting deps already present (no new dep needed)** — `gui/package.json:22-37`: **`@visx/*`**
  (SVG chart primitives), **`echarts` ^6.1.0**, **`lightweight-charts` ^5.2.0** (TradingView
  OSS), `three`. Existing hand-built charts: `gui/src/viz/{VolSmile,CurveChart,SmileChart}.tsx`,
  `gui/src/products/PayoffChart.tsx`; `gui/src/components/Provenance.tsx:18-50` is currently a
  **text-only** attribution line — a `PricingProvenance` *waterfall chart* does not yet exist.

## 4. Part 3 — architecture & data pipeline

The two pillars share one collection discipline (§1) and one GUI tab, but have distinct
storage/rollup shapes. **The hot core is untouched** — it already emits POD samples over the
lossy telemetry ring and lossless audit channel; Analytics is a *drain-side consumer*.

```
  hot core (pinned, alloc/lock/log-free)
     │  push HotSample (lossy SPSC ring)         │  AuditSink.record (lossless mpsc)
     ▼                                           ▼
  celnet-telemetry drain task                 celnet-analytics ingest task
     │  LatencyByKind (HdrHistogram/op)           │  event fold → TCA rollups
     │  + optional span export (sampled)          │  (per trade/desk/book/cpty/instrument)
     ▼                                           ▼
  LatencySnapshot store (per OpKind)          AnalyticsStore (bounded, queryable)
     │                                           │
     └──────────── query API (gRPC + WS) ────────┘
                         │
                   Analytics GUI tab (native charts)   +   Prometheus scrape/OTLP  ─▶  user-run Grafana/Jaeger
```

### 4.1 Pillar B — latency & span collection (`celnet-telemetry`)
- **In-process histograms (primary).** Wrap each §3.2 seam call site with a `LatencyByKind`
  recorder keyed by a new `OpKind` variant (`PriceInstrument`, `RiskRoute`, `Consolidate`,
  `TieringRun`, `QuoteRequest`, `QuoteAccept`, `Book`). Recording is a single POD push on the
  edge side; the drain task owns the HdrHistograms. Snapshots (`LatencySnapshot`, a `Copy` POD)
  are read lock-free per-op. **Coordinated-omission correction** is already in `LatencyRecorder`
  — use the expected-interval recording form for fixed-cadence paths (streaming) so a stall
  doesn't flatter the tail.
- **Span traces (secondary, sampled).** A `tracing` span per request (`request_quote`,
  `accept_quote`, ESP tick) with child spans at the §3.2 seams, exported via
  `tracing-opentelemetry` → **OTLP** to a user-run **Jaeger** (OTLP-native). **Head sampling**
  at span creation (cheap) is the default; a low sample rate on the hot path, 100% on
  error/slow paths. **No tail sampling in-process** — if wanted, it runs in the collector.
  Span export is bounded and offloaded; dropping spans never blocks a request.
- **Metrics export.** The existing `metrics_facade` keys already carry the histogram + counters;
  install a `metrics-exporter-prometheus` recorder behind the facade to expose a Prometheus
  scrape endpoint (native histograms for latency). This feeds ops/SRE Grafana (arm's-length).

### 4.2 Pillar A — TCA rollups (`celnet-analytics`)
- **Event source.** Tail the lossless **audit log** (§3.1) — `AuditStage` transitions carry
  quote/execution/book events with monotonic sequence, never dropped — joined to the
  `PricingProvenance` on each `Execution`/`Deal`. This is a pure, deterministic fold; no new
  server-side capture is required (the provenance and timestamps are already on the wire, §3.3).
- **Rollup dimensions.** Incrementally maintained aggregates keyed by *(trade, desk, book,
  counterparty, instrument, pricing_group, mode, time-bucket)* — O(1) per event, bounded memory.
- **Mark-out engine.** For each fill, sample the prevailing mid at a curve of horizons
  (+100ms/1s/5s/30s/1m/5m) from the live composite/surface and store drift vs `raw_mid` and vs
  executed price. Horizons are asynchronous timers on the drain side (a bounded delay-wheel),
  never on the trade path. Realized mark-out = executed price − mid@horizon (signed by side).
- **Derived metrics (from provenance + timestamps).**
  - *Spread capture / effective spread* = `outbound_offer − outbound_bid` vs `raw` composite;
    *applied margin/skew* read straight off provenance.
  - *Price improvement* = client-favourable delta of `outbound` vs `tiered`/benchmark.
  - *Tiering / pricing-group effectiveness* = win-rate & captured margin bucketed by
    `pricing_group_id` + `mode` (the provenance's own keys).
  - *RFQ win/loss & response time* = `QuoteRecord.dealers` + `booked_lp_id`;
    response time = `Execution.epoch_nanos − Quote.epoch_nanos`; *hit-ratio normalised by
    panel size* (BestX "RFQ Par" idea) = wins / f(panel_size).
  - *Slippage vs arrival/mid*, *implementation shortfall*, *reversion* from the mark-out series.
  - *PnL attribution* decomposed per provenance stage (raw→constructed→tiered→outbound).
  - *Volumes* = notional/count sums per dimension.
- **Storage.** In-memory `AnalyticsStore` (bounded ring per time-bucket + HdrHistograms for
  distributional metrics like mark-out and response time), rebuildable by replaying the audit
  log on restart. Sizing is O(dimensions × buckets); IB-scale is bounded by choosing
  bucket granularity + retention (guardrail 6).

### 4.3 Query API (both pillars)
New RPCs on the existing service surface (mirror pricing-groups / aggregated-book CRUD wiring:
proto → `handle_unary` + gRPC → dual WS codec (hand + generated, differential test) → store):
- `GetLatencySnapshots` — per-`OpKind` p50/p99/p99.9/p99.99/max + count (pillar B).
- `QueryTradeAnalytics{filter, group_by, horizon}` — the pillar-A rollups, paginated,
  filterable by the §4.2 dimensions and time range.
- `StreamAnalytics` — a WS push (off `NotificationBroker`, §3.3) for live tiles, memoised per
  rollup-version like the aggregation composite (lazy, not a timer thread).
No versioned APIs (guardrail 9): one current contract.

## 5. Part 3 — UI: the Analytics domain tab

A new **"Analytics"** domain, permission-gated, mounted next to Administration.

- **Registry** — add `"analytics"` (trader-facing pillar A) to `WorkspaceId` + a `RAIL` row
  (`gui/src/lib/commands.ts`), gated like Tiering (`quote_respond·fixed_income`, cross-asset via
  `CAPABILITY_ASSETS` so both FX-Options and FI traders see their own view). Pillar B's ops
  latency/trace dashboards go in a **second, admin-only** row (`"opslatency"`) added to
  `ADMIN_ONLY_WORKSPACES` alongside `connections/admin/permissions/pricinggroups/refdata` — so
  infra internals are ops-gated while traders get their TCA. (Alternatively one row with
  internal lens-gating, the pattern `surface/risk/book` already use.)
- **Mounting** — `analytics: AnalyticsWorkspace` (+ `opslatency: OpsLatencyWorkspace`) in
  `WORKSPACE_VIEW` (`gui/src/app/Shell.tsx`), each a new `gui/src/workspaces/*Workspace.tsx`.
- **Permission** — new RPCs gated via `AuthEdge::require_capability` (§3.4). If analytics-view
  must be distinct from per-trade `View`, add an `Action::Analytics` variant to the
  entitlements enum (a small, single-file, well-tested change: extend `ALL`/`label`/`from_label`).

### 5.1 Pillar-A dashboards (`AnalyticsWorkspace.tsx`) — native charts (visx/echarts)
- **Provenance waterfall chart** — a NEW component beside `Provenance.tsx`, rendering
  raw→constructed→tiered→outbound + margin/skew per fill (the exact wire fields, §3.3). The
  visual centrepiece: *see how each pricing stage moved the price*.
- **Mark-out curve** — drift vs horizon, per desk/book/counterparty/instrument (line/scatter;
  toxicity lens). **Spread & margin capture** tiles + trend. **Hit-ratio / win-rate**, RFQ
  **win/loss** and **response-time** distributions (panel-size-normalised "RFQ Par"). **PnL
  attribution** stacked by provenance stage. **Volumes / heatmaps** by dimension. **Tiering /
  pricing-group effectiveness** leaderboard. Filter/group-by controls map to the §4.3 query API;
  drill-through links to the blotter (showing the fill's provenance).
- Follow the dataviz skill; compositor-friendly motion only; theme-aware; both light/dark.

### 5.2 Pillar-B dashboards (`OpsLatencyWorkspace.tsx`)
- **Per-seam latency panels** — p50/p99/p99.9/p99.99 + count for each `OpKind`
  (`price_instrument`, `RiskRouter::route`, consolidation, tiering, quote, accept, book), as
  time-series + a percentile-bar strip. **Span waterfall** — a picked/sampled trace rendered as
  a span timeline (which stage took how long). A link-out to the user-run Grafana/Jaeger for
  deep infra drill-down (arm's-length; §6.3).

## 6. OSS stack decision & licensing verdict (guardrail 7)

### 6.1 What we ADOPT as runtime deps (all MIT/Apache/BSD)
`hdrhistogram` (already in `celnet-observability`) for p50/p99/p99.9; `metrics` +
`metrics-exporter-prometheus` behind the existing facade for Prometheus export; `tracing` +
`tracing-opentelemetry` + `opentelemetry`/`opentelemetry-otlp` for OTLP span export;
`pprof-rs` (Apache-2.0) if in-product CPU flame graphs are wanted. GUI charts reuse the
already-present visx / echarts / lightweight-charts.

### 6.2 What is REFERENCE-ONLY / arm's-length
BestX (commercial) — TCA methodology reference only. `tcapy` (Apache-2.0, Python) — port/reference.
Almgren–Chriss — academic method, implement first-party. `inferno` (CDDL-1.0) — dev/CI flamegraph
tool, never a linked dep. Grafana ≥ v8 / Jaeger / Zipkin / Prometheus server — **user-operated**
backends we export TO, never bundle.

### 6.3 Verdict table
| Tool / standard | License | Runtime dep? | Role for us |
|---|---|---|---|
| OpenTelemetry / OTLP / semconv | Apache-2.0 | ✅ | Span export wire format + conventions |
| `opentelemetry*` (Rust) | Apache-2.0 | ✅ (traces Beta) | OTLP exporter |
| `tracing` / `tracing-opentelemetry` | MIT | ✅ | Rust span instrumentation (coarse on hot path) |
| `hdrhistogram` (Rust) | MIT OR Apache-2.0 | ✅ | p50/p99/p99.9 + coordinated-omission (already used) |
| `metrics` + `-exporter-prometheus` | MIT AND Apache-2.0 | ✅ | Metrics facade + Prometheus export |
| `rust-prometheus` | Apache-2.0 | ✅ | Alt Prometheus client |
| `pprof-rs` | Apache-2.0 | ✅ (optional) | In-product CPU flame graphs |
| `inferno` | **CDDL-1.0** ⚠ | ❌ dev/CI only | Offline flamegraph render (prefer `pprof-rs`) |
| Prometheus (server) | Apache-2.0 | operate, not link | Metrics TSDB (native histograms) |
| Jaeger / Zipkin | Apache-2.0 | operate, not link | OTLP trace backend / interop |
| **Grafana ≥ v8, Loki, Tempo** | **AGPLv3** ⚠ | **arm's-length export only** — never bundle/fork/modify | User-run dashboards on our exported metrics |
| BestX | Commercial | ❌ | TCA reference model only |
| `tcapy` | Apache-2.0 (Python) | reference/port | OSS TCA methodology |
| Almgren–Chriss | Academic method | implement first-party | Impact / IS decomposition |

## 7. Naming (guardrail 8)
- **`celnet-telemetry`** (NEW leaf crate) — pillar-B latency/span collection, drain-side
  aggregation, snapshot store. (Distinct from `celnet-observability`, which stays the low-level
  hot-core-adjacent primitive layer; `celnet-telemetry` is the higher-level *aggregation/query*
  layer over it. Confirm the split vs folding into `celnet-observability` — §9.)
- **`celnet-analytics`** (NEW leaf crate) — pillar-A TCA rollups (pure fold over audit +
  provenance; independently oracle-testable against a hand truth table).
- GUI: `AnalyticsWorkspace` (trader TCA) + `OpsLatencyWorkspace` (admin infra). No vendor/
  person/paper names in any identifier (mark-out, not "Kissell"; impact model provenance in doc
  comments only).

## 8. Phased plan & crate/workstream breakdown (parallel-safe, disjoint files)

- **P0 — this spec + ADRs.** Confirm §9 open items; ADR for the export-vs-native split (§6.3),
  the `celnet-telemetry` vs `celnet-observability` boundary, and any new `Action::Analytics`.
- **P1 — pillar B collection.** `celnet-telemetry` (NEW): new `OpKind` variants + `LatencyByKind`
  wiring at the 7 §3.2 seams + snapshot store; `GetLatencySnapshots` RPC (proto → gRPC → dual WS
  codec + differential test). Independent oracle: known-latency injection → asserted percentiles.
- **P2 — pillar A rollups.** `celnet-analytics` (NEW): audit-log + provenance fold →
  `AnalyticsStore` rollups + mark-out engine; `QueryTradeAnalytics`/`StreamAnalytics` RPCs.
  Independent oracle: hand-computed truth table of fills → expected metrics.
- **P3 — GUI.** `AnalyticsWorkspace` (provenance waterfall + TCA dashboards) +
  `OpsLatencyWorkspace` (per-seam percentiles + span waterfall); registry + `WORKSPACE_VIEW` +
  capability gating; contract/transport + tests. Verified live.
- **P4 — export (optional, non-blocking).** Prometheus scrape endpoint behind the metrics facade
  + OTLP span export to Jaeger; sample Grafana dashboard JSON (arm's-length, shipped as *docs*,
  not bundled Grafana).

Each phase gated (`just t1` per crate; `just t2` at land). Numerical/statistical rollups
validated against an independent reference, never merely asserted plausible (guardrail 5).

## 9. Open items for the next review
- **Crate boundary:** new `celnet-telemetry` aggregation crate vs. extending
  `celnet-observability` in place. (Recommend the split — keeps the hot-core-adjacent primitive
  layer minimal; the aggregation/query layer has server-ish deps.)
- **New `Action::Analytics`?** or is `Action::View` scoped by `AssetClass` sufficient to gate the
  tab? (Recommend reuse `View` for pillar A; pillar B ops dashboards admin-gated.)
- **Mark-out horizon source:** which mid to sample against (live composite vs surface mid) and
  the exact horizon curve; how to handle instruments that stop quoting inside a horizon.
- **Retention & storage:** in-memory bounded rings (rebuild by audit replay) vs a durable
  time-series store for long-horizon TCA history; retention window per guardrail 6 scale budget.
- **Span sampling policy:** hot-path head-sample rate, and whether tail sampling in a collector
  is in scope for P4.
- **Export surface:** confirm we ship *dashboard JSON + a scrape endpoint* only, with Grafana/
  Jaeger explicitly user-operated (no bundling) — the AGPL boundary (§6).

---

### Appendix — key citations
- Observability primitives: `crates/celnet-observability/src/{latency.rs:1-354, metrics_facade.rs:1-238, audit.rs:1-336, logging.rs:1-372}`; init `crates/celnet-server/src/main.rs:49-51`.
- Seams: `crates/celnet-server/src/pricer.rs`; `crates/celnet-risk-routing/src/router.rs`; `crates/celnet-aggregation/src/consolidate.rs` + `services/aggregation.rs:666-695`; `crates/celnet-tiering/src/feature_pipeline.rs:32-49`; `services/{stream.rs, quote.rs:897/1443, desk/mod.rs:784, risk/store.rs:757-890}`.
- Data: `crates/celnet-proto/proto/celnet.proto` (`Quote:1579-1619`, `Execution:1731-1753`, `PricingProvenance:4426-4459`); `services/quote.rs:266-301`; `services/desk/store.rs:116-120`.
- Permissions: `crates/celnet-entitlements/src/capability.rs:1-361`; `services/auth.rs`; `gui/src/app/Shell.tsx:100-102`.
- GUI: `gui/src/lib/commands.ts` (`RAIL:106-148`, `ADMIN_ONLY_WORKSPACES:198-204`); `gui/src/app/Shell.tsx` (`WORKSPACE_VIEW:88-135`); `gui/package.json:22-37`; `gui/src/{viz/*,products/PayoffChart.tsx,components/Provenance.tsx:18-50}`.
- External: OpenTelemetry (Apache-2.0, [OTLP](https://opentelemetry.io/docs/specs/otlp/)); HdrHistogram + [coordinated omission](https://www.scylladb.com/2021/04/22/on-coordinated-omission/); Grafana [AGPL relicensing](https://grafana.com/blog/grafana-loki-tempo-relicensing-to-agplv3/); [pprof-rs](https://github.com/tikv/pprof-rs)/[inferno CDDL](https://github.com/jonhoo/inferno); TCA ([LMAX](https://www.lmax.com/documents/LMAXExchange-FX-TCA-Transaction-Cost-Analysis-Whitepaper.pdf), [markout](https://questdb.com/docs/cookbook/sql/finance/markout/), [BestX RFQ Par](https://thefullfx.com/bestx-rfq-par-introduces-a-new-way-to-look-at-hit-ratios/)); [tcapy Apache-2.0](https://github.com/cuemacro/tcapy).
</content>
</invoke>
