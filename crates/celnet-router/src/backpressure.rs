//! Bounded backpressure — a per-replica inflight cap.
//!
//! `docs/SCALE-OUT.md` §6 mandates that a slow downstream **never** unbounded-
//! queues nor back-pressures the pricing core: the edge sheds instead. This
//! module is the routing-tier expression of that rule — each replica has a hard
//! cap on concurrently in-flight requests; admission past the cap returns a
//! typed [`Admission::Shed`] so the caller can fail fast (or re-route), never an
//! unbounded queue.
//!
//! The limiter is a flat array of atomic counters keyed by replica index, so
//! `try_admit`/`complete` are lock-free, allocation-free, and safe to call
//! concurrently from many router threads. A successful admission yields a
//! [`Permit`] RAII guard that decrements the counter on drop, so an in-flight
//! count cannot leak even on an early return or panic in the caller.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::replica::{ReplicaId, ReplicaSet};

/// The result of asking to admit one request to a replica.
#[derive(Debug)]
pub enum Admission {
    /// Admitted; hold the [`Permit`] for the request's lifetime.
    Admitted(Permit),
    /// Rejected because the replica is at its inflight cap. Carries the cap so
    /// the caller can surface a precise reason.
    Shed {
        /// The replica that shed the request.
        replica: ReplicaId,
        /// The configured inflight cap that was hit.
        cap: u32,
    },
}

impl Admission {
    /// Whether the request was admitted.
    #[must_use]
    pub const fn is_admitted(&self) -> bool {
        matches!(self, Admission::Admitted(_))
    }
}

/// A per-replica inflight limiter.
///
/// One counter per replica, capped at a configured maximum. Clone-cheap
/// (`Arc`-backed) so every router thread shares the same counters.
#[derive(Debug, Clone)]
pub struct InflightLimiter {
    inner: Arc<LimiterInner>,
}

#[derive(Debug)]
struct LimiterInner {
    cap: u32,
    /// `ids[i]` is the replica whose inflight count is `counts[i]`.
    ids: Vec<ReplicaId>,
    counts: Vec<AtomicU32>,
}

impl InflightLimiter {
    /// A limiter for the given replica set with a uniform per-replica `cap`.
    ///
    /// The slot layout is fixed from the set at construction; rebuild the
    /// limiter when membership changes (a router holds one limiter per live
    /// membership version, alongside the [`crate::PartitionMap`]).
    #[must_use]
    pub fn new(set: &ReplicaSet, cap: u32) -> Self {
        let ids: Vec<ReplicaId> = set.replicas().iter().map(|r| r.id).collect();
        let counts = (0..ids.len()).map(|_| AtomicU32::new(0)).collect();
        Self {
            inner: Arc::new(LimiterInner { cap, ids, counts }),
        }
    }

    /// The configured per-replica cap.
    #[must_use]
    pub fn cap(&self) -> u32 {
        self.inner.cap
    }

    /// Current inflight count for a replica, or `None` if it is not tracked.
    #[must_use]
    pub fn inflight(&self, replica: ReplicaId) -> Option<u32> {
        self.slot(replica)
            .map(|i| self.inner.counts[i].load(Ordering::Acquire))
    }

    /// Try to admit one request to `replica`.
    ///
    /// Returns [`Admission::Admitted`] with a [`Permit`] that releases the slot
    /// on drop, or [`Admission::Shed`] if the replica is at its cap. An unknown
    /// replica (not in the set the limiter was built from) is treated as shed
    /// with `cap = 0` — a routing bug should fail closed, not over-admit.
    #[must_use]
    pub fn try_admit(&self, replica: ReplicaId) -> Admission {
        let Some(idx) = self.slot(replica) else {
            return Admission::Shed { replica, cap: 0 };
        };
        let cap = self.inner.cap;
        let counter = &self.inner.counts[idx];
        // Compare-and-swap loop: only commit the increment if we stay at/under
        // the cap, so the cap is never exceeded under concurrent admits.
        let mut cur = counter.load(Ordering::Acquire);
        loop {
            if cur >= cap {
                return Admission::Shed { replica, cap };
            }
            match counter.compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    return Admission::Admitted(Permit {
                        inner: Arc::clone(&self.inner),
                        idx,
                    });
                }
                Err(observed) => cur = observed,
            }
        }
    }

    #[inline]
    fn slot(&self, replica: ReplicaId) -> Option<usize> {
        self.inner.ids.iter().position(|&id| id == replica)
    }
}

/// An RAII admission permit. Holding it counts as one in-flight request against
/// its replica; dropping it releases the slot. The slot is released exactly once
/// even on panic, so the inflight count cannot leak.
#[derive(Debug)]
pub struct Permit {
    inner: Arc<LimiterInner>,
    idx: usize,
}

impl Permit {
    /// The replica this permit is held against.
    #[must_use]
    pub fn replica(&self) -> ReplicaId {
        self.inner.ids[self.idx]
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        // Saturating at zero defends against a double-release bug rather than
        // wrapping the counter; under correct use it never saturates.
        let counter = &self.inner.counts[self.idx];
        let mut cur = counter.load(Ordering::Acquire);
        loop {
            let next = cur.saturating_sub(1);
            match counter.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(observed) => cur = observed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replica::Replica;

    fn set(ids: &[u64]) -> ReplicaSet {
        ReplicaSet::new(ids.iter().map(|&i| Replica::up(ReplicaId(i))).collect()).unwrap()
    }

    #[test]
    fn admits_up_to_cap_then_sheds() {
        let s = set(&[1, 2]);
        let lim = InflightLimiter::new(&s, 2);
        let p1 = lim.try_admit(ReplicaId(1));
        let p2 = lim.try_admit(ReplicaId(1));
        assert!(p1.is_admitted() && p2.is_admitted());
        assert_eq!(lim.inflight(ReplicaId(1)), Some(2));

        match lim.try_admit(ReplicaId(1)) {
            Admission::Shed { replica, cap } => {
                assert_eq!(replica, ReplicaId(1));
                assert_eq!(cap, 2);
            }
            Admission::Admitted(_) => panic!("should have shed at cap"),
        }
        // a different replica is unaffected
        assert!(lim.try_admit(ReplicaId(2)).is_admitted());
    }

    #[test]
    fn permit_release_frees_a_slot() {
        let s = set(&[1]);
        let lim = InflightLimiter::new(&s, 1);
        {
            let p = lim.try_admit(ReplicaId(1));
            assert!(p.is_admitted());
            assert!(!lim.try_admit(ReplicaId(1)).is_admitted());
        } // permit dropped here
        assert_eq!(lim.inflight(ReplicaId(1)), Some(0));
        assert!(lim.try_admit(ReplicaId(1)).is_admitted());
    }

    #[test]
    fn unknown_replica_fails_closed() {
        let s = set(&[1]);
        let lim = InflightLimiter::new(&s, 8);
        match lim.try_admit(ReplicaId(42)) {
            Admission::Shed { replica, cap } => {
                assert_eq!(replica, ReplicaId(42));
                assert_eq!(cap, 0);
            }
            Admission::Admitted(_) => panic!("unknown replica must not be admitted"),
        }
    }
}
