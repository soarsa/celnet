//! The Raft consensus role state machine: a single cohesive [`RaftNode`] that is
//! a **Follower**, **Candidate**, or **Leader**, driving the real-socket
//! [`crate::wire`] transport over the durable [`crate::log::Log`] and the
//! [`crate::state::BookState`] state machine, with [`crate::persist`] holding the
//! durable `current_term`/`voted_for`.
//!
//! Provenance: this implements the consensus algorithm of Ongaro & Ousterhout
//! ("In Search of an Understandable Consensus Algorithm", USENIX ATC 2014) —
//! commonly "Raft". Per the naming guardrail, public identifiers are
//! purpose-named (e.g. [`RaftNode`], `Role`); the provenance lives only here in
//! doc comments.
//!
//! # Roles and transitions
//!
//! * **Follower** — passive. Resets its randomized election timer on a valid
//!   [`Message::AppendEntries`] (incl. heartbeats) from the current-term leader,
//!   or on granting a vote. On timeout it becomes a Candidate.
//! * **Candidate** — first runs a **Pre-Vote** straw poll at its *hypothetical*
//!   next term (no persistent change); only if a strict majority would grant does it
//!   increment `current_term`, vote for itself (durably), reset the timer, and send
//!   real [`Message::RequestVote`]s. A strict majority of real grants → Leader. A
//!   higher term observed → step down. A timeout with no winner → a new attempt.
//!   Pre-Vote (Ongaro thesis §9.6) stops a flaky/partitioned node from disrupting a
//!   healthy leader by inflating the term.
//! * **Leader** — sends periodic [`Message::AppendEntries`] (heartbeats + real
//!   entries), maintains `next_index`/`match_index` per peer, advances
//!   `commit_index` per the §5.4.2 rule (a majority `match_index >= N` **and**
//!   `log[N].term == current_term`), and applies committed entries in order.
//!   A higher term observed → step down.
//!
//! # Concurrency model (and how we never deadlock on blocking IO)
//!
//! All mutable node state lives behind one [`Mutex<NodeCore>`]. A **serve thread**
//! accepts peer connections and answers inbound RPCs; a **tick thread** drives the
//! election timer and (when leader) replication. The hard rule: **the core mutex
//! is never held across a blocking socket call.** The tick thread *snapshots* the
//! work to do under the lock, releases it, performs the blocking RPC, then
//! re-acquires the lock to fold in the reply. The serve thread holds the lock only
//! for the short, non-blocking state mutation of handling one decoded frame. Every
//! socket carries the cluster `io_timeout`, so a dead peer fails fast — no call
//! can hang, which is what keeps every test deadline-bounded.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::compaction::{Snapshot, SnapshotStore, snapshot_path};
use crate::entry::LogEntry;
use crate::log::{EMPTY_PREV, Log};
use crate::persist::PersistStore;
use crate::state::{BookState, BookUpdate};
use crate::wire::{EMPTY_LOG, FrameRead, Message, read_frame, read_frame_or_idle, write_frame};

/// The role a node currently occupies in the consensus state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Passive replica, awaiting a leader's AppendEntries.
    Follower,
    /// Soliciting votes for a new term.
    Candidate,
    /// The elected coordinator replicating its log.
    Leader,
}

/// Timing parameters for the consensus protocol. The election-timeout *range*
/// must be comfortably larger than the heartbeat interval (so a live leader keeps
/// followers from timing out) and randomized per node (so split votes resolve).
#[derive(Debug, Clone, Copy)]
pub struct RaftConfig {
    /// Lower bound of the randomized election timeout.
    pub election_min: Duration,
    /// Upper bound of the randomized election timeout.
    pub election_max: Duration,
    /// Leader heartbeat / replication interval.
    pub heartbeat: Duration,
    /// Per-RPC socket deadline so a dead peer fails fast (never hangs).
    pub io_timeout: Duration,
}

impl Default for RaftConfig {
    fn default() -> Self {
        // The election timeout is ~10–20× the heartbeat with a wide random spread —
        // the standard Raft ratio — so an occasional stall (notably a durable
        // `fsync` taking tens of ms while the core lock is held, which briefly delays
        // the next heartbeat) does not trip a follower into a spurious election, yet
        // a genuinely dead leader is still detected within ~half a second. These are
        // loopback/local-disk-tuned; a cross-host deploy would widen them further.
        Self {
            election_min: Duration::from_millis(400),
            election_max: Duration::from_millis(800),
            heartbeat: Duration::from_millis(40),
            io_timeout: Duration::from_secs(2),
        }
    }
}

/// One peer's address plus the leader's view of its replication progress.
struct PeerState {
    addr: SocketAddr,
    /// Index of the next log entry to send to this peer (leader state, §5.3).
    next_index: u64,
    /// Highest index known replicated on this peer (leader state).
    match_index: Option<u64>,
}

/// All mutable node state, guarded by one mutex.
struct NodeCore {
    /// This node's stable id (its listen port) — used as `candidate_id`.
    id: u64,
    role: Role,
    persist: PersistStore,
    log: Log,
    /// The durable snapshot store (the `<journal>.snapshot` sibling file). Holds
    /// the applied state captured at the most recent compaction boundary.
    snapshots: SnapshotStore,
    /// Applied committed state machine (the bit-identity oracle target).
    applied: BookState,
    /// Highest index known committed, or `None`.
    commit_index: Option<u64>,
    /// Highest index applied to `applied`, or `None`.
    last_applied: Option<u64>,
    /// Total cluster size (for the strict-majority threshold). Fixed membership.
    cluster_size: usize,
    /// Per-peer replication state, keyed by peer id (its port).
    peers: HashMap<u64, PeerState>,
    /// When the election timer currently fires (Instant). Reset on valid leader
    /// contact or on granting a vote; consulted by the tick thread.
    election_deadline: Instant,
    /// Votes gathered while a Candidate in the current term (includes self).
    votes_for_me: usize,
    /// Instant of the last valid contact from a current-term leader (an
    /// AppendEntries, incl. a heartbeat). Used by the Pre-Vote rule: a node only
    /// grants a pre-vote if it has NOT heard from a leader within the minimum
    /// election timeout — so a flaky node cannot disrupt a healthy leader.
    last_leader_contact: Instant,
    cfg: RaftConfig,
}

impl NodeCore {
    fn current_term(&self) -> u64 {
        self.persist.current_term()
    }

    /// The §5.4.1 "candidate's log is at least as up-to-date as mine" test:
    /// higher last-term wins; on equal last-term, the longer log wins.
    fn candidate_log_ok(&self, cand_last_index: u64, cand_last_term: u64) -> bool {
        let my_last_term = self.log.last_term();
        let my_last_index = self.log.last_index();
        if cand_last_term != my_last_term {
            return cand_last_term > my_last_term;
        }
        // Equal terms: compare lengths. EMPTY_LOG sentinel is "no entry" → -1.
        let cand_len = if cand_last_index == EMPTY_LOG {
            0u128
        } else {
            cand_last_index as u128 + 1
        };
        let my_len = my_last_index.map_or(0u128, |i| i as u128 + 1);
        cand_len >= my_len
    }

    /// Step down to follower in `term` (a strictly higher term observed). Durably
    /// records the new term and clears the vote. Resets the election timer.
    ///
    /// # Errors
    ///
    /// Propagates the persistent-state IO error.
    fn step_down(&mut self, term: u64) -> std::io::Result<()> {
        self.persist.save(term, None)?;
        self.role = Role::Follower;
        self.votes_for_me = 0;
        self.reset_election_timer();
        Ok(())
    }

    /// Reset the randomized election timer (next deadline = now + U[min,max]).
    fn reset_election_timer(&mut self) {
        let span = self
            .cfg
            .election_max
            .saturating_sub(self.cfg.election_min)
            .as_nanos()
            .max(1);
        let jitter = next_jitter(self.id) % (span as u64);
        self.election_deadline =
            Instant::now() + self.cfg.election_min + Duration::from_nanos(jitter);
    }

    /// Advance the commit index to `new_commit` (if it is strictly higher),
    /// **durably persist the watermark** (so a committed entry survives a crash and
    /// is never truncated), and apply the newly-committed entries in order.
    fn set_commit_index(&mut self, new_commit: Option<u64>) {
        let advance = match (self.commit_index, new_commit) {
            (Some(c), Some(n)) => n > c,
            (None, Some(_)) => true,
            (_, None) => false,
        };
        if !advance {
            return;
        }
        self.commit_index = new_commit;
        // Durable watermark first (a committed entry must never be lost), then apply.
        let _ = self.persist.save_commit_index(new_commit);
        self.apply_committed();
    }

    /// Apply newly-committed entries `(last_applied, commit_index]` in order.
    fn apply_committed(&mut self) {
        let Some(commit) = self.commit_index else {
            return;
        };
        let start = self.last_applied.map_or(0, |a| a + 1);
        if start > commit {
            return;
        }
        // Replay the committed slice straight from the durable log.
        if let Ok(entries) = self.log.entries_from(start) {
            for e in entries {
                if e.index > commit {
                    break;
                }
                if let Ok(upd) = BookUpdate::decode(&e.payload) {
                    self.applied.apply(&upd);
                }
                self.last_applied = Some(e.index);
            }
        }
    }

    /// Capture a durable snapshot of the applied state **as of `at_index`** and
    /// discard the now-redundant log prefix `[.., at_index]` (Raft §7 compaction).
    ///
    /// `at_index` must be **committed and applied** (it is clamped to
    /// `last_applied`); because `last_applied <= commit_index`, the snapshot only
    /// ever subsumes committed entries — an uncommitted entry is never discarded,
    /// and a committed entry it captures is never lost (the snapshot is durably
    /// written before the prefix is discarded).
    ///
    /// The captured state is the state machine **exactly through `boundary`**, not
    /// the node's current (possibly further-applied) state — it is reconstructed by
    /// replaying the committed prefix `[base_index, boundary]` from the retained
    /// durable log on top of the previous snapshot's state (if any). This is what
    /// lets recovery seed from the snapshot and then replay only the *tail*
    /// `(boundary, ..]` without double-applying entries `(boundary, last_applied]`.
    ///
    /// The snapshot is written FIRST (durably), THEN the prefix is discarded, so a
    /// crash between the two leaves the (redundant) prefix on disk to be recovered
    /// harmlessly. A request at or below an existing snapshot boundary is a no-op.
    ///
    /// Returns the boundary index actually snapshotted, or `None` if there was
    /// nothing new to compact. Errors propagate the durable IO failure.
    fn compact_to(&mut self, at_index: u64) -> std::io::Result<Option<u64>> {
        // Only compact committed+applied entries.
        let Some(applied_hw) = self.last_applied else {
            return Ok(None);
        };
        let boundary = at_index.min(applied_hw);
        // No-op if already snapshotted at/beyond this boundary.
        if self.log.snapshot_index().is_some_and(|s| s >= boundary) {
            return Ok(None);
        }
        // The boundary entry's term must be known (it is within the retained log,
        // or it IS the current snapshot boundary). A committed+applied index is
        // always within range, so this is always Some.
        let Some(boundary_term) = self.log.term_at(boundary) else {
            return Ok(None);
        };

        // Reconstruct the state machine AS OF `boundary` (not the current applied
        // state, which may include entries above `boundary`). Start from the prior
        // snapshot's state if one exists (its boundary is `base_index - 1`), then
        // replay the retained committed prefix up to and including `boundary`.
        let mut state_at_boundary = match self.snapshots.load()? {
            Some(prev) if Some(prev.last_included_index) == self.log.snapshot_index() => prev.state,
            _ => BookState::new(),
        };
        let replay_from = self.log.base_index();
        if replay_from <= boundary {
            for e in self.log.entries_from(replay_from)? {
                if e.index > boundary {
                    break;
                }
                if let Ok(upd) = BookUpdate::decode(&e.payload) {
                    state_at_boundary.apply(&upd);
                }
            }
        }

        // 1) Durably write the snapshot of the state AS OF `boundary`.
        let snap = Snapshot::new(boundary, boundary_term, state_at_boundary);
        self.snapshots.save(&snap)?;
        // 2) Discard the log prefix up to and including `boundary`.
        self.log.discard_prefix(boundary, boundary_term)?;
        Ok(Some(boundary))
    }

    /// Follower side of the InstallSnapshot RPC (Raft §7): durably install a
    /// snapshot the leader shipped because it had compacted past the entries this
    /// node needs. Returns the boundary index installed, or `None` if the install
    /// was stale (the node is already at or beyond `last_included_index`).
    ///
    /// The ordering mirrors local compaction's "never lose a committed entry" rule:
    /// the snapshot file is written durably FIRST, THEN the log is reshaped to the
    /// boundary, THEN the applied state machine + watermarks are reseeded. A crash
    /// between the snapshot write and the log reshape leaves the (redundant) old log
    /// on disk to be recovered harmlessly; a crash before the snapshot is durable
    /// reads the snapshot back as absent and the old log is replayed — either way
    /// the committed state is exact.
    ///
    /// On install the applied [`BookState`] is reseeded from the snapshot, and
    /// `last_applied`/`commit_index` are advanced to the boundary (clamped to never
    /// regress) — the snapshot *is* the applied state through `last_included_index`,
    /// so the follower is now caught up to that committed point and resumes normal
    /// AppendEntries from `last_included_index + 1`.
    ///
    /// # Errors
    ///
    /// Propagates the durable snapshot-write / log-reshape IO failure.
    fn install_snapshot(&mut self, snapshot: &Snapshot) -> std::io::Result<Option<u64>> {
        let lii = snapshot.last_included_index;
        let lit = snapshot.last_included_term;
        // Stale: we already hold this boundary or a later one (via our own snapshot
        // or simply a longer log that already covers it). A node that already has the
        // boundary entry committed+applied needs no install.
        if self.log.snapshot_index().is_some_and(|s| s >= lii) {
            return Ok(None);
        }
        if self.last_applied.is_some_and(|a| a >= lii) {
            return Ok(None);
        }

        // 1) Durably write the snapshot FIRST (the safe ordering).
        self.snapshots.save(snapshot)?;
        // 2) Reshape the durable log to the boundary (retain a matching tail, else
        //    discard the whole log) — really on disk, atomically.
        let advanced = self.log.install_snapshot(lii, lit)?;
        if !advanced {
            // The log already accounted for the boundary (matching tail discard was
            // a no-op); nothing further to reseed.
            return Ok(None);
        }
        // 3) Reseed the applied state machine + watermarks from the snapshot. The
        //    snapshot captures the applied state through `last_included_index`, so it
        //    becomes our applied state and our last_applied/commit at the boundary.
        self.applied = snapshot.state.clone();
        self.last_applied = Some(lii);
        // Advance the commit watermark to the boundary (monotone — never regress).
        let new_commit = match self.commit_index {
            Some(c) if c >= lii => Some(c),
            _ => Some(lii),
        };
        self.commit_index = new_commit;
        let _ = self.persist.save_commit_index(new_commit);
        // Any retained tail above the boundary that is now committed (≤ commit_index)
        // is applied in order on top of the seeded state.
        self.apply_committed();
        Ok(Some(lii))
    }

    /// Leader §5.4.2 commit advance: find the highest `N > commit_index` such that
    /// a strict majority of nodes (self + peers with `match_index >= N`) hold it
    /// **and** `log[N].term == current_term`, then advance and apply.
    fn leader_advance_commit(&mut self) {
        if self.role != Role::Leader {
            return;
        }
        let Some(last) = self.log.last_index() else {
            return;
        };
        let majority = self.cluster_size / 2 + 1;
        let current_term = self.current_term();
        let start = self.commit_index.map_or(0, |c| c + 1);
        let mut new_commit = self.commit_index;
        for n in start..=last {
            // Only directly commit entries from the leader's own current term.
            if self.log.term_at(n) != Some(current_term) {
                continue;
            }
            let mut holders = 1usize; // the leader holds it
            for p in self.peers.values() {
                if p.match_index.is_some_and(|m| m >= n) {
                    holders += 1;
                }
            }
            if holders >= majority {
                new_commit = Some(n);
            }
        }
        self.set_commit_index(new_commit);
    }
}

/// A running consensus node: a real loopback listener + a serve thread + a tick
/// thread, all sharing one [`NodeCore`].
pub struct RaftNode {
    id: u64,
    addr: SocketAddr,
    path: PathBuf,
    core: Arc<Mutex<NodeCore>>,
    /// Wakes the tick thread immediately (e.g. on a new proposal) and is the
    /// stop signal's notify channel.
    wake: Arc<(Mutex<bool>, Condvar)>,
    stop: Arc<AtomicBool>,
    serve_handle: Option<JoinHandle<()>>,
    tick_handle: Option<JoinHandle<()>>,
}

impl RaftNode {
    /// Boot a consensus node.
    ///
    /// `journal_path` is the durable log path; the persistent `current_term`/
    /// `voted_for` live in a sibling `<journal_path>.raft` file. The node binds a
    /// loopback listener on an **ephemeral** port (its id), recovers its durable
    /// log + persistent state, and starts as a **Follower**. `peer_addrs` are the
    /// other nodes' loopback addresses; `cluster_size` is the total node count
    /// (must be `1 + peer_addrs.len()` for a fixed-membership cluster).
    ///
    /// Because a node's ephemeral port is only known after it binds, but fixed
    /// membership requires every node to know all peer addresses at boot, callers
    /// that need the address before constructing the peer set should bind the
    /// listener themselves and use [`RaftNode::boot_on`].
    ///
    /// Peers are dialled lazily (a peer may not be listening yet at boot), so the
    /// cluster forms as nodes come up; an unreachable peer never blocks boot.
    ///
    /// # Errors
    ///
    /// Propagates journal-open, persistent-state, and socket-bind IO failures.
    pub fn boot(
        journal_path: impl Into<PathBuf>,
        peer_addrs: &[SocketAddr],
        cluster_size: usize,
        cfg: RaftConfig,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        Self::boot_on(listener, journal_path, peer_addrs, cluster_size, cfg)
    }

    /// Boot a node on an **already-bound** loopback listener.
    ///
    /// This is the address-stable constructor for forming a fully-meshed cluster:
    /// bind every node's listener first (so all addresses are known), then hand
    /// each pre-bound listener here together with the complete peer-address set.
    /// The node adopts the listener's port as its stable id.
    ///
    /// # Errors
    ///
    /// Propagates journal-open and persistent-state IO failures.
    pub fn boot_on(
        listener: TcpListener,
        journal_path: impl Into<PathBuf>,
        peer_addrs: &[SocketAddr],
        cluster_size: usize,
        cfg: RaftConfig,
    ) -> std::io::Result<Self> {
        assert!(
            cluster_size == 1 + peer_addrs.len(),
            "cluster_size must equal 1 (self) + the peer count"
        );
        let path = journal_path.into();
        let mut log = Log::open(&path)?;
        let persist = PersistStore::open(persist_path(&path))?;
        let snapshots = SnapshotStore::new(snapshot_path(&path));

        let addr = listener.local_addr()?;
        let id = u64::from(addr.port());

        // --- Snapshot-aware recovery (Raft §7). Recovery order:
        //   1. If a durable snapshot exists, SEED the applied state machine and
        //      the (last_applied, commit) watermarks from it FIRST, and adopt its
        //      `(last_included_index, last_included_term)` boundary on the log so
        //      the durable tail's absolute indices are correct over the already-
        //      discarded prefix.
        //   2. Replay ONLY the retained committed tail (entries strictly above the
        //      snapshot boundary, up to the durable commit watermark) on top of the
        //      seeded state. The result is `to_bits`-identical to a full-log replay.
        // The commit-index watermark is persisted (Raft §5 + our durable-commit
        // design), so a restarted node knows exactly which prefix is committed; only
        // the *uncommitted* tail above it is eligible for conflicting-tail
        // truncation. Entries above the watermark are NOT applied on boot; the
        // leader re-drives their commit via AppendEntries, and apply is monotone.
        let snapshot = snapshots.load()?;
        let mut applied = BookState::new();
        let mut last_applied = None;
        if let Some(snap) = &snapshot {
            log.adopt_snapshot_boundary(snap.last_included_index, snap.last_included_term);
            applied = snap.state.clone();
            last_applied = Some(snap.last_included_index);
        }

        // The committed high-water this node may safely apply: the durable watermark
        // clamped to what the log accounts for. With a snapshot, `log.last_index()`
        // already reflects the boundary, so a node whose tail is wholly subsumed
        // still has its full committed prefix applied (from the snapshot).
        let commit_index = match persist.commit_index() {
            // Clamp the durable watermark to the log high-water (None when there is
            // no snapshot and the log is empty — nothing committed can be applied).
            Some(c) => log.last_index().map(|last| c.min(last)),
            // No durable watermark, but a snapshot implies its boundary was once
            // committed — that prefix is captured and applied via the seed above.
            None => snapshot.as_ref().map(|s| s.last_included_index),
        };

        // Replay the retained committed tail above the seeded boundary.
        if let Some(commit) = commit_index {
            let start = last_applied.map_or(0, |a| a + 1);
            if start <= commit {
                for e in log.entries_from(start)? {
                    if e.index > commit {
                        break;
                    }
                    if let Ok(upd) = BookUpdate::decode(&e.payload) {
                        applied.apply(&upd);
                    }
                    last_applied = Some(e.index);
                }
            }
        }

        let mut peers = HashMap::new();
        for &a in peer_addrs {
            peers.insert(
                u64::from(a.port()),
                PeerState {
                    addr: a,
                    next_index: log.last_index().map_or(0, |i| i + 1),
                    match_index: None,
                },
            );
        }

        let mut core = NodeCore {
            id,
            role: Role::Follower,
            persist,
            log,
            snapshots,
            applied,
            commit_index,
            last_applied,
            cluster_size,
            peers,
            election_deadline: Instant::now(), // reset just below
            votes_for_me: 0,
            // Seed "last contact" in the past so a fresh cluster (no leader yet) is
            // immediately willing to grant pre-votes and elect a first leader.
            last_leader_contact: Instant::now()
                .checked_sub(cfg.election_max)
                .unwrap_or_else(Instant::now),
            cfg,
        };
        core.reset_election_timer();
        let core = Arc::new(Mutex::new(core));

        let wake = Arc::new((Mutex::new(false), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));

        // Serve thread: accept peer connections and answer inbound RPCs.
        let serve_core = Arc::clone(&core);
        let serve_stop = Arc::clone(&stop);
        let serve_handle = thread::Builder::new()
            .name(format!("raft-serve-{}", addr.port()))
            .spawn(move || serve_loop(listener, serve_core, serve_stop))
            .expect("spawn serve thread");

        // Tick thread: election timer + leader replication.
        let tick_core = Arc::clone(&core);
        let tick_stop = Arc::clone(&stop);
        let tick_wake = Arc::clone(&wake);
        let tick_handle = thread::Builder::new()
            .name(format!("raft-tick-{}", addr.port()))
            .spawn(move || tick_loop(tick_core, tick_stop, tick_wake))
            .expect("spawn tick thread");

        Ok(Self {
            id,
            addr,
            path,
            core,
            wake,
            stop,
            serve_handle: Some(serve_handle),
            tick_handle: Some(tick_handle),
        })
    }

    /// The node's stable id (its listen port).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The loopback address peers dial.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The durable journal path (for restart / inspection).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The node's current role.
    #[must_use]
    pub fn role(&self) -> Role {
        self.core.lock().expect("core lock").role
    }

    /// Whether this node currently believes itself the leader.
    #[must_use]
    pub fn is_leader(&self) -> bool {
        self.role() == Role::Leader
    }

    /// The node's current term.
    #[must_use]
    pub fn term(&self) -> u64 {
        self.core.lock().expect("core lock").current_term()
    }

    /// The highest committed index, or `None`.
    #[must_use]
    pub fn commit_index(&self) -> Option<u64> {
        self.core.lock().expect("core lock").commit_index
    }

    /// The durable high-water (last log) index, or `None`.
    #[must_use]
    pub fn last_log_index(&self) -> Option<u64> {
        self.core.lock().expect("core lock").log.last_index()
    }

    /// A snapshot clone of the applied committed [`BookState`].
    #[must_use]
    pub fn applied_state(&self) -> BookState {
        self.core.lock().expect("core lock").applied.clone()
    }

    /// The `to_bits` digest of the applied committed state — the cross-node
    /// bit-identity oracle.
    #[must_use]
    pub fn applied_bits(&self) -> Vec<(u64, u64)> {
        self.core.lock().expect("core lock").applied.to_bits()
    }

    /// The encoded entry bytes of this node's durable log (byte-comparison).
    ///
    /// # Errors
    ///
    /// Propagates journal IO errors.
    pub fn log_bytes(&self) -> std::io::Result<Vec<Vec<u8>>> {
        self.core.lock().expect("core lock").log.entry_bytes()
    }

    /// Propose a [`BookUpdate`] through the leader. Returns the assigned index, or
    /// `None` if this node is **not** the leader (the caller should retry on the
    /// current leader). The proposal is durably appended locally and the tick
    /// thread is woken to replicate it immediately; commitment is asynchronous —
    /// poll [`RaftNode::commit_index`] / [`RaftNode::wait_for_commit`].
    ///
    /// # Errors
    ///
    /// Propagates a leader-local durable-append IO failure.
    pub fn propose(&self, update: &BookUpdate) -> std::io::Result<Option<u64>> {
        let index = {
            let mut core = self.core.lock().expect("core lock");
            if core.role != Role::Leader {
                return Ok(None);
            }
            let index = core.log.last_index().map_or(0, |i| i + 1);
            let term = core.current_term();
            let entry = LogEntry::new(term, index, update.encode());
            core.log.append(&entry)?;
            // A single-node cluster commits immediately on its own durability.
            if core.cluster_size == 1 {
                core.leader_advance_commit();
            }
            index
        };
        self.signal_wake();
        Ok(Some(index))
    }

    /// Block until `index` is committed (and applied) on this node, or the
    /// `deadline` elapses. Returns whether it committed in time. Polls without
    /// holding the core lock across the wait.
    #[must_use]
    pub fn wait_for_commit(&self, index: u64, deadline: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if self.commit_index().is_some_and(|c| c >= index) {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        self.commit_index().is_some_and(|c| c >= index)
    }

    /// Compact the durable log: capture a snapshot of the applied state at
    /// `at_index` (clamped to `last_applied`) and discard the prefix `[..,
    /// at_index]` (Raft §7). Returns the boundary index actually snapshotted, or
    /// `None` if nothing new was compacted (e.g. nothing applied yet, or already
    /// compacted at/beyond `at_index`).
    ///
    /// Safe to call on any node (leader or follower) — it compacts only this
    /// node's own committed+applied prefix and never affects consensus safety. A
    /// leader that compacts past a far-behind follower's replicated point now
    /// catches that follower up automatically via the **InstallSnapshot RPC** (Raft
    /// §7 — built; see the crate root): the next replication round ships the durable
    /// snapshot instead of the (discarded) entries. [`RaftNode::safe_compact_index`]
    /// remains available as a *conservative* boundary (compact no higher than the
    /// slowest follower) for operators who prefer to avoid the snapshot transfer
    /// entirely, but it is no longer required for correctness.
    ///
    /// # Errors
    ///
    /// Propagates the durable snapshot-write / prefix-discard IO failure.
    pub fn compact(&self, at_index: u64) -> std::io::Result<Option<u64>> {
        let mut core = self.core.lock().expect("core lock");
        core.compact_to(at_index)
    }

    /// Compact this node's log up to its current `last_applied` (the full
    /// committed+applied prefix). Convenience over [`RaftNode::compact`].
    ///
    /// # Errors
    ///
    /// Propagates the durable IO failure.
    pub fn compact_applied(&self) -> std::io::Result<Option<u64>> {
        let at = {
            let core = self.core.lock().expect("core lock");
            core.last_applied
        };
        match at {
            Some(at) => self.compact(at),
            None => Ok(None),
        }
    }

    /// The highest index it is **safe to compact past** without a far-behind
    /// follower needing a discarded entry: the minimum of this node's
    /// `last_applied` and every peer's known `match_index`. With the
    /// **InstallSnapshot RPC** now built (see the crate root), this is no longer
    /// required for correctness — a leader that compacts past a lagging follower
    /// catches it up automatically by shipping the snapshot. It is retained as the
    /// *conservative* operational choice for an operator who would rather never
    /// trigger a snapshot transfer (e.g. to bound steady-state network cost).
    /// Returns `None` if any peer's progress is still unknown (be conservative — do
    /// not compact) or nothing is applied yet.
    #[must_use]
    pub fn safe_compact_index(&self) -> Option<u64> {
        let core = self.core.lock().expect("core lock");
        let mut floor = core.last_applied?;
        for p in core.peers.values() {
            floor = floor.min(p.match_index?);
        }
        Some(floor)
    }

    /// The boundary index of this node's most recent durable snapshot, or `None`
    /// if no prefix has been discarded.
    #[must_use]
    pub fn snapshot_index(&self) -> Option<u64> {
        self.core.lock().expect("core lock").log.snapshot_index()
    }

    /// The absolute index of physical log position 0 — `last_included_index + 1`
    /// after a compaction, else `0`. The count of physically-retained entries is
    /// `last_log_index + 1 - base_log_index`.
    #[must_use]
    pub fn base_log_index(&self) -> u64 {
        self.core.lock().expect("core lock").log.base_index()
    }

    /// The number of entries physically retained in the durable log (the tail
    /// after any discarded prefix) — for asserting a compaction really shrank the
    /// on-disk log.
    #[must_use]
    pub fn retained_log_len(&self) -> usize {
        self.core.lock().expect("core lock").log.len()
    }

    /// Wake the tick thread immediately (e.g. after a proposal).
    fn signal_wake(&self) {
        let (lock, cvar) = &*self.wake;
        *lock.lock().expect("wake lock") = true;
        cvar.notify_all();
    }

    /// Cleanly stop the node's threads and join them (deadline-free local
    /// teardown: the stop flag + a self-connect unblock the accept/serve loops).
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake the tick thread out of its condvar wait.
        {
            let (lock, cvar) = &*self.wake;
            *lock.lock().expect("wake lock") = true;
            cvar.notify_all();
        }
        // Nudge the accept loop so a blocked accept() returns and sees the stop.
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.tick_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.serve_handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for RaftNode {
    fn drop(&mut self) {
        if self.serve_handle.is_some() || self.tick_handle.is_some() {
            self.stop_and_join();
        }
    }
}

/// Path of the persistent-state file beside the journal.
fn persist_path(journal_path: &Path) -> PathBuf {
    let mut s = journal_path.as_os_str().to_os_string();
    s.push(".raft");
    PathBuf::from(s)
}

// ---------------------------------------------------------------------------
// Serve loop: answer inbound RPCs (AppendEntries, RequestVote, StatusRequest).
// ---------------------------------------------------------------------------

fn serve_loop(listener: TcpListener, core: Arc<Mutex<NodeCore>>, stop: Arc<AtomicBool>) {
    // One worker thread PER accepted connection so several peers can be served
    // concurrently — a single-threaded accept-and-serve loop would head-of-line
    // block one peer's RPC behind another's, stalling heartbeats/replication under
    // the all-to-all dialing of a multi-node cluster. Each `request_once` client is
    // one-shot (connect → one frame → reply → drop), so a worker handles one short
    // exchange and exits on the peer disconnect or the stop flag; workers are
    // detached (they self-terminate), and the listener close on shutdown unblocks
    // the accept below.
    while !stop.load(Ordering::Acquire) {
        let conn = match listener.accept() {
            Ok((conn, _)) => conn,
            Err(_) => break,
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        let _ = conn.set_read_timeout(Some(Duration::from_millis(100)));
        let worker_core = Arc::clone(&core);
        let worker_stop = Arc::clone(&stop);
        match thread::Builder::new()
            .name("raft-conn".into())
            .spawn(move || serve_conn(conn, &worker_core, &worker_stop))
        {
            Ok(_handle) => { /* detached: the worker self-terminates */ }
            Err(_) => {
                // Spawn failed (resource exhaustion): the connection is dropped by
                // scope; the peer's one-shot RPC will simply retry next round. This
                // never corrupts state — it is at worst a missed heartbeat.
            }
        }
    }
}

/// Service one peer connection until it disconnects or a stop is requested.
fn serve_conn(mut conn: TcpStream, core: &Arc<Mutex<NodeCore>>, stop: &Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let msg = match read_frame_or_idle(&mut conn) {
            Ok(FrameRead::Idle) => continue,
            Ok(FrameRead::Frame(m)) => m,
            Err(_) => return,
        };
        let reply = {
            let mut c = core.lock().expect("core lock");
            handle_rpc(&mut c, msg)
        };
        if let Some(reply) = reply
            && write_frame(&mut conn, &reply).is_err()
        {
            return;
        }
    }
}

/// Handle one inbound RPC under the core lock (no blocking IO here). Returns the
/// reply frame to send, or `None` for messages that need no reply.
fn handle_rpc(core: &mut NodeCore, msg: Message) -> Option<Message> {
    match msg {
        Message::AppendEntries {
            term,
            prev_log_index,
            prev_log_term,
            entries,
            leader_commit,
        } => Some(handle_append_entries(
            core,
            term,
            prev_log_index,
            prev_log_term,
            entries,
            leader_commit,
        )),
        Message::RequestVote {
            term,
            pre_vote,
            candidate_id,
            last_log_index,
            last_log_term,
        } => Some(handle_request_vote(
            core,
            term,
            pre_vote,
            candidate_id,
            last_log_index,
            last_log_term,
        )),
        Message::InstallSnapshot {
            term,
            leader_id,
            last_included_index,
            last_included_term,
            snapshot_bytes,
        } => Some(handle_install_snapshot(
            core,
            term,
            leader_id,
            last_included_index,
            last_included_term,
            &snapshot_bytes,
        )),
        Message::StatusRequest => Some(Message::Status {
            last_index: core.log.last_index().unwrap_or(EMPTY_LOG),
            term: core.current_term(),
        }),
        // Replies are consumed by the tick thread's request path, not the serve
        // loop; if one arrives here it is unsolicited and ignored.
        Message::AppendReply { .. } | Message::VoteReply { .. } | Message::Status { .. } => None,
    }
}

/// AppendEntries receiver (§5.1–5.3): term check → step-down → log-matching →
/// conflicting-tail reconcile → commit advance → reply.
fn handle_append_entries(
    core: &mut NodeCore,
    term: u64,
    prev_log_index: u64,
    prev_log_term: u64,
    entries: Vec<LogEntry>,
    leader_commit: u64,
) -> Message {
    let my_term = core.current_term();
    // (1) Reply false if the leader's term is stale.
    if term < my_term {
        return Message::AppendReply {
            term: my_term,
            success: false,
            match_index: core.log.last_index().unwrap_or(EMPTY_LOG),
        };
    }
    // (2) A term >= ours from a leader: adopt it and (re)become a follower.
    if term > my_term {
        let _ = core.persist.save(term, None);
    }
    core.role = Role::Follower;
    core.last_leader_contact = Instant::now();
    core.reset_election_timer();

    // (3) Log-matching: reject if we lack an entry at prev_log_index w/ prev_log_term.
    if !core.log.matches_prev(prev_log_index, prev_log_term) {
        return Message::AppendReply {
            term: core.current_term(),
            success: false,
            match_index: core.log.last_index().unwrap_or(EMPTY_LOG),
        };
    }

    // (4) Reconcile entries (idempotent skip / conflicting-tail truncate / append).
    if !entries.is_empty() && core.log.reconcile(&entries).is_err() {
        return Message::AppendReply {
            term: core.current_term(),
            success: false,
            match_index: core.log.last_index().unwrap_or(EMPTY_LOG),
        };
    }

    // (5) Advance commit index to min(leader_commit, last log index) and apply
    //     (durably persisting the watermark via set_commit_index).
    if leader_commit != EMPTY_LOG && core.log.last_index().is_some() {
        let last = core.log.last_index().unwrap_or(0);
        core.set_commit_index(Some(leader_commit.min(last)));
    }

    Message::AppendReply {
        term: core.current_term(),
        success: true,
        match_index: core.log.last_index().unwrap_or(EMPTY_LOG),
    }
}

/// InstallSnapshot receiver (§7): term check → step-down → durably install the
/// snapshot (reseed applied state + log boundary + watermarks) → reply.
///
/// The reply reuses [`Message::AppendReply`]: `term` always (so a stale leader
/// steps down), and on success `match_index == last_included_index` so the leader
/// advances this follower's `next_index`/`match_index` to the snapshot boundary,
/// then resumes AppendEntries from `last_included_index + 1`.
fn handle_install_snapshot(
    core: &mut NodeCore,
    term: u64,
    _leader_id: u64,
    last_included_index: u64,
    last_included_term: u64,
    snapshot_bytes: &[u8],
) -> Message {
    let my_term = core.current_term();
    // (1) Reply false if the leader's term is stale — do not touch any state.
    if term < my_term {
        return Message::AppendReply {
            term: my_term,
            success: false,
            match_index: core.log.last_index().unwrap_or(EMPTY_LOG),
        };
    }
    // (2) A term >= ours from a leader: adopt it and (re)become a follower; this is
    //     valid leader contact, so reset the election timer.
    if term > my_term {
        let _ = core.persist.save(term, None);
    }
    core.role = Role::Follower;
    core.last_leader_contact = Instant::now();
    core.reset_election_timer();

    // (3) Decode the shipped snapshot. A corrupt frame is rejected (success=false)
    //     rather than silently installing garbage — the leader simply retries.
    let Ok(snapshot) = Snapshot::decode(snapshot_bytes) else {
        return Message::AppendReply {
            term: core.current_term(),
            success: false,
            match_index: core.log.last_index().unwrap_or(EMPTY_LOG),
        };
    };
    // The boundary the leader declared in the RPC must match the snapshot it shipped
    // (defence against a mismatched/forged header) — reject otherwise.
    if snapshot.last_included_index != last_included_index
        || snapshot.last_included_term != last_included_term
    {
        return Message::AppendReply {
            term: core.current_term(),
            success: false,
            match_index: core.log.last_index().unwrap_or(EMPTY_LOG),
        };
    }

    // (4) Durably install (snapshot file → log reshape → reseed applied + commit).
    //     A stale install (we already cover the boundary) is harmless — reply success
    //     reporting our own high-water so the leader advances next_index correctly.
    match core.install_snapshot(&snapshot) {
        Ok(_installed) => Message::AppendReply {
            term: core.current_term(),
            success: true,
            // The boundary is now durably accounted for: confirm match up to it (or
            // our own higher high-water if our retained tail already extends past it).
            match_index: core
                .log
                .last_index()
                .map_or(last_included_index, |hw| hw.max(last_included_index)),
        },
        // A durable IO failure: do not claim success — the leader retries.
        Err(_) => Message::AppendReply {
            term: core.current_term(),
            success: false,
            match_index: core.log.last_index().unwrap_or(EMPTY_LOG),
        },
    }
}

/// RequestVote receiver. Two modes:
///
/// * **Pre-vote** (`pre_vote == true`, Ongaro thesis §9.6): a *non-binding straw
///   poll* at the candidate's hypothetical term. It changes NO persistent state on
///   either side. It is granted iff (a) the hypothetical `term` is at least our
///   current term, (b) the candidate's log is at least as up-to-date (§5.4.1), AND
///   (c) we have NOT heard from a leader within the minimum election timeout (so a
///   node still hearing a healthy leader's heartbeats refuses, which is exactly what
///   stops a flaky/partitioned node from disrupting the leader by bumping terms).
///
/// * **Real vote** (`pre_vote == false`, §5.2 + §5.4.1): step down on a higher term;
///   grant iff not yet voted (or already voted for this candidate) this term AND the
///   candidate's log is at least as up-to-date — durably recording the vote first.
fn handle_request_vote(
    core: &mut NodeCore,
    term: u64,
    pre_vote: bool,
    candidate_id: u64,
    last_log_index: u64,
    last_log_term: u64,
) -> Message {
    let my_term = core.current_term();

    if pre_vote {
        // A pre-vote never mutates persistent term/vote on either side.
        let term_ok = term >= my_term;
        let log_ok = core.candidate_log_ok(last_log_index, last_log_term);
        let leader_silent = core.last_leader_contact.elapsed() >= core.cfg.election_min
            && core.role != Role::Leader;
        return Message::VoteReply {
            term: my_term,
            pre_vote: true,
            granted: term_ok && log_ok && leader_silent,
        };
    }

    // Real vote. Stale candidate: refuse and report our higher term.
    if term < my_term {
        return Message::VoteReply {
            term: my_term,
            pre_vote: false,
            granted: false,
        };
    }
    // A newer term: adopt it (clears any prior vote) and become a follower.
    if term > my_term {
        let _ = core.persist.save(term, None);
        core.role = Role::Follower;
        core.votes_for_me = 0;
    }
    let voted_for = core.persist.voted_for();
    let can_vote = voted_for.is_none() || voted_for == Some(candidate_id);
    let log_ok = core.candidate_log_ok(last_log_index, last_log_term);
    if can_vote && log_ok {
        // Durably record the vote BEFORE replying (Raft persistence requirement).
        let _ = core.persist.save(core.current_term(), Some(candidate_id));
        // Granting a vote is "valid candidate contact" → reset the timer so we do
        // not immediately start a competing election.
        core.reset_election_timer();
        Message::VoteReply {
            term: core.current_term(),
            pre_vote: false,
            granted: true,
        }
    } else {
        Message::VoteReply {
            term: core.current_term(),
            pre_vote: false,
            granted: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Tick loop: election timer + leader replication. Issues outbound RPCs without
// holding the core lock across blocking IO.
// ---------------------------------------------------------------------------

fn tick_loop(core: Arc<Mutex<NodeCore>>, stop: Arc<AtomicBool>, wake: Arc<(Mutex<bool>, Condvar)>) {
    let heartbeat = core.lock().expect("core lock").cfg.heartbeat;
    while !stop.load(Ordering::Acquire) {
        let role = core.lock().expect("core lock").role;
        match role {
            Role::Leader => {
                replicate_round(&core, &stop);
                wait_or_wake(&wake, heartbeat);
            }
            Role::Follower | Role::Candidate => {
                let now = Instant::now();
                let deadline = core.lock().expect("core lock").election_deadline;
                if now >= deadline {
                    start_election(&core, &stop);
                } else {
                    // Sleep until the deadline (or a wake), but cap the slice so
                    // we re-check the stop flag responsively.
                    let slice = deadline.saturating_duration_since(now).min(heartbeat);
                    wait_or_wake(&wake, slice);
                }
            }
        }
    }
}

/// Wait up to `dur` for a wake signal or timeout; clears the wake flag.
fn wait_or_wake(wake: &Arc<(Mutex<bool>, Condvar)>, dur: Duration) {
    let (lock, cvar) = &**wake;
    let mut woken = lock.lock().expect("wake lock");
    if !*woken {
        let (g, _timeout) = cvar.wait_timeout(woken, dur).expect("condvar wait_timeout");
        woken = g;
    }
    *woken = false;
}

/// Begin an election attempt. With **Pre-Vote** (Ongaro thesis §9.6): first run a
/// non-binding straw poll at the hypothetical next term WITHOUT bumping our term; if
/// (and only if) a strict majority would grant, run the real election (bump term,
/// self-vote durably, solicit real votes). This prevents a flaky or partitioned node
/// from disrupting a healthy leader: its repeated pre-votes are refused by peers that
/// still hear the leader, so it never inflates the term and never forces a re-election.
fn start_election(core: &Arc<Mutex<NodeCore>>, stop: &Arc<AtomicBool>) {
    // Snapshot the election parameters under the lock (no persistent change yet).
    let (current_term, id, last_log_index, last_log_term, peer_addrs, io_timeout, cluster_size) = {
        let mut c = core.lock().expect("core lock");
        // Become a candidate (volatile role) so the tick loop keeps timing out, but
        // do NOT bump the persistent term until pre-vote succeeds.
        c.role = Role::Candidate;
        c.reset_election_timer();
        (
            c.current_term(),
            c.id,
            c.log.last_index().unwrap_or(EMPTY_LOG),
            c.log.last_term(),
            c.peers
                .values()
                .map(|p| p.addr)
                .collect::<Vec<SocketAddr>>(),
            c.cfg.io_timeout,
            c.cluster_size,
        )
    };
    let majority = cluster_size / 2 + 1;

    if stop.load(Ordering::Acquire) {
        return;
    }

    // --- Pre-vote phase (hypothetical term = current_term + 1; no persistent change).
    let hypothetical = current_term + 1;
    let pre_req = Message::RequestVote {
        term: hypothetical,
        pre_vote: true,
        candidate_id: id,
        last_log_index,
        last_log_term,
    };
    let (pre_grants, _) = solicit(&peer_addrs, io_timeout, &pre_req);
    // +1 for our own (implicit) pre-vote for ourselves.
    if pre_grants + 1 < majority {
        // We could not win — do not bump the term; remain a follower-ish candidate
        // that will retry after the next timeout. Resetting role to Follower avoids
        // a stuck Candidate; the timer was already reset above.
        let mut c = core.lock().expect("core lock");
        if c.current_term() == current_term {
            c.role = Role::Follower;
        }
        return;
    }

    // --- Real election: durably bump term + self-vote, then solicit real votes.
    let term = {
        let mut c = core.lock().expect("core lock");
        // Someone may have moved us on during the pre-vote round.
        if c.current_term() != current_term {
            return;
        }
        let new_term = current_term + 1;
        if c.persist.save(new_term, Some(id)).is_err() {
            c.reset_election_timer();
            return;
        }
        c.role = Role::Candidate;
        c.votes_for_me = 1; // self-vote
        c.reset_election_timer();
        // A single-node cluster (majority == 1) wins on its own self-vote — there
        // are no peers to solicit, so become leader immediately rather than fall
        // through the (empty) reply-folding loop below.
        if c.votes_for_me >= majority {
            become_leader(&mut c);
            return;
        }
        new_term
    };

    if stop.load(Ordering::Acquire) {
        return;
    }

    let req = Message::RequestVote {
        term,
        pre_vote: false,
        candidate_id: id,
        last_log_index,
        last_log_term,
    };
    let (_, replies) = solicit(&peer_addrs, io_timeout, &req);

    // Fold the real votes in.
    let mut c = core.lock().expect("core lock");
    if c.role != Role::Candidate || c.current_term() != term {
        return; // moved on
    }
    for reply in replies {
        if let Some(Message::VoteReply {
            term: reply_term,
            pre_vote: false,
            granted,
        }) = reply
        {
            if reply_term > term {
                let _ = c.step_down(reply_term);
                return;
            }
            if granted {
                c.votes_for_me += 1;
                if c.votes_for_me >= majority {
                    become_leader(&mut c);
                    return;
                }
            }
        }
    }
}

/// Broadcast a (pre-)vote request to all peers CONCURRENTLY (a dead peer must not
/// delay the live peers — a sequential loop would burn `io_timeout` per dead peer,
/// overrunning the election timeout). Returns `(grants, raw_replies)` where `grants`
/// counts granted replies of the matching kind.
fn solicit(
    peer_addrs: &[SocketAddr],
    io_timeout: Duration,
    req: &Message,
) -> (usize, Vec<Option<Message>>) {
    let replies: Vec<Option<Message>> = thread::scope(|scope| {
        let handles: Vec<_> = peer_addrs
            .iter()
            .map(|&addr| {
                let req = req.clone();
                scope.spawn(move || request_once(addr, io_timeout, &req))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("vote thread"))
            .collect()
    });
    let grants = replies
        .iter()
        .filter(|r| matches!(r, Some(Message::VoteReply { granted: true, .. })))
        .count();
    (grants, replies)
}

/// Transition the locked core to Leader: initialise per-peer `next_index` to the
/// end of our log and `match_index` to unknown (§5.3 leader init).
fn become_leader(c: &mut NodeCore) {
    c.role = Role::Leader;
    let next = c.log.last_index().map_or(0, |i| i + 1);
    for p in c.peers.values_mut() {
        p.next_index = next;
        p.match_index = None;
    }
    // A single-node cluster can commit immediately.
    if c.cluster_size == 1 {
        c.leader_advance_commit();
    }
}

/// One leader replication round: for each peer, send the entries it lacks (or a
/// heartbeat), fold the reply (advancing/backing-up `next_index`/`match_index`),
/// then advance the commit index from the resulting majority. Blocking IO happens
/// outside the lock.
fn replicate_round(core: &Arc<Mutex<NodeCore>>, stop: &Arc<AtomicBool>) {
    // Snapshot what to send to each peer under the lock.
    struct Plan {
        pid: u64,
        addr: SocketAddr,
        msg: Message,
        sent_match: u64, // the match_index a success implies
    }
    let (plans, rpc_timeout, term) = {
        let c = core.lock().expect("core lock");
        if c.role != Role::Leader {
            return;
        }
        let term = c.current_term();
        let leader_commit = c.commit_index.unwrap_or(EMPTY_LOG);
        let last = c.log.last_index();
        // Bound each replication RPC well BELOW the minimum election timeout so one
        // slow/unreachable follower cannot delay the round (and thus the heartbeat
        // to the OTHER followers) long enough to make them time out and disrupt this
        // leader. A round therefore completes within `rpc_timeout`, keeping the
        // heartbeat cadence tight; a peer that misses a round is simply retried next
        // round. (Elections keep the longer `io_timeout` — they are not on the
        // steady-state heartbeat cadence.) Healthy loopback RPCs are sub-millisecond,
        // so this only ever clamps a genuinely stuck peer.
        let rpc_timeout = (c.cfg.heartbeat * 3)
            .min(c.cfg.io_timeout)
            .max(Duration::from_millis(20));
        let base_index = c.log.base_index();
        let leader_id = c.id;
        // Read the leader's durable snapshot once if any peer is behind the log base
        // (it has been compacted past) — shared across all such peers this round.
        let need_snapshot = c
            .peers
            .values()
            .any(|p| base_index > 0 && p.next_index < base_index);
        let snapshot = if need_snapshot {
            c.snapshots.load().ok().flatten()
        } else {
            None
        };
        let mut plans = Vec::new();
        // Gather per-peer plans; reading entries from the log needs the lock.
        let peer_ids: Vec<u64> = c.peers.keys().copied().collect();
        for pid in peer_ids {
            let next_index = c.peers[&pid].next_index;
            let addr = c.peers[&pid].addr;

            // If the entries this peer needs (from `next_index`) have been compacted
            // away (`next_index < base_index`), AppendEntries cannot bridge the gap —
            // ship the durable snapshot instead (§7). `sent_match` on success is the
            // snapshot boundary, so a successful install advances the peer to it. (If
            // the base shifted but no durable snapshot is on disk — which cannot
            // happen, since the base only shifts via `discard_prefix`, always preceded
            // by a snapshot save — we fall through to a heartbeat rather than send a
            // gap the follower would reject, and retry next round.)
            let behind_base = base_index > 0 && next_index < base_index;
            if let Some(snap) = snapshot.as_ref().filter(|_| behind_base) {
                plans.push(Plan {
                    pid,
                    addr,
                    msg: Message::InstallSnapshot {
                        term,
                        leader_id,
                        last_included_index: snap.last_included_index,
                        last_included_term: snap.last_included_term,
                        snapshot_bytes: snap.encode(),
                    },
                    sent_match: snap.last_included_index,
                });
                continue;
            }

            // prev = entry just before next_index.
            let (prev_log_index, prev_log_term) = match next_index.checked_sub(1) {
                None => (EMPTY_PREV, 0),
                Some(prev) => (prev, c.log.term_at(prev).unwrap_or(0)),
            };
            let entries = match last {
                Some(l) if l >= next_index => c.log.entries_from(next_index).unwrap_or_default(),
                _ => Vec::new(), // heartbeat
            };
            let sent_match = entries.last().map_or(
                // No new entries: a success confirms up to prev (or empty).
                match prev_log_index {
                    EMPTY_PREV => EMPTY_LOG,
                    p => p,
                },
                |e| e.index,
            );
            plans.push(Plan {
                pid,
                addr,
                msg: Message::AppendEntries {
                    term,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit,
                },
                sent_match,
            });
        }
        (plans, rpc_timeout, term)
    };

    if stop.load(Ordering::Acquire) {
        return;
    }

    // Issue every peer RPC CONCURRENTLY (one scoped thread each), each bounded by the
    // short `rpc_timeout`, so a slow or dead peer cannot delay the heartbeat to the
    // others — a sequential round would let one unreachable peer stall the live
    // followers of heartbeats until they time out and start spurious elections.
    // Replies are collected, then folded under the lock.
    let replies: Vec<(u64, u64, Option<Message>)> = thread::scope(|scope| {
        let handles: Vec<_> = plans
            .iter()
            .map(|plan| {
                let addr = plan.addr;
                let msg = plan.msg.clone();
                scope.spawn(move || {
                    (
                        plan.pid,
                        plan.sent_match,
                        request_once(addr, rpc_timeout, &msg),
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("rpc thread"))
            .collect()
    });

    // Fold all replies under one lock acquisition.
    let mut c = core.lock().expect("core lock");
    if c.role != Role::Leader || c.current_term() != term {
        return; // stepped down meanwhile
    }
    for (pid, sent_match, reply) in replies {
        match reply {
            Some(Message::AppendReply {
                term: reply_term,
                success,
                match_index,
            }) => {
                if reply_term > term {
                    let _ = c.step_down(reply_term);
                    return;
                }
                if let Some(p) = c.peers.get_mut(&pid) {
                    if success {
                        let m = (sent_match != EMPTY_LOG).then_some(sent_match);
                        p.match_index = m;
                        p.next_index = m.map_or(0, |x| x + 1);
                    } else {
                        // Log-matching failure: back up next_index toward the
                        // follower's reported high-water (fast) or by one (safe).
                        let reported = match match_index {
                            EMPTY_LOG => 0,
                            n => n + 1,
                        };
                        let backed = p.next_index.saturating_sub(1).min(reported);
                        p.next_index = backed;
                    }
                }
            }
            Some(_) | None => { /* unreachable peer or odd reply → retry next round */ }
        }
    }

    // Advance commit from the freshly-folded match_index majority.
    if c.role == Role::Leader && c.current_term() == term {
        c.leader_advance_commit();
    }
}

/// Open a one-shot connection to `addr`, send `req`, read one reply, all bounded
/// by `io_timeout`. Returns `None` on any IO failure (treated as "peer down this
/// round"). A fresh connection per RPC keeps the transport stateless and avoids a
/// half-closed cached socket desyncing a later round — correctness over a small
/// connection cost, acceptable on the loopback proof path.
fn request_once(addr: SocketAddr, io_timeout: Duration, req: &Message) -> Option<Message> {
    let mut stream = TcpStream::connect_timeout(&addr, io_timeout).ok()?;
    stream.set_read_timeout(Some(io_timeout)).ok()?;
    stream.set_write_timeout(Some(io_timeout)).ok()?;
    stream.set_nodelay(true).ok()?;
    write_frame(&mut stream, req).ok()?;
    read_frame(&mut stream).ok()
}

/// A cheap, lock-free source of election-timer jitter, mixing the node id with a
/// process-global monotonic counter through splitmix64. Successive calls (and
/// distinct node ids) yield well-spread values, which is all election timing
/// requires: the timers need to be *desynchronized* across nodes and *varied*
/// across re-elections so split votes resolve — they need not be reproducible.
/// (Not a cryptographic RNG; not a pricing input, so priced-state determinism is
/// unaffected.)
fn next_jitter(id: u64) -> u64 {
    use std::sync::atomic::AtomicU64;
    static CTR: AtomicU64 = AtomicU64::new(0xABCD_1234_5678_9F01);
    let c = CTR.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    let mut x = id
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(c)
        .wrapping_add(0xD1B5_4A32_D192_ED03);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod core_tests {
    //! Direct, deterministic unit tests of the consensus core: the pure decision
    //! helpers ([`NodeCore::candidate_log_ok`], [`NodeCore::reset_election_timer`]),
    //! the RPC receivers ([`handle_append_entries`] / [`handle_install_snapshot`] /
    //! [`handle_request_vote`]), and the single-node [`RaftNode`] accessor /
    //! lifecycle surface. These exercise the branch arithmetic the end-to-end
    //! loopback gates (`tests/replication.rs`) drive only indirectly, so a syntactic
    //! mutation of a comparison, a boolean connective, or a returned value is caught
    //! by a *direct* observable assertion rather than relying on a timing-sensitive
    //! cluster outcome. The independent oracle is the Raft §5.3/§5.4 specification
    //! itself, re-derived inline at each assertion (not the production decision
    //! grading itself).

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// A unique temp directory + journal path for an isolated test node.
    fn temp_journal() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "celnet-replog-core-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.push("node.journal");
        dir
    }

    /// Build a bare [`NodeCore`] on a fresh temp journal (no threads, no sockets) —
    /// the unit-test substrate for the pure decision logic. `cluster_size` peers are
    /// registered at loopback placeholder addresses (never dialed in these tests).
    fn core(cluster_size: usize) -> NodeCore {
        let path = temp_journal();
        let log = Log::open(&path).unwrap();
        let persist = PersistStore::open(persist_path(&path)).unwrap();
        let snapshots = SnapshotStore::new(snapshot_path(&path));
        let cfg = RaftConfig::default();
        let mut peers = HashMap::new();
        for i in 0..cluster_size.saturating_sub(1) {
            let pid = 50_000 + i as u64;
            peers.insert(
                pid,
                PeerState {
                    addr: SocketAddr::from(([127, 0, 0, 1], pid as u16)),
                    next_index: 0,
                    match_index: None,
                },
            );
        }
        NodeCore {
            id: 40_000,
            role: Role::Follower,
            persist,
            log,
            snapshots,
            applied: BookState::new(),
            commit_index: None,
            last_applied: None,
            cluster_size,
            peers,
            election_deadline: Instant::now(),
            votes_for_me: 0,
            last_leader_contact: Instant::now()
                .checked_sub(Duration::from_secs(3600))
                .unwrap_or_else(Instant::now),
            cfg,
        }
    }

    fn data_entry(term: u64, index: u64, key: u64, value: f64) -> LogEntry {
        LogEntry::new(term, index, BookUpdate::Set { key, value }.encode())
    }

    // ---- candidate_log_ok (§5.4.1 up-to-date rule) — lines 151/154/162 -------

    #[test]
    fn candidate_log_ok_higher_last_term_wins_over_longer_log() {
        let mut c = core(3);
        // Our log: two entries, last term 5. A candidate with a SHORTER log but a
        // HIGHER last term is up-to-date (term dominates length — §5.4.1).
        c.log.append(&data_entry(5, 0, 1, 1.0)).unwrap();
        c.log.append(&data_entry(5, 1, 2, 2.0)).unwrap();
        // Candidate: last_index 0 (shorter), last_term 6 (higher) → granted.
        assert!(
            c.candidate_log_ok(0, 6),
            "higher last term must be up-to-date"
        );
        // Candidate: last_index 9 (longer) but last_term 4 (lower) → NOT up-to-date.
        // (kills `replace candidate_log_ok -> true`, and the `!=`/`>` term branch.)
        assert!(
            !c.candidate_log_ok(9, 4),
            "a lower last term is stale regardless of length"
        );
    }

    #[test]
    fn candidate_log_ok_equal_term_compares_length() {
        let mut c = core(3);
        c.log.append(&data_entry(5, 0, 1, 1.0)).unwrap();
        c.log.append(&data_entry(5, 1, 2, 2.0)).unwrap(); // my_last_index = 1, len 2
        // Equal last term (5): a candidate at least as long is up-to-date.
        assert!(
            c.candidate_log_ok(1, 5),
            "equal term + equal length → up-to-date"
        );
        assert!(c.candidate_log_ok(2, 5), "equal term + longer → up-to-date");
        // Equal term but a STRICTLY SHORTER log is NOT up-to-date. With len = idx+1,
        // candidate last_index 0 → len 1 < my len 2. This kills the `>=`→`>`/`<`/`==`
        // and the `+ 1`→`- 1`/`* 1` length-arithmetic mutants: only correct `+ 1`
        // makes (cand_len = 0+1 = 1) < (my_len = 1+1 = 2) reject, while (cand 1 → 2)
        // ties and (cand 2 → 3) accepts.
        assert!(
            !c.candidate_log_ok(0, 5),
            "equal term + strictly shorter → stale"
        );
    }

    #[test]
    fn candidate_log_ok_empty_log_sentinel() {
        let c = core(3); // empty log: last_term 0, last_index None (len 0)
        // An empty candidate (EMPTY_LOG sentinel, term 0) ties our empty log → ok.
        assert!(c.candidate_log_ok(EMPTY_LOG, 0));
        // Any real entry (term 1) beats our empty log.
        assert!(c.candidate_log_ok(0, 1));
    }

    // ---- reset_election_timer span arithmetic — line 190 ---------------------

    #[test]
    fn reset_election_timer_deadline_within_configured_window() {
        let mut c = core(3);
        // election_min/max come from RaftConfig::default(); the deadline must land in
        // [now + min, now + max]. The `+`→`-` mutant on line 190 would compute
        // `min - jitter`, pulling the deadline BEFORE `now + min` (often into the
        // past), which this lower-bound assertion catches.
        let before = Instant::now();
        c.reset_election_timer();
        let deadline = c.election_deadline;
        assert!(
            deadline >= before + c.cfg.election_min,
            "deadline must be at least now + election_min (kills the - mutant)"
        );
        assert!(
            deadline <= Instant::now() + c.cfg.election_max,
            "deadline must not exceed now + election_max"
        );
    }

    // ---- handle_append_entries — lines 965/973/1000 --------------------------

    #[test]
    fn append_entries_rejects_stale_term_and_accepts_current() {
        let mut c = core(3);
        c.persist.save(5, None).unwrap(); // my term = 5
        // Stale leader (term 4 < 5): reject, report my term. Kills `< with >`/`==`.
        let reply = handle_append_entries(&mut c, 4, EMPTY_PREV, 0, vec![], EMPTY_LOG);
        match reply {
            Message::AppendReply { term, success, .. } => {
                assert_eq!(term, 5);
                assert!(!success, "a term strictly below ours is rejected");
            }
            _ => panic!("expected AppendReply"),
        }
        // A current-term heartbeat from a valid leader (empty entries, empty prev):
        // accepted, role demoted to Follower.
        let reply = handle_append_entries(&mut c, 5, EMPTY_PREV, 0, vec![], EMPTY_LOG);
        match reply {
            Message::AppendReply { success, .. } => {
                assert!(success, "a current-term, matching heartbeat is accepted");
                assert_eq!(c.role, Role::Follower);
            }
            _ => panic!("expected AppendReply"),
        }
    }

    #[test]
    fn append_entries_adopts_a_strictly_higher_term() {
        let mut c = core(3);
        c.persist.save(3, Some(40_009)).unwrap(); // term 3, voted for someone
        c.role = Role::Candidate;
        // A leader at a STRICTLY HIGHER term (5 > 3) must be adopted: the durable
        // term advances to 5 and the prior vote is cleared (Raft §5.1). The reply
        // therefore reports term 5. Kills `> with ==` (`5 == 3` false → not adopted →
        // term stays 3) and `> with <` (`5 < 3` false → not adopted).
        let reply = handle_append_entries(&mut c, 5, EMPTY_PREV, 0, vec![], EMPTY_LOG);
        match reply {
            Message::AppendReply { term, success, .. } => {
                assert!(success);
                assert_eq!(
                    term, 5,
                    "the higher term is adopted (kills 973 `>`->`==`/`<`)"
                );
            }
            _ => panic!("expected AppendReply"),
        }
        assert_eq!(c.current_term(), 5, "durable term advanced to the leader's");
        assert_eq!(
            c.persist.voted_for(),
            None,
            "adopting a higher term clears the vote"
        );
        assert_eq!(c.role, Role::Follower);
    }

    #[test]
    fn append_entries_at_equal_term_preserves_the_vote() {
        // The term-adopt guard is `if term > my_term { save(term, None) }` (line 973).
        // At an EQUAL term the original does NOT re-save, so an existing `voted_for`
        // this term is PRESERVED (Raft §5: a node must not forget its vote within a
        // term — re-clearing it could enable a double vote and break election safety).
        // The `> with >=` mutant makes `term >= my_term` true at equality → it would
        // `save(term, None)`, WRONGLY clearing the vote. We assert the vote survives an
        // equal-term AppendEntries (a current-term leader's heartbeat), killing the
        // `>=` flip (the `==`/`<` flips are killed by the higher-term test above).
        let mut c = core(3);
        c.persist.save(4, Some(40_012)).unwrap(); // term 4, voted for 40_012
        // A current-term (4 == 4) heartbeat from the leader (empty entries/prev).
        let reply = handle_append_entries(&mut c, 4, EMPTY_PREV, 0, vec![], EMPTY_LOG);
        assert!(matches!(reply, Message::AppendReply { success: true, .. }));
        assert_eq!(
            c.current_term(),
            4,
            "an equal-term heartbeat does not bump the term"
        );
        assert_eq!(
            c.persist.voted_for(),
            Some(40_012),
            "an equal-term AppendEntries must PRESERVE the vote (kills 973 `>`->`>=`)"
        );
    }

    #[test]
    fn append_entries_commit_advances_only_with_a_present_log_and_real_commit() {
        let mut c = core(3);
        c.persist.save(3, None).unwrap();
        // Leader appends one entry (index 0) and declares leader_commit = 0.
        let e0 = data_entry(3, 0, 7, 1.5);
        let reply = handle_append_entries(&mut c, 3, EMPTY_PREV, 0, vec![e0], 0);
        match reply {
            Message::AppendReply {
                success,
                match_index,
                ..
            } => {
                assert!(success);
                assert_eq!(match_index, 0);
            }
            _ => panic!("expected AppendReply"),
        }
        // The commit guard `leader_commit != EMPTY_LOG && last_index().is_some()`
        // (line 1000) held (both true): the entry committed and applied.
        assert_eq!(
            c.commit_index,
            Some(0),
            "commit advanced to the matched index"
        );
        assert_eq!(c.applied.get(7).map(f64::to_bits), Some(1.5f64.to_bits()));

        // Now the kill for the `&&`->`||` mutant: a SECOND heartbeat with
        // leader_commit == EMPTY_LOG (the leader has committed nothing) and a present
        // log. Original `false && true = false` → NO further commit. The `||` mutant
        // `false || true = true` → would run `set_commit_index(Some(EMPTY_LOG.min(last)))`
        // = set commit to `last` (0) — but more importantly, on a longer log it would
        // WRONGLY commit everything. Append a second entry first so an erroneous
        // commit-to-last is observable, then send the EMPTY_LOG-commit heartbeat.
        let mut c2 = core(3);
        c2.persist.save(3, None).unwrap();
        c2.log.append(&data_entry(3, 0, 1, 1.0)).unwrap();
        c2.log.append(&data_entry(3, 1, 2, 2.0)).unwrap();
        // prev (1, term 3) matches the tail; entries empty; leader_commit EMPTY_LOG.
        let reply = handle_append_entries(&mut c2, 3, 1, 3, vec![], EMPTY_LOG);
        assert!(matches!(reply, Message::AppendReply { success: true, .. }));
        assert_eq!(
            c2.commit_index, None,
            "an EMPTY_LOG leader_commit must NOT advance commit (kills 1000 `&&`->`||`)"
        );
        assert!(
            c2.applied.is_empty(),
            "nothing applied without a real leader_commit"
        );
    }

    // ---- handle_install_snapshot — lines 1029/1038/1057 ----------------------

    #[test]
    fn install_snapshot_rejects_stale_term() {
        let mut c = core(3);
        c.persist.save(7, None).unwrap();
        let snap = Snapshot::new(2, 3, BookState::new());
        // term 6 < my 7 → reject without touching state. Kills `< with ==`/`>`/`<=`.
        let reply = handle_install_snapshot(&mut c, 6, 99, 2, 3, &snap.encode());
        match reply {
            Message::AppendReply { term, success, .. } => {
                assert_eq!(term, 7);
                assert!(!success);
            }
            _ => panic!("expected AppendReply"),
        }
        assert!(
            c.log.snapshot_index().is_none(),
            "a stale install changes nothing"
        );
    }

    #[test]
    fn install_snapshot_adopts_a_strictly_higher_term() {
        let mut c = core(3);
        c.persist.save(3, Some(40_009)).unwrap(); // term 3, voted
        c.role = Role::Candidate;
        // A leader at a STRICTLY HIGHER term (5 > 3) installs a valid snapshot: the
        // handler must adopt term 5 and clear the vote (line 1038 `if term > my_term
        // { save(term, None) }`). The reply reports term 5. Kills `> with ==`
        // (`5 == 3` false → not adopted), `> with <` (`5 < 3` false), and `> with >=`
        // (would also re-adopt on an EQUAL term, clearing a vote it should keep — the
        // equal-term path is exercised by other tests; here the strict-higher adopt
        // must land at 5).
        let snap = Snapshot::new(2, 1, BookState::new());
        let reply = handle_install_snapshot(&mut c, 5, 99, 2, 1, &snap.encode());
        match reply {
            Message::AppendReply { term, success, .. } => {
                assert!(success, "a valid higher-term install succeeds");
                assert_eq!(
                    term, 5,
                    "the higher term is adopted (kills 1038 `>`->`==`/`<`/`>=`)"
                );
            }
            _ => panic!("expected AppendReply"),
        }
        assert_eq!(c.current_term(), 5, "durable term advanced to the leader's");
        assert_eq!(
            c.persist.voted_for(),
            None,
            "adopting a higher term clears the vote"
        );
        assert_eq!(c.role, Role::Follower);
    }

    #[test]
    fn install_snapshot_at_equal_term_preserves_the_vote() {
        // The term-adopt guard is `if term > my_term { save(term, None) }` (line 1038).
        // At an EQUAL term the original does NOT re-save — so an existing `voted_for`
        // this term is PRESERVED (Raft §5: a node must not forget its vote within a
        // term; re-clearing it could enable a double vote and break election safety).
        // The `> with >=` mutant makes `term >= my_term` true at equality → it would
        // `save(term, None)`, WRONGLY clearing the vote. We assert the vote survives an
        // equal-term install, killing the `>=` flip (the `==`/`<` flips are killed by
        // `install_snapshot_adopts_a_strictly_higher_term`).
        let mut c = core(3);
        c.persist.save(4, Some(40_011)).unwrap(); // term 4, voted for 40_011
        let snap = Snapshot::new(2, 1, BookState::new());
        let reply = handle_install_snapshot(&mut c, 4, 99, 2, 1, &snap.encode()); // term == my 4
        assert!(matches!(reply, Message::AppendReply { success: true, .. }));
        assert_eq!(
            c.current_term(),
            4,
            "an equal-term install does not bump the term"
        );
        assert_eq!(
            c.persist.voted_for(),
            Some(40_011),
            "an equal-term install must PRESERVE the vote (kills 1038 `>`->`>=`)"
        );
    }

    #[test]
    fn install_snapshot_rejects_header_boundary_mismatch() {
        let mut c = core(3);
        c.persist.save(4, None).unwrap();
        // The shipped snapshot's boundary is (5, 2) but the RPC header declares
        // (5, 9): the term mismatch must be rejected (defence against a forged
        // header). Kills the `||`→`&&` (line 1057) and `!=`→`==` (line 1057:40)
        // mutants on the boundary-equality guard.
        let snap = Snapshot::new(5, 2, BookState::new());
        let reply = handle_install_snapshot(&mut c, 4, 99, 5, 9, &snap.encode());
        match reply {
            Message::AppendReply { success, .. } => {
                assert!(!success, "header term mismatch must be rejected");
            }
            _ => panic!("expected AppendReply"),
        }
        assert!(c.log.snapshot_index().is_none());
        // And a header that DOES match installs (success), so the guard is not
        // simply always-false.
        let snap_ok = Snapshot::new(5, 2, BookState::new());
        let reply = handle_install_snapshot(&mut c, 4, 99, 5, 2, &snap_ok.encode());
        match reply {
            Message::AppendReply { success, .. } => assert!(success),
            _ => panic!("expected AppendReply"),
        }
        assert_eq!(c.log.snapshot_index(), Some(5));
    }

    #[test]
    fn install_snapshot_stale_index_guard_is_high_water() {
        let mut c = core(3);
        c.persist.save(4, None).unwrap();
        // Install boundary 5, then a second install at the SAME boundary is stale
        // (already covered) — but the follower replies success reporting its own
        // high-water. The `> with ==`/`<`/`>=` mutants on the `last_applied >= lii`
        // (line 1038) guard direction are exercised: the second install must remain
        // a no-op (snapshot_index unchanged at 5).
        let snap = Snapshot::new(5, 2, BookState::new());
        let _ = handle_install_snapshot(&mut c, 4, 99, 5, 2, &snap.encode());
        assert_eq!(c.log.snapshot_index(), Some(5));
        let snap2 = Snapshot::new(5, 2, BookState::new());
        let reply = handle_install_snapshot(&mut c, 4, 99, 5, 2, &snap2.encode());
        match reply {
            Message::AppendReply { success, .. } => assert!(success),
            _ => panic!("expected AppendReply"),
        }
        assert_eq!(
            c.log.snapshot_index(),
            Some(5),
            "stale re-install is a no-op"
        );
    }

    // ---- handle_request_vote — lines 1117/1121/1126/1134/1140/1142 -----------

    #[test]
    fn request_vote_real_grants_then_refuses_a_second_candidate() {
        let mut c = core(3);
        c.persist.save(1, None).unwrap();
        c.log.append(&data_entry(1, 0, 1, 1.0)).unwrap();
        // Candidate 7 at term 2 with an up-to-date log → grant + durable vote.
        let reply = handle_request_vote(&mut c, 2, false, 7, 0, 1);
        match reply {
            Message::VoteReply {
                granted,
                term,
                pre_vote,
            } => {
                assert!(granted, "first up-to-date candidate is granted");
                assert_eq!(term, 2);
                assert!(!pre_vote);
            }
            _ => panic!("expected VoteReply"),
        }
        assert_eq!(c.persist.voted_for(), Some(7));
        // A DIFFERENT candidate 8 in the SAME term must be refused (already voted).
        // Kills `can_vote` connective mutants (line 1142 `&&`→`||`) and the
        // `voted_for == Some(candidate_id)` (line 1140 `==`→`!=`) check.
        let reply = handle_request_vote(&mut c, 2, false, 8, 0, 1);
        match reply {
            Message::VoteReply { granted, .. } => {
                assert!(
                    !granted,
                    "a second distinct candidate in the term is refused"
                )
            }
            _ => panic!("expected VoteReply"),
        }
    }

    #[test]
    fn request_vote_refuses_stale_log_even_when_unvoted() {
        let mut c = core(3);
        c.persist.save(3, None).unwrap();
        c.log.append(&data_entry(3, 0, 1, 1.0)).unwrap();
        c.log.append(&data_entry(3, 1, 2, 2.0)).unwrap(); // my last (idx1, term3)
        // Unvoted, but the candidate's log is STALE (last term 2 < 3): refuse.
        // Kills `can_vote && log_ok` (line 1142) collapsing to `||`.
        let reply = handle_request_vote(&mut c, 4, false, 9, 5, 2);
        match reply {
            Message::VoteReply { granted, .. } => {
                assert!(
                    !granted,
                    "a stale-log candidate is refused even when unvoted"
                )
            }
            _ => panic!("expected VoteReply"),
        }
    }

    #[test]
    fn request_vote_grants_at_an_equal_term_when_unvoted() {
        let mut c = core(3);
        c.persist.save(2, None).unwrap(); // term 2, NOT yet voted
        c.log.append(&data_entry(2, 0, 1, 1.0)).unwrap();
        // A candidate at the SAME term (2) as ours, with an up-to-date log, and we
        // have not voted → MUST be granted (Raft §5.2: a voter grants at most one
        // vote per term; an equal-term candidate is NOT stale). The stale guard is
        // `if term < my_term { refuse }` (line 1126): `2 < 2` is false → proceed →
        // grant. The `< with <=` mutant makes `2 <= 2` true → it would wrongly refuse
        // the equal-term candidate as stale. The grant below kills it.
        let reply = handle_request_vote(&mut c, 2, false, 7, 0, 2);
        match reply {
            Message::VoteReply { granted, term, .. } => {
                assert!(
                    granted,
                    "an equal-term, up-to-date, unvoted candidate is granted"
                );
                assert_eq!(term, 2);
            }
            _ => panic!("expected VoteReply"),
        }
        assert_eq!(
            c.persist.voted_for(),
            Some(7),
            "the equal-term vote is recorded"
        );
    }

    #[test]
    fn request_vote_prevote_requires_term_log_and_leader_silence() {
        let mut c = core(3);
        c.persist.save(2, None).unwrap();
        c.log.append(&data_entry(2, 0, 1, 1.0)).unwrap();
        // last_leader_contact is far in the past (set by `core`), role Follower →
        // leader_silent true. A pre-vote at term 3 with an up-to-date log is granted,
        // and changes NO persistent state. Kills the `&&` connectives at lines
        // 1117/1121 (term_ok && log_ok && leader_silent).
        let reply = handle_request_vote(&mut c, 3, true, 7, 0, 2);
        match reply {
            Message::VoteReply {
                granted, pre_vote, ..
            } => {
                assert!(
                    granted,
                    "an up-to-date pre-vote during leader silence is granted"
                );
                assert!(pre_vote);
            }
            _ => panic!("expected VoteReply"),
        }
        assert_eq!(
            c.persist.voted_for(),
            None,
            "a pre-vote never records a vote"
        );
        assert_eq!(c.current_term(), 2, "a pre-vote never bumps the term");
        // A pre-vote at a LOWER term (term_ok false) is refused — kills a connective
        // that would ignore the term gate (line 1126 `< with ==`/`<=` on `term < my`
        // is in the real-vote path; the pre-vote `term >= my` gate is exercised here).
        let reply = handle_request_vote(&mut c, 1, true, 7, 0, 2);
        match reply {
            Message::VoteReply { granted, .. } => {
                assert!(!granted, "a below-term pre-vote is refused")
            }
            _ => panic!("expected VoteReply"),
        }

        // A node that has RECENTLY heard from a leader (elapsed < election_min) is NOT
        // leader-silent and MUST refuse a pre-vote — this is the disruption guard
        // (Ongaro thesis §9.6). `leader_silent = elapsed >= election_min && role !=
        // Leader` (line 1117): with a fresh contact `elapsed < min` is false, role is
        // Follower (true), so `false && true = false` → refuse. The `&& with ||` mutant
        // makes `false || true = true` → it would WRONGLY grant during a healthy
        // leader's heartbeats. The refusal below kills the 1117 connective.
        c.last_leader_contact = Instant::now(); // just heard from the leader
        let reply = handle_request_vote(&mut c, 3, true, 7, 0, 2);
        match reply {
            Message::VoteReply { granted, .. } => {
                assert!(
                    !granted,
                    "a recently-contacted node refuses pre-votes (kills 1117 `&&`->`||`)"
                )
            }
            _ => panic!("expected VoteReply"),
        }
    }

    #[test]
    fn request_vote_steps_down_on_higher_term_then_grants() {
        let mut c = core(3);
        c.persist.save(2, Some(99)).unwrap(); // already voted for 99 in term 2
        c.role = Role::Candidate;
        // A real vote at a HIGHER term (3 > 2) adopts the term, clears the vote
        // (becomes follower), then grants to this candidate. Kills `< with ==`/`<=`
        // (line 1126, stale guard) and `> with ==`/`<`/`>=` (line 1134, adopt guard).
        let reply = handle_request_vote(&mut c, 3, false, 5, EMPTY_LOG, 0);
        match reply {
            Message::VoteReply { granted, term, .. } => {
                assert!(granted, "a higher-term candidate with an OK log is granted");
                assert_eq!(term, 3);
            }
            _ => panic!("expected VoteReply"),
        }
        assert_eq!(c.role, Role::Follower);
        assert_eq!(c.persist.voted_for(), Some(5));
    }

    // ---- leader_advance_commit / set_commit_index (quorum + apply) ----------

    #[test]
    fn leader_advance_commit_requires_a_current_term_majority() {
        let mut c = core(3); // 2 peers + self, majority = 2
        c.persist.save(4, None).unwrap();
        c.role = Role::Leader;
        c.log.append(&data_entry(4, 0, 1, 10.0)).unwrap();
        c.log.append(&data_entry(4, 1, 2, 20.0)).unwrap();
        // No peer has acknowledged anything yet → only the leader holds the entries,
        // which is 1 of 3 (< majority 2) → no commit.
        c.leader_advance_commit();
        assert_eq!(
            c.commit_index, None,
            "a lone leader cannot commit (no majority)"
        );
        // One peer now holds index 1 → holders = 2 (leader + peer) >= majority →
        // commit advances to 1 and BOTH entries apply in order.
        let pid = *c.peers.keys().next().unwrap();
        c.peers.get_mut(&pid).unwrap().match_index = Some(1);
        c.leader_advance_commit();
        assert_eq!(c.commit_index, Some(1), "a current-term majority commits");
        assert_eq!(c.applied.get(1).map(f64::to_bits), Some(10.0f64.to_bits()));
        assert_eq!(c.applied.get(2).map(f64::to_bits), Some(20.0f64.to_bits()));
    }

    // ---- step_down (§5.1) — line 173 ----------------------------------------

    #[test]
    fn step_down_records_term_clears_vote_and_demotes() {
        let mut c = core(3);
        c.persist.save(2, Some(99)).unwrap(); // term 2, voted for 99
        c.role = Role::Leader;
        c.votes_for_me = 2;
        // Step down to a strictly higher term. The `step_down -> Ok(())` mutant skips
        // ALL of this (no save, no role change, no vote clear), which the direct
        // assertions below catch — step_down is otherwise only reached via the
        // threaded leader reply path (replicate_round), so this is its direct pin.
        c.step_down(5).unwrap();
        assert_eq!(
            c.current_term(),
            5,
            "step_down durably adopts the higher term"
        );
        assert_eq!(c.persist.voted_for(), None, "step_down clears the vote");
        assert_eq!(c.role, Role::Follower, "step_down demotes to follower");
        assert_eq!(c.votes_for_me, 0, "step_down resets the vote tally");
    }

    // ---- set_commit_index advance guard — line 198 --------------------------

    #[test]
    fn set_commit_index_equal_value_does_not_re_apply() {
        let mut c = core(3);
        c.log.append(&data_entry(1, 0, 1, 1.0)).unwrap();
        c.log.append(&data_entry(1, 1, 2, 2.0)).unwrap();
        c.log.append(&data_entry(1, 2, 3, 3.0)).unwrap();
        // Construct the observable boundary state: commit_index is ALREADY Some(2)
        // but NOTHING has been applied yet (last_applied None, applied empty). The
        // original guard `n > c` for set_commit_index(Some(2)) is `2 > 2 = false`, so
        // it MUST NOT advance/apply — the book stays empty. The `> with >=` mutant
        // makes `2 >= 2 = true`, which would (wrongly) run apply_committed over
        // [0,2] and populate the book — directly observable here.
        c.commit_index = Some(2);
        c.last_applied = None;
        assert!(c.applied.is_empty(), "precondition: nothing applied yet");
        c.set_commit_index(Some(2)); // equal to the current commit → no-op under `>`
        assert!(
            c.applied.is_empty(),
            "an equal (non-advancing) commit must not re-apply (kills `> with >=`)"
        );
        assert_eq!(
            c.last_applied, None,
            "last_applied unchanged on a non-advance"
        );
        // A strictly-higher commit DOES advance and apply — so the guard is not
        // simply always-false.
        c.commit_index = None; // reset to exercise the real advance below
        c.last_applied = None;
        c.set_commit_index(Some(1));
        assert_eq!(c.commit_index, Some(1));
        assert_eq!(c.applied.get(1).map(f64::to_bits), Some(1.0f64.to_bits()));
        assert_eq!(c.applied.get(2).map(f64::to_bits), Some(2.0f64.to_bits()));
        assert_eq!(c.applied.get(3), None, "only [0,1] applied at commit 1");
    }

    // ---- NodeCore::compact_to (§7 local compaction) — line 278 --------------

    /// Build a leader core with `n` committed+applied data entries (value == index
    /// as f64), so compaction has a real applied prefix to snapshot.
    fn core_with_committed(n: u64) -> NodeCore {
        let mut c = core(1); // single node: a proposal commits on its own durability
        c.persist.save(1, None).unwrap();
        c.role = Role::Leader;
        for i in 0..n {
            c.log.append(&data_entry(1, i, i, i as f64)).unwrap();
        }
        c.set_commit_index(Some(n - 1)); // commit + apply the whole prefix
        assert_eq!(c.last_applied, Some(n - 1));
        c
    }

    #[test]
    fn compact_to_then_second_compaction_seeds_from_the_prior_snapshot() {
        let mut c = core_with_committed(6); // indices 0..=5 applied
        // First compaction to boundary 2 → snapshot captures state-as-of-2 = {0,1,2}.
        assert_eq!(c.compact_to(2).unwrap(), Some(2));
        assert_eq!(c.log.snapshot_index(), Some(2));
        // Second compaction to boundary 4. The retained log now starts at index 3
        // (base_index 3), so reconstructing state-as-of-4 MUST seed from the prior
        // snapshot's captured {0,1,2} and replay only the retained [3,4] on top —
        // line 278's guard `Some(prev.last_included_index) == self.log.snapshot_index()`
        // selects that seed. Mutating the guard to `false` discards the prior state
        // (snapshot-as-of-4 would be missing keys 0,1,2); to `true` would seed even
        // from a NON-matching prior snapshot. We verify the resulting snapshot's state
        // has ALL of keys 0..=4 with exact bits — only the correct seed produces that.
        assert_eq!(c.compact_to(4).unwrap(), Some(4));
        let snap = c.snapshots.load().unwrap().expect("snapshot present");
        assert_eq!(snap.last_included_index, 4);
        for k in 0..=4u64 {
            assert_eq!(
                snap.state.get(k).map(f64::to_bits),
                Some((k as f64).to_bits()),
                "state-as-of-4 must include key {k} from the seeded prior snapshot"
            );
        }
        assert_eq!(
            snap.state.get(5),
            None,
            "boundary 4 does not include index 5"
        );
    }

    #[test]
    fn compact_to_noop_when_nothing_applied_or_already_at_boundary() {
        let mut c = core(1);
        c.persist.save(1, None).unwrap();
        c.role = Role::Leader;
        // Nothing applied yet → None (no snapshot written).
        assert_eq!(c.compact_to(0).unwrap(), None);
        assert!(c.snapshots.load().unwrap().is_none());
        // Now commit a prefix and compact to 1; a repeat at/below 1 is a no-op.
        for i in 0..3 {
            c.log.append(&data_entry(1, i, i, i as f64)).unwrap();
        }
        c.set_commit_index(Some(2));
        assert_eq!(c.compact_to(1).unwrap(), Some(1));
        assert_eq!(
            c.compact_to(1).unwrap(),
            None,
            "re-compact at the boundary is a no-op"
        );
        assert_eq!(
            c.compact_to(0).unwrap(),
            None,
            "below the boundary is a no-op"
        );
    }

    // ---- NodeCore::install_snapshot — lines 329 / 332 / 341 / 353 -----------

    #[test]
    fn install_snapshot_advances_seeds_state_and_commit_watermark() {
        let mut c = core(3);
        // A follower with an empty log installs a snapshot at boundary (3, 2) whose
        // captured state has keys {10,11}. The install must: reshape the log
        // (base → 4), reseed applied state, and advance last_applied + commit to 3.
        let mut snap_state = BookState::new();
        snap_state.apply(&BookUpdate::Set {
            key: 10,
            value: 100.0,
        });
        snap_state.apply(&BookUpdate::Set {
            key: 11,
            value: 110.0,
        });
        let snap = Snapshot::new(3, 2, snap_state);
        let advanced = c.install_snapshot(&snap).unwrap();
        assert_eq!(
            advanced,
            Some(3),
            "a fresh install advances to the boundary"
        );
        assert_eq!(c.log.snapshot_index(), Some(3));
        assert_eq!(c.last_applied, Some(3));
        assert_eq!(
            c.commit_index,
            Some(3),
            "commit watermark advanced to the boundary"
        );
        assert_eq!(
            c.applied.get(10).map(f64::to_bits),
            Some(100.0f64.to_bits())
        );
        assert_eq!(
            c.applied.get(11).map(f64::to_bits),
            Some(110.0f64.to_bits())
        );
    }

    #[test]
    fn install_snapshot_is_stale_when_boundary_already_covered() {
        let mut c = core(3);
        // Set up the state in which line 329 (`snapshot_index >= lii`) is the SOLE
        // guard that can reject a stale install: a durable snapshot boundary at 5
        // adopted on the log, but `last_applied` only at 2 (BELOW the boundary — so
        // the sibling line-332 `last_applied >= lii` guard would NOT fire for lii=3).
        // We save a recognizable durable @5 snapshot and adopt its boundary.
        let mut s5 = BookState::new();
        s5.apply(&BookUpdate::Set {
            key: 50,
            value: 5.0,
        });
        c.snapshots.save(&Snapshot::new(5, 2, s5)).unwrap();
        c.log.adopt_snapshot_boundary(5, 2); // snapshot_index = Some(5)
        c.last_applied = Some(2); // strictly below the boundary
        // A LOWER boundary (3) install is stale via line 329 `5 >= 3` true → None,
        // leaving the durable @5 snapshot untouched. The `>= with <` mutant (`5 < 3`
        // false) falls through line 329 AND line 332 (`2 >= 3` false), reaching
        // `snapshots.save(snap3)` — OVERWRITING the durable @5 snapshot with the older
        // @3 (before `Log::install_snapshot`'s own guard returns no-advance, so the
        // RETURN value is None either way). We assert the DURABLE snapshot content is
        // unchanged (still boundary 5 with key 50), which only the original preserves.
        let snap3 = Snapshot::new(3, 1, BookState::new());
        assert_eq!(
            c.install_snapshot(&snap3).unwrap(),
            None,
            "an older boundary is a stale no-op"
        );
        assert_eq!(
            c.log.snapshot_index(),
            Some(5),
            "boundary unchanged by a stale install"
        );
        let durable = c
            .snapshots
            .load()
            .unwrap()
            .expect("durable snapshot present");
        assert_eq!(
            durable.last_included_index, 5,
            "the durable snapshot must NOT be overwritten by the stale install (kills 329 `>=`->`<`)"
        );
        assert_eq!(
            durable.state.get(50).map(f64::to_bits),
            Some(5.0f64.to_bits()),
            "the durable @5 state survives the stale @3 install"
        );
    }

    #[test]
    fn install_snapshot_stale_when_already_applied_past_boundary() {
        // A follower that has already APPLIED past the boundary needs no install:
        // the `last_applied >= lii` guard (line 332) makes it a no-op. Build a node
        // that has applied through index 4, then install boundary 3.
        let mut c = core_with_committed(5); // applied 0..=4, single-node leader
        // (no prior snapshot; snapshot_index is None, so the line-329 guard passes.)
        assert_eq!(c.last_applied, Some(4));
        let snap = Snapshot::new(3, 1, BookState::new());
        assert_eq!(
            c.install_snapshot(&snap).unwrap(),
            None,
            "already applied past the boundary → stale no-op (kills the 332 `>=`->`<` flip)"
        );
        assert!(
            c.log.snapshot_index().is_none(),
            "no install happened, so no snapshot boundary was set"
        );
    }

    #[test]
    fn install_snapshot_matching_tail_retains_and_advances_commit_monotonically() {
        // A follower holds [0..=5] durably with commit ALREADY at 5 but last_applied
        // only at 2 (it has not yet replayed the committed tail — a legitimate
        // mid-recovery state). Installing a snapshot at boundary (3, term) that
        // MATCHES its own log entry-3 reaches the matching-tail path (line 332
        // `last_applied 2 >= 3` is false, so it proceeds): it discards only the prefix
        // and RETAINS the tail [4,5]. The `!advanced` guard (line 341) is false (work
        // was done → Some(3), not None), and the commit watermark must NOT regress
        // below the existing 5 — the `c >= lii` match (line 353) keeps Some(5). Kills
        // the line-341 `delete !` and the line-353 `c >= lii` true/false + `>=`->`<`.
        let mut c = core(3); // follower
        c.persist.save(1, None).unwrap();
        for i in 0..6u64 {
            c.log.append(&data_entry(1, i, i, i as f64)).unwrap();
        }
        c.commit_index = Some(5); // committed through 5 ...
        c.last_applied = Some(2); // ... but only applied through 2
        let boundary_term = c.log.term_at(3).unwrap();
        let snap = Snapshot::new(3, boundary_term, BookState::new());
        let advanced = c.install_snapshot(&snap).unwrap();
        assert_eq!(
            advanced,
            Some(3),
            "a matching-tail install advances (work was done)"
        );
        assert_eq!(
            c.log.snapshot_index(),
            Some(3),
            "prefix discarded to the boundary"
        );
        assert_eq!(
            c.log.last_index(),
            Some(5),
            "the matching tail [4,5] is retained"
        );
        assert_eq!(
            c.commit_index,
            Some(5),
            "commit watermark must NOT regress below the existing 5 (kills the 353 mutants)"
        );
    }

    #[test]
    fn install_snapshot_advances_commit_when_below_the_boundary() {
        // A follower whose commit_index is BELOW the snapshot boundary must advance
        // it UP to the boundary on install (the snapshot IS committed state through
        // lii). Line 353: `match commit_index { Some(c) if c >= lii => Some(c), _ =>
        // Some(lii) }`. With commit = Some(1) and lii = 3: `1 >= 3` is false → the
        // arm picks `Some(lii) = Some(3)` (advance). The `c >= lii with true` mutant
        // would (wrongly) keep `Some(c) = Some(1)`; the `>= with <` flip (`1 < 3`
        // true) would ALSO take the `Some(c)` arm and keep 1. Both are caught by
        // asserting the watermark advanced to 3.
        let mut c = core(3);
        c.commit_index = Some(1); // below the boundary we are about to install
        let snap = Snapshot::new(3, 1, BookState::new());
        assert_eq!(c.install_snapshot(&snap).unwrap(), Some(3));
        assert_eq!(
            c.commit_index,
            Some(3),
            "commit advances UP to the boundary when it was below (kills 353 `true`/`>=`->`<`)"
        );
    }

    // ---- single-node RaftNode accessors / lifecycle -------------------------
    //
    // A cluster_size==1 node commits on its own durable append (no peers to dial),
    // so these tests are fully deterministic with no socket timing.

    fn single_node() -> RaftNode {
        let path = temp_journal();
        let node = RaftNode::boot(path, &[], 1, RaftConfig::default()).unwrap();
        // A lone node elects itself leader within a tick; wait briefly.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !node.is_leader() {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(node.is_leader(), "a single-node cluster must self-elect");
        node
    }

    #[test]
    fn single_node_propose_commits_and_accessors_reflect_it() {
        let node = single_node();
        // propose: returns the assigned index (kills `propose == with !=` role guard —
        // a leader proposes Some(0), not None). last_log_index reflects the append
        // (kills `last_log_index -> None`/`Some(1)`).
        assert_eq!(node.last_log_index(), None, "fresh log is empty");
        let idx = node
            .propose(&BookUpdate::Set { key: 3, value: 9.0 })
            .unwrap();
        assert_eq!(
            idx,
            Some(0),
            "the leader assigns index 0 to the first proposal"
        );
        // wait_for_commit must observe the (immediate, single-node) commit — kills
        // the `< with <=` loop-bound and `>= with <` predicate mutants (a strict `<`
        // returning false would fail to confirm a real commit).
        assert!(
            node.wait_for_commit(0, Duration::from_secs(5)),
            "index 0 commits on a single node"
        );
        assert_eq!(node.last_log_index(), Some(0));
        // applied_state reflects the committed update (kills `applied_state ->
        // Default::default()`).
        assert_eq!(
            node.applied_state().get(3).map(f64::to_bits),
            Some(9.0f64.to_bits()),
            "the committed update is in the applied state"
        );
    }

    #[test]
    fn single_node_compact_applied_and_safe_index() {
        let node = single_node();
        for i in 0..4u64 {
            node.propose(&BookUpdate::Set {
                key: i,
                value: i as f64,
            })
            .unwrap();
        }
        assert!(node.wait_for_commit(3, Duration::from_secs(5)));
        // safe_compact_index on a lone node (no peers) is just last_applied = 3
        // (kills `safe_compact_index -> None`/`Some(0)`/`Some(1)`).
        assert_eq!(node.safe_compact_index(), Some(3));
        // compact_applied snapshots+discards through last_applied (3) and returns the
        // boundary (kills `compact_applied -> Ok(None)`/`Ok(Some(1))`).
        assert_eq!(node.compact_applied().unwrap(), Some(3));
        assert_eq!(node.snapshot_index(), Some(3));
        // Idempotent re-call: nothing new to compact → None (so the prior Some(3) was
        // a real boundary, not a constant).
        assert_eq!(node.compact_applied().unwrap(), None);
    }

    #[test]
    fn node_id_is_its_listen_port() {
        // `id()` returns the node's stable id (its listen port). Kills
        // `RaftNode::id -> 0`/`1`: the bound ephemeral port is neither 0 nor 1, and
        // `id()` must equal `addr().port()`.
        let node = single_node();
        assert_eq!(node.id(), u64::from(node.addr().port()));
        assert!(
            node.id() > 1,
            "an ephemeral port is far above the 0/1 constants"
        );
    }

    #[test]
    fn shutdown_and_drop_release_the_listen_port() {
        // The teardown (`shutdown` → `stop_and_join`, and the `Drop` backstop) must
        // STOP the serve thread, which owns the bound listener. We observe that
        // deterministically: after teardown the port is FREE, so a fresh bind on the
        // SAME address succeeds. A `shutdown -> ()` / `Drop::drop -> ()` no-op leaves
        // the serve thread alive holding the port, so the rebind fails — killing
        // those mutants without relying on thread-count introspection.
        let node = single_node(); // boots + waits for self-election
        let addr = node.addr();
        node.propose(&BookUpdate::Set { key: 1, value: 1.0 })
            .unwrap();
        assert!(node.wait_for_commit(0, Duration::from_secs(5)));
        node.shutdown(); // explicit join; the serve thread must exit and free the port.
        // Bind the exact same address — succeeds only if the serve thread released it.
        let rebind = std::net::TcpListener::bind(addr);
        assert!(
            rebind.is_ok(),
            "shutdown must stop the serve thread and free the listen port (kills `shutdown -> ()`)"
        );
        drop(rebind);

        // The Drop path (a node that is merely dropped, never `shutdown`) must do the
        // same — kills `Drop::drop -> ()`.
        let path2 = temp_journal();
        let node2 = RaftNode::boot(&path2, &[], 1, RaftConfig::default()).unwrap();
        let addr2 = node2.addr();
        drop(node2); // Drop::drop -> stop_and_join must free the port.
        assert!(
            std::net::TcpListener::bind(addr2).is_ok(),
            "Drop must stop the serve thread and free the listen port (kills `Drop::drop -> ()`)"
        );
    }

    #[test]
    fn boot_recovery_from_snapshot_does_not_double_apply_the_boundary() {
        // The boot-time tail-replay starts at `last_applied + 1` (line 510). When a
        // node recovers from a durable snapshot, `last_applied` is seeded to the
        // snapshot boundary, and ONLY entries strictly above it must be replayed — a
        // boundary entry already captured in the snapshot must NOT be re-applied. We
        // pin this end-to-end with `Add` updates (re-applying an Add would double it):
        // build a node, propose Adds, compact, reboot, and assert the recovered state
        // is bit-identical to the pre-reboot state. (The line-510 `a + 1` replay-start
        // arithmetic is provably EQUIVALENT under `*`/`-` — `entries_from` clamps a
        // sub-`base_index` request to physical position 0, so the exact start value is
        // immaterial as long as it is ≤ the retained range; that equivalence is
        // recorded in the gate config. This test still guards the broader
        // recovery-is-exact contract against any future regression.)
        let path = temp_journal();
        let expected_bits;
        {
            let node = RaftNode::boot(&path, &[], 1, RaftConfig::default()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && !node.is_leader() {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(node.is_leader());
            // Three Adds to the SAME key — the running total is order/idempotency
            // sensitive (re-applying any one changes the sum).
            node.propose(&BookUpdate::Add { key: 9, delta: 1.0 })
                .unwrap();
            node.propose(&BookUpdate::Add { key: 9, delta: 2.0 })
                .unwrap();
            node.propose(&BookUpdate::Add { key: 9, delta: 4.0 })
                .unwrap();
            assert!(node.wait_for_commit(2, Duration::from_secs(5)));
            // Compact at boundary 1 (so index 1's Add is captured in the snapshot and
            // its log body discarded; index 2 remains in the retained tail).
            assert_eq!(node.compact_applied().unwrap(), Some(2));
            expected_bits = node.applied_bits(); // key 9 == 1+2+4 = 7
            node.shutdown();
        }
        // Re-boot from the same durable path: seed from the snapshot, replay only the
        // retained tail above the boundary. The recovered state must be bit-identical.
        let recovered = RaftNode::boot(&path, &[], 1, RaftConfig::default()).unwrap();
        assert_eq!(
            recovered.applied_bits(),
            expected_bits,
            "recovery must not double-apply the snapshot boundary (kills 510 `+`->`*`/`-`)"
        );
        assert_eq!(
            recovered.applied_state().get(9).map(f64::to_bits),
            Some(7.0f64.to_bits()),
            "the Add total survives recovery exactly (1+2+4=7, not doubled)"
        );
        recovered.shutdown();
    }
}
