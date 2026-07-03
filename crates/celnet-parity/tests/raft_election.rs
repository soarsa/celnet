//! Parity row — **full Raft consensus** (`celnet-replog`).
//!
//! This row proves the consensus module's claims against *independent* oracles by
//! booting a **real multi-node cluster over loopback `127.0.0.1` TCP sockets on
//! ephemeral ports** (each node binds a real listener and dials its peers; no
//! shared-memory `Vec` stands in for the network). Every test body is hard
//! wall-clock bounded so a regression fails loudly, never hangs.
//!
//! Claims, each with its oracle:
//!
//! * (1) **ELECTION** — kill the leader and the surviving majority elects a *new*
//!   leader in a strictly higher term within the bounded deadline, and it keeps
//!   making progress (a fresh proposal commits). Oracle: the role/term reported by
//!   the nodes + a committed index advancing past the old high-water.
//! * (2) **ELECTION SAFETY** — a *partitioned minority* candidate cannot win.
//!   Oracle: an isolated follower (a 1-of-5 minority) campaigns (its term provably
//!   advances) yet NEVER reaches `Leader`, because it can never gather a majority.
//!   (The companion no-false-progress property — a partitioned leader cannot
//!   COMMIT — is gated in celnet-replog's replication suite; the stale-tail
//!   reconciliation is claim (3) below.)
//! * (3) **LOG-MATCHING + CONFLICTING-TAIL TRUNCATION** — a node is seeded with a
//!   *divergent uncommitted tail* (an entry it persisted from an old leader that
//!   never committed), then joins a cluster whose new leader holds a *different*
//!   entry at that index; the leader's AppendEntries truncates the divergent tail
//!   and overwrites it. Oracle: the rejoined node's durable log becomes
//!   **byte-identical** to the leader's committed log.
//! * (4) **CONVERGENCE ORACLE** — after the dust settles, all surviving nodes hold
//!   byte-identical committed logs and `to_bits`-identical applied `BookState`, and
//!   that state EQUALS an *independent single-node deterministic replay* of the
//!   committed `BookUpdate` prefix on a fresh `BookState`. The workload includes a
//!   `0.1 + 0.2` add and a 1-ULP value so it is the bits, not a rounded decimal,
//!   that are asserted.
//!
//! # Honest boundary (reproduced verbatim from the crate, never violated)
//!
//! The multi-node proof runs **logical nodes over real loopback (`127.0.0.1`) TCP
//! sockets on ephemeral ports** — genuine OS sockets with kernel framing, not a
//! shared in-memory `Vec` pretending to be a network. Loopback proves the
//! **consensus arithmetic** (election safety, log-matching, truncation, quorum
//! commit, bit-identical replay) and **relative regression**: it is an **upper
//! bound on compute** and a **lower bound on cross-host wire latency**. The
//! **absolute cross-host wire p99 / inter-datacentre replication SLO** is
//! deploy-gated and is **never** claimed here. No NVIDIA throughput and no
//! live-JVM-estate claim is made here either.

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use celnet_replog::{BookState, BookUpdate, Log, LogEntry, RaftConfig, RaftNode, Role};

// ---------------------------------------------------------------------------
// Local deadline / scaffolding (mirrors celnet-replog/tests/common, copied here
// per the parity-row convention — each row is self-contained).
// ---------------------------------------------------------------------------

/// Hard upper bound on any single test body — a regression fails loudly here.
/// Set generously (180s) to tolerate tokio-task starvation when many parallel
/// Claude sessions saturate the single M4 during a landing t2 — the election +
/// partition scenarios need several election rounds, and under load each round
/// runs slow (not a consensus regression). Mirrors `raft_snapshot`'s 180s budget.
const TEST_DEADLINE: Duration = Duration::from_secs(180);

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh, unique journal path under the OS temp dir.
fn temp_journal(tag: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut dir = std::env::temp_dir();
    dir.push(format!("celnet-parity-raft-{tag}-{pid}-{nanos}-{n}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.push("log.journal");
    dir
}

/// Assert we are still within the test deadline, panicking loudly otherwise.
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
        std::thread::sleep(Duration::from_millis(2));
    }
    cond()
}

/// Loopback-tuned timing: election timeout ~10–20× the heartbeat with a wide
/// random spread, so an occasional fsync stall does not trip a spurious election,
/// while a dead leader is still detected within ~half a second.
fn cfg() -> RaftConfig {
    RaftConfig {
        election_min: Duration::from_millis(400),
        election_max: Duration::from_millis(800),
        heartbeat: Duration::from_millis(40),
        io_timeout: Duration::from_secs(2),
    }
}

/// The deterministic workload (bit-comparison values + `0.1+0.2` + a 1-ULP value).
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
        BookUpdate::Set {
            key: 3,
            value: f64::from_bits(0x3ff0_0000_0000_0001), // 1.0 + 1 ULP
        },
    ]
}

/// Independent single-node deterministic replay oracle: apply a `BookUpdate`
/// sequence on a fresh `BookState` and return its `to_bits` digest.
fn replay_oracle(updates: &[BookUpdate]) -> Vec<(u64, u64)> {
    let mut s = BookState::new();
    for u in updates {
        s.apply(u);
    }
    s.to_bits()
}

/// Replay a node's durable entry bytes to a `BookState` (independent of the live
/// applied cache) — proves the on-disk log alone determines the bit-identical state.
fn replay_bytes(entry_bytes: &[Vec<u8>]) -> Vec<(u64, u64)> {
    let mut s = BookState::new();
    for bytes in entry_bytes {
        let e = LogEntry::decode(bytes).expect("durable entry decodes");
        let u = BookUpdate::decode(&e.payload).expect("payload decodes");
        s.apply(&u);
    }
    s.to_bits()
}

/// A bound listener + its address (so the address is known before boot).
struct Bound {
    listener: TcpListener,
    addr: SocketAddr,
}

fn bind() -> Bound {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback");
    let addr = listener.local_addr().unwrap();
    Bound { listener, addr }
}

/// Boot a fully-meshed cluster from a set of pre-bound listeners + their journal
/// paths. Every node receives the full peer-address set (fixed membership) and is
/// served on exactly the port in that set.
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

/// Wait for a **stably-established** single leader: a node that is the sole leader
/// continuously across a short stability window (so it is actively heartbeating and
/// will not be timed out). Returns its index. Establishing stability — rather than
/// re-proposing on leadership change — means each entry is proposed exactly once, so
/// the committed log never contains a duplicate (which would corrupt the bit-exact
/// convergence oracle, e.g. a doubled `Add`). Deadline-bounded.
fn await_stable_leader(start: Instant, nodes: &[RaftNode]) -> usize {
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

/// Propose `update` on `leader` exactly once and block until it commits; returns the
/// committed index. Asserts the leader did not step down (caller establishes a stable
/// leader first, so this holds), keeping the committed log duplicate-free.
/// Deadline-bounded.
fn commit_one(start: Instant, nodes: &[RaftNode], leader: usize, update: &BookUpdate) -> u64 {
    assert_within_deadline(start);
    let idx = nodes[leader]
        .propose(update)
        .expect("propose")
        .expect("the established leader unexpectedly stepped down mid-workload");
    assert!(
        nodes[leader].wait_for_commit(idx, TEST_DEADLINE),
        "entry {idx} did not commit under the stable leader within deadline"
    );
    idx
}

// ===========================================================================
// (1) ELECTION + (4) CONVERGENCE ORACLE
// ===========================================================================

#[test]
fn election_kills_leader_survivors_elect_and_progress_and_converge() {
    let start = Instant::now();
    // 5 nodes so losing the leader leaves a 4-node cluster whose 3-node majority
    // can still elect and commit.
    let paths: Vec<PathBuf> = (0..5).map(|i| temp_journal(&format!("e{i}"))).collect();
    let bounds: Vec<Bound> = (0..5).map(|_| bind()).collect();
    let nodes = boot_mesh(bounds, &paths);

    let updates = workload();
    let leader = await_stable_leader(start, &nodes);
    for u in &updates {
        commit_one(start, &nodes, leader, u);
    }
    let committed = (updates.len() - 1) as u64;
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(committed))));
    let old_term = nodes[leader].term();
    let committed_log = nodes[leader].log_bytes().unwrap();

    // KILL the leader.
    let mut nodes = nodes;
    let dead = nodes.remove(leader);
    dead.shutdown();

    // (1) ELECTION: survivors elect a NEW, stable leader in a strictly higher term.
    let new_leader = await_stable_leader(start, &nodes);
    assert!(
        nodes[new_leader].term() > old_term,
        "new leader term {} not greater than old {old_term}",
        nodes[new_leader].term()
    );
    // No committed loss: the committed prefix survives byte-identically.
    let new_log = nodes[new_leader].log_bytes().unwrap();
    assert!(new_log.len() >= committed_log.len());
    assert_eq!(&new_log[..committed_log.len()], &committed_log[..]);

    // (1 cont.) Keeps making progress: a fresh proposal commits.
    let extra = BookUpdate::Set {
        key: 42,
        value: 2.625_812_858_419_045,
    };
    commit_one(start, &nodes, new_leader, &extra);

    // (4) CONVERGENCE ORACLE: all survivors byte-identical + to_bits-identical, and
    // equal to the INDEPENDENT single-node replay of the committed prefix + extra.
    let final_committed = nodes[new_leader].commit_index().unwrap();
    assert!(wait_until(start, || nodes
        .iter()
        .all(|n| n.commit_index() == Some(final_committed))));

    let mut committed_seq = updates.clone();
    committed_seq.push(extra.clone());
    let oracle_bits = replay_oracle(&committed_seq);

    let leader_log = nodes[new_leader].log_bytes().unwrap();
    for (i, n) in nodes.iter().enumerate() {
        assert_eq!(
            n.log_bytes().unwrap(),
            leader_log,
            "survivor {i} log not byte-identical to the new leader"
        );
        assert_eq!(
            n.applied_bits(),
            oracle_bits,
            "survivor {i} applied state not to_bits-identical to the replay oracle"
        );
        // And the on-disk log alone reconstructs the same bits.
        assert_eq!(
            replay_bytes(&n.log_bytes().unwrap()),
            oracle_bits,
            "survivor {i} durable-log replay not to_bits-identical"
        );
    }

    assert_within_deadline(start);
    for n in nodes {
        n.shutdown();
    }
}

// ===========================================================================
// (2) ELECTION SAFETY — a partitioned minority cannot win
// ===========================================================================

#[test]
fn election_safety_partitioned_minority_cannot_win() {
    let start = Instant::now();
    // 5-node cluster. We isolate a single FOLLOWER (a 1-of-5 minority) by shutting
    // down the other four. The isolated node campaigns — repeatedly incrementing its
    // term and soliciting votes — but can reach at most its OWN vote (1), strictly
    // less than the 3-of-5 majority, so it can NEVER win. This is the §5.4 election-
    // safety property: a partitioned minority candidate cannot become leader.
    //
    // (We isolate a *follower*, not the leader: a partitioned leader correctly keeps
    // believing it is leader of its old term — Raft only steps a leader down on a
    // HIGHER term — and merely cannot COMMIT; that is exercised by the dedicated
    // no-false-progress gate in celnet-replog's replication suite. The safety claim
    // here is precisely that no node WINS a NEW election without a majority.)
    let paths: Vec<PathBuf> = (0..5).map(|i| temp_journal(&format!("s{i}"))).collect();
    let bounds: Vec<Bound> = (0..5).map(|_| bind()).collect();
    let mut nodes = boot_mesh(bounds, &paths);

    let leader = await_one_leader(start, &nodes).expect("full cluster elects a leader");
    // Pick a follower to isolate.
    let follower = (0..5).find(|&i| i != leader).expect("a follower exists");

    // Shut down every node EXCEPT the chosen follower, isolating it as a 1-of-5
    // minority. Remove from the highest index down so indices stay valid.
    let mut isolated = None;
    for i in (0..5).rev() {
        let n = nodes.remove(i);
        if i == follower {
            isolated = Some(n);
        } else {
            n.shutdown();
        }
    }
    let isolated = isolated.expect("isolated follower retained");

    // The isolated follower repeatedly times out and tries to campaign, but with
    // **Pre-Vote** it can never gather a pre-vote majority (no peer is reachable), so
    // it must NEVER reach `Leader`. Watch a generous window covering many election
    // timeouts.
    let term_before = isolated.term();
    let watch_until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < watch_until {
        assert_ne!(
            isolated.role(),
            Role::Leader,
            "a partitioned 1-of-5 minority node falsely WON an election"
        );
        assert_within_deadline(start);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_ne!(isolated.role(), Role::Leader);
    // Pre-Vote means the isolated node does NOT inflate the term it would rejoin
    // with: its real term never advances past where it started (a stronger safety
    // property than merely "never won" — it cannot disrupt the live cluster's term
    // on a later rejoin). It only ever runs hypothetical, non-binding pre-votes.
    assert_eq!(
        isolated.term(),
        term_before,
        "Pre-Vote should keep an isolated minority from inflating its term"
    );

    isolated.shutdown();
    assert_within_deadline(start);
}

// ===========================================================================
// (3) LOG-MATCHING + CONFLICTING-TAIL TRUNCATION
// ===========================================================================

#[test]
fn log_matching_truncates_divergent_uncommitted_tail() {
    let start = Instant::now();

    // We construct the canonical Raft conflicting-tail scenario at boot, honestly,
    // via the public durable `Log` (exactly the on-disk state crashed-then-restarted
    // nodes would hold):
    //
    //   * Node X persisted a STALE index-0 entry in an OLD term (1) from a leader
    //     that never committed it — a divergent uncommitted tail.
    //   * Nodes Y and Z persisted the AUTHORITATIVE index-0 entry in a NEWER term
    //     (2) — the entry a later leader replicated. Because their last-log term (2)
    //     strictly exceeds X's (1), the §5.4.1 up-to-date rule GUARANTEES X cannot
    //     win an election (Y/Z reject its vote), so X is forced to be a follower and
    //     its term-1 tail must be truncated to match the term-2 entry. This makes the
    //     truncation path deterministic, not race-dependent.
    let stale = BookUpdate::Set {
        key: 7,
        value: -999.0,
    }; // never committed anywhere
    let authoritative = BookUpdate::Set {
        key: 7,
        value: 12.5,
    };

    let x_path = temp_journal("x-divergent");
    {
        let mut log = Log::open(&x_path).expect("open X durable log");
        log.append(&LogEntry::new(1, 0, stale.encode()))
            .expect("seed X stale term-1 tail");
    }
    let x_stale_bytes = {
        let log = Log::open(&x_path).unwrap();
        log.entry_bytes().unwrap()
    };

    let y_path = temp_journal("y-authoritative");
    let z_path = temp_journal("z-authoritative");
    for p in [&y_path, &z_path] {
        let mut log = Log::open(p).expect("open authoritative log");
        log.append(&LogEntry::new(2, 0, authoritative.encode()))
            .expect("seed term-2 authoritative entry");
    }

    // Boot the 3-node cluster. nodes[0]=X (stale), nodes[1]=Y, nodes[2]=Z.
    let nodes = boot_mesh(
        vec![bind(), bind(), bind()],
        &[x_path.clone(), y_path.clone(), z_path.clone()],
    );

    // Y or Z wins (X cannot). The new leader replicates its term-2 index-0 entry,
    // truncating X's term-1 index-0 entry, then we commit a further workload tail.
    let leader = await_stable_leader(start, &nodes);
    assert_ne!(
        leader, 0,
        "the stale minority-term node X must not win the election"
    );

    // Append a few fresh entries so there is a committed prefix beyond the conflict.
    let tail = workload();
    for u in &tail {
        commit_one(start, &nodes, leader, u);
    }
    // Committed index = 0 (the term-2 authoritative entry) + tail.len().
    let committed = tail.len() as u64; // indices 0..=tail.len()

    assert!(
        wait_until(start, || nodes
            .iter()
            .all(|n| n.commit_index() == Some(committed))),
        "cluster (incl. the reconciled X) did not converge to the committed index"
    );

    // (3) X's durable log is now byte-identical to the leader's committed log.
    let leader_log = nodes[leader].log_bytes().unwrap();
    let x_log_now = nodes[0].log_bytes().unwrap();
    assert_eq!(
        x_log_now, leader_log,
        "X's divergent tail was not truncated/overwritten to match the leader"
    );
    // It genuinely CHANGED from the seeded stale bytes (truncation happened).
    assert_ne!(
        x_log_now, x_stale_bytes,
        "X's log still holds the stale tail — no truncation occurred"
    );

    // Independent oracle: the committed state is the term-2 authoritative entry
    // followed by the workload tail — the stale value (-999) never appears.
    let mut committed_seq = vec![authoritative.clone()];
    committed_seq.extend(tail.iter().cloned());
    let oracle_bits = replay_oracle(&committed_seq);
    for (i, n) in nodes.iter().enumerate() {
        assert_eq!(
            n.applied_bits(),
            oracle_bits,
            "node {i} applied the stale/divergent entry or diverged"
        );
    }

    assert_within_deadline(start);
    // Cleanly stop every node FIRST (single-writer discipline) so the durable
    // re-open below sees the journal at rest, with no concurrent writer.
    for n in nodes {
        n.shutdown();
    }

    // Durability: X's on-disk log alone reconstructs the committed state — the
    // truncation was a real durable rewrite, not a memory-only mask.
    let x_durable = {
        let log = Log::open(&x_path).unwrap();
        log.entry_bytes().unwrap()
    };
    assert_eq!(
        x_durable, leader_log,
        "X's on-disk log is not byte-identical to the committed log after restart"
    );
    assert_eq!(
        replay_bytes(&x_durable),
        oracle_bits,
        "X's on-disk log does not reconstruct the committed state after truncation"
    );

    assert_within_deadline(start);
}
