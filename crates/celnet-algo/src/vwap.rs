//! Volume-Weighted Average Price (VWAP) Order Slicer.
#![deny(missing_docs)]

use crate::{AlgoError, PeggingStyle};

/// Configuration parameters for VWAP slicing.
#[derive(Debug, Clone, PartialEq)]
pub struct VwapConfig {
    /// Total duration in seconds.
    pub duration_seconds: f64,
    /// Historical intraday volume profile buckets (must sum to ~1.0).
    pub volume_profile: Vec<f64>,
    /// Pegging style.
    pub pegging_style: PeggingStyle,
    /// Pacing responsiveness multiplier [0.5, 2.0].
    pub pacing_multiplier: f64,
}

impl Default for VwapConfig {
    fn default() -> Self {
        // Standard U-shaped volume curve over 10 intervals
        let u_shape = vec![0.15, 0.11, 0.08, 0.07, 0.06, 0.06, 0.08, 0.10, 0.13, 0.16];
        Self {
            duration_seconds: 3600.0,
            volume_profile: u_shape,
            pegging_style: PeggingStyle::PassiveTouch,
            pacing_multiplier: 1.0,
        }
    }
}

/// A planned child slice in a VWAP schedule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VwapSliceSchedule {
    /// Slice index.
    pub index: usize,
    /// Target start time offset.
    pub scheduled_offset_seconds: f64,
    /// Target slice quantity.
    pub target_quantity: f64,
    /// Expected fraction of daily volume.
    pub profile_weight: f64,
}

/// VWAP schedule generator.
pub struct VwapSlicer;

impl VwapSlicer {
    /// Generate planned schedule based on volume profile.
    pub fn build_schedule(
        total_quantity: f64,
        config: &VwapConfig,
    ) -> Result<Vec<VwapSliceSchedule>, AlgoError> {
        let n = config.volume_profile.len();
        if n == 0 {
            return Err(AlgoError::InvalidParameter("volume profile empty".into()));
        }

        let sum_weight: f64 = config.volume_profile.iter().sum();
        if sum_weight <= 0.0 {
            return Err(AlgoError::InvalidParameter("volume profile sum must be > 0".into()));
        }

        let interval_sec = config.duration_seconds / (n as f64);
        let mut schedule = Vec::with_capacity(n);
        let mut accumulated_qty = 0.0;

        for (i, &raw_weight) in config.volume_profile.iter().enumerate() {
            let normalized_weight = raw_weight / sum_weight;
            let qty = if i == n - 1 {
                total_quantity - accumulated_qty
            } else {
                total_quantity * normalized_weight
            };
            accumulated_qty += qty;

            schedule.push(VwapSliceSchedule {
                index: i,
                scheduled_offset_seconds: (i as f64) * interval_sec,
                target_quantity: qty,
                profile_weight: normalized_weight,
            });
        }

        Ok(schedule)
    }

    /// Dynamically adjust current interval slice size based on realized market volume.
    pub fn adjust_for_pacing(
        target_qty: f64,
        realized_market_volume: f64,
        expected_market_volume: f64,
        pacing_multiplier: f64,
    ) -> f64 {
        if expected_market_volume <= 0.0 {
            return target_qty;
        }
        let pacing_ratio = realized_market_volume / expected_market_volume;
        // Clamp pacing between 0.5x and 2.0x
        let factor = (1.0 + (pacing_ratio - 1.0) * pacing_multiplier).max(0.5).min(2.0);
        target_qty * factor
    }
}
