# Celnet — Fixed-Income Verification Contract (per-product anti-circular oracle plan)

**Status:** P0 spec (first synthesis) · 2026-06-22 · branch `feature/fixedincome`
**Scope:** the per-product oracle + independent-structural-identity plan for the `celnet-rates` P0
products (curve DF, swap PV, par rate, bond price, bond-future CF/CTD, PV01/DV01, FRA), plus the
load-bearing OSS-engine license verdicts that gate the oracle set. Vol/credit/inflation products are
deferred with their own contracts (later passes).
**Synthesised from:** [`../_research/fixed-income-findings.md`](../_research/fixed-income-findings.md)
§5/§6. Mirrors [`../VERIFICATION-CONTRACT.md`](../VERIFICATION-CONTRACT.md) gates **(a)–(g)** and is
**subordinate** to it — the parent contract's anti-circular rule, golden-vector mechanics, and
five-client conformance lint all apply unchanged; this doc specialises them for rates.

> **The rule (inherited, NON-NEGOTIABLE).** Every number reaches a reference by **two independent
> routes**: (a) a genuinely different engine (QuantLib/ORE), AND a structural identity that holds
> **regardless of the engine**. An oracle that re-derives the production algebra is a **circular
> oracle** — it passes while both are wrong (the FRTB `0.75ρ` cautionary tale,
> [`../VERIFICATION-CONTRACT.md`](../VERIFICATION-CONTRACT.md) §(a); the W2
> "don't re-derive `F = S·e^{(r_d−r_f)t}`" lesson, findings §6).

---

## 1. The oracle set — and the load-bearing license verdicts

Pass-1's most consequential guardrail output: **of the four candidate OSS reference engines, only
QuantLib and ORE are actually permissively licensed** (findings §5). The oracle set is therefore:

| Engine | License (verified) | Verdict | Rates use as oracle |
|---|---|---|---|
| **QuantLib** | **Modified BSD (3-clause)**, GPL-compatible, permits commercial use | ✅ **CLEAN — primary golden oracle** | multi-curve bootstrap, OIS/IRS/FRA, basis, bond futures, day-counts, calendars |
| **ORE (Open Source Risk Engine)** | **Modified BSD**, built on QuantLib | ✅ **CLEAN** | risk-cube / **FRTB-sensitivity** + multi-curve scenarios (the closest open analogue to the product) |
| **rateslib** | ⚠️ **source-available NON-commercial — NOT OSS**; commercial use needs a paid licence | ❌ **EXCLUDED** — not a dep, arguably not even an oracle in a commercial pipeline without legal sign-off (pending **Q8**) | none |
| **FinancePy** | **GPL-3.0** (copyleft) | ❌ **EXCLUDED as a dep** (would force GPL); usable only as a disposable out-of-process CLI oracle, never linked — but QuantLib/ORE cover the same ground (pending **Q9**) | none by default |

(findings §5 — [QuantLib license](https://www.quantlib.org/license.shtml),
[ORE FAQ](https://www.opensourcerisk.org/faqs/),
[rateslib license](https://github.com/attack68/rateslib),
[FinancePy license](https://github.com/domokane/FinancePy)).

**Net oracle set for `celnet-rates`: QuantLib (primary) + ORE (risk/FRTB)**, wired through the
existing `celnet-golden` harness, **out-of-process and disposable** (a Python/C++ script in CI/test
tooling), NEVER a Cargo runtime dependency — this keeps the existing `cargo-deny` MIT/Apache/BSD
policy intact ([`../ARCHITECTURE.md`](../ARCHITECTURE.md) §2.3). `rateslib` and `FinancePy` are removed
from the dependency-eligible set.

> **Pending Q8 / Q9.** The seed defaults in `OPEN-QUESTIONS.md` are *exclude rateslib entirely* and
> *FinancePy as out-of-process oracle only*. This contract assumes **QuantLib + ORE suffice** and
> excludes both. **Operator must confirm Q8 (rateslib excluded even as oracle)** and **Q9 (FinancePy
> excluded by default)** before either is touched.

---

## 2. Per-product oracle + independent structural identity

Each headline P0 product gets **BOTH** an engine oracle (QuantLib/ORE) **AND** an engine-agnostic
structural identity (findings §6). The identity is the load-bearing anti-circular check — it holds
regardless of the engine, so a shared algebraic slip cannot hide.

| Product | Independent structural identity (engine-agnostic) | + Engine oracle |
|---|---|---|
| **Curve DF** | `DF(0)=1`; `DF` strictly positive & (for sane curves) monotone-decreasing; **re-pricing the calibrating instruments reproduces their market quotes to tolerance** (the bootstrap fixed-point); `f(t)=−d ln DF/dt > 0` under monotone-convex. | QuantLib `PiecewiseYieldCurve` DFs at the same pillars. |
| **Swap PV** | **Par-swap PV ≈ 0** (build at the curve's own par rate ⇒ PV vanishes); **Receiver + Payer = 0**; **leg additivity** — PV computed two independent ways (cashflow sum vs `annuity·(K − S_par)`). | QuantLib `VanillaSwap` NPV. |
| **Par swap rate** | `S_par = floatPV / annuity`; **independently**, a 1-D root-find that zeroes a freshly-built swap must equal the closed-form `S_par` (two different computations agree). | QuantLib `swap.fairRate()`. |
| **Bond price** | **Bond = Σ discounted cashflows**: dirty = Σ c_i·DF(t_i) + redemption·DF(T); **clean = dirty − accrued**; **price↔yield round-trip** is identity. | QuantLib `FixedRateBond` clean/dirty + yield. |
| **Bond future CF/CTD** | **CF** = clean price of the deliverable at **6% notional yield**, recomputed independently and matched to the **CME published CF to 4dp**; **CTD** = argmax implied-repo across the basket, cross-checked against min net basis (two equivalent rankings agree). | QuantLib `BondForward`/futures **+ CME published CFs as a third anchor** (pending Q13). |
| **PV01 / DV01** | **PV01 = annuity** (analytic) must match a `±1bp` finite-difference bump of the fixed rate (analytic vs numerical agree); **key-rate deltas sum ≈ parallel DV01** (decomposition closes). | QuantLib bucketed sensitivities; ORE for the FRTB-sensitivity cross-check. |
| **FRA** | FRA PV = the corresponding **single-period swap** PV (a FRA is a 1-period swaplet) — a cross-product identity with no shared code path. | QuantLib `ForwardRateAgreement`. |

(All identities: findings §6, sourced from §2.1–§2.8.)

> **Pending Q13 — CME conversion factors as a third anchor.** CME published CFs give a rare
> **engine-independent third anchor** for bond-future CF/CTD, beyond QuantLib + the structural
> identity (findings §B Q13). Seed default: **yes, ingest CME CFs in the golden harness**. Operator to
> confirm.

---

## 3. How this maps onto the parent contract gates (a)–(g)

The parent [`../VERIFICATION-CONTRACT.md`](../VERIFICATION-CONTRACT.md) gates apply unchanged; the
rates specialisation:

- **(a) Independent, model-disjoint oracle** — §1 + §2: QuantLib/ORE engine **and** an identity that
  can *disagree*. Re-derive any standards constant (6% CF yield, ISDA day-count) from the primary
  source text in the test, never from the implementation.
- **(b) Frozen golden table / pinned reference** — curve DF / swap PV / par / bond / CF tables frozen
  under `celnet-golden/` (QuantLib-generated, regenerated only deliberately); CME CFs pinned with
  citation (pending Q13). Closed-form rates math carries a **tight** tolerance; any future MC family
  (vol pass) carries a positive `price_std_error`.
- **(c) Cross-client golden vector** — each new rates `Instrument.product` arm (FRA/OIS/IRS/basis/
  XCCY/STIR/bond-future) gets a `celnet-golden/vectors/<family>.json`; the family set must equal the
  proto oneof arm set exactly, enforced by `tools/check-verification-coverage.mjs` (the existing lint
  — it parses the proto, so the **additive** rates arms are picked up automatically once added).
- **(d) Five-client conformance** — server == SDK == CLI == Excel == GUI against a real edge; a
  parity-matrix row per `(asset-class=rates, product)`. The new GUI Fixed-Income workspace
  (D2, [`FI-ARCHITECTURE.md`](./FI-ARCHITECTURE.md)) and Excel `CELNET.*` rates functions are part of
  this axis.
- **(e) Performance budget** — curve build is cheap (`<ms`); the budgeted concern is the **risk cube**
  (n_instruments × n_pillars × n_curves bump-reprice), which reuses the existing server-owned
  `RiskService` hierarchy and bench gates (findings §4.4; [`../ARCHITECTURE.md`](../ARCHITECTURE.md) §1.2).
- **(f) Mutation / fuzz** — `celnet-rates` is **numeric-core**, so it adds its own mutation-kill-rate
  gate (mirroring `mutants-gate-{vanilla,…}`); any new wire/byte decoder adds a fuzz target.
- **(g) Deploy-bound scope statement** — **no live rates market-data feed is claimed in-repo**
  (curves/fixings are operator-supplied static, pending Q12); the carried honest-boundary statement
  names what is proven here (curve/pricing/risk **compute** against frozen quotes) vs deferred (live
  feed ingestion at deploy) — exactly the FX NDF-feed boundary discipline
  ([`../CONVENTIONS.md`](../CONVENTIONS.md) "No live EM/NDF feed data is claimed").

---

## 4. P0 verification checklist (paste into a rates-product PR)

- [ ] (a) QuantLib/ORE oracle **+** the engine-agnostic identity from §2 — and it can *disagree*.
- [ ] (b) Frozen golden table / pinned reference (CME CF where applicable) with honest tolerance.
- [ ] (c) `celnet-golden/vectors/<family>.json` for the new rates arm; lint green.
- [ ] (d) server == SDK == CLI == Excel == GUI against a real edge; parity-matrix row.
- [ ] (e) Risk-cube bench not regressed (or "curve build not on a budgeted path").
- [ ] (f) `celnet-rates` mutation gate; fuzz target for any new decoder.
- [ ] (g) Honest "no live feed claimed; compute-only against frozen quotes" scope statement.
- [ ] `just verification-coverage` + `just check` green.

---

### Sources

All sources are the pass-1 citations in
[`../_research/fixed-income-findings.md`](../_research/fixed-income-findings.md) §5/§6 (and §2 for the
identity derivations) plus its consolidated source list; the load-bearing ones are inlined above.
