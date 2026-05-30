//! The model-registry seam.
//!
//! The engine resolves a model by [`ModelId`] / [`ModelKind`] through a
//! [`ModelRegistry`] without knowing whether the model is a compiled-in native
//! implementation (the `inventory`-style trait registry) or a sandboxed Wasm
//! component (resolved by the plugin host). Both backends implement this one
//! trait, which is exactly what makes first-party and user plugins
//! interchangeable (see `docs/ARCHITECTURE.md` §6.1). The registry only exposes
//! *descriptors* here — the host turns a descriptor into a callable handle —
//! keeping this crate dependency-light (no `wasmtime`, no dynamic-dispatch
//! lifetime entanglement) while still pinning the discovery contract.

use crate::descriptor::{ModelDescriptor, ModelId, ModelKind};
use crate::error::PluginResult;

/// Discovery interface over a set of registered models.
///
/// Implementations are expected to be cheap to query (descriptor lookups, not
/// model instantiation) so the engine can route on the hot path.
pub trait ModelRegistry {
    /// All registered model descriptors, in a stable order.
    fn descriptors(&self) -> &[ModelDescriptor];

    /// The descriptor for `id`, if registered.
    ///
    /// # Errors
    /// Returns [`crate::PluginError::NotFound`] if no model has that id.
    fn descriptor(&self, id: ModelId) -> PluginResult<ModelDescriptor> {
        self.descriptors()
            .iter()
            .find(|d| d.id == id)
            .copied()
            .ok_or(crate::error::PluginError::NotFound("model id"))
    }

    /// Whether a model of the given `id` and `kind` is registered.
    fn provides(&self, id: ModelId, kind: ModelKind) -> bool {
        self.descriptors()
            .iter()
            .any(|d| d.id == id && d.kind == kind)
    }

    /// Descriptors of every registered model of a given `kind`, collected in
    /// registration order.
    fn of_kind(&self, kind: ModelKind) -> Vec<ModelDescriptor> {
        self.descriptors()
            .iter()
            .copied()
            .filter(|d| d.kind == kind)
            .collect()
    }
}
