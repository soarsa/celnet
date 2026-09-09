//! Integration test verifying all aligned CelNet APIs and capabilities end-to-end.
//!
//! Validates:
//! 1. ValuationService (Polymorphic pricing)
//! 2. TradeService (ISDA CDM 2026 JSON export)
//! 3. MarginService (SPAN 2 FHS portfolio initial margin & pre-trade margin check)
//! 4. AlgoExecutionService (TWAP & Almgren-Chriss order submission, fill recording, tracking)
//! 5. ClusterService (Autonomous topology, scaling, zero-downtime hot upgrade twin validation, chaos resilience)
//! 6. AuthService (Hardware TPM 2.0 PCR attestation, Biscuit license capabilities)

mod common;

use celnet_client::{
    AlgoOrderStatus, AlgoPeggingStyle, AlgoStrategyType, AttestationRequest,
    ChaosType, ChildSliceStatus, ClearedPositionDto, MarginCalculationRequest,
    MarginProductFamily, NodeLifecycleStatus, PreTradeMarginOutcome,
    PreTradeMarginRequest, RecordAlgoFillRequest, SubmitAlgoOrderRequest,
    TwapConfigDto, ValuationRequest,
};

use common::{
    STEP_DEADLINE, TEST_DEADLINE, start_edge_and_authed_client,
};

#[tokio::test]
async fn test_all_apis_aligned_and_intuitive_e2e() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, client, _data_dir) = start_edge_and_authed_client().await;

        // =====================================================================
        // 1. ValuationService: Universal polymorphic pricing
        // =====================================================================
        let rates_req = celnet_proto::RatesPriceRequest {
            request_id: 2,
            curve_set: Some(celnet_proto::CurveSet {
                currency: "USD".to_owned(),
                reference_date: Some(celnet_proto::BrokenDate {
                    year: 2026,
                    month: 6,
                    day: 25,
                }),
                ois_pillars: vec![
                    celnet_proto::OisPillar {
                        tenor: Some(celnet_proto::PillarTenor {
                            point: Some(celnet_proto::pillar_tenor::Point::Years(1)),
                        }),
                        par_rate: 0.0420,
                    },
                    celnet_proto::OisPillar {
                        tenor: Some(celnet_proto::PillarTenor {
                            point: Some(celnet_proto::pillar_tenor::Point::Years(5)),
                        }),
                        par_rate: 0.0405,
                    },
                ],
            }),
            instrument: Some(celnet_proto::RatesInstrument {
                instrument: Some(celnet_proto::rates_instrument::Instrument::Ois(
                    celnet_proto::OisInstrument {
                        tenor_years: 5,
                        fixed_rate: 0.04,
                        notional: 10_000_000.0,
                        side: celnet_proto::Side::Buy as i32,
                    },
                )),
            }),
            correlation_id: Some(101),
        };
        let val_req = ValuationRequest {
            request_id: "VAL-REQ-001".to_string(),
            correlation_id: Some(101),
            payload: Some(celnet_proto::valuation_request::Payload::Rates(rates_req)),
        };
        let val_resp = tokio::time::timeout(STEP_DEADLINE, client.calculate(val_req))
            .await
            .expect("valuation calculate in time")
            .expect("valuation calculate succeeds");
        assert_eq!(val_resp.request_id, "VAL-REQ-001");
        assert_eq!(val_resp.correlation_id, Some(101));
        match val_resp.payload {
            Some(celnet_proto::valuation_response::Payload::Rates(rates_resp)) => {
                let result = rates_resp.result.expect("rates result present");
                assert!(result.par_rate > 0.0, "par rate is positive");
                assert!(result.dv01 != 0.0, "dv01 is non-zero");
            }
            _ => panic!("Expected Rates payload response from ValuationService"),
        }

        // =====================================================================
        // 2. TradeService: ISDA CDM 2026 digital trade export
        // =====================================================================
        let cdm_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.export_cdm(99001, "5493000M7F61Q12345", "CLIENT-DESK-A"),
        )
        .await
        .expect("export cdm in time")
        .expect("export cdm succeeds");

        assert_eq!(cdm_resp.execution_id, 99001);
        assert!(!cdm_resp.uti.is_empty(), "UTI must be populated");
        assert_eq!(cdm_resp.cdm_event_type, "Execution");
        assert!(
            cdm_resp.cdm_json.contains("TRD-CELNET-99001") && cdm_resp.cdm_json.contains("Execution"),
            "CDM JSON must represent valid ISDA execution event"
        );

        // =====================================================================
        // 3. MarginService: SPAN 2 / SIMM Portfolio Initial Margin & Pre-Trade
        // =====================================================================
        let scenarios = (0..200)
            .map(|i| ((i as f64) * 0.1).sin() * 2500.0)
            .collect();
        let margin_req = MarginCalculationRequest {
            portfolio_id: "PORTFOLIO-GLOBAL-MACRO".to_string(),
            positions: vec![ClearedPositionDto {
                symbol: "ZFZ26".to_string(),
                product_family: MarginProductFamily::BondFuture as i32,
                quantity: 20.0,
                contract_size: 100_000.0,
                initial_margin_per_contract: 2_500.0,
                is_short: false,
                current_price: 105.20,
                pnl_scenarios: scenarios,
            }],
            confidence_level: 0.99,
            lookback_days: 500,
        };
        let margin_resp = tokio::time::timeout(STEP_DEADLINE, client.calculate_margin(margin_req))
            .await
            .expect("calculate margin in time")
            .expect("calculate margin succeeds");

        assert_eq!(margin_resp.portfolio_id, "PORTFOLIO-GLOBAL-MACRO");
        assert!(margin_resp.total_initial_margin > 0.0, "Total margin must be positive");
        assert_eq!(margin_resp.currency, "USD");

        // Pre-trade simulation: Approved small trade
        let pre_trade_approved = PreTradeMarginRequest {
            portfolio_id: "PORTFOLIO-GLOBAL-MACRO".to_string(),
            existing_positions: vec![],
            candidate_position: Some(ClearedPositionDto {
                symbol: "ZFZ26".to_string(),
                product_family: MarginProductFamily::BondFuture as i32,
                quantity: 5.0,
                contract_size: 100_000.0,
                initial_margin_per_contract: 2_500.0,
                is_short: false,
                current_price: 105.20,
                pnl_scenarios: vec![-200.0, 100.0, -400.0, 300.0],
            }),
            available_collateral: 1_000_000.0,
            credit_line: 250_000.0,
            confidence_level: 0.99,
        };
        let pre_trade_ok = tokio::time::timeout(
            STEP_DEADLINE,
            client.simulate_pre_trade_margin(pre_trade_approved),
        )
        .await
        .expect("simulate pre trade in time")
        .expect("simulate pre trade succeeds");
        assert_eq!(pre_trade_ok.outcome, PreTradeMarginOutcome::Approved as i32);
        assert!(pre_trade_ok.delta_margin > 0.0);

        // Pre-trade simulation: Exceeds collateral outsized trade
        let pre_trade_breach = PreTradeMarginRequest {
            portfolio_id: "PORTFOLIO-GLOBAL-MACRO".to_string(),
            existing_positions: vec![],
            candidate_position: Some(ClearedPositionDto {
                symbol: "ZFZ26".to_string(),
                product_family: MarginProductFamily::BondFuture as i32,
                quantity: 1000.0, // Outsized order
                contract_size: 100_000.0,
                initial_margin_per_contract: 2_500.0,
                is_short: false,
                current_price: 105.20,
                pnl_scenarios: vec![-10_000_000.0, 5_000_000.0],
            }),
            available_collateral: 100.0, // Insufficient collateral
            credit_line: 0.0,
            confidence_level: 0.99,
        };
        let pre_trade_bad = tokio::time::timeout(
            STEP_DEADLINE,
            client.simulate_pre_trade_margin(pre_trade_breach),
        )
        .await
        .expect("simulate pre trade breach in time")
        .expect("simulate pre trade breach succeeds");
        assert_eq!(
            pre_trade_bad.outcome,
            PreTradeMarginOutcome::ExceedsCollateral as i32
        );

        // =====================================================================
        // 4. AlgoExecutionService: Algorithmic order slicing & fill tracking
        // =====================================================================
        let submit_req = SubmitAlgoOrderRequest {
            client_order_id: "ALGO-CLIENT-7788".to_string(),
            symbol: "ZFZ26".to_string(),
            total_quantity: 300.0,
            arrival_price: 105.10,
            is_buy: true,
            strategy_type: AlgoStrategyType::AlgoStrategyTwap as i32,
            twap: Some(TwapConfigDto {
                duration_seconds: 600.0,
                slice_count: 3,
                jitter_factor: 0.05,
                pegging_style: AlgoPeggingStyle::AlgoPeggingMidpoint as i32,
            }),
            optimal: None,
        };
        let algo_order = tokio::time::timeout(STEP_DEADLINE, client.submit_algo_order(submit_req))
            .await
            .expect("submit algo order in time")
            .expect("submit algo order succeeds");

        assert_eq!(algo_order.symbol, "ZFZ26");
        assert_eq!(algo_order.status, AlgoOrderStatus::Active as i32);
        assert_eq!(algo_order.slices.len(), 3);
        let parent_id = algo_order.parent_order_id.clone();

        // Get algo order
        let fetched_order = tokio::time::timeout(STEP_DEADLINE, client.get_algo_order(&parent_id))
            .await
            .expect("get algo order in time")
            .expect("get algo order succeeds");
        assert_eq!(fetched_order.parent_order_id, parent_id);

        // Record child slice fill
        let fill_req = RecordAlgoFillRequest {
            parent_order_id: parent_id.clone(),
            slice_index: 0,
            fill_quantity: 100.0,
            fill_price: 105.12,
        };
        let filled_order = tokio::time::timeout(STEP_DEADLINE, client.record_algo_fill(fill_req))
            .await
            .expect("record fill in time")
            .expect("record fill succeeds");
        assert_eq!(filled_order.executed_quantity, 100.0);
        assert_eq!(
            filled_order.slices[0].status,
            ChildSliceStatus::Filled as i32
        );

        // List orders
        let list_orders = tokio::time::timeout(STEP_DEADLINE, client.list_algo_orders("ZFZ"))
            .await
            .expect("list algo orders in time")
            .expect("list algo orders succeeds");
        assert!(!list_orders.orders.is_empty());

        // =====================================================================
        // 5. ClusterService: Scaling, Zero-Downtime Hot Upgrade & Chaos Testing
        // =====================================================================
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

        // Scale up node
        let scale_up = tokio::time::timeout(
            STEP_DEADLINE,
            client.scale_up_node("node-dynamic-4", "127.0.0.1:50554"),
        )
        .await
        .expect("scale up in time")
        .expect("scale up succeeds");
        assert_eq!(scale_up.node_id, "node-dynamic-4");
        assert_eq!(scale_up.new_status, NodeLifecycleStatus::NodeStatusActive as i32);

        // Scale down node
        let scale_down = tokio::time::timeout(
            STEP_DEADLINE,
            client.scale_down_node("node-dynamic-4", false),
        )
        .await
        .expect("scale down in time")
        .expect("scale down succeeds");
        assert_eq!(scale_down.node_id, "node-dynamic-4");
        assert_eq!(
            scale_down.new_status,
            NodeLifecycleStatus::NodeStatusDraining as i32
        );

        // Upgrade status
        let upgrade_status = tokio::time::timeout(STEP_DEADLINE, client.get_upgrade_status())
            .await
            .expect("get upgrade status in time")
            .expect("get upgrade status succeeds");
        assert_eq!(upgrade_status.cutover_status, "STANDBY_READY");
        assert!(upgrade_status.twin_comparison_passed);

        // Bit-exact shadow twin validation
        let baseline_prices = vec![105.10, 105.12, 105.15, 105.20];
        let candidate_prices = vec![105.10, 105.12, 105.15, 105.20];
        let twin_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.trigger_twin_validation(baseline_prices, candidate_prices, 0),
        )
        .await
        .expect("trigger twin validation in time")
        .expect("trigger twin validation succeeds");
        assert!(twin_resp.passed, "Bit-identical prices must pass twin validation");
        assert!(twin_resp.bit_exact);
        assert_eq!(twin_resp.max_ulp_divergence, 0);

        // Chaos resilience test
        let chaos_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.execute_chaos_test(
                ChaosType::NetworkPartition,
                vec!["node-dynamic-4".to_string()],
                500,
            ),
        )
        .await
        .expect("execute chaos test in time")
        .expect("execute chaos test succeeds");
        assert!(chaos_resp.cluster_resilient, "Cluster must survive network partition");
        assert!(chaos_resp.recovery_time_ms <= 100);

        // =====================================================================
        // 6. AuthService: Hardware TPM 2.0 PCR Quote Attestation & Capability Tokens
        // =====================================================================
        let att_req = AttestationRequest {
            node_id: "node-1".to_string(),
            pcr_digest_hex: "3a7b9c1d0e5f2a4b6c8d1e3f5a7b9c1d".to_string(),
            signature: vec![10, 20, 30, 40, 50],
            nonce: 0xfeed_face_cafe_babe,
        };
        let att_resp = tokio::time::timeout(STEP_DEADLINE, client.verify_attestation(att_req))
            .await
            .expect("verify attestation in time")
            .expect("verify attestation succeeds");
        assert!(att_resp.valid, "Valid PCR quote must pass attestation");
        assert!(
            att_resp.hardware_fingerprint.contains("TPM2-HW-SHA256"),
            "Hardware fingerprint must reflect TPM 2.0 PCR quote"
        );

        let lic_resp = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_license_capabilities("biscuit-token-tier-enterprise-v1"),
        )
        .await
        .expect("get license capabilities in time")
        .expect("get license capabilities succeeds");
        assert!(lic_resp.valid, "Valid license token must be accepted");
        assert!(
            lic_resp.active_capabilities.iter().any(|c| c == "algo:twap"),
            "Active capabilities must contain algo:twap"
        );
        assert!(
            lic_resp.active_capabilities.iter().any(|c| c == "margin:simm"),
            "Active capabilities must contain margin:simm"
        );
    })
    .await
    .expect("test completed within deadline");
}
