# Term-Structure Unification — implementation plan

**Operationalizes ADR-0010** (converge FI rates onto the carry seam) with a validated,
file:line-anchored critique and a phased, gated build order. Continues ADR-0012 (the gBSM
options kernel unification, landed `4371f17`) into the FI/rates + stochastic-vol half of the
platform. Honours CLAUDE.md guardrails #6/#8/#9/#10/#11 and the ADR-0010 FX-byte-identity
invariant.

## 1. Validated critique — current state (3-agent graph audit, 2026-07-01)

The platform has **≥4 distinct pricing paradigms; only the gBSM options kernel is unified.**
The root architectural gap is a **single missing Rust abstraction: a `DiscountCurve`
term-structure trait.** Everything downstream flows from it. (The GUI TypeScript *already*
has this interface — `gui/src/data/ratesPricing.ts:284` `DiscountCurve` — the Rust core does
not.)

### 1.1 The discount/forward substrate is flat everywhere in options
- `celnet-types::Carry` (`lib.rs:919-934`) = two flat scalars; `discount_df(t)=e^{-r·t}`,
  `forward_factor(t)=e^{b·t}` (`lib.rs:957,963`). No term-structure.
- The gBSM kernel `gbsm_carry_price`/`gbsm_carry_greeks` (`celnet-core/src/carry.rs:357,383`)
  takes raw `(b, r): f64`. **Every tenor gets the same `e^{-r·t}`.**
- Flat carry propagates to exotics (`ExoticInputs.carry_df` `celnet-exotics/src/inputs.rs:114`),
  the vol surface ATM anchor (`MarketContext.carry` `celnet-surface/src/quotes.rs:160`), and
  FX linear (`LinearInputs.carry` `celnet-linear/src/inputs.rs:130`).
- The **only** implicit term-structure in options is a workaround: the MC particle engine
  derives carry from the surface forward `(ln F1 − ln F0)/dt` (`celnet-exotics/src/particle.rs:204-212`).

### 1.2 FI/rates is a complete but fully parallel stack
- `celnet-rates` depends on **`celnet-types` + `celnet-calendar` only — NOT `celnet-core`**
  (its own de-facto graph cluster #163). It owns the real term structure:
  `Curve{nodes, knot_fwds, scheme}` (`curve.rs:86`) with `discount_factor(t)` (`curve.rs:318`),
  bootstrapped via `bootstrap_ois` (Brent per pillar) / `bootstrap_futures_strip` / turns.
- Products (all curve-only, no core dep): OIS, FRA, vanilla IRS, cash bond (pv/z-spread/
  asset-swap), STIR futures (the only vol op: convexity `0.5σ²T1T2`). No swaptions/caps.
- Risk = bump-and-reprice key-rate DV01 ladders (`ois_risk`/`fra_risk`/`swap_risk`), which
  **never merge** with the options greeks cube.

### 1.3 The parallel-vocabulary tax (every layer duplicated)
| Layer | Options core | FI/rates | Root defect (file:line) |
|---|---|---|---|
| Discount substrate | flat `Carry` | term-structure `Curve` | no shared trait; `Carry::discount_df` `types:957` vs `Curve::discount_factor` `curve.rs:318` |
| Contract underlying | `Underlying{Fx,Metal,Equity,Commodity,DigitalAsset}` | keyed by `Ccy` directly | `Underlying` has **no `InterestRate` arm** — `celnet-types/src/lib.rs:410` (**root cascade**) |
| Risk aggregate | `NetGreeks` (11 greeks) | `RatesNodeAggregate` (pv/pv01/dv01/key-rate) | no shared node; `celnet-risk-cube/src/additive.rs:33` vs `celnet-risk-fleet/src/rates.rs:168` |
| FRTB | FX-SBM only | none | no GIRR — `celnet-risk-cube/src/frtb_params.rs:82` |
| Dispatch | `engines::dispatch` 22 arms | `price_rates` 1 arm (OIS) | `celnet-server/src/pricer/engines.rs:1442` vs `rates_pricing.rs:212` |
| Proto | `PricingService.Price`, `AggregateRisk` | `PriceRates`, `AggregateRatesRisk`, `RatesInstrument{ois}` only | `celnet.proto:2992,3035,3961` |
| Streaming | `PriceFanout{underlying}` | none (blocked by missing arm) | `pricefanout.rs:238` |

**Already-built-but-unserved:** `VanillaSwap`/`Fra`/`CashBond` analytics exist in
`celnet-rates` but only OIS is wired to the server (`rates_pricing.rs:231`) — `swap_risk`,
`fra_risk`, `bond_pv`, `z_spread`, `asset_swap_spread` are server-side dead-ends.

### 1.4 Genuine domain differences (DO NOT collapse)
- The **Greek-vector** (δ/γ/ν/θ/…) and the **DV01 key-rate ladder** are different risk
  representations for different products — unify the org-hierarchy + response envelope +
  `Underlying` vocabulary, **not** the risk numbers.
- **Linear cashflow PV** vs **vol-based swaption/cap** payoff engines stay separate — they
  share the curve substrate + risk cube, not the pricer (ADR-0010).

## 2. The design (concrete, extends ADR-0010)

### 2.1 The one new abstraction — `DiscountCurve` in `celnet-core`
```rust
pub trait DiscountCurve {
    fn discount_factor(&self, t: f64) -> f64;         // DF(0,t)
    fn forward_factor(&self, t: f64) -> f64 { 1.0 / self.discount_factor(t) } // growth
}
```
- `celnet-types::Carry` implements it (flat: `discount_factor = e^{-r·t}`) — the **degenerate
  one-pillar curve**.
- `celnet-rates::Curve` implements it (term structure) — the general case. (Needs a
  `celnet-rates → celnet-core` dep, one direction, or the trait in `celnet-types`; decide in
  Phase 0 to keep deps acyclic.)
- FX two-rate carry = **two** degenerate curves (`Carry::FxRates` ≡ `{dom, for_}`), already
  proven equal by `fx_carry_inputs_byte_identical` (`carry.rs:525`) — now exploited, not just
  tested.

### 2.2 Curve-backed carry (the pricing seam)
- Generalize `CarryInputs` (`celnet-core/src/carry.rs:81`) so `forward()`/`discount_df()`
  delegate to a `&dyn DiscountCurve` pair; add `Carry::Curves{dom, for_: Arc<Curve>}`.
- `gbsm_carry_price`/`greeks` gain a curve-aware overload; the flat `(b,r)` path stays for the
  degenerate case → **FX byte-identity preserved by construction** (flat variant untouched).

### 2.3 Contract + risk envelope (unify the hierarchy, not the numbers)
- `Underlying += InterestRate(Ccy)` (`celnet-types/src/lib.rs:410`) → rates get a `FactKey`,
  enter the cube org-hierarchy, and become streamable.
- Risk node carries a tagged measure `Measure = Options(NetGreeks) | Rates(RatesNodeAggregate)`
  (or `NetGreeks` gains pv/pv01/dv01 + a curve-bucketed key-rate ladder) so a rates position
  rolls up in the ONE firm cube; `AggregateRatesRisk` becomes a typed projection of it.
- A shared `RateKey{ccy, tenor_bucket}` sensitivity dimension lets options rho and FI DV01
  net (the FX-options-hedged-with-swaps use case).

### 2.4 Dispatch + wire (one contract)
- Register FI `ProductEngine`s (Fra/Swap/Bond) so `price_instrument` dispatches rates too;
  fold `price_rates` into the registry. Wire the `RatesInstrument` oneof arms + the missing
  bond YTM/g-spread endpoints. `price_rates` currently serves only OIS — the analytics for the
  rest already exist.

## 3. Phased, gated build order (each phase: build → T1 → batched t2 → land; FX byte-identity gate every phase)

- **Phase 0 — the trait.** `DiscountCurve` in core; `Carry` + `Curve` impl it. No behaviour
  change. Gate: full workspace builds; `Carry` flat path byte-identical (`to_bits`).
- **Phase 1 — curve-backed carry + FX 2-curve.** `Carry::Curves`, curve-aware kernel overload,
  `CarryInputs` generalization. Gate: **FX/vanilla/exotic byte-identity to_bits** (flat
  degenerate) + a new curve-vs-flat oracle (term-structure forward = `S·df_for/df_dom`)
  validated against QuantLib ≤1e-12. **This is the ADR-0010 no-regression gate.**
- **Phase 2 — unify the risk cube.** `Underlying::InterestRate`, the tagged risk envelope,
  rates into the cube hierarchy; `AggregateRatesRisk` as a projection. Gate: existing options
  cube byte-identical; rates roll-up matches the standalone `RatesFirmRollup`.
- **Phase 3 — dispatch + contract.** FI ProductEngine arms, `price_rates`→registry, proto
  `RatesInstrument` arms (Fra/Swap/Bond) + bond endpoints, all 5 clients. Gate: full t2 incl.
  gui/excel e2e (the FI desk + workspaces become views over the one contract).
- **Phase 4 — SOTA completeness.** GIRR FRTB for the rates book; STIR convexity sourced from a
  cap/swaption vol surface (not a caller scalar); rates streaming via `PriceFanout`; swaption/
  cap payoff engine (separate pricer, shared curve+cube). Each vs an independent oracle ≤1e-12.

## 4. SOTA grounding
- Multi-curve / OIS-discounting (post-2008 standard): OIS discount curve + per-index projection
  curves (`CurveSet`), FX pricing as `S·df_for/df_dom` — the flat two-rate carry is the
  single-pillar special case. (Cite Bianchetti / Ametrano-Bianchetti multi-curve; Piterbarg
  funding/discounting; QuantLib as the golden oracle per guardrail #7.)
- FRTB SBM GIRR (MAR21) for the unified capital number across options + rates.

## 5. Verification & coordination
- **FX byte-identity** (`to_bits`) is the hard no-regression gate on every phase (ADR-0010
  `cl_c701d7d260f82523`). New curve behaviour validated vs **independent** oracles ≤1e-12
  (QuantLib multi-curve, published multi-curve FX forwards), never self-referential.
- **Shared interface crates** (`celnet-types`, `celnet-core`, `celnet-proto`,
  `celnet-risk-cube`) — stabilize the trait + `Underlying` arm + envelope **first**, then
  coordinate; single-M4 cargo serializes; coordinator owns merges + the t2 gate; disjoint-lane
  discipline (`docs/PARALLEL-SESSIONS.md`). Encode each phase as a `coord/board` manifest task.
