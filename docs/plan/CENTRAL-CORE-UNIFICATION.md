# Central Best-Practice Pricing + Risk Core — unify every asset class on one contract

**Status:** DESIGN (design-first; no code until operator confirms). Branch `arch/central-core`.
**Mission (operator, 2026-07-01):** *"refactor all FI code if needed to bring it into a core
central best practice, OR evolve a central best practice and migrate all asset classes on it."*
**Non-goal (explicitly rejected):** forcing linear FI through the options `Underlying` enum.
Linear FI stays linear; the unification is a *common contract*, not a common representation.

Synthesised from two full lodestar architecture maps (options paradigm + FI paradigm) of
`origin/main@9d5642d`.

---

## 1. The problem — two paradigms, partial overlap

Today the platform prices + risks **two ways**:

| Concern | Options (FX/Metal/Equity/Commodity/Crypto) | FI (rates/bonds) |
|---|---|---|
| RPC | `Price(PriceRequest)` | `PriceRates(RatesPriceRequest)` |
| Dispatch | `price_instrument` → `is_cross_asset` router → `ProductEngine` (FX) / `price_cross_asset` (x-asset) | `price_rates` → hand `Instrument::Ois` match |
| Pricing seam | `CarryPricer` trait + unified `gbsm_carry` kernel | bespoke `price_ois` / `celnet-bond` DCF (not server-wired) |
| Market | live `SurfaceBook` snapshot + `MarketContext` | `CurveSet` supplied per-request (pure fn) |
| Risk | `CarryGreeks`→`PositionRisk`→`NodeAggregate` (`celnet-risk-cube`; additive Greeks + non-additive VaR/FRTB) | `PV/PV01/DV01/key-rate` (`RatesFleetReducer`; purely additive; **bypasses** the risk cube) |
| Extensibility | pluggable `ProductEngine` + plugin-host | fixed proto `oneof` |
| Instrument id | `Underlying(pair) + product` | `RatesInstrument` oneof (no `Underlying`) |

This is the "two systems" the mission targets: duplicate dispatch, duplicate risk aggregation,
duplicate RPC/API surface, duplicate extensibility story.

## 2. The key finding — the central contract is already *latent*

The two paradigms **already converge** on a large shared substrate. The unification is mostly
*formalising + re-seating* what exists, not inventing:

- **`DiscountCurve` trait** (`celnet-types`) — implemented by BOTH `Carry` (flat, degenerate
  one-pillar, ADR-0010) AND `celnet_rates::Curve` (bootstrapped term structure). The discounting
  seam is already one type.
- **`CarryPricer` / `gbsm_carry` / `CurveCarry` / `curve_carry_price`** — a forward-space pricing
  kernel that is *already asset-class-agnostic* (`F = S·e^{b·t}`, `df` from a `DiscountCurve`),
  with zero `Underlying`/`Carry` matches. FI linear products are `curve_carry` with strike→0 /
  a cashflow sum over the same curve.
- **`PricingEdge`** hosts both `price` and `price_rates`; **`RiskEdge`** hosts both
  `AggregateRisk` and `AggregateRatesRisk` — same readiness gate, auth, `resolve_caller`.
- **`celnet-router`** HRW/`PartitionMap`, **`celnet-risk-fleet`** `FleetTopology`,
  **`celnet_risk_cube::{EntityId,BookId}`** — shared by both fan-outs; only the partition-key
  fn differs (`(entity,pair)` vs `(entity,ccy)`).
- Both risk paths are **additive at the linear layer** (`NetGreeks`+`VegaLadder` sum;
  rate ladders sum bit-exactly) — compatible aggregation semantics.

The divergence is concentrated in **three seams**, and each has a clean unification:

1. **Dispatch/extensibility** — options has `ProductEngine`; FI has a hand match. → generalise
   `ProductEngine` to a central `PricingEngine` that every asset class (incl. FI) registers with.
2. **Market resolution** — options snapshots a vol surface; FI bootstraps a curve. → a
   `MarketResolver` seam whose output is a `ResolvedMarket` exposing `DiscountCurve` (+ optional
   vol surface). Options = surface path; FI = curve-bootstrap path; both yield the same handle.
3. **Risk representation** — options emits Greeks; FI emits rate ladders. → one
   **tagged `RiskMeasure`** in one cube: additive measures (Greeks, rate ladders) sum; the
   non-additive layer (VaR/ES/FRTB) re-derives over the *union* of positions.

## 3. The central contract

Three traits + one risk model, in `celnet-core` / `celnet-risk-cube` (the existing homes):

```rust
// (1) The universal pricing seam — every product family (options AND FI) implements it.
//     Generalises the existing CarryPricer + ProductEngine; the kernel already exists.
pub trait Priceable {
    /// The resolved-market handle this product needs (curve + optional vol surface).
    fn price(&self, mkt: &ResolvedMarket, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError>;
    /// The risk sensitivities in the unified tagged form.
    fn risk(&self, mkt: &ResolvedMarket, ctx: &EngineCtx<'_>) -> Result<RiskMeasure, PriceError>;
}

// (2) The universal market-resolution seam — abstracts "get me the market this needs".
//     Options: snapshot SurfaceBook (resolve_pinned_vol). FI: bootstrap the CurveSet.
pub trait MarketResolver {
    fn resolve(&self, req: &MarketRequest) -> Result<ResolvedMarket, MarketError>;
}
/// What every pricer consumes. Discounting is the already-shared DiscountCurve seam.
pub struct ResolvedMarket<'a> {
    pub discount: &'a dyn DiscountCurve,        // Carry (flat) | Curve (bootstrapped)
    pub foreign:  Option<&'a dyn DiscountCurve>,// FX two-curve / cross-currency
    pub vol:      Option<&'a VolSurface>,       // None for linear FI
    pub spot:     Option<f64>,                  // None for pure-rates
}

// (3) The unified, tagged risk measure — one cube for every asset class.
pub enum RiskMeasure {
    /// Options: the 13-member Greeks + vega ladder (nonlinear; VaR/FRTB re-derive).
    OptionGreeks(CarryGreeks),
    /// FI/rates: PV + parallel + key-rate ladder (linear; purely additive).
    RateLadder(RatesRisk),
    // future: CreditSpread(CR01…), etc. — additive tags extend freely.
}
```

**Dispatch** becomes one `PricingEngine::price(instrument, resolver, ctx)` that (a) calls the
right `MarketResolver` for the instrument's market kind, (b) dispatches to the instrument's
`Priceable`. Options vanillas/exotics and FI OIS/bonds are all `Priceable` registrants — the
`price_cross_asset` match and the `price_ois` match both dissolve into the registry.

**Risk cube** ingests `RiskMeasure` for every position regardless of asset class. The additive
roll-up sums `OptionGreeks.net` and `RateLadder` side-by-side (a portfolio is one cube with
Greeks *and* DV01 buckets); the non-additive VaR/ES/FRTB layer re-derives over the union. FI
contributes **additively and bit-exactly** — the `RatesFleetReducer`'s linearity is preserved as
the linear tag, not lost.

## 4. Why this is sound (the design invariants)

- **Linear FI stays linear.** FI is a `RateLadder` tag, not an `Underlying` variant, not forced
  through `gbsm_carry`. A bond is a cashflow sum over `ResolvedMarket.discount`; its risk is the
  additive ladder. No nonlinear machinery is imposed on it.
- **Hot-core embargo preserved (ADR-0016).** `ResolvedMarket` is a **borrowed, request/batch-tier**
  handle (like `CurveCarry` today) — it never enters `MarketState`. The pinned FX streaming core
  stays flat-`f64`; `Carry` stays `Copy`. FI/curve pricing is request-tier by nature. The embargo
  test extends to assert no `ResolvedMarket`/`Arc<Curve>` reaches the hot path.
- **Byte-identity per migration step.** Re-seating options behind `Priceable`/`PricingEngine` is a
  pure indirection change over the *same* `gbsm_carry` calls → `to_bits`-identical (proven the
  same way the gBSM-kernel unification was). FI conformance reuses the *same* `ois_risk`/curve math
  behind the trait → identical numbers.
- **Perf.** The seam is `&dyn` at the request tier only (already true for `CurveCarry`/exotics);
  the zero-alloc hot streaming path is untouched. Dispatch stays static where it is today
  (`ProductEngine` unit structs) — the registry is compile-time.
- **Reduces replication (the actual goal).** One RPC contract, one dispatch, one market-resolution
  seam, one risk cube, one extensibility story — measured by deleting `price_rates`'s bespoke
  dispatch + `RatesFleetReducer`'s separate path once folded into the unified cube.

## 5. Migration plan — incremental, gated, coordinated

Each phase: bounded lane → scoped gate → **full t2** → land. Byte-identity + ≤1e-12 vs independent
oracle every step. Coordinate with the **active FI session** (they own `celnet-bond/credit/rates`
+ the FI remnant) — do the trait/core work first, hand them the conformance shim, or sequence.

- **Phase A — formalise the contract, re-seat OPTIONS (byte-identical).** Define `Priceable`,
  `MarketResolver`, `ResolvedMarket`, `RiskMeasure` in `celnet-core`/`celnet-risk-cube`. Make the
  existing options `ProductEngine`s + `price_cross_asset` leaves implement `Priceable`; wrap the
  `SurfaceBook` snapshot as a `MarketResolver`. **Zero numeric change** (`to_bits`-identical over
  the whole options oracle + frozen-pin suite). This proves the contract on the paradigm that
  already fits.
- **Phase B — make FI conform.** `celnet-rates` OIS + `celnet-bond` implement `Priceable`
  (curve-bootstrap `MarketResolver`); FI risk emits `RiskMeasure::RateLadder`; wire `celnet-bond`
  to a server engine (it is currently unwired). Same `ois_risk`/DCF math behind the trait →
  identical numbers vs today's `PriceRates`.
- **Phase C — unify the surface.** One `PricingEngine` dispatch; fold FI into the unified risk cube
  (retire the separate `RatesFleetReducer` path once the cube reproduces it bit-exactly); one
  pricing RPC (or a clean asset-class-tagged dispatch preserving both wire shapes during cutover —
  no versioning, one current contract). Retire the bespoke `price_rates`/`price_cross_asset`
  dispatch. Delete the dead duplication.
- **Phase D — extensibility parity.** FI product families become pluggable `Priceable` registrants
  (credit leaf, new rates instruments) exactly like options product engines; plugin-host reaches FI.

## 6. Risks + open questions (for critique + operator)

- **Coordination with the live FI session** — Phases B/C touch their active crates. Sequence:
  land A (options-only, no FI touch) first; coordinate B/C, or provide the trait and let them
  implement conformance. Do NOT clobber their in-flight credit/leaf-registration work.
- **The unified RPC (Phase C)** — is one `Price` RPC that tags asset class the right end-state, or
  keep `Price`/`PriceRates` as thin facades over the one engine? (Leaning: one engine, keep both
  wire entry points during cutover, then converge — no mixed-version window.)
- **Risk-cube linear/nonlinear coexistence** — confirm the additive roll-up cleanly carries both
  tags and the non-additive layer treats a linear `RateLadder` as zero-curvature (it does: no
  optionality → no VaR convexity term).
- **ADR numbering** — this is ADR-0017; the pre-existing 2×ADR-0012 / 2×ADR-0013 collisions on
  main (FI-leaf vs gBSM-kernel; credit vs single-front-end) need a separate cross-session renumber.

## 7. Recommendation

The central contract is **sound and largely latent** — the seams (`DiscountCurve`, `CarryPricer`,
`CurveCarry`, shared edges, additive risk) already exist; this formalises them into one
`Priceable`/`MarketResolver`/`RiskMeasure` contract and migrates every class onto it, linear FI
kept linear. **Phase A is byte-identical and FI-untouching** — the safe, high-confidence first
step that proves the contract. Recommend: land Phase A, then coordinate B–D with the FI session.
