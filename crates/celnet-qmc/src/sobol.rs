//! Scrambled Joe-Kuo Sobol' sequence generator (gray-code, 32-bit).
//!
//! A [`SobolSequence`] precomputes, for each of `dim` dimensions, the 32
//! *direction numbers* `v_{j,k}` (each a 32-bit fixed-point fraction in `(0,1)`)
//! from the embedded Joe-Kuo table ([`crate::direction_numbers`]). The `i`-th
//! point's `j`-th coordinate is then the bitwise XOR of the direction numbers
//! selected by the set bits of the gray code `g(i) = i ⊕ (i >> 1)`:
//!
//! ```text
//! x_{i,j} = (⊕_{k : bit k of g(i) set} v_{j,k}) / 2^32
//! ```
//!
//! Successive points differ in exactly one gray-code bit, so a streaming
//! generator updates each coordinate with a single XOR ([`SobolStream`]) — the
//! standard `O(1)`-per-point recurrence. Dimension 1 (`j = 0`) uses the
//! identity direction numbers `v_{0,k} = 2^{31-k}`, which makes `x_{i,0}` exactly
//! the van der Corput / bit-reversal radical-inverse sequence in base 2.
//!
//! # Scrambling — Owen-style nested digital scramble
//!
//! To obtain an **unbiased** randomized-QMC estimator with a usable variance
//! estimate from independent replications, each coordinate is passed through an
//! Owen-style *nested* digital scramble: the output bit at depth `d` is XORed
//! with a hash of the *prefix* of the higher-order bits (depths `0..d`) together
//! with the dimension and a per-replication seed. Owen scrambling preserves the
//! `(t,m,s)`-net equidistribution exactly (it permutes within each elementary
//! interval) while making each scrambled point uniformly distributed, so the
//! mean over scrambles is the exact integral and the between-scramble variance is
//! an unbiased error estimate. We realise the nested permutation with a fast
//! mixing hash (SplitMix64) of `(seed, dim, depth, prefix)` — a deterministic,
//! bit-reproducible Owen-style scramble.
//!
//! # Method provenance (doc comments only)
//!
//! Sobol' (1967); gray-code enumeration: Antonov-Saleev (1979). Joe & Kuo (2008)
//! direction numbers. Nested digital scrambling: Owen (1995, 1997); the
//! hash-realised nested scramble follows Burley (2020, *Practical Hash-based
//! Owen Scrambling*). SplitMix64: Steele-Lea-Flood (2014). Identifiers are
//! purpose-named; provenance lives only here.
//!
//! # Determinism & reuse
//!
//! No floating-point transcendentals are used in the integer core; the direction
//! numbers and scramble are pure `u32`/`u64` arithmetic, so the table and the
//! point construction are **bit-identical across platforms** and can be reused
//! verbatim by a future GPU backend (the same `v_{j,k}` upload to device memory).

use crate::direction_numbers::{DIR_ROWS, MAX_DIM};

/// Number of fixed-point bits per Sobol coordinate.
pub(crate) const BITS: u32 = 32;

/// 32-bit SplitMix64-based avalanche hash (used for the Owen-style scramble).
#[inline]
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Precomputed direction numbers for a fixed dimensionality.
///
/// `v[j][k]` is the `k`-th 32-bit direction number of dimension `j`
/// (`k = 0..32`, `j = 0..dim`).
pub struct SobolSequence {
    dim: usize,
    /// `dim × 32` direction numbers, row-major by dimension.
    v: Vec<[u32; 32]>,
}

impl SobolSequence {
    /// Build the Sobol direction numbers for `dim` dimensions.
    ///
    /// # Panics
    ///
    /// Panics if `dim == 0` or `dim > `[`MAX_DIM`] — the embedded Joe-Kuo table
    /// supports a **documented maximum** of [`MAX_DIM`] dimensions (a gated bound;
    /// see [`crate::direction_numbers`]).
    #[must_use]
    pub fn new(dim: usize) -> Self {
        assert!(dim >= 1, "Sobol dimension must be >= 1");
        assert!(
            dim <= MAX_DIM,
            "Sobol dimension {dim} exceeds the embedded Joe-Kuo table maximum {MAX_DIM}"
        );
        let mut v = Vec::with_capacity(dim);

        // Dimension 1 (j = 0): identity direction numbers v_k = 2^{31-k}, giving
        // the van der Corput / bit-reversal sequence in base 2.
        let mut v0 = [0u32; 32];
        for (k, slot) in v0.iter_mut().enumerate() {
            *slot = 1u32 << (BITS - 1 - k as u32);
        }
        v.push(v0);

        // Dimensions 2..=dim (j >= 1): build from the Joe-Kuo (s, a, m) row using
        // the canonical recurrence operating directly on the 32-bit left-justified
        // direction integers `V[k] = m_k · 2^{32-k}` (the form in Joe & Kuo's
        // reference `sobol.cc`):
        //   for k > s:  V[k] = V[k-s] ⊕ (V[k-s] >> s)
        //                      ⊕ ⊕_{i=1}^{s-1} a_i · V[k-i],
        // where the coefficient bit a_i = (a >> (s-1-i)) & 1 (MSB = a_1).
        for j in 1..dim {
            let row = &DIR_ROWS[j - 1]; // DIR_ROWS[i] holds dimension i+2 == (j+1)
            let s = row.s as usize;
            let a = row.a;
            let mut vk = [0u32; 33]; // 1-based V[1..=32]
            // Seed V[k] = m_k · 2^{32-k} for the first `min(s, 32)` directions.
            let seed_count = s.min(BITS as usize);
            for (idx, &mk) in row.m.iter().take(seed_count).enumerate() {
                let k = idx + 1; // 1-based direction index
                vk[k] = mk << (BITS - k as u32);
            }
            if (BITS as usize) > s {
                // Extend by the canonical recurrence for k > s (cross-indexes
                // vk[k-s]/vk[k-i], so the index is structurally required).
                for k in (s + 1)..=(BITS as usize) {
                    let mut val = vk[k - s] ^ (vk[k - s] >> s as u32);
                    for i in 1..s {
                        let a_i = (a >> (s - 1 - i)) & 1;
                        if a_i == 1 {
                            val ^= vk[k - i];
                        }
                    }
                    vk[k] = val;
                }
            }
            let mut vj = [0u32; 32];
            for (k, slot) in vj.iter_mut().enumerate() {
                *slot = vk[k + 1];
            }
            v.push(vj);
        }

        Self { dim, v }
    }

    /// Dimensionality of the sequence.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The raw 32 direction numbers of dimension `j` (`j < dim`).
    ///
    /// Exposed so a GPU backend can upload the identical table and reproduce the
    /// sequence on-device.
    #[must_use]
    pub fn direction_numbers(&self, j: usize) -> &[u32; 32] {
        &self.v[j]
    }

    /// The `i`-th **unscrambled** point as raw 32-bit integers (gray-code XOR of
    /// the direction numbers selected by the set bits of `g(i) = i ⊕ (i >> 1)`).
    ///
    /// The construction is 32-bit: exactly the low [`BITS`] gray-code bits
    /// select direction numbers, so the sequence period is `2^32` (indices
    /// beyond it reduce to their low 32 gray bits — a documented contract, and
    /// the loop below is structurally total over the 32 direction numbers).
    #[must_use]
    pub fn point_u32(&self, i: u64) -> Vec<u32> {
        let g = i ^ (i >> 1);
        self.v
            .iter()
            .map(|vj| {
                vj.iter()
                    .enumerate()
                    .filter(|(k, _)| (g >> k) & 1 == 1)
                    .fold(0u32, |acc, (_, &vk)| acc ^ vk)
            })
            .collect()
    }

    /// The `i`-th unscrambled point as `f64` coordinates in `[0, 1)`.
    #[must_use]
    pub fn point(&self, i: u64) -> Vec<f64> {
        self.point_u32(i)
            .into_iter()
            .map(|u| u as f64 * U32_TO_UNIT)
            .collect()
    }

    /// The `i`-th **Owen-scrambled** point as `f64` coordinates in `(0, 1)`,
    /// using replication seed `seed`.
    #[must_use]
    pub fn scrambled_point(&self, i: u64, seed: u64) -> Vec<f64> {
        self.point_u32(i)
            .into_iter()
            .enumerate()
            .map(|(j, u)| owen_scramble_u32(u, j as u64, seed))
            .map(u32_to_open_unit)
            .collect()
    }

    /// A streaming generator over the **scrambled** sequence (one XOR per
    /// coordinate per step via the gray-code recurrence). Far cheaper than
    /// repeated [`Self::scrambled_point`] when consuming the sequence in order.
    #[must_use]
    pub fn stream(&self, seed: u64) -> SobolStream<'_> {
        SobolStream {
            seq: self,
            seed,
            i: 0,
            state: vec![0u32; self.dim],
        }
    }
}

/// `2^-32` — scales a 32-bit integer fraction into `[0, 1)`.
const U32_TO_UNIT: f64 = 1.0 / 4_294_967_296.0;

/// Map a scrambled 32-bit integer into the **open** interval `(0, 1)`, biasing by
/// half an LSB so a zero integer never yields exactly `0.0` (which would make the
/// inverse-normal map produce `-∞`).
#[inline]
fn u32_to_open_unit(u: u32) -> f64 {
    (u as f64 + 0.5) * U32_TO_UNIT
}

/// Hash-based nested (Owen) digital scramble of a single 32-bit coordinate.
///
/// Processes bits MSB→LSB; the output bit at depth `d` is flipped by a hash of
/// the already-emitted prefix together with `(seed, dim, depth)`. Because the
/// flip at depth `d` depends only on the higher-order bits, the map is a valid
/// nested digital permutation — it permutes points within (but never across)
/// every dyadic elementary interval, preserving `(t,m,s)`-net structure while
/// randomising the point.
#[inline]
pub(crate) fn owen_scramble_u32(x: u32, dim: u64, seed: u64) -> u32 {
    let base = mix64(seed ^ mix64(dim.wrapping_mul(0x2545_f491_4f6c_dd1d)));
    let mut out = 0u32;
    let mut prefix = 0u32; // scrambled bits emitted so far (MSB-aligned)
    for d in 0..BITS {
        let in_bit = (x >> (BITS - 1 - d)) & 1;
        // Flip is a hash of (base, depth, prefix-so-far).
        let h = mix64(base ^ ((d as u64) << 40) ^ ((prefix as u64) << 1));
        let flip = (h & 1) as u32;
        let out_bit = in_bit ^ flip;
        out |= out_bit << (BITS - 1 - d);
        prefix = (prefix << 1) | out_bit;
    }
    out
}

/// A streaming view over a [`SobolSequence`]'s scrambled points, advancing by the
/// `O(1)` gray-code recurrence.
pub struct SobolStream<'a> {
    seq: &'a SobolSequence,
    seed: u64,
    i: u64,
    state: Vec<u32>,
}

impl SobolStream<'_> {
    /// Emit the next scrambled point as `f64` coordinates in `(0, 1)`, advancing
    /// the recurrence. Returns the point and increments the internal index.
    pub fn next_point(&mut self, out: &mut [f64]) {
        debug_assert_eq!(out.len(), self.seq.dim);
        if self.i == 0 {
            // Point 0 of the gray-code sequence is all-zero integers.
            for s in self.state.iter_mut() {
                *s = 0;
            }
        } else {
            // The gray codes g(i-1) and g(i) differ in exactly bit
            // `c = trailing_zeros(i)`; XOR each coordinate's c-th direction number.
            let c = self.i.trailing_zeros() as usize;
            for (j, s) in self.state.iter_mut().enumerate() {
                *s ^= self.seq.v[j][c];
            }
        }
        for (j, o) in out.iter_mut().enumerate() {
            *o = u32_to_open_unit(owen_scramble_u32(self.state[j], j as u64, self.seed));
        }
        self.i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny deterministic LCG for in-test sample generation (test-only; not a
    /// statistical claim — just a spread of bit patterns).
    fn lcg(state: &mut u64) -> u32 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (*state >> 32) as u32
    }

    /// The defining property of a *nested* digital permutation: the top `k`
    /// output bits depend only on the top `k` input bits (the scramble permutes
    /// points within — never across — dyadic elementary intervals). For any two
    /// inputs sharing a `k`-bit prefix, the outputs must share a `k`-bit prefix.
    #[test]
    fn owen_scramble_preserves_dyadic_prefixes() {
        let mut state = 0x1234_5678_9abc_def0u64;
        for trial in 0..200 {
            let x = lcg(&mut state);
            let y = lcg(&mut state);
            let dim = u64::from(lcg(&mut state) % 64);
            let seed = u64::from(lcg(&mut state));
            for k in [1u32, 4, 9, 17, 31] {
                // Force y to share x's top-k bits.
                let mask = !0u32 << (BITS - k);
                let y_shared = (x & mask) | (y & !mask);
                let sx = owen_scramble_u32(x, dim, seed);
                let sy = owen_scramble_u32(y_shared, dim, seed);
                assert_eq!(
                    sx & mask,
                    sy & mask,
                    "prefix k={k} not preserved (trial {trial})"
                );
            }
        }
    }

    /// A digital permutation is bijective on every prefix length: the 256
    /// possible top-byte patterns must map onto a permutation of all 256
    /// top-byte patterns (for any fixed `(dim, seed)`).
    #[test]
    fn owen_scramble_top_byte_is_a_permutation() {
        for (dim, seed) in [(0u64, 1u64), (3, 0xDEAD_BEEF), (63, 42)] {
            let mut seen = [false; 256];
            for k in 0u32..256 {
                let out = owen_scramble_u32(k << 24, dim, seed) >> 24;
                assert!(
                    !seen[out as usize],
                    "top byte {out} hit twice (dim={dim}, seed={seed})"
                );
                seen[out as usize] = true;
            }
            assert!(seen.iter().all(|&b| b));
        }
    }

    /// The streaming gray-code recurrence must agree bit-for-bit with the direct
    /// `point_u32` construction (before scrambling).
    #[test]
    fn stream_matches_direct_unscrambled() {
        let seq = SobolSequence::new(8);
        // Reconstruct the unscrambled integer state by replaying the recurrence.
        let mut state = vec![0u32; 8];
        for i in 0..256u64 {
            if i != 0 {
                let c = i.trailing_zeros() as usize;
                for (j, s) in state.iter_mut().enumerate() {
                    *s ^= seq.v[j][c];
                }
            }
            assert_eq!(state, seq.point_u32(i), "mismatch at i={i}");
        }
    }
}
