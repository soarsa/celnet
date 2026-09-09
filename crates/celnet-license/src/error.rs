//! Typed error taxonomy for cryptographic licensing and capability verification.

use thiserror::Error;

/// Error arising during license token verification or capability evaluation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LicenseError {
    /// Invalid cryptographic signature on token root or block.
    #[error("invalid cryptographic signature: {0}")]
    InvalidSignature(String),

    /// Token attenuation chain is broken or hashes do not link.
    #[error("broken attenuation chain at block {index}: expected {expected}, got {actual}")]
    BrokenAttenuationChain {
        /// Block index where the hash break occurred.
        index: usize,
        /// Expected SHA/BLAKE hash.
        expected: String,
        /// Actual computed hash.
        actual: String,
    },

    /// License has expired based on ambient time.
    #[error("license expired: valid until {expires_at}, current time is {current_time}")]
    LicenseExpired {
        /// Expiration UNIX timestamp in seconds.
        expires_at: u64,
        /// Ambient UNIX timestamp in seconds.
        current_time: u64,
    },

    /// Requested cores exceed licensed capacity.
    #[error("core quota exceeded: requested {requested}, licensed maximum is {max_cores}")]
    CoreQuotaExceeded {
        /// Cores requested by the node.
        requested: usize,
        /// Maximum cores permitted by license.
        max_cores: usize,
    },

    /// Required capability or asset class not granted by Datalog policy.
    #[error("capability denied: {0}")]
    CapabilityDenied(String),

    /// Datalog evaluation failure or unsatisfiable caveat.
    #[error("datalog verification check failed: {0}")]
    DatalogCheckFailed(String),

    /// Hardware attestation failure (TPM quote mismatch or untrusted enclave).
    #[error("hardware attestation failed: {0}")]
    AttestationFailed(String),

    /// Serialization or parsing error.
    #[error("license serialization error: {0}")]
    Serialization(String),
}
