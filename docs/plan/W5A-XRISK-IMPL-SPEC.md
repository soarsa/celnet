# W5-A-XRISK — Cross-Asset Risk Normalization + FRTB Bucket Assignment

**Lane:** W5-A-XRISK · **Owns:** `crates/celnet-risk-normalize`, `crates/celnet-risk-cube`
**Depends on (read-only):** `celnet-core` (carry seam), `celnet-types` (`Underlying`/`Carry`/`RateSensitivities`), the W5-B leaves `celnet-equity-vanilla` / `celnet-commodity-vanilla`, `celnet-crypto-vanilla`, `celnet-vanilla` (FX).
**Status of those two crates today:** both already exist and are *FX-only* — `celnet-risk-normalize` re-prices via `celnet_vanilla::{price,greeks,adjoint_greeks,convention_delta}` over `VanillaInputs`/`CcyPair`; `celnet-risk-cube` sums `CanonicalLeaf`, buckets vega, and re-prices positions for VaR/curvature via `celnet_vanilla::price`. W5-A **generalizes them in place** (one unversioned contract; FX stays byte-identical) so the equity/commodity/crypto leaves feed the *same* normalization + cube **through the agnostic seam, with no match-on-`Underlying`/`Carry` in the hot path**.

Guardrails honored: ADR-0008 (carry-as-forward/discount producer; asset-class-agnostic payoff; **no hot-path match-on-Underlying/Carry**), one unversioned contract (no `schema_version`), OSS-only, numerics validated vs an **independent** (non-circular) oracle, vendor-/person-neutral identifiers (method provenance in prose only). The **FRTB 0.75ρ circular-oracle lesson** is honored explicitly in §3.

---

## 0. The core problem and the chosen seam

### 0.1 Why today's code is FX-locked

`celnet-risk-normalize::PositionRisk` carries `pair: CcyPair`, `inputs: VanillaInputs`, `quoted_delta: DeltaConvention`, `premium_style: PremiumStyle`, and `canonicalize_with` calls `celnet_vanilla::{greeks,adjoint_greeks,convention_delta}` directly. The cube's `nonadditive::position_pnl`/`curvature_legs` re-price with `celnet_vanilla::price`. Every other asset-class leaf (`celnet-equity-vanilla` `EquityInputs`/`EquityGreeks`, `celnet-commodity-vanilla` `CommodityInputs`, `celnet-crypto-vanilla` `LinearInputs`/`InverseInputs`) is a **separate input/greek type**, and they all already emit `RateSensitivities::Carry { discount_rho, carry_rho }` while FX emits `RateSensitivities::Fx { rho_dom, rho_for }`.

### 0.2 The seam (already built in `celnet-core::carry`)

- `CarryInputs { spot, strike, vol, t, underlying: Underlying, carry: Carry }` — the asset-agnostic pricing input.
- `CarryGreeks { price, delta_spot, delta_forward, gamma, vega, theta, rates: RateSensitivities, vanna, volga, charm, speed, zomma, color }` — the asset-agnostic Greek strip (mirror of `Greeks`, but with carry-tagged `rates`).
- `trait CarryPricer { fn price(opt, &CarryInputs) -> Result<f64,CarryPriceError>; fn price_greeks(opt, &CarryInputs) -> Result<CarryGreeks,CarryPriceError>; }`.
- `CarryPriceError::{UnsupportedUnderlying, UnsupportedCarry}`.

The seam is the **dispatch boundary**. The whole point of W5-A: the risk layer holds a `&dyn CarryPricer` (or a generic `P: CarryPricer`) and never matches on the asset class. The *leaf* decides whether it can price a `CarryInputs` (and returns a typed error if not). Selecting which leaf is **not** in the hot path: it is a one-time wiring step that builds a dispatcher; the per-position pricing call is a single virtual call through the seam.

### 0.3 Dependency-order summary (full detail in §7)

1. **Leaf seam impls** (in each leaf crate's own dir — *these are the W5-B leaves + crypto + FX; W5-A only specifies the trait-impl shape and a thin in-crate adapter when a leaf does not yet impl `CarryPricer`*). Because W5-A may not edit the leaf crates, §1.2 specifies a **`celnet-risk-normalize`-owned `AssetPricer` dispatcher** that wraps the leaves it is allowed to depend on, implementing `CarryPricer` itself. This keeps the generalization inside the two owned crates.
2. `celnet-risk-normalize`: generalize `PositionRisk` → carry-tagged; route canonicalization through the seam; generalize `CanonicalLeaf`/numeraire to non-FX underlyings.
3. `celnet-risk-cube`: route `position_pnl`/`curvature_legs`/scenario apply through the seam; generalize the fact key's underlying axis.
4. `celnet-risk-cube::frtb`: add **bucket assignment** with FRTB-text-re-derived params (§3).
5. The longhand oracle (§4) and the FX-invariant gate (§5).

---

## 1. `celnet-risk-normalize` — cross-asset canonicalization

### 1.1 Generalize `PositionRisk` (file: `crates/celnet-risk-normalize/src/leaf.rs`)

Replace the FX-locked fields with the carry-tagged vocabulary. **Keep the convention provenance fields** — they remain meaningful for FX and are inert (`None`) for non-FX.

```rust
pub struct PositionRisk {
    pub underlying: Underlying,            // was: pair: CcyPair
    pub option: OptionType,
    pub notional_base: f64,
    pub inputs: CarryInputs,               // was: inputs: VanillaInputs
    /// FX delta-convention provenance; `None` for non-FX asset classes (whose
    /// delta has a single canonical spot definition and no premium-adjusted view).
    pub quoted_delta: Option<DeltaConvention>,
    /// FX premium-style provenance; `None` for non-FX (premium currency is the
    /// numeraire/quote leg and is never paid in the asset leg).
    pub premium_style: Option<PremiumStyle>,
}
```

- Add a back-compat-ergonomic FX constructor `PositionRisk::fx(pair, option, notional_base, vanilla: VanillaInputs, quoted_delta, premium_style)` that lifts `VanillaInputs` → `CarryInputs::new(spot,strike,vol,t, Underlying::Fx(pair), Carry::FxRates{r_dom,r_for})` and wraps the conventions in `Some(..)`. This keeps every existing FX test one-line-changed (`PositionRisk::new(...)` → `PositionRisk::fx(...)`).
- Add `PositionRisk::carry(underlying, option, notional_base, inputs: CarryInputs)` for the non-FX path (conventions `None`).
- Derive helpers: `fn numeraire_ccy(&self) -> Ccy` = `self.underlying.as_ccy_pair().quote` (the premium/quote leg — correct for FX, metals, and any future fiat-quoted underlying; for crypto-inverse see §1.5).

`Underlying::as_ccy_pair()` already exists and projects FX and metals to a `CcyPair`. For equity/commodity/crypto arms **that are not yet variants of `Underlying`**, see §1.6 — the spec adds the needed `Underlying` arms in `celnet-types` *only if absent* (today only `Fx`/`Metal` exist). **This is the single coordinated change to the frozen `celnet-types` interface and must be claimed in the work-stream ledger before editing** (guardrail: shared interface crate, change-with-coordination).

### 1.2 The seam dispatcher `AssetPricer` (NEW file: `crates/celnet-risk-normalize/src/pricer.rs`)

This is the **one place** asset class is resolved, and it is **not** on the per-call hot path of any reducer — reducers receive a `&P: CarryPricer` and call it blindly.

```rust
use celnet_core::carry::{CarryGreeks, CarryInputs, CarryPriceError, CarryPricer};

/// The default cross-asset dispatcher. It owns no state; it implements the agnostic
/// `CarryPricer` seam by delegating to whichever leaf supports the `CarryInputs`.
/// The asset-class resolution happens HERE, once per price call, behind the seam —
/// the risk reducers never see it (ADR-0008 no-hot-path-match: the reducers hold a
/// `&dyn CarryPricer`/`&P` and the *match is the leaf's*, not the aggregation loop's).
pub struct AssetPricer;

impl CarryPricer for AssetPricer {
    fn price(&self, opt, i: &CarryInputs) -> Result<f64, CarryPriceError> { ... }
    fn price_greeks(&self, opt, i: &CarryInputs) -> Result<CarryGreeks, CarryPriceError> { ... }
}
```

**Critical design rule to satisfy "no match-on-underlying in the hot path":** the dispatch table is a `&'static [&'static dyn CarryPricer]` of leaf adapters, tried in order; each leaf returns `Err(UnsupportedUnderlying/UnsupportedCarry)` for inputs it doesn't own, and `AssetPricer` returns the first `Ok`. This is the ADR-0008 pattern: **the leaf owns the rejection**, the dispatcher owns *ordering*, and the reducer owns *nothing* — it just calls `pricer.price_greeks(opt, &ci)`. The match that does exist (inside each leaf adapter) is the leaf's legitimate "is this mine?" guard, which ADR-0008 explicitly permits (see `fx_vanilla_inputs` already doing exactly this in `celnet-core`).

**Leaf adapters** (in `pricer.rs`, one thin struct per leaf the crate is allowed to depend on):

| Adapter | Leaf | Lowering | Lift to `CarryGreeks` |
|---|---|---|---|
| `FxLeaf` | `celnet-vanilla` | `celnet_core::carry::fx_vanilla_inputs` (exists) → `VanillaInputs` | `celnet_core::carry::fx_carry_greeks` (exists) |
| `EquityLeaf` | `celnet-equity-vanilla` | `CarryInputs` → `EquityInputs{spot,strike,vol,t, r: carry.discount_rate(), q: −(carry.carry_rate()−discount_rate()), repo:0}` **iff** `Carry::CostOfCarry` and `Underlying::Equity` | identity field copy; `rates` already `Carry{discount_rho,carry_rho}` |
| `CommodityLeaf` | `celnet-commodity-vanilla` | `CarryInputs` → `CommodityInputs::new(spot,strike,vol,t,carry)` iff `Underlying::Commodity` | identity field copy |
| `CryptoLeaf` | `celnet-crypto-vanilla` (linear arm only for the additive seam) | `CarryInputs` → `LinearInputs` iff `Underlying::Crypto` linear | identity field copy |

Note the equity lowering: `EquityInputs` parameterizes by `(r, q, repo)` and combines them to `b = r − q − repo`. To round-trip a generic `Carry::CostOfCarry { r, b }` losslessly through the equity leaf, set `r := r`, `q := r − b`, `repo := 0` (so `b = r − q − 0 = b`). **Equivalently, do not route generic cost-of-carry through the equity leaf at all** — prefer the commodity leaf, which takes `Carry` directly without re-deriving `q` (cleaner, fewer lossy reparameterizations). Recommended: **`CommodityLeaf` is the canonical generic-`CostOfCarry` pricer** for the additive Greek strip; `EquityLeaf`/`CryptoLeaf` are only selected when the `Underlying` arm names them (because their *risk tagging* / inverse-payoff differs). Make this explicit in `AssetPricer`'s ordering and document it.

### 1.3 Route `canonicalize_with` through the seam (file: `leaf.rs`)

```rust
pub fn canonicalize_with<P: CarryPricer>(pricer: &P, engine: GreekEngine, pos: &PositionRisk)
    -> Result<CanonicalLeaf, CarryPriceError>
```

- `let g: CarryGreeks = pricer.price_greeks(pos.option, &pos.inputs)?;`
- **Canonical delta.** For FX, keep the convention-pinned `SpotUnadjusted` delta (today via `convention_delta`). Generalize: the canonical delta is the **spot-unadjusted, premium-excluded** delta, which for every carry asset class is exactly `g.delta_spot` (the leaves already define `delta_spot = ∂V/∂S` premium-unadjusted). For FX, assert `g.delta_spot == convention_delta(SpotUnadjusted, …)` to ~1e-12 (they are equal by construction — this preserves the existing `canonical_delta_is_convention_independent` test). So: `delta_unadj = g.delta_spot` for non-FX; for FX, `delta_unadj = convention_delta(SpotUnadjusted, pos.option, fx_inputs)` (unchanged path, gated equal to `g.delta_spot`). The convention re-derivation runs **only when `pos.quoted_delta.is_some()`** (i.e. FX), so it never touches a non-FX position. This `is_some()` test is a provenance branch, not an asset-class match in a numeric hot loop — acceptable.
- `GreekEngine` generalization: today `Adjoint`/`Analytic` both call `celnet-vanilla`. Generalize to: `Analytic` = `pricer.price_greeks` (the leaf's closed form / its own AD), `Adjoint` = the FX-only fast path **kept FX-specific** behind `pos.underlying.as_fx()`; for non-FX, `Adjoint` falls back to `Analytic` (the leaves emit analytic closed-form Greeks already, which are themselves the scale path). Document that the adjoint specialization is an FX acceleration; the equivalence gate `adjoint_leaf_matches_analytic_leaf` stays FX-scoped.
- `vega_premium_ccy = pos.numeraire_ccy()` (the quote leg) — unchanged semantics.
- `premium_quote = g.price * n`.
- `canonicalize(pos)` becomes `canonicalize(&AssetPricer, pos)` (defaulting the dispatcher); it returns `Result` now. FX tests change `canonicalize(&pos)` → `canonicalize(&pos).unwrap()`.

**Why this satisfies "select the right leaf's sensitivities through the agnostic seam, NO match-on-underlying in hot path":** the canonicalize loop, the cube roll-up, and every reducer call `pricer.price_greeks(...)`; the `Underlying` discriminant is read once *inside the leaf adapter's* `price_greeks`, which is the leaf's own "is this mine" guard — exactly the ADR-0008 sanctioned location. No `match underlying { Fx => …, Equity => … }` appears in any aggregation/reduction loop.

### 1.4 `CanonicalLeaf` — drop the FX-only `pair`, keep currency legs

`CanonicalLeaf` today carries `pair: CcyPair` used by `numeraire::add_leaf_delta` to net `+delta_base` in `pair.base` and `−delta_base·spot` in `pair.quote`. Generalize the field to `underlying: Underlying`, and derive the two legs from `underlying.as_ccy_pair()` (base/quote). For FX/metals this is byte-identical to today. For equity/commodity/crypto-linear: the "base" leg is the asset's own unit and the "quote" leg is the numeraire — `add_leaf_delta` still produces `+delta_base` (asset units) and `−delta_base·spot` (numeraire). The asset-unit leg participates in the per-currency exposure vector keyed by the asset's `Ccy` projection (e.g. an equity ticker projected to a synthetic asset code, or — simpler and recommended — **carry an `AssetLeg` enum `{ Ccy(Ccy), AssetUnit(AssetId) }`** so the exposure vector nets fiat legs in `Ccy` and asset legs by asset id, never cross-converting an equity share count into a currency). See §1.5.

### 1.5 Currency-vs-asset exposure netting (file: `numeraire.rs`)

The FX assumption "every delta leg is a currency amount convertible at spot" breaks for equity/crypto-inverse. Make `CurrencyExposure` an **`ExposureVector`** over a `Leg` key:

```rust
pub enum LegKey { Ccy(Ccy), Asset(AssetId) }   // AssetId interned per non-fiat underlying
```

- Fiat legs (`Ccy`) convert to the reporting numeraire via the existing `SpotResolver` (unchanged).
- Asset legs (`Asset`) net **only among themselves** (you cannot add IBM shares to USD); converting an asset leg to the numeraire requires the asset's own spot, supplied by an extended resolver `fn asset_rate_into_numeraire(asset: AssetId) -> Option<f64>`. For the **vanilla FX/metal** case there are no `Asset` legs, so `in_numeraire` is byte-identical to today (the §5 invariant depends on this).
- **Crypto-inverse** (`1/S_T` payoff) settles in the **base coin**: its premium and vega are in coin units, not the quote numeraire. Tag `vega_premium_ccy`/premium as `LegKey::Asset(coin)` and require the coin spot for numeraire conversion. The inverse leaf's `delta_spot` is the coin-measure delta; the leaf already exposes a USD-equivalent — use the USD-equivalent for the *cross-asset numeraire view* and keep the coin-measure for the coin-margined desk node. Document this as the one place a per-asset-class settlement subtlety is honored, still without a hot-path match (the leaf adapter sets the `LegKey`).

**Minimal-risk alternative (recommended for the first landing):** restrict W5-A's cross-asset *numeraire collapse* to fiat-quoted underlyings (FX, metals, equity-index/commodity quoted in a fiat numeraire, crypto-**linear** in USD), all of which net cleanly in `Ccy` legs and need only the existing `SpotResolver`. Defer crypto-**inverse** coin-leg netting to a follow-up (it is the genuine non-linear settlement case). The additive Greek roll-up and FRTB §3 work for **all** arms; only the inverse coin-leg numeraire collapse is deferred — call this out honestly in the crate docs (no fake completeness).

### 1.6 `celnet-types` coordinated change (only if leaf arms are absent)

Today `Underlying = { Fx(CcyPair), Metal(MetalPair) }`. Equity/commodity/crypto leaves exist but their *underlying* is not yet an `Underlying` arm. Add (claim the WS-0 interface row first):

```rust
pub enum Underlying {
    Fx(CcyPair), Metal(MetalPair),
    Equity(EquityRef),        // ticker + listing/numeraire ccy
    Commodity(CommodityRef),  // commodity code + quote ccy (+ future/spot tag)
    Crypto(CryptoRef),        // coin pair + SettlementStyle{Linear,Inverse}
}
```
Each new arm must extend `Underlying::as_ccy_pair()` to project to the fiat **numeraire** leg (quote), and `as_fx()`/`as_metal()` return `None`. Keep arms minimal and purpose-named (no vendor names). Mirror in `celnet-proto` (W1 already generalized the wire; coordinate). **If the parallel session has already added these arms, skip — read the current `Underlying` before editing.**

---

## 2. `celnet-risk-cube` — route re-pricing through the seam

### 2.1 `dimension.rs`

- `FactKey.ccy_pair: CcyPair` → `underlying: Underlying`; `group_value(DimensionId::CcyPair)` packs `underlying.as_ccy_pair()` (unchanged hashing for FX; the dimension is renamed `Underlying` in `DimensionId` for clarity, but keep the same `u64` packing to preserve FX group identity).
- `FactMeasure.position: PositionRisk` is now carry-tagged (§1.1) — no further change; the exotic arm is unchanged.

### 2.2 `nonadditive.rs` — scenario apply + position re-price through the seam

- `Scenario` today has `spot_rel, vol_abs, rate_dom_abs, rate_for_abs`. Generalize `rate_dom_abs`/`rate_for_abs` to `discount_abs`/`carry_abs` (the two carry coordinates `r` and `b`); for FX, `r=r_dom`, `b=r_dom−r_for`, so a `rate_dom`/`rate_for` shock maps to `discount_abs = Δr_dom`, `carry_abs = Δr_dom − Δr_for`. Add `Scenario::apply(&CarryInputs) -> CarryInputs` that shocks `spot *= 1+spot_rel`, `vol += vol_abs`, and bumps the `Carry`:
  - `Carry::FxRates{r_dom,r_for}` → `FxRates{ r_dom + discount_abs, r_for + (discount_abs − carry_abs) }` (so Δr_dom=discount_abs, Δ(r_dom−r_for)=carry_abs). Verify this reproduces today's FX behavior byte-for-byte when only `discount_abs`/`carry_abs` derived from the old `rate_dom_abs`/`rate_for_abs` are set — gate it.
  - `Carry::CostOfCarry{r,b}` → `CostOfCarry{ r + discount_abs, b + carry_abs }`.
  - The shock arithmetic lives on `Carry` (add `Carry::shift(discount_abs, carry_abs) -> Carry` in `celnet-types` or a free fn in the cube) — **no match-on-underlying**, only the legitimate two-arm match inside `Carry` itself (an asset-agnostic carry transform, ADR-0008-clean).
- `position_pnl(pricer, pos, scenario)` and `node_pnl(pricer, positions, scenario)` take `&P: CarryPricer`; re-price via `pricer.price(pos.option, &ci)` and `pricer.price(pos.option, &scenario.apply(&ci))`. The FX `celnet_vanilla::price` call is replaced by the seam.
- `sensitivity_var_es` / `adjoint_greeks`: keep the FX adjoint fast path behind `pos.underlying.as_fx()`; for non-FX use the leaf's analytic `CarryGreeks` and Taylor-expand identically (the Taylor expansion is over `{spot, vol, discount, carry}` shocks and the carry-tagged Greeks — generalize the `rho_dom/rho_for` Taylor terms to `discount_rho/carry_rho` via `RateSensitivities`). The oracle (`historical_var_es`) is the bump-and-revalue through the seam — always exact, asset-agnostic.

### 2.3 `cube.rs` / `exotic.rs`

- `Cube`, `NodeAggregate`, `firm_aggregate`, `group_by`, `numeraire_view` thread `&P: CarryPricer` (or default `AssetPricer`) where they currently call FX pricing. `VegaPillarMap::pillar_of` already takes `&PositionRisk` — its `position.inputs.t` becomes `position.inputs.t` on `CarryInputs` (same field), no change.
- `curvature_legs` (in `frtb.rs`, see §3.4) re-prices through the seam.

---

## 3. FRTB bucket assignment — params **re-derived from the published text** (file: `crates/celnet-risk-cube/src/frtb.rs`)

The existing `frtb.rs` is the SbM **aggregation** machinery and is correct and asset-agnostic *given* buckets + weights as caller data. **What W5-A adds is the bucket/correlation *assignment*** — mapping a cross-asset node to `RiskBucket`/`CurvatureBucket` with the **regulatory** risk weights and correlations, re-derived from the BCBS MAR text.

### 3.1 The circular-oracle lesson (binding constraint)

The FRTB 0.75ρ bug: a constant was reused from an in-repo source that was itself unverified, making the oracle circular. **Rule for W5-A:** every numeric FRTB parameter introduced here (risk weights, intra-bucket ρ, cross-bucket γ) **must be re-derived in a doc comment from the cited BCBS MAR paragraph**, computed independently, and the *test* must assert the constant against a **separately hand-typed value from the paragraph** — never against another in-repo constant. The aggregation code already takes these as data (`SbmParams`, `gamma` closure), so the new code is a **`StandardFrtbParams` provider** that *supplies* the cited constants; the provider is what gets gated.

### 3.2 NEW: `StandardFrtbParams` (the cited regulatory parameter set)

NEW file `crates/celnet-risk-cube/src/frtb_params.rs`, re-exported from `frtb.rs`. Provides the prescribed constants per FRTB risk class, **each with a `// MAR2x.y(z): <verbatim-derivation>` citation**:

- **FX delta risk weight** — MAR21.88 (FX delta): the prescribed RW is **15%** for currency pairs, with a **divide-by-√2** relief (≈ ×0.7071) for the regulator-specified liquid pairs (MAR21.88 footnote / MAR21.91). Re-derive: `RW_fx = 0.15`; for a liquid-pair currency, `RW_fx_liquid = 0.15 / √2 = 0.106066…`. **Cite MAR21.88 and re-type 0.15 and √2 independently.**
- **FX vega risk weight** — MAR21.93 / MAR21.94: vega RW from the regulatory liquidity horizon `LH_FX` and `σ = RW_σ·√(LH/10)` with `RW_σ = 0.55` (MAR21.94) and `LH_FX = 40` (MAR21.93 Table). Re-derive `RW_vega_fx = min(RW_σ·√(LH_FX/10), 1.0) = 0.55·√4 = 1.10 → capped at 1.0`. **Cite the LH table value 40 and RW_σ 0.55, re-typed.**
- **FX delta intra/cross correlation** — FX delta has a **single risk factor per pair** (the spot), so the intra-bucket ρ is degenerate (one factor → no off-diagonal). The cross-pair γ is the FX-vs-FX correlation; MAR does not prescribe a single FX-delta cross-bucket γ (each currency is its own bucket; correlation enters via the common USD leg, handled by the currency-node netting in §1.5, not a γ). **Document that FX-delta cross-bucket correlation is structurally absent** — do not invent a γ. The vega buckets *do* carry the prescribed option-maturity correlation `ρ(T_k,T_l) = exp(−α·|T_k−T_l|/min(T_k,T_l))` with **α = 0.01** (MAR21.94) — re-derive and re-type α.
- **Curvature** — MAR21.98: curvature RW = the **largest delta RW** of the bucket's factors (so for FX, curvature shift = the 15% relief-adjusted RW). Curvature cross-bucket γ = (delta γ)² (already in `curvature_class`). **Cite MAR21.98.**
- **LOW-correlation scenario** — the `0.75ρ` floor that bit us: MAR21.6(2) says LOW = `max(2ρ−1, 0.75ρ)`. The existing `CorrelationScenario::Low::scale` already implements this. **Add a test that re-derives 0.75 from MAR21.6 verbatim and asserts `Low.scale(ρ)` matches the hand-typed `max(2ρ−1, 0.75ρ)` at several ρ** (this is the explicit anti-circular guard for the historical bug). Do **not** read 0.75 from any other constant.

### 3.3 NEW: bucket-assignment fns (in `frtb_params.rs`)

```rust
/// Assign a node's net delta sensitivities to FRTB delta buckets with the
/// MAR21.88-cited risk weights — asset-agnostic, driven off the canonical leaf's
/// `delta_base` and its currency/asset legs (NOT a match on Underlying; the bucket
/// id is the leg's currency/asset id, resolved through the seam-produced leaf).
pub fn standard_delta_buckets(node: &NodeAggregate, rw: &StandardFrtbParams) -> Vec<RiskBucket>;

/// Assign vega to (currency × maturity) buckets with the MAR21.94 maturity
/// correlation kernel.
pub fn standard_vega_buckets(node: &NodeAggregate, rw: &StandardFrtbParams) -> (Vec<RiskBucket>, impl Fn(u32,u32)->f64);

/// Curvature bucket with the MAR21.98 largest-delta-RW shift, via curvature_legs.
pub fn standard_curvature_buckets(pricer: &impl CarryPricer, node: &NodeAggregate, rw: &StandardFrtbParams) -> Vec<CurvatureBucket>;
```

Bucket id = the currency/asset **leg** the sensitivity belongs to (from the exposure vector of §1.5). This is the cross-asset generalization: a EURUSD delta, an XAUUSD delta, and an equity delta all bucket by their **risk-factor leg**, and the SbM aggregation (`delta_vega_class`) is unchanged. **No match-on-underlying:** the bucket key is read from the canonical leaf's legs, which were produced through the seam.

### 3.4 `curvature_legs` through the seam

`frtb.rs::curvature_legs` and `node_curvature_bucket` currently call `celnet_vanilla::{price,greeks}`. Re-route to `pricer.price(...)` / the seam's `delta_spot` via `pricer.price_greeks(...)`. The reprice-net-of-delta arithmetic is unchanged; only the pricing call changes. The existing `curvature_charges_short_gamma_only` test stays (FX), and a new equity/commodity curvature test is added (§5).

---

## 4. The LONGHAND recomputation oracle (file: `crates/celnet-risk-cube/tests/longhand_oracle.rs`)

A test-only, **maximally explicit, code-disjoint** recomputation of a small mixed-asset firm aggregate + its FRTB charge, written so it shares **no production helper** with the cube/normalize/frtb code. This is the independent oracle.

### 4.1 Longhand additive aggregate

For a hand-built 3-position mixed book (one FX EURUSD call, one XAUUSD put, one equity call), the test recomputes each canonical leaf **by hand**:

- Price/Greeks: call the **leaf crate's public `price`/`greeks` directly** (`celnet_vanilla::greeks`, `celnet_equity_vanilla::greeks`, etc.) — NOT through `AssetPricer`/`canonicalize`. Scale by notional in the test. Sum `delta_base`, `vega`, `gamma`, etc. with a plain `for` loop and `+=`.
- Assert the cube's `firm_aggregate(&AssetPricer, &pillar).net_greeks` equals the hand sum to **1e-12 relative** (these are the *same* numbers reached two code-disjoint ways: production routes through the seam, the oracle calls the leaves directly).

This proves the seam dispatch returns exactly the leaf's own Greeks (no silent FX-proxy for a non-FX position) — the central W5-A correctness claim.

### 4.2 Longhand FRTB charge

For the same book, recompute one risk-class SbM charge **fully longhand** in the test:

- Hand-type the MAR risk weights from §3.2 as literals in the test (re-derived in comments from the paragraph), compute `WS_k = RW_k · s_k`, the within-bucket `K_b = √(ΣWS² + ΣΣρ WS WS)`, the cross-bucket `√(ΣK_b² + ΣΣγ S_b S_c)`, and the three-scenario max — each step written out with explicit loops, no call to `SbmParams::class_charge`/`quadratic_form`.
- Assert the production `assemble_capital(standard_delta_buckets(...), …).sbm_total()` equals the longhand value to 1e-12.
- **Anti-circularity:** the test's risk weights are typed from the paragraph, the test's algebra is hand-written; the production path uses `StandardFrtbParams` (also typed from the paragraph) + `frtb.rs` algebra. They agree because the *paragraph* is the shared truth, not an in-repo constant. Add one assertion that the production `StandardFrtbParams::fx_delta_rw()` equals the hand-typed `0.15` (and the relieved `0.15/√2`) — catching any future drift of the production constant.

### 4.3 Curvature longhand

Recompute `CVR± = −[V(x(1±rw)) − V(x) ∓ rw·x·δ]` for the FX leg by calling `celnet_vanilla::price`/`greeks` directly in the test, and assert it equals `curvature_legs(&AssetPricer, …)` to 1e-9. Do the same for the equity leg via `celnet_equity_vanilla` directly — this proves curvature re-prices the *correct* leaf through the seam.

---

## 5. The FX invariant: `firm_aggregate == single-node 1e-12 stays green`

Two binding invariants, both gate tests in `crates/celnet-risk-cube/tests/fx_invariance.rs` and inline:

### 5.1 FX byte/1e-12 non-regression

Every pre-existing FX test in `risk-normalize` and `risk-cube` (the `lib.rs` test module, `numeraire.rs` tests, `frtb.rs` tests) must **stay green after the generalization**, modulo the mechanical `new→fx`/`unwrap` edits. Specifically:
- `additive_rollup_sum_of_children_equals_parent`, `firm_numeraire_view_collapses_cross_pair`, `shard_merge_is_associative_for_additive`, `var_es_matches_independent_quantile`, `curvature_charges_short_gamma_only` — all unchanged in assertions.
- Add an explicit **byte-identity** gate: build the same EURUSD book two ways — (a) via the old FX path (`PositionRisk::fx` → `AssetPricer`/seam) and (b) by calling `celnet_vanilla` directly and hand-canonicalizing — and assert `firm.net_greeks` fields are `to_bits`-identical for `delta_base` (which is convention-pinned, so bit-exact) and 1e-12 for the rest. This is the "FX stays byte-identical through the generalized seam" proof ADR-0008 requires.

### 5.2 `firm_aggregate == single-node`

The named invariant: a firm aggregate over N facts equals the aggregate computed by a **single node containing the same N positions**, to **1e-12**. Concretely:
- Build a cube with N mixed-asset facts under distinct trader/book/desk keys; `firm_aggregate(&AssetPricer, &pillar)`.
- Build a second "single-node" reference: one `NodeAggregate` constructed by folding all N canonical leaves directly (no hierarchy). Assert every `net_greeks` field, the `vega_ladder.total()`, and the `numeraire_view().delta_numeraire` agree to 1e-12.
- Also assert **roll-up associativity across assets**: `firm == Σ group_by(Desk)` to 1e-12 (the existing FX test, extended to a mixed-asset book). This guards that introducing non-FX legs did not break the associative additive contract.
- Non-additive guard: `node_var_es` (oracle, bump-revalue through seam) of the firm node equals the single-node `node_var_es` to 1e-12 (it must — same positions, same scenarios, same seam).

---

## 6. Module / type / fn manifest (exact targets)

**`celnet-risk-normalize`**
- `src/leaf.rs`: `PositionRisk{ underlying, option, notional_base, inputs: CarryInputs, quoted_delta: Option<_>, premium_style: Option<_> }`; `PositionRisk::fx(...)`, `::carry(...)`, `::numeraire_ccy()`. `CanonicalLeaf{ underlying, spot, greeks, premium_quote, vega_premium_ccy, quoted_was_premium_adjusted }`. `canonicalize<P:CarryPricer>(pricer,pos)->Result<_,CarryPriceError>`, `canonicalize_with<P>(pricer,engine,pos)`. `GreekEngine` unchanged (Adjoint FX-only behind `as_fx()`).
- `src/pricer.rs` (NEW): `AssetPricer`, `FxLeaf`, `EquityLeaf`, `CommodityLeaf`, `CryptoLeaf` (each `impl CarryPricer`), dispatch table.
- `src/numeraire.rs`: `LegKey{Ccy(Ccy),Asset(AssetId)}`, `ExposureVector` (rename of `CurrencyExposure`), `add_leaf_delta` keyed by `LegKey`, `in_numeraire` (fiat path unchanged; asset path errors if no asset rate). `AssetId` interned id. (First landing may keep fiat-only per §1.5.)
- `src/lib.rs`: re-export the new symbols; doc the cross-asset scope and the deferred crypto-inverse coin-leg honestly.

**`celnet-risk-cube`**
- `src/dimension.rs`: `FactKey.underlying: Underlying`; `DimensionId::Underlying` (was `CcyPair`); `group_value` packs `as_ccy_pair()`.
- `src/nonadditive.rs`: `Scenario{spot_rel,vol_abs,discount_abs,carry_abs}`, `Scenario::apply(&CarryInputs)`, `position_pnl<P>`, `node_pnl<P>`, `sensitivity_var_es<P>` (FX adjoint behind `as_fx()`), `historical_var_es<P>` (seam, exact).
- `src/cube.rs`, `src/additive.rs`, `src/exotic.rs`: thread `&P: CarryPricer`; additive unchanged in algebra.
- `src/frtb.rs`: `curvature_legs<P>`, `node_curvature_bucket<P>` through the seam.
- `src/frtb_params.rs` (NEW): `StandardFrtbParams` (cited constants), `standard_delta_buckets`, `standard_vega_buckets`, `standard_curvature_buckets`, vega maturity-correlation kernel; re-exported from `frtb.rs`/`lib.rs`.

**`celnet-types`** (coordinated, only if arms absent): `Underlying::{Equity,Commodity,Crypto}` + `EquityRef`/`CommodityRef`/`CryptoRef`; extend `as_ccy_pair`; optional `Carry::shift(d,c)`.

---

## 7. Dependency order (mechanical build sequence)

1. **(coord)** If needed, add `Underlying` arms + refs in `celnet-types` (claim WS-0 row). Gate `celnet-types` alone.
2. `celnet-risk-normalize/src/pricer.rs` — `AssetPricer` + leaf adapters. Add dev-deps on the leaf crates. Gate: each adapter prices its asset and rejects the others with the typed error.
3. `celnet-risk-normalize/src/leaf.rs` — generalize `PositionRisk`/`CanonicalLeaf`/`canonicalize`. Add `::fx` constructor. Update FX tests (`new→fx`, `unwrap`). Gate normalize crate.
4. `celnet-risk-normalize/src/numeraire.rs` — `LegKey`/`ExposureVector` (or fiat-only first landing). Gate.
5. `celnet-risk-cube/src/dimension.rs` + `nonadditive.rs` + `cube.rs` + `additive.rs` + `exotic.rs` — thread the seam. Update FX tests. Gate cube crate.
6. `celnet-risk-cube/src/frtb.rs` + `frtb_params.rs` — seam re-route + cited bucket assignment. Gate.
7. **Oracle + invariant gates**: `tests/longhand_oracle.rs`, `tests/fx_invariance.rs`. Gate.
8. Cross-crate integration gate (`just check-crate celnet-risk-normalize`, `just check-crate celnet-risk-cube`), then the milestone `just check` before commit.

Each step gates only the touched crate (`just check-crate <crate>`); the full-workspace `just check` is the pre-commit milestone gate.

---

## 8. Exact gate-test inventory (what "done" means)

| # | Test (file) | Asserts |
|---|---|---|
| G1 | `pricer::adapters_dispatch_by_seam` (pricer.rs) | each leaf adapter prices its asset; returns `UnsupportedUnderlying`/`UnsupportedCarry` for others; `AssetPricer` returns the first `Ok`; **no `match underlying` in any reducer** (enforced by review + a doc-test note). |
| G2 | `canonical_delta_is_convention_independent` (leaf.rs, kept) | FX canonical delta still convention-invariant + equals `convention_delta(SpotUnadjusted)·n`. |
| G3 | `non_fx_canonical_delta_is_leaf_delta_spot` (leaf.rs, NEW) | equity/commodity canonical `delta_base == leaf.greeks.delta_spot·n` to 1e-12 (proves the seam returns the leaf's own delta, not an FX proxy). |
| G4 | `adjoint_leaf_matches_analytic_leaf` (leaf.rs, kept, FX-scoped) | FX adjoint==analytic to ~1e-9. |
| G5 | `usd_legs_net_across_pairs`, `vega_converts_through_premium_currency` (numeraire.rs, kept) | FX currency-node netting unchanged. |
| G6 | `correlation_scenarios_transform_correctly` + **NEW** `low_scenario_075_floor_rederived` (frtb.rs) | `Low.scale(ρ)==max(2ρ−1, 0.75ρ)` with 0.75 **hand-typed from MAR21.6**, the explicit anti-circular guard for the historical bug. |
| G7 | `standard_frtb_params_match_cited_text` (frtb_params.rs, NEW) | `fx_delta_rw()==0.15`, `fx_delta_rw_liquid()==0.15/√2`, `fx_vega_rw()==min(0.55·2,1.0)`, `vega_corr_alpha()==0.01` — each vs a hand-typed paragraph value, **never vs another in-repo constant**. |
| G8 | `longhand_additive_oracle` (tests/longhand_oracle.rs, NEW) | mixed 3-asset firm `net_greeks` == leaf-direct hand sum to 1e-12. |
| G9 | `longhand_frtb_oracle` (tests/longhand_oracle.rs, NEW) | production SbM total == fully hand-written SbM algebra to 1e-12. |
| G10 | `longhand_curvature_oracle` (tests/longhand_oracle.rs, NEW) | `curvature_legs` (seam) == leaf-direct CVR± to 1e-9 for FX and equity legs. |
| G11 | `fx_firm_aggregate_byte_identical` (tests/fx_invariance.rs, NEW) | EURUSD book via seam == via direct `celnet_vanilla` path; `delta_base` `to_bits`-identical, rest 1e-12. |
| G12 | `firm_aggregate_equals_single_node` (tests/fx_invariance.rs, NEW) | N-fact firm == single-node fold to 1e-12 (additive + vega ladder + numeraire delta + non-additive `node_var_es` oracle). |
| G13 | `rollup_associative_across_assets` (cube lib.rs, extended) | `firm == Σ group_by(Desk)` to 1e-12 on a mixed-asset book. |
| G14 | all kept FX cube/normalize tests | green post-generalization (mechanical edits only). |

---

## 9. Honest scope boundaries (no fake depth)

- **Crypto-inverse coin-leg numeraire collapse** is the one genuine non-linear settlement case; first landing may net it only within its own coin node and expose the leaf's USD-equivalent for the cross-asset view, deferring full coin-leg netting (documented, not stubbed).
- **GIRR / rates delta** buckets (MAR21.40s) are a distinct risk class; W5-A scopes FX-delta, FX-vega, generic carry-asset delta/vega, and curvature. Rates-curve bucketing is a named follow-on, not faked here.
- **DRC for non-FX** (a defaultable equity/credit issuer) carries a genuine JTD; the existing `fx_default_risk_charge` is a cited zero for FX only. A non-FX DRC module is out of W5-A scope and must not be faked — `assemble_capital` keeps DRC as a caller-supplied/zero-for-FX input with the honest doc.
- All FRTB constants are **provided** by `StandardFrtbParams` (cited) and **consumed** as data by the unchanged `frtb.rs` aggregation — recalibration stays data, not a recompile (guardrail §2.11).

