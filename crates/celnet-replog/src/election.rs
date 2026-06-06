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
        let log = Log::open(&path)?;
        let persist = PersistStore::open(persist_path(&path))?;

        let addr = listener.local_addr()?;
        let id = u64::from(addr.port());

        // Recover the applied state from the DURABLE COMMITTED prefix only. The
        // commit-index watermark is persisted (Raft §5 + our durable-commit design),
        // so a restarted node knows exactly which prefix is committed — and only the
        // *uncommitted* tail above it is eligible for conflicting-tail truncation.
        // Entries above the watermark are NOT applied on boot; the leader re-drives
        // their commit via AppendEntries, and apply is monotone.
        let mut applied = BookState::new();
        let mut last_applied = None;
        let commit_index = match persist.commit_index() {
            // Clamp the durable watermark to what the (possibly torn-tail-healed)
            // log actually holds, then apply that committed prefix.
            Some(c) => log.last_index().map(|last| c.min(last)),
            None => None,
        };
        if let Some(commit) = commit_index {
            for e in log.entries_from(0)? {
                if e.index > commit {
                    break;
                }
                if let Ok(upd) = BookUpdate::decode(&e.payload) {
                    applied.apply(&upd);
                }
                last_applied = Some(e.index);
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
        let mut plans = Vec::new();
        // Gather per-peer plans; reading entries from the log needs the lock.
        let peer_ids: Vec<u64> = c.peers.keys().copied().collect();
        for pid in peer_ids {
            let next_index = c.peers[&pid].next_index;
            let addr = c.peers[&pid].addr;
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
