//! The leader node: durably appends, replicates to followers over real sockets,
//! and commits an entry **only on quorum durability**.
//!
//! # Protocol (thin leader-replicated, explicitly *before* Raft election)
//!
//! 1. A client calls [`Leader::propose`] with a [`BookUpdate`].
//! 2. The leader stamps it `(term, next_index)`, durably appends it to its own
//!    [`Journal`] (the leader itself counts as one durable copy), and broadcasts
//!    a [`Message::Append`] to every connected follower.
//! 3. Each follower durably appends and replies [`Message::Ack`].
//! 4. The leader counts acks. The entry is **committed** once a **majority of the
//!    cluster** (leader + followers) durably holds it: `acks + 1 (self) >
//!    cluster_size / 2`. The commit index advances monotonically to the highest
//!    such index.
//! 5. The leader applies committed entries to its own [`BookState`] and
//!    piggybacks the new commit index on the next `Append` so followers apply too.
//!
//! A **stalled minority never advances the commit index**: with `f` of `2f+1`
//! nodes reachable where `f < f+1`, `propose` returns `Committed(false)` — the
//! entry is durable on the leader but *not* reported committed, and the applied
//! state does not include it. This is the safety property gate (b) asserts.
//!
//! Leader **election** (auto-advancing the term, voting) is the next increment
//! and is deliberately *not* built here; a leader is constructed explicitly with
//! its term, and [`crate::standby`] performs an operator/standby-driven promotion.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use celnet_journal::Journal;

use crate::entry::{Index, LogEntry, Term};
use crate::follower::journal_entry_bytes;
use crate::state::{BookState, BookUpdate};
use crate::wire::{EMPTY_LOG, Message, read_frame, write_frame};

/// A live connection to one follower, plus that follower's last-known durable
/// high-water index.
struct FollowerLink {
    /// The current real loopback socket to the follower, or `None` when the link
    /// is down (a write/read error, or never dialled). A down link does not count
    /// toward quorum — modelling a partition/crash — and is re-dialled by address
    /// before the next replicate round.
    stream: Option<TcpStream>,
    /// The follower's address (for diagnostics / reconnection).
    addr: std::net::SocketAddr,
    /// Highest index this follower has acked as durable, or `None`.
    match_index: Option<Index>,
}

impl FollowerLink {
    /// Whether the link currently holds a usable socket.
    fn alive(&self) -> bool {
        self.stream.is_some()
    }
}

/// The outcome of a [`Leader::propose`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProposeOutcome {
    /// The index assigned to the proposed entry.
    pub index: Index,
    /// `true` iff the entry reached **quorum durability** and is committed.
    /// `false` means it is durable on the leader but a majority did not ack
    /// (e.g. a partition that lost quorum) — the safety property: no false progress.
    pub committed: bool,
}

/// The leader node.
pub struct Leader {
    journal: Journal,
    path: PathBuf,
    term: Term,
    /// Cluster size = leader (1) + the followers it was constructed with. Used
    /// for the strict-majority quorum threshold; fixed for a leader's tenure
    /// (membership change is a separate concern, documented in the crate root).
    cluster_size: usize,
    followers: Vec<FollowerLink>,
    /// Next index to assign.
    next_index: Index,
    /// Highest committed (quorum-durable) index, or `None`.
    commit_index: Option<Index>,
    /// The leader's own applied committed state.
    applied: BookState,
    /// Per-RPC socket deadline so a dead peer fails fast instead of hanging.
    io_timeout: Duration,
}

impl Leader {
    /// Construct a leader at `journal_path`, in `term`, replicating to the
    /// followers at `follower_addrs`, for a cluster of `cluster_size` total nodes
    /// (must be `>= 1 + follower_addrs.len()`; typically exactly that).
    ///
    /// The leader dials each follower over a **real loopback TCP socket** and
    /// recovers its own durable log from its journal. `io_timeout` bounds every
    /// socket op so an unreachable peer fails fast (never hangs) — the deadline
    /// that makes the quorum/partition tests fail loudly rather than block.
    ///
    /// # Errors
    ///
    /// Propagates journal-open IO errors. A follower that cannot be dialled is
    /// recorded as a dead link (does not abort construction) so a degraded
    /// cluster still forms — quorum is then evaluated over the live majority.
    pub fn boot(
        journal_path: impl Into<PathBuf>,
        term: Term,
        follower_addrs: &[std::net::SocketAddr],
        cluster_size: usize,
        io_timeout: Duration,
    ) -> std::io::Result<Self> {
        assert!(
            cluster_size > follower_addrs.len(),
            "cluster_size must include the leader and all followers"
        );
        let path = journal_path.into();
        let journal = Journal::open(&path).map_err(to_io)?;
        let next_index = journal.next_sequence();
        let commit_index = journal.last_sequence();

        // Rebuild the leader's applied state from its durable log.
        let mut applied = BookState::new();
        journal
            .replay(|rec| {
                if let Ok(entry) = LogEntry::decode(&rec.payload)
                    && let Ok(upd) = BookUpdate::decode(&entry.payload)
                {
                    applied.apply(&upd);
                }
            })
            .map_err(to_io)?;

        let mut followers = Vec::with_capacity(follower_addrs.len());
        for &addr in follower_addrs {
            // An unreachable follower is recorded as a down link (no stream); it
            // is re-dialled before each replicate round, so a degraded cluster
            // still forms and a healed follower rejoins.
            let stream = dial(addr, io_timeout).ok();
            followers.push(FollowerLink {
                stream,
                addr,
                match_index: None,
            });
        }

        Ok(Self {
            journal,
            path,
            term,
            cluster_size,
            followers,
            next_index,
            commit_index,
            applied,
            io_timeout,
        })
    }

    /// The leadership term.
    #[must_use]
    pub fn term(&self) -> Term {
        self.term
    }

    /// The highest committed (quorum-durable) index, or `None`.
    #[must_use]
    pub fn commit_index(&self) -> Option<Index> {
        self.commit_index
    }

    /// The next index to be assigned.
    #[must_use]
    pub fn next_index(&self) -> Index {
        self.next_index
    }

    /// A clone of the leader's applied committed [`BookState`].
    #[must_use]
    pub fn applied_state(&self) -> BookState {
        self.applied.clone()
    }

    /// The `to_bits` digest of the applied committed state.
    #[must_use]
    pub fn applied_bits(&self) -> Vec<(u64, u64)> {
        self.applied.to_bits()
    }

    /// The encoded entry bytes of the leader's durable log (for byte comparison).
    ///
    /// # Errors
    ///
    /// Propagates journal IO errors.
    pub fn journal_bytes(&self) -> std::io::Result<Vec<Vec<u8>>> {
        journal_entry_bytes(&self.path)
    }

    /// Propose one [`BookUpdate`]: stamp it, durably append it locally, replicate
    /// to all live followers, and commit iff a strict majority durably holds it.
    ///
    /// Returns the assigned index and whether it committed. A non-committing
    /// propose (lost quorum) leaves the entry durable on the leader but does
    /// **not** advance the commit index or applied state — the no-false-progress
    /// guarantee.
    ///
    /// # Errors
    ///
    /// Propagates a leader-local journal append IO failure (the only fatal case;
    /// follower IO failures merely mark a link dead and reduce the live count).
    pub fn propose(&mut self, update: &BookUpdate) -> std::io::Result<ProposeOutcome> {
        let index = self.next_index;
        let entry = LogEntry::new(self.term, index, update.encode());

        // 1) Durable on the leader first (the leader is one durable copy).
        self.journal.append(&entry.encode()).map_err(to_io)?;
        self.next_index += 1;

        // 2) Replicate to every live follower and gather fresh acks.
        let leader_commit = self.commit_index.unwrap_or(EMPTY_LOG);
        self.replicate(&entry, leader_commit);

        // 3) Recompute the commit index from quorum durability.
        let prev_commit = self.commit_index;
        self.recompute_commit();

        // 4) If the commit index advanced, broadcast it so followers apply the
        //    just-committed tail (the entry whose Append predated this commit).
        if self.commit_index != prev_commit
            && let Some(c) = self.commit_index
        {
            self.broadcast_commit(c);
        }

        let committed = self.commit_index.is_some_and(|c| c >= index);
        Ok(ProposeOutcome { index, committed })
    }

    /// Send `entry` to each live follower and record its ack (durable high-water).
    fn replicate(&mut self, entry: &LogEntry, leader_commit: u64) {
        let msg = Message::Append {
            entry: entry.clone(),
            leader_commit,
        };
        let timeout = self.io_timeout;
        for link in &mut self.followers {
            if !link.alive() {
                // Best-effort revive: try to re-dial a previously-down follower so
                // a healed partition rejoins (no fake — a real new socket).
                match dial(link.addr, timeout) {
                    Ok(stream) => link.stream = Some(stream),
                    Err(_) => continue,
                }
            }
            let stream = link.stream.as_mut().expect("alive link has a stream");
            if write_frame(stream, &msg).is_err() {
                link.stream = None;
                continue;
            }
            match read_frame(stream) {
                Ok(Message::Ack { match_index, term }) if term <= self.term => {
                    link.match_index = Some(match_index);
                }
                Ok(Message::Status { last_index, .. }) => {
                    // The follower refused a gapped append and told us its
                    // high-water; record it so a future resync can backfill.
                    link.match_index = match last_index {
                        EMPTY_LOG => None,
                        n => Some(n),
                    };
                }
                // Any other reply or an IO error marks the link down this round.
                _ => link.stream = None,
            }
        }
    }

    /// Broadcast the current commit index to every live follower so they apply
    /// the committed tail. Best-effort: a follower that misses this still applies
    /// the commit on the next `Append`'s piggybacked `leader_commit`.
    fn broadcast_commit(&mut self, leader_commit: u64) {
        let msg = Message::Commit { leader_commit };
        for link in &mut self.followers {
            let Some(stream) = link.stream.as_mut() else {
                continue;
            };
            if write_frame(stream, &msg).is_err() {
                link.stream = None;
                continue;
            }
            match read_frame(stream) {
                Ok(Message::Ack { match_index, .. }) => link.match_index = Some(match_index),
                Ok(_) => {}
                Err(_) => link.stream = None,
            }
        }
    }

    /// Advance `commit_index` to the highest index that a strict majority of the
    /// cluster (leader self + acked followers) durably holds.
    fn recompute_commit(&mut self) {
        let majority = self.cluster_size / 2 + 1;
        // For each candidate index from the highest down to just past the current
        // commit, count durable holders. The leader holds every index it has
        // appended (`< next_index`).
        let highest = match self.next_index.checked_sub(1) {
            Some(h) => h,
            None => return, // nothing appended yet
        };
        let start_at = self.commit_index.map_or(0, |c| c + 1);
        // Walk upward; commit is monotone so the first index that fails to reach
        // quorum stops the advance (an index can only be committed if all prior
        // are — guaranteed by in-order replication).
        let mut new_commit = self.commit_index;
        for idx in start_at..=highest {
            let mut holders = 1usize; // the leader itself
            for link in &self.followers {
                if link.alive() && link.match_index.is_some_and(|m| m >= idx) {
                    holders += 1;
                }
            }
            if holders >= majority {
                new_commit = Some(idx);
            } else {
                break;
            }
        }
        if new_commit != self.commit_index {
            self.apply_through(new_commit);
            self.commit_index = new_commit;
        }
    }

    /// Apply committed entries `(old_commit, target]` to the leader's state by
    /// replaying the durable journal (state built purely from persisted entries).
    fn apply_through(&mut self, target: Option<Index>) {
        let Some(target) = target else { return };
        let start = self.commit_index.map_or(0, |c| c + 1);
        if start > target {
            return;
        }
        let journal = match Journal::open(&self.path) {
            Ok(j) => j,
            Err(_) => return,
        };
        let applied = &mut self.applied;
        let _ = journal.replay(|rec| {
            if let Ok(entry) = LogEntry::decode(&rec.payload)
                && entry.index >= start
                && entry.index <= target
                && let Ok(upd) = BookUpdate::decode(&entry.payload)
            {
                applied.apply(&upd);
            }
        });
    }

    /// Number of currently-live follower links (for tests/observability).
    #[must_use]
    pub fn live_followers(&self) -> usize {
        self.followers.iter().filter(|l| l.alive()).count()
    }
}

/// Dial a follower over a real loopback TCP socket with a connect+IO deadline.
fn dial(addr: std::net::SocketAddr, timeout: Duration) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

fn to_io<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Read back a follower's (or any node's) durable entry bytes from its journal
/// path — exposed for cross-node byte-identity assertions in tests.
///
/// # Errors
///
/// Propagates journal IO errors.
pub fn durable_entry_bytes(path: &Path) -> std::io::Result<Vec<Vec<u8>>> {
    journal_entry_bytes(path)
}
