//! `celnet-margin` — Real-Time Clearing House Initial Margin Engine.
//!
//! Provides production-grade clearing initial margin simulation and pre-trade margin checking:
//!
//! 1. **Filtered Historical Simulation (FHS Margin)**:
//!    - Multi-scenario asset P&L evaluation across historical market shocks.
//!    - Volatility updating via EWMA/GARCH filtering.
//!    - Liquidity Risk Add-on (LRA) scaling with position size relative to ADV.
//!    - Concentration Risk Add-on (CRA).
//!    - Basis Risk Add-on (BRA) across correlated tenors.
//!    - Short Option Minimum (SOM) capital floor.
//! 2. **Scenario Grid Margin**:
//!    - Multi-curve stress scenarios (parallel shifts, steepeners, flatteners, butterflies).
//!    - Volatility surface shocks.
//!    - Cross-product correlation offsets with statutory haircut bounds.
//! 3. **Real-Time Pre-Trade Delta Margin**:
//!    - Sub-20 microsecond fast-path evaluation using pre-aggregated scenario vectors.
//!    - Pre-trade margin threshold and collateral limit validation.
//!
//! Grounded in quantitative clearing methodologies adhering to `#![forbid(unsafe_code)]`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod cross_margin;
pub mod fhs;
pub mod grid;
pub mod portfolio;
pub mod pre_trade;

use thiserror::Error;

/// Error conditions in clearing house margin calculations.
#[derive(Debug, Error, PartialEq, Clone)]
pub enum MarginError {
    /// Scenario count mismatch between portfolio and incoming trade.
    #[error("scenario count mismatch: expected {expected}, got {actual}")]
    ScenarioCountMismatch {
        /// Expected scenarios.
        expected: usize,
        /// Actual scenarios.
        actual: usize,
    },
    /// Position not found in portfolio.
    #[error("position not found: {0}")]
    PositionNotFound(String),
    /// Calculation parameter invalid (e.g. alpha outside (0, 1) or ADV <= 0).
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
}

/// Margin model methodology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MarginMethodology {
    /// Filtered Historical Simulation (SPAN 2 standard).
    FilteredHistoricalSimulation,
    /// Standardized Stress Scenario Grid (Prisma standard).
    ScenarioGrid,
    /// Hybrid conservative maximum of both models.
    ConservativeHybrid,
}

/// Detailed breakdown of initial margin requirements.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MarginBreakdown {
    /// Core market risk charge (VaR / Expected Shortfall).
    pub core_market_risk: f64,
    /// Liquidity risk add-on (cost of liquidating over MPOR).
    pub liquidity_add_on: f64,
    /// Concentration risk charge for outsized directional exposures.
    pub concentration_charge: f64,
    /// Basis risk charge between correlated products.
    pub basis_risk_add_on: f64,
    /// Short option minimum statutory floor charge.
    pub short_option_minimum: f64,
    /// Total initial margin required.
    pub total_margin: f64,
}

impl MarginBreakdown {
    /// Compute total margin by summing components.
    pub fn compute_total(&mut self) {
        self.total_margin = self.core_market_risk
            + self.liquidity_add_on
            + self.concentration_charge
            + self.basis_risk_add_on
            + self.short_option_minimum;
    }
}
