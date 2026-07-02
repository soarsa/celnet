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
See §8 for the corrections that revise this.

## 8. Critique incorporated — celnet-verifier verdict: **SOUND-WITH-FIXES**

Adversarial verification confirmed the "already latent" thesis is TRUE and correctly cited (the
`DiscountCurve` dual-impl, the asset-class-agnostic carry kernel, the shared `PricingEdge`/`RiskEdge`
/fleet, the additive rate ladders, and `celnet-bond` being unwired all verify against real code).
But it corrected three over-claims — these SUPERSEDE the optimistic framing above:

- **F1 (HIGH) — the RISK unification is not free (this is the real decision).** §3's original "the
  non-additive layer re-derives over the union, `RateLadder` is zero-curvature so it's fine" is the
  **wrong axis**. The existing non-additive VaR/ES/FRTB path (`cube.rs:272,320`) bumps **spot/vol**
  scenarios and re-prices via `CarryPricer`/`ExoticLegPricer`; a linear FI ladder is neither, and it
  *does* carry first-order VaR under **rate** shocks the cube cannot compute (FRTB is spot-curvature
  only — no GIRR bucket). ⇒ full risk unification is **R-a** (build a curve-repricer + rate-scenario
  /GIRR generator — genuinely NEW machinery) or **R-b** (keep FI tail-risk in its own additive+rate
  path, co-located behind the unified façade). The **additive** cube unifies now regardless; §3 is
  corrected to say so. This is the central open decision for the operator.
- **F2 (HIGH) — Phase A is NOT "pure indirection."** `price_instrument` is a multi-guard cascade
  (perpetual special-case, LSV refusal, FX two-rate guard, an **LSV booking-model selector → a
  different engine**, and a **plugin-host runtime `dispatch_live` registry**), and `price_cross_asset`
  is a hand-match over **five leaf crates** + sub-arms + a delta-key refusal — none engine-shaped like
  the FX `ProductEngine` unit structs. ⇒ Phase A is re-scoped: **(i)** keep the entire guard/selector
  cascade verbatim as pre-dispatch, **(ii)** re-seat `Priceable` at the **leaf**, not the dispatch,
  **(iii)** budget reifying each cross-asset match arm into an engine type, **(iv)** preserve
  plugin-host dynamic dispatch and the `ExoticLegPricer` VaR seam (do not collapse both pricing seams
  into one and silently drop exotics from VaR). Byte-identity is still achievable, but it is a
  leaf-level re-seat, not a free wrap.
- **F3 (MED) — Phase C deletes the dispatch/RPC surface, not the risk math.** The additive rate-ladder
  logic *relocates* into the cube (must reproduce `RatesFleetReducer::fan_in_additive` bit-exactly),
  and under R-a the non-additive FI-VaR is *new* code. So Phase C is net-additive in the risk layer;
  the genuine deletion is the duplicate dispatch/RPC/edge/fan-out wiring. §5 Phase C is corrected.
- **F4 (MED) — `MarketResolver` is request/batch-tier only.** Options read a live hot `MarketState`
  via `ArcSwap`; `MarketResolver::resolve` models per-request resolution (fits FI, request-tier view
  for options — it does NOT cover the streaming hot path, which keeps its own market). `ResolvedMarket`
  must also carry `ConventionSet` + the delta-key solver (threaded today as `conv: &ConventionSet`).
- **F5 (LOW-MED) — DELIVER the ADR-0016 embargo test in Phase A.** The `trybuild`/reflection test
  asserting no curve handle reaches `MarketState` is an **unbuilt** future deliverable — Phase A must
  build it, not cite it as an existing guardrail.
- **F6 (LOW, ties to the ADR-verification mandate) — the ADR corpus is broken.** `docs/adr/` stops at
  0013 (with 2×0012 + 2×0013 collisions), yet **ADR-0014/0015/0016 are cited 32× in code with NO
  files** (ADR-0016 is cited as the hot-core embargo authority but does not exist as a document), and
  ADR-0017 is already referenced 2×. ⇒ this design's ADR is **renumbered pending the ADR-corpus
  reconciliation** (the parallel knowledge-completeness pass), and the missing ADR-0014..0016 must be
  written + made lodestar-governing (manage_adr Decision→Deliverable + anchored claims) — not left as
  dangling citations. This is exactly the "ADRs should be lodestar-verified" gap.

**Revised recommendation.** The PRICING unification (`Priceable`/`MarketResolver` + additive risk) is
sound, high-value, and Phase-A-byte-identical after the F2 re-scope — proceed. The full RISK-cube
unification is a genuine fork (R-a new rate-VaR machinery vs R-b co-located façade) that the operator
should choose before Phase C. Do NOT claim "one cube, delete `RatesFleetReducer`" until R-a is
designed as new work. Each phase ships its verified lodestar knowledge + a lodestar-governing ADR.
