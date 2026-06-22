# Fixed-Income Research Findings — Pass 1 (P0 Foundation)

**Status:** Research findings (first pass) · 2026-06-22 · Branch `feature/fixedincome`
**Executor:** deep-research agent · **Charter:** [`../fixed-income/FIXED-INCOME-RESEARCH-BRIEF.md`](../fixed-income/FIXED-INCOME-RESEARCH-BRIEF.md)
**Web access:** AVAILABLE (WebSearch + WebFetch). Every non-obvious claim carries a source URL or is
tagged `UNVERIFIED — knowledge-based, confirm`.

> **Convention for this doc.** Confidence = high/med/low. "high" = corroborated by a primary
> standards/CME/Fed source or multiple independent sources. Closed-form quant identities that are
> textbook-standard but whose *exact* primary-source page I could not extract from a binary PDF are
> marked **high (textbook-standard)** with the closest cited source. Method/person names (Hagan,
> Henrard, Bachelier, Hull-White) appear here only as *provenance* — per guardrail §3.2 they must
> never enter `celnet-rates` identifiers; they live in doc comments only.

## Scope of this pass

This is the **P0 foundation only**, depth over breadth: (1) multi-curve construction, (2) linear
rates products, (3) conventions & standards as a config-schema sketch, (4) the deterministic
curve/numerics needed, (5) the OSS reference-engine landscape **as oracles** (with the load-bearing
license verdicts for `cargo-deny`), and (6) a per-product anti-circular oracle plan. Vol/optionality
(SABR/Bachelier/Hull-White), credit/CDS, inflation, and bond static-data breadth are **flagged for
later passes**, not designed here.

The single most consequential finding for the guardrails: **of the four candidate OSS reference
engines, only QuantLib and ORE are actually permissively licensed. `rateslib` is source-available
non-commercial (NOT open source) and `FinancePy` is GPL-3.0 — both are `cargo-deny`/runtime traps and
must be used, if at all, only as out-of-process disposable oracle scripts, never as dependencies.**

---

## 1. Multi-curve construction (RFR/OIS discounting + projection)

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|1.1|The post-LIBOR world is **multi-curve**: one OIS/RFR curve discounts, separate per-tenor curves project forwards.|Collateralised derivatives discount on the OIS (RFR) curve; forward fixings come from a distinct projection curve per index tenor (1M/3M/6M). A single curve no longer prices both legs consistently.|[risk.net](https://www.risk.net/insight/risk-management/7957765/calibrating-interest-rate-curves-for-a-new-era), [arxiv 1006.4767 (Bianchetti/Mercurio-style multi-curve)](https://arxiv.org/pdf/1006.4767)|high|
|1.2|SOFR curve build mirrors the old Fed-Funds/LIBOR bootstrap but with extra structure.|Instruments: overnight rate, SOFR futures (1M/3M, with **convexity adjustment**), SOFR OIS swaps (fixed vs compounded SOFR), and EFFR/SOFR basis swaps for the funds curve. CME has cleared SOFR swaps since Oct 2018.|[Mathema SOFR curve methodology](https://help.mathema.com.cn/latest/docs/fixedincome/sofr_curve), [Quantifi](https://www.quantifisolutions.com/tackling-interest-rate-curve-construction-complexity/)|high|
|1.3|SOFR calibration is **harder** than EFFR/LIBOR.|Daily averaging, retrospective (in-arrears) payments, geometric compounding, and the need to splice a **historical realised** segment with a **projected** segment when calibrating to futures.|[Mathema](https://help.mathema.com.cn/latest/docs/fixedincome/sofr_curve), [Quantifi](https://www.quantifisolutions.com/tackling-interest-rate-curve-construction-complexity/)|high|
|1.4|Multi-curve calibration has a **circular dependency** ⇒ simultaneous solve.|When discount and projection curves are co-dependent (e.g. basis swaps reference one to price the other), a single global solver across both curves is required; sequential bootstrap is only exact when the dependency graph is acyclic (e.g. pure SOFR-discount + SOFR-projection).|[Quantifi](https://www.quantifisolutions.com/tackling-interest-rate-curve-construction-complexity/)|high|
|1.5|**Bootstrap vs global calibration** are both valid; choice trades locality vs smoothness.|Sequential bootstrap (instrument-by-instrument, exact reprice, local) vs global optimisation (minimise pricing error across all instruments under a smoothness/spline penalty). Global is needed when the instrument graph is cyclic or over/under-determined.|[QuantLib curve bootstrapping guide](https://www.quantlibguide.com/Curve%20bootstrapping.html), [BlueGamma methodology](https://www.bluegamma.io/documentation/methodology/how-to-bootstrap-the-yield-curve)|high|
|1.6|**Curve internal representation**: discount factors / zero rates / instantaneous forwards are interconvertible; choose the one the interpolation acts on.|`DF(t)=exp(−z(t)·t)`; instantaneous forward `f(t)=−d ln DF/dt`. Interpolating in different spaces (log-DF vs zero vs forward) yields materially different forward shapes.|[Hagan-West, Methods for constructing a yield curve](https://www.deriscope.com/docs/Hagan_West_curves_AMF.pdf)|high (textbook-standard)|
|1.7|**Interpolation choice is a first-class arbitrage decision.** Log-linear on DF ⇒ piecewise-flat (discontinuous) forwards; monotone-convex ⇒ positive, mostly-continuous forwards.|Hagan-West "monotone convex" fits quadratics to estimated instantaneous forwards, guaranteeing positive forwards whenever discrete forwards are positive; it can still produce material forward discontinuities in edge cases. Log-linear-DF is simple/local/arbitrage-free in DF but gives a sawtooth forward.|[Hagan-West paper](https://www.deriscope.com/docs/Hagan_West_curves_AMF.pdf), [Monotone Convex note (rnfc)](https://www.rnfc.org/courses/finance/modules/bond-yields-modelling/Monotone_Convex_Interpolation.pdf), [SciELO Hagan-West review](https://scielo.org.za/scielo.php?script=sci_arttext&pid=S2222-34362013000400003)|high|
|1.8|Forward-rate interpolation and discount-factor interpolation are **formally equivalent under a transform**.|There is a proven equivalence between forward-rate interpolation schemes and DF interpolation schemes — useful for choosing the cheapest computational representation without changing the curve.|[arxiv 2005.13890](https://arxiv.org/pdf/2005.13890)|med|
|1.9|**Turn-of-year / central-bank meeting-date jumps** must be modelled as forward discontinuities.|Year-end funding spikes and CB-meeting step changes are imposed as deliberate jumps in the instantaneous forward, not smoothed away — otherwise the curve misprices dated futures/OIS spanning those dates.|Brief §4.1 (charter); corroborated by [risk.net](https://www.risk.net/insight/risk-management/7957765/calibrating-interest-rate-curves-for-a-new-era)|med|
|1.10|**CSA / collateral discounting**: discount on the curve of the *collateral rate actually paid*; multi-currency CSAs add a "cheapest-to-deliver collateral" optionality.|Proper collateralisation acts dominantly through the discount factors; cross-currency trades with notional exchange are especially sensitive. A CSA allowing multiple eligible collateral currencies gives the poster an option to deliver the cheapest, raising the effective discount curve.|[FSA note on multi-curve construction](https://www.fsa.go.jp/frtc/nenpou/2009/07-1.pdf), [arxiv 1703.00923 (multiple collateral currencies)](https://arxiv.org/pdf/1703.00923), [arxiv 1101.5849 (asymmetric collateralisation)](https://arxiv.org/pdf/1101.5849)|high|
|1.11|**v1 simplification is defensible**: single OIS-discount curve, defer full multi-CSA (matches OPEN-QUESTIONS Q4 default).|For USD-SOFR-CSA trades, SOFR-discount + SOFR-projection is acyclic and bootstrappable without a CCY-basis solve; multi-CSA / CTD-collateral is a later complexity step.|Synthesis of 1.4/1.10 + charter Q4|med|

**P0 recommendation (curves):** support **log-linear-on-log-DF** (cheap, local, robust, arbitrage-free
in DF — the default for a fast hot path) **and monotone-convex-on-forwards** (smooth, positive forwards
— the "nice surface" option), selectable per curve. Build via sequential bootstrap where the dependency
graph is acyclic (USD-SOFR self-discounting), with a global multi-curve solver as the general fallback
for cyclic graphs (basis/XCCY). Model turn/meeting jumps as explicit forward steps.

---

## 2. Linear rates products (PV / par / DV01-PV01 / key-rate)

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|2.1|**FRA**: payoff is the discounted difference between the fixed agreed rate and the realised index fixing over the FRA period, settled at the *start* of the period (discounted one period).|`Payoff = N·τ·(L−K)/(1+τ·L)` paid at fixing date (market FRA discounting), where `L` is the realised fixing, `K` the agreed rate, `τ` the accrual. Forward value uses the projection-curve forward for `L` and the discount curve for PV.|[arxiv 1006.4767](https://arxiv.org/pdf/1006.4767); textbook-standard|high (textbook-standard)|
|2.2|**OIS / compounded RFR leg**: the floating coupon is the *geometric* compound of daily overnight fixings over the accrual, not a single fixing.|`R_comp = (∏_i (1 + r_i·d_i/360) − 1)·(360/D)` over business days `i` in the period; this is the in-arrears compounded RFR. Day-basis ACT/360 for USD/EUR, ACT/365 for GBP-SONIA.|[ARRC Users Guide to SOFR (2021)](https://www.newyorkfed.org/medialibrary/Microsites/arrc/files/2021/users-guide-to-sofr2021-update.pdf)|high|
|2.3|**Vanilla IRS PV** = PV(fixed leg) − PV(float leg) (payer perspective); each leg = Σ discounted cashflows on the discount curve, with float forwards from the projection curve.|`PV_fix = N·K·Σ τ_j·DF(t_j)`; `PV_flt = N·Σ τ_j·F_j·DF(t_j)` with `F_j` the projection-curve forward. Multi-curve: forwards and DFs come from **different** curves.|[arxiv 1006.4767](https://arxiv.org/pdf/1006.4767); textbook-standard|high (textbook-standard)|
|2.4|**Par swap rate** = the fixed rate making PV = 0 = (float-leg PV) / (fixed-leg annuity).|`S_par = (Σ τ_j·F_j·DF_j) / (Σ τ_k·DF_k)`. The denominator is the **annuity / PV01 of a basis point of fixed rate**. This is also the structural identity proving a par swap prices to ≈0 (oracle §6).|[Bond Math / swap duration](https://ebrary.net/14302/economics/interest_rate_swap_duration); textbook-standard|high (textbook-standard)|
|2.5|**PV01** (a.k.a. annuity) = ∂PV/∂(fixed rate) per bp = `N·Σ τ_k·DF_k·1e−4`; **DV01** = ∂PV/∂(market rate curve) per bp shift.|PV01 is rate-of-the-instrument sensitivity (analytic, = annuity); DV01 is curve-shift sensitivity (numerical bump-and-reprice of the calibrating quotes). For a par swap they are close but not identical; DV01 depends on the fixed rate.|[OpenGamma Strata forum: PV01 vs DV01](https://forums.opengamma.com/t/pv01-and-dv01-for-fixed-float-interest-rate-vanilla-swaps/579), [ICE DV01](https://idd.ice.com/IRHelp/Content/FM/DV01.htm)|high|
|2.6|**Key-rate / bucketed delta**: bump each curve pillar (or each calibrating instrument's quote) by 1bp, reprice, difference; the vector sums (approx.) to parallel DV01.|Two flavours: (a) **zero/par-rate key-rate durations** (bump curve node) and (b) **instrument-Jacobian deltas** (bump the calibrating quote and re-solve) — the latter is the trader-facing "delta ladder" and is what desks hedge on.|[OpenGamma Strata forum](https://forums.opengamma.com/t/pv01-and-dv01-for-fixed-float-interest-rate-vanilla-swaps/579); [risk metrics note](https://analystprep.com/study-notes/frm/part-1/valuation-and-risk-management/one-factor-risk-metrics-and-hedges/)|high|
|2.7|**Bond futures — conversion factor**: the CF is the clean price (per 1) of the deliverable at a **6% notional yield**, rounded to 4 dp.|CF>1 when coupon>6%, CF<1 when coupon<6%. Standardises heterogeneous deliverables to one futures contract.|[CME — Calculating UST Futures Conversion Factors](https://www.cmegroup.com/articles/2024/calculating-us-treasury-futures-conversion-factors.html), [CME PDF](https://www.cmegroup.com/trading/interest-rates/files/Calculating_U.S.Treasury_Futures_Conversion_Factors.pdf)|high|
|2.8|**Bond futures — invoice price & CTD**: `Invoice = FuturesPrice·CF + AccruedInterest`; CTD = the deliverable maximising the short's return, identified by **highest implied repo rate** (equivalently lowest net basis).|`ImpliedRepo` compares buying the cash bond and delivering into the future vs funding cost. In ultra-low-rate regimes the lowest-coupon shortest bond tends to be CTD.|[CME Treasury Analytics user guide](https://www.cmegroup.com/tools-information/quikstrike/quikstrike-treasury-analytics-user-guide.html), [OpenGamma Bond Futures note](https://quant.opengamma.io/Bond-Futures-OpenGamma.pdf), [RJO'Brien CTD](https://fixedincomegroup.com/fig-presentations/ust-conversion-factor/)|high|
|2.9|**STIR futures** need a **convexity adjustment** vs the FRA-implied forward (daily margining ⇒ futures rate > forward rate).|The futures price implies a rate biased above the forward; the adjustment depends on rate vol and is required for an arbitrage-free short-end curve build. (Magnitude is a vol-model input — flag for the vol pass; a deterministic Ho-Lee/HW-style adjustment is the standard P0 placeholder.)|[Mathema](https://help.mathema.com.cn/latest/docs/fixedincome/sofr_curve); textbook-standard|high (textbook-standard)|
|2.10|**Tenor basis swaps** (e.g. 3M vs 6M) and **cross-currency basis swaps** are the calibrating instruments for the *basis* between projection curves / across currencies.|XCCY basis swaps are float/float with a spread on the weaker-currency leg; **MtM (resettable)** variants reset the stronger-currency notional each period to cut counterparty exposure; **constant-notional** variants do not. Quoted spread sits on the weaker-currency constant-notional leg.|[Clarus — Mechanics of XCCY swaps](https://www.clarusft.com/mechanics-of-cross-currency-swaps/), [Finastra multicurrency curve note](https://www.finastra.com/sites/default/files/documents/2020/02/market-insight_curve-building-part-2-multicurrencies-curve-construction.pdf)|high|
|2.11|XCCY basis referencing **backward-looking RFRs** (post-LIBOR) has its own pricing literature.|Compounded-RFR XCCY basis swaps differ from old LIBOR ones in the timing of the floating accrual; relevant once EUR/GBP RFR curves enter.|[arxiv 2410.08477](https://arxiv.org/pdf/2410.08477)|med|

**P0 recommendation (products):** FRA, OIS, vanilla fixed-float IRS, tenor-basis & XCCY-basis swaps
(as curve calibrators *and* tradeable), STIR & bond futures (CTD/CF/implied-repo). Analytics: PV, par
rate, PV01 (analytic annuity), DV01 (bump calibrating quotes), key-rate delta ladder (instrument
Jacobian). Convexity adjustment for STIR enters as a deterministic placeholder now, vol-model-driven
in the vol pass.

---

## 3. Conventions & standards (config-schema sketch)

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|3.1|**ISDA day-count fractions** are the authoritative DCF set; each has a precise 2006-ISDA / ICMA clause.|`ACT/360` (ISDA 4.16e, ICMA 251.1(i)) — USD/EUR money-market & SOFR/€STR legs. `ACT/365F` (ISDA 4.16d) — GBP/SONIA. `30/360` (bond/US-corp fixed legs). `ACT/ACT ISDA` and `ACT/ACT ICMA` (govt bonds; ICMA divides by period-days×frequency).|[2006 ISDA Definitions](https://www.sc.com/en/uploads/sites/66/content/docs/2006-ISDA-Definitions.pdf), [ISDA 30/360 note](https://www.isda.org/2008/12/22/30-360-day-count-conventions/), [Wikipedia day-count (clause-cited)](https://en.wikipedia.org/wiki/Day_count_convention), [OpenGamma Strata DayCounts](https://strata.opengamma.io/apidocs/com/opengamma/strata/basics/date/DayCounts.html)|high|
|3.2|**Business-day conventions** (date rolling): Following, **Modified Following** (market default for swaps), Preceding, Modified Preceding, plus an **adjusted vs unadjusted** flag for accrual computation.|Mod-Following rolls to next business day unless that crosses a month-end, then rolls back. Accrual may use adjusted or unadjusted dates independently of payment-date rolling — this is a separate config axis.|[Wikipedia day-count](https://en.wikipedia.org/wiki/Day_count_convention), [OpenGamma IR conventions guide](https://quant.opengamma.io/Interest-Rate-Instruments-and-Market-Conventions.pdf)|high|
|3.3|**RFR observation methods** are the new convention axis vs LIBOR: lookback, observation(period) shift, lockout, payment delay — standardised by ISDA Supplement 75 / 2021 Definitions (effective 13 May 2021).|**Lookback**: use the rate from N business days before each observation day (rate weighting unchanged). **Observation shift**: shift the whole observation *period* back N days (weights shift too). **Lockout**: freeze the rate for the last N days of the period. **Payment delay**: pay N days after period end.|[ISDA RFR Conventions & Fallbacks Product Table (Oct 2021)](https://www.isda.org/a/bdigE/RFR-Conventions-and-IBOR-Fallbacks-Product-Table-October-2021.pdf), [ISDA Key Changes 2021 Definitions](https://www.isda.org/a/BNEgE/Key-Changes-in-the-2021-ISDA-Interest-Rate-Derivatives-Definitions-June-2021.pdf), [ARRC Users Guide to SOFR](https://www.newyorkfed.org/medialibrary/Microsites/arrc/files/2021/users-guide-to-sofr2021-update.pdf)|high|
|3.4|**Fixing sources / calendars** are per-currency: SOFR (NY Fed), €STR (ECB), SONIA (BoE); calendars US (FED/NYSE/SIFMA), EUR (TARGET2), GBP (London).|RFR fixings are published by the central bank the morning after the rate day; calendars drive both observation-day enumeration and payment rolling. Calendars must be data, not code.|[ARRC How to Use SOFR](https://www.newyorkfed.org/medialibrary/microsites/arrc/files/2019/How_to_Use_SOFR.pdf); charter §4.6|high|
|3.5|**Spot/settlement lag** is per-instrument: USD/GBP swaps typically T+2, money-market deposits T+0/T+2 by currency; bonds T+1 (UST) / T+2 (many).|These lags drive the effective/start date from trade date and must be configurable per (instrument, currency).|[OpenGamma IR conventions guide](https://quant.opengamma.io/Interest-Rate-Instruments-and-Market-Conventions.pdf); charter §4.2/§4.6|med|

**Config-schema sketch** (mirrors `docs/CONVENTIONS.md`'s per-(pair,tenor) record; this is a *schema
shape*, not Rust):

```
RatesConvention {                       // keyed per (currency, index, tenor) — never a global default
  index:            { name, fixing_source, currency }       // SOFR / €STR / SONIA …
  day_count:        DayCount             // ACT/360 | ACT/365F | 30/360 | ACT/ACT_ISDA | ACT/ACT_ICMA
  business_day_conv: BdConvention        // Following | ModifiedFollowing | Preceding | ModPreceding
  accrual_dates:    Adjusted | Unadjusted   // separate axis from payment rolling
  calendars:        [CalendarId]         // e.g. [US_FED, NYSE] | [TARGET2] | [GB_LON]
  spot_lag:         BusinessDays         // T+0/T+1/T+2
  payment_freq:     Frequency
  rfr_observation:  RfrObservation {     // null for term-rate / IBOR-style legs
     method:        Lookback | ObservationShift | Lockout | PaymentDelay
     offset_days:   BusinessDays
     lockout_days:  BusinessDays
     payment_delay: BusinessDays
     compounding:   Compounded | Averaged
  }
  roll_convention:  RollConvention       // EOM | IMM | day-of-month
  rounding:         { dp, mode }
}
```

---

## 4. Models / numerics for P0 (mostly deterministic curve math)

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|4.1|P0 is **deterministic curve math** — no stochastic model needed for FRA/OIS/IRS/basis/bond-futures PV & deltas.|All P0 numbers are discounted-cashflow + root-find (bootstrap) + bump (risk). The only stochastic input in P0 is the STIR-futures convexity adjustment.|Synthesis of §1–§2|high|
|4.2|**Root-finding**: bootstrap = 1-D Newton/Brent per pillar; global calibration = multi-D least-squares (Levenberg-Marquardt / Gauss-Newton) over all instruments with a smoothness penalty.|Sequential bootstrap is `O(n)` 1-D solves; global is one `n`-dim nonlinear solve. AD/dual numbers give exact pillar Jacobians for fast key-rate risk (rateslib/ORE both do this — but as *reference reading only*, see §5).|[QuantLib bootstrapping guide](https://www.quantlibguide.com/Curve%20bootstrapping.html), [BlueGamma](https://www.bluegamma.io/documentation/methodology/how-to-bootstrap-the-yield-curve)|high|
|4.3|**Where stochastic models enter (LATER passes, flag only):** swaptions/caps need **Bachelier (normal)** quoting — mandatory for negative rates — and **SABR** (incl. shifted/normal-SABR) for the smile; **Hull-White 1F/G2++** for Bermudans, CMS convexity, and the term-structure-consistent STIR convexity adjustment.|Normal vols are quoted in bp; lognormal in %. Negative rates ⇒ normal or shifted-lognormal/shifted-SABR is non-negotiable. None of this is P0; it is the `celnet-rates-vol` workstream.|[SABR (Wikipedia)](https://en.wikipedia.org/wiki/SABR_volatility_model), [MathWorks normal-SABR negative strikes](https://www.mathworks.com/help/fininst/calibrate-sabr-model-using-normal-volatilities-with-negative-strikes.html)|high|
|4.4|**Scale**: a curve build is cheap (`<ms`); the cost at IB scale is the **risk cube** (n_instruments × n_pillars × n_curves bump-reprice). Reuse the existing server-owned hierarchical-risk rollup rather than client loop-sum.|Favour analytic/AD deltas where exact; vectorise bump-reprice; the curve object should be an immutable, cheaply-cloned snapshot so many scenarios run in parallel. Pinned hot core stays alloc/lock-free per CLAUDE.md.|charter §2 (RiskService, SCALE-OUT), §9|med|

---

## 5. OSS reference engines AS ORACLES (license verdicts are load-bearing)

> **This table is the most important guardrail output of the pass.** "Oracle" = an independent engine
> we reconcile numbers against; it is **out-of-process and disposable** (a Python/C++ script in CI/test
> tooling), NEVER a Cargo runtime dependency. License verdict gates even that use.

| Engine | License (verified) | `cargo-deny`/runtime verdict | Rates coverage | How Celnet validates against it | Source |
|---|---|---|---|---|---|
|**QuantLib**|**Modified BSD (3-clause)**, GPL-compatible, permits proprietary/commercial use|✅ **CLEAN.** Safe even to link if ever FFI'd; primary golden oracle. (Still keep it out-of-process per existing `celnet-golden` pattern.)|Full: multi-curve bootstrap, OIS, IRS, FRA, basis, bond futures, day-counts, calendars, SABR, Hull-White, G2++.|Reconcile DF curve, swap PV, par rate, bond price, futures CF/CTD via QuantLib-Python in the golden harness, comparing to bit/tolerance bands.|[quantlib.org/license](https://www.quantlib.org/license.shtml)|
|**ORE (Open Source Risk Engine)**|**Modified BSD**, explicitly permits commercial incorporation; built *on* QuantLib|✅ **CLEAN** (same license posture as QuantLib).|Extends QuantLib: XVA, simulation, FRTB sensitivities, curve/risk infrastructure — the closest open analogue to the target product.|Use as the oracle for **risk-cube / FRTB-sensitivity** and multi-curve scenarios where QuantLib alone is lower-level. Out-of-process.|[opensourcerisk.org FAQ](https://www.opensourcerisk.org/faqs/), [github OpenSourceRisk/Engine](https://github.com/OpenSourceRisk/Engine)|
|**rateslib**|⚠️ **Dual: source-available NON-COMMERCIAL + paid commercial.** Project states it is "source-available, **not** open source"; commercial use requires a paid licence.|❌ **TRAP — NOT OSS.** Must NOT be a dependency and arguably must not be used as an oracle in a commercial pipeline without a licence. Treat as **excluded**; do not vendor, do not `cargo-deny`-allow. If any reference use is desired, get explicit operator/legal sign-off first.|Curves, IRS, XCS, FX swaps, bonds, bond futures, AD delta/gamma — technically excellent, but licence makes it unusable for us.|**Do not use** as a runtime dep; flag to operator as a new open question (licence ≠ OSS).|[github attack68/rateslib (license)](https://github.com/attack68/rateslib)|
|**FinancePy**|**GPL-3.0** (copyleft)|❌ **TRAP — copyleft.** GPL-3.0 as a *linked dependency* would force GPL on Celnet — forbidden by guardrail §3.1. As an **out-of-process oracle script only** it does not infect us (no linking), but prefer QuantLib/ORE to avoid any ambiguity.|Bonds, IBOR swaptions, callable swaps, discount curves, BDT/Vasicek trees, swaption vol surfaces.|Acceptable ONLY as a disposable, separately-invoked CLI oracle (no linkage) — and even then secondary to QuantLib/ORE. Never a dependency.|[github domokane/FinancePy (license)](https://github.com/domokane/FinancePy)|

**Net:** the golden-oracle set for `celnet-rates` is **QuantLib (primary) + ORE (risk/FRTB)**.
`rateslib` and `FinancePy` are removed from the dependency-eligible set; `rateslib`'s non-OSS licence is
a **new operator decision** (see §B). This keeps the existing `cargo-deny` MIT/Apache/BSD policy intact.

---

## 6. Per-product oracle plan (independent structural identity — anti-circular)

The rule (from `docs/VERIFICATION-CONTRACT.md` and the W2 "don't re-derive `F=S·e^{(r_d−r_f)t}`" lesson):
every number reaches a reference by **two independent routes** — (a) a genuinely different engine
(QuantLib/ORE), AND (b) a structural identity that holds *regardless of the engine*. Identities below are
the load-bearing anti-circular checks.

| Product | Independent structural identity (engine-agnostic) | + Engine oracle | Conf |
|---|---|---|---|
|**Curve DF**|`DF(0)=1`, `DF` strictly positive & (for sane curves) monotone-decreasing; **re-pricing the calibrating instruments reproduces their market quotes to tolerance** (the bootstrap fixed-point). Forward `f(t)=−d ln DF/dt > 0` under monotone-convex.|QuantLib `PiecewiseYieldCurve` DFs at the same pillars.|high|
|**Swap PV**|**Par-swap PV ≈ 0**: build the swap at the curve's own par rate ⇒ PV must vanish. **Receiver + Payer = 0** (offsetting). **Leg additivity**: PV = PV(fixed) − PV(float) computed two independent ways (cashflow sum vs annuity·(K−S_par)).|QuantLib `VanillaSwap` NPV.|high|
|**Par swap rate**|`S_par = floatPV / annuity`; **independently**, the rate that zeroes a freshly-built swap by 1-D root-find must equal the closed-form `S_par` (two different computations agree).|QuantLib `swap.fairRate()`.|high|
|**Bond price**|**Bond = Σ discounted cashflows**: dirty price = Σ c_i·DF(t_i) + redemption·DF(T); **clean = dirty − accrued**; **price↔yield round-trip** (price→yield→price is identity).|QuantLib `FixedRateBond` clean/dirty + yield.|high|
|**Bond future CF/CTD**|**CF** = clean price of deliverable at 6% yield (recompute independently of the engine and match CME published CF to 4dp). **CTD** = argmax implied-repo across the basket; verify by checking the chosen bond also has min net basis (two equivalent rankings agree).|QuantLib `BondForward`/futures + CME published CFs as a *third* anchor.|high|
|**PV01 / DV01**|**PV01 = annuity** computed analytically must match a `±1bp` finite-difference bump of the fixed rate (analytic vs numerical agree). **Key-rate deltas sum ≈ parallel DV01** (decomposition closes).|QuantLib bucketed sensitivities.|high|
|**FRA**|FRA PV must equal the corresponding **single-period swap** PV (a FRA is a 1-period swaplet) — cross-product identity, no shared code path.|QuantLib `ForwardRateAgreement`.|high|

CME published conversion factors give a rare *third* independent anchor for futures — use it.

---

## A. Proposed P0 crate sketch — `celnet-rates` (reflecting D1)

Per the **locked D1 decision**, fixed income gets its **own crate family**, depending only on the shared
seams, never folded into `celnet-vanilla`/`celnet-linear`. Proposed P0 split:

```
celnet-rates                       (P0 — curves + linear rates)
├── depends on (shared seams only):
│     celnet-types         (Currency, Money, Tenor, Date, day-count enums)
│     celnet-conventions   (extended with RatesConvention §3 schema)
│     celnet-calendar      (US_FED, TARGET2, GB_LON business-day calendars)
│     celnet-core          (the pricing-trait/discount seam — reuse discount-factor abstraction)
│   (NO dependency on celnet-vanilla / celnet-linear — D1)
│
├── modules:
│   ├── curve/             immutable Curve snapshot: DF | zero | inst-fwd repr; interpolation
│   │   ├── interp/        log_linear_df, monotone_convex_fwd  (provenance: Hagan-West — comment only)
│   │   └── jumps/         turn-of-year / meeting-date forward steps
│   ├── build/             bootstrap (sequential, acyclic) + global solver (LM, cyclic/basis/XCCY)
│   ├── conventions/       RatesConvention resolution per (ccy,index,tenor); RFR observation methods
│   ├── product/           Fra, Ois, VanillaSwap, TenorBasisSwap, CrossCcyBasisSwap, StirFuture, BondFuture(CTD)
│   ├── rfr/               compounded/averaged RFR accrual; lookback/shift/lockout/payment-delay
│   ├── pricing/           PV, par rate, cashflow projection (discount curve × projection curve)
│   └── risk/              pv01 (annuity), dv01 (quote bump), key_rate (instrument Jacobian)
│
├── exposes (for the additive contract & clients):
│     CurveSet            (discount + per-tenor projection curves, immutable, cheaply cloned)
│     RatesQuote / RatesInstrument specs (FRA/OIS/IRS/basis/XCCY/futures)
│     PricingResult { pv, par_rate, pv01, dv01, key_rate_ladder }
│     — surfaced via additive celnet.wire arms (Options | Fixed-Income), reused by all 5 clients;
│       risk folds into the existing server-owned RiskService hierarchy (no client loop-sum).
│
└── later: celnet-rates-vol  (P-next: swaptions/caps — Bachelier/normal-SABR/shifted, Hull-White/G2++;
                              vol cube expiry×tenor×strike) — depends on celnet-rates + celnet-surface.
```

Golden oracle for the crate: **QuantLib (primary) + ORE (risk-cube/FRTB)**, wired through the existing
`celnet-golden` harness, out-of-process. (`rateslib`/`FinancePy` excluded — §5.)

---

## B. New open questions to append to `OPEN-QUESTIONS.md`

> The agent surfaces these; the operator fills the Decision column. (Do not assume.)

| # | Question | Why it matters | Proposed default |
|---|---|---|---|
|Q8|**`rateslib` is source-available non-commercial, NOT OSS** — exclude entirely, or seek a commercial/eval licence for *oracle-only* use? | Guardrail §3.1 (OSS-only); rateslib is technically excellent but non-OSS. Affects which engines validate XCS/AD-delta numbers. | **Exclude.** Validate with QuantLib + ORE only; revisit only if a clear gap appears and legal signs off. |
|Q9|**FinancePy is GPL-3.0** — permit as an out-of-process (non-linked) oracle script, or exclude to keep zero GPL anywhere in the pipeline? | GPL doesn't infect via separate-process invocation, but the operator may want zero GPL even in tooling. | **Exclude by default**; QuantLib/ORE cover the same ground. |
|Q10|**Default interpolation for the P0 curve**: log-linear-on-log-DF (fast/local) vs monotone-convex-forward (smooth/positive) as the *shipping default*, with the other selectable? | Drives forward-rate shape, risk-ladder smoothness, and hot-path cost. | Ship **both**, default **log-linear-DF** for speed/robustness; monotone-convex for the "nice surface" view. |
|Q11|**STIR-futures convexity adjustment in P0**: deterministic placeholder (Ho-Lee/HW-style closed form) now, or defer the short end to the vol pass? | The short-end curve is wrong without *some* adjustment; the exact one needs a vol model. | Deterministic placeholder now; replace with term-structure-consistent adjustment in `celnet-rates-vol`. |
|Q12|**Calendar/fixing data sourcing** under the no-commercial-feed guardrail: hand-curate US_FED/TARGET2/GB_LON + NY-Fed/ECB/BoE published fixings, or pull an OSS calendar lib? | Calendars/fixings must be data, not code; need an open, auditable source. | Operator-supplied/curated static from the central-bank publications (all free/published). |
|Q13|**Anchor on CME published conversion factors** as a third oracle for bond futures? | Gives an engine-independent third anchor (not just QuantLib) for CF/CTD — strengthens anti-circular. | Yes — ingest CME CFs in the golden harness for futures. |

## C. Recommended next research pass

**Pass 2 — `celnet-rates-vol` foundation + cash-bond breadth** (depth over breadth again):
1. **Rates volatility**: Bachelier/normal quoting, shifted-lognormal & shifted/normal-SABR smile
   calibration, the swaption vol cube (expiry × tenor × strike), caps/floors; Hull-White 1F & G2++ for
   Bermudans/CMS; the numerical methods (analytic/tree/PDE/LSM) and their IB-scale convergence — with
   QuantLib/ORE as oracle and put-call (receiver+payer = forward swap) as the structural identity.
2. **Cash-bond breadth + market conventions per market** (UST/Gilt/Bund/JGB/OAT): accrued, ex-div,
   settlement, Z-spread/ASW/OAS, FRNs, inflation-linked (real curve, indexation lag, deflation floor).
3. **Credit (separate workstream scoping)**: single-name CDS + ISDA Standard Model, hazard/survival
   bootstrap — gated behind operator Q3.

Then synthesise both passes into the full corpus (`FI-CURVES-SPEC.md`, `FI-ANALYTICS-SPEC.md`,
`FI-CONVENTIONS.md`, `FI-ARCHITECTURE.md`, `FI-VERIFICATION-CONTRACT.md`, `FI-ROADMAP.md`).

---

### Sources (consolidated)

- risk.net — Calibrating interest rate curves for a new era: https://www.risk.net/insight/risk-management/7957765/calibrating-interest-rate-curves-for-a-new-era
- Multi-curve interest-rate modelling (arxiv 1006.4767): https://arxiv.org/pdf/1006.4767
- Mathema — SOFR curve construction methodology: https://help.mathema.com.cn/latest/docs/fixedincome/sofr_curve
- Quantifi — Tackling IR curve construction complexity: https://www.quantifisolutions.com/tackling-interest-rate-curve-construction-complexity/
- QuantLib curve bootstrapping guide: https://www.quantlibguide.com/Curve%20bootstrapping.html
- BlueGamma bootstrap methodology: https://www.bluegamma.io/documentation/methodology/how-to-bootstrap-the-yield-curve
- Hagan-West, Interpolation Methods for Curve Construction: https://www.deriscope.com/docs/Hagan_West_curves_AMF.pdf
- Monotone Convex method note (rnfc): https://www.rnfc.org/courses/finance/modules/bond-yields-modelling/Monotone_Convex_Interpolation.pdf
- SciELO — positive/continuous forward curves (Hagan-West review): https://scielo.org.za/scielo.php?script=sci_arttext&pid=S2222-34362013000400003
- Forward-rate vs DF interpolation equivalence (arxiv 2005.13890): https://arxiv.org/pdf/2005.13890
- Multiple collateral currencies (arxiv 1703.00923): https://arxiv.org/pdf/1703.00923
- Asymmetric/imperfect collateralization & CVA (arxiv 1101.5849): https://arxiv.org/pdf/1101.5849
- FSA — construction of multiple swap curves: https://www.fsa.go.jp/frtc/nenpou/2009/07-1.pdf
- ARRC Users Guide to SOFR (2021): https://www.newyorkfed.org/medialibrary/Microsites/arrc/files/2021/users-guide-to-sofr2021-update.pdf
- ARRC How to Use SOFR (2019): https://www.newyorkfed.org/medialibrary/microsites/arrc/files/2019/How_to_Use_SOFR.pdf
- ISDA RFR Conventions & IBOR Fallbacks Product Table (Oct 2021): https://www.isda.org/a/bdigE/RFR-Conventions-and-IBOR-Fallbacks-Product-Table-October-2021.pdf
- ISDA Key Changes in the 2021 IRD Definitions: https://www.isda.org/a/BNEgE/Key-Changes-in-the-2021-ISDA-Interest-Rate-Derivatives-Definitions-June-2021.pdf
- 2006 ISDA Definitions: https://www.sc.com/en/uploads/sites/66/content/docs/2006-ISDA-Definitions.pdf
- ISDA 30/360 day-count note: https://www.isda.org/2008/12/22/30-360-day-count-conventions/
- Wikipedia — Day count convention (ISDA/ICMA clause-cited): https://en.wikipedia.org/wiki/Day_count_convention
- OpenGamma Strata DayCounts API: https://strata.opengamma.io/apidocs/com/opengamma/strata/basics/date/DayCounts.html
- OpenGamma Interest-Rate Instruments & Market Conventions Guide: https://quant.opengamma.io/Interest-Rate-Instruments-and-Market-Conventions.pdf
- OpenGamma Strata forum — PV01 vs DV01: https://forums.opengamma.com/t/pv01-and-dv01-for-fixed-float-interest-rate-vanilla-swaps/579
- ICE — DV01: https://idd.ice.com/IRHelp/Content/FM/DV01.htm
- AnalystPrep — one-factor risk metrics: https://analystprep.com/study-notes/frm/part-1/valuation-and-risk-management/one-factor-risk-metrics-and-hedges/
- Bond Math — interest-rate-swap duration: https://ebrary.net/14302/economics/interest_rate_swap_duration
- CME — Calculating UST Futures Conversion Factors (article): https://www.cmegroup.com/articles/2024/calculating-us-treasury-futures-conversion-factors.html
- CME — Conversion Factors (PDF): https://www.cmegroup.com/trading/interest-rates/files/Calculating_U.S.Treasury_Futures_Conversion_Factors.pdf
- CME — Treasury Analytics user guide (CTD/implied repo): https://www.cmegroup.com/tools-information/quikstrike/quikstrike-treasury-analytics-user-guide.html
- OpenGamma — Bond Futures description & pricing: https://quant.opengamma.io/Bond-Futures-OpenGamma.pdf
- RJO'Brien — UST conversion factor / CTD: https://fixedincomegroup.com/fig-presentations/ust-conversion-factor/
- Clarus — Mechanics of Cross Currency Swaps: https://www.clarusft.com/mechanics-of-cross-currency-swaps/
- Finastra — Multicurrency curve construction (part 2): https://www.finastra.com/sites/default/files/documents/2020/02/market-insight_curve-building-part-2-multicurrencies-curve-construction.pdf
- XCCY basis swaps referencing backward-looking rates (arxiv 2410.08477): https://arxiv.org/pdf/2410.08477
- QuantLib license: https://www.quantlib.org/license.shtml
- Open Source Risk Engine FAQ: https://www.opensourcerisk.org/faqs/
- OpenSourceRisk/Engine (GitHub): https://github.com/OpenSourceRisk/Engine
- rateslib (GitHub, licence): https://github.com/attack68/rateslib
- FinancePy (GitHub, licence): https://github.com/domokane/FinancePy
- SABR volatility model (Wikipedia): https://en.wikipedia.org/wiki/SABR_volatility_model
- MathWorks — normal-SABR with negative strikes: https://www.mathworks.com/help/fininst/calibrate-sabr-model-using-normal-volatilities-with-negative-strikes.html
