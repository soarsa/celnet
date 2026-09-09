# ADR-0008 Conformance Audit — Asset-Class-Agnostic Carry Seam

**Audited tree:** a clean worktree of `origin/main` @ `0f8933b`.
**Method:** read-only (Read/Grep + the seam source in `celnet-core`/`celnet-types`); no `cargo` build/test/clippy run (compute-courtesy lane). Adversarial: a silent `if underlying == FX` inside a pricer counts as a finding.
**Scope:** every leaf/engine crate named in the task — `celnet-vanilla`, `celnet-equity-vanilla`, `celnet-commodity-vanilla`, `celnet-crypto-vanilla`, `celnet-linear`, `celnet-exotics`, `celnet-rfq`, `celnet-surface`, `celnet-core`.

## The ADR-0008 rule being audited

The carry seam (`crates/celnet-core/src/carry.rs`, types in `crates/celnet-types/src/lib.rs`) makes pricing **asset-class-agnostic**:

- `CarryInputs { spot, strike, vol, t, underlying: Underlying, carry: Carry }` is the generalized input.
- A leaf forms its forward/discount **only** through the carry producer — `Carry::forward_factor(t)` / `Carry::discount_df(t)` (and, for Greeks, `Carry::discount_rate()` / `Carry::carry_rate()`).
- A pricing leaf **must not branch on asset class on the hot path**: no `match carry { … }`, no `match underlying { … }`, no `if underlying == Fx`, no asset-class enum dispatch *inside the pricing math*. The single sanctioned `match` on `Carry`/`Underlying` is the **seam-entry validation** in `celnet-core::fx_vanilla_inputs` (lower-or-typed-reject), never a silent mis-price.
- Genuinely different **payoff shapes** (e.g. an inverse `1/S_T` crypto contract vs a linear payoff) may differ — ADR-0008 generalizes carry and the *payoff*, so a settlement/denomination discriminator is permitted; an asset-class carry/underlying discriminator is not.

`OptionType` (call/put) matching is orthogonal and always permitted.

## Per-crate verdict

| Crate | Verdict | Input type | Carry routing | Asset-class branch in math? | Evidence |
|-------|:------:|-----------|---------------|-----------------------------|----------|
| `celnet-core` | **PASS** | defines `CarryInputs` | is the seam | the *only* sanctioned `match` (seam entry) | `carry.rs:193-211` lower-or-typed-reject; `lib.rs:709-739` carry accessors |
| `celnet-vanilla` | **PASS** | `CarryInputs` → lowers to FX | via `fx_vanilla_inputs` + `fx_carry_greeks` | none | `pricer.rs:30-44` (zero branch; delegates to unchanged GK) |
| `celnet-equity-vanilla` | **PASS** | `EquityInputs` (carry `r,b`) | `carry()`/`discount_df()` only | none (only `OptionType`) | `lib.rs:164,171-282` generalized-BSM, reads `b`,`r` only |
| `celnet-commodity-vanilla` | **PASS** | `CommodityInputs` over `Carry::CostOfCarry` | `discount_rate()`/`carry_rate()`/`forward_factor()`/`discount_df()` | none (only `OptionType`) | `lib.rs:148-289` Black-76; never matches the `Carry` variant |
| `celnet-crypto-vanilla` | **PASS** | `LinearInputs`/`InverseInputs` over `Carry` | seam accessors only; `Underlying` absent from src | none (only `OptionType`) | `linear.rs:85-234`, `inverse.rs:143-238`; `funding.rs:40` assembles carry |
| `celnet-linear` | **PASS** | `LinearInputs { underlying, carry, … }` | `carry.forward_factor()`/`discount_df()`; Greeks decompose via `carry_rate()` | none; `underlying` stored as identity, never read in math | `forward.rs:34-106`, `inputs.rs:224-232`, `swap.rs`, `ndf.rs` |
| `celnet-rfq` | **PASS** (no pricing math) | aggregates quotes; no pricing | n/a | none | orchestration only; matches are on `side`/ranking/`option_type` |
| `celnet-exotics` | **FLAG** | `VanillaInputs` (FX `r_dom`/`r_for`) | hard-wires FX two-rate drift `r_dom − r_for` | no `match`, but FX-coupled input type (not seam-consuming) | ~30 drift sites (see findings); `CarryInputs` absent from src |
| `celnet-surface` | **FLAG** | `MarketContext { r_dom, r_for, … }` → emits `VanillaInputs` | FX two-rate forward `S·e^{(r_dom−r_for)t}` | no `match`, but FX-coupled market state (not seam-consuming) | `quotes.rs:152-202` |

## Findings

### F1 — PASS: the four cross-asset vanilla leaves are textbook-conformant
`celnet-equity-vanilla`, `celnet-commodity-vanilla`, `celnet-crypto-vanilla` and the FX `celnet-vanilla` adapter all form forward/discount **exclusively** through the carry producer and report rate Greeks via `RateSensitivities::Carry` (or `::Fx` for the FX adapter). The only `match` expressions in their pricing math are on `OptionType`. Crucially, `Underlying` does **not appear at all** in `celnet-crypto-vanilla/src/*.rs`, and `Carry`/`Underlying` are matched in commodity/equity/crypto only inside `#[cfg(test)]` constructors. This is exactly the ADR-0008 shape: one engine, parameterized by `(r, b)`, agnostic to which asset class produced that carry.

### F2 — PASS: `celnet-linear` carries `underlying` as identity only
`LinearInputs` holds an `Underlying` field but it is **never read in pricing math** (`grep '\.underlying' crates/celnet-linear/src/*.rs` → no non-test hits). PV/Greeks come purely from `carry.forward_factor()`, `carry.discount_df()`, `carry.discount_rate()`, `carry.carry_rate()` (`forward.rs:34-106`, `inputs.rs:224-232`). The forward leg's Greeks are reported in the FX `(r_dom, r_for)` decomposition, but they are **derived from carry accessors** (`r_for = r_dom − carry_rate()`, `forward.rs:77-79`), not by matching `Carry::FxRates`. The crate's `match self` sites are on `Side`/`SwapError`. Fully conformant.

### F3 — NOTE (conforms): crypto `SettlementStyle` routing is a payoff axis, not an asset-class branch
`crates/celnet-crypto-vanilla/src/settlement.rs:40-59` `route_price` matches on `SettlementStyle { Linear, InverseCoin }`. This is a **settlement-denomination / payoff-shape** discriminator (USD-margined linear payoff vs coin-margined `1/S_T` convex payoff), which ADR-0008 explicitly permits ("the inverse is a genuinely new payoff *shape*"). Both arms share the identical carry seam and forward/discount; neither branches on `Carry` or `Underlying`. The module doc (`settlement.rs:11-16`) calls this out. **Conforms** — recorded so a future reviewer does not mistake it for an asset-class branch.

### F4 — NOTE (cross-asset identity gap, non-ADR-0008): `celnet-rfq` keys on `CcyPair`
`crates/celnet-rfq/src/panel.rs:50` types the RFQ on `CcyPair`, not `Underlying`. RFQ performs **no pricing math** (it fans out, ranks best-bid/offer, tie-breaks, times out), so this is **not** an ADR-0008 hot-path concern. But the request identity cannot today carry a metal/crypto/equity underlying; when those waves wire RFQ, the field should widen `CcyPair → Underlying` to match the rest of the contract. Logged as a cross-asset-readiness smell, **not a conformance violation**.

### F5 — FLAG: `celnet-exotics` is structurally FX-coupled (consumes `VanillaInputs`, not the carry seam)
Every exotics engine takes `celnet_types::VanillaInputs` (the FX-specific `{spot, strike, vol, t, r_dom, r_for}`) and **hard-wires the FX two-rate drift** `i.r_dom - i.r_for`. `CarryInputs` appears nowhere in `crates/celnet-exotics/src/`; the crate uses `celnet-core` only for `math::{exp,ln,sqrt,...}` and `Smile`/`FlatSmile`. The FX drift is baked in at ~30 sites across **all** pricing paths (MC, PDE, ADI, analytic):

- `accumulator.rs:155`, `barrier.rs:747`, `tarf.rs:167`, `mc.rs:80,366`, `touch.rs:566`, `forward_start.rs:302,381`, `lookback.rs:75,122,241,464` — `(i.r_dom - i.r_for - ½σ²)` drift / `b = r_dom − r_for`.
- `adi.rs:152,194`, `pde.rs:170,352`, `american.rs:384,469,620` — same FX drift in the PDE/ADI/American FD generators.
- `asian.rs:148,193,361,610,628,672`, `digital.rs:69`, `lsv.rs:293,488,535,578,749` — analytic `b = r_dom − r_for`.

**Adversarial note:** this is *not* a forbidden `match underlying`/`match carry` — there is no asset-class branch. It is the *other* ADR-0008 failure mode: the leaf does not consume the agnostic `CarryInputs`/`CarryModel` at all, so it is **structurally FX-only**. An equity/commodity/crypto exotic cannot be priced without re-deriving each engine on `CarryInputs`. Because the FX arithmetic is correct and unbranched, FX pricing is unaffected; the gap is cross-asset reach, and it is real (this is the largest pricing crate). Consistent with the W1 follow-up note in `CLAUDE.md` ("celnet-surface FX→neutral split … only when a non-FX surface leaf lands"), exotics carries the analogous deferral but is not yet flagged in the ledger.

### F6 — FLAG: `celnet-surface` market context is FX-coupled (`r_dom`/`r_for` → `VanillaInputs`)
`crates/celnet-surface/src/quotes.rs:152-202`: `MarketContext` stores `r_dom`/`r_for`, computes the outright forward as `S·e^{(r_dom−r_for)·t}` (`quotes.rs:186`), and emits `VanillaInputs` via `template()` (`quotes.rs:201-202`). Like exotics, there is **no asset-class branch** (matches are on `OptionType`/convention only), but the market state is the FX two-rate form, not the agnostic `Carry`. CLAUDE.md already records this as the open **"celnet-surface FX→neutral split"** backlog item, so F6 confirms a *known* deferral rather than discovering a new one.

## Remediation

The two FLAGs are the **same structural fix** (route the engine input through the agnostic seam) applied to the two FX-coupled leaves. No PASS crate needs any change.

1. **`celnet-exotics` (F5) — adopt `CarryInputs` as the engine input.**
   - Replace `VanillaInputs` params with `CarryInputs` (or a thin `ExoticInputs` that holds a `Carry`) across the engine signatures. Inside each engine, replace `i.r_dom - i.r_for` with `i.carry.carry_rate()` (the net drift `b`) and `exp(-i.r_dom * t)` with `i.carry.discount_df(t)` (or `i.carry.discount_rate()`). For FX the `Carry::FxRates` arm reproduces today's numbers **bit-for-bit** (`celnet-types` `forward_factor`/`discount_df` are byte-identical to `VanillaInputs::forward`/`df_dom`, proved by `fx_carry_inputs_byte_identical`), so this is a refactor with a `to_bits` FX-identity gate, not a numerical change. Where an engine reports `r_dom`/`r_for` rhos (e.g. `american.rs`), emit `RateSensitivities::Carry { discount_rho, carry_rho }` (FX maps back via `rho_dom = discount_rho + carry_rho`, `rho_for = −carry_rho`). Stage it module-by-module behind the existing golden grid so each engine's FX output stays byte-identical.
   - **Sequencing:** this is a sizeable refactor on the largest pricing crate; if it cannot land whole, narrow scope (convert the closed-form engines first: `digital`, `asian`, `lookback`), and record the remaining MC/PDE engines as an explicit ADR-tracked deferral — do **not** leave it silently FX-only.

2. **`celnet-surface` (F6) — generalize `MarketContext` to the carry seam.**
   - Replace `MarketContext { r_dom, r_for }` with a `Carry`-carrying context; `forward()` becomes `spot * carry.forward_factor(t)` and `template()` lowers to whatever the downstream pricer takes (FX leaf via `fx_vanilla_inputs`, or directly `CarryInputs`). FX byte-identity is again guaranteed by the `Carry::FxRates` arm. This is the already-named "FX→neutral split"; the audit confirms the precise call sites (`quotes.rs:152-202`).

3. **`celnet-rfq` (F4, optional, non-blocking) — widen request identity to `Underlying`.**
   - Change `panel.rs:50` `pair: CcyPair` → `underlying: Underlying` when a non-FX RFQ wave lands. No pricing impact today.

## Conclusion

- **No ADR-0008 hot-path violation exists** in any audited crate: zero `match`/`if` on `Underlying`/`Carry`/`CcyPair`/asset-class enums inside pricing math, and zero silent `if underlying == FX` guards (adversarial scan clean).
- **7 of 9 crates fully conform** by routing through the agnostic carry seam (`celnet-core`, `celnet-vanilla`, `celnet-equity-vanilla`, `celnet-commodity-vanilla`, `celnet-crypto-vanilla`, `celnet-linear`, `celnet-rfq`).
- **2 crates are FLAGged** for the *input-coupling* failure mode (not the *branching* one): `celnet-exotics` and `celnet-surface` consume the FX-specific `VanillaInputs`/`r_dom`/`r_for` rather than the agnostic `CarryInputs`/`Carry`. Both are correct and unbranched for FX today; both are known cross-asset deferrals (surface is already in the ledger, exotics is not). Remediation is a byte-identity-gated refactor onto the carry seam, sequence-able module-by-module.
