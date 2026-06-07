//! Parity row — XVA (CVA/DVA/FVA) on synthetic netting sets.
//!
//! Validates `celnet-xva` against a **hand-derived closed-form CVA** in exact
//! limits. The oracle is coded here independently of the engine (it does not call
//! the engine's aggregation): it recomputes the discrete CVA sum from first
//! principles with `celnet_core::math::exp`, and — to guard against the engine and oracle
//! sharing a mis-stated constant (Lesson c: a prior wave shipped an FRTB bug
//! because a longhand oracle re-derived the same wrong constant) — it ALSO pins
//! the hazard/survival/LGD constants and a fully hand-evaluated discrete CVA to
//! literal published numeric values computed offline.
//!
//! Rows:
//!  1. CVA == hand-derived closed form on a deterministic single-factor exposure
//!     profile to ~1e-9, with every intermediate constant hand-pinned.
//!  2. CVA monotone strictly increasing in the hazard λ and in the LGD.
//!  3. CVA == 0 at zero default probability (λ → 0).
//!  4. DVA symmetry — DVA on a profile equals CVA on the role-/sign-flipped
//!     profile; and FVA sign tracks the net expected exposure.
//!  5. The MC/QMC exposure SIMULATION (not just the deterministic feed) is real:
//!     at zero diffusion vol the simulated EPE equals an independently hand-built
//!     deterministic profile, so the whole simulate→aggregate pipeline matches a
//!     hand-checkable closed form.
//!
//! Honest boundary: synthetic netting sets only — no live CSA/collateral/
//! wrong-way risk (those are deploy/estate-gated).

use celnet_types::OptionType;
use celnet_xva::{
    ExposureConfig, ExposureProfile, NettedTrade, NettingSet, SurvivalCurve, XvaInputs, compute_xva,
};

/// Independent in-test oracle: the discrete unilateral CVA
/// `LGD · Σ_k D(t_k)·EPE(t_k)·[S(t_{k-1}) − S(t_k)]` for a FLAT hazard `lambda`
/// and FLAT discount rate `r`. Coded from scratch (does not call the engine).
fn closed_form_cva(grid: &[f64], epe: &[f64], lambda: f64, r: f64, lgd: f64) -> f64 {
    let survival = |t: f64| celnet_core::math::exp(-lambda * t);
    let discount = |t: f64| celnet_core::math::exp(-r * t);
    let mut cva = 0.0;
    for k in 1..grid.len() {
        let dp = survival(grid[k - 1]) - survival(grid[k]);
        cva += lgd * discount(grid[k]) * epe[k] * dp;
    }
    cva
}

#[test]
fn row1_cva_matches_hand_derived_closed_form() {
    // ----- HAND-PINNED constants (computed offline, see derivation below) -----
    // Flat hazard λ = 0.02, flat discount r = 0.03, LGD = 1 − R = 0.6 (R = 0.4,
    // the ISDA standard senior-unsecured recovery). Deterministic EPE profile on a
    // 1y grid out to 3y: EPE = {0, 10, 8, 5}, ENE = 0.
    const LAMBDA: f64 = 0.02;
    const R: f64 = 0.03;
    const LGD: f64 = 0.6;
    let grid = [0.0, 1.0, 2.0, 3.0];
    let epe = [0.0, 10.0, 8.0, 5.0];
    let ene = [0.0, 0.0, 0.0, 0.0];

    // Hand-pinned marginal default probabilities S(t_{k-1}) − S(t_k):
    //   S(0)=1, S(1)=e^{-0.02}=0.980198673306755253,
    //   S(2)=e^{-0.04}=0.960789439152323177, S(3)=e^{-0.06}=0.941764533584248720.
    //   (0,1]: 1            − 0.980198673306755253 = 0.019801326693244747
    //   (1,2]: 0.980198673… − 0.960789439152323177 = 0.019409234154432076
    //   (2,3]: 0.960789439… − 0.941764533584248720 = 0.019024905568074457
    let dp_pinned = [
        0.019_801_326_693_244_747,
        0.019_409_234_154_432_076,
        0.019_024_905_568_074_457,
    ];
    // Confirm the engine's survival mechanics reproduce the hand-pinned increments
    // (pins the constant itself, not just the downstream sum).
    let curve = SurvivalCurve::flat(LAMBDA);
    for (k, &dp) in dp_pinned.iter().enumerate() {
        let a = grid[k];
        let b = grid[k + 1];
        assert!(
            (curve.marginal_default(a, b) - dp).abs() < 1e-15,
            "marginal default ({a},{b}] mismatch vs hand-pinned value"
        );
    }

    // Hand-pinned fully evaluated discrete CVA (computed offline):
    //   D(1)=e^{-0.03}=0.970445533548508155, D(2)=e^{-0.06}=0.941764533584248720,
    //   D(3)=e^{-0.09}=0.913931185271228186.
    //   CVA = 0.6·[ D(1)·10·dp1 + D(2)·8·dp2 + D(3)·5·dp3 ] = 0.255197873856521484
    //   (nearest f64: 0.2551978738565215)
    const CVA_PINNED: f64 = 0.255_197_873_856_521_5;

    // The in-test closed-form oracle (independent of the engine) must hit the pin.
    let oracle = closed_form_cva(&grid, &epe, LAMBDA, R, LGD);
    assert!(
        (oracle - CVA_PINNED).abs() < 1e-12,
        "in-test oracle {oracle} disagrees with hand-pinned CVA {CVA_PINNED}"
    );

    // The ENGINE on a deterministic profile must match the oracle to ~1e-9 and the
    // hand-pinned literal.
    let profile = ExposureProfile::deterministic(grid.to_vec(), epe.to_vec(), ene.to_vec(), R);
    let alive = SurvivalCurve::flat(0.0);
    let res = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &curve,
        own: &alive,
        lgd_counterparty: LGD,
        lgd_own: LGD,
        funding_spread: 0.0,
    });
    assert!(
        (res.cva - oracle).abs() < 1e-9,
        "engine CVA {} vs oracle {oracle}",
        res.cva
    );
    assert!(
        (res.cva - CVA_PINNED).abs() < 1e-9,
        "engine CVA {} vs hand-pinned {CVA_PINNED}",
        res.cva
    );
    // ENE = 0 ⇒ DVA = 0; funding spread = 0 ⇒ FVA = 0.
    assert!(res.dva.abs() < 1e-15);
    assert!(res.fva.abs() < 1e-15);
}

#[test]
fn row2_cva_monotone_in_hazard_and_lgd() {
    let grid = vec![0.0, 1.0, 2.0, 3.0];
    let epe = vec![0.0, 6.0, 5.0, 3.0];
    let ene = vec![0.0, 0.0, 0.0, 0.0];
    let profile = ExposureProfile::deterministic(grid, epe, ene, 0.025);
    let alive = SurvivalCurve::flat(0.0);

    let cva = |lambda: f64, lgd: f64| {
        compute_xva(&XvaInputs {
            profile: &profile,
            counterparty: &SurvivalCurve::flat(lambda),
            own: &alive,
            lgd_counterparty: lgd,
            lgd_own: 0.6,
            funding_spread: 0.0,
        })
        .cva
    };

    // Strictly increasing in hazard at fixed LGD.
    let mut prev = cva(0.0, 0.6);
    assert!(prev.abs() < 1e-15);
    for &lam in &[0.01, 0.02, 0.05, 0.10, 0.20] {
        let v = cva(lam, 0.6);
        assert!(v > prev, "CVA not increasing at λ={lam}: {v} ≤ {prev}");
        prev = v;
    }
    // Strictly increasing in LGD at fixed hazard, and exactly linear in LGD.
    let a = cva(0.05, 0.3);
    let b = cva(0.05, 0.6);
    assert!(b > a, "CVA not increasing in LGD");
    assert!((b - 2.0 * a).abs() < 1e-12, "CVA must be linear in LGD");
}

#[test]
fn row3_zero_default_prob_gives_zero_cva() {
    let grid = vec![0.0, 0.5, 1.0, 2.0];
    let epe = vec![0.0, 4.0, 7.0, 9.0];
    let ene = vec![0.0, 0.0, 0.0, 0.0];
    let profile = ExposureProfile::deterministic(grid, epe, ene, 0.03);
    let alive = SurvivalCurve::flat(0.0); // λ = 0 ⇒ never defaults

    let res = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &alive,
        own: &alive,
        lgd_counterparty: 0.6,
        lgd_own: 0.6,
        funding_spread: 0.0,
    });
    assert!(
        res.cva.abs() < 1e-15,
        "CVA must vanish at zero default prob"
    );
    assert!(res.dva.abs() < 1e-15);

    // And the limit is approached continuously: CVA(λ) → 0 as λ → 0⁺.
    let small = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &SurvivalCurve::flat(1e-8),
        own: &alive,
        lgd_counterparty: 0.6,
        lgd_own: 0.6,
        funding_spread: 0.0,
    });
    assert!(small.cva > 0.0 && small.cva < 1e-6);
}

#[test]
fn row4_dva_symmetry_and_fva_sign() {
    // DVA is structurally CVA with (EPE↔ENE) and (counterparty↔own) swapped. Build
    // a profile with non-trivial EPE AND ENE; check DVA on it equals the CVA the
    // engine produces on the EPE↔ENE-swapped profile with curves swapped.
    let grid = vec![0.0, 1.0, 2.0, 3.0];
    let epe = vec![0.0, 6.0, 5.0, 3.0];
    let ene = vec![0.0, 2.0, 4.0, 1.0];
    let r = 0.02;
    let profile = ExposureProfile::deterministic(grid.clone(), epe.clone(), ene.clone(), r);
    // Sign-swapped profile (EPE ↔ ENE) for the symmetric check.
    let swapped = ExposureProfile::deterministic(grid.clone(), ene.clone(), epe.clone(), r);

    let cpty = SurvivalCurve::flat(0.04);
    let own = SurvivalCurve::flat(0.03);

    let res = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &cpty,
        own: &own,
        lgd_counterparty: 0.55,
        lgd_own: 0.45,
        funding_spread: 0.0,
    });
    // CVA on the swapped profile with curves+LGDs swapped must equal the original
    // DVA (the two adjustments are mirror images under role exchange).
    let mirror = compute_xva(&XvaInputs {
        profile: &swapped,
        counterparty: &own,
        own: &cpty,
        lgd_counterparty: 0.45,
        lgd_own: 0.55,
        funding_spread: 0.0,
    });
    assert!(
        (res.dva - mirror.cva).abs() < 1e-12,
        "DVA {} not mirror of swapped CVA {}",
        res.dva,
        mirror.cva
    );

    // FVA sign: a net-asset profile (EPE dominates ENE) is a funding COST (> 0).
    let cost = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &cpty,
        own: &own,
        lgd_counterparty: 0.55,
        lgd_own: 0.45,
        funding_spread: 0.015,
    });
    assert!(cost.fva > 0.0, "net-asset profile must give FVA cost > 0");
    // A net-liability profile (ENE dominates) is a funding BENEFIT (< 0).
    let benefit = compute_xva(&XvaInputs {
        profile: &swapped,
        counterparty: &cpty,
        own: &own,
        lgd_counterparty: 0.55,
        lgd_own: 0.45,
        funding_spread: 0.015,
    });
    assert!(
        benefit.fva < 0.0,
        "net-liability profile must give FVA benefit < 0"
    );
}

#[test]
fn row5_zero_vol_simulation_matches_independent_deterministic_profile() {
    // At zero diffusion vol the simulated spot path is deterministic:
    //   S(t) = S0·exp((r_d − r_f)·t),
    // so the simulated EPE/ENE collapse to a profile we can build by hand. This
    // proves the FULL simulate→aggregate pipeline (not just the deterministic
    // constructor) reproduces a hand-checkable closed form.
    let r_dom = 0.03;
    let r_for = 0.01;
    let spot0 = 1.10;
    // A single long call: net value ≥ 0 ⇒ EPE = net value, ENE = 0.
    let trade = NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.12, 1.0);
    let set = NettingSet::new(vec![trade], r_dom, r_for);

    let steps = 8;
    let horizon = set.horizon();
    let dt = horizon / steps as f64;

    // Independently build the deterministic profile the simulation MUST yield.
    let mut grid = Vec::with_capacity(steps + 1);
    let mut epe = Vec::with_capacity(steps + 1);
    let mut ene = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let t = k as f64 * dt;
        // Deterministic zero-vol spot.
        let spot = spot0 * celnet_core::math::exp((r_dom - r_for) * t);
        let v = set.net_value(t, spot);
        grid.push(t);
        epe.push(v.max(0.0));
        ene.push((-v).max(0.0));
    }
    let hand = ExposureProfile::deterministic(grid, epe, ene, r_dom);

    let cfg = ExposureConfig {
        spot0,
        sigma: 0.0, // zero diffusion ⇒ deterministic path
        paths: 64,  // every path identical at σ = 0
        seed: 12345,
    };
    let sim = ExposureProfile::simulate(&set, &cfg, steps);

    for (k, (&a, &b)) in sim.epe().iter().zip(hand.epe()).enumerate() {
        assert!(
            (a - b).abs() < 1e-12,
            "EPE mismatch at k={k}: sim {a} vs hand {b}"
        );
    }
    for (&a, &b) in sim.ene().iter().zip(hand.ene()) {
        assert!(a.abs() < 1e-12 && b.abs() < 1e-12);
    }

    // And the CVA off the simulated profile equals the CVA off the independent
    // hand-built profile (same hazard/LGD), closing the loop.
    let cpty = SurvivalCurve::flat(0.03);
    let alive = SurvivalCurve::flat(0.0);
    let mk = |p: &ExposureProfile| {
        compute_xva(&XvaInputs {
            profile: p,
            counterparty: &cpty,
            own: &alive,
            lgd_counterparty: 0.6,
            lgd_own: 0.6,
            funding_spread: 0.0,
        })
        .cva
    };
    assert!((mk(&sim) - mk(&hand)).abs() < 1e-12);
}
