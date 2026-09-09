//! Correlated multi-asset options — weighted **basket**, **best-of-N** and
//! **worst-of-N** (rainbow) calls and puts over a portfolio of underlyings
//! (currency pairs, or any carry-parametrised asset), priced by a
//! Cholesky-correlated multi-asset geometric-Brownian-motion Monte-Carlo engine
//! over the scrambled-Sobol / Brownian-bridge quasi-random stack
//! ([`celnet_qmc`]).
//!
//! # Model
//!
//! Each leg `a ∈ {0..N}` is a lognormal underlying with its own spot `S_a(0)`,
//! volatility `σ_a` and net cost-of-carry `b_a` (FX: `b_a = r_d − r_f,a`), all
//! discounted under one **shared numeraire** [`Carry`] — the settlement currency
//! of the basket (the instrument's top-level pair), whose
//! [`Carry::discount_rate`] is the historical domestic rate `r_d`. Under the
//! settlement risk-neutral measure each leg evolves as
//!
//! ```text
//! dS_a / S_a = b_a dt + σ_a dW_a,   dW_a · dW_b = ρ_ab dt
//! ```
//!
//! so `S_a(T) = S_a(0) · exp[(b_a − ½σ_a²) T + σ_a W_a(T)]`, where the
//! Brownian vector `W = (W_0..W_{N−1})` has instantaneous correlation `ρ`.
//! For FX legs `b_a = r_d − r_f,a` computed with the same two flops as the
//! historical form, so the FX drift is byte-identical.
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
//! # Greeks & Sensitivities
//!
//! Multi-asset basket options support per-leg spot delta sensitivities
//! `∂V/∂S_a` computed via exact, unbiased pathwise differentiation
//! (Broadie & Glasserman 1996) alongside the discounted payoff over the
//! scrambled-Sobol / Brownian-bridge simulation.
//!
//! Callers that need only the price and standard error use [`price_basket`];
//! callers requiring the full per-leg sensitivity strip use
//! [`price_basket_with_sensitivities`], which produces a [`BasketSensitivities`]
//! report containing per-leg deltas and their measured between-scramble
//! standard errors.

use celnet_core::math::{exp, sqrt};
use celnet_qmc::{BrownianBridge, SobolSequence, inv_norm_cdf};
use celnet_types::{Carry, OptionType};

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

/// One leg of a correlated multi-asset option: an underlying with its own
/// market data and basket weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasketLeg {
    /// Spot level `S_a(0)` (quote per unit base, for FX).
    pub spot: f64,
    /// Annualised lognormal volatility `σ_a` (absolute, e.g. `0.10` = 10 vol).
    pub vol: f64,
    /// Net cost-of-carry `b_a` of the leg under the shared settlement measure
    /// (FX: `b_a = r_d − r_f,a`; equity: `r − q_a`; commodity:
    /// `r − convenience_a`).
    pub carry_rate: f64,
    /// The leg weight `w_a` applied to `S_a(T)` in the basket / rainbow
    /// aggregation. May be negative (a short basket leg).
    pub weight: f64,
}

impl BasketLeg {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(spot: f64, vol: f64, carry_rate: f64, weight: f64) -> Self {
        Self {
            spot,
            vol,
            carry_rate,
            weight,
        }
    }
}

/// The full specification of a correlated multi-asset option.
#[derive(Debug, Clone, PartialEq)]
pub struct BasketSpec {
    /// The legs (one underlying each); `legs.len() == N ≥ 1`.
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
/// For callers needing the price and its standard error only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasketEstimate {
    /// The discounted price estimate (in domestic / numeraire currency, per unit
    /// of the aggregated underlying).
    pub price: f64,
    /// The measured between-scramble standard error of [`Self::price`].
    pub std_error: f64,
}

/// The result of a multi-asset Monte-Carlo pricing and sensitivity run.
///
/// Provides the discounted price estimate and standard error alongside the
/// unbiased per-leg pathwise delta sensitivities (`∂V/∂S_a`) and their
/// measured standard errors across independent scrambles.
#[derive(Debug, Clone, PartialEq)]
pub struct BasketSensitivities {
    /// The discounted price estimate (in domestic / numeraire currency, per unit
    /// of the aggregated underlying).
    pub price: f64,
    /// The measured between-scramble standard error of [`Self::price`].
    pub price_std_error: f64,
    /// Per-leg spot delta `∂V/∂S_a` (unbiased pathwise estimator).
    pub leg_deltas: Vec<f64>,
    /// The measured between-scramble standard error of each leg's delta.
    pub leg_delta_std_errors: Vec<f64>,
}

impl BasketSensitivities {
    /// Extract the price-only [`BasketEstimate`].
    #[must_use]
    pub fn estimate(&self) -> BasketEstimate {
        BasketEstimate {
            price: self.price,
            std_error: self.price_std_error,
        }
    }
}

impl From<BasketSensitivities> for BasketEstimate {
    fn from(s: BasketSensitivities) -> Self {
        s.estimate()
    }
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
    /// Dimension `N`.
    dim: usize,
    /// Flat row-major lower-triangular factor; `data[i * dim + j] == 0` for `j > i`.
    data: Vec<f64>,
}

impl CholeskyFactor {
    /// Dimension `N`.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The `(i, j)` factor entry (`0` above the diagonal).
    #[must_use]
    pub fn get(&self, i: usize, j: usize) -> f64 {
        if i < self.dim && j <= i {
            self.data[i * self.dim + j]
        } else {
            0.0
        }
    }

    /// Apply the factor: `x = L · z` (correlate an i.i.d. standard-normal vector
    /// `z` into one with covariance `Σ`). `z.len()` and `out.len()` must equal
    /// [`Self::dim`].
    #[allow(clippy::needless_range_loop)]
    fn apply(&self, z: &[f64], out: &mut [f64]) {
        let n = self.dim;
        for i in 0..n {
            let row_offset = i * n;
            let mut acc = 0.0;
            for j in 0..=i {
                acc += self.data[row_offset + j] * z[j];
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

    // Build L row by row in a flat contiguous buffer:
    let mut data = vec![0.0f64; n * n];
    for (i, sigma_row) in sigma.iter().enumerate() {
        let i_offset = i * n;
        for j in 0..=i {
            let j_offset = j * n;
            // s = Σ_ij − Σ_{k<j} L_ik L_jk (dot of row i and row j first j entries).
            let dot: f64 = (0..j)
                .map(|k| data[i_offset + k] * data[j_offset + k])
                .sum();
            let s = sigma_row[j] - dot;
            if i == j {
                // Diagonal: a non-positive pivot ⇒ not positive-definite.
                if s <= 0.0 {
                    return Err(CorrelationError::NotPositiveDefinite);
                }
                data[i_offset + j] = sqrt(s);
            } else {
                data[i_offset + j] = s / data[j_offset + j];
            }
        }
    }
    Ok(CholeskyFactor { dim: n, data })
}

/// Price a correlated multi-asset option (weighted basket / best-of / worst-of)
/// by Cholesky-correlated multi-asset GBM Monte-Carlo over the scrambled-Sobol /
/// Brownian-bridge QMC stack.
///
/// `carry` is the shared numeraire (settlement-currency) carry — only its
/// discount side is read (`discount_df`; FX: `e^{−r_d·t}`, byte-identical to the
/// historical form). Each leg supplies its own [`BasketLeg::carry_rate`]. `t` is
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
    carry: Carry,
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
    let df = carry.discount_df(t);

    // Per-leg deterministic drift and diffusion scale at the terminal horizon.
    // S_a(T) = S_a(0) · exp[(b_a − ½σ_a²) T + σ_a W_a(T)].
    let drift: Vec<f64> = spec
        .legs
        .iter()
        .map(|leg| (leg.carry_rate - 0.5 * leg.vol * leg.vol) * t)
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

/// Price a correlated multi-asset option alongside per-leg spot delta sensitivities
/// (`∂V/∂S_a`) by Cholesky-correlated multi-asset GBM Monte-Carlo with exact,
/// unbiased pathwise differentiation.
///
/// Returns a [`BasketSensitivities`] report containing the discounted price
/// estimate, standard error, per-leg deltas, and their measured between-scramble
/// standard errors.
///
/// # Errors
///
/// [`CorrelationError`] if `spec.correlation` is not a valid SPD correlation
/// matrix for `spec.legs.len()` legs.
///
/// # Panics
///
/// Panics if `spec.legs` is empty, or if the MC config has a zero budget /
/// replication / step count.
pub fn price_basket_with_sensitivities(
    spec: &BasketSpec,
    carry: Carry,
    t: f64,
    cfg: BasketMcConfig,
) -> Result<BasketSensitivities, CorrelationError> {
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
    let df = carry.discount_df(t);

    // Per-leg deterministic drift and diffusion scale at the terminal horizon.
    // S_a(T) = S_a(0) · exp[(b_a − ½σ_a²) T + σ_a W_a(T)].
    let drift: Vec<f64> = spec
        .legs
        .iter()
        .map(|leg| (leg.carry_rate - 0.5 * leg.vol * leg.vol) * t)
        .collect();

    let mut rep_means = Vec::with_capacity(cfg.replications);
    let mut delta_reps: Vec<Vec<f64>> = vec![Vec::with_capacity(cfg.replications); n];

    // Scratch buffers reused across paths (zero per-path allocation in the hot
    // loop beyond the Sobol point vector).
    let mut u = vec![0.0f64; dim];
    let mut z_step = vec![0.0f64; n];
    let mut x_step = vec![0.0f64; n];
    let mut z_corr = vec![0.0f64; dim];
    let mut path = vec![0.0f64; m];
    let mut w_terminal = vec![0.0f64; n];
    let mut delta_rep_acc = vec![0.0f64; n];

    for r in 0..cfg.replications {
        let seed = derive_seed(cfg.seed, r as u64);
        let mut stream = seq.stream(seed);
        let mut acc = 0.0f64;
        delta_rep_acc.fill(0.0);

        for _ in 0..cfg.budget {
            stream.next_point(&mut u);

            for k in 0..m {
                for (a, zs) in z_step.iter_mut().enumerate() {
                    *zs = inv_norm_cdf(u[a * m + k]);
                }
                chol.apply(&z_step, &mut x_step);
                for (a, &xs) in x_step.iter().enumerate() {
                    z_corr[a * m + k] = xs;
                }
            }

            for (a, wt) in w_terminal.iter_mut().enumerate() {
                let base = a * m;
                bridge.build(&z_corr[base..base + m], &mut path);
                *wt = path[m - 1];
            }

            acc += discounted_payoff(spec, &drift, &w_terminal, df);
            accumulate_pathwise_deltas(spec, &drift, &w_terminal, df, &mut delta_rep_acc);
        }
        rep_means.push(acc / cfg.budget as f64);
        for a in 0..n {
            delta_reps[a].push(delta_rep_acc[a] / cfg.budget as f64);
        }
    }

    let estimate = mean(&rep_means);
    let var = sample_variance(&rep_means, estimate);
    let std_error = sqrt(var / cfg.replications as f64);

    let mut leg_deltas = Vec::with_capacity(n);
    let mut leg_delta_std_errors = Vec::with_capacity(n);
    for rep in delta_reps.iter().take(n) {
        let d_est = mean(rep);
        let d_var = sample_variance(rep, d_est);
        let d_se = sqrt(d_var / cfg.replications as f64);
        leg_deltas.push(d_est);
        leg_delta_std_errors.push(d_se);
    }

    Ok(BasketSensitivities {
        price: estimate,
        price_std_error: std_error,
        leg_deltas,
        leg_delta_std_errors,
    })
}

#[inline]
fn accumulate_pathwise_deltas(
    spec: &BasketSpec,
    drift: &[f64],
    w_terminal: &[f64],
    df: f64,
    delta_acc: &mut [f64],
) {
    match spec.kind {
        BasketKind::Basket => {
            let agg = spec
                .legs
                .iter()
                .zip(drift)
                .zip(w_terminal)
                .map(|((leg, &mu), &w)| leg.weight * terminal_level(leg, mu, w))
                .sum::<f64>();

            let is_itm = match spec.option_type {
                OptionType::Call => agg > spec.strike,
                OptionType::Put => agg < spec.strike,
            };

            if is_itm {
                let sign = match spec.option_type {
                    OptionType::Call => 1.0,
                    OptionType::Put => -1.0,
                };
                let factor = sign * df;
                for (a, (leg, (&mu, &wt))) in
                    spec.legs.iter().zip(drift.iter().zip(w_terminal)).enumerate()
                {
                    let r_a = exp(mu + leg.vol * wt);
                    delta_acc[a] += factor * leg.weight * r_a;
                }
            }
        }
        BasketKind::BestOf => {
            let mut best_idx = 0;
            let mut best_val = f64::NEG_INFINITY;
            for (a, ((leg, &mu), &wt)) in spec.legs.iter().zip(drift).zip(w_terminal).enumerate() {
                let val = leg.weight * terminal_level(leg, mu, wt);
                if val > best_val {
                    best_val = val;
                    best_idx = a;
                }
            }
            let is_itm = match spec.option_type {
                OptionType::Call => best_val > spec.strike,
                OptionType::Put => best_val < spec.strike,
            };
            if is_itm {
                let sign = match spec.option_type {
                    OptionType::Call => 1.0,
                    OptionType::Put => -1.0,
                };
                let leg = &spec.legs[best_idx];
                let r = exp(drift[best_idx] + leg.vol * w_terminal[best_idx]);
                delta_acc[best_idx] += sign * df * leg.weight * r;
            }
        }
        BasketKind::WorstOf => {
            let mut worst_idx = 0;
            let mut worst_val = f64::INFINITY;
            for (a, ((leg, &mu), &wt)) in spec.legs.iter().zip(drift).zip(w_terminal).enumerate() {
                let val = leg.weight * terminal_level(leg, mu, wt);
                if val < worst_val {
                    worst_val = val;
                    worst_idx = a;
                }
            }
            let is_itm = match spec.option_type {
                OptionType::Call => worst_val > spec.strike,
                OptionType::Put => worst_val < spec.strike,
            };
            if is_itm {
                let sign = match spec.option_type {
                    OptionType::Call => 1.0,
                    OptionType::Put => -1.0,
                };
                let leg = &spec.legs[worst_idx];
                let r = exp(drift[worst_idx] + leg.vol * w_terminal[worst_idx]);
                delta_acc[worst_idx] += sign * df * leg.weight * r;
            }
        }
    }
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

    /// The settlement-cash numeraire carry at rate `r`: one unit of settlement
    /// cash has forward 1 (zero net carry) and discounts at `r`.
    fn numeraire(r: f64) -> Carry {
        Carry::CostOfCarry { r, b: 0.0 }
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
        let r_dom = 0.02;
        let spec = BasketSpec {
            legs: vec![
                BasketLeg::new(1.0, 0.1, r_dom - 0.01, 1.0),
                BasketLeg::new(1.2, 0.12, r_dom - 0.015, 1.0),
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
        let a = price_basket(&spec, numeraire(r_dom), 1.0, cfg).unwrap();
        let b = price_basket(&spec, numeraire(r_dom), 1.0, cfg).unwrap();
        assert_eq!(a.price.to_bits(), b.price.to_bits());
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
    }

    #[test]
    fn structural_sandwich_worst_le_best() {
        // worst-of ≤ best-of for a call on identical legs, any correlation.
        let r_dom = 0.02;
        let legs = vec![
            BasketLeg::new(1.0, 0.15, r_dom - 0.01, 1.0),
            BasketLeg::new(1.0, 0.18, r_dom - 0.01, 1.0),
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
        let best = price_basket(&mk(BasketKind::BestOf), numeraire(r_dom), 1.0, cfg)
            .unwrap()
            .price;
        let worst = price_basket(&mk(BasketKind::WorstOf), numeraire(r_dom), 1.0, cfg)
            .unwrap()
            .price;
        assert!(worst <= best, "worst {worst} should be ≤ best {best}");
    }

    #[test]
    fn sensitivities_price_matches_price_basket_bit_identical() {
        let r_dom = 0.02;
        let spec = BasketSpec {
            legs: vec![
                BasketLeg::new(1.0, 0.1, r_dom - 0.01, 1.0),
                BasketLeg::new(1.2, 0.12, r_dom - 0.015, 1.0),
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
        let base = price_basket(&spec, numeraire(r_dom), 1.0, cfg).unwrap();
        let sens = price_basket_with_sensitivities(&spec, numeraire(r_dom), 1.0, cfg).unwrap();
        assert_eq!(base.price.to_bits(), sens.price.to_bits());
        assert_eq!(base.std_error.to_bits(), sens.price_std_error.to_bits());
        assert_eq!(sens.leg_deltas.len(), 2);
        assert_eq!(sens.leg_delta_std_errors.len(), 2);
        assert!(sens.leg_deltas[0] > 0.0, "call delta must be positive");
        assert!(sens.leg_deltas[1] > 0.0, "call delta must be positive");
    }

    #[test]
    fn single_leg_basket_delta_matches_black_scholes() {
        use celnet_core::math::norm_cdf;
        let r_dom = 0.03;
        let r_for = 0.01;
        let spot = 100.0;
        let strike = 100.0;
        let vol = 0.20;
        let t = 1.0;
        let b = r_dom - r_for;

        let spec = BasketSpec {
            legs: vec![BasketLeg::new(spot, vol, b, 1.0)],
            correlation: vec![vec![1.0]],
            option_type: OptionType::Call,
            strike,
            kind: BasketKind::Basket,
        };
        let cfg = BasketMcConfig {
            budget: 8192,
            replications: 16,
            steps: 1,
            seed: 42,
        };
        let sens = price_basket_with_sensitivities(&spec, numeraire(r_dom), t, cfg).unwrap();

        // Analytical Black-Scholes delta: e^{-r_d * t} * e^{b * t} * N(d1)
        let d1 = ((spot / strike).ln() + (b + 0.5 * vol * vol) * t) / (vol * sqrt(t));
        let bs_delta = exp(-r_for * t) * norm_cdf(d1);

        let mc_delta = sens.leg_deltas[0];
        let mc_se = sens.leg_delta_std_errors[0];
        let diff = (mc_delta - bs_delta).abs();
        assert!(
            diff < 3.0 * mc_se,
            "MC delta {mc_delta} vs BS delta {bs_delta} diff {diff} exceeds 3*SE ({mc_se})"
        );
    }

    #[test]
    fn sensitivities_pathwise_delta_matches_finite_difference() {
        let r_dom = 0.02;
        let spec = BasketSpec {
            legs: vec![
                BasketLeg::new(100.0, 0.15, r_dom - 0.01, 0.5),
                BasketLeg::new(100.0, 0.20, r_dom - 0.015, 0.5),
            ],
            correlation: corr2(0.4),
            option_type: OptionType::Call,
            strike: 100.0,
            kind: BasketKind::Basket,
        };
        let cfg = BasketMcConfig {
            budget: 4096,
            replications: 8,
            steps: 1,
            seed: 1234,
        };
        let sens = price_basket_with_sensitivities(&spec, numeraire(r_dom), 1.0, cfg).unwrap();

        // Central finite difference for leg 0:
        let h = 0.01; // 100.0 * 1e-4
        let mut spec_plus = spec.clone();
        spec_plus.legs[0].spot += h;
        let mut spec_minus = spec.clone();
        spec_minus.legs[0].spot -= h;

        let p_plus = price_basket(&spec_plus, numeraire(r_dom), 1.0, cfg).unwrap().price;
        let p_minus = price_basket(&spec_minus, numeraire(r_dom), 1.0, cfg).unwrap().price;
        let fd_delta = (p_plus - p_minus) / (2.0 * h);

        let mc_delta = sens.leg_deltas[0];
        let mc_se = sens.leg_delta_std_errors[0];
        let diff = (mc_delta - fd_delta).abs();
        assert!(
            diff < 3.5 * mc_se,
            "pathwise delta {mc_delta} vs FD delta {fd_delta} diff {diff} exceeds 3.5*SE ({mc_se})"
        );
    }
}
