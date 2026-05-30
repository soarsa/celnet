//! The calibration contract.
//!
//! [`Calibration`] is the seam for fitting a model's parameters to observed
//! market quotes — a smile to broker risk-reversal/butterfly/ATM marks, or a
//! pricing model to a set of premiums. It is generic over the produced model so
//! a calibrator yields a *ready-to-use* [`SmileModel`] or [`PricingModel`], and
//! over the target type so the same trait serves vol-quote and premium-quote
//! fits. Like the other SDK seams it is deterministic and the host runs it
//! inside the fuel-metered sandbox for untrusted user calibrators.

use crate::descriptor::ModelDescriptor;
use crate::error::PluginResult;

/// A single calibration target: an observed quantity the calibrated model must
/// reproduce, together with the inverse-variance `weight` it carries in the
/// objective (`0.0` excludes it; larger means tighter fit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationTarget {
    /// The abscissa the quote is observed at (e.g. a strike, or a signed delta in
    /// `[-1, 1]`), interpreted by the concrete calibrator.
    pub abscissa: f64,
    /// The observed value to match (e.g. a Black vol, or a premium).
    pub observed: f64,
    /// Relative weight in the least-squares objective.
    pub weight: f64,
}

impl CalibrationTarget {
    /// Convenience constructor with unit weight.
    #[must_use]
    pub const fn new(abscissa: f64, observed: f64) -> Self {
        Self {
            abscissa,
            observed,
            weight: 1.0,
        }
    }
}

/// Diagnostics returned alongside a calibrated model so callers can audit fit
/// quality without re-running the objective.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationReport {
    /// Number of iterations the solver took.
    pub iterations: u32,
    /// Final root-mean-square residual across the weighted targets.
    pub rms_residual: f64,
    /// Largest absolute residual across the targets.
    pub max_residual: f64,
}

/// Fits a model to market targets, producing a ready-to-evaluate model.
///
/// `Model` is the concrete model the fit yields (typically implementing
/// [`crate::SmileModel`] or [`crate::PricingModel`]); `Target` is the observed
/// quote type, defaulting to [`CalibrationTarget`].
pub trait Calibration {
    /// The model type this calibrator produces.
    type Model;
    /// The observed-quote type this calibrator fits to.
    type Target;

    /// Self-description used by the registry to route work to this calibrator.
    fn descriptor(&self) -> ModelDescriptor;

    /// Fit the model to `targets` at the given outright `forward` and
    /// time-to-expiry `t` (years), returning the calibrated model and a
    /// [`CalibrationReport`].
    ///
    /// Implementors must be deterministic (a fixed target set yields a
    /// bit-identical model) and must route their numerics through
    /// `celnet_core::math`.
    ///
    /// # Errors
    /// Returns [`crate::PluginError::InvalidInput`] for ill-posed targets
    /// (e.g. non-positive `forward`/`t`, too few targets, all-zero weights),
    /// [`crate::PluginError::DidNotConverge`] if the solver hits its iteration
    /// budget, or [`crate::PluginError::CalibrationFailed`] if it converges
    /// outside tolerance.
    fn calibrate(
        &self,
        forward: f64,
        t: f64,
        targets: &[Self::Target],
    ) -> PluginResult<(Self::Model, CalibrationReport)>;
}
