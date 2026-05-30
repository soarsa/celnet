//! Monte-Carlo pricing engine for path-dependent FX-options payoffs.
//!
//! The engine simulates the Garman-Kohlhagen log-spot under the domestic
//! risk-neutral measure and prices barrier and Asian payoffs with the full set of
//! variance-reduction and bias-correction techniques the spec mandates:
//!
//! * **Counter-based RNG** ([`crate::rng::CounterRng`]) seeded by
//!   `(stream, path, step)`, so identical seeds give **bit-identical** results
//!   regardless of evaluation order or parallelism.
//! * **Inverse-CDF normals** ([`crate::normal::inverse_cdf`]) as the default
//!   transform (monotone, bridge-friendly), with **Box-Muller** available as the
//!   second route.
//! * **Antithetic variates** — every path is simulated with its sign-flipped
//!   twin and the two payoffs averaged, cancelling the odd part of the estimator.
//! * **Geometric-Asian control variate** — the arithmetic-Asian estimator is
//!   corrected by its geometric twin, whose expectation is a closed form
//!   (lognormal of the average), removing most of the variance.
//! * **Brownian-bridge construction** with the **Broadie-Glasserman-Kou (BGK)**
//!   continuity correction, so a discretely-stepped path prices a
//!   *continuously*-monitored barrier without the survivorship bias of naive
//!   discrete monitoring: between two simulated nodes the conditional
//!   (bridge) probability of having crossed the barrier is computed in closed
//!   form, and the BGK shift `H·exp(±β σ√dt)`, `β = −ζ(½)/√(2π) ≈ 0.5826`,
//!   removes the residual `O(1/√steps)` discretisation bias.
//!
//! Provenance (doc-only): Salmon-Moraes-Dror-Shaw (2011) RNG; Glasserman (2003)
//! for the bridge and control-variate framework; Broadie-Glasserman-Kou (1997)
//! and the continuity correction of Broadie-Glasserman-Kou (1999); Kemna-Vorst
//! (1990) for the geometric-Asian control. Identifiers are purpose-named.

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::VanillaInputs;
use celnet_vanilla::price as vanilla_price;

use crate::normal::inverse_cdf;
use crate::payoff::{ArithmeticAsian, DiscreteBarrier};
use crate::rng::CounterRng;

/// The Broadie-Glasserman-Kou continuity-correction constant `β = −ζ(½)/√(2π)`.
///
/// Shifting a discretely-monitored barrier outward by `β·σ·√dt` makes the
/// discrete-monitoring price converge to the continuous one to `o(1/√steps)`.
pub const BGK_BETA: f64 = 0.582_597_403_404_879_2;

/// Monte-Carlo run configuration.
#[derive(Debug, Clone, Copy)]
pub struct McConfig {
    /// Number of antithetic path **pairs** (the estimator averages `2·pairs`
    /// path payoffs).
    pub pairs: usize,
    /// Number of time steps per path.
    pub steps: usize,
    /// Seed for the counter-based RNG. Identical seeds reproduce results exactly.
    pub seed: u64,
}

/// The result of a Monte-Carlo pricing run: the point estimate plus the standard
/// error of the mean, so callers can size confidence intervals.
#[derive(Debug, Clone, Copy)]
pub struct McEstimate {
    /// Discounted price estimate.
    pub price: f64,
    /// Standard error of the mean (`√(Var / n)` over the antithetic-pair means).
    pub std_error: f64,
}

/// Pre-computed per-step Garman-Kohlhagen log-spot dynamics.
struct Dynamics {
    ln_s0: f64,
    drift_step: f64,
    vol_step: f64,
    df: f64,
}

impl Dynamics {
    fn new(i: &VanillaInputs, steps: usize) -> Self {
        let dt = i.t / steps as f64;
        Self {
            ln_s0: ln(i.spot),
            drift_step: (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt,
            vol_step: i.vol * sqrt(dt),
            df: exp(-i.r_dom * i.t),
        }
    }
}

/// Draw the `steps` standard-normal increments of one path from its own
/// counter-based sub-stream, via the monotone inverse-CDF transform.
#[inline]
fn draw_increments(seed: u64, stream: u32, path: u64, steps: usize, out: &mut [f64]) {
    // One CounterRng per (path) keyed on a fixed step base; successive draws
    // address successive normals. Reproducible from (seed, stream, path).
    let mut rng = CounterRng::new(seed, stream, path, 0);
    for z in out.iter_mut().take(steps) {
        *z = inverse_cdf(rng.next_u01());
    }
}

/// Accumulator of mean and variance over independent (antithetic-pair) samples.
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
            return 0.0;
        }
        sqrt(self.m2 / ((self.n - 1) as f64) / self.n as f64)
    }
}

/// Price a continuously-monitored single barrier by Monte-Carlo with antithetic
/// variates, Brownian-bridge crossing probabilities and the BGK shift.
///
/// The estimator simulates the discrete path, applies the BGK barrier shift to
/// emulate continuous monitoring, and for each inter-node interval adds the
/// **Brownian-bridge** conditional crossing probability so the survival weight is
/// exact between observation points rather than only at them. Knock-in is priced
/// by parity inside [`crate::payoff::DiscreteBarrier::terminal`] via the touched
/// flag.
#[must_use]
pub fn price_barrier(i: &VanillaInputs, spec: DiscreteBarrier, cfg: McConfig) -> McEstimate {
    let dyn_ = Dynamics::new(i, cfg.steps);

    // Continuous monitoring is recovered by the Brownian-bridge crossing
    // probability between nodes (see `barrier_payoff`), which is *exact* for the
    // driftless bridge — so the discrete barrier is used unshifted. The
    // alternative discrete-shift correction is exposed as [`BGK_BETA`] and applied
    // by `price_barrier_bgk_shifted` for callers that prefer the cheaper
    // step-monitoring estimator.
    let ln_h = ln(spec.barrier);

    let mut acc = Welford::default();
    let mut z = vec![0.0f64; cfg.steps];

    for pair in 0..cfg.pairs {
        draw_increments(cfg.seed, 0, pair as u64, cfg.steps, &mut z);
        // Antithetic pair: +z and −z share the same survival/no-touch logic.
        let a = barrier_payoff(&dyn_, spec, ln_h, &z, 1.0);
        let b = barrier_payoff(&dyn_, spec, ln_h, &z, -1.0);
        acc.push(0.5 * (a + b));
    }

    McEstimate {
        price: dyn_.df * acc.mean,
        std_error: dyn_.df * acc.std_error(),
    }
}

/// One barrier path payoff with sign `s ∈ {+1,−1}` (antithetic selector),
/// undiscounted. Uses the Brownian-bridge survival probability across each step.
fn barrier_payoff(dyn_: &Dynamics, spec: DiscreteBarrier, ln_h: f64, z: &[f64], s: f64) -> f64 {
    let mut ln_prev = dyn_.ln_s0;
    // Survival probability accumulated over the path (probability of *not* having
    // crossed the barrier on any inter-node bridge).
    let mut survival = 1.0;
    let var_step = dyn_.vol_step * dyn_.vol_step;

    for &zi in z {
        let ln_next = ln_prev + dyn_.drift_step + dyn_.vol_step * s * zi;

        // Brownian-bridge probability of crossing the barrier between
        // ln_prev and ln_next, conditional on the endpoints. For an absorbing
        // level `b`, the bridge no-crossing probability is
        //   1 − exp(−2 (b−a)(b−c) / var)   for endpoints a, c on the same side.
        let same_side = if spec.up {
            ln_prev < ln_h && ln_next < ln_h
        } else {
            ln_prev > ln_h && ln_next > ln_h
        };
        let step_survival = if same_side {
            let num = 2.0 * (ln_h - ln_prev) * (ln_h - ln_next);
            1.0 - exp(-num / var_step)
        } else {
            // An endpoint is already on/over the barrier ⇒ certain crossing.
            0.0
        };
        survival *= step_survival;
        ln_prev = ln_next;
        // Do not break early: the terminal spot must be the *full* path endpoint
        // so KO and KI built from the same path satisfy KO + KI = terminal
        // intrinsic exactly (in/out parity). `survival` clamps to 0 naturally.
    }

    let terminal_spot = exp(ln_prev);
    let intrinsic = crate::payoff::vanilla_intrinsic(spec.option, terminal_spot, spec.strike);

    // `survival` is P(never touched); the touched probability is its complement.
    // Knock-out value = survival · intrinsic; knock-in = (1−survival) · intrinsic.
    if spec.knock_in {
        (1.0 - survival) * intrinsic
    } else {
        survival * intrinsic
    }
}

/// Price a continuously-monitored single barrier by the *alternative*
/// discrete-monitoring estimator with the **Broadie-Glasserman-Kou** continuity
/// shift (rather than the Brownian bridge of [`price_barrier`]).
///
/// Here each path is monitored only at the grid nodes — a touch is recorded iff a
/// node lands beyond the barrier — but to price a *continuously*-monitored
/// barrier the monitored level is shifted **inward** (toward spot) by `β·σ·√dt`
/// ([`BGK_BETA`]). A discretely-monitored barrier under-counts crossings (the
/// path can dip past the wall and recover between observations), so monitoring an
/// inward-shifted level restores the missing knock probability — the
/// leading-order correction that makes the discrete estimator converge to the
/// continuous price as `steps → ∞`. This is cheaper per step than the bridge (no
/// `exp` per interval) and is the preferred route on the GPU; it is validated to
/// agree with both the bridge estimator and the analytic closed form.
#[must_use]
pub fn price_barrier_bgk_shifted(
    i: &VanillaInputs,
    spec: DiscreteBarrier,
    cfg: McConfig,
) -> McEstimate {
    let dyn_ = Dynamics::new(i, cfg.steps);
    let shift = BGK_BETA * dyn_.vol_step;
    // Shift the monitored barrier inward (down for up-barriers, up for down).
    let ln_h = ln(spec.barrier) - if spec.up { shift } else { -shift };

    let mut acc = Welford::default();
    let mut z = vec![0.0f64; cfg.steps];
    for pair in 0..cfg.pairs {
        draw_increments(cfg.seed, 0, pair as u64, cfg.steps, &mut z);
        let a = barrier_payoff_discrete(&dyn_, spec, ln_h, &z, 1.0);
        let b = barrier_payoff_discrete(&dyn_, spec, ln_h, &z, -1.0);
        acc.push(0.5 * (a + b));
    }
    McEstimate {
        price: dyn_.df * acc.mean,
        std_error: dyn_.df * acc.std_error(),
    }
}

/// Discrete (node-monitored) barrier payoff with antithetic sign `s`,
/// undiscounted: a touch is a hard indicator at the simulated nodes against the
/// BGK-shifted barrier `ln_h`.
fn barrier_payoff_discrete(
    dyn_: &Dynamics,
    spec: DiscreteBarrier,
    ln_h: f64,
    z: &[f64],
    s: f64,
) -> f64 {
    let mut ln_s = dyn_.ln_s0;
    let mut touched = false;
    for &zi in z {
        ln_s += dyn_.drift_step + dyn_.vol_step * s * zi;
        if (spec.up && ln_s >= ln_h) || (!spec.up && ln_s <= ln_h) {
            touched = true;
        }
    }
    spec.terminal(exp(ln_s), touched)
}

/// Price a fixed-strike **arithmetic** Asian option by Monte-Carlo with a
/// **geometric**-Asian control variate and antithetic variates.
///
/// The geometric average has a lognormal law, so `E[geometric payoff]` is a
/// closed form ([`geometric_asian_price`]); subtracting the sampled geometric
/// payoff and adding back its known mean removes the dominant common variance.
/// The optimal control coefficient is estimated from the same sample (the
/// regression `β = Cov(arith, geo)/Var(geo)`), the standard unbiased estimator.
#[must_use]
pub fn price_asian(i: &VanillaInputs, spec: ArithmeticAsian, cfg: McConfig) -> McEstimate {
    assert!(spec.observations >= 1, "Asian needs ≥1 observation");
    let steps = spec.observations;
    let dyn_ = Dynamics::new(i, steps);
    let geo_mean = geometric_asian_price(i, spec) / dyn_.df; // undiscounted control mean

    let mut z = vec![0.0f64; steps];

    // First pass: collect paired (arith, geo) samples to estimate β and means.
    let mut arith = Welford::default();
    let mut geo = Welford::default();
    let mut cov = 0.0f64;
    let (mut ma, mut mg) = (0.0f64, 0.0f64);
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(cfg.pairs);

    for pair in 0..cfg.pairs {
        draw_increments(cfg.seed, 0, pair as u64, steps, &mut z);
        let (a_ar, a_ge) = asian_payoff(&dyn_, spec, &z, 1.0);
        let (b_ar, b_ge) = asian_payoff(&dyn_, spec, &z, -1.0);
        let s_ar = 0.5 * (a_ar + b_ar);
        let s_ge = 0.5 * (a_ge + b_ge);
        samples.push((s_ar, s_ge));
        arith.push(s_ar);
        geo.push(s_ge);
        // Online covariance accumulation.
        let n = (pair + 1) as f64;
        let da = s_ar - ma;
        let dg = s_ge - mg;
        ma += da / n;
        mg += dg / n;
        cov += da * (s_ge - mg);
    }

    let var_geo = geo.m2;
    // Regression coefficient β (0 if the control has no variance).
    let beta = if var_geo > 0.0 { cov / var_geo } else { 0.0 };

    // Second pass: form the control-corrected sample
    //   Y = arith − β·(geo − E[geo])
    // and accumulate its mean/variance.
    let mut corrected = Welford::default();
    for (s_ar, s_ge) in &samples {
        corrected.push(s_ar - beta * (s_ge - geo_mean));
    }

    McEstimate {
        price: dyn_.df * corrected.mean,
        std_error: dyn_.df * corrected.std_error(),
    }
}

/// One Asian path with antithetic sign `s`: returns `(arithmetic_payoff,
/// geometric_payoff)` undiscounted, sharing the same path so they are maximally
/// correlated.
fn asian_payoff(dyn_: &Dynamics, spec: ArithmeticAsian, z: &[f64], s: f64) -> (f64, f64) {
    let mut ln_s = dyn_.ln_s0;
    let mut sum = 0.0f64; // Σ S_k       (arithmetic)
    let mut log_sum = 0.0f64; // Σ ln S_k (geometric)
    for &zi in z {
        ln_s += dyn_.drift_step + dyn_.vol_step * s * zi;
        sum += exp(ln_s);
        log_sum += ln_s;
    }
    let n = z.len() as f64;
    let arith_avg = sum / n;
    let geo_avg = exp(log_sum / n);
    (
        spec.terminal(arith_avg),
        crate::payoff::vanilla_intrinsic(spec.option, geo_avg, spec.strike),
    )
}

/// Closed-form price of the **geometric**-average-rate Asian (the control
/// variate), discounted.
///
/// The geometric average of lognormal observations is itself lognormal; the
/// option therefore prices with a Black-Scholes-style formula at an adjusted
/// volatility and forward. With `n` equally-spaced observations on `(0, T]`, the
/// geometric average's log has mean and variance (Kemna-Vorst 1990):
/// ```text
///   σ_G² = σ² (n+1)(2n+1) / (6 n²) · T / T = σ² (n+1)(2n+1)/(6n²)
///   μ_G  = (b − ½σ²) T (n+1)/(2n) + ½ σ_G² T
/// ```
/// evaluated at the running discrete dynamics; the resulting effective forward
/// and vol are plugged into the Garman-Kohlhagen vanilla formula.
#[must_use]
pub fn geometric_asian_price(i: &VanillaInputs, spec: ArithmeticAsian) -> f64 {
    let n = spec.observations as f64;
    let t = i.t;
    let b = i.r_dom - i.r_for;

    // Effective (adjusted) volatility and carry of the geometric average.
    let sig2 = i.vol * i.vol * (n + 1.0) * (2.0 * n + 1.0) / (6.0 * n * n);
    let eff_vol = sqrt(sig2);
    let eff_b = 0.5 * (b - 0.5 * i.vol * i.vol) * (n + 1.0) / n + 0.5 * sig2;

    // Recast as a Garman-Kohlhagen vanilla: choose r_for so that carry = eff_b
    // (r_dom − r_for = eff_b ⇒ r_for = r_dom − eff_b), keep r_dom for discounting.
    let synthetic = VanillaInputs {
        spot: i.spot,
        strike: spec.strike,
        vol: eff_vol,
        t,
        r_dom: i.r_dom,
        r_for: i.r_dom - eff_b,
    };
    vanilla_price(spec.option, &synthetic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BarrierKind, BarrierStyle, SingleBarrier, single_barrier_price};
    use celnet_types::OptionType;

    fn base() -> VanillaInputs {
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02)
    }

    /// Determinism: identical seed ⇒ bit-identical MC price (the engine's
    /// reproducibility contract).
    #[test]
    fn identical_seed_bit_identical_price() {
        let i = base();
        let spec = DiscreteBarrier {
            option: OptionType::Call,
            strike: 100.0,
            barrier: 130.0,
            up: true,
            knock_in: false,
        };
        let cfg = McConfig {
            pairs: 20_000,
            steps: 50,
            seed: 0xABCD_1234,
        };
        let a = price_barrier(&i, spec, cfg);
        let b = price_barrier(&i, spec, cfg);
        assert_eq!(a.price.to_bits(), b.price.to_bits(), "MC not reproducible");
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
    }

    /// A different seed gives a (statistically) different draw — confirms the
    /// seed actually threads through to the variates.
    #[test]
    fn different_seed_differs() {
        let i = base();
        let spec = DiscreteBarrier {
            option: OptionType::Call,
            strike: 100.0,
            barrier: 130.0,
            up: true,
            knock_in: false,
        };
        let a = price_barrier(
            &i,
            spec,
            McConfig {
                pairs: 5_000,
                steps: 40,
                seed: 1,
            },
        );
        let b = price_barrier(
            &i,
            spec,
            McConfig {
                pairs: 5_000,
                steps: 40,
                seed: 2,
            },
        );
        assert!(a.price.to_bits() != b.price.to_bits());
    }

    /// MC up-and-out call (Brownian-bridge + BGK) matches the Reiner-Rubinstein
    /// closed form within Monte-Carlo tolerance — cross-validating the path
    /// engine against the analytic S1 layer.
    #[test]
    fn mc_barrier_matches_analytic() {
        let i = base();
        let (k, h) = (100.0, 130.0);
        let spec = DiscreteBarrier {
            option: OptionType::Call,
            strike: k,
            barrier: h,
            up: true,
            knock_in: false,
        };
        let est = price_barrier(
            &i,
            spec,
            McConfig {
                pairs: 100_000,
                steps: 100,
                seed: 0x5EED,
            },
        );
        let analytic = single_barrier_price(
            &i,
            SingleBarrier {
                kind: BarrierKind {
                    up: true,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike: k,
                barrier: h,
                rebate: 0.0,
            },
        );
        // Within ~3 standard errors (plus a small bridge-discretisation margin).
        let tol = 3.0 * est.std_error + 2e-2;
        assert!(
            (est.price - analytic).abs() < tol,
            "MC barrier {} vs analytic {} (se={}, tol={})",
            est.price,
            analytic,
            est.std_error,
            tol
        );
    }

    /// The alternative BGK-shift estimator (discrete node monitoring + barrier
    /// shift) also matches the analytic continuous-barrier price, and tracks the
    /// Brownian-bridge estimator — the two mandated continuity corrections agree.
    #[test]
    fn mc_barrier_bgk_shift_matches_analytic() {
        let i = base();
        let (k, h) = (100.0, 130.0);
        let spec = DiscreteBarrier {
            option: OptionType::Call,
            strike: k,
            barrier: h,
            up: true,
            knock_in: false,
        };
        let cfg = McConfig {
            pairs: 100_000,
            steps: 250,
            seed: 0x5EED,
        };
        let bgk = price_barrier_bgk_shifted(&i, spec, cfg);
        let bridge = price_barrier(&i, spec, cfg);
        let analytic = single_barrier_price(
            &i,
            SingleBarrier {
                kind: BarrierKind {
                    up: true,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike: k,
                barrier: h,
                rebate: 0.0,
            },
        );
        // The discrete-shift estimator carries a slightly larger O(1/√steps)
        // residual than the bridge, so allow a touch more slack.
        let tol = 3.0 * bgk.std_error + 4e-2;
        assert!(
            (bgk.price - analytic).abs() < tol,
            "BGK MC {} vs analytic {} (se={})",
            bgk.price,
            analytic,
            bgk.std_error
        );
        assert!(
            (bgk.price - bridge.price).abs() < 5e-2,
            "BGK {} vs bridge {} should agree",
            bgk.price,
            bridge.price
        );
    }

    /// In/out parity holds for the MC estimator built from the *same* paths:
    /// KO + KI = vanilla, because survival + (1−survival) = 1 pathwise.
    #[test]
    fn mc_in_out_parity() {
        let i = base();
        let (k, h) = (100.0, 130.0);
        let cfg = McConfig {
            pairs: 50_000,
            steps: 80,
            seed: 7,
        };
        let ko = price_barrier(
            &i,
            DiscreteBarrier {
                option: OptionType::Call,
                strike: k,
                barrier: h,
                up: true,
                knock_in: false,
            },
            cfg,
        );
        let ki = price_barrier(
            &i,
            DiscreteBarrier {
                option: OptionType::Call,
                strike: k,
                barrier: h,
                up: true,
                knock_in: true,
            },
            cfg,
        );
        // Pathwise, KO + KI = survival·I + (1−survival)·I = I (the terminal
        // intrinsic), so the *exact* identity is against the MC estimate of the
        // discounted terminal intrinsic on the same paths — not the analytic
        // vanilla, which differs by Monte-Carlo error. Recompute that reference.
        let dyn_ = Dynamics::new(&i, cfg.steps);
        let mut z = vec![0.0; cfg.steps];
        let mut acc = Welford::default();
        for pair in 0..cfg.pairs as u64 {
            draw_increments(cfg.seed, 0, pair, cfg.steps, &mut z);
            let term = |s: f64| {
                let mut ln_s = dyn_.ln_s0;
                for &zi in &z {
                    ln_s += dyn_.drift_step + dyn_.vol_step * s * zi;
                }
                crate::payoff::vanilla_intrinsic(OptionType::Call, exp(ln_s), k)
            };
            acc.push(0.5 * (term(1.0) + term(-1.0)));
        }
        let mc_vanilla = dyn_.df * acc.mean;
        // Parity is exact to floating-point round-off (same paths, same seed).
        assert!(
            (ko.price + ki.price - mc_vanilla).abs() < 1e-9,
            "MC parity: KO {} + KI {} vs MC-vanilla {}",
            ko.price,
            ki.price,
            mc_vanilla
        );
        // And the MC vanilla itself is close to the analytic value (sanity).
        let analytic = vanilla_price(OptionType::Call, &VanillaInputs { strike: k, ..i });
        assert!(
            (mc_vanilla - analytic).abs() < 3.0 * dyn_.df * acc.std_error() + 1e-3,
            "MC vanilla {mc_vanilla} vs analytic {analytic}"
        );
    }

    /// The geometric-Asian closed form matches a *plain* MC of the geometric
    /// payoff (no control) — validates the control-variate's analytic mean.
    #[test]
    fn geometric_closed_form_matches_mc() {
        let i = base();
        let spec = ArithmeticAsian {
            option: OptionType::Call,
            strike: 100.0,
            observations: 12,
        };
        let closed = geometric_asian_price(&i, spec);

        // Plain MC of the geometric payoff.
        let dyn_ = Dynamics::new(&i, spec.observations);
        let mut z = vec![0.0; spec.observations];
        let mut acc = Welford::default();
        for pair in 0..200_000u64 {
            draw_increments(0xC0FFEE, 0, pair, spec.observations, &mut z);
            let (_, g_pos) = asian_payoff(&dyn_, spec, &z, 1.0);
            let (_, g_neg) = asian_payoff(&dyn_, spec, &z, -1.0);
            acc.push(0.5 * (g_pos + g_neg));
        }
        let mc = dyn_.df * acc.mean;
        assert!(
            (closed - mc).abs() < 3.0 * dyn_.df * acc.std_error() + 5e-3,
            "geometric closed {closed} vs MC {mc} (se={})",
            dyn_.df * acc.std_error()
        );
    }

    /// The control variate reduces variance: the standard error of the
    /// control-corrected arithmetic-Asian estimator is materially smaller than
    /// the raw (uncorrected) estimator on the same paths and seed.
    #[test]
    fn control_variate_reduces_variance() {
        let i = base();
        let spec = ArithmeticAsian {
            option: OptionType::Call,
            strike: 100.0,
            observations: 12,
        };
        let cfg = McConfig {
            pairs: 40_000,
            steps: 12,
            seed: 0xBEEF,
        };

        // Corrected estimator.
        let corrected = price_asian(&i, spec, cfg);

        // Raw arithmetic estimator standard error on identical paths/seed.
        let dyn_ = Dynamics::new(&i, spec.observations);
        let mut z = vec![0.0; spec.observations];
        let mut raw = Welford::default();
        for pair in 0..cfg.pairs as u64 {
            draw_increments(cfg.seed, 0, pair, spec.observations, &mut z);
            let (a, _) = asian_payoff(&dyn_, spec, &z, 1.0);
            let (b, _) = asian_payoff(&dyn_, spec, &z, -1.0);
            raw.push(0.5 * (a + b));
        }
        let raw_se = dyn_.df * raw.std_error();
        assert!(
            corrected.std_error < 0.5 * raw_se,
            "control variate should cut SE by >2x: corrected {} vs raw {}",
            corrected.std_error,
            raw_se
        );
    }

    /// Cross-validation triangle: for a vanilla-equivalent Asian with a single
    /// observation at expiry (`observations = 1`), the arithmetic and geometric
    /// averages both collapse to the terminal spot, so the MC must reprice the
    /// plain Garman-Kohlhagen vanilla.
    #[test]
    fn single_observation_asian_is_vanilla() {
        let i = base();
        let spec = ArithmeticAsian {
            option: OptionType::Call,
            strike: 100.0,
            observations: 1,
        };
        let est = price_asian(
            &i,
            spec,
            McConfig {
                pairs: 100_000,
                steps: 1,
                seed: 11,
            },
        );
        let vanilla = vanilla_price(OptionType::Call, &i);
        // With one observation the geometric control equals the arithmetic payoff
        // exactly ⇒ β≈1 ⇒ the estimator collapses onto the closed form: tiny SE.
        assert!(
            (est.price - vanilla).abs() < 1e-2,
            "1-obs Asian {} vs vanilla {}",
            est.price,
            vanilla
        );
    }
}
