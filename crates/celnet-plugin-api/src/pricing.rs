//! The pricing-model contract.
//!
//! [`PricingModel`] is the seam both the native trait registry (compiled-in
//! first-party models, hot path) and the Wasm component host (sandboxed user
//! models) implement, so first-party and user models are *interchangeable*
//! behind one registry (see `docs/ARCHITECTURE.md` §6). A model receives the
//! generalized, carry-tagged [`celnet_core::CarryInputs`] and the
//! [`celnet_types::OptionType`] and returns a numeraire-currency price; the
//! sensitivity strip is the carry-tagged [`celnet_core::CarryGreeks`], whose rate
//! block ([`celnet_types::RateSensitivities`]) reports the FX two rhos *or* the
//! discount/carry rho pair depending on the asset class. Greeks are an opt-in
//! extension a model advertises through its [`ModelDescriptor`] so the engine
//! never asks for a sensitivity the model cannot produce.
//!
//! # Why the generalized vocabulary
//!
//! The contract names the [`celnet_core::CarryInputs`] (`spot, strike, vol, t`
//! plus an [`celnet_types::Underlying`] and a [`celnet_types::Carry`]), not the
//! FX-only `VanillaInputs`. This lets a user author a **non-FX** model — e.g. an
//! equity option under [`celnet_types::Carry::CostOfCarry`] with net carry
//! `b = r − q` — against the *same* extensibility contract that serves FX. The FX
//! arm is exactly [`celnet_types::Carry::FxRates`] and reproduces the FX two-rate
//! arithmetic bit-for-bit (proven in `celnet-core`); a model that only prices FX
//! simply rejects a [`celnet_types::Carry::CostOfCarry`] input with a typed
//! [`celnet_core::CarryPriceError`]-shaped [`crate::PluginError`].
//!
//! The trait is deliberately *side-effect free and allocation-free on the hot
//! path*: a call takes `&self` plus `Copy` POD inputs and returns `Copy` POD
//! outputs, which is exactly the shape that marshals across the Wasm boundary
//! and that the engine can fan out across an IB-sized portfolio without locks.

use celnet_core::{CarryGreeks, CarryInputs};
use celnet_types::OptionType;

use crate::descriptor::ModelDescriptor;
use crate::error::PluginResult;

/// A model that prices options over the generalized carry vocabulary and,
/// optionally, their carry-tagged Greeks.
///
/// Implementors must be **deterministic** (same inputs ⇒ bit-identical output)
/// and must route transcendentals through `celnet_core::math` so results are
/// reproducible across platforms; they must never compare floats with `==` or
/// assert on `NaN` (use `celnet_core::is_close`). These are contract
/// obligations, not merely conventions — the deterministic replay harness in the
/// plugin host asserts them.
///
/// A model declares the asset classes it handles by accepting or rejecting the
/// [`CarryInputs`]'s [`celnet_types::Underlying`] / [`celnet_types::Carry`]: an
/// FX-only model returns [`crate::PluginError::Unsupported`] for a
/// [`celnet_types::Carry::CostOfCarry`] input rather than silently mis-pricing
/// it under FX arithmetic, exactly as a [`celnet_core::CarryPricer`] leaf does.
pub trait PricingModel {
    /// Self-description used by the registry to route work to this model.
    fn descriptor(&self) -> ModelDescriptor;

    /// Present value in the numeraire currency, per 1 unit of base notional.
    ///
    /// # Errors
    /// Returns [`crate::PluginError::InvalidInput`] if the inputs are outside the
    /// model's domain (e.g. non-positive spot/strike, negative time, non-finite
    /// vol), [`crate::PluginError::Unsupported`] if the model does not price the
    /// given underlying/carry asset class, or
    /// [`crate::PluginError::DidNotConverge`] if an internal numerical routine
    /// fails to converge.
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> PluginResult<f64>;

    /// Price together with the full carry-tagged Greek strip in a single pass.
    ///
    /// The default implementation reports the model does not produce Greeks; a
    /// model that advertises Greek support in its [`ModelDescriptor`] overrides
    /// this. Producing price and Greeks together lets a model share intermediate
    /// quantities (the `d1`/`d2` block) rather than recomputing them.
    ///
    /// # Errors
    /// Returns [`crate::PluginError::Unsupported`] if the model does not produce
    /// Greeks, or the same input/convergence errors as [`PricingModel::price`].
    fn price_and_greeks(&self, opt: OptionType, inputs: &CarryInputs) -> PluginResult<CarryGreeks> {
        let _ = (opt, inputs);
        Err(crate::error::PluginError::Unsupported(
            "model does not produce Greeks",
        ))
    }
}
