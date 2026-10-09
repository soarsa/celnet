<sub>[← Prev: Executive Summary](01-executive-summary.md) · [Index](../CELNET-CAPABILITIES.md) · [Next: System Architecture →](03-system-architecture.md) · [Showcase ↗](../celnet-capabilities.html)</sub>

# 2. Capability Map — What Celnet Does

Celnet is not a thin challenger closing gaps — it is a functionally complete, evidence-backed superset of what a derivatives desk stitches together today, proven by a runnable parity matrix against independent oracles, behind one unversioned contract reachable identically from five clients. This section is the scannable master inventory — what the platform does, grouped by capability cluster. Every later chapter expands a row here, and the recurring promise across all of them is the same: every value is bit-identical across the GUI, the Excel add-in, the Rust SDK, the admin CLI, and the WebSocket mirror, because they all consume the one contract.

![The Celnet capability landscape — pricing & analytics, engine & performance, GPU, risk, extensibility, edge & API, clients, and CelNet integration, all served from one contract.](../assets/celnet-capabilities/fig-12-capability-landscape.png)
*Figure 12 ([index](../CELNET-CAPABILITIES.md#figure-index)) — The capability landscape: a full FX-options catalogue (vanilla → first-generation exotics → structured & path-dependent → American/Bermudan → correlated basket → LSV booking), a nanosecond-class in-core engine, GPU acceleration, server-side hierarchical risk with FRTB-SA and internal XVA, an open quant SDK, and CelNet trade-lifecycle integration — unified by a single API-first contract.*

## 2.1 Pricing & analytics — the full catalogue

Celnet prices the complete FX-options book a real desk trades: vanilla through the full first-generation exotics catalogue, the structured and path-dependent products, American/Bermudan early exercise, correlated multi-asset baskets, and a particle-calibrated LSV booking model — **all on the one wire (18-product arms on the unified `Instrument`, `celnet.proto:1016-1062`), each parity-gated against an independent oracle and reachable from all five clients.** Vanilla pricing is Garman-Kohlhagen off the outright forward with separate domestic and foreign discount factors. The smile and surface engine carries five market-standard parametrisations and calibrates them from broker quotes behind arbitrage gates. The Open Quant SDK *extends* this already-deep catalogue with private-IP models; it is not the route to depth, because the depth ships in core.

| Capability | What it delivers |
|---|---|
| Vanilla pricing | Garman-Kohlhagen on the outright forward, separate domestic/foreign discounting (`celnet-vanilla`) |
| Full FX Greek set | Price + 13 Greeks in one pass — spot- & forward-delta, gamma, vega, theta, rho-domestic & rho-foreign, vanna, volga, charm, speed, zomma, color — finite-difference cross-validated (`celnet-vanilla`, `celnet-parity/tests/greeks.rs`) |
| Delta conventions | Spot / forward × unadjusted / premium-adjusted, with a branch-aware strike↔delta solver |
| At-the-money rules | At-the-money-forward and delta-neutral straddle, sign-correct per convention |
| Smile / surface models — **5 families** | Vanna-Volga, SABR, raw-SVI, SSVI, and **eSSVI (extended SSVI)** — `SMILE_MODEL_{MARKET_HEDGE, STOCHASTIC_VOL, PARAMETRIC, PARAMETRIC_SURFACE, EXTENDED_SURFACE}` (`celnet.proto:372-381`), with a wire-level selector to mark and recalibrate (`celnet-surface`, `celnet-parity/tests/{surface,essvi,essvi_hardening}.rs`) |
| Calibration | Broker-strangle → smile-strangle fixed-point fit; eSSVI by damped Gauss-Newton projected into the butterfly domain; Dupire local-vol surface |
| Arbitrage discipline | Butterfly density, calendar total-variance monotonicity, and vertical gates; arb-free term structure in total variance |
| **First-generation exotics** (analytic / PDE / MC / VV-overlay) | Digitals, one-touch / no-touch, double-no-touch / double-touch, single & double barriers (knock-in/knock-out), and **window barriers** — by analytic reflection, Crank-Nicolson PDE with a Rannacher start-up, Philox Monte-Carlo, and a survival-weighted Vanna-Volga overlay (`celnet-exotics/{digital,touch,barrier,pde}.rs`, `celnet-parity/tests/exotics.rs`) |
| **Structured & path-dependent** (closed-form & MC) | Variance & volatility swaps (log-contract static replication / Carr-Lee convexity adjustment), arithmetic Asians (Turnbull-Wakeman / Curran), forward-start & cliquet (Rubinstein), quanto, lookback — plus MC-priced TARFs and accumulators that **carry a price standard error** (`celnet-exotics/{var_swap,vol_swap,asian,forward_start,quanto,lookback,tarf,accumulator}.rs`, `celnet-parity/tests/{var_vol_swap,asian,forward_start,structured}.rs`) |
| **Early exercise — American / Bermudan** | Projected-SOR free-boundary finite difference (default) and Longstaff-Schwartz regression Monte-Carlo (LSM, carrying a std-error) (`celnet-exotics/american.rs`, `celnet.proto:1054-1057`) |
| **Correlated multi-asset** — basket / best-of / worst-of | Cholesky-correlated multi-asset GBM Monte-Carlo over the scrambled-Sobol / Brownian-bridge engine across N currency-pair legs, carrying a std-error (`celnet-exotics/multiasset.rs`, `celnet.proto:1058-1062`, `celnet-parity/tests/basket.rs`) |
| **LSV booking model + standalone Heston** | A local-stochastic-volatility booking engine — Heston backbone × Dupire leverage, particle-calibrated, ADI PDE — selected by `PRICING_MODEL_LOCAL_STOCH_VOL` (`celnet-proto:205-212`), plus a standalone Heston engine (Carr-Madan + Fang-Oosterlee COS) gated against frozen QuantLib (`celnet-exotics/{lsv,leverage,particle,adi,stochvol}.rs`, `celnet-heston`, `celnet-parity/tests/{lsv,heston}.rs`) |
| Sobol QMC | Scrambled Joe-Kuo Sobol + Brownian-bridge variance reduction (measured ≈38×/≈88× on exact targets), reused by the MC catalogue and the GPU path kernel (`celnet-qmc`, `celnet-parity/tests/{qmc,qmc_highdim}.rs`) |
| Market plumbing | Dual-calendar date engine and a per-(currency-pair, tenor) convention registry across a 19-pair universe (`celnet-calendar`, `celnet-conventions`, `celnet-parity/tests/pair_universe.rs`) |

> **Validation honesty (per product class).** Frozen QuantLib golden tables back **vanilla, both digital styles, all 8 barriers, touch, and Heston** to ~1e-10/last-bit; the rest are gated against closed-form limits, finite-difference oracles, and hand-pinned published constants. MC-priced products (TARF, accumulator, discrete lookback, basket/best-of/worst-of, American-via-LSM) carry a **price standard error** and are never labelled "machine precision" — that bar is reserved for the analytic/PDE/golden-gated set.

## 2.2 Engine, performance, GPU & durability

The core is so fast that network framing, not arithmetic, is the latency floor. An async edge meets pinned, allocation-free hot cores over wait-free rings; state is published lock-free and handed over without downtime; and the journal makes the book durable across a crash.

| Capability | What it delivers |
|---|---|
| Two-tier architecture | Async edge (gRPC / WebSocket mirror / FIX) joined to pinned, zero-allocation hot cores by wait-free single-producer/single-consumer rings |
| Lock-free state | Atomically-swapped market-state snapshot, single-writer seqlock top-of-book, cache-padded counters |
| Hot upgrades | Blue-green, zero-downtime state handoff |
| Durability | Checksummed, append-only journal with clean crash recovery and built compaction/checkpoint (`celnet-journal`) |
| Zero-cost observability | Tail-percentile latency histograms (HdrHistogram p50/p99/p99.9), coordinated-omission aware, over a bounded drop-on-full telemetry ring — the hot core stays log/lock/allocation-free |
| GPU backend & kernels | Pricing-backend abstraction over the cross-platform GPU stack (Metal / Vulkan / DX12) with a high-precision CPU oracle. Shipped kernels: a multi-step **path** kernel (`path.wgsl`/`path.rs`), **pathwise / likelihood-ratio Greeks** (`greeks.wgsl`/`pathwise.rs`), a **batch closed-form** vanilla kernel (`batch.wgsl`/`batch.rs`), and a **scenario** grid (`scenario.wgsl`/`scenario.rs`), with Sobol-QMC-on-GPU known-answer-tested (`celnet-parity/tests/{gpu_path,gpu_greeks}.rs`) |
| Deterministic RNG | Counter-based RNG bit-identical between CPU and GPU; GPU f32 results reconciled element-wise against the f64 oracle within a separately-derived round-off bound; CPU SIMD fallback |

> **GPU boundary (verbatim).** The M4 Metal dev host **lacks f64**, so the in-repo GPU path proves **correctness (GPU-f32 ≈ CPU-f64 ≈ golden) + RATIOS only** (M4 / Lavapipe). CUDA/NVIDIA **absolute** GPU throughput, the ≤50ms exotic figure, and the Workload-A/B absolute numbers are **deploy-gated** — never claimed in-repo, and f64-on-Metal is never claimed.

## 2.3 Risk

Risk has complementary zooms of one underlying fact cube: book-shaped scenario risk for the desk in front of the screen, a firm-wide OLAP position-fact cube for aggregation, limits, and entitlement — now including booked **exotic legs** — plus regulatory capital (FRTB-SA) and counterparty valuation adjustments (XVA, internal).

| Capability | What it delivers |
|---|---|
| Scenario grid | Two-axis shock grid by real repricing, with swappable axes |
| Bucketed vega | Vega per (tenor, delta) pillar |
| Higher-order book risk | Cross-gamma stencil and theta-roll over horizons |
| Marked-surface registry | Versioned official-vs-live surfaces (incl. eSSVI) with pinning; an unknown version is rejected, never silently re-derived live |
| Position-fact cube | Convention-canonicalized common-numeraire netting over eight dimensions (Trader / Book / Desk / CurrencyPair / Location / Entity / ValueDate / Session) |
| **Exotic-leg aggregation** | Exotic positions take a genuine seat in the cube with their **real exotic** sensitivities (closed-form digital Greeks, central-FD barrier Greeks) and full reprice under shocks — cross-shard fan-out reconciled fan-out==single-node (`celnet-risk-cube/exotic.rs`, `celnet-parity/tests/exotic_risk_cube.rs`) |
| Roll-up semantics | Additive incremental roll-up, plus non-additive per-node re-derivation (e.g. VaR / ES, curvature) |
| **Regulatory capital — FRTB-SA** | Sensitivities-based method: within-bucket K_b and cross-bucket aggregation, the three correlation scenarios → max with the 0.75ρ low-correlation floor (hand-pinned BCBS constants), curvature, RRAO, and an honest DRC = 0 for deliverable FX (`celnet-risk-cube/frtb.rs`, `celnet-parity/tests/frtb.rs`) |
| **Counterparty valuation adjustments — XVA** | CVA / DVA / FVA over EPE/ENE on **synthetic netting sets**, closed-form CVA parity (`celnet-xva/{cva,exposure,netting,survival}.rs`, `celnet-parity/tests/xva.rs`) — **internal-only, NOT on the wire** |
| Limit tree | Cascading board → entity → desk → book → trader limits with pre/post-trade checks |
| Entitlement | Server-side, entitlement-aware pre-aggregation pruning |
| Book ↔ Risk | Drill from an aggregated book row straight to a position's scenario risk — two views of the same cube |

> **XVA boundary (verbatim).** XVA is **internal-only, with no client/wire surface, on synthetic netting sets only** (a grep of `celnet.proto` for cva/xva returns nothing). Live CSAs, collateral, and wrong-way risk are **deploy-gated** — not in-repo data.

## 2.4 Extensibility & plugins

Desks run their own private-IP models inside the engine without forking Celnet — to *extend* an already-deep built-in catalogue, not to reach it. The Open Quant SDK is a frozen, runtime-agnostic contract; the host runs models across tiers, **two of which ship today** and two of which are designed to slot in behind the same frozen contract.

| Capability | Status | What it delivers |
|---|---|---|
| Open Quant SDK | **Shipped** | Frozen PricingModel / SmileModel (incl. eSSVI) / Calibration traits, a self-describing model registry, and a WIT interface mirror |
| Native tier (Tier-0) | **Shipped** | Native-speed in-engine models via the tier-blind host seam (`celnet-plugin-host/src/native.rs`) |
| WebAssembly sandbox (Tier-2) | **Shipped** | Compute-budgeted, capability-scoped wasmi guest — only the core math primitives (exp / ln / sqrt / norm_pdf / norm_cdf) cross into the guest; boundary NaN-canonicalization; strict marshalling ABI; 14 tests across the four WS-G gates (`celnet-plugin-host/src/wasm.rs`) |
| Signed shared-object tier (Tier-1) | **Designed (deploy-gated)** | A `stabby`-ABI signed-`.so` native tier, designed and seamed behind the same frozen contract; not shipped in-repo (`PLUGIN-HOST-ALT.md`) |
| OS-sandbox tier (Tier-3) | **Designed (deploy-gated)** | A Landlock / seccomp OS-isolation tier, designed and seamed behind the same frozen contract; not shipped in-repo (`PLUGIN-HOST-ALT.md`) |
| Determinism & safety | **Shipped** | Bit-identical replay; a per-call compute budget so an errant model fails typed, never hangs |
| Self-check | **Shipped** | Built-in butterfly no-arbitrage check |

> **Plugin tier boundary (verbatim).** Only **Tier-0 native + Tier-2 wasmi are shipped**. **Tier-1 signed-`.so` (stabby) and Tier-3 Landlock/seccomp are designed-only** — proven at deploy, never claimed as built in-repo.

## 2.5 Edge, API & wire

One clean, unversioned contract carries everything across **five gRPC services** plus a byte-identical WebSocket JSON mirror. The streaming session multiplexes many subscriptions over a single channel, and click-to-trade is bound to the streamed line with an unguessable token and last-look.

| Capability | What it delivers |
|---|---|
| Service families — **5 gRPC services** | `PricingService.Price`; `QuoteService.{RequestQuote, AcceptQuote, RejectQuote}`; the multiplex `StreamService.StreamSession`; `RiskService.{ListPositions, AggregateRisk, DrillRisk, LimitStatus}`; `SurfaceService.{GetSmile, MarkSurface, Scenario}` (`celnet.proto:2447-2504`) |
| Unified instrument | A single `Instrument` type with **18-product arms** and surface-version pinning; each MC product carries `price_std_error` |
| Streaming | One bidirectional channel, many subscriptions: Subscribe → Snapshot → Update(seq) → Heartbeat, gap → Resync, in-place Modify re-baseline |
| Click-to-trade | Unguessable keyed-MAC tradable token (sell-at-bid / buy-at-offer) bound to the line, with last-look validity and idempotent execution — stale / forged / already-consumed are rejected |
| WebSocket mirror | Byte-identical WebSocket JSON mirror of the **entire** contract — all five services (`celnet-server/src/lib.rs:31,177,359`) |
| Clients on the wire | Typed Rust client SDK (reconnect-liveness, typed errors) and an admin CLI |
| FIX | Real FIX 4.4 engine (acceptor + initiator) with an FX-options dialect |
| Market-data layer | Vendor-feed normalisation / blend / divergence and a conflating egress governor (token-bucket + counted drops). *The blend/staleness/divergence algorithm is built and gated; live multi-vendor quote values are an integration/deploy target.* |
| Fleet sharding | Intra-fleet highest-random-weight partition map for horizontal scale-out |
| Series & attribution | A market-series feed (ATM vol / SPOT / RR / BF / FORWARD observables) multiplexed on the same StreamSession, a smile-model selector (5 families), and a book/owner attribution dimension |

**API-first parity.** Every capability lives in the one contract. The GUI, the Rust SDK, the CLI, and the Excel add-in all consume that same contract — the front-end has no privileged path — so a value is bit-identical across every surface, proven continuously by `CLIENT-PARITY-MATRIX.md` (all 18-products × the service families reachable from all five surfaces, with honest exceptions, e.g. basket Greeks deliberately zeroed).

## 2.6 Clients — GUI, Excel, SDK, CLI

| Client | What it delivers |
|---|---|
| GUI (React + WebGPU) | Live-WebSocket-by-default, single-window five-workspace shell — Ticket / Stream / Surface / Risk / Book (Cmd-1..5), with a Firm/desk/book scope breadcrumb, a pair navigator, a Cmd-K command palette, a `?` keyboard-shortcuts overlay, light/dark, and trend modes. The Ticket prices the full exotic catalogue (incl. American/Bermudan and basket/best-of/worst-of); the Surface workspace carries **5 model chips incl. eSSVI** |
| Excel add-in (Office.js) | **27 `CELNET.*` functions** — pricing (PRICE, GREEKS), exotics (BARRIER, WINDOWBARRIER, DIGITAL, TOUCH, VARSWAP, VOLSWAP, ASIAN, FORWARDSTART, CLIQUET, QUANTO, TARF, ACCUMULATOR, LOOKBACK, AMERICAN, BASKET), surface (SURFACE, MARKSURFACE, MARK), stream (RFQ, SUBSCRIBE, SERIES), and server-side risk (RISK, POSITIONS, LIMITS, STATUS) — no pricing in the cell; every number is the server's value, bit-identical to the GUI, with per-cell convention/surface-version transparency, MC std-error disclosure, and typed #CELNET_* errors |
| Rust SDK | Typed client with reconnect-liveness and typed errors; an InstrumentSpec builder for all 18-products and the LSV pricing-model directive |
| Admin CLI | Operational control over the same contract (price / surface / exotic / basket / convention / risk / stream) |

![The live multiplex RFS blotter — click-to-trade with last-look and a trend selector.](../assets/celnet-capabilities/shot-01-stream-blotter.png)
*Screenshot 1 — The Stream workspace: live multiplex RFS blotter with click-to-trade and the trend selector.*

![The branded Excel hero — CELNET formulas spilling RFQ, the full Greek vector, and a surface smile alongside live series cells.](../assets/celnet-capabilities/shot-10-excel-grid-branded.png)
*Screenshot 10 — The Excel add-in: a CELNET.RFQ formula bar, RFQ and full GREEKS spills, a SURFACE smile spill, and live SERIES cells — every value the server's, bit-identical to the GUI.*

## 2.7 Distributed correctness & scale-out

Horizontal scale-out is a built substrate, not a promise: a Raft-replicated event log, a lock-free broadcast ring under the edge, and a cross-fleet risk fan-out — each proven on localhost multi-process to be correct and bit-identical to the single-node result.

| Capability | What it delivers |
|---|---|
| Raft-replicated log | Leader election + Pre-Vote, conflicting-tail truncation, snapshot compaction, and **InstallSnapshot** reseed, with bit-identical (`f64::to_bits`) replay and hot-standby takeover (`celnet-replog`, `celnet-parity/tests/{raft_election,raft_compaction,raft_snapshot}.rs`) |
| SPMC broadcast ring | A lock-free single-producer/multi-consumer fan-out ring **wired under the edge**, fanning each pair's tick to all subscribers, zero-alloc, with exact skip-accounting conflation (`celnet-fanout`, `celnet-server/src/services/pricefanout.rs`) |
| Cross-fleet risk fan-out | HRW partitioning with additive merge / non-additive re-gather, reconciled fan-out==single-node to 1e-12 (`celnet-risk-fleet`) |

> **Scale-out boundary (verbatim).** Localhost multi-process proves correctness / quorum / framing only. **Cross-host wire p99, kernel-bypass NIC latency, cross-DC transport, real network partitions, Raft §6 dynamic membership, the §11 absolute wire-latency SLOs, and the physical cross-node risk transport are deploy-gated** — never claimed as in-repo-proven.

## 2.8 CelNet integration

Celnet is the FX-options pricing system-of-record inside the CelNet trade lifecycle. It injects option price, Greeks, and surface into the price path and option risk into the risk and position path, and runs in three deployment modes reached by swapping adapters on the seam traits — never by a rewrite.

| Capability | What it delivers |
|---|---|
| Price path | Injects FX-option price / Greeks / surface into venues → FIX-in → market-data → market-merchant → distributor → WebSocket → CelNet Trader |
| Risk / position path | Injects option risk into order-routing → risk → destination → clearing → position-manager |
| Deployment modes | Standalone, Hybrid, and **CelnetIntegrated (designed estate-native binding)** — reversible adapter swaps on market-data-source / price-sink / order-and-exec seams |
| Adaptability | The same market-data seam ingests external products and feeds — for example Fenics, Bloomberg, Refinitiv, EBS — as adapter targets |

> **CelNet boundary (verbatim).** The in-repo work is the **seams + adapters only** (trait swap + FIX 4.4 loopback). The **entire live JVM CelNet estate lifecycle** — distributor sidecar handshake, mailbox calibration, live quote-feed entitlement, tenant overlays — is **deploy/live-gated, proven at deploy against the running estate**, not exercised in-repo.

## 2.9 Positioning

Celnet's parity claims are executable: `CLIENT-PARITY-MATRIX.md` plus ~26 `celnet-parity` rows render each capability as a gated test against an independent oracle, so "meets or beats" is continuously proven rather than asserted. Comparisons use vendor-neutral archetypes — a closed terminal, a front-to-back platform, a data venue, a modern library. The platform now matches the deep-catalogue front-to-back incumbents on structured / path-dependent / American / multi-asset breadth **while retaining the edges they structurally lack**: an open quant SDK, one clean unversioned contract with bit-identical values across five surfaces, a pinned zero-alloc nanosecond hot core, and server-side hierarchical risk — supply-chain-clean and cross-platform deterministic.

| Capability | What it delivers |
|---|---|
| Executable parity | ~26 capability rows gated against independent oracles + frozen QuantLib golden tables, proven continuously |
| Supply-chain clean | Open-source, permissive-licence dependencies only |
| Determinism | Cross-platform, bit-identical results (mutation + fuzz estate) |
| Honesty as a differentiator | Every figure is labelled (in-core / M4 / loopback); deploy-gated absolutes are never claimed in-repo — a reviewer doing diligence finds proof, not marketing fiction |

**See also:** each cluster above is expanded in its own chapter — [§3 System Architecture](03-system-architecture.md), [§4 Quant Coverage](04-quant-coverage.md), [§5 Extensibility](05-extensibility-plugins.md), [§6 Risk Management](06-risk-management.md), [§7 Performance & Latency](07-performance-latency.md), [§8 Scalability & Scale-Out](08-scalability-scaleout.md), [§9 API & Client Parity](09-api-contract-parity.md) and [§12 CelNet Integration](12-celnet-integration.md).

---
<sub>[← Prev: Executive Summary](01-executive-summary.md) · [Index](../CELNET-CAPABILITIES.md) · [Next: System Architecture →](03-system-architecture.md) · [Showcase ↗](../celnet-capabilities.html)</sub>
