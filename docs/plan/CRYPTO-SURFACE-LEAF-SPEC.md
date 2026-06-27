# CRYPTO-SURFACE-LEAF — strike-axis smile leaf + asset-class-neutral surface core

**Status:** implementation-ready spec (read-only design pass; no `cargo`/`npm` run — compute-courtesy §4.1, session-B holds the build window).
**Source tree:** a clean worktree of `origin/main`.
**Lane:** SURFACE-CRYPTO-LEAF (W3 §5 / §7-S6 deferred item; ADR-0008 §7 cross-asset enablement).
**Inputs read:** `crates/celnet-surface/src/{lib,quotes,calibrate,parametric,parametric_surface,extended_surface,arbitrage,termstructure,surface,strangle,market_hedge,mathx}.rs`; `crates/celnet-surface/tests/surface_properties.rs`; `crates/celnet-parity/tests/surface.rs`; `crates/celnet-crypto-vanilla/src/{lib,funding,inverse}.rs`; `crates/celnet-types/src/lib.rs` (`Carry`); `docs/W3-CRYPTO-PLAN.md` §5/§6/§7; `docs/SURFACE-WORKFLOW.md`; `docs/plan/ADR0008-EXOTICS-SURFACE-REMEDIATION.md` §4/§7.

---

## 1. Scope & binding invariants

Crypto option chains (the `celnet-crypto-vanilla` underlyings: Deribit-convention European BTC/ETH, linear + inverse settlement — provenance prose-only) quote volatility on a **strike / log-moneyness axis** (exchange chains by strike), not FX delta pillars. `celnet-surface` today only ingests FX broker quotes (ATM + 25Δ/10Δ RR/BF). This lane delivers:

- **(a)** the **asset-class-neutral surface core** made explicit: a precise inventory of what is already quote-basis-agnostic, plus the *minimal* neutral seam for a second quote basis;
- **(b)** the **strike-axis leaf**: log-moneyness raw-SVI calibration off strike-gridded quotes, forward through the ADR-0008 `Carry` seam (funding carry `b = r − funding`), 24×7 clock semantics, all existing no-arbitrage machinery reused;
- **(c)** an **independent, can-disagree oracle** (injected ground truth + published-formula re-derivations + independent linear algebra + a cross-path canary);
- **(d)** **gate tests** including a new frozen-bits FX regression proving the FX path is `to_bits`-unchanged through the refactor.

Binding rules honored throughout: **no mocks/placeholders**; **vendor-neutral identifiers** (Deribit / Gatheral / De Marco–Martini / QuantLib in doc comments only); **no hot-path asset-class matching** (the only `match` on `Carry` stays inside the sanctioned `celnet-types` seam accessors); **one unversioned contract** (no wire change in this lane — see §8); **independent non-circular oracles** (the FRTB-0.75ρ lesson: every reference constant/formula re-derived from the published source, never read back from production); **FX byte-identity** (`to_bits`, not epsilon).

---

## 2. Neutrality audit of `celnet-surface` (deliverable a)

### 2.1 Already quote-convention-agnostic (reused unchanged, zero edits)

| Module | Items | Why it is already neutral |
|---|---|---|
| `parametric.rs` | `ParametricSlice` (raw SVI), `total_variance`, `d_total_variance`, `d2_total_variance`, `butterfly_density_factor` (Durrleman g), `min_butterfly_density_factor`, `satisfies_wing_bound` (Lee `b(1+|ρ|) ≤ 2`), `is_butterfly_free`, `vol_at`, `impl Smile` | Defined purely on `(k = ln(K/F), w = σ²t)` anchored at `(forward, t)`. No conventions, no rates, no delta. |
| `parametric_surface.rs` | `ParametricSurface` (SSVI), `phi`, `total_variance`, `is_butterfly_free`, `is_calendar_free`, `to_slice` | Pure `(k, θ)` math; closed-form Gatheral–Jacquier conditions are dimensionless. |
| `extended_surface.rs` | `ExtendedSlice`/`ExtendedSurface` (eSSVI, `(θ, ρ, ψ)`) | Same — `(k, θ, ρ, ψ)` only. |
| `arbitrage.rs` | `check_slice`, `implied_density`, `forward_call_strike_slope`, `ArbitrageReport` | Generic over `S: Smile`, evaluated on a strike grid against `(forward, t)`; the undiscounted forward call removes all discounting/rate coupling. |
| `termstructure.rs` | `BusinessClock`, `CalendarClock`, `TenorPillar<S>`, `TermStructure<S, C>`, `min_calendar_increment`, `is_calendar_free` | `(k, w, τ)` only. **`CalendarClock` (identity `τ(t) = t`) is exactly the 24×7 crypto clock** — continuous time, no weekend skip — so crypto needs *no new clock*. |
| `surface.rs` | `SmileModel`, `VolSurface<S, C>`, `SurfaceArbitrageReport`, `MaturitySlice` | Generic over `Smile` + clock. `SmileModel::Parametric` is the tag for the crypto leaf's slices — no new variant, no wire change. |
| `calibrate.rs` (numerics only) | `clamp`, `sumsq`, `gauss_newton_2/3/4`, `solve3`, `solve4`, `gaussian_eliminate`, `FIT_ITERS`, `FIT_FD_H` | Deterministic libm-only solver kit; nothing FX about it. Today **private to `calibrate.rs`** — the one code-motion this lane performs (§2.3). |
| `mathx.rs` | `powf` | Neutral. |
| `celnet-core` | `Smile` trait, `math::{exp, ln, sqrt, norm_cdf, norm_pdf}`, `is_close` | Neutral. |

### 2.2 FX-quote-coupled (stays FX-only; **untouched by this lane** except the §2.3 import-only edit)

| Module | Items | Coupling |
|---|---|---|
| `quotes.rs` | `DeltaPillar`, `RiskReversalButterfly`, `MarketQuotes` | Delta-pillar broker handles — FX quote vocabulary. |
| `quotes.rs` | `MarketContext` | Post-Wave-S the *market state* (`spot`, `carry: Carry`, `t`, `forward()`, `template()`) is already carry-neutral, **but** the struct requires a resolved FX `ConventionRecord` and carries the delta↔strike machinery (`strike_at_delta`, `atm_strike`, `delta_convention`, `atm_convention`). A strike-quoted chain has no delta convention to resolve. |
| `strangle.rs` | `market_strangle`, `calibrate_pillar`, `MarketStrangle`, `CalibratedPillar` | The broker-strangle → smile-strangle fixed point is intrinsically delta-space FX. |
| `market_hedge.rs` | `MarketHedgeSmile` | The vanna-volga FX broker baseline (3 delta-pillar anchors). |
| `calibrate.rs` | `anchors()`, `fit_sabr`, `fit_svi`, `fit_ssvi`, `fit_essvi`, `build_model_smile`, `CalibratedSmile` | Entry points take `(MarketContext, MarketQuotes)`; anchors and seeds derive from `calibrate_pillar` (delta-space). |
| `lib.rs` | `build_smile`, `build_smile_and_outer` | FX pipeline entry. |

### 2.3 The minimal neutral seam — and why **not** a `QuoteBasis` runtime enum

**Decision: the seam is the `(k, w)` total-variance anchor plane plus a `Carry`-bearing slice context, reached by two *typed front-ends* that lower into one shared neutral back half.** This is the ADR-0008 lowering pattern applied to quotes, exactly as `Carry` applied it to rates:

```
FX delta front-end (existing, untouched):
  MarketQuotes ──calibrate_pillar──▶ (k_i, w_i) anchors ──fit──▶ ParametricSlice ─┐
Strike front-end (NEW, this lane):                                                ├─▶ Smile / arbitrage.rs /
  StrikeQuoteSlice ──ln(K/F), σ²t──▶ (k_i, w_i) anchors ──fit──▶ ParametricSlice ─┘   termstructure.rs / VolSurface
```

A runtime `QuoteBasis` enum (matched inside one calibration function) is **rejected**: it would put a per-asset-class `match` into the calibration path — precisely the F5/F6 anti-pattern ADR-0008 removed — and would force the FX arm through rewritten code, jeopardizing byte-identity for zero benefit. Typed front-ends select the basis at compile time; downstream of the fit there is **one** shared, already-neutral machine (§2.1). The wire-side tagged ingestion (`MarkSurfaceRequest` today carries only `repeated BrokerQuoteSet`) is the sanctioned place for a tagged input and is the tracked S7 follow-up (§8) — *not* this lane.

**The only edit to existing files** is pure code motion: hoist the private deterministic solver kit out of `calibrate.rs` into a new crate-internal module so the strike leaf can reuse it.

**File: `crates/celnet-surface/src/fitmath.rs` (NEW, `pub(crate)`)** — move, verbatim (zero arithmetic change, identical token sequence inside each function):

- `pub(crate) fn clamp(x, lo, hi) -> f64`
- `pub(crate) fn sumsq(r: &[f64]) -> f64`
- `pub(crate) fn gauss_newton_2<R, P>(...)`, `gauss_newton_3<R>(...)`, `gauss_newton_4<R, P>(...)`
- `pub(crate) fn solve3(a: &[[f64; 3]; 3], b: &[f64; 3]) -> Option<[f64; 3]>`, `solve4(...)`
- `pub(crate) fn gaussian_eliminate<const N: usize, const C: usize>(...)`
- `pub(crate) const FIT_ITERS: usize = 60;`, `pub(crate) const FIT_FD_H: f64 = 1e-6;`

`calibrate.rs` then adds `use crate::fitmath::{clamp, gauss_newton_2, gauss_newton_3, gauss_newton_4, solve3, solve4, FIT_ITERS, FIT_FD_H};` and deletes the local copies. **Moving a private pure `f64` function between modules cannot change IEEE-754 results** (Rust performs no float reassociation), but the claim is *gated*, not asserted: the §5.1 frozen-bits test pins all five FX model fits across the move. `quotes.rs`, `strangle.rs`, `market_hedge.rs`, `lib.rs` FX entry points: **zero edits**.

**Why `MarketContext` is not generalized further:** making `ConventionRecord` optional would churn the ~60 constructor call sites across ~12 crates (ADR-0008 §4.4) for no numerical gain. A separate light context (§3.1) is additive and carries zero FX risk. Recorded as a decision in the module doc.

---

## 3. The strike-axis leaf (deliverable b)

### 3.1 New module `crates/celnet-surface/src/strike_quotes.rs`

Module doc opens with the convention pin (provenance prose-only):

> Strike-axis (log-moneyness) smile slice from exchange-chain quotes. Digital-asset venues quote European options on a **strike grid** per expiry (e.g. the Deribit BTC/ETH chains — deribit.com/kb: European, coin-settled, expiry cut **08:00 UTC**, premium in coins), not at FX delta pillars. The slice's `t` is the continuous (24×7) year fraction to the venue's UTC expiry cut — crypto has no weekend/holiday skip, so the identity `CalendarClock` is the correct business clock. The forward is formed **only** through the [`celnet_types::Carry`] seam (`F = S·e^{b·t}`, with `b = r − funding` assembled by the crypto leaf's `funding_carry`); no function in this module inspects the carry variant (ADR-0008 no-match-carry). Fit provenance (doc-only): raw-SVI slice of Gatheral (2004); the dimension-reduced "quasi-explicit" calibration of De Marco & Martini (2009): the inner `(a, c, d)` problem is linear least squares solved exactly, the outer search runs over `(m, σ)` only.

```rust
use celnet_core::math::{ln, sqrt};
use celnet_types::Carry;

use crate::fitmath::{clamp, gauss_newton_2, solve3, sumsq, FIT_ITERS};
use crate::parametric::ParametricSlice;
use crate::strangle::CalibrationError;
use crate::surface::{SmileModel, VolSurface};
use crate::termstructure::{CalendarClock, TenorPillar};

/// One exchange-chain quote: a strike and its Black implied volatility.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrikeQuote {
    /// Strike `K` (quote currency per unit of underlying; USD for a coin chain).
    pub strike: f64,
    /// Black implied volatility (absolute, e.g. `0.65` = 65 vols).
    pub vol: f64,
}

/// Minimum quotes for a well-posed five-parameter slice fit.
pub const MIN_STRIKE_QUOTES: usize = 5;

/// A validated one-expiry strike-gridded quote set (ascending strikes).
#[derive(Debug, Clone, PartialEq)]
pub struct StrikeQuoteSlice { quotes: Vec<StrikeQuote> }

impl StrikeQuoteSlice {
    /// # Panics — structural invalidity (house style, cf. `ParametricSlice::new`):
    /// fewer than [`MIN_STRIKE_QUOTES`]; strikes not strictly increasing or
    /// non-positive; vols non-positive / non-finite.
    #[must_use] pub fn new(quotes: Vec<StrikeQuote>) -> Self { /* asserts above */ }
    #[must_use] pub fn quotes(&self) -> &[StrikeQuote] { &self.quotes }
    #[must_use] pub fn len(&self) -> usize { self.quotes.len() }
    #[must_use] pub fn is_empty(&self) -> bool { false } // by construction; keep for clippy len-without-is-empty
}

/// The carry-seam market state for one strike-quoted slice: spot, the
/// cost-of-carry producer, and vol-time. The strike axis needs no delta/ATM
/// conventions — deliberately *not* [`crate::quotes::MarketContext`] (which
/// requires a resolved FX `ConventionRecord`); see the module doc decision note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrikeSliceContext {
    /// Spot price of the underlying.
    pub spot: f64,
    /// Cost-of-carry model (`Carry::CostOfCarry { r, b = r − funding }` for a
    /// digital asset; any `Carry` is accepted — the seam is asset-class-blind).
    pub carry: Carry,
    /// Vol-time to the venue expiry cut, years (continuous 24×7 day count).
    pub t: f64,
}

impl StrikeSliceContext {
    /// # Panics — non-positive `spot` or `t`.
    #[must_use] pub fn new(spot: f64, carry: Carry, t: f64) -> Self { /* asserts */ }
    /// Outright forward `F = S·e^{b·t}` — the IDENTICAL op sequence as
    /// `MarketContext::forward` (`spot * carry.forward_factor(t)`).
    #[must_use] pub fn forward(&self) -> f64 { self.spot * self.carry.forward_factor(self.t) }
    /// Total variance `σ²·t` helper (mirrors `MarketContext::atm_total_variance`).
    #[must_use] pub fn total_variance(&self, vol: f64) -> f64 { vol * vol * self.t }
}

/// A fitted strike-axis slice plus its reproduction diagnostics. The fit never
/// hides quality behind a threshold: callers gate on the reported errors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrikeSliceFit {
    /// The fitted arbitrage-projected raw-SVI slice (anchored at `F`, `t`).
    pub slice: ParametricSlice,
    /// Root-mean-square absolute vol error over the input quotes.
    pub rms_vol_error: f64,
    /// Worst absolute vol error over the input quotes.
    pub max_vol_error: f64,
}

pub fn fit_strike_slice(
    ctx: &StrikeSliceContext,
    quotes: &StrikeQuoteSlice,
) -> Result<StrikeSliceFit, CalibrationError> { /* §3.2 */ }

/// Assemble per-expiry strike-axis fits into the unified re-strikable surface
/// (`SmileModel::Parametric` pillars on the identity clock — the 24×7 clock).
/// `max_vol_error` is the per-slice reproduction gate (absolute vols); a slice
/// exceeding it maps to `CalibrationError::NoConvergence` — never a silent
/// garbage pillar.
pub fn strike_surface(
    pillars: &[(StrikeSliceContext, StrikeQuoteSlice)],
    max_vol_error: f64,
) -> Result<VolSurface<ParametricSlice, CalendarClock>, CalibrationError> {
    // fit each, gate max_vol_error, then
    // VolSurface::new(SmileModel::Parametric,
    //     pillars.map(|(c, _, fit)| TenorPillar::new(fit.slice, c.forward(), c.t)))
}
```

`lib.rs` additions: `pub mod strike_quotes;` + `pub use strike_quotes::{StrikeQuote, StrikeQuoteSlice, StrikeSliceContext, StrikeSliceFit, MIN_STRIKE_QUOTES, fit_strike_slice, strike_surface};` + crate-doc retitle (the crate is the *asset-class-neutral* surface with two quote-basis front-ends; keep the FX pipeline doc intact, add a "Strike-axis leaf" section). Update `Cargo.toml` `description` accordingly. One doc-comment touch in `strangle.rs`: widen `CalibrationError::NoConvergence`'s doc to "an iterative calibration (strangle fixed point or slice fit) did not converge / reproduce within budget" — enum *shape* unchanged.

### 3.2 The fit — math and derivation (cite-checked, re-derivable)

**Anchors.** With `F = ctx.forward()` (Carry seam) and `t = ctx.t`, each quote lowers to the neutral plane exactly as the FX `anchors()` does:

```
k_i = ln(K_i / F)          (celnet_core::math::ln)
w_i = vol_i · vol_i · t    (total implied variance)
```

**Model.** Raw-SVI total variance (Gatheral 2004; Gatheral–Jacquier 2014 eq. 3.1 — re-typed in the oracle, §4):

```
w(k) = a + b·[ ρ·(k − m) + √((k − m)² + σ²) ],   b ≥ 0, |ρ| < 1, σ > 0.
```

**Dimension reduction (the quasi-explicit decomposition).** For fixed `(m, σ)` substitute `y_i = (k_i − m)/σ`, `z_i = √(y_i² + 1)`; then `w = a + c·y + d·z` with `c = bρσ`, `d = bσ` — **linear** in `(a, c, d)`. The inner problem `min Σ (a + c·y_i + d·z_i − w_i)²` is solved exactly by the 3×3 normal equations (deterministic ascending-`i` accumulation, plain `f64` sums):

```
| n    Σy    Σz  | |a|   | Σw  |
| Σy   Σy²   Σyz | |c| = | Σyw |       → fitmath::solve3
| Σz   Σyz   Σz² | |d|   | Σzw |
```

**Constraint projection on the inner solution** (each clamp is a no-op when the unconstrained optimum is admissible — exact ground truth recovers exactly):

1. solver failure / non-finite `(a, c, d)` → return the constant residual vector `[1e6; n]` (the Levenberg guard in `gauss_newton_2` then rejects the step deterministically);
2. `d ← max(d, 0.0)`; if `d == 0` then `c ← 0` (flat slice, `b = 0`);
3. `|ρ| ≤ 1`: if `|c| > d` then `c ← c.signum()·d`;
4. **Lee wing bound** `b(1+|ρ|) ≤ 2` ⇔ `(d + |c|)/σ ≤ 2`: if violated, scale `s = 2σ/(d + |c|)`; `c ← c·s; d ← d·s` (preserves `ρ`, caps the slope — same dimensionless constant 2 the FX `project_svi` enforces, no `t`);
5. **non-negative minimum variance** `w_min = a + bσ√(1−ρ²) = a + √(d² − c²) ≥ 0` (step 3 guarantees `d ≥ |c|`): if `w_min < 0`, lift `a ← a − w_min`.

**Outer search.** Profile the objective over `(m, σ)` only and reuse `fitmath::gauss_newton_2` verbatim (fixed `FIT_ITERS = 60`, forward-difference Jacobian, Levenberg damping, residual-decrease acceptance — bit-reproducible). Inside `residuals(m, σ)`: run the inner solve **including the projection**, then return `r_i = a + c·y_i + d·z_i − w_i` (the projected-profile residual — projection must be identical in the residual closure and the final extraction). Box projection: `m ∈ [k_1 − 0.5, k_n + 0.5]`, `σ ∈ [1e-4, 5.0]` (matching the FX `project_svi` σ cap). Deterministic seeds: `m₀ = k_{argmin w}` (first occurrence), `σ₀ = clamp(0.25·(k_n − k_1), 1e-2, 1.0)`.

**Extraction.** At the final `(m, σ)` recompute the projected inner solution; map `b = d/σ`, `ρ = if d > 1e-12 { c/d } else { 0.0 }`, then `ρ ← clamp(ρ, −0.999_999, 0.999_999)` (a no-op for any market-plausible fit; satisfies the strict `|ρ| < 1` constructor assert). Guards (mirroring `fit_svi`): any non-finite parameter, `σ ≤ 0`, or `w_min < −1e-12` → `Err(CalibrationError::DegenerateQuote)`. Otherwise construct **`ParametricSlice::new(a, b, ρ, m, σ, F, t)`** — the same validated type the FX fits return, so *everything downstream is shared*: `Smile`, `butterfly_density_factor`, `satisfies_wing_bound`, `check_slice`, `TenorPillar`, `VolSurface::arbitrage_report`.

Diagnostics: `vol_err_i = |√(w_fit(k_i)/t) − vol_i|`; `rms_vol_error = √(Σ vol_err_i²/n)`, `max_vol_error = max_i vol_err_i`.

**Determinism:** libm-only via `celnet_core::math` + `mathx`, fixed iteration budget, no RNG/clock/global state — same bit-reproducibility contract as `calibrate.rs` (gated by a test, §5.2). Allocation note: calibration is the marking path, not the pinned zero-alloc hot core; `Vec` use matches the existing `calibrate.rs` idiom.

### 3.3 Term structure & 24×7 semantics

No new clock: `CalendarClock` *is* the crypto clock (identity `τ(t) = t`; crypto expiries are exchange-fixed UTC instants, no weekend/holiday deweighting — module doc states this with the 08:00 UTC cut identity, value-free). `strike_surface` builds `TenorPillar::new(fit.slice, ctx.forward(), ctx.t)` per expiry; calendar no-arbitrage is *reported* by the existing fixed-strike `min_calendar_increment` / `arbitrage_report` (consistent with the FX surface: assemble, report, let the publish gate decide — never silently repair). The FX `EventClock` backlog item (SURFACE-WORKFLOW §10.8) is untouched.

### 3.4 The funding-carry forward

The leaf takes `Carry` opaquely. A crypto caller assembles it with the existing `celnet_crypto_vanilla::funding_carry(r, funding)` (`Carry::CostOfCarry { r, b: r − funding }`); the slice forward is `spot · carry.forward_factor(t)` — bit-identical to `spot · e^{(r−funding)·t}` with `b` formed once in `funding_carry` (gated §4-O4). `celnet-surface` gains **no runtime dependency** on `celnet-crypto-vanilla` (the leaves stay siblings); it is added as a **dev-dependency only**, mirroring how `celnet-crypto-vanilla` dev-deps `celnet-vanilla`:

```toml
[dev-dependencies]
# The crypto vanilla leaf — used ONLY by tests as (1) the funding-carry assembler
# exercised through the seam and (2) the published venue-convention pins
# (the `deribit` identity constants). NOT a runtime dependency: leaves are siblings.
celnet-crypto-vanilla.workspace = true
```

---

## 4. The independent oracle (deliverable c)

Five independent legs; none reuses production assembly; at least two can disagree with the production fitter by construction.

- **O1 — Injected ground truth (the disagree-capable leg).** Synthetic grids are generated by an **in-test re-typed** raw-SVI formula (`svi_w_reference(k, a, b, rho, m, sigma)` written from the published Gatheral/Gatheral–Jacquier equation, *not* calling `ParametricSlice::total_variance`), at known arb-free parameters. The fitter never sees the truth parameters — only `(K_i, σ_i)`. Recovery failure = loud disagreement. This is the surface-leaf analogue of the W3 splitmix64-MC discipline: the data source has zero analytic structure shared with the fitter's solve path.
- **O2 — Independent linear algebra.** At the *fitted* `(m, σ)` (read from the returned `ParametricSlice.m/.sigma` public fields), the test re-derives the 3×3 normal equations from scratch and solves them by **Cramer's rule** (independent of production `gaussian_eliminate`); asserts the production-implied `(a, c = bρσ, d = bσ)` match within `1e-9`. Catches a transposed/sign-flipped normal-equation assembly in either implementation.
- **O3 — Density law re-derivation.** Breeden–Litzenberger second-difference density of the fitted slice computed with an **in-test re-typed** undiscounted Black forward call (the `surface_properties.rs` pattern — independent of `arbitrage.rs`), required `≥ −1e-7` across `k ∈ ±2·max(σ_ATM√t, σ_truth-span)`; cross-checked against the closed-form Durrleman `g ≥ 0` claim of `is_butterfly_free`. Two independent no-arb notions must agree.
- **O4 — Carry-seam byte identity.** `StrikeSliceContext::forward().to_bits() == (spot * libm::exp((r − funding) * t)).to_bits()` with the carry built by `celnet_crypto_vanilla::funding_carry` (the test recomputes `r − funding` with the identical expression); and, for a `Carry::FxRates` context, `StrikeSliceContext::forward().to_bits() == MarketContext::forward().to_bits()` — the strike context accepts any carry with no asset-class branch (neutrality proof, not just a crypto fact).
- **O5 — Cross-path can-disagree canary vs the FX path.** Calibrate the FX delta path (`build_model_smile(SmileModel::Parametric, …)` on EURUSD broker quotes) → an exact SVI slice. Sample 9 strikes spanning `[0.85F, 1.20F]` off that slice, feed them to `fit_strike_slice` **as a strike grid** (with the same `Carry::FxRates` — allowed, the leaf is basis-defined, not asset-defined). (i) *Agreement when they must agree:* `|w_strike(k) − w_fx(k)| ≤ 1e-7` across `k ∈ [−0.25, 0.25]` (the strike fit of an exact SVI recovers it). (ii) *Disagreement when they must disagree:* bump one strike quote by `+0.01` (one vol point) and refit — the strike-path `w` at that `k` must move by `> 1e-4` while the FX-path slice (unchanged broker quotes) stays **bit-identical**. Proves the two front-ends are driven by disjoint inputs — the strike path cannot be secretly reading the FX broker machinery, and the oracle pair can disagree.
- **Convention pins (published reference, identity-only).** Via the dev-dep, re-assert the venue identities the leaf's `t`/axis semantics rest on: `deribit::EXPIRY_CUT_UTC_SECONDS == 28_800` (08:00 UTC), `deribit::STYLE_EUROPEAN`, `deribit::SETTLEMENT_IS_INVERSE_COIN` — citation in the test header; live quote/fixing **values** remain ENV (W3 §9 honest boundary).

---

## 5. Gate tests (deliverable d)

### 5.1 FX `to_bits` regression — `crates/celnet-surface/tests/fx_fit_pin.rs` (NEW, lands **before** the refactor)

Pins the *complete* FX calibration pipeline output bits across the §2.3 code motion (and any future surface change):

- Fixed fixtures: EURUSD-1Y conventions, `spot = 1.10`, `Carry::FxRates { r_dom: 0.02, r_for: 0.01 }`, `t = 1.0`; quote set A `MarketQuotes::five_point(0.11, -0.02, 0.006, -0.035, 0.012)` (pronounced skew) and set B `MarketQuotes::three_point(0.10, -0.005, 0.0025)` (mild).
- For each of the **five** `SmileModel`s × each quote set, evaluate `build_model_smile(...).implied_vol(F·x, F, t).0` at `x ∈ {0.85, 0.95, 1.00, 1.05, 1.15}` → 50 `u64` pins asserted with `assert_eq!(got.to_bits(), PIN)`.
- Generation choreography (no fakery): the file also contains `#[test] #[ignore] fn emit_pins()` printing the literal pin table (`--nocapture`). **D0** runs it once against *unmodified* `main` code, pastes the constants, commits green. The ignored emitter stays in-tree for sanctioned regeneration when quotes are intentionally changed (documented in the test header). Pins are platform-portable because every fit transcendental routes through libm (the crate's existing bit-reproducibility contract).
- Existing FX byte-identity tests stay green untouched: `quotes.rs::fx_carry_context_is_byte_identical_to_two_rate_form` (Carry seam `to_bits` grid) and `calibrate.rs::calibration_is_bit_reproducible`.

### 5.2 In-crate unit tests (`strike_quotes.rs::tests`)

| Test | Asserts |
|---|---|
| `context_forward_is_carry_seam_bits` | O4 both halves (`funding_carry` bits; `FxRates` parity with `MarketContext::forward` bits). |
| `anchors_match_hand_computed` | `k_i`/`w_i` vs in-test `ln`/`σ²t` re-computation, `to_bits` equal. |
| `exact_svi_grid_round_trips` | O1 on a BTC-scale fixture (`spot = 60_000`, `r = 0.05`, `funding = 0.02`, `t = 0.25`; truth `a = 0.012, b = 0.08, ρ = −0.4, m = 0.02, σ = 0.15`; 13 log-spaced strikes 40k–90k): on-grid `max_vol_error ≤ 5e-7`; `|w_fit − w_truth| ≤ 1e-7` at 41 off-grid `k ∈ ±0.6`; parameters within `1e-4` (relative for `b, σ`; absolute for `ρ, m`; `a` within `1e-6`). |
| `flat_grid_fits_flat` | Constant-vol grid reproduces the flat vol ≤ `1e-10`; slice butterfly-free. |
| `fit_is_bit_reproducible` | Two identical runs: fitted-slice fields and `implied_vol` bits equal. |
| `skewed_grid_reproduces_skew_direction` | Put-skewed grid ⇒ `ρ < 0`, put-wing vol > call-wing vol. |
| `hostile_grid_stays_butterfly_free` | A V-shaped (negative-density) grid: fit returns Ok with **large** `max_vol_error` (the no-arb projection refuses to chase arbitrage), and the fitted slice still passes `is_butterfly_free(2.0, 1e-6)` + `satisfies_wing_bound()`. |
| `structural_invalidity_panics` | `#[should_panic]` for < 5 quotes / unsorted strikes / non-positive vol / non-positive `spot`/`t`. |
| `surface_gates_bad_pillar` | `strike_surface` maps an over-budget pillar to `CalibrationError::NoConvergence`. |

### 5.3 Integration oracle — `crates/celnet-surface/tests/strike_axis_oracle.rs` (NEW)

O1 (a second, ETH-scale fixture), O2 (Cramer cross-check), O3 (FD density vs Durrleman g), O5 (cross-path canary), the Deribit identity pins, plus the 24×7 term-structure leg: two expiries (`t = 7/365`, `28/365`, monotone θ) → `strike_surface(..., 1e-4)` → `arbitrage_report(0.5, 81, 8, 1e-3).is_arbitrage_free(1e-4)`; a decreasing-θ pair flags `min_calendar_increment < 0`.

### 5.4 Parity property row — `crates/celnet-parity/tests/strike_surface.rs` (NEW)

House standard "over a domain, not a point" (proptest, 256 cases): random arb-free SVI truths (`a ∈ [1e-4, 0.05]`, `b ∈ [0.01, 0.5]`, `ρ ∈ [−0.8, 0.8]`, `m ∈ [−0.2, 0.2]`, `σ ∈ [0.05, 0.5]`, filtered to `b(1+|ρ|) ≤ 2 ∧ w_min > 0`; `F ∈ [100, 1e5]`, `t ∈ [0.02, 1.0]`), 11-point grids `k ∈ ±0.6` generated from the in-test reference formula → fit → assert: on-grid `max_vol_error ≤ 1e-4` (0.01 vol point), butterfly-free, wing bound, FD density `≥ −1e-6`. If a corner exceeds the GN budget: investigate first; the sanctioned narrowing is tightening the proptest domain with a comment — **never** loosening below `1e-4`.

### 5.5 Gates to run (in session-B-quiet windows only; serialize heavy cargo per the mesh-coordination memory)

Per increment `just check-crate celnet-surface` (D5 adds `just check-crate celnet-parity`); milestone = full `just check` with the literal **"All gates passed."** line verified (not the wrapper exit code).

---

## 6. Dependency order (each commit independently green)

- **D0 — FX pin capture (pre-refactor, must be first):** add `tests/fx_fit_pin.rs`; run `emit_pins` against unmodified code; freeze the 50 pins; gate; commit.
- **D1 — code motion:** add `src/fitmath.rs`; `calibrate.rs` switches to imports (no other edit anywhere); gate incl. `fx_fit_pin` green ⇒ byte identity proven; commit.
- **D2 — the leaf:** add `src/strike_quotes.rs` (types + fit + `strike_surface` + unit tests), `lib.rs` module/exports/crate-doc, `Cargo.toml` description + dev-dep, the one-line `NoConvergence` doc widening; gate; commit.
- **D3 — oracle:** add `tests/strike_axis_oracle.rs` (O1–O5 + convention pins + term-structure leg); gate; commit.
- **D4 — parity row:** add `crates/celnet-parity/tests/strike_surface.rs`; gate `celnet-parity`; commit.
- **D5 — docs + milestone:** sync `docs/W3-CRYPTO-PLAN.md` §7-S6 (mark the in-crate leaf Built; S7 wire ingestion stays tracked), `docs/INTERFACES.md` (if it enumerates `celnet-surface` exports — verify, don't assume), ledger top entry + `PARALLEL-SESSIONS.md` lane close + memory; full-workspace `just check`; milestone commit (graph re-indexes via hooks).

---

## 7. Files-touched manifest

**New:** `crates/celnet-surface/src/fitmath.rs`; `crates/celnet-surface/src/strike_quotes.rs`; `crates/celnet-surface/tests/fx_fit_pin.rs`; `crates/celnet-surface/tests/strike_axis_oracle.rs`; `crates/celnet-parity/tests/strike_surface.rs`.
**Edited:** `crates/celnet-surface/src/calibrate.rs` (imports only — solver kit moved out verbatim); `crates/celnet-surface/src/lib.rs` (module + re-exports + crate doc); `crates/celnet-surface/Cargo.toml` (description; dev-dep `celnet-crypto-vanilla`); `crates/celnet-surface/src/strangle.rs` (one doc comment on `NoConvergence`); docs per D5.
**Untouched (load-bearing for FX byte identity):** `quotes.rs`, `strangle.rs` code, `market_hedge.rs`, `parametric*.rs`, `extended_surface.rs`, `arbitrage.rs`, `termstructure.rs`, `surface.rs`, all FX entry points in `lib.rs`, every other crate's runtime code, the wire contract, all golden vectors.

---

## 8. Out of scope — tracked tail (named, never silent)

1. **Wire ingestion of strike-axis quotes** (`MarkSurfaceRequest` today carries only `repeated BrokerQuoteSet`; the tagged `oneof` quote basis is the sanctioned wire-side seam) + server routing + 5-client surfacing — W3 §7-S7..S9, proto-window-gated (session-B holds the window).
2. **Joint multi-expiry eSSVI fit from strike grids** (per-expiry `(θ, ρ, ψ)` into `ExtendedSurface` with the Hendriks–Martini calendar conditions enforced *across* slices at fit time, not just reported) — additive on top of this leaf; the per-slice SVI + `TermStructure` report is the complete, honest v1.
3. **FX `EventClock`** (SURFACE-WORKFLOW §10.8) — unrelated to the 24×7 path, which needs no clock work.
4. **Live crypto chain/funding/fixing values** — ENV per the W3 §9 honest boundary; only math, axis shape, and convention identity are in-repo.

## 9. Honest scope & risk

- **Depth risk: low.** The fit is the published dimension-reduced calibration on top of an already-gated solver kit; everything downstream of `ParametricSlice` is reused, already-verified machinery.
- **FX risk: ~zero by construction, and gated anyway.** The FX path's only change is an import path; the D0/D1 pin choreography turns "should be identical" into a `to_bits` test.
- **The sharp numerical edge** is the inner-projection consistency (the projected solution must be used both inside the GN residual closure and at extraction — a mismatch silently biases fits while staying green on mild data). The O2 Cramer cross-check at the *final* `(m, σ)` is the targeted guard.
- **No GPU/perf implications:** marking-path code, off the pinned hot core; batch re-mark cost characteristics match the existing FX fits (same iteration budget, 3×3 solves).

