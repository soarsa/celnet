//! The unified [`ModelRegistry`] — one routing table over every tier.
//!
//! A caller resolves a model by [`ModelId`] and prices through it without
//! knowing or caring whether a Tier-0 native model or a Tier-2 Wasm model serves
//! the call (requirement R5 of `docs/PLUGIN-HOST-ALT.md`). The registry stores
//! type-erased [`HostModel`] handles keyed by id, fans the frozen
//! [`celnet_plugin_api::ModelRegistry`] discovery trait over their descriptors,
//! and hands back a `&dyn HostModel` for execution.
//!
//! Registration is explicit and ordered (insertion order is the stable discovery
//! order). Native first-party models are registered via [`ModelRegistry::register_native`];
//! sandboxed user models via [`ModelRegistry::register_wasm`]. Both land in the
//! same map, so routing is tier-blind.

use celnet_plugin_api::{
    ModelDescriptor, ModelId, ModelRegistry as DiscoveryRegistry, PluginError, PricingModel,
};

use crate::error::{HostError, HostResult};
use crate::model::HostModel;
use crate::native::NativeModel;
use crate::wasm::{FuelBudget, WasmModel};

/// A unified, tier-blind registry of priceable models.
///
/// Holds boxed [`HostModel`] handles (native or Wasm) and a parallel descriptor
/// list kept in insertion order for cheap, stable discovery. Lookups are by
/// [`ModelId`]; duplicate ids are rejected so identity stays unambiguous.
#[derive(Default)]
pub struct ModelRegistry {
    entries: Vec<Box<dyn HostModel>>,
    descriptors: Vec<ModelDescriptor>,
}

impl core::fmt::Debug for ModelRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ModelRegistry")
            .field("descriptors", &self.descriptors)
            .finish_non_exhaustive()
    }
}

impl ModelRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a compiled-in native (Tier-0) [`PricingModel`].
    ///
    /// # Errors
    /// Returns [`HostError::Model`] wrapping [`PluginError::InvalidInput`] if a
    /// model with the same [`ModelId`] is already registered.
    pub fn register_native<M: PricingModel + 'static>(&mut self, model: M) -> HostResult<ModelId> {
        self.insert(Box::new(NativeModel::new(model)))
    }

    /// Register an already-loaded sandboxed (Tier-2) [`WasmModel`].
    ///
    /// # Errors
    /// As [`ModelRegistry::register_native`].
    pub fn register_wasm(&mut self, model: WasmModel) -> HostResult<ModelId> {
        self.insert(Box::new(model))
    }

    /// Load a Tier-2 model from core-Wasm `bytes` under `descriptor` with the
    /// given `fuel_budget`, then register it. Convenience over
    /// [`WasmModel::load`] + [`ModelRegistry::register_wasm`].
    ///
    /// # Errors
    /// Any [`WasmModel::load`] error, or the duplicate-id error from
    /// [`ModelRegistry::register_wasm`].
    pub fn load_wasm(
        &mut self,
        descriptor: ModelDescriptor,
        bytes: &[u8],
        fuel_budget: FuelBudget,
    ) -> HostResult<ModelId> {
        let model = WasmModel::load(descriptor, bytes, fuel_budget)?;
        self.register_wasm(model)
    }

    /// Common insertion path: reject duplicate ids, record the descriptor, store
    /// the type-erased handle.
    fn insert(&mut self, model: Box<dyn HostModel>) -> HostResult<ModelId> {
        let descriptor = model.descriptor();
        if self.descriptors.iter().any(|d| d.id == descriptor.id) {
            return Err(HostError::Model(PluginError::InvalidInput(
                "duplicate model id",
            )));
        }
        self.entries.push(model);
        self.descriptors.push(descriptor);
        Ok(descriptor.id)
    }

    /// Borrow the tier-blind model handle for `id`, ready to price.
    ///
    /// # Errors
    /// Returns [`HostError::Model`] wrapping [`PluginError::NotFound`] if no model
    /// has that id.
    pub fn model(&self, id: ModelId) -> HostResult<&dyn HostModel> {
        self.descriptors
            .iter()
            .position(|d| d.id == id)
            .map(|i| self.entries[i].as_ref())
            .ok_or(HostError::Model(PluginError::NotFound("model id")))
    }

    /// Number of registered models across all tiers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the registry holds no models.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The frozen discovery contract: descriptor-only introspection, tier-blind.
///
/// Implementing [`celnet_plugin_api::ModelRegistry`] gives callers the same
/// `descriptor`/`provides`/`of_kind` routing surface they use against any other
/// registry, so the host plugs into the engine without a bespoke discovery path.
impl DiscoveryRegistry for ModelRegistry {
    fn descriptors(&self) -> &[ModelDescriptor] {
        &self.descriptors
    }
}
