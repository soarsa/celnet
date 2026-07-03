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
    ModelDescriptor, ModelId, ModelKind, ModelRegistry as DiscoveryRegistry, PluginError,
    PricingModel, RatesPricingModel, RatesProductKind,
};

use crate::error::{HostError, HostResult};
use crate::model::HostModel;
use crate::native::NativeModel;
use crate::rates_model::{NativeRatesModel, RatesHostModel};
use crate::wasm::{FuelBudget, WasmModel};

/// One registered linear fixed-income model: the [`RatesProductKind`] it serves
/// (the routing key), its descriptor, and the tier-blind handle.
struct RatesEntry {
    kind: RatesProductKind,
    descriptor: ModelDescriptor,
    model: Box<dyn RatesHostModel>,
}

/// A unified, tier-blind registry of priceable models.
///
/// Holds boxed [`HostModel`] handles (native or Wasm) and a parallel descriptor
/// list kept in insertion order for cheap, stable discovery. Lookups are by
/// [`ModelId`]; duplicate ids are rejected so identity stays unambiguous.
#[derive(Default)]
pub struct ModelRegistry {
    entries: Vec<Box<dyn HostModel>>,
    descriptors: Vec<ModelDescriptor>,
    /// Registered linear fixed-income models, keyed by the [`RatesProductKind`]
    /// each serves. Kept separate from the option `entries` because a rates model
    /// implements a distinct contract ([`RatesPricingModel`]) and is resolved by
    /// FI product kind rather than by option [`ModelKind`] — the FI half of the
    /// uniform-asset-class dispatch (ADR-0021).
    rates_entries: Vec<RatesEntry>,
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

    /// The active house pricing model — the first registered
    /// [`ModelKind::Pricing`] handle in stable insertion order, or `None` if no
    /// pricing model is registered.
    ///
    /// The non-allocating hot-path resolver an engine consults to serve an
    /// analytic arm through a registered model: a linear scan returning a borrowed
    /// `&dyn HostModel` with **no `Vec`**, unlike the discovery-trait
    /// [`celnet_plugin_api::ModelRegistry::of_kind`] (which allocates). A server
    /// registers the single house pricer it wants on the analytic arm; if several
    /// pricing models are registered the first registered is authoritative, so the
    /// resolution is deterministic.
    #[must_use]
    pub fn active_pricing_model(&self) -> Option<&dyn HostModel> {
        self.descriptors
            .iter()
            .position(|d| d.kind == ModelKind::Pricing)
            .map(|i| self.entries[i].as_ref())
    }

    /// Register a compiled-in native (Tier-0) [`RatesPricingModel`] against the
    /// [`RatesProductKind`] it serves — the FI counterpart of
    /// [`ModelRegistry::register_native`]. A desk registers its OIS model and its
    /// bond model as two calls (one per kind); the FI dispatch then resolves the
    /// registered model **by kind** via [`ModelRegistry::active_rates_model`].
    ///
    /// # Errors
    /// Returns [`HostError::Model`] wrapping [`PluginError::InvalidInput`] if a
    /// model with the same [`ModelId`] is already registered for the same kind.
    pub fn register_native_rates<M: RatesPricingModel + 'static>(
        &mut self,
        kind: RatesProductKind,
        model: M,
    ) -> HostResult<ModelId> {
        let handle: Box<dyn RatesHostModel> = Box::new(NativeRatesModel::new(model));
        let descriptor = handle.descriptor();
        if self
            .rates_entries
            .iter()
            .any(|e| e.kind == kind && e.descriptor.id == descriptor.id)
        {
            return Err(HostError::Model(PluginError::InvalidInput(
                "duplicate rates model id for kind",
            )));
        }
        self.rates_entries.push(RatesEntry {
            kind,
            descriptor,
            model: handle,
        });
        Ok(descriptor.id)
    }

    /// The active house rates model for `kind` — the first registered rates model
    /// serving that [`RatesProductKind`] in stable insertion order, or `None` if
    /// none is registered for it.
    ///
    /// The non-allocating hot-path resolver the FI dispatch consults to serve an
    /// OIS / IRS / FRA / bond arm through a registered model: a linear scan
    /// returning a borrowed `&dyn RatesHostModel`. `None` (the default for an
    /// unregistered kind) leaves the arm on the verbatim native path — the FI
    /// analog of [`ModelRegistry::active_pricing_model`].
    #[must_use]
    pub fn active_rates_model(&self, kind: RatesProductKind) -> Option<&dyn RatesHostModel> {
        self.rates_entries
            .iter()
            .find(|e| e.kind == kind)
            .map(|e| e.model.as_ref())
    }

    /// Number of registered option models across all tiers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the registry holds no option models.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of registered linear fixed-income models across all kinds.
    #[must_use]
    pub fn rates_len(&self) -> usize {
        self.rates_entries.len()
    }

    /// Whether the registry holds no rates models.
    #[must_use]
    pub fn rates_is_empty(&self) -> bool {
        self.rates_entries.is_empty()
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

#[cfg(test)]
mod tests {
    use super::ModelRegistry;
    use celnet_plugin_api::example::ConstantRatesModel;
    use celnet_plugin_api::{RatesCurvePillar, RatesMeasures, RatesProductKind, RatesTerms};

    fn measures(pv: f64) -> RatesMeasures {
        RatesMeasures {
            pv,
            par_rate: 0.04,
            pv01: 1.0,
            dv01: 1.0,
            key_rate_ladder: vec![],
        }
    }

    fn ois_terms() -> RatesTerms {
        RatesTerms::Ois {
            tenor_years: 5,
            fixed_rate: 0.04,
            notional: 1e6,
            receive_fixed: true,
        }
    }

    /// A native rates model registers by [`RatesProductKind`] and resolves for
    /// that kind only; the option registry stays independent.
    #[test]
    fn rates_models_register_and_resolve_by_kind() {
        let mut reg = ModelRegistry::new();
        assert!(reg.rates_is_empty());
        assert!(reg.active_rates_model(RatesProductKind::Ois).is_none());

        reg.register_native_rates(
            RatesProductKind::Ois,
            ConstantRatesModel::new(measures(42.0)),
        )
        .unwrap();
        assert_eq!(reg.rates_len(), 1);

        let curve = [RatesCurvePillar::new(1.0, 0.04)];
        let priced = reg
            .active_rates_model(RatesProductKind::Ois)
            .unwrap()
            .price(&ois_terms(), &curve)
            .unwrap();
        assert_eq!(priced.pv, 42.0);

        // No model for the other kinds.
        assert!(reg.active_rates_model(RatesProductKind::Irs).is_none());
        assert!(reg.active_rates_model(RatesProductKind::Bond).is_none());

        // The option registry is untouched by rates registration.
        assert!(reg.is_empty());
        assert!(reg.active_pricing_model().is_none());
    }

    /// A duplicate `(kind, id)` is rejected, but the same id under a different
    /// kind is a distinct registration.
    #[test]
    fn duplicate_rates_id_for_kind_is_rejected() {
        let mut reg = ModelRegistry::new();
        reg.register_native_rates(
            RatesProductKind::Ois,
            ConstantRatesModel::new(measures(1.0)),
        )
        .unwrap();
        assert!(
            reg.register_native_rates(
                RatesProductKind::Ois,
                ConstantRatesModel::new(measures(2.0))
            )
            .is_err()
        );
        // Same model id, different kind: allowed.
        reg.register_native_rates(
            RatesProductKind::Bond,
            ConstantRatesModel::new(measures(3.0)),
        )
        .unwrap();
        assert_eq!(reg.rates_len(), 2);
    }
}
