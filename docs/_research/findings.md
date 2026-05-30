# Celnet — Research Findings (raw)

> Verbatim structured output from the research/design workflow (30 May 2026, 11 research agents).
> Source of truth for the curated docs in the parent directory. Brand renamed Celnet→Celnet.

## Market-standard analytics for vanilla FX options (as of May 2026)

FX vanilla options are priced with the Garman-Kohlhagen (GK) model, the FX specialization of Black-Scholes-Merton where the foreign interest rate r_f acts as a continuous dividend yield on the foreign-currency asset. The defining complication of FX (versus equity) is that there is no single unambiguous premium currency, delta, or ATM strike: each currency pair and tenor carries explicit market conventions. Desks quote the smile not by strike but by ATM volatility plus 25-delta and 10-delta risk reversals and butterflies, with brokers actually trading the "broker (market) strangle" rather than the theoretical "smile strangle," requiring a calibration step to recover smile-consistent vols. Correct analytics therefore require pinning down four delta conventions (spot/forward x premium-adjusted/unadjusted), the premium currency and pips-vs-percent quote style, the ATM convention (ATM-forward or delta-neutral straddle), date logic (horizon -> spot, expiry -> delivery, cut time), and settlement style (deliverable vs NDO/NDF cash fixing). The full FX Greek set extends beyond delta/gamma/vega/theta/rho to the volatility-of-volatility Greeks (vanna, volga/vomma) central to vanna-volga pricing, plus charm, speed, zomma. Authoritative references are Reiswich-Wystup (2010), Wystup "FX Options and Structured Products," Castagna-Mercurio vanna-volga, and Iain Clark "Foreign Exchange Option Pricing."

**Key Findings**

- Garman-Kohlhagen (1983) is the market-standard vanilla FX model: call = S e^{-r_f T} N(d1) - K e^{-r_d T} N(d2), with d1 = [ln(S/K) + (r_d - r_f + sigma^2/2)T] / (sigma sqrt(T)), d2 = d1 - sigma sqrt(T). It is BSM with the foreign rate r_f as the asset's dividend yield; equivalently price off the forward F = S e^{(r_d - r_f)T} via the Black-76 form with discounting at e^{-r_d T}.
- Domestic (DOM) = numeraire/quote ccy = CCY2; foreign (FOR) = asset/base ccy = CCY1 in the CCY1CCY2 pair (e.g. EURUSD: FOR=EUR, DOM=USD). r_d discounts the strike/cash leg, r_f the spot/asset leg.
- Four premium quotation styles exist: domestic pips (price per 1 unit FOR, in DOM, e.g. USD pips per EUR -> 'EURUSD'), %FOR (percentage of foreign notional), %DOM, and foreign pips. Convention is pair-specific; the premium currency determines whether delta must be premium-adjusted.
- Premium-included (premium-adjusted) vs premium-excluded (unadjusted) delta: when the option premium is paid in the FOR currency (e.g. EURUSD premium in EUR), the premium itself carries FX risk, so the hedge delta is reduced by the premium's delta -> premium-adjusted delta. If premium is in DOM (the numeraire), no adjustment -> unadjusted/raw delta.
- The four delta conventions: (1) spot delta unadjusted = e^{-r_f T} N(d1); (2) forward delta unadjusted = N(d1); (3) premium-adjusted spot delta = e^{-r_f T} (K/F) N(d2); (4) premium-adjusted forward delta = (K/F) N(d2). Choice is by pair AND tenor (short tenors typically spot delta, long tenors >~1-2Y forward/driftless delta; e.g. USDJPY <= 1Y spot premium-adjusted).
- ATM conventions are pair-specific: ATM-Forward (ATMF, K = F, the delta-neutral point only under unadjusted forward delta) vs Delta-Neutral Straddle (DNS, strike where call delta + put delta = 0 so a straddle has zero delta). DNS is the dominant interbank ATM for most G10/EM. Under unadjusted forward delta, DNS strike K = F exp(0.5 sigma^2 T); under premium-adjusted delta the DNS strike is lower (K = F exp(-0.5 sigma^2 T)).
- Smile is quoted as: ATM vol, Risk Reversal RR_25 = sigma(25d call) - sigma(25d put) (and RR_10), Butterfly BF_25 = 0.5*(sigma(25d call) + sigma(25d put)) - sigma_ATM (and BF_10). RR measures skew, BF measures convexity/smile curvature.
- CRITICAL distinction: the BROKER butterfly (market fly / market strangle) is what brokers actually trade and quote; the SMILE butterfly is the arithmetic-average convexity used to read the smile. The broker/market strangle uses a single vol (sigma_ATM + BF_broker) applied to BOTH 25d-call and 25d-put strikes (struck at separate strikes) and must reprice to the same value as the smile; recovering the smile requires a calibration (e.g. Reiswich-Wystup, Clark) because the two flies differ, sometimes materially, especially for high-RR/EM pairs.
- Vanna-volga (VV) method (Castagna-Mercurio 2007) is the market-standard light smile interpolation/pricing engine: it builds a hedge of ATM, RR and BF that zeroes out the BSM vega, vanna and volga of the target option and adds the market cost of that hedge to the flat-vol BSM price.
- Date conventions: Horizon (trade/today) -> Spot date (T+2 for most pairs, T+1 for USDCAD/USDTRY/USDRUB and same-currency-region pairs); Expiry date -> Delivery/settlement date computed from expiry by the SAME spot-lag rule (delivery = spot date relative to expiry). Tenors (1W,1M,...) are added to spot then adjusted by business-day/modified-following and end-of-month rules using BOTH currencies' (and USD's) holiday calendars.
- Cut (expiry time/fixing): NY cut = 10:00 AM New York time (the standard interbank OTC cut for most pairs and the CME FX option fix as a 60s VWAP ending 10:00:00 ET); Tokyo cut = 15:00 Tokyo time (3pm JST), standard for JPY-region/Asian business. The cut determines the fixing source/time for exercise.
- Day count for the vol/time-to-expiry is typically ACT/365 (calendar days expiry-horizon over 365); interest accrual uses each currency's money-market basis (ACT/360 for USD/EUR, ACT/365 for GBP/AUD etc.). Some desks separate 'expiry time' (vol time) from 'settlement discounting time'.
- Settlement: Deliverable options exchange full notional of both currencies on delivery date (physical). Non-Deliverable Options (NDO, on NDF pairs like USDKRW, USDTWD, USDINR, BRL, etc.) cash-settle in the convertible ccy (usually USD) at a published fixing (e.g. KFTC18/WMR/EMTA fixing) on the fixing date 1-2 days before settlement; NDF is the forward analogue.
- First-order Greeks on the desk: delta (spot or forward, adjusted/unadjusted per convention), vega = S e^{-r_f T} sqrt(T) phi(d1) (per 1 vol point /100), theta (time decay), rho-domestic = dV/dr_d (= K T e^{-r_d T} N(d2) for a call) and rho-foreign = dV/dr_f (= -S T e^{-r_f T} N(d1)) -- FX has TWO rhos.
- Second-order Greeks: gamma = e^{-r_f T} phi(d1)/(S sigma sqrt(T)); vanna = dDelta/dsigma = d^2V/dS dsigma = -e^{-r_f T} phi(d1) d2/sigma (skew/RR sensitivity); volga (vomma) = d^2V/dsigma^2 = vega * d1 d2 / sigma (convexity/BF sensitivity). Vanna and volga are the core VV Greeks.
- Higher-order Greeks tracked on FX desks: charm = dDelta/dt (delta decay / 'delta bleed'), speed = d^3V/dS^3 (rate of change of gamma w.r.t. spot), zomma = dGamma/dsigma (= d^3V/dS^2 dsigma), and color = dGamma/dt (gamma decay). Vega is also commonly reported as a vega ladder/bucketed surface vega.
- Implied vols are always quoted against GK delta (not strike), so converting a quoted delta to a strike requires solving K(delta) consistently in the same delta convention used to quote that pair/tenor -- a fixed-point/root-find because vol depends on strike via the smile.

**Recommendations**

- Model the GK/BSM core off the forward F = S e^{(r_d - r_f)T} and store the two discount factors (DF_d, DF_f) separately rather than continuous rates, so deliverable vs NDO and dual-curve discounting plug in cleanly.
- Encode the convention set as first-class per-(pair, tenor) configuration: {delta type in spot/fwd x adjusted/unadjusted, ATM type ATMF/DNS, premium ccy + pips/percent, spot lag, cut, day count}. Do not hardcode global defaults -- USDJPY, EURUSD, and EM NDFs differ.
- Implement strike<->delta as a robust root-finder (Brent/Newton) that respects the configured delta convention and uses the smile vol at the trial strike; premium-adjusted delta is non-monotone in strike for calls, so guard the solver (bracket on the correct branch, cap at the delta max).
- Build smile calibration as: take ATM(DNS), RR_25/10, broker BF_25/10; solve for the 25d/10d call and put strikes+vols such that the broker (market) strangle reprices -- i.e. distinguish broker fly from smile fly explicitly; then interpolate (vanna-volga or SVI/SABR in delta space) and verify no calendar/butterfly arbitrage.
- Expose two rhos (rho-domestic and rho-foreign) and the full higher-order Greek set (vanna, volga, charm, speed, zomma, color) since vanna/volga drive the VV mark and charm/gamma decay drive intraday hedging; report vega bucketed by tenor.
- Drive all dates off a calendar engine that intersects BOTH currency calendars (plus USD for cross-via-USD pairs), computes spot from horizon and delivery from expiry by identical lag rules, and applies modified-following + end-of-month; parameterize cut (NY 10am / Tokyo 3pm) per pair.
- For NDF/NDO pairs, model cash settlement at the named fixing (EMTA/WMR/central-bank fixings) with the fixing-to-settlement lag, and discount the cash payoff on the settlement currency curve -- the pricing vol surface is still GK but settlement and delivery logic differ from deliverables.
- Validate the engine against Reiswich-Wystup (2010) worked examples and Clark's reference numbers for each delta/ATM convention before trusting marks.

**Risks**

- Mixing delta conventions silently is the #1 error: a 25d strike under spot-unadjusted vs forward-premium-adjusted delta differs, so vols/strikes will be wrong if the pair's actual convention is not respected. Always confirm the convention per pair/tenor.
- Treating the broker (market) butterfly as the smile butterfly overstates/understates wing vols; for high-skew or EM pairs this can be a multi-vol-point error and produce arbitrageable smiles. The broker strangle must be repriced, not averaged.
- Premium-adjusted delta is not monotonic in strike for OTM calls -> naive Newton solvers can converge to the wrong root or diverge; bracket carefully.
- Under premium-adjusted forward delta the ATM-DNS strike is BELOW the forward (F exp(-0.5 sigma^2 T)), opposite to the unadjusted case (F exp(+0.5 sigma^2 T)) -- using the wrong sign mislocates ATM.
- Confusing the two rhos: FX has rho-domestic AND rho-foreign; a single 'rho' (as in equity BSM) is meaningless for FX risk.
- Spot lag is not universally T+2: USDCAD/USDTRY/USDPHP are T+1, and holiday calendars (incl. USD even for non-USD crosses settled via USD) shift spot AND delivery; getting delivery wrong corrupts discounting and the forward.
- Vanna-volga is an approximation/market-fit, not arbitrage-free; for far-OTM strikes, exotics, or very long tenors it can produce negative densities -- validate against a no-arbitrage smile (e.g. arbitrage-free Reiswich-Wystup construction) for wings.
- Vol time (ACT/365) and interest-accrual day count (ACT/360 etc.) differ; using one basis for both biases vega/discounting. Keep expiry-time and settlement-time distinct.
- ATM meaning is ambiguous (ATMF vs DNS, premium in/out) -- pulling a vendor ATM vol without its convention tag and applying it under a different convention mis-strikes the whole surface.

**Sources**

- https://www.researchgate.net/publication/275905055_A_Guide_to_FX_Options_Quoting_Conventions
- https://www.mathfinance.com/wp-content/uploads/2025/02/Wystup-FXcolumn-Greeks.pdf
- https://link.springer.com/article/10.1007/s11147-022-09189-9
- https://www.ncbi.nlm.nih.gov/pmc/articles/PMC9483449/
- https://www.quantpie.co.uk/fx/fx_summary.php
- https://www.quantpie.co.uk/fx/fx_rr_str.php
- https://arxiv.org/pdf/0904.1074
- https://finpricing.com/lib/FxVolIntroduction.html
- http://www.derivativeengines.com/25deltabfrr.aspx
- https://en.wikipedia.org/wiki/Foreign_exchange_date_conventions
- https://www.cmegroup.com/trading/fx/expiration-time-change-for-cme-fx-options.html
- https://www.thegoldensource.com/unlocking-the-complex-world-of-fx-options-simplifying-quoting-conventions/
- https://volquant.medium.com/almost-everything-you-wanted-to-know-about-fx-volatility-smile-part-i-intro-to-the-fx-market-4a3ba8052e08
- https://arxiv.org/pdf/2512.19621

## Market-standard FX volatility surface construction and smile models (as of May 2026)

FX vol surface construction is governed by FX-specific quoting conventions: smiles are built per-tenor from three liquid quotes (ATM, 25-delta risk reversal, 25-delta butterfly; with 10-delta added for liquid pairs), and the surface lives in delta space rather than strike space, under a sticky-delta assumption. The Vanna-Volga (VV) method of Castagna-Mercurio (2007) is the FX-native technique for inferring a full smile from those three quotes by adding the cost of hedging vega/vanna/volga to a Black-Scholes price; it is fast and closed-form but not guaranteed arbitrage-free at extreme strikes. SABR (Hagan et al. 2002), shifted/normal SABR, and arbitrage-free PDE SABR (Hagan 2014) are used as smooth, arbitrage-aware parametric smiles (more native to rates, but applied in FX). SVI/SSVI (Gatheral 2004; Gatheral-Jacquier 2014) parametrize total variance and give tractable, provably arbitrage-free surfaces when calibrated under butterfly and calendar constraints; SSVI ties slices via an ATM total-variance term structure. The critical FX-specific steps are correct delta/ATM convention handling and converting the quoted broker (market) strangle into the smile strangle before calibration; arbitrage control requires non-negative risk-neutral density (butterfly) and non-decreasing total variance in maturity (calendar).

**Key Findings**

- FX smiles are quoted/built from three per-tenor instruments: ATM volatility, 25-delta risk reversal (RR = skew/slope) and 25-delta butterfly (BF = curvature/kurtosis); liquid pairs add 10-delta RR/BF, giving a 5-point smile (ATM, 25dC/P, 10dC/P). This 3-quote structure is exactly why Vanna-Volga is FX-native.
- Vanna-Volga (Castagna & Mercurio, 'The vanna-volga method for implied volatilities', Risk 2007): builds a locally-replicating BS-vega-neutral portfolio of the three benchmark options and adds the smile hedging cost (vega, vanna = d-vega/d-spot, volga = d-vega/d-vol) to the flat BS price to get a smile-consistent implied vol for any strike/delta. A first-order and a second-order (asymptotically constant at extreme strikes) approximation exist.
- Vanna-Volga is FX-specific, fast, needs no numerical calibration and extrapolates beyond the 25-delta wings, but does NOT guarantee no-arbitrage and degrades at extreme strikes; practice applies corrections (Castagna-Mercurio second approximation; Bossens/Rayee/Wystup adjustments) and a survival-probability / first-exit-time weighting for first-generation exotics (barriers, one-touch).
- VV market-practice reference: Bossens, Rayee, Skantzos, Deelstra, 'Vanna-Volga methods applied to FX derivatives: from theory to market practice' (arXiv:0904.1074) documents survival-probability and vega-weighting adjustments for exotics.
- FX uses 4 delta conventions (spot delta, premium-adjusted spot delta, forward delta, premium-adjusted forward delta) and several ATM conventions (ATM-spot, ATM-forward, delta-neutral straddle/DNS, 50-delta). Short tenors typically use spot delta, long tenors forward delta; pairs quoted in a premium currency (many USD pairs) use premium-adjusted delta. Getting these right is the dominant FX-specific source of error.
- Surface interpolation is done in delta space, not strike space: a fixed delta maps to widely different strikes across tenors, so delta space gives better coverage and aligns with sticky-delta market behavior. delta-to-strike conversion is nonlinear and vol-dependent (Garman-Kohlhagen), requiring a root-solve per point.
- FX-critical conversion: the quoted BUTTERFLY is the broker/market strangle (a single tradeable strangle vol quoted under a smile-symmetry assumption), NOT the smile strangle (arithmetic average of 25d call/put smile vols). The market fly must be converted to a smile fly via iterative calibration before use, because the strangle strikes differ from the RR strikes (Reiswich & Wystup 2010/2012; Clark 2011).
- Canonical FX references: Iain Clark, 'Foreign Exchange Option Pricing: A Practitioner's Guide' (2011); Reiswich & Wystup, 'FX Volatility Smile Construction' (CPQF Working Paper 20, 2010/2012); Wystup, 'FX Options and Structured Products'.
- SABR (Hagan, Kumar, Lesniak, Woodward, 'Managing Smile Risk', Wilmott 2002): stochastic-vol model dF = alpha F^beta dW1, d(alpha) = nu alpha dW2, corr rho; parameters alpha (level), beta (backbone/CEV exponent), rho (skew), nu (vol-of-vol/curvature). Hagan's singular-perturbation expansion gives a closed-form implied (lognormal/Black) vol.
- Shifted SABR (F+shift) and Normal SABR (beta=0 with Bachelier/normal vols) are market practice for low/negative rates; more relevant to rates but used where forwards approach zero. Hagan's lognormal expansion breaks down near zero/negative forwards, motivating these variants.
- Arbitrage-free SABR (Hagan, Kumar, Lesniak, Woodward, 'Arbitrage-Free SABR', Wilmott 2014): reduces SABR to a 1-D effective forward PDE for the probability density solved numerically; removes the negative-density/arbitrage at low strikes present in the asymptotic formula while matching it elsewhere. Expansion validity needs sqrt(alpha), nu*sqrt(T) and |beta-1|*sqrt(T) not too large.
- SVI (Gatheral, 'A parsimonious arbitrage-free implied volatility parameterization', 2004; origin Merrill Lynch 1999): raw SVI gives total implied variance w(k) = a + b{rho(k-m) + sqrt((k-m)^2 + sigma^2)} in log-moneyness k; 5 parameters per slice, linear wings consistent with Lee's moment formula.
- SSVI / Surface SVI (Gatheral & Jacquier, 'Arbitrage-free SVI volatility surfaces', Quantitative Finance 14(1):59-71, 2014): a single surface parametrized by ATM total variance theta_t, a constant correlation rho and a curvature function phi(theta); gives explicit closed-form sufficient conditions for NO butterfly arbitrage and NO calendar-spread arbitrage across the whole surface.
- Arbitrage conditions: (1) Butterfly/static - risk-neutral density g(k) >= 0, equivalently Gatheral's g-function/Durrleman condition per slice (call prices convex in strike); (2) Calendar - total variance w(k,T)=sigma^2 T non-decreasing in T at fixed log-moneyness (no crossing on a total-variance plot); plus call-spread monotonicity in strike.
- Temporal/term-structure interpolation in FX interpolates ATM, RR and BF separately across tenors (often linear in ATM total variance, with business-time/event weighting for weekends, holidays and scheduled events), then rebuilds the smile per target date; total-variance interpolation preserves calendar no-arbitrage better than interpolating vols directly.
- Calendar arbitrage in FX surfaces is a recognized practical issue (MathFinance FX Column / Wystup et al. 2020): independent per-tenor RR/BF interpolation can create crossed total-variance curves; SVI/SSVI total-variance constraints are used to detect and remove it.
- Recent (2024-2026) trend: comparative and hybrid approaches - Del Valle Saragoni, 'Arbitrage-Free Volatility Surface Construction: SVI, SABR, SSVI, and Vanna-Volga Calibration Methods' (SSRN 2025); SABR-informed machine learning, 'SABR-Informed Multitask Gaussian Process' (arXiv:2506.22888, 2025) using calibrated-SABR synthetic data to regularize sparse-data fits; neural correctors with calendar/butterfly penalty terms.
- Practical model split: Vanna-Volga and broker-fly-calibrated delta-space smiles (Clark / Reiswich-Wystup) dominate FX vanilla desks for speed and exact reproduction of the 3-5 quotes, while SVI/SSVI and arbitrage-free SABR are layered on for arbitrage-clean, smooth, continuously-interpolated production surfaces and exotics/local-vol bootstrapping.

**Recommendations**

- Build the FX surface in delta space with explicit, configurable delta/ATM conventions per currency pair and tenor (spot vs forward delta, premium-adjusted vs plain, ATM-DNS vs ATM-forward). Convention errors dwarf model errors; source conventions from Clark (2011) and Reiswich-Wystup tables.
- Always convert the quoted broker (market) strangle into the smile strangle via the iterative calibration before fitting any smile model; never treat the quoted BF as the arithmetic 25d smile-strangle, or 10d wings will be mispriced.
- Use Vanna-Volga (Castagna-Mercurio second approximation) as the fast vanilla smile engine that exactly reprices the 3 quotes, but layer an arbitrage check (density non-negativity per slice; non-decreasing total variance in tenor) and fall back to / cross-check against SSVI or arbitrage-free SABR when VV produces negative densities at the wings.
- For a globally arbitrage-free, smoothly interpolated production surface (needed for local-vol / exotics), prefer SSVI (Gatheral-Jacquier 2014) parametrized on an ATM total-variance term structure with constant rho and a curvature function, since it gives closed-form butterfly + calendar no-arbitrage guarantees.
- Where a stochastic-vol smile is wanted (smooth backbone, dynamic-hedging interpretation), use SABR with Hagan's expansion for speed but switch to the arbitrage-free PDE SABR (Hagan 2014) for low-vol/low-rate or deep-wing strikes; use shifted/normal SABR only if forwards can approach or cross zero.
- Interpolate ATM/RR/BF across tenors in total-variance / business-time terms with event and weekend/holiday vol weighting, then reconstruct smiles per target date, rather than interpolating implied vols directly, to preserve calendar no-arbitrage.
- Validate every constructed surface with Bloomberg OVML-style checks: reprice the input ATM/RR/BF exactly, verify positive density (butterfly), monotone total variance (calendar) and call-price monotonicity in strike.

**Risks**

- Vanna-Volga is not arbitrage-free: it can produce negative implied densities and crossing in the wings, especially for high RR/BF (skewed/kinked smiles) and short tenors; do not use raw VV for exotics or local-vol stripping without correction.
- Mishandling the broker fly vs smile fly distinction is the most common production bug; it silently biases the wings and breaks 10-delta reproduction.
- Hagan's SABR asymptotic implied-vol formula is inaccurate and can be arbitrageable at low/extreme strikes and long maturities (validity needs nu*sqrt(T), |beta-1|*sqrt(T) small); use arbitrage-free PDE SABR there.
- SABR's beta is weakly identified from a single smile (beta and rho are partially confounded); fix beta from market convention/backbone behavior rather than free-fitting it.
- Raw SVI can admit butterfly and/or calendar arbitrage if fitted slice-by-slice without constraints; only constrained SVI or SSVI guarantees no static arbitrage. Independent per-tenor RR/BF interpolation commonly introduces calendar arbitrage (crossed total-variance curves).
- Delta-to-strike conversion is vol-dependent and needs a robust root-solver; premium-adjusted delta makes the call-delta non-monotonic and can yield two solutions / a maximum-delta strike, so handle the premium-adjusted branch explicitly.
- Term-structure interpolation that ignores weekends, holidays and scheduled events (central-bank meetings, fixings) produces unrealistic short-dated ATM vols; naive calendar-time interpolation is a known pitfall.
- Newer ML/Gaussian-process surface methods (2025) are promising for sparse data but can violate no-arbitrage unless penalties/constraints are explicitly enforced and verified; treat as research-grade, not drop-in production.

**Sources**

- https://www.deriscope.com/docs/The_Vanna_Volga_method_for_implied_volatilities_Castagna_Mercurio_2007.pdf
- https://arxiv.org/pdf/0904.1074
- https://www.risk.net/derivatives/equity-derivatives/1506580/vanna-volga-method-implied-volatilities
- https://www.researchgate.net/publication/264718376_Arbitrage-free_SABR
- https://onlinelibrary.wiley.com/doi/abs/10.1002/wilm.10290
- https://arxiv.org/pdf/1204.0646
- https://arxiv.org/pdf/1210.7111
- https://mfe.baruch.cuny.edu/wp-content/uploads/2015/06/VW3.pdf
- https://arxiv.org/pdf/1804.04924
- https://www.semanticscholar.org/paper/Series-No-.-20-FX-Volatility-Smile-Construction-and-Reiswich-Wystup/277709d212faba2b6defb210f6cc3c5757026545
- https://link.springer.com/article/10.1007/s11147-022-09189-9
- https://www.mathfinance.com/wp-content/uploads/2025/01/FX_Column_2020-03-calendar-arbitrage-SVI.pdf
- https://github.com/cfrm17/BrokerStrangleAlgorithm
- https://timxiao1203.github.io/brokerStrange.html
- https://papers.ssrn.com/sol3/papers.cfm?abstract_id=6197858
- https://arxiv.org/pdf/2506.22888
- https://volquant.medium.com/almost-everything-you-wanted-to-know-about-fx-volatility-smile-part-i-intro-to-the-fx-market-4a3ba8052e08
- https://arxiv.org/pdf/1810.07457

## Market-standard FX exotic options pricing and stochastic/local volatility models (as of May 2026)

FX exotics pricing in 2026 is split between fast quoting/risk-managing tools and full model-based valuation. First-generation exotics (single/double barriers, one-touch, no-touch, DNT, European digitals) are still routinely quoted using the Vanna-Volga (VV) "traders' rule of thumb" that overlays smile cost (vanna + volga) on the Black-Scholes/Garman-Kohlhagen theoretical value, calibrated to the three standard FX smile instruments (ATM, 25-delta risk reversal, 25-delta butterfly). For booking, hedging, and second-generation/path-dependent exotics (window/partial barriers, TARFs/accumulators, Asians, lookbacks, forward-starts), the market standard is a calibrated Local-Stochastic Volatility (LSV) model that combines a Dupire local-vol leverage function with a Heston-type stochastic-vol backbone, calibrated via the particle method (Gyongy/Markovian-projection identity with a kernel-estimated conditional expectation). Numerical engines are PDE finite-difference (Crank-Nicolson with Rannacher smoothing, ADI for 2D LSV) for low-dimension barriers/digitals and Monte Carlo (Andersen QE scheme for Heston/LSV, Brownian-bridge barrier correction, Sobol quasi-MC with Brownian-bridge dimension ordering) for high-dimension path-dependent products. Pure Heston is now rarely used standalone for exotics because it cannot fit the short-dated smile and exaggerates vol convexity; pure local vol mis-prices forward-smile/barrier-touch products, so LSV (with a mixing/blending weight between LV and SV dynamics) is the dominant production model.

**Key Findings**

- FX smile is quoted via three market instruments per tenor: ATM volatility (typically delta-neutral straddle), 25-delta (and often 10-delta) Risk Reversal (RR = vol of call minus vol of put), and 25-delta Butterfly/strangle (BF = average of 25d call/put vols minus ATM). These feed both smile construction and VV pricing.
- Vanna-Volga (VV) is the market-standard fast/indicative pricer for first-generation FX exotics (one-touch, no-touch, DNT, single/double KO/KI barriers, European digitals). It adjusts the BS/Garman-Kohlhagen TV by the cost of a hedging portfolio in ATM/RR/BF that neutralizes vega, vanna (dVega/dSpot) and volga/vomma (dVega/dVol).
- For barriers and touch options VV uses a survival-probability (no-touch probability) weighting p so the smile correction is scaled by the probability the option survives to maturity / has not yet touched; common variants weight by p or use first-exit-time/symmetry adjustments. VV does not guarantee arbitrage-free prices and needs these heuristic dampers for touches near the barrier.
- Garman-Kohlhagen (BS with two rates, domestic r_d and foreign r_f as a dividend yield) is the FX vanilla/closed-form benchmark; barrier/touch closed forms (Reiner-Rubinstein/Merton-style reflection-principle formulas) give the BS TV that VV then corrects.
- Local Volatility (Dupire) calibrates exactly to the vanilla surface via the Dupire forward equation (local vol as a function of strike/maturity from call-price derivatives); it is well-suited to path-dependent payoffs like TARFs but produces a flattening/unrealistic forward smile and mis-prices forward-vol-sensitive and barrier products.
- Heston stochastic vol: variance follows CIR sqrt process (mean-reversion kappa, long-run theta, vol-of-vol sigma, correlation rho); has a closed-form characteristic function solving Riccati ODEs, priced via Carr-Madan FFT, Lewis, or the COS (Fourier-cosine) method. Feller condition 2*kappa*theta >= sigma^2 governs whether variance can hit zero; it is frequently violated in FX calibrations, requiring full-truncation/QE schemes that remain valid regardless.
- Heston alone cannot match the short-dated FX smile and over-states long-dated convexity, so it is used as the stochastic backbone inside LSV rather than standalone for exotics.
- Local-Stochastic Volatility (LSV / SLV) is the production market standard: dS = ... + L(S,t)*sqrt(v)*S dW, where L(S,t) is a leverage function multiplying a Heston-type stochastic variance. It calibrates exactly to vanillas (via leverage) while retaining realistic forward-smile/spot-vol dynamics (via SV).
- LSV calibration uses Gyongy's theorem / Markovian projection: the leverage L(S,t)^2 = sigma_Dupire(S,t)^2 / E[v_t | S_t = S]. The conditional expectation E[v|S] is estimated by the particle method (Guyon-Henry-Labordere) using a kernel/regression estimator over a simulated cloud of particles, giving a McKean-Vlasov SDE solved forward in time.
- A 'mixing weight' (eta) blends LV vs SV dynamics in LSV: eta=0 is pure local vol, eta=1 is full stochastic vol on top of leverage; the weight is tuned to match exotic/forward-smile market quotes (e.g., barrier/touch reversals) since vanillas are matched for any eta.
- TARFs (Target Redemption Forwards) and accumulators/decumulators are path-dependent: a strip of forwards/options that knocks out when accumulated client gain reaches a target; pricing requires Monte Carlo (or low-dim PDE/quadrature for approximations) under LV or LSV, and they carry significant gap/digital risk. Risk.net, ION, and d-fine note resurgent corporate/retail volumes (e.g., UBS reported large YoY growth in accumulator/TARF activity).
- Window/partial barriers (barrier active only on a sub-interval of the option life, possibly with multiple windows and per-window rebates) are priced by finite-difference PDE with time-dependent boundary conditions or by MC; Heston with piecewise-constant parameters has been used for efficient window-barrier pricing (Goutte/Ismail-type work).
- Variance swaps are statically/perfectly replicated by a log contract: a continuum of OTM puts and calls weighted 1/K^2 plus a dynamic 1/S_t stock position; strike = model-free integral of OTM option prices. Volatility swaps are NOT statically replicable (sqrt is concave/convex in variance) and require a convexity adjustment (Carr-Lee robust replication, or model-based).
- Monte Carlo for continuously-monitored barriers suffers discrete-monitoring bias; corrections are (a) Broadie-Glasserman-Kou continuity shift of the barrier by ~0.5826*sigma*sqrt(dt), and (b) Brownian-bridge exit-probability between time steps using the closed-form max/min distribution of a Brownian bridge.
- Quasi-Monte Carlo with Sobol' low-discrepancy sequences plus Brownian-bridge (or PCA) path construction concentrates variance in the first few dimensions, materially outperforming pseudo-random MC for path-dependent FX exotics; PCA construction often beats Brownian bridge but is costlier.
- Asian options: geometric-average Asians have closed-form (lognormal product) solutions; arithmetic-average Asians have no exact closed form and use moment-matching (Turnbull-Wakeman/Levy/Curran), PDE dimension reduction (Rogers-Shi/Vecer), or MC with control variates (geometric Asian as control variate). FX TARFs are effectively averaging/strip structures.
- Lookbacks (floating/fixed strike on path max/min) and forward-start options (strike set at a future fixing, key for cliquets) are highly sensitive to forward volatility/forward smile, which is precisely where pure local vol fails and LSV/Heston dynamics matter.
- PDE finite-difference standard: Crank-Nicolson with Rannacher time-stepping (start-up implicit steps) to damp oscillations from non-smooth payoffs (digitals/barriers); ADI schemes (Craig-Sneyd / Hundsdorfer-Verwer) for the 2D Heston/LSV PDE; log-spot grids and barrier-aligned nodes for accuracy.
- Arbitrage-free FX smile construction (e.g., SABR, SVI, or the Garman-Kohlhagen-delta-based parametrizations such as Reiswich-Wystup / Clark) is the upstream input; VV is increasingly checked against or replaced by arbitrage-free interpolation that coincides with VV between the 25d call and 25d put.

**Recommendations**

- Adopt a two-tier architecture: a fast Vanna-Volga engine for indicative quotes and risk on first-generation exotics (touches, DNTs, single/double barriers, European digitals), and a calibrated LSV engine for booking, hedging Greeks, and all path-dependent/second-generation products.
- Implement LSV as Heston-backbone + Dupire leverage with particle-method calibration (Gyongy identity, kernel-estimated E[v|S]); expose a mixing/blending weight to recalibrate to market barrier/touch and forward-smile quotes since vanillas are matched for any weight.
- Build the vanilla surface first with an arbitrage-free, FX-delta-aware smile (handle premium-adjusted vs unadjusted deltas, ATM straddle convention, RR/BF strangle conventions) before deriving Dupire local vol; validate VV against the arbitrage-free smile between 25d wings.
- Use PDE finite-difference (Crank-Nicolson + Rannacher start-up, ADI Craig-Sneyd/Hundsdorfer-Verwer for 2D LSV) for low-dimension barriers, digitals, and window barriers; place grid nodes on barrier levels and use log-spot coordinates.
- Use Monte Carlo for TARFs, accumulators, Asians, lookbacks, multi-window/multi-asset products: Andersen QE discretization for the variance process, Sobol' quasi-MC with Brownian-bridge (or PCA) path ordering, geometric-Asian/vanilla control variates, and Brownian-bridge or Broadie-Glasserman-Kou continuity correction for continuously-monitored barriers.
- For TARFs specifically, model gap/digital risk explicitly and stress the target/knock-out feature; decide LV-vs-LSV per desk policy (LV is often deemed sufficient for vanilla strips but LSV is safer where forward smile/barrier dynamics matter) and reserve for model risk.
- Replicate variance swaps with the model-free log-contract OTM option strip (1/K^2 weights) for a model-independent strike; price volatility swaps with an explicit convexity adjustment (Carr-Lee robust replication or LSV-model expectation), never static replication.
- Calibrate Heston via FFT/COS pricing with analytic gradients in a nonlinear least-squares fit; use full-truncation or QE Monte Carlo so the engine is robust when the Feller condition is violated (common in FX).
- Validate exotic prices against multiple methods (VV vs LSV-PDE vs LSV-MC) and track barrier/touch hedge cost vs realized; reconcile against broker DNT/one-touch quotes which are the de-facto market calibration targets for the SV mixing weight.

**Risks**

- Vanna-Volga is a heuristic, not arbitrage-free: it can produce negative prices/probabilities or values outside [0, notional] for touches/DNTs near the barrier without survival-probability and first-exit-time dampers; do not use it for booking or Greeks on second-generation exotics.
- Pure Local Volatility produces a flat/unrealistic forward smile and systematically mis-prices forward-vol-sensitive products (forward-starts, cliquets, barrier reverse knock-outs, touch options); it under/over-hedges vega-gamma cross (vanna/volga).
- Pure Heston cannot fit the short-dated smile and exaggerates long-dated vol convexity; using it standalone for exotics mis-prices wings and gives poor short-tenor barrier/touch values.
- LSV calibration via the particle method is numerically delicate: the conditional expectation E[v|S] becomes unstable in low-density regions (deep wings, t->0), causing leverage blow-ups; needs kernel-bandwidth tuning, particle counts in the 10^5-10^6 range, regularization, and careful initial time-stepping.
- Feller condition is routinely violated in FX Heston/LSV calibrations; Euler discretization of CIR variance then produces negative variances and bias - must use full-truncation or Andersen QE schemes.
- Continuously-monitored barriers priced by naive discrete-step Monte Carlo are biased (overstate survival); failing to apply Brownian-bridge or Broadie-Glasserman-Kou (0.5826*sigma*sqrt(dt)) corrections gives materially wrong barrier/touch prices.
- Digital and barrier payoffs are discontinuous: PDE schemes without Rannacher smoothing oscillate and give unstable Greeks; grids not aligned to barrier levels lose accuracy/convergence order.
- TARFs/accumulators carry asymmetric, leveraged, path-dependent risk with documented history of large client losses and litigation/fines (per Risk.net and Fideres reporting); model and reserve for gap risk, correlation of fixings, and the embedded knock-out optionality, and ensure suitability/disclosure.
- Volatility swaps cannot be statically replicated; ignoring the convexity adjustment (treating vol swap strike = sqrt of var swap strike) systematically misprices and under-hedges.
- FX delta/ATM conventions (premium-adjusted vs unadjusted delta, spot vs forward delta, straddle vs forward ATM, strangle/smile vs broker BF) vary by currency pair and tenor; mishandling them corrupts the smile, Dupire local vol, and every downstream exotic price.
- VV smile-implied volatilities can deviate from arbitrage-free interpolation outside the 25d-call/25d-put range, especially in stressed regimes, introducing wing mispricing.
- Quasi-MC error estimation is harder than pseudo-MC (no simple variance estimate); requires randomized QMC (scrambled Sobol') to get confidence intervals.

**Sources**

- https://arxiv.org/pdf/0904.1074
- https://www.mathfinance.com/wp-content/uploads/2024/10/FXOptionsVannaVolga.pdf
- https://arxiv.org/abs/2211.12652
- https://help.mathema.com.cn/latest/docs/pricing/vv
- https://help.mathema.com.cn/latest/docs/pricing/slv
- https://arxiv.org/pdf/1911.00877
- https://arxiv.org/pdf/1701.06001
- https://arxiv.org/pdf/1906.06478
- https://arxiv.org/html/2504.14343
- https://www.frontiersin.org/journals/artificial-intelligence/articles/10.3389/frai.2019.00004/full
- https://iongroup.com/blog/markets/using-a-local-volatility-model-for-the-evaluation-of-tarf-fx-options/
- https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2606957
- https://www.risk.net/definition/target-redemption-forward-tarf
- https://www.d-fine.com/en/news/dfine-aspects-an-afternoon-on-tarfs/
- https://www.fideres.com/banks-push-risky-tarf-products-despite-history-of-billion-dollar-fines-and-settlements/
- https://arxiv.org/pdf/1805.04704
- https://www.tandfonline.com/doi/full/10.1080/23322039.2019.1598835
- https://www.sciencedirect.com/science/article/abs/pii/S0165188900000026
- https://en.wikipedia.org/wiki/Barrier_option
- https://en.wikipedia.org/wiki/Asian_option
- https://digitalcommons.usu.edu/cgi/viewcontent.cgi?article=1315&context=gradreports
- https://emanuelderman.com/wp-content/uploads/1999/02/gs-volatility_swaps.pdf
- https://math.uchicago.edu/~rl/rrvd.pdf
- https://business.columbia.edu/sites/default/files-efs/pubfiles/3967/pricing_hedging.pdf
- https://en.wikipedia.org/wiki/Volatility_swap
- https://arxiv.org/pdf/1707.04293
- https://www.acenumerics.com/miscellaneous/monte-carlo-pricing-of-continuously-monitored-barrier-options-with-heston
- https://arxiv.org/pdf/1906.11002
- https://arxiv.org/pdf/1511.08718
- https://en.wikipedia.org/wiki/Heston_model
- https://repository.tudelft.nl/file/File_bf06c85f-2c44-4fef-b94d-9d41e3e6704f
- https://link.springer.com/article/10.1007/s11147-022-09189-9
- https://pmc.ncbi.nlm.nih.gov/articles/PMC9483449/
- https://volquant.medium.com/almost-everything-you-wanted-to-know-about-fx-volatility-smile-part-i-intro-to-the-fx-market-4a3ba8052e08

## (untitled)



**Sources**

- https://synoption.com/
- https://synoption.com/optimus.php
- https://synoption.com/primus.php
- https://synoption.com/optionstradingvenue/
- https://thefullfx.com/synoption-launches-options-portfolio-management-tool/
- https://thefullfx.com/synoption-orbit-markets-team-up-in-crypto-options-market/
- https://www.businesswire.com/news/home/20200128005896/en/Singapore-Based-Startup-SynOption-Launches-Venue-for-FX-Options-Trading
- https://tradetechfx.wbresearch.com/sponsors/synoption
- https://www.techjockey.com/detail/synchro
- https://www.crunchbase.com/organization/synoption
- https://www.cbinsights.com/company/synoption
- https://e-forex.net/unlocking-more-opportunities-with-new-fx-option-trading-models-and-platforms/
- https://www.nomuraconnects.com/focused-thinking-posts/fx-options-embracing-the-potential-of-platforms/

## (untitled)



**Sources**

- https://www.fenicsmd.com/
- https://www.fenicsmd.com/products/fx-options/
- https://www.fenicsmd.com/products/
- https://www.prnewswire.com/news-releases/fenics-market-data-expands-fx-options-fxo-offering-with-ai-and-new-analytics-301419519.html
- https://thefullfx.com/fenics-upgrades-fx-options-vol-offering/
- https://wilmott.com/vol-surfaces-fmd-expands-fx-options-offering/
- https://www.marketswiki.com/wiki/Fenics_Market_Data
- https://www.bgcg.com/fenics-market-data-expands-fx-pricing-data-services/
- https://www.prnewswire.com/news-releases/fenics-market-data-expands-fx-pricing-data-services-300932853.html
- https://www.lseg.com/en/data-analytics/financial-data/pricing-and-market-data/fixed-income-pricing-data/fenics-market-data
- https://home.fenicsdirect.com/
- https://home.fenicsdirect.com/platform-details/
- https://www.fenicsfx.com/fenics-fx-platform-details/
- https://www.fmxfx.com/
- https://www.kacefinancial.com/kace-fxo/
- https://www.kacefinancial.com/solutions/pricing-and-analytics/
- https://www.financemagnates.com/institutional-forex/technology/fenics-launches-lsv-based-fx-options-pricing-software-for-exotics/
- https://www.fx-markets.com/tech-and-data/7948619/fenics-kace-launches-fx-options-oms

## (untitled)



**Sources**

- https://www.bloomberg.com/professional/insights/webinar/pricing-fx-options-tips-tricks/
- https://data.bloomberglp.com/professional/sites/10/750114_Real-Time-Volatilities.pdf
- https://www.bloomberg.com/professional/products/risk/mars/
- https://www.fif.com/index.php?option=com_content&view=article&id=21480
- https://costbench.com/software/financial-data-terminals/bloomberg-terminal/
- https://godeldiscount.com/blog/bloomberg-terminal-cost-2026
- https://www.ice.com/fixed-income-data-services/data-and-analytics/derivatives/valuations
- https://www.ice.com/fixed-income-data-services/data-and-analytics/pricing-and-analytics/derivatives
- https://www.murex.com/en/insights/brochure/mx3-fx-options-trading
- https://www.murex.com/en/insights/article/murex-fx-options-product-spotlight-modeling-capacity
- https://www.murex.com/en/solutions/technology/mx3-architecture
- https://www.capco.com/services/consulting/murex/murex-total-cost-of-ownership
- https://www.gartner.com/reviews/product/mx3-platform
- https://www.numerix.com/crossasset
- https://www.numerix.com/product/CrossAsset-SDK
- https://www.numerix.com/numerix-cloud
- https://www.numerix.com/product/numerix/oneview
- https://www.fenicsmd.com/products/fx-options/
- https://www.fenicsmd.com/wp-content/uploads/2025/02/FenicsMarketData_FlysheetFXOSolutions_v6.pdf
- https://www.fx-markets.com/tech-and-data/7948619/fenics-kace-launches-fx-options-oms
- https://www.digitalvega.com/fx-options
- https://www.digitalvega.com/execution
- https://www.marketaxess.com/trade/rfq-hub
- https://investor.marketaxess.com/news/news-details/2025/MarketAxess-Acquires-Majority-Control-of-RFQ-hub-Holdings-LLC/default.aspx
- https://www.deutsche-boerse.com/dbg-en/markets-services/trading/ps-trading-models-services/ps-models-fx-360t
- https://www.financemagnates.com/institutional-forex/deutsche-brse-units-360t-and-qb-combine-forex-trading-capabilities/
- https://www.360t.com/
- https://www.quantifisolutions.com/derivatives-valuation/
- https://www.quantifisolutions.com/intel-and-quantifi-accelerate-derivative-valuations-by-700x-using-ai-on-intel-processors/
- https://quantra.io/
- https://github.com/avhz/RustQuant
- https://www.acuitykp.com/blog/rust-financial-technology/
- https://developer.nvidia.com/blog/accelerating-python-for-exotic-option-pricing/
- https://www.mdpi.com/2076-3417/15/18/9961

## Best practices for ultra-low-latency, mission-critical Rust services (as of May 2026)

As of May 2026, the dominant pattern for ultra-low-latency mission-critical Rust services is a thread-per-core architecture with cores pinned to isolated CPUs, a custom global allocator, and pre-allocated zero-alloc hot paths communicating over wait-free SPSC ring buffers. Runtime choice is bimodal: tokio remains the safe, ecosystem-rich default for IO-bound services and anything touching gRPC/HTTP, while io_uring thread-per-core runtimes (monoio, glommio) win throughput/latency benchmarks but trade away maturity, portability, and the `Send + Sync` ecosystem. For the truly latency-critical inner loop (market data, matching, risk), the winning approach is often NOT an async runtime at all but busy-polling pinned OS threads with lock-free structures (crossbeam, arc-swap, seqlock) and rkyv/zero-copy or hand-rolled binary/FIX wire formats. Tail latency (p99/p99.9), not mean, is the optimization target — so the goal is eliminating sources of jitter: allocations, syscalls, page faults, cache misses, false sharing, and scheduler preemption. mimalloc/jemalloc, huge pages, core pinning, and SIMD round out the toolkit, and divan/criterion plus perf/flamegraph drive measurement.

**Key Findings**

- Runtime split: tokio (latest 1.4x line, mature work-stealing multi-thread scheduler) is the default for IO-bound and gRPC/HTTP services; monoio (ByteDance, thread-per-core, pure io_uring) benchmarks ~2x tokio at 4 cores and ~3x at 16 cores, but is comparatively less actively maintained and lags the fast-moving io_uring kernel interface.
- glommio (Datadog, thread-per-core, Seastar-style, 3 io_uring rings per thread, proportional-share scheduler) offers richer high-level APIs than monoio; monoio reportedly saves ~25-33% CPU vs glommio at lower latency. At single-core/few-connections, io_uring runtimes can have HIGHER latency than tokio's epoll.
- Thread-per-core eliminates work-stealing cross-core cache traffic and lets futures/data stay thread-local (no Send/Sync requirement), but you lose tokio's ecosystem (tonic, axum, hyper, sqlx all assume tokio + Send).
- For the most latency-critical inner loop, the 2026 consensus is to avoid async entirely: use dedicated busy-polling OS threads pinned to isolated cores, communicating via wait-free SPSC queues. Async is reserved for the IO edge.
- arc-swap (docs.rs/arc-swap) is the standard for atomically swappable read-mostly config/state (Atomic<Arc<T>> semantics, lock-free reads via hybrid hazard-pointer/generation-lock); ideal for hot-reloadable reference data and routing tables.
- seqlock pattern (e.g. seqlock crate / ShabbirHasan1/seqlock-rust) is the canonical low-latency primitive for a single writer publishing small POD structs (price/quote snapshots) to many readers with no reader-side contention; readers retry on torn reads.
- left-right (the left_right crate) gives wait-free reads with eventually-consistent writes for larger read-mostly maps; good when arc-swap's whole-struct swap is too coarse.
- crossbeam provides the lock-free toolbox: ArrayQueue (bounded MPMC), SegQueue (unbounded), crossbeam-epoch for safe memory reclamation, and crucially crossbeam-utils CachePadded to prevent false sharing.
- SPSC ring buffers: rtrb (mgeier/rtrb) is the realtime-safe / wait-free SPSC standard with CachePadded heads/tails; ringbuffer-spsc (Mallets) is a no_std power-of-two/bitmask-masked alternative that avoids modulo in the hot path. Both are preferred over channels for the hottest hops.
- Allocators (2026 benchmarks): mimalloc leads small-allocation tail latency (~15% lower p99 than jemalloc, ~22% lower than tcmalloc) due to segment-based free lists and cache locality; jemalloc scales better across many threads (~30% higher throughput) with a longer production track record; for steady-state pre-allocated systems the gap narrows to ~3%. Use tikv-jemallocator (successor to jemallocator) for jemalloc, or the mimalloc crate.
- SIMD: std::simd / portable_simd is STILL nightly-only as of May 2026 (no stabilization shipped) — requires pinned nightly. For stable Rust, the wide crate (Lokathor/wide) is the recommended portable SIMD abstraction (uses safe_arch intrinsics on x86_64/aarch64-neon/wasm32); slightly slower than std::simd in some cases but stable. std::arch intrinsics remain the escape hatch for target-specific hand-tuning.
- Serialization: rkyv 0.8.11 is the zero-copy / zero-deserialization framework of choice for internal IPC and persistence (access archived data directly from the byte buffer; supports no_std and no_alloc), benchmarking faster than bincode/capnp/flatbuffers/prost/postcard. tonic (~0.13) + prost (~0.13) on the tokio/hyper stack is the gRPC standard for service boundaries; hand-rolled binary or FIX for the exchange wire.
- False sharing: CachePadded (crossbeam-utils) around per-thread/atomic hot fields aligns to cache-line boundaries (64B, or 128B on Apple/some x86 to cover the adjacent-line prefetcher) and is described as the single most important SPSC optimization.
- Core pinning: core_affinity crate pins threads to specific cores; pair with kernel isolcpus + nohz_full + rcu_nocbs to remove isolated cores from the scheduler and timer interrupts. NUMA awareness matters because cross-node access triggers TLB shootdowns/page faults; allocate memory on the local node and pin accordingly.
- Huge pages reduce TLB misses and page-fault jitter (e.g. vm.nr_hugepages, or transparent huge pages with caution); pre-fault and mlock memory at startup to avoid first-touch page faults on the hot path.
- Determinism: pre-allocate all buffers/object pools at startup; never allocate, never syscall, never lock, and never log synchronously on the hot path. Use bounded queues to a separate logging/telemetry core. Rigtorp's Low Latency Tuning Guide is the canonical Linux tuning reference (C-states disabled, P-states/turbo fixed, IRQ affinity, isolcpus).
- Benchmarking: criterion remains the statistically rigorous standard; divan is the simpler, faster newer alternative that also measures allocations and benchmarks generic functions. iai/iai-callgrind give instruction-count (deterministic, CI-friendly) measurements. For profiling use cargo-flamegraph + Linux perf; CodSpeed/Bencher for continuous benchmarking in CI. Measure p99/p99.9 with HdrHistogram, not just mean.

**Recommendations**

- Adopt a two-tier architecture: a non-async, busy-polling, core-pinned hot path for the latency-critical inner loop (market data ingest, matching, risk checks) and a tokio-based async edge for gRPC/HTTP/control-plane. Connect the tiers with rtrb SPSC ring buffers, not channels.
- Default the async edge to tokio (1.4x line) for ecosystem (tonic ~0.13, prost ~0.13, hyper, axum). Only reach for monoio/glommio if you have a measured, IO-uring-bound workload AND can accept reduced maturity and the loss of Send/Sync ecosystem crates. Prototype-benchmark before committing.
- Set a custom global allocator: start with mimalloc (best small-alloc tail latency) for allocation-heavy parsing paths; use tikv-jemallocator for highly multi-threaded throughput-oriented services. Benchmark both against your real workload — the gap is workload-dependent and small in steady state.
- Make hot paths zero-alloc: pre-allocate object pools and ring buffers at startup, reuse buffers, prefer arrayvec/smallvec/heapless and fixed-size types, and assert no allocation in the hot loop (e.g. with a counting allocator guard in tests or dhat in CI).
- Use the right concurrency primitive per access pattern: arc-swap for whole-struct read-mostly config; seqlock for a single writer publishing small POD snapshots to many readers; left_right for larger read-mostly maps; crossbeam ArrayQueue for bounded MPMC; rtrb/ringbuffer-spsc for SPSC. Wrap all shared hot atomics/fields in CachePadded.
- Pin each hot thread to a dedicated isolated core via core_affinity, and configure the kernel with isolcpus + nohz_full + rcu_nocbs for those cores; allocate and mlock + pre-fault memory on the local NUMA node and enable huge pages to remove page-fault and TLB jitter.
- For internal IPC, shared-memory, and persistence use rkyv 0.8 (zero-copy access, no_std/no_alloc capable). For service-to-service RPC use tonic+prost. For the exchange wire implement a hand-rolled zero-copy binary/FIX parser (consider rkyv or manual byte parsing with SIMD field scanning).
- Use SIMD via the wide crate to stay on stable Rust (FIX/message field scanning, batch math); only pin nightly for std::simd or drop to std::arch intrinsics where a measured hotspot justifies target-specific tuning.
- Build a measurement harness from day one: criterion or divan microbenchmarks, iai-callgrind for deterministic CI gates, HdrHistogram for production p99/p99.9 latency, and cargo-flamegraph + perf for profiling. Track tail latency as the primary KPI, regression-gate it in CI.
- Tune the host per Rigtorp's Low Latency Tuning Guide: disable deep C-states, fix CPU frequency/turbo, set IRQ affinity away from isolated cores, disable hyperthreading on critical cores, and warm up code paths (JIT-free but warm caches/branch predictors) before going live.

**Risks**

- monoio/glommio lock you out of the Send/Sync tokio ecosystem (tonic, sqlx, axum, most middleware) and monoio's maintenance pace lags the fast-evolving io_uring interface — a long-term maintenance and security risk for mission-critical code.
- io_uring itself has had a notable history of kernel security vulnerabilities; some hardened/locked-down production environments disable it entirely, which would break monoio/glommio. Verify your kernel policy and version.
- Benchmarks (including rtrb's own #39 and vendor numbers) are workload-specific and 'deeply flawed' out of context — never adopt a runtime/allocator on published numbers alone; measure your own access pattern and message sizes.
- std::simd is still nightly in May 2026; building production on it means pinning a nightly toolchain (supply-chain and reproducibility risk). Prefer wide on stable unless a profiled hotspot demands otherwise.
- Custom allocators interact with security features and memory profiling; mimalloc's secure mode costs ~10% performance, and swapping allocators can change RSS/fragmentation behavior — validate memory growth under sustained load.
- False sharing is silent: forgetting CachePadded on adjacent atomics/per-core counters can cost 2-10x in the hot path with no compile error. Audit struct layout and consider 128-byte padding for prefetcher adjacency.
- seqlock readers can observe torn reads and must retry; using it for non-POD/non-Copy or large structs, or assuming a single writer when there are several, leads to UB or livelock. Restrict to small Copy types and a single writer.
- Logging, metrics, and panics are stealth latency killers: synchronous logging, lock-based metrics, or an allocation in a tracing macro on the hot path injects tail jitter. Offload all telemetry over a bounded queue to a non-critical core.
- Thread-per-core can underperform tokio at low connection counts / single core, and naive sharding can create hot-shard imbalance; validate under realistic and skewed load, not just uniform synthetic load.
- Transparent huge pages can cause unpredictable stalls (khugepaged compaction); explicit hugetlbfs + pre-fault + mlock is more deterministic than relying on THP for latency-critical regions.

**Sources**

- https://github.com/bytedance/monoio/blob/master/docs/en/benchmark.md
- https://lib.rs/crates/monoio
- https://www.datadoghq.com/blog/engineering/introducing-glommio/
- https://iggy.apache.org/blogs/2026/02/27/thread-per-core-io_uring/
- https://docs.rs/arc-swap
- https://github.com/ShabbirHasan1/seqlock-rust
- https://lib.rs/concurrency
- https://oneuptime.com/blog/post/2026-01-30-how-to-build-a-lock-free-data-structure-in-rust/view
- https://markaicode.com/real-time-trading-systems-rust-latency-optimization/
- https://github.com/mgeier/rtrb
- https://docs.rs/rtrb/latest/rtrb/struct.RingBuffer.html
- https://github.com/Mallets/ringbuffer-spsc
- https://dev.to/codeapprentice/low-latency-rust-building-a-cache-friendly-lock-free-spsc-ring-buffer-in-rust-ddm
- https://docs.rs/rkyv
- https://docs.rs/crate/rkyv/latest
- https://david.kolo.ski/blog/rkyv-is-faster-than/
- https://crates.io/crates/tikv-jemallocator
- https://github.com/tikv/jemallocator
- https://stratcraft.ai/nexusfix/news/memory-allocator-benchmarks-2026
- https://www.kunalganglani.com/blog/jemalloc-vs-malloc-tcmalloc-p99-latency
- https://microsoft.github.io/mimalloc/bench.html
- https://doc.rust-lang.org/std/simd/index.html
- https://pythonspeed.com/articles/simd-stable-rust/
- https://github.com/Lokathor/wide
- https://shnatsel.medium.com/the-state-of-simd-in-rust-in-2025-32c263e5f53d
- https://docs.rs/core_affinity/latest/core_affinity/
- https://rigtorp.se/low-latency-guide/
- https://manuel.bernhardt.io/posts/2023-11-16-core-pinning/
- https://nikolaivazquez.com/blog/divan/
- https://nnethercote.github.io/perf-book/benchmarking.html
- https://hegdenu.net/posts/performance-optimization-flamegraph-divan/
- https://docs.rs/tonic
- https://lib.rs/crates/tonic

## Rust best practices (May 2026) for Celnet: multi-crate workspace layering, zero-downtime hot upgrades for stateful low-latency services, and user-extensible analytics plugin/SDK architecture

Celnet is a greenfield repo (only a .git directory exists at /Users/adrian/code/celeroption), so this is architectural guidance for a fresh start. For a mission-critical, low-latency product, the consensus 2026 practice is a flat virtual Cargo workspace layered by domain function (not technical tier): a dependency-free core/domain layer, shared types/wire-protocol crates, infra adapters, and thin binary crates on top, with workspace-wide dependency and lint pinning. For zero-downtime upgrades of stateful low-latency services, the durable pattern is SO_REUSEPORT-based graceful socket handoff (now also achievable via eBPF reuseport steering) combined with explicit live state handoff over a versioned, evolution-safe wire protocol; blue-green and shadow-pod handoff (per the 2026 SHADOW paper) are the complementary orchestration layers. For user-extensible analytics (custom pricing models/workflows), WebAssembly via wasmtime 45.0.0 + the Component Model + WASI 0.2.x is the recommended primary path because it gives a true security sandbox plus deterministic, fuel-metered execution that native ABI plugins fundamentally cannot match. abi_stable is effectively unmaintained (last release Oct 2023) and stabby (72.1.x, RC Jan 2026) is healthier but still carries native-code risk; reserve them for trusted first-party plugins only. Trait-object registries remain the right choice for compiled-in, first-party extensions.

**Key Findings**

- Repo is empty/greenfield: /Users/adrian/code/celeroption contains only .git, no Cargo.toml exists yet — this is architecture guidance, not a review of existing code.
- wasmtime latest stable is 45.0.0, released 2026-05-21 (very recent); it is the reference runtime and first to fully implement WASI Preview 2 and the Component Model.
- WASI 0.2.10 shipped as expected to be the last 0.2.x release; WASI Preview 3 (P3) release-candidate work is underway as of early 2026 (adds native async).
- wasmtime 45 has experimental component-model support for map<K,V> in WIT, with struct support targeted next — useful for analytics data passing but not yet stable.
- Wasmtime fuel metering is fully deterministic (same input + same fuel = interrupt at same instruction) but expensive; epoch-based interruption is ~10% overhead and faster but non-deterministic.
- For deterministic pricing-model execution you must use fuel (not epochs) plus Config::wasm_floats/NaN canonicalization and disable non-deterministic features to get reproducible results across hosts.
- abi_stable latest is 0.11.3 from 2023-10-12 — effectively stale/unmaintained; risky as a foundation for a new mission-critical plugin ABI.
- stabby latest stable is 72.1.1 (2025-05-12) with 72.1.2-rc1 (2026-01-18); it models layout incl. niches via the IStable trait associated types, giving niche-optimized stable-ABI enums/sum-types that abi_stable cannot.
- prost (Protobuf for Rust) latest is 0.14.3 (2026-01-10); Protobuf now has a year-based Editions system with a minimum_required_edition field to gate incompatible descriptors.
- Recommended flat virtual-manifest workspace layout (matklad/rust-analyzer style) for 10k–1M LOC projects; root is a virtual manifest, crates named identically to their folders, prefixes NOT stripped.
- Layer by domain function not technical tier: dependency-free core/domain crate, shared types crate, wire-protocol crate, infra/adapter crates, thin binary crates — new interfaces (gRPC, admin CLI) added as separate thin crates.
- Use workspace.dependencies and workspace.lints to pin versions and lint policy centrally; single shared Cargo.lock and target dir; deny warnings in CI for mission-critical code.
- SO_REUSEPORT remains the core primitive for graceful socket handoff during in-place restarts; eBPF SO_REUSEPORT (BPF_PROG_TYPE_SK_REUSEPORT) now allows custom steering so the old process drains while the new one accepts new connections with zero dropped connects.
- 2026 SHADOW paper (arXiv 2603.25484) describes a ShadowPod handoff for stateful microservices: shadow created on target while source serves, cutting restore phase up to 92%, eliminating downtime, reducing migration time up to 77% with zero message loss.
- Protobuf/prost schema-evolution rules for versioned wire protocols: never reuse/renumber field tags, reserve removed numbers, add fields as optional, avoid lossy oneof changes; this enables old/new process versions to interoperate during handoff.
- Wasmtime sandboxing uses the Linker to inject host functions, exposing only an explicit capability set (capability-based security) — the host decides exactly what market data / pricing primitives a plugin can touch.

**Recommendations**

- Initialize a flat virtual-workspace: [workspace] root with members under crates/. Suggested crates: celer-core (pure domain: pricing math, order/position model, zero IO and zero framework deps), celer-types (serde/wire DTOs), celer-proto (versioned wire protocol), celer-engine (the stateful low-latency service), celer-plugin-api (the SDK/host trait + WIT world), celer-plugin-host (wasmtime embedding), celer-server / celer-cli (thin binaries). Keep core dependency-free so it compiles fast and is trivially testable.
- Pin everything centrally via [workspace.dependencies] and enforce policy via [workspace.lints]; commit one Cargo.lock; deny warnings + run cargo-deny (licenses/advisories/bans) and cargo-audit in CI for the mission-critical tree.
- Make WebAssembly + wasmtime 45.0.0 + Component Model + WASI 0.2.x the PRIMARY, default path for user/third-party pricing models and workflows. Untrusted code must be sandboxed; only Wasm gives you that plus determinism. Define the plugin contract as a WIT 'world' so plugins can be authored in Rust, and later other languages.
- For deterministic pricing: enable fuel metering (Config::consume_fuel(true)), set per-call fuel budgets to bound runtime, canonicalize NaNs / pin float behavior, disable threads/SIMD-nondeterminism and any wall-clock/random host imports unless explicitly seeded. Treat the fuel budget as the latency/compute SLA per plugin invocation.
- Use a trait-object registry (e.g. an inventory of `dyn PricingModel`) ONLY for compiled-in, first-party models that ship in the binary — fastest path, no ABI/sandbox concerns. Keep the same trait shape that the Wasm host implements so first-party and user plugins are interchangeable behind one registry.
- If you genuinely need native dynamic .so/.dylib plugins (e.g. trusted partner C++/Rust models needing full native speed and no sandbox), prefer stabby 72.1.x over abi_stable — abi_stable's last release is Oct 2023 and should be considered unmaintained. Gate native plugins behind code-signing and a trusted-publisher allowlist; never load untrusted native plugins.
- For zero-downtime upgrades: implement in-place rolling restart using SO_REUSEPORT so the new process binds the same port, the old one stops accepting and drains in-flight work, then hands off live state over a Unix-domain socket / shared memory using celer-proto. Add a /readyz + connection-drain phase so the LB/eBPF steering only routes new connections to the warm new instance.
- Version the wire protocol explicitly: embed a protocol/schema version in every message header, use prost 0.14.x with strict Protobuf evolution discipline (reserve removed tags, additive optional fields, never renumber). This is what makes old<->new process state handoff and rolling upgrades safe. Maintain N and N-1 compatibility as a hard CI gate.
- Adopt blue-green at the orchestration layer for risky upgrades and SHADOW-style shadow-instance state pre-warming for the lowest-downtime stateful migrations; keep the in-process SO_REUSEPORT handoff for fast routine restarts and blue-green for schema-breaking releases.
- Treat the plugin SDK as a versioned product: ship celer-plugin-api with semver, generate guest bindings from the WIT world, provide a deterministic test harness (replay fixed market-data snapshots through the fuel-metered sandbox and assert bit-identical pricing output) so user models are reproducible and auditable.
- Track WASI P3 / component-model struct & map support: design the plugin data interface to migrate to native component async (P3) and richer types later, but build today on the stable WASI 0.2.x surface to avoid depending on experimental WIT features in production.

**Risks**

- abi_stable is effectively unmaintained (no release since Oct 2023) — do not build a new mission-critical native plugin ABI on it.
- Native dynamic plugins (abi_stable/stabby/cdylib) provide NO security sandbox and NO determinism guarantee: a buggy or malicious user pricing model can corrupt memory, crash the whole low-latency engine, or block the hot path. Never load untrusted native code.
- stabby's latest is a release candidate (72.1.2-rc1, Jan 2026) and its versioning (72.x) is unusual; validate ABI stability across your exact toolchain before relying on it, and pin compiler versions for native plugin builds.
- Wasm fuel metering adds meaningful overhead vs native — for ultra-hot per-tick pricing loops, benchmark; you may need to compile first-party hot models natively (trait registry) and reserve Wasm for user-supplied / less hot-path models.
- Component Model map/struct WIT support in wasmtime 45 is still experimental — relying on it in production risks breakage; stick to stable WASI 0.2.x interface types until those land stably.
- SO_REUSEPORT handoff does not by itself migrate in-flight session/order state — without an explicit, versioned state-transfer protocol you will drop or corrupt stateful connections during upgrade.
- Wire-protocol evolution mistakes (renumbering Protobuf fields, lossy oneof edits, removing-without-reserving) silently corrupt state during mixed-version rolling upgrades; enforce N/N-1 compatibility tests in CI.
- Putting the main package at the workspace root (instead of a virtual manifest) forces --workspace flags everywhere and pollutes the root — avoid; use a virtual root.
- Determinism leaks: any host import exposing wall-clock time, RNG, threads, or non-canonical floats to plugins destroys reproducibility of pricing results and breaks audit/replay — lock these down at the Linker.
- Kubernetes StatefulSet rolling updates do NOT give true zero-downtime for stateful low-latency services out of the box; relying on default k8s rollout will cause latency spikes/drops — you must implement app-level handoff.

**Sources**

- https://matklad.github.io/2021/08/22/large-rust-workspaces.html
- https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
- https://reintech.io/blog/cargo-workspace-best-practices-large-rust-projects
- https://infoworld.com/article/4050654/organize-rust-projects-for-faster-compilation-with-cargo-workspaces.html
- https://github.com/bytecodealliance/wasmtime/releases
- https://crates.io/api/v1/crates/wasmtime
- https://docs.wasmtime.dev/wasip2-plugins.html
- https://docs.wasmtime.dev/examples-deterministic-wasm-execution.html
- https://docs.wasmtime.dev/examples-interrupting-wasm.html
- https://blog.iamcristhian.dev/2026/04/webassembly-component-model-wasm-beyond-browser-2026
- https://eunomia.dev/blog/2025/02/16/wasi-and-the-webassembly-component-model-current-status/
- https://github.com/ZettaScaleLabs/stabby
- https://crates.io/api/v1/crates/stabby
- https://crates.io/api/v1/crates/abi_stable
- https://docs.rs/abi_stable/
- https://stevana.github.io/towards_zero-downtime_upgrades_of_stateful_systems.html
- https://ebpfchirp.substack.com/p/ebpf-powered-load-balancing-for-so_reuseport
- https://arxiv.org/pdf/2603.25484
- https://github.com/tokio-rs/prost
- https://crates.io/api/v1/crates/prost
- https://oneuptime.com/blog/post/2026-01-24-protocol-buffer-evolution/view
- https://instagit.com/protocolbuffers/protobuf/schema-versioning-strategies-for-protobuf-compatibility/

## Cross-platform GPU acceleration in Rust for derivatives pricing (Monte Carlo + PDE), macOS/Metal + Windows/Linux CUDA/Vulkan/DX12 + containers, as of May 2026

For a cross-platform Rust derivatives-pricing engine that must run on Apple M-series (Metal), Windows/Linux NVIDIA (CUDA) plus Vulkan/DX12, and in containers, the strongest single bet in May 2026 is CubeCL (tracel-ai, v0.10.0, May 2026): one #[cube] Rust kernel compiles on demand to CUDA, ROCm/HIP, Metal/Vulkan/WGSL (all via wgpu), and a CPU-SIMD runtime, giving native CUDA speed on NVIDIA and automatic graceful degradation to CPU when no GPU is present. The dominant hard constraint is precision: Metal and WebGPU/WGSL have no native f64 (WGSL types are only f32/f16/i32/u32; atomics are integer-only), so a truly portable engine must standardize on f32 on GPU and reserve f64 for a CPU path or NVIDIA/Vulkan-CUDA where double precision is real. Counter-based RNGs (Philox 4x32) are the correct GPU RNG choice because they are stateless, reproducible per-path from (path_index, step) counters, and vectorize across all backends; Sobol QMC is feasible on GPU via precomputed direction numbers but needs care with Brownian-bridge/dimension ordering. wgpu alone is the most portable and production-proven foundation (Metal/Vulkan/DX12/GL/WebGPU) but forces hand-written WGSL and f32-only; cudarc is the best choice for an NVIDIA-only fast path; candle is tensor-shaped and awkward for path-dependent MC/PDE; rust-gpu and Rust-CUDA/cuda-oxide remain promising but rough (nightly, non-integrated codegen). Recommended architecture: a Rust trait-based pricing-engine abstraction with a CubeCL primary path (or wgpu+WGSL if you want fewer dependencies), an f32 GPU numeric policy with f64 CPU fallback for validation, Philox RNG, and Lavapipe/CPU fallback in containers.

**Key Findings**

- CubeCL v0.10.0 (released May 11, 2026, tracel-ai) compiles a single #[cube] Rust function to CUDA, ROCm/HIP, Metal, Vulkan/SPIR-V, WebGPU/WGSL, AND a CPU-SIMD runtime, selecting best per-platform instructions; it is the compute backend behind the Burn ML framework.
- CubeCL features autotune (runtime micro-benchmark kernel selection), comptime (compile-time IR specialization), and tensor-core auto-dispatch; kernels are real type-checked/borrow-checked Rust, unit-testable on CPU. Feature flags: features=["cuda"|"wgpu"|"hip"|"cpu"].
- CubeCL ships a companion kernel library 'cubek' with matmul, reductions, convolutions, attention, quantization, and random number generation kernels.
- Metal (Apple GPUs) and Metal Performance Shaders have NO native FP64/f64; emulation exists (philipturner/metal-float64 via double-single float32x2), but Metal's compiler optimizes away naive double-single and emulation is slow versus hardware.
- WebGPU/WGSL has no f64 at all: base scalar types are f32, f16, i32, u32 only; atomics are restricted to atomic<i32>/atomic<u32> (no float atomics); workgroup shared memory guaranteed >=16384 bytes via maxComputeWorkgroupStorageSize.
- wgpu (gfx-rs) is a safe pure-Rust WebGPU implementation running natively on Vulkan, Metal, DirectX 12, OpenGL ES, plus browser WebGPU; consumes WGSL, SPIR-V, and GLSL shaders on any backend.
- wgpu f64 in shaders is a NATIVE-ONLY feature (Vulkan/Metal*/DX12 via SPIR-V SHADER_FLOAT64 capability, never in browser WebGPU), and is typically 16-64x slower than f32 even where hardware supports it; Metal still lacks true f64 so wgpu+Metal is effectively f32-only.
- cudarc (coreylowman/chelsea0x3b) is a maintained safe Rust wrapper over the CUDA Driver/Runtime API (low-level unsafe + thin wrapper); it is host-side orchestration and you supply kernels (PTX/cuBLAS/cuRAND), giving full f64 on NVIDIA. NVIDIA-only.
- Rust-CUDA project (Rust-GPU/Rust-CUDA) was rebooted Jan 2025, is community-driven, compiles Rust to NVVM IR; tied to NVIDIA toolchain LLVM 7.1 (distribution friction). rust-gpu compiles Rust to SPIR-V for any Vulkan target.
- NVIDIA released cuda-oxide (NVlabs, May 2026): an experimental rustc codegen backend compiling #[kernel]-annotated idiomatic Rust directly to PTX (no DSL/FFI/C++); experimental, NVIDIA-only.
- As of July 2025 the 'Rust on every GPU' demo runs one codebase across Rust-CUDA(PTX), rust-gpu(SPIR-V -> Vulkan/Metal-via-MSL/DX12-via-HLSL), WebGPU, and CPU, but DX is 'rough and bolted together': non-integrated codegen, specific nightly Rust, divergent thread-index/std APIs between Rust-CUDA and rust-gpu.
- candle (HuggingFace) has CPU(MKL)/CUDA/cuDNN/Metal backends and supports custom kernels (CUDA PTX, Metal MSL), but it is tensor/ML-shaped; Windows CUDA lags Linux; it is awkward for path-dependent Monte Carlo and stencil PDEs.
- std::simd (portable-simd) remains nightly-only for the foreseeable future; for stable cross-platform CPU SIMD use the `wide` crate, plus `rayon` (.par_iter) for multicore; the `faster` crate is abandoned.
- Counter-based / Philox RNG is the recommended GPU RNG: stateless, keyed transforms of counters, vectorizes/parallelizes with minimal memory and is faster than cuRAND on a single NVIDIA GPU; reproducible per path from (path_index, step). GPUs have no built-in RNG (naive WGSL examples resort to LCG).
- Sobol QMC gives ~O(1/N) vs MC O(1/sqrt(N)); ~2x paths to halve error and up to ~10x fewer paths for smooth payoffs; on GPU it needs precomputed direction numbers and careful dimension assignment (Brownian bridge) per path.
- Containers: NVIDIA Container Toolkit exposes CUDA + Vulkan + OpenCL to Linux containers (host-side toolkit, NVIDIA_DRIVER_CAPABILITIES=all, NVIDIA_VISIBLE_DEVICES=all); a Rust+wgpu binary on debian:sid-slim with libglvnd can use Vulkan on a Tesla T4. Driver 550->570 broke container Vulkan in a known issue. Mesa Lavapipe (LLVMpipe) gives a software Vulkan device for GPU-less CI/headless.
- A published Rust wgpu Monte Carlo option-pricing example reports ~2000x speedup over Python and compares Python-NumPy(CPU), Python-OpenCL(GPU), Rust-rayon(CPU), Rust-wgpu(GPU).

**Recommendations**

- Adopt CubeCL (v0.10.x) as the primary GPU layer: one #[cube] Rust kernel for path generation, payoff reduction, and finite-difference stencils compiles to CUDA on NVIDIA, Metal/Vulkan/WGSL via wgpu elsewhere, and a CPU-SIMD runtime as the automatic no-GPU fallback. This single-codebase, multi-backend property directly satisfies the macOS+Windows+Linux+container requirement.
- Standardize GPU numerics on f32 and treat f64 as a CPU-only / NVIDIA-only validation path. Because Metal and WGSL have no native f64, an f64-everywhere GPU engine is not portable; price in f32 on GPU and run a periodic f64 CPU reconciliation (rayon + `wide`) to bound numerical error for FX exotics.
- Define a clean Rust trait abstraction, e.g. trait PricingBackend { fn simulate_paths(...); fn reduce_payoff(...); fn solve_pde(...); }, with implementations CubeClBackend (default) and CpuBackend, selected at runtime by probing for an adapter; degrade to CPU when no GPU adapter is found. Keep payoff/model logic generic over the numeric scalar type.
- Use a Philox-4x32-10 counter-based RNG implemented as a #[cube] kernel (or WGSL), seeding each variate from (global_seed, path_index, time_step, dimension). This is reproducible, bit-stable across backends, embarrassingly parallel, and avoids per-thread RNG state. Box-Muller/inverse-CDF for normals.
- For QMC, precompute Sobol direction numbers on the host, upload as a buffer, generate Sobol points on GPU, and assign dimensions to time steps via a Brownian bridge for path-dependent FX exotics (barriers, autocallables). Keep MC (Philox) and QMC (Sobol) as swappable variate sources behind the same kernel interface.
- Implement explicit/ADI finite-difference PDE solvers as tiled stencil compute kernels using workgroup shared memory; for implicit/Crank-Nicolson use cyclic-reduction or PCR tridiagonal solvers that map well to GPU. Validate against the MC engine on vanilla options.
- If you want minimal dependencies / maximum control and accept hand-written shaders, use wgpu + WGSL directly (f32-only) as an alternative to CubeCL; reserve cudarc for an optional NVIDIA-only high-precision/high-throughput fast path that uses f64 and cuRAND/cuBLAS.
- For deployment: ship Linux containers with the NVIDIA Container Toolkit for CUDA/Vulkan GPU access, and bundle Mesa Lavapipe so GPU-less CI and headless nodes still exercise the same wgpu/Vulkan path via a software device. Pin NVIDIA driver/toolkit versions to avoid the 550->570-class container-Vulkan regressions.
- Avoid betting the core engine on rust-gpu, Rust-CUDA, or cuda-oxide today; they are promising but nightly/experimental with non-integrated codegen. Revisit them as a future single-language path once they stabilize and converge their APIs.

**Risks**

- Metal and WebGPU/WGSL have zero native f64. Any design assuming double precision on Apple Silicon or in-browser will fail or fall back to slow emulation (metal-float64) that Metal's optimizer can even break; do not promise f64 on GPU cross-platform.
- f64 on backends that DO support it (Vulkan/DX12/CUDA) is 16-64x slower than f32; uniform f64 GPU pricing will be far slower than expected and still non-portable.
- WGSL atomics are integer-only (no float atomics), so payoff accumulation/reductions must use integer atomics, fixed-point tricks, or tree-reduction rather than atomic float add.
- RNG reproducibility and correctness: naive LCG RNGs (common in tutorials) have poor statistical quality for finance; per-thread stateful RNGs are hard to parallelize. Use counter-based Philox and validate distribution quality and cross-backend bit-stability.
- Sobol QMC pitfalls: wrong dimension-to-timestep assignment, missing Brownian bridge, or scrambling errors silently degrade convergence for high-dimensional path-dependent FX exotics; verify against MC.
- CubeCL/Burn ecosystem velocity is high (v0.10.0 May 2026) but still pre-1.0; APIs can churn, ROCm/HIP is work-in-progress, and Metal/WGSL feature parity (subgroups, f16, atomics) varies by backend and driver.
- Container GPU access is fragile: NVIDIA driver/toolkit version mismatches (e.g. 550->570) have broken in-container Vulkan; ECS/long-running containers have lost GPU access over time. Pin versions and add health checks.
- rust-gpu / Rust-CUDA / cuda-oxide require specific nightly toolchains and non-integrated codegen backends (rustc_codegen_spirv / nvvm; Rust-CUDA tied to LLVM 7.1), creating reproducibility and CI/distribution headaches if adopted prematurely.
- candle is optimized for dense tensor/ML workloads; expressing path-dependent Monte Carlo state machines and barrier/autocall logic as tensor ops is awkward and likely memory-bound; not a natural fit for this engine.
- std::simd is nightly-only indefinitely; relying on it for the stable CPU fallback is risky. Use the `wide` crate + `rayon` instead for a stable, portable CPU path.

**Sources**

- https://github.com/tracel-ai/cubecl
- https://crates.io/crates/cubecl-wgpu
- https://news.ycombinator.com/item?id=43777731
- https://bestai.com/news/Rust-GPU-kernels-CUDA-ROCm-WGPU-cda90879af
- https://github.com/gfx-rs/wgpu
- https://wgpu.rs/
- https://docs.rs/wgpu-types/latest/wgpu_types/struct.Features.html
- https://rustify.rs/articles/rust-gpu-computing-wgpu-2026
- https://github.com/philipturner/metal-float64
- https://developer.apple.com/forums/thread/797778
- https://github.com/gpuweb/gpuweb/issues/2805
- https://hugodaniel.com/posts/webgpu-shader-limits/
- https://google.github.io/tour-of-wgsl/types/atomics/atomic-types/
- https://docs.rs/cudarc
- https://github.com/coreylowman/cudarc
- https://rust-gpu.github.io/blog/2025/07/25/rust-on-every-gpu/
- https://rust-gpu.github.io/blog/2025/08/11/rust-cuda-update/
- https://github.com/Rust-GPU/Rust-CUDA
- https://github.com/NVlabs/cuda-oxide
- https://www.marktechpost.com/2026/05/09/nvidia-ai-just-released-cuda-oxide-an-experimental-rust-to-cuda-compiler-backend-that-compiles-simt-gpu-kernels-directly-to-ptx/
- https://github.com/huggingface/candle
- https://shnatsel.medium.com/the-state-of-simd-in-rust-in-2025-32c263e5f53d
- https://doc.rust-lang.org/std/simd/index.html
- https://medium.com/@joseph.frost_91327/gpu-monte-carlo-simulations-in-python-and-rust-c9b345525bcf
- https://arxiv.org/pdf/2502.17731
- https://www.docker.com/blog/docker-model-runner-vulkan-gpu-support/
- https://unrealcontainers.com/docs/concepts/nvidia-docker
- https://github.com/NVIDIA/nvidia-container-toolkit/issues/1041

## bisect risk

test

**Key Findings**

- a

**Recommendations**

- p

**Risks**

- codebase-memory-mcp finds zero cross-service edges (in-proc distributor and Protobuf and FIX, not HTTP or gRPC), so automated cross-service impact analysis misses Celnet deps; maintain the map manually and verify against Spring config and distributor notifyUsers names.
- Hops orderrouting to risk and risk to destination to clearing to positionmanager are marked inferred from api deps and handler signatures, not runtime-traced; validate before relying on them.
- The distributor is a bounded disruptor mailbox with skip-while-full back-pressure so a high-frequency option pricer can cause silent price drops; size mailboxes and rate-limit.
- The distributor runs in-process in the JVM baseserver so a Rust process cannot natively join it and must use a JVM adapter or the distributor socket protocol DistributorProducerChannelHandler; decide before committing.
- MarketMerchantPriceService is WS-only with no fallback and a roughly 6 concurrent HTTP connection semaphore per domain; new option price streams must be resilient to WS disconnects.
- No option product type exists so adding one touches celertech-type, staticdata, every api proto enum, positionmanager netting keys, risk exposure models, and destination FIX dialect mappings, a broad change.
- Config is layered at deploy via celer-client tenant overlays not baked into artifacts and the celer-client subgroup is not in the default repo pull, so config wiring may be invisible locally; confirm tenant overlay ownership with devops.
- FX options need a vol surface and Greeks input that spot-only marketdata may not supply; verify marketdata-api can deliver vol data or plan an additional feed handler.

**Sources**

- p

## World-class deterministic testing & CI stack for a mission-critical Rust quant pricing library (best practices, May 2026)

For a mission-critical Rust quant pricing library, layer the test pyramid as: numerical golden/reference tests cross-validated against QuantLib and published benchmark prices, financial invariant tests (put-call parity, monotonicity, smile no-arbitrage: butterfly/calendar/vertical), property-based tests (proptest preferred over quickcheck for Strategy-based generation and superior shrinking), structure-aware fuzzing via cargo-fuzz/libFuzzer, snapshot tests (insta) for large/structured outputs, and criterion or divan benchmarks gated against baselines. Determinism is the central discipline: pin a single toolchain + target, avoid FMA contraction and fast-math, prefer the correctly-rounded rust-lang/libm for cross-platform reproducibility, compare floats with explicit ULP/relative tolerances rather than equality, and never depend on NaN bit patterns. Quality gates layer in cargo-llvm-cov coverage with fail-under thresholds, cargo-mutants mutation testing (incremental on PRs, full on main), and a supply-chain trio of cargo-deny (subsumes cargo-audit) + cargo-vet. CI runs a Linux/macOS/Windows matrix on cargo-nextest, with a strict MSRV job, and the 2024 edition with the MSRV-aware resolver (stable since 1.84/1.85). Reproducibility hinges on checking proptest-regressions and fuzz corpora into source control so failing seeds replay deterministically everywhere.

**Key Findings**

- proptest is the recommended property-testing framework over quickcheck: it uses explicit Strategy objects (per-value, not per-type generation), supports multiple strategies per type, respects constraints during generation/shrinking, and has materially better shrinking. quickcheck is type-driven and requires newtype wrappers for custom generators.
- proptest persists failing cases under a proptest-regressions/ directory keyed to the source file, storing the SEED (not the input) so replays are deterministic and non-spurious. These files MUST be committed to source control so CI and collaborators replay the same known-bad cases.
- cargo-fuzz is the de-facto Rust fuzzer, a thin layer over libFuzzer. It requires nightly + a C++ toolchain and only works on x86-64/Aarch64 Unix-like OSes (NOT Windows). ASan is on by default (valuable for unsafe code); disable sanitizers for a throughput boost if no unsafe. 2025 added --strip-dead-code, --disable-branch-folding, and --codegen-units (default 1 for throughput).
- Fuzz inputs implement the Arbitrary trait for structure-aware fuzzing (turn raw bytes into valid pricing inputs like rate/vol/strike/maturity). Crash cases reproduce via `cargo +nightly fuzz run`; commit the corpus and crash artifacts for regression.
- cargo-mutants is the leading Rust mutation tester (zero-config, no source-tree changes, works on any recent stable/nightly). It catches test-suite gaps that coverage cannot. Recommended CI pattern: incremental mutation testing on PR diffs, full runs async on the main/dev branch. Main cost is per-mutant incremental builds.
- cargo-llvm-cov is the recommended coverage tool (LLVM source-based instrumentation, precise, supports nextest/doctests/proc-macros). Best practice: run via `cargo llvm-cov nextest`, emit HTML for humans + LCOV/JSON for CI upload (Codecov/Coveralls/GitLab Cobertura), enforce fail-under thresholds, and `cargo llvm-cov clean` between runs. As of late 2025, x86_64-pc-windows-gnu and aarch64-pc-windows-msvc are known broken under default setup.
- insta is the standard snapshot library. Use inline snapshots (@"...") and `cargo insta review` to accept changes; never hand-edit snapshots. In CI, set the CI=1 env var so out-of-date snapshots FAIL rather than silently writing new files. Redactions handle volatile fields (timestamps, ids). Useful for serialized vol surfaces, risk reports, diagnostics.
- The built-in #[bench] attribute is a hard error on stable as of Rust 1.88, so criterion or divan are the only no-nightly benchmark options. criterion is the mature statistical standard (port of Haskell Criterion); divan is newer, simpler, stable-only, with sample-size scaling that reduces CI timing noise.
- Benchmark regression gating: compare current criterion JSON output against committed baselines with per-key tolerances; default lenient (report but pass), optional strict mode to fail the job. Bencher and CodSpeed provide managed continuous-benchmarking against baselines.
- Supply chain: cargo-deny SUBSUMES cargo-audit (it includes RustSec advisory checking plus license + source/ban policy enforcement). cargo-vet (Mozilla) is complementary and different in kind: it enforces that every third-party dependency has been explicitly audited/certified by a trusted entity, rather than checking a vuln database. Run cargo-deny and cargo-audit on every PR; cargo-vet the same way but needs an initial audit-baseline setup.
- Float determinism: Rust follows IEEE-754-2008. FP is deterministic per-(compiler, instruction set) but NOT reproducible across machines/compilers/OSes. FP addition is non-associative so operation/parallel-reduction order changes results. NaN bit-pattern generation is explicitly non-deterministic and must never be relied upon. RFC 3514 (float semantics) is the relevant standardization track.
- rust-lang/libm now provides correctly-rounded math functions (verified against MPFR to <=1.0 ULP, tested with property-based random inputs across the FP range); correct rounding gives exactly one answer, making results reproducible across platform/OS/libm updates. libm merged into the compiler-builtins repo. Prefer it over relying on the system libm for cross-platform reproducibility.
- Numerical validation: cross-validate against QuantLib (e.g. QuantLib-Python/PyQL 1.40) as the reference oracle and against prices published in literature. QuantLib's own test-suite explicitly checks put-call parity (including for deltas, digitals, caps/floors/swaps) and validates by reproducing web/literature results and comparing against Black pricing.
- Smile/surface no-arbitrage invariants to assert as tests: absence of calendar-spread arbitrage (total-variance lines must not cross; monotonicity of price in maturity), absence of butterfly arbitrage (implied risk-neutral density must be non-negative; price convex in strike), and absence of vertical/call-spread arbitrage. SVI/SSVI parameterizations have known sufficient conditions guaranteeing static-arbitrage-free surfaces — good golden-model targets.
- MSRV policy: declare MSRV in Cargo.toml and test it in a dedicated CI job. API-guidelines treat an MSRV bump as non-breaking but bump minor (>=1.0) or patch (<1.0). Rust 1.84 stabilized the MSRV-aware resolver; the 2024 edition (stable 1.85) enables it by default, letting libraries support older toolchains without manually pinning old dep versions. Common policies: hyper supports >=6-month-old compilers; kube trails 2 stable releases.
- CI matrix best practice: ubuntu-latest + macos-latest + windows-latest, run with cargo-nextest as a faster/parallel test runner. Use a setup action (moonrepo/setup-rust or dtolnay/rust-toolchain) that can install cargo-nextest, set GITHUB_TOKEN to avoid rate limits, cache ~/.cargo + target, and use cache-extra-identifier for unique matrix cache keys. houseabsolute/actions-rust-cross handles cross-compiled targets (aarch64 linux/darwin, windows-msvc).

**Recommendations**

- Adopt a layered test strategy: (1) deterministic golden/reference tests vs QuantLib + published benchmark prices with explicit ULP/relative tolerances; (2) invariant property tests; (3) proptest property tests; (4) cargo-fuzz fuzzing; (5) insta snapshots for structured outputs; (6) criterion/divan perf gates.
- Use proptest (not quickcheck) for property-based testing. Define explicit Strategy generators for valid market inputs (spot>0, vol in a sane band, strikes>0, T>=0, rates). Commit the proptest-regressions/ directory so failing seeds replay deterministically in CI and on every developer machine.
- Encode financial invariants as property tests over wide random parameter ranges: European put-call parity (C - P == S*exp(-qT) - K*exp(-rT) within tolerance), price monotonicity in spot/vol/maturity, convexity in strike (butterfly>=0), non-negative implied risk-neutral density, no calendar-spread arbitrage (non-crossing total-variance), Greeks consistency vs finite-difference bumps, and intrinsic-value lower bounds.
- Establish QuantLib as the external reference oracle. Generate a frozen CSV/JSON table of QuantLib prices+Greeks across a parameter grid (in a pinned, version-locked harness), commit it, and assert the Rust engine matches within documented ULP/relative/absolute tolerances. Re-generate only on deliberate review, tracking QuantLib version.
- Lock determinism: pin the toolchain via rust-toolchain.toml; forbid fast-math / FMA contraction in hot paths where reproducibility matters (or test both contracted and non-contracted paths); prefer rust-lang/libm for transcendental functions to get correct rounding and cross-platform identical results; never assert on NaN bit patterns; centralize float comparison in an approx/ulp helper (e.g. the `approx` crate or a custom assert_close with relative+absolute+ULP tolerances).
- Run cargo-fuzz targets (nightly, Linux x86-64/Aarch64) for serialization/parsing, calibration solvers, and any unsafe SIMD math. Implement Arbitrary to map fuzzer bytes to valid-but-adversarial market inputs. Commit corpora and crash artifacts; add a CI job that replays the corpus on every PR and a scheduled long-running fuzzing job (e.g. OSS-Fuzz-style nightly).
- Add insta snapshot tests for serialized vol surfaces, risk reports, and CLI/diagnostic output, with redactions for volatile fields. Set CI=1 in CI so stale snapshots fail; never auto-accept in CI.
- Set up criterion (mature) or divan (simpler, stable, low CI noise) benchmarks for pricing/calibration hot paths. Commit baseline JSON and add a comparator gate: lenient by default, strict (fail-the-job) on protected branches with per-benchmark tolerances. Consider Bencher/CodSpeed for managed tracking.
- Run cargo-llvm-cov nextest with a fail-under threshold (e.g. >=90% on core pricing modules), emit LCOV+HTML, upload to Codecov, and `cargo llvm-cov clean` between runs. Note Windows-gnu/aarch64-msvc coverage limitations — restrict the coverage job to Linux.
- Add cargo-mutants: incremental (PR-diff-scoped) on every PR with a kill-rate gate on core modules, plus a scheduled full run on the main branch. Treat surviving mutants in pricing/invariant code as test-gap tickets.
- Supply chain: run cargo-deny (advisories + licenses + bans + sources) and cargo-audit on every PR; adopt cargo-vet with an audit baseline so all third-party deps are explicitly certified. Commit deny.toml and supply-chain/ vet config. Pin/commit Cargo.lock for the binary/test artifacts.
- CI matrix: ubuntu/macos/windows-latest x {stable, MSRV} running cargo-nextest, plus separate jobs for fmt+clippy(-D warnings), coverage (Linux), fuzz-corpus-replay (Linux nightly), mutation (scheduled), benchmark-gate, and supply-chain. Use dtolnay/rust-toolchain or moonrepo/setup-rust with caching and a dedicated MSRV verification job.
- Declare MSRV in Cargo.toml `rust-version`, adopt edition 2024 with the MSRV-aware resolver, verify MSRV in a pinned CI job, and treat MSRV bumps as minor-version changes per API guidelines. Pick an explicit support window (e.g. N-2 stable, or >=6 months old).

**Risks**

- Float reproducibility is fragile across OS/arch/compiler: SIMD vectorization, FMA contraction, parallel reduction order, and differing system libm implementations all silently change low bits. Exact-equality assertions WILL produce flaky cross-platform CI failures — always use tolerances and pin the math path (prefer rust-lang/libm).
- NaN bit-pattern generation is non-deterministic per IEEE/Rust semantics; tests that assert on NaN payloads or that compare structures containing NaN with == will be non-reproducible. Guard against NaN/Inf propagation explicitly in pricing inputs and outputs.
- cargo-fuzz does NOT run on Windows and needs nightly + a C++ toolchain — do not put fuzzing in the cross-platform matrix; isolate it to a Linux nightly job. Disabling sanitizers boosts throughput but loses memory-safety detection for unsafe/SIMD code.
- cargo-mutants and full coverage runs are slow (each mutant = an incremental build + test run); running them blocking on every PR will throttle the pipeline. Scope mutation to PR diffs and run full passes on a schedule; cache aggressively.
- Snapshot tests can rot into rubber-stamped diffs if reviewers blindly `cargo insta accept`. Require human review of every snapshot change and forbid acceptance in CI (CI=1). Same risk with golden tables — regenerate only under explicit review with version tracking.
- Benchmark regression gates are noisy on shared CI runners; without sample-size scaling (divan) or warmups and per-key tolerances, you get false regressions. Pin runner type and keep gates lenient off protected branches.
- cargo-vet requires significant up-front audit-baseline effort and ongoing maintenance; without trusted-import aggregation it can become a bottleneck. cargo-deny already subsumes cargo-audit, so running both audit and deny is redundant — prefer cargo-deny for advisories.
- An overly aggressive MSRV bump silently becomes a de-facto breaking change for downstream consumers if not gated in CI; an MSRV-pinned job that doesn't actually use the MSRV-aware resolver can pass locally yet break for users on older toolchains.
- Windows coverage targets (x86_64-pc-windows-gnu, aarch64-pc-windows-msvc) are known-broken under default cargo-llvm-cov setup as of late 2025 — don't gate Windows on coverage.
- Treating QuantLib as ground truth without version-pinning is risky: QuantLib results can change across versions and engines; freeze the reference version and the generated golden table together.

**Sources**

- https://github.com/sourcefrog/cargo-mutants
- https://mutants.rs/
- https://proptest-rs.github.io/proptest/proptest/vs-quickcheck.html
- https://proptest-rs.github.io/proptest/proptest/failure-persistence.html
- https://github.com/BurntSushi/quickcheck
- https://github.com/rust-fuzz/cargo-fuzz
- https://rust-fuzz.github.io/book/cargo-fuzz.html
- https://rust-fuzz.github.io/book/cargo-fuzz/structure-aware-fuzzing.html
- https://appsec.guide/docs/fuzzing/rust/cargo-fuzz/
- https://github.com/mitsuhiko/insta
- https://insta.rs/docs/advanced/
- https://nikolaivazquez.com/blog/divan/
- https://bencher.dev/learn/track-in-ci/rust/criterion/
- https://codspeed.io/docs/guides/how-to-benchmark-rust-with-divan
- https://github.com/taiki-e/cargo-llvm-cov
- https://rustprojectprimer.com/checks/audit.html
- https://github.com/mozilla/cargo-vet
- https://mozilla.github.io/cargo-vet/
- https://blog.logrocket.com/comparing-rust-supply-chain-safety-tools/
- https://rust-lang.github.io/rfcs/3514-float-semantics.html
- https://users.rust-lang.org/t/determinism-for-floating-point-operations-in-rust/4426
- https://deepwiki.com/rust-lang/libm
- https://github.com/lballabio/QuantLib/blob/master/test-suite/americanoption.cpp
- https://ar5iv.labs.arxiv.org/html/1204.0646
- https://www.imperial.ac.uk/media/imperial-college/research-centres-and-groups/cfm-imperial-institute-of-quantitative-finance/events/distinguished-lectures/Gatheral-2nd-Lecture.pdf
- https://github.com/rust-lang/api-guidelines/discussions/231
- https://rust-lang.github.io/rfcs/3537-msrv-resolver.html
- https://blog.rust-lang.org/2025/01/09/Rust-1.84.0/
- https://ahmedjama.com/blog/2025/12/cross-platform-rust-pipeline-github-actions/
- https://docs.github.com/actions/tutorials/build-and-test-code/building-and-testing-rust
