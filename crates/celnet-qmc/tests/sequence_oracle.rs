//! Independent-oracle tests for the quasi-Monte-Carlo stack (W6 rigor wave).
//!
//! Every assertion here pins `celnet-qmc` behaviour against a quantity derived
//! **outside** the implementation under test:
//!
//! * exact dyadic rationals of the base-2 radical-inverse / gray-code sequence
//!   (hand-derivable, pinned `to_bits`);
//! * an in-test re-implementation of the primitive-polynomial direction-number
//!   recurrence in the `m_k` domain (the implementation works in the
//!   left-justified 32-bit `V_k` domain — code-disjoint arithmetic), fed with
//!   rows typed directly from the published Joe-Kuo D(6) data file;
//! * published high-precision standard-normal quantiles plus an independent
//!   `Φ(x) = ½·erfc(−x/√2)` round-trip via the `libm` dev-dependency;
//! * the exact discrete Brownian covariance `Cov(W(t_i),W(t_j)) = min(t_i,t_j)`
//!   that any correct bridge factorization must reproduce;
//! * closed-form monomial integrals (`∫₀¹x dx = ½`, `∫₀¹x² dx = ⅓`) through the
//!   full Sobol → scramble → inverse-normal → bridge → estimator pipeline;
//! * frozen-bits regression rows (the house `fx_byte_identity` pattern): the
//!   integer Sobol/scramble core and the `libm`-routed transcendentals are
//!   bit-identical across platforms, so exact `u64` pins are stable. They are
//!   regression/mutation pins, not correctness claims — correctness is carried
//!   by the analytic oracles above.

use celnet_core::assert_close;
use celnet_qmc::{BrownianBridge, MAX_DIM, SobolSequence, inv_norm_cdf, rqmc_estimate};

/// Independent reference `Φ(x) = ½·erfc(−x/√2)` (libm dev-dep; code-disjoint
/// from the Acklam-seed + Halley-polish implementation under test).
fn phi_ref(x: f64) -> f64 {
    0.5 * libm::erfc(-x * core::f64::consts::FRAC_1_SQRT_2)
}

// ---------------------------------------------------------------------------
// 1. Low-discrepancy reference points (exact dyadic rationals, to_bits).
// ---------------------------------------------------------------------------

/// Dimension 0 is the base-2 radical-inverse (van der Corput) sequence in
/// gray-code order. With single-bit direction numbers `v_k = 2^{31-k}` the
/// gray-code recurrence `x_{i+1} = x_i ⊕ v_{ctz(i+1)}` gives, by hand:
///
/// ```text
/// x1 = .1      = 0.5      x5 = .111 = 0.875
/// x2 = .11     = 0.75     x6 = .101 = 0.625
/// x3 = .01     = 0.25     x7 = .001 = 0.125
/// x4 = .011    = 0.375    x8 = .0011 = 0.1875
/// ```
///
/// All are exactly representable, so the pin is `to_bits` equality.
#[test]
fn low_discrepancy_reference_points() {
    let seq = SobolSequence::new(MAX_DIM);
    assert_eq!(seq.dim(), MAX_DIM);

    // Point 0 is the origin in every coordinate (gray code g(0) = 0).
    for (j, c) in seq.point(0).iter().enumerate() {
        assert_eq!(c.to_bits(), 0.0f64.to_bits(), "point 0, dim {j}");
    }

    // Hand-derived gray-code radical-inverse values, dimension 0.
    let expected: [f64; 8] = [0.5, 0.75, 0.25, 0.375, 0.875, 0.625, 0.125, 0.1875];
    for (idx, &e) in expected.iter().enumerate() {
        let i = idx as u64 + 1;
        let p = seq.point(i);
        assert_eq!(
            p[0].to_bits(),
            e.to_bits(),
            "dim 0, point {i}: got {} want {e}",
            p[0]
        );
    }

    // The first 16 dim-0 points are exactly {k/16 : k = 0..16} as a SET (the
    // gray code permutes the van der Corput points within each 2^m block).
    let mut seen = [false; 16];
    for i in 0..16u64 {
        let x = seq.point(i)[0];
        let k = (x * 16.0) as usize;
        assert_eq!((k as f64 / 16.0).to_bits(), x.to_bits(), "non-dyadic {x}");
        assert!(!seen[k], "duplicate dim-0 point {x}");
        seen[k] = true;
    }
    assert!(seen.iter().all(|&b| b));

    // Every Joe-Kuo dimension has leading direction integer m_1 = 1, i.e.
    // v_1 = 2^31 — so the FIRST nonzero point is 0.5 in every dimension.
    let p1 = seq.point(1);
    for &j in &[0usize, 1, 2, 7, 31, 63, MAX_DIM - 1] {
        assert_eq!(p1[j].to_bits(), 0.5f64.to_bits(), "point 1, dim {j}");
    }
}

// ---------------------------------------------------------------------------
// 2. Direction numbers vs an independent m_k-domain recurrence.
// ---------------------------------------------------------------------------

/// Rows typed directly from the published Joe-Kuo `new-joe-kuo-6.21201` data
/// file (`d s a m_1..m_s`) — the same published source the embedded table was
/// generated from, but entered independently here.
const REFERENCE_ROWS: &[(usize, u32, u32, &[u32])] = &[
    (2, 1, 0, &[1]),
    (3, 2, 1, &[1, 3]),
    (4, 3, 1, &[1, 3, 1]),
    (5, 3, 2, &[1, 1, 1]),
    (6, 4, 1, &[1, 1, 3, 3]),
    (7, 4, 4, &[1, 3, 5, 13]),
    (8, 5, 2, &[1, 1, 5, 5, 17]),
    (9, 5, 4, &[1, 1, 5, 5, 5]),
    (10, 5, 7, &[1, 1, 7, 11, 19]),
    (20, 7, 1, &[1, 3, 7, 11, 23, 15, 103]),
    (50, 8, 97, &[1, 1, 1, 3, 23, 43, 57, 177]),
    (100, 9, 244, &[1, 1, 5, 5, 11, 5, 45, 117, 217]),
    (200, 11, 227, &[1, 3, 1, 3, 29, 25, 21, 155, 11, 191, 197]),
    (300, 11, 789, &[1, 1, 7, 15, 1, 33, 31, 233, 161, 507, 387]),
];

/// Re-derive all 32 direction numbers of one dimension in the `m_k` domain:
///
/// ```text
/// m_k = 2·a_1·m_{k−1} ⊕ 2²·a_2·m_{k−2} ⊕ … ⊕ 2^{s−1}·a_{s−1}·m_{k−s+1}
///       ⊕ 2^s·m_{k−s} ⊕ m_{k−s},          v_k = m_k · 2^{32−k},
/// ```
///
/// with `a_i` the i-th coefficient bit of `a` (MSB = a_1). This is the
/// recurrence as published; the implementation instead iterates directly on
/// the left-justified `V_k` (right-shift form), so the two derivations share
/// no arithmetic shape.
fn reference_direction_numbers(s: u32, a: u32, m_init: &[u32]) -> [u32; 32] {
    let s = s as usize;
    assert_eq!(m_init.len(), s);
    let mut m = [0u32; 33]; // 1-based m_1..m_32
    m[1..=s].copy_from_slice(m_init);
    for k in (s + 1)..=32 {
        let mut val = (m[k - s] << s) ^ m[k - s];
        for i in 1..s {
            if (a >> (s - 1 - i)) & 1 == 1 {
                val ^= m[k - i] << i;
            }
        }
        m[k] = val;
    }
    let mut v = [0u32; 32];
    for k in 1..=32 {
        v[k - 1] = m[k] << (32 - k as u32);
    }
    v
}

#[test]
fn direction_numbers_match_recurrence() {
    let seq = SobolSequence::new(MAX_DIM);

    // Dimension 1 (index 0): the identity direction numbers v_k = 2^{31-k}
    // (the van der Corput special case, stated in the published convention).
    let v0 = seq.direction_numbers(0);
    for (k, &v) in v0.iter().enumerate() {
        assert_eq!(v, 1u32 << (31 - k), "dim 1, k={k}");
    }

    // Sampled dimensions 2..=300: bit-for-bit against the m_k-domain oracle.
    for &(d, s, a, m_init) in REFERENCE_ROWS {
        let expected = reference_direction_numbers(s, a, m_init);
        let got = seq.direction_numbers(d - 1);
        assert_eq!(got, &expected, "direction numbers mismatch, dimension {d}");
    }
}

// ---------------------------------------------------------------------------
// 3. Streaming gray-code recurrence == direct construction (scrambled).
// ---------------------------------------------------------------------------

/// `SobolStream::next_point` is documented as the index-for-index scrambled
/// sequence: emission `i` equals `scrambled_point(i, seed)`. Pinned `to_bits`
/// over the first 1,024 indices — this kills the XOR/ctz update internals
/// (the stream replays the recurrence; the direct path re-XORs from scratch).
#[test]
fn gray_code_stream_equals_direct_points() {
    let seq = SobolSequence::new(16);
    let seed = 0xDEC0_DE5Eu64;
    let mut stream = seq.stream(seed);
    let mut buf = vec![0.0f64; 16];
    for i in 0..1024u64 {
        stream.next_point(&mut buf);
        let direct = seq.scrambled_point(i, seed);
        for (j, (&a, &b)) in buf.iter().zip(direct.iter()).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "i={i}, dim {j}");
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Scramble: determinism, seed sensitivity, open-unit-cube range.
// ---------------------------------------------------------------------------

#[test]
fn scrambled_point_is_deterministic_and_in_unit_cube() {
    let seq = SobolSequence::new(8);
    let mut any_diff = false;
    for i in 0..64u64 {
        let a = seq.scrambled_point(i, 7);
        let b = seq.scrambled_point(i, 7);
        let c = seq.scrambled_point(i, 8);
        for j in 0..8 {
            // Same (i, seed) twice: bit-identical.
            assert_eq!(a[j].to_bits(), b[j].to_bits(), "i={i}, dim {j}");
            // Strictly inside (0,1): the half-LSB bias forbids 0.0 exactly.
            assert!(a[j] > 0.0 && a[j] < 1.0, "out of (0,1): {} (i={i})", a[j]);
            if a[j].to_bits() != c[j].to_bits() {
                any_diff = true;
            }
        }
    }
    // A different seed must change the realisation somewhere.
    assert!(any_diff, "seed 7 and seed 8 scrambles are identical");
}

/// Frozen-bits rows (house `fx_byte_identity` pattern): exact `u64` pins of
/// `scrambled_point` for a frozen `(dim=8, seed=0x5EED_F00D)` configuration,
/// captured from the unmutated implementation. The integer scramble core is
/// bit-identical across platforms, so these are stable; they pin the hash /
/// nested-permutation / open-unit-map arithmetic that property tests cannot
/// distinguish (any valid scramble passes the properties; only THIS scramble
/// passes the pins).
#[test]
fn scrambled_point_frozen_bits() {
    const SEED: u64 = 0x5EED_F00D;
    const ROWS: [(u64, [u64; 8]); 4] = [
        (
            0,
            [
                0x3fd9b93ade600000,
                0x3fba7d1693800000,
                0x3fe63d99bdd00000,
                0x3fb509e2f1800000,
                0x3fe05c41de700000,
                0x3fe446478dd00000,
                0x3fd756ceec600000,
                0x3fe36f1e31700000,
            ],
        ),
        (
            1,
            [
                0x3fe199c158b00000,
                0x3fe5e72cba500000,
                0x3fd8f7bc5e200000,
                0x3fe7319a9ed00000,
                0x3fc9646262c00000,
                0x3fde0c146aa00000,
                0x3fe5449e02f00000,
                0x3fd162d532a00000,
            ],
        ),
        (
            5,
            [
                0x3fef1a9d05b00000,
                0x3fea9cc318b00000,
                0x3fe3088196b00000,
                0x3fdb840ad7a00000,
                0x3fdc221587e00000,
                0x3fd44dec58a00000,
                0x3fed73edb5300000,
                0x3fef684db5d00000,
            ],
        ),
        (
            255,
            [
                0x3fd9d3dd7c200000,
                0x3fe98c64f7f00000,
                0x3fc6c94404400000,
                0x3fe4e704ffb00000,
                0x3fc4ad230b400000,
                0x3fc66628c9400000,
                0x3fed3f9dbeb00000,
                0x3fe74bd193b00000,
            ],
        ),
    ];
    let seq = SobolSequence::new(8);
    for &(i, expected) in &ROWS {
        let p = seq.scrambled_point(i, SEED);
        for (j, (&c, &e)) in p.iter().zip(expected.iter()).enumerate() {
            assert_eq!(
                c.to_bits(),
                e,
                "frozen scramble drifted at i={i}, dim {j}: got {c} (0x{:016x})",
                c.to_bits()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Inverse normal CDF vs published quantiles + independent erfc round-trip.
// ---------------------------------------------------------------------------

/// Published high-precision standard-normal quantiles (standard statistical
/// tables / high-precision computation), pinned to ≤ 1e-13 relative.
#[test]
fn inverse_normal_matches_reference() {
    // Φ⁻¹(0.5) = 0 exactly (the central rational seed is odd in q = p − ½ and
    // the Halley correction vanishes at e = Φ(0) − ½ = 0).
    assert_eq!(inv_norm_cdf(0.5).to_bits(), 0.0f64.to_bits());

    let published = [
        (0.975, 1.959_963_984_540_054),
        (0.95, 1.644_853_626_951_472_2),
        (0.99, 2.326_347_874_040_840_8),
        (0.995, 2.575_829_303_548_900_4),
        (0.999, 3.090_232_306_167_813),
    ];
    for &(p, x) in &published {
        assert_close!(inv_norm_cdf(p), x, 1e-13, 1e-15);
        // Symmetric negatives.
        assert_close!(inv_norm_cdf(1.0 - p), -x, 1e-13, 1e-15);
    }

    // Symmetry property inv(p) == −inv(1−p) on a grid spanning both the
    // central and tail branches. The grid stays at p ≥ 1e-3 because for deeper
    // tails the symmetry comparison is ill-posed in double precision: fl(1−p)
    // is not exactly 1−p, and Φ(x) near 1 carries an intrinsic ~1 ULP(1)
    // absolute error that maps to ~ULP(1)/φ(x) in x — a representation limit,
    // not an implementation defect. (Lower-tail depth is covered by the
    // independent round-trip below, where p is exactly representable.)
    for &p in &[1e-3, 0.01, 0.024, 0.05, 0.2, 0.4, 0.49, 0.499] {
        assert_close!(inv_norm_cdf(p), -inv_norm_cdf(1.0 - p), 1e-12, 1e-14);
    }

    // Round-trip against the INDEPENDENT reference Φ(x) = ½·erfc(−x/√2),
    // including the p = 1e-12 deep tail (relative — tail values are tiny).
    for &p in &[
        1e-12, 1e-9, 1e-6, 1e-3, 0.01, 0.024_25, 0.1, 0.25, 0.5, 0.75, 0.9, 0.975_75, 0.99, 0.999,
    ] {
        let x = inv_norm_cdf(p);
        assert_close!(phi_ref(x), p, 1e-13, 0.0);
    }

    // Frozen bits exactly AT the central/tail branch break-points p = 0.02425
    // and p = 1 − 0.02425: both branch seeds polish to the same value to ≤1ULP,
    // so only an exact pin distinguishes "took the documented branch" from
    // "took the other branch" (captured from the unmutated implementation;
    // libm-routed transcendentals are bit-identical across platforms).
    assert_eq!(inv_norm_cdf(0.024_25).to_bits(), 0xbfff913f9b7aa943);
    assert_eq!(inv_norm_cdf(1.0 - 0.024_25).to_bits(), 0x3fff913f9b7aa943);
}

/// Frozen-bits ladder pinning the **Halley polish** (`x ← x − u/(1 + ½·x·u)`)
/// at extreme-tail quantiles. In the standard domain the polish differs from
/// any perturbation of itself (Newton step, sign-flipped denominator, …) by
/// `O(x·u²)` where `u ~ 1e-9·x` is the rational-seed error — *sub-ULP*, so no
/// tolerance-based oracle can see those mutants. In the extreme tails the
/// seed degrades (`u` grows), the deviation crosses 1 ULP (≈10¹² ULPs at the
/// subnormal floor), and the realised bits become load-bearing. Captured from
/// the unmutated implementation; `libm`-routed transcendentals make them
/// platform-stable (house fx_byte_identity pattern). Correctness (vs the
/// independent erfc round-trip) is asserted in
/// `inverse_normal_matches_reference`; these pins add bit-exactness of the
/// documented Halley form, including both tail branches of the seed.
#[test]
fn halley_polish_is_load_bearing() {
    const PINS: [(f64, u64); 6] = [
        (5e-324, 0xc043_3bd3_f311_79e2), // subnormal floor, x ≈ −38.467
        (1e-250, 0xc040_e658_d6f7_0771), // x ≈ −33.799586…
        (1e-50, 0xc02d_ddde_6ad8_1776),  // x ≈ −14.933337…
        (1e-12, 0xc01c_234f_ba57_a32a),  // x ≈ −7.0344838…
        (0.999_999_999_068_677_4, 0x4018_0993_fb2b_69f3), // 1 − 2⁻³⁰, x ≈ +6.0093536
        (0.999_999_999_999_999_9, 0x4020_6b48_5255_82da), // 1 − 2⁻⁵³, x ≈ +8.2095361
    ];
    let mut prev = f64::NEG_INFINITY;
    for (p, bits) in PINS {
        let x = inv_norm_cdf(p);
        assert_eq!(
            x.to_bits(),
            bits,
            "Halley pin drifted at p={p:e}: got {x} (0x{:016x})",
            x.to_bits()
        );
        // Sanity: strictly increasing across the ladder.
        assert!(x > prev, "not monotone at p={p:e}");
        prev = x;
    }
}

// ---------------------------------------------------------------------------
// 6. Brownian-bridge factorization reproduces the exact covariance.
// ---------------------------------------------------------------------------

/// The bridge weight matrix `L` (path = L·z) must satisfy the mathematical
/// characterization `(L·Lᵀ)[i][j] = min(t_i, t_j)` — ANY wrong interpolation
/// weight or standard deviation breaks this. Additionally the principal-
/// bisection loading is pinned: column 0 (the terminal normal z₀) must carry
/// `L[i][0] = t_i/√T` (the conditional-mean interpolation of W(T)), and the
/// first bisection (z₁) drives the midpoint with std `√(t_m(T−t_m)/T)`.
#[test]
fn bridge_factorization_reproduces_brownian_covariance() {
    for &(m, t_total) in &[(1usize, 1.0), (2, 0.7), (3, 2.0), (8, 2.0), (64, 1.5)] {
        let bb = BrownianBridge::new(m, t_total);
        assert_eq!(bb.steps(), m);
        let l = bb.weight_matrix();

        // Grid times: t_i = (i+1)·T/m, re-derived with the same arithmetic.
        let dt = t_total / m as f64;
        for i in 0..m {
            assert_eq!(bb.time(i).to_bits(), ((i + 1) as f64 * dt).to_bits());
        }

        // Exact discrete covariance.
        for i in 0..m {
            for j in 0..m {
                let c: f64 = l[i].iter().zip(l[j].iter()).map(|(x, y)| x * y).sum();
                let expected = bb.time(i).min(bb.time(j));
                assert_close!(c, expected, 1e-12, 1e-12);
            }
        }

        // Principal-bisection variance loading: z₀ is the terminal normal and
        // its coefficient interpolates linearly in time, L[i][0] = t_i/√T.
        let sqrt_t = t_total.sqrt();
        for (i, row) in l.iter().enumerate() {
            assert_close!(row[0], bb.time(i) / sqrt_t, 1e-12, 1e-14);
        }
    }

    // First bisection: for m = 8 the second normal z₁ fills the midpoint
    // (index 3, t = T/2) with std √(t·(T−t)/T) = √T/2 — pins the bisection
    // pivot choice (any other pivot moves this coefficient).
    let t_total = 2.0;
    let bb = BrownianBridge::new(8, t_total);
    let l = bb.weight_matrix();
    let t_mid = bb.time(3);
    assert_close!(t_mid, t_total / 2.0, 1e-15, 1e-15);
    let expected_std = (t_mid * (t_total - t_mid) / t_total).sqrt();
    assert_close!(l[3][1], expected_std, 1e-13, 1e-15);

    // `build` on the standard basis vectors reproduces the columns of L
    // bit-for-bit (links the hot-path `build` to `weight_matrix`).
    let m = 8;
    let mut z = vec![0.0f64; m];
    let mut path = vec![0.0f64; m];
    for k in 0..m {
        z.fill(0.0);
        z[k] = 1.0;
        bb.build(&z, &mut path);
        for (i, row) in l.iter().enumerate() {
            assert_eq!(path[i].to_bits(), row[k].to_bits(), "k={k}, i={i}");
        }
    }
}

/// Hand-derived weight matrix for `m = 3, T = 3` (grid times exactly 1, 2, 3)
/// — pins the *documented deterministic construction order*, not just the
/// covariance. The first bisection of the interior `[lo, right-1]` must pick
/// the FLOOR midpoint `lo + (right - lo - 1)/2`; for the even interior span of
/// m = 3 (indices {0, 1}) that is index 0, so `z₁` carries the full bridge
/// std at index 0. An alternative (ceil) pivot is *distributionally* identical
/// — same covariance, so the covariance oracle above cannot see it — but it
/// permutes which normal drives which grid point, breaking the bit-exact
/// CPU/GPU path-reproducibility contract. Derivation (Brownian bridge,
/// Σ = min(t_i,t_j)):
///
/// ```text
/// step 0: W(3) = √3·z₀                            → row 2 = [√3, 0, 0]
/// step 1: W(1) | W(3): frac = 1/3, var = 1·2/3    → row 0 = [√3/3, √(2/3), 0]
/// step 2: W(2) | W(1),W(3): frac = 1/2, var = 1/2 → row 1 = [2√3/3, √(2/3)/2, √(1/2)]
/// ```
#[test]
fn bridge_plan_uses_documented_floor_midpoint() {
    let bb = BrownianBridge::new(3, 3.0);
    let l = bb.weight_matrix();
    let s3 = 3.0f64.sqrt();
    let s23 = (2.0f64 / 3.0).sqrt();
    let expected = [
        [s3 / 3.0, s23, 0.0],
        [2.0 * s3 / 3.0, s23 / 2.0, 0.5f64.sqrt()],
        [s3, 0.0, 0.0],
    ];
    for (i, row) in expected.iter().enumerate() {
        for (k, &e) in row.iter().enumerate() {
            assert_close!(l[i][k], e, 1e-15, 1e-15);
        }
    }
}

// ---------------------------------------------------------------------------
// 7. RQMC estimator: closed-form monomial integrals + std-error re-derivation.
// ---------------------------------------------------------------------------

/// `W(t_k)/√t_k` is standard normal for every grid index, so
/// `Φ(W/√t) ~ U(0,1)` and the estimator must reproduce `∫₀¹ x dx = ½` and
/// `∫₀¹ x² dx = ⅓` through the entire pipeline (Sobol → scramble → Φ⁻¹ →
/// bridge). Checked on the terminal coordinate AND the first grid coordinate
/// (a nontrivial mix of all bridge normals) for m = 1..=4 steps.
#[test]
fn rqmc_estimate_integrates_monomials() {
    /// Estimate `E[u^power]` for `u = Φ(W(t_idx)/√t_idx)` (a U(0,1) variate).
    fn estimate_monomial(bb: &BrownianBridge, idx: usize, power: u32) -> celnet_qmc::RqmcResult {
        let t = bb.time(idx);
        rqmc_estimate(bb, 4096, 8, 0x0123_4567_89AB_CDEF, |p| {
            phi_ref(p[idx] / t.sqrt()).powi(power as i32)
        })
    }
    for m in 1usize..=4 {
        let bb = BrownianBridge::new(m, 1.0);
        // Terminal coordinate AND first grid coordinate, first two monomials.
        for idx in [m - 1, 0] {
            for (power, target) in [(1u32, 0.5), (2, 1.0 / 3.0)] {
                let res = estimate_monomial(&bb, idx, power);
                assert!(
                    (res.estimate - target).abs() <= 1e-4,
                    "m={m}, idx={idx}, E[u^{power}]: estimate {} vs {target} (se {})",
                    res.estimate,
                    res.std_error
                );
                assert!(res.std_error.is_finite() && res.std_error > 0.0);
            }
        }
    }
}

/// The reported std-error must equal the independent plain-loop sample
/// variance of the per-replication means. Replicate `r` of an `R`-replication
/// run is reproduced exactly by a 1-replication run whose base seed is shifted
/// by `r · 0x9e3779b97f4a7c15` (the documented per-replication seed
/// derivation), which simultaneously pins that derivation: any change to the
/// seed arithmetic desynchronizes the two runs.
#[test]
fn std_error_matches_replicate_means() {
    let bb = BrownianBridge::new(4, 1.0);
    let payoff = |p: &[f64]| p.iter().sum::<f64>() + p[0] * p[0];
    let base = 0xABCD_EF01u64;
    let reps = 8usize;
    let budget = 512usize;

    let full = rqmc_estimate(&bb, budget, reps, base, payoff);
    assert_eq!(full.replications, reps);
    assert_eq!(full.budget, budget);

    let mut rep_means = Vec::with_capacity(reps);
    for r in 0..reps {
        let shifted = base.wrapping_add((r as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let one = rqmc_estimate(&bb, budget, 1, shifted, payoff);
        // A single replication has no between-scramble variance: NaN std-error.
        assert!(one.std_error.is_nan());
        rep_means.push(one.estimate);
    }

    // Mean of the replicate means — identical summation order, so exact.
    let n = rep_means.len() as f64;
    let mean = rep_means.iter().sum::<f64>() / n;
    assert_eq!(full.estimate.to_bits(), mean.to_bits());

    // Independent plain-loop unbiased variance → std error.
    let mut ss = 0.0;
    for &x in &rep_means {
        ss += (x - mean) * (x - mean);
    }
    let se = (ss / (n - 1.0) / n).sqrt();
    assert_close!(full.std_error, se, 1e-12, 1e-15);
}

/// Frozen-bits pin of the full RQMC pipeline for one frozen configuration
/// (house `fx_byte_identity` pattern; captured from the unmutated
/// implementation). Kills mutants that any distributional property tolerates —
/// e.g. the SplitMix64 seed-avalanche internals, where ANY mixing constant
/// still yields a statistically valid scramble but a different realisation.
#[test]
fn rqmc_frozen_bits() {
    let bb = BrownianBridge::new(8, 1.5);
    let res = rqmc_estimate(&bb, 128, 4, 42, |p| {
        p.iter().map(|w| w.abs()).sum::<f64>() + p[7] * p[7]
    });
    assert_eq!(
        res.estimate.to_bits(),
        0x401c_883c_ea71_9dd6,
        "estimate drifted: {} (0x{:016x})",
        res.estimate,
        res.estimate.to_bits()
    );
    assert_eq!(
        res.std_error.to_bits(),
        0x3f9f_cc22_f55e_cd6f,
        "std_error drifted: {} (0x{:016x})",
        res.std_error,
        res.std_error.to_bits()
    );
}

// ---------------------------------------------------------------------------
// Contract panics (gates the validation asserts against weakening mutants).
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "dimension must be >= 1")]
fn sobol_dim_zero_panics() {
    let _ = SobolSequence::new(0);
}

#[test]
#[should_panic(expected = "exceeds the embedded")]
fn sobol_dim_beyond_table_panics() {
    let _ = SobolSequence::new(MAX_DIM + 1);
}

#[test]
fn sobol_dim_bounds_are_inclusive() {
    assert_eq!(SobolSequence::new(1).dim(), 1);
    assert_eq!(SobolSequence::new(MAX_DIM).dim(), MAX_DIM);
}

#[test]
#[should_panic(expected = "at least one step")]
fn bridge_zero_steps_panics() {
    let _ = BrownianBridge::new(0, 1.0);
}

#[test]
#[should_panic(expected = "total time must be positive")]
fn bridge_zero_time_panics() {
    let _ = BrownianBridge::new(4, 0.0);
}

#[test]
#[should_panic(expected = "total time must be positive")]
fn bridge_negative_time_panics() {
    let _ = BrownianBridge::new(4, -1.0);
}

#[test]
#[should_panic(expected = "need >= 1 replication")]
fn rqmc_zero_replications_panics() {
    let bb = BrownianBridge::new(2, 1.0);
    let _ = rqmc_estimate(&bb, 16, 0, 1, |p| p[0]);
}

#[test]
#[should_panic(expected = "need >= 1 point")]
fn rqmc_zero_budget_panics() {
    let bb = BrownianBridge::new(2, 1.0);
    let _ = rqmc_estimate(&bb, 0, 4, 1, |p| p[0]);
}
