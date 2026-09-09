# CelNet Master Cross-Asset Integration Audit & Target Optimizations
**Institutional Review of Mathematical Invariants, Wire Contracts, IPC Protocols, and Risk Aggregation Across All Crates**
**Evaluation Date:** September 2026 | **Target Platform:** CelNet Zero-Allocation High-Throughput Pricing & Risk Engine

---

## 1. Executive Summary & Asset Class Scorecard

CelNet was originally conceived as a sub-microsecond FX & Precious Metals options pricing and streaming engine. Through architectural decisions (notably **ADR-0008** for multi-asset cost-of-carry, **ADR-0010** for rate curve convergence, **ADR-0012** for the unified forward carry seam, **ADR-0018** for fixed income, **ADR-0019** for credit, and **ADR-0020** for the central `Priceable`/`MarketResolver`/`RiskMeasure` contract), the platform has expanded toward a universal cross-asset trading and risk system.

This audit evaluates the depth, completeness, and optimization of integration across all six core institutional asset classes:
1. **Foreign Exchange (FX & Metals)**
2. **Rates & Fixed Income (GIRR, OIS, Vanilla IRS, FRA, Cash Bonds)**
3. **Credit Derivatives (CDS, Hazard Rates, Survival Curves, Jump-to-Default)**
4. **Equity Derivatives (Single Stocks, Indices, Dividends, Corporate Actions)**
5. **Commodities (Futures Options, Energy, Agriculture, Contango/Backwardation)**
6. **Digital Assets / Crypto (Linear USD-Margined, Inverse Coin-Margined, Perpetuals)**

### Cross-Asset Institutional Scorecard

| Asset Class | Quant / Math Engine | Core Contract (`Priceable`) | Protobuf Wire (`celnet.proto`) | ULL IPC (`celnet-sbe`) | Risk Cube / FRTB | GUI / Ticket Parity | Overall Grade |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **FX & Metals** | **Grade A+** (22.5 ns Garman-Kohlhagen, 13 Greeks, LSV, Exotics) | **Grade A+** (`VanillaEngine`, 23 Exotics) | **Grade A+** (Complete parity) | **Grade A** (Full SBE messages, 14 Greeks) | **Grade A+** (Full AAD + bump VaR, SbM) | **Grade A+** (Full ticket, blotter, surface) | **OPTIMAL** |
| **Rates / FI** | **Grade A** (OIS bootstrap, DCF, duration, convexity) | **Grade B** (OIS & Bond impl `Priceable`; IRS & FRA delegate ad-hoc) | **Grade B-** (Separate `RatesInstrument` wire silo) | **Grade F** (Zero Rates messages in SBE) | **Grade A-** (GIRR delta SbM live; curvature deferred) | **Grade B+** (Curve workspace, rates book, blotter) | **PARTIAL** |
| **Equities** | **Grade B** (Generalized BSM, continuous dividend $q$) | **Grade A-** (`EquityVanillaEngine` live) | **Grade B+** (`EquityRef` on `Underlying`) | **Grade D** (No ticker/dividend fields in SBE) | **Grade B** (Additive Greeks live; discrete divs absent) | **Grade B** (Vanilla ticket supports equity) | **SUB-OPTIMAL** |
| **Commodities** | **Grade B** (Black-76, scalar carry $b$) | **Grade A-** (`CommodityVanillaEngine` live) | **Grade B+** (`CommodityRef` on `Underlying`) | **Grade D** (No delivery/contract fields in SBE) | **Grade B** (Additive Greeks live; term structure absent) | **Grade B** (Vanilla ticket supports commodity) | **SUB-OPTIMAL** |
| **Digital Assets** | **Grade A-** (Linear & Inverse Coin $1/S_T$, Perpetual) | **Grade A-** (`CryptoLinear`, `CryptoInverse`, `Perpetual`) | **Grade A-** (`CryptoPair`, `SettlementStyle`) | **Grade F** (No funding rate, no coin delta in SBE) | **Grade C+** (Inverse coin numeraire collapse deferred) | **Grade B+** (Inverse settlement selector in ticket) | **SUB-OPTIMAL** |
| **Credit** | **Grade F** (Unbuilt; ADR-0019 design only) | **Grade F** (No `celnet-credit` crate) | **Grade F** (Zero credit messages on wire) | **Grade F** (No CDS messages in SBE) | **Grade F** (CR01/JTD absent from cube) | **Grade F** (No CDS ticket or credit curve) | **UNBUILT** |

---

## 2. Codebase Topography & Cross-Asset Architecture

The virtual workspace comprises 71 crates under `crates/`, with clear layered segregation:

```
                      [ GUI / WebSockets / gRPC Client / Excel Add-In ]
                                             │
                       [ celnet-server (Gateway / Edge / RPC) ]
                      ┌──────────────────────┼──────────────────────┐
                      ▼                      ▼                      ▼
            [ celnet-vanilla ]      [ celnet-rates ]       [ celnet-linear ]
            [ celnet-surface ]      [ celnet-bond ]        [ celnet-equity-vanilla ]
            [ celnet-exotics ]      [ celnet-rates-risk ]  [ celnet-commodity-vanilla ]
            [ celnet-heston ]                              [ celnet-crypto-vanilla ]
                      │                      │                      │
                      └──────────────────────┼──────────────────────┘
                                             ▼
                                    [ celnet-core ]
                        (ADR-0020 Central Contract, Carry Seam)
                                             │
                                    [ celnet-types ]
                       (DiscountCurve, Carry, Greeks, DayCount)
                                             │
                                    [ celnet-proto ]
                                    [ celnet-sbe ]
                                    [ celnet-shm ]
```

### Core Invariants Maintained Across All Crates
1. **`#![forbid(unsafe_code)]`**: Enforced workspace-wide.
2. **Deterministic Arithmetic**: Routed through `libm` transcendentals (`exp`, `ln`, `sqrt`, `erfc`); float comparison via `is_close`.
3. **Hot-Core Zero Allocation**: Hot streaming loops never allocate on the heap or dereference dynamic pointers.
4. **ADR-0016 Embargo**: `MarketState` holds flat scalars (`spot, r_dom, r_for, t, conventions, smile`); curve handles and borrowed references are strictly embargoed from the hot core.

---

## 3. Detailed Audit by Asset Class

### 3.1 Foreign Exchange (FX) & Precious Metals
- **Quant Implementation**: [`celnet-vanilla`](../../crates/celnet-vanilla), [`celnet-surface`](../../crates/celnet-surface), [`celnet-exotics`](../../crates/celnet-exotics), [`celnet-linear`](../../crates/celnet-linear).
- **Status**: **Optimal / Benchmark Standard**.
- **Strengths**:
  - Garman-Kohlhagen (1983) analytic pricing calculates present value and the full 13-Greek set in $22.5\text{ ns}$ with zero heap allocation.
  - Volatility surface pipeline is industry-leading: broker ATM + RR/BF 25Δ/10Δ market-strangle calibration with fixed-point iteration, SABR Hagan (2002/2014) density refinement, SVI/SSVI/eSSVI parametric slices with static calendar/butterfly no-arbitrage enforcement.
  - Exotics suite is comprehensive: 23 product families (barriers, double barriers, touches, digitals, Asians, cliquets, TARFs, accumulators, quantos, variance/volatility swaps, LSV particle calibration with 2-D Hundsdorfer-Verwer ADI finite-difference PDE solver).
  - Outright forwards, FX swaps, and NDFs are decoupled into `celnet-linear` with exact discounted cashflow identities.

### 3.2 Rates & Fixed Income (GIRR, OIS, IRS, FRA, Bonds)
- **Quant Implementation**: [`celnet-rates`](../../crates/celnet-rates), [`celnet-bond`](../../crates/celnet-bond), [`celnet-rates-risk`](../../crates/celnet-rates-risk).
- **Status**: **Partially Integrated; Architectural Leaks Identified**.
- **Findings & Critical Flaws**:
  1. **Two-Paradigm Wire Split (`celnet.proto`)**:
     - Despite ADR-0020's mandate to unify the platform, `celnet.proto` splits rates into a dedicated `message RatesInstrument` (lines 8664–8676) and a dedicated `RatesService` RPC (`PriceRates`), completely disconnected from `message Instrument` and `CelnetService.Price`.
  2. **Incomplete `Priceable` Implementation (`rates_pricing/contract.rs`)**:
     - `RatesOisEngine` and `BondEngine` implement `celnet_core::contract::Priceable`.
     - However, **`VanillaIrs` and `Fra` DO NOT implement `Priceable`!** In [`rates_pricing/contract.rs` lines 327–335](../../crates/celnet-server/src/rates_pricing/contract.rs#L327-L335), they bypass the central contract and delegate directly to helper functions (`price_irs`, `price_fra`).
  3. **Single-Curve Wire Limitation**:
     - In `RatesPriceRequest`, only a single `CurveSet` can be passed. Modern post-2008 interest rate pricing requires multi-curve discounting (discounting on OIS/SOFR/EURSTR while forecasting forward rates on Euribor or Term SOFR tenors). While `celnet-rates::vanilla_swap` supports multi-curve math internally, the wire API cannot ingest dual curves.
  4. **Key-Rate DV01 Ladder Vectorization**:
     - Key-rate DV01 calculation in `celnet-rates::ois_risk` uses sequential curve re-bootstrapping for each tenor bump. This lacks SIMD vectorization across the standard tenor vertices (1m, 3m, 6m, 1y, 2y, 3y, 5y, 7y, 10y, 15y, 20y, 30y).

### 3.3 Credit Derivatives (CDS, Hazard Rates, Survival Curves)
- **Quant Implementation**: Intended as `celnet-credit` per [ADR-0019](../../docs/adr/ADR-0019-credit-pricing-leaf.md) and [`docs/FI-CREDIT-ENGINE-DESIGN.md`](../../docs/fixed-income/FI-CREDIT-ENGINE-DESIGN.md).
- **Status**: **Completely Unbuilt (Missing Leaf)**.
- **Findings & Critical Flaws**:
  - There is currently **no `crates/celnet-credit`**.
  - Survival curve bootstrapping from CDS par spreads, hazard rate term structure, CDS upfront / MtM pricing, and Jump-to-Default (JTD) / CR01 calculations do not exist in the codebase.
  - Credit positions cannot be ingested into `celnet-risk-cube`.

### 3.4 Equity Derivatives
- **Quant Implementation**: [`celnet-equity-vanilla`](../../crates/celnet-equity-vanilla), [`celnet-corpactions`](../../crates/celnet-corpactions).
- **Status**: **Sub-Optimal; Exotics Gated & Discrete Dividends Omitted**.
- **Findings & Critical Flaws**:
  1. **Continuous Dividend Yield vs Discrete Cash Dividends**:
     - `celnet-equity-vanilla` models dividends solely as a continuous dividend yield $q$ ($b = r - q$).
     - Single-stock equity options require discrete dividend schedules with drop-off dates and cash amounts. Pricing with continuous yield on single stocks misprices OTM and ITM options significantly around ex-dividend dates.
  2. **The Server Cross-Asset Exotics Gate Defect**:
     - In [`celnet-server/src/pricer/engine.rs` lines 228–255](../../crates/celnet-server/src/pricer/engine.rs#L228-L255), `dispatch_cross_asset` **strictly rejects** any equity exotic (barriers, Asians, digitals, cliquets) with `PriceError::UnsupportedModel`.
     - Although `celnet-exotics` uses `ExoticInputs` capable of handling equity cost-of-carry, the server gateway artificially forbids them.
  3. **Corporate Actions Focus**:
     - `celnet-corpactions` is exclusively implemented for bonds (`BondSchedule`). Equity corporate actions (stock splits, spin-offs, rights offerings, special dividends) that adjust contract strike $K' = K / r$ and notional $N' = N \cdot r$ are missing.

### 3.5 Commodity Derivatives
- **Quant Implementation**: [`celnet-commodity-vanilla`](../../crates/celnet-commodity-vanilla).
- **Status**: **Sub-Optimal; Futures Term Structure Omitted**.
- **Findings & Critical Flaws**:
  1. **Futures Curve Dynamics (Contango & Backwardation)**:
     - The engine treats commodity carry as a flat scalar $b = r - y$ (convenience yield $y$).
     - Commodity markets trade on distinct futures strips (monthly contracts) where the basis changes along the curve (seasonal contango/backwardation). Scalar $b$ is insufficient for calendar spreads or strip options.
  2. **Exotics Gated at Server**:
     - Commodity Asian options (average price options, standard in crude oil and refined products) are fully coded in `celnet-exotics::asian`, but blocked by `dispatch_cross_asset` in `celnet-server`.

### 3.6 Digital Assets / Crypto
- **Quant Implementation**: [`celnet-crypto-vanilla`](../../crates/celnet-crypto-vanilla).
- **Status**: **Sub-Optimal; Inverse Numeraire Collapse Deferred & SBE Missing**.
- **Findings & Critical Flaws**:
  1. **Inverse Coin Numeraire Collapse Defect**:
     - In [`celnet-risk-normalize/src/lib.rs` lines 66–72](../../crates/celnet-risk-normalize/src/lib.rs#L66-L72), crypto-inverse (`1/S_T`) coin-margined payoffs (whose margin, premium, and vega settle in base coins like BTC/ETH) are explicitly **excluded from the additive linear numeraire seam**.
     - Base coin delta ($\Delta_{\text{coin}} = -\frac{K}{S^2} \Phi(d_2)$) is not netted in the risk cube's reporting currency vector.
  2. **Funding Rate Basis Protocol**:
     - Perpetuals are modeled as $t=0$ American perpetual options rather than rolling linear contracts with 8-hour funding rate intervals.

---

## 4. Cross-Cutting Systems & Algorithmic Audit

### 4.1 SBE Ultra-Low-Latency IPC Architecture (`celnet-sbe`)
The current SBE schema in [`celnet-sbe.xml`](../../crates/celnet-sbe/schema/celnet-sbe.xml) exhibits critical architectural deficiencies:
1. **Hardcoded `pairId: uint32`**:
   - Assumes every instrument maps to a 32-bit integer pair ID. Bonds (ISINs, CUSIPs), equity tickers (`AAPL.XNAS`), commodity futures contracts, and digital assets cannot be represented without external state.
2. **FX-Specific `GreeksStrip`**:
   ```xml
   <composite name="GreeksStrip">
     <type name="price" primitiveType="double"/>
     <type name="deltaSpot" primitiveType="double"/>
     <type name="deltaForward" primitiveType="double"/>
     <type name="gamma" primitiveType="double"/>
     <type name="vega" primitiveType="double"/>
     <type name="theta" primitiveType="double"/>
     <type name="rhoDomestic" primitiveType="double"/>
     <type name="rhoForeign" primitiveType="double"/>
     <!-- ... -->
   </composite>
   ```
   - Rates, Equities, Commodities, and Crypto must either fabricate dummy values for `rhoDomestic`/`rhoForeign` or fail to encode their natural sensitivities (`discountRho`, `carryRho`, `pv01`, `dv01`).
3. **Absence of Rates/Bond SBE Templates**:
   - There are zero SBE templates for `RatesQuote`, `BondQuote`, `YieldCurveTick`, or `KeyRateLadder`.

### 4.2 Cache Thrashing in Multi-Asset Basket Exotics (`celnet-exotics::multiasset`)
In [`celnet-exotics/src/multiasset.rs` lines 201–205](../../crates/celnet-exotics/src/multiasset.rs#L201-L205):
```rust
pub struct CholeskyFactor {
    l: Vec<Vec<f64>>,
}
```
- **Flaw**: `Vec<Vec<f64>>` creates an array of heap-allocated pointers.
- In multi-asset Monte Carlo ($N \times M$ paths), evaluating $L \cdot z$ causes repeated pointer chasing and L1 cache line evictions.
- **Optimization**: For $N \le 16$, a flat contiguous array `[f64; 256]` or single `Vec<f64>` with stride $N$ guarantees L1 cache line residency and allows auto-vectorization into AVX-512 / ARM Neon FMA registers.

### 4.3 Multi-Asset Greeks Omission
In [`celnet-exotics/src/multiasset.rs` lines 152–158](../../crates/celnet-exotics/src/multiasset.rs#L152-L158), basket options return `BasketEstimate { price, std_error }` with Greeks explicitly omitted.
- **Impact**: Baskets cannot flow into `celnet-risk-cube`, preventing book-level aggregation of rainbow and basket options.

---

## 5. Prioritized Target Optimizations Roadmap

```
Wave 1: Contract & Gateway Unification (Wire & Server)
Wave 2: Ultra-Low-Latency SBE IPC Protocol Generalization
Wave 3: Cache Locality & SIMD Numerical Acceleration
Wave 4: Cross-Asset Risk Cube & Inverse Numeraire Completion
Wave 5: Credit Asset Class Implementation (celnet-credit)
```

### Wave 1: Contract & Gateway Unification (Server & Core)
1. **Unify `price_cross_asset` in `celnet-server/src/pricer/engine.rs`**:
   - Extend `dispatch_cross_asset` to dispatch all 23 exotic product families for Equity, Commodity, and Crypto underlyings by bridging into `celnet-exotics::ExoticInputs`.
2. **Implement `Priceable` for `VanillaIrsEngine` and `FraEngine`**:
   - Complete the implementations in `crates/celnet-server/src/rates_pricing/contract.rs` so all linear FI instruments conform to ADR-0020.
3. **Multi-Curve Support in `RatesPriceRequest`**:
   - Add `optional CurveSet forecast_curve_set` to `RatesPriceRequest` in `celnet.proto`.

### Wave 2: Ultra-Low-Latency SBE IPC Protocol Generalization
1. **Upgrade `celnet-sbe.xml`**:
   - Replace `pairId: uint32` with `instrumentId: uint64` across all message templates.
   - Generalize `GreeksStrip` with a union:
     ```xml
     <composite name="GreeksStrip">
       <type name="price" primitiveType="double"/>
       <type name="delta" primitiveType="double"/>
       <type name="gamma" primitiveType="double"/>
       <type name="vega" primitiveType="double"/>
       <type name="theta" primitiveType="double"/>
       <type name="rateSens1" primitiveType="double"/> <!-- rhoDom / discountRho / pv01 -->
       <type name="rateSens2" primitiveType="double"/> <!-- rhoFor / carryRho / dv01 -->
     </composite>
     ```
   - Add Template 106 (`RatesQuote`) and Template 107 (`BondQuote`).

### Wave 3: Cache Locality & SIMD Numerical Acceleration
1. **Flat Contiguous Matrix in `CholeskyFactor`**:
   ```rust
   pub struct CholeskyFactor {
       dim: usize,
       data: [f64; 256], // Up to 16x16 stack-allocated, L1 cache resident
   }
   ```
2. **Vectorized Key-Rate DV01 Bumping**:
   - Pack the 12 standard GIRR vertices into a contiguous float buffer and re-evaluate discount factors using AVX-512 vectorized exponentials.
3. **Pathwise Basket Greeks**:
   - Implement pathwise adjoint differentiation in `celnet-exotics::multiasset` to output the $N$-dimensional delta vector.

### Wave 4: Cross-Asset Risk Cube & Inverse Numeraire Completion
1. **Crypto-Inverse Numeraire Normalization**:
   - In `celnet-risk-normalize::numeraire`, convert coin-margined Greeks to reporting currency:
     $$\Delta_{\text{reporting}} = \Delta_{\text{coin}} \cdot S + \text{Exposure}_{\text{coin}}$$
2. **Complete FRTB GIRR Curvature and Vega**:
   - Implement the full MAR21 curvature formulas in `celnet-rates-risk::girr`.

### Wave 5: Credit Asset Class Implementation (`celnet-credit`)
1. **Create `crates/celnet-credit`**:
   - `SurvivalCurve`: Piecewise-constant hazard rate bootstrap from single-name CDS spreads.
   - `CreditDefaultSwap`: Par spread, MtM, and upfront calculation off `celnet-rates::Curve` and `SurvivalCurve`.
   - `CreditRisk`: Analytic CR01 (1bp hazard shift) and Jump-to-Default ($JTD = (1 - R) \cdot \text{Notional} - \text{MtM}$).
2. **Fold into `celnet-risk-cube`**:
   - Add `RiskMeasure::CreditLadder` to `celnet-core::contract`.

---
*Verified against the CelNet Knowledge Graph (Project: `Users-adrian-code-celnet`, 40,085 nodes, 235,112 edges).*
