//! Tier-0 — the native, compiled-in model tier (the hot path).
//!
//! A first-party model implementing [`celnet_plugin_api::PricingModel`] runs at
//! full native speed with no ABI or sandbox cost. [`NativeModel`] adapts any such
//! model to the tier-agnostic [`crate::HostModel`] seam so it routes through the
//! same [`crate::ModelRegistry`] as a Tier-2 Wasm model. There is **no** sandbox
//! here by design — Tier-0 is trusted first-party code (see
//! `docs/PLUGIN-HOST-ALT.md` §4).
//!
//! Domain errors a native model returns ([`celnet_plugin_api::PluginError`]) are
//! wrapped verbatim into [`crate::HostError::Model`], so a caller observes the
//! *same* model verdict whether the model ran native or in the interpreter — the
//! basis of the Tier-0 == Tier-2 interchangeability gate.

use celnet_plugin_api::{ModelDescriptor, PricingModel};
use celnet_types::{Greeks, OptionType, VanillaInputs};

use crate::error::HostResult;
use crate::model::HostModel;

/// Adapts a compiled-in [`PricingModel`] to the tier-agnostic [`HostModel`] seam.
///
/// Generic over the concrete model so the native call is a direct, monomorphized,
/// inlinable dispatch with zero per-call allocation — the hot-path property
/// Tier-0 exists to preserve.
#[derive(Debug, Clone, Copy)]
pub struct NativeModel<M: PricingModel> {
    model: M,
}

impl<M: PricingModel> NativeModel<M> {
    /// Wrap a native pricing model as a Tier-0 host model.
    pub const fn new(model: M) -> Self {
        Self { model }
    }

    /// Borrow the wrapped native model.
    pub const fn inner(&self) -> &M {
        &self.model
    }
}

impl<M: PricingModel> HostModel for NativeModel<M> {
    fn descriptor(&self) -> ModelDescriptor {
        self.model.descriptor()
    }

    fn price(&self, opt: OptionType, inputs: &VanillaInputs) -> HostResult<f64> {
        Ok(self.model.price(opt, inputs)?)
    }

    fn price_and_greeks(&self, opt: OptionType, inputs: &VanillaInputs) -> HostResult<Greeks> {
        Ok(self.model.price_and_greeks(opt, inputs)?)
    }
}
