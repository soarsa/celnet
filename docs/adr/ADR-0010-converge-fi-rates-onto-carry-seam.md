# ADR-0010 — Converge FI rates onto the carry seam as a term-structure

- **Status:** Superseded by ADR-0018 and ADR-0020 (Central Cross-Asset Pricing/Risk Contract).
  The convergence of fixed income rates onto the central contract is realized via
  `celnet-core::contract` (`Priceable`/`MarketResolver`/`RiskMeasure`) and `celnet-bond`.
- **Aligns with:** master-plan items A (carry-seam-to-edge), C (`price_instrument` →
  `ProductEngine`), E (risk-cube `RepriceFn`), F (shared carry→sensitivity mapper).
  See `docs/plan/NEXT-ARCHITECTURE-IMPLEMENTATION.md`.

## Context (grounded in the code)

- The FX-options core hangs off ONE abstraction — a discount/forward *producer*:
  `celnet-types::Carry` (`FxRates{r_dom,r_for}` | `CostOfCarry{r,b}`) with
  `discount_df` / `forward` / `yield_rate` / `discount_rate`, consumed via
  `celnet-core::carry::CarryInputs::{discount_df, forward}`. It is **flat** —
  single-rate, single-expiry.
- The parallel session built `celnet-rates::curve::Curve` (nodes + interpolation;
  `discount_factor` / `zero_rate` / `forward_rate_continuous` / `instantaneous_forward`)
  — a full bootstrapped **term structure** — plus linear products (FRA/IRS/OIS),
  key-rate-DV01 risk (`celnet-rates::risk`, the key-rate ladder), `RatesInstrument` /
  `CurveSet` proto arms, `RiskService.AggregateRatesRisk`, and dedicated GUI rates
  workspaces. It is functionally complete but stands **side-by-side** with the
  carry/risk core.
- **Key insight:** `Carry` is a **degenerate flat term-structure**; the rates `Curve` is
  the **general case**. `Carry::FxRates{r_dom,r_for}` is already two flat discount curves
  producing a forward — i.e. FX carry is a **2-curve special case of multi-curve**. So
  convergence is principled, not forced.

## Decision

Model FI rates into core by converging `celnet-rates` onto the carry seam at **three
layers**, keeping the payoff engines separate:

1. **Term-structure substrate (unify).** Hoist a `DiscountCurve` / `TermStructure` trait
   into `celnet-core` that BOTH `Carry` (flat) and `celnet-rates::Curve` (term structure)
   implement; generalize `CarryInputs` from a flat rate to a curve-backed context; the
   multi-curve `CurveSet` (OIS discount + per-index projection) is the container, with FX
   two-rate carry as the 2-curve special case. (Items A + F.)

2. **Risk (unify into the ONE cube).** Make `RateSensitivities` carry a curve-bucketed
   key-rate-DV01 ladder and admit a rates underlying/dimension so rates positions roll up
   in the firm-wide risk cube alongside FX / equity / crypto; `AggregateRatesRisk` becomes
   a **typed projection** of the same cube, not a silo. (Item E + API-first parity.)

3. **Contract / clients (one Instrument oneof).** Dispatch rates via the `ProductEngine`
   registry (`celnet-server::pricer::price_instrument`, item C) so all five clients consume
   rates through the same contract; the dedicated rates GUI workspaces become **views over
   the one surface**.

### Keep separate — the valuation paradigm (do NOT force one pricer)

Linear rates (cashflow PV over the curve — deterministic, no vol; `celnet-rates` /
`celnet-linear`) vs rates options (swaptions / caps: the forward swap / forward rate is
*produced by the curve*, but priced with normal/Bachelier + SABR vol under the annuity
measure). They **share the curve substrate and the risk cube, NOT the payoff engine.** A
distinct `celnet-rates` crate remains correct: multi-curve bootstrapping, cashflow
schedules / day-count / calendars (via `celnet-calendar`), and curve risk have no
FX-option analog.

## Consequences

- FI-rates "into core" is largely the **payoff of plan items A / C / E / F**; the work is
  the **convergence** (curve trait, one risk cube, registry dispatch), **not** dissolving
  `celnet-rates` into the option core.
- **Invariant to preserve — FX byte-identity.** The flat-carry FX path
  (`Carry::FxRates` → `CarryInputs::discount_df` / `forward` → `VanillaInputs`) must remain
  **bit-identical** (`to_bits` equality) after the curve-trait generalization. The flat
  curve is the single-point degenerate of a term structure; the generalization must not
  perturb the FX two-rate discount/forward by even one ULP. This is the no-regression gate
  on the convergence (continues the ADR-0008 / `carry-seam` byte-identity contract).

## Supporting verified claims (lodestar knowledge layer)

Authored graph-anchored, lifecycle **draft** (proposed direction; promotion to `active`
awaits a cross-family Stage-2 review **and** implementation):

- `cl_f412933641ecc72d` (decision) — Carry is the degenerate flat case; `Curve` is the
  general case; converge onto one `DiscountCurve` trait in `celnet-core`. Anchors: `Carry`,
  `Curve` (+ `discount_factor`/`zero_rate`/`forward_rate_continuous`), `Carry::discount_df`.
- `cl_a2a865886c8bfe90` (decision) — rates risk unifies into the one risk cube via a
  curve-bucketed `RateSensitivities`; `AggregateRatesRisk` is a projection, not a separate
  engine. Anchors: `RateSensitivities`, `AggregateRatesRisk`, `celnet-rates::risk::ladder`,
  `CurveSet`, `RatesInstrument`.
- `cl_c701d7d260f82523` (invariant) — FX byte-identity must survive the carry→curve-trait
  generalization (flat-carry path bit-identical). Anchors: `CarryInputs::discount_df`,
  `CarryInputs::forward`, `Carry`.

## Alternatives rejected

- **Dissolve `celnet-rates` into the option core** — wrong: multi-curve bootstrapping,
  cashflow schedules, and curve risk have no FX-option analog; a distinct crate is correct.
- **One pricer for linear rates and rates options** — wrong: deterministic cashflow PV and
  vol-based swaption/cap pricing are different valuation paradigms; they share the curve
  substrate and risk cube, not the payoff engine.
- **Keep rates a permanent silo (separate risk engine + separate client surface)** —
  rejected: violates API-first parity and the one-contract guardrail; rates positions must
  roll up in the firm-wide cube and dispatch through `price_instrument`.
