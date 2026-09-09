"""
Integration tests verifying all four institutional workflows across all five asset classes
(FX, Rates, Equities, Commodities, Crypto) using the Celnet Python SDK.
"""

import unittest
import json
from celnet import (
    AssetClass,
    OptionType,
    Side,
    BarrierKind,
    CcyPair,
    VanillaInstrument,
    BarrierInstrument,
    RatesInstrument,
    EquityInstrument,
    CommodityInstrument,
    CryptoInstrument,
    ClearedPosition,
    MarginCalculationRequest,
    PreTradeMarginRequest,
    PreTradeMarginOutcome,
    SubmitAlgoOrderRequest,
    AlgoStrategyType,
    AlgoOrderStatus,
    ChildSliceStatus,
    SyncCelnetClient,
    pricing_to_dataframe,
    margin_to_dataframe,
    algo_slices_to_dataframe,
)


class TestEndToEndUserWorkflows(unittest.TestCase):
    def setUp(self):
        self.client = SyncCelnetClient()

    def tearDown(self):
        self.client.close()

    def test_workflow_1_quant_trader_cross_asset_and_cdm(self):
        """
        Workflow 1: Quant Trader / PM
        - Cross-Asset Valuation (FX, Rates, Equities, Commodities, Crypto)
        - Greeks & Sensitivities
        - RFQ Two-Way Quoting & Trade Booking
        - Post-Trade ISDA CDM 2026 Event Export
        """
        # 1. FX Vanilla European Call
        fx_instr = VanillaInstrument(
            pair=CcyPair(base="EUR", quote="USD"),
            tenor="1Y",
            expiry_years=1.0,
            option_type=OptionType.CALL,
            strike=1.0850,
            notional=5_000_000.0,
        )
        fx_priced = self.client.price(fx_instr, spot=1.0850, vol=0.082)
        self.assertEqual(fx_priced.asset_class, AssetClass.FX)
        self.assertGreater(fx_priced.mid_price, 0.0)
        self.assertGreater(fx_priced.greeks.delta_spot, 0.4)
        self.assertGreater(fx_priced.greeks.vega, 0.0)

        # 2. Fixed Income: USD SOFR 5Y OIS Swap
        rates_instr = RatesInstrument(
            symbol="USD-SOFR-5Y",
            fixed_rate=0.040,
            tenor_years=5.0,
            notional=50_000_000.0,
            is_payer=True,
            discount_rate=0.042,
        )
        rates_priced = self.client.price(rates_instr)
        self.assertEqual(rates_priced.asset_class, AssetClass.RATES)
        self.assertEqual(rates_priced.par_rate, 0.042)
        self.assertGreater(rates_priced.dv01, 0.0)
        self.assertGreater(rates_priced.pv01, 0.0)

        # 3. Equities: AAPL 3M Vanilla Call
        eq_instr = EquityInstrument(
            symbol="AAPL",
            tenor="3M",
            expiry_years=0.25,
            option_type=OptionType.CALL,
            strike=190.0,
            notional=1_000_000.0,
            dividend_yield=0.005,
        )
        eq_priced = self.client.price(eq_instr, spot=185.0, vol=0.22)
        self.assertEqual(eq_priced.asset_class, AssetClass.EQUITIES)
        self.assertGreater(eq_priced.mid_price, 0.0)
        self.assertGreater(eq_priced.greeks.gamma, 0.0)

        # 4. Commodities: BRENT 6M Call
        comm_instr = CommodityInstrument(
            symbol="BRENT",
            tenor="6M",
            expiry_years=0.5,
            option_type=OptionType.CALL,
            strike=85.0,
            notional=2_000_000.0,
            cost_of_carry=0.025,
        )
        comm_priced = self.client.price(comm_instr, spot=82.5, vol=0.26)
        self.assertEqual(comm_priced.asset_class, AssetClass.COMMODITIES)
        self.assertGreater(comm_priced.mid_price, 0.0)
        self.assertGreater(comm_priced.greeks.vega, 0.0)

        # 5. Crypto: BTC/USDT 1M Down-and-Out Barrier Option
        crypto_instr = BarrierInstrument(
            pair=CcyPair(base="BTC", quote="USD"),
            tenor="1M",
            expiry_years=0.0833,
            option_type=OptionType.CALL,
            strike=66000.0,
            barrier=60000.0,
            kind=BarrierKind.DOWN_AND_OUT,
            notional=100.0,
        )
        crypto_priced = self.client.price(crypto_instr, spot=65000.0, vol=0.55)
        self.assertGreater(crypto_priced.mid_price, 0.0)
        self.assertGreater(crypto_priced.greeks.delta_spot, 0.0)

        # 6. Verify Pricing DataFrame conversion across all options
        df_pricing = pricing_to_dataframe([fx_priced, eq_priced, comm_priced, crypto_priced])
        self.assertEqual(len(df_pricing), 4)

        # 7. Execute complete RFQ -> Trade Booking -> ISDA CDM 2026 Export Workflow
        flow_result = self.client.execute_trade_and_export_cdm("EURUSD", 10_000_000.0, is_buy=True)
        self.assertIn("quote_id", flow_result["rfq"])
        self.assertEqual(flow_result["execution"]["status"], "BOOKED")
        self.assertEqual(flow_result["cdm"].cdm_event_type, "TradeExecution")
        self.assertTrue(flow_result["cdm"].uti.startswith("UTI-2026-US"))

        cdm_parsed = json.loads(flow_result["cdm"].cdm_json)
        self.assertEqual(cdm_parsed["isdaCdmVersion"], "2026.1")
        self.assertEqual(cdm_parsed["eventType"], "TradeExecution")

    def test_workflow_2_risk_and_clearing_manager(self):
        """
        Workflow 2: Risk & Clearing Manager
        - Cross-Asset Portfolio Aggregation
        - Portfolio Initial Margin (SIMM 2.7 / Expected Shortfall / VaR)
        - Pre-Trade What-If Margin Simulation
        - Collateral Headroom & Limit Enforcement
        """
        # 1. Multi-asset portfolio with positions in FX, Rates, Equities, Commodities, Crypto
        positions = [
            ClearedPosition(symbol="EURUSD-1Y-CALL", notional=10_000_000.0, is_buy=True),
            ClearedPosition(symbol="USD-SOFR-5Y-SWAP", notional=25_000_000.0, is_buy=False),
            ClearedPosition(symbol="AAPL-3M-CALL", notional=5_000_000.0, is_buy=True),
            ClearedPosition(symbol="BRENT-6M-CALL", notional=8_000_000.0, is_buy=True),
            ClearedPosition(symbol="BTCUSDT-PERP", notional=3_000_000.0, is_buy=False),
        ]
        margin_req = MarginCalculationRequest(
            portfolio_id="PORTFOLIO-CROSS-ASSET-GLOBAL",
            confidence_level=0.99,
            lookback_days=500,
            positions=positions,
        )
        margin_resp = self.client.calculate_margin(margin_req)
        self.assertEqual(margin_resp.portfolio_id, "PORTFOLIO-CROSS-ASSET-GLOBAL")
        self.assertGreater(margin_resp.total_initial_margin, 0.0)
        self.assertGreater(margin_resp.expected_shortfall, 0.0)
        self.assertGreater(margin_resp.value_at_risk, 0.0)
        self.assertEqual(margin_resp.currency, "USD")

        # Tabular DataFrame export
        df_margin = margin_to_dataframe(margin_resp)
        self.assertEqual(len(df_margin), 4)

        # 2. Pre-trade what-if simulation: Approved trade within collateral
        approved_req = PreTradeMarginRequest(
            portfolio_id="PORTFOLIO-CROSS-ASSET-GLOBAL",
            candidate_position=ClearedPosition(symbol="USDJPY-1M-FORWARD", notional=5_000_000.0, is_buy=True),
            available_collateral=20_000_000.0,
            credit_line=2_000_000.0,
        )
        approved_resp = self.client.simulate_pre_trade_margin(approved_req)
        self.assertEqual(approved_resp.outcome, PreTradeMarginOutcome.APPROVED)
        self.assertGreater(approved_resp.collateral_headroom, 0.0)
        self.assertGreater(approved_resp.delta_margin, 0.0)

        # 3. Pre-trade what-if simulation: Outsized trade exceeding collateral
        breach_req = PreTradeMarginRequest(
            portfolio_id="PORTFOLIO-CROSS-ASSET-GLOBAL",
            candidate_position=ClearedPosition(symbol="BRENT-1Y-SWAP", notional=500_000_000.0, is_buy=True),
            available_collateral=100_000.0,
            credit_line=0.0,
        )
        breach_resp = self.client.simulate_pre_trade_margin(breach_req)
        self.assertEqual(breach_resp.outcome, PreTradeMarginOutcome.EXCEEDS_COLLATERAL)
        self.assertEqual(breach_resp.collateral_headroom, 0.0)

    def test_workflow_3_algorithmic_execution_trader(self):
        """
        Workflow 3: Algorithmic Execution Trader
        - Large Block Parent Order Initialization
        - TWAP / Almgren-Chriss Slicing Schedule Generation
        - Venue Fill Tracking & Execution Status Transitions
        - Implementation Shortfall (IS bps) Transaction Cost Analysis
        """
        order_req = SubmitAlgoOrderRequest(
            symbol="ZFZ26",  # 5Y US Treasury Note Future
            total_quantity=20_000_000.0,
            arrival_price=105.15,
            is_buy=True,
            strategy_type=AlgoStrategyType.TWAP,
            duration_seconds=600,
            slices_count=5,
            client_order_id="ALGO-MACRO-TWAP-001",
        )
        order = self.client.submit_algo_order(order_req)
        self.assertEqual(order.client_order_id, "ALGO-MACRO-TWAP-001")
        self.assertEqual(order.symbol, "ZFZ26")
        self.assertEqual(order.status, AlgoOrderStatus.ACTIVE)
        self.assertEqual(len(order.slices), 5)
        self.assertEqual(order.slices[0].target_quantity, 4_000_000.0)
        self.assertEqual(order.slices[0].status, ChildSliceStatus.FILLED)
        self.assertEqual(order.slices[1].status, ChildSliceStatus.PENDING)

        # Check Implementation Shortfall
        self.assertLessEqual(order.implementation_shortfall_bps, 2.0)

        # DataFrame tabular export
        df_slices = algo_slices_to_dataframe(order)
        self.assertEqual(len(df_slices), 5)

    def test_workflow_4_sre_and_platform_security_officer(self):
        """
        Workflow 4: SRE & Platform Security Officer
        - Raft Consensus Cluster Health & Node Membership
        - TPM 2.0 PCR Hardware Attestation Root-of-Trust
        - Biscuit Cryptographic Capability Token Inspection
        - Zero-Downtime Shadow Twin 0-ULP Upgrade Verification
        """
        # 1. Cluster topology
        topo = self.client.get_cluster_topology()
        self.assertEqual(topo.cluster_id, "celnet-prod-us-east")
        self.assertEqual(topo.leader_id, "celnet-node-1")
        self.assertGreaterEqual(topo.active_generation, 1)
        self.assertEqual(len(topo.members), 3)

        # 2. Hardware attestation
        att = self.client.verify_attestation(expected_fingerprint="PCR-SHA256-0x9F3E4A77BC")
        self.assertTrue(att.valid)
        self.assertIn("PCR-SHA256", att.hardware_fingerprint)

        # 3. Biscuit License capabilities
        lic = self.client.get_license_capabilities()
        self.assertTrue(lic.valid)
        self.assertEqual(lic.tier, "TIER_ENTERPRISE_GLOBAL")
        self.assertIn("ISDA_SIMM_MARGIN", lic.active_capabilities)
        self.assertIn("ALGO_EXECUTION", lic.active_capabilities)
        self.assertIn("ZERO_DOWNTIME_CLUSTER", lic.active_capabilities)

        # 4. Shadow Twin Upgrade verification
        upgrade = self.client.get_upgrade_status()
        self.assertTrue(upgrade.twin_comparison_passed)
        self.assertEqual(upgrade.max_ulp_divergence, 0)
        self.assertEqual(upgrade.cutover_status, "READY_FOR_CUTOVER")


if __name__ == "__main__":
    unittest.main()
