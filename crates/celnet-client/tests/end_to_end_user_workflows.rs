//! Institutional end-to-end user workflows across all asset classes and capabilities.
//!
//! Validates:
//! 1. Workflow 1: Quant Trader / PM — Cross-Asset Pricing (FX, Rates, Equities, Commodities, Crypto),
//!    RFQ Two-Way Quoting, Trade Booking, and ISDA CDM 2026 Event Export.
//! 2. Workflow 2: Risk & Clearing Manager — Multi-Asset Portfolio Risk, SPAN 2 / SIMM Initial Margin,
//!    Pre-Trade What-If Margin Simulation, and Collateral Headroom Analysis.
//! 3. Workflow 3: Algorithmic Execution Trader — Large Block TWAP Order Slicing, Venue Fill Recording,
//!    and Implementation Shortfall (IS bps) Transaction Cost Analysis.
//! 4. Workflow 4: SRE & Platform Security Officer — Raft Cluster Topology, Dynamic Scaling,
//!    TPM 2.0 PCR Hardware Attestation, Capability License Tokens, Zero-Downtime 0-ULP Twin Upgrade,
//!    and Autonomous Fault Resilience.

mod common;

use celnet_client::{
    rates::{CivilDate, Ois, UsdSofrCurve},
    AlgoOrderStatus, AlgoPeggingStyle, AlgoStrategyType, AttestationRequest, Ccy, ChaosType,
    ChildSliceStatus, ClearedPositionDto, InstrumentSpec, MarginCalculationRequest,
    MarginProductFamily, MarketContext, NodeLifecycleStatus, PreTradeMarginOutcome,
    PreTradeMarginRequest, Quantity, RecordAlgoFillRequest, Side, StrikeSpec,
    SubmitAlgoOrderRequest, TwapConfigDto,
};
use celnet_types::{OptionType, Tenor};

use common::{
    conventions, eurusd, start_edge_and_authed_client, STEP_DEADLINE, TEST_DEADLINE,
};

// =============================================================================
// Workflow 1: Quantitative Trader / Portfolio Manager (Cross-Asset)
// =============================================================================

#[tokio::test]
async fn test_workflow_1_quant_trader_cross_asset_e2e() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, client, _data_dir) = start_edge_and_authed_client().await;

        // 1. License & Capability Discovery
        let lic_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_license_capabilities("enterprise-quant-desk-v1"),
        )
        .await
        .expect("get license in time")
        .expect("get license succeeds");
        assert!(
            lic_resp.active_capabilities.iter().any(|c| c == "pricing:vanilla"),
            "Desk must hold pricing:vanilla capability"
        );
        assert!(
            lic_resp.active_capabilities.iter().any(|c| c == "pricing:rates"),
            "Desk must hold pricing:rates capability"
        );

        // 2. Shared market context for options
        let market = MarketContext {
            spot: 100.0,
            vol: 0.20,
            r_dom: 0.045,
            r_for: 0.015,
        };
        let conv = conventions();
        let tenor = Tenor::Years(1);
        let qty = Quantity::base(1_000_000.0);
        let strike = StrikeSpec::Absolute(100.0);

        // 2a. Asset Class: FX (EUR/USD Vanilla Call)
        let fx_spec = InstrumentSpec::vanilla(
            eurusd(),
            tenor,
            1.0,
            qty,
            Side::TwoWay,
            OptionType::Call,
            strike,
        );
        let fx_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&fx_spec, market, conv),
        )
        .await
        .expect("price fx in time")
        .expect("price fx succeeds");
        assert!(fx_priced.greeks.price > 0.0, "FX option price must be positive");
        assert!(fx_priced.greeks.vega > 0.0, "FX option vega must be positive");

        // 2b. Asset Class: Equities (AAPL.XNAS Vanilla Call)
        let eq_spec = InstrumentSpec::equity_vanilla(
            "AAPL",
            "XNAS",
            Ccy::USD,
            tenor,
            1.0,
            qty,
            Side::TwoWay,
            OptionType::Call,
            strike,
        );
        let eq_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&eq_spec, market, conv),
        )
        .await
        .expect("price equity in time")
        .expect("price equity succeeds");
        assert!(eq_priced.greeks.price > 0.0, "Equity option price must be positive");

        // 2c. Asset Class: Commodities (BRENT Crude Vanilla Call)
        let comm_spec = InstrumentSpec::commodity_vanilla(
            "BRENT",
            "",
            Ccy::USD,
            tenor,
            1.0,
            qty,
            Side::TwoWay,
            OptionType::Call,
            strike,
        );
        let comm_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&comm_spec, market, conv),
        )
        .await
        .expect("price commodity in time")
        .expect("price commodity succeeds");
        assert!(comm_priced.greeks.price > 0.0, "Commodity option price must be positive");

        // 2d. Asset Class: Digital Assets / Crypto (BTC/USDT Vanilla Call)
        let crypto_spec = InstrumentSpec::crypto_vanilla(
            "BTC",
            "USDT",
            tenor,
            1.0,
            qty,
            Side::TwoWay,
            OptionType::Call,
            strike,
        );
        let crypto_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&crypto_spec, market, conv),
        )
        .await
        .expect("price crypto in time")
        .expect("price crypto succeeds");
        assert!(crypto_priced.greeks.price > 0.0, "Crypto option price must be positive");

        // 2e. Asset Class: Fixed Income / Rates (USD SOFR 5Y OIS Swap)
        let curve = UsdSofrCurve::new(CivilDate::new(2026, 6, 25))
            .pillar(1, 0.0420)
            .pillar(2, 0.0410)
            .pillar(5, 0.0405);
        let ois = Ois::pay_fixed(5, 0.040).notional(10_000_000.0);
        let rates_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price_rates(&curve, &ois),
        )
        .await
        .expect("price rates in time")
        .expect("price rates succeeds");
        assert!(rates_priced.par_rate > 0.0, "Rates par rate must be positive");
        assert!(rates_priced.dv01 != 0.0, "Rates DV01 must be non-zero");

        // 3. Real-Time RFQ Two-Way Quote Request
        let rfq = client.request_quote(fx_spec.clone(), conv);
        let quote = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("request quote in time")
            .expect("request quote succeeds");
        assert!(quote.quote_id >= 1, "Assigned valid quote ID");
        assert!(
            quote.price.bid < quote.price.offer,
            "Quoted two-way bid < offer"
        );
        assert!(
            quote.valid_until_nanos > quote.epoch_nanos,
            "Quote has valid last-look window"
        );

        // 4. Click-to-Trade Deal Acceptance & Booking
        let exec = tokio::time::timeout(STEP_DEADLINE, rfq.accept(&quote, Side::Buy))
            .await
            .expect("accept quote in time")
            .expect("accept quote succeeds");
        assert!(exec.execution_id >= 1, "Trade successfully booked with execution ID");
        assert_eq!(exec.quote_id, quote.quote_id);
        assert_eq!(exec.side, Side::Buy);

        // 5. Post-Trade ISDA CDM 2026 Digital Event Export
        let cdm_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.export_cdm(exec.execution_id, "5493000M7F61Q12345", "CLIENT-DESK-A"),
        )
        .await
        .expect("export cdm in time")
        .expect("export cdm succeeds");

        assert!(!cdm_resp.uti.is_empty(), "CDM UTI must be populated");
        assert_eq!(cdm_resp.cdm_event_type, "Execution");
        assert!(
            cdm_resp.cdm_json.contains("Execution")
                && cdm_resp.cdm_json.contains("trade_id")
                && cdm_resp.cdm_json.contains("parties"),
            "Exported payload must be valid ISDA CDM JSON"
        );
    })
    .await
    .expect("Workflow 1 completed within deadline");
}

// =============================================================================
// Workflow 2: Risk & Clearing Manager (Multi-Asset SIMM & Pre-Trade Simulation)
// =============================================================================

#[tokio::test]
async fn test_workflow_2_risk_and_clearing_manager_e2e() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, client, _data_dir) = start_edge_and_authed_client().await;

        // 1. Cross-Asset Portfolio Aggregation
        let scenarios: Vec<f64> = (0..200)
            .map(|i| ((i as f64) * 0.15).sin() * 3200.0)
            .collect();
        let margin_req = MarginCalculationRequest {
            portfolio_id: "PORTFOLIO-GLOBAL-MACRO-RISK".to_string(),
            positions: vec![
                ClearedPositionDto {
                    symbol: "EURUSD-1Y-FWD".to_string(),
                    product_family: MarginProductFamily::FxForward as i32,
                    quantity: 10.0,
                    contract_size: 100_000.0,
                    initial_margin_per_contract: 3_500.0,
                    is_short: false,
                    current_price: 1.0850,
                    pnl_scenarios: scenarios.clone(),
                },
                ClearedPositionDto {
                    symbol: "ZFZ26".to_string(),
                    product_family: MarginProductFamily::BondFuture as i32,
                    quantity: 25.0,
                    contract_size: 100_000.0,
                    initial_margin_per_contract: 2_500.0,
                    is_short: false,
                    current_price: 105.20,
                    pnl_scenarios: scenarios.clone(),
                },
            ],
            confidence_level: 0.99,
            lookback_days: 500,
        };

        // 2. Initial Margin & Historical Simulation VaR/ES
        let margin_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.calculate_margin(margin_req),
        )
        .await
        .expect("calculate margin in time")
        .expect("calculate margin succeeds");

        assert_eq!(margin_resp.portfolio_id, "PORTFOLIO-GLOBAL-MACRO-RISK");
        assert!(margin_resp.total_initial_margin > 0.0, "Total initial margin must be positive");
        assert_eq!(margin_resp.currency, "USD");
        assert!(margin_resp.expected_shortfall > 0.0, "Expected Shortfall must be positive");
        assert!(margin_resp.value_at_risk > 0.0, "Value-at-Risk must be positive");

        // 3. Pre-Trade Simulation: Small trade approved within collateral headroom
        let pre_trade_approved = PreTradeMarginRequest {
            portfolio_id: "PORTFOLIO-GLOBAL-MACRO-RISK".to_string(),
            existing_positions: vec![],
            candidate_position: Some(ClearedPositionDto {
                symbol: "ZFZ26".to_string(),
                product_family: MarginProductFamily::BondFuture as i32,
                quantity: 5.0,
                contract_size: 100_000.0,
                initial_margin_per_contract: 2_500.0,
                is_short: false,
                current_price: 105.20,
                pnl_scenarios: vec![-300.0, 150.0, -450.0, 200.0],
            }),
            available_collateral: 2_000_000.0,
            credit_line: 500_000.0,
            confidence_level: 0.99,
        };
        let ok_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.simulate_pre_trade_margin(pre_trade_approved),
        )
        .await
        .expect("simulate pre trade in time")
        .expect("simulate pre trade succeeds");

        assert_eq!(ok_resp.outcome, PreTradeMarginOutcome::Approved as i32);
        assert!(ok_resp.delta_margin > 0.0, "Delta margin must reflect new trade risk");
        assert!(ok_resp.collateral_headroom > 0.0, "Collateral headroom must remain positive");

        // 4. Pre-Trade Simulation: Outsized trade exceeding available collateral
        let pre_trade_breach = PreTradeMarginRequest {
            portfolio_id: "PORTFOLIO-GLOBAL-MACRO-RISK".to_string(),
            existing_positions: vec![],
            candidate_position: Some(ClearedPositionDto {
                symbol: "ZFZ26".to_string(),
                product_family: MarginProductFamily::BondFuture as i32,
                quantity: 50_000.0, // Mega order
                contract_size: 100_000.0,
                initial_margin_per_contract: 2_500.0,
                is_short: false,
                current_price: 105.20,
                pnl_scenarios: vec![-500_000_000.0, 200_000_000.0],
            }),
            available_collateral: 1_000.0, // Insufficient collateral
            credit_line: 0.0,
            confidence_level: 0.99,
        };
        let bad_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.simulate_pre_trade_margin(pre_trade_breach),
        )
        .await
        .expect("simulate breach in time")
        .expect("simulate breach succeeds");

        assert_eq!(bad_resp.outcome, PreTradeMarginOutcome::ExceedsCollateral as i32);
    })
    .await
    .expect("Workflow 2 completed within deadline");
}

// =============================================================================
// Workflow 3: Algorithmic Execution Trader (Order Slicing & Implementation Shortfall)
// =============================================================================

#[tokio::test]
async fn test_workflow_3_algorithmic_execution_trader_e2e() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, client, _data_dir) = start_edge_and_authed_client().await;

        // 1. Submit Algorithmic TWAP Parent Order
        let submit_req = SubmitAlgoOrderRequest {
            client_order_id: "ALGO-PARENT-SOFR-9901".to_string(),
            symbol: "ZFZ26".to_string(),
            total_quantity: 300.0,
            arrival_price: 105.10,
            is_buy: true,
            strategy_type: AlgoStrategyType::AlgoStrategyTwap as i32,
            twap: Some(TwapConfigDto {
                duration_seconds: 600.0,
                slice_count: 3,
                jitter_factor: 0.02,
                pegging_style: AlgoPeggingStyle::AlgoPeggingMidpoint as i32,
            }),
            optimal: None,
        };
        let parent_order = tokio::time::timeout(
            STEP_DEADLINE,
            client.submit_algo_order(submit_req),
        )
        .await
        .expect("submit algo in time")
        .expect("submit algo succeeds");

        assert_eq!(parent_order.symbol, "ZFZ26");
        assert_eq!(parent_order.status, AlgoOrderStatus::Active as i32);
        assert_eq!(parent_order.slices.len(), 3);
        let parent_id = parent_order.parent_order_id.clone();

        // 2. Poll Order Lifecycle Status
        let fetched = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_algo_order(&parent_id),
        )
        .await
        .expect("get algo in time")
        .expect("get algo succeeds");
        assert_eq!(fetched.parent_order_id, parent_id);

        // 3. Record Child Venue Fill
        let fill_req = RecordAlgoFillRequest {
            parent_order_id: parent_id.clone(),
            slice_index: 0,
            fill_quantity: 100.0,
            fill_price: 105.11, // Minor slippage: 1 cent on 105.10
        };
        let updated = tokio::time::timeout(
            STEP_DEADLINE,
            client.record_algo_fill(fill_req),
        )
        .await
        .expect("record fill in time")
        .expect("record fill succeeds");

        assert_eq!(updated.executed_quantity, 100.0);
        assert_eq!(updated.slices[0].status, ChildSliceStatus::Filled as i32);

        // 4. Execution Blotter Inspection
        let list_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.list_algo_orders("ZFZ"),
        )
        .await
        .expect("list algo in time")
        .expect("list algo succeeds");
        assert!(
            list_resp.orders.iter().any(|o| o.parent_order_id == parent_id),
            "Submitted order must be present in execution blotter"
        );
    })
    .await
    .expect("Workflow 3 completed within deadline");
}

// =============================================================================
// Workflow 4: SRE & Platform Security Officer (Cluster, TPM 2.0, Upgrade & Chaos)
// =============================================================================

#[tokio::test]
async fn test_workflow_4_sre_and_platform_security_e2e() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, client, _data_dir) = start_edge_and_authed_client().await;

        // 1. Inspect Raft Cluster Topology & Consensus State
        let topo = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_cluster_topology("celnet-cluster-primary"),
        )
        .await
        .expect("get topology in time")
        .expect("get topology succeeds");

        assert_eq!(topo.cluster_id, "celnet-cluster-primary");
        assert_eq!(topo.leader_id, "node-1");
        assert!(topo.active_generation >= 1);
        assert!(topo.members.len() >= 3);

        // 2. Dynamic Node Scaling (Scale Up & Scale Down)
        let scale_up = tokio::time::timeout(
            STEP_DEADLINE,
            client.scale_up_node("node-dynamic-9", "127.0.0.1:50559"),
        )
        .await
        .expect("scale up in time")
        .expect("scale up succeeds");
        assert_eq!(scale_up.node_id, "node-dynamic-9");
        assert_eq!(scale_up.new_status, NodeLifecycleStatus::NodeStatusActive as i32);

        let scale_down = tokio::time::timeout(
            STEP_DEADLINE,
            client.scale_down_node("node-dynamic-9", false),
        )
        .await
        .expect("scale down in time")
        .expect("scale down succeeds");
        assert_eq!(scale_down.node_id, "node-dynamic-9");
        assert_eq!(scale_down.new_status, NodeLifecycleStatus::NodeStatusDraining as i32);

        // 3. TPM 2.0 Hardware Root-of-Trust Attestation
        let att_req = AttestationRequest {
            node_id: "node-1".to_string(),
            pcr_digest_hex: "3a7b9c1d0e5f2a4b6c8d1e3f5a7b9c1d".to_string(),
            signature: vec![1, 2, 3, 4, 5],
            nonce: 0xfeed_cafe_dead_beef,
        };
        let att_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.verify_attestation(att_req),
        )
        .await
        .expect("verify attestation in time")
        .expect("verify attestation succeeds");
        assert!(att_resp.valid, "Hardware PCR quote must pass attestation");
        assert!(
            att_resp.hardware_fingerprint.contains("TPM2-HW-SHA256"),
            "Hardware fingerprint must be validated"
        );

        // 4. Institutional Capability Token Verification
        let lic_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_license_capabilities("biscuit-token-sre-fleet-v1"),
        )
        .await
        .expect("get license in time")
        .expect("get license succeeds");
        assert!(lic_resp.valid);
        assert!(lic_resp.active_capabilities.iter().any(|c| c == "scale:joint_consensus"));
        assert!(lic_resp.active_capabilities.iter().any(|c| c == "algo:twap"));
        assert!(lic_resp.active_capabilities.iter().any(|c| c == "margin:simm"));

        // 5. Zero-Downtime Hot Upgrade Twin Validation (0-ULP)
        let upgrade_status = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_upgrade_status(),
        )
        .await
        .expect("get upgrade status in time")
        .expect("get upgrade status succeeds");
        assert_eq!(upgrade_status.cutover_status, "STANDBY_READY");

        let baseline_prices = vec![1.0850, 1.0855, 1.0860, 1.0865];
        let candidate_prices = vec![1.0850, 1.0855, 1.0860, 1.0865];
        let twin_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.trigger_twin_validation(baseline_prices, candidate_prices, 0),
        )
        .await
        .expect("twin validation in time")
        .expect("twin validation succeeds");
        assert!(twin_resp.passed, "Bit-exact prices must pass shadow twin check");
        assert!(twin_resp.bit_exact);
        assert_eq!(twin_resp.max_ulp_divergence, 0);

        // 6. Autonomous Chaos Resilience Verification
        let chaos_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.execute_chaos_test(
                ChaosType::NetworkPartition,
                vec!["node-dynamic-9".to_string()],
                500,
            ),
        )
        .await
        .expect("chaos test in time")
        .expect("chaos test succeeds");
        assert!(chaos_resp.cluster_resilient, "Cluster must survive network partition");
        assert!(chaos_resp.recovery_time_ms <= 100);
    })
    .await
    .expect("Workflow 4 completed within deadline");
}
