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
    Carry, DigitalKind, ExoticKind, Greeks, OptionType, RateSensitivities, Underlying,
    VanillaInputs,
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
    match opt {
        OptionType::Call => a.df * (a.f * norm_cdf(a.d1) - strike * norm_cdf(a.d2)),
        OptionType::Put => a.df * (strike * norm_cdf(-a.d2) - a.f * norm_cdf(-a.d1)),
    }
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
    let nd1 = norm_cdf(d1);
    let nd2 = norm_cdf(d2);
    let nmd1 = norm_cdf(-d1);
    let nmd2 = norm_cdf(-d2);

    let price = match opt {
        OptionType::Call => df * (f * nd1 - k * nd2),
        OptionType::Put => df * (k * nmd2 - f * nmd1),
    };

    // Spot delta ∂V/∂S. With F = S·e^{b t}, ∂F/∂S = e^{b t}, and V = df·BS76(F):
    //   ∂V/∂S = e^{b t}·∂V/∂F = e^{(b−r) t}·Φ(±d1).
    let delta_spot = match opt {
        OptionType::Call => fwd_factor * df * nd1,
        OptionType::Put => fwd_factor * df * (nd1 - 1.0),
    };
    // Driftless forward delta ∂V_fwd/∂F = Φ(±d1), where V_fwd = V·e^{r t} is the
    // undiscounted forward value (F·Φ(d1) − K·Φ(d2) for a call). This is the FX-desk
    // "forward delta" convention the equity/FX leaves report verbatim; the
    // commodity/crypto leaves scale it by df in their adapter to report the
    // DISCOUNTED ∂V/∂F = df·Φ(±d1) their desks use (ADR-0012). The two are the same
    // Greek of two value functions (undiscounted forward vs discounted premium).
    let delta_forward = match opt {
        OptionType::Call => nd1,
        OptionType::Put => nd1 - 1.0,
    };

    // Symmetric across call/put. gamma = e^{2 b t}·df·φ(d1)/(F·σ√t).
    let gamma = fwd_factor * fwd_factor * df * pd1 / (f * vsqt);
    let vega = df * f * sqt * pd1;
    let vanna = -fwd_factor * df * pd1 * d2 / vol;
    let volga = vega * d1 * d2 / vol;
    let speed = -gamma / s * (d1 / vsqt + 1.0);
    let zomma = gamma * (d1 * d2 - 1.0) / vol;

    // theta = −∂V/∂T. The cross-multiplied pdf identity F·φ(d1) = K·φ(d2) collapses
    // the pdf bracket to a POSITIVE term e^{−rT}·F·φ(d1)·σ/(2√T):
    //   theta_call = −[ e^{−rT}·F·φ(d1)·σ/(2√T) + (b−r)·S e^{(b−r)T}Φ(d1) + r·K e^{−rT}Φ(d2) ].
    let theta_pdf = df * f * pd1 * vol / (2.0 * sqt);
    let theta = match opt {
        OptionType::Call => -(theta_pdf + (b - r) * s * fwd_factor * df * nd1 + r * k * df * nd2),
        OptionType::Put => -(theta_pdf - (b - r) * s * fwd_factor * df * nmd1 - r * k * df * nmd2),
    };

    // Discount-rho ∂V/∂r at FIXED b. V = e^{−r t}·[F·Φ − K·Φ] with F = S e^{b t}
    // independent of r ⇒ ∂V/∂r = −t·V.
    let discount_rho = -t * price;
    // Carry-rho ∂V/∂b at FIXED r. Only F = S e^{b t} depends on b: ∂F/∂b = t·F,
    // ∂V/∂F = df·Φ(±d1) ⇒ ∂V/∂b = t·F·df·Φ(±d1).
    let carry_rho = match opt {
        OptionType::Call => t * f * df * nd1,
        OptionType::Put => -t * f * df * nmd1,
    };

    // charm = ∂(delta_spot)/∂T. ln(F/K) = ln(S/K) + b·T ⇒
    //   ∂d1/∂T = b/(σ√T) + ½σ/√T − d1/(2T).
    let dd1_dt = b / vsqt + 0.5 * vol / sqt - d1 / (2.0 * t);
    let charm = match opt {
        OptionType::Call => (b - r) * fwd_factor * df * nd1 + fwd_factor * df * pd1 * dd1_dt,
        OptionType::Put => (b - r) * fwd_factor * df * (nd1 - 1.0) + fwd_factor * df * pd1 * dd1_dt,
    };

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
}
