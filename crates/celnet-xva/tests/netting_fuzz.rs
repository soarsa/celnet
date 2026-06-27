//! Proptest mirror of `fuzz/fuzz_targets/xva_netting.rs`.
//!
//! Asserts the same netting/exposure/XVA contracts as the nightly libFuzzer
//! target, on the stable toolchain inside `just check`:
//!
//!   1. No panic on valid in-domain inputs;
//!   2. Profile invariants: grid[0]==0, EPE≥0, ENE≥0, discount correct, all finite;
//!   3. Determinism: simulate bit-identical on two calls with same draw;
//!   4. CVA≥0, DVA≥0, FVA finite; total_adjustment()==cva-dva+fva exactly;
//!   5. Doubling lambda_cpty does not decrease CVA.
//!
//! 128 cases; same draw ranges as the fuzz target.

use celnet_types::OptionType;
use celnet_xva::{
    ExposureConfig, ExposureProfile, NettedTrade, NettingSet, SurvivalCurve, XvaInputs, compute_xva,
};
use proptest::prelude::*;

fn clamp_into(raw: f64, lo: f64, hi: f64) -> f64 {
    let mid = 0.5 * (lo + hi);
    if !raw.is_finite() {
        return mid;
    }
    let t = 0.5 * (libm::tanh(raw) + 1.0);
    lo + t * (hi - lo)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..Default::default() })]

    #[test]
    fn xva_netting_invariants(
        is_call in any::<bool>(),
        strike_raw in any::<f64>(),
        vol_raw in any::<f64>(),
        notional_raw in any::<f64>(),
        r_dom_raw in any::<f64>(),
        r_for_raw in any::<f64>(),
        spot0_raw in any::<f64>(),
        sigma_raw in any::<f64>(),
        seed in any::<u64>(),
        lambda_cpty_raw in any::<f64>(),
        lambda_own_raw in any::<f64>(),
        lgd_c_raw in any::<f64>(),
        lgd_o_raw in any::<f64>(),
        funding_spread_raw in any::<f64>(),
        steps_u8 in any::<u8>(),
    ) {
        let strike = clamp_into(strike_raw, 1e-3, 1e4);
        let vol = clamp_into(vol_raw, 1e-4, 3.0);
        let notional = clamp_into(notional_raw, -1e7, 1e7);
        let r_dom = clamp_into(r_dom_raw, -0.25, 0.25);
        let r_for = clamp_into(r_for_raw, -0.25, 0.25);
        let spot0 = clamp_into(spot0_raw, 1e-3, 1e4);
        let sigma = clamp_into(sigma_raw, 1e-4, 3.0);
        let lambda_cpty = clamp_into(lambda_cpty_raw, 1e-6, 2.0);
        let lambda_own = clamp_into(lambda_own_raw, 1e-6, 2.0);
        let lgd_c = clamp_into(lgd_c_raw, 0.0, 1.0);
        let lgd_o = clamp_into(lgd_o_raw, 0.0, 1.0);
        let funding_spread = clamp_into(funding_spread_raw, -0.05, 0.05);
        let steps = (4 + (steps_u8 % 29)) as usize;

        // Use a fixed expiry that is always > 0.
        let expiry = 1.0_f64;
        let opt = if is_call { OptionType::Call } else { OptionType::Put };
        let trade = NettedTrade::new(opt, strike, expiry, vol, notional);
        let set = NettingSet::new(vec![trade], r_dom, r_for);

        prop_assert!(set.horizon() > 0.0, "horizon must be positive");

        let cfg = ExposureConfig { spot0, sigma, paths: 64, seed };

        // Contract 1: no panic.
        let profile = ExposureProfile::simulate(&set, &cfg, steps);

        // Contract 2: profile invariants.
        prop_assert_eq!(profile.grid()[0], 0.0_f64, "grid[0] must be 0");
        for &epe in profile.epe() {
            prop_assert!(epe >= 0.0 && epe.is_finite(), "EPE must be ≥ 0 and finite");
        }
        for &ene in profile.ene() {
            prop_assert!(ene >= 0.0 && ene.is_finite(), "ENE must be ≥ 0 and finite");
        }
        for (&t_k, &d_k) in profile.grid().iter().zip(profile.discount()) {
            let expected = libm::exp(-r_dom * t_k);
            prop_assert!((d_k - expected).abs() <= 1e-12 * (1.0 + expected),
                "discount factor mismatch at t={t_k}: got {d_k}, expected {expected}");
        }

        // Contract 3: determinism.
        let profile2 = ExposureProfile::simulate(&set, &cfg, steps);
        for (&a, &b) in profile.epe().iter().zip(profile2.epe()) {
            prop_assert_eq!(a.to_bits(), b.to_bits(), "EPE must be bit-reproducible");
        }

        // Contracts 4 + 5: XVA adjustments.
        let cpty = SurvivalCurve::flat(lambda_cpty);
        let own = SurvivalCurve::flat(lambda_own);
        let xva = compute_xva(&XvaInputs {
            profile: &profile,
            counterparty: &cpty,
            own: &own,
            lgd_counterparty: lgd_c,
            lgd_own: lgd_o,
            funding_spread,
        });
        prop_assert!(xva.cva >= 0.0 && xva.cva.is_finite(), "CVA must be ≥ 0 and finite");
        prop_assert!(xva.dva >= 0.0 && xva.dva.is_finite(), "DVA must be ≥ 0 and finite");
        prop_assert!(xva.fva.is_finite(), "FVA must be finite");
        prop_assert_eq!(
            xva.total_adjustment().to_bits(),
            (xva.cva - xva.dva + xva.fva).to_bits(),
            "total_adjustment must equal cva-dva+fva exactly"
        );

        // Contract 5: doubling lambda_cpty must not decrease CVA.
        let cpty2 = SurvivalCurve::flat(lambda_cpty * 2.0);
        let xva2 = compute_xva(&XvaInputs {
            profile: &profile,
            counterparty: &cpty2,
            own: &own,
            lgd_counterparty: lgd_c,
            lgd_own: lgd_o,
            funding_spread,
        });
        prop_assert!(xva2.cva >= xva.cva - 1e-12,
            "doubling hazard must not decrease CVA: before={}, after={}", xva.cva, xva2.cva);
    }
}
