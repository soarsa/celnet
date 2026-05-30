/**
 * A deterministic, seedable PRNG (SplitMix64 → xoshiro-style float draw) so the
 * mock/replay source is fully reproducible: the same seed yields the same tape,
 * mirroring the platform's determinism discipline (libm, no nondeterminism on
 * the pricing path). Used only by the standalone data source, never by render.
 */

export class Rng {
  private state: bigint;

  constructor(seed: bigint) {
    // Avoid the all-zero fixed point.
    this.state = seed === 0n ? 0x9e37_79b9_7f4a_7c15n : seed;
  }

  /** Next raw 64-bit value (SplitMix64). */
  private nextU64(): bigint {
    this.state = (this.state + 0x9e37_79b9_7f4a_7c15n) & 0xffff_ffff_ffff_ffffn;
    let z = this.state;
    z = ((z ^ (z >> 30n)) * 0xbf58_476d_1ce4_e5b9n) & 0xffff_ffff_ffff_ffffn;
    z = ((z ^ (z >> 27n)) * 0x94d0_49bb_1331_11ebn) & 0xffff_ffff_ffff_ffffn;
    return z ^ (z >> 31n);
  }

  /** Uniform double in [0, 1). */
  next(): number {
    // Top 53 bits → [0,1) with full mantissa precision.
    const bits = this.nextU64() >> 11n;
    return Number(bits) / 9_007_199_254_740_992; // 2^53
  }

  /** Uniform in [lo, hi). */
  range(lo: number, hi: number): number {
    return lo + (hi - lo) * this.next();
  }

  /** A standard normal draw via the Box-Muller transform (deterministic). */
  normal(): number {
    const u1 = Math.max(1e-12, this.next());
    const u2 = this.next();
    return Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
  }
}
