//! Independent reference oracles for the golden-vector corpus.
//!
//! Every function here computes a product value by a route **independent of the
//! production wire/server path** (`celnet-server::pricer` → `celnet-exotics`). The
//! closed-form families are re-derived here from the Garman-Kohlhagen primitives;
//! the path-dependent families use a `splitmix64` Monte-Carlo reimplementation
//! that is code-disjoint from the production counter-RNG estimator.
//!
//! This module deliberately does **not** depend on `celnet-exotics` (no exotic
//! production pricer is called). It uses only `celnet-core::math` for the scalar
//! special functions (`exp`/`ln`/`sqrt`/`norm_cdf`) — those are the
//! library-agnostic numerical primitives, not a pricing path, and are themselves
//! gated against QuantLib by the vanilla golden table, so a closed form built on
//! them is an independent oracle in exactly the sense the anti-circular rule
//! requires.

use celnet_core::math::{exp, ln, norm_cdf, sqrt};

/// Call/put sign: `+1` for a call, `-1` for a put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cp {
    /// A call (right to buy).
    Call,
    /// A put (right to sell).
    Put,
}

impl Cp {
    /// `+1.0` for a call, `-1.0` for a put.
    #[must_use]
    pub fn sign(self) -> f64 {
        match self {
            Cp::Call => 1.0,
            Cp::Put => -1.0,
        }
    }

    /// Parse the canonical `"CALL"` / `"PUT"` token.
    ///
    /// # Panics
    /// On any other token.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "CALL" => Cp::Call,
            "PUT" => Cp::Put,
            other => panic!("unknown option type `{other}`"),
        }
    }
}

// ===========================================================================
// Closed-form oracles
// ===========================================================================

/// Garman-Kohlhagen European vanilla present value (premium per unit base), the
/// textbook closed form re-derived here independently of `celnet-vanilla`.
#[must_use]
pub fn gk_price(cp: Cp, spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> f64 {
    if t <= 0.0 {
        return (cp.sign() * (spot - strike)).max(0.0);
    }
    let vsqt = vol * sqrt(t);
    let d1 = (ln(spot / strike) + (r_dom - r_for + 0.5 * vol * vol) * t) / vsqt;
    let d2 = d1 - vsqt;
    let df_dom = exp(-r_dom * t);
    let df_for = exp(-r_for * t);
    match cp {
        Cp::Call => spot * df_for * norm_cdf(d1) - strike * df_dom * norm_cdf(d2),
        Cp::Put => strike * df_dom * norm_cdf(-d2) - spot * df_for * norm_cdf(-d1),
    }
}

/// Forward-start vanilla (Rubinstein 1990 FX dual-carry form): the strike fixes at
/// `reset` to `moneyness · S(reset)`. `V = e^{-r_f·t₁}·S₀·u(m, T−t₁)`, with `u`
/// the unit-spot GK vanilla over the residual maturity. Re-derived independently
/// of `celnet-exotics::forward_start_price`.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn forward_start_price(
    cp: Cp,
    spot: f64,
    moneyness: f64,
    reset: f64,
    expiry: f64,
    vol: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    let residual = expiry - reset;
    let unit = if residual <= 0.0 {
        (cp.sign() * (1.0 - moneyness)).max(0.0)
    } else {
        // Unit-spot vanilla: spot 1, strike `moneyness`, maturity `residual`.
        gk_price(cp, 1.0, moneyness, vol, residual, r_dom, r_for)
    };
    exp(-r_for * reset) * spot * unit
}

/// Plain (unclamped) cliquet: the exact sum of `periods` evenly-spaced
/// forward-start legs over `[0, expiry]`. Re-derived independently.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn cliquet_plain_price(
    cp: Cp,
    spot: f64,
    moneyness: f64,
    periods: usize,
    expiry: f64,
    vol: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    let mut total = 0.0;
    for k in 1..=periods {
        let reset = expiry * (k - 1) as f64 / periods as f64;
        let leg_expiry = expiry * k as f64 / periods as f64;
        total += forward_start_price(cp, spot, moneyness, reset, leg_expiry, vol, r_dom, r_for);
    }
    total
}

/// Quanto vanilla: GK with the quanto-drift correction `−ρ·σ_S·σ_Z` folded into
/// the carry (realised as `r_for_adj = r_for + ρ·σ_S·σ_Z`); discount at `r_dom`.
/// Re-derived independently of `celnet-exotics::quanto_vanilla_price`.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn quanto_vanilla_price(
    cp: Cp,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    conversion_vol: f64,
    correlation: f64,
) -> f64 {
    let adjustment = -correlation * vol * conversion_vol; // carry shift
    let r_for_adj = r_for - adjustment; // r_for_new = r_for − adjustment
    gk_price(cp, spot, strike, vol, t, r_dom, r_for_adj)
}

/// Quanto cash-or-nothing digital paying one unit of settlement cash:
/// `e^{-r_dom·T}·Φ(±d₂_Q)` at the quanto-adjusted carry. Re-derived independently.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn quanto_digital_price(
    cp: Cp,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    conversion_vol: f64,
    correlation: f64,
) -> f64 {
    let adjustment = -correlation * vol * conversion_vol;
    let r_for_adj = r_for - adjustment;
    let vsqt = vol * sqrt(t);
    let d1 = (ln(spot / strike) + (r_dom - r_for_adj + 0.5 * vol * vol) * t) / vsqt;
    let d2 = d1 - vsqt;
    let df = exp(-r_dom * t);
    match cp {
        Cp::Call => df * norm_cdf(d2),
        Cp::Put => df * norm_cdf(-d2),
    }
}

/// Floating-strike continuous-monitoring lookback (Goldman-Sosin-Gatto), at
/// inception (running extremum = spot). Re-derived independently of
/// `celnet-exotics::floating_lookback_price`.
#[must_use]
pub fn floating_lookback_price(cp: Cp, spot: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> f64 {
    let s = spot;
    let sst = vol * sqrt(t);
    let b = r_dom - r_for;
    let df_dom = exp(-r_dom * t);
    let df_for = exp(-r_for * t);
    let a1 = (b + 0.5 * vol * vol) * t / sst;
    let a2 = a1 - sst;
    let two_b_over_sig2 = 2.0 * b / (vol * vol);
    match cp {
        Cp::Call => {
            let main = s * df_for * norm_cdf(a1) - s * df_dom * norm_cdf(a2);
            let refl = s
                * df_dom
                * (vol * vol / (2.0 * b))
                * (norm_cdf(-a1 + two_b_over_sig2 * sst) - exp(b * t) * norm_cdf(-a1));
            main + refl
        }
        Cp::Put => {
            let main = s * df_dom * norm_cdf(-a2) - s * df_for * norm_cdf(-a1);
            let refl = s
                * df_dom
                * (vol * vol / (2.0 * b))
                * (-norm_cdf(a1 - two_b_over_sig2 * sst) + exp(b * t) * norm_cdf(a1));
            main + refl
        }
    }
}

/// **At-hit** one-touch present value (Reiner-Rubinstein 1991): pays `rebate` at
/// the moment of the first touch of `barrier`. Re-derived here from the published
/// closed form (independent of `celnet-exotics::one_touch_price`) — the server's
/// wire one-touch uses at-hit timing, so the QuantLib at-expiry touch CSV is *not*
/// the right oracle for it; this is.
///
/// `OT = R·[ (H/S)^{μ+λ}·Φ(η·a₁) + (H/S)^{μ−λ}·Φ(η·a₂) ]`, with `μ = (b−½σ²)/σ²`,
/// `λ = √(μ²+2r_d/σ²)`, `b = r_dom−r_for`, `η = −1` for an upper barrier (above
/// spot) / `+1` for a lower barrier, and `a₁/a₂` the first-passage normal
/// arguments. If spot already sits at/through the barrier the payout is certain
/// (`= rebate`).
#[must_use]
pub fn one_touch_at_hit_price(
    spot: f64,
    barrier: f64,
    rebate: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    let upper = barrier > spot;
    let through = if upper {
        spot >= barrier
    } else {
        spot <= barrier
    };
    if through {
        return rebate; // at-hit, hit is immediate
    }
    let b = r_dom - r_for;
    let sig2 = vol * vol;
    let mu = (b - 0.5 * sig2) / sig2;
    let lam = (mu * mu + 2.0 * r_dom / sig2).sqrt();
    let vsqt = vol * sqrt(t);
    let z = ln(barrier / spot); // >0 upper, <0 lower
    // Side orientation (mirrors the canonical reflected-normal pair): for an upper
    // barrier the running-max tail uses base = −z/vsqt with a +drift; a lower
    // barrier the running-min with base = +z/vsqt and a −drift.
    let side_sign = if upper { -1.0 } else { 1.0 };
    let drift_sign = -side_sign;
    let base = side_sign * z / vsqt;
    let a1 = base + drift_sign * lam * vsqt;
    let a2 = base - drift_sign * lam * vsqt;
    let pow_cdf = |p: f64, arg: f64| -> f64 {
        let phi = norm_cdf(arg);
        if phi <= 0.0 {
            0.0
        } else {
            exp(p * z + ln(phi))
        }
    };
    let v = rebate * (pow_cdf(mu + lam, a1) + pow_cdf(mu - lam, a2));
    let max_df = exp(-r_dom * t).max(1.0);
    v.clamp(0.0, rebate.max(0.0) * max_df)
}

// ===========================================================================
// Code-disjoint Monte-Carlo oracle (path-dependent families)
// ===========================================================================

/// A `splitmix64` PRNG — deliberately distinct from the production
/// counter-based RNG so the Monte-Carlo oracle shares no RNG code with the pricer
/// under test. (Steele, Lea & Flood 2014, "Fast splittable pseudorandom number
/// generators".)
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Seed the generator.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next raw 64-bit value.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Next uniform in the open interval `(0, 1)` (never `0` so `ln` is safe in
    /// Box–Muller).
    #[inline]
    pub fn next_u01(&mut self) -> f64 {
        // 53 significant bits, then nudge off the closed endpoints.
        let bits = self.next_u64() >> 11;
        let u = (bits as f64 + 0.5) * (1.0 / 9_007_199_254_740_992.0);
        u.clamp(f64::MIN_POSITIVE, 1.0 - f64::EPSILON)
    }

    /// A pair of independent standard normals via the Box–Muller transform — an
    /// independent route to Gaussian draws (the production path uses an
    /// inverse-CDF of a counter RNG; this shares neither the RNG nor the
    /// normal-inversion code).
    #[inline]
    pub fn next_normal_pair(&mut self) -> (f64, f64) {
        let u1 = self.next_u01();
        let u2 = self.next_u01();
        let r = sqrt(-2.0 * ln(u1));
        let theta = std::f64::consts::TAU * u2;
        (r * theta.cos(), r * theta.sin())
    }
}

/// A Welford online mean / standard-error-of-the-mean accumulator (independent
/// local copy).
#[derive(Default)]
pub struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    /// Add one observation.
    #[inline]
    pub fn push(&mut self, x: f64) {
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (x - self.mean);
    }

    /// The running mean.
    #[must_use]
    pub fn mean(&self) -> f64 {
        self.mean
    }

    /// The standard error of the mean.
    #[must_use]
    pub fn std_error(&self) -> f64 {
        if self.n < 2 {
            0.0
        } else {
            sqrt(self.m2 / ((self.n - 1) as f64) / self.n as f64)
        }
    }
}

/// A Monte-Carlo estimate: mean price and its standard error.
#[derive(Debug, Clone, Copy)]
pub struct McEstimate {
    /// Discounted price estimate (per unit base / per the family's notional).
    pub price: f64,
    /// Standard error of the mean.
    pub std_error: f64,
}

/// Arithmetic-average-rate fixed-strike Asian by code-disjoint terminal-discretised
/// GBM Monte-Carlo (antithetic). `observations` equally-spaced future fixings.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn asian_arithmetic_mc(
    cp: Cp,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    observations: usize,
    pairs: usize,
    seed: u64,
) -> McEstimate {
    let n = observations.max(1);
    let dt = t / n as f64;
    let drift = (r_dom - r_for - 0.5 * vol * vol) * dt;
    let diff = vol * sqrt(dt);
    let df = exp(-r_dom * t);
    let phi = cp.sign();
    let mut rng = SplitMix64::new(seed);
    let mut acc = Welford::default();
    // Reusable normal buffer, drawn fresh per pair from the disjoint RNG.
    let mut z = vec![0.0f64; n];
    for _ in 0..pairs {
        let mut i = 0;
        while i < n {
            let (a, b) = rng.next_normal_pair();
            z[i] = a;
            if i + 1 < n {
                z[i + 1] = b;
            }
            i += 2;
        }
        let payoff = |sign: f64| -> f64 {
            let mut ln_s = ln(spot);
            let mut sum = 0.0;
            for &zk in &z {
                ln_s += drift + diff * sign * zk;
                sum += exp(ln_s);
            }
            let avg = sum / n as f64;
            (phi * (avg - strike)).max(0.0)
        };
        acc.push(0.5 * (payoff(1.0) + payoff(-1.0)) * df);
    }
    McEstimate {
        price: acc.mean(),
        std_error: acc.std_error(),
    }
}

/// Floating- or fixed-strike lookback by code-disjoint discrete-monitoring GBM
/// Monte-Carlo (antithetic), over `observations` equally-spaced monitoring dates.
/// This is the **discrete** lookback the server prices by Monte-Carlo.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn lookback_discrete_mc(
    floating: bool,
    cp: Cp,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    observations: usize,
    pairs: usize,
    seed: u64,
) -> McEstimate {
    let n = observations.max(1);
    let dt = t / n as f64;
    let drift = (r_dom - r_for - 0.5 * vol * vol) * dt;
    let diff = vol * sqrt(dt);
    let df = exp(-r_dom * t);
    let var_step = vol * vol * dt;
    let mut rng = SplitMix64::new(seed);
    let mut acc = Welford::default();
    // Per-path log-step normals + per-step bridge uniforms. The Brownian-bridge
    // **extremum simulation** (Beaglehole-Dybvig-Zhou 1997 / Glasserman 2003) draws
    // the exact continuous extremum of each [t_{k-1}, t_k] segment from a uniform,
    // removing the O(1/√steps) under-sampling bias of a naive node-max — matching
    // the server's discrete-monitoring convention, derived here independently.
    let mut z = vec![0.0f64; n];
    let mut u = vec![0.0f64; n];
    for _ in 0..pairs {
        for k in 0..n {
            let (a, _b) = rng.next_normal_pair();
            z[k] = a;
            u[k] = rng.next_u01();
        }
        let mut ln_prev = ln(spot);
        let mut run_max = spot;
        let mut run_min = spot;
        let mut terminal = spot;
        for k in 0..n {
            let ln_next = ln_prev + drift + diff * z[k];
            let s_next = exp(ln_next);
            terminal = s_next;
            // Bridge max/min of the segment in log-space:
            //   x_max = ½[(x0+x1) + √((x1−x0)² − 2σ²dt·ln U)]
            //   x_min = ½[(x0+x1) − √((x1−x0)² − 2σ²dt·ln U)]
            let dx = ln_next - ln_prev;
            let disc = (dx * dx - 2.0 * var_step * ln(u[k])).max(0.0).sqrt();
            let seg_max = exp(0.5 * (ln_prev + ln_next + disc));
            let seg_min = exp(0.5 * (ln_prev + ln_next - disc));
            run_max = run_max.max(seg_max);
            run_min = run_min.min(seg_min);
            ln_prev = ln_next;
        }
        let raw = if floating {
            match cp {
                Cp::Call => terminal - run_min,
                Cp::Put => run_max - terminal,
            }
        } else {
            match cp {
                Cp::Call => (run_max - strike).max(0.0),
                Cp::Put => (strike - run_min).max(0.0),
            }
        };
        acc.push(raw.max(0.0) * df);
    }
    McEstimate {
        price: acc.mean(),
        std_error: acc.std_error(),
    }
}

/// TARF redemption (gap-risk) convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TarfRedemption {
    /// The breaching fixing pays its full intrinsic (may overshoot the target).
    FullGain,
    /// The breaching fixing pays only the remaining target (no overshoot).
    CappedGain,
}

/// TARF **bank** present value by code-disjoint Monte-Carlo (antithetic). Mirrors
/// the production payoff semantics (favourable client gains accrue toward `target`
/// and knock the structure out; the adverse leg is geared by `leverage` and
/// received by the bank). `notional` is the per-fixing notional.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn tarf_bank_pv_mc(
    favourable: Cp,
    spot: f64,
    strike: f64,
    target: f64,
    leverage: f64,
    notional: f64,
    redemption: TarfRedemption,
    fixings: usize,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    pairs: usize,
    seed: u64,
) -> McEstimate {
    let n = fixings.max(1);
    let dt = t / n as f64;
    let drift = (r_dom - r_for - 0.5 * vol * vol) * dt;
    let diff = vol * sqrt(dt);
    let mut dfs = vec![0.0f64; n];
    for (k, d) in dfs.iter_mut().enumerate() {
        *d = exp(-r_dom * (k + 1) as f64 * dt);
    }
    let gain_sign = favourable.sign();
    let mut rng = SplitMix64::new(seed);
    let mut acc = Welford::default();
    let mut z = vec![0.0f64; n];
    let walk = |z: &[f64], sign: f64| -> f64 {
        let mut ln_s = ln(spot);
        let mut accumulated = 0.0f64;
        let mut bank_pv = 0.0f64;
        for (k, &zk) in z.iter().enumerate() {
            ln_s += drift + diff * sign * zk;
            let s_k = exp(ln_s);
            let signed = gain_sign * (s_k - strike);
            if signed > 0.0 {
                let remaining = target - accumulated;
                let raw = signed;
                if raw >= remaining {
                    let settled = match redemption {
                        TarfRedemption::FullGain => raw,
                        TarfRedemption::CappedGain => remaining,
                    };
                    bank_pv -= settled * notional * dfs[k];
                    return bank_pv;
                }
                bank_pv -= raw * notional * dfs[k];
                accumulated += raw;
            } else if signed < 0.0 {
                bank_pv += leverage * (-signed) * notional * dfs[k];
            }
        }
        bank_pv
    };
    for _ in 0..pairs {
        let mut i = 0;
        while i < n {
            let (a, b) = rng.next_normal_pair();
            z[i] = a;
            if i + 1 < n {
                z[i + 1] = b;
            }
            i += 2;
        }
        acc.push(0.5 * (walk(&z, 1.0) + walk(&z, -1.0)));
    }
    McEstimate {
        price: acc.mean(),
        std_error: acc.std_error(),
    }
}

/// Accumulator monitoring convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccumulatorMonitoring {
    /// Knock-out checked only at the discrete fixings.
    Discrete,
    /// Knock-out checked continuously via the Brownian-bridge crossing probability
    /// between fixings.
    Continuous,
}

/// Accumulator **client** present value by code-disjoint Monte-Carlo (antithetic).
/// The client accumulates `notional·(S_k − pivot)` per fixing (geared by
/// `leverage` below the pivot) until the up-and-out `barrier` knocks the structure
/// out. The `Discrete` monitoring variant checks the barrier only at fixings.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn accumulator_client_pv_mc(
    spot: f64,
    pivot: f64,
    barrier: f64,
    leverage: f64,
    notional: f64,
    monitoring: AccumulatorMonitoring,
    fixings: usize,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    pairs: usize,
    seed: u64,
) -> McEstimate {
    let n = fixings.max(1);
    let dt = t / n as f64;
    let drift = (r_dom - r_for - 0.5 * vol * vol) * dt;
    let diff = vol * sqrt(dt);
    let mut dfs = vec![0.0f64; n];
    for (k, d) in dfs.iter_mut().enumerate() {
        *d = exp(-r_dom * (k + 1) as f64 * dt);
    }
    let mut rng = SplitMix64::new(seed);
    let mut acc = Welford::default();
    let mut z = vec![0.0f64; n];
    let walk = |z: &[f64], sign: f64| -> f64 {
        let mut ln_s = ln(spot);
        let mut prev = spot;
        let mut client_pv = 0.0f64;
        // Running survival probability across continuous-monitoring fixings. For
        // discrete monitoring it stays 1 until a hard fixing touch.
        let mut survival = 1.0f64;
        for (k, &zk) in z.iter().enumerate() {
            ln_s += drift + diff * sign * zk;
            let s_k = exp(ln_s);
            match monitoring {
                AccumulatorMonitoring::Discrete => {
                    // Hard discrete knock-out at the fixing: no further coupons.
                    if s_k >= barrier {
                        return client_pv;
                    }
                    let coupon = if s_k >= pivot {
                        s_k - pivot
                    } else {
                        -leverage * (pivot - s_k)
                    };
                    client_pv += coupon * notional * dfs[k];
                }
                AccumulatorMonitoring::Continuous => {
                    // Survival-weighted coupon expectation: a fixing that closes at
                    // or above the barrier knocks the path out (survival → 0);
                    // otherwise the in-segment Brownian-bridge survival to `barrier`
                    // over [t_{k-1}, t_k] multiplies the running survival.
                    if s_k >= barrier {
                        return client_pv;
                    }
                    let num = 2.0 * ln(barrier / prev) * ln(barrier / s_k);
                    let seg_survival = 1.0 - exp(-num / (vol * vol * dt));
                    survival *= seg_survival.clamp(0.0, 1.0);
                    let coupon = if s_k >= pivot {
                        s_k - pivot
                    } else {
                        -leverage * (pivot - s_k)
                    };
                    client_pv += coupon * notional * dfs[k] * survival;
                }
            }
            prev = s_k;
        }
        client_pv
    };
    for _ in 0..pairs {
        let mut i = 0;
        while i < n {
            let (a, b) = rng.next_normal_pair();
            z[i] = a;
            if i + 1 < n {
                z[i + 1] = b;
            }
            i += 2;
        }
        acc.push(0.5 * (walk(&z, 1.0) + walk(&z, -1.0)));
    }
    McEstimate {
        price: acc.mean(),
        std_error: acc.std_error(),
    }
}

/// Clamped (locally-capped/floored) cliquet by code-disjoint Monte-Carlo
/// (antithetic), terminal-settled. Each period's per-unit option return
/// `(φ·(ratio − m))⁺` is clamped into the optional local `[floor, cap]`, then the
/// accumulated payoff is bounded into the optional global `[floor, cap]`.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn cliquet_clamped_mc(
    cp: Cp,
    spot: f64,
    moneyness: f64,
    periods: usize,
    local_floor: Option<f64>,
    local_cap: Option<f64>,
    global_floor: Option<f64>,
    global_cap: Option<f64>,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    pairs: usize,
    seed: u64,
) -> McEstimate {
    let n = periods.max(1);
    let dt = t / n as f64;
    let drift = (r_dom - r_for - 0.5 * vol * vol) * dt;
    let diff = vol * sqrt(dt);
    let df = exp(-r_dom * t);
    let phi = cp.sign();
    let mut rng = SplitMix64::new(seed);
    let mut acc = Welford::default();
    let mut z = vec![0.0f64; n];
    let clamped_return = |ratio: f64| -> f64 {
        let mut ret = (phi * (ratio - moneyness)).max(0.0);
        if let Some(f) = local_floor {
            ret = ret.max(f);
        }
        if let Some(c) = local_cap {
            ret = ret.min(c);
        }
        ret
    };
    let walk = |z: &[f64], sign: f64| -> f64 {
        let mut ln_s = ln(spot);
        let mut prev = spot;
        let mut total = 0.0f64;
        for &zk in z {
            ln_s += drift + diff * sign * zk;
            let s = exp(ln_s);
            let ratio = s / prev;
            // The opening-spot scaling: each leg's option return is on the opening
            // spot of the period (so the plain strip is Σ forward-start legs).
            total += clamped_return(ratio) * prev;
            prev = s;
        }
        if let Some(f) = global_floor {
            total = total.max(f);
        }
        if let Some(c) = global_cap {
            total = total.min(c);
        }
        total * df
    };
    for _ in 0..pairs {
        let mut i = 0;
        while i < n {
            let (a, b) = rng.next_normal_pair();
            z[i] = a;
            if i + 1 < n {
                z[i + 1] = b;
            }
            i += 2;
        }
        acc.push(0.5 * (walk(&z, 1.0) + walk(&z, -1.0)));
    }
    McEstimate {
        price: acc.mean(),
        std_error: acc.std_error(),
    }
}

/// One leg of a correlated multi-asset basket.
#[derive(Debug, Clone, Copy)]
pub struct BasketLeg {
    /// Leg spot.
    pub spot: f64,
    /// Leg volatility.
    pub vol: f64,
    /// Leg foreign rate.
    pub r_for: f64,
    /// Leg basket weight.
    pub weight: f64,
}

/// Basket aggregation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasketKind {
    /// Weighted arithmetic basket of the per-leg terminal levels.
    Basket,
    /// Best-of (max) of the per-leg weighted terminal levels.
    BestOf,
    /// Worst-of (min) of the per-leg weighted terminal levels.
    WorstOf,
}

/// Correlated multi-asset basket / best-of / worst-of by code-disjoint
/// Cholesky-correlated terminal GBM Monte-Carlo (antithetic). `correlation` is the
/// row-major N×N matrix; `r_dom` is the shared settlement rate. A single terminal
/// step is exact for these European payoffs.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn basket_mc(
    cp: Cp,
    strike: f64,
    kind: BasketKind,
    legs: &[BasketLeg],
    correlation: &[Vec<f64>],
    t: f64,
    r_dom: f64,
    paths: usize,
    seed: u64,
) -> McEstimate {
    let n = legs.len();
    let chol = cholesky(correlation);
    let df = exp(-r_dom * t);
    let phi = cp.sign();
    let mut rng = SplitMix64::new(seed);
    let mut acc = Welford::default();
    let mut z = vec![0.0f64; n];
    let mut corr_z = vec![0.0f64; n];
    let aggregate = |corr_z: &[f64], sign: f64| -> f64 {
        let mut basket_sum = 0.0;
        let mut best = f64::NEG_INFINITY;
        let mut worst = f64::INFINITY;
        for (a, leg) in legs.iter().enumerate() {
            let drift = (r_dom - leg.r_for - 0.5 * leg.vol * leg.vol) * t;
            let s_t = leg.spot * exp(drift + leg.vol * sqrt(t) * sign * corr_z[a]);
            let w = leg.weight * s_t;
            basket_sum += w;
            best = best.max(w);
            worst = worst.min(w);
        }
        let agg = match kind {
            BasketKind::Basket => basket_sum,
            BasketKind::BestOf => best,
            BasketKind::WorstOf => worst,
        };
        (phi * (agg - strike)).max(0.0)
    };
    for _ in 0..paths {
        let mut i = 0;
        while i < n {
            let (a, b) = rng.next_normal_pair();
            z[i] = a;
            if i + 1 < n {
                z[i + 1] = b;
            }
            i += 2;
        }
        // corr_z = L · z
        for (r, row) in chol.iter().enumerate() {
            let mut acc_r = 0.0;
            for (c, &l) in row.iter().enumerate().take(r + 1) {
                acc_r += l * z[c];
            }
            corr_z[r] = acc_r;
        }
        acc.push(0.5 * (aggregate(&corr_z, 1.0) + aggregate(&corr_z, -1.0)) * df);
    }
    McEstimate {
        price: acc.mean(),
        std_error: acc.std_error(),
    }
}

/// Lower-triangular Cholesky factor `L` of a symmetric positive-definite matrix
/// (`A = L·Lᵀ`).
///
/// # Panics
/// If the matrix is not positive-definite.
#[allow(clippy::needless_range_loop)] // the classic doubly-indexed Cholesky recurrence reads clearest with explicit i/j/k indices
fn cholesky(a: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = a.len();
    let mut l = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut sum = a[i][j];
            for k in 0..j {
                sum -= l[i][k] * l[j][k];
            }
            if i == j {
                assert!(sum > 0.0, "correlation matrix is not positive-definite");
                l[i][j] = sqrt(sum);
            } else {
                l[i][j] = sum / l[j][j];
            }
        }
    }
    l
}

/// Window knock-out barrier (single, up-and-out or down-and-out, active only on
/// `[window_start, window_end] ⊆ [0, T]`) by code-disjoint discrete-monitoring GBM
/// Monte-Carlo (antithetic). The underlying terminal payoff is a vanilla on
/// `strike`; a touch of `barrier` while the window is active extinguishes it.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn window_barrier_mc(
    cp: Cp,
    up: bool,
    spot: f64,
    strike: f64,
    barrier: f64,
    window_start: f64,
    window_end: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    steps: usize,
    pairs: usize,
    seed: u64,
) -> McEstimate {
    let n = steps.max(2);
    let dt = t / n as f64;
    let drift = (r_dom - r_for - 0.5 * vol * vol) * dt;
    let diff = vol * sqrt(dt);
    let df = exp(-r_dom * t);
    let phi = cp.sign();
    let mut rng = SplitMix64::new(seed);
    let mut acc = Welford::default();
    let mut z = vec![0.0f64; n];
    let walk = |z: &[f64], sign: f64| -> f64 {
        let mut ln_s = ln(spot);
        let mut terminal = spot;
        let mut knocked = false;
        for (k, &zk) in z.iter().enumerate() {
            ln_s += drift + diff * sign * zk;
            let s = exp(ln_s);
            terminal = s;
            let time = (k + 1) as f64 * dt;
            let in_window = time >= window_start && time <= window_end;
            if in_window {
                let touch = if up { s >= barrier } else { s <= barrier };
                if touch {
                    knocked = true;
                }
            }
        }
        if knocked {
            0.0
        } else {
            (phi * (terminal - strike)).max(0.0) * df
        }
    };
    for _ in 0..pairs {
        let mut i = 0;
        while i < n {
            let (a, b) = rng.next_normal_pair();
            z[i] = a;
            if i + 1 < n {
                z[i + 1] = b;
            }
            i += 2;
        }
        acc.push(0.5 * (walk(&z, 1.0) + walk(&z, -1.0)));
    }
    McEstimate {
        price: acc.mean(),
        std_error: acc.std_error(),
    }
}
