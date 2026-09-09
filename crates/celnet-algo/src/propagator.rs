//! Transient Market Impact & Order Flow Propagator Slicer (Bouchaud-Farmer-Lillo / Nutz & Voss, Aug 2026).
//!
//! Models execution under transient market impact where impact decays according to a power-law
//! or exponential propagator kernel G(τ):
//!
//! I(t) = ∑_{t_j < t} n_j * G(t - t_j)
//!
//! Incorporates the August 2026 stochastic tracking formulation where real-time Order Book
//! Imbalance (OBI) dynamically modulates child slice sizes to exploit passive liquidity and
//! mitigate adverse selection.
#![deny(missing_docs)]

use crate::AlgoError;

/// Kernel decay specification for transient market impact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropagatorKernelType {
    /// Exponential decay kernel: G(τ) = exp(-β * τ).
    Exponential {
        /// Half-life of price impact decay in seconds: β = ln(2) / half_life.
        half_life_seconds: f64,
    },
    /// Power-law decay kernel: G(τ) = (1 + τ / τ_0)^(-α), representing long memory in order flow.
    PowerLaw {
        /// Characteristic timescale τ_0 in seconds.
        tau_zero_seconds: f64,
        /// Power-law decay exponent α (typically 0.3 to 0.7 for equity/FX microstructure).
        alpha: f64,
    },
}

impl Default for PropagatorKernelType {
    fn default() -> Self {
        Self::Exponential {
            half_life_seconds: 120.0, // 2 minutes
        }
    }
}

/// Configuration parameters for Propagator Optimal Execution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropagatorExecutionConfig {
    /// Total trading horizon in seconds.
    pub horizon_seconds: f64,
    /// Number of trading intervals N.
    pub step_count: usize,
    /// Transient market impact scale parameter η > 0.
    pub impact_scale_eta: f64,
    /// Volatility of the asset per second.
    pub volatility_per_sec: f64,
    /// Risk aversion parameter λ >= 0.
    pub risk_aversion: f64,
    /// Propagator decay kernel specification.
    pub kernel: PropagatorKernelType,
    /// Sensitivity to real-time Order Book Imbalance (OBI ∈ [-1, 1]).
    /// Favorable imbalance accelerates execution, adverse imbalance decelerates.
    pub obi_sensitivity: f64,
}

impl Default for PropagatorExecutionConfig {
    fn default() -> Self {
        Self {
            horizon_seconds: 1800.0, // 30 minutes
            step_count: 10,
            impact_scale_eta: 1.0e-5,
            volatility_per_sec: 0.0002,
            risk_aversion: 1.0e-6,
            kernel: PropagatorKernelType::default(),
            obi_sensitivity: 0.25,
        }
    }
}

/// Dynamic volatility and liquidity regime profile across discrete trading steps.
#[derive(Debug, Clone, PartialEq)]
pub struct DynamicRegimeProfile {
    /// Step-by-step volatility vector σ_j per second (length equal to step_count, or empty for constant).
    pub step_volatilities: Vec<f64>,
    /// Volatility-to-impact scaling exponent γ: η_j = η_0 * (σ_j / σ_0)^γ. Typically 0.5 (square-root law).
    pub vol_impact_exponent: f64,
    /// Step-by-step power-law decay exponents α_j (empty to use base kernel exponent).
    pub step_decay_alphas: Vec<f64>,
    /// Step-by-step intraday turnover or liquidity volume weights w_j (e.g. U-shaped intraday volume curve).
    pub liquidity_time_weights: Vec<f64>,
}

impl Default for DynamicRegimeProfile {
    fn default() -> Self {
        Self {
            step_volatilities: Vec::new(),
            vol_impact_exponent: 0.5,
            step_decay_alphas: Vec::new(),
            liquidity_time_weights: Vec::new(),
        }
    }
}

/// Discretized trajectory step under transient propagator impact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropagatorStep {
    /// Step index (0 to N).
    pub step_index: usize,
    /// Time offset in seconds from start.
    pub time_seconds: f64,
    /// Remaining holdings x_j.
    pub remaining_holdings: f64,
    /// Baseline child slice size n_j before OBI conditioning.
    pub base_slice_size: f64,
    /// Conditioned child slice size accounting for order book imbalance.
    pub conditioned_slice_size: f64,
    /// Accumulated transient market impact at this step.
    pub accumulated_impact: f64,
}

/// Summary metrics of propagator execution trajectory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropagatorTrajectorySummary {
    /// Total expected market impact cost in currency units.
    pub total_expected_impact_cost: f64,
    /// Peak market impact experienced during execution.
    pub peak_market_impact: f64,
    /// Residual permanent impact remaining at horizon T.
    pub terminal_residual_impact: f64,
    /// Variance of execution cost.
    pub cost_variance: f64,
}

/// Propagator Optimal Execution Slicer.
pub struct PropagatorExecutionSlicer;

impl PropagatorExecutionSlicer {
    /// Evaluate propagator kernel value G(τ).
    #[inline]
    pub fn evaluate_kernel(tau_seconds: f64, kernel: &PropagatorKernelType) -> f64 {
        let t = tau_seconds.max(0.0);
        match kernel {
            PropagatorKernelType::Exponential { half_life_seconds } => {
                let beta = libm::log(2.0) / half_life_seconds.max(1.0e-3);
                libm::exp(-beta * t)
            }
            PropagatorKernelType::PowerLaw {
                tau_zero_seconds,
                alpha,
            } => {
                let base = 1.0 + t / tau_zero_seconds.max(1.0e-3);
                libm::pow(base, -alpha)
            }
        }
    }

    /// Compute optimal execution schedule under transient propagator impact with constant parameters.
    pub fn compute_trajectory(
        total_quantity: f64,
        observed_obi: f64,
        config: &PropagatorExecutionConfig,
    ) -> Result<(Vec<PropagatorStep>, PropagatorTrajectorySummary), AlgoError> {
        Self::compute_regime_modulated_trajectory(total_quantity, observed_obi, config, None)
    }

    /// Compute optimal execution schedule with dynamic intraday regime modulation:
    /// - Dynamic volatility-scaled market impact: η_j = η_0 * (σ_j / σ_0)^γ
    /// - Time-varying power-law propagator decay exponents α_j
    /// - Intraday liquidity-time acceleration via turnover weights
    pub fn compute_regime_modulated_trajectory(
        total_quantity: f64,
        observed_obi: f64,
        config: &PropagatorExecutionConfig,
        regime: Option<&DynamicRegimeProfile>,
    ) -> Result<(Vec<PropagatorStep>, PropagatorTrajectorySummary), AlgoError> {
        if config.step_count == 0 {
            return Err(AlgoError::InvalidParameter("step_count must be > 0".into()));
        }
        if config.horizon_seconds <= 0.0 {
            return Err(AlgoError::InvalidParameter("horizon_seconds must be > 0".into()));
        }
        if config.impact_scale_eta <= 0.0 {
            return Err(AlgoError::InvalidParameter("impact_scale_eta must be > 0".into()));
        }

        let n = config.step_count;
        let tau = config.horizon_seconds / (n as f64);
        let clamped_obi = observed_obi.clamp(-1.0, 1.0);

        // Extract or compute step-varying volatilities and impact scales
        let mut step_etas = Vec::with_capacity(n);
        let mut step_vols = Vec::with_capacity(n);
        let base_sigma = config.volatility_per_sec.max(1.0e-8);

        for j in 0..n {
            let vol = if let Some(r) = regime {
                if j < r.step_volatilities.len() && r.step_volatilities[j] > 0.0 {
                    r.step_volatilities[j]
                } else {
                    config.volatility_per_sec
                }
            } else {
                config.volatility_per_sec
            };
            step_vols.push(vol);

            let eta = if let Some(r) = regime {
                let ratio = vol / base_sigma;
                let scale = libm::pow(ratio, r.vol_impact_exponent);
                (config.impact_scale_eta * scale).clamp(config.impact_scale_eta * 0.05, config.impact_scale_eta * 20.0)
            } else {
                config.impact_scale_eta
            };
            step_etas.push(eta);
        }

        // Helper to evaluate step-specific kernel
        let eval_step_kernel = |tau_sec: f64, step_idx: usize| -> f64 {
            if let Some(r) = regime {
                if step_idx < r.step_decay_alphas.len() {
                    let tau_0 = match config.kernel {
                        PropagatorKernelType::PowerLaw { tau_zero_seconds, .. } => tau_zero_seconds,
                        _ => 10.0,
                    };
                    let alpha = r.step_decay_alphas[step_idx].max(0.01);
                    let base = 1.0 + tau_sec.max(0.0) / tau_0.max(1.0e-3);
                    return libm::pow(base, -alpha);
                }
            }
            Self::evaluate_kernel(tau_sec, &config.kernel)
        };

        // Compute baseline weights with risk aversion and liquidity-time weighting
        let mut raw_weights = Vec::with_capacity(n);
        for j in 0..n {
            let t_mid = ((j as f64) + 0.5) * tau;
            let decay_to_end = eval_step_kernel(config.horizon_seconds - t_mid, j);
            let risk_weight = 1.0 + config.risk_aversion * (config.horizon_seconds - t_mid);
            let liq_weight = if let Some(r) = regime {
                if j < r.liquidity_time_weights.len() && r.liquidity_time_weights[j] > 0.0 {
                    r.liquidity_time_weights[j]
                } else {
                    1.0
                }
            } else {
                1.0
            };
            let weight = (1.0 / (1.0 + decay_to_end * 0.5)) * risk_weight * liq_weight;
            raw_weights.push(weight);
        }

        let sum_raw: f64 = raw_weights.iter().sum();
        let normalized_weights: Vec<f64> = if sum_raw.abs() > 1e-12 {
            raw_weights.iter().map(|w| w / sum_raw).collect()
        } else {
            vec![1.0 / (n as f64); n]
        };

        let direction = if total_quantity >= 0.0 { 1.0 } else { -1.0 };
        let obi_factor = 1.0 + direction * clamped_obi * config.obi_sensitivity;

        let mut steps = Vec::with_capacity(n + 1);
        let mut remaining = total_quantity;
        let mut accumulated_impact: f64 = 0.0;
        let mut total_expected_impact_cost: f64 = 0.0;
        let mut peak_market_impact: f64 = 0.0;
        let mut cost_variance_sum: f64 = 0.0;

        // Step 0: Initial state
        steps.push(PropagatorStep {
            step_index: 0,
            time_seconds: 0.0,
            remaining_holdings: total_quantity,
            base_slice_size: 0.0,
            conditioned_slice_size: 0.0,
            accumulated_impact: 0.0,
        });

        // Compute slices
        let mut slices = Vec::with_capacity(n);
        for j in 0..n {
            let base_slice = total_quantity * normalized_weights[j];
            let conditioned_slice = if j < n / 2 {
                base_slice * obi_factor
            } else {
                base_slice * (2.0 - obi_factor).max(0.1)
            };
            slices.push((base_slice, conditioned_slice));
        }

        // Re-scale conditioned slices so their sum strictly matches total_quantity
        let sum_conditioned: f64 = slices.iter().map(|s| s.1).sum();
        let scale_correction = if sum_conditioned.abs() > 1e-12 {
            total_quantity / sum_conditioned
        } else {
            1.0
        };

        for j in 1..=n {
            let t_j = (j as f64) * tau;
            let (base_slice, raw_cond) = slices[j - 1];
            let conditioned_slice = raw_cond * scale_correction;

            remaining -= conditioned_slice;

            // Update accumulated transient impact from all past slices up to t_j using step-varying etas and kernels
            let mut current_step_impact = 0.0;
            for k in 1..=j {
                let (_, past_slice) = slices[k - 1];
                let actual_slice = past_slice * scale_correction;
                let dt = t_j - ((k as f64) - 0.5) * tau;
                let kernel_val = eval_step_kernel(dt, k - 1);
                current_step_impact += step_etas[k - 1] * actual_slice * kernel_val;
            }

            accumulated_impact = current_step_impact;
            if accumulated_impact.abs() > peak_market_impact.abs() {
                peak_market_impact = accumulated_impact;
            }

            // Expected impact cost for this slice: n_j * (I_j + 0.5 * eta_j * n_j)
            let eta_j = step_etas[j - 1];
            let slice_cost = conditioned_slice * (accumulated_impact + 0.5 * eta_j * conditioned_slice);
            total_expected_impact_cost += slice_cost.abs();

            let vol_j = step_vols[j - 1];
            cost_variance_sum += tau * remaining * remaining * vol_j * vol_j;

            steps.push(PropagatorStep {
                step_index: j,
                time_seconds: t_j,
                remaining_holdings: remaining,
                base_slice_size: base_slice,
                conditioned_slice_size: conditioned_slice,
                accumulated_impact,
            });
        }

        let summary = PropagatorTrajectorySummary {
            total_expected_impact_cost,
            peak_market_impact,
            terminal_residual_impact: accumulated_impact,
            cost_variance: cost_variance_sum,
        };

        Ok((steps, summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_propagator_exponential_and_power_law_kernels() {
        let exp_kernel = PropagatorKernelType::Exponential {
            half_life_seconds: 60.0,
        };
        assert_eq!(PropagatorExecutionSlicer::evaluate_kernel(0.0, &exp_kernel), 1.0);
        let val_60 = PropagatorExecutionSlicer::evaluate_kernel(60.0, &exp_kernel);
        assert!((val_60 - 0.5).abs() < 1e-6);

        let power_kernel = PropagatorKernelType::PowerLaw {
            tau_zero_seconds: 10.0,
            alpha: 0.5,
        };
        assert_eq!(PropagatorExecutionSlicer::evaluate_kernel(0.0, &power_kernel), 1.0);
        let val_30 = PropagatorExecutionSlicer::evaluate_kernel(30.0, &power_kernel);
        // (1 + 30/10)^(-0.5) = 4^(-0.5) = 0.5
        assert!((val_30 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_propagator_trajectory_exact_total_sum() {
        let config = PropagatorExecutionConfig {
            horizon_seconds: 600.0,
            step_count: 5,
            impact_scale_eta: 2.0e-5,
            volatility_per_sec: 0.0001,
            risk_aversion: 1.0e-6,
            kernel: PropagatorKernelType::Exponential {
                half_life_seconds: 120.0,
            },
            obi_sensitivity: 0.3,
        };

        // Long order with positive (favorable) order book imbalance
        let total_qty = 50_000.0;
        let (steps, summary) =
            PropagatorExecutionSlicer::compute_trajectory(total_qty, 0.4, &config).unwrap();

        assert_eq!(steps.len(), 6);
        let sum_conditioned: f64 = steps.iter().map(|s| s.conditioned_slice_size).sum();
        assert!((sum_conditioned - total_qty).abs() < 1e-5);
        assert!(summary.total_expected_impact_cost > 0.0);
        assert!(summary.peak_market_impact > 0.0);
        assert!(steps.last().unwrap().remaining_holdings.abs() < 1e-5);

        // Short order with negative (favorable for sell) order book imbalance
        let (steps_short, summary_short) =
            PropagatorExecutionSlicer::compute_trajectory(-50_000.0, -0.4, &config).unwrap();
        let sum_short: f64 = steps_short.iter().map(|s| s.conditioned_slice_size).sum();
        assert!((sum_short - (-total_qty)).abs() < 1e-5);
        assert!(summary_short.total_expected_impact_cost > 0.0);
    }

    #[test]
    fn test_propagator_dynamic_regime_modulation() {
        let config = PropagatorExecutionConfig {
            horizon_seconds: 600.0,
            step_count: 5,
            impact_scale_eta: 1.0e-5,
            volatility_per_sec: 0.0002,
            risk_aversion: 1.0e-6,
            kernel: PropagatorKernelType::PowerLaw {
                tau_zero_seconds: 10.0,
                alpha: 0.5,
            },
            obi_sensitivity: 0.2,
        };

        // Step-varying volatility spike in middle intervals: [0.0002, 0.0004, 0.0006, 0.0003, 0.0002]
        let regime = DynamicRegimeProfile {
            step_volatilities: vec![0.0002, 0.0004, 0.0006, 0.0003, 0.0002],
            vol_impact_exponent: 0.5,
            step_decay_alphas: vec![0.5, 0.4, 0.3, 0.5, 0.6],
            liquidity_time_weights: vec![1.2, 0.8, 0.7, 1.1, 1.4], // U-shaped liquidity volume
        };

        let total_qty = 100_000.0;
        let (steps, summary) = PropagatorExecutionSlicer::compute_regime_modulated_trajectory(
            total_qty,
            0.1,
            &config,
            Some(&regime),
        )
        .unwrap();

        assert_eq!(steps.len(), 6);
        let sum_conditioned: f64 = steps.iter().map(|s| s.conditioned_slice_size).sum();
        assert!((sum_conditioned - total_qty).abs() < 1e-4);
        assert!(summary.total_expected_impact_cost > 0.0);
        assert!(summary.peak_market_impact > 0.0);
        assert!(summary.cost_variance > 0.0);
        assert!(steps.last().unwrap().remaining_holdings.abs() < 1e-4);
    }
}
