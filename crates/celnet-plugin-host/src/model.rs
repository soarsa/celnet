//! The tier-agnostic model handle the unified registry hands to callers.
//!
//! A caller (the engine) must price through Tier-0 native models and Tier-2
//! Wasm models **identically** — that interchangeability is requirement R5 of
//! `docs/PLUGIN-HOST-ALT.md`. [`HostModel`] is the single object-safe seam that
//! makes that true: it mirrors the [`celnet_plugin_api::PricingModel`] shape but
//! returns the richer [`crate::HostError`] (so sandbox outcomes like fuel
//! exhaustion are expressible), and it is the only type the
//! [`crate::ModelRegistry`] exposes. Whether a call is served by a compiled-in
//! `dyn PricingModel` or by a fuel-metered wasmi interpreter is invisible here.

use celnet_core::{CarryGreeks, CarryInputs};
use celnet_plugin_api::ModelDescriptor;
use celnet_types::OptionType;

use crate::error::HostResult;

/// A priceable model behind the unified registry, agnostic to its tier.
///
/// Method semantics match [`celnet_plugin_api::PricingModel`]; the only
/// difference is the error type, widened to [`crate::HostError`] so a sandbox
/// failure (capability denial, trap, fuel exhaustion) is representable alongside
/// the model's own [`celnet_plugin_api::PluginError`] (surfaced as
/// [`crate::HostError::Model`]).
///
/// Implementations may carry interior mutability (the Wasm tier owns a `Store`
/// it resets per call), so methods take `&self`; a single model handle is **not**
/// `Sync` and is intended to be owned by one pricing worker. The engine fans an
/// IB-sized portfolio across workers, each with its own handle, exactly as the
/// native trait registry already does.
pub trait HostModel {
    /// Self-description used by the registry to route work to this model.
    fn descriptor(&self) -> ModelDescriptor;

    /// Present value in the numeraire currency, per 1 unit of base notional.
    ///
    /// # Errors
    /// Returns [`crate::HostError::Model`] for a model-domain failure, or a
    /// sandbox failure ([`crate::HostError::Trapped`],
    /// [`crate::HostError::FuelExhausted`], …) for a Tier-2 model.
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> HostResult<f64>;

    /// Price together with the full carry-tagged Greek strip in a single pass.
    ///
    /// # Errors
    /// As [`HostModel::price`], plus [`crate::HostError::Model`] wrapping
    /// [`celnet_plugin_api::PluginError::Unsupported`] if the model advertises no
    /// Greek support.
    fn price_and_greeks(&self, opt: OptionType, inputs: &CarryInputs) -> HostResult<CarryGreeks>;
}
