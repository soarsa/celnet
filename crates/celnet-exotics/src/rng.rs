//! Counter-based pseudo-random number generation for the Monte-Carlo engine.
//!
//! The generator is the 4×32-bit, 10-round counter-based stream of Salmon,
//! Moraes, Dror & Shaw (2011) ("Parallel random numbers: as easy as 1, 2, 3").
//! Unlike a stateful linear generator, a counter-based generator is a *keyed
//! bijection* on a 128-bit counter: the `n`-th output is a pure function
//! `f(key, counter)` with no carried state. That property is exactly what a
//! reproducible, embarrassingly-parallel pricing grid needs — any path/step can
//! be drawn independently and in any order, on CPU or GPU, and the result is
//! **bit-identical** because it never depends on iteration order or thread
//! scheduling.
//!
//! Here the 128-bit counter is partitioned as `(stream, path, step, draw)` so a
//! single 64-bit seed plus the logical coordinates of a draw fully determine the
//! uniform variate. Reseeding with the same seed reproduces every variate
//! bit-for-bit (asserted in the test suite), which is the determinism guarantee
//! the engine relies on.
//!
//! All arithmetic is integer (wrapping) and the only float operation is the
//! final fixed `u64 → (0,1)` scaling, so there is no transcendental and no
//! platform-dependent rounding on this path. Provenance lives only in this doc
//! comment; the public type is purpose-named [`PhiloxStream`] (an established
//! technical acronym, not a surname).

#![allow(clippy::unreadable_literal)]

/// The two 32-bit multipliers of the 4×32 counter-based bijection.
const M0: u64 = 0xD2511F53;
const M1: u64 = 0xCD9E8D57;
/// The two 32-bit key-schedule "Weyl" increments (golden-ratio / √3 fractions).
const W0: u32 = 0x9E3779B9;
const W1: u32 = 0xBB67AE85;
/// Round count. Ten rounds is the standard cryptographically-conservative depth
/// at which the generator passes the full empirical-randomness batteries.
const ROUNDS: usize = 10;

/// Reciprocal of `2^32`, the fixed scale taking a 32-bit word to `[0,1)`.
const TWO_POW_32_INV: f64 = 1.0 / 4_294_967_296.0;

/// One 4×32 counter-based block: a keyed bijection of a 128-bit counter into
/// four 32-bit pseudo-random words.
#[inline]
fn block(counter: [u32; 4], key0: u32, key1: u32) -> [u32; 4] {
    let mut c = counter;
    let mut k0 = key0;
    let mut k1 = key1;
    let mut round = 0;
    loop {
        // Single round: two 64-bit multiplies, hi/lo split, xor-mix with the key.
        let p0 = M0 * u64::from(c[0]);
        let p1 = M1 * u64::from(c[2]);
        let hi0 = (p0 >> 32) as u32;
        let lo0 = p0 as u32;
        let hi1 = (p1 >> 32) as u32;
        let lo1 = p1 as u32;
        c = [hi1 ^ c[1] ^ k0, lo1, hi0 ^ c[3] ^ k1, lo0];
        round += 1;
        if round == ROUNDS {
            break;
        }
        // Bump the key (the key schedule) between rounds.
        k0 = k0.wrapping_add(W0);
        k1 = k1.wrapping_add(W1);
    }
    c
}

/// A reproducible, counter-based uniform stream over `(0,1)`.
///
/// Construct from a seed and the logical coordinates `(stream, path, step)` of
/// the work item; successive calls to [`PhiloxStream::next_u01`] draw the
/// `draw = 0, 1, 2, …` words of the corresponding counter block, refilling a new
/// block every four draws. The mapping from coordinates to output is pure, so
/// two streams built with identical coordinates yield identical sequences.
#[derive(Debug, Clone)]
pub struct PhiloxStream {
    key0: u32,
    key1: u32,
    /// High 64 bits of the counter: `(stream, path)` — fixed for this stream.
    hi: u64,
    /// Low 64 bits: `(step, draw)`. The draw counter advances; `step` is the
    /// caller-chosen base offset so distinct simulation steps never collide.
    step_base: u32,
    draw: u32,
    cache: [u32; 4],
    cache_len: u8,
}

impl PhiloxStream {
    /// Open the stream for work item `(stream, path, step)` under `seed`.
    ///
    /// `seed` keys the bijection; `stream` separates independent uses of the
    /// engine (e.g. the main estimate vs. an antithetic or control replica),
    /// `path` is the Monte-Carlo path index, and `step` is the time-step base.
    /// Distinct tuples address disjoint regions of the 128-bit counter space, so
    /// their variate sequences are statistically independent yet each is exactly
    /// reproducible from its tuple.
    #[must_use]
    pub fn new(seed: u64, stream: u32, path: u64, step: u32) -> Self {
        Self {
            key0: seed as u32,
            key1: (seed >> 32) as u32 ^ stream,
            hi: ((u64::from(stream) << 32) ^ path).rotate_left(17) ^ (path << 1),
            step_base: step,
            draw: 0,
            cache: [0; 4],
            cache_len: 0,
        }
    }

    /// Draw the next uniform variate in the open interval `(0, 1)`.
    ///
    /// The `+0.5` centring guarantees the value is strictly inside `(0,1)` so it
    /// is always a legal argument to the inverse-CDF (the normal quantile
    /// diverges at the endpoints).
    #[must_use]
    pub fn next_u01(&mut self) -> f64 {
        if self.cache_len == 0 {
            self.refill();
        }
        let idx = 4 - self.cache_len as usize;
        let word = self.cache[idx];
        self.cache_len -= 1;
        (f64::from(word) + 0.5) * TWO_POW_32_INV
    }

    /// Recompute the next 4-word block from the current counter and advance the
    /// draw counter.
    #[inline]
    fn refill(&mut self) {
        let counter = [
            self.hi as u32,
            (self.hi >> 32) as u32,
            self.step_base,
            self.draw,
        ];
        self.cache = block(counter, self.key0, self.key1);
        self.cache_len = 4;
        self.draw = self.draw.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Identical coordinates ⇒ bit-identical sequence (the core determinism
    /// guarantee the MC engine depends on).
    #[test]
    fn same_seed_same_sequence_bit_identical() {
        let mut a = PhiloxStream::new(0xDEAD_BEEF_CAFE_F00D, 3, 42, 7);
        let mut b = PhiloxStream::new(0xDEAD_BEEF_CAFE_F00D, 3, 42, 7);
        for _ in 0..1000 {
            let (x, y) = (a.next_u01(), b.next_u01());
            assert_eq!(x.to_bits(), y.to_bits(), "streams diverged");
        }
    }

    /// Distinct coordinates ⇒ distinct sequences (no accidental aliasing across
    /// stream / path / step).
    #[test]
    fn distinct_coordinates_differ() {
        let base = PhiloxStream::new(1, 0, 0, 0).key_words();
        let s1 = PhiloxStream::new(1, 1, 0, 0).key_words();
        // Different stream perturbs the key schedule.
        assert_ne!(base, s1);

        let mut p0 = PhiloxStream::new(1, 0, 0, 0);
        let mut p1 = PhiloxStream::new(1, 0, 1, 0);
        let mut p2 = PhiloxStream::new(1, 0, 0, 1);
        let v0: Vec<u64> = (0..8).map(|_| p0.next_u01().to_bits()).collect();
        let v1: Vec<u64> = (0..8).map(|_| p1.next_u01().to_bits()).collect();
        let v2: Vec<u64> = (0..8).map(|_| p2.next_u01().to_bits()).collect();
        assert_ne!(v0, v1, "path index must change the stream");
        assert_ne!(v0, v2, "step base must change the stream");
    }

    /// Output lies strictly in `(0,1)` — never 0 or 1 (so the inverse-CDF is
    /// always finite).
    #[test]
    fn uniform_in_open_unit_interval() {
        let mut s = PhiloxStream::new(99, 0, 0, 0);
        for _ in 0..100_000 {
            let u = s.next_u01();
            assert!(u > 0.0 && u < 1.0, "u01 out of (0,1): {u}");
        }
    }

    /// First and second moments of the uniform stream match `U(0,1)`
    /// (mean ½, variance 1/12) to sampling tolerance — a sanity check that the
    /// bijection is well-mixed.
    #[test]
    fn uniform_moments() {
        let mut s = PhiloxStream::new(0x1234_5678, 5, 1, 1);
        let n = 1_000_000usize;
        let (mut sum, mut sumsq) = (0.0f64, 0.0f64);
        for _ in 0..n {
            let u = s.next_u01();
            sum += u;
            sumsq += u * u;
        }
        let mean = sum / n as f64;
        let var = sumsq / n as f64 - mean * mean;
        assert!((mean - 0.5).abs() < 2e-3, "mean {mean}");
        assert!((var - 1.0 / 12.0).abs() < 2e-3, "var {var}");
    }

    impl PhiloxStream {
        /// Test helper: expose the derived key words for the aliasing check.
        fn key_words(&self) -> (u32, u32) {
            (self.key0, self.key1)
        }
    }
}
