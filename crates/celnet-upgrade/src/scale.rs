//! Enterprise Cluster Scale-Up, Scale-Down, and Fault-Tolerance Orchestration.
//!
//! Grounded in dynamic joint consensus configuration changes and high-assurance
//! adversarial network partition and crash resilience testing.

use std::time::{Duration, Instant};

use celnet_replog::RaftNode;

use crate::error::UpgradeError;
use crate::ingress::AtomicIngressRouter;
use crate::twin::{BitExactTwinComparator, TwinVerificationReport};

/// Summary report of a scale-up or scale-down operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleReport {
    /// Initial cluster size before scaling.
    pub initial_size: usize,
    /// Final cluster size after scaling.
    pub final_size: usize,
    /// Nodes added or removed.
    pub delta_nodes: Vec<u64>,
    /// Total duration elapsed during reconfiguration.
    pub duration_millis: u64,
    /// Verification report from twin state machine comparator.
    pub twin_report: TwinVerificationReport,
}

/// Orchestrator managing dynamic cluster expansion and contraction.
pub struct ClusterScaleManager;

impl ClusterScaleManager {
    /// Dynamically scale up a cluster by adding `new_nodes` to the active topology.
    ///
    /// # Protocol:
    /// 1. Bidirectional peer discovery: registers addresses between existing and new nodes.
    /// 2. Log replication & snapshot catch-up: waits until new nodes reach the leader's commit index.
    /// 3. Twin verification: asserts 100% IEEE-754 bit-identity before promoting.
    /// 4. Dynamic Joint Consensus: leader enters $C_{\text{old, new}}$ and finalizes $C_{\text{new}}$.
    pub fn scale_up(
        leader: &RaftNode,
        current_nodes: &[&RaftNode],
        new_nodes: &[&RaftNode],
        timeout: Duration,
    ) -> Result<ScaleReport, UpgradeError> {
        let start = Instant::now();
        let initial_size = current_nodes.len();
        let mut new_ids = Vec::with_capacity(new_nodes.len());

        // Step 1: Mutual peer registration
        for new_node in new_nodes {
            let n_id = new_node.id();
            let n_addr = new_node.addr();
            new_ids.push(n_id);

            // Existing nodes register new node
            for cur in current_nodes {
                cur.add_peer(n_id, n_addr);
            }

            // New node registers existing nodes
            for cur in current_nodes {
                new_node.add_peer(cur.id(), cur.addr());
            }

            // New nodes register each other
            for other_new in new_nodes {
                if other_new.id() != n_id {
                    new_node.add_peer(other_new.id(), other_new.addr());
                }
            }
        }

        // Step 2: Catch-up synchronization
        let target_commit = leader.commit_index().unwrap_or(0);
        let catchup_start = Instant::now();
        for new_node in new_nodes {
            while catchup_start.elapsed() < timeout {
                if new_node.commit_index().unwrap_or(0) >= target_commit {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if new_node.commit_index().unwrap_or(0) < target_commit {
                return Err(UpgradeError::CatchUpTimeout {
                    target_index: target_commit,
                    current_index: new_node.commit_index(),
                });
            }
        }

        // Step 3: Twin state verification
        let leader_bits = leader.applied_bits();
        for new_node in new_nodes {
            let new_bits = new_node.applied_bits();
            BitExactTwinComparator::verify_bit_identity(&leader_bits, &new_bits)?;
        }
        let twin_report = BitExactTwinComparator::verify_bit_identity(
            &leader_bits,
            &new_nodes[0].applied_bits(),
        )?;

        // Step 4: Dynamic Joint Consensus expansion
        let mut target_membership: Vec<u64> =
            current_nodes.iter().map(|n| n.id()).collect();
        target_membership.extend(new_ids.iter().copied());
        target_membership.sort_unstable();
        target_membership.dedup();

        leader
            .enter_joint_consensus(target_membership.clone())
            .map_err(UpgradeError::JointConsensusFailed)?;

        leader
            .finalize_joint_consensus()
            .map_err(UpgradeError::JointConsensusFailed)?;

        let final_size = leader.cluster_config().total_members();

        Ok(ScaleReport {
            initial_size,
            final_size,
            delta_nodes: new_ids,
            duration_millis: start.elapsed().as_millis() as u64,
            twin_report,
        })
    }

    /// Dynamically scale down a cluster by gracefully removing `retiring_nodes`.
    ///
    /// # Protocol:
    /// 1. Ingress traffic drain: redirects and drains client traffic from retiring nodes.
    /// 2. Dynamic Joint Consensus contraction: transitions to $C_{\text{new}}$ excluding retiring nodes.
    /// 3. Peer disconnection: removes retiring nodes from surviving peers' topologies.
    /// 4. Clean decommission: shuts down retired nodes cleanly.
    pub fn scale_down(
        leader: &RaftNode,
        surviving_nodes: &[&RaftNode],
        retiring_nodes: Vec<RaftNode>,
        ingress: Option<&AtomicIngressRouter>,
        timeout: Duration,
    ) -> Result<ScaleReport, UpgradeError> {
        let start = Instant::now();
        let initial_size = surviving_nodes.len() + retiring_nodes.len();
        let retiring_ids: Vec<u64> = retiring_nodes.iter().map(|n| n.id()).collect();

        // Step 1: Traffic redirection and drain
        if let Some(router) = ingress {
            if retiring_ids.contains(&router.active_target()) {
                let fallback = surviving_nodes[0].id();
                router.redirect_to(fallback);
            }
            if !router.drain_in_flight(timeout) {
                return Err(UpgradeError::DrainTimeout);
            }
        }

        // Step 2: Dynamic Joint Consensus contraction
        let mut target_membership: Vec<u64> =
            surviving_nodes.iter().map(|n| n.id()).collect();
        target_membership.sort_unstable();
        target_membership.dedup();

        leader
            .enter_joint_consensus(target_membership)
            .map_err(UpgradeError::JointConsensusFailed)?;

        leader
            .finalize_joint_consensus()
            .map_err(UpgradeError::JointConsensusFailed)?;

        // Step 3: Peer disconnection
        for s in surviving_nodes {
            for &r_id in &retiring_ids {
                s.remove_peer(r_id);
            }
        }

        // Step 4: Decommission retiring nodes
        for r in retiring_nodes {
            r.shutdown();
        }

        // Step 5: Bit-identity verification on survivors after catch-up
        let target_commit = leader.commit_index().unwrap_or(0);
        let catchup_start = Instant::now();
        for s in surviving_nodes {
            while catchup_start.elapsed() < timeout {
                if s.commit_index().unwrap_or(0) >= target_commit {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if s.commit_index().unwrap_or(0) < target_commit {
                return Err(UpgradeError::CatchUpTimeout {
                    target_index: target_commit,
                    current_index: s.commit_index(),
                });
            }
        }

        let leader_bits = leader.applied_bits();
        for s in surviving_nodes {
            let s_bits = s.applied_bits();
            BitExactTwinComparator::verify_bit_identity(&leader_bits, &s_bits)?;
        }
        let twin_report = TwinVerificationReport {
            total_keys_verified: leader_bits.len(),
            is_bit_identical: true,
            verified_at_epoch_nanos: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
        };

        let final_size = leader.cluster_config().total_members();

        Ok(ScaleReport {
            initial_size,
            final_size,
            delta_nodes: retiring_ids,
            duration_millis: start.elapsed().as_millis() as u64,
            twin_report,
        })
    }
}

/// Adversarial fault injection and cluster resilience testing harness.
pub struct ChaosEngine;

/// Institutional alias for fault injection engine.
pub type FaultInjectionEngine = ChaosEngine;
/// Institutional alias for cluster resilience engine.
pub type ClusterResilienceEngine = ChaosEngine;

impl ChaosEngine {
    /// Sever communication between a minority set and a majority set to simulate a network partition.
    pub fn isolate_nodes(minority: &[&RaftNode], majority: &[&RaftNode]) {
        for m in minority {
            for maj in majority {
                m.remove_peer(maj.id());
                maj.remove_peer(m.id());
            }
        }
    }

    /// Reconnect partitioned nodes and restore network topology.
    pub fn heal_partition(minority: &[&RaftNode], majority: &[&RaftNode]) {
        for m in minority {
            for maj in majority {
                m.add_peer(maj.id(), maj.addr());
                maj.add_peer(m.id(), m.addr());
            }
        }
    }
}
