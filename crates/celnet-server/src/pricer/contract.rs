//! The central pricing contract, re-seated onto the FX options leaf (ADR-0017
//! **Phase A1**).
//!
//! This module realises the `celnet_core` contract ([`celnet_core::contract`]) on
//! the paradigm that already fits it — the FX (Garman-Kohlhagen) vanilla path —
//! and proves the re-seat carries **zero numeric drift**:
//!
//! * [`FxSurfaceResolver`] wraps a resolved (marked-surface-pinned) market context
//!   as a [`MarketResolver`], producing a [`ResolvedMarket`] handle.
//! * [`VanillaEngine`] and its plugin-host sibling [`PluginModelEngine`] implement
//!   [`Priceable`] — the **leaf-level** re-seat (critique F2): the entire
//!   pre-dispatch guard/selector cascade in [`super::price_instrument`] (perpetual
//!   term-shape, asset-class route, FX two-rate carry guard, LSV booking-model
//!   selector, plugin-host `dispatch_live`) is preserved verbatim; only the leaf
//!   gains the contract seam, so the FX numbers are byte-for-byte unchanged.
//!
//! # FX byte-identity — why the two rates come from the request context
//!
//! The FX outright forward is `F = S·e^{(r_dom−r_for)·t}` — a **single** `exp`,
//! the ADR-0010/0012 byte-identity anchor pinned by the parity + frozen-pin
//! suites. Re-deriving it from the [`ResolvedMarket`]'s discount/foreign
//! `DiscountCurve` legs as the ratio `DF_for/DF_dom` is the *curve* path, which is
//! equal only to ≤1e-12 (two `exp`s + a divide), **not** `to_bits`. So the FX
//! two-rate leaf reads its rates + resolved scalar vol from the request context
//! (the frozen-pin arithmetic) and consumes the [`ResolvedMarket`] for the F4
//! delta-key solver (its conventions) and the resolved spot; the discount/foreign
//! curve legs are the unified handle the linear-FI / cross-asset (Phase A2 / B)
//! leaves consume. Byte-identity is therefore preserved by construction — the leaf
//! calls the *identical* [`super::resolve_strike`] + [`super::price_vanilla_leg`].

use celnet_core::carry::CarryGreeks;
use celnet_core::contract::{
    FlatDiscountCurve, MarketResolver, Priceable, ResolvedMarket, RiskMeasure,
};
use celnet_core::fx_carry_greeks;
use celnet_proto::{Instrument, MarketContext as WireMarketContext, instrument};
use celnet_types::{Greeks, RateSensitivities};

use celnet_commodity_vanilla::CommodityInputs;
use celnet_crypto_vanilla::{
    InverseInputs as CryptoInverseInputs, LinearInputs as CryptoLinearInputs,
    inverse as crypto_inverse, linear as crypto_linear,
};
use celnet_equity_vanilla::EquityInputs;

use super::engines::{
    AccumulatorEngine, AmericanEngine, AsianOptionEngine, BasketEngine, CliquetEngine,
    DigitalEngine, DoubleBarrierEngine, EngineCtx, ForwardStartEngine, FxForwardEngine,
    FxSwapEngine, ListedFutureOptionEngine, LookbackEngine, NdfEngine, PerpetualOptionEngine,
    PivotEngine, PluginModelEngine, ProductEngine, QuantoEngine, SingleBarrierEngine,
    StrategyEngine, TarfEngine, TouchEngine, VanillaEngine, VarianceSwapEngine,
    VolatilitySwapEngine, WindowBarrierEngine,
};
use super::{
    ConventionSet, PriceError, Priced, carry_greeks_to_greeks, cost_of_carry,
    cross_asset_vanilla_terms, decode_option_type, price_listed_future_option, price_perpetual,
    price_vanilla_leg, resolve_strike,
};

/// Extract the vanilla product the FX vanilla leaf prices from the request
/// instrument, or a typed error if the instrument does not carry one.
fn expect_vanilla(instrument: &Instrument) -> Result<&celnet_proto::Vanilla, PriceError> {
    match instrument.product.as_ref() {
        Some(instrument::Product::Vanilla(v)) => Ok(v),
        Some(_) => Err(PriceError::Domain(
            "the vanilla Priceable leaf prices only the vanilla product",
        )),
        None => Err(PriceError::EmptyProduct),
    }
}

/// A request-tier [`MarketResolver`] that wraps a resolved FX market context — the
/// output of the marked-surface pin (`crate::services::pin::resolve_pinned_vol`
/// over the [`crate::surface_book::SurfaceBook`] snapshot) — as a
/// [`ResolvedMarket`] handle.
///
/// It captures the resolution result once (the flat two-rate discount legs, the
/// resolved spot + scalar vol, and the conventions) and lends it as a
/// [`ResolvedMarket`] on [`MarketResolver::resolve`] — the standard lending
/// pattern, so celnet-core need never own the request's per-call curves.
pub(super) struct FxSurfaceResolver {
    /// Domestic (numeraire) leg `DF_dom(t) = e^{−r_dom·t}`.
    dom: FlatDiscountCurve,
    /// Foreign (asset) leg `DF_for(t) = e^{−r_for·t}`.
    for_: FlatDiscountCurve,
    /// The resolved scalar Black vol the request-tier price uses.
    vol: f64,
    /// The resolved spot.
    spot: f64,
    /// The resolved trade conventions (carries the delta-key solver, F4).
    conventions: ConventionSet,
}

impl FxSurfaceResolver {
    /// Capture a resolved FX market context as an FX market resolver.
    ///
    /// `market` is the market the pricer consumes: for a pinned request the edge's
    /// `resolve_pinned_vol` has already stamped the marked-surface vol onto it
    /// (`market.vol`); for an unpinned request it is the live context. The two flat
    /// discount legs are the degenerate one-pillar curves at the FX two rates.
    pub(super) fn from_market(market: &WireMarketContext, conv: &ConventionSet) -> Self {
        Self {
            dom: FlatDiscountCurve::new(market.r_dom()),
            for_: FlatDiscountCurve::new(market.r_for()),
            vol: market.vol,
            spot: market.spot,
            conventions: *conv,
        }
    }
}

impl MarketResolver for FxSurfaceResolver {
    type Request = ();
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Error = PriceError;

    fn resolve(&self, _request: &()) -> Result<ResolvedMarket<'_, ConventionSet>, PriceError> {
        Ok(ResolvedMarket::new(
            &self.dom,
            Some(&self.for_),
            Some(self.vol),
            Some(self.spot),
            &self.conventions,
        ))
    }
}

impl Priceable for VanillaEngine {
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Ctx<'a> = EngineCtx<'a>;
    type Priced = Priced;
    type Error = PriceError;

    fn price(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let v = expect_vanilla(ctx.instrument)?;
        let option_type = decode_option_type(v.option_type)?;
        let spec = v
            .strike
            .as_ref()
            .and_then(|s| s.spec.as_ref())
            .ok_or(PriceError::MissingField("vanilla.strike"))?;
        // F4: the delta-key solver reads the conventions the ResolvedMarket carries
        // (identical to `ctx.conv`, so byte-identical to the dispatch route).
        let strike = resolve_strike(
            spec,
            ctx.market,
            ctx.expiry,
            market.conventions,
            option_type,
        )?;
        // FX two-rate byte-identity: the forward + scalar vol come from the request
        // context (the single-exp frozen-pin arithmetic), NOT the DF-ratio curve
        // path. Identical call to the `ProductEngine` dispatch route.
        Ok(price_vanilla_leg(
            option_type,
            strike,
            ctx.market,
            ctx.expiry,
        ))
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<RiskMeasure, PriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        // The FX Greek strip lifted to the unified carry-tagged measure (the two FX
        // rhos verbatim, `RateSensitivities::Fx`).
        Ok(RiskMeasure::OptionGreeks(fx_carry_greeks(&priced.greeks)))
    }
}

impl Priceable for PluginModelEngine<'_> {
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Ctx<'a> = EngineCtx<'a>;
    type Priced = Priced;
    type Error = PriceError;

    fn price(
        &self,
        _market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let v = expect_vanilla(ctx.instrument)?;
        // Delegate to the established plugin body: the conventions flow identically
        // (`ctx.conv` == the ResolvedMarket's conventions), so the re-seat is
        // byte-identical to the plugin dispatch route.
        ProductEngine::price(self, v, ctx)
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<RiskMeasure, PriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::OptionGreeks(fx_carry_greeks(&priced.greeks)))
    }
}

// ===========================================================================
// Cross-asset (equity / commodity / digital-asset) leaves — ADR-0017 Phase A2
// ===========================================================================
//
// The cross-asset companion of the A1 FX re-seat above: each `super::price_cross_asset`
// match arm (equity / commodity / digital-asset linear+inverse, plus the perpetual
// and listed-future sub-cases) is reified into a `Priceable` leaf engine wrapping the
// SAME leaf call it does today. Byte-identity is preserved by construction — every
// engine sources the net carry from the request context via `super::cost_of_carry`
// (the frozen cost-of-carry arithmetic) and the resolved scalar spot + vol from the
// `ResolvedMarket`; the discount/spot/vol scalars are the verbatim request scalars the
// resolver captured, so each leaf's price/greeks is `to_bits`-identical to the
// established `price_cross_asset` arm (gated by `tests::cross_asset_reseat_is_byte_identical`).

/// Extract the perpetual product the cross-asset perpetual leaf prices, or a typed
/// error if the instrument does not carry one.
fn expect_perpetual(instrument: &Instrument) -> Result<&celnet_proto::PerpetualOption, PriceError> {
    match instrument.product.as_ref() {
        Some(instrument::Product::PerpetualOption(p)) => Ok(p),
        Some(_) => Err(PriceError::Domain(
            "the perpetual Priceable leaf prices only the perpetual option product",
        )),
        None => Err(PriceError::EmptyProduct),
    }
}

/// Extract the listed-future option the cross-asset listed-future leaf prices, or a
/// typed error if the instrument does not carry one.
fn expect_listed_future(
    instrument: &Instrument,
) -> Result<&celnet_proto::ListedFutureOption, PriceError> {
    match instrument.product.as_ref() {
        Some(instrument::Product::ListedFutureOption(o)) => Ok(o),
        Some(_) => Err(PriceError::Domain(
            "the listed-future Priceable leaf prices only the listed-future option product",
        )),
        None => Err(PriceError::EmptyProduct),
    }
}

/// The resolved scalar spot + vol a cross-asset leaf prices off, read from the
/// [`ResolvedMarket`] the [`CrossAssetCarryResolver`] produced. Both are the
/// verbatim request-context scalars the resolver captured (`market.spot` /
/// `market.vol`), so the leaf stays byte-identical to [`super::price_cross_asset`].
/// A pure-rates market (no spot / vol) is refused — unreachable on the cross-asset
/// path, total by construction.
fn resolved_spot_vol(market: &ResolvedMarket<'_, ConventionSet>) -> Result<(f64, f64), PriceError> {
    let spot = market.spot.ok_or(PriceError::Domain(
        "cross-asset resolved market carries no spot",
    ))?;
    let vol = market.vol.ok_or(PriceError::Domain(
        "cross-asset resolved market carries no vol",
    ))?;
    Ok((spot, vol))
}

/// Lift a cross-asset leaf's flat wire [`Greeks`] into the unified carry-tagged
/// [`CarryGreeks`] risk strip for [`RiskMeasure::OptionGreeks`]. Carry-neutral
/// fields copy bit-for-bit; the rate block is re-expressed in the carry-natural
/// `(discount_rho, carry_rho)` coordinates by the documented bijection — the exact
/// inverse of [`super::carry_greeks_to_greeks`]: `discount_rho = rho_dom + rho_for`,
/// `carry_rho = −rho_for`. (This is the risk *view*; the price/greeks path — the
/// byte-identity gate — is untouched.)
fn carry_option_greeks(g: &Greeks) -> CarryGreeks {
    CarryGreeks {
        price: g.price,
        delta_spot: g.delta_spot,
        delta_forward: g.delta_forward,
        gamma: g.gamma,
        vega: g.vega,
        theta: g.theta,
        rates: RateSensitivities::Carry {
            discount_rho: g.rho_dom + g.rho_for,
            carry_rho: -g.rho_for,
        },
        vanna: g.vanna,
        volga: g.volga,
        charm: g.charm,
        speed: g.speed,
        zomma: g.zomma,
        color: g.color,
    }
}

/// A request-tier [`MarketResolver`] that wraps a resolved cross-asset (equity /
/// commodity / digital-asset) cost-of-carry market as a [`ResolvedMarket`] handle
/// (ADR-0017 Phase A2) — the cross-asset companion of [`FxSurfaceResolver`].
///
/// Mirrors `FxSurfaceResolver` for the single-curve cost-of-carry paradigm: one
/// discount leg `r` (the numeraire `discount_rate`), **no foreign leg** — the net
/// carry `b` is a scalar on the generalized carry arm ([`super::cost_of_carry`]),
/// not a second discount curve, so it never enters the `ResolvedMarket`; the leaf
/// re-derives it from the request context, exactly as the FX leaf sources its two
/// rates — plus the resolved scalar spot + vol. Captured once and lent as a
/// [`ResolvedMarket`] on [`MarketResolver::resolve`] (the standard lending pattern).
pub(super) struct CrossAssetCarryResolver {
    /// The numeraire discount leg `DF(t) = e^{−r·t}`, `r = market.discount_rate`.
    discount: FlatDiscountCurve,
    /// The resolved scalar Black vol the request-tier price uses.
    vol: f64,
    /// The resolved spot (the physical spot / the quoted futures price).
    spot: f64,
    /// The resolved trade conventions.
    conventions: ConventionSet,
}

impl CrossAssetCarryResolver {
    /// Capture a resolved cross-asset market context as a cross-asset market
    /// resolver. The discount leg is the degenerate one-pillar curve at the
    /// numeraire discount rate `r` (= `market.discount_rate`); the net carry `b`
    /// is NOT captured here — the leaf reads it from the request context via
    /// [`super::cost_of_carry`] (the byte-identity seam).
    pub(super) fn from_market(market: &WireMarketContext, conv: &ConventionSet) -> Self {
        Self {
            discount: FlatDiscountCurve::new(market.discount_rate),
            vol: market.vol,
            spot: market.spot,
            conventions: *conv,
        }
    }
}

impl MarketResolver for CrossAssetCarryResolver {
    type Request = ();
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Error = PriceError;

    fn resolve(&self, _request: &()) -> Result<ResolvedMarket<'_, ConventionSet>, PriceError> {
        Ok(ResolvedMarket::new(
            &self.discount,
            None, // single-curve cost-of-carry market — no foreign leg
            Some(self.vol),
            Some(self.spot),
            &self.conventions,
        ))
    }
}

/// The cross-asset **equity** vanilla leaf (generalized-BSM, carry `b = r − q`)
/// re-seated onto the unified contract. Byte-identical to the equity arm of
/// [`super::price_cross_asset`].
pub(super) struct EquityVanillaEngine;
impl Priceable for EquityVanillaEngine {
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Ctx<'a> = EngineCtx<'a>;
    type Priced = Priced;
    type Error = PriceError;

    fn price(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let v = expect_vanilla(ctx.instrument)?;
        let (option_type, strike) = cross_asset_vanilla_terms(v)?;
        let (spot, vol) = resolved_spot_vol(market)?;
        // The net carry is read from the request context (the frozen cost-of-carry
        // arithmetic), NOT the ResolvedMarket's single discount leg — mirroring the
        // A1 FX byte-identity rationale.
        let carry = cost_of_carry(ctx.market)?;
        // Generalized-BSM: the equity leaf takes (r, q) with q = r − b.
        let r = carry.discount_rate();
        let q = r - carry.carry_rate();
        let inputs = EquityInputs::dividend_paying(spot, strike, vol, ctx.expiry, r, q);
        let g = celnet_equity_vanilla::greeks(option_type, &inputs);
        // The equity leaf's EquityGreeks mirrors CarryGreeks field-for-field; reuse
        // the shared carry→flat-greeks projection (verbatim from price_cross_asset).
        let cg = CarryGreeks {
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
        };
        Ok(Priced {
            greeks: carry_greeks_to_greeks(&cg),
            resolved_strike: strike,
            vol,
            std_error: None,
        })
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<RiskMeasure, PriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::OptionGreeks(carry_option_greeks(
            &priced.greeks,
        )))
    }
}

/// The cross-asset **commodity** vanilla leaf (Black-76 / cost-of-carry) re-seated
/// onto the unified contract. Byte-identical to the commodity arm of
/// [`super::price_cross_asset`].
pub(super) struct CommodityVanillaEngine;
impl Priceable for CommodityVanillaEngine {
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Ctx<'a> = EngineCtx<'a>;
    type Priced = Priced;
    type Error = PriceError;

    fn price(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let v = expect_vanilla(ctx.instrument)?;
        let (option_type, strike) = cross_asset_vanilla_terms(v)?;
        let (spot, vol) = resolved_spot_vol(market)?;
        let carry = cost_of_carry(ctx.market)?;
        // The commodity leaf consumes the `Carry` directly (spot is the physical
        // spot, `b` the net carry).
        let inputs = CommodityInputs::new(spot, strike, vol, ctx.expiry, carry);
        let g = celnet_commodity_vanilla::greeks(option_type, &inputs);
        Ok(Priced {
            greeks: carry_greeks_to_greeks(&g),
            resolved_strike: strike,
            vol,
            std_error: None,
        })
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<RiskMeasure, PriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::OptionGreeks(carry_option_greeks(
            &priced.greeks,
        )))
    }
}

/// The cross-asset **digital-asset (crypto) linear** vanilla leaf — USD-margined
/// generalized-BSM (`b = r − funding`) — re-seated onto the unified contract.
/// Byte-identical to the digital-asset LINEAR arm of [`super::price_cross_asset`].
pub(super) struct CryptoLinearEngine;
impl Priceable for CryptoLinearEngine {
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Ctx<'a> = EngineCtx<'a>;
    type Priced = Priced;
    type Error = PriceError;

    fn price(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let v = expect_vanilla(ctx.instrument)?;
        let (option_type, strike) = cross_asset_vanilla_terms(v)?;
        let (spot, vol) = resolved_spot_vol(market)?;
        let carry = cost_of_carry(ctx.market)?;
        let inputs = CryptoLinearInputs::new(spot, strike, vol, ctx.expiry, carry);
        let g = crypto_linear::greeks(option_type, &inputs);
        Ok(Priced {
            greeks: carry_greeks_to_greeks(&g),
            resolved_strike: strike,
            vol,
            std_error: None,
        })
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<RiskMeasure, PriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::OptionGreeks(carry_option_greeks(
            &priced.greeks,
        )))
    }
}

/// The cross-asset **digital-asset (crypto) inverse** vanilla leaf — coin-margined
/// (`1/S_T` payoff) — re-seated onto the unified contract. The headline strip is in
/// COINS (the contract's natural unit a coin-margined desk hedges in), taken via
/// [`crypto_inverse::greeks`]'s `.coin` arm. Byte-identical to the digital-asset
/// INVERSE_COIN arm of [`super::price_cross_asset`].
pub(super) struct CryptoInverseEngine;
impl Priceable for CryptoInverseEngine {
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Ctx<'a> = EngineCtx<'a>;
    type Priced = Priced;
    type Error = PriceError;

    fn price(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let v = expect_vanilla(ctx.instrument)?;
        let (option_type, strike) = cross_asset_vanilla_terms(v)?;
        let (spot, vol) = resolved_spot_vol(market)?;
        let carry = cost_of_carry(ctx.market)?;
        let inputs = CryptoInverseInputs::new(spot, strike, vol, ctx.expiry, carry);
        let g = crypto_inverse::greeks(option_type, &inputs).coin;
        Ok(Priced {
            greeks: carry_greeks_to_greeks(&g),
            resolved_strike: strike,
            vol,
            std_error: None,
        })
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<RiskMeasure, PriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::OptionGreeks(carry_option_greeks(
            &priced.greeks,
        )))
    }
}

/// The cross-asset **perpetual American** leaf (time-homogeneous, over the
/// generalized cost-of-carry arm) re-seated onto the unified contract.
/// Byte-identical to the perpetual sub-arm of [`super::price_cross_asset`].
pub(super) struct CrossAssetPerpetualEngine;
impl Priceable for CrossAssetPerpetualEngine {
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Ctx<'a> = EngineCtx<'a>;
    type Priced = Priced;
    type Error = PriceError;

    fn price(
        &self,
        _market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let p = expect_perpetual(ctx.instrument)?;
        // The perpetual leaf consumes the full request context (spot, vol, carry),
        // so — like the A1 plugin-model engine — the ResolvedMarket rides as the
        // produced contract handle rather than the arithmetic source. The carry is
        // the generalized cost-of-carry arm (verbatim from price_cross_asset).
        price_perpetual(p, ctx.market, cost_of_carry(ctx.market)?)
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<RiskMeasure, PriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::OptionGreeks(carry_option_greeks(
            &priced.greeks,
        )))
    }
}

/// The cross-asset **listed-future option** leaf (asset-class-agnostic Black-76 on
/// the quoted future) re-seated onto the unified contract. Byte-identical to the
/// listed-future sub-arm of [`super::price_cross_asset`].
pub(super) struct CrossAssetListedFutureEngine;
impl Priceable for CrossAssetListedFutureEngine {
    type Market<'a> = ResolvedMarket<'a, ConventionSet>;
    type Ctx<'a> = EngineCtx<'a>;
    type Priced = Priced;
    type Error = PriceError;

    fn price(
        &self,
        _market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let o = expect_listed_future(ctx.instrument)?;
        // Asset-class-agnostic: the quoted futures price embodies the carry, so the
        // leaf reads only the request context (identical to the listed-future
        // sub-arm of price_cross_asset and to the FX-path ListedFutureOptionEngine).
        price_listed_future_option(o, ctx.market, ctx.expiry)
    }

    fn risk(
        &self,
        market: &ResolvedMarket<'_, ConventionSet>,
        ctx: &EngineCtx<'_>,
    ) -> Result<RiskMeasure, PriceError> {
        let priced = <Self as Priceable>::price(self, market, ctx)?;
        Ok(RiskMeasure::OptionGreeks(carry_option_greeks(
            &priced.greeks,
        )))
    }
}

// ===========================================================================
// FX exotic product-family leaves — ADR-0017 Phase A2b
// ===========================================================================
//
// The exotic companion of the A1 FX vanilla re-seat above: every NON-vanilla FX
// product-family [`ProductEngine`] in [`super::engines`] ALSO implements the
// unified [`Priceable`] contract, wrapping the SAME [`ProductEngine::price`] body
// it dispatches today. This completes the OPTIONS side of the contract (vanilla
// FX = A1, cross-asset = A2, exotics = A2b).
//
// # Byte-identity by construction (critique F2 — leaf-level re-seat only)
//
// The re-seat is purely a seam: [`Priceable::price`] extracts the family's decoded
// product oneof from `ctx.instrument` and calls the identical
// `ProductEngine::price(self, product, ctx)` against the SAME [`EngineCtx`] the
// `price_instrument` dispatch route builds — same market, expiry, conventions,
// same finite-difference / Monte-Carlo (`seed`, `pairs`) sourcing. So every exotic
// price/greek is `to_bits`-unchanged from the dispatch route (gated by
// `crate::pricer::tests::exotic_reseat_is_byte_identical` plus the server
// exotic-routing frozen pins). The exotic engines read every input from `ctx`
// (never from the resolved market), so the [`ResolvedMarket`] rides only as the
// produced contract handle — exactly as the A1 plugin-model leaf does.
//
// `risk()` reports the unified [`RiskMeasure::OptionGreeks`] tag lifted from the
// priced FX Greek strip by the single carry-tagged source [`fx_carry_greeks`] (the
// two FX rhos verbatim, [`celnet_types::RateSensitivities::Fx`]) — identical to the
// A1 vanilla risk seam, because every FX exotic prices on the FX two-rate arm.
//
// The [`celnet_core::carry::ExoticLegPricer`] VaR seam (consumed by the risk cube)
// and the plugin-host dynamic dispatch are UNTOUCHED: this is an additive
// leaf-level re-seat, not a change to routing, the guard cascade, or the VaR path.

/// Reify the [`Priceable`] leaf re-seat for an FX exotic [`ProductEngine`].
///
/// `$engine` is the (zero-sized) family engine, `$variant` its
/// [`celnet_proto::instrument::Product`] oneof arm, and `$label` the human name for
/// the mismatch error. The generated [`Priceable::price`] extracts the decoded
/// product oneof from `ctx.instrument` and calls the verbatim
/// [`ProductEngine::price`]; [`Priceable::risk`] lifts the priced FX Greek strip to
/// the unified [`RiskMeasure::OptionGreeks`] tag via [`fx_carry_greeks`]. The
/// resolved market is unused by the price (the engine reads `ctx`), so the re-seat
/// is byte-identical to the dispatch route by construction.
macro_rules! fx_exotic_priceable {
    ($engine:ty, $variant:ident, $label:literal) => {
        impl Priceable for $engine {
            type Market<'a> = ResolvedMarket<'a, ConventionSet>;
            type Ctx<'a> = EngineCtx<'a>;
            type Priced = Priced;
            type Error = PriceError;

            fn price(
                &self,
                _market: &ResolvedMarket<'_, ConventionSet>,
                ctx: &EngineCtx<'_>,
            ) -> Result<Priced, PriceError> {
                // Extract this family's decoded product oneof from the request
                // instrument, then price via the IDENTICAL ProductEngine body the
                // dispatch route calls — the whole of the byte-identity guarantee.
                let product = match ctx.instrument.product.as_ref() {
                    Some(instrument::Product::$variant(p)) => p,
                    Some(_) => {
                        return Err(PriceError::Domain(concat!(
                            "the ",
                            $label,
                            " Priceable leaf prices only the ",
                            $label,
                            " product"
                        )));
                    }
                    None => return Err(PriceError::EmptyProduct),
                };
                ProductEngine::price(self, product, ctx)
            }

            fn risk(
                &self,
                market: &ResolvedMarket<'_, ConventionSet>,
                ctx: &EngineCtx<'_>,
            ) -> Result<RiskMeasure, PriceError> {
                let priced = <Self as Priceable>::price(self, market, ctx)?;
                // Every FX exotic prices on the FX two-rate arm ⇒ the two FX rhos
                // verbatim (`RateSensitivities::Fx`), identical to the A1 vanilla
                // risk seam.
                Ok(RiskMeasure::OptionGreeks(fx_carry_greeks(&priced.greeks)))
            }
        }
    };
}

// The 23 non-vanilla FX product families (the FX/analytic dispatch registry of
// `super::engines::dispatch`, minus `Vanilla` — that arm is the A1 re-seat above,
// including its plugin-host override). One `Priceable` leaf per family, each a
// verbatim wrapper of the family's `ProductEngine::price`.
fx_exotic_priceable!(StrategyEngine, Strategy, "strategy");
fx_exotic_priceable!(SingleBarrierEngine, SingleBarrier, "single-barrier");
fx_exotic_priceable!(DoubleBarrierEngine, DoubleBarrier, "double-barrier");
fx_exotic_priceable!(DigitalEngine, Digital, "digital");
fx_exotic_priceable!(TouchEngine, Touch, "touch");
fx_exotic_priceable!(VarianceSwapEngine, VarianceSwap, "variance-swap");
fx_exotic_priceable!(VolatilitySwapEngine, VolatilitySwap, "volatility-swap");
fx_exotic_priceable!(AsianOptionEngine, AsianOption, "Asian-option");
fx_exotic_priceable!(ForwardStartEngine, ForwardStart, "forward-start");
fx_exotic_priceable!(CliquetEngine, Cliquet, "cliquet");
fx_exotic_priceable!(QuantoEngine, Quanto, "quanto");
fx_exotic_priceable!(TarfEngine, Tarf, "TARF");
fx_exotic_priceable!(PivotEngine, Pivot, "pivot-TRA");
fx_exotic_priceable!(AccumulatorEngine, Accumulator, "accumulator");
fx_exotic_priceable!(LookbackEngine, Lookback, "lookback");
fx_exotic_priceable!(AmericanEngine, American, "American");
fx_exotic_priceable!(BasketEngine, Basket, "basket");
fx_exotic_priceable!(WindowBarrierEngine, WindowBarrier, "window-barrier");
fx_exotic_priceable!(FxForwardEngine, FxForward, "FX-forward");
fx_exotic_priceable!(FxSwapEngine, FxSwap, "FX-swap");
fx_exotic_priceable!(NdfEngine, Ndf, "NDF");
fx_exotic_priceable!(PerpetualOptionEngine, PerpetualOption, "perpetual-option");
fx_exotic_priceable!(
    ListedFutureOptionEngine,
    ListedFutureOption,
    "listed-future-option"
);

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::CarryInputs;
    use celnet_plugin_api::example::FlatSmilePricer;
    use celnet_plugin_host::ModelRegistry;
    use celnet_proto::{
        Instrument, MarketContext, OptionType as WOptionType, StrikeOrDelta, Underlying, Vanilla,
        instrument::Product, strike_or_delta::Spec,
    };
    use celnet_types::{Carry, RateSensitivities};

    fn conv() -> ConventionSet {
        let wire = celnet_proto::Conventions {
            delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
            atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
            premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
            cut: celnet_proto::Cut::NewYork1000 as i32,
            day_count: celnet_proto::DayCount::Act365Fixed as i32,
            settlement: celnet_proto::Settlement::Deliverable as i32,
        };
        ConventionSet::decode(&wire).unwrap()
    }

    fn eurusd() -> Underlying {
        Underlying::fx(celnet_proto::CcyPair {
            base: "EUR".into(),
            quote: "USD".into(),
        })
    }

    fn instr(option: WOptionType, spec: Spec, expiry: f64) -> Instrument {
        Instrument {
            underlying: Some(eurusd()),
            expiry_years: expiry,
            side: celnet_proto::Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::Vanilla(Vanilla {
                option_type: option as i32,
                strike: Some(StrikeOrDelta { spec: Some(spec) }),
            })),
            ..Default::default()
        }
    }

    /// The full grid the byte-identity gate sweeps: markets across sign of rates,
    /// vols and tenors, both option types, and both strike specifications
    /// (absolute + delta-keyed — the latter exercising the F4 delta solver read
    /// from the `ResolvedMarket`'s conventions).
    fn grid() -> Vec<(Instrument, MarketContext)> {
        let markets = [
            MarketContext::fx(1.10, 0.10, 0.02, 0.01),
            MarketContext::fx(1.2345, 0.185, 0.055, 0.005),
            MarketContext::fx(0.80, 0.45, -0.01, 0.06),
            MarketContext::fx(150.0, 0.09, 0.001, -0.004),
        ];
        let expiries = [7.0 / 365.0, 0.5, 1.0, 3.0];
        let mut out = Vec::new();
        for market in &markets {
            for &expiry in &expiries {
                for option in [WOptionType::Call, WOptionType::Put] {
                    // An OTM + an ITM absolute strike, scaled to the spot regime.
                    for k in [1.05_f64, 1.25] {
                        out.push((
                            instr(option, Spec::Strike(k * market.spot / 1.10), expiry),
                            *market,
                        ));
                    }
                    // A sign-valid delta key (call deltas are positive, put deltas
                    // negative) — exercises the F4 delta solver read from the
                    // ResolvedMarket's conventions. Two magnitudes for breadth.
                    for mag in [0.25_f64, 0.10] {
                        let d = match option {
                            WOptionType::Call => mag,
                            WOptionType::Put => -mag,
                        };
                        out.push((instr(option, Spec::Delta(d), expiry), *market));
                    }
                }
            }
        }
        out
    }

    fn assert_priced_bit_identical(got: &Priced, want: &Priced) {
        let g = &got.greeks;
        let w = &want.greeks;
        // All 14 members (price + 13 sensitivities) to the bit.
        for (name, a, b) in [
            ("price", g.price, w.price),
            ("delta_spot", g.delta_spot, w.delta_spot),
            ("delta_forward", g.delta_forward, w.delta_forward),
            ("gamma", g.gamma, w.gamma),
            ("vega", g.vega, w.vega),
            ("theta", g.theta, w.theta),
            ("rho_dom", g.rho_dom, w.rho_dom),
            ("rho_for", g.rho_for, w.rho_for),
            ("vanna", g.vanna, w.vanna),
            ("volga", g.volga, w.volga),
            ("charm", g.charm, w.charm),
            ("speed", g.speed, w.speed),
            ("zomma", g.zomma, w.zomma),
            ("color", g.color, w.color),
        ] {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "greek `{name}` drifted through the re-seat"
            );
        }
        assert_eq!(
            got.resolved_strike.to_bits(),
            want.resolved_strike.to_bits()
        );
        assert_eq!(got.vol.to_bits(), want.vol.to_bits());
        assert_eq!(
            got.std_error.map(f64::to_bits),
            want.std_error.map(f64::to_bits)
        );
    }

    /// THE GATE: every FX vanilla price/greek is `to_bits`-UNCHANGED through the
    /// `Priceable` re-seat vs the established `price_instrument` dispatch route,
    /// across the whole grid (calls/puts, absolute + delta strikes, ± rates).
    #[test]
    fn fx_vanilla_reseat_is_byte_identical() {
        let conv = conv();
        for (instrument, market) in grid() {
            // The canonical dispatch route (the unified engine → plugin-host
            // `dispatch_live` → the vanilla `ProductEngine`).
            let want = super::super::price_instrument(&instrument, &market, &conv)
                .expect("dispatch route prices");
            // The `Priceable` vanilla-leaf route (the C2 risk seam), exercised through
            // the `#[cfg(test)]` contract dispatcher whose vanilla arm is `VanillaEngine`.
            let got = super::super::price_exotic_via_contract(&instrument, &market, &conv)
                .expect("contract route prices");
            assert_priced_bit_identical(&got, &want);
        }
    }

    /// The plugin-host `Priceable` re-seat is byte-identical to the plugin dispatch
    /// route (a registered house model priced through both seams), so the plugin
    /// pricing path is preserved (critique F2 "preserve plugin-host dynamic
    /// dispatch").
    #[test]
    fn plugin_reseat_is_byte_identical() {
        let conv = conv();
        let mut registry = ModelRegistry::new();
        registry
            .register_native(FlatSmilePricer::new(0.10))
            .unwrap();

        let market = MarketContext::fx(1.10, 0.10, 0.02, 0.01);
        let instrument = instr(WOptionType::Call, Spec::Strike(1.12), 1.0);
        let product = instrument.product.as_ref().unwrap();

        // Dispatch route with the registry present (routes Vanilla through the
        // plugin engine internally).
        let want = super::super::engines::dispatch(
            product,
            &EngineCtx {
                instrument: &instrument,
                market: &market,
                expiry: 1.0,
                conv: &conv,
                plugin_models: Some(&registry),
            },
        )
        .unwrap();

        // Priceable route over the same registered model.
        let model = registry.active_pricing_model().unwrap();
        let engine =
            PluginModelEngine::new(model, celnet_types::Underlying::try_from(eurusd()).unwrap());
        let resolver = FxSurfaceResolver::from_market(&market, &conv);
        let rm = resolver.resolve(&()).unwrap();
        let ctx = EngineCtx {
            instrument: &instrument,
            market: &market,
            expiry: 1.0,
            conv: &conv,
            plugin_models: Some(&registry),
        };
        let got = <PluginModelEngine as Priceable>::price(&engine, &rm, &ctx).unwrap();
        assert_priced_bit_identical(&got, &want);
    }

    /// The `Priceable::risk` re-seat reports the unified `OptionGreeks` tag whose
    /// strip is the byte-identical FX Greek set (the two FX rhos verbatim), so the
    /// risk seam introduces no drift either.
    #[test]
    fn fx_vanilla_risk_tag_is_byte_identical() {
        let conv = conv();
        let market = MarketContext::fx(1.10, 0.10, 0.02, 0.01);
        let instrument = instr(WOptionType::Call, Spec::Strike(1.12), 1.0);
        let priced = super::super::price_instrument(&instrument, &market, &conv).unwrap();

        let resolver = FxSurfaceResolver::from_market(&market, &conv);
        let rm = resolver.resolve(&()).unwrap();
        let ctx = EngineCtx {
            instrument: &instrument,
            market: &market,
            expiry: 1.0,
            conv: &conv,
            plugin_models: None,
        };
        let risk = <VanillaEngine as Priceable>::risk(&VanillaEngine, &rm, &ctx).unwrap();
        match risk {
            RiskMeasure::OptionGreeks(cg) => {
                assert_eq!(cg.price.to_bits(), priced.greeks.price.to_bits());
                assert_eq!(cg.delta_spot.to_bits(), priced.greeks.delta_spot.to_bits());
                assert_eq!(cg.vega.to_bits(), priced.greeks.vega.to_bits());
                match cg.rates {
                    RateSensitivities::Fx { rho_dom, rho_for } => {
                        assert_eq!(rho_dom.to_bits(), priced.greeks.rho_dom.to_bits());
                        assert_eq!(rho_for.to_bits(), priced.greeks.rho_for.to_bits());
                    }
                    RateSensitivities::Carry { .. } => panic!("FX risk must tag as Fx"),
                }
            }
            RiskMeasure::RateLadder(_) => panic!("FX vanilla must report the option arm"),
        }
    }

    /// The resolver produces a genuinely-populated `ResolvedMarket` (not a dummy):
    /// its resolved spot/vol echo the market and its flat FX legs discount the two
    /// rates. (`CarryInputs` is imported to pin the FX carry semantics the legs
    /// mirror: `DF_dom = e^{−r_dom·t}`, `DF_for = e^{−r_for·t}`.)
    #[test]
    fn resolver_builds_a_real_resolved_market() {
        let conv = conv();
        let market = MarketContext::fx(1.2345, 0.185, 0.055, 0.005);
        let resolver = FxSurfaceResolver::from_market(&market, &conv);
        let rm = resolver.resolve(&()).unwrap();

        assert_eq!(rm.spot, Some(market.spot));
        assert_eq!(rm.vol, Some(market.vol));
        // The flat legs discount the FX two rates, matching the carry the FX leaf
        // prices off (`Carry::discount_df`).
        let fx = Carry::FxRates {
            r_dom: market.r_dom(),
            r_for: market.r_for(),
        };
        for &t in &[0.25_f64, 1.0, 3.0] {
            assert!(
                celnet_core::is_close(
                    rm.discount.discount_factor(t),
                    fx.discount_df(t),
                    1e-15,
                    1e-15
                ),
                "domestic leg must discount at r_dom"
            );
            let for_leg = rm.foreign.unwrap().discount_factor(t);
            let want_for = celnet_core::math::exp(-market.r_for() * t);
            assert!(
                celnet_core::is_close(for_leg, want_for, 1e-15, 1e-15),
                "foreign leg must discount at r_for"
            );
        }
        // A `CarryInputs` built from the same resolved market forms the identical
        // forward the leaf uses — the seam the resolved market feeds.
        let ci = CarryInputs::new(
            market.spot,
            1.30,
            market.vol,
            1.0,
            eurusd().try_into().unwrap(),
            fx,
        );
        assert!(celnet_core::is_close(
            ci.forward(),
            market.spot * celnet_core::math::exp((market.r_dom() - market.r_for()) * 1.0),
            1e-15,
            1e-15
        ));
    }

    // =====================================================================
    // Cross-asset (equity / commodity / digital-asset) Phase-A2 re-seat.
    // =====================================================================

    /// A cross-asset market with the generalized cost-of-carry arm (`{r, b}`).
    fn ca_market(spot: f64, vol: f64, r: f64, b: f64) -> MarketContext {
        MarketContext {
            spot,
            vol,
            discount_rate: r,
            carry: Some(celnet_proto::CarryModel {
                model: Some(celnet_proto::carry_model::Model::Generalized(
                    celnet_proto::CostOfCarry { b },
                )),
            }),
        }
    }

    fn ca_equity() -> Underlying {
        celnet_proto::Underlying::equity(celnet_proto::EquityRef::new(
            celnet_proto::Symbol::new("AAPL", "XNAS"),
            "USD",
        ))
    }

    fn ca_commodity() -> Underlying {
        celnet_proto::Underlying::commodity(celnet_proto::CommodityRef::new(
            celnet_proto::Symbol::new("BRENT", ""),
            "USD",
        ))
    }

    fn ca_crypto(quote: &str) -> Underlying {
        celnet_proto::Underlying::digital_asset(celnet_proto::CryptoPair::new("BTC", quote))
    }

    fn ca_vanilla(
        underlying: Underlying,
        option: WOptionType,
        strike: f64,
        expiry: f64,
        settlement: celnet_proto::SettlementStyle,
    ) -> Instrument {
        Instrument {
            underlying: Some(underlying),
            expiry_years: expiry,
            side: celnet_proto::Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            settlement_style: settlement as i32,
            product: Some(Product::Vanilla(Vanilla {
                option_type: option as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(Spec::Strike(strike)),
                }),
            })),
            ..Default::default()
        }
    }

    fn ca_perpetual(underlying: Underlying, option: WOptionType, strike: f64) -> Instrument {
        Instrument {
            underlying: Some(underlying),
            expiry_years: 0.0,
            side: celnet_proto::Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::PerpetualOption(celnet_proto::PerpetualOption {
                option_type: option as i32,
                strike,
                notional: 1_000_000.0,
            })),
            ..Default::default()
        }
    }

    fn ca_listed_future(
        underlying: Underlying,
        option: WOptionType,
        strike: f64,
        expiry: f64,
        future_expiry: f64,
        margining: celnet_proto::Margining,
    ) -> Instrument {
        Instrument {
            underlying: Some(underlying),
            expiry_years: expiry,
            side: celnet_proto::Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::ListedFutureOption(
                celnet_proto::ListedFutureOption {
                    future_symbol: Some(celnet_proto::Symbol::new("BRN-DEC26", "IFEU")),
                    future_expiry_years: future_expiry,
                    option_type: option as i32,
                    strike,
                    notional: 1_000.0,
                    margining: margining as i32,
                },
            )),
            ..Default::default()
        }
    }

    /// The full cross-asset grid the byte-identity gate sweeps: equity + commodity
    /// (two spot regimes) and both crypto settlement styles, across both carry-arm
    /// encodings (the generalized `{r, b}` arm AND the FX two-rate arm the carry
    /// guard derives `b = r_dom − r_for` from), signs of the net carry, calls/puts,
    /// tenors and OTM/ITM strikes — plus the perpetual (equity/commodity/crypto,
    /// `b < r` so a call stays finite) and listed-future (equity/commodity, both
    /// margining styles) sub-arms.
    fn ca_grid() -> Vec<(Instrument, MarketContext)> {
        use celnet_proto::SettlementStyle;
        let expiries = [7.0 / 365.0, 0.5, 1.0, 3.0];
        let opts = [WOptionType::Call, WOptionType::Put];
        let mut out = Vec::new();

        // Equity (kind 0) + commodity (kind 1), two spot regimes each.
        for (spot, kind) in [(100.0_f64, 0u8), (2500.0, 0), (50.0, 1), (85.0, 1)] {
            let markets = [
                ca_market(spot, 0.20, 0.05, 0.02),
                ca_market(spot, 0.35, 0.03, -0.01),
                ca_market(spot, 0.28, 0.04, 0.0),
                // FX two-rate arm — cost_of_carry derives b = r_dom − r_for.
                MarketContext::fx(spot, 0.22, 0.05, 0.03),
            ];
            for m in &markets {
                for &expiry in &expiries {
                    for &opt in &opts {
                        for mul in [0.9_f64, 1.1] {
                            let u = if kind == 0 {
                                ca_equity()
                            } else {
                                ca_commodity()
                            };
                            out.push((
                                ca_vanilla(u, opt, mul * spot, expiry, SettlementStyle::Linear),
                                *m,
                            ));
                        }
                    }
                }
            }
        }

        // Crypto linear + inverse — spot ~30k, both settlement styles per case.
        let crypto_markets = [
            ca_market(30_000.0, 0.65, 0.05, 0.03),
            ca_market(30_000.0, 0.80, 0.02, -0.02),
            MarketContext::fx(30_000.0, 0.65, 0.05, 0.02),
        ];
        for m in &crypto_markets {
            for &expiry in &expiries {
                for &opt in &opts {
                    for mul in [0.9_f64, 1.1] {
                        out.push((
                            ca_vanilla(
                                ca_crypto("USDT"),
                                opt,
                                mul * 30_000.0,
                                expiry,
                                SettlementStyle::Linear,
                            ),
                            *m,
                        ));
                        out.push((
                            ca_vanilla(
                                ca_crypto("USD"),
                                opt,
                                mul * 30_000.0,
                                expiry,
                                SettlementStyle::InverseCoin,
                            ),
                            *m,
                        ));
                    }
                }
            }
        }

        // Perpetual American on equity / commodity / crypto underlyings (expiry 0).
        for kind in 0u8..3 {
            for m in [
                ca_market(100.0, 0.30, 0.05, 0.02),
                ca_market(100.0, 0.25, 0.06, 0.0),
            ] {
                for &opt in &opts {
                    for strike in [90.0_f64, 110.0] {
                        let u = match kind {
                            0 => ca_equity(),
                            1 => ca_commodity(),
                            _ => ca_crypto("USDT"),
                        };
                        out.push((ca_perpetual(u, opt, strike), m));
                    }
                }
            }
        }

        // Listed-future option on equity / commodity, both margining styles.
        for kind in [0u8, 1] {
            let m = ca_market(85.0, 0.30, 0.05, 0.0);
            for margining in [
                celnet_proto::Margining::EquityStyle,
                celnet_proto::Margining::FuturesStyle,
            ] {
                for &opt in &opts {
                    for strike in [80.0_f64, 95.0] {
                        let u = if kind == 0 {
                            ca_equity()
                        } else {
                            ca_commodity()
                        };
                        out.push((ca_listed_future(u, opt, strike, 0.5, 0.6, margining), m));
                    }
                }
            }
        }

        out
    }

    /// THE GATE: every cross-asset price/greek is `to_bits`-UNCHANGED through the
    /// unified engine's `Priceable`-leaf cross-asset dispatch (`price_instrument` →
    /// `PricingEngine`) vs the frozen pre-C1 native `price_cross_asset` oracle, across
    /// the whole grid — equity, commodity, crypto linear + inverse, perpetual and
    /// listed-future, over both carry-arm encodings.
    #[test]
    fn cross_asset_reseat_is_byte_identical() {
        let conv = conv();
        let grid = ca_grid();
        assert!(!grid.is_empty(), "the cross-asset grid must not be empty");
        for (instrument, market) in grid {
            // Production path: the unified engine routes cross-asset through the
            // cost-of-carry `Priceable` leaves.
            let got = super::super::price_instrument(&instrument, &market, &conv)
                .expect("engine cross-asset path prices");
            // Independent oracle: the frozen pre-C1 native `price_cross_asset` dispatch
            // (the retired duplicate, kept `#[cfg(test)]` — never the engine checking
            // itself).
            let wire_underlying = instrument.underlying.as_ref().unwrap();
            let underlying = celnet_types::Underlying::try_from(wire_underlying.clone()).unwrap();
            let product = instrument.product.as_ref().unwrap();
            let want = super::super::price_cross_asset(
                &underlying,
                &instrument,
                product,
                &market,
                instrument.expiry_years,
            )
            .expect("frozen native oracle prices");
            assert_priced_bit_identical(&got, &want);
        }
    }

    /// The cross-asset `Priceable::risk` re-seat reports the unified `OptionGreeks`
    /// measure carry-tagged (`RateSensitivities::Carry`): the price + carry-neutral
    /// strip are the priced greeks bit-for-bit, and the rate block is the documented
    /// lossless recovery of the flat cross-asset rhos (`carry_rho = −rho_for`,
    /// `discount_rho = rho_dom + rho_for`).
    #[test]
    fn cross_asset_risk_tags_the_carry_option_measure() {
        let conv = conv();
        let m = ca_market(100.0, 0.20, 0.05, 0.02);
        let instrument = ca_vanilla(
            ca_equity(),
            WOptionType::Call,
            105.0,
            1.0,
            celnet_proto::SettlementStyle::Linear,
        );
        let priced = super::super::price_instrument(&instrument, &m, &conv).unwrap();

        let resolver = CrossAssetCarryResolver::from_market(&m, &conv);
        let rm = resolver.resolve(&()).unwrap();
        let ctx = EngineCtx {
            instrument: &instrument,
            market: &m,
            expiry: 1.0,
            conv: &conv,
            plugin_models: None,
        };
        let risk =
            <EquityVanillaEngine as Priceable>::risk(&EquityVanillaEngine, &rm, &ctx).unwrap();
        match risk {
            RiskMeasure::OptionGreeks(cg) => {
                assert_eq!(cg.price.to_bits(), priced.greeks.price.to_bits());
                assert_eq!(cg.delta_spot.to_bits(), priced.greeks.delta_spot.to_bits());
                assert_eq!(cg.vega.to_bits(), priced.greeks.vega.to_bits());
                match cg.rates {
                    celnet_types::RateSensitivities::Carry {
                        discount_rho,
                        carry_rho,
                    } => {
                        assert_eq!(carry_rho.to_bits(), (-priced.greeks.rho_for).to_bits());
                        assert_eq!(
                            discount_rho.to_bits(),
                            (priced.greeks.rho_dom + priced.greeks.rho_for).to_bits()
                        );
                    }
                    celnet_types::RateSensitivities::Fx { .. } => {
                        panic!("cross-asset risk must tag the carry arm")
                    }
                }
            }
            RiskMeasure::RateLadder(_) => {
                panic!("cross-asset vanilla must report the option arm")
            }
        }
    }
}
