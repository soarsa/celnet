# FI Credit Pricing Engine (`celnet-credit`) — Design

- **Status:** Proposed design (2026-07-01). **Design only — no credit code yet.**
- **Relates to:** `docs/FI-PRICING-ENGINE-DESIGN.md` (the cash-bond/credit engine design
  this refines for the credit leg), ADR-0018 (fixed income as a new asset-class leaf),
  ADR-0010 (FI rates on the carry seam), ADR-0008 (multi-asset carry / asset-class routing).
- **Build home when scheduled:** on the FI feature line (`feature/fi-reference-data`) for the
  pure analytics leaf; the *contract/registration* step targets `main`'s pricing core exactly
  as ADR-0018 prescribes for `celnet-bond`.
- **Reuses:** `celnet-rates::Curve` (the risk-free discount substrate), `celnet-bond` (the
  risk-free cashflow/DCF machinery), `celnet-calendar` (schedules/day-counts), the risk cube
  (CR01 as a new dimension, per ADR-0018 §3), and the shipped RFQ / FIX / `Book` / limits /
  entitlements — all unchanged.

## 1. Purpose

A full **credit** market-making capability layered on the cash-bond leaf: build issuer/name
**credit (survival) curves**, price **credit-risky bonds** and **single-name credit default
swaps (CDS)**, and produce credit risk — **CR01** (credit-spread DV01) and **JTD**
(jump-to-default) — that rolls up firm-wide alongside rates/FX in the one risk cube. It feeds
the same RFQ→Quote→Order→Fill lifecycle and the auto-hedge loop the FI pricing design defines,
so a credit name quotes and books through the identical paths every other asset class uses —
never a forked engine.

Vendor-neutral naming (guardrail #8): all identifiers are purpose-named — `SurvivalCurve`,
`CreditDefaultSwap`, `credit_risky_price`, `par_spread`, `cr01`, `jump_to_default`. Method
provenance (the standard-model hazard-rate bootstrap; the reduced-form survival model) appears
in **doc comments only**, never in API names. No index/product names (no `CDX`/`iTraxx`) in
identifiers — those are reference-data symbols, not code identifiers.

## 2. What already exists (reuse, don't rebuild)

- **`celnet-rates::Curve`** — `discount_factor(t)` / `zero_rate(t)`: the risk-free discount
  substrate credit prices *on top of*. Credit adds a second, multiplicative survival factor;
  it does not re-implement discounting.
- **`celnet-bond`** — `Bond`, its `CashflowSchedule`, `dirty_price`/`price_from_curve`,
  DCF/DV01/duration. A credit-risky bond is a `Bond` whose cashflows are additionally weighted
  by survival probability plus a recovery-on-default term — so `celnet-credit` *consumes*
  `celnet-bond`'s schedule and risk-free legs rather than duplicating them.
- **`celnet-calendar`** — CDS premium schedules (quarterly, standard roll), accrual, business-day
  adjustment. Reused wholesale.
- **The risk cube** — already asset-keyed and DV01-carrying (ADR-0010); CR01 enters as a new
  dimension, not a credit-only silo (ADR-0018 §3).
- **RFQ / FIX / `Book` / `celnet-limits` / entitlements** — the dealer-quoting slice already
  shipped; credit rides it unchanged. Quote construction (inventory skew + client-tier spread)
  and the fill→inventory→hedge loop are server services over these, per ADR-0018 §4.

So `celnet-credit` is a **pure numerics leaf** plus the standard contract ripple — mirroring how
`celnet-bond` entered.

## 3. The math core (`celnet-credit`)

The model is the market-standard **reduced-form (hazard-rate) survival model** — the same
family the open ISDA CDS Standard Model uses (cited here as method provenance only).

### 3.1 Survival curve

A piecewise-constant **hazard rate** `λ(t)` (forward default intensity) defines the **survival
probability**

```
S(t) = exp( − ∫₀ᵗ λ(u) du )         (piecewise-constant λ ⇒ S is log-linear between pillars)
```

`SurvivalCurve` holds pillar times + hazard rates and answers `survival(t)` and
`default_density(t) = λ(t)·S(t)`. It is the credit analogue of `celnet-rates::Curve` and is
**time-consistent** with it (same ACT/365F time axis).

### 3.2 CDS pricing (the calibration and quoting instrument)

For a single-name CDS with spread `c`, recovery `R`, on the discount curve `DF` and survival
`S`:

- **Premium (fee) leg** — coupons paid while the name survives, plus accrual-on-default:
  ```
  PV_prem(c) = c · [ Σⱼ DF(tⱼ)·S(tⱼ)·Δⱼ  +  accrual-on-default term ]
  ```
- **Protection (contingent) leg** — `(1−R)` paid at default:
  ```
  PV_prot = (1−R) · ∫ DF(t)·(−dS(t))      (evaluated on a fine time grid; −dS = default density)
  ```
- **Par spread** solves `PV_prem(s) = PV_prot ⇒ s = PV_prot / RiskyAnnuity`, where
  `RiskyAnnuity = Σ DF·S·Δ (+ accrual-on-default)`.
- **Mark-to-market** of an existing contract at coupon `c`: `MtM = PV_prot − c·RiskyAnnuity`
  (sign per protection buyer/seller), plus the **upfront** = MtM − accrued.

### 3.3 Bootstrap (calibration)

`bootstrap_survival_curve(quotes, discount, recovery)` solves the hazard pillars **sequentially
short→long** so each par-CDS quote reprices to zero upfront — the exact reprice-to-par oracle
discipline `celnet-rates` already uses for the discount curve (`brent_root`, 1e-8 spread /
1e-10 PV tolerances). Alternative inputs supported by the same seam: **bond Z-spread /
asset-swap spread** → an implied flat/again-bootstrapped hazard, so a name with liquid bonds but
no CDS still gets a curve.

### 3.4 Credit-risky bond

```
DirtyPrice = Σᵢ CFᵢ · DF(tᵢ) · S(tᵢ)   +   R · Notional · ∫ DF(t)·(−dS(t))
             └ survival-weighted cashflows ┘   └ recovery on default ┘
```

`celnet-bond` supplies `CFᵢ`, `tᵢ`, `DF(tᵢ)`; `celnet-credit` supplies `S(tᵢ)` and the recovery
integral. **Z-spread** and **par-CDS-implied** pricing are the two entry points.

### 3.5 Credit risk

- **CR01 (credit-spread DV01)** — PV change under a **+1bp parallel bump of the credit curve**
  (spread → re-bootstrap → reprice). Analytic where cheap, bump-and-reprice otherwise; validated
  analytic-vs-FD as `celnet-bond` does for DV01.
- **JTD (jump-to-default)** — instantaneous PV change if the name defaults now:
  `JTD = (R·Notional) − DirtyValue` for a long risky position; `(1−R)·Notional` for a bought
  protection leg. A distinct dimension from CR01 (a discontinuity, not a sensitivity).
- **IR DV01** of a credit instrument (sensitivity to the *risk-free* curve) is the **same**
  `celnet-bond`/`celnet-rates` DV01 — so a credit position's rate risk nets with the rates desk
  in the one cube automatically.

## 4. Crate design

```
celnet-credit/                    # pure leaf; no I/O, no server deps
  src/lib.rs
  src/survival.rs    SurvivalCurve { hazard pillars } ; survival(t), default_density(t)
  src/bootstrap.rs   bootstrap_survival_curve(quotes, &Curve, recovery) -> SurvivalCurve
  src/cds.rs         CreditDefaultSwap ; par_spread, mark_to_market, upfront, risky_annuity
  src/bond.rs        credit_risky_price(&Bond, &Curve, &SurvivalCurve, recovery) ; z_spread
  src/risk.rs        CreditRisk { cr01, ir_dv01, jump_to_default, recovery_01 }
  tests/published_vectors.rs
Cargo.toml deps: celnet-types, celnet-rates, celnet-bond, celnet-calendar, time
```

Public API (returns the same `Priced` shape every leaf returns once registered):
`SurvivalCurve::survival`, `bootstrap_survival_curve`, `CreditDefaultSwap::par_spread /
mark_to_market`, `credit_risky_price`, `z_spread`, `credit_risk`.

## 5. Oracle / validation (guardrail #5 — never merely "plausible")

QuantLib is not runnable on this workstation (disclosed in `celnet-bond`'s vectors); the
independent oracles are:

1. **Flat-hazard closed form** — with constant `λ` and flat `r`, `RiskyAnnuity` and the
   protection integral have closed forms (geometric/exponential); cross-check the engine's grid
   evaluation to 1e-8.
2. **Par round-trip** — `bootstrap_survival_curve` then reprice every input CDS ⇒ upfront ≈ 0
   (1e-10 PV); and `par_spread` of a freshly bootstrapped pillar == its input quote.
3. **Credit-triangle identity** — for a flat curve, `par_spread ≈ λ·(1−R)` (the standard
   credit-triangle approximation) to first order; assert within the known second-order bound.
4. **Recovery / no-default limits** — `R=1` ⇒ risky price == risk-free `celnet-bond` price;
   `λ=0` ⇒ `S≡1` ⇒ risky price == risk-free price; `S→0` ⇒ price → recovery PV.
5. **Analytic-vs-FD** CR01 and JTD agreement.
6. **Published single-name CDS example** (a worked flat-curve upfront) as a literal with its
   derivation in-comment.

## 6. Contract & registration (targets `main`, per ADR-0018)

1. Credit `Instrument` arms added to the one canonical contract: `CreditDefaultSwap` and
   `CreditRiskyBond`; the asset-class router gains a credit branch beside FX/metal/equity/
   commodity/crypto/fixed-income.
2. Each arm is a `ProductEngine` registry entry whose engine calls `celnet-credit` and returns
   the standard `Priced`. Adding a credit product = one registry entry (the proven pattern).
3. **CR01 + JTD are new risk-cube dimensions** — credit positions roll up firm-wide alongside
   rates/FX; no credit-only store (extends ADR-0010's projection, honours ADR-0018 §3).
4. Every new RPC gets the standard gRPC + WS-mirror frame; the GUI/Excel gain a credit-curve /
   CDS view and a CR01/JTD column (the standard ripple, paid once). Reference data (issuer,
   seniority, recovery, restructuring clause, standard coupon) rides the existing instrument
   registry.
5. Streaming credit marks + RFQ ride the existing `celnet-fix` rates dialect.

## 7. Build order (each independently gated against its oracle)

1. **`SurvivalCurve` + closed-form survival/default-density** — flat-hazard oracle (§5.1).
2. **CDS legs + `par_spread` + `risky_annuity`** — credit-triangle + limit oracles (§5.3/5.4).
3. **`bootstrap_survival_curve`** — par round-trip oracle (§5.2).
4. **`credit_risky_price` + `z_spread`** (consuming `celnet-bond`) — recovery/no-default limits.
5. **`credit_risk` (CR01 / IR-DV01 / JTD)** — analytic-vs-FD (§5.5).
6. **Contract arms + `ProductEngine` registration + risk-cube CR01/JTD dimension** (targets
   `main`) — parity corpus, byte-identical routing for existing asset classes.
7. **GUI/Excel credit views + client-tier credit spread + auto-hedge of CR01/JTD** — over the
   shipped `Book`/limits/entitlements.

## 8. Open questions

- **Recovery convention:** fixed 40% senior-unsecured default vs. name/seniority-specific from
  reference data vs. recovery as a calibrated/quoted input. Recommend reference-data-driven with
  a 40% fallback, and expose `recovery_01` (sensitivity to the recovery assumption).
- **Accrual-on-default:** full-coupon-at-default vs. the standard mid-period accrual. Recommend
  the standard-model mid-period treatment; make it a curve-config knob.
- **Restructuring clause / standard coupons:** the 100/500 fixed-coupon + upfront convention vs.
  par-spread quoting — support both quoting styles into the same bootstrap.
- **Integration grid:** the protection-leg integral step size (accuracy vs. latency); pick the
  coarsest grid that holds the §5 tolerances and record it, mirroring `celnet-rates`.

## 9. References

- `docs/FI-PRICING-ENGINE-DESIGN.md`, `docs/adr/ADR-0018-fixed-income-as-a-new-asset-class-leaf.md`,
  `docs/adr/ADR-0010-converge-fi-rates-onto-carry-seam.md`.
- `crates/celnet-bond/` (risk-free DCF/risk this leaf composes with), `crates/celnet-rates/src/curve.rs`
  (the discount substrate), `crates/celnet-calendar/`.
- Method provenance (doc-comment only): the reduced-form hazard-rate survival model; the open
  ISDA CDS Standard Model conventions for the premium/protection legs and upfront.
