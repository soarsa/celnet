//! Mutation-killing value-pinning suite for `celnet-router` (the `--jobs 3`
//! `cargo mutants -p celnet-router` gate, `.config/mutants-celnet-router.toml`).
//!
//! The crate's property tests in `tests/routing_properties.rs` prove the *shape*
//! of routing (balance, minimal reshuffle, no key loss, bounded shed) but leave a
//! cluster of mutants alive because they only assert *relations* (descending,
//! distinct, `!= downed`) — not the **exact** digest bits, the **exact** HRW
//! weight, or **which** replica a fallback selects. A syntactic mutant that
//! changes the hash mixing, the digest byte-packing, or the fallback argmax
//! tie-break can still satisfy "distinct" / "not the downed node", so the suite
//! misses it.
//!
//! This file closes that gap by pinning every load-bearing value against an
//! **independent oracle**: a code-disjoint re-implementation of the frozen
//! `splitmix64` mixing / lane-fold / digest spec (re-derived from the documented
//! algorithm in `src/hash.rs` and `src/key.rs`, NOT calling the crate's private
//! `mix64`/`fold64`/`rendezvous_weight`), plus a brute-force max-weight argmax
//! with a lowest-id tie-break. Routing output (`digest`, `RankedReplica.weight`,
//! `route`, `natural_owner`) is graded against that oracle — the oracle never
//! grades itself.

use celnet_router::{
    BookId, InflightLimiter, PartitionKey, PartitionMap, Replica, ReplicaId, ReplicaSet,
    RouteReason, TenantId,
};
use celnet_types::{Ccy, CcyPair};

// ---------------------------------------------------------------------------
// Independent oracle — a code-disjoint re-implementation of the FROZEN hashing
// spec. This is NOT the crate's hash module (that is `pub(crate)`); it is a
// hand-recomputation of the documented `splitmix64` finalizer, the rotate-fold
// lane combiner, the rendezvous weight, the replica seed, and the partition-key
// digest, written from the algorithm description so a mutant in the crate's copy
// is caught by disagreeing with this independent copy.
// ---------------------------------------------------------------------------

/// Independent `splitmix64` finalizer (the documented avalanching mix).
fn oracle_mix64(z0: u64) -> u64 {
    let mut z = z0;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Independent rotate-fold lane combiner: `mix64(rotl(acc,23) + mix64(lane))`.
fn oracle_fold64(acc: u64, lane: u64) -> u64 {
    let rotated = acc.rotate_left(23);
    oracle_mix64(rotated.wrapping_add(oracle_mix64(lane)))
}

/// Independent rendezvous weight: `mix64(fold64(seed, digest))`.
fn oracle_weight(replica_seed: u64, key_digest: u64) -> u64 {
    oracle_mix64(oracle_fold64(replica_seed, key_digest))
}

/// Independent replica seed: `mix64(id)`.
fn oracle_seed(id: u64) -> u64 {
    oracle_mix64(id)
}

/// Independent optional-id tag: `None -> sentinel`, `Some(v) -> mix64(v ^ mask)`.
fn oracle_tagged(id: Option<u64>) -> u64 {
    match id {
        None => 0xA5A5_A5A5_A5A5_A5A5,
        Some(v) => oracle_mix64(v ^ 0x5555_5555_5555_5555),
    }
}

/// Independent partition-key digest: pack the six currency bytes big-endian into
/// one lane (`b0<<40 | b1<<32 | b2<<24 | q0<<16 | q1<<8 | q2`), mix it, then fold
/// the tagged tenant and book lanes.
fn oracle_digest(pair: CcyPair, tenant: Option<u64>, book: Option<u64>) -> u64 {
    let b = pair.base.as_str().as_bytes();
    let q = pair.quote.as_str().as_bytes();
    let pair_lane = (u64::from(b[0]) << 40)
        | (u64::from(b[1]) << 32)
        | (u64::from(b[2]) << 24)
        | (u64::from(q[0]) << 16)
        | (u64::from(q[1]) << 8)
        | u64::from(q[2]);
    let mut acc = oracle_mix64(pair_lane);
    acc = oracle_fold64(acc, oracle_tagged(tenant));
    acc = oracle_fold64(acc, oracle_tagged(book));
    acc
}

fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}

fn up_set(ids: &[u64]) -> ReplicaSet {
    ReplicaSet::new(ids.iter().map(|&i| Replica::up(ReplicaId(i))).collect()).unwrap()
}

/// Brute-force HRW pick over a slice of replica ids for one key digest, using the
/// INDEPENDENT oracle weight and the documented lowest-id tie-break. Returns the
/// chosen replica id. This is the specification of `route`/`natural_owner`.
fn oracle_argmax(ids: &[u64], digest: u64) -> u64 {
    let mut best: Option<(u64, u64)> = None; // (id, weight)
    for &id in ids {
        let w = oracle_weight(oracle_seed(id), digest);
        best = match best {
            Some((bid, bw)) if bw > w || (bw == w && bid <= id) => Some((bid, bw)),
            _ => Some((id, w)),
        };
    }
    best.unwrap().0
}

// ===========================================================================
// 1. Digest — exact bit value (kills byte-packing `<<`/`|` and `tagged` mutants,
//    and the `mix64` second-step `^`/`>>` mutants, since digest calls mix64).
// ===========================================================================

#[test]
fn digest_matches_independent_spec_bit_for_bit() {
    // A spread of pairs and sub-key combinations: each must equal the
    // independently-recomputed digest exactly. Any mutated shift amount,
    // OR-vs-AND-vs-XOR pack, or `tagged` xor-mask change shifts these bits.
    let cases: &[(CcyPair, Option<u64>, Option<u64>)] = &[
        (eurusd(), None, None),
        (eurusd(), Some(7), None),
        (eurusd(), None, Some(9)),
        (eurusd(), Some(0), Some(0)),
        (CcyPair::new(Ccy::USD, Ccy::JPY), Some(3), Some(5)),
        (CcyPair::new(Ccy::GBP, Ccy::CHF), Some(0xDEAD_BEEF), None),
        (CcyPair::new(Ccy::AUD, Ccy::NZD), None, Some(0xFEED_FACE)),
    ];
    for &(pair, t, b) in cases {
        let mut k = PartitionKey::pair(pair);
        if let Some(t) = t {
            k = k.with_tenant(TenantId(t));
        }
        if let Some(b) = b {
            k = k.with_book(BookId(b));
        }
        assert_eq!(
            k.digest(),
            oracle_digest(pair, t, b),
            "digest disagrees with the independent spec for {pair:?} t={t:?} b={b:?}"
        );
    }
}

#[test]
fn digest_byte_positions_are_distinct_per_currency_slot() {
    // Each of the six currency-byte slots occupies a DIFFERENT shift, so a pair
    // that differs only in one slot must change the digest. This pins the per-
    // slot `<<` shift amounts and the `|` packing against `<<`->`>>` / `|`->`&`
    // / `|`->`^` mutants (which collapse or relocate a slot's bits).
    let base = PartitionKey::pair(eurusd()).digest();
    // Swap quote USD->JPY (changes q0,q1,q2 region):
    let q_changed = PartitionKey::pair(CcyPair::new(Ccy::EUR, Ccy::JPY)).digest();
    // Swap base EUR->GBP (changes b0,b1,b2 region):
    let b_changed = PartitionKey::pair(CcyPair::new(Ccy::GBP, Ccy::USD)).digest();
    assert_ne!(base, q_changed, "quote-slot change must move the digest");
    assert_ne!(base, b_changed, "base-slot change must move the digest");
    assert_ne!(q_changed, b_changed, "base vs quote change must differ");
}

#[test]
fn tagged_present_zero_distinct_from_absent_and_each_other() {
    // The `tagged` xor-mask makes a present id of 0 distinct from the absent
    // sentinel; a `^`->`|`/`&` mutant breaks that. Independent oracle pins it.
    let absent = PartitionKey::pair(eurusd());
    let t0 = absent.with_tenant(TenantId(0));
    let b0 = absent.with_book(BookId(0));
    assert_eq!(t0.digest(), oracle_digest(eurusd(), Some(0), None));
    assert_eq!(b0.digest(), oracle_digest(eurusd(), None, Some(0)));
    assert_ne!(absent.digest(), t0.digest());
    assert_ne!(absent.digest(), b0.digest());
    assert_ne!(t0.digest(), b0.digest());
}

// ===========================================================================
// 2. Rendezvous weight — exact value via the public RankedReplica.weight (kills
//    mix64 / fold64 / rendezvous / seed mutants by exact disagreement).
// ===========================================================================

#[test]
fn ranked_weights_match_independent_rendezvous_spec() {
    let ids = [1u64, 2, 7, 42, 1000, 65535];
    let set = up_set(&ids);
    let map = PartitionMap::new(&set);

    for seed in 0..50u64 {
        let key = PartitionKey::pair(eurusd())
            .with_tenant(TenantId(seed | 1))
            .with_book(BookId(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15)));
        let digest = oracle_digest(
            eurusd(),
            Some(seed | 1),
            Some(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
        );

        let mut out = Vec::new();
        map.ranked_into(key, &mut out);
        assert_eq!(out.len(), ids.len());
        for ranked in &out {
            let expected = oracle_weight(oracle_seed(ranked.id.0), digest);
            assert_eq!(
                ranked.weight, expected,
                "weight for replica {} disagrees with the independent rendezvous spec",
                ranked.id.0
            );
        }
        // Ranked order is exactly descending weight, lowest-id tie-break.
        for w in out.windows(2) {
            assert!(
                w[0].weight > w[1].weight || (w[0].weight == w[1].weight && w[0].id.0 < w[1].id.0),
                "ranked order violates (weight desc, id asc)"
            );
        }
    }
}

// ===========================================================================
// 3. Natural owner / primary route — argmax pinned to the brute-force oracle.
// ===========================================================================

#[test]
fn natural_owner_and_primary_route_equal_brute_force_argmax() {
    let ids = [3u64, 11, 19, 23, 31, 47];
    let set = up_set(&ids);
    let map = PartitionMap::new(&set);

    for seed in 0..400u64 {
        let key = PartitionKey::pair(eurusd()).with_tenant(TenantId(seed));
        let digest = oracle_digest(eurusd(), Some(seed), None);
        let expected = oracle_argmax(&ids, digest);

        let owner = map.natural_owner(key).unwrap();
        assert_eq!(owner.0, expected, "natural_owner != brute-force argmax");

        let route = map.route(key).unwrap();
        assert_eq!(route.reason, RouteReason::Primary);
        assert_eq!(route.replica.0, expected, "primary route != argmax");
        assert_eq!(route.natural_owner.0, expected);
    }
}

// ===========================================================================
// 4. HRW fallback — the exact fallback replica equals the argmax over the
//    HEALTHY replicas. This kills the line-172 fallback-loop mutants (min vs
//    max, first vs last, guard true/false, &&/||, ==/!=) — every one of which
//    selects a DIFFERENT healthy replica for some configuration.
// ===========================================================================

#[test]
fn hrw_fallback_picks_argmax_among_healthy() {
    // No standbys declared, so a downed owner falls through to the next healthy
    // replica in HRW order. For each key whose natural owner is downed, the
    // chosen replica MUST be the brute-force argmax over the remaining healthy
    // ids — pinning min/max/first/last selection exactly.
    let ids = [1u64, 2, 3, 4, 5, 6, 7];

    // Down a single owner at a time; for every key it owned, verify the fallback.
    for &down in &ids {
        let healthy: Vec<u64> = ids.iter().copied().filter(|&i| i != down).collect();
        let replicas: Vec<Replica> = ids
            .iter()
            .map(|&i| {
                let r = Replica::up(ReplicaId(i));
                if i == down { r.down() } else { r }
            })
            .collect();
        let set = ReplicaSet::new(replicas).unwrap();
        let map = PartitionMap::new(&set);

        let mut checked_fallback = false;
        for seed in 0..600u64 {
            let key = PartitionKey::pair(eurusd())
                .with_tenant(TenantId(seed))
                .with_book(BookId(down));
            let digest = oracle_digest(eurusd(), Some(seed), Some(down));
            let natural = oracle_argmax(&ids, digest);
            let route = map.route(key).unwrap();

            if natural == down {
                // Owner is down, no standby -> fall through to argmax over healthy.
                let expected = oracle_argmax(&healthy, digest);
                assert_eq!(route.reason, RouteReason::HrwFallback);
                assert_eq!(
                    route.replica.0, expected,
                    "fallback replica != argmax over healthy (down={down}, seed={seed})"
                );
                assert_eq!(route.natural_owner.0, down);
                checked_fallback = true;
            } else {
                assert_eq!(route.reason, RouteReason::Primary);
                assert_eq!(route.replica.0, natural);
            }
        }
        assert!(
            checked_fallback,
            "no key exercised the fallback for down={down}; widen the seed range"
        );
    }
}

// ===========================================================================
// 5. Hot-standby health gate — a DOWN standby must be rejected (kills the
//    `matches!(r.health, Up) -> true` mutant in `healthy_standby`).
// ===========================================================================

#[test]
fn down_standby_is_rejected_and_falls_through() {
    // Find a key whose natural owner is replica 1. Bounded search (not an
    // unbounded `(0..)`) so a degenerate/constant hash — e.g. a `mix64 -> 0`
    // mutant that collapses every weight and pins the owner to one id — FAILS
    // FAST here (the `expect`) instead of looping forever; on the real mixer a
    // matching key is found within a handful of tenants.
    let probe = up_set(&[1, 2, 3]);
    let pm = PartitionMap::new(&probe);
    let key = (0u64..10_000)
        .map(|s| PartitionKey::pair(eurusd()).with_tenant(TenantId(s)))
        .find(|&k| pm.natural_owner(k) == Some(ReplicaId(1)))
        .expect("no key in [0,10000) owned by replica 1 — degenerate hash?");

    // Owner 1 down, its declared standby 2 ALSO down, 3 healthy. A correct
    // `healthy_standby` rejects the down standby and falls through to HRW; the
    // `matches!(.., Up) -> true` mutant would (wrongly) route to the down
    // standby 2.
    let set = ReplicaSet::new(vec![
        Replica::up(ReplicaId(1)).with_standby(ReplicaId(2)).down(),
        Replica::up(ReplicaId(2)).down(),
        Replica::up(ReplicaId(3)),
    ])
    .unwrap();
    let m = PartitionMap::new(&set);
    let r = m.route(key).unwrap();
    assert_eq!(r.natural_owner, ReplicaId(1));
    assert_eq!(
        r.reason,
        RouteReason::HrwFallback,
        "down standby must be rejected (not used as Standby)"
    );
    assert_eq!(
        r.replica,
        ReplicaId(3),
        "must fall through to the only healthy replica"
    );

    // And the positive control: with the standby healthy, it IS used.
    let set_ok = ReplicaSet::new(vec![
        Replica::up(ReplicaId(1)).with_standby(ReplicaId(2)).down(),
        Replica::up(ReplicaId(2)),
        Replica::up(ReplicaId(3)),
    ])
    .unwrap();
    let m_ok = PartitionMap::new(&set_ok);
    let r_ok = m_ok.route(key).unwrap();
    assert_eq!(r_ok.reason, RouteReason::Standby);
    assert_eq!(r_ok.replica, ReplicaId(2));
}

// ===========================================================================
// 6. Backpressure cap accessor (kills `cap -> 0` / `cap -> 1`).
// ===========================================================================

#[test]
fn limiter_reports_its_configured_cap() {
    let set = up_set(&[1, 2]);
    // A distinctive cap that is neither of the constant-mutant values (0, 1).
    let lim = InflightLimiter::new(&set, 7);
    assert_eq!(lim.cap(), 7, "cap accessor must return the configured cap");
}

// ===========================================================================
// 7. Error Display strings (kills the `fmt -> Ok(Default::default())` mutants
//    for RouteError and MembershipError, which would emit an empty string).
// ===========================================================================

#[test]
fn error_display_strings_are_nonempty_and_specific() {
    use celnet_router::{MembershipError, RouteError};

    assert_eq!(
        RouteError::EmptySet.to_string(),
        "replica set is empty",
        "EmptySet display must match"
    );
    assert_eq!(
        RouteError::NoHealthyReplica.to_string(),
        "no healthy replica to route to",
        "NoHealthyReplica display must match"
    );
    assert_eq!(
        MembershipError::DuplicateId(ReplicaId(5)).to_string(),
        "duplicate replica id 5",
        "DuplicateId display must match"
    );
    assert_eq!(
        MembershipError::UnknownStandby(ReplicaId(9)).to_string(),
        "standby 9 is not a member of the set",
        "UnknownStandby display must match"
    );
}
