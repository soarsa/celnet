//! Parity row — **forward-start vanilla** and **cliquet (ratchet)** pricers,
//! each gated against a genuinely independent oracle at its honest accuracy.
//!
//! `celnet-exotics` adds:
//!
//!   * [`forward_start_price`] — the exact Rubinstein (1990) FX dual-carry
//!     **closed form** for a strike-reset vanilla `φ·(S(T) − m·S(t₁))⁺`;
//!   * [`cliquet_price_plain`] — the plain ratchet as the **exact sum** of its
//!     per-period forward-start legs (closed form);
//!   * [`cliquet_price_capped_mc`] / [`cliquet_price_plain_mc`] — the
//!     locally-capped / -floored cliquet (and its unclamped check) by
//!     Monte-Carlo on the crate's counter-based RNG.
//!
//! The oracles are independent of the implementation under test:
//!
//!   (i)   **closed form == converged MC** — the forward-start closed form is
//!         reconciled with a from-scratch path-simulation MC written *here* in
//!         the test (two log-return increments per path: one to the reset, one
//!         from reset to expiry; strike fixed at the simulated `S(t₁)`), which
//!         reports its own standard error. Agreement is required within a few
//!         reported MC standard errors across a sweep of `m / t₁ / T / vol`.
//!   (ii)  **degenerate limit** — `t₁ → 0` reduces the forward-start exactly
//!         (~1e-9) to the plain Garman-Kohlhagen vanilla struck at `m·S₀`
//!         ([`celnet_vanilla::price`]).
//!   (iii) **cliquet == Σ forward-starts** (~1e-10), the locally-capped cliquet
//!         MC reconciles to a from-scratch independent MC of the full clamped
//!         payoff written here, and a tighter local cap strictly reduces the
//!         value (monotone, structural).
//!
//! Method provenance (Rubinstein 1990; Wilmott 2002) lives in the
//! `celnet-exotics::forward_start` module docs; identifiers here are
//! purpose-named.

use celnet_core::math::{exp, sqrt};
use celnet_exotics::{
    Cliquet, CliquetMcConfig, CliquetSchedule, ForwardStart, cliquet_price_capped_mc,
    cliquet_price_plain, cliquet_price_plain_mc, forward_start_price,
};
use celnet_exotics::{CounterRng, inverse_cdf};
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::price as vanilla_price;

fn inputs(spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> VanillaInputs {
    VanillaInputs::new(spot, strike, vol, t, r_dom, r_for)
}

/// Online mean / variance accumulator (Welford), for an honest reported MC
/// standard error in the from-scratch test oracles.
#[derive(Default)]
struct Stats {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Stats {
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

/// Independent from-scratch Monte-Carlo of the forward-start payoff.
///
/// Simulates two correlated-by-construction GBM legs per path: the log-ratio to
/// the reset `t₁` and the log-ratio from `t₁` to expiry `T`, both driven by
/// fresh standard normals drawn from the crate's counter-based RNG (a genuinely
/// different code path from the closed form). The strike is fixed at the
/// simulated `S(t₁) = S₀·exp(leg₁)`, the terminal `S(T) = S(t₁)·exp(leg₂)`, and
/// the discounted payoff `e^{−r_d T}·φ·(S(T) − m·S(t₁))⁺` is averaged with its
/// antithetic twin.
fn forward_start_mc(i: &VanillaInputs, spec: ForwardStart, pairs: usize, seed: u64) -> (f64, f64) {
    let t1 = spec.reset;
    let t2 = spec.expiry;
    let dt1 = t1;
    let dt2 = t2 - t1;
    let drift1 = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt1;
    let drift2 = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt2;
    let vol1 = i.vol * sqrt(dt1);
    let vol2 = i.vol * sqrt(dt2);
    let df = exp(-i.r_dom * t2);
    let phi = match spec.option {
        OptionType::Call => 1.0,
        OptionType::Put => -1.0,
    };

    let mut stats = Stats::default();
    for path in 0..pairs as u64 {
        let mut rng = CounterRng::new(seed, 0, path, 0);
        let z1 = inverse_cdf(rng.next_u01());
        let z2 = inverse_cdf(rng.next_u01());
        let payoff = |s: f64| {
            let s_reset = i.spot * exp(drift1 + vol1 * s * z1);
            let s_term = s_reset * exp(drift2 + vol2 * s * z2);
            (phi * (s_term - spec.moneyness * s_reset)).max(0.0)
        };
        stats.push(0.5 * (payoff(1.0) + payoff(-1.0)));
    }
    (df * stats.mean, df * stats.std_error())
}

/// (i) Closed form == converged independent MC across a sweep of `m / t₁ / T /
/// vol` for both calls and puts.
#[test]
fn closed_form_matches_independent_mc() {
    for &(spot, vol, r_dom, r_for) in &[
        (1.30, 0.10, 0.03, 0.01),
        (100.0, 0.20, 0.05, 0.02),
        (0.85, 0.15, 0.01, 0.04),
    ] {
        for &(t1, expiry) in &[(0.25, 1.0), (0.5, 1.5), (1.0, 2.0)] {
            for &m in &[0.95, 1.0, 1.05] {
                for opt in [OptionType::Call, OptionType::Put] {
                    let i = inputs(spot, spot, vol, expiry, r_dom, r_for);
                    let spec = ForwardStart {
                        option: opt,
                        moneyness: m,
                        reset: t1,
                        expiry,
                    };
                    let closed = forward_start_price(&(&i).into(), spec);
                    let (mc, se) = forward_start_mc(&i, spec, 400_000, 0x5EED_0001);
                    // Within a few reported MC standard errors (antithetic, so the
                    // se is honest), plus a tiny floor for the near-zero deep-OTM
                    // cases where the absolute se collapses.
                    let tol = 4.0 * se + 1e-4;
                    assert!(
                        (closed - mc).abs() < tol,
                        "forward-start closed {closed} vs MC {mc} (se {se}, tol {tol}) \
                         [spot {spot} vol {vol} t1 {t1} T {expiry} m {m} {opt:?}]"
                    );
                }
            }
        }
    }
}

/// (ii) Degenerate limit: `t₁ → 0` reduces the forward-start exactly (~1e-9) to
/// the plain GK vanilla struck at `m·S₀`.
#[test]
fn reset_to_zero_is_gk_vanilla() {
    let i = inputs(1.30, 1.30, 0.12, 1.0, 0.03, 0.01);
    for &m in &[0.8, 0.9, 1.0, 1.1, 1.25] {
        for opt in [OptionType::Call, OptionType::Put] {
            let fs = forward_start_price(
                &(&i).into(),
                ForwardStart {
                    option: opt,
                    moneyness: m,
                    reset: 0.0,
                    expiry: i.t,
                },
            );
            let plain = vanilla_price(
                opt,
                &VanillaInputs {
                    strike: m * i.spot,
                    ..i
                },
            );
            assert!(
                (fs - plain).abs() <= 1e-9 * plain.abs().max(1.0) + 1e-12,
                "reset→0 forward-start {fs} vs GK vanilla {plain} (m {m} {opt:?})"
            );
        }
    }
}

/// (iii-a) Plain cliquet == exact sum of its per-period forward-start legs
/// (~1e-10).
#[test]
fn plain_cliquet_equals_sum_of_legs() {
    let i = inputs(100.0, 100.0, 0.18, 2.0, 0.04, 0.015);
    for &periods in &[2usize, 4, 8] {
        for opt in [OptionType::Call, OptionType::Put] {
            let c = Cliquet {
                option: opt,
                moneyness: 1.0,
                schedule: CliquetSchedule::equal(periods, i.t),
                local_floor: None,
                local_cap: None,
                global_floor: None,
                global_cap: None,
            };
            let total = cliquet_price_plain(&(&i).into(), &c);
            let d = &c.schedule.dates;
            let mut hand = 0.0;
            for k in 1..d.len() {
                hand += forward_start_price(
                    &(&i).into(),
                    ForwardStart {
                        option: opt,
                        moneyness: 1.0,
                        reset: d[k - 1],
                        expiry: d[k],
                    },
                );
            }
            assert!(
                (total - hand).abs() <= 1e-10 * hand.abs().max(1.0) + 1e-12,
                "cliquet {total} vs Σ legs {hand} ({periods} periods, {opt:?})"
            );
            assert!(total > 0.0);
        }
    }
}

/// (iii-b) The unclamped cliquet MC reconciles with the closed-form
/// sum-of-forward-starts: opening the local clamp and dropping the global clamp,
/// the per-leg-discounted MC is an unbiased estimator of the same quantity and
/// agrees within a few reported standard errors.
#[test]
fn unclamped_cliquet_mc_matches_closed_form() {
    let i = inputs(1.25, 1.25, 0.14, 1.0, 0.025, 0.01);
    let c = Cliquet {
        option: OptionType::Call,
        moneyness: 1.0,
        schedule: CliquetSchedule::equal(4, i.t),
        local_floor: None,
        local_cap: None,
        global_floor: None,
        global_cap: None,
    };
    let closed = cliquet_price_plain(&(&i).into(), &c);
    let mc = cliquet_price_plain_mc(
        &(&i).into(),
        &c,
        CliquetMcConfig {
            pairs: 400_000,
            seed: 0xC119_0007,
        },
    );
    let tol = 4.0 * mc.std_error + 1e-4;
    assert!(
        (closed - mc.price).abs() < tol,
        "closed {closed} vs unclamped MC {} (se {}, tol {tol})",
        mc.price,
        mc.std_error
    );
}

/// (iii-c) The locally-capped cliquet MC reconciles with a from-scratch
/// independent MC of the full clamped, terminal-settled payoff (a genuinely
/// separate code path written here), within a few reported standard errors.
#[test]
fn capped_cliquet_mc_matches_independent_clamped_mc() {
    let i = inputs(100.0, 100.0, 0.20, 1.0, 0.03, 0.01);
    let periods = 4usize;
    let local_floor = 0.0;
    // Cap on the per-unit period return (each leg is scaled by the opening spot):
    // 0.08 is a binding upside clamp over a quarterly period at 20 vol.
    let local_cap = 0.08;
    let c = Cliquet {
        option: OptionType::Call,
        moneyness: 1.0,
        schedule: CliquetSchedule::equal(periods, i.t),
        local_floor: Some(local_floor),
        local_cap: Some(local_cap),
        global_floor: None,
        global_cap: None,
    };
    let cfg = CliquetMcConfig {
        pairs: 300_000,
        seed: 0xCAFE_1234,
    };
    let engine = cliquet_price_capped_mc(&(&i).into(), &c, cfg);

    // Independent from-scratch MC of the same terminal-settled clamped payoff.
    let d = &c.schedule.dates;
    let df_t = exp(-i.r_dom * d[d.len() - 1]);
    let mut stats = Stats::default();
    for path in 0..cfg.pairs as u64 {
        let mut rng = CounterRng::new(cfg.seed, 0, path, 0);
        let z: Vec<f64> = (0..periods).map(|_| inverse_cdf(rng.next_u01())).collect();
        let payoff = |s: f64| {
            let mut acc = 0.0;
            let mut spot = i.spot;
            for k in 1..d.len() {
                let dt = d[k] - d[k - 1];
                let drift = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt;
                let diffusion = i.vol * sqrt(dt) * s * z[k - 1];
                let ratio = exp(drift + diffusion);
                // Forward-start leg: opening spot × clamped per-unit return.
                let ret = (ratio - 1.0).max(local_floor).min(local_cap);
                acc += spot * ret;
                spot *= ratio;
            }
            acc
        };
        stats.push(0.5 * (payoff(1.0) + payoff(-1.0)));
    }
    let oracle = df_t * stats.mean;
    let oracle_se = df_t * stats.std_error();
    let tol = 4.0 * (engine.std_error + oracle_se) + 1e-4;
    assert!(
        (engine.price - oracle).abs() < tol,
        "capped cliquet engine {} vs independent clamped MC {oracle} \
         (engine se {}, oracle se {oracle_se}, tol {tol})",
        engine.price,
        engine.std_error
    );
}

/// (iii-d) A tighter local cap strictly reduces the capped-cliquet value
/// (monotone, structural) under common random numbers.
#[test]
fn tighter_local_cap_reduces_value() {
    let i = inputs(100.0, 100.0, 0.22, 1.0, 0.03, 0.01);
    let mk = |cap: f64| Cliquet {
        option: OptionType::Call,
        moneyness: 1.0,
        schedule: CliquetSchedule::equal(4, i.t),
        local_floor: Some(0.0),
        local_cap: Some(cap),
        global_floor: None,
        global_cap: None,
    };
    let cfg = CliquetMcConfig {
        pairs: 80_000,
        seed: 0xBADC_0FFE,
    };
    // Caps are on the per-unit period return; 0.10 vs 0.04 are both binding at
    // 22 vol over a quarter, with 0.04 strictly tighter.
    let loose = cliquet_price_capped_mc(&(&i).into(), &mk(0.10), cfg);
    let tight = cliquet_price_capped_mc(&(&i).into(), &mk(0.04), cfg);
    assert!(
        tight.price < loose.price,
        "tighter cap {} should be < looser cap {}",
        tight.price,
        loose.price
    );
}
