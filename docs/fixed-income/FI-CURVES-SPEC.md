# Celnet — Fixed-Income Curves Spec (multi-curve construction)

**Status:** P0 spec (first synthesis) · 2026-06-22 · branch `feature/fixedincome`
**Scope:** the multi-curve term-structure foundation for `celnet-rates` — instruments → curves,
internal representation, interpolation, OIS/CSA discounting, calibration, and the deterministic
numerics. Optionality (swaptions/caps), credit, inflation, and cash-bond static-data breadth are
**deferred** to later passes (`celnet-rates-vol` and a cash-bond pass).
**Synthesised from:** [`../_research/fixed-income-findings.md`](../_research/fixed-income-findings.md)
§1/§2/§4 (every claim below carries its pass-1 source). Mirrors the house style of
[`../ANALYTICS-SPEC.md`](../ANALYTICS-SPEC.md); cross-references the FX corpus rather than
duplicating it.

> **Naming guardrail.** Method/person/paper provenance (e.g. the monotone-convex forward
> interpolation, the multi-curve framework) appears in this prose and in future doc-comments
> **only** — never in `celnet-rates` identifiers (CLAUDE.md §8). Inputs are purpose-named
> (`CurveBuildSpec`, not a person's name).

---

## 1. The post-LIBOR multi-curve picture (why one curve no longer prices both legs)

The modern rates world is **multi-curve**: a single OIS/RFR curve **discounts**, while separate
per-tenor **projection** curves supply forward fixings. A single curve can no longer price both
legs of a swap consistently — collateralised derivatives discount on the OIS (RFR) curve, and
forward fixings come from a distinct projection curve per index tenor (1M/3M/6M).
(findings §1.1 — [risk.net](https://www.risk.net/insight/risk-management/7957765/calibrating-interest-rate-curves-for-a-new-era),
[arxiv 1006.4767](https://arxiv.org/pdf/1006.4767))

A **`CurveSet`** is therefore the unit of market state, not a single curve:

| Member | Role | Built from |
|---|---|---|
| **Discount curve** | discounts every cashflow on every leg | OIS/RFR instruments (overnight, OIS swaps) under the trade's CSA collateral rate |
| **Projection curve(s)** | supplies the forward for each floating coupon, one per index tenor | per-tenor deposits/futures/swaps + tenor-basis swaps |

For a USD-SOFR-CSA trade the discount curve *is* the SOFR curve and the projection curve is also
SOFR-driven — the **acyclic** case (§5). Cross-currency / tenor-basis introduce co-dependence and
push to the **global solver** (§5.2).

---

## 2. Which instruments build which curve

The calibrating instrument set per curve (findings §1.2, §2.10):

| Curve | Calibrating instruments (short → long) | Notes |
|---|---|---|
| **OIS / discount (e.g. SOFR)** | overnight rate · SOFR/RFR futures (1M/3M, **convexity-adjusted** §6.3) · OIS swaps (fixed vs compounded RFR) · EFFR/SOFR basis (funds curve) | CME has cleared SOFR swaps since Oct 2018 (findings §1.2). |
| **Per-tenor projection** | per-tenor deposits · STIR futures (convexity-adjusted) · IRS at that tenor · **tenor-basis swaps** (e.g. 3M vs 6M) for the spread between projection curves | tenor-basis swaps are *both* calibrators and tradeable products. |
| **Cross-currency** | **cross-currency basis swaps** (constant-notional and MtM/resettable) | the quoted spread sits on the weaker-currency constant-notional leg (findings §2.10). |

SOFR calibration is **materially harder** than the old EFFR/LIBOR bootstrap: daily averaging,
retrospective (in-arrears) payment, geometric compounding, and the need to **splice a historical
realised segment with a projected segment** when calibrating to futures (findings §1.3 —
[Mathema](https://help.mathema.com.cn/latest/docs/fixedincome/sofr_curve),
[Quantifi](https://www.quantifisolutions.com/tackling-interest-rate-curve-construction-complexity/)).
The compounded-RFR accrual mechanics live in [`FI-CONVENTIONS.md`](FI-CONVENTIONS.md) §4.

---

## 3. Curve internal representation (DF / zero / instantaneous-forward)

Discount factors, zero rates, and instantaneous forwards are **interconvertible**; the design
choice is *which space the interpolation acts on* (findings §1.6):

```
DF(t) = exp(−z(t)·t)            zero rate z
f(t)  = −d ln DF / dt           instantaneous forward
```

A `celnet-rates` **`Curve`** is an **immutable, cheaply-cloned snapshot** carrying pillar dates and
the chosen representation, so many scenarios fan out in parallel without re-allocating (findings
§4.4; mirrors the FX surface snapshot discipline and the alloc/lock-free pinned core of
[`../ARCHITECTURE.md`](../ARCHITECTURE.md) §3). Interpolating in **different spaces** (log-DF vs
zero vs forward) yields **materially different forward shapes** — so the representation is a
first-class part of the curve identity, not an implementation detail.

Forward-rate interpolation and DF interpolation are **formally equivalent under a transform**
(findings §1.8 — [arxiv 2005.13890](https://arxiv.org/pdf/2005.13890)): this lets `celnet-rates`
pick the cheapest computational representation for the hot path *without changing the curve*.

---

## 4. Interpolation — a first-class arbitrage decision

Interpolation choice is **not** cosmetic: it determines the forward-rate shape and therefore the
arbitrage and risk-ladder behaviour (findings §1.7). P0 ships **two** schemes, selectable per
curve:

| Scheme | Forward shape | Trade-off | When |
|---|---|---|---|
| **log-linear on log-DF** | piecewise-flat (discontinuous) forwards — a "sawtooth" | simple · local · cheap · arbitrage-free in DF | the **fast hot-path default** (per Q10 default, see below) |
| **monotone-convex on forwards** | positive, mostly-continuous forwards (can still jump in edge cases) | smooth · positive forwards whenever discrete forwards are positive · costlier | the "nice surface" view |

Monotone-convex fits quadratics to estimated instantaneous forwards, guaranteeing positive
forwards whenever the discrete forwards are positive, but can still produce material forward
discontinuities in edge cases (findings §1.7 — Hagan-West provenance, prose only;
[Hagan-West paper](https://www.deriscope.com/docs/Hagan_West_curves_AMF.pdf),
[rnfc note](https://www.rnfc.org/courses/finance/modules/bond-yields-modelling/Monotone_Convex_Interpolation.pdf),
[SciELO review](https://scielo.org.za/scielo.php?script=sci_arttext&pid=S2222-34362013000400003)).

> **Pending Q10 (default selection).** Pass-1 findings §B recommend shipping **both**, default
> **log-linear-DF** for speed/robustness, monotone-convex for the smooth view. `OPEN-QUESTIONS.md`
> Q10's *seed* default is the opposite (monotone-convex default). The default flag is a one-line
> config switch either way; **the operator must confirm Q10** before the shipping default is
> frozen. This spec is written so that flipping the default touches only the `CurveBuildSpec`
> default, not the curve algebra.

### 4.1 Turn-of-year / central-bank meeting-date jumps

Year-end funding spikes and central-bank-meeting step changes are modelled as **deliberate jumps
in the instantaneous forward**, not smoothed away — otherwise the curve misprices dated futures/OIS
spanning those dates (findings §1.9 — charter §4.1, corroborated by
[risk.net](https://www.risk.net/insight/risk-management/7957765/calibrating-interest-rate-curves-for-a-new-era)).
The `curve/jumps/` module imposes these as explicit forward steps layered onto the interpolated
forward, independent of the interpolation scheme.

---

## 5. Calibration — sequential bootstrap (acyclic) vs global solver (cyclic)

Both bootstrap and global calibration are valid; the choice trades **locality vs smoothness**, and
is **forced** by the instrument dependency graph (findings §1.4, §1.5):

### 5.1 Sequential bootstrap — the acyclic default

When the discount/projection dependency graph is **acyclic** (the USD-SOFR self-discounting case:
SOFR-discount + SOFR-projection), the curve is built **instrument-by-instrument**: each pillar is
solved so its instrument reprices exactly, marching short → long. This is `O(n)` one-dimensional
solves, local, and robust (findings §1.4, §1.5, §4.2 —
[QuantLib bootstrapping guide](https://www.quantlibguide.com/Curve%20bootstrapping.html),
[BlueGamma](https://www.bluegamma.io/documentation/methodology/how-to-bootstrap-the-yield-curve)).

### 5.2 Global solver — the cyclic / basis / XCCY fallback

When discount and projection curves are **co-dependent** — basis swaps reference one curve to price
the other, or the system is over/under-determined — a **single global solver across both curves** is
required; sequential bootstrap is exact *only* when the graph is acyclic (findings §1.4). The global
build minimises pricing error across all instruments under a smoothness/spline penalty — one
`n`-dimensional nonlinear least-squares solve (findings §1.5, §4.2).

`celnet-rates` therefore exposes **both** build modes behind one `build/` module: bootstrap is the
acyclic fast path, the global solver is the general fallback the build planner selects when it
detects a cyclic graph (basis/XCCY).

### 5.3 Root-finding numerics

- **Bootstrap pillar solve** — 1-D **Newton** (with analytic/AD derivative where available) falling
  back to **Brent** for robustness (findings §4.2).
- **Global solve** — multi-dimensional **Levenberg-Marquardt / Gauss-Newton** least-squares over all
  instruments with a smoothness penalty (findings §4.2).
- **Exact pillar Jacobians** via AD/dual numbers give fast, exact key-rate risk (findings §4.2). The
  reference engines (rateslib/ORE) do this, but they are **reference reading / oracles only** — never
  dependencies (see [`FI-VERIFICATION-CONTRACT.md`](FI-VERIFICATION-CONTRACT.md) and findings §5).

P0 is **deterministic curve math**: discounted-cashflow + root-find (bootstrap) + bump (risk). The
*only* stochastic input in P0 is the STIR-futures convexity adjustment (§6.3) (findings §4.1).

---

## 6. Discounting & collateral (OIS/CSA) — and the v1 simplification

### 6.1 CSA / collateral discounting

Discount on the curve of the **collateral rate actually paid**. Proper collateralisation acts
dominantly through the discount factors; cross-currency trades with notional exchange are especially
sensitive (findings §1.10 — [FSA note](https://www.fsa.go.jp/frtc/nenpou/2009/07-1.pdf),
[arxiv 1703.00923](https://arxiv.org/pdf/1703.00923),
[arxiv 1101.5849](https://arxiv.org/pdf/1101.5849)).

### 6.2 Multi-currency CSA & cheapest-to-deliver collateral (deferred)

A CSA allowing multiple eligible collateral currencies gives the poster an **option to deliver the
cheapest collateral**, raising the effective discount curve (findings §1.10). This CTD-collateral
optionality is a **later complexity step** — not in v1.

### 6.3 STIR-futures convexity adjustment

STIR futures need a **convexity adjustment** vs the FRA-implied forward (daily margining ⇒ futures
rate > forward rate); this is required for an arbitrage-free short-end build (findings §2.9, §4.1).
The magnitude is a vol-model input.

> **Pending Q11.** P0 uses a **deterministic placeholder** (a Ho-Lee/Hull-White-style closed-form
> adjustment — provenance prose only), flagged, with the term-structure-consistent adjustment
> deferred to `celnet-rates-vol` (findings §B Q11). `OPEN-QUESTIONS.md` Q11's seed default is the
> opposite (*defer / zero-adjustment placeholder*). **Operator must confirm Q11** — a non-zero
> placeholder vs a flagged-zero placeholder changes the short-end curve.

### 6.4 The v1 single-OIS-curve simplification

For USD-SOFR-CSA trades, **SOFR-discount + SOFR-projection is acyclic and bootstrappable** without a
cross-currency-basis solve; multi-CSA / CTD-collateral is a later step (findings §1.11, matching
`OPEN-QUESTIONS.md` **Q4 default**: single OIS-discount curve in v1, multi-CSA later). This is the
shipping v1 posture.

---

## 7. P0 scope summary (what this spec commissions)

- **`CurveSet`**: one discount curve + per-tenor projection curve(s), immutable & cheaply cloned.
- **Representations**: DF / zero / instantaneous-forward, interconvertible; interpolation acts on the
  chosen space.
- **Interpolation**: log-linear-on-log-DF **and** monotone-convex-on-forwards, per-curve selectable
  (default pending Q10); explicit turn/meeting forward jumps.
- **Calibration**: sequential bootstrap (acyclic) + global LM solver (cyclic/basis/XCCY); Newton/Brent
  + LM root-finding; AD pillar Jacobians for key-rate risk.
- **Discounting**: single OIS-discount curve v1 (Q4); multi-CSA/CTD-collateral deferred.
- **Convexity**: deterministic STIR placeholder (pending Q11), vol-model-driven later.

**Out of P0 (flagged):** swaption/cap vol, SABR/Bachelier/Hull-White, credit/CDS, inflation, full
multi-CSA — see [`FI-ARCHITECTURE.md`](FI-ARCHITECTURE.md) §"later: `celnet-rates-vol`" and findings §C.

**Oracle:** every curve number is validated against **QuantLib (primary) + ORE (risk/FRTB)** plus an
engine-agnostic structural identity — see [`FI-VERIFICATION-CONTRACT.md`](FI-VERIFICATION-CONTRACT.md).

---

### Sources

All sources are the pass-1 citations in
[`../_research/fixed-income-findings.md`](../_research/fixed-income-findings.md) §1/§2/§4 and its
consolidated source list; the load-bearing ones are inlined above.
