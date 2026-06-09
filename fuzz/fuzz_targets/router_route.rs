//! Fuzz target: the deterministic fleet router
//! (`celnet_router::{ReplicaSet, PartitionMap}`).
//!
//! The router has no wire format; its surface is an arbitrary **membership**
//! (ids, health, hot-standby pins — possibly malformed) plus an arbitrary
//! **partition key**. The fuzzer drives the real validation + routing code and
//! asserts the full routing contract, including a **differential cross-check**:
//! `route`'s inline single-pass HRW scan must agree with `ranked_into`'s
//! independent `sort_unstable_by`. Those are two separate implementations of the
//! same highest-random-weight + deterministic-tie-break order, so any mutation to
//! a weight comparison or a health filter in either path makes them disagree —
//! the property the mutation gate hardens.
//!
//! Invariants asserted:
//!   * **Validation never panics:** `ReplicaSet::new` returns `Ok` or a typed
//!     `MembershipError` (`DuplicateId` / `UnknownStandby`).
//!   * **Routing never panics:** `route` returns `Ok` or a typed `RouteError`
//!     (`EmptySet` ⇔ empty; `NoHealthyReplica` ⇔ non-empty with `up_count == 0`).
//!   * **Determinism:** `route(key) == route(key)`.
//!   * **Liveness:** a routed replica is a member and is `Health::Up`.
//!   * **HRW order:** `ranked_into` is a strict descending permutation of all
//!     members (weight desc, ties by ascending id), and `ranked[0] == natural_owner`.
//!   * **Reason consistency + differential check:** `Primary`/`Standby`/
//!     `HrwFallback` each match their precise precondition, and a `HrwFallback`
//!     target equals the first healthy replica in the independently-sorted rank.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run router_route -- -max_total_time=120

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use celnet_router::{
    Health, MembershipError, PartitionKey, PartitionMap, Replica, ReplicaId, ReplicaSet,
    RouteError, RouteReason, TenantId,
};
use celnet_types::{Ccy, CcyPair};

#[derive(Arbitrary, Debug)]
struct ReplicaSpec {
    /// Small id domain (`u8`) so duplicates + standby references actually collide
    /// and exercise the validation paths.
    id: u8,
    down: bool,
    standby: Option<u8>,
}

#[derive(Arbitrary, Debug)]
struct Scenario {
    pair_sel: u8,
    tenant: Option<u64>,
    book: Option<u64>,
    replicas: Vec<ReplicaSpec>,
}

/// A fixed set of valid pairs — routing variety comes from the tenant/book digest
/// entropy, not from needing every pair.
fn pick_pair(sel: u8) -> CcyPair {
    const PAIRS: [(Ccy, Ccy); 4] = [
        (Ccy::EUR, Ccy::USD),
        (Ccy::USD, Ccy::JPY),
        (Ccy::EUR, Ccy::JPY),
        (Ccy::USD, Ccy::EUR),
    ];
    let (b, q) = PAIRS[(sel as usize) % PAIRS.len()];
    CcyPair::new(b, q)
}

fn build_key(s: &Scenario) -> PartitionKey {
    let mut k = PartitionKey::pair(pick_pair(s.pair_sel));
    if let Some(t) = s.tenant {
        k = k.with_tenant(TenantId(t));
    }
    if let Some(b) = s.book {
        k = k.with_book(celnet_router::BookId(b));
    }
    k
}

fuzz_target!(|s: Scenario| {
    let members: Vec<Replica> = s
        .replicas
        .iter()
        .map(|r| {
            let mut rep = Replica::up(ReplicaId(u64::from(r.id)));
            if let Some(sb) = r.standby {
                rep = rep.with_standby(ReplicaId(u64::from(sb)));
            }
            if r.down { rep.down() } else { rep }
        })
        .collect();

    // Contract 1: validation is total (Ok or a typed error — never a panic).
    let set = match ReplicaSet::new(members) {
        Ok(set) => set,
        Err(MembershipError::DuplicateId(_) | MembershipError::UnknownStandby(_)) => return,
    };

    let key = build_key(&s);
    let map = PartitionMap::new(&set);

    // Contract 2: routing is total + deterministic.
    let routed = map.route(key);
    assert_eq!(routed, map.route(key), "route is non-deterministic");

    match routed {
        Err(RouteError::EmptySet) => assert!(set.is_empty(), "EmptySet on a non-empty set"),
        Err(RouteError::NoHealthyReplica) => {
            assert!(!set.is_empty(), "NoHealthyReplica on an empty set");
            assert_eq!(set.up_count(), 0, "NoHealthyReplica with a healthy member");
        }
        Ok(r) => {
            assert!(set.up_count() > 0, "Ok route with zero healthy members");

            // Liveness: the chosen replica is a healthy member.
            let chosen = set.get(r.replica).expect("routed to a non-member");
            assert!(
                matches!(chosen.health, Health::Up),
                "routed to a DOWN replica"
            );

            // Natural owner is a member and agrees across the three code paths.
            assert!(
                set.get(r.natural_owner).is_some(),
                "natural owner not a member"
            );
            assert_eq!(
                map.natural_owner(key),
                Some(r.natural_owner),
                "natural_owner disagrees"
            );

            // HRW rank: descending, strict tie-break, full permutation, owner first.
            let mut ranked = Vec::new();
            map.ranked_into(key, &mut ranked);
            assert_eq!(ranked.len(), set.len(), "rank size != member count");
            assert_eq!(ranked[0].id, r.natural_owner, "rank head != natural owner");
            for w in ranked.windows(2) {
                let ordered = w[0].weight > w[1].weight
                    || (w[0].weight == w[1].weight && w[0].id.0 < w[1].id.0);
                assert!(
                    ordered,
                    "rank not in strict HRW + ascending-id tie-break order"
                );
            }
            let mut ids: Vec<u64> = ranked.iter().map(|x| x.id.0).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(
                ids.len(),
                set.len(),
                "rank is not a permutation of the members"
            );

            // Differential reason check: route()'s inline scan vs the sorted rank.
            let owner = set.get(r.natural_owner).expect("owner member");
            let first_healthy = ranked
                .iter()
                .find(|rr| matches!(set.get(rr.id).map(|m| m.health), Some(Health::Up)))
                .map(|rr| rr.id);
            match r.reason {
                RouteReason::Primary => {
                    assert_eq!(r.replica, r.natural_owner, "Primary served by a non-owner");
                    assert!(
                        matches!(owner.health, Health::Up),
                        "Primary with a down owner"
                    );
                }
                RouteReason::Standby => {
                    assert!(
                        matches!(owner.health, Health::Down),
                        "Standby with a healthy owner"
                    );
                    assert_eq!(
                        Some(r.replica),
                        owner.standby,
                        "Standby != owner's declared standby"
                    );
                    assert!(
                        matches!(chosen.health, Health::Up),
                        "promoted an unhealthy standby"
                    );
                }
                RouteReason::HrwFallback => {
                    assert!(
                        matches!(owner.health, Health::Down),
                        "Fallback with a healthy owner"
                    );
                    assert_ne!(r.replica, r.natural_owner, "Fallback served by the owner");
                    // A fallback only happens when there is no healthy declared standby.
                    let standby_healthy = owner
                        .standby
                        .and_then(|sb| set.get(sb))
                        .is_some_and(|m| matches!(m.health, Health::Up));
                    assert!(
                        !standby_healthy,
                        "Fallback chosen despite a healthy standby"
                    );
                    // The crux differential: inline scan must equal sorted-rank first-healthy.
                    assert_eq!(
                        Some(r.replica),
                        first_healthy,
                        "fallback != first healthy in rank"
                    );
                }
            }
        }
    }
});
