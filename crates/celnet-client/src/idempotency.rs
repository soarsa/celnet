//! Idempotency keys handled for the caller.
//!
//! Every RFQ carries a client-supplied idempotency key: a retry after a timeout
//! that re-sends the *same* key returns the *same* `quote_id` (and the same booked
//! `Execution` on accept), so a flaky network can never double-request or
//! double-book. The SDK takes that burden off the caller — a [`crate::Rfq`] is
//! built once and owns a stable key for its whole request→accept lifecycle, so a
//! caller simply retries the same `Rfq` handle.
//!
//! Keys are generated from a per-client 128-bit session nonce plus a monotonic
//! per-client counter, formatted as a UUID-shaped token. The nonce is drawn from
//! the OS CSPRNG once at [`crate::Client::connect`] (off any pricing path — key
//! generation is an edge concern, never a determinism-sensitive one), so two
//! independent clients never collide, while within a client the counter keeps keys
//! unique and a retry reuses an already-issued key verbatim.

use std::sync::atomic::{AtomicU64, Ordering};

use rand_session::SessionNonce;

/// A per-client idempotency-key minter: a fixed 128-bit session nonce and a
/// monotonic counter. Cloned handles share the same counter so keys stay unique
/// across concurrent calls on one client.
#[derive(Debug, Clone)]
pub(crate) struct KeyMinter {
    nonce: SessionNonce,
    counter: std::sync::Arc<AtomicU64>,
}

impl KeyMinter {
    /// Mint a minter with a fresh random session nonce.
    pub(crate) fn new() -> Self {
        Self {
            nonce: SessionNonce::random(),
            counter: std::sync::Arc::new(AtomicU64::new(0)),
        }
    }

    /// Issue the next unique idempotency key for this client session, formatted as
    /// a UUID-shaped `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` token.
    pub(crate) fn next_key(&self) -> String {
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        let (hi, lo) = self.nonce.parts();
        // Fold the monotonic sequence into the low word so each key is distinct
        // while the high word identifies the client session.
        let lo = lo ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            (hi >> 32) as u32,
            ((hi >> 16) & 0xFFFF) as u16,
            (hi & 0xFFFF) as u16,
            ((lo >> 48) & 0xFFFF) as u16,
            lo & 0xFFFF_FFFF_FFFF,
        )
    }
}

/// A tiny self-contained session-nonce source so the SDK needs no external RNG
/// crate. Draws 128 bits of entropy once per client from the OS and mixes it,
/// then exposes the two 64-bit halves. Never used on a pricing path.
mod rand_session {
    use std::hash::{BuildHasher, Hasher};

    /// A 128-bit per-session nonce identifying one client.
    #[derive(Debug, Clone, Copy)]
    pub(super) struct SessionNonce {
        hi: u64,
        lo: u64,
    }

    impl SessionNonce {
        /// Draw a fresh nonce. Seeds from the standard-library
        /// [`std::collections::hash_map::RandomState`] (which itself seeds from the
        /// OS CSPRNG) by hashing two distinct sentinels, then runs each through a
        /// `splitmix64` finaliser so the two halves are well-mixed and independent.
        pub(super) fn random() -> Self {
            let state = std::collections::hash_map::RandomState::new();
            let raw_hi = {
                let mut h = state.build_hasher();
                h.write_u64(0xA5A5_A5A5_5A5A_5A5A);
                h.finish()
            };
            let raw_lo = {
                let mut h = state.build_hasher();
                h.write_u64(0x1234_5678_9ABC_DEF0);
                h.finish()
            };
            Self {
                hi: splitmix64(raw_hi ^ 0xD1B5_4A32_D192_ED03),
                lo: splitmix64(raw_lo ^ 0x9E37_79B9_7F4A_7C15),
            }
        }

        /// The two 64-bit halves of the nonce.
        pub(super) fn parts(&self) -> (u64, u64) {
            (self.hi, self.lo)
        }
    }

    /// The public-domain `splitmix64` finaliser (Vigna), used here only to mix
    /// OS-seeded entropy into well-distributed 64-bit halves.
    fn splitmix64(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn keys_are_unique_within_a_client() {
        let minter = KeyMinter::new();
        let mut seen = HashSet::new();
        for _ in 0..10_000 {
            assert!(seen.insert(minter.next_key()), "keys must be unique");
        }
    }

    #[test]
    fn keys_are_uuid_shaped() {
        let key = KeyMinter::new().next_key();
        let parts: Vec<&str> = key.split('-').collect();
        assert_eq!(parts.len(), 5, "uuid-shaped key has five groups");
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn two_clients_do_not_collide() {
        let a = KeyMinter::new();
        let b = KeyMinter::new();
        // The session nonces differ, so even the first key of each client differs.
        assert_ne!(a.next_key(), b.next_key());
    }
}
