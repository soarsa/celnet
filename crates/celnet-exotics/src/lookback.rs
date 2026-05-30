//! Lookback options — fixed-strike and floating-strike — on the running extremum
//! of the Garman-Kohlhagen spot path.
//!
//! A **floating-strike** lookback lets the holder buy at the path minimum (call)
//! or sell at the path maximum (put): the payoffs are `S_T − m` and `M − S_T`
//! respectively, where `m = min_t S_t` and `M = max_t S_t`. A **fixed-strike**
//! lookback pays off the *optimal* exercise against a fixed strike: `(M − K)⁺`
//! (call) and `(K − m)⁺` (put). Both are continuously-monitored on the spot.
//!
//! # Closed form (continuous monitoring)
//!
//! For continuous monitoring under geometric Brownian motion with carry
//! `b = r_d − r_f`, volatility `σ` and `b ≠ 0`, the lookback values are known in
//! closed form (Goldman-Sosin-Gatto for the floating strike, Conze-Viswanathan
//! for the fixed strike). Writing `a₁ = [ln(S/ξ) + (b + ½σ²)T]/(σ√T)`,
//! `a₂ = a₁ − σ√T` and `Y = −2(b − ½σ²)/σ²` for the running extremum `ξ` (the
//! observed min for a floating call / observed max for a floating put), the
//! present values follow the standard reflection terms implemented in
//! [`floating_lookback_price`] and [`fixed_lookback_price`]. These exact values
//! are the oracle the Monte-Carlo engine here is cross-validated against.
//!
//! # Monte-Carlo with the Brownian-bridge extremum correction
//!
//! A discretely-stepped path **under-samples** the continuous extremum (the true
//! min/max almost surely lies beyond the sampled nodes). Between two simulated
//! log-spots `x_{k}`, `x_{k+1}` the conditional law of the bridge extremum is
//! known: the continuous minimum of the interval has expectation obtained from the
//! reflected bridge, and a single exact draw of the bridge extremum per step
//! removes the `O(1/√steps)` discretisation bias. The engine samples that bridge
//! extremum directly from a per-step uniform — the standard "Brownian-bridge
//! extremum simulation" — so the discrete MC converges to the *continuous*-
//! monitoring closed form, which the tests assert.
//!
//! Provenance (doc-only): Goldman, Sosin & Gatto (1979); Conze & Viswanathan
//! (1991); the generalised-BSM presentation in Haug (2007); the Brownian-bridge
//! extremum simulation of Beaglehole-Dybvig-Zhou (1997) / Glasserman (2003,
//! §6.4). Identifiers are purpose-named and vendor/research-neutral.

use celnet_core::math::{exp, ln, norm_cdf, sqrt};
use celnet_types::{OptionType, VanillaInputs};

use crate::normal::inverse_cdf;
use crate::rng::CounterRng;

/// The two lookback families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LookbackStyle {
    /// Floating strike: settle against the path extremum (`S_T − m` call,
    /// `M − S_T` put). Always finishes in the money (≥ 0).
    FloatingStrike,
    /// Fixed strike: optimal exercise against a fixed `K` (`(M − K)⁺` call,
    /// `(K − m)⁺` put).
    FixedStrike,
}

/// A continuously-monitored lookback specification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lookback {
    /// Floating- or fixed-strike family.
    pub style: LookbackStyle,
    /// Call or put.
    pub option: OptionType,
}

/// Floating-strike lookback closed form (continuous monitoring), per unit base
/// notional, started at-inception (the running extremum equals current spot).
///
/// Call pays `S_T − min`, put pays `max − S_T`.
#[must_use]
pub fn floating_lookback_price(i: &VanillaInputs, option: OptionType) -> f64 {
    let s = i.spot;
    let t = i.t;
    let sig = i.vol;
    let sst = sig * sqrt(t);
    let b = i.r_dom - i.r_for;
    let df_dom = exp(-i.r_dom * t);
    let df_for = exp(-i.r_for * t);

    // At-inception, the running extremum ξ = S. a1 = (b + ½σ²)T / (σ√T).
    let a1 = (b + 0.5 * sig * sig) * t / sst;
    let a2 = a1 - sst;
    // Y exponent of the reflection term: 2b/σ².
    let two_b_over_sig2 = 2.0 * b / (sig * sig);

    match option {
        // Floating-strike lookback call: V = S e^{-r_f T} Φ(a1) − S e^{-r_d T} Φ(a2)
        //   + S e^{-r_d T} (σ²/2b) [ (S/S)^{-2b/σ²} Φ(−a1 + 2b√T/σ) − e^{bT} Φ(−a1) ]
        // with ξ = S so (S/ξ) = 1.
        OptionType::Call => {
            let main = s * df_for * norm_cdf(a1) - s * df_dom * norm_cdf(a2);
            let refl = s
                * df_dom
                * (sig * sig / (2.0 * b))
                * (norm_cdf(-a1 + two_b_over_sig2 * sst) - exp(b * t) * norm_cdf(-a1));
            main + refl
        }
        // Floating-strike lookback put: V = S e^{-r_d T} Φ(−a2) − S e^{-r_f T} Φ(−a1)
        //   + S e^{-r_d T} (σ²/2b) [ −(S/S)^{-2b/σ²} Φ(a1 − 2b√T/σ) + e^{bT} Φ(a1) ]
        OptionType::Put => {
            let main = s * df_dom * norm_cdf(-a2) - s * df_for * norm_cdf(-a1);
            let refl = s
                * df_dom
                * (sig * sig / (2.0 * b))
                * (-norm_cdf(a1 - two_b_over_sig2 * sst) + exp(b * t) * norm_cdf(a1));
            main + refl
        }
    }
}

/// Fixed-strike lookback closed form (continuous monitoring), per unit base
/// notional, started at-inception (the running extremum equals current spot).
///
/// Call pays `(max − K)⁺`, put pays `(K − min)⁺`. Implements the Conze-Viswanathan
/// representation, split on whether the strike is below/above the current spot.
#[must_use]
pub fn fixed_lookback_price(i: &VanillaInputs, option: OptionType) -> f64 {
    let s = i.spot;
    let k = i.strike;
    let t = i.t;
    let sig = i.vol;
    let sst = sig * sqrt(t);
    let b = i.r_dom - i.r_for;
    let df_dom = exp(-i.r_dom * t);
    let df_for = exp(-i.r_for * t);
    let two_b_over_sig2 = 2.0 * b / (sig * sig);

    // d1/d2 against the fixed strike K.
    let d1 = (ln(s / k) + (b + 0.5 * sig * sig) * t) / sst;
    let d2 = d1 - sst;
    // e1/e2 against the current spot S (the at-inception running extremum).
    let e1 = (b + 0.5 * sig * sig) * t / sst;
    let e2 = e1 - sst;

    match option {
        OptionType::Call => {
            if k >= s {
                // K above the running max: standard Conze-Viswanathan call.
                let main = s * df_for * norm_cdf(d1) - k * df_dom * norm_cdf(d2);
                let refl = s
                    * df_dom
                    * (sig * sig / (2.0 * b))
                    * (-((s / k).powf(-two_b_over_sig2)) * norm_cdf(d1 - two_b_over_sig2 * sst)
                        + exp(b * t) * norm_cdf(d1));
                main + refl
            } else {
                // K below the running max (= S at inception): intrinsic-lock form.
                let main = df_dom * (s * exp(b * t) - k) + s * df_for * norm_cdf(e1)
                    - s * df_dom * exp(b * t) * norm_cdf(e2);
                let refl = s
                    * df_dom
                    * (sig * sig / (2.0 * b))
                    * (-norm_cdf(e1 - two_b_over_sig2 * sst) + exp(b * t) * norm_cdf(e1));
                main + refl
            }
        }
        OptionType::Put => {
            if k <= s {
                // K below the running min: standard Conze-Viswanathan put.
                let main = k * df_dom * norm_cdf(-d2) - s * df_for * norm_cdf(-d1);
                let refl = s
                    * df_dom
                    * (sig * sig / (2.0 * b))
                    * ((s / k).powf(-two_b_over_sig2) * norm_cdf(-d1 + two_b_over_sig2 * sst)
                        - exp(b * t) * norm_cdf(-d1));
                main + refl
            } else {
                // K above the running min (= S at inception): intrinsic-lock form.
                let main = df_dom * (k - s * exp(b * t)) + s * df_dom * exp(b * t) * norm_cdf(-e2)
                    - s * df_for * norm_cdf(-e1);
                let refl = s
                    * df_dom
                    * (sig * sig / (2.0 * b))
                    * (norm_cdf(-e1 + two_b_over_sig2 * sst) - exp(b * t) * norm_cdf(-e1));
                main + refl
            }
        }
    }
}

/// Welford accumulator (mean + std-error of the mean).
#[derive(Default)]
struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    #[inline]
    fn push(&mut self, x: f64) {
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (x - self.mean);
    }
    fn std_error(&self) -> f64 {
        if self.n < 2 {
            0.0
        } else {
            sqrt(self.m2 / ((self.n - 1) as f64) / self.n as f64)
        }
    }
}

/// Monte-Carlo configuration for lookback pricing.
#[derive(Debug, Clone, Copy)]
pub struct LookbackMcConfig {
    /// Number of antithetic path **pairs**.
    pub pairs: usize,
    /// Number of time steps per path (the monitoring grid; the bridge correction
    /// recovers continuous monitoring as a sub-grid refinement).
    pub steps: usize,
    /// Seed for the counter-based RNG.
    pub seed: u64,
}

/// A Monte-Carlo lookback estimate.
#[derive(Debug, Clone, Copy)]
pub struct LookbackEstimate {
    /// Discounted price estimate.
    pub price: f64,
    /// Standard error of the mean.
    pub std_error: f64,
}

/// Price a continuously-monitored lookback by Monte-Carlo with antithetic variates
/// and the **Brownian-bridge extremum** correction, so the discrete path prices
/// the continuous-monitoring value.
///
/// For each step the interval continuous minimum/maximum is sampled exactly from
/// the reflected-bridge law: given endpoints `x₀`, `x₁` of one step (variance
/// `v = σ²dt`), the interval minimum of a Brownian bridge is
/// `m = ½[(x₀+x₁) − √((x₁−x₀)² − 2v ln U)]` for an independent `U∼(0,1)`, and the
/// maximum is the sign-flipped analogue. Tracking the running min/max of these
/// per-step bridge extrema gives an unbiased continuous-monitoring extremum.
#[must_use]
pub fn lookback_mc(i: &VanillaInputs, spec: Lookback, cfg: LookbackMcConfig) -> LookbackEstimate {
    assert!(cfg.steps >= 1, "lookback MC needs ≥1 step");
    let dt = i.t / cfg.steps as f64;
    let ln_s0 = ln(i.spot);
    let drift = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt;
    let vol_sqrt_dt = i.vol * sqrt(dt);
    let var_step = vol_sqrt_dt * vol_sqrt_dt;
    let df = exp(-i.r_dom * i.t);

    let mut acc = Welford::default();
    // Per path we need `steps` normals for the spot increments and `steps`
    // uniforms for the bridge-extremum draws: two independent sub-streams.
    let mut z = vec![0.0f64; cfg.steps];
    let mut u = vec![0.0f64; cfg.steps];

    for pair in 0..cfg.pairs as u64 {
        // Sub-stream 0: normal increments; sub-stream 1: bridge-extremum uniforms.
        let mut rz = CounterRng::new(cfg.seed, 0, pair, 0);
        let mut ru = CounterRng::new(cfg.seed, 1, pair, 0);
        for k in 0..cfg.steps {
            z[k] = inverse_cdf(rz.next_u01());
            u[k] = ru.next_u01();
        }
        let a = lookback_payoff(
            ln_s0,
            drift,
            vol_sqrt_dt,
            var_step,
            &z,
            &u,
            spec,
            i.strike,
            1.0,
        );
        let b = lookback_payoff(
            ln_s0,
            drift,
            vol_sqrt_dt,
            var_step,
            &z,
            &u,
            spec,
            i.strike,
            -1.0,
        );
        acc.push(0.5 * (a + b));
    }

    LookbackEstimate {
        price: df * acc.mean,
        std_error: df * acc.std_error(),
    }
}

/// One lookback path payoff (undiscounted) with antithetic sign `s`, tracking the
/// continuous running extremum via the per-step bridge-extremum draw.
#[allow(clippy::too_many_arguments)]
fn lookback_payoff(
    ln_s0: f64,
    drift: f64,
    vol_sqrt_dt: f64,
    var_step: f64,
    z: &[f64],
    u: &[f64],
    spec: Lookback,
    strike: f64,
    s: f64,
) -> f64 {
    let mut x_prev = ln_s0;
    let mut run_min = ln_s0;
    let mut run_max = ln_s0;
    for k in 0..z.len() {
        let x_next = x_prev + drift + vol_sqrt_dt * s * z[k];
        // Reflected-bridge interval extrema between x_prev and x_next.
        // min = ½[(x0+x1) − √((x1−x0)² − 2v lnU)];  max = ½[(x0+x1) + √(…)].
        let diff = x_next - x_prev;
        let radicand = diff * diff - 2.0 * var_step * ln(u[k]);
        let root = sqrt(radicand);
        let bridge_min = 0.5 * ((x_prev + x_next) - root);
        let bridge_max = 0.5 * ((x_prev + x_next) + root);
        if bridge_min < run_min {
            run_min = bridge_min;
        }
        if bridge_max > run_max {
            run_max = bridge_max;
        }
        x_prev = x_next;
    }
    let s_t = exp(x_prev);
    let min = exp(run_min);
    let max = exp(run_max);

    match (spec.style, spec.option) {
        (LookbackStyle::FloatingStrike, OptionType::Call) => s_t - min,
        (LookbackStyle::FloatingStrike, OptionType::Put) => max - s_t,
        (LookbackStyle::FixedStrike, OptionType::Call) => (max - strike).max(0.0),
        (LookbackStyle::FixedStrike, OptionType::Put) => (strike - min).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> VanillaInputs {
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02)
    }

    /// A floating-strike lookback is always at least as valuable as the
    /// corresponding vanilla (it buys at the best price seen), and strictly
    /// positive — a basic no-arbitrage sanity bound on the closed form.
    #[test]
    fn floating_dominates_and_is_positive() {
        let i = base();
        for opt in [OptionType::Call, OptionType::Put] {
            let lb = floating_lookback_price(&i, opt);
            let vanilla = celnet_vanilla::price(opt, &i);
            assert!(lb > 0.0, "floating lookback must be positive, got {lb}");
            assert!(
                lb >= vanilla - 1e-9,
                "floating lookback {lb} must dominate vanilla {vanilla}"
            );
        }
    }

    /// Monte-Carlo with the bridge-extremum correction reproduces the
    /// continuous-monitoring floating-strike closed form — the headline
    /// cross-validation.
    #[test]
    fn mc_matches_floating_closed_form() {
        let i = base();
        let cfg = LookbackMcConfig {
            pairs: 120_000,
            steps: 50,
            seed: 0x100B,
        };
        for opt in [OptionType::Call, OptionType::Put] {
            let closed = floating_lookback_price(&i, opt);
            let mc = lookback_mc(
                &i,
                Lookback {
                    style: LookbackStyle::FloatingStrike,
                    option: opt,
                },
                cfg,
            );
            let tol = 4.0 * mc.std_error + 8e-2;
            assert!(
                (mc.price - closed).abs() < tol,
                "floating {opt:?}: MC {} vs closed {closed} (se {}, tol {tol})",
                mc.price,
                mc.std_error
            );
        }
    }

    /// Monte-Carlo reproduces the fixed-strike closed form (ATM strike).
    #[test]
    fn mc_matches_fixed_closed_form() {
        let i = base();
        let cfg = LookbackMcConfig {
            pairs: 120_000,
            steps: 50,
            seed: 0xF1AE,
        };
        for opt in [OptionType::Call, OptionType::Put] {
            let closed = fixed_lookback_price(&i, opt);
            let mc = lookback_mc(
                &i,
                Lookback {
                    style: LookbackStyle::FixedStrike,
                    option: opt,
                },
                cfg,
            );
            let tol = 4.0 * mc.std_error + 8e-2;
            assert!(
                (mc.price - closed).abs() < tol,
                "fixed {opt:?}: MC {} vs closed {closed} (se {}, tol {tol})",
                mc.price,
                mc.std_error
            );
        }
    }

    /// Reproducibility: identical seed ⇒ bit-identical MC price.
    #[test]
    fn mc_is_reproducible() {
        let i = base();
        let cfg = LookbackMcConfig {
            pairs: 20_000,
            steps: 30,
            seed: 0x5EED,
        };
        let spec = Lookback {
            style: LookbackStyle::FloatingStrike,
            option: OptionType::Call,
        };
        let a = lookback_mc(&i, spec, cfg);
        let b = lookback_mc(&i, spec, cfg);
        assert_eq!(a.price.to_bits(), b.price.to_bits());
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
    }

    /// The bridge-extremum correction matters: a naive node-only extremum
    /// under-prices the continuous lookback, so the bridge MC must sit *above* the
    /// crude node-only MC on the same paths (the correction adds value).
    #[test]
    fn bridge_correction_lifts_above_node_only() {
        let i = base();
        let cfg = LookbackMcConfig {
            pairs: 60_000,
            steps: 20,
            seed: 0xB81D,
        };
        let bridge = lookback_mc(
            &i,
            Lookback {
                style: LookbackStyle::FloatingStrike,
                option: OptionType::Call,
            },
            cfg,
        );

        // Node-only MC of the same floating call (running extremum = sampled nodes).
        let dt = i.t / cfg.steps as f64;
        let ln_s0 = ln(i.spot);
        let drift = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt;
        let vol_sqrt_dt = i.vol * sqrt(dt);
        let df = exp(-i.r_dom * i.t);
        let mut acc = Welford::default();
        let mut z = vec![0.0f64; cfg.steps];
        for pair in 0..cfg.pairs as u64 {
            let mut rz = CounterRng::new(cfg.seed, 0, pair, 0);
            for zk in z.iter_mut() {
                *zk = inverse_cdf(rz.next_u01());
            }
            let node_payoff = |s: f64| {
                let mut x = ln_s0;
                let mut run_min = ln_s0;
                for &zk in &z {
                    x += drift + vol_sqrt_dt * s * zk;
                    if x < run_min {
                        run_min = x;
                    }
                }
                exp(x) - exp(run_min)
            };
            acc.push(0.5 * (node_payoff(1.0) + node_payoff(-1.0)));
        }
        let node_only = df * acc.mean;
        assert!(
            bridge.price > node_only,
            "bridge-corrected {} must exceed node-only {}",
            bridge.price,
            node_only
        );
    }
}
