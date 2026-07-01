# ADR-0012 — One unified generalized-Black-Scholes-Merton (gBSM) carry kernel

- **Status:** Accepted (core landed on branch `integ/gbsm-kernel`; full-workspace
  byte-identity restoration is a follow-up — see §6 "Blast radius").
- **Date:** 2026-06-30
- **Supersedes/extends:** ADR-0008 (multi-asset carry architecture — the `Carry`
  seam and `RateSensitivities` this kernel produces).

## 1. Context

Four leaf crates each carried their own copy of the generalized-Black-Scholes-Merton
closed form and its full first-/higher-order Greek strip (~100 lines each, ~265
duplicated total), differing **only** in how the net cost-of-carry `b` and discount
rate `r` are sourced:

- `celnet-vanilla` (FX / Garman-Kohlhagen): `b = r_dom − r_for`, `r = r_dom`.
- `celnet-equity-vanilla`: `b = r − q − repo`.
- `celnet-commodity-vanilla` (Black-76): `b = r − convenience` (or `b = 0` on a
  listed future).
- `celnet-crypto-vanilla` (linear/USDT-margined arm): `b = r − funding`.

Critically, the four were **mathematically identical but not bit-identical**: they
used three different IEEE-754 float reductions of the same model —

- **forward-space** (commodity): `d1 = [ln(F/K) + ½σ²t]/σ√t`, `F = S·e^{bt}`,
  `price = df·(F·Φ(d1) − K·Φ(d2))`;
- **single-exp spot-space** (equity/FX): `d1 = [ln(S/K) + (b+½σ²)t]/σ√t`,
  `price = S·e^{(b−r)t}·Φ(d1) − K·e^{−rt}·Φ(d2)`;
- **FX-equivalent reconstruction** (crypto linear): a spot-space form that recovers
  `r_for = r − b` so it reproduced the FX leaf bit-for-bit.

A prior investigation established that **no single `(b,r)` kernel can be bit-identical
to all leaves simultaneously**, because each leaf's `greeks_price_is_bit_identical_to_price`
gate ties its `greeks().price` to its own `price()` float form, and the crypto leaf
was additionally pinned bit-for-bit to the FX leaf. A truly single kernel therefore
requires accepting a **sub-1e-12 behavioural change** on the leaves that do not match
the chosen canonical form. The operator authorised that change (full behavioural
unification on the canonical **forward-space** kernel — QuantLib's single
`blackFormula` on the forward; Haug's one-formula / `b`-table generalized-BSM).

## 2. Decision

Extract **one** canonical forward-space kernel into `celnet-core`:

```rust
// crates/celnet-core/src/carry.rs
pub fn gbsm_carry_price (opt, b, r, spot, strike, vol, t) -> f64;
pub fn gbsm_carry_greeks(opt, b, r, spot, strike, vol, t) -> CarryGreeks;
pub fn carry_greeks_to_greeks(cg: &CarryGreeks) -> Greeks; // FX two-rho projection
```

`F = S·e^{bt}`, `df = e^{−rt}`, forward-space `d1/d2`, and the full desk strip in one
pass. The forward/discount read the `Carry` seam (`forward_factor`/`discount_df`)
exactly as the commodity (Black-76) leaf did, so **that leaf is byte-for-byte
unchanged**. The rate sensitivities are the carry-natural
`RateSensitivities::Carry { discount_rho = ∂V/∂r, carry_rho = ∂V/∂b }`.

Each leaf's `price`/`greeks` became a ~15-line adapter that assembles its `b` and
calls the kernel:

- **commodity** → `b = carry_rate()`, `r = discount_rate()` (its carry is already
  canonical; output byte-for-byte unchanged).
- **equity** → `b = r − q − repo`; maps `CarryGreeks → EquityGreeks`.
- **crypto linear** → `b = carry_rate()`; the `fx_equiv_rates` detour is deleted.
- **FX (Garman-Kohlhagen)** → `b = r_dom − r_for`, `r = r_dom`; **only the core gBSM
  math unifies** — the FX leaf keeps its **two-rate `rho_dom`/`rho_for` output
  basis** by projecting the kernel's carry rho pair through
  `carry_greeks_to_greeks` (`rho_dom = discount_rho + carry_rho`,
  `rho_for = −carry_rho`, the lossless `RateSensitivities::flat_rhos` bijection).

### `delta_forward` convention (the one non-rounding subtlety)

The leaves reported **two different** "forward deltas": commodity/crypto the
*discounted* `∂V/∂F = df·Φ(±d1)`; equity/FX the *driftless* `∂V_fwd/∂F = Φ(±d1)`
(the FX-desk forward delta, load-bearing for the delta-convention machinery). These
differ by a full `df` factor — **not** a rounding difference. The kernel emits the
driftless `Φ(±d1)`; equity/FX pass it through verbatim, while commodity/crypto scale
by `df` in their adapter. Because `Φ·df` is bit-identical to the former `df·Φ`
(IEEE multiplication commutes), commodity stays byte-for-byte unchanged.

## 3. SOTA basis

- **QuantLib** (`blackFormula`): a single forward-space Black closed form serves every
  cost-of-carry asset class; the asset class only supplies the forward and discount.
- **Haug, *The Complete Guide to Option Pricing Formulas* (2nd ed.):** the
  generalized-BSM "one formula, four `b`" table (`b = r` equity no-div, `b = r − q`
  equity with yield, `b = 0` Black-76 future, `b = r_d − r_f` FX). This kernel is that
  table with `b` supplied by the leaf.

## 4. The accepted behavioural change

The change is a **sub-1e-12 reassociation** of the same model (forward-space vs
spot-space float order). Every **independent-oracle** and **≤1e-12 parity** gate holds:

- QuantLib golden grid (`celnet-golden/tests/vanilla_grid.rs`, 3200 rows): price rel
  1e-11, delta 1e-11, rhos 1e-10 — all pass (the reroute perturbs ~1e-16 relative).
- `vectors_selfcheck` re-derives from an independent oracle (untouched by the leaf).
- Each leaf's hand-pinned oracle (Hull index option, Black-76 published value,
  put-call parity at 1e-12, `q=0` standard-BSM limit at 1e-12) — all pass.

## 5. Bit-identity tests relaxed / regenerated (minimum set, never below 1e-12)

These pinned **determinism / cross-path parity**, not correctness (the 1e-12 oracle
is the correctness bar):

| Test | Was | Now | Why |
|---|---|---|---|
| `celnet-crypto-vanilla` `funding_maps_to_fx_foreign_rate_bit_identical` → `…_within_1e12` | `to_bits` == FX leaf | `assert_close` 1e-12 | crypto passes `b` directly, FX reconstructs `b = r_dom − r_for` = `r − (r − b)` (not bit-equal to `b`); residual ~1e-15. |
| `celnet-vanilla` `adjoint::aad_price_bit_identical` → `…_matches_within_1e12` | `to_bits` == `price()` | `assert_close` 1e-12 | the AAD tape is a genuine spot-space GK graph; `price()` is now forward-space. Same model, two arrangements; `aad_matches_analytic` + the FD oracle remain exact-to-tolerance. |
| `celnet-parity` `determinism::…GOLDEN_VANILLA` | frozen bits | **regenerated** (still exact) | a frozen cross-run/build ULP-stability baseline; regenerated (not relaxed) to the new deterministic output — sanctioned by the table's own "regenerate intentionally when a numerical change is deliberate" note. Price value unchanged to 1e-10. |

Intra-leaf `greeks_price_is_bit_identical_to_price` **stays exact** everywhere (both
`price` and `greeks` now flow through the kernel). The FX `pricer.rs`
trait-vs-direct byte-identity **stays exact** (it delegates to `price`/`greeks`).

## 6. Full-workspace integration (completed)

The FX-leaf reroute is a sub-1e-12 change to `celnet_vanilla::price`/`greeks`, consumed
workspace-wide. The unification was carried through the whole affected surface:

### 6.1 The third duplicate gBSM — `celnet-exotics`

`celnet-exotics` carried **its own** spot-space gBSM, `inputs::carry_vanilla_price_at`
(the vanilla component of the geometric-Asian control variates, forward-start/cliquet
legs, variance/vol-swap replication strips, and quanto closed forms). It is now routed
through the **same** `gbsm_carry_price` kernel (`b = carry_rate()`, `r = discount_rate()`).
Consequences, all verified:

- `inputs::carry_vanilla_byte_identical_to_fx_forms` — the **genuine `Carry::FxRates`**
  assert is **RESTORED bit-for-bit** (both sides assemble `b = r_dom − r_for` and route
  the one kernel — the ADR-0008 invariant re-established on the unified kernel). The
  **synthetic `CostOfCarry`** assert relaxes to **1e-12** (kernel takes `b` directly; the
  legacy recast reconstructs `r − (r − b)`, the `funding_maps` class; residual ~1e-15).
- `quanto::…drift_adjustment_matches_adjusted_rate_vanilla` — **RESTORED bit-for-bit**
  (the quanto adjustment builds an `FxRates` carry, so both paths assemble a
  bit-identical `b` and route the same kernel).
- `inputs::carry_vanilla_at_matches_materialized_bitwise` — **stays exact**.
- `celnet-exotics/tests/fx_byte_identity.rs` — the engines that price through
  `carry_vanilla_price` (analytic + MC geometric Asian, forward-start/cliquet, quanto,
  variance/vol-swap) were **re-baselined** (18 frozen `gate` literals across 5 test fns,
  drift 2–9 ULP — a deliberate sub-1e-12 reassociation). The byte-identity **property is
  preserved** (carry path and FX leaf now share the kernel), just at the new bits. The
  other engines (MC barrier, accumulator, TARF, pivot, lookback, PDE, American, basket,
  LSV) price off the `Carry` seam directly and are **byte-for-byte unchanged**.

### 6.2 `celnet-surface` frozen calibration pins (regenerated)

The FX delta→strike calibration shifts sub-1e-12, and the (flat-optimum) least-squares
smile fits amplify that into a small converged-parameter move — **fit quality preserved**
(the reprice residuals are unchanged to ~9 significant figures). Regenerated as a
deliberate sub-1e-12 rebaseline (correctness unaffected — no oracle loosened):

- `tests/fit_pins.rs`: `BENIGN_SV`/`STRESSED_SV` (SABR α, ρ, ν) + the six parametric
  `FrozenSlice`s (SVI/SSVI/ESSVI × benign/stressed) + `STRESSED_MAX_ANCHOR_ERR`. The
  `converged_cost_is_pinned` `≤ frozen + 1e-12` bounds still hold.
- `tests/fx_fit_pin.rs`: the `PINS` table (50 = 5 models × 2 quote sets × 5 strikes),
  re-pinned via its own sanctioned print-then-paste aid.

### 6.3 `celnet-xva` exposure-simulation frozen pins (regenerated)

`celnet-xva` is **not** in the kernel diff, but `NettedTrade::mark`
(`crates/celnet-xva/src/netting.rs`) prices each netting-set trade through
`celnet_vanilla::price`, so the FX-leaf spot→forward re-association propagates into every
exposure quantity the Monte-Carlo profile derives from those marks. The XVA/survival/
netting oracles (CVA/DVA/FVA vs independent quadrature, the from-scratch two-rate mark
re-derivation, survival identities) are **unchanged and green** — none was loosened.
Regenerated as a deliberate sub-1e-12 rebaseline:

- `tests/closed_form_oracle.rs`: the four exposure frozen-bits arrays
  (`MIXED_EPE_BITS`/`MIXED_ENE_BITS`/`NET_SHORT_ENE_BITS`; the all-zero
  `NET_SHORT_EPE_BITS` and the matured `⇒ 0.0` last-node pins are **unchanged**) plus the
  human-readable decimals. Drift 1–25 ULP (≤ 3e-15 rel). The two **deterministic** node-0
  marks (`net_value(0,·).max(0)`) were **re-validated ≤1e-12** against the test's own
  independent from-scratch two-rate closed form (`vanilla_ref`, raw `std`/`erfc`, never
  `celnet_vanilla`): MIXED epe node 0 rel 2.2e-15, NET_SHORT ene node 0 rel 1.1e-15. The
  **Monte-Carlo** median pins (MIXED node 4, NET_SHORT node 3 — no closed form) drifted
  only ≤ 3e-16 rel and `exposure_simulation_is_bit_reproducible` still passes, so they
  remain regression/mutation guards whose correctness is carried by the oracle-validated
  mark + averaging logic.
- `netting_fuzz.rs` carries **no** frozen bit pins (property/invariant assertions only) —
  unchanged and green.

### 6.4 Unchanged / green

`celnet-engine`, `celnet-risk-cube`, `celnet-cli`, `celnet-gpu` pass
unchanged (within-path routing, independent oracles, and tolerance gates). Every
independent-oracle + correctness gate holds at ≤1e-12 across the whole surface. The
**only** relaxations to 1e-12 are the genuinely-different-parameterisation bit-pins
(`funding_maps`, the FX-adjoint, the exotics synthetic-`CostOfCarry` tie); every pin that
the unified kernel makes params match is **restored exact**.

## 7. What stays distinct (deliberately NOT unified)

- **crypto-vanilla INVERSE arm** — coin-margined `1/S_T` measure-change payoff (a
  genuinely different closed form).
- **celnet-linear** — linear FX products (forward/swap/NDF) priced by DCF, not gBSM.
- **celnet-rfq** — multi-dealer RFQ ranking, no option math.
- **FX leaf's two-rate output basis** — FX still reports `rho_dom`/`rho_for`; only its
  *core* gBSM computation unifies.
