//! A small, stateless, cross-platform-deterministic PRNG for the LP simulator.
//!
//! This deliberately mirrors the seeding scheme of [`celnet_aggregation::sim`]
//! (SplitMix64 keyed on `(seed, venue, instrument, tick)`) so the two simulators
//! share ONE determinism convention rather than inventing a second. SplitMix64
//! (Steele, Lea & Flood, "Fast Splittable Pseudorandom Number Generators",
//! OOPSLA 2014) is a fast, statelessly-seedable mixing function; the venue and
//! instrument hashes use the standard-library `DefaultHasher` (SipHash with fixed
//! zero keys), which is deterministic across processes and platforms. No `rand`
//! crate and no wall-clock are used anywhere, so every draw is a pure function of
//! its key and the whole simulator is reproducible bit-for-bit.

use std::hash::{Hash, Hasher};

use celnet_aggregation::{Instrument, VenueId};

/// One `SplitMix64` mixing step (the standard finalizer constants).
#[must_use]
pub(crate) fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A stable, cross-process-deterministic `u64` hash of any `Hash` value via the
/// standard-library `DefaultHasher` (SipHash keyed on fixed zero keys).
fn stable_hash<T: Hash>(value: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// A deterministic noise draw in `[−1, 1)` keyed on `(seed, venue, instrument,
/// tick)`. Stateless: identical inputs always yield the identical value, and
/// distinct keys decorrelate. Uses the top 53 bits (an exact `f64` mantissa) so
/// the mapping to `[−1, 1)` is uniform and rounding-free.
#[must_use]
pub(crate) fn seeded_unit(seed: u64, venue: &VenueId, instrument: &Instrument, tick: i64) -> f64 {
    let mut x = seed;
    x ^= stable_hash(&venue.0).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = splitmix64(x);
    x ^= stable_hash(instrument).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x = splitmix64(x);
    x ^= (tick as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    let bits = splitmix64(x);
    // Top 53 bits → a uniform in [0, 1); affine-map to [−1, 1).
    let unit = (bits >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0); // 2^-53
    2.0 * unit - 1.0
}

/// Derive a decorrelated child seed for fleet member `index` from a root `seed`
/// (two SplitMix64 rounds so adjacent indices do not share low-bit structure).
#[must_use]
pub(crate) fn child_seed(seed: u64, index: usize) -> u64 {
    splitmix64(seed ^ splitmix64(index as u64))
}

/// A deterministic scalar draw in `[0, 1)` keyed on `(seed, salt)` — the
/// scalar-parameter analogue of [`seeded_unit`] (which keys on a venue/instrument/
/// tick). Used to disperse a member's quoting *character* (half-spread, size,
/// cadence, quality) and to schedule occasional faults per round, all reproducibly.
/// Uses the top 53 bits so the mapping is uniform and rounding-free.
#[must_use]
pub(crate) fn unit01(seed: u64, salt: u64) -> f64 {
    let bits = splitmix64(seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    (bits >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0) // 2^-53
}
