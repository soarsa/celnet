//! Deterministic, allocation-free 64-bit hashing for rendezvous (HRW) routing.
//!
//! The router must agree on a partition→replica assignment **bit-identically**
//! across every node and process in the fleet, given only the (key, replica)
//! pair — there is no shared token ring to consult. That rules out
//! [`std::collections::hash_map::DefaultHasher`] (its seed is process-random)
//! and any hasher whose output is not fixed by the standard. We therefore carry
//! a small, self-contained integer mixer with a frozen specification.
//!
//! The mixer is the well-known `splitmix64` finalizer (an avalanching sequence
//! of xor-shift / odd-constant multiplies); it has excellent diffusion — a
//! one-bit input change flips ~half the output bits — which is exactly what the
//! highest-random-weight construction needs so that per-replica weights are
//! effectively independent uniform draws (see `docs/SCALE-OUT.md` §2). The
//! method name is documentation only; no public identifier is named for it
//! (GUIDE.md rule 8).
//!
//! All operations are `const`-evaluable, branch-free, and never allocate, so
//! they are safe to call from the routing fast path.

/// Avalanching 64-bit finalizer (the `splitmix64` mix). Bijective, so it never
/// collapses distinct inputs.
#[inline]
#[must_use]
pub(crate) const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Fold one 64-bit lane into a running accumulator, then re-mix.
///
/// Used to chain the fields of a composite partition key into a single stable
/// digest. Combining by `wrapping_add` of mixed lanes (rather than a plain xor)
/// keeps the order-sensitivity that distinguishes, e.g., a `(pair, book)` key
/// from a `(book, pair)` one — even though the router never builds the latter.
#[inline]
#[must_use]
pub(crate) const fn fold64(acc: u64, lane: u64) -> u64 {
    // Rotate the accumulator so successive lanes occupy different bit positions
    // before mixing — defeats trivial cancellation of equal lanes.
    let rotated = acc.rotate_left(23);
    mix64(rotated.wrapping_add(mix64(lane)))
}

/// The rendezvous **weight** of a (replica-seed, key-digest) pair.
///
/// HRW assigns a key to the replica maximizing this weight. Both inputs are
/// pre-mixed and combined through [`fold64`] so the weight behaves like an
/// independent uniform 64-bit draw per (replica, key) — the property that makes
/// the load even and the reshuffle on membership change minimal (only the keys
/// whose new argmax changed move; in expectation `1/N` of them).
#[inline]
#[must_use]
pub(crate) const fn rendezvous_weight(replica_seed: u64, key_digest: u64) -> u64 {
    // Seed the key digest with the replica seed, then mix once more so the two
    // sources are fully entangled before comparison.
    mix64(fold64(replica_seed, key_digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix64_is_bijective_on_samples() {
        // A bijection never maps two distinct inputs to the same output; spot
        // check a spread of inputs map to distinct outputs.
        let mut seen = std::collections::HashSet::new();
        for i in 0..10_000u64 {
            let x = i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            assert!(seen.insert(mix64(x)), "collision at {x}");
        }
    }

    #[test]
    fn mix64_avalanches() {
        // Flipping a single input bit should change roughly half the output
        // bits (good diffusion). Average Hamming distance over 64 single-bit
        // flips on a fixed base should land near 32.
        let base = 0x0123_4567_89ab_cdefu64;
        let h0 = mix64(base);
        let mut total = 0u32;
        for bit in 0..64 {
            let h1 = mix64(base ^ (1u64 << bit));
            total += (h0 ^ h1).count_ones();
        }
        let avg = f64::from(total) / 64.0;
        assert!((24.0..40.0).contains(&avg), "weak avalanche: avg={avg}");
    }

    #[test]
    fn fold_is_order_sensitive() {
        let a = fold64(fold64(0, 1), 2);
        let b = fold64(fold64(0, 2), 1);
        assert_ne!(a, b, "fold must be order-sensitive");
    }

    #[test]
    fn weights_are_deterministic() {
        // Same inputs → same weight, every time (cross-process stability).
        assert_eq!(rendezvous_weight(7, 42), rendezvous_weight(7, 42));
        assert_ne!(rendezvous_weight(7, 42), rendezvous_weight(8, 42));
    }
}
