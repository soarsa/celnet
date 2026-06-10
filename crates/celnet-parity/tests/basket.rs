//! Parity row — **correlated multi-asset FX options** (weighted basket /
//! best-of-N / worst-of-N), priced by the Cholesky-correlated multi-asset GBM
//! Monte-Carlo engine in `celnet-exotics` ([`price_basket`]), each gated against
//! a genuinely independent oracle.
//!
//! The engine is Monte-Carlo, so every gate is an honest statistical / structural
//! / approximation band — never a closed-form-precision claim:
//!
//!   (a) **degenerate basket == single-asset vanilla** — a one-leg, weight-1
//!       basket call/put collapses to a plain Garman-Kohlhagen vanilla; the MC
//!       price matches [`celnet_vanilla::price`] within a few reported MC
//!       standard errors. This reuses the production GK closed form (already
//!       golden-gated against QuantLib) as the oracle.
//!
//!   (b) **structural sandwich** — for any correlation, on identical legs (so the
//!       "single-asset value" is well-defined), `worst-of ≤ each single ≤
//!       best-of`, and the weighted basket sits between worst-of and best-of.
//!       Proven on common per-path inputs (the same MC config), so the ordering
//!       is a genuine path-wise inequality, not a noise artefact.
//!
//!   (c) **comonotonic limit** — as `ρ → 1` the legs move together, so best-of
//!       and worst-of of the (equal-weight, identical-leg) rainbow approach the
//!       common single-asset option value (max/min of perfectly-correlated
//!       identical legs is the leg itself). Gated to a tight band at `ρ = 1`.
//!
//!   (d) **hand-pinned 2-asset reference** — an *independent* Levy (1992)
//!       lognormal moment-matched basket approximation, computed BY HAND in this
//!       test (matching the basket-forward's first two moments to a single
//!       lognormal and pricing by Black-on-the-forward), and a value pinned from
//!       that derivation. The MC price is gated within the documented Levy
//!       approximation band (a moment-matching approximation, NOT an MC-precision
//!       claim). The pinned constant guards against a silent drift in either the
//!       engine or the in-test oracle.
//!
//!   (e) **non-PSD correlation → domain error** — an over-correlated (`ρ > 1`)
//!       matrix is rejected with [`CorrelationError::NotPositiveDefinite`] rather
//!       than silently regularised.
//!
//! Method provenance (Cholesky decomposition; Levy 1992 moment matching) lives in
//! the `celnet-exotics::multiasset` module docs; identifiers here are
//! purpose-named.

use celnet_core::math::{exp, ln, sqrt};
use celnet_exotics::{
    BasketKind, BasketLeg, BasketMcConfig, BasketSpec, CorrelationError, price_basket,
};
use celnet_types::{Carry, OptionType, VanillaInputs};

/// The settlement-cash numeraire carry at rate `r` (forward 1 ⇒ b = 0).
fn numeraire(r: f64) -> Carry {
    Carry::CostOfCarry { r, b: 0.0 }
}

/// A shared, well-converged MC config for the limit / pinned-reference gates.
fn cfg(seed: u64) -> BasketMcConfig {
    BasketMcConfig {
        budget: 65_536,
        replications: 32,
        steps: 1,
        seed,
    }
}

/// A lighter config for the structural-inequality gate (the ordering holds at
/// modest precision; the tolerances absorb each estimate's reported stderr).
fn cfg_light(seed: u64) -> BasketMcConfig {
    BasketMcConfig {
        budget: 8_192,
        replications: 16,
        steps: 1,
        seed,
    }
}

/// Standard normal CDF via the error function (independent of the engine's
/// inverse-CDF path).
fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + libm::erf(x / core::f64::consts::SQRT_2))
}

// ---------------------------------------------------------------------------
// (a) degenerate basket == single-asset Garman-Kohlhagen vanilla
// ---------------------------------------------------------------------------

#[test]
fn degenerate_one_leg_basket_matches_gk_vanilla() {
    let spot = 1.10;
    let vol = 0.12;
    let r_dom = 0.025;
    let r_for = 0.01;
    let t = 0.75;
    for (kind_opt, strike) in [
        (OptionType::Call, 1.08),
        (OptionType::Call, 1.15),
        (OptionType::Put, 1.08),
        (OptionType::Put, 1.15),
    ] {
        let spec = BasketSpec {
            legs: vec![BasketLeg::new(spot, vol, r_dom - r_for, 1.0)],
            correlation: vec![vec![1.0]],
            option_type: kind_opt,
            strike,
            kind: BasketKind::Basket,
        };
        let est = price_basket(&spec, numeraire(r_dom), t, cfg(0xA11CE)).unwrap();

        // Oracle: the production GK closed form (golden-gated vs QuantLib).
        let gk = celnet_vanilla::price(
            kind_opt,
            &VanillaInputs::new(spot, strike, vol, t, r_dom, r_for),
        );

        let tol = 4.0 * est.std_error + 1e-9;
        assert!(
            (est.price - gk).abs() <= tol,
            "one-leg basket {kind_opt:?} K={strike}: MC {} vs GK {} (tol {})",
            est.price,
            gk,
            tol
        );
    }
}

// ---------------------------------------------------------------------------
// (b) structural sandwich: worst-of ≤ single ≤ best-of, basket between
// ---------------------------------------------------------------------------

#[test]
fn structural_sandwich_worst_le_single_le_best_basket_between() {
    // Two identical legs (same law). For best/worst/single the legs carry UNIT
    // weight, so the rainbow aggregates max/min(S1, S2) and the single is S_i —
    // all on the same scale, with worst ≤ single ≤ best for any correlation. The
    // weighted BASKET carries weights summing to 1 (0.5 each), so its aggregate
    // (S1 + S2)/2 is a convex combination of the legs and therefore lies between
    // min(S1, S2) and max(S1, S2) path-wise ⇒ worst ≤ basket ≤ best.
    let r_dom = 0.02;
    let unit_leg = BasketLeg::new(1.0, 0.16, r_dom - 0.01, 1.0);
    let half_leg = BasketLeg::new(1.0, 0.16, r_dom - 0.01, 0.5);
    let t = 1.0;
    let strike = 1.0;

    for &rho in &[-0.3, 0.0, 0.5, 0.9] {
        let corr = vec![vec![1.0, rho], vec![rho, 1.0]];
        let mk = |legs: Vec<BasketLeg>, kind| BasketSpec {
            legs,
            correlation: corr.clone(),
            option_type: OptionType::Call,
            strike,
            kind,
        };
        let best = price_basket(
            &mk(vec![unit_leg, unit_leg], BasketKind::BestOf),
            numeraire(r_dom),
            t,
            cfg_light(0xB00),
        )
        .unwrap();
        let worst = price_basket(
            &mk(vec![unit_leg, unit_leg], BasketKind::WorstOf),
            numeraire(r_dom),
            t,
            cfg_light(0xB00),
        )
        .unwrap();
        let basket = price_basket(
            &mk(vec![half_leg, half_leg], BasketKind::Basket),
            numeraire(r_dom),
            t,
            cfg_light(0xB00),
        )
        .unwrap();

        // The single-asset value: one UNIT leg (the exact single-underlying GK).
        let single = price_basket(
            &BasketSpec {
                legs: vec![unit_leg],
                correlation: vec![vec![1.0]],
                option_type: OptionType::Call,
                strike,
                kind: BasketKind::Basket,
            },
            numeraire(r_dom),
            t,
            cfg_light(0xB00),
        )
        .unwrap();

        // Tolerances absorb the independent MC noise of each estimate.
        let se = best.std_error + worst.std_error + basket.std_error + single.std_error;
        let tol = 4.0 * se + 1e-9;
        assert!(
            worst.price <= single.price + tol,
            "rho={rho}: worst {} > single {}",
            worst.price,
            single.price
        );
        assert!(
            single.price <= best.price + tol,
            "rho={rho}: single {} > best {}",
            single.price,
            best.price
        );
        assert!(
            worst.price <= basket.price + tol && basket.price <= best.price + tol,
            "rho={rho}: basket {} not between worst {} and best {}",
            basket.price,
            worst.price,
            best.price
        );
    }
}

// ---------------------------------------------------------------------------
// (c) comonotonic limit: ρ → 1 ⇒ best-of, worst-of → the single value
// ---------------------------------------------------------------------------

#[test]
fn comonotonic_limit_best_and_worst_collapse_to_single() {
    // Identical legs, ρ → 1: the two underlyings become the same random variable,
    // so max → min → the single leg's value. ρ = exactly 1 is a singular
    // (PSD-but-not-PD) matrix, correctly rejected by the Cholesky factor, so the
    // limit is taken at ρ = 1 − ε.
    let r_dom = 0.02;
    let leg = BasketLeg::new(1.0, 0.15, r_dom - 0.012, 1.0);
    let legs = vec![leg, leg];
    let t = 1.0;
    let strike = 1.0;
    let rho = 1.0 - 1e-6;
    let corr = vec![vec![1.0, rho], vec![rho, 1.0]];
    let mk = |kind| BasketSpec {
        legs: legs.clone(),
        correlation: corr.clone(),
        option_type: OptionType::Call,
        strike,
        kind,
    };
    let best = price_basket(&mk(BasketKind::BestOf), numeraire(r_dom), t, cfg(0xC0)).unwrap();
    let worst = price_basket(&mk(BasketKind::WorstOf), numeraire(r_dom), t, cfg(0xC0)).unwrap();
    let single = price_basket(
        &BasketSpec {
            legs: vec![leg],
            correlation: vec![vec![1.0]],
            option_type: OptionType::Call,
            strike,
            kind: BasketKind::Basket,
        },
        numeraire(r_dom),
        t,
        cfg(0xC0),
    )
    .unwrap();

    // At ρ = 1 − 1e-6 the residual comonotonic gap is tiny but non-zero; the band
    // is the combined MC noise plus a small allowance for the 1e-6 decorrelation.
    let tol = 4.0 * (best.std_error + worst.std_error + single.std_error) + 5e-4;
    assert!(
        (best.price - single.price).abs() <= tol,
        "comonotonic best {} vs single {} (tol {})",
        best.price,
        single.price,
        tol
    );
    assert!(
        (worst.price - single.price).abs() <= tol,
        "comonotonic worst {} vs single {} (tol {})",
        worst.price,
        single.price,
        tol
    );
}

// ---------------------------------------------------------------------------
// (d) hand-pinned 2-asset reference — independent Levy (1992) moment matching
// ---------------------------------------------------------------------------

/// Levy (1992) lognormal moment-matched fair value of a 2-asset weighted basket
/// CALL, computed independently here (no call into `celnet-exotics`): match the
/// basket-forward's first two moments to a single lognormal and price by Black on
/// that forward, discounted at `r_dom`. A moment-matching approximation, accurate
/// to ~0.1–1% for the low-vol FX regime here.
fn levy_basket_call(
    legs: &[(f64, f64, f64, f64)], // (weight, spot, vol, r_for)
    correlation: &[Vec<f64>],
    r_dom: f64,
    t: f64,
    strike: f64,
) -> f64 {
    let n = legs.len();
    // Weighted forwards F_i = w_i · S_i · e^{(r_d − r_f,i) t}.
    let fwd: Vec<f64> = legs
        .iter()
        .map(|&(w, s, _v, rf)| w * s * exp((r_dom - rf) * t))
        .collect();
    let vols: Vec<f64> = legs.iter().map(|&(_w, _s, v, _rf)| v).collect();
    let m1: f64 = fwd.iter().sum();
    let mut m2 = 0.0;
    for i in 0..n {
        for j in 0..n {
            m2 += fwd[i] * fwd[j] * exp(correlation[i][j] * vols[i] * vols[j] * t);
        }
    }
    let fb = m1;
    let sig2t = ln(m2 / (m1 * m1));
    let std = sqrt(sig2t);
    let d1 = (ln(fb / strike) + 0.5 * sig2t) / std;
    let d2 = d1 - std;
    exp(-r_dom * t) * (fb * norm_cdf(d1) - strike * norm_cdf(d2))
}

#[test]
fn hand_pinned_two_asset_basket_within_levy_band() {
    // Legs: (weight, spot, vol, r_for).
    let legs = [(0.5, 1.10, 0.11, 0.015), (0.5, 1.27, 0.13, 0.020)];
    let rho = 0.4;
    let corr = vec![vec![1.0, rho], vec![rho, 1.0]];
    let r_dom = 0.02;
    let t = 1.0;
    let strike = 1.18;

    // The value pinned BY HAND from the Levy derivation (recomputed in-test by
    // `levy_basket_call`, and frozen as a constant so neither the engine nor the
    // oracle can silently drift):
    //   Fb = 1.1877568865, σ = 0.1015333960, Levy call = 0.0508837560.
    const PINNED_LEVY: f64 = 0.0508837560;

    // (i) the in-test Levy oracle reproduces the pinned constant.
    let levy = levy_basket_call(&legs, &corr, r_dom, t, strike);
    assert!(
        (levy - PINNED_LEVY).abs() < 1e-9,
        "in-test Levy {} drifted from the hand-pinned constant {}",
        levy,
        PINNED_LEVY
    );

    // (ii) the engine MC price is within the documented Levy approximation band.
    let spec = BasketSpec {
        legs: legs
            .iter()
            .map(|&(w, s, v, rf)| BasketLeg::new(s, v, r_dom - rf, w))
            .collect(),
        correlation: corr,
        option_type: OptionType::Call,
        strike,
        kind: BasketKind::Basket,
    };
    let est = price_basket(&spec, numeraire(r_dom), t, cfg(0xD00D)).unwrap();

    // Levy is a moment-matching APPROXIMATION (not an exact oracle): gate at a
    // band that comfortably covers its documented error for this regime while
    // still catching a gross engine error. Observed gap ≈ 1.3e-5 (≈ 0.025%); the
    // band is 0.5% relative plus the MC stderr.
    let band = 0.005 * PINNED_LEVY + 4.0 * est.std_error;
    assert!(
        (est.price - PINNED_LEVY).abs() <= band,
        "MC basket {} vs hand-pinned Levy {} (band {}, MC stderr {})",
        est.price,
        PINNED_LEVY,
        band,
        est.std_error
    );
}

// ---------------------------------------------------------------------------
// (e) non-PSD correlation is rejected honestly
// ---------------------------------------------------------------------------

#[test]
fn non_psd_correlation_is_a_domain_error() {
    let spec = BasketSpec {
        legs: vec![
            BasketLeg::new(1.10, 0.11, 0.02 - 0.015, 0.5),
            BasketLeg::new(1.27, 0.13, 0.02 - 0.020, 0.5),
        ],
        // ρ = 1.01 > 1 ⇒ indefinite ⇒ not a valid correlation matrix.
        correlation: vec![vec![1.0, 1.01], vec![1.01, 1.0]],
        option_type: OptionType::Call,
        strike: 1.18,
        kind: BasketKind::Basket,
    };
    match price_basket(&spec, numeraire(0.02), 1.0, cfg(0xE)) {
        Err(CorrelationError::NotPositiveDefinite) => {}
        other => panic!("expected NotPositiveDefinite, got {other:?}"),
    }
}
