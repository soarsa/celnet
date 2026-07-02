# ADR-0019: Credit (survival curves + CDS + credit-risky bonds) is a leaf on the FI branch

> **Renumbered 2026-07-01:** was ADR-0013; the number collided with ADR-0013 (single sell-side front-end, the canonical/heavily-referenced decision). Moved to the next free number, kept consecutive with its FI sibling ADR-0018 (fixed income). Internal references to the fixed-income ADR now read ADR-0018.

- **Status:** Proposed (2026-07-01)
- **Relates to:** ADR-0018 (fixed income as a new asset-class leaf), ADR-0010 (FI rates onto
  the carry seam), ADR-0008 (multi-asset carry / asset-class routing), ADR-0007 (one
  unversioned contract).
- **Context doc:** `docs/FI-CREDIT-ENGINE-DESIGN.md`.
- **Branch:** drafted on `feature/fi-reference-data`. The analytics leaf builds on the FI
  branch; the contract/registration step targets `main`'s pricing core, exactly as ADR-0018
  prescribes for `celnet-bond`.

## Context

ADR-0018 admitted fixed income as a new asset-class leaf on the cross-asset `ProductEngine`
router, with `celnet-bond` as the first pure analytics leaf. Credit — single-name CDS,
credit-risky bonds, issuer survival curves, and credit risk (CR01 / jump-to-default) — is the
next FI capability. The structural question is the same one ADR-0018 answered for bonds: does
credit price through the existing core, or as a parallel credit service with its own curve store
and risk silo?

The building blocks already exist: `celnet-rates::Curve` is the risk-free discount substrate;
`celnet-bond` owns the cashflow/DCF/DV01 machinery; the risk cube is asset-keyed and DV01-carrying
(ADR-0010); the dealer-quoting slice (RFQ/`Book`/limits/FIX) is shipped. The genuinely new pieces
are the **survival-curve math** and **CR01/JTD** — a discipline, not an infrastructure.

## Decision

**Credit is a pure analytics leaf `celnet-credit` on the existing FI asset-class leaf — not a
parallel credit pricing system, and not bolted onto `celnet-rates`.**

1. `celnet-credit` is a pure numerics crate (deps: `celnet-types`, `celnet-rates`, `celnet-bond`,
   `celnet-calendar`, `time`) implementing the reduced-form hazard-rate survival model:
   `SurvivalCurve`, `bootstrap_survival_curve`, `CreditDefaultSwap` (par spread / MtM / upfront),
   `credit_risky_price` / `z_spread`, and `credit_risk` (CR01 / IR-DV01 / JTD). It **consumes**
   `celnet-rates` (discounting) and `celnet-bond` (risk-free legs); it duplicates neither.
2. Credit `Instrument` arms (`CreditDefaultSwap`, `CreditRiskyBond`) are added to the one
   canonical contract (ADR-0007); the asset-class router gains a credit branch and each product
   is one `ProductEngine` registry entry returning the standard `Priced`.
3. **CR01 and JTD are new dimensions of the existing risk cube**, not a credit-only store —
   credit positions roll up firm-wide alongside rates/FX, and a credit instrument's IR DV01 nets
   with the rates desk automatically (extends ADR-0010's projection, honours ADR-0018 §3).
4. Quote construction (client-tier credit spread + inventory skew) and the fill→inventory→hedge
   loop for CR01/JTD are server services over the shipped `Book` / `celnet-limits` / entitlements
   — no new authz model, no second risk store.
5. Every new RPC gets the standard gRPC + WS-mirror frame; outbound streaming/RFQ rides the
   existing `celnet-fix` rates dialect.

## Consequences

**Positive**
- One contract, one hot path, one risk cube — credit greeks and firm-wide roll-up stay consistent
  with every other asset class (honours ADR-0007/0008/0010/0012).
- Maximal reuse: discount curve, cashflow/DCF machinery, RFQ workflow, FIX, streaming, `Book`,
  limits, entitlements are unchanged. Adding a credit product mirrors the bond/equity pattern.
- The analytics leaf is validated against independent oracles (flat-hazard closed form, par
  round-trip, credit-triangle, recovery/no-default limits, analytic-vs-FD) before any wiring —
  same discipline as `celnet-bond`.

**Negative / costs**
- The `Instrument`/`Priced` contract grows credit arms; the WS mirror, SDK, and GUI/Excel gain
  credit-curve/CDS views and a CR01/JTD column (the standard ripple, paid once).
- The risk cube must accommodate CR01 + JTD without regressing rates/FX projections; JTD is a
  discontinuity dimension, distinct from the smooth CR01 sensitivity.
- Convention surface (recovery, accrual-on-default, restructuring, standard coupons) must be
  reference-data-driven; getting these wrong misprices — hence the open questions in the design.

**Neutral**
- Latency: survival math is cheap (µs per instrument on a fine time grid) and rides the existing
  zero-alloc core; no new infra (no JVM/off-heap ceremony — consistent with ADR-0018).

## Alternatives considered

- **Parallel credit pricing service.** Rejected: duplicates the contract, the risk store, and
  the hot path; produces a second greeks dialect and breaks firm-wide roll-up. Contradicts
  ADR-0007/0008/0012.
- **Bolt credit onto `celnet-rates`.** Rejected as the crate home: rates is the risk-free
  numerics substrate (the discount curve), not the credit model. `celnet-credit` *consumes*
  `celnet-rates` but stays a separate leaf so the dependency points one way and the survival
  model does not pollute the discount curve.
- **Bolt credit onto `celnet-bond`.** Rejected: a CDS is not a bond, and a survival curve is not
  bond risk. `celnet-credit` consumes `celnet-bond`'s risk-free legs but owns the survival model
  and CDS instrument itself — keeping each leaf single-purpose.

## Supporting verified claims

To be authored against the graph on promotion (not yet verified): (a) `celnet-rates::Curve` is a
pure consumable discount substrate; (b) `celnet-bond` exposes the cashflow schedule + DV01 the
credit leaf composes with; (c) the risk-cube projection is asset-keyed and extensible to a CR01/JTD
dimension. Author via `knowledge_put` with `kind: spec:satisfies` against the committed credit
acceptance corpus when the credit milestone starts.
