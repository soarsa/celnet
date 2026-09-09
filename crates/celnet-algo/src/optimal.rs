//! Almgren-Chriss (2000) Optimal Liquidation Trajectory Slicer.
//!
//! Computes the closed-form optimal liquidation schedule that minimizes
//! expected total execution cost while penalizing market risk variance:
//!
//! min E[x] + lambda * V[x]
#![deny(missing_docs)]

use crate::AlgoError;

/// Configuration parameters for Almgren-Chriss optimal execution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OptimalExecutionConfig {
    /// Total trading horizon in seconds.
    pub horizon_seconds: f64,
    /// Number of trading intervals N.
    pub step_count: usize,
    /// Annualized asset volatility sigma (e.g. 0.15 for 15%).
    pub volatility: f64,
    /// Trader risk aversion parameter lambda >= 0 (0 = risk-neutral VWAP/TWAP, >0 = risk-averse).
    pub risk_aversion: f64,
    /// Temporary market impact parameter eta > 0.
    pub temp_impact_eta: f64,
    /// Permanent market impact parameter gamma > 0.
    pub perm_impact_gamma: f64,
}

impl Default for OptimalExecutionConfig {
    fn default() -> Self {
        Self {
            horizon_seconds: 3600.0, // 1 hour
            step_count: 10,
            volatility: 0.15,
            risk_aversion: 1.0e-6,
            temp_impact_eta: 2.5e-6,
            perm_impact_gamma: 1.0e-7,
        }
    }
}

/// Almgren-Chriss optimal trajectory point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrajectoryStep {
    /// Step index (0 to N).
    pub step_index: usize,
    /// Time offset in seconds from start.
    pub time_seconds: f64,
    /// Planned remaining holdings x_j.
    pub remaining_holdings: f64,
    /// Child order trade slice size n_j = x_{j-1} - x_j.
    pub trade_slice_size: f64,
}

/// Summary metrics of optimal trajectory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OptimalTrajectorySummary {
    /// Urgency parameter kappa.
    pub kappa: f64,
    /// Expected total execution cost in dollars.
    pub expected_cost: f64,
    /// Variance of total execution cost.
    pub cost_variance: f64,
    /// Half-life of liquidation in seconds.
    pub half_life_seconds: f64,
}

/// Almgren-Chriss optimal liquidation solver.
pub struct OptimalExecutionSlicer;

impl OptimalExecutionSlicer {
    /// Solve for optimal liquidation trajectory x_j and trade slices n_j.
    pub fn compute_trajectory(
        total_quantity: f64,
        config: &OptimalExecutionConfig,
    ) -> Result<(Vec<TrajectoryStep>, OptimalTrajectorySummary), AlgoError> {
        if config.step_count == 0 {
            return Err(AlgoError::InvalidParameter("step_count must be > 0".into()));
        }
        if config.horizon_seconds <= 0.0 {
            return Err(AlgoError::InvalidParameter("horizon_seconds must be > 0".into()));
        }
        if config.temp_impact_eta <= 0.0 {
            return Err(AlgoError::InvalidParameter("temp_impact_eta must be > 0".into()));
        }

        let n = config.step_count;
        let tau = config.horizon_seconds / (n as f64);
        let t_total = config.horizon_seconds;

        // Urgency parameter kappa: cosh(kappa * tau) = 1 + 0.5 * (lambda * sigma^2 * tau^2) / eta
        let sigma2 = config.volatility * config.volatility;
        let numerator = config.risk_aversion * sigma2 * tau * tau;
        let cosh_val = 1.0 + 0.5 * (numerator / config.temp_impact_eta);
        let kappa = if cosh_val > 1.0000000001 {
            // arccosh(y) = ln(y + sqrt(y^2 - 1)) / tau
            let acosh = libm::log(cosh_val + libm::sqrt(cosh_val * cosh_val - 1.0));
            acosh / tau
        } else {
            // Near risk-neutral limit (kappa -> 0, linear schedule)
            1.0e-8
        };

        let sinh_kt = libm::sinh(kappa * t_total);

        let mut steps = Vec::with_capacity(n + 1);
        let mut prev_x = total_quantity;

        // Step 0: Initial position
        steps.push(TrajectoryStep {
            step_index: 0,
            time_seconds: 0.0,
            remaining_holdings: total_quantity,
            trade_slice_size: 0.0,
        });

        let mut expected_cost_temp = 0.0;
        let mut variance_sum = 0.0;

        for j in 1..=n {
            let t_j = (j as f64) * tau;
            let x_j = if kappa < 1.0e-6 {
                // Linear fallback
                total_quantity * (1.0 - (j as f64) / (n as f64))
            } else {
                let numer = libm::sinh(kappa * (t_total - t_j));
                total_quantity * (numer / sinh_kt)
            };

            let n_j = prev_x - x_j;
            expected_cost_temp += (n_j * n_j) / tau;
            variance_sum += tau * x_j * x_j;

            steps.push(TrajectoryStep {
                step_index: j,
                time_seconds: t_j,
                remaining_holdings: x_j,
                trade_slice_size: n_j,
            });

            prev_x = x_j;
        }

        // Expected cost: 0.5 * gamma * X^2 + eta * sum(n_j^2 / tau)
        let expected_cost = 0.5 * config.perm_impact_gamma * total_quantity * total_quantity
            + config.temp_impact_eta * expected_cost_temp;

        // Variance: sigma^2 * sum(tau * x_j^2)
        let cost_variance = sigma2 * variance_sum;

        // Half-life: ln(2) / kappa
        let half_life_seconds = libm::log(2.0) / kappa;

        let summary = OptimalTrajectorySummary {
            kappa,
            expected_cost,
            cost_variance,
            half_life_seconds,
        };

        Ok((steps, summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_almgren_chriss_positive_and_negative_liquidation() {
        let config = OptimalExecutionConfig {
            horizon_seconds: 1800.0,
            step_count: 6,
            volatility: 0.20,
            risk_aversion: 1.0e-5,
            temp_impact_eta: 1.0e-5,
            perm_impact_gamma: 1.0e-6,
        };

        // Long liquidation (positive qty)
        let (steps_pos, summary_pos) =
            OptimalExecutionSlicer::compute_trajectory(100_000.0, &config).unwrap();
        assert_eq!(steps_pos.len(), 7);
        let total_traded_pos: f64 = steps_pos.iter().map(|s| s.trade_slice_size).sum();
        assert!((total_traded_pos - 100_000.0).abs() < 1e-4);
        assert!(summary_pos.expected_cost > 0.0);
        assert!(summary_pos.cost_variance > 0.0);

        // Short covering / acquisition (negative qty)
        let (steps_neg, summary_neg) =
            OptimalExecutionSlicer::compute_trajectory(-100_000.0, &config).unwrap();
        assert_eq!(steps_neg.len(), 7);
        let total_traded_neg: f64 = steps_neg.iter().map(|s| s.trade_slice_size).sum();
        assert!((total_traded_neg - (-100_000.0)).abs() < 1e-4);
        // Costs are symmetric due to quadratic impact
        assert!((summary_neg.expected_cost - summary_pos.expected_cost).abs() < 1e-6);
        assert!((summary_neg.cost_variance - summary_pos.cost_variance).abs() < 1e-6);
    }
}
