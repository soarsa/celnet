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

use celnet_types::{Carry, Greeks, OptionType, RateSensitivities, Underlying, VanillaInputs};

/// Generalized, carry-tagged pricing input.
///
/// The market state (`spot`, `strike`, `vol`, `t`) shared by every asset class,
/// plus the [`Underlying`] discriminator (*what* is priced) and the [`Carry`]
/// model (*how* the forward and discounting are formed). The forward and discount
/// factor are delegated to [`Carry`], so the FX arm reproduces the FX two-rate
/// arithmetic bit-for-bit (see [`Carry::FxRates`]).
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// Lower an FX [`CarryInputs`] (an `Fx` underlying carried by [`Carry::FxRates`])
/// to the FX leaf's [`VanillaInputs`].
///
/// Returns [`CarryPriceError`] for a non-FX underlying or a non-`FxRates` carry, so
/// an FX leaf can reject what it is not the correct pricer for. The mapping is a
/// pure field copy of `(r_dom, r_for)` — so the resulting `forward`/`df_dom`/
/// `df_for` are byte-identical to the generalized [`CarryInputs::forward`]/
/// [`CarryInputs::discount_df`] (proved in [`fx_carry_inputs_byte_identical`]).
///
/// # Errors
/// Returns [`CarryPriceError::UnsupportedUnderlying`] for a non-FX underlying and
/// [`CarryPriceError::UnsupportedCarry`] for a non-`FxRates` carry.
pub fn fx_vanilla_inputs(inputs: &CarryInputs) -> Result<VanillaInputs, CarryPriceError> {
    match inputs.underlying {
        Underlying::Fx(_) => {}
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
}
