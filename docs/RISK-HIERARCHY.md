# Celnet Hierarchical Risk Aggregation — SOTA Design & Competitive Critique

> State-of-the-art (May-2026) design for firm-wide, real-time, hierarchical FX-options risk
> aggregation, limits, and the trader/risk-manager experience — plus an honest, cited
> competitive critique of the incumbent platforms.
>
> **Provenance discipline.** This document separates **verified facts (cited)** from the
> **Celnet design proposal**. Every competitor capability claim carries a `[source / confidence]`
> tag: **high** = directly documented on a vendor/primary page; **medium** = strongly implied or
> secondary source; **low / inference** = reasoned argument-from-absence or workload-class
> inference. Negative claims ("incumbent X does *not* do Y") are explicitly flagged as
> arguments-from-absence — absence of public evidence is **not** proof of absence. Proposed Celnet
> identifiers are vendor-neutral and purpose-named per CLAUDE.md rule 8; competitor names appear
> only in the critique prose (research context), never as shipped API identifiers.

---

## 1. Purpose & scope

Celnet today prices and risks at the **single-instrument** level: `celnet-vanilla` produces a
full 14-Greek set; `celnet-engine` carries a pinned, zero-alloc hot core whose `BucketedRisk`
already computes vega bucketed by `(tenor × delta)`, a cross-gamma 2-D stencil, and a theta-roll
axis. What does **not** yet exist is the layer above a single position: a **firm-wide,
convention-normalized, multi-dimensional risk cube** that rolls atomic position risk up through
the organizational hierarchy (trader → book → desk → currency-pair → booking-location → legal
entity → firm), nets it correctly across pricing conventions and currency numeraires, enforces
**limits** at every node, supports **entitlement-scoped drill-down**, and streams all of it to a
trader/risk-manager UI at sub-second cadence.

This document specifies that layer. Scope:

- **§2** — the formal **dimension model**: the organizational hierarchy as an OLAP-style cube over
  an immutable position-level Greek fact table; the risk dimensions (Greeks, bucketed vega,
  scenario/stress, VaR/ES, FRTB-SA sensitivities, P&L-explain); and how the hard FX-specific
  aggregation problems are solved (convention normalization, common-numeraire delta/vega, cross-pair
  delta triangulation, hierarchical roll-up + drill-down, entitlements, real-time vs EOD,
  follow-the-sun).
- **§3** — how Celnet computes this **at scale**, tying the new aggregation/limits services to the
  existing zero-alloc engine and the (designed-only) scale-out tier.
- **§4** — the **entitlements** model.
- **§5** — the **limits** framework.
- **§6** — the trader/risk-manager **experience** (hierarchical explorer, heatmaps, limit dashboard,
  scenario, P&L-explain).
- **§7** — the **competitive critique** with citations and confidence flags.

Out of scope (explicitly scoped out so the omission is honest, not silent): **XVA / SA-CVA**
sensitivities (their own delta/vega buckets distinct from market-risk SbM), **FX settlement /
Herstatt / CLS PvP** risk as a capital line, and **Default Risk Charge (DRC)** beyond noting it is
immaterial for vanilla FX (FX has no issuer-default leg unless a credit-linked or EM-sovereign
wrapper is present — see §2.4). These are flagged as future workstreams in `docs/ROADMAP.md`, not
quietly dropped.

> **Implementation status (2026-05-31).** The single-node layer this document specifies is **shipped
> and served end-to-end**, no longer proposal-only. The four crates of §3.2 are **built and gated
> green** — `celnet-risk-cube` (dimension model + group-by/reduce roll-up + per-node non-additive
> re-derivation), `celnet-risk-normalize` (convention canonicalization + common-numeraire conversion),
> `celnet-limits` (limit tree + utilization/RAG + pre/post-trade checks), `celnet-entitlements`
> (server-side pre-aggregation pruning, grant-all default). They are exposed over the **one
> `celnet-proto` contract** as `RiskService` (`ListPositions` / `AggregateRisk` / `DrillRisk` /
> `LimitStatus`) and **served by `celnet-server`** over both gRPC and the WS mirror, off a shared live
> `PositionStore` the RFS click-to-trade path books vanilla fills into — so aggregation is now done
> **server-side**, retiring the client-side roll-up. See `docs/INTERFACES.md` §"Phase-2 contract:
> `RiskService`". **Now built (§3.3):** the **AAD adjoint-Greek** scale path is wired —
> `celnet-risk-normalize::canonicalize` derives the additive leaf from a single reverse-mode sweep
> (`celnet_vanilla::adjoint_greeks`) by default, and `celnet_risk_cube::sensitivity_var_es` re-derives
> the non-additive VaR/ES from one adjoint sweep per position + a second-order Taylor expansion; the
> **batched-GPU scenario grid** (`celnet_risk_cube::gpu_pv_grid` over `celnet-gpu`'s `ScenarioPricer`)
> prices a whole spot×vol ladder in one dispatch. Both are validated against the bump-and-revalue /
> closed-form oracle, which is **retained** as the reference (not removed). Measured (M4 Metal,
> `celnet-bench`): AAD first-order block ~42 ns vs ~129 ns bump-and-revalue (~3.0×, O(1) vs O(factors));
> batched-GPU scenario grid ~13 ms vs ~165 ms unbatched per-node MC (~12.7×). **Still deferred (target
> architecture, not shipped):** the cross-shard / HRW-router fleet tier (§3.4 — single-node only). The client lanes (GUI Book, `celnet-client` SDK,
> Excel `CELNET.*`) **all now consume the served contract in lockstep** — the GUI Book's client-side
> roll-up is **deleted** (`gui/src/data/portfolioRisk.ts` removed; the Book drives `AggregateRisk`/
> `LimitStatus`/`DrillRisk` server-side), the SDK exposes the four calls as typed `Client` methods, and
> Excel ships `CELNET.RISK`/`CELNET.POSITIONS`/`CELNET.LIMITS`. The client-side-aggregation violation is
> retired.

---

## 2. The risk-aggregation model

### 2.1 The organizational hierarchy is a CUBE, not a single tree

The canonical roll-up at a bank/market-maker, finest → coarsest, is:

1. **Position / trade** — a single option carrying its own pricing-convention metadata (premium
   currency, delta convention, cut, ATM convention, day count).
2. **Trader / book** — a trader runs one or more *books*.
3. **Desk** — books grouped into desks, typically by product/region specialization (**G10 vol**,
   **EM vol**, **exotics/structured**). Under FRTB the trading desk is a **regulatory** unit — "an
   unambiguously defined group of traders or trading accounts," with one (max two) head trader, each
   trading account assigned to a single desk, and a defined risk scope / permitted risk factors.
   *(Accenture / GreenPoint / OSFI CAR Ch.9, matching BCBS MAR12 — high.)*
4. **Currency-pair / underlying** — *orthogonal* to the desk axis: a delta ladder per pair and a
   vega surface/grid per `(pair, tenor, strike-or-delta)`.
5. **Country / booking-location** — follow-the-sun; matters for regulatory reporting and
   legal-entity ownership.
6. **Legal entity** — the regulatory-capital / large-exposure unit; FRTB capital binds here.
7. **Firm / global** — the consolidated CRO/board view. Supervisors expect aggregation on a fully
   consolidated basis across business lines, legal entities, and activities — concentrations not
   visible from a single branch become visible at the firm. *(Federal Reserve trading-and-capital-
   markets supervision manual — principle high; treat any specific wording as paraphrase, not a
   verified literal quote — medium.)*

The crucial structural point: **desk, currency-pair, booking-location, and legal-entity are
independent dimensions, not a single nesting.** A position is simultaneously "owned by trader T in
book B on the EM-vol desk," "an EURTRY exposure," "booked in the London entity," and "in the UK
booking-location." Forcing these into one tree loses roll-up paths. The correct model is an
**OLAP cube** (star/snowflake of dimensions) over an **immutable position-level Greek fact table**,
with multiple roll-up paths along independent dimensions. *(Synthesis — Celnet design proposal; the
cube/star-schema framing is standard data-modeling practice, not a sourced vendor claim.)*

**Celnet dimension model (proposed).** Define the fact and dimensions as `celnet-types` value types:

```
RiskFact  (one immutable row per position, per valuation)        // the leaf "measure carrier"
  position_id           : PositionId
  // dimension keys (foreign keys into the dimension tables)
  trader                : TraderId
  book                  : BookId
  desk                  : DeskId
  ccy_pair              : CcyPair
  booking_location      : LocationId
  legal_entity          : EntityId
  value_date            : ValueDate          // 7th axis — settlement/CLS horizon (§2.8)
  session               : TradingSession     // follow-the-sun cut context (§2.9)
  // measures, ALL pre-normalized to a single canonical convention + reporting numeraire (§2.2–2.3)
  greeks_canonical      : CanonicalGreeks     // delta-by-ccy-leg, vega ladder, vanna, volga, ...
  surface_version       : SurfaceVersion      // which marked surface produced this fact (§2.7)
```

Dimensions are a `DimensionId` enum (`Trader`, `Book`, `Desk`, `CcyPair`, `Location`, `Entity`,
`ValueDate`, `Session`); each carries a **parent-pointer hierarchy** (e.g. `Book → Desk`,
`Location → Entity → Firm`) so a roll-up is "group by the dimension's ancestor at level L." The
aggregation engine is then a **group-by + reduce** over the fact table along any subset of
dimensions, at any level — the canonical OLAP operation, made FX-correct by §2.2–2.4.

### 2.2 Convention normalization — you cannot add raw Greeks

This is a **convention** problem, not a model problem. Two books may both report "delta 0.25" under
*different* delta conventions; adding them is meaningless. The convention axes
(`docs/CONVENTIONS.md`, all four enums verified verbatim):

- **Delta convention** — spot vs forward × unadjusted vs premium-adjusted
  (`DeltaConvention {SpotUnadjusted, ForwardUnadjusted, SpotPremiumAdjusted, ForwardPremiumAdjusted}`).
  Standard formulas: spot-unadjusted Δ = e^(−r_f τ)·φ·N(φd₁); spot-premium-adjusted
  Δ = (K/S)·e^(−r_d τ)·φ·N(φd₂); short tenors → spot, long ≳1–2Y → forward. *(quantpie / Wystup /
  Celnet CONVENTIONS.md — high.)*
- **ATM convention** — ATM-forward vs delta-neutral-straddle (`AtmConvention {AtmForward,
  DeltaNeutralStraddle}`). *(CONVENTIONS.md — high.)*
- **Premium style / currency** — domestic pips vs %-foreign vs %-domestic vs foreign pips
  (`PremiumStyle {DomesticPips, PercentForeign, PercentDomestic, ForeignPips}`). *(CONVENTIONS.md —
  high.)*

The substance — *positions under different delta definitions cannot be aggregated without
conversion; consistent premium-convention application is essential for valid netting* — is correct
and well-documented (Wystup documents the trader-delta vs risk-manager-delta distinction and
premium-adjustment mechanics). We present it as established practice, **not** as a verbatim Wystup
quote (the specific sentence is not verifiably in his "FX Greeks" column — medium on attribution,
high on substance).

**Celnet rule.** The aggregation engine re-derives every position's risk into **one canonical
internal convention** *before* aggregation. Proposed canonical: **spot-unadjusted, premium
excluded**, with the premium carried as a *separate* monetary line so premium-adjusted views are
reconstructable. This canonicalization is a pure, deterministic transform of `celnet-vanilla`
outputs (it re-uses the same strike↔delta machinery) and is the *only* place convention lives — the
cube above it is convention-free. *(Celnet design proposal; the specific canonical choice is an
engineering decision, medium confidence as "the" right one — alternatives are defensible.)*

### 2.3 Currency-of-risk / common numeraire

Delta is a *currency amount*, not a scalar. For EURUSD, delta is the EUR (CCY1) hedge amount; for
USDJPY it is the USD amount. To net across books and pairs you must resolve every leg into a
**per-currency exposure vector** and convert to the reporting numeraire at spot. *(Standard; matches
Celnet `BucketedRisk` — high.)*

- **Delta** nets at the **currency-node** level: a EURUSD book's EUR leg and a EURJPY book's EUR leg
  net into a single EUR exposure; the USD legs net separately; etc. The firm view is a vector over
  currencies, then converted to the reporting ccy.
- **Vega** is normalized to a **1-vol-point (1%) move** and bucketed by `(pair, tenor)` and by
  `(tenor × delta)` — exactly Celnet's existing `BucketedRisk` shape. *(High.)* Crucially, vega
  P&L is in **premium-currency terms per position**, so it requires the *same* premium-ccy
  normalization as delta before cross-book vega netting — §2.2 and §2.3 are coupled, not separate.
  *(Celnet design point — medium; under-developed in most public treatments and a genuine source of
  bugs.)*
- **Second-order FX exposure on the spot-conversion of delta** (the reporting-ccy conversion is
  itself FX-sensitive) is real but small; carried as a separate quanto-style adjustment. *(Inference
  — medium.)*

**Regulatory tenor pillars are recalibrated — do not hard-code.** The ISDA-SIMM FX-volatility tenor
vertices are **2W, 1M, 3M, 6M, 1Y, 2Y, 3Y, 5Y, 10Y, 15Y, 20Y, 30Y** (12 vertices, shared with IR
delta). *(ISDA SIMM v2.6 — high.)* The FRTB SbM FX-vega vertices are **0.5Y, 1Y, 3Y, 5Y, 10Y** (FX
vega is one-dimensional in option maturity). *(ActiveViam/Atoti, MAR21.92 — high.)* SIMM moved to a
**twice-yearly recalibration** cadence in 2025; **v2.8 (calibrated to June 2025) is current as of
May 2026.** Celnet must treat regulatory weights/pillars as **versioned, externally-supplied data**,
never compiled-in constants — see the stale-weight caution in §7. *(ISDA SIMM release cadence —
high.)*

### 2.4 Cross-pair delta netting & triangulation

EURJPY ≈ long-EURUSD + long-USDJPY with the USD legs cancelling; net at the currency node. The
triangular spot identity S(JPY/EUR) = S(JPY/USD)·S(USD/EUR) holds. *(mathema / triangular-arbitrage
literature — high.)* FRTB's **base-currency approach** explicitly "acknowledges the triangular
relationship of currency pairs," letting a bank compute FX risk relative to a chosen base currency
rather than the reporting currency. *(Journal of Risk — high.)* In FRTB SbM each currency pair is its
own bucket, aggregated across buckets at a flat **60 % cross-pair correlation** (γ_bc = 0.60).
*(BIS d436/d457 — high for the per-pair-bucket structure and the documented 60 %; we did not land a
*primary* re-pull of γ_bc in this pass, so flag the constant medium-high.)* ISDA SIMM places all FX
in a **single bucket**. *(ISDA SIMM — high.)*

**Vol triangulation — STATE THE QUOTE CONVENTION or readers will mis-sign.** The clean
σ²(EJ) = σ²(EU) + σ²(UJ) − 2ρ·σ(EU)·σ(UJ) form (minus sign) holds **only** when both legs are quoted
against a *common quote currency* (EUR/USD and JPY/USD) and ρ is the **EURUSD–JPYUSD** correlation.
Written as EURJPY = EURUSD × USDJPY (USD is the quote of one leg and the base of the other), the
commonly-quoted **EURUSD–USDJPY** correlation enters with the **opposite** sign. Celnet therefore
stores a **signed cross-pair correlation matrix with an explicit quote-convention tag** and resolves
the sign at netting time — never assumes a global minus. *(mathema — confirmed-with-caveat; this is
the single subtlest place a cross-pair vega/vanna aggregation goes wrong.)*

**Higher-order cross-Greeks are the FX-specific hard part.** Vanna and volga dominate smile P&L; how
they net across books with different conventions, and across pairs via the vol triangle, is the
genuinely difficult aggregation. Celnet rolls up **vanna and volga** as first-class cube measures
(not just first-order delta/vega), using the same canonicalization (§2.2) and the signed correlation
matrix (above). *(Celnet design proposal — the vanna/volga roll-up is a deliberate differentiator;
see §7.)*

### 2.5 Hierarchical roll-up WITH drill-down

The position-level `RiskFact` is an **immutable leaf** (canonical convention + reporting ccy).
Roll-up and drill-down operate over the *same* facts, so a number at any node is always reconcilable
to its constituents. Key correctness rule:

- **Additive measures** (canonical delta-by-ccy-leg, raw vega ladder, vanna, volga, theta) aggregate
  by associative/commutative sum and are cheap to roll up incrementally.
- **Non-additive / correlation-weighted measures** (FRTB curvature, the correlation-weighted vega
  aggregate, VaR/ES, cross-pair-correlated vega) **must be recomputed at each node** from the node's
  constituent facts — they cannot be summed from child node results. This is the central
  architectural driver: the cube stores additive measures for instant roll-up, and re-runs the
  non-additive reducers per node on demand. *(Synthesis — sound and important; Opensee is a real
  vendor positioned on intraday high-cardinality risk-cube aggregation, corroborating the cube
  approach — high for Opensee's existence/positioning.)*

This distinction is also why FRTB SbM requires running the **whole** charge under **three correlation
scenarios** (low ×0.75 / medium ×1.0 / high ×1.25) and taking the **max** — the aggregation engine
must evaluate the inter-bucket reduction **three times**, not once. *(ActiveViam SbM docs / Clarus —
high.)*

### 2.6 Entitlements as a first-class aggregation filter

Role- and hierarchy-scoped visibility, plus **information barriers ("Chinese walls")** as *legal*
need-to-know controls, and the independence of risk/product-control from the front office, are
standard, well-documented principles. *(CFI / Fed manual principle — high.)* The design implication:
entitlements are a **first-class filter at the aggregation/drill-down API**, applied **server-side**
before any reduction, and **audited**. Pruning must happen before aggregation so a node total cannot
**leak** the magnitude of sub-trees the viewer may not see. *(Sound synthesis — high on the
principle, design-specific on the server-side-pruning mechanism.)* Full model in §4.

### 2.7 Real-time intraday vs end-of-day official risk

EOD **official/certified** risk is reconciled to **Independent Price Verification (IPV)** (often a
monthly cadence that diverges from daily marks); **intraday** risk runs continuously with
trade/hedge desync and no standard frequency, in tension between independence and practicality.
*(Risk.net / Opensee / product-control refs — high.)* Celnet already has the "official vs live"
primitive: `surface_version` / `MarkSurface` (the versioned marked-surface registry referenced in
`docs/SCALE-OUT.md`, where the joint-portfolio/IPV run executes off the hot shard). The design
mapping is: **every `RiskFact` is stamped with the `surface_version` that produced it**, so a node
can be aggregated under the *live* surface, the *official* (IPV-pinned) surface, or any historical
version — the same cube, three lenses. *(High on the primitive's existence; design mapping is sound.)*

This is also where the **FRTB P&L-Attribution (PLA) test** and **Risk-Factor Eligibility Test
(RFET)** live. PLA compares hypothetical P&L (HPL) against risk-theoretical P&L (RTPL) over 250 days
via **Spearman correlation + Kolmogorov–Smirnov** statistics; green requires **Spearman > 0.80 and
KS < 0.09**, amber between, red → the desk is forced onto the Standardised Approach. *(Zanders /
ActiveViam / Forvis Mazars — high, including the green-zone cutoffs.)* **Do NOT implement the
mean-ratio ±10 % / variance-ratio < 20 % tests** — those are the **January-2016 draft metrics,
acknowledged as problematic in 2018 and REMOVED in the January-2019 final standard (d457)**, replaced
by Spearman + KS. *(Multiple FRTB sources — high; this is the one place a naive implementation would
be wrong.)* RFET (the 24-real-price-observations-in-12-months / max-1-month-gap rule) is the upstream
gate that determines which FX-vol points become **Non-Modellable Risk Factors (NMRFs)** attracting a
Stressed-ES (SES) add-on — long-dated/EM FX-vol points are the typical NMRFs. *(Standard FRTB IMA —
high.)* PLA/RFET are the *real* reason intraday risk must reconcile to the official/IPV pipeline.

### 2.8 The value-date / settlement axis

Risk by **value date / settlement date** is a distinct seventh aggregation axis (in `RiskFact`
above), separate from booking-location. It connects to **CLS settlement windows** and the
weekend/settlement gap, and is where **FX settlement / Herstatt risk** would be measured. Celnet
carries `value_date` as a dimension so settlement-bucketed exposure is a first-class roll-up; the
settlement-*capital* treatment (CLS PvP) is scoped out of the first cut (§1). *(Synthesis — design
proposal; the value-date-as-axis point is a real omission in most treatments.)*

### 2.9 Follow-the-sun book passing

FX trades 24×5; the same economic book is managed sequentially London → New York → Asia, and the
consolidated firm view must stay continuous. *(High.)* The accurate operational picture is **regional
co-location with the engine following liquidity** — e.g. *swinging a single pricing engine* between
London and New York. The earlier-circulated "three concurrent regional engine instances passing the
book" figure is **not supported by primary sources** (the cited blog describes swinging *one* engine
L↔NY) and is dropped here as unverified. *(mdavey blog — high for single-engine swing; the
"three-instance" count is refuted/unverified.)*

Celnet's relevant primitives, **stated honestly by build status**:

- **Blue-green handoff** (`celnet-engine`) — **BUILT** per `docs/SCALE-OUT.md`. This is the
  state-preserving cutover primitive that makes "the book continues across an engine swing" real.
- **HRW (rendezvous-hash) partition map / router tier / replicated log** (`celnet-router`) —
  **DESIGNED ONLY, not built** per `docs/SCALE-OUT.md` §8. This document does **not** present the
  fleet/router layer as shipped; it is the target for the scale-out aggregation work in §3.

*(SCALE-OUT.md — high; the build-status distinction is load-bearing for honesty.)*

### 2.10 The risk dimensions carried on each node

Each cube node exposes, at any level:

| Dimension | Content | Additive? |
|---|---|---|
| **Greeks** | delta-by-ccy-leg (canonical), gamma, vega, theta, **vanna, volga**, charm/speed/zomma/color | additive (first/second order); correlation effects re-derived |
| **Bucketed vega** | vega ladder by `(tenor × delta)` and by `(pair, tenor)` — Celnet `BucketedRisk` | raw ladder additive; correlation-weighted aggregate re-derived |
| **Scenario / stress** | spot×vol grids; FRTB curvature (two ±RW shocks, worse side); historical & hypothetical stress | re-derived per node |
| **VaR / ES** | ES 97.5 % one-tailed, stressed-calibrated, LH ∈ {10,20,40,60,120}, NMRF→SES add-on | re-derived per node |
| **FRTB-SA sensitivities** | delta/vega/curvature per risk-class→bucket→factor; 3 correlation scenarios → max; RRAO (1.0 % exotic / 0.1 % other) | re-derived per node |
| **P&L-explain** | Greek-additive Taylor decomposition + higher-order (vanna/volga, gamma-covariance) + residual | additive components + re-derived residual |

*(FRTB constants — ES 97.5 %/LH set, curvature mechanics, 3-scenario-max, RRAO 1.0 %/0.1 %, FX-vega
RW_σ = 55 % with RW = min(RW_σ·√(LH/10), 100 %) and LH = 40d, FX-delta RW = 15 % / liquid-pair
15/√2 ≈ 10.6 % — all verified high. **DRC is immaterial for vanilla FX** (no issuer-default leg) —
stated explicitly rather than left dangling.)*

### 2.11 FRTB-SA capital aggregation — the full SbM charge (BUILT)

The non-additive lenses of §2.5 (FRTB-SbM **curvature** reprice, and the
`√(quadratic-form)` **correlation-weighted vega**) are the building blocks; the
**Standardised-Approach capital aggregation** that turns net sensitivities into a
single capital number is now built in `celnet-risk-cube::frtb` (BCBS **MAR21** SbM,
**MAR23** RRAO, **MAR22** DRC). It **composes with**, and does not duplicate, the
§2.5 lenses — the within-bucket `K_b`, the vega aggregation and the cross-bucket step
all route through one shared `√(Σ WS² + ΣΣ ρ WS WS)` quadratic-form kernel
(`frtb::quadratic_form`), gated bit-identical to the existing
`nonadditive::correlation_weighted_vega`.

- **Within bucket** (MAR21.4): `K_b = √( max(0, Σ_k WS_k² + Σ_k Σ_{l≠k} ρ_{kl} WS_k
  WS_l) )` for delta/vega, with `WS_k = RW_k·s_k`.
- **Across buckets** (MAR21.5): `K = √( Σ_b K_b² + Σ_b Σ_{c≠b} γ_{bc} S_b S_c )` with
  the signed bucket sums `S_b = Σ WS_k`, and the **MAR21.6 low-correlation
  alternative** `S_b = max(min(Σ WS_k, K_b), −K_b)` applied only when the radicand
  would go negative.
- **Three correlation scenarios** (MAR21.6): the whole charge is evaluated under
  **HIGH** (`ρ,γ → min(1, 1.25ρ)`), **MEDIUM** (prescribed), and **LOW**
  (`ρ,γ → max(0, 2ρ−1)`), and the reported capital is the **maximum** of the three —
  the defining SbM property. For the SbM *total* across risk classes the single
  scenario that maximises the *sum* of the per-class charges is chosen (MAR21.6).
- **Curvature** (MAR21.5.2): per-bucket `K_b = max(K_b^+, K_b^-)` over the up/down
  reprice-net-of-delta legs (the same arithmetic as the §2.5 curvature lens, re-used
  via `frtb::curvature_legs`, with the selected direction's signed `CVR` and the
  `ψ`-zeroing of both-negative cross terms carried into the cross-bucket `γ²` sum).
- **RRAO** (MAR23): `Σ |notional|·weight` — **1.0 %** of gross exotic-underlying
  notional + **0.1 %** of gross other-residual notional. For an FX-**exotics** book
  this is the most relevant FRTB piece: barriers, digitals, one-touches/DNTs and
  TARFs are textbook `OtherResidual` (gap/digital risk).
- **DRC** (MAR22): an **honest, documented zero** for a deliverable-FX book. DRC
  capitalises *issuer* jump-to-default; a deliverable FX option references two
  sovereign currencies, not a defaultable issuer security, so its gross JTD — and
  hence market-risk DRC — is identically zero (settlement / counterparty risk is the
  **CCR/CVA** framework, not market-risk DRC). The JTD aggregation is implemented and
  returns zero on FX with that rationale; no fabricated non-zero charge.

All risk weights and correlations are **caller-supplied data** (`SbmParams`,
`RiskBucket::rho_intra`, the `γ` closure), never compiled in — a recalibration is
data, not a recompile. Validated in `celnet-parity/tests/frtb.rs` against a
**longhand** independent recomputation (the SbM aggregation written out by hand to
~1e-10), the three-scenario-max identity, single-bucket reduction, perfectly-hedged
`K_b = 0`, monotonicity, the exact RRAO hand-sum, and the documented FX DRC zero.

---

## 3. Computing this at scale

### 3.1 Tie to the existing zero-alloc engine

The pinned hot core (`celnet-engine`, log/lock/alloc-free) already produces, per instrument, the
exact leaf measures the cube needs: full Greeks (incl. vanna/volga from `celnet-vanilla`) and
`BucketedRisk` (vega by `tenor × delta`, cross-gamma stencil, theta-roll). **The hot core does not
change.** The aggregation layer sits **downstream** of it, consuming a stream of `RiskFact` leaves
over the existing bounded SPSC offload queue (the same telemetry-style seam that keeps the hot core
allocation-free). Aggregation, limits, and entitlement evaluation run on **non-critical cores**, off
the hot path — preserving the latency budget in `docs/ARCHITECTURE.md` §1.2. *(Design proposal,
consistent with the engine's existing offload architecture.)*

### 3.2 New services (crates — **BUILT & SERVED**)

These four crates are **built and gated green**, and exposed over the one `celnet-proto` contract as
`RiskService` (served by `celnet-server` over gRPC + the WS mirror — see the Implementation-status note
under §1 and `docs/INTERFACES.md` §"Phase-2 contract: `RiskService`"). The roll-up runs **server-side**
off a shared live `PositionStore`; clients never sum positions themselves.

- **`celnet-risk-cube`** — the dimension/hierarchy model + the OLAP fact store and group-by/reduce
  engine. Holds the immutable `RiskFact` table, the additive-measure roll-up index (incremental:
  a new/changed fact updates ancestor sums in O(depth)), and the per-node re-derivation of
  non-additive measures (VaR/ES, FRTB curvature, correlation-weighted vega, 3-scenario SbM).
- **`celnet-risk-normalize`** — the convention-canonicalization (§2.2) and common-numeraire
  conversion (§2.3) transforms; pure functions over `celnet-vanilla`/`celnet-surface` outputs.
- **`celnet-limits`** — the limit tree, utilization, pre/post-trade checks, breach/escalation (§5).
- **`celnet-entitlements`** — the entitlement model + server-side pruning filter (§4).

These depend on `celnet-types` (dimension keys), `celnet-core` (math), and `celnet-surface`
(for re-pricing under scenario shocks). They sit **above** `celnet-engine` and feed `celnet-server`'s
streaming edge.

### 3.3 Throughput: incremental roll-up + AAD/batched-GPU, not bump-and-revalue

At investment-banking portfolio scale, **bump-and-revalue does not scale** for the non-additive
measures (a full-book reval on every tick or every limit check is exactly why incumbents batch).
Celnet's answer is three-fold:

1. **Additive measures roll up incrementally** — a single trade touches O(depth) ancestor sums, not
   the whole book. This makes first-order Greek and raw-vega-ladder roll-up effectively free per
   trade.
2. **Non-additive measures use a recompute-trigger strategy**, not every-tick reval: delta-driven
   (recompute a node only when its constituents' risk moves beyond a threshold) and throttled
   (bounded cadence per node), so the cube cost tracks *activity*, not clock ticks.
3. **Scenario/VaR/curvature reval uses adjoint algorithmic differentiation (AAD) and batched GPU**
   (`celnet-gpu`) — the same Philox-seeded, f32-GPU/f64-CPU-reconciled path used in pricing. This is
   the **single most important throughput requirement**: a firm-hierarchical risk claim that relied
   on bump-and-revalue would lose on throughput exactly where it claims to win. Numerix's headline
   scaling lever for portfolio Greeks is AAD *(numerix.com/oneview-xva — high)*; Celnet matches it.
   **Build-status honesty: this is built, validated, and benched.** Two levers:
   - **AAD adjoint Greeks.** `celnet_vanilla::adjoint_greeks` is **genuine reverse-mode** algorithmic
     differentiation of the Garman-Kohlhagen graph (a forward struct-tape record → a reverse adjoint
     sweep, *not* finite differences and *not* the analytic closed forms renamed). One sweep yields the
     full first-order block (`delta`, `vega`, `theta`, `rho_d`, `rho_f`) at a small constant multiple of
     one price, *independent of factor count* (O(1) vs bump-and-revalue's O(factors)); `gamma`/`vanna`/
     `volga` come from a reverse-over-reverse second-order sweep, and the mixed/third-order
     `charm`/`speed`/`zomma`/`color` are taken verbatim from the validated analytic closed forms (this
     boundary is documented honestly in the module, not faked as AAD). It is wired as the **default**
     additive leaf (`celnet-risk-normalize::canonicalize`) and as the engine of the non-additive
     **sensitivity-based** VaR/ES (`celnet_risk_cube::sensitivity_var_es`): one adjoint sweep per
     position, then each scenario's node P&L is a second-order Taylor expansion in the shocked factors —
     `O(positions)` sweeps + `O(positions × scenarios)` cheap arithmetic, versus
     `O(positions × scenarios)` repricings for bump-and-revalue.
   - **Batched-GPU scenario reval.** `celnet_risk_cube::gpu_pv_grid` drives `celnet-gpu`'s
     `ScenarioPricer::price_scenario_batch` — one GPU dispatch per position over a whole spot×vol
     ladder under common random numbers — falling back to the exact f64 CPU oracle when no adapter is
     present (headless CI). The f32 GPU result reconciles per-node to the f64 CPU/closed-form oracle
     within `celnet-gpu`'s first-principles f32 bound, and `NodeScenarioGrid::reconciles_to_analytic`
     gates each node against the closed-form analytic PV inside the Monte-Carlo standard-error band.

   The **bump-and-revalue / closed-form path is retained as the oracle** the AAD/GPU paths are
   validated against (`celnet_risk_cube::node_var_es` / `node_curvature_spot` / `analytic_pv_grid`),
   never removed. **Measured (M4 Metal, `celnet-bench`):** the AAD first-order block runs ~42 ns vs
   ~129 ns for central-difference bump-and-revalue (~3.0× on a 5-factor block; the gap widens linearly
   with factor count). The batched-GPU scenario grid (121 nodes × 3 positions, 65 536 paths/node) runs
   ~13 ms vs ~165 ms for unbatched per-node MC dispatch (~12.7× — the genuine batching win within the
   Monte-Carlo regime). **Honest crossover:** for *vanilla* payoffs the exact closed form (~9.6 µs)
   dominates both MC paths, because vanillas need no paths; the GPU-batched MC kernel is the scale path
   for path-dependent / no-closed-form payoffs and for grids large enough to amortize MC, not for
   analytic vanillas. The remaining throughput frontier is the cross-shard fleet fan-out (§3.4), which
   stays deferred.

### 3.4 Scale-out

Horizontal scale-out reuses the (designed-only) `celnet-router` HRW partition map: positions
partition by a stable key (e.g. `(legal_entity, ccy_pair)`), each shard owns its slice of the fact
table and rolls it up locally; a **cross-shard reducer** combines shard-level additive measures
directly and re-derives non-additive firm-level measures from shard contributions where the measure
permits, or gathers constituent facts where it does not. IPV / joint-portfolio runs execute off the
hot shard, per `docs/SCALE-OUT.md`. **Build-status honesty: the router/HRW tier is designed, not
built** — §3.4 is the target architecture, not a shipped capability.

### 3.5 Latency budget — set a Celnet-specific number, don't borrow the FIX figure

The pre-trade limit check (§5) must run inside the engine's tail-latency budget. The widely-cited
**~4 µs** pre-trade risk-check figure is a **FIX-stack** number (B2BITS, Linux + Solarflare
OpenOnload, co-located risk-check module) *(b2bits.com — high)* — it is **not** an
options-revaluation cost. An incremental-Greek limit check on an options book is a different cost
class. Celnet must publish its **own** budget for the incremental pre-trade Greek/utilization check
(target: low-single-digit µs for the additive-Greek limit path, separate from any scenario reval) and
gate it in CI like the pricing path. *(Honest design point — no competitor publishes a portfolio
risk-roll-up latency number, so Celnet can be the first to publish one, but must not claim to beat a
number that does not exist; see §7.)*

---

## 4. Entitlements model (who sees what)

- **Principals & roles.** A principal (trader, risk manager, product control, CRO, auditor) carries
  a set of **role grants**, each scoped to a **dimension subtree** (e.g. "read risk for desk = EM-vol,
  any book"; "read firm-consolidated VaR only"; "no view across the information barrier into desk X").
- **Server-side pruning before aggregation.** The entitlement filter is applied **before** any
  reduction: the cube computes a node total only over facts the principal may see. This prevents
  **aggregate leakage** — a parent total that betrays the size of an invisible sub-tree. *(High on
  the principle; the pre-aggregation enforcement point is the design-specific mechanism.)*
- **Information barriers** are modeled as **deny rules** that cut specific dimension subtrees for
  specific roles, enforced as legal need-to-know controls and **audited** (every entitlement decision
  logged via `celnet-observability`). *(CFI / Fed manual — high.)*
- **Separation of duties** — risk/product-control principals have read scopes the front office does
  not, reflecting the independence of the risk function. *(High.)*
- **Drill-down respects entitlements at every level** — a principal can drill only into subtrees
  their grants cover; the breadcrumb/filter path is itself entitlement-checked.

---

## 5. Limits framework

Limits cascade **down the same hierarchy** as risk and are checked **pre-trade at multiple nodes
simultaneously** (a trade consumes limit at the trader, book, desk, ccy-pair, and entity nodes at
once) — this is the operational reason the cube must aggregate in real time.

### 5.1 Limit types

*(All a verified industry-standard taxonomy — high.)*

- **Greek limits** — delta / gamma / vega (and **vanna / volga**, the FX-specific additions),
  per-pair and aggregate.
- **Bucketed-vega limits** — per `(tenor)` or `(tenor × delta)` pillar; plus FX-specific **gap / pin
  risk** near barriers and fixings (WMR 4pm London, Tokyo cut), and **NDF vs deliverable**
  distinctions.
- **Concentration limits** — single-pair, single-tenor, single-counterparty.
- **VaR / ES limits.**
- **Scenario / stress limits** — loss under a defined shock grid.
- **Stop-loss limits.**

### 5.2 Per-level setting, soft vs hard

Limits are set at any node and **cascade board → entity → desk → book → trader**, with child limits
constrained by parents. Each limit is **soft** (early-warning, e.g. 80 %/90 % utilization alerts —
illustrative thresholds, configurable) or **hard** (blocking). Utilization = exposure / limit is a
first-class node measure. *(Verified taxonomy + cascade — high; specific 80/90 % thresholds are
illustrative.)*

### 5.3 Pre/post-trade, breach & escalation

- **Pre-trade / at-trade** — the incremental risk of a proposed trade is added to the relevant nodes
  and checked against every limit on the path *before* execution (MiFID II / SEC 15c3-5 mandate
  pre-trade controls — high). Hard-limit breach **blocks**; soft breach **warns**.
- **Post-trade** — continuous utilization monitoring; breach triggers **escalation workflow**
  (suspend / hedge / block, four-eyes approval for temporary excess), mirroring documented incumbent
  workflows. *(Murex "limit suspension, trade hedging or blocking contracts breaching limits" +
  four-eyes; Nasdaq Calypso real-time pre-deal checks + automatic actioning — high.)*
- **Latency** — the additive-Greek pre-trade path runs inside the µs-class budget of §3.5; scenario/
  VaR limits that need reval run on the recompute-trigger cadence of §3.3.

---

## 6. The trader / risk-manager experience

SOTA, macOS-polished, real-time. Corroborated by `docs/GUI-DESIGN.md`, which already encodes
flash-as-signal, a perceptual diverging color ramp, a WebGPU surface, and a virtualized blotter.

- **Hierarchical risk explorer** — a **lazy / virtualized tree grid with a server-side row model**;
  aggregation happens on group nodes (the cube does the netting, the grid renders it). At IB
  cardinality this needs O(log n) streaming-sort over a custom tree rather than O(n) re-sort on every
  delta — a documented technique (AG-Grid's `deltaSort` is O(n); a custom tree gets O(log n)).
  *(AG-Grid / Proof Trading case study — high; note AG-Grid does not support pivot + tree-data
  together — high.)* The drill path doubles as a **breadcrumb + filter**. Anchored on the canonical
  InfoVis pattern: **overview first, zoom & filter, details on demand** (Shneiderman). *(Design
  proposal anchored on canonical InfoVis — high for the pattern.)*
- **Heatmaps** — **vega-by-tenor×delta** and a **signed cross-pair correlation** heatmap (with the
  quote-convention-correct sign of §2.4). Use **perceptually-uniform, colorblind-safe** colormaps
  (viridis/magma family) and a **diverging** map for signed data; rainbow is a *correctness* hazard
  (non-uniform "kinks"; ~8 % of men have red-green deficiency). *(Colormap science — high; prevalence
  ~5–8 %, cite a prevalence source, medium on the exact 1-in-12 figure.)* GPU/WebGPU rendering makes
  large grids interactive (OSS WebGPU demos render millions of points at >100 FPS — high as "the tech
  can do this," not a competitor benchmark).
- **Greeks blotter with bucketed ladders** — a streaming blotter showing PV / Δ / Γ / Vega / Θ across
  a user-set spot range, vega per 1 vol point, theta as a one-day roll — the documented Saxo
  "Options Risk Ladder" pattern *(help.saxo — high; SaxoTrader-desktop-only — note)*. Celnet rolls up
  **vanna/volga** in the ladder too, which no incumbent publicly documents (§7). Streaming-grid
  scale references (≈50k records/**day**, each updating 5–100×; 1-second bucketed payloads) are
  per-day record counts, not 50k concurrently-live rows. *(Proof Trading case study — high, with the
  per-day clarification.)*
- **Limit dashboard** — utilization + limit + % with RAG status / gauges, soft-breach pre-alerts,
  escalation actions inline. *(Verified pattern; Nasdaq/Murex real-time breach actioning — high.)*
- **Scenario / stress** — spot×vol grids, **tornado charts** of factor sensitivities, and **what-if /
  pre-trade** simulation. Live what-if sliders re-stream risk via the recompute-trigger path of §3.3
  (not a naive full-book reval per slider tick). *(Murex what-if — high; tornado — high; the latency
  caveat is the honest design point of §3.3.)*
- **P&L-explain waterfall** — Greek-additive Taylor decomposition **plus higher-order vanna/volga and
  gamma-covariance** terms and a residual, consistent with the richer-decomposition view of recent
  options-PLA literature (Daviaud, J.P. Morgan, Risk.net 2024 / SSRN 4495530 — high that the article
  exists and covers higher-order/vol-spot effects; we do **not** pin a "residual = diagnostic" thesis
  on it).
- **Session-aware & auditable** — the explorer is **follow-the-sun session-aware** (cut times,
  fixings) and supports **point-in-time replay** ("what did the desk see at 14:32?") via the
  deterministic-replay recovery model + `surface_version` stamping. *(Design proposal; the
  handover-UI screen specifically is inference/low — no incumbent screen was found.)*
- **Accessibility** — beyond colormaps: WCAG contrast, keyboard navigation of the deep virtualized
  tree, and screen-reader semantics for the streaming grid (a known-hard problem). *(Design
  requirement.)*

---

## 7. Competitive critique

**Reading guide.** Where a cell credits an incumbent with a capability, it is documented (high) unless
flagged. Where a cell marks a gap, it is an **argument-from-absence** (no public evidence of the
feature) — *not* proof the vendor lacks it; flagged **inf** (inference). The honest whitespace Celnet
targets is **streaming, push-native, hierarchical (trader→book→desk→entity) FX-options Greek roll-up
with vega-by-tenor/delta + vanna/volga + cross-gamma at sub-second cadence, upstream of FRTB
reporting, with a sandboxed model SDK and in-process embeddability** — none of which any incumbent
*publicly documents*. Every adjacent capability they *do* have is credited, so the wedge reads as
credible rather than strawman.

### 7.1 Capability matrix

| Capability | Bloomberg MARS | Murex MX.3 | Numerix OneView | Calypso / Adenza (Nasdaq) | Fenics FX | SuperDerivatives / ICE | ION XTP Risk | Synoption Omega | **Celnet (proposed)** |
|---|---|---|---|---|---|---|---|---|---|
| **Firm-wide hierarchical aggregation** | Consolidates "across your firm"; **no** named trader/desk/entity risk-**tree drill-down UI** documented `[MARS page / high; tree-UI absent — inf]` | **Strongest reference**: intraday risk at **book/desk/global continuously**, drill to **finest inputs** (trades/sensitivities), desk/BU reallocation `[murex.com / high]` | Risk by **desk/sector/region/ccy/custom**, trade→enterprise `[numerix.com / high]` | **Multi-entity, any legal-entity complexity**; global-position→trade drill `[nasdaq.com / high]` | Single-interface pricing/risk/STP; **no** firm→entity roll-up documented `[bobsguide / high for suite; roll-up absent — inf]` | Independent portfolio valuation, MTM any cutoff; **no** firm roll-up documented `[ICE/Wikipedia / high; roll-up absent — inf]` | **Configurable real-time hierarchy** (client/region/desk/market/asset), 80+ measures `[iongroup.com / high]` | **Dynamic multi-level portfolio trees** by trader/ccy/exchange/account `[synoption.com / high]` | **Cube** over immutable facts, all 7 dims, convention-normalized, **drill-to-position** with entitlement pruning |
| **Real-time cadence** | Intraday + EOD `[high]`; not a documented µs streaming bus `[inf]` | Intraday **continuous** recompute on a JVM **grid** `[high]` | Cloud-native on-demand recompute `[inf]` | Real-time pre-deal + dashboards `[high]` | Instant revaluation `[high]` | RFQ/valuation cadence `[inf]` | **Real-time** dashboards `[high]` | Real-time analytics + proactive alerts `[high]` | **Tick-driven streaming** off the zero-alloc core; sub-second roll-up |
| **Architecture** | API on B-PIPE/SAPI/BQL/BQNT `[high]` | **Apache Storm / Spark / Pivotal Gemfire in-memory grid + CPU/GPU compute grid** `[murex.com/.../mx3-architecture / high]` — JVM/GC batch-recompute fabric | Cloud + GPU American MC, **AAD** for XVA Greeks `[high]` | Platform | BGC subsidiary suite `[high]` | ICE-owned `[high]` | HTML5 dashboards; cleared/listed-margin lineage `[high]` | Listed+OTC portfolio module (~2022 launch) `[TheFullFX / high]` | **Pinned zero-alloc, lock/alloc/log-free Rust hot core + SPSC streaming**; aggregation off-core |
| **FRTB-SA / IMA** | MARS regulatory capital `[high]` | **FRTB-SA & IMA**, full-reval vs Taylor `[high]` | Market-risk + XVA `[high]` | **VaR + ES** (param/hist/MC), stress `[high]` | — `[inf]` | — `[inf]` | Listed/cleared margin (SPAN2) focus `[high]` | — (not published) `[low]` | **Feeds** FRTB delta/vega/curvature buckets at **streaming cadence** — *upstream of*, not competing with, the reg-reporting layer |
| **Vega-by-tenor×delta + vanna/volga as aggregation dims** | "FX Vega" + "IR Vega Matrix" named; FX vega **matrix** not evidenced `[high names; matrix — inf]` | Sensitivities drill-down `[high]`; vega-matrix-as-dim not evidenced `[inf]` | Cross-Greeks + ladder reports `[high]` | Implied-vol sensitivities `[medium]` | — `[inf]` | — `[inf]` | — `[inf]` | — `[low]` | **First-class streaming dimension** (vega tenor×delta + cross-gamma + **vanna/volga** roll-up) |
| **Hierarchical limits + pre/at-trade** | — `[inf]` | **Suspend/hedge/block + four-eyes + reallocation** `[high]` | Pre-deal limits implied `[inf]` | **Real-time pre-deal limit checks + auto-action** `[high — Calypso moat]` | — `[inf]` | — `[inf]` | Real-time limit monitoring `[high]` | Proactive risk alerts `[high]` | **Limit tree** cascading board→entity→desk→trader, pre/at/post-trade, µs additive path |
| **User-pluggable sandboxed model SDK** | BQuant/Python research scripting (not in-process model injection) `[medium/inf]` | Vendor PS customization `[inf]` | Model library/scripting `[medium]` | Template config `[inf]` | — `[inf]` | — `[inf]` | Template-configurable `[inf]` | — `[low]` | **wasmi/native tiered plugin host** (in-process, fuel-metered sandbox) `[high]` |
| **Deployment / embeddability** | Seat-priced; rides B-PIPE/SAPI; not an embeddable in-process lib `[high/inf]` | Licensed platform `[high]` | Licensed/cloud `[high]` | Licensed platform `[high]` | Licensed `[high]` | Licensed/feed `[high]` | Licensed platform `[high]` | Subscription `[high]` | **In-process, hot-upgradable, embeds into the Celer estate** `[design]` |

### 7.2 The two incumbent archetypes

The competitive set splits cleanly:

- **Enterprise hierarchical risk platforms** (Murex MX.3, Numerix OneView, Calypso/Adenza, Bloomberg
  MARS): genuinely do multi-entity / desk-level aggregation, FRTB-SA/IMA, VaR/ES, limits — but on
  **batch / scheduled / on-demand recompute** fabrics (Murex's named **Storm/Spark/Gemfire JVM grid**
  is the clearest example; a GC'd grid is architecturally distinct from a pinned zero-alloc Rust hot
  core). Their 2025 roadmaps confirm the seam is still open: Bloomberg MARS added **buyside
  fund-leverage VaR (SEC 18f-4 / UCITS / AIFMD)** in Feb-2025 — a *reporting/EOD* move; ION enhanced
  XTP Risk JANUS with AI for **SPAN2 margin approximation** in Apr-2025 — a *cleared-margin* move.
  Neither moved toward push-native streaming FX-options Greek roll-up. *(prnewswire 302368155 /
  302439502 — high.)*
- **FX-options specialist / single-book tools** (Fenics FX, SuperDerivatives/ICE, Synoption Omega):
  strong FX-options pricing and (Synoption) dynamic portfolio trees by trader/ccy/exchange/account,
  but with **no public evidence** of firm→entity→country consolidation or FRTB capital. SuperDeriv-
  atives pioneered real-time internet FX-options pricing and independent portfolio valuation
  *(ICE/Wikipedia — high)*; Synoption Omega (~2022) spans listed + OTC with proactive alerts and
  delta-hedge lifecycle *(synoption.com / high)* — but the deep firm hierarchy and a formal limit
  tree are **not published** (low).

### 7.3 Where the incumbents are pricing-tools-not-risk-systems

Fenics FX and SuperDerivatives/ICE are, on the public evidence, **FX-options pricing/valuation
tools** with portfolio valuation — there is **no public evidence** of a firm→entity→desk→trader→book
limit-and-risk hierarchy. ION XTP Risk is a genuine real-time risk hierarchy but with a **listed /
cleared-margin** center of gravity (SPAN2), not an FX-OTC-options analytics engine; ION's own FX
T&RM does **not** publicly document FX-options Greeks. *(iongroup.com — high that FX-options Greeks
are not documented.)* These are honest argument-from-absence flags, not asserted deficiencies.

### 7.4 No competitor publishes a portfolio-risk latency/throughput number

A standalone finding: across the entire set, **essentially zero quantitative latency or throughput
figures are published for FX-options portfolio risk roll-up.** That absence is Celnet's opening for a
**published-benchmark headline** — but it also means Celnet can only claim to be the **first to
publish** such a number, not to "beat" a number that does not exist. *(Inference across all vendor
material — defensible.)*

---

## 8. Citations

**Regulatory / standards (high):**

- BCBS *Minimum capital requirements for market risk* (FRTB), d436 / d457 — SbM risk-class→bucket→
  factor structure; FX delta RW 15 % (liquid-pair 15/√2 ≈ 10.6 %); FX-vega RW_σ 55 %, LH 40d,
  RW = min(RW_σ·√(LH/10), 100 %); curvature two-shock mechanics; three correlation scenarios
  (×0.75/×1.0/×1.25, take max); RRAO 1.0 % exotic / 0.1 % other; ES 97.5 % stressed, LH set
  {10,20,40,60,120}; PLA (Spearman > 0.80 & KS < 0.09; **2016 mean/variance ratio tests REMOVED in
  2019 d457**); RFET / NMRF / SES; FX **base-currency approach** acknowledges triangulation.
- BCBS MAR12 — FRTB trading-desk definition (unambiguous group, head trader, single-desk assignment);
  via Accenture / GreenPoint / OSFI CAR Ch.9.
- Federal Reserve trading & capital-markets supervision manual — consolidated-basis aggregation
  principle (paraphrase, not verbatim).
- ISDA SIMM v2.6 (FX-vol tenor vertices 2W…30Y; single FX bucket; vega concentration thresholds) and
  the v2.8 / twice-yearly-recalibration cadence (current as of May-2026; FX vega RW ≈ 0.47–0.48 in
  v2.4–2.6 — **0.21 is the stale R1.x figure, do not use**).
- MiFID II / SEC Rule 15c3-5 — pre-trade risk-control mandate.

**Vendor / product (high unless noted):**

- Bloomberg MARS product page (modules; FX Delta / FX Vega / IR Vega Matrix; intraday+EOD; API on
  B-PIPE/SAPI/BQL/BQNT; firm consolidation — **no named risk-tree UI**); OVML/OVDV/BVOL/VCUB.
  prnewswire 302368155 (MARS buyside-leverage VaR, Feb-2025).
- Murex MX.3: murex.com enterprise-risk / market-risk (intraday book/desk/global, drill-to-finest-
  input, suspend/hedge/block + four-eyes, VaR/ES, FRTB-SA/IMA, what-if); murex.com/.../technology/
  mx3-architecture (**Apache Storm / Spark / Pivotal Gemfire in-memory grid + CPU/GPU grid** — current
  page). ("never twice" engine phrasing — secondary, medium.)
- Numerix: numerix.com/oneview-market-risk (risk by desk/sector/region/ccy/custom); numerix.com/
  oneview-xva (cross-Greeks, ladder reports, **AAD** for XVA Greeks, CSA hierarchy/scripting);
  Risk.net 2023 XVA Calculation Product of the Year.
- Nasdaq Calypso: nasdaq.com middle-office-trading-risk (multi-entity, real-time pre-deal limits,
  what-if, stress, VaR + ES param/hist/MC); Adenza→Nasdaq close **Nov 1, 2023** (Nasdaq 8-K, SEC).
  (Calypso "bucketed / implied-vol sensitivities" exact wording — medium.)
- Fenics: bobsguide.com/fenics-professional (350+ sites, single interface, BGC subsidiary, instant
  revaluation; founded 1987 — source is dated). Firm roll-up — **inference/medium negative**.
- SuperDerivatives / ICE: ICE press release / Business Wire (close **Oct 7–8, 2014, ~$350M**); first
  real-time internet FXO pricing tool; independent portfolio valuation MTM any cutoff. Firm roll-up —
  **inference/medium negative**.
- ION XTP Risk: iongroup.com/products/markets/xtp-risk/ (80+ measures, configurable real-time
  client/region/desk/market/asset hierarchy, HTML5); iongroup.com FX T&RM (FX-options Greeks **not**
  documented); prnewswire 302439502 (JANUS AI / SPAN2, Apr-2025).
- Synoption Omega: synoption.com/omega.php (dynamic multi-level portfolio trees by trader/ccy/
  exchange/account, proactive alerts, real-time analytics, delta-hedge lifecycle); TheFullFX / EIN
  Presswire (portfolio module ~2022, listed + OTC). Exact Greeks/vega-buckets/limit-hierarchy — **not
  published / low**.

**UX / technique (high unless noted):**

- AG-Grid docs (group-node aggregation; pivot **not** supported with tree-data); blog.ag-grid.com
  Proof Trading case study (custom O(log n) tree sort vs O(n) `deltaSort`; ≈50k records/**day**,
  5–100 updates each, 500k+ target, 1-sec bucket payloads — per-day counts).
- help.saxo Options Risk Ladder (PV/Δ/Γ/Vega/Θ over user-set spot range; vega = 1 vol pt; theta =
  1-day roll; SaxoTrader-desktop-only).
- Colormap science: mpetroff.net / viridis / BIDS colormaps (rainbow non-uniform; viridis/diverging
  recommended); red-green deficiency prevalence ~5–8 % (cite a prevalence source — medium on the
  exact 1-in-12 figure).
- WebGPU rendering demos (ChartGPU etc.) — millions of points >100 FPS (OSS demos, "tech can do
  this", not a competitor benchmark).
- Shneiderman, *The Eyes Have It* (1996) — overview-first / zoom-filter / details-on-demand
  (canonical InfoVis pattern).
- Risk.net / SSRN 4495530 (Daviaud, J.P. Morgan, 2024) — higher-order options P&L attribution
  (vanna/volga, gamma-covariance); we do **not** attribute a "residual = diagnostic" thesis to it.
- Opensee (opensee.io) — intraday high-cardinality risk-cube vendor positioning.
- B2BITS (b2bits.com) — ~4 µs pre-trade risk check on a tuned FIX stack (a FIX-stack figure, **not**
  an options-reval figure).
- mdavey blog — follow-the-sun "swing a single pricing engine" L↔NY (the "three regional instances"
  count is **unverified / dropped**).

**Internal (Celnet docs):**

- `docs/CONVENTIONS.md` — `DeltaConvention` / `AtmConvention` / `PremiumStyle` enums (verbatim).
- `docs/SCALE-OUT.md` — blue-green handoff **BUILT**; router / HRW partition map / replicated log
  **DESIGNED ONLY**; IPV runs off the hot shard.
- `docs/GUI-DESIGN.md` — flash-as-signal, perceptual diverging ramp, WebGPU surface, virtualized
  blotter, SynOption-Optimus framing.
- `docs/ARCHITECTURE.md` §1.2 — latency/throughput budgets; `celnet-engine` `BucketedRisk`.

---

> **Verified-vs-proposed boundary.** Everything in §7 with a `[source / confidence]` tag and
> everything in §8 is **verified fact** at the stated confidence. The Celnet dimension model (§2.1),
> canonical-convention choice (§2.2), service decomposition (§3.2), entitlement pre-aggregation
> pruning (§4), limit tree (§5), and UX (§6) are the **Celnet design proposal** — engineering
> decisions grounded in the verified facts but not themselves vendor-sourced. The two most important
> honest risks: (1) the firm-hierarchical scale claim **depends on AAD/batched-GPU** rather than
> bump-and-revalue — **now built and benched** (§3.3: genuine reverse-mode AAD as the additive leaf +
> sensitivity-VaR engine, ~3.0× vs bumps; batched-GPU scenario grid, ~12.7× vs unbatched MC; the
> bump/closed-form oracle is retained as the validation reference); (2) the router/HRW scale-out tier
> is **designed, not built** (§2.9/§3.4) — the one remaining throughput frontier.
