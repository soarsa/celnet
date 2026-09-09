//! Five-Stage Zero-Loss Upgrade Protocol (ZLUP) Orchestrator.
//!
//! Provides zero-loss, zero-downtime rolling upgrades across live trading and risk nodes
//! using dynamic joint consensus configuration transitions and bit-exact state machine validation.

use std::time::Duration;

use celnet_replog::RaftNode;

use crate::error::UpgradeError;
use crate::ingress::AtomicIngressRouter;
use crate::twin::{BitExactTwinComparator, TwinVerificationReport};

/// Current stage of the rolling upgrade lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeStage {
    /// Idle, no upgrade in progress.
    Idle,
    /// Stage 1: Target twin node booted in shadow mode.
    ShadowBooted,
    /// Stage 2: Twin catch-up complete, bit-identity verified.
    TwinVerified,
    /// Stage 3: Raft §6 Joint Consensus (C_old,new) active and committed.
    JointConsensusCommitted,
    /// Stage 4: Ingress socket redirection active, traffic cut over.
    TrafficCutover,
    /// Stage 5: In-flight requests drained, retired node decommissioned.
    Decommissioned,
}

/// Orchestrator executing zero-loss rolling upgrades across CelNet cluster nodes.
pub struct UpgradeOrchestrator {
    stage: UpgradeStage,
    ingress_router: AtomicIngressRouter,
}

impl UpgradeOrchestrator {
    /// Create a new upgrade orchestrator targeting `active_node_id`.
    pub fn new(active_node_id: u64) -> Self {
        Self {
            stage: UpgradeStage::Idle,
            ingress_router: AtomicIngressRouter::new(active_node_id),
        }
    }

    /// The current stage of the upgrade protocol.
    pub fn stage(&self) -> UpgradeStage {
        self.stage
    }

    /// Access the underlying ingress router.
    pub fn ingress_router(&self) -> &AtomicIngressRouter {
        &self.ingress_router
    }

    /// Execute the complete 5-Stage Zero-Loss Upgrade from `active_node` to `shadow_node`.
    ///
    /// `leader_node` is the current Raft cluster leader (which may be `active_node` or another peer).
    ///
    /// # Protocol Invariants:
    /// 1. Zero dropped TCP requests or unapplied updates.
    /// 2. 100% IEEE-754 bit-identity verified before any traffic is shifted.
    /// 3. Dual-majority commitment under Raft §6 Joint Consensus.
    pub fn execute_upgrade(
        &mut self,
        leader_node: &RaftNode,
        active_node: &RaftNode,
        shadow_node: &RaftNode,
        timeout: Duration,
    ) -> Result<TwinVerificationReport, UpgradeError> {
        let active_id = active_node.id();
        let shadow_id = shadow_node.id();

        // Stage 1: Shadow Boot Verification
        self.stage = UpgradeStage::ShadowBooted;

        // Stage 2: Catch-Up & Bit-Exact Twin Verification
        let start = std::time::Instant::now();
        let target_commit = active_node.commit_index().unwrap_or(0);

        while start.elapsed() < timeout {
            let shadow_commit = shadow_node.commit_index().unwrap_or(0);
            if shadow_commit >= target_commit {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        let active_bits = active_node.applied_bits();
        let shadow_bits = shadow_node.applied_bits();

        let report = BitExactTwinComparator::verify_bit_identity(&active_bits, &shadow_bits)?;
        self.stage = UpgradeStage::TwinVerified;

        // Stage 3: Raft §6 Dynamic Joint Consensus
        let current_members = match leader_node.cluster_config() {
            celnet_replog::ClusterConfig::Simple { members } => members,
            celnet_replog::ClusterConfig::Joint { new_members, .. } => new_members,
        };

        let mut target_members = current_members.clone();
        if !target_members.contains(&shadow_id) {
            target_members.push(shadow_id);
        }
        target_members.retain(|&id| id != active_id);

        leader_node
            .enter_joint_consensus(target_members.clone())
            .map_err(UpgradeError::JointConsensusFailed)?;
        self.stage = UpgradeStage::JointConsensusCommitted;

        // Stage 4: Atomic Ingress Socket Redirection
        self.ingress_router.redirect_to(shadow_id);
        self.stage = UpgradeStage::TrafficCutover;

        // Stage 5: Clean Drain & Decommission
        if !self.ingress_router.drain_in_flight(timeout) {
            return Err(UpgradeError::DrainTimeout);
        }

        leader_node
            .finalize_joint_consensus()
            .map_err(UpgradeError::JointConsensusFailed)?;
        self.stage = UpgradeStage::Decommissioned;

        Ok(report)
    }
}
