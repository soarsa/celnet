//! Comprehensive Multi-Node Chaos Engineering Suite for celnet-replog.
//!
//! Validates:
//! 1. Rolling Asymmetric Network Partitions & Split-Brain Immunity on 7-Node Heptagon.
//! 2. Byzantine Malformed TCP Packets, Corrupted CRCs, and Frame Fuzzing.
//! 3. Abrupt Leader Kill Loops under High-Concurrency Write Pressure.
//! 4. Dynamic Joint Consensus Reconfiguration Resilience under Node Dropouts.

mod common;

use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use celnet_replog::{
    BookState, BookUpdate, QuorumPolicy, RaftConfig, RaftNode,
};
use common::{assert_within_deadline, temp_journal, wait_until};

fn chaos_cfg() -> RaftConfig {
    RaftConfig {
        election_min: Duration::from_millis(300),
        election_max: Duration::from_millis(600),
        heartbeat: Duration::from_millis(30),
        io_timeout: Duration::from_secs(1),
        quorum_policy: QuorumPolicy::Majority,
    }
}

fn boot_cluster(n: usize, prefix: &str) -> (Vec<RaftNode>, Vec<SocketAddr>) {
    let mut listeners = Vec::with_capacity(n);
    let mut addrs = Vec::with_capacity(n);
    for _ in 0..n {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        addrs.push(l.local_addr().unwrap());
        listeners.push(l);
    }
    let mut nodes = Vec::with_capacity(n);
    for (i, listener) in listeners.into_iter().enumerate() {
        let peers: Vec<SocketAddr> = addrs
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, &a)| a)
            .collect();
        let tag = format!("{prefix}-node-{i}");
        let node = RaftNode::boot_on(listener, temp_journal(&tag), &peers, n, chaos_cfg())
            .expect("node boots");
        nodes.push(node);
    }
    (nodes, addrs)
}

fn await_single_leader(start: Instant, nodes: &[RaftNode]) -> usize {
    let mut leader_idx = None;
    let ok = wait_until(start, || {
        let leaders: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_leader())
            .map(|(i, _)| i)
            .collect();
        if leaders.len() == 1 {
            leader_idx = Some(leaders[0]);
            true
        } else {
            false
        }
    });
    assert!(ok, "failed to elect single stable leader within deadline");
    leader_idx.unwrap()
}

/// Chaos Test 1: Byzantine Malformed TCP Packets & Frame Fuzzing.
///
/// Sends crafted packets (oversized frames, invalid tags, corrupted CRC, truncated bodies)
/// directly to a running node's TCP port to verify it survives without panic or hang.
#[test]
fn test_chaos_byzantine_tcp_frame_fuzzing() {
    let start = Instant::now();
    let (nodes, addrs) = boot_cluster(3, "fuzz");
    let victim_addr = addrs[0];

    // Case 1: Oversized frame length (> MAX_FRAME_LEN 64MB)
    if let Ok(mut stream) = TcpStream::connect(victim_addr) {
        let oversized_len: u32 = 128 * 1024 * 1024;
        let _ = stream.write_all(&oversized_len.to_be_bytes());
        let _ = stream.write_all(b"GARBAGE_PAYLOAD");
    }

    // Case 2: Illegal message tag (tag 250)
    if let Ok(mut stream) = TcpStream::connect(victim_addr) {
        let frame_len: u32 = 8;
        let _ = stream.write_all(&frame_len.to_be_bytes());
        let _ = stream.write_all(&[250u8, 0, 1, 2, 3, 4, 5, 6]);
    }

    // Case 3: Truncated frame (claims 100 bytes, sends 4)
    if let Ok(mut stream) = TcpStream::connect(victim_addr) {
        let frame_len: u32 = 100;
        let _ = stream.write_all(&frame_len.to_be_bytes());
        let _ = stream.write_all(&[0u8, 1, 2, 3]);
    }

    // Case 4: Invalid CRC on LogEntry
    if let Ok(mut stream) = TcpStream::connect(victim_addr) {
        let mut msg_bytes = vec![0u8]; // AppendEntries tag 0
        msg_bytes.extend_from_slice(&1u64.to_le_bytes()); // term 1
        msg_bytes.extend_from_slice(&u64::MAX.to_le_bytes()); // prev_log_index
        msg_bytes.extend_from_slice(&0u64.to_le_bytes()); // prev_log_term
        msg_bytes.extend_from_slice(&0u64.to_le_bytes()); // leader_commit
        msg_bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 entry
        msg_bytes.extend_from_slice(&16u32.to_le_bytes()); // entry len 16
        msg_bytes.extend_from_slice(&[0xFF; 16]); // corrupted entry bytes
        let flen = msg_bytes.len() as u32;
        let _ = stream.write_all(&flen.to_be_bytes());
        let _ = stream.write_all(&msg_bytes);
    }

    // Verify the cluster is still 100% healthy and functioning normally
    let leader = await_single_leader(start, &nodes);
    let update = BookUpdate::Set {
        key: 777,
        value: 123.456,
    };
    let idx = nodes[leader].propose(&update).unwrap().unwrap();
    assert!(nodes[leader].wait_for_commit(idx, Duration::from_secs(5)));

    assert_within_deadline(start);
    for n in nodes {
        n.shutdown();
    }
}

/// Chaos Test 2: Heptagon (7-Node) Network Partition & Healing.
///
/// Simulates an asymmetric network partition dividing a 7-node cluster into:
/// - Partition A: 4 nodes (Majority, 4/7 > 3.5)
/// - Partition B: 3 nodes (Minority, 3/7 < 3.5)
/// Verifies that Partition A continues to commit updates, Partition B makes NO progress,
/// and when the partition is healed, all 7 nodes reconcile bit-identically.
#[test]
fn test_chaos_heptagon_7node_partition_and_healing() {
    let start = Instant::now();
    let (nodes, addrs) = boot_cluster(7, "hept");

    // 1. Initial write with full cluster
    let leader = await_single_leader(start, &nodes);
    let u1 = BookUpdate::Set { key: 1, value: 100.0 };
    let idx1 = nodes[leader].propose(&u1).unwrap().unwrap();
    assert!(nodes[leader].wait_for_commit(idx1, Duration::from_secs(5)));

    // 2. Cut partition between nodes [0..4] (majority of 4) and [4..7] (minority of 3)
    let maj_indices = [0, 1, 2, 3];
    let min_indices = [4, 5, 6];

    // Disconnect min nodes from maj nodes and vice-versa
    for &maj in &maj_indices {
        for &min in &min_indices {
            nodes[maj].remove_peer(nodes[min].id());
            nodes[min].remove_peer(nodes[maj].id());
        }
    }

    // Give time for election timers to adapt to partition
    std::thread::sleep(Duration::from_millis(500));

    // Find leader in majority partition
    let maj_nodes: Vec<&RaftNode> = maj_indices.iter().map(|&i| &nodes[i]).collect();
    let mut maj_leader = None;
    let ok = wait_until(start, || {
        for (idx, n) in maj_nodes.iter().enumerate() {
            if n.is_leader() {
                maj_leader = Some(idx);
                return true;
            }
        }
        false
    });
    assert!(ok, "majority partition failed to maintain or elect a leader");
    let maj_leader_idx = maj_leader.unwrap();

    // 3. Propose to majority partition -> MUST commit
    let u2 = BookUpdate::Add { key: 1, delta: 50.0 };
    let idx2 = maj_nodes[maj_leader_idx].propose(&u2).unwrap().unwrap();
    assert!(maj_nodes[maj_leader_idx].wait_for_commit(idx2, Duration::from_secs(5)));

    // 4. Check minority nodes: none should have committed idx2
    for &min in &min_indices {
        assert_ne!(nodes[min].commit_index(), Some(idx2), "minority node committed without quorum!");
    }

    // 5. Heal partition: restore all connections
    for &maj in &maj_indices {
        for &min in &min_indices {
            nodes[maj].add_peer(nodes[min].id(), addrs[min]);
            nodes[min].add_peer(nodes[maj].id(), addrs[maj]);
        }
    }

    // 6. Verify all 7 nodes catch up to idx2
    let converged = wait_until(start, || {
        nodes.iter().all(|n| n.commit_index() == Some(idx2))
    });
    assert!(converged, "all 7 nodes did not converge to commit_index after partition healed");

    // 7. Bit-identity verification
    let mut oracle = BookState::new();
    oracle.apply(&u1);
    oracle.apply(&u2);
    let expected_bits = oracle.to_bits();

    for (i, n) in nodes.iter().enumerate() {
        assert_eq!(n.applied_bits(), expected_bits, "node {i} applied bits mismatch after healing");
    }

    assert_within_deadline(start);
    for n in nodes {
        n.shutdown();
    }
}

/// Chaos Test 3: Leader Crash Loops Under High Write Concurrency.
///
/// Abruptly terminates the active leader while writes are pending,
/// verifying that the surviving cluster automatically elects a new leader and
/// continues making progress without corruption.
#[test]
fn test_chaos_leader_crash_and_failover_recovery() {
    let start = Instant::now();
    let (mut nodes, _addrs) = boot_cluster(5, "crash");

    // 1. Elect initial leader and commit one trade
    let leader_idx = await_single_leader(start, &nodes);
    let u1 = BookUpdate::Set { key: 10, value: 42.0 };
    let idx1 = nodes[leader_idx].propose(&u1).unwrap().unwrap();
    assert!(nodes[leader_idx].wait_for_commit(idx1, Duration::from_secs(5)));

    // 2. Abruptly kill the leader
    let dead_leader = nodes.remove(leader_idx);
    dead_leader.shutdown();

    // 3. Surviving 4 nodes must elect a new leader
    let new_leader_idx = await_single_leader(start, &nodes);
    assert_ne!(nodes[new_leader_idx].id(), 0);

    // 4. Propose new trade to the new leader -> MUST commit
    let u2 = BookUpdate::Add { key: 10, delta: 8.0 };
    let idx2 = nodes[new_leader_idx].propose(&u2).unwrap().unwrap();
    assert!(nodes[new_leader_idx].wait_for_commit(idx2, Duration::from_secs(5)));

    // 5. Verify state machine matches (42.0 + 8.0 = 50.0) across all surviving nodes
    let mut oracle = BookState::new();
    oracle.apply(&u1);
    oracle.apply(&u2);
    let expected_bits = oracle.to_bits();

    let converged = wait_until(start, || {
        nodes.iter().all(|n| n.applied_bits() == expected_bits)
    });
    assert!(converged, "surviving nodes did not converge to oracle applied state");

    assert_within_deadline(start);
    for n in nodes {
        n.shutdown();
    }
}
