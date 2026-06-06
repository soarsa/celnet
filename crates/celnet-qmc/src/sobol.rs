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
    #[must_use]
    pub fn point_u32(&self, i: u64) -> Vec<u32> {
        let g = i ^ (i >> 1);
        let mut out = vec![0u32; self.dim];
        for (j, slot) in out.iter_mut().enumerate() {
            let mut acc = 0u32;
            let mut bits = g;
            let mut k = 0;
            while bits != 0 && k < BITS as usize {
                if bits & 1 == 1 {
                    acc ^= self.v[j][k];
                }
                bits >>= 1;
                k += 1;
            }
            *slot = acc;
        }
        out
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
