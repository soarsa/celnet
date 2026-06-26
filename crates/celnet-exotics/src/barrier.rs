//! Single- and double-barrier options under Garman-Kohlhagen, via the
//! reflection-principle (Reiner-Rubinstein) closed forms.
//!
//! # Single barriers
//!
//! A single-barrier option is a vanilla that either *activates* (knock-**in**) or
//! *extinguishes* (knock-**out**) when the continuously-monitored spot first
//! touches a barrier `H`, optionally paying a rebate `R` (at hit for a knocked-out
//! out-option, at expiry for a never-knocked-in in-option). The eight standard
//! flavours are the cross of {up, down} × {in, out} × {call, put}.
//!
//! The Reiner-Rubinstein construction writes each barrier price as a signed sum
//! of six building blocks `A…F` (themselves Black-Scholes-style `S·Φ − K·Φ`
//! terms), with the rebate handled by the touch closed forms in [`crate::touch`].
//! The defining property — **in/out parity** —
//!
//! ```text
//!   knock_in + knock_out = vanilla            (zero rebate)
//! ```
//!
//! holds **by construction**: knock-out is computed as `vanilla − knock_in`, so
//! parity is a *structural identity*, not an independent cross-check — it cannot
//! detect an error in the Reiner-Rubinstein block formulas or the regime
//! selection. The genuine, independent validation of the block selection is the
//! frozen QuantLib golden grid (`celnet-golden::single_barrier_grid`, all eight
//! flavours at `1e-9` relative); the in-crate suite additionally pins a small set
//! of hard-coded reference knock-in prices so a block error is caught without
//! leaving the crate.
//!
//! # Double barriers
//!
//! A double-barrier **knock-out** extinguishes if *either* of two barriers
//! `L < U` is touched. Its price is the method-of-images series over the vanilla
//! payoff truncated to the corridor — an alternating sum of reflected
//! Black-Scholes terms that converges geometrically (Kunitomo-Ikeda 1992).
//!
//! Provenance (doc-only): Reiner-Rubinstein (1991); the generalised-BSM
//! presentation of Haug (2007); Kunitomo-Ikeda (1992) for the double barrier.
//! Identifiers are purpose-named and vendor/research-neutral.

use celnet_core::math::{exp, norm_cdf, sqrt};
use celnet_types::OptionType;
use celnet_vanilla::price as vanilla_price;

use crate::inputs::ExoticInputs;
use crate::touch::RebateTiming;
use crate::{Lognormal, dlog, one_touch_price};

/// In/out knock style of a barrier option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BarrierStyle {
    /// Knock-in: the option activates only after the barrier is touched.
    KnockIn,
    /// Knock-out: the option extinguishes when the barrier is touched.
    KnockOut,
}

/// The full kind of a single barrier: direction (up/down), knock style, and the
/// underlying option type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BarrierKind {
    /// `true` if the barrier sits **above** spot (an *up* barrier), else *down*.
    pub up: bool,
    /// Knock-in or knock-out.
    pub style: BarrierStyle,
    /// Call or put underlying.
    pub option: OptionType,
}

/// A single-barrier option specification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SingleBarrier {
    /// What kind of barrier.
    pub kind: BarrierKind,
    /// Strike `K`.
    pub strike: f64,
    /// Barrier level `H`.
    pub barrier: f64,
    /// Rebate `R` paid if the option fails to pay out (at hit for knock-out, at
    /// expiry for knock-in). Set to `0.0` for a plain barrier.
    pub rebate: f64,
}

/// Internal building blocks of the Reiner-Rubinstein single-barrier formula.
///
/// `φ = +1` for a call, `−1` for a put; `η = +1` for a down barrier, `−1` for an
/// up barrier. With `μ = b/σ² − ½`, `vsqt = σ√T`:
/// ```text
///   x1 = ln(S/K)/vsqt + (1+μ)vsqt          x2 = ln(S/H)/vsqt + (1+μ)vsqt
///   y1 = ln(H²/(S K))/vsqt + (1+μ)vsqt     y2 = ln(H/S)/vsqt + (1+μ)vsqt
/// ```
struct RrBlocks {
    phi: f64,
    eta: f64,
    /// `(H/S)^{2(μ+1)}` power weight.
    pow_2mu2: f64,
    /// `(H/S)^{2μ}` power weight.
    pow_2mu: f64,
    s_disc: f64,
    k_disc: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl RrBlocks {
    fn new(i: &ExoticInputs, strike: f64, barrier: f64, option: OptionType, up: bool) -> Self {
        let l = Lognormal::from_inputs(i);
        let vsqt = l.sigma_sqrt_t();
        let mu = l.mu();
        let s = i.spot;
        let h = barrier;
        let k = strike;

        let x1 = dlog(s / k) / vsqt + (1.0 + mu) * vsqt;
        let x2 = dlog(s / h) / vsqt + (1.0 + mu) * vsqt;
        let y1 = dlog(h * h / (s * k)) / vsqt + (1.0 + mu) * vsqt;
        let y2 = dlog(h / s) / vsqt + (1.0 + mu) * vsqt;

        let hs = h / s;
        let pow_2mu2 = pow(hs, 2.0 * (mu + 1.0));
        let pow_2mu = pow(hs, 2.0 * mu);

        Self {
            phi: option.sign(),
            eta: if up { -1.0 } else { 1.0 },
            pow_2mu2,
            pow_2mu,
            s_disc: s * l.df_for(),
            k_disc: k * l.df_dom(),
            x1,
            x2,
            y1,
            y2,
        }
    }

    fn vsqt(i: &ExoticInputs) -> f64 {
        i.vol * sqrt(i.t)
    }

    /// A = φ·S·e^{−r_f T}·Φ(φ x1) − φ·K·e^{−r_d T}·Φ(φ(x1 − σ√T)).
    fn a(&self, vsqt: f64) -> f64 {
        self.phi * self.s_disc * norm_cdf(self.phi * self.x1)
            - self.phi * self.k_disc * norm_cdf(self.phi * (self.x1 - vsqt))
    }
    /// B uses x2 in place of x1 (barrier in place of strike).
    fn b(&self, vsqt: f64) -> f64 {
        self.phi * self.s_disc * norm_cdf(self.phi * self.x2)
            - self.phi * self.k_disc * norm_cdf(self.phi * (self.x2 - vsqt))
    }
    /// C = φ·S·e^{−r_f T}·(H/S)^{2(μ+1)}·Φ(η y1) − φ·K·e^{−r_d T}·(H/S)^{2μ}·Φ(η(y1−σ√T)).
    fn c(&self, vsqt: f64) -> f64 {
        self.phi * self.s_disc * self.pow_2mu2 * norm_cdf(self.eta * self.y1)
            - self.phi * self.k_disc * self.pow_2mu * norm_cdf(self.eta * (self.y1 - vsqt))
    }
    /// D uses y2 in place of y1.
    fn d(&self, vsqt: f64) -> f64 {
        self.phi * self.s_disc * self.pow_2mu2 * norm_cdf(self.eta * self.y2)
            - self.phi * self.k_disc * self.pow_2mu * norm_cdf(self.eta * (self.y2 - vsqt))
    }
}

/// Deterministic power for positive `base`.
#[inline]
fn pow(base: f64, exp_: f64) -> f64 {
    exp(exp_ * dlog(base))
}

/// `e^{ln_scale}·(Φ(a) − Φ(b))`, evaluated tail-stably for the image series.
///
/// The Ikeda-Kunitomo reflection terms multiply an exponential weight that can
/// reach `e^{±500}` when the carry-to-variance ratio `|b|/σ²` is large by a
/// difference of normal CDFs whose true magnitude offsets it. Evaluated
/// literally, both `Φ` saturate to `1.0` in the upper tail, so the difference
/// collapses to `0` or to a ±1-ulp rounding residue that the huge weight
/// amplifies into a completely wrong price (the `double_ko_bounds_property`
/// regression at `b/σ² ≈ −45.9` summed weights of `e^{462}` against ulp noise
/// and blew through the vanilla upper bound). Two repairs:
///
/// * **Tail-stable difference**: for `a, b ≥ 0`, `Φ(a) − Φ(b) = Φ(−b) − Φ(−a)`
///   — both operands move to the erfc-based *lower* tail of
///   [`celnet_core::math::norm_cdf`], which keeps full relative precision down
///   to the underflow floor instead of saturating at 1.
/// * **Log-space product**: the weight and the difference combine as
///   `±exp(ln_scale + ln|Δ|)`, so a huge weight meets a tiny difference in the
///   exponent — bounded for the convergent series — instead of overflowing or
///   amplifying rounding noise.
///
/// A difference that is exactly `0.0` (both CDFs identical in f64: the true
/// weighted term is far below any representable magnitude) returns `0`; a
/// non-finite combination (unreachable inside the series' convergence domain)
/// is clamped to `0`, matching [`crate::touch`]'s `pow_cdf` guard.
#[inline]
fn scaled_cdf_diff(ln_scale: f64, a: f64, b: f64) -> f64 {
    let diff = if a >= 0.0 && b >= 0.0 {
        norm_cdf(-b) - norm_cdf(-a)
    } else {
        norm_cdf(a) - norm_cdf(b)
    };
    if diff == 0.0 {
        return 0.0;
    }
    let v = diff.signum() * exp(ln_scale + dlog(diff.abs()));
    if v.is_finite() { v } else { 0.0 }
}

/// Price of a single-barrier option (per unit base notional, domestic premium),
/// including rebate.
///
/// The knock-out / knock-in values without rebate are the Reiner-Rubinstein
/// signed combinations of the `A…D` blocks selected by the strike-vs-barrier
/// regime; the rebate adds the appropriate touch value (a knocked-out option pays
/// its rebate **at hit** via a one-touch; a knock-in pays at expiry if never
/// knocked in, via a no-touch on the same barrier).
#[must_use]
pub fn single_barrier_price(i: &ExoticInputs, spec: SingleBarrier) -> f64 {
    let SingleBarrier {
        kind,
        strike,
        barrier,
        rebate,
    } = spec;

    let bare = single_barrier_no_rebate(i, kind, strike, barrier);

    if rebate == 0.0 {
        return bare;
    }

    // Rebate leg: an out-option pays the rebate when it knocks out (one-touch,
    // at hit); an in-option pays the rebate at expiry if it never knocks in
    // (no-touch on the barrier).
    let reb = match kind.style {
        BarrierStyle::KnockOut => one_touch_price(i, barrier, rebate, RebateTiming::AtHit),
        BarrierStyle::KnockIn => crate::touch::no_touch_price(i, barrier, rebate),
    };
    bare + reb
}

/// The rebate-free single-barrier value.
fn single_barrier_no_rebate(i: &ExoticInputs, kind: BarrierKind, strike: f64, barrier: f64) -> f64 {
    // The Reiner-Rubinstein in/out parity leg is the plain vanilla at `strike`.
    // This analytic closed form is the Garman-Kohlhagen (FX) image construction;
    // a non-FX barrier prices on the PDE/MC engines, so lowering to the FX vanilla
    // here typed-rejects a non-FX carry rather than silently mis-pricing it.
    let vanilla = vanilla_price(
        kind.option,
        &i.as_fx_vanilla(strike)
            .expect("analytic single-barrier is the FX (Garman-Kohlhagen) closed form"),
    );

    // If the barrier is already breached the knock is resolved immediately.
    let breached = if kind.up {
        i.spot >= barrier
    } else {
        i.spot <= barrier
    };
    if breached {
        return match kind.style {
            // Knock-out already dead (rebate handled separately) ⇒ 0 intrinsic option value.
            BarrierStyle::KnockOut => 0.0,
            // Knock-in already active ⇒ it is the plain vanilla.
            BarrierStyle::KnockIn => vanilla,
        };
    }

    let blk = RrBlocks::new(i, strike, barrier, kind.option, kind.up);
    let vsqt = RrBlocks::vsqt(i);
    let (a, b, c, d) = (blk.a(vsqt), blk.b(vsqt), blk.c(vsqt), blk.d(vsqt));

    // Reiner-Rubinstein selection by {up/down, call/put, K vs H}. We compute the
    // KNOCK-IN value directly from the blocks, then derive knock-out by parity
    // (KO = vanilla − KI), which guarantees exact in/out parity.
    let call = matches!(kind.option, OptionType::Call);
    let k_ge_h = strike >= barrier;

    let knock_in = if kind.up {
        // Up-and-in.
        if call {
            // Up-and-in call.
            if k_ge_h { a } else { b - c + d }
        } else {
            // Up-and-in put.
            if k_ge_h { a - b + d } else { c }
        }
    } else {
        // Down-and-in.
        if call {
            // Down-and-in call.
            if k_ge_h { c } else { a - b + d }
        } else {
            // Down-and-in put.
            if k_ge_h { b - c + d } else { a }
        }
    };

    match kind.style {
        BarrierStyle::KnockIn => knock_in,
        BarrierStyle::KnockOut => vanilla - knock_in,
    }
}

/// A double-barrier knock-out option: a vanilla that extinguishes if *either*
/// barrier `lower < upper` is touched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoubleBarrierKnockOut {
    /// Underlying option type.
    pub option: OptionType,
    /// Strike `K`.
    pub strike: f64,
    /// Lower barrier `L`.
    pub lower: f64,
    /// Upper barrier `U`.
    pub upper: f64,
}

impl DoubleBarrierKnockOut {
    /// Construct a double-barrier knock-out.
    ///
    /// # Panics
    ///
    /// Panics unless `0 < lower < upper`.
    #[must_use]
    pub fn new(option: OptionType, strike: f64, lower: f64, upper: f64) -> Self {
        assert!(
            lower > 0.0 && lower < upper,
            "double-barrier corridor must satisfy 0 < lower < upper: L={lower}, U={upper}"
        );
        Self {
            option,
            strike,
            lower,
            upper,
        }
    }
}

/// Image-series term count for the double-barrier knock-out (geometric decay; the
/// truncation is verified converged in the tests).
const DKO_TERMS: i32 = 10;

/// Price of a [`DoubleBarrierKnockOut`] via the Ikeda-Kunitomo method-of-images
/// series (per unit base notional, domestic premium). No rebate (a double-barrier
/// rebate would be a double-touch leg; first-generation KO is rebate-free here —
/// callers needing a rebate add [`crate::double_touch_price`] explicitly).
///
/// The series sums reflected Black-Scholes-style asset/cash legs. For a knock-out
/// **call** the payoff is integrated over `(F₁, F₂) = (K, U)`; for a **put** over
/// `(L, K)`. With `μ₁ = 2(μ+1) = 2b/σ² + 1` and `vsqt = σ√T`, the `n`-th
/// reflection contributes (asset leg shown; the cash leg replaces `μ₁` by
/// `μ₁ − 2 = 2μ` and shifts each argument by `−σ√T`):
/// ```text
///   (U/L)^{n·μ₁}·[ Φ(φ·dᵃ₁) − Φ(φ·dᵃ₂) ]
///   − (Lⁿ⁺¹/(Uⁿ·S))^{μ₁}·[ Φ(φ·dᵇ₁) − Φ(φ·dᵇ₂) ]
/// ```
/// where the four `d`-arguments carry the corridor-reflected log-moneyness levels
/// at `F₁`, `F₂`. The series converges geometrically in `|n|` (each reflection
/// scales the corridor width in log-space), so the fixed term count reaches
/// machine precision for realistic corridors. Provenance (doc-only): Ikeda-Kunitomo
/// (1992); the presentation in Haug (2007).
#[must_use]
pub fn double_knock_out_price(i: &ExoticInputs, spec: DoubleBarrierKnockOut) -> f64 {
    let DoubleBarrierKnockOut {
        option,
        strike,
        lower,
        upper,
    } = spec;

    // Already outside the corridor ⇒ knocked out ⇒ worthless.
    if i.spot <= lower || i.spot >= upper {
        return 0.0;
    }

    let l = Lognormal::from_inputs(i);
    let vsqt = l.sigma_sqrt_t();
    let mu = l.mu();
    let mu1 = 2.0 * (mu + 1.0); // = 2b/σ² + 1
    let s = i.spot;
    let k = strike;
    let big_l = lower;
    let big_u = upper;
    let df_for = l.df_for();
    let df_dom = l.df_dom();
    let phi = option.sign();

    // Payoff integration band on the corridor: call ⇒ (K, U); put ⇒ (L, K).
    let (f_lo, f_hi) = match option {
        OptionType::Call => (k.max(big_l), big_u),
        OptionType::Put => (big_l, k.min(big_u)),
    };
    // Empty band (e.g. a call struck above the upper barrier) ⇒ zero.
    if f_lo >= f_hi {
        return 0.0;
    }

    let drift = (1.0 + mu) * vsqt;
    let ul = big_u / big_l;
    let ln_ul = dlog(ul);
    let ln_s = dlog(s);
    let ln_l = dlog(big_l);
    let ln_u = dlog(big_u);

    let mut sum = 0.0;
    for n in -DKO_TERMS..=DKO_TERMS {
        let nf = f64::from(n);
        // Direct image spot S·(U/L)^{2n}: ln(S'/level) = lnS + 2n·ln(U/L) − ln(level).
        let direct_log = ln_s + 2.0 * nf * ln_ul;
        let dd_lo = (direct_log - dlog(f_lo)) / vsqt + drift;
        let dd_hi = (direct_log - dlog(f_hi)) / vsqt + drift;
        // ln of the direct reflection weight (U/L)^{n·μ₁}.
        let ln_pow1 = nf * mu1 * ln_ul;

        // Lower-wall mirror image spot S' = L^{2n+2}/(U^{2n}·S):
        //   ln(S'/level) = (2n+2)lnL − 2n·lnU − lnS − ln(level).
        let img_log = (2.0 * nf + 2.0) * ln_l - 2.0 * nf * ln_u - ln_s;
        let dm_lo = (img_log - dlog(f_lo)) / vsqt + drift;
        let dm_hi = (img_log - dlog(f_hi)) / vsqt + drift;
        // ln of the mirror weight base L^{n+1}/(Uⁿ·S).
        let ln_mirror = (nf + 1.0) * ln_l - nf * ln_u - ln_s;

        // Each weight×CDF-difference product goes through the log-space
        // tail-stable form — see `scaled_cdf_diff` for why the literal
        // `pow(..)·(Φ−Φ)` evaluation breaks down at large |b|/σ².
        let asset = s
            * df_for
            * (scaled_cdf_diff(ln_pow1, phi * dd_lo, phi * dd_hi)
                - scaled_cdf_diff(mu1 * ln_mirror, phi * dm_lo, phi * dm_hi));

        // Cash leg: power exponent μ₁ − 2 = 2μ, arguments shifted by −σ√T.
        let cash = k
            * df_dom
            * (scaled_cdf_diff(
                nf * (mu1 - 2.0) * ln_ul,
                phi * (dd_lo - vsqt),
                phi * (dd_hi - vsqt),
            ) - scaled_cdf_diff(
                (mu1 - 2.0) * ln_mirror,
                phi * (dm_lo - vsqt),
                phi * (dm_hi - vsqt),
            ));

        sum += asset - cash;
    }

    // `phi` already orients every `norm_cdf(phi · d)` argument inside the loop, so
    // `sum` is the put/call value directly; the payoff floor is the only guard.
    // (A spurious outer `phi·sum` would negate the put, clamping it to zero — the
    // bug the `double_barrier_grid` QuantLib golden gate now catches.)
    sum.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::{assert_close, is_close};
    use celnet_types::VanillaInputs;

    fn base() -> ExoticInputs {
        // S=100, K varies, σ=20%, 1Y, r_d=5%, r_f=2%.
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02).into()
    }

    /// Lower the agnostic test fixture back to the FX `VanillaInputs` (with a
    /// substituted strike) for the parity/oracle vanilla legs.
    fn vi(i: &ExoticInputs, strike: f64) -> VanillaInputs {
        i.as_fx_vanilla(strike).expect("FX test fixture")
    }

    /// Hard-coded knock-in reference prices from QuantLib 1.42.1's
    /// `AnalyticBarrierEngine` (Garman-Kohlhagen, `S=100, σ=20%, T=1y, r_d=5%,
    /// r_f=2%`). Unlike the in/out parity identity — which holds *by construction*
    /// because knock-out is `vanilla − knock_in` and therefore cannot validate the
    /// Reiner-Rubinstein block selection — these pin the actual knock-in block
    /// formulas to an external oracle *inside the crate*, so a mis-selected block
    /// is caught here without the `celnet-golden` grid (audit `barrier.rs`).
    #[test]
    fn knock_in_matches_quantlib_reference() {
        let i = base();
        // (up, option, K, H, QuantLib KI price).
        let cases = [
            (false, OptionType::Call, 100.0, 80.0, 0.093_699_071_656),
            (true, OptionType::Call, 100.0, 120.0, 8.094_513_367_157),
            (false, OptionType::Put, 100.0, 80.0, 4.597_402_864_380),
            (true, OptionType::Put, 100.0, 120.0, 0.230_613_308_713),
        ];
        for &(up, option, k, h, reference) in &cases {
            let ki = single_barrier_price(
                &i,
                SingleBarrier {
                    kind: BarrierKind {
                        up,
                        style: BarrierStyle::KnockIn,
                        option,
                    },
                    strike: k,
                    barrier: h,
                    rebate: 0.0,
                },
            );
            assert_close!(ki, reference, 1e-9, 1e-11);
        }
    }

    /// In/out parity for every single-barrier flavour: KI + KO = vanilla (zero
    /// rebate), across up/down × call/put × K-above/below-barrier regimes.
    ///
    /// NOTE: this is a **structural** identity (knock-out is computed as
    /// `vanilla − knock_in`), so it cannot catch a wrong Reiner-Rubinstein block —
    /// the independent block validation is [`knock_in_matches_quantlib_reference`]
    /// (in-crate) and the `celnet-golden` QuantLib grid (cross-crate). It is kept
    /// as a cheap invariant guard, not as the headline correctness check.
    #[test]
    fn in_out_parity_all_flavours() {
        let i = base();
        let cases = [
            // (up, K, H) with H on the correct side of spot (100).
            (true, 100.0, 120.0),
            (true, 130.0, 120.0),
            (true, 110.0, 115.0),
            (false, 100.0, 80.0),
            (false, 70.0, 80.0),
            (false, 95.0, 85.0),
        ];
        for &(up, k, h) in &cases {
            for option in [OptionType::Call, OptionType::Put] {
                let ki = single_barrier_price(
                    &i,
                    SingleBarrier {
                        kind: BarrierKind {
                            up,
                            style: BarrierStyle::KnockIn,
                            option,
                        },
                        strike: k,
                        barrier: h,
                        rebate: 0.0,
                    },
                );
                let ko = single_barrier_price(
                    &i,
                    SingleBarrier {
                        kind: BarrierKind {
                            up,
                            style: BarrierStyle::KnockOut,
                            option,
                        },
                        strike: k,
                        barrier: h,
                        rebate: 0.0,
                    },
                );
                let vanilla = vanilla_price(option, &vi(&i, k));
                assert_close!(ki + ko, vanilla, 1e-10, 1e-10);
            }
        }
    }

    /// Barrier values are non-negative and never exceed the vanilla (zero
    /// rebate): the barrier feature can only remove value.
    #[test]
    fn barrier_bounded_by_vanilla() {
        let i = base();
        for &(up, k, h) in &[(true, 100.0, 125.0), (false, 100.0, 80.0)] {
            for option in [OptionType::Call, OptionType::Put] {
                for style in [BarrierStyle::KnockIn, BarrierStyle::KnockOut] {
                    let v = single_barrier_price(
                        &i,
                        SingleBarrier {
                            kind: BarrierKind { up, style, option },
                            strike: k,
                            barrier: h,
                            rebate: 0.0,
                        },
                    );
                    let vanilla = vanilla_price(option, &vi(&i, k));
                    assert!(
                        v >= -1e-10 && v <= vanilla + 1e-9,
                        "barrier {v} out of [0,{vanilla}] for up={up},{option:?},{style:?}"
                    );
                }
            }
        }
    }

    /// A knocked-out option that has already breached its barrier is worthless
    /// (bare), and a knocked-in one is the live vanilla.
    #[test]
    fn already_breached_resolves() {
        let i = base();
        // Up barrier at 95 < spot 100 ⇒ already breached.
        let ko = single_barrier_price(
            &i,
            SingleBarrier {
                kind: BarrierKind {
                    up: true,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike: 100.0,
                barrier: 95.0,
                rebate: 0.0,
            },
        );
        assert_close!(ko, 0.0, 1e-12, 1e-12);
        let ki = single_barrier_price(
            &i,
            SingleBarrier {
                kind: BarrierKind {
                    up: true,
                    style: BarrierStyle::KnockIn,
                    option: OptionType::Call,
                },
                strike: 100.0,
                barrier: 95.0,
                rebate: 0.0,
            },
        );
        assert_close!(
            ki,
            vanilla_price(OptionType::Call, &vi(&i, i.strike)),
            1e-12,
            1e-12
        );
    }

    /// A rebate strictly increases the value of a knock-out (it pays something on
    /// the knock-out event that the bare option does not).
    #[test]
    fn rebate_adds_value() {
        let i = base();
        let bare = single_barrier_price(
            &i,
            SingleBarrier {
                kind: BarrierKind {
                    up: true,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike: 100.0,
                barrier: 125.0,
                rebate: 0.0,
            },
        );
        let with_reb = single_barrier_price(
            &i,
            SingleBarrier {
                kind: BarrierKind {
                    up: true,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike: 100.0,
                barrier: 125.0,
                rebate: 5.0,
            },
        );
        assert!(
            with_reb > bare,
            "rebate should add value: {with_reb} > {bare}"
        );
    }

    /// Hard-coded double-knock-out reference prices from QuantLib 1.42.1's
    /// `AnalyticDoubleBarrierEngine` (`S=100, σ=20%, T=1y, r_d=5%, r_f=2%`,
    /// corridor `(85,120)`), for **both** call and put across strikes. The put
    /// rows are the regression for the Ikeda-Kunitomo sign bug an earlier version
    /// carried (a spurious outer `φ·sum` negated the put leg, clamping it to zero);
    /// the in-crate Monte-Carlo cross-check exercised only the call, so the bug was
    /// invisible until this external reference and the `celnet-golden`
    /// double-barrier grid were added.
    #[test]
    fn double_ko_matches_quantlib_reference() {
        let i = base();
        let cases = [
            (OptionType::Call, 90.0, 2.646_056_291_260),
            (OptionType::Call, 100.0, 0.897_576_951_965),
            (OptionType::Call, 110.0, 0.117_517_109_455),
            (OptionType::Put, 90.0, 0.025_103_294_543),
            (OptionType::Put, 100.0, 0.573_400_152_430),
            (OptionType::Put, 110.0, 2.090_116_507_102),
        ];
        for &(option, k, reference) in &cases {
            let dko =
                double_knock_out_price(&i, DoubleBarrierKnockOut::new(option, k, 85.0, 120.0));
            assert!(
                is_close(dko, reference, 1e-6, 1e-8),
                "DKO {option:?} K={k}: celnet {dko} vs QuantLib {reference}"
            );
        }
    }

    /// A double-barrier knock-out is non-negative, no greater than the vanilla,
    /// and worthless once the corridor is breached.
    #[test]
    fn double_ko_bounds() {
        let i = base();
        let spec = DoubleBarrierKnockOut::new(OptionType::Call, 100.0, 85.0, 120.0);
        let v = double_knock_out_price(&i, spec);
        let vanilla = vanilla_price(OptionType::Call, &vi(&i, i.strike));
        assert!(
            v >= -1e-10 && v <= vanilla + 1e-9,
            "DKO {v} in [0,{vanilla}]"
        );

        // Breached corridor ⇒ 0.
        let outside = ExoticInputs {
            spot: 130.0,
            ..i.clone()
        };
        assert_close!(double_knock_out_price(&outside, spec), 0.0, 1e-12, 1e-12);
    }

    /// A wide double-barrier corridor approaches the vanilla (the barriers rarely
    /// bind), while a tight corridor is worth much less — monotone in width.
    #[test]
    fn double_ko_monotone_in_width() {
        let i = base();
        let tight = double_knock_out_price(
            &i,
            DoubleBarrierKnockOut::new(OptionType::Call, 100.0, 92.0, 109.0),
        );
        let wide = double_knock_out_price(
            &i,
            DoubleBarrierKnockOut::new(OptionType::Call, 100.0, 70.0, 145.0),
        );
        assert!(wide > tight, "wide DKO {wide} should exceed tight {tight}");
        // And the wide KO is below the unconstrained vanilla.
        assert!(wide <= vanilla_price(OptionType::Call, &vi(&i, i.strike)) + 1e-9);
    }

    /// Sandwich: a double knock-out cannot exceed either single knock-out built
    /// from the same two barriers — adding a second wall only removes value.
    #[test]
    fn double_ko_below_each_single_ko() {
        let i = base();
        for option in [OptionType::Call, OptionType::Put] {
            let (k, lo, up) = (100.0, 85.0, 120.0);
            let dko = double_knock_out_price(&i, DoubleBarrierKnockOut::new(option, k, lo, up));
            let down_out = single_barrier_price(
                &i,
                SingleBarrier {
                    kind: BarrierKind {
                        up: false,
                        style: BarrierStyle::KnockOut,
                        option,
                    },
                    strike: k,
                    barrier: lo,
                    rebate: 0.0,
                },
            );
            let up_out = single_barrier_price(
                &i,
                SingleBarrier {
                    kind: BarrierKind {
                        up: true,
                        style: BarrierStyle::KnockOut,
                        option,
                    },
                    strike: k,
                    barrier: up,
                    rebate: 0.0,
                },
            );
            assert!(
                dko <= down_out + 1e-9 && dko <= up_out + 1e-9,
                "{option:?}: DKO {dko} must be ≤ single KOs ({down_out}, {up_out})"
            );
            assert!(dko >= -1e-10);
        }
    }

    /// Regression (proptest `cc d16c25ad…`, persisted in
    /// `proptest-regressions/barrier.txt`): an extreme carry-to-variance ratio
    /// (`b/σ² ≈ −45.9`: 7% negative carry against 3.9% vol) drove the raw
    /// Ikeda-Kunitomo reflection weights to `e^{±463}` while their CDF
    /// differences saturated to 1.0-ulp, so the series summed rounding noise
    /// amplified by ~1e200 and the "price" blew far through the vanilla bound.
    /// The log-space tail-stable evaluation (`scaled_cdf_diff`) keeps every
    /// term's magnitude in the exponent. The corridor here is wide (the lower
    /// wall sits ≈6.2σ below spot, the upper ≈5.0σ above, drift ≈−2.1σ), so
    /// the true knock-out probability strips only ~2e-6 of value: the DKO must
    /// sit fractionally BELOW the vanilla, not above it — pinned both ways.
    #[test]
    fn double_ko_extreme_carry_regression() {
        let i: ExoticInputs = VanillaInputs::new(
            0.25,
            0.25,
            0.039_156_509_308_435_41,
            1.351_005_303_060_065_5,
            0.030_973_909_748_948_616,
            0.101_257_786_905_037_64,
        )
        .into();
        let lower = 0.25 * (1.0 - 0.246_647_360_229_049_7);
        let upper = 0.25 * (1.0 + 0.254_740_468_358_782_83);
        let spec = DoubleBarrierKnockOut::new(OptionType::Put, 0.25, lower, upper);
        let v = double_knock_out_price(&i, spec);
        let vanilla = vanilla_price(OptionType::Put, &vi(&i, 0.25));
        // Two-sided magnitude pin: barriers ~4σ+ effective ⇒ KO strips < 1e-4
        // of value, and a knock-out can never exceed its vanilla.
        assert!(
            v <= vanilla && v >= vanilla - 1e-4,
            "DKO {v} must sit fractionally below vanilla {vanilla}"
        );
        // Sandwich vs each single-KO leg (both closed forms remain stable here).
        for (up, barrier) in [(false, lower), (true, upper)] {
            let single = single_barrier_price(
                &i,
                SingleBarrier {
                    kind: BarrierKind {
                        up,
                        style: BarrierStyle::KnockOut,
                        option: OptionType::Put,
                    },
                    strike: 0.25,
                    barrier,
                    rebate: 0.0,
                },
            );
            assert!(
                v <= single + 1e-12,
                "DKO {v} must be ≤ the up={up} single KO {single}"
            );
        }
    }

    /// Reference validation of the closed-form double knock-out against a
    /// deterministic, Brownian-bridge-corrected discrete Monte-Carlo estimate
    /// (counter-based Philox-style stream, finely time-stepped). The closed form
    /// must agree with the converged path estimate to MC tolerance.
    #[test]
    fn double_ko_matches_monte_carlo() {
        let i = base();
        let spec = DoubleBarrierKnockOut::new(OptionType::Call, 100.0, 85.0, 120.0);
        let analytic = double_knock_out_price(&i, spec);
        let mc = mc_double_ko(&i, spec, 240, 200_000, 0x51ED_C0DE_1234_5678);
        // Brownian-bridge corrected MC of a continuously-monitored barrier carries
        // O(1/√N) noise; the corridor option here is ≈ a few price units, so a few
        // ×1e-2 absolute tolerance is appropriate for 2·10⁵ paths.
        assert!(
            (analytic - mc).abs() < 5e-2,
            "analytic DKO {analytic} vs MC {mc} (|diff|={})",
            (analytic - mc).abs()
        );
    }

    /// A self-contained, deterministic Monte-Carlo estimator for the continuously
    /// monitored double knock-out, used only as an independent reference oracle in
    /// the test above. Uses a counter-based splitmix64 → normal stream (seeded,
    /// reproducible). Continuous monitoring is recovered from the discrete grid by
    /// the Broadie-Glasserman-Kou barrier shift `H_eff = H·exp(±β·σ√dt)`,
    /// `β ≈ 0.5826`, which shrinks the corridor to cancel the discrete-monitoring
    /// survivorship bias to `o(1/√steps)`.
    fn mc_double_ko(
        i: &ExoticInputs,
        spec: DoubleBarrierKnockOut,
        steps: usize,
        paths: usize,
        seed: u64,
    ) -> f64 {
        use celnet_core::math::{exp, ln, sqrt};

        let dt = i.t / steps as f64;
        let drift = (i.carry_rate() - 0.5 * i.vol * i.vol) * dt;
        let vol_step = i.vol * sqrt(dt);
        let df = i.discount_df();
        // Broadie-Glasserman-Kou continuity correction: shrink the corridor.
        const BETA: f64 = 0.582_597_403_404_879_2; // −ζ(½)/√(2π)
        let shift = BETA * vol_step;
        let ln_l = ln(spec.lower) + shift; // raise lower wall
        let ln_u = ln(spec.upper) - shift; // lower upper wall

        // Counter-based normal generator (splitmix64 → Box-Muller); deterministic.
        let mut counter = seed;
        let mut next_u01 = || {
            counter = counter.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = counter;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            ((z >> 11) as f64 + 0.5) * (1.0 / (1u64 << 53) as f64)
        };
        let mut next_normal = || {
            let u1: f64 = next_u01();
            let u2: f64 = next_u01();
            sqrt(-2.0 * ln(u1)) * (core::f64::consts::TAU * u2).cos()
        };

        let mut acc = 0.0;
        for _ in 0..paths {
            let mut ln_s = ln(i.spot);
            let mut alive = true;
            for _ in 0..steps {
                ln_s += drift + vol_step * next_normal();
                if ln_s <= ln_l || ln_s >= ln_u {
                    alive = false;
                    break;
                }
            }
            if alive {
                let payoff = (exp(ln_s) - spec.strike).max(0.0);
                acc += payoff;
            }
        }
        df * acc / paths as f64
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use celnet_testkit::{arb_inputs, arb_vol};
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// In/out parity (`KI + KO = vanilla`, zero rebate) holds for every random
        /// market state and every single-barrier flavour, with the barrier placed
        /// on the correct side of spot at a random multiplicative offset.
        #[test]
        fn in_out_parity_property(
            i in arb_inputs(),
            up in any::<bool>(),
            call in any::<bool>(),
            offset in 0.05f64..0.60,
            strike_mult in 0.6f64..1.6,
        ) {
            let i: ExoticInputs = i.into();
            // Barrier strictly on the chosen side of spot.
            let barrier = if up { i.spot * (1.0 + offset) } else { i.spot * (1.0 - offset) };
            let strike = i.spot * strike_mult;
            let option = if call { OptionType::Call } else { OptionType::Put };

            let ki = single_barrier_price(&i, SingleBarrier {
                kind: BarrierKind { up, style: BarrierStyle::KnockIn, option },
                strike, barrier, rebate: 0.0,
            });
            let ko = single_barrier_price(&i, SingleBarrier {
                kind: BarrierKind { up, style: BarrierStyle::KnockOut, option },
                strike, barrier, rebate: 0.0,
            });
            let vanilla = vanilla_price(option, &i.as_fx_vanilla(strike).unwrap());
            prop_assert!(celnet_core::is_close(ki + ko, vanilla, 1e-7, 1e-7));
            // Each leg is a non-negative fraction of the vanilla.
            prop_assert!(ki >= -1e-7 && ko >= -1e-7);
            prop_assert!(ki <= vanilla + 1e-6 && ko <= vanilla + 1e-6);
        }

        /// A double knock-out is non-negative and bounded by the unconstrained
        /// vanilla for any random market and corridor around spot.
        #[test]
        fn double_ko_bounds_property(
            i in arb_inputs(),
            call in any::<bool>(),
            lo_off in 0.05f64..0.45,
            hi_off in 0.05f64..0.45,
            _v in arb_vol(),
        ) {
            let i: ExoticInputs = i.into();
            let option = if call { OptionType::Call } else { OptionType::Put };
            let lower = i.spot * (1.0 - lo_off);
            let upper = i.spot * (1.0 + hi_off);
            let spec = DoubleBarrierKnockOut::new(option, i.spot, lower, upper);
            let v = double_knock_out_price(&i, spec);
            let vanilla = vanilla_price(option, &i.as_fx_vanilla(i.spot).unwrap());
            prop_assert!(v >= -1e-7 && v <= vanilla + 1e-6);
        }
    }
}
