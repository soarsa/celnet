# Celnet FX-Options Analytics Specification

**Status:** Market-standard baseline as of May 2026
**Scope:** Definitive specification of the FX-options analytics Celnet implements. Every convention, model, and numerical method below is the documented market standard for institutional FX-options desks. No proprietary or speculative methods are introduced.

---

## 0. Notation and Core Quantities

For a currency pair **CCY1CCY2** (e.g. EURUSD):
- **FOR** = foreign = asset = base = CCY1 (EUR). Its rate is `r_f`.
- **DOM** = domestic = numeraire = quote = CCY2 (USD). Its rate is `r_d`.
- `S` = spot (DOM per 1 FOR), `K` = strike, `T` = time to expiry (vol time), `sigma` = implied vol.
- Forward: `F = S e^{(r_d - r_f) T}`. Discount factors are stored **separately** as `DF_d = e^{-r_d T}` and `DF_f = e^{-r_f T}` (not as continuous rates), so deliverable, NDO, and dual-curve discounting plug in cleanly.
- `phi(.)` = standard normal pdf, `N(.)` = standard normal cdf.

---

## 1. Conventions

Conventions are **first-class per-(pair, tenor) configuration**, never global hardcoded defaults. USDJPY, EURUSD, and EM NDFs differ materially. The convention record for each (pair, tenor) carries:

```
{ delta_type, atm_type, premium_ccy, premium_style, spot_lag, cut, day_count_vol, day_count_accrual_FOR, day_count_accrual_DOM, settlement_style }
```

### 1.1 Delta conventions (four)

Implied vols are always quoted against **GK delta, not strike**, so strike<->delta conversion is a root-find (see §3.5).

| Convention | Call delta formula |
|---|---|
| (1) Spot delta, unadjusted | `e^{-r_f T} N(d1)` |
| (2) Forward delta, unadjusted | `N(d1)` |
| (3) Premium-adjusted spot delta | `e^{-r_f T} (K/F) N(d2)` |
| (4) Premium-adjusted forward delta | `(K/F) N(d2)` |

- Choice is **by pair AND tenor**: short tenors typically use spot delta; long tenors (>~1–2Y) use forward/driftless delta. Example: USDJPY <= 1Y uses spot premium-adjusted delta.
- **Premium-adjusted vs unadjusted:** when premium is paid in the **FOR** currency (e.g. EURUSD premium in EUR), the premium itself carries FX risk, so the hedge delta is reduced by the premium's delta -> premium-adjusted. When premium is in **DOM** (the numeraire), no adjustment -> unadjusted/raw delta.

### 1.2 Premium conventions

Four premium quotation styles exist; convention is pair-specific and determines whether delta must be premium-adjusted:
1. **Domestic pips** (price per 1 unit FOR, in DOM — e.g. USD pips per EUR for EURUSD)
2. **%FOR** (percentage of foreign notional)
3. **%DOM** (percentage of domestic notional)
4. **Foreign pips**

### 1.3 ATM conventions

Pair-specific:
- **ATM-Forward (ATMF):** `K = F`. This is the delta-neutral point only under unadjusted forward delta.
- **Delta-Neutral Straddle (DNS):** strike where call delta + put delta = 0, so a straddle has zero delta. **DNS is the dominant interbank ATM for most G10/EM.**
  - Under unadjusted forward delta: `K_DNS = F exp(+0.5 sigma^2 T)`.
  - Under premium-adjusted delta: `K_DNS = F exp(-0.5 sigma^2 T)` (**below** the forward — opposite sign; mislocating this corrupts the whole surface).

### 1.4 Risk reversal / butterfly quoting

Per-tenor smile quotes: ATM vol, plus 25-delta (and 10-delta for liquid pairs) RR and BF, giving a 5-point smile (ATM, 25dC/P, 10dC/P):
- `RR_25 = sigma(25d call) - sigma(25d put)` (skew); also `RR_10`.
- `BF_25 = 0.5*(sigma(25d call) + sigma(25d put)) - sigma_ATM` (convexity); also `BF_10`.

**CRITICAL — broker (market) butterfly vs smile butterfly:** brokers actually trade and quote the **broker/market strangle**, which applies a **single vol** (`sigma_ATM + BF_broker`) to **both** the 25d-call and 25d-put strikes and must reprice to the same value as the smile. The **smile butterfly** is the arithmetic-average convexity used to read the smile. The two differ — sometimes materially for high-RR/EM pairs. Recovering the smile requires an explicit **calibration step** (Reiswich-Wystup 2010, Clark 2011): solve for the 25d/10d call/put strikes+vols such that the broker (market) strangle reprices. **Never treat the quoted BF as the arithmetic 25d smile-strangle** — this is the #1 production bug; it silently biases the wings and breaks 10-delta reproduction.

### 1.5 Date / cut / delivery conventions

Driven by a calendar engine intersecting **both** currency calendars (plus USD for cross-via-USD pairs):
- **Horizon (trade/today) -> Spot date:** T+2 for most pairs; **T+1 for USDCAD/USDTRY/USDRUB/USDPHP** and same-region pairs.
- **Expiry date -> Delivery/settlement date:** computed from expiry by the **same** spot-lag rule (delivery = spot date relative to expiry).
- Tenors (1W, 1M, ...) added to spot, then adjusted by **modified-following** business-day rule and **end-of-month** rule.
- **Cut (expiry/fixing time):** **NY cut = 10:00 AM New York** (the standard interbank OTC cut; CME FX option fix is a 60s VWAP ending 10:00:00 ET). **Tokyo cut = 15:00 Tokyo (3pm JST)** for JPY-region/Asian business. Parameterize per pair.
- **Day count:** vol/time-to-expiry uses **ACT/365** (calendar days / 365). Interest accrual uses each currency's money-market basis (**ACT/360** for USD/EUR, **ACT/365** for GBP/AUD). Keep **vol time** and **settlement-discounting time** distinct.

### 1.6 Deliverable vs NDF/NDO

- **Deliverable:** exchange full notional of both currencies on the delivery date (physical).
- **Non-Deliverable Options (NDO)** on NDF pairs (USDKRW, USDTWD, USDINR, BRL, etc.): cash-settle in the convertible currency (usually USD) at a published **fixing** (e.g. KFTC18 / WMR / EMTA / central-bank fixings) on the fixing date 1–2 days before settlement, with the fixing-to-settlement lag; discount the cash payoff on the settlement-currency curve. The pricing vol surface is still GK; only settlement/delivery logic differs. NDF is the forward analogue.

---

## 2. Vanilla Pricing — Garman-Kohlhagen (1983)

GK is the market-standard vanilla FX model: BSM with the foreign rate `r_f` acting as a continuous dividend yield on the foreign-currency asset. The core is computed off the forward `F` and the two discount factors.

**Price (call):**
```
C = S e^{-r_f T} N(d1) - K e^{-r_d T} N(d2)
P = K e^{-r_d T} N(-d2) - S e^{-r_f T} N(-d1)
d1 = [ln(S/K) + (r_d - r_f + sigma^2/2) T] / (sigma sqrt(T))
d2 = d1 - sigma sqrt(T)
```
Equivalently price off `F = S e^{(r_d - r_f)T}` via the **Black-76** form discounting at `e^{-r_d T}`.

### 2.1 Full Greek set (exact definitions)

**First-order:**
- **Delta** — per the configured convention (spot/forward × adjusted/unadjusted), §1.1.
- **Vega** = `S e^{-r_f T} sqrt(T) phi(d1)` (reported per 1 vol point, i.e. /100). Also reported as a **vega ladder bucketed by tenor**.
- **Theta** — time decay.
- **Rho-domestic** = `dV/dr_d` = `K T e^{-r_d T} N(d2)` for a call.
- **Rho-foreign** = `dV/dr_f` = `-S T e^{-r_f T} N(d1)` for a call.
- **FX has TWO rhos** — a single equity-style "rho" is meaningless and must not be exposed.

**Second-order:**
- **Gamma** = `e^{-r_f T} phi(d1) / (S sigma sqrt(T))`.
- **Vanna** = `dDelta/dsigma = d^2V/(dS dsigma)` = `-e^{-r_f T} phi(d1) d2 / sigma` (skew/RR sensitivity).
- **Volga (vomma)** = `d^2V/dsigma^2` = `vega * d1 d2 / sigma` (convexity/BF sensitivity).
- Vanna and volga are the **core vanna-volga Greeks**.

**Higher-order:**
- **Charm** = `dDelta/dt` (delta decay / "delta bleed").
- **Speed** = `d^3V/dS^3` (rate of change of gamma w.r.t. spot).
- **Zomma** = `dGamma/dsigma = d^3V/(dS^2 dsigma)`.
- **Color** = `dGamma/dt` (gamma decay).

---

## 3. Volatility Surface

The FX surface lives in **delta space** (not strike space), under a **sticky-delta** assumption, built per-tenor from ATM + 25d RR/BF (+ 10d for liquid pairs). A fixed delta maps to widely different strikes across tenors, giving better coverage and matching sticky-delta market behavior.

### 3.1 Vanna-Volga (baseline) — Castagna-Mercurio (2007)

The FX-native, fast, closed-form smile engine that exactly reprices the 3 quotes. It builds a locally-replicating, BS-vega-neutral portfolio of the three benchmark options (ATM, RR, BF) that zeroes the BS **vega, vanna, and volga** of the target option, then adds the **market cost of that hedge** to the flat-vol BS price. Implement the **Castagna-Mercurio second approximation** (asymptotically constant at extreme strikes). VV needs **no numerical calibration** and extrapolates beyond the 25d wings.

### 3.2 SABR — Hagan et al. (2002)

Stochastic-vol smile: `dF = alpha F^beta dW1`, `d(alpha) = nu alpha dW2`, `corr = rho`. Parameters: `alpha` (level), `beta` (CEV backbone exponent), `rho` (skew), `nu` (vol-of-vol/curvature). Use **Hagan's singular-perturbation lognormal (Black) implied-vol expansion** for speed. **Fix beta** by market convention/backbone (it is weakly identified / confounded with rho from a single smile). Switch to **arbitrage-free PDE SABR (Hagan 2014)** — the 1-D effective-forward density PDE — for low-vol/low-rate or deep-wing strikes where the asymptotic formula is inaccurate/arbitrageable (validity needs `nu sqrt(T)`, `|beta-1| sqrt(T)` small). Use **shifted/normal SABR** only if forwards approach or cross zero.

### 3.3 SVI / SSVI — Gatheral (2004), Gatheral-Jacquier (2014)

- **Raw SVI** total implied variance per slice: `w(k) = a + b{ rho(k - m) + sqrt((k - m)^2 + sigma^2) }` in log-moneyness `k` (5 params/slice, linear wings consistent with Lee's moment formula).
- **SSVI / Surface SVI:** single surface parametrized by ATM total variance `theta_t`, constant correlation `rho`, and a curvature function `phi(theta)`. Gives **explicit closed-form sufficient conditions for no butterfly arbitrage and no calendar-spread arbitrage** across the whole surface — the preferred choice for a globally arbitrage-free, smoothly interpolated **production surface** (needed for local-vol/exotics stripping).

### 3.4 Arbitrage-free constraints (asserted on every surface)

1. **Butterfly (static):** risk-neutral density `g(k) >= 0`; equivalently call prices convex in strike (Gatheral g-function / Durrleman condition per slice).
2. **Calendar:** total variance `w(k,T) = sigma^2 T` non-decreasing in `T` at fixed log-moneyness (no crossing on a total-variance plot).
3. **Vertical:** call-spread monotonicity in strike.

VV is **not** arbitrage-free and can produce negative densities/crossing in the wings (especially high RR/BF, short tenors); when VV violates these, **fall back to / cross-check against SSVI or arbitrage-free PDE SABR**, and do not use raw VV for exotics or local-vol stripping without correction.

### 3.5 Delta-space interpolation and strike<->delta conversion

- Interpolate in **delta space**. `delta -> strike` is nonlinear and vol-dependent (GK), solved with a robust **Brent/Newton** root-finder that respects the configured delta convention and uses the smile vol at the trial strike.
- **Premium-adjusted call delta is non-monotonic in strike** (has a maximum-delta strike / two solutions) — handle this branch explicitly: bracket on the correct branch and cap at the delta max.

### 3.6 Term-structure interpolation

Interpolate **ATM, RR, and BF separately** across tenors in **total-variance / business-time** terms, with **weekend/holiday and scheduled-event weighting** (central-bank meetings, fixings), then reconstruct the smile per target date. Total-variance interpolation preserves calendar no-arbitrage far better than interpolating vols directly; independent per-tenor RR/BF interpolation is a known cause of crossed total-variance curves (calendar arbitrage) — detect and remove with SVI/SSVI constraints.

---

## 4. Exotics Catalogue — Model + Numerical Method per Product

**Two-tier architecture:** (a) a fast **Vanna-Volga** engine for indicative quotes / risk on first-generation exotics; (b) a calibrated **LSV** engine for booking, hedging Greeks, and all path-dependent / second-generation products. Validate prices across methods (VV vs LSV-PDE vs LSV-MC).

| Product | Primary model | Numerical method | Notes |
|---|---|---|---|
| European digital | Closed-form GK + VV smile cost | Analytic; VV correction | Reprice vs arbitrage-free smile between 25d wings |
| One-touch / no-touch | VV (indicative) / LSV (booking) | Reiner-Rubinstein reflection closed form + VV **survival-probability** weighting; PDE/MC for LSV | VV scaled by no-touch (survival) probability `p`; first-exit-time/symmetry dampers |
| DNT (double no-touch) | VV (indicative) / LSV | VV + survival weighting; ADI PDE / MC | Reconcile to broker DNT quotes (SV mixing-weight calibration target) |
| Single / double barrier (KO/KI) | VV (indicative) / LSV (booking & Greeks) | Crank-Nicolson + **Rannacher** start-up; **ADI (Craig-Sneyd / Hundsdorfer-Verwer)** for 2D LSV; nodes on barrier, log-spot grid | MC needs Brownian-bridge / BGK barrier correction |
| Window / partial barrier | LSV / Heston (piecewise-constant) | PDE with time-dependent boundary conditions (per-window) or MC | Multi-window, per-window rebates |
| Asian (geometric) | Closed form (lognormal product) | Analytic | — |
| Asian (arithmetic) | No exact closed form | Moment-matching (**Turnbull-Wakeman / Levy / Curran**), PDE reduction (**Rogers-Shi / Vecer**), or MC with **geometric-Asian control variate** | — |
| Lookback (floating/fixed strike) | LSV (forward-vol sensitive) | MC (path max/min) / PDE | Pure LV mis-prices forward smile |
| Forward-start / cliquet | LSV (forward-smile sensitive) | MC / PDE | Strike set at future fixing; pure LV fails here |
| TARF / accumulator / decumulator | LV often sufficient for vanilla strips; **LSV** where forward-smile/barrier dynamics matter | **Monte Carlo** (low-dim PDE/quadrature for approximations) | Model **gap/digital risk** explicitly; stress target/knock-out; reserve for model risk; suitability/disclosure |
| Variance swap | Model-free static replication | **Log-contract**: continuum of OTM puts+calls weighted `1/K^2` + dynamic `1/S_t` position | Strike = model-free integral of OTM option prices |
| Volatility swap | **Not** statically replicable | Explicit **convexity adjustment** (Carr-Lee robust replication) or LSV-model expectation | Never set vol-swap strike = sqrt(var-swap strike) |

**Continuously-monitored barriers in MC** require a discrete-monitoring bias correction: (a) **Broadie-Glasserman-Kou** barrier shift by `~0.5826 sigma sqrt(dt)`, or (b) **Brownian-bridge** exit-probability between steps. Digital/barrier payoffs are discontinuous -> **Rannacher smoothing** in PDE and barrier-aligned grid nodes.

---

## 5. Advanced Models and Numerical Methods

### 5.1 Local Volatility — Dupire

Local vol `sigma(K,T)` from the **Dupire forward equation** (call-price derivatives), calibrating exactly to the vanilla surface. Well-suited to path-dependent strips (TARFs) but produces a **flat/unrealistic forward smile** and mis-prices forward-vol-sensitive and barrier/touch products. Strip Dupire local vol only from an **arbitrage-free** upstream smile (SSVI / arbitrage-free SABR / broker-fly-calibrated delta-space).

### 5.2 Heston

Variance is a CIR sqrt process: mean-reversion `kappa`, long-run `theta`, vol-of-vol `sigma`, correlation `rho`. Closed-form characteristic function (Riccati ODEs) priced via **Carr-Madan FFT**, **Lewis**, or the **COS (Fourier-cosine) method**; calibrate by nonlinear least squares with analytic gradients. **Feller condition** `2 kappa theta >= sigma^2` is frequently violated in FX, so use **full-truncation or Andersen QE** schemes that remain valid regardless. Heston alone cannot fit the short-dated smile and over-states long-dated convexity, so it is used as the **stochastic backbone inside LSV**, not standalone for exotics.

### 5.3 Local-Stochastic Volatility (LSV / SLV) — production standard

`dS = ... + L(S,t) sqrt(v) S dW`, with leverage `L(S,t)` multiplying a Heston-type stochastic variance. Calibrates exactly to vanillas via leverage while retaining realistic forward-smile / spot-vol dynamics via the SV backbone.

- **Calibration:** Gyongy theorem / Markovian projection: `L(S,t)^2 = sigma_Dupire(S,t)^2 / E[v_t | S_t = S]`. Estimate the conditional expectation `E[v|S]` by the **particle method (Guyon-Henry-Labordère)** — kernel/regression estimator over a simulated particle cloud (McKean-Vlasov SDE solved forward), particle counts `~10^5–10^6`, with kernel-bandwidth tuning, regularization, and careful initial time-stepping (E[v|S] is unstable in low-density regions / `t->0`, risking leverage blow-ups).
- **Mixing weight `eta`** blends LV (`eta=0`) vs full SV (`eta=1`) on top of leverage; tune to market **barrier/touch and forward-smile** quotes since vanillas match for any `eta`.

### 5.4 Numerical methods

- **Analytic / quasi-closed-form:** GK vanillas, Reiner-Rubinstein barriers/touches, geometric Asians, Heston char-function (FFT/COS).
- **PDE finite-difference:** **Crank-Nicolson with Rannacher time-stepping** (implicit start-up steps to damp digital/barrier oscillations); **ADI Craig-Sneyd / Hundsdorfer-Verwer** for the 2D Heston/LSV PDE; **log-spot grids** with **barrier-aligned nodes**. Best for low-dimension barriers, digitals, window barriers.
- **Monte Carlo:** **Andersen QE** discretization for the variance process; **Sobol' quasi-MC** with **Brownian-bridge (or PCA) path construction/dimension ordering** to concentrate variance in the first dimensions; **geometric-Asian / vanilla control variates**; **Brownian-bridge or BGK** continuity correction for continuously-monitored barriers; **scrambled (randomized) Sobol'** for confidence intervals (QMC has no simple variance estimate). Best for TARFs, accumulators, Asians, lookbacks, multi-window/multi-asset products.

---

## 6. Prioritized Implementation Order (P0 / P1 / P2) by Workspace Crate

Crates per the Celnet flat virtual Cargo workspace: pure-domain math lives in **`celer-core`**; surface construction is a **`celer-core`** module; calibration solvers in **`celer-core`**; convention/config types in **`celer-types`**; the calendar engine in **`celer-core`** (date logic) with holiday data in **`celer-types`**. GPU exotics back-ends sit behind the `PricingBackend` trait. User-supplied models arrive via **`celer-plugin-api`** (WIT world) / **`celer-plugin-host`** (wasmtime).

### P0 — Foundational vanilla + surface (must ship first)

| Item | Crate |
|---|---|
| Convention record types: delta type (4), ATM (ATMF/DNS), premium ccy + pips/percent, spot lag, cut, day counts, settlement style — per (pair, tenor) | `celer-types` |
| Calendar engine: dual-calendar (+USD) intersection, horizon->spot, expiry->delivery, modified-following, end-of-month, NY/Tokyo cut | `celer-core` |
| Garman-Kohlhagen vanilla pricer off `F`, `DF_d`, `DF_f` (call/put) | `celer-core` |
| Full Greek set: delta (per convention), vega (+ tenor-bucketed ladder), theta, **rho-domestic AND rho-foreign**, gamma, vanna, volga, charm, speed, zomma, color | `celer-core` |
| Strike<->delta root-finder (Brent/Newton), premium-adjusted non-monotone branch handling | `celer-core` |
| Vanna-Volga (Castagna-Mercurio 2nd approximation) baseline smile | `celer-core` |
| Broker(market)-strangle -> smile-strangle calibration (Reiswich-Wystup / Clark); 5-point delta-space smile | `celer-core` |
| Delta-space interpolation + term-structure (ATM/RR/BF separately, total-variance / business-time, event weighting) | `celer-core` |
| Arbitrage checks: butterfly (density >= 0), calendar (non-decreasing total variance), vertical | `celer-core` |
| Deliverable settlement; trait-object `PricingModel` registry for first-party models | `celer-core` |

### P1 — Production surface + first-generation exotics + NDF

| Item | Crate |
|---|---|
| SSVI surface (Gatheral-Jacquier) with closed-form no-arbitrage conditions; arbitrage-free production surface for stripping | `celer-core` |
| SABR (Hagan 2002 lognormal expansion; fixed beta) + arbitrage-free PDE SABR (Hagan 2014) for wings/low-rate | `celer-core` |
| Dupire local vol from arbitrage-free smile | `celer-core` |
| European digitals, one-touch / no-touch / DNT via Reiner-Rubinstein + VV survival-probability weighting | `celer-core` |
| Single/double barriers (KO/KI): Crank-Nicolson + Rannacher PDE, log-spot, barrier-aligned nodes | `celer-core` |
| Monte Carlo engine: Sobol' QMC + Brownian-bridge ordering, control variates, BGK / Brownian-bridge barrier correction | `celer-core` (engine), `PricingBackend` for GPU |
| NDF/NDO cash settlement at named fixing (EMTA/WMR/central-bank), fixing-to-settlement lag, settlement-curve discounting | `celer-types` (config) + `celer-core` (logic) |
| `celer-plugin-api` WIT world + `celer-plugin-host` wasmtime embedding for user vol models / payoffs (fuel-metered, deterministic) | `celer-plugin-api`, `celer-plugin-host` |

### P2 — Full exotics + advanced models + GPU

| Item | Crate |
|---|---|
| Heston (FFT / COS / Lewis), QE / full-truncation simulation, NLS calibration | `celer-core` |
| LSV: Heston backbone + Dupire leverage, particle-method calibration (Gyongy / Markovian projection), mixing weight `eta` | `celer-core` |
| Window/partial barriers (time-dependent BC PDE; piecewise-constant Heston) | `celer-core` |
| Asians: geometric closed form; arithmetic via Turnbull-Wakeman / Levy / Curran, Rogers-Shi / Vecer, MC + control variate | `celer-core` |
| Lookbacks, forward-start / cliquet (LSV via MC/PDE) | `celer-core` |
| TARF / accumulator / decumulator: MC under LV/LSV, explicit gap/digital-risk modelling + reserves | `celer-core` |
| Variance swap (log-contract `1/K^2` replication); volatility swap (Carr-Lee convexity adjustment) | `celer-core` |
| GPU exotics back-end: CubeCL `#[cube]` kernels (CUDA/Metal/Vulkan/WGSL/CPU), f32 GPU numerics with f64 CPU reconciliation, Philox-4x32-10 counter-based RNG, Sobol direction numbers + Brownian bridge | `PricingBackend` impls (`CubeClBackend` / `CpuBackend`) |

### Cross-cutting validation (all phases)

Validate against **Reiswich-Wystup (2010)** worked examples and **Iain Clark** reference numbers for each delta/ATM convention before trusting marks; cross-validate prices against **QuantLib** golden tables (version-pinned) and published benchmark prices; assert financial invariants (put-call parity, monotonicity, butterfly/calendar/vertical no-arbitrage) as property tests with explicit ULP/relative tolerances.