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
    ExoticPricingModel, ModelDescriptor, ModelId, ModelKind, ModelRegistry as DiscoveryRegistry,
    PluginError, PricingModel, RatesPricingModel, RatesProductKind,
};

use crate::error::{HostError, HostResult};
use crate::exotic_model::{ExoticHostModel, NativeExoticModel};
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

/// One registered exotic / path-dependent pricing model.
struct ExoticEntry {
    descriptor: ModelDescriptor,
    model: Box<dyn ExoticHostModel>,
}

/// A unified, tier-blind registry of priceable models.
///
/// Holds boxed [`HostModel`] handles (native or Wasm) and a parallel descriptor
/// list kept in insertion order for cheap, stable discovery. Lookups are by
/// [`ModelId`]; duplicate ids are rejected on simple registration, while hot-swapping
/// and replace methods allow atomic in-place replacement.
#[derive(Default)]
pub struct ModelRegistry {
    entries: Vec<Box<dyn HostModel>>,
    descriptors: Vec<ModelDescriptor>,
    active_pricing_id: Option<ModelId>,
    /// Registered linear fixed-income models, keyed by the [`RatesProductKind`]
    /// each serves. Kept separate from the option `entries` because a rates model
    /// implements a distinct contract ([`RatesPricingModel`]) and is resolved by
    /// FI product kind rather than by option [`ModelKind`] — the FI half of the
    /// uniform-asset-class dispatch (ADR-0021).
    rates_entries: Vec<RatesEntry>,
    active_rates_ids: Vec<(RatesProductKind, ModelId)>,
    /// Registered exotic and multi-asset pricing models.
    exotic_entries: Vec<ExoticEntry>,
    active_exotic_id: Option<ModelId>,
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

    /// Atomically replace an existing native model in place or insert if new.
    ///
    /// Preserves discovery and routing ordering if replacing an existing model ID.
    pub fn replace_or_insert_native<M: PricingModel + 'static>(
        &mut self,
        model: M,
    ) -> HostResult<ModelId> {
        self.replace_or_insert(Box::new(NativeModel::new(model)))
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

    /// Load a Tier-2 model from a cryptographically verified, licensed [`ComponentArtifact`].
    pub fn load_licensed_artifact(
        &mut self,
        descriptor: ModelDescriptor,
        artifact: &celnet_license::ComponentArtifact,
        authority_key: &[u8; 32],
        fuel_budget: FuelBudget,
    ) -> HostResult<ModelId> {
        artifact
            .verify_signature(authority_key)
            .map_err(|_| HostError::Model(PluginError::InvalidInput("invalid artifact signature")))?;
        if artifact.component_id != descriptor.id.0 {
            return Err(HostError::Model(PluginError::InvalidInput(
                "component_id does not match descriptor id",
            )));
        }
        self.load_wasm(descriptor, &artifact.payload, fuel_budget)
    }

    /// Check whether a model with `id` is currently registered.
    #[must_use]
    pub fn has_model(&self, id: ModelId) -> bool {
        self.descriptors.iter().any(|d| d.id == id)
    }

    /// Hot-swap or register a Tier-2 model from a cryptographically verified [`celnet_license::ComponentArtifact`].
    ///
    /// If a model with `descriptor.id` already exists, its execution instance and descriptor
    /// are atomically replaced in place, preserving stable discovery order. If it is new, it is inserted.
    pub fn hot_swap_licensed_artifact(
        &mut self,
        descriptor: ModelDescriptor,
        artifact: &celnet_license::ComponentArtifact,
        authority_key: &[u8; 32],
        fuel_budget: FuelBudget,
    ) -> HostResult<ModelId> {
        artifact
            .verify_signature(authority_key)
            .map_err(|_| HostError::Model(PluginError::InvalidInput("invalid artifact signature")))?;
        if artifact.component_id != descriptor.id.0 {
            return Err(HostError::Model(PluginError::InvalidInput(
                "component_id does not match descriptor id",
            )));
        }
        let model = WasmModel::load(descriptor, &artifact.payload, fuel_budget)?;
        self.replace_or_insert(Box::new(model))
    }

    /// Hydrate and hot-swap a model directly from a local artifact cache.
    pub fn hydrate_from_cache(
        &mut self,
        descriptor: ModelDescriptor,
        cache: &celnet_license::LocalArtifactCache,
        version: &str,
        authority_key: &[u8; 32],
        fuel_budget: FuelBudget,
    ) -> HostResult<ModelId> {
        let artifact = cache
            .get(descriptor.id.0, version)
            .ok_or_else(|| HostError::Model(PluginError::NotFound("artifact not in local cache")))?;
        self.hot_swap_licensed_artifact(descriptor, &artifact, authority_key, fuel_budget)
    }

    /// Atomically replace an existing model in place or append if new.
    fn replace_or_insert(&mut self, model: Box<dyn HostModel>) -> HostResult<ModelId> {
        let descriptor = model.descriptor();
        if let Some(pos) = self.descriptors.iter().position(|d| d.id == descriptor.id) {
            self.entries[pos] = model;
            self.descriptors[pos] = descriptor;
            Ok(descriptor.id)
        } else {
            self.entries.push(model);
            self.descriptors.push(descriptor);
            Ok(descriptor.id)
        }
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

    /// Explicitly designate the active pricing model by [`ModelId`].
    ///
    /// # Errors
    /// Returns [`HostError::Model`] wrapping [`PluginError::NotFound`] if the id is
    /// not registered, or [`PluginError::InvalidInput`] if it is not a pricing model.
    pub fn set_active_pricing_model(&mut self, id: ModelId) -> HostResult<()> {
        let desc = self
            .descriptors
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| HostError::Model(PluginError::NotFound("model id")))?;
        if desc.kind != ModelKind::Pricing {
            return Err(HostError::Model(PluginError::InvalidInput(
                "model is not a pricing model",
            )));
        }
        self.active_pricing_id = Some(id);
        Ok(())
    }

    /// The id of the currently active pricing model, if explicitly set or deterministically resolved.
    #[must_use]
    pub fn active_pricing_model_id(&self) -> Option<ModelId> {
        if let Some(id) = self.active_pricing_id {
            if self.has_model(id) {
                return Some(id);
            }
        }
        self.descriptors
            .iter()
            .find(|d| d.kind == ModelKind::Pricing)
            .map(|d| d.id)
    }

    /// The active house pricing model: returns the model designated by
    /// [`ModelRegistry::set_active_pricing_model`] if set, or falls back to
    /// the first registered [`ModelKind::Pricing`] handle in stable insertion order.
    #[must_use]
    pub fn active_pricing_model(&self) -> Option<&dyn HostModel> {
        if let Some(id) = self.active_pricing_id {
            if let Ok(m) = self.model(id) {
                return Some(m);
            }
        }
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

    /// Atomically replace an existing native rates model in place or insert if new.
    pub fn replace_or_insert_native_rates<M: RatesPricingModel + 'static>(
        &mut self,
        kind: RatesProductKind,
        model: M,
    ) -> HostResult<ModelId> {
        let handle: Box<dyn RatesHostModel> = Box::new(NativeRatesModel::new(model));
        let descriptor = handle.descriptor();
        if let Some(pos) = self
            .rates_entries
            .iter()
            .position(|e| e.kind == kind && e.descriptor.id == descriptor.id)
        {
            self.rates_entries[pos] = RatesEntry {
                kind,
                descriptor,
                model: handle,
            };
            Ok(descriptor.id)
        } else {
            self.rates_entries.push(RatesEntry {
                kind,
                descriptor,
                model: handle,
            });
            Ok(descriptor.id)
        }
    }

    /// Explicitly designate the active rates model for a given [`RatesProductKind`] by [`ModelId`].
    pub fn set_active_rates_model(
        &mut self,
        kind: RatesProductKind,
        id: ModelId,
    ) -> HostResult<()> {
        if !self
            .rates_entries
            .iter()
            .any(|e| e.kind == kind && e.descriptor.id == id)
        {
            return Err(HostError::Model(PluginError::NotFound(
                "rates model id for kind",
            )));
        }
        if let Some(pos) = self.active_rates_ids.iter().position(|(k, _)| *k == kind) {
            self.active_rates_ids[pos].1 = id;
        } else {
            self.active_rates_ids.push((kind, id));
        }
        Ok(())
    }

    /// The id of the active rates model for `kind`, if set or resolved.
    #[must_use]
    pub fn active_rates_model_id(&self, kind: RatesProductKind) -> Option<ModelId> {
        if let Some((_, id)) = self.active_rates_ids.iter().find(|(k, _)| *k == kind) {
            if self
                .rates_entries
                .iter()
                .any(|e| e.kind == kind && e.descriptor.id == *id)
            {
                return Some(*id);
            }
        }
        self.rates_entries
            .iter()
            .find(|e| e.kind == kind)
            .map(|e| e.descriptor.id)
    }

    /// The active house rates model for `kind` — returns the model configured via
    /// [`ModelRegistry::set_active_rates_model`] if present, or falls back to the
    /// first registered rates model serving `kind`.
    #[must_use]
    pub fn active_rates_model(&self, kind: RatesProductKind) -> Option<&dyn RatesHostModel> {
        if let Some((_, id)) = self.active_rates_ids.iter().find(|(k, _)| *k == kind) {
            if let Some(entry) = self
                .rates_entries
                .iter()
                .find(|e| e.kind == kind && e.descriptor.id == *id)
            {
                return Some(entry.model.as_ref());
            }
        }
        self.rates_entries
            .iter()
            .find(|e| e.kind == kind)
            .map(|e| e.model.as_ref())
    }

    /// Register a compiled-in native (Tier-0) [`ExoticPricingModel`].
    pub fn register_native_exotic<M: ExoticPricingModel + 'static>(
        &mut self,
        model: M,
    ) -> HostResult<ModelId> {
        let handle: Box<dyn ExoticHostModel> = Box::new(NativeExoticModel::new(model));
        let descriptor = handle.descriptor();
        if self
            .exotic_entries
            .iter()
            .any(|e| e.descriptor.id == descriptor.id)
        {
            return Err(HostError::Model(PluginError::InvalidInput(
                "duplicate exotic model id",
            )));
        }
        self.exotic_entries.push(ExoticEntry {
            descriptor,
            model: handle,
        });
        Ok(descriptor.id)
    }

    /// Atomically replace an existing native exotic model in place or insert if new.
    pub fn replace_or_insert_native_exotic<M: ExoticPricingModel + 'static>(
        &mut self,
        model: M,
    ) -> HostResult<ModelId> {
        let handle: Box<dyn ExoticHostModel> = Box::new(NativeExoticModel::new(model));
        let descriptor = handle.descriptor();
        if let Some(pos) = self
            .exotic_entries
            .iter()
            .position(|e| e.descriptor.id == descriptor.id)
        {
            self.exotic_entries[pos] = ExoticEntry {
                descriptor,
                model: handle,
            };
            Ok(descriptor.id)
        } else {
            self.exotic_entries.push(ExoticEntry {
                descriptor,
                model: handle,
            });
            Ok(descriptor.id)
        }
    }

    /// Explicitly designate the active exotic model by [`ModelId`].
    pub fn set_active_exotic_model(&mut self, id: ModelId) -> HostResult<()> {
        if !self.exotic_entries.iter().any(|e| e.descriptor.id == id) {
            return Err(HostError::Model(PluginError::NotFound("exotic model id")));
        }
        self.active_exotic_id = Some(id);
        Ok(())
    }

    /// Borrow the exotic model handle for `id`.
    pub fn exotic_model(&self, id: ModelId) -> HostResult<&dyn ExoticHostModel> {
        self.exotic_entries
            .iter()
            .find(|e| e.descriptor.id == id)
            .map(|e| e.model.as_ref())
            .ok_or(HostError::Model(PluginError::NotFound("exotic model id")))
    }

    /// The active house exotic model — returns the designated active model if set,
    /// or falls back to the first registered exotic model in stable insertion order.
    #[must_use]
    pub fn active_exotic_model(&self) -> Option<&dyn ExoticHostModel> {
        if let Some(id) = self.active_exotic_id {
            if let Ok(m) = self.exotic_model(id) {
                return Some(m);
            }
        }
        self.exotic_entries.first().map(|e| e.model.as_ref())
    }

    /// Number of registered exotic models.
    #[must_use]
    pub fn exotic_len(&self) -> usize {
        self.exotic_entries.len()
    }

    /// Whether the registry holds no exotic models.
    #[must_use]
    pub fn exotic_is_empty(&self) -> bool {
        self.exotic_entries.is_empty()
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

    /// Native pricing models can be replaced in-place and active model can be selected dynamically.
    #[test]
    fn native_model_replace_in_place_and_active_selection() {
        use celnet_core::CarryInputs;
        use celnet_plugin_api::{GreekSupport, ModelDescriptor, ModelId, ModelKind, PricingModel};
        use celnet_types::{Carry, CcyPair, OptionType, Underlying};

        struct CustomPricer {
            id: ModelId,
            val: f64,
        }
        impl PricingModel for CustomPricer {
            fn descriptor(&self) -> ModelDescriptor {
                ModelDescriptor::new(self.id, ModelKind::Pricing, GreekSupport::PRICE_ONLY)
            }
            fn price(&self, _opt: OptionType, _inputs: &CarryInputs) -> celnet_plugin_api::PluginResult<f64> {
                Ok(self.val)
            }
        }

        let mut reg = ModelRegistry::new();
        let m1 = CustomPricer { id: ModelId("pricer.alpha"), val: 100.0 };
        let m2 = CustomPricer { id: ModelId("pricer.beta"), val: 200.0 };

        reg.register_native(m1).unwrap();
        reg.register_native(m2).unwrap();

        let inputs = CarryInputs::new(
            1.10, 1.10, 0.10, 1.0,
            Underlying::Fx(CcyPair::parse("EURUSD").unwrap()),
            Carry::FxRates { r_dom: 0.03, r_for: 0.01 },
        );

        // Default active model is first registered ("pricer.alpha")
        assert_eq!(reg.active_pricing_model_id(), Some(ModelId("pricer.alpha")));
        let p = reg.active_pricing_model().unwrap().price(OptionType::Call, &inputs).unwrap();
        assert_eq!(p, 100.0);

        // Replace pricer.alpha in place with updated calibration (val: 150.0)
        let m1_updated = CustomPricer { id: ModelId("pricer.alpha"), val: 150.0 };
        reg.replace_or_insert_native(m1_updated).unwrap();
        assert_eq!(reg.len(), 2); // Count didn't increase
        let p_updated = reg.active_pricing_model().unwrap().price(OptionType::Call, &inputs).unwrap();
        assert_eq!(p_updated, 150.0);

        // Switch active model to pricer.beta
        reg.set_active_pricing_model(ModelId("pricer.beta")).unwrap();
        assert_eq!(reg.active_pricing_model_id(), Some(ModelId("pricer.beta")));
        let p_beta = reg.active_pricing_model().unwrap().price(OptionType::Call, &inputs).unwrap();
        assert_eq!(p_beta, 200.0);
    }

    /// Rates models can be replaced in-place and active model can be selected dynamically.
    #[test]
    fn native_rates_model_replace_and_active_selection() {
        let mut reg = ModelRegistry::new();
        reg.register_native_rates(
            RatesProductKind::Ois,
            ConstantRatesModel::new(measures(10.0)),
        )
        .unwrap();

        let curve = [RatesCurvePillar::new(1.0, 0.04)];
        let p1 = reg
            .active_rates_model(RatesProductKind::Ois)
            .unwrap()
            .price(&ois_terms(), &curve)
            .unwrap();
        assert_eq!(p1.pv, 10.0);

        // In-place replacement
        reg.replace_or_insert_native_rates(
            RatesProductKind::Ois,
            ConstantRatesModel::new(measures(99.0)),
        )
        .unwrap();
        assert_eq!(reg.rates_len(), 1);

        let p2 = reg
            .active_rates_model(RatesProductKind::Ois)
            .unwrap()
            .price(&ois_terms(), &curve)
            .unwrap();
        assert_eq!(p2.pv, 99.0);
    }

    /// Native exotic models can be registered, replaced, and priced.
    #[test]
    fn native_exotic_model_registration_and_replacement() {
        use celnet_plugin_api::{
            ExoticArchetype, ExoticPayoffDescriptor, ExoticPricingModel, GreekSupport,
            ModelDescriptor, ModelId, ModelKind, MultiAssetInputs, PluginResult,
        };
        use celnet_types::{Ccy, CcyPair, Underlying};

        struct CustomBarrierPricer {
            pv_override: f64,
        }
        impl ExoticPricingModel for CustomBarrierPricer {
            fn descriptor(&self) -> ModelDescriptor {
                ModelDescriptor::new(
                    ModelId("desk.barrier"),
                    ModelKind::ExoticPricing,
                    GreekSupport::PRICE_ONLY,
                )
            }
            fn price_exotic(
                &self,
                _payoff: &ExoticPayoffDescriptor,
                _inputs: &MultiAssetInputs,
            ) -> PluginResult<f64> {
                Ok(self.pv_override)
            }
        }

        let mut reg = ModelRegistry::new();
        assert!(reg.exotic_is_empty());

        reg.register_native_exotic(CustomBarrierPricer { pv_override: 12.34 }).unwrap();
        assert_eq!(reg.exotic_len(), 1);

        let ccy_eur = Ccy::parse("EUR").unwrap();
        let u1 = Underlying::Fx(CcyPair::parse("EURUSD").unwrap());
        let inputs = MultiAssetInputs {
            underlyings: vec![u1],
            spots: vec![1.10],
            vols: vec![0.10],
            correlation_matrix: vec![1.0],
            expiry_years: 0.5,
            observation_schedule: vec![],
            past_fixings: vec![],
            numeraire: ccy_eur,
        };
        let payoff = ExoticPayoffDescriptor {
            archetype: ExoticArchetype::Barrier,
            strike: 1.10,
            upper_barrier: Some(1.20),
            lower_barrier: None,
            rebate: 0.0,
            weights: vec![1.0],
        };

        let pv = reg.active_exotic_model().unwrap().price_exotic(&payoff, &inputs).unwrap();
        assert_eq!(pv, 12.34);

        // Replace in place
        reg.replace_or_insert_native_exotic(CustomBarrierPricer { pv_override: 56.78 }).unwrap();
        assert_eq!(reg.exotic_len(), 1);
        let pv2 = reg.active_exotic_model().unwrap().price_exotic(&payoff, &inputs).unwrap();
        assert_eq!(pv2, 56.78);
    }
}
