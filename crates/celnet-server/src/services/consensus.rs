//! ADR-0015 **consensus activation** — the runtime handle that wires the dormant,
//! already-complete `celnet-replog` Raft as the engine of the `Strong` consistency
//! tier. **No new consensus code lives here**: this module only boots the built-and-
//! validated [`celnet_replog::RaftNode`] and routes `Strong`-tier authoritative book
//! writes through its `propose` → quorum-commit path.
//!
//! **Wired everywhere, forced nowhere.** The state stores (the FX
//! [`PositionStore`](crate::services::risk::store::PositionStore) and the
//! [`RatesPositionStore`](crate::services::rates_book::RatesPositionStore)) hold an
//! `Option<Arc<ConsensusHandle>>`; a `None` handle — or a book that resolves to
//! [`ConsistencyLevel::Local`] — is the byte-identical single-node fast path.
//!
//! **HARD INVARIANT (ADR-0015 §4.3).** None of this ever touches the pinned pricing
//! thread. A `Strong` book's ms-scale quorum commit happens on the async booking /
//! state tier only; pricing, greeks, surface, and streaming stay µs regardless of any
//! book's level.
//!
//! ## Reconciling the two `BookState` types
//!
//! The live server book is [`celnet_engine::BookState`]-adjacent risk-fact state, while
//! the replicated applied state machine is the **disjoint** `celnet_replog::BookState`
//! (a deterministic `u64 → f64` priced book). Activation reconciles them by making the
//! replicated book a **faithful, deterministic, quorum-replicated projection** of the
//! server's live book keyed by `position_id → economic size`: a `Strong` booking
//! proposes a `BookUpdate::Set { key, value }` whose `value` is the position's
//! **must-order economic size** (FX signed base notional / rates signed linear PV01) —
//! deliberately **not** the derived mark, which is regenerable derived data (§4.3) that
//! a crashed `Local` book can always re-derive. The replicated book therefore survives
//! failover with the exact position sizes, bit-identically (the `to_bits` apply oracle),
//! while the un-replicated derived mark is regenerated on the recovering node. Full
//! unification of the risk-fact table onto the replicated log is the deferred follow-on.

// `tonic::Status` is the platform-standard edge error (the `Strong`-commit `unavailable`
// reject rides it, uniform with every service edge); a large `Err` variant is the house
// convention (see `services::rates_book` / `services::quote`), not boxed per call.
#![allow(clippy::result_large_err)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use celnet_replog::{BookUpdate, RaftConfig, RaftNode};

use crate::config::consistency::{ConsistencyLevel, ConsistencyPolicy};

/// The high-bit tag that keeps the FX and linear-rates `position_id` key ranges
/// **disjoint** in the one shared replicated `celnet_replog::BookState` (a `u64 → f64`
/// map). FX position ids fit `u32` (the risk-cube handle space), so tagging rates with
/// bit 63 guarantees no cross-book-family key collision.
const RATES_KEY_TAG: u64 = 1 << 63;

/// The replicated-book key for an FX position — its `position_id` unchanged (FX ids fit
/// `u32`, so they never intrude on the bit-63-tagged rates range).
#[must_use]
pub fn fx_book_key(position_id: u64) -> u64 {
    position_id
}

/// The replicated-book key for a linear-rates position — bit-63 tagged, disjoint from
/// the FX range.
#[must_use]
pub fn rates_book_key(position_id: u64) -> u64 {
    position_id | RATES_KEY_TAG
}

/// The activated consistency tier: one booted [`RaftNode`] plus the resolved
/// [`ConsistencyPolicy`]. Shared (behind an `Arc`) by every state store so **one** node
/// backs both the FX and the rates books (their key ranges are kept disjoint by
/// [`fx_book_key`] / [`rates_book_key`]). Constructed only when
/// [`ConsistencyPolicy::any_strong`] holds (see [`boot_if_strong`]) — a pure-`Local`
/// fleet never builds one, so there is zero overhead.
pub struct ConsensusHandle {
    raft: Arc<RaftNode>,
    policy: ConsistencyPolicy,
    commit_deadline: Duration,
    leader_deadline: Duration,
}

impl std::fmt::Debug for ConsensusHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsensusHandle")
            .field("node_id", &self.raft.id())
            .field("is_leader", &self.raft.is_leader())
            .field("commit_deadline", &self.commit_deadline)
            .finish_non_exhaustive()
    }
}

impl ConsensusHandle {
    /// Wrap a booted node + resolved policy into a shareable handle. `commit_deadline`
    /// bounds a `Strong` commit's wait for quorum; `leader_deadline` bounds the wait for
    /// leadership when a booking lands before this node has established it.
    #[must_use]
    pub fn new(
        raft: Arc<RaftNode>,
        policy: ConsistencyPolicy,
        commit_deadline: Duration,
        leader_deadline: Duration,
    ) -> Self {
        Self {
            raft,
            policy,
            commit_deadline,
            leader_deadline,
        }
    }

    /// The resolved consistency policy.
    #[must_use]
    pub fn policy(&self) -> &ConsistencyPolicy {
        &self.policy
    }

    /// The underlying Raft node — its `applied_state` is the cross-node bit-identity
    /// oracle a `Strong` write is committed into.
    #[must_use]
    pub fn raft(&self) -> &Arc<RaftNode> {
        &self.raft
    }

    /// The resolved level for an FX book name.
    #[must_use]
    pub fn level_for_book(&self, book: &str) -> ConsistencyLevel {
        self.policy.resolve_book(book)
    }

    /// The resolved level for a linear-rates book id.
    #[must_use]
    pub fn level_for_rates_book(&self, book: u32) -> ConsistencyLevel {
        self.policy.resolve_rates(book)
    }

    /// Commit a `Strong`-tier authoritative book write — a `Set { key, value }` — to the
    /// quorum log, blocking until it is committed and applied on this node. Runs on the
    /// async booking / state tier only, **never** the pinned pricing thread (§4.3).
    ///
    /// # Errors
    /// `unavailable` if this node is not (and cannot within the leader deadline become)
    /// the consensus leader, if the durable leader-append fails, or if the quorum commit
    /// does not reach a majority within the commit deadline. On any error the caller
    /// leaves the local book unmutated — a `Strong` book never silently degrades to
    /// un-replicated durability.
    pub fn commit_book_write(&self, key: u64, value: f64) -> Result<(), tonic::Status> {
        self.commit(BookUpdate::Set { key, value })
    }

    /// Commit a `Strong`-tier removal (a closed position) to the quorum log.
    ///
    /// # Errors
    /// As [`Self::commit_book_write`].
    pub fn commit_book_remove(&self, key: u64) -> Result<(), tonic::Status> {
        self.commit(BookUpdate::Remove { key })
    }

    /// Propose `update` on the leader and block until it commits.
    fn commit(&self, update: BookUpdate) -> Result<(), tonic::Status> {
        // A booking can land moments before this node has established leadership
        // (single-node self-election, or a fresh multi-node quorum still forming), so
        // give leadership a bounded chance rather than spuriously rejecting the first
        // Strong booking of a fresh cluster.
        let index = match self.propose_on_leader(&update)? {
            Some(index) => index,
            None => {
                if !self.wait_until_leader(self.leader_deadline) {
                    return Err(tonic::Status::unavailable(
                        "strong-tier consensus has no established leader on this node; \
                         retry the booking on the current leader",
                    ));
                }
                self.propose_on_leader(&update)?.ok_or_else(|| {
                    tonic::Status::unavailable(
                        "strong-tier consensus leadership was lost mid-commit; retry",
                    )
                })?
            }
        };
        if self.raft.wait_for_commit(index, self.commit_deadline) {
            Ok(())
        } else {
            Err(tonic::Status::unavailable(
                "strong-tier quorum commit did not reach a majority within the deadline",
            ))
        }
    }

    /// Propose on the leader, mapping a durable-append IO failure to `unavailable`.
    /// `Ok(None)` means this node is not currently the leader.
    fn propose_on_leader(&self, update: &BookUpdate) -> Result<Option<u64>, tonic::Status> {
        self.raft.propose(update).map_err(|e| {
            tonic::Status::unavailable(format!("strong-tier leader-append failed: {e}"))
        })
    }

    /// Poll until this node believes itself the leader, or `deadline` elapses.
    #[must_use]
    pub fn wait_until_leader(&self, deadline: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if self.raft.is_leader() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.raft.is_leader()
    }
}

/// The deploy-time knobs for booting the consensus tier (ADR-0015 §2.1), resolved from
/// the environment by [`Self::from_env`] mirroring the `CELNET_FLEET_*` discipline.
#[derive(Debug, Clone)]
pub struct ConsensusBoot {
    /// The durable Raft journal path; its sibling `.raft` (persistent state) and
    /// `.snapshot` files sit beside it.
    pub journal_path: PathBuf,
    /// The other nodes' Raft loopback / transport addresses. Empty ⇒ a single-node group
    /// (commits on its own durability — the byte-identical `InProcess` default).
    pub peers: Vec<SocketAddr>,
    /// How long a `Strong` commit waits for quorum before failing `unavailable`.
    pub commit_deadline: Duration,
    /// How long boot (and a first booking) waits for leadership to establish.
    pub leader_deadline: Duration,
}

impl ConsensusBoot {
    /// Resolve the boot knobs from the environment (read once, at boot):
    ///
    /// * the durable journal roots at `CELNET_RAFT_DIR`, else `<data_dir>/raft`, else a
    ///   CWD-relative `celnet-raft` fallback — `<root>/book.journal`;
    /// * `CELNET_RAFT_PEERS` — comma-separated peer `HOST:PORT` addresses (empty ⇒ a
    ///   single-node group);
    /// * `CELNET_RAFT_COMMIT_MS` — the `Strong`-commit quorum deadline (default 5000);
    /// * `CELNET_RAFT_LEADER_MS` — the leadership-establish deadline (default 3000).
    #[must_use]
    pub fn from_env(data_dir: Option<&Path>) -> Self {
        let root = std::env::var_os("CELNET_RAFT_DIR")
            .map(PathBuf::from)
            .or_else(|| data_dir.map(|d| d.join("raft")))
            .unwrap_or_else(|| PathBuf::from("celnet-raft"));
        let peers = std::env::var("CELNET_RAFT_PEERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<SocketAddr>().ok())
            .collect();
        Self {
            journal_path: root.join("book.journal"),
            peers,
            commit_deadline: Duration::from_millis(env_millis("CELNET_RAFT_COMMIT_MS", 5_000)),
            leader_deadline: Duration::from_millis(env_millis("CELNET_RAFT_LEADER_MS", 3_000)),
        }
    }
}

/// Boot the consensus tier **iff** the policy opts at least one book into `Strong`
/// (ADR-0015 §2.1 — zero overhead for a pure-`Local` fleet: no node is bound). Peers come
/// from `boot` (empty ⇒ a single-node group). Waits, bounded, for leadership so the first
/// `Strong` booking finds a leader.
///
/// This is a **blocking** call (durable journal open + socket bind + a bounded leader
/// wait); the async edge boot runs it via [`boot_if_strong_async`] so it never stalls the
/// tokio reactor.
///
/// # Errors
/// Propagates the durable-journal-open / persistent-state / socket-bind IO failure from
/// [`RaftNode::boot`], or a failure creating the journal's parent directory.
pub fn boot_if_strong(
    policy: ConsistencyPolicy,
    boot: ConsensusBoot,
) -> std::io::Result<Option<Arc<ConsensusHandle>>> {
    if !policy.any_strong() {
        return Ok(None);
    }
    if let Some(parent) = boot.journal_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cluster_size = 1 + boot.peers.len();
    let raft = RaftNode::boot(
        boot.journal_path,
        &boot.peers,
        cluster_size,
        RaftConfig::default(),
    )?;
    let handle = ConsensusHandle::new(
        Arc::new(raft),
        policy,
        boot.commit_deadline,
        boot.leader_deadline,
    );
    // Best-effort: let leadership establish before the edge begins booking Strong (a
    // single-node group self-elects; a multi-node group forms). A first booking that
    // still races ahead is handled by the bounded leader wait in `commit`.
    let _ = handle.wait_until_leader(boot.leader_deadline);
    Ok(Some(Arc::new(handle)))
}

/// The async wrapper for the edge boot path: runs the blocking [`boot_if_strong`] on a
/// blocking thread so the socket bind + bounded leader wait never stall the tokio
/// reactor.
///
/// # Errors
/// Propagates the [`boot_if_strong`] IO failure (or a join failure).
pub async fn boot_if_strong_async(
    policy: ConsistencyPolicy,
    boot: ConsensusBoot,
) -> std::io::Result<Option<Arc<ConsensusHandle>>> {
    tokio::task::spawn_blocking(move || boot_if_strong(policy, boot))
        .await
        .map_err(std::io::Error::other)?
}

/// Read a millisecond duration from an environment variable, falling back to `default`.
fn env_millis(var: &str, default: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(default)
}
