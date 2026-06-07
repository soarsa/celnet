//! Correlated multi-asset FX options — weighted **basket**, **best-of-N** and
//! **worst-of-N** (rainbow) calls and puts over a portfolio of currency pairs,
//! priced by a Cholesky-correlated multi-asset geometric-Brownian-motion
//! Monte-Carlo engine over the scrambled-Sobol / Brownian-bridge quasi-random
//! stack ([`celnet_qmc`]).
//!
//! # Model
//!
//! Each leg `a ∈ {0..N}` is a lognormal Garman-Kohlhagen underlying with its own
//! spot `S_a(0)`, volatility `σ_a` and foreign rate `r_f,a`, all discounted under
//! one **shared domestic (numeraire) rate** `r_d` — the settlement currency of
//! the basket (the instrument's top-level pair). Under the domestic
//! risk-neutral measure each leg evolves as
//!
//! ```text
//! dS_a / S_a = (r_d − r_f,a) dt + σ_a dW_a,   dW_a · dW_b = ρ_ab dt
//! ```
//!
//! so `S_a(T) = S_a(0) · exp[(r_d − r_f,a − ½σ_a²) T + σ_a W_a(T)]`, where the
//! Brownian vector `W = (W_0..W_{N−1})` has instantaneous correlation `ρ`.
//!
//! # Correlated path construction (Cholesky + Brownian bridge)
//!
//! The correlation matrix `ρ` (symmetric, unit diagonal) is factorised by a
//! **lower-triangular Cholesky decomposition** `ρ = L · Lᵀ` (provenance:
//! Benoît / standard numerical linear algebra; the identifier is purpose-named).
//! A draw of `N` independent standard normals `z` is mapped to a correlated
//! vector `x = L · z` with `Cov(x) = ρ`. The decomposition only exists when `ρ`
//! is **symmetric positive-definite**; a non-SPD (e.g. an arbitrage-inconsistent
//! over-correlated) matrix is rejected honestly with [`CorrelationError`] rather
//! than silently regularised.
//!
//! Time discretisation reuses the [`celnet_qmc`] randomized-QMC machinery so the
//! draws are low-discrepancy and bit-reproducible:
//!
//! * one scrambled-Sobol point of dimension `N · m` is drawn per path
//!   (`m` = time steps), the leg-major block `[a·m .. a·m+m)` feeding leg `a`;
//! * each leg's `m` uniforms are mapped to standard normals by the inverse
//!   normal CDF and built into a **Brownian-bridge** Brownian path so the
//!   dominant path variance loads onto the best-distributed Sobol dimensions;
//! * **cross-asset correlation is imposed in the standardized-increment space**:
//!   the per-step increment vector across legs is rotated by `L` *before* the
//!   (linear) bridge, so the shared bridge weight matrix `A` carries the
//!   correlation through to `Cov(W_a(t_i), W_b(t_j)) = ρ_ab · min(t_i, t_j)`
//!   exactly. Equivalently `W_a = A·(L row a · Z)`.
//!
//! The estimate is an unbiased randomized-QMC average over independent scrambles;
//! its [`McEstimate::std_error`] is the *measured* between-scramble standard
//! error — surfaced honestly on the wire (`PriceResponse.price_std_error`) so a
//! caller never mistakes the Monte-Carlo estimate for closed-form precision.
//!
//! # Greeks
//!
//! Multi-asset basket Greeks require a per-leg sensitivity strip (a full
//! `N × {spot, vol}` Jacobian plus the cross-gammas) that is a distinct, larger
//! increment than this first vertical slice; this module returns the **price and
//! its standard error only**. The deferral is explicit (see
//! [`BasketEstimate`]) — the strip is documented as the next increment rather
//! than faked with a single-underlying bump that would be silently wrong for a
//! multi-asset payoff.

use celnet_core::math::{exp, sqrt};
use celnet_qmc::{BrownianBridge, SobolSequence, inv_norm_cdf};
use celnet_types::OptionType;

/// How the per-leg terminal levels combine into the option underlying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasketKind {
    /// Weighted arithmetic basket: the underlying is `Σ_a w_a · S_a(T)` and the
    /// payoff is a vanilla call/put on that weighted sum against the strike.
    Basket,
    /// Best-of-N (rainbow max): the underlying is `max_a w_a · S_a(T)`.
    BestOf,
    /// Worst-of-N (rainbow min): the underlying is `min_a w_a · S_a(T)`.
    WorstOf,
}

/// One leg of a correlated multi-asset option: an FX underlying with its own
/// market data and basket weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasketLeg {
    /// Spot FX level `S_a(0)` (quote per unit base).
    pub spot: f64,
    /// Annualised lognormal volatility `σ_a` (absolute, e.g. `0.10` = 10 vol).
    pub vol: f64,
    /// Continuously-compounded foreign (base) rate `r_f,a` of the leg.
    pub r_for: f64,
    /// The leg weight `w_a` applied to `S_a(T)` in the basket / rainbow
    /// aggregation. May be negative (a short basket leg).
    pub weight: f64,
}

impl BasketLeg {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(spot: f64, vol: f64, r_for: f64, weight: f64) -> Self {
        Self {
            spot,
            vol,
            r_for,
            weight,
        }
    }
}

/// The full specification of a correlated multi-asset option.
#[derive(Debug, Clone, PartialEq)]
pub struct BasketSpec {
    /// The legs (one FX underlying each); `legs.len() == N ≥ 1`.
    pub legs: Vec<BasketLeg>,
    /// The `N × N` instantaneous correlation matrix, row-major. Must be
    /// symmetric with unit diagonal and **positive-definite**.
    pub correlation: Vec<Vec<f64>>,
    /// Call or put on the aggregated underlying.
    pub option_type: OptionType,
    /// The strike `K` on the aggregated underlying.
    pub strike: f64,
    /// The aggregation kind (basket / best-of / worst-of).
    pub kind: BasketKind,
}

/// The Monte-Carlo run configuration for the multi-asset engine.
#[derive(Debug, Clone, Copy)]
pub struct BasketMcConfig {
    /// Scrambled-Sobol points per replication (paths per scramble).
    pub budget: usize,
    /// Independent randomized scrambles (≥ 2 for a finite standard error).
    pub replications: usize,
    /// Time steps per path (`m`).
    pub steps: usize,
    /// The base scramble seed; identical seeds reproduce results bit-for-bit.
    pub seed: u64,
}

impl Default for BasketMcConfig {
    fn default() -> Self {
        Self {
            budget: 8_192,
            replications: 16,
            steps: 1,
            seed: 0,
        }
    }
}

/// The result of a multi-asset Monte-Carlo pricing run.
///
/// Greeks are intentionally **absent** — multi-asset basket sensitivities are a
/// distinct larger increment (a per-leg `N × {spot, vol}` Jacobian plus
/// cross-gammas). This first slice reports the price and its measured standard
/// error only; the field set makes the deferral explicit rather than returning a
/// fabricated single-underlying strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasketEstimate {
    /// The discounted price estimate (in domestic / numeraire currency, per unit
    /// of the aggregated underlying).
    pub price: f64,
    /// The measured between-scramble standard error of [`Self::price`].
    pub std_error: f64,
}

/// Why a correlation matrix was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrelationError {
    /// The matrix is not square / does not match the leg count.
    Shape,
    /// The matrix is not symmetric (within a tight tolerance).
    NotSymmetric,
    /// A diagonal entry is not (within tolerance) `1.0`.
    NonUnitDiagonal,
    /// The matrix is not positive-definite (Cholesky encountered a non-positive
    /// pivot) — it is not a valid correlation matrix.
    NotPositiveDefinite,
}

impl core::fmt::Display for CorrelationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            CorrelationError::Shape => "correlation matrix shape does not match the leg count",
            CorrelationError::NotSymmetric => "correlation matrix is not symmetric",
            CorrelationError::NonUnitDiagonal => "correlation matrix diagonal is not unit",
            CorrelationError::NotPositiveDefinite => {
                "correlation matrix is not positive-definite (not a valid correlation matrix)"
            }
        };
        f.write_str(s)
    }
}

impl std::error::Error for CorrelationError {}

/// A validated lower-triangular Cholesky factor `L` of a correlation matrix
/// (`Σ = L · Lᵀ`), produced by [`cholesky`].
#[derive(Debug, Clone, PartialEq)]
pub struct CholeskyFactor {
    /// Row-major lower-triangular factor; `l[i][j] == 0` for `j > i`.
    l: Vec<Vec<f64>>,
}

impl CholeskyFactor {
    /// Dimension `N`.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.l.len()
    }

    /// The `(i, j)` factor entry (`0` above the diagonal).
    #[must_use]
    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.l[i][j]
    }

    /// Apply the factor: `x = L · z` (correlate an i.i.d. standard-normal vector
    /// `z` into one with covariance `Σ`). `z.len()` and `out.len()` must equal
    /// [`Self::dim`].
    fn apply(&self, z: &[f64], out: &mut [f64]) {
        for (i, row) in self.l.iter().enumerate() {
            // Lower-triangular: only j ≤ i contribute.
            let mut acc = 0.0;
            for (j, &lij) in row.iter().enumerate().take(i + 1) {
                acc += lij * z[j];
            }
            out[i] = acc;
        }
    }
}

/// Validate and Cholesky-decompose a correlation matrix `Σ = L · Lᵀ`.
///
/// Provenance: the classic Cholesky (Benoît) decomposition for a symmetric
/// positive-definite matrix; the identifier stays purpose-named.
///
/// # Errors
///
/// [`CorrelationError`] if the matrix is not square `n × n`, not symmetric, has
/// a non-unit diagonal, or is not positive-definite.
pub fn cholesky(sigma: &[Vec<f64>], n: usize) -> Result<CholeskyFactor, CorrelationError> {
    if sigma.len() != n || sigma.iter().any(|r| r.len() != n) {
        return Err(CorrelationError::Shape);
    }
    // Symmetry and unit diagonal (correlation-matrix invariants).
    const SYM_TOL: f64 = 1e-12;
    const DIAG_TOL: f64 = 1e-9;
    for (i, row) in sigma.iter().enumerate() {
        if (row[i] - 1.0).abs() > DIAG_TOL {
            return Err(CorrelationError::NonUnitDiagonal);
        }
        for (j, &rij) in row.iter().enumerate().skip(i + 1) {
            if (rij - sigma[j][i]).abs() > SYM_TOL {
                return Err(CorrelationError::NotSymmetric);
            }
        }
    }

    // Build L row by row: each new row `i` reads only the already-completed rows
    // `done` (j < i) and its own partial entries (j == i ⇒ the diagonal pivot).
    let mut l: Vec<Vec<f64>> = Vec::with_capacity(n);
    for (i, sigma_row) in sigma.iter().enumerate() {
        let done: &[Vec<f64>] = &l;
        let mut li = vec![0.0f64; n];
        for j in 0..=i {
            // s = Σ_ij − Σ_{k<j} L_ik L_jk (dot of the two rows' first j entries).
            let lj: &[f64] = if j < i { &done[j] } else { &li };
            let dot: f64 = li[..j].iter().zip(&lj[..j]).map(|(a, b)| a * b).sum();
            let s = sigma_row[j] - dot;
            if i == j {
                // Diagonal: a non-positive pivot ⇒ not positive-definite.
                if s <= 0.0 {
                    return Err(CorrelationError::NotPositiveDefinite);
                }
                li[j] = sqrt(s);
            } else {
                li[j] = s / done[j][j];
            }
        }
        l.push(li);
    }
    Ok(CholeskyFactor { l })
}

/// Price a correlated multi-asset option (weighted basket / best-of / worst-of)
/// by Cholesky-correlated multi-asset GBM Monte-Carlo over the scrambled-Sobol /
/// Brownian-bridge QMC stack.
///
/// `r_dom` is the shared domestic (numeraire / settlement-currency) rate; `t` is
/// the expiry in vol-time years.
///
/// # Errors
///
/// [`CorrelationError`] if `spec.correlation` is not a valid SPD correlation
/// matrix for `spec.legs.len()` legs.
///
/// # Panics
///
/// Panics if `spec.legs` is empty, or if the MC config has a zero budget /
/// replication / step count (caller-side invariants the server validates first).
pub fn price_basket(
    spec: &BasketSpec,
    r_dom: f64,
    t: f64,
    cfg: BasketMcConfig,
) -> Result<BasketEstimate, CorrelationError> {
    let n = spec.legs.len();
    assert!(n >= 1, "a basket needs at least one leg");
    assert!(cfg.budget >= 1, "need ≥ 1 path");
    assert!(cfg.replications >= 2, "need ≥ 2 scrambles for a std-error");
    assert!(cfg.steps >= 1, "need ≥ 1 time step");

    let chol = cholesky(&spec.correlation, n)?;

    let m = cfg.steps;
    let bridge = BrownianBridge::new(m, t);
    let dim = n * m;
    let seq = SobolSequence::new(dim);
    let df = exp(-r_dom * t);

    // Per-leg deterministic drift and diffusion scale at the terminal horizon.
    // S_a(T) = S_a(0) · exp[(r_d − r_f,a − ½σ_a²) T + σ_a W_a(T)].
    let drift: Vec<f64> = spec
        .legs
        .iter()
        .map(|leg| (r_dom - leg.r_for - 0.5 * leg.vol * leg.vol) * t)
        .collect();

    let mut rep_means = Vec::with_capacity(cfg.replications);

    // Scratch buffers reused across paths (zero per-path allocation in the hot
    // loop beyond the Sobol point vector).
    let mut u = vec![0.0f64; dim];
    // z_step[a] = leg a's independent normal increment at the current step;
    // x_step[a] = correlated increment after L; one bridge path buffer per leg.
    let mut z_step = vec![0.0f64; n];
    let mut x_step = vec![0.0f64; n];
    // The correlated standard normals laid out leg-major for the bridge.
    let mut z_corr = vec![0.0f64; dim];
    let mut path = vec![0.0f64; m];
    let mut w_terminal = vec![0.0f64; n];

    for r in 0..cfg.replications {
        let seed = derive_seed(cfg.seed, r as u64);
        let mut stream = seq.stream(seed);
        let mut acc = 0.0f64;
        for _ in 0..cfg.budget {
            stream.next_point(&mut u);

            // Map uniforms → independent normals, then correlate across legs at
            // EACH step (the shared linear bridge carries ρ through to the path
            // covariance). Layout: leg-major, u[a*m + k] is leg a, step k.
            for k in 0..m {
                for (a, zs) in z_step.iter_mut().enumerate() {
                    *zs = inv_norm_cdf(u[a * m + k]);
                }
                chol.apply(&z_step, &mut x_step);
                for (a, &xs) in x_step.iter().enumerate() {
                    z_corr[a * m + k] = xs;
                }
            }

            // Bridge each leg's correlated normals into its Brownian path and
            // read off W_a(T) (the last grid point).
            for (a, wt) in w_terminal.iter_mut().enumerate() {
                let base = a * m;
                bridge.build(&z_corr[base..base + m], &mut path);
                *wt = path[m - 1];
            }

            acc += discounted_payoff(spec, &drift, &w_terminal, df);
        }
        rep_means.push(acc / cfg.budget as f64);
    }

    let estimate = mean(&rep_means);
    let var = sample_variance(&rep_means, estimate);
    let std_error = sqrt(var / cfg.replications as f64);

    Ok(BasketEstimate {
        price: estimate,
        std_error,
    })
}

/// One path's discounted payoff given the terminal Brownian levels `W_a(T)`.
#[inline]
fn discounted_payoff(spec: &BasketSpec, drift: &[f64], w_terminal: &[f64], df: f64) -> f64 {
    // Aggregate the weighted terminal levels per the basket kind.
    let agg = match spec.kind {
        BasketKind::Basket => spec
            .legs
            .iter()
            .zip(drift)
            .zip(w_terminal)
            .map(|((leg, &mu), &w)| leg.weight * terminal_level(leg, mu, w))
            .sum::<f64>(),
        BasketKind::BestOf => spec
            .legs
            .iter()
            .zip(drift)
            .zip(w_terminal)
            .map(|((leg, &mu), &w)| leg.weight * terminal_level(leg, mu, w))
            .fold(f64::NEG_INFINITY, f64::max),
        BasketKind::WorstOf => spec
            .legs
            .iter()
            .zip(drift)
            .zip(w_terminal)
            .map(|((leg, &mu), &w)| leg.weight * terminal_level(leg, mu, w))
            .fold(f64::INFINITY, f64::min),
    };

    let intrinsic = match spec.option_type {
        OptionType::Call => (agg - spec.strike).max(0.0),
        OptionType::Put => (spec.strike - agg).max(0.0),
    };
    df * intrinsic
}

/// Terminal lognormal level `S_a(T) = S_a(0) · exp[μ_a + σ_a W_a(T)]`.
#[inline]
fn terminal_level(leg: &BasketLeg, drift: f64, w_terminal: f64) -> f64 {
    leg.spot * exp(drift + leg.vol * w_terminal)
}

/// Derive an independent per-replication scramble seed (SplitMix64 avalanche),
/// matching the spacing [`celnet_qmc::rqmc_estimate`] uses.
#[inline]
fn derive_seed(base: u64, rep: u64) -> u64 {
    let mut z = base.wrapping_add(rep.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[inline]
fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

#[inline]
fn sample_variance(xs: &[f64], m: f64) -> f64 {
    let n = xs.len();
    debug_assert!(n >= 2);
    xs.iter().map(|&x| (x - m) * (x - m)).sum::<f64>() / (n - 1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corr2(rho: f64) -> Vec<Vec<f64>> {
        vec![vec![1.0, rho], vec![rho, 1.0]]
    }

    #[test]
    fn cholesky_recovers_matrix() {
        let sigma = corr2(0.5);
        let l = cholesky(&sigma, 2).unwrap();
        // L·Lᵀ == Σ.
        for (i, sigma_row) in sigma.iter().enumerate() {
            for (j, &sij) in sigma_row.iter().enumerate() {
                let s: f64 = (0..2).map(|k| l.get(i, k) * l.get(j, k)).sum();
                assert!((s - sij).abs() < 1e-14);
            }
        }
    }

    #[test]
    fn cholesky_rejects_non_psd() {
        // ρ = 1.01 is over-correlated ⇒ indefinite.
        let bad = corr2(1.01);
        assert_eq!(
            cholesky(&bad, 2),
            Err(CorrelationError::NotPositiveDefinite)
        );
    }

    #[test]
    fn cholesky_rejects_asymmetric() {
        let bad = vec![vec![1.0, 0.3], vec![0.5, 1.0]];
        assert_eq!(cholesky(&bad, 2), Err(CorrelationError::NotSymmetric));
    }

    #[test]
    fn reproducible_bit_identical() {
        let spec = BasketSpec {
            legs: vec![
                BasketLeg::new(1.0, 0.1, 0.01, 1.0),
                BasketLeg::new(1.2, 0.12, 0.015, 1.0),
            ],
            correlation: corr2(0.3),
            option_type: OptionType::Call,
            strike: 2.2,
            kind: BasketKind::Basket,
        };
        let cfg = BasketMcConfig {
            budget: 512,
            replications: 4,
            steps: 1,
            seed: 7,
        };
        let a = price_basket(&spec, 0.02, 1.0, cfg).unwrap();
        let b = price_basket(&spec, 0.02, 1.0, cfg).unwrap();
        assert_eq!(a.price.to_bits(), b.price.to_bits());
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
    }

    #[test]
    fn structural_sandwich_worst_le_best() {
        // worst-of ≤ best-of for a call on identical legs, any correlation.
        let legs = vec![
            BasketLeg::new(1.0, 0.15, 0.01, 1.0),
            BasketLeg::new(1.0, 0.18, 0.01, 1.0),
        ];
        let cfg = BasketMcConfig {
            budget: 4096,
            replications: 16,
            steps: 1,
            seed: 99,
        };
        let mk = |kind| BasketSpec {
            legs: legs.clone(),
            correlation: corr2(0.4),
            option_type: OptionType::Call,
            strike: 1.0,
            kind,
        };
        let best = price_basket(&mk(BasketKind::BestOf), 0.02, 1.0, cfg)
            .unwrap()
            .price;
        let worst = price_basket(&mk(BasketKind::WorstOf), 0.02, 1.0, cfg)
            .unwrap()
            .price;
        assert!(worst <= best, "worst {worst} should be ≤ best {best}");
    }
}
