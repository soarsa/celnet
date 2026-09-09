<sub>[← Prev: Extensibility](05-extensibility-plugins.md) · [Index](../CELNET-CAPABILITIES.md) · [Next: Performance & Latency →](07-performance-latency.md) · [Showcase ↗](../celnet-capabilities.html)</sub>

# 6. Risk Management

Celnet treats risk as a single, integrated capability that spans two altitudes at once: the *micro* view of an individual position's behaviour under shocks, and the *macro* view of the firm's entire option book netted, rolled up, and policed against limits. Both are two zooms of one underlying truth — a position-fact cube — so a trader and a board-level risk officer are always looking at the same numbers, never a reconciliation of two systems.

![Risk architecture: book-shaped scenario risk and the firm-wide position-fact cube as two zooms of one fact model](../assets/celnet-capabilities/fig-06-risk-architecture.png)
*Figure 6 ([index](../CELNET-CAPABILITIES.md#figure-index)) — Risk architecture. A versioned marked-surface registry feeds book-shaped scenario risk (the desk's working view) and a convention-canonicalized OLAP position-fact cube (the firm-wide view) that aggregates vanilla **and exotic** legs. Normalize → cube → limits ∥ entitlements: one fact model, drilled at every altitude, extended to FRTB-SA regulatory capital and (internal) XVA.*

## 6.1 Book-shaped scenario risk

Risk at the desk is computed by real repricing, not by linear extrapolation from a single point. Every scenario number is produced by re-running the same nanosecond-scale pricing core that quotes live, so the risk surface is internally consistent with the price.

| Measure | What it gives the desk |
|---|---|
| Two-axis scenario grid | A shock matrix — typically spot × vol — where every cell is a genuine reprice. Axes are swappable, so the same grid renders P&L, delta, or vega exposure. |
| Bucketed vega ladder | Vega decomposed per `(tenor, delta)` pillar, so volatility risk is read where the desk actually marks it — short-dated vs long-dated, wings vs ATM. |
| Cross-gamma stencil | A two-dimensional curvature stencil capturing how delta moves as spot *and* vol move together, not just along a single axis. |
| Theta-roll | Time decay projected over forward horizons, rolling expiries node by node so the desk sees the carry of the book as the clock advances. |

Underpinning all of it is a **versioned marked-surface registry**. A calibrated smile is deposited under a fresh surface version and becomes the official mark; pricing and risk then *pin* against a named version. The registry holds the distinction between the official mark and the live market explicitly — official-vs-live is a first-class concept, not an overwrite. Asking for an unknown surface version is rejected outright rather than silently falling back to live data, so a risk run is always reproducible against exactly the surface it claims to use.

![Risk workspace: spot × vol reprice grid with swappable axes, P&L/delta/vega tabs, the per-tenor × delta vega ladder, cross-gamma and theta-roll](../assets/celnet-capabilities/shot-04-risk-scenario.png)
*Screenshot — the Risk workspace (Cmd-4): a spot × vol reprice grid with swappable axes and P&L / delta / vega tabs, the vega ladder bucketed per tenor × delta, plus cross-gamma and theta-roll.*

## 6.2 The firm-wide position-fact cube

Above the desk view sits an OLAP **position-fact cube** that gives the firm one canonical, queryable picture of all option risk. Positions arrive in many conventions and numeraires; Celnet canonicalizes them — convention-normalized and netted into a common numeraire — before anything is aggregated, so figures across pairs, books, and entities are genuinely additive where they should be.

The cube is an immutable fact table over eight dimensions:

| Dimension | Slices the firm by |
|---|---|
| Trader | the individual running the position |
| Book | the trading book |
| Desk | the desk |
| CurrencyPair | the FX-options pair |
| Location | trading location |
| Entity | legal entity |
| ValueDate | settlement / value date |
| Session | trading session |

Roll-up respects the mathematics of each measure. **Additive** measures (notional, delta, vega) accumulate incrementally up the hierarchy — fast, and exact. **Non-additive** measures (VaR, expected shortfall, curvature) are *re-derived per node* rather than summed, because a portfolio's tail risk is not the sum of its parts. The cube knows the difference and applies the correct treatment automatically at every level.

On top of the cube sits a **cascading limit tree** — board → entity → desk → book → trader — with both pre-trade and post-trade checks, so a candidate trade is tested against every limit it would touch before it is done, and the live book is policed continuously after. Finally, aggregation is **entitlement-aware**: the server prunes the cube to what a given viewer is allowed to see *before* it aggregates, so a trader sees their own slice, a desk head sees the desk, and the firm view rolls up only what the requester is entitled to — pre-aggregation pruning, not after-the-fact redaction.

## 6.3 One cube, two views — Book ↔ Risk drill

Because the desk's book view and the scenario-risk view are projections of the same fact cube, the GUI lets a trader move between altitudes in a single gesture. The **Book** workspace presents net P&L, Vega, Gamma, and Theta as headline cards with a per-pair breakdown and an aggregate vega ladder; selecting any aggregated book row **drills straight into that position's scenario risk** in the Risk workspace — same numbers, deeper zoom, no context switch and no separate tool.

![Book workspace: net P&L / Vega / Gamma / Theta cards, per-pair breakdown with drill-to-Risk, and the aggregate vega ladder](../assets/celnet-capabilities/shot-05-book-aggregate.png)
*Screenshot — the Book workspace (Cmd-5): net P&L / Vega / Gamma / Theta cards, a per-pair breakdown that drills to Risk, and the aggregate vega ladder rolled up from the cube.*

The result is one risk capability with no seams: real-reprice scenario risk for the trader, a convention-canonicalized OLAP cube with limits and entitlements for the firm, a versioned marked surface as the shared source of truth, and a single drill that connects the firm-wide book to a single position's behaviour under stress.

## 6.4 Exotic legs aggregate as first-class citizens

The cube is not a vanilla-only ledger. A booked exotic — a barrier, a digital — earns a genuine seat in every roll-up, contributing its **real** sensitivities rather than a vanilla proxy or, worse, a silent zero that would understate the firm's risk. An exotic leg supplies its full canonical Greek set and premium line so it sums into the net-Greeks and vega-ladder roll-up exactly like a vanilla leaf, and it re-prices the *real exotic payoff* under each scenario for VaR/ES and supplies its own up/down curvature legs for FRTB — so the firm tail and curvature reflect a knock-out's gamma sign-flip near the barrier or a digital's pin risk, never a smoothed-over vanilla approximation. Digitals use the closed-form `celnet-exotics` digital Greeks; barriers use a central finite-difference of the closed-form Reiner-Rubinstein price (the barrier closed form has no published higher-order Greek set, so finite-difference of the exact price is the honest, deterministic, `libm`-routed and bit-reproducible source). The first-class exotic legs are the **deterministic, closed-form** members of the catalogue — single barrier and European digital — which re-price exactly under shocks with no Monte-Carlo estimator noise, so they slot into the cube's machine-exact VaR/curvature path. MC-priced exotics (Asian, TARF, accumulator, discrete lookback) extend the same `ExoticKind` seam through a finite-difference of a common-random-number MC price that carries an explicit Monte-Carlo standard-error caveat — a named extension, never a stub.

Crucially, this exotic-leg aggregation is preserved end-to-end through the distributed fan-out: a firm book *including* exotic legs, partitioned across an HRW (highest-random-weight) fleet and re-aggregated through the cross-shard algebra, reconciles to the single-node firm aggregate — additive Greeks to ~1e-12 and the re-gathered non-additive VaR/ES and curvature to the same bit-level tolerance. The exotic seat survives sharding; the firm view is the same number whether computed on one node or fanned out across the fleet. *(`celnet-risk-cube/src/exotic.rs`; parity rows `celnet-parity/tests/exotic_risk_cube.rs` Gate A and `celnet-parity/tests/exotic_risk_cube.rs`; the cross-shard algebra lives in `celnet-risk-fleet`.)*

## 6.5 Regulatory capital — FRTB-SA (Standardised Approach)

On top of the position-fact cube, Celnet computes market-risk capital under the Basel **FRTB Standardised Approach** (BCBS *MAR21/22/23*) — the full Sensitivities-based Method (SbM), not a stand-in. Capital is built bottom-up from net sensitivities the cube already holds:

| Stage | Method (BCBS reference) |
|---|---|
| Weighted sensitivity | `WS_k = RW_k · s_k` — each net delta/vega scaled by its prescribed risk weight (MAR21.3). |
| Within bucket `K_b` | `K_b = √( max(0, Σ WS_k² + Σ_{k≠l} ρ_{kl} WS_k WS_l) )`, with the MAR21.4(3) non-negativity floor under stressed correlation. |
| Across buckets | `K = √( Σ K_b² + Σ_{b≠c} γ_{bc} S_b S_c )` with signed bucket sums `S_b`, and the MAR21.6 alternative `S_b = max(min(Σ WS_k, K_b), −K_b)` when the cross-bucket radicand goes negative. |
| Three correlation scenarios | The whole charge is recomputed under **HIGH** (`ρ,γ → min(1, 1.25·ρ)`), **MEDIUM** (as prescribed) and **LOW** (`ρ,γ → max(2·ρ−1, 0.75·ρ)`) correlations, and the capital is the **maximum** of the three — the defining SbM property (MAR21.6(1)). |
| Curvature | Per-bucket `K_b = max(K_b⁺, K_b⁻)` from the up/down reprice net of delta, with the `ρ²/γ²` curvature correlations and the chosen direction's `CVR` sign carried into the cross-bucket sum (MAR21.5.2). |
| RRAO | A flat add-on — **1.0%** of gross notional on exotic-underlying instruments plus **0.1%** on instruments carrying other residual risk (gap/digital, correlation, behavioural). This is the FRTB piece most relevant to an FX-exotics book: barriers, digitals, one-touches and TARFs are textbook RRAO (MAR23.4/.5). |
| DRC | For a pure deliverable-FX book the Default Risk Charge is a **documented, cited zero** (MAR22) — there is no issuer jump-to-default in deliverable FX — stated honestly rather than fabricated. |

The quadratic-form kernel `√(Σ WS² + ΣΣ ρ WS WS)` is factored once and reused for delta, vega and the cross-bucket step, so there is no duplicated formula and no opportunity for the three sites to drift. The `0.75ρ` LOW-correlation floor — material for FX where `γ = 0.6` must decorrelate to `0.45`, not `0.2` — is pinned to **hand-computed BCBS constants** in a dedicated test, after a prior wave caught a circular-oracle defect (a longhand check that had independently re-derived the *same* wrong floor). The whole SbM machinery is gated against a genuinely independent longhand oracle — every formula re-written from raw weighted sensitivities using only `f64` arithmetic and `f64::sqrt`, never calling back into the production code — agreeing to ~1e-10, with structural identities (a perfectly-hedged bucket gives `K_b = 0`; RRAO equals the exact hand-summed `Σ |notional|·weight`). *(`celnet-risk-cube/src/frtb.rs`; parity row `celnet-parity/tests/frtb.rs`.)*

## 6.6 Counterparty valuation adjustments — XVA

Celnet computes the counterparty-risk valuation adjustments — **CVA** (credit), **DVA** (debit/own-credit) and **FVA** (funding) — over a netting set of vanilla FX options. The pipeline is end-to-end and exact: an exposure simulation evolves spot under risk-neutral GBM on an exposure-date grid, driven by low-discrepancy **Sobol** normals from `celnet-qmc` (far lower exposure-profile variance than plain pseudo-random MC at the same path budget), reprices and **nets within the set** at each grid date, and reduces across paths to the expected positive/negative exposure profiles `EPE(t_k)` / `ENE(t_k)`; a piecewise-constant hazard-rate curve gives risk-neutral survival `S(t) = exp(−∫λ)`; and the discrete Basel/ISDA aggregation forms unilateral CVA `= LGD · Σ_k D(t_k)·EPE(t_k)·[S(t_{k−1}) − S(t_k)]`, the symmetric DVA over own-survival and ENE, and the funding adjustment FVA over the net expected exposure on the joint-survival measure. The arithmetic is validated against a hand-derived closed-form CVA in exact limits — agreeing to ~1e-9 with every intermediate constant **hand-pinned to offline-computed published values** (guarding against the engine and oracle sharing a mis-stated constant), monotone in hazard and LGD, zero at zero default probability, with DVA/FVA sign symmetry. *(`celnet-xva/src/{cva,exposure,netting,survival}.rs`; parity row `celnet-parity/tests/xva.rs`.)*

> **Honest boundary — XVA is internal-only.** The XVA engine has **no client or wire surface** (`cva`/`xva` appear nowhere in `celnet.proto`); it operates on **synthetic netting sets** of vanilla FX options under a self-contained risk-neutral GBM exposure model. It deliberately does **not** model live **CSAs / collateral / margin**, **wrong-way risk** (exposure–default correlation), or the live credit/funding-curve estate — those are deploy-/estate-gated and out of scope in-repo. The CVA/DVA/FVA arithmetic, survival mechanics and exposure simulation are all exact and parity-gated; the *integration* with live counterparty data is the deploy target.

---

## Honest boundary (carried into every claim above)

- **XVA (CVA/DVA/FVA, `celnet-xva`)** — internal-only, **no client/wire surface**, **synthetic netting sets only**; live CSAs / collateral / wrong-way risk are deploy-gated.
- **Cross-fleet risk fan-out** — the additive-merge / non-additive-re-gather **algebra** is built and reconciled fan-out == single-node to ~1e-12 on localhost multi-process; **physical cross-node risk transport, cross-host wire p99 and cross-DC** are deploy-gated.
- **MC-priced exotic legs** (Asian, TARF, accumulator, discrete lookback) carry a **price std-error** on their cube Greeks — never "machine-precision"; that bar is reserved for the analytic/closed-form (barrier, digital) and golden-gated products.

**See also:** [§8 Scalability & Scale-Out](08-scalability-scaleout.md) shows the cross-fleet risk fan-out reconciling to the single-node aggregate; [§9 API & Client Parity](09-api-contract-parity.md) is the RiskService contract these roll-ups are served over.

---
<sub>[← Prev: Extensibility](05-extensibility-plugins.md) · [Index](../CELNET-CAPABILITIES.md) · [Next: Performance & Latency →](07-performance-latency.md) · [Showcase ↗](../celnet-capabilities.html)</sub>
