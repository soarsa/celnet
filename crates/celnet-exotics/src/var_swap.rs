//! Variance-swap fair strike by **log-contract static replication**.
//!
//! A variance swap pays the realised variance of the log-returns of an
//! underlying against a fixed *fair variance strike* `K_var`. The model-free
//! result — replicating the log contract `−2·E[ln(S_T/F)]` with a continuum of
//! out-of-the-money European options — gives the fair (annualised) variance as
//! the `1/K²`-weighted strip of OTM option *forward* (undiscounted) values:
//!
//! ```text
//!            2   ⎡  ⌠ F  P̃(K)        ⌠ ∞  C̃(K)      ⎤
//!  K_var =  ─── ⎢   ⎮   ───── dK  +   ⎮    ───── dK  ⎥
//!            T   ⎣  ⌡ 0   K²          ⌡ F    K²       ⎦
//! ```
//!
//! with `F` the outright forward, `P̃(K) = E^Q[(K−S_T)^+]` the **forward**
//! (undiscounted) put for `K ≤ F`, and `C̃(K) = E^Q[(S_T−K)^+]` the forward
//! call for `K ≥ F`. Each option is priced from the *smile-consistent* implied
//! volatility `σ(K)` read off an arbitrage-free [`Smile`], so the fair variance
//! is the genuine smile-weighted figure (not a single-vol Black number).
//!
//! # Why the forward (undiscounted) values
//!
//! The static replication is a statement under the `T`-forward measure: the fair
//! variance equals `−(2/T)·E^Q[ln(S_T/F)]`, and the Carr–Madan identity rewrites
//! that expectation as the OTM strip of **expectations** `E^Q[(K−S_T)^+]` /
//! `E^Q[(S_T−K)^+]`. Those are forward (undiscounted) option values; the present
//! value priced by [`celnet_vanilla::price`] carries the domestic discount factor
//! `e^{−r_d T}`, which we divide out. Equivalently the whole integral may be
//! discounted and then re-inflated by `e^{r_d T}`; we do the per-option division
//! so the integrand is measure-correct term by term.
//!
//! # Numerical strip — convergent, adaptive-wing, fixed-resolution
//!
//! The two semi-infinite integrals are evaluated on a **log-spaced** strike grid
//! (uniform in `u = ln(K/F)`), which keeps the `u`-resolution constant while the
//! strike spacing widens with the wings. On each leg:
//!
//! * the change of variable `K = F·e^u`, `dK = K du` turns `∫ Õ(K)/K² dK` into
//!   `∫ Õ(F e^u)/(F e^u) du`, integrated by the composite **Simpson** rule at a
//!   uniform step `h = wing_std·σ_atm·√T / nodes_per_leg`;
//! * the wing is **extended adaptively**, block by block of width
//!   `wing_std·σ_atm·√T`, until a block's contribution falls below a tiny relative
//!   fraction of the accumulated leg. A fixed `σ√T`-multiple truncation
//!   under-integrates exactly when the smile extrapolates to *rising* far-wing
//!   vols (the integrand then decays only slowly); the adaptive extension makes
//!   the leg converged for **any** smile shape, not just a fast-decaying one;
//! * Simpson is second-order accurate in the (constant) `u`-step, so refining the
//!   node count moves the strip by `O(h²)`; the production default sits on the
//!   `~1e-9` convergence plateau (the parity convergence row pins this).
//!
//! # Method provenance (doc comments only)
//!
//! Log-contract static replication of variance: Neuberger (1994); Dupire (1993);
//! the OTM-strip presentation Demeterfi, Derman, Kamal & Zou (1999); the
//! spanning identity Carr & Madan (1998, 2001). All identifiers here are
//! purpose-named and vendor/research-neutral; provenance lives only in docs.

use celnet_core::Smile;
use celnet_core::math::exp;
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::price as vanilla_price;

/// Inputs that fix the variance-swap valuation context: the forward, expiry,
/// and the domestic/foreign carry needed to price the replicating options.
///
/// The smile itself is passed separately (as any [`Smile`]) so the same context
/// can be evaluated against different surfaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VarSwapContext {
    /// Outright forward `F` (quote per 1 unit of base).
    pub forward: f64,
    /// Time to expiry in years `T` (vol-time).
    pub t: f64,
    /// Continuously-compounded domestic (quote) rate `r_d`.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) rate `r_f`.
    pub r_for: f64,
}

impl VarSwapContext {
    /// Build a context from a forward, expiry and rates.
    #[must_use]
    pub const fn new(forward: f64, t: f64, r_dom: f64, r_for: f64) -> Self {
        Self {
            forward,
            t,
            r_dom,
            r_for,
        }
    }

    /// Build a context from a [`VanillaInputs`] template: the forward is derived
    /// from spot and carry, expiry and rates are copied. The template's
    /// `strike`/`vol` fields are ignored (the smile supplies per-strike vols).
    #[must_use]
    pub fn from_inputs(i: &VanillaInputs) -> Self {
        Self {
            forward: i.forward(),
            t: i.t,
            r_dom: i.r_dom,
            r_for: i.r_for,
        }
    }
}

/// Discretisation of the OTM strike strip used to replicate the log contract.
///
/// The strip integrates each leg in fixed-resolution `u = ln(K/F)` blocks and
/// **extends the wing adaptively** until the marginal block is negligible, so
/// the result is converged for any smile shape (including ones whose far-wing
/// vols rise under extrapolation). The two knobs only set the resolution and the
/// reference block width; the [`Default`] sits on the `~1e-9` convergence plateau
/// (see the parity convergence row).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VarSwapStrip {
    /// Number of Simpson sub-intervals across **one reference block** (a block is
    /// `wing_std · σ_atm · √T` wide in `u`). This fixes the uniform `u`-step
    /// `h = wing_std·σ_atm·√T / nodes_per_leg` used for every block on both legs.
    pub nodes_per_leg: usize,
    /// Reference block width in standard-deviation units: the wing is integrated
    /// in blocks of `wing_std · σ_atm · √T` in log-moneyness `u`, extended
    /// outward until the tail contribution is negligible.
    pub wing_std: f64,
}

impl Default for VarSwapStrip {
    fn default() -> Self {
        Self {
            // 4000 Simpson panels per 6-σ reference block (≈ 667 panels/σ√T) with
            // the adaptive wing is on the ~1e-9 plateau for any FX smile, while
            // staying a sub-millisecond strip.
            nodes_per_leg: 4000,
            wing_std: 6.0,
        }
    }
}

/// The fair (annualised) variance strike `K_var` of a variance swap, with the
/// realised-vol equivalent and the strip diagnostics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VarSwapResult {
    /// Fair annualised variance `K_var` (so a flat `σ` returns `σ²`). The
    /// variance-swap *vol strike* quoted to a desk is `√K_var`.
    pub fair_variance: f64,
    /// Contribution of the put leg (`K ≤ F`) to `K_var`.
    pub put_leg: f64,
    /// Contribution of the call leg (`K ≥ F`) to `K_var`.
    pub call_leg: f64,
}

impl VarSwapResult {
    /// `√K_var` — the fair variance-swap volatility strike (the number quoted to
    /// a trader). This is **not** the volatility-swap fair strike, which carries
    /// a (negative) convexity adjustment; see [`crate::vol_swap`].
    #[must_use]
    pub fn fair_vol(&self) -> f64 {
        self.fair_variance.max(0.0).sqrt()
    }
}

/// Forward (undiscounted) value of the OTM option at strike `K`:
/// `E^Q[(K−S_T)^+]` for a put (`K ≤ F`) or `E^Q[(S_T−K)^+]` for a call
/// (`K ≥ F`), priced at the smile vol `σ(K)`.
///
/// The present value from [`celnet_vanilla::price`] carries `e^{−r_d T}`; we
/// divide it out to land in the forward measure the replication is stated in.
#[inline]
fn otm_forward_value<S: Smile>(smile: &S, ctx: &VarSwapContext, k: f64, opt: OptionType) -> f64 {
    let sigma = smile.implied_vol(k, ctx.forward, ctx.t).0;
    // Spot consistent with the forward: F = S·e^{(r_d−r_f)T} ⇒ S = F·e^{−(r_d−r_f)T}.
    let spot = ctx.forward * exp(-(ctx.r_dom - ctx.r_for) * ctx.t);
    let inputs = VanillaInputs {
        spot,
        strike: k,
        vol: sigma,
        t: ctx.t,
        r_dom: ctx.r_dom,
        r_for: ctx.r_for,
    };
    let pv = vanilla_price(opt, &inputs);
    // Undiscount to the forward measure (divide out the domestic discount).
    pv * exp(ctx.r_dom * ctx.t)
}

/// Composite-Simpson integral of one block `[u_lo, u_hi]` of a strip leg in
/// log-moneyness `u`, at step `h` (so `n = (u_hi−u_lo)/h` even panels).
///
/// The substitution `K = F·e^u`, `dK = K du` turns `∫ Õ(K)/K² dK` into
/// `∫ Õ(F e^u)/(F e^u) du`. `opt` selects the OTM flavour for the leg. The step
/// `h` is held constant across blocks so the adaptive-wing extension below keeps
/// a uniform `u`-resolution.
fn simpson_block<S: Smile>(
    smile: &S,
    ctx: &VarSwapContext,
    u_lo: f64,
    u_hi: f64,
    h: f64,
    opt: OptionType,
) -> f64 {
    let f = ctx.forward;
    // Integrand g(u) = Õ(F e^u) / (F e^u): the per-`du` contribution to
    // ∫ Õ(K)/K² dK after the K = F e^u change of variable.
    let g = |u: f64| -> f64 {
        let k = f * exp(u);
        otm_forward_value(smile, ctx, k, opt) / k
    };

    // Even panel count covering the block at (approximately) the target step.
    let span = u_hi - u_lo;
    let mut n = (span / h).round() as usize;
    if n < 2 {
        n = 2;
    }
    if !n.is_multiple_of(2) {
        n += 1;
    }
    let hh = span / n as f64;

    let mut acc = g(u_lo) + g(u_hi);
    for j in 1..n {
        let u = u_lo + j as f64 * hh;
        let w = if j.is_multiple_of(2) { 2.0 } else { 4.0 };
        acc += w * g(u);
    }
    acc * hh / 3.0
}

/// One OTM leg of the strip with an **adaptive wing**: integrate outward from
/// the forward in fixed-width `u`-blocks (uniform resolution) and keep extending
/// the wing until a block's contribution is negligible relative to the leg total.
///
/// A fixed `σ√T`-multiple wing under-integrates when the smile extrapolates to
/// *rising* far-wing vols (the integrand then decays only slowly), which is
/// exactly where a naïve truncation loses accuracy. Extending until the marginal
/// block is below `REL_CUTOFF·leg` makes the leg converged for any smile shape.
fn integrate_leg<S: Smile>(
    smile: &S,
    ctx: &VarSwapContext,
    h: f64,
    block_width: f64,
    sign: f64, // +1 for the call wing (u > 0), −1 for the put wing (u < 0)
    opt: OptionType,
    max_u: f64,
) -> f64 {
    // Relative cutoff: stop extending once a block adds < this fraction of the
    // accumulated leg (and we have integrated at least the first block).
    const REL_CUTOFF: f64 = 1e-12;

    let mut leg = 0.0;
    let mut u0 = 0.0_f64;
    loop {
        let u1 = u0 + block_width;
        let (a, b) = if sign > 0.0 { (u0, u1) } else { (-u1, -u0) };
        let block = simpson_block(smile, ctx, a, b, h, opt);
        leg += block;
        // Stop when the marginal block is negligible vs the leg, or the hard
        // safety bound on the wing is reached (well past any material mass).
        if (u1 >= max_u) || (leg > 0.0 && block <= REL_CUTOFF * leg) {
            break;
        }
        u0 = u1;
    }
    leg
}

/// Fair variance strike `K_var` of a variance swap by log-contract static
/// replication over the supplied [`Smile`], with an explicit strip
/// discretisation.
///
/// `K_var` is the **annualised** fair variance, so for a flat smile `σ` it
/// returns `σ²` (the genuinely independent closed-form oracle the parity row
/// pins). The realised-vol-equivalent variance-swap strike is `√K_var`
/// ([`VarSwapResult::fair_vol`]).
///
/// # Panics
///
/// Never panics for well-posed inputs (`t > 0`, `forward > 0`, a finite smile).
#[must_use]
pub fn fair_variance_with<S: Smile>(
    smile: &S,
    ctx: &VarSwapContext,
    strip: VarSwapStrip,
) -> VarSwapResult {
    // ATM σ√T sets the reference scale: `wing_std` of them is the per-block
    // width and (with `nodes_per_leg`) fixes the uniform `u`-step `h`. The wing
    // is then extended adaptively block-by-block until its tail is negligible.
    let atm = smile.implied_vol(ctx.forward, ctx.forward, ctx.t).0;
    let block_width = strip.wing_std * atm * ctx.t.sqrt();
    // Uniform step: `nodes_per_leg` panels across one reference block width.
    let h = block_width / strip.nodes_per_leg as f64;
    // Hard safety bound on the wing (far past any material mass for any smile):
    // many reference widths out. The adaptive cutoff almost always stops first.
    let max_u = block_width * 8.0;

    // Put leg (u < 0, OTM puts) and call leg (u > 0, OTM calls).
    let put_leg = integrate_leg(smile, ctx, h, block_width, -1.0, OptionType::Put, max_u);
    let call_leg = integrate_leg(smile, ctx, h, block_width, 1.0, OptionType::Call, max_u);

    let scale = 2.0 / ctx.t;
    VarSwapResult {
        fair_variance: scale * (put_leg + call_leg),
        put_leg: scale * put_leg,
        call_leg: scale * call_leg,
    }
}

/// Fair variance strike `K_var` with the production-default strip
/// ([`VarSwapStrip::default`]).
#[must_use]
pub fn fair_variance<S: Smile>(smile: &S, ctx: &VarSwapContext) -> VarSwapResult {
    fair_variance_with(smile, ctx, VarSwapStrip::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::FlatSmile;
    use celnet_surface::MarketHedgeSmile;

    fn ctx() -> VarSwapContext {
        // EURUSD-like 1Y, F ≈ spot·e^{(r_d−r_f)T}.
        let i = VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01);
        VarSwapContext::from_inputs(&i)
    }

    #[test]
    fn flat_smile_recovers_sigma_squared() {
        // The genuinely independent closed-form oracle: a flat smile σ must give
        // K_var = σ² exactly (model-free replication of a lognormal world).
        for sigma in [0.05_f64, 0.10, 0.20, 0.35] {
            let c = ctx();
            let res = fair_variance(&FlatSmile::new(sigma), &c);
            celnet_core::assert_close!(res.fair_variance, sigma * sigma, 1e-7, 1e-9);
        }
    }

    #[test]
    fn legs_are_positive_and_sum_to_total() {
        let c = ctx();
        let res = fair_variance(&FlatSmile::new(0.12), &c);
        assert!(res.put_leg > 0.0 && res.call_leg > 0.0);
        celnet_core::assert_close!(res.fair_variance, res.put_leg + res.call_leg, 1e-15, 1e-15);
    }

    #[test]
    fn convex_smile_lifts_fair_variance_above_atm() {
        // A positive-butterfly (convex) smile has OTM vols above ATM, so the
        // 1/K²-weighted strip exceeds σ_atm² — fair variance > ATM variance.
        let c = ctx();
        let f = c.forward;
        let (kp, kc) = (f / 1.10, f * 1.10);
        let smile = MarketHedgeSmile::new([kp, f, kc], [0.115, 0.10, 0.115], f, c.t);
        let res = fair_variance(&smile, &c);
        assert!(
            res.fair_variance > 0.10 * 0.10,
            "convex smile must lift fair variance above ATM σ²: {} vs {}",
            res.fair_variance,
            0.01
        );
    }

    #[test]
    fn fair_vol_is_sqrt_of_fair_variance() {
        let c = ctx();
        let res = fair_variance(&FlatSmile::new(0.18), &c);
        celnet_core::assert_close!(res.fair_vol(), res.fair_variance.sqrt(), 1e-15, 1e-15);
    }
}
