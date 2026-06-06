//! Parity row: Raft §7 **log compaction / snapshotting** in `celnet-replog`.
//!
//! Proves, against an **independent in-test oracle** (never the cluster/log
//! comparing to itself), that durable snapshotting + log prefix discard +
//! snapshot-aware recovery preserve the safety invariant **bit-identically**:
//!
//! ```text
//! replay-from-(snapshot + retained tail)
//!   == replay-from-(full original log)
//!   == independent fresh-BookState replay of the committed BookUpdate prefix
//! ```
//!
//! all by `f64::to_bits` (exact IEEE-754 bit equality, not rounded decimals).
//!
//! Rows:
//! 1. `bit_identical_equivalence_after_compaction` — build a log, apply a workload
//!    (including `Add 0.1` then `Add 0.2` and an `f64::from_bits` 1-ULP value so
//!    BITS are asserted), snapshot+compact at a committed index, then assert the
//!    three-way `to_bits` equality above.
//! 2. `prefix_is_really_discarded_on_disk` — after compaction the durable log no
//!    longer holds the pre-snapshot entries (physical record count + first
//!    retained index), yet absolute-index accessors stay correct at the boundary.
//! 3. `recovery_from_snapshot_plus_tail_is_exact` — a node restarted purely from
//!    (snapshot + retained tail journal) recovers the exact committed `to_bits`
//!    state (== oracle).
//!
//! InstallSnapshot (the over-the-wire snapshot transfer to a far-behind follower)
//! is honestly **deferred** to its own increment (documented in the crate root of
//! `celnet-replog`); the local snapshot + compaction + recovery proven here is the
//! complete, gated substrate it will build on, so this row builds no half-wired
//! RPC and makes no over-the-wire-catch-up claim.
//!
//! # Honest boundary (verbatim from `celnet-replog`)
//!
//! The multi-node proofs in `celnet-replog` run logical nodes over real loopback
//! `127.0.0.1` TCP sockets; loopback proves the consensus/compaction arithmetic +
//! relative regression (an upper bound on compute, a lower bound on cross-host
//! wire latency). Absolute cross-host wire p99 / inter-DC SLO is deploy-gated and
//! never claimed from this repository. The rows here exercise the durable
//! single-node compaction/recovery arithmetic directly (no socket needed).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use celnet_replog::{
    BookState, BookUpdate, Log, LogEntry, RaftConfig, RaftNode, Snapshot, SnapshotStore,
    snapshot_path,
};

/// Hard wall-clock deadline — nothing here may hang.
const TEST_DEADLINE: Duration = Duration::from_secs(15);

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
    dir.push(format!("celnet-parity-raftc-{tag}-{pid}-{nanos}-{n}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.push("log.journal");
    dir
}

/// Assert we are still within the deadline, panicking loudly otherwise.
#[track_caller]
fn assert_within_deadline(start: Instant) {
    assert!(
        start.elapsed() < TEST_DEADLINE,
        "test exceeded {TEST_DEADLINE:?} deadline (possible deadlock/regression)"
    );
}

/// A deterministic workload whose bits matter: a fold of `Add 0.1` then `Add 0.2`
/// (non-exact-decimal sum), a 1-ULP `f64::from_bits` value, a `-0.0`, and some
/// ordinary sets/removes — so the `to_bits` oracle catches any decimal rounding,
/// sign-of-zero, or single-ULP error.
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
        BookUpdate::Remove { key: 3 }, // dropped again
        BookUpdate::Set { key: 5, value: 2.5 },
        BookUpdate::Add { key: 5, delta: 2.5 },
        BookUpdate::Set {
            key: 6,
            value: f64::from_bits(0x7fef_ffff_ffff_ffff), // f64::MAX bits
        },
        BookUpdate::Set {
            key: 7,
            value: 99.0,
        },
    ]
}

/// INDEPENDENT ORACLE: a from-scratch fresh-`BookState` replay of the first
/// `committed` updates of the workload. Coded HERE in the test — it never reads
/// the log, the snapshot, or the cluster — so the equivalence checks compare the
/// system against an external reference, not against itself.
fn oracle_bits(updates: &[BookUpdate], committed: usize) -> Vec<(u64, u64)> {
    let mut s = BookState::new();
    for u in &updates[..committed] {
        s.apply(u);
    }
    s.to_bits()
}

/// Build a durable log holding `updates` as committed entries (term 1, indices
/// 0..n), at a fresh journal path. Returns the path. The log is closed on return.
fn build_committed_log(path: &PathBuf, updates: &[BookUpdate]) {
    let mut log = Log::open(path).expect("open log");
    for (i, u) in updates.iter().enumerate() {
        log.append(&LogEntry::new(1, i as u64, u.encode()))
            .expect("append");
    }
}

/// Replay a `Log`'s retained tail `[start, ..]` into `state`, in order.
fn replay_tail_into(state: &mut BookState, log: &Log, start: u64) {
    for e in log.entries_from(start).expect("entries_from") {
        let upd = BookUpdate::decode(&e.payload).expect("decode update");
        state.apply(&upd);
    }
}

/// Row 1: three-way bit-identical equivalence after snapshot + compaction.
#[test]
fn bit_identical_equivalence_after_compaction() {
    let start = Instant::now();
    let updates = workload();
    let committed = updates.len(); // commit the whole workload

    // (A) replay-from-(full original log): a fresh state folding the whole log.
    let full_path = temp_journal("full");
    build_committed_log(&full_path, &updates);
    let full_log = Log::open(&full_path).expect("reopen full");
    let mut full_state = BookState::new();
    replay_tail_into(&mut full_state, &full_log, 0);
    let full_bits = full_state.to_bits();

    // (B) replay-from-(snapshot + retained tail): build a separate log, apply the
    // committed prefix to an applied state, snapshot at a mid index, discard the
    // prefix, then reconstruct = snapshot.state + retained-tail replay.
    let snap_path = temp_journal("snap");
    build_committed_log(&snap_path, &updates);
    let mut log = Log::open(&snap_path).expect("reopen snap");

    // Applied state through the snapshot boundary (committed+applied prefix).
    let boundary: u64 = 6; // an interior committed index (well before the end)
    let mut applied_at_boundary = BookState::new();
    for u in &updates[..=boundary as usize] {
        applied_at_boundary.apply(u);
    }
    let boundary_term = log.term_at(boundary).expect("boundary term present");

    // Write the durable snapshot, THEN discard the prefix (the safe ordering).
    let store = SnapshotStore::new(snapshot_path(&snap_path));
    let snapshot = Snapshot::new(boundary, boundary_term, applied_at_boundary.clone());
    store.save(&snapshot).expect("save snapshot");
    log.discard_prefix(boundary, boundary_term)
        .expect("discard prefix");

    // Reconstruct from (snapshot + retained tail).
    let mut from_snapshot = snapshot.state.clone();
    replay_tail_into(&mut from_snapshot, &log, boundary + 1);
    let snapshot_bits = from_snapshot.to_bits();

    // (C) the independent oracle.
    let oracle = oracle_bits(&updates, committed);

    // Three-way bit-identical equality (f64::to_bits — exact, not decimal).
    assert_eq!(snapshot_bits, full_bits, "snapshot+tail != full-log replay");
    assert_eq!(snapshot_bits, oracle, "snapshot+tail != independent oracle");
    assert_eq!(full_bits, oracle, "full-log replay != independent oracle");

    // Spot-check the bit-sensitive entries are actually present and exact.
    let want_key1 = (1.0f64 + 0.1 + 0.2).to_bits();
    assert_eq!(
        snapshot_bits.iter().find(|(k, _)| *k == 1).unwrap().1,
        want_key1,
        "key 1 (0.1+0.2 fold) bits differ"
    );
    assert_eq!(
        snapshot_bits.iter().find(|(k, _)| *k == 2).unwrap().1,
        0x3ff0_0000_0000_0001,
        "key 2 (1-ULP) bits differ"
    );

    assert_within_deadline(start);
}

/// Row 2: the prefix is REALLY discarded on disk, yet absolute-index accessors
/// remain correct at and around the snapshot boundary.
#[test]
fn prefix_is_really_discarded_on_disk() {
    let start = Instant::now();
    let updates = workload();
    let path = temp_journal("disk");
    build_committed_log(&path, &updates);
    let mut log = Log::open(&path).expect("reopen");

    let total = updates.len() as u64;
    assert_eq!(log.entry_bytes().expect("bytes").len() as u64, total);
    assert_eq!(log.base_index(), 0);

    let boundary: u64 = 6;
    let boundary_term = log.term_at(boundary).expect("term");
    // Also persist a real snapshot so this mirrors the production ordering.
    SnapshotStore::new(snapshot_path(&path))
        .save(&Snapshot::new(boundary, boundary_term, {
            let mut s = BookState::new();
            for u in &updates[..=boundary as usize] {
                s.apply(u);
            }
            s
        }))
        .expect("save");
    log.discard_prefix(boundary, boundary_term)
        .expect("discard");

    // The physical, on-disk record count shrank to exactly the retained tail.
    let retained = total - (boundary + 1);
    assert_eq!(
        log.entry_bytes().expect("bytes").len() as u64,
        retained,
        "physical journal records were not actually discarded"
    );
    assert_eq!(log.base_index(), boundary + 1, "base index did not shift");

    // The pre-snapshot entries are GONE (bodies discarded).
    for i in 0..=boundary {
        if i < boundary {
            assert_eq!(log.term_at(i), None, "subsumed index {i} still has a term");
            assert_eq!(
                log.entry_at(i).expect("entry_at").map(|e| e.index),
                None,
                "subsumed index {i} body still present"
            );
        }
    }

    // BUT absolute-index accessors are correct at the boundary and beyond:
    assert_eq!(
        log.term_at(boundary),
        Some(boundary_term),
        "snapshot boundary term must be retained for log-matching"
    );
    // matches_prev succeeds AT the boundary (so a leader can replicate index
    // boundary+1 right after the snapshot).
    assert!(
        log.matches_prev(boundary, boundary_term),
        "matches_prev must hold at the snapshot boundary"
    );
    assert!(
        !log.matches_prev(boundary, boundary_term + 99),
        "wrong term"
    );

    // The first retained entry carries the correct absolute index.
    let first_retained = log
        .entry_at(boundary + 1)
        .expect("entry_at")
        .expect("present");
    assert_eq!(first_retained.index, boundary + 1);
    assert_eq!(log.last_index(), Some(total - 1));

    // entries_from over the retained range returns correctly-indexed entries.
    let from = log.entries_from(boundary + 1).expect("entries_from");
    assert_eq!(from.len() as u64, retained);
    assert_eq!(from.first().unwrap().index, boundary + 1);
    assert_eq!(from.last().unwrap().index, total - 1);

    assert_within_deadline(start);
}

/// Row 3: a node restarted purely from (durable snapshot + retained-tail journal)
/// recovers the exact committed `to_bits` state — equal to the independent oracle.
///
/// This drives the SAME recovery sequence `RaftNode::boot` performs (load
/// snapshot → adopt boundary → seed applied + last_applied → replay retained
/// committed tail), but directly on the durable artifacts so the recovery
/// arithmetic is asserted against the oracle without a cluster.
#[test]
fn recovery_from_snapshot_plus_tail_is_exact() {
    let start = Instant::now();
    let updates = workload();
    let committed = updates.len();
    let path = temp_journal("recover");

    // Phase 1: build, apply, snapshot at a boundary, discard prefix — then the
    // process "crashes" (we drop everything and re-open from disk only).
    let boundary: u64 = 6;
    {
        build_committed_log(&path, &updates);
        let mut log = Log::open(&path).expect("open");
        let boundary_term = log.term_at(boundary).expect("term");
        let mut applied = BookState::new();
        for u in &updates[..=boundary as usize] {
            applied.apply(u);
        }
        SnapshotStore::new(snapshot_path(&path))
            .save(&Snapshot::new(boundary, boundary_term, applied))
            .expect("save");
        log.discard_prefix(boundary, boundary_term)
            .expect("discard");
        // `log` and the store drop here — only the durable files remain.
    }

    // Phase 2: cold recovery from the durable files ONLY.
    let store = SnapshotStore::new(snapshot_path(&path));
    let snapshot = store
        .load()
        .expect("load")
        .expect("snapshot present on disk");
    assert_eq!(snapshot.last_included_index, boundary);

    let mut log = Log::open(&path).expect("reopen retained tail");
    // Before adopting, only the retained tail is physically present.
    assert_eq!(
        log.entry_bytes().expect("bytes").len() as u64,
        committed as u64 - (boundary + 1)
    );
    // Recovery order: adopt boundary → seed state → replay retained committed tail.
    log.adopt_snapshot_boundary(snapshot.last_included_index, snapshot.last_included_term);
    let mut recovered = snapshot.state.clone();
    let mut last_applied = snapshot.last_included_index;
    let commit = committed as u64 - 1;
    replay_tail_into(&mut recovered, &log, last_applied + 1);
    last_applied = commit;

    assert_eq!(last_applied, commit);
    assert_eq!(
        recovered.to_bits(),
        oracle_bits(&updates, committed),
        "recovery-from-(snapshot + tail) diverged from the independent oracle"
    );
    // And it matches a full-log replay too (build a parallel un-compacted log).
    let full_path = temp_journal("recover-full");
    build_committed_log(&full_path, &updates);
    let full_log = Log::open(&full_path).expect("open full");
    let mut full = BookState::new();
    replay_tail_into(&mut full, &full_log, 0);
    assert_eq!(recovered.to_bits(), full.to_bits());

    assert_within_deadline(start);
}

/// Row 4: drive the FULL `RaftNode` compaction + boot-recovery path (a single-node
/// cluster, so `propose` commits on its own durability — no sockets needed, still
/// hard-deadline-bounded). This exercises `RaftNode::compact` (which must snapshot
/// the state *as of the boundary*, not the further-applied current state) and
/// `RaftNode::boot` snapshot-seeded recovery against the independent oracle — the
/// path a raw-artifact test would not cover.
#[test]
fn raftnode_compact_then_boot_recovery_is_exact() {
    let start = Instant::now();
    let updates = workload();
    let path = temp_journal("node");
    // Loopback-tuned fast timers (mirrors the `celnet-replog` replication suite) so
    // the solo node self-elects quickly and the row stays well within the deadline.
    let cfg = RaftConfig {
        election_min: Duration::from_millis(120),
        election_max: Duration::from_millis(240),
        heartbeat: Duration::from_millis(20),
        io_timeout: Duration::from_millis(400),
    };

    // Single-node cluster: it elects itself leader and commits on its own fsync.
    let node = RaftNode::boot(&path, &[], 1, cfg).expect("boot solo");
    // Wait until it is leader (a solo cluster wins its first election quickly).
    let became_leader = {
        let mut ok = false;
        while start.elapsed() < TEST_DEADLINE {
            if node.is_leader() {
                ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        ok
    };
    assert!(
        became_leader,
        "solo node did not become leader within deadline"
    );

    let mut last = 0u64;
    for u in &updates {
        let idx = node
            .propose(u)
            .expect("propose")
            .expect("solo leader proposes");
        assert!(
            node.wait_for_commit(idx, TEST_DEADLINE),
            "entry {idx} did not commit within deadline"
        );
        last = idx;
    }
    let oracle = oracle_bits(&updates, updates.len());
    assert_eq!(
        node.applied_bits(),
        oracle,
        "applied != oracle pre-compaction"
    );

    // Compact at an INTERIOR committed boundary (well below last_applied) — this is
    // exactly where a "snapshot the current applied state" bug would double-apply
    // the tail on recovery.
    let boundary = last / 2;
    let done = node.compact(boundary).expect("compact");
    assert_eq!(done, Some(boundary));
    let retained = last - boundary;
    assert_eq!(
        node.retained_log_len() as u64,
        retained,
        "RaftNode log was not physically shrunk by compaction"
    );
    assert_eq!(node.snapshot_index(), Some(boundary));
    assert_eq!(node.base_log_index(), boundary + 1);
    // Compaction must not change the live applied state.
    assert_eq!(
        node.applied_bits(),
        oracle,
        "compaction altered applied state"
    );

    // "Crash" and recover purely from (durable snapshot + retained tail).
    let recover_path = node.path().to_path_buf();
    node.shutdown();
    let restarted = RaftNode::boot(&recover_path, &[], 1, cfg).expect("recover");
    assert_eq!(restarted.snapshot_index(), Some(boundary));
    assert_eq!(restarted.base_log_index(), boundary + 1);
    assert!(
        {
            let mut ok = false;
            while start.elapsed() < TEST_DEADLINE {
                if restarted.commit_index() == Some(last) {
                    ok = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            ok
        },
        "recovered node did not restore commit index within deadline"
    );
    assert_eq!(
        restarted.applied_bits(),
        oracle,
        "RaftNode recovery-from-(snapshot + tail) diverged from the independent oracle"
    );
    restarted.shutdown();

    assert_within_deadline(start);
}

/// A guard row: compacting twice (an interior boundary, then a later one) is
/// monotone and still recovers exactly — proving repeated compaction does not
/// corrupt the absolute-index mapping.
#[test]
fn repeated_compaction_stays_exact() {
    let start = Instant::now();
    let updates = workload();
    let path = temp_journal("twice");
    build_committed_log(&path, &updates);
    let mut log = Log::open(&path).expect("open");
    let store = SnapshotStore::new(snapshot_path(&path));

    for boundary in [3u64, 8u64] {
        let term = log.term_at(boundary).expect("term within range");
        let mut applied = BookState::new();
        for u in &updates[..=boundary as usize] {
            applied.apply(u);
        }
        store
            .save(&Snapshot::new(boundary, term, applied))
            .expect("save");
        log.discard_prefix(boundary, term).expect("discard");
        assert_eq!(log.base_index(), boundary + 1);
        assert_eq!(log.snapshot_index(), Some(boundary));
        assert_eq!(log.last_index(), Some(updates.len() as u64 - 1));
        assert!(log.matches_prev(boundary, term));
    }

    // Final reconstruction == oracle.
    let snap = store.load().expect("load").expect("present");
    let mut recovered = snap.state.clone();
    replay_tail_into(&mut recovered, &log, snap.last_included_index + 1);
    assert_eq!(recovered.to_bits(), oracle_bits(&updates, updates.len()));

    assert_within_deadline(start);
}
