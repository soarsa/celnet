//! Cross-Margining & Multi-Venue Initial Margin Optimizer (CCP FHS VaR & Bilateral ISDA SIMM 2.6).
//!
//! Provides simultaneous evaluation and capital optimization across:
//! 1. Cleared CCP Portfolio (Filtered Historical Simulation / SPAN 2).
//! 2. Bilateral OTC Portfolio (ISDA SIMM 2.6 Delta & Vega risk-weight sensitivity aggregation).
//! 3. Cross-Margining Offset Engine:
//!    - Correlated offset allocation between exchange-traded and OTC bilateral portfolios.
//!    - Net margin requirement computation with statutory correlation caps.
//!    - Trade novation / clearing allocation optimizer to minimize total initial margin.
#![deny(missing_docs)]

use serde::{Deserialize, Serialize};

use crate::fhs::{FhsMarginCalculator, FhsMarginConfig};
use crate::portfolio::MarginPortfolio;
use crate::{MarginBreakdown, MarginError};

/// Risk bucket under ISDA SIMM 2.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SimmRiskClass {
    /// Interest Rate (G10 currencies: USD, EUR, GBP, JPY).
    InterestRate,
    /// Foreign Exchange (developed & emerging currencies).
    Fx,
    /// Credit (Qualifying investment grade / high yield).
    Credit,
    /// Equity (large cap / small cap).
    Equity,
    /// Commodity.
    Commodity,
}

/// Sensitivity input for ISDA SIMM 2.6 calculation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimmSensitivity {
    /// Risk class.
    pub risk_class: SimmRiskClass,
    /// Tenor label or bucket (e.g. "1Y", "5Y", "10Y", "30Y").
    pub bucket: String,
    /// Net directional delta sensitivity (DV01 or currency units per 1bp/1% shift).
    pub delta_sensitivity: f64,
    /// Regulatory risk weight (e.g. 0.0075 for 5Y IR, 0.075 for FX).
    pub risk_weight: f64,
}

impl SimmSensitivity {
    /// Construct new SIMM sensitivity.
    pub fn new(risk_class: SimmRiskClass, bucket: &str, delta_sensitivity: f64, risk_weight: f64) -> Self {
        Self {
            risk_class,
            bucket: bucket.to_string(),
            delta_sensitivity,
            risk_weight,
        }
    }

    /// Weighted sensitivity: WS_k = s_k * RW_k.
    pub fn weighted_sensitivity(&self) -> f64 {
        self.delta_sensitivity * self.risk_weight
    }
}

/// Result of cross-margining optimization between CCP and Bilateral portfolios.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossMarginOptimizationResult {
    /// Initial margin required for cleared CCP portfolio (FHS VaR).
    pub ccp_margin: f64,
    /// Initial margin required for bilateral OTC portfolio (ISDA SIMM 2.6).
    pub bilateral_simm_margin: f64,
    /// Un-optimized standalone gross margin: CCP + Bilateral.
    pub standalone_gross_margin: f64,
    /// Net cross-margining requirement after correlation offset.
    pub optimized_net_margin: f64,
    /// Net capital savings / margin relief achieved through cross-margining.
    pub margin_relief_amount: f64,
    /// Percentage margin reduction (0.0 to 1.0).
    pub margin_reduction_ratio: f64,
}

/// Decision recommendation for clearing trade novation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NovationRecommendation {
    /// Whether moving trade to cleared CCP is margin-beneficial.
    pub should_clear: bool,
    /// Margin with trade held in bilateral OTC book.
    pub margin_if_bilateral: f64,
    /// Margin with trade cleared in CCP book.
    pub margin_if_cleared: f64,
    /// Net margin savings if cleared.
    pub savings_amount: f64,
}

/// Cross-Margining & Multi-Venue Initial Margin Optimizer.
pub struct CrossMarginOptimizer;

impl CrossMarginOptimizer {
    /// Compute Bilateral Initial Margin according to ISDA SIMM 2.6 methodology:
    ///
    /// K = sqrt( sum_i WS_i^2 + sum_{i != j} rho_{ij} * WS_i * WS_j )
    pub fn compute_simm_margin(
        sensitivities: &[SimmSensitivity],
        intra_bucket_correlation: f64,
    ) -> Result<f64, MarginError> {
        if sensitivities.is_empty() {
            return Ok(0.0);
        }
        let rho = intra_bucket_correlation.clamp(-1.0, 1.0);
        let ws: Vec<f64> = sensitivities.iter().map(|s| s.weighted_sensitivity()).collect();

        let mut variance = 0.0;
        let n = ws.len();
        for i in 0..n {
            variance += ws[i] * ws[i];
            for j in (i + 1)..n {
                // Adjacent or intra-class correlation
                variance += 2.0 * rho * ws[i] * ws[j];
            }
        }

        let margin = libm::sqrt(variance.max(0.0));
        Ok(margin)
    }

    /// Compute Cross-Margining Portfolio Requirement across Cleared CCP and Bilateral books:
    ///
    /// M_net = sqrt( M_ccp^2 + M_simm^2 - 2 * rho_cross * M_ccp * M_simm ) when hedging,
    /// bounded by statutory minimum haircuts.
    pub fn compute_cross_margin(
        ccp_breakdown: &MarginBreakdown,
        bilateral_simm_margin: f64,
        cross_venue_correlation: f64,
        is_hedging: bool,
    ) -> Result<CrossMarginOptimizationResult, MarginError> {
        let m_ccp = ccp_breakdown.total_margin.max(0.0);
        let m_simm = bilateral_simm_margin.max(0.0);
        let standalone_gross = m_ccp + m_simm;

        if standalone_gross == 0.0 {
            return Ok(CrossMarginOptimizationResult {
                ccp_margin: 0.0,
                bilateral_simm_margin: 0.0,
                standalone_gross_margin: 0.0,
                optimized_net_margin: 0.0,
                margin_relief_amount: 0.0,
                margin_reduction_ratio: 0.0,
            });
        }

        // Regulatory cap on cross-margining correlation offsets (Basel-IOSCO / CFTC standard 80%)
        let rho = cross_venue_correlation.clamp(0.0, 0.80);

        let cross_variance = if is_hedging {
            let var = m_ccp * m_ccp + m_simm * m_simm - 2.0 * rho * m_ccp * m_simm;
            let floor = (m_ccp - m_simm).abs();
            var.max(floor * floor)
        } else {
            m_ccp * m_ccp + m_simm * m_simm + 2.0 * rho * m_ccp * m_simm
        };

        let optimized_net = libm::sqrt(cross_variance.max(0.0));
        let relief = (standalone_gross - optimized_net).max(0.0);
        let ratio = relief / standalone_gross;

        Ok(CrossMarginOptimizationResult {
            ccp_margin: m_ccp,
            bilateral_simm_margin: m_simm,
            standalone_gross_margin: standalone_gross,
            optimized_net_margin: optimized_net,
            margin_relief_amount: relief,
            margin_reduction_ratio: ratio,
        })
    }

    /// Evaluate whether novating an incoming trade to cleared CCP is capital-optimal.
    pub fn evaluate_clearing_novation(
        ccp_portfolio: &MarginPortfolio,
        ccp_params: &FhsMarginConfig,
        simm_sensitivities: &[SimmSensitivity],
        candidate_simm_sensitivity: &SimmSensitivity,
        candidate_cleared_pnl: &[f64],
        candidate_quantity: f64,
        adv: f64,
        mark_price: f64,
        is_short_option: bool,
    ) -> Result<NovationRecommendation, MarginError> {
        // 1. Margin if trade stays bilateral
        let mut bilateral_with_trade = simm_sensitivities.to_vec();
        bilateral_with_trade.push(candidate_simm_sensitivity.clone());
        let simm_margin_bilateral = Self::compute_simm_margin(&bilateral_with_trade, 0.80)?;
        let ccp_margin_curr = FhsMarginCalculator::calculate_margin(ccp_portfolio, ccp_params)?.total_margin;
        let total_if_bilateral = ccp_margin_curr + simm_margin_bilateral;

        // 2. Margin if trade is novated to CCP
        let simm_margin_cleared = Self::compute_simm_margin(simm_sensitivities, 0.80)?;
        let mut ccp_with_trade = ccp_portfolio.clone();
        let candidate_pos = crate::portfolio::ClearedPosition::new(
            "NOVATED_CANDIDATE",
            crate::portfolio::MarginProductFamily::InterestRateSwap,
            candidate_quantity,
            1.0,
            adv,
            is_short_option,
            mark_price,
            candidate_cleared_pnl.to_vec(),
        );
        ccp_with_trade.apply_trade(candidate_pos);
        let ccp_margin_cleared = FhsMarginCalculator::calculate_margin(&ccp_with_trade, ccp_params)?.total_margin;
        let total_if_cleared = ccp_margin_cleared + simm_margin_cleared;

        let should_clear = total_if_cleared < total_if_bilateral;
        let savings = (total_if_bilateral - total_if_cleared).max(0.0);

        Ok(NovationRecommendation {
            should_clear,
            margin_if_bilateral: total_if_bilateral,
            margin_if_cleared: total_if_cleared,
            savings_amount: savings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isda_simm_margin_calculation() {
        let sensitivities = vec![
            SimmSensitivity::new(SimmRiskClass::InterestRate, "5Y", 10_000.0, 0.0075),
            SimmSensitivity::new(SimmRiskClass::InterestRate, "10Y", -8_000.0, 0.0075),
        ];

        let margin = CrossMarginOptimizer::compute_simm_margin(&sensitivities, 0.82).unwrap();
        assert!(margin > 0.0);
        // Hedged 5Y vs 10Y position should have margin less than standalone sum of absolute weighted sensitivities
        let sum_abs = 10_000.0 * 0.0075 + 8_000.0 * 0.0075;
        assert!(margin < sum_abs);
    }

    #[test]
    fn test_cross_margining_relief() {
        let mut ccp_breakdown = MarginBreakdown {
            core_market_risk: 100_000.0,
            liquidity_add_on: 10_000.0,
            concentration_charge: 5_000.0,
            basis_risk_add_on: 0.0,
            short_option_minimum: 0.0,
            total_margin: 0.0,
        };
        ccp_breakdown.compute_total();

        let bilateral_simm = 120_000.0;
        let result = CrossMarginOptimizer::compute_cross_margin(&ccp_breakdown, bilateral_simm, 0.75, true).unwrap();

        assert_eq!(result.ccp_margin, 115_000.0);
        assert_eq!(result.bilateral_simm_margin, 120_000.0);
        assert_eq!(result.standalone_gross_margin, 235_000.0);
        assert!(result.optimized_net_margin < result.standalone_gross_margin);
        assert!(result.margin_relief_amount > 0.0);
        assert!(result.margin_reduction_ratio > 0.10);
    }
}
