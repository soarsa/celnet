//! End-to-end integration tests for Zero-Loss Upgrade Protocol (ZLUP).
//!
//! Validates the full 5-stage rolling upgrade workflow:
//! 1. Pre-flight Shadow Boot
//! 2. Catch-up & Bit-Exact Twin Verification (IEEE-754 bit-identity oracle)
//! 3. Raft §6 Dynamic Joint Consensus (C_old,new promotion)
//! 4. Atomic Ingress Socket Redirection (0 TCP drops)
//! 5. Clean Drain & Decommission

use std::path::PathBuf;
use std::time::Duration;

use celnet_replog::{BookUpdate, RaftConfig, RaftNode};
use celnet_upgrade::{
    BitExactTwinComparator, UpgradeError, UpgradeOrchestrator, UpgradeStage,
};

fn temp_journal(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "celnet_upg_{}_{}",
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("node.journal")
}

#[test]
fn test_twin_comparator_detects_one_ulp_divergence() {
    let active_bits = vec![
        (101, 1.0850f64.to_bits()),
        (102, 155.25f64.to_bits()),
        (103, 0.8540f64.to_bits()),
    ];

    // Perfect twin
    let twin_bits = active_bits.clone();
    let report = BitExactTwinComparator::verify_bit_identity(&active_bits, &twin_bits).unwrap();
    assert!(report.is_bit_identical);
    assert_eq!(report.total_keys_verified, 3);

    // Corrupted twin: 1 ULP on key 102
    let mut bad_twin = active_bits.clone();
    bad_twin[1].1 += 1;

    let err = BitExactTwinComparator::verify_bit_identity(&active_bits, &bad_twin).unwrap_err();
    match err {
        UpgradeError::TwinDivergence {
            instrument_key,
            active_bits: a,
            shadow_bits: s,
        } => {
            assert_eq!(instrument_key, 102);
            assert_eq!(a, 155.25f64.to_bits());
            assert_eq!(s, 155.25f64.to_bits() + 1);
        }
        other => panic!("expected TwinDivergence, got {:?}", other),
    }
}

fn wait_for_leader(node: &RaftNode) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline && !node.is_leader() {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(node.is_leader(), "node must elect itself leader");
}

#[test]
fn test_five_stage_zero_loss_upgrade_lifecycle() {
    // 1. Boot active node V_n
    let active_path = temp_journal("active_vn");
    let cfg = RaftConfig::default();
    let active_node = RaftNode::boot(&active_path, &[], 1, cfg).unwrap();
    wait_for_leader(&active_node);

    // 2. Propose pricing updates to active node
    let update1 = BookUpdate::Set {
        key: 101,
        value: 1.0850,
    };
    let update2 = BookUpdate::Set {
        key: 102,
        value: 155.25,
    };
    let update3 = BookUpdate::Add {
        key: 101,
        delta: 0.0005,
    };

    let _idx1 = active_node.propose(&update1).unwrap().unwrap();
    let _idx2 = active_node.propose(&update2).unwrap().unwrap();
    let idx3 = active_node.propose(&update3).unwrap().unwrap();

    assert!(active_node.wait_for_commit(idx3, Duration::from_secs(1)));

    let active_bits = active_node.applied_bits();
    assert_eq!(active_bits.len(), 2); // keys 101 and 102

    // 3. Boot shadow twin node V_n+1 on its own durable journal
    let shadow_path = temp_journal("shadow_vnp1");
    let shadow_node = RaftNode::boot(&shadow_path, &[], 1, cfg).unwrap();
    wait_for_leader(&shadow_node);

    // Replicate / catch up the same deterministic updates to shadow node
    let _ = shadow_node.propose(&update1);
    let _ = shadow_node.propose(&update2);
    let s_idx3 = shadow_node.propose(&update3).unwrap().unwrap();
    assert!(shadow_node.wait_for_commit(s_idx3, Duration::from_secs(1)));

    // 4. Initialize Upgrade Orchestrator targeting active node
    let mut orchestrator = UpgradeOrchestrator::new(active_node.id());
    assert_eq!(orchestrator.stage(), UpgradeStage::Idle);
    assert_eq!(orchestrator.ingress_router().active_target(), active_node.id());

    // Send a client request through the ingress router before upgrade and complete it
    let in_flight_client_req = orchestrator.ingress_router().begin_request();
    assert_eq!(in_flight_client_req.target_node(), active_node.id());
    drop(in_flight_client_req);

    // 5. Execute full 5-stage upgrade protocol
    let res = orchestrator.execute_upgrade(
        &active_node,
        &active_node,
        &shadow_node,
        Duration::from_secs(2),
    );

    // Now execute with completed drain
    let report = res.unwrap();
    assert!(report.is_bit_identical);
    assert_eq!(report.total_keys_verified, active_bits.len());

    // Verify final state
    assert_eq!(orchestrator.stage(), UpgradeStage::Decommissioned);
    assert_eq!(orchestrator.ingress_router().active_target(), shadow_node.id());

    // Subsequent client requests automatically route to shadow node (V_n+1)
    let new_client_req = orchestrator.ingress_router().begin_request();
    assert_eq!(new_client_req.target_node(), shadow_node.id());
    drop(new_client_req);

    // Teardown
    active_node.shutdown();
    shadow_node.shutdown();
}
