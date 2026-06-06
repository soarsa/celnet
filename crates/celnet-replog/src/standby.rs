//! Hot-standby pre-warm + operator-driven failover (promotion to leader).
//!
//! A **hot standby** is an ordinary [`crate::follower::Follower`] that has been
//! kept caught-up by the leader's replication stream. Because every committed
//! (quorum-acked) entry is durably on a majority — including, in a healthy
//! cluster, the standby — a standby whose durable high-water has reached the
//! cluster's committed index can take over with **zero loss of committed
//! entries**.
//!
//! # Promotion procedure (and what happens to in-flight entries)
//!
//! [`promote`] takes a caught-up follower's journal path and the *surviving*
//! followers' addresses and constructs a new [`crate::leader::Leader`] in a
//! **higher term**. The new leader's durable log is its own journal — which
//! already contains every committed entry (the standby held them) — so:
//!
//! * **Committed entries are preserved exactly** (they are on the promoted
//!   node's disk and re-counted as durable at promotion).
//! * **Uncommitted in-flight entries** (durable on the old leader but never
//!   quorum-acked) are handled per leader-replicated rules: if the promoted
//!   node happened to hold such a tail entry it remains in its log and will be
//!   *re-proposed under the new term* (re-replicated to the surviving quorum);
//!   if it did not, that never-committed entry is simply absent — it was never
//!   acknowledged to any client, so dropping it is correct. This crate
//!   re-replicates the promoted node's whole durable prefix on the first
//!   post-promotion proposals, so the surviving followers converge to the new
//!   leader's log. (Full conflicting-tail truncation across divergent followers
//!   is a Raft-election concern, documented as the next increment in the crate
//!   root — not half-built here.)
//!
//! The promotion is **bounded**: constructing the new leader dials the survivors
//! with the same `io_timeout`, so an unreachable survivor fails fast rather than
//! hanging the takeover.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use crate::entry::Term;
use crate::leader::Leader;

/// Promote a caught-up standby (identified by its journal `path` and prior
/// `old_term`) to leader of the surviving cluster.
///
/// `survivor_addrs` are the loopback addresses of the followers that remain after
/// the old leader was lost. `cluster_size` is the size of the *new* cluster
/// (`1 + survivor_addrs.len()`), over which the new leader evaluates quorum.
/// `io_timeout` bounds every dial/RPC so the takeover is deadline-bounded.
///
/// The new leader runs in `old_term + 1`, the standard term bump that fences the
/// failed leader: any late ack from the old term is rejected by followers.
///
/// # Errors
///
/// Propagates [`Leader::boot`] IO errors (journal open). An unreachable survivor
/// becomes a down link inside the new leader (does not abort promotion), so the
/// takeover succeeds as long as the promoted node's own log is readable.
pub fn promote(
    path: &Path,
    old_term: Term,
    survivor_addrs: &[SocketAddr],
    cluster_size: usize,
    io_timeout: Duration,
) -> std::io::Result<Leader> {
    Leader::boot(
        path.to_path_buf(),
        old_term + 1,
        survivor_addrs,
        cluster_size,
        io_timeout,
    )
}

/// Whether a follower is sufficiently caught up to be promoted without losing
/// committed entries: its durable high-water must be at least the cluster's
/// last known committed index.
///
/// This is the pre-warm gate an operator (or an automatic supervisor, the next
/// increment) checks before [`promote`]. `follower_high_water` is `None` for an
/// empty log; `committed_index` is `None` when nothing has committed yet (in
/// which case any follower is trivially caught up).
#[must_use]
pub fn is_caught_up(follower_high_water: Option<u64>, committed_index: Option<u64>) -> bool {
    match (committed_index, follower_high_water) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(c), Some(h)) => h >= c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caught_up_gate() {
        // Args are (follower_high_water, committed_index).
        assert!(is_caught_up(None, None)); // nothing committed → trivially caught up
        assert!(is_caught_up(Some(5), Some(5))); // exactly at the committed index
        assert!(is_caught_up(Some(7), Some(5))); // ahead of the committed index
        assert!(!is_caught_up(Some(4), Some(5))); // behind the committed index
        assert!(!is_caught_up(None, Some(0))); // empty log but something committed
    }
}
