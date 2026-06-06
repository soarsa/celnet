//! Parity row: **FRTB-SA Standardised-Approach capital reproduces an independent
//! longhand oracle** (BCBS MAR21 SbM, MAR23 RRAO, MAR22 DRC).
//!
//! Wave 4d catalogue increment. `celnet-risk-cube::frtb` implements the full SbM
//! capital aggregation (within-bucket `K_b`, cross-bucket roll-up, the three-
//! correlation-scenario maximum, and curvature `K_b^+/K_b^-` selection) plus RRAO
//! and the honest FX DRC. This file gates that machinery against a **genuinely
//! independent** oracle — every formula is re-written **longhand** in the test, a
//! separate recomputation that never calls back into `frtb.rs`:
//!
//!  (i)   **worked-example longhand SbM (~1e-10).** On a fixed 2-bucket FX-delta
//!        portfolio the within-bucket `K_b`, the signed bucket sums `S_b`, the
//!        cross-bucket double sum, and the three-scenario maximum are each summed
//!        out by hand from the raw weighted sensitivities, with the HIGH/MEDIUM/LOW
//!        correlation transforms applied explicitly. Agreement to ~1e-10 proves the
//!        production aggregation against an independent algebraic recomputation.
//!
//!  (ii)  **three-scenario max & single-bucket reduction.** The reported capital is
//!        exactly `max(HIGH, MEDIUM, LOW)`, each scenario is internally consistent
//!        (HIGH ≥ MEDIUM ≥ LOW for a positively-correlated, same-signed book), and a
//!        single-bucket portfolio reduces to its `K_b`.
//!
//!  (iii) **structural identities.** A perfectly-hedged (equal-and-opposite) bucket
//!        gives `K_b = 0`; the cross-bucket charge is monotone in a bucket's
//!        sensitivity; RRAO equals the exact hand-summed `Σ |notional|·weight`; and
//!        DRC is the honest documented zero for deliverable FX.
//!
//! The oracle never imports a production formula: it is built from `f64` arithmetic
//! and `f64::sqrt` only, so the SbM capital is cross-validated against a wholly
//! independent recomputation rather than the code under test under another name.

use celnet_risk_cube::frtb::{
    CorrelationScenario, CurvatureBucket, ResidualInstrument, ResidualKind, RiskBucket, SbmParams,
    curvature_class, delta_vega_class, fx_default_risk_charge, residual_addon,
};

/// Longhand within-bucket `K_b = √( max(0, Σ WS² + ΣΣ_{k≠l} ρ WS_k WS_l) )` for a
/// single intra-bucket correlation `rho`, written out independently of `frtb.rs`.
fn longhand_k_b(ws: &[f64], rho: f64) -> f64 {
    let mut acc = 0.0;
    for k in 0..ws.len() {
        acc += ws[k] * ws[k];
        for l in 0..ws.len() {
            if k != l {
                acc += rho * ws[k] * ws[l];
            }
        }
    }
    acc.max(0.0).sqrt()
}

/// Longhand HIGH/MEDIUM/LOW transform of a base (MEDIUM) correlation (MAR21.6(2)),
/// re-derived here independently: HIGH `min(1.25ρ, 1)`, MEDIUM `ρ`, LOW
/// `max(2ρ−1, 0.75ρ)` — the prescribed low-correlation scenario includes the
/// `0.75ρ` floor, it is **not** a plain `max(2ρ−1, 0)`.
fn longhand_scale(scenario: CorrelationScenario, rho: f64) -> f64 {
    match scenario {
        CorrelationScenario::High => (1.25 * rho).min(1.0),
        CorrelationScenario::Medium => rho,
        CorrelationScenario::Low => (2.0 * rho - 1.0).max(0.75 * rho),
    }
}

/// The three-scenario correlation transform pinned directly to BCBS MAR21.6(2)
/// hand-computed values — an oracle for the transform itself, independent of any
/// aggregation. This is the gate that would have caught a missing `0.75ρ` floor
/// in the LOW scenario (e.g. the FX cross-bucket `γ = 0.6` ⇒ `0.45`, not `0.2`).
#[test]
fn correlation_scenario_transform_matches_basel_constants() {
    use celnet_core::is_close;
    // (ρ, high, medium, low) hand-computed from MAR21.6(2):
    //   high = min(1.25ρ, 1), low = max(2ρ−1, 0.75ρ).
    let cases = [
        (0.60_f64, 0.75_f64, 0.60_f64, 0.45_f64), // FX γ: low = 0.75·0.6 = 0.45
        (0.50, 0.625, 0.50, 0.375),               // low = 0.75·0.5 = 0.375 (not 0)
        (0.90, 1.0, 0.90, 0.80),                  // high capped at 1; low = 2·0.9−1 = 0.8
        (0.40, 0.50, 0.40, 0.30),                 // low = 0.75·0.4 = 0.30 (not 0)
        (1.0, 1.0, 1.0, 1.0),                     // perfectly correlated stays 1 in all
    ];
    for (rho, hi, mid, lo) in cases {
        assert!(
            is_close(CorrelationScenario::High.scale(rho), hi, 1e-12, 1e-12),
            "HIGH({rho}) = {} != {hi}",
            CorrelationScenario::High.scale(rho)
        );
        assert!(is_close(
            CorrelationScenario::Medium.scale(rho),
            mid,
            1e-12,
            1e-12
        ));
        assert!(
            is_close(CorrelationScenario::Low.scale(rho), lo, 1e-12, 1e-12),
            "LOW({rho}) = {} != {lo} (the 0.75ρ floor)",
            CorrelationScenario::Low.scale(rho)
        );
    }
}

/// Longhand full delta/vega class charge for a 2-bucket portfolio under one
/// scenario, with the MAR21.6 alternative `S_b` floor when the radicand is negative
/// — a complete independent recomputation.
fn longhand_two_bucket(
    ws0: &[f64],
    rho0: f64,
    ws1: &[f64],
    rho1: f64,
    gamma: f64,
    scenario: CorrelationScenario,
) -> f64 {
    let rho0s = longhand_scale(scenario, rho0);
    let rho1s = longhand_scale(scenario, rho1);
    let g = longhand_scale(scenario, gamma);
    let kb0 = longhand_k_b(ws0, rho0s);
    let kb1 = longhand_k_b(ws1, rho1s);
    let s0: f64 = ws0.iter().sum();
    let s1: f64 = ws1.iter().sum();
    let sum_k2 = kb0 * kb0 + kb1 * kb1;
    // Cross term Σ_b Σ_{c≠b} γ S_b S_c = 2·γ·S_0·S_1 (two buckets, symmetric).
    let cross = 2.0 * g * s0 * s1;
    let radicand = sum_k2 + cross;
    if radicand >= 0.0 {
        radicand.sqrt()
    } else {
        // MAR21.6 alternative: floor each S_b into [−K_b, K_b].
        let s0a = s0.min(kb0).max(-kb0);
        let s1a = s1.min(kb1).max(-kb1);
        (sum_k2 + 2.0 * g * s0a * s1a).max(0.0).sqrt()
    }
}

/// **(i) Worked-example longhand oracle.** A fixed 2-bucket FX-delta portfolio: the
/// production `delta_vega_class` capital equals the longhand three-scenario maximum
/// recomputed independently, to ~1e-10, scenario-by-scenario and at the max.
#[test]
fn sbm_two_bucket_matches_longhand_oracle() {
    // Bucket 0: three FX-delta risk factors (already RW-weighted sensitivities).
    let ws0 = [12.0_f64, -4.0, 7.0];
    let rho0 = 0.50;
    // Bucket 1: two factors.
    let ws1 = [-9.0_f64, 3.0];
    let rho1 = 0.50;
    // Inter-bucket correlation γ (MEDIUM).
    let gamma_med = 0.60;

    let params = SbmParams::new(
        vec![
            RiskBucket::new(10, ws0.to_vec(), rho0),
            RiskBucket::new(20, ws1.to_vec(), rho1),
        ],
        move |_, _| gamma_med,
    );
    let charge = delta_vega_class(&params);

    for sc in CorrelationScenario::ALL {
        let want = longhand_two_bucket(&ws0, rho0, &ws1, rho1, gamma_med, sc);
        let got = charge.under(sc);
        assert!(
            (got - want).abs() <= 1e-10 * (1.0 + want.abs()),
            "{sc:?}: production {got} vs longhand {want}"
        );
    }

    let want_max = CorrelationScenario::ALL
        .iter()
        .map(|&sc| longhand_two_bucket(&ws0, rho0, &ws1, rho1, gamma_med, sc))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (charge.capital() - want_max).abs() <= 1e-10 * (1.0 + want_max.abs()),
        "capital {} vs longhand max {want_max}",
        charge.capital()
    );
}

/// **(ii) The reported capital is exactly `max(HIGH, MEDIUM, LOW)`, and each scenario
/// is internally consistent.** For a positively-correlated, same-signed two-bucket
/// book (all `S_b` same sign, γ>0) the cross term grows with correlation, so
/// HIGH ≥ MEDIUM ≥ LOW and the capital is the HIGH scenario.
#[test]
fn sbm_capital_is_three_scenario_max() {
    let ws0 = [8.0_f64, 5.0]; // both positive
    let ws1 = [6.0_f64, 2.0]; // both positive → S_0, S_1 same sign
    let params = SbmParams::new(
        vec![
            RiskBucket::new(10, ws0.to_vec(), 0.40),
            RiskBucket::new(20, ws1.to_vec(), 0.40),
        ],
        |_, _| 0.40,
    );
    let charge = delta_vega_class(&params);
    // Monotone in the (positive) correlation when all signs align.
    assert!(charge.high >= charge.medium - 1e-12);
    assert!(charge.medium >= charge.low - 1e-12);
    // Capital is the maximum, and (here) the HIGH scenario.
    let max = charge.high.max(charge.medium).max(charge.low);
    assert!((charge.capital() - max).abs() <= 1e-12);
    assert!((charge.capital() - charge.high).abs() <= 1e-12);
}

/// **(ii) A single-bucket portfolio reduces to its `K_b`.** With one bucket there is
/// no cross-bucket term, so the class charge equals the longhand `K_b` in every
/// scenario.
#[test]
fn sbm_single_bucket_reduces_to_k_b() {
    let ws = [10.0_f64, -3.0, 4.0];
    let rho = 0.55;
    let params = SbmParams::new(vec![RiskBucket::new(10, ws.to_vec(), rho)], |_, _| 0.0);
    let charge = delta_vega_class(&params);
    for sc in CorrelationScenario::ALL {
        let want = longhand_k_b(&ws, longhand_scale(sc, rho));
        assert!(
            (charge.under(sc) - want).abs() <= 1e-10 * (1.0 + want.abs()),
            "{sc:?}: {} vs {want}",
            charge.under(sc)
        );
    }
}

/// **(iii) Perfectly-hedged bucket → `K_b = 0` for delta.** Equal-and-opposite
/// weighted sensitivities at perfect (`ρ=1`) intra-bucket correlation give a zero
/// within-bucket charge (the radicand `Σ WS² + 2ρ·WS₀WS₁ = a² + a² − 2a² = 0`).
#[test]
fn perfectly_hedged_bucket_is_zero() {
    let a = 7.5_f64;
    let params = SbmParams::new(vec![RiskBucket::new(10, vec![a, -a], 1.0)], |_, _| 0.0);
    let charge = delta_vega_class(&params);
    // MEDIUM uses ρ=1 → exactly zero; the longhand confirms.
    assert_eq!(charge.medium, 0.0);
    assert!((charge.medium - longhand_k_b(&[a, -a], 1.0)).abs() <= 1e-12);
}

/// **(iii) Monotonicity.** Increasing a bucket's net sensitivity (with everything
/// else fixed, same-signed, positive γ) never decreases the SbM capital — the
/// standard's same-direction monotonicity.
#[test]
fn sbm_capital_is_monotone_in_sensitivity() {
    let make = |w: f64| {
        SbmParams::new(
            vec![
                RiskBucket::new(10, vec![w, 2.0], 0.40),
                RiskBucket::new(20, vec![5.0, 1.0], 0.40),
            ],
            |_, _| 0.40,
        )
    };
    let small = delta_vega_class(&make(4.0)).capital();
    let big = delta_vega_class(&make(9.0)).capital();
    assert!(big >= small, "capital must be monotone: {big} vs {small}");
}

/// **(iii) Curvature: long-gamma → zero, short-gamma → positive, both scenarios
/// consistent.** A single curvature bucket whose up/down legs are both negative
/// (long gamma) charges zero; a bucket with a positive leg charges that leg.
#[test]
fn curvature_class_charges_short_gamma() {
    // Long gamma: both CVR legs negative → K_b = 0 → class charge 0.
    let long_gamma = [CurvatureBucket {
        id: 1,
        cvr_up: -3.0,
        cvr_down: -2.0,
        rho_intra: 0.0,
    }];
    let lg = curvature_class(&long_gamma, |_, _| 0.25);
    assert_eq!(lg.capital(), 0.0);

    // Short gamma single bucket: K_b = max(CVR^+,CVR^-,0); no cross term.
    let short_gamma = [CurvatureBucket {
        id: 1,
        cvr_up: 4.0,
        cvr_down: 1.5,
        rho_intra: 0.0,
    }];
    let sg = curvature_class(&short_gamma, |_, _| 0.25);
    // Single-bucket curvature: capital = max(CVR^+, CVR^-, 0) = 4.0 in every scenario.
    assert!((sg.capital() - 4.0).abs() <= 1e-12);
    for sc in CorrelationScenario::ALL {
        assert!((sg.under(sc) - 4.0).abs() <= 1e-12);
    }
}

/// **(iii) RRAO is the exact hand-summed notional × weight** (MAR23): 1.0% on
/// exotic-underlying gross notional, 0.1% on other-residual gross notional, 0 on
/// vanillas. Gross (magnitude) notional, so a short barrier still contributes.
#[test]
fn rrao_equals_hand_summed_notional_weight() {
    let book = [
        ResidualInstrument {
            notional: 25_000_000.0,
            kind: ResidualKind::OtherResidual, // barrier
        },
        ResidualInstrument {
            notional: -10_000_000.0,
            kind: ResidualKind::OtherResidual, // short digital (gross)
        },
        ResidualInstrument {
            notional: 4_000_000.0,
            kind: ResidualKind::ExoticUnderlying,
        },
        ResidualInstrument {
            notional: 50_000_000.0,
            kind: ResidualKind::None, // plain vanilla → no RRAO
        },
    ];
    // Longhand: 0.1%·(25M+10M) + 1.0%·4M + 0.
    let want = 0.001 * (25_000_000.0 + 10_000_000.0) + 0.01 * 4_000_000.0;
    let got = residual_addon(&book);
    assert!(
        (got - want).abs() <= 1e-6 * (1.0 + want.abs()),
        "RRAO {got} vs hand-summed {want}"
    );
}

/// **(iii) DRC for deliverable FX is the honest documented zero** (MAR22). A pure
/// deliverable-FX book references sovereign currencies, not a defaultable issuer
/// security, so its issuer jump-to-default — and hence the market-risk DRC — is
/// identically zero (settlement/counterparty risk is the CCR/CVA framework, not
/// market-risk DRC). This is a correct, cited zero, not a fabricated number.
#[test]
fn fx_drc_is_documented_zero() {
    assert_eq!(fx_default_risk_charge(0), 0.0);
    assert_eq!(fx_default_risk_charge(1), 0.0);
    assert_eq!(fx_default_risk_charge(500), 0.0);
}
