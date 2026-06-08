# W2 — FX linear products + pair/metals breadth: staged execution plan

> Wave 2 of `docs/MASTER-EVOLUTION-PROGRAM.md` §5. **Assumes the W1 core contract is in
> place**: `celnet_types::{Underlying (Fx arm only), Carry, RateSensitivities}` on the wire,
> a Carry-based pricing trait in `celnet-core`, and `celnet-vanilla` as the FX (GK) leaf.
> This doc is the execution plan; the single backlog truth stays `docs/WORLD-CLASS-BACKLOG.md`
> (items `[W2] linear/fx-forward-swap-ndf`, `[W2] conventions/metals-breadth`,
> `[W2] conventions/pair-universe-superset`). Every gate is the per-product set in
> `docs/VERIFICATION-CONTRACT.md` (a)–(g). FX-default clients per ADR-0008.

## 0. Scope, two tracks, and the crux

W2 is **P0/P2 breadth — the highest-value, lowest-risk wave after the core**: it adds the
linear book (forward / swap / NDF as *priced* products, not just `forward()` math) and grows
the convention/calendar universe past SynOption's 75-pair panel, including the platinum-group
metals and metal crosses. It introduces **no new pricing model** beyond closed-form
discounted cashflows and reuses the carry seam W1 established.

Two disjoint tracks (separate crates / dir trees ⇒ parallel git-worktree lanes), with **one
shared contract touch** that must land first and serialize the two lanes at that single point:

- **Track A — `celnet-linear` (NEW crate):** FX outright forward, FX swap (near + far legs),
  and NDF as first-class priced products. New proto payoff arms `fx_forward` / `fx_swap` /
  `ndf`. Oracle: QuantLib `FxForward` PV + an independent closed-form discount-factor
  derivation; NDF `(F − K) · notional · df` hand-derived. Cross-client parity (all 5).
- **Track B — pair/metals breadth (no new crate):** grow `celnet-conventions` /
  `celnet-calendar` / `celnet-types` to a **>75-pair** superset + **XPT/XPD** + metal crosses
  (XAUEUR/XAUJPY/XAGEUR/…), loco-London T+2 with a **lease-rate** carry. Needs an additive
  `Underlying::Metal` arm. Oracle: EMTA/ISDA/LBMA published tables + an independent rata-die /
  holiday-walk + QuantLib GK with lease = foreign-rate.

**The crux risk (Track A):** the FX forward/swap/NDF are *linear* in spot — a forward's PV
is a discounted-cashflow identity, NOT an option payoff — so the danger is a **circular
oracle** that re-derives the same `F = S·e^{(r_d−r_f)t}` the production code uses. The
discipline (per the FRTB 0.75ρ lesson, VERIFICATION-CONTRACT §a) is to reach the reference by
an **independent route**: QuantLib's `FxForward`/`DiscountingFxForwardEngine` (a genuinely
different implementation), plus a **structural identity** (a forward struck at the fair
forward has PV 0; a forward = long discount-bond − short the other; put−call parity of the
two synthetic positions) that the production algebra cannot satisfy if it is wrong.

**The crux risk (Track B):** the spot-date oracle must not reuse the library's `time`-crate
date engine — `pair_universe.rs` already solves this with an in-test Hinnant rata-die +
re-derived holiday computus. We **extend that independent oracle**, never copy the library's
calendar into it.

## 1. Track A — `celnet-linear` crate layout

New leaf crate, deps `{celnet-core, celnet-types}` (acyclic; same shape as `celnet-qmc`):

```
crates/celnet-linear/
  Cargo.toml          # deps via `.workspace = true` ONLY (no internal path-deps —
                      #   the `just workspace-deps` lint from W0 bans path-deps outside
                      #   the [workspace.dependencies] registry)
  src/lib.rs          # crate doc + honest-boundary banner + re-exports
  src/forward.rs      # FX outright forward: PV, fair-forward, the linear Greeks
  src/swap.rs         # FX swap = near leg + far leg (two outright forwards, opposite sign)
  src/ndf.rs          # non-deliverable forward: (F − K)·notional·df in settlement ccy
  src/inputs.rs       # LinearInputs { underlying, notional, strike(=contract rate),
                      #   near_settle_t, far_settle_t?, side } reading Carry::forward/discount
```

**Auto-join + registry (mechanics):** the workspace uses `members = ["crates/*"]`, so the
new crate auto-joins. Add **one line** to the root `[workspace.dependencies]`
(`celnet-linear = { path = "crates/celnet-linear" }`, matching the 28 existing entries in
`Cargo.toml`) and depend from `celnet-server` / `celnet-client` / `celnet-cli` /
`celnet-parity` / `celnet-golden` via `celnet-linear.workspace = true`. The
`just workspace-deps` lint (W0, `[W0] arch/workspace-dep-registry`) then enforces no internal
path-dep escapes the registry.

**The no-workaround test (ADR-0008):** `celnet-linear` pricers call
`carry.forward_factor(t)` / `carry.discount_df(t)` and **never** `match carry { FxRates =>
… }`. A forward's PV is `side · notional · df_settle · (forward_rate − strike)` where
`forward_rate = spot · carry.forward_factor(t)`; this is asset-class-agnostic, so an NDF on a
(future) crypto or metal underlying reuses the identical engine. Any branch on the `Carry`
variant in the linear pricer is a review blocker.

**Why a new crate, not an arm of `celnet-vanilla`:** these are not Garman-Kohlhagen option
payoffs; folding them into the FX option leaf would couple linear DCF math to the GK leaf and
violate the agnostic-payoff layering. A dedicated leaf keeps the lane disjoint and lets W3+
crypto/metal linear products reuse it unchanged.

## 2. Track A — proto contract (new payoff SHAPES)

Per MASTER §3, new payoff shapes get **new oneof arms** (existing arms generalize over
`Underlying`; forwards are a genuinely new shape). Append to `oneof product` in
`message Instrument` (current arms run to field 25; use the **next free tags**, noting 22 is
already retired/skipped — verify against `celnet.proto` at edit time and reuse no number):

```proto
oneof product {
  // … existing 7..25 …
  FxForward fx_forward = 26;   // outright forward: fair-fwd + PV at a contract rate
  FxSwap    fx_swap    = 27;   // near + far legs (two outrights, opposite direction)
  Ndf       ndf        = 28;   // non-deliverable forward, cash-settled in the convertible ccy
}

message FxForward { double contract_rate = 1; double notional = 2; Side side = 3; }
message FxSwap    { FxForward near = 1; FxForward far = 2; }   // far.side opposite near.side
message Ndf       { double contract_rate = 1; double notional = 2; Side side = 3;
                    FixingSource fixing = 4; Ccy settlement_ccy = 5; }
```

- `Side` reuses the existing buy/sell enum if present, else a small additive `Side` enum
  (use the `protobuf` skill for enum-prefix hygiene — `SIDE_UNSPECIFIED = 0`).
- The PV path emits a `PriceResponse`; closed-form ⇒ `price_std_error = null`/absent (these
  are exact, not MC). The `Underlying` + `CarryModel` already on `MarketContext`/`Instrument`
  (W1) carry spot/carry; the linear arms add only the contract rate, notional, side and (NDF)
  the fixing identity + settlement currency.
- **Server product × underlying validity matrix (W1 seam):** `fx_forward`/`fx_swap` valid for
  `Underlying::Fx` (deliverable); `ndf` valid only for a **non-deliverable** underlying (the
  registry's `Settlement::NonDeliverable`). An `ndf` on a deliverable pair ⇒
  `INVALID_ARGUMENT`, never a silent fallback.

Gate every proto step with a convert round-trip + `to_bits` on the PV (the existing
`celnet-proto` codec test pattern).

## 3. Track A — the INDEPENDENT oracle (per product)

`crates/celnet-parity/tests/linear.rs` (one new file). Each oracle is reached by a route
disjoint from the production DCF algebra; at least one is a structural/limit gate that **can
disagree**.

- **FX outright forward.**
  - **Primary:** QuantLib `FxForward` PV via the frozen golden table
    (`crates/celnet-golden/` — add `fx_forward.csv` generated by `gen_vectors.rs`, gated by
    `vectors_selfcheck.rs`), tolerance ~1e-10 closed-form-to-closed-form.
  - **Structural (can disagree):** a forward struck **at the fair forward** has PV exactly 0
    (to_bits 0.0 after sign normalization); PV is **linear** in notional (double notional ⇒
    double PV, to ~1e-12); a long-forward + short-forward at the same rate net to 0. These
    catch a forward/discount/sign slip the QuantLib row alone could share.
  - **Limit:** `t → 0` ⇒ PV `→ side · notional · (spot − strike)` (no discounting), a
    different expression from the discounted form.
  - **Circular-oracle risk: HIGH** — both the production code and a naive "second
    implementation" compute `S·e^{(r_d−r_f)t}`. Mitigation: the QuantLib engine is a genuinely
    independent codebase, AND the fair-forward-⇒-PV-0 + linearity gates do not reuse the
    forward expression. Re-derive the carry-factor identity from the covered-interest-parity
    primary statement in the test comment, not from the impl.

- **FX swap (near + far).**
  - **Oracle:** the swap PV == the **independent sum** of two separately-priced outright
    forwards (near long, far short) — but the *swap points* (far rate − near rate) are
    cross-checked against the QuantLib forward-points identity, an independent route.
  - **Structural:** a swap with near == far date and opposite sides nets to 0; the swap-point
    sign matches the carry sign (positive carry ⇒ premium/discount direction). 
  - **Circular-oracle risk: MEDIUM** — the two-leg sum shares the forward formula with the
    leg pricer, so the **QuantLib forward-points** row is the genuinely-independent anchor;
    the netting/sign gates can disagree.

- **NDF.**
  - **Oracle:** **hand-derived** `PV = side · notional · df_settle · (F − K)` where
    `F = spot · forward_factor(t)`, `df_settle = discount_df(t)` in the **settlement
    (convertible) currency**, pinned as a literal in the test with the derivation in the
    comment (VERIFICATION-CONTRACT §a class 3, published/hand-derived). Cross-checked against
    a deliverable forward of equal terms: an NDF and a deliverable forward have the **same PV**
    when settled in the same numeraire (the non-deliverability changes settlement mechanics,
    not the risk-neutral PV) — an independent structural identity.
  - **Settlement identity:** the cash settlement is in the convertible leg (USD for the
    covered USD/EM pairs) at the named `FixingSource`; the test asserts the
    settlement-currency discounting uses the convertible-leg rate, not the restricted leg.
  - **Circular-oracle risk: MEDIUM** — the hand-derived literal and the "NDF == deliverable
    fwd PV" identity are independent of each other (one is an absolute value, the other a
    structural equality), so a shared slip cannot hide.

**Honest boundary (VERIFICATION-CONTRACT §g, verbatim in `linear.rs` + crate docs):** the
in-repo proof is the **payoff/DCF math + convention identity**. **Live NDF fixing VALUES are
ENV** — only the fixing *identity* (`FixingSource`) and settlement convention are in-repo; the
realized fixing rate is an estate-gated feed, never sourced here.

## 4. Track B — pair universe → >75 + XPT/XPD + metal crosses

### 4.1 `Underlying::Metal` arm (the one additive type change)

XAU/XAG already resolve as `CcyPair` with `InstrumentClass::PreciousMetal` in
`celnet-conventions`. W2 promotes metals to a **first-class `Underlying` arm** so the carry is
a **lease rate**, not a fiat foreign rate, and so XPT/XPD and metal crosses are nameable:

```rust
// celnet-types — additive enum growth (one contract, no versioning, ADR-0008 §Identity).
pub enum Underlying {
    Fx(CcyPair),
    Metal(MetalPair),   // NEW: metal as the asset leg vs a fiat quote ccy
}
pub struct MetalPair { pub metal: Metal, pub quote: Ccy }   // metal is always the base/asset
pub enum Metal { Gold, Silver, Platinum, Palladium }        // XAU/XAG/XPT/XPD
```

- **Carry:** the metal leg carries a **lease rate** in place of a foreign deposit rate. Per
  ADR-0008 the carry is a `forward()`/`discount()` producer: `Carry::FxRates { r_dom = quote
  rate, r_for = lease_rate }` reproduces the existing XAUUSD path **byte-identically** (lease
  modelled as the foreign rate — exactly what `InstrumentClass::PreciousMetal`'s doc already
  states), so **no payoff-engine edit** and the GK leaf prices a metal option unchanged. The
  *naming* (lease vs foreign rate) is a risk-layer/convention concern, not a math change ⇒ FX
  byte-identity preserved by construction.
- **Wire:** add the `metal` arm to the `Underlying` oneof (`MetalPair { Metal metal = 1; Ccy
  quote = 2; }`, `Metal` enum). The codec maps a metal `CcyPair` (XAU/XAG/XPT/XPD base) into
  `Underlying.metal` — but **existing XAUUSD/XAGUSD `CcyPair` flows must stay byte-equivalent**
  (W1 byte-identity gate extended to the metal projection). Keep `MetalPair → CcyPair`
  conversion so `celnet-conventions`/`celnet-calendar` (which key on `CcyPair`) are untouched.

### 4.2 Conventions + calendar breadth

- **`celnet-conventions/src/registry.rs`:** grow `is_covered_key` + `profile_for_canonical`
  from 19 to a **>75-pair** superset (G10 majors + all major crosses + the EM deliverable +
  EM NDF panel matching/exceeding SynOption's 75). Add **XPT/XPD vs USD** (mirror the
  XAU/XAG `PreciousMetal` profile) and **metal crosses** (XAUEUR, XAUJPY, XAGEUR, XPTUSD,
  XPDUSD …) — metal as base, loco-London settlement, DNS ATM, premium in the quote ccy
  (unadjusted), NY cut, T+2.
- **`celnet-calendar`:** loco-London metals settle on the **London (UK) calendar** (already
  modelled — `CentreId::UnitedKingdom`); metal crosses intersect London ∩ the quote-ccy
  centre (∩ USD for a non-USD cross). Add any new fiat centres needed for the >75 panel whose
  holidays are **fully Gregorian-computable**; for currencies whose onshore calendar is
  **lunisolar/not modellable**, set `has_calendar_support = false` honestly (the existing
  NDF discipline) rather than fake a calendar.
- **Lease rate:** documented in the metal `PairMeta` (a `lease_rate`-bearing convention field
  or the existing accrual-leg slot repurposed for the metal leg), with the carry mapping
  above. **Live lease-rate VALUES are ENV** — only the convention (which leg carries the
  lease, day-count, loco) is in-repo.

### 4.3 Track B — the INDEPENDENT oracle

Extend `crates/celnet-parity/tests/pair_universe.rs` (the existing file — it already carries
the Hinnant rata-die + computus oracle and the `PUBLISHED` table pattern):

- **Resolved conventions vs published tables.** Grow the `PUBLISHED` table to the full >75 +
  XPT/XPD + metal-cross set, each row an **EMTA/ISDA (FX) or LBMA (metals)** market-standard
  literal. The published values are the oracle, not a copy of the code.
- **Algorithmic spot date vs independent walk.** Extend `holiday_for` / `legs_for` to the new
  Gregorian-computable centres + the metal-cross London∩quote∩USD leg logic; sweep every
  (pair, trade-date) over 2024–2025. The independent Hinnant walk must equal the library to
  the day. **Circular-oracle risk: LOW** — the oracle re-derives holidays from primary
  national/LBMA rules and uses a *different* date engine; `independent_date_engine_is_correct`
  pins the rata-die to known anchors so two-broken-implementations-agree cannot occur.
- **QuantLib metal cross-check.** Add a `celnet-golden` row pricing a metal **vanilla** with
  lease = foreign rate via QuantLib GK, asserting the metal-as-`Underlying::Metal` path is
  to_bits-identical to the metal-as-`CcyPair` path (the byte-identity gate for the new arm).
- **Structural invariants.** Every metal carries metal-as-base + quote-ccy premium
  (unadjusted); metal crosses are loco-London T+2; NDF invariants hold across the grown panel;
  orientation-inversion stays non-contradictory (the existing `orientation_inversion_*` test
  generalizes).

**Circular-oracle flag (carry-naming):** because the metal lease rate is *modelled as the FX
foreign rate*, do **not** let the metal price oracle reuse the production carry assembly —
the QuantLib GK row (independent engine) is the anchor; the `to_bits` metal==CcyPair gate is a
byte-identity check, not a correctness oracle.

## 5. Staged GREEN increments (each commit gated; lanes serialize only at the proto touch)

Per MASTER §2 the *contract edit* serializes; incremental green commits do not. Stage so the
workspace compiles + `just check` is green at every commit.

| # | Step | Gate |
|---|------|------|
| S0 | Track B: add `Underlying::Metal`/`MetalPair`/`Metal` to `celnet-types` **alongside** Fx; `MetalPair↔CcyPair` conversions; unit tests incl. `to_bits` XAUUSD round-trip vs the FX path. | `just check-crate celnet-types` green; metal `CcyPair` projection byte-identical. |
| S1 | Track B: grow `celnet-conventions` registry + `celnet-calendar` centres to the >75 + XPT/XPD + metal-cross superset (no wire change yet). | `check-crate celnet-conventions`/`celnet-calendar`; `pair_universe.rs` extended `PUBLISHED` + independent-walk sweep green. |
| S2 | **Proto touch (serializes both lanes):** add `Underlying.metal` arm + `fx_forward`/`fx_swap`/`ndf` product arms + messages + `Side`; codec maps legacy metal pair → `Underlying.metal`, FX/metal `CcyPair` flows byte-equivalent. `protobuf` skill for hygiene. | convert round-trip + `to_bits` PV/projection byte-identity; W0 5-client conformance corpus stays green. |
| S3 | Track A: `celnet-linear` crate (forward/swap/NDF) reading the W1 carry seam; server pricer arms behind the product×underlying validity matrix; WS-JSON mirror. | `check-crate celnet-linear`/`celnet-server`; `linear.rs` parity rows (QuantLib + structural + limit) green; invalid combo ⇒ `INVALID_ARGUMENT`. |
| S4 | Golden vectors + coverage lint: add `crates/celnet-golden/vectors/{fx_forward,fx_swap,ndf}.json` (engine-generated, oracle-cross-checked) + the metal-vanilla `to_bits` row; add the **three** `FAMILY_TO_PARITY_FILE` map entries (`fx_forward`/`fx_swap`/`ndf` → `linear.rs`) in `tools/check-verification-coverage.mjs`. | `just verification-coverage` green at 21/21 arms (was 18); `vectors_selfcheck.rs` green. |
| S5 | Cross-client surfacing (all 5), FX-default preserved (see §6). | SDK/CLI/Excel/GUI build + tests; **real-edge** conformance: forward/swap/NDF + a metal-cross price identical server==SDK==CLI==Excel==GUI vs a freshly-booted `demo_edge`. |
| S6 | Delete legacy + reconcile (#10): retire any metal-as-FX-pair special-casing now superseded by `Underlying::Metal`; re-index codebase-memory; reconcile INTERFACES/CONVENTIONS/ANALYTICS-SPEC docs (metals breadth + linear book → Built, with citations); update `CLIENT-PARITY-MATRIX.md` (generated from the harness). | full `just check` prints literal **"All gates passed."**; all 5 client suites + conformance green; `verification-coverage` 21/21. Milestone commit + push. |

## 6. Cross-client surfacing (FX default preserved — ADR-0008 §intuitive clients)

Every capability lands in all 5 clients in lockstep (api-first parity); FX stays the
zero-friction default and the asset-class/product selector is purely additive.

- **SDK (`celnet-client`):** typed builders `Instrument::fx_forward(underlying).rate(k)
  .notional(n).side(…)`, `::fx_swap(near, far)`, `::ndf(underlying).rate(k).fixing(…)`; an
  `Underlying::metal(Metal::Platinum, Ccy::USD)` constructor alongside the FX-pair default.
  One runnable example per product gated against a real edge (the existing SDK e2e pattern).
- **CLI (`celnet-cli`):** new `forward` / `swap` / `ndf` subcommands (sibling to `exotic`),
  underlying parsed FX-pair-first so `celnet forward EURUSD --rate … --notional …` works with
  no asset-class flag; `--metal XPTUSD` and metal crosses resolve via the grown registry.
  CLI == SDK == server proven against a real edge (the existing CLI conformance pattern).
- **Excel:** additive `CELNET.FORWARD` / `CELNET.SWAP` / `CELNET.NDF` (or, if W1 already
  delivered the polymorphic `CELNET.PRICE(underlying, product, terms)`, route these as new
  product tokens — prefer that, retiring per-product fns at parity per #10). Real-edge Excel
  conformance suite extended.
- **GUI:** the Ticket/Structuring workspace gains a forward/swap/NDF leg type; the
  UniverseNavigator surfaces XPT/XPD + metal crosses under a Metals bucket; FX remains the
  default landing universe. Playwright real-edge e2e: navigate → structure a forward →
  price → (metal cross) price, GUI price == server, plus axe a11y.

## 7. Gates summary (VERIFICATION-CONTRACT (a)–(g), per new product)

- **(a) independent oracle that can disagree:** QuantLib `FxForward` + structural
  (fair-fwd⇒PV0, linearity, netting) + limits; NDF hand-derived + NDF==deliverable-fwd-PV;
  metals QuantLib GK + Hinnant walk. Circular-oracle risks flagged §3/§4.3.
- **(b) frozen reference:** `crates/celnet-golden/vectors/{fx_forward,fx_swap,ndf}.json` +
  `fx_forward.csv` table; closed-form ⇒ tight ~1e-10 tolerance, `price_std_error = null`.
- **(c) cross-client golden vector:** the three new vector files; `verification-coverage`
  enforces arm ⇄ vector ⇄ parity-row at 21/21.
- **(d) cross-client conformance:** server == SDK == CLI == Excel == GUI vs a real edge;
  `CLIENT-PARITY-MATRIX.md` rows generated from the harness.
- **(e) performance:** linear PV is a handful of `exp`/multiply ops — well inside the §1.2
  in-core budget; assert no regression on `core_load`/`bench_gate` (state "trivially within
  budget" in the parity module doc). Not a new budgeted hot path.
- **(f) fuzz/mutation:** `celnet-linear` is numeric-core ⇒ add a `mutants-gate-linear`
  (≥90% non-equivalent kill-rate, audited equivalence set). The new proto arms add to the
  existing wire/proto fuzz target (no new untrusted-byte decoder — the codec is shared);
  state explicitly in the parity module doc that no new byte decoder is introduced.
- **(g) deploy-bound scope statement:** verbatim honest boundary — **live NDF/metal fixing
  and lease-rate VALUES are ENV**; only fixing identity + convention are in-repo.

## 8. Out of W2 scope (later waves / ENV)
Crypto linear/funding (W3 reuses `celnet-linear` unchanged), equity/commodity leaves (W5),
multi-dealer RFQ for the linear book (W4). ENV: live fixing/lease VALUES, regulated-venue
status. Onshore lunisolar NDF calendars stay honestly `has_calendar_support = false`.

## 9. Open questions (resolve at implementation)
- Confirm the **next free proto field numbers** (22 appears skipped; 25 is the last used) and
  whether a `Side` enum already exists on the wire before adding one.
- Whether W1 already shipped the polymorphic `CELNET.PRICE(underlying, product, terms)` Excel
  surface — if so, route the linear products through it rather than adding three new fns.
- Whether the lease rate should be a **distinct named convention field** on the metal
  `PairMeta` or ride the existing foreign-accrual-leg slot (favour an explicit named field for
  honest risk reporting, but it must keep the metal-vanilla `to_bits` FX-byte-identity gate).
- Exact target panel for ">75": the precise EMTA/ISDA pair list to encode (G10 + all crosses
  + the EM deliverable/NDF set) so the count provably exceeds 75 with published-table backing.
