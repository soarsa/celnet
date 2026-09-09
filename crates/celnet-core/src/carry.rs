//! The generalized, carry-tagged pricing vocabulary and the `CarryPricer` seam.
//!
//! This is the asset-class-agnostic generalization of the FX vanilla input/output
//! pair. Where [`celnet_types::VanillaInputs`] is the FX leaf's *legitimate* input
//! (spot, strike, vol, time, and the FX two-rate carry baked in as `r_dom`/`r_for`),
//! [`CarryInputs`] carries the same market state but parameterizes the forward and
//! discounting through a [`Carry`] discriminator and tags the underlying with an
//! [`Underlying`]. This lets *one* unversioned pricing contract name FX today and
//! grow to other asset classes additively (equity `b = r − q`, commodity
//! `b = r − convenience`, digital-asset `b = r − funding`) without ever touching the
//! FX leaf's Garman-Kohlhagen arithmetic.
//!
//! The [`CarryPricer`] trait is the seam a concrete pricing leaf implements. The FX
//! (Garman-Kohlhagen) leaf lives in `celnet-vanilla` and implements it by delegating
//! to its existing `price`/`greeks` — the generalization is purely at the *type*
//! level; the FX numbers are byte-for-byte unchanged (proved by
//! [`fx_carry_inputs_byte_identical`] here and by the full-grid `to_bits` gate in
//! `celnet-vanilla`).

use celnet_types::{
    Carry, DigitalKind, DiscountCurve, ExoticKind, Greeks, OptionType, RateSensitivities,
    Underlying, VanillaInputs,
};

use crate::math::{ln, norm_cdf, norm_pdf, sqrt};

/// Generalized, carry-tagged pricing input.
///
/// The market state (`spot`, `strike`, `vol`, `t`) shared by every asset class,
/// plus the [`Underlying`] discriminator (*what* is priced) and the [`Carry`]
/// model (*how* the forward and discounting are formed). The forward and discount
/// factor are delegated to [`Carry`], so the FX arm reproduces the FX two-rate
/// arithmetic bit-for-bit (see [`Carry::FxRates`]).
///
/// `CarryInputs` is `Clone` but not `Copy`: the [`Underlying`] discriminator now
/// carries string-bearing cross-asset arms (equity / commodity / digital-asset
/// symbols), which cannot be `Copy`. The pricing seam takes `&CarryInputs`, so
/// the hot path never copies it regardless.
#[derive(Debug, Clone, PartialEq)]
pub struct CarryInputs {
    /// Spot price of the underlying (quote per 1 unit of base, for FX).
    pub spot: f64,
    /// Strike (quote per 1 unit of base, for FX).
    pub strike: f64,
    /// Annualized volatility (absolute, e.g. `0.10` = 10 vol).
    pub vol: f64,
    /// Time to expiry in years (vol-time).
    pub t: f64,
    /// The underlying asset (FX pair today; further arms added by their wave).
    pub underlying: Underlying,
    /// The cost-of-carry model behind the forward and discounting.
    pub carry: Carry,
}

impl CarryInputs {
    /// Construct a generalized pricing input.
    #[must_use]
    pub const fn new(
        spot: f64,
        strike: f64,
        vol: f64,
        t: f64,
        underlying: Underlying,
        carry: Carry,
    ) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            underlying,
            carry,
        }
    }

    /// Outright forward `F = spot · e^{b·t}`, delegated to [`Carry`].
    ///
    /// For an FX [`Carry::FxRates`] carry this is byte-identical to
    /// [`VanillaInputs::forward`].
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.spot * self.carry.forward_factor(self.t)
    }

    /// Discount factor `e^{−r·t}` (the numeraire discount), delegated to [`Carry`].
    ///
    /// For an FX [`Carry::FxRates`] carry this is byte-identical to
    /// [`VanillaInputs::df_dom`].
    #[must_use]
    pub fn discount_df(&self) -> f64 {
        self.carry.discount_df(self.t)
    }
}

/// The generalized first-order-plus output strip of a [`CarryPricer`].
///
/// This is the asset-class-agnostic mirror of [`celnet_types::Greeks`]: the
/// non-rate sensitivities are carry-neutral and shared verbatim, while the rate
/// sensitivities are tagged by [`RateSensitivities`] so the FX arm reports the two
/// FX rhos and other asset classes report the discount/carry rho pair. The `price`
/// and the carry-neutral block are kept identical to [`Greeks`] (an FX leaf maps
/// across them by simple field copy), so the FX projection is byte-identical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarryGreeks {
    /// Present value (premium) in the numeraire currency, per 1 unit of base.
    pub price: f64,
    /// Spot delta (premium-unadjusted): `∂V/∂S` scaled appropriately.
    pub delta_spot: f64,
    /// Forward delta (premium-unadjusted).
    pub delta_forward: f64,
    /// Gamma: `∂²V/∂S²`.
    pub gamma: f64,
    /// Vega: `∂V/∂σ` (per `1.0` absolute vol).
    pub vega: f64,
    /// Theta: `∂V/∂t` per year (`−∂V/∂T`).
    pub theta: f64,
    /// Carry-tagged rate sensitivities (FX two rhos, or discount/carry rho pair).
    pub rates: RateSensitivities,
    /// Vanna: `∂²V/∂S∂σ` (= `∂delta_spot/∂σ`).
    pub vanna: f64,
    /// Volga / vomma: `∂²V/∂σ²`.
    pub volga: f64,
    /// Charm: `∂(delta_spot)/∂T` (delta decay, per year).
    pub charm: f64,
    /// Speed: `∂³V/∂S³` (= `∂gamma/∂S`).
    pub speed: f64,
    /// Zomma: `∂gamma/∂σ`.
    pub zomma: f64,
    /// Color: `∂gamma/∂T`.
    pub color: f64,
}

/// Why a [`CarryPricer`] could not price a [`CarryInputs`].
///
/// A leaf rejects an input it is not the correct pricer for — e.g. the FX
/// (Garman-Kohlhagen) leaf rejects a non-FX [`Underlying`] or a non-FX [`Carry`]
/// rather than silently mis-pricing it under FX arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarryPriceError {
    /// The leaf does not price this [`Underlying`] asset class.
    UnsupportedUnderlying,
    /// The leaf does not price under this [`Carry`] model.
    UnsupportedCarry,
}

impl core::fmt::Display for CarryPriceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CarryPriceError::UnsupportedUnderlying => {
                f.write_str("pricer does not support this underlying asset class")
            }
            CarryPriceError::UnsupportedCarry => {
                f.write_str("pricer does not support this cost-of-carry model")
            }
        }
    }
}

impl core::error::Error for CarryPriceError {}

/// A pricing leaf over the generalized, carry-tagged vocabulary.
///
/// A concrete leaf (e.g. the FX Garman-Kohlhagen leaf in `celnet-vanilla`) prices a
/// [`CarryInputs`] it supports and rejects one it does not with a typed
/// [`CarryPriceError`] (never a silent mis-price). This is the asset-class-agnostic
/// pricing seam the wire and clients can grow against without forking the contract.
pub trait CarryPricer {
    /// Present value of the option, or an error if this leaf does not price the
    /// given underlying/carry combination.
    ///
    /// # Errors
    /// Returns [`CarryPriceError`] when the [`Underlying`] or [`Carry`] is outside
    /// this leaf's supported asset class.
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> Result<f64, CarryPriceError>;

    /// Price plus the full generalized sensitivity strip, or an error.
    ///
    /// # Errors
    /// Returns [`CarryPriceError`] when the [`Underlying`] or [`Carry`] is outside
    /// this leaf's supported asset class.
    fn price_greeks(
        &self,
        opt: OptionType,
        inputs: &CarryInputs,
    ) -> Result<CarryGreeks, CarryPriceError>;
}

/// Lower an FX [`CarryInputs`] (an `Fx` or `Metal` underlying carried by
/// [`Carry::FxRates`]) to the FX leaf's [`VanillaInputs`].
///
/// Both FX and precious-metal underlyings price through this leaf: a metal's
/// lease rate is modelled as the FX foreign rate (`Carry::FxRates.r_for`), so the
/// XAU/XAG/XPT/XPD-vs-fiat forward/discount arithmetic is the identical FX
/// two-rate path (ADR-0008 §metals byte-identity). Returns [`CarryPriceError`]
/// for a non-`FxRates` carry (the equity/commodity cost-of-carry arms route to
/// their own leaves). The mapping is a pure field copy of `(r_dom, r_for)` — so
/// the resulting `forward`/`df_dom`/`df_for` are byte-identical to the
/// generalized [`CarryInputs::forward`]/[`CarryInputs::discount_df`] (proved in
/// [`fx_carry_inputs_byte_identical`]).
///
/// # Errors
/// Returns [`CarryPriceError::UnsupportedCarry`] for a non-`FxRates` carry.
pub fn fx_vanilla_inputs(inputs: &CarryInputs) -> Result<VanillaInputs, CarryPriceError> {
    match inputs.underlying {
        // FX and metals both lower through the FX two-rate path (metal lease rate
        // modelled as the foreign rate).
        Underlying::Fx(_) | Underlying::Metal(_) => {}
        // The cross-asset arms (equity / commodity / digital-asset) route to
        // their own carry leaves (the generalized cost-of-carry / inverse-coin
        // paths), not the FX two-rate lowering — refuse rather than misprice.
        Underlying::Equity(_) | Underlying::Commodity(_) | Underlying::DigitalAsset(_) => {
            return Err(CarryPriceError::UnsupportedUnderlying);
        }
    }
    let (r_dom, r_for) = match inputs.carry {
        Carry::FxRates { r_dom, r_for } => (r_dom, r_for),
        Carry::CostOfCarry { .. } => return Err(CarryPriceError::UnsupportedCarry),
    };
    Ok(VanillaInputs::new(
        inputs.spot,
        inputs.strike,
        inputs.vol,
        inputs.t,
        r_dom,
        r_for,
    ))
}

/// Lift an FX leaf's [`Greeks`] into the generalized [`CarryGreeks`] strip, tagging
/// the rate sensitivities as [`RateSensitivities::Fx`] (the two FX rhos verbatim).
///
/// Every carry-neutral field is copied bit-for-bit, so the FX projection of
/// `CarryGreeks` is byte-identical to the FX [`Greeks`].
#[must_use]
pub fn fx_carry_greeks(g: &Greeks) -> CarryGreeks {
    CarryGreeks {
        price: g.price,
        delta_spot: g.delta_spot,
        delta_forward: g.delta_forward,
        gamma: g.gamma,
        vega: g.vega,
        theta: g.theta,
        rates: RateSensitivities::Fx {
            rho_dom: g.rho_dom,
            rho_for: g.rho_for,
        },
        vanna: g.vanna,
        volga: g.volga,
        charm: g.charm,
        speed: g.speed,
        zomma: g.zomma,
        color: g.color,
    }
}

/// Project a generalized [`CarryGreeks`] back into the FX-shaped [`Greeks`] strip.
///
/// The carry-neutral fields are copied bit-for-bit; the two FX rate rhos are the
/// documented lossless projection of the carry-tagged rate block
/// ([`RateSensitivities::flat_rhos`]): for the [`RateSensitivities::Carry`] arm,
/// `rho_dom = discount_rho + carry_rho`, `rho_for = −carry_rho` (the exact inverse
/// of how the cross-asset leaves populate the carry arm — for FX `r = r_dom`,
/// `b = r_dom − r_for`). This is the inverse companion of [`fx_carry_greeks`]: it
/// lets the FX (Garman-Kohlhagen) leaf keep its two-rate `rho_dom`/`rho_for`
/// **output basis** while its core gBSM math is produced by the unified
/// [`gbsm_carry_greeks`] kernel (ADR-0012).
#[must_use]
pub fn carry_greeks_to_greeks(cg: &CarryGreeks) -> Greeks {
    let (rho_dom, rho_for) = cg.rates.flat_rhos();
    Greeks {
        price: cg.price,
        delta_spot: cg.delta_spot,
        delta_forward: cg.delta_forward,
        gamma: cg.gamma,
        vega: cg.vega,
        theta: cg.theta,
        rho_dom,
        rho_for,
        vanna: cg.vanna,
        volga: cg.volga,
        charm: cg.charm,
        speed: cg.speed,
        zomma: cg.zomma,
        color: cg.color,
    }
}

// ===========================================================================
// The unified generalized-Black-Scholes-Merton (gBSM) carry kernel
// ===========================================================================
//
// ONE forward-space closed form for every cost-of-carry option leaf — equity
// (`b = r − q − repo`), commodity / Black-76 (`b = r − convenience`, or `b = 0`
// on a listed future), digital-asset "linear" (`b = r − funding`), and the FX
// Garman-Kohlhagen *core* (`b = r_dom − r_for`, `r = r_dom`). Every leaf assembles
// its own `b` from its asset-class carry parameters, then prices through this one
// kernel; nothing here names an asset class.
//
// The model is the generalized-BSM in forward space (Haug, *The Complete Guide to
// Option Pricing Formulas*, 2nd ed., the one-formula/`b`-table gBSM; QuantLib's
// single `blackFormula` on the forward): with `F = S·e^{b·t}` and `df = e^{−r·t}`,
//
//   d1 = [ln(F/K) + ½σ²·t] / (σ·√t),   d2 = d1 − σ·√t
//   Call = df·[F·Φ(d1) − K·Φ(d2)],     Put = df·[K·Φ(−d2) − F·Φ(−d1)]
//
// and the full desk Greek strip in a single pass. The rate sensitivities are the
// carry-natural `(discount_rho = ∂V/∂r, carry_rho = ∂V/∂b)` pair
// ([`RateSensitivities::Carry`]); the FX leaf projects them to its two-rate basis
// via [`carry_greeks_to_greeks`]. All forward/discount arithmetic reads the carry
// seam ([`Carry::forward_factor`] / [`Carry::discount_df`]) exactly as the
// commodity leaf did, so the Black-76 leaf is byte-for-byte unchanged (ADR-0012).

/// Intermediate quantities shared by the kernel's price and Greek passes.
struct CarryAux {
    /// Outright forward `F = S·e^{b·t}`.
    f: f64,
    /// Discount factor `e^{−r·t}`.
    df: f64,
    /// Forward factor `e^{b·t}`.
    fwd_factor: f64,
    d1: f64,
    d2: f64,
    sqt: f64,
    vsqt: f64,
}

#[inline]
fn carry_aux(b: f64, r: f64, spot: f64, strike: f64, vol: f64, t: f64) -> CarryAux {
    // Read the forward/discount through the carry seam — the identical path the
    // commodity (Black-76) leaf used, so its output is byte-for-byte unchanged.
    let carry = Carry::CostOfCarry { r, b };
    let sqt = sqrt(t);
    let vsqt = vol * sqt;
    let fwd_factor = carry.forward_factor(t); // e^{b t}
    let f = spot * fwd_factor;
    let df = carry.discount_df(t); // e^{−r t}
    // Forward-space d1 = [ln(F/K) + ½σ²t]/(σ√t).
    let d1 = (ln(f / strike) + 0.5 * vol * vol * t) / vsqt;
    let d2 = d1 - vsqt;
    CarryAux {
        f,
        df,
        fwd_factor,
        d1,
        d2,
        sqt,
        vsqt,
    }
}

/// The forward-space generalized-BSM discounted premium given the OUTRIGHT forward `f`,
/// the numeraire discount factor `df`, the strike, and the pre-computed `d1`/`d2`.
///
/// This is the **single home** of the closed-form price expression
/// `df·[F·Φ(d1) − K·Φ(d2)]` (call) / `df·[K·Φ(−d2) − F·Φ(−d1)]` (put). Both the flat
/// scalar kernel ([`gbsm_carry_price`], which sources `f`/`df`/`d1`/`d2` from
/// [`carry_aux`]) and the term-structure kernel ([`curve_carry_price`], which sources
/// them from a [`DiscountCurve`] pair via [`curve_carry_aux`]) call it. Factoring the
/// expression here — rather than duplicating it — keeps the two paths bit-for-bit
/// consistent and preserves the ADR-0010 FX byte-identity invariant: the flat path passes
/// the *identical* `(f, df, d1, d2)` it computed before, so its output is unchanged
/// to the last bit (proved by `kernel_price_is_bit_identical_to_greeks_price` and the
/// `celnet-vanilla`/`celnet-parity` frozen-pin `to_bits` gates).
#[inline]
fn forward_space_price(opt: OptionType, f: f64, df: f64, strike: f64, d1: f64, d2: f64) -> f64 {
    match opt {
        OptionType::Call => df * (f * norm_cdf(d1) - strike * norm_cdf(d2)),
        OptionType::Put => df * (strike * norm_cdf(-d2) - f * norm_cdf(-d1)),
    }
}

/// Present value (premium per 1 unit of the underlying), discounted, under the
/// unified generalized-BSM forward-space kernel. `b` is the net cost of carry and
/// `r` the numeraire discount rate; the leaf assembles `b`/`r` from its asset-class
/// carry parameters. Bit-identical to [`gbsm_carry_greeks`]`(…).price` (the two
/// share [`carry_aux`] and the identical price expression).
#[must_use]
pub fn gbsm_carry_price(
    opt: OptionType,
    b: f64,
    r: f64,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
) -> f64 {
    let a = carry_aux(b, r, spot, strike, vol, t);
    forward_space_price(opt, a.f, a.df, strike, a.d1, a.d2)
}

/// Price and the full generalized Greek strip in a single pass, under the unified
/// forward-space generalized-BSM kernel.
///
/// `b` is the net cost of carry (`F = S·e^{b·t}`) and `r` the numeraire discount
/// rate (`df = e^{−r·t}`). The rate sensitivities are the carry-natural pair
/// [`RateSensitivities::Carry`] `{ discount_rho = ∂V/∂r, carry_rho = ∂V/∂b }`; a
/// two-rate leaf (FX) projects them with [`carry_greeks_to_greeks`]. See
/// [`CarryGreeks`] for the precise definition and units of each sensitivity.
#[must_use]
#[allow(clippy::similar_names)] // d1/d2, nd1/nd2 are the canonical option-pricing names
pub fn gbsm_carry_greeks(
    opt: OptionType,
    b: f64,
    r: f64,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
) -> CarryGreeks {
    let a = carry_aux(b, r, spot, strike, vol, t);
    let (f, df, fwd_factor, d1, d2, sqt, vsqt) =
        (a.f, a.df, a.fwd_factor, a.d1, a.d2, a.sqt, a.vsqt);
    let (s, k) = (spot, strike);

    let pd1 = norm_pdf(d1);
    let theta_pdf = df * f * pd1 * vol / (2.0 * sqt);

    // Evaluate Φ only for the active option type: exactly 2 transcendental norm_cdf
    // calls instead of 4. For Call: Φ(d1), Φ(d2). For Put: Φ(-d1), Φ(-d2).
    // Note: delta_forward for Put is -Φ(-d1), avoiding catastrophic cancellation in deep tails.
    let (price, delta_spot, delta_forward, theta, carry_rho, charm_first_term) = match opt {
        OptionType::Call => {
            let nd1 = norm_cdf(d1);
            let nd2 = norm_cdf(d2);
            let p = df * (f * nd1 - k * nd2);
            let ds = fwd_factor * df * nd1;
            let dfwd = nd1;
            let th = -(theta_pdf + (b - r) * s * fwd_factor * df * nd1 + r * k * df * nd2);
            let crho = t * f * df * nd1;
            let cf = (b - r) * fwd_factor * df * nd1;
            (p, ds, dfwd, th, crho, cf)
        }
        OptionType::Put => {
            let nmd1 = norm_cdf(-d1);
            let nmd2 = norm_cdf(-d2);
            let p = df * (k * nmd2 - f * nmd1);
            let ds = -fwd_factor * df * nmd1;
            let dfwd = -nmd1;
            let th = -(theta_pdf - (b - r) * s * fwd_factor * df * nmd1 - r * k * df * nmd2);
            let crho = -t * f * df * nmd1;
            let cf = -(b - r) * fwd_factor * df * nmd1;
            (p, ds, dfwd, th, crho, cf)
        }
    };

    // Symmetric across call/put. gamma = e^{2 b t}·df·φ(d1)/(F·σ√t).
    let gamma = fwd_factor * fwd_factor * df * pd1 / (f * vsqt);
    let vega = df * f * sqt * pd1;
    let vanna = -fwd_factor * df * pd1 * d2 / vol;
    let volga = vega * d1 * d2 / vol;
    let speed = -gamma / s * (d1 / vsqt + 1.0);
    let zomma = gamma * (d1 * d2 - 1.0) / vol;

    // Discount-rho ∂V/∂r at FIXED b. V = e^{−r t}·[F·Φ − K·Φ] with F = S e^{b t}
    // independent of r ⇒ ∂V/∂r = −t·V.
    let discount_rho = -t * price;

    // charm = ∂(delta_spot)/∂T. ln(F/K) = ln(S/K) + b·T ⇒
    //   ∂d1/∂T = b/(σ√T) + ½σ/√T − d1/(2T).
    let dd1_dt = b / vsqt + 0.5 * vol / sqt - d1 / (2.0 * t);
    let charm = charm_first_term + fwd_factor * df * pd1 * dd1_dt;

    // color = ∂gamma/∂T = gamma·[ (b−r) − 1/(2T) − d1·∂d1/∂T ].
    let color = gamma * ((b - r) - 1.0 / (2.0 * t) - d1 * dd1_dt);

    CarryGreeks {
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
    }
}

// ===========================================================================
// Curve-backed carry — the term-structure pricing seam (ADR-0010 §2.2, P2 Phase-1)
// ===========================================================================
//
// The flat [`Carry`] is the DEGENERATE one-pillar term structure; a bootstrapped
// `celnet_rates::curve::Curve` is the GENERAL case. Both implement the object-safe
// [`DiscountCurve`] trait (`celnet-types`), so the gBSM discounting can run off a real
// term structure through the SAME forward-space closed form the flat kernel uses.
//
// # Why a borrowed `&dyn DiscountCurve`, NOT a `Carry::Curves` variant
//
// [`Carry`] derives `Copy + PartialEq + Serialize + Deserialize` and is embedded flat
// (by value) throughout the hot seam. An `Arc<dyn DiscountCurve>` variant would strip
// every one of those derives from `Carry` — a platform-wide blast radius that also
// threatens the byte-identity contract. Homing the curve path here as a borrowed
// `&dyn DiscountCurve` pair (the trait is object-safe) instead:
//   * adds NO dependency edge and forms NO cycle — `celnet-rates` already depends on
//     `celnet-types` (where the trait and its `Curve` impl live), and `celnet-core`
//     already depends on `celnet-types`; nothing new points backwards;
//   * leaves `Carry` — and therefore `CarryInputs`, and therefore the flat streaming
//     `MarketState` — completely untouched and byte-identical (the whole gate);
//   * keeps `celnet-core` allocation- and ownership-free (its stated doctrine): the
//     OWNED `Arc<dyn DiscountCurve>` handle lives at the request/batch/surface-tier
//     call site (e.g. `celnet-server`, Phase-3 wiring), which passes `&*arc` here.
//
// # Hot-core embargo (ADR-0016 §B2)
//
// This seam is the REQUEST / BATCH / SURFACE tier ONLY. The pinned streaming core
// (`celnet-engine::MarketState`, flat `f64`) neither depends on `celnet-rates` nor names
// [`CurveCarry`]; `CarryInputs` (flat `Carry`, `Copy`) is not on the drain loop. A curve
// handle is therefore structurally unreachable from the hot path — a curve can never be
// constructed into the flat seam because `Carry` stays `Copy`/flat (pinned by
// `carry_stays_flat_for_hot_core_embargo`).

/// A curve-backed carry: a domestic (numeraire/discount) and foreign (asset/growth-leg)
/// [`DiscountCurve`] pair, borrowed for the duration of a pricing call.
///
/// The two legs generalize the FX two-rate carry ([`Carry::FxRates`]) — which is exactly
/// the two-flat-curve special case — to a full term structure. All accessors read the
/// shared [`DiscountCurve`] contract, so a flat [`Carry`] leg and a bootstrapped
/// `celnet_rates::curve::Curve` leg plug in interchangeably.
///
/// `CurveCarry` is `Copy` (a pair of thin references) and holds no owned heap handle —
/// the caller owns the `Arc<dyn DiscountCurve>` (or `&Curve`) and lends it here, keeping
/// `celnet-core` allocation-free.
#[derive(Clone, Copy)]
pub struct CurveCarry<'a> {
    /// Domestic / numeraire discount curve — supplies `DF_dom(0,t)` and the discounting.
    pub dom: &'a dyn DiscountCurve,
    /// Foreign / asset-leg discount curve — supplies `DF_for(0,t)` for the outright forward.
    pub for_: &'a dyn DiscountCurve,
}

impl<'a> CurveCarry<'a> {
    /// Borrow a domestic + foreign [`DiscountCurve`] pair as a curve-backed carry.
    #[must_use]
    pub fn new(dom: &'a dyn DiscountCurve, for_: &'a dyn DiscountCurve) -> Self {
        Self { dom, for_ }
    }

    /// Numeraire discount factor `DF_dom(0,t)` — the domestic-curve discount.
    ///
    /// In the flat-curve limit (`DF_dom = e^{−r_dom·t}`) this is byte-identical to
    /// [`Carry::discount_df`] for [`Carry::FxRates`] (both a single `e^{−r_dom·t}`).
    #[must_use]
    pub fn discount_df(&self, t: f64) -> f64 {
        self.dom.discount_factor(t)
    }

    /// Multi-curve FX outright-forward growth factor
    /// `DF_for(0,t) / DF_dom(0,t)` (multiply by spot for the forward `F`).
    ///
    /// This reproduces the [`Carry::FxRates`] sign EXACTLY: in the flat limit
    /// `DF_for/DF_dom = e^{−r_for·t}/e^{−r_dom·t} = e^{(r_dom−r_for)·t}`, i.e. the
    /// [`Carry::forward_factor`] `e^{b·t}` with `b = r_dom − r_for` — the published
    /// multi-curve FX forward (Bianchetti / Ametrano-Bianchetti). (`DF_dom/DF_for` would
    /// be the inverse-signed carry and is a bug; guarded by `curve_carry_forward_sign`.)
    #[must_use]
    pub fn forward_factor(&self, t: f64) -> f64 {
        self.for_.discount_factor(t) / self.dom.discount_factor(t)
    }

    /// Outright forward `F = spot · DF_for(0,t) / DF_dom(0,t)`.
    #[must_use]
    pub fn forward(&self, spot: f64, t: f64) -> f64 {
        spot * self.forward_factor(t)
    }
}

/// Curve-backed sibling of [`carry_aux`]: the intermediate forward-space quantities with
/// the forward `f` and discount `df` sourced from a [`CurveCarry`] pair instead of the
/// flat `(b, r)` scalars. Mirrors [`carry_aux`]'s `d1`/`d2`/`sqt`/`vsqt` arithmetic
/// verbatim so the two kernels differ only in *where the forward and discount come from*.
#[inline]
fn curve_carry_aux(curve: &CurveCarry, spot: f64, strike: f64, vol: f64, t: f64) -> CarryAux {
    let sqt = sqrt(t);
    let vsqt = vol * sqt;
    let fwd_factor = curve.forward_factor(t); // DF_for/DF_dom
    let f = spot * fwd_factor;
    let df = curve.discount_df(t); // DF_dom
    // Forward-space d1 = [ln(F/K) + ½σ²t]/(σ√t) — identical to `carry_aux`.
    let d1 = (ln(f / strike) + 0.5 * vol * vol * t) / vsqt;
    let d2 = d1 - vsqt;
    CarryAux {
        f,
        df,
        fwd_factor,
        d1,
        d2,
        sqt,
        vsqt,
    }
}

/// Present value (premium per 1 unit of the underlying) under the unified forward-space
/// generalized-BSM kernel, with the forward and discounting produced by a term-structure
/// [`CurveCarry`] pair (`F = spot·DF_for/DF_dom`, `df = DF_dom`).
///
/// This is the curve-backed overload of [`gbsm_carry_price`]: it shares the *identical*
/// closed-form price expression ([`forward_space_price`]) and `d1`/`d2` arithmetic, so a
/// [`CurveCarry`] built from two FLAT curves prices the flat [`Carry::FxRates`] case to
/// well within `1e-12` (the exp∘reciprocal round-trip is the only difference), while a
/// genuinely term-structured pair prices off the real curve. The flat scalar kernel is
/// untouched — FX byte-identity is preserved by construction.
#[must_use]
pub fn curve_carry_price(
    opt: OptionType,
    curve: &CurveCarry,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
) -> f64 {
    let a = curve_carry_aux(curve, spot, strike, vol, t);
    forward_space_price(opt, a.f, a.df, strike, a.d1, a.d2)
}

/// The repricing seam for a closed-form **exotic leg** under the risk cube.
///
/// The risk cube ([`celnet-risk-cube`]) names a booked exotic by
/// [`celnet_types::ExoticKind`] and re-prices it under scenario shocks, but must
/// not depend on the heavy `celnet-exotics` pricing crate (arch-program item E,
/// `docs/INTERFACES.md` one-way edges). This trait inverts that dependency: the
/// cube takes a `&dyn ExoticLegPricer` and the **server** implements it over the
/// concrete `celnet-exotics` engines, injecting the pricer at construction /
/// repricing time. The cube's finite-difference Greek machinery and scenario
/// reprice call only these two methods, so the exact same `celnet-exotics`
/// arithmetic runs with the exact same inputs and call order — byte-identical risk
/// output.
///
/// All methods price **one unit** of payout (per unit base for a barrier, per
/// payout unit for a digital); the leg's notional scaling is applied by the cube.
pub trait ExoticLegPricer {
    /// Price one **unit** of the closed-form exotic `kind` under the FX market
    /// state `inputs` (spot, strike, vol, time, and the two FX rates). For a
    /// [`ExoticKind::SingleBarrier`] this is the Reiner-Rubinstein barrier price;
    /// for a [`ExoticKind::Digital`] the cash-/asset-or-nothing digital price.
    fn unit_price(&self, kind: ExoticKind, inputs: &VanillaInputs) -> f64;

    /// The digital's **closed-form** `(delta, gamma, vega)` per unit payout under
    /// `inputs` — the exact published first/second-order spot Greeks + vega the cube
    /// prefers over a finite difference for a digital leg (no FD round-off). `delta`
    /// and `gamma` are `∂V/∂S`, `∂²V/∂S²`; `vega` is `∂V/∂σ` (per 1.0 absolute vol).
    fn digital_greeks(&self, kind: DigitalKind, inputs: &VanillaInputs) -> (f64, f64, f64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::CcyPair;

    fn eurusd() -> Underlying {
        Underlying::Fx(CcyPair::parse("EURUSD").unwrap())
    }

    /// The FX projection of `CarryInputs` (an `Fx` underlying under `Carry::FxRates`)
    /// reproduces `VanillaInputs::forward`/`df_dom`/`df_for` BIT-FOR-BIT — both via
    /// the generalized accessors and via the lowering to `VanillaInputs`.
    #[test]
    fn fx_carry_inputs_byte_identical() {
        for &(spot, strike, vol, t, r_dom, r_for) in &[
            (1.10, 1.25, 0.09, 0.5, 0.02, 0.01),
            (100.0, 100.0, 0.2, 1.0, 0.05, 0.0),
            (0.80, 0.95, 0.45, 3.0, -0.01, 0.06),
        ] {
            let vi = VanillaInputs::new(spot, strike, vol, t, r_dom, r_for);
            let ci = CarryInputs::new(
                spot,
                strike,
                vol,
                t,
                eurusd(),
                Carry::FxRates { r_dom, r_for },
            );
            assert_eq!(ci.forward().to_bits(), vi.forward().to_bits());
            assert_eq!(ci.discount_df().to_bits(), vi.df_dom().to_bits());

            // The lowering is a pure field copy ⇒ the lowered VanillaInputs is
            // byte-identical on every derived quantity.
            let lowered = fx_vanilla_inputs(&ci).unwrap();
            assert_eq!(lowered.forward().to_bits(), vi.forward().to_bits());
            assert_eq!(lowered.df_dom().to_bits(), vi.df_dom().to_bits());
            assert_eq!(lowered.df_for().to_bits(), vi.df_for().to_bits());
        }
    }

    /// A `CostOfCarry` carry under an FX underlying is rejected by the FX lowering
    /// — the FX leaf only prices FX, it never silently mis-prices.
    #[test]
    fn fx_lowering_rejects_cost_of_carry() {
        let ci = CarryInputs::new(
            50.0,
            55.0,
            0.2,
            1.0,
            eurusd(),
            Carry::CostOfCarry { r: 0.04, b: 0.01 },
        );
        assert_eq!(
            fx_vanilla_inputs(&ci),
            Err(CarryPriceError::UnsupportedCarry)
        );
    }

    /// `fx_carry_greeks` copies every carry-neutral field bit-for-bit and tags the
    /// rate block as `RateSensitivities::Fx` with the two rhos verbatim.
    #[test]
    fn fx_carry_greeks_lifts_byte_identically() {
        let g = Greeks {
            price: 1.234_567_89,
            delta_spot: 0.5,
            delta_forward: 0.55,
            gamma: 0.01,
            vega: 0.2,
            theta: -0.03,
            rho_dom: 0.12,
            rho_for: -0.08,
            vanna: 0.001,
            volga: 0.002,
            charm: -0.004,
            speed: 0.000_5,
            zomma: 0.000_25,
            color: -0.000_125,
        };
        let cg = fx_carry_greeks(&g);
        assert_eq!(cg.price.to_bits(), g.price.to_bits());
        assert_eq!(cg.delta_spot.to_bits(), g.delta_spot.to_bits());
        assert_eq!(cg.delta_forward.to_bits(), g.delta_forward.to_bits());
        assert_eq!(cg.gamma.to_bits(), g.gamma.to_bits());
        assert_eq!(cg.vega.to_bits(), g.vega.to_bits());
        assert_eq!(cg.theta.to_bits(), g.theta.to_bits());
        assert_eq!(cg.vanna.to_bits(), g.vanna.to_bits());
        assert_eq!(cg.volga.to_bits(), g.volga.to_bits());
        assert_eq!(cg.charm.to_bits(), g.charm.to_bits());
        assert_eq!(cg.speed.to_bits(), g.speed.to_bits());
        assert_eq!(cg.zomma.to_bits(), g.zomma.to_bits());
        assert_eq!(cg.color.to_bits(), g.color.to_bits());
        match cg.rates {
            RateSensitivities::Fx { rho_dom, rho_for } => {
                assert_eq!(rho_dom.to_bits(), g.rho_dom.to_bits());
                assert_eq!(rho_for.to_bits(), g.rho_for.to_bits());
            }
            RateSensitivities::Carry { .. } => panic!("FX greeks must tag as Fx"),
        }
    }

    // -----------------------------------------------------------------------
    // The unified gBSM carry kernel.
    // -----------------------------------------------------------------------

    /// `gbsm_carry_price` is bit-identical to `gbsm_carry_greeks(…).price` — the two
    /// public entry points share `carry_aux` and the identical price expression, so
    /// a leaf that computes its price one way and its Greek strip the other stays
    /// self-consistent to the last bit.
    #[test]
    fn kernel_price_is_bit_identical_to_greeks_price() {
        for &(b, r, s, k, vol, t) in &[
            (0.05, 0.08, 930.0, 900.0, 0.20, 1.0 / 6.0),
            (0.0, 0.05, 100.0, 110.0, 0.25, 1.0),
            (-0.01, -0.01, 50.0, 55.0, 0.45, 3.0),
            (0.03, 0.05, 30_000.0, 30_000.0, 0.65, 0.5),
        ] {
            for opt in [OptionType::Call, OptionType::Put] {
                assert_eq!(
                    gbsm_carry_price(opt, b, r, s, k, vol, t).to_bits(),
                    gbsm_carry_greeks(opt, b, r, s, k, vol, t).price.to_bits(),
                    "kernel price/greeks price must match to the bit"
                );
            }
        }
    }

    /// Model-free put-call parity on the forward: `C − P = df·(F − K)`, reached by a
    /// route disjoint from the per-leg Φ-weighted price (a sign/weight bug surfaces).
    #[test]
    fn kernel_put_call_parity() {
        for &(b, r, s, k, vol, t) in &[
            (0.05, 0.08, 930.0, 900.0, 0.20, 1.0 / 6.0),
            (-0.02, 0.03, 100.0, 95.0, 0.30, 2.0),
            (0.10, 0.04, 2_000.0, 2_200.0, 0.80, 1.0),
        ] {
            let c = gbsm_carry_price(OptionType::Call, b, r, s, k, vol, t);
            let p = gbsm_carry_price(OptionType::Put, b, r, s, k, vol, t);
            let carry = Carry::CostOfCarry { r, b };
            let df = carry.discount_df(t);
            let f = s * carry.forward_factor(t);
            assert!(crate::is_close(c - p, df * (f - k), 1e-12, 1e-12));
        }
    }

    /// INDEPENDENT oracle: the Hull "European option on an index" worked example
    /// (S=930, K=900, r=8%, q=3% ⇒ b=r−q=5%, σ=20%, T=2/12), whose full-precision
    /// generalized-BSM call/put were re-derived externally (Python `math.erf`,
    /// code-disjoint from this kernel) as 51.832_956_796_490_86 / 14.550_996_773_772_4.
    /// The kernel reproduces them to 1e-9 (the sub-1e-12 forward/spot-space rounding
    /// gap is far under this bar).
    #[test]
    fn kernel_matches_hull_index_reference() {
        let (b, r, s, k, vol, t) = (0.05, 0.08, 930.0, 900.0, 0.20, 1.0 / 6.0);
        assert!(crate::is_close(
            gbsm_carry_price(OptionType::Call, b, r, s, k, vol, t),
            51.832_956_796_490_86,
            1e-9,
            1e-9
        ));
        assert!(crate::is_close(
            gbsm_carry_price(OptionType::Put, b, r, s, k, vol, t),
            14.550_996_773_772_4,
            1e-9,
            1e-9
        ));
    }

    /// `carry_greeks_to_greeks` copies every carry-neutral field verbatim and
    /// projects the carry rate block to the FX two-rho basis via `flat_rhos`
    /// (`rho_dom = discount_rho + carry_rho`, `rho_for = −carry_rho`).
    #[test]
    fn carry_greeks_to_greeks_projects_rhos() {
        let cg = gbsm_carry_greeks(OptionType::Call, 0.01, 0.05, 1.10, 1.25, 0.09, 0.5);
        let g = carry_greeks_to_greeks(&cg);
        assert_eq!(g.price.to_bits(), cg.price.to_bits());
        assert_eq!(g.delta_spot.to_bits(), cg.delta_spot.to_bits());
        assert_eq!(g.vega.to_bits(), cg.vega.to_bits());
        assert_eq!(g.color.to_bits(), cg.color.to_bits());
        let (rho_dom, rho_for) = cg.rates.flat_rhos();
        assert_eq!(g.rho_dom.to_bits(), rho_dom.to_bits());
        assert_eq!(g.rho_for.to_bits(), rho_for.to_bits());
    }

    // -----------------------------------------------------------------------
    // Curve-backed carry (ADR-0010 §2.2, P2 Phase-1).
    // -----------------------------------------------------------------------

    /// A flat single-rate discount curve `DF(t) = e^{−rate·t}` — the degenerate
    /// one-pillar term structure. `libm::exp` matches `Carry::discount_df` bit-for-bit.
    struct FlatRateCurve {
        rate: f64,
    }
    impl DiscountCurve for FlatRateCurve {
        fn discount_factor(&self, t: f64) -> f64 {
            libm::exp(-self.rate * t)
        }
    }

    /// A genuinely term-structured discount curve `DF(t) = e^{−z(t)·t}` with a quadratic
    /// continuously-compounded zero `z(t) = a + b·t + c·t²`. Matches the independent
    /// QuantLib oracle in `tests/oracle_curve_forward.py`.
    struct PolyZeroCurve {
        a: f64,
        b: f64,
        c: f64,
    }
    impl DiscountCurve for PolyZeroCurve {
        fn discount_factor(&self, t: f64) -> f64 {
            let z = self.a + self.b * t + self.c * t * t;
            libm::exp(-z * t)
        }
    }

    /// DEGENERATE case: a [`CurveCarry`] built from two FLAT single-rate curves prices the
    /// equivalent flat [`Carry::FxRates`] case identically — discount factor byte-identical
    /// (`to_bits`), forward and price to ≤1e-12 (the only gap is `DF_for/DF_dom` vs the
    /// single `e^{b·t}`, ~1 ULP). This is the ADR-0010 no-regression bridge: the curve path
    /// collapses onto the flat path in the one-pillar limit. The asymmetric `r_dom ≠ r_for`
    /// triples also pin the forward SIGN (`DF_for/DF_dom`, not the inverse).
    #[test]
    fn curve_carry_degenerate_equals_fxrates() {
        for &(r_dom, r_for) in &[(0.05, 0.02), (0.01, 0.06), (-0.01, 0.03)] {
            let dom = FlatRateCurve { rate: r_dom };
            let for_ = FlatRateCurve { rate: r_for };
            let curve = CurveCarry::new(&dom, &for_);
            let fx = Carry::FxRates { r_dom, r_for };

            for &(spot, strike, vol, t) in &[
                (1.10, 1.25, 0.09, 0.5),
                (100.0, 100.0, 0.20, 1.0),
                (0.80, 0.95, 0.45, 3.0),
            ] {
                // Domestic discount is a single `e^{−r_dom·t}` on both paths ⇒ bit-identical.
                assert_eq!(
                    curve.discount_df(t).to_bits(),
                    fx.discount_df(t).to_bits(),
                    "curve DF_dom must equal FxRates discount_df to the bit"
                );
                // Forward `spot·DF_for/DF_dom` vs `spot·e^{(r_dom−r_for)·t}` ⇒ ≤1e-12.
                assert!(
                    crate::is_close(
                        curve.forward(spot, t),
                        spot * fx.forward_factor(t),
                        1e-12,
                        1e-12
                    ),
                    "curve forward must equal FxRates forward to 1e-12"
                );
                // Price via both kernels (flat b = r_dom − r_for, r = r_dom) ⇒ ≤1e-12.
                for opt in [OptionType::Call, OptionType::Put] {
                    let flat = gbsm_carry_price(opt, r_dom - r_for, r_dom, spot, strike, vol, t);
                    let via_curve = curve_carry_price(opt, &curve, spot, strike, vol, t);
                    assert!(
                        crate::is_close(via_curve, flat, 1e-12, 1e-12),
                        "degenerate curve price must equal flat FxRates price to 1e-12: \
                         opt={opt:?} r_dom={r_dom} r_for={r_for} curve={via_curve} flat={flat}"
                    );
                }
            }
        }
    }

    /// TERM STRUCTURE: a genuinely curved `DF(t) = e^{−z(t)·t}` pair produces the published
    /// multi-curve FX forward `F = S·DF_for(t)/DF_dom(t)` (Bianchetti / Ametrano-Bianchetti),
    /// validated to ≤1e-12 against the INDEPENDENT oracle in `tests/oracle_curve_forward.py`
    /// — QuantLib 1.42.1 discount-curve objects cross-checked against a raw-`math.exp`
    /// re-derivation (two code-disjoint routes agreeing to <1e-13), never the celnet engine
    /// reprised as its own check.
    #[test]
    fn curve_carry_forward_matches_multicurve_oracle() {
        const SPOT: f64 = 1.2345;
        // z_dom(t) = 0.030 + 0.010 t − 0.0008 t² ; z_for(t) = 0.015 − 0.004 t + 0.0003 t².
        let dom = PolyZeroCurve {
            a: 0.030,
            b: 0.010,
            c: -0.0008,
        };
        let for_ = PolyZeroCurve {
            a: 0.015,
            b: -0.004,
            c: 0.0003,
        };
        let curve = CurveCarry::new(&dom, &for_);

        // (t, oracle DF_dom, oracle DF_for, oracle F) from tests/oracle_curve_forward.py.
        for &(t, df_dom, df_for, fwd) in &[
            (
                0.25_f64,
                0.991_920_317_524_108,
                0.996_501_446_748_935_4,
                1.240_201_470_096_071_3,
            ),
            (
                1.0,
                0.961_558_378_238_269_4,
                0.988_763_605_194_998_2,
                1.269_427_523_318_568_1,
            ),
            (
                2.0,
                0.910_646_948_177_994_9,
                0.983_733_747_846_952_3,
                1.333_578_632_363_343_2,
            ),
            (
                5.0,
                0.740_818_220_681_717_9,
                0.987_577_800_493_881_4,
                1.645_700_336_025_473_6,
            ),
        ] {
            assert!(
                crate::is_close(curve.discount_df(t), df_dom, 1e-12, 1e-12),
                "DF_dom(t={t}) must match the oracle"
            );
            assert!(
                crate::is_close(curve.for_.discount_factor(t), df_for, 1e-12, 1e-12),
                "DF_for(t={t}) must match the oracle"
            );
            assert!(
                crate::is_close(curve.forward(SPOT, t), fwd, 1e-12, 1e-12),
                "multi-curve forward F(t={t}) must match the QuantLib oracle to 1e-12: \
                 got {} want {fwd}",
                curve.forward(SPOT, t)
            );
            // And the seam identity F = spot·DF_for/DF_dom holds exactly on-curve.
            assert!(crate::is_close(
                curve.forward(SPOT, t),
                SPOT * curve.for_.discount_factor(t) / curve.dom.discount_factor(t),
                1e-15,
                1e-15
            ));
        }
    }

    /// The forward SIGN is `DF_for/DF_dom` — flipping to `DF_dom/DF_for` (the inverse carry)
    /// changes an asymmetric term-structured forward by far more than 1e-12, so this pins the
    /// direction independently of the degenerate test.
    #[test]
    fn curve_carry_forward_sign() {
        let dom = PolyZeroCurve {
            a: 0.030,
            b: 0.010,
            c: -0.0008,
        };
        let for_ = PolyZeroCurve {
            a: 0.015,
            b: -0.004,
            c: 0.0003,
        };
        let curve = CurveCarry::new(&dom, &for_);
        let t = 5.0;
        let correct = 1.2345 * for_.discount_factor(t) / dom.discount_factor(t);
        let flipped = 1.2345 * dom.discount_factor(t) / for_.discount_factor(t);
        assert!(crate::is_close(
            curve.forward(1.2345, t),
            correct,
            1e-12,
            1e-12
        ));
        assert!(
            (curve.forward(1.2345, t) - flipped).abs() > 1e-3,
            "asymmetric curves must distinguish the forward direction"
        );
    }

    /// HOT-CORE EMBARGO (ADR-0016 §B2): the flat [`Carry`] seam the pinned streaming
    /// `MarketState` builds on must stay a `Copy`, flat-`f64` value — a heap term-structure
    /// handle can NEVER enter it. `Copy` (an `Arc` field is not `Copy`) plus the flat layout
    /// size together forbid it. The curve-backed path is a DISTINCT borrowed [`CurveCarry`]
    /// (`Copy` refs, no owned heap handle) confined to the request/batch tier — it cannot
    /// reach `Carry`/`MarketState` (`celnet-engine` neither names it nor depends on the curve
    /// crate). This is the structural guarantee this lane carries; the `MarketState`
    /// field-reflection / `trybuild` embargo test is ADR-0016's own (future) deliverable.
    #[test]
    fn carry_stays_flat_for_hot_core_embargo() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<Carry>();
        assert_copy::<CurveCarry<'static>>();
        // Flat two-`f64` payload + tag — no heap pointer smuggled into the carry.
        assert_eq!(
            core::mem::size_of::<Carry>(),
            3 * core::mem::size_of::<f64>()
        );
    }
}
