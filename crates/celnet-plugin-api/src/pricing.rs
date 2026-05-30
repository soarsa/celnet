//! The pricing-model contract.
//!
//! [`PricingModel`] is the seam both the native trait registry (compiled-in
//! first-party models, hot path) and the Wasm component host (sandboxed user
//! models) implement, so first-party and user models are *interchangeable*
//! behind one registry (see `docs/ARCHITECTURE.md` §6). A model receives the
//! canonical [`celnet_types::VanillaInputs`] and the [`celnet_types::OptionType`]
//! and returns a domestic-currency price; Greeks are an opt-in extension a model
//! advertises through its [`ModelDescriptor`] so the engine never asks for a
//! sensitivity the model cannot produce.
//!
//! The trait is deliberately *side-effect free and allocation-free on the hot
//! path*: a call takes `&self` plus `Copy` POD inputs and returns `Copy` POD
//! outputs, which is exactly the shape that marshals across the Wasm boundary
//! and that the engine can fan out across an IB-sized portfolio without locks.

use celnet_types::{Greeks, OptionType, VanillaInputs};

use crate::descriptor::ModelDescriptor;
use crate::error::PluginResult;

/// A model that prices vanilla FX options and, optionally, their Greeks.
///
/// Implementors must be **deterministic** (same inputs ⇒ bit-identical output)
/// and must route transcendentals through `celnet_core::math` so results are
/// reproducible across platforms; they must never compare floats with `==` or
/// assert on `NaN` (use `celnet_core::is_close`). These are contract
/// obligations, not merely conventions — the deterministic replay harness in the
/// plugin host asserts them.
pub trait PricingModel {
    /// Self-description used by the registry to route work to this model.
    fn descriptor(&self) -> ModelDescriptor;

    /// Present value in domestic currency, per 1 unit of base notional.
    ///
    /// # Errors
    /// Returns [`crate::PluginError::InvalidInput`] if the inputs are outside the
    /// model's domain (e.g. non-positive spot/strike, negative time, non-finite
    /// vol), or [`crate::PluginError::DidNotConverge`] if an internal numerical
    /// routine fails to converge.
    fn price(&self, opt: OptionType, inputs: &VanillaInputs) -> PluginResult<f64>;

    /// Price together with the full Celnet Greek set in a single pass.
    ///
    /// The default implementation reports the model does not produce Greeks; a
    /// model that advertises Greek support in its [`ModelDescriptor`] overrides
    /// this. Producing price and Greeks together lets a model share intermediate
    /// quantities (the `d1`/`d2` block) rather than recomputing them.
    ///
    /// # Errors
    /// Returns [`crate::PluginError::Unsupported`] if the model does not produce
    /// Greeks, or the same input/convergence errors as [`PricingModel::price`].
    fn price_and_greeks(&self, opt: OptionType, inputs: &VanillaInputs) -> PluginResult<Greeks> {
        let _ = (opt, inputs);
        Err(crate::error::PluginError::Unsupported(
            "model does not produce Greeks",
        ))
    }
}
