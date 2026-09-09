//! Spread units and their conversion to an absolute price offset.
//!
//! A spread magnitude is meaningless without a unit — "25 bps" on a bond is
//! ambiguous (price bps vs yield bps, linked by duration). The engine carries an
//! explicit [`SpreadUnit`] and converts every magnitude to a **price offset** in
//! the mid's price space before composing the two-way.
//!
//! Provenance: the bond bps convention (price bps vs duration-consistent yield
//! bps via `Δprice ≈ −ModDur·Δyield·price`) is documented in
//! `docs/fixed-income/FI-TIERING-RESEARCH.md` §3.

use crate::QuoteCtx;
use serde::{Deserialize, Serialize};

/// Errors from converting a spread magnitude to a price offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TieringError {
    /// An input or intermediate value was NaN/±∞.
    NonFinite,
    /// A [`SpreadUnit::YieldBps`] conversion was requested but the context
    /// carried neither `dv01` nor `mod_duration`.
    MissingDuration,
}

/// The unit a spread magnitude is expressed in.
///
/// Serialized on a book's persisted tiering config so a GUI can edit spreads in
/// the trader's preferred unit; the engine converts to a price offset per quote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpreadUnit {
    /// **Price basis points.** `1 bp = 0.01 price points` per 100 face. A flat
    /// "±25 price bps" is `±0.25` in price. Simple, but a fixed price offset maps
    /// to a *different* yield spread at every maturity.
    PriceBps,
    /// **Yield basis points.** `1 bp = 0.01%` of yield, converted to a price
    /// offset via `price_offset = DV01 · yield_bps` (from
    /// `Δprice ≈ −ModDur·Δyield·price`). Duration-consistent across the curve —
    /// the right default for a government-bond book. Requires DV01/ModDur in the
    /// [`QuoteCtx`].
    YieldBps,
    /// **Price points** per 100 face — the magnitude *is* the price offset.
    PricePoints,
    /// **Percent of price.** A magnitude of `1.0` is 1% of the mid, i.e. an
    /// offset of `mid / 100`.
    Percent,
}

impl SpreadUnit {
    /// Convert a spread `magnitude` (in `self`'s unit) to an absolute price
    /// offset in the mid's price space. **Sign-preserving** — a negative
    /// magnitude (e.g. a skew contribution) yields a negative offset, because
    /// every unit's conversion factor is non-negative for well-formed inputs.
    ///
    /// - [`SpreadUnit::PricePoints`] → `magnitude`
    /// - [`SpreadUnit::PriceBps`] → `magnitude · 0.01`
    /// - [`SpreadUnit::Percent`] → `magnitude · mid / 100`
    /// - [`SpreadUnit::YieldBps`] → `magnitude · DV01`
    pub fn to_price_offset(self, magnitude: f64, ctx: &QuoteCtx) -> Result<f64, TieringError> {
        if !magnitude.is_finite() {
            return Err(TieringError::NonFinite);
        }
        let offset = match self {
            SpreadUnit::PricePoints => magnitude,
            SpreadUnit::PriceBps => magnitude * 0.01,
            SpreadUnit::Percent => magnitude * ctx.mid.0 / 100.0,
            SpreadUnit::YieldBps => magnitude * ctx.dv01_resolved()?,
        };
        if offset.is_finite() {
            Ok(offset)
        } else {
            Err(TieringError::NonFinite)
        }
    }
}
