//! Celnet core: pure-domain math primitives, deterministic float comparison,
//! and the cross-crate trait seams that the pricing layers share.
//!
//! This crate has **zero IO and zero framework dependencies** — it cannot
//! allocate, lock, or call a runtime — so determinism and zero-alloc are
//! structurally enforceable here. It sits just above [`celnet_types`] in the
//! frozen interface layer (see `docs/ROADMAP.md`).

#![forbid(unsafe_code)]

pub mod math;

mod compare;
pub use compare::{DEFAULT_ABS, DEFAULT_REL, is_close};

use celnet_types::Vol;

/// A volatility smile/surface evaluated in strike space.
///
/// This is the seam between the surface-construction layer
/// (`celnet-surface`: Vanna-Volga, SABR, SVI/SSVI) and any consumer that needs an
/// implied volatility at a given strike — the vanilla and exotic engines depend
/// on this trait, never on a concrete surface model, so models are swappable.
pub trait Smile {
    /// Implied (Black) volatility for `strike` given the outright `forward` and
    /// time-to-expiry `t` (years). Implementations must be arbitrage-aware.
    fn implied_vol(&self, strike: f64, forward: f64, t: f64) -> Vol;

    /// At-the-money-forward volatility (`strike == forward`), provided as a
    /// hot-path convenience; the default delegates to [`Smile::implied_vol`].
    fn atm_forward_vol(&self, forward: f64, t: f64) -> Vol {
        self.implied_vol(forward, forward, t)
    }
}

/// A flat (constant) smile — the simplest [`Smile`], useful as a building block,
/// a degenerate surface, and a test oracle. Fully implemented (not a mock):
/// it returns the same Black vol at every strike.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlatSmile {
    /// The constant Black volatility returned at every strike.
    pub vol: f64,
}

impl FlatSmile {
    /// Construct a flat smile at the given absolute volatility.
    #[must_use]
    pub const fn new(vol: f64) -> Self {
        Self { vol }
    }
}

impl Smile for FlatSmile {
    fn implied_vol(&self, _strike: f64, _forward: f64, _t: f64) -> Vol {
        Vol(self.vol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_smile_is_constant() {
        let s = FlatSmile::new(0.12);
        assert_close!(s.implied_vol(90.0, 100.0, 0.5).0, 0.12);
        assert_close!(s.atm_forward_vol(100.0, 0.5).0, 0.12);
    }
}
