//! Pricing context and value types for the tiering engine.
//!
//! These are the *inputs* a strategy reads and the *contribution* it produces.
//! All prices are unit-agnostic values — for bonds the convention is **per 100
//! face** (a mid of `99.55` is 99.55% of par), but the engine never assumes an
//! asset class: it works in whatever price space the composite mid is expressed
//! in. Spread magnitudes are converted to an absolute **price offset** in that
//! same space (see [`crate::SpreadUnit`]).

use crate::TieringError;

/// The composite mid (or micro-price) the outbound two-way is built around.
///
/// Sourced upstream from `celnet-aggregation`'s consolidated best bid/offer;
/// the tiering engine treats it as an opaque price in the instrument's price
/// space (per-100-face for bonds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mid(pub f64);

/// An outbound two-way price: what we stream/quote to a client.
///
/// The engine's core invariant is `bid < offer` and `offer - bid >= spread_floor`
/// (see [`crate::quote`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoWay {
    /// The price at which we buy from the client (per 100 face for bonds).
    pub bid: f64,
    /// The price at which we sell to the client (per 100 face for bonds).
    pub offer: f64,
}

/// A single strategy's contribution, expressed as **absolute price offsets**
/// (already unit-converted from the strategy's [`crate::SpreadUnit`]).
///
/// Composed additively by the pipeline: `h = Σ half_spread`, `s = Σ skew`, then
/// `bid = mid − h − s`, `offer = mid + h − s`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpreadSkew {
    /// Symmetric half-spread `h ≥ 0` (price offset). Widens both sides equally;
    /// `offer − bid = 2h` is invariant under skew.
    pub half_spread: f64,
    /// Signed skew `s` (price offset). `s > 0` shifts the **whole** two-way down
    /// (cheaper offer, lower bid) — the direction a **long** dealer skews to shed
    /// inventory. See [`crate::InventorySkew`].
    pub skew: f64,
}

/// Immutable pricing context threaded to every strategy.
///
/// Carries the composite mid plus the risk/market state a strategy may read:
/// signed inventory `q`, requested `size` `z`, realized vol `σ` against a
/// reference `σ_ref`, and — for yield-bps bond spreads — the bond's `DV01` or
/// modified duration used to convert a yield move to a price offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuoteCtx {
    /// Composite mid the two-way is built around.
    pub mid: Mid,
    /// Signed dealer inventory `q` (position). Positive = long.
    pub inventory: f64,
    /// Requested trade size `z` (same face/notional unit as `inventory`).
    pub size: f64,
    /// Realized volatility `σ` (for the vol-scaling strategy, phase 3).
    pub vol: f64,
    /// Reference volatility `σ_ref` the realized vol is scaled against.
    pub vol_ref: f64,
    /// Bond DV01 — price change per **1 bp** of yield, in the mid's price space
    /// (`DV01 = ModDur · Price / 10000`). Required for [`crate::SpreadUnit::YieldBps`].
    pub dv01: Option<f64>,
    /// Bond modified duration — fallback source for DV01 when `dv01` is absent
    /// (`DV01 = mod_duration · mid / 10000`).
    pub mod_duration: Option<f64>,
    /// The fading-memory **smoothed** observed spread `Sₙ` (absolute price offset),
    /// already advanced by the stateful [`crate::smooth`] updater in the layer that
    /// owns the per-(book, instrument) EWMA state. Read by
    /// [`crate::ScaledSmoothedSpread`]. `None` ⇒ the observed level is unavailable
    /// (the strategy then quotes at its Max Output Spread — the indicative fallback).
    pub smoothed_spread: Option<f64>,
    /// The current **raw** observed spread `Rₙ` (absolute price offset) the smoothing
    /// consumed — the composite's own consolidated `best_offer − best_bid` in the
    /// streaming integration. Carried for provenance/telemetry; the strategy prices
    /// off [`Self::smoothed_spread`], not this. `None` ⇒ observed level unavailable.
    pub raw_spread: Option<f64>,
    /// Upstream freshness signal: `true` when the composite is stale or the LP
    /// quorum was lost. Triggers the [`crate::StalePolicy`] in the pipeline.
    pub is_stale: bool,
}

impl QuoteCtx {
    /// A fresh context at `mid` with neutral defaults (flat inventory, unit vol,
    /// no duration data, not stale). Build up with the `with_*` methods.
    #[must_use]
    pub fn new(mid: f64) -> Self {
        Self {
            mid: Mid(mid),
            inventory: 0.0,
            size: 0.0,
            vol: 1.0,
            vol_ref: 1.0,
            dv01: None,
            mod_duration: None,
            smoothed_spread: None,
            raw_spread: None,
            is_stale: false,
        }
    }

    /// Set signed inventory `q`.
    #[must_use]
    pub fn with_inventory(mut self, q: f64) -> Self {
        self.inventory = q;
        self
    }

    /// Set requested trade size `z`.
    #[must_use]
    pub fn with_size(mut self, z: f64) -> Self {
        self.size = z;
        self
    }

    /// Set realized vol `σ` and its reference `σ_ref`.
    #[must_use]
    pub fn with_vol(mut self, vol: f64, vol_ref: f64) -> Self {
        self.vol = vol;
        self.vol_ref = vol_ref;
        self
    }

    /// Supply the bond DV01 (price change per 1 bp of yield).
    #[must_use]
    pub fn with_dv01(mut self, dv01: f64) -> Self {
        self.dv01 = Some(dv01);
        self
    }

    /// Supply modified duration (DV01 is then derived as `mod_duration·mid/10000`).
    #[must_use]
    pub fn with_mod_duration(mut self, mod_duration: f64) -> Self {
        self.mod_duration = Some(mod_duration);
        self
    }

    /// Supply the smoothed observed spread `Sₙ` (absolute price offset) read by
    /// [`crate::ScaledSmoothedSpread`].
    #[must_use]
    pub fn with_smoothed_spread(mut self, smoothed_spread: f64) -> Self {
        self.smoothed_spread = Some(smoothed_spread);
        self
    }

    /// Supply the raw observed spread `Rₙ` (absolute price offset) the smoothing
    /// consumed, for provenance/telemetry.
    #[must_use]
    pub fn with_raw_spread(mut self, raw_spread: f64) -> Self {
        self.raw_spread = Some(raw_spread);
        self
    }

    /// Mark the context stale (composite/quorum lost upstream).
    #[must_use]
    pub fn stale(mut self) -> Self {
        self.is_stale = true;
        self
    }

    /// Resolve the DV01 used for yield-bps conversion.
    ///
    /// Prefers an explicit [`Self::dv01`]; otherwise derives it from
    /// [`Self::mod_duration`] and the mid as `DV01 = ModDur · mid / 10000`
    /// (from `Δprice ≈ −ModDur · Δyield · price`, with `Δyield = 1 bp = 1e-4`).
    /// Returns [`TieringError::MissingDuration`] when neither is present.
    pub(crate) fn dv01_resolved(&self) -> Result<f64, TieringError> {
        let raw = if let Some(dv01) = self.dv01 {
            dv01
        } else if let Some(md) = self.mod_duration {
            md * self.mid.0 / 10_000.0
        } else {
            return Err(TieringError::MissingDuration);
        };
        if raw.is_finite() {
            Ok(raw)
        } else {
            Err(TieringError::NonFinite)
        }
    }
}
