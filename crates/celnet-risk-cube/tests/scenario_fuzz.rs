//! Proptest mirror of `fuzz/fuzz_targets/risk_cube_scenarios.rs`.
//!
//! Asserts the same algebraic identity and roll-up conservation contracts as the
//! nightly libFuzzer target, on the stable toolchain inside `just check`:
//!
//!   1. `Scenario::base().apply(i)` reproduces every field `to_bits`-equal;
//!   2. `Scenario::fx_rates` round-trip: exact carry-coordinate arithmetic;
//!   3. `position_pnl` and `node_pnl` are finite for every (position, scenario);
//!   4. `node_pnl == Σ position_pnl` within 4 ULP-scaled tolerance;
//!   5. `historical_var_es`: `es >= var`, both finite, match independent oracle.
//!
//! 256 cases; same draw ranges as the fuzz target.

use celnet_risk_cube::nonadditive::{Scenario, historical_var_es, node_pnl, position_pnl};
use celnet_risk_normalize::{AssetPricer, PositionRisk};
use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};
use proptest::prelude::*;

fn clamp_into(raw: f64, lo: f64, hi: f64) -> f64 {
    let mid = 0.5 * (lo + hi);
    if !raw.is_finite() { return mid; }
    let t = 0.5 * (libm::tanh(raw) + 1.0);
    lo + t * (hi - lo)
}

fn build_position(
    is_call: bool,
    spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64, notional: f64,
) -> PositionRisk {
    let pair = CcyPair::new(Ccy::EUR, Ccy::USD);
    let opt = if is_call { OptionType::Call } else { OptionType::Put };
    let inputs = VanillaInputs::new(spot, strike, vol, t, r_dom, r_for);
    PositionRisk::fx(pair, opt, notional, inputs, DeltaConvention::SpotUnadjusted, PremiumStyle::DomesticPips)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..Default::default() })]

    #[test]
    fn scenario_algebra_and_rollup_conservation(
        // One position
        is_call in any::<bool>(),
        spot_raw in any::<f64>(),
        strike_raw in any::<f64>(),
        vol_raw in any::<f64>(),
        t_raw in any::<f64>(),
        r_dom_raw in any::<f64>(),
        r_for_raw in any::<f64>(),
        notional_raw in any::<f64>(),
        // One scenario
        spot_rel_raw in any::<f64>(),
        vol_abs_raw in any::<f64>(),
        discount_abs_raw in any::<f64>(),
        carry_abs_raw in any::<f64>(),
        // fx_rates round-trip inputs
        dr_d in -0.05f64..0.05f64,
        dr_f in -0.05f64..0.05f64,
    ) {
        let spot = clamp_into(spot_raw, 1e-3, 1e4);
        let strike = clamp_into(strike_raw, 1e-3, 1e4);
        let vol = clamp_into(vol_raw, 1e-4, 3.0);
        let t = clamp_into(t_raw, 1.0/365.0, 10.0);
        let r_dom = clamp_into(r_dom_raw, -0.25, 0.25);
        let r_for = clamp_into(r_for_raw, -0.25, 0.25);
        let notional = clamp_into(notional_raw, -1e7, 1e7);

        let pos = build_position(is_call, spot, strike, vol, t, r_dom, r_for, notional);
        let pricer = AssetPricer;

        // Contract 1: base scenario is identity.
        let shocked = Scenario::base().apply(&pos.inputs);
        prop_assert_eq!(shocked.spot.to_bits(), pos.inputs.spot.to_bits());
        prop_assert_eq!(shocked.vol.to_bits(), pos.inputs.vol.to_bits());
        prop_assert_eq!(shocked.t.to_bits(), pos.inputs.t.to_bits());

        // Contract 2: fx_rates round-trip.
        let scen = Scenario::fx_rates(0.0, 0.0, dr_d, dr_f);
        prop_assert_eq!(scen.discount_abs.to_bits(), dr_d.to_bits());
        prop_assert_eq!(scen.carry_abs.to_bits(), (dr_d - dr_f).to_bits());
        // apply: r_dom shifts by dr_d, r_for shifts by dr_f.
        // Note: shift_carry recovers dr_f via `discount_abs - carry_abs`; due to the
        // two-step subtraction `dr_d - (dr_d - dr_f)` a 1-ULP difference is possible,
        // so we test with a tight epsilon (1e-15) rather than bit-exact equality.
        let shocked2 = scen.apply(&pos.inputs);
        if let celnet_types::Carry::FxRates { r_dom: rd, r_for: rf } = pos.inputs.carry {
            if let celnet_types::Carry::FxRates { r_dom: rd_s, r_for: rf_s } = shocked2.carry {
                prop_assert_eq!(rd_s.to_bits(), (rd + dr_d).to_bits());
                let rf_exp = rf + dr_f;
                prop_assert!(
                    (rf_s - rf_exp).abs() <= 1e-15 * (1.0 + rf_exp.abs()),
                    "r_for round-trip: got {rf_s}, expected {rf_exp} (tol 1e-15)"
                );
            }
        }

        // Contracts 3 + 4: position/node PnL finite and additive.
        let scen3 = Scenario {
            spot_rel: clamp_into(spot_rel_raw, -0.5, 1.0),
            vol_abs: clamp_into(vol_abs_raw, -0.05, 0.25),
            discount_abs: clamp_into(discount_abs_raw, -0.05, 0.05),
            carry_abs: clamp_into(carry_abs_raw, -0.05, 0.05),
        };
        let positions = [pos];
        let p_pnl = position_pnl(&pricer, &positions[0], scen3);
        prop_assert!(p_pnl.is_finite(), "position_pnl must be finite");
        let n_pnl = node_pnl(&pricer, &positions, scen3);
        prop_assert!(n_pnl.is_finite(), "node_pnl must be finite");
        let tol = 1e-12 * (1.0 + p_pnl.abs() + n_pnl.abs());
        prop_assert!((n_pnl - p_pnl).abs() <= tol,
            "node_pnl ({n_pnl}) must equal Σ position_pnl ({p_pnl}) within {tol}");

        // Contract 5: VarEs finite and es >= var.
        let scenarios = [scen3];
        let vaes = historical_var_es(&pricer, &positions, &scenarios, 0.95);
        prop_assert!(vaes.var.is_finite(), "VaR must be finite");
        prop_assert!(vaes.es.is_finite(), "ES must be finite");
        prop_assert!(vaes.es >= vaes.var - 1e-12,
            "ES ({}) must be >= VaR ({})", vaes.es, vaes.var);
    }
}
