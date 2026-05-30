//! Replicas — the stateless pricing nodes a key can route to.
//!
//! A [`ReplicaId`] names one `celnet-engine` shard process. A [`ReplicaSet`] is
//! the live membership the router routes against: which replicas exist, which
//! are healthy, and the **hot-standby** pairing that backs each replica. The set
//! is the *only* state a stateless router holds (`docs/SCALE-OUT.md` §3); it is
//! versioned and gossiped so routers converge without a global recompute.

use crate::hash::mix64;

/// A stable, fleet-wide replica identifier. Only its bits matter for routing;
/// the [`seed`](ReplicaId::seed) is a well-mixed function of the id so that
/// per-replica rendezvous weights are effectively independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReplicaId(pub u64);

impl ReplicaId {
    /// The replica's rendezvous seed — its id passed through the avalanching
    /// mixer so that two numerically-adjacent ids (`5`, `6`) produce
    /// uncorrelated weight sequences and the HRW load stays even.
    #[inline]
    #[must_use]
    pub const fn seed(self) -> u64 {
        mix64(self.0)
    }
}

/// Health of a replica as the router currently believes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Health {
    /// Accepting work.
    Up,
    /// Marked down (failed health check / drained for upgrade). Routed *around*
    /// deterministically — its keys move to their next HRW choice or to a
    /// declared hot standby.
    Down,
}

/// One membership entry: a replica, its health, and an optional **hot standby**
/// that deterministically backs it on failure.
///
/// The standby is an explicit pin (not derived from HRW) so an operator can
/// place a pre-warmed shadow shard on a chosen node; when the primary is
/// [`Health::Down`] and the standby is [`Health::Up`], every key the primary
/// owned routes to that standby with **no key loss** — see
/// [`crate::PartitionMap::route`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replica {
    /// The replica's identity.
    pub id: ReplicaId,
    /// Current health.
    pub health: Health,
    /// Declared hot standby that backs this replica on failure, if any.
    pub standby: Option<ReplicaId>,
}

impl Replica {
    /// A healthy replica with no declared standby.
    #[must_use]
    pub const fn up(id: ReplicaId) -> Self {
        Self {
            id,
            health: Health::Up,
            standby: None,
        }
    }

    /// Declare a hot standby that backs this replica on failure.
    #[must_use]
    pub const fn with_standby(mut self, standby: ReplicaId) -> Self {
        self.standby = Some(standby);
        self
    }

    /// Mark this replica down (failed / draining).
    #[must_use]
    pub const fn down(mut self) -> Self {
        self.health = Health::Down;
        self
    }

    #[inline]
    #[must_use]
    pub(crate) const fn is_up(&self) -> bool {
        matches!(self.health, Health::Up)
    }
}

/// The live replica membership a router routes against.
///
/// Construction validates uniqueness of ids; routing reads it lock-free. The
/// set is small (one entry per shard process) so a linear HRW scan per route is
/// cheaper than any index and stays allocation-free.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplicaSet {
    replicas: Vec<Replica>,
}

/// Why a [`ReplicaSet`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipError {
    /// The same [`ReplicaId`] appeared more than once.
    DuplicateId(ReplicaId),
    /// A declared standby is not itself a member of the set.
    UnknownStandby(ReplicaId),
}

impl core::fmt::Display for MembershipError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MembershipError::DuplicateId(id) => write!(f, "duplicate replica id {}", id.0),
            MembershipError::UnknownStandby(id) => {
                write!(f, "standby {} is not a member of the set", id.0)
            }
        }
    }
}

impl core::error::Error for MembershipError {}

impl ReplicaSet {
    /// Build a set from its members, validating that ids are unique and every
    /// declared standby is itself a member.
    ///
    /// # Errors
    /// Returns [`MembershipError`] on a duplicate id or a dangling standby.
    pub fn new(replicas: Vec<Replica>) -> Result<Self, MembershipError> {
        for (i, r) in replicas.iter().enumerate() {
            if replicas[..i].iter().any(|o| o.id == r.id) {
                return Err(MembershipError::DuplicateId(r.id));
            }
        }
        for r in &replicas {
            if let Some(sb) = r.standby
                && !replicas.iter().any(|o| o.id == sb)
            {
                return Err(MembershipError::UnknownStandby(sb));
            }
        }
        Ok(Self { replicas })
    }

    /// The members, in declaration order.
    #[must_use]
    pub fn replicas(&self) -> &[Replica] {
        &self.replicas
    }

    /// Number of members (healthy or not).
    #[must_use]
    pub fn len(&self) -> usize {
        self.replicas.len()
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.replicas.is_empty()
    }

    /// Count of healthy members.
    #[must_use]
    pub fn up_count(&self) -> usize {
        self.replicas.iter().filter(|r| r.is_up()).count()
    }

    /// Look up a member by id.
    #[must_use]
    pub fn get(&self, id: ReplicaId) -> Option<&Replica> {
        self.replicas.iter().find(|r| r.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_ids() {
        let err = ReplicaSet::new(vec![Replica::up(ReplicaId(1)), Replica::up(ReplicaId(1))])
            .unwrap_err();
        assert_eq!(err, MembershipError::DuplicateId(ReplicaId(1)));
    }

    #[test]
    fn rejects_dangling_standby() {
        let err = ReplicaSet::new(vec![Replica::up(ReplicaId(1)).with_standby(ReplicaId(9))])
            .unwrap_err();
        assert_eq!(err, MembershipError::UnknownStandby(ReplicaId(9)));
    }

    #[test]
    fn seed_decorrelates_adjacent_ids() {
        assert_ne!(ReplicaId(5).seed(), ReplicaId(6).seed());
    }

    #[test]
    fn up_count_tracks_health() {
        let set = ReplicaSet::new(vec![
            Replica::up(ReplicaId(1)),
            Replica::up(ReplicaId(2)).down(),
            Replica::up(ReplicaId(3)),
        ])
        .unwrap();
        assert_eq!(set.up_count(), 2);
        assert_eq!(set.len(), 3);
    }
}
