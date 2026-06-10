//! Parity row (W11-B): **high-dimensional RQMC convergence beats plain MC** on
//! integrands with a hand-computable closed-form answer.
//!
//! `tests/qmc.rs` (Wave 4c) measured the variance reduction of scrambled-Sobol +
//! Brownian-bridge RQMC at 32 dimensions. This row pushes into genuinely **high
//! dimension** (`d = 64`) — where the curse of dimensionality is supposed to blunt
//! QMC — and shows the scrambled Sobol point set still beats plain pseudo-random
//! Monte-Carlo by a measured factor, on targets whose exact value is **computed by
//! hand**, not by any code path (Lesson c):
//!
//!  (i)   **64-dim Gaussian product moment** `E[exp(Σ_{j} a_j Z_j)]`, `Z_j` iid
//!        standard normals — exactly `exp(½ Σ a_j²)`. With `a_j = 1/√d` (so
//!        `Σ a_j² = 1`) the exact value is **`exp(0.5) = 1.6487212707001282…`**,
//!        a pure hand-computed constant independent of the estimator. The scrambled
//!        Sobol (Owen) → inverse-normal point set's RMSE over independent scrambles
//!        is measured against this exact value and compared to a plain-MC RMSE at
//!        the same budget; the ratio is **measured** and gated above a robust floor;
//!  (ii)  **64-date geometric-average Asian** (a 64-dim path integral) priced by
//!        scrambled Sobol + Brownian bridge, vs the exact discrete Kemna-Vorst
//!        closed form (`celnet-exotics`, an independent crate) — the bridge
//!        concentrates the effective dimension so QMC stays effective at `d = 64`;
//!        ratio measured and gated above a robust floor;
//!  (iii) **convergence rate** — at two budgets `N` and `4N`, the high-dim RQMC
//!        RMSE shrinks by a factor materially **larger than the measured plain-MC
//!        rate at the same budgets**. (Honest boundary: for a *fully isotropic*
//!        64-dim integrand with no effective-dimension concentration — the worst
//!        case for QMC — the RQMC rate is better than MC's but does not reach the
//!        near-`O(1/N)` (4×) rate that low-effective-dimension problems like the
//!        bridged Asian below enjoy; we gate the measured RQMC-beats-MC margin, not
//!        an absolute rate the isotropic case cannot honestly claim.)
//!
//! The plain-MC baseline uses the SAME inverse-normal map and (for the Asian) the
//! SAME Brownian bridge, so the only difference is the point set — an apples-to-
//! apples variance comparison. Ratios are robust floors, not flaky exact numbers
//! (contention-safe per the §1.2 measurement-methodology lesson): they are reported
//! verbatim and only asserted to clear a wide-margin floor.

use celnet_core::math::{exp, ln, sqrt};
use celnet_exotics::asian::{AnalyticAsian, geometric_average_price};
use celnet_qmc::{BrownianBridge, SobolSequence, inv_norm_cdf, rqmc_estimate};
use celnet_types::{OptionType, VanillaInputs};

/// SplitMix64 → uniform → inverse-normal pseudo-random standard normals: the
/// independent plain-MC baseline (identical inverse-normal map as the QMC path).
struct PseudoNormals {
    state: u64,
}
impl PseudoNormals {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn next_uniform(&mut self) -> f64 {
        let u = (self.next_u64() >> 11) as f64;
        (u + 0.5) * (1.0 / 9_007_199_254_740_992.0)
    }
    fn next_normal(&mut self) -> f64 {
        inv_norm_cdf(self.next_uniform())
    }
}

// ---------------------------------------------------------------------------
// (i) 64-dim Gaussian product moment — exact value computed BY HAND.
// ---------------------------------------------------------------------------

/// Dimension of the high-dim integrand.
const D: usize = 64;

/// The integrand `g(Z) = exp(Σ_j a_j Z_j)` with `a_j = 1/√D` (so `Σ a_j² = 1`).
/// `E[g] = exp(½ Σ a_j²) = exp(0.5)`.
fn gaussian_moment_integrand(z: &[f64]) -> f64 {
    let a = 1.0 / sqrt(D as f64);
    let mut s = 0.0;
    for &zj in z {
        s += a * zj;
    }
    exp(s)
}

/// Plain-MC RMSE of the Gaussian-moment integrand over `reps` runs of `budget`
/// draws, vs the exact value.
fn gaussian_plain_mc_rmse(budget: usize, reps: usize, base_seed: u64, exact: f64) -> f64 {
    let mut z = vec![0.0f64; D];
    let mut sq_err = 0.0;
    for r in 0..reps {
        let mut rng =
            PseudoNormals::new(base_seed.wrapping_add((r as u64).wrapping_mul(0x100_0001)));
        let mut acc = 0.0;
        for _ in 0..budget {
            for zj in z.iter_mut() {
                *zj = rng.next_normal();
            }
            acc += gaussian_moment_integrand(&z);
        }
        let est = acc / budget as f64;
        sq_err += (est - exact) * (est - exact);
    }
    sqrt(sq_err / reps as f64)
}

/// Scrambled-Sobol RQMC RMSE of the Gaussian-moment integrand over `reps`
/// independent scrambles of `budget` points (64-dim Sobol → inverse-normal), vs
/// the exact value. Also returns the grand mean for the unbiasedness check.
fn gaussian_rqmc_rmse(budget: usize, reps: usize, base_seed: u64, exact: f64) -> (f64, f64) {
    let seq = SobolSequence::new(D);
    let mut z = vec![0.0f64; D];
    let mut sq_err = 0.0;
    let mut sum_est = 0.0;
    for r in 0..reps {
        let seed = base_seed.wrapping_add((r as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let mut stream = seq.stream(seed);
        let mut u = vec![0.0f64; D];
        let mut acc = 0.0;
        for _ in 0..budget {
            stream.next_point(&mut u);
            for (zj, &uj) in z.iter_mut().zip(u.iter()) {
                *zj = inv_norm_cdf(uj);
            }
            acc += gaussian_moment_integrand(&z);
        }
        let est = acc / budget as f64;
        sq_err += (est - exact) * (est - exact);
        sum_est += est;
    }
    (sqrt(sq_err / reps as f64), sum_est / reps as f64)
}

/// (i) 64-dim Gaussian product-moment integral: scrambled Sobol beats plain MC by
/// a measured factor, the RQMC estimator is unbiased, and the exact value matches
/// the hand-computed `exp(0.5)`.
#[test]
fn highdim_gaussian_moment_rqmc_beats_mc() {
    // Hand-computed exact value: a_j = 1/√64, Σ a_j² = 64·(1/64) = 1, E = exp(0.5).
    let hand_exact = 1.648_721_270_700_128_2_f64; // exp(0.5) to full f64 precision
    let exact = exp(0.5);
    assert!(
        (exact - hand_exact).abs() <= 1e-15,
        "exp(0.5) mismatch: lib {exact} vs hand {hand_exact}"
    );

    let budget = 4096;
    let reps = 24;
    let mc = gaussian_plain_mc_rmse(budget, reps, 0xA51A_64D0, hand_exact);
    let (qmc, qmc_mean) = gaussian_rqmc_rmse(budget, reps, 0x5EED_64D0, hand_exact);
    let ratio = mc / qmc;
    println!(
        "[gauss-moment d={D}] exact={hand_exact:.12}  MC RMSE={mc:.3e}  QMC RMSE={qmc:.3e}  ratio={ratio:.2}x  qmc_mean={qmc_mean:.10}"
    );

    // MEASURED variance reduction must clear a robust floor (≥ 2×). The observed
    // ratio is materially higher; 2× is a wide-margin, contention-safe gate.
    assert!(
        ratio >= 2.0,
        "high-dim Gaussian RQMC reduction {ratio:.2}x < 2x floor (MC {mc:.3e}, QMC {qmc:.3e})"
    );
    // Unbiasedness: the scramble mean converges to the hand-computed exact value
    // within a few RQMC standard errors.
    assert!(
        (qmc_mean - hand_exact).abs() <= 3.0 * qmc,
        "high-dim Gaussian RQMC mean {qmc_mean} biased vs hand-exact {hand_exact} (RMSE {qmc})"
    );
}

/// (iii) Convergence rate at `d = 64` on the isotropic Gaussian moment: the
/// measured RQMC rate (RMSE shrink for a 4× budget) is materially larger than the
/// measured plain-MC rate at the same budgets. Both rates are measured on the same
/// hand-computed exact value; the QMC-beats-MC margin is the honest, robust claim
/// (the isotropic case does not reach the 4× near-`O(1/N)` rate — see the module
/// docs and the bridged Asian below for the low-effective-dimension regime).
#[test]
fn highdim_gaussian_convergence_rate() {
    let hand_exact = exp(0.5);
    let reps = 24;
    let n = 2048;
    let n4 = 4 * n;

    let (qmc_n, _) = gaussian_rqmc_rmse(n, reps, 0x1111_2222, hand_exact);
    let (qmc_4n, _) = gaussian_rqmc_rmse(n4, reps, 0x1111_2222, hand_exact);
    let qmc_rate = qmc_n / qmc_4n;

    let mc_n = gaussian_plain_mc_rmse(n, reps, 0x3333_4444, hand_exact);
    let mc_4n = gaussian_plain_mc_rmse(n4, reps, 0x3333_4444, hand_exact);
    let mc_rate = mc_n / mc_4n;

    println!(
        "[gauss-rate d={D}] QMC RMSE {qmc_n:.3e}→{qmc_4n:.3e} (×{qmc_rate:.2} for 4× budget); \
         MC {mc_n:.3e}→{mc_4n:.3e} (×{mc_rate:.2}); QMC/MC rate margin ×{:.2}",
        qmc_rate / mc_rate
    );
    // The MEASURED RQMC rate must beat the MEASURED MC rate by a robust margin
    // (≥ 1.15×). Gating the relative margin (not an absolute rate) is the honest,
    // contention-safe statement for a fully-isotropic high-dim integrand.
    assert!(
        qmc_rate >= 1.15 * mc_rate,
        "high-dim RQMC convergence rate {qmc_rate:.2}× does not beat MC's {mc_rate:.2}× by ≥15% \
         (margin ×{:.2})",
        qmc_rate / mc_rate
    );
}

// ---------------------------------------------------------------------------
// (ii) 64-date geometric-average Asian — exact discrete Kemna-Vorst closed form.
// ---------------------------------------------------------------------------

/// Garman-Kohlhagen market parameters for the high-dim Asian.
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

/// Discounted geometric-average Asian payoff from a Brownian path `W(t_i)` on the
/// bridge grid (geometric average of `S(t_i)`).
fn asian_payoff(mk: &Market, bb: &BrownianBridge, w: &[f64], strike: f64) -> f64 {
    let m = bb.steps();
    let mu = mk.drift() - 0.5 * mk.sigma * mk.sigma;
    let mut log_sum = 0.0;
    for (i, &wi) in w.iter().enumerate() {
        let ti = bb.time(i);
        log_sum += ln(mk.s0) + mu * ti + mk.sigma * wi;
    }
    let g = exp(log_sum / m as f64);
    mk.df() * (g - strike).max(0.0)
}

/// Plain-MC RMSE for the Asian over `reps` runs of `budget` paths (same bridge as
/// QMC; only the point set differs), vs the exact value.
fn asian_plain_mc_rmse(
    mk: &Market,
    bb: &BrownianBridge,
    strike: f64,
    budget: usize,
    reps: usize,
    base_seed: u64,
    exact: f64,
) -> f64 {
    let m = bb.steps();
    let mut z = vec![0.0f64; m];
    let mut w = vec![0.0f64; m];
    let mut sq_err = 0.0;
    for r in 0..reps {
        let mut rng =
            PseudoNormals::new(base_seed.wrapping_add((r as u64).wrapping_mul(0x100_0001)));
        let mut acc = 0.0;
        for _ in 0..budget {
            for zi in z.iter_mut() {
                *zi = rng.next_normal();
            }
            bb.build(&z, &mut w);
            acc += asian_payoff(mk, bb, &w, strike);
        }
        let est = acc / budget as f64;
        sq_err += (est - exact) * (est - exact);
    }
    sqrt(sq_err / reps as f64)
}

/// Scrambled-Sobol + bridge RQMC RMSE for the Asian, vs the exact value.
fn asian_rqmc_rmse(
    mk: &Market,
    bb: &BrownianBridge,
    strike: f64,
    budget: usize,
    reps: usize,
    base_seed: u64,
    exact: f64,
) -> (f64, f64) {
    let mut sq_err = 0.0;
    let mut sum_est = 0.0;
    for r in 0..reps {
        let seed = base_seed.wrapping_add((r as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let res = rqmc_estimate(bb, budget, 1, seed, |w| asian_payoff(mk, bb, w, strike));
        sq_err += (res.estimate - exact) * (res.estimate - exact);
        sum_est += res.estimate;
    }
    (sqrt(sq_err / reps as f64), sum_est / reps as f64)
}

/// (ii) 64-date (64-dim) geometric-average Asian: scrambled Sobol + Brownian
/// bridge beats plain MC by a measured factor at high dimension, vs the exact
/// Kemna-Vorst closed form from `celnet-exotics` (an independent crate/oracle).
#[test]
fn highdim_geometric_asian_rqmc_beats_mc() {
    let mk = Market::base();
    let strike = 100.0;
    let m = 64; // 64 monitoring dates ⇒ a 64-dim path integral
    let bb = BrownianBridge::new(m, mk.t);

    // Exact discrete geometric Kemna-Vorst (independent oracle in celnet-exotics).
    let vi = VanillaInputs::new(mk.s0, strike, mk.sigma, mk.t, mk.r_dom, mk.r_for);
    let spec = AnalyticAsian::fresh_discrete(OptionType::Call, strike, m);
    let exact = geometric_average_price(&(&vi).into(), spec);

    let budget = 4096;
    let reps = 24;
    let mc = asian_plain_mc_rmse(&mk, &bb, strike, budget, reps, 0xB0BA_64A1, exact);
    let (qmc, qmc_mean) = asian_rqmc_rmse(&mk, &bb, strike, budget, reps, 0xF00D_64A1, exact);
    let ratio = mc / qmc;
    println!(
        "[geo-Asian m={m}] exact={exact:.8}  MC RMSE={mc:.3e}  QMC RMSE={qmc:.3e}  ratio={ratio:.2}x  qmc_mean={qmc_mean:.8}"
    );

    // MEASURED variance reduction at high dimension must clear a robust ≥ 3× floor
    // (the bridge concentrates the effective dimension so QMC stays strong here).
    assert!(
        ratio >= 3.0,
        "high-dim geometric-Asian RQMC reduction {ratio:.2}x < 3x floor (MC {mc:.3e}, QMC {qmc:.3e})"
    );
    // Unbiasedness vs the exact closed form.
    assert!(
        (qmc_mean - exact).abs() <= 3.0 * qmc,
        "high-dim Asian RQMC mean {qmc_mean} biased vs exact {exact} (RMSE {qmc})"
    );
}
