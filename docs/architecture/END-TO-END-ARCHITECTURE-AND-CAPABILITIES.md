# Celnet — End-to-End Architecture & Capabilities Specification

> **The Sovereign Institutional Derivatives & Fixed Income Platform**  
> **Unified Cross-Asset Pricing, Real-Time Risk, Algorithmic Hedging, and Ultra-Low Latency Execution Engine**  
> *Specification Version: 2026.4 — Post-P0/P1 Optimization Milestone*

---

## 1. Executive Overview & Strategic Thesis

**Celnet** is a mission-critical, enterprise-grade quantitative pricing, real-time risk aggregation, and execution platform built for tier-1 investment banks, institutional market makers, and liquidity providers. Spanning **57 modular Rust crates** configured in a strictly one-way acyclic dependency graph, Celnet delivers a unified financial computing architecture across **Foreign Exchange (FX) Options, Fixed Income & Rates, Credit, Cash Equities, Commodities, and Digital Assets**.

```
                           +-------------------------------------------------------------+
                           |                     CLIENT & ACCESS SURFACES                |
                           |  [React/WebGPU GUI]  [Excel 27-Func]  [Rust SDK]  [CLI/FIX] |
                           +------------------------------+------------------------------+
                                                          |
                                      gRPC / WebSocket / Shared Memory / FIX
                                                          |
                           +------------------------------v------------------------------+
                           |               TIER 4: ASYNC API GATEWAY & EDGE              |
                           |   AuthService | ValuationService | TradeService | RiskService|
                           +------------------------------+------------------------------+
                                                          |
                                       Wait-Free SPSC Queues / POSIX /dev/shm
                                                          |
                           +------------------------------v------------------------------+
                           |              TIER 1: PINNED HOT NUMERICAL ENGINE            |
                           |   celnet-core | celnet-vanilla | celnet-rates | celnet-bond |
                           |   13.63M opt/s/core · 42ns p50 · 84ns p99 · #![forbid(unsafe)]  |
                           +------------------------------+------------------------------+
                                                          |
                                      Zero-Copy SBE Flyweights / Aeron UDP Ring
                                                          |
                           +------------------------------v------------------------------+
                           |            TIER 2 & 3: DURABILITY & FLEET SCALE-OUT         |
                           |   Raft Quorum Log | SPMC Fanout | Cross-Fleet Cube Partition |
                           +-------------------------------------------------------------+
```

### The Core Architectural Pillars

1. **Deterministic Mechanical Sympathy**:
   - The numerical hot path operates under `#![forbid(unsafe_code)]` with zero runtime heap allocations.
   - Eliminates all software transcendental fallbacks in favor of IEEE 754-2008 compliant native hardware instructions ([`fsqrt`](../../crates/celnet-core/src/math.rs)).
   - Eliminates redundant transcendental evaluations via branchless active-arm Φ evaluation in [`gbsm_carry_greeks`](../../crates/celnet-core/src/carry.rs).
   - Factors cash-flow discounting into closed-form Horner power recurrences ($D_k = D_{k-1} \cdot v$) in [`CashflowSchedule`](../../crates/celnet-bond/src/schedule.rs).
   - Precomputes invariant curve slopes in [`Curve::Node`](../../crates/celnet-rates/src/curve.rs), turning 15-cycle `fdiv` operations into 1-cycle Fused Multiply-Add (FMA) instructions.
2. **One Unversioned Contract (ADR-0007)**:
   - Exactly **one canonical wire schema** ([`celnet.proto`](../../crates/celnet-proto/proto/celnet.proto)) governs all 7 distribution surfaces: gRPC, WebSocket JSON mirror, Binary SBE, POSIX Shared Memory, FIX 4.4/5.0SP2, Excel Add-in, and the Trader GUI.
   - Every surface produces **bit-identical numerical results** (`f64::to_bits` parity) across all supported asset classes.
3. **Unified Cost-of-Carry Kernel (ADR-0012)**:
   - A single generalized Black-Scholes-Merton (gBSM) forward-space kernel ([`celnet_core::carry`](../../crates/celnet-core/src/carry.rs)) prices FX (Garman-Kohlhagen), Equities (Black-Scholes with dividends), Commodities (Black-76), and Crypto (Linear & Inverse Coin-margined), collapsing maintenance surface area and eliminating model divergence.
4. **Zero-Allocation Inter-Process Communication (IPC)**:
   - Sub-15 nanosecond process-to-process communication via POSIX `/dev/shm` circular rings with Simple Binary Encoding (SBE) flyweights ([`celnet-shm`](../../crates/celnet-shm)).
5. **Real-Time Distributed Durability & Risk Scale-Out**:
   - Resilient Raft consensus replication ([`celnet-replog`](../../crates/celnet-replog)) providing bit-identical state recovery, uncommitted tail truncation, and zero-downtime blue/green hot upgrades.
   - Cross-fleet risk aggregation ([`celnet-risk-fleet`](../../crates/celnet-risk-fleet)) partitioning books across multiple nodes where federated query latency is $\max(\text{node}_i)$ rather than $\sum(\text{node}_i)$, matching single-node calculations to $10^{-12}$.

---

## 2. Quantitative & Financial Engineering Capabilities

Celnet provides native, institutional-grade analytics across six primary asset classes:

### 2.1 FX Options & Volatility Surfaces

* **Vanilla Pricing & Greeks ([`crates/celnet-vanilla`](../../crates/celnet-vanilla))**:
  - Garman-Kohlhagen (1983) and forward-space gBSM kernels.
  - Full 13-Greek analytic risk strip calculated in a single unified pass: Spot Delta, Forward Delta, Gamma, Vega, Theta, Domestic Rho, Foreign Rho, Vanna, Volga (Vomma), Charm (Delta decay), Speed, Zomma, and Color.
  - Four market-standard quoted delta conventions: Spot Unadjusted, Forward Unadjusted, Spot Premium-Adjusted, and Forward Premium-Adjusted.
  - At-The-Money (ATM) conventions: ATM Forward ($K = F$) and Delta-Neutral Straddle (DNS).
  - Adjoint Algorithmic Differentiation (AAD) engine ([`crates/celnet-vanilla/src/adjoint.rs`](../../crates/celnet-vanilla/src/adjoint.rs)) computing exact machine-precision sensitivities with a single reverse pass.
* **Exotics & Structured Catalogue (24 Discrete Instrument Arms)**:
  - **First-Generation Exotics ([`crates/celnet-exotics`](../../crates/celnet-exotics))**: Regular Barriers (Up-and-In, Up-and-Out, Down-and-In, Down-and-Out), Window Barriers (discrete monitoring intervals), Double Barriers, Digitals (European cash-or-nothing), Touches (One-Touch, No-Touch, Double-One-Touch, Double-No-Touch).
  - **Path-Dependent & Averaging Products**: Asian options priced via Curran (1994) geometric conditioning and Turnbull-Wakeman (1991) moment matching; Lookback options (floating strike, fixed strike); Forward-Start Options; Cliquets (locally capped/floored ratchet structures); Quanto options.
  - **Structured Flow Products**: Target Accrual Redemption Forwards (TARF) with gain caps; Accumulators/Decumulators with knockout barriers; Variance Swaps and Volatility Swaps with continuous fair-variance replication.
  - **Early Exercise & Multi-Asset**: American and Bermudan options priced via Projected Successive Over-Relaxation (PSOR) free-boundary PDE and Longstaff-Schwartz Least Squares Monte Carlo (LSM); Multi-Asset Cholesky-correlated Baskets (Best-of, Worst-of, Rainbow).
* **Arbitrage-Free Volatility Smiles & Surface Models ([`crates/celnet-surface`](../../crates/celnet-surface))**:
  - Five switchable parametric smile families: **Vanna-Volga (VV)**, **SABR** (Hagan 2002), **SVI** (Gatheral 2004 raw and natural parameterizations), **SSVI** (Surface SVI), and **Extended SSVI (eSSVI)**.
  - Continuous **Dupire Local Volatility** surface derivation: $\sigma_L^2(K, T) = \frac{\partial C / \partial T + r K \partial C / \partial K}{\frac{1}{2} K^2 \partial^2 C / \partial K^2}$.
  - **Local Stochastic Volatility (LSV)** hybrid booking engine reconciling market smiles with forward skew dynamics.
  - Strict mathematical no-arbitrage enforcement: Butterfly arbitrage gate (Breeden-Litzenberger density $g(K) \ge 0$), Calendar spread arbitrage gate ($\partial w / \partial T \ge 0$), and Vertical monotonicity gate.

### 2.2 Fixed Income, Rates & Credit

* **Bond Valuation & Schedule Analytics ([`crates/celnet-bond`](../../crates/celnet-bond))**:
  - Government, sovereign, supranational, and corporate cash bonds with regular, irregular, short/long stub coupons.
  - Day-count conventions: Thirty360 (Bond Basis, ISDA, European), Actual365Fixed, Actual360, ActualActual (ICMA/ISDA).
  - Yield-to-Maturity (YTM) solving via safeguarded hybrid Newton-Raphson / Bisection on Horner recurrence schedules.
  - Analytic bond sensitivities: Macaulay Duration, Modified Duration, DV01 ($1\text{ bp}$ price shift), and Analytic Convexity.
  - Spread analytics: G-spread (interpolated government benchmark), Z-spread (zero-volatility curve spread), and Asset Swap Spread (ASW par-par).
* **Curve Bootstrapping & Multi-Curve Term Structures ([`crates/celnet-rates`](../../crates/celnet-rates))**:
  - Two distinct interpolation schemes: **Log-Linear on Log-DF** (piecewise-constant forwards, QuantLib golden match) and **Monotone-Convex on Forwards** (continuous, monotonicity-preserving instantaneous forward rates).
  - Generalized bootstrapping engine accepting heterogeneous calibration instruments: Short-term cash deposits, Forward Rate Agreements (FRAs), Interest Rate Futures strips with convexity bias debiasing, and Overnight Index Swaps (OIS, e.g. USD SOFR, EUR ESTR, GBP SONIA).
  - Post-LIBOR dual-curve separation: Pure OIS discounting decoupled from tenor projection curves (SOFR 1M/3M, Euribor 3M/6M).
* **Bond Corporate Actions ([`crates/celnet-corpactions`](../../crates/celnet-corpactions), [`crates/celnet-refstore`](../../crates/celnet-refstore))**:
  - Full ISO 15022 / ISO 20022 corporate actions lifecycle: `REDM` (Final Redemption), `INTR` (Interest/Coupon), `MCAL` (Full Call), `PCAL` (Partial Call), `PRED` (Partial Redemption), `DRAW` (Lottery Drawing), `BPUT` (Bondholder Put).
  - Dynamic instrument schedule re-derivation and post-event position booking.
* **Credit & Valuation Adjustments (XVA) ([`crates/celnet-xva`](../../crates/celnet-xva))**:
  - Synthetic netting set exposure simulation.
  - Unilateral Credit Valuation Adjustment (CVA), Debit Valuation Adjustment (DVA), and Funding Valuation Adjustment (FVA) calculated over continuous counterparty survival probability hazard curves.

### 2.3 Regulatory Capital & Enterprise Risk

* **FRTB Standardised Approach (GIRR & FX) ([`crates/celnet-rates-risk`](../../crates/celnet-rates-risk))**:
  - General Interest Rate Risk (GIRR) sensitivities mapped across BCBS 10 regulatory vertex tenors (0.25Y, 0.5Y, 1Y, 2Y, 3Y, 5Y, 10Y, 15Y, 20Y, 30Y).
  - Correlation matrices and prescribed cross-bucket aggregation formulas (Low, Medium, High regulatory correlation scenarios).
* **Firm-Wide OLAP Position-Fact Cube ([`crates/celnet-risk-cube`](../../crates/celnet-risk-cube))**:
  - Multi-dimensional aggregation over 8 organizational axes: Book, Desk, Entity, Counterparty, Currency Pair, Tenor Bucket, Strategy, and Asset Class.
  - Joint Non-Additive Tail Risk: Joint bump-and-revalue historical VaR and Expected Shortfall (ES) accounting for cross-asset diversification between options and linear rates.
  - Cascading limit monitoring with real-time Red/Amber/Green (RAG) threshold warnings and hard-breach execution blocks ([`crates/celnet-limits`](../../crates/celnet-limits)).

---

## 3. Detailed Latencies & Performance Benchmarks

All metrics below represent **empirically measured host performance** executed on an Apple Silicon M4 64-bit architecture (10.0M timed samples, 2.0M warmup discarded), release profile, native instructions enabled:

```
+----------------------------------------------------------------------------------------------------+
|                                    CELNET PERFORMANCE SCORECARD                                    |
+------------------------------------+-----------------------+-----------------------+---------------+
| Metric / Workload                  | Baseline (Pre-Opt)    | Optimized (Post-Opt)  | Delta / Gain  |
+------------------------------------+-----------------------+-----------------------+---------------+
| In-Core Sustained Throughput       | 10.88 M opt/s/core    | 13.63 M opt/s/core    | +25.3%        |
| In-Core Median Latency (p50)       | 41.9 ns               | 42.0 ns (0.042 us)    | ALU Bound     |
| In-Core Tail Latency (p99)         | 125.0 ns              | 84.0 ns (0.084 us)    | -32.8%        |
| In-Core Extreme Tail (p99.9)       | 250.0 ns              | 125.0 ns (0.125 us)   | -50.0%        |
| Maximum Latency Ceiling            | 18.2 us               | 13.1 us               | -28.0%        |
| Divan: Price Only (Vanilla)        | 22.85 ns              | 22.53 ns (22.04 ns)   | -1.4%         |
| Divan: Price + Full 13 Greeks (C)  | 42.71 ns              | 36.53 ns (35.88 ns)   | -14.5%        |
| Divan: Price + Full 13 Greeks (P)  | 42.38 ns              | 37.18 ns (36.53 ns)   | -12.3%        |
| Shared Memory IPC (SBE /dev/shm)   | 588.0 us (gRPC loop)  | 14.46 ns (direct SHM) | 40,600x       |
| Shared Memory Drain Rate           | 25.0 M msg/s          | 88.4 M msg/s (batch)  | +253.6%       |
| Surface Calibration: Parametric    | 6.8 us                | 6.3 us (budget: 150us)| 24x margin    |
| Surface Calibration: MarketHedge   | 18.1 us               | 16.0 us (budget: 150us| 9.4x margin   |
+------------------------------------+-----------------------+-----------------------+---------------+
```

### §1.2 Absolute In-Core Commitment Budget Verification

Under the Celnet Verification Canon, core pricing is subject to rigid deterministic time gates:

| Performance Gate | Measured Result | Committed Budget | Margin vs Budget | Verdict |
|---|---|---|---|---|
| **Median Latency (p50)** | **0.042 µs** (42 ns) | $\le 2.000\ \mu\text{s}$ | **48x margin** | **PASSED** |
| **Tail Latency (p99)** | **0.084 µs** (84 ns) | $\le 10.000\ \mu\text{s}$ | **119x margin** | **PASSED** |
| **Deep Tail Latency (p99.9)** | **0.125 µs** (125 ns) | $\le 25.000\ \mu\text{s}$ | **200x margin** | **PASSED** |

---

## 4. End-to-End System Architecture

Celnet is partitioned into five concentric performance tiers:

```
 [Tier 4: Gateways & Clients]
   ├── gRPC (HTTP/2 + Protobuf)
   ├── WebSocket JSON Stream (Multiplexed StreamSession)
   ├── FIX Acceptor (FIX 4.4 / 5.0SP2 Engine)
   ├── Excel Add-in (27 CELNET.* C-API Functions)
   └── React/WebGPU GUI (Five Consolidated Studios)
         │
         ▼ (Wait-Free SPSC Memory Rings / SBE /dev/shm)
 [Tier 1: Pinned Hot Numerical Core]
   ├── celnet-core (Deterministic Math Primitives, Hardware fsqrt)
   ├── celnet-vanilla (Garman-Kohlhagen, Forward-Space gBSM, 13 Analytic Greeks, AAD)
   ├── celnet-surface (VV, SABR, SVI, SSVI, eSSVI, Dupire, LSV, No-Arbitrage Gates)
   ├── celnet-exotics (24-Product Analytic & QMC Pricing Engines)
   ├── celnet-rates (Multi-Curve Bootstrapping, OIS, Swaps, FRAs, STIR Strips)
   └── celnet-bond (Cashflow Horner Recurrences, YTM Solvers, Spread Analytics)
         │
         ▼ (Zero-Copy Ring Buffer & SBE Encodings)
 [Tier 2: Durability & Messaging Edge]
   ├── celnet-fanout (Single-Writer Multi-Reader SPMC Atomic Ring Buffer)
   ├── celnet-shm (14.46 ns POSIX Shared Memory Transport)
   └── celnet-sbe (Simple Binary Encoding Direct Buffer Codecs & Aeron UDP Multicast)
         │
         ▼ (Raft Quorum Log & Fleet Partitions)
 [Tier 3: Distributed State & Scale-Out Fleet]
   ├── celnet-replog (Raft Consensus, Leader Election, Snapshot Compaction)
   ├── celnet-risk-fleet (Consistent Hashing Book Partitioning, Zero-Loss Fanout)
   └── celnet-risk-cube (Real-Time In-Memory Multi-Dimensional OLAP Aggregator)
         │
         ▼ (Capability Guardrails & Sandboxing)
 [Tier 0 & 2: Quant SDK & Extensibility]
   ├── Tier-0 Native C ABI Plugins (Zero-Overhead Static Linkage)
   └── Tier-2 WebAssembly Plugins (wasmi Sandboxed Private-IP Model Host)
```

---

## 5. Comprehensive API Catalog & Wire Contracts

Every RPC and message in Celnet is defined in [`crates/celnet-proto/proto/celnet.proto`](../../crates/celnet-proto/proto/celnet.proto) (9,898 lines of proto3 definitions) and mapped 1:1 to equivalent JSON WebSocket frames.

### 5.1 Pricing & Valuation APIs

#### `ValuationService`
Universal, stateless pricing calculation across all asset classes. Caller supplies instrument specification and market parameters explicitly.

* **`Calculate(ValuationRequest) -> ValuationResponse`**
  - *Inputs*: [`ValuationRequest`](../../crates/celnet-proto/proto/celnet.proto) carrying `Instrument` (any of the 24 product arms) and `MarketContext` (spot, discount curves, volatility surface, dividend/borrow yields).
  - *Outputs*: Net Present Value (PV), full 13-Greek strip, model calibration metadata, and Monte Carlo standard errors.

#### `PricingService`
Dedicated one-shot calculation endpoints for options, linear rates, and credit risk.

* **`Price(PriceRequest) -> PriceResponse`**: High-speed vanilla/exotic option calculation returning premium and full Greek risk.
* **`PriceRates(RatesPriceRequest) -> RatesPriceResponse`**: Linear fixed-income calculation over calibrated curve sets, yielding PV, PV01, parallel DV01, and 10-bucket key-rate DV01 ladders.
* **`PriceXva(PriceXvaRequest) -> PriceXvaResponse`**: CVA/DVA/FVA computation across synthetic netting sets against survival hazard curves.

### 5.2 Trading, RFQ & Order Execution APIs

#### `TradeService` & `QuoteService`
End-to-end Request-For-Quote (RFQ) lifecycle for bilateral, multi-dealer, and click-to-trade workflows.

* **`RequestQuote(QuoteRequest) -> Quote`**: Generates an executable two-way (bid/offer) quote bound to an unguessable, cryptographically signed line token with microsecond expiration. Idempotent on `idempotency_key`.
* **`RequestMultiDealerQuote(QuoteRequest) -> MultiDealerQuote`**: Fans out RFQ-to-many across enabled Liquidity Providers (LPs), aggregating and pre-ranking dealer ladders with best-bid/best-offer execution routing.
* **`RequestRatesQuote(RatesQuoteRequest) -> RatesQuote`**: Fixed income request-for-quote on bonds, OIS swaps, and FRAs returning par rate / clean price quotes and DV01 risk.
* **`AcceptQuote(QuoteAccept) -> Execution`**: Accepts a quote token, verifies last-look validity and credit limits, commits the transaction to the Raft log, and emits a confirmed `Execution`.
* **`RejectQuote(QuoteReject) -> RejectAck`**: Explicitly declines a quote, immediately releasing dealer reserve capital.
* **`ListDeals(ListDealsRequest) -> ListDealsResponse`**: Queries executed transactions from the trade repository blotter.

### 5.3 Streaming & Real-Time Market Data APIs

#### `StreamService`
Multiplexed bidirectional request-for-stream (RFS) channel ([`ClientStreamMessage`](../../crates/celnet-proto/proto/celnet.proto) $\leftrightarrow$ [`ServerStreamMessage`](../../crates/celnet-proto/proto/celnet.proto)). A single connection multiplexes hundreds of concurrent live subscriptions:

* **Subscription Lifecycle**:
  - `Subscribe`: Initiates streaming for an instrument structure and convention set.
  - `Snapshot`: Emits initial order book state, active Greeks, and tradable execution tokens.
  - `Update`: Low-latency delta frame emitting tick updates, implied vol movements, and price shifts.
  - `Modify`: Mutates subscription parameters in-place (e.g. roll tenor, adjust strike) without resubscribing.
  - `Resync`: Gap-recovery protocol replaying missed sequence numbers.
  - `Execute`: Click-to-trade execution directly on a streamed line token.
  - `Heartbeat`: Off-hot-path telemetry carrying server health, conflation drops, and p50/p99/p99.9 latency statistics.

### 5.4 Risk & Position Management APIs

#### `RiskService`
Firm-wide hierarchical risk management computed server-side across the position cube:

* **`ListPositions(ListPositionsRequest) -> ListPositionsResponse`**: Returns open positions with full attribution metadata, filtered by user entitlements.
* **`AggregateRisk(AggregateRiskRequest) -> AggregateRiskResponse`**: Rolls up risk facts across chosen organizational dimensions, performing common-numeraire conversion and non-additive VaR/ES calculations.
* **`DrillRisk(DrillRiskRequest) -> DrillRiskResponse`**: Drills into an aggregation node to reveal contributing child sub-nodes and position tickets.
* **`LimitStatus(LimitStatusRequest) -> LimitStatusResponse`**: Returns hierarchical limit utilization, warnings, and hard-breach flags.
* **`AggregateRatesRisk(AggregateRatesRiskRequest) -> AggregateRatesRiskResponse`**: Shards rates books across the cluster and rolls up per-currency net PV, PV01, and key-rate ladders.
* **`BookRatesPosition(BookRatesPositionRequest) -> BookRatesPositionResponse`**: Authoritative position capture into the rates book.
* **`ListRatesPositions(ListRatesPositionsRequest) -> ListRatesPositionsResponse`**: Queries the rates position store.
* **`CombinedTailRisk(CombinedTailRiskRequest) -> CombinedTailRiskResponse`**: Unified cross-asset Monte Carlo re-valuation over aligned FX and rates shock scenarios, producing joint multi-asset VaR and Expected Shortfall.

### 5.5 Volatility Surfaces & Term Structure APIs

#### `SurfaceService`
Market data marking, smile calibration, and scenario stress testing:

* **`GetSmile(GetSmileRequest) -> Smile`**: Returns calibrated smile curve on delta/strike axes for a specific pair and tenor.
* **`MarkSurface(MarkSurfaceRequest) -> MarkSurfaceResponse`**: Recalibrates a pair's surface from broker quotes (ATM, Risk Reversals, Butterflies) and publishes an immutable, versioned surface.
* **`Scenario(ScenarioRequest) -> ScenarioResponse`**: Reprices an instrument across a 2D/3D spot, vol, and rate shock grid.
* **`GetCurve(GetCurveRequest) -> GetCurveResponse`**: Returns discount factors and zero rates for a bootstrapped interest rate curve.
* **`MarkCurve(MarkCurveRequest) -> MarkCurveResponse`**: Calibrates and persists a versioned discount curve from deposit, FRA, futures, and swap quotes.

### 5.6 Enterprise Governance, Desks & Administration APIs

* **`AuthService`**: Session management (Argon2id hashing), granular user/role administration, desk hierarchies, legal entities, netting books, aggregated book configurations, and firm-wide pricing kill-switches.
* **`FixAdminService`**: Inbound/outbound FIX connection administration, Liquidity Provider feed health monitoring, and live session traffic inspection.
* **`RfqDeskService`**: Dealer-side market maker desk inbox, client IOI handling, quote response dispatch, and manual intervention overrides.
* **`NotificationService`**: Server-push notification channel streaming trade confirmations, credit limit alerts, and inter-desk risk transfer requests.
* **`CorporateActionsService`**: Effective-dated bond corporate actions lifecycle (`ListInstrumentSchedule`, `ListCorporateActions`, `ConfirmCorporateAction`, `ApplyCorporateAction`).

### 5.7 Excel Integration (27 CELNET.* Worksheet Functions)

Celnet provides an institutional C-API Excel Add-in ([`crates/celnet-client`](../../crates/celnet-client)) that exposes 27 dynamic-array spill functions:

| Function Category | Function Names | Capability / Description |
|---|---|---|
| **Vanilla & Strategy** | `CELNET.PRICE`, `CELNET.GREEKS`, `CELNET.STRATEGY` | Full Greek spill, premium styles, two-way bid/offer. |
| **Exotics & Structured** | `CELNET.BARRIER`, `CELNET.DIGITAL`, `CELNET.ASIAN`, `CELNET.TARF` | Analytical and QMC pricing of exotic payoff structures. |
| **Fixed Income & Rates** | `CELNET.BOND.PRICE`, `CELNET.BOND.YTM`, `CELNET.OIS.PV`, `CELNET.CURVE` | Bond pricing, YTM solves, curve bootstrapping, DV01 ladders. |
| **Market Data & Smile** | `CELNET.SMILE`, `CELNET.SURFACE.MARK`, `CELNET.VOL` | Dynamic vol surface extraction, smile parameter spills. |
| **Risk & Analytics** | `CELNET.RISK.CUBE`, `CELNET.VAR`, `CELNET.XVA` | Server-side position rollup, VaR/ES, and CVA/DVA calculations. |
| **Trading & RFQ** | `CELNET.RFQ.REQUEST`, `CELNET.RFQ.ACCEPT`, `CELNET.STREAM` | Real-time streaming and one-click execution from Excel. |

---

## 6. Competitive Differentiators

Celnet structurally outperforms both legacy vendor platforms and custom in-house banking engines:

```
+---------------------------+------------------------+--------------------------+-------------------------+
| Capability / Feature      | Celnet                 | Legacy Monoliths         | Modular In-House Stacks |
|                           |                        | (Murex MX.3, Calypso)    | (QuantLib, Custom C++)  |
+---------------------------+------------------------+--------------------------+-------------------------+
| Core Pricing Latency      | 42 ns p50 / 84 ns p99  | 150 us - 2.5 ms          | 1.2 us - 15 us          |
| Peak Pricing Throughput   | 13.63 M opt/s/core     | 50k - 250k opt/s/core    | 500k - 2.0 M opt/s/core |
| Hot-Path Allocation       | Zero Heap Allocation   | Heavy JVM/C++ Malloc     | Partial / Variable      |
| Codebase Memory Safety    | #![forbid(unsafe_code)]| C/C++ Segfault Risks     | Unsafe Memory Risk      |
| Wire Contract Parity      | 1 Contract, 7 Surfaces | Fragmented APIs & ETL    | Point-to-Point Wrappers |
| Internal IPC Latency      | 14.46 ns (SBE /dev/shm)| 250 us - 1.2 ms (gRPC/MQ)| 5 us - 50 us (Pipes)    |
| Quant Extensibility       | WASM Sandboxed SDK     | Proprietary C/Java SDK   | Hard-Coded Recompiles   |
| High Availability / State | Raft Replicated Log    | Active/Passive DB Mirror | Fragile Redis / DB Sync |
| Cross-Asset Carry Model   | Unified ADR-0012 Kernel| Siloed Asset Libraries   | Duplicated Code Paths   |
+---------------------------+------------------------+--------------------------+-------------------------+
```

### 1. Structural Performance Hegemony
While traditional banking monoliths spend microseconds inside garbage collectors, memory allocators, and XML/JSON serialization layers, Celnet executes in **pure mechanical sympathy**:
- Pinned CPU cores process options at **13.63 Million valuations per second per core**.
- The entire Greek strip (13 sensitivities) evaluates in **36.53 nanoseconds**—faster than an L3 cache miss on legacy hardware.

### 2. Elimination of the "Micro-Price Discrepancy"
Because Celnet enforces a single wire contract and mathematical core, pricing an option in the **WebGPU Trader GUI**, calling it via **Excel Add-in**, streaming it via **WebSocket**, or lifting it via **FIX** produces identical floating-point bit representations (`to_bits`). The multi-million dollar reconciliation disputes common between trading desks and risk departments are mathematically eliminated.

### 3. Hot-Plug Quant Agility via WebAssembly
Traditional institutions require 6-month release cycles to deploy proprietary pricing models to production. Celnet's **Open Quant SDK** allows quant teams to compile proprietary models into sandboxed WebAssembly (WASM) modules ([`celnet-plugin-host`](../../crates/celnet-plugin-host)). Models run in-engine with microsecond execution times, full mathematical memory safety, and zero risk of crashing the primary trading process.

---

## 7. Production Deployment & Hardware Sizing

Celnet is designed for deterministic bare-metal deployment:

* **Recommended Production Sizing (Single Engine Node)**:
  - **CPU**: AMD EPYC 9654 (96 cores, 2.4 GHz base) or Ampere Altra Max (128 ARM64 Neoverse cores) or Apple Silicon M-series.
  - **Memory**: 128 GB DDR5-4800 ECC (Quad-Channel or Octa-Channel).
  - **OS**: Linux (Kernel 6.11+ with `io_uring` zero-copy receive support, `CONFIG_PREEMPT_RT` enabled for real-time determinism).
  - **Network**: Solarflare Onload / Mellanox ConnectX-6 Dx 25/100GbE (Kernel-bypass EF_VI or AF_XDP).
* **Cluster Deployment Topology**:
  - **Cluster Size**: 3 to 5 nodes participating in Raft state replication.
  - **High Availability**: Sub-50ms leader failover with zero transactional state loss.
  - **Scale-Out**: Consistent hashing book partitioner dynamically balances millions of active positions across the fleet.

---

## 8. Summary of Active Crates in the Core Engine

The 57 crates of Celnet represent an engineered hierarchy of separation of concerns:

- **Core Primitives**: [`celnet-core`](../../crates/celnet-core), [`celnet-types`](../../crates/celnet-types), [`celnet-proto`](../../crates/celnet-proto).
- **Asset Class Pricers**: [`celnet-vanilla`](../../crates/celnet-vanilla), [`celnet-exotics`](../../crates/celnet-exotics), [`celnet-surface`](../../crates/celnet-surface), [`celnet-rates`](../../crates/celnet-rates), [`celnet-bond`](../../crates/celnet-bond), [`celnet-equity-vanilla`](../../crates/celnet-equity-vanilla), [`celnet-commodity-vanilla`](../../crates/celnet-commodity-vanilla), [`celnet-crypto-vanilla`](../../crates/celnet-crypto-vanilla), [`celnet-linear`](../../crates/celnet-linear), [`celnet-heston`](../../crates/celnet-heston), [`celnet-xva`](../../crates/celnet-xva).
- **Risk & Analytics**: [`celnet-rates-risk`](../../crates/celnet-rates-risk), [`celnet-risk-cube`](../../crates/celnet-risk-cube), [`celnet-risk-fleet`](../../crates/celnet-risk-fleet), [`celnet-risk-normalize`](../../crates/celnet-risk-normalize), [`celnet-limits`](../../crates/celnet-limits), [`celnet-risk-accel`](../../crates/celnet-risk-accel), [`celnet-qmc`](../../crates/celnet-qmc), [`celnet-gpu`](../../crates/celnet-gpu).
- **Edge Transport & Messaging**: [`celnet-shm`](../../crates/celnet-shm), [`celnet-sbe`](../../crates/celnet-sbe), [`celnet-fanout`](../../crates/celnet-fanout), [`celnet-fix`](../../crates/celnet-fix), [`celnet-replog`](../../crates/celnet-replog), [`celnet-journal`](../../crates/celnet-journal).
- **Workflow & Execution**: [`celnet-rfq`](../../crates/celnet-rfq), [`celnet-hedge-routing`](../../crates/celnet-hedge-routing), [`celnet-corpactions`](../../crates/celnet-corpactions), [`celnet-refstore`](../../crates/celnet-refstore), [`celnet-entitlements`](../../crates/celnet-entitlements), [`celnet-server`](../../crates/celnet-server), [`celnet-client`](../../crates/celnet-client), [`celnet-cli`](../../crates/celnet-cli).

Celnet delivers the ultimate unification of modern quantitative mathematics and mechanical ultra-low latency engineering.
