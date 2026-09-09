//! Unit and integration tests for algorithmic order execution suite.

use celnet_algo::engine::{AlgoParentOrder, ChildSliceStatus, ParentOrderStatus};
use celnet_algo::optimal::{OptimalExecutionConfig, OptimalExecutionSlicer};
use celnet_algo::pov::{PovConfig, PovSlicer};
use celnet_algo::twap::{TwapConfig, TwapSlicer};
use celnet_algo::vwap::{VwapConfig, VwapSlicer};
use celnet_algo::PeggingStyle;

#[test]
fn test_twap_schedule_generation() {
    let config = TwapConfig {
        duration_seconds: 1200.0, // 20 mins
        slice_count: 5,
        jitter_factor: 0.10,
        pegging_style: PeggingStyle::PassiveTouch,
    };

    let schedule = TwapSlicer::build_schedule(500_000.0, &config).expect("build twap");
    assert_eq!(schedule.len(), 5);

    let total_sched_qty: f64 = schedule.iter().map(|s| s.target_quantity).sum();
    assert!((total_sched_qty - 500_000.0).abs() < 1e-4);

    for s in &schedule {
        assert!(s.scheduled_offset_seconds >= 0.0);
        assert!(s.scheduled_offset_seconds <= 1200.0);
    }
}

#[test]
fn test_vwap_schedule_generation_and_pacing() {
    let config = VwapConfig::default();
    let schedule = VwapSlicer::build_schedule(1_000_000.0, &config).expect("build vwap");
    assert_eq!(schedule.len(), 10);

    let total_qty: f64 = schedule.iter().map(|s| s.target_quantity).sum();
    assert!((total_qty - 1_000_000.0).abs() < 1e-4);

    // Test dynamic pacing adjustment
    let base_slice = 100_000.0;
    let accelerated = VwapSlicer::adjust_for_pacing(base_slice, 200_000.0, 100_000.0, 1.0);
    assert_eq!(accelerated, 200_000.0); // 2x cap

    let throttled = VwapSlicer::adjust_for_pacing(base_slice, 50_000.0, 100_000.0, 1.0);
    assert_eq!(throttled, 50_000.0); // 0.5x
}

#[test]
fn test_pov_slice_calculation() {
    let config = PovConfig {
        target_participation_rate: 0.10, // 10%
        max_slice_quantity: 50_000.0,
        min_slice_quantity: 1_000.0,
    };

    // Tape prints 90,000 shares: 10% participation => (0.10 / 0.90) * 90,000 = 10,000 shares
    let slice = PovSlicer::calculate_child_slice(100_000.0, 90_000.0, &config).expect("calc pov");
    assert!((slice - 10_000.0).abs() < 1e-4);

    // Remainder capping
    let final_slice = PovSlicer::calculate_child_slice(3_000.0, 90_000.0, &config).expect("calc pov");
    assert_eq!(final_slice, 3_000.0);
}

#[test]
fn test_almgren_chriss_optimal_liquidation() {
    let config = OptimalExecutionConfig {
        horizon_seconds: 3600.0,
        step_count: 10,
        volatility: 0.20,
        risk_aversion: 1.0e-5,
        temp_impact_eta: 2.0e-6,
        perm_impact_gamma: 1.0e-7,
    };

    let (trajectory, summary) = OptimalExecutionSlicer::compute_trajectory(1_000_000.0, &config)
        .expect("compute optimal");

    assert_eq!(trajectory.len(), 11);
    assert_eq!(trajectory[0].remaining_holdings, 1_000_000.0);
    assert!((trajectory[10].remaining_holdings).abs() < 1.0); // fully liquidated

    // Monotonically decreasing holdings
    for i in 1..=10 {
        assert!(trajectory[i].remaining_holdings <= trajectory[i - 1].remaining_holdings);
        assert!(trajectory[i].trade_slice_size > 0.0);
    }

    assert!(summary.expected_cost > 0.0);
    assert!(summary.cost_variance > 0.0);
}

#[test]
fn test_algo_parent_order_lifecycle_and_tca() {
    let config = TwapConfig {
        duration_seconds: 600.0,
        slice_count: 3,
        jitter_factor: 0.0,
        pegging_style: PeggingStyle::Midpoint,
    };

    let mut parent = AlgoParentOrder::new_twap("ORD-TWAP-101", "ZFZ26", 300.0, 105.00, &config)
        .expect("create parent");

    assert_eq!(parent.status, ParentOrderStatus::Active);
    assert_eq!(parent.slices.len(), 3);

    // Record Fill 1: 100 contracts at 105.02
    parent.record_fill(0, 100.0, 105.02).expect("fill 1");
    assert_eq!(parent.slices[0].status, ChildSliceStatus::Filled);
    assert_eq!(parent.executed_quantity, 100.0);

    // Record Fill 2: 100 contracts at 105.05
    parent.record_fill(1, 100.0, 105.05).expect("fill 2");

    // Record Fill 3: 100 contracts at 105.08
    parent.record_fill(2, 100.0, 105.08).expect("fill 3");

    assert_eq!(parent.status, ParentOrderStatus::Completed);
    assert_eq!(parent.executed_quantity, 300.0);

    // Average price: (100*105.02 + 100*105.05 + 100*105.08) / 300 = 105.05
    assert!((parent.avg_exec_price - 105.05).abs() < 1e-4);

    // Implementation Shortfall bps for Buy: (105.05 - 105.00) / 105.00 * 10000 = 4.7619 bps
    let is_bps = parent.implementation_shortfall_bps(true);
    assert!((is_bps - 4.7619).abs() < 1e-2);
}
