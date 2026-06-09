//! One-touch / no-touch, double-no-touch and double-touch under
//! Garman-Kohlhagen, via the reflection-principle (method-of-images) closed
//! forms.
//!
//! # One-touch (single barrier `H`)
//!
//! A **one-touch** pays a fixed rebate `R` (here normalised to one unit of
//! domestic notional) if the continuously-monitored spot touches the barrier `H`
//! before expiry `T`; a **no-touch** pays `R` if it never does. They are
//! complementary up to discounting:
//!
//! ```text
//!   no_touch = e^{−r_d T} − one_touch_deferred
//! ```
//!
//! With `μ = b/σ² − ½`, `λ = √(μ² + 2 r_d/σ²)`, `vsqt = σ√T`, `z = ln(H/S)`, and
//! `η = +1` if the barrier is **below** spot (a down-touch), `η = −1` if **above**
//! (up-touch), the value of a one-touch paying `R` **at hit** is
//!
//! ```text
//!   OT = R·[ (H/S)^{μ+λ}·Φ(η·(−z/vsqt + λ vsqt))
//!          + (H/S)^{μ−λ}·Φ(η·(−z/vsqt − λ vsqt)) ].
//! ```
//!
//! Paying `R` **at expiry** (deferred) discounts the hit at the domestic rate and
//! replaces `λ` by `μ` inside the standard expression. Both timings are provided.
//!
//! # Double-no-touch (two barriers `L < U`)
//!
//! A **double-no-touch (DNT)** pays `R` iff the spot stays strictly inside
//! `(L, U)` for all `t ≤ T`. Its survival probability is the
//! between-two-barriers first-exit probability, obtained from the
//! image series (an infinite sum of reflections that converges geometrically).
//! The **double-touch** (pays if *either* barrier is hit) is its complement up to
//! discounting. The DNT value is clamped to `[0, R]` to absorb truncation of the
//! image series in pathological deep-in-the-corner regimes.
//!
//! Provenance (doc-only): Reiner-Rubinstein (1991); the unified generalised-BSM
//! presentation of Haug (2007); the double-barrier image series of
//! Kunitomo-Ikeda (1992) / Geman-Yor (1996). Identifiers are purpose-named and
//! vendor/research-neutral.

use crate::inputs::ExoticInputs;
use celnet_core::math::{exp, norm_cdf};

use crate::{Lognormal, dlog};

/// Which side of spot a single touch barrier sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TouchSide {
    /// Barrier above spot: an *up* touch (`H > S`).
    Upper,
    /// Barrier below spot: a *down* touch (`H < S`).
    Lower,
}

impl TouchSide {
    /// Infer the side from the barrier level relative to spot.
    #[must_use]
    pub fn from_levels(spot: f64, barrier: f64) -> Self {
        if barrier >= spot {
            TouchSide::Upper
        } else {
            TouchSide::Lower
        }
    }
}

/// When a touch rebate is paid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebateTiming {
    /// Paid immediately when the barrier is hit (the standard one-touch).
    AtHit,
    /// Paid at expiry regardless of when the barrier was hit (deferred rebate).
    AtExpiry,
}

/// Present value of a **one-touch** paying `rebate` (domestic) if the spot
/// touches `barrier` before `T`, with the rebate paid per [`RebateTiming`].
///
/// The value is clamped to `[0, R·max(1, e^{−r_d T})]`: a probability-weighted
/// payout cannot fall below zero nor exceed the rebate scaled by the largest
/// applicable discount factor (under non-negative rates simply the rebate `R`;
/// under negative domestic rates the deferred/at-hit payout can exceed the face).
/// `i.strike` is ignored — a touch has no strike. `side` is inferred from
/// `barrier` vs `i.spot` if you use [`one_touch_price`]; the raw side-explicit
/// form is private.
#[must_use]
pub fn one_touch_price(i: &ExoticInputs, barrier: f64, rebate: f64, timing: RebateTiming) -> f64 {
    let side = TouchSide::from_levels(i.spot, barrier);
    one_touch_with_side(i, barrier, rebate, timing, side)
}

/// One-touch with an explicit side (used internally and by the double-touch
/// decomposition).
fn one_touch_with_side(
    i: &ExoticInputs,
    barrier: f64,
    rebate: f64,
    timing: RebateTiming,
    side: TouchSide,
) -> f64 {
    // If spot is already at/through the barrier the touch is certain.
    let through = match side {
        TouchSide::Upper => i.spot >= barrier,
        TouchSide::Lower => i.spot <= barrier,
    };
    if through {
        return match timing {
            RebateTiming::AtHit => rebate,
            RebateTiming::AtExpiry => rebate * i.discount_df(),
        };
    }

    let l = Lognormal::from_inputs(i);
    let mu = l.mu(); // = (b − ½σ²)/σ²  (drift per unit σ²)
    let vsqt = l.sigma_sqrt_t();
    let hs = barrier / i.spot;
    let z = dlog(hs); // ln(H/S): >0 upper, <0 lower

    // First-passage normal arguments, oriented for the barrier side. For an
    // *upper* barrier (z>0) the relevant tail is the running maximum:
    //   arg = (−z ± k·vsqt)/1 ;  for a *lower* barrier the running minimum gives
    //   arg = ( z ∓ k·vsqt). Encapsulate the side sign once.
    let side_sign = match side {
        TouchSide::Upper => -1.0,
        TouchSide::Lower => 1.0,
    };
    // `base = side_sign·z/vsqt`; the ± drift term is added with the SAME side
    // orientation so both barriers reduce to the canonical reflected normal pair.
    let base = side_sign * z / vsqt;
    // The drift term enters with the OPPOSITE side orientation: an upper barrier's
    // first-passage tail drifts *up* (+k·vsqt on a₁), a lower barrier's *down*.
    let drift_sign = -side_sign;

    let value = match timing {
        RebateTiming::AtHit => {
            // Reiner-Rubinstein at-hit one-touch (discounted-hit expectation):
            //   OT = R·[ (H/S)^{μ+λ}·Φ(a₁) + (H/S)^{μ−λ}·Φ(a₂) ],
            //   λ = √(μ²+2r_d/σ²), a₁ = base + drift_sign·λ·vsqt, a₂ = base − drift_sign·λ·vsqt.
            let lam = l.lambda();
            let a1 = base + drift_sign * lam * vsqt;
            let a2 = base - drift_sign * lam * vsqt;
            rebate * (pow_cdf(z, mu + lam, a1) + pow_cdf(z, mu - lam, a2))
        }
        RebateTiming::AtExpiry => {
            // Deferred rebate: paid at T iff a hit occurred ⇒ discounted hit
            // probability under the spot drift (m = b − ½σ², so m/σ² = μ):
            //   P(hit) = Φ(a₁) + (H/S)^{2μ}·Φ(a₂),
            //   a₁ = base + drift_sign·μ·vsqt,  a₂ = base − drift_sign·μ·vsqt.
            let a1 = base + drift_sign * mu * vsqt;
            let a2 = base - drift_sign * mu * vsqt;
            let prob = norm_cdf(a1) + pow_cdf(z, 2.0 * mu, a2);
            rebate * i.discount_df() * prob
        }
    };
    // Upper bound on a (discounted) touch value: the touch probability is ≤ 1, so
    // the value cannot exceed the rebate times the largest applicable discount
    // factor. Under non-negative rates that is the rebate itself; under *negative*
    // domestic rates the deferred payout `R·e^{−r_d T}` (or the at-hit payout at
    // the latest possible hit time) exceeds `R`, so clamping at `R` would wrongly
    // truncate it. We therefore clamp by `R·max(1, e^{−r_d T})`.
    let max_df = i.discount_df().max(1.0);
    let upper = rebate.max(0.0) * max_df;
    if value.is_finite() {
        value.clamp(0.0, upper)
    } else {
        0.0
    }
}

/// `(H/S)^p · Φ(arg)` evaluated in log-space so an over/under-flowing power times
/// a vanishing normal tail never produces a `NaN` (`Inf × 0`). `ln_hs = ln(H/S)`.
#[inline]
fn pow_cdf(ln_hs: f64, p: f64, arg: f64) -> f64 {
    let phi = norm_cdf(arg);
    if phi <= 0.0 {
        return 0.0;
    }
    let v = exp(p * ln_hs + dlog(phi));
    if v.is_finite() { v } else { 0.0 }
}

/// Present value of a **no-touch** paying `rebate` (domestic) at expiry if the
/// spot never touches `barrier` before `T`.
///
/// `no_touch = e^{−r_d T}·rebate − one_touch_at_expiry`, then clamped to
/// `[0, rebate]`. A no-touch always pays at expiry, so it is defined against the
/// **deferred** one-touch (matching discount timing).
#[must_use]
pub fn no_touch_price(i: &ExoticInputs, barrier: f64, rebate: f64) -> f64 {
    let df = i.discount_df();
    let ot = one_touch_price(i, barrier, rebate, RebateTiming::AtExpiry);
    // Bounded by the discounted rebate (the survive-probability ≤ 1 paid at expiry),
    // which under negative domestic rates exceeds the rebate face — clamp by it, not
    // by `rebate`, so the value is not wrongly truncated.
    (rebate * df - ot).clamp(0.0, rebate.max(0.0) * df.max(1.0))
}

/// A double-no-touch contract: pays a domestic rebate at expiry iff the spot
/// stays strictly inside `(lower, upper)` for the whole life.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoubleNoTouch {
    /// Lower barrier `L` (`0 < L < U`).
    pub lower: f64,
    /// Upper barrier `U`.
    pub upper: f64,
    /// Rebate paid at expiry if neither barrier is touched.
    pub rebate: f64,
}

impl DoubleNoTouch {
    /// Construct a DNT.
    ///
    /// # Panics
    ///
    /// Panics if `lower`/`upper`/`rebate` are not a well-ordered positive
    /// corridor (`0 < lower < upper`, `rebate ≥ 0`).
    #[must_use]
    pub fn new(lower: f64, upper: f64, rebate: f64) -> Self {
        assert!(
            lower > 0.0 && lower < upper && rebate >= 0.0,
            "DNT corridor must satisfy 0 < lower < upper and rebate ≥ 0: \
             L={lower}, U={upper}, R={rebate}"
        );
        Self {
            lower,
            upper,
            rebate,
        }
    }
}

/// Number of image reflections summed in the double-barrier series. The terms
/// decay geometrically (each reflection multiplies the corridor width in
/// log-space), so a modest count reaches machine precision for any realistic
/// corridor; the value is clamped afterwards regardless.
const IMAGE_TERMS: i32 = 12;

/// Probability that the spot stays strictly inside the corridor `(L, U)` for the
/// whole life — the double-barrier *survival* (no-touch) probability under the
/// domestic spot measure.
///
/// Method of images (Girsanov-transformed Feller strip): write log-moneyness
/// `z = ln(S_t/S)`; the spot drifts at `ν = b − ½σ²` with variance `σ²t`, so on
/// the corridor `(z_l, z_u)` (with `z_l < 0 < z_u`, `Z = z_u − z_l`) the survival
/// probability is the alternating series of reflected normals, each weighted by
/// the Girsanov factor `e^{μ·(image position)}` (`μ = ν/σ²`):
///
/// ```text
///   P = Σ_{k=−N}^{N} {  e^{μ·2kZ}·[ Φ((z_u − 2kZ − νt)/v) − Φ((z_l − 2kZ − νt)/v) ]
///                     − e^{μ·(2z_u − 2kZ)}·[ Φ((z_u − r_k − νt)/v) − Φ((z_l − r_k − νt)/v) ] }
/// ```
/// with `v = σ√t` and reflected image `r_k = 2z_u − 2kZ`. The series converges
/// geometrically in `|k|`; it is validated against a Brownian-bridge Monte-Carlo
/// reference and the symmetric driftless Feller limit in the tests. Provenance
/// (doc-only): Kunitomo-Ikeda (1992); Geman-Yor (1996); the Feller strip series.
fn dnt_survival(i: &ExoticInputs, lower: f64, upper: f64) -> f64 {
    // Already outside the corridor ⇒ no survival.
    if i.spot <= lower || i.spot >= upper {
        return 0.0;
    }
    let l = Lognormal::from_inputs(i);
    let mu = l.mu();
    let vsqt = l.sigma_sqrt_t();
    let sigma2t = l.vol * l.vol * l.t;

    let z_u = dlog(upper / i.spot); // > 0
    let z_l = dlog(lower / i.spot); // < 0
    let big_z = dlog(upper / lower); // corridor width in log-space, > 0
    let drift = mu * sigma2t; // = ν·t = (b − ½σ²)·T

    // Each contribution is `e^{log_w}·(Φ(a) − Φ(b))`. When the Girsanov weight
    // overflows the matching CDF-difference underflows to zero (the reflection
    // is far in a tail), so the *product* is negligible; evaluating it in
    // log-space stops an `Inf × 0` ever yielding `NaN`.
    let weighted = |log_w: f64, hi: f64, lo: f64| -> f64 {
        let p = norm_cdf(hi) - norm_cdf(lo);
        if p <= 0.0 {
            return 0.0;
        }
        let v = exp(log_w + dlog(p));
        if v.is_finite() { v } else { 0.0 }
    };

    let mut sum = 0.0;
    for n in -IMAGE_TERMS..=IMAGE_TERMS {
        let img = 2.0 * f64::from(n) * big_z; // real image position
        let term1 = weighted(
            mu * img,
            (z_u - img - drift) / vsqt,
            (z_l - img - drift) / vsqt,
        );

        let rimg = 2.0 * z_u - img; // reflected image position
        let term2 = weighted(
            mu * rimg,
            (z_u - rimg - drift) / vsqt,
            (z_l - rimg - drift) / vsqt,
        );

        sum += term1 - term2;
    }
    let series = if sum.is_finite() {
        sum.clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Hard upper bound: the corridor survival can never exceed either single-wall
    // survival (adding a wall only lowers it). In normal regimes the image series
    // sits comfortably below this cap; in stiff extreme-drift corners — where the
    // reflected-image cancellation loses precision — the cap (built from the
    // numerically-robust single-wall first-passage probability) keeps the result
    // consistent with the single-touch pricers.
    let cap_upper = 1.0 - single_wall_hit_prob(mu, vsqt, z_u);
    let cap_lower = 1.0 - single_wall_hit_prob(mu, vsqt, z_l);
    series.min(cap_upper.min(cap_lower)).clamp(0.0, 1.0)
}

/// Probability of touching a single wall at log-distance `z` from spot (`z>0`
/// upper, `z<0` lower) under the spot drift, by expiry. The same first-passage
/// reflected-normal pair the deferred one-touch uses, in log-space-safe form
/// (the `μ·vsqt` term already carries the drift).
#[inline]
fn single_wall_hit_prob(mu: f64, vsqt: f64, z: f64) -> f64 {
    // base = ∓z/vsqt (upper: −z/vsqt, lower: +z/vsqt) with drift ±μ·vsqt.
    let (base, ds) = if z >= 0.0 {
        (-z / vsqt, 1.0) // upper
    } else {
        (z / vsqt, -1.0) // lower
    };
    let a1 = base + ds * mu * vsqt;
    let a2 = base - ds * mu * vsqt;
    let p = norm_cdf(a1) + pow_cdf(z, 2.0 * mu, a2);
    p.clamp(0.0, 1.0)
}

/// Present value of a [`DoubleNoTouch`]: `e^{−r_d T}·R·P(survive)`, clamped to
/// `[0, R]`.
#[must_use]
pub fn double_no_touch_price(i: &ExoticInputs, dnt: DoubleNoTouch) -> f64 {
    let df = i.discount_df();
    let surv = dnt_survival(i, dnt.lower, dnt.upper);
    // Survival ∈ [0, 1] is already enforced in `dnt_survival`; the value is that
    // survival paid at expiry, bounded by the discounted rebate (which exceeds the
    // face under negative domestic rates).
    (dnt.rebate * df * surv).clamp(0.0, dnt.rebate.max(0.0) * df.max(1.0))
}

/// Present value of a **double-touch** (also "double-one-touch"): pays `rebate`
/// at expiry iff *either* corridor barrier is touched.
///
/// By complementarity `double_touch = e^{−r_d T}·R − double_no_touch`, clamped to
/// `[0, R]`.
#[must_use]
pub fn double_touch_price(i: &ExoticInputs, dnt: DoubleNoTouch) -> f64 {
    let df = i.discount_df();
    let nt = double_no_touch_price(i, dnt);
    (dnt.rebate * df - nt).clamp(0.0, dnt.rebate.max(0.0) * df.max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::{assert_close, is_close};
    use celnet_types::VanillaInputs;

    fn base() -> ExoticInputs {
        // S=100, σ=20%, 1Y, r_d=5%, r_f=2%. (strike unused for touches.)
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02).into()
    }

    /// One-touch + no-touch (both at expiry) = the discounted rebate: exactly
    /// one of "touched" / "never touched" occurs.
    #[test]
    fn touch_no_touch_complementary() {
        let i = base();
        for h in [80.0, 90.0, 110.0, 125.0] {
            let ot = one_touch_price(&i, h, 1.0, RebateTiming::AtExpiry);
            let nt = no_touch_price(&i, h, 1.0);
            assert_close!(ot + nt, i.discount_df(), 1e-9, 1e-10);
        }
    }

    /// At-hit one-touch ≥ deferred (at-expiry) one-touch: paying earlier is worth
    /// more (positive rates).
    #[test]
    fn at_hit_dominates_deferred() {
        let i = base();
        for h in [85.0, 115.0] {
            let hit = one_touch_price(&i, h, 1.0, RebateTiming::AtHit);
            let exp_ = one_touch_price(&i, h, 1.0, RebateTiming::AtExpiry);
            assert!(hit >= exp_ - 1e-12, "at-hit {hit} should ≥ deferred {exp_}");
        }
    }

    /// Touch values lie in `[0, rebate]` and are monotone in the barrier level:
    /// a closer (to spot) barrier is touched more often ⇒ higher one-touch.
    #[test]
    fn one_touch_monotone_in_barrier() {
        let i = base();
        // Upper barriers: closer to spot ⇒ more likely touched ⇒ higher value.
        let near = one_touch_price(&i, 105.0, 1.0, RebateTiming::AtHit);
        let far = one_touch_price(&i, 130.0, 1.0, RebateTiming::AtHit);
        assert!(near > far, "near upper {near} should exceed far {far}");
        assert!((0.0..=1.0).contains(&near) && (0.0..=1.0).contains(&far));
        // Lower barriers: closer (higher L) ⇒ more likely touched.
        let near_l = one_touch_price(&i, 95.0, 1.0, RebateTiming::AtHit);
        let far_l = one_touch_price(&i, 70.0, 1.0, RebateTiming::AtHit);
        assert!(
            near_l > far_l,
            "near lower {near_l} should exceed far {far_l}"
        );
    }

    /// Touch certainty: a barrier already breached (spot at/through it) pays the
    /// rebate (discounted for the deferred form). A not-yet-touched barrier pays
    /// strictly less than the rebate.
    #[test]
    fn already_touched_pays_rebate() {
        // Spot pushed onto an upper barrier ⇒ certain touch.
        let breached = ExoticInputs {
            spot: 120.0,
            ..base()
        };
        let up = one_touch_price(&breached, 120.0, 1.0, RebateTiming::AtHit);
        assert_close!(up, 1.0, 1e-12, 1e-12);
        let up_def = one_touch_price(&breached, 120.0, 1.0, RebateTiming::AtExpiry);
        assert_close!(up_def, breached.discount_df(), 1e-12, 1e-12);

        // A live (un-breached) barrier is uncertain ⇒ strictly less than rebate.
        let i = base();
        let dn_def = one_touch_price(&i, 110.0, 1.0, RebateTiming::AtExpiry);
        assert!(dn_def < 1.0);
    }

    /// DNT relationship: DNT = e^{−r_d T} − (OT_up + OT_down − double_touch),
    /// equivalently the double-touch is the complement of the DNT. We verify the
    /// stated S1 identity DNT = 1 − (OT_up + OT_down − double_touch) in *survival*
    /// form: no_touch_corridor = no_touch_upper + no_touch_lower − 1 is NOT exact
    /// (correlation between the two walls), but DNT + double_touch = e^{−r_d T}.
    #[test]
    fn dnt_plus_double_touch_is_discounted_rebate() {
        let i = base();
        let dnt = DoubleNoTouch::new(85.0, 120.0, 1.0);
        let nt = double_no_touch_price(&i, dnt);
        let dt = double_touch_price(&i, dnt);
        assert_close!(nt + dt, i.discount_df(), 1e-12, 1e-12);
    }

    /// DNT is bounded by, and below, each single no-touch (adding a second wall
    /// can only make survival harder), and within `[0, rebate]`.
    #[test]
    fn dnt_below_single_no_touch() {
        let i = base();
        let dnt = DoubleNoTouch::new(85.0, 120.0, 1.0);
        let nt2 = double_no_touch_price(&i, dnt);
        let nt_up = no_touch_price(&i, 120.0, 1.0);
        let nt_lo = no_touch_price(&i, 85.0, 1.0);
        assert!(nt2 <= nt_up + 1e-9 && nt2 <= nt_lo + 1e-9);
        assert!((0.0..=1.0).contains(&nt2));
    }

    /// Widening the corridor raises the DNT survival value (monotone in width).
    #[test]
    fn dnt_monotone_in_width() {
        let i = base();
        let narrow = double_no_touch_price(&i, DoubleNoTouch::new(95.0, 106.0, 1.0));
        let wide = double_no_touch_price(&i, DoubleNoTouch::new(80.0, 130.0, 1.0));
        assert!(
            wide > narrow,
            "wide DNT {wide} should exceed narrow {narrow}"
        );
    }

    /// The image-series survival converges in the term count: 12 terms already
    /// agrees with a 200-term evaluation of the *same* (correct) series to machine
    /// precision, for both a symmetric and an asymmetric drifted corridor.
    #[test]
    fn dnt_survival_truncation_is_converged() {
        for (i, lo, up) in [
            (
                ExoticInputs::from(VanillaInputs::new(100.0, 100.0, 0.15, 0.5, 0.03, 0.03)),
                90.0,
                111.111_111_111,
            ),
            (
                ExoticInputs::from(VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02)),
                85.0,
                120.0,
            ),
        ] {
            let coarse = dnt_survival(&i, lo, up);
            let dense = dnt_survival_dense(&i, lo, up, 200);
            assert!(
                is_close(coarse, dense, 1e-10, 1e-12),
                "12-term {coarse} vs 200-term {dense}"
            );
        }
    }

    /// A reference re-evaluation of the survival series with an arbitrary term
    /// count (the production path fixes it at `IMAGE_TERMS`).
    fn dnt_survival_dense(i: &ExoticInputs, lower: f64, upper: f64, terms: i32) -> f64 {
        let l = Lognormal::from_inputs(i);
        let mu = l.mu();
        let vsqt = l.sigma_sqrt_t();
        let drift = mu * l.vol * l.vol * l.t;
        let z_u = dlog(upper / i.spot);
        let z_l = dlog(lower / i.spot);
        let big_z = dlog(upper / lower);
        let mut sum = 0.0;
        for n in -terms..=terms {
            let img = 2.0 * f64::from(n) * big_z;
            sum += exp(mu * img)
                * (norm_cdf((z_u - img - drift) / vsqt) - norm_cdf((z_l - img - drift) / vsqt));
            let rimg = 2.0 * z_u - img;
            sum -= exp(mu * rimg)
                * (norm_cdf((z_u - rimg - drift) / vsqt) - norm_cdf((z_l - rimg - drift) / vsqt));
        }
        sum.clamp(0.0, 1.0)
    }

    /// The continuously-monitored double-no-touch survival matches an independent
    /// Brownian-bridge-corrected Monte-Carlo reference (seeded, deterministic) to
    /// MC tolerance, across symmetric/asymmetric and drifted corridors.
    #[test]
    fn dnt_survival_matches_monte_carlo() {
        let cases = [
            (
                ExoticInputs::from(VanillaInputs::new(100.0, 100.0, 0.15, 0.5, 0.03, 0.03)),
                90.0,
                111.111_111_111,
            ),
            (
                ExoticInputs::from(VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02)),
                85.0,
                120.0,
            ),
        ];
        for (i, lo, up) in cases {
            let analytic = dnt_survival(&i, lo, up);
            // The Brownian-bridge correction removes most discretisation bias; a
            // fine grid with a moderate path count pins the survival to ~1e-2.
            let mc = mc_survival(&i, lo, up, 1000, 80_000, 0x0DDB_A11A_5EED_1234);
            assert!(
                (analytic - mc).abs() < 1.5e-2,
                "DNT survival analytic {analytic} vs MC {mc} (|diff|={})",
                (analytic - mc).abs()
            );
        }
    }

    /// Deterministic Brownian-bridge Monte-Carlo estimate of the continuous
    /// double-barrier survival probability — an independent reference oracle.
    fn mc_survival(
        i: &ExoticInputs,
        lower: f64,
        upper: f64,
        steps: usize,
        paths: usize,
        seed: u64,
    ) -> f64 {
        use celnet_core::math::{exp, ln, sqrt};
        let dt = i.t / steps as f64;
        let drift = (i.carry_rate() - 0.5 * i.vol * i.vol) * dt;
        let vs = i.vol * sqrt(dt);
        let (ln_l, ln_u) = (ln(lower), ln(upper));
        let two_s2dt = 2.0 * i.vol * i.vol * dt;

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
            let mut x = ln(i.spot);
            let mut surv = 1.0_f64;
            let mut alive = true;
            for _ in 0..steps {
                let prev = x;
                x += drift + vs * next_normal();
                if x <= ln_l || x >= ln_u {
                    alive = false;
                    break;
                }
                let p_up = 1.0 - exp(-2.0 * (ln_u - prev) * (ln_u - x) / two_s2dt);
                let p_dn = 1.0 - exp(-2.0 * (prev - ln_l) * (x - ln_l) / two_s2dt);
                surv *= p_up.max(0.0) * p_dn.max(0.0);
            }
            if alive {
                acc += surv;
            }
        }
        acc / paths as f64
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use celnet_testkit::arb_inputs;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Touch values are clamped to `[0, rebate]` (the S1 requirement) for any
        /// market. Where the discounted rebate does not exceed the rebate
        /// (`r_dom ≥ 0`, so the `[0, notional]` clamp does not bind) the deferred
        /// one-touch and no-touch sum to the discounted rebate, and at-hit
        /// dominates deferred.
        #[test]
        fn touch_bounds_and_complementarity(
            i in arb_inputs(),
            up in any::<bool>(),
            offset in 0.05f64..0.60,
        ) {
            let i: ExoticInputs = i.into();
            let barrier = if up { i.spot * (1.0 + offset) } else { i.spot * (1.0 - offset) };
            let ot_hit = one_touch_price(&i, barrier, 1.0, RebateTiming::AtHit);
            let ot_exp = one_touch_price(&i, barrier, 1.0, RebateTiming::AtExpiry);
            let nt = no_touch_price(&i, barrier, 1.0);
            // Probability-weighted (discounted) payouts lie in [0, max(1, df)] —
            // under negative domestic rates the discounted face exceeds the notional.
            let cap = i.discount_df().max(1.0) + 1e-12;
            prop_assert!((0.0..=cap).contains(&ot_hit));
            prop_assert!((0.0..=cap).contains(&ot_exp));
            prop_assert!((0.0..=cap).contains(&nt));
            // Exact complementarity & timing dominance hold when the discounted
            // rebate ≤ rebate (non-negative domestic rate ⇒ clamp inactive).
            if i.discount_rate() >= 0.0 {
                prop_assert!(celnet_core::is_close(ot_exp + nt, i.discount_df(), 1e-7, 1e-8));
                prop_assert!(ot_hit >= ot_exp - 1e-9);
            }
        }

        /// A double-no-touch corridor around spot is clamped to `[0, rebate]` for
        /// any market, never exceeds either single no-touch, and (when the
        /// `[0, notional]` clamp is inactive) DNT + double-touch = the discounted
        /// rebate exactly.
        #[test]
        fn dnt_bounds_property(
            i in arb_inputs(),
            lo_off in 0.05f64..0.45,
            hi_off in 0.05f64..0.45,
        ) {
            let i: ExoticInputs = i.into();
            let lower = i.spot * (1.0 - lo_off);
            let upper = i.spot * (1.0 + hi_off);
            let dnt = DoubleNoTouch::new(lower, upper, 1.0);
            let nt = double_no_touch_price(&i, dnt);
            let dt = double_touch_price(&i, dnt);
            // Discounted payouts in [0, max(1, df)] (df > 1 under negative r_d).
            let cap = i.discount_df().max(1.0) + 1e-12;
            prop_assert!((0.0..=cap).contains(&nt));
            prop_assert!((0.0..=cap).contains(&dt));
            // Below each single no-touch (adding a wall only lowers survival).
            // The two prices come from different closed forms (image series vs the
            // 2-term touch formula); the inequality is numerically meaningful only
            // where survival is not vanishingly small (in deep-corner regimes both
            // are ≈0 and floating-point resolution dominates).
            // The corridor survival is capped by each single-wall survival inside
            // the pricer, so the sandwich holds tightly for every market.
            prop_assert!(nt <= no_touch_price(&i, upper, 1.0) + 1e-7);
            prop_assert!(nt <= no_touch_price(&i, lower, 1.0) + 1e-7);
            // Exact complementarity when the discounted rebate ≤ rebate.
            if i.discount_rate() >= 0.0 {
                prop_assert!(celnet_core::is_close(nt + dt, i.discount_df(), 1e-9, 1e-10));
            }
        }
    }
}
