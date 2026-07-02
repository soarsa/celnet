//! The crate's single error type for scenario construction and repricing.

use celnet_bond::BondError;
use celnet_rates::CurveError;

/// A failure while building a shocked curve or repricing a fixed-income position under one.
///
/// The three arms are the only ways the rate-scenario engine can fail: a shock vector whose
/// length does not match the curve's dated-pillar count, an invalid shocked curve (e.g. a shock
/// large enough to drive a discount factor non-positive — surfaced by the curve builder), and a
/// bond that cannot be scheduled/priced. OIS repricing is infallible (a valid schedule always
/// prices on a valid curve), so it contributes no arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateRiskError {
    /// A [`crate::RateShock`] carried a different number of per-pillar shifts than the curve has
    /// dated pillars, so it cannot be applied.
    ShockLength {
        /// The number of dated pillars the base curve carries (the required shift length).
        expected: usize,
        /// The number of shifts the offending shock actually carried.
        got: usize,
    },
    /// The shocked pillar set was rejected by the curve builder (propagated [`CurveError`]).
    Curve(CurveError),
    /// A cash bond could not be scheduled or priced (propagated [`BondError`]).
    Bond(BondError),
}

impl core::fmt::Display for RateRiskError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShockLength { expected, got } => write!(
                f,
                "rate shock has {got} shifts but the curve has {expected} dated pillars"
            ),
            Self::Curve(e) => write!(f, "shocked curve is invalid: {e}"),
            Self::Bond(e) => write!(f, "bond reprice failed: {e}"),
        }
    }
}

impl core::error::Error for RateRiskError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::ShockLength { .. } => None,
            Self::Curve(e) => Some(e),
            Self::Bond(e) => Some(e),
        }
    }
}

impl From<CurveError> for RateRiskError {
    fn from(e: CurveError) -> Self {
        Self::Curve(e)
    }
}

impl From<BondError> for RateRiskError {
    fn from(e: BondError) -> Self {
        Self::Bond(e)
    }
}
