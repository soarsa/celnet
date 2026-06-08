<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Quant & Pricing Methodology Coverage</sub>

# 4. Quant & Pricing Methodology Coverage

Celnet is not a thin challenger closing gaps — it is a functionally complete, evidence-backed superset of the quant catalogue a derivatives desk stitches together today, proven by a runnable parity matrix against independent oracles and reachable identically from five clients. Every price, every Greek, every smile and every exotic is computed by the **same in-core engine** that powers the live pricing edge — so the number a trader sees in the ticket, the number that streams over the wire, and the number a quant pulls into a spreadsheet are one and the same. This section maps the analytics surface end to end: from a single vanilla through the full Greek set and the four delta conventions, the five-model smile/surface engine, the **full exotic and structured catalogue** — first-generation barriers/digitals/touches, closed-form structured products, Monte-Carlo path-dependents, American/Bermudan early exercise, correlated multi-asset baskets, and an LSV booking engine with a standalone Heston backbone — to the per-product-class validation regime and the SDK seam that extends Celnet without forking it.

![Quant coverage map — vanilla, Greeks, conventions, smile/surface, the full exotic & structured catalogue, and the SDK extension seam](../assets/celnet-capabilities/fig-03-quant-coverage.png)
*Figure 3 — The Celnet quant coverage map: a single in-core library spanning vanilla pricing, the full FX desk Greek set, the four delta conventions, the five-model smile/surface engine, and the full exotic & structured catalogue (vanilla-family · path-dependent · structured/MC · American–Bermudan · correlated basket · LSV booking), validated by ~26 parity rows against independent oracles plus frozen QuantLib golden tables, with the Open Quant SDK as the extension seam to bespoke products.*

### 4.1 Vanilla pricing and the full FX desk Greek set in one pass

Vanilla European options are priced with the Garman–Kohlhagen model, struck off the outright forward and discounted with **separate domestic and foreign discount factors** — the correct FX construction, not an equity model bent into shape. From that single valuation Celnet returns **price plus the full FX desk Greek set in one pass**: spot-delta and forward-delta, gamma, vega, theta, both rhos (domestic `rho_dom` and foreign `rho_for`), and the full second- and third-order book — vanna, volga, charm, speed, zomma and color. A desk gets every sensitivity it risks against from a single call, with no second pricing round-trip to chase a cross-Greek.

Every one of these sensitivities is **cross-validated by finite differences** against the analytic closed form, so the analytic Greeks the desk trades on are continuously checked against an independent numerical bump of the same model (`celnet-parity/tests/greeks.rs`). Correctness is a property of the build, not a hope. The hot core that produces price + 13 Greeks is the pinned zero-alloc loop benchmarked in chapter 7.

| Greek family | Sensitivities delivered in the single pass |
|---|---|
| Value | Price (off the outright forward, dual discount factors) |
| First order | Spot-delta, forward-delta, vega, theta, `rho_dom`, `rho_for` |
| Second order | Gamma, vanna, volga, charm |
| Third order | Speed, zomma, color |

### 4.2 Delta conventions and the branch-aware strike↔delta solver

FX desks quote in delta, not strike, and the mapping between them depends on convention. Celnet implements all **four delta conventions** — spot and forward, each in unadjusted and premium-adjusted form — and a **branch-aware strike↔delta solver** that inverts the relationship robustly. Crucially, the solver is aware of the **premium-adjusted call-delta maximum**: where the premium-adjusted delta function turns over and a naive root-finder would pick the wrong branch, Celnet selects the correct one. Both at-the-money rules are built in and applied **sign-correctly per convention**: at-the-money-forward (ATMF) and the delta-neutral straddle (DNS). A trader can pin a wing by 25-delta or 10-delta, anchor the smile at ATMF or DNS, and trust that the strike Celnet returns is the strike the market means.

### 4.3 The smile and surface engine — five smile families

Celnet marks volatility with a full smile-and-surface engine rather than a single fixed parameterisation. **Five smile models** are available — **Vanna-Volga, SABR, raw-SVI, SSVI and eSSVI** (the extended, maturity-dependent-ρ surface in `celnet-surface/src/extended_surface.rs`) — and a **smile-model selector** lets a desk mark or recalibrate a smile under any of them and compare. SSVI is byte-recovered as the constant-ρ special case of eSSVI, so the extension is additive, not a fork. Market quotes enter the engine the way they are actually traded: a **broker-strangle → smile-strangle fixed-point calibration** resolves the market (broker) strangle into a consistent smile strangle, so the calibrated smile reproduces the prices a desk was shown.

No-arbitrage is enforced, not assumed. **Arbitrage gates** check butterfly density non-negativity (no negative implied densities, via pointwise Breeden–Litzenberger re-pricing), vertical-spread monotonicity, and calendar total-variance monotonicity across tenors. The eSSVI surface carries the closed-form static no-arbitrage conditions (per-slice butterfly + consecutive-slice calendar). The term structure is assembled **arbitrage-free, interpolated in total variance**, so volatility between pillar tenors is consistent and free of calendar arbitrage by construction. For the exotic and LSV engines below, the smile is converted to a **Dupire local-volatility** surface (`celnet-exotics/src/leverage.rs`).

![Surface pipeline — broker quotes through calibration, arbitrage gates, model selection, and the arb-free term structure](../assets/celnet-capabilities/fig-04-surface-pipeline.png)
*Figure 4 — The surface pipeline: broker (market) strangle quotes enter a smile-strangle fixed-point calibration, pass butterfly / vertical / calendar arbitrage gates, are marked under a selectable smile model (VV / SABR / SVI / SSVI / eSSVI), and are woven into an arbitrage-free term structure interpolated in total variance.*

| Stage | What the engine does |
|---|---|
| Quote intake | Broker (market) strangle → smile-strangle fixed-point calibration |
| Smile models | Vanna-Volga, SABR, raw-SVI, SSVI, **eSSVI** — chosen via the model selector |
| Arbitrage gates | Butterfly density non-negative, vertical monotonic, calendar total-variance monotonic |
| Term structure | Arbitrage-free, interpolated in total variance |
| Local vol | Dupire local-volatility extraction feeding the exotic/PDE/LSV engines |
| Marking | Mark / recalibrate / publish under any model with surface versioning |

The marking workflow is exercised live from the GUI's Surface workspace — a smile chart alongside the ATM / 25RR / 25BF / 10RR / 10BF marking grid, the arbitrage-free gate, the broker-calibrated smile, an explicit surface version with reset and publish controls, and **five model chips including eSSVI**.

![Surface marking workspace — smile chart, ATM/25RR/25BF/10RR/10BF grid, arb-free gate, broker calibration, surface version](../assets/celnet-capabilities/shot-03-surface-marking.png)
*Screenshot 3 — The Surface workspace in the live GUI: smile chart, the ATM / 25RR / 25BF / 10RR / 10BF marking grid, the arbitrage-free gate, broker-calibrated smile, surface version, the five model chips (VV / SABR / SVI / SSVI / eSSVI), and Reset / Publish.*

### 4.4 The full exotic & structured catalogue

On top of vanilla and the smile engine, Celnet ships the **full FX exotic and structured catalogue** — not a first-generation subset. Every product below is a real, shipped engine in `celnet-exotics` (and `celnet-heston`), exposed on the one wire as a oneof arm of the unified `Instrument`, and gated by an independent parity row in `celnet-parity`. The catalogue is organised by the numerical regime each product class genuinely lives in — which is exactly what determines its validation bar in §4.5.

**(a) First-generation vanilla-family exotics** (`barrier.rs`, `digital.rs`, `touch.rs`, plus the window-barrier path) — **digitals, one-touch and no-touch, double-no-touch and double-touch, single and double barriers** (knock-in and knock-out), and **window barriers**. What sets this band apart is **method triangulation**: each product is reachable by multiple independent methods, cross-validated against one another so PDE, Monte-Carlo and analytic agree before a price is trusted.

- **Analytic** — closed-form barrier valuation via the reflection-principle construction.
- **Crank-Nicolson PDE** — a finite-difference solver (`pde.rs`) with a **Rannacher start-up** that damps the oscillations barriers and digital payoffs would otherwise induce near the boundary.
- **Philox Monte-Carlo** — a counter-based simulation path (`rng.rs`), bit-reproducible across runs and across CPU and GPU.
- **Survival-weighted Vanna-Volga overlay** (`market_hedge_overlay.rs`) — a market overlay that re-introduces smile risk into the touch / barrier price, weighted by survival probability.

For this band, **PDE ≈ MC ≈ analytic** is a continuously enforced invariant, and digitals, both touch styles, all eight barriers and the window barrier are validated against the **frozen QuantLib golden tables** (`celnet-golden/data/{digital_gk,touch_gk,barrier_gk,double_barrier_gk}.csv`).

| Product | Analytic | Crank-Nicolson / Rannacher PDE | Philox Monte-Carlo | Survival-weighted VV overlay |
|---|---|---|---|---|
| Digitals | ✓ | ✓ | ✓ | ✓ |
| One-touch / No-touch | ✓ | ✓ | ✓ | ✓ |
| Double-no-touch / Double-touch | ✓ | ✓ | ✓ | ✓ |
| Single barriers (KI / KO) | ✓ | ✓ | ✓ | ✓ |
| Double barriers (KI / KO) | ✓ | ✓ | ✓ | ✓ |
| Window barriers | — | ✓ | ✓ | ✓ |

**(b) Closed-form structured products** — priced by genuine closed-form or model-free static replication, then gated at closed-form precision against an independent recomputation:

- **Variance swap** (`var_swap.rs`) — fair variance strike by **log-contract `1/K²` static replication** (Demeterfi–Kamal–Zou / Carr–Madan) over the smile-consistent OTM forward-option strip, with an adaptive wing extended to a relative-convergence floor.
- **Volatility swap** (`vol_swap.rs`) — the **Carr–Lee convexity (Jensen) adjustment** `K_vol = √K_var − Var(v)/(8·K_var^{3/2})`, strictly below `√K_var` for any non-degenerate smile.
- **Arithmetic Asian** (`asian.rs`) — the arithmetic average is not lognormal, so there is no exact form; Celnet ships the two market-standard fast analytic estimators, each labelled at its true accuracy: **Turnbull–Wakeman** two-moment lognormal matching and **Curran** geometric-conditioning, handling the FX carry and the seasoned (in-progress-average) case.
- **Forward-start vanilla & cliquet** (`forward_start.rs`) — the exact **Rubinstein (1990)** FX dual-carry strike-reset closed form `V = e^{−r_f·t₁}·S₀·u(m, T−t₁)`; a plain (uncapped) cliquet is the exact sum of its forward-start legs, while a locally-capped/floored cliquet falls to Monte-Carlo.
- **Quanto** (`quanto.rs`) — exact closed form via the quanto drift adjustment `−ρ·σ_S·σ_Z`, with the Monte-Carlo engine as a cross-check.
- **Lookback** (`lookback.rs`) — fixed- and floating-strike closed forms (Goldman–Sosin–Gatto / Conze–Viswanathan) for continuous monitoring, with a Brownian-bridge extremum-correction Monte-Carlo that converges to the continuous-monitoring oracle.

**(c) Monte-Carlo path-dependents** — products whose payoff structure has no closed form; priced on the counter-based, antithetic, bit-reproducible Monte-Carlo engine over the scrambled-Sobol / Brownian-bridge quasi-random stack. **Each carries an honest Monte-Carlo `price_std_error` on the wire — never a machine-precision claim:**

- **TARF** (`tarf.rs`) — target-redemption forward: a strip of periodic fixings with a cumulative target that knocks the structure out, gearing on the adverse side, and the explicit `FullGain` vs `CappedGain` gap-risk premium.
- **Accumulator / decumulator** (`accumulator.rs`) — periodic accumulation at a discounted pivot with a knock-out barrier and adverse-side gearing, in discrete and Brownian-bridge-continuous monitoring conventions.
- **Correlated multi-asset basket / best-of / worst-of** (`multiasset.rs`) — weighted basket, best-of-N and worst-of-N (rainbow) calls and puts over a portfolio of pairs, priced by a **Cholesky-correlated** multi-asset GBM Monte-Carlo over the QMC stack; a non-SPD correlation matrix is rejected honestly rather than silently regularised.

**(d) American / Bermudan early exercise** (`american.rs`) — physically-settled FX options trade American-style, and Celnet prices them **two independent ways and cross-validates them**: a **projected-SOR (PSOR) free-boundary Crank–Nicolson finite-difference** solver that treats early exercise as a linear-complementarity problem and discovers the exercise boundary without tracking it, and a **Longstaff–Schwartz regression Monte-Carlo (LSM)** over Sobol/Brownian-bridge paths. The Bermudan case applies the exercise projection only at permitted dates; the LSM estimator carries an honest Monte-Carlo `price_std_error`.

**(e) LSV booking engine + standalone Heston** — the booking-grade second-generation layer:

- **Local-Stochastic-Volatility booking model** (`lsv.rs`, with `stochvol.rs`, `leverage.rs`, `particle.rs`, `adi.rs`) — a Heston variance backbone with a Dupire **leverage** surface calibrated by the **interacting-particle method** (the McKean–Vlasov leverage identity, calibrated forward-in-time, non-parametrically via a Nadaraya–Watson kernel estimate of `E[v|S]`). It prices on **two independent engines** — a 2-D Hundsdorfer–Verwer **ADI PDE** on the (spot, variance) grid and the antithetic QE-stepper Monte-Carlo — and is anchored by three asserted guarantees: it reprices the arbitrage-free vanilla surface (PDE), ADI-PDE ≈ MC on a window-barrier payoff, and the pure-local-vol limit (`ξ=0, v₀=θ`) recovers the Dupire price. Selectable on the wire via the `PricingModel` directive (`DEFAULT` analytic vs `LOCAL_STOCH_VOL`).
- **Standalone Heston** (`celnet-heston`) — European vanilla via **two genuinely independent Fourier transforms** of the same branch-cut-free (Cui–del Baño Rollin–Germano) characteristic function: **Carr–Madan** damped-integral Gauss–Legendre quadrature and the **Fang–Oosterlee COS** method, agreeing to `|cm − cos| ≤ 1e-8 + 1e-7·price` over the ≤3y FX grid, and gated against a frozen **QuantLib golden table** (`celnet-golden/data/heston_fo.csv`). No external FFT dependency — single-strike pricing controls accuracy directly.

### 4.x Sobol quasi-Monte-Carlo variance reduction

The Monte-Carlo path-dependents above ride a complete **randomized quasi-Monte-Carlo (RQMC)** stack (`celnet-qmc`): a gray-code **Joe–Kuo Sobol'** generator with an embedded BSD-licensed direction-number table, an **Owen-style nested digital scramble** for unbiased replications, principal-bisection **Brownian-bridge** path construction loading the dominant variance onto the best-distributed dimensions, and a full-precision inverse-normal CDF. The variance reduction is **measured, not asserted** (`celnet-parity/tests/qmc.rs`, gated ≥ 3×): on exact-value targets the QMC RMSE is roughly **38× smaller for a geometric-average Asian and ~88× smaller for a European** than a fair plain pseudo-random estimator at the same path budget. The Sobol direction numbers are exposed verbatim for GPU reuse; any GPU throughput claim is correctness/ratio-only (see chapters 7–8).

### 4.5 Validation regime — the right oracle for each product class

Pricing is only as good as the dates, conventions and oracles underneath it. Celnet runs a **dual-calendar date engine** (`celnet-calendar`) — resolving spot, expiry, delivery and roll across the two settlement calendars an FX pair actually depends on, including ON/TN/SN, IMM and broken dates — and a **per-(currency-pair, tenor) convention registry** (`celnet-conventions`) carrying the right delta convention, ATM rule, day-count and premium treatment per instrument, validated against published EMTA/ISDA tables across a 19-pair universe.

**Honesty as a differentiator: the validation method is stated per product class — Celnet does not blanket-claim "machine precision vs QuantLib".**

| Product class | Validation oracle | Bar |
|---|---|---|
| Vanilla, digitals, both touch styles, all 8 barriers, window barrier | Frozen **QuantLib 1.42.1 golden tables** + analytic/PDE cross-validation | ~1e-9/1e-10 (closed-form), ~1e-6 (series-gated double-barrier) |
| Standalone Heston | Frozen **QuantLib golden table** (`heston_fo.csv`) + CM-vs-COS internal agreement | `1e-8 + 1e-7·price` |
| Variance/vol swap, forward-start/cliquet, lookback (continuous), quanto | Independent closed-form recomputation / closed-form limits | ~1e-6 to ~1e-10 |
| Asian (Turnbull–Wakeman, Curran) | Closed-form degenerate limits + independent MC within reported stderr | TW gated at its true ~1.5% approximation band; Curran within MC stderr |
| eSSVI surface | Pointwise Breeden–Litzenberger density + calendar-monotone re-pricing; SSVI byte-recovery | density ≥ 0, ≤1e-9 self-reprice |
| TARF, accumulator, basket/best-of/worst-of, American/Bermudan (LSM) | Independent code-disjoint MC / closed-form limits | within reported **price std-error** band |
| LSV booking model | Vanilla-surface reprice + ADI-PDE ≈ MC + pure-local-vol Dupire limit | tolerance-gated, asserted in-suite |

The whole library is re-checked on every build by **~26 parity rows against genuinely independent oracles** (`celnet-parity/tests/*`) plus the frozen golden tables — never plausibility, always agreement with an independent computation. **MC-priced products carry an honest `price_std_error`; "machine precision" is reserved for the analytic/PDE/golden-gated set.** (The validation discipline is sharpened by a real defense: a circular self-oracle bug in the FRTB low-correlation floor was caught and the constant re-derived against the published BCBS source — see chapter 14.)

### 4.6 Extensible to bespoke products via the Open Quant SDK

The catalogue above is the **shipped core**, already matching the structured/path-dependent breadth the deep-catalogue platforms charge for — not the ceiling, and not the only route beyond first-generation. The **Open Quant SDK** exposes the same `PricingModel`, `SmileModel` and `Calibration` seams the core uses, so a desk can extend Celnet to **house and bespoke payoffs** and run them **inside the same engine** — sandboxed (Tier-0 native or the Tier-2 wasmi deterministic sandbox), bit-reproducible by replay, and hot-loadable, without forking Celnet. Coverage grows with the desk's book; structures like TARFs, quantos and baskets are **already shipped core**, so the SDK is for genuinely novel IP rather than filling catalogue gaps.

The Ticket workspace is where this breadth becomes a workflow: a structure selector spanning the **full catalogue** (vanilla strategies, the first-generation exotics, Asians, forward-start/cliquet, quanto, lookback, TARF, accumulator, window barriers, **American/Bermudan**, and **correlated baskets/best-of/worst-of**), the notional and tenor strip, the legs and strikes, a zero-cost Solve, two-way BID / MID / OFFER, the active conventions shown on the face of the ticket, and direct hand-off to Request-quote, Stream or Add-to-risk.

![Ticket structuring — structure selector, notional, tenor strip, legs and strikes, Solve, BID/MID/OFFER, conventions on the face](../assets/celnet-capabilities/shot-02-ticket-structuring.png)
*Screenshot 2 — The Ticket workspace: a full-catalogue structure selector, notional, tenor strip, legs with strikes, zero-cost Solve, two-way BID / MID / OFFER, conventions shown on the face, and Request-quote / Stream / Add-to-risk actions.*

---

### Honest boundary (carried verbatim across the doc set)

- **MC-priced products** (TARF, accumulator, discrete lookback, basket/best-of/worst-of, American via LSM, capped/floored cliquet) carry a **price std-error** — never labelled "machine precision"; that bar is reserved for analytic/PDE/golden-gated products.
- **CUDA/NVIDIA absolute GPU throughput, ≤50ms exotic and Workload-A/B absolute numbers are deploy-gated.** M4 Metal lacks f64, so in-repo GPU proofs are **correctness + ratios only** (M4/Lavapipe). Never claim f64 on Metal.
- **Multi-source surface aggregation** — the blend/staleness/divergence algorithm is built and gated; live multi-vendor quote *values* are an integration/deploy target, not in-repo data.

---
<sub>[← System Architecture](03-system-architecture.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Extensibility →](05-extensibility-plugins.md)</sub>
