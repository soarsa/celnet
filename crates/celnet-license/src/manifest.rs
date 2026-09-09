//! Capability Manifest derived from verified cryptographic tokens.

use std::collections::HashSet;
use serde::{Deserialize, Serialize};

use crate::error::LicenseError;
use crate::nhed::NodeHardwareDescriptor;

/// Licensed functional capability tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LicenseTier {
    /// Baseline vanilla option and linear pricing.
    CorePricing,
    /// Ultra-low-latency SBE wire encoding and kernel-bypass ingress.
    UltraLowLatencySbe,
    /// Dual-curve discounting, cash bonds, and pool factor corporate action revaluation.
    RatesAndBonds,
    /// Multi-tier exotics, LSV particle models, and structured payoffs.
    ExoticsAndStructured,
    /// Distributed Multi-Raft consensus and SIMD scenario grid risk fleet.
    DistributedRiskFleet,
    /// Hardware-accelerated GPU compute kernels and batch Greeks.
    GpuAadAcceleration,
    /// ISDA SIMM 2.6 and CME SPAN portfolio margin calculation.
    PortfolioMargin,
    /// Algorithmic order execution and market impact slicing (TWAP/POV).
    AlgoExecution,
}

/// Authorized capabilities granted to an individual node or cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityManifest {
    /// Licensed institutional tenant identifier.
    pub tenant_id: String,
    /// Set of authorized asset class identifiers (e.g. "FX", "RATES", "EQUITY").
    pub licensed_asset_classes: HashSet<String>,
    /// Set of authorized operational tiers.
    pub licensed_tiers: HashSet<LicenseTier>,
    /// Maximum execution cores permitted under this license.
    pub max_cores: usize,
    /// Maximum message throughput ceiling in messages/sec.
    pub max_throughput_msg_per_sec: u64,
    /// Expiration timestamp in seconds since UNIX epoch.
    pub expires_at: u64,
}

impl CapabilityManifest {
    /// Check if a specific asset class and capability tier are authorized.
    pub fn is_feature_authorized(&self, asset_class: Option<&str>, tier: LicenseTier) -> bool {
        if !self.licensed_tiers.contains(&tier) {
            return false;
        }
        if let Some(asset) = asset_class {
            let asset_upper = asset.to_uppercase();
            if !self.licensed_asset_classes.contains(&asset_upper) {
                return false;
            }
        }
        true
    }

    /// Validate the capability manifest against physical node hardware and current timestamp.
    pub fn validate_node(
        &self,
        hw: &NodeHardwareDescriptor,
        current_time_secs: u64,
    ) -> Result<(), LicenseError> {
        if current_time_secs >= self.expires_at {
            return Err(LicenseError::LicenseExpired {
                expires_at: self.expires_at,
                current_time: current_time_secs,
            });
        }

        if hw.physical_cores > self.max_cores {
            return Err(LicenseError::CoreQuotaExceeded {
                requested: hw.physical_cores,
                max_cores: self.max_cores,
            });
        }

        Ok(())
    }
}
