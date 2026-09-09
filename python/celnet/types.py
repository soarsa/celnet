"""
Celnet Python SDK — Strongly Typed Domain Objects & Wire Contracts.
"""

from dataclasses import dataclass, field
from enum import Enum
from typing import List, Optional, Dict, Any


class AssetClass(str, Enum):
    FX = "FX"
    RATES = "RATES"
    EQUITIES = "EQUITIES"
    COMMODITIES = "COMMODITIES"
    CRYPTO = "CRYPTO"


class OptionType(str, Enum):
    CALL = "CALL"
    PUT = "PUT"


class Side(str, Enum):
    BUY = "BUY"
    SELL = "SELL"
    TWO_WAY = "TWO_WAY"


class BarrierKind(str, Enum):
    DOWN_AND_OUT = "DOWN_AND_OUT"
    DOWN_AND_IN = "DOWN_AND_IN"
    UP_AND_OUT = "UP_AND_OUT"
    UP_AND_IN = "UP_AND_IN"


class PreTradeMarginOutcome(str, Enum):
    APPROVED = "APPROVED"
    WARNING = "WARNING"
    EXCEEDS_COLLATERAL = "EXCEEDS_COLLATERAL"


class AlgoStrategyType(str, Enum):
    TWAP = "TWAP"
    VWAP = "VWAP"
    OPTIMAL_LIQUIDATION = "OPTIMAL_LIQUIDATION"
    POV = "POV"


class AlgoOrderStatus(str, Enum):
    PENDING = "PENDING"
    ACTIVE = "ACTIVE"
    COMPLETED = "COMPLETED"
    CANCELLED = "CANCELLED"


class ChildSliceStatus(str, Enum):
    PENDING = "PENDING"
    DISPATCHED = "DISPATCHED"
    FILLED = "FILLED"
    CANCELLED = "CANCELLED"


class NodeLifecycleStatus(str, Enum):
    PENDING = "PENDING"
    ACTIVE = "ACTIVE"
    DRAINING = "DRAINING"
    RETIRED = "RETIRED"
    FAILED = "FAILED"


@dataclass(frozen=True)
class CcyPair:
    base: str
    quote: str

    @classmethod
    def from_str(cls, pair_str: str) -> "CcyPair":
        s = pair_str.replace("/", "").strip().upper()
        if len(s) == 6:
            return cls(base=s[:3], quote=s[3:])
        raise ValueError(f"Invalid currency pair: {pair_str}")

    def __str__(self) -> str:
        return f"{self.base}{self.quote}"


@dataclass
class Greeks:
    price: float
    delta_spot: float
    delta_fwd: float
    gamma: float
    vega: float
    theta: float
    rho_dom: float
    rho_for: float
    vanna: float
    volga: float
    charm: float
    speed: float
    zomma: float
    color: float


@dataclass
class VanillaInstrument:
    pair: CcyPair
    tenor: str
    expiry_years: float
    option_type: OptionType
    strike: float
    notional: float
    side: Side = Side.TWO_WAY


@dataclass
class BarrierInstrument:
    pair: CcyPair
    tenor: str
    expiry_years: float
    option_type: OptionType
    strike: float
    barrier: float
    kind: BarrierKind
    notional: float
    upper_barrier: Optional[float] = None
    rebate: float = 0.0
    side: Side = Side.TWO_WAY


@dataclass
class PriceResult:
    instrument_name: str
    greeks: Greeks
    bid_price: float
    offer_price: float
    mid_price: float
    surface_version: int
    asset_class: AssetClass = AssetClass.FX


@dataclass
class RatesInstrument:
    symbol: str
    fixed_rate: float
    tenor_years: float
    notional: float
    is_payer: bool = True
    discount_rate: float = 0.040


@dataclass
class RatesPriceResult:
    instrument_name: str
    pv: float
    par_rate: float
    pv01: float
    dv01: float
    currency: str = "USD"
    asset_class: AssetClass = AssetClass.RATES


@dataclass
class EquityInstrument:
    symbol: str
    tenor: str
    expiry_years: float
    option_type: OptionType
    strike: float
    notional: float
    dividend_yield: float = 0.005
    side: Side = Side.TWO_WAY


@dataclass
class CommodityInstrument:
    symbol: str
    tenor: str
    expiry_years: float
    option_type: OptionType
    strike: float
    notional: float
    cost_of_carry: float = 0.02
    side: Side = Side.TWO_WAY


@dataclass
class CryptoInstrument:
    symbol: str
    tenor: str
    expiry_years: float
    option_type: OptionType
    strike: float
    notional: float
    funding_rate: float = 0.01
    barrier: Optional[float] = None
    barrier_kind: Optional[BarrierKind] = None
    side: Side = Side.TWO_WAY


@dataclass
class ClearedPosition:
    symbol: str
    notional: float
    is_buy: bool
    market_price: float = 0.0


@dataclass
class MarginCalculationRequest:
    portfolio_id: str
    confidence_level: float = 0.99
    lookback_days: int = 500
    positions: List[ClearedPosition] = field(default_factory=list)


@dataclass
class MarginCalculationResponse:
    portfolio_id: str
    total_initial_margin: float
    expected_shortfall: float
    value_at_risk: float
    stress_component: float
    currency: str
    calculated_epoch_nanos: int


@dataclass
class PreTradeMarginRequest:
    portfolio_id: str
    candidate_position: ClearedPosition
    available_collateral: float
    credit_line: float = 0.0
    confidence_level: float = 0.99


@dataclass
class PreTradeMarginResponse:
    portfolio_id: str
    outcome: PreTradeMarginOutcome
    initial_margin_before: float
    initial_margin_after: float
    delta_margin: float
    collateral_headroom: float
    reason: str


@dataclass
class SubmitAlgoOrderRequest:
    symbol: str
    total_quantity: float
    arrival_price: float
    is_buy: bool
    strategy_type: AlgoStrategyType = AlgoStrategyType.TWAP
    duration_seconds: int = 300
    slices_count: int = 5
    client_order_id: Optional[str] = None


@dataclass
class ChildSlice:
    slice_index: int
    scheduled_offset_seconds: int
    target_quantity: float
    filled_quantity: float
    avg_fill_price: float
    status: ChildSliceStatus


@dataclass
class AlgoOrderResponse:
    parent_order_id: str
    client_order_id: str
    symbol: str
    total_quantity: float
    executed_quantity: float
    arrival_price: float
    avg_exec_price: float
    is_buy: bool
    status: AlgoOrderStatus
    implementation_shortfall_bps: float
    slices: List[ChildSlice]
    created_epoch_nanos: int


@dataclass
class NodeMember:
    node_id: str
    endpoint: str
    status: NodeLifecycleStatus
    active_in_flight_trades: int
    joined_epoch_nanos: int


@dataclass
class ClusterTopologyResponse:
    cluster_id: str
    leader_id: str
    active_generation: int
    members: List[NodeMember]
    joint_consensus_active: bool


@dataclass
class UpgradeStatusResponse:
    active_generation: int
    current_version: str
    shadow_version: str
    twin_comparison_passed: bool
    max_ulp_divergence: int
    evaluated_trades_count: int
    cutover_status: str


@dataclass
class ExportCdmResponse:
    execution_id: int
    uti: str
    cdm_event_type: str
    cdm_json: str


@dataclass
class AttestationResponse:
    valid: bool
    attestation_timestamp_nanos: int
    hardware_fingerprint: str
    status_message: str


@dataclass
class LicenseCapabilityResponse:
    valid: bool
    subject: str
    tier: str
    active_capabilities: List[str]
    expiry_epoch_secs: int
