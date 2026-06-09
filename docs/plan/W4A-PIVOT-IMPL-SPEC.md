# W4-A-PIVOT — Pivot Target-Redemption Accumulator

**Lane:** W4-A-PIVOT. **Owns (new files only, code-disjoint):**
- `crates/celnet-exotics/src/pivot.rs` (NEW module)
- `crates/celnet-exotics/tests/pivot_oracle.rs` (NEW integration test, the code-disjoint oracle lives here)
- 6 lines added to `crates/celnet-exotics/src/lib.rs` (one `pub mod` + one `pub use` re-export + one crate-doc bullet — additive, no edits to existing items)

**Read but not modified:** `tarf.rs`, `accumulator.rs`, `mc.rs`, `rng.rs`, `normal.rs`, `lib.rs` scaffolding, `celnet-types` `VanillaInputs`, `celnet-core::math`.

This spec is precise enough to code mechanically: exact types, fns, math with derivation, the independent (can-disagree) oracle, the degenerate→TARF 1e-12 limit, the gate tests, and the build order.

---

## 1. Product definition — the "pivot TRA" family

A **Pivot Target-Redemption Accumulator (TRA)** generalizes the existing `Tarf` along the one axis the existing `Tarf` and `Accumulator` each handle separately but never together: it has **two strike levels that both bind on every fixing** — a *pivot* `P` and a *target strike* `K` — plus the cumulative-target redemption mechanic.

On each of `n` equally-spaced fixings the realised spot `S_k` produces, **for the client**, a per-unit cash flow `c_k` that is a *piecewise-linear* function of `S_k` with a **kink at the pivot** `P`:

```
                favourable side (gain accrues to target)
                         │
 c_k(S) =  + (S − K)     │   for S ≥ P     (above pivot: enhanced long leg)
           − L·(K − S)   │   for S <  P     (below pivot: geared short leg)
                         P
```

written here for the **call-favourable** orientation (`favourable_side = Call`, client gains when spot rises). For the **put-favourable** orientation (the classic exporter TARF) every comparison and sign mirrors (see §3.4). The two economically distinct levels are:

- **`K` (target strike)** — the level the *intrinsic* `S − K` is measured against; the running sum of the **positive** part of `c_k` accrues toward the cumulative target `T`.
- **`P` (pivot)** — the level at which the **leg switches** from the un-geared favourable leg to the `L`-geared adverse leg. The pivot is what makes this a genuinely richer product than `Tarf`: in `Tarf` the kink and the intrinsic reference coincide (`P ≡ K`), so the favourable and adverse legs meet exactly at `K`. Here `P` can sit **away from** `K`, opening a *dead band* (`P > K`: a corridor `[K, P]` where the client is mildly long but the gearing has not yet engaged) or an *overlap* (`P < K`: the client is already paying intrinsic before the gearing kicks in). This is the standard "pivot accumulator" / "boosted TARF" structuring lever.

**Redemption (target knock-out):** identical mechanic to `Tarf`. The cumulative *favourable* gain `G = Σ max(c_k, 0)` accrues; once `G ≥ T` the structure redeems (no further fixings). The breaching fixing settles per `RedemptionStyle` (reuse the existing enum):
- `FullGain` — pays its full positive `c_k` (overshoot kept by client → gap risk);
- `CappedGain` — pays only the remaining target `T − G_prev` (exact redemption, no overshoot).

**Sign / PV convention:** to make the degenerate→TARF gate byte-comparable, `pivot.rs` reports the **bank's** present value, exactly as `tarf.rs` does. Per path the bank receives the geared adverse legs and pays away the favourable legs; each leg is discounted to its own fixing date at `r_dom`.

### 1.1 Degeneracy map (the whole point of the design)

The family is built so that **two independent parameter limits each collapse it to an existing, already-validated product**, giving two free regression oracles:

1. **`P = K`  ⇒  exact `Tarf`.** When the pivot equals the target strike, the kink and intrinsic reference coincide, the dead band/overlap vanishes, and `c_k = (S−K)` above and `−L(K−S)` below with the switch at `K`. This is **bit-for-bit** the `tarf.rs` payoff. Gated to_bits / 1e-12 against `tarf::tarf_price` (§5.1).
2. **`T = +∞` (unreachable target) with `K = P`  ⇒  geared forward strip** (the `Tarf::unreachable_target` limit). Secondary sanity bound (§5.5).

The pivot family therefore **contains** `Tarf` as a codimension-1 slice; it does not duplicate it — it adds the second level `P` and collapses onto `tarf.rs` exactly when `P=K`.

---

## 2. Module `pivot.rs` — types and public API

All names are purpose-named, vendor/person-neutral (guardrail #8). Method provenance (Wystup 2017; Caspers 2014 for TARF gap risk; the pivot/boosted variant is standard desk structuring) appears **only** in the module doc comment.

```rust
//! Pivot Target-Redemption Accumulator (pivot TRA) — a strip of periodic fixings
//! with a TWO-level piecewise-linear per-fixing payoff (a `pivot` P at which the
//! geared adverse leg engages, distinct from the `strike` K against which intrinsic
//! is measured) and a cumulative-gain `target` redemption (knock-out on target)
//! with explicit gap-risk handling at the breaching fixing.
//!
//! Collapses to the plain `crate::tarf::Tarf` exactly when `pivot == strike`
//! (gated to 1e-12 in tests). Priced on the shared counter-based MC engine with
//! antithetic variates + a forward-strip control variate; reproducible from seed.
//!
//! Provenance (doc-only): TARF payoff & gap-risk decomposition — Wystup (2017),
//! *FX Options and Structured Products*; Caspers (2014). Pivot/boosted accumulator
//! structuring — Wystup (2017). Identifiers are purpose-named & vendor-neutral.

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::{OptionType, VanillaInputs};

use crate::normal::inverse_cdf;
use crate::rng::CounterRng;
use crate::tarf::RedemptionStyle; // reuse the frozen gap-risk enum — no duplication
```

### 2.1 Spec struct

```rust
/// A Pivot Target-Redemption Accumulator specification.
///
/// `favourable_side` is the side of `strike` on which the client accrues gains:
/// `OptionType::Call` ⇒ gain when `S_k > strike`; `OptionType::Put` ⇒ gain when
/// `S_k < strike` (the classic exporter orientation). The `pivot` is the level at
/// which the geared adverse leg engages; setting `pivot == strike` recovers the
/// plain TARF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PivotTra {
    /// Target strike `K`: the level intrinsic `S − K` is measured against, and the
    /// reference for the cumulative gain that accrues toward `target`.
    pub strike: f64,
    /// Pivot `P`: the kink at which the per-fixing leg switches from the un-geared
    /// favourable leg to the geared adverse leg. `pivot == strike` ⇒ plain TARF.
    pub pivot: f64,
    /// Number of equally-spaced fixing dates over `[0, T]` (the last at `T`).
    pub fixings: usize,
    /// Cumulative gain target. Accrued client gain at or above this redeems.
    pub target: f64,
    /// Gearing/leverage on the adverse leg (the far side of the pivot). `≥ 0`.
    pub leverage: f64,
    /// The side of `strike` on which the client accrues gains.
    pub favourable_side: OptionType,
    /// Per-fixing notional (units of base per fixing).
    pub notional: f64,
    /// Gap-risk settlement convention of the redeeming fixing.
    pub redemption: RedemptionStyle,
}

impl PivotTra {
    fn validate(&self) {
        assert!(self.fixings >= 1, "pivot TRA needs ≥1 fixing");
        assert!(self.target > 0.0, "pivot TRA target must be positive");
        assert!(self.leverage >= 0.0, "pivot TRA leverage must be non-negative");
        assert!(self.notional > 0.0, "pivot TRA notional must be positive");
        assert!(self.strike > 0.0 && self.pivot > 0.0, "strike/pivot must be positive");
    }

    /// The degenerate slice equal to a plain TARF — the canonical way to take the
    /// `P = K` limit in tests.
    #[must_use]
    pub fn as_tarf_slice(strike: f64, fixings: usize, target: f64, leverage: f64,
                         favourable_side: OptionType, notional: f64,
                         redemption: RedemptionStyle) -> Self {
        Self { strike, pivot: strike, fixings, target, leverage, favourable_side,
               notional, redemption }
    }
}
```

### 2.2 Config + result (mirror `Tarf` exactly so the gate is apples-to-apples)

```rust
#[derive(Debug, Clone, Copy)]
pub struct PivotTraMcConfig { pub pairs: usize, pub seed: u64 }

#[derive(Debug, Clone, Copy)]
pub struct PivotTraResult {
    /// Discounted present value to the **bank** (seller). Positive = value to bank.
    pub price: f64,
    /// Standard error of the mean of the present value.
    pub std_error: f64,
    /// Expected (fractional) fixing index at which the structure redeems, or
    /// `fixings` if it never redeems on average.
    pub expected_redemption_fixing: f64,
    /// Expected realised gain overshoot beyond the target (gap exposure; identically
    /// zero under `RedemptionStyle::CappedGain`).
    pub expected_overshoot: f64,
}
```

A private `Welford { n, mean, m2 }` — copy the 12-line numerically-stable accumulator verbatim from `tarf.rs`. Duplicating a private helper inside a leaf module matches the existing `tarf.rs`/`accumulator.rs` pattern (each carries its own copy) and keeps the module self-contained.

### 2.3 Pricer

```rust
#[must_use]
pub fn pivot_tra_price(i: &VanillaInputs, spec: PivotTra, cfg: PivotTraMcConfig)
    -> PivotTraResult
```

Body (mirrors `tarf_price`, the **production** path):

1. `spec.validate();`
2. `n = spec.fixings; dt = i.t / n; ln_s0 = ln(i.spot);`
3. `drift_step = (i.r_dom − i.r_for − 0.5·vol²)·dt;  vol_sqrt_dt = vol·√dt;`
4. Per-fixing discounts `dfs[k] = exp(−r_dom·(k+1)·dt)`.
5. Loop `pair = 0..cfg.pairs`: `rng = CounterRng::new(cfg.seed, 0, pair, 0)`; fill `z[0..n]` with `inverse_cdf(rng.next_u01())`; walk `+z` and `−z` via `walk_path(...)`, push the antithetic mean of `(bank_pv, redeem_index, overshoot)` into three `Welford`s.
6. Return `PivotTraResult`.

**Critical determinism note (so the degenerate gate hits to_bits/1e-12):** to reproduce `tarf.rs` *bit-for-bit* in the `P=K` slice, the production path must use the **same RNG coordinates and the same arithmetic order** as `tarf.rs`: `CounterRng::new(seed, 0, pair, 0)`, fill all `z` first, then two `walk_path` calls (`+1.0`, `−1.0`), antithetic mean `0.5·(a+b)`. Do **not** fold the control variate into the default pricer's reported `price` — the default pricer reports the plain antithetic mean (the control variate is a separate `pivot_tra_price_cv` entry point, §4) so the to-bits gate holds.

### 2.4 `walk_path` — the production payoff

```rust
struct PathOutcome { bank_pv: f64, redeem_index: f64, overshoot: f64 }

fn walk_path(ln_s0, drift_step, vol_sqrt_dt, z: &[f64], dfs: &[f64],
             spec: PivotTra, sign: f64) -> PathOutcome
```

Per-fixing per-unit client cash flow with the **two levels**, favourable-side sign `g = spec.favourable_side.sign()` (`+1` call, `−1` put):

```
ln_s += drift_step + vol_sqrt_dt·sign·z[k];   s = exp(ln_s);

let d_strike = g * (s - spec.strike);   //  favourable intrinsic > 0 when in gain
let d_pivot  = g * (s - spec.pivot);    //  > 0 on the favourable side of the pivot

// Leg SELECTED by the pivot (kink), VALUED by the strike (intrinsic):
let c = if d_pivot >= 0.0 {
    d_strike                       // favourable side of pivot: un-geared
} else {
    -spec.leverage * (-d_strike)   // adverse side of pivot: geared (= leverage*d_strike)
};
```

**Corner cases (where the family earns its keep, and where a naive `tarf.rs` copy would be wrong):**

- **`P = K` (TARF slice):** `d_pivot ≥ 0 ⇔ d_strike ≥ 0`. Above: `c = d_strike ≥ 0`. Below: `c = leverage·d_strike` with `d_strike < 0` ⇒ geared loss. **Identical to `tarf.rs`** branch-for-branch: `tarf.rs` uses `signed = g·(s−K)`, pays `signed` when `>0` and the bank receives `leverage·(−signed)` when `<0`; here `c` above `= signed`, and the adverse branch (below) pays the bank `(−c) = leverage·(−signed)` — same multiplications, same operand order. **This identity is the to_bits/1e-12 gate.**
- **Dead band `P > K` (call-favourable):** for `K ≤ S < P`, `d_strike ≥ 0` but `d_pivot < 0`, so `c = leverage·d_strike ≥ 0` — the geared branch with a *positive* intrinsic (client in the money, desk has geared the position). The accrual rule keys off **`sign(c)`**, so this is a favourable accrual, handled uniformly.
- **Overlap `P < K` (call-favourable):** for `P ≤ S < K`, `d_pivot ≥ 0` but `d_strike < 0`, so `c = d_strike < 0` un-geared — a small un-geared loss, handled by the `sign(c)` rule.

**Uniform accrual + redemption (identical structure to `tarf.rs`, keyed on `c`):**

```
let raw_gain = c;                       // signed per-unit client cash flow
if raw_gain > 0.0 {
    let remaining = spec.target - accumulated_gain;
    if raw_gain >= remaining {          // breach ⇒ redemption
        let settled = match spec.redemption {
            FullGain   => raw_gain,
            CappedGain => remaining,
        };
        bank_pv -= settled * spec.notional * dfs[k];
        let overshoot = match spec.redemption {
            FullGain   => (raw_gain - remaining).max(0.0),
            CappedGain => 0.0,
        };
        return PathOutcome { bank_pv, redeem_index: (k+1) as f64, overshoot };
    }
    bank_pv -= raw_gain * spec.notional * dfs[k];
    accumulated_gain += raw_gain;
} else if raw_gain < 0.0 {
    bank_pv += (-raw_gain) * spec.notional * dfs[k];   // adverse (gearing already in c)
}
// raw_gain == 0: no cash flow, no accrual.
```

> **Gearing-already-applied subtlety:** `c` already contains `leverage` on the geared branch, so the adverse branch pays the bank `(−raw_gain)` *without* re-multiplying. At `P=K` this equals `tarf.rs`'s `leverage·(−signed)` to the bit because `raw_gain = leverage·signed` there. Compute `c = -spec.leverage * (-d_strike)` as a single expression so the operand order matches `tarf.rs`'s `spec.leverage * (-signed)`. **The reviewer MUST add a `to_bits` assertion in §5.1 rather than trust this prose**; if the orders disagree at the bit level, mirror `tarf.rs` literally in the geared branch. Prefer making the general expression match so there is no `P==K` special-case.

---

## 3. The math, derived

### 3.1 Path law (exact, no time-step bias between fixings)

Under the domestic risk-neutral measure the spot is GBM with drift `b = r_dom − r_for`:
`S_k = S_{k−1}·exp[(b − ½σ²)·dt + σ·√dt·Z_k]`, `Z_k ~ N(0,1)` iid, `dt = T/n`, `t_k = k·dt`, `k=1..n`. Each fixing uses the **exact** lognormal increment (not Euler), so the only error is statistical — identical to `tarf.rs`/`accumulator.rs`.

### 3.2 Per-fixing payoff (the two-level kink), formal

```
c_k(S) = 𝟙[g·(S−P) ≥ 0] · g·(S−K)  −  𝟙[g·(S−P) < 0] · L·g·(S−K)
```

— piecewise-linear in `S` with slope `g` on the favourable side of `P` and slope `L·g` on the adverse side; continuous only when `P=K` (the pieces meet at `c=0` at `S=K=P`). For `P≠K` there is a **jump** of size `(L−1)·g·(P−K)` at `S=P` — the structural discontinuity that distinguishes the pivot family and that the oracle must reproduce.

### 3.3 Cumulative target and redemption

Accrued gain after fixing `m` (pre-redemption): `G_m = Σ_{k≤m} max(c_k, 0)`. Redemption at the first `m*` with `G_{m*} ≥ T`. Bank PV per unit notional (seller):

```
V_bank = − E[ Σ_{k=1}^{m*}  settle_k · e^{−r_dom·t_k} ]
settle_k    = c_k                          for k < m*
settle_{m*} = c_{m*}        (FullGain)  | (T − G_{m*−1})  (CappedGain)
```

`settle_k` is signed (positive ⇒ client receives ⇒ bank pays; negative ⇒ bank receives). Overshoot `= max(c_{m*} − (T − G_{m*−1}), 0)` under FullGain, `0` under CappedGain.

### 3.4 Put-favourable orientation (mirror)

For `favourable_side = Put` (`g = −1`): client gains when `S < K`, geared adverse leg on the `S > P` side. All formulae hold with `g = −1`: `d_strike = (K−S)`, `d_pivot = (P−S)`. The exporter TARF (`Put`, `P=K`) is the validated reference in `tarf.rs`'s own suite.

---

## 4. Variance reduction (the `_cv` entry point)

The default `pivot_tra_price` reports the plain antithetic-pair mean (so the to-bits TARF gate holds). A second entry point delivers the variance-reduced estimate used in the convergence/Greeks tests:

```rust
#[must_use]
pub fn pivot_tra_price_cv(i: &VanillaInputs, spec: PivotTra, cfg: PivotTraMcConfig)
    -> PivotTraResult
```

**Two techniques, both standard and both already in this crate:**

1. **Antithetic variates** — `±Z` per pair (already in the production path; `tarf.rs`/`accumulator.rs` use exactly this).

2. **Forward-strip control variate** (modelled on `mc.rs:price_asian`'s geometric-Asian control). The control is the **same strip with the target disabled and no kink** — a plain forward strip whose expectation is a *closed form*:

   ```
   X = Σ_{k=1}^{n}  g·(S_k − K) · e^{−r_dom·t_k}              (per path)
   E[X] = Σ_{k=1}^{n} g·(F_k − K)·e^{−r_dom·t_k}
        = Σ_{k=1}^{n} g·(S_0·e^{−r_for·t_k} − K·e^{−r_dom·t_k})   (each leg a forward)
   ```

   using `E[S_k] = F_k = S_0·e^{b·t_k}` ⇒ `F_k·e^{−r_dom·t_k} = S_0·e^{−r_for·t_k}`. Highly correlated with the pivot-TRA payoff (shares the path and the dominant linear drift), cheap (closed-form mean, no extra paths), and provably correct (each leg is a forward). Implement the two-pass regression exactly as `price_asian`: pass 1 collects paired `(pivot_pv, X)` antithetic-mean samples and online covariance; estimate `β = cov(pivot,X)/var(X)` (guard `β=0` when `var(X)=0`); pass 2 forms `Y = pivot_pv − β·(X − E[X])` and reports `mean(Y) ± std_error(Y)`. Antithetic and control compose: build the antithetic-pair mean of *both* `pivot_pv` and `X` before the regression (one sample per pair).

   *Why the forward strip, not the geometric-Asian control?* The pivot TRA is not an average-rate product; its dominant variance is the linear forward-strip exposure, so the forward strip is the maximally-correlated cheap control. In redemption-dominated regimes correlation drops and the regression `β→0` degrades gracefully to antithetic-only (same guard as `price_asian`).

Reported `std_error` in `pivot_tra_price_cv` is the control-corrected one; in `pivot_tra_price` it is the plain antithetic-pair std-error.

---

## 5. The independent oracle + gate tests

All gates live in **`crates/celnet-exotics/tests/pivot_oracle.rs`** (a NEW integration test → compiled as a *separate crate* against the public API, so the oracle cannot share private helpers with `pivot.rs`). The oracle is **code-disjoint**: independent RNG, independent path construction, independent payoff accumulation — it *can* disagree.

### 5.0 The code-disjoint Monte-Carlo oracle

**Independence checklist (every axis MUST differ from `pivot.rs`):**

| Axis | `pivot.rs` (production) | `tests/pivot_oracle.rs` (oracle) |
|---|---|---|
| RNG | `CounterRng` (Philox 4×32) | inline 30-line SplitMix64 in the test (no `celnet-exotics` RNG) — or an existing `rand` dev-dep `StdRng` (ChaCha) |
| Normal transform | `normal::inverse_cdf` (Acklam quantile) | Box-Muller `√(−2ln u)·cos(2πu')` written inline in the test (NOT `gaussian_pair_from_uniforms`) |
| Path build | running `ln_s`, antithetic `±z` | full `Vec<f64>` of `S_k` built forward, **plain MC, no antithetic**, larger `n_paths` |
| Payoff/redeem | `walk_path` branch-on-`d_pivot` | re-derive `c_k` from the §3.2 closed form `𝟙[..]·g·(S−K) − 𝟙[..]·L·g·(S−K)`, separate accrual loop |
| Estimator | Welford + control variate | naive sum/sumsq mean + `√(var/N)` |

Prefer the **inline SplitMix64 + Box-Muller** so the oracle touches no `Cargo.toml` and is maximally self-contained and obviously independent. (`rand` is MIT/Apache — OSS-OK per guardrail #7 — if already a dev-dep.)

Oracle signature: `fn oracle_price(i: &VanillaInputs, spec: PivotTra, n_paths: usize, seed: u64) -> (f64 /*bank PV*/, f64 /*std_err*/, f64 /*E[redeem idx]*/, f64 /*E[overshoot]*/)`.

### 5.1 GATE — degenerate→TARF limit, exact (to_bits / 1e-12) — the headline

```rust
#[test]
fn pivot_collapses_to_tarf_when_pivot_equals_strike() {
    let i = VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01);
    let cfg_t = TarfMcConfig    { pairs: 200_000, seed: 0x7A4F };
    let cfg_p = PivotTraMcConfig{ pairs: 200_000, seed: 0x7A4F }; // SAME seed
    for style in [RedemptionStyle::FullGain, RedemptionStyle::CappedGain] {
        let tarf = Tarf { strike:1.32, fixings:12, target:0.06, leverage:2.0,
                          favourable_side: OptionType::Put, notional:1.0,
                          redemption: style };
        let piv  = PivotTra::as_tarf_slice(1.32, 12, 0.06, 2.0,
                          OptionType::Put, 1.0, style);   // pivot == strike == 1.32
        let t = tarf_price(&i, tarf, cfg_t);
        let p = pivot_tra_price(&i, piv, cfg_p);
        assert_eq!(p.price.to_bits(), t.price.to_bits(),
            "pivot(P=K) must equal TARF bit-for-bit: {} vs {}", p.price, t.price);
        assert_eq!(p.std_error.to_bits(), t.std_error.to_bits());
        assert_eq!(p.expected_redemption_fixing.to_bits(),
                   t.expected_redemption_fixing.to_bits());
        assert_eq!(p.expected_overshoot.to_bits(), t.expected_overshoot.to_bits());
    }
}
```

This is the **exact (to_bits)** gate the lane requires. Fallback if a benign reassociation breaks exactness: `assert!((p.price - t.price).abs() < 1e-12)` — but §2.3/§2.4 are written so exact equality holds; attempt `to_bits` first and only relax with a recorded justification.

### 5.2 GATE — independent oracle agreement (can-disagree)

```rust
#[test]
fn pivot_matches_independent_oracle() {
    let i = VanillaInputs::new(1.30, 1.30, 0.12, 1.0, 0.03, 0.01);
    let spec = PivotTra { strike:1.28, pivot:1.33, fixings:12, target:0.08,   // dead band P>K
                          leverage:2.0, favourable_side: OptionType::Call,
                          notional:1.0, redemption: RedemptionStyle::FullGain };
    let prod = pivot_tra_price_cv(&i, spec, PivotTraMcConfig{ pairs:300_000, seed:0xC0FFEE });
    let (op, ose, ofix, oov) = oracle_price(&i, spec, 1_500_000, 0xA11CE);
    let tol = 4.0*(prod.std_error + ose) + 1e-9;       // oracle CAN disagree; it doesn't
    assert!((prod.price - op).abs() < tol);
    assert!((prod.expected_redemption_fixing - ofix).abs() < 0.05);
    assert!((prod.expected_overshoot - oov).abs() < 4.0*(prod.std_error+ose)+1e-3);
}
```

Parametrize over **both** `RedemptionStyle`s × **both** `favourable_side`s × {dead-band `P>K`, overlap `P<K`} — so every branch of the two-level payoff is exercised against the disjoint oracle.

### 5.3 GATE — Greeks finite-difference check (cross-checked against the oracle's OWN CRN-FD, never self-oracle)

```rust
#[test]
fn pivot_greeks_finite_difference() {
    let i = VanillaInputs::new(1.30, 1.30, 0.12, 1.0, 0.03, 0.01);
    let spec = /* dead-band call-favourable, leverage 2.0 */;
    let cfg = PivotTraMcConfig{ pairs:400_000, seed:0xDDEE };   // common random numbers
    let h = 1e-3 * i.spot;
    let up = pivot_tra_price_cv(&VanillaInputs{spot:i.spot+h, ..i}, spec, cfg);
    let dn = pivot_tra_price_cv(&VanillaInputs{spot:i.spot-h, ..i}, spec, cfg);
    let delta_bank = (up.price - dn.price) / (2.0*h);
    let (op_u,..)=oracle_price(&VanillaInputs{spot:i.spot+h,..i}, spec, 1_000_000, 0xBEE);
    let (op_d,..)=oracle_price(&VanillaInputs{spot:i.spot-h,..i}, spec, 1_000_000, 0xBEE);
    let delta_oracle = (op_u - op_d)/(2.0*h);
    assert!((delta_bank - delta_oracle).abs() < 0.05*delta_bank.abs().max(1e-3) + combined_band);
    assert!(delta_bank < 0.0, "bank short the favourable leg ⇒ negative spot delta");
}
```

Also a **vega FD** (bump `vol` by `1e-3`): the FullGain overshoot is long convexity; cross-check the bank-vega magnitude against the oracle vega within the combined band (signs asserted against the oracle, not in isolation — avoids a circular self-oracle, guardrail #5).

### 5.4 GATE — pivot is a real lever (not a no-op)

```rust
#[test]
fn pivot_away_from_strike_changes_price() {
    let p_eq = pivot_tra_price_cv(&i, PivotTra{ pivot:1.28, strike:1.28, ..base_spec }, cfg);
    let p_db = pivot_tra_price_cv(&i, PivotTra{ pivot:1.33, strike:1.28, ..base_spec }, cfg);
    assert!((p_eq.price - p_db.price).abs() > 4.0*(p_eq.std_error + p_db.std_error),
        "moving the pivot off the strike must move the price beyond MC noise");
}
```

### 5.5 GATE — monotonicity + bounds (retargeted copies of `tarf.rs`'s suite)

In-module unit tests in `pivot.rs` (`#[cfg(test)]`):
- `mc_is_reproducible` — same seed ⇒ `to_bits`-identical `price` and `std_error`.
- `tighter_target_redeems_earlier` — smaller `target` ⇒ smaller `expected_redemption_fixing`.
- `higher_leverage_raises_bank_value` — larger `leverage` ⇒ larger bank PV.
- `gap_risk_full_gain_costs_more_than_capped` — FullGain bank PV `<` CappedGain; FullGain overshoot `>1e-4`, CappedGain overshoot `<1e-12`.
- `unreachable_target_never_redeems` — `target=1e6` ⇒ `expected_redemption_fixing == fixings`, overshoot `0`; and at `target=1e6, P=K` the price matches the `Tarf` geared-strip limit to MC tolerance.

### 5.6 Convergence + variance-reduction sanity

Loop `pairs ∈ {25k, 100k, 400k}`: assert `pivot_tra_price_cv` std-error shrinks ~`1/√N`, and at matched `pairs` the control-variate std-error is strictly below the plain antithetic std-error (proves the control actually reduces variance). Use a loose-target scenario where the control is genuinely effective.

---

## 6. `lib.rs` wiring (additive — the only edit to an existing file)

Add (keep the `pub mod` block sorted — after `particle`, before `payoff`):
```rust
pub mod pivot;
```
Re-export (after the `pde::` re-export, mirroring the `tarf` line):
```rust
pub use pivot::{PivotTra, PivotTraMcConfig, PivotTraResult, pivot_tra_price, pivot_tra_price_cv};
```
Add one crate-doc bullet to section 5 ("Structured & path-dependent breadth"), after the `accumulator` bullet:
```
//!    * [`pivot`] — the **pivot Target-Redemption Accumulator**: a two-level
//!      (pivot/strike) piecewise-linear fixing strip with cumulative-target
//!      redemption and gap-risk handling, collapsing to [`tarf::Tarf`] exactly when
//!      `pivot == strike` (gated to 1e-12), priced by Monte-Carlo with a forward-
//!      strip control variate and cross-validated against a code-disjoint oracle.
```
No edits to `tarf.rs`, `accumulator.rs`, `mc.rs`, `rng.rs`, `normal.rs`, or any other crate. `RedemptionStyle` is *re-used* from `tarf` (imported), not redefined — no duplication (guardrail #10).

---

## 7. Dependency / build order (mechanical)

1. **`pivot.rs` types** — `PivotTra` (+ `validate`, `as_tarf_slice`), `PivotTraMcConfig`, `PivotTraResult`, private `Welford`. `just check-crate celnet-exotics`.
2. **`walk_path` + `pivot_tra_price`** — production path, mirroring `tarf_price` arithmetic order exactly (§2.3/§2.4). Add the in-module unit tests (§5.5) retargeted from `tarf.rs`. Compile + test.
3. **`lib.rs` wiring** (§6). `just check-crate celnet-exotics` — confirms the re-exports resolve.
4. **`pivot_tra_price_cv`** — forward-strip control variate (two-pass regression à la `price_asian`). In-module convergence test (§5.6).
5. **`tests/pivot_oracle.rs`** — the code-disjoint oracle + gates §5.1–§5.4, in order: write `oracle_price` first, then §5.1 (TARF to_bits), §5.2 (oracle agreement), §5.3 (Greeks FD), §5.4 (pivot-is-a-lever).
6. **Full gate** — `just check` (fmt, clippy -D warnings, nextest, deny). Confirm clippy runs on the **test target** too (the §5 oracle file), per the recorded "clippy the parity test target" lesson in MEMORY.

**Upstream deps:** none new. `pivot.rs` consumes only `celnet-core::math`, `celnet-types::{VanillaInputs, OptionType}`, and `crate::{normal, rng, tarf::RedemptionStyle}` — all already in the crate's dependency closure. The oracle test needs no new runtime dep (inline SplitMix64 + Box-Muller preferred).

---

## 8. Guardrail compliance ledger

- **No mocks/placeholders/`todo!()`** — every fn fully specified; control variate and oracle are complete algorithms.
- **Vendor/person-neutral naming** — `PivotTra`, `pivot`, `pivot_tra_price`; provenance only in doc comments.
- **ADR-0008 carry seam** — consumes `VanillaInputs` exactly as `tarf.rs`/`accumulator.rs`; **no `match` on `Underlying`/`Carry` in the hot path** — the per-fixing drift `(r_dom − r_for − ½σ²)·dt` is computed once outside the path loop from the plain rate fields. (A future cross-asset carry generalization lands the same way `tarf.rs` would — out of scope; this lane mirrors the established FX leaf.)
- **One unversioned contract** — additive only; no `schema_version`, no compat shims.
- **OSS-only** — no new deps, or only `rand` (MIT/Apache); inline RNG preferred.
- **Independent oracle, never circular** — the oracle is a separately-compiled integration test with its own RNG/normal/path/payoff/estimator; Greeks/sign assertions check against the oracle's own CRN-FD, not against `pivot.rs`'s output. The TARF-limit gate regresses against `tarf.rs` (an already independently-validated product) — a legitimate cross-product regression, not a self-oracle.
- **Determinism** — all transcendentals via `celnet_core::math`; production path is `to_bits`-reproducible; the only float `==` is `to_bits` equality in the reproducibility/degeneracy gates.

---

## 9. Open risks for the implementer

1. **to_bits TARF identity** — §5.1 is specified *exact*. The reviewer must verify the geared-branch arithmetic order in `walk_path` matches `tarf.rs` to the bit (§2.4 subtlety) by adding the `to_bits` assertion. If a benign reassociation breaks exactness, relax to `<1e-12` with a recorded note; the rest is unaffected.
2. **Dead-band accrual semantics** — the decision that a *positive geared* cash flow (`P>K` region) accrues to the target (keyed on `sign(c)`) is the standard reading; if the desk convention is "only the un-geared favourable leg accrues," flip the accrual key to `d_pivot ≥ 0 && c > 0`, and mirror it in the oracle. Confirm against `docs/ANALYTICS-SPEC.md` before freezing.
3. **Control-variate correlation in redemption-dominated regimes** — for very tight targets the forward-strip control's correlation drops; the `β→0` guard makes this safe (degrades to antithetic-only), but the §5.6 "CV beats plain" assertion should use a loose-ish target where the control is genuinely effective.
4. **Read-only design** — no build/clippy/test run (compute-courtesy for session-B's proto window). Compile-ability and exact to_bits behaviour are asserted by construction from reading `tarf.rs`/`accumulator.rs`/`mc.rs`, not verified empirically — the §5.1 to_bits gate is the implementer's empirical check.
