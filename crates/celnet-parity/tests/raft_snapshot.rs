//! Parity row: Raft §7 **InstallSnapshot RPC** in `celnet-replog` — the last open
//! Raft seam, proven over a **real `127.0.0.1` loopback cluster**.
//!
//! The local snapshotting, log prefix discard, and snapshot-seeded recovery were
//! already built and gated (`raft_compaction.rs`). This row proves the missing
//! piece: when a leader has **compacted past** the entries a far-behind / restarted
//! follower needs (the follower's required next index < the leader's log
//! `base_index`), the leader cannot AppendEntries the gap — it **transfers the
//! durable snapshot over the wire** ([`celnet_replog::Message::InstallSnapshot`]),
//! and the follower installs it and resumes from `last_included_index + 1`,
//! reaching **bit-identical** (`f64::to_bits`) committed [`BookState`].
//!
//! Rows (each over a real loopback cluster, hard wall-clock bounded):
//! * (a) `lagging_follower_caught_up_via_install_snapshot` — a leader that compacted
//!   past a lagging (killed→restarted-from-its-stale-journal) follower brings it to
//!   bit-identical state via InstallSnapshot.
//! * (b) `restarted_empty_follower_catches_up_via_snapshot_then_tail` — a follower
//!   restarted from an **empty** journal catches up entirely from
//!   (snapshot + AppendEntries tail) to the same to_bits state.
//! * (c) `snapshot_plus_tail_equals_full_log_single_node_replay` — the caught-up
//!   follower's committed to_bits state equals an **independent single-node
//!   deterministic replay** of the full committed `BookUpdate` log (the
//!   `celnet-journal`-backed replay oracle).
//! * (d) `election_and_commit_converge_after_install` — after an install the cluster
//!   still elects (kill the leader) and commits fresh proposals to a single
//!   bit-identical committed state across all survivors.
//!
//! # Independent oracle (never circular)
//!
//! The golden committed state is a **from-scratch single-node deterministic replay**
//! of the committed `BookUpdate` prefix, coded HERE in the test ([`oracle_bits`] /
//! [`single_node_replay_bits`]) — it never reads the cluster, the shipped snapshot,
//! or any node's applied state. Every "caught up" claim is asserted against that
//! external reference, not against the system comparing to itself. The workload
//! carries `0.1 + 0.2` and a 1-ULP `f64::from_bits` value so it is the BITS, not a
//! rounded decimal, that must match.
//!
//! # Honest boundary (verbatim from `celnet-replog`)
//!
//! The multi-node proof runs **logical nodes over real loopback (`127.0.0.1`) TCP
//! sockets on ephemeral ports** — genuine OS sockets, not a shared-memory fake.
//! Loopback proves the **consensus + snapshot-transfer arithmetic** (the
//! InstallSnapshot send-when-behind, the durable follower install, bit-identical
//! catch-up) and **relative regression**: an upper bound on compute, a lower bound
//! on cross-host wire latency. The **absolute cross-host wire p99 / inter-datacentre
//! replication SLO**, real network partitions, and the §11 absolute wire SLOs are
//! **deploy-gated** and never claimed from this repository.

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use celnet_replog::{BookState, BookUpdate, LogEntry, RaftConfig, RaftNode};

/// Hard wall-clock deadline — nothing here may hang. Sized generously for the
/// heaviest snapshot+failover workflow (multiple elections at 400–800ms each +
/// InstallSnapshot + a leader-kill re-election) running under the fully-loaded
/// `just t2` parallel test-integration on one machine — AND concurrently with
/// other parallel Claude sessions' non-cargo load (vitest/Playwright e2e) on the
/// same M4, where CPU starvation can stretch each Raft phase (observed a clean run
/// land at 92s against the prior 90s cap). A genuine deadlock still trips 180s well
/// within. (Mirrors the celnet-server/replog TEST_DEADLINE contention hardening.)
const TEST_DEADLINE: Duration = Duration::from_secs(180);

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Loopback-tuned timing identical in spirit to the `celnet-replog` replication
/// suite: election timeout ~10–20× the heartbeat with a wide random spread so an
/// occasional fsync stall does not trip a spurious election, while a dead leader is
/// still detected within ~half a second.
fn cfg() -> RaftConfig {
    RaftConfig {
        election_min: Duration::from_millis(400),
        election_max: Duration::from_millis(800),
        heartbeat: Duration::from_millis(40),
        io_timeout: Duration::from_secs(2),
    }
}

/// A fresh, unique journal directory + path under the OS temp dir. Returns the
/// directory (so a test can wipe it to simulate an empty-journal restart) and the
/// journal path inside it.
fn temp_journal(tag: &str) -> (PathBuf, PathBuf) {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut dir = std::env::temp_dir();
    dir.push(format!("celnet-parity-raftsnap-{tag}-{pid}-{nanos}-{n}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let mut path = dir.clone();
    path.push("log.journal");
    (dir, path)
}

#[track_caller]
fn assert_within_deadline(start: Instant) {
    assert!(
        start.elapsed() < TEST_DEADLINE,
        "test exceeded {TEST_DEADLINE:?} deadline (possible deadlock/regression)"
    );
}

/// Spin until `cond` is true or the deadline elapses; return whether it became true.
fn wait_until(start: Instant, mut cond: impl FnMut() -> bool) -> bool {
    while start.elapsed() < TEST_DEADLINE {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    cond()
}

/// A deterministic workload whose BITS matter: a `0.1 + 0.2` fold, a 1-ULP
/// `f64::from_bits` value, a `-0.0`, ordinary sets/adds/removes — so the `to_bits`
/// oracle catches any decimal rounding, sign-of-zero, or single-ULP error. Long
/// enough that an interior compaction boundary strands a follower meaningfully.
fn workload() -> Vec<BookUpdate> {
    vec![
        BookUpdate::Set { key: 1, value: 1.0 },
        BookUpdate::Add { key: 1, delta: 0.1 },
        BookUpdate::Add { key: 1, delta: 0.2 }, // key 1 = 1.0 + 0.1 + 0.2 (bit-exact)
        BookUpdate::Set {
            key: 2,
            value: f64::from_bits(0x3ff0_0000_0000_0001), // 1.0 + 1 ULP
        },
        BookUpdate::Set {
            key: 3,
            value: -0.0,
        },
        BookUpdate::Set {
            key: 4,
            value: 12_345.678_9,
        },
        BookUpdate::Add {
            key: 4,
            delta: -0.000_1,
        },
        BookUpdate::Remove { key: 3 },
        BookUpdate::Set { key: 5, value: 2.5 },
        BookUpdate::Add { key: 5, delta: 2.5 },
        BookUpdate::Set {
            key: 6,
            value: 1.0 / 3.0,
        },
        BookUpdate::Set {
            key: 7,
            value: 99.0,
        },
    ]
}

/// INDEPENDENT ORACLE: a from-scratch fresh-`BookState` replay of the first
/// `committed` updates of the workload. Coded HERE — never reads the cluster, the
/// snapshot, or any node's state.
fn oracle_bits(updates: &[BookUpdate], committed: usize) -> Vec<(u64, u64)> {
    let mut s = BookState::new();
    for u in &updates[..committed] {
        s.apply(u);
    }
    s.to_bits()
}

/// INDEPENDENT ORACLE (the `celnet-journal` deterministic-replay golden): replay a
/// node's durable log entry bytes through a fresh `BookState`, proving the on-disk
/// log alone determines the committed state — used to assert snapshot+tail catch-up
/// equals a full single-node replay of the committed `BookUpdate` log.
fn single_node_replay_bits(entry_bytes: &[Vec<u8>]) -> Vec<(u64, u64)> {
    let mut s = BookState::new();
    for bytes in entry_bytes {
        let entry = LogEntry::decode(bytes).expect("durable entry decodes");
        let upd = BookUpdate::decode(&entry.payload).expect("payload decodes");
        s.apply(&upd);
    }
    s.to_bits()
}

/// A bound listener + its address (the address is known before boot).
struct Bound {
    listener: TcpListener,
    addr: SocketAddr,
}

fn bind() -> Bound {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback");
    let addr = listener.local_addr().unwrap();
    Bound { listener, addr }
}

/// Re-bind a listener to a SPECIFIC (previously-used) loopback port, retrying
/// briefly so a node can be restarted on its stable id/port after shutdown. The
/// listening socket itself is not held in TIME_WAIT (only accepted connections may
/// be, and those do not block a fresh LISTEN), so this binds immediately on
/// loopback in practice; the retry is defensive against scheduler jitter.
fn rebind(addr: SocketAddr, start: Instant) -> TcpListener {
    loop {
        assert_within_deadline(start);
        match TcpListener::bind(addr) {
            Ok(l) => return l,
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Boot a fully-meshed N-node cluster from pre-bound listeners + journal paths.
/// Every node gets the full peer-address set (fixed membership) and is served on
/// exactly the port in that set.
fn boot_mesh(bounds: Vec<Bound>, paths: &[PathBuf]) -> Vec<RaftNode> {
    let n = bounds.len();
    assert_eq!(n, paths.len());
    let addrs: Vec<SocketAddr> = bounds.iter().map(|b| b.addr).collect();
    let mut nodes = Vec::with_capacity(n);
    for (i, b) in bounds.into_iter().enumerate() {
        let peers: Vec<SocketAddr> = addrs
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, &a)| a)
            .collect();
        nodes.push(
            RaftNode::boot_on(b.listener, paths[i].clone(), &peers, n, cfg())
                .expect("node boots on loopback"),
        );
    }
    nodes
}

/// Wait until exactly one node is leader; return its index.
fn await_one_leader(start: Instant, nodes: &[RaftNode]) -> Option<usize> {
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

/// Wait for a stably-established sole leader (it stays the only leader across a
/// short stability window, so it is actively heartbeating). Avoids committing any
/// priming entry. Returns the stable leader index. Deadline-bounded.
fn establish_stable_leader(start: Instant, nodes: &[RaftNode]) -> usize {
    let stable_window = Duration::from_millis(250);
    loop {
        assert_within_deadline(start);
        let Some(leader) = await_one_leader(start, nodes) else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
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

/// Propose+commit `updates` in order under the current stable leader among the
/// (live) `nodes`, exactly once each. Re-finds the leader if it steps down. Returns
/// the final committed index. Deadline-bounded.
fn commit_each(start: Instant, nodes: &[RaftNode], updates: &[BookUpdate]) -> u64 {
    let mut leader = establish_stable_leader(start, nodes);
    let mut last = 0u64;
    for u in updates {
        let idx = loop {
            assert_within_deadline(start);
            match nodes[leader].propose(u).expect("propose") {
                Some(i) => break i,
                None => {
                    // Stepped down; re-find a leader and retry this update.
                    leader = match await_one_leader(start, nodes) {
                        Some(l) => l,
                        None => {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                    };
                }
            }
        };
        assert!(
            nodes[leader].wait_for_commit(idx, TEST_DEADLINE),
            "entry {idx} did not commit within deadline"
        );
        last = idx;
    }
    last
}

// ===========================================================================
// (a) A leader that COMPACTED PAST a lagging follower catches it up via
//     InstallSnapshot to bit-identical committed state.
// ===========================================================================

#[test]
fn lagging_follower_caught_up_via_install_snapshot() {
    let start = Instant::now();
    let updates = workload();

    // 3-node mesh. Keep the addresses so we can restart the lagging node on its port.
    let (_d0, p0) = temp_journal("a-n0");
    let (_d1, p1) = temp_journal("a-n1");
    let (_d2, p2) = temp_journal("a-n2");
    let bounds = vec![bind(), bind(), bind()];
    let addrs: Vec<SocketAddr> = bounds.iter().map(|b| b.addr).collect();
    let paths = [p0.clone(), p1.clone(), p2.clone()];
    let mut nodes = boot_mesh(bounds, &paths);

    // Commit the first half of the workload with FULL quorum so every node (incl.
    // the soon-to-lag victim) shares a common committed prefix on disk.
    let half = updates.len() / 2;
    let committed_first = commit_each(start, &nodes, &updates[..half]);
    assert_eq!(committed_first, (half - 1) as u64);
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(committed_first))));

    // Pick a FOLLOWER victim to lag, and keep its (path, addr).
    let leader0 = establish_stable_leader(start, &nodes);
    let victim = (0..3).find(|&i| i != leader0).expect("a follower");
    let victim_addr = addrs[victim];
    let victim_path = paths[victim].clone();

    // Kill the victim. The survivors (leader + 1) are a bare majority of 3 → still
    // commit. (Remove from the vec so the victim's threads are joined/stopped.)
    let victim_node = nodes.remove(victim);
    victim_node.shutdown();
    // Index bookkeeping after removal.
    let mut live_paths: Vec<PathBuf> = paths
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != victim)
        .map(|(_, p)| p.clone())
        .collect();

    // Commit the SECOND half on the surviving majority — the victim now lags these.
    let last = commit_each(start, &nodes, &updates[half..]);
    assert_eq!(last, (updates.len() - 1) as u64);
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(last))));

    // The surviving leader COMPACTS PAST the victim's stale match index: compact to
    // its own full applied prefix. After this, the entries the victim needs are GONE
    // from the leader's log (base_index has shifted past them) — so the only way to
    // catch the victim up is the InstallSnapshot RPC.
    let live_leader = establish_stable_leader(start, &nodes);
    let boundary = nodes[live_leader]
        .compact_applied()
        .expect("compact")
        .expect("something compacted");
    assert!(
        boundary >= half as u64,
        "must compact past the victim's prefix"
    );
    assert!(
        nodes[live_leader].base_log_index() > 0,
        "leader log base must have shifted past a discarded prefix"
    );
    // The leader genuinely no longer holds the entries below the boundary.
    assert_eq!(nodes[live_leader].snapshot_index(), Some(boundary));

    // Restart the victim on its STALE journal (it keeps its old committed-first-half
    // prefix on disk) on the SAME port, rejoining the SAME cluster as a real peer.
    let victim_listener = rebind(victim_addr, start);
    let peers: Vec<SocketAddr> = addrs
        .iter()
        .copied()
        .filter(|&a| a != victim_addr)
        .collect();
    let revived = RaftNode::boot_on(victim_listener, victim_path.clone(), &peers, 3, cfg())
        .expect("victim rejoins on its port");
    live_paths.push(victim_path.clone());

    // The leader must now ship the snapshot (next_index < base_index) and bring the
    // revived victim to the EXACT committed to_bits state. Bounded wait.
    let oracle = oracle_bits(&updates, updates.len());
    assert!(
        wait_until(start, || revived.commit_index() == Some(last)
            && revived.applied_bits() == oracle),
        "revived follower did not catch up via InstallSnapshot to the committed state"
    );

    // It actually installed the snapshot over the wire — its durable log base shifted
    // to the leader's boundary (it could only have learned that via InstallSnapshot,
    // since it was offline during the compaction).
    assert_eq!(
        revived.snapshot_index(),
        Some(boundary),
        "revived follower did not install the shipped snapshot boundary"
    );
    assert!(
        revived.base_log_index() > boundary,
        "revived follower's log base did not advance to the snapshot boundary"
    );

    // Spot-check the bit-sensitive entries are exactly right on the revived node.
    let key1 = (1.0f64 + 0.1 + 0.2).to_bits();
    let bits = revived.applied_bits();
    assert_eq!(bits.iter().find(|(k, _)| *k == 1).unwrap().1, key1);
    assert_eq!(
        bits.iter().find(|(k, _)| *k == 2).unwrap().1,
        0x3ff0_0000_0000_0001
    );

    assert_within_deadline(start);
    revived.shutdown();
    for n in nodes {
        n.shutdown();
    }
}

// ===========================================================================
// (b) A follower restarted from an EMPTY journal catches up entirely from
//     (snapshot + AppendEntries tail) to the same to_bits state.
// ===========================================================================

#[test]
fn restarted_empty_follower_catches_up_via_snapshot_then_tail() {
    let start = Instant::now();
    let updates = workload();

    let (d0, p0) = temp_journal("b-n0");
    let (d1, p1) = temp_journal("b-n1");
    let (d2, p2) = temp_journal("b-n2");
    let dirs = [d0, d1, d2];
    let bounds = vec![bind(), bind(), bind()];
    let addrs: Vec<SocketAddr> = bounds.iter().map(|b| b.addr).collect();
    let paths = [p0, p1, p2];
    let mut nodes = boot_mesh(bounds, &paths);

    // Commit the whole workload with full quorum.
    let last = commit_each(start, &nodes, &updates);
    assert_eq!(last, (updates.len() - 1) as u64);
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(last))));

    // Pick a follower; kill it and WIPE its journal directory → it restarts empty.
    let leader0 = establish_stable_leader(start, &nodes);
    let victim = (0..3).find(|&i| i != leader0).expect("a follower");
    let victim_addr = addrs[victim];
    let victim_path = paths[victim].clone();
    let victim_dir = dirs[victim].clone();
    let victim_node = nodes.remove(victim);
    victim_node.shutdown();
    // Remove ALL durable artifacts (journal, snapshot, persist) — a from-empty boot.
    std::fs::remove_dir_all(&victim_dir).expect("wipe victim journal dir");
    std::fs::create_dir_all(&victim_dir).expect("recreate empty victim dir");

    // The surviving leader compacts its full applied prefix, so the discarded
    // entries are gone — a from-empty follower can ONLY catch up via snapshot+tail.
    let live_leader = establish_stable_leader(start, &nodes);
    let boundary = nodes[live_leader]
        .compact_applied()
        .expect("compact")
        .expect("compacted");
    assert_eq!(nodes[live_leader].snapshot_index(), Some(boundary));

    // Propose a FEW MORE entries AFTER the boundary, so the from-empty follower must
    // first install the snapshot, then receive the post-boundary tail via normal
    // AppendEntries — exercising BOTH halves of the catch-up.
    let extra = [
        BookUpdate::Set {
            key: 8,
            value: 4.25,
        },
        BookUpdate::Add {
            key: 8,
            delta: 0.75,
        }, // key 8 = 5.0
        BookUpdate::Set {
            key: 9,
            value: f64::from_bits(0x3ff0_0000_0000_0002), // 1.0 + 2 ULP
        },
    ];
    let mut full = updates.clone();
    full.extend_from_slice(&extra);
    let last2 = commit_each(start, &nodes, &extra);
    assert_eq!(last2, (full.len() - 1) as u64);

    // Restart the victim FROM EMPTY on its port, rejoining the same cluster.
    let victim_listener = rebind(victim_addr, start);
    let peers: Vec<SocketAddr> = addrs
        .iter()
        .copied()
        .filter(|&a| a != victim_addr)
        .collect();
    let revived = RaftNode::boot_on(victim_listener, victim_path, &peers, 3, cfg())
        .expect("from-empty victim rejoins");

    // It catches up via snapshot (the compacted prefix) THEN the post-boundary tail
    // (AppendEntries), to the EXACT committed to_bits state == independent oracle.
    let oracle = oracle_bits(&full, full.len());
    assert!(
        wait_until(start, || revived.commit_index() == Some(last2)
            && revived.applied_bits() == oracle),
        "from-empty follower did not catch up via snapshot + tail"
    );
    assert_eq!(
        revived.snapshot_index(),
        Some(boundary),
        "from-empty follower did not install the snapshot boundary"
    );
    // Bit-sensitive post-boundary tail landed exactly.
    let bits = revived.applied_bits();
    assert_eq!(
        bits.iter().find(|(k, _)| *k == 8).unwrap().1,
        5.0f64.to_bits()
    );
    assert_eq!(
        bits.iter().find(|(k, _)| *k == 9).unwrap().1,
        0x3ff0_0000_0000_0002
    );

    assert_within_deadline(start);
    revived.shutdown();
    for n in nodes {
        n.shutdown();
    }
}

// ===========================================================================
// (c) snapshot + tail catch-up == full-log single-node deterministic replay
//     (the celnet-journal replay oracle is the golden committed state).
// ===========================================================================

#[test]
fn snapshot_plus_tail_equals_full_log_single_node_replay() {
    let start = Instant::now();
    let updates = workload();

    let (d0, p0) = temp_journal("c-n0");
    let (d1, p1) = temp_journal("c-n1");
    let (d2, p2) = temp_journal("c-n2");
    let dirs = [d0, d1, d2];
    let bounds = vec![bind(), bind(), bind()];
    let addrs: Vec<SocketAddr> = bounds.iter().map(|b| b.addr).collect();
    let paths = [p0, p1, p2];
    let mut nodes = boot_mesh(bounds, &paths);

    let last = commit_each(start, &nodes, &updates);
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(last))));

    // Capture an UN-COMPACTED node's full durable committed log BEFORE compaction —
    // this is the byte source for the independent single-node replay oracle.
    let leader0 = establish_stable_leader(start, &nodes);
    let keeper = (0..3).find(|&i| i != leader0).expect("a follower keeper");
    let full_log_bytes = nodes[keeper].log_bytes().expect("keeper log bytes");
    let replay_oracle = single_node_replay_bits(&full_log_bytes);
    // Sanity: the journal-replay oracle == the from-scratch update oracle.
    assert_eq!(replay_oracle, oracle_bits(&updates, updates.len()));

    // Wipe a DIFFERENT follower, compact the leader past it, restart it from empty,
    // and assert its snapshot+tail catch-up equals the single-node replay oracle.
    let victim = (0..3)
        .find(|&i| i != leader0 && i != keeper)
        .expect("third node");
    let victim_addr = addrs[victim];
    let victim_path = paths[victim].clone();
    let victim_dir = dirs[victim].clone();
    let victim_node = nodes.remove(victim);
    victim_node.shutdown();
    std::fs::remove_dir_all(&victim_dir).expect("wipe");
    std::fs::create_dir_all(&victim_dir).expect("recreate");

    let live_leader = establish_stable_leader(start, &nodes);
    let boundary = nodes[live_leader]
        .compact_applied()
        .expect("compact")
        .expect("compacted");

    let victim_listener = rebind(victim_addr, start);
    let peers: Vec<SocketAddr> = addrs
        .iter()
        .copied()
        .filter(|&a| a != victim_addr)
        .collect();
    let revived = RaftNode::boot_on(victim_listener, victim_path, &peers, 3, cfg())
        .expect("rejoins from empty");

    assert!(
        wait_until(start, || revived.commit_index() == Some(last)
            && revived.applied_bits() == replay_oracle),
        "snapshot+tail catch-up != full-log single-node replay oracle"
    );
    assert_eq!(revived.snapshot_index(), Some(boundary));

    assert_within_deadline(start);
    revived.shutdown();
    for n in nodes {
        n.shutdown();
    }
}

// ===========================================================================
// (d) Election + commit still converge after an InstallSnapshot: kill the
//     leader, survivors (incl. the just-installed node) elect a new leader and
//     commit fresh proposals to one bit-identical committed state.
// ===========================================================================

#[test]
fn election_and_commit_converge_after_install() {
    let start = Instant::now();
    let updates = workload();

    let (d0, p0) = temp_journal("d-n0");
    let (d1, p1) = temp_journal("d-n1");
    let (d2, p2) = temp_journal("d-n2");
    let dirs = [d0, d1, d2];
    let bounds = vec![bind(), bind(), bind()];
    let addrs: Vec<SocketAddr> = bounds.iter().map(|b| b.addr).collect();
    let paths = [p0, p1, p2];
    let mut nodes = boot_mesh(bounds, &paths);

    let last = commit_each(start, &nodes, &updates);
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(last))));

    // Wipe + compact-past + restart-from-empty a follower so it catches up via
    // InstallSnapshot (same machinery as (b)).
    let leader0 = establish_stable_leader(start, &nodes);
    let victim = (0..3).find(|&i| i != leader0).expect("a follower");
    let victim_addr = addrs[victim];
    let victim_path = paths[victim].clone();
    let victim_dir = dirs[victim].clone();
    let victim_node = nodes.remove(victim);
    victim_node.shutdown();
    std::fs::remove_dir_all(&victim_dir).expect("wipe");
    std::fs::create_dir_all(&victim_dir).expect("recreate");

    let live_leader = establish_stable_leader(start, &nodes);
    let boundary = nodes[live_leader]
        .compact_applied()
        .expect("compact")
        .expect("compacted");

    let victim_listener = rebind(victim_addr, start);
    let peers: Vec<SocketAddr> = addrs
        .iter()
        .copied()
        .filter(|&a| a != victim_addr)
        .collect();
    let revived = RaftNode::boot_on(victim_listener, victim_path, &peers, 3, cfg())
        .expect("rejoins from empty");

    let oracle = oracle_bits(&updates, updates.len());
    assert!(
        wait_until(start, || revived.commit_index() == Some(last)
            && revived.applied_bits() == oracle),
        "revived follower did not install the snapshot before the failover phase"
    );
    assert_eq!(revived.snapshot_index(), Some(boundary));

    // Re-form the full 3-node live set: the two that stayed up + the revived node.
    // (We own `nodes` (2 live) plus `revived`.) KILL the current leader and require
    // the survivors to elect a NEW leader and keep committing.
    let mut live: Vec<RaftNode> = nodes;
    live.push(revived);
    let old_leader = establish_stable_leader(start, &live);
    let old_term = live[old_leader].term();
    let dead = live.remove(old_leader);
    dead.shutdown();

    // Survivors (2 of 3 = a majority) elect a strictly-higher-term leader.
    let new_leader = await_one_leader(start, &live).expect("survivors elect a new leader");
    assert!(
        live[new_leader].term() > old_term,
        "new leader did not advance the term after failover"
    );

    // Fresh proposals commit on the new leader.
    let extra = [
        BookUpdate::Set {
            key: 11,
            value: 7.0,
        },
        BookUpdate::Add {
            key: 11,
            delta: 0.5,
        },
    ];
    let last2 = commit_each(start, &live, &extra);

    let mut full = updates.clone();
    full.extend_from_slice(&extra);
    let oracle2 = oracle_bits(&full, full.len());
    assert!(
        wait_until(start, || live.iter().all(|n| n.commit_index()
            == Some(last2)
            && n.applied_bits() == oracle2)),
        "survivors did not converge to one bit-identical committed state after failover"
    );

    assert_within_deadline(start);
    for n in live {
        n.shutdown();
    }
}
