# W1 — Multi-asset CORE wave: staged execution plan

> The single highest-risk wave of `docs/MASTER-EVOLUTION-PROGRAM.md` (§2/§3). Generalizes the
> three FX-only Layer-0 seams to a cross-asset vocabulary **in place** (one unversioned
> contract, delete legacy — guardrails #9/#10), with **FX proven byte-identical** as the
> no-regression gate. This doc is the execution plan; the single backlog truth stays
> `docs/WORLD-CLASS-BACKLOG.md`.

## 0. The crux risk & the governing principle

The blast radius is large: 80+ crates read `VanillaInputs.r_dom/r_for` (mostly via the
`forward()`/`df_dom()`/`df_for()` methods); 20+ read `Greeks.rho_dom/rho_for`; 50+ files
reference `CcyPair`. The crux is **byte-identity of the two FX rhos** — any reformulation of
the Garman-Kohlhagen arithmetic risks a 1-ULP drift that fails the golden gate.

**Governing principle:** *do not touch the FX (GK) arithmetic.* The generalization is in the
**type system** (an `Underlying` discriminator, a `Carry` model, carry-tagged
`Sensitivities`); `celnet-vanilla` keeps its exact GK formulas, reading the FX carry and
writing the FX projection of the generalized sensitivities. FX byte-identity is then
preserved *by construction* (same arithmetic), not by luck. Non-FX leaves (W3/W5) implement
the generalized carry path; W1 only needs to prove the generalization works for FX +
**one** non-FX validation (an equity-dividend plugin model vs QuantLib) per the master plan.

## 1. Target vocabulary (celnet-types / celnet-core)

```rust
// celnet-types — the asset-class discriminator. W1 ships only the Fx arm; W3/W5 add arms
// (one unversioned contract ⇒ additive enum growth, no placeholder arms now).
pub enum Underlying { Fx(CcyPair) /* + Metal/DigitalAsset/Equity/Listed in later waves */ }
pub struct Symbol(/* length-validated, for non-3-char symbols — added when first non-FX arm lands */);

// The carry model. FxRates keeps FX EXACT (no arithmetic change ⇒ byte-identical).
// CostOfCarry is the generalized-BSM (r, b) arm for equity (b=r−q) / commodity / crypto.
pub enum Carry {
    FxRates { r_dom: f64, r_for: f64 },   // forward = S·e^{(r_dom−r_for)t}, df = e^{−r_dom·t}
    CostOfCarry { r: f64, b: f64 },        // forward = S·e^{b·t},          df = e^{−r·t}
}
impl Carry { fn discount_df(&self, t)->f64; fn forward_factor(&self, t)->f64; fn r_discount(&self)->f64; }

// Carry-tagged rate sensitivities. Fx arm is byte-identical to today's two flat rhos.
pub enum RateSensitivities {
    Fx { rho_dom: f64, rho_for: f64 },     // FX
    Carry { discount_rho: f64, carry_rho: f64 }, // equity dividend-rho / commodity carry-rho
}
```

`VanillaInputs` keeps `spot/strike/vol/t` and replaces the two raw rate fields with a
`Carry`. Its `forward()/df_dom()/df_for()` accessors are preserved for FX callers (FxRates
delegates to the identical `libm::exp` calls), so the ~80-crate read-blast shrinks to: code
that **constructs** `VanillaInputs` with `r_dom/r_for`, and code that **reads** `.r_dom/.r_for`
directly. `Greeks` replaces the two flat `rho_dom/rho_for` fields with `RateSensitivities`;
consumers pattern-match (FX path unchanged in value).

## 2. Contract (celnet-proto §3) — generalize in place, FX byte-equivalent

- `Underlying` message `oneof ref { CcyPair fx = 1; }` (+ future arms) + `Ccy settlement_ccy`.
  Replace `CcyPair pair` on `Instrument`, `BasketLeg`, `MarketSeries*`, `OrgKey.ccy_pair`.
  The codec maps an old-style FX pair into `Underlying.fx` ⇒ existing flows byte-equivalent.
- `MarketContext`/`VanillaInputs`: `{spot, vol, discount_rate, CarryModel carry}` with
  `CarryModel oneof { FxRates fx; CostOfCarry generalized; }`. FX ⇒ `FxRates{r_for}` +
  `discount_rate=r_dom`. Delete top-level `r_dom/r_for`.
- `Greeks`: replace `rho_dom=7/rho_for=8` with `RateSensitivities` (oneof fx/carry). FX bit-identical.
- Product oneof stays flat (18 arms). New payoff SHAPES (FxForward/Swap/Ndf/Pivot/…) are LATER
  waves, not W1. `BasketLeg` re-embeds `Underlying`+`CarryModel`.
- `VegaPillar` axis oneof + `RISK_DIMENSION_CCY_PAIR → RISK_DIMENSION_UNDERLYING`.
- Use the `protobuf` skill for enum-prefix/field-number hygiene. Gate every step with a
  convert round-trip + `to_bits` byte-identity for the FX projection.

## 3. plugin-api (§3) — generalize the headline differentiator

`PricingModel::price(OptionType, &VanillaInputs) -> Greeks` parameterized over the new
vocabulary (so a non-FX model is expressible). WIT `vanilla-inputs`/`greeks` + the wasmi
`(ptr,len)` ABI (`celnet-plugin-host/src/abi.rs`, INPUT_BYTES/GREEKS_BYTES) updated in
lockstep. Gate: the 4 existing plugin gates stay green for FX **+** a NEW gate registering an
equity-dividend (`CostOfCarry`) model reconciled to an independent QuantLib/closed-form oracle
(this is the proof the carry generalization is real, not FX-reshaped).

## 4. Staged GREEN increments (single driver; each commit gated; NOT parallel lanes)

The §2 sequencing rule forbids *parallel-lane* contract edits, not incremental green commits.
Stage so the workspace compiles + `just check` is green at every commit:

- **S1 (additive vocabulary):** add `Underlying`/`Carry`/`RateSensitivities` to
  celnet-types/celnet-core ALONGSIDE the existing fields (nothing removed). Add `From`
  conversions FX↔Carry and unit tests incl. `to_bits` FX round-trip. `just check` green. Commit.
- **S2 (celnet-vanilla on Carry, FX byte-identical):** the GK leaf reads the FxRates carry and
  emits `RateSensitivities::Fx`; arithmetic untouched. Gate: full vanilla golden CSV +
  `celnet-parity/tests/greeks.rs` byte-identical (`to_bits`). Commit.
- **S3 (exotics/surface/engine/risk consumers):** migrate the field reads to the carry
  accessors; FX values unchanged. Per-crate `check-crate` + golden/parity byte-identical. Commit.
- **S4 (proto + convert):** generalize the wire messages + codec; FX convert round-trip
  `to_bits`-identical; the W0 golden-vector conformance corpus stays green across all 5 clients
  (the no-regression gate). Commit.
- **S5 (plugin-api + WIT + wasmi ABI):** generalize; 4 existing gates green + NEW equity-dividend
  model gate vs QuantLib. Commit.
- **S6 (delete legacy + reconcile):** remove the superseded flat `r_dom/r_for`/`rho_*` paths
  (#10), re-index codebase-memory, reconcile INTERFACES/ARCHITECTURE/CONVENTIONS docs, add the
  `Underlying`/`Carry` rows to the W0 verification corpus. Full `just check` + all 5 client
  suites + the conformance harness green. Milestone commit + push.

## 5. Gates (every step) & the milestone bar

- FX **byte-identical**: vanilla golden CSV + `celnet-parity` rows + proto convert round-trip,
  all `to_bits`-identical (never "approximately equal" for the FX projection).
- The W0 cross-client conformance corpus (84 vectors / 18 families) stays green on all 5 clients.
- Full `just check` prints the literal **"All gates passed."** (verified by me, not a wrapper
  exit code); GUI vitest + Playwright e2e + Excel real-edge e2e green.
- One NEW non-FX proof: an equity-dividend plugin model vs an independent oracle.
- Re-index codebase-memory; reconcile docs (no stale "FX-only" claims for generalized code).

## 6. Out of W1 scope (later waves, designed-seam only here)
New asset-class leaves (crypto/equity/commodity — W3/W5), new payoff shapes
(forward/swap/NDF/Pivot — W2/W4), multi-dealer RFQ (W4). W1 delivers the *seams* so those are
data/registry/leaf additions, not rewrites. Deploy-bound items stay ENV per the honest boundary.
