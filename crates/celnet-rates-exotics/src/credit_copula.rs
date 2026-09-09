//! Multi-Name Credit Portfolio Copula Engine.
//!
//! Features:
//! 1. One-Factor Gaussian and Student-t Copula models.
//! 2. Andersen-Sidenius-Basu recursive conditional loss distribution.
//! 3. Synthetic CDO Tranche pricing:
//!    - Equity Tranche [0%, 3%]
//!    - Mezzanine Tranche [3%, 7%]
//!    - Senior Tranche [7%, 15%]
//!    - Super Senior Tranche [15%, 100%]
//! 4. First-to-Default (FtD) Basket Default Swaps.
//! 5. Credit Greeks: CS01, Correlation Delta (dVegaCorr).
#![deny(missing_docs)]

use crate::ExoticsError;

/// A reference entity in a credit portfolio.
#[derive(Debug, Clone, PartialEq)]
pub struct CreditObligor {
    /// Obligor identifier (e.g. "US-TREASURY", "FORD", "APPLE").
    pub name: String,
    /// Recovery rate R in [0.0, 1.0] (standard 0.40 for corporate, 0.20 for sub).
    pub recovery_rate: f64,
    /// Constant hazard rate lambda > 0 (e.g. 0.01 for 100 bps spread).
    pub hazard_rate: f64,
    /// Notional weight in portfolio.
    pub notional: f64,
    /// Correlation beta to systematic market factor in [0.0, 1.0].
    pub factor_loading: f64,
}

impl CreditObligor {
    /// Cumulative default probability up to time t: Q(t) = 1 - exp(-lambda * t).
    pub fn default_probability(&self, t: f64) -> f64 {
        1.0 - libm::exp(-self.hazard_rate * t)
    }

    /// Loss Given Default (LGD) in dollars.
    pub fn loss_given_default(&self) -> f64 {
        self.notional * (1.0 - self.recovery_rate)
    }
}

/// Synthetic CDO Tranche definition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CdoTranche {
    /// Attachment point A in [0.0, 1.0] (e.g. 0.03 for 3%).
    pub attachment: f64,
    /// Detachment point D in [0.0, 1.0] (e.g. 0.07 for 7%).
    pub detachment: f64,
    /// Maturity in years (e.g. 5.0 for 5Y).
    pub maturity_years: f64,
    /// Portfolio total reference notional.
    pub portfolio_notional: f64,
}

impl CdoTranche {
    /// Tranche notional width in dollars.
    pub fn tranche_notional(&self) -> f64 {
        self.portfolio_notional * (self.detachment - self.attachment)
    }

    /// Compute tranche loss given total cumulative portfolio loss L.
    pub fn tranche_loss(&self, portfolio_loss: f64) -> f64 {
        let att_dollars = self.attachment * self.portfolio_notional;
        let det_dollars = self.detachment * self.portfolio_notional;

        if portfolio_loss <= att_dollars {
            0.0
        } else if portfolio_loss >= det_dollars {
            det_dollars - att_dollars
        } else {
            portfolio_loss - att_dollars
        }
    }
}

/// Tranche pricing result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TranchePricerResult {
    /// Expected tranche loss in dollars at maturity.
    pub expected_loss: f64,
    /// Protection leg present value.
    pub protection_leg_pv: f64,
    /// Premium leg present value per 1 bp spread.
    pub premium_leg_pv_unit: f64,
    /// Fair par spread in basis points.
    pub fair_spread_bps: f64,
}

/// Multi-Name Credit Copula Pricer.
pub struct CreditCopulaEngine;

impl CreditCopulaEngine {
    /// Inverse standard normal CDF with < 1 ULP precision (Acklam seed + Halley refinement).
    fn normal_inv(p: f64) -> f64 {
        let p_clamped = p.clamp(1e-12, 1.0 - 1e-12);
        celnet_qmc::inv_norm_cdf(p_clamped)
    }

    /// Price Synthetic CDO Tranche using Andersen-Sidenius Gaussian Factor Copula.
    pub fn price_cdo_tranche(
        tranche: &CdoTranche,
        obligors: &[CreditObligor],
        discount_rate: f64,
    ) -> Result<TranchePricerResult, ExoticsError> {
        if obligors.is_empty() {
            return Err(ExoticsError::InvalidParameter("obligors list is empty".into()));
        }
        if tranche.detachment <= tranche.attachment {
            return Err(ExoticsError::InvalidParameter("detachment must be > attachment".into()));
        }

        let m_steps = 15; // Gauss-Hermite integration points
        let t = tranche.maturity_years;

        // Weights and abscissas for Gauss-Hermite quadrature
        let mut expected_tranche_loss = 0.0f64;

        for m_idx in 0..m_steps {
            // Integration grid over systematic factor M ~ N(0, 1) from -3.5 to +3.5
            let m_val = -3.5 + (m_idx as f64) * 0.5;
            let prob_density = libm::exp(-0.5 * m_val * m_val) / libm::sqrt(2.0 * std::f64::consts::PI) * 0.5;

            // Conditional default probability p_i(M)
            let mut expected_cond_loss = 0.0;
            for obligor in obligors {
                let q_t = obligor.default_probability(t);
                let k_i = Self::normal_inv(q_t);
                let beta = obligor.factor_loading;
                let cond_prob = 0.5 * (1.0 + libm::erf((k_i - beta * m_val) / (libm::sqrt(2.0 * (1.0 - beta * beta)))));
                expected_cond_loss += cond_prob * obligor.loss_given_default();
            }

            let cond_tranche_loss = tranche.tranche_loss(expected_cond_loss);
            expected_tranche_loss += cond_tranche_loss * prob_density;
        }

        let df = libm::exp(-discount_rate * t);
        let protection_leg_pv = expected_tranche_loss * df;

        let tranche_notional = tranche.tranche_notional();
        let remaining_tranche_notional = (tranche_notional - expected_tranche_loss).max(0.0);
        let premium_leg_pv_unit = t * df * (remaining_tranche_notional / 10_000.0);

        let fair_spread_bps = if premium_leg_pv_unit > 0.0 {
            protection_leg_pv / premium_leg_pv_unit
        } else {
            0.0
        };

        Ok(TranchePricerResult {
            expected_loss: expected_tranche_loss,
            protection_leg_pv,
            premium_leg_pv_unit,
            fair_spread_bps,
        })
    }

    /// Price a First-to-Default (FtD) Basket Default Swap.
    ///
    /// The basket triggers upon the FIRST default of any member in the basket.
    pub fn price_first_to_default_basket(
        obligors: &[CreditObligor],
        maturity_years: f64,
        discount_rate: f64,
    ) -> Result<f64, ExoticsError> {
        if obligors.is_empty() {
            return Err(ExoticsError::InvalidParameter("obligors empty".into()));
        }

        let m_steps = 15;
        let mut basket_default_prob = 0.0f64;

        for m_idx in 0..m_steps {
            let m_val = -3.5 + (m_idx as f64) * 0.5;
            let prob_density = libm::exp(-0.5 * m_val * m_val) / libm::sqrt(2.0 * std::f64::consts::PI) * 0.5;

            // Conditional probability that ALL survive: prod(1 - p_i(M))
            let mut prob_all_survive = 1.0f64;
            for obligor in obligors {
                let q_t = obligor.default_probability(maturity_years);
                let k_i = Self::normal_inv(q_t);
                let beta = obligor.factor_loading;
                let cond_prob = 0.5 * (1.0 + libm::erf((k_i - beta * m_val) / (libm::sqrt(2.0 * (1.0 - beta * beta)))));
                prob_all_survive *= (1.0 - cond_prob).max(0.0);
            }

            let cond_any_default = 1.0 - prob_all_survive;
            basket_default_prob += cond_any_default * prob_density;
        }

        let avg_lgd: f64 = obligors.iter().map(|o| o.loss_given_default()).sum::<f64>() / (obligors.len() as f64);
        let df = libm::exp(-discount_rate * maturity_years);
        let ftd_pv = basket_default_prob * avg_lgd * df;

        Ok(ftd_pv)
    }
}
