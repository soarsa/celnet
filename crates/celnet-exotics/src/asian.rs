//! Analytic (closed-form-ish) fixed-strike **arithmetic-average-rate Asian**
//! pricers — fast, MC-free, moment-matching and geometric-conditioning
//! approximations under the lognormal carry-seam dynamics (Garman-Kohlhagen for
//! FX, byte-identical).
//!
//! The arithmetic average of lognormal observations is **not** lognormal, so the
//! arithmetic-Asian option has **no exact closed form**. This module implements
//! the two market-standard fast analytic estimators, each labelled and gated at
//! its *true* accuracy:
//!
//! 1. [`turnbull_wakeman_price`] — a **lognormal moment-matching** approximation:
//!    match the first two moments of the arithmetic average `A` under
//!    Garman-Kohlhagen, then price the option as a Black-Scholes-style closed form
//!    on the moment-matched effective forward `M₁` and effective volatility
//!    `σ_a = √(ln(M₂/M₁²)/T_rem)`. Available in the continuous-averaging form
//!    ([`AveragingSchedule::Continuous`]) and the discrete-observation form
//!    ([`AveragingSchedule::Discrete`]). It is *exact* in the limits where the
//!    average degenerates to a single lognormal (one observation ⇒ the plain
//!    vanilla) and in the zero-volatility limit (the discounted intrinsic of the
//!    deterministic average).
//!
//! 2. [`curran_price`] — the **geometric-conditioning** ("Curran's method")
//!    approximation: condition the arithmetic payoff on the geometric mean `G`
//!    (which *is* lognormal) and integrate the conditional expectation in closed
//!    form. Because the arithmetic and geometric averages are strongly correlated,
//!    conditioning on `G` captures most of the payoff's structure, making this a
//!    **more accurate** independent analytic estimator than the two-moment fit,
//!    especially for low-to-moderate volatility and longer averaging windows.
//!
//! Both pricers handle the net cost-of-carry `b` (`= r_d − r_f` for FX) and the
//! **in-progress-average (seasoned)** case, where some of the averaging
//! observations have already fixed: the realised fixings enter as a deterministic
//! contribution to the average (an effective-strike shift), and only the
//! remaining future observations carry randomness.
//!
//! # Method provenance (doc comments only)
//!
//! Lognormal two-moment matching: Turnbull-Wakeman (1991), *A Quick Algorithm for
//! Pricing European Average Options*, JFQA 26(3). Geometric-conditioning:
//! Curran (1994), *Valuing Asian and Portfolio Options by Conditioning on the
//! Geometric Mean Price*, Management Science 40(12); see also Vyncke-Goovaerts-
//! Dhaene (2004) for the comonotonic-bounds framing. All identifiers here are
//! purpose-named and vendor/research-neutral; provenance lives only in docs.
//!
//! # Determinism
//!
//! Every transcendental routes through [`celnet_core::math`] (the deterministic
//! `rust-lang/libm` software backend); no float is compared with `==`. The
//! conditional integral in [`curran_price`] uses a fixed-node Gauss-Legendre
//! quadrature implemented in-crate (no external FFT/quadrature dependency).

use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::{Carry, OptionType};

use crate::inputs::{ExoticInputs, carry_vanilla_price};

/// How the averaging observations are laid out on the averaging window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AveragingSchedule {
    /// `n` equally-spaced *discrete* observations on `(t_start, T]` (the form a
    /// fixing schedule actually takes). The observation times are
    /// `t_k = t_start + k·(T − t_start)/n`, `k = 1..=n`.
    Discrete {
        /// Number of future (not-yet-fixed) observations, `≥ 1`.
        future_obs: usize,
    },
    /// **Continuous** arithmetic averaging over the remaining window
    /// `(t_start, T]` (the integral limit of the discrete form as `n → ∞`).
    Continuous,
}

/// A fixed-strike arithmetic-average-rate Asian, including the *seasoning* state
/// of an in-progress average.
///
/// The averaging period is `[0, T]` with `T = inputs.t`. If `elapsed_avg` and
/// `elapsed_weight` are non-zero the average is **seasoned**: a fraction
/// `elapsed_weight ∈ [0, 1)` of the total average weight has already fixed with
/// realised running average `elapsed_avg`, and only the remaining
/// `1 − elapsed_weight` of the weight is still random over `(t_start, T]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalyticAsian {
    /// Call or put.
    pub option: OptionType,
    /// Strike `K`.
    pub strike: f64,
    /// Layout of the *future* (still-random) averaging observations.
    pub schedule: AveragingSchedule,
    /// Start of the **remaining** averaging window as a year fraction from now;
    /// for a fresh (not-yet-started) average this is `0.0`. The remaining window
    /// is `(t_start, T]` with `T = inputs.t`.
    pub t_start: f64,
    /// Realised arithmetic average of the **already-fixed** observations (the
    /// running average). Ignored when `elapsed_weight == 0`.
    pub elapsed_avg: f64,
    /// Weight `∈ [0, 1)` of the total average already accumulated by the fixed
    /// observations. `0.0` for a fresh average; e.g. with 12 monthly fixings and
    /// 3 already fixed this is `3/12 = 0.25`.
    pub elapsed_weight: f64,
}

impl AnalyticAsian {
    /// A fresh (not-yet-started) equally-spaced discrete Asian over `n`
    /// observations on `(0, T]`.
    #[must_use]
    pub fn fresh_discrete(option: OptionType, strike: f64, observations: usize) -> Self {
        Self {
            option,
            strike,
            schedule: AveragingSchedule::Discrete {
                future_obs: observations,
            },
            t_start: 0.0,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        }
    }

    /// A fresh continuously-averaged Asian over `(0, T]`.
    #[must_use]
    pub fn fresh_continuous(option: OptionType, strike: f64) -> Self {
        Self {
            option,
            strike,
            schedule: AveragingSchedule::Continuous,
            t_start: 0.0,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        }
    }
}

/// The lognormal moment structure of the **future** (still-random) part of the
/// arithmetic average, expressed per unit of *future* weight.
///
/// `m1 = E[Ā_fut]` and `m2 = E[Ā_fut²]`, where `Ā_fut` is the weighted average of
/// the future observations only (its weights summing to 1). All carry / vol
/// dependence lives here; the seasoning shift and discounting are applied by the
/// callers.
#[derive(Debug, Clone, Copy)]
struct AverageMoments {
    m1: f64,
    m2: f64,
}

/// Future observation times on `(t_start, T]` and the per-observation weights
/// (each `1/n` for the discrete case). For the continuous case we discretise the
/// moment integrals on a dense uniform grid — the moments of the *continuous*
/// average are the `n → ∞` limit, computed to machine precision by a fine grid
/// plus the exact closed-form continuous double integral where available.
fn discrete_moments(i: &ExoticInputs, t_start: f64, n: usize) -> AverageMoments {
    let b = i.carry_rate();
    let v2 = i.vol * i.vol;
    let t_rem = i.t - t_start;
    let dt = t_rem / n as f64;
    // Observation times t_k = t_start + k·dt, k = 1..=n.
    let mut times = Vec::with_capacity(n);
    for k in 1..=n {
        times.push(t_start + k as f64 * dt);
    }
    let inv_n = 1.0 / n as f64;

    // First moment: M1 = (1/n) Σ S0 e^{b t_k}.
    let mut m1 = 0.0;
    for &tk in &times {
        m1 += exp(b * tk);
    }
    m1 *= i.spot * inv_n;

    // Second moment: M2 = (1/n²) Σ_i Σ_j S0² e^{b(t_i+t_j) + σ² min(t_i,t_j)}.
    let mut m2 = 0.0;
    for (a, &ti) in times.iter().enumerate() {
        for &tj in times.iter().skip(a) {
            // t_i ≤ t_j for j ≥ i (times sorted ascending) ⇒ min = ti.
            let term = exp(b * (ti + tj) + v2 * ti);
            // Off-diagonal pairs counted twice (symmetric), diagonal once.
            if tj > ti {
                m2 += 2.0 * term;
            } else {
                m2 += term;
            }
        }
    }
    m2 *= i.spot * i.spot * inv_n * inv_n;

    AverageMoments { m1, m2 }
}

/// Moments of the **continuous** arithmetic average `Ā = (1/τ)∫_{t_start}^{T} S_u du`
/// over the remaining window `τ = T − t_start`, in exact closed form.
///
/// First moment: `M₁ = S₀/τ ∫ e^{b(t_start+u)} du`. Second moment uses
/// `E[S_a S_c] = S₀² e^{b(a+c)+σ² min(a,c)}`:
/// `M₂ = (2 S₀²/τ²) ∫₀^τ ∫₀^s e^{b(2t_start + s + r) + σ²(t_start + r)} dr ds`,
/// integrated analytically.
fn continuous_moments(i: &ExoticInputs, t_start: f64) -> AverageMoments {
    let b = i.carry_rate();
    let v2 = i.vol * i.vol;
    let tau = i.t - t_start;
    let s0 = i.spot;

    // First moment (handle b → 0 by the limit τ).
    let m1 = if b.abs() < 1e-12 {
        s0 * exp(b * t_start)
    } else {
        s0 * exp(b * t_start) * (exp(b * tau) - 1.0) / (b * tau)
    };

    // Second moment by the double integral, shifting r' = t_start + r,
    // a' = t_start + s. With a = b (carry) we write the inner integral over r in
    // [0, s] of e^{(b)(r) + σ² r} = e^{(b+σ²) r}, then the outer over s of
    // e^{(2b) s} · (that), all times the common prefactor
    // S0² e^{(2b)t_start + σ² t_start}. Integrate analytically.
    let pref = s0 * s0 * exp(2.0 * b * t_start + v2 * t_start);
    let p = b + v2; // inner exponent rate over r ∈ [0, s]:  e^{(b+σ²) r}
    let q = b; // outer extra rate from the e^{b s} factor

    // Inner: I(s) = ∫₀^s e^{p r} dr = (e^{p s} − 1)/p   (or s if p≈0).
    // Outer: ∫₀^τ e^{q s} I(s) ds.
    //   = ∫₀^τ e^{q s} (e^{p s} − 1)/p ds
    //   = (1/p)[ (e^{(p+q)τ}−1)/(p+q) − (e^{q τ}−1)/q ]
    // with the usual limits when any rate → 0.
    let expm1_over = |rate: f64, x: f64| -> f64 {
        if rate.abs() < 1e-12 {
            x
        } else {
            (exp(rate * x) - 1.0) / rate
        }
    };
    let inner_outer = if p.abs() < 1e-12 {
        // I(s) = s ⇒ ∫₀^τ e^{q s} s ds.
        if q.abs() < 1e-12 {
            0.5 * tau * tau
        } else {
            // ∫ s e^{q s} ds = e^{q s}(q s − 1)/q² ; evaluate 0..τ.
            (exp(q * tau) * (q * tau - 1.0) + 1.0) / (q * q)
        }
    } else {
        (expm1_over(p + q, tau) - expm1_over(q, tau)) / p
    };
    let m2 = 2.0 * pref * inner_outer / (tau * tau);

    AverageMoments { m1, m2 }
}

/// Combine the future-average moments with the seasoning state into the moments
/// of the *whole* average `A = w·elapsed_avg + (1−w)·Ā_fut`, then express the
/// option on the random part with a shifted strike.
///
/// Returns `(eff_forward, eff_variance_of_log, fixed_contribution)` where
/// `eff_forward = (1−w)·M₁_fut` is the forward of the random contribution to `A`,
/// `fixed_contribution = w·elapsed_avg` the deterministic part, and the matched
/// log-variance is `ln(M₂_total/M₁_total²)` of the *random contribution*.
fn seasoned_match(spec: &AnalyticAsian, m: AverageMoments) -> (f64, f64, f64) {
    let w = spec.elapsed_weight;
    let fixed = w * spec.elapsed_avg;
    let rand_w = 1.0 - w;
    // Random contribution X = (1−w)·Ā_fut: E[X] = (1−w)M1, E[X²] = (1−w)²M2.
    let ex = rand_w * m.m1;
    let ex2 = rand_w * rand_w * m.m2;
    (ex, ex2, fixed)
}

/// Price a fixed-strike arithmetic-average-rate Asian by **lognormal two-moment
/// matching** (Turnbull-Wakeman, 1991).
///
/// The random contribution `X` to the arithmetic average is matched to a
/// lognormal with the same mean `E[X]` and second moment `E[X²]`; the payoff
/// `(φ(A − K))⁺ = (φ(X − K'))⁺` with seasoned strike `K' = K − fixed` then prices
/// with the Black formula at forward `E[X]`, total variance `ln(E[X²]/E[X]²)`,
/// discounted at `r_d`.
///
/// **Accuracy:** this is an *approximation* (the arithmetic average is not exactly
/// lognormal); it is **exact** when the random part is a single lognormal
/// (`future_obs == 1`, fresh) and in the zero-volatility limit. Typical relative
/// error vs a converged Monte-Carlo is `O(10⁻³)` at moderate vol and grows with
/// `σ²T`; see the crate parity row for the gated band.
#[must_use]
pub fn turnbull_wakeman_price(i: &ExoticInputs, spec: AnalyticAsian) -> f64 {
    let m = future_moments(i, spec);
    let (ex, ex2, fixed) = seasoned_match(&spec, m);
    let df = i.discount_df();
    let k_eff = spec.strike - fixed;

    black_on_average(spec.option, ex, ex2, k_eff, df)
}

/// Future-average moments dispatched on the schedule, with the discrete grid
/// refined for the continuous case via the exact closed form.
fn future_moments(i: &ExoticInputs, spec: AnalyticAsian) -> AverageMoments {
    match spec.schedule {
        AveragingSchedule::Discrete { future_obs } => {
            assert!(future_obs >= 1, "Asian needs ≥1 future observation");
            discrete_moments(i, spec.t_start, future_obs)
        }
        AveragingSchedule::Continuous => continuous_moments(i, spec.t_start),
    }
}

/// Black-style closed form on a lognormal whose forward is `ex = E[X]` and whose
/// second moment is `ex2 = E[X²]`, struck at `k_eff`, discounted by `df`.
///
/// Implements the exact-limit handling: if the random forward is (numerically)
/// deterministic (`ex2 ≤ ex²`, i.e. zero matched variance), the option reduces to
/// the discounted intrinsic `df·(φ(ex − k_eff))⁺`. A non-positive effective
/// strike makes a call certainly-exercised (`df·(ex − k_eff)`) and a put
/// worthless, handled by the Black formula limits directly.
fn black_on_average(option: OptionType, ex: f64, ex2: f64, k_eff: f64, df: f64) -> f64 {
    let phi = option.sign();
    // Matched log-variance of the random part.
    let ratio = if ex > 0.0 { ex2 / (ex * ex) } else { 1.0 };
    let var = if ratio > 1.0 { ln(ratio) } else { 0.0 };

    if var <= 0.0 || ex <= 0.0 {
        // Degenerate (deterministic) random part ⇒ discounted intrinsic.
        return df * (phi * (ex - k_eff)).max(0.0);
    }
    if k_eff <= 0.0 {
        // Strike below the support floor: a call is forward-minus-strike,
        // a put is worthless.
        return match phi > 0.0 {
            true => df * (ex - k_eff),
            false => 0.0,
        };
    }
    let sd = sqrt(var);
    let d1 = (ln(ex / k_eff) + 0.5 * var) / sd;
    let d2 = d1 - sd;
    df * phi * (ex * norm_cdf(phi * d1) - k_eff * norm_cdf(phi * d2))
}

/// Price a fixed-strike arithmetic-average-rate Asian by **geometric
/// conditioning** (Curran's method, 1994) — a more accurate independent analytic
/// estimator than the two-moment fit.
///
/// The geometric mean `G` of the (future) observations is lognormal. Conditioning
/// the arithmetic payoff on `G = g` and integrating over the lognormal law of `G`
/// gives, for a call,
/// ```text
///   C = df · E_G[ ( E[A | G] − K' )·Φ(d⁺(G)) ]   (Curran's conditional form)
/// ```
/// where the conditional expectation `E[S_{t_k} | G]` and the conditional
/// exercise boundary are available in closed form because `(ln S_{t_k}, ln G)` is
/// jointly Gaussian. We integrate the one-dimensional `ln G`-Gaussian by a
/// fixed-node Gauss-Legendre rule (implemented in-crate), which converges to
/// machine precision in the conditioning variable.
///
/// **Accuracy:** Curran's conditioning captures the cross-observation correlation
/// the two-moment fit only approximates, so it is **at least as accurate** as
/// [`turnbull_wakeman_price`] in the convex (call/put) regime and materially
/// better at higher `σ²T`. It is **exact** in the same degenerate limits (single
/// observation; zero vol). See the parity row for the gated bands.
#[must_use]
pub fn curran_price(i: &ExoticInputs, spec: AnalyticAsian) -> f64 {
    let (n, t_start) = match spec.schedule {
        AveragingSchedule::Discrete { future_obs } => {
            assert!(future_obs >= 1, "Asian needs ≥1 future observation");
            (future_obs, spec.t_start)
        }
        // The continuous average is the n→∞ limit; a dense discretisation of the
        // conditioning integral reproduces it to the gated tolerance.
        AveragingSchedule::Continuous => (256usize, spec.t_start),
    };

    let b = i.carry_rate();
    let v2 = i.vol * i.vol;
    let t_rem = i.t - t_start;
    let dt = t_rem / n as f64;
    let w = spec.elapsed_weight;
    let rand_w = 1.0 - w;
    let fixed = w * spec.elapsed_avg;
    let df = i.discount_df();
    let phi = spec.option.sign();
    let k_eff = spec.strike - fixed;

    // Observation times and the per-observation log-dynamics.
    //   ln S_{t_k} = ln S0 + (b − ½σ²) t_k + σ W_{t_k}.
    // Means and the covariance Cov(ln S_i, ln G).
    let mut times = Vec::with_capacity(n);
    for k in 1..=n {
        times.push(t_start + k as f64 * dt);
    }
    let inv_n = 1.0 / n as f64;
    let ln_s0 = ln(i.spot);

    // ln G = (1/n) Σ ln S_{t_k}: a Gaussian with
    //   mean μ_G = ln S0 + (b − ½σ²)·t̄,   t̄ = (1/n)Σ t_k
    //   var  σ_G² = (σ²/n²) Σ_i Σ_j min(t_i, t_j).
    // ln G is Gaussian with var σ_G² = (σ²/n²) Σ_i Σ_j min(t_i, t_j); we integrate
    // over the standardised conditioning variable z = (ln G − μ_G)/σ_G, so only
    // σ_G (and the per-observation covariances below) are needed.
    let mut var_g = 0.0;
    for &ti in &times {
        for &tj in &times {
            var_g += ti.min(tj);
        }
    }
    var_g *= v2 * inv_n * inv_n;
    let sd_g = sqrt(var_g.max(0.0));

    // Cov(ln S_{t_k}, ln G) = (σ²/n) Σ_j min(t_k, t_j).
    let mut cov_kg = Vec::with_capacity(n);
    for &tk in &times {
        let s: f64 = times.iter().map(|&tj| tk.min(tj)).sum();
        cov_kg.push(v2 * inv_n * s);
    }

    // Degenerate (zero-vol) limit: A is deterministic ⇒ discounted intrinsic.
    if sd_g <= 0.0 {
        let a_det: f64 =
            rand_w * inv_n * times.iter().map(|&tk| exp(ln_s0 + b * tk)).sum::<f64>() + fixed;
        return df * (phi * (a_det - spec.strike)).max(0.0);
    }

    // Conditional on ln G = g (with z = (g − μ_G)/σ_G standard normal):
    //   E[S_{t_k} | ln G = g]
    //     = exp( μ_k + cov_kg/σ_G² · (g − μ_G) + ½(Var(ln S_k) − cov_kg²/σ_G²) )
    // where μ_k = ln S0 + (b − ½σ²) t_k, Var(ln S_k) = σ² t_k.
    // The conditional E[A | g] = (1−w)·(1/n)Σ E[S_k | g]  + fixed is monotone in
    // z, so the exercise region {A > K} is z ≷ z*. We integrate over z by
    // Gauss-Legendre on a wide standard-normal range and apply the payoff
    // (φ(E[A|g] − K'))⁺ directly — Curran's conditioning makes the *conditional*
    // payoff the exact conditional option value (the residual conditional variance
    // of A about its conditional mean is second order and captured by the
    // moment-matched conditional lognormal correction below).
    let mu_k: Vec<f64> = times
        .iter()
        .map(|&tk| ln_s0 + (b - 0.5 * v2) * tk)
        .collect();
    let var_k: Vec<f64> = times.iter().map(|&tk| v2 * tk).collect();
    let inv_var_g = 1.0 / var_g;

    // Conditional mean and conditional second moment of A given z, so we can apply
    // a *conditional* Black correction for the residual within-G dispersion
    // (this is what lifts Curran above a pure boundary integral and gives the
    // higher accuracy). For each z:
    //   cond_mean_k = exp( μ_k + cov_kg·invσG²·(z·σ_G) + ½ res_k ),
    //     res_k = var_k − cov_kg²·invσG²   (≥ 0, conditional variance of ln S_k)
    //   E[A|z]     = (1−w)/n Σ cond_mean_k
    //   E[A²|z]    = ((1−w)/n)² Σ_i Σ_j exp( m_i+m_j + ½(res_i+res_j) + c_ij )
    //     m_k = μ_k + cov_kg·invσG²·(z σ_G),  c_ij = conditional Cov(lnS_i,lnS_j).
    // Conditional Cov(ln S_i, ln S_j | G)
    //   = σ² min(t_i,t_j) − cov_iG·cov_jG·invσG².
    let res_k: Vec<f64> = (0..n)
        .map(|k| (var_k[k] - cov_kg[k] * cov_kg[k] * inv_var_g).max(0.0))
        .collect();

    // Pre-compute the conditional second-moment covariance matrix once (z-free).
    let scale = rand_w * inv_n;
    let mut c_mat = vec![0.0f64; n * n];
    for ii in 0..n {
        for jj in 0..n {
            c_mat[ii * n + jj] =
                v2 * times[ii].min(times[jj]) - cov_kg[ii] * cov_kg[jj] * inv_var_g;
        }
    }

    // The per-`z` conditional option value. The payoff is
    //   (φ(A − K))⁺ = (φ(R − K'))⁺,   R = scale·Σ S_k  (the random part),
    //   K' = K − fixed  (the seasoned effective strike),
    // so the conditional Black is priced on the **random part's** conditional
    // moments (its conditional mean `rand_mean` and second moment `rand_2`) struck
    // at `k_eff` — the deterministic `fixed` contribution is a pure strike shift
    // and must NOT enter the lognormal moments.
    //   rand_mean = scale·Σ exp(m_k + ½ res_k)
    //   rand_2    = scale²·Σ_ij exp(m_i+m_j+½(res_i+res_j)+c_ij)
    // This is smooth in z away from the conditional exercise boundary.
    let cond_value = |z: f64| -> f64 {
        let shift = z * sd_g;
        let mk: Vec<f64> = (0..n)
            .map(|k| mu_k[k] + cov_kg[k] * inv_var_g * shift)
            .collect();
        let mut cond_mean = 0.0;
        for k in 0..n {
            cond_mean += exp(mk[k] + 0.5 * res_k[k]);
        }
        let mut e_sum2 = 0.0;
        for ii in 0..n {
            for jj in 0..n {
                e_sum2 += exp(mk[ii] + mk[jj] + 0.5 * (res_k[ii] + res_k[jj]) + c_mat[ii * n + jj]);
            }
        }
        let rand_mean = scale * cond_mean;
        let rand_2 = scale * scale * e_sum2;
        black_on_average(spec.option, rand_mean, rand_2, k_eff, 1.0)
    };

    // The conditional mean E[A|z] is monotone increasing in z, so there is a
    // single conditional exercise boundary z* where E[A|z*] = K'. Splitting the
    // outer Gaussian integral at z* makes each piece smooth (the conditional Black
    // value degenerates to a kinked intrinsic only in the zero-residual limit, and
    // that kink sits exactly at z*). We bisect for z* on a wide window.
    let zmax = 8.0;
    let cond_mean_only = |z: f64| -> f64 {
        let shift = z * sd_g;
        let mut cond_mean = 0.0;
        for k in 0..n {
            cond_mean += exp(mu_k[k] + cov_kg[k] * inv_var_g * shift + 0.5 * res_k[k]);
        }
        scale * cond_mean + fixed
    };
    // Boundary where the *total* conditional average A crosses the strike K
    // (cond_mean_only already includes the seasoned `fixed` contribution).
    let (mut lo, mut hi) = (-zmax, zmax);
    let f_lo = cond_mean_only(lo) - spec.strike;
    let f_hi = cond_mean_only(hi) - spec.strike;
    let z_star = if f_lo * f_hi >= 0.0 {
        // No interior crossing on the window: integrate as a single smooth piece.
        if f_hi < 0.0 { hi } else { lo }
    } else {
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if (cond_mean_only(mid) - spec.strike) * (cond_mean_only(lo) - spec.strike) <= 0.0 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        0.5 * (lo + hi)
    };

    // Integrate ∫ pdf(z)·cond_value(z) dz over [-zmax, z*] and [z*, zmax] with the
    // 64-node Gauss-Legendre rule on each (smooth) sub-interval.
    let (nodes, gl_w) = gauss_legendre_64();
    let integrate = |a: f64, c: f64| -> f64 {
        if c - a <= 0.0 {
            return 0.0;
        }
        let half = 0.5 * (c - a);
        let mid = 0.5 * (a + c);
        let mut acc = 0.0;
        for (gn, gw) in nodes.iter().zip(gl_w.iter()) {
            let z = mid + half * gn;
            acc += gw * half * norm_pdf(z) * cond_value(z);
        }
        acc
    };
    let price = integrate(-zmax, z_star) + integrate(z_star, zmax);
    df * price
}

/// 64-node Gauss-Legendre quadrature on `[-1, 1]`, computed once via the
/// Newton-iterated Legendre roots (deterministic, in-crate — no external
/// quadrature dependency). Returns `(nodes, weights)`.
fn gauss_legendre_64() -> (Vec<f64>, Vec<f64>) {
    const N: usize = 64;
    let mut nodes = vec![0.0; N];
    let mut weights = vec![0.0; N];
    // Roots are symmetric; compute the positive half and mirror.
    let m = N.div_ceil(2);
    for idx in 0..m {
        // Initial guess (Chebyshev-like asymptotic).
        let i_f = idx as f64;
        let mut x = libm::cos(std::f64::consts::PI * (i_f + 0.75) / (N as f64 + 0.5));
        // Newton iteration on the Legendre polynomial P_N.
        for _ in 0..100 {
            let (p, dp) = legendre_p_dp(N, x);
            let dx = -p / dp;
            x += dx;
            if dx.abs() < 1e-15 {
                break;
            }
        }
        let (_, dp) = legendre_p_dp(N, x);
        let wgt = 2.0 / ((1.0 - x * x) * dp * dp);
        // x is the (idx)-th positive-side root counting from +1 inward.
        nodes[idx] = -x;
        nodes[N - 1 - idx] = x;
        weights[idx] = wgt;
        weights[N - 1 - idx] = wgt;
    }
    (nodes, weights)
}

/// Legendre polynomial `P_n(x)` and its derivative `P_n'(x)` via the upward
/// recurrence (deterministic three-term recurrence).
fn legendre_p_dp(n: usize, x: f64) -> (f64, f64) {
    let mut p0 = 1.0;
    let mut p1 = x;
    if n == 0 {
        return (1.0, 0.0);
    }
    for k in 2..=n {
        let kf = k as f64;
        let p2 = ((2.0 * kf - 1.0) * x * p1 - (kf - 1.0) * p0) / kf;
        p0 = p1;
        p1 = p2;
    }
    // P_n' = n (x P_n − P_{n-1}) / (x² − 1).
    let dp = n as f64 * (x * p1 - p0) / (x * x - 1.0);
    (p1, dp)
}

/// The analytic **geometric**-average-rate Asian closed form, recomputed here on
/// the [`AnalyticAsian`] schedule (fresh, equally-spaced discrete observations),
/// used as an *exact* in-limit oracle in the parity row.
///
/// This is the Kemna-Vorst lognormal-of-the-geometric-average closed form and is
/// **exact** (the geometric average of lognormals is lognormal). It is provided
/// here so the parity row can assert the analytic geometric leg against the
/// crate's existing [`crate::geometric_asian_price`] to `~1e-12`.
#[must_use]
pub fn geometric_average_price(i: &ExoticInputs, spec: AnalyticAsian) -> f64 {
    let n = match spec.schedule {
        AveragingSchedule::Discrete { future_obs } => future_obs,
        AveragingSchedule::Continuous => {
            // Continuous geometric average: σ_G² = σ²/3, μ adjustment = b/2 − σ²/12.
            return continuous_geometric_price(i, spec.strike, spec.option);
        }
    };
    assert!(spec.t_start == 0.0, "fresh geometric oracle only");
    let nf = n as f64;
    let t = i.t;
    let b = i.carry_rate();
    let sig2 = i.vol * i.vol * (nf + 1.0) * (2.0 * nf + 1.0) / (6.0 * nf * nf);
    let eff_vol = sqrt(sig2);
    let eff_b = 0.5 * (b - 0.5 * i.vol * i.vol) * (nf + 1.0) / nf + 0.5 * sig2;
    // Synthetic cost-of-carry recast: keep the numeraire discount rate, set the
    // net carry to `eff_b`. Asset-class-agnostic; for FX byte-identical to the
    // historical `r_for = r_dom − eff_b` recast because the carry-seam vanilla
    // prices in the `(r, q = r − b)` form (see [`carry_vanilla_price`]).
    let synthetic = ExoticInputs {
        spot: i.spot,
        strike: spec.strike,
        vol: eff_vol,
        t,
        underlying: i.underlying.clone(),
        carry: Carry::CostOfCarry {
            r: i.discount_rate(),
            b: eff_b,
        },
    };
    carry_vanilla_price(spec.option, &synthetic)
}

/// Continuous geometric-average-rate Asian closed form (the `n → ∞` Kemna-Vorst
/// limit): `σ_G² = σ²/3`, effective carry `b_G = ½(b − σ²/6)`.
fn continuous_geometric_price(i: &ExoticInputs, strike: f64, option: OptionType) -> f64 {
    let b = i.carry_rate();
    let v2 = i.vol * i.vol;
    let eff_vol = sqrt(v2 / 3.0);
    let eff_b = 0.5 * (b - v2 / 6.0);
    // Synthetic cost-of-carry recast (see `geometric_average_price`).
    let synthetic = ExoticInputs {
        spot: i.spot,
        strike,
        vol: eff_vol,
        t: i.t,
        underlying: i.underlying.clone(),
        carry: Carry::CostOfCarry {
            r: i.discount_rate(),
            b: eff_b,
        },
    };
    carry_vanilla_price(option, &synthetic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;
    use celnet_types::VanillaInputs;

    fn base() -> ExoticInputs {
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02).into()
    }

    /// Single future observation ⇒ the average IS the terminal spot ⇒ both
    /// analytic pricers reduce to the plain Garman-Kohlhagen vanilla, exactly.
    #[test]
    fn single_observation_is_vanilla() {
        let i = base();
        let spec = AnalyticAsian::fresh_discrete(OptionType::Call, 100.0, 1);
        let vanilla = celnet_vanilla::price(OptionType::Call, &i.as_fx_vanilla(i.strike).unwrap());
        let tw = turnbull_wakeman_price(&i, spec);
        let cur = curran_price(&i, spec);
        assert_close!(tw, vanilla, 1e-10, 1e-10);
        assert_close!(cur, vanilla, 1e-9, 1e-9);
    }

    /// Zero-vol ⇒ discounted intrinsic on the deterministic average.
    #[test]
    fn zero_vol_is_discounted_intrinsic() {
        let mut i = base();
        i.vol = 0.0;
        let spec = AnalyticAsian::fresh_discrete(OptionType::Call, 100.0, 12);
        // Deterministic average of S0 e^{b t_k}.
        let b = i.carry_rate();
        let n = 12;
        let mut a = 0.0;
        for k in 1..=n {
            a += i.spot * exp(b * (k as f64) * i.t / n as f64);
        }
        a /= n as f64;
        let df = i.discount_df();
        let expected = df * (a - 100.0).max(0.0);
        let tw = turnbull_wakeman_price(&i, spec);
        let cur = curran_price(&i, spec);
        assert_close!(tw, expected, 1e-10, 1e-10);
        assert_close!(cur, expected, 1e-9, 1e-9);
    }

    /// Curran and Turnbull-Wakeman agree within their documented cross-method
    /// band on a standard at-the-money Asian.
    #[test]
    fn curran_tw_agree_atm() {
        let i = base();
        let spec = AnalyticAsian::fresh_discrete(OptionType::Call, 100.0, 12);
        let tw = turnbull_wakeman_price(&i, spec);
        let cur = curran_price(&i, spec);
        let rel = (tw - cur).abs() / cur;
        assert!(rel < 5e-3, "TW {tw} vs Curran {cur} rel {rel}");
    }

    /// Put-call parity for the Asian: C − P = df·(E[A] − K).
    #[test]
    fn asian_put_call_parity() {
        let i = base();
        let call = AnalyticAsian::fresh_discrete(OptionType::Call, 95.0, 12);
        let put = AnalyticAsian::fresh_discrete(OptionType::Put, 95.0, 12);
        // E[A] via the first moment.
        let m = future_moments(&i, call);
        let df = i.discount_df();
        let parity = df * (m.m1 - 95.0);
        let c = turnbull_wakeman_price(&i, call);
        let p = turnbull_wakeman_price(&i, put);
        assert_close!(c - p, parity, 1e-9, 1e-9);
        let cc = curran_price(&i, call);
        let pp = curran_price(&i, put);
        assert_close!(cc - pp, parity, 1e-7, 1e-7);
    }

    /// Continuous geometric leg matches the discrete one in the dense limit.
    #[test]
    fn continuous_geometric_is_dense_discrete_limit() {
        let i = base();
        let cont = continuous_geometric_price(&i, 100.0, OptionType::Call);
        let dense = geometric_average_price(
            &i,
            AnalyticAsian::fresh_discrete(OptionType::Call, 100.0, 4000),
        );
        assert_close!(cont, dense, 1e-3, 1e-3);
    }

    /// Gauss-Legendre integrates the standard normal density to 1 over the wide
    /// window — sanity on the in-crate quadrature.
    #[test]
    fn gauss_legendre_integrates_normal() {
        let (nodes, w) = gauss_legendre_64();
        let zmax = 8.0;
        let mut total = 0.0;
        for (n, w) in nodes.iter().zip(w.iter()) {
            let z = zmax * n;
            total += w * zmax * norm_pdf(z);
        }
        assert_close!(total, 1.0, 1e-10, 1e-10);
    }

    /// Seasoned average: with a fully-fixed average (weight → 1) the option is a
    /// deterministic discounted intrinsic on the realised running average.
    #[test]
    fn fully_seasoned_is_intrinsic() {
        let i = base();
        let spec = AnalyticAsian {
            option: OptionType::Call,
            strike: 100.0,
            schedule: AveragingSchedule::Discrete { future_obs: 1 },
            t_start: i.t - 1e-6,
            elapsed_avg: 105.0,
            elapsed_weight: 1.0 - 1e-9,
        };
        let df = i.discount_df();
        let tw = turnbull_wakeman_price(&i, spec);
        // Almost entirely the fixed 105 average ⇒ ≈ df·(105 − 100).
        assert_close!(tw, df * 5.0, 1e-3, 1e-3);
    }
}
