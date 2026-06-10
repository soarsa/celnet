//! Parity row — **scrambled Sobol + Brownian-bridge quasi-Monte-Carlo**
//! (`celnet-qmc`, Wave 4c).
//!
//! Each claim is backed by an *independent* oracle:
//!
//! * (i) **Sobol KAT** — the first 16 points across 8 dimensions match a
//!   reference generated from the canonical Joe-Kuo `sobol.cc` recurrence (an
//!   external/structural oracle), and dimension 1 is *exactly* the van der
//!   Corput / bit-reversal radical-inverse sequence in base 2.
//! * (ii) **(t,m,s)-net equidistribution** — for `2^m` Sobol points in `s`
//!   dimensions, every elementary dyadic box of volume `2^{-m}` contains
//!   *exactly* its fair share of points (a structural property of a `(0,m,s)`-net,
//!   independent of any pricing).
//! * (iii) **MEASURED variance reduction ≥ 3×** — on a known-value target (a
//!   discretely-monitored geometric-average Asian with the exact Kemna-Vorst
//!   closed form from `celnet-exotics`, and a European vanilla vs the
//!   `celnet-vanilla` Black-Scholes value), the randomized-QMC + Brownian-bridge
//!   estimator's RMSE (over independent scrambles at a fixed budget) is at least
//!   3× smaller than a plain pseudo-random Monte-Carlo estimator of the same
//!   payoff at the same budget. Both RMSEs and the measured ratio are reported;
//!   the ratio is *measured*, not asserted.
//! * (iv) **bridge correctness + unbiasedness** — the Brownian-bridge weight
//!   matrix reproduces the exact discrete covariance `min(t_i, t_j)` (an exact
//!   algebraic identity), and the RQMC estimator is unbiased: its mean over many
//!   scrambles converges to the exact known value within the reported standard
//!   error.

use celnet_core::math::{exp, ln, sqrt};
use celnet_exotics::asian::{AnalyticAsian, geometric_average_price};
use celnet_qmc::{BrownianBridge, MAX_DIM, SobolSequence, inv_norm_cdf, rqmc_estimate};
use celnet_types::{OptionType, VanillaInputs};

// ---------------------------------------------------------------------------
// (i) Sobol known-answer test (KAT)
// ---------------------------------------------------------------------------

/// First 16 unscrambled Sobol points (8 dims) as raw `u32`, generated from the
/// canonical Joe-Kuo reference `sobol.cc` recurrence (the external oracle). See
/// the test module docs for provenance.
const KAT_U32: [[u32; 8]; 16] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [
        2147483648, 2147483648, 2147483648, 2147483648, 2147483648, 2147483648, 2147483648,
        2147483648,
    ],
    [
        3221225472, 1073741824, 1073741824, 1073741824, 3221225472, 3221225472, 1073741824,
        3221225472,
    ],
    [
        1073741824, 3221225472, 3221225472, 3221225472, 1073741824, 1073741824, 3221225472,
        1073741824,
    ],
    [
        1610612736, 1610612736, 2684354560, 3758096384, 1610612736, 536870912, 1610612736,
        3758096384,
    ],
    [
        3758096384, 3758096384, 536870912, 1610612736, 3758096384, 2684354560, 3758096384,
        1610612736,
    ],
    [
        2684354560, 536870912, 3758096384, 2684354560, 2684354560, 3758096384, 536870912, 536870912,
    ],
    [
        536870912, 2684354560, 1610612736, 536870912, 536870912, 1610612736, 2684354560, 2684354560,
    ],
    [
        805306368, 1342177280, 4026531840, 1879048192, 2415919104, 1342177280, 1879048192,
        4026531840,
    ],
    [
        2952790016, 3489660928, 1879048192, 4026531840, 268435456, 3489660928, 4026531840,
        1879048192,
    ],
    [
        4026531840, 268435456, 2952790016, 805306368, 1342177280, 2415919104, 805306368, 805306368,
    ],
    [
        1879048192, 2415919104, 805306368, 2952790016, 3489660928, 268435456, 2952790016,
        2952790016,
    ],
    [
        1342177280, 805306368, 1342177280, 2415919104, 4026531840, 1879048192, 268435456, 268435456,
    ],
    [
        3489660928, 2952790016, 3489660928, 268435456, 1879048192, 4026531840, 2415919104,
        2415919104,
    ],
    [
        2415919104, 1879048192, 268435456, 3489660928, 805306368, 2952790016, 1342177280,
        3489660928,
    ],
    [
        268435456, 4026531840, 2415919104, 1342177280, 2952790016, 805306368, 3489660928,
        1342177280,
    ],
];

#[test]
fn sobol_known_answer_first_points() {
    let seq = SobolSequence::new(8);
    for (i, expected) in KAT_U32.iter().enumerate() {
        let got = seq.point_u32(i as u64);
        assert_eq!(&got[..], &expected[..], "Sobol KAT mismatch at point {i}");
    }
}

/// Dimension 1 of the gray-code Sobol sequence is *exactly* the van der Corput /
/// bit-reversal sequence in base 2 evaluated at the **gray code** `g(i)=i⊕(i>>1)`
/// — the Antonov-Saleev gray-code enumeration of the radical-inverse sequence.
/// (The first `2^k` points are exactly `{0, 1/2^k, ..., (2^k-1)/2^k}` permuted,
/// the defining van der Corput property.) Exact algebraic identity, independent
/// structural oracle for the first column.
#[test]
fn sobol_dim1_is_van_der_corput_base2() {
    let seq = SobolSequence::new(3);
    for i in 0u64..512 {
        let g = (i ^ (i >> 1)) as u32;
        // Van der Corput in base 2: reverse the bits of the gray code.
        let vdc = g.reverse_bits();
        assert_eq!(
            seq.point_u32(i)[0],
            vdc,
            "dim-1 is not the base-2 radical inverse of the gray code at i={i}"
        );
    }
    // Defining property: the first 2^k dim-1 points are exactly the dyadic grid
    // {0, 1/2^k, ..., (2^k-1)/2^k} (a perfect (0,k,1)-net), in some order.
    for k in 1u32..=8 {
        let n = 1u64 << k;
        let mut seen = vec![false; n as usize];
        for i in 0..n {
            let cell = (seq.point_u32(i)[0] >> (32 - k)) as usize;
            assert!(!seen[cell], "dim-1 dyadic cell {cell} hit twice (k={k})");
            seen[cell] = true;
        }
        assert!(seen.iter().all(|&b| b), "dim-1 not a (0,{k},1)-net");
    }
}

/// The documented maximum dimension is honoured: constructing at the bound works,
/// and exceeding it panics (a documented, gated bound — not a silent cap).
#[test]
fn sobol_documented_max_dimension() {
    let seq = SobolSequence::new(MAX_DIM);
    assert_eq!(seq.dim(), MAX_DIM);
    // A point in the highest dimension is a proper fraction in (0,1).
    let p = seq.point(123);
    assert!(p[MAX_DIM - 1] >= 0.0 && p[MAX_DIM - 1] < 1.0);
}

#[test]
#[should_panic(expected = "exceeds the embedded Joe-Kuo table maximum")]
fn sobol_beyond_max_dimension_panics() {
    let _ = SobolSequence::new(MAX_DIM + 1);
}

// ---------------------------------------------------------------------------
// (ii) (t,m,s)-net equidistribution / balance
// ---------------------------------------------------------------------------

/// The first `2^m` points of the Sobol sequence form a `(0,m,s)`-net in base 2
/// for `s ≤ 2`: every elementary dyadic box whose side-lengths are `2^{-q_j}`
/// with `Σ q_j = m` contains *exactly* `2^{m - Σq_j} = 1` point. This is the
/// defining equidistribution property — purely structural, independent of any
/// pricing.
///
/// The first two Sobol dimensions `{0, 1}` always form a perfect `(0,m,2)`-net
/// (a consequence of the unit-triangular direction-number construction), and each
/// individual axis is a `(0,m,1)`-net. These are the rigorously-exact net
/// properties we assert here. (For general higher-dimensional projections the
/// Joe-Kuo direction numbers give *good* but not perfect `(0,m,2)` projections —
/// the net `t`-value is generally `> 0` — so we deliberately do NOT overclaim a
/// `(0,m,s)`-net for arbitrary `s ≥ 2` projections.)
#[test]
fn sobol_is_tms_net_balanced() {
    // Each single axis is the van der Corput / dyadic equidistribution => a
    // (0,m,1)-net for any m. Check the first several axes.
    for j in 0..6 {
        check_net_balanced(&[j], 9);
    }
    // The first two dimensions form a perfect (0,m,2)-net: 2^6 = 64 points, every
    // elementary dyadic box of total order 6 holds exactly 1 point.
    check_net_balanced(&[0, 1], 6);
    // ...at a finer resolution too (every box of total order 8 holds exactly 1 of
    // the 256 points).
    check_net_balanced(&[0, 1], 8);
}

/// On the coordinate projection given by `dims` (a `(0,m,|dims|)`-net for the
/// cases we assert), verify that for *every* composition `(q_0,..)` of `m` into
/// `|dims|` parts, each elementary dyadic box of side `2^{-q_k}` along axis
/// `dims[k]` contains exactly one of the `2^m` Sobol points.
fn check_net_balanced(dims: &[usize], m: u32) {
    let s = dims.len();
    let max_dim = dims.iter().copied().max().unwrap() + 1;
    let seq = SobolSequence::new(max_dim);
    let n = 1u64 << m;
    let pts: Vec<Vec<u32>> = (0..n).map(|i| seq.point_u32(i)).collect();

    for comp in compositions(m, s) {
        // Box index of a point = concatenation of the top q_k bits of each
        // projected coordinate.
        let mut counts = vec![0u32; n as usize];
        for p in &pts {
            let mut idx: u64 = 0;
            for (k, &q) in comp.iter().enumerate() {
                let bits = if q == 0 { 0 } else { p[dims[k]] >> (32 - q) };
                idx = (idx << q) | bits as u64;
            }
            counts[idx as usize] += 1;
        }
        assert!(
            counts.iter().all(|&c| c == 1),
            "(0,{m},{s})-net imbalance for dims={dims:?} comp={comp:?}"
        );
    }
}

/// All ordered compositions of `m` into exactly `s` non-negative parts.
fn compositions(m: u32, s: usize) -> Vec<Vec<u32>> {
    if s == 1 {
        return vec![vec![m]];
    }
    let mut out = Vec::new();
    for first in 0..=m {
        for rest in compositions(m - first, s - 1) {
            let mut v = Vec::with_capacity(s);
            v.push(first);
            v.extend(rest);
            out.push(v);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// (iv) Brownian-bridge covariance identity (exact algebra)
// ---------------------------------------------------------------------------

/// `A·Aᵀ = Σ` with `Σ_{ij} = min(t_i, t_j)` for the bridge weight matrix `A`.
/// Exact algebraic identity — the defining property of a discrete Brownian path.
#[test]
fn brownian_bridge_reproduces_discrete_covariance() {
    for &m in &[1usize, 2, 4, 8, 16, 17, 32] {
        let t_total = 1.75;
        let bb = BrownianBridge::new(m, t_total);
        let a = bb.weight_matrix();
        for i in 0..m {
            for j in 0..m {
                let c: f64 = a[i].iter().zip(a[j].iter()).map(|(x, y)| x * y).sum();
                let expected = bb.time(i).min(bb.time(j));
                let err = (c - expected).abs();
                assert!(
                    err < 1e-12,
                    "covariance mismatch m={m} i={i} j={j}: got {c} want {expected}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// (iii)+(iv) MEASURED variance reduction & unbiasedness on known-value targets
// ---------------------------------------------------------------------------

/// Garman-Kohlhagen market parameters for the variance-reduction targets.
struct Market {
    s0: f64,
    r_dom: f64,
    r_for: f64,
    sigma: f64,
    t: f64,
}

impl Market {
    fn base() -> Self {
        Self {
            s0: 100.0,
            r_dom: 0.05,
            r_for: 0.02,
            sigma: 0.20,
            t: 1.0,
        }
    }
    fn drift(&self) -> f64 {
        self.r_dom - self.r_for
    }
    fn df(&self) -> f64 {
        exp(-self.r_dom * self.t)
    }
}

/// A deterministic, reproducible pseudo-random standard-normal generator
/// (SplitMix64 → uniforms → inverse-normal CDF). Used as the *independent*
/// plain-MC baseline against which QMC variance reduction is measured.
struct PseudoNormals {
    state: u64,
}
impl PseudoNormals {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_u64(&mut self) -> u64 {
        // SplitMix64.
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn next_uniform(&mut self) -> f64 {
        // 53-bit mantissa uniform in the open interval (0,1).
        let u = (self.next_u64() >> 11) as f64;
        (u + 0.5) * (1.0 / 9_007_199_254_740_992.0)
    }
    fn next_normal(&mut self) -> f64 {
        inv_norm_cdf(self.next_uniform())
    }
}

/// Build the GBM asset path `S(t_i)` from a Brownian path `W(t_i)` on the bridge
/// grid, then evaluate the discounted payoff. `payoff_geo` toggles between the
/// geometric-average Asian and the terminal European vanilla.
fn discounted_payoff(mk: &Market, bb: &BrownianBridge, w: &[f64], strike: f64, geo: bool) -> f64 {
    let m = bb.steps();
    let mu = mk.drift() - 0.5 * mk.sigma * mk.sigma;
    if geo {
        // Geometric average of S(t_i): exp( (1/m) Σ ln S(t_i) ).
        let mut log_sum = 0.0;
        for (i, &wi) in w.iter().enumerate() {
            let ti = bb.time(i);
            log_sum += ln(mk.s0) + mu * ti + mk.sigma * wi;
        }
        let g = exp(log_sum / m as f64);
        mk.df() * (g - strike).max(0.0)
    } else {
        // Terminal European: only S(T) matters.
        let ti = bb.time(m - 1);
        let st = mk.s0 * exp(mu * ti + mk.sigma * w[m - 1]);
        mk.df() * (st - strike).max(0.0)
    }
}

/// Plain pseudo-random MC RMSE over `reps` independent runs of `budget` paths,
/// vs the exact value. Uses the SAME Brownian-bridge construction (so the only
/// difference vs QMC is the point set: pseudo-random uniforms instead of
/// scrambled Sobol) — an apples-to-apples variance comparison.
/// Bundled inputs for the RMSE comparison helpers — one config value rather than
/// eight positional arguments (apples-to-apples: the plain-MC and RQMC helpers
/// take the identical case).
struct RmseCase<'a> {
    mk: &'a Market,
    bb: &'a BrownianBridge,
    strike: f64,
    geo: bool,
    budget: usize,
    reps: usize,
    exact: f64,
    base_seed: u64,
}

fn plain_mc_rmse(c: &RmseCase) -> f64 {
    let m = c.bb.steps();
    let mut z = vec![0.0f64; m];
    let mut w = vec![0.0f64; m];
    let mut sq_err = 0.0;
    for r in 0..c.reps {
        let mut rng = PseudoNormals::new(
            c.base_seed
                .wrapping_add((r as u64).wrapping_mul(0x100_0001)),
        );
        let mut acc = 0.0;
        for _ in 0..c.budget {
            for zi in z.iter_mut() {
                *zi = rng.next_normal();
            }
            c.bb.build(&z, &mut w);
            acc += discounted_payoff(c.mk, c.bb, &w, c.strike, c.geo);
        }
        let est = acc / c.budget as f64;
        sq_err += (est - c.exact) * (est - c.exact);
    }
    sqrt(sq_err / c.reps as f64)
}

/// RQMC (scrambled-Sobol + bridge) RMSE over `reps` independent scrambles of
/// `budget` paths, vs the exact value.
fn rqmc_rmse(c: &RmseCase) -> (f64, f64) {
    let mut sq_err = 0.0;
    let mut sum_est = 0.0;
    for r in 0..c.reps {
        // One scramble = one RQMC replication of `budget` points.
        let seed = c
            .base_seed
            .wrapping_add((r as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let res = rqmc_estimate(c.bb, c.budget, 1, seed, |w| {
            discounted_payoff(c.mk, c.bb, w, c.strike, c.geo)
        });
        sq_err += (res.estimate - c.exact) * (res.estimate - c.exact);
        sum_est += res.estimate;
    }
    (sqrt(sq_err / c.reps as f64), sum_est / c.reps as f64)
}

/// **MEASURED variance reduction** on the geometric-average Asian (exact
/// Kemna-Vorst closed form) and the European vanilla (exact Black-Scholes). The
/// ratio is computed, printed, and asserted ≥ 3×; it is a measured number.
#[test]
fn measured_variance_reduction_vs_plain_mc() {
    let mk = Market::base();
    let strike = 100.0;
    let m = 32; // 32 monitoring dates -> a 32-dim QMC problem (bridge shines here)
    let bb = BrownianBridge::new(m, mk.t);

    // ----- Target A: discretely-monitored geometric-average Asian (exact). -----
    let vi = VanillaInputs::new(mk.s0, strike, mk.sigma, mk.t, mk.r_dom, mk.r_for);
    let spec = AnalyticAsian::fresh_discrete(OptionType::Call, strike, m);
    let exact_geo = geometric_average_price(&(&vi).into(), spec);

    let budget = 4096;
    let reps = 24;
    let mc_geo = plain_mc_rmse(&RmseCase {
        mk: &mk,
        bb: &bb,
        strike,
        geo: true,
        budget,
        reps,
        exact: exact_geo,
        base_seed: 0xA51A_0001,
    });
    let (qmc_geo, qmc_geo_mean) = rqmc_rmse(&RmseCase {
        mk: &mk,
        bb: &bb,
        strike,
        geo: true,
        budget,
        reps,
        exact: exact_geo,
        base_seed: 0x5EED_0001,
    });
    let ratio_geo = mc_geo / qmc_geo;
    println!(
        "[geo-Asian] exact={exact_geo:.8}  MC RMSE={mc_geo:.3e}  QMC RMSE={qmc_geo:.3e}  ratio={ratio_geo:.2}x  qmc_mean={qmc_geo_mean:.8}"
    );

    // ----- Target B: European vanilla (exact Black-Scholes). -----
    let exact_eu = celnet_vanilla::price(OptionType::Call, &vi);
    let mc_eu = plain_mc_rmse(&RmseCase {
        mk: &mk,
        bb: &bb,
        strike,
        geo: false,
        budget,
        reps,
        exact: exact_eu,
        base_seed: 0xB0BA_0002,
    });
    let (qmc_eu, qmc_eu_mean) = rqmc_rmse(&RmseCase {
        mk: &mk,
        bb: &bb,
        strike,
        geo: false,
        budget,
        reps,
        exact: exact_eu,
        base_seed: 0xF00D_0002,
    });
    let ratio_eu = mc_eu / qmc_eu;
    println!(
        "[European] exact={exact_eu:.8}  MC RMSE={mc_eu:.3e}  QMC RMSE={qmc_eu:.3e}  ratio={ratio_eu:.2}x  qmc_mean={qmc_eu_mean:.8}"
    );

    // (iii) The MEASURED reduction must be at least 3x on both targets.
    assert!(
        ratio_geo >= 3.0,
        "geometric-Asian QMC variance reduction {ratio_geo:.2}x < 3x (MC {mc_geo:.3e}, QMC {qmc_geo:.3e})"
    );
    assert!(
        ratio_eu >= 3.0,
        "European QMC variance reduction {ratio_eu:.2}x < 3x (MC {mc_eu:.3e}, QMC {qmc_eu:.3e})"
    );

    // (iv) Unbiasedness: the mean over scrambles converges to the exact value
    // within a few QMC standard errors (QMC RMSE estimates that error).
    assert!(
        (qmc_geo_mean - exact_geo).abs() <= 3.0 * qmc_geo,
        "geo-Asian RQMC mean {qmc_geo_mean} biased vs exact {exact_geo} (RMSE {qmc_geo})"
    );
    assert!(
        (qmc_eu_mean - exact_eu).abs() <= 3.0 * qmc_eu,
        "European RQMC mean {qmc_eu_mean} biased vs exact {exact_eu} (RMSE {qmc_eu})"
    );
}

/// Unbiasedness, sharpened: at a *small* per-scramble budget but *many* scrambles,
/// the grand mean of the RQMC estimator converges to the exact known value to
/// well within the standard error of the mean (`RMSE/√reps`). This is the direct
/// statement that scrambled QMC is an unbiased estimator of the true integral.
#[test]
fn rqmc_estimator_is_unbiased() {
    let mk = Market::base();
    let strike = 105.0;
    let m = 16;
    let bb = BrownianBridge::new(m, mk.t);

    let vi = VanillaInputs::new(mk.s0, strike, mk.sigma, mk.t, mk.r_dom, mk.r_for);
    let spec = AnalyticAsian::fresh_discrete(OptionType::Call, strike, m);
    let exact = geometric_average_price(&(&vi).into(), spec);

    let budget = 1024;
    let reps = 64;
    let (rmse, mean) = rqmc_rmse(&RmseCase {
        mk: &mk,
        bb: &bb,
        strike,
        geo: true,
        budget,
        reps,
        exact,
        base_seed: 0xC0DE_1234,
    });
    // Standard error of the grand mean over the `reps` scrambles.
    let se_mean = rmse / sqrt(reps as f64);
    println!(
        "[unbiased] exact={exact:.8} mean={mean:.8} bias={:.3e} se_of_mean={se_mean:.3e}",
        mean - exact
    );
    assert!(
        (mean - exact).abs() <= 4.0 * se_mean,
        "RQMC appears biased: mean {mean}, exact {exact}, |bias| {} > 4·SE {}",
        (mean - exact).abs(),
        4.0 * se_mean
    );
}
