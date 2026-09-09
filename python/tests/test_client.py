import unittest
import asyncio
from celnet import (
    OptionType,
    CcyPair,
    VanillaInstrument,
    ClearedPosition,
    MarginCalculationRequest,
    PreTradeMarginRequest,
    PreTradeMarginOutcome,
    SubmitAlgoOrderRequest,
    AlgoStrategyType,
    CelnetClient,
    SyncCelnetClient,
    pricing_to_dataframe,
    margin_to_dataframe,
    algo_slices_to_dataframe,
)


class TestCelnetPythonSDK(unittest.TestCase):
    def setUp(self):
        self.client = SyncCelnetClient()

    def tearDown(self):
        self.client.close()

    def test_pricing_vanilla_and_greeks(self):
        instr = VanillaInstrument(
            pair=CcyPair(base="EUR", quote="USD"),
            tenor="1Y",
            expiry_years=1.0,
            option_type=OptionType.CALL,
            strike=1.0850,
            notional=1_000_000.0,
        )
        res = self.client.price_vanilla(
            instrument=instr,
            spot=1.0850,
            vol=0.085,
            r_dom=0.045,
            r_for=0.035,
        )
        self.assertGreater(res.mid_price, 0.0)
        self.assertGreater(res.greeks.delta_spot, 0.4)
        self.assertLess(res.greeks.delta_spot, 0.6)
        self.assertGreater(res.greeks.gamma, 0.0)
        self.assertGreater(res.greeks.vega, 0.0)

        # Test dataframe export
        df = pricing_to_dataframe([res])
        self.assertTrue(len(df) == 1)

    def test_margin_and_pretrade_simulation(self):
        # 1. Margin calculation
        req = MarginCalculationRequest(
            portfolio_id="PORTFOLIO-GLOBAL-1",
            confidence_level=0.99,
            lookback_days=500,
            positions=[
                ClearedPosition(symbol="EURUSD", notional=10_000_000, is_buy=True),
                ClearedPosition(symbol="USDJPY", notional=5_000_000, is_buy=False),
            ],
        )
        m_res = self.client.calculate_margin(req)
        self.assertGreater(m_res.total_initial_margin, 0.0)
        self.assertGreater(m_res.expected_shortfall, 0.0)
        self.assertEqual(m_res.currency, "USD")

        # Test margin dataframe
        df_margin = margin_to_dataframe(m_res)
        self.assertEqual(len(df_margin), 4)

        # 2. Pre-trade margin check
        pt_req = PreTradeMarginRequest(
            portfolio_id="PORTFOLIO-GLOBAL-1",
            candidate_position=ClearedPosition(symbol="GBPUSD", notional=2_000_000, is_buy=True),
            available_collateral=10_000_000.0,
            credit_line=1_000_000.0,
        )
        pt_res = self.client.simulate_pre_trade_margin(pt_req)
        self.assertEqual(pt_res.outcome, PreTradeMarginOutcome.APPROVED)
        self.assertGreater(pt_res.collateral_headroom, 0.0)

    def test_algorithmic_execution(self):
        algo_req = SubmitAlgoOrderRequest(
            symbol="EURUSD",
            total_quantity=5_000_000.0,
            arrival_price=1.0850,
            is_buy=True,
            strategy_type=AlgoStrategyType.TWAP,
            duration_seconds=300,
            slices_count=5,
        )
        order = self.client.submit_algo_order(algo_req)
        self.assertEqual(order.symbol, "EURUSD")
        self.assertEqual(len(order.slices), 5)
        self.assertEqual(order.slices[0].target_quantity, 1_000_000.0)

        # List algo orders
        orders = self.client.list_algo_orders()
        self.assertGreaterEqual(len(orders), 1)

        # Test algo schedule dataframe
        df_slices = algo_slices_to_dataframe(order)
        self.assertEqual(len(df_slices), 5)

    def test_cluster_topology_and_twin_upgrade(self):
        # 1. Raft cluster
        topo = self.client.get_cluster_topology()
        self.assertEqual(topo.cluster_id, "celnet-prod-us-east")
        self.assertEqual(len(topo.members), 3)
        self.assertEqual(topo.members[0].status.value, "ACTIVE")

        # 2. Upgrade status
        upgrade = self.client.get_upgrade_status()
        self.assertTrue(upgrade.twin_comparison_passed)
        self.assertEqual(upgrade.max_ulp_divergence, 0)
        self.assertEqual(upgrade.cutover_status, "READY_FOR_CUTOVER")

    def test_cdm_attestation_and_licensing(self):
        # 1. ISDA CDM 2026
        cdm = self.client.export_cdm(execution_id=884422)
        self.assertIn("UTI-2026-US00884422", cdm.uti)
        self.assertEqual(cdm.cdm_event_type, "TradeExecution")

        # 2. TPM 2.0 Attestation
        att = self.client.verify_attestation()
        self.assertTrue(att.valid)
        self.assertIn("PCR-SHA256", att.hardware_fingerprint)

        # 3. Biscuit License
        lic = self.client.get_license_capabilities()
        self.assertTrue(lic.valid)
        self.assertIn("PRICING_ADVANCED", lic.active_capabilities)
        self.assertIn("ISDA_SIMM_MARGIN", lic.active_capabilities)
        self.assertIn("ZERO_DOWNTIME_CLUSTER", lic.active_capabilities)


if __name__ == "__main__":
    unittest.main()
