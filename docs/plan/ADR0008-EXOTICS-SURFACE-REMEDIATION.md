# ADR-0008 Remediation — `celnet-exotics` & `celnet-surface` onto the Agnostic Carry Seam

**Status:** implementation-ready plan (read-only design pass; no `cargo`/git run — compute-courtesy lane).
**Source tree:** `/Users/adrian/code/celnet-coord` @ `0f8933b` (clean worktree off `origin/main`).
**Inputs read:** `docs/AUDIT-ADR0008-CONFORMANCE.md`; `crates/celnet-core/src/carry.rs`; `crates/celnet-types/src/lib.rs` (`Carry`/`VanillaInputs`); the ~30 drift sites across `crates/celnet-exotics/src/*`; `crates/celnet-surface/src/quotes.rs`; `crates/celnet-proto/proto/celnet.proto` (`MarketContext`); the golden vector grid (`crates/celnet-golden/vectors/*.json`).

---

## 1. Problem statement (what the audit flagged, restated precisely)

ADR-0008 makes pricing **asset-class-agnostic** by routing every forward/discount through the carry producer `Carry` (`celnet-types`) via `celnet-core`'s `CarryInputs` seam. The audit (F5/F6) found **no hot-path `match`-on-`Underlying`/`Carry` violation** anywhere — both flagged crates are clean of asset-class branching. The flag is the **other** failure mode: **input coupling**. Two crates never consume the agnostic seam at all:

- **`celnet-exotics` (F5):** every engine takes `celnet_types::VanillaInputs` (`{spot, strike, vol, t, r_dom, r_for}`) and hard-wires the FX two-rate drift `r_dom − r_for` / discount `e^{−r_dom·t}` at **~30 sites**. `CarryInputs` appears nowhere in the crate. Result: an equity/commodity/crypto exotic is **unpriceable** without re-deriving each engine. This is the largest pricing crate.
- **`celnet-surface` (F6):** `MarketContext { spot, r_dom, r_for, t, conventions }` (`quotes.rs:148`) computes the FX forward `S·e^{(r_dom−r_for)t}` and emits `VanillaInputs` via `template()` (`quotes.rs:201`). FX-only market state, no agnostic carry.

Both are **numerically correct and unbranched for FX today**; the gap is cross-asset reach. Remediation is a **byte-identity-gated refactor onto the carry seam**, not a numerical change — because `Carry::FxRates` reproduces `VanillaInputs::forward`/`df_dom` **bit-for-bit** (already proved by `fx_carry_inputs_byte_identical`, `carry.rs:253`).

---

## 2. The seam mapping (the one identity that makes FX byte-identical)

The entire migration rests on this algebraic identity, already implemented and gated in `celnet-core`/`celnet-types`:

| FX-coupled quantity (today) | Agnostic seam accessor (target) | Byte-identity basis |
|---|---|---|
| `i.r_dom − i.r_for` (net drift `b`) | `carry.carry_rate()` | `Carry::FxRates ⇒ r_dom − r_for` (`lib.rs` `carry_rate`) — **same two flops, same order** |
| `i.r_dom` (discount rate `r`) | `carry.discount_rate()` | `Carry::FxRates ⇒ r_dom` (const accessor) |
| `i.r_for` (foreign rate) | `carry.discount_rate() − carry.carry_rate()` | `r_dom − (r_dom − r_for) = r_for` — **NOT byte-identical in general** (see §4.3) |
| `exp(−i.r_dom·t)` (domestic DF) | `carry.discount_df(t)` | `libm::exp(−discount_rate()·t)` — identical call |
| `i.spot·exp((r_dom−r_for)t)` (forward) | `spot · carry.forward_factor(t)` | `libm::exp(carry_rate()·t)` — identical call |
| `exp(−i.r_for·t)` (foreign DF) | `exp(−(discount_rate()−carry_rate())·t)` | float-rounding-sensitive — see §4.3 |

**Key insight for FX byte-identity:** `Carry::FxRates { r_dom, r_for }` stores `r_dom` and `r_for` *verbatim* (no pre-combination). `carry_rate()` computes `r_dom − r_for` with the **identical IEEE-754 operation** the engines do today, so `b` is bit-identical. `discount_rate()` returns `r_dom` unchanged. Therefore every site that uses **only** `b`, `r_dom`, or DFs derived from them is **trivially byte-identical**.

**The one hazard — the foreign rate `r_for` in isolation.** A handful of sites use `r_for` *directly* (not via `b`): the foreign discount `exp(−r_for·t)` (`lookback.rs:77,124`, `digital.rs:82,122`, `forward_start.rs:96`), the PDE/American far-field `s·e^{−r_for·τ}` (`pde.rs:318`, `american.rs:442`), and the forward-delta scaling `exp(r_for·t)` (`american.rs:301`). Reconstructing `r_for = r_dom − b = discount_rate() − carry_rate()` is **a different float expression** than reading the stored `r_for`, and `(r_dom − (r_dom − r_for))` does **not** in general round to `r_for`. **Resolution:** `Carry::FxRates` must expose `r_for` *directly* — see §4.3 for the exact `celnet-core` accessor to add (`Carry::foreign_rate() → Option<f64>` or a typed `fx_rates()` projection), so these sites read the stored `r_for` with zero arithmetic. This keeps FX byte-identical and is the **only** `celnet-core` addition required.

---

## 3. `celnet-exotics` remediation (F5) — the structural plan

### 3.1 Target input type — introduce `ExoticInputs` (NOT raw `CarryInputs`)

Do **not** thread `CarryInputs` directly: it lacks a `strike`-free constructor pattern and the exotics crate's pervasive struct-update idiom (`VanillaInputs { strike: k, ..i }`, used ~15×). Instead add a thin crate-local wrapper that **is** the carry seam but preserves the exotics ergonomics:

**File: `crates/celnet-exotics/src/inputs.rs` (NEW), exported from `lib.rs`.**

```rust
use celnet_types::{Carry, Underlying};

/// The agnostic market state every exotic engine prices against — the carry-seam
/// replacement for the FX-only `VanillaInputs`. Holds the same (spot, strike, vol, t)
/// plus a `Carry` producer and an `Underlying` identity. Forward/discount are formed
/// ONLY through `carry`; no engine reads `underlying` in math (it is identity/routing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExoticInputs {
    pub spot: f64,
    pub strike: f64,
    pub vol: f64,
    pub t: f64,
    pub underlying: Underlying,
    pub carry: Carry,
}

impl ExoticInputs {
    #[inline] pub fn carry_rate(&self) -> f64 { self.carry.carry_rate() }          // b = r_dom − r_for for FX
    #[inline] pub fn discount_rate(&self) -> f64 { self.carry.discount_rate() }     // r = r_dom for FX
    #[inline] pub fn discount_df(&self) -> f64 { self.carry.discount_df(self.t) }   // e^{−r·t}
    #[inline] pub fn forward(&self) -> f64 { self.spot * self.carry.forward_factor(self.t) }
    /// Foreign/yield discount e^{−(r−b)·t}; for FX e^{−r_for·t}. Reads the stored
    /// `r_for` via `carry.foreign_rate()` (see celnet-core §4.3) — NO `r_dom − b` recompute.
    #[inline] pub fn carry_df(&self) -> f64 { /* exp(-(self.carry.yield_rate())*self.t) */ }
}
```

`ExoticInputs` mirrors `CarryInputs` exactly but is owned by the exotics crate so its accessors and struct-update churn don't pollute the frozen core seam. **A `From<&ExoticInputs> for CarryInputs` and `From<CarryInputs> for ExoticInputs`** keep it a pure view, not a fork. (Decision: a wrapper, not raw `CarryInputs`, because the crate mutates `strike`/`spot`/`vol` per-leg ~15× and needs `carry_df`/`forward` helpers that don't belong on the core type.)

### 3.2 Rewrite the central hub first — `Lognormal` (lib.rs:218–285)

`Lognormal` (`lib.rs`) is the **single carry hub** for every analytic engine (26 call sites of `.carry()/.df_dom()/.df_for()/.mu()/.lambda()`). Migrate it once and most analytic engines follow mechanically:

```rust
// lib.rs — replace the (r_dom, r_for) fields with the carry producer.
pub(crate) struct Lognormal { pub vol: f64, pub t: f64, pub carry: Carry }

impl Lognormal {
    pub(crate) fn from_inputs(i: &ExoticInputs) -> Self {
        Self { vol: i.vol, t: i.t, carry: i.carry }
    }
    #[inline] pub(crate) fn carry(&self) -> f64 { self.carry.carry_rate() }      // was r_dom − r_for
    #[inline] pub(crate) fn df_dom(&self) -> f64 { self.carry.discount_df(self.t) } // was exp(−r_dom·t)
    #[inline] pub(crate) fn df_for(&self) -> f64 { exp(-self.carry.yield_rate() * self.t) } // was exp(−r_for·t); reads stored r_for
    // mu(), lambda(), sigma_sqrt_t() unchanged — they call carry()/discount_rate() which are byte-identical.
    #[inline] fn r_dom(&self) -> f64 { self.carry.discount_rate() } // internal, for lambda()
}
```

`mu()` (`lib.rs:274`) and `lambda()` (`lib.rs:281`) use `self.carry()` and `self.r_dom` — both become `carry_rate()`/`discount_rate()`, byte-identical. **This single change carries `touch.rs`, `barrier.rs` (analytic), `digital.rs`, `forward_start.rs` (analytic), `lookback.rs` (analytic), and `asian.rs` (analytic)** for the closed-form paths, since they build a `Lognormal` from inputs.

### 3.3 Enumerated site-by-site change list (the ~30 sites, grouped by sequencing wave)

> Convention: `b ← i.carry_rate()`, `r ← i.discount_rate()`, `e^{−r_dom·t} ← i.discount_df()` (or `i.carry.discount_df(t_k)` for term-`t`), `e^{−r_for·t} ← i.carry_df()`/`exp(−i.carry.yield_rate()·t)`, `F ← i.forward()`. Every replacement below is **byte-identical for FX** (`Carry::FxRates`).

**Wave A — closed-form analytics (lowest risk, gate first):**

| File:line | Today | Change |
|---|---|---|
| `lib.rs:235-242` | `Lognormal::from_inputs(&VanillaInputs)` | take `&ExoticInputs`, store `carry` (§3.2) |
| `lib.rs:248` | `r_dom − r_for` | `self.carry.carry_rate()` |
| `lib.rs:254` | `exp(−r_dom·t)` | `self.carry.discount_df(self.t)` |
| `lib.rs:260` | `exp(−r_for·t)` | `exp(−self.carry.yield_rate()·t)` |
| `lib.rs:283` | `2.0·r_dom/σ²` | `2.0·self.carry.discount_rate()/σ²` |
| `digital.rs:69` | `(r_dom − r_for + ½σ²)` | `(i.carry_rate() + ½σ²)` |
| `digital.rs:81,82,121,122` | `exp(−r_dom·t)`, `exp(−r_for·t)` | `i.discount_df()`, `i.carry_df()` |
| `asian.rs:148,193,361,610,628,672` | `b = r_dom − r_for` | `let b = i.carry_rate();` |
| `asian.rs:278,368,679,707,756` | `exp(−r_dom·t)` | `i.carry.discount_df(t)` |
| `lookback.rs:75,122` | `b = r_dom − r_for` | `i.carry_rate()` |
| `lookback.rs:76,77,123,124` | `exp(−r_dom·t)`, `exp(−r_for·t)` | `i.carry.discount_df(t)`, `exp(−i.carry.yield_rate()·t)` |
| `touch.rs:111,154,163,192,343,358` | `exp(−r_dom·t)` | `i.carry.discount_df(i.t)` (all via `Lognormal` once §3.2 lands) |
| `barrier.rs` analytic (via `Lognormal`) | `mu/lambda/df` | carried by §3.2 |
| `forward_start.rs:96` | `exp(−r_for·spec.reset)·spot·unit` | `exp(−i.carry.yield_rate()·spec.reset)·spot·unit` |
| `forward_start.rs:115,116` | `VanillaInputs { r_dom: i.r_dom, r_for: i.r_for }` (synthetic reset input) | build `ExoticInputs { carry: i.carry, .. }` |

**Wave B — MC path generators (drift-step form):**

| File:line | Today | Change |
|---|---|---|
| `mc.rs:80` | `(r_dom − r_for − ½σ²)·dt` | `(i.carry_rate() − ½σ²)·dt` |
| `mc.rs:82` | `exp(−r_dom·t)` | `i.discount_df()` |
| `mc.rs:366,373-381` | `b = r_dom − r_for`; recast `r_for = r_dom − eff_b` synthetic GK | `b = i.carry_rate()`; synthetic recast becomes `Carry::CostOfCarry { r: i.discount_rate(), b: eff_b }` (general — works for any asset class, byte-identical for FX since `discount_rate()=r_dom`) |
| `accumulator.rs:155,163` | `(r_dom−r_for−½σ²)dt`, `exp(−r_dom·t_k)` | `(i.carry_rate()−½σ²)dt`, `i.carry.discount_df(t_k)` |
| `tarf.rs:167,174` | same drift/DF | `i.carry_rate()`, `i.carry.discount_df(t_k)` |
| `barrier.rs:747,749` | `(r_dom−r_for−½σ²)dt`, `exp(−r_dom·t)` | `i.carry_rate()`, `i.discount_df()` |
| `touch.rs:566` | same drift | `i.carry_rate()` |
| `lookback.rs:241,244,464,466` | same drift/DF | `i.carry_rate()`, `i.carry.discount_df(i.t)` |
| `forward_start.rs:302,340,381,386` | drift + `exp(−r_dom·d[k])` | `i.carry_rate()`, `i.carry.discount_df(d[k])` |
| `asian.rs:619-620,637-638` | synthetic `r_for = r_dom − eff_b` | `Carry::CostOfCarry { r: i.discount_rate(), b: eff_b }` |

**Wave C — PDE / ADI / American finite-difference (far-field uses `r_for` directly):**

| File:line | Today | Change |
|---|---|---|
| `pde.rs:170` | `(r_dom−r_for−½σ²)t` | `(i.carry_rate()−½σ²)t` |
| `pde.rs:306-307,318,357` | `r_dom`,`r_for` fields; far-field `s·e^{−r_for·τ} − K·e^{−r_dom·τ}`; `diag −r_dom` | store `carry`; `s·exp(−i.carry.yield_rate()·τ) − K·i.carry.discount_df(τ)`; `diag −i.discount_rate()` |
| `adi.rs:152,193,194,624,653,740,743` | `mu`, `r_dom`, `carry = r_dom−r_for`, stencil `−r_dom` | `i.carry_rate()`, `i.discount_rate()` throughout |
| `american.rs:384,425-435,442,469,474,620,629` | drift, far-field `r_dom`/`r_for`, LSM DF | `i.carry_rate()`, far-field `exp(−i.carry.yield_rate()·τ)` + `i.discount_df`, LSM `exp(−i.discount_rate()·(node+1)dt)` |
| `american.rs:255-271` | rho bumps on `r_dom`/`r_for` (finite-diff Greeks) | see §3.4 — bump `Carry` components, report `RateSensitivities::Carry` |
| `american.rs:301` | `delta_forward = delta_spot·exp(r_for·t)` | `delta_spot·exp(i.carry.yield_rate()·t)` |

**Wave D — composite / specialized engines:**

| File:line | Today | Change |
|---|---|---|
| `var_swap.rs:75-101,171-182` | `VarSwapContext { r_dom, r_for }`; `spot = forward·e^{−(r_dom−r_for)t}`; `pv·e^{r_dom·t}` | replace ctx fields with `carry: Carry`; `spot = forward·exp(−i.carry_rate()·t)`; final `pv·exp(i.discount_rate()·t)` |
| `vol_swap.rs` (`fair_volatility_with`) | delegates to var_swap | carried by var_swap change |
| `lsv.rs:124,138,156,292,293,488,535,578,749` | `VanillaInputs` field; `df = exp(−r_dom·t)`; `carry = r_dom − r_for` (5×) | take `ExoticInputs`; `df = i.discount_df()`; `carry = i.carry_rate()` |
| `quanto.rs:83-98,114-121,183-224` | `quanto_adjusted_inputs` mutates `r_for ← r_for − adjustment`; downstream `r_dom − r_for`, `exp(−r_dom·t)` | adjust the **carry** not the rate: produce `Carry::CostOfCarry { r: i.discount_rate(), b: i.carry_rate() + adjustment }` (FX byte-identity: when `adjustment=0`, `b = r_dom−r_for` exactly; settlement discount still `i.discount_rate()=r_dom`). Replaces the `r_for ← r_for − adjustment` trick with the **mathematically primary** carry shift the module-doc already describes (`quanto.rs:20`). |
| `multiasset.rs:87-100,299,315,322` | per-leg `r_for`; basket drift `(r_dom − leg.r_for − ½σ²)t`; `exp(−r_dom·t)` | per-leg carry `b_leg`; shared `Carry` for discount: `(b_leg − ½σ²)t`, `i.carry.discount_df(t)`. Each leg carries its own `carry_rate`; numeraire `discount_rate` shared. |

> **Wave-D implementation note (as landed):** the quanto carry shift is realised on the
> **yield side** of the two-rate `(discount, yield)` carry — `Carry::FxRates { r_dom: i.discount_rate(),
> r_for: i.yield_rate() − adjustment }` — rather than the `CostOfCarry { r, b + adjustment }` sketch
> above. Both are the same carry shift (`b_Q = b + adjustment` exactly, since `b = r − q`), but only the
> yield-side form reproduces the historical FX `r_for ← r_for − adjustment` float arithmetic
> **bit-for-bit** for a *non-zero* adjustment (`(r_dom ⊖ r_for) ⊕ c` does not round-trip through
> `r ⊖ (r ⊖ b)`), which the §6 headline gate and the §8 sharp edge (read the stored yield via
> `Carry::yield_rate()`) make mandatory. The to_bits gates in
> `crates/celnet-exotics/tests/fx_byte_identity.rs` (`quanto_byte_identical`) freeze this. The basket's
> shared discount landed as a `Carry` parameter on `price_basket` (the settlement-cash numeraire,
> forward 1 ⇒ `b = 0`, discounting at `r_dom`); each `BasketLeg` carries its own `carry_rate` `b_a`.

### 3.4 Rate Greeks — `RateSensitivities::Carry` (american.rs:255-301)

`american.rs` is the only exotics engine reporting rate rhos (finite-diff, `fd_greeks`). Today it bumps `r_dom`/`r_for` and packs `Greeks { rho_dom, rho_for }`. Migrate to the generalized strip:

- Bump `discount_rate` (→ `discount_rho`) and `carry_rate` (→ `carry_rho`) of the `Carry`.
- Emit `RateSensitivities::Carry { discount_rho, carry_rho }` (the existing enum in `celnet-types`).
- **FX projection (byte-identity gate):** `rho_dom = discount_rho + carry_rho`, `rho_for = −carry_rho` (since FX `r = r_dom`, `b = r_dom − r_for` ⇒ `∂/∂r_dom = ∂/∂r + ∂/∂b`, `∂/∂r_for = −∂/∂b`). Provide `RateSensitivities::Fx { rho_dom, rho_for }` for FX underlyings by this projection so the golden grid stays identical. **Bump-direction note:** to keep FX rhos byte-identical, bump along `r_dom`/`r_for` directly for an FX `Carry::FxRates` (same `H_R` finite-diff perturbations as today) and *label* them `discount_rho`/`carry_rho` only at the API boundary — i.e. the finite-diff math is unchanged, only the output tag changes. (Decision recorded: changing the bump basis from `(r_dom,r_for)` to `(r,b)` would alter the rounding of the central differences; keep the bump basis FX-native and re-tag, preserving byte-identity.)

### 3.5 Downstream callers of exotics engines

Callers that construct `VanillaInputs` and pass it to an exotics engine must construct `ExoticInputs` instead. Grep target: every `VanillaInputs::new(...)` feeding an exotics `pub fn` outside `#[cfg(test)]`. Primary non-test callers live in `celnet-server` (`lsv_pricer.rs`, `pricer.rs`, `services/pricing.rs`) and `celnet-engine`. These already have a `Carry` in scope (proto `MarketContext` now carries `discount_rate` + `CarryModel` — see §4.1), so they build `ExoticInputs { carry, .. }` directly with **no FX numeric change**. Test fixtures (`VanillaInputs::new(.., r_dom, r_for)`) become `ExoticInputs { carry: Carry::FxRates { r_dom, r_for }, underlying: <fx>, .. }`.

---

## 4. `celnet-surface` remediation (F6) — smaller crate, larger blast radius

### 4.1 Critical de-risking finding: the WIRE is already generalized

The proto `MarketContext` (`crates/celnet-proto/proto/celnet.proto:539`) **already** carries `discount_rate` + `CarryModel carry` — **not** `r_dom`/`r_for`. The W1 wave generalized the wire MarketContext; a `CarryModel → Carry` adapter already exists in `celnet-proto`. **Only the surface crate's internal `MarketContext` struct (`quotes.rs:148`) remains FX-coupled.** This means F6 does **not** touch the contract — it is an internal-struct migration, far smaller than the audit's prose implies.

### 4.2 The change (`quotes.rs:148-203`)

```rust
pub struct MarketContext {
    pub spot: f64,
    pub carry: Carry,              // replaces r_dom + r_for
    pub t: f64,
    pub conventions: ConventionRecord,
}
impl MarketContext {
    pub fn new(spot: f64, carry: Carry, t: f64, conventions: ConventionRecord) -> Self { .. }
    pub fn forward(&self) -> f64 { self.spot * self.carry.forward_factor(self.t) }   // was S·e^{(r_dom−r_for)t}
    pub fn template(&self, strike: f64, vol: f64) -> ExoticOrVanillaInputs { .. }     // see §4.3
    // atm_strike / atm_total_variance unchanged (they call forward()).
}
```

`forward()` (`quotes.rs:188`) becomes `spot · carry.forward_factor(t)` — byte-identical via `Carry::FxRates`. `template()` (`quotes.rs:201`) must still feed `celnet-vanilla`'s `price`/`atm_strike` (which take `VanillaInputs`); lower through the **existing** `celnet_core::fx_vanilla_inputs(&CarryInputs)` (which returns `VanillaInputs` byte-identically and typed-rejects non-FX carry). So `template()` builds a `CarryInputs` and calls `fx_vanilla_inputs` — this **reuses the already-gated lowering**, no new arithmetic.

### 4.3 The `Carry::foreign_rate()` accessor — the ONE `celnet-core`/`celnet-types` addition

The §2 hazard (direct `r_for` reads in `lookback`/`digital`/`pde`/`american`/`forward_start` and surface's foreign DF) requires reading the **stored** `r_for`, not recomputing `r_dom − b`. Add to `Carry` (`celnet-types/src/lib.rs`):

```rust
/// The yield/foreign rate `q` such that discount=e^{−r·t}, growth=e^{−q·t}, b=r−q.
/// For FX this is the STORED `r_for` (read verbatim — never `r_dom − b`, which would
/// not round-trip bit-for-bit). For CostOfCarry it is `r − b`.
#[must_use] pub fn yield_rate(&self) -> f64 {
    match self {
        Carry::FxRates { r_for, .. } => *r_for,            // verbatim — byte-identical
        Carry::CostOfCarry { r, b } => r - b,
    }
}
```

This is the **only** change to a frozen core type, and it is purely additive (a new accessor; no field/contract change, ADR-0008-compliant: the `match` is the sanctioned seam-internal one, not a hot-path pricer branch). It must land **first** (Wave 0) so `carry_df`/`Lognormal::df_for`/surface foreign-DF can read `r_for` verbatim. **Gate it** with a `to_bits` test in `celnet-core`: for the §2 grid, `Carry::FxRates{r_dom,r_for}.yield_rate().to_bits() == r_for.to_bits()`.

### 4.4 Surface blast radius (honest scope)

`MarketContext::new(spot, r_dom, r_for, t, conv)` has **~60 call sites** across `celnet-cli`, `celnet-bench`, `celnet-proto` (`helpers.rs:91-151`), `celnet-engine`, `celnet-client`, `celnet-golden`, `celnet-parity`, `celnet-server` (~12 files), `celnet-integration`, and GUI/Excel tests. **All are constructors, none are math.** Each `MarketContext::new(s, r_dom, r_for, t, c)` becomes `MarketContext::new(s, Carry::FxRates { r_dom, r_for }, t, c)`. The proto adapter (`helpers.rs`) already has a `CarryModel`/`Carry` in hand, so it constructs the new form directly. **This is a wide but mechanical rename** — the risk is breadth (compile fan-out across ~12 crates + GUI/Excel mirrors of the *struct shape*, not the wire), not depth. Sequence it as its own wave (§5 Wave S) so the exotics waves stay independently green.

---

## 5. Dependency order & "gates green throughout" sequencing

Each wave compiles and passes its crate's gate **before** the next starts. FX byte-identity is the invariant across all of them.

- **Wave 0 — `celnet-types`/`celnet-core` (prereq, tiny):** add `Carry::yield_rate()` (§4.3) + its `to_bits` self-check. Gate: `just check-crate celnet-types && just check-crate celnet-core`. No downstream change yet → all dependents still compile. **Must be first.**
- **Wave A — exotics closed forms** (`inputs.rs`, `Lognormal`, `digital`, `asian` analytic, `lookback` analytic, `touch`, `barrier` analytic, `forward_start` analytic, §3.3 Wave A). Gate: `just check-crate celnet-exotics` + the golden FX-identity gate (§6). Lowest risk — pure carry-accessor swaps.
- **Wave B — exotics MC** (`mc`, `accumulator`, `tarf`, MC arms of `barrier`/`touch`/`lookback`/`forward_start`/`asian`). Same gate. The synthetic-recast sites switch to `Carry::CostOfCarry { r, b }` (general, FX-identical).
- **Wave C — exotics FD** (`pde`, `adi`, `american` incl. §3.4 Greeks). Same gate. Highest exotics risk: far-field `r_for` reads (now via `yield_rate()`) and the rho re-tag.
- **Wave D — exotics composite** (`var_swap`, `vol_swap`, `lsv`, `quanto`, `multiasset`). Same gate.
- **Wave E — exotics downstream callers** (`celnet-server`, `celnet-engine` construct `ExoticInputs`). Gate: `just check-crate celnet-server celnet-engine`.
- **Wave S — surface** (`MarketContext`→`carry`, `template` via `fx_vanilla_inputs`, §4.2) **+ the ~60 constructor call sites** (§4.4). Independent of exotics waves; can run in parallel after Wave 0. Gate: `just check-crate celnet-surface` then the full `just check` (it touches ~12 crates + GUI/Excel struct mirrors).
- **Wave Z — milestone integration gate:** full-workspace `just check` (literal "All gates passed.") + conformance + GUI/Excel e2e. Re-index codebase-memory; ADR note via `manage_adr`.

**If exotics cannot land whole** (it is the largest crate): the audit's narrowing is honored by the wave split — ship Waves 0+A+S (closed forms + surface) as the first green milestone, and record Waves B/C/D as an **explicit ADR-tracked deferral** (never silently FX-only). Each wave is independently shippable because `ExoticInputs`/`Carry` coexist with the unchanged FX numbers.

---

## 6. The FX byte-identity gate (the safety net — exact tests)

The existing golden vector grid is the substrate: `crates/celnet-golden/vectors/{digital,asian_option,lookback,touch,single_barrier,double_barrier,window_barrier,accumulator,tarf,forward_start,cliquet,american,quanto,variance_swap,volatility_swap,basket}.json` — one per migrated engine.

**Per-wave gate (the load-bearing assertion):** for every vector, price the engine **twice** — once via the legacy `VanillaInputs` path (git-stash the pre-migration binary, or assert against the committed golden JSON which was generated from the FX path) and once via the new `ExoticInputs { carry: Carry::FxRates { r_dom, r_for }, .. }` path — and assert **`new.to_bits() == golden.to_bits()`** (exact, not `assert_close`). Add to `crates/celnet-golden/tests/` a `carry_seam_byte_identity.rs` that loops every vector grid through the new seam and `to_bits`-compares to the stored golden value. **`to_bits`, not epsilon** — the whole claim is that `Carry::FxRates` is bit-for-bit, so anything looser would hide a regression.

**Why this is sound (not circular):** the golden JSONs were generated against QuantLib / published closed forms (the independent oracle), *before* this refactor. The gate proves the refactor changed **nothing** numerically for FX. The independent oracle is the pre-existing golden grid, untouched by this work.

**Cross-asset enablement test (the payoff this unlocks):** add an *independent-oracle* test for one non-FX exotic — e.g. a digital on an equity `Carry::CostOfCarry { r, b: r − q }` reconciled to a **generalized-BSM digital closed form re-derived by hand** (`e^{−r·t}·Φ(d₂)` with `d₂ = [ln(S/K)+(b−½σ²)t]/(σ√t)`), 1e-12. This is the same independent-oracle discipline W1 used for the equity-dividend plugin (no circular oracle: the closed form is re-derived, not read back from the engine).

---

## 7. Cross-asset enablement unlocked

Once `celnet-exotics` consumes `ExoticInputs`/`Carry`, **every exotic prices any asset class for free**, with zero per-engine work:
- **Equity exotics** (`Carry::CostOfCarry { r, b: r − q }`): barriers/touches/digitals/Asians/lookbacks/American on dividend-paying equities.
- **Commodity exotics** (`b: r − convenience`): Black-76-style barriers/Asians (Asians are *the* commodity product).
- **Crypto exotics** (`b: r − funding`): perp-funded barrier/touch structures.
- **Quanto** generalizes correctly: the carry-shift form (§3.3 D) is asset-class-agnostic, settlement discount stays `discount_rate()`.

This is the largest cross-asset reach unlock in the platform — exotics is the biggest pricing crate, and it is the last FX-coupled engine.

---

## 8. Honest scope & risk assessment

- **F5 (exotics) is genuinely large:** ~30 sites across 14 modules + the `VanillaInputs`→`ExoticInputs` signature change on every `pub fn` + downstream callers. The `Lognormal` hub (§3.2) collapses most analytic sites into one change, but MC/FD/composite are per-module. Realistic as 4–5 sequenced sub-waves, each independently green. **Risk: medium**, mitigated by the `Lognormal` hub + the per-wave `to_bits` golden gate making any drift a hard test failure.
- **F6 (surface) is small in math, wide in compile fan-out:** the *struct* change is ~5 methods, but `MarketContext::new` has ~60 call sites across ~12 crates + GUI/Excel struct mirrors. **The wire is already generalized** (proto `MarketContext` carries `discount_rate`+`CarryModel`), so **the contract is untouched** — this is the key de-risking fact the audit's prose under-weighted. **Risk: low-depth, high-breadth.**
- **The single sharp edge** is the direct-`r_for` reconstruction (§2/§4.3): naïvely using `discount_rate() − carry_rate()` would silently break byte-identity at ~8 foreign-discount sites. The `Carry::yield_rate()` accessor (read stored `r_for` verbatim) is **mandatory and must land in Wave 0**; its `to_bits` self-check is the guard.
- **No mocks/placeholders/`todo!()`:** every wave is a complete, gated migration. If exotics can't finish whole, narrow to Waves 0+A+S and ADR-track the remainder — never leave a silently-FX-only engine.
- **Naming:** `ExoticInputs`/`Carry`/`yield_rate` are purpose-named, vendor-/person-neutral; "generalized-BSM"/"Reiner-Rubinstein" provenance stays in doc comments only.
- **One unversioned contract:** no `schema_version`; the proto already evolved in place (W1). Surface struct change is internal; `ExoticInputs` is crate-local.

---

## 9. Files touched (complete manifest)

**New:** `crates/celnet-exotics/src/inputs.rs`; `crates/celnet-golden/tests/carry_seam_byte_identity.rs`.
**`celnet-types`:** `src/lib.rs` (+`Carry::yield_rate()`).
**`celnet-exotics`:** `lib.rs` (Lognormal + exports), `digital.rs`, `asian.rs`, `lookback.rs`, `touch.rs`, `barrier.rs`, `forward_start.rs`, `mc.rs`, `accumulator.rs`, `tarf.rs`, `pde.rs`, `adi.rs`, `american.rs`, `var_swap.rs`, `vol_swap.rs`, `lsv.rs`, `quanto.rs`, `multiasset.rs`.
**`celnet-surface`:** `quotes.rs` (MarketContext + template + forward).
**Downstream constructor updates (mechanical):** `celnet-server` (~12 files), `celnet-engine`, `celnet-cli`, `celnet-bench`, `celnet-proto/helpers.rs`, `celnet-client`, `celnet-golden`, `celnet-parity`, `celnet-integration`, + GUI/Excel `MarketContext` struct mirrors.

