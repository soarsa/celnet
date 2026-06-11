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
//! comment; the public type is purpose-named [`CounterRng`] (an established
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
/// the work item; successive calls to [`CounterRng::next_u01`] draw the
/// `draw = 0, 1, 2, …` words of the corresponding counter block, refilling a new
/// block every four draws. The mapping from coordinates to output is pure, so
/// two streams built with identical coordinates yield identical sequences.
#[derive(Debug, Clone)]
pub struct CounterRng {
    key0: u32,
    key1: u32,
    /// Counter words 0–1: the full 64-bit `path` index — fixed for this stream.
    /// `path` occupies its own two disjoint 32-bit lanes, so the coordinate map is
    /// obviously injective (no folding/aliasing).
    path: u64,
    /// Counter word 2: the caller-chosen `step` base offset, disjoint from `path`
    /// and `draw` so distinct simulation steps never collide.
    step_base: u32,
    /// Counter word 3: the within-block draw index, advanced every refill.
    draw: u32,
    cache: [u32; 4],
    cache_len: u8,
}

impl CounterRng {
    /// Open the stream for work item `(stream, path, step)` under `seed`.
    ///
    /// `seed` keys the bijection; `stream` separates independent uses of the
    /// engine (e.g. the main estimate vs. an antithetic or control replica),
    /// `path` is the Monte-Carlo path index, and `step` is the time-step base.
    /// Distinct tuples address disjoint regions of the counter/key space, so
    /// their variate sequences are statistically independent yet each is exactly
    /// reproducible from its tuple. The coordinate→counter map is a strict
    /// partition — `path` fills counter words 0–1, `step` word 2, `draw` word 3,
    /// and `stream` keys the bijection (XOR-folded into `key1`) — so it is
    /// obviously injective in `(path, step, draw)` for a fixed `(seed, stream)`,
    /// and independent across `stream` (which perturbs the Philox key). No ad-hoc
    /// hashing of the coordinates into a single lane, so two distinct
    /// `(stream, path)` tuples can never alias the same counter prefix.
    #[must_use]
    pub fn new(seed: u64, stream: u32, path: u64, step: u32) -> Self {
        Self {
            key0: seed as u32,
            key1: (seed >> 32) as u32 ^ stream,
            path,
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
            self.path as u32,
            (self.path >> 32) as u32,
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
        let mut a = CounterRng::new(0xDEAD_BEEF_CAFE_F00D, 3, 42, 7);
        let mut b = CounterRng::new(0xDEAD_BEEF_CAFE_F00D, 3, 42, 7);
        for _ in 0..1000 {
            let (x, y) = (a.next_u01(), b.next_u01());
            assert_eq!(x.to_bits(), y.to_bits(), "streams diverged");
        }
    }

    /// Distinct coordinates ⇒ distinct sequences (no accidental aliasing across
    /// stream / path / step).
    #[test]
    fn distinct_coordinates_differ() {
        let base = CounterRng::new(1, 0, 0, 0).key_words();
        let s1 = CounterRng::new(1, 1, 0, 0).key_words();
        // Different stream perturbs the key schedule.
        assert_ne!(base, s1);

        let mut p0 = CounterRng::new(1, 0, 0, 0);
        let mut p1 = CounterRng::new(1, 0, 1, 0);
        let mut p2 = CounterRng::new(1, 0, 0, 1);
        let v0: Vec<u64> = (0..8).map(|_| p0.next_u01().to_bits()).collect();
        let v1: Vec<u64> = (0..8).map(|_| p1.next_u01().to_bits()).collect();
        let v2: Vec<u64> = (0..8).map(|_| p2.next_u01().to_bits()).collect();
        assert_ne!(v0, v1, "path index must change the stream");
        assert_ne!(v0, v2, "step base must change the stream");
    }

    /// Injective coordinate map: sweeping a large block of `path` indices (and a
    /// few adversarial high-bit / power-of-two patterns that the previous ad-hoc
    /// `((stream<<32)^path).rotate_left(17) ^ (path<<1)` mix could alias) yields a
    /// first output word that is *unique* per `path`. This pins the counter-based
    /// generator's defining property — a provably-injective coordinate → substream
    /// map — rather than spot-checking a handful of tuples (audit `rng.rs:103`).
    #[test]
    fn distinct_paths_do_not_collide_in_first_block() {
        use std::collections::HashSet;
        let mut seen: HashSet<u64> = HashSet::new();
        let mut first_words = |seed: u64, stream: u32| {
            for path in 0u64..50_000 {
                let mut s = CounterRng::new(seed, stream, path, 0);
                // 64-bit fingerprint of the first two draws of the path's block.
                let fp = (s.next_u01().to_bits() ^ (path.wrapping_mul(0)))
                    ^ s.next_u01().to_bits().rotate_left(32);
                assert!(
                    seen.insert(fp.wrapping_add(u64::from(stream))),
                    "path {path} (stream {stream}) collided in the first block"
                );
            }
        };
        first_words(0xA5A5_1234_DEAD_0001, 0);
        // Adversarial high-bit/aliasing-prone path patterns under a second stream.
        let mut s = HashSet::new();
        for &path in &[
            0u64,
            1,
            1 << 16,
            1 << 17,
            1 << 31,
            1 << 32,
            1 << 33,
            (1 << 32) | 1,
            u32::MAX as u64,
            (u32::MAX as u64) << 1,
            u64::MAX,
            u64::MAX - 1,
        ] {
            let mut st = CounterRng::new(7, 2, path, 0);
            let fp = st.next_u01().to_bits() ^ st.next_u01().to_bits().rotate_left(17);
            assert!(s.insert(fp), "adversarial path {path} collided");
        }
    }

    /// The 4×32/10-round keyed bijection pinned against the PUBLISHED
    /// known-answer vectors of the reference implementation (Salmon, Moraes,
    /// Dror & Shaw 2011, the Random123 `kat_vectors` file, `philox4x32 10`
    /// rows), independently cross-verified against a from-the-paper
    /// re-implementation before being typed in. Any mutant of the round
    /// arithmetic (multiplier lanes, hi/lo split, xor-mix wiring, key
    /// schedule, round count) changes these words — the determinism anchor the
    /// `same_seed` self-consistency test alone cannot provide (a mutant
    /// perturbs both replicas identically).
    #[test]
    fn block_matches_published_known_answer_vectors() {
        assert_eq!(
            block([0, 0, 0, 0], 0, 0),
            [0x6627_e8d5, 0xe169_c58d, 0xbc57_ac4c, 0x9b00_dbd8]
        );
        assert_eq!(
            block([u32::MAX; 4], u32::MAX, u32::MAX),
            [0x408f_276d, 0x41c8_3b0e, 0xa20b_c7c6, 0x6d54_51fd]
        );
        assert_eq!(
            block(
                [0x243f_6a88, 0x85a3_08d3, 0x1319_8a2e, 0x0370_7344],
                0xa409_3822,
                0x299f_31d0
            ),
            [0xd16c_fe09, 0x94fd_cceb, 0x5001_e420, 0x2412_6ea1]
        );
    }

    /// The `(seed, stream, path, step)` → `(counter, key)` layout contract and
    /// the `u32 → (0,1)` scaling, pinned end-to-end through the public API:
    ///
    /// * the zero tuple reproduces the zero known-answer block word-by-word in
    ///   document order, then continues into the `draw = 1` block (so the
    ///   refill/cache indexing and the draw advance are all value-pinned);
    /// * a non-trivial tuple matches `block()` applied to the documented
    ///   layout `[path_lo, path_hi, step, 0]`, key `(seed_lo, seed_hi ⊕ stream)`
    ///   — a mutant anywhere in `new`/`refill`/`next_u01` breaks the
    ///   correspondence (while `block` itself is pinned externally above).
    #[test]
    fn stream_layout_and_scaling_match_known_answer() {
        let scale = |w: u32| (f64::from(w) + 0.5) / 4_294_967_296.0;
        let mut s = CounterRng::new(0, 0, 0, 0);
        for w in [0x6627_e8d5u32, 0xe169_c58d, 0xbc57_ac4c, 0x9b00_dbd8] {
            assert_eq!(s.next_u01().to_bits(), scale(w).to_bits());
        }
        // Fifth draw: word 0 of the draw=1 block.
        let next = block([0, 0, 0, 1], 0, 0);
        assert_eq!(s.next_u01().to_bits(), scale(next[0]).to_bits());

        // Layout: path → words 0–1, step → word 2, seed splits into the key,
        // stream XORs into the high key word.
        let mut t = CounterRng::new(
            0x1122_3344_5566_7788,
            0xA0B0_C0D0,
            0x0102_0304_0506_0708,
            0x99AA_0011,
        );
        let kat = block(
            [0x0506_0708, 0x0102_0304, 0x99AA_0011, 0],
            0x5566_7788,
            0x1122_3344 ^ 0xA0B0_C0D0,
        );
        for w in kat {
            assert_eq!(t.next_u01().to_bits(), scale(w).to_bits());
        }
    }

    /// Output lies strictly in `(0,1)` — never 0 or 1 (so the inverse-CDF is
    /// always finite).
    #[test]
    fn uniform_in_open_unit_interval() {
        let mut s = CounterRng::new(99, 0, 0, 0);
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
        let mut s = CounterRng::new(0x1234_5678, 5, 1, 1);
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

    impl CounterRng {
        /// Test helper: expose the derived key words for the aliasing check.
        fn key_words(&self) -> (u32, u32) {
            (self.key0, self.key1)
        }
    }
}
