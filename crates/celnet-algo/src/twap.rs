//! Time-Weighted Average Price (TWAP) Order Slicer.
#![deny(missing_docs)]

use crate::{AlgoError, PeggingStyle};

/// Configuration parameters for TWAP slicing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwapConfig {
    /// Total execution duration in seconds.
    pub duration_seconds: f64,
    /// Number of discrete child order slices.
    pub slice_count: usize,
    /// Randomized jitter factor in [0.0, 0.5] to prevent front-running.
    pub jitter_factor: f64,
    /// Order pegging behavior.
    pub pegging_style: PeggingStyle,
}

impl Default for TwapConfig {
    fn default() -> Self {
        Self {
            duration_seconds: 3600.0, // 1 hour
            slice_count: 20,          // 20 slices (every 3 minutes)
            jitter_factor: 0.15,      // +/- 15% random time jitter
            pegging_style: PeggingStyle::PassiveTouch,
        }
    }
}

/// A planned child slice in a TWAP schedule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwapSliceSchedule {
    /// Slice sequence index.
    pub index: usize,
    /// Planned execution time in seconds from schedule start.
    pub scheduled_offset_seconds: f64,
    /// Target quantity to execute in this slice.
    pub target_quantity: f64,
    /// Pegging style.
    pub pegging_style: PeggingStyle,
}

/// TWAP schedule generator.
pub struct TwapSlicer;

impl TwapSlicer {
    /// Generate a deterministic TWAP schedule for total quantity with jitter.
    pub fn build_schedule(
        total_quantity: f64,
        config: &TwapConfig,
    ) -> Result<Vec<TwapSliceSchedule>, AlgoError> {
        if config.slice_count == 0 {
            return Err(AlgoError::InvalidParameter("slice_count must be > 0".into()));
        }
        if config.duration_seconds <= 0.0 {
            return Err(AlgoError::InvalidParameter("duration_seconds must be > 0".into()));
        }

        let base_interval = config.duration_seconds / (config.slice_count as f64);
        let base_slice_qty = total_quantity / (config.slice_count as f64);

        let mut schedule = Vec::with_capacity(config.slice_count);
        let mut accumulated_qty = 0.0;

        for i in 0..config.slice_count {
            // Anti-front-running pseudo-random jitter based on sinusoidal hash
            let hash = libm::sin((i as f64 + 1.0) * 1.618) * 1000.0;
            let jitter_fraction = (hash - libm::floor(hash) - 0.5) * 2.0 * config.jitter_factor;
            let interval = base_interval * (1.0 + jitter_fraction);

            let offset = (i as f64) * base_interval + (interval - base_interval);
            let offset_clamped = offset.max(0.0).min(config.duration_seconds);

            let qty = if i == config.slice_count - 1 {
                total_quantity - accumulated_qty
            } else {
                base_slice_qty
            };
            accumulated_qty += qty;

            schedule.push(TwapSliceSchedule {
                index: i,
                scheduled_offset_seconds: offset_clamped,
                target_quantity: qty,
                pegging_style: config.pegging_style,
            });
        }

        Ok(schedule)
    }
}
