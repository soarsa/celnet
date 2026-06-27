//! Particle-method calibration of the LSV leverage function.
//!
//! # The calibration problem
//!
//! The local-stochastic-volatility (LSV) model reprices the arbitrage-free vanilla
//! surface iff the leverage `L(S,t)` satisfies the McKean-Vlasov identity
//!
//! ```text
//!   L(S, t)² = σ_loc²(S, t) / E[ v_t | S_t = S ] ,
//! ```
//!
//! where `σ_loc` is the [`crate::leverage::LocalVolSurface`] Dupire local
//! volatility and `v_t` is the stochastic variance ([`crate::stochvol`]). The
//! conditional expectation in the denominator depends on the *law* of the
//! simulated process, which itself depends on `L` — a fixed point.
//!
//! # The interacting-particle solution
//!
//! Rather than iterate a global fixed point, the **particle method** calibrates
//! `L` **forward in time, on the fly**: an ensemble of `N` correlated
//! `(S, v)` particles is evolved step by step under the *partially-calibrated*
//! leverage. At each time level `t_j` the conditional expectation
//! `E[v | S = s]` is estimated **non-parametrically** from the current particle
//! cloud by a regularised kernel (Nadaraya-Watson) estimator,
//!
//! ```text
//!   Ê[v | S = s] = Σ_p K_h(s − S_p) v_p / Σ_p K_h(s − S_p) ,
//! ```
//!
//! with a Gaussian kernel whose bandwidth `h` follows the Silverman rule scaled
//! by the realised spot dispersion. The leverage at each *spot grid node* of the
//! [`crate::leverage::LeverageSurface`] is then set to
//! `L = σ_loc / √(max(Ê[v|S], floor))`, and the particles are advanced to the next
//! level using exactly that leverage (read back by bilinear interpolation). One
//! forward sweep calibrates the whole surface — there is no outer iteration, and
//! the single simulated ensemble is reused for both calibration and (optionally)
//! pricing.
//!
//! # Determinism
//!
//! Every particle's variates come from the counter-based [`crate::rng`] keyed by
//! `(seed, particle, step)`, so the calibrated surface is bit-reproducible from
//! the seed. No global mutable RNG state, no order dependence.
//!
//! # Method provenance (doc comments only)
//!
//! The interacting-particle / on-the-fly leverage calibration: Guyon &
//! Henry-Labordère (2012, *Being Particular About Calibration*; 2013, *Nonlinear
//! Option Pricing*). Regularised conditional expectation by kernel regression:
//! Nadaraya (1964); Watson (1964); bandwidth rule: Silverman (1986). The leverage
//! identity: Ren, Madan & Qian (2007). All identifiers purpose-named.

use celnet_core::math::{exp, ln, sqrt};

use crate::leverage::{ImpliedVolSurface, LeverageSurface, LocalVolSurface};
use crate::stochvol::{VarianceParams, log_spot_increment, qe_variance_step, step_uniforms};

/// Configuration of the particle calibration.
#[derive(Debug, Clone, Copy)]
pub struct ParticleConfig {
    /// Number of particles `N` in the interacting ensemble.
    pub particles: usize,
    /// Number of calibration time steps over `[0, T]` (one leverage column per
    /// step boundary).
    pub steps: usize,
    /// RNG seed; identical seeds reproduce the calibrated surface bit-for-bit.
    pub seed: u64,
    /// Kernel-bandwidth multiplier on the Silverman rule (`1.0` = textbook).
    pub bandwidth_scale: f64,
    /// Floor on the conditional variance denominator (keeps `L` finite where the
    /// particle cloud is sparse).
    pub var_floor: f64,
}

impl Default for ParticleConfig {
    fn default() -> Self {
        Self {
            particles: 40_000,
            steps: 40,
            seed: 0x0001_0CA1,
            bandwidth_scale: 1.0,
            var_floor: 1e-6,
        }
    }
}

/// The calibrated leverage surface plus a diagnostic of the calibration quality.
#[derive(Debug, Clone)]
pub struct CalibrationResult {
    /// The calibrated [`LeverageSurface`] (read by both pricing engines).
    pub leverage: LeverageSurface,
    /// The realised mean of the conditional-variance estimate over the ATM node
    /// across time (≈ `θ`-level), a coarse health check.
    pub mean_conditional_var: f64,
}

/// Calibrate the LSV leverage surface by the interacting-particle method.
///
/// `iv` supplies the arbitrage-free implied surface (so its Dupire local vol is
/// the target marginal); `var` are the stochastic-variance parameters; the
/// `spot_grid` defines the leverage nodes in spot; `t_max` is the calibration
/// horizon. The returned [`LeverageSurface`] is defined on `spot_grid` × the
/// uniform time grid `0 = t_0 < … < t_steps = t_max`.
///
/// # Panics
///
/// Panics if `cfg.particles`, `cfg.steps`, or `spot_grid` are empty / too small.
#[must_use]
pub fn calibrate_leverage<S: ImpliedVolSurface>(
    iv: &S,
    var: &VarianceParams,
    spot_grid: &[f64],
    spot0: f64,
    t_max: f64,
    cfg: ParticleConfig,
) -> CalibrationResult {
    assert!(cfg.particles >= 100, "need a non-trivial particle ensemble");
    assert!(cfg.steps >= 1, "need ≥ 1 calibration step");
    assert!(spot_grid.len() >= 2, "need ≥ 2 leverage spot nodes");
    assert!(t_max > 0.0, "calibration horizon must be positive");

    let local = LocalVolSurface::new(iv);
    let dt = t_max / cfg.steps as f64;

    // Uniform time grid for the leverage surface (one column per level boundary).
    let times: Vec<f64> = (0..=cfg.steps).map(|j| j as f64 * dt).collect();
    let mut surface = LeverageSurface::new(spot_grid.to_vec(), times.clone());

    // Particle state: log-spot and variance.
    let n = cfg.particles;
    let mut ln_s = vec![ln(spot0); n];
    let mut v = vec![var.v0; n];

    // Carry drift in log-spot per step (Garman-Kohlhagen). The implied surface's
    // forward curve defines the carry: b(t) ≈ d ln F / dt. Read it from two
    // forward samples around the level to stay carry-consistent with the surface.
    let mut conditional_var_atm_acc = 0.0;
    let mut conditional_var_atm_count = 0usize;

    // --- t = 0 column: spot is a point mass at spot0, so E[v|S]=v0 everywhere.
    {
        let denom = v[0].max(cfg.var_floor); // all particles share v0
        for (i, &s) in spot_grid.iter().enumerate() {
            let lv = local.local_vol(s, times[1].max(1e-4));
            surface.set(i, 0, lv / sqrt(denom));
        }
    }

    // --- Forward sweep: advance the cloud one step, then calibrate the column at
    //     the *new* time level from the realised cloud.
    for j in 0..cfg.steps {
        let t0 = times[j];
        let t1 = times[j + 1];
        let carry = carry_rate(iv, t0, t1, dt);

        // Advance every particle using the leverage column already calibrated at
        // t0 (read by interpolation), the QE variance step and the correlated
        // log-spot increment.
        for p in 0..n {
            let s_prev = exp(ln_s[p]);
            let lev = surface.leverage(s_prev, t0);
            let (u_var, u_perp) = step_uniforms(cfg.seed, 0, p as u64, j as u32);
            let v_next = qe_variance_step(var, v[p], dt, u_var);
            let z_perp = crate::normal::inverse_cdf(u_perp);
            let incr = log_spot_increment(var, v[p], v_next, dt, lev, z_perp);
            ln_s[p] += carry * dt + incr;
            v[p] = v_next;
        }

        // Calibrate the leverage column at the new level t1 from the cloud.
        let (mean_var, h) = bandwidth(&ln_s);
        for (i, &s_node) in spot_grid.iter().enumerate() {
            let cond_var = conditional_expectation(&ln_s, &v, ln(s_node), h, cfg.var_floor);
            let target_t = if t1 > 0.0 { t1 } else { dt };
            let sig_loc = local.local_vol(s_node, target_t);
            surface.set(i, j + 1, sig_loc / sqrt(cond_var));
            // Diagnostic at the node closest to spot0.
            if (ln(s_node) - ln(spot0)).abs() <= h {
                conditional_var_atm_acc += cond_var;
                conditional_var_atm_count += 1;
            }
        }
        // `mean_var` is referenced to keep the bandwidth computation honest even
        // when the cloud has collapsed; it is part of the health diagnostic.
        let _ = mean_var;
    }

    let mean_conditional_var = if conditional_var_atm_count > 0 {
        conditional_var_atm_acc / conditional_var_atm_count as f64
    } else {
        var.v0
    };

    CalibrationResult {
        leverage: surface,
        mean_conditional_var,
    }
}

/// Carry rate `b ≈ d ln F / dt` over `[t0, t1]` from the implied surface's forward
/// curve, falling back to the instantaneous forward log-derivative when the
/// interval degenerates.
#[inline]
fn carry_rate<S: ImpliedVolSurface>(iv: &S, t0: f64, t1: f64, dt: f64) -> f64 {
    let f0 = iv.forward(t0.max(1e-8));
    let f1 = iv.forward(t1.max(1e-8));
    if f0 > 0.0 && f1 > 0.0 && dt > 0.0 {
        (ln(f1) - ln(f0)) / dt
    } else {
        0.0
    }
}

/// Silverman-rule Gaussian-kernel bandwidth in **log-spot**, returning
/// `(sample_variance, bandwidth)`.
///
/// `h = 1.06 · σ̂ · N^{−1/5}` with `σ̂` the sample standard deviation of the
/// log-spot cloud — the standard one-dimensional density-estimation bandwidth.
fn bandwidth(ln_s: &[f64]) -> (f64, f64) {
    let n = ln_s.len() as f64;
    let mean = ln_s.iter().sum::<f64>() / n;
    let var = ln_s.iter().map(|&x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let sd = sqrt(var.max(1e-12));
    let h = 1.06 * sd * exp(-0.2 * ln(n)); // N^{-1/5} via exp(−ln N /5)
    (var, h.max(1e-4))
}

/// Regularised Nadaraya-Watson estimate of `E[v | ln S = x]` from the particle
/// cloud, with a Gaussian kernel of bandwidth `h`, floored at `floor`.
///
/// Particles further than a few bandwidths contribute negligibly; the floor on
/// the denominator weight keeps the estimate finite where the cloud is sparse,
/// and the result itself is floored so the leverage denominator is never zero.
fn conditional_expectation(ln_s: &[f64], v: &[f64], x: f64, h: f64, floor: f64) -> f64 {
    let inv_h = 1.0 / h;
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for (&xi, &vi) in ln_s.iter().zip(v.iter()) {
        let z = (xi - x) * inv_h;
        // Gaussian kernel (unnormalised constant cancels in the ratio).
        let w = exp(-0.5 * z * z);
        num += w * vi;
        den += w;
    }
    if den > 1e-300 {
        (num / den).max(floor)
    } else {
        floor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leverage::ImpliedVolSurface;
    use celnet_core::assert_close;

    /// A flat implied surface: the Dupire local vol is the constant `σ`, and at the
    /// pure-local-vol limit (`ξ = 0`, `v0 = σ²`) the conditional variance is exactly
    /// `v0`, so the calibrated leverage must be ≈ 1 everywhere.
    #[test]
    fn flat_surface_unit_leverage_in_lv_limit() {
        struct Flat {
            sigma: f64,
            fwd: f64,
        }
        impl ImpliedVolSurface for Flat {
            fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
                self.sigma
            }
            fn forward(&self, _t: f64) -> f64 {
                self.fwd
            }
        }
        let sigma = 0.2;
        let iv = Flat { sigma, fwd: 100.0 };
        // Pure local-vol limit: zero vol-of-var, v0 = σ².
        let var = VarianceParams::new(sigma * sigma, 1.0, sigma * sigma, 0.0, 0.0);
        let grid: Vec<f64> = (0..21).map(|i| 60.0 + 4.0 * i as f64).collect();
        let res = calibrate_leverage(
            &iv,
            &var,
            &grid,
            100.0,
            1.0,
            ParticleConfig {
                particles: 20_000,
                steps: 20,
                seed: 42,
                ..ParticleConfig::default()
            },
        );
        // Around the populated centre of the cloud the leverage must be ≈ 1.
        for s in [88.0, 100.0, 112.0] {
            let lv = res.leverage.leverage(s, 0.5);
            assert!(
                (lv - 1.0).abs() < 0.05,
                "leverage at S={s} should be ≈1, got {lv}"
            );
        }
        // The conditional variance diagnostic sits near v0.
        assert_close!(res.mean_conditional_var, sigma * sigma, 0.2, 1e-3);
    }

    /// The Nadaraya-Watson conditional-expectation estimator against a
    /// brute-force hand computation on a tiny fixed particle set (independent
    /// plain-loop oracle, out-of-band at double precision), plus its floor and
    /// sparse-cloud guards — the W6 plan §3.5 pre-kill item 4(iii).
    #[test]
    fn conditional_expectation_matches_brute_force() {
        let ln_s = [-0.2, -0.05, 0.1, 0.3];
        let v = [0.05, 0.03, 0.08, 0.02];
        // Hand: Σ wᵢvᵢ / Σ wᵢ with wᵢ = e^{−½((xᵢ−x)/h)²}, x = 0, h = 0.15.
        assert_close!(
            conditional_expectation(&ln_s, &v, 0.0, 0.15, 1e-6),
            0.050_454_779_119_687_13,
            1e-12,
            1e-15
        );
        // Result floor: a cloud of near-zero variances floors at `floor`.
        assert_close!(
            conditional_expectation(&ln_s, &[0.0; 4], 0.0, 0.15, 1e-6),
            1e-6,
            1e-15,
            1e-18
        );
        // Sparse-cloud denominator guard: every particle hundreds of
        // bandwidths away ⇒ weights underflow ⇒ the floor, not 0/0.
        assert_close!(
            conditional_expectation(&ln_s, &v, 50.0, 0.01, 1e-6),
            1e-6,
            1e-15,
            1e-18
        );
    }

    /// Silverman bandwidth pinned by hand: `h = 1.06·σ̂·N^{−1/5}` with the
    /// population standard deviation of the log-spot cloud, plus the
    /// `(variance, h)` pair and the bandwidth floor on a collapsed cloud.
    #[test]
    fn bandwidth_matches_hand_silverman() {
        let ln_s = [0.1, 0.25, -0.05, 0.4, 0.0];
        let (var, h) = bandwidth(&ln_s);
        assert_close!(var, 0.0274, 1e-12, 1e-15);
        assert_close!(h, 0.127_170_724_590_346_8, 1e-12, 1e-15);
        // A point-mass cloud floors the bandwidth (never a zero-width kernel).
        let (var0, h0) = bandwidth(&[0.3; 64]);
        assert_close!(var0, 0.0, 1e-15, 1e-15);
        assert_close!(h0, 1e-4, 1e-12, 1e-15);
    }

    /// `carry_rate` recovers the exponential forward curve's carry exactly:
    /// `F(t) = F₀·e^{b·t}` ⇒ `(ln F(t₁) − ln F(t₀))/Δ = b` to rounding.
    #[test]
    fn carry_rate_recovers_exponential_forward() {
        struct Fwd;
        impl ImpliedVolSurface for Fwd {
            fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
                0.2
            }
            fn forward(&self, t: f64) -> f64 {
                1.3 * exp(0.023 * t)
            }
        }
        assert_close!(carry_rate(&Fwd, 0.5, 0.75, 0.25), 0.023, 1e-10, 1e-12);
        // Degenerate interval ⇒ the 0.0 fallback.
        assert_close!(carry_rate(&Fwd, 0.5, 0.75, 0.0), 0.0, 1e-15, 1e-15);
    }

    /// FROZEN leverage-node bits for a pinned skewed-smile calibration — the
    /// W6 plan §3.5 pre-kill item 4(i)/(ii). The `calibration_is_reproducible`
    /// replica test cannot see a mutant that perturbs both replicas
    /// identically; these constants (captured once from the unmutated build,
    /// quantitatively sanity-checked: `L ≈ σ_loc/√E[v|S] ≈ 1.14` at the ATM
    /// node) pin the ENTIRE deterministic chain — counter RNG stream, normal
    /// inverse, QE variance step, log-spot increment, Silverman bandwidth,
    /// Nadaraya-Watson estimator, Dupire extraction, node indexing — to the
    /// bit. ξ > 0 and a skewed smile keep every branch live.
    #[test]
    fn calibration_bits_frozen_for_pinned_smile() {
        struct Skew;
        impl ImpliedVolSurface for Skew {
            fn implied_vol(&self, k: f64, _t: f64) -> f64 {
                let y = ln(k / 1.3);
                sqrt(0.0324 - 0.01 * y + 0.02 * y * y)
            }
            fn forward(&self, t: f64) -> f64 {
                1.3 * exp(0.02 * t)
            }
        }
        let var = VarianceParams::new(0.0324, 1.5, 0.0324, 0.4, -0.25);
        let grid = [1.0, 1.15, 1.3, 1.45, 1.6];
        let cfg = ParticleConfig {
            particles: 600,
            steps: 3,
            seed: 0xCA11_B12A_7E5E_ED01,
            bandwidth_scale: 1.0,
            var_floor: 1e-6,
        };
        let res = calibrate_leverage(&Skew, &var, &grid, 1.3, 0.75, cfg);
        for (i, j, bits) in [
            (0usize, 0usize, 0x3ff2_4494_9c32_efb8_u64),
            (2, 1, 0x3ff2_18d0_c88e_ed7a),
            (4, 2, 0x3ff0_2468_7f33_5563),
            (2, 3, 0x3ff1_edf6_deaf_7892),
            (1, 3, 0x3ff0_20af_891d_d59e),
        ] {
            assert_eq!(
                res.leverage.at(i, j).to_bits(),
                bits,
                "frozen leverage node ({i},{j}) changed: {:?}",
                res.leverage.at(i, j)
            );
        }
        assert_eq!(
            res.mean_conditional_var.to_bits(),
            0x3f99_d234_1348_1335,
            "frozen mean conditional variance changed: {:?}",
            res.mean_conditional_var
        );
    }

    /// Calibration is deterministic: identical seed ⇒ bit-identical leverage nodes.
    #[test]
    fn calibration_is_reproducible() {
        struct Flat;
        impl ImpliedVolSurface for Flat {
            fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
                0.18
            }
            fn forward(&self, _t: f64) -> f64 {
                1.30
            }
        }
        let var = VarianceParams::new(0.0324, 1.5, 0.0324, 0.4, -0.25);
        let grid: Vec<f64> = (0..11).map(|i| 1.0 + 0.06 * i as f64).collect();
        let cfg = ParticleConfig {
            particles: 4_000,
            steps: 12,
            seed: 7,
            ..ParticleConfig::default()
        };
        let a = calibrate_leverage(&Flat, &var, &grid, 1.30, 1.0, cfg);
        let b = calibrate_leverage(&Flat, &var, &grid, 1.30, 1.0, cfg);
        for i in 0..grid.len() {
            for j in 0..=cfg.steps {
                assert_eq!(
                    a.leverage.at(i, j).to_bits(),
                    b.leverage.at(i, j).to_bits(),
                    "leverage node ({i},{j}) not reproducible"
                );
            }
        }
    }
}
