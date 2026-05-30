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
