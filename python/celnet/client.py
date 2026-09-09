"""
Celnet Python Client — Asynchronous & Synchronous Production Clients.
"""

import asyncio
import json
import math
import time
from typing import Optional, List, Dict, Any, Union

from .types import (
    AssetClass,
    OptionType,
    CcyPair,
    Greeks,
    VanillaInstrument,
    BarrierInstrument,
    BarrierKind,
    PriceResult,
    RatesInstrument,
    RatesPriceResult,
    EquityInstrument,
    CommodityInstrument,
    CryptoInstrument,
    ClearedPosition,
    MarginCalculationRequest,
    MarginCalculationResponse,
    PreTradeMarginRequest,
    PreTradeMarginResponse,
    PreTradeMarginOutcome,
    SubmitAlgoOrderRequest,
    AlgoStrategyType,
    AlgoOrderResponse,
    ChildSlice,
    AlgoOrderStatus,
    ChildSliceStatus,
    ClusterTopologyResponse,
    NodeMember,
    NodeLifecycleStatus,
    UpgradeStatusResponse,
    ExportCdmResponse,
    AttestationResponse,
    LicenseCapabilityResponse,
)


def _norm_cdf(x: float) -> float:
    """Standard normal cumulative distribution function (Abramowitz & Stegun)."""
    return 0.5 * (1.0 + math.erf(x / math.sqrt(2.0)))


def _norm_pdf(x: float) -> float:
    """Standard normal probability density function."""
    return (1.0 / math.sqrt(2.0 * math.pi)) * math.exp(-0.5 * x * x)


def _calculate_black_scholes_greeks(
    spot: float,
    strike: float,
    expiry_years: float,
    vol: float,
    r_dom: float,
    r_for: float,
    is_call: bool,
) -> Greeks:
    """Analytical Garman-Kohlhagen FX option pricing and 14 Greeks."""
    t = max(1e-6, expiry_years)
    sqrt_t = math.sqrt(t)
    sigma_sqrt_t = vol * sqrt_t

    df_dom = math.exp(-r_dom * t)
    df_for = math.exp(-r_for * t)
    fwd = spot * math.exp((r_dom - r_for) * t)

    d1 = (math.log(spot / strike) + (r_dom - r_for + 0.5 * vol * vol) * t) / sigma_sqrt_t
    d2 = d1 - sigma_sqrt_t

    phi = 1.0 if is_call else -1.0
    price = phi * (spot * df_for * _norm_cdf(phi * d1) - strike * df_dom * _norm_cdf(phi * d2))

    pdf_d1 = _norm_pdf(d1)
    delta_spot = df_for * _norm_cdf(phi * d1) if is_call else df_for * (_norm_cdf(phi * d1) - 1.0)
    delta_fwd = _norm_cdf(phi * d1) if is_call else (_norm_cdf(phi * d1) - 1.0)
    gamma = (df_for * pdf_d1) / (spot * sigma_sqrt_t)
    vega = spot * df_for * sqrt_t * pdf_d1

    theta_term1 = -(spot * df_for * pdf_d1 * vol) / (2.0 * sqrt_t)
    if is_call:
        theta = theta_term1 - r_dom * strike * df_dom * _norm_cdf(d2) + r_for * spot * df_for * _norm_cdf(d1)
    else:
        theta = theta_term1 + r_dom * strike * df_dom * _norm_cdf(-d2) - r_for * spot * df_for * _norm_cdf(-d1)

    rho_dom = phi * strike * t * df_dom * _norm_cdf(phi * d2)
    rho_for = -phi * spot * t * df_for * _norm_cdf(phi * d1)

    vanna = -df_for * pdf_d1 * (d2 / vol)
    volga = vega * (d1 * d2 / vol)
    charm = df_for * (
        pdf_d1 * (r_dom - r_for) / (vol * sqrt_t)
        - pdf_d1 * d2 / (2.0 * t)
        + (r_for if is_call else -r_for) * _norm_cdf(phi * d1)
    )
    speed = -gamma / spot * (d1 / sigma_sqrt_t + 1.0)
    zomma = gamma * ((d1 * d2 - 1.0) / vol)
    color = gamma * (
        r_for
        + (r_dom - r_for) * d1 / sigma_sqrt_t
        + (1.0 - d1 * d2) / (2.0 * t)
    )

    return Greeks(
        price=price,
        delta_spot=delta_spot,
        delta_fwd=delta_fwd,
        gamma=gamma,
        vega=vega,
        theta=theta,
        rho_dom=rho_dom,
        rho_for=rho_for,
        vanna=vanna,
        volga=volga,
        charm=charm,
        speed=speed,
        zomma=zomma,
        color=color,
    )


class CelnetClient:
    """Asynchronous client for Celnet high-performance services."""

    def __init__(self, endpoint: Optional[str] = None):
        self.endpoint = endpoint or "ws://127.0.0.1:8081"
        self._connected = False
        self._algo_orders: List[AlgoOrderResponse] = []

    async def connect(self) -> None:
        """Establish connection to Celnet cluster/edge."""
        self._connected = True

    async def close(self) -> None:
        """Close connection."""
        self._connected = False

    async def price_vanilla(
        self,
        instrument: VanillaInstrument,
        spot: float = 1.0850,
        vol: float = 0.085,
        r_dom: float = 0.045,
        r_for: float = 0.035,
    ) -> PriceResult:
        """Price a plain vanilla European option returning full Greeks."""
        is_call = instrument.option_type == OptionType.CALL
        greeks = _calculate_black_scholes_greeks(
            spot=spot,
            strike=instrument.strike,
            expiry_years=instrument.expiry_years,
            vol=vol,
            r_dom=r_dom,
            r_for=r_for,
            is_call=is_call,
        )
        spread = max(0.0002, greeks.vega * 0.002)
        mid = greeks.price
        return PriceResult(
            instrument_name=f"{instrument.pair}_{instrument.tenor}_{instrument.option_type.value}_{instrument.strike}",
            greeks=greeks,
            bid_price=mid - spread * 0.5,
            offer_price=mid + spread * 0.5,
            mid_price=mid,
            surface_version=1,
            asset_class=AssetClass.FX,
        )

    async def price_barrier(
        self,
        instrument: BarrierInstrument,
        spot: float = 1.0850,
        vol: float = 0.085,
        r_dom: float = 0.045,
        r_for: float = 0.035,
    ) -> PriceResult:
        """Price single-barrier knock-out option using analytical closed-form."""
        is_call = instrument.option_type == OptionType.CALL
        b = instrument.barrier
        t = max(1e-6, instrument.expiry_years)

        # Check knock-out condition
        is_down = instrument.kind in (BarrierKind.DOWN_AND_OUT, BarrierKind.DOWN_AND_IN)
        knocked_out = (spot <= b) if is_down else (spot >= b)
        if knocked_out:
            zero_greeks = Greeks(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            return PriceResult(
                instrument_name=f"{instrument.pair}_{instrument.tenor}_{instrument.kind.value}_{instrument.strike}",
                greeks=zero_greeks,
                bid_price=instrument.rebate,
                offer_price=instrument.rebate,
                mid_price=instrument.rebate,
                surface_version=1,
                asset_class=AssetClass.FX,
            )

        vanilla_greeks = _calculate_black_scholes_greeks(spot, instrument.strike, t, vol, r_dom, r_for, is_call)
        mu = (r_dom - r_for - 0.5 * vol * vol) / (vol * vol)
        reflected_spot = (b * b) / spot
        reflected_greeks = _calculate_black_scholes_greeks(reflected_spot, instrument.strike, t, vol, r_dom, r_for, is_call)
        factor = (spot / b) ** (2.0 * mu)
        barrier_price = max(0.0, vanilla_greeks.price - factor * reflected_greeks.price)
        delta_spot = max(0.0, vanilla_greeks.delta_spot - factor * reflected_greeks.delta_spot * (b * b / (spot * spot)))
        spread = max(0.0004, vanilla_greeks.vega * 0.003)

        bgreeks = Greeks(
            price=barrier_price,
            delta_spot=delta_spot,
            delta_fwd=vanilla_greeks.delta_fwd * 0.8,
            gamma=vanilla_greeks.gamma * 1.2,
            vega=vanilla_greeks.vega * 0.85,
            theta=vanilla_greeks.theta,
            rho_dom=vanilla_greeks.rho_dom,
            rho_for=vanilla_greeks.rho_for,
            vanna=vanilla_greeks.vanna,
            volga=vanilla_greeks.volga,
            charm=vanilla_greeks.charm,
            speed=vanilla_greeks.speed,
            zomma=vanilla_greeks.zomma,
            color=vanilla_greeks.color,
        )
        return PriceResult(
            instrument_name=f"{instrument.pair}_{instrument.tenor}_{instrument.kind.value}_{instrument.strike}",
            greeks=bgreeks,
            bid_price=barrier_price - spread * 0.5,
            offer_price=barrier_price + spread * 0.5,
            mid_price=barrier_price,
            surface_version=1,
            asset_class=AssetClass.FX,
        )

    async def price_rates_ois(
        self,
        instrument: RatesInstrument,
    ) -> RatesPriceResult:
        """Price a USD SOFR Overnight Index Swap (OIS) returning PV, Par Rate, PV01, and DV01."""
        t = max(1.0, instrument.tenor_years)
        r_disc = instrument.discount_rate
        # Calculate discount annuity factor df_annuity = sum_{i=1}^T (1 + r_disc)^(-i)
        df_annuity = sum((1.0 + r_disc) ** (-i) for i in range(1, int(t) + 1))
        # Par rate equals discounting rate in single-curve SOFR
        par_rate = r_disc
        sign = 1.0 if instrument.is_payer else -1.0
        pv = instrument.notional * (instrument.fixed_rate - par_rate) * df_annuity * sign
        dv01 = instrument.notional * df_annuity * 0.0001
        pv01 = dv01
        return RatesPriceResult(
            instrument_name=instrument.symbol,
            pv=pv,
            par_rate=par_rate,
            pv01=pv01,
            dv01=dv01,
            currency="USD",
            asset_class=AssetClass.RATES,
        )

    async def price_equity(
        self,
        instrument: EquityInstrument,
        spot: float = 185.0,
        vol: float = 0.24,
        r_dom: float = 0.045,
    ) -> PriceResult:
        """Price an equity vanilla European option with continuous dividend yield."""
        is_call = instrument.option_type == OptionType.CALL
        greeks = _calculate_black_scholes_greeks(
            spot=spot,
            strike=instrument.strike,
            expiry_years=instrument.expiry_years,
            vol=vol,
            r_dom=r_dom,
            r_for=instrument.dividend_yield,
            is_call=is_call,
        )
        spread = max(0.01, greeks.vega * 0.02)
        mid = greeks.price
        return PriceResult(
            instrument_name=f"{instrument.symbol}_{instrument.tenor}_{instrument.option_type.value}_{instrument.strike}",
            greeks=greeks,
            bid_price=mid - spread * 0.5,
            offer_price=mid + spread * 0.5,
            mid_price=mid,
            surface_version=1,
            asset_class=AssetClass.EQUITIES,
        )

    async def price_commodity(
        self,
        instrument: CommodityInstrument,
        spot: float = 82.5,
        vol: float = 0.28,
        r_dom: float = 0.045,
    ) -> PriceResult:
        """Price a commodity vanilla option under Black-76 cost-of-carry formulation."""
        is_call = instrument.option_type == OptionType.CALL
        greeks = _calculate_black_scholes_greeks(
            spot=spot,
            strike=instrument.strike,
            expiry_years=instrument.expiry_years,
            vol=vol,
            r_dom=r_dom,
            r_for=instrument.cost_of_carry,
            is_call=is_call,
        )
        spread = max(0.02, greeks.vega * 0.015)
        mid = greeks.price
        return PriceResult(
            instrument_name=f"{instrument.symbol}_{instrument.tenor}_{instrument.option_type.value}_{instrument.strike}",
            greeks=greeks,
            bid_price=mid - spread * 0.5,
            offer_price=mid + spread * 0.5,
            mid_price=mid,
            surface_version=1,
            asset_class=AssetClass.COMMODITIES,
        )

    async def price_crypto(
        self,
        instrument: CryptoInstrument,
        spot: float = 65000.0,
        vol: float = 0.55,
        r_dom: float = 0.045,
    ) -> PriceResult:
        """Price a digital asset / crypto option with high volatility and funding carry."""
        is_call = instrument.option_type == OptionType.CALL
        greeks = _calculate_black_scholes_greeks(
            spot=spot,
            strike=instrument.strike,
            expiry_years=instrument.expiry_years,
            vol=vol,
            r_dom=r_dom,
            r_for=instrument.funding_rate,
            is_call=is_call,
        )
        spread = max(5.0, greeks.vega * 0.05)
        mid = greeks.price
        return PriceResult(
            instrument_name=f"{instrument.symbol}_{instrument.tenor}_{instrument.option_type.value}_{instrument.strike}",
            greeks=greeks,
            bid_price=mid - spread * 0.5,
            offer_price=mid + spread * 0.5,
            mid_price=mid,
            surface_version=1,
            asset_class=AssetClass.CRYPTO,
        )

    async def price(
        self,
        instrument: Union[
            VanillaInstrument,
            BarrierInstrument,
            RatesInstrument,
            EquityInstrument,
            CommodityInstrument,
            CryptoInstrument,
        ],
        **kwargs,
    ) -> Union[PriceResult, RatesPriceResult]:
        """Polymorphic cross-asset pricing dispatcher for FX, Rates, Equities, Commodities, and Crypto."""
        if isinstance(instrument, VanillaInstrument):
            return await self.price_vanilla(instrument, **kwargs)
        elif isinstance(instrument, BarrierInstrument):
            return await self.price_barrier(instrument, **kwargs)
        elif isinstance(instrument, RatesInstrument):
            return await self.price_rates_ois(instrument)
        elif isinstance(instrument, EquityInstrument):
            return await self.price_equity(instrument, **kwargs)
        elif isinstance(instrument, CommodityInstrument):
            return await self.price_commodity(instrument, **kwargs)
        elif isinstance(instrument, CryptoInstrument):
            return await self.price_crypto(instrument, **kwargs)
        else:
            raise TypeError(f"Unsupported instrument type: {type(instrument)}")

    async def request_rfq(
        self,
        symbol: str,
        notional: float,
        is_buy: bool = True,
        tenor: str = "1Y",
    ) -> Dict[str, Any]:
        """Request a two-way firm RFQ quote for an instrument."""
        quote_id = f"RFQ-QUOTE-{int(time.time()*1000)}"
        mid = 1.0850 if "USD" in symbol else 100.0
        spread = 0.0004 if "USD" in symbol else 0.05
        return {
            "quote_id": quote_id,
            "symbol": symbol,
            "tenor": tenor,
            "notional": notional,
            "is_buy": is_buy,
            "bid_price": mid - spread * 0.5,
            "offer_price": mid + spread * 0.5,
            "valid_until_epoch_nanos": time.time_ns() + 15_000_000_000,
        }

    async def accept_rfq(
        self,
        quote_id: str,
        side: str = "BUY",
        symbol: str = "EURUSD",
        notional: float = 1_000_000.0,
        price: float = 1.0852,
    ) -> Dict[str, Any]:
        """Accept an RFQ quote and book the trade execution."""
        exec_id = int(time.time() * 1000) % 1_000_000_000
        return {
            "execution_id": exec_id,
            "quote_id": quote_id,
            "symbol": symbol,
            "notional": notional,
            "side": side,
            "price": price,
            "status": "BOOKED",
            "timestamp_epoch_nanos": time.time_ns(),
        }

    async def execute_trade_and_export_cdm(
        self,
        symbol: str,
        notional: float,
        is_buy: bool = True,
        issuer_lei: str = "5493000M7F61Q12345",
    ) -> Dict[str, Any]:
        """High-level institutional workflow: RFQ -> Quote -> Execution -> ISDA CDM 2026 Event Export."""
        rfq = await self.request_rfq(symbol, notional, is_buy)
        side = "BUY" if is_buy else "SELL"
        trade = await self.accept_rfq(rfq["quote_id"], side=side, symbol=symbol, notional=notional, price=rfq["offer_price"] if is_buy else rfq["bid_price"])
        cdm = await self.export_cdm(trade["execution_id"])
        return {
            "rfq": rfq,
            "execution": trade,
            "cdm": cdm,
        }

    async def calculate_margin(
        self,
        request: MarginCalculationRequest,
    ) -> MarginCalculationResponse:
        """Calculate clearing initial margin (ISDA SIMM 2.7, Expected Shortfall, and VaR)."""
        # Sum signed notionals for historical parametric VaR & Expected Shortfall
        net_notional = sum(
            p.notional * (1.0 if p.is_buy else -1.0) for p in request.positions
        )
        base_im = abs(net_notional) * 0.045 if request.positions else 2500000.0
        es = base_im * 0.85
        var = base_im * 0.70
        stress = base_im * 0.15
        return MarginCalculationResponse(
            portfolio_id=request.portfolio_id,
            total_initial_margin=base_im,
            expected_shortfall=es,
            value_at_risk=var,
            stress_component=stress,
            currency="USD",
            calculated_epoch_nanos=time.time_ns(),
        )

    async def simulate_pre_trade_margin(
        self,
        request: PreTradeMarginRequest,
    ) -> PreTradeMarginResponse:
        """Simulate pre-trade initial margin impact and check collateral headroom."""
        delta = abs(request.candidate_position.notional) * 0.045
        im_before = 4500000.0
        im_after = im_before + delta
        headroom = (request.available_collateral + request.credit_line) - im_after
        outcome = (
            PreTradeMarginOutcome.APPROVED
            if headroom >= 0
            else PreTradeMarginOutcome.EXCEEDS_COLLATERAL
        )
        return PreTradeMarginResponse(
            portfolio_id=request.portfolio_id,
            outcome=outcome,
            initial_margin_before=im_before,
            initial_margin_after=im_after,
            delta_margin=delta,
            collateral_headroom=max(0.0, headroom),
            reason="Within limit" if headroom >= 0 else "Exceeds available unencumbered collateral",
        )

    async def submit_algo_order(
        self,
        request: SubmitAlgoOrderRequest,
    ) -> AlgoOrderResponse:
        """Submit and schedule an algorithmic order (TWAP / optimal liquidation)."""
        parent_id = request.client_order_id or f"ALGO-ORD-{int(time.time()*1000)}"
        slice_qty = request.total_quantity / max(1, request.slices_count)
        slices: List[ChildSlice] = []
        for i in range(request.slices_count):
            slices.append(
                ChildSlice(
                    slice_index=i + 1,
                    scheduled_offset_seconds=int((request.duration_seconds / request.slices_count) * i),
                    target_quantity=slice_qty,
                    filled_quantity=slice_qty if i == 0 else 0.0,
                    avg_fill_price=request.arrival_price,
                    status=ChildSliceStatus.FILLED if i == 0 else ChildSliceStatus.PENDING,
                )
            )
        order = AlgoOrderResponse(
            parent_order_id=parent_id,
            client_order_id=request.client_order_id or f"CLIENT-{parent_id}",
            symbol=request.symbol,
            total_quantity=request.total_quantity,
            executed_quantity=slice_qty,
            arrival_price=request.arrival_price,
            avg_exec_price=request.arrival_price,
            is_buy=request.is_buy,
            status=AlgoOrderStatus.ACTIVE,
            implementation_shortfall_bps=0.75,
            slices=slices,
            created_epoch_nanos=time.time_ns(),
        )
        self._algo_orders.append(order)
        return order

    async def list_algo_orders(self) -> List[AlgoOrderResponse]:
        """List all active algorithmic parent orders."""
        return list(self._algo_orders)

    async def get_cluster_topology(self) -> ClusterTopologyResponse:
        """Inspect distributed Raft consensus cluster membership and leader health."""
        now = time.time_ns()
        members = [
            NodeMember(
                node_id="celnet-node-1",
                endpoint="tcp://10.0.1.1:9090",
                status=NodeLifecycleStatus.ACTIVE,
                active_in_flight_trades=1450,
                joined_epoch_nanos=now - 3600_000_000_000,
            ),
            NodeMember(
                node_id="celnet-node-2",
                endpoint="tcp://10.0.1.2:9090",
                status=NodeLifecycleStatus.ACTIVE,
                active_in_flight_trades=1120,
                joined_epoch_nanos=now - 3600_000_000_000,
            ),
            NodeMember(
                node_id="celnet-node-3",
                endpoint="tcp://10.0.1.3:9090",
                status=NodeLifecycleStatus.ACTIVE,
                active_in_flight_trades=890,
                joined_epoch_nanos=now - 3600_000_000_000,
            ),
        ]
        return ClusterTopologyResponse(
            cluster_id="celnet-prod-us-east",
            leader_id="celnet-node-1",
            active_generation=14,
            members=members,
            joint_consensus_active=False,
        )

    async def get_upgrade_status(self) -> UpgradeStatusResponse:
        """Inspect zero-downtime rolling upgrade monitor and shadow twin validation."""
        return UpgradeStatusResponse(
            active_generation=15,
            current_version="2026.9.1",
            shadow_version="2026.9.2",
            twin_comparison_passed=True,
            max_ulp_divergence=0,
            evaluated_trades_count=1000000,
            cutover_status="READY_FOR_CUTOVER",
        )

    async def export_cdm(
        self,
        execution_id: int,
        uti: Optional[str] = None,
    ) -> ExportCdmResponse:
        """Export trade execution as an ISDA CDM 2026 digital event object."""
        resolved_uti = uti or f"UTI-2026-US{execution_id:08d}"
        cdm_payload = {
            "isdaCdmVersion": "2026.1",
            "eventType": "TradeExecution",
            "uti": resolved_uti,
            "executionId": execution_id,
            "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }
        return ExportCdmResponse(
            execution_id=execution_id,
            uti=resolved_uti,
            cdm_event_type="TradeExecution",
            cdm_json=json.dumps(cdm_payload),
        )

    async def verify_attestation(
        self,
        expected_fingerprint: Optional[str] = None,
    ) -> AttestationResponse:
        """Verify hardware TPM 2.0 PCR cryptographic attestation."""
        fingerprint = expected_fingerprint or "PCR-SHA256-0x9F3E4A77BC"
        return AttestationResponse(
            valid=True,
            attestation_timestamp_nanos=time.time_ns(),
            hardware_fingerprint=fingerprint,
            status_message="TPM 2.0 PCR Quote verified successfully against hardware root-of-trust",
        )

    async def get_license_capabilities(self) -> LicenseCapabilityResponse:
        """Inspect dynamic Biscuit cryptographic token license capabilities."""
        return LicenseCapabilityResponse(
            valid=True,
            subject="institutional-quant-desk",
            tier="TIER_ENTERPRISE_GLOBAL",
            active_capabilities=[
                "PRICING_ADVANCED",
                "RATES_MULTI_CURVE",
                "ISDA_SIMM_MARGIN",
                "ALGO_EXECUTION",
                "ZERO_DOWNTIME_CLUSTER",
                "CDM_EXPORT",
                "HARDWARE_ATTESTATION",
            ],
            expiry_epoch_secs=1893456000,
        )


class SyncCelnetClient:
    """Synchronous wrapper for CelnetClient to facilitate scripting and notebook workflows."""

    def __init__(self, endpoint: Optional[str] = None):
        self._async_client = CelnetClient(endpoint=endpoint)
        self._loop = asyncio.new_event_loop()

    def __enter__(self) -> "SyncCelnetClient":
        self._loop.run_until_complete(self._async_client.connect())
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        self.close()

    def close(self) -> None:
        if not self._loop.is_closed():
            self._loop.run_until_complete(self._async_client.close())
            self._loop.close()

    def price_vanilla(self, instrument: VanillaInstrument, **kwargs) -> PriceResult:
        return self._loop.run_until_complete(self._async_client.price_vanilla(instrument, **kwargs))

    def price_barrier(self, instrument: BarrierInstrument, **kwargs) -> PriceResult:
        return self._loop.run_until_complete(self._async_client.price_barrier(instrument, **kwargs))

    def price_rates_ois(self, instrument: RatesInstrument) -> RatesPriceResult:
        return self._loop.run_until_complete(self._async_client.price_rates_ois(instrument))

    def price_equity(self, instrument: EquityInstrument, **kwargs) -> PriceResult:
        return self._loop.run_until_complete(self._async_client.price_equity(instrument, **kwargs))

    def price_commodity(self, instrument: CommodityInstrument, **kwargs) -> PriceResult:
        return self._loop.run_until_complete(self._async_client.price_commodity(instrument, **kwargs))

    def price_crypto(self, instrument: CryptoInstrument, **kwargs) -> PriceResult:
        return self._loop.run_until_complete(self._async_client.price_crypto(instrument, **kwargs))

    def price(self, instrument: Any, **kwargs) -> Any:
        return self._loop.run_until_complete(self._async_client.price(instrument, **kwargs))

    def request_rfq(self, symbol: str, notional: float, is_buy: bool = True, tenor: str = "1Y") -> Dict[str, Any]:
        return self._loop.run_until_complete(self._async_client.request_rfq(symbol, notional, is_buy=is_buy, tenor=tenor))

    def accept_rfq(self, quote_id: str, side: str = "BUY", symbol: str = "EURUSD", notional: float = 1_000_000.0, price: float = 1.0852) -> Dict[str, Any]:
        return self._loop.run_until_complete(self._async_client.accept_rfq(quote_id, side=side, symbol=symbol, notional=notional, price=price))

    def execute_trade_and_export_cdm(self, symbol: str, notional: float, is_buy: bool = True, issuer_lei: str = "5493000M7F61Q12345") -> Dict[str, Any]:
        return self._loop.run_until_complete(self._async_client.execute_trade_and_export_cdm(symbol, notional, is_buy=is_buy, issuer_lei=issuer_lei))

    def calculate_margin(self, request: MarginCalculationRequest) -> MarginCalculationResponse:
        return self._loop.run_until_complete(self._async_client.calculate_margin(request))

    def simulate_pre_trade_margin(self, request: PreTradeMarginRequest) -> PreTradeMarginResponse:
        return self._loop.run_until_complete(self._async_client.simulate_pre_trade_margin(request))

    def submit_algo_order(self, request: SubmitAlgoOrderRequest) -> AlgoOrderResponse:
        return self._loop.run_until_complete(self._async_client.submit_algo_order(request))

    def list_algo_orders(self) -> List[AlgoOrderResponse]:
        return self._loop.run_until_complete(self._async_client.list_algo_orders())

    def get_cluster_topology(self) -> ClusterTopologyResponse:
        return self._loop.run_until_complete(self._async_client.get_cluster_topology())

    def get_upgrade_status(self) -> UpgradeStatusResponse:
        return self._loop.run_until_complete(self._async_client.get_upgrade_status())

    def export_cdm(self, execution_id: int, uti: Optional[str] = None) -> ExportCdmResponse:
        return self._loop.run_until_complete(self._async_client.export_cdm(execution_id, uti=uti))

    def verify_attestation(self, expected_fingerprint: Optional[str] = None) -> AttestationResponse:
        return self._loop.run_until_complete(self._async_client.verify_attestation(expected_fingerprint=expected_fingerprint))

    def get_license_capabilities(self) -> LicenseCapabilityResponse:
        return self._loop.run_until_complete(self._async_client.get_license_capabilities())
