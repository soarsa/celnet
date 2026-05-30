//! The single, current error contract for SDK plugins.
//!
//! Errors crossing the SDK boundary must be **portable**: the same shape is
//! returned by a compiled-in native model (the trait registry) and by a
//! sandboxed Wasm component (the host marshals the WIT `result<_, plugin-error>`
//! into this type). The variants are therefore deliberately coarse and free of
//! host-specific detail (no `std::io`, no backtraces), so they survive the
//! Wasm/native boundary without loss and remain `Copy`-friendly on the hot path.

use core::fmt;

/// Why a plugin call could not produce a result.
///
/// Kept small and `Clone` (never carrying an OS handle or allocation-heavy
/// payload) so it can be returned identically by native and Wasm models.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginError {
    /// An input was outside the model's domain (e.g. negative time, non-positive
    /// spot/strike, non-finite vol). The `&'static str` names the offending field
    /// or constraint; it is provenance-neutral and contains no user data.
    InvalidInput(&'static str),
    /// The model was asked for a capability it does not provide (e.g. Greeks from
    /// a price-only model, or a tenor/strike outside its calibrated range).
    Unsupported(&'static str),
    /// A numerical routine failed to converge within its iteration/tolerance
    /// budget (root-find, calibration, implied-vol inversion).
    DidNotConverge(&'static str),
    /// Calibration could not fit the supplied targets within tolerance.
    CalibrationFailed(&'static str),
    /// A named model/plugin was requested from a registry but is not present.
    NotFound(&'static str),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginError::InvalidInput(w) => write!(f, "invalid input: {w}"),
            PluginError::Unsupported(w) => write!(f, "unsupported: {w}"),
            PluginError::DidNotConverge(w) => write!(f, "did not converge: {w}"),
            PluginError::CalibrationFailed(w) => write!(f, "calibration failed: {w}"),
            PluginError::NotFound(w) => write!(f, "not found: {w}"),
        }
    }
}

impl core::error::Error for PluginError {}

/// The single result alias used across the SDK surface.
pub type PluginResult<T> = Result<T, PluginError>;
