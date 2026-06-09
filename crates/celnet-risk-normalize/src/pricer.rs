//! The cross-asset pricing dispatcher behind the agnostic [`CarryPricer`] seam.
//!
//! This is the **one** place an asset class is resolved into a concrete pricing
//! leaf — and it is *not* on the per-position hot path of any reducer. The risk
//! reducers in [`crate::leaf`] (and the cube above) hold a `&P: CarryPricer` and
//! call `pricer.price_greeks(opt, &inputs)` blindly; the discriminant on
//! [`celnet_types::Underlying`] / [`celnet_types::Carry`] is read *inside the leaf
//! adapter's* own "is this mine?" guard, which is the location ADR-0008 explicitly
//! sanctions (the FX leaf's [`celnet_core::fx_vanilla_inputs`] already does exactly
//! this). No `match underlying { Fx => …, Equity => … }` appears in any
//! aggregation/reduction loop.
//!
//! # Dispatch model
//!
//! [`AssetPricer`] owns a fixed, ordered table of leaf adapters
//! (`&'static [&'static dyn CarryPricer]`). For each price call it tries each leaf
//! in order and returns the first `Ok`; a leaf that does not own the input returns
//! [`CarryPriceError::UnsupportedUnderlying`] / [`CarryPriceError::UnsupportedCarry`].
//! The dispatcher owns *ordering*, each leaf owns *rejection*, and the reducer owns
//! *nothing*. If no leaf accepts the input the dispatcher surfaces the typed error —
//! never a silent FX-proxy mis-price (the W1 no-silent-fallback rule).
//!
//! # Leaf ordering (which leaf prices a generic cost-of-carry)
//!
//! The adapters are ordered so the resolution is unambiguous and lossless:
//!
//! 1. [`FxLeaf`] — claims `Underlying::Fx` / `Underlying::Metal` under
//!    `Carry::FxRates` (the FX two-rate Garman-Kohlhagen arithmetic, byte-identical
//!    to the direct FX path).
//! 2. [`CryptoLeaf`] — claims `Underlying::DigitalAsset` (the linear coin payoff on
//!    the funding carry); its *risk tagging* is the crypto desk's, so it is selected
//!    only when the underlying names it.
//! 3. [`EquityLeaf`] — claims `Underlying::Equity` (generalized-BSM with a dividend
//!    yield); its dividend-rho tagging is the equity desk's, so it too is selected
//!    only by the underlying arm.
//! 4. [`CommodityLeaf`] — claims `Underlying::Commodity`, and is the **canonical
//!    generic `Carry::CostOfCarry` pricer**: it takes the [`celnet_types::Carry`]
//!    directly without re-deriving a dividend `q`, so it is the cleanest lowering
//!    for any fiat-quoted cost-of-carry asset. It is ordered last so the
//!    asset-tagged equity/crypto arms win their own underlyings first.
//!
//! Every adapter is a zero-sized handle, so the table is a compile-time constant
//! and the dispatch allocates nothing.

use celnet_core::{CarryGreeks, CarryInputs, CarryPriceError, CarryPricer};
use celnet_types::{Carry, OptionType};

/// The FX (Garman-Kohlhagen) leaf adapter.
///
/// Delegates to `celnet-vanilla`'s [`celnet_vanilla::FxPricer`], which lowers an FX
/// `CarryInputs` to `VanillaInputs`, prices with the unchanged FX arithmetic, and
/// lifts the result back to [`CarryGreeks`] tagged `RateSensitivities::Fx`. Rejects
/// any non-FX underlying / non-`FxRates` carry with the typed error — the FX leaf
/// never mis-prices another asset class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FxLeaf;

impl CarryPricer for FxLeaf {
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> Result<f64, CarryPriceError> {
        celnet_vanilla::FxPricer.price(opt, inputs)
    }

    fn price_greeks(
        &self,
        opt: OptionType,
        inputs: &CarryInputs,
    ) -> Result<CarryGreeks, CarryPriceError> {
        celnet_vanilla::FxPricer.price_greeks(opt, inputs)
    }
}

/// The equity (generalized-BSM, dividend yield) leaf adapter.
///
/// Claims `Underlying::Equity` only. The lowering maps the generic carry to the
/// equity leaf's `(r, q, repo)` parameterization losslessly: `r := discount_rate`,
/// `q := r − b`, `repo := 0`, so the equity leaf reconstructs `b = r − q − 0 = b`
/// and the discount `e^{−rt}` exactly. The equity leaf's `EquityGreeks` already
/// carry `RateSensitivities::Carry { discount_rho, carry_rho }`, so the lift to
/// [`CarryGreeks`] is a field copy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EquityLeaf;

impl EquityLeaf {
    /// Lower a generic `CarryInputs` to the equity leaf's input, or reject.
    fn lower(inputs: &CarryInputs) -> Result<celnet_equity_vanilla::EquityInputs, CarryPriceError> {
        if inputs.underlying.as_equity().is_none() {
            return Err(CarryPriceError::UnsupportedUnderlying);
        }
        let (r, b) = match inputs.carry {
            Carry::CostOfCarry { r, b } => (r, b),
            // An equity underlying must carry a generic cost-of-carry; an FX
            // two-rate carry under an equity underlying is a malformed input.
            Carry::FxRates { .. } => return Err(CarryPriceError::UnsupportedCarry),
        };
        Ok(celnet_equity_vanilla::EquityInputs::new(
            inputs.spot,
            inputs.strike,
            inputs.vol,
            inputs.t,
            r,
            // q := r − b ⇒ b = r − q − repo with repo = 0.
            r - b,
            0.0,
        ))
    }
}

impl CarryPricer for EquityLeaf {
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> Result<f64, CarryPriceError> {
        Ok(celnet_equity_vanilla::price(opt, &Self::lower(inputs)?))
    }

    fn price_greeks(
        &self,
        opt: OptionType,
        inputs: &CarryInputs,
    ) -> Result<CarryGreeks, CarryPriceError> {
        let g = celnet_equity_vanilla::greeks(opt, &Self::lower(inputs)?);
        Ok(CarryGreeks {
            price: g.price,
            delta_spot: g.delta_spot,
            delta_forward: g.delta_forward,
            gamma: g.gamma,
            vega: g.vega,
            theta: g.theta,
            rates: g.rates,
            vanna: g.vanna,
            volga: g.volga,
            charm: g.charm,
            speed: g.speed,
            zomma: g.zomma,
            color: g.color,
        })
    }
}

/// The commodity (Black-76 over a generic cost-of-carry) leaf adapter — the
/// **canonical generic `Carry::CostOfCarry` pricer**.
///
/// Claims `Underlying::Commodity`. The commodity leaf takes a [`celnet_types::Carry`]
/// directly (no `q` re-derivation), and its `greeks` already return
/// [`CarryGreeks`], so the lowering and lift are both lossless field passes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommodityLeaf;

impl CommodityLeaf {
    fn lower(
        inputs: &CarryInputs,
    ) -> Result<celnet_commodity_vanilla::CommodityInputs, CarryPriceError> {
        if inputs.underlying.as_commodity().is_none() {
            return Err(CarryPriceError::UnsupportedUnderlying);
        }
        match inputs.carry {
            Carry::CostOfCarry { .. } => {}
            Carry::FxRates { .. } => return Err(CarryPriceError::UnsupportedCarry),
        }
        Ok(celnet_commodity_vanilla::CommodityInputs::new(
            inputs.spot,
            inputs.strike,
            inputs.vol,
            inputs.t,
            inputs.carry,
        ))
    }
}

impl CarryPricer for CommodityLeaf {
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> Result<f64, CarryPriceError> {
        Ok(celnet_commodity_vanilla::price(opt, &Self::lower(inputs)?))
    }

    fn price_greeks(
        &self,
        opt: OptionType,
        inputs: &CarryInputs,
    ) -> Result<CarryGreeks, CarryPriceError> {
        Ok(celnet_commodity_vanilla::greeks(opt, &Self::lower(inputs)?))
    }
}

/// The crypto **linear** (USD-margined coin payoff) leaf adapter.
///
/// Claims `Underlying::DigitalAsset` under a generic cost-of-carry (the funding
/// carry `b = r − funding`). Only the *linear* arm participates in the additive
/// Greek seam — the inverse (`1/S_T`) coin-margined payoff settles in the base coin
/// and its cross-asset numeraire collapse is a deliberately deferred follow-up
/// (see the crate-level honest-scope note); routing inverse risk through this
/// linear leaf would mis-state the settlement leg, so it is not done here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CryptoLeaf;

impl CryptoLeaf {
    fn lower(inputs: &CarryInputs) -> Result<celnet_crypto_vanilla::LinearInputs, CarryPriceError> {
        if inputs.underlying.as_digital_asset().is_none() {
            return Err(CarryPriceError::UnsupportedUnderlying);
        }
        match inputs.carry {
            Carry::CostOfCarry { .. } => {}
            Carry::FxRates { .. } => return Err(CarryPriceError::UnsupportedCarry),
        }
        Ok(celnet_crypto_vanilla::LinearInputs::new(
            inputs.spot,
            inputs.strike,
            inputs.vol,
            inputs.t,
            inputs.carry,
        ))
    }
}

impl CarryPricer for CryptoLeaf {
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> Result<f64, CarryPriceError> {
        Ok(celnet_crypto_vanilla::linear::price(
            opt,
            &Self::lower(inputs)?,
        ))
    }

    fn price_greeks(
        &self,
        opt: OptionType,
        inputs: &CarryInputs,
    ) -> Result<CarryGreeks, CarryPriceError> {
        Ok(celnet_crypto_vanilla::linear::greeks(
            opt,
            &Self::lower(inputs)?,
        ))
    }
}

/// The default cross-asset dispatcher. It owns no state; it implements the agnostic
/// [`CarryPricer`] seam by delegating to whichever leaf supports the [`CarryInputs`].
///
/// The asset-class resolution happens HERE, once per price call, behind the seam —
/// the risk reducers never see it (ADR-0008 no-hot-path-match: the reducers hold a
/// `&dyn CarryPricer`/`&P` and the *match is the leaf's*, not the aggregation
/// loop's). The dispatch table is a compile-time constant; the dispatch is a single
/// linear scan over a handful of zero-sized handles and allocates nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AssetPricer;

/// The ordered leaf table (see the module docs for the ordering rationale). Each
/// entry is a reference to a zero-sized adapter, so the array is a compile-time
/// constant built per call with no allocation. (`&'static [&dyn CarryPricer]` is not
/// `Sync` — the trait carries no `Sync` bound — so the table is a local const array
/// rather than a `static`; both are equally zero-cost.)
const LEAVES: [&dyn CarryPricer; 4] = [&FxLeaf, &CryptoLeaf, &EquityLeaf, &CommodityLeaf];

impl CarryPricer for AssetPricer {
    fn price(&self, opt: OptionType, inputs: &CarryInputs) -> Result<f64, CarryPriceError> {
        dispatch(inputs, |leaf| leaf.price(opt, inputs))
    }

    fn price_greeks(
        &self,
        opt: OptionType,
        inputs: &CarryInputs,
    ) -> Result<CarryGreeks, CarryPriceError> {
        dispatch(inputs, |leaf| leaf.price_greeks(opt, inputs))
    }
}

/// Try each leaf in table order, returning the first `Ok`. If every leaf rejects
/// the input, surface the most specific typed error: an `UnsupportedCarry` (a known
/// underlying with the wrong carry shape) is reported in preference to a generic
/// `UnsupportedUnderlying`, so a malformed carry is never masked as an unknown
/// asset class. Never returns a silent fallback price.
fn dispatch<T>(
    inputs: &CarryInputs,
    mut call: impl FnMut(&dyn CarryPricer) -> Result<T, CarryPriceError>,
) -> Result<T, CarryPriceError> {
    let mut err = CarryPriceError::UnsupportedUnderlying;
    for &leaf in &LEAVES {
        match call(leaf) {
            Ok(v) => return Ok(v),
            // A leaf that recognizes the underlying but rejects the carry is the
            // most informative failure — keep it over a generic "no leaf owns this".
            Err(e @ CarryPriceError::UnsupportedCarry) => err = e,
            Err(CarryPriceError::UnsupportedUnderlying) => {}
        }
    }
    let _ = inputs; // the discriminant is read by the leaves, never here.
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::{fx_carry_greeks, is_close};
    use celnet_types::{
        Ccy, CcyPair, CommodityRef, CryptoPair, EquityRef, Metal, MetalPair, Symbol, Underlying,
    };

    fn fx_inputs() -> CarryInputs {
        CarryInputs::new(
            1.10,
            1.12,
            0.10,
            0.5,
            Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD)),
            Carry::FxRates {
                r_dom: 0.04,
                r_for: 0.02,
            },
        )
    }

    fn metal_inputs() -> CarryInputs {
        CarryInputs::new(
            2_000.0,
            2_050.0,
            0.15,
            1.0,
            Underlying::Metal(MetalPair::new(Metal::Gold, Ccy::USD)),
            Carry::FxRates {
                r_dom: 0.05,
                r_for: 0.01,
            },
        )
    }

    fn equity_inputs() -> CarryInputs {
        CarryInputs::new(
            100.0,
            105.0,
            0.20,
            1.0,
            Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD)),
            Carry::CostOfCarry { r: 0.03, b: 0.01 },
        )
    }

    fn commodity_inputs() -> CarryInputs {
        CarryInputs::new(
            80.0,
            85.0,
            0.30,
            0.75,
            Underlying::Commodity(CommodityRef::new(Symbol::new("BRENT", ""), Ccy::USD)),
            Carry::CostOfCarry { r: 0.04, b: 0.02 },
        )
    }

    fn crypto_inputs() -> CarryInputs {
        CarryInputs::new(
            60_000.0,
            65_000.0,
            0.65,
            0.25,
            Underlying::DigitalAsset(CryptoPair::new("BTC", "USD")),
            Carry::CostOfCarry { r: 0.05, b: 0.02 },
        )
    }

    /// G1: each leaf adapter prices its own asset and rejects every other with the
    /// typed error; `AssetPricer` resolves each input to exactly one accepting leaf
    /// and returns its price. The dispatch never silently falls back to FX.
    #[test]
    fn adapters_dispatch_by_seam() {
        let fx = fx_inputs();
        let metal = metal_inputs();
        let eq = equity_inputs();
        let com = commodity_inputs();
        let cr = crypto_inputs();
        let opt = OptionType::Call;

        // FX leaf: prices FX + metal, rejects the cost-of-carry asset classes.
        assert!(FxLeaf.price(opt, &fx).is_ok());
        assert!(FxLeaf.price(opt, &metal).is_ok());
        assert_eq!(
            FxLeaf.price(opt, &eq),
            Err(CarryPriceError::UnsupportedUnderlying)
        );
        assert_eq!(
            FxLeaf.price(opt, &com),
            Err(CarryPriceError::UnsupportedUnderlying)
        );
        assert_eq!(
            FxLeaf.price(opt, &cr),
            Err(CarryPriceError::UnsupportedUnderlying)
        );

        // Equity leaf: prices equity only.
        assert!(EquityLeaf.price(opt, &eq).is_ok());
        assert_eq!(
            EquityLeaf.price(opt, &fx),
            Err(CarryPriceError::UnsupportedUnderlying)
        );
        assert_eq!(
            EquityLeaf.price(opt, &com),
            Err(CarryPriceError::UnsupportedUnderlying)
        );
        assert_eq!(
            EquityLeaf.price(opt, &cr),
            Err(CarryPriceError::UnsupportedUnderlying)
        );

        // Commodity leaf: prices commodity only.
        assert!(CommodityLeaf.price(opt, &com).is_ok());
        assert_eq!(
            CommodityLeaf.price(opt, &eq),
            Err(CarryPriceError::UnsupportedUnderlying)
        );
        assert_eq!(
            CommodityLeaf.price(opt, &fx),
            Err(CarryPriceError::UnsupportedUnderlying)
        );

        // Crypto leaf: prices digital-asset only.
        assert!(CryptoLeaf.price(opt, &cr).is_ok());
        assert_eq!(
            CryptoLeaf.price(opt, &eq),
            Err(CarryPriceError::UnsupportedUnderlying)
        );

        // The dispatcher resolves each to exactly one accepting leaf, and the
        // resolved price equals that leaf's own price (no FX proxy).
        let cases: [(CarryInputs, &dyn CarryPricer); 5] = [
            (fx.clone(), &FxLeaf),
            (metal.clone(), &FxLeaf),
            (eq.clone(), &EquityLeaf),
            (com.clone(), &CommodityLeaf),
            (cr.clone(), &CryptoLeaf),
        ];
        for (ci, leaf) in cases {
            let via_dispatch = AssetPricer.price(opt, &ci).unwrap();
            let via_leaf = leaf.price(opt, &ci).unwrap();
            assert_eq!(
                via_dispatch.to_bits(),
                via_leaf.to_bits(),
                "dispatch must return the owning leaf's exact price for {:?}",
                ci.underlying
            );
        }
    }

    /// FX dispatch is byte-identical to the direct `celnet-vanilla` FX path — the
    /// generalization moves no FX ULP.
    #[test]
    fn fx_dispatch_is_byte_identical_to_direct_fx() {
        let ci = fx_inputs();
        let opt = OptionType::Put;
        let vi = celnet_core::fx_vanilla_inputs(&ci).unwrap();
        let direct = celnet_vanilla::price(opt, &vi);
        let dispatched = AssetPricer.price(opt, &ci).unwrap();
        assert_eq!(dispatched.to_bits(), direct.to_bits());

        let dg = AssetPricer.price_greeks(opt, &ci).unwrap();
        let direct_g = fx_carry_greeks(&celnet_vanilla::greeks(opt, &vi));
        assert_eq!(dg.price.to_bits(), direct_g.price.to_bits());
        assert_eq!(dg.delta_spot.to_bits(), direct_g.delta_spot.to_bits());
        assert_eq!(dg.vega.to_bits(), direct_g.vega.to_bits());
        assert_eq!(dg.gamma.to_bits(), direct_g.gamma.to_bits());
    }

    /// The equity lowering round-trips the generic carry losslessly: the dispatched
    /// equity greeks equal the equity leaf called on the directly-lowered input.
    #[test]
    fn equity_lowering_is_lossless() {
        let ci = equity_inputs();
        let opt = OptionType::Call;
        let lowered = EquityLeaf::lower(&ci).unwrap();
        // b = r − q − repo must reproduce the generic carry's b.
        if let Carry::CostOfCarry { r, b } = ci.carry {
            assert!(is_close(lowered.r, r, 0.0, 1e-15));
            assert!(is_close(lowered.carry(), b, 0.0, 1e-15));
        }
        let direct = celnet_equity_vanilla::greeks(opt, &lowered);
        let dispatched = AssetPricer.price_greeks(opt, &ci).unwrap();
        assert_eq!(dispatched.price.to_bits(), direct.price.to_bits());
        assert_eq!(dispatched.delta_spot.to_bits(), direct.delta_spot.to_bits());
        match (dispatched.rates, direct.rates) {
            (
                celnet_types::RateSensitivities::Carry {
                    discount_rho: a,
                    carry_rho: b,
                },
                celnet_types::RateSensitivities::Carry {
                    discount_rho: c,
                    carry_rho: d,
                },
            ) => {
                assert_eq!(a.to_bits(), c.to_bits());
                assert_eq!(b.to_bits(), d.to_bits());
            }
            _ => panic!("equity must tag RateSensitivities::Carry"),
        }
    }

    /// A known underlying with the wrong carry shape surfaces `UnsupportedCarry`
    /// (the informative error), not a generic `UnsupportedUnderlying` — and never a
    /// silent FX price.
    #[test]
    fn malformed_carry_surfaces_unsupported_carry() {
        // Equity underlying mistakenly carrying FX two-rate carry.
        let bad = CarryInputs::new(
            100.0,
            105.0,
            0.2,
            1.0,
            Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD)),
            Carry::FxRates {
                r_dom: 0.03,
                r_for: 0.01,
            },
        );
        assert_eq!(
            AssetPricer.price(OptionType::Call, &bad),
            Err(CarryPriceError::UnsupportedCarry)
        );
    }
}
