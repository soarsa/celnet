---
name: cs01-z-spread-from-live-price
description: CS01 design DECIDED + analytics built (0237e614) — the credit spread is SOLVED from the live price as a z-spread, never stored as reference data. Wire + client integration still to do.
metadata:
  type: project
---

**2026-08-19, operator-approved.** A DV01-hedged corporate displays as flat while carrying
full credit exposure, because the rates hedge removes `∂P/∂y` and nothing removes `∂P/∂z`.
Reporting it needs a credit spread per issue, and none exists in `celnet-refdata` /
`celnet-bond`.

**Decision: derive the spread, never store it.** The z-spread is the single constant added
to every continuously-compounded zero rate that reprices the bond to its observed market
price — the price the book already marks with.

```
solve  P_market = Σ CFₖ · DF(tₖ) · e^(−z·tₖ)   for z
then   CS01     = −∂P/∂z · 1bp                  (analytic, positive)
```

Rejected: a `spread_duration`/`credit_spread` refdata field (no permissibly-licensed source
to populate it — guardrail 7 — and hand-entered figures are stale on entry), and a
rating/sector credit-curve registry (much larger, still needs the curves sourced, and gives
a bucket average rather than an issue-specific number).

**Built (`0237e614`, `crates/celnet-bond/src/spread.rs`):** `z_spread`, `cs01`,
`spread_duration`, plus `price_on_curve_with_spread` / `curve_price_spread_derivative` on
the shared `CashflowSchedule`. Same safeguarded Newton+bisection scheme as `yield_solve`
(P is strictly decreasing in z ⇒ single-rooted). **The bracket floor admits NEGATIVE
spreads** — a bond richer than the risk-free curve is ordinary (on-the-run Treasuries), and
a solver bracketed at zero refuses those prices outright.

Validated against *constructions*, not the engine's own forward function: a flat-curve
shift identity (price off a flat `r+z` curve, solve off flat `r`, must return `z`), the
exact zero-coupon closed form `CS01 = T·P·1bp`, a CENTRAL-difference derivative check (a
one-sided bump differs by the O(h²·convexity) term — ~1e-5 here — which would force a
tolerance loose enough to hide a wrong derivative), and a sloped-curve round trip.

**NOT yet wired — this is the remaining work.** CS01 needs a discount curve, and
`genuine_position_dv01` (`rates_book.rs`) is deliberately curve-free. The natural seam is
`services/rates_risk/aggregate.rs`, whose `fact_from_position` already has the `CurveSet`.
Full integration is a **proto-window change across 5 clients**: add `cs01` to
`RatesRiskFact` + `EntityCell` + the rollup, the proto rollup messages, the hand AND
generated WS codecs + the differential test, then GUI / Excel / SDK / CLI parity per the
uniform-asset-class tenet. T2 gate. Sized like the other proto-window arms, not a wiring job.

Related: [[hedge-rate-cap-lifetime-deadlock]], [[bond-hedge-books-no-offsetting-leg]].
