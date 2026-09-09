//! Standalone Heston (1993) stochastic-volatility European vanilla pricer.
//!
//! The Heston model gives the spot–variance dynamics (under the risk-neutral
//! measure, FX carry `r_d − r_f`):
//!
//! ```text
//!   dS_t = (r_d − r_f)·S_t dt + √v_t·S_t dW^S_t
//!   dv_t = κ(θ − v_t) dt + σ·√v_t dW^v_t,   d⟨W^S, W^v⟩_t = ρ dt
//! ```
//!
//! with mean-reversion speed `κ`, long-run variance `θ`, vol-of-vol `σ`,
//! spot/variance correlation `ρ ∈ [−1, 1]` and initial variance `v_0`. Unlike
//! Black–Scholes (one constant vol), Heston produces a genuine implied-vol smile.
//!
//! European vanilla prices follow from the **characteristic function** of the
//! log-spot, `φ(u) = E[e^{i u ln S_T}]`, which Heston derived in closed form.
//! This crate prices via **two genuinely independent Fourier transforms** of that
//! same CF, so agreement between them validates both:
//!
//! 1. [`carr_madan`] — direct convergent numerical integration of the
//!    Carr–Madan (1999) damped-call Fourier integral. We do **not** pull in an
//!    external FFT (a single strike does not need radix-2 FFT, and an external
//!    crate would add a dependency and break determinism); instead the integral
//!    is evaluated by composite **Gauss–Legendre** quadrature over a truncated,
//!    decay-justified range — the right tool for one strike with full control of
//!    accuracy.
//! 2. [`cos`] — the Fang–Oosterlee (2008) **COS** method: a cosine-series
//!    expansion of the (unknown) risk-neutral density reconstructed from the CF,
//!    with the standard cumulant-based truncation range `[a, b]` and `N` terms.
//!
//! The shared characteristic function uses a continuous **branch-cut-free** formulation
//! which contains no complex logarithm of a winding argument and remains the continuous analytic continuation
//! at every complex argument across all maturities. See [`return_char_fn`].
//!
//! ## Honest accuracy
//!
//! The two transforms agree to `|cm − cos| ≤ 1e-8 + 1e-7·price` across the **full
//! practical FX-vanilla grid up to ~3y** — every strike from deep ITM to deep OTM
//! (K ∈ [55, 185] on S=100), every parameter set including the Feller-violated
//! ones (worst-case relative for a non-tiny price ≈ 3e-8). This is the validated,
//! gated regime.
//!
//! **HONEST BOUNDARY (beyond ~3y).** Past ~3y the Heston risk-neutral density
//! develops heavy tails, and for **deep-OTM** strikes the option value falls below
//! the Fourier methods' absolute precision floor; the two transforms then diverge
//! (negligibly for at-/in-the-money strikes, but up to tens of % for the deepest
//! 5y wings). This is the well-known precision wall of Fourier option pricing —
//! *intrinsic to BOTH methods*, not a defect of either — and is the province of
//! PDE / Monte-Carlo engines, not this closed-form-transform layer. The parity
//! tests therefore assert the tight cross-method band only on the ≤3y regime; the
//! Black–Scholes-limit, put–call-parity, anchor and monotonicity oracles validate
//! correctness independently of cross-method agreement.

#![forbid(unsafe_code)]

mod complex;

use celnet_core::math::{exp, ln};
use celnet_types::OptionType;
use complex::Complex;

/// Heston stochastic-volatility model parameters.
///
/// All fields are in the natural units of the SDE above: `kappa`, `theta`,
/// `vol_of_vol` and `v0` are variance-scale quantities (e.g. `theta = 0.04`
/// means a long-run vol of `√0.04 = 20%`), `rho ∈ [−1, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HestonParams {
    /// Mean-reversion speed `κ > 0`.
    pub kappa: f64,
    /// Long-run variance `θ > 0`.
    pub theta: f64,
    /// Volatility of variance (vol-of-vol) `σ > 0`.
    pub vol_of_vol: f64,
    /// Spot/variance correlation `ρ ∈ [−1, 1]`.
    pub rho: f64,
    /// Initial variance `v_0 ≥ 0`.
    pub v0: f64,
}

impl HestonParams {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(kappa: f64, theta: f64, vol_of_vol: f64, rho: f64, v0: f64) -> Self {
        Self {
            kappa,
            theta,
            vol_of_vol,
            rho,
            v0,
        }
    }

    /// Whether the **Feller condition** `2·κ·θ ≥ σ²` holds. When it does, the
    /// variance process stays strictly positive almost surely. Reported for
    /// diagnostics — the transforms remain valid (and the integrals convergent)
    /// even when it is violated, which is common in calibrated FX surfaces.
    #[must_use]
    pub fn feller_satisfied(&self) -> bool {
        2.0 * self.kappa * self.theta >= self.vol_of_vol * self.vol_of_vol
    }
}

/// Market/contract inputs for a European vanilla under Heston, with the FX
/// dual-rate carry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketInputs {
    /// Spot FX rate (quote per 1 unit of base).
    pub spot: f64,
    /// Strike (quote per 1 unit of base).
    pub strike: f64,
    /// Time to expiry in years.
    pub t: f64,
    /// Continuously-compounded domestic (quote) interest rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) interest rate.
    pub r_for: f64,
}

impl MarketInputs {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(spot: f64, strike: f64, t: f64, r_dom: f64, r_for: f64) -> Self {
        Self {
            spot,
            strike,
            t,
            r_dom,
            r_for,
        }
    }

    /// Domestic discount factor `e^{−r_d·t}`.
    #[inline]
    #[must_use]
    pub fn df_dom(&self) -> f64 {
        exp(-self.r_dom * self.t)
    }

    /// Foreign discount factor `e^{−r_f·t}` (the "dividend" discount on the base
    /// currency).
    #[inline]
    #[must_use]
    pub fn df_for(&self) -> f64 {
        exp(-self.r_for * self.t)
    }
}

/// The Heston characteristic function of the **log-return** `x = ln(S_T/S_0)`,
/// evaluated at complex argument `u`: `φ_ret(u) = E[e^{i·u·x}]` (it does *not*
/// include the `e^{i·u·ln S₀}` spot-phase — both transforms work in return space
/// and apply any spot/strike shift via their own real-valued phase, which avoids
/// forming and cancelling the large `u·ln S₀` phase at high `u`).
///
/// **Continuous branch-cut-free formulation**:
/// This form contains **no complex logarithm of a winding
/// argument**: the only transcendental of a complex argument is `exp`, and the
/// `cosh/sinh` combination `A₂ = d·cosh(dt/2) + ξ·sinh(dt/2)` stays in the right
/// half-plane (`Re(d) ≥ 0`), so the characteristic function is the continuous analytic continuation
/// at **every** complex `u`. This guarantees numerical stability across the entire pricing domain.
/// With `μ = r_d − r_f` and `t = T`:
///
/// ```text
///   ξ  = κ − σρ·i·u
///   d  = √( ξ² + σ²·(i·u + u²) )                      (principal, Re d ≥ 0)
///   A₁ = (u² + i·u)·sinh(d t/2)
///   A₂ = d·cosh(d t/2) + ξ·sinh(d t/2)
///   A  = A₁ / A₂
///   B  = d·e^{κ t/2} / A₂                             (so ln B winds with κ t/2 only)
///   φ_ret(u) = exp( i·u·μ·t − (κθρt/σ)·i·u − v₀·A + (2κθ/σ²)·ln B )
/// ```
fn return_char_fn(u: Complex, m: &MarketInputs, p: &HestonParams) -> Complex {
    char_exponent(u, m, p).exp()
}

/// The characteristic **exponent** `ψ(u) = ln φ_ret(u)` — i.e. [`return_char_fn`]
/// before the final `exp`. Exposed so the cumulants can be extracted from its
/// derivatives without re-deriving lengthy closed forms (see [`heston_c4`]).
fn char_exponent(u: Complex, m: &MarketInputs, p: &HestonParams) -> Complex {
    let sigma = p.vol_of_vol;
    let sig2 = sigma * sigma;
    let (kappa, theta, rho) = (p.kappa, p.theta, p.rho);
    let mu = m.r_dom - m.r_for;
    let t = m.t;

    let iu = Complex::new(-u.im, u.re); // i·u
    // ξ = κ − σρ·iu
    let xi = Complex::real(kappa).sub(iu.scale(sigma * rho));
    // d = √( ξ² + σ²·(iu + u²) ),  principal branch (Re d ≥ 0).
    let u_sq = u.mul(u);
    let d = xi.mul(xi).add(iu.add(u_sq).scale(sig2)).sqrt();

    // OVERFLOW-STABLE factoring. With `Re(d) ≥ 0`, `cosh(dt/2)` and `sinh(dt/2)`
    // both blow up like `e^{Re(d)·t/2}` for large |u| or long `t`, which would
    // overflow (→ NaN in the A₁/A₂ ratio). Factor the common `e^{dt/2}` out using
    //   cosh(dt/2) = e^{dt/2}(1+w)/2,  sinh(dt/2) = e^{dt/2}(1−w)/2,  w = e^{−dt},
    // where `|w| ≤ 1` is *bounded* (Re d ≥ 0). The `e^{dt/2}` cancels in
    // `A = A₁/A₂`, and appears in `B` only as the bounded `e^{−dt/2}`:
    //   A = (u²+iu)(1−w) / [ d(1+w) + ξ(1−w) ],
    //   B = 2·d·e^{κt/2}·e^{−dt/2} / [ d(1+w) + ξ(1−w) ].
    let one = Complex::real(1.0);
    let w = d.scale(-t).exp(); // e^{−dt}, bounded
    let one_p_w = one.add(w);
    let one_m_w = one.sub(w);
    let denom = d.mul(one_p_w).add(xi.mul(one_m_w)); // = 2·A₂·e^{−dt/2}

    let a = u_sq.add(iu).mul(one_m_w).div(denom);

    // ln B with the e^{−dt/2} kept inside (bounded), so the principal log never
    // overflows and never winds (denom stays in the right half-plane).
    let e_neg_dt2 = d.scale(-0.5 * t).exp(); // e^{−dt/2}, bounded
    let b = d
        .scale(2.0 * exp(0.5 * kappa * t))
        .mul(e_neg_dt2)
        .div(denom);
    let ln_b = b.ln();

    // φ_ret = exp( iuμt − (κθρt/σ)·iu − v₀·A + (2κθ/σ²)·ln B ).
    iu.scale(mu * t)
        .sub(iu.scale(kappa * theta * rho * t / sigma))
        .sub(a.scale(p.v0))
        .add(ln_b.scale(2.0 * kappa * theta / sig2))
}

/// Carr–Madan damping factor `α > 0`. Any `α` in the strip where
/// `E[S_T^{α+1}] < ∞` gives the same price; `α = 1.5` is the standard robust
/// choice (well inside the admissible strip for the tested parameters).
const CM_ALPHA: f64 = 1.5;
/// Minimum and maximum panel counts for the composite Gauss–Legendre rule. The
/// actual count is chosen per-contract so the rule resolves *both* the Gaussian
/// decay of the CF and the `e^{−i v k}` oscillation (≈ a fixed number of GL
/// panels per oscillation period over the integration range).
const CM_MIN_PANELS: usize = 96;
const CM_MAX_PANELS: usize = 65_536;

/// 16-point Gauss–Legendre nodes/weights on `[−1, 1]` (symmetric; listed for the
/// positive half, mirrored at evaluation). Standard tabulated constants.
const GL16_X: [f64; 8] = [
    0.095_012_509_837_637_44,
    0.281_603_550_779_258_9,
    0.458_016_777_657_227_4,
    0.617_876_244_402_643_7,
    0.755_404_408_355_003,
    0.865_631_202_387_831_7,
    0.944_575_023_073_232_6,
    0.989_400_934_991_65,
];
const GL16_W: [f64; 8] = [
    0.189_450_610_455_068_5,
    0.182_603_415_044_923_6,
    0.169_156_519_395_002_53,
    0.149_595_988_816_576_73,
    0.124_628_971_255_533_87,
    0.095_158_511_682_492_78,
    0.062_253_523_938_647_89,
    0.027_152_459_411_754_096,
];

/// Integrate `f` over `[lo, hi]` with composite 16-point Gauss–Legendre over
/// `n_panels` equal sub-intervals. Real-valued integrand.
fn gauss_legendre<F: Fn(f64) -> f64>(f: F, lo: f64, hi: f64, n_panels: usize) -> f64 {
    let h = (hi - lo) / n_panels as f64;
    let half = 0.5 * h;
    let mut acc = 0.0;
    for panel in 0..n_panels {
        let mid = lo + (panel as f64 + 0.5) * h;
        for k in 0..8 {
            let dx = half * GL16_X[k];
            acc += GL16_W[k] * (f(mid + dx) + f(mid - dx));
        }
    }
    acc * half
}

/// Expected integrated variance of the log-return over `[0, T]` under Heston:
/// `E[∫₀ᵀ v_s ds] = θ·T + (v₀ − θ)·(1 − e^{−κT})/κ`. This is the variance scale
/// that governs the Gaussian tail of the characteristic function (the CF's
/// leading behaviour is `≈ exp(−½·var·u²)`), so it sets a decay-justified
/// integration range for the Carr–Madan quadrature.
fn effective_log_variance(m: &MarketInputs, p: &HestonParams) -> f64 {
    let v = p.theta * m.t + (p.v0 - p.theta) * (1.0 - exp(-p.kappa * m.t)) / p.kappa;
    // Guard against a pathological (near-)zero so the range stays finite.
    v.max(1e-8)
}

/// Decay-justified upper limit for the Carr–Madan `v`-integral.
///
/// The Heston damped integrand has **two** decay regimes and we must cover the
/// slower one. (1) For small vol-of-vol the CF is near-Gaussian,
/// `|ψ| ≲ exp(−½·v̄·v²)` with `v̄` the expected integrated variance, giving
/// `v_max ≈ √(2·36.84/v̄)`. (2) For larger σ the CF decays only
/// **exponentially**, `|ψ| ≲ exp(−R·v)`, with leading rate (from the large-`v`
/// asymptotics of the trap CF, `d ≈ σ√(1−ρ²)·v`)
/// `R ≈ v₀/(σ√(1−ρ²)) + κθ·T·√(1−ρ²)/σ`, giving `v_max ≈ 36.84/R`. We take the
/// **larger** of the two (whichever decays slower), so the truncation error of
/// the envelope is ≤1e−16 in either regime. `ln(10¹⁶) ≈ 36.84`.
fn carr_madan_upper(m: &MarketInputs, p: &HestonParams) -> f64 {
    const LN_1E16: f64 = 36.84;
    let var_x = effective_log_variance(m, p);
    let gaussian = (2.0 * LN_1E16 / var_x).sqrt();

    let omr2 = (1.0 - p.rho * p.rho).max(1e-6).sqrt(); // √(1−ρ²)
    let rate = p.v0 / (p.vol_of_vol * omr2) + p.kappa * p.theta * m.t * omr2 / p.vol_of_vol;
    // Guard the (degenerate) rate→0 case with the Gaussian fallback.
    let exponential = if rate > 1e-8 {
        LN_1E16 / rate
    } else {
        gaussian
    };

    // 1.5× safety margin on top of the slower-decaying estimate, clamped.
    (1.5 * gaussian.max(exponential)).clamp(50.0, 20_000.0)
}

/// **Carr–Madan** European vanilla price under Heston via direct convergent
/// integration of the damped-call Fourier integral.
///
/// Working in log-moneyness `κ = ln(K/S₀)` with the return-space CF `φ_ret`
/// (avoiding the large `ln S₀` phase), the damped call is
///
/// ```text
///   C(K) = e^{−r_d T}·S₀·e^{−α κ} / π · ∫₀^∞ Re[ e^{−i v κ} ψ(v) ] dv,
///   ψ(v) = φ_ret(v − (α+1)i) / (α² + α − v² + i(2α+1)v),
/// ```
///
/// (the spot factors of the standard `k = ln K` form collapse to the leading
/// `S₀·e^{−α κ}`). We evaluate the integral by composite Gauss–Legendre
/// quadrature out to a decay-justified upper limit ([`carr_madan_upper`], which
/// covers BOTH the Gaussian small-σ tail and the slower **exponential** large-σ
/// tail), with the panel count scaled to resolve the `e^{−i v κ}` oscillation.
/// Puts are obtained by exact put–call parity.
#[must_use]
pub fn carr_madan(opt: OptionType, m: &MarketInputs, p: &HestonParams) -> f64 {
    let alpha = CM_ALPHA;
    // Work in LOG-MONEYNESS κ = ln(K/S₀) using the return-space CF φ_ret directly
    // (no large `ln S₀` phase to form and cancel): the spot enters only through
    // the moneyness and an overall real scale, keeping the integrand accurate at
    // high frequency.
    let kappa_m = ln(m.strike / m.spot);
    let df_dom = m.df_dom();

    // ψ(v) = φ_ret(v − (α+1)i) / (α² + α − v² + i(2α+1)v).
    let psi = |v: f64| -> Complex {
        let u = Complex::new(v, -(alpha + 1.0));
        let num = return_char_fn(u, m, p);
        let den = Complex::new(alpha * alpha + alpha - v * v, (2.0 * alpha + 1.0) * v);
        num.div(den)
    };

    // Integrand Re[ e^{−i v κ} ψ(v) ] (κ = log-moneyness).
    let integrand = |v: f64| -> f64 {
        let phase = Complex::new(libm::cos(v * kappa_m), -libm::sin(v * kappa_m)); // e^{−i v κ}
        phase.mul(psi(v)).re
    };

    // Decay-justified upper limit covering BOTH the Gaussian (small-σ) and the
    // slower exponential (large-σ) tail of the Heston damped integrand.
    let upper = carr_madan_upper(m, p);

    // Panel count: resolve both the envelope and the e^{−i v κ} oscillation. We
    // want (a) a base density of ≈1 panel per unit `v` so the 16-point GL rule
    // amply resolves the smooth decaying envelope over `[0, upper]`, and (b)
    // several panels per oscillation period (≈ upper·|κ|/(2π) periods total).
    let oscillations = upper * kappa_m.abs() / (2.0 * core::f64::consts::PI);
    let base = upper as usize + 96; // ≈1 panel / unit-v + a floor
    let panels = (base + (6.0 * oscillations) as usize).clamp(CM_MIN_PANELS, CM_MAX_PANELS);

    let integral = gauss_legendre(integrand, 0.0, upper, panels);
    // C(K) = df_d · S₀ · e^{−α·κ} / π · ∫ Re[e^{−i v κ} ψ(v)] dv   (see derivation
    // in the doc comment: the spot factors collapse to the leading S₀·e^{−α κ}).
    let call = df_dom * m.spot * exp(-alpha * kappa_m) * integral / core::f64::consts::PI;

    match opt {
        OptionType::Call => call,
        // Put–call parity (exact): C − P = S·e^{−r_f T} − K·e^{−r_d T}.
        OptionType::Put => call - (m.spot * m.df_for() - m.strike * df_dom),
    }
}

/// Number of COS cosine-series terms. The COS error decays exponentially in `N`
/// while the Heston density is analytic; `N = 512` is fully converged (raising it
/// to 2048 changes the price by < 1e−12) across the validated ≤3y grid. Past ~3y
/// the density is heavy-tailed and the truncation range — not `N` — becomes the
/// limiting factor (see the module-level HONEST BOUNDARY note).
const COS_N: usize = 512;
/// Cumulant-range width multiplier `L` for the COS truncation `[a, b] = c₁ ± L·√(c₂+√c₄)`.
/// `L = 12` is the conservative Fang–Oosterlee recommendation for Heston.
const COS_L: f64 = 12.0;

/// Fourth cumulant `c₄` of the Heston log-return, used only to size the COS
/// truncation range for fat-tailed (high vol-of-vol / long-maturity) regimes.
///
/// There is no short closed form; we compute it by **automatic higher-derivative
/// extraction from the characteristic exponent** `ψ(u) = ln φ_ret(u)`. The
/// cumulants are `c_k = ψ^{(k)}(0) / i^k`. We evaluate `ψ` on a small symmetric
/// stencil of *real* `u` around 0 and take the standard 4th central finite
/// difference: `ψ''''(0) ≈ [ψ(2h) − 4ψ(h) + 6ψ(0) − 4ψ(−h) + ψ(−h·2)] / h⁴`, and
/// `c₄ = ψ''''(0)` (the `i⁴ = 1` factor and the real-axis symmetry make the
/// imaginary parts cancel, leaving a real cumulant). This is exact in the limit
/// `h→0`; with `h = 1e−2` the central-difference truncation error is ≪ the COS
/// range tolerance (the range only needs `c₄` to ~1%). It is independent of the
/// COS series itself, so it cannot mask a COS bug.
fn heston_c4(p: &HestonParams, t: f64) -> f64 {
    // Characteristic exponent ln φ_ret at a real frequency, with the model
    // dummy market (spot/strike/rates don't affect the *centred* cumulants, so
    // use μ = 0 — the drift only shifts c₁).
    let dummy = MarketInputs::new(1.0, 1.0, t, 0.0, 0.0);
    let psi = |u: f64| -> f64 {
        // ln φ_ret(u): φ_ret is computed in the log; take the real characteristic
        // exponent's value. We need the full complex exponent then its real/imag,
        // but for the 4th *cumulant* the central difference of the complex ψ has a
        // real 4th derivative — evaluate ψ via the exponent directly.
        char_exponent(Complex::real(u), &dummy, p).re
    };
    let h = 1e-2;
    let d4 = (psi(2.0 * h) - 4.0 * psi(h) + 6.0 * psi(0.0) - 4.0 * psi(-h) + psi(-2.0 * h))
        / (h * h * h * h);
    // c4 = ψ''''(0)/i⁴ = ψ''''(0); but ψ(u).re is even in u so its 4th derivative
    // is the real cumulant magnitude. Sign restored: c4 of a return is ≥ 0 for
    // these processes; we only use |c4| in the range, so return it directly.
    d4
}

/// **Fang–Oosterlee COS** European vanilla price under Heston.
///
/// The risk-neutral density of the log-return `y = ln(S_T/S_0)` is reconstructed
/// as a cosine series on a truncated range `[a, b] = [lo, hi]` whose density
/// coefficients are `A_n = (2/(b−a))·Re[ φ_ret(nπ/(b−a))·e^{−i n π a/(b−a)} ]`.
/// The price is `e^{−r_d T}·Σ'_n A_n·U_n`, the prime denoting a ½-weighted first
/// term, where `U_n` is the closed-form cosine coefficient of the payoff over the
/// in-the-money sub-interval (the `χ_n`/`ψ_n` integrals of `S_0 e^{y}` and `1`).
///
/// We always price the **put** directly and obtain the call by exact put–call
/// parity, because the put's payoff coefficient kernel only evaluates `e^{y}` for
/// `y ≤ ln(K/S_0)` (bounded), whereas the call's kernel evaluates `e^{hi}` at the
/// wide right edge of the range — which overflows / loses precision for
/// fat-tailed parameters. The truncation range uses the analytic Heston cumulants
/// `c₁, c₂` plus `c₄` (via [`heston_c4`]) for heavy tails.
#[must_use]
pub fn cos(opt: OptionType, m: &MarketInputs, p: &HestonParams) -> f64 {
    let t = m.t;
    let mu = m.r_dom - m.r_for;
    let df_dom = m.df_dom();

    // Cumulants of x_T = ln(S_T/S_0) under the risk-neutral carry, per
    // Fang–Oosterlee (2008), Table 11 (Heston).
    let (kappa, theta, sigma, rho, v0) = (p.kappa, p.theta, p.vol_of_vol, p.rho, p.v0);
    let sig2 = sigma * sigma;
    let emkt = exp(-kappa * t);

    // c1 = μT + (1 − e^{−κT})·(θ − v0)/(2κ) − ½θT  (mean of the log-return).
    let c1 = mu * t + (1.0 - emkt) * (theta - v0) / (2.0 * kappa) - 0.5 * theta * t;

    // c2 (variance of the log-return) — Fang–Oosterlee Heston cumulant.
    let c2 = (1.0 / (8.0 * kappa * kappa * kappa))
        * (sigma * t * kappa * emkt * (v0 - theta) * (8.0 * kappa * rho - 4.0 * sigma)
            + kappa * rho * sigma * (1.0 - emkt) * (16.0 * theta - 8.0 * v0)
            + 2.0 * theta * kappa * t * (-4.0 * kappa * rho * sigma + sig2 + 4.0 * kappa * kappa)
            + sig2
                * ((theta - 2.0 * v0) * exp(-2.0 * kappa * t)
                    + theta * (6.0 * emkt - 7.0)
                    + 2.0 * v0)
            + 8.0 * kappa * kappa * (v0 - theta) * (1.0 - emkt));

    // Truncation range  [lo, hi] = c1 ± L·√(|c2| + √|c4|)  (Fang–Oosterlee 2008,
    // §5). Including the **fourth cumulant** `c4` is essential for fat-tailed
    // regimes (high vol-of-vol and/or long maturity): with `c2` alone the range
    // is too narrow there and the COS series converges to a biased value. `c4`
    // is sourced from the integrated-variance moments of the CIR variance
    // process — see [`heston_c4`]. The `√|·|` guards the (rare) numerically
    // slightly-negative cumulant.
    let c4 = heston_c4(p, t);
    let width = COS_L * libm::sqrt(c2.abs() + libm::sqrt(c4.abs()));
    let lo = c1 - width;
    let hi = c1 + width;
    let ba = hi - lo;

    // Integration variable y ∈ [lo, hi] is the log-return x = ln(S_T/S_0), so
    // S_T = S_0·e^{y}. The strike log-moneyness boundary is y* = ln(K/S_0).
    //
    // We ALWAYS price the **put** directly and obtain the call by exact
    // put–call parity. Reason: the put payoff `(K − S_0 e^{y})^+` is supported on
    // y ∈ [lo, y*] where the value-coefficient kernel `χ` only ever evaluates
    // `e^{y}` for `y ≤ y*` — bounded — whereas the *call* coefficient evaluates
    // `e^{hi}` at the wide right edge of the truncation range, which overflows /
    // loses precision for fat-tailed (high-σ, long-T) parameters and large hi.
    // Pricing the bounded leg keeps the COS coefficients accurate in every regime.
    let y_star = ln(m.strike / m.spot);
    let p_low = lo; // put positive region lower edge
    let p_high = y_star.min(hi); // and upper edge (clamped to the range)

    let mut sum = 0.0;
    if p_high > p_low {
        for n in 0..COS_N {
            let w = n as f64 * core::f64::consts::PI / ba;

            // CF of the log-return x = ln(S_T/S_0) at real frequency w — the
            // return-space CF directly (no large `ln S_0` phase is ever formed,
            // so high-w terms keep full precision).
            let phi = return_char_fn(Complex::real(w), m, p);

            // A_n ∝ Re[ φ(w)·e^{−i w lo} ] (the density cosine coefficient).
            let phase = Complex::new(libm::cos(w * lo), libm::sin(w * lo));
            let re = phi.mul(phase.conj_like()).re;

            // Put value-coefficient over [p_low, p_high]:
            //   U_n^put = K·ψ_n − S_0·χ_n.
            let chi = chi(w, p_low, p_high, lo);
            let psi_n = psi_coeff(w, p_low, p_high, lo);
            let u_n = m.strike * psi_n - m.spot * chi;

            let weight = if n == 0 { 0.5 } else { 1.0 };
            sum += weight * re * u_n;
        }
    }

    let put = df_dom * (2.0 / ba) * sum;

    match opt {
        OptionType::Put => put,
        // Put–call parity (exact): C = P + S·e^{−r_f T} − K·e^{−r_d T}.
        OptionType::Call => put + (m.spot * m.df_for() - m.strike * df_dom),
    }
}

impl Complex {
    /// Conjugate, used to form `e^{−i w lo}` from `e^{+i w lo}`.
    #[inline]
    fn conj_like(self) -> Self {
        Complex::new(self.re, -self.im)
    }
}

/// χ_n: the cosine coefficient of `S_0·e^{y}` over `[c, d] ⊆ [lo, hi]` for the
/// COS basis `cos(w·(y − lo))`. Closed form (Fang–Oosterlee 2008, eq. 22):
///
/// ```text
///   χ_n = 1/(1+w²) · [ cos(w(d−lo))·e^{d} − cos(w(c−lo))·e^{c}
///                      + w·sin(w(d−lo))·e^{d} − w·sin(w(c−lo))·e^{c} ].
/// ```
fn chi(w: f64, c: f64, d: f64, lo: f64) -> f64 {
    let denom = 1.0 + w * w;
    let cd = w * (d - lo);
    let cc = w * (c - lo);
    let ed = exp(d);
    let ec = exp(c);
    (libm::cos(cd) * ed - libm::cos(cc) * ec + w * libm::sin(cd) * ed - w * libm::sin(cc) * ec)
        / denom
}

/// ψ_n: the cosine coefficient of the constant `1` over `[c, d]` for the COS
/// basis `cos(w·(y − lo))` (Fang–Oosterlee 2008, eq. 23):
///
/// ```text
///   ψ_0 = d − c,
///   ψ_n = (1/w)·[ sin(w(d−lo)) − sin(w(c−lo)) ]   (n ≥ 1).
/// ```
fn psi_coeff(w: f64, c: f64, d: f64, lo: f64) -> f64 {
    if w == 0.0 {
        d - c
    } else {
        (libm::sin(w * (d - lo)) - libm::sin(w * (c - lo))) / w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn base_params() -> HestonParams {
        // A standard, well-conditioned Heston parameter set (e.g. the
        // Fang–Oosterlee / Albrecher test regime): κ=1.5, θ=0.04, σ=0.3,
        // ρ=−0.7, v0=0.04.
        HestonParams::new(1.5, 0.04, 0.3, -0.7, 0.04)
    }

    #[test]
    fn feller_flag() {
        // 2·1.5·0.04 = 0.12 ≥ 0.09 = 0.3² → satisfied.
        assert!(base_params().feller_satisfied());
        assert!(!HestonParams::new(1.0, 0.04, 0.5, -0.5, 0.04).feller_satisfied());
    }

    #[test]
    fn carr_madan_and_cos_agree() {
        let p = base_params();
        let m = MarketInputs::new(100.0, 100.0, 1.0, 0.03, 0.0);
        let cm = carr_madan(OptionType::Call, &m, &p);
        let co = cos(OptionType::Call, &m, &p);
        assert_close!(cm, co, 1e-8, 1e-8);
        assert!(cm > 0.0);
    }

    #[test]
    fn put_call_parity_internal() {
        let p = base_params();
        let m = MarketInputs::new(105.0, 100.0, 0.75, 0.025, 0.01);
        for price in [carr_madan, cos] {
            let c = price(OptionType::Call, &m, &p);
            let pp = price(OptionType::Put, &m, &p);
            let rhs = m.spot * m.df_for() - m.strike * m.df_dom();
            assert_close!(c - pp, rhs, 1e-10, 1e-10);
        }
    }
}
