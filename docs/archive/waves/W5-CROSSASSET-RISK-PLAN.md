# W5 — Cross-asset risk + equity/commodity leaves: staged execution plan

> Executes the **W5** wave of [MASTER-EVOLUTION-PROGRAM.md](MASTER-EVOLUTION-PROGRAM.md) §5
> (P1/P2) and closes four [WORLD-CLASS-BACKLOG.md](../audits/WORLD-CLASS-BACKLOG.md) items:
> `risk/cross-asset-canonical-leaf`, `risk/frtb-cross-asset-buckets`,
> `equity/equity-vanilla-leaf`, `commodity/commodity-vanilla-leaf`. This is where the
> **factor-keyed risk layer of [ADR-0008](../../adr/ADR-0008-multi-asset-carry-architecture.md)**
> (the "factor decomposition into FRTB GIRR/equity/commodity buckets happens at the risk
> layer, whose roll-up algebra is already factor-generic") is realised, and where the first
> two cross-asset payoff **leaves** land on the W1 carry seam. The single backlog truth stays
> `WORLD-CLASS-BACKLOG.md`; the mandatory per-product gate set is
> [VERIFICATION-CONTRACT.md](../../VERIFICATION-CONTRACT.md).

## 0. Preconditions (ASSUMED IN PLACE from W1) & the governing principle

W1 is assumed complete and FX-byte-identical:

- `celnet-types` carries `Underlying` (`Fx(CcyPair)` arm shipped; this wave adds the
  `Equity`/`Listed`/`Commodity` arms only as the leaves need them — additive enum growth,
  no placeholder), `Carry` (`FxRates` + `CostOfCarry{r,b}`), and `RateSensitivities`
  (`Fx{rho_dom,rho_for}` / `Carry{discount_rho,carry_rho}`). `crates/celnet-types/src` already
  documents `b = r − q` (equity) and `b = r − convenience` (commodity) and the carry-rho split
  — see the existing `RateSensitivities::Carry` doc.
- A Carry-based pricing trait lives in `celnet-core`; payoff engines call
  `inputs.forward(t)` / `inputs.discount_df(t)` and **never** branch on the `Carry` variant
  (ADR-0008 "no-workaround test").
- `celnet-vanilla` is the FX (GK) leaf; `celnet-proto` carries `Underlying`/`CarryModel`/
  `RateSensitivities` + the product×underlying validity matrix.

**Governing principle for W5 (two halves, one rule):**

- **Track A (risk) is a *generalization-in-place* of `celnet-risk-normalize` /
  `celnet-risk-cube`.** The non-negotiable invariant is that the existing FX
  `firm_aggregate == single-node` parity (1e-12, in `crates/celnet-parity/tests/exotic_risk_cube.rs`
  and `crates/celnet-risk-fleet/tests/`) **stays byte-green** — the additive-merge /
  non-additive-re-gather algebra (`Cube::merge_additive`, `firm_aggregate`) is **already
  factor-generic and must not change**. We widen the *fact* (asset-class tag + generic factor
  set + underlying ref), not the algebra.
- **Track B (leaves) is *additive* — two new disjoint leaf crates** on the W1 carry seam, with
  **zero payoff-engine edits to FX** and zero edits to the shared contract beyond the additive
  `Underlying`/`CarryModel` arms W1 already designed for.
- **The FRTB cross-asset buckets carry the 0.75ρ circular-oracle lesson verbatim**
  (VERIFICATION-CONTRACT §(a)): every BCBS constant is **re-derived from the MAR primary text**
  in the test, the oracle is a **longhand independent recomputation** that does not reuse the
  production algebra, and at least one gate pins the constant transform to hand-computed MAR
  values so it cannot silently drift.

Tracks A and B are **crate-disjoint** and run on parallel git-worktree lanes; the only shared
edit is the additive proto arms (Track B) and the additive `RiskFact` widening (Track A), which
touch different files. The contract edits are tiny and additive (W1 already froze the seam), so
they do not violate the §2 sequencing rule against dribbling contract edits.

---

## 1. Track A — cross-asset risk fact + FRTB GIRR/equity/commodity/CSR buckets

### 1.1 Generalize `CanonicalLeaf` → a cross-asset risk fact

Today (`crates/celnet-risk-normalize/src/leaf.rs`): `PositionRisk.pair: CcyPair`,
`PositionRisk.inputs: VanillaInputs`, `CanonicalLeaf.pair: CcyPair`,
`CanonicalLeaf.vega_premium_ccy: Ccy`. These FX-typed fields block a mixed FX+equity+commodity
firm book. The cube's `RiskFact`/`FactMeasure` (`crates/celnet-risk-cube/src/dimension.rs`) embed
`PositionRisk`/`CanonicalLeaf` directly, and `DimensionId::CcyPair` keys the org placement.

**The widening (additive, FX-default, algebra-unchanged):**

- New `enum AssetClass { Fx, Equity, Commodity }` (+ later `Crypto`, `Rates`) in
  `celnet-risk-normalize` — the **only** place the risk layer names asset class, mirroring
  ADR-0008's `Underlying` discipline. The non-additive reducers route on this tag to the right
  re-pricer (FX vanilla / equity / commodity), exactly as `FactMeasure.exotic` already routes a
  barrier to the exotic pricer vs a vanilla to `celnet-vanilla`. **The additive roll-up never
  reads the tag** — `CanonicalGreeks` sums regardless of class (ADR-0008: the additive algebra is
  already factor-generic).
- Replace `CanonicalLeaf.pair: CcyPair` / `PositionRisk.pair: CcyPair` with an
  `UnderlyingRef` carrying the W1 `Underlying` + the spot/settlement currency the leaf nets at.
  FX leaves keep `vega_premium_ccy` semantics; equity/commodity vega is in the underlying's
  premium currency (e.g. index points × multiplier → settlement ccy). The numeraire-conversion
  leg (`CanonicalLeaf::premium_base_delta_adjustment`, `NodeAggregate::numeraire_view`) stays
  FX-typed only where it genuinely is an FX conversion; a non-FX leaf supplies its own
  spot→numeraire rate via the same `SpotResolver` seam (already generic).
- `PositionRisk.inputs: VanillaInputs` → the **carry-tagged** inputs from W1 (`VanillaInputs`
  now holds a `Carry`). The canonical Greek set is re-derived through the **asset-class leaf's**
  Greeks (FX → `celnet-vanilla`, equity → `celnet-equity-vanilla`, commodity →
  `celnet-commodity-vanilla`) selected by `AssetClass`, replacing the hard `use celnet_vanilla`.
  The `GreekEngine::{Adjoint,Analytic}` choice is preserved as an FX-only acceleration (the AAD
  graph is GK-specific); non-FX leaves use their analytic Greeks until an adjoint is added.
- `DimensionId::CcyPair` → `DimensionId::Underlying` (the W1 proto already renames
  `RISK_DIMENSION_CCY_PAIR → RISK_DIMENSION_UNDERLYING`); the FX path keys on the FX underlying,
  byte-equivalent.

**Factor-keyed sensitivities at the risk layer (ADR-0008 §"factor-keyed risk layer"):** the
hot-core `Greeks` keeps its fixed POD shape with `RateSensitivities` (no allocating map). The
risk layer decomposes `RateSensitivities` into **FRTB-bucketed factors** here, off the hot path:
FX `(rho_dom, rho_for)` → GIRR vertices of the two legs' curves; equity `(discount_rho,
carry_rho=dividend_rho)` → GIRR + equity-repo; commodity `(discount_rho, carry_rho=convenience)`
→ GIRR + commodity carry. This decomposition is a **risk-layer projection function**, not a new
hot-core field — exactly ADR-0008's reconciliation of "general" with "zero-alloc."

### 1.2 Extend FRTB-SA from the FX bucket to GIRR / equity / commodity / CSR

`crates/celnet-risk-cube/src/frtb.rs` already implements the full SbM machine generically —
`RiskBucket`, `SbmParams<G>`, `quadratic_form`, the three-scenario `CorrelationScenario`, `K_b`,
the cross-bucket roll-up, and curvature `CurvatureBucket`. **The aggregation algebra is already
risk-class-agnostic** (`SbmParams<G>` takes caller-supplied risk weights and γ — `§2.3` external
data, never compiled in). W5 does **not** rewrite the kernel; it adds:

- **The bucketing/risk-weight DATA for the new risk classes**, supplied as caller data
  (`SbmParams`), with the MAR risk-class structure documented:
  - **GIRR** (MAR21.8–21.10): per-curve tenor-vertex delta buckets (one bucket per curve/currency),
    intra-bucket ρ by tenor distance + same/diff-curve, cross-bucket γ; the **basis/inflation**
    sub-risks are documented as the next increment (designed-seam) if not funded — honestly, not
    faked.
  - **Equity** (MAR21.5x): the 11 prescribed equity buckets (large/small cap × sector + indices),
    spot + repo sub-classes.
  - **Commodity** (MAR21.5x): the 11 commodity buckets (energy/metals/agri/…), with the
    same-/different-commodity intra-bucket ρ structure.
  - **CSR non-securitised** (MAR21.5x): sector/credit-quality buckets — included because a
    cross-asset firm book that holds any credit-linked leg needs it; if no in-repo product emits
    CSR sensitivities, the bucket set is delivered + parity-gated against the longhand oracle but
    flagged "no in-repo product currently sources CSR factors" (honest scope, not a fake row).
- **The factor-projection** (§1.1) that turns a cross-asset `NodeAggregate` into the per-class
  `SbmParams` so `delta_vega_class` / curvature produce a real multi-class capital number.

### 1.3 Parity rows & oracle (Track A) — the 0.75ρ lesson, applied per class

New parity rows under `crates/celnet-parity/tests/`:

- `cross_asset_risk_cube.rs` — **the FX no-regression gate + the mixed-asset gate.** Asserts
  (i) the existing FX `firm_aggregate == single-node` to **1e-12** still holds after the
  `RiskFact` widening (re-run the existing FX fixtures through the generalized fact); (ii) a
  **mixed FX+equity+commodity** book nets **per factor** (FX delta legs net at the FX underlying
  node; equity delta at the equity node; GIRR/discount-rho legs of all classes net at the shared
  GIRR vertices) and the firm roll-up == an independent single-node recomputation to 1e-12 for the
  additive measures and 1e-9 for the re-gathered non-additive measures (the existing tolerance
  pair). **Oracle:** an in-test independent re-aggregation that sums the per-leaf canonical Greeks
  by hand into per-factor buckets — it must NOT call `merge_additive`/`firm_aggregate` (that would
  be circular).
- `frtb_cross_asset.rs` — **FRTB SbM for GIRR / equity / commodity / CSR**, one worked-example
  longhand oracle per class, in the established `frtb.rs` template:
  - **Oracle:** a `longhand_<class>` recomputation written out independently in the test
    (the existing `longhand_k_b` / `longhand_scale` / `longhand_two_bucket` pattern), reaching the
    charge by a genuinely different route.
  - **Anti-circular constant gate (MANDATORY):** extend the existing
    `correlation_scenario_transform_matches_basel_constants` with **per-class hand-computed MAR
    values** — the GIRR/equity/commodity intra- and inter-bucket ρ/γ taken **from the MAR primary
    text**, with the explicit `0.75ρ` low-correlation floor pinned for each (the documented FRTB
    bug template). **CIRCULAR-ORACLE RISK FLAG:** the FX longhand oracle in `frtb.rs` shares the
    SbM *structure* with the production kernel (both compute `√(ΣWS² + ΣΣρWSWS)`); this is
    acceptable ONLY because (a) it is re-typed independently in the test and (b) the constant gate
    pins the transform to external hand-values. For the new classes we **must not** lift the
    bucket ρ/γ data from the same constant module the production `SbmParams` is fed from — the test
    re-derives them from MAR text. If a class's longhand oracle ends up re-using the production
    risk-weight/correlation table verbatim, that is a circular oracle and fails gate (a); add a
    qualitative gate (a hedged-bucket `K_b == 0`, monotonicity in ρ, or a published worked example)
    that can disagree.

Track A introduces **no new byte decoder** and **no new numeric-core MC**; per
VERIFICATION-CONTRACT §(f) the parity-row module doc states "neither fuzz nor mutation applies —
this is offline analytic aggregation, exempt from the hot-path budget" (§(e)), and the existing
`just mutants-gate-risk-cube` continues to cover the kernel.

---

## 2. Track B — `celnet-equity-vanilla` + `celnet-commodity-vanilla` leaves

Two new leaf crates, siblings of `celnet-vanilla`, on the W1 carry seam. Each auto-joins via
`members = ["crates/*"]` and is registered in the root `[workspace.dependencies]`
(`celnet-equity-vanilla = { path = "crates/celnet-equity-vanilla" }`, likewise commodity); all
internal deps go through the registry as `celnet-x.workspace = true` (the `just workspace-deps`
lint forbids relative `path = "../celnet-*"`).

### 2.1 `celnet-equity-vanilla`

- **Deps:** `{ celnet-types, celnet-core }` (mirrors `celnet-vanilla`), dev-dep
  `celnet-conventions` if needed.
- **Math:** generalized-BSM via the W1 `Carry::CostOfCarry{r, b}` with `b = r − q` (continuous
  dividend yield) + optional repo spread folded into `b` (`b = r − q − repo`). Price + full Greek
  strip, emitting `RateSensitivities::Carry{discount_rho = ∂V/∂r, carry_rho = ∂V/∂b}` where
  `carry_rho` is the **dividend-rho**. **No `match carry`** — the engine reads
  `inputs.forward(t)` / `inputs.discount_df(t)` per the ADR-0008 no-workaround test.
- **Underlying:** an `Underlying::Equity(EquityRef)` arm (additive) with a length-validated
  `Symbol` (not a 3-char `Ccy`), settlement = cash in the index/quote currency.

### 2.2 `celnet-commodity-vanilla`

- **Deps:** `{ celnet-types, celnet-core }`.
- **Math:** **Black-76** on the futures price `F` (the commodity option is on a listed future →
  zero carry on `F` under the futures measure: `forward = F`, `discount = e^{−rT}`), plus the
  **spot+convenience** representation `b = r − convenience` for an option on spot/forward — both
  expressible through `Carry::CostOfCarry` (Black-76 is the `b = 0` degenerate, priced off `F`
  directly). Full Greek strip; `carry_rho` = the **convenience/carry-rho**; futures-settled
  (cash-settled at the future's settlement). `Underlying::Listed(ContractRef)` / a `Commodity`
  arm (additive).

### 2.3 Oracle & parity rows (Track B) — VERIFICATION-CONTRACT (a)–(d)

- **`crates/celnet-parity/tests/equity_vanilla.rs`:**
  - **Oracle (a):** QuantLib `AnalyticEuropeanEngine` with a `BlackScholesMertonProcess`
    (dividend yield), frozen into `crates/celnet-golden/` as a new table, gated to **1e-12** (the
    backlog requirement). **CIRCULAR-ORACLE RISK FLAG:** generalized-BSM with `b = r − q` is
    *algebraically the same* as Garman-Kohlhagen with `r_for = q` — the equity price is the FX
    price with `r_for ↦ q`. So a "second closed form" written in the test would be circular. The
    oracle must therefore be the **external QuantLib value** (genuinely different code path) AND a
    **qualitative gate that can disagree**: put-call parity `C − P = e^{−qT}S − e^{−rT}K`, the
    `q = 0 → ` non-dividend BSM limit, and a published equity-option reference value
    (e.g. Hull worked example) hand-pinned with citation. The dividend-rho (`carry_rho`) is
    cross-checked by central finite difference (an independent route from the analytic).
  - **(c)** golden vector `crates/celnet-golden/vectors/equity_vanilla.json` (the family key is the
    new proto oneof arm name) generated by the engine, each value oracle-checked.
- **`crates/celnet-parity/tests/commodity_vanilla.rs`:**
  - **Oracle (a):** Black-76 closed form reached **independently** (the test writes the Black-76
    formula directly — but see the flag below) **plus** a QuantLib commodity-future-option golden
    table to **1e-12**, **plus** qualitative gates: put-call parity on futures
    `C − P = e^{−rT}(F − K)`, the `convenience = r` (`b = 0`) ↔ Black-76-off-`F` equivalence, and a
    published Black-76 worked example. **CIRCULAR-ORACLE RISK FLAG:** the production pricer *is*
    Black-76, so an in-test Black-76 is circular by construction — the independent leg MUST be the
    QuantLib golden table + the parity/limit gates, not a re-typed Black-76. State this in the
    module doc.
  - **(c)** golden vector `crates/celnet-golden/vectors/commodity_vanilla.json`.
- **(f)** both leaves are **numeric-core** → each adds its own mutation gate
  (`just mutants-gate-equity-vanilla` / `-commodity-vanilla`, audited equivalence set in
  `.config/mutants-*.toml`, ≥90% non-equivalent kill-rate). No untrusted-byte decoder → no fuzz
  target (state explicitly in the module doc).

---

## 3. Cross-client surfacing (api-first, FX-default) — VERIFICATION-CONTRACT (d)

Per [VERIFICATION-CONTRACT.md](../../VERIFICATION-CONTRACT.md) §(d), each new product/asset-class is
"done" only when reachable and identical from **server == SDK == CLI == Excel == GUI** against a
**real booted edge**, with a generated `CLIENT-PARITY-MATRIX.md` row. FX stays the default
everywhere (ADR-0008: "no new friction for the FX user").

- **Contract:** add the `equity_vanilla` / `commodity_vanilla` arms to the `Instrument.product`
  oneof (additive; the §2 sequencing allows additive arms now that the W1 seam is frozen) + the
  `Underlying::{Equity,Commodity/Listed}` + `CarryModel::CostOfCarry` arms. The server enforces the
  **product × underlying validity matrix** (equity-vanilla on an FX underlying ⇒
  `INVALID_ARGUMENT`, never a silent fallback — ADR-0008 "no silent mis-pricing"). Use the
  `protobuf` skill for enum-prefix/field-number hygiene; gate with a convert round-trip.
- **Server:** route the new arms to the new leaves through the W1 Carry-based trait.
- **SDK (`celnet-client`):** typed constructors `Underlying::equity(symbol)` /
  `Underlying::commodity(contract)` + `CarryModel::dividend_yield(q)` /
  `::convenience_yield(c)`; one runnable example per leaf gated against a real edge (the
  `demo_edge` pattern) asserting SDK == server == the frozen vector.
- **CLI (`celnet-cli`):** `--underlying`/`--asset-class` already defaults to FX-pair parsing
  (W1); add `celnet price --equity AAPL …` / `celnet price --commodity CL …`, proven CLI == SDK ==
  server.
- **Excel:** the polymorphic `CELNET.PRICE(underlying, product, terms-range)` set (W1/W4) reaches
  the new arms with no new per-product function; add the equity/commodity rows to the **real-edge**
  Excel conformance suite.
- **GUI:** the asset-class-aware `UniverseNavigator` gains Equity/Commodity buckets; the
  composable Structuring workspace reaches the new arms with **zero bespoke form code**; a
  Playwright real-edge e2e drives ticket → price for one equity and one commodity vanilla.
- **Risk clients:** the GUI Risk/Book views and `CELNET.RISK` surface the new FRTB class capitals
  and the mixed-asset roll-up; `RISK_DIMENSION_UNDERLYING` replaces the FX-pair dimension label.

The **api-first parity gate** (`tools/check-verification-coverage.mjs`, run by `just check`)
mechanically requires, in one change per new arm: the proto oneof field, the golden vector, the
parity row, and the one-line family→test map entry. The lint is never weakened to pass.

---

## 4. Staged GREEN increments (each commit gated; Track A ∥ Track B on worktree lanes)

Workspace compiles + `just check` is green at every commit.

- **S0 (hygiene, do first):** register the two new crate names in `[workspace.dependencies]`;
  confirm `just workspace-deps` green. (Track B prerequisite.)
- **S1 (Track A — fact widening, FX byte-identical):** add `AssetClass` + `UnderlyingRef` to
  `celnet-risk-normalize`; generalize `PositionRisk`/`CanonicalLeaf`; route Greeks by class
  (FX→`celnet-vanilla` only, since the leaves don't exist yet). **Gate:** the existing
  `celnet-risk-normalize` + `celnet-risk-cube` tests + `exotic_risk_cube.rs` +
  `celnet-risk-fleet` FX `firm_aggregate==single-node` all **1e-12 byte-green** (`check-crate`
  each). Commit.
- **S2 (Track B — equity leaf):** `celnet-equity-vanilla` + golden table + golden vector +
  `equity_vanilla.rs` parity row (QuantLib 1e-12 + parity/limit/published gates) +
  `just mutants-gate-equity-vanilla`. **Gate:** parity green; `verification-coverage` green for
  the new arm; mutation kill-rate ≥90%. Commit.
- **S3 (Track B — commodity leaf):** `celnet-commodity-vanilla` + golden table + vector +
  `commodity_vanilla.rs` parity row (QuantLib commodity golden 1e-12 + Black-76 parity/limit
  gates) + `just mutants-gate-commodity-vanilla`. **Gate:** as S2. Commit.
- **S4 (Track A — FRTB cross-asset buckets):** GIRR/equity/commodity/CSR bucket data + the
  factor-projection from `NodeAggregate` → per-class `SbmParams`; `frtb_cross_asset.rs` with the
  longhand per-class oracle + the extended per-class `*_matches_basel_constants` gate (0.75ρ
  re-derived from MAR text per class). **Gate:** parity green; the constant gate disagrees with a
  deliberately-wrong floor (proven by a negative control in the test). Commit.
- **S5 (Track A — mixed-asset roll-up):** wire the equity/commodity facts into the cube's
  non-additive re-pricers (route by `AssetClass`); `cross_asset_risk_cube.rs` (FX no-regression
  1e-12 + mixed-asset firm==single-node 1e-12/1e-9). **Gate:** both rows green; the FX rows
  unchanged. Commit.
- **S6 (clients + reconcile):** proto arms + server routing + validity matrix; SDK/CLI/Excel/GUI
  surfacing (§3); the 5-client conformance harness rows for equity + commodity vanilla green
  against a real edge; generate `CLIENT-PARITY-MATRIX.md`. Reconcile `RISK-HIERARCHY.md`,
  `INTERFACES.md`, `ARCHITECTURE.md` (risk layer now cross-asset; FRTB classes now
  GIRR/equity/commodity/CSR; two new leaves), `ANALYTICS-SPEC.md`. lodestar auto-indexes; run `detect_changes`.
  Full `just check` prints the literal **"All gates passed."** + GUI vitest/Playwright + Excel
  real-edge e2e green. Milestone commit + push.

---

## 5. Gates summary & milestone bar

- **FX no-regression (the binding invariant):** every existing FX risk parity row
  (`exotic_risk_cube.rs`, `celnet-risk-fleet`, `frtb.rs`) stays **byte-green to 1e-12** after the
  `RiskFact` widening — proven, not assumed.
- **New leaves:** QuantLib 1e-12 (equity dividend / commodity) + qualitative gates that **can
  disagree** (parity, limits, published values) + golden vector + ≥90% mutation kill-rate.
- **FRTB cross-asset:** longhand independent oracle per class + per-class BCBS-constant gate
  re-derived from MAR text (the 0.75ρ template), with a negative control proving the gate fails on
  a wrong floor.
- **api-first parity:** `just verification-coverage` green (arm ⇄ vector ⇄ parity-row) for both
  new arms; reachable + identical from all 5 clients against a real edge.
- Full `just check` = literal **"All gates passed."** (verified, not the wrapper exit code).
- Docs reconciled (no stale "FX-only risk layer" claims); lodestar auto-indexes; run `detect_changes`.

---

## 6. Deploy-bound carve-out (ENV) — VERIFICATION-CONTRACT (g)

In-repo W5 proves the **payoff math** (equity/commodity vanilla vs QuantLib), the **convention/
factor identity** (dividend-rho, convenience-rho, FRTB bucket projection), and the **aggregation
algebra** (cross-asset roll-up + SbM capital). The following remain **deploy/live-gated**, designed
+ seamed + ADR'd in-repo, never claimed in-repo (the standing honest boundary, unchanged):

- **Live dividend/repo curves, listed-future settlement prices, commodity convenience-yield
  curves, and equity borrow** are sourced from the live estate at deploy — in-repo uses static
  caller-supplied `CostOfCarry` inputs (the `b` is data, not a feed).
- **FRTB regulatory risk-weight / correlation calibrations** are caller-supplied data
  (`SbmParams`, never compiled in — the existing §2.3/§2.11 discipline); the live regulatory
  parameter set is a deploy-time configuration, not an in-repo constant.
- **Cross-host wire p99 / live JVM CelNet estate / NVIDIA GPU absolutes** — unchanged honest
  boundary; the cross-asset risk roll-up's in-repo proof is the algebra + a host-local relative
  regression, never an absolute cross-host SLO.

Selected at boot via the existing `CELNET_DEPLOY` / `CELNET_FLEET_MODE` knob discipline
(`crates/celnet-server/src/lib.rs`); default standalone behaviour is byte-identical FX.

---

## 7. Open questions (resolve before/within the wave)

- **CSR sourcing:** no in-repo product currently emits CSR (credit-spread) sensitivities. Deliver
  the CSR bucket set + longhand parity row anyway (cross-asset completeness), explicitly flagged
  "no in-repo product sources CSR factors yet" — or defer CSR to W6 as a designed-seam? (Plan
  assumes deliver-with-honest-flag; confirm.)
- **GIRR basis/inflation sub-risks:** include now, or designed-seam to a funded rates wave (W6+
  `rates/rates-vanilla-leaf`)? (Plan assumes designed-seam, vanilla GIRR delta only now.)
- **Equity-leaf adjoint:** `GreekEngine::Adjoint` is GK-graph-specific. Add an equity AAD now (for
  the canonical-leaf scale path) or use analytic equity Greeks until W6 rigor uplift? (Plan assumes
  analytic-only for non-FX leaves in W5.)
- **`Underlying::Commodity` vs `Underlying::Listed`:** does W1 reserve a dedicated `Commodity` arm,
  or do listed-future options reuse `Listed(ContractRef)` with a commodity tag? Confirm the W1 arm
  set so Track B adds the right additive arm.
- **Multiplier/contract-size:** equity index and commodity options carry a contract multiplier;
  confirm whether it lives on `Underlying` (identity) or `terms` (instrument) so the golden vectors
  and validity matrix are consistent across clients.
