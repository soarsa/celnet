//! Instrument pricing: map a wire [`celnet_proto::Instrument`] priced against a
//! [`celnet_proto::MarketContext`] under a [`celnet_proto::Conventions`] to the
//! full 14-member [`celnet_types::Greeks`] set, a resolved strike, and the vol
//! the smile/market assigned.
//!
//! This is the deterministic analytics router shared by every edge service (RFQ,
//! RFS, scenario): it dispatches the [`celnet_proto::instrument::Product`] oneof
//! to the closed-form `celnet-vanilla` math for vanilla / multi-leg structures
//! and to the `celnet-exotics` closed forms for barriers, digitals and touches.
//!
//! # Determinism
//!
//! Every value routes through `celnet_core::math` (via the analytics crates); no
//! wall-clock, no RNG, no allocation on the per-instrument fast path beyond the
//! transient leg vector of a multi-leg [`celnet_proto::Strategy`]. The exotic
//! Greek set is produced by **central finite differences** over the closed-form
//! price (a single price call per bumped axis), so the same instrument always
//! yields the same Greeks bit-for-bit.
//!
//! This module runs on the **async edge**, never on the pinned pricing core: the
//! core hot path stays the zero-allocation vanilla ring (`celnet-engine`); RFQ /
//! scenario pricing — which carries a client-supplied market context rather than
//! the live published state — is evaluated here off the hot path.

use celnet_proto::{
    Conventions as WireConventions, Instrument, MarketContext as WireMarketContext, instrument,
    strike_or_delta,
};
use celnet_types::{
    AtmConvention, Carry, Cut, DayCount, DeltaConvention, Greeks, OptionType, PremiumStyle,
    RateSensitivities, Settlement, SettlementStyle, Underlying, VanillaInputs,
};

use celnet_core::{CarryGreeks, FlatSmile};
use celnet_exotics::{
    Accumulator as ExAccumulator, AccumulatorMcConfig, AmericanGrid, AmericanOption as ExAmerican,
    AnalyticAsian, AveragingSchedule, BarrierKind as ExBarrierKind, BarrierStyle,
    BasketKind as ExBasketKind, BasketLeg as ExBasketLeg, BasketMcConfig, BasketSpec, Cliquet,
    CliquetMcConfig, CliquetSchedule, DigitalKind, DoubleBarrierKnockOut, DoubleNoTouch,
    ExerciseStyle as ExExerciseStyle, ForwardStart, Lookback as ExLookback, LookbackMcConfig,
    LookbackStyle as ExLookbackStyle, LsmConfig, Monitoring as ExMonitoring, PerpetualInputs,
    PivotTra, PivotTraMcConfig, QuantoParams, RebateTiming, RedemptionStyle as ExRedemptionStyle,
    SingleBarrier as ExSingleBarrier, Tarf as ExTarf, TarfMcConfig, VarSwapContext,
    accumulator_price, american_fd_greeks, american_lsm, cliquet_price_capped_mc,
    cliquet_price_plain, curran_price, digital_price, double_knock_out_price,
    double_no_touch_price, double_touch_price, fair_variance, fair_volatility,
    fixed_lookback_price, floating_lookback_price, forward_start_price, lookback_mc,
    no_touch_price, one_touch_price, perpetual_greeks, perpetual_price, pivot_tra_price,
    price_basket, quanto_digital_price, quanto_vanilla_price, single_barrier_price, tarf_price,
    turnbull_wakeman_price,
};
use celnet_linear::{LinearInputs, LinearTerms, Side as LinearSide, ndf::Ndf as LinearNdf, swap};

use celnet_commodity_vanilla::{CommodityInputs, Margining as CommodityMargining};
use celnet_crypto_vanilla::{
    InverseInputs as CryptoInverseInputs, LinearInputs as CryptoLinearInputs,
    SettlementStyle as CryptoSettlementStyle, inverse as crypto_inverse, linear as crypto_linear,
};
use celnet_equity_vanilla::EquityInputs;

/// A failure pricing a wire instrument: a malformed / unsupported message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriceError {
    /// A required nested message was absent on the wire.
    MissingField(&'static str),
    /// A proto3 enum tag did not correspond to any known variant.
    UnknownEnum {
        /// The enum type name.
        kind: &'static str,
        /// The unrecognized integer tag.
        tag: i32,
    },
    /// The instrument carried no `product` variant.
    EmptyProduct,
    /// A delta-keyed strike could not be inverted to an absolute strike.
    DeltaSolve(celnet_vanilla::DeltaSolveError),
    /// An input was outside the domain the analytics can price (e.g. a
    /// non-positive corridor, or a non-positive expiry).
    Domain(&'static str),
    /// A pricing model was selected for a product it cannot price. The caller
    /// must either pick a supported product or the default model — never a silent
    /// fallback. Mapped to `INVALID_ARGUMENT` at the service boundary.
    UnsupportedModel {
        /// The selected pricing model (e.g. `"LOCAL_STOCH_VOL"`).
        model: &'static str,
        /// The product the model does not support (e.g. `"asian_option"`).
        product: &'static str,
    },
    /// A linear (non-option) product — an FX outright forward, FX swap, or NDF —
    /// reached the option-payoff pricer. These are priced by the dedicated linear
    /// book (`celnet-linear`), not the Garman-Kohlhagen option engine; routing
    /// one here is a dispatch error, never a silent fallback. Mapped to
    /// `INVALID_ARGUMENT` at the service boundary.
    LinearProductNotAnOption {
        /// The linear product variant (e.g. `"fx_forward"`).
        product: &'static str,
    },
}

impl core::fmt::Display for PriceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PriceError::MissingField(field) => write!(f, "missing required field `{field}`"),
            PriceError::UnknownEnum { kind, tag } => write!(f, "unknown {kind} enum tag: {tag}"),
            PriceError::EmptyProduct => write!(f, "instrument carried no product variant"),
            PriceError::DeltaSolve(e) => write!(f, "delta→strike inversion failed: {e:?}"),
            PriceError::Domain(why) => write!(f, "input out of pricing domain: {why}"),
            PriceError::UnsupportedModel { model, product } => write!(
                f,
                "pricing model {model} does not support product {product}; \
                 select a supported product or the default model"
            ),
            PriceError::LinearProductNotAnOption { product } => write!(
                f,
                "product {product} is a linear (non-option) product priced by the \
                 linear book, not the option engine"
            ),
        }
    }
}

impl std::error::Error for PriceError {}

/// The fully priced result for one instrument: the 13-Greek set, the strike the
/// pricing resolved to (the same as the input for an absolute strike; the solved
/// strike for a delta key), and the Black vol used for the headline leg.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Priced {
    /// The full Garman-Kohlhagen Greek set (price + the 13 sensitivities).
    pub greeks: Greeks,
    /// The absolute strike the headline leg resolved to (quote per unit base).
    pub resolved_strike: f64,
    /// The absolute Black vol used for the headline leg.
    pub vol: f64,
    /// For a Monte-Carlo-priced product (the clamped cliquet), the standard
    /// error of the mean of `greeks.price`; `None` for the closed-form products
    /// whose price is exact. Surfaced honestly on `PriceResponse.price_std_error`
    /// so a client never mistakes an MC estimate for closed-form precision.
    pub std_error: Option<f64>,
}

/// The resolved trade conventions decoded from the wire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConventionSet {
    /// The delta convention.
    pub delta: DeltaConvention,
    /// The ATM strike convention.
    pub atm: AtmConvention,
    /// The premium quotation style.
    pub premium: PremiumStyle,
    /// The expiry cut.
    pub cut: Cut,
    /// The vol-time day-count basis.
    pub day_count: DayCount,
    /// The settlement style.
    pub settlement: Settlement,
}

impl ConventionSet {
    /// Decode the wire conventions into the domain convention set.
    ///
    /// # Errors
    ///
    /// [`PriceError::UnknownEnum`] if any convention tag is out of range.
    pub fn decode(w: &WireConventions) -> Result<Self, PriceError> {
        use celnet_proto::{
            AtmConvention as WAtm, Cut as WCut, DayCount as WDc, DeltaConvention as WDelta,
            PremiumStyle as WPrem, Settlement as WSettle,
        };
        let delta = WDelta::try_from(w.delta_convention)
            .map_err(|_| PriceError::UnknownEnum {
                kind: "DeltaConvention",
                tag: w.delta_convention,
            })?
            .into();
        let atm = WAtm::try_from(w.atm_convention)
            .map_err(|_| PriceError::UnknownEnum {
                kind: "AtmConvention",
                tag: w.atm_convention,
            })?
            .into();
        let premium = WPrem::try_from(w.premium_style)
            .map_err(|_| PriceError::UnknownEnum {
                kind: "PremiumStyle",
                tag: w.premium_style,
            })?
            .into();
        let cut = WCut::try_from(w.cut)
            .map_err(|_| PriceError::UnknownEnum {
                kind: "Cut",
                tag: w.cut,
            })?
            .into();
        let day_count = WDc::try_from(w.day_count)
            .map_err(|_| PriceError::UnknownEnum {
                kind: "DayCount",
                tag: w.day_count,
            })?
            .into();
        let settlement = WSettle::try_from(w.settlement)
            .map_err(|_| PriceError::UnknownEnum {
                kind: "Settlement",
                tag: w.settlement,
            })?
            .into();
        Ok(Self {
            delta,
            atm,
            premium,
            cut,
            day_count,
            settlement,
        })
    }
}

/// Decode a proto `OptionType` tag.
fn decode_option_type(tag: i32) -> Result<OptionType, PriceError> {
    celnet_proto::OptionType::try_from(tag)
        .map(Into::into)
        .map_err(|_| PriceError::UnknownEnum {
            kind: "OptionType",
            tag,
        })
}

/// Build the Garman-Kohlhagen inputs for `strike` from a market context and the
/// instrument expiry, taking the per-strike vol from `vol`.
fn inputs_at(
    market: &WireMarketContext,
    expiry_years: f64,
    strike: f64,
    vol: f64,
) -> VanillaInputs {
    VanillaInputs::new(
        market.spot,
        strike,
        vol,
        expiry_years,
        market.r_dom(),
        market.r_for(),
    )
}

/// Build the agnostic carry-seam exotic inputs for `strike` — the
/// [`celnet_exotics::ExoticInputs`] view of [`inputs_at`] (`Carry::FxRates`,
/// byte-identical to the FX two-rate form) consumed by every migrated exotics
/// engine.
fn exotic_inputs_at(
    market: &WireMarketContext,
    expiry_years: f64,
    strike: f64,
    vol: f64,
) -> celnet_exotics::ExoticInputs {
    (&inputs_at(market, expiry_years, strike, vol)).into()
}

/// Lower the American engine's generalized [`CarryGreeks`] strip onto the FX wire
/// [`Greeks`] shape. The FX arm is a verbatim field copy (byte-identical to the
/// pre-ADR-0008 strip); the generalized arm maps by the documented projection
/// `rho_dom = discount_rho + carry_rho`, `rho_for = −carry_rho`
/// ([`RateSensitivities`] docs) — unreachable from the FX-only wire
/// `MarketContext` until ADR-0008 Wave S, but total by construction.
fn fx_wire_greeks(g: &CarryGreeks) -> Greeks {
    let (rho_dom, rho_for) = match g.rates {
        RateSensitivities::Fx { rho_dom, rho_for } => (rho_dom, rho_for),
        RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        } => (discount_rho + carry_rho, -carry_rho),
    };
    Greeks {
        price: g.price,
        delta_spot: g.delta_spot,
        delta_forward: g.delta_forward,
        gamma: g.gamma,
        vega: g.vega,
        theta: g.theta,
        rho_dom,
        rho_for,
        vanna: g.vanna,
        volga: g.volga,
        charm: g.charm,
        speed: g.speed,
        zomma: g.zomma,
        color: g.color,
    }
}

/// Build a [`VarSwapContext`] (forward + carry) for the swap/replication math
/// from the wire market context and the instrument expiry. The forward is
/// `F = S·e^{(r_d−r_f)T}`, derived via [`VanillaInputs::from_inputs`] semantics.
fn var_swap_context(market: &WireMarketContext, expiry_years: f64) -> VarSwapContext {
    let template = exotic_inputs_at(market, expiry_years, market.spot, market.vol);
    VarSwapContext::from_inputs(&template)
}

/// Resolve a [`celnet_proto::StrikeOrDelta`] to an absolute strike. A delta key
/// is inverted under `conv` against a template built from the market context.
fn resolve_strike(
    spec: &strike_or_delta::Spec,
    market: &WireMarketContext,
    expiry_years: f64,
    conv: &ConventionSet,
    option_type: OptionType,
) -> Result<f64, PriceError> {
    match *spec {
        strike_or_delta::Spec::Strike(k) => Ok(k),
        strike_or_delta::Spec::Delta(d) => {
            let template = inputs_at(market, expiry_years, market.spot, market.vol);
            celnet_vanilla::strike_from_delta(conv.delta, option_type, d, &template)
                .map_err(PriceError::DeltaSolve)
        }
    }
}

/// Price a vanilla leg, returning its Greeks and the strike/vol used.
fn price_vanilla_leg(
    option_type: OptionType,
    strike: f64,
    market: &WireMarketContext,
    expiry_years: f64,
) -> Priced {
    let inputs = inputs_at(market, expiry_years, strike, market.vol);
    Priced {
        greeks: celnet_vanilla::greeks(option_type, &inputs),
        resolved_strike: strike,
        vol: market.vol,
        std_error: None,
    }
}

/// The exotic price closure signature: price the product at a (possibly shocked)
/// market context. Used to finite-difference the exotic Greek set.
type ExoticPrice<'a> = dyn Fn(&WireMarketContext) -> f64 + 'a;

/// The relative spot bump for finite-difference Greeks (1 bp of spot).
const FD_SPOT_REL: f64 = 1e-4;
/// The absolute vol bump for finite-difference vega/vanna/volga (1 vol-point).
const FD_VOL_ABS: f64 = 1e-4;
/// The absolute rate bump for finite-difference rho.
const FD_RATE_ABS: f64 = 1e-4;
/// The relative time bump for finite-difference theta/charm/color.
const FD_TIME_REL: f64 = 1e-4;
/// Default antithetic Monte-Carlo path pairs for a clamped cliquet when the wire
/// request leaves `mc_pairs` unset. Sized so the reported standard error is small
/// relative to the price for typical structured-note specs.
const DEFAULT_CLIQUET_MC_PAIRS: usize = 200_000;
/// Default antithetic Monte-Carlo path pairs for a TARF when the wire request
/// leaves `mc_pairs` unset.
const DEFAULT_TARF_MC_PAIRS: usize = 200_000;
/// Default antithetic Monte-Carlo path pairs for a pivot Target-Redemption
/// Accumulator when the wire request leaves `mc_pairs` unset (the same budget as
/// its degenerate `pivot == strike` TARF slice, so the two arms are comparable).
const DEFAULT_PIVOT_MC_PAIRS: usize = 200_000;
/// Default antithetic Monte-Carlo path pairs for an accumulator when the wire
/// request leaves `mc_pairs` unset.
const DEFAULT_ACCUMULATOR_MC_PAIRS: usize = 200_000;
/// Default antithetic Monte-Carlo path pairs for a discrete lookback when the
/// wire request leaves `mc_pairs` unset.
const DEFAULT_LOOKBACK_MC_PAIRS: usize = 200_000;
/// Default monitoring observations for a discrete lookback when the wire request
/// leaves `observations` unset.
const DEFAULT_LOOKBACK_OBSERVATIONS: usize = 64;
/// Default scrambled-Sobol points per replication for a multi-asset basket when
/// the wire request leaves `mc_paths` unset.
const DEFAULT_BASKET_MC_PATHS: usize = 16_384;
/// Default independent randomized scrambles for a multi-asset basket when the
/// wire request leaves `mc_replications` unset (`≥ 2` for a finite std-error).
const DEFAULT_BASKET_MC_REPLICATIONS: usize = 24;
/// Default time steps for a multi-asset basket when the wire request leaves
/// `mc_steps` unset (terminal-only `1` step suffices for these European
/// payoffs).
const DEFAULT_BASKET_MC_STEPS: usize = 1;

/// Shock a market context's spot multiplicatively.
fn bump_spot(m: &WireMarketContext, rel: f64) -> WireMarketContext {
    m.with_spot(m.spot * (1.0 + rel))
}

/// Shock a market context's vol additively.
fn bump_vol(m: &WireMarketContext, d: f64) -> WireMarketContext {
    m.with_vol(m.vol + d)
}

/// Build the full 13-Greek set of an exotic from central finite differences of
/// its closed-form price. `price` reprices the product at any market context;
/// `expiry_years` is bumped for the time Greeks via the `price_at_expiry` closure.
fn exotic_greeks(
    price: &ExoticPrice<'_>,
    price_at_expiry: &dyn Fn(f64, &WireMarketContext) -> f64,
    market: &WireMarketContext,
    expiry_years: f64,
) -> Greeks {
    let base = price(market);

    // Spot Greeks (delta_spot / gamma / speed) via successive spot bumps.
    let h_s = market.spot * FD_SPOT_REL;
    let m_up = bump_spot(market, FD_SPOT_REL);
    let m_dn = bump_spot(market, -FD_SPOT_REL);
    let p_up = price(&m_up);
    let p_dn = price(&m_dn);
    let delta_spot = (p_up - p_dn) / (2.0 * h_s);
    let gamma = (p_up - 2.0 * base + p_dn) / (h_s * h_s);
    // Speed: third derivative via a wider 4-point stencil.
    let m_up2 = bump_spot(market, 2.0 * FD_SPOT_REL);
    let m_dn2 = bump_spot(market, -2.0 * FD_SPOT_REL);
    let p_up2 = price(&m_up2);
    let p_dn2 = price(&m_dn2);
    let speed = (p_up2 - 2.0 * p_up + 2.0 * p_dn - p_dn2) / (2.0 * h_s * h_s * h_s);

    // Vol Greeks (vega / volga) via vol bumps.
    let h_v = FD_VOL_ABS;
    let v_up = price(&bump_vol(market, h_v));
    let v_dn = price(&bump_vol(market, -h_v));
    let vega = (v_up - v_dn) / (2.0 * h_v);
    let volga = (v_up - 2.0 * base + v_dn) / (h_v * h_v);

    // Vanna: cross spot/vol second derivative.
    let p_su_vu = price(&bump_vol(&m_up, h_v));
    let p_su_vd = price(&bump_vol(&m_up, -h_v));
    let p_sd_vu = price(&bump_vol(&m_dn, h_v));
    let p_sd_vd = price(&bump_vol(&m_dn, -h_v));
    let vanna = (p_su_vu - p_su_vd - p_sd_vu + p_sd_vd) / (4.0 * h_s * h_v);

    // Zomma: d(gamma)/d(vol).
    let gamma_vu =
        (price(&bump_vol(&m_up, h_v)) - 2.0 * v_up + price(&bump_vol(&m_dn, h_v))) / (h_s * h_s);
    let gamma_vd =
        (price(&bump_vol(&m_up, -h_v)) - 2.0 * v_dn + price(&bump_vol(&m_dn, -h_v))) / (h_s * h_s);
    let zomma = (gamma_vu - gamma_vd) / (2.0 * h_v);

    // Rate Greeks (rho_dom / rho_for) via rate bumps.
    let h_r = FD_RATE_ABS;
    let rho_dom = {
        let up = price(&market.with_r_dom(market.r_dom() + h_r));
        let dn = price(&market.with_r_dom(market.r_dom() - h_r));
        (up - dn) / (2.0 * h_r)
    };
    let rho_for = {
        let up = price(&market.with_r_for(market.r_for() + h_r));
        let dn = price(&market.with_r_for(market.r_for() - h_r));
        (up - dn) / (2.0 * h_r)
    };

    // Time Greeks: theta = -dV/dT; charm = d(delta)/dT; color = d(gamma)/dT.
    let h_t = expiry_years * FD_TIME_REL;
    let t_up = expiry_years + h_t;
    let t_dn = (expiry_years - h_t).max(f64::MIN_POSITIVE);
    let theta = -(price_at_expiry(t_up, market) - price_at_expiry(t_dn, market)) / (2.0 * h_t);
    let charm = {
        let d_up = (price_at_expiry(t_up, &m_up) - price_at_expiry(t_up, &m_dn)) / (2.0 * h_s);
        let d_dn = (price_at_expiry(t_dn, &m_up) - price_at_expiry(t_dn, &m_dn)) / (2.0 * h_s);
        (d_up - d_dn) / (2.0 * h_t)
    };
    let color = {
        let g_up = (price_at_expiry(t_up, &m_up) - 2.0 * price_at_expiry(t_up, market)
            + price_at_expiry(t_up, &m_dn))
            / (h_s * h_s);
        let g_dn = (price_at_expiry(t_dn, &m_up) - 2.0 * price_at_expiry(t_dn, market)
            + price_at_expiry(t_dn, &m_dn))
            / (h_s * h_s);
        (g_up - g_dn) / (2.0 * h_t)
    };

    // The forward delta uses the discount-factor ratio e^{r_for·T}·delta_spot/?.
    // For an exotic FD set we report the driftless forward delta consistently by
    // scaling the spot delta by the foreign discount factor's inverse — the same
    // relation the vanilla closed form satisfies (delta_forward = e^{r_for T}·
    // delta_spot for an unadjusted spot delta). This keeps the field meaningful
    // without a second inversion.
    let delta_forward = delta_spot * celnet_core::math::exp(market.r_for() * expiry_years);

    Greeks {
        price: base,
        delta_spot,
        delta_forward,
        gamma,
        vega,
        theta,
        rho_dom,
        rho_for,
        vanna,
        volga,
        charm,
        speed,
        zomma,
        color,
    }
}

/// Price a wire [`Instrument`] against a market context and conventions.
///
/// Resolves a delta-keyed strike to an absolute strike, dispatches the product
/// oneof, and returns the full Greek set with the resolved strike and vol.
///
/// # Errors
///
/// [`PriceError`] if the instrument is malformed, carries an unknown enum, an
/// empty product, or an out-of-domain input.
pub fn price_instrument(
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

    // Asset-class routing FIRST (ADR-0008): branch on the decoded underlying before
    // the FX carry guard. An equity / commodity / digital-asset underlying is priced
    // by the cross-asset cost-of-carry leaves (`celnet-{equity,commodity,crypto}-
    // vanilla`), which ACCEPT the generalized `CostOfCarry { b }` carry; that path
    // never touches the FX two-rate guard below. An FX / metal underlying (or an
    // absent underlying, as a pure-context price request carries) falls through to
    // the UNCHANGED FX path + its guard, so the FX/metal contract stays
    // byte-identical. The underlying is decoded for routing only — the FX path's own
    // FX-option validity guard (`validate_fx_underlying`) is unaffected.
    if let Some(wire_underlying) = instrument.underlying.as_ref()
        && let Ok(underlying) = celnet_types::Underlying::try_from(wire_underlying.clone())
        && is_cross_asset(&underlying)
    {
        let product = instrument
            .product
            .as_ref()
            .ok_or(PriceError::EmptyProduct)?;
        // The LSV booking model is the FX vol-surface engine (local-stochastic vol
        // calibrated to the FX smile); it does not apply to a cross-asset
        // cost-of-carry leaf. Selecting it for a cross-asset underlying is refused
        // with a typed error, never silently downgraded to the analytic leaf.
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
        return price_cross_asset(&underlying, instrument, product, market, expiry);
    }

    // Carry-producing-market architecture (no silent fallback): the FX pricing path
    // prices the FX two-rate carry only. An explicitly-supplied generalized
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
    let model = celnet_proto::PricingModel::try_from(instrument.pricing_model).map_err(|_| {
        PriceError::UnknownEnum {
            kind: "PricingModel",
            tag: instrument.pricing_model,
        }
    })?;
    if matches!(model, celnet_proto::PricingModel::LocalStochVol) {
        return price_instrument_lsv(product, market, expiry);
    }

    match product {
        instrument::Product::Vanilla(v) => {
            let option_type = decode_option_type(v.option_type)?;
            let spec = v
                .strike
                .as_ref()
                .and_then(|s| s.spec.as_ref())
                .ok_or(PriceError::MissingField("vanilla.strike"))?;
            let strike = resolve_strike(spec, market, expiry, conv, option_type)?;
            Ok(price_vanilla_leg(option_type, strike, market, expiry))
        }
        instrument::Product::Strategy(s) => {
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
                let side = celnet_proto::Side::try_from(leg.side).map_err(|_| {
                    PriceError::UnknownEnum {
                        kind: "Side",
                        tag: leg.side,
                    }
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
        instrument::Product::SingleBarrier(b) => {
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
            let kind = celnet_proto::BarrierKind::try_from(b.kind).map_err(|_| {
                PriceError::UnknownEnum {
                    kind: "BarrierKind",
                    tag: b.kind,
                }
            })?;
            let bside = celnet_proto::BarrierSide::try_from(b.side).map_err(|_| {
                PriceError::UnknownEnum {
                    kind: "BarrierSide",
                    tag: b.side,
                }
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
                single_barrier_price(&(&inputs_at(m, expiry, strike, m.vol)).into(), ex_spec)
            };
            let price_at = |t: f64, m: &WireMarketContext| {
                single_barrier_price(&(&inputs_at(m, t, strike, m.vol)).into(), ex_spec)
            };
            let greeks = exotic_greeks(&price, &price_at, market, expiry);
            Ok(Priced {
                greeks,
                resolved_strike: strike,
                vol: market.vol,
                std_error: None,
            })
        }
        instrument::Product::DoubleBarrier(b) => {
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
            let kind = celnet_proto::BarrierKind::try_from(b.kind).map_err(|_| {
                PriceError::UnknownEnum {
                    kind: "BarrierKind",
                    tag: b.kind,
                }
            })?;
            let ko =
                DoubleBarrierKnockOut::new(option_type, strike, b.lower_barrier, b.upper_barrier);
            // A double knock-in is priced by in-out parity: KI = vanilla − KO.
            let knock_in = matches!(kind, celnet_proto::BarrierKind::KnockIn);
            let price = move |m: &WireMarketContext| {
                let ki = inputs_at(m, expiry, strike, m.vol);
                let ko_px = double_knock_out_price(&(&ki).into(), ko);
                if knock_in {
                    celnet_vanilla::price(option_type, &ki) - ko_px
                } else {
                    ko_px
                }
            };
            let price_at = move |t: f64, m: &WireMarketContext| {
                let ki = inputs_at(m, t, strike, m.vol);
                let ko_px = double_knock_out_price(&(&ki).into(), ko);
                if knock_in {
                    celnet_vanilla::price(option_type, &ki) - ko_px
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
        instrument::Product::Digital(d) => {
            let option_type = decode_option_type(d.option_type)?;
            let style = celnet_proto::DigitalStyle::try_from(d.style).map_err(|_| {
                PriceError::UnknownEnum {
                    kind: "DigitalStyle",
                    tag: d.style,
                }
            })?;
            let kind = match style {
                celnet_proto::DigitalStyle::CashOrNothing => DigitalKind::cash(option_type),
                celnet_proto::DigitalStyle::AssetOrNothing => DigitalKind::asset(option_type),
            };
            let strike = d.strike;
            let payout = d.payout;
            let price = move |m: &WireMarketContext| {
                payout * digital_price(kind, &(&inputs_at(m, expiry, strike, m.vol)).into())
            };
            let price_at = move |t: f64, m: &WireMarketContext| {
                payout * digital_price(kind, &(&inputs_at(m, t, strike, m.vol)).into())
            };
            let greeks = exotic_greeks(&price, &price_at, market, expiry);
            Ok(Priced {
                greeks,
                resolved_strike: strike,
                vol: market.vol,
                std_error: None,
            })
        }
        instrument::Product::Touch(t) => {
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
                        &(&inputs_at(m, expiry, lower, m.vol)).into(),
                        lower,
                        rebate,
                        RebateTiming::AtHit,
                    )
                }),
                celnet_proto::TouchKind::NoTouch => Box::new(move |m: &WireMarketContext| {
                    no_touch_price(&(&inputs_at(m, expiry, lower, m.vol)).into(), lower, rebate)
                }),
                celnet_proto::TouchKind::DoubleNoTouch => {
                    if !(lower > 0.0 && lower < upper) {
                        return Err(PriceError::Domain(
                            "double-no-touch corridor must satisfy 0 < lower < upper",
                        ));
                    }
                    Box::new(move |m: &WireMarketContext| {
                        double_no_touch_price(
                            &(&inputs_at(m, expiry, lower, m.vol)).into(),
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
                            &(&inputs_at(m, expiry, lower, m.vol)).into(),
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
                        &(&inputs_at(m, t_exp, lower, m.vol)).into(),
                        lower,
                        rebate,
                        RebateTiming::AtHit,
                    ),
                    celnet_proto::TouchKind::NoTouch => {
                        no_touch_price(&(&inputs_at(m, t_exp, lower, m.vol)).into(), lower, rebate)
                    }
                    celnet_proto::TouchKind::DoubleNoTouch => double_no_touch_price(
                        &(&inputs_at(m, t_exp, lower, m.vol)).into(),
                        DoubleNoTouch::new(lower, upper, rebate),
                    ),
                    celnet_proto::TouchKind::DoubleOneTouch => double_touch_price(
                        &(&inputs_at(m, t_exp, lower, m.vol)).into(),
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
        instrument::Product::VarianceSwap(_vs) => {
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
        instrument::Product::VolatilitySwap(_vs) => {
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
        instrument::Product::AsianOption(a) => {
            let option_type = decode_option_type(a.option_type)?;
            let averaging = celnet_proto::AveragingStyle::try_from(a.averaging).map_err(|_| {
                PriceError::UnknownEnum {
                    kind: "AveragingStyle",
                    tag: a.averaging,
                }
            })?;
            let method = celnet_proto::AsianMethod::try_from(a.method).map_err(|_| {
                PriceError::UnknownEnum {
                    kind: "AsianMethod",
                    tag: a.method,
                }
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
                    celnet_proto::AsianMethod::TurnbullWakeman => {
                        turnbull_wakeman_price(&inputs, spec)
                    }
                }
            };
            let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
                let inputs = exotic_inputs_at(m, t, strike, m.vol);
                match method {
                    celnet_proto::AsianMethod::Curran => curran_price(&inputs, spec),
                    celnet_proto::AsianMethod::TurnbullWakeman => {
                        turnbull_wakeman_price(&inputs, spec)
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
        instrument::Product::ForwardStart(fs) => {
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
        instrument::Product::Cliquet(c) => {
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
        instrument::Product::Quanto(q) => {
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
        instrument::Product::Tarf(t) => {
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
        instrument::Product::Pivot(p) => {
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
        instrument::Product::Accumulator(a) => {
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
        instrument::Product::Lookback(l) => {
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
                            ExLookbackStyle::FloatingStrike => {
                                floating_lookback_price(&inputs, option)
                            }
                            ExLookbackStyle::FixedStrike => fixed_lookback_price(&inputs, option),
                        }
                    };
                    let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
                        let inputs = exotic_inputs_at(m, t, strike, m.vol);
                        match style {
                            ExLookbackStyle::FloatingStrike => {
                                floating_lookback_price(&inputs, option)
                            }
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
        instrument::Product::American(a) => {
            let option = decode_option_type(a.option_type)?;
            if !(a.strike.is_finite() && a.strike > 0.0) {
                return Err(PriceError::Domain("American strike must be positive"));
            }
            let exercise =
                celnet_proto::ExerciseStyle::try_from(a.exercise_style).map_err(|_| {
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
        instrument::Product::Basket(b) => {
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
        instrument::Product::WindowBarrier(_) => {
            // A window barrier has no closed form: it is priced only under the LSV
            // model (handled above). Selecting the default model for it is a clear
            // error, never a silent fallback.
            Err(PriceError::UnsupportedModel {
                model: "DEFAULT",
                product: "window_barrier",
            })
        }
        // The linear (non-option) products are priced by the dedicated linear
        // book (`celnet-linear`), not the option engine. Each routes to the
        // closed-form discounted-cashflow leaf behind the product×underlying
        // validity matrix (deliverable forward/swap vs non-deliverable NDF).
        instrument::Product::FxForward(f) => price_fx_forward(instrument, market, expiry, f),
        instrument::Product::FxSwap(s) => price_fx_swap(instrument, market, expiry, s),
        instrument::Product::Ndf(n) => price_ndf(instrument, market, expiry, n),
        // The perpetual on the FX path prices over the FX two-rate carry — the
        // carry guard above has already pinned the market to the FX arm, so the
        // carry branching is byte-identical to the vanilla path's (an absent
        // carry stays the FX default, exactly as for a vanilla).
        instrument::Product::PerpetualOption(p) => price_perpetual(
            p,
            market,
            Carry::FxRates {
                r_dom: market.r_dom(),
                r_for: market.r_for(),
            },
        ),
        instrument::Product::ListedFutureOption(o) => price_listed_future_option(o, market, expiry),
    }
}

// ===========================================================================
// Cross-asset routing (equity / commodity / digital-asset) — ADR-0008
// ===========================================================================

/// Whether the underlying is a cross-asset (non-FX/non-metal) arm routed through
/// the generalized cost-of-carry leaves. FX and metal stay on the FX option path
/// (a metal's lease rate is modelled as the FX foreign rate), byte-identically.
fn is_cross_asset(underlying: &Underlying) -> bool {
    matches!(
        underlying,
        Underlying::Equity(_) | Underlying::Commodity(_) | Underlying::DigitalAsset(_)
    )
}

/// Read the generalized cost-of-carry `(r, b)` from the market context for a
/// cross-asset price. The discount rate `r` is the out-of-band `discount_rate`
/// (= `r_dom`); the net carry `b` is the generalized [`celnet_proto::CostOfCarry`]
/// arm. The carry MUST be the generalized arm — an FX two-rate carry on a
/// cross-asset underlying is a mismatch (a misrouted FX context), refused with a
/// typed error rather than silently coerced; an absent carry is likewise refused
/// (`b` is the whole pricing input — there is no safe default across asset
/// classes), mirroring the FX path's no-silent-fallback guard.
fn cost_of_carry(market: &WireMarketContext) -> Result<Carry, PriceError> {
    use celnet_proto::carry_model::Model;
    let carry = market.carry.as_ref().ok_or(PriceError::Domain(
        "a cross-asset underlying requires a carry market arm (FX two-rate or generalized cost-of-carry)",
    ))?;
    // A cross-asset underlying prices over the SAME carry-producing market as FX
    // (ADR-0008 — asset class is identity-only for the payoff): the net cost-of-carry
    // `b` is either taken directly from the generalized arm, or derived from the FX
    // two-rate arm as `b = r − r_for` (where `r_for` is the asset's carry yield — the
    // dividend yield for an equity, convenience for a commodity, funding for a crypto
    // pair). Both arms are valid and produce the same forward `S·e^{b·t}`. An ABSENT
    // carry is refused — no silent fallback to `b = r` (the ADR-0008 carry guard).
    match carry.model.as_ref() {
        Some(Model::Generalized(g)) => Ok(Carry::CostOfCarry {
            r: market.discount_rate,
            b: g.b,
        }),
        Some(Model::Fx(fx)) => Ok(Carry::CostOfCarry {
            r: market.discount_rate,
            b: market.discount_rate - fx.r_for,
        }),
        None => Err(PriceError::Domain(
            "a cross-asset underlying requires a non-empty carry market arm; no silent fallback",
        )),
    }
}

/// Build the wire-facing [`Greeks`] strip from a leaf [`celnet_core::carry::CarryGreeks`]
/// strip. The leaf reports the rate sensitivities as
/// [`RateSensitivities::Carry`] (discount-rho `∂V/∂r`, carry-rho `∂V/∂b`); these
/// project losslessly onto the flat FX-shaped [`Greeks`] rhos via the
/// `celnet_types` bijection `rho_dom = discount_rho + carry_rho`,
/// `rho_for = −carry_rho`, exactly as an FX vanilla strip populates them (the FX
/// rhos ARE that projection with `r = r_dom`, `b = r_dom − r_for`). The wire
/// boundary then emits them through the same `Greeks → WireGreeks` path the FX
/// vanilla uses.
fn carry_greeks_to_greeks(g: &celnet_core::carry::CarryGreeks) -> Greeks {
    let (rho_dom, rho_for) = match g.rates {
        RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        } => (discount_rho + carry_rho, -carry_rho),
        // The cross-asset leaves always tag Carry; an Fx arm here would be a leaf
        // contract break. Carry the rhos through unchanged rather than fabricate.
        RateSensitivities::Fx { rho_dom, rho_for } => (rho_dom, rho_for),
    };
    Greeks {
        price: g.price,
        delta_spot: g.delta_spot,
        delta_forward: g.delta_forward,
        gamma: g.gamma,
        vega: g.vega,
        theta: g.theta,
        rho_dom,
        rho_for,
        vanna: g.vanna,
        volga: g.volga,
        charm: g.charm,
        speed: g.speed,
        zomma: g.zomma,
        color: g.color,
    }
}

/// The carry-tagged rate-sensitivity arm to present on a **streamed** wire
/// [`Greeks`], selected by the instrument's asset class — the streamed edge of the
/// carry seam (`docs/plan/CARRY-SEAM-TO-EDGE.md` P2). An FX / metal underlying
/// carries the two-rho [`RateSensitivities::Fx`] arm, **byte-identical** to the
/// flat rhos the FX stream has always emitted; a cross-asset (equity / commodity /
/// digital-asset) underlying carries the generalized [`RateSensitivities::Carry`]
/// arm `{discount_rho, carry_rho}`, recovered from the FX-shaped flat rhos by
/// inverting the lossless projection [`carry_greeks_to_greeks`] applies on the way
/// out (`discount_rho = rho_dom + rho_for`, `carry_rho = −rho_for`). The carry-rho
/// is recovered **exactly** (a negation); the discount-rho is the FX-shaped sum
/// `rho_dom + rho_for` — the native discount-rho to within one ULP of that addition
/// (fp addition is not associative, so the double round-trip is not bit-exact —
/// economically nil on a Greek, and the **FX arm, the only byte-identity gate, is
/// untouched**). The native arm flows through bit-exactly once the shared
/// carry→sensitivity mapper lands (plan item F); until then the stream names the
/// arm and carries the FX-shaped magnitudes (pinned by
/// `streamed_rate_sensitivities_round_trips_carry`).
///
/// The discriminator is the underlying's asset class ([`is_cross_asset`]), **not a
/// `match carry`** over the carry model — the ADR-0008 streamed-path review-blocker.
///
/// Scope: this carry-tags the STREAMED edge only (Snapshot/Update). The unary
/// price/quote paths keep emitting the lossless FX-shaped projection until the
/// shared carry→sensitivity mapper lands (plan item F); a client recovers the same
/// numbers from either, and the stream additionally names the asset-class arm.
pub(crate) fn streamed_rate_sensitivities(
    instrument: &Instrument,
    greeks: &Greeks,
) -> RateSensitivities {
    let cross_asset = instrument
        .underlying
        .as_ref()
        .and_then(|u| celnet_types::Underlying::try_from(u.clone()).ok())
        .is_some_and(|u| is_cross_asset(&u));
    if cross_asset {
        RateSensitivities::Carry {
            discount_rho: greeks.rho_dom + greeks.rho_for,
            carry_rho: -greeks.rho_for,
        }
    } else {
        RateSensitivities::Fx {
            rho_dom: greeks.rho_dom,
            rho_for: greeks.rho_for,
        }
    }
}

/// Route a cross-asset (equity / commodity / digital-asset) instrument to its
/// generalized cost-of-carry leaf. The cross-asset option products are the
/// vanilla (equity/commodity/crypto options are `Product::Vanilla` over a
/// non-FX underlying), the perpetual American (same carry seam, no expiry) and
/// the listed-future option (asset-class-agnostic Black-76 on the quoted
/// future); every other product on a cross-asset underlying is refused with a
/// typed error rather than silently mispriced on the FX exotic path. The
/// caller has already rejected the LSV booking model (the FX vol-surface
/// engine) for cross-asset underlyings.
fn price_cross_asset(
    underlying: &Underlying,
    instrument: &Instrument,
    product: &instrument::Product,
    market: &WireMarketContext,
    expiry: f64,
) -> Result<Priced, PriceError> {
    let v = match product {
        instrument::Product::Vanilla(v) => v,
        // A perpetual on a cross-asset underlying prices over the SAME
        // generalized cost-of-carry market arm as the cross-asset vanilla —
        // the ADR-0008 carry guard applies identically (both carry arms
        // accepted, an absent carry refused; no silent fallback).
        instrument::Product::PerpetualOption(p) => {
            return price_perpetual(p, market, cost_of_carry(market)?);
        }
        // A listed-future option is asset-class-agnostic (the quoted futures
        // price already embodies the underlying's carry), so the cross-asset
        // arm routes to the same Black-76 `on_future` leaf as the FX path.
        instrument::Product::ListedFutureOption(o) => {
            return price_listed_future_option(o, market, expiry);
        }
        other => {
            return Err(PriceError::UnsupportedModel {
                model: "DEFAULT",
                product: product_name(other),
            });
        }
    };
    let option_type = decode_option_type(v.option_type)?;
    // A delta-keyed strike requires the FX delta-convention solver, which is an
    // FX-option construct; a cross-asset option carries an absolute strike. A
    // delta key on a cross-asset underlying is a clear input error.
    let spec = v
        .strike
        .as_ref()
        .and_then(|s| s.spec.as_ref())
        .ok_or(PriceError::MissingField("vanilla.strike"))?;
    let strike = match spec {
        strike_or_delta::Spec::Strike(k) => *k,
        strike_or_delta::Spec::Delta(_) => {
            return Err(PriceError::Domain(
                "a cross-asset (equity/commodity/digital-asset) option requires an \
                 absolute strike, not an FX delta key",
            ));
        }
    };
    let carry = cost_of_carry(market)?;
    let (spot, vol) = (market.spot, market.vol);

    match underlying {
        Underlying::Equity(_) => {
            // Generalized-BSM (carry b = r − q). The leaf's spot-space `Carry` is
            // `CostOfCarry { r, b }`; the equity leaf takes (r, q) with q = r − b.
            let r = carry.discount_rate();
            let q = r - carry.carry_rate();
            let inputs = EquityInputs::dividend_paying(spot, strike, vol, expiry, r, q);
            let g = celnet_equity_vanilla::greeks(option_type, &inputs);
            // Reuse the shared carry→flat-greeks projection: the equity leaf's
            // EquityGreeks mirrors CarryGreeks field-for-field.
            let cg = celnet_core::carry::CarryGreeks {
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
        Underlying::Commodity(_) => {
            // Black-76 / cost-of-carry: the commodity leaf consumes the `Carry`
            // directly (the spot is the physical spot, `b` the net carry).
            let inputs = CommodityInputs::new(spot, strike, vol, expiry, carry);
            let g = celnet_commodity_vanilla::greeks(option_type, &inputs);
            Ok(Priced {
                greeks: carry_greeks_to_greeks(&g),
                resolved_strike: strike,
                vol,
                std_error: None,
            })
        }
        Underlying::DigitalAsset(_) => {
            // Linear (USD-margined) generalized-BSM or inverse (coin-margined)
            // `1/S_T` payoff, selected by the instrument's settlement style. The
            // crypto carry seam is the SAME `CostOfCarry { r, b }` the other leaves
            // read (crypto's `b = r − funding`, so the carry-rho is the funding-rho);
            // it is passed through unchanged (no `r − b` round-trip, which is not
            // bit-exact). The inverse leaf's headline strip is in COINS (the
            // contract's natural unit a coin-margined desk hedges in); the
            // USD-equivalent rides on `InverseGreeks::usd_equivalent`.
            let style = decode_settlement_style(instrument.settlement_style)?;
            let g = match style {
                CryptoSettlementStyle::Linear => {
                    let inputs = CryptoLinearInputs::new(spot, strike, vol, expiry, carry);
                    crypto_linear::greeks(option_type, &inputs)
                }
                CryptoSettlementStyle::InverseCoin => {
                    let inputs = CryptoInverseInputs::new(spot, strike, vol, expiry, carry);
                    crypto_inverse::greeks(option_type, &inputs).coin
                }
            };
            Ok(Priced {
                greeks: carry_greeks_to_greeks(&g),
                resolved_strike: strike,
                vol,
                std_error: None,
            })
        }
        // FX / metal never reach here (the caller routes them to the FX path).
        Underlying::Fx(_) | Underlying::Metal(_) => Err(PriceError::Domain(
            "internal: FX/metal underlying routed to the cross-asset path",
        )),
    }
}

/// Decode the wire [`celnet_proto::SettlementStyle`] tag into the crypto leaf's
/// local settlement discriminator. The proto3 default (`0`, `LINEAR`) is the
/// ordinary USD-margined contract; `INVERSE_COIN` selects the coin-margined
/// `1/S_T` payoff. An out-of-range tag is a typed error, never a silent default.
fn decode_settlement_style(tag: i32) -> Result<CryptoSettlementStyle, PriceError> {
    // Decode through the domain `SettlementStyle` (the wire ↔ domain map lives in
    // `celnet-proto`), then lower onto the crypto leaf's local enum.
    let domain = celnet_proto::SettlementStyle::try_from(tag)
        .map(SettlementStyle::from)
        .map_err(|_| PriceError::UnknownEnum {
            kind: "SettlementStyle",
            tag,
        })?;
    Ok(match domain {
        SettlementStyle::Linear => CryptoSettlementStyle::Linear,
        SettlementStyle::InverseCoin => CryptoSettlementStyle::InverseCoin,
    })
}

/// The standard spot-settlement time for the near leg of an FX swap, in years.
///
/// An FX swap's near leg settles on the spot date (the valuation horizon) and the
/// far leg settles at the instrument's forward tenor (`expiry_years`). The spot
/// date sits at the valuation date for present-value purposes, so the near leg's
/// settlement time is `0.0` (the degenerate spot-settling case `LinearInputs`
/// explicitly permits). The forward-points spread between the two legs is the
/// economically meaningful swap quantity and is carried entirely by the
/// `near = 0` / `far = expiry_years` time separation.
const SWAP_NEAR_SETTLE_YEARS: f64 = 0.0;

/// Decode the wire [`celnet_proto::Side`] into the linear book's buy/sell
/// discriminator. `TWO_WAY` is a quoting directive, not a booked direction, so a
/// linear *pricing* request carrying it is rejected (the linear PV needs a
/// definite side).
fn decode_linear_side(tag: i32) -> Result<LinearSide, PriceError> {
    match celnet_proto::Side::try_from(tag) {
        Ok(celnet_proto::Side::Buy) => Ok(LinearSide::Buy),
        Ok(celnet_proto::Side::Sell) => Ok(LinearSide::Sell),
        Ok(celnet_proto::Side::TwoWay) => Err(PriceError::Domain(
            "a linear product needs a definite BUY/SELL side, not TWO_WAY",
        )),
        Err(_) => Err(PriceError::UnknownEnum { kind: "Side", tag }),
    }
}

/// Decode the instrument's [`celnet_proto::Underlying`] into the domain identity,
/// the FX carry produced from the market context, and the spot — the shared input
/// the linear pricers consume. A missing/malformed underlying or a non-positive
/// notional is a typed error (`INVALID_ARGUMENT` at the boundary), never a silent
/// coercion.
fn linear_market(
    instrument: &Instrument,
    market: &WireMarketContext,
) -> Result<(celnet_types::Underlying, celnet_types::Carry), PriceError> {
    let wire_underlying = instrument
        .underlying
        .as_ref()
        .ok_or(PriceError::MissingField("instrument.underlying"))?;
    let underlying = celnet_types::Underlying::try_from(wire_underlying.clone())
        .map_err(|_| PriceError::Domain("instrument.underlying is malformed"))?;
    // The carry guard at the top of `price_instrument` has already rejected a
    // non-FX carry, so the FX two-rate carry reproduces the forward/discount the
    // option leaf would use bit-for-bit (the byte-identity contract).
    let carry = celnet_types::Carry::FxRates {
        r_dom: market.r_dom(),
        r_for: market.r_for(),
    };
    Ok((underlying, carry))
}

/// Whether the instrument's underlying is a non-deliverable (NDF/NDO) pair, per
/// the convention registry. Settlement style is a pair-level property (invariant
/// under tenor and orientation), so any tenor resolves the same class; a metal
/// underlying is always deliverable (loco-London).
fn underlying_is_non_deliverable(underlying: &celnet_types::Underlying) -> bool {
    // Non-deliverability is an FX-pair convention. Only the leg-pair arms (FX /
    // metal) project to a registry-keyed `CcyPair`; the cross-asset arms
    // (equity / commodity / digital-asset) have no FX-pair projection and are
    // never NDF/NDO pairs, so they are deliverable by construction.
    let Some(pair) = underlying.as_ccy_pair() else {
        return false;
    };
    celnet_conventions::resolve(pair, celnet_types::Tenor::Years(1))
        .record
        .is_non_deliverable()
}

/// Build the [`Priced`] result for a linear product from its exact closed-form
/// Greeks. A linear (discounted-cashflow) product has, by construction, zero
/// gamma / vega / vanna / volga / charm / speed / zomma / color and no
/// `price_std_error` (it is exact, not Monte-Carlo). The forward delta uses the
/// same `e^{r_for·t}·delta_spot` relation the vanilla closed form satisfies.
fn linear_priced(
    g: celnet_linear::ForwardGreeks,
    resolved_strike: f64,
    market: &WireMarketContext,
    t: f64,
) -> Priced {
    let delta_forward = g.delta * celnet_core::math::exp(market.r_for() * t);
    Priced {
        greeks: Greeks {
            price: g.pv,
            delta_spot: g.delta,
            delta_forward,
            gamma: 0.0,
            vega: 0.0,
            theta: g.theta,
            rho_dom: g.rho_dom,
            rho_for: g.rho_for,
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        },
        resolved_strike,
        vol: 0.0,
        std_error: None,
    }
}

/// Price an FX outright forward via the `celnet-linear` forward leaf, behind the
/// deliverable validity matrix (a non-deliverable underlying handed to a
/// deliverable forward is rejected as `INVALID_ARGUMENT`).
fn price_fx_forward(
    instrument: &Instrument,
    market: &WireMarketContext,
    expiry: f64,
    f: &celnet_proto::FxForward,
) -> Result<Priced, PriceError> {
    let (underlying, carry) = linear_market(instrument, market)?;
    if underlying_is_non_deliverable(&underlying) {
        return Err(PriceError::Domain(
            "fx_forward requires a deliverable underlying; this pair is non-deliverable (use ndf)",
        ));
    }
    let side = decode_linear_side(f.side)?;
    let inputs = LinearInputs::outright(
        market.spot,
        underlying,
        carry,
        LinearTerms::new(f.contract_rate, f.notional, side),
        expiry,
    )
    .map_err(|_| PriceError::Domain("fx_forward notional must be positive"))?;
    Ok(linear_priced(
        celnet_linear::greeks(&inputs),
        f.contract_rate,
        market,
        expiry,
    ))
}

/// Price an FX swap (near leg at the spot date + far leg at the forward tenor,
/// opposite sides) via the `celnet-linear` swap leaf. Deliverable underlying
/// only; the swap PV is the sum of the two leg PVs and the headline Greek strip
/// is the **net** (near + far) risk — the honest risk view of the whole swap,
/// each leg being a linear product (higher-order Greeks stay zero).
fn price_fx_swap(
    instrument: &Instrument,
    market: &WireMarketContext,
    expiry: f64,
    s: &celnet_proto::FxSwap,
) -> Result<Priced, PriceError> {
    let (underlying, carry) = linear_market(instrument, market)?;
    if underlying_is_non_deliverable(&underlying) {
        return Err(PriceError::Domain(
            "fx_swap requires a deliverable underlying; this pair is non-deliverable",
        ));
    }
    let near = s
        .near
        .as_ref()
        .ok_or(PriceError::MissingField("fx_swap.near"))?;
    let side = decode_linear_side(near.side)?;
    // The near leg anchors the swap: its contract rate / notional / side drive
    // both legs (the far leg trades the opposite side by convention, formed
    // inside the swap leaf). The near leg settles at the spot date, the far leg
    // at the instrument's forward tenor.
    let near_inputs = LinearInputs::outright(
        market.spot,
        underlying,
        carry,
        LinearTerms::new(near.contract_rate, near.notional, side),
        SWAP_NEAR_SETTLE_YEARS,
    )
    .map_err(|_| PriceError::Domain("fx_swap near notional must be positive"))?
    .with_far(expiry)
    .map_err(|_| PriceError::Domain("fx_swap far settlement time must be non-negative"))?;
    let pv = swap::pv(&near_inputs).map_err(|_| PriceError::MissingField("fx_swap.far"))?;
    // The net swap risk is the near leg's Greeks (settling at the spot date) plus
    // the far leg's Greeks (the opposite side, settling at the forward tenor).
    // Reconstruct each leg as a standalone outright forward and sum the strips —
    // the exact analytic Greek of the two-leg sum.
    let near_only = LinearInputs {
        far_settle_t: None,
        ..near_inputs.clone()
    };
    let far_only = LinearInputs {
        side: side.opposite(),
        far_settle_t: None,
        near_settle_t: expiry,
        ..near_inputs
    };
    let near_priced = linear_priced(
        celnet_linear::greeks(&near_only),
        near.contract_rate,
        market,
        SWAP_NEAR_SETTLE_YEARS,
    );
    let far_priced = linear_priced(
        celnet_linear::greeks(&far_only),
        near.contract_rate,
        market,
        expiry,
    );
    let mut greeks = add_scaled(&near_priced.greeks, &far_priced.greeks, 1.0);
    // The summed price must equal the swap leaf PV exactly (same arithmetic); pin
    // it to the leaf value so the headline PV is the canonical swap PV.
    greeks.price = pv;
    Ok(Priced {
        greeks,
        resolved_strike: near.contract_rate,
        vol: 0.0,
        std_error: None,
    })
}

/// Price a non-deliverable forward via the `celnet-linear` NDF leaf, behind the
/// non-deliverable validity matrix (an NDF on a *deliverable* pair, or on a metal
/// underlying, is rejected as `INVALID_ARGUMENT`).
fn price_ndf(
    instrument: &Instrument,
    market: &WireMarketContext,
    expiry: f64,
    n: &celnet_proto::Ndf,
) -> Result<Priced, PriceError> {
    let (underlying, carry) = linear_market(instrument, market)?;
    // An NDF references a restricted-currency FX pair, never a metal (metals are
    // deliverable loco-London).
    if underlying.as_fx().is_none() {
        return Err(PriceError::Domain(
            "ndf requires a non-deliverable FX underlying, not a metal pair",
        ));
    }
    if !underlying_is_non_deliverable(&underlying) {
        return Err(PriceError::Domain(
            "ndf requires a non-deliverable underlying; this pair is deliverable (use fx_forward)",
        ));
    }
    // The fixing identity is carried for booking/reconciliation only — it does
    // not enter the deterministic discounted-cashflow PV. A malformed tag is a
    // clear error rather than a silent default.
    let fixing = decode_fixing_source(n.fixing)?;
    let side = decode_linear_side(n.side)?;
    let inputs = LinearInputs::outright(
        market.spot,
        underlying,
        carry,
        LinearTerms::new(n.contract_rate, n.notional, side),
        expiry,
    )
    .map_err(|_| PriceError::Domain("ndf notional must be positive"))?;
    // The NDF PV equals the deliverable-forward PV of equal terms; report the
    // full linear Greek strip from the same inputs. Take the Greek strip before
    // moving `inputs` into the NDF (`LinearInputs` is no longer `Copy`).
    let greeks = celnet_linear::greeks(&inputs);
    let ndf = LinearNdf::new(inputs, fixing);
    let mut priced = linear_priced(greeks, n.contract_rate, market, expiry);
    priced.greeks.price = ndf.pv();
    Ok(priced)
}

/// Decode the wire [`celnet_proto::FixingSource`] into the domain fixing identity.
///
/// The two enums mirror each other one-for-one (the proto comment states it
/// mirrors `celnet_types::FixingSource`); the match is exhaustive so a future
/// fixing added to one enum forces a compile error here rather than a silent
/// mis-map. The fixing is booking/reconciliation metadata only — it never enters
/// the deterministic discounted-cashflow PV (the honest boundary: live fixing
/// VALUES are an estate-gated feed, never sourced in-repo).
fn decode_fixing_source(tag: i32) -> Result<celnet_types::FixingSource, PriceError> {
    use celnet_proto::FixingSource as W;
    use celnet_types::FixingSource as D;
    let w = W::try_from(tag).map_err(|_| PriceError::UnknownEnum {
        kind: "FixingSource",
        tag,
    })?;
    Ok(match w {
        W::KrwKftc18 => D::KrwKftc18,
        W::TwdTaipei => D::TwdTaipei,
        W::InrRbiRef => D::InrRbiRef,
        W::BrlPtax => D::BrlPtax,
        W::ClpDolarObs => D::ClpDolarObs,
        W::CopTrm => D::CopTrm,
    })
}

// ===========================================================================
// Perpetual American + listed-future option routing (proto arms 30/31)
// ===========================================================================

/// Lower a `celnet-proto` decode/validity [`celnet_proto::convert::WireError`]
/// onto the pricer's typed [`PriceError`]. Both families map to
/// `INVALID_ARGUMENT` at the service boundary, so the lowering is
/// message-preserving where the payload is static and never silently absorbs
/// an error.
fn wire_error_to_price_error(e: celnet_proto::convert::WireError) -> PriceError {
    use celnet_proto::convert::WireError as W;
    match e {
        W::UnknownEnum { kind, tag } => PriceError::UnknownEnum { kind, tag },
        W::MissingField { field } => PriceError::MissingField(field),
        W::InvalidTerms { constraint, .. } => PriceError::Domain(constraint),
        // The remaining wire-decode families cannot arise from the term
        // validators this module calls, but the lowering stays total (a future
        // validator change surfaces as a clear domain error, never a panic).
        W::InvalidCcy { .. } => PriceError::Domain("underlying currency code is malformed"),
        W::OutOfRange { .. } => {
            PriceError::Domain("wire scalar is outside the domain type's range")
        }
        W::WrongUnderlying { .. } => {
            PriceError::Domain("underlying asset class is invalid for this product family")
        }
    }
}

/// The perpetual's cross / higher-order spot-vol sensitivities (vanna, volga,
/// speed, zomma) by central finite differences of the leaf's closed-form price
/// — the same stencils and bump sizes as [`exotic_greeks`]. The leaf's own
/// strip ([`celnet_exotics::PerpetualGreeks`]) is exact for price / delta /
/// gamma / vega and the rhos and is taken verbatim; the time-bumping arm of
/// [`exotic_greeks`] cannot apply here (a perpetual has no expiry to bump) and
/// is never needed (every time Greek is identically zero — see
/// [`price_perpetual`]), so only the four spot/vol stencils are evaluated.
fn perpetual_cross_greeks(option: OptionType, i: &PerpetualInputs) -> (f64, f64, f64, f64) {
    let at = |spot: f64, vol: f64| {
        // Only spot and vol are bumped — the carry is untouched, so the leaf's
        // carry-domain refusal (a b > r call) cannot newly trigger here: the
        // caller has already priced these inputs through `perpetual_greeks`.
        perpetual_price(option, &PerpetualInputs::new(spot, i.strike, vol, i.carry))
            .expect("spot/vol bumps preserve the already-validated carry domain")
    };
    let (s, v) = (i.spot, i.vol);
    let h_s = s * FD_SPOT_REL;
    let h_v = FD_VOL_ABS;
    let (s_up, s_dn) = (s * (1.0 + FD_SPOT_REL), s * (1.0 - FD_SPOT_REL));

    // Vanna: cross spot/vol second derivative (4-point stencil).
    let vanna = (at(s_up, v + h_v) - at(s_up, v - h_v) - at(s_dn, v + h_v) + at(s_dn, v - h_v))
        / (4.0 * h_s * h_v);
    // Volga: second vol derivative.
    let volga = (at(s, v + h_v) - 2.0 * at(s, v) + at(s, v - h_v)) / (h_v * h_v);
    // Speed: third spot derivative via the wider 4-point stencil.
    let (s_up2, s_dn2) = (s * (1.0 + 2.0 * FD_SPOT_REL), s * (1.0 - 2.0 * FD_SPOT_REL));
    let speed = (at(s_up2, v) - 2.0 * at(s_up, v) + 2.0 * at(s_dn, v) - at(s_dn2, v))
        / (2.0 * h_s * h_s * h_s);
    // Zomma: d(gamma)/d(vol) over the gamma stencil at the bumped vols.
    let gamma_at = |vol: f64| (at(s_up, vol) - 2.0 * at(s, vol) + at(s_dn, vol)) / (h_s * h_s);
    let zomma = (gamma_at(v + h_v) - gamma_at(v - h_v)) / (2.0 * h_v);
    (vanna, volga, speed, zomma)
}

/// Price a perpetual (no-expiry) American vanilla via the `celnet-exotics`
/// perpetual leaf over the given carry — the FX two-rate arm on the FX path,
/// the generalized cost-of-carry arm on the cross-asset path (the caller has
/// already applied its path's carry guard, so the branching mirrors the
/// vanilla product's exactly).
///
/// # Greek strip honesty
///
/// The perpetual value is **time-homogeneous**, so theta, charm and color are
/// **identically zero** — those zeros are the exact closed-form values (the
/// stationary value has no time dependence), not placeholders. There is no
/// settlement tenor, so no forward exists to define a forward delta:
/// `delta_forward` is structurally absent and rides as the proto3-zero `0.0`
/// (never fabricated from an `e^{r_for·t}` scaling with no `t`). The leaf's
/// analytic strip (price / delta / gamma / vega and the exact chain-rule rhos)
/// is taken verbatim and the rhos projected through the same
/// [`RateSensitivities`] bijection every carry leaf uses; the cross spot-vol
/// sensitivities the leaf does not model analytically (vanna / volga / speed /
/// zomma) are completed by central finite differences of the closed-form price
/// ([`perpetual_cross_greeks`]) — the module's standard exotic-Greek route.
fn price_perpetual(
    p: &celnet_proto::PerpetualOption,
    market: &WireMarketContext,
    carry: Carry,
) -> Result<Priced, PriceError> {
    let option = decode_option_type(p.option_type)?;
    if !(p.strike.is_finite() && p.strike > 0.0) {
        return Err(PriceError::Domain("perpetual strike must be positive"));
    }
    // The leaf's documented domain: a perpetual claim under a negative
    // discount rate has no finite value (the discounted strike grows without
    // bound), so the request is refused as INVALID_ARGUMENT — never priced
    // through to a NaN.
    if carry.discount_rate() < 0.0 {
        return Err(PriceError::Domain(
            "a perpetual option has no finite value under a negative discount rate",
        ));
    }
    let inputs = PerpetualInputs::new(market.spot, p.strike, market.vol, carry);
    // The leaf's typed refusal (a CALL with carry strictly exceeding the
    // discount rate diverges — no finite value) surfaces as INVALID_ARGUMENT,
    // mirroring the negative-discount-rate refusal above. On the FX path this
    // is exactly r_for < 0 (b = r_dom − r_for > r_dom).
    let g = perpetual_greeks(option, &inputs).map_err(|_| {
        PriceError::Domain(
            "a perpetual call with carry exceeding the discount rate has no finite value",
        )
    })?;
    // The same rho bijection as `carry_greeks_to_greeks`: the FX arm is
    // verbatim; the generalized arm projects losslessly via
    // `rho_dom = discount_rho + carry_rho`, `rho_for = −carry_rho`.
    let (rho_dom, rho_for) = match g.rates {
        RateSensitivities::Fx { rho_dom, rho_for } => (rho_dom, rho_for),
        RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        } => (discount_rho + carry_rho, -carry_rho),
    };
    let (vanna, volga, speed, zomma) = perpetual_cross_greeks(option, &inputs);
    Ok(Priced {
        greeks: Greeks {
            price: g.price,
            delta_spot: g.delta,
            // Structurally absent: no settlement tenor defines a forward.
            delta_forward: 0.0,
            gamma: g.gamma,
            vega: g.vega,
            // Identically zero (exact): the perpetual value is stationary.
            theta: 0.0,
            rho_dom,
            rho_for,
            vanna,
            volga,
            // Identically zero (exact): time-homogeneous delta and gamma.
            charm: 0.0,
            speed,
            zomma,
            color: 0.0,
        },
        resolved_strike: p.strike,
        vol: market.vol,
        std_error: None,
    })
}

/// Price an option on a listed future via the `celnet-commodity-vanilla`
/// Black-76 leaf's `on_future` representation, under the wire margining
/// convention. Asset-class-agnostic: the quoted futures price (the market
/// context's `spot`) already embodies the underlying's carry, so the leaf's
/// carry is structurally `b = 0` for EVERY asset class and the market's carry
/// arm is never read — the only rate that enters is the numeraire discount
/// `r = r_dom` (the `discount_rate`). Under futures-style margining not even
/// that: the daily margin sweep removes the financing leg entirely, so the
/// leaf's discount-rho is the honest, exact `0.0` — a financial statement, not
/// a numerical shortcut (see `futures_style_greeks` in the leaf).
fn price_listed_future_option(
    o: &celnet_proto::ListedFutureOption,
    market: &WireMarketContext,
    expiry: f64,
) -> Result<Priced, PriceError> {
    // The contract's canonical term validator: a present `future_symbol`, a
    // known margining tag, and the expiry ordering
    // `future_expiry_years >= expiry_years > 0` (the future must outlive the
    // option) — each violation is INVALID_ARGUMENT, never clamped. The symbol
    // is the booked contract identity, not a pricing input.
    celnet_proto::convert::validate_listed_future_terms(o, expiry)
        .map_err(wire_error_to_price_error)?;
    let option = decode_option_type(o.option_type)?;
    if !(o.strike.is_finite() && o.strike > 0.0) {
        return Err(PriceError::Domain(
            "listed-future option strike must be positive",
        ));
    }
    let margining = match celnet_proto::Margining::try_from(o.margining) {
        Ok(celnet_proto::Margining::EquityStyle) => CommodityMargining::EquityStyle,
        Ok(celnet_proto::Margining::FuturesStyle) => CommodityMargining::FuturesStyle,
        Err(_) => {
            return Err(PriceError::UnknownEnum {
                kind: "Margining",
                tag: o.margining,
            });
        }
    };
    let inputs =
        CommodityInputs::on_future(market.spot, o.strike, market.vol, expiry, market.r_dom());
    let g = celnet_commodity_vanilla::greeks_with_margining(option, margining, &inputs);
    Ok(Priced {
        greeks: carry_greeks_to_greeks(&g),
        resolved_strike: o.strike,
        vol: market.vol,
        std_error: None,
    })
}

/// Dispatch an instrument selected for the LSV booking model to the
/// [`crate::lsv_pricer`] route. Supports vanilla, single-barrier (continuous
/// knock-out / knock-in) and window-barrier; every other product is a clear
/// [`PriceError::UnsupportedModel`] (`INVALID_ARGUMENT` at the boundary), never a
/// silent fallback to the analytic engine.
fn price_instrument_lsv(
    product: &instrument::Product,
    market: &WireMarketContext,
    expiry: f64,
) -> Result<Priced, PriceError> {
    let grids = crate::lsv_pricer::LsvGrids::default();
    match product {
        instrument::Product::Vanilla(v) => {
            crate::lsv_pricer::price_vanilla_lsv(v, market, expiry, &grids)
        }
        instrument::Product::SingleBarrier(b) => {
            crate::lsv_pricer::price_single_barrier_lsv(b, market, expiry, &grids)
        }
        instrument::Product::WindowBarrier(w) => {
            crate::lsv_pricer::price_window_barrier_lsv(w, market, expiry, &grids)
        }
        other => Err(PriceError::UnsupportedModel {
            model: "LOCAL_STOCH_VOL",
            product: product_name(other),
        }),
    }
}

/// A stable, purpose-named label for a product oneof variant (for clear error
/// messages when an unsupported model is selected).
fn product_name(product: &instrument::Product) -> &'static str {
    match product {
        instrument::Product::Vanilla(_) => "vanilla",
        instrument::Product::Strategy(_) => "strategy",
        instrument::Product::SingleBarrier(_) => "single_barrier",
        instrument::Product::DoubleBarrier(_) => "double_barrier",
        instrument::Product::Digital(_) => "digital",
        instrument::Product::Touch(_) => "touch",
        instrument::Product::VarianceSwap(_) => "variance_swap",
        instrument::Product::VolatilitySwap(_) => "volatility_swap",
        instrument::Product::AsianOption(_) => "asian_option",
        instrument::Product::ForwardStart(_) => "forward_start",
        instrument::Product::Cliquet(_) => "cliquet",
        instrument::Product::Quanto(_) => "quanto",
        instrument::Product::Tarf(_) => "tarf",
        instrument::Product::Pivot(_) => "pivot",
        instrument::Product::Accumulator(_) => "accumulator",
        instrument::Product::Lookback(_) => "lookback",
        instrument::Product::WindowBarrier(_) => "window_barrier",
        instrument::Product::American(_) => "american",
        instrument::Product::Basket(_) => "basket",
        instrument::Product::FxForward(_) => "fx_forward",
        instrument::Product::FxSwap(_) => "fx_swap",
        instrument::Product::Ndf(_) => "ndf",
        instrument::Product::PerpetualOption(_) => "perpetual_option",
        instrument::Product::ListedFutureOption(_) => "listed_future_option",
    }
}

/// The all-zero Greek set — the additive identity used to fold a multi-leg
/// strategy. ([`celnet_types::Greeks`] does not derive `Default`.)
const ZERO_GREEKS: Greeks = Greeks {
    price: 0.0,
    delta_spot: 0.0,
    delta_forward: 0.0,
    gamma: 0.0,
    vega: 0.0,
    theta: 0.0,
    rho_dom: 0.0,
    rho_for: 0.0,
    vanna: 0.0,
    volga: 0.0,
    charm: 0.0,
    speed: 0.0,
    zomma: 0.0,
    color: 0.0,
};

/// `acc + w·g`, field-by-field over the Greek set. Used to fold the legs of a
/// multi-leg [`celnet_proto::Strategy`] into one risk vector.
fn add_scaled(acc: &Greeks, g: &Greeks, w: f64) -> Greeks {
    Greeks {
        price: acc.price + w * g.price,
        delta_spot: acc.delta_spot + w * g.delta_spot,
        delta_forward: acc.delta_forward + w * g.delta_forward,
        gamma: acc.gamma + w * g.gamma,
        vega: acc.vega + w * g.vega,
        theta: acc.theta + w * g.theta,
        rho_dom: acc.rho_dom + w * g.rho_dom,
        rho_for: acc.rho_for + w * g.rho_for,
        vanna: acc.vanna + w * g.vanna,
        volga: acc.volga + w * g.volga,
        charm: acc.charm + w * g.charm,
        speed: acc.speed + w * g.speed,
        zomma: acc.zomma + w * g.zomma,
        color: acc.color + w * g.color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_proto::{
        Conventions as WC, MarketContext as WM, StrikeOrDelta, Vanilla, instrument::Product,
    };

    fn conv_set() -> ConventionSet {
        ConventionSet {
            delta: DeltaConvention::SpotUnadjusted,
            atm: AtmConvention::AtmForward,
            premium: PremiumStyle::DomesticPips,
            cut: Cut::NewYork1000,
            day_count: DayCount::Act365Fixed,
            settlement: Settlement::Deliverable,
        }
    }

    fn wire_conv() -> WC {
        WC {
            delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
            atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
            premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
            cut: celnet_proto::Cut::NewYork1000 as i32,
            day_count: celnet_proto::DayCount::Act365Fixed as i32,
            settlement: celnet_proto::Settlement::Deliverable as i32,
        }
    }

    fn market() -> WM {
        WM::fx(1.10, 0.10, 0.02, 0.01)
    }

    fn vanilla_instrument(strike: f64) -> Instrument {
        Instrument {
            underlying: None,
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
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

    #[test]
    fn vanilla_matches_direct_garman_kohlhagen() {
        let m = market();
        let instr = vanilla_instrument(1.12);
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let direct = celnet_vanilla::greeks(
            OptionType::Call,
            &VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.02, 0.01),
        );
        assert!(is_close(priced.greeks.price, direct.price, 1e-14, 1e-14));
        assert!(is_close(priced.greeks.vega, direct.vega, 1e-14, 1e-14));
        assert!(is_close(priced.resolved_strike, 1.12, 1e-14, 1e-14));
    }

    #[test]
    fn convention_decode_round_trips() {
        let set = ConventionSet::decode(&wire_conv()).unwrap();
        assert_eq!(set, conv_set());
    }

    #[test]
    fn delta_keyed_vanilla_resolves_strike() {
        let m = market();
        let mut instr = vanilla_instrument(0.0);
        instr.product = Some(Product::Vanilla(Vanilla {
            option_type: celnet_proto::OptionType::Call as i32,
            strike: Some(StrikeOrDelta {
                spec: Some(strike_or_delta::Spec::Delta(0.25)),
            }),
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        // The resolved strike must reproduce the requested 25-delta.
        let dd = celnet_vanilla::convention_delta(
            DeltaConvention::SpotUnadjusted,
            OptionType::Call,
            &VanillaInputs::new(1.10, priced.resolved_strike, 0.10, 1.0, 0.02, 0.01),
        );
        assert!(is_close(dd, 0.25, 1e-8, 1e-8), "resolved delta {dd}");
    }

    #[test]
    fn barrier_fd_greeks_have_sane_delta_against_bump() {
        // A down-and-out call has positive spot delta; the FD delta must match a
        // coarse manual central difference of the closed-form price.
        let m = market();
        let instr = Instrument {
            underlying: None,
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
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
        };
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let spec = ExSingleBarrier {
            kind: ExBarrierKind {
                up: false,
                style: BarrierStyle::KnockOut,
                option: OptionType::Call,
            },
            strike: 1.10,
            barrier: 0.95,
            rebate: 0.0,
        };
        let base = single_barrier_price(
            &(&VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01)).into(),
            spec,
        );
        assert!(is_close(priced.greeks.price, base, 1e-12, 1e-12));
        // Manual delta over a 1% bump, compared loosely to the FD field.
        let up = single_barrier_price(
            &(&VanillaInputs::new(1.111, 1.10, 0.10, 1.0, 0.02, 0.01)).into(),
            spec,
        );
        let dn = single_barrier_price(
            &(&VanillaInputs::new(1.089, 1.10, 0.10, 1.0, 0.02, 0.01)).into(),
            spec,
        );
        let manual = (up - dn) / (2.0 * 0.011);
        assert!(
            (priced.greeks.delta_spot - manual).abs() < 0.05,
            "fd delta {} vs manual {}",
            priced.greeks.delta_spot,
            manual
        );
    }

    #[test]
    fn strategy_is_signed_leg_sum() {
        let m = market();
        let leg = |ot: i32, strike: f64, side: i32| celnet_proto::Leg {
            option_type: ot,
            strike: Some(StrikeOrDelta {
                spec: Some(strike_or_delta::Spec::Strike(strike)),
            }),
            side,
            ratio: 1.0,
        };
        let instr = Instrument {
            underlying: None,
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::Strategy(celnet_proto::Strategy {
                kind: celnet_proto::StrategyKind::RiskReversal as i32,
                legs: vec![
                    leg(
                        celnet_proto::OptionType::Call as i32,
                        1.15,
                        celnet_proto::Side::Buy as i32,
                    ),
                    leg(
                        celnet_proto::OptionType::Put as i32,
                        1.05,
                        celnet_proto::Side::Sell as i32,
                    ),
                ],
            })),
            ..Default::default()
        };
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let call = celnet_vanilla::greeks(
            OptionType::Call,
            &VanillaInputs::new(1.10, 1.15, 0.10, 1.0, 0.02, 0.01),
        );
        let put = celnet_vanilla::greeks(
            OptionType::Put,
            &VanillaInputs::new(1.10, 1.05, 0.10, 1.0, 0.02, 0.01),
        );
        assert!(is_close(
            priced.greeks.price,
            call.price - put.price,
            1e-12,
            1e-12
        ));
        assert!(is_close(
            priced.greeks.vega,
            call.vega - put.vega,
            1e-12,
            1e-12
        ));
    }

    // ====================================================================
    // Wave-1 products: variance swap / volatility swap / arithmetic Asian.
    // Each gate proves the SERVER pricer == the celnet-exotics closed form
    // (the independent oracle), plus a closed-form limit oracle.
    // ====================================================================

    fn base_instrument(product: Product) -> Instrument {
        Instrument {
            underlying: None,
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(product),
            ..Default::default()
        }
    }

    #[test]
    fn variance_swap_matches_exotics_closed_form() {
        use celnet_core::FlatSmile;
        use celnet_exotics::{VarSwapContext, fair_variance};
        let m = market();
        let instr = base_instrument(Product::VarianceSwap(celnet_proto::VarianceSwap {
            strike_vol: 0.0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        // Independent oracle: the exotics closed form on the same flat smile/ctx.
        let template = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let ctx = VarSwapContext::from_inputs(&(&template).into());
        let oracle = fair_variance(&FlatSmile::new(m.vol), &ctx);
        assert!(
            is_close(priced.greeks.price, oracle.fair_variance, 1e-9, 1e-12),
            "server K_var {} vs oracle {}",
            priced.greeks.price,
            oracle.fair_variance
        );
        // The echoed resolved strike is K_var; the echoed vol is √K_var.
        assert!(is_close(
            priced.resolved_strike,
            oracle.fair_variance,
            1e-9,
            1e-12
        ));
        assert!(is_close(
            priced.vol,
            oracle.fair_variance.sqrt(),
            1e-9,
            1e-12
        ));
    }

    #[test]
    fn variance_swap_flat_sigma_recovers_sigma_squared() {
        // The genuinely independent closed-form limit oracle: a flat vol σ must
        // give the fair variance K_var == σ² exactly. Catches forward / discount
        // / scale / sign errors in the wire-path replication.
        let conv = conv_set();
        for sigma in [0.05_f64, 0.10, 0.20, 0.35] {
            let m = WM::fx(1.30, sigma, 0.03, 0.01);
            let instr = base_instrument(Product::VarianceSwap(celnet_proto::VarianceSwap {
                strike_vol: 0.0,
            }));
            let priced = price_instrument(&instr, &m, &conv).unwrap();
            assert!(
                is_close(priced.greeks.price, sigma * sigma, 1e-7, 1e-9),
                "K_var {} != σ²={} for σ={}",
                priced.greeks.price,
                sigma * sigma,
                sigma
            );
        }
    }

    #[test]
    fn volatility_swap_matches_exotics_closed_form() {
        use celnet_core::FlatSmile;
        use celnet_exotics::{VarSwapContext, fair_volatility};
        let m = market();
        let instr = base_instrument(Product::VolatilitySwap(celnet_proto::VolatilitySwap {
            strike_vol: 0.0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let template = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let ctx = VarSwapContext::from_inputs(&(&template).into());
        let oracle = fair_volatility(&FlatSmile::new(m.vol), &ctx);
        assert!(
            is_close(priced.greeks.price, oracle.fair_vol, 1e-9, 1e-12),
            "server K_vol {} vs oracle {}",
            priced.greeks.price,
            oracle.fair_vol
        );
        assert!(is_close(
            priced.resolved_strike,
            oracle.fair_vol,
            1e-9,
            1e-12
        ));
        assert!(is_close(priced.vol, oracle.fair_vol, 1e-9, 1e-12));
    }

    #[test]
    fn volatility_swap_flat_sigma_recovers_sigma() {
        // Flat smile ⇒ zero convexity gap ⇒ K_vol == σ exactly.
        let conv = conv_set();
        for sigma in [0.08_f64, 0.15, 0.25] {
            let m = WM::fx(1.30, sigma, 0.03, 0.01);
            let instr = base_instrument(Product::VolatilitySwap(celnet_proto::VolatilitySwap {
                strike_vol: 0.0,
            }));
            let priced = price_instrument(&instr, &m, &conv).unwrap();
            assert!(
                is_close(priced.greeks.price, sigma, 1e-6, 1e-7),
                "K_vol {} != σ={}",
                priced.greeks.price,
                sigma
            );
        }
    }

    #[test]
    fn asian_curran_matches_exotics_closed_form() {
        use celnet_exotics::{AnalyticAsian, curran_price};
        let m = market();
        let instr = base_instrument(Product::AsianOption(celnet_proto::AsianOption {
            option_type: celnet_proto::OptionType::Call as i32,
            strike: 1.10,
            averaging: celnet_proto::AveragingStyle::Discrete as i32,
            observations: 12,
            method: celnet_proto::AsianMethod::Curran as i32,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, 1.10, m.vol, 1.0, m.r_dom(), m.r_for());
        let spec = AnalyticAsian::fresh_discrete(OptionType::Call, 1.10, 12);
        let oracle = curran_price(&(&inputs).into(), spec);
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-12),
            "server Asian (Curran) {} vs oracle {}",
            priced.greeks.price,
            oracle
        );
        assert!(is_close(priced.resolved_strike, 1.10, 1e-14, 1e-14));
        assert!(priced.greeks.price > 0.0 && priced.greeks.vega > 0.0);
    }

    #[test]
    fn asian_turnbull_wakeman_matches_exotics_closed_form() {
        use celnet_exotics::{AnalyticAsian, turnbull_wakeman_price};
        let m = market();
        let instr = base_instrument(Product::AsianOption(celnet_proto::AsianOption {
            option_type: celnet_proto::OptionType::Put as i32,
            strike: 1.12,
            averaging: celnet_proto::AveragingStyle::Continuous as i32,
            observations: 0,
            method: celnet_proto::AsianMethod::TurnbullWakeman as i32,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, 1.12, m.vol, 1.0, m.r_dom(), m.r_for());
        let spec = AnalyticAsian::fresh_continuous(OptionType::Put, 1.12);
        let oracle = turnbull_wakeman_price(&(&inputs).into(), spec);
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-12),
            "server Asian (TW) {} vs oracle {}",
            priced.greeks.price,
            oracle
        );
    }

    #[test]
    fn asian_seasoned_average_matches_exotics_closed_form() {
        use celnet_exotics::{AnalyticAsian, AveragingSchedule, curran_price};
        let m = market();
        let instr = base_instrument(Product::AsianOption(celnet_proto::AsianOption {
            option_type: celnet_proto::OptionType::Call as i32,
            strike: 1.08,
            averaging: celnet_proto::AveragingStyle::Discrete as i32,
            observations: 9,
            method: celnet_proto::AsianMethod::Curran as i32,
            elapsed_avg: 1.095,
            elapsed_weight: 0.25,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, 1.08, m.vol, 1.0, m.r_dom(), m.r_for());
        let spec = AnalyticAsian {
            option: OptionType::Call,
            strike: 1.08,
            schedule: AveragingSchedule::Discrete { future_obs: 9 },
            t_start: 0.0,
            elapsed_avg: 1.095,
            elapsed_weight: 0.25,
        };
        let oracle = curran_price(&(&inputs).into(), spec);
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-12),
            "seasoned Asian {} vs oracle {}",
            priced.greeks.price,
            oracle
        );
    }

    #[test]
    fn asian_rejects_zero_discrete_observations() {
        let m = market();
        let instr = base_instrument(Product::AsianOption(celnet_proto::AsianOption {
            option_type: celnet_proto::OptionType::Call as i32,
            strike: 1.10,
            averaging: celnet_proto::AveragingStyle::Discrete as i32,
            observations: 0,
            method: celnet_proto::AsianMethod::Curran as i32,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        }));
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    // ====================================================================
    // Wave-2 products: forward-start vanilla / cliquet (plain closed form +
    // clamped MC) / quanto (vanilla + digital). Each gate proves the SERVER
    // pricer == the celnet-exotics closed form (the independent oracle), plus a
    // closed-form limit oracle; the clamped cliquet is checked against the
    // independent celnet-exotics MC and surfaces its standard error honestly.
    // ====================================================================

    #[test]
    fn forward_start_matches_exotics_closed_form() {
        use celnet_exotics::{ForwardStart, forward_start_price};
        let m = market();
        let instr = base_instrument(Product::ForwardStart(celnet_proto::ForwardStart {
            option_type: celnet_proto::OptionType::Call as i32,
            moneyness: 1.0,
            reset: 0.25,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = forward_start_price(
            &(&inputs).into(),
            ForwardStart {
                option: OptionType::Call,
                moneyness: 1.0,
                reset: 0.25,
                expiry: 1.0,
            },
        );
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-12),
            "server forward-start {} vs oracle {}",
            priced.greeks.price,
            oracle
        );
        assert!(priced.std_error.is_none());
        assert!(priced.greeks.price > 0.0 && priced.greeks.vega > 0.0);
    }

    #[test]
    fn forward_start_t1_to_zero_recovers_gk_vanilla() {
        // Closed-form limit oracle: as the reset t₁ → 0 the forward-start strike
        // fixes immediately at m·S₀, so the value collapses to the plain GK
        // vanilla struck at m·S₀ over the full maturity.
        let m = market();
        let moneyness = 1.0;
        let instr = base_instrument(Product::ForwardStart(celnet_proto::ForwardStart {
            option_type: celnet_proto::OptionType::Call as i32,
            moneyness,
            reset: 0.0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let gk = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, moneyness * m.spot, m.vol, 1.0, m.r_dom(), m.r_for()),
        );
        assert!(
            is_close(priced.greeks.price, gk, 1e-9, 1e-12),
            "forward-start t1→0 {} vs GK vanilla {}",
            priced.greeks.price,
            gk
        );
    }

    #[test]
    fn plain_cliquet_matches_sum_of_forward_start_legs() {
        use celnet_exotics::{ForwardStart, forward_start_price};
        let m = market();
        let periods = 4u32;
        let moneyness = 1.0;
        let instr = base_instrument(Product::Cliquet(celnet_proto::Cliquet {
            option_type: celnet_proto::OptionType::Call as i32,
            moneyness,
            periods,
            local_floor: None,
            local_cap: None,
            global_floor: None,
            global_cap: None,
            mc_pairs: 0,
            mc_seed: 0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        // Independent oracle: the exact Σ forward-start legs over [0, 1].
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let mut sum = 0.0;
        for k in 1..=periods {
            let reset = (k - 1) as f64 / periods as f64;
            let expiry = k as f64 / periods as f64;
            sum += forward_start_price(
                &(&inputs).into(),
                ForwardStart {
                    option: OptionType::Call,
                    moneyness,
                    reset,
                    expiry,
                },
            );
        }
        assert!(
            is_close(priced.greeks.price, sum, 1e-10, 1e-12),
            "plain cliquet {} vs Σ legs {}",
            priced.greeks.price,
            sum
        );
        assert!(priced.std_error.is_none());
    }

    #[test]
    fn capped_cliquet_matches_exotics_mc_and_carries_std_error() {
        use celnet_exotics::{Cliquet, CliquetMcConfig, CliquetSchedule, cliquet_price_capped_mc};
        let m = market();
        let periods = 4u32;
        let moneyness = 1.0;
        let cap = 0.03;
        let pairs = 50_000u32;
        let seed = 0xCABC_1190_u64;
        let instr = base_instrument(Product::Cliquet(celnet_proto::Cliquet {
            option_type: celnet_proto::OptionType::Call as i32,
            moneyness,
            periods,
            local_floor: Some(0.0),
            local_cap: Some(cap),
            global_floor: None,
            global_cap: None,
            mc_pairs: pairs,
            mc_seed: seed,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        // Independent oracle: the celnet-exotics clamped MC with the SAME (seed,
        // pairs), so the estimator is bit-reproducible and the prices match
        // exactly (the server arm calls the same function).
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let spec = Cliquet {
            option: OptionType::Call,
            moneyness,
            schedule: CliquetSchedule::equal(periods as usize, 1.0),
            local_floor: Some(0.0),
            local_cap: Some(cap),
            global_floor: None,
            global_cap: None,
        };
        let oracle = cliquet_price_capped_mc(
            &(&inputs).into(),
            &spec,
            CliquetMcConfig {
                pairs: pairs as usize,
                seed,
            },
        );
        // Server price == the independent MC mean within its reported std-error
        // (here exactly, same seed) and the std-error is surfaced honestly.
        assert!(
            is_close(
                priced.greeks.price,
                oracle.price,
                oracle.std_error.max(1e-12),
                1e-12
            ),
            "capped cliquet {} vs exotics MC {} (stderr {})",
            priced.greeks.price,
            oracle.price,
            oracle.std_error
        );
        let stderr = priced
            .std_error
            .expect("clamped cliquet must carry std-error");
        assert!(
            is_close(stderr, oracle.std_error, 1e-12, 1e-12),
            "surfaced stderr {} vs oracle {}",
            stderr,
            oracle.std_error
        );
        assert!(stderr > 0.0, "MC std-error must be positive");
        // A capped leg return is bounded above by the plain leg ⇒ the capped
        // cliquet is worth strictly less than the plain ratchet (structural).
        let plain = base_instrument(Product::Cliquet(celnet_proto::Cliquet {
            option_type: celnet_proto::OptionType::Call as i32,
            moneyness,
            periods,
            local_floor: None,
            local_cap: None,
            global_floor: None,
            global_cap: None,
            mc_pairs: 0,
            mc_seed: 0,
        }));
        let plain_priced = price_instrument(&plain, &m, &conv_set()).unwrap();
        assert!(
            priced.greeks.price < plain_priced.greeks.price,
            "capped {} should be < plain {}",
            priced.greeks.price,
            plain_priced.greeks.price
        );
    }

    #[test]
    fn quanto_vanilla_matches_exotics_closed_form() {
        use celnet_exotics::{QuantoParams, quanto_vanilla_price};
        let m = market();
        let strike = 1.10;
        let instr = base_instrument(Product::Quanto(celnet_proto::Quanto {
            payoff: celnet_proto::QuantoPayoff::Vanilla as i32,
            option_type: celnet_proto::OptionType::Call as i32,
            strike,
            conversion_vol: 0.09,
            correlation: -0.3,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = quanto_vanilla_price(
            OptionType::Call,
            &(&inputs).into(),
            QuantoParams::new(0.09, -0.3),
        );
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-12),
            "server quanto vanilla {} vs oracle {}",
            priced.greeks.price,
            oracle
        );
        assert!(is_close(priced.resolved_strike, strike, 1e-14, 1e-14));
    }

    #[test]
    fn quanto_digital_matches_exotics_closed_form() {
        use celnet_exotics::{QuantoParams, quanto_digital_price};
        let m = market();
        let strike = 1.12;
        let instr = base_instrument(Product::Quanto(celnet_proto::Quanto {
            payoff: celnet_proto::QuantoPayoff::Digital as i32,
            option_type: celnet_proto::OptionType::Put as i32,
            strike,
            conversion_vol: 0.07,
            correlation: 0.4,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = quanto_digital_price(
            OptionType::Put,
            &(&inputs).into(),
            QuantoParams::new(0.07, 0.4),
        );
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-12),
            "server quanto digital {} vs oracle {}",
            priced.greeks.price,
            oracle
        );
    }

    #[test]
    fn quanto_zero_correlation_recovers_plain_vanilla() {
        // The drift-adjustment is −ρ·σ_S·σ_Z, so at ρ = 0 the quanto correction
        // vanishes and the price collapses to the plain GK vanilla — a genuinely
        // independent closed-form limit that pins the sign/scale of the adjustment.
        let m = market();
        let strike = 1.10;
        let instr = base_instrument(Product::Quanto(celnet_proto::Quanto {
            payoff: celnet_proto::QuantoPayoff::Vanilla as i32,
            option_type: celnet_proto::OptionType::Call as i32,
            strike,
            conversion_vol: 0.09,
            correlation: 0.0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let gk = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom(), m.r_for()),
        );
        assert!(
            is_close(priced.greeks.price, gk, 1e-12, 1e-12),
            "quanto ρ=0 {} vs plain GK {}",
            priced.greeks.price,
            gk
        );
        // And the quanto-drift sign: a positive correlation lowers a call's value
        // (carry shifted down), a negative correlation raises it.
        let mk = |rho: f64| {
            let i = base_instrument(Product::Quanto(celnet_proto::Quanto {
                payoff: celnet_proto::QuantoPayoff::Vanilla as i32,
                option_type: celnet_proto::OptionType::Call as i32,
                strike,
                conversion_vol: 0.09,
                correlation: rho,
            }));
            price_instrument(&i, &m, &conv_set()).unwrap().greeks.price
        };
        assert!(mk(0.5) < gk, "positive ρ should lower a quanto call");
        assert!(mk(-0.5) > gk, "negative ρ should raise a quanto call");
    }

    #[test]
    fn cliquet_rejects_zero_periods() {
        let m = market();
        let instr = base_instrument(Product::Cliquet(celnet_proto::Cliquet {
            option_type: celnet_proto::OptionType::Call as i32,
            moneyness: 1.0,
            periods: 0,
            local_floor: None,
            local_cap: None,
            global_floor: None,
            global_cap: None,
            mc_pairs: 0,
            mc_seed: 0,
        }));
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    #[test]
    fn quanto_rejects_out_of_range_correlation() {
        let m = market();
        let instr = base_instrument(Product::Quanto(celnet_proto::Quanto {
            payoff: celnet_proto::QuantoPayoff::Vanilla as i32,
            option_type: celnet_proto::OptionType::Call as i32,
            strike: 1.10,
            conversion_vol: 0.09,
            correlation: 1.5,
        }));
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    // ====================================================================
    // Wave-3 products: TARF (MC) / accumulator (MC) / lookback (continuous
    // closed-form + discrete MC). Each gate proves the SERVER pricer == the
    // celnet-exotics reference (MC bit-exact at the same seed; lookback
    // continuous closed-form to ~1e-9), surfaces a positive std-error on the
    // MC products and `None` on the closed-form lookback, and pins a structural
    // invariant (TARF FullGain vs CappedGain redemption ordering; accumulator
    // knock-out reduces value; lookback dominates the equivalent vanilla).
    // ====================================================================

    fn tarf_schedule() -> celnet_proto::FixingSchedule {
        celnet_proto::FixingSchedule {
            fixing_years: vec![0.25, 0.5, 0.75, 1.0],
            fixing_notional: 1.0,
        }
    }

    #[test]
    fn tarf_matches_exotics_mc_and_carries_std_error() {
        let m = market();
        let strike = 1.10;
        let target = 0.30;
        let leverage = 2.0;
        let pairs = 20_000u32;
        let seed = 0x7A2F_BEEF_u64;
        let instr = base_instrument(Product::Tarf(celnet_proto::Tarf {
            option_type: celnet_proto::OptionType::Put as i32,
            strike,
            target,
            leverage,
            redemption: celnet_proto::TarfRedemption::FullGain as i32,
            schedule: Some(tarf_schedule()),
            mc_pairs: pairs,
            mc_seed: seed,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        // Independent oracle: the celnet-exotics MC with the SAME (seed, pairs)
        // and the same fixing count ⇒ bit-reproducible ⇒ exact agreement.
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = tarf_price(
            &(&inputs).into(),
            ExTarf {
                strike,
                fixings: 4,
                target,
                leverage,
                favourable_side: OptionType::Put,
                notional: 1.0,
                redemption: ExRedemptionStyle::FullGain,
            },
            TarfMcConfig {
                pairs: pairs as usize,
                seed,
            },
        );
        assert!(
            is_close(priced.greeks.price, oracle.price, 1e-12, 1e-12),
            "server TARF {} vs exotics MC {}",
            priced.greeks.price,
            oracle.price
        );
        let stderr = priced.std_error.expect("TARF must carry MC std-error");
        assert!(
            is_close(stderr, oracle.std_error, 1e-12, 1e-12) && stderr > 0.0,
            "surfaced stderr {stderr} vs oracle {} (must be positive)",
            oracle.std_error
        );
    }

    #[test]
    fn tarf_full_gain_redeems_below_capped_gain_in_bank_pv() {
        // FullGain lets the breaching fixing pay the client its full overshoot ⇒
        // the bank pays away more ⇒ a strictly lower bank PV than CappedGain. The
        // shared seed gives common random numbers for a clean structural spread.
        let m = market();
        let seed = 0x7A2F_0042_u64;
        let mk = |redemption: celnet_proto::TarfRedemption| {
            let instr = base_instrument(Product::Tarf(celnet_proto::Tarf {
                option_type: celnet_proto::OptionType::Put as i32,
                strike: 1.10,
                target: 0.30,
                leverage: 2.0,
                redemption: redemption as i32,
                schedule: Some(tarf_schedule()),
                mc_pairs: 40_000,
                mc_seed: seed,
            }));
            price_instrument(&instr, &m, &conv_set())
                .unwrap()
                .greeks
                .price
        };
        let full = mk(celnet_proto::TarfRedemption::FullGain);
        let capped = mk(celnet_proto::TarfRedemption::CappedGain);
        assert!(
            full < capped,
            "FullGain bank PV {full} should be below CappedGain {capped}"
        );
    }

    #[test]
    fn tarf_rejects_missing_schedule() {
        let m = market();
        let instr = base_instrument(Product::Tarf(celnet_proto::Tarf {
            option_type: celnet_proto::OptionType::Put as i32,
            strike: 1.10,
            target: 0.30,
            leverage: 2.0,
            redemption: celnet_proto::TarfRedemption::FullGain as i32,
            schedule: None,
            mc_pairs: 0,
            mc_seed: 0,
        }));
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::MissingField(_))
        ));
    }

    #[test]
    fn pivot_matches_exotics_mc_and_carries_std_error() {
        // The pivot TRA arm (32): server == celnet-exotics MC with the SAME
        // (seed, pairs) ⇒ bit-reproducible ⇒ exact agreement, std-error surfaced.
        let m = market();
        let strike = 1.08;
        let pivot = 1.13; // dead band P > K, call-favourable
        let target = 0.20;
        let leverage = 2.5;
        let pairs = 20_000u32;
        let seed = 0x9_1707_BEEF_u64;
        let instr = base_instrument(Product::Pivot(celnet_proto::Pivot {
            option_type: celnet_proto::OptionType::Call as i32,
            strike,
            pivot,
            target,
            leverage,
            redemption: celnet_proto::TarfRedemption::FullGain as i32,
            schedule: Some(tarf_schedule()),
            mc_pairs: pairs,
            mc_seed: seed,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = pivot_tra_price(
            &(&inputs).into(),
            PivotTra {
                strike,
                pivot,
                fixings: 4,
                target,
                leverage,
                favourable_side: OptionType::Call,
                notional: 1.0,
                redemption: ExRedemptionStyle::FullGain,
            },
            PivotTraMcConfig {
                pairs: pairs as usize,
                seed,
            },
        );
        assert!(
            is_close(priced.greeks.price, oracle.price, 1e-12, 1e-12),
            "server pivot TRA {} vs exotics MC {}",
            priced.greeks.price,
            oracle.price
        );
        let stderr = priced.std_error.expect("pivot TRA must carry MC std-error");
        assert!(
            is_close(stderr, oracle.std_error, 1e-12, 1e-12) && stderr > 0.0,
            "surfaced stderr {stderr} vs oracle {} (must be positive)",
            oracle.std_error
        );
    }

    #[test]
    fn pivot_at_strike_is_bit_identical_to_tarf_arm() {
        // The degeneracy law on the WIRE: a Pivot arm with `pivot == strike`
        // must price bit-for-bit equal to the Tarf arm with the same terms
        // (engine identity: same RNG coordinates, same arithmetic order).
        let m = market();
        let strike = 1.10;
        let target = 0.30;
        let leverage = 2.0;
        let pairs = 20_000u32;
        let seed = 0x7A2F_0099_u64;
        let pivot_instr = base_instrument(Product::Pivot(celnet_proto::Pivot {
            option_type: celnet_proto::OptionType::Put as i32,
            strike,
            pivot: strike,
            target,
            leverage,
            redemption: celnet_proto::TarfRedemption::FullGain as i32,
            schedule: Some(tarf_schedule()),
            mc_pairs: pairs,
            mc_seed: seed,
        }));
        let tarf_instr = base_instrument(Product::Tarf(celnet_proto::Tarf {
            option_type: celnet_proto::OptionType::Put as i32,
            strike,
            target,
            leverage,
            redemption: celnet_proto::TarfRedemption::FullGain as i32,
            schedule: Some(tarf_schedule()),
            mc_pairs: pairs,
            mc_seed: seed,
        }));
        let p = price_instrument(&pivot_instr, &m, &conv_set()).unwrap();
        let t = price_instrument(&tarf_instr, &m, &conv_set()).unwrap();
        assert_eq!(
            p.greeks.price.to_bits(),
            t.greeks.price.to_bits(),
            "pivot(P=K) {} must equal TARF {} bit-for-bit on the wire path",
            p.greeks.price,
            t.greeks.price
        );
        assert_eq!(
            p.std_error.unwrap().to_bits(),
            t.std_error.unwrap().to_bits()
        );
    }

    #[test]
    fn pivot_rejects_missing_schedule_and_bad_domain() {
        let m = market();
        let mk = |strike: f64, pivot: f64, target: f64, schedule| {
            base_instrument(Product::Pivot(celnet_proto::Pivot {
                option_type: celnet_proto::OptionType::Put as i32,
                strike,
                pivot,
                target,
                leverage: 2.0,
                redemption: celnet_proto::TarfRedemption::FullGain as i32,
                schedule,
                mc_pairs: 0,
                mc_seed: 0,
            }))
        };
        assert!(matches!(
            price_instrument(&mk(1.10, 1.13, 0.30, None), &m, &conv_set()),
            Err(PriceError::MissingField("pivot.schedule"))
        ));
        assert!(matches!(
            price_instrument(&mk(1.10, 1.13, 0.0, Some(tarf_schedule())), &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
        assert!(matches!(
            price_instrument(&mk(1.10, 0.0, 0.30, Some(tarf_schedule())), &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
        assert!(matches!(
            price_instrument(&mk(0.0, 1.13, 0.30, Some(tarf_schedule())), &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    fn accumulator_schedule() -> celnet_proto::FixingSchedule {
        celnet_proto::FixingSchedule {
            fixing_years: vec![0.25, 0.5, 0.75, 1.0],
            fixing_notional: 1.0,
        }
    }

    #[test]
    fn accumulator_matches_exotics_mc_and_carries_std_error() {
        let m = market();
        let pivot = 1.10;
        let barrier = 1.16;
        let leverage = 2.0;
        let pairs = 20_000u32;
        let seed = 0xACC0_BEEF_u64;
        let instr = base_instrument(Product::Accumulator(celnet_proto::Accumulator {
            pivot,
            barrier,
            leverage,
            monitoring: celnet_proto::AccumulatorMonitoring::Discrete as i32,
            schedule: Some(accumulator_schedule()),
            mc_pairs: pairs,
            mc_seed: seed,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = accumulator_price(
            &(&inputs).into(),
            ExAccumulator {
                pivot,
                barrier,
                fixings: 4,
                leverage,
                notional: 1.0,
                monitoring: ExMonitoring::Discrete,
            },
            AccumulatorMcConfig {
                pairs: pairs as usize,
                seed,
            },
        );
        assert!(
            is_close(priced.greeks.price, oracle.price, 1e-12, 1e-12),
            "server accumulator {} vs exotics MC {}",
            priced.greeks.price,
            oracle.price
        );
        let stderr = priced
            .std_error
            .expect("accumulator must carry MC std-error");
        assert!(stderr > 0.0, "accumulator MC std-error must be positive");
    }

    #[test]
    fn accumulator_continuous_knock_out_reduces_value_vs_discrete() {
        // Continuous monitoring sees between-fixing barrier crossings discrete
        // monitoring misses ⇒ knocks out sooner ⇒ strictly fewer settled fixings
        // ⇒ the accumulated economics are smaller in magnitude (the structure's PV
        // shrinks toward zero — model-free regardless of its net sign). Shared
        // seed ⇒ common random numbers for a clean structural spread.
        let m = market();
        let seed = 0xACC0_0042_u64;
        let mk = |monitoring: celnet_proto::AccumulatorMonitoring| {
            let instr = base_instrument(Product::Accumulator(celnet_proto::Accumulator {
                pivot: 1.10,
                barrier: 1.16,
                leverage: 2.0,
                monitoring: monitoring as i32,
                schedule: Some(accumulator_schedule()),
                mc_pairs: 40_000,
                mc_seed: seed,
            }));
            price_instrument(&instr, &m, &conv_set())
                .unwrap()
                .greeks
                .price
        };
        let discrete = mk(celnet_proto::AccumulatorMonitoring::Discrete);
        let continuous = mk(celnet_proto::AccumulatorMonitoring::Continuous);
        assert!(
            continuous.abs() < discrete.abs(),
            "continuous-monitoring KO must shrink the PV magnitude: \
             continuous {continuous} vs discrete {discrete}"
        );
    }

    #[test]
    fn lookback_continuous_matches_exotics_closed_form_and_dominates_vanilla() {
        let m = market();
        // Floating-strike call: closed form, no std-error, dominates the ATM
        // vanilla (the path minimum is ≤ the strike at expiry).
        let instr = base_instrument(Product::Lookback(celnet_proto::Lookback {
            style: celnet_proto::LookbackStyle::Floating as i32,
            option_type: celnet_proto::OptionType::Call as i32,
            monitoring: celnet_proto::LookbackMonitoring::Continuous as i32,
            strike: 0.0,
            observations: 0,
            mc_pairs: 0,
            mc_seed: 0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = floating_lookback_price(&(&inputs).into(), OptionType::Call);
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-12),
            "server floating lookback {} vs closed form {oracle}",
            priced.greeks.price
        );
        assert!(
            priced.std_error.is_none(),
            "a continuous lookback is closed-form and must NOT carry a std-error"
        );
        let vanilla = celnet_vanilla::price(OptionType::Call, &inputs);
        assert!(
            priced.greeks.price > vanilla,
            "lookback {} must dominate the vanilla {vanilla}",
            priced.greeks.price
        );
    }

    #[test]
    fn lookback_fixed_continuous_matches_exotics_closed_form() {
        let m = market();
        let strike = 1.05;
        let instr = base_instrument(Product::Lookback(celnet_proto::Lookback {
            style: celnet_proto::LookbackStyle::Fixed as i32,
            option_type: celnet_proto::OptionType::Call as i32,
            monitoring: celnet_proto::LookbackMonitoring::Continuous as i32,
            strike,
            observations: 0,
            mc_pairs: 0,
            mc_seed: 0,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = fixed_lookback_price(&(&inputs).into(), OptionType::Call);
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-12),
            "server fixed lookback {} vs closed form {oracle}",
            priced.greeks.price
        );
        assert!(priced.std_error.is_none());
        assert!(is_close(priced.resolved_strike, strike, 1e-14, 1e-14));
    }

    #[test]
    fn lookback_discrete_matches_exotics_mc_and_carries_std_error() {
        let m = market();
        let strike = 1.05;
        let observations = 32u32;
        let pairs = 20_000u32;
        let seed = 0x100C_BAC4_u64;
        let instr = base_instrument(Product::Lookback(celnet_proto::Lookback {
            style: celnet_proto::LookbackStyle::Fixed as i32,
            option_type: celnet_proto::OptionType::Call as i32,
            monitoring: celnet_proto::LookbackMonitoring::Discrete as i32,
            strike,
            observations,
            mc_pairs: pairs,
            mc_seed: seed,
        }));
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let inputs = VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom(), m.r_for());
        let oracle = lookback_mc(
            &(&inputs).into(),
            ExLookback {
                style: ExLookbackStyle::FixedStrike,
                option: OptionType::Call,
            },
            LookbackMcConfig {
                pairs: pairs as usize,
                steps: observations as usize,
                seed,
            },
        );
        assert!(
            is_close(priced.greeks.price, oracle.price, 1e-12, 1e-12),
            "server discrete lookback {} vs exotics MC {}",
            priced.greeks.price,
            oracle.price
        );
        let stderr = priced
            .std_error
            .expect("a discrete lookback is MC and must carry a std-error");
        assert!(
            stderr > 0.0,
            "discrete lookback MC std-error must be positive"
        );
    }

    // ====================================================================
    // American / Bermudan early-exercise — wire-arm gates (PC-AMERICAN)
    // ====================================================================

    /// Build an `Instrument` carrying an American/Bermudan vanilla against the
    /// given market/expiry, with the supplied LSM knobs (lsm_paths == 0 selects
    /// the FD engine).
    fn american_instrument(
        option: celnet_proto::OptionType,
        strike: f64,
        expiry: f64,
        exercise: celnet_proto::ExerciseStyle,
        bermudan_dates: Vec<f64>,
        lsm_paths: u32,
        lsm_seed: u64,
    ) -> Instrument {
        Instrument {
            underlying: None,
            tenor: None,
            expiry_years: expiry,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::American(celnet_proto::AmericanOption {
                option_type: option as i32,
                strike,
                exercise_style: exercise as i32,
                bermudan_dates,
                lsm_paths,
                lsm_exercise_dates: 0,
                lsm_seed,
            })),
            ..Default::default()
        }
    }

    fn american_fd_market() -> WM {
        // A 1Y FX market with positive carry both ways so early exercise can bind.
        WM::fx(100.0, 0.25, 0.08, 0.0)
    }

    /// Gate (a): the American value is never below the European value for the same
    /// inputs (the early-exercise premium is non-negative). Routed through the
    /// wire `price_instrument` arm; the oracle is the independent
    /// `celnet_vanilla` European Garman-Kohlhagen price.
    #[test]
    fn american_at_least_european_on_the_wire() {
        let m = american_fd_market();
        for option in [
            celnet_proto::OptionType::Call,
            celnet_proto::OptionType::Put,
        ] {
            for k in [80.0, 100.0, 120.0] {
                let instr = american_instrument(
                    option,
                    k,
                    1.0,
                    celnet_proto::ExerciseStyle::American,
                    Vec::new(),
                    0,
                    0,
                );
                let american = price_instrument(&instr, &m, &conv_set())
                    .unwrap()
                    .greeks
                    .price;
                let euro = celnet_vanilla::price(
                    option.into(),
                    &VanillaInputs::new(m.spot, k, m.vol, 1.0, m.r_dom(), m.r_for()),
                );
                // Dominance holds up to the FD discretisation error.
                assert!(
                    american >= euro - 5e-3,
                    "{option:?} K={k}: American {american} < European {euro}"
                );
            }
        }
    }

    /// Gate (b): the discriminating no-early-exercise oracle. An American FX CALL
    /// with `r_for = 0` is never worth exercising early, so the wire-priced
    /// American value equals the independent European Garman-Kohlhagen value to FD
    /// tolerance.
    /// No silent fallback: an explicitly-supplied generalized (cost-of-carry) carry
    /// on the FX pricing path is refused with a typed error, never priced as FX with
    /// r_for = 0. The FX projection (absent / FX-arm carry) is unaffected.
    #[test]
    fn generalized_carry_is_rejected_on_the_fx_price_path() {
        let mut m = WM::fx(100.0, 0.20, 0.05, 0.01);
        m.carry = Some(celnet_proto::CarryModel {
            model: Some(celnet_proto::carry_model::Model::Generalized(
                celnet_proto::CostOfCarry { b: 0.04 },
            )),
        });
        let instr = vanilla_instrument(100.0);
        assert!(
            matches!(
                price_instrument(&instr, &m, &conv_set()),
                Err(PriceError::Domain(msg)) if msg.contains("carry")
            ),
            "an explicit generalized carry must be refused on the FX price path"
        );
        // And the FX projection still prices (the guard only refuses the non-FX arm).
        let fx = WM::fx(100.0, 0.20, 0.05, 0.01);
        assert!(price_instrument(&vanilla_instrument(100.0), &fx, &conv_set()).is_ok());
    }

    #[test]
    fn american_call_no_foreign_rate_equals_european_on_the_wire() {
        let m = WM::fx(100.0, 0.20, 0.05, 0.0);
        for k in [80.0, 100.0, 120.0] {
            let instr = american_instrument(
                celnet_proto::OptionType::Call,
                k,
                1.0,
                celnet_proto::ExerciseStyle::American,
                Vec::new(),
                0,
                0,
            );
            let american = price_instrument(&instr, &m, &conv_set())
                .unwrap()
                .greeks
                .price;
            let euro = celnet_vanilla::price(
                OptionType::Call,
                &VanillaInputs::new(m.spot, k, m.vol, 1.0, m.r_dom(), m.r_for()),
            );
            assert!(
                (american - euro).abs() < 5e-3,
                "American call (r_for=0) K={k}: wire {american} vs European {euro}"
            );
        }
    }

    /// Gate (c): the PSOR finite-difference price (wire FD engine) agrees with the
    /// Longstaff-Schwartz regression Monte-Carlo (wire LSM engine) within the
    /// reported MC standard error — two genuinely independent engines.
    #[test]
    fn american_fd_matches_lsm_within_stderr_on_the_wire() {
        let m = american_fd_market();
        let fd_instr = american_instrument(
            celnet_proto::OptionType::Put,
            100.0,
            1.0,
            celnet_proto::ExerciseStyle::American,
            Vec::new(),
            0,
            0,
        );
        let lsm_instr = american_instrument(
            celnet_proto::OptionType::Put,
            100.0,
            1.0,
            celnet_proto::ExerciseStyle::American,
            Vec::new(),
            200_000,
            0xABCD,
        );
        let fd = price_instrument(&fd_instr, &m, &conv_set()).unwrap();
        let lsm = price_instrument(&lsm_instr, &m, &conv_set()).unwrap();
        let se = lsm
            .std_error
            .expect("the LSM engine must carry a price standard error");
        assert!(se > 0.0, "LSM std-error must be positive");
        let tol = 4.0 * se + 1e-2;
        assert!(
            (fd.greeks.price - lsm.greeks.price).abs() < tol,
            "FD {} vs LSM {} (se {se}, tol {tol})",
            fd.greeks.price,
            lsm.greeks.price
        );
        // The FD engine is exact: no std-error.
        assert!(
            fd.std_error.is_none(),
            "the FD engine must report no std-error"
        );
    }

    /// Gate (d): a HAND-PINNED published American option value. Longstaff &
    /// Schwartz (2001), Table 1, first row: American PUT `S₀ = K = 40`, `r = 0.06`,
    /// `σ = 0.20`, `T = 1`, no dividend (`r_for = 0`); their finite-difference
    /// reference is `2.314`. The wire FD arm must reproduce it to FD tolerance.
    #[test]
    fn american_matches_published_value_on_the_wire() {
        let m = WM::fx(40.0, 0.20, 0.06, 0.0);
        let instr = american_instrument(
            celnet_proto::OptionType::Put,
            40.0,
            1.0,
            celnet_proto::ExerciseStyle::American,
            Vec::new(),
            0,
            0,
        );
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        const PUBLISHED_FD: f64 = 2.314;
        assert!(
            (priced.greeks.price - PUBLISHED_FD).abs() < 1e-2,
            "wire American put (LS 2001 Table 1) {} vs published {PUBLISHED_FD}",
            priced.greeks.price
        );
    }

    /// Gate (e): a Bermudan with a single date at expiry equals the European value;
    /// a dense date set approaches (and never exceeds) the American value.
    #[test]
    fn bermudan_endpoints_on_the_wire() {
        let m = WM::fx(100.0, 0.30, 0.10, 0.0);
        // Single date at expiry ⇒ European.
        let one = american_instrument(
            celnet_proto::OptionType::Put,
            110.0,
            1.0,
            celnet_proto::ExerciseStyle::Bermudan,
            vec![1.0],
            0,
            0,
        );
        let berm_one = price_instrument(&one, &m, &conv_set())
            .unwrap()
            .greeks
            .price;
        let euro = celnet_vanilla::price(
            OptionType::Put,
            &VanillaInputs::new(m.spot, 110.0, m.vol, 1.0, m.r_dom(), m.r_for()),
        );
        assert!(
            (berm_one - euro).abs() < 5e-3,
            "Bermudan(1 date @ T) {berm_one} vs European {euro}"
        );

        // Dense date set ⇒ approaches American from below.
        let dense_dates: Vec<f64> = (1..=50).map(|k| k as f64 / 50.0).collect();
        let dense = american_instrument(
            celnet_proto::OptionType::Put,
            110.0,
            1.0,
            celnet_proto::ExerciseStyle::Bermudan,
            dense_dates,
            0,
            0,
        );
        let berm_dense = price_instrument(&dense, &m, &conv_set())
            .unwrap()
            .greeks
            .price;
        let american = price_instrument(
            &american_instrument(
                celnet_proto::OptionType::Put,
                110.0,
                1.0,
                celnet_proto::ExerciseStyle::American,
                Vec::new(),
                0,
                0,
            ),
            &m,
            &conv_set(),
        )
        .unwrap()
        .greeks
        .price;
        assert!(
            berm_dense <= american + 1e-3 && (american - berm_dense) < 5e-2,
            "Bermudan(dense) {berm_dense} vs American {american}"
        );
        assert!(
            berm_dense >= berm_one - 1e-3,
            "more dates ⇒ at least as valuable"
        );
    }

    /// An empty Bermudan date set is a clear domain error (no exercise
    /// opportunity), never a silent fallback.
    #[test]
    fn empty_bermudan_dates_is_a_domain_error() {
        let m = american_fd_market();
        let instr = american_instrument(
            celnet_proto::OptionType::Put,
            100.0,
            1.0,
            celnet_proto::ExerciseStyle::Bermudan,
            Vec::new(),
            0,
            0,
        );
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    /// Build a wire basket instrument.
    fn basket_instrument(
        legs: Vec<celnet_proto::BasketLeg>,
        correlations: Vec<f64>,
        option: celnet_proto::OptionType,
        strike: f64,
        kind: celnet_proto::BasketKind,
        expiry: f64,
    ) -> Instrument {
        Instrument {
            underlying: None,
            tenor: None,
            expiry_years: expiry,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::Basket(celnet_proto::BasketOption {
                legs,
                correlations,
                option_type: option as i32,
                strike,
                kind: kind as i32,
                mc_paths: 16_384,
                mc_replications: 24,
                mc_steps: 1,
                mc_seed: 0xBA5_3E7,
            })),
            ..Default::default()
        }
    }

    /// Gate: a one-leg, weight-1 basket priced on the wire equals the independent
    /// Garman-Kohlhagen vanilla within the reported MC standard error, using the
    /// request market context's `r_dom` as the shared domestic (settlement) rate
    /// and the leg's own spot/vol/r_for. Also asserts the Greek strip is the
    /// honest zeroed (deferred) set and the std-error is carried.
    #[test]
    fn degenerate_basket_matches_vanilla_on_the_wire() {
        let m = market();
        let spot = 1.12;
        let vol = 0.13;
        let r_for = 0.012;
        let strike = 1.10;
        for option in [
            celnet_proto::OptionType::Call,
            celnet_proto::OptionType::Put,
        ] {
            let instr = basket_instrument(
                vec![celnet_proto::BasketLeg {
                    underlying: None,
                    weight: 1.0,
                    spot,
                    vol,
                    r_for,
                }],
                vec![1.0],
                option,
                strike,
                celnet_proto::BasketKind::Basket,
                1.0,
            );
            let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
            let gk = celnet_vanilla::price(
                option.into(),
                &VanillaInputs::new(spot, strike, vol, 1.0, m.r_dom(), r_for),
            );
            let se = priced.std_error.expect("MC basket carries a std-error");
            assert!(
                (priced.greeks.price - gk).abs() <= 4.0 * se + 1e-9,
                "{option:?}: wire basket {} vs GK {} (4·se {})",
                priced.greeks.price,
                gk,
                4.0 * se
            );
            // The Greek strip is the honest deferred zero (price-only).
            assert_eq!(priced.greeks.delta_spot, 0.0);
            assert_eq!(priced.greeks.vega, 0.0);
            assert_eq!(priced.greeks.gamma, 0.0);
        }
    }

    /// Gate: a non-PSD correlation matrix on the wire is a clear domain error,
    /// never silently regularised.
    #[test]
    fn non_psd_basket_correlation_is_a_domain_error() {
        let m = market();
        let leg = |spot| celnet_proto::BasketLeg {
            underlying: None,
            weight: 0.5,
            spot,
            vol: 0.12,
            r_for: 0.01,
        };
        let instr = basket_instrument(
            vec![leg(1.10), leg(1.27)],
            // ρ = 1.01 > 1 ⇒ indefinite.
            vec![1.0, 1.01, 1.01, 1.0],
            celnet_proto::OptionType::Call,
            1.18,
            celnet_proto::BasketKind::Basket,
            1.0,
        );
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    /// Gate: a wrong-length correlation array (not legs²) is a clear domain error.
    #[test]
    fn basket_wrong_correlation_length_is_a_domain_error() {
        let m = market();
        let leg = |spot| celnet_proto::BasketLeg {
            underlying: None,
            weight: 0.5,
            spot,
            vol: 0.12,
            r_for: 0.01,
        };
        let instr = basket_instrument(
            vec![leg(1.10), leg(1.27)],
            vec![1.0, 0.4], // should be 4 entries for 2 legs
            celnet_proto::OptionType::Call,
            1.18,
            celnet_proto::BasketKind::Basket,
            1.0,
        );
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    // =====================================================================
    // W2 — linear book (FX forward / FX swap / NDF) server routing.
    //
    // Each test proves: server `price_instrument` == the `celnet-linear` leaf
    // == an INDEPENDENT oracle (the two-discount-bond route, hand-derived
    // literals, structural identities), and the product×underlying validity
    // matrix (deliverable forward/swap vs non-deliverable NDF) is enforced as
    // INVALID_ARGUMENT (a `PriceError`), never a silent fallback.
    // =====================================================================

    /// A deliverable FX underlying (EURUSD).
    fn eurusd_underlying() -> celnet_proto::Underlying {
        celnet_proto::Underlying::fx(celnet_proto::CcyPair {
            base: "EUR".into(),
            quote: "USD".into(),
        })
    }

    /// A non-deliverable FX underlying (USDBRL — the registry marks BRL as a
    /// PTAX-fixed NDF pair).
    fn usdbrl_underlying() -> celnet_proto::Underlying {
        celnet_proto::Underlying::fx(celnet_proto::CcyPair {
            base: "USD".into(),
            quote: "BRL".into(),
        })
    }

    /// The streamed carry-tagged arm is FX **byte-identical** and the cross-asset
    /// `Carry` arm projects back to the original flat rhos exactly (the inverse
    /// bijection) — the no-regression + correctness contract for the streamed edge
    /// of the carry seam (`streamed_rate_sensitivities`).
    #[test]
    fn streamed_rate_sensitivities_round_trips_carry() {
        use celnet_types::RateSensitivities as RS;
        // A flat Greek strip with distinctive rhos (the values an FX stream emits).
        let mut g = Greeks::price_only(1.23);
        g.rho_dom = 0.456;
        g.rho_for = -0.789;

        // FX underlying → the two-rho Fx arm, byte-identical to the flat rhos.
        let fx = Instrument {
            underlying: Some(eurusd_underlying()),
            ..Default::default()
        };
        match streamed_rate_sensitivities(&fx, &g) {
            RS::Fx { rho_dom, rho_for } => {
                assert_eq!(rho_dom.to_bits(), g.rho_dom.to_bits());
                assert_eq!(rho_for.to_bits(), g.rho_for.to_bits());
            }
            other => panic!("FX underlying must carry the Fx arm, got {other:?}"),
        }

        // Cross-asset underlying → the generalized Carry arm; projecting it back to
        // the FX-shaped flat rhos recovers the originals bit-for-bit.
        let eq = Instrument {
            underlying: Some(celnet_proto::Underlying::equity(celnet_proto::EquityRef::new(
                celnet_proto::Symbol::new("AAPL", "XNAS"),
                "USD",
            ))),
            ..Default::default()
        };
        match streamed_rate_sensitivities(&eq, &g) {
            RS::Carry {
                discount_rho,
                carry_rho,
            } => {
                // The arm is the documented projection of the FX-shaped flat rhos:
                // discount = rho_dom + rho_for (exact as that sum), carry = −rho_for
                // (exact negation).
                assert_eq!(discount_rho.to_bits(), (g.rho_dom + g.rho_for).to_bits());
                assert_eq!(carry_rho.to_bits(), (-g.rho_for).to_bits());
                // Projecting the carry arm back to the FX-shaped flat rhos recovers
                // rho_for bit-exactly and rho_dom to within one ULP (fp addition is
                // not associative — economically nil on a Greek).
                assert_eq!((-carry_rho).to_bits(), g.rho_for.to_bits());
                assert!(is_close(discount_rho + carry_rho, g.rho_dom, 1e-15, 1e-15));
            }
            other => panic!("cross-asset underlying must carry the Carry arm, got {other:?}"),
        }
    }

    /// A market context whose spot/rates suit a linear-book worked example
    /// (spot 1.20, r_dom 0.05, r_for 0.02). Vol is irrelevant to a linear PV.
    fn linear_market_ctx() -> WM {
        WM::fx(1.20, 0.10, 0.05, 0.02)
    }

    fn fx_forward_instrument(
        underlying: celnet_proto::Underlying,
        contract_rate: f64,
        notional: f64,
        side: celnet_proto::Side,
        expiry: f64,
    ) -> Instrument {
        Instrument {
            underlying: Some(underlying),
            tenor: None,
            expiry_years: expiry,
            quantity: None,
            side: side as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::FxForward(celnet_proto::FxForward {
                contract_rate,
                notional,
                side: side as i32,
            })),
            ..Default::default()
        }
    }

    /// Server forward PV == the `celnet-linear` leaf == the INDEPENDENT
    /// two-discount-bond oracle, and the linear Greek strip is surfaced with
    /// the higher-order option Greeks exactly zero.
    #[test]
    fn fx_forward_matches_linear_leaf_and_independent_oracle() {
        let m = linear_market_ctx();
        let (k, n, t) = (1.25, 1_000_000.0, 1.0);
        let instr = fx_forward_instrument(eurusd_underlying(), k, n, celnet_proto::Side::Buy, t);
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();

        // The `celnet-linear` leaf, constructed directly from the same inputs.
        let li = LinearInputs::outright(
            1.20,
            celnet_types::Underlying::Fx(celnet_types::CcyPair::parse("EURUSD").unwrap()),
            celnet_types::Carry::FxRates {
                r_dom: 0.05,
                r_for: 0.02,
            },
            LinearTerms::new(k, n, LinearSide::Buy),
            t,
        )
        .unwrap();
        let leaf = celnet_linear::greeks(&li);
        assert_eq!(priced.greeks.price.to_bits(), leaf.pv.to_bits());
        assert_eq!(priced.greeks.delta_spot.to_bits(), leaf.delta.to_bits());
        assert_eq!(priced.greeks.rho_dom.to_bits(), leaf.rho_dom.to_bits());
        assert_eq!(priced.greeks.rho_for.to_bits(), leaf.rho_for.to_bits());
        assert_eq!(priced.greeks.theta.to_bits(), leaf.theta.to_bits());

        // INDEPENDENT oracle: value each leg as a zero-coupon discount bond
        // straight from the raw rates (a route that never forms F or df) —
        // PV = side·N·(spot·e^{−r_for·t} − K·e^{−r_dom·t}).
        let oracle = 1.0
            * n
            * (1.20 * celnet_core::math::exp(-0.02 * t) - k * celnet_core::math::exp(-0.05 * t));
        assert!(
            (priced.greeks.price - oracle).abs() <= 1e-7 * oracle.abs().max(1.0),
            "server {} vs oracle {oracle}",
            priced.greeks.price
        );

        // A linear product has no optionality: gamma/vega/vanna/volga/charm/
        // speed/zomma/color are exactly zero.
        for g in [
            priced.greeks.gamma,
            priced.greeks.vega,
            priced.greeks.vanna,
            priced.greeks.volga,
            priced.greeks.charm,
            priced.greeks.speed,
            priced.greeks.zomma,
            priced.greeks.color,
        ] {
            assert_eq!(g.to_bits(), 0.0_f64.to_bits());
        }
        // Exact (not Monte-Carlo) ⇒ no standard error.
        assert_eq!(priced.std_error, None);
        assert_eq!(priced.resolved_strike.to_bits(), k.to_bits());
    }

    /// A forward struck at the fair forward has PV exactly 0 (a structural gate
    /// that exercises the full server dispatch).
    #[test]
    fn fx_forward_at_fair_forward_is_zero_pv() {
        let m = linear_market_ctx();
        // Fair forward F = 1.20·e^{(0.05−0.02)·1} = 1.20·e^{0.03}.
        let fair = 1.20 * celnet_core::math::exp(0.03);
        let instr = fx_forward_instrument(
            eurusd_underlying(),
            fair,
            1_000_000.0,
            celnet_proto::Side::Buy,
            1.0,
        );
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        assert!(
            priced.greeks.price.abs() <= 1e-6,
            "pv {}",
            priced.greeks.price
        );
    }

    /// An FX forward on a non-deliverable pair is rejected (INVALID_ARGUMENT),
    /// never silently delivered.
    #[test]
    fn fx_forward_on_non_deliverable_is_rejected() {
        let m = linear_market_ctx();
        let instr = fx_forward_instrument(
            usdbrl_underlying(),
            5.1,
            1_000_000.0,
            celnet_proto::Side::Buy,
            0.5,
        );
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    /// A linear product needs a definite side; TWO_WAY is a quoting directive
    /// and is rejected.
    #[test]
    fn fx_forward_two_way_side_is_rejected() {
        let m = linear_market_ctx();
        let instr = fx_forward_instrument(
            eurusd_underlying(),
            1.25,
            1_000_000.0,
            celnet_proto::Side::TwoWay,
            1.0,
        );
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    /// A forward with no underlying is rejected (no silent coercion).
    #[test]
    fn fx_forward_missing_underlying_is_rejected() {
        let m = linear_market_ctx();
        let mut instr =
            fx_forward_instrument(eurusd_underlying(), 1.25, 1e6, celnet_proto::Side::Buy, 1.0);
        instr.underlying = None;
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::MissingField("instrument.underlying"))
        ));
    }

    /// Server swap PV == the `celnet-linear` swap leaf (near at spot, far at the
    /// instrument tenor, opposite sides) == the INDEPENDENT two-outright sum.
    #[test]
    fn fx_swap_matches_linear_leaf_and_two_leg_sum() {
        let m = linear_market_ctx();
        let (k, n, far_t) = (1.25, 2_000_000.0, 1.0);
        let near_leg = celnet_proto::FxForward {
            contract_rate: k,
            notional: n,
            side: celnet_proto::Side::Buy as i32,
        };
        let far_leg = celnet_proto::FxForward {
            contract_rate: k,
            notional: n,
            side: celnet_proto::Side::Sell as i32,
        };
        let instr = Instrument {
            underlying: Some(eurusd_underlying()),
            tenor: None,
            expiry_years: far_t,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::FxSwap(celnet_proto::FxSwap {
                near: Some(near_leg),
                far: Some(far_leg),
            })),
            ..Default::default()
        };
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();

        // The leaf: near at spot (t=0), far at the tenor, opposite sides.
        let inputs = LinearInputs::outright(
            1.20,
            celnet_types::Underlying::Fx(celnet_types::CcyPair::parse("EURUSD").unwrap()),
            celnet_types::Carry::FxRates {
                r_dom: 0.05,
                r_for: 0.02,
            },
            LinearTerms::new(k, n, LinearSide::Buy),
            SWAP_NEAR_SETTLE_YEARS,
        )
        .unwrap()
        .with_far(far_t)
        .unwrap();
        let leaf_pv = swap::pv(&inputs).unwrap();
        assert_eq!(priced.greeks.price.to_bits(), leaf_pv.to_bits());

        // INDEPENDENT two-outright sum from the discount-bond oracle.
        let bond = |side: f64, t: f64| {
            side * n
                * (1.20 * celnet_core::math::exp(-0.02 * t) - k * celnet_core::math::exp(-0.05 * t))
        };
        let oracle = bond(1.0, SWAP_NEAR_SETTLE_YEARS) + bond(-1.0, far_t);
        assert!(
            (priced.greeks.price - oracle).abs() <= 1e-7 * oracle.abs().max(1.0),
            "swap server {} vs oracle {oracle}",
            priced.greeks.price
        );
        assert_eq!(priced.std_error, None);
    }

    fn ndf_instrument(
        underlying: celnet_proto::Underlying,
        contract_rate: f64,
        notional: f64,
        side: celnet_proto::Side,
        fixing: celnet_proto::FixingSource,
        expiry: f64,
    ) -> Instrument {
        Instrument {
            underlying: Some(underlying),
            tenor: None,
            expiry_years: expiry,
            quantity: None,
            side: side as i32,
            solve: None,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::Ndf(celnet_proto::Ndf {
                contract_rate,
                notional,
                side: side as i32,
                fixing: fixing as i32,
                settlement_ccy: "USD".into(),
            })),
            ..Default::default()
        }
    }

    /// Server NDF PV == the `celnet-linear` NDF leaf == the HAND-DERIVED literal
    /// (USD/BRL worked example from `ndf.rs`), and equals the deliverable-forward
    /// PV of equal terms (structural identity).
    #[test]
    fn ndf_matches_linear_leaf_and_hand_derived_literal() {
        // USD/BRL: spot 5.0, K 5.1, r_dom(BRL) 0.10, r_for(USD) 0.05, t 0.5,
        // notional 1_000_000, BUY. df = e^{−0.05}, F = 5·e^{0.025}.
        let m = WM::fx(5.0, 0.10, 0.10, 0.05);
        let instr = ndf_instrument(
            usdbrl_underlying(),
            5.1,
            1_000_000.0,
            celnet_proto::Side::Buy,
            celnet_proto::FixingSource::BrlPtax,
            0.5,
        );
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();

        // The leaf NDF, same inputs.
        let li = LinearInputs::outright(
            5.0,
            celnet_types::Underlying::Fx(celnet_types::CcyPair::parse("USDBRL").unwrap()),
            celnet_types::Carry::FxRates {
                r_dom: 0.10,
                r_for: 0.05,
            },
            LinearTerms::new(5.1, 1_000_000.0, LinearSide::Buy),
            0.5,
        )
        .unwrap();
        let leaf = LinearNdf::new(li, celnet_types::FixingSource::BrlPtax);
        assert_eq!(priced.greeks.price.to_bits(), leaf.pv().to_bits());

        // HAND-DERIVED literal (external recomputation, see ndf.rs).
        let expected = 25_279.495_188_022_105_f64;
        assert!(
            (priced.greeks.price - expected).abs() <= 1e-6,
            "ndf server {} vs pinned {expected}",
            priced.greeks.price
        );
        assert_eq!(priced.std_error, None);
    }

    /// An NDF on a DELIVERABLE pair is rejected (INVALID_ARGUMENT) — use a
    /// deliverable forward instead; never a silent fallback.
    #[test]
    fn ndf_on_deliverable_is_rejected() {
        let m = linear_market_ctx();
        let instr = ndf_instrument(
            eurusd_underlying(),
            1.25,
            1_000_000.0,
            celnet_proto::Side::Buy,
            celnet_proto::FixingSource::BrlPtax,
            1.0,
        );
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    /// An NDF on a metal underlying is rejected (metals are deliverable
    /// loco-London, never a non-deliverable FX pair).
    #[test]
    fn ndf_on_metal_is_rejected() {
        let m = WM::fx(2000.0, 0.10, 0.05, 0.0);
        let metal = celnet_proto::Underlying::metal(celnet_proto::MetalPair::new(
            celnet_proto::Metal::Gold,
            "USD",
        ));
        let instr = ndf_instrument(
            metal,
            2000.0,
            100.0,
            celnet_proto::Side::Buy,
            celnet_proto::FixingSource::BrlPtax,
            1.0,
        );
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    // ---- cross-asset routing (equity / commodity / digital-asset) -----------

    /// A market context carrying the generalized cost-of-carry arm `CostOfCarry{b}`
    /// over the discount rate `r` — the cross-asset market shape.
    fn cross_asset_market(spot: f64, vol: f64, r: f64, b: f64) -> WM {
        WM {
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

    fn cross_asset_vanilla(underlying: celnet_proto::Underlying, strike: f64) -> Instrument {
        Instrument {
            underlying: Some(underlying),
            expiry_years: 1.0,
            side: celnet_proto::Side::Buy as i32,
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

    #[test]
    fn equity_vanilla_routes_to_equity_leaf() {
        // r = 0.05, q = 0.03 ⇒ b = r − q = 0.02.
        let (r, q) = (0.05, 0.03);
        let m = cross_asset_market(100.0, 0.20, r, r - q);
        let underlying = celnet_proto::Underlying::equity(celnet_proto::EquityRef::new(
            celnet_proto::Symbol::new("AAPL", "XNAS"),
            "USD",
        ));
        let instr = cross_asset_vanilla(underlying, 100.0);
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let oracle = celnet_equity_vanilla::price(
            OptionType::Call,
            &celnet_equity_vanilla::EquityInputs::dividend_paying(100.0, 100.0, 0.20, 1.0, r, q),
        );
        assert!(
            is_close(priced.greeks.price, oracle, 1e-12, 1e-12),
            "server equity {} vs leaf {oracle}",
            priced.greeks.price
        );
        assert!(is_close(priced.resolved_strike, 100.0, 1e-14, 1e-14));
    }

    #[test]
    fn commodity_vanilla_routes_to_black76_leaf() {
        // Option on a future: b = 0, the pure Black-76 degenerate.
        let r = 0.05;
        let m = cross_asset_market(50.0, 0.30, r, 0.0);
        let underlying = celnet_proto::Underlying::commodity(celnet_proto::CommodityRef::new(
            celnet_proto::Symbol::new("BRENT", ""),
            "USD",
        ));
        let instr = cross_asset_vanilla(underlying, 55.0);
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let oracle = celnet_commodity_vanilla::price(
            OptionType::Call,
            &celnet_commodity_vanilla::CommodityInputs::on_future(50.0, 55.0, 0.30, 1.0, r),
        );
        assert!(
            is_close(priced.greeks.price, oracle, 1e-12, 1e-12),
            "server commodity {} vs leaf {oracle}",
            priced.greeks.price
        );
    }

    #[test]
    fn crypto_linear_routes_to_linear_leaf() {
        // b = r − funding ⇒ funding = r − b.
        let (r, b) = (0.05, 0.03);
        let m = cross_asset_market(30_000.0, 0.65, r, b);
        let underlying =
            celnet_proto::Underlying::digital_asset(celnet_proto::CryptoPair::new("BTC", "USDT"));
        let mut instr = cross_asset_vanilla(underlying, 31_000.0);
        instr.settlement_style = celnet_proto::SettlementStyle::Linear as i32;
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let oracle = celnet_crypto_vanilla::linear::price(
            OptionType::Call,
            &celnet_crypto_vanilla::LinearInputs::new(
                30_000.0,
                31_000.0,
                0.65,
                1.0,
                celnet_types::Carry::CostOfCarry { r, b },
            ),
        );
        assert!(
            is_close(priced.greeks.price, oracle, 1e-9, 1e-9),
            "server crypto-linear {} vs leaf {oracle}",
            priced.greeks.price
        );
    }

    #[test]
    fn crypto_inverse_routes_to_inverse_leaf() {
        let (r, b) = (0.05, 0.03);
        let m = cross_asset_market(30_000.0, 0.65, r, b);
        let underlying =
            celnet_proto::Underlying::digital_asset(celnet_proto::CryptoPair::new("BTC", "USD"));
        let mut instr = cross_asset_vanilla(underlying, 31_000.0);
        instr.settlement_style = celnet_proto::SettlementStyle::InverseCoin as i32;
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        // The inverse headline price is in COINS (a small premium).
        let oracle = celnet_crypto_vanilla::inverse::price(
            OptionType::Call,
            &celnet_crypto_vanilla::InverseInputs::new(
                30_000.0,
                31_000.0,
                0.65,
                1.0,
                celnet_types::Carry::CostOfCarry { r, b },
            ),
        );
        assert!(
            is_close(priced.greeks.price, oracle, 1e-12, 1e-12),
            "server crypto-inverse {} vs leaf {oracle}",
            priced.greeks.price
        );
        assert!(priced.greeks.price < 1.0, "inverse premium is in coins");
    }

    #[test]
    fn cross_asset_accepts_fx_carry_rejects_absent_carry() {
        // ADR-0008: a cross-asset underlying prices over the SAME carry-producing
        // market as FX. The FX two-rate carry is a valid net cost-of-carry
        // (b = r − r_for, with r_for the asset's carry yield) and is ACCEPTED; an
        // ABSENT carry is refused — no silent fallback to b = r.
        let underlying = celnet_proto::Underlying::equity(celnet_proto::EquityRef::new(
            celnet_proto::Symbol::new("AAPL", "XNAS"),
            "USD",
        ));
        let instr = cross_asset_vanilla(underlying, 100.0);

        // FX two-rate carry → accepted (prices the equity leaf with q = r_for).
        let m_fx = WM::fx(100.0, 0.20, 0.05, 0.03);
        assert!(
            price_instrument(&instr, &m_fx, &conv_set()).is_ok(),
            "cross-asset must accept the FX two-rate carry (b = r − r_for)"
        );

        // Absent carry → rejected (the no-silent-fallback guard).
        let mut m_absent = WM::fx(100.0, 0.20, 0.05, 0.03);
        m_absent.carry = None;
        assert!(matches!(
            price_instrument(&instr, &m_absent, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    #[test]
    fn cross_asset_lsv_is_rejected() {
        let m = cross_asset_market(100.0, 0.20, 0.05, 0.02);
        let underlying = celnet_proto::Underlying::equity(celnet_proto::EquityRef::new(
            celnet_proto::Symbol::new("AAPL", "XNAS"),
            "USD",
        ));
        let mut instr = cross_asset_vanilla(underlying, 100.0);
        instr.pricing_model = celnet_proto::PricingModel::LocalStochVol as i32;
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::UnsupportedModel { .. })
        ));
    }

    #[test]
    fn cross_asset_non_vanilla_product_is_rejected() {
        // Only vanilla is a cross-asset option; a digital on an equity is refused.
        let m = cross_asset_market(100.0, 0.20, 0.05, 0.02);
        let underlying = celnet_proto::Underlying::equity(celnet_proto::EquityRef::new(
            celnet_proto::Symbol::new("AAPL", "XNAS"),
            "USD",
        ));
        let instr = Instrument {
            underlying: Some(underlying),
            expiry_years: 1.0,
            product: Some(Product::Digital(celnet_proto::Digital {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: 100.0,
                style: celnet_proto::DigitalStyle::CashOrNothing as i32,
                payout: 1.0,
            })),
            ..Default::default()
        };
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::UnsupportedModel { .. })
        ));
    }

    #[test]
    fn fx_path_unchanged_when_underlying_absent() {
        // An absent underlying (a pure-context FX price request) takes the FX path
        // byte-identically — the cross-asset branch is not entered.
        let m = market();
        let instr = vanilla_instrument(1.12);
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();
        let direct = celnet_vanilla::greeks(
            OptionType::Call,
            &VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.02, 0.01),
        );
        assert_eq!(priced.greeks.price.to_bits(), direct.price.to_bits());
    }

    // =====================================================================
    // Perpetual American (oneof arm 30) + listed-future option (arm 31).
    // Each arm gates BITWISE against its engine leaf, and the term-validity
    // matrix (perpetual expiry == 0; future outlives option) is enforced as
    // INVALID_ARGUMENT (`PriceError`), never a silent fallback.
    // =====================================================================

    fn perpetual_instrument(
        underlying: Option<celnet_proto::Underlying>,
        option_type: celnet_proto::OptionType,
        strike: f64,
    ) -> Instrument {
        Instrument {
            underlying,
            expiry_years: 0.0,
            side: celnet_proto::Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::PerpetualOption(celnet_proto::PerpetualOption {
                option_type: option_type as i32,
                strike,
                notional: 1_000_000.0,
            })),
            ..Default::default()
        }
    }

    /// Server perpetual on the FX path == the `celnet-exotics` perpetual leaf
    /// over the FX two-rate carry, bitwise — including the verbatim FX rho
    /// pair — with the time Greeks identically zero (the stationary value has
    /// no time dependence) and no fabricated forward delta.
    #[test]
    fn perpetual_matches_exotics_leaf_on_the_fx_path() {
        let m = market(); // spot 1.10, vol 0.10, r_dom 0.02, r_for 0.01
        let instr = perpetual_instrument(None, celnet_proto::OptionType::Call, 1.05);
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();

        let leaf = perpetual_greeks(
            OptionType::Call,
            &PerpetualInputs::new(
                1.10,
                1.05,
                0.10,
                Carry::FxRates {
                    r_dom: 0.02,
                    r_for: 0.01,
                },
            ),
        )
        .unwrap();
        assert_eq!(priced.greeks.price.to_bits(), leaf.price.to_bits());
        assert_eq!(priced.greeks.delta_spot.to_bits(), leaf.delta.to_bits());
        assert_eq!(priced.greeks.gamma.to_bits(), leaf.gamma.to_bits());
        assert_eq!(priced.greeks.vega.to_bits(), leaf.vega.to_bits());
        match leaf.rates {
            RateSensitivities::Fx { rho_dom, rho_for } => {
                assert_eq!(priced.greeks.rho_dom.to_bits(), rho_dom.to_bits());
                assert_eq!(priced.greeks.rho_for.to_bits(), rho_for.to_bits());
            }
            RateSensitivities::Carry { .. } => panic!("FX carry must tag the FX rho pair"),
        }
        // Time-homogeneous: theta/charm/color are the exact zeros of the
        // stationary value; delta_forward has no tenor to define it.
        for g in [
            priced.greeks.theta,
            priced.greeks.charm,
            priced.greeks.color,
            priced.greeks.delta_forward,
        ] {
            assert_eq!(g.to_bits(), 0.0_f64.to_bits());
        }
        // Exact closed form ⇒ no Monte-Carlo standard error.
        assert_eq!(priced.std_error, None);
        assert_eq!(priced.resolved_strike.to_bits(), 1.05_f64.to_bits());
    }

    /// Server perpetual on a cross-asset underlying == the leaf over the
    /// generalized cost-of-carry arm, bitwise, with the carry-tagged rhos
    /// projected through the same `RateSensitivities` bijection every carry
    /// leaf uses; an absent carry is refused (the ADR-0008 guard).
    #[test]
    fn perpetual_on_cross_asset_carry_matches_leaf_with_rho_bijection() {
        let (r, b) = (0.08, 0.04);
        let m = cross_asset_market(100.0, 0.30, r, b);
        let underlying = celnet_proto::Underlying::equity(celnet_proto::EquityRef::new(
            celnet_proto::Symbol::new("AAPL", "XNAS"),
            "USD",
        ));
        let instr = perpetual_instrument(
            Some(underlying.clone()),
            celnet_proto::OptionType::Call,
            100.0,
        );
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();

        let leaf = perpetual_greeks(
            OptionType::Call,
            &PerpetualInputs::new(100.0, 100.0, 0.30, Carry::CostOfCarry { r, b }),
        )
        .unwrap();
        assert_eq!(priced.greeks.price.to_bits(), leaf.price.to_bits());
        assert_eq!(priced.greeks.delta_spot.to_bits(), leaf.delta.to_bits());
        assert_eq!(priced.greeks.gamma.to_bits(), leaf.gamma.to_bits());
        assert_eq!(priced.greeks.vega.to_bits(), leaf.vega.to_bits());
        match leaf.rates {
            RateSensitivities::Carry {
                discount_rho,
                carry_rho,
            } => {
                assert_eq!(
                    priced.greeks.rho_dom.to_bits(),
                    (discount_rho + carry_rho).to_bits()
                );
                assert_eq!(priced.greeks.rho_for.to_bits(), (-carry_rho).to_bits());
            }
            RateSensitivities::Fx { .. } => panic!("cost-of-carry must tag the carry rho pair"),
        }

        // Absent carry → refused, never a silent b = r fallback.
        let mut m_absent = cross_asset_market(100.0, 0.30, r, b);
        m_absent.carry = None;
        let instr_absent =
            perpetual_instrument(Some(underlying), celnet_proto::OptionType::Call, 100.0);
        assert!(matches!(
            price_instrument(&instr_absent, &m_absent, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    /// The FD-completed cross sensitivities (vanna / volga / speed / zomma)
    /// against an INDEPENDENT reference: central differences of the leaf's
    /// ANALYTIC delta / gamma / vega strips (first differences of exact
    /// derivatives at a different step), a route disjoint from the server's
    /// second differences of the price.
    #[test]
    fn perpetual_cross_sensitivities_match_fd_of_analytic_strip() {
        let (s, k, v, r_dom, r_for) = (1.30, 1.25, 0.10, 0.05, 0.01);
        let m = WM::fx(s, v, r_dom, r_for);
        let carry = Carry::FxRates { r_dom, r_for };
        for (opt_wire, opt) in [
            (celnet_proto::OptionType::Call, OptionType::Call),
            (celnet_proto::OptionType::Put, OptionType::Put),
        ] {
            let instr = perpetual_instrument(None, opt_wire, k);
            let priced = price_instrument(&instr, &m, &conv_set()).unwrap();

            let leaf = |spot: f64, vol: f64| {
                perpetual_greeks(opt, &PerpetualInputs::new(spot, k, vol, carry)).unwrap()
            };
            let h_v = 1e-6;
            let h_s = s * 1e-6;
            let vanna_ref = (leaf(s, v + h_v).delta - leaf(s, v - h_v).delta) / (2.0 * h_v);
            let volga_ref = (leaf(s, v + h_v).vega - leaf(s, v - h_v).vega) / (2.0 * h_v);
            let speed_ref = (leaf(s + h_s, v).gamma - leaf(s - h_s, v).gamma) / (2.0 * h_s);
            let zomma_ref = (leaf(s, v + h_v).gamma - leaf(s, v - h_v).gamma) / (2.0 * h_v);

            assert!(
                is_close(priced.greeks.vanna, vanna_ref, 1e-4, 1e-7),
                "{opt:?} vanna {} vs analytic-FD {vanna_ref}",
                priced.greeks.vanna
            );
            assert!(
                is_close(priced.greeks.volga, volga_ref, 1e-4, 1e-6),
                "{opt:?} volga {} vs analytic-FD {volga_ref}",
                priced.greeks.volga
            );
            assert!(
                is_close(priced.greeks.speed, speed_ref, 1e-2, 1e-7),
                "{opt:?} speed {} vs analytic-FD {speed_ref}",
                priced.greeks.speed
            );
            assert!(
                is_close(priced.greeks.zomma, zomma_ref, 1e-3, 1e-6),
                "{opt:?} zomma {} vs analytic-FD {zomma_ref}",
                priced.greeks.zomma
            );
        }
    }

    /// A perpetual with a non-zero (or non-finite) expiry is rejected — the
    /// arm has no expiry to encode, and a stray expiry is never silently
    /// ignored.
    #[test]
    fn perpetual_with_nonzero_expiry_is_rejected() {
        let m = market();
        let mut instr = perpetual_instrument(None, celnet_proto::OptionType::Call, 1.05);
        instr.expiry_years = 0.25;
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
        instr.expiry_years = f64::NAN;
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    /// The leaf's documented domain: a perpetual under a negative discount
    /// rate has no finite value — refused, never priced through to a NaN.
    #[test]
    fn perpetual_negative_discount_rate_is_rejected() {
        let m = WM::fx(1.10, 0.10, -0.01, 0.0);
        let instr = perpetual_instrument(None, celnet_proto::OptionType::Put, 1.05);
        assert!(matches!(
            price_instrument(&instr, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }

    /// The leaf's carry-domain law on the wire: an FX perpetual CALL with
    /// `r_for < 0` has `b = r_dom − r_for > r_dom = r` strictly — the value
    /// diverges (no finite value exists), so the request is refused as the
    /// typed INVALID_ARGUMENT, mirroring the negative-discount-rate refusal
    /// (adversarial-verify refutation of the old `V = S` pin, which sat below
    /// the same-terms long-dated European — an internal arbitrage). The PUT on
    /// the very same market is unaffected and still prices.
    #[test]
    fn perpetual_call_with_carry_exceeding_discount_is_rejected() {
        let m = WM::fx(1.25, 0.10, 0.02, -0.005); // b = 0.025 > r = 0.02
        let call = perpetual_instrument(None, celnet_proto::OptionType::Call, 1.10);
        assert!(matches!(
            price_instrument(&call, &m, &conv_set()),
            Err(PriceError::Domain(
                "a perpetual call with carry exceeding the discount rate has no finite value"
            ))
        ));
        // Puts price on the y₂ branch for every carry: finite, never refused.
        let put = perpetual_instrument(None, celnet_proto::OptionType::Put, 1.10);
        let priced = price_instrument(&put, &m, &conv_set()).unwrap();
        assert!(priced.greeks.price.is_finite() && priced.greeks.price >= 0.0);
    }

    /// Neither new arm has an LSV (FX vol-surface) booking model: selecting it
    /// is a clear typed refusal naming the product, never a silent analytic
    /// fallback.
    #[test]
    fn perpetual_and_listed_future_reject_lsv_model() {
        let m = market();
        let mut perpetual = perpetual_instrument(None, celnet_proto::OptionType::Call, 1.05);
        perpetual.pricing_model = celnet_proto::PricingModel::LocalStochVol as i32;
        assert!(matches!(
            price_instrument(&perpetual, &m, &conv_set()),
            Err(PriceError::UnsupportedModel {
                product: "perpetual_option",
                ..
            })
        ));

        let mut future_option =
            listed_future_instrument(None, 90.0, 0.5, 0.55, celnet_proto::Margining::EquityStyle);
        future_option.pricing_model = celnet_proto::PricingModel::LocalStochVol as i32;
        assert!(matches!(
            price_instrument(&future_option, &m, &conv_set()),
            Err(PriceError::UnsupportedModel {
                product: "listed_future_option",
                ..
            })
        ));
    }

    fn listed_future_instrument(
        underlying: Option<celnet_proto::Underlying>,
        strike: f64,
        expiry: f64,
        future_expiry: f64,
        margining: celnet_proto::Margining,
    ) -> Instrument {
        Instrument {
            underlying,
            expiry_years: expiry,
            side: celnet_proto::Side::Buy as i32,
            pricing_model: celnet_proto::PricingModel::Default as i32,
            product: Some(Product::ListedFutureOption(
                celnet_proto::ListedFutureOption {
                    future_symbol: Some(celnet_proto::Symbol::new("BRN-DEC26", "IFEU")),
                    future_expiry_years: future_expiry,
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike,
                    notional: 1_000.0,
                    margining: margining as i32,
                },
            )),
            ..Default::default()
        }
    }

    fn brent_underlying() -> celnet_proto::Underlying {
        celnet_proto::Underlying::commodity(celnet_proto::CommodityRef::new(
            celnet_proto::Symbol::new("BRENT", ""),
            "USD",
        ))
    }

    /// Server listed-future option (equity-style premium) == the Black-76
    /// `on_future` leaf under the same margining, bitwise — the full strip
    /// through the shared carry-rho bijection.
    #[test]
    fn listed_future_option_equity_style_matches_black76_leaf() {
        let m = cross_asset_market(85.0, 0.30, 0.05, 0.0);
        let instr = listed_future_instrument(
            Some(brent_underlying()),
            90.0,
            0.5,
            0.55,
            celnet_proto::Margining::EquityStyle,
        );
        let priced = price_instrument(&instr, &m, &conv_set()).unwrap();

        let inputs = CommodityInputs::on_future(85.0, 90.0, 0.30, 0.5, 0.05);
        let leaf = celnet_commodity_vanilla::greeks_with_margining(
            OptionType::Call,
            CommodityMargining::EquityStyle,
            &inputs,
        );
        assert_eq!(priced.greeks.price.to_bits(), leaf.price.to_bits());
        assert_eq!(
            priced.greeks.delta_spot.to_bits(),
            leaf.delta_spot.to_bits()
        );
        assert_eq!(
            priced.greeks.delta_forward.to_bits(),
            leaf.delta_forward.to_bits()
        );
        assert_eq!(priced.greeks.gamma.to_bits(), leaf.gamma.to_bits());
        assert_eq!(priced.greeks.vega.to_bits(), leaf.vega.to_bits());
        assert_eq!(priced.greeks.theta.to_bits(), leaf.theta.to_bits());
        match leaf.rates {
            RateSensitivities::Carry {
                discount_rho,
                carry_rho,
            } => {
                assert_eq!(
                    priced.greeks.rho_dom.to_bits(),
                    (discount_rho + carry_rho).to_bits()
                );
                assert_eq!(priced.greeks.rho_for.to_bits(), (-carry_rho).to_bits());
            }
            RateSensitivities::Fx { .. } => panic!("the Black-76 leaf tags the carry rho pair"),
        }
        assert_eq!(priced.std_error, None);
        assert_eq!(priced.resolved_strike.to_bits(), 90.0_f64.to_bits());
    }

    /// Futures-style (daily-margined) premium == the undiscounted leaf bitwise,
    /// with the HONEST zero discount-rho (the margin sweep removes the
    /// financing leg, so `∂V/∂r ≡ 0` exactly — a financial statement, not a
    /// numerical shortcut) surfacing as `rho_dom == −rho_for` on the wire; and
    /// the undiscounted premium strictly dominates the discounted one at
    /// `r > 0`.
    #[test]
    fn listed_future_option_futures_style_has_honest_zero_discount_rho() {
        let m = cross_asset_market(85.0, 0.30, 0.05, 0.0);
        let futures_style = listed_future_instrument(
            Some(brent_underlying()),
            90.0,
            0.5,
            0.55,
            celnet_proto::Margining::FuturesStyle,
        );
        let priced = price_instrument(&futures_style, &m, &conv_set()).unwrap();

        let inputs = CommodityInputs::on_future(85.0, 90.0, 0.30, 0.5, 0.05);
        let leaf_price = celnet_commodity_vanilla::futures_style_price(OptionType::Call, &inputs);
        assert_eq!(priced.greeks.price.to_bits(), leaf_price.to_bits());

        // discount_rho ≡ 0 ⇒ the bijection collapses to rho_dom = carry_rho =
        // −rho_for, exactly.
        assert_eq!(
            priced.greeks.rho_dom.to_bits(),
            (-priced.greeks.rho_for).to_bits()
        );

        // Undiscounted vs discounted: V_futures-style > V_equity-style for r > 0.
        let equity_style = listed_future_instrument(
            Some(brent_underlying()),
            90.0,
            0.5,
            0.55,
            celnet_proto::Margining::EquityStyle,
        );
        let discounted = price_instrument(&equity_style, &m, &conv_set()).unwrap();
        assert!(
            priced.greeks.price > discounted.greeks.price,
            "undiscounted {} must dominate discounted {}",
            priced.greeks.price,
            discounted.greeks.price
        );
    }

    /// Asset-class agnosticism: the SAME terms priced over a commodity
    /// underlying (cross-asset branch) and over the FX path (absent
    /// underlying, FX two-rate context with the same discount rate) hit the
    /// identical Black-76 `on_future` engine bitwise — the quoted futures
    /// price already embodies the underlying's carry, so only `r` enters.
    #[test]
    fn listed_future_option_is_asset_class_agnostic() {
        let commodity = price_instrument(
            &listed_future_instrument(
                Some(brent_underlying()),
                90.0,
                0.5,
                0.55,
                celnet_proto::Margining::EquityStyle,
            ),
            &cross_asset_market(85.0, 0.30, 0.05, 0.0),
            &conv_set(),
        )
        .unwrap();
        let fx_path = price_instrument(
            &listed_future_instrument(None, 90.0, 0.5, 0.55, celnet_proto::Margining::EquityStyle),
            &WM::fx(85.0, 0.30, 0.05, 0.02),
            &conv_set(),
        )
        .unwrap();
        assert_eq!(
            commodity.greeks.price.to_bits(),
            fx_path.greeks.price.to_bits()
        );
        assert_eq!(
            commodity.greeks.delta_spot.to_bits(),
            fx_path.greeks.delta_spot.to_bits()
        );
        assert_eq!(
            commodity.greeks.vega.to_bits(),
            fx_path.greeks.vega.to_bits()
        );
        assert_eq!(
            commodity.greeks.rho_dom.to_bits(),
            fx_path.greeks.rho_dom.to_bits()
        );
    }

    /// The listed-future term-validity matrix: the future must outlive the
    /// option (`future_expiry_years >= expiry_years > 0`), the contract
    /// identity must be present, the margining tag known and the strike
    /// positive — each violation INVALID_ARGUMENT, never clamped.
    #[test]
    fn listed_future_option_validity_rejections() {
        let m = cross_asset_market(85.0, 0.30, 0.05, 0.0);

        // Future expires before the option.
        let dies_early = listed_future_instrument(
            Some(brent_underlying()),
            90.0,
            0.5,
            0.4,
            celnet_proto::Margining::EquityStyle,
        );
        assert!(matches!(
            price_instrument(&dies_early, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));

        // Zero option expiry (the shared positive-expiry guard).
        let no_expiry = listed_future_instrument(
            Some(brent_underlying()),
            90.0,
            0.0,
            0.55,
            celnet_proto::Margining::EquityStyle,
        );
        assert!(matches!(
            price_instrument(&no_expiry, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));

        // Missing future contract identity.
        let mut no_symbol = listed_future_instrument(
            Some(brent_underlying()),
            90.0,
            0.5,
            0.55,
            celnet_proto::Margining::EquityStyle,
        );
        if let Some(Product::ListedFutureOption(o)) = no_symbol.product.as_mut() {
            o.future_symbol = None;
        }
        assert!(matches!(
            price_instrument(&no_symbol, &m, &conv_set()),
            Err(PriceError::MissingField("ListedFutureOption.future_symbol"))
        ));

        // Out-of-range margining tag.
        let mut bad_margining = listed_future_instrument(
            Some(brent_underlying()),
            90.0,
            0.5,
            0.55,
            celnet_proto::Margining::EquityStyle,
        );
        if let Some(Product::ListedFutureOption(o)) = bad_margining.product.as_mut() {
            o.margining = 7;
        }
        assert!(matches!(
            price_instrument(&bad_margining, &m, &conv_set()),
            Err(PriceError::UnknownEnum {
                kind: "Margining",
                tag: 7
            })
        ));

        // Non-positive strike.
        let zero_strike = listed_future_instrument(
            Some(brent_underlying()),
            0.0,
            0.5,
            0.55,
            celnet_proto::Margining::EquityStyle,
        );
        assert!(matches!(
            price_instrument(&zero_strike, &m, &conv_set()),
            Err(PriceError::Domain(_))
        ));
    }
}
