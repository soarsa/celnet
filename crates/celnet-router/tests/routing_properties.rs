//! Property tests for the fleet router (`docs/SCALE-OUT.md` §2, §3, §6):
//! balance, minimal reshuffle on membership change, failover with no key loss,
//! and bounded backpressure. These exercise the router as the server/edge would.

use std::collections::HashMap;

use celnet_router::{
    Admission, BookId, Health, InflightLimiter, PartitionKey, PartitionMap, Replica, ReplicaId,
    ReplicaSet, RouteReason, TenantId,
};
use celnet_types::{Ccy, CcyPair};
use proptest::prelude::*;

/// A spread of currency pairs so keys span a realistic universe.
const PAIRS: &[(Ccy, Ccy)] = &[
    (Ccy::EUR, Ccy::USD),
    (Ccy::USD, Ccy::JPY),
    (Ccy::GBP, Ccy::USD),
    (Ccy::AUD, Ccy::USD),
    (Ccy::USD, Ccy::CHF),
    (Ccy::USD, Ccy::CAD),
    (Ccy::NZD, Ccy::USD),
];

/// Build a partition key deterministically from a synthetic index, exercising
/// the full pair / tenant / book hierarchy.
fn key_from(i: u64) -> PartitionKey {
    let (b, q) = PAIRS[(i as usize) % PAIRS.len()];
    PartitionKey::pair(CcyPair::new(b, q))
        .with_tenant(TenantId(i.rotate_left(17) | 1))
        .with_book(BookId(i.wrapping_mul(0x9e37_79b9_7f4a_7c15)))
}

fn up_set(ids: &[u64]) -> ReplicaSet {
    ReplicaSet::new(ids.iter().map(|&i| Replica::up(ReplicaId(i))).collect()).unwrap()
}

/// Owner of each key under a given set (natural HRW owner, health-agnostic).
fn owners(set: &ReplicaSet, n_keys: u64) -> HashMap<u64, ReplicaId> {
    let map = PartitionMap::new(set);
    (0..n_keys)
        .map(|i| (i, map.natural_owner(key_from(i)).unwrap()))
        .collect()
}

proptest! {
    // ------------------------------------------------------------------
    // 1. Balance — keys spread roughly evenly across replicas.
    // ------------------------------------------------------------------
    #[test]
    fn keys_spread_evenly(n_replicas in 3usize..12) {
        let ids: Vec<u64> = (0..n_replicas as u64).collect();
        let set = up_set(&ids);
        let map = PartitionMap::new(&set);

        let n_keys: u64 = 6_000;
        let mut counts: HashMap<ReplicaId, u64> = HashMap::new();
        for i in 0..n_keys {
            let r = map.route(key_from(i)).unwrap();
            prop_assert_eq!(r.reason, RouteReason::Primary);
            *counts.entry(r.replica).or_default() += 1;
        }
        // Every replica owns some keys, and none is wildly over/under loaded.
        let expected = n_keys as f64 / n_replicas as f64;
        for &id in &ids {
            let c = *counts.get(&ReplicaId(id)).unwrap_or(&0) as f64;
            prop_assert!(c > 0.0, "replica {id} owns nothing");
            // Generous band (HRW is uniform but finite-sample noisy): within 45%.
            prop_assert!(
                (c - expected).abs() < 0.45 * expected,
                "replica {id} load {c} far from expected {expected}"
            );
        }
    }

    // ------------------------------------------------------------------
    // 2. Minimal reshuffle — adding/removing one replica moves only ~1/N keys.
    // ------------------------------------------------------------------
    #[test]
    fn join_moves_about_one_over_n(n in 4usize..10) {
        let n_keys: u64 = 8_000;
        let base_ids: Vec<u64> = (0..n as u64).collect();
        let before = owners(&up_set(&base_ids), n_keys);

        // Add one replica.
        let mut grown = base_ids.clone();
        grown.push(n as u64);
        let after = owners(&up_set(&grown), n_keys);

        let moved = (0..n_keys).filter(|i| before[i] != after[i]).count();
        let frac = moved as f64 / n_keys as f64;
        let ideal = 1.0 / (n as f64 + 1.0); // keys captured by the newcomer

        // Moved keys should be close to 1/(N+1); allow a finite-sample band and
        // assert HRW's guarantee that *no* key moves between two incumbents.
        prop_assert!(
            (frac - ideal).abs() < 0.5 * ideal + 0.02,
            "join reshuffle frac={frac} ideal={ideal}"
        );
        // Every moved key must now belong to the *newcomer* — HRW never moves a
        // key between two replicas that both survived the change.
        for i in 0..n_keys {
            if before[&i] != after[&i] {
                prop_assert_eq!(after[&i], ReplicaId(n as u64),
                    "key {} moved to a non-newcomer", i);
            }
        }
    }

    #[test]
    fn leave_only_rehomes_the_departing_keys(n in 5usize..11) {
        let n_keys: u64 = 8_000;
        let ids: Vec<u64> = (0..n as u64).collect();
        let before = owners(&up_set(&ids), n_keys);

        // Remove the last replica.
        let smaller: Vec<u64> = ids[..n - 1].to_vec();
        let after = owners(&up_set(&smaller), n_keys);
        let gone = ReplicaId(n as u64 - 1);

        for i in 0..n_keys {
            if before[&i] == gone {
                // its keys must move somewhere else...
                prop_assert_ne!(after[&i], gone);
            } else {
                // ...and no surviving replica's keys move.
                prop_assert_eq!(before[&i], after[&i],
                    "key {} moved despite its owner surviving", i);
            }
        }
    }

    // ------------------------------------------------------------------
    // 3. Failover — every key still routes to a *live* replica, no loss.
    // ------------------------------------------------------------------
    #[test]
    fn standby_failover_loses_no_key(n in 3usize..9, down_idx in 0usize..9) {
        let down_idx = down_idx % n;
        // Pair each replica with the next as its hot standby (ring).
        let replicas: Vec<Replica> = (0..n)
            .map(|i| {
                let standby = ReplicaId(((i + 1) % n) as u64);
                let r = Replica::up(ReplicaId(i as u64)).with_standby(standby);
                if i == down_idx { r.down() } else { r }
            })
            .collect();
        let set = ReplicaSet::new(replicas).unwrap();
        let map = PartitionMap::new(&set);

        let healthy_set = up_set(&(0..n as u64).collect::<Vec<_>>());
        let healthy_map = PartitionMap::new(&healthy_set);

        for i in 0..4_000u64 {
            let key = key_from(i);
            let route = map.route(key).unwrap();
            // Never routed to the downed replica.
            prop_assert_ne!(route.replica, ReplicaId(down_idx as u64));
            // The set member it routed to is actually up.
            prop_assert_eq!(set.get(route.replica).unwrap().health, Health::Up);

            let natural = healthy_map.natural_owner(key).unwrap();
            if natural == ReplicaId(down_idx as u64) {
                // Keys the downed node owned go to its declared standby.
                prop_assert_eq!(route.reason, RouteReason::Standby);
                prop_assert_eq!(route.replica, ReplicaId(((down_idx + 1) % n) as u64));
            } else {
                // Everyone else's keys are untouched.
                prop_assert_eq!(route.reason, RouteReason::Primary);
                prop_assert_eq!(route.replica, natural);
            }
        }
    }

    // ------------------------------------------------------------------
    // 4. Backpressure — admits exactly `cap`, then sheds; releases reopen slots.
    // ------------------------------------------------------------------
    #[test]
    fn inflight_cap_sheds_beyond_limit(cap in 1u32..32, extra in 1u32..16) {
        let set = up_set(&[1, 2, 3]);
        let lim = InflightLimiter::new(&set, cap);
        let target = ReplicaId(2);

        let mut permits = Vec::new();
        for _ in 0..cap {
            match lim.try_admit(target) {
                Admission::Admitted(p) => permits.push(p),
                Admission::Shed { .. } => prop_assert!(false, "shed before cap"),
            }
        }
        prop_assert_eq!(lim.inflight(target), Some(cap));

        // Beyond the cap, every attempt sheds with the right reason.
        for _ in 0..extra {
            match lim.try_admit(target) {
                Admission::Shed { replica, cap: c } => {
                    prop_assert_eq!(replica, target);
                    prop_assert_eq!(c, cap);
                }
                Admission::Admitted(_) => prop_assert!(false, "admitted beyond cap"),
            }
        }
        // Other replicas are independent.
        prop_assert!(lim.try_admit(ReplicaId(1)).is_admitted());

        // Releasing one permit reopens exactly one slot.
        permits.pop();
        prop_assert_eq!(lim.inflight(target), Some(cap - 1));
        prop_assert!(lim.try_admit(target).is_admitted());
    }
}

/// Concurrent backpressure: many threads hammer one replica; the cap is never
/// exceeded and total admissions equal the cap until permits release.
#[test]
fn concurrent_admits_never_exceed_cap() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;

    let set = up_set(&[1]);
    let cap = 8u32;
    let lim = InflightLimiter::new(&set, cap);
    let admitted = Arc::new(AtomicU32::new(0));
    let max_seen = Arc::new(AtomicU32::new(0));

    let handles: Vec<_> = (0..16)
        .map(|_| {
            let lim = lim.clone();
            let admitted = Arc::clone(&admitted);
            let max_seen = Arc::clone(&max_seen);
            thread::spawn(move || {
                let mut held = Vec::new();
                for _ in 0..200 {
                    if let Admission::Admitted(p) = lim.try_admit(ReplicaId(1)) {
                        admitted.fetch_add(1, Ordering::Relaxed);
                        let cur = lim.inflight(ReplicaId(1)).unwrap();
                        max_seen.fetch_max(cur, Ordering::Relaxed);
                        held.push(p);
                        if held.len() > 2 {
                            held.remove(0); // release older permits over time
                        }
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    assert!(
        max_seen.load(Ordering::Relaxed) <= cap,
        "inflight exceeded cap: {} > {cap}",
        max_seen.load(Ordering::Relaxed)
    );
    assert_eq!(lim.inflight(ReplicaId(1)), Some(0), "all permits released");
}
