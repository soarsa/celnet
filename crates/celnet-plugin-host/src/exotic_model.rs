//! The tier-agnostic **exotic** pricing model handle the unified registry hands to
//! callers — the multi-asset and path-dependent analog of [`crate::HostModel`].
//!
//! Allows trading desks to plug in, replace, or hot-swap custom pricing engines for
//! barrier options, Asians, cliquets, accumulators, and multi-asset baskets behind
//! the unified [`crate::ModelRegistry`].

use celnet_plugin_api::{
    ExoticPayoffDescriptor, ExoticPricingModel, ModelDescriptor, MultiAssetInputs,
};

use crate::error::HostResult;

/// A priceable exotic / path-dependent model behind the unified registry.
pub trait ExoticHostModel {
    /// Self-description used by the registry to route work to this model.
    fn descriptor(&self) -> ModelDescriptor;

    /// Price the exotic payoff against multi-asset market inputs.
    ///
    /// # Errors
    /// Returns [`crate::HostError::Model`] on invalid inputs or convergence failure.
    fn price_exotic(
        &self,
        payoff: &ExoticPayoffDescriptor,
        inputs: &MultiAssetInputs,
    ) -> HostResult<f64>;

    /// Delta sensitivities across underlyings.
    ///
    /// # Errors
    /// Returns [`crate::HostError::Model`] if unsupported or on computation failure.
    fn deltas(
        &self,
        payoff: &ExoticPayoffDescriptor,
        inputs: &MultiAssetInputs,
    ) -> HostResult<Vec<f64>>;
}

/// Adapts a compiled-in [`ExoticPricingModel`] to the tier-agnostic
/// [`ExoticHostModel`] seam — Tier-0 (native, hot-path) exotic tier.
pub struct NativeExoticModel<M: ExoticPricingModel> {
    model: M,
}

impl<M: ExoticPricingModel> NativeExoticModel<M> {
    /// Wrap a native exotic pricing model as a Tier-0 host model.
    pub const fn new(model: M) -> Self {
        Self { model }
    }

    /// Borrow the wrapped native model.
    pub const fn inner(&self) -> &M {
        &self.model
    }
}

impl<M: ExoticPricingModel> ExoticHostModel for NativeExoticModel<M> {
    fn descriptor(&self) -> ModelDescriptor {
        self.model.descriptor()
    }

    fn price_exotic(
        &self,
        payoff: &ExoticPayoffDescriptor,
        inputs: &MultiAssetInputs,
    ) -> HostResult<f64> {
        Ok(self.model.price_exotic(payoff, inputs)?)
    }

    fn deltas(
        &self,
        payoff: &ExoticPayoffDescriptor,
        inputs: &MultiAssetInputs,
    ) -> HostResult<Vec<f64>> {
        Ok(self.model.deltas(payoff, inputs)?)
    }
}
