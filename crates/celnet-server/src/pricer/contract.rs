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

use celnet_core::contract::{
    FlatDiscountCurve, MarketResolver, Priceable, ResolvedMarket, RiskMeasure,
};
use celnet_core::fx_carry_greeks;
use celnet_proto::{Instrument, MarketContext as WireMarketContext, instrument};

use super::engines::{EngineCtx, PluginModelEngine, ProductEngine, VanillaEngine};
use super::{
    ConventionSet, PriceError, Priced, decode_option_type, price_vanilla_leg, resolve_strike,
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
            // The established product-`match` dispatch route.
            let want = super::super::price_instrument(&instrument, &market, &conv)
                .expect("dispatch route prices");
            // The unified-contract route: resolver → ResolvedMarket → Priceable.
            let got = super::super::price_vanilla_via_contract(&instrument, &market, &conv)
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
}
