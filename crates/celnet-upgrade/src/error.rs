//! Typed error taxonomy for Zero-Loss Upgrade Protocol (ZLUP).

use std::fmt;

/// Error arising during zero-loss rolling upgrade operations.
#[derive(Debug, PartialEq, Eq)]
pub enum UpgradeError {
    /// Bit-exact twin comparison failed: shadow state diverged from active state.
    TwinDivergence {
        /// Instrument key where divergence occurred.
        instrument_key: u64,
        /// Bit pattern observed on active node.
        active_bits: u64,
        /// Bit pattern observed on shadow candidate node.
        shadow_bits: u64,
    },
    /// Shadow node failed to catch up to active watermark within deadline.
    CatchUpTimeout {
        /// Target committed index to reach.
        target_index: u64,
        /// Current committed or applied index reached by shadow.
        current_index: Option<u64>,
    },
    /// Joint consensus configuration failed to commit across both quorums.
    JointConsensusFailed(String),
    /// In-flight traffic drain failed or timed out.
    DrainTimeout,
    /// Invalid upgrade state transition.
    InvalidState(String),
    /// Underneath consensus node error.
    Consensus(String),
    /// IO failure during socket redirection or journal access.
    Io(String),
}

impl fmt::Display for UpgradeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TwinDivergence {
                instrument_key,
                active_bits,
                shadow_bits,
            } => {
                write!(
                    f,
                    "Twin divergence at key {}: active {:#018x} vs shadow {:#018x}",
                    instrument_key, active_bits, shadow_bits
                )
            }
            Self::CatchUpTimeout {
                target_index,
                current_index,
            } => {
                write!(
                    f,
                    "Catch-up timed out: target index {}, current {:?}",
                    target_index, current_index
                )
            }
            Self::JointConsensusFailed(msg) => write!(f, "Joint consensus failed: {}", msg),
            Self::DrainTimeout => write!(f, "Drain timeout while waiting for in-flight requests"),
            Self::InvalidState(msg) => write!(f, "Invalid upgrade state: {}", msg),
            Self::Consensus(msg) => write!(f, "Consensus error: {}", msg),
            Self::Io(msg) => write!(f, "IO error: {}", msg),
        }
    }
}

impl std::error::Error for UpgradeError {}
