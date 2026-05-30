//! The volatility-smile-model contract.
//!
//! [`SmileModel`] extends the core [`celnet_core::Smile`] seam (an implied vol at
//! a strike) with the plugin-registry obligations: self-description and an
//! arbitrage self-check the host can run during the deterministic replay before
//! a user surface is admitted to the hot path. Keeping `Smile` as the supertrait
//! means a `SmileModel` is usable anywhere the vanilla/exotic engines already
//! accept a `Smile`, with no adapter.

use celnet_core::Smile;
use celnet_types::Vol;

use crate::descriptor::ModelDescriptor;
use crate::error::PluginResult;

/// A user- or first-party-supplied volatility smile/surface.
///
/// Implementors get [`Smile::implied_vol`] / [`Smile::atm_forward_vol`] from the
/// core supertrait; this trait adds the registry metadata and an explicit
/// no-arbitrage check so the host can reject a static-arbitrageable surface
/// before it ever prices a book.
pub trait SmileModel: Smile {
    /// Self-description used by the registry to route work to this model.
    fn descriptor(&self) -> ModelDescriptor;

    /// Implied (Black) volatility, surfacing convergence/domain failures as a
    /// [`crate::PluginError`] rather than the infallible [`Smile::implied_vol`].
    ///
    /// The default delegates to the infallible supertrait method; a model whose
    /// evaluation can fail (e.g. an out-of-range extrapolation it refuses to make)
    /// overrides this to return the appropriate error.
    ///
    /// # Errors
    /// Returns [`crate::PluginError::InvalidInput`] for a non-positive
    /// strike/forward or negative time, or
    /// [`crate::PluginError::Unsupported`] for a strike outside the model's
    /// calibrated range.
    fn try_implied_vol(&self, strike: f64, forward: f64, t: f64) -> PluginResult<Vol> {
        Ok(self.implied_vol(strike, forward, t))
    }

    /// Verify the smile is free of **static (butterfly) arbitrage** at `forward`
    /// / time `t` over the inclusive strike grid `strikes`, i.e. that the implied
    /// risk-neutral density (the second strike-derivative of the undiscounted
    /// call price) is non-negative everywhere on the grid.
    ///
    /// The default supplies a model-agnostic finite-difference check so every
    /// `SmileModel` is arbitrage-checkable out of the box; a model that knows it
    /// is arbitrage-free by construction may override with a cheaper proof. The
    /// check uses only `celnet_core::math` (deterministic, cross-platform).
    ///
    /// # Errors
    /// Returns [`crate::PluginError::InvalidInput`] if `forward`/`t` are
    /// non-positive or the grid has fewer than three strictly-increasing strikes;
    /// [`crate::PluginError::Unsupported`] (carrying `"butterfly arbitrage"`) if a
    /// negative density is detected.
    fn check_no_arbitrage(&self, forward: f64, t: f64, strikes: &[f64]) -> PluginResult<()> {
        no_arb::butterfly_check(self, forward, t, strikes)
    }
}

mod no_arb {
    //! Model-agnostic static-arbitrage check for a [`super::SmileModel`].
    //!
    //! Static (butterfly) arbitrage is absent iff the undiscounted call price
    //! `C(K) = F·Φ(d₁) − K·Φ(d₂)` (Black, expressed off the forward) is convex in
    //! strike: `∂²C/∂K² = φ(d₂)/(K·σ√t) ≥ 0`, which is the risk-neutral density.
    //! We evaluate `C` with each strike's *own* smile vol and confirm a
    //! second-difference `C(Kᵢ₋₁) − 2·C(Kᵢ) + C(Kᵢ₊₁) ≥ 0` on the user grid — a
    //! direct, model-free density-positivity test. (Cf. the butterfly /
    //! call-spread no-arbitrage conditions, e.g. the Carr–Madan density
    //! representation; provenance in this doc comment only.)

    use celnet_core::is_close;
    use celnet_core::math::{ln, norm_cdf, sqrt};

    use super::SmileModel;
    use crate::error::{PluginError, PluginResult};

    /// Undiscounted Black call value off the forward, at strike `k` with vol `v`.
    fn undiscounted_call(forward: f64, k: f64, v: f64, t: f64) -> f64 {
        let vsqt = v * sqrt(t);
        // Degenerate vol/time ⇒ intrinsic on the forward.
        if vsqt <= 0.0 {
            return (forward - k).max(0.0);
        }
        let d1 = (ln(forward / k) + 0.5 * v * v * t) / vsqt;
        let d2 = d1 - vsqt;
        forward * norm_cdf(d1) - k * norm_cdf(d2)
    }

    pub(super) fn butterfly_check<M: SmileModel + ?Sized>(
        model: &M,
        forward: f64,
        t: f64,
        strikes: &[f64],
    ) -> PluginResult<()> {
        if !forward.is_finite() || forward <= 0.0 || !t.is_finite() || t <= 0.0 {
            return Err(PluginError::InvalidInput("forward and t must be positive"));
        }
        if strikes.len() < 3 {
            return Err(PluginError::InvalidInput(
                "need at least three strikes for a convexity check",
            ));
        }
        // Require a strictly-increasing grid (a `NaN` strike fails this too,
        // since it compares false to its neighbour).
        for w in strikes.windows(2) {
            if w[0].is_nan() || w[1] <= w[0] {
                return Err(PluginError::InvalidInput(
                    "strikes must be strictly increasing",
                ));
            }
        }

        for w in strikes.windows(3) {
            let (kl, km, kr) = (w[0], w[1], w[2]);
            let cl = undiscounted_call(forward, kl, model.implied_vol(kl, forward, t).0, t);
            let cm = undiscounted_call(forward, km, model.implied_vol(km, forward, t).0, t);
            let cr = undiscounted_call(forward, kr, model.implied_vol(kr, forward, t).0, t);
            let second_diff = cl - 2.0 * cm + cr;
            // Allow a tiny negative within tolerance (rounding), reject the rest.
            if second_diff < 0.0 && !is_close(second_diff, 0.0, 1e-9, 1e-12) {
                return Err(PluginError::Unsupported("butterfly arbitrage"));
            }
        }
        Ok(())
    }
}
