//! **Leader-replicated log — real loopback-socket multi-node correctness.**
//!
//! Every test boots ≥3 logical nodes as a leader + followers, each a real
//! `celnet_journal::Journal`-backed node, communicating over **real
//! `127.0.0.1` TCP sockets on ephemeral ports** (the followers bind listeners,
//! the leader dials them — see [`celnet_replog::Follower::boot`] /
//! [`celnet_replog::Leader::boot`]). No shared-memory fake stands in for the
//! network. Each test body is hard wall-clock bounded so a regression fails fast.
//!
//! Gates proven here:
//!
//! * (a) **kill-leader → surviving follower has the byte-identical committed log
//!   AND replays to bit-identical (`to_bits`) state** — the f64 oracle.
//! * (b) **quorum-commit correctness**: committed iff a majority durably holds it;
//!   a stalled minority never advances the commit index (no false progress).
//! * (c) **hot-standby takeover** within a bounded deadline, zero committed loss.
//! * (d) **crash-recovery**: a node restarted from its journal recovers the exact
//!   committed state, bit-identically (reusing journal torn-tail recovery).

mod common;

use std::time::Instant;

use celnet_replog::{
    BookState, BookUpdate, Follower, Leader, durable_entry_bytes, is_caught_up, promote,
};

use common::{IO_TIMEOUT, assert_within_deadline, temp_journal, wait_until};

/// A deterministic stream of book updates — `Set`s and `Add`s with values whose
/// exact bits we will compare cross-node. Includes a `0.1 + 0.2`-style add to
/// make the point that the *bits* (not a rounded decimal) replicate identically.
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

/// Boot a 3-node cluster (1 leader + 2 followers) over real loopback sockets and
/// return the leader plus the two followers.
fn boot_three() -> (Leader, Follower, Follower) {
    let f1 = Follower::boot(temp_journal("f1")).expect("follower 1 boots on ephemeral port");
    let f2 = Follower::boot(temp_journal("f2")).expect("follower 2 boots on ephemeral port");
    let leader = Leader::boot(
        temp_journal("leader"),
        1,
        &[f1.addr(), f2.addr()],
        3,
        IO_TIMEOUT,
    )
    .expect("leader boots and dials followers over loopback");
    (leader, f1, f2)
}

// ---------------------------------------------------------------------------
// Gate (a): kill-leader → byte-identical committed log + to_bits-identical state
// ---------------------------------------------------------------------------

#[test]
fn gate_a_kill_leader_follower_has_byte_identical_log_and_bit_identical_state() {
    let start = Instant::now();
    let (mut leader, f1, f2) = boot_three();
    let updates = workload();

    // Replicate the whole workload; every propose reaches a 2/3 majority.
    for u in &updates {
        let out = leader.propose(u).expect("propose");
        assert!(
            out.committed,
            "entry {} should commit on 2/3 quorum",
            out.index
        );
        assert_within_deadline(start);
    }
    let committed = leader.commit_index().expect("something committed");
    assert_eq!(committed, (updates.len() - 1) as u64);

    // Wait until both followers have applied through the committed index.
    let want = committed;
    assert!(
        wait_until(start, || f1.shared().commit_index() == Some(want)
            && f2.shared().commit_index() == Some(want)),
        "followers did not converge to committed index within deadline"
    );

    // The leader's durable log bytes...
    let leader_log = leader.journal_bytes().expect("leader log bytes");
    // "Kill" the leader: drop it (closes its sockets, ends its thread context).
    drop(leader);

    // A surviving follower's durable committed log is BYTE-IDENTICAL.
    let f1_log = f1.journal_bytes().expect("f1 log bytes");
    let f2_log = f2.journal_bytes().expect("f2 log bytes");
    assert_eq!(
        leader_log, f1_log,
        "follower-1 log not byte-identical to leader"
    );
    assert_eq!(
        leader_log, f2_log,
        "follower-2 log not byte-identical to leader"
    );

    // And it replays to BIT-IDENTICAL state (f64 to_bits oracle).
    let oracle = oracle_bits(&updates, updates.len());
    assert_eq!(
        f1.shared().applied_bits(),
        oracle,
        "f1 state not to_bits-identical"
    );
    assert_eq!(
        f2.shared().applied_bits(),
        oracle,
        "f2 state not to_bits-identical"
    );

    // Re-derive bit-identity straight off f1's durable journal (independent of
    // the live applied cache) to prove the on-disk log alone reconstructs it.
    let replayed = replay_log_to_state(&f1_log);
    assert_eq!(
        replayed.to_bits(),
        oracle,
        "f1 journal replay not to_bits-identical"
    );

    assert_within_deadline(start);
    f1.shutdown();
    f2.shutdown();
}

// ---------------------------------------------------------------------------
// Gate (b): quorum-commit correctness — no false progress on lost quorum
// ---------------------------------------------------------------------------

#[test]
fn gate_b_lost_quorum_makes_no_false_progress() {
    let start = Instant::now();
    let (mut leader, f1, f2) = boot_three();
    let updates = workload();

    // First, commit two entries with full 3-node quorum.
    for u in &updates[..2] {
        assert!(leader.propose(u).expect("propose").committed);
    }
    let committed_before = leader.commit_index();
    assert_eq!(committed_before, Some(1));

    // Now partition the cluster down to a MINORITY: kill BOTH followers. The
    // leader alone is 1 of 3 — strictly less than the majority (2). Subsequent
    // proposes are durable on the leader but MUST NOT commit (no false progress).
    f1.shutdown();
    f2.shutdown();

    let mut any_committed = false;
    for u in &updates[2..5] {
        let out = leader.propose(u).expect("propose under minority");
        any_committed |= out.committed;
        assert_within_deadline(start);
    }
    assert!(
        !any_committed,
        "a minority leader falsely advanced the commit index"
    );
    // The commit index is UNCHANGED from the pre-partition value.
    assert_eq!(
        leader.commit_index(),
        committed_before,
        "commit index advanced without a durable majority"
    );
    // The leader's applied state only reflects the truly-committed prefix.
    let oracle = oracle_bits(&updates, 2);
    assert_eq!(
        leader.applied_bits(),
        oracle,
        "applied state included uncommitted entries"
    );

    assert_within_deadline(start);
}

#[test]
fn gate_b_bare_majority_commits_minority_does_not() {
    let start = Instant::now();
    // 3 nodes: leader + 2 followers. Kill ONE follower → leader + 1 follower = 2
    // of 3 = a bare majority → still commits. This is the boundary of gate (b).
    let (mut leader, f1, f2) = boot_three();
    let updates = workload();

    f2.shutdown(); // down to leader + f1 = exactly majority

    for u in &updates[..3] {
        let out = leader.propose(u).expect("propose with bare majority");
        assert!(
            out.committed,
            "bare 2/3 majority should commit index {}",
            out.index
        );
        assert_within_deadline(start);
    }
    assert_eq!(leader.commit_index(), Some(2));

    // The surviving follower converged to the committed, to_bits-identical state.
    let want = 2u64;
    assert!(wait_until(start, || f1.shared().commit_index() == Some(want)));
    assert_eq!(f1.shared().applied_bits(), oracle_bits(&updates, 3));

    assert_within_deadline(start);
    f1.shutdown();
}

// ---------------------------------------------------------------------------
// Gate (c): hot-standby takeover, bounded, zero committed-entry loss
// ---------------------------------------------------------------------------

#[test]
fn gate_c_hot_standby_takeover_loses_no_committed_entries() {
    let start = Instant::now();
    // 5-node cluster: leader + 4 followers. We will promote follower f1 as the
    // hot standby after the original leader is killed; the 3 remaining followers
    // (f2,f3,f4) are its surviving quorum (new cluster size 4 → majority 3).
    let f1 = Follower::boot(temp_journal("s-f1")).unwrap();
    let f2 = Follower::boot(temp_journal("s-f2")).unwrap();
    let f3 = Follower::boot(temp_journal("s-f3")).unwrap();
    let f4 = Follower::boot(temp_journal("s-f4")).unwrap();
    let mut leader = Leader::boot(
        temp_journal("s-leader"),
        1,
        &[f1.addr(), f2.addr(), f3.addr(), f4.addr()],
        5,
        IO_TIMEOUT,
    )
    .unwrap();

    let updates = workload();
    for u in &updates {
        assert!(leader.propose(u).expect("propose").committed);
        assert_within_deadline(start);
    }
    let committed = leader.commit_index().unwrap();
    assert_eq!(committed, (updates.len() - 1) as u64);

    // The hot standby (f1) must be caught up to the committed index before we
    // promote it — this is the pre-warm gate.
    assert!(wait_until(start, || f1.shared().commit_index() == Some(committed)));
    assert!(is_caught_up(f1.shared().match_index(), Some(committed)));

    let leader_log = leader.journal_bytes().unwrap();
    let f1_path = f1.path().to_path_buf();
    let old_term = leader.term();

    // KILL the leader. Then promote f1's *standby state* — but f1 is itself a
    // running follower; to promote we take over its journal. Stop f1's serve
    // loop first so the journal has a single writer, then promote from its path.
    drop(leader);
    f1.shutdown();

    // Surviving cluster: the new leader (promoted from f1's journal) + f2,f3,f4.
    let t0 = Instant::now();
    let mut new_leader = promote(
        &f1_path,
        old_term,
        &[f2.addr(), f3.addr(), f4.addr()],
        4,
        IO_TIMEOUT,
    )
    .expect("standby promotes to leader");
    let takeover = t0.elapsed();
    // Bounded takeover.
    assert!(
        takeover < common::TEST_DEADLINE,
        "standby takeover exceeded the bounded deadline"
    );

    // ZERO committed-entry loss: the promoted leader's durable log is byte-identical
    // to the dead leader's committed log, and its applied state is to_bits-identical.
    assert_eq!(
        new_leader.journal_bytes().unwrap(),
        leader_log,
        "promoted leader lost committed log bytes"
    );
    let oracle = oracle_bits(&updates, updates.len());
    assert_eq!(
        new_leader.applied_bits(),
        oracle,
        "promoted leader state diverged"
    );
    // The new leader runs in a higher term (fences the old leader).
    assert_eq!(new_leader.term(), old_term + 1);

    // And it can keep serving: a fresh proposal commits on the new quorum and
    // replicates to the survivors, to_bits-identically.
    let extra = BookUpdate::Set {
        key: 99,
        value: 1.234_567_890_123_456_7,
    };
    let out = new_leader.propose(&extra).expect("post-promotion propose");
    assert!(
        out.committed,
        "new leader cannot make progress on survivors"
    );
    let mut want_state = BookState::new();
    for u in &updates {
        want_state.apply(u);
    }
    want_state.apply(&extra);
    let want_bits = want_state.to_bits();
    let new_committed = new_leader.commit_index().unwrap();
    assert!(wait_until(start, || f2.shared().commit_index()
        == Some(new_committed)
        && f3.shared().commit_index() == Some(new_committed)));
    assert_eq!(f2.shared().applied_bits(), want_bits);
    assert_eq!(f3.shared().applied_bits(), want_bits);

    assert_within_deadline(start);
    f2.shutdown();
    f3.shutdown();
    f4.shutdown();
}

// ---------------------------------------------------------------------------
// Gate (d): crash-recovery — restart from journal recovers exact committed state
// ---------------------------------------------------------------------------

#[test]
fn gate_d_crash_recovery_rebuilds_exact_committed_state() {
    let start = Instant::now();
    let (mut leader, f1, f2) = boot_three();
    let updates = workload();
    for u in &updates {
        assert!(leader.propose(u).expect("propose").committed);
    }
    let committed = leader.commit_index().unwrap();
    assert!(wait_until(start, || f2.shared().commit_index() == Some(committed)));

    let f2_path = f2.path().to_path_buf();
    let pre_crash_log = f2.journal_bytes().unwrap();
    let oracle = oracle_bits(&updates, updates.len());
    assert_eq!(f2.shared().applied_bits(), oracle);

    // "Crash" f2 (drop the running node — its journal is on disk, fsync'd).
    drop(leader);
    f1.shutdown();
    f2.shutdown();

    // Restart f2 PURELY from its journal: boot recovers durable state.
    let f2_restarted = Follower::boot(&f2_path).expect("f2 recovers from journal");
    // Byte-identical durable log survived the crash.
    assert_eq!(f2_restarted.journal_bytes().unwrap(), pre_crash_log);
    // And it recovered the EXACT committed state, bit-identically.
    assert_eq!(
        f2_restarted.shared().applied_bits(),
        oracle,
        "crash-recovered state not to_bits-identical"
    );
    assert_eq!(f2_restarted.shared().commit_index(), Some(committed));

    assert_within_deadline(start);
    f2_restarted.shutdown();
}

// ---------------------------------------------------------------------------
// Helper: replay raw durable entry bytes to a BookState (independent oracle path)
// ---------------------------------------------------------------------------

/// Reconstruct a [`BookState`] purely from a node's durable entry bytes (each is
/// an encoded `LogEntry` whose payload is an encoded `BookUpdate`) — proving the
/// on-disk log alone determines the bit-identical state.
fn replay_log_to_state(entry_bytes: &[Vec<u8>]) -> BookState {
    use celnet_replog::LogEntry;
    let mut s = BookState::new();
    for bytes in entry_bytes {
        let entry = LogEntry::decode(bytes).expect("durable entry decodes");
        let upd = BookUpdate::decode(&entry.payload).expect("payload decodes");
        s.apply(&upd);
    }
    s
}

// ---------------------------------------------------------------------------
// A direct durable-bytes cross-check helper sanity test (uses the public
// `durable_entry_bytes` re-export against a follower's path).
// ---------------------------------------------------------------------------

#[test]
fn durable_bytes_helper_matches_follower_journal() {
    let start = Instant::now();
    let (mut leader, f1, f2) = boot_three();
    for u in workload().iter().take(3) {
        assert!(leader.propose(u).expect("propose").committed);
    }
    assert!(wait_until(start, || f1.shared().match_index() == Some(2)));
    let via_helper = durable_entry_bytes(f1.path()).unwrap();
    let via_method = f1.journal_bytes().unwrap();
    assert_eq!(via_helper, via_method);
    drop(leader);
    f1.shutdown();
    f2.shutdown();
    assert_within_deadline(start);
}
