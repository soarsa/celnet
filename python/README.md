# Celnet Python SDK

High-performance Python SDK for institutional pricing, clearing initial margin, algorithmic execution, distributed Raft cluster topology, and enterprise governance.

## Key Capabilities

- **Cross-Asset Valuation & Greeks**: Analytical Black-Scholes/Garman-Kohlhagen with full second/third order Greeks (Delta, Gamma, Vega, Theta, Rho, Vanna, Volga, Charm, Speed, Zomma, Color).
- **Clearing Initial Margin & Pre-Trade Simulation**: ISDA SIMM 2.7, Expected Shortfall (ES 97.5%), Value-at-Risk (VaR 99%), and Stress add-on calculations with what-if collateral headroom checks.
- **Algorithmic Order Execution**: Optimal liquidation (Almgren-Chriss) and TWAP order scheduling with Implementation Shortfall (IS) tracking.
- **Distributed Raft Cluster**: Cluster topology inspection, leader consensus health, and node lifecycle status.
- **Zero-Downtime Rolling Upgrade & Shadow Twin**: Hardware/software bit-exact 0-ULP verification before live cutover.
- **Enterprise Governance**: ISDA CDM 2026 digital trade event export, TPM 2.0 PCR hardware quote attestation, and Biscuit dynamic macaroon license token validation.
- **DataFrame Integration**: Native conversion of pricing rosters, margin breakdowns, and algo schedules into pandas DataFrames.

## Installation

```bash
pip install .
# with pandas dataframe support:
pip install .[dataframe]
```

## Quickstart

```python
from celnet import SyncCelnetClient, VanillaInstrument, CcyPair, OptionType

with SyncCelnetClient() as client:
    instrument = VanillaInstrument(
        pair=CcyPair(base="EUR", quote="USD"),
        tenor="1Y",
        expiry_years=1.0,
        option_type=OptionType.CALL,
        strike=1.0850,
        notional=1_000_000.0,
    )
    result = client.price_vanilla(instrument, spot=1.0850, vol=0.085)
    print(f"Mid Price: {result.mid_price:.5f}, Delta: {result.greeks.delta_spot:.4f}")
```
