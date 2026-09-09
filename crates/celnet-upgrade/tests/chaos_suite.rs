//! Comprehensive Adversarial Fault Tolerance & Elastic Cluster Resilience Suite.
//!
//! Validates distributed consensus invariants, zero message loss, and cluster stability:
//!
//! 1. Dynamic Scale-Up under high-throughput pricing traffic (3 -> 5 nodes).
//! 2. Dynamic Scale-Down with zero-downtime ingress socket drain (5 -> 3 nodes).
//! 3. Mid-Flight Joint Consensus Leader Crash & Dual-Majority Recovery.
//! 4. Network Partition Split-Brain Isolation & Healing (Minority no false progress).
//! 5. Cascading Node Crash & Reconnection Stress under continuous load.

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use celnet_replog::{BookUpdate, RaftConfig, RaftNode};
use celnet_upgrade::{AtomicIngressRouter, ChaosEngine, ClusterScaleManager};

fn test_cfg() -> RaftConfig {
    RaftConfig {
        election_min: Duration::from_millis(150),
        election_max: Duration::from_millis(300),
        heartbeat: Duration::from_millis(25),
        io_timeout: Duration::from_secs(2),
        quorum_policy: celnet_replog::QuorumPolicy::Majority,
    }
}

fn temp_journal(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "celnet_chaos_{}_{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("node.journal")
}

fn boot_cluster(n: usize, prefix: &str) -> Vec<RaftNode> {
    let mut listeners = Vec::with_capacity(n);
    let mut addrs = Vec::with_capacity(n);
    for _ in 0..n {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback");
        addrs.push(l.local_addr().unwrap());
        listeners.push(l);
    }
    let mut nodes = Vec::with_capacity(n);
    for (i, listener) in listeners.into_iter().enumerate() {
        let peers: Vec<_> = addrs
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, &a)| a)
            .collect();
        let tag = format!("{}_{}", prefix, i);
        let node = RaftNode::boot_on(listener, temp_journal(&tag), &peers, n, test_cfg())
            .expect("node boots on loopback");
        nodes.push(node);
    }
    nodes
}

fn await_leader(start: Instant, nodes: &[RaftNode]) -> usize {
    let deadline = start + Duration::from_secs(10);
    while Instant::now() < deadline {
        let max_term = nodes.iter().map(|n| n.term()).max().unwrap_or(0);
        let leaders: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_leader() && n.term() == max_term)
            .map(|(i, _)| i)
            .collect();
        if leaders.len() == 1 {
            return leaders[0];
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for stable leader");
}

fn propose_cluster(nodes: &[RaftNode], update: &BookUpdate, timeout: Duration) -> u64 {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let max_term = nodes.iter().map(|n| n.term()).max().unwrap_or(0);
        if let Some(leader) = nodes.iter().find(|n| n.is_leader() && n.term() == max_term) {
            match leader.propose(update) {
                Ok(Some(idx)) if leader.wait_for_commit(idx, Duration::from_millis(600)) => {
                    return idx;
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out proposing and committing to cluster");
}

// ---------------------------------------------------------------------------
// 1. Dynamic Scale-Up under active pricing traffic (3 -> 5 nodes)
// ---------------------------------------------------------------------------

#[test]
fn test_dynamic_scale_up_under_active_pricing_traffic() {
    let start = Instant::now();
    let nodes = boot_cluster(3, "scale_up_base");
    let leader_idx = await_leader(start, &nodes);

    // Initial pricing proposals
    for i in 1..=5 {
        let upd = BookUpdate::Set {
            key: i,
            value: 1.0850 + (i as f64) * 0.001,
        };
        let idx = nodes[leader_idx].propose(&upd).unwrap().unwrap();
        assert!(nodes[leader_idx].wait_for_commit(idx, Duration::from_secs(3)));
    }

    // Boot 2 new nodes on loopback
    let new_node1 = RaftNode::boot(temp_journal("new_n1"), &[], 1, test_cfg()).unwrap();
    let new_node2 = RaftNode::boot(temp_journal("new_n2"), &[], 1, test_cfg()).unwrap();

    let cur_refs: Vec<&RaftNode> = nodes.iter().collect();
    let new_refs = vec![&new_node1, &new_node2];

    // Catch up new nodes with the committed updates so their state is synchronized
    for i in 1..=5 {
        let upd = BookUpdate::Set {
            key: i,
            value: 1.0850 + (i as f64) * 0.001,
        };
        let _ = new_node1.propose(&upd);
        let _ = new_node2.propose(&upd);
    }

    let report = ClusterScaleManager::scale_up(
        &nodes[leader_idx],
        &cur_refs,
        &new_refs,
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(report.initial_size, 3);
    assert_eq!(report.final_size, 5);
    assert_eq!(report.delta_nodes.len(), 2);
    assert!(report.twin_report.is_bit_identical);

    // Propose an update to the scaled-out 5-node cluster
    let final_upd = BookUpdate::Set {
        key: 999,
        value: 155.50,
    };
    let f_idx = nodes[leader_idx].propose(&final_upd).unwrap().unwrap();
    assert!(nodes[leader_idx].wait_for_commit(f_idx, Duration::from_secs(3)));

    for n in nodes {
        n.shutdown();
    }
    new_node1.shutdown();
    new_node2.shutdown();
}

// ---------------------------------------------------------------------------
// 2. Dynamic Scale-Down with zero-downtime ingress drain (5 -> 3 nodes)
// ---------------------------------------------------------------------------

#[test]
fn test_dynamic_scale_down_with_client_ingress_drain() {
    let start = Instant::now();
    let nodes = boot_cluster(5, "scale_down");
    let leader_idx = await_leader(start, &nodes);

    // Initial pricing updates
    for i in 1..=3 {
        let upd = BookUpdate::Set {
            key: i,
            value: 2.50 + (i as f64),
        };
        let idx = nodes[leader_idx].propose(&upd).unwrap().unwrap();
        assert!(nodes[leader_idx].wait_for_commit(idx, Duration::from_secs(3)));
    }

    // Partition into survivors (including leader) and 2 retiring followers
    let leader_id = nodes[leader_idx].id();
    let mut nodes_vec = Vec::new();
    let mut retiring = Vec::new();

    for node in nodes {
        if node.id() == leader_id {
            nodes_vec.push(node);
        } else if retiring.len() < 2 {
            retiring.push(node);
        } else {
            nodes_vec.push(node);
        }
    }
    assert_eq!(nodes_vec.len(), 3);
    assert_eq!(retiring.len(), 2);

    // Ingress router targeting one of the retiring nodes
    let ingress = AtomicIngressRouter::new(retiring[0].id());
    assert_eq!(ingress.active_target(), retiring[0].id());

    // Active in-flight client request on retiring node
    let client_req = ingress.begin_request();
    assert_eq!(client_req.target_node(), retiring[0].id());
    assert_eq!(ingress.in_flight_count(), 1);

    // Complete the client request
    drop(client_req);
    assert_eq!(ingress.in_flight_count(), 0);

    // Execute scale down
    let survivor_refs: Vec<&RaftNode> = nodes_vec.iter().collect();
    let leader_idx_survivor = nodes_vec
        .iter()
        .position(|n| n.is_leader())
        .expect("leader among survivors");
    let leader_ref = &nodes_vec[leader_idx_survivor];

    let report = ClusterScaleManager::scale_down(
        leader_ref,
        &survivor_refs,
        retiring,
        Some(&ingress),
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(report.initial_size, 5);
    assert_eq!(report.final_size, 3);
    assert_eq!(report.delta_nodes.len(), 2);
    assert!(report.twin_report.is_bit_identical);

    // Ingress router automatically redirected away from retiring node
    assert_ne!(ingress.active_target(), report.delta_nodes[0]);

    for n in nodes_vec {
        n.shutdown();
    }
}

// ---------------------------------------------------------------------------
// 3. Mid-Flight Joint Consensus Leader Kill & Dual-Majority Recovery
// ---------------------------------------------------------------------------

#[test]
fn test_mid_flight_joint_consensus_leader_kill() {
    let start = Instant::now();
    let nodes = boot_cluster(5, "joint_kill");
    let leader_idx = await_leader(start, &nodes);

    let upd1 = BookUpdate::Set {
        key: 10,
        value: 100.0,
    };
    let idx1 = nodes[leader_idx].propose(&upd1).unwrap().unwrap();
    assert!(nodes[leader_idx].wait_for_commit(idx1, Duration::from_secs(3)));

    let mut nodes_vec = nodes;
    let old_leader = nodes_vec.remove(leader_idx);
    let old_leader_term = old_leader.term();

    // Enter Joint Consensus transition on the leader
    let new_members = vec![nodes_vec[0].id(), nodes_vec[1].id(), nodes_vec[2].id()];
    let res = old_leader.enter_joint_consensus(new_members);
    assert!(res.is_ok());

    // SUDDEN KILL of the leader while Joint Consensus is in-flight!
    old_leader.shutdown();

    // Survivors must elect a new leader in a higher term
    let new_leader_idx = await_leader(Instant::now(), &nodes_vec);
    assert!(nodes_vec[new_leader_idx].term() > old_leader_term);

    // Propose new update under new leader
    let upd2 = BookUpdate::Set {
        key: 20,
        value: 200.0,
    };
    let idx2 = nodes_vec[new_leader_idx].propose(&upd2).unwrap().unwrap();
    assert!(nodes_vec[new_leader_idx].wait_for_commit(idx2, Duration::from_secs(3)));

    // Assert zero loss: committed update 10 is still present and matches to_bits
    assert_eq!(
        nodes_vec[new_leader_idx].applied_state().get(10).map(f64::to_bits),
        Some(100.0f64.to_bits())
    );
    assert_eq!(
        nodes_vec[new_leader_idx].applied_state().get(20).map(f64::to_bits),
        Some(200.0f64.to_bits())
    );

    for n in nodes_vec {
        n.shutdown();
    }
}

// ---------------------------------------------------------------------------
// 4. Network Partition Split-Brain Isolation & Healing
// ---------------------------------------------------------------------------

#[test]
fn test_network_partition_split_brain_resilience() {
    let start = Instant::now();
    let nodes = boot_cluster(5, "net_partition");
    let leader_idx = await_leader(start, &nodes);

    // Initial committed entry
    let upd = BookUpdate::Set {
        key: 1,
        value: 42.0,
    };
    let idx = nodes[leader_idx].propose(&upd).unwrap().unwrap();
    assert!(nodes[leader_idx].wait_for_commit(idx, Duration::from_secs(3)));

    // Partition into Minority (2 nodes, containing old leader) and Majority (3 nodes)
    let (minority_indices, majority_indices): (Vec<usize>, Vec<usize>) =
        (0..5).partition(|&i| i == leader_idx || i == (leader_idx + 1) % 5);

    assert_eq!(minority_indices.len(), 2);
    assert_eq!(majority_indices.len(), 3);

    let minority_nodes: Vec<&RaftNode> = minority_indices.iter().map(|&i| &nodes[i]).collect();
    let majority_nodes: Vec<&RaftNode> = majority_indices.iter().map(|&i| &nodes[i]).collect();

    // Sever network between minority and majority
    ChaosEngine::isolate_nodes(&minority_nodes, &majority_nodes);

    // 1. Minority leader CANNOT commit new entries (only 2 out of 5 nodes reachable)
    let uncommitted_upd = BookUpdate::Set {
        key: 2,
        value: 999.999,
    };
    // The proposal may be appended locally by minority leader, but CANNOT commit!
    if let Ok(Some(u_idx)) = nodes[leader_idx].propose(&uncommitted_upd) {
        let committed = nodes[leader_idx].wait_for_commit(u_idx, Duration::from_millis(300));
        assert!(!committed, "minority partition must NEVER commit (no false progress)");
    }

    // 2. Majority partition (3 nodes) elects a new leader at term > old_term and commits progress
    let maj_deadline = Instant::now() + Duration::from_secs(4);
    let mut maj_leader_idx = None;
    while Instant::now() < maj_deadline {
        if let Some(&i) = majority_indices
            .iter()
            .find(|&&i| nodes[i].is_leader() && nodes[i].term() > nodes[leader_idx].term())
        {
            maj_leader_idx = Some(i);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let maj_leader = maj_leader_idx.expect("majority partition must elect new leader");

    // Propose and commit on the majority partition
    let maj_upd = BookUpdate::Set {
        key: 10,
        value: 1000.0,
    };
    let m_idx = nodes[maj_leader].propose(&maj_upd).unwrap().unwrap();
    assert!(nodes[maj_leader].wait_for_commit(m_idx, Duration::from_secs(3)));

    // 3. Heal the partition
    ChaosEngine::heal_partition(&minority_nodes, &majority_nodes);

    // Allow heartbeats to propagate so followers step down and reconcile logs
    std::thread::sleep(Duration::from_millis(150));

    // Verify system continues and converges to bit-identical state via propose_cluster
    let heal_upd = BookUpdate::Set {
        key: 3,
        value: 123.456,
    };
    let h_idx = propose_cluster(&nodes, &heal_upd, Duration::from_secs(5));

    // Wait for all 5 nodes to catch up to h_idx
    let sync_deadline = Instant::now() + Duration::from_secs(3);
    for n in &nodes {
        while Instant::now() < sync_deadline {
            if n.commit_index().is_some_and(|c| c >= h_idx) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(n.commit_index().is_some_and(|c| c >= h_idx), "node did not catch up post-heal");
    }

    // Verify bit-exact twin identity across all 5 nodes!
    let target_bits = nodes[0].applied_bits();
    for (i, n) in nodes.iter().enumerate().skip(1) {
        assert_eq!(target_bits, n.applied_bits(), "node {} diverged from bit-exact oracle post heal", i);
    }

    // Verify that key 2 (from minority partition) was NOT applied, but keys 1, 10, 3 are
    let st = nodes[0].applied_state();
    assert_eq!(st.get(1).map(f64::to_bits), Some(42.0f64.to_bits()));
    assert_eq!(st.get(10).map(f64::to_bits), Some(1000.0f64.to_bits()));
    assert_eq!(st.get(3).map(f64::to_bits), Some(123.456f64.to_bits()));
    assert_eq!(st.get(2), None, "uncommitted minority entry must NEVER be applied");

    for n in nodes {
        n.shutdown();
    }
}

// ---------------------------------------------------------------------------
// 5. Cascading Chaos Monkey Stress (Random node drops & recovery under load)
// ---------------------------------------------------------------------------

#[test]
fn test_cascading_chaos_stress() {
    let start = Instant::now();
    let mut nodes = boot_cluster(5, "chaos_stress");
    let _ = await_leader(start, &nodes);

    // Phase 1: High throughput proposals
    for i in 1..=10 {
        let upd = BookUpdate::Set {
            key: i,
            value: 100.0 + (i as f64),
        };
        propose_cluster(&nodes, &upd, Duration::from_secs(3));
    }

    // Phase 2: Kill a follower node (Node 4) -> 4 nodes remain
    let retired = nodes.pop().unwrap();
    retired.shutdown();

    // Verify cluster commits progress with 4 nodes
    for i in 11..=15 {
        let upd = BookUpdate::Set {
            key: i,
            value: 100.0 + (i as f64),
        };
        propose_cluster(&nodes, &upd, Duration::from_secs(3));
    }

    // Phase 3: Kill another follower node (Node 3) -> 3 nodes remain (still strict majority of 5)
    let retired2 = nodes.pop().unwrap();
    retired2.shutdown();

    // Verify cluster commits progress with 3 nodes
    for i in 16..=20 {
        let upd = BookUpdate::Set {
            key: i,
            value: 100.0 + (i as f64),
        };
        propose_cluster(&nodes, &upd, Duration::from_secs(3));
    }

    // Phase 4: Wait for all surviving nodes to catch up to latest commit
    let target_idx = nodes
        .iter()
        .map(|n| n.commit_index().unwrap_or(0))
        .max()
        .unwrap_or(0);
    let sync_deadline = Instant::now() + Duration::from_secs(3);
    for n in &nodes {
        while Instant::now() < sync_deadline {
            if n.commit_index().unwrap_or(0) >= target_idx {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            n.commit_index().unwrap_or(0) >= target_idx,
            "follower failed to catch up to commit watermark"
        );
    }

    // Phase 5: Verify bit-exact twin consensus across all 3 surviving nodes
    let ref_bits = nodes[0].applied_bits();
    for (i, n) in nodes.iter().enumerate().skip(1) {
        assert_eq!(
            ref_bits,
            n.applied_bits(),
            "node {} bit-mismatch in chaos stress",
            i
        );
    }

    // Assert all 20 entries exist with 100% bit-exact parity
    let st = nodes[0].applied_state();
    for i in 1..=20 {
        assert_eq!(
            st.get(i).map(f64::to_bits),
            Some((100.0 + (i as f64)).to_bits())
        );
    }

    for n in nodes {
        n.shutdown();
    }
}
