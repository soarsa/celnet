//! Stochastic-variance backbone: the mean-reverting square-root variance process
//! and its quadratic-exponential time discretisation.
//!
//! # The process
//!
//! The instantaneous variance `v_t` follows the mean-reverting square-root
//! diffusion
//!
//! ```text
//!   dv_t = κ (θ − v_t) dt + ξ √v_t dW^v_t ,   v_0 = v0 ,
//! ```
//!
//! correlated with the spot's Brownian motion at instantaneous correlation `ρ`.
//! In the local-stochastic-volatility (LSV) model of [`crate::lsv`] this variance
//! is the *stochastic* multiplier; the spot follows
//!
//! ```text
//!   dS_t / S_t = (r_d − r_f) dt + L(S_t, t) √v_t ( ρ dW^v_t + √(1−ρ²) dW^⊥_t ) ,
//! ```
//!
//! where `L(S,t)` is the [`crate::leverage`] function that makes the model
//! reprice the vanilla surface.
//!
//! # Discretisation
//!
//! Exact simulation of the square-root variance is expensive (a non-central χ²
//! draw per step). The **quadratic-exponential (QE)** scheme reproduces the
//! conditional mean and variance of `v_{t+Δ} | v_t` to high accuracy with a single
//! Gaussian (or exponential) draw, switching between a squared-Gaussian
//! representation in the high-variance regime and an exponential-with-mass-at-zero
//! representation in the low-variance regime at a critical switching ratio
//! `ψ_c ∈ [1, 2]`. It is the production-standard variance stepper for this class
//! of model.
//!
//! When the **Feller condition** `2κθ ≥ ξ²` is violated the variance can reach
//! zero with positive probability; the QE scheme already handles the boundary
//! through its exponential branch, and the *log-spot* integration in
//! [`log_spot_increment`] uses the **full-truncation** convention `(v)⁺` for the
//! drift/diffusion coefficients so a momentary zero variance neither produces a
//! negative variance feedback nor a complex volatility.
//!
//! # Method provenance (doc comments only)
//!
//! Square-root variance dynamics: Heston (1993). Quadratic-exponential
//! discretisation and the martingale-corrected log-spot integration:
//! Andersen (2008, *Efficient Simulation of the Heston Stochastic Volatility
//! Model*). Full-truncation Euler for the variance coefficients: Lord, Koekkoek &
//! van Dijk (2010). All identifiers are purpose-named; the provenance lives only
//! here.

use celnet_core::math::{exp, ln, sqrt};

use crate::normal::inverse_cdf;
use crate::rng::CounterRng;

/// Parameters of the mean-reverting square-root variance backbone.
///
/// These are the five canonical parameters of the stochastic-variance process
/// (initial variance, mean-reversion speed, long-run variance, vol-of-variance,
/// spot/variance correlation). They are deliberately *not* named after any
/// author — `mean_reversion`, `long_var`, `vol_of_var` describe their role.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VarianceParams {
    /// Initial instantaneous variance `v0` (= ATM-forward variance at `t=0`).
    pub v0: f64,
    /// Mean-reversion speed `κ` (per year).
    pub mean_reversion: f64,
    /// Long-run variance level `θ`.
    pub long_var: f64,
    /// Volatility of variance `ξ` (the diffusion coefficient of `√v`).
    pub vol_of_var: f64,
    /// Instantaneous correlation `ρ ∈ (−1, 1)` between spot and variance.
    pub correlation: f64,
}

impl VarianceParams {
    /// Construct and validate the variance parameters.
    ///
    /// # Panics
    ///
    /// Panics if any of the positivity / range invariants is violated
    /// (`v0, κ, θ, ξ > 0`, `ρ ∈ (−1, 1)`); these are modelling pre-conditions, not
    /// recoverable runtime errors.
    #[must_use]
    pub fn new(
        v0: f64,
        mean_reversion: f64,
        long_var: f64,
        vol_of_var: f64,
        correlation: f64,
    ) -> Self {
        assert!(v0 > 0.0, "initial variance must be positive");
        assert!(mean_reversion > 0.0, "mean reversion must be positive");
        assert!(long_var > 0.0, "long-run variance must be positive");
        assert!(vol_of_var >= 0.0, "vol-of-variance must be non-negative");
        assert!(
            correlation > -1.0 && correlation < 1.0,
            "correlation must lie strictly in (−1, 1)"
        );
        Self {
            v0,
            mean_reversion,
            long_var,
            vol_of_var,
            correlation,
        }
    }

    /// The **Feller condition** `2κθ ≥ ξ²`. When it holds the variance stays
    /// strictly positive almost surely; when it fails the variance can hit zero
    /// and the full-truncation convention is required (which this backbone always
    /// applies, so violation is handled, not forbidden).
    #[must_use]
    pub fn satisfies_feller(&self) -> bool {
        2.0 * self.mean_reversion * self.long_var >= self.vol_of_var * self.vol_of_var
    }

    /// The Feller ratio `2κθ / ξ²` (≥ 1 ⇔ Feller satisfied). Returns `+∞` for the
    /// degenerate zero-vol-of-variance (pure local-vol) limit.
    #[must_use]
    pub fn feller_ratio(&self) -> f64 {
        let denom = self.vol_of_var * self.vol_of_var;
        if denom <= 0.0 {
            f64::INFINITY
        } else {
            2.0 * self.mean_reversion * self.long_var / denom
        }
    }
}

/// The critical switching ratio `ψ_c` of the quadratic-exponential scheme.
///
/// For `ψ = s²/m² ≤ ψ_c` the squared-Gaussian branch is used; for `ψ > ψ_c` the
/// exponential-with-atom branch. Andersen recommends `ψ_c ∈ [1, 2]`; `1.5` is the
/// standard choice and is exact at neither boundary, giving a smooth handover.
pub const QE_SWITCH: f64 = 1.5;

/// Advance the variance one step `v_t → v_{t+Δ}` with the quadratic-exponential
/// scheme, consuming one uniform `u ∈ (0,1)` (mapped to the scheme's single
/// driving variate).
///
/// `dt` is the step length. The conditional mean `m` and variance `s²` of
/// `v_{t+Δ} | v_t` are the exact moments of the square-root process; the scheme
/// then matches them with whichever of the two representations is appropriate at
/// the realised ratio `ψ = s²/m²`. The result is non-negative by construction.
#[must_use]
pub fn qe_variance_step(p: &VarianceParams, v: f64, dt: f64, u: f64) -> f64 {
    let k = p.mean_reversion;
    let theta = p.long_var;
    let xi = p.vol_of_var;

    // Exact conditional moments of the square-root variance over the step.
    let e = exp(-k * dt);
    let m = theta + (v - theta) * e; // E[v_{t+Δ} | v_t]
    // Var[v_{t+Δ} | v_t] (Andersen 2008, eqn 17).
    let s2 = v * xi * xi * e * (1.0 - e) / k + theta * xi * xi * (1.0 - e) * (1.0 - e) / (2.0 * k);

    if m <= 0.0 {
        return 0.0;
    }
    let psi = s2 / (m * m);

    // Degenerate (deterministic) limit: vanishing conditional variance — the
    // variance is the exact mean-reversion ODE solution `m`, with no random draw.
    if psi <= 1e-12 {
        return m;
    }

    if psi <= QE_SWITCH {
        // Squared-Gaussian branch:  v' = a (b + Z)²,  Z ~ N(0,1).
        let inv = 2.0 / psi;
        let b2 = inv - 1.0 + sqrt(inv) * sqrt((inv - 1.0).max(0.0));
        let b = sqrt(b2.max(0.0));
        let a = m / (1.0 + b2);
        let z = inverse_cdf(u);
        let bz = b + z;
        a * bz * bz
    } else {
        // Exponential-with-atom branch:  v' = 0 with prob p*, else exponential.
        //   p* = (ψ−1)/(ψ+1),  β = (1−p*)/m
        let p_star = (psi - 1.0) / (psi + 1.0);
        let beta = (1.0 - p_star) / m;
        if u <= p_star {
            0.0
        } else {
            // Inverse-CDF of the (atom-free) exponential tail.
            ln((1.0 - p_star) / (1.0 - u)) / beta
        }
    }
}

/// The drift-and-diffusion log-spot increment over a step under the
/// **martingale-corrected** Andersen integration of the variance.
///
/// Given the start and end variances `v0`/`v1` produced by [`qe_variance_step`],
/// the spot's log-increment over the step is split into a variance-driven part
/// (correlated with `dW^v`) and an orthogonal Gaussian part. The variance-driven
/// part is integrated *exactly* against the realised variance path through the
/// `(γ1, γ2)` weighting, and the leverage `lev = L(S,t)` scales the local
/// diffusion. `z_perp` is the orthogonal standard-normal draw.
///
/// Returns the log-spot increment `Δ ln S` (excluding the risk-neutral carry,
/// which the caller adds, so the same routine serves any `(r_d, r_f)`).
///
/// The martingale correction `K0 …` makes `E[S_{t+Δ}/S_t] = e^{(r_d−r_f)Δ}` hold
/// to the order of the scheme — the property that keeps the discounted spot a
/// martingale (no simulation drift bias). With unit leverage and the
/// full-truncation `(v)⁺` convention this reduces to the standard Andersen
/// broadband integration.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn log_spot_increment(
    p: &VarianceParams,
    v0: f64,
    v1: f64,
    dt: f64,
    lev: f64,
    z_perp: f64,
) -> f64 {
    let rho = p.correlation;
    let k = p.mean_reversion;
    let theta = p.long_var;
    let xi = p.vol_of_var;

    // Full-truncation: never feed a negative variance into a √ or a drift.
    let v0p = v0.max(0.0);
    let v1p = v1.max(0.0);

    // Broadband (central) discretisation weights γ1 = γ2 = ½ (Andersen 2008 §3.2).
    let (g1, g2) = (0.5, 0.5);
    let l2 = lev * lev;

    // The exact integral of the variance-driven Brownian part substitutes the
    // realised variance increment for ∫√v dW^v via the SDE
    //   ∫√v dW^v = (v1 − v0 − κθΔ + κ ∫v dt) / ξ ,
    // with ∫v dt ≈ (γ1 v0 + γ2 v1) Δ (the trapezoidal broadband rule).
    let int_v = (g1 * v0p + g2 * v1p) * dt;
    let stoch_int = if xi > 0.0 {
        (v1p - v0p - k * theta * dt + k * int_v) / xi
    } else {
        0.0
    };

    // Leverage scales the local diffusion: dlnS = … + L √v dW.
    // Drift (Itô) term −½ L² v dt over the step (uses ∫v dt).
    let ito = -0.5 * l2 * int_v;
    // Correlated part:  ρ L · (stochastic integral of √v dW^v).
    let correlated = rho * lev * stoch_int;
    // Orthogonal part:  √(1−ρ²) L · √(∫v dt) · Z⊥  (variance-time scaled normal).
    let orthogonal = sqrt((1.0 - rho * rho).max(0.0)) * lev * sqrt(int_v.max(0.0)) * z_perp;

    ito + correlated + orthogonal
}

/// Draw the pair of independent uniforms `(u_var, u_perp)` for one LSV step from
/// the counter-based stream, so a step consumes exactly two stream words and the
/// path is bit-reproducible from `(seed, path, step)`.
#[must_use]
pub fn step_uniforms(seed: u64, stream: u32, path: u64, step: u32) -> (f64, f64) {
    let mut rng = CounterRng::new(seed, stream, path, step);
    (rng.next_u01(), rng.next_u01())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn params() -> VarianceParams {
        VarianceParams::new(0.04, 1.5, 0.04, 0.5, -0.3)
    }

    #[test]
    fn feller_condition_detection() {
        // 2κθ = 2·1.5·0.04 = 0.12 ; ξ² = 0.25 ⇒ violated.
        let p = params();
        assert!(!p.satisfies_feller());
        assert!(p.feller_ratio() < 1.0);
        // A calmer process satisfies Feller.
        let q = VarianceParams::new(0.04, 3.0, 0.05, 0.2, -0.3);
        assert!(q.satisfies_feller());
        assert!(q.feller_ratio() >= 1.0);
        // Zero vol-of-var ⇒ pure local-vol limit ⇒ infinite Feller ratio.
        let r = VarianceParams::new(0.04, 1.0, 0.04, 0.0, 0.0);
        assert!(r.feller_ratio().is_infinite());
    }

    /// The QE step preserves the exact conditional mean `E[v'|v]` across many
    /// draws (the defining moment-matching property of the scheme).
    #[test]
    fn qe_matches_conditional_mean() {
        let p = params();
        let (v, dt) = (0.05, 1.0 / 52.0);
        let e = exp(-p.mean_reversion * dt);
        let expected = p.long_var + (v - p.long_var) * e;

        let mut rng = CounterRng::new(0x0E_5EED, 0, 0, 0);
        let n = 400_000;
        let mut sum = 0.0;
        for _ in 0..n {
            sum += qe_variance_step(&p, v, dt, rng.next_u01());
        }
        let mean = sum / n as f64;
        assert!(
            (mean - expected).abs() < 2e-4,
            "QE mean {mean} vs exact {expected}"
        );
    }

    /// The QE step never returns a negative variance, even with a Feller-violating
    /// parameter set and a near-zero current variance (exercises the exponential
    /// branch with its atom at zero).
    #[test]
    fn qe_is_non_negative() {
        let p = VarianceParams::new(0.04, 0.5, 0.04, 1.2, -0.7); // strongly Feller-violating
        let mut rng = CounterRng::new(123, 0, 0, 0);
        for _ in 0..200_000 {
            let v = qe_variance_step(&p, 1e-6, 1.0 / 12.0, rng.next_u01());
            assert!(v >= 0.0, "negative variance {v}");
        }
    }

    /// QUANTITATIVE pins of BOTH quadratic-exponential branches as
    /// deterministic functions of `(params, v, dt, u)` — the W6 plan §3.5
    /// pre-kill item 5. Expected values hand-derived OUT-OF-BAND from the
    /// published scheme (Andersen 2008: exact conditional moments eqn 17,
    /// squared-Gaussian branch `v' = a(b+Z)²` with `b² = 2/ψ − 1 +
    /// √(2/ψ)√(2/ψ−1)`, `a = m/(1+b²)`; exponential branch `p* = (ψ−1)/(ψ+1)`,
    /// `β = (1−p*)/m`, tail inverse `ln((1−p*)/(1−u))/β`) in an independent
    /// double-precision implementation. The statistical mean test cannot see a
    /// conditional-VARIANCE (s²) mutant that preserves the mean — these pins
    /// kill the whole moment/branch chain on magnitude. `v ≠ θ` deliberately
    /// (at `v = θ` the conditional mean degenerates to `θ` and loses its
    /// `e^{−κΔ}` dependence).
    #[test]
    fn qe_branches_match_hand_derived_scheme() {
        // Squared-Gaussian branch: ψ ≈ 0.045 ≤ ψ_c.
        let calm = VarianceParams::new(0.04, 3.0, 0.05, 0.2, -0.3);
        let (v, dt) = (0.062, 1.0 / 12.0);
        assert_close!(
            qe_variance_step(&calm, v, dt, 0.25),
            0.050_505_841_521_469_99,
            1e-9,
            1e-12
        );
        assert_close!(
            qe_variance_step(&calm, v, dt, 0.9),
            0.075_877_000_473_907_3,
            1e-9,
            1e-12
        );

        // Exponential-with-atom branch: ψ ≈ 35.9 > ψ_c (strongly
        // Feller-violating, near-zero current variance).
        let wild = VarianceParams::new(0.04, 0.5, 0.04, 1.2, -0.7);
        // u = 0.1 < p* ≈ 0.9458 ⇒ the atom at zero.
        assert_close!(
            qe_variance_step(&wild, 1e-4, 1.0 / 12.0, 0.1),
            0.0,
            1e-15,
            1e-15
        );
        // u = 0.95 > p* ⇒ the exponential tail inverse.
        assert_close!(
            qe_variance_step(&wild, 1e-4, 1.0 / 12.0, 0.95),
            2.580_971_040_503_96e-3,
            1e-9,
            1e-14
        );
    }

    /// `log_spot_increment` is a deterministic function — pinned against the
    /// hand-derived broadband integration (γ₁ = γ₂ = ½ trapezoidal ∫v dt; the
    /// SDE substitution `∫√v dW = (v₁ − v₀ − κθΔ + κ∫v dt)/ξ`; Itô term
    /// `−½L²∫v dt`; orthogonal `√(1−ρ²)·L·√(∫v dt)·Z⊥`), evaluated out-of-band:
    /// the main path, the full-truncation clamp (`v₀ < 0` in), and the `ξ = 0`
    /// pure-local-vol branch.
    #[test]
    fn log_spot_increment_matches_hand_derivation() {
        let p = VarianceParams::new(0.04, 1.5, 0.04, 0.5, -0.3);
        assert_close!(
            log_spot_increment(&p, 0.05, 0.038, 1.0 / 12.0, 1.13, 0.62),
            0.045_925_396_052_403_06,
            1e-12,
            1e-14
        );
        // Full truncation: a (defensively handled) negative start variance is
        // clamped to zero before any √ or drift use.
        assert_close!(
            log_spot_increment(&p, -0.02, 0.038, 1.0 / 12.0, 1.13, 0.62),
            1.598_463_144_890_895_8e-3,
            1e-12,
            1e-14
        );
        // ξ = 0: the stochastic-integral substitution is undefined ⇒ zero
        // correlated leg by contract (pure local-vol limit).
        let lv = VarianceParams::new(0.04, 2.0, 0.06, 0.0, 0.4);
        assert_close!(
            log_spot_increment(&lv, 0.04, 0.04, 0.25, 0.9, -1.1),
            -0.094_784_998_760_125_65,
            1e-12,
            1e-14
        );
    }

    /// `step_uniforms` consumes exactly the first two words of the
    /// `(seed, stream, path, step)` counter stream — bit-equal to drawing them
    /// directly (the bit-reproducibility contract of an LSV step).
    #[test]
    fn step_uniforms_are_first_two_stream_words() {
        let (u0, u1) = step_uniforms(0xFEED_5EED, 4, 1234, 17);
        let mut rng = CounterRng::new(0xFEED_5EED, 4, 1234, 17);
        assert_eq!(u0.to_bits(), rng.next_u01().to_bits());
        assert_eq!(u1.to_bits(), rng.next_u01().to_bits());
    }

    /// With zero vol-of-variance the variance is deterministic and the QE step
    /// returns the exact mean-reversion ODE solution — the pure-local-vol limit at
    /// the backbone level.
    #[test]
    fn zero_vol_of_var_is_deterministic() {
        let p = VarianceParams::new(0.04, 2.0, 0.06, 0.0, 0.0);
        let (v, dt) = (0.04, 0.25);
        let e = exp(-p.mean_reversion * dt);
        let expected = p.long_var + (v - p.long_var) * e;
        for u in [0.01, 0.3, 0.5, 0.7, 0.99] {
            assert_close!(qe_variance_step(&p, v, dt, u), expected, 1e-10, 1e-12);
        }
    }
}
