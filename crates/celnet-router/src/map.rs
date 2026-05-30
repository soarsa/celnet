//! The partition map — rendezvous (HRW) assignment of keys to replicas, with
//! deterministic hot-standby failover.
//!
//! Given a [`PartitionKey`] and a [`ReplicaSet`], the map computes, for every
//! replica, the rendezvous weight `w(replica, key)` and assigns the key to the
//! replica of maximum weight (`docs/SCALE-OUT.md` §2). This needs no token ring,
//! spreads keys evenly, and — crucially — moves only the keys whose argmax
//! actually changed when membership changes, i.e. in expectation `1/N` of keys
//! on a single join/leave (the HRW minimal-reshuffle property).
//!
//! Routing is layered on top:
//!   1. **Primary** = highest-weight replica that is [`Health::Up`].
//!   2. If the natural HRW owner is **down** but declares a healthy hot
//!      **standby**, the standby takes the key — no key loss, no reshuffle of
//!      the surviving keys.
//!   3. Otherwise the key falls through to the next healthy replica in HRW
//!      order (graceful degradation when no standby is declared).
//!
//! The map borrows the [`ReplicaSet`]; it holds no owned state, so a router can
//! hot-swap a new gossiped membership by simply pointing at a new set.

use crate::hash::rendezvous_weight;
use crate::key::PartitionKey;
use crate::replica::{Health, Replica, ReplicaId, ReplicaSet};

/// A view over a [`ReplicaSet`] that answers routing queries by HRW.
#[derive(Debug, Clone, Copy)]
pub struct PartitionMap<'a> {
    set: &'a ReplicaSet,
}

/// The outcome of routing one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// The replica that should serve this key right now.
    pub replica: ReplicaId,
    /// The natural HRW owner (independent of health), for observability and to
    /// decide when to re-home after a recovered replica comes back.
    pub natural_owner: ReplicaId,
    /// Why this replica (not the natural owner) was chosen.
    pub reason: RouteReason,
}

/// Why a particular replica was selected for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteReason {
    /// The natural HRW owner is healthy and serves its own key.
    Primary,
    /// The natural owner is down; its declared hot standby serves the key.
    Standby,
    /// The natural owner is down with no healthy standby; the key fell through
    /// to the next healthy replica in HRW order.
    HrwFallback,
}

/// Why a key could not be routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    /// No replica in the set is healthy.
    NoHealthyReplica,
    /// The set is empty.
    EmptySet,
}

impl core::fmt::Display for RouteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RouteError::NoHealthyReplica => f.write_str("no healthy replica to route to"),
            RouteError::EmptySet => f.write_str("replica set is empty"),
        }
    }
}

impl core::error::Error for RouteError {}

impl<'a> PartitionMap<'a> {
    /// Wrap a replica set for routing.
    #[must_use]
    pub const fn new(set: &'a ReplicaSet) -> Self {
        Self { set }
    }

    /// The HRW-ordered list of *all* replicas for a key, highest weight first —
    /// independent of health. The first entry is the natural owner; the rest are
    /// the ordered fallbacks. Ties (equal weight) break by [`ReplicaId`] so the
    /// order is total and deterministic across the fleet.
    ///
    /// Writes into `out` to stay allocation-free on the hot path; `out` is
    /// cleared first and filled with at most [`ReplicaSet::len`] entries.
    pub fn ranked_into(&self, key: PartitionKey, out: &mut Vec<RankedReplica>) {
        out.clear();
        let digest = key.digest();
        for r in self.set.replicas() {
            out.push(RankedReplica {
                id: r.id,
                weight: rendezvous_weight(r.id.seed(), digest),
            });
        }
        // Descending weight; deterministic tie-break by id so every node agrees.
        out.sort_unstable_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.id.0.cmp(&b.id.0)));
    }

    /// The natural HRW owner of a key, ignoring health. `None` only if the set
    /// is empty.
    #[must_use]
    pub fn natural_owner(&self, key: PartitionKey) -> Option<ReplicaId> {
        let digest = key.digest();
        self.set
            .replicas()
            .iter()
            .map(|r| (r.id, rendezvous_weight(r.id.seed(), digest)))
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.0.cmp(&b.0.0)))
            .map(|(id, _)| id)
    }

    /// Route a key to a live replica, applying hot-standby failover then HRW
    /// fallback.
    ///
    /// # Errors
    /// [`RouteError::EmptySet`] if there are no replicas; otherwise
    /// [`RouteError::NoHealthyReplica`] if every replica (and every standby) is
    /// down.
    pub fn route(&self, key: PartitionKey) -> Result<Route, RouteError> {
        if self.set.is_empty() {
            return Err(RouteError::EmptySet);
        }
        let digest = key.digest();

        // Find the natural owner and its weight in a single pass.
        let mut owner: Option<(ReplicaId, u64)> = None;
        for r in self.set.replicas() {
            let w = rendezvous_weight(r.id.seed(), digest);
            owner = match owner {
                Some((id, bw)) if bw > w || (bw == w && id.0 <= r.id.0) => Some((id, bw)),
                _ => Some((r.id, w)),
            };
        }
        let natural_owner = owner.expect("non-empty set has an owner").0;
        let owner_replica = self
            .set
            .get(natural_owner)
            .expect("owner is a member by construction");

        // 1. Natural owner healthy → serve directly.
        if owner_replica.is_up() {
            return Ok(Route {
                replica: natural_owner,
                natural_owner,
                reason: RouteReason::Primary,
            });
        }

        // 2. Natural owner down → declared hot standby, if healthy.
        if let Some(standby) = healthy_standby(self.set, owner_replica) {
            return Ok(Route {
                replica: standby,
                natural_owner,
                reason: RouteReason::Standby,
            });
        }

        // 3. Fall through to the next healthy replica in HRW order. This is a
        //    deterministic re-home of just this key — surviving keys (those
        //    whose owner is up) are untouched, so the reshuffle is minimal.
        let mut best: Option<(ReplicaId, u64)> = None;
        for r in self.set.replicas() {
            if !r.is_up() {
                continue;
            }
            let w = rendezvous_weight(r.id.seed(), digest);
            best = match best {
                Some((id, bw)) if bw > w || (bw == w && id.0 <= r.id.0) => Some((id, bw)),
                _ => Some((r.id, w)),
            };
        }
        match best {
            Some((id, _)) => Ok(Route {
                replica: id,
                natural_owner,
                reason: RouteReason::HrwFallback,
            }),
            None => Err(RouteError::NoHealthyReplica),
        }
    }
}

/// Resolve a replica's declared standby to a *healthy* replica id, or `None`.
#[inline]
fn healthy_standby(set: &ReplicaSet, owner: &Replica) -> Option<ReplicaId> {
    let sb = owner.standby?;
    match set.get(sb) {
        Some(r) if matches!(r.health, Health::Up) => Some(sb),
        _ => None,
    }
}

/// A replica paired with its rendezvous weight for a specific key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankedReplica {
    /// The replica.
    pub id: ReplicaId,
    /// Its rendezvous weight for the key (higher = preferred).
    pub weight: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::TenantId;
    use celnet_types::{Ccy, CcyPair};

    fn pk(seed: u64) -> PartitionKey {
        PartitionKey::pair(CcyPair::new(Ccy::EUR, Ccy::USD)).with_tenant(TenantId(seed))
    }

    fn set(ids: &[u64]) -> ReplicaSet {
        ReplicaSet::new(ids.iter().map(|&i| Replica::up(ReplicaId(i))).collect()).unwrap()
    }

    #[test]
    fn empty_set_errors() {
        let s = ReplicaSet::default();
        let m = PartitionMap::new(&s);
        assert_eq!(m.route(pk(1)), Err(RouteError::EmptySet));
    }

    #[test]
    fn healthy_owner_is_primary() {
        let s = set(&[1, 2, 3]);
        let m = PartitionMap::new(&s);
        let r = m.route(pk(1)).unwrap();
        assert_eq!(r.reason, RouteReason::Primary);
        assert_eq!(r.replica, r.natural_owner);
    }

    #[test]
    fn ranked_is_descending_and_complete() {
        let s = set(&[1, 2, 3, 4, 5]);
        let m = PartitionMap::new(&s);
        let mut out = Vec::new();
        m.ranked_into(pk(7), &mut out);
        assert_eq!(out.len(), 5);
        for w in out.windows(2) {
            assert!(w[0].weight >= w[1].weight);
        }
        // first ranked == natural owner
        assert_eq!(out[0].id, m.natural_owner(pk(7)).unwrap());
    }

    #[test]
    fn standby_takes_a_downed_owner() {
        // Find a key whose natural owner is replica 1, then down it with a
        // declared standby 99.
        let probe =
            ReplicaSet::new(vec![Replica::up(ReplicaId(1)), Replica::up(ReplicaId(2))]).unwrap();
        let pm = PartitionMap::new(&probe);
        let key = (0..)
            .map(pk)
            .find(|&k| pm.natural_owner(k) == Some(ReplicaId(1)))
            .unwrap();

        let s = ReplicaSet::new(vec![
            Replica::up(ReplicaId(1)).with_standby(ReplicaId(99)).down(),
            Replica::up(ReplicaId(2)),
            Replica::up(ReplicaId(99)),
        ])
        .unwrap();
        let m = PartitionMap::new(&s);
        let r = m.route(key).unwrap();
        assert_eq!(r.natural_owner, ReplicaId(1));
        assert_eq!(r.reason, RouteReason::Standby);
        assert_eq!(r.replica, ReplicaId(99));
    }

    #[test]
    fn hrw_fallback_when_no_standby() {
        let probe = set(&[1, 2, 3]);
        let pm = PartitionMap::new(&probe);
        let key = (0..)
            .map(pk)
            .find(|&k| pm.natural_owner(k) == Some(ReplicaId(1)))
            .unwrap();

        let s = ReplicaSet::new(vec![
            Replica::up(ReplicaId(1)).down(),
            Replica::up(ReplicaId(2)),
            Replica::up(ReplicaId(3)),
        ])
        .unwrap();
        let m = PartitionMap::new(&s);
        let r = m.route(key).unwrap();
        assert_eq!(r.natural_owner, ReplicaId(1));
        assert_eq!(r.reason, RouteReason::HrwFallback);
        assert_ne!(r.replica, ReplicaId(1));
    }

    #[test]
    fn all_down_errors() {
        let s = ReplicaSet::new(vec![
            Replica::up(ReplicaId(1)).down(),
            Replica::up(ReplicaId(2)).down(),
        ])
        .unwrap();
        let m = PartitionMap::new(&s);
        assert_eq!(m.route(pk(1)), Err(RouteError::NoHealthyReplica));
    }
}
