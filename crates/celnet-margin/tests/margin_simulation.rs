//! Unit and integration tests for clearing house initial margin simulation.

use celnet_margin::fhs::{FhsMarginCalculator, FhsMarginConfig};
use celnet_margin::grid::{ScenarioGridCalculator, ScenarioGridConfig};
use celnet_margin::portfolio::{ClearedPosition, MarginPortfolio, MarginProductFamily};
use celnet_margin::pre_trade::{MarginCheckOutcome, PreTradeMarginSimulator};

fn build_test_scenario_pnl(count: usize, base_step: f64) -> Vec<f64> {
    (0..count)
        .map(|i| {
            let angle = (i as f64) * 0.15 - 3.14159; // sweeps negative and positive
            libm::sin(angle) * base_step * 1000.0
        })
        .collect()
}

#[test]
fn test_fhs_margin_single_position() {
    let scenarios = build_test_scenario_pnl(500, 2.5);
    let pos = ClearedPosition::new(
        "ZFZ26",
        MarginProductFamily::BondFuture,
        50.0, // 50 contracts long
        100_000.0,
        15_000.0, // ADV 15,000 contracts
        false,
        105.25,
        scenarios,
    );

    let mut portfolio = MarginPortfolio::new("ACCT-HEDGE-1");
    portfolio.add_or_update(pos);

    let config = FhsMarginConfig::default();
    let margin = FhsMarginCalculator::calculate_margin(&portfolio, &config).expect("calc margin");

    assert!(margin.core_market_risk > 0.0);
    assert!(margin.total_margin >= margin.core_market_risk);
}

#[test]
fn test_fhs_margin_diversification_offset() {
    let scenarios_pos = build_test_scenario_pnl(500, 2.0);
    let mut scenarios_correlated = vec![0.0; 500];
    for i in 0..500 {
        scenarios_correlated[i] = scenarios_pos[i] * 0.95; // correlated asset
    }

    let leg1 = ClearedPosition::new(
        "ZFZ26",
        MarginProductFamily::BondFuture,
        100.0, // long 100
        100_000.0,
        20_000.0,
        false,
        105.0,
        scenarios_pos,
    );

    let leg2 = ClearedPosition::new(
        "ZNZ26",
        MarginProductFamily::BondFuture,
        -100.0, // short 100 on correlated contract
        100_000.0,
        20_000.0,
        false,
        110.0,
        scenarios_correlated,
    );

    let mut port_unhedged = MarginPortfolio::new("UNHEDGED");
    port_unhedged.add_or_update(leg1.clone());

    let mut port_hedged = MarginPortfolio::new("HEDGED");
    port_hedged.add_or_update(leg1);
    port_hedged.add_or_update(leg2);

    let config = FhsMarginConfig::default();
    let margin_unhedged = FhsMarginCalculator::calculate_margin(&port_unhedged, &config).unwrap();
    let margin_hedged = FhsMarginCalculator::calculate_margin(&port_hedged, &config).unwrap();

    // Hedged portfolio margin must be substantially lower due to risk offset!
    assert!(margin_hedged.core_market_risk < margin_unhedged.core_market_risk * 0.25);
}

#[test]
fn test_pre_trade_delta_margin_fast_path() {
    let scenarios = build_test_scenario_pnl(500, 1.5);
    let existing_pos = ClearedPosition::new(
        "USD-SOFR-5Y",
        MarginProductFamily::InterestRateSwap,
        10.0,
        1_000_000.0,
        500.0,
        false,
        100.0,
        scenarios.clone(),
    );

    let mut portfolio = MarginPortfolio::new("PORT-007");
    portfolio.add_or_update(existing_pos);

    let candidate_order = ClearedPosition::new(
        "USD-SOFR-5Y-CANDIDATE",
        MarginProductFamily::InterestRateSwap,
        5.0,
        1_000_000.0,
        500.0,
        false,
        100.0,
        scenarios,
    );

    let config = FhsMarginConfig::default();
    let check = PreTradeMarginSimulator::check_trade(
        &portfolio,
        &candidate_order,
        1_000_000.0, // $1M collateral deposited
        200_000.0,   // $200k credit limit
        &config,
    )
    .expect("pre-trade check");

    assert_eq!(check.outcome, MarginCheckOutcome::Approved);
    assert!(check.delta_margin > 0.0);
    assert!(check.initial_margin_after > check.initial_margin_before);
}

#[test]
fn test_scenario_grid_margin() {
    let scenarios = build_test_scenario_pnl(16, 5.0);
    let pos = ClearedPosition::new(
        "BUND-10Y",
        MarginProductFamily::BondFuture,
        20.0,
        100_000.0,
        10_000.0,
        false,
        130.0,
        scenarios,
    );

    let mut portfolio = MarginPortfolio::new("GRID-PORT");
    portfolio.add_or_update(pos);

    let config = ScenarioGridConfig::default();
    let grid_margin = ScenarioGridCalculator::calculate_grid_margin(&portfolio, &config).unwrap();
    assert!(grid_margin.total_margin > 0.0);
}

#[test]
fn test_pre_trade_same_instrument_accumulation() {
    let scenarios = build_test_scenario_pnl(16, 50.0);
    let existing_pos = ClearedPosition::new(
        "EURUSD-OPT",
        MarginProductFamily::FxOption,
        10.0,
        100_000.0,
        100.0,
        false,
        1.10,
        scenarios.clone(),
    );

    let mut portfolio = MarginPortfolio::new("PORT-ACC");
    portfolio.add_or_update(existing_pos);

    // Candidate trade for same instrument with 10.0 more contracts
    let candidate_order = ClearedPosition::new(
        "EURUSD-OPT",
        MarginProductFamily::FxOption,
        10.0,
        100_000.0,
        100.0,
        false,
        1.10,
        scenarios,
    );

    let config = FhsMarginConfig::default();
    let check = PreTradeMarginSimulator::check_trade(
        &portfolio,
        &candidate_order,
        2_000_000.0,
        1_000_000.0,
        &config,
    )
    .expect("pre-trade check");

    assert_eq!(check.outcome, MarginCheckOutcome::Approved);
    assert!(check.delta_margin > 0.0);
    assert!(check.initial_margin_after > check.initial_margin_before);
}
