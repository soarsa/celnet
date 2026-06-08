//! Inverse / coin-margined crypto vanilla — the new payoff shape (the crux).
//!
//! The classic Deribit BTC/ETH option is **inverse** (coin-margined): the premium,
//! the payoff, and the P&L are all denominated in the **base coin**, not in USD. Per
//! USD-notional-1, the contract pays
//!
//! ```text
//! payoff_coin = max(φ(S_T − K), 0) / S_T   coins
//! ```
//!
//! where `S_T` is USD-per-coin at expiry. That `1/S_T` factor makes the payoff a
//! **non-linear function of the terminal price** with a genuine convexity/measure
//! subtlety — it is *not* a rescaled vanilla, and `V_lin / S_0` is **WRONG** (the
//! `Cov(1/S_T, payoff)` convexity term is material). This module derives the exact
//! coin price from first principles and exposes the coin-measure Greeks a
//! coin-margined desk actually hedges with.
//!
//! ## Derivation of the coin price (from the USD risk-neutral measure)
//!
//! Let `S` = USD-per-coin spot, `K` = USD strike, `r` = USD numeraire rate,
//! `b = r − funding` the net carry, `F = S·e^{b·t}` the outright forward,
//! `df = e^{−r·t}`, `φ = +1` call / `−1` put. Under the USD risk-neutral measure
//! `Q` the coin price of the option is the discounted `Q`-expectation of the coin
//! payoff:
//!
//! ```text
//! V_coin = E^Q[ df · max(φ(S_T − K), 0) / S_T ]   coins.   (per USD-notional-1)
//! ```
//!
//! Under `Q`, `S_T = F·exp(−½σ²t + σ√t·Z)`, `Z ~ N(0,1)`. Write the indicator event
//! `φ(S_T − K) > 0`. For the **call** (`φ=+1`):
//!
//! ```text
//! V_coin = df·E^Q[(1 − K/S_T)·𝟙_{S_T>K}]
//!        = df·( Q(S_T>K) − K·E^Q[𝟙_{S_T>K}/S_T] ).
//! ```
//!
//! With `d1 = (ln(F/K) + ½σ²t)/(σ√t)`, `d2 = d1 − σ√t`, `d3 = d1 − 2σ√t = d2 − σ√t`:
//!
//! * `Q(S_T>K) = Φ(d2)` (the standard digital probability).
//! * `1/S_T = (1/F)·exp(½σ²t − σ√t·Z)`. Completing the square inside the Gaussian,
//!   `−½z² + ½σ²t − σ√t·z = −½(z+σ√t)² + σ²t`, so
//!   `E^Q[𝟙_{S_T>K}/S_T] = (1/F)·e^{σ²t}·Φ(d2 − σ√t) = (1/F)·e^{σ²t}·Φ(d3)`.
//!
//! Hence the **call** coin price:
//!
//! ```text
//! V_coin_call = df·[ Φ(d2) − (K/F)·e^{σ²t}·Φ(d3) ].
//! ```
//!
//! The same algebra on `(K/S_T − 1)·𝟙_{S_T<K}` gives the **put**:
//!
//! ```text
//! V_coin_put  = df·[ (K/F)·e^{σ²t}·Φ(−d3) − Φ(−d2) ].
//! ```
//!
//! Unified:
//!
//! ```text
//! V_coin = φ·df·[ Φ(φ·d2) − (K/F)·e^{σ²t}·Φ(φ·d3) ]   coins.
//! ```
//!
//! ## Why this is NOT `V_lin / S_0`
//!
//! The naive rescale assumes `E^Q[payoff/S_T] = E^Q[payoff]/S_0`, i.e. that `1/S_T`
//! and the payoff are uncorrelated and that `E^Q[1/S_T] = 1/S_0`. Both are false:
//! `E^Q[1/S_T] = (1/F)·e^{σ²t} = e^{−bt}·e^{σ²t}/S_0 ≠ 1/S_0`, and `1/S_T` is
//! negatively correlated with a call payoff. The presence of the `e^{σ²t}` factor
//! and the **shifted** `Φ(d3)` term (`d3 = d2 − σ√t`, a second `σ√t` shift versus the
//! linear `Φ(d2)`) is exactly the measure correction the naive form drops.
//!
//! The structural convexity gate is `V_coin·S₀ ≠ V_lin` STRICTLY, with the SIGN set
//! by `Cov(1/S_T, payoff)`: a **call** payoff grows with `S_T` so the `1/S_T` weight
//! down-weights its tail ⇒ `V_coin·S₀ < V_lin`; a **put** payoff grows as `S_T` falls
//! so `1/S_T` up-weights it ⇒ `V_coin·S₀ > V_lin`. A naive `V_lin/S₀` rescale gives
//! exact equality (`0` difference), so this signed gate fails loudly on it (gated in
//! [`tests::convexity_sandwich_vs_linear_is_signed`]).
//!
//! `K → 0` collapses the call to `df·Φ(+∞) = df` coins — the payoff is `S_T/S_T = 1`
//! coin discounted — a clean limit independent of `σ` and `K`, a sanity anchor.
//!
//! ## Coin-measure Greeks
//!
//! A coin-margined desk hedges in coins, so the strip's `price` and sensitivities are
//! in **coins**. They are the partials of `V_coin` (closed form above). The
//! USD-equivalent value `V_coin·S` is exposed as a derived field
//! ([`InverseGreeks::usd_equivalent`]) for cross-asset risk that nets in USD.
//!
//! ## Honest boundary (W3 §9)
//!
//! The payoff math, the measure correctness, and the carry assembly are in-repo and
//! gated three independent ways (a code-disjoint splitmix64 + Box-Muller MC within its
//! reported standard error, a high-resolution deterministic quadrature of the raw
//! `1/S_T`-weighted payoff, and the signed structural convexity sandwich — all three
//! in this module's tests; the parity rows in `celnet-parity` are the coordinator's).
//! Live index-fixing VALUES and venue lifecycle are ENV.

use celnet_core::carry::CarryGreeks;
use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::{Carry, OptionType, RateSensitivities};

/// Inverse (coin-margined) crypto vanilla input. Same market state as the linear
/// leaf; only the settlement/denomination differs (the payoff is in coins).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InverseInputs {
    /// Spot: USD price of one coin (`S`).
    pub spot: f64,
    /// Strike `K` in USD.
    pub strike: f64,
    /// Annualized volatility `σ` (absolute).
    pub vol: f64,
    /// Time to expiry `t` in years.
    pub t: f64,
    /// Cost-of-carry model `Carry::CostOfCarry { r, b = r − funding }`.
    pub carry: Carry,
}

impl InverseInputs {
    /// Construct directly from a [`Carry`].
    #[must_use]
    pub const fn new(spot: f64, strike: f64, vol: f64, t: f64, carry: Carry) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            carry,
        }
    }

    /// Construct with the funding carry `b = r − funding`.
    #[must_use]
    pub fn funded(spot: f64, strike: f64, vol: f64, t: f64, r: f64, funding: f64) -> Self {
        Self::new(
            spot,
            strike,
            vol,
            t,
            crate::funding::funding_carry(r, funding),
        )
    }

    /// Outright forward `F = spot · e^{b·t}`, via the carry seam.
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.spot * self.carry.forward_factor(self.t)
    }

    /// Numeraire discount factor `e^{−r·t}`, via the carry seam.
    #[must_use]
    pub fn discount_df(&self) -> f64 {
        self.carry.discount_df(self.t)
    }
}

/// The inverse-vanilla coin-measure Greek strip: every sensitivity is in **coins**
/// (the contract's natural unit), with the USD-equivalent of the premium exposed
/// separately ([`InverseGreeks::usd_equivalent`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InverseGreeks {
    /// Coin-measure strip (price + the 13 sensitivities), all in coins.
    pub coin: CarryGreeks,
    /// USD-equivalent of the coin premium: `V_coin · S` (USD per USD-notional-1).
    /// A derived field for cross-asset risk that nets in USD; the desk's hedging
    /// Greeks remain the coin-measure ones in [`InverseGreeks::coin`].
    pub usd_equivalent: f64,
}

/// Intermediate quantities shared by price and Greeks.
struct Aux {
    f: f64,
    df: f64,
    /// `e^{σ²t}` — the measure-correction factor on the `(K/F)·Φ(d3)` term.
    es2t: f64,
    d1: f64,
    d2: f64,
    /// `d3 = d1 − 2σ√t` — the second `σ√t`-shifted term from the `1/S_T` measure.
    d3: f64,
    sqt: f64,
    vsqt: f64,
}

#[inline]
fn aux(i: &InverseInputs) -> Aux {
    let sqt = sqrt(i.t);
    let vsqt = i.vol * sqt;
    let f = i.forward();
    let df = i.discount_df();
    let s2t = i.vol * i.vol * i.t;
    let d1 = (ln(f / i.strike) + 0.5 * s2t) / vsqt;
    let d2 = d1 - vsqt;
    let d3 = d1 - 2.0 * vsqt;
    Aux {
        f,
        df,
        es2t: exp(s2t),
        d1,
        d2,
        d3,
        sqt,
        vsqt,
    }
}

/// Coin price `V_coin = φ·df·[Φ(φ d2) − (K/F)·e^{σ²t}·Φ(φ d3)]` (coins per
/// USD-notional-1). Derived in the module doc from the USD risk-neutral expectation
/// of the `1/S_T`-weighted payoff — NOT a rescale of the linear price.
#[must_use]
pub fn price(opt: OptionType, i: &InverseInputs) -> f64 {
    let a = aux(i);
    let k_over_f = i.strike / a.f;
    match opt {
        OptionType::Call => a.df * (norm_cdf(a.d2) - k_over_f * a.es2t * norm_cdf(a.d3)),
        OptionType::Put => a.df * (k_over_f * a.es2t * norm_cdf(-a.d3) - norm_cdf(-a.d2)),
    }
}

/// Price and the full coin-measure Greek strip in a single pass, plus the
/// USD-equivalent of the premium.
///
/// Every sensitivity in [`InverseGreeks::coin`] is a partial of the **coin** price
/// `V_coin`. The rate sensitivities are reported as [`RateSensitivities::Carry`]
/// (discount-rho `∂V_coin/∂r`, carry-rho `∂V_coin/∂b`). The higher-order strip is
/// the analytic partials of the closed form above, each cross-validated against
/// central finite differences of `price` in the test suite.
#[must_use]
#[allow(clippy::similar_names)] // d1/d2/d3, nd2/nd3 are the canonical names
pub fn greeks(opt: OptionType, i: &InverseInputs) -> InverseGreeks {
    let a = aux(i);
    let (f, df, es2t, d1, d2, d3, sqt, vsqt) = (a.f, a.df, a.es2t, a.d1, a.d2, a.d3, a.sqt, a.vsqt);
    let (s, k, t, vol) = (i.spot, i.strike, i.t, i.vol);
    let r = i.carry.discount_rate();
    let b = i.carry.carry_rate();
    let k_over_f = k / f;

    // sign = φ; for a put the CDF args flip and the pdf is even.
    let sign = match opt {
        OptionType::Call => 1.0_f64,
        OptionType::Put => -1.0_f64,
    };
    let cap_d2 = norm_cdf(sign * d2);
    let cap_d3 = norm_cdf(sign * d3);
    // pd2 is φ(d2); we work in terms of pd2 throughout using the EXACT lognormal pdf
    // identity below.
    let pd2 = norm_pdf(d2);

    // The measure-correction amplitude A = (K/F)·e^{σ²t}, and the recurring product
    // ψ₃ ≡ A·Φ(φ d3). The key analytic collapse is the lognormal pdf identity
    //     A·φ(d3) = φ(d2)   EXACTLY,
    // because d2 − d3 = σ√t ⇒ φ(d3)/φ(d2) = exp(σ√t·d2 − ½σ²t) = (F/K)·e^{−σ²t} = 1/A.
    // This makes every spot/forward/carry pdf cross-term cancel, giving clean closed
    // forms for the whole strip (verified against central FD of the closed-form price
    // in the test suite). `amp` = A; `psi3` = ψ₃.
    let amp = k_over_f * es2t;
    let psi3 = amp * cap_d3;

    // Coin price: φ·df·[Φ(φd2) − A·Φ(φd3)] = φ·df·(cap_d2 − psi3).
    let price = sign * df * (cap_d2 - psi3);

    // FIRST ORDER (all pdf cross-terms cancel via A·φ(d3)=φ(d2)):
    //   delta_spot    = φ·df·ψ₃/S           (the two pdf legs cancel; only ∂A/∂S survives)
    //   delta_forward = φ·df·ψ₃/F
    //   discount_rho  = −t·V               (r enters only through df)
    //   carry_rho     = φ·df·t·ψ₃          (b enters via F = S e^{b t}; pdf legs cancel)
    let delta_spot = sign * df * psi3 / s;
    let delta_forward = sign * df * psi3 / f;
    let discount_rho = -t * price;
    let carry_rho = sign * df * t * psi3;

    // vega: σ enters d2,d3 (which DIFFER, ∂d2/∂σ−∂d3/∂σ = √t) and A via e^{σ²t}
    // (∂A/∂σ = 2σt·A). Using A·φ(d3)=φ(d2):
    //   vega = df·φ(d2)·√t − φ·df·2σt·ψ₃.
    let vega = df * pd2 * sqt - sign * df * 2.0 * vol * t * psi3;

    // ∂d1/∂σ and the ∂d/∂T chains used by the second-order strip.
    let lnfk = ln(f / k);
    let dd1_ds = -lnfk / (vol * vol * sqt) + 0.5 * sqt; // ∂d1/∂σ
    let dd2_ds = dd1_ds - sqt; // ∂d2/∂σ
    let dd3_ds = dd1_ds - 2.0 * sqt; // ∂d3/∂σ
    let dd1_dt = b / vsqt + 0.5 * vol / sqt - d1 / (2.0 * t); // ∂d1/∂T
    let dd2_dt = dd1_dt - 0.5 * vol / sqt; // ∂d2/∂T
    let dd3_dt = dd1_dt - vol / sqt; // ∂d3/∂T
    let u = 1.0 / (s * vsqt); // ∂d1/∂S = ∂d2/∂S = ∂d3/∂S

    // theta = −∂V/∂T. V = φ·df·(Φ(φd2) − A·Φ(φd3)), df_t = −r·df,
    //   ∂A/∂T = A(σ² − b);  ∂Φ(φd2)/∂T = φ(d2)·sign·dd2_dt; A·φ(d3)=φ(d2).
    //   ∂(A Φ(φd3))/∂T = A(σ²−b)Φ(φd3) + φ(d2)·sign·dd3_dt = (σ²−b)ψ₃ + sign·φ(d2)·dd3_dt.
    let inner = cap_d2 - psi3;
    let dinner_dt = pd2 * sign * dd2_dt - ((vol * vol - b) * psi3 + sign * pd2 * dd3_dt);
    let theta = -(sign * (-r * df * inner + df * dinner_dt));

    // SECOND ORDER — analytic partials of the clean first-order forms above. ψ₃
    // derivatives:  ∂ψ₃/∂S = −ψ₃/S + sign·φ(d2)·u ; ∂ψ₃/∂σ = 2σt·ψ₃ + sign·φ(d2)·dd3_ds ;
    // ∂ψ₃/∂T = (σ²−b)ψ₃ + sign·φ(d2)·dd3_dt.   (φ(d3)·A = φ(d2) throughout.)

    // vanna = ∂(delta_spot)/∂σ = sign·df·(∂ψ₃/∂σ)/S.
    let vanna = sign * df * (2.0 * vol * t * psi3 + sign * pd2 * dd3_ds) / s;

    // volga = ∂vega/∂σ. ∂φ(d2)/∂σ = −d2·φ(d2)·dd2_ds.
    let dpsi3_ds = 2.0 * vol * t * psi3 + sign * pd2 * dd3_ds;
    let volga =
        df * sqt * (-d2 * pd2 * dd2_ds) - sign * df * (2.0 * t * psi3 + 2.0 * vol * t * dpsi3_ds);

    // charm = ∂(delta_spot)/∂T, including ∂df/∂T = −r·df.
    let dpsi3_dt = (vol * vol - b) * psi3 + sign * pd2 * dd3_dt;
    let charm = sign * df * (-r * psi3 + dpsi3_dt) / s;

    // gamma = ∂(delta_spot)/∂S = df·φ(d2)·u/S − sign·df·2·ψ₃/S².
    let gamma = df * pd2 * u / s - sign * df * 2.0 * psi3 / (s * s);

    // speed = ∂gamma/∂S. ∂φ(d2)/∂S = −d2·φ(d2)·u, ∂u/∂S = −u/S, ∂ψ₃/∂S as above.
    let speed = df * (-d2 * pd2 * u * u / s - 4.0 * pd2 * u / (s * s))
        + sign * df * 6.0 * psi3 / (s * s * s);

    // zomma = ∂gamma/∂σ. ∂φ(d2)/∂σ = −d2·φ(d2)·dd2_ds, ∂u/∂σ = −u/σ.
    let zomma =
        df * pd2 * u / s * (-d2 * dd2_ds - 1.0 / vol) - sign * df * 2.0 / (s * s) * dpsi3_ds;

    // color = ∂gamma/∂T. ∂df/∂T=−r·df, ∂φ(d2)/∂T=−d2·φ(d2)·dd2_dt, ∂u/∂T=−u/(2T).
    let color = df * pd2 * u / s * (-r - d2 * dd2_dt - 1.0 / (2.0 * t))
        - sign * df * 2.0 / (s * s) * ((vol * vol - b - r) * psi3 + sign * pd2 * dd3_dt);

    let coin = CarryGreeks {
        price,
        delta_spot,
        delta_forward,
        gamma,
        vega,
        theta,
        rates: RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        },
        vanna,
        volga,
        charm,
        speed,
        zomma,
        color,
    };
    InverseGreeks {
        coin,
        usd_equivalent: price * s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn cost_of_carry(r: f64, b: f64) -> Carry {
        Carry::CostOfCarry { r, b }
    }

    /// ORACLE (2 of 3) — Gauss-Hermite quadrature of the RAW payoff. The production
    /// coin price equals a DIRECT lognormal expectation of `df·max(φ(S_T−K),0)/S_T`
    /// evaluated by 40-node Gauss-Hermite quadrature — a different assembly than the
    /// CDF closed form (it integrates the literal `1/S_T`-weighted payoff against the
    /// density, never touching the production `Φ(d2)/Φ(d3)` combination). This is the
    /// HIGH-circular-risk anchor; it shares the lognormal-measure assumption with
    /// production, so the disagree-capable MC below (which has NO analytic structure)
    /// and the structural sandwich are the genuine error-catchers (W3 §6).
    #[test]
    fn closed_form_matches_deterministic_quadrature() {
        // High-resolution midpoint quadrature of E^Q[df·payoff/S_T] against the
        // standard-normal density φ(z) over z ∈ [−12, 12]. The `1/S_T` weight gives the
        // integrand a fat left tail (small S_T ⇒ large 1/S_T), which Gauss-Hermite
        // (tuned for polynomial×gaussian) under-resolves at crypto vols; a fine uniform
        // grid integrates the LITERAL payoff with no analytic structure shared with the
        // production CDF assembly. 400k panels reach ~1e-7 on the closed form.
        let lo = -12.0_f64;
        let hi = 12.0_f64;
        let panels = 400_000usize;
        let dz = (hi - lo) / panels as f64;
        let inv_sqrt_2pi = 1.0 / (2.0 * std::f64::consts::PI).sqrt();
        for &(s, k, vol, t, r, funding) in &[
            (30_000.0, 32_000.0, 0.65, 0.25, 0.05, 0.02),
            (2_000.0, 1_800.0, 0.80, 0.5, 0.04, 0.10),
            (45_000.0, 45_000.0, 0.55, 1.0, 0.03, -0.01),
        ] {
            let i = InverseInputs::funded(s, k, vol, t, r, funding);
            let f = i.forward();
            let df = i.discount_df();
            let drift = -0.5 * vol * vol * t;
            let diff = vol * t.sqrt();
            for opt in [OptionType::Call, OptionType::Put] {
                let phi = match opt {
                    OptionType::Call => 1.0,
                    OptionType::Put => -1.0,
                };
                let mut acc = 0.0;
                for j in 0..panels {
                    let z = lo + (j as f64 + 0.5) * dz;
                    let pdf = inv_sqrt_2pi * (-0.5 * z * z).exp();
                    let st = f * (drift + diff * z).exp();
                    let payoff = (phi * (st - k)).max(0.0) / st;
                    acc += payoff * pdf * dz;
                }
                let quad = df * acc;
                assert_close!(price(opt, &i), quad, 1e-6, 1e-7);
            }
        }
    }

    /// ORACLE (1 of 3) — the DISAGREE-CAPABLE code-disjoint Monte-Carlo. An in-test
    /// splitmix64 RNG + Box-Muller simulates `S_T = F·exp(−½σ²t + σ√t·Z)` and averages
    /// the LITERAL coin payoff `df·max(φ(S_T−K),0)/S_T` with ZERO analytic structure —
    /// so a shared measure error in BOTH closed forms (production AND the quadrature,
    /// which share the lognormal assumption) surfaces here as an MC mismatch. We gate
    /// WITHIN the reported MC standard error (≈3σ band), NEVER to closed-form
    /// precision (VERIFICATION-CONTRACT (a)/(b)). This is the FRTB-0.75ρ-class
    /// circular-oracle guard: the MC can disagree.
    #[test]
    fn closed_form_within_monte_carlo_standard_error() {
        // Deterministic, code-disjoint RNG (no dependency on any production sampler).
        struct SplitMix64(u64);
        impl SplitMix64 {
            fn next_u64(&mut self) -> u64 {
                self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^ (z >> 31)
            }
            /// Uniform in (0,1) (excluding the exact endpoints so ln() is finite).
            fn next_uniform(&mut self) -> f64 {
                // 53-bit mantissa; +0.5 keeps it strictly inside (0,1).
                ((self.next_u64() >> 11) as f64 + 0.5) * (1.0 / (1u64 << 53) as f64)
            }
        }

        let n: usize = 4_000_000;
        for &(s, k, vol, t, r, funding) in &[
            (30_000.0, 32_000.0, 0.65, 0.25, 0.05, 0.02),
            (2_000.0, 1_800.0, 0.80, 0.5, 0.04, 0.10),
            (45_000.0, 45_000.0, 0.55, 1.0, 0.03, -0.01),
        ] {
            let i = InverseInputs::funded(s, k, vol, t, r, funding);
            let f = i.forward();
            let df = i.discount_df();
            let drift = -0.5 * vol * vol * t;
            let diff = vol * t.sqrt();
            for opt in [OptionType::Call, OptionType::Put] {
                let phi = match opt {
                    OptionType::Call => 1.0,
                    OptionType::Put => -1.0,
                };
                let mut rng = SplitMix64(0xC0FF_EE12_3456_789A ^ (k.to_bits()));
                let mut sum = 0.0;
                let mut sum_sq = 0.0;
                let mut draws = 0usize;
                while draws < n {
                    // Box-Muller: two independent standard normals per uniform pair.
                    let u1 = rng.next_uniform();
                    let u2 = rng.next_uniform();
                    let radius = (-2.0 * u1.ln()).sqrt();
                    let angle = std::f64::consts::TAU * u2;
                    for z in [radius * angle.cos(), radius * angle.sin()] {
                        let st = f * (drift + diff * z).exp();
                        let payoff = df * (phi * (st - k)).max(0.0) / st;
                        sum += payoff;
                        sum_sq += payoff * payoff;
                        draws += 1;
                    }
                }
                let nn = draws as f64;
                let mean = sum / nn;
                let var = (sum_sq / nn - mean * mean).max(0.0);
                let stderr = (var / nn).sqrt();
                let closed = price(opt, &i);
                // Gate within ≈4 standard errors (no lowered closed-form tolerance):
                // the MC is the disagree-capable oracle, so the band is its OWN stderr.
                assert!(
                    (closed - mean).abs() <= 4.0 * stderr + 1e-12,
                    "inverse {opt:?}: closed={closed} mc={mean} |Δ|={} > 4·stderr={} (S={s},K={k})",
                    (closed - mean).abs(),
                    4.0 * stderr
                );
            }
        }
    }

    /// `K → 0` call → `df` coins (payoff `S_T/S_T = 1` discounted), independent of σ/K.
    #[test]
    fn k_to_zero_call_is_discount_factor() {
        let i = InverseInputs::funded(30_000.0, 1e-6, 0.65, 0.5, 0.05, 0.02);
        assert_close!(price(OptionType::Call, &i), i.discount_df(), 1e-9, 1e-9);
    }

    /// Deep-ITM / deep-OTM monotonicity of the coin price.
    #[test]
    fn deep_itm_otm_monotonic() {
        // Deep-ITM call (S ≫ K): coin price → df·(1 − small) ≈ df.
        let itm = InverseInputs::funded(60_000.0, 30_000.0, 0.5, 0.5, 0.05, 0.0);
        assert!(price(OptionType::Call, &itm) < itm.discount_df());
        assert!(price(OptionType::Call, &itm) > 0.4 * itm.discount_df());
        // Deep-OTM call (S ≪ K): → ~0.
        let otm = InverseInputs::funded(10_000.0, 60_000.0, 0.5, 0.5, 0.05, 0.0);
        assert!(price(OptionType::Call, &otm) < 1e-4);
        // Monotone increasing in spot.
        let lo = InverseInputs::funded(28_000.0, 30_000.0, 0.6, 0.5, 0.05, 0.0);
        let hi = InverseInputs::funded(32_000.0, 30_000.0, 0.6, 0.5, 0.05, 0.0);
        assert!(price(OptionType::Call, &hi) > price(OptionType::Call, &lo));
    }

    /// ORACLE (3 of 3) — the structural convexity sandwich: `V_coin·S₀ ≠ V_linear`
    /// STRICTLY, with the SIGNED direction the `Cov(1/S_T, payoff)` term dictates. A
    /// naive `V_lin/S₀` rescale would give exact equality `V_coin·S₀ == V_lin`; the
    /// `1/S_T` weight breaks it, and the SIGN is determined by the correlation:
    ///
    ///   * **call** — the payoff grows with `S_T`, so the `1/S_T` weight DOWN-weights
    ///     the large-payoff tail ⇒ `Cov(1/S_T, call payoff) < 0` ⇒ `V_coin·S₀ < V_lin`.
    ///   * **put** — the payoff grows as `S_T` falls, so `1/S_T` UP-weights it ⇒
    ///     `Cov(1/S_T, put payoff) > 0` ⇒ `V_coin·S₀ > V_lin`.
    ///
    /// (`V_coin·S₀ = E^Q[payoff·S₀/S_T]` vs `V_lin = E^Q[payoff]`; equal iff `1/S_T`
    /// and the payoff are uncorrelated AND `E^Q[1/S_T]=1/S₀` — both false.) This
    /// qualitative, signed gate fails loudly on ANY naive rescale (which gives `0`
    /// difference) and is the disagree-capable structural anchor (W3 §6 anti-circular
    /// discipline).
    #[test]
    fn convexity_sandwich_vs_linear_is_signed() {
        for &(s, k, vol, t, r, funding) in &[
            (30_000.0, 30_000.0, 0.65, 0.5, 0.05, 0.02),
            (2_000.0, 2_500.0, 0.80, 1.0, 0.04, 0.10),
            (45_000.0, 40_000.0, 0.55, 0.25, 0.03, -0.01),
        ] {
            let inv = InverseInputs::funded(s, k, vol, t, r, funding);
            let lin = crate::linear::LinearInputs::funded(s, k, vol, t, r, funding);

            let coin_call = price(OptionType::Call, &inv) * s;
            let usd_call = crate::linear::price(OptionType::Call, &lin);
            assert!(
                coin_call < usd_call * (1.0 - 1e-6),
                "call convexity: coin·S={coin_call} must be STRICTLY < V_lin={usd_call}"
            );

            let coin_put = price(OptionType::Put, &inv) * s;
            let usd_put = crate::linear::price(OptionType::Put, &lin);
            assert!(
                coin_put > usd_put * (1.0 + 1e-6),
                "put convexity: coin·S={coin_put} must be STRICTLY > V_lin={usd_put}"
            );

            // Either way: STRICTLY not the naive rescale (the binding anti-circular
            // statement) — a `V_lin/S₀` implementation would make both differences 0.
            assert!((coin_call - usd_call).abs() > 1e-6 * usd_call);
            assert!((coin_put - usd_put).abs() > 1e-6 * usd_put);
        }
    }

    /// USD-equivalent derived field is `V_coin · S`.
    #[test]
    fn usd_equivalent_is_coin_times_spot() {
        let i = InverseInputs::funded(30_000.0, 31_000.0, 0.65, 0.5, 0.05, 0.02);
        let g = greeks(OptionType::Call, &i);
        assert_eq!(
            g.usd_equivalent.to_bits(),
            (g.coin.price * i.spot).to_bits()
        );
        assert_eq!(
            g.coin.price.to_bits(),
            price(OptionType::Call, &i).to_bits()
        );
    }

    // ---- finite-difference Greek oracle: every coin Greek vs central FD of the
    // closed-form coin price (the independent gate). ----

    fn fd1<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
        (f(x + h) - f(x - h)) / (2.0 * h)
    }
    fn with_spot(i: &InverseInputs, s: f64) -> InverseInputs {
        InverseInputs { spot: s, ..*i }
    }
    fn with_vol(i: &InverseInputs, v: f64) -> InverseInputs {
        InverseInputs { vol: v, ..*i }
    }
    fn with_t(i: &InverseInputs, t: f64) -> InverseInputs {
        InverseInputs { t, ..*i }
    }
    fn with_r(i: &InverseInputs, r: f64) -> InverseInputs {
        let b = i.carry.carry_rate();
        InverseInputs {
            carry: Carry::CostOfCarry { r, b },
            ..*i
        }
    }
    fn with_b(i: &InverseInputs, b: f64) -> InverseInputs {
        let r = i.carry.discount_rate();
        InverseInputs {
            carry: Carry::CostOfCarry { r, b },
            ..*i
        }
    }

    fn check_greeks(opt: OptionType, i: &InverseInputs) {
        let g = greeks(opt, i).coin;
        let p = |x: &InverseInputs| price(opt, x);
        let hs = 1e-4 * i.spot;

        assert_close!(
            g.delta_spot,
            fd1(|s| p(&with_spot(i, s)), i.spot, hs),
            1e-4,
            1e-9
        );
        assert_close!(g.vega, fd1(|v| p(&with_vol(i, v)), i.vol, 1e-5), 1e-4, 1e-9);

        // forward delta = ∂V/∂F via the spot-FD divided by the carry factor.
        let carry = i.carry.forward_factor(i.t);
        assert_close!(
            g.delta_forward,
            fd1(|s| p(&with_spot(i, s)), i.spot, hs) / carry,
            1e-4,
            1e-9
        );

        assert_close!(g.theta, -fd1(|t| p(&with_t(i, t)), i.t, 1e-6), 5e-4, 1e-9);

        match g.rates {
            RateSensitivities::Carry {
                discount_rho,
                carry_rho,
            } => {
                assert_close!(
                    discount_rho,
                    fd1(|r| p(&with_r(i, r)), i.carry.discount_rate(), 1e-6),
                    1e-4,
                    1e-9
                );
                assert_close!(
                    carry_rho,
                    fd1(|b| p(&with_b(i, b)), i.carry.carry_rate(), 1e-6),
                    1e-4,
                    1e-9
                );
            }
            RateSensitivities::Fx { .. } => panic!("crypto inverse greeks must tag as Carry"),
        }

        // Second-order via differencing the relevant first-order coin Greek.
        let ds = |s: f64| greeks(opt, &with_spot(i, s)).coin.delta_spot;
        let gam = |x: &InverseInputs| greeks(opt, x).coin.gamma;
        assert_close!(g.gamma, fd1(ds, i.spot, hs), 1e-3, 1e-9);
        assert_close!(
            g.vanna,
            fd1(
                |v| greeks(opt, &with_vol(i, v)).coin.delta_spot,
                i.vol,
                1e-5
            ),
            1e-3,
            1e-9
        );
        assert_close!(
            g.volga,
            fd1(|v| greeks(opt, &with_vol(i, v)).coin.vega, i.vol, 1e-5),
            1e-3,
            1e-9
        );
        assert_close!(
            g.charm,
            fd1(|t| greeks(opt, &with_t(i, t)).coin.delta_spot, i.t, 1e-5),
            1e-3,
            1e-9
        );
        assert_close!(
            g.speed,
            fd1(|s| gam(&with_spot(i, s)), i.spot, hs),
            1e-2,
            1e-9
        );
        assert_close!(
            g.zomma,
            fd1(|v| gam(&with_vol(i, v)), i.vol, 1e-5),
            1e-2,
            1e-9
        );
        assert_close!(g.color, fd1(|t| gam(&with_t(i, t)), i.t, 1e-5), 1e-2, 1e-9);
    }

    #[test]
    fn greeks_vs_finite_difference() {
        let cases = [
            InverseInputs::funded(30_000.0, 30_000.0, 0.65, 0.5, 0.05, 0.02),
            InverseInputs::funded(2_000.0, 2_200.0, 0.80, 1.0, 0.04, 0.10),
            InverseInputs::new(45_000.0, 40_000.0, 0.55, 0.25, cost_of_carry(0.03, -0.04)),
        ];
        for i in &cases {
            check_greeks(OptionType::Call, i);
            check_greeks(OptionType::Put, i);
        }
    }

    /// `greeks(opt, i).coin.price` MUST be bit-identical to `price(opt, i)`.
    #[test]
    fn greeks_price_is_bit_identical_to_price() {
        let cases = [
            InverseInputs::funded(30_000.0, 30_000.0, 0.65, 0.5, 0.05, 0.02),
            InverseInputs::new(45_000.0, 40_000.0, 0.55, 0.25, cost_of_carry(0.03, -0.04)),
        ];
        for i in &cases {
            for opt in [OptionType::Call, OptionType::Put] {
                assert_eq!(price(opt, i).to_bits(), greeks(opt, i).coin.price.to_bits());
            }
        }
    }
}
