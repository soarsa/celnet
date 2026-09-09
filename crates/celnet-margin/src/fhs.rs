//! Filtered Historical Simulation (FHS VaR) Initial Margin Engine (SPAN 2 standard).
#![deny(missing_docs)]

use crate::portfolio::MarginPortfolio;
use crate::{MarginBreakdown, MarginError};

/// Parameters governing FHS margin calculation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FhsMarginConfig {
    /// Confidence level (typically 0.99 for 99% VaR/ES).
    pub confidence_level: f64,
    /// Margin Period of Risk in days (2 days for ETD futures/options, 5 days for OTC IRS).
    pub mpor_days: f64,
    /// Liquidity add-on scaling coefficient.
    pub liquidity_scale: f64,
    /// Concentration threshold in gross notional.
    pub concentration_threshold: f64,
    /// Concentration quadratic charge rate.
    pub concentration_rate: f64,
    /// Short option minimum charge per contract.
    pub som_per_contract: f64,
}

impl Default for FhsMarginConfig {
    fn default() -> Self {
        Self {
            confidence_level: 0.99,
            mpor_days: 2.0,
            liquidity_scale: 0.05,
            concentration_threshold: 50_000_000.0, // $50M gross
            concentration_rate: 1.0e-9,
            som_per_contract: 50.0, // $50 per short option contract
        }
    }
}

/// Filtered Historical Simulation Margin Calculator.
pub struct FhsMarginCalculator;

impl FhsMarginCalculator {
    /// Calculate comprehensive initial margin requirement for a portfolio.
    pub fn calculate_margin(
        portfolio: &MarginPortfolio,
        config: &FhsMarginConfig,
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

        // 1. Core Market Risk: VaR / Expected Shortfall over scenario vector
        let mut pnl_copy = portfolio.cached_aggregate_pnl.clone();
        let tail = celnet_core::tail_var_es(&mut pnl_copy, config.confidence_level);
        // Scale by sqrt(MPOR / 1-day)
        let mpor_scale = libm::sqrt(config.mpor_days);
        let core_market_risk = tail.es * mpor_scale;

        // 2. Liquidity Risk Add-on (LRA)
        let mut liquidity_add_on = 0.0;
        for pos in &portfolio.positions {
            let participation = pos.net_quantity.abs() / pos.average_daily_volume;
            if participation > 0.05 {
                // Surcharge for taking >5% of ADV
                let impact = libm::sqrt(participation) * pos.gross_notional() * config.liquidity_scale;
                liquidity_add_on += impact;
            }
        }

        // 3. Concentration Charge
        let mut gross_notional = 0.0;
        for pos in &portfolio.positions {
            gross_notional += pos.gross_notional();
        }
        let excess_notional = (gross_notional - config.concentration_threshold).max(0.0);
        let concentration_charge = excess_notional * excess_notional * config.concentration_rate;

        // 4. Short Option Minimum (SOM)
        let mut short_contracts = 0.0;
        for pos in &portfolio.positions {
            if pos.is_short_option && pos.net_quantity < 0.0 {
                short_contracts += pos.net_quantity.abs();
            }
        }
        let short_option_minimum = short_contracts * config.som_per_contract;

        // 5. Basis Risk Add-on (cross-family correlation penalty)
        let mut irs_notional = 0.0;
        let mut future_notional = 0.0;
        for pos in &portfolio.positions {
            match pos.family {
                crate::portfolio::MarginProductFamily::InterestRateSwap => irs_notional += pos.net_quantity * pos.unit_notional,
                crate::portfolio::MarginProductFamily::BondFuture => future_notional += pos.net_quantity * pos.unit_notional,
                _ => {}
            }
        }
        // If holding both IRS and Futures offsetting, charge basis risk (e.g. 5 bps)
        let basis_risk_add_on = if irs_notional * future_notional < 0.0 {
            irs_notional.abs().min(future_notional.abs()) * 0.0005
        } else {
            0.0
        };

        let mut breakdown = MarginBreakdown {
            core_market_risk,
            liquidity_add_on,
            concentration_charge,
            basis_risk_add_on,
            short_option_minimum,
            total_margin: 0.0,
        };
        breakdown.compute_total();

        Ok(breakdown)
    }
}
