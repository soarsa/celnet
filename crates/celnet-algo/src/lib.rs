//! `celnet-algo` — Institutional Algorithmic Order Execution Suite.
//!
//! Provides mathematically rigorous institutional order slicing and execution algorithms:
//!
//! 1. **TWAP Slicer (`twap`)**: Time-Weighted Average Price with customizable randomized
//!    anti-front-running timing jitter and passive/midpoint/aggressive pegging.
//! 2. **VWAP Slicer (`vwap`)**: Volume-Weighted Average Price using historical intraday
//!    volume distribution profiles and real-time participation pacing.
//! 3. **POV Slicer (`pov`)**: Percentage of Volume continuous inline participation tracking.
//! 4. **Optimal Liquidation Slicer (`optimal`)**: Closed-form Almgren-Chriss (2000) optimal
//!    execution trajectory balancing expected market impact against volatility risk variance.
//! 5. **Algo Engine (`engine`)**: Parent order lifecycle management, child slice tracking,
//!    and Implementation Shortfall / VWAP slippage transaction cost analysis (TCA).
//!
//! Grounded in quantitative market microstructure literature with `#![forbid(unsafe_code)]`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod engine;
pub mod optimal;
pub mod pov;
pub mod propagator;
pub mod strategy;
pub mod twap;
pub mod vwap;

use thiserror::Error;

/// Error conditions in algorithmic order execution.
#[derive(Debug, Error, PartialEq, Clone)]
pub enum AlgoError {
    /// Total duration or slice count invalid.
    #[error("invalid schedule parameter: {0}")]
    InvalidParameter(String),
    /// Order already completed or cancelled.
    #[error("order in terminal state: {0:?}")]
    TerminalState(String),
    /// Child slice index out of bounds.
    #[error("child slice not found: index {0}")]
    ChildSliceNotFound(usize),
}

/// Execution style / order pegging behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PeggingStyle {
    /// Post passively at touch (earning half-spread).
    PassiveTouch,
    /// Peg to midpoint of consolidated book.
    Midpoint,
    /// Cross the spread to execute aggressively.
    AggressiveSweep,
}

/// Strategy discriminator for algorithmic execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AlgoStrategyType {
    /// Time-Weighted Average Price.
    Twap,
    /// Volume-Weighted Average Price.
    Vwap,
    /// Percentage of Volume.
    Pov,
    /// Almgren-Chriss Optimal Liquidation.
    OptimalLiquidation,
    /// Transient Market Impact & Order Flow Propagator (August 2026 literature).
    Propagator,
    /// Pluggable user-defined or proprietary execution strategy.
    Custom,
}

pub use strategy::{
    AdaptiveSpreadStrategy, MarketBookSnapshot, PluggableExecutionStrategy,
    StrategyChildSlice, StrategyExecutionContext, StrategyFactory, StrategyRegistry,
};

