//! Convention-aware option delta.
//!
//! FX implied vols are quoted against **delta, not strike**, so every strike
//! must be expressed as the delta of the configured [`DeltaConvention`]
//! (`docs/ANALYTICS-SPEC.md` §1.1). The four conventions are spot/forward ×
//! premium-unadjusted/premium-adjusted; for a call,
//!
//! ```text
//!   (1) spot unadjusted      Δ = e^{-r_f T} · N(d1)
//!   (2) forward unadjusted    Δ = N(d1)
//!   (3) spot prem-adjusted    Δ = e^{-r_f T} · (K/F) · N(d2)
//!   (4) forward prem-adjusted Δ = (K/F) · N(d2)
//! ```
//!
//! The put delta in each convention is the call delta minus the delta of the
//! "carried" leg: `e^{-r_f T}` (spot) or `1` (forward) for the unadjusted
//! styles, and `factor·(K/F)` for the premium-adjusted styles (because
//! premium-adjusted put `= factor·(K/F)·N(-d2) = factor·(K/F) − Δ_call`).
//!
//! The premium-adjusted **call** delta is *non-monotone* in strike — it rises
//! from 0, peaks at a "maximum-delta strike", then falls — so strike↔delta
//! inversion on that branch needs guarded bracketing. The closed-form location
//! of that maximum is provided by [`premium_adjusted_call_delta_max`] (Wystup,
//! "FX Options and Structured Products", 2nd ed., 2017, §1.5.6).

use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::{DeltaConvention, OptionType, VanillaInputs};

/// `d1`, `d2` and `σ√T` for a set of inputs (shared with the pricer).
pub(crate) struct DeltaAux {
    pub(crate) d1: f64,
    pub(crate) d2: f64,
    pub(crate) factor: f64, // e^{-r_f T} for spot deltas, 1 for forward deltas
    pub(crate) k_over_f: f64,
}

#[inline]
pub(crate) fn delta_aux(conv: DeltaConvention, i: &VanillaInputs) -> DeltaAux {
    let sqt = sqrt(i.t);
    let vsqt = i.vol * sqt;
    let d1 = (ln(i.spot / i.strike) + (i.r_dom - i.r_for + 0.5 * i.vol * i.vol) * i.t) / vsqt;
    let d2 = d1 - vsqt;
    let factor = match conv {
        DeltaConvention::SpotUnadjusted | DeltaConvention::SpotPremiumAdjusted => {
            exp(-i.r_for * i.t)
        }
        DeltaConvention::ForwardUnadjusted | DeltaConvention::ForwardPremiumAdjusted => 1.0,
    };
    let k_over_f = i.strike / i.forward();
    DeltaAux {
        d1,
        d2,
        factor,
        k_over_f,
    }
}

/// Signed delta of `opt` in the configured [`DeltaConvention`].
///
/// Calls are non-negative, puts non-positive. The value is the market-quoted
/// hedge ratio used to label strikes (e.g. the `25Δ` call strike is the strike
/// whose call delta in this convention equals `0.25`).
#[must_use]
pub fn delta(conv: DeltaConvention, opt: OptionType, i: &VanillaInputs) -> f64 {
    let a = delta_aux(conv, i);
    match conv {
        DeltaConvention::SpotUnadjusted | DeltaConvention::ForwardUnadjusted => match opt {
            OptionType::Call => a.factor * norm_cdf(a.d1),
            OptionType::Put => a.factor * (norm_cdf(a.d1) - 1.0),
        },
        DeltaConvention::SpotPremiumAdjusted | DeltaConvention::ForwardPremiumAdjusted => {
            let call = a.factor * a.k_over_f * norm_cdf(a.d2);
            match opt {
                OptionType::Call => call,
                // premium-adjusted put = factor·(K/F)·N(-d2) = factor·(K/F) − Δ_call.
                OptionType::Put => call - a.factor * a.k_over_f,
            }
        }
    }
}

/// `∂Δ/∂K` for `opt` in the configured convention — the slope used by the
/// Newton refinement step of the strike↔delta solver. Closed form,
/// deterministic.
///
/// For the unadjusted conventions the put delta differs from the call delta by a
/// strike-independent constant, so the two share a slope. For the
/// premium-adjusted conventions the put carries the extra `−factor·(K/F)` term,
/// whose `−factor/F` slope must be added.
#[must_use]
pub fn delta_d_strike(conv: DeltaConvention, opt: OptionType, i: &VanillaInputs) -> f64 {
    let sqt = sqrt(i.t);
    let vsqt = i.vol * sqt;
    let a = delta_aux(conv, i);
    // ∂d1/∂K = ∂d2/∂K = −1/(K σ√T).
    let dd_dk = -1.0 / (i.strike * vsqt);
    match conv {
        DeltaConvention::SpotUnadjusted | DeltaConvention::ForwardUnadjusted => {
            // Call and put deltas differ by a K-independent constant.
            a.factor * norm_pdf(a.d1) * dd_dk
        }
        DeltaConvention::SpotPremiumAdjusted | DeltaConvention::ForwardPremiumAdjusted => {
            // Δ_call = factor·(K/F)·N(d2); product rule in K (F is K-independent).
            let f = i.forward();
            let call_slope =
                a.factor * (norm_cdf(a.d2) / f + (i.strike / f) * norm_pdf(a.d2) * dd_dk);
            match opt {
                OptionType::Call => call_slope,
                // Δ_put = Δ_call − factor·(K/F) ⇒ slope_put = slope_call − factor/F.
                OptionType::Put => call_slope - a.factor / f,
            }
        }
    }
}

/// The strike at which the premium-adjusted **call** delta attains its maximum.
///
/// On the premium-adjusted branch `Δ(K) = factor·(K/F)·N(d2(K))` is non-monotone:
/// it increases from `0`, peaks, then decreases back to `0`. The peak is the
/// unique strike where `∂Δ/∂K = 0`, i.e. where `N(d2)·σ√T = φ(d2)`. With
/// `m = ln(K/F)` this is a scalar equation in `d2` independent of the spot/forward
/// factor, which we solve to machine precision by bisection. Returns the strike
/// `K_max`; the requested delta is reachable iff it does not exceed `Δ(K_max)`,
/// and the standard (low-strike, OTM-call) branch is `K ≤ K_max`.
///
/// Used by the solver to bracket on the correct branch and to cap the
/// achievable delta (`docs/ANALYTICS-SPEC.md` §3.5).
#[must_use]
pub fn premium_adjusted_call_delta_max(i: &VanillaInputs) -> f64 {
    let vsqt = i.vol * sqrt(i.t);
    let f = i.forward();
    // Solve g(d2) = N(d2)·σ√T − φ(d2) = 0. g is increasing in d2 (N grows, φ
    // falls for d2 ≥ 0), the root sits at small positive d2. Bracket generously.
    let g = |d2: f64| norm_cdf(d2) * vsqt - norm_pdf(d2);
    let mut lo = -20.0_f64;
    let mut hi = 20.0_f64;
    // g(lo) < 0, g(hi) > 0 by construction (N→0,φ→0 from below; N→1 dominates).
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if g(mid) > 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let d2_star = 0.5 * (lo + hi);
    // d2 = (ln(F/K) − ½σ²T)/(σ√T) ⇒ ln(K/F) = −½σ²T − d2·σ√T.
    let m = -0.5 * vsqt * vsqt - d2_star * vsqt;
    f * exp(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    const ALL: [DeltaConvention; 4] = [
        DeltaConvention::SpotUnadjusted,
        DeltaConvention::ForwardUnadjusted,
        DeltaConvention::SpotPremiumAdjusted,
        DeltaConvention::ForwardPremiumAdjusted,
    ];

    fn base() -> VanillaInputs {
        VanillaInputs::new(1.20, 1.25, 0.12, 0.75, 0.03, 0.015)
    }

    /// The spot-delta carry factor `e^{−r_f T}` must be applied EXACTLY: the spot
    /// unadjusted call delta equals `e^{−r_f T}·N(d1)` and the forward unadjusted
    /// call delta equals `N(d1)` (factor 1). Round-trip tests can absorb a wrong
    /// `factor` (the strike↔delta map stays self-consistent), so we pin the
    /// closed-form delta VALUE here — killing a mutant that drops the minus sign
    /// or flips the `r_for·t` product in `delta_aux`.
    #[test]
    fn spot_vs_forward_delta_carry_factor_is_exact() {
        let i = base();
        let a = delta_aux(DeltaConvention::SpotUnadjusted, &i);
        let nd1 = celnet_core::math::norm_cdf(a.d1);
        let carry = celnet_core::math::exp(-i.r_for * i.t);

        let spot_call = delta(DeltaConvention::SpotUnadjusted, OptionType::Call, &i);
        let fwd_call = delta(DeltaConvention::ForwardUnadjusted, OptionType::Call, &i);
        assert!(
            is_close(spot_call, carry * nd1, 1e-13, 1e-14),
            "spot call delta {spot_call} must equal e^(-r_f T)·N(d1) = {}",
            carry * nd1
        );
        assert!(
            is_close(fwd_call, nd1, 1e-13, 1e-14),
            "forward call delta {fwd_call} must equal N(d1) = {nd1}"
        );
        // The spot/forward ratio is exactly the carry factor (≠ 1 here since
        // r_for ≠ 0) — a dropped minus or wrong product breaks this.
        assert!(
            is_close(spot_call / fwd_call, carry, 1e-12, 1e-13),
            "spot/forward call-delta ratio must be the carry factor {carry}"
        );
        assert!(carry < 1.0, "with r_for>0 the carry factor must be < 1");
    }

    fn central_difference<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
        (f(x + h) - f(x - h)) / (2.0 * h)
    }

    /// `delta_d_strike` is the analytic `∂Δ/∂K`; difference the analytic `delta`
    /// in strike and require agreement, for every convention and option type.
    /// This is an INDEPENDENT finite-difference oracle for the solver's Newton
    /// slope — a mutant that flips a sign or drops a term in `delta_d_strike`
    /// would still leave bisection converging in the solver, so only a direct FD
    /// gate kills it.
    #[test]
    fn delta_d_strike_matches_finite_difference() {
        let i = base();
        let hk = 1e-6 * i.strike;
        for conv in ALL {
            for opt in [OptionType::Call, OptionType::Put] {
                // Stay clear of the premium-adjusted call turning point so the FD
                // is well-conditioned (the analytic identity holds everywhere; we
                // sample a clean point either side of the forward).
                for strike in [0.9 * i.forward(), 1.05 * i.forward(), 1.2 * i.forward()] {
                    let inp = VanillaInputs { strike, ..i };
                    let analytic = delta_d_strike(conv, opt, &inp);
                    let fd = central_difference(
                        |k| delta(conv, opt, &VanillaInputs { strike: k, ..inp }),
                        strike,
                        hk,
                    );
                    assert!(
                        is_close(analytic, fd, 1e-5, 1e-8),
                        "{conv:?} {opt:?} K={strike}: ∂Δ/∂K analytic {analytic} vs FD {fd}"
                    );
                }
            }
        }
    }

    /// At the premium-adjusted call maximum `K_max`, the call delta's strike
    /// derivative is exactly zero (the defining first-order condition of the
    /// turning point). This pins `premium_adjusted_call_delta_max` against the
    /// independent `delta_d_strike` oracle: a mutant that perturbs the bisection
    /// arithmetic (wrong `g`, wrong moneyness back-out) moves `K_max` off the
    /// stationary point and is caught.
    #[test]
    fn premium_adjusted_call_max_is_the_stationary_point() {
        for conv in [
            DeltaConvention::SpotPremiumAdjusted,
            DeltaConvention::ForwardPremiumAdjusted,
        ] {
            for i in [
                VanillaInputs::new(1.20, 1.20, 0.12, 0.75, 0.03, 0.015),
                VanillaInputs::new(100.0, 100.0, 0.25, 1.5, 0.02, 0.04),
                VanillaInputs::new(0.85, 0.85, 0.08, 0.25, 0.05, 0.01),
            ] {
                let k_max = premium_adjusted_call_delta_max(&i);
                let at_max = VanillaInputs { strike: k_max, ..i };
                // First-order condition: ∂Δ_call/∂K = 0 at K_max.
                let slope = delta_d_strike(conv, OptionType::Call, &at_max);
                assert!(
                    slope.abs() < 1e-7,
                    "{conv:?} K_max={k_max} not stationary: ∂Δ/∂K={slope}"
                );
                // And it is a MAXIMUM: delta is lower just either side.
                let dmax = delta(conv, OptionType::Call, &at_max);
                let lo = delta(
                    conv,
                    OptionType::Call,
                    &VanillaInputs {
                        strike: k_max * 0.97,
                        ..i
                    },
                );
                let hi = delta(
                    conv,
                    OptionType::Call,
                    &VanillaInputs {
                        strike: k_max * 1.03,
                        ..i
                    },
                );
                assert!(
                    dmax >= lo && dmax >= hi,
                    "{conv:?} K_max delta {dmax} is not a maximum (lo={lo}, hi={hi})"
                );
            }
        }
    }
}
