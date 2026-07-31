//! The position/inventory sink a corporate-action apply drives, and an in-memory reference book.
//!
//! The lifecycle (§7.5) turns a confirmed CA event into a [`celnet_corpactions::PositionDelta`] and
//! pushes it here: `REDM/MCAL` **realise** (position → 0), `PCAL/PRED/DRAW` **scale** (nominal ↓),
//! `INTR` pays income, `EXOF/CONV` exchange into a target. The server implements this trait over its
//! `RatesPositionStore` (`crates/celnet-server/src/services/rates_book.rs`) so a CA books through the
//! exact same path a trade does — no parallel store (§8). This crate ships [`InMemoryPositionBook`]
//! for tests and as the reference semantics an adapter must match.

use std::collections::HashMap;

use celnet_corpactions::PositionDelta;

/// Failure modes of applying a position delta to a sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkError {
    /// The instrument has no position to adjust (a realise/scale on an empty holding).
    NoPosition(String),
    /// The delta is not finite.
    NonFiniteDelta,
}

impl core::fmt::Display for SinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoPosition(id) => write!(f, "no position to adjust for instrument {id}"),
            Self::NonFiniteDelta => f.write_str("the position delta is not finite"),
        }
    }
}

impl core::error::Error for SinkError {}

/// The sink a corporate-action apply drives — the seam the server's `RatesPositionStore` implements.
pub trait PositionSink {
    /// Apply `delta` (a face change + cash + optional exchange leg) to the holding in `instrument_id`.
    ///
    /// # Errors
    /// Implementation-defined; the reference book returns [`SinkError`] on a non-finite delta or a
    /// realise/scale against an absent position.
    fn apply_delta(&mut self, instrument_id: &str, delta: &PositionDelta) -> Result<(), SinkError>;
}

/// One holding in the reference book.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Holding {
    /// The current held face (nominal).
    pub face: f64,
    /// Cumulative cash thrown off by applied corporate actions on this holding.
    pub cash: f64,
    /// Target legs created by exchanges/conversions on this holding: `(target_id, units)`.
    pub exchanged_into: Vec<(String, f64)>,
}

/// An in-memory position book — the reference [`PositionSink`] semantics + a test double.
#[derive(Debug, Clone, Default)]
pub struct InMemoryPositionBook {
    holdings: HashMap<String, Holding>,
}

impl InMemoryPositionBook {
    /// An empty book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed an opening holding of `face` in `instrument_id` (a booked trade the CA later adjusts).
    pub fn set_face(&mut self, instrument_id: &str, face: f64) {
        self.holdings
            .entry(instrument_id.to_string())
            .or_default()
            .face = face;
    }

    /// The current holding for `instrument_id`, if any.
    #[must_use]
    pub fn holding(&self, instrument_id: &str) -> Option<&Holding> {
        self.holdings.get(instrument_id)
    }
}

impl PositionSink for InMemoryPositionBook {
    fn apply_delta(&mut self, instrument_id: &str, delta: &PositionDelta) -> Result<(), SinkError> {
        if !(delta.face_delta.is_finite() && delta.cash.is_finite()) {
            return Err(SinkError::NonFiniteDelta);
        }
        let entry = self
            .holdings
            .get_mut(instrument_id)
            .ok_or_else(|| SinkError::NoPosition(instrument_id.to_string()))?;
        entry.face += delta.face_delta;
        entry.cash += delta.cash;
        if let Some(leg) = &delta.exchange_into {
            entry.exchanged_into.push((leg.target.clone(), leg.units));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_corpactions::{PositionEffect, position_delta};

    #[test]
    fn scale_reduces_face_and_accrues_cash() {
        let mut book = InMemoryPositionBook::new();
        book.set_face("BOND1", 1_000_000.0);
        let delta = position_delta(
            &PositionEffect::Scale {
                retained_fraction: 0.7,
                cash_per_100_redeemed: 100.0,
            },
            1_000_000.0,
        );
        book.apply_delta("BOND1", &delta).expect("applied");
        let h = book.holding("BOND1").unwrap();
        assert!((h.face - 700_000.0).abs() < 1e-6);
        assert!((h.cash - 300_000.0).abs() < 1e-6);
    }

    #[test]
    fn realise_on_absent_position_errors() {
        let mut book = InMemoryPositionBook::new();
        let delta = position_delta(
            &PositionEffect::Realise {
                cash_per_100: 100.0,
            },
            1_000_000.0,
        );
        assert!(matches!(
            book.apply_delta("MISSING", &delta),
            Err(SinkError::NoPosition(_))
        ));
    }
}
