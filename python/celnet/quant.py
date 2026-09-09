"""
Celnet Python SDK — Quantitative Analytics and DataFrame Conversion.
"""

from typing import List, Dict, Any, Union
from .types import PriceResult, MarginCalculationResponse, AlgoOrderResponse


def pricing_to_dataframe(results: List[PriceResult]) -> Any:
    """Convert a list of PriceResult objects to a pandas DataFrame (or list of dicts)."""
    rows = []
    for r in results:
        rows.append({
            "instrument": r.instrument_name,
            "mid_price": r.mid_price,
            "bid_price": r.bid_price,
            "offer_price": r.offer_price,
            "delta_spot": r.greeks.delta_spot,
            "delta_fwd": r.greeks.delta_fwd,
            "gamma": r.greeks.gamma,
            "vega": r.greeks.vega,
            "theta": r.greeks.theta,
            "rho_dom": r.greeks.rho_dom,
            "rho_for": r.greeks.rho_for,
            "vanna": r.greeks.vanna,
            "volga": r.greeks.volga,
            "charm": r.greeks.charm,
            "speed": r.greeks.speed,
            "zomma": r.greeks.zomma,
            "color": r.greeks.color,
        })
    try:
        import pandas as pd
        return pd.DataFrame(rows)
    except ImportError:
        return rows


def margin_to_dataframe(resp: MarginCalculationResponse) -> Any:
    """Convert MarginCalculationResponse to a pandas DataFrame (or list of dicts)."""
    rows = [
        {"metric": "Total Initial Margin", "value": resp.total_initial_margin, "currency": resp.currency},
        {"metric": "Expected Shortfall (ES 97.5%)", "value": resp.expected_shortfall, "currency": resp.currency},
        {"metric": "Value-at-Risk (VaR 99%)", "value": resp.value_at_risk, "currency": resp.currency},
        {"metric": "Stress Add-on", "value": resp.stress_component, "currency": resp.currency},
    ]
    try:
        import pandas as pd
        return pd.DataFrame(rows)
    except ImportError:
        return rows


def algo_slices_to_dataframe(order: AlgoOrderResponse) -> Any:
    """Convert AlgoOrderResponse child slices into a schedule DataFrame (or list of dicts)."""
    rows = []
    for s in order.slices:
        rows.append({
            "slice_index": s.slice_index,
            "scheduled_offset_seconds": s.scheduled_offset_seconds,
            "target_quantity": s.target_quantity,
            "filled_quantity": s.filled_quantity,
            "avg_fill_price": s.avg_fill_price,
            "status": s.status.value,
        })
    try:
        import pandas as pd
        return pd.DataFrame(rows)
    except ImportError:
        return rows
