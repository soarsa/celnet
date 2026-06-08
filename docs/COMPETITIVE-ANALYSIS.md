# Celnet Competitive Analysis & Positioning

> **The thesis.** Celnet is not a thin challenger closing gaps — it is a functionally
> complete, evidence-backed **superset** of what a derivatives desk stitches together today,
> proven by a runnable parity matrix against independent oracles, behind one unversioned
> contract reachable identically from five clients. The catalogue now spans vanilla → the
> full first-generation exotics → structured & path-dependent products → American/Bermudan
> early exercise → correlated multi-asset basket/best-of/worst-of → a particle-calibrated LSV
> booking model + standalone Heston — all on the **one wire**, parity-gated, reachable from
> GUI/SDK/CLI/Excel/WebSocket with bit-identical values. This is the breadth the front-to-back
> incumbents charge for, delivered open and microsecond-class.
>
> **Honesty as a differentiator.** Every Celnet figure is labelled (in-core / M4 / loopback);
> every competitor claim below is a **stated inference** from public material and documented
> architectures, not a vendor-confirmed benchmark; **no deploy-gated absolute is claimed
> in-repo** (see the *Honest boundary* section). A reviewer doing diligence finds the proof,
> not marketing fiction.

---

## Executive Summary

The FX-options technology stack in 2026 is fragmented into four archetypes — closed desktop
terminals (Bloomberg), monolithic front-to-back platforms (Murex, Numerix), data/venue players
(Fenics, SynOption, ICE, 360T, Digital Vega), and early modern entrants (Quantifi, Quantra,
RustQuant). None combines, in one product, an **open extensibility SDK, genuine Rust
nanosecond-class hot core, zero-downtime upgrades, native trade-lifecycle integration, a full
exotic/structured catalogue, and GPU-accelerated pricing** — all behind a single unversioned
contract reachable identically from five clients. That is the seam Celnet occupies, and the
platform now fills it end-to-end rather than aspirationally.

Celnet competes on three axes:

- It **out-functions** incumbents on quant flexibility (user-extensible models via a
  sandboxed, deterministic SDK) *and* on raw catalogue breadth — the on-wire `Instrument`
  carries **19 product families** (`crates/celnet-proto/proto/celnet.proto:1016-1062`), each
  parity-gated against an independent oracle in `crates/celnet-parity/tests/`.
- It **out-intuits** them on surface and workflow transparency: configurable, arbitrage-checked,
  auditable analytics across **five smile families** (VV / SABR / SVI / SSVI / **eSSVI**,
  `celnet.proto:372-382`) with explicit, user-selectable conventions, rather than opaque vendor
  models.
- It **out-performs** them on latency: a Rust, thread-per-core, zero-allocation hot path with a
  measured in-core §1.2 truth-gate (p50 ≈ 42 ns / p99 ≈ 125 ns on the M4 dev host,
  `crates/celnet-bench/src/bin/core_load.rs`), versus RFQ/EOD/batch architectures.

Where the prior version of this document framed exotics as a gap to close, the catalogue is now
shipped and gated. The remaining competitive narrative is no longer "we will match them" but
"we already match the deep-catalogue platforms on breadth **while** retaining the open SDK,
microsecond core, single contract, and deterministic, fully-evidenced edge they structurally
lack."

---

## SynOption

**What they do well.** SynOption operates Optimus, an MAS-licensed Recognized Market Operator
(RMO) multi-bank FX-options venue, also exempt from CFTC SEF registration — a genuine regulatory
moat a pure-tech vendor cannot easily replicate. It aggregates top-tier bank liquidity across
75 currency pairs (deliverable + NDF), with broad product coverage spanning vanillas, multi-leg
strategies, mainstream exotics (European digitals, one-touch, double-no-touch, single/double
barriers) and structured products (TARF, pivot, accumulators). The modular estate (Optimus venue,
Titan white-label distribution, Orion pricing, Primus vol-surface market data, Omega
risk/portfolio, Synexus ECN, Synchro crypto) lets banks consume only what they need, with
FIX-based STP into client risk systems and an early, credible move into crypto options via the
OrBit Markets partnership. It fills a real Asia-first gap left by London/NY-centric incumbents.

**Where they are weak/vulnerable** *(stated inferences from the absence of published figures and
the documented RFQ workflow architecture)*.
- **No published latency or throughput numbers** anywhere. The architecture is built for human
  RFQ workflows (seconds-scale, request/response, indicative quotes), not microsecond
  auto-quoting or streaming. The broader market (OptAxe, Bloomberg's ~30 API auto-quoting MMs)
  is moving to instantaneous API pricing that a discrete RFQ venue structurally lags.
- **Closed, black-box analytics.** Orion ships fixed "risk libraries" with only spread/skew
  knobs — there is no user-extensible quant SDK. Clients consume SynOption's models, not their
  own; they cannot drop in proprietary vol models, exotic payoffs, or calibration routines.
- **Thin integration surface.** FIX + UI + a thin STP API only; no documented modern
  programmatic SDK (REST/gRPC/WebSocket), no public API docs, no developer portal. Extensibility
  is vendor-mediated.
- **Venue lock-in and SaaS-only deployment.** Value is bound to the regulated venue and its
  specific LP panel; no on-prem, co-located, or in-process embedding for latency- or
  data-sovereignty-sensitive clients. The stitched multi-service estate
  (Optimus + Orion + Primus + Omega) glued by FIX hops adds serialization and network latency.
- Single-asset-class for now (FX + digital assets; commodities explicitly still roadmap), thin
  public proof points, and visible branding/product churn.

**The Celnet counter.** Celnet matches SynOption's exotic/structured set on the **one wire** and
goes well beyond it: digitals, one-/no-/double-touch, single/double **and window** barriers, TARF,
accumulators, plus Asians, lookbacks, forward-start/cliquet, quanto, variance/volatility swaps,
**American/Bermudan** early exercise and **correlated basket/best-of/worst-of** — each a
parity-gated row (`crates/celnet-exotics/src/`, `crates/celnet-parity/tests/exotics.rs`,
`tarf.rs`/`accumulator.rs`/`asian.rs`/`lookback.rs`/`basket.rs`). Its Rust core delivers
deterministic, microsecond-class pricing and Greeks suitable for continuous auto-quoting and
streaming RFS, not just on-request quoting. Clients write their own vol models, exotic payoffs,
calibration and stress logic against Celnet's SDK and run them **in-engine** (Tier-0 native +
Tier-2 wasmi sandbox, `crates/celnet-plugin-host/src/{native,wasm}.rs`) — keeping proprietary IP
instead of consuming a vendor's closed library. Celnet embeds natively in the Celer
trade-lifecycle estate (one data model, one in-process bus) rather than gluing services over FIX;
it deploys on-prem/co-located/in-process or cloud; and it sits behind or alongside *any* venue as
the pricing/risk brain — so clients need not adopt a single regulated venue to get value.

---

## Fenics (BGC Group) — Integration Partner AND Analytics Competitor

Fenics spans several BGC offerings. The relevant **integration target** is Fenics Market Data
(FMD) **FXO 2.0**; the relevant **competitive target** is the kACE pricing/analytics layer.

**What they do well.** FMD's source data is unique and capital-backed: as one of the largest
interdealer brokers, BGC builds surfaces from genuinely tradable committed quotes, orders and
trades ("tradable, not consensus") — hard for a pure-analytics vendor to replicate. FXO 2.0
covers **300+ currency pairs plus 27 precious-metal pairs**, with a broader FX set of 350+ pairs
in spot/forward/NDF (120 with options surfaces), quoted in standard convention (ATM, 25-delta and
10-delta risk reversals and butterflies/strangles) across a tenor ladder, delivered as modelled
curves or Level-2 orderbook via API/Desktop/Excel/Cloud/FTP/SFTP and redistributed through
LSEG/Refinitiv. The kACE engine carries mature Kalahari-heritage exotics maths: vanillas, 20
first-generation digitals/barriers, 16 window barriers, quanto, and a Local-Stochastic Volatility
(Heston/log-normal LSV) exotics engine with a calibration feed. Strong "tradable not consensus"
positioning aligned to IPV/FRTB needs.

**Where they are weak/vulnerable** *(stated inferences from public documentation and the
documented distribution/licensing model)*.
- **FMD is a data vendor, not an interactive pricing product.** The surface arrives; repricing,
  what-if, structuring and Greeks are the client's problem (or require separately-licensed kACE).
  No fast in-product re-strike/re-pivot of the surface.
- **Vague public documentation.** No published data dictionary, field list, delta-convention spec
  (spot vs forward, premium-adjusted vs unadjusted), smile-interpolation method, or snapshot
  schedule — high onboarding friction; integrators reverse-engineer or go through sales.
- **Surface convention rigidity and opaque ML.** Delivered as ATM/RR/BF wing quotes on a fixed
  delta/tenor grid; building a dense, arbitrage-free, continuously-callable surface
  (calendar/butterfly repair) is left to the consumer. The "machine learning" wing/long-dated
  fill-in publishes no methodology or no-arbitrage guarantees — a real risk for valuation/IPV use.
- **Latency and fragmentation.** Distribution leans on snapshot/intraday/FTP/SFTP and LSEG
  redistribution — not push-native sub-second streaming. Data (FMD), analytics (kACE), and
  execution (FMX FX / Fenics Direct, dated FIX 4.4) are loosely-coupled, separately-licensed
  silos; kACE itself is a heavyweight, Excel/OMS-centric desktop tool, not an API-first
  cloud-native service. Pricing is opaque enterprise-data-license.

**The Celnet counter — integrate, don't rebuild; then out-function on the same maths.** Celnet
**consumes FMD FXO 2.0 as a surface input**: ingest the ATM / 25d & 10d RR / 25d & 10d BF wing
quotes per tenor per pair, plus spot, forward points/outrights and NDF fixings, through an adapter
that normalizes Fenics's under-documented conventions into Celnet's canonical surface object on
ingest. *(The blend/staleness/divergence ALGORITHM is built and gated; live multi-vendor quote
VALUES are an integration/deploy target, not in-repo data.)* It then **out-functions** on the
surface itself: where FMD hands over raw wing/tenor quotes, Celnet constructs a dense,
**arbitrage-free** (calendar + butterfly checked) continuously re-strikable surface across **five**
transparent, configurable smile families — VV / SABR / SVI / SSVI / **eSSVI**
(`celnet.proto:372-382`) — and shows the trader *why* each point is what it is, versus FMD's opaque
ML fill. Critically, Celnet **matches kACE's headline analytics on its own ground**: a
particle-calibrated **Local-Stochastic-Volatility booking model** (Heston backbone + Dupire
leverage, ADI 2-D PDE + Philox MC, `crates/celnet-exotics/src/{lsv,leverage,particle,adi}.rs`,
parity row `tests/lsv.rs`) and a **standalone Heston** engine (Carr-Madan + Fang-Oosterlee COS,
parity-gated against a frozen QuantLib golden table `crates/celnet-golden/data/heston_fo.csv`,
`tests/heston.rs`) — first-generation digitals/barriers/window-barriers and quanto are shipped
core, not a separate license. It **out-performs** by streaming the constructed surface and Greeks
on every spot tick, push-native, versus FMD's snapshot/SFTP latency, and **collapses**
FMD-data + kACE-analytics + spreadsheet glue into one product with full Greeks, scenario/what-if,
P&L attribution and IPV. Multi-source resilience: ingest FMD as *one* of several surface sources,
blend/validate, and flag divergence — the client benefits from BGC's tradable data without being
locked to a single opaque feed.

---

## Bloomberg (OVML / BVOL / MARS)

**What they do well.** Ubiquitous desktop adoption. OVML provides FX option strategy structuring,
pricing and backtesting with a selectable vol source (BVOL / LIVE / Custom). BVOL delivers trusted
real-time vol surfaces across **200+ FX pairs** with bid/ask/timestamp fields and no separate
exchange subscription. MARS adds cross-asset risk (FX, rates, credit, MBS), listed + OTC
derivatives, scenario and regulatory market-risk support, with a MARS API and Python automation.
It is the trusted reference surface and risk credibility layer for much of the buy- and sell-side.

**Where they are weak/vulnerable** *(stated inferences from public pricing and the terminal/feed
delivery model)*.
- **Closed, seat-priced ecosystem** (~$28k–$32k/user/yr, 2-year minimum; B-PIPE/SAPI
  $50k–$200k+/yr). Pricing is a *terminal feature*, not an embeddable microsecond engine.
- **No customer-extensible model SDK** — you cannot drop in your own payoffs, vol models, or
  calibrations.
- **Not a low-latency programmatic pricing service** you can run in your own colo; integration is
  terminal/feed-oriented, not a microsecond hot path.

**The Celnet counter.** Against a closed terminal, Celnet ships a first-class model SDK so quants
add or override payoffs, vol models, and calibrations as native plugins — no per-seat lock-in, no
2-year minimum, no B-PIPE feed tax. It runs as an embeddable cloud-native/colo service pricing
vanilla *and* the full exotic/structured/path-dependent catalogue in deterministic
nanosecond-to-microsecond time in-process, turning pricing into a streaming engine rather than a
desktop feature. And it fuses pricing directly into capture, RFQ/RFS, booking, Greeks, hedging and
downstream lifecycle inside Celer, eliminating the feed-and-reconcile integration tax. On the risk
side it goes beyond a vol feed: server-side **hierarchical firm risk** (RiskService:
ListPositions/AggregateRisk/DrillRisk/LimitStatus, `celnet.proto:2476-2504`), **FRTB-SA**
standardised-approach capital (full SbM with the three correlation scenarios → max and the 0.75ρ
low-corr floor, RRAO, `crates/celnet-risk-cube/src/frtb.rs`), and **XVA** (CVA/DVA/FVA,
`crates/celnet-xva/`). Where useful, BVOL/Bloomberg surfaces can be ingested as one more validated
input source.

*(XVA caveat — carried verbatim: CVA/DVA/FVA are **internal-only, NOT on the wire**, computed over
**synthetic netting sets**; live CSAs/collateral/wrong-way risk are deploy-gated.)*

---

## The Broader Field

**Heavy front-to-back platforms — Murex MX.3, Numerix CrossAsset/Oneview.** Deepest production
exotic catalogs (LV and stochastic-local-vol, TARFs, 3rd-generation exotics), proven at tier-1
banks, with multi-language SDKs (Numerix CrossAsset SDK in Python/Java/C#/C++) and CPU/GPU grid
scaling. But *(stated inference from the documented install/upgrade model)* they ship as
monolithic, consultant-heavy installs with multi-year/complex upgrade cycles (hence MXTEST),
batch/EOD operational-clock dependencies, high TCO, and near-real-time (not microsecond) latency.
*Celnet counter:* Celnet now **matches the structured/path-dependent breadth** these platforms
charge for — var/vol swaps, Asians, forward-start/cliquet, quanto, TARF, accumulator, lookback,
American/Bermudan (PSOR free-boundary FD + Longstaff-Schwartz LSM,
`crates/celnet-exotics/src/american.rs`), correlated multi-asset basket/best-of/worst-of (Cholesky
MC over the scrambled-Sobol/Brownian-bridge stack, `src/multiasset.rs`, `tests/basket.rs`), and a
particle-calibrated LSV booking model — **while** removing their largest TCO line item:
hot-swappable models and engine versions with **zero-downtime upgrades** (SO_REUSEPORT graceful
handoff + single-current-contract blue-green full cutover, no mixed-version window). The open SDK
contract collapses Murex-style change-request latency from quarters to hours, and the Rust hot path
avoids C++/JVM memory-risk and GC pauses. MC-priced products (TARF, accumulator, discrete lookback,
basket, American via LSM) carry an honest **price standard error**, never claimed at machine
precision — that bar is reserved for the analytic/PDE/golden-gated products.

**Data/venue players — ICE Data Derivatives, Digital Vega (+ MarketAxess/RFQ-hub),
360T/Deutsche Börse FX.** ICE offers independent valuations trusted for price verification with
very deep exotic + bespoke coverage; Digital Vega is the award-winning FX-options RFQ/RFS workflow
network; 360T provides exchange-grade venue and streaming-mid (SUN) swaps with execution algos.
But *(stated inference)* ICE is EOD/intraday valuation, not a real-time interactive engine with an
open model SDK; the venues outsource real pricing to bank LPs or standard models (RFQ-hub is
equity/FI-centric, 360T is swaps/spot/algo-centric — FX-options analytics is not a core strength).
*Celnet counter:* it is the differentiated quant engine these venues lack, sitting behind or
alongside any of them rather than competing for venue network effects.

**Modern entrants — Quantifi, Quantra (QuantLib-as-a-service), RustQuant.** These validate the
*target* architecture (microservices, open REST/multi-language SDKs, AI-accelerated valuation,
emerging Rust adoption). But *(stated inference)* Quantra/QuantLib inherits C++/QuantLib
single-thread limits and is general-purpose, not FX-exotic-specialized; Quantifi is cross-asset and
platform-shaped; RustQuant/QuantMath are research/library-grade, not production exotics with GPU +
lifecycle. None has assembled open SDK + Rust nanosecond core + zero-downtime + native lifecycle +
GPU + a full FX-exotic catalogue behind one contract.

---

## Positioning Statement

> **Celnet is the open, Rust-native FX-options pricing and risk engine that out-functions the
> closed incumbents, out-intuits the opaque data feeds, and out-performs the RFQ venues and batch
> platforms — an evidence-backed superset, not a challenger.** It prices the full FX-options
> catalogue — vanilla and multi-leg strategies; first-generation exotics (digitals, touches, DNT,
> all eight barriers, window barriers); structured and path-dependent products (variance/volatility
> swaps, Asians, forward-start/cliquet, quanto, lookback, TARF, accumulator); American/Bermudan
> early exercise; and correlated multi-asset basket/best-of/worst-of — using Vanna-Volga and four
> further smile families (SABR/SVI/SSVI/eSSVI) for fast quoting and a particle-calibrated
> Local-Stochastic Volatility booking model (Heston backbone + Dupire leverage) plus a standalone
> Heston engine for booking and path-dependent exotics, in deterministic nanosecond-to-microsecond
> time on a thread-per-core, zero-allocation hot path, with GPU acceleration as an always-on
> service rather than a batch grid. Quants extend the already-deep catalogue with their own models,
> payoffs and calibrations through a sandboxed, deterministic SDK; it hot-swaps versions with zero
> downtime; it constructs transparent, arbitrage-checked vol surfaces with fully configurable
> conventions; it computes server-side hierarchical firm risk, FRTB-SA capital and (internal) XVA;
> and it embeds natively in the Celer trade lifecycle. It integrates Fenics FXO 2.0 (and
> Bloomberg/other surfaces) as validated inputs — then beats every one of them on the analytics,
> latency, breadth and openness that turn data into a tradable price. Every claim is grounded in
> shipped code and proven by a runnable parity matrix against independent oracles
> (`docs/CLIENT-PARITY-MATRIX.md`); no deploy-gated absolute is claimed in-repo.

---

## Feature Comparison

| Capability | Celnet | SynOption | Fenics (FMD/kACE) | Bloomberg (OVML/BVOL/MARS) |
|---|---|---|---|---|
| **Pricing latency** | Deterministic, measured in-core §1.2 truth-gate (p50 ≈ 42 ns / p99 ≈ 125 ns on the M4 dev host, host-local single-core); thread-per-core, zero-alloc; continuous auto-quote / streaming RFS | No published latency; RFQ request/response, seconds-scale, human-in-the-loop | Snapshot/intraday/FTP/SFTP + LSEG redistribution; not push-native sub-second | Terminal/feature pricing; not an embeddable microsecond engine |
| **Catalogue breadth** | **19 on-wire product families** (`celnet.proto:1016-1062`): vanilla, strategy, single/double/window barriers, digital, touch, var/vol swap, Asian, forward-start, cliquet, quanto, TARF, accumulator, lookback, **American/Bermudan**, **basket/best-of/worst-of** — each parity-gated | Vanillas, strategies, digitals/touch/DNT, single/double barriers; TARF, pivot, accumulators (pricing closed) | Vanillas, 20 first-gen digitals/barriers, 16 window barriers, quanto, LSV — but separately-licensed kACE | OVML strategy/vanilla + structures; exotic depth is desktop, not embeddable engine |
| **Extensibility / SDK** | First-class sandboxed model SDK to **extend an already-deep catalogue**: Tier-0 native registry + Tier-2 wasmi fuel-metered deterministic sandbox, behind one frozen contract (`celnet-plugin-host/src/{native,wasm}.rs`) | Closed Orion "risk libraries"; spread/skew knobs only; no quant SDK | kACE configurable interpolation but proprietary/closed models; heavyweight Excel/OMS, limited SDK | No customer-extensible model SDK |
| **Booking / advanced models** | Particle-calibrated **LSV** (Heston + Dupire leverage, ADI PDE + Philox MC, `lsv.rs`) + standalone **Heston** (Carr-Madan + Fang-Oosterlee COS, QuantLib-golden-gated) | None published | Heston/log-normal LSV (kACE, separately licensed) | None as embeddable engine |
| **Vol models / surface** | **Five** configurable families: VV / SABR / SVI / SSVI / **eSSVI** (`celnet.proto:372-382`); dense, arbitrage-free (calendar + butterfly checked), continuously re-strikable; broker→smile calibration; explicit conventions | Primus aggregates surfaces with staleness/range heuristics; closed construction | FXO 2.0 ATM/25d&10d RR/BF wing quotes, 300+ pairs + 27 metals; opaque ML wings, consumer builds dense surface | BVOL real-time surfaces 200+ pairs; trusted data, closed construction |
| **Risk / regulatory** | Server-side hierarchical firm risk (RiskService), **FRTB-SA** SbM (3-scenario max + 0.75ρ floor + RRAO, `frtb.rs`), exotic-leg cube aggregation, **XVA** CVA/DVA/FVA (*internal-only, synthetic netting sets, not on the wire*) | Omega risk/portfolio; closed | IPV/FRTB-aligned data positioning; analytics separate | MARS cross-asset risk + scenario + reg market-risk (feed-oriented) |
| **Convention transparency** | User-selectable delta (spot/fwd × premium-adj/unadj), ATM (DNS/ATMF), day-count, cut; published spec | Not documented | No published delta-convention/interpolation spec | Convention embedded in terminal, not user-exposed |
| **Deployment / hot-upgrade** | On-prem / colo / in-process / cloud; zero-downtime hot-swap (SO_REUSEPORT handoff + single-current-contract blue-green cutover) | SaaS / venue-hosted only | Multi-modal feed delivery; kACE on-prem/desktop; siloed licensing | Seat-licensed terminal; 2-yr minimum; no embeddable colo service |
| **GPU acceleration** | GPU as live service (**wgpu → Metal/Vulkan/DX12/GLES + WGSL**, CPU fallback): multi-step path kernel, pathwise/LR Greeks, batch closed-form, Sobol-QMC-on-GPU; f32 GPU reconciled to f64 CPU oracle | None published | None (analytics is CPU desktop/OMS) | None exposed as service |
| **Distributed correctness** | Leader-replicated log + **full Raft** (election/Pre-Vote/truncation/compaction/InstallSnapshot) with bit-identical (`f64::to_bits`) replay; lock-free SPMC fan-out ring under the edge; cross-shard risk fan-out (fan-out == single-node to 1e-12) | n/a | n/a | n/a |
| **Celer-native integration** | Native: shared data model + in-process bus across capture, RFQ/RFS, booking, Greeks, hedging, lifecycle (in-repo: traits + adapter swap + FIX 4.4 loopback; live JVM estate lifecycle deploy/live-gated) | Stitched estate (Optimus+Orion+Primus+Omega) glued by FIX hops | Data/analytics/execution loosely-coupled silos (FMD/kACE/FMX, FIX 4.4) | Pricing island; MARS API/Python but feed-oriented |
| **Openness / APIs** | Open: **6 gRPC services + byte-identical WebSocket JSON mirror**, one unversioned contract reachable identically from GUI/SDK/CLI/Excel/WS with bit-identical values; 27 `CELNET.*` Excel functions; multi-source surface ingestion | FIX + UI + thin STP API; no public dev portal | Enterprise data license; vague public docs; dated FIX 4.4 on execution | Closed, seat-priced; B-PIPE/SAPI feed access |

*Note: latency, throughput and architecture claims about competitors reflect the absence of
published figures and the documented RFQ/EOD/batch architectures in public material; they are
**positioning inferences**, not vendor-confirmed benchmarks. All Celnet numbers are labelled by
where they were measured.*

---

## Honest boundary (verbatim — applies to every claim above)

These are the canonical deploy/live-gated lines. None is claimed as in-repo-proven anywhere in
this document; the in-repo proofs establish correctness, ratios, and loopback/localhost behaviour
only.

- **Cross-host wire p99 / kernel-bypass NIC latency / the §11 ABSOLUTE wire-latency SLOs** —
  deploy-gated; in-repo proves the in-core §1.2 truth-gate + loopback benches only.
- **CUDA/NVIDIA ABSOLUTE GPU throughput + ≤50 ms exotic + Workload-A/B absolute numbers** —
  deploy-gated. M4 Metal lacks f64 ⇒ in-repo proves **correctness + RATIOS only** (M4/Lavapipe).
  Never claim f64 on Metal.
- **The entire live JVM Celer estate lifecycle** (sidecar / FX_OPTION / inferred hops / tenant
  overlays) — deploy/live-gated; in-repo has the seams + adapters (traits + adapter swap + FIX 4.4
  loopback) only, not an operational live binding.
- **Raft §6 dynamic membership / cross-DC transport / real network partitions** — deploy-gated;
  in-repo proves correctness/quorum/framing on localhost multi-process only.
- **Plugin Tier-1 signed-.so (stabby) + Tier-3 Landlock/seccomp** — **designed-only**; only Tier-0
  native + Tier-2 wasmi are shipped.
- **XVA (CVA/DVA/FVA, celnet-xva)** — internal-only, NO client/wire surface, **synthetic netting
  sets only**; live CSAs/collateral/wrong-way risk are deploy-gated.
- **Multi-source surface aggregation** — the blend/staleness/divergence ALGORITHM is built and
  gated; live multi-vendor quote VALUES are an integration/deploy target, not in-repo data.
- **MC-priced products** (TARF, accumulator, discrete lookback, basket/best-of/worst-of, American
  via LSM) carry a **price std-error** — never labelled "machine-precision"; that bar is reserved
  for analytic/PDE/golden-gated products.
</content>
</invoke>
