//! A follower node: a real TCP server that durably mirrors the leader's log.
//!
//! A [`Follower`] binds a loopback [`TcpListener`] on an **ephemeral port**, then
//! a background thread accepts one leader connection and services its frames:
//!
//! * [`Message::Append`] → validate the index is the expected next one, append
//!   the entry to the local [`Journal`] (durable `fsync`), advance the durable
//!   high-water `match_index`, advance the applied state machine for every entry
//!   now at-or-below the piggybacked `leader_commit`, then reply [`Message::Ack`].
//! * [`Message::StatusRequest`] → reply [`Message::Status`] with the durable
//!   high-water index and term (used by a recovering/standby node).
//!
//! The follower's journal **is** its source of truth: on Append it persists
//! before acking, so a leader that has counted this follower's ack knows the
//! entry is on stable storage here. The applied [`BookState`] is derived purely
//! by replaying committed entries, so it is bit-identical to the leader's.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use celnet_journal::Journal;

use crate::entry::LogEntry;
use crate::state::{BookState, BookUpdate};
use crate::wire::{EMPTY_LOG, FrameRead, Message, read_frame_or_idle, write_frame};

/// Shared, observable state of a follower, readable by the owning test/harness
/// and by a standby that promotes this node.
#[derive(Debug)]
pub struct FollowerShared {
    /// Durable high-water index: the highest log index this node has `fsync`'d.
    /// `EMPTY_LOG` (`u64::MAX`) until the first entry lands.
    match_index: AtomicU64,
    /// Commit index: the highest index known quorum-committed (≤ `match_index`),
    /// learned from the leader's piggybacked `leader_commit`.
    commit_index: AtomicU64,
    /// Current term acknowledged.
    term: AtomicU64,
    /// The applied, committed [`BookState`] (bit-identity oracle).
    applied: Mutex<BookState>,
    /// Set to request the accept/serve loop to stop.
    stop: AtomicBool,
}

impl FollowerShared {
    fn new() -> Self {
        Self {
            match_index: AtomicU64::new(EMPTY_LOG),
            commit_index: AtomicU64::new(EMPTY_LOG),
            term: AtomicU64::new(0),
            applied: Mutex::new(BookState::new()),
            stop: AtomicBool::new(false),
        }
    }

    /// The durable high-water index, or `None` if the log is empty.
    #[must_use]
    pub fn match_index(&self) -> Option<u64> {
        match self.match_index.load(Ordering::Acquire) {
            EMPTY_LOG => None,
            n => Some(n),
        }
    }

    /// The highest quorum-committed index this follower has learned, or `None`.
    #[must_use]
    pub fn commit_index(&self) -> Option<u64> {
        match self.commit_index.load(Ordering::Acquire) {
            EMPTY_LOG => None,
            n => Some(n),
        }
    }

    /// The current acknowledged term.
    #[must_use]
    pub fn term(&self) -> u64 {
        self.term.load(Ordering::Acquire)
    }

    /// A snapshot clone of the applied committed [`BookState`].
    #[must_use]
    pub fn applied_state(&self) -> BookState {
        self.applied.lock().expect("applied lock").clone()
    }

    /// The `to_bits` digest of the applied committed state — the cross-node
    /// bit-identity oracle.
    #[must_use]
    pub fn applied_bits(&self) -> Vec<(u64, u64)> {
        self.applied.lock().expect("applied lock").to_bits()
    }
}

/// A running follower node bound to a real loopback socket.
pub struct Follower {
    addr: SocketAddr,
    path: PathBuf,
    shared: Arc<FollowerShared>,
    handle: Option<JoinHandle<()>>,
}

impl Follower {
    /// Boot a follower: open (or recover) its journal at `journal_path`, bind a
    /// loopback listener on an **ephemeral** port, and spawn the serve thread.
    ///
    /// On boot the journal is replayed to rebuild the durable high-water index
    /// and the applied state, so a restarted node resumes from disk
    /// (crash-recovery). Committed-vs-uncommitted is re-established by the leader
    /// on reconnect (it re-streams from the follower's reported high-water).
    ///
    /// # Errors
    ///
    /// Propagates journal-open and socket-bind IO failures.
    pub fn boot(journal_path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = journal_path.into();
        let shared = Arc::new(FollowerShared::new());

        // Recover durable state from the journal (torn-tail-healed by `open`).
        let journal = Journal::open(&path).map_err(to_io)?;
        if let Some(last) = journal.last_sequence() {
            shared.match_index.store(last, Ordering::Release);
            // On a clean restart every durably-appended entry was, by the
            // leader-replicated protocol, part of the replicated stream. We
            // rebuild the applied state from the full durable prefix and treat it
            // as the recovered committed watermark; the leader re-confirms on
            // reconnect. (Uncommitted-tail handling is documented in the crate root.)
            let mut state = shared.applied.lock().expect("applied lock");
            journal
                .replay(|rec| {
                    if let Ok(entry) = LogEntry::decode(&rec.payload)
                        && let Ok(upd) = BookUpdate::decode(&entry.payload)
                    {
                        state.apply(&upd);
                    }
                })
                .map_err(to_io)?;
            shared.commit_index.store(last, Ordering::Release);
        }

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;

        let serve_shared = Arc::clone(&shared);
        let serve_path = path.clone();
        let handle = thread::Builder::new()
            .name("replog-follower".into())
            .spawn(move || serve(listener, serve_path, serve_shared))
            .expect("spawn follower serve thread");

        Ok(Self {
            addr,
            path,
            shared,
            handle: Some(handle),
        })
    }

    /// The ephemeral loopback address the leader dials.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The journal path (for restart / inspection).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Shared, observable state handle.
    #[must_use]
    pub fn shared(&self) -> Arc<FollowerShared> {
        Arc::clone(&self.shared)
    }

    /// Read every durably-stored entry's encoded bytes back from the journal —
    /// the on-disk committed log, for byte-identical cross-node comparison.
    ///
    /// # Errors
    ///
    /// Propagates journal IO errors.
    pub fn journal_bytes(&self) -> std::io::Result<Vec<Vec<u8>>> {
        journal_entry_bytes(&self.path)
    }

    /// Signal the serve loop to stop and join it (a clean, deadline-free local
    /// teardown — the listener close unblocks the accept).
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        // Nudge the accept loop by connecting to ourselves so a blocked
        // `accept()` returns and observes the stop flag.
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        if self.handle.is_some() {
            self.stop_and_join();
        }
    }
}

/// Read back the encoded entry bytes of a journal at `path` (the durable log).
pub(crate) fn journal_entry_bytes(path: &Path) -> std::io::Result<Vec<Vec<u8>>> {
    let journal = Journal::open(path).map_err(to_io)?;
    let recs = journal.records().map_err(to_io)?;
    Ok(recs.into_iter().map(|r| r.payload).collect())
}

/// The follower serve loop: accept one connection at a time and service frames
/// until the peer disconnects or a stop is requested, then accept the next
/// (so a leader that reconnects after a transport blip is handled).
fn serve(listener: TcpListener, path: PathBuf, shared: Arc<FollowerShared>) {
    // Each accepted connection re-opens the journal append handle; the journal's
    // single-writer discipline is preserved because we serve one connection at a
    // time and never two writers concurrently.
    while !shared.stop.load(Ordering::Acquire) {
        let conn = match listener.accept() {
            Ok((conn, _)) => conn,
            Err(_) => break,
        };
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        // A short read timeout makes the per-connection serve loop responsive to
        // the stop flag and to a silently-vanished peer (it returns to re-check
        // rather than blocking forever). It is NOT a synchronisation crutch: the
        // loop loops on `WouldBlock`/`TimedOut`, it does not assume progress.
        let _ = conn.set_read_timeout(Some(Duration::from_millis(200)));
        // A fresh journal handle positioned at the durable end.
        let journal = match Journal::open(&path) {
            Ok(j) => j,
            Err(_) => continue,
        };
        serve_conn(conn, journal, &path, &shared);
    }
}

/// Service one leader connection: durably apply Appends, answer Status.
fn serve_conn(
    mut conn: TcpStream,
    mut journal: Journal,
    path: &Path,
    shared: &Arc<FollowerShared>,
) {
    loop {
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        let msg = match read_frame_or_idle(&mut conn) {
            // Idle read window elapsed at a frame boundary: loop to re-check the
            // stop flag. Liveness for teardown, not an assumption of progress.
            Ok(FrameRead::Idle) => continue,
            Ok(FrameRead::Frame(m)) => m,
            // Peer disconnect or transport error: return to the accept loop.
            Err(_) => return,
        };
        match msg {
            Message::Append {
                entry,
                leader_commit,
            } => {
                let expected = match shared.match_index.load(Ordering::Acquire) {
                    EMPTY_LOG => 0,
                    n => n + 1,
                };
                // Strict in-order replication: the leader streams entries in
                // index order. An already-held index (< expected) is idempotently
                // re-acked, not re-appended; a forward gap is refused and the
                // leader resyncs from our Status (no silent divergence).
                if entry.index < expected {
                    let _ = write_frame(
                        &mut conn,
                        &Message::Ack {
                            match_index: shared.match_index.load(Ordering::Acquire),
                            term: shared.term.load(Ordering::Acquire),
                        },
                    );
                    continue;
                }
                if entry.index != expected {
                    let _ = write_frame(&mut conn, &status_msg(shared));
                    continue;
                }
                // Durably append the entry's canonical bytes. The journal assigns
                // sequence == entry.index by construction (in-order, from 0).
                let bytes = entry.encode();
                if journal.append(&bytes).is_err() {
                    // IO failure: do not ack (the leader will not count us).
                    return;
                }
                shared.term.store(entry.term, Ordering::Release);
                shared.match_index.store(entry.index, Ordering::Release);
                advance_commit(path, shared, leader_commit);
                let _ = write_frame(
                    &mut conn,
                    &Message::Ack {
                        match_index: entry.index,
                        term: entry.term,
                    },
                );
            }
            Message::Commit { leader_commit } => {
                // Advance the applied/commit watermark for the just-committed tail
                // (the entry whose Append predated the commit it enabled) and ack.
                advance_commit(path, shared, leader_commit);
                let _ = write_frame(
                    &mut conn,
                    &Message::Ack {
                        match_index: shared.match_index.load(Ordering::Acquire),
                        term: shared.term.load(Ordering::Acquire),
                    },
                );
            }
            Message::StatusRequest => {
                if write_frame(&mut conn, &status_msg(shared)).is_err() {
                    return;
                }
            }
            // A follower never receives Ack/Status as a request; ignore.
            Message::Ack { .. } | Message::Status { .. } => {}
        }
    }
}

/// Advance the applied state machine to cover every entry at-or-below
/// `leader_commit` that this follower durably holds, replaying the newly
/// committed slice from the durable journal so applied state is built purely
/// from persisted, ordered entries.
fn advance_commit(path: &Path, shared: &Arc<FollowerShared>, leader_commit: u64) {
    if leader_commit == EMPTY_LOG {
        return;
    }
    let durable = match shared.match_index.load(Ordering::Acquire) {
        EMPTY_LOG => return,
        n => n,
    };
    let target = leader_commit.min(durable);
    let start = match shared.commit_index.load(Ordering::Acquire) {
        EMPTY_LOG => 0,
        c if c >= target => return, // nothing new
        c => c + 1,
    };
    let mut state = shared.applied.lock().expect("applied lock");
    let journal = match Journal::open(path) {
        Ok(j) => j,
        Err(_) => return,
    };
    let _ = journal.replay(|rec| {
        if let Ok(entry) = LogEntry::decode(&rec.payload)
            && entry.index >= start
            && entry.index <= target
            && let Ok(upd) = BookUpdate::decode(&entry.payload)
        {
            state.apply(&upd);
        }
    });
    shared.commit_index.store(target, Ordering::Release);
}

fn status_msg(shared: &Arc<FollowerShared>) -> Message {
    Message::Status {
        last_index: shared.match_index.load(Ordering::Acquire),
        term: shared.term.load(Ordering::Acquire),
    }
}

fn to_io<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
