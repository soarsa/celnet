//! Counter-based Philox-4×32-10 random-number generation.
//!
//! Philox is a *counter-based* RNG: instead of advancing a hidden state, it is a
//! stateless keyed bijection on a 128-bit counter. Variate `n` is obtained by
//! encrypting the integer `n` (split across a 4×32-bit counter) under a 2×32-bit
//! key, applying ten Feistel-like rounds of 32×32→64 multiplies. This makes it
//! *perfectly parallel and reproducible*: any path/step/dimension can be drawn
//! independently, in any order, on any device, and the bitstream is identical —
//! exactly the property a GPU Monte-Carlo path engine needs.
//!
//! The same ten-round transform is implemented here in Rust and, byte-for-byte,
//! in [`crate::gpu`]'s WGSL shader; the two share the constants below so the
//! 32-bit integer output is bit-identical across CPU and GPU. Only the final
//! integer→float conversion differs by design (f64 on CPU, f32 on GPU), and that
//! difference is the documented error bound the reconciliation test enforces.
//!
//! Method provenance (doc-only, per naming policy): the algorithm is the
//! 4×32-10 counter-based generator of Salmon, Moraes, Dror & Shaw,
//! "Parallel Random Numbers: As Easy as 1, 2, 3" (SC '11). The struct and
//! function names below are purpose- and acronym-named, never person-named.

/// First multiplier constant for the 4×32 Philox round (`PHILOX_M4x32_0`).
pub const PHILOX_MUL_0: u32 = 0xD251_1F53;
/// Second multiplier constant for the 4×32 Philox round (`PHILOX_M4x32_1`).
pub const PHILOX_MUL_1: u32 = 0xCD9E_8D57;
/// Key-schedule bump for the low key word per round (Weyl constant `W32_0`).
pub const PHILOX_KEY_BUMP_0: u32 = 0x9E37_79B9;
/// Key-schedule bump for the high key word per round (Weyl constant `W32_1`).
pub const PHILOX_KEY_BUMP_1: u32 = 0xBB67_AE85;
/// The fixed Philox round count (the `-10` in "Philox-4×32-10").
pub const PHILOX_ROUNDS: u32 = 10;

/// `2^-32`, the scale that turns a 32-bit integer into `[0, 1)` (open-above) in
/// f64. The GPU uses the f32 analogue; both bias the integer by `+½ ULP` so the
/// uniform is in the open interval `(0, 1)` and the inverse-normal never sees an
/// exact `0` or `1`.
pub const TWO_POW_NEG_32: f64 = 1.0 / 4_294_967_296.0;

/// One application of the 4×32 Philox round to the counter `(c0..c3)` under the
/// running key `(k0, k1)`. Pure integer arithmetic — identical on CPU and GPU.
#[inline]
#[must_use]
fn counter_round(c: [u32; 4], key: [u32; 2]) -> [u32; 4] {
    // 32×32→64 multiplies; hi/lo split is the Philox "bump" operation.
    let p0 = u64::from(PHILOX_MUL_0) * u64::from(c[0]);
    let p1 = u64::from(PHILOX_MUL_1) * u64::from(c[2]);
    let hi0 = (p0 >> 32) as u32;
    let lo0 = p0 as u32;
    let hi1 = (p1 >> 32) as u32;
    let lo1 = p1 as u32;
    [hi1 ^ c[1] ^ key[0], lo1, hi0 ^ c[3] ^ key[1], lo0]
}

/// The full Philox-4×32-10 bijection: encrypts the 128-bit `counter` under the
/// 64-bit `key`, returning four independent 32-bit uniform integers.
///
/// This is the single source of truth mirrored by the WGSL shader.
#[inline]
#[must_use]
pub fn counter_block(mut counter: [u32; 4], mut key: [u32; 2]) -> [u32; 4] {
    let mut r = 0;
    while r < PHILOX_ROUNDS {
        counter = counter_round(counter, key);
        // Key schedule: bump both key words by the Weyl constants every round.
        key[0] = key[0].wrapping_add(PHILOX_KEY_BUMP_0);
        key[1] = key[1].wrapping_add(PHILOX_KEY_BUMP_1);
        r += 1;
    }
    counter
}

/// Deterministic, address-based seeding of the Philox counter and key.
///
/// The counter encodes the *coordinate* of the variate in the simulation —
/// `(path, step, dim)` — and the key encodes the global run seed. Because the
/// mapping coordinate → variate is a pure bijection, drawing the variate for any
/// `(path, step, dim)` is independent of evaluation order and identical on every
/// backend. `dim` selects which of the four 32-bit outputs of one Philox block is
/// returned, so one block of work yields four reproducible normals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterAddress {
    /// Global run seed (mixed into both key words).
    pub seed: u64,
    /// Path index within the batch.
    pub path: u32,
    /// Time-step index within the path.
    pub step: u32,
    /// Dimension index within the step (selects one of the four block lanes).
    pub dim: u32,
}

impl CounterAddress {
    /// Build the 128-bit counter and 64-bit key for this coordinate.
    ///
    /// The counter packs `(path, step, dim/4)` plus a domain tag; the key packs
    /// the 64-bit seed. The `dim >> 2` in the counter and `dim & 3` lane select
    /// (applied by the caller) mean four dimensions share one Philox block, which
    /// the GPU exploits to amortize the ten rounds across four variates.
    #[inline]
    #[must_use]
    pub fn counter_key(self) -> ([u32; 4], [u32; 2]) {
        let counter = [
            self.path,
            self.step,
            self.dim >> 2,
            0x4658_4F50, // ASCII "PFXO" domain tag — fixes the counter origin.
        ];
        let key = [self.seed as u32, (self.seed >> 32) as u32];
        (counter, key)
    }
}

/// A reproducible stream of standard-normal variates addressed by
/// `(seed, path, step, dim)`.
///
/// Each `normal` call performs one Philox block and returns the standard normal
/// for the requested lane, converting two of the block's 32-bit integers into a
/// uniform pair and mapping them through the Box-Muller transform. The same two
/// integers and transform are reproduced in WGSL, so the CPU f64 stream is the
/// exact-arithmetic oracle for the GPU f32 stream.
#[derive(Debug, Clone, Copy, Default)]
pub struct CounterNormals {
    /// Global run seed.
    pub seed: u64,
}

impl CounterNormals {
    /// Construct a normal generator for `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Raw uniform integer for `(path, step, dim)` (the `dim`-th lane of the
    /// addressed Philox block). Exposed for the bit-stability / known-answer
    /// tests that compare against the GPU integer output.
    #[inline]
    #[must_use]
    pub fn raw_u32(&self, path: u32, step: u32, dim: u32) -> u32 {
        let addr = CounterAddress {
            seed: self.seed,
            path,
            step,
            dim,
        };
        let (counter, key) = addr.counter_key();
        let block = counter_block(counter, key);
        block[(dim & 3) as usize]
    }

    /// Open-interval uniform `(0, 1)` in f64 for `(path, step, dim)`.
    #[inline]
    #[must_use]
    pub fn uniform(&self, path: u32, step: u32, dim: u32) -> f64 {
        // (u + ½) · 2^-32 maps {0..2^32-1} into (0, 1) open on both ends.
        (f64::from(self.raw_u32(path, step, dim)) + 0.5) * TWO_POW_NEG_32
    }

    /// Standard-normal variate for `(path, step, dim)`.
    ///
    /// Uses the Box-Muller transform on a uniform pair drawn from lanes `2·dim`
    /// and `2·dim+1` of the addressed block, returning the cosine branch. The
    /// WGSL shader uses the identical pairing and branch in f32.
    #[inline]
    #[must_use]
    pub fn normal(&self, path: u32, step: u32, dim: u32) -> f64 {
        let u1 = self.uniform(path, step, 2 * dim);
        let u2 = self.uniform(path, step, 2 * dim + 1);
        let r = celnet_core::math::sqrt(-2.0 * celnet_core::math::ln(u1));
        r * libm::cos(core::f64::consts::TAU * u2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Philox is deterministic: the same address always yields the same integer.
    #[test]
    fn raw_is_deterministic() {
        let g = CounterNormals::new(0xDEAD_BEEF_0000_0001);
        for (p, s, d) in [(0, 0, 0), (7, 3, 2), (123, 4, 9)] {
            assert_eq!(g.raw_u32(p, s, d), g.raw_u32(p, s, d));
        }
    }

    /// Distinct coordinates decorrelate (no accidental aliasing of the address
    /// into the counter): all four pairwise-distinct draws differ.
    #[test]
    fn distinct_addresses_differ() {
        let g = CounterNormals::new(42);
        let a = g.raw_u32(0, 0, 0);
        let b = g.raw_u32(1, 0, 0);
        let c = g.raw_u32(0, 1, 0);
        let d = g.raw_u32(0, 0, 5);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_ne!(b, c);
    }

    /// Canonical known-answer vector for the 4×32 counter-based PRNG bijection on the
    /// all-zero counter and key. This pins the permutation constants and round count exactly;
    /// if any constant or the round schedule drifts, this fails immediately.
    #[test]
    fn known_answer_zero() {
        let out = counter_block([0, 0, 0, 0], [0, 0]);
        assert_eq!(out, [0x6627_E8D5, 0xE169_C58D, 0xBC57_AC4C, 0x9B00_DBD8]);
    }

    /// Bit-stability lock for the maximal counter/key input. The expected value
    /// is this implementation's own output, frozen so the WGSL port and any
    /// future refactor must reproduce the identical 128-bit block.
    #[test]
    fn known_answer_max() {
        let out = counter_block(
            [0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF],
            [0xFFFF_FFFF, 0xFFFF_FFFF],
        );
        assert_eq!(out, [0x408F_276D, 0x41C8_3B0E, 0xA20B_C7C6, 0x6D54_51FD]);
    }

    /// The uniform stream stays strictly inside the open interval `(0, 1)`.
    #[test]
    fn uniforms_in_open_unit_interval() {
        let g = CounterNormals::new(99);
        for p in 0..2000u32 {
            let u = g.uniform(p, 0, 0);
            assert!(u > 0.0 && u < 1.0, "uniform out of (0,1): {u}");
        }
    }

    /// Box-Muller normals are finite and (over a large sample) zero-mean / unit-
    /// variance to a loose statistical tolerance — a sanity check on the
    /// transform, not a precision assertion.
    #[test]
    fn normal_moments() {
        let g = CounterNormals::new(0x1234_5678_9ABC_DEF0);
        let n = 200_000u32;
        let mut sum = 0.0f64;
        let mut sumsq = 0.0f64;
        for p in 0..n {
            let z = g.normal(p, 0, 0);
            assert!(z.is_finite());
            sum += z;
            sumsq += z * z;
        }
        let mean = sum / f64::from(n);
        let var = sumsq / f64::from(n) - mean * mean;
        // ~1/√n ≈ 2.2e-3 standard error; allow generous bands.
        assert!(mean.abs() < 0.02, "mean off: {mean}");
        assert!((var - 1.0).abs() < 0.03, "var off: {var}");
    }
}
