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
// Linear (discounted-cashflow) oracles — independent of `celnet-linear`
// ===========================================================================
//
// The linear FX book (outright forward / swap / NDF) is priced by the production
// `celnet-linear` crate as `side · notional · df · (F − K)` with
// `F = spot · forward_factor(t)` and `df = discount_df(t)`. This module computes
// the SAME products by a genuinely different route — the two-zero-coupon-bond
// decomposition re-derived here from the raw rates, never forming `F` or `df`:
//
//   A long forward = long a base-currency discount bond worth `spot·e^{−r_for·t}`
//   minus `K` quote-currency discount bonds worth `K·e^{−r_dom·t}`. Hence
//
//       PV = side · notional · ( spot·e^{−r_for·t}  −  K·e^{−r_dom·t} )          (★)
//
// This is algebraically identical to the production form (CIP: `F =
// spot·e^{(r_dom−r_for)t}`, so `df·(F−K) = spot·e^{−r_for·t} − K·e^{−r_dom·t}`),
// but the ORACLE route goes straight to the two discount-bond legs — a different
// rounding path that does not reuse the carry-factor / discount-factor product.
// A forward/discount/sign slip in the production path surfaces as a disagreement
// here. (This mirrors the independent oracle in `celnet-linear`'s own unit tests,
// but is duplicated here so the golden corpus is self-contained and never calls
// the crate under test — the anti-circular-oracle rule.)

/// Present value of an FX outright forward by the independent two-discount-bond
/// route `(★)`: `side · notional · (spot·e^{−r_for·t} − K·e^{−r_dom·t})`, in the
/// quote (settlement) numeraire. `side` is `+1` for a buy (long the base
/// forward), `−1` for a sell.
#[must_use]
pub fn fx_forward_pv(
    side: f64,
    spot: f64,
    strike: f64,
    notional: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    side * notional * (spot * exp(-r_for * t) - strike * exp(-r_dom * t))
}

/// Present value of an FX swap: a near leg at `side` settling at `near_t` plus a
/// far leg at the **opposite** side settling at `far_t`, each an independent
/// outright forward `(★)` at the same contract `strike`/`spot`/rates. By market
/// convention the far leg trades opposite the near leg.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn fx_swap_pv(
    near_side: f64,
    spot: f64,
    strike: f64,
    notional: f64,
    near_t: f64,
    far_t: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    let near = fx_forward_pv(near_side, spot, strike, notional, near_t, r_dom, r_for);
    let far = fx_forward_pv(-near_side, spot, strike, notional, far_t, r_dom, r_for);
    near + far
}

/// The swap points — the far outright forward minus the near outright forward
/// `spot·(e^{(r_dom−r_for)·far_t} − e^{(r_dom−r_for)·near_t})` — re-derived here
/// directly from the CIP carry factor (an independent identity used to
/// cross-check the production `swap_points`).
#[must_use]
pub fn fx_swap_points(spot: f64, near_t: f64, far_t: f64, r_dom: f64, r_for: f64) -> f64 {
    let b = r_dom - r_for;
    spot * (exp(b * far_t) - exp(b * near_t))
}

/// Present value of a non-deliverable forward, identical risk-neutral PV to a
/// deliverable outright forward `(★)` in the same (convertible/settlement)
/// numeraire — non-deliverability changes settlement mechanics, not the PV. The
/// fixing identity is metadata and does not enter the value.
#[must_use]
pub fn ndf_pv(
    side: f64,
    spot: f64,
    strike: f64,
    notional: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    fx_forward_pv(side, spot, strike, notional, t, r_dom, r_for)
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

/// **At-hit** one-touch present value: pays `rebate` at the moment of the first
/// touch of `barrier`. The server's wire one-touch uses at-hit timing, so the
/// QuantLib at-expiry touch CSV is *not* the right oracle for it; this is.
///
/// **Route — deliberately disjoint from the production closed form**: numerical
/// quadrature of the *discounted first-passage density*
///
/// ```text
///   OT = R·∫₀ᵀ e^{−r_dom·t}·f_τ(t) dt,
///   f_τ(t) = |z| / (σ·√(2π)·t^{3/2}) · exp(−(z − ν·t)² / (2σ²t)),
///   z = ln(H/S),   ν = (r_dom − r_for) − ½σ²,
/// ```
///
/// the reflection-principle density of the first passage of drifted Brownian
/// motion to a single level (Shreve 2004 §8.3; provenance doc-only). No
/// `λ`-exponent, no `Φ`, no power×CDF pairing appears anywhere in this route, so
/// the closed form's historical failure mode — pairing `(H/S)^{μ±λ}` with the
/// wrong CDF argument, the P0 at-hit defect that a previous revision of this
/// oracle inherited as a structural copy of the engine — *cannot be expressed
/// here*. The oracle is structurally able to disagree with `celnet-exotics`,
/// which is exactly what the anti-circularity rule requires.
///
/// Accuracy: panel-doubling stable to the last bit and agreeing with the
/// corrected published closed form (double-precision `erfc` evaluation) to
/// ~2e-15 relative on the frozen touch markets — see the unit tests. If spot
/// already sits at/through the barrier the payout is certain (`= rebate`).
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
    // Sideless convention, mirroring the engine's `TouchSide::from_levels`
    // (`barrier >= spot ⇒ Upper`): with only (spot, barrier) on the signature a
    // "breached" state is inexpressible — spot strictly beyond the level simply
    // means the OTHER side's alive contract. The touch is certain exactly at
    // `spot == barrier` (boundary continuity is law-tested in the engine).
    if spot == barrier {
        return rebate; // at-hit: the touch is immediate and certain
    }
    let z = ln(barrier / spot); // >0 upper, <0 lower
    let nu = (r_dom - r_for) - 0.5 * vol * vol;
    rebate * discounted_first_passage_integral(z, nu, vol, r_dom, t, 48)
}

/// `∫₀ᵀ e^{−r·t}·f_τ(t) dt` for the first passage of `ν·t + σ·W_t` to level `z`,
/// by composite fixed-order quadrature: the 24-node Gauss-Legendre rule
/// ([`QUAD_NODES`]/[`QUAD_WEIGHTS`]) on each geometric panel `[T/2^{k+1}, T/2^k]`,
/// `k = 0..panels`, refining toward `t → 0` where the density is smooth but
/// increasingly steep (it vanishes as `e^{−z²/(2σ²t)}`). The untouched
/// `[0, T/2^panels]` head contributes less than `e^{−z²·2^{panels−1}/(2σ²T)}` —
/// identically zero in double precision at the default 48 panels for any market
/// in the corpus. Exposed with an explicit `panels` so the self-tests can assert
/// panel-doubling convergence.
fn discounted_first_passage_integral(
    z: f64,
    nu: f64,
    vol: f64,
    r: f64,
    t: f64,
    panels: u32,
) -> f64 {
    let scale = z.abs() / (vol * sqrt(2.0 * std::f64::consts::PI));
    // e^{−r·t}·f_τ(t); the exponent is evaluated first and cut at −700 (below
    // e^{−700} ≈ 1e−304 the term cannot move a double-precision sum, and the cut
    // prevents the `t^{−3/2}` prefactor from manufacturing `Inf·0`).
    let integrand = |tt: f64| -> f64 {
        if tt <= 0.0 {
            return 0.0;
        }
        let dev = z - nu * tt;
        let e = -dev * dev / (2.0 * vol * vol * tt) - r * tt;
        if e < -700.0 {
            return 0.0;
        }
        scale * exp(e) / (tt * sqrt(tt))
    };
    let mut total = 0.0;
    let mut hi = t;
    for _ in 0..panels {
        let lo = 0.5 * hi;
        let c = 0.5 * (lo + hi);
        let h = 0.5 * (hi - lo);
        for (x, w) in QUAD_NODES.iter().zip(QUAD_WEIGHTS.iter()) {
            total += w * h * integrand(c + h * x);
        }
        hi = lo;
    }
    total
}

/// 24-node Gauss-Legendre abscissae on `[−1, 1]` (symmetric Newton-refined
/// Legendre-polynomial roots; the matching weights are `2/((1−x²)·P′ₙ(x)²)`).
const QUAD_NODES: [f64; 24] = [
    0.995_187_219_997_021_3,
    0.974_728_555_971_309_5,
    0.938_274_552_002_732_8,
    0.886_415_527_004_401_1,
    0.820_001_985_973_903,
    0.740_124_191_578_554_4,
    0.648_093_651_936_975_5,
    0.545_421_471_388_839_6,
    0.433_793_507_626_045_2,
    0.315_042_679_696_163_4,
    0.191_118_867_473_616_3,
    0.064_056_892_862_605_63,
    -0.064_056_892_862_605_63,
    -0.191_118_867_473_616_3,
    -0.315_042_679_696_163_4,
    -0.433_793_507_626_045_2,
    -0.545_421_471_388_839_6,
    -0.648_093_651_936_975_5,
    -0.740_124_191_578_554_4,
    -0.820_001_985_973_903,
    -0.886_415_527_004_401_1,
    -0.938_274_552_002_732_8,
    -0.974_728_555_971_309_5,
    -0.995_187_219_997_021_3,
];

/// The weights paired with [`QUAD_NODES`] (sum = 2 to 4 ulp).
const QUAD_WEIGHTS: [f64; 24] = [
    0.012_341_229_799_985_648,
    0.028_531_388_628_933_813,
    0.044_277_438_817_419_676,
    0.059_298_584_915_436_66,
    0.073_346_481_411_080_27,
    0.086_190_161_531_953_22,
    0.097_618_652_104_113_68,
    0.107_444_270_115_965_62,
    0.115_505_668_053_725_61,
    0.121_670_472_927_803_35,
    0.125_837_456_346_828_39,
    0.127_938_195_346_752_24,
    0.127_938_195_346_752_24,
    0.125_837_456_346_828_39,
    0.121_670_472_927_803_35,
    0.115_505_668_053_725_61,
    0.107_444_270_115_965_62,
    0.097_618_652_104_113_68,
    0.086_190_161_531_953_22,
    0.073_346_481_411_080_27,
    0.059_298_584_915_436_66,
    0.044_277_438_817_419_676,
    0.028_531_388_628_933_813,
    0.012_341_229_799_985_648,
];

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

// ===========================================================================
// Cross-asset closed-form oracles (equity / commodity / crypto)
// ===========================================================================
//
// These price the SAME `vanilla` product arm of the `celnet.proto` `oneof product`
// seen through a NON-FX `Underlying.ref` arm (equity / commodity / digital-asset),
// each routed through the asset-class-agnostic cost-of-carry seam (ADR-0008). The
// production leaves under test are `celnet-equity-vanilla` (generalized BSM,
// `b = r − q − repo`), `celnet-commodity-vanilla` (Black-76, `b = 0` on a future /
// `b = r − convenience` on spot) and `celnet-crypto-vanilla` (linear funded BSM +
// the inverse / coin-margined `1/S_T` closed form).
//
// ## Why these oracles are CODE-DISJOINT from the leaves (anti-circular rule)
//
// The leaves compute the normal CDF via `celnet_core::math::norm_cdf`, which is
// `½·erfc(−x/√2)` over **`libm::erfc`**. The oracles below deliberately route the
// normal CDF through a DIFFERENT special function — `½·(1 + erf(x/√2))` over
// **`libm::erf`** — and call `libm::{exp,log,sqrt}` directly rather than the
// `celnet_core::math` wrappers. `erf` and `erfc` are independent library routines
// (different polynomial/rational branches; `erfc` is *not* internally `1 − erf`
// in the deep tail), so a closed form built on `xerf` shares no arithmetic with the
// production `norm_cdf` path — exactly the independence the FRTB-0.75ρ
// circular-oracle lesson requires. The closed-form algebra itself is re-derived
// here from each model's primary source (Merton 1973 / Black 1976 / the `1/S_T`
// risk-neutral expectation), not read back from any leaf crate (`celnet-golden`
// does not depend on the equity/commodity/crypto leaves).

/// Standard-normal CDF via the **erf** route `½·(1 + erf(x/√2))` — deliberately a
/// DIFFERENT special function than the production `norm_cdf` (which is
/// `½·erfc(−x/√2)` over `erfc`), so the cross-asset oracle is code-disjoint from
/// the leaves under test.
#[inline]
#[must_use]
pub fn xerf_norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + libm::erf(x * std::f64::consts::FRAC_1_SQRT_2))
}

/// **Generalized Black-Scholes-Merton (Merton 1973) equity vanilla** present value,
/// re-derived independently of `celnet-equity-vanilla`. The net cost of carry is
/// `b = r − q − repo`; the forward is `F = S·e^{b·t}`, the discount `e^{−r·t}`:
///
/// ```text
/// d1 = [ln(S/K) + (b + ½σ²)·t] / (σ√t),   d2 = d1 − σ√t
/// Call = S·e^{(b−r)t}·Φ(d1) − K·e^{−r t}·Φ(d2)
/// Put  = K·e^{−r t}·Φ(−d2) − S·e^{(b−r)t}·Φ(−d1)
/// ```
///
/// `Φ` is the **erf-route** CDF [`xerf_norm_cdf`]; `exp`/`ln`/`sqrt` are called on
/// `libm` directly. Code-disjoint from the leaf's `erfc`-route arithmetic.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn equity_bsm_price(
    cp: Cp,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r: f64,
    q: f64,
    repo: f64,
) -> f64 {
    let b = r - q - repo;
    if t <= 0.0 {
        return (cp.sign() * (spot - strike)).max(0.0);
    }
    let vsqt = vol * libm::sqrt(t);
    let d1 = (libm::log(spot / strike) + (b + 0.5 * vol * vol) * t) / vsqt;
    let d2 = d1 - vsqt;
    let s_disc = spot * libm::exp((b - r) * t);
    let k_disc = strike * libm::exp(-r * t);
    match cp {
        Cp::Call => s_disc * xerf_norm_cdf(d1) - k_disc * xerf_norm_cdf(d2),
        Cp::Put => k_disc * xerf_norm_cdf(-d2) - s_disc * xerf_norm_cdf(-d1),
    }
}

/// **Black-76 (1976) commodity option** present value on a forward/future `f`,
/// re-derived independently of `celnet-commodity-vanilla`. Black's model is the
/// `b = 0` degenerate of generalized BSM under the futures measure:
///
/// ```text
/// d1 = [ln(F/K) + ½σ²·t] / (σ√t),   d2 = d1 − σ√t
/// Call = e^{−r t}·[ F·Φ(d1) − K·Φ(d2) ]
/// Put  = e^{−r t}·[ K·Φ(−d2) − F·Φ(−d1) ]
/// ```
///
/// The caller passes the **forward** `f` directly (for a listed future this is the
/// futures price; for the spot representation it is `S·e^{b·t}`), so this oracle is
/// reached by a route that never re-forms the leaf's `spot · forward_factor(t)`.
/// `Φ` is the erf-route CDF; transcendentals via `libm` directly.
#[must_use]
pub fn black76_price(cp: Cp, f: f64, strike: f64, vol: f64, t: f64, r: f64) -> f64 {
    if t <= 0.0 {
        return libm::exp(-r * t) * (cp.sign() * (f - strike)).max(0.0);
    }
    let vsqt = vol * libm::sqrt(t);
    let d1 = (libm::log(f / strike) + 0.5 * vol * vol * t) / vsqt;
    let d2 = d1 - vsqt;
    let df = libm::exp(-r * t);
    match cp {
        Cp::Call => df * (f * xerf_norm_cdf(d1) - strike * xerf_norm_cdf(d2)),
        Cp::Put => df * (strike * xerf_norm_cdf(-d2) - f * xerf_norm_cdf(-d1)),
    }
}

/// **Linear (USD-margined) crypto vanilla** present value (USD per USD-notional-1),
/// re-derived independently of `celnet-crypto-vanilla::linear`. The funding carry is
/// `b = r − funding`; this is the generalized-BSM forward-space form, written here
/// in forward space `df·[F·Φ(d1) − K·Φ(d2)]` (NOT the leaf's spot-space
/// `S·e^{−r_for t}·Φ(d1) − K·e^{−r_dom t}·Φ(d2)` operation order), with the
/// erf-route CDF — a genuinely different rounding path than the leaf, which is
/// itself `to_bits`-tied to the FX leaf.
#[must_use]
pub fn crypto_linear_price(
    cp: Cp,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r: f64,
    funding: f64,
) -> f64 {
    let b = r - funding;
    let f = spot * libm::exp(b * t);
    black76_price(cp, f, strike, vol, t, r)
}

/// **Inverse / coin-margined crypto vanilla** present value (COINS per
/// USD-notional-1), re-derived independently of `celnet-crypto-vanilla::inverse`
/// from the USD risk-neutral expectation of the `1/S_T`-weighted payoff
/// `df·max(φ(S_T−K),0)/S_T`. The funding carry is `b = r − funding`,
/// `F = S·e^{b·t}`, `df = e^{−r·t}`:
///
/// ```text
/// d1 = [ln(F/K) + ½σ²t]/(σ√t),  d2 = d1 − σ√t,  d3 = d1 − 2σ√t
/// V_coin = φ·df·[ Φ(φ·d2) − (K/F)·e^{σ²t}·Φ(φ·d3) ]   coins.
/// ```
///
/// This is the genuine non-linear convexity transform (the `e^{σ²t}` measure
/// factor and the doubly-shifted `Φ(d3)`), NOT a `V_lin/S₀` rescale. `Φ` is the
/// erf-route CDF; transcendentals via `libm` directly. Code-disjoint from the leaf.
#[must_use]
pub fn crypto_inverse_price(
    cp: Cp,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r: f64,
    funding: f64,
) -> f64 {
    let b = r - funding;
    let f = spot * libm::exp(b * t);
    let df = libm::exp(-r * t);
    let s2t = vol * vol * t;
    let vsqt = vol * libm::sqrt(t);
    let d1 = (libm::log(f / strike) + 0.5 * s2t) / vsqt;
    let d2 = d1 - vsqt;
    let d3 = d1 - 2.0 * vsqt;
    let amp = (strike / f) * libm::exp(s2t); // (K/F)·e^{σ²t}
    let phi = cp.sign();
    phi * df * (xerf_norm_cdf(phi * d2) - amp * xerf_norm_cdf(phi * d3))
}

// ===========================================================================
// New-product-arm closed-form oracles (proto arms 30/31: perpetual American /
// option on a listed future)
// ===========================================================================
//
// Both oracles below are CODE-DISJOINT from the production engines under test
// (`celnet-exotics::perpetual` and `celnet-commodity-vanilla`), per the
// anti-circular rule:
//
// * the listed-future oracle routes the normal CDF through `libm::erf`
//   ([`xerf_norm_cdf`]) — a different special function than the engines'
//   `libm::erfc`-based `celnet_core::math::norm_cdf`;
// * the perpetual oracle solves the characteristic quadratic by
//   expanding-bracket **bisection in the `y·(y−1)` product form** and completes
//   the power closed form with `libm::pow` — a different float route than the
//   production engine's standard-form discriminant + cancellation-free root
//   pairing + `exp(y·ln x)` seam power.

/// **Futures-style (daily-margined) Black (1976) option on a future**: the
/// UNDISCOUNTED Black expectation
///
/// ```text
/// d1 = [ln(F/K) + ½σ²·t] / (σ√t),   d2 = d1 − σ√t
/// Call = F·Φ(d1) − K·Φ(d2)        Put = K·Φ(−d2) − F·Φ(−d1)
/// ```
///
/// Under futures-style premium margining the option premium is itself margined
/// daily like the future, so no money is financed over the option's life and the
/// fair value carries **no discount factor** (`df ≡ 1` written out directly —
/// never the discounted [`black76_price`] divided back by `e^{−rt}`, which would
/// add a spurious divide to the float route). Put-call parity holds undiscounted:
/// `C − P = F − K`. `Φ` is the **erf-route** CDF [`xerf_norm_cdf`];
/// transcendentals via `libm` directly — code-disjoint from the production
/// `erfc`-route engine.
#[must_use]
pub fn black76_undiscounted_price(cp: Cp, f: f64, strike: f64, vol: f64, t: f64) -> f64 {
    if t <= 0.0 {
        return (cp.sign() * (f - strike)).max(0.0);
    }
    let vsqt = vol * libm::sqrt(t);
    let d1 = (libm::log(f / strike) + 0.5 * vol * vol * t) / vsqt;
    let d2 = d1 - vsqt;
    match cp {
        Cp::Call => f * xerf_norm_cdf(d1) - strike * xerf_norm_cdf(d2),
        Cp::Put => strike * xerf_norm_cdf(-d2) - f * xerf_norm_cdf(-d1),
    }
}

/// **Perpetual (no-expiry) American vanilla** present value under lognormal
/// cost-of-carry dynamics (discount rate `r ≥ 0`, net carry `b`, vol `σ > 0`),
/// re-derived independently of `celnet-exotics::perpetual` from the stationary
/// pricing ODE's power solutions (McKean 1965; Merton 1973; the cost-of-carry
/// generalization as in Haug 2007, 2nd ed. — provenance in docs only):
///
/// ```text
/// ψ(y) = ½σ²·y·(y−1) + b·y − r = 0 ,
/// call: y₁ > 1 (exists iff b < r),  S*  = K·y₁/(y₁−1),  V = (S*−K)·(S/S*)^{y₁}
/// put:  y₂ < 0,                     S** = K·y₂/(y₂−1),  V = (K−S**)·(S/S**)^{y₂}
/// ```
///
/// with `V` = intrinsic beyond the free boundary, and the exact degenerate arms:
/// a call with `b == r` **exactly** is never exercised (`ψ(1) = 0` makes
/// `y₁ = 1`, so `V = S` — the `T → ∞` limit of the same-terms European call); a
/// put with `r = 0` has the exact factorization `ψ(y) = y·(½σ²·(y−1) + b)`, so
/// `y₂ = 1 − 2b/σ²` when `b > ½σ²` and otherwise the boundary collapses and
/// `V = K`.
///
/// A call with `b > r` **strictly** has NO finite value — stopping at any level
/// `L > K` is worth `(L − K)·(S/L)^{y₁}` with `y₁ < 1` (`ψ(1) = b − r > 0` puts
/// the larger root below 1), unbounded as `L → ∞` (`e^{−rt}·S_t` is a strict
/// submartingale) — so the oracle **refuses** (`None`), exactly where the
/// production engine refuses with its typed error: the oracle must never
/// produce a number the engine refuses. Puts are unaffected (the `y₂ ≤ 0`
/// branch exists for every `b`; the put payoff is bounded by `K`).
///
/// INDEPENDENT route (anti-circular): the root is found by expanding-bracket
/// **bisection of ψ in the `y·(y−1)` product form** (200 halvings reach machine
/// precision) and the value is completed with `libm::pow` — sharing neither the
/// production engine's standard-form quadratic discriminant + cancellation-free
/// pairing nor its `exp(y·ln x)` power route.
#[must_use]
pub fn perpetual_american_price(
    cp: Cp,
    spot: f64,
    strike: f64,
    vol: f64,
    r: f64,
    b: f64,
) -> Option<f64> {
    let psi = |y: f64| 0.5 * vol * vol * y * (y - 1.0) + b * y - r;
    // Bisection with the sign invariant ψ(neg) ≤ 0 < ψ(pos) (the bracket may be
    // numerically reversed — only the signs matter). 200 halvings exhaust f64.
    let bisect = |mut neg: f64, mut pos: f64| -> f64 {
        for _ in 0..200 {
            let mid = 0.5 * (neg + pos);
            if psi(mid) <= 0.0 {
                neg = mid;
            } else {
                pos = mid;
            }
        }
        0.5 * (neg + pos)
    };
    Some(match cp {
        Cp::Call => {
            // b > r STRICTLY: the perpetual call diverges (no finite value) —
            // the oracle refuses, mirroring the engine's typed refusal.
            if b > r {
                return None;
            }
            // b == r EXACTLY (structural comparison): ψ(1) = 0 makes y₁ = 1 —
            // early exercise is never optimal and the value is the spot itself
            // (the exact y₁ → 1⁺ limit, also the T → ∞ European-call limit).
            if b == r {
                return Some(spot);
            }
            // y₁ > 1 since ψ(1) = b − r < 0 here; expand the upper bracket.
            let mut hi = 2.0;
            while psi(hi) <= 0.0 {
                hi *= 2.0;
            }
            let y = bisect(1.0, hi);
            // In the sub-ulp window b ∈ (r − O(ulp·r), r) the bisected root
            // collapses to exactly 1.0 even though b < r strictly: the exact
            // arm is the y₁ → 1⁺ limit (V = spot, no finite boundary) — the
            // same law as b == r, never a 1/0 boundary evaluation. (Exact
            // structural comparison; the bisection's invariant keeps y ≥ 1.)
            if y == 1.0 {
                return Some(spot);
            }
            let boundary = strike * y / (y - 1.0);
            if spot >= boundary {
                return Some(spot - strike); // stopped: immediate exercise, intrinsic
            }
            (boundary - strike) * libm::pow(spot / boundary, y)
        }
        Cp::Put => {
            let y = if r == 0.0 {
                // Exact factorization ψ(y) = y·(½σ²·(y−1) + b): the small root
                // is 0 when b ≤ ½σ² (boundary collapses to the origin, value =
                // the unattained supremum K), else 1 − 2b/σ².
                if b <= 0.5 * vol * vol {
                    return Some(strike);
                }
                1.0 - 2.0 * b / (vol * vol)
            } else {
                // y₂ < 0 since ψ(0) = −r < 0; expand the lower bracket.
                let mut lo = -2.0;
                while psi(lo) <= 0.0 {
                    lo *= 2.0;
                }
                bisect(0.0, lo)
            };
            let boundary = strike * y / (y - 1.0); // ∈ (0, K) for y < 0
            if spot <= boundary {
                return Some(strike - spot); // stopped: immediate exercise, intrinsic
            }
            (strike - boundary) * libm::pow(spot / boundary, y)
        }
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// **P0 pairing-regression pin + convergence.** The density quadrature
    /// reproduces the corrected at-hit values on the frozen `touch-1` /
    /// `touch-3` markets (cross-checked against the published closed form,
    /// `erfc` double evaluation, agreement ~2e-15 relative), and doubling the
    /// geometric panel count moves nothing. The historical flipped-pairing
    /// values — 0.0561744592212154 (+28.3%) and 0.00011562220480909844 (×2.08),
    /// frozen into the corpus by the structural-copy oracle this route
    /// replaced — are excluded by construction.
    #[test]
    fn at_hit_quadrature_pins_corrected_values_and_is_converged() {
        let t = 0.082_191_780_821_917_8;
        let v105 = one_touch_at_hit_price(100.0, 105.0, 1.0, 0.08, t, 0.05, 0.01);
        assert!(
            (v105 - 0.043_780_187_274_957_35).abs() <= 1e-13 * v105,
            "touch-1 at-hit: got {v105}"
        );
        let v110 = one_touch_at_hit_price(100.0, 110.0, 1.0, 0.08, t, 0.05, 0.01);
        assert!(
            (v110 - 5.547_023_411_149_521_4e-5).abs() <= 1e-12 * v110,
            "touch-3 at-hit: got {v110}"
        );
        // Panel-doubling convergence (48 → 96) on the touch-1 market.
        let z = ln(105.0 / 100.0);
        let nu = (0.05 - 0.01) - 0.5 * 0.08 * 0.08;
        let i48 = discounted_first_passage_integral(z, nu, 0.08, 0.05, t, 48);
        let i96 = discounted_first_passage_integral(z, nu, 0.08, 0.05, t, 96);
        assert!(
            (i48 - i96).abs() <= 1e-15,
            "panel doubling moved the integral: {i48} vs {i96}"
        );
    }

    /// **Zero-rate identity:** with `r = 0` the discounted first-passage
    /// integral is the plain hit probability, which has its own independent
    /// reflection-principle CDF form (drift `ν`, Girsanov factor `e^{2νz/σ²}`
    /// — *not* the λ machinery the engine's at-hit form uses). Both barrier
    /// sides are exercised.
    #[test]
    fn zero_rate_quadrature_recovers_reflection_hit_probability() {
        let (s, vol, t, r_for) = (100.0, 0.2, 2.0, 0.01);
        let nu = (0.0 - r_for) - 0.5 * vol * vol;
        let vsqt = vol * sqrt(t);
        // Lower barrier (z < 0): P = Φ((z−νT)/σ√T) + e^{2νz/σ²}·Φ((z+νT)/σ√T).
        let z_lo = ln(90.0 / s);
        let q_lo = discounted_first_passage_integral(z_lo, nu, vol, 0.0, t, 48);
        let p_lo = norm_cdf((z_lo - nu * t) / vsqt)
            + exp(2.0 * nu * z_lo / (vol * vol)) * norm_cdf((z_lo + nu * t) / vsqt);
        assert!((q_lo - p_lo).abs() <= 1e-12, "lower: {q_lo} vs {p_lo}");
        // Upper barrier (z > 0): P = Φ((−z+νT)/σ√T) + e^{2νz/σ²}·Φ((−z−νT)/σ√T).
        let z_up = ln(115.0 / s);
        let q_up = discounted_first_passage_integral(z_up, nu, vol, 0.0, t, 48);
        let p_up = norm_cdf((-z_up + nu * t) / vsqt)
            + exp(2.0 * nu * z_up / (vol * vol)) * norm_cdf((-z_up - nu * t) / vsqt);
        assert!((q_up - p_up).abs() <= 1e-12, "upper: {q_up} vs {p_up}");
    }

    /// At `spot == barrier` the touch is certain and the at-hit rebate pays
    /// immediately (undiscounted). Spot strictly beyond the level is NOT a
    /// "breach" under the sideless signature — it is the other side's alive
    /// contract (the engine's `TouchSide::from_levels` convention), so it must
    /// price strictly inside `(0, rebate)`.
    #[test]
    fn at_hit_at_barrier_pays_rebate_certain() {
        let v = one_touch_at_hit_price(110.0, 110.0, 2.5, 0.1, 1.0, 0.05, 0.01);
        assert!((v - 2.5).abs() < 1e-15);
        // 120 vs a 110 level = an alive LOWER touch, not a breach.
        let alive = one_touch_at_hit_price(120.0, 110.0, 2.5, 0.1, 1.0, 0.05, 0.01);
        assert!(alive > 0.0 && alive < 2.5, "alive lower touch: {alive}");
    }
}
