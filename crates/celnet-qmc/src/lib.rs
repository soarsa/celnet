//! Celnet quasi-Monte-Carlo engine (CPU-first).
//!
//! A complete, state-of-the-art randomized quasi-Monte-Carlo (RQMC) stack:
//!
//! * [`SobolSequence`] — a gray-code Joe-Kuo (2008) Sobol' generator with an
//!   embedded BSD-licensed direction-number table (documented maximum dimension
//!   [`MAX_DIM`]), plus an Owen-style **nested digital scramble** for unbiased
//!   randomized replications.
//! * [`BrownianBridge`] — principal-bisection Brownian-bridge path construction
//!   that loads the dominant path variance onto the best-distributed (first)
//!   Sobol dimensions, reproducing the exact discrete covariance
//!   `Cov(W(t_i),W(t_j)) = min(t_i,t_j)`.
//! * [`inv_norm_cdf`] — a full-precision inverse standard-normal CDF (mapping
//!   the uniform Sobol coordinates to standard normals).
//! * [`rqmc_estimate`] — a convenience randomized-QMC estimator: averages a
//!   path-functional payoff over a Sobol path-budget, repeats over independent
//!   scrambles, and returns both the estimate and its (unbiased) standard error.
//!
//! # Design for GPU reuse (Wave-5)
//!
//! The Sobol core is pure integer arithmetic and the direction numbers are
//! exposed verbatim ([`SobolSequence::direction_numbers`]); a future GPU backend
//! consumes the **same** table and reproduces points/scrambling on-device. No GPU
//! result is claimed in this wave — everything here is in-repo CPU numerics.
//!
//! # Determinism
//!
//! Transcendentals route through [`celnet_core::math`] (the `rust-lang/libm`
//! software backend); the integer Sobol/scramble core is bit-identical across
//! platforms. Given the same `(seed, n, dim)` every result is reproducible.

#![forbid(unsafe_code)]

mod bridge;
mod direction_numbers;
mod normal;
mod sobol;

pub use bridge::BrownianBridge;
pub use direction_numbers::MAX_DIM;
pub use normal::inv_norm_cdf;
pub use sobol::{SobolSequence, SobolStream};

/// Result of a randomized-QMC estimation: the point estimate and its standard
/// error (the standard deviation of the per-scramble means divided by `√R`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RqmcResult {
    /// The estimate (mean of the `R` per-scramble means).
    pub estimate: f64,
    /// Unbiased standard error of [`Self::estimate`] from the between-scramble
    /// variance. With `R` scrambles this is `s / √R` where `s²` is the sample
    /// variance of the per-scramble means.
    pub std_error: f64,
    /// Number of independent scrambled replications used.
    pub replications: usize,
    /// Sobol points per replication.
    pub budget: usize,
}

/// Convenience randomized-QMC estimator over a Brownian-bridged Sobol path.
///
/// For each of `replications` independent scrambles (seeds derived from
/// `base_seed`), it draws `budget` scrambled Sobol points of dimension
/// `bridge.steps()`, maps each to standard normals via [`inv_norm_cdf`], builds
/// the bridged Brownian path with `bridge`, evaluates `payoff(path) -> f64`, and
/// averages. The reported [`RqmcResult::estimate`] is the mean of the per-scramble
/// means and [`RqmcResult::std_error`] is their unbiased standard error — the
/// honest, *measured* error of an unbiased RQMC estimator.
///
/// `payoff` receives the `m`-step Brownian path `W(t_1)..W(t_m)`; the caller maps
/// it to the asset path and discounted payoff.
///
/// # Panics
///
/// Panics if `replications == 0` or `budget == 0`.
pub fn rqmc_estimate<F>(
    bridge: &BrownianBridge,
    budget: usize,
    replications: usize,
    base_seed: u64,
    mut payoff: F,
) -> RqmcResult
where
    F: FnMut(&[f64]) -> f64,
{
    assert!(replications >= 1, "need >= 1 replication");
    assert!(budget >= 1, "need >= 1 point");
    let m = bridge.steps();
    let seq = SobolSequence::new(m);

    let mut z = vec![0.0f64; m];
    let mut path = vec![0.0f64; m];
    let mut u = vec![0.0f64; m];
    let mut rep_means = Vec::with_capacity(replications);

    for r in 0..replications {
        // Derive an independent scramble seed per replication.
        let seed = splitmix(base_seed.wrapping_add((r as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)));
        let mut stream = seq.stream(seed);
        let mut acc = 0.0f64;
        for _ in 0..budget {
            stream.next_point(&mut u);
            for (zi, &ui) in z.iter_mut().zip(u.iter()) {
                *zi = inv_norm_cdf(ui);
            }
            bridge.build(&z, &mut path);
            acc += payoff(&path);
        }
        rep_means.push(acc / budget as f64);
    }

    let estimate = mean(&rep_means);
    let std_error = if replications >= 2 {
        let var = sample_variance(&rep_means, estimate);
        (var / replications as f64).sqrt()
    } else {
        f64::NAN
    };

    RqmcResult {
        estimate,
        std_error,
        replications,
        budget,
    }
}

/// SplitMix64 avalanche (re-exported scrambling helper for derived seeds).
#[inline]
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn sample_variance(xs: &[f64], m: f64) -> f64 {
    let n = xs.len();
    debug_assert!(n >= 2);
    xs.iter().map(|&x| (x - m) * (x - m)).sum::<f64>() / (n - 1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial payoff `W(T)` has mean 0 (driftless Brownian motion), and the
    /// RQMC estimator's std-error must be a positive, finite measured number.
    #[test]
    fn rqmc_driftless_brownian_mean_zero() {
        let bb = BrownianBridge::new(8, 1.0);
        let res = rqmc_estimate(&bb, 1024, 16, 0xC0FFEE, |path| *path.last().unwrap());
        assert!(res.std_error.is_finite() && res.std_error > 0.0);
        // E[W(T)] = 0; within a few standard errors.
        assert!(
            res.estimate.abs() < 5.0 * res.std_error,
            "estimate {} not within 5 SE {}",
            res.estimate,
            res.std_error
        );
    }

    /// Reproducibility: identical seeds ⇒ bit-identical results.
    #[test]
    fn reproducible() {
        let bb = BrownianBridge::new(4, 0.75);
        let f = |p: &[f64]| p.iter().sum::<f64>();
        let a = rqmc_estimate(&bb, 256, 8, 42, f);
        let b = rqmc_estimate(&bb, 256, 8, 42, f);
        assert_eq!(a.estimate.to_bits(), b.estimate.to_bits());
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
    }
}
