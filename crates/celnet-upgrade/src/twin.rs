//! IEEE-754 Bit-Exact Twin Comparator.
//!
//! Evaluates deterministic state identity between an active production node (V_n)
//! and a candidate shadow twin (V_n+1) before traffic cutover is authorized.
//!
//! Adheres strictly to CelNet's bit-identity oracle invariants:
//! No float comparisons via `==` or epsilon thresholds; all evaluations compare
//! raw IEEE-754 binary bit patterns (`f64::to_bits`).

use crate::error::UpgradeError;

/// Report detailing the outcome of a twin state machine comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwinVerificationReport {
    /// Total instrument prices verified.
    pub total_keys_verified: usize,
    /// Whether bit-exact equivalence was satisfied across 100% of keys.
    pub is_bit_identical: bool,
    /// Execution timestamp of comparison (nanoseconds since epoch).
    pub verified_at_epoch_nanos: u64,
}

/// Bit-exact comparator verifying twin node state machines.
pub struct BitExactTwinComparator;

impl BitExactTwinComparator {
    /// Compare the applied states of the active node and shadow candidate.
    ///
    /// Accepts sorted `(instrument_key, float_bits)` sequences from `RaftNode::applied_bits()`.
    ///
    /// # Errors
    /// Returns `UpgradeError::TwinDivergence` on the first observed bit mismatch.
    pub fn verify_bit_identity(
        active_bits: &[(u64, u64)],
        shadow_bits: &[(u64, u64)],
    ) -> Result<TwinVerificationReport, UpgradeError> {
        if active_bits.len() != shadow_bits.len() {
            // Find divergence point if keys differ
            let (act_key, act_b, sh_b) = active_bits
                .iter()
                .zip(shadow_bits.iter())
                .find(|((k1, b1), (k2, b2))| k1 != k2 || b1 != b2)
                .map(|((k1, b1), (_, b2))| (*k1, *b1, *b2))
                .unwrap_or_else(|| {
                    let k = active_bits.first().map(|(k, _)| *k).unwrap_or(0);
                    (k, 0, 0)
                });

            return Err(UpgradeError::TwinDivergence {
                instrument_key: act_key,
                active_bits: act_b,
                shadow_bits: sh_b,
            });
        }

        for ((k1, b1), (k2, b2)) in active_bits.iter().zip(shadow_bits.iter()) {
            if k1 != k2 || b1 != b2 {
                return Err(UpgradeError::TwinDivergence {
                    instrument_key: *k1,
                    active_bits: *b1,
                    shadow_bits: *b2,
                });
            }
        }

        Ok(TwinVerificationReport {
            total_keys_verified: active_bits.len(),
            is_bit_identical: true,
            verified_at_epoch_nanos: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_twin_comparator_bit_identical() {
        let active = vec![(1, 1.2345f64.to_bits()), (2, 99.999f64.to_bits())];
        let shadow = vec![(1, 1.2345f64.to_bits()), (2, 99.999f64.to_bits())];

        let report = BitExactTwinComparator::verify_bit_identity(&active, &shadow).unwrap();
        assert!(report.is_bit_identical);
        assert_eq!(report.total_keys_verified, 2);
    }

    #[test]
    fn test_twin_comparator_divergence_rejected() {
        let active = vec![(1, 1.2345f64.to_bits())];
        // 1 ULP off
        let shadow = vec![(1, 1.2345f64.to_bits() + 1)];

        let err = BitExactTwinComparator::verify_bit_identity(&active, &shadow).unwrap_err();
        match err {
            UpgradeError::TwinDivergence { instrument_key, active_bits, shadow_bits } => {
                assert_eq!(instrument_key, 1);
                assert_eq!(active_bits, 1.2345f64.to_bits());
                assert_eq!(shadow_bits, 1.2345f64.to_bits() + 1);
            }
            other => panic!("expected TwinDivergence, got {:?}", other),
        }
    }
}
