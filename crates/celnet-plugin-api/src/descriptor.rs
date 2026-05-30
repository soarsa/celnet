//! Self-describing plugin metadata shared by every SDK model kind.
//!
//! A registry (native or Wasm) keys models by [`ModelId`] and advertises a
//! [`ModelDescriptor`] so the engine can pick a model by purpose without knowing
//! its concrete type. This mirrors the `model-descriptor` record in
//! `wit/celnet.wit`, so a native registry entry and a Wasm component describe
//! themselves identically.

/// A stable, human-readable identifier for a registered model.
///
/// Borrowed (`&'static str` for compiled-in native models; the Wasm host backs
/// it with an interned string) so identity is cheap to copy and compare on the
/// registry hot path. Purpose-named — never a vendor or author name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelId(pub &'static str);

impl ModelId {
    /// The identifier as a string slice.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0
    }
}

impl core::fmt::Display for ModelId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// The analytic capability a model exposes, used by the registry to route work
/// to a model that can satisfy a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelKind {
    /// Prices vanilla options and (optionally) Greeks — implements
    /// [`crate::PricingModel`].
    Pricing,
    /// Produces a volatility smile/surface — implements [`crate::SmileModel`].
    Smile,
    /// Fits model parameters to market targets — implements
    /// [`crate::Calibration`].
    Calibration,
}

/// Which Greeks a [`crate::PricingModel`] is able to produce, advertised up
/// front so the engine never calls into a model for a sensitivity it cannot
/// supply (avoiding a per-call [`crate::PluginError::Unsupported`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GreekSupport {
    /// First-order: spot/forward delta, vega, theta, both rhos.
    pub first_order: bool,
    /// Second-order: gamma, vanna, volga.
    pub second_order: bool,
    /// Third-order: speed, zomma, color, charm.
    pub third_order: bool,
}

impl GreekSupport {
    /// A model that supplies the full Celnet Greek set.
    pub const FULL: GreekSupport = GreekSupport {
        first_order: true,
        second_order: true,
        third_order: true,
    };
    /// A price-only model (no Greeks).
    pub const PRICE_ONLY: GreekSupport = GreekSupport {
        first_order: false,
        second_order: false,
        third_order: false,
    };
}

/// Everything the engine needs to discover and route to a model without knowing
/// its concrete type. The Wasm host populates this from the component's exported
/// `describe` call; native models return it from
/// [`crate::PricingModel::descriptor`] (and the smile/calibration analogues).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelDescriptor {
    /// Stable purpose-named identity.
    pub id: ModelId,
    /// Which analytic surface this model implements.
    pub kind: ModelKind,
    /// Which Greeks a pricing model produces (ignored for non-pricing kinds;
    /// set to [`GreekSupport::PRICE_ONLY`] there by convention).
    pub greeks: GreekSupport,
}

impl ModelDescriptor {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(id: ModelId, kind: ModelKind, greeks: GreekSupport) -> Self {
        Self { id, kind, greeks }
    }
}
