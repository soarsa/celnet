//! High-Performance Consensus, Asymmetric Quorums, Joint Consensus, and Multi-Raft Tests
//!
//! Validates atomic group membership transitions, quorum intersection invariants, and partition resilience.

use std::collections::HashMap;

use celnet_replog::{
    BookUpdate, ClusterConfig, MultiRaftRouter, QuorumPolicy, RaftConfig,
};

#[test]
fn test_flexible_paxos_quorum_configuration() {
    let cfg = RaftConfig {
        quorum_policy: QuorumPolicy::Flexible {
            fast_commit_quorum: 2,
            election_quorum: 4,
        },
        ..Default::default()
    };

    assert_eq!(
        cfg.quorum_policy,
        QuorumPolicy::Flexible {
            fast_commit_quorum: 2,
            election_quorum: 4,
        }
    );

    // Verify mathematical safety invariant: Q_elect + Q_commit > N
    if let QuorumPolicy::Flexible {
        fast_commit_quorum,
        election_quorum,
    } = cfg.quorum_policy
    {
        let cluster_size = 5;
        assert!(
            fast_commit_quorum + election_quorum > cluster_size,
            "Flexible Paxos intersection invariant violated"
        );
    }
}

#[test]
fn test_raft_joint_consensus_transitions() {
    let c_old = ClusterConfig::simple(vec![1, 2, 3]);
    assert_eq!(c_old.total_members(), 3);
    assert!(c_old.contains(1));
    assert!(!c_old.contains(4));

    // Simple majority check: 2 out of 3 needed
    let mut matches = HashMap::new();
    matches.insert(1, 10);
    matches.insert(2, 5); // below target 10
    matches.insert(3, 5); // below target 10
    assert!(!c_old.is_committed(&matches, 10));

    matches.insert(2, 10); // now 2 nodes have matched target 10
    assert!(c_old.is_committed(&matches, 10));

    // Transition to Joint Consensus C_old,new = [1, 2, 3] and [2, 3, 4, 5]
    let c_joint = c_old.enter_joint(vec![2, 3, 4, 5]);
    assert_eq!(c_joint.total_members(), 5);

    // During Joint Consensus, BOTH majorities must hold:
    // C_old ([1, 2, 3]) requires 2 nodes
    // C_new ([2, 3, 4, 5]) requires 3 nodes
    matches.clear();
    matches.insert(1, 10);
    matches.insert(2, 10);
    // C_old has 2 nodes (majority), but C_new only has 1 node (2) -> not committed!
    assert!(!c_joint.is_committed(&matches, 10));

    // Add nodes 3 and 4
    matches.insert(3, 10);
    matches.insert(4, 10);
    // C_old has 1, 2, 3 (3/3 >= 2). C_new has 2, 3, 4 (3/4 >= 3). -> committed!
    assert!(c_joint.is_committed(&matches, 10));

    // Finalize to C_new
    let c_new = c_joint.finalize_joint().unwrap();
    assert_eq!(c_new.total_members(), 4);
    assert!(!c_new.contains(1)); // node 1 safely decommissioned!
    assert!(c_new.contains(4));
    assert!(c_new.contains(5));
}

#[test]
fn test_multi_raft_sharded_partition_routing() {
    let router = MultiRaftRouter::new();

    router.register_group("EURUSD");
    router.register_group("USDJPY");
    router.register_group("BTCUSD");

    assert!(router.has_group("EURUSD"));
    assert!(router.has_group("USDJPY"));
    assert!(router.has_group("BTCUSD"));
    assert!(!router.has_group("ETHUSD"));

    // Apply updates to EURUSD
    router
        .apply(
            "EURUSD",
            1,
            1,
            &BookUpdate::Set {
                key: 101,
                value: 1.0850,
            },
        )
        .unwrap();

    // Apply updates to USDJPY
    router
        .apply(
            "USDJPY",
            1,
            1,
            &BookUpdate::Set {
                key: 201,
                value: 155.25,
            },
        )
        .unwrap();

    // Verify isolation: EURUSD state has 101, USDJPY state has 201
    assert_eq!(router.get_price("EURUSD", 101).unwrap(), Some(1.0850));
    assert_eq!(router.get_price("EURUSD", 201).unwrap(), None);

    assert_eq!(router.get_price("USDJPY", 201).unwrap(), Some(155.25));
    assert_eq!(router.get_price("USDJPY", 101).unwrap(), None);

    // Apply additive delta to EURUSD
    router
        .apply(
            "EURUSD",
            2,
            1,
            &BookUpdate::Add {
                key: 101,
                delta: 0.0005,
            },
        )
        .unwrap();

    let updated = router.get_price("EURUSD", 101).unwrap().unwrap();
    assert!((updated - 1.0855).abs() < 1e-12);
}

#[test]
fn test_raft_node_dynamic_joint_consensus_workflow() {
    let dir = std::env::temp_dir().join(format!(
        "celnet_joint_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let journal_path = dir.join("node.journal");

    let cfg = RaftConfig::default();
    let node = celnet_replog::RaftNode::boot(&journal_path, &[], 1, cfg).unwrap();

    // Propose update to ensure leadership
    let update = BookUpdate::Set {
        key: 42,
        value: std::f64::consts::PI,
    };
    let _ = node.propose(&update);

    // Initial config is simple
    let config = node.cluster_config();
    assert_eq!(config.total_members(), 1);

    if node.is_leader() {
        let res = node.enter_joint_consensus(vec![node.id(), 9999]);
        assert!(res.is_ok());
        assert_eq!(node.cluster_config().total_members(), 2);

        let final_res = node.finalize_joint_consensus();
        assert!(final_res.is_ok());
        assert_eq!(node.cluster_config().total_members(), 2);
    }
    node.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
