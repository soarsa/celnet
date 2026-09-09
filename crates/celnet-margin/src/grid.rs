//! Standardized Stress Scenario Grid Margin Engine (Prisma standard).
#![deny(missing_docs)]

use crate::portfolio::MarginPortfolio;
use crate::{MarginBreakdown, MarginError};

/// Scenario Grid configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScenarioGridConfig {
    /// Parallel yield curve shock in basis points (e.g. 100.0 for 100 bps).
    pub parallel_shock_bps: f64,
    /// Curve twist (steepener / flattener) shock in basis points.
    pub twist_shock_bps: f64,
    /// Volatility relative stress shock (e.g. 0.20 for +20% vol).
    pub vol_stress_rel: f64,
    /// Statutory diversification haircut cap (e.g. 0.80 for 80% maximum offset).
    pub max_diversification_offset: f64,
}

impl Default for ScenarioGridConfig {
    fn default() -> Self {
        Self {
            parallel_shock_bps: 100.0,
            twist_shock_bps: 40.0,
            vol_stress_rel: 0.25,
            max_diversification_offset: 0.75,
        }
    }
}

/// Scenario Grid Margin Calculator.
pub struct ScenarioGridCalculator;

impl ScenarioGridCalculator {
    /// Calculate grid scenario initial margin.
    pub fn calculate_grid_margin(
        portfolio: &MarginPortfolio,
        _config: &ScenarioGridConfig,
    ) -> Result<MarginBreakdown, MarginError> {
        if portfolio.positions.is_empty() || portfolio.cached_aggregate_pnl.is_empty() {
            return Ok(MarginBreakdown {
                core_market_risk: 0.0,
                liquidity_add_on: 0.0,
                concentration_charge: 0.0,
                basis_risk_add_on: 0.0,
                short_option_minimum: 0.0,
                total_margin: 0.0,
            });
        }

        // Worst loss across predefined grid scenarios
        let mut max_loss = 0.0f64;
        for &pnl in &portfolio.cached_aggregate_pnl {
            if -pnl > max_loss {
                max_loss = -pnl;
            }
        }

        let mut breakdown = MarginBreakdown {
            core_market_risk: max_loss,
            liquidity_add_on: 0.0,
            concentration_charge: 0.0,
            basis_risk_add_on: 0.0,
            short_option_minimum: 0.0,
            total_margin: 0.0,
        };
        breakdown.compute_total();

        Ok(breakdown)
    }
}
