//! Server Integration Test: Real-Time Clearing House Initial Margin.
//!
//! Validates pre-trade initial margin simulation (SPAN 2 FHS) on incoming client and hedge orders.

use celnet_margin::fhs::{FhsMarginCalculator, FhsMarginConfig};
use celnet_margin::portfolio::{ClearedPosition, MarginPortfolio, MarginProductFamily};
use celnet_margin::pre_trade::{MarginCheckOutcome, PreTradeMarginSimulator};

fn make_scenarios(count: usize, scale: f64) -> Vec<f64> {
    (0..count)
        .map(|i| ((i as f64) * 0.15 - 3.14159).sin() * scale * 1000.0)
        .collect()
}

#[test]
fn test_server_pre_trade_margin_gate() {
    let scenarios = make_scenarios(500, 2.5);

    // Current portfolio holding: 40 Treasury futures contracts
    let base_position = ClearedPosition::new(
        "ZFZ26",
        MarginProductFamily::BondFuture,
        40.0,
        100_000.0,
        15_000.0,
        false,
        105.00,
        scenarios.clone(),
    );

    let mut portfolio = MarginPortfolio::new("DESK-GLOBAL-MACRO");
    portfolio.add_or_update(base_position);

    let config = FhsMarginConfig::default();
    let initial_margin = FhsMarginCalculator::calculate_margin(&portfolio, &config).unwrap();
    assert!(initial_margin.total_margin > 0.0);

    // 1. Candidate Order 1: Small trade (10 contracts) within collateral limits
    let candidate_ok = ClearedPosition::new(
        "ZFZ26-NEW",
        MarginProductFamily::BondFuture,
        10.0,
        100_000.0,
        15_000.0,
        false,
        105.00,
        scenarios.clone(),
    );

    let check_ok = PreTradeMarginSimulator::check_trade(
        &portfolio,
        &candidate_ok,
        500_000.0, // $500k deposited collateral
        100_000.0, // $100k credit line
        &config,
    )
    .expect("check candidate ok");

    assert_eq!(check_ok.outcome, MarginCheckOutcome::Approved);
    assert!(check_ok.delta_margin > 0.0);

    // 2. Candidate Order 2: Outsized rogue order (500 contracts) that breaches collateral
    let candidate_rogue = ClearedPosition::new(
        "ZFZ26-ROGUE",
        MarginProductFamily::BondFuture,
        500.0, // 500 contracts ($50M notional!)
        100_000.0,
        15_000.0,
        false,
        105.00,
        scenarios,
    );

    let check_breach = PreTradeMarginSimulator::check_trade(
        &portfolio,
        &candidate_rogue,
        500_000.0, // only $500k collateral
        100_000.0,
        &config,
    )
    .expect("check candidate rogue");

    assert_eq!(check_breach.outcome, MarginCheckOutcome::ExceedsCollateral);
    println!(
        "Server Margin Pre-Trade Gate: Rogue order correctly rejected: Req Margin=${:.2} > Collateral=${:.2}",
        check_breach.initial_margin_after, 500_000.0
    );
}
