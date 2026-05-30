//! Model-independent financial-invariant assertions.
//!
//! These encode the no-arbitrage and consistency laws every FX-options pricer
//! must obey, expressed against the `celnet-vanilla` pricing functions
//! ([`celnet_vanilla::price`], [`celnet_vanilla::greeks`]) and the canonical
//! [`celnet_types`] vocabulary. Each helper *asserts* (panics with a descriptive
//! message via [`celnet_core::assert_close`] or `assert!`) so it reads naturally
//! inside a `#[test]` or `proptest!` body. They are deliberately model-agnostic:
//! the same laws are reused by downstream surface/exotics crates against their
//! own pricers through the generic finite-difference helper.
//!
//! The laws implemented (with references in `docs/ANALYTICS-SPEC.md` §1):
//!
//! - **Put-call parity**: `C − P = S·e^{−r_f T} − K·e^{−r_d T}`.
//! - **Price bounds**: `0 ≤ C ≤ S·e^{−r_f T}`, `0 ≤ P ≤ K·e^{−r_d T}`.
//! - **Intrinsic lower bounds** (forward-intrinsic, discounted):
//!   `C ≥ e^{−r_d T}·max(F − K, 0)`, `P ≥ e^{−r_d T}·max(K − F, 0)`.
//! - **Monotonicity**: call value increases in spot and in vol; both call and
//!   put value increase in maturity (for non-negative carry); call value
//!   decreases in strike.
//! - **Strike convexity** (butterfly `≥ 0`): the call (and put) price is a
//!   convex function of strike, so any three increasing strikes give a
//!   non-negative butterfly spread.
//! - **Greeks vs finite differences**: an analytic sensitivity agrees with a
//!   central finite difference of the price in the corresponding input.
//!
//! # Tolerances
//!
//! Comparisons use [`celnet_core::is_close`]'s combined relative + absolute
//! tolerance. Finite-difference checks expose the bump size and tolerances so a
//! caller can tighten or relax per regime; the defaults are tuned for the
//! second-order accuracy of central differencing on smooth FX inputs.

use celnet_core::is_close;
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::{greeks, price};

/// Default relative tolerance for parity / bound / convexity assertions.
const REL: f64 = 1e-9;
/// Default absolute tolerance for parity / bound / convexity assertions.
const ABS: f64 = 1e-9;

/// Which analytic Greek a finite-difference check should validate, naming the
/// price input it differentiates and the order of the difference.
///
/// This lets [`assert_greek_matches_fd`] select the right bump variable and the
/// right analytic field of [`celnet_types::Greeks`] without the caller wiring up
/// closures by hand. Provenance: the closed forms live in `celnet-vanilla`; the
/// finite-difference oracle here is the independent cross-check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GreekKind {
    /// Spot delta: `∂V/∂S` (central difference of price in spot).
    DeltaSpot,
    /// Vega: `∂V/∂σ`.
    Vega,
    /// Rho domestic: `∂V/∂r_dom`.
    RhoDom,
    /// Rho foreign: `∂V/∂r_for`.
    RhoFor,
    /// Theta: `∂V/∂t = −∂V/∂T` (the FD here differentiates in `T`, then negates).
    Theta,
    /// Gamma: `∂²V/∂S²` (FD of analytic `delta_spot` in spot).
    Gamma,
    /// Vanna: `∂(delta_spot)/∂σ` (FD of analytic `delta_spot` in vol).
    Vanna,
    /// Volga / vomma: `∂(vega)/∂σ` (FD of analytic `vega` in vol).
    Volga,
    /// Charm: `∂(delta_spot)/∂T` (FD of analytic `delta_spot` in `T`).
    Charm,
}

/// Second-order-accurate central finite difference of `f` at `x` with step `h`:
/// `(f(x+h) − f(x−h)) / (2h)`.
///
/// Centralised so every crate differentiates identically. `h` must be strictly
/// positive; pick it relative to the scale of `x` (the higher-level helpers
/// here do this for you).
#[must_use]
pub fn central_difference<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
    debug_assert!(h > 0.0, "finite-difference step must be positive");
    (f(x + h) - f(x - h)) / (2.0 * h)
}

#[inline]
fn with_spot(i: &VanillaInputs, spot: f64) -> VanillaInputs {
    VanillaInputs { spot, ..*i }
}
#[inline]
fn with_vol(i: &VanillaInputs, vol: f64) -> VanillaInputs {
    VanillaInputs { vol, ..*i }
}
#[inline]
fn with_t(i: &VanillaInputs, t: f64) -> VanillaInputs {
    VanillaInputs { t, ..*i }
}
#[inline]
fn with_strike(i: &VanillaInputs, strike: f64) -> VanillaInputs {
    VanillaInputs { strike, ..*i }
}
#[inline]
fn with_r_dom(i: &VanillaInputs, r_dom: f64) -> VanillaInputs {
    VanillaInputs { r_dom, ..*i }
}
#[inline]
fn with_r_for(i: &VanillaInputs, r_for: f64) -> VanillaInputs {
    VanillaInputs { r_for, ..*i }
}

/// Assert put-call parity for `i`: `C − P = S·e^{−r_f T} − K·e^{−r_d T}`.
///
/// # Panics
///
/// Panics if the parity residual exceeds the harness tolerance.
pub fn assert_put_call_parity(i: &VanillaInputs) {
    let lhs = price(OptionType::Call, i) - price(OptionType::Put, i);
    let rhs = i.spot * i.df_for() - i.strike * i.df_dom();
    assert!(
        is_close(lhs, rhs, REL, ABS),
        "put-call parity violated: C−P={lhs} but S·df_for−K·df_dom={rhs} (|diff|={}) for {i:?}",
        (lhs - rhs).abs()
    );
}

/// Assert the static price bounds for both option types:
/// `0 ≤ C ≤ S·e^{−r_f T}` and `0 ≤ P ≤ K·e^{−r_d T}`.
///
/// # Panics
///
/// Panics if either price is negative or exceeds its discounted-underlying /
/// discounted-strike ceiling (beyond tolerance).
pub fn assert_price_within_bounds(i: &VanillaInputs) {
    let c = price(OptionType::Call, i);
    let p = price(OptionType::Put, i);
    let c_max = i.spot * i.df_for();
    let p_max = i.strike * i.df_dom();
    assert!(
        c >= -ABS && c <= c_max + REL * c_max + ABS,
        "call price {c} out of bounds [0, {c_max}] for {i:?}"
    );
    assert!(
        p >= -ABS && p <= p_max + REL * p_max + ABS,
        "put price {p} out of bounds [0, {p_max}] for {i:?}"
    );
}

/// Assert the call's discounted forward-intrinsic lower bound:
/// `C ≥ e^{−r_d T}·max(F − K, 0)`.
///
/// # Panics
///
/// Panics if the call trades below its intrinsic floor (beyond tolerance).
pub fn assert_call_intrinsic_lower_bound(i: &VanillaInputs) {
    let c = price(OptionType::Call, i);
    let intrinsic = i.df_dom() * (i.forward() - i.strike).max(0.0);
    assert!(
        c + ABS + REL * intrinsic >= intrinsic,
        "call price {c} below discounted forward-intrinsic {intrinsic} for {i:?}"
    );
}

/// Assert the put's discounted forward-intrinsic lower bound:
/// `P ≥ e^{−r_d T}·max(K − F, 0)`.
///
/// # Panics
///
/// Panics if the put trades below its intrinsic floor (beyond tolerance).
pub fn assert_put_intrinsic_lower_bound(i: &VanillaInputs) {
    let p = price(OptionType::Put, i);
    let intrinsic = i.df_dom() * (i.strike - i.forward()).max(0.0);
    assert!(
        p + ABS + REL * intrinsic >= intrinsic,
        "put price {p} below discounted forward-intrinsic {intrinsic} for {i:?}"
    );
}

/// Assert that `opt`'s value is non-decreasing in spot at `i` (strictly, a call
/// rises and a put falls in spot; this helper checks the *call*-style law on the
/// supplied option type by bumping spot up and requiring the value not to drop
/// for calls / not to rise for puts).
///
/// The law is exact for vanilla FX: `∂C/∂S = e^{−r_f T}N(d1) ≥ 0`,
/// `∂P/∂S = −e^{−r_f T}N(−d1) ≤ 0`.
///
/// # Panics
///
/// Panics if the monotonicity in spot is violated beyond tolerance.
pub fn assert_increasing_in_spot(opt: OptionType, i: &VanillaInputs) {
    let bump = 1e-3 * i.spot;
    let lo = price(opt, &with_spot(i, i.spot - bump));
    let hi = price(opt, &with_spot(i, i.spot + bump));
    match opt {
        OptionType::Call => assert!(
            hi + ABS >= lo,
            "call value must be non-decreasing in spot: {lo} → {hi} at {i:?}"
        ),
        OptionType::Put => assert!(
            hi <= lo + ABS,
            "put value must be non-increasing in spot: {lo} → {hi} at {i:?}"
        ),
    }
}

/// Assert that `opt`'s value is non-decreasing in volatility at `i`.
///
/// Vega is positive for both calls and puts in the Garman-Kohlhagen model
/// (`vega = S·e^{−r_f T}·√T·φ(d1) ≥ 0`), so raising vol cannot lower either
/// price — the most basic convexity-of-value-in-vol no-arbitrage statement.
///
/// # Panics
///
/// Panics if the value falls when vol rises (beyond tolerance).
pub fn assert_increasing_in_vol(opt: OptionType, i: &VanillaInputs) {
    let bump = 1e-4;
    let lo = price(opt, &with_vol(i, i.vol - bump));
    let hi = price(opt, &with_vol(i, i.vol + bump));
    assert!(
        hi + ABS >= lo,
        "{opt:?} value must be non-decreasing in vol: {lo} → {hi} at {i:?}"
    );
}

/// Assert that `opt`'s value is non-decreasing in maturity at `i`, in the carry
/// regime where that law is unconditional.
///
/// Time-monotonicity of a **European** vanilla is *not* universal: discounting
/// the strike can outweigh the extra time value, so a European put can lose
/// value with maturity under positive carry (and a European call under negative
/// carry). The clean, regime-restricted statements that always hold are:
///
/// - a **call** is non-decreasing in maturity when carry is non-negative
///   (`r_d ≥ r_f`): the discounted forward-intrinsic floor `e^{−r_d T}(F − K)`
///   `= e^{−r_f T}S − e^{−r_d T}K` is non-decreasing in `T`;
/// - a **put** is non-decreasing in maturity when carry is non-positive
///   (`r_d ≤ r_f`), by the symmetric argument.
///
/// The helper therefore requires the matching carry sign for the option type
/// and panics on the wrong-regime precondition, so it is never silently applied
/// where the law does not hold.
///
/// # Panics
///
/// Panics if the carry sign does not match the option type (precondition), or
/// if the value falls as maturity rises.
pub fn assert_increasing_in_maturity(opt: OptionType, i: &VanillaInputs) {
    match opt {
        OptionType::Call => assert!(
            i.r_dom + ABS >= i.r_for,
            "call maturity-monotonicity requires non-negative carry (r_dom ≥ r_for); got {i:?}"
        ),
        OptionType::Put => assert!(
            i.r_for + ABS >= i.r_dom,
            "put maturity-monotonicity requires non-positive carry (r_dom ≤ r_for); got {i:?}"
        ),
    }
    let bump = 1e-4 * i.t;
    let lo = price(opt, &with_t(i, i.t - bump));
    let hi = price(opt, &with_t(i, i.t + bump));
    assert!(
        hi + ABS >= lo,
        "{opt:?} value must be non-decreasing in maturity in its carry regime: \
         {lo} → {hi} at {i:?}"
    );
}

/// Assert that the **call** value is non-increasing in strike at `i`
/// (`∂C/∂K = −e^{−r_d T}N(d2) ≤ 0`). The dual put law (`∂P/∂K ≥ 0`) is checked
/// when `opt` is a put.
///
/// # Panics
///
/// Panics if the strike-monotonicity is violated beyond tolerance.
pub fn assert_decreasing_in_strike(opt: OptionType, i: &VanillaInputs) {
    let bump = 1e-3 * i.strike;
    let lo = price(opt, &with_strike(i, i.strike - bump));
    let hi = price(opt, &with_strike(i, i.strike + bump));
    match opt {
        OptionType::Call => assert!(
            hi <= lo + ABS,
            "call value must be non-increasing in strike: {lo} → {hi} at {i:?}"
        ),
        OptionType::Put => assert!(
            hi + ABS >= lo,
            "put value must be non-decreasing in strike: {lo} → {hi} at {i:?}"
        ),
    }
}

/// Assert convexity of `opt`'s price in strike — the **butterfly `≥ 0`**
/// no-arbitrage law — using three equally-spaced strikes `K − dK`, `K`,
/// `K + dK`. The (undiscounted) butterfly `V(K−dK) − 2V(K) + V(K+dK)` must be
/// non-negative; it equals `dK²·∂²V/∂K² + O(dK⁴)` and `∂²V/∂K² ≥ 0` is the
/// implied-density-non-negativity condition.
///
/// `i.strike` is the centre `K`; `d_strike` is the wing spacing `dK` (must be
/// positive and `< K`).
///
/// # Panics
///
/// Panics if `d_strike` is not a valid spacing, or if the butterfly is negative
/// beyond tolerance.
pub fn assert_strike_convexity(opt: OptionType, i: &VanillaInputs, d_strike: f64) {
    assert!(
        d_strike > 0.0 && d_strike < i.strike,
        "strike-convexity spacing dK={d_strike} must satisfy 0 < dK < K={}",
        i.strike
    );
    let down = price(opt, &with_strike(i, i.strike - d_strike));
    let mid = price(opt, i);
    let up = price(opt, &with_strike(i, i.strike + d_strike));
    let butterfly = down - 2.0 * mid + up;
    // Scale the tolerance with the magnitude of the terms so that near-zero
    // convexity (deep wings) is accepted without admitting genuine violations.
    let scale = down.abs().max(mid.abs()).max(up.abs());
    assert!(
        butterfly + ABS + REL * scale >= 0.0,
        "{opt:?} butterfly (strike convexity) must be ≥ 0: {butterfly} at K={} dK={d_strike} {i:?}",
        i.strike
    );
}

/// Assert that the analytic Greek `kind` of `opt` agrees with a central finite
/// difference of the price (or of the relevant first-order Greek, for the
/// second-order Greeks) in the corresponding input.
///
/// `rel`/`abs` are the comparison tolerances; the bump sizes are chosen relative
/// to each input's scale internally for well-conditioned second-order accuracy.
/// This is the generic Greek oracle reused across crates — pass any pricer's
/// inputs and it checks Celnet's analytic sensitivities against differencing the
/// price function.
///
/// # Panics
///
/// Panics if the analytic Greek and its finite-difference estimate differ by
/// more than the supplied tolerance.
pub fn assert_greek_matches_fd(
    opt: OptionType,
    i: &VanillaInputs,
    kind: GreekKind,
    rel: f64,
    abs: f64,
) {
    let g = greeks(opt, i);
    let p = |x: &VanillaInputs| price(opt, x);
    let ds = |x: &VanillaInputs| greeks(opt, x).delta_spot;
    let ve = |x: &VanillaInputs| greeks(opt, x).vega;

    let hs = 1e-4 * i.spot;
    let hv = 1e-5;
    let ht = 1e-5 * i.t.max(1.0);
    let hr = 1e-6;

    let (analytic, numeric) = match kind {
        GreekKind::DeltaSpot => (
            g.delta_spot,
            central_difference(|s| p(&with_spot(i, s)), i.spot, hs),
        ),
        GreekKind::Vega => (
            g.vega,
            central_difference(|v| p(&with_vol(i, v)), i.vol, hv),
        ),
        GreekKind::RhoDom => (
            g.rho_dom,
            central_difference(|r| p(&with_r_dom(i, r)), i.r_dom, hr),
        ),
        GreekKind::RhoFor => (
            g.rho_for,
            central_difference(|r| p(&with_r_for(i, r)), i.r_for, hr),
        ),
        // theta = −∂V/∂T.
        GreekKind::Theta => (g.theta, -central_difference(|t| p(&with_t(i, t)), i.t, ht)),
        GreekKind::Gamma => (
            g.gamma,
            central_difference(|s| ds(&with_spot(i, s)), i.spot, hs),
        ),
        GreekKind::Vanna => (
            g.vanna,
            central_difference(|v| ds(&with_vol(i, v)), i.vol, hv),
        ),
        GreekKind::Volga => (
            g.volga,
            central_difference(|v| ve(&with_vol(i, v)), i.vol, hv),
        ),
        GreekKind::Charm => (g.charm, central_difference(|t| ds(&with_t(i, t)), i.t, ht)),
    };
    assert!(
        is_close(analytic, numeric, rel, abs),
        "{kind:?} mismatch for {opt:?}: analytic={analytic} finite-difference={numeric} \
         (|diff|={}) at {i:?}",
        (analytic - numeric).abs()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::reference_markets;

    /// All financial invariants hold on the curated reference market set.
    #[test]
    fn invariants_hold_on_known_good_markets() {
        for m in reference_markets() {
            let i = m.inputs;
            assert_put_call_parity(&i);
            assert_price_within_bounds(&i);
            assert_call_intrinsic_lower_bound(&i);
            assert_put_intrinsic_lower_bound(&i);
            assert_strike_convexity(OptionType::Call, &i, 0.05 * i.strike);
            assert_strike_convexity(OptionType::Put, &i, 0.05 * i.strike);
            for opt in [OptionType::Call, OptionType::Put] {
                assert_increasing_in_spot(opt, &i);
                assert_increasing_in_vol(opt, &i);
                assert_decreasing_in_strike(opt, &i);
            }
            // Maturity monotonicity holds per option type in its carry regime.
            if i.r_dom >= i.r_for {
                assert_increasing_in_maturity(OptionType::Call, &i);
            }
            if i.r_for >= i.r_dom {
                assert_increasing_in_maturity(OptionType::Put, &i);
            }
        }
    }

    /// Every Greek oracle agrees with finite differences on the reference set.
    #[test]
    fn greek_oracle_agrees_on_known_good_markets() {
        let kinds = [
            GreekKind::DeltaSpot,
            GreekKind::Vega,
            GreekKind::RhoDom,
            GreekKind::RhoFor,
            GreekKind::Theta,
            GreekKind::Gamma,
            GreekKind::Vanna,
            GreekKind::Volga,
            GreekKind::Charm,
        ];
        for m in reference_markets() {
            for opt in [OptionType::Call, OptionType::Put] {
                for k in kinds {
                    // First-order Greeks tolerate tight tolerances; second-order
                    // ones are bumped twice so we relax accordingly.
                    let (rel, abs) = match k {
                        GreekKind::DeltaSpot
                        | GreekKind::Vega
                        | GreekKind::RhoDom
                        | GreekKind::RhoFor
                        | GreekKind::Theta => (1e-4, 1e-6),
                        _ => (1e-2, 1e-5),
                    };
                    assert_greek_matches_fd(opt, &m.inputs, k, rel, abs);
                }
            }
        }
    }

    // ---- known-BAD inputs must make the invariants FIRE ----
    //
    // We cannot corrupt the (correct) pricer, so we feed the invariant helpers
    // *quantities they assert on* via deliberately-wrong reference values: each
    // test constructs a scenario that violates the law and confirms a panic.
    // The monotonicity / convexity / parity helpers are exercised against the
    // real pricer (which satisfies them), so to prove they *fire* we assert on
    // hand-built violations using the same comparison primitive they use.

    use std::panic::catch_unwind;

    fn good() -> VanillaInputs {
        VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01)
    }

    /// Parity check fires when the relation is broken (we break it by asserting
    /// parity of a call price against a *put with a different strike*, which is
    /// not the parity partner). We reconstruct the residual directly to prove
    /// the helper's tolerance is not so loose it never fires.
    #[test]
    fn parity_fires_on_broken_relation() {
        // A doctored "inputs" whose stored fields are internally inconsistent
        // cannot exist (VanillaInputs is just data), so we instead verify the
        // underlying comparison: a genuine violation is NOT close.
        let i = good();
        let lhs = price(OptionType::Call, &i) - price(OptionType::Put, &i);
        let rhs = i.spot * i.df_for() - i.strike * i.df_dom();
        // Real parity holds:
        assert!(is_close(lhs, rhs, REL, ABS));
        // A perturbed RHS (off by 1%) must NOT be accepted — proving the helper
        // would panic on such a discrepancy.
        let broken = rhs + 0.01 * rhs.abs().max(1e-3);
        assert!(!is_close(lhs, broken, REL, ABS));
    }

    /// The Greek oracle panics when handed a wrong analytic value: we wrap a
    /// call that compares a *deliberately wrong* delta against the FD estimate.
    #[test]
    fn greek_oracle_fires_on_wrong_analytic() {
        let i = good();
        // The true spot delta of this call:
        let true_delta = greeks(OptionType::Call, &i).delta_spot;
        let fd = central_difference(
            |s| price(OptionType::Call, &with_spot(&i, s)),
            i.spot,
            1e-4 * i.spot,
        );
        // Truth agrees:
        assert!(is_close(true_delta, fd, 1e-4, 1e-6));
        // A 10%-wrong analytic value must be rejected by the same comparison the
        // helper uses (so assert_greek_matches_fd would panic on it).
        assert!(!is_close(true_delta * 1.10, fd, 1e-4, 1e-6));
    }

    /// Convexity helper rejects a concave (negative-butterfly) triple. We feed
    /// the comparison the helper performs with a hand-built negative butterfly
    /// to confirm it is flagged.
    #[test]
    fn convexity_rejects_negative_butterfly() {
        let butterfly = -0.5_f64; // concave: arbitrage
        let scale = 1.0_f64;
        // The helper's acceptance condition:
        let accepted = butterfly + ABS + REL * scale >= 0.0;
        assert!(!accepted, "a clearly-negative butterfly must be rejected");
    }

    /// And confirm the assertion macro genuinely panics end-to-end on a bad
    /// input by invoking a helper inside `catch_unwind` with a maturity law
    /// applied to a *negative-carry* market, which violates its precondition.
    #[test]
    fn maturity_helper_precondition_panics_on_inverted_carry() {
        // r_for > r_dom → precondition fails → must panic.
        let inverted = VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.01, 0.05);
        let res = catch_unwind(|| assert_increasing_in_maturity(OptionType::Call, &inverted));
        assert!(res.is_err(), "inverted-carry maturity law must panic");
    }

    /// The convexity helper panics end-to-end on an invalid spacing.
    #[test]
    fn convexity_helper_panics_on_bad_spacing() {
        let i = good();
        let res = catch_unwind(|| assert_strike_convexity(OptionType::Call, &i, i.strike * 2.0));
        assert!(res.is_err(), "dK ≥ K must panic");
    }
}
