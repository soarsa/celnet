# Fixed-Income Research Findings — Pass 2 (`celnet-rates-vol` + cash-bond breadth + credit scoping)

**Status:** Research findings (second pass) · 2026-06-22 · Branch `feature/fixedincome`
**Executor:** deep-research agent · **Charter:** [`../fixed-income/FIXED-INCOME-RESEARCH-BRIEF.md`](../fixed-income/FIXED-INCOME-RESEARCH-BRIEF.md)
**Builds on:** [`fixed-income-findings.md`](fixed-income-findings.md) (Pass 1 — P0 curves + linear products). **Do not** re-read Pass-1 scope here; this pass is the next layer.
**Web access:** AVAILABLE (WebSearch + WebFetch). Every non-obvious claim carries a source URL or is tagged `UNVERIFIED — knowledge-based, confirm`.

> **Convention for this doc (inherited from Pass 1).** Confidence = high/med/low. "high" = corroborated by a primary
> standards/CME/Fed/ISDA source or multiple independent sources. Closed-form quant identities that are textbook-standard
> but whose *exact* primary-source page could not be extracted from a binary PDF are marked **high (textbook-standard)**
> with the closest cited source. Method/person names (Bachelier, SABR, Hagan, Hull-White, G2++, Jamshidian, Jensen)
> appear here only as *provenance* — per guardrail §3.2 they must **never** enter `celnet-rates-vol` identifiers; they
> live in doc comments only. Proposed identifiers below obey this (e.g. `SwaptionInputs`, `NormalVol`, `VolCube`,
> never `HullWhiteInputs`/`SabrParams`-as-public-API).

## Scope of this pass

Per Pass-1 §C "Recommended next research pass", depth over breadth, three tracks:

1. **Rates volatility foundation** (`celnet-rates-vol`): normal (Bachelier, bp) vs lognormal quoting; negative-rate
   forcing functions (normal / shifted-lognormal / shifted- & normal-SABR); SABR smile calibration; the swaption vol
   **cube** (expiry × tenor × strike); caps/floors + caplet stripping; short-rate models (1F & 2F Gaussian) for
   Bermudans / CMS convexity / term-structure-consistent STIR convexity; numerical methods (analytic / tree / PDE /
   LSM) and their IB-scale convergence/perf; the structural identities (receiver + payer = forward swap; cap − floor = swap).
2. **Cash-bond breadth + per-market conventions** (UST / Gilt / Bund / JGB / OAT): accrued, ex-div, settlement,
   clean/dirty, yield↔price; Z-spread / ASW / OAS; FRNs (discount margin); inflation-linked (real curve, lag, deflation floor).
3. **Credit (scoping only, gated behind operator Q3)**: single-name CDS + ISDA Standard Model, hazard/survival bootstrap,
   recovery, index CDS — **scoped, not designed**.

**The single most consequential guardrail finding of this pass:** the **ISDA CDS Standard Model source is NOT
permissively licensed** — it ships under the *custom* "ISDA CDS Standard Model Public License v1.0", which carries an
**assent-on-redistribution requirement, indemnification, trademark and patent-termination clauses** and is **neither
OSI- nor FSF-approved**; it would **fail the existing `cargo-deny` MIT/Apache/BSD allowlist**. The clean path is that
**QuantLib's `IsdaCdsEngine` (modified-BSD) re-implements the ISDA model methodology** — so we get the *standard* CDS
numbers as a **permissive oracle** without ever touching ISDA's restrictive licence. (Track 3, §9; new question **Q19**.)

---

## 7. Rates volatility — quoting conventions (normal/bp vs lognormal) & negative rates

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|7.1|Two quoting worlds coexist: **lognormal (Black) vol in %** and **normal (Bachelier) vol in bp** ("bp vol" / "normal vol"). bp vol is NOT a separate model — it is the Bachelier vol expressed in absolute units of the underlying.|Black-model implied vol is quoted in %; Bachelier-model implied vol is quoted in bp and called *normal* or *basis-point* vol. A vol cube/surface must carry its quoting basis as data.|[MathWorks — work with negative rates](https://www.mathworks.com/help/fininst/work-with-negative-interest-rates-using-functions.html), [arxiv 1112.1782 (normal↔lognormal equivalence)](https://arxiv.org/pdf/1112.1782)|high|
|7.2|**Negative rates break Black/lognormal** — the forward can be ≤ 0, so `ln(F/K)` is undefined. The market moved to **normal vols** or **shifted (displaced) lognormal/shifted-Black** vols for caps/floors/swaptions.|A lognormal forward cannot go negative; once EUR/CHF/JPY forwards went sub-zero the Black quote is mathematically ill-posed. Two fixes: quote normal (bp) vol, or shift the forward by a displacement `s` so `F+s>0` and quote shifted-lognormal.|[MathWorks negative rates](https://www.mathworks.com/help/fininst/work-with-negative-interest-rates-using-functions.html)|high|
|7.3|**Bachelier (normal) closed form** for a payer swaption: `PV = A · [(F−K)·Φ(d) + σ_N·√T·φ(d)]`, `d=(F−K)/(σ_N√T)`, with `A` = annuity (PV01), `F` = forward swap rate, `σ_N` = normal vol. Receiver is the symmetric put.|Forward swap rate is a driftless arithmetic Brownian motion under the **annuity (swap) measure**, where it is a martingale; the payer = call, receiver = put. Handles `F<0` and `K<0` natively.|[MathWorks `swaptionbynormal`](https://www.mathworks.com/help/fininst/swaptionbynormal.html), [Baruch IRC Lecture 5](https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture5_2019.pdf)|high (textbook-standard)|
|7.4|There is a **model-free normal↔lognormal vol map** for ATM and a known relationship away from ATM — useful for cross-checking a cube quoted in one basis against the other.|For ATM, `σ_N ≈ σ_LN · F` to leading order; an exact model-free transform exists. Lets the oracle reconcile a bp-quoted cube against a %-quoted engine and vice-versa (anti-circular cross-basis check).|[arxiv 1112.1782](https://arxiv.org/pdf/1112.1782)|high|
|7.5|**Structural identity (load-bearing):** **payer − receiver swaption = forward swap**: `Payer − Receiver = A·(F − K)` (same expiry/strike/underlying). Equivalently receiver+payer at strike F = the forward annuity value; this is the swaption put–call parity.|`A` = annuity, `F` = forward swap rate, `K` = strike. Engine-agnostic: holds regardless of normal/lognormal/SABR. The Pass-2 oracle analogue of FX put-call parity.|[Wikipedia — Swaption](https://en.wikipedia.org/wiki/Swaption), [Baruch IRC Lecture 5](https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture5_2019.pdf)|high|
|7.6|**Structural identity:** **cap − floor = a swap** (same strike/schedule), the rate analogue of put-call parity. Long cap + short floor at strike K = pay-fixed-K / receive-float swap.|`Cap(K) − Floor(K) = PV of receiving float − paying fixed K` over the schedule; collapses to the par swap when `K = par`. Engine-agnostic cross-product check between the cap/floor and the linear (Pass-1) swap engine.|[ryanoconnellfinance — swaptions](https://ryanoconnellfinance.com/swaptions/), corroborated [Baruch IRC L5](https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture5_2019.pdf)|high|

---

## 8. SABR smile & the swaption vol cube

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|8.1|**SABR** is the market-standard 4-parameter stochastic-vol smile model: `α` (ATM level), `β` (CEV/backbone exponent, fixed not fitted), `ρ` (skew, fwd↔vol correlation), `ν` (vol-of-vol, smile curvature). It preserves Black-like closed forms for caps/floors/swaptions via an asymptotic implied-vol expansion.|`ρ` controls slope/skew, `ν` controls convexity/smile. The Hagan-2002 asymptotic expansion makes calibration tractable; only 3 free params per (expiry,tenor) cell once `β` is pinned.|[Wikipedia — SABR](https://en.wikipedia.org/wiki/SABR_volatility_model), [MathWorks normal-SABR](https://www.mathworks.com/help/fininst/calibrate-sabr-model-using-normal-volatilities-with-negative-strikes.html), [Mathema swaption cube](https://help.mathema.com.cn/latest/docs/fixedincome/swaptioncube)|high|
|8.2|**Negative-rate SABR**: set **`β = 0` ⇒ normal-SABR**, which admits negative rates/strikes; or use **shifted-SABR** (displace the forward by `s`). Both are calibrated to **normal (bp)** market vols.|`β=0` makes the SABR backbone normal (Bachelier-like), so the model permits `F<0`,`K<0`. The shift `s` is the alternative when the desk quotes shifted-lognormal.|[MathWorks normal-SABR negative strikes](https://www.mathworks.com/help/fininst/calibrate-sabr-model-using-normal-volatilities-with-negative-strikes.html)|high|
|8.3|**Hagan asymptotic SABR is not arbitrage-free at low strikes** (negative implied densities). SOTA fixes: **arbitrage-free SABR** (PDE on the density, Hagan 2014), **No-Arbitrage SABR (Doust)**, and **stochastic-collocation** projection onto arbitrage-free variables.|For pricing far-OTM / low-strike the asymptotic formula can imply a negative probability density; the production-grade cube uses an arbitrage-free variant for the wings. QuantLib ships both `SabrSwaptionVolatilityCube` (Hagan-2002) and `NoArbSabrSwaptionVolatilityCube` (Doust).|[ResearchGate — Arbitrage-free SABR](https://www.researchgate.net/publication/264718376_Arbitrage-free_SABR), [arxiv 2510.10343 (exact SABR)](https://arxiv.org/pdf/2510.10343), [QuantLib SABR cube hdr](https://rkapl123.github.io/QLAnnotatedSource/d5/d22/sabrswaptionvolatilitycube_8hpp.html)|high|
|8.4|**The swaption vol cube is 3-D: (option expiry) × (underlying swap tenor) × (strike).** ATM is the par swap rate; off-ATM cells are usually quoted as **vol spreads to ATM**. The strike axis must contain 0-spread (=ATM).|First axis strike-spread (Δ from ATM), second option expiry, third swap tenor. Non-ATM sub-tables hold *spreads* to the ATM vol surface; SABR fills the strike dimension when only ATM + a few smile points are quoted.|[Mathema swaption cube](https://help.mathema.com.cn/latest/docs/fixedincome/swaptioncube), [Studocu vol-cube methodology](https://www.studocu.com/en-us/document/university-at-buffalo/mathematical-finance-2/volatility-cube-construction/82566158)|high|
|8.5|**Cube interpolation = "fit early, interpolate later"**: fit SABR per (expiry,tenor) cell to its smile, then **linearly interpolate the SABR parameters** across the expiry/tenor grid for off-grid points (rather than interpolating raw vols).|Calibrating SABR per cell then interpolating `α,ρ,ν` keeps each slice arbitrage-aware; QuantLib's cube uses this. Off-grid (expiry,tenor) gets interpolated params; the strike smile is then evaluated analytically.|[Mathema swaption cube](https://help.mathema.com.cn/latest/docs/fixedincome/swaptioncube), [QuantLib SABR cube hdr](https://rkapl123.github.io/QLAnnotatedSource/d5/d22/sabrswaptionvolatilitycube_8hpp.html)|high|
|8.6|Newer **no-arbitrage cube** research uses **Gaussian-process regression under no-arb constraints** to fill sparse cubes — a candidate enhancement, not a P-next requirement.|GP-regression interpolation of the cube enforcing monotonicity/convexity (no-arb) is an academic SOTA option for sparse-quote markets; flag as a later optimisation, validate against SABR.|[MDPI Risks 10(12):232 — GP swaption cube](https://www.mdpi.com/2227-9091/10/12/232)|med|

---

## 9. Caps/floors & caplet stripping

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|9.1|**A cap = Σ caplets; a floor = Σ floorlets**; each caplet is a Black/Bachelier option on a single forward fixing. So a cap is priced as a strip of independent optionlets.|Each caplet/floorlet prices off the forward for its period and a per-period (forward-forward) vol; the cap value is their sum. Caps are quoted at one **flat term vol** that prices the whole strip.|[OpenGamma — Intro to caplet stripping](https://quant.opengamma.io/Caplet-Stripping-OpenGamma.pdf), [Balaraman — QuantLib caps/floors](http://gouthamanbalaraman.com/blog/interest-rate-cap-floor-valuation-quantlib-python.html)|high|
|9.2|**Caplet stripping** = invert quoted *flat cap term vols* into the per-caplet **forward-forward** vols (an optionlet vol surface). Quoted caps overlap in their caplets, so stripping is a sequential/bootstrap inversion.|Cap(T2) shares all the caplets of Cap(T1) plus the new ones between T1,T2; stripping recovers each caplet vol. QuantLib provides `OptionletStripper1` (from a term-vol *surface*) and `OptionletStripper2` (adds ATM curve). Output: normal or shifted-lognormal optionlet vols.|[OpenGamma caplet stripping](https://quant.opengamma.io/Caplet-Stripping-OpenGamma.pdf), [QuantLib `OptionletStripper1`](https://rkapl123.github.io/QLAnnotatedSource/df/da8/class_quant_lib_1_1_optionlet_stripper1.html), [QuantExt/ORE `OptionletStripper2`](https://www.opensourcerisk.org/docs/qle/class_quant_ext_1_1_optionlet_stripper2.html)|high|
|9.3|Cap/floor vols are stripped to **normal or shifted-lognormal** optionlet surfaces (same negative-rate logic as swaptions); SABR can then be fit to the strike dimension of the optionlet surface.|Post-2015 most cap/floor desks quote and strip in normal/shifted vols. A known gap: SABR interpolation for cap/floor vols is *not* fully built out in QuantLib/ORE (only swaption cubes) — noted for our own implementation.|[QuantLib/ORE mailing list — missing SABR cap/floor interp](https://sourceforge.net/p/quantlib/mailman/message/37877801/)|med|

---

## 10. Short-rate models (1F & 2F Gaussian) — Bermudans, CMS, STIR convexity

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|10.1|**1-factor Gaussian short-rate model** (provenance: Hull-White) is the workhorse for **Bermudan swaptions** and term-structure-consistent adjustments: it exactly refits today's curve via a time-dependent drift `θ(t)`, and is calibrated to the ATM swaption diagonal (mean-reversion + vol).|`θ(t)` is solved so the model reprices the discount curve; mean-reversion `a` and short-rate vol `σ` are calibrated to co-terminal/diagonal ATM swaptions. Closed-form European swaption via Jamshidian decomposition (a coupon-bond option splits into a strip of zero-bond options).|[Wikipedia — Hull–White](https://en.wikipedia.org/wiki/Hull%E2%80%93White_model), [arxiv 0901.1776 (efficient HW swaptions)](https://arxiv.org/pdf/0901.1776), [diva2:946248 (HW swaptions)](https://www.diva-portal.org/smash/get/diva2:946248/FULLTEXT01.pdf)|high|
|10.2|**2-factor Gaussian model** (provenance: G2++ / 2F Hull-White) adds a second factor + factor correlation ⇒ realistic **decorrelation** of curve points; preferred for CMS-spread and curve-shape-sensitive Bermudans. The 1F model is a special case of the 2F.|G2++ ≡ 2F-HW (a coordinate change). Calibrated to swaptions; correlation `ρ` gives a non-trivial term-structure of forward-rate correlations the 1F model can't produce.|[Calibration of 1F/2F HW using swaptions (Bergamo)](https://aisberg.unibg.it/retrieve/e40f7b8a-9809-afca-e053-6605fe0aeaf2/Calibration%20One-%20Two-Factor%20-%20final_SecondoInvi.pdf)|high|
|10.3|**Numerical methods for early exercise (Bermudan):** (a) **analytic** Europeans (Jamshidian, 1F); (b) **trinomial tree** on the short rate; (c) **PDE / finite-difference** (Feynman-Kac); (d) **Monte-Carlo + LSM** (least-squares regression for the exercise boundary) — mandatory in high-dimensional models (LMM/2F).|FD engines **outperform trinomial trees** for Bermudan swaptions (HPC-QuantLib). 1F/2F Gaussian ⇒ tree or PDE is feasible and fast; LMM / many-factor ⇒ Monte-Carlo + LSM (lattices infeasible by dimensionality). QuantLib supports operator-splitting PDE schemes for multi-D.|[HPC-QuantLib — Bermudan swaption FD](https://hpcquantlib.wordpress.com/2011/12/19/bermudan-swaption-pricing-based-on-finite-difference-methods/), [arxiv 0901.1776](https://arxiv.org/pdf/0901.1776)|high|
|10.4|**CMS convexity adjustment** via **static replication by European swaptions**: a CMS caplet/floorlet/swaplet is replicated by a continuum (in practice a 10bp-bucketed strip) of payer/receiver swaptions; the adjustment comes from the **bond/annuity numeraire ratio** (the CMS rate is not the forward swap rate — Jensen convexity under the measure change).|`E^T[S_T] = S_0 + convexity`; the replication links the CMS adjustment directly to the **swaption smile** (use the SABR cube to value the replicating strip). Most accurate method; keeps CMS pricing consistent with the vanilla swaption book.|[Hagan — Convexity Conundrums (PDF)](https://www.deriscope.com/docs/Hagan_Convexity_Conundrums.pdf), [Baruch IRC Lecture 6 (CMS/convexity)](https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture6_2019.pdf), [Burgess — Convexity Adjustments Made Easy](https://papers.ssrn.com/sol3/Delivery.cfm/SSRN_ID4052825_code1728976.pdf?abstractid=3401235&mirid=1)|high|
|10.5|**STIR-futures convexity adjustment** (Pass-1 §2.9 placeholder) becomes **term-structure-consistent** here: a Gaussian short-rate model gives the closed-form futures-vs-forward convexity from `a,σ`, replacing the deterministic placeholder.|The 1F Gaussian model yields an analytic futures/forward convexity term; this is the principled replacement for the Pass-1 deterministic short-end adjustment (closes Pass-1 Q11).|[Wikipedia — Hull–White](https://en.wikipedia.org/wiki/Hull%E2%80%93White_model); corroborated Pass-1 §2.9|med|

**P-next recommendation (vol):** ship **(1)** a normal/lognormal/shifted quoting layer (bp & %); **(2)** an analytic
**Bachelier + Black** European swaption & caplet/floorlet engine; **(3)** **SABR** (β-fixed, normal-SABR `β=0` for
negative rates, shifted-SABR option) with an **arbitrage-free wing** (No-Arb-SABR / collocation), fit-early-interpolate-
later into the **vol cube**; **(4)** **caplet stripping** to an optionlet surface; **(5)** a **1F Gaussian** short-rate
engine (Jamshidian analytic Europeans + tree/PDE Bermudans) with **2F** as the decorrelation upgrade; **(6)** **CMS**
via swaption-strip replication off the cube; **(7)** the term-structure-consistent **STIR convexity** from the Gaussian
model. Oracle throughout: QuantLib (primary) + ORE/QuantExt (cube/optionlet infra).

---

## 11. Cash-bond breadth & per-market conventions (UST / Gilt / Bund / JGB / OAT)

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|11.1|**Dirty (full) price = clean price + accrued interest**; the traded/quoted price is **clean**, settlement pays **dirty**. **Yield↔price is a round-trip** (price→yield→price is the identity) — the cash-bond structural check.|`Dirty = Σ c_i·DF(t_i) + R·DF(T)`; `Clean = Dirty − Accrued`. Accrued uses the market's day-count over the elapsed coupon fraction. The yield is the single rate that reprices the dirty price.|[NYU Stern — YTM/accrued/invoice](https://pages.stern.nyu.edu/~eelton/debt_inst_class/YTM.pdf), [Pass-1 §6 bond identity]|high (textbook-standard)|
|11.2|**Day-count & yield basis are per-market**: **UST** ACT/ACT (semi-annual, street yield); **Gilt** ACT/ACT, semi-annual; **Bund/OAT** ACT/ACT ICMA, **annual** coupon; **JGB** ACT/365 (simple-yield convention historically). These materially change accrued and yield.|UST/Gilt semi-annual; Bund & OAT annual ACT/ACT-ICMA (Eurobond govvies); legacy 30E/360 only on some pre-Euro/Swiss/Swedish paper. JGB uses a distinct simple-yield quoting convention. Convention is per-(market) data, never global.|[Wikipedia — day-count convention](https://en.wikipedia.org/wiki/Day_count_convention), [ACT Wiki — day-count](http://wiki.treasurers.org/wiki/Day_count_conventions), [DMO — About Gilts](https://www.dmo.gov.uk/responsibilities/gilt-market/about-gilts/)|high|
|11.3|**Settlement lag is per-market**: **UST T+1**; **Gilt T+1**; **JGB T+1** (since 2018-05-01, ~90% of flow); **Bund/OAT T+2** (European govvies). Drives the dirty-price valuation date.|JGB cut T+2→T+1 on 2018-05-01. UST/Gilt cash settle T+1; Eurozone govvies generally T+2. Settlement date sets the accrual end and the discount horizon.|[JPX/JSCC — JGB T+1](https://www.jpx.co.jp/jscc/en/cash/jgbcc/jgb_settlement_t1.html), [Wikipedia — T+2](https://en.wikipedia.org/wiki/T+2), [Achievable — settlement](https://app.achievable.me/study/finra-series-7/learn/bond-fundamentals-trading-settlement)|high|
|11.4|**Ex-dividend** (esp. **Gilts**): for trades settling inside the **ex-div period (7 business days before coupon)**, accrued is **negative (rebate)** — the seller pays the buyer rebate interest. Must be modelled, not ignored.|Gilt ex-div = 7 business days before the coupon date; a buyer settling in that window does not receive the imminent coupon, so accrued goes negative (rebate). Other markets have their own ex-div rules (some none).|[DMO — About Gilts](https://www.dmo.gov.uk/responsibilities/gilt-market/about-gilts/), [LSE — accrued interest guide](https://docs.londonstockexchange.com/sites/default/files/documents/accrued-interest-corp-supra.pdf)|high|

---

## 12. Bond spread measures (Z-spread / ASW / OAS) & FRNs

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|12.1|**Z-spread** = the constant spread added to the **whole zero/discount curve** so that discounting the bond's cashflows reproduces its market (dirty) price. It does **not** value embedded options.|Solve for `z` s.t. `Price = Σ c_i·exp(−(zero_i + z)·t_i)`. Uses the entire curve (unlike a single-point benchmark spread). Curve usually the swap/OIS zero curve.|[Wikipedia — OAS](https://en.wikipedia.org/wiki/Option-adjusted_spread), [Nikko AM — credit spreads](https://en.nikkoam.com/articles/2021/credit-spreads-explained)|high|
|12.2|**Asset-swap spread (ASW)** = the spread over the floating index paid on the floating leg of a **par asset-swap package** (buy the bond, swap its fixed coupons into float). A par-par ASW is the standard quote.|The asset-swap package combines the bond with an IRS so the investor receives index+ASW; the ASW isolates the bond's funding spread vs the swap curve. Differs from Z-spread by the par-vs-dirty packaging.|[Nikko AM — credit spreads](https://en.nikkoam.com/articles/2021/credit-spreads-explained), [Veridelisi — credit spreads 101](https://veridelisi.substack.com/p/credit-spreads-101-why-they-matter)|high|
|12.3|**OAS = Z-spread − option cost**; for a bond with **no** embedded options, **OAS = Z-spread** (the structural check). OAS needs a term-structure model (tree / Monte-Carlo) to value the option across rate scenarios.|`OAS = Z − optionCost`. For callable/putable bonds the option cost is computed by averaging over modelled rate paths (binomial tree or MC); OAS is the spread that, *after* removing optionality, reprices the bond.|[Wikipedia — OAS](https://en.wikipedia.org/wiki/Option-adjusted_spread), [AnalystPrep — OAS](https://analystprep.com/study-notes/cfa-level-2/explain-the-calculation-and-use-of-option-adjusted-spreads/)|high|
|12.4|**FRN discount margin (DM)** = the spread over the reference index that, added to the index, reprices the FRN to its market price (par on a reset date when DM = quoted margin). DM > quoted-margin ⇒ price < par; DM < quoted-margin ⇒ premium.|`Price = Σ (index+QM)·τ·FV·DF(index+DM) + FV·DF`; solve for `DM`. At reset with `DM = QM`, price = par — the FRN structural check. Simple/quoted/required-margin variants exist.|[AnalystPrep — FRN yield spreads](https://analystprep.com/cfa-level-1-exam/fixed-income/yield-spread-measures-for-floating-rate-instruments/), [MathWorks `floatdiscmargin`](https://www.mathworks.com/help/finance/floatdiscmargin.html)|high|

---

## 13. Inflation-linked bonds

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|13.1|An ILB's principal (and coupon, on a fraction of principal) is scaled by an **index ratio** = reference CPI(settlement) / reference CPI(issue). Cashflows discount on a **real curve**; the **breakeven** vs the nominal bond is market-implied inflation.|`IndexedPrincipal = Par · CPI_ref(t)/CPI_ref(base)`. Coupon = real-rate × indexed principal. Real yield drives price (ILB price ↑ as real yields ↓). Breakeven = nominal yield − real yield.|[Wikipedia — inflation-indexed bond](https://en.wikipedia.org/wiki/Inflation-indexed_bond), [ryanoconnellfinance — TIPS](https://ryanoconnellfinance.com/tips-inflation-indexed-bonds/)|high|
|13.2|**Indexation lag** is a per-programme convention (e.g. **TIPS: 3-month lag**, daily index ratio by linear interpolation of the monthly non-seasonally-adjusted CPI-U). Programmes differ in reference index, lag, and floor.|TIPS: principal references CPI-U ~3m earlier; between monthly releases the daily index ratio is linearly interpolated. UK linkers historically 8m (old) / 3m (new CPI-style) lag — the lag is config, not code.|[ryanoconnellfinance — TIPS](https://ryanoconnellfinance.com/tips-inflation-indexed-bonds/), [Wikipedia — inflation-indexed bond](https://en.wikipedia.org/wiki/Inflation-indexed_bond)|high|
|13.3|**Deflation floor**: many programmes (e.g. **TIPS**) guarantee **redemption ≥ par** at maturity even if the index ratio < 1 — an embedded option. The floor protects principal at maturity only (not premium paid in secondary, not interim coupons).|At maturity pay `max(IndexedPrincipal, Par)`; this is a put on the price index struck at par — must be modelled as optionality for accurate ILB pricing where the index ratio is near/below 1.|[ryanoconnellfinance — TIPS](https://ryanoconnellfinance.com/tips-inflation-indexed-bonds/), [Wikipedia — inflation-indexed bond](https://en.wikipedia.org/wiki/Inflation-indexed_bond)|high|

**Cash-bond breadth recommendation:** extend `celnet-rates` with a **bond product family** (fixed-coupon govvie with
per-market convention record, FRN, inflation-linked) + a **spread analytics** module (Z-spread, ASW, OAS, FRN discount
margin). OAS and the ILB deflation floor are the two pieces that **reach into the vol layer** (need a rate-tree/MC and a
price-index option respectively) — so they sequence *after* `celnet-rates-vol`. Conventions extend the Pass-1
`RatesConvention` schema with a `BondConvention` sibling (§15).

---

## 14. Credit / CDS — **scoping only** (gated behind operator Q3)

> Per charter §4.5 and Pass-1 Q3, credit is a **distinct later workstream**. This section **scopes** it — products,
> the standard model, the data, the oracle, the structural identity — and surfaces the licence trap. It does **not**
> design the crate.

| # | Claim | Detail | Source | Conf |
|---|---|---|---|---|
|14.1|**Single-name CDS** has two legs: **premium leg** (buyer pays running spread/coupon, weighted by survival probability) and **protection leg** (PV of `(1−R)·N` paid on default). **Par spread** = the spread equating the two legs.|`PV_premium = s·Σ τ_i·Q(t_i)·DF(t_i)` (+accrual-on-default); `PV_protection = (1−R)·∫ DF·(−dQ)`. Fair `s` ⇒ legs equal. Market trades **fixed coupon (100bp IG / 500bp HY) + upfront**; the model converts upfront↔spread.|[Baruch IRC Lecture 3 — CDS](https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture3_2019.pdf), [OpenGamma — Pricing & Risk of CDS](https://quant.opengamma.io/Pricing-and-Risk-Management-of-Credit-Default-Swaps-OpenGamma.pdf)|high|
|14.2|**Survival/hazard curve bootstrap**: assume **piecewise-constant hazard** (⇒ piecewise-exponential survival `Q`); bootstrap term-by-term from the shortest CDS maturity outward, exactly as the rate-curve bootstrap. Recovery standardised at **40% (IG) / 20% (HY)** for the ISDA convention.|`Q(t)=exp(−∫h)`; staircase hazard. Bootstrap shortest→longest reprices each CDS to its market quote — same fixed-point discipline as Pass-1 curve build (re-prices calibrating instruments = the anti-circular check).|[ISDA CDS Standard Model](https://www.cdsmodel.com/), [credule — credit curve bootstrap](https://cran.r-project.org/web/packages/credule/vignettes/credule.html), [OpenGamma CDS](https://quant.opengamma.io/Pricing-and-Risk-Management-of-Credit-Default-Swaps-OpenGamma.pdf)|high|
|14.3|**Structural identity ("credit triangle")**: `par spread ≈ hazard × (1 − R)` (flat-hazard, short-horizon). The engine-agnostic anti-circular check for CDS — independent of the bootstrap path.|`s ≈ h·(1−R)`. Also `RPV01` (risky annuity, PV of 1bp contingent on survival) converts running spread ↔ upfront and collapses to 0 on default — the CDS analogue of the swap annuity.|[Baruch IRC Lecture 3](https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture3_2019.pdf), [arxiv 0912.4623 (credit term structures)](https://arxiv.org/pdf/0912.4623)|high|
|14.4|**Index CDS** (CDX / iTraxx) = a portfolio of single names; **intrinsic spread ≈ RPV01-weighted average** of constituent spreads; carries an **index factor** (on-the-run survivorship) and trades vs intrinsic (skew).|Index MTM uses the ISDA Standard Model on a single index curve; intrinsic value uses the per-name curves. Index factor scales notional as names default/roll. Tranches/correlation are well beyond v1 scope.|[OpenGamma — Forward CDS, Indices & Options](https://quant.opengamma.io/CDS-Options-OpenGamma.pdf), [IHS Markit — CDS Indices Primer](https://cdn.ihsmarkit.com/www/pdf/1221/CDS-Indices-Primer---2021.pdf)|high|
|14.5|**LICENCE TRAP (load-bearing).** The **ISDA CDS Standard Model source** ships under the **"ISDA CDS Standard Model Public License v1.0"** — a **custom** licence with **assent-on-redistribution, indemnification, trademark, and patent-termination** clauses; **not OSI-approved, not FSF-approved**, and it would **fail the `cargo-deny` MIT/Apache/BSD allowlist**. **Do not vendor, link, or allowlist it.**|It is *called* "open source" but is **not** a permissive licence in the cargo-deny sense. The clean path: **QuantLib's `IsdaCdsEngine` (modified-BSD) re-implements the ISDA model methodology** — use it as the permissive oracle. `FinancePy` (GPL-3.0, Pass-1 §5) also covers CDS but only as a disposable out-of-process oracle.|[ISDA CDS Model Public License v1.0 text](https://bnikolic.co.uk/isdacdslicensev1), [QuantLib `IsdaCdsEngine`](https://rkapl123.github.io/QLAnnotatedSource/d3/dfb/class_quant_lib_1_1_isda_cds_engine.html), [cdsmodel.com](https://www.cdsmodel.com/)|high|

**Credit scoping verdict:** a **`celnet-credit`** workstream (NOT a `celnet-rates-vol` module) — single-name CDS +
hazard/survival bootstrap (recovery 40/20), upfront↔spread conversion, RPV01, then index CDS (intrinsic + index factor).
Tranches/correlation/CVA are explicitly **out of v1 credit**. **Oracle = QuantLib `IsdaCdsEngine` (BSD)** for the ISDA
methodology + the **credit-triangle** identity; **never** the ISDA Public-License source as a dependency. Gated behind
operator **Q3** (build now/later) — this pass only scopes it.

---

## 15. Config-schema sketch — the vol cube & bond conventions (extends Pass-1 §3)

Mirrors `docs/CONVENTIONS.md`'s per-(key) record discipline; a *schema shape*, not Rust. Identifiers are vendor-neutral
(no `SABR`/`HullWhite`/`Bachelier` in public names — method provenance in doc comments only).

```
VolQuoteBasis = Normal(bp) | Lognormal(pct) | ShiftedLognormal{ shift }   // carried as data on every vol object

VolCube {                                  // keyed per (currency, index)  — never a global default
  basis:         VolQuoteBasis             // the quoting world of the raw quotes
  axes: {
    expiry:      [Tenor]                   // option expiry  (1M…30Y)
    tenor:       [Tenor]                   // underlying swap tenor (1Y…30Y)
    strike:      [StrikeSpread]            // Δ-from-ATM; MUST contain 0 (= ATM); ATM = par swap rate
  }
  atm_surface:   Grid<expiry × tenor → Vol>          // the ATM slice
  smile:         Grid<expiry × tenor → SmileParams>  // per-cell smile fit (3 free params, backbone fixed)
  smile_model:   Asymptotic | NoArbWing | Collocation // arbitrage-free wing selector
  interp:        FitEarlyInterpolateLater            // interpolate smile params across (expiry,tenor)
}

OptionletSurface {                         // stripped from flat cap/floor term vols (per currency,index)
  basis:         VolQuoteBasis             // Normal | ShiftedLognormal
  strip_from:    CapFloorTermVols          // OptionletStripper1/2 analogue
  surface:       Grid<expiry × strike → Vol>
}

ShortRateModelConfig {                     // provenance: 1F/2F Gaussian — names in comments only
  factors:       One | Two                 // Two adds factor correlation (decorrelation)
  mean_reversion:[Real]                    // a (per factor)
  vol:           PiecewiseConstant         // σ(t), calibrated to ATM swaption diagonal
  factor_corr:   Option<Real>              // ρ, two-factor only
  numeric:       Analytic | Tree | Pde | MonteCarloLsm   // engine selector for Europeans/Bermudans
}

BondConvention {                           // sibling of Pass-1 RatesConvention; per (market)
  day_count:     DayCount                  // UST/Gilt ACT/ACT; Bund/OAT ACT/ACT_ICMA(annual); JGB ACT/365
  coupon_freq:   Frequency                 // semi-annual (UST/Gilt) | annual (Bund/OAT)
  settle_lag:    BusinessDays              // UST/Gilt/JGB T+1 ; Bund/OAT T+2
  ex_div:        Option<{ days, rule }>    // Gilt: 7 business days before coupon ⇒ negative (rebate) accrued
  yield_basis:   StreetYield | SimpleYield // JGB simple-yield is distinct
  inflation:     Option<{ reference_index, lag, daily_interp, deflation_floor: bool }>  // ILB programme
  frn:           Option<{ index, quoted_margin }>                                      // FRN
}
```

---

## 16. Per-product oracle plan (independent structural identity — anti-circular, extends Pass-1 §6)

Same rule: every number reaches a reference by **two independent routes** — (a) a genuinely different engine
(QuantLib / ORE / QuantExt; QuantLib `IsdaCdsEngine` for credit), AND (b) a structural identity that holds **regardless
of engine**. The identities below are the load-bearing anti-circular checks for Pass-2 products.

| Product | Independent structural identity (engine-agnostic) | + Engine oracle | Conf |
|---|---|---|---|
|**European swaption**|**Payer − Receiver = forward swap** `= A·(F−K)` (§7.5). **At-the-money payer = receiver**. **Normal↔lognormal ATM vol map** (§7.4) reconciles a bp-quoted price against a %-quoted engine. **PV ≥ 0** and ≥ intrinsic.|QuantLib `Swaption` (Black/Bachelier + `SabrSwaptionVolatilityCube`).|high|
|**Cap / floor**|**Cap(K) − Floor(K) = swap(K)** (§7.6) → cross-checks against the Pass-1 linear-swap engine (no shared code). **Cap = Σ caplets** (decomposition closes). **Stripping round-trip**: strip flat term vols → caplets → re-aggregate → reprice the original caps.|QuantLib `CapFloor` + `OptionletStripper1/2`.|high|
|**SABR smile**|**Smile reprices its own calibrating quotes** to tolerance (fit fixed-point). **No-arb wing**: implied density ≥ 0 across strikes (Hagan-asymptotic fails this at low strike ⇒ must match the No-Arb-SABR engine there).|QuantLib `SabrSwaptionVolatilityCube` vs `NoArbSabrSwaptionVolatilityCube`.|high|
|**Bermudan swaption**|**Bermudan ≥ max(co-terminal Europeans)** (early-exercise premium ≥ 0) and **≤ sum bound**. **Tree vs PDE vs LSM agree** to tolerance (three independent numerics on one model). **European limit**: a 1-exercise Bermudan = the European.|QuantLib `Swaption` under 1F/2F Gaussian (tree, FD, MC engines).|high|
|**CMS swaplet**|**CMS ≠ forward swap rate**: adjustment > 0 (Jensen convexity); the **replicating swaption strip reprices the CMS** independently of any CMS closed form (two routes agree). **CMS → 0 vol ⇒ CMS = forward swap rate** (adjustment vanishes).|QuantLib `CmsCoupon` (replication pricer) off the SABR cube.|high|
|**Govvie bond**|Pass-1 §6 bond identities **+ per-market accrued** (recompute UST ACT/ACT vs Bund ACT/ACT-ICMA-annual independently and match). **Gilt ex-div ⇒ negative accrued** (sign check). **Yield↔price round-trip**.|QuantLib `FixedRateBond` with the market's DayCount/Schedule + DMO/CME published accrued.|high|
|**FRN**|**At a reset with DM = quoted margin ⇒ price = par** (§12.4). **DM > QM ⇒ discount; DM < QM ⇒ premium** (monotonic sign check). **Z-spread of a fixed-coupon proxy** cross-checks the discount-curve construction.|QuantLib `FloatingRateBond` / `floatdiscmargin` analogue.|high|
|**Inflation-linked**|**Index ratio = CPI_ref(t)/CPI_ref(base)** recomputed from the published CPI + lag, matched independently. **Deflation floor = put on the index at par** ⇒ floored redemption ≥ par (option ≥ 0). **Breakeven = nominal − real** reconciles to the nominal bond.|QuantLib `CPIBond` / `ZeroCouponInflationSwap` + DMO/Treasury index ratios.|high|
|**Z-spread / OAS**|**No embedded option ⇒ OAS = Z-spread** (§12.3) — the decomposition closes. **Z-spread = 0 ⇒ bond reprices off the bare curve.** OAS via tree must equal OAS via MC (two numerics agree).|QuantLib bond + `OAS` / spread routines.|high|
|**Single-name CDS** *(scoping)*|**Credit triangle** `s ≈ h·(1−R)` (§14.3, flat hazard). **Bootstrap reprices its calibrating CDS** to par. **Upfront↔spread round-trip** via RPV01. **Protection leg → 0 as R → 1**.|**QuantLib `IsdaCdsEngine` (BSD)** — replicates the ISDA model; never the ISDA-PL source.|high|
|**Index CDS** *(scoping)*|**Intrinsic spread ≈ RPV01-weighted avg of constituents** (decomposition). **Index factor scales notional** (survivorship check). Index vs intrinsic = skew (sign/magnitude sanity).|QuantLib per-name `IsdaCdsEngine` aggregated to intrinsic.|high|

---

## A. Proposed crate sketch — `celnet-rates-vol` (extends Pass-1 §A `celnet-rates`)

Consistent with Pass-1's locked **D1** (fixed income is its own crate family on the shared seams). `celnet-rates-vol`
sits **above** `celnet-rates` and reuses `celnet-surface` (the existing vol-surface infra), exactly as Pass-1 §A foretold.

```
celnet-rates-vol                   (P-next — rates optionality)
├── depends on (one-way):
│     celnet-rates        (curves, discount × projection, par swap rate, annuity/PV01 — Pass-1)
│     celnet-surface      (existing vol-surface/cube infrastructure — REUSE, do not fork)
│     celnet-types        (Tenor, Currency, Money, Vol, day-count)
│     celnet-core         (pricing-trait seam)
│   (NO dependency on celnet-vanilla / FX leaves — D1)
│
├── modules:
│   ├── quote/            VolQuoteBasis: Normal(bp) | Lognormal(pct) | ShiftedLognormal; normal↔lognormal map
│   ├── analytic/         Bachelier (normal) + Black European swaption & caplet/floorlet closed forms
│   ├── smile/            4-param smile fit (backbone fixed; β=0 normal variant for negative rates)
│   │   ├── wing/         arbitrage-free wing: noarb | collocation   (provenance: Hagan-2014/Doust — comment only)
│   ├── cube/             VolCube (expiry×tenor×strike); fit-early-interpolate-later; ATM + smile-spread grids
│   ├── optionlet/        caplet/floorlet stripping → OptionletSurface (provenance: OpenGamma method — comment only)
│   ├── shortrate/        1F & 2F Gaussian: θ(t) curve-fit; analytic Europeans (zero-bond decomposition);
│   │   └── numeric/      tree | pde (operator-split) | montecarlo_lsm  — Bermudan early exercise
│   ├── cms/              CMS swaplet/cap/floor via replicating swaption strip off the cube (convexity)
│   └── convexity/        term-structure-consistent STIR futures/forward convexity from the Gaussian model
│
├── exposes (additive contract & 5 clients):
│     SwaptionInputs / CapFloorInputs / CmsInputs (NOT *HullWhite*/*Sabr* — guardrail §3.2)
│     VolCube / OptionletSurface (immutable, cheaply-cloned snapshots, like the Pass-1 CurveSet)
│     PricingResult { pv, vega, normal_vol, lognormal_vol, smile_greeks, exercise_value }
│     — surfaced via additive celnet.wire arms (Options | Fixed-Income | FI-Vol); risk folds into the
│       server-owned RiskService hierarchy (vega cube alongside the Pass-1 delta cube; no client loop-sum).
│
└── sibling (gated, operator Q3):  celnet-credit  (single-name CDS + hazard/survival bootstrap, RPV01,
        upfront↔spread, index CDS) — depends on celnet-rates (discount curve) only; NOT a celnet-rates-vol module.
        Oracle: QuantLib IsdaCdsEngine (BSD). Tranches/correlation/CVA out of v1.
```

**Scale/perf note (charter §3.5).** The IB-scale cost in the vol layer is the **vega cube** (instruments × cube cells ×
scenarios) and **Bermudan/CMS** valuation (path/lattice). Mitigations, all consistent with the pinned zero-alloc hot
core: (1) **analytic** Europeans/caplets wherever exact (Bachelier/Black + SABR closed form) — no simulation on the hot
path; (2) **immutable, cheaply-cloned cube snapshots** so many scenarios fan out in parallel; (3) **FD/tree** (not MC)
for 1F/2F Bermudans — FD outperforms trees and is deterministic (§10.3); reserve **MC+LSM** for genuinely
high-dimensional models; (4) the **CMS replication strip** is a fixed bucketed sum off the analytic cube — vectorisable;
(5) vega/risk folds into the **existing server-owned hierarchical-risk rollup**, never a client loop-sum.

Golden oracle for the crate: **QuantLib (primary)** — `Swaption`/`CapFloor`/`SabrSwaptionVolatilityCube`/
`NoArbSabrSwaptionVolatilityCube`/`OptionletStripper1+2`/1F-2F-Gaussian engines/`CmsCoupon` — **+ ORE/QuantExt**
(optionlet & cube infra at scale). Credit oracle: **QuantLib `IsdaCdsEngine` (BSD)**. (`rateslib` non-OSS and
`FinancePy` GPL-3.0 remain **excluded** as deps — Pass-1 §5; **ISDA-PL source excluded** — §14.5.)

---

## B. New open questions to append to `OPEN-QUESTIONS.md` (continuing Q-numbering from Pass-1's Q13 ⇒ Q14+)

> The agent surfaces these; the operator fills the Decision column. (Do not assume.)

| # | Question | Why it matters | Proposed default |
|---|---|---|---|
|Q14|**Default swaption/cap quoting basis for v1**: ship **normal (bp)** as the primary basis (negative-rate-safe), with lognormal & shifted-lognormal selectable? | Drives every vol object's representation and the whole negative-rate story; mixing bases silently misprices. | **Normal (bp) primary**, lognormal/shifted selectable per (ccy,index) as data. |
|Q15|**SABR backbone `β`**: fix `β` (market convention, e.g. 0.5 for positive-rate ccys, **0 = normal-SABR** for negative-rate ccys), or expose it as a calibrated/config param per currency? | `β` is conventionally pinned not fitted; the wrong choice distorts the skew and the negative-rate handling. | **Fix per (ccy): β=0 (normal-SABR)** where rates can be ≤0, else market-standard β; configurable, not free-fitted. |
|Q16|**Arbitrage-free wing**: which low-strike fix ships — **No-Arb-SABR (Doust)**, **Hagan-2014 PDE**, or **stochastic collocation**? Or ship Hagan-asymptotic for v1 and add the no-arb wing as a fast-follow? | Hagan-asymptotic admits negative densities at low strike (mis-prices far-OTM CMS/wings). | **No-Arb-SABR (Doust)** as the wing (QuantLib has it as a direct oracle); asymptotic for the core. |
|Q17|**Short-rate factor count for v1 optionality**: **1F Gaussian** (cheap, Jamshidian-analytic Europeans, FD/tree Bermudans) first, with **2F** as the decorrelation upgrade for CMS-spread / curve-shape products? | 1F can't decorrelate curve points (mis-prices CMS-spread & some Bermudans); 2F is costlier to calibrate/solve. | **1F first**; **2F** as a P-next+1 upgrade gated by demand for CMS-spread / curve-shape optionality. |
|Q18|**CMS depth for v1**: full **swaption-strip replication** (most accurate, smile-consistent) from day one, or a cheaper closed-form convexity approximation first? | Replication is the accurate, book-consistent method but costs a bucketed swaption strip per coupon. | **Replication off the cube** (it reuses the cube we already build); approximation only as a fast pre-trade estimate. |
|Q19|**ISDA CDS Standard Model licence**: confirm we **exclude the ISDA-PL source entirely** (non-OSI/FSF, assent+indemnity clauses, fails `cargo-deny`) and rely on **QuantLib `IsdaCdsEngine` (BSD)** for the ISDA methodology as oracle? | New, load-bearing licence trap (§14.5): "open source" ≠ permissive; vendoring/linking it would breach guardrail §3.1. | **Exclude ISDA-PL source; use QuantLib `IsdaCdsEngine` (BSD)** for ISDA-standard CDS numbers. Legal sign-off if ever reconsidered. |
|Q20|**Credit workstream timing (resolves Pass-1 Q3)**: build **`celnet-credit` (single-name + index CDS)** now as a parallel lane, or defer until rates-vol lands? | Credit is a disjoint crate (depends only on the discount curve); could run in parallel, but adds surface area. | **Defer** to a dedicated `W-FI` lane after `celnet-rates-vol`; scope is locked here so it can start cleanly. |
|Q21|**Inflation scope for v1 cash bonds**: ship **ILB pricing incl. the deflation-floor option** (needs a price-index option model), or linear-index ILB (ignore the floor) first? | The deflation floor is embedded optionality reaching into the vol layer; ignoring it mis-prices near/below-par index ratios. | **Linear-index ILB first**; add the **deflation-floor option** alongside `celnet-rates-vol` (shares the option machinery). |
|Q22|**OAS engine for callable/putable bonds**: reuse the **1F Gaussian tree/MC** (Q17) for OAS, or a dedicated bond-option model? | OAS needs a term-structure model; reusing the swaption short-rate engine keeps one calibrated model across the book. | **Reuse the 1F Gaussian** OAS engine (one calibrated model, book-consistent), validate `OAS = Z` on option-free bonds. |

---

## C. Recommended next research pass (Pass 3 — synthesis + the remaining tails)

Pass 1 (curves+linear) and Pass 2 (vol + cash breadth + credit scoping) now cover charter §4.1–§4.5 to depth. Pass 3
should **synthesise the corpus** and close the remaining tails:

1. **Synthesise the design corpus** (charter §6): `FI-CURVES-SPEC.md`, `FI-ANALYTICS-SPEC.md` (fold in Pass-2 vol +
   bonds), `FI-CONVENTIONS.md` (Pass-1 `RatesConvention` + Pass-2 `BondConvention`/`VolCube` schemas),
   `FI-ARCHITECTURE.md` (`celnet-rates` + `celnet-rates-vol` + gated `celnet-credit`, additive proto arms, 5-client
   surface), `FI-VERIFICATION-CONTRACT.md` (Pass-1 §6 + Pass-2 §16 oracle plan), `FI-ROADMAP.md` (`W-FI-1…n`).
2. **Remaining §4.6 cross-cutting tails**: **FRTB GIRR/CSR** sensitivity mapping (vega + curvature now in scope — ORE is
   the oracle), **repo/financing** curves, and the **five-client surface** detail (GUI vol-cube workspace, Excel
   `CELNET.*` swaption/cap/CMS functions, SDK, FIX, federation) — the integration §10 of the question map.
3. **`celnet-credit` design** (only if operator un-gates Q20): hazard/survival bootstrap, RPV01, upfront↔spread, index
   CDS — against QuantLib `IsdaCdsEngine` (BSD) + the credit-triangle identity.

---

### Sources (consolidated — Pass 2)

- MathWorks — Work with negative interest rates: https://www.mathworks.com/help/fininst/work-with-negative-interest-rates-using-functions.html
- MathWorks — Calibrate SABR using normal (Bachelier) vols with negative strikes: https://www.mathworks.com/help/fininst/calibrate-sabr-model-using-normal-volatilities-with-negative-strikes.html
- MathWorks — `swaptionbynormal` (Bachelier swaption): https://www.mathworks.com/help/fininst/swaptionbynormal.html
- MathWorks — `floatdiscmargin` (FRN discount margin): https://www.mathworks.com/help/finance/floatdiscmargin.html
- arxiv 1112.1782 — Equivalence of normal & lognormal implied vol (model-free): https://arxiv.org/pdf/1112.1782
- Wikipedia — Swaption (payer/receiver parity): https://en.wikipedia.org/wiki/Swaption
- Wikipedia — SABR volatility model: https://en.wikipedia.org/wiki/SABR_volatility_model
- Wikipedia — Hull–White model: https://en.wikipedia.org/wiki/Hull%E2%80%93White_model
- Wikipedia — Day count convention: https://en.wikipedia.org/wiki/Day_count_convention
- Wikipedia — Option-adjusted spread: https://en.wikipedia.org/wiki/Option-adjusted_spread
- Wikipedia — T+2 (settlement): https://en.wikipedia.org/wiki/T+2
- Wikipedia — Inflation-indexed bond: https://en.wikipedia.org/wiki/Inflation-indexed_bond
- ResearchGate — Arbitrage-free SABR (Hagan 2014): https://www.researchgate.net/publication/264718376_Arbitrage-free_SABR
- arxiv 2510.10343 — Learning the Exact SABR Model: https://arxiv.org/pdf/2510.10343
- arxiv 0901.1776 — Efficient swaption pricing in the 1F Hull-White model: https://arxiv.org/pdf/0901.1776
- arxiv 0912.4623 — A Guide to Modeling Credit Term Structures: https://arxiv.org/pdf/0912.4623
- Mathema — Swaption cube construction methodology: https://help.mathema.com.cn/latest/docs/fixedincome/swaptioncube
- Studocu — SABR & vol-cube construction methodology: https://www.studocu.com/en-us/document/university-at-buffalo/mathematical-finance-2/volatility-cube-construction/82566158
- MDPI Risks 10(12):232 — GP regression for swaption cube under no-arbitrage: https://www.mdpi.com/2227-9091/10/12/232
- QuantLib — `SabrSwaptionVolatilityCube` header: https://rkapl123.github.io/QLAnnotatedSource/d5/d22/sabrswaptionvolatilitycube_8hpp.html
- QuantLib — `OptionletStripper1`: https://rkapl123.github.io/QLAnnotatedSource/df/da8/class_quant_lib_1_1_optionlet_stripper1.html
- QuantExt/ORE — `OptionletStripper2`: https://www.opensourcerisk.org/docs/qle/class_quant_ext_1_1_optionlet_stripper2.html
- QuantLib — `IsdaCdsEngine` (BSD, replicates ISDA model): https://rkapl123.github.io/QLAnnotatedSource/d3/dfb/class_quant_lib_1_1_isda_cds_engine.html
- QuantLib/ORE mailing list — missing SABR cap/floor interpolation: https://sourceforge.net/p/quantlib/mailman/message/37877801/
- OpenGamma — An introduction to caplet stripping: https://quant.opengamma.io/Caplet-Stripping-OpenGamma.pdf
- OpenGamma — Pricing & Risk Management of CDS (ISDA model focus): https://quant.opengamma.io/Pricing-and-Risk-Management-of-Credit-Default-Swaps-OpenGamma.pdf
- OpenGamma — Forward CDS, Indices and Options: https://quant.opengamma.io/CDS-Options-OpenGamma.pdf
- HPC-QuantLib — Bermudan swaption pricing via finite-difference: https://hpcquantlib.wordpress.com/2011/12/19/bermudan-swaption-pricing-based-on-finite-difference-methods/
- Hagan — Convexity Conundrums: Pricing CMS Swaps, Caps and Floors (PDF): https://www.deriscope.com/docs/Hagan_Convexity_Conundrums.pdf
- Baruch MFE — IRC Lecture 3 (CDS mechanics & valuation): https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture3_2019.pdf
- Baruch MFE — IRC Lecture 5 (LIBOR/swaption options): https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture5_2019.pdf
- Baruch MFE — IRC Lecture 6 (Convexity & CMS): https://mfe.baruch.cuny.edu/wp-content/uploads/2019/12/IRC_Lecture6_2019.pdf
- Burgess — Convexity Adjustments Made Easy (SSRN): https://papers.ssrn.com/sol3/Delivery.cfm/SSRN_ID4052825_code1728976.pdf?abstractid=3401235&mirid=1
- Calibration of 1F & 2F Hull-White using swaptions (Univ. Bergamo): https://aisberg.unibg.it/retrieve/e40f7b8a-9809-afca-e053-6605fe0aeaf2/Calibration%20One-%20Two-Factor%20-%20final_SecondoInvi.pdf
- UK DMO — About Gilts (ex-div, accrued, conventions): https://www.dmo.gov.uk/responsibilities/gilt-market/about-gilts/
- ACT Wiki — Day count conventions: http://wiki.treasurers.org/wiki/Day_count_conventions
- LSE — Accrued interest guide (corp/supra): https://docs.londonstockexchange.com/sites/default/files/documents/accrued-interest-corp-supra.pdf
- JPX/JSCC — JGB T+1 settlement: https://www.jpx.co.jp/jscc/en/cash/jgbcc/jgb_settlement_t1.html
- NYU Stern — YTM, accrued interest, invoice price: https://pages.stern.nyu.edu/~eelton/debt_inst_class/YTM.pdf
- AnalystPrep — Yield spreads for floating-rate instruments (FRN DM): https://analystprep.com/cfa-level-1-exam/fixed-income/yield-spread-measures-for-floating-rate-instruments/
- AnalystPrep — Option-adjusted spread (OAS): https://analystprep.com/study-notes/cfa-level-2/explain-the-calculation-and-use-of-option-adjusted-spreads/
- Nikko AM — Credit spreads explained (Z-spread/ASW/OAS): https://en.nikkoam.com/articles/2021/credit-spreads-explained
- Veridelisi — Credit spreads 101: https://veridelisi.substack.com/p/credit-spreads-101-why-they-matter
- ryanoconnellfinance — Swaptions (payer/receiver, cap-floor parity): https://ryanoconnellfinance.com/swaptions/
- ryanoconnellfinance — TIPS & inflation-indexed bonds: https://ryanoconnellfinance.com/tips-inflation-indexed-bonds/
- ISDA CDS Standard Model (cdsmodel.com): https://www.cdsmodel.com/
- ISDA CDS Standard Model Public License v1.0 (text): https://bnikolic.co.uk/isdacdslicensev1
- IHS Markit — CDS Indices Primer (Nov 2021): https://cdn.ihsmarkit.com/www/pdf/1221/CDS-Indices-Primer---2021.pdf
- credule (R) — Credit curve bootstrapping vignette: https://cran.r-project.org/web/packages/credule/vignettes/credule.html
