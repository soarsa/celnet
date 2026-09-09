//! Parent Order Lifecycle Engine and TCA Analytics.
#![deny(missing_docs)]

use crate::optimal::{OptimalExecutionConfig, OptimalExecutionSlicer};
use crate::twap::{TwapConfig, TwapSlicer};
use crate::vwap::{VwapConfig, VwapSlicer};
use crate::{AlgoError, AlgoStrategyType};

/// Status of an algorithmic parent order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ParentOrderStatus {
    /// Actively generating and executing child slices.
    Active,
    /// Paused by trader.
    Paused,
    /// Fully executed.
    Completed,
    /// Cancelled before full execution.
    Cancelled,
}

/// Child slice state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChildSliceStatus {
    /// Scheduled for future execution.
    Pending,
    /// Submitted to market / router.
    Routed,
    /// Partially filled.
    PartiallyFilled,
    /// Completely filled.
    Filled,
    /// Cancelled.
    Cancelled,
}

/// A dispatched child order slice.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChildSlice {
    /// Unique slice ID.
    pub slice_id: String,
    /// Target quantity.
    pub target_quantity: f64,
    /// Filled quantity.
    pub filled_quantity: f64,
    /// Average fill price.
    pub avg_fill_price: f64,
    /// Current slice status.
    pub status: ChildSliceStatus,
    /// Planned execution offset in seconds.
    pub scheduled_offset_sec: f64,
}

/// Algorithmic Parent Order tracking state.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AlgoParentOrder {
    /// Unique parent order identifier.
    pub order_id: String,
    /// Instrument symbol (e.g. "ZFZ26").
    pub instrument: String,
    /// Strategy type: TWAP, VWAP, POV, Optimal.
    pub strategy: AlgoStrategyType,
    /// Total requested parent quantity.
    pub total_quantity: f64,
    /// Cumulative executed quantity.
    pub executed_quantity: f64,
    /// Volume-weighted average execution price across all fills.
    pub avg_exec_price: f64,
    /// Market arrival price at order submission time.
    pub arrival_price: f64,
    /// Current parent order status.
    pub status: ParentOrderStatus,
    /// Child slices.
    pub slices: Vec<ChildSlice>,
}

impl AlgoParentOrder {
    /// Create and initialize a new TWAP parent order.
    pub fn new_twap(
        order_id: &str,
        instrument: &str,
        total_quantity: f64,
        arrival_price: f64,
        config: &TwapConfig,
    ) -> Result<Self, AlgoError> {
        let schedule = TwapSlicer::build_schedule(total_quantity, config)?;
        let slices = schedule
            .into_iter()
            .map(|s| ChildSlice {
                slice_id: format!("{}-SLICE-{}", order_id, s.index),
                target_quantity: s.target_quantity,
                filled_quantity: 0.0,
                avg_fill_price: 0.0,
                status: ChildSliceStatus::Pending,
                scheduled_offset_sec: s.scheduled_offset_seconds,
            })
            .collect();

        Ok(Self {
            order_id: order_id.to_string(),
            instrument: instrument.to_string(),
            strategy: AlgoStrategyType::Twap,
            total_quantity,
            executed_quantity: 0.0,
            avg_exec_price: 0.0,
            arrival_price,
            status: ParentOrderStatus::Active,
            slices,
        })
    }

    /// Create and initialize a new VWAP parent order.
    pub fn new_vwap(
        order_id: &str,
        instrument: &str,
        total_quantity: f64,
        arrival_price: f64,
        config: &VwapConfig,
    ) -> Result<Self, AlgoError> {
        let schedule = VwapSlicer::build_schedule(total_quantity, config)?;
        let slices = schedule
            .into_iter()
            .map(|s| ChildSlice {
                slice_id: format!("{}-VWAP-{}", order_id, s.index),
                target_quantity: s.target_quantity,
                filled_quantity: 0.0,
                avg_fill_price: 0.0,
                status: ChildSliceStatus::Pending,
                scheduled_offset_sec: s.scheduled_offset_seconds,
            })
            .collect();

        Ok(Self {
            order_id: order_id.to_string(),
            instrument: instrument.to_string(),
            strategy: AlgoStrategyType::Vwap,
            total_quantity,
            executed_quantity: 0.0,
            avg_exec_price: 0.0,
            arrival_price,
            status: ParentOrderStatus::Active,
            slices,
        })
    }

    /// Create and initialize a new Almgren-Chriss Optimal Liquidation parent order.
    pub fn new_optimal_liquidation(
        order_id: &str,
        instrument: &str,
        total_quantity: f64,
        arrival_price: f64,
        config: &OptimalExecutionConfig,
    ) -> Result<Self, AlgoError> {
        let (steps, _summary) = OptimalExecutionSlicer::compute_trajectory(total_quantity, config)?;
        let slices = steps
            .into_iter()
            .filter(|s| s.step_index > 0)
            .map(|s| ChildSlice {
                slice_id: format!("{}-OPT-{}", order_id, s.step_index),
                target_quantity: s.trade_slice_size,
                filled_quantity: 0.0,
                avg_fill_price: 0.0,
                status: ChildSliceStatus::Pending,
                scheduled_offset_sec: s.time_seconds,
            })
            .collect();

        Ok(Self {
            order_id: order_id.to_string(),
            instrument: instrument.to_string(),
            strategy: AlgoStrategyType::OptimalLiquidation,
            total_quantity,
            executed_quantity: 0.0,
            avg_exec_price: 0.0,
            arrival_price,
            status: ParentOrderStatus::Active,
            slices,
        })
    }

    /// Create and initialize an un-scheduled parent order driven by a dynamic pluggable strategy.
    pub fn new_pluggable(
        order_id: &str,
        instrument: &str,
        total_quantity: f64,
        arrival_price: f64,
    ) -> Result<Self, AlgoError> {
        if total_quantity <= 0.0 {
            return Err(AlgoError::InvalidParameter("total_quantity must be positive".to_string()));
        }
        if arrival_price <= 0.0 {
            return Err(AlgoError::InvalidParameter("arrival_price must be positive".to_string()));
        }

        Ok(Self {
            order_id: order_id.to_string(),
            instrument: instrument.to_string(),
            strategy: AlgoStrategyType::Custom,
            total_quantity,
            executed_quantity: 0.0,
            avg_exec_price: 0.0,
            arrival_price,
            status: ParentOrderStatus::Active,
            slices: Vec::new(),
        })
    }

    /// Step a pluggable strategy to generate the next child execution slice.
    ///
    /// Evaluates current order completion, populates strategy context with live market book state,
    /// and invokes [`PluggableExecutionStrategy::compute_slice`].
    pub fn step_strategy(
        &mut self,
        strategy: &mut dyn crate::PluggableExecutionStrategy,
        book: &crate::MarketBookSnapshot,
        elapsed_seconds: f64,
        total_duration_seconds: f64,
        historical_volume_fraction: f64,
    ) -> Result<Option<ChildSlice>, AlgoError> {
        if self.status != ParentOrderStatus::Active {
            return Err(AlgoError::TerminalState(format!("{:?}", self.status)));
        }

        let remaining_qty = (self.total_quantity - self.executed_quantity).max(0.0);
        if remaining_qty <= 0.0 {
            self.status = ParentOrderStatus::Completed;
            return Ok(None);
        }

        let ctx = crate::StrategyExecutionContext {
            total_order_qty: self.total_quantity,
            executed_qty: self.executed_quantity,
            remaining_qty,
            elapsed_seconds,
            total_duration_seconds,
            book,
            historical_volume_fraction,
        };

        match strategy.compute_slice(&ctx)? {
            Some(slice_decision) if slice_decision.quantity > 0.0 => {
                let slice_idx = self.slices.len();
                let child = ChildSlice {
                    slice_id: format!("{}-CUSTOM-{}", self.order_id, slice_idx),
                    target_quantity: slice_decision.quantity.min(remaining_qty),
                    filled_quantity: 0.0,
                    avg_fill_price: 0.0,
                    status: ChildSliceStatus::Pending,
                    scheduled_offset_sec: elapsed_seconds,
                };
                self.slices.push(child.clone());
                Ok(Some(child))
            }
            _ => Ok(None),
        }
    }

    /// Record a child fill report.
    pub fn record_fill(
        &mut self,
        slice_idx: usize,
        filled_qty: f64,
        fill_price: f64,
    ) -> Result<(), AlgoError> {
        if slice_idx >= self.slices.len() {
            return Err(AlgoError::ChildSliceNotFound(slice_idx));
        }

        let slice = &mut self.slices[slice_idx];
        slice.filled_quantity += filled_qty;
        slice.avg_fill_price = fill_price;
        if slice.filled_quantity >= slice.target_quantity {
            slice.status = ChildSliceStatus::Filled;
        } else {
            slice.status = ChildSliceStatus::PartiallyFilled;
        }

        // Update parent order totals
        let prev_notional = self.executed_quantity * self.avg_exec_price;
        let fill_notional = filled_qty * fill_price;
        self.executed_quantity += filled_qty;

        if self.executed_quantity > 0.0 {
            self.avg_exec_price = (prev_notional + fill_notional) / self.executed_quantity;
        }

        if self.executed_quantity >= self.total_quantity {
            self.status = ParentOrderStatus::Completed;
        }

        Ok(())
    }

    /// Implementation Shortfall (IS) in basis points relative to arrival price.
    /// Positive = slippage loss (bought higher or sold lower).
    pub fn implementation_shortfall_bps(&self, is_buy: bool) -> f64 {
        if self.arrival_price <= 0.0 || self.avg_exec_price <= 0.0 {
            return 0.0;
        }
        let diff = if is_buy {
            self.avg_exec_price - self.arrival_price
        } else {
            self.arrival_price - self.avg_exec_price
        };
        (diff / self.arrival_price) * 10_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdaptiveSpreadStrategy, MarketBookSnapshot};

    #[test]
    fn test_pluggable_order_lifecycle() {
        let mut order = AlgoParentOrder::new_pluggable("ORDER-101", "EURUSD", 100_000.0, 1.0850).unwrap();
        assert_eq!(order.strategy, AlgoStrategyType::Custom);
        assert_eq!(order.status, ParentOrderStatus::Active);
        assert_eq!(order.slices.len(), 0);

        let mut strategy = AdaptiveSpreadStrategy::new(1.0);
        let book = MarketBookSnapshot::new(1.0850, 50_000.0, 1.0852, 50_000.0);

        // Step strategy
        let slice = order.step_strategy(&mut strategy, &book, 0.0, 300.0, 0.10).unwrap().unwrap();
        assert_eq!(slice.target_quantity, 10_000.0);
        assert_eq!(order.slices.len(), 1);

        // Record fill
        order.record_fill(0, 10_000.0, 1.0851).unwrap();
        assert_eq!(order.executed_quantity, 10_000.0);
        assert_eq!(order.avg_exec_price, 1.0851);
        assert_eq!(order.slices[0].status, ChildSliceStatus::Filled);
    }
}
