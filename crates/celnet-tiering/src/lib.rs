//! Celnet outbound price **tiering** — the pure, asset-agnostic engine that
//! margins and skews an outbound two-way from a composite mid/touch.
//!
//! We consolidate multiple LP feeds into a composite best bid/offer
//! (`celnet-aggregation`) and today publish it as-is. This crate constructs the
//! price we stream to **our** clients by **widening around mid** and/or
//! **skewing** — e.g. an LP composite of `99.50 / 99.60` (mid `99.55`) becomes,
//! under a flat `±25` price-bps markup, `99.30 / 99.80`.
//!
//! # Model
//!
//! The outbound two-way, following optimal inventory-control market making:
//!
//! ```text
//! bid   = mid − h − s      (h = half-spread ≥ 0,  s = skew, signed)
//! offer = mid + h − s
//! ```
//!
//! with two functionally distinct knobs: the **half-spread `h`** (fill frequency
//! vs profit; grows with vol and size, ~constant in inventory) and the **skew
//! `s`** (inventory risk; ~linear in the signed position `q`, clamped). A dealer
//! **long** inventory skews the whole two-way **down** (`s > 0`) to shed risk.
//! `docs/fixed-income/FI-TIERING-RESEARCH.md`. Real desks use clamped linear heuristics
//! rather than solving the HJB — that is what this engine implements.
//!
//! # Seam
//!
//! One [`TieringStrategy`] trait, N strategies composed additively by [`quote`].
//! Phase 1 ships [`FlatMarkup`] and [`InventorySkew`]; four more (vol scale, size
//! ladder, toxicity widen, per-client tier base) slot in behind the same trait.
//! [`Guardrails`] clamp the result and keep the book strictly two-sided.
//!
//! # Units
//!
//! Spread magnitudes carry an explicit [`SpreadUnit`] (price bps, yield bps,
//! price points, or percent) and are converted to an absolute price offset per
//! quote — yield bps via the bond's DV01/modified duration in the [`QuoteCtx`].
//!
//! Purely arithmetic and deterministic: no I/O, no server or wire dependency.

mod config;
mod context;
mod feature;
mod feature_pipeline;
mod pipeline;
mod strategy;
mod unit;

pub use config::{StrategySpec, TieringConfig};
pub use context::{Mid, QuoteCtx, SpreadSkew, TwoWay};
pub use feature::{AxeSide, FeatureKind, PricingCtx, PricingFeature};
pub use feature_pipeline::{FeaturePipeline, FeatureSpec, PricedResult};
pub use pipeline::{Guardrails, StalePolicy, SuppressReason, Suppressed, quote};
pub use strategy::{FlatMarkup, InventorySkew, ScaledSmoothedSpread, TieringStrategy, smooth};
pub use unit::{SpreadUnit, TieringError};
