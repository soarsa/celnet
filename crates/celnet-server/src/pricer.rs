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
    AtmConvention, Cut, DayCount, DeltaConvention, Greeks, OptionType, PremiumStyle, Settlement,
    VanillaInputs,
};

use celnet_core::FlatSmile;
use celnet_exotics::{
    AnalyticAsian, AveragingSchedule, BarrierKind as ExBarrierKind, BarrierStyle, Cliquet,
    CliquetMcConfig, CliquetSchedule, DigitalKind, DoubleBarrierKnockOut, DoubleNoTouch,
    ForwardStart, QuantoParams, RebateTiming, SingleBarrier as ExSingleBarrier, VarSwapContext,
    cliquet_price_capped_mc, cliquet_price_plain, curran_price, digital_price,
    double_knock_out_price, double_no_touch_price, double_touch_price, fair_variance,
    fair_volatility, forward_start_price, no_touch_price, one_touch_price, quanto_digital_price,
    quanto_vanilla_price, single_barrier_price, turnbull_wakeman_price,
};

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
}

impl core::fmt::Display for PriceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PriceError::MissingField(field) => write!(f, "missing required field `{field}`"),
            PriceError::UnknownEnum { kind, tag } => write!(f, "unknown {kind} enum tag: {tag}"),
            PriceError::EmptyProduct => write!(f, "instrument carried no product variant"),
            PriceError::DeltaSolve(e) => write!(f, "delta→strike inversion failed: {e:?}"),
            PriceError::Domain(why) => write!(f, "input out of pricing domain: {why}"),
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
        market.r_dom,
        market.r_for,
    )
}

/// Build a [`VarSwapContext`] (forward + carry) for the swap/replication math
/// from the wire market context and the instrument expiry. The forward is
/// `F = S·e^{(r_d−r_f)T}`, derived via [`VanillaInputs::from_inputs`] semantics.
fn var_swap_context(market: &WireMarketContext, expiry_years: f64) -> VarSwapContext {
    let template = inputs_at(market, expiry_years, market.spot, market.vol);
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

/// Shock a market context's spot multiplicatively.
fn bump_spot(m: &WireMarketContext, rel: f64) -> WireMarketContext {
    WireMarketContext {
        spot: m.spot * (1.0 + rel),
        ..*m
    }
}

/// Shock a market context's vol additively.
fn bump_vol(m: &WireMarketContext, d: f64) -> WireMarketContext {
    WireMarketContext {
        vol: m.vol + d,
        ..*m
    }
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
        let up = price(&WireMarketContext {
            r_dom: market.r_dom + h_r,
            ..*market
        });
        let dn = price(&WireMarketContext {
            r_dom: market.r_dom - h_r,
            ..*market
        });
        (up - dn) / (2.0 * h_r)
    };
    let rho_for = {
        let up = price(&WireMarketContext {
            r_for: market.r_for + h_r,
            ..*market
        });
        let dn = price(&WireMarketContext {
            r_for: market.r_for - h_r,
            ..*market
        });
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
    let delta_forward = delta_spot * celnet_core::math::exp(market.r_for * expiry_years);

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
    if expiry <= 0.0 || !expiry.is_finite() {
        return Err(PriceError::Domain("expiry_years must be positive"));
    }
    let product = instrument
        .product
        .as_ref()
        .ok_or(PriceError::EmptyProduct)?;

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
                single_barrier_price(&inputs_at(m, expiry, strike, m.vol), ex_spec)
            };
            let price_at = |t: f64, m: &WireMarketContext| {
                single_barrier_price(&inputs_at(m, t, strike, m.vol), ex_spec)
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
                let ko_px = double_knock_out_price(&ki, ko);
                if knock_in {
                    celnet_vanilla::price(option_type, &ki) - ko_px
                } else {
                    ko_px
                }
            };
            let price_at = move |t: f64, m: &WireMarketContext| {
                let ki = inputs_at(m, t, strike, m.vol);
                let ko_px = double_knock_out_price(&ki, ko);
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
                payout * digital_price(kind, &inputs_at(m, expiry, strike, m.vol))
            };
            let price_at = move |t: f64, m: &WireMarketContext| {
                payout * digital_price(kind, &inputs_at(m, t, strike, m.vol))
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
                        &inputs_at(m, expiry, lower, m.vol),
                        lower,
                        rebate,
                        RebateTiming::AtHit,
                    )
                }),
                celnet_proto::TouchKind::NoTouch => Box::new(move |m: &WireMarketContext| {
                    no_touch_price(&inputs_at(m, expiry, lower, m.vol), lower, rebate)
                }),
                celnet_proto::TouchKind::DoubleNoTouch => {
                    if !(lower > 0.0 && lower < upper) {
                        return Err(PriceError::Domain(
                            "double-no-touch corridor must satisfy 0 < lower < upper",
                        ));
                    }
                    Box::new(move |m: &WireMarketContext| {
                        double_no_touch_price(
                            &inputs_at(m, expiry, lower, m.vol),
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
                            &inputs_at(m, expiry, lower, m.vol),
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
                        &inputs_at(m, t_exp, lower, m.vol),
                        lower,
                        rebate,
                        RebateTiming::AtHit,
                    ),
                    celnet_proto::TouchKind::NoTouch => {
                        no_touch_price(&inputs_at(m, t_exp, lower, m.vol), lower, rebate)
                    }
                    celnet_proto::TouchKind::DoubleNoTouch => double_no_touch_price(
                        &inputs_at(m, t_exp, lower, m.vol),
                        DoubleNoTouch::new(lower, upper, rebate),
                    ),
                    celnet_proto::TouchKind::DoubleOneTouch => double_touch_price(
                        &inputs_at(m, t_exp, lower, m.vol),
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
                let inputs = inputs_at(m, expiry, strike, m.vol);
                match method {
                    celnet_proto::AsianMethod::Curran => curran_price(&inputs, spec),
                    celnet_proto::AsianMethod::TurnbullWakeman => {
                        turnbull_wakeman_price(&inputs, spec)
                    }
                }
            };
            let price_at = move |t: f64, m: &WireMarketContext| -> f64 {
                let inputs = inputs_at(m, t, strike, m.vol);
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
                let inputs = inputs_at(m, expiry, m.spot, m.vol);
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
                let inputs = inputs_at(m, t, m.spot, m.vol);
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
                moneyness * market.spot * ((market.r_dom - market.r_for) * reset).exp();
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
            let build = move |m: &WireMarketContext, t: f64| -> (VanillaInputs, Cliquet) {
                let inputs = inputs_at(m, t, m.spot, m.vol);
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
                let inputs = inputs_at(m, expiry, strike, m.vol);
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
                let inputs = inputs_at(m, t, strike, m.vol);
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
        WM {
            spot: 1.10,
            vol: 0.10,
            r_dom: 0.02,
            r_for: 0.01,
        }
    }

    fn vanilla_instrument(strike: f64) -> Instrument {
        Instrument {
            pair: None,
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
            product: Some(Product::Vanilla(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(strike)),
                }),
            })),
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
            pair: None,
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
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
        let base =
            single_barrier_price(&VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01), spec);
        assert!(is_close(priced.greeks.price, base, 1e-12, 1e-12));
        // Manual delta over a 1% bump, compared loosely to the FD field.
        let up = single_barrier_price(
            &VanillaInputs::new(1.111, 1.10, 0.10, 1.0, 0.02, 0.01),
            spec,
        );
        let dn = single_barrier_price(
            &VanillaInputs::new(1.089, 1.10, 0.10, 1.0, 0.02, 0.01),
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
            pair: None,
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
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
            pair: None,
            tenor: None,
            expiry_years: 1.0,
            quantity: None,
            side: celnet_proto::Side::Buy as i32,
            solve: None,
            product: Some(product),
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
        let template = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom, m.r_for);
        let ctx = VarSwapContext::from_inputs(&template);
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
            let m = WM {
                spot: 1.30,
                vol: sigma,
                r_dom: 0.03,
                r_for: 0.01,
            };
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
        let template = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom, m.r_for);
        let ctx = VarSwapContext::from_inputs(&template);
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
            let m = WM {
                spot: 1.30,
                vol: sigma,
                r_dom: 0.03,
                r_for: 0.01,
            };
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
        let inputs = VanillaInputs::new(m.spot, 1.10, m.vol, 1.0, m.r_dom, m.r_for);
        let spec = AnalyticAsian::fresh_discrete(OptionType::Call, 1.10, 12);
        let oracle = curran_price(&inputs, spec);
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
        let inputs = VanillaInputs::new(m.spot, 1.12, m.vol, 1.0, m.r_dom, m.r_for);
        let spec = AnalyticAsian::fresh_continuous(OptionType::Put, 1.12);
        let oracle = turnbull_wakeman_price(&inputs, spec);
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
        let inputs = VanillaInputs::new(m.spot, 1.08, m.vol, 1.0, m.r_dom, m.r_for);
        let spec = AnalyticAsian {
            option: OptionType::Call,
            strike: 1.08,
            schedule: AveragingSchedule::Discrete { future_obs: 9 },
            t_start: 0.0,
            elapsed_avg: 1.095,
            elapsed_weight: 0.25,
        };
        let oracle = curran_price(&inputs, spec);
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
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom, m.r_for);
        let oracle = forward_start_price(
            &inputs,
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
            &VanillaInputs::new(m.spot, moneyness * m.spot, m.vol, 1.0, m.r_dom, m.r_for),
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
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom, m.r_for);
        let mut sum = 0.0;
        for k in 1..=periods {
            let reset = (k - 1) as f64 / periods as f64;
            let expiry = k as f64 / periods as f64;
            sum += forward_start_price(
                &inputs,
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
        let inputs = VanillaInputs::new(m.spot, m.spot, m.vol, 1.0, m.r_dom, m.r_for);
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
            &inputs,
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
        let inputs = VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom, m.r_for);
        let oracle = quanto_vanilla_price(OptionType::Call, &inputs, QuantoParams::new(0.09, -0.3));
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
        let inputs = VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom, m.r_for);
        let oracle = quanto_digital_price(OptionType::Put, &inputs, QuantoParams::new(0.07, 0.4));
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
            &VanillaInputs::new(m.spot, strike, m.vol, 1.0, m.r_dom, m.r_for),
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
}
