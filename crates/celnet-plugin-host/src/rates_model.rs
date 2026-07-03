//! The tier-agnostic **rates** model handle the unified registry hands to
//! callers — the linear fixed-income analog of [`crate::HostModel`].
//!
//! A caller (the FI dispatch) prices a linear fixed-income instrument through a
//! registered model without knowing its concrete type, exactly as
//! [`crate::HostModel`] hides an option model's tier. [`RatesHostModel`] mirrors
//! the [`celnet_plugin_api::RatesPricingModel`] shape but returns the richer
//! [`crate::HostError`], so a (future) sandboxed rates tier's outcomes (trap,
//! fuel exhaustion) are expressible alongside the model's own
//! [`celnet_plugin_api::PluginError`]. It is the only rates type the
//! [`crate::ModelRegistry`] exposes.

use celnet_plugin_api::{
    ModelDescriptor, RatesCurvePillar, RatesMeasures, RatesPricingModel, RatesTerms,
};

use crate::error::HostResult;

/// A priceable linear fixed-income model behind the unified registry, agnostic to
/// its tier.
///
/// Method semantics match [`celnet_plugin_api::RatesPricingModel`]; the only
/// difference is the error type, widened to [`crate::HostError`]. As for
/// [`crate::HostModel`], a single handle is **not** `Sync` and is owned by one
/// pricing worker.
pub trait RatesHostModel {
    /// Self-description used by the registry to route work to this model.
    fn descriptor(&self) -> ModelDescriptor;

    /// Price the instrument's [`RatesTerms`] against the calibrating `curve`.
    ///
    /// # Errors
    /// Returns [`crate::HostError::Model`] for a model-domain failure (or, for a
    /// future sandboxed tier, a sandbox failure).
    fn price(&self, terms: &RatesTerms, curve: &[RatesCurvePillar]) -> HostResult<RatesMeasures>;
}

/// Adapts a compiled-in [`RatesPricingModel`] to the tier-agnostic
/// [`RatesHostModel`] seam — the Tier-0 (native, hot-path) rates tier, the FI
/// analog of [`crate::NativeModel`].
///
/// Generic over the concrete model so the native call is a direct, monomorphized,
/// inlinable dispatch. Domain errors ([`celnet_plugin_api::PluginError`]) are
/// wrapped verbatim into [`crate::HostError::Model`], so a caller observes the
/// same model verdict regardless of tier.
pub struct NativeRatesModel<M: RatesPricingModel> {
    model: M,
}

impl<M: RatesPricingModel> NativeRatesModel<M> {
    /// Wrap a native rates pricing model as a Tier-0 host model.
    pub const fn new(model: M) -> Self {
        Self { model }
    }
}

impl<M: RatesPricingModel> RatesHostModel for NativeRatesModel<M> {
    fn descriptor(&self) -> ModelDescriptor {
        self.model.descriptor()
    }

    fn price(&self, terms: &RatesTerms, curve: &[RatesCurvePillar]) -> HostResult<RatesMeasures> {
        Ok(self.model.price(terms, curve)?)
    }
}
