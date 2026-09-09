# Latency & Hedging Analytics — Requirements / Design

> Status: **requirements + design for review** (2026-07-30). No code yet. Extends the
> shipped **Analytics** domain (`docs/ANALYTICS-REQUIREMENTS.md`; pillar-A client-flow is
> **live** — crate `celnet-analytics`, `ListClientFlowMetrics` RPC, the `analytics` GUI
> domain) with **two new Analytics-tab surfaces** — a **Latency / Ops** workspace and a
> **Hedging** workspace — sitting beside the client-flow one, cross-asset, gated on the
> already-shipped `ViewAnalytics` capability. Grounds every seam `file:line` (current as of
> this doc). Follows the shipped **client-flow-analytics** / **risk-transfer** patterns
> (pure fold crate + proto/WS query RPC + a dedicated GUI workspace) as the template.
> Vendor-neutral naming (guardrail 8). OSS-only, academically-grounded methods (guardrail
> 7). **Capture must never slow the pinned zero-alloc pricing hot core (guardrail 11).** No
> mocks / placeholders / `todo!()` (guardrail 2).

---

## 0. The headline finding — the latency pipeline is BUILT-BUT-UNWIRED

The single most important grounding fact for **Part 1**: Celnet's latency-capture *primitives
already exist, are unit-tested, and embody guardrail 11 by construction* — **but almost none
of them are wired into the running server**. The work is overwhelmingly **wiring + exposure**,
not green-field invention.

- **BUILT + tested, UNWIRED:** the lossy wait-free SPSC telemetry ring (`telemetry_channel`
  → `HotProbe` / `TelemetryDrain`, `crates/celnet-observability/src/channel.rs`), the
  per-`OpKind` HdrHistogram aggregator (`LatencyByKind`, `latency.rs:200-233`), the
  coordinated-omission-corrected recorder (`LatencyRecorder`, `latency.rs:31-166`), the POD
  hot sample + tick→ns conversion (`HotSample` 32-byte `#[repr(C)]`, `TickRate`,
  `record.rs`), and the vendor-neutral metrics facade (`record_op` + the `celnet.*` keys,
  `metrics_facade.rs:17-73`). A repo-wide search finds **no** caller of `telemetry_channel`,
  `TelemetryDrain`, `HotProbe`, `LatencyByKind`, or `record_op` anywhere in
  `celnet-server` / `celnet-engine` (only the observability crate's own tests). **The
  pricing engine does not emit `HotSample`s at all today** — no cycle-counter (`cntvct_el0`
  / `rdtsc`) read, no `OpKind` use in `celnet-engine`.
- **The ONE live latency capture** is a single per-subscription `LatencyRecorder` on the ESP
  streaming edge: `crates/celnet-server/src/services/stream.rs:401` (field), `:1581`/`:3070`
  (construct), recorded at `:647` (`sub.latency.record_ns(compute_ns)`), and **already
  surfaced on the wire** as `server_price_p50_nanos` / `p99` / `p999` on every stream
  heartbeat (`stream.rs:2516-2518`; proto fields `celnet.proto:1857-1864`). Its own doc
  comment is the canonical guardrail-11 statement (`stream.rs`, `make_update`): *"Time the
  server-side price compute on the DRAIN path (the streaming edge), never the pinned
  zero-alloc hot core … so the price+Greek loop itself is untouched."* **This is the exact
  template the whole Part-1 capture design generalises.**

**Consequence for the plan (§8):** P0 is *"wire the capture that already exists"* — install a
`TelemetryDrain` + `LatencyByKind` on a non-critical core, teach the engine core to publish
`HotSample`s, and stand up the metrics facade behind a Prometheus recorder — **before** any
new stage instrumentation. Most of Part 1 is turning on latent, already-verified machinery.

---

## 1. Concept & goals — two surfaces, one capture discipline

Pillar A (client-flow / P&L attribution, `docs/ANALYTICS-REQUIREMENTS.md §11`) is **shipped**.
This doc adds the two surfaces the original Analytics design named but did not build out:

- **Latency / Ops analytics** (`docs/ANALYTICS-REQUIREMENTS.md` pillar B, §1/§2.1/§4.1):
  *how long the code took* and *how good our order/execution was* — the **tick-to-quote**
  price-latency path and the **tick-to-trade** best-order/execution path, as p50/p99/p99.9/
  p99.99/max HdrHistograms + per-stage spans. **The user's explicit emphasis: specify exactly
  WHAT to CAPTURE and WHERE to instrument to produce best order/execution stats and
  price-latency stats.** (§ Part 1.)
- **Hedging analytics** (consumes the just-spec'd **auto-hedging** feature,
  `docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`): internalisation ratio, warehouse
  duration / inventory half-life, warehouse P&L, hedge cost / slippage, hedge effectiveness,
  threshold-band time distribution, skew effectiveness — folded from `HedgeProvenance` + the
  position/risk history. (§ Part 2.)

Both mount on the existing **Analytics** domain tab beside **Client Flow**
(`gui/src/lib/commands.ts:105/244`, `section:"analytics"`), gated on `ViewAnalytics`
(`crates/celnet-entitlements/src/capability.rs:111`, label `"view_analytics"` `:153`).

### Design tenets (locked by guardrails)
- **Capture lives OFF the pinned zero-alloc hot core (guardrail 11).** Two capture tiers,
  both off the pricing thread: (a) the **pinned engine core** stamps a monotonic tick delta
  into a 32-byte POD `HotSample` and pushes it through the **lossy** SPSC ring — no alloc, no
  lock, no format, drop-on-full; (b) **async-edge stages** (already non-critical) record
  directly into a drain-side `LatencyByKind` exactly as `stream.rs:647` does today. All
  HdrHistogram bookkeeping, span export, and rollup run on the **drain / telemetry tier**.
  **Capture never adds work to a fill's synchronous pricing path** (§ Part 1 capture
  architecture).
- **Reuse the shipped fold pattern (DRY).** Hedging analytics is a **pure deterministic fold**
  (`HedgeRecord → HedgeMetrics`) mirroring `celnet-analytics`'s `metrics_from` /
  `ClientFlowMetrics` (`crates/celnet-analytics/src/metrics.rs:166`), oracle-testable against a
  hand truth table (guardrail 5). Latency is its own telemetry pipeline (HdrHistogram), not a
  trade fold.
- **OSS-only, MIT/Apache/BSD runtime deps (guardrail 7).** `hdrhistogram` (already linked),
  `metrics` + `metrics-exporter-prometheus`, `tracing` + `tracing-opentelemetry` +
  `opentelemetry-otlp` for span export. Grafana ≥ v8 / Jaeger are **arm's-length,
  user-operated** export targets, never bundled. Full verdict in §7.
- **Vendor-neutral, purpose-named (guardrail 8).** New crate **`celnet-telemetry`** (latency
  aggregation/query over `celnet-observability`) and **`celnet-hedge-analytics`** (the hedging
  fold). No vendor/person/paper names in identifiers; method provenance is doc-comment only.
- **One current contract, no versioning (guardrail 9); scale to IB portfolios & HFT
  counterparties (guardrail 6)** — HdrHistogram is fixed-size; the folds are O(events),
  bounded memory; query RPCs paginate.

---

# Part 1 — Latency & best-order / best-execution analytics

## 2. External / academic + regulatory research (cited)

Celnet reproduces the *concepts and outputs* of best-in-class latency/TCA tooling with
first-party OSS math (guardrail 7); per-tool licensing in §7.

### 2.1 Tail-latency measurement & distributed tracing (already partly in-repo)
- **HdrHistogram + coordinated omission (Gil Tene).** Record across a huge dynamic range in
  fixed memory and report **p50/p99/p99.9/p99.99** accurately instead of averaging the tail
  away; a stalled system stops issuing requests during the stall, silently omitting the very
  samples that should show it (**coordinated omission**), flattering p99 — corrected by
  recording against an *expected inter-arrival interval* that back-fills the omitted window.
  **This is already implemented and tested** in `LatencyRecorder::with_expected_interval` /
  `record_correct` (`latency.rs:53-84`, `coordinated_omission_inflates_the_tail` test
  `:280-302`). `hdrhistogram` = **MIT OR Apache-2.0**.
  ([HdrHistogram_rust](https://github.com/HdrHistogram/HdrHistogram_rust),
  [On Coordinated Omission](https://www.scylladb.com/2021/04/22/on-coordinated-omission/),
  [Gil Tene — How NOT to Measure Latency](https://www.infoq.com/presentations/latency-response-time/))
- **OpenTelemetry (OTel) spans.** A trace is a tree of **spans** (start/end, attributes,
  events) sharing a `traceId`/`parentSpanId`; **context propagation** stitches spans across
  boundaries; **OTLP** (protobuf over gRPC/HTTP) is the export wire; **semantic conventions**
  standardise attribute names. Rust: `opentelemetry` / `opentelemetry-otlp` +
  `tracing-opentelemetry` bridge. All **Apache-2.0 / MIT**. **Caveat for us:** `tracing` is
  *not* truly zero-cost even for a filtered span (per-callsite runtime check) — so spans stay
  at **coarse async-edge boundaries or compile-gated**, never inside the pinned pricing loop.
  ([OTLP](https://opentelemetry.io/docs/specs/otlp/),
  [opentelemetry-rust](https://github.com/open-telemetry/opentelemetry-rust),
  [tracing](https://github.com/tokio-rs/tracing))
- **Head vs tail sampling.** *Head sampling* (decide at span creation, before work) is the
  right default for a latency-critical service; *tail sampling* (keep slow/error traces) gives
  better signal but costs throughput and **must run in the collector, off-box** — never in the
  hot path. **In-process HdrHistogram aggregation beats per-span export** for latency SLOs.
  ([The Benefit of Hindsight](https://arxiv.org/pdf/2202.05769),
  [OTel sampling](https://uptrace.dev/opentelemetry/sampling))
- **Tick-to-trade** is the industry term for the full path from a market-data packet arriving
  to an order leaving; low-latency trading systems budget and measure it stage-by-stage
  (feed-handler → strategy/pricing → order-gateway), which is exactly the per-stage span
  decomposition below. ([STAC-T1 tick-to-trade benchmark](https://www.stacresearch.com/news/2019/03/11/stac-t1),
  [Databento — measuring tick-to-trade latency](https://databento.com/blog))

### 2.2 Clock & timestamp discipline — the regulatory basis for trustworthy stats
Best-order / best-execution statistics are only as trustworthy as their timestamps. The
regulated standard is explicit and is our design law for the two timestamp sources (§4.3):
- **MiFID II RTS 25 — clock synchronisation** (Commission Delegated Regulation **(EU)
  2017/574**): business clocks must be traceable to **UTC**, with a **maximum divergence from
  UTC** and a **timestamp granularity** tightening with activity — e.g. gateway-to-gateway
  latency ≤ 1 ms operations require **100 µs max divergence and 1 µs granularity**, and HFT
  requires **1 µs divergence / 1 ns granularity**. This is *why* Celnet separates a **trusted
  wall-clock** (UTC `epoch_nanos`, for cross-event RFQ/execution timing and best-ex) from a
  **monotonic tick counter** (for stage latency), never conflating them. ([EUR-Lex RTS 25 /
  2017/574](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32017R0574),
  [ESMA MiFID II clock sync Q&A](https://www.esma.europa.eu/))
- **Monotonic vs wall for durations.** A duration must be measured with a **monotonic** source
  (never wall-clock, which can step backwards on NTP adjustment); a *cross-process* timestamp
  for correlation must be **UTC wall-clock**. Celnet already does both: `TickRate`/`HotSample`
  carry opaque **monotonic** ticks converted to ns only on the drain (`record.rs:14-16,
  247-290`), and `Clock::now_nanos` is **system UTC** (`crates/celnet-server/src/clock.rs:75`),
  the source of every `epoch_nanos` on Quote/Execution. ([Rust `Instant` monotonicity](https://doc.rust-lang.org/std/time/struct.Instant.html))

### 2.3 Best-execution / execution-quality measurement (the "best order stats")
- **MiFID II best execution** (Directive 2014/65/EU **Art. 27**): firms must take *"all
  sufficient steps to obtain the best possible result"* on price, cost, speed, likelihood of
  execution/settlement, size and nature — i.e. **speed and likelihood are first-class
  execution-quality axes**, alongside price. This frames *why* we capture response latency,
  fill ratio, and reject/timeout rates as best-ex metrics, not just ops metrics.
  ([MiFID II Art. 27](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32014L0065))
- **RTS 27 / RTS 28 — execution-quality reporting** (Delegated Regs **(EU) 2017/575** venue
  quarterly, **(EU) 2017/576** firm top-5-venues annual). These define the *shape* of an
  execution-quality report: per-instrument **price, cost, speed, likelihood**, and for RTS 27
  intraday **latency between order receipt and execution** and **best-bid/offer at receipt vs
  execution**. **Reference model only** — RTS 27 was suspended and later removed under the
  MiFID "quick-fix" (Directive **(EU) 2021/338**) and UK divergence; we mirror the *metric
  taxonomy*, not the filing. ([RTS 27 / 2017/575](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32017R0575),
  [RTS 28 / 2017/576](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32017R0576),
  [MiFID quick-fix 2021/338](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32021L0338))
- **FX Global Code** (already cited in `ANALYTICS §11.1a`) governs speed/last-look/information
  handling for FX — the non-statutory best-ex analogue for our FX-ESP path.
  ([globalfxc.org](https://www.globalfxc.org/))
- **Latency-conditioned win analysis / TCA.** Standard practice conditions **win-rate on
  response latency** (a slow quote loses even at a good price), measures **slippage vs
  arrival/mid**, **price improvement vs cover** (cross-referencing the shipped client-flow
  cover metric, `celnet-analytics` `cover_distance`), and grades LP/venue **response time and
  fill ratio** — the same TCA lineage already cited in `ANALYTICS §2.3/§2.4` (LMAX TCA, markout,
  BestX RFQ-Par panel-size normalisation; **BestX is a reference model, never a dep**).

### 2.4 Low-overhead capture on a hot path (what is genuinely cheap)
- **A cycle-counter read is a few nanoseconds** (`rdtsc` on x86, `cntvct_el0` on aarch64) and
  is the correct hot-path clock — it does not trap, does not allocate, does not lock. Two reads
  bracketing an op yield an opaque `elapsed_ticks` (already the `HotSample` contract,
  `record.rs:170-199`). Conversion to ns is deferred to the drain via `TickRate`
  (`record.rs:247-290`). ([Intel — using rdtsc](https://www.intel.com/content/dam/www/public/us/en/documents/white-papers/ia-32-ia-64-benchmark-code-execution-paper.pdf))
- **Disruptor-style lossy offload (LMAX).** A wait-free SPSC ring with cache-line-isolated
  drop counters lets the producer publish-or-drop without ever blocking — **exactly**
  `channel.rs` (`HotProbe::publish`, `CachePadded` drop counter, provenance note `channel.rs:20-24`).
  ([LMAX Disruptor](https://lmax-exchange.github.io/disruptor/disruptor.html))

## 3. Celnet grounding — the primitives we build ON (cited `file:line`)

### 3.1 `celnet-observability` — the capture toolkit (mostly unwired, §0)
| Primitive | Location | State |
|---|---|---|
| `LatencyRecorder` (HdrHistogram + coordinated-omission) — `percentile_ns`/`p50`/`p99`/`p999`/`p9999`/`max`/`mean`/`snapshot`/`clear` | `latency.rs:31-166` | **built + tested; live only in `stream.rs`** |
| `LatencyByKind` (`[LatencyRecorder; OpKind::COUNT]`, no hashing on drain) | `latency.rs:200-233` | built + tested; **unwired** |
| `LatencySnapshot` (`Copy` POD: count/min/p50/p99/p999/p9999/max/mean) | `latency.rs:176-194` | built |
| `REPORTED_PERCENTILES = [50,99,99.9,99.99,100]` | `latency.rs:24` | built |
| `HotSample` (32-byte `#[repr(C)]` POD: `request_id, seq, elapsed_ticks, kind, class, core_id`) | `record.rs:170-205` | built; **engine emits none** |
| `OpKind` (`#[repr(u16)]`, 6 variants, `COUNT=6`) — `VanillaPrice/SurfaceVol/ExoticPrice/StreamQuote/RfqQuote/StatePublish` | `record.rs:24-79` | built; **needs extension (§4.2)** |
| `ErrorClass` (7 outcomes: Ok/InvalidInput/StaleState/NoConvergence/Arbitrage/TelemetryDropped/Internal) | `record.rs:94-162` | built |
| `TickRate` (opaque ticks → ns, 128-bit, drain-side only) | `record.rs:247-290` | built |
| `telemetry_channel` → `HotProbe::publish` (wait-free SPSC, drop-on-full, `CachePadded` counters) / `TelemetryDrain::drain`/`drain_all` (bounded) | `channel.rs:66-218` | built + tested; **no caller** |
| `metrics_facade` — keys `celnet.op.latency.ns`, `celnet.ops.total`, `celnet.telemetry.drained/dropped.total`, `celnet.engine.inflight`, `celnet.quotes.streamed.total`, `celnet.audit.committed.total`; `record_op(kind,class,ns)` | `metrics_facade.rs:17-73` | built; **no recorder installed** |
| `logging.rs` — `AuditRecord`/`AuditStage` (`quote_requested→quote_issued→quote_accepted/rejected→trade_booked`), `init_json_subscriber` | `logging.rs:142-224` | built; subscriber **installed** `main.rs:49-51` |
| `audit.rs` — lossless (never-drop) audit channel vs lossy telemetry split | `audit.rs:1-30, 229` | built |

Crate deps present: `hdrhistogram`, `metrics`, `tracing`, `tracing-subscriber`, `rtrb`,
`crossbeam-utils` (`crates/celnet-observability/Cargo.toml`). **NOT yet present (NEW, all
MIT/Apache):** `metrics-exporter-prometheus`, `opentelemetry`, `opentelemetry-otlp`,
`tracing-opentelemetry`.

### 3.2 The one live capture — the template to generalise
`stream.rs` `make_update` (the ESP heartbeat path) already does the guardrail-11-correct
thing: on the **edge/drain**, bracket only the `sub.price(...)` call with `Instant::now()` (a
monotonic read), record ns into a per-subscription `LatencyRecorder`, and surface p50/p99/p999
on the wire:
```
let t0 = std::time::Instant::now();
let (priced, two_way) = sub.price(market, spread)?;   // the pricer step
let compute_ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
sub.latency.record_ns(compute_ns);                    // stream.rs:647
// … server_price_p50_nanos/p99/p999 = sub.latency.p50_ns()/p99/p999  (stream.rs:2516-2518)
```
This proves the pattern end-to-end (capture → HdrHistogram → wire). Part 1 **generalises it**:
(a) more **stages**, (b) more **paths** (RFQ, booking) beyond ESP, (c) a **queryable
per-`OpKind` snapshot store** instead of a single per-subscription field, (d) full
**tick-to-trade spans**, and (e) the **pinned-core** producer via the SPSC ring (which
`stream.rs` sidesteps because it is already off-core).

### 3.3 The seams to instrument (price-latency + best-order paths)
Confirmed live signatures/call sites:
| Seam | File:line | Role in the latency path |
|---|---|---|
| Pricer | `crates/celnet-server/src/pricer.rs:511` `pub fn price_instrument(...)` (+ `price_instrument_lsv` `:1398`) | **priced** stage of tick-to-quote |
| ESP stream tick | `crates/celnet-server/src/services/stream.rs` `make_update` (`sub.price` @ `:647`), `make_snapshot` (`:600`), `drive_tick`/`drive_rates_tick`/`drive_agg_tick`/`drive_risk_tick` (`:2269/:2313/:2352/:2399`) | **market-data-in → priced → publish** (tick-to-quote); ESP cadence |
| Risk-version gate (stream) | `stream.rs:1210` `combined_risk_version`, tick loop `:2407` | republish trigger; latency of surface/risk rebuild |
| RFQ request | `crates/celnet-server/src/services/quote.rs:1032` `async fn request_quote`, publishes `epoch_nanos: now` `:1246` | **RFQ receive → price → respond** |
| RFQ accept | `quote.rs:1578` `async fn accept_quote` (`epoch_nanos` @ `:1562`/`:1811`) | **quote → lift/acceptance** timing |
| Desk twin | `crates/celnet-server/src/services/desk/mod.rs:949` `accept_desk_quote`; `:668` `book_fix_lift` (firm NewOrderSingle venue lift) | RFQ/venue accept + **order → book** |
| Booking sink (FX) | `crates/celnet-server/src/services/risk/store.rs:757` `PositionStore::book` | **ack → fill → book** commit latency |
| Booking sink (FI) | `crates/celnet-server/src/services/rates_book.rs` `RatesPositionStore::book` (via `risk/mod.rs:1019` `book_rates_position`) | FI **book** latency |
| Multi-dealer RFQ (external) | `crates/celnet-rfq/src/panel.rs` `MultiDealerEngine`; `lp_fix.rs` `FixLpAdapter` (FIX 4.4 `QuoteRequest→Quote`) | **per-LP response time**, win-rate-by-latency, fill/timeout |
| Wall clock | `crates/celnet-server/src/clock.rs:75` `Clock::now_nanos` (system UTC) | trusted `epoch_nanos` source (best-ex timing) |

None carry a `LatencyByKind`/`OpKind` recorder today (except the single `stream.rs`
per-subscription recorder). These are the instrumentation targets.

### 3.4 Timestamps already on the wire (best-order raw material — no new capture)
`docs/ANALYTICS-REQUIREMENTS.md §3.3` established these; they are the *cross-event* timing
inputs (distinct from stage latency): `Quote.epoch_nanos` (publication) + `valid_until_nanos`
(last-look deadline) (`celnet.proto:1579-1619`), `Execution.epoch_nanos` (`:1731-1753`),
`QuoteRecord.dealers: Vec<DealerQuote>` + `booked_lp_id` + `requester`
(`services/quote.rs:266-301`). RFQ **response time** = `Quote.epoch_nanos −
request-receive-nanos`; **acceptance latency** = `Execution.epoch_nanos − Quote.epoch_nanos`;
**win-rate-by-latency** and **response-time-by-LP** roll up off `dealers`/`booked_lp_id`. These
need only a **fold**, not new capture.

## 4. WHAT TO CAPTURE — the capture specification (the explicit ask)

Two families, two capture mechanisms, two clocks. **The rule: stage *durations* are captured
with the monotonic tick/`Instant` source on the producing side and aggregated off-core;
cross-event *timings* are computed by folding the UTC `epoch_nanos` already on the records.**

### 4.1 Price-latency capture points (tick-to-quote)
Each is a **stage span** with a start/stop; record the delta into a `LatencyByKind` recorder
keyed by a new `OpKind` (edge stages) or via the `HotSample` ring (pinned-core stages).

| # | Capture point | Where (seam) | Clock | Producer tier |
|---|---|---|---|---|
| L1 | **market-data-in → surface/curve rebuilt** (surface mark / republish age) | surface mark path feeding `StatePublish`; `stream.rs` `drive_*_tick` republish | monotonic ticks | pinned core (`HotSample`, kind `StatePublish` — already defined) |
| L2 | **priced** — `price_instrument` compute (vanilla/exotic/LSV) | `pricer.rs:511`/`:1398` | monotonic (`rdtsc`/`cntvct`) | pinned core (`HotSample`; `VanillaPrice`/`ExoticPrice`/`SurfaceVol` — already defined) |
| L3 | **spread / tiering applied** — `FeaturePipeline::run` (mid-shift→tier→guardrail) | `crates/celnet-tiering/src/feature_pipeline.rs` | monotonic `Instant` | async edge (`LatencyByKind`, NEW `OpKind::TieringRun`) |
| L4 | **aggregation / consolidation** — `ConsolidatedBook::from_quotes` | `crates/celnet-aggregation/src/consolidate.rs`; driver `services/aggregation.rs` | monotonic `Instant` | async edge (NEW `OpKind::Consolidate`) |
| L5 | **quote published** — snapshot/update assembled + token-minted | `stream.rs` `make_update`/`make_snapshot` | monotonic `Instant` | async edge (extend the live `sub.latency`; NEW `OpKind::StreamPublish`) |
| L6 | **tick-to-quote end-to-end** — L1→L5 total; **price staleness / age** = `now − last-republish` | `stream.rs` tick loop | ticks + UTC for age | derived |
| L7 | **ESP update cadence** — inter-publication interval per subscriber (drives coordinated-omission `expected_interval`) | `stream.rs` tick loop | monotonic | derived; feeds `with_expected_interval` (`latency.rs:53`) |

**Price staleness/age** (L6) uses the UTC clock (`now_nanos − epoch_nanos` of the last mark) —
a *freshness* metric, distinct from compute latency; a stale-but-fast quote is still a bad
quote.

### 4.2 `OpKind` extension (the one enum change Part 1 needs)
`OpKind` (`record.rs:24-79`) currently has 6 hot-core pricing variants (`COUNT=6`). The edge
stages above need labels for `LatencyByKind`. Extend the enum + bump `COUNT` + add
`from_u16`/`label` arms (a small, single-file, well-tested change; the array sizing keys off
`COUNT` automatically): **`TieringRun`, `Consolidate`, `StreamPublish`, `RfqRespond`,
`QuoteAccept`, `Book`, `RiskRoute`, `HedgeFire`**. Pinned-core stages (L1/L2) reuse the
existing 6. **Design note:** the SPSC ring is for the **pinned engine core** only; the async
edge stages (L3–L5, the RFQ/booking stages below) do **not** need the ring — they record
straight into a shared drain-side `LatencyByKind` behind a lightweight lock/atomic on the
already-non-critical edge, exactly as `stream.rs:647` records today. Two producer tiers, one
aggregator shape.

### 4.3 Best-order / best-execution capture points (tick-to-trade)
| # | Capture point | Where | Clock | Mechanism |
|---|---|---|---|---|
| O1 | **RFQ receive → priced → respond** latency | `quote.rs:1032` `request_quote` → `:1246` publish | monotonic (stage) + UTC (`epoch_nanos`) | `LatencyByKind` `RfqRespond` + fold of `epoch_nanos` |
| O2 | **quote → lift / acceptance** timing | `Quote.epoch_nanos` → `Execution.epoch_nanos` (`accept_quote` `quote.rs:1578`) | UTC | fold (no new capture) |
| O3 | **order ack → fill → book** latency | `desk/mod.rs:668` `book_fix_lift` / `store.rs:757` `book` / `rates_book.rs` `book` | monotonic (stage) | `LatencyByKind` `Book` |
| O4 | **execution quality — slippage vs arrival/mid** | fold of `Execution` price vs mid@receipt (reuse client-flow markout engine `ANALYTICS §4.2`) | UTC + composite mid | fold |
| O5 | **price improvement vs cover** | `QuoteRecord.dealers`/`booked_lp_id` (`quote.rs:266-301`) — **reuse shipped `cover_distance`** (`celnet-analytics`) | — | fold (already computed for client-flow) |
| O6 | **fill ratio / reject / timeout rates** | RFQ accept/reject/expire outcomes (`accept_quote`, last-look `valid_until_nanos`); `MultiDealerEngine` timeout/last-look (`panel.rs`) | UTC | count fold + `ErrorClass` |
| O7 | **best-ex venue/LP comparison** — response time by LP, fill ratio by LP, **win-rate-by-latency** | `MultiDealerEngine` `RankedPanel` per-LP `(price, epoch_nanos, lp_id)`; `FixLpAdapter` round-trip (`lp_fix.rs`) | UTC + monotonic | per-LP fold + `LatencyByKind` keyed by LP |
| O8 | **win-rate conditioned on our response latency** | join O1 latency to O2/O5 win/loss | UTC + monotonic | fold |

O4/O5/O6/O8 are **folds over data already captured** (per `ANALYTICS §3.3/§11.4`); O1/O3/O7
add **stage timers** on the RFQ/booking/LP paths. **Regulatory framing:** O1–O8 are precisely
the RTS 27 execution-quality axes (price, cost, **speed**, **likelihood**) and the MiFID Art.
27 best-ex factors (§2.3) — captured first-party, reported in the Latency/Ops workspace.

### 4.4 Metric catalogue — latency (mirrors `ANALYTICS §11.3` style)
Price-latency (per `OpKind`, HdrHistogram): `price_latency_ns{p50,p99,p999,p9999,max,mean,
count}` per stage L1–L5; `tick_to_quote_ns[percentiles]` (L6); `price_age_ns` / `staleness_ns`
(freshness); `esp_update_cadence_ns` (L7); `surface_rebuild_ns` (L1); `throughput_ops_per_s`
per `OpKind`; `telemetry_dropped_total` / `drained_total` (ring health, `metrics_facade` keys).
Best-order/execution: `rfq_respond_ns[pct]` (O1), `quote_to_lift_ns[pct]` (O2),
`ack_to_book_ns[pct]` (O3), `slippage_vs_arrival` / `slippage_vs_mid` (O4),
`price_improvement_vs_cover` (O5, reuse `cover_distance`), `fill_ratio`,
`reject_rate`/`timeout_rate` (O6), `lp_response_ns_by_lp[pct]` / `fill_ratio_by_lp` /
`win_rate_by_lp` (O7), `win_rate_by_response_latency_bucket` (O8), `hit_ratio` (panel-size
"RFQ-Par" normalised, reuse client-flow). All p50/p99/p99.9/p99.99/max via `LatencyRecorder`;
distributional metrics (slippage, markout) via HdrHistogram; ratios via count folds.

## 5. Zero-cost capture architecture (guardrail 11 — the critical section)

Two producer tiers, one drain tier. **The pinned pricing core is never asked to allocate,
lock, format, or read a wall clock.**

```
  PINNED ENGINE CORE (alloc/lock/log-free)          ASYNC EDGE (already non-critical)
     │  t0=rdtsc; price(); dt=rdtsc−t0                  │  Instant::now(); stage(); record_ns()
     │  HotProbe.publish(HotSample{elapsed_ticks=dt,    │  LatencyByKind.record_ns(kind, ns)
     │     kind, class, core_id})  ── drop-on-full ──▶  │      (L3–L5, O1, O3, O7)
     ▼          (lossy SPSC ring, channel.rs)           ▼
  ┌─────────────────────── TELEMETRY / DRAIN TIER (non-critical core) ──────────────────────┐
  │  TelemetryDrain.drain_all(sink):  TickRate.ticks_to_nanos(elapsed) → LatencyByKind       │
  │  + record_op(kind,class,ns) → metrics facade;  optional sampled tracing span export      │
  │  → per-OpKind LatencySnapshot store (lock-free Copy POD read)                            │
  └───────────────┬─────────────────────────────────────────────┬───────────────────────────┘
       GetLatencySnapshots RPC (gRPC + WS)          Prometheus scrape / OTLP → user-run Grafana/Jaeger
                  │                                              (arm's-length, §7)
          Latency/Ops GUI workspace
```

- **Hot-path cost budget (pinned core):** two cycle-counter reads (`rdtsc`/`cntvct_el0`, a few
  ns each, no trap) + one `HotProbe::publish` = one `rtrb` push of a 32-byte POD into
  pre-allocated ring storage, or a `Relaxed` drop-counter bump if full (`channel.rs:73-87`).
  **No heap, no lock, no format, no syscall, no wall-clock.** The `celnet-observability`
  crate's own `tests/zero_alloc.rs` allocation-counting guard (Cargo.toml note) exists to keep
  it so.
- **Lossy by design:** telemetry drops under back-pressure and counts the drop
  (`ErrorClass::TelemetryDropped`, `metrics.telemetry.dropped.total`) — it never blocks a
  price. The **audit** log (`audit.rs`, lossless) remains the never-drop stream for the *trade*
  events the best-order folds need.
- **Off-core rollup:** `TickRate` conversion, HdrHistogram `record_correct` (coordinated-
  omission), `record_op`, span export, and the snapshot store all live on the drain — heap and
  bookkeeping are fine there. Snapshots are `Copy` PODs (`LatencySnapshot`) read lock-free per
  `OpKind`.
- **Edge stages skip the ring:** L3–L5/O1/O3/O7 are already off the pricing thread, so they
  record straight into a shared drain-side `LatencyByKind` (the `stream.rs:647` pattern) — no
  SPSC hop needed, still zero cost to pricing.
- **Spans are coarse + sampled:** one `tracing` span per request (`request_quote`,
  `accept_quote`, ESP tick) with child spans only at async-edge seams; **head-sampled** at
  creation, 100% on error/slow; OTLP export is bounded and offloaded; dropping a span never
  blocks a request. **No span inside the pinned pricing loop.**

## 6. Latency query API + GUI (Latency / Ops workspace)

- **RPCs** (mirror the shipped `ListClientFlowMetrics` proto/WS/CRUD wiring — proto → gRPC
  `handle_unary` → dual WS codec (hand + generated, differential test) → store):
  `GetLatencySnapshots` (per-`OpKind` p50/p99/p999/p9999/max/count for L1–L5, O1/O3/O7),
  `QueryExecutionQuality{filter, group_by, horizon}` (O4–O8 folds, paginated), and a
  `StreamLatency` WS push off the existing tick/notification cadence for live tiles. No
  versioned APIs (guardrail 9).
- **Gating:** `ViewAnalytics` (`capability.rs:111`). Ops-internal infra (raw span waterfalls,
  ring drop-rates) MAY be additionally admin-gated via `ADMIN_ONLY_WORKSPACES`
  (`commands.ts`) if desks should see execution-quality but not infra internals — decided at
  review (§9).
- **GUI** `LatencyOpsWorkspace.tsx` under `section:"analytics"` beside `clientflow`
  (`commands.ts:244`), mounted in `WORKSPACE_VIEW` (`gui/src/app/Shell.tsx`). Native charts
  from the already-present libs (`gui/package.json`: visx / echarts / lightweight-charts — no
  new dep): per-stage **percentile-bar strip** + time-series (tick-to-quote L1→L5 decomposed),
  **best-ex panels** (RFQ respond / quote-to-lift / ack-to-book distributions; fill-ratio &
  reject/timeout; **LP league table** — response time, fill ratio, win-rate by LP;
  **win-rate-by-latency** curve), a **sampled span waterfall** (which stage took how long), and
  a link-out to the user-run Grafana/Jaeger for infra drill-down (arm's-length, §7). Follow the
  dataviz skill; theme-aware; compositor-friendly motion only.

---

# Part 2 — Hedging analytics (consumes the auto-hedging feature)

## 7A. Concept
Auto-hedging (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`) is *"internalise up to a
threshold, then hedge the overflow"* — a banded warehouse budget (green=warehouse,
amber=skew-to-attract, red=hedge-overflow), a trader-composed decision graph, and an immutable
**`HedgeProvenance`** stamped on every fired hedge. **Hedging analytics measures whether that
machinery is *working*:** are we internalising as much as we should, is warehoused inventory
turning over, is warehouse P&L positive net of adverse selection, and is external hedging
cheap and effective? It is the pillar-A sibling for the *inventory/hedge* lens, closing the
loop the client-flow analytics opened.

## 7B. External / academic research (cited — reuses the auto-hedging bibliography)
- **Internalisation ratio & inventory half-life.** Butz & Oomen (2019), *Internalisation by
  electronic FX spot dealers* (Quantitative Finance 19(1); SSRN 3076575) — the internalisation
  ratio (`1 − externalised/total`, 80–95%+ at large dealers) and the **half-life of inventory**
  are the two headline monitored metrics; higher internalisation saves impact/leakage but
  raises warehousing risk and holding time. **Direct basis** for `internalisation_ratio` and
  `inventory_half_life` as first-class outputs.
- **Warehouse-band optimality (what "good" looks like).** Barzykin, Bergault & Guéant (2023),
  *Algorithmic market making … with hedging and market impact* (Math. Finance 33(1);
  arXiv **2106.06974**) — the endogenous **no-hedge inventory band**: internalise by skewing
  inside the band, externalise only outside it. Hedging analytics reports **band-time
  distribution** (fraction of time green/amber/red) and **threshold-breach frequency** to show
  whether the configured band matches realised flow. Barzykin et al. (2021, arXiv **2112.02269**)
  — the continuous **hedging rate** (ramped externalisation) whose realised aggressiveness we
  measure.
- **Warehouse P&L = spread capture + mean-reversion − adverse selection.** Ho & Stoll (1981),
  Avellaneda & Stoikov (2008), Guéant–Lehalle–Fernandez-Tapia (2013), Cartea–Jaimungal–Penalva
  (2015) — inventory earns the spread and mean-reversion while bearing adverse selection.
  **Warehouse P&L attribution** decomposes realised inventory P&L into *spread captured* +
  *mean-reversion earned while holding* − *adverse-selection cost* (the markout on the
  warehoused flow, reusing the client-flow markout engine).
- **Hedge cost / slippage (execution of the overflow).** Almgren & Chriss (2000, J. Risk 3(2))
  + Almgren (2003) + Obizhaeva & Wang (2013) — the impact-vs-timing frontier; a `WORKED` hedge
  is scheduled, a `SUBMIT_MARKET_ORDER` is immediate. **Hedge cost** = external fill vs
  arrival/mid at fire (`HedgeProvenance.cost.slippage_bp`, `mid_at_fire`), the direct analogue
  of the client-flow slippage metric.
- **Skew effectiveness (did the passive lever work).** Avellaneda–Stoikov inventory skew /
  Bergault et al. (arXiv **1810.04383**, already in `celnet-tiering/src/strategy.rs:13`) — the
  amber-band `SKEW` lean should *attract offsetting flow*; we measure whether a skewed
  instrument subsequently mean-reverted (position reduced) and at what markout cost — the
  `skew_effectiveness` metric already named in `ANALYTICS §11.3`, now sourced from hedge/skew
  events.
- **Adverse selection / toxicity (why a hedge fired early).** Glosten–Milgrom (1985),
  Easley–López de Prado–O'Hara VPIN (2012), Cartea et al. (arXiv **2312.05827** / **2407.04510**
  / **2606.06413**) — toxic flow should externalise sooner; hedging analytics reports the
  **realised** toxicity of flow that was internalised vs externalised, validating the policy's
  toxicity branch.

All open/published; **no method identifier carries a person/paper name** (guardrail 8);
provenance is doc-only.

## 7C. Celnet grounding — the source events (cited `file:line`)
- **`HedgeProvenance`** (spec'd, `AUTO-HEDGING §8.1`) — the primary source event, mirroring
  `RiskTransferProvenance` / `PricingProvenance` (`celnet.proto:4426`): `trigger{metric,
  threshold, net_risk, utilization, band}`, `policy_path`, `action: ExitAction`,
  `netting{internal_crossed, external_hedged, residual}`, `fills: Vec<FillRef>`,
  `cost{hedge_price, mid_at_fire, slippage_bp, lp_won}`. **Every hedging metric below is a
  fold over a stream of `HedgeProvenance` records** joined to the position history.
- **Warehouse bands** (`AUTO-HEDGING §4`, `celnet-limits`): `LimitMetric` (`limit.rs:39`),
  `RagStatus` green/amber/red (`limit.rs:240`), `Utilization::headroom` (`limit.rs:283`) — the
  band-time-distribution source.
- **Position / inventory history:** `PositionStore` net/greeks + `aggregate_risk_book`
  (`services/risk/book_risk.rs:275`), `RatesPositionStore` DV01 (`book_risk.rs:157/237`),
  key-rate ladder + scenario VaR/ES (`celnet-rates-risk/src/{ladder,var}.rs`); the
  `risk_version` bump (`stream.rs:1210` `combined_risk_version`) times inventory changes for
  **half-life / aging**. Flat inventory + skew: `InstrumentInventory` (`services/aggregation.rs:97-133`),
  `InventorySkew` (`celnet-tiering/src/strategy.rs:91-137`), `AutoSkewSource` seam
  (`aggregation/src/risk.rs:48`) — skew-effectiveness inputs.
- **Internal vs external legs:** `CROSS_INTERNAL` (Agg Book cross, `celnet-aggregation`) vs
  `SUBMIT_MARKET_ORDER`/`RFQ_OUT` (`celnet-rfq` `MultiDealerEngine` / `FixLpAdapter`) —
  `HedgeProvenance.netting` records the split → **internalisation ratio** numerator/denominator.
- **The fold to reuse (DRY):** the shipped `celnet-analytics` pure fold — `FlowRecord →
  ClientFlowMetrics` via `metrics_from` (`crates/celnet-analytics/src/metrics.rs:166`), grouping
  helpers (`grouping.rs`). Hedging analytics is a **parallel crate `celnet-hedge-analytics`**
  with a `HedgeRecord → HedgeMetrics` fold of the **same shape** (single-pass `Acc`,
  divide-by-zero → `None`, oracle-testable). **Note the existing seam:** `FlowRecord` already
  carries `hedge_cost: Option<f64>` and `markout: Option<f64>` as *inputs*
  (`celnet-analytics/src/record.rs:92-95`) — hedging analytics is precisely what **produces**
  the `hedge_cost` figure the client-flow $/mm-net attribution consumes. The two folds
  compose: hedge-analytics → per-fill hedge cost → client-flow `dpm_net`.

## 7D. Metric catalogue — hedging (mirrors `ANALYTICS §11.3`)
Per instrument / book / desk (+ time-bucket): `internalisation_ratio` (`1 − Σexternal /
Σtotal`, per instrument/desk), `externalisation_cost` (Σ external hedge slippage $),
`inventory_half_life_secs` (decay time of net inventory magnitude), `warehouse_duration_secs`
(mean holding time before offset/hedge), `warehouse_pnl` decomposed `{spread_captured,
mean_reversion_earned, adverse_selection_cost}`, `hedge_cost_bp` / `hedge_slippage_vs_arrival`
(from `HedgeProvenance.cost`), `hedge_effectiveness` (residual net-risk reduction = `net_risk`
before vs after; **tracking error** of the hedged book vs a flat target),
`threshold_breach_frequency` (red-band fires per interval), `band_time_distribution`
(fraction green/amber/red, from `RagStatus`), `skew_effectiveness` (position mean-reversion
attributable to amber `SKEW` fires, at what markout cost), `internal_cross_fill_ratio`
(crossed vs attempted internal offset), `advisory_vs_live_divergence` (in dry-run: what the
policy *would* have done vs realised — the shadow-run metric, `AUTO-HEDGING §8.4`). Ratios
guard divide-by-zero → `None` (never `NaN`/`inf`, mirroring `celnet-analytics`).

Source-event map: every metric ← a fold of `HedgeProvenance` (netting/cost/trigger) × position
history (`risk_version`-timed snapshots) × the client-flow markout engine (adverse-selection
term). No new hot-path capture — `HedgeProvenance` is stamped off-core on the async booking
tier (`AUTO-HEDGING §7`), and the fold runs on the analytics ingest tier.

## 7E. Hedging query API + GUI (Hedging workspace)
- **RPC** `QueryHedgeAnalytics{filter, group_by, time_range}` (paginated) + `StreamHedgeMetrics`
  WS push, mirroring `ListClientFlowMetrics`. `ViewAnalytics`-gated.
- **GUI** `HedgingWorkspace.tsx` under `section:"analytics"`: **internalisation-ratio** tile +
  trend (per instrument/desk); **inventory half-life / aging** chart; **warehouse-P&L
  attribution** waterfall (spread / mean-reversion / adverse-selection — the pillar-A provenance
  waterfall discipline applied to warehousing); **hedge-cost / slippage** distribution;
  **band-time distribution** (green/amber/red stacked area) + breach-frequency; **skew
  effectiveness** (did skew attract offset); an **advisory-vs-live** panel for shadow-run desks.
  Drill-through to the `HedgeProvenance` record (the "why it hedged" beside routing's "why it
  landed" and transfer's "why it moved").

---

# Part 3 — Integration & architecture

## 8'. How the two surfaces sit in the Analytics domain
- **Both are NEW rows** under the existing `"analytics"` section (`commands.ts:244`), beside
  **Client Flow**, cross-asset via `CAPABILITY_ASSETS`, gated on `ViewAnalytics`
  (`capability.rs:111`; `WORKSPACE_CAPABILITY.clientflow = "view_analytics"` `commands.ts:343`
  is the exact precedent). Add `"latencyops"` and `"hedging"` `WorkspaceId`s + `RAIL` rows +
  `WORKSPACE_VIEW`/`WORKSPACE_CAPABILITY` entries. **No new capability needed** — `ViewAnalytics`
  already spans FI+FXO by design (`capability.rs:102-111`). *Finer split (open item §9):* an
  optional admin-only variant of the Latency/Ops row for raw infra internals.
- **Rollup reuse:** hedging analytics **reuses the `celnet-analytics` fold pattern** verbatim
  in shape (`HedgeRecord → HedgeMetrics`, pure, oracle-tested). Latency is **its own telemetry
  pipeline** (`celnet-telemetry` over `celnet-observability`'s HdrHistogram) — not a trade fold.
- **Off-core throughout (guardrail 11):** latency capture is the §5 two-tier design (pinned-core
  ring + edge timers, drained off-core); hedging analytics folds `HedgeProvenance` which is
  itself stamped off the pricing thread on the booking tier. Neither adds work to a price.
- **Provenance/audit consistency:** best-order stats fold the lossless **audit** stream
  (`audit.rs`) + `epoch_nanos`; hedging stats fold the immutable `HedgeProvenance` — the same
  never-drop, structured, attributable discipline as pillar A and risk-transfer.

## 7. OSS / licensing verdict (guardrail 7)
### Runtime deps (ADOPT — all MIT/Apache/BSD)
`hdrhistogram` (already linked; p50/p99/p99.9 + coordinated omission); `metrics` +
**`metrics-exporter-prometheus`** (NEW; MIT/Apache — Prometheus scrape behind the existing
facade, native histograms for latency); **`opentelemetry` / `opentelemetry-otlp` /
`tracing-opentelemetry`** (NEW; Apache-2.0/MIT — sampled OTLP span export); GUI charts reuse
the present visx / echarts / lightweight-charts (`gui/package.json`). `pprof-rs` (Apache-2.0)
optional for in-product CPU flame graphs.
### Reference-only / arm's-length
BestX / RTS 27-RTS 28 report formats — execution-quality **reference model only**, never a
dep (and RTS 27 is regulatorily suspended, §2.3). `tcapy` (Apache-2.0) — TCA methodology
port/reference. Almgren–Chriss / Butz–Oomen / Barzykin et al. — **academic methods, implemented
first-party** (cited in comments). `inferno` (CDDL-1.0) — dev/CI flamegraph render only.
**Grafana ≥ v8 / Jaeger / Zipkin / Prometheus server** — **user-operated** backends we export
*to*, never bundle/fork/modify (Grafana AGPLv3 boundary, `ANALYTICS §6.2`).
### Verdict table (additions to `ANALYTICS §6.3`)
| Tool / standard | License | Runtime dep? | Role |
|---|---|---|---|
| `hdrhistogram` | MIT/Apache-2.0 | ✅ (linked) | Tail percentiles + coordinated omission (built) |
| `metrics` + `-exporter-prometheus` | MIT/Apache-2.0 | ✅ NEW | Facade + Prometheus scrape (native histograms) |
| `opentelemetry*` / `tracing-opentelemetry` | Apache-2.0 / MIT | ✅ NEW (traces Beta) | Sampled OTLP span export |
| `pprof-rs` | Apache-2.0 | ✅ optional | In-product CPU flame graphs |
| MiFID II RTS 25 (2017/574) clock sync | EU regulation | ❌ (adopt the discipline) | UTC/monotonic timestamp policy (§2.2/§4.3) |
| MiFID II RTS 27/28 (2017/575-576) | EU regulation | ❌ reference | Execution-quality metric taxonomy (§2.3) |
| FX Global Code | Industry code | ❌ reference | FX best-ex / last-look / info-handling |
| Almgren–Chriss / Butz–Oomen / Barzykin et al. | Academic | implement first-party | Hedge cost / internalisation / band metrics |
| Grafana ≥ v8 / Jaeger / Prometheus (servers) | AGPLv3 / Apache-2.0 | operate, not link | Arm's-length export backends |

## 8. Phased plan (parallel-safe, disjoint files; gated `just t1` per crate, `just t2` at land)
- **P0 — WIRE THE CAPTURE THAT ALREADY EXISTS (the §0 finding).** Install a `TelemetryDrain` +
  a shared `LatencyByKind` on a non-critical core in `celnet-server`; teach the engine core to
  read the cycle counter and `HotProbe::publish` a `HotSample` around `price_instrument` (L1/L2,
  reusing the 6 existing `OpKind`s); install a `metrics-exporter-prometheus` recorder behind the
  facade (`record_op` starts flowing); a `GetLatencySnapshots` RPC exposing the per-`OpKind`
  `LatencySnapshot` store. **Oracle:** known-latency injection → asserted percentiles (extend
  the `latency.rs` tests). *Turns on latent, already-verified machinery — minimal new code.*
- **P1 — edge stage timers + `OpKind` extension.** Add the NEW `OpKind` arms (§4.2) + bump
  `COUNT`; wire L3–L5 (tiering / consolidation / stream-publish) and O1/O3 (RFQ respond / book)
  edge timers into a shared drain-side `LatencyByKind` (the `stream.rs:647` pattern); generalise
  the live per-subscription recorder into the store. Tick-to-quote (L6) + staleness/cadence
  (L6/L7).
- **P2 — best-order/execution folds + `celnet-telemetry` query crate.** `QueryExecutionQuality`
  (O4–O8): slippage / fill-ratio / reject-timeout / LP league table / win-rate-by-latency —
  folds over audit + `epoch_nanos` + `QuoteRecord` (reuse `cover_distance`). Independent oracle:
  hand truth table of fills/timestamps → expected stats.
- **P3 — Latency/Ops GUI.** `LatencyOpsWorkspace.tsx`: per-stage percentile strips, tick-to-quote
  decomposition, best-ex panels, LP league table, sampled span waterfall; registry + gating +
  transport tests. Verified live.
- **P4 — sampled OTLP spans + Prometheus export (optional, non-blocking).** `tracing` spans at
  async-edge seams, head-sampled, OTLP → user-run Jaeger; sample Grafana dashboard JSON shipped
  as *docs* (arm's-length, no bundled Grafana).
- **P5 — hedging analytics** *(depends on auto-hedging P1+ landing so `HedgeProvenance` exists)*.
  `celnet-hedge-analytics` (NEW leaf crate): `HedgeRecord → HedgeMetrics` fold (§7D),
  oracle-gated; `QueryHedgeAnalytics`/`StreamHedgeMetrics` RPCs; `HedgingWorkspace.tsx`
  (internalisation ratio, half-life, warehouse-P&L waterfall, hedge cost, band-time, skew
  effectiveness, advisory-vs-live). Composes back into client-flow `dpm_net` via the shared
  `hedge_cost` input (`celnet-analytics/src/record.rs:92-95`).

## 9. Open items for the next review
- **`OpKind` growth vs a separate stage enum.** Extending the hot-core POD `OpKind` (§4.2) is
  simple but mixes pinned-core pricing ops with async-edge stages in one enum/array. Alternative:
  a distinct `StageKind` for the edge tier, leaving `OpKind` for the ring. (Leaning: one enum —
  the `COUNT`-sized array is trivial and the drain aggregates uniformly.)
- **`celnet-telemetry` crate boundary.** New aggregation/query crate over `celnet-observability`
  vs extending `celnet-observability` in place (the `ANALYTICS §9` open item, still open).
  Leaning: split (keep the hot-core-adjacent primitive layer minimal).
- **Latency/Ops gating granularity.** `ViewAnalytics` for execution-quality (traders' concern)
  vs an additional admin-only row for raw infra internals (ring drop-rates, span waterfalls). Is
  best-ex a trader surface and infra an ops surface, or one row with lens-gating?
- **Coordinated-omission interval per path.** Which paths use `with_expected_interval` (ESP has
  a natural cadence L7; RFQ/booking are event-driven with none) — and how the ESP cadence is
  measured to feed it.
- **Span sampling policy.** Hot-path is span-free by rule; confirm the async-edge head-sample
  rate and whether tail sampling in a collector is in P4 scope.
- **Clock-sync assurance (RTS 25).** Do we assert/monitor UTC divergence + granularity of the
  `Clock` source (§2.2), and surface a clock-health metric — a prerequisite for defensible
  best-ex timestamps at HFT granularity.
- **Hedging analytics depends on auto-hedging.** `HedgeProvenance` must land first; until then
  the warehouse-P&L / band-time metrics can be *back-computed* from the position history +
  limits bands (no `HedgeProvenance`) as an interim — decide whether P5 waits or ships a
  provenance-free interim.
- **Warehouse-P&L attribution boundary.** Splitting realised inventory P&L into spread /
  mean-reversion / adverse-selection needs a mark series + a holding-period attribution rule
  (reuse the client-flow markout horizons?) — confirm the horizon curve and the mean-reversion
  vs adverse-selection cut.
- **Retention & scale (guardrail 6).** HdrHistogram is fixed-size; the best-order/hedging folds
  are bounded rings per time-bucket rebuildable from the audit log. Confirm bucket granularity +
  retention window for IB-scale.

---

### Appendix — key citations
- **Observability primitives (built):** `crates/celnet-observability/src/{latency.rs:24-233,
  record.rs:24-290, channel.rs:66-218, metrics_facade.rs:17-73, audit.rs:1-30, logging.rs:142-224}`;
  subscriber init `crates/celnet-server/src/main.rs:49-51`. **Live capture:**
  `crates/celnet-server/src/services/stream.rs:401/647/2516-2518`; wire fields
  `crates/celnet-proto/proto/celnet.proto:1857-1864`.
- **Seams:** `pricer.rs:511/1398`; `services/quote.rs:1032/1578/266-301`;
  `services/desk/mod.rs:668/949`; `services/risk/store.rs:757`; `services/rates_book.rs`;
  `crates/celnet-rfq/src/{panel.rs,lp_fix.rs}`; `crates/celnet-tiering/src/feature_pipeline.rs`;
  `crates/celnet-aggregation/src/consolidate.rs`; `clock.rs:75`.
- **Shipped pillar-A reuse:** `crates/celnet-analytics/src/{lib.rs, record.rs:56-96,
  metrics.rs:166}`; proto `celnet.proto:5022/5061-5076`; capability `celnet-entitlements/src/capability.rs:102-153`;
  GUI `gui/src/lib/commands.ts:105/244/343`, `gui/src/workspaces/analytics/`.
- **Auto-hedging source:** `docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` (`HedgeProvenance`
  §8.1, bands §4, seams §2); `crates/celnet-limits/src/limit.rs:39/240/283`;
  `services/risk/book_risk.rs:157/237/275`.
- **External / regulatory:** MiFID II RTS 25 clock sync ([2017/574](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32017R0574));
  RTS 27/28 exec-quality ([2017/575](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32017R0575),
  [2017/576](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32017R0576)), quick-fix
  ([2021/338](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32021L0338)); MiFID Art. 27
  ([2014/65/EU](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32014L0065));
  [FX Global Code](https://www.globalfxc.org/); HdrHistogram + [coordinated omission](https://www.scylladb.com/2021/04/22/on-coordinated-omission/) /
  [Gil Tene](https://www.infoq.com/presentations/latency-response-time/); [OTLP](https://opentelemetry.io/docs/specs/otlp/) /
  [opentelemetry-rust](https://github.com/open-telemetry/opentelemetry-rust) / [tracing](https://github.com/tokio-rs/tracing);
  [tail sampling](https://arxiv.org/pdf/2202.05769); [LMAX Disruptor](https://lmax-exchange.github.io/disruptor/disruptor.html).
- **Hedging methods:** Butz & Oomen 2019 (SSRN 3076575); Barzykin–Bergault–Guéant 2023
  (arXiv 2106.06974) / 2021 (arXiv 2112.02269); Almgren–Chriss 2000, Almgren 2003, Obizhaeva–Wang
  2013; Ho–Stoll 1981, Avellaneda–Stoikov 2008, Guéant–Lehalle–Fernandez-Tapia 2013,
  Cartea–Jaimungal–Penalva 2015; Glosten–Milgrom 1985, Easley–López de Prado–O'Hara 2012 (VPIN);
  Cartea et al. arXiv 2312.05827 / 2407.04510 / 2606.06413 (all per `AUTO-HEDGING §12` / `ANALYTICS §11`).
</content>
</invoke>
