//! Percentage of Volume (POV) Order Slicer.
#![deny(missing_docs)]

use crate::AlgoError;

/// Configuration for POV execution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PovConfig {
    /// Target participation rate in (0.0, 0.5] (e.g. 0.10 for 10% of tape).
    pub target_participation_rate: f64,
    /// Maximum allowed single slice quantity.
    pub max_slice_quantity: f64,
    /// Minimum single slice quantity.
    pub min_slice_quantity: f64,
}

impl Default for PovConfig {
    fn default() -> Self {
        Self {
            target_participation_rate: 0.10, // 10%
            max_slice_quantity: 100_000.0,
            min_slice_quantity: 1_000.0,
        }
    }
}

/// POV calculation logic.
pub struct PovSlicer;

impl PovSlicer {
    /// Calculate child order size given observed market print volume.
    ///
    /// Formula:
    /// Q_child = min(Remaining, (alpha / (1 - alpha)) * V_market)
    pub fn calculate_child_slice(
        remaining_parent_qty: f64,
        observed_market_volume: f64,
        config: &PovConfig,
    ) -> Result<f64, AlgoError> {
        if config.target_participation_rate <= 0.0 || config.target_participation_rate >= 1.0 {
            return Err(AlgoError::InvalidParameter("participation rate must be in (0, 1)".into()));
        }

        if remaining_parent_qty <= 0.0 || observed_market_volume <= 0.0 {
            return Ok(0.0);
        }

        let ratio = config.target_participation_rate / (1.0 - config.target_participation_rate);
        let ideal_slice = observed_market_volume * ratio;

        let clamped_slice = ideal_slice
            .min(config.max_slice_quantity)
            .min(remaining_parent_qty);

        if clamped_slice < config.min_slice_quantity && clamped_slice < remaining_parent_qty {
            // Buffer until print volume accumulates past minimum threshold
            Ok(0.0)
        } else {
            Ok(clamped_slice)
        }
    }
}
