//! Parity row — **multi-leg strategy** pricing reproduces genuinely independent,
//! model-free no-arbitrage oracles.
//!
//! A strategy (risk-reversal, straddle, butterfly, seagull, …) is priced as the
//! signed, ratio-weighted sum of its vanilla legs: `Σ side·ratio·GK(leg)`. The leg
//! pricer is the production [`celnet_vanilla::price`]; the composition is what the
//! server assembles from the wire `Strategy` message (the wire assembly is
//! conformance-tested against the frozen golden vector in
//! `celnet-client/tests/conformance.rs`). Here we validate the *pricing model* of
//! the family against oracles independent of the Black-Scholes/GK formula itself:
//!
//!   (i)   **Synthetic forward = put-call parity** (~1e-12, model-free): a long
//!         call + short put struck at the same `K` must equal the discounted
//!         forward `S e^{-r_f t} − K e^{-r_d t}`, regardless of the option model.
//!         Pins leg composition + sign handling to a no-arbitrage identity that
//!         shares no code with the pricer.
//!   (ii)  **ATM-forward symmetry** (~1e-12, model-free): at `K = F` the synthetic
//!         forward is zero, so the call and put legs are equal — the straddle is
//!         exactly twice either leg.
//!   (iii) **Butterfly convexity ≥ 0** (model-free): `C(K−Δ) − 2C(K) + C(K+Δ) ≥ 0`
//!         (convexity of the call price in strike) for any arbitrage-free prices.
//!   (iv)  **Independent GK leg-sum** (~1e-9): a full risk-reversal reproduces an
//!         independent, from-scratch GK leg-sum oracle (built here, not
//!         `celnet_vanilla`), cross-checking the production leg pricer.

use celnet_core::math::{exp, ln, norm_cdf, sqrt};
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::price as vanilla_price;

/// Independent from-scratch Garman-Kohlhagen oracle (no `celnet_vanilla`).
fn gk(opt: OptionType, spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> f64 {
    let srt = vol * sqrt(t);
    let d1 = (ln(spot / strike) + (r_dom - r_for + 0.5 * vol * vol) * t) / srt;
    let d2 = d1 - srt;
    match opt {
        OptionType::Call => {
            spot * exp(-r_for * t) * norm_cdf(d1) - strike * exp(-r_dom * t) * norm_cdf(d2)
        }
        OptionType::Put => {
            strike * exp(-r_dom * t) * norm_cdf(-d2) - spot * exp(-r_for * t) * norm_cdf(-d1)
        }
    }
}

/// Production single-leg price (the leaf the server composes into a strategy).
fn leg(opt: OptionType, spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> f64 {
    vanilla_price(opt, &VanillaInputs::new(spot, strike, vol, t, r_dom, r_for))
}

const SPOT: f64 = 1.10;
const VOL: f64 = 0.10;
const T: f64 = 1.0;
const R_DOM: f64 = 0.02;
const R_FOR: f64 = 0.015;

/// (i) long call(K) − short put(K) == discounted forward (model-free put-call parity).
#[test]
fn strategy_synthetic_forward_equals_put_call_parity() {
    let k = 1.12;
    let strat = leg(OptionType::Call, SPOT, k, VOL, T, R_DOM, R_FOR)
        - leg(OptionType::Put, SPOT, k, VOL, T, R_DOM, R_FOR);
    let parity = SPOT * exp(-R_FOR * T) - k * exp(-R_DOM * T);
    assert!(
        (strat - parity).abs() < 1e-12,
        "synthetic forward {strat} must equal put-call parity {parity}"
    );
}

/// (ii) At K=F the call and put legs are equal; the straddle is twice either leg.
#[test]
fn strategy_atm_forward_straddle_symmetry() {
    let fwd = SPOT * exp((R_DOM - R_FOR) * T);
    let call = leg(OptionType::Call, SPOT, fwd, VOL, T, R_DOM, R_FOR);
    let put = leg(OptionType::Put, SPOT, fwd, VOL, T, R_DOM, R_FOR);
    assert!(
        (call - put).abs() < 1e-12,
        "at K=F the call {call} and put {put} legs must be equal"
    );
    let straddle = call + put;
    assert!(
        (straddle - 2.0 * call).abs() < 1e-12,
        "straddle {straddle} must be exactly twice the leg"
    );
}

/// (iii) Call butterfly is convex (≥ 0) in strike — a model-free no-arbitrage oracle.
#[test]
fn strategy_butterfly_convexity_nonnegative() {
    let k = 1.10;
    let d = 0.05;
    let fly = leg(OptionType::Call, SPOT, k - d, VOL, T, R_DOM, R_FOR)
        - 2.0 * leg(OptionType::Call, SPOT, k, VOL, T, R_DOM, R_FOR)
        + leg(OptionType::Call, SPOT, k + d, VOL, T, R_DOM, R_FOR);
    assert!(
        fly >= -1e-12,
        "call butterfly {fly} must be convex (≥ 0) in strike"
    );
}

/// (iv) A risk-reversal's production leg composition reproduces an independent,
/// from-scratch GK leg-sum.
#[test]
fn strategy_risk_reversal_matches_independent_gk_leg_sum() {
    let (kc, kp) = (1.18, 1.02);
    let production = leg(OptionType::Call, SPOT, kc, VOL, T, R_DOM, R_FOR)
        - leg(OptionType::Put, SPOT, kp, VOL, T, R_DOM, R_FOR);
    let oracle = gk(OptionType::Call, SPOT, kc, VOL, T, R_DOM, R_FOR)
        - gk(OptionType::Put, SPOT, kp, VOL, T, R_DOM, R_FOR);
    assert!(
        (production - oracle).abs() < 1e-9,
        "risk-reversal production {production} vs independent GK {oracle}"
    );
}
