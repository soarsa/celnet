//! Margin Portfolio and Position Representation.
#![deny(missing_docs)]

use serde::{Deserialize, Serialize};

/// Product family for margin aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarginProductFamily {
    /// Interest Rate Swaps (IRS).
    InterestRateSwap,
    /// Government Bond Futures (e.g. 2Y, 5Y, 10Y, 30Y Treasury / Bund).
    BondFuture,
    /// Cash Government Bonds.
    CashBond,
    /// FX Vanilla Options.
    FxOption,
    /// Overnight Index Swaps (OIS).
    OisSwap,
}

/// A position in a cleared instrument for margin computation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClearedPosition {
    /// Contract identifier (e.g. "ZFZ26", "USD-SOFR-10Y").
    pub instrument_id: String,
    /// Product family.
    pub family: MarginProductFamily,
    /// Net position units (positive = long, negative = short).
    pub net_quantity: f64,
    /// Contract face or notional per unit.
    pub unit_notional: f64,
    /// 30-day average daily volume in contracts for liquidity scaling.
    pub average_daily_volume: f64,
    /// Whether this position is a short written option contract.
    pub is_short_option: bool,
    /// Base price per unit.
    pub mark_price: f64,
    /// Pre-evaluated scenario P&L vector per 1 unit position.
    pub unit_scenario_pnl: Vec<f64>,
}

impl ClearedPosition {
    /// Construct new cleared position.
    pub fn new(
        instrument_id: &str,
        family: MarginProductFamily,
        net_quantity: f64,
        unit_notional: f64,
        adv: f64,
        is_short_option: bool,
        mark_price: f64,
        unit_scenario_pnl: Vec<f64>,
    ) -> Self {
        Self {
            instrument_id: instrument_id.to_string(),
            family,
            net_quantity,
            unit_notional,
            average_daily_volume: adv.max(1.0),
            is_short_option,
            mark_price,
            unit_scenario_pnl,
        }
    }

    /// Gross notional exposure.
    pub fn gross_notional(&self) -> f64 {
        self.net_quantity.abs() * self.unit_notional
    }

    /// Compute total portfolio P&L contribution across all scenarios.
    pub fn scenario_pnl(&self) -> Vec<f64> {
        self.unit_scenario_pnl
            .iter()
            .map(|&p| p * self.net_quantity)
            .collect()
    }
}

/// Portfolio of cleared positions with cached aggregate scenario vectors.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MarginPortfolio {
    /// Portfolio identifier or account number.
    pub portfolio_id: String,
    /// List of member positions.
    pub positions: Vec<ClearedPosition>,
    /// Pre-cached aggregate scenario P&L vector (length = S).
    pub cached_aggregate_pnl: Vec<f64>,
}

impl MarginPortfolio {
    /// Construct empty portfolio.
    pub fn new(portfolio_id: &str) -> Self {
        Self {
            portfolio_id: portfolio_id.to_string(),
            positions: Vec::new(),
            cached_aggregate_pnl: Vec::new(),
        }
    }

    /// Add or update position (replaces existing position with same instrument_id) and refresh cached aggregate P&L vector.
    pub fn add_or_update(&mut self, pos: ClearedPosition) {
        if let Some(existing) = self.positions.iter_mut().find(|p| p.instrument_id == pos.instrument_id) {
            *existing = pos;
        } else {
            self.positions.push(pos);
        }
        self.rebuild_cache();
    }

    /// Apply an executed or candidate trade by adding net quantity to an existing position, or inserting a new position.
    pub fn apply_trade(&mut self, trade: ClearedPosition) {
        if let Some(existing) = self.positions.iter_mut().find(|p| p.instrument_id == trade.instrument_id) {
            existing.net_quantity += trade.net_quantity;
        } else {
            self.positions.push(trade);
        }
        self.rebuild_cache();
    }

    /// Rebuild cached aggregate scenario P&L vector across all positions.
    pub fn rebuild_cache(&mut self) {
        if self.positions.is_empty() {
            self.cached_aggregate_pnl.clear();
            return;
        }

        let num_scenarios = self.positions[0].unit_scenario_pnl.len();
        let mut agg = vec![0.0; num_scenarios];

        for pos in &self.positions {
            for (i, p) in pos.unit_scenario_pnl.iter().enumerate() {
                if i < num_scenarios {
                    agg[i] += p * pos.net_quantity;
                }
            }
        }
        self.cached_aggregate_pnl = agg;
    }
}
