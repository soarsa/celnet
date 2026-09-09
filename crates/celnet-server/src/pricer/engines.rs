//! Per-product-family pricing engines: the [`ProductEngine`] registry behind
//! [`super::price_instrument`].
//!
//! Architecture item C (`docs/plan/NEXT-ARCHITECTURE-IMPLEMENTATION.md`):
//! `price_instrument` was a ~1200-line product `match` over the
//! [`celnet_proto::instrument::Product`] oneof. That god-`match` is now a thin
//! decode → guard → **dispatch**: each product family is a unit-struct
//! [`ProductEngine`] whose [`ProductEngine::price`] holds the family's pricing
//! body, and `price_instrument` dispatches the decoded oneof variant to the
//! corresponding engine with a single line per arm.
//!
//! # Byte-identity
//!
//! This is a **pure structural** refactor: every engine body is the verbatim
//! pre-refactor `match`-arm body (same engine calls, same finite-difference
//! closure construction order, same Monte-Carlo seed/`cfg` sourcing/defaulting,
//! same `Priced` construction). No priced quantity changes by a single bit; the
//! parity/determinism corpus is the gate. The dispatch is a static `match` on
//! the product discriminant (zero-allocation, no heap registry) — the hot path
//! stays allocation-free.
//!
//! The guards `price_instrument` keeps ahead of dispatch (perpetual term-shape →
//! asset-class route → FX-carry guard → LSV booking-model branch) are unchanged;
//! the engines below price the FX/analytic product path only.

use celnet_proto::{Instrument, MarketContext as WireMarketContext, instrument};
use celnet_types::{Carry, Greeks};

use celnet_core::{CarryInputs, FlatSmile};
use celnet_exotics::{
    Accumulator as ExAccumulator, AccumulatorMcConfig, AmericanGrid, AmericanOption as ExAmerican,
    AnalyticAsian, AveragingSchedule, BarrierKind as ExBarrierKind, BarrierStyle,
    BasketKind as ExBasketKind, BasketLeg as ExBasketLeg, BasketMcConfig, BasketSpec, Cliquet,
    CliquetMcConfig, CliquetSchedule, DigitalKind, DoubleBarrierKnockOut, DoubleNoTouch,
    ExerciseStyle as ExExerciseStyle, ForwardStart, Lookback as ExLookback, LookbackMcConfig,
    LookbackStyle as ExLookbackStyle, LsmConfig, Monitoring as ExMonitoring, PivotTra,
    PivotTraMcConfig, QuantoParams, RebateTiming, RedemptionStyle as ExRedemptionStyle,
    SingleBarrier as ExSingleBarrier, Tarf as ExTarf, TarfMcConfig, accumulator_price,
    american_fd_greeks, american_lsm, cliquet_price_capped_mc, cliquet_price_plain, curran_price,
    digital_price, double_knock_out_price, double_no_touch_price, double_touch_price,
    fair_variance, fair_volatility, fixed_lookback_price, floating_lookback_price,
    forward_start_price, lookback_mc, no_touch_price, one_touch_price, pivot_tra_price,
    price_basket, quanto_digital_price, quanto_vanilla_price, single_barrier_price, tarf_price,
    turnbull_wakeman_price,
};
use celnet_plugin_api::{ExoticArchetype, ExoticPayoffDescriptor, MultiAssetInputs};
use celnet_plugin_host::{ExoticHostModel, HostModel, ModelRegistry};

use super::{
    DEFAULT_ACCUMULATOR_MC_PAIRS, DEFAULT_BASKET_MC_PATHS, DEFAULT_BASKET_MC_REPLICATIONS,
    DEFAULT_BASKET_MC_STEPS, DEFAULT_CLIQUET_MC_PAIRS, DEFAULT_LOOKBACK_MC_PAIRS,
    DEFAULT_LOOKBACK_OBSERVATIONS, DEFAULT_PIVOT_MC_PAIRS, DEFAULT_TARF_MC_PAIRS, ExoticPrice,
    PriceError, Priced, ZERO_GREEKS, add_scaled, decode_option_type, exotic_greeks,
    exotic_inputs_at, fx_wire_greeks, price_fx_forward, price_fx_swap,
    price_listed_future_option, price_ndf, price_perpetual, price_vanilla_leg, resolve_strike,
    var_swap_context,
};

/// The shared, decoded per-request inputs every [`ProductEngine`] reads: the wire
/// instrument, its market context, the resolved positive expiry (years), and the
/// decoded conventions. Borrowed for the duration of the dispatch — no allocation.
pub(super) struct EngineCtx<'a> {
    /// The wire instrument being priced.
    pub instrument: &'a Instrument,
    /// The market context the instrument is priced against.
    pub market: &'a WireMarketContext,
    /// The instrument's expiry in years (already term-shape-validated > 0).
    pub expiry: f64,
    /// The decoded trade conventions.
    pub conv: &'a super::ConventionSet,
    /// The calling pricing worker's installed house-model registry (architecture
    /// item D — dormant-crate activation). When a pricing model is registered, the
    /// analytic vanilla arm prices through it via the plugin-host
    /// [`ModelRegistry`] — priced *as a* [`ProductEngine`], the same decode →
    /// guard → dispatch seam as a native arm. `None` (the default) leaves every
    /// arm on the verbatim native static path (byte-identical, zero-allocation).
    /// Borrowed for the dispatch only; the registry is per-worker (`!Sync` by
    /// design — a model handle is owned by one worker), never shared across
    /// threads.
    pub plugin_models: Option<&'a ModelRegistry>,
}

/// One product family's pricing engine. `Product` is the decoded oneof payload
/// the dispatch hands the engine; [`price`](ProductEngine::price) holds the
/// family's pricing body verbatim. Implemented by a zero-sized unit struct per
/// family so dispatch is a static call with no allocation.
pub(super) trait ProductEngine {
    /// The decoded product oneof payload this engine prices.
    type Product;
    /// Price the decoded product against the shared engine context.
    ///
    /// # Errors
    ///
    /// [`PriceError`] for a malformed product, an unknown enum tag, or an
    /// out-of-domain input — identical to the pre-refactor arm.
    fn price(&self, product: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError>;
}

/// The closed-form European vanilla / FX option.
pub(super) struct VanillaEngine;
impl ProductEngine for VanillaEngine {
    type Product = celnet_proto::Vanilla;
    fn price(&self, v: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry, conv) = (ctx.market, ctx.expiry, ctx.conv);
        let option_type = decode_option_type(v.option_type)?;
        let spec = v
            .strike
            .as_ref()
            .and_then(|s| s.spec.as_ref())
            .ok_or(PriceError::MissingField("vanilla.strike"))?;
        let strike = resolve_strike(spec, market, expiry, conv, option_type)?;
        Ok(price_vanilla_leg(option_type, strike, market, expiry))
    }
}

/// Prices the analytic vanilla arm through a **registered house pricing model**
/// resolved from the plugin-host [`ModelRegistry`], rather than the built-in
/// closed form — the activation of the otherwise-dormant `celnet-plugin-host`
/// crate (architecture item D). A registered model is priced *as a*
/// [`ProductEngine`], so it flows through the identical decode → guard → dispatch
/// seam as a native arm: same strike resolution, same [`Priced`] shape.
///
/// The model receives the generalized carry-tagged [`CarryInputs`] for the FX
/// two-rate arm — exactly the `(spot, strike, vol, t, r_dom, r_for)` the native
/// [`VanillaEngine`] prices off — and returns the carry-tagged
/// [`celnet_core::CarryGreeks`], mapped onto the wire [`Greeks`] by the single
/// source of the carry↔flat-rho bijection ([`super::carry_greeks_to_greeks`]).
/// For the reference FX model this is byte-identical to the native Garman-
/// Kohlhagen strip; a desk's own model legitimately differs. Selecting a house
/// model touches only this arm — the other product arms stay static dispatch.
pub(super) struct PluginModelEngine<'r> {
    /// The resolved house pricing-model handle (tier-blind: native or sandboxed).
    model: &'r dyn HostModel,
    /// The decoded FX/metal underlying handed to the model so it can confirm the
    /// asset class it prices (the FX two-rate arm).
    underlying: celnet_types::Underlying,
}

// Production builds construct `PluginModelEngine` inline in [`dispatch`]; this
// constructor exists only so the `Priceable` re-seat test ([`super::contract`])
// can instantiate the plugin engine directly.
#[cfg(test)]
impl<'r> PluginModelEngine<'r> {
    /// Build a plugin-model engine over a resolved house-model handle and the
    /// decoded FX/metal underlying the model prices.
    pub(super) fn new(model: &'r dyn HostModel, underlying: celnet_types::Underlying) -> Self {
        Self { model, underlying }
    }
}

impl ProductEngine for PluginModelEngine<'_> {
    type Product = celnet_proto::Vanilla;
    fn price(&self, v: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let option_type = decode_option_type(v.option_type)?;
        let spec = v
            .strike
            .as_ref()
            .and_then(|s| s.spec.as_ref())
            .ok_or(PriceError::MissingField("vanilla.strike"))?;
        let strike = resolve_strike(spec, market, expiry, ctx.conv, option_type)?;
        // The generalized carry-tagged inputs for the FX two-rate arm — the same
        // `(spot, strike, vol, t, r_dom, r_for)` the native vanilla path prices
        // off. The FX `Carry` arm reproduces the FX two-rate arithmetic
        // bit-for-bit (proven in `celnet-core`), so the reference model stays FX
        // byte-identical; a desk's own model is free to differ.
        let inputs = CarryInputs {
            spot: market.spot,
            strike,
            vol: market.vol,
            t: expiry,
            underlying: self.underlying.clone(),
            carry: Carry::FxRates {
                r_dom: market.r_dom(),
                r_for: market.r_for(),
            },
        };
        // The registered model prices and returns the full carry-tagged Greek
        // strip in one pass. A model-domain failure is surfaced as a typed domain
        // error at the boundary (mapped to INVALID_ARGUMENT) — never a silent
        // fallback to the built-in form. (Richer plugin-error propagation rides
        // the canonical error→Status taxonomy, owned by a separate lane.)
        let carry_greeks = self
            .model
            .price_and_greeks(option_type, &inputs)
            .map_err(|_| {
                PriceError::Domain("registered pricing model could not price this instrument")
            })?;
        Ok(Priced {
            greeks: super::carry_greeks_to_greeks(&carry_greeks),
            resolved_strike: strike,
            vol: market.vol,
            std_error: None,
        })
    }
}

/// Exotic plugin pricer adapter bridging the unified [`ModelRegistry`] to the exotic dispatch seam.
pub(super) struct ExoticPluginModelEngine<'r> {
    model: &'r dyn ExoticHostModel,
    underlying: celnet_types::Underlying,
}

impl<'r> ExoticPluginModelEngine<'r> {
    /// Wrap a registered exotic host model.
    pub(super) fn new(
        model: &'r dyn ExoticHostModel,
        underlying: celnet_types::Underlying,
    ) -> Self {
        Self { model, underlying }
    }

    /// Price a single barrier instrument through the registered exotic model.
    pub(super) fn price_single_barrier(
        &self,
        b: &celnet_proto::SingleBarrier,
        ctx: &EngineCtx<'_>,
    ) -> Result<Priced, PriceError> {
        let (market, expiry, conv) = (ctx.market, ctx.expiry, ctx.conv);
        let v = b
            .vanilla
            .as_ref()
            .ok_or(PriceError::MissingField("single_barrier.vanilla"))?;
        let option_type = decode_option_type(v.option_type)?;
        let spec = v
            .strike
            .as_ref()
            .and_then(|s| s.spec.as_ref())
            .ok_or(PriceError::MissingField("single_barrier.vanilla.strike"))?;
        let strike = resolve_strike(spec, market, expiry, conv, option_type)?;
        let bside =
            celnet_proto::BarrierSide::try_from(b.side).map_err(|_| PriceError::UnknownEnum {
                kind: "BarrierSide",
                tag: b.side,
            })?;
        let up = matches!(bside, celnet_proto::BarrierSide::Up);

        let payoff = ExoticPayoffDescriptor {
            archetype: ExoticArchetype::Barrier,
            strike,
            upper_barrier: if up { Some(b.barrier) } else { None },
            lower_barrier: if !up { Some(b.barrier) } else { None },
            rebate: b.rebate,
            weights: vec![1.0],
        };
        let numeraire = self
            .underlying
            .as_ccy_pair()
            .map(|p| p.quote)
            .unwrap_or_else(|| celnet_types::Ccy::parse("USD").unwrap());
        let inputs = MultiAssetInputs {
            underlyings: vec![self.underlying.clone()],
            spots: vec![market.spot],
            vols: vec![market.vol],
            correlation_matrix: vec![1.0],
            expiry_years: expiry,
            observation_schedule: vec![],
            past_fixings: vec![],
            numeraire,
        };

        match self.model.price_exotic(&payoff, &inputs) {
            Ok(pv) => {
                let deltas = self.model.deltas(&payoff, &inputs).unwrap_or_default();
                let delta_spot = deltas.first().copied().unwrap_or(0.0);
                Ok(Priced {
                    greeks: Greeks {
                        price: pv,
                        delta_spot,
                        ..ZERO_GREEKS
                    },
                    resolved_strike: strike,
                    vol: market.vol,
                    std_error: None,
                })
            }
            Err(_) => SingleBarrierEngine.price(b, ctx),
        }
    }
}

/// A multi-leg strategy: the signed-ratio Greek sum of its vanilla legs.
pub(super) struct StrategyEngine;
impl ProductEngine for StrategyEngine {
    type Product = celnet_proto::Strategy;
    fn price(&self, s: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry, conv) = (ctx.market, ctx.expiry, ctx.conv);
        if s.legs.is_empty() {
            return Err(PriceError::Domain("strategy must carry at least one leg"));
        }
        // The structure's Greeks are the signed-ratio sum of its legs. The
        // headline resolved strike / vol is the first leg's (display anchor).
        let mut acc = ZERO_GREEKS;
        let mut head_strike = 0.0;
        let mut head_vol = market.vol;
        for (idx, leg) in s.legs.iter().enumerate() {
            let option_type = decode_option_type(leg.option_type)?;
            let spec = leg
                .strike
                .as_ref()
                .and_then(|sp| sp.spec.as_ref())
                .ok_or(PriceError::MissingField("strategy.leg.strike"))?;
            let strike = resolve_strike(spec, market, expiry, conv, option_type)?;
            let side =
                celnet_proto::Side::try_from(leg.side).map_err(|_| PriceError::UnknownEnum {
                    kind: "Side",
                    tag: leg.side,
                })?;
            let sign = match side {
                celnet_proto::Side::Sell => -1.0,
                _ => 1.0,
            };
            let w = sign * leg.ratio;
            let leg_priced = price_vanilla_leg(option_type, strike, market, expiry);
            acc = add_scaled(&acc, &leg_priced.greeks, w);
            if idx == 0 {
                head_strike = strike;
                head_vol = leg_priced.vol;
            }
        }
        Ok(Priced {
            greeks: acc,
            resolved_strike: head_strike,
            vol: head_vol,
            std_error: None,
        })
    }
}

/// A single-barrier knock-in / knock-out option (FD Greek strip).
pub(super) struct SingleBarrierEngine;
impl ProductEngine for SingleBarrierEngine {
    type Product = celnet_proto::SingleBarrier;
    fn price(&self, b: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry, conv) = (ctx.market, ctx.expiry, ctx.conv);
        let v = b
            .vanilla
            .as_ref()
            .ok_or(PriceError::MissingField("single_barrier.vanilla"))?;
        let option_type = decode_option_type(v.option_type)?;
        let spec = v
            .strike
            .as_ref()
            .and_then(|s| s.spec.as_ref())
            .ok_or(PriceError::MissingField("single_barrier.vanilla.strike"))?;
        let strike = resolve_strike(spec, market, expiry, conv, option_type)?;
        let kind =
            celnet_proto::BarrierKind::try_from(b.kind).map_err(|_| PriceError::UnknownEnum {
                kind: "BarrierKind",
                tag: b.kind,
            })?;
        let bside =
            celnet_proto::BarrierSide::try_from(b.side).map_err(|_| PriceError::UnknownEnum {
                kind: "BarrierSide",
                tag: b.side,
            })?;
        let up = matches!(bside, celnet_proto::BarrierSide::Up);
        let style = match kind {
            celnet_proto::BarrierKind::KnockIn => BarrierStyle::KnockIn,
            celnet_proto::BarrierKind::KnockOut => BarrierStyle::KnockOut,
        };
        let barrier = b.barrier;
        let rebate = b.rebate;
        let ex_spec = ExSingleBarrier {
            kind: ExBarrierKind {
                up,
                style,
                option: option_type,
            },
            strike,
            barrier,
            rebate,
        };
        let price = |m: &WireMarketContext| {
            single_barrier_price(&exotic_inputs_at(m, expiry, strike, m.vol), ex_spec)
        };
        let price_at = |t: f64, m: &WireMarketContext| {
            single_barrier_price(&exotic_inputs_at(m, t, strike, m.vol), ex_spec)
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: None,
        })
    }
}

/// A double-barrier knock-out (knock-in priced by in-out parity).
pub(super) struct DoubleBarrierEngine;
impl ProductEngine for DoubleBarrierEngine {
    type Product = celnet_proto::DoubleBarrier;
    fn price(&self, b: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry, conv) = (ctx.market, ctx.expiry, ctx.conv);
        let v = b
            .vanilla
            .as_ref()
            .ok_or(PriceError::MissingField("double_barrier.vanilla"))?;
        let option_type = decode_option_type(v.option_type)?;
        let spec = v
            .strike
            .as_ref()
            .and_then(|s| s.spec.as_ref())
            .ok_or(PriceError::MissingField("double_barrier.vanilla.strike"))?;
        let strike = resolve_strike(spec, market, expiry, conv, option_type)?;
        if !(b.lower_barrier > 0.0 && b.lower_barrier < b.upper_barrier) {
            return Err(PriceError::Domain(
                "double_barrier corridor must satisfy 0 < lower < upper",
            ));
        }
        let kind =
            celnet_proto::BarrierKind::try_from(b.kind).map_err(|_| PriceError::UnknownEnum {
                kind: "BarrierKind",
                tag: b.kind,
            })?;
        let ko = DoubleBarrierKnockOut::new(option_type, strike, b.lower_barrier, b.upper_barrier);
        // A double knock-in is priced by in-out parity: KI = vanilla − KO.
        let knock_in = matches!(kind, celnet_proto::BarrierKind::KnockIn);
        let price = move |m: &WireMarketContext| {
            let ki = exotic_inputs_at(m, expiry, strike, m.vol);
            let ko_px = double_knock_out_price(&ki, ko);
            if knock_in {
                celnet_core::gbsm_carry_price(
                    option_type,
                    ki.carry_rate(),
                    ki.discount_rate(),
                    ki.spot,
                    ki.strike,
                    ki.vol,
                    ki.t,
                ) - ko_px
            } else {
                ko_px
            }
        };
        let price_at = move |t: f64, m: &WireMarketContext| {
            let ki = exotic_inputs_at(m, t, strike, m.vol);
            let ko_px = double_knock_out_price(&ki, ko);
            if knock_in {
                celnet_core::gbsm_carry_price(
                    option_type,
                    ki.carry_rate(),
                    ki.discount_rate(),
                    ki.spot,
                    ki.strike,
                    ki.vol,
                    ki.t,
                ) - ko_px
            } else {
                ko_px
            }
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: None,
        })
    }
}

/// A cash-or-nothing / asset-or-nothing digital option.
pub(super) struct DigitalEngine;
impl ProductEngine for DigitalEngine {
    type Product = celnet_proto::Digital;
    fn price(&self, d: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let option_type = decode_option_type(d.option_type)?;
        let style =
            celnet_proto::DigitalStyle::try_from(d.style).map_err(|_| PriceError::UnknownEnum {
                kind: "DigitalStyle",
                tag: d.style,
            })?;
        let kind = match style {
            celnet_proto::DigitalStyle::CashOrNothing => DigitalKind::cash(option_type),
            celnet_proto::DigitalStyle::AssetOrNothing => DigitalKind::asset(option_type),
        };
        let strike = d.strike;
        let payout = d.payout;
        let price = move |m: &WireMarketContext| {
            payout * digital_price(kind, &exotic_inputs_at(m, expiry, strike, m.vol))
        };
        let price_at = move |t: f64, m: &WireMarketContext| {
            payout * digital_price(kind, &exotic_inputs_at(m, t, strike, m.vol))
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: None,
        })
    }
}

/// One-touch / no-touch / double-(no-)touch.
pub(super) struct TouchEngine;
impl ProductEngine for TouchEngine {
    type Product = celnet_proto::Touch;
    fn price(&self, t: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let kind =
            celnet_proto::TouchKind::try_from(t.kind).map_err(|_| PriceError::UnknownEnum {
                kind: "TouchKind",
                tag: t.kind,
            })?;
        let lower = t.lower_barrier;
        let upper = t.upper_barrier;
        let rebate = t.rebate;
        // Touches carry no strike; the headline strike echoes the lower
        // barrier so a display has a sensible level.
        let price: Box<ExoticPrice<'_>> = match kind {
            celnet_proto::TouchKind::OneTouch => Box::new(move |m: &WireMarketContext| {
                one_touch_price(
                    &exotic_inputs_at(m, expiry, lower, m.vol),
                    lower,
                    rebate,
                    RebateTiming::AtHit,
                )
            }),
            celnet_proto::TouchKind::NoTouch => Box::new(move |m: &WireMarketContext| {
                no_touch_price(&exotic_inputs_at(m, expiry, lower, m.vol), lower, rebate)
            }),
            celnet_proto::TouchKind::DoubleNoTouch => {
                if !(lower > 0.0 && lower < upper) {
                    return Err(PriceError::Domain(
                        "double-no-touch corridor must satisfy 0 < lower < upper",
                    ));
                }
                Box::new(move |m: &WireMarketContext| {
                    double_no_touch_price(
                        &exotic_inputs_at(m, expiry, lower, m.vol),
                        DoubleNoTouch::new(lower, upper, rebate),
                    )
                })
            }
            celnet_proto::TouchKind::DoubleOneTouch => {
                if !(lower > 0.0 && lower < upper) {
                    return Err(PriceError::Domain(
                        "double-one-touch corridor must satisfy 0 < lower < upper",
                    ));
                }
                Box::new(move |m: &WireMarketContext| {
                    double_touch_price(
                        &exotic_inputs_at(m, expiry, lower, m.vol),
                        DoubleNoTouch::new(lower, upper, rebate),
                    )
                })
            }
        };
        // Re-derive the price-at-expiry closure mirroring the chosen kind for
        // the time Greeks (touches are time-homogeneous in the same form).
        let price_at = move |t_exp: f64, m: &WireMarketContext| -> f64 {
            match kind {
                celnet_proto::TouchKind::OneTouch => one_touch_price(
                    &exotic_inputs_at(m, t_exp, lower, m.vol),
                    lower,
                    rebate,
                    RebateTiming::AtHit,
                ),
                celnet_proto::TouchKind::NoTouch => {
                    no_touch_price(&exotic_inputs_at(m, t_exp, lower, m.vol), lower, rebate)
                }
                celnet_proto::TouchKind::DoubleNoTouch => double_no_touch_price(
                    &exotic_inputs_at(m, t_exp, lower, m.vol),
                    DoubleNoTouch::new(lower, upper, rebate),
                ),
                celnet_proto::TouchKind::DoubleOneTouch => double_touch_price(
                    &exotic_inputs_at(m, t_exp, lower, m.vol),
                    DoubleNoTouch::new(lower, upper, rebate),
                ),
            }
        };
        let greeks = exotic_greeks(price.as_ref(), &price_at, market, expiry);
        Ok(Priced {
            greeks,
            resolved_strike: lower,
            vol: market.vol,
            std_error: None,
        })
    }
}

/// A variance swap: the fair (annualised) variance strike + its FD sensitivities.
pub(super) struct VarianceSwapEngine;
impl ProductEngine for VarianceSwapEngine {
    type Product = celnet_proto::VarianceSwap;
    fn price(&self, _vs: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        // Fair (annualised) variance strike `K_var` by log-contract static
        // replication over the marked smile. The wire market context carries
        // a single Black vol, so the smile-consistent surface here is the
        // flat smile at that vol (a flat σ replicates to K_var = σ² exactly).
        // The headline `price` is the fair *variance* strike `K_var`; the
        // echoed `vol` is its realised-vol equivalent `√K_var`. The FD Greek
        // set is the sensitivity of the fair strike to spot/vol/rates/time.
        let price = move |m: &WireMarketContext| -> f64 {
            let ctx = var_swap_context(m, expiry);
            fair_variance(&FlatSmile::new(m.vol), &ctx).fair_variance
        };
        let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
            let ctx = var_swap_context(m, t);
            fair_variance(&FlatSmile::new(m.vol), &ctx).fair_variance
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        let fair_var = greeks.price;
        Ok(Priced {
            greeks,
            resolved_strike: fair_var,
            vol: fair_var.max(0.0).sqrt(),
            std_error: None,
        })
    }
}

/// A volatility swap: the fair vol strike (Carr-Lee convexity adjustment).
pub(super) struct VolatilitySwapEngine;
impl ProductEngine for VolatilitySwapEngine {
    type Product = celnet_proto::VolatilitySwap;
    fn price(&self, _vs: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        // Fair volatility strike `K_vol = √K_var − convexity_correction`
        // (Carr-Lee Jensen adjustment). The headline `price` is the fair vol
        // strike; under a flat smile the convexity gap is zero so K_vol = σ
        // exactly. FD Greeks are the fair-vol-strike sensitivities.
        let price = move |m: &WireMarketContext| -> f64 {
            let ctx = var_swap_context(m, expiry);
            fair_volatility(&FlatSmile::new(m.vol), &ctx).fair_vol
        };
        let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
            let ctx = var_swap_context(m, t);
            fair_volatility(&FlatSmile::new(m.vol), &ctx).fair_vol
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        let fair_vol = greeks.price;
        Ok(Priced {
            greeks,
            resolved_strike: fair_vol,
            vol: fair_vol,
            std_error: None,
        })
    }
}

/// An arithmetic Asian option (Curran / Turnbull-Wakeman closed form).
pub(super) struct AsianOptionEngine;
impl ProductEngine for AsianOptionEngine {
    type Product = celnet_proto::AsianOption;
    fn price(&self, a: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let option_type = decode_option_type(a.option_type)?;
        let averaging = celnet_proto::AveragingStyle::try_from(a.averaging).map_err(|_| {
            PriceError::UnknownEnum {
                kind: "AveragingStyle",
                tag: a.averaging,
            }
        })?;
        let method =
            celnet_proto::AsianMethod::try_from(a.method).map_err(|_| PriceError::UnknownEnum {
                kind: "AsianMethod",
                tag: a.method,
            })?;
        let schedule = match averaging {
            celnet_proto::AveragingStyle::Discrete => {
                if a.observations < 1 {
                    return Err(PriceError::Domain(
                        "discrete Asian needs at least one future observation",
                    ));
                }
                AveragingSchedule::Discrete {
                    future_obs: a.observations as usize,
                }
            }
            celnet_proto::AveragingStyle::Continuous => AveragingSchedule::Continuous,
        };
        if !(a.elapsed_weight >= 0.0 && a.elapsed_weight < 1.0) {
            return Err(PriceError::Domain(
                "Asian elapsed_weight must lie in [0, 1)",
            ));
        }
        let strike = a.strike;
        let spec = AnalyticAsian {
            option: option_type,
            strike,
            schedule,
            t_start: 0.0,
            elapsed_avg: a.elapsed_avg,
            elapsed_weight: a.elapsed_weight,
        };
        // The arithmetic-Asian price is a genuine discounted option value, so
        // the standard FD Greek machinery applies exactly as for the other
        // exotic legs. `t_start` stays 0 (a fresh remaining window); the
        // realised running average enters via the seasoned-strike shift.
        let price = move |m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, expiry, strike, m.vol);
            match method {
                celnet_proto::AsianMethod::Curran => curran_price(&inputs, spec),
                celnet_proto::AsianMethod::TurnbullWakeman => turnbull_wakeman_price(&inputs, spec),
            }
        };
        let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, t, strike, m.vol);
            match method {
                celnet_proto::AsianMethod::Curran => curran_price(&inputs, spec),
                celnet_proto::AsianMethod::TurnbullWakeman => turnbull_wakeman_price(&inputs, spec),
            }
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: None,
        })
    }
}

/// A forward-start (Rubinstein dual-carry) option.
pub(super) struct ForwardStartEngine;
impl ProductEngine for ForwardStartEngine {
    type Product = celnet_proto::ForwardStart;
    fn price(&self, fs: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let option = decode_option_type(fs.option_type)?;
        if !(fs.reset >= 0.0 && expiry >= fs.reset) {
            return Err(PriceError::Domain(
                "forward-start reset must satisfy 0 ≤ reset ≤ expiry",
            ));
        }
        let moneyness = fs.moneyness;
        let reset = fs.reset;
        // Closed-form FX dual-carry forward-start (Rubinstein). The strike is
        // reset-determined, so `inputs.strike` is unused by the pricer; the
        // resolved strike echoed is the ATM-forward reset level `m·F(t₁)`
        // (informational — the contract strikes at `reset`). The full Greek
        // strip is FD over the closed form.
        let price = move |m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, expiry, m.spot, m.vol);
            forward_start_price(
                &inputs,
                ForwardStart {
                    option,
                    moneyness,
                    reset,
                    expiry,
                },
            )
        };
        let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, t, m.spot, m.vol);
            forward_start_price(
                &inputs,
                ForwardStart {
                    option,
                    // Clamp the reset to the (bumped) residual maturity so the
                    // time-Greek FD never produces reset > expiry.
                    moneyness,
                    reset: reset.min(t),
                    expiry: t,
                },
            )
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        // The strike fixes at `reset` to `m·S(reset)`; echo the forward reset
        // level `m·F(t₁) = m·S₀·e^{(r_d−r_f)·t₁}` as the informational strike.
        let reset_strike =
            moneyness * market.spot * ((market.r_dom() - market.r_for()) * reset).exp();
        Ok(Priced {
            greeks,
            resolved_strike: reset_strike,
            vol: market.vol,
            std_error: None,
        })
    }
}

/// A cliquet / ratchet (plain closed form, clamped via Monte-Carlo).
pub(super) struct CliquetEngine;
impl ProductEngine for CliquetEngine {
    type Product = celnet_proto::Cliquet;
    fn price(&self, c: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let option = decode_option_type(c.option_type)?;
        if c.periods < 1 {
            return Err(PriceError::Domain("cliquet needs at least one period"));
        }
        let moneyness = c.moneyness;
        let periods = c.periods as usize;
        let build =
            move |m: &WireMarketContext, t: f64| -> (celnet_exotics::ExoticInputs, Cliquet) {
                let inputs = exotic_inputs_at(m, t, m.spot, m.vol);
                let spec = Cliquet {
                    option,
                    moneyness,
                    schedule: CliquetSchedule::equal(periods, t),
                    local_floor: c.local_floor,
                    local_cap: c.local_cap,
                    global_floor: c.global_floor,
                    global_cap: c.global_cap,
                };
                (inputs, spec)
            };
        let is_plain = build(market, expiry).1.is_plain();
        if is_plain {
            // Plain (unclamped) ratchet: exact closed form (Σ forward-start
            // legs). Full Greek strip via FD over the closed form.
            let price = move |m: &WireMarketContext| -> f64 {
                let (inputs, spec) = build(m, expiry);
                cliquet_price_plain(&inputs, &spec)
            };
            let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
                let (inputs, spec) = build(m, t);
                cliquet_price_plain(&inputs, &spec)
            };
            let greeks = exotic_greeks(&price, &price_at, market, expiry);
            Ok(Priced {
                greeks,
                resolved_strike: moneyness,
                vol: market.vol,
                std_error: None,
            })
        } else {
            // Clamped (locally-capped/floored or globally-bounded) cliquet:
            // priced by Monte-Carlo, honestly carrying its standard error. The
            // MC estimator is bit-reproducible from (seed, path), so the FD
            // Greek strip is a deterministic function of the market context
            // and the same seed — the differences are real, not RNG jitter.
            let pairs = if c.mc_pairs == 0 {
                DEFAULT_CLIQUET_MC_PAIRS
            } else {
                c.mc_pairs as usize
            };
            let cfg = CliquetMcConfig {
                pairs,
                seed: c.mc_seed,
            };
            let price = move |m: &WireMarketContext| -> f64 {
                let (inputs, spec) = build(m, expiry);
                cliquet_price_capped_mc(&inputs, &spec, cfg).price
            };
            let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
                let (inputs, spec) = build(m, t);
                cliquet_price_capped_mc(&inputs, &spec, cfg).price
            };
            let greeks = exotic_greeks(&price, &price_at, market, expiry);
            // The headline standard error is the base-context MC estimate's
            // std-error, surfaced honestly on the wire.
            let (inputs, spec) = build(market, expiry);
            let estimate = cliquet_price_capped_mc(&inputs, &spec, cfg);
            Ok(Priced {
                greeks,
                resolved_strike: moneyness,
                vol: market.vol,
                std_error: Some(estimate.std_error),
            })
        }
    }
}

/// A quanto-drift-adjusted vanilla / cash-or-nothing digital.
pub(super) struct QuantoEngine;
impl ProductEngine for QuantoEngine {
    type Product = celnet_proto::Quanto;
    fn price(&self, q: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let payoff = celnet_proto::QuantoPayoff::try_from(q.payoff).map_err(|_| {
            PriceError::UnknownEnum {
                kind: "QuantoPayoff",
                tag: q.payoff,
            }
        })?;
        let option = decode_option_type(q.option_type)?;
        if !(-1.0..=1.0).contains(&q.correlation) {
            return Err(PriceError::Domain("quanto correlation must lie in [-1, 1]"));
        }
        if q.conversion_vol < 0.0 {
            return Err(PriceError::Domain(
                "quanto conversion vol must be non-negative",
            ));
        }
        let strike = q.strike;
        let params = QuantoParams::new(q.conversion_vol, q.correlation);
        // Closed-form quanto-drift-adjusted vanilla / cash-or-nothing digital.
        // Full Greek strip via FD over the closed form.
        let price = move |m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, expiry, strike, m.vol);
            match payoff {
                celnet_proto::QuantoPayoff::Vanilla => {
                    quanto_vanilla_price(option, &inputs, params)
                }
                celnet_proto::QuantoPayoff::Digital => {
                    quanto_digital_price(option, &inputs, params)
                }
            }
        };
        let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, t, strike, m.vol);
            match payoff {
                celnet_proto::QuantoPayoff::Vanilla => {
                    quanto_vanilla_price(option, &inputs, params)
                }
                celnet_proto::QuantoPayoff::Digital => {
                    quanto_digital_price(option, &inputs, params)
                }
            }
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: None,
        })
    }
}

/// A Target-Redemption Forward (Monte-Carlo).
pub(super) struct TarfEngine;
impl ProductEngine for TarfEngine {
    type Product = celnet_proto::Tarf;
    fn price(&self, t: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let favourable_side = decode_option_type(t.option_type)?;
        let redemption = match celnet_proto::TarfRedemption::try_from(t.redemption) {
            Ok(celnet_proto::TarfRedemption::FullGain) => ExRedemptionStyle::FullGain,
            Ok(celnet_proto::TarfRedemption::CappedGain) => ExRedemptionStyle::CappedGain,
            Err(_) => {
                return Err(PriceError::UnknownEnum {
                    kind: "TarfRedemption",
                    tag: t.redemption,
                });
            }
        };
        let schedule = t
            .schedule
            .as_ref()
            .ok_or(PriceError::MissingField("tarf.schedule"))?;
        let fixings = schedule.fixing_years.len();
        if fixings < 1 {
            return Err(PriceError::Domain("TARF needs at least one fixing"));
        }
        if !t.target.is_finite() || t.target <= 0.0 {
            return Err(PriceError::Domain("TARF target must be positive"));
        }
        if t.leverage < 0.0 {
            return Err(PriceError::Domain("TARF leverage must be non-negative"));
        }
        if !schedule.fixing_notional.is_finite() || schedule.fixing_notional <= 0.0 {
            return Err(PriceError::Domain("TARF fixing notional must be positive"));
        }
        let strike = t.strike;
        let target = t.target;
        let leverage = t.leverage;
        let notional = schedule.fixing_notional;
        let cfg = TarfMcConfig {
            pairs: if t.mc_pairs == 0 {
                DEFAULT_TARF_MC_PAIRS
            } else {
                t.mc_pairs as usize
            },
            seed: t.mc_seed,
        };
        let spec_at = move |fx: usize| ExTarf {
            strike,
            fixings: fx,
            target,
            leverage,
            favourable_side,
            notional,
            redemption,
        };
        // The MC estimator is bit-reproducible from (seed, pairs), so the FD
        // Greek strip is a deterministic function of the market context — the
        // differences are real, not RNG jitter.
        let price = move |m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, expiry, m.spot, m.vol);
            tarf_price(&inputs, spec_at(fixings), cfg).price
        };
        let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, t, m.spot, m.vol);
            tarf_price(&inputs, spec_at(fixings), cfg).price
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        let inputs = exotic_inputs_at(market, expiry, market.spot, market.vol);
        let estimate = tarf_price(&inputs, spec_at(fixings), cfg);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: Some(estimate.std_error),
        })
    }
}

/// A pivot Target-Redemption Accumulator (Monte-Carlo).
pub(super) struct PivotEngine;
impl ProductEngine for PivotEngine {
    type Product = celnet_proto::Pivot;
    fn price(&self, p: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        // The pivot Target-Redemption Accumulator: the TARF mechanic with a
        // distinct pivot kink (`pivot == strike` ⇒ the exact TARF slice, the
        // engine's gated bit-identity). Decode/validity mirrors the TARF arm
        // as its sibling, plus the pivot level's own domain checks.
        let favourable_side = decode_option_type(p.option_type)?;
        let redemption = match celnet_proto::TarfRedemption::try_from(p.redemption) {
            Ok(celnet_proto::TarfRedemption::FullGain) => ExRedemptionStyle::FullGain,
            Ok(celnet_proto::TarfRedemption::CappedGain) => ExRedemptionStyle::CappedGain,
            Err(_) => {
                return Err(PriceError::UnknownEnum {
                    kind: "TarfRedemption",
                    tag: p.redemption,
                });
            }
        };
        let schedule = p
            .schedule
            .as_ref()
            .ok_or(PriceError::MissingField("pivot.schedule"))?;
        let fixings = schedule.fixing_years.len();
        if fixings < 1 {
            return Err(PriceError::Domain("pivot TRA needs at least one fixing"));
        }
        if !p.target.is_finite() || p.target <= 0.0 {
            return Err(PriceError::Domain("pivot TRA target must be positive"));
        }
        if p.leverage < 0.0 {
            return Err(PriceError::Domain(
                "pivot TRA leverage must be non-negative",
            ));
        }
        if !p.strike.is_finite() || p.strike <= 0.0 {
            return Err(PriceError::Domain("pivot TRA strike must be positive"));
        }
        if !p.pivot.is_finite() || p.pivot <= 0.0 {
            return Err(PriceError::Domain("pivot TRA pivot must be positive"));
        }
        if !schedule.fixing_notional.is_finite() || schedule.fixing_notional <= 0.0 {
            return Err(PriceError::Domain(
                "pivot TRA fixing notional must be positive",
            ));
        }
        let strike = p.strike;
        let pivot = p.pivot;
        let target = p.target;
        let leverage = p.leverage;
        let notional = schedule.fixing_notional;
        let cfg = PivotTraMcConfig {
            pairs: if p.mc_pairs == 0 {
                DEFAULT_PIVOT_MC_PAIRS
            } else {
                p.mc_pairs as usize
            },
            seed: p.mc_seed,
        };
        let spec_at = move |fx: usize| PivotTra {
            strike,
            pivot,
            fixings: fx,
            target,
            leverage,
            favourable_side,
            notional,
            redemption,
        };
        // The MC estimator is bit-reproducible from (seed, pairs), so the FD
        // Greek strip is a deterministic function of the market context — the
        // differences are real, not RNG jitter. The plain antithetic pricer
        // (not the control-variate entry point) keeps the `pivot == strike`
        // slice to_bits-identical to the TARF arm.
        let price = move |m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, expiry, m.spot, m.vol);
            pivot_tra_price(&inputs, spec_at(fixings), cfg).price
        };
        let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, t, m.spot, m.vol);
            pivot_tra_price(&inputs, spec_at(fixings), cfg).price
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        let inputs = exotic_inputs_at(market, expiry, market.spot, market.vol);
        let estimate = pivot_tra_price(&inputs, spec_at(fixings), cfg);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: Some(estimate.std_error),
        })
    }
}

/// An accumulator / decumulator (Monte-Carlo).
pub(super) struct AccumulatorEngine;
impl ProductEngine for AccumulatorEngine {
    type Product = celnet_proto::Accumulator;
    fn price(&self, a: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let monitoring = match celnet_proto::AccumulatorMonitoring::try_from(a.monitoring) {
            Ok(celnet_proto::AccumulatorMonitoring::Discrete) => ExMonitoring::Discrete,
            Ok(celnet_proto::AccumulatorMonitoring::Continuous) => ExMonitoring::Continuous,
            Err(_) => {
                return Err(PriceError::UnknownEnum {
                    kind: "AccumulatorMonitoring",
                    tag: a.monitoring,
                });
            }
        };
        let schedule = a
            .schedule
            .as_ref()
            .ok_or(PriceError::MissingField("accumulator.schedule"))?;
        let fixings = schedule.fixing_years.len();
        if fixings < 1 {
            return Err(PriceError::Domain("accumulator needs at least one fixing"));
        }
        if !(a.pivot.is_finite() && a.barrier.is_finite()) || a.barrier <= a.pivot {
            return Err(PriceError::Domain(
                "accumulator barrier must sit above the pivot",
            ));
        }
        if a.leverage < 0.0 {
            return Err(PriceError::Domain(
                "accumulator leverage must be non-negative",
            ));
        }
        if !schedule.fixing_notional.is_finite() || schedule.fixing_notional <= 0.0 {
            return Err(PriceError::Domain(
                "accumulator fixing notional must be positive",
            ));
        }
        let pivot = a.pivot;
        let barrier = a.barrier;
        let leverage = a.leverage;
        let notional = schedule.fixing_notional;
        let cfg = AccumulatorMcConfig {
            pairs: if a.mc_pairs == 0 {
                DEFAULT_ACCUMULATOR_MC_PAIRS
            } else {
                a.mc_pairs as usize
            },
            seed: a.mc_seed,
        };
        let spec = ExAccumulator {
            pivot,
            barrier,
            fixings,
            leverage,
            notional,
            monitoring,
        };
        let price = move |m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, expiry, m.spot, m.vol);
            accumulator_price(&inputs, spec, cfg).price
        };
        let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
            let inputs = exotic_inputs_at(m, t, m.spot, m.vol);
            accumulator_price(&inputs, spec, cfg).price
        };
        let greeks = exotic_greeks(&price, &price_at, market, expiry);
        let inputs = exotic_inputs_at(market, expiry, market.spot, market.vol);
        let estimate = accumulator_price(&inputs, spec, cfg);
        Ok(Priced {
            greeks,
            resolved_strike: pivot,
            vol: market.vol,
            std_error: Some(estimate.std_error),
        })
    }
}

/// A lookback (continuous closed form / discrete Monte-Carlo).
pub(super) struct LookbackEngine;
impl ProductEngine for LookbackEngine {
    type Product = celnet_proto::Lookback;
    fn price(&self, l: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let option = decode_option_type(l.option_type)?;
        let style = match celnet_proto::LookbackStyle::try_from(l.style) {
            Ok(celnet_proto::LookbackStyle::Floating) => ExLookbackStyle::FloatingStrike,
            Ok(celnet_proto::LookbackStyle::Fixed) => ExLookbackStyle::FixedStrike,
            Err(_) => {
                return Err(PriceError::UnknownEnum {
                    kind: "LookbackStyle",
                    tag: l.style,
                });
            }
        };
        let monitoring =
            celnet_proto::LookbackMonitoring::try_from(l.monitoring).map_err(|_| {
                PriceError::UnknownEnum {
                    kind: "LookbackMonitoring",
                    tag: l.monitoring,
                }
            })?;
        // The fixed-strike family prices against `l.strike`; the floating-strike
        // family settles against the path extremum (its `inputs.strike` is
        // unused by the closed form), so we echo the spot as the resolved strike.
        let strike = match style {
            ExLookbackStyle::FixedStrike => l.strike,
            ExLookbackStyle::FloatingStrike => market.spot,
        };
        match monitoring {
            celnet_proto::LookbackMonitoring::Continuous => {
                // Exact closed form (Goldman-Sosin-Gatto floating /
                // Conze-Viswanathan fixed) — no Monte-Carlo std-error.
                let price = move |m: &WireMarketContext| -> f64 {
                    let inputs = exotic_inputs_at(m, expiry, strike, m.vol);
                    match style {
                        ExLookbackStyle::FloatingStrike => floating_lookback_price(&inputs, option),
                        ExLookbackStyle::FixedStrike => fixed_lookback_price(&inputs, option),
                    }
                };
                let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
                    let inputs = exotic_inputs_at(m, t, strike, m.vol);
                    match style {
                        ExLookbackStyle::FloatingStrike => floating_lookback_price(&inputs, option),
                        ExLookbackStyle::FixedStrike => fixed_lookback_price(&inputs, option),
                    }
                };
                let greeks = exotic_greeks(&price, &price_at, market, expiry);
                Ok(Priced {
                    greeks,
                    resolved_strike: strike,
                    vol: market.vol,
                    std_error: None,
                })
            }
            celnet_proto::LookbackMonitoring::Discrete => {
                let steps = if l.observations == 0 {
                    DEFAULT_LOOKBACK_OBSERVATIONS
                } else {
                    l.observations as usize
                };
                let cfg = LookbackMcConfig {
                    pairs: if l.mc_pairs == 0 {
                        DEFAULT_LOOKBACK_MC_PAIRS
                    } else {
                        l.mc_pairs as usize
                    },
                    steps,
                    seed: l.mc_seed,
                };
                let spec = ExLookback { style, option };
                let price = move |m: &WireMarketContext| -> f64 {
                    let inputs = exotic_inputs_at(m, expiry, strike, m.vol);
                    lookback_mc(&inputs, spec, cfg).price
                };
                let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
                    let inputs = exotic_inputs_at(m, t, strike, m.vol);
                    lookback_mc(&inputs, spec, cfg).price
                };
                let greeks = exotic_greeks(&price, &price_at, market, expiry);
                let inputs = exotic_inputs_at(market, expiry, strike, market.vol);
                let estimate = lookback_mc(&inputs, spec, cfg);
                Ok(Priced {
                    greeks,
                    resolved_strike: strike,
                    vol: market.vol,
                    std_error: Some(estimate.std_error),
                })
            }
        }
    }
}

/// An American / Bermudan option (projected-SOR FD default; LSM Monte-Carlo).
pub(super) struct AmericanEngine;
impl ProductEngine for AmericanEngine {
    type Product = celnet_proto::AmericanOption;
    fn price(&self, a: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let option = decode_option_type(a.option_type)?;
        if !(a.strike.is_finite() && a.strike > 0.0) {
            return Err(PriceError::Domain("American strike must be positive"));
        }
        let exercise = celnet_proto::ExerciseStyle::try_from(a.exercise_style).map_err(|_| {
            PriceError::UnknownEnum {
                kind: "ExerciseStyle",
                tag: a.exercise_style,
            }
        })?;
        let style = match exercise {
            celnet_proto::ExerciseStyle::American => ExExerciseStyle::American,
            celnet_proto::ExerciseStyle::Bermudan => {
                // Validate the date set: every entry must be a finite
                // year-fraction in (0, expiry]; an empty Bermudan set is a
                // clear error (no exercise opportunity before expiry).
                if a.bermudan_dates.is_empty() {
                    return Err(PriceError::Domain(
                        "Bermudan option needs at least one exercise date",
                    ));
                }
                for d in &a.bermudan_dates {
                    if !(d.is_finite() && *d > 0.0 && *d <= expiry + 1e-12) {
                        return Err(PriceError::Domain(
                            "Bermudan exercise dates must lie in (0, expiry]",
                        ));
                    }
                }
                ExExerciseStyle::Bermudan {
                    dates: a.bermudan_dates.clone(),
                }
            }
        };
        let strike = a.strike;
        let spec = ExAmerican {
            option,
            strike,
            style,
        };
        let grid = AmericanGrid::default();
        if a.lsm_paths == 0 {
            // Default engine: projected-SOR free-boundary finite difference.
            // Exact to grid tolerance; the full Greek strip is central FD over
            // the FD price (each bumped axis re-solves the free boundary).
            let greeks = fx_wire_greeks(&american_fd_greeks(
                &exotic_inputs_at(market, expiry, strike, market.vol),
                &spec,
                grid,
            ));
            Ok(Priced {
                greeks,
                resolved_strike: strike,
                vol: market.vol,
                std_error: None,
            })
        } else {
            // Longstaff-Schwartz regression Monte-Carlo engine: the price and
            // its honest standard error come from the simulation; the Greek
            // strip is taken from the deterministic FD engine (an MC-FD-of-MC
            // risk would be dominated by simulation noise — the FD risk is the
            // sound choice, and the FD/LSM prices agree within stderr).
            let cfg = LsmConfig {
                paths: a.lsm_paths as usize,
                exercise_dates: if a.lsm_exercise_dates == 0 {
                    LsmConfig::default().exercise_dates
                } else {
                    a.lsm_exercise_dates as usize
                },
                seed: a.lsm_seed,
            };
            let inputs = exotic_inputs_at(market, expiry, strike, market.vol);
            let estimate = american_lsm(&inputs, &spec, cfg);
            let mut greeks = fx_wire_greeks(&american_fd_greeks(&inputs, &spec, grid));
            // Report the LSM price (with its std-error) as the headline; the
            // FD-derived risk sensitivities ride alongside.
            greeks.price = estimate.price;
            Ok(Priced {
                greeks,
                resolved_strike: strike,
                vol: market.vol,
                std_error: Some(estimate.std_error),
            })
        }
    }
}

/// A multi-asset basket / best-of / worst-of (scrambled-Sobol Monte-Carlo).
pub(super) struct BasketEngine;
impl ProductEngine for BasketEngine {
    type Product = celnet_proto::BasketOption;
    fn price(&self, b: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let (market, expiry) = (ctx.market, ctx.expiry);
        let option_type = decode_option_type(b.option_type)?;
        let kind = match celnet_proto::BasketKind::try_from(b.kind) {
            Ok(celnet_proto::BasketKind::Basket) => ExBasketKind::Basket,
            Ok(celnet_proto::BasketKind::BestOf) => ExBasketKind::BestOf,
            Ok(celnet_proto::BasketKind::WorstOf) => ExBasketKind::WorstOf,
            Err(_) => {
                return Err(PriceError::UnknownEnum {
                    kind: "BasketKind",
                    tag: b.kind,
                });
            }
        };
        let n = b.legs.len();
        if n < 1 {
            return Err(PriceError::Domain("basket needs at least one leg"));
        }
        if !(b.strike.is_finite() && b.strike > 0.0) {
            return Err(PriceError::Domain("basket strike must be positive"));
        }
        // Decode per-leg market data (carried IN the leg — the single-pair
        // MarketContext cannot hold N underlyings; the shared domestic rate
        // comes from MarketContext.r_dom).
        let mut legs = Vec::with_capacity(n);
        for leg in &b.legs {
            if !(leg.spot.is_finite() && leg.spot > 0.0) {
                return Err(PriceError::Domain("basket leg spot must be positive"));
            }
            if !(leg.vol.is_finite() && leg.vol >= 0.0) {
                return Err(PriceError::Domain("basket leg vol must be non-negative"));
            }
            if !leg.weight.is_finite() {
                return Err(PriceError::Domain("basket leg weight must be finite"));
            }
            if !leg.r_for.is_finite() {
                return Err(PriceError::Domain("basket leg r_for must be finite"));
            }
            legs.push(ExBasketLeg::new(
                leg.spot,
                leg.vol,
                market.r_dom() - leg.r_for,
                leg.weight,
            ));
        }
        // The correlation array is the row-major N×N matrix.
        if b.correlations.len() != n * n {
            return Err(PriceError::Domain(
                "basket correlations length must be exactly legs²",
            ));
        }
        if b.correlations.iter().any(|c| !c.is_finite()) {
            return Err(PriceError::Domain("basket correlations must be finite"));
        }
        let correlation: Vec<Vec<f64>> = (0..n)
            .map(|i| b.correlations[i * n..i * n + n].to_vec())
            .collect();

        let spec = BasketSpec {
            legs,
            correlation,
            option_type,
            strike: b.strike,
            kind,
        };
        let cfg = BasketMcConfig {
            budget: if b.mc_paths == 0 {
                DEFAULT_BASKET_MC_PATHS
            } else {
                b.mc_paths as usize
            },
            replications: if b.mc_replications == 0 {
                DEFAULT_BASKET_MC_REPLICATIONS
            } else {
                (b.mc_replications as usize).max(2)
            },
            steps: if b.mc_steps == 0 {
                DEFAULT_BASKET_MC_STEPS
            } else {
                b.mc_steps as usize
            },
            seed: b.mc_seed,
        };
        // The shared domestic (numeraire / settlement-currency) rate is the
        // request market context's r_dom; the SPD-correlation check rejects a
        // non-PSD matrix as INVALID_ARGUMENT rather than regularising it.
        // Settlement-cash numeraire carry: one unit of settlement cash has
        // forward 1 (zero net carry) and discounts at the context's r_dom —
        // `discount_df` is byte-identical to the historical e^{−r_dom·t}.
        let numeraire = celnet_types::Carry::CostOfCarry {
            r: market.r_dom(),
            b: 0.0,
        };
        let estimate = price_basket(&spec, numeraire, expiry, cfg)
            .map_err(|_| PriceError::Domain("basket correlation matrix is not valid SPD"))?;
        // Greek deferral: multi-asset basket sensitivities are a distinct
        // larger increment (per-leg N×{spot,vol} Jacobian + cross-gammas).
        // Report the price + its measured MC std-error with an honest zero
        // strip rather than a fabricated single-underlying bump.
        // A multi-asset basket has no single headline Black vol (each leg
        // carries its own); report 0.0 for the scalar headline-vol field
        // rather than an arbitrary/misleading single value. The per-leg vols
        // live in the instrument's legs.
        Ok(Priced {
            greeks: Greeks::price_only(estimate.price),
            resolved_strike: b.strike,
            vol: 0.0,
            std_error: Some(estimate.std_error),
        })
    }
}

/// A window barrier: no closed form (priced only under LSV, refused here).
pub(super) struct WindowBarrierEngine;
impl ProductEngine for WindowBarrierEngine {
    type Product = celnet_proto::WindowBarrier;
    fn price(&self, _w: &Self::Product, _ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        // A window barrier has no closed form: it is priced only under the LSV
        // model (handled above). Selecting the default model for it is a clear
        // error, never a silent fallback.
        Err(PriceError::UnsupportedModel {
            model: "DEFAULT",
            product: "window_barrier",
        })
    }
}

/// An FX outright forward (linear book).
pub(super) struct FxForwardEngine;
impl ProductEngine for FxForwardEngine {
    type Product = celnet_proto::FxForward;
    fn price(&self, f: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        price_fx_forward(ctx.instrument, ctx.market, ctx.expiry, f)
    }
}

/// An FX swap (linear book).
pub(super) struct FxSwapEngine;
impl ProductEngine for FxSwapEngine {
    type Product = celnet_proto::FxSwap;
    fn price(&self, s: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        price_fx_swap(ctx.instrument, ctx.market, ctx.expiry, s)
    }
}

/// A non-deliverable forward (linear book).
pub(super) struct NdfEngine;
impl ProductEngine for NdfEngine {
    type Product = celnet_proto::Ndf;
    fn price(&self, n: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        price_ndf(ctx.instrument, ctx.market, ctx.expiry, n)
    }
}

/// A perpetual American option on the FX two-rate carry.
pub(super) struct PerpetualOptionEngine;
impl ProductEngine for PerpetualOptionEngine {
    type Product = celnet_proto::PerpetualOption;
    fn price(&self, p: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        let market = ctx.market;
        // The perpetual on the FX path prices over the FX two-rate carry — the
        // carry guard above has already pinned the market to the FX arm, so the
        // carry branching is byte-identical to the vanilla path's (an absent
        // carry stays the FX default, exactly as for a vanilla).
        price_perpetual(
            p,
            market,
            Carry::FxRates {
                r_dom: market.r_dom(),
                r_for: market.r_for(),
            },
        )
    }
}

/// An option on a listed future (asset-class-agnostic Black-76).
pub(super) struct ListedFutureOptionEngine;
impl ProductEngine for ListedFutureOptionEngine {
    type Product = celnet_proto::ListedFutureOption;
    fn price(&self, o: &Self::Product, ctx: &EngineCtx<'_>) -> Result<Priced, PriceError> {
        price_listed_future_option(o, ctx.market, ctx.expiry)
    }
}

/// Dispatch a decoded product oneof to its [`ProductEngine`].
///
/// The static `match` is the registry: each arm extracts the oneof payload and
/// calls the family engine with one line. Zero-allocation (the engines are
/// zero-sized unit structs; no heap table). The pre-dispatch guards (term shape,
/// asset-class routing, FX-carry, LSV booking model) are applied by the caller
/// ([`super::price_instrument`]) and are NOT repeated here — this prices the
/// FX/analytic product path only.
pub(super) fn dispatch(
    product: &instrument::Product,
    ctx: &EngineCtx<'_>,
) -> Result<Priced, PriceError> {
    use instrument::Product as P;
    match product {
        // House-model override (architecture item D): when the worker has a
        // registered pricing model, the analytic vanilla arm prices through it via
        // the plugin-host registry — priced as a `ProductEngine`, the same seam as
        // a native arm. It needs a decodable FX/metal underlying to hand the model
        // (the asset class it prices). Absent a registry, a registered model, or
        // an FX/metal underlying this is the verbatim native static path
        // (byte-identical, zero-allocation). Only this arm consults the registry;
        // every other arm below stays pure static dispatch — so the registry
        // lookup never regresses the native fast path.
        P::Vanilla(v) => {
            if let Some(registry) = ctx.plugin_models
                && let Some(model) = registry.active_pricing_model()
                && let Some(underlying) = ctx
                    .instrument
                    .underlying
                    .as_ref()
                    .and_then(|u| celnet_types::Underlying::try_from(u).ok())
                    .filter(|u| u.as_ccy_pair().is_some())
            {
                PluginModelEngine { model, underlying }.price(v, ctx)
            } else {
                VanillaEngine.price(v, ctx)
            }
        }
        P::Strategy(s) => StrategyEngine.price(s, ctx),
        P::SingleBarrier(b) => {
            if let Some(registry) = ctx.plugin_models
                && let Some(model) = registry.active_exotic_model()
                && let Some(underlying) = ctx
                    .instrument
                    .underlying
                    .as_ref()
                    .and_then(|u| celnet_types::Underlying::try_from(u).ok())
            {
                ExoticPluginModelEngine::new(model, underlying).price_single_barrier(b, ctx)
            } else {
                SingleBarrierEngine.price(b, ctx)
            }
        }
        P::DoubleBarrier(b) => DoubleBarrierEngine.price(b, ctx),
        P::Digital(d) => DigitalEngine.price(d, ctx),
        P::Touch(t) => TouchEngine.price(t, ctx),
        P::VarianceSwap(vs) => VarianceSwapEngine.price(vs, ctx),
        P::VolatilitySwap(vs) => VolatilitySwapEngine.price(vs, ctx),
        P::AsianOption(a) => AsianOptionEngine.price(a, ctx),
        P::ForwardStart(fs) => ForwardStartEngine.price(fs, ctx),
        P::Cliquet(c) => CliquetEngine.price(c, ctx),
        P::Quanto(q) => QuantoEngine.price(q, ctx),
        P::Tarf(t) => TarfEngine.price(t, ctx),
        P::Pivot(p) => PivotEngine.price(p, ctx),
        P::Accumulator(a) => AccumulatorEngine.price(a, ctx),
        P::Lookback(l) => LookbackEngine.price(l, ctx),
        P::American(a) => AmericanEngine.price(a, ctx),
        P::Basket(b) => BasketEngine.price(b, ctx),
        P::WindowBarrier(w) => WindowBarrierEngine.price(w, ctx),
        P::FxForward(f) => FxForwardEngine.price(f, ctx),
        P::FxSwap(s) => FxSwapEngine.price(s, ctx),
        P::Ndf(n) => NdfEngine.price(n, ctx),
        P::PerpetualOption(p) => PerpetualOptionEngine.price(p, ctx),
        P::ListedFutureOption(o) => ListedFutureOptionEngine.price(o, ctx),
    }
}

thread_local! {
    /// The calling pricing worker's installed house-model registry (architecture
    /// item D). The plugin-host [`ModelRegistry`] is per-worker by design
    /// (`!Sync` — a model handle, especially a sandboxed one, is owned by one
    /// worker), so it lives in thread-local state rather than a shared global:
    /// each worker installs its own set once and reads it lock-free on the hot
    /// path. Empty by default (no house models configured ⇒ the verbatim native
    /// dispatch). For the async edge a server installs it from the tokio runtime's
    /// `on_thread_start` hook so every worker thread carries the registry.
    static HOUSE_MODELS: core::cell::RefCell<Option<ModelRegistry>> =
        const { core::cell::RefCell::new(None) };
}

/// Install the calling pricing worker's house-model [`ModelRegistry`], activating
/// its registered models on this worker's analytic vanilla dispatch (architecture
/// item D — dormant-crate activation). Per-worker by design (the registry is
/// `!Sync`): call once per pricing worker at start-up — for the async edge, from
/// the tokio runtime's `on_thread_start` hook. Replaces any registry previously
/// installed on this thread.
pub fn install_house_models(registry: ModelRegistry) {
    HOUSE_MODELS.with(|cell| *cell.borrow_mut() = Some(registry));
}

/// The live dispatch entry: borrow the calling worker's installed house-model
/// registry (if any) and dispatch with it threaded onto the [`EngineCtx`]. Absent
/// an installed registry the borrow is `None` and every product arm is the
/// verbatim native static dispatch (byte-identical, zero-allocation) — so a server
/// with no house models priced bit-for-bit as before this seam existed.
pub(super) fn dispatch_live(
    product: &instrument::Product,
    instrument: &Instrument,
    market: &WireMarketContext,
    expiry: f64,
    conv: &super::ConventionSet,
) -> Result<Priced, PriceError> {
    HOUSE_MODELS.with(|cell| {
        let guard = cell.borrow();
        dispatch(
            product,
            &EngineCtx {
                instrument,
                market,
                expiry,
                conv,
                plugin_models: guard.as_ref(),
            },
        )
    })
}

#[cfg(test)]
mod plugin_dispatch_tests {
    use super::{EngineCtx, dispatch, dispatch_live, install_house_models};
    use celnet_core::{CarryInputs, is_close};
    use celnet_plugin_api::example::FlatSmilePricer;
    use celnet_plugin_host::ModelRegistry;
    use celnet_types::{Carry, OptionType, Underlying};

    fn conv() -> crate::pricer::ConventionSet {
        let wire = celnet_proto::Conventions {
            delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
            atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
            premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
            cut: celnet_proto::Cut::NewYork1000 as i32,
            day_count: celnet_proto::DayCount::Act365Fixed as i32,
            settlement: celnet_proto::Settlement::Deliverable as i32,
        };
        crate::pricer::ConventionSet::decode(&wire).unwrap()
    }

    fn market() -> celnet_proto::MarketContext {
        celnet_proto::MarketContext::fx(1.10, 0.10, 0.02, 0.01)
    }

    fn eurusd() -> celnet_proto::Underlying {
        celnet_proto::Underlying::fx(celnet_proto::CcyPair {
            base: "EUR".into(),
            quote: "USD".into(),
        })
    }

    fn vanilla_fx(strike: f64) -> celnet_proto::Instrument {
        celnet_proto::Instrument {
            underlying: Some(eurusd()),
            expiry_years: 1.0,
            side: celnet_proto::Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(celnet_proto::instrument::Product::Vanilla(
                celnet_proto::Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(celnet_proto::StrikeOrDelta {
                        spec: Some(celnet_proto::strike_or_delta::Spec::Strike(strike)),
                    }),
                },
            )),
            ..Default::default()
        }
    }

    /// A registered plugin model serves the vanilla arm *through the dispatch
    /// path*: the result is the registered model's own price (proving the
    /// dispatch→ModelRegistry edge), and for the reference FX model it agrees with
    /// the native Garman-Kohlhagen strip — the plugin seam preserves FX parity.
    #[test]
    fn plugin_model_serves_vanilla_arm_through_dispatch() {
        let mut registry = ModelRegistry::new();
        registry
            .register_native(FlatSmilePricer::new(0.10))
            .unwrap();

        let instr = vanilla_fx(1.12);
        let market = market();
        let conv = conv();
        let product = instr.product.as_ref().unwrap();

        // Native (no registry): the built-in closed form.
        let native = dispatch(
            product,
            &EngineCtx {
                instrument: &instr,
                market: &market,
                expiry: 1.0,
                conv: &conv,
                plugin_models: None,
            },
        )
        .unwrap();

        // Plugin route (registry present): prices through the registered model.
        let via_plugin = dispatch(
            product,
            &EngineCtx {
                instrument: &instr,
                market: &market,
                expiry: 1.0,
                conv: &conv,
                plugin_models: Some(&registry),
            },
        )
        .unwrap();

        // The edge routed through the registry: identical to the model's own call.
        let model = registry.active_pricing_model().unwrap();
        let direct = model
            .price_and_greeks(
                OptionType::Call,
                &CarryInputs {
                    spot: 1.10,
                    strike: native.resolved_strike,
                    vol: 0.10,
                    t: 1.0,
                    underlying: Underlying::try_from(eurusd()).unwrap(),
                    carry: Carry::FxRates {
                        r_dom: 0.02,
                        r_for: 0.01,
                    },
                },
            )
            .unwrap();
        assert_eq!(via_plugin.greeks.price, direct.price);
        assert_eq!(via_plugin.greeks.delta_spot, direct.delta_spot);
        assert_eq!(via_plugin.greeks.vega, direct.vega);

        // The reference FX model reproduces native Garman-Kohlhagen, so the plugin
        // seam preserves FX parity (Tier-0 interchangeability).
        assert!(is_close(
            via_plugin.greeks.price,
            native.greeks.price,
            1e-12,
            1e-12
        ));
        assert!(is_close(
            via_plugin.greeks.vega,
            native.greeks.vega,
            1e-12,
            1e-12
        ));
        assert!(is_close(
            via_plugin.greeks.rho_dom,
            native.greeks.rho_dom,
            1e-12,
            1e-12
        ));
        assert_eq!(via_plugin.resolved_strike, native.resolved_strike);
    }

    /// A context-only request (no underlying) cannot be handed to a model that
    /// needs the asset class, so it stays on the native path even with a model
    /// registered — byte-identical to no registry.
    #[test]
    fn context_only_request_stays_native_with_registry() {
        let mut registry = ModelRegistry::new();
        registry
            .register_native(FlatSmilePricer::new(0.10))
            .unwrap();
        let mut instr = vanilla_fx(1.12);
        instr.underlying = None;
        let market = market();
        let conv = conv();
        let product = instr.product.as_ref().unwrap();

        let with = dispatch(
            product,
            &EngineCtx {
                instrument: &instr,
                market: &market,
                expiry: 1.0,
                conv: &conv,
                plugin_models: Some(&registry),
            },
        )
        .unwrap();
        let without = dispatch(
            product,
            &EngineCtx {
                instrument: &instr,
                market: &market,
                expiry: 1.0,
                conv: &conv,
                plugin_models: None,
            },
        )
        .unwrap();
        assert_eq!(with.greeks.price, without.greeks.price);
    }

    /// The live `dispatch_live` reads the worker's installed registry: with a
    /// house model installed it routes through the plugin path; reset to empty it
    /// returns to the native path bit-for-bit. A drop guard resets the worker's
    /// registry even on panic so the install never leaks to sibling tests sharing
    /// this thread under `cargo test`.
    #[test]
    fn install_house_models_routes_live_dispatch() {
        struct ResetGuard;
        impl Drop for ResetGuard {
            fn drop(&mut self) {
                install_house_models(ModelRegistry::new());
            }
        }
        let _reset = ResetGuard;

        let instr = vanilla_fx(1.12);
        let market = market();
        let conv = conv();
        let product = instr.product.as_ref().unwrap();

        // Nothing installed yet on this worker: the native path.
        let native = dispatch_live(product, &instr, &market, 1.0, &conv).unwrap();

        // Install a house model; the live path now routes through it.
        let mut registry = ModelRegistry::new();
        registry
            .register_native(FlatSmilePricer::new(0.10))
            .unwrap();
        install_house_models(registry);
        let live = dispatch_live(product, &instr, &market, 1.0, &conv).unwrap();
        assert!(is_close(
            live.greeks.price,
            native.greeks.price,
            1e-12,
            1e-12
        ));

        // Reset to an empty registry: back to the native path, bit-for-bit.
        install_house_models(ModelRegistry::new());
        let after = dispatch_live(product, &instr, &market, 1.0, &conv).unwrap();
        assert_eq!(after.greeks.price, native.greeks.price);
    }
}
