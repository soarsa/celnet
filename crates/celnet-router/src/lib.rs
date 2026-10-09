//! Celnet fleet router — the horizontal scale-out tier (work-stream WS-J; design
//! in `docs/SCALE-OUT.md`). It routes pricing work across stateless replicas so
//! Celnet scales to investment-banking-sized portfolios by scaling *around* the
//! pinned hot path, **never through it** — this crate performs **no network IO**
//! and holds no pricing state; it is the pure, deterministic routing core that
//! the server/edge wires.
//!
//! # What it provides
//!
//! - **Partition map** ([`PartitionMap`]) — shards work by [`PartitionKey`]
//!   (currency-pair, optionally sub-sharded by tenant / book) across `N`
//!   stateless replicas via **highest-random-weight (rendezvous) hashing**. This
//!   is deterministic and process-independent (every node agrees with no shared
//!   token ring) and reshuffles only ~`1/N` of keys when a replica joins or
//!   leaves.
//! - **Replica routing** ([`PartitionMap::route`], [`PartitionMap::ranked_into`])
//!   — given a key and a live [`ReplicaSet`], pick the owning replica plus the
//!   HRW-ordered fallbacks.
//! - **Hot-standby failover** — a replica marked [`Health::Down`] routes
//!   deterministically to its declared hot [`Replica::standby`] with no key loss
//!   and no reshuffle of surviving keys; with no standby, the key falls through
//!   to the next healthy replica in HRW order.
//! - **Bounded backpressure** ([`InflightLimiter`]) — a per-replica inflight cap
//!   that returns a typed [`Admission::Shed`] instead of unbounded queueing,
//!   honouring the "conflate/shed, never buffer" rule (`docs/SCALE-OUT.md` §6).
//!
//! # Determinism
//!
//! All routing is a pure function of `(key, replica set)`. The hashing is a
//! frozen integer mixer (see the `hash` module), not the process-seeded standard
//! hasher, so assignments are bit-identical across the fleet and across runs — a
//! prerequisite for stateless routers to converge on a gossiped membership and
//! for deterministic replay (`docs/SCALE-OUT.md` §4). No public identifier is
//! named for a method, person, or vendor (GUIDE.md rule 8).

#![forbid(unsafe_code)]

mod backpressure;
mod hash;
mod key;
mod map;
mod replica;

pub use backpressure::{Admission, InflightLimiter, Permit};
pub use key::{BookId, PartitionKey, TenantId};
pub use map::{PartitionMap, RankedReplica, Route, RouteError, RouteReason};
pub use replica::{Health, MembershipError, Replica, ReplicaId, ReplicaSet};
