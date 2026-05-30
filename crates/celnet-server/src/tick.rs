//! A deterministic, seeded market-tick source driving the RFS stream.
//!
//! The WebSocket streaming endpoint pushes a fresh price/Greek update whenever
//! the market moves. In production that motion comes from a vendor feed; for a
//! self-contained, **deterministic** edge (and for tests) this module synthesizes
//! ticks from a *counter-based* generator — never the wall clock and never an OS
//! RNG — so a stream is bit-exactly reproducible from `(seed, base_state)`.
//!
//! Each tick advances an internal counter and derives a small multiplicative spot
//! bump from a `splitmix64` hash of `(seed, counter)`. `splitmix64` is the
//! standard, public-domain counter mixer (purpose-named here, no author name in
//! the API); it gives a well-distributed deterministic sequence with no shared
//! state, matching the determinism discipline (counter-based / seeded RNG only).
//!
//! The source republishes a new [`celnet_engine::MarketState`] (a clone of the
//! base with a bumped spot) so the pricing core reprices against it lock-free.

use celnet_engine::MarketState;

/// The 64-bit `splitmix64` finalizer applied to a counter to yield a uniform
/// `u64`. Public-domain mixing constants (Steele/Lea splitmix); purpose-named.
const fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Map a uniform `u64` to a value in `[-1.0, 1.0)` deterministically.
fn unit_signed(bits: u64) -> f64 {
    // Take 53 bits for a double in [0,1), then center to [-1,1).
    let unit = (bits >> 11) as f64 / (1u64 << 53) as f64;
    unit.mul_add(2.0, -1.0)
}

/// A deterministic source of market ticks for the RFS stream.
///
/// Holds the base market state, a seed, a per-tick spot volatility (the maximum
/// fractional spot bump), and a monotonic counter. Cloning yields an independent
/// stream from the same starting point (same seed ⇒ same sequence).
#[derive(Debug, Clone)]
pub struct TickSource {
    base: MarketState,
    seed: u64,
    bump: f64,
    counter: u64,
}

impl TickSource {
    /// Construct a tick source over a base market state.
    ///
    /// `seed` fixes the (counter-based) sequence; `bump` is the maximum
    /// fractional spot move per tick (e.g. `0.001` = up to ±10 bp). A `bump` of
    /// `0.0` yields a constant-spot stream (useful for a pure heartbeat).
    #[must_use]
    pub fn new(base: MarketState, seed: u64, bump: f64) -> Self {
        Self {
            base,
            seed,
            bump,
            counter: 0,
        }
    }

    /// The base spot the bumps are applied around.
    #[must_use]
    pub fn base_spot(&self) -> f64 {
        self.base.spot
    }

    /// The number of ticks produced so far.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.counter
    }

    /// Produce the next market state: the base with a deterministically-bumped
    /// spot.
    ///
    /// The bump is `base_spot · (1 + bump · u)` with `u ∈ [-1, 1)` derived from
    /// `splitmix64(seed ⊕ counter)`, so the spot stays strictly positive for any
    /// `bump < 1.0`. Advances the internal counter.
    #[must_use]
    pub fn next_state(&mut self) -> MarketState {
        let mixed = splitmix64(self.seed ^ self.counter.wrapping_mul(0x2545_F491_4F6C_DD1D));
        self.counter = self.counter.wrapping_add(1);
        let u = unit_signed(mixed);
        let spot = self.base.spot * u.mul_add(self.bump, 1.0);
        let mut state = self.base.clone();
        state.spot = spot;
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_engine::testing::make_state;
    use celnet_types::{CcyPair, Tenor};

    fn conv() -> celnet_conventions::ConventionRecord {
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
    }

    #[test]
    fn unit_signed_is_in_range() {
        for c in 0u64..1000 {
            let u = unit_signed(splitmix64(c));
            assert!((-1.0..1.0).contains(&u), "u={u} out of range");
        }
    }

    #[test]
    fn same_seed_same_sequence() {
        let base = make_state(1.10, conv());
        let mut a = TickSource::new(base.clone(), 42, 0.001);
        let mut b = TickSource::new(base, 42, 0.001);
        for _ in 0..50 {
            let sa = a.next_state();
            let sb = b.next_state();
            // Bit-exact determinism: identical seed ⇒ identical spot path.
            assert_eq!(sa.spot.to_bits(), sb.spot.to_bits());
        }
        assert_eq!(a.count(), 50);
    }

    #[test]
    fn spot_stays_positive_and_near_base() {
        let base = make_state(1.10, conv());
        let mut s = TickSource::new(base, 7, 0.01);
        for _ in 0..1000 {
            let st = s.next_state();
            assert!(st.spot > 0.0);
            // Within ±1% of base by construction.
            assert!((st.spot - 1.10).abs() <= 1.10 * 0.01 + 1e-12);
        }
    }

    #[test]
    fn zero_bump_is_constant() {
        let base = make_state(1.23, conv());
        let mut s = TickSource::new(base, 99, 0.0);
        for _ in 0..10 {
            assert_eq!(s.next_state().spot.to_bits(), 1.23_f64.to_bits());
        }
    }
}
