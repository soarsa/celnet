//! Server Integration Test: Algorithmic Order Slicing Suite.
//!
//! Validates parent-order algorithmic decomposition (TWAP & Almgren-Chriss)
//! integrated with server execution tracking.

use celnet_algo::engine::{AlgoParentOrder, ChildSliceStatus, ParentOrderStatus};
use celnet_algo::optimal::OptimalExecutionConfig;
use celnet_algo::twap::TwapConfig;
use celnet_algo::PeggingStyle;

#[test]
fn test_server_algo_twap_execution_pipeline() {
    let config = TwapConfig {
        duration_seconds: 900.0, // 15 mins
        slice_count: 3,
        jitter_factor: 0.05,
        pegging_style: PeggingStyle::Midpoint,
    };

    let mut algo_order = AlgoParentOrder::new_twap(
        "SERVER-TWAP-77",
        "ZFZ26",
        300.0, // 300 contracts
        105.10, // Arrival price
        &config,
    )
    .expect("init twap order");

    assert_eq!(algo_order.slices.len(), 3);
    assert_eq!(algo_order.status, ParentOrderStatus::Active);

    // Simulate sequential fills arriving from exchange gateway
    for i in 0..3 {
        assert_eq!(algo_order.slices[i].status, ChildSliceStatus::Pending);
        let fill_px = 105.10 + (i as f64) * 0.01;
        algo_order.record_fill(i, 100.0, fill_px).expect("record fill");
        assert_eq!(algo_order.slices[i].status, ChildSliceStatus::Filled);
    }

    assert_eq!(algo_order.status, ParentOrderStatus::Completed);
    assert_eq!(algo_order.executed_quantity, 300.0);
    assert!((algo_order.avg_exec_price - 105.11).abs() < 1e-4);

    let is_bps = algo_order.implementation_shortfall_bps(true);
    assert!(is_bps > 0.0);
    println!("Server TWAP Algo: Completed with IS={:.2} bps", is_bps);
}

#[test]
fn test_server_almgren_chriss_liquidation_schedule() {
    let config = OptimalExecutionConfig {
        horizon_seconds: 1800.0, // 30 mins
        step_count: 6,
        volatility: 0.18,
        risk_aversion: 1.0e-5,
        temp_impact_eta: 2.0e-6,
        perm_impact_gamma: 1.0e-7,
    };

    let algo_order = AlgoParentOrder::new_optimal_liquidation(
        "SERVER-OPT-99",
        "USD-SOFR-10Y",
        600.0,
        99.50,
        &config,
    )
    .expect("init optimal liquidation");

    assert_eq!(algo_order.slices.len(), 6);
    let total_slice_qty: f64 = algo_order.slices.iter().map(|s| s.target_quantity).sum();
    assert!((total_slice_qty - 600.0).abs() < 1.0);
}
