//! The unified **pricing engine** — the ONE internal pricing dispatch (ADR-0017
//! **Phase C1**, `docs/plan/CENTRAL-CORE-UNIFICATION.md`).
//!
//! # Why one engine
//!
//! Before C1 the platform carried *parallel* dispatch structures over the same
//! math. The native product routing reachable from [`super::price_instrument`]
//! (FX/analytic through [`super::engines::dispatch_live`], cross-asset through the
//! former `price_cross_asset`) and the linear-FI [`crate::rates_pricing::price_rates`]
//! routing were each shadowed by a contract-native twin (`price_*_via_contract`)
//! whose only job was to PROVE the [`celnet_core::contract::Priceable`] leaf is
//! `to_bits`-identical to the native path. Those proofs are done — the leaves are
//! byte-identical — so C1 collapses the duplication into a single canonical
//! dispatch.
//!
//! [`PricingEngine`] is that dispatch. It runs the options pre-dispatch guard
//! cascade verbatim, then routes each instrument to its [`Priceable`] leaf through
//! the correct [`celnet_core::contract::MarketResolver`]: the marked-surface
//! resolver for FX/metal options, the cost-of-carry resolver
//! ([`super::contract::CrossAssetCarryResolver`]) for cross-asset, and the
//! curve-bootstrap resolver for linear FI. The two edge RPCs
//! ([`crate::services::pricing`]) are thin façades over it (no proto change,
//! ADR-0007/0009 — one unversioned contract); the proof-only `via_contract`
//! entries and the duplicate native cross-asset dispatch are retired.
//!
//! # What is preserved verbatim (byte-identity)
//!
//! * The **options guard cascade** — the perpetual term-shape guard, asset-class
//!   routing ([`super::is_cross_asset`]), the FX two-rate carry guard, the
//!   LSV booking-model selector, and the plugin-host
//!   [`super::engines::dispatch_live`] terminal (which threads the calling worker's
//!   installed house-model registry) — is moved here UNCHANGED from
//!   `price_instrument`. The engine dispatches only AFTER those routing decisions.
//! * The FX/analytic terminal stays [`super::engines::dispatch_live`], so the
//!   plugin-host dynamic dispatch AND the [`celnet_core::carry::ExoticLegPricer`]
//!   VaR seam are untouched. The FX vanilla/exotic `Priceable` leaves — proven equal
//!   to that dispatch — remain the (Phase C2) risk seam, not the price path.
//! * Cross-asset routes through the cost-of-carry [`Priceable`] leaves; each leaf
//!   reads its resolved spot/vol from the [`ResolvedMarket`](celnet_core::contract::ResolvedMarket)
//!   the resolver produced and its net carry from the request context, so its
//!   price/greeks are `to_bits`-identical to the retired `price_cross_asset` (the
//!   Phase-A2 byte-identity gate, re-asserted by [`tests::unified_engine_matches_pre_c1_dispatch`]).
//! * Linear FI stays on [`crate::rates_pricing::price_rates`], the single-bootstrap
//!   OIS body. The FI curve resolver ([`crate::rates_pricing`]'s `RatesCurveResolver`)
//!   is the C2 risk seam; the OIS leaf re-bootstraps from the request quotes and
//!   ignores the resolved discount leg, so forcing the calc path through the resolver
//!   would only duplicate the bootstrap for zero numeric change — the engine keeps
//!   the direct body and stays byte-identical.

use celnet_core::contract::{MarketResolver, Priceable};
use celnet_crypto_vanilla::SettlementStyle as CryptoSettlementStyle;
use celnet_proto::{
    Instrument, MarketContext as WireMarketContext, RatesPriceRequest, RatesPricingResult,
    instrument,
};
use celnet_types::Underlying;

use super::contract::{
    CommodityVanillaEngine, CrossAssetCarryResolver, CrossAssetListedFutureEngine,
    CrossAssetPerpetualEngine, CryptoInverseEngine, CryptoLinearEngine, EquityVanillaEngine,
};
use super::engines::{self, EngineCtx};
use super::{
    ConventionSet, PriceError, Priced, decode_settlement_style, is_cross_asset,
    price_instrument_lsv, product_name, wire_error_to_price_error,
};
use crate::rates_pricing::{RatesPriceError, dispatch_rates_live};

/// The one internal pricing dispatch (ADR-0017 Phase C1).
///
/// A stateless dispatcher: it owns no market data (each call carries its resolved
/// market context), so its methods are associated functions. It is the single
/// canonical routing every edge RPC and internal caller reaches — the guard cascade
/// plus the resolver/leaf selection, and nothing else.
#[derive(Debug, Clone, Copy)]
pub struct PricingEngine;

impl PricingEngine {
    /// Price a wire [`Instrument`] against a market context and conventions — the
    /// unified **options + cross-asset** dispatch.
    ///
    /// Runs the options pre-dispatch guard cascade (perpetual term-shape,
    /// asset-class routing, the FX two-rate carry guard, the LSV booking-model
    /// selector) verbatim, then routes to the correct
    /// [`MarketResolver`]/[`Priceable`] leaf: cross-asset through the cost-of-carry
    /// resolver + its leaf, FX/analytic through the plugin-host
    /// [`engines::dispatch_live`]. This is the sole canonical replacement for the
    /// former `price_instrument` product routing; [`super::price_instrument`] is now
    /// a thin façade over it.
    ///
    /// # Errors
    ///
    /// [`PriceError`] if the instrument is malformed, carries an unknown enum, an
    /// empty product, or an out-of-domain input.
    pub fn price(
        instrument: &Instrument,
        market: &WireMarketContext,
        conv: &ConventionSet,
    ) -> Result<Priced, PriceError> {
        let expiry = instrument.expiry_years;
        // Term-shape guard. Every product needs a positive finite expiry EXCEPT the
        // perpetual arm, which has NO expiry by construction: its arm requires
        // `expiry_years == 0` exactly, enforced by the contract's canonical
        // validator (`celnet_proto::convert::validate_perpetual_terms`) and mapped
        // to `INVALID_ARGUMENT` at the boundary — never silently ignored.
        if matches!(
            instrument.product.as_ref(),
            Some(instrument::Product::PerpetualOption(_))
        ) {
            celnet_proto::convert::validate_perpetual_terms(expiry)
                .map_err(wire_error_to_price_error)?;
        } else if expiry <= 0.0 || !expiry.is_finite() {
            return Err(PriceError::Domain("expiry_years must be positive"));
        }

        // Asset-class routing FIRST (ADR-0008): branch on the decoded underlying
        // before the FX carry guard. An equity / commodity / digital-asset
        // underlying is priced by the cross-asset cost-of-carry leaves, which ACCEPT
        // the generalized `CostOfCarry { b }` carry; that path never touches the FX
        // two-rate guard below. An FX / metal underlying (or an absent underlying, as
        // a pure-context price request carries) falls through to the UNCHANGED FX
        // path + its guard, so the FX/metal contract stays byte-identical. The
        // underlying is decoded for routing only.
        if let Some(wire_underlying) = instrument.underlying.as_ref()
            && let Ok(underlying) = Underlying::try_from(wire_underlying.clone())
            && is_cross_asset(&underlying)
        {
            let product = instrument
                .product
                .as_ref()
                .ok_or(PriceError::EmptyProduct)?;
            // The LSV booking model is the FX vol-surface engine; it does not apply
            // to a cross-asset cost-of-carry leaf. Selecting it for a cross-asset
            // underlying is refused with a typed error, never silently downgraded to
            // the analytic leaf.
            let model =
                celnet_proto::PricingModel::try_from(instrument.pricing_model).map_err(|_| {
                    PriceError::UnknownEnum {
                        kind: "PricingModel",
                        tag: instrument.pricing_model,
                    }
                })?;
            if matches!(model, celnet_proto::PricingModel::LocalStochVol) {
                return Err(PriceError::UnsupportedModel {
                    model: "LOCAL_STOCH_VOL",
                    product: product_name(product),
                });
            }
            return Self::dispatch_cross_asset(&underlying, instrument, product, market, conv);
        }

        // Carry-producing-market architecture (no silent fallback): the FX pricing
        // path prices the FX two-rate carry only. An explicitly-supplied generalized
        // (cost-of-carry) carry is refused here with a typed error — never read as FX
        // with r_for = 0 and a wrong forward. (An absent carry stays the FX default,
        // exactly as before; this guard also covers the LSV path it dispatches to.)
        if market
            .carry
            .as_ref()
            .is_some_and(|c| c.fx_r_for().is_none())
        {
            return Err(PriceError::Domain(
                "market carry must be the FX two-rate arm",
            ));
        }
        let product = instrument
            .product
            .as_ref()
            .ok_or(PriceError::EmptyProduct)?;

        // Booking-model selector (CLAUDE.md rule 9: a pricing directive, not an API
        // version). An absent or DEFAULT model takes the analytic path below
        // byte-identically; LOCAL_STOCH_VOL routes the supported products through the
        // LSV engine and rejects every other product clearly.
        let model =
            celnet_proto::PricingModel::try_from(instrument.pricing_model).map_err(|_| {
                PriceError::UnknownEnum {
                    kind: "PricingModel",
                    tag: instrument.pricing_model,
                }
            })?;
        if matches!(model, celnet_proto::PricingModel::LocalStochVol) {
            return price_instrument_lsv(product, market, expiry);
        }

        // Decode → guard → DISPATCH: the FX/analytic product path is a single static
        // dispatch into the per-family engine registry (`engines.rs`).
        // `dispatch_live` threads the calling worker's installed house-model registry
        // onto the engine context (plugin-host dynamic dispatch): when a pricing
        // model is registered the analytic vanilla arm prices through the plugin-host
        // registry, otherwise every arm is the verbatim native static dispatch. The
        // `ExoticLegPricer` VaR seam is threaded by the same engines untouched.
        engines::dispatch_live(product, instrument, market, expiry, conv)
    }

    /// Dispatch an already-routed cross-asset (equity / commodity / digital-asset)
    /// instrument to its cost-of-carry [`Priceable`] leaf through the
    /// [`CrossAssetCarryResolver`].
    ///
    /// The caller ([`Self::price`]) has already decoded the underlying, verified it
    /// is cross-asset, extracted the product, and refused the LSV booking model —
    /// exactly the guard cascade the former `price_cross_asset` sat behind. This
    /// mirrors that routing arm-for-arm through the contract seam: the resolved
    /// market carries the single discount leg + resolved scalar spot/vol, each leaf
    /// reads the net carry from the request context (`super::cost_of_carry`, the
    /// frozen cost-of-carry arithmetic), so every price/greek is byte-identical to
    /// the retired native dispatch. The delta-key refusal, crypto settlement-style
    /// selection, and exotic / FX-metal refusals live in the leaves / this match.
    fn dispatch_cross_asset(
        underlying: &Underlying,
        instrument: &Instrument,
        product: &instrument::Product,
        market: &WireMarketContext,
        conv: &ConventionSet,
    ) -> Result<Priced, PriceError> {
        // The cross-asset ResolvedMarket the leaf engines price against: a single
        // discount leg `r`, no foreign leg, the resolved scalar spot + vol.
        let resolver = CrossAssetCarryResolver::from_market(market, conv);
        let resolved = resolver.resolve(&())?;
        let ctx = EngineCtx {
            instrument,
            market,
            expiry: instrument.expiry_years,
            conv,
            plugin_models: None,
        };

        match product {
            instrument::Product::Vanilla(_) => match underlying {
                Underlying::Equity(_) => EquityVanillaEngine.price(&resolved, &ctx),
                Underlying::Commodity(_) => CommodityVanillaEngine.price(&resolved, &ctx),
                Underlying::DigitalAsset(_) => {
                    match decode_settlement_style(instrument.settlement_style)? {
                        CryptoSettlementStyle::Linear => CryptoLinearEngine.price(&resolved, &ctx),
                        CryptoSettlementStyle::InverseCoin => {
                            CryptoInverseEngine.price(&resolved, &ctx)
                        }
                    }
                }
                Underlying::Fx(_) | Underlying::Metal(_) => Err(PriceError::Domain(
                    "internal: FX/metal underlying routed to the cross-asset path",
                )),
            },
            instrument::Product::PerpetualOption(_) => {
                CrossAssetPerpetualEngine.price(&resolved, &ctx)
            }
            instrument::Product::ListedFutureOption(_) => {
                CrossAssetListedFutureEngine.price(&resolved, &ctx)
            }
            other => Err(PriceError::UnsupportedModel {
                model: "DEFAULT",
                product: product_name(other),
            }),
        }
    }

    /// Price a linear-rates [`RatesPriceRequest`] — the unified **linear FI**
    /// dispatch (the `PriceRates` edge RPC's engine entry).
    ///
    /// Linear FI is a distinct paradigm (a bootstrapped discount curve + a linear
    /// swap, not the asset-class-agnostic option carry kernel) with a distinct wire
    /// request/result, so it is a distinct entry on the one engine rather than an arm
    /// of [`Self::price`]. It routes through the pluggable FI dispatch
    /// [`crate::rates_pricing::dispatch_rates_live`] — the fixed-income analog of the
    /// option [`engines::dispatch_live`] terminal (ADR-0021): the calling worker's
    /// installed FI house-model registry is threaded in, so a registered rates model
    /// overrides the native pricer per FI product kind, otherwise every arm is the
    /// verbatim native [`crate::rates_pricing::price_rates`] body (byte-identical).
    /// `celnet_rates` / `celnet_bond` engines are byte-for-byte unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`RatesPriceError`] for a missing/invalid curve set, an unsupported
    /// currency, malformed pillars, a missing/invalid instrument, a numeric
    /// schedule/bootstrap failure, or a registered house-model failure.
    pub fn price_rates(req: &RatesPriceRequest) -> Result<RatesPricingResult, RatesPriceError> {
        dispatch_rates_live(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The pre-C1 native rates dispatch: the independent oracle the unified
    // engine's rates path (now the pluggable `dispatch_rates_live`, empty-registry)
    // is byte-checked against.
    use crate::rates_pricing::price_rates;
    use celnet_proto::{
        BrokenDate, CarryModel, CcyPair, CommodityRef, CostOfCarry, CryptoPair, CurveSet,
        EquityRef, MarketContext, OisInstrument, OisPillar, PillarTenor, RatesInstrument,
        SettlementStyle, Side, StrikeOrDelta, Symbol, Vanilla, carry_model, instrument::Product,
        pillar_tenor, rates_instrument, strike_or_delta,
    };

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

    /// Assert two priced results are equal to the bit across all 14 Greek members and
    /// the resolved strike / vol / std-error — the C1 byte-identity gate.
    fn assert_priced_bit_identical(got: &Priced, want: &Priced, label: &str) {
        let g = &got.greeks;
        let w = &want.greeks;
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
                "[{label}] greek `{name}` drifted through the unified engine: {a} vs {b}"
            );
        }
        assert_eq!(
            got.resolved_strike.to_bits(),
            want.resolved_strike.to_bits(),
            "[{label}] resolved_strike drifted"
        );
        assert_eq!(
            got.vol.to_bits(),
            want.vol.to_bits(),
            "[{label}] vol drifted"
        );
        assert_eq!(
            got.std_error.map(f64::to_bits),
            want.std_error.map(f64::to_bits),
            "[{label}] std_error drifted"
        );
    }

    fn eurusd() -> celnet_proto::Underlying {
        celnet_proto::Underlying::fx(CcyPair {
            base: "EUR".into(),
            quote: "USD".into(),
        })
    }

    fn fx_vanilla(strike: f64, expiry: f64) -> Instrument {
        Instrument {
            underlying: Some(eurusd()),
            expiry_years: expiry,
            side: Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::Vanilla(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(strike)),
                }),
            })),
            ..Default::default()
        }
    }

    fn single_barrier() -> Instrument {
        Instrument {
            underlying: Some(eurusd()),
            expiry_years: 1.0,
            side: Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::SingleBarrier(celnet_proto::SingleBarrier {
                vanilla: Some(Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(StrikeOrDelta {
                        spec: Some(strike_or_delta::Spec::Strike(1.10)),
                    }),
                }),
                kind: celnet_proto::BarrierKind::KnockOut as i32,
                side: celnet_proto::BarrierSide::Down as i32,
                barrier: 0.95,
                rebate: 0.0,
                monitoring: celnet_proto::MonitoringStyle::Continuous as i32,
            })),
            ..Default::default()
        }
    }

    fn ca_market(spot: f64, vol: f64, r: f64, b: f64) -> MarketContext {
        MarketContext {
            spot,
            vol,
            discount_rate: r,
            carry: Some(CarryModel {
                model: Some(carry_model::Model::Generalized(CostOfCarry { b })),
            }),
        }
    }

    fn equity_vanilla(strike: f64) -> Instrument {
        Instrument {
            underlying: Some(celnet_proto::Underlying::equity(EquityRef::new(
                Symbol::new("AAPL", "XNAS"),
                "USD",
            ))),
            expiry_years: 1.0,
            side: Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::Vanilla(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(strike)),
                }),
            })),
            ..Default::default()
        }
    }

    fn commodity_vanilla(strike: f64) -> Instrument {
        Instrument {
            underlying: Some(celnet_proto::Underlying::commodity(CommodityRef::new(
                Symbol::new("BRENT", ""),
                "USD",
            ))),
            expiry_years: 0.75,
            side: Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::Vanilla(Vanilla {
                option_type: celnet_proto::OptionType::Put as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(strike)),
                }),
            })),
            ..Default::default()
        }
    }

    fn crypto_vanilla(quote: &str, settlement: SettlementStyle, strike: f64) -> Instrument {
        Instrument {
            underlying: Some(celnet_proto::Underlying::digital_asset(CryptoPair::new(
                "BTC", quote,
            ))),
            expiry_years: 0.5,
            side: Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            settlement_style: settlement as i32,
            product: Some(Product::Vanilla(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(strike)),
                }),
            })),
            ..Default::default()
        }
    }

    /// THE C1 GATE (FX / exotic / cross-asset): the unified [`PricingEngine::price`]
    /// is `to_bits`-identical to the pre-C1 dispatch it replaced — the plugin-host
    /// [`engines::dispatch_live`] terminal for FX/analytic (unchanged) and the frozen
    /// native `price_cross_asset` for the cross-asset arm (the retired duplicate,
    /// kept as the independent oracle) — across every asset class + product family.
    #[test]
    fn unified_engine_matches_pre_c1_dispatch() {
        let conv = conv();

        // FX vanilla + FX exotic: the engine's terminal is the unchanged
        // `dispatch_live`. Compare the engine against that terminal directly (proves
        // the FX/analytic routing is the verbatim plugin-host dispatch).
        let fx_market = MarketContext::fx(1.10, 0.10, 0.02, 0.01);
        for (label, instr) in [
            ("fx_vanilla", fx_vanilla(1.12, 1.0)),
            ("fx_vanilla_otm", fx_vanilla(1.30, 0.5)),
            ("fx_single_barrier", single_barrier()),
        ] {
            let product = instr.product.as_ref().unwrap();
            let want =
                engines::dispatch_live(product, &instr, &fx_market, instr.expiry_years, &conv)
                    .unwrap_or_else(|e| panic!("[{label}] dispatch_live must price: {e:?}"));
            let got = PricingEngine::price(&instr, &fx_market, &conv)
                .unwrap_or_else(|e| panic!("[{label}] engine must price: {e:?}"));
            assert_priced_bit_identical(&got, &want, label);
        }

        // Cross-asset: the engine routes through the cost-of-carry `Priceable` leaves.
        // Compare against the frozen pre-C1 native `super::super::price_cross_asset`
        // dispatch (the retired duplicate, preserved as the independent oracle).
        let cross_grid: Vec<(&str, Instrument, MarketContext)> = vec![
            (
                "equity",
                equity_vanilla(105.0),
                ca_market(100.0, 0.20, 0.05, 0.02),
            ),
            (
                "equity_itm",
                equity_vanilla(90.0),
                ca_market(100.0, 0.35, 0.03, -0.01),
            ),
            (
                "commodity",
                commodity_vanilla(80.0),
                ca_market(85.0, 0.30, 0.04, 0.0),
            ),
            (
                "crypto_linear",
                crypto_vanilla("USDT", SettlementStyle::Linear, 32_000.0),
                ca_market(30_000.0, 0.65, 0.05, 0.03),
            ),
            (
                "crypto_inverse",
                crypto_vanilla("USD", SettlementStyle::InverseCoin, 28_000.0),
                ca_market(30_000.0, 0.80, 0.02, -0.02),
            ),
        ];
        for (label, instr, market) in cross_grid {
            let wire_underlying = instr.underlying.as_ref().unwrap();
            let underlying = Underlying::try_from(wire_underlying.clone()).unwrap();
            let product = instr.product.as_ref().unwrap();
            let want = super::super::price_cross_asset(
                &underlying,
                &instr,
                product,
                &market,
                instr.expiry_years,
            )
            .unwrap_or_else(|e| panic!("[{label}] frozen native oracle must price: {e:?}"));
            let got = PricingEngine::price(&instr, &market, &conv)
                .unwrap_or_else(|e| panic!("[{label}] engine must price: {e:?}"));
            assert_priced_bit_identical(&got, &want, label);
        }
    }

    fn rates_request(tenor: u32, fixed_rate: f64, notional: f64, side: Side) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(CurveSet {
                currency: "USD".to_string(),
                reference_date: Some(BrokenDate {
                    year: 2026,
                    month: 6,
                    day: 25,
                }),
                ois_pillars: vec![
                    OisPillar {
                        tenor: Some(PillarTenor {
                            point: Some(pillar_tenor::Point::Years(1)),
                        }),
                        par_rate: 0.0420,
                    },
                    OisPillar {
                        tenor: Some(PillarTenor {
                            point: Some(pillar_tenor::Point::Years(2)),
                        }),
                        par_rate: 0.0410,
                    },
                    OisPillar {
                        tenor: Some(PillarTenor {
                            point: Some(pillar_tenor::Point::Years(5)),
                        }),
                        par_rate: 0.0405,
                    },
                    OisPillar {
                        tenor: Some(PillarTenor {
                            point: Some(pillar_tenor::Point::Years(10)),
                        }),
                        par_rate: 0.0415,
                    },
                ],
            }),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: tenor,
                    fixed_rate,
                    notional,
                    side: side as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    /// THE C1 GATE (linear FI): the unified [`PricingEngine::price_rates`] is
    /// `to_bits`-identical to the pre-C1 [`crate::rates_pricing::price_rates`]
    /// dispatch it now fronts — PV, par, PV01, DV01, and the full key-rate ladder —
    /// for both receive-fixed and pay-fixed sides across tenors.
    #[test]
    fn unified_engine_rates_matches_pre_c1_dispatch() {
        for &(tenor, fixed, notional) in &[
            (5u32, 0.04, 100_000_000.0),
            (7, 0.041, 50_000_000.0),
            (2, 0.0410, 25_000_000.0),
        ] {
            for side in [Side::Sell, Side::Buy] {
                let req = rates_request(tenor, fixed, notional, side);
                let want = price_rates(&req).expect("pre-C1 rates dispatch prices");
                let got = PricingEngine::price_rates(&req).expect("engine rates prices");
                assert_eq!(got.pv.to_bits(), want.pv.to_bits(), "pv drifted");
                assert_eq!(
                    got.par_rate.to_bits(),
                    want.par_rate.to_bits(),
                    "par_rate drifted"
                );
                assert_eq!(got.pv01.to_bits(), want.pv01.to_bits(), "pv01 drifted");
                assert_eq!(got.dv01.to_bits(), want.dv01.to_bits(), "dv01 drifted");
                assert_eq!(
                    got.key_rate_ladder.len(),
                    want.key_rate_ladder.len(),
                    "ladder length drifted"
                );
                for (i, (a, b)) in got
                    .key_rate_ladder
                    .iter()
                    .zip(&want.key_rate_ladder)
                    .enumerate()
                {
                    assert_eq!(a.to_bits(), b.to_bits(), "key_rate[{i}] drifted");
                }
            }
        }
    }
}
