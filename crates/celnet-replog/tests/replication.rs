//! **Raft consensus — real loopback-socket multi-node correctness.**
//!
//! Every test boots ≥3 logical [`celnet_replog::RaftNode`]s, each backed by its
//! own durable journal, communicating over **real `127.0.0.1` TCP sockets on
//! ephemeral ports** (each node binds a listener and dials its peers). No
//! shared-memory fake stands in for the network. Each test body is hard
//! wall-clock bounded so a regression fails fast, never hangs.
//!
//! Gates proven here (the original a–d, evolved to auto-election):
//!
//! * (a) **commit → every node has the byte-identical committed log AND replays to
//!   bit-identical (`to_bits`) state** — the f64 oracle.
//! * (b) **quorum-commit correctness**: a partitioned MINORITY leader cannot
//!   advance the commit index (no false progress); a bare majority still commits.
//! * (c) **leader failover**: kill the leader → the survivors auto-elect a new
//!   leader within the bounded deadline and keep making progress (a fresh proposal
//!   commits), losing no committed entry.
//! * (d) **crash-recovery**: a node restarted from its journal recovers the exact
//!   committed state, bit-identically (reusing the journal torn-tail recovery).

mod common;

use std::time::{Duration, Instant};

use celnet_replog::{BookState, BookUpdate, LogEntry, RaftConfig, RaftNode, Role};

use common::{TEST_DEADLINE, assert_within_deadline, temp_journal, wait_until};

/// Loopback-tuned timing: election timeout ~10–20× the heartbeat with a wide
/// random spread, so an occasional fsync stall does not trip a spurious election,
/// while a dead leader is still detected within ~half a second.
fn test_cfg() -> RaftConfig {
    RaftConfig {
        election_min: Duration::from_millis(400),
        election_max: Duration::from_millis(800),
        heartbeat: Duration::from_millis(40),
        io_timeout: Duration::from_secs(2),
    }
}

/// A deterministic stream of book updates — `Set`s and `Add`s with values whose
/// exact bits we compare cross-node, including a `0.1 + 0.2` add and a 1-ULP value
/// so it is the *bits* (not a rounded decimal) that must replicate.
fn workload() -> Vec<BookUpdate> {
    vec![
        BookUpdate::Set {
            key: 1,
            value: 1.105_171_098_5,
        },
        BookUpdate::Set {
            key: 2,
            value: -0.876_543_210_987_654_3,
        },
        BookUpdate::Add { key: 1, delta: 0.1 },
        BookUpdate::Add { key: 1, delta: 0.2 },
        BookUpdate::Set {
            key: 3,
            value: 1.0 / 3.0,
        },
        BookUpdate::Add {
            key: 2,
            delta: 0.000_000_000_1,
        },
        BookUpdate::Remove { key: 3 },
        BookUpdate::Set {
            key: 3,
            value: f64::from_bits(0x3ff0_0000_0000_0001), // 1.0 + 1 ULP
        },
    ]
}

/// Build the oracle state by applying a committed prefix `[0, committed]` of the
/// workload on a single node — what every replica MUST match to_bits.
fn oracle_bits(updates: &[BookUpdate], committed: usize) -> Vec<(u64, u64)> {
    let mut s = BookState::new();
    for u in &updates[..committed] {
        s.apply(u);
    }
    s.to_bits()
}

/// Reconstruct a [`BookState`] purely from durable entry bytes — proving the
/// on-disk log alone determines the bit-identical state.
fn replay_log_to_state(entry_bytes: &[Vec<u8>]) -> BookState {
    let mut s = BookState::new();
    for bytes in entry_bytes {
        let entry = LogEntry::decode(bytes).expect("durable entry decodes");
        let upd = BookUpdate::decode(&entry.payload).expect("payload decodes");
        s.apply(&upd);
    }
    s
}

/// Boot an N-node fully-meshed cluster over real loopback sockets.
///
/// Fixed Raft membership means every node must know the complete peer-address set
/// at boot, but a node's ephemeral port is only assigned when it binds. We resolve
/// that by **binding every node's listener first** (so all addresses are known),
/// then handing each pre-bound listener into [`RaftNode::boot_on`] with the full
/// address plan. The listener's port becomes the node's stable id, so the peer
/// addresses match exactly what each node serves on — no port-reuse race.
fn boot_cluster(n: usize, tags: &[&str]) -> Vec<RaftNode> {
    use std::net::TcpListener;
    assert_eq!(n, tags.len());
    let mut listeners = Vec::with_capacity(n);
    let mut addrs = Vec::with_capacity(n);
    for _ in 0..n {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback");
        addrs.push(l.local_addr().unwrap());
        listeners.push(l);
    }
    let mut nodes = Vec::with_capacity(n);
    for (i, (tag, listener)) in tags.iter().zip(listeners).enumerate() {
        let peers: Vec<_> = addrs
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, &a)| a)
            .collect();
        let node = RaftNode::boot_on(listener, temp_journal(tag), &peers, n, test_cfg())
            .expect("node boots on loopback");
        nodes.push(node);
    }
    nodes
}

/// Wait until exactly one node reports itself leader, returning its index.
fn await_leader(start: Instant, nodes: &[RaftNode]) -> Option<usize> {
    let mut found = None;
    let ok = wait_until(start, || {
        let leaders: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_leader())
            .map(|(i, _)| i)
            .collect();
        if leaders.len() == 1 {
            found = Some(leaders[0]);
            true
        } else {
            false
        }
    });
    if ok { found } else { None }
}

/// Wait for a **stably-established** leader: a single node that is leader AND has
/// stayed leader continuously for a short stability window (so it is the only
/// leader and is actively heartbeating, keeping followers from timing out). This
/// avoids committing any priming entry — so the committed log is exactly the
/// workload, with no duplicate. Returns the stable leader index. Deadline-bounded.
fn establish_stable_leader(start: Instant, nodes: &[RaftNode]) -> usize {
    let stable_window = Duration::from_millis(250);
    loop {
        assert_within_deadline(start);
        let Some(leader) = await_leader(start, nodes) else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        // Confirm this same node stays the sole leader across the stability window.
        let term = nodes[leader].term();
        let window_start = Instant::now();
        let mut stable = true;
        while window_start.elapsed() < stable_window {
            assert_within_deadline(start);
            let leaders = nodes.iter().filter(|n| n.is_leader()).count();
            if leaders != 1 || !nodes[leader].is_leader() || nodes[leader].term() != term {
                stable = false;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if stable {
            return leader;
        }
    }
}

/// Drive a whole workload under a single stably-established leader, committing each
/// entry in order EXACTLY once (no re-proposing ⇒ the committed log is precisely the
/// workload). Returns `(leader_index, final_committed_index)`. Deadline-bounded.
fn drive_workload(start: Instant, nodes: &[RaftNode], updates: &[BookUpdate]) -> (usize, u64) {
    assert!(!updates.is_empty(), "workload must be non-empty");
    let leader = establish_stable_leader(start, nodes);
    let mut last_committed = 0u64;
    for u in updates {
        assert_within_deadline(start);
        let idx = nodes[leader]
            .propose(u)
            .expect("propose")
            .expect("the established leader unexpectedly stepped down mid-workload");
        assert!(
            nodes[leader].wait_for_commit(idx, TEST_DEADLINE),
            "entry {idx} did not commit under the stable leader within deadline"
        );
        last_committed = idx;
    }
    (leader, last_committed)
}

// ---------------------------------------------------------------------------
// Gate (a): commit → byte-identical committed log + to_bits-identical state
// ---------------------------------------------------------------------------

#[test]
fn gate_a_commit_yields_byte_identical_log_and_bit_identical_state() {
    let start = Instant::now();
    let nodes = boot_cluster(3, &["a-n0", "a-n1", "a-n2"]);
    let updates = workload();
    let (leader, committed) = drive_workload(start, &nodes, &updates);
    // No re-proposing ⇒ the committed log is exactly the workload.
    assert_eq!(committed, (updates.len() - 1) as u64);

    // Every node converges to the committed index.
    assert!(
        wait_until(start, || nodes
            .iter()
            .all(|n| n.commit_index() == Some(committed))),
        "nodes did not converge to committed index within deadline"
    );

    let leader_log = nodes[leader].log_bytes().expect("leader log bytes");
    let oracle = oracle_bits(&updates, updates.len());
    for (i, n) in nodes.iter().enumerate() {
        assert_eq!(
            n.log_bytes().unwrap(),
            leader_log,
            "node {i} log not byte-identical to leader"
        );
        assert_eq!(
            n.applied_bits(),
            oracle,
            "node {i} state not to_bits-identical"
        );
        // Re-derive bit-identity straight off the durable log bytes.
        assert_eq!(
            replay_log_to_state(&n.log_bytes().unwrap()).to_bits(),
            oracle,
            "node {i} journal replay not to_bits-identical"
        );
    }

    assert_within_deadline(start);
    for n in nodes {
        n.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Gate (b): quorum-commit correctness — no false progress on a lost quorum
// ---------------------------------------------------------------------------

#[test]
fn gate_b_partitioned_minority_makes_no_false_progress() {
    let start = Instant::now();
    let nodes = boot_cluster(3, &["b-n0", "b-n1", "b-n2"]);
    let updates = workload();
    let (leader, _committed) = drive_workload(start, &nodes[..], &updates[..2]);
    let committed_before = nodes[leader].commit_index();
    assert_eq!(committed_before, Some(1));

    // Kill the OTHER two nodes, leaving the leader a minority of 1/3. We must keep
    // the leader and shut down the two non-leaders. Move the leader out first.
    let mut nodes = nodes;
    let leader_node = nodes.remove(leader);
    for n in nodes {
        n.shutdown(); // the two peers vanish → leader is an isolated minority
    }

    // The leader will detect its peers are gone. While it still believes itself
    // leader it may accept proposals locally, but they MUST NOT commit (no
    // majority). It may also step down to candidate. Either way: no new commit.
    for u in &updates[2..5] {
        let _ = leader_node.propose(u).expect("propose under minority");
        assert_within_deadline(start);
    }
    // Give the cluster ample time; none of the minority-era entries may commit.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        leader_node.commit_index(),
        committed_before,
        "a minority leader falsely advanced the commit index"
    );
    // Applied state reflects only the truly-committed prefix.
    assert_eq!(
        leader_node.applied_bits(),
        oracle_bits(&updates, 2),
        "applied state included uncommitted entries"
    );

    assert_within_deadline(start);
    leader_node.shutdown();
}

#[test]
fn gate_b_bare_majority_commits() {
    let start = Instant::now();
    let nodes = boot_cluster(3, &["bm-n0", "bm-n1", "bm-n2"]);
    let updates = workload();
    // Commit one entry with full quorum, then drop ONE non-leader: leader + 1
    // survivor = 2/3 = a bare majority → still commits.
    let (leader, _committed) = drive_workload(start, &nodes[..], &updates[..1]);
    assert_eq!(nodes[leader].commit_index(), Some(0));

    let mut nodes = nodes;
    // Remove and shut down one non-leader.
    let victim = (0..3).find(|&i| i != leader).unwrap();
    let victim_node = nodes.remove(victim);
    victim_node.shutdown();
    // recompute leader index after removal
    let leader = nodes.iter().position(|n| n.is_leader());
    let leader = match leader {
        Some(l) => l,
        // If the leader changed, re-elect over the surviving 2 (still a quorum of 3? no:
        // surviving 2 of original 3 IS a majority, so an election can still succeed).
        None => await_leader(start, &nodes).expect("survivors elect a leader"),
    };

    for u in &updates[1..3] {
        let idx = loop {
            assert_within_deadline(start);
            if let Some(i) = nodes[leader].propose(u).expect("propose") {
                break i;
            }
            // leader may have momentarily stepped down; re-find it
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            nodes[leader].wait_for_commit(idx, TEST_DEADLINE),
            "bare-majority entry {idx} did not commit"
        );
    }
    assert_eq!(nodes[leader].commit_index(), Some(2));

    // The surviving follower converged to the committed, to_bits-identical state.
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(2))));
    for n in &nodes {
        assert_eq!(n.applied_bits(), oracle_bits(&updates, 3));
    }

    assert_within_deadline(start);
    for n in nodes {
        n.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Gate (c): leader failover — survivors auto-elect and keep progressing
// ---------------------------------------------------------------------------

#[test]
fn gate_c_leader_failover_elects_new_leader_no_committed_loss() {
    let start = Instant::now();
    // 5-node cluster so losing one leaves a 4-node cluster whose 3-node majority
    // can still elect and commit.
    let nodes = boot_cluster(5, &["c-n0", "c-n1", "c-n2", "c-n3", "c-n4"]);
    let updates = workload();
    let (leader, committed) = drive_workload(start, &nodes[..], &updates);
    assert_eq!(committed, (updates.len() - 1) as u64);
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(committed))));
    let old_term = nodes[leader].term();
    let committed_log = nodes[leader].log_bytes().unwrap();

    // KILL the leader.
    let mut nodes = nodes;
    let dead = nodes.remove(leader);
    dead.shutdown();

    // The survivors must elect a NEW leader in a strictly higher term, bounded.
    let new_leader = await_leader(start, &nodes).expect("survivors elect a new leader");
    assert!(
        nodes[new_leader].term() > old_term,
        "new leader did not advance the term"
    );

    // ZERO committed loss: the new leader's committed prefix is byte-identical.
    let new_log = nodes[new_leader].log_bytes().unwrap();
    assert!(
        new_log.len() >= committed_log.len(),
        "new leader lost committed entries"
    );
    assert_eq!(
        &new_log[..committed_log.len()],
        &committed_log[..],
        "new leader's committed prefix diverged"
    );

    // It keeps making progress: a fresh proposal commits.
    let extra = BookUpdate::Set {
        key: 99,
        value: 1.234_567_890_123_456_7,
    };
    let idx = loop {
        assert_within_deadline(start);
        if let Some(i) = nodes[new_leader].propose(&extra).expect("propose") {
            break i;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        nodes[new_leader].wait_for_commit(idx, TEST_DEADLINE),
        "new leader could not commit a fresh proposal"
    );

    // All survivors converge to the new committed state, to_bits-identically.
    let new_committed = nodes[new_leader].commit_index().unwrap();
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(new_committed))));
    let mut want = BookState::new();
    for u in &updates {
        want.apply(u);
    }
    want.apply(&extra);
    let want_bits = want.to_bits();
    for n in &nodes {
        assert_eq!(n.applied_bits(), want_bits, "survivor state diverged");
    }

    assert_within_deadline(start);
    for n in nodes {
        n.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Gate (d): crash-recovery — restart from journal recovers exact committed state
// ---------------------------------------------------------------------------

#[test]
fn gate_d_crash_recovery_rebuilds_exact_committed_state() {
    let start = Instant::now();
    let nodes = boot_cluster(3, &["d-n0", "d-n1", "d-n2"]);
    let updates = workload();
    let (leader, committed) = drive_workload(start, &nodes[..], &updates);
    assert_eq!(committed, (updates.len() - 1) as u64);
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(committed))));

    // Pick a follower to crash and recover.
    let victim = (0..3).find(|&i| i != leader).unwrap();
    let victim_path = nodes[victim].path().to_path_buf();
    let pre_crash_log = nodes[victim].log_bytes().unwrap();
    let oracle = oracle_bits(&updates, updates.len());
    assert_eq!(nodes[victim].applied_bits(), oracle);

    // "Crash" the whole cluster (journals are fsync'd on disk).
    for n in nodes {
        n.shutdown();
    }

    // Restart the victim node PURELY from its journal (solo cluster of 1 so it
    // recovers and can read its own durable state without needing peers). Its
    // recovered durable log and applied state must be exact.
    let restarted =
        RaftNode::boot(&victim_path, &[], 1, test_cfg()).expect("victim recovers from journal");
    assert_eq!(
        restarted.log_bytes().unwrap(),
        pre_crash_log,
        "durable log not byte-identical after crash"
    );
    assert!(wait_until(start, || restarted.commit_index() == Some(committed)));
    assert_eq!(
        restarted.applied_bits(),
        oracle,
        "crash-recovered state not to_bits-identical"
    );

    assert_within_deadline(start);
    restarted.shutdown();
}

// ---------------------------------------------------------------------------
// Sanity: a freshly booted multi-node cluster elects exactly one leader.
// ---------------------------------------------------------------------------

#[test]
fn elects_exactly_one_leader() {
    let start = Instant::now();
    let nodes = boot_cluster(3, &["e-n0", "e-n1", "e-n2"]);
    let leader = await_leader(start, &nodes).expect("exactly one leader elected");
    assert_eq!(nodes[leader].role(), Role::Leader);
    // The other two are followers (or transient candidates that lost) — none is a
    // second leader.
    let leaders = nodes.iter().filter(|n| n.is_leader()).count();
    assert_eq!(leaders, 1, "more than one leader at once");
    assert_within_deadline(start);
    for n in nodes {
        n.shutdown();
    }
}
