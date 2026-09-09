//! Raft §6 Dynamic Cluster Membership Changes (Joint Consensus)
//!
//! Grounded in Ongaro's PhD thesis (2014) §4 and Raft §6.
//!
//! Provides zero-downtime configuration transitions using Joint Consensus:
//! 1. The leader proposes a joint configuration `C_old,new`.
//! 2. Once `C_old,new` is committed by majorities of BOTH `C_old` and `C_new`,
//!    the leader proposes the final configuration `C_new`.
//! 3. Once `C_new` is committed, nodes not in `C_new` are safely decommissioned.

use std::collections::{HashMap, HashSet};

/// Representation of active cluster membership configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterConfig {
    /// Standard single-configuration state.
    Simple {
        /// Active cluster member node IDs.
        members: Vec<u64>,
    },
    /// Joint consensus state active during dynamic configuration transitions.
    Joint {
        /// Prior cluster member node IDs.
        old_members: Vec<u64>,
        /// Target cluster member node IDs.
        new_members: Vec<u64>,
    },
}

impl ClusterConfig {
    /// Create a new simple configuration.
    pub fn simple(members: Vec<u64>) -> Self {
        let mut sorted = members;
        sorted.sort_unstable();
        sorted.dedup();
        Self::Simple { members: sorted }
    }

    /// Initiate a joint consensus transition to `new_members`.
    pub fn enter_joint(&self, new_members: Vec<u64>) -> Self {
        let old = match self {
            Self::Simple { members } => members.clone(),
            Self::Joint { new_members: n, .. } => n.clone(),
        };
        let mut new_sorted = new_members;
        new_sorted.sort_unstable();
        new_sorted.dedup();

        Self::Joint {
            old_members: old,
            new_members: new_sorted,
        }
    }

    /// Finalize joint consensus into the target `new_members` configuration.
    pub fn finalize_joint(&self) -> Option<Self> {
        match self {
            Self::Joint { new_members, .. } => Some(Self::Simple {
                members: new_members.clone(),
            }),
            Self::Simple { .. } => None,
        }
    }

    /// Total distinct nodes in the union of active configurations.
    pub fn total_members(&self) -> usize {
        match self {
            Self::Simple { members } => members.len(),
            Self::Joint {
                old_members,
                new_members,
            } => {
                let set: HashSet<u64> = old_members.iter().chain(new_members.iter()).copied().collect();
                set.len()
            }
        }
    }

    /// Check if a node ID is a member of the current configuration.
    pub fn contains(&self, id: u64) -> bool {
        match self {
            Self::Simple { members } => members.contains(&id),
            Self::Joint {
                old_members,
                new_members,
            } => old_members.contains(&id) || new_members.contains(&id),
        }
    }

    /// Evaluate if an index has achieved quorum commitment under this configuration.
    ///
    /// Under Joint Consensus: an entry commits only when a majority of `C_old` AND
    /// a majority of `C_new` hold the entry (Raft §6).
    pub fn is_committed(&self, match_indices: &HashMap<u64, u64>, target_index: u64) -> bool {
        match self {
            Self::Simple { members } => {
                let majority = members.len() / 2 + 1;
                let matches = members
                    .iter()
                    .filter(|&&id| match_indices.get(&id).copied().is_some_and(|m| m >= target_index))
                    .count();
                matches >= majority
            }
            Self::Joint {
                old_members,
                new_members,
            } => {
                let old_majority = old_members.len() / 2 + 1;
                let new_majority = new_members.len() / 2 + 1;

                let old_matches = old_members
                    .iter()
                    .filter(|&&id| match_indices.get(&id).copied().is_some_and(|m| m >= target_index))
                    .count();

                let new_matches = new_members
                    .iter()
                    .filter(|&&id| match_indices.get(&id).copied().is_some_and(|m| m >= target_index))
                    .count();

                old_matches >= old_majority && new_matches >= new_majority
            }
        }
    }

    /// Evaluate if a candidate has received enough votes to be elected leader.
    ///
    /// Under standard simple configuration: strict majority of members.
    /// Under Joint Consensus: a candidate must receive a majority of votes from `C_old`
    /// AND a majority of votes from `C_new` (Raft §6).
    pub fn is_elected(&self, voters: &HashSet<u64>) -> bool {
        match self {
            Self::Simple { members } => {
                let majority = members.len() / 2 + 1;
                let count = members.iter().filter(|id| voters.contains(id)).count();
                count >= majority
            }
            Self::Joint {
                old_members,
                new_members,
            } => {
                let old_majority = old_members.len() / 2 + 1;
                let new_majority = new_members.len() / 2 + 1;

                let old_count = old_members.iter().filter(|id| voters.contains(id)).count();
                let new_count = new_members.iter().filter(|id| voters.contains(id)).count();

                old_count >= old_majority && new_count >= new_majority
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_joint_consensus_transitions_and_commit_election() {
        // C_old: {1, 2, 3} (majority = 2)
        let config = ClusterConfig::simple(vec![1, 2, 3]);
        assert_eq!(config.total_members(), 3);

        // Transition to C_new: {2, 3, 4, 5} (majority = 3)
        let joint = config.enter_joint(vec![2, 3, 4, 5]);
        assert_eq!(joint.total_members(), 5);

        // Check commitment under joint consensus
        let mut match_indices = HashMap::new();
        match_indices.insert(1, 100);
        match_indices.insert(2, 100);
        // Only 2 nodes committed: in C_old {1, 2} >= 2 (ok), but in C_new only {2} < 3 (not committed!)
        assert!(!joint.is_committed(&match_indices, 100));

        // Add node 3
        match_indices.insert(3, 100);
        // In C_old {1, 2, 3} >= 2 (ok), in C_new {2, 3} = 2 < 3 (still not committed!)
        assert!(!joint.is_committed(&match_indices, 100));

        // Add node 4
        match_indices.insert(4, 100);
        // In C_new {2, 3, 4} = 3 >= 3 (committed under both C_old and C_new!)
        assert!(joint.is_committed(&match_indices, 100));

        // Check election under joint consensus
        let mut voters = HashSet::new();
        voters.insert(1);
        voters.insert(2);
        assert!(!joint.is_elected(&voters));
        voters.insert(3);
        assert!(!joint.is_elected(&voters));
        voters.insert(4);
        assert!(joint.is_elected(&voters));

        // Finalize joint configuration
        let finalized = joint.finalize_joint().unwrap();
        assert_eq!(finalized, ClusterConfig::simple(vec![2, 3, 4, 5]));
        assert!(!finalized.contains(1)); // node 1 decommissioned
        assert!(finalized.contains(5));
    }
}
