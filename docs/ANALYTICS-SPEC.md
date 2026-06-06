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
- **Standard ladder (1W, 1M, ...)** added to spot, then adjusted by **modified-following** business-day rule and **end-of-month** rule.
- **Pre-spot short end:** **ON** (overnight) = next good business day after **today/horizon** (~T+1); **TN** (tom-next) = the good day after ON; **SN** (spot-next) = next good day after **spot**. ON/TN are anchored on horizon (not spot) — vol-time accrues from a `vol_anchor` of `horizon` for ON/TN (else `spot`) so the short end is never non-positive. (`celnet-calendar::fx`: `expiry_for_tenor(pair, horizon, spot, tenor)` carries both dates; see `docs/CONVENTIONS.md`.)
- **IMM dates:** the `n`-th 3rd-Wednesday of the Mar/Jun/Sep/Dec cycle strictly after horizon (CME-style, modified-following). **Broken dates:** an explicit civil expiry date, modified-following onto a good day.
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

### 3.3 SVI / SSVI / eSSVI — Gatheral (2004), Gatheral-Jacquier (2014), Hendriks-Martini (2019)

- **Raw SVI** total implied variance per slice: `w(k) = a + b{ rho(k - m) + sqrt((k - m)^2 + sigma^2) }` in log-moneyness `k` (5 params/slice, linear wings consistent with Lee's moment formula).
- **SSVI / Surface SVI:** single surface parametrized by ATM total variance `theta_t`, constant correlation `rho`, and a curvature function `phi(theta)`. Gives **explicit closed-form sufficient conditions for no butterfly arbitrage and no calendar-spread arbitrage** across the whole surface — the preferred choice for a globally arbitrage-free, smoothly interpolated **production surface** (needed for local-vol/exotics stripping).
- **eSSVI / Extended SSVI:** generalizes SSVI by making the correlation **maturity-dependent**, `rho -> rho(theta)`, while keeping the closed-form static no-arbitrage conditions. Each slice is `(theta, rho, psi)` with `psi = theta·phi(theta)` the ATM skew-scale (`d_k w|_{k=0} = rho·psi`) and `w(k) = (theta/2)·{ 1 + rho·(psi/theta)·k + sqrt(((psi/theta)·k + rho)^2 + (1 - rho^2)) }`. **Butterfly (per slice):** `psi·(1+|rho|) < 4` and `(psi^2/theta)·(1+|rho|) <= 4`. **Calendar (consecutive slices `theta_1 < theta_2`):** `psi_1 <= psi_2` and `|rho_2·psi_2 - rho_1·psi_1| <= psi_2 - psi_1`. **SSVI is the special case `rho(theta) == const`** with `psi = theta·phi`, which `celnet-surface::extended_surface` recovers **byte-for-byte**. (`crates/celnet-surface/src/extended_surface.rs`; closed-form predicates validated against the Breeden-Litzenberger density + pointwise calendar numerics in `crates/celnet-parity/tests/essvi.rs`.)

### 3.4 Arbitrage-free constraints (asserted on every surface)

1. **Butterfly (static):** risk-neutral density `g(k) >= 0`; equivalently call prices convex in strike (Gatheral g-function / Durrleman condition per slice).
2. **Calendar:** total variance `w(k,T) = sigma^2 T` non-decreasing in `T` at fixed log-moneyness (no crossing on a total-variance plot).
3. **Vertical:** call-spread monotonicity in strike.

VV is **not** arbitrage-free and can produce negative densities/crossing in the wings (especially high RR/BF, short tenors); when VV violates these, **fall back to / cross-check against SSVI or arbitrage-free PDE SABR**, and do not use raw VV for exotics or local-vol stripping without correction.

### 3.4a Smile-model selection (per-mark, callable)

The smile family used to calibrate a mark is **selectable on the wire** via `SmileModel` (the
vendor/method-neutral enum; the §3.1–3.3 provenance is documentation only):

| `SmileModel` | Calibration family (§) | Notes |
|---|---|---|
| `MarketHedge` (default) | Vanna-Volga (§3.1) | The byte-for-byte baseline; reprices the 3 broker quotes exactly. |
| `StochasticVol` | SABR (§3.2) | β=1 lognormal-FX; fits (α, ρ, ν) to ATM + 25Δ (+10Δ) anchors. |
| `Parametric` | SVI (§3.3) | fits (b, ρ, m, σ) with `a` ATM-pinned; step projected into the no-butterfly box. |
| `ParametricSurface` | SSVI (§3.3) | θ pinned to ATM total variance; (ρ, φ) under the closed-form butterfly conditions. |
| `ExtendedSurface` | eSSVI (§3.3) | θ pinned to ATM total variance; (ρ, ψ) under the closed-form (θ,ρ,ψ) butterfly conditions; maturity-dependent ρ across slices, SSVI byte-recovered at constant ρ. |

All four are deterministic (libm-only damped Gauss-Newton in `(log-moneyness, total-variance)`
space) and calibrate to the **same** VV-anchored ATM/25Δ(/10Δ) points, so model selection changes
the **wings**, never the ATM reprice. `SurfaceService.MarkSurface` routes the choice through
`celnet_surface::build_model_smile`, deposits the model-tagged `CalibratedSmile` under the returned
`surface_version` (so a pinned RFQ/RFS re-prices against the exact marked model), and echoes the
model used in `Smile.arbitrage.note` as `model=<family>` provenance. Absent ⇒ market-hedge default.

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
| Asian (geometric) | Closed form (lognormal product) | Analytic | **Built** — `celnet-exotics::mc::geometric_asian_price` (Kemna-Vorst, exact); the analytic `asian::geometric_average_price` re-derivation matches it to ~1e-12 (parity row `celnet-parity/tests/asian.rs`). |
| Asian (arithmetic) | No exact closed form | Moment-matching (**Turnbull-Wakeman / Levy / Curran**), PDE reduction (**Rogers-Shi / Vecer**), or MC with **geometric-Asian control variate** | **Built (analytic + MC)** — `celnet-exotics::asian`: two fast MC-free pricers, `turnbull_wakeman_price` (lognormal two-moment matching) and `curran_price` (geometric-conditioning, the more accurate estimator), both with continuous & discrete schedules and the **seasoned (in-progress-average)** case; plus the existing `mc::price_asian` (geometric-control-variate MC). **No exact closed form exists, so tolerances are honest:** *exact* in the single-observation (→ GK vanilla, ~1e-9) and zero-vol (→ discounted intrinsic, ~1e-9) limits; **Curran** tracks a converged independent MC to within a few reported standard errors; **Turnbull-Wakeman** is a two-moment **approximation** gated at its true ~1.5%-relative band (a bias, not noise); Curran ≤ TW error in the convex regime — all gated in `celnet-parity/tests/asian.rs`. Provenance (Turnbull-Wakeman 1991, Curran 1994) in module docs only. |
| Lookback (floating/fixed strike) | LSV (forward-vol sensitive) | MC (path max/min) / PDE | Pure LV mis-prices forward smile |
| Forward-start / cliquet | LSV (forward-smile sensitive) | MC / PDE | Strike set at future fixing; pure LV fails here |
| TARF / accumulator / decumulator | LV often sufficient for vanilla strips; **LSV** where forward-smile/barrier dynamics matter | **Monte Carlo** (low-dim PDE/quadrature for approximations) | Model **gap/digital risk** explicitly; stress target/knock-out; reserve for model risk; suitability/disclosure |
| Variance swap | Model-free static replication | **Log-contract**: continuum of OTM puts+calls weighted `1/K^2` + dynamic `1/S_t` position | Strike = model-free integral of OTM option prices. **Built** — `celnet-exotics::var_swap` (`fair_variance`): adaptive-wing, fixed-`u`-resolution Simpson strip of OTM **forward** values off any arbitrage-free `Smile`. Recovers `sigma^2` exactly for a flat smile; gated in `celnet-parity/tests/var_vol_swap.rs` against an independent adaptive-Simpson quadrature (~1e-6) and the flat closed form (~1e-6). |
| Volatility swap | **Not** statically replicable | Explicit **convexity adjustment** (Carr-Lee robust replication) or LSV-model expectation | Never set vol-swap strike = sqrt(var-swap strike). **Built** — `celnet-exotics::vol_swap` (`fair_volatility`): `K_vol = sqrt(K_var) - Var(v)/(8*K_var^{3/2})`, the Carr-Lee/Brockhaus-Long convexity (Jensen) adjustment from the log-contract-weighted variance-of-variance. Strictly `< sqrt(K_var)` for any non-degenerate smile; gated in the same parity row (strict bound + Var(v) monotone in butterfly). |

**Continuously-monitored barriers in MC** require a discrete-monitoring bias correction: (a) **Broadie-Glasserman-Kou** barrier shift by `~0.5826 sigma sqrt(dt)`, or (b) **Brownian-bridge** exit-probability between steps. Digital/barrier payoffs are discontinuous -> **Rannacher smoothing** in PDE and barrier-aligned grid nodes.

---

## 5. Advanced Models and Numerical Methods

### 5.1 Local Volatility — Dupire

Local vol `sigma(K,T)` from the **Dupire forward equation** (call-price derivatives), calibrating exactly to the vanilla surface. Well-suited to path-dependent strips (TARFs) but produces a **flat/unrealistic forward smile** and mis-prices forward-vol-sensitive and barrier/touch products. Strip Dupire local vol only from an **arbitrage-free** upstream smile (SSVI / arbitrage-free SABR / broker-fly-calibrated delta-space).

### 5.2 Heston

Variance is a CIR sqrt process: mean-reversion `kappa`, long-run `theta`, vol-of-vol `sigma`, correlation `rho`. Closed-form characteristic function (Riccati ODEs) priced via **Carr-Madan FFT**, **Lewis**, or the **COS (Fourier-cosine) method**; calibrate by nonlinear least squares with analytic gradients. **Feller condition** `2 kappa theta >= sigma^2` is frequently violated in FX, so use **full-truncation or Andersen QE** schemes that remain valid regardless. Heston alone cannot fit the short-dated smile and over-states long-dated convexity, so it is used as the **stochastic backbone inside LSV**, not standalone for exotics.

**Built — standalone European vanilla pricer `celnet-heston`** (Wave 4b). Two **genuinely independent** characteristic-function transforms of the *same* closed-form CF, cross-validated against each other:
- `carr_madan` — direct convergent numerical integration of the **Carr-Madan (1999)** damped-call Fourier integral by composite **Gauss-Legendre** quadrature (no external FFT crate; a single strike needs none, and an external dep would break determinism), with a decay-justified upper limit that covers both the Gaussian (small-`sigma`) and the slower **exponential** (large-`sigma`) tail of the integrand.
- `cos` — the **Fang-Oosterlee (2008) COS** method: cosine-series reconstruction of the risk-neutral density with the cumulant-based truncation range `[c1 +/- L*sqrt(c2 + sqrt(c4))]` (c4 from automatic higher-derivative extraction of the characteristic exponent, essential for heavy tails); the bounded-payoff (put) leg is priced directly and the call obtained by exact put-call parity.

The shared CF uses the **branch-cut-free Cui-del-Bano-Germano (2017)** formulation (no complex-log winding; the only complex transcendental is `exp`), with overflow-stable `e^{-dt}` factoring — strictly more robust than the original 1993 grouping or the Albrecher et al. (2007) "little Heston trap", and finite out to T=10. **FX dual-rate carry** `r_d, r_f`, both put and call.

Gated in `celnet-parity/tests/heston.rs`: (i) **Carr-Madan == COS** to `|diff| <= 1e-8 + 1e-7*price` across the **full practical FX-vanilla grid up to ~3y** — all strikes deep-ITM to deep-OTM, every parameter set including Feller-**violated** (worst non-tiny relative ~3e-8); (ii) **Black-Scholes limit** — as `sigma -> 0` with `v0 = theta`, BOTH transforms converge to the independent `celnet-vanilla` Garman-Kohlhagen price at `sqrt(theta)` at the genuine **O(sigma^2)** rate (rho=0; honestly O(sigma) for rho != 0), verified by the err(sigma/10) ~ err(sigma)/100 ratio; (iii) **put-call parity** to 1e-10 and call monotone-non-increasing in strike. **HONEST BOUNDARY:** beyond ~3y the deep-OTM Fourier precision wall makes the two transforms diverge for the farthest wings (intrinsic to *both* methods, not a defect of either) — the tight cross-method claim is asserted on the <=3y regime; far-dated deep-OTM is the province of PDE/MC engines, and Heston remains the **stochastic backbone inside LSV** (sec 5.3) rather than a standalone exotics pricer. Published anchor: the Albrecher (2007) Little-Heston-Trap set reprices the ATM 1y call to the literature ~5.785.

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

Crate homes in the **implemented** 19-crate tree (this section was written against an early
all-in-`celnet-core` sketch; the real homes are): pure math primitives (libm-routed
transcendentals, `is_close`, `Smile` trait) in **`celnet-core`**; POD/convention/config types
in **`celnet-types`**; the **calendar/date engine** in **`celnet-calendar`**; the convention
registry in **`celnet-conventions`**; Garman-Kohlhagen + Greeks + strike↔delta solver in
**`celnet-vanilla`**; **surface construction (VV/SABR/SVI/SSVI + arbitrage gates + term
structure)** in **`celnet-surface`**; **exotics, LSV, PDE and MC numerics** in
**`celnet-exotics`**. GPU back-ends sit behind the `PricingBackend` trait in **`celnet-gpu`**
(wgpu/WGSL — *not* CubeCL). User-supplied models arrive via **`celnet-plugin-api`** (WIT world
+ traits, built); the Wasm host **`celnet-plugin-host`** is built on **wasmi** (fuel-metered,
deterministic; wasmtime rejected for advisories — see `docs/PLUGIN-HOST-ALT.md`).

> The per-item `Crate` columns below retain the original sketch's `celnet-core` labels for
> P0–P2 traceability; read them through the mapping above (calendar → `celnet-calendar`,
> surface/smiles → `celnet-surface`, exotics/LSV/PDE/MC → `celnet-exotics`, GPU → `celnet-gpu`).
> **Implemented-status note (vs spec):** the MC engine ships **Philox** pseudo-random paths,
> the **BGK** barrier shift and control variates today; **Sobol' QMC + Brownian-bridge path
> construction are designed but not yet implemented** (so wherever §3–§5/§6 cite Sobol or
> Brownian-bridge they describe the target method, not current code). The GPU backend is
> **wgpu/WGSL**, not `CubeClBackend`.

### P0 — Foundational vanilla + surface (must ship first)

| Item | Crate |
|---|---|
| Convention record types: delta type (4), ATM (ATMF/DNS), premium ccy + pips/percent, spot lag, cut, day counts, settlement style — per (pair, tenor) | `celnet-types` |
| Calendar engine: dual-calendar (+USD) intersection, horizon->spot, expiry->delivery, modified-following, end-of-month, NY/Tokyo cut | `celnet-core` |
| Garman-Kohlhagen vanilla pricer off `F`, `DF_d`, `DF_f` (call/put) | `celnet-core` |
| Full Greek set: delta (per convention), vega (+ tenor-bucketed ladder), theta, **rho-domestic AND rho-foreign**, gamma, vanna, volga, charm, speed, zomma, color | `celnet-core` |
| Strike<->delta root-finder (Brent/Newton), premium-adjusted non-monotone branch handling | `celnet-core` |
| Vanna-Volga (Castagna-Mercurio 2nd approximation) baseline smile | `celnet-core` |
| Broker(market)-strangle -> smile-strangle calibration (Reiswich-Wystup / Clark); 5-point delta-space smile | `celnet-core` |
| Delta-space interpolation + term-structure (ATM/RR/BF separately, total-variance / business-time, event weighting) | `celnet-core` |
| Arbitrage checks: butterfly (density >= 0), calendar (non-decreasing total variance), vertical | `celnet-core` |
| Deliverable settlement; trait-object `PricingModel` registry for first-party models | `celnet-core` |

### P1 — Production surface + first-generation exotics + NDF

| Item | Crate |
|---|---|
| SSVI surface (Gatheral-Jacquier) with closed-form no-arbitrage conditions; arbitrage-free production surface for stripping | `celnet-core` |
| SABR (Hagan 2002 lognormal expansion; fixed beta) + arbitrage-free PDE SABR (Hagan 2014) for wings/low-rate | `celnet-core` |
| Dupire local vol from arbitrage-free smile | `celnet-core` |
| European digitals, one-touch / no-touch / DNT via Reiner-Rubinstein + VV survival-probability weighting | `celnet-core` |
| Single/double barriers (KO/KI): Crank-Nicolson + Rannacher PDE, log-spot, barrier-aligned nodes | `celnet-core` |
| Monte Carlo engine: Sobol' QMC + Brownian-bridge ordering, control variates, BGK / Brownian-bridge barrier correction | `celnet-core` (engine), `PricingBackend` for GPU |
| NDF/NDO cash settlement at named fixing (EMTA/WMR/central-bank), fixing-to-settlement lag, settlement-curve discounting | `celnet-types` (config) + `celnet-core` (logic) |
| `celnet-plugin-api` WIT world + `celnet-plugin-host` **wasmi** embedding (core modules, no-WASI capability linker) for user vol models / payoffs (fuel-metered, deterministic) | `celnet-plugin-api`, `celnet-plugin-host` |

### P2 — Full exotics + advanced models + GPU

| Item | Crate |
|---|---|
| Heston (FFT / COS / Lewis), QE / full-truncation simulation, NLS calibration | `celnet-core` |
| LSV: Heston backbone + Dupire leverage, particle-method calibration (Gyongy / Markovian projection), mixing weight `eta` | `celnet-core` |
| Window/partial barriers (time-dependent BC PDE; piecewise-constant Heston) | `celnet-core` |
| Asians: geometric closed form; arithmetic via Turnbull-Wakeman / Levy / Curran, Rogers-Shi / Vecer, MC + control variate | `celnet-core` |
| Lookbacks, forward-start / cliquet (LSV via MC/PDE) | `celnet-core` |
| TARF / accumulator / decumulator: MC under LV/LSV, explicit gap/digital-risk modelling + reserves | `celnet-core` |
| Variance swap (log-contract `1/K^2` replication); volatility swap (Carr-Lee convexity adjustment) | `celnet-core` |
| GPU exotics back-end: **wgpu/WGSL** kernels (Metal/Vulkan/GLES/DX12), f32 GPU numerics with f64 CPU reconciliation, Philox-4x32-10 counter-based RNG (Sobol direction numbers + Brownian bridge are deferred QMC work) | `celnet-gpu` `PricingBackend` impls (wgpu GPU / CPU) |

### Cross-cutting validation (all phases)

Validate against **Reiswich-Wystup (2010)** worked examples and **Iain Clark** reference numbers for each delta/ATM convention before trusting marks; cross-validate prices against **QuantLib** golden tables (version-pinned) and published benchmark prices; assert financial invariants (put-call parity, monotonicity, butterfly/calendar/vertical no-arbitrage) as property tests with explicit ULP/relative tolerances.