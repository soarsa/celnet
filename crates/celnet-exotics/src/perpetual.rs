//! Perpetual (no-expiry) American vanilla pricing on the agnostic carry seam.
//!
//! A perpetual option has no terminal date: the holder may exercise at any
//! time, and exercise is the only way the contract ends — the product is
//! American by construction. Under the lognormal cost-of-carry dynamics
//! (discount rate `r = carry.discount_rate()`, net carry `b = carry.carry_rate()`,
//! volatility `σ`) the value is **time-homogeneous** — it depends only on the
//! current spot — and solves the stationary pricing ODE
//!
//! ```text
//!   ½σ²·S²·V″(S) + b·S·V′(S) − r·V(S) = 0 ,
//! ```
//!
//! whose power solutions `V = S^y` have exponents at the roots of the
//! characteristic quadratic
//!
//! ```text
//!   ψ(y) = ½σ²·y·(y−1) + b·y − r = 0 .
//! ```
//!
//! For `r ≥ 0` the roots straddle the origin: `y₂ ≤ 0 ≤ y₁`. Value matching
//! **and** smooth pasting (`V′` continuous) at the free early-exercise boundary
//! pin both the boundary and the closed form:
//!
//! * **Call** (uses `y₁ > 1`, which exists iff `b < r`): exercise boundary
//!   `S* = K·y₁/(y₁−1)`, and on the continuation region `S < S*`
//!
//!   ```text
//!   V = (S* − K) · (S/S*)^{y₁}      (= K/(y₁−1) · ((y₁−1)/y₁ · S/K)^{y₁}) ,
//!   ```
//!
//!   with `V = S − K` (immediate exercise) for `S ≥ S*`. At `b = r` **exactly**
//!   the quadratic has `ψ(1) = b − r = 0`, so `y₁ = 1`: holding the asset costs
//!   exactly nothing relative to discounting, early exercise is never optimal,
//!   and the value is the spot itself — `V = S`, the exact `y₁ → 1⁺` limit of
//!   the closed form and the `T → ∞` limit of the same-terms European call
//!   (the one degenerate where the perpetual call equals its underlying). This
//!   arm is handled exactly, never as a near-singular evaluation of the power
//!   form. The same exact arm catches the **sub-ulp window**
//!   `b ∈ (r − O(ulp·r), r)` where the finite-precision root collapses to
//!   exactly `y₁ = 1` even though `b < r` strictly — the rounding-collapsed
//!   root takes the limit arm, never a `1/0` boundary evaluation.
//!
//!   For `b > r` **strictly** the perpetual call has **no finite value** and
//!   the pricer refuses with the typed [`PerpetualError`]: stopping at any
//!   level `L > K` is worth `(L − K)·(S/L)^{y₁}` with `y₁ < 1` (since
//!   `ψ(1) = b − r > 0` puts the larger root strictly below 1), which grows
//!   without bound as `L → ∞`; equivalently, `e^{−rt}·S_t` is a **strict
//!   submartingale** under `b > r`, so the value of the never-ending right to
//!   buy the asset diverges. There is no `V = S` pin here — pinning any finite
//!   number creates an internal arbitrage against the finite-maturity European
//!   (whose value already exceeds `S` at long maturities under `b > r`).
//! * **Put** (uses `y₂ < 0`, which exists iff `r > 0`, or `r = 0` with
//!   `b > ½σ²`): exercise boundary `S** = K·y₂/(y₂−1) ∈ (0, K)`, and on the
//!   continuation region `S > S**`
//!
//!   ```text
//!   V = (K − S**) · (S/S**)^{y₂} ,
//!   ```
//!
//!   with `V = K − S` for `S ≤ S**`. When `r = 0` **and** `b ≤ ½σ²` the small
//!   root collapses to `y₂ = 0` and the boundary to `S** = 0`: with the
//!   log-spot drifting down and no discounting, the spot reaches any level
//!   below with probability one, so the value is the (unattained) supremum
//!   `V = K` — handled exactly, never as a `0^0` power evaluation.
//!
//! # Greeks (all analytic — no finite differences)
//!
//! On the continuation region the closed form depends on `(σ, r, b)` **only
//! through the root** `y`, so every sensitivity is exact chain rule:
//!
//! * `delta = y·V/S`, `gamma = y·(y−1)·V/S²` (direct differentiation of the
//!   power form);
//! * `∂V/∂y = V·ln(S/S_b)` where `S_b` is the exercise boundary (the boundary's
//!   own `y`-dependence cancels exactly — smooth pasting);
//! * implicit-function sensitivities of the root through `ψ(y) = 0` with
//!   `ψ′(y) = σ²·y + b − ½σ²` (which equals `±√disc` at `y₁`/`y₂`):
//!   `dy/dσ = −σ·y·(y−1)/ψ′(y)`, `dy/dr = 1/ψ′(y)`, `dy/db = −y/ψ′(y)`;
//! * `vega = ∂V/∂y · dy/dσ`, `discount_rho = ∂V/∂y · dy/dr`,
//!   `carry_rho = ∂V/∂y · dy/db`.
//!
//! Because the value is time-homogeneous, theta is **identically zero** and
//! there is no tenor to define a forward delta or any other time-decay Greek —
//! so this module returns its own [`PerpetualGreeks`] strip rather than a
//! [`celnet_core::CarryGreeks`] padded with vacuous fields. Rate sensitivities
//! are carry-tagged ([`RateSensitivities`]): an FX [`Carry::FxRates`] input
//! reports `(rho_dom, rho_for)` by the exact chain rule through
//! `r = r_dom`, `b = r_dom − r_for` (`rho_dom = ∂V/∂r + ∂V/∂b`,
//! `rho_for = −∂V/∂b`); a [`Carry::CostOfCarry`] input reports
//! `(discount_rho, carry_rho)` directly.
//!
//! # Domain
//!
//! `spot > 0`, `strike > 0`, `vol > 0`, and `r ≥ 0`: a perpetual claim under a
//! negative numeraire rate has no finite value (the discounted strike grows
//! without bound), so the closed form is not defined there. A **call** with
//! `b > r` strictly is refused with [`PerpetualError::CallCarryExceedsDiscount`]
//! (no finite value — see the call bullet above); **puts are unaffected** by
//! that refusal: the put prices on the small root `y₂ ≤ 0`, which exists for
//! every `b` whenever `r ≥ 0` (the product of roots is `−r/(½σ²) ≤ 0`), and the
//! put payoff is bounded by `K`, so its value is finite for all carries.
//!
//! # Method provenance (doc comments only)
//!
//! The perpetual American free-boundary closed form: McKean (1965); Merton
//! (1973, the perpetual put); Samuelson (1965); the cost-of-carry
//! generalization as presented in Haug (2007, *The Complete Guide to Option
//! Pricing Formulas*, 2nd ed.). All identifiers here are purpose-named and
//! vendor/research-neutral; provenance lives only in documentation.

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::{Carry, OptionType, RateSensitivities};

/// Typed domain refusal of the perpetual closed form — the contract has **no
/// finite value** on the refused inputs, so no number is ever fabricated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerpetualError {
    /// A perpetual **call** with net carry strictly exceeding the discount
    /// rate (`b > r`) diverges: stopping at any level `L > K` is worth
    /// `(L − K)·(S/L)^{y₁}` with `y₁ < 1` (`ψ(1) = b − r > 0` puts the larger
    /// characteristic root strictly below 1), unbounded as `L → ∞`
    /// (`e^{−rt}·S_t` is a strict submartingale). Only `b = r` exactly admits
    /// the degenerate `V = S`; puts are unaffected (the `y₂` branch).
    CallCarryExceedsDiscount,
}

impl core::fmt::Display for PerpetualError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            PerpetualError::CallCarryExceedsDiscount => {
                "a perpetual call with carry exceeding the discount rate has no finite value"
            }
        };
        f.write_str(s)
    }
}

impl std::error::Error for PerpetualError {}

/// The market state a perpetual American vanilla prices against.
///
/// A perpetual has **no expiry**, so there is no `t` field — the value is
/// time-homogeneous. Forward growth and discounting enter only through the
/// agnostic [`Carry`] seam (`r = discount_rate()`, `b = carry_rate()`); the
/// pricer never matches on the carry variant in pricing math.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerpetualInputs {
    /// Spot price of the underlying (quote per 1 unit of base, for FX).
    pub spot: f64,
    /// Strike `K` (quote per 1 unit of base, for FX).
    pub strike: f64,
    /// Annualized volatility `σ` (absolute, e.g. `0.10` = 10 vol).
    pub vol: f64,
    /// The cost-of-carry model behind the (perpetual) forward growth and
    /// discounting.
    pub carry: Carry,
}

impl PerpetualInputs {
    /// Construct a perpetual pricing input.
    #[must_use]
    pub const fn new(spot: f64, strike: f64, vol: f64, carry: Carry) -> Self {
        Self {
            spot,
            strike,
            vol,
            carry,
        }
    }
}

/// The analytic Greek strip of a perpetual American vanilla.
///
/// A perpetual is time-homogeneous: theta/charm/color are **identically zero**
/// and there is no settlement tenor to define a forward delta, so those fields
/// do not exist here (returning them zeroed inside a generic strip would
/// misrepresent structurally-absent sensitivities as computed ones). All
/// sensitivities are raw, matching the conventions of
/// [`celnet_core::CarryGreeks`]: vega per `1.0` of absolute vol, rhos per `1.0`
/// of continuously-compounded rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerpetualGreeks {
    /// Present value (premium) in the numeraire currency, per 1 unit of base.
    pub price: f64,
    /// Spot delta `∂V/∂S`.
    pub delta: f64,
    /// Gamma `∂²V/∂S²`.
    pub gamma: f64,
    /// Vega `∂V/∂σ` (per `1.0` absolute vol).
    pub vega: f64,
    /// Carry-tagged rate sensitivities (FX two rhos, or discount/carry rho
    /// pair), all exact `y`-root chain rule — see the module docs.
    pub rates: RateSensitivities,
}

/// `x^y` for `x > 0` through the deterministic core math seam, which exposes
/// `exp`/`ln` but no direct power: `x^y = e^{y·ln x}`.
#[inline]
fn powy(x: f64, y: f64) -> f64 {
    exp(y * ln(x))
}

/// The two roots of the characteristic quadratic
/// `ψ(y) = ½σ²·y² + (b − ½σ²)·y − r = 0`, with `ψ′` evaluated at each.
struct CharacteristicRoots {
    /// The larger root `y₁` (`≥ 0` for `r ≥ 0`; `> 1` iff `b < r`).
    y_high: f64,
    /// The smaller root `y₂` (`≤ 0` for `r ≥ 0`).
    y_low: f64,
    /// `√((b − ½σ²)² + 2σ²r)` — equals `ψ′(y₁)` exactly (`ψ′(y₂) = −√disc`).
    sqrt_disc: f64,
}

fn characteristic_roots(vol: f64, r: f64, b: f64) -> CharacteristicRoots {
    let quad = 0.5 * vol * vol; // quadratic coefficient ½σ²
    let lin = b - quad; // linear coefficient b − ½σ²
    let con = -r; // constant coefficient −r
    // disc = (b − ½σ²)² + 2σ²r ≥ 0 whenever r ≥ 0 (the documented domain), so
    // both roots are real.
    let disc = lin * lin - 4.0 * quad * con;
    let sqrt_disc = sqrt(disc);
    // Cancellation-free root pairing: fold the same-signed pair (lin, ±√disc)
    // into `half`, then recover one root from `half/quad` and the other from
    // the product of roots `con/quad = (half/quad)·(con/half)`. Neither root is
    // ever formed by subtracting nearly-equal magnitudes, which matters in the
    // σ→0 regime where `|b|/σ² → ∞`.
    let half = if lin >= 0.0 {
        -0.5 * (lin + sqrt_disc)
    } else {
        -0.5 * (lin - sqrt_disc)
    };
    // `half == 0` requires lin == 0 AND disc == 0, i.e. r == 0 with b == ½σ²:
    // a double root at the origin. (Exact-zero structural comparison.)
    let (first, second) = if half == 0.0 {
        (0.0, 0.0)
    } else {
        (half / quad, con / half)
    };
    let (y_high, y_low) = if first >= second {
        (first, second)
    } else {
        (second, first)
    };
    CharacteristicRoots {
        y_high,
        y_low,
        sqrt_disc,
    }
}

/// Where on the exercise diagram the contract sits, with everything the price
/// and the Greek strip need from the continuation closed form.
enum Valuation {
    /// Call with `b = r` exactly (or the rounding-collapsed `y₁ = 1.0` sub-ulp
    /// window inside `b < r`): never exercised, `V = S` exactly (the `y₁ → 1⁺`
    /// limit). Unit delta, every other sensitivity zero on the open region.
    NeverExercisedCall,
    /// Put with `r = 0` and `b ≤ ½σ²` (`y₂ = 0`): the boundary collapses to 0
    /// and the value is the unattained supremum `V = K` exactly. Flat in every
    /// input on the open region.
    NeverExercisedPut,
    /// Spot beyond the free boundary: stopped, `V` = intrinsic exactly.
    Exercised,
    /// Continuation region: the power closed form.
    Continuation {
        /// The characteristic root the side prices on (`y₁` call / `y₂` put).
        y: f64,
        /// The free early-exercise boundary (`S*` call / `S**` put).
        boundary: f64,
        /// The present value `(|S_b − K|)·(S/S_b)^y`.
        value: f64,
        /// `ψ′(y)` at the side's root (`+√disc` call / `−√disc` put).
        dpsi_dy: f64,
    },
}

fn valuation(opt: OptionType, i: &PerpetualInputs) -> Result<Valuation, PerpetualError> {
    let r = i.carry.discount_rate();
    let b = i.carry.carry_rate();
    Ok(match opt {
        OptionType::Call => {
            // b > r STRICTLY: the perpetual call diverges (ψ(1) = b − r > 0
            // puts y₁ < 1, so stopping at L gives (L−K)(S/L)^{y₁} → ∞;
            // e^{−rt}·S_t is a strict submartingale) — refused, never pinned.
            if b > r {
                return Err(PerpetualError::CallCarryExceedsDiscount);
            }
            // b == r EXACTLY (structural comparison, not a tolerance): the
            // degenerate where ψ(1) = 0 makes y₁ = 1 — early exercise is never
            // optimal and V = S exactly, the y₁ → 1⁺ limit of the closed form
            // and the T → ∞ limit of the same-terms European call.
            if b == r {
                return Ok(Valuation::NeverExercisedCall);
            }
            let roots = characteristic_roots(i.vol, r, b);
            let y = roots.y_high; // > 1, since ψ(1) = b − r < 0 here
            // ψ(1) = b − r < 0 puts the TRUE root strictly above 1, but in the
            // sub-ulp window b ∈ (r − O(ulp·r), r) the finite-precision root
            // collapses to exactly 1.0 — the same structural rounding collapse
            // the put handles at y₂ == 0. The exact arm is the documented
            // y₁ → 1⁺ limit (V = S, no finite boundary); evaluating the power
            // form there would form the 1/0 boundary and a NaN value.
            // (Exact-zero structural comparison, not a tolerance.)
            if y == 1.0 {
                return Ok(Valuation::NeverExercisedCall);
            }
            let boundary = i.strike * y / (y - 1.0);
            if i.spot >= boundary {
                return Ok(Valuation::Exercised);
            }
            Valuation::Continuation {
                y,
                boundary,
                value: (boundary - i.strike) * powy(i.spot / boundary, y),
                dpsi_dy: roots.sqrt_disc,
            }
        }
        OptionType::Put => {
            let roots = characteristic_roots(i.vol, r, b);
            let y = roots.y_low;
            // y₂ is constructed as the exact product-of-roots quotient −r/half,
            // so it is ±0 precisely when r == 0 (exact-zero structural
            // comparison, not a tolerance).
            if y == 0.0 {
                return Ok(Valuation::NeverExercisedPut);
            }
            let boundary = i.strike * y / (y - 1.0); // ∈ (0, K) for y < 0
            if i.spot <= boundary {
                return Ok(Valuation::Exercised);
            }
            Valuation::Continuation {
                y,
                boundary,
                value: (i.strike - boundary) * powy(i.spot / boundary, y),
                dpsi_dy: -roots.sqrt_disc,
            }
        }
    })
}

/// Present value of a perpetual American vanilla (premium in the numeraire
/// currency, per 1 unit of base). See the module docs for the closed form,
/// the degenerate arms and the domain.
///
/// # Errors
///
/// [`PerpetualError::CallCarryExceedsDiscount`] for a call with `b > r`
/// strictly — the value diverges, so no number exists to return (puts are
/// unaffected; `b == r` exactly is the finite `V = S` degenerate).
pub fn perpetual_price(opt: OptionType, i: &PerpetualInputs) -> Result<f64, PerpetualError> {
    Ok(match valuation(opt, i)? {
        Valuation::NeverExercisedCall => i.spot,
        Valuation::NeverExercisedPut => i.strike,
        Valuation::Exercised => opt.sign() * (i.spot - i.strike),
        Valuation::Continuation { value, .. } => value,
    })
}

/// The free early-exercise boundary of a perpetual American vanilla:
/// `S* = K·y₁/(y₁−1)` (call) or `S** = K·y₂/(y₂−1)` (put), independent of the
/// current spot.
///
/// Degenerate arms return the exact limit: `+∞` for a call with `b == r`
/// exactly (never exercised — no finite boundary) and `0` for a put with
/// `r = 0` and `b ≤ ½σ²` (the boundary collapses to the origin).
///
/// # Errors
///
/// [`PerpetualError::CallCarryExceedsDiscount`] for a call with `b > r`
/// strictly — the contract has no finite value and therefore no boundary.
pub fn perpetual_exercise_boundary(
    opt: OptionType,
    i: &PerpetualInputs,
) -> Result<f64, PerpetualError> {
    let r = i.carry.discount_rate();
    let b = i.carry.carry_rate();
    Ok(match opt {
        OptionType::Call => {
            if b > r {
                return Err(PerpetualError::CallCarryExceedsDiscount);
            }
            if b == r {
                return Ok(f64::INFINITY);
            }
            let y = characteristic_roots(i.vol, r, b).y_high;
            // Rounding-collapsed y₁ == 1.0 (the sub-ulp b → r⁻ window): no
            // finite boundary — the exact y₁ → 1⁺ limit, mirroring `valuation`.
            if y == 1.0 {
                return Ok(f64::INFINITY);
            }
            i.strike * y / (y - 1.0)
        }
        OptionType::Put => {
            let y = characteristic_roots(i.vol, r, b).y_low;
            // Exact ±0 when r == 0 (see `valuation`); the boundary limit is 0.
            if y == 0.0 {
                return Ok(0.0);
            }
            i.strike * y / (y - 1.0)
        }
    })
}

/// Price and the analytic Greek strip of a perpetual American vanilla in one
/// pass — every sensitivity is the exact closed form / `y`-root chain rule of
/// the module docs (no finite differences anywhere).
///
/// On the stopped (immediate-exercise) region the strip is the intrinsic one:
/// `delta = ±1`, everything else zero. On the never-exercised degenerate arms
/// the value is flat in every input over the open region, so the strip is
/// `delta = 1` (call, `V = S` on `b == r`) or all-zero (put, `V = K`); on
/// those degenerate sets these are the exact derivatives along the arm.
///
/// # Errors
///
/// [`PerpetualError::CallCarryExceedsDiscount`] for a call with `b > r`
/// strictly — there is no finite value, hence no strip (puts are unaffected).
pub fn perpetual_greeks(
    opt: OptionType,
    i: &PerpetualInputs,
) -> Result<PerpetualGreeks, PerpetualError> {
    let (price, delta, gamma, vega, discount_rho, carry_rho) = match valuation(opt, i)? {
        Valuation::NeverExercisedCall => (i.spot, 1.0, 0.0, 0.0, 0.0, 0.0),
        Valuation::NeverExercisedPut => (i.strike, 0.0, 0.0, 0.0, 0.0, 0.0),
        Valuation::Exercised => {
            let sign = opt.sign();
            (sign * (i.spot - i.strike), sign, 0.0, 0.0, 0.0, 0.0)
        }
        Valuation::Continuation {
            y,
            boundary,
            value,
            dpsi_dy,
        } => {
            let delta = y * value / i.spot;
            let gamma = y * (y - 1.0) * value / (i.spot * i.spot);
            // ∂V/∂y = V·ln(S/S_b): the boundary's own y-dependence cancels
            // exactly (smooth pasting), leaving the pure log-moneyness factor.
            let dv_dy = value * ln(i.spot / boundary);
            // Implicit-function derivatives of ψ(y; σ, r, b) = 0.
            let dy_dsigma = -(i.vol * y * (y - 1.0)) / dpsi_dy;
            let dy_dr = 1.0 / dpsi_dy;
            let dy_db = -y / dpsi_dy;
            (
                value,
                delta,
                gamma,
                dv_dy * dy_dsigma,
                dv_dy * dy_dr,
                dv_dy * dy_db,
            )
        }
    };
    // Carry-tagged rhos (ADR-0008 idiom): the FX arm reports the exact chain
    // rule through r = r_dom, b = r_dom − r_for. This match selects each arm's
    // natural-coordinate rate-Greeks (per-request reporting setup, never pricing
    // math); the matching arm *type* is wrapped by the single-source mapper
    // `Carry::rate_sensitivities` (pure type selection, no arithmetic) —
    // byte-identical to the former inline arm constructors.
    let (rate_greek_a, rate_greek_b) = match i.carry {
        // FX natural coords are the two flat rhos: rho_dom = discount_rho +
        // carry_rho (in that order), rho_for = −carry_rho.
        Carry::FxRates { .. } => (discount_rho + carry_rho, -carry_rho),
        Carry::CostOfCarry { .. } => (discount_rho, carry_rho),
    };
    let rates = i.carry.rate_sensitivities(rate_greek_a, rate_greek_b);
    Ok(PerpetualGreeks {
        price,
        delta,
        gamma,
        vega,
        rates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn cost_of_carry(r: f64, b: f64) -> Carry {
        Carry::CostOfCarry { r, b }
    }

    fn inp(spot: f64, strike: f64, vol: f64, r: f64, b: f64) -> PerpetualInputs {
        PerpetualInputs::new(spot, strike, vol, cost_of_carry(r, b))
    }

    /// INDEPENDENT re-derivation of the characteristic roots by expanding-
    /// bracket interval **bisection** of ψ(y) = ½σ²·y·(y−1) + b·y − r, written
    /// in the y·(y−1) *product* form — a different float route from the
    /// engine's standard-form quadratic + discriminant + cancellation-free
    /// pairing. The value is then completed with `std`'s `f64::powf`, not the
    /// engine's `exp(y·ln x)` seam route. Agreement kills a shared-route bug.
    #[test]
    fn closed_form_matches_independent_bisection_rederivation() {
        let psi = |y: f64, vol: f64, r: f64, b: f64| 0.5 * vol * vol * y * (y - 1.0) + b * y - r;
        let bisect = |mut lo: f64, mut hi: f64, vol: f64, r: f64, b: f64| {
            // ψ(lo) < 0 < ψ(hi) required; 200 halvings reach machine precision.
            for _ in 0..200 {
                let mid = 0.5 * (lo + hi);
                if psi(mid, vol, r, b) <= 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            0.5 * (lo + hi)
        };
        for &(s, k, vol, r, b) in &[
            (100.0, 100.0, 0.30, 0.08, 0.04),
            (100.0, 110.0, 0.25, 0.06, 0.02),
            (1.30, 1.25, 0.10, 0.05, 0.01),
            (80.0, 95.0, 0.40, 0.07, -0.03),
            (50.0, 45.0, 0.15, 0.03, 0.0),
            (100.0, 100.0, 1e-3, 0.08, 0.04), // σ→0 regime: the stable pairing matters
        ] {
            let i = inp(s, k, vol, r, b);

            // Call root y₁ > 1: ψ(1) = b − r < 0 on this grid; expand hi.
            let mut hi = 2.0;
            while psi(hi, vol, r, b) <= 0.0 {
                hi *= 2.0;
            }
            let y1 = bisect(1.0, hi, vol, r, b);
            let s_star = k * y1 / (y1 - 1.0);
            let call_ref = if s >= s_star {
                s - k
            } else {
                (s_star - k) * (s / s_star).powf(y1)
            };
            assert_close!(
                perpetual_price(OptionType::Call, &i).unwrap(),
                call_ref,
                1e-10,
                1e-12
            );
            assert_close!(
                perpetual_exercise_boundary(OptionType::Call, &i).unwrap(),
                s_star,
                1e-10,
                1e-12
            );

            // Put root y₂ < 0: ψ(0) = −r < 0 on this grid; expand lo downward
            // until ψ(lo) > 0, then bisect the reversed bracket (negative side
            // keeps ψ > 0, non-negative side keeps ψ ≤ 0).
            let mut lo = -2.0;
            while psi(lo, vol, r, b) <= 0.0 {
                lo *= 2.0;
            }
            let (mut neg_lo, mut neg_hi) = (lo, 0.0); // ψ(neg_lo) > 0 > ψ(neg_hi)
            for _ in 0..200 {
                let mid = 0.5 * (neg_lo + neg_hi);
                if psi(mid, vol, r, b) <= 0.0 {
                    neg_hi = mid;
                } else {
                    neg_lo = mid;
                }
            }
            let y2 = 0.5 * (neg_lo + neg_hi);
            let s_dstar = k * y2 / (y2 - 1.0);
            let put_ref = if s <= s_dstar {
                k - s
            } else {
                (k - s_dstar) * (s / s_dstar).powf(y2)
            };
            assert_close!(
                perpetual_price(OptionType::Put, &i).unwrap(),
                put_ref,
                1e-10,
                1e-12
            );
            assert_close!(
                perpetual_exercise_boundary(OptionType::Put, &i).unwrap(),
                s_dstar,
                1e-10,
                1e-12
            );
        }
    }

    /// Hand-pinned reference values RE-DERIVED OFFLINE from the primary closed
    /// form (CPython 3 / IEEE-754 `math.sqrt` + `math.pow` — a float route
    /// disjoint from this crate's `libm` seam), longhand:
    ///
    /// Call S=100, K=100, σ=0.30, r=0.08, b=0.04:
    ///   ½σ² = 0.045,  b − ½σ² = −0.005,  disc = 0.005² + 2·0.09·0.08
    ///        = 0.000025 + 0.0144 = 0.014425,  √disc = 0.12010412159862777
    ///   y₁ = (0.005 + √disc)/0.09 = 1.3900457943849238
    ///   S* = 100·y₁/(y₁−1)        = 356.38015186830387
    ///   V  = (S*−K)·(S/S*)^{y₁}   = 43.82272192156737
    ///
    /// Put S=100, K=110, σ=0.25, r=0.06, b=0.02:
    ///   ½σ² = 0.03125,  b − ½σ² = −0.01125,
    ///   disc = 0.01125² + 2·0.0625·0.06 = 0.0076265625, √disc = 0.08733018665778777
    ///   y₂ = (0.01125 − √disc)/0.0625 = −1.2172830779766854
    ///   S** = 110·y₂/(y₂−1)           = 60.38973548637859
    ///   V   = (K−S**)·(S/S**)^{y₂}    = 26.849837653852706
    #[test]
    fn pinned_offline_reference_values() {
        let call = inp(100.0, 100.0, 0.30, 0.08, 0.04);
        assert_close!(
            perpetual_price(OptionType::Call, &call).unwrap(),
            43.822_721_921_567_37,
            1e-10,
            1e-9
        );
        assert_close!(
            perpetual_exercise_boundary(OptionType::Call, &call).unwrap(),
            356.380_151_868_303_87,
            1e-12,
            1e-9
        );

        let put = inp(100.0, 110.0, 0.25, 0.06, 0.02);
        assert_close!(
            perpetual_price(OptionType::Put, &put).unwrap(),
            26.849_837_653_852_706,
            1e-10,
            1e-9
        );
        assert_close!(
            perpetual_exercise_boundary(OptionType::Put, &put).unwrap(),
            60.389_735_486_378_59,
            1e-12,
            1e-9
        );
    }

    /// Degenerate law: `b == r` EXACTLY ⇒ the perpetual call is never
    /// exercised early and `V = S` exactly (`ψ(1) = 0`, `y₁ = 1` — the `T → ∞`
    /// European-call limit; handled as an exact arm, no `y₁ → 1⁺` NaN). The
    /// `to_bits` equalities are the documented exact-arm carve-out from the
    /// no-float-`==` rule.
    #[test]
    fn carry_equal_to_discount_call_is_spot_exactly() {
        for &(r, b) in &[(0.05, 0.05), (0.0, 0.0), (0.123, 0.123)] {
            let i = inp(123.45, 100.0, 0.2, r, b);
            let p = perpetual_price(OptionType::Call, &i).unwrap();
            assert_eq!(p.to_bits(), 123.45f64.to_bits());
            assert!(
                perpetual_exercise_boundary(OptionType::Call, &i)
                    .unwrap()
                    .is_infinite()
            );
            let g = perpetual_greeks(OptionType::Call, &i).unwrap();
            assert_eq!(g.price.to_bits(), p.to_bits());
            assert_eq!(g.delta.to_bits(), 1.0f64.to_bits());
            assert_eq!(g.gamma.to_bits(), 0.0f64.to_bits());
            assert_eq!(g.vega.to_bits(), 0.0f64.to_bits());
        }
        // Continuity into the arm: b just below r must stay finite and land
        // next to S (the y₁ → 1⁺ limit), not blow up.
        let near = inp(100.0, 100.0, 0.2, 0.05, 0.05 - 1e-9);
        let p = perpetual_price(OptionType::Call, &near).unwrap();
        assert!(p.is_finite());
        assert_close!(p, 100.0, 1e-4, 1e-4);
    }

    /// Divergence law: a call with `b > r` STRICTLY has NO finite value
    /// (stopping at `L` yields `(L−K)(S/L)^{y₁}` with `y₁ < 1` → ∞;
    /// `e^{−rt}·S_t` is a strict submartingale) — price, boundary and the
    /// Greek strip all refuse with the typed error, never a pinned `V = S`
    /// (adversarial-verify refutation: the old `V = S` pin at `(r, b) =
    /// (0.02, 0.025)` sat BELOW the same-terms 100y European, an internal
    /// arbitrage). Puts on the same carries are UNAFFECTED: the `y₂ ≤ 0`
    /// branch exists for every `b` and the put payoff is bounded by `K`.
    #[test]
    fn call_carry_strictly_exceeding_discount_is_refused() {
        for &(r, b) in &[(0.03, 0.06), (0.02, 0.025), (0.0, 0.05), (0.05, 0.0500001)] {
            let i = inp(123.45, 100.0, 0.2, r, b);
            assert_eq!(
                perpetual_price(OptionType::Call, &i),
                Err(PerpetualError::CallCarryExceedsDiscount),
                "r={r} b={b}"
            );
            assert_eq!(
                perpetual_exercise_boundary(OptionType::Call, &i),
                Err(PerpetualError::CallCarryExceedsDiscount),
                "r={r} b={b}"
            );
            assert_eq!(
                perpetual_greeks(OptionType::Call, &i),
                Err(PerpetualError::CallCarryExceedsDiscount),
                "r={r} b={b}"
            );
            // The put is untouched by the call's divergence: finite, within
            // the no-arbitrage sandwich intrinsic ≤ V ≤ K.
            let put = perpetual_price(OptionType::Put, &i).unwrap();
            assert!(
                put.is_finite() && (0.0..=100.0).contains(&put),
                "put must stay finite/bounded at r={r} b={b}: {put}"
            );
        }
        // The typed message is the wire-facing contract text.
        assert_eq!(
            PerpetualError::CallCarryExceedsDiscount.to_string(),
            "a perpetual call with carry exceeding the discount rate has no finite value"
        );
    }

    /// The sub-ulp law: `b` strictly below `r` by as little as ONE ulp keeps
    /// the valuation finite and on the `y₁ → 1⁺` limit arm. In that window the
    /// finite-precision root collapses to exactly `y₁ = 1.0`, and the engine
    /// must take the exact limit arm (`V = S`, boundary `+∞`), never the `1/0`
    /// boundary evaluation whose value is `∞·0 = NaN` (adversarial-verify
    /// regression: the unguarded power form returned NaN at `b = r − 1 ulp`).
    #[test]
    fn call_sub_ulp_below_r_takes_the_limit_arm() {
        let r = 0.05f64;
        for ulps in 1..=64u64 {
            let b = f64::from_bits(r.to_bits() - ulps);
            assert!(b < r, "scan must stay strictly inside b < r");
            let i = inp(100.0, 100.0, 0.2, r, b);
            let p = perpetual_price(OptionType::Call, &i).unwrap();
            assert!(p.is_finite(), "NaN/inf at b = r - {ulps} ulps: {p}");
            // No-arbitrage sandwich: intrinsic ≤ V ≤ S, and the value sits on
            // the S limit to within the closed form's own collapse error
            // (measured ≤ 3e-11 absolute across the 64-ulp window).
            assert!((0.0..=100.0).contains(&p));
            assert_close!(p, 100.0, 1e-9, 1e-9);
            let g = perpetual_greeks(OptionType::Call, &i).unwrap();
            assert!(
                g.delta.is_finite() && g.gamma.is_finite() && g.vega.is_finite(),
                "non-finite Greeks at b = r - {ulps} ulps"
            );
            assert!(perpetual_exercise_boundary(OptionType::Call, &i).unwrap() > 100.0);
        }
    }

    /// Degenerate law: spot beyond the free boundary ⇒ the value is the
    /// immediate-exercise intrinsic exactly, with the intrinsic Greek strip.
    #[test]
    fn beyond_boundary_is_intrinsic_exactly() {
        // Call: S* = 356.38… (pinned above); S = 400 sits beyond it.
        let call = inp(400.0, 100.0, 0.30, 0.08, 0.04);
        assert_eq!(
            perpetual_price(OptionType::Call, &call).unwrap().to_bits(),
            300.0f64.to_bits()
        );
        let gc = perpetual_greeks(OptionType::Call, &call).unwrap();
        assert_eq!(gc.delta.to_bits(), 1.0f64.to_bits());
        assert_eq!(gc.gamma.to_bits(), 0.0f64.to_bits());
        assert_eq!(gc.vega.to_bits(), 0.0f64.to_bits());

        // Put: S** = 60.39… (pinned above); S = 40 sits beyond (below) it.
        let put = inp(40.0, 110.0, 0.25, 0.06, 0.02);
        assert_eq!(
            perpetual_price(OptionType::Put, &put).unwrap().to_bits(),
            70.0f64.to_bits()
        );
        let gp = perpetual_greeks(OptionType::Put, &put).unwrap();
        assert_eq!(gp.delta.to_bits(), (-1.0f64).to_bits());
        assert_eq!(gp.gamma.to_bits(), 0.0f64.to_bits());
        assert_eq!(gp.vega.to_bits(), 0.0f64.to_bits());
    }

    /// σ→0 limits against the INDEPENDENTLY-derived deterministic-exercise
    /// values. With no volatility the spot grows deterministically at `b`, so
    /// the optimal exercise maximizes `e^{−rt}·(±(S·e^{bt} − K))`: first-order
    /// condition `S·e^{bt*} = r·K/(r−b)` (both sides), giving
    /// `V = |S₀* − K|·(S/S₀*)^{r/b}` with `S₀* = rK/(r−b)` — derived from the
    /// deterministic optimization, never from the engine's quadratic.
    #[test]
    fn sigma_to_zero_deterministic_limits() {
        // Call, 0 < b < r: S₀* = 0.08·100/0.04 = 200, V = 100·(100/200)² = 25.
        let call = inp(100.0, 100.0, 1e-4, 0.08, 0.04);
        assert_close!(
            perpetual_price(OptionType::Call, &call).unwrap(),
            25.0,
            1e-4,
            1e-4
        );

        // Put, b < 0 < r: S₀* = 0.06·100/0.09 = 200/3,
        // V = (100 − 200/3)·(100/(200/3))^{0.06/−0.03} = (100/3)·(2/3)² = 400/27.
        let put = inp(100.0, 100.0, 1e-4, 0.06, -0.03);
        assert_close!(
            perpetual_price(OptionType::Put, &put).unwrap(),
            400.0 / 27.0,
            1e-4,
            1e-4
        );
    }

    /// `r = 0` put laws. With `b ≤ ½σ²` the small root is exactly 0: the
    /// boundary collapses and the value is the supremum `K` (downward log-drift
    /// reaches any level with probability one, undiscounted). With `b > ½σ²`
    /// the quadratic factors exactly as `y·(½σ²(y−1) + b)`, so
    /// `y₂ = 1 − 2b/σ²` — an analytic re-derivation disjoint from the engine's
    /// discriminant route.
    #[test]
    fn zero_rate_put_degeneracies() {
        // b = 0.01 ≤ ½σ² = 0.02 ⇒ V = K exactly, boundary 0, flat strip.
        let collapsed = inp(100.0, 90.0, 0.20, 0.0, 0.01);
        assert_eq!(
            perpetual_price(OptionType::Put, &collapsed)
                .unwrap()
                .to_bits(),
            90.0f64.to_bits()
        );
        assert_eq!(
            perpetual_exercise_boundary(OptionType::Put, &collapsed)
                .unwrap()
                .to_bits(),
            0.0f64.to_bits()
        );
        let g = perpetual_greeks(OptionType::Put, &collapsed).unwrap();
        assert_eq!(g.delta.to_bits(), 0.0f64.to_bits());
        assert_eq!(g.vega.to_bits(), 0.0f64.to_bits());

        // b = 0.05 > ½σ² = 0.02 ⇒ y₂ = 1 − 2·0.05/0.04 = −1.5,
        // S** = 90·(−1.5)/(−2.5) = 54, V = (90−54)·(100/54)^{−1.5}.
        let active = inp(100.0, 90.0, 0.20, 0.0, 0.05);
        let y2 = 1.0 - 2.0 * 0.05 / (0.20 * 0.20);
        let boundary = 90.0 * y2 / (y2 - 1.0);
        let reference = (90.0 - boundary) * (100.0f64 / boundary).powf(y2);
        assert_close!(
            perpetual_exercise_boundary(OptionType::Put, &active).unwrap(),
            54.0,
            1e-12,
            1e-12
        );
        assert_close!(
            perpetual_price(OptionType::Put, &active).unwrap(),
            reference,
            1e-12,
            1e-12
        );
    }

    /// Structural monotonicity: the call is non-decreasing and the put
    /// non-increasing in spot; both are non-decreasing in vol; and every value
    /// respects the no-arbitrage sandwich (≥ intrinsic; call ≤ S, put ≤ K).
    #[test]
    fn monotone_in_spot_and_vol_with_bounds() {
        let (k, r, b) = (100.0, 0.08, 0.02);
        let mut prev_call = f64::NEG_INFINITY;
        let mut prev_put = f64::INFINITY;
        for step in 0..=72 {
            let s = 40.0 + 5.0 * f64::from(step);
            let i = inp(s, k, 0.30, r, b);
            let c = perpetual_price(OptionType::Call, &i).unwrap();
            let p = perpetual_price(OptionType::Put, &i).unwrap();
            assert!(c >= prev_call - 1e-12, "call not monotone in S at {s}");
            assert!(p <= prev_put + 1e-12, "put not monotone in S at {s}");
            assert!(c >= (s - k).max(0.0) - 1e-12 && c <= s + 1e-12);
            assert!(p >= (k - s).max(0.0) - 1e-12 && p <= k + 1e-12);
            (prev_call, prev_put) = (c, p);
        }
        let mut prev = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for step in 1..=20 {
            let vol = 0.05 * f64::from(step);
            let i = inp(90.0, k, vol, r, b);
            let c = perpetual_price(OptionType::Call, &i).unwrap();
            let p = perpetual_price(OptionType::Put, &i).unwrap();
            assert!(c >= prev.0 - 1e-12, "call not monotone in σ at {vol}");
            assert!(p >= prev.1 - 1e-12, "put not monotone in σ at {vol}");
            prev = (c, p);
        }
    }

    /// Every analytic Greek against an independent central finite difference of
    /// the price, on both carry arms (the FX arm exercises the chain-rule
    /// tagging through `r = r_dom`, `b = r_dom − r_for`).
    #[test]
    fn analytic_greeks_match_central_finite_difference() {
        let fd1 = |f: &dyn Fn(f64) -> f64, x: f64, h: f64| (f(x + h) - f(x - h)) / (2.0 * h);

        // Cost-of-carry arm.
        for &(s, k, vol, r, b) in &[
            (100.0, 100.0, 0.30, 0.08, 0.04), // call continuation
            (100.0, 110.0, 0.25, 0.06, 0.02), // put continuation
            (80.0, 95.0, 0.40, 0.07, -0.03),
            (1.30, 1.25, 0.10, 0.05, 0.01),
        ] {
            for opt in [OptionType::Call, OptionType::Put] {
                let i = inp(s, k, vol, r, b);
                let g = perpetual_greeks(opt, &i).unwrap();
                assert_eq!(
                    g.price.to_bits(),
                    perpetual_price(opt, &i).unwrap().to_bits()
                );

                let hs = 1e-4 * s;
                let at_spot =
                    |x: f64| perpetual_price(opt, &PerpetualInputs { spot: x, ..i }).unwrap();
                assert_close!(g.delta, fd1(&at_spot, s, hs), 1e-6, 1e-9);
                let delta_at = |x: f64| {
                    perpetual_greeks(opt, &PerpetualInputs { spot: x, ..i })
                        .unwrap()
                        .delta
                };
                assert_close!(g.gamma, fd1(&delta_at, s, hs), 1e-6, 1e-9);

                let at_vol =
                    |x: f64| perpetual_price(opt, &PerpetualInputs { vol: x, ..i }).unwrap();
                assert_close!(g.vega, fd1(&at_vol, vol, 1e-6), 1e-6, 1e-8);

                let at_r = |x: f64| {
                    perpetual_price(
                        opt,
                        &PerpetualInputs {
                            carry: cost_of_carry(x, b),
                            ..i
                        },
                    )
                    .unwrap()
                };
                let at_b = |x: f64| {
                    perpetual_price(
                        opt,
                        &PerpetualInputs {
                            carry: cost_of_carry(r, x),
                            ..i
                        },
                    )
                    .unwrap()
                };
                match g.rates {
                    RateSensitivities::Carry {
                        discount_rho,
                        carry_rho,
                    } => {
                        assert_close!(discount_rho, fd1(&at_r, r, 1e-7), 1e-5, 1e-7);
                        assert_close!(carry_rho, fd1(&at_b, b, 1e-7), 1e-5, 1e-7);
                    }
                    RateSensitivities::Fx { .. } => {
                        panic!("cost-of-carry input must tag rhos as Carry")
                    }
                }
            }
        }

        // FX arm: rho_dom / rho_for by native two-rate bumps.
        let fx = PerpetualInputs::new(
            1.30,
            1.25,
            0.10,
            Carry::FxRates {
                r_dom: 0.05,
                r_for: 0.01,
            },
        );
        for opt in [OptionType::Call, OptionType::Put] {
            let g = perpetual_greeks(opt, &fx).unwrap();
            let at_dom = |x: f64| {
                perpetual_price(
                    opt,
                    &PerpetualInputs {
                        carry: Carry::FxRates {
                            r_dom: x,
                            r_for: 0.01,
                        },
                        ..fx
                    },
                )
                .unwrap()
            };
            let at_for = |x: f64| {
                perpetual_price(
                    opt,
                    &PerpetualInputs {
                        carry: Carry::FxRates {
                            r_dom: 0.05,
                            r_for: x,
                        },
                        ..fx
                    },
                )
                .unwrap()
            };
            match g.rates {
                RateSensitivities::Fx { rho_dom, rho_for } => {
                    assert_close!(rho_dom, fd1(&at_dom, 0.05, 1e-7), 1e-5, 1e-7);
                    assert_close!(rho_for, fd1(&at_for, 0.01, 1e-7), 1e-5, 1e-7);
                }
                RateSensitivities::Carry { .. } => panic!("FX input must tag rhos as Fx"),
            }
        }
    }

    /// Smooth pasting: approaching the free boundary from the continuation
    /// region, the value meets the intrinsic and the delta meets ±1 — the
    /// defining property of the optimal boundary (a kink would mean the
    /// boundary is suboptimal).
    #[test]
    fn smooth_pasting_at_the_boundary() {
        let call = inp(100.0, 100.0, 0.30, 0.08, 0.04);
        let s_star = perpetual_exercise_boundary(OptionType::Call, &call).unwrap();
        let just_inside = PerpetualInputs {
            spot: s_star * (1.0 - 1e-7),
            ..call
        };
        assert_close!(
            perpetual_price(OptionType::Call, &just_inside).unwrap(),
            s_star - 100.0,
            1e-5,
            1e-5
        );
        assert_close!(
            perpetual_greeks(OptionType::Call, &just_inside)
                .unwrap()
                .delta,
            1.0,
            1e-5,
            1e-5
        );

        let put = inp(100.0, 110.0, 0.25, 0.06, 0.02);
        let s_dstar = perpetual_exercise_boundary(OptionType::Put, &put).unwrap();
        let just_inside = PerpetualInputs {
            spot: s_dstar * (1.0 + 1e-7),
            ..put
        };
        assert_close!(
            perpetual_price(OptionType::Put, &just_inside).unwrap(),
            110.0 - s_dstar,
            1e-5,
            1e-5
        );
        assert_close!(
            perpetual_greeks(OptionType::Put, &just_inside)
                .unwrap()
                .delta,
            -1.0,
            1e-5,
            1e-5
        );
    }
}
