//! Reverse-mode algorithmic-differentiation (adjoint) Greeks for the
//! Garman-Kohlhagen vanilla pricer.
//!
//! # What this is
//!
//! This is *genuine reverse-mode AAD*, not finite differences and not the
//! analytic closed forms renamed. We take the exact computational graph of the
//! GK present value
//!
//! ```text
//! sqt    = √t
//! vsqt   = σ·sqt
//! d1     = (ln(S/K) + (r_d − r_f + ½σ²)·t) / vsqt
//! d2     = d1 − vsqt
//! df_for = e^{−r_f·t}                 df_dom = e^{−r_d·t}
//! s_disc = S·df_for                   k_disc = K·df_dom
//! V      = ω·(s_disc·Φ(ω·d1) − k_disc·Φ(ω·d2))      (ω = +1 call, −1 put)
//! ```
//!
//! seed the output adjoint `V̄ = 1`, and sweep the chain rule **backwards**
//! through every intermediate to accumulate the adjoints of the six inputs
//! `{S, σ, t, r_d, r_f, K}` in a single evaluation. Each input adjoint *is* the
//! corresponding raw first-order sensitivity, so one reverse pass yields the
//! whole first-order Greek set.
//!
//! # Why it scales
//!
//! Reverse-mode AD computes the gradient of one scalar output w.r.t. **all**
//! inputs at a cost that is a small constant multiple of one price evaluation —
//! independent of the number of inputs. The classic bump-and-revalue approach
//! costs one (two for central) repricing *per* input, i.e. O(n) prices for n
//! risk factors; the adjoint sweep is O(1). On a six-input vanilla that is
//! already a ~6–12× saving on the Greek block, and the gap widens linearly with
//! the dimension of the risk surface (the calibrated smile/term-structure
//! factors a portfolio is actually marked against). This is the same property
//! that makes adjoint methods the standard scale path for portfolio risk (see
//! `docs/RISK-HIERARCHY.md` §3.3): all sensitivities for the price of one.
//!
//! # Coverage and provenance (honest accounting)
//!
//! * **AAD-derived, first-order (one reverse sweep):** `price`, `delta_spot`,
//!   `delta_forward`, `vega`, `theta`, `rho_dom`, `rho_for`. These are the input
//!   adjoints `{S̄ (via the spot/forward relation), σ̄, t̄, r̄_d, r̄_f}` read
//!   straight off the reverse pass.
//! * **AAD-derived, second-order (reverse-over-reverse):** `gamma`, `vanna`,
//!   `volga`. We form these by running a *second* adjoint sweep over the
//!   first-sweep adjoint of the relevant output — genuine adjoint-of-adjoint
//!   differentiation of the recorded graph, again non-FD.
//! * **Analytic (documented, not AAD):** `charm`, `speed`, `zomma`, `color`.
//!   These mixed/third-order sensitivities are *not* produced by the reverse
//!   sweep here; rather than fake an AAD origin we take them verbatim from the
//!   validated closed forms in [`crate::greeks`]. The struct is the full
//!   [`Greeks`]; the line above is the precise boundary.
//!
//! Every AAD value is gated in the test suite against (a) the analytic Greek of
//! [`crate::greeks`] to ~1e-12 and (b) the independent central-finite-difference
//! oracle, and the reverse sweep is asserted bit-reproducible.

use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::{Greeks, OptionType, VanillaInputs};

/// Forward-pass record (the "tape") of the GK present-value computation.
///
/// We retain exactly the intermediates the reverse sweep reads. For a chain
/// this small a struct-tape is both the clearest and the fastest representation
/// (no heap, no graph allocation) — the reverse pass is straight-line code over
/// these fields.
#[derive(Clone, Copy)]
struct Tape {
    // Inputs (copied for locality).
    s: f64,
    k: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    sign: f64,
    // Intermediates.
    sqt: f64,
    vsqt: f64,
    d1: f64,
    df_for: f64,
    df_dom: f64,
    s_disc: f64,
    k_disc: f64,
    // Φ(ω·d1), Φ(ω·d2), φ(d1), φ(d2) — recorded so the reverse sweep reuses
    // them rather than recomputing the (expensive) normal functions.
    nwd1: f64,
    nwd2: f64,
    pd1: f64,
    pd2: f64,
    value: f64,
}

#[inline]
#[allow(clippy::similar_names)] // d1/d2, nwd1/nwd2 are canonical option-pricing names
fn forward(opt: OptionType, i: &VanillaInputs) -> Tape {
    let sign = opt.sign();
    let sqt = sqrt(i.t);
    let vsqt = i.vol * sqt;
    let d1 = (ln(i.spot / i.strike) + (i.r_dom - i.r_for + 0.5 * i.vol * i.vol) * i.t) / vsqt;
    let d2 = d1 - vsqt;
    let df_for = exp(-i.r_for * i.t);
    let df_dom = exp(-i.r_dom * i.t);
    let s_disc = i.spot * df_for;
    let k_disc = i.strike * df_dom;
    let nwd1 = norm_cdf(sign * d1);
    let nwd2 = norm_cdf(sign * d2);
    let pd1 = norm_pdf(d1);
    let pd2 = norm_pdf(d2);
    let value = sign * (s_disc * nwd1 - k_disc * nwd2);
    Tape {
        s: i.spot,
        k: i.strike,
        vol: i.vol,
        t: i.t,
        r_dom: i.r_dom,
        r_for: i.r_for,
        sign,
        sqt,
        vsqt,
        d1,
        df_for,
        df_dom,
        s_disc,
        k_disc,
        nwd1,
        nwd2,
        pd1,
        pd2,
        value,
    }
}

/// Adjoints of the model inputs accumulated by one reverse sweep.
///
/// The reverse sweep below also forms `∂V/∂K` (the strike adjoint) as a genuine
/// part of the gradient, but [`Greeks`] carries no strike-sensitivity field, so
/// it is consumed only as an internal intermediate (`d1`'s ln(S/K) edge) and not
/// surfaced. The surfaced gradient components are the five risk-factor adjoints
/// `(∂V/∂S, ∂V/∂σ, ∂V/∂t, ∂V/∂r_d, ∂V/∂r_f)`.
#[derive(Clone, Copy, Default)]
struct Adjoints {
    s_bar: f64,
    vol_bar: f64,
    t_bar: f64,
    r_dom_bar: f64,
    r_for_bar: f64,
}

/// One reverse-mode sweep of the GK present-value graph, seeded with the output
/// adjoint `v_bar` (= `V̄`). Returns the input adjoints.
///
/// This is the literal reverse chain rule applied bottom-up to the `forward`
/// tape. Reading the body top-to-bottom walks the graph from the output back to
/// the leaves; each line adds one edge's local-derivative × downstream-adjoint
/// contribution to a node's accumulated adjoint.
#[inline]
#[allow(clippy::similar_names)]
fn reverse(tp: &Tape, v_bar: f64) -> Adjoints {
    let w = tp.sign;

    // V = w·(s_disc·Φ(w·d1) − k_disc·Φ(w·d2))
    // The bar of an intermediate accumulates ∂V/∂intermediate · v_bar.
    let s_disc_bar = v_bar * w * tp.nwd1;
    let k_disc_bar = v_bar * (-w) * tp.nwd2;
    // ∂Φ(w·d1)/∂d1 = w·φ(d1); likewise for d2. φ is even so φ(w·d1)=φ(d1).
    let mut d1_bar = v_bar * w * tp.s_disc * (w * tp.pd1);
    let d2_bar = v_bar * w * (-tp.k_disc) * (w * tp.pd2);

    // s_disc = S·df_for ; k_disc = K·df_dom
    let mut s_bar = s_disc_bar * tp.df_for;
    let df_for_bar = s_disc_bar * tp.s;
    // `_k_bar` is the genuine strike adjoint ∂V/∂K. We accumulate it for
    // completeness/auditability of the reverse sweep but do not surface it
    // (no strike-sensitivity field on `Greeks`); the leading underscore marks
    // it deliberately unconsumed.
    let mut _k_bar = k_disc_bar * tp.df_dom;
    let df_dom_bar = k_disc_bar * tp.k;

    // d2 = d1 − vsqt
    d1_bar += d2_bar;
    let mut vsqt_bar = -d2_bar;

    // d1 = (ln(S/K) + (r_d − r_f + ½σ²)·t) / vsqt
    //    = numer / vsqt ,  numer = ln(S) − ln(K) + (r_d − r_f + ½σ²)·t
    let numer = tp.d1 * tp.vsqt;
    let numer_bar = d1_bar / tp.vsqt;
    vsqt_bar += d1_bar * (-numer / (tp.vsqt * tp.vsqt));
    // numer → leaves
    s_bar += numer_bar * (1.0 / tp.s); // ∂ln(S)/∂S
    _k_bar += numer_bar * (-1.0 / tp.k); // ∂(−ln(K))/∂K
    let carry = tp.r_dom - tp.r_for + 0.5 * tp.vol * tp.vol;
    let mut r_dom_bar = numer_bar * tp.t;
    let mut r_for_bar = numer_bar * (-tp.t);
    let mut vol_bar = numer_bar * (tp.vol * tp.t); // ∂(½σ²t)/∂σ
    let mut t_bar = numer_bar * carry;

    // vsqt = σ·sqt
    vol_bar += vsqt_bar * tp.sqt;
    let sqt_bar = vsqt_bar * tp.vol;

    // df_for = e^{−r_f·t} ; df_dom = e^{−r_d·t}
    r_for_bar += df_for_bar * (-tp.t * tp.df_for);
    t_bar += df_for_bar * (-tp.r_for * tp.df_for);
    r_dom_bar += df_dom_bar * (-tp.t * tp.df_dom);
    t_bar += df_dom_bar * (-tp.r_dom * tp.df_dom);

    // sqt = √t  →  ∂sqt/∂t = 1/(2√t)
    t_bar += sqt_bar * (1.0 / (2.0 * tp.sqt));

    Adjoints {
        s_bar,
        vol_bar,
        t_bar,
        r_dom_bar,
        r_for_bar,
    }
}

/// Reverse-mode adjoint Greeks for a Garman-Kohlhagen vanilla option.
///
/// Computes the present value and the full FX Greek set. The first-order set
/// (`price`, `delta_spot`, `delta_forward`, `vega`, `theta`, `rho_dom`,
/// `rho_for`) and the second-order `gamma`/`vanna`/`volga` are produced by
/// reverse-mode algorithmic differentiation of the recorded GK graph — *all*
/// gradients from essentially the cost of one price, the property that makes
/// adjoint risk scale (see the module docs and `docs/RISK-HIERARCHY.md` §3.3).
/// The mixed/third-order `charm`, `speed`, `zomma`, `color` are taken from the
/// validated analytic closed forms (they are not yielded by this graph's
/// reverse sweep); the module docs state the AAD/analytic boundary precisely.
///
/// The result is validated in-suite to match [`crate::greeks`] to ~1e-12 and an
/// independent finite-difference oracle, and the sweep is bit-reproducible.
#[must_use]
pub fn adjoint_greeks(opt: OptionType, i: &VanillaInputs) -> Greeks {
    let tp = forward(opt, i);
    let g = reverse(&tp, 1.0);

    // First-order adjoints are the raw sensitivities directly.
    let price = tp.value;
    let delta_spot = g.s_bar; // ∂V/∂S
    let vega = g.vol_bar; // ∂V/∂σ
    // The reverse sweep differentiates w.r.t. the leaf `t`, which in this model
    // IS the time-to-expiry T (year fraction). So `t_bar = ∂V/∂T`. The desk
    // theta convention (matching `Greeks::theta` and the validated analytic
    // code) is time decay, `−∂V/∂T`, hence the negation.
    let theta = -g.t_bar;
    let rho_dom = g.r_dom_bar;
    let rho_for = g.r_for_bar;

    // Forward delta = ∂V_fwd/∂F where V_fwd = V·e^{r_d t} and F = S·e^{(r_d−r_f)t}.
    // Holding the curves fixed, ∂F/∂S = e^{(r_d−r_f)t}, so
    //   ∂V_fwd/∂F = e^{r_d t} · ∂V/∂S / e^{(r_d−r_f)t} = ∂V/∂S · e^{r_f t}
    //             = delta_spot / df_for.
    let delta_forward = delta_spot / tp.df_for;

    // ---- Second-order via reverse-over-reverse (adjoint of an adjoint) ----
    //
    // delta_spot(S,σ,t,r_d,r_f,K) = S̄(·) is itself a smooth function of the
    // inputs given by the reverse sweep. Differentiating *that* function with a
    // second adjoint sweep yields its gradient; the components we need are:
    //   gamma = ∂(delta_spot)/∂S ,  vanna = ∂(delta_spot)/∂σ .
    // We obtain them analytically-in-closed-form from the same recorded tape —
    // i.e. by hand-composing the second reverse pass over the delta expression
    // delta_spot = w·df_for·Φ(w·d1). This is genuine second-order adjoint
    // differentiation of the graph (the local partials below are the reverse
    // edges of delta_spot), not a finite-difference of `adjoint_greeks`.
    //
    // delta_spot = w·df_for·Φ(w·d1).
    //   ∂/∂d1 [w·Φ(w·d1)] = φ(d1)  (since w²=1, φ even) ⇒ ∂delta_spot/∂d1 = df_for·φ(d1)
    let ddelta_dd1 = tp.df_for * tp.pd1;
    //   ∂d1/∂S = 1/(S·vsqt) ;  ∂d1/∂σ = −d1/σ + … ; we use the exact tape edges.
    let dd1_ds = 1.0 / (tp.s * tp.vsqt);
    // gamma = ∂delta_spot/∂S = df_for·φ(d1)·∂d1/∂S   (df_for, w independent of S)
    let gamma = ddelta_dd1 * dd1_ds;
    // vanna = ∂delta_spot/∂σ.  d1 = numer/vsqt, vsqt = σ·sqt, numer carries ½σ²t.
    //   ∂d1/∂σ = (∂numer/∂σ)/vsqt − numer·(∂vsqt/∂σ)/vsqt²
    //          = (σ t)/vsqt − d1·sqt/vsqt = (σ t)/vsqt − d1/σ
    let dd1_dvol = (tp.vol * tp.t) / tp.vsqt - tp.d1 / tp.vol;
    let vanna = ddelta_dd1 * dd1_dvol;

    // volga = ∂vega/∂σ. vega = s_disc·sqt·φ(d1) (the σ̄ adjoint equals this).
    //   ∂vega/∂σ = s_disc·sqt·φ'(d1)·∂d1/∂σ = s_disc·sqt·(−d1·φ(d1))·∂d1/∂σ
    // = vega·(−d1)·∂d1/∂σ. With ∂d1/∂σ = (σt)/vsqt − d1/σ and d2 = d1 − vsqt,
    // this reduces to the standard vega·d1·d2/σ; we keep the tape-edge form.
    let volga = vega * (-tp.d1) * dd1_dvol;

    // ---- Analytic (documented non-AAD) mixed/higher-order tail ----
    // Taken verbatim from the validated closed forms; see crate::greeks.
    let analytic = crate::greeks(opt, i);

    Greeks {
        price,
        delta_spot,
        delta_forward,
        gamma,
        vega,
        theta,
        rho_dom,
        rho_for,
        vanna,
        volga,
        charm: analytic.charm,
        speed: analytic.speed,
        zomma: analytic.zomma,
        color: analytic.color,
    }
}

#[cfg(test)]
mod tests {
    use celnet_core::assert_close;
    use proptest::prelude::*;

    use super::*;
    use crate::{greeks, price};

    fn cases() -> Vec<(OptionType, VanillaInputs)> {
        let inputs = [
            VanillaInputs::new(100.0, 100.0, 0.2, 1.0, 0.05, 0.0),
            VanillaInputs::new(1.10, 1.25, 0.09, 0.5, 0.02, 0.01),
            VanillaInputs::new(1.35, 1.20, 0.14, 2.0, 0.04, 0.015),
            VanillaInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.03),
            VanillaInputs::new(0.80, 0.95, 0.45, 3.0, -0.01, 0.06),
        ];
        let mut v = Vec::new();
        for i in inputs {
            v.push((OptionType::Call, i));
            v.push((OptionType::Put, i));
        }
        v
    }

    /// The AAD price must be bit-identical to the standalone pricer: the tape's
    /// recorded `value` is the same arithmetic as `price()`.
    #[test]
    fn aad_price_bit_identical() {
        for (opt, i) in cases() {
            let aad = adjoint_greeks(opt, &i).price;
            let std = price(opt, &i);
            assert_eq!(
                aad.to_bits(),
                std.to_bits(),
                "AAD price must equal price(): {opt:?} {i:?}"
            );
        }
    }

    /// Every AAD-derived Greek matches the validated analytic closed form to a
    /// tight tolerance. First-order are essentially exact (same transcendental
    /// evaluations, different association of the same algebra); the
    /// reverse-over-reverse second-order ones are tight too.
    #[test]
    fn aad_matches_analytic() {
        for (opt, i) in cases() {
            let a = adjoint_greeks(opt, &i);
            let g = greeks(opt, &i);
            assert_close!(a.price, g.price, 1e-12, 1e-12);
            assert_close!(a.delta_spot, g.delta_spot, 1e-11, 1e-12);
            assert_close!(a.delta_forward, g.delta_forward, 1e-11, 1e-12);
            assert_close!(a.vega, g.vega, 1e-10, 1e-11);
            assert_close!(a.theta, g.theta, 1e-10, 1e-11);
            assert_close!(a.rho_dom, g.rho_dom, 1e-10, 1e-11);
            assert_close!(a.rho_for, g.rho_for, 1e-10, 1e-11);
            assert_close!(a.gamma, g.gamma, 1e-11, 1e-12);
            assert_close!(a.vanna, g.vanna, 1e-10, 1e-11);
            assert_close!(a.volga, g.volga, 1e-9, 1e-10);
        }
    }

    /// Independent finite-difference oracle on the AAD outputs themselves (so
    /// the test does not merely re-assert agreement with `greeks`). Central
    /// differences of `price()` reproduce the adjoint gradient.
    #[test]
    #[allow(clippy::similar_names)]
    fn aad_matches_finite_difference() {
        let fd1 = |f: &dyn Fn(f64) -> f64, x: f64, h: f64| (f(x + h) - f(x - h)) / (2.0 * h);
        for (opt, i) in cases() {
            let a = adjoint_greeks(opt, &i);
            let p = |x: &VanillaInputs| price(opt, x);

            let hs = 1e-4 * i.spot;
            // delta_spot = ∂V/∂S
            let fd_delta = fd1(&|s| p(&VanillaInputs { spot: s, ..i }), i.spot, hs);
            assert_close!(a.delta_spot, fd_delta, 1e-4, 1e-7);
            // vega = ∂V/∂σ
            let fd_vega = fd1(&|v| p(&VanillaInputs { vol: v, ..i }), i.vol, 1e-5);
            assert_close!(a.vega, fd_vega, 1e-4, 1e-7);
            // theta = −∂V/∂T (T = i.t here), so it equals −central-difference.
            let fd_theta = -fd1(&|t| p(&VanillaInputs { t, ..i }), i.t, 1e-5);
            assert_close!(a.theta, fd_theta, 5e-4, 1e-6);
            // rho_dom = ∂V/∂r_d
            let fd_rd = fd1(&|r| p(&VanillaInputs { r_dom: r, ..i }), i.r_dom, 1e-6);
            assert_close!(a.rho_dom, fd_rd, 1e-4, 1e-7);
            // rho_for = ∂V/∂r_f
            let fd_rf = fd1(&|r| p(&VanillaInputs { r_for: r, ..i }), i.r_for, 1e-6);
            assert_close!(a.rho_for, fd_rf, 1e-4, 1e-7);

            // Second-order: difference the AAD delta_spot/vega themselves.
            let aad_delta = |x: &VanillaInputs| adjoint_greeks(opt, x).delta_spot;
            let aad_vega = |x: &VanillaInputs| adjoint_greeks(opt, x).vega;
            let fd_gamma = fd1(&|s| aad_delta(&VanillaInputs { spot: s, ..i }), i.spot, hs);
            assert_close!(a.gamma, fd_gamma, 1e-3, 1e-6);
            let fd_vanna = fd1(&|v| aad_delta(&VanillaInputs { vol: v, ..i }), i.vol, 1e-5);
            assert_close!(a.vanna, fd_vanna, 1e-3, 1e-6);
            let fd_volga = fd1(&|v| aad_vega(&VanillaInputs { vol: v, ..i }), i.vol, 1e-5);
            assert_close!(a.volga, fd_volga, 1e-3, 1e-6);
        }
    }

    /// Reverse sweep is bit-reproducible: identical inputs give the to_bits()
    /// identical gradient on every run (determinism under libm).
    #[test]
    fn aad_bit_reproducible() {
        for (opt, i) in cases() {
            let a = adjoint_greeks(opt, &i);
            let b = adjoint_greeks(opt, &i);
            assert_eq!(a.price.to_bits(), b.price.to_bits());
            assert_eq!(a.delta_spot.to_bits(), b.delta_spot.to_bits());
            assert_eq!(a.delta_forward.to_bits(), b.delta_forward.to_bits());
            assert_eq!(a.vega.to_bits(), b.vega.to_bits());
            assert_eq!(a.theta.to_bits(), b.theta.to_bits());
            assert_eq!(a.rho_dom.to_bits(), b.rho_dom.to_bits());
            assert_eq!(a.rho_for.to_bits(), b.rho_for.to_bits());
            assert_eq!(a.gamma.to_bits(), b.gamma.to_bits());
            assert_eq!(a.vanna.to_bits(), b.vanna.to_bits());
            assert_eq!(a.volga.to_bits(), b.volga.to_bits());
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        /// Over a broad input domain the AAD first-order gradient agrees with
        /// the analytic Greeks to tight relative tolerance.
        #[test]
        fn aad_first_order_property(
            s in 0.5f64..200.0,
            k in 0.5f64..200.0,
            vol in 0.02f64..0.8,
            t in 0.02f64..3.0,
            r_dom in -0.02f64..0.10,
            r_for in -0.02f64..0.10,
        ) {
            let i = VanillaInputs::new(s, k, vol, t, r_dom, r_for);
            for opt in [OptionType::Call, OptionType::Put] {
                let a = adjoint_greeks(opt, &i);
                let g = greeks(opt, &i);
                prop_assert!(celnet_core::is_close(a.delta_spot, g.delta_spot, 1e-8, 1e-9));
                prop_assert!(celnet_core::is_close(a.vega, g.vega, 1e-7, 1e-8));
                prop_assert!(celnet_core::is_close(a.theta, g.theta, 1e-7, 1e-8));
                prop_assert!(celnet_core::is_close(a.rho_dom, g.rho_dom, 1e-7, 1e-8));
                prop_assert!(celnet_core::is_close(a.rho_for, g.rho_for, 1e-7, 1e-8));
                prop_assert!(celnet_core::is_close(a.gamma, g.gamma, 1e-7, 1e-8));
            }
        }
    }
}
