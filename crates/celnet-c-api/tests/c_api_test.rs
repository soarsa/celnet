use celnet_c_api::*;

#[test]
fn test_c_api_version() {
    assert_eq!(celnet_c_api_version(), 20260901);
}

#[test]
fn test_c_api_vanilla_pricing() {
    let greeks = celnet_price_vanilla(
        1.0850,
        1.0850,
        1.0,
        0.085,
        0.045,
        0.035,
        true,
    );
    assert!(greeks.price > 0.0);
    assert!(greeks.delta_spot > 0.45 && greeks.delta_spot < 0.55);
    assert!(greeks.gamma > 0.0);
    assert!(greeks.vega > 0.0);
}

#[test]
fn test_c_api_barrier_pricing() {
    // Spot above barrier -> live option
    let live = celnet_price_barrier(
        1.0850,
        1.0850,
        1.0200,
        1.0,
        0.085,
        0.045,
        0.035,
        true,
        true, // down-and-out
    );
    assert!(live.price > 0.0);

    // Spot breaches barrier -> knocked out (price = 0)
    let ko = celnet_price_barrier(
        1.0100,
        1.0850,
        1.0200,
        1.0,
        0.085,
        0.045,
        0.035,
        true,
        true, // down-and-out
    );
    assert_eq!(ko.price, 0.0);
    assert_eq!(ko.delta_spot, 0.0);
}

#[test]
fn test_c_api_rates_ois() {
    let ois = celnet_price_rates_ois(
        0.045,
        5.0,
        10_000_000.0,
        0.045,
        true,
    );
    assert_eq!(ois.pv, 0.0); // at par
    assert_eq!(ois.par_rate, 0.045);
    assert!(ois.pv01 > 0.0);
}

#[test]
fn test_c_api_margin_and_pretrade() {
    let margin = celnet_calculate_margin(10_000_000.0, 0.99, 500);
    assert_eq!(margin.total_initial_margin, 450_000.0);
    assert_eq!(margin.expected_shortfall, 450_000.0 * 0.85);

    let pt_approved = celnet_simulate_pre_trade_margin(
        2_000_000.0,
        1_000_000.0,
        450_000.0,
    );
    assert!(pt_approved.is_approved);
    assert_eq!(pt_approved.delta_margin, 90_000.0);
    assert_eq!(pt_approved.collateral_headroom, 460_000.0);

    let pt_rejected = celnet_simulate_pre_trade_margin(
        50_000_000.0,
        500_000.0,
        450_000.0,
    );
    assert!(!pt_rejected.is_approved);
    assert_eq!(pt_rejected.collateral_headroom, 0.0);
}

#[test]
fn test_c_api_twap_and_cluster() {
    let algo = celnet_plan_twap_algo(
        10_000_000.0,
        1.0850,
        300,
        5,
        true,
    );
    assert_eq!(algo.total_quantity, 10_000_000.0);
    assert_eq!(algo.executed_quantity, 2_000_000.0);
    assert_eq!(algo.slice_count, 5);

    let cluster = celnet_check_cluster_health(3, 42);
    assert!(cluster.is_consensus_healthy);
    assert_eq!(cluster.active_generation, 42);

    let upgrade_pass = celnet_verify_shadow_twin_ulp(0, 1_000_000);
    assert!(upgrade_pass.is_bit_exact_pass);

    let upgrade_fail = celnet_verify_shadow_twin_ulp(2, 1_000_000);
    assert!(!upgrade_fail.is_bit_exact_pass);
}

#[test]
fn test_c_api_attestation_and_licensing() {
    let att_valid = celnet_verify_hardware_attestation(0xAABBCCDD, 0xAABBCCDD);
    assert!(att_valid.is_valid);
    assert!(att_valid.hardware_pcr_match);

    let att_invalid = celnet_verify_hardware_attestation(0xAABBCCDD, 0x11223344);
    assert!(!att_invalid.is_valid);
    assert!(!att_invalid.hardware_pcr_match);

    let lic_enterprise = celnet_verify_biscuit_license_caps(3, 1893456000);
    assert!(lic_enterprise.is_valid);
    assert_eq!(lic_enterprise.capability_mask, u64::MAX);
}

#[test]
fn test_c_api_digital_pricing() {
    let call = celnet_price_digital(100.0, 100.0, 1.0, 0.20, 0.05, 0.0, 1000.0, true);
    assert!(call.price > 0.0 && call.price < 1000.0);
    assert!(call.delta_spot > 0.0);

    let put = celnet_price_digital(100.0, 100.0, 1.0, 0.20, 0.05, 0.0, 1000.0, false);
    assert!(put.price > 0.0 && put.price < 1000.0);
    assert!(put.delta_spot < 0.0);
}

#[test]
fn test_c_api_irs_fra_bond_pricing() {
    // IRS
    let irs = celnet_price_rates_irs(0.04, 0.04, 5.0, 2, 10_000_000.0, 0.04, true);
    assert!(irs.pv.abs() < 1e-4);
    assert!(irs.pv01 > 0.0);

    // FRA
    let fra = celnet_price_rates_fra(0.035, 0.040, 0.5, 1.0, 10_000_000.0, 0.04, true);
    assert!(fra.pv > 0.0);
    assert!(fra.pv01 > 0.0);

    // Bond
    let bond = celnet_price_bond(0.05, 0.05, 10.0, 2, 0.0);
    assert!((bond.dirty_price - 100.0).abs() < 1e-3);
    assert!((bond.clean_price - 100.0).abs() < 1e-3);
    assert!(bond.modified_duration > 0.0);
    assert!(bond.convexity > 0.0);
}

#[test]
fn test_c_api_signature_vol_and_propagator_algo() {
    // 1. Signature Rough Volatility Smile
    let sig_pt = celnet_price_signature_vol(
        100.0,
        95.0,
        0.25,
        0.10,
        0.20,
        0.45,
        -0.65,
        0.04,
    );
    assert!(sig_pt.atm_vol > 0.0);
    assert!(sig_pt.atm_skew < 0.0); // steep negative skew
    assert!(sig_pt.implied_vol > 0.0);

    // 2. Transient Propagator Algo with Order Book Imbalance (OBI)
    let algo = celnet_plan_propagator_algo(
        100_000.0,
        1.0850,
        1800,
        10,
        0.5, // strong buy order book imbalance
        120.0, // 2-minute decay half life
        1.0e-5,
    );
    assert_eq!(algo.total_quantity, 100_000.0);
    assert!(algo.executed_quantity > 0.0);
    assert!(algo.is_active);
}
