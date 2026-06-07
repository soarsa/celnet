//! JSON codec between [`serde_json::Value`] and the [`celnet_proto`] wire types.
//!
//! The WebSocket mirror is **not** a second contract — it is a second *encoding* of
//! the single, current [`celnet_proto`] contract, exactly as gRPC is its protobuf
//! encoding (`CLAUDE.md` rule 9: one current contract, no fork). Every JSON object
//! maps a proto message field-for-field, by the proto field's snake_case name;
//! every proto enum is carried by its canonical proto **enum number** (the same
//! numeric tag `prost` assigns), so the JSON form is unambiguous and reversible.
//! Optional (presence-tracked) fields are `null`/absent when `None`.
//!
//! This file holds only the (de)serialization; the dispatch onto the shared
//! services lives in [`super`]. The conversions go through the generated `prost`
//! types directly — there is no parallel DTO hierarchy to drift.

use serde_json::{Map, Value, json};

use celnet_proto::{
    Accumulator, AdditiveRisk, AggregateRiskRequest, AggregateRiskResponse, AmericanOption,
    ArbReport, AsianOption, BrokerQuoteSet, BucketedRisk, CcyExposureLeg, CcyPair, Cliquet,
    Conventions, CrossGamma, Digital, DoubleBarrier, DrillRiskRequest, DrillRiskResponse,
    EntitlementPrincipal, EntitlementRule, Execute, Executed, Execution, FixingSchedule,
    ForwardStart, GetSmileRequest, Greeks, Instrument, Leg, LimitStatusRequest,
    LimitStatusResponse, LimitUtilization, ListPositionsRequest, ListPositionsResponse, Lookback,
    MarkSurfaceRequest, MarkSurfaceResponse, MarketContext, Modify, NonAdditiveRisk, NumeraireRate,
    OrgKey, PriceRequest, PriceResponse, Quantity, Quanto, Quote, QuoteAccept, QuoteReject,
    QuoteRequest, RejectAck, ReportingNumeraire, Resync, RiskBucketRequest, RiskNode, RiskPosition,
    RiskScope, ScenarioPoint, ScenarioRequest, ScenarioResponse, ShockAxis, SingleBarrier, Smile,
    SmilePoint, Snapshot, Solve, Strategy, StrategyKind, StreamEnd, StreamReject, StrikeOrDelta,
    Subscribe, SubscriptionId, Tarf, Tenor, Touch, TradableToken, TwoWayPrice, Unsubscribe, Update,
    Vanilla, VanillaInputs, VarianceSwap, VegaLadderBucket, VegaPillar, VolatilitySwap,
    WindowBarrier, instrument, shock_axis, strike_or_delta, tenor,
};

/// A codec error: a malformed or out-of-contract JSON message. Carries a
/// human-readable reason echoed back to the client as a typed `error` frame.
#[derive(Debug, Clone)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CodecError {}

/// Shorthand for building a codec error.
fn err(msg: impl Into<String>) -> CodecError {
    CodecError(msg.into())
}

type Result<T> = std::result::Result<T, CodecError>;

// ---------------------------------------------------------------------------
// scalar field accessors (decode side)
// ---------------------------------------------------------------------------

/// The object body of a JSON value, or an error if it is not an object.
fn obj<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>> {
    v.as_object()
        .ok_or_else(|| err(format!("{what} must be a JSON object")))
}

/// A required `f64` field.
fn f64_field(o: &Map<String, Value>, key: &str) -> Result<f64> {
    o.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| err(format!("missing or non-numeric field `{key}`")))
}

/// An optional `f64` field defaulting to `0.0` (proto3 scalar default).
fn f64_or_zero(o: &Map<String, Value>, key: &str) -> f64 {
    o.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// A required `u64` field.
fn u64_field(o: &Map<String, Value>, key: &str) -> Result<u64> {
    o.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| err(format!("missing or non-integer field `{key}`")))
}

/// An optional `u64` field defaulting to `0`.
fn u64_or_zero(o: &Map<String, Value>, key: &str) -> u64 {
    o.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// An optional presence-tracked `u64` field (`null`/absent ⇒ `None`).
fn opt_u64(o: &Map<String, Value>, key: &str) -> Option<u64> {
    match o.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => v.as_u64(),
    }
}

/// An optional presence-tracked `f64` field (`null`/absent ⇒ `None`).
fn opt_f64(o: &Map<String, Value>, key: &str) -> Option<f64> {
    match o.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => v.as_f64(),
    }
}

/// An optional `bool` field defaulting to `false`.
fn bool_or_false(o: &Map<String, Value>, key: &str) -> bool {
    o.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A required `String` field.
fn string_field(o: &Map<String, Value>, key: &str) -> Result<String> {
    o.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| err(format!("missing or non-string field `{key}`")))
}

/// An optional `String` field defaulting to empty.
fn string_or_empty(o: &Map<String, Value>, key: &str) -> String {
    o.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// An i32 proto enum-tag field defaulting to the proto3 zero value.
fn enum_or_zero(o: &Map<String, Value>, key: &str) -> i32 {
    o.get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

/// A nested object field decoded with `f`, required.
fn nested<T>(o: &Map<String, Value>, key: &str, f: impl FnOnce(&Value) -> Result<T>) -> Result<T> {
    let v = o
        .get(key)
        .ok_or_else(|| err(format!("missing nested field `{key}`")))?;
    f(v)
}

/// A nested object field decoded with `f`, optional (`null`/absent ⇒ `None`).
fn opt_nested<T>(
    o: &Map<String, Value>,
    key: &str,
    f: impl FnOnce(&Value) -> Result<T>,
) -> Result<Option<T>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => f(v).map(Some),
    }
}

/// A repeated `f64` field defaulting to empty.
fn f64_vec(o: &Map<String, Value>, key: &str) -> Vec<f64> {
    o.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// vocabulary
// ---------------------------------------------------------------------------

fn ccy_pair_from_json(v: &Value) -> Result<CcyPair> {
    let o = obj(v, "pair")?;
    Ok(CcyPair {
        base: string_field(o, "base")?,
        quote: string_field(o, "quote")?,
    })
}

fn ccy_pair_to_json(p: &CcyPair) -> Value {
    json!({ "base": p.base, "quote": p.quote })
}

fn tenor_from_json(v: &Value) -> Result<Tenor> {
    let o = obj(v, "tenor")?;
    Ok(Tenor {
        unit: enum_or_zero(o, "unit"),
        count: u32::try_from(u64_or_zero(o, "count")).unwrap_or(0),
        broken_date: opt_nested(o, "brokenDate", broken_date_from_json)?,
    })
}

fn broken_date_from_json(v: &Value) -> Result<celnet_proto::BrokenDate> {
    let o = obj(v, "brokenDate")?;
    let year = o
        .get("year")
        .and_then(Value::as_i64)
        .and_then(|y| i32::try_from(y).ok())
        .unwrap_or(0);
    Ok(celnet_proto::BrokenDate {
        year,
        month: u32::try_from(u64_or_zero(o, "month")).unwrap_or(0),
        day: u32::try_from(u64_or_zero(o, "day")).unwrap_or(0),
    })
}

fn conventions_from_json(v: &Value) -> Result<Conventions> {
    let o = obj(v, "conventions")?;
    Ok(Conventions {
        delta_convention: enum_or_zero(o, "delta_convention"),
        atm_convention: enum_or_zero(o, "atm_convention"),
        premium_style: enum_or_zero(o, "premium_style"),
        cut: enum_or_zero(o, "cut"),
        day_count: enum_or_zero(o, "day_count"),
        settlement: enum_or_zero(o, "settlement"),
    })
}

fn conventions_to_json(c: &Conventions) -> Value {
    json!({
        "delta_convention": c.delta_convention,
        "atm_convention": c.atm_convention,
        "premium_style": c.premium_style,
        "cut": c.cut,
        "day_count": c.day_count,
        "settlement": c.settlement,
    })
}

fn market_context_from_json(v: &Value) -> Result<MarketContext> {
    let o = obj(v, "market")?;
    Ok(MarketContext {
        spot: f64_field(o, "spot")?,
        vol: f64_field(o, "vol")?,
        r_dom: f64_or_zero(o, "r_dom"),
        r_for: f64_or_zero(o, "r_for"),
    })
}

fn market_context_to_json(m: &MarketContext) -> Value {
    json!({ "spot": m.spot, "vol": m.vol, "r_dom": m.r_dom, "r_for": m.r_for })
}

fn quantity_from_json(v: &Value) -> Result<Quantity> {
    let o = obj(v, "quantity")?;
    Ok(Quantity {
        notional: f64_or_zero(o, "notional"),
        base_ccy: bool_or_false(o, "base_ccy"),
    })
}

fn solve_from_json(v: &Value) -> Result<Solve> {
    let o = obj(v, "solve")?;
    Ok(Solve {
        target: enum_or_zero(o, "target"),
        target_premium: f64_or_zero(o, "target_premium"),
    })
}

fn greeks_to_json(g: &Greeks) -> Value {
    json!({
        "price": g.price,
        "delta_spot": g.delta_spot,
        "delta_forward": g.delta_forward,
        "gamma": g.gamma,
        "vega": g.vega,
        "theta": g.theta,
        "rho_dom": g.rho_dom,
        "rho_for": g.rho_for,
        "vanna": g.vanna,
        "volga": g.volga,
        "charm": g.charm,
        "speed": g.speed,
        "zomma": g.zomma,
        "color": g.color,
    })
}

fn two_way_to_json(p: &TwoWayPrice) -> Value {
    json!({ "bid": p.bid, "offer": p.offer })
}

fn tradable_to_json(t: &TradableToken) -> Value {
    json!({
        "token": t.token,
        "side": t.side,
        "premium": t.premium,
        "valid_until_nanos": t.valid_until_nanos,
    })
}

// ---------------------------------------------------------------------------
// instrument oneof
// ---------------------------------------------------------------------------

fn strike_or_delta_from_json(v: &Value) -> Result<StrikeOrDelta> {
    let o = obj(v, "strike")?;
    // Exactly one of `strike` / `delta` is set (mirrors the proto oneof).
    let spec = if let Some(s) = o.get("strike").and_then(Value::as_f64) {
        Some(strike_or_delta::Spec::Strike(s))
    } else if let Some(d) = o.get("delta").and_then(Value::as_f64) {
        Some(strike_or_delta::Spec::Delta(d))
    } else {
        return Err(err("strike must carry exactly one of `strike` or `delta`"));
    };
    Ok(StrikeOrDelta { spec })
}

fn vanilla_from_json(v: &Value) -> Result<Vanilla> {
    let o = obj(v, "vanilla")?;
    Ok(Vanilla {
        option_type: enum_or_zero(o, "option_type"),
        strike: Some(nested(o, "strike", strike_or_delta_from_json)?),
    })
}

fn leg_from_json(v: &Value) -> Result<Leg> {
    let o = obj(v, "leg")?;
    Ok(Leg {
        option_type: enum_or_zero(o, "option_type"),
        strike: Some(nested(o, "strike", strike_or_delta_from_json)?),
        side: enum_or_zero(o, "side"),
        ratio: f64_or_zero(o, "ratio"),
    })
}

fn strategy_from_json(v: &Value) -> Result<Strategy> {
    let o = obj(v, "strategy")?;
    let legs = o
        .get("legs")
        .and_then(Value::as_array)
        .ok_or_else(|| err("strategy needs a `legs` array"))?
        .iter()
        .map(leg_from_json)
        .collect::<Result<Vec<_>>>()?;
    Ok(Strategy {
        kind: enum_or_zero(o, "kind"),
        legs,
    })
}

fn single_barrier_from_json(v: &Value) -> Result<SingleBarrier> {
    let o = obj(v, "single_barrier")?;
    Ok(SingleBarrier {
        vanilla: Some(nested(o, "vanilla", vanilla_from_json)?),
        kind: enum_or_zero(o, "kind"),
        side: enum_or_zero(o, "side"),
        barrier: f64_field(o, "barrier")?,
        rebate: f64_or_zero(o, "rebate"),
        monitoring: enum_or_zero(o, "monitoring"),
    })
}

fn double_barrier_from_json(v: &Value) -> Result<DoubleBarrier> {
    let o = obj(v, "double_barrier")?;
    Ok(DoubleBarrier {
        vanilla: Some(nested(o, "vanilla", vanilla_from_json)?),
        kind: enum_or_zero(o, "kind"),
        lower_barrier: f64_field(o, "lower_barrier")?,
        upper_barrier: f64_field(o, "upper_barrier")?,
        rebate: f64_or_zero(o, "rebate"),
        monitoring: enum_or_zero(o, "monitoring"),
    })
}

fn digital_from_json(v: &Value) -> Result<Digital> {
    let o = obj(v, "digital")?;
    Ok(Digital {
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        style: enum_or_zero(o, "style"),
        payout: f64_or_zero(o, "payout"),
    })
}

fn touch_from_json(v: &Value) -> Result<Touch> {
    let o = obj(v, "touch")?;
    Ok(Touch {
        kind: enum_or_zero(o, "kind"),
        lower_barrier: f64_field(o, "lower_barrier")?,
        upper_barrier: f64_or_zero(o, "upper_barrier"),
        rebate: f64_or_zero(o, "rebate"),
        monitoring: enum_or_zero(o, "monitoring"),
    })
}

fn variance_swap_from_json(v: &Value) -> Result<VarianceSwap> {
    let o = obj(v, "variance_swap")?;
    Ok(VarianceSwap {
        strike_vol: f64_or_zero(o, "strike_vol"),
    })
}

fn volatility_swap_from_json(v: &Value) -> Result<VolatilitySwap> {
    let o = obj(v, "volatility_swap")?;
    Ok(VolatilitySwap {
        strike_vol: f64_or_zero(o, "strike_vol"),
    })
}

fn asian_option_from_json(v: &Value) -> Result<AsianOption> {
    let o = obj(v, "asian_option")?;
    let observations = u32::try_from(u64_or_zero(o, "observations"))
        .map_err(|_| err("asian_option.observations out of range"))?;
    Ok(AsianOption {
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        averaging: enum_or_zero(o, "averaging"),
        observations,
        method: enum_or_zero(o, "method"),
        elapsed_avg: f64_or_zero(o, "elapsed_avg"),
        elapsed_weight: f64_or_zero(o, "elapsed_weight"),
    })
}

fn forward_start_from_json(v: &Value) -> Result<ForwardStart> {
    let o = obj(v, "forward_start")?;
    Ok(ForwardStart {
        option_type: enum_or_zero(o, "option_type"),
        moneyness: f64_field(o, "moneyness")?,
        reset: f64_field(o, "reset")?,
    })
}

fn cliquet_from_json(v: &Value) -> Result<Cliquet> {
    let o = obj(v, "cliquet")?;
    let periods = u32::try_from(u64_or_zero(o, "periods"))
        .map_err(|_| err("cliquet.periods out of range"))?;
    let mc_pairs = u32::try_from(u64_or_zero(o, "mc_pairs"))
        .map_err(|_| err("cliquet.mc_pairs out of range"))?;
    Ok(Cliquet {
        option_type: enum_or_zero(o, "option_type"),
        moneyness: f64_field(o, "moneyness")?,
        periods,
        local_floor: opt_f64(o, "local_floor"),
        local_cap: opt_f64(o, "local_cap"),
        global_floor: opt_f64(o, "global_floor"),
        global_cap: opt_f64(o, "global_cap"),
        mc_pairs,
        mc_seed: u64_or_zero(o, "mc_seed"),
    })
}

fn quanto_from_json(v: &Value) -> Result<Quanto> {
    let o = obj(v, "quanto")?;
    Ok(Quanto {
        payoff: enum_or_zero(o, "payoff"),
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        conversion_vol: f64_or_zero(o, "conversion_vol"),
        correlation: f64_or_zero(o, "correlation"),
    })
}

/// Decode a nested `FixingSchedule` (the `schedule` body inside a TARF /
/// accumulator product).
fn fixing_schedule_from_json(v: &Value) -> Result<FixingSchedule> {
    let o = obj(v, "schedule")?;
    Ok(FixingSchedule {
        fixing_years: f64_vec(o, "fixing_years"),
        fixing_notional: f64_or_zero(o, "fixing_notional"),
    })
}

fn tarf_from_json(v: &Value) -> Result<Tarf> {
    let o = obj(v, "tarf")?;
    let mc_pairs =
        u32::try_from(u64_or_zero(o, "mc_pairs")).map_err(|_| err("tarf.mc_pairs out of range"))?;
    let schedule = o
        .get("schedule")
        .map(fixing_schedule_from_json)
        .transpose()?;
    Ok(Tarf {
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        target: f64_field(o, "target")?,
        leverage: f64_or_zero(o, "leverage"),
        redemption: enum_or_zero(o, "redemption"),
        schedule,
        mc_pairs,
        mc_seed: u64_or_zero(o, "mc_seed"),
    })
}

fn accumulator_from_json(v: &Value) -> Result<Accumulator> {
    let o = obj(v, "accumulator")?;
    let mc_pairs = u32::try_from(u64_or_zero(o, "mc_pairs"))
        .map_err(|_| err("accumulator.mc_pairs out of range"))?;
    let schedule = o
        .get("schedule")
        .map(fixing_schedule_from_json)
        .transpose()?;
    Ok(Accumulator {
        pivot: f64_field(o, "pivot")?,
        barrier: f64_field(o, "barrier")?,
        leverage: f64_or_zero(o, "leverage"),
        monitoring: enum_or_zero(o, "monitoring"),
        schedule,
        mc_pairs,
        mc_seed: u64_or_zero(o, "mc_seed"),
    })
}

fn lookback_from_json(v: &Value) -> Result<Lookback> {
    let o = obj(v, "lookback")?;
    let observations = u32::try_from(u64_or_zero(o, "observations"))
        .map_err(|_| err("lookback.observations out of range"))?;
    let mc_pairs = u32::try_from(u64_or_zero(o, "mc_pairs"))
        .map_err(|_| err("lookback.mc_pairs out of range"))?;
    Ok(Lookback {
        style: enum_or_zero(o, "style"),
        option_type: enum_or_zero(o, "option_type"),
        monitoring: enum_or_zero(o, "monitoring"),
        strike: f64_or_zero(o, "strike"),
        observations,
        mc_pairs,
        mc_seed: u64_or_zero(o, "mc_seed"),
    })
}

fn window_barrier_from_json(v: &Value) -> Result<WindowBarrier> {
    let o = obj(v, "window_barrier")?;
    let mc_pairs = u32::try_from(u64_or_zero(o, "mc_pairs"))
        .map_err(|_| err("window_barrier.mc_pairs out of range"))?;
    let mc_steps = u32::try_from(u64_or_zero(o, "mc_steps"))
        .map_err(|_| err("window_barrier.mc_steps out of range"))?;
    Ok(WindowBarrier {
        vanilla: Some(nested(o, "vanilla", vanilla_from_json)?),
        barrier: f64_field(o, "barrier")?,
        side: enum_or_zero(o, "side"),
        window_start: f64_field(o, "window_start")?,
        window_end: f64_field(o, "window_end")?,
        mc_pairs,
        mc_steps,
        mc_seed: u64_or_zero(o, "mc_seed"),
    })
}

fn american_from_json(v: &Value) -> Result<AmericanOption> {
    let o = obj(v, "american")?;
    let lsm_paths = u32::try_from(u64_or_zero(o, "lsm_paths"))
        .map_err(|_| err("american.lsm_paths out of range"))?;
    let lsm_exercise_dates = u32::try_from(u64_or_zero(o, "lsm_exercise_dates"))
        .map_err(|_| err("american.lsm_exercise_dates out of range"))?;
    // The optional Bermudan date set: a JSON array of year-fractions.
    let bermudan_dates = match o.get("bermudan_dates") {
        None | Some(Value::Null) => Vec::new(),
        Some(arr) => arr
            .as_array()
            .ok_or_else(|| err("american.bermudan_dates must be an array of numbers"))?
            .iter()
            .map(|d| {
                d.as_f64()
                    .ok_or_else(|| err("american.bermudan_dates entries must be numbers"))
            })
            .collect::<Result<Vec<f64>>>()?,
    };
    Ok(AmericanOption {
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        exercise_style: enum_or_zero(o, "exercise_style"),
        bermudan_dates,
        lsm_paths,
        lsm_exercise_dates,
        lsm_seed: u64_or_zero(o, "lsm_seed"),
    })
}

/// Decode the instrument `product` oneof. The JSON carries exactly one of the
/// product keys (`vanilla`, `strategy`, `single_barrier`, `double_barrier`,
/// `digital`, `touch`, `variance_swap`, `volatility_swap`, `asian_option`,
/// `forward_start`, `cliquet`, `quanto`, `tarf`, `accumulator`, `lookback`) — the
/// same shape as the proto oneof.
fn product_from_json(o: &Map<String, Value>) -> Result<instrument::Product> {
    // Each product variant nests its body under its own key (mirroring the proto
    // oneof field names); descend into that body before decoding.
    if let Some(v) = o.get("vanilla") {
        Ok(instrument::Product::Vanilla(vanilla_from_json(v)?))
    } else if let Some(v) = o.get("strategy") {
        Ok(instrument::Product::Strategy(strategy_from_json(v)?))
    } else if let Some(v) = o.get("single_barrier") {
        Ok(instrument::Product::SingleBarrier(
            single_barrier_from_json(v)?,
        ))
    } else if let Some(v) = o.get("double_barrier") {
        Ok(instrument::Product::DoubleBarrier(
            double_barrier_from_json(v)?,
        ))
    } else if let Some(v) = o.get("digital") {
        Ok(instrument::Product::Digital(digital_from_json(v)?))
    } else if let Some(v) = o.get("touch") {
        Ok(instrument::Product::Touch(touch_from_json(v)?))
    } else if let Some(v) = o.get("variance_swap") {
        Ok(instrument::Product::VarianceSwap(variance_swap_from_json(
            v,
        )?))
    } else if let Some(v) = o.get("volatility_swap") {
        Ok(instrument::Product::VolatilitySwap(
            volatility_swap_from_json(v)?,
        ))
    } else if let Some(v) = o.get("asian_option") {
        Ok(instrument::Product::AsianOption(asian_option_from_json(v)?))
    } else if let Some(v) = o.get("forward_start") {
        Ok(instrument::Product::ForwardStart(forward_start_from_json(
            v,
        )?))
    } else if let Some(v) = o.get("cliquet") {
        Ok(instrument::Product::Cliquet(cliquet_from_json(v)?))
    } else if let Some(v) = o.get("quanto") {
        Ok(instrument::Product::Quanto(quanto_from_json(v)?))
    } else if let Some(v) = o.get("tarf") {
        Ok(instrument::Product::Tarf(tarf_from_json(v)?))
    } else if let Some(v) = o.get("accumulator") {
        Ok(instrument::Product::Accumulator(accumulator_from_json(v)?))
    } else if let Some(v) = o.get("lookback") {
        Ok(instrument::Product::Lookback(lookback_from_json(v)?))
    } else if let Some(v) = o.get("window_barrier") {
        Ok(instrument::Product::WindowBarrier(
            window_barrier_from_json(v)?,
        ))
    } else if let Some(v) = o.get("american") {
        Ok(instrument::Product::American(american_from_json(v)?))
    } else {
        Err(err(
            "instrument needs exactly one product (vanilla / strategy / \
             single_barrier / double_barrier / digital / touch / variance_swap / \
             volatility_swap / asian_option / forward_start / cliquet / quanto / \
             tarf / accumulator / lookback / window_barrier / american)",
        ))
    }
}

pub(super) fn instrument_from_json(v: &Value) -> Result<Instrument> {
    let o = obj(v, "instrument")?;
    Ok(Instrument {
        pair: opt_nested(o, "pair", ccy_pair_from_json)?,
        tenor: opt_nested(o, "tenor", tenor_from_json)?,
        expiry_years: f64_field(o, "expiry_years")?,
        quantity: opt_nested(o, "quantity", quantity_from_json)?,
        side: enum_or_zero(o, "side"),
        solve: opt_nested(o, "solve", solve_from_json)?,
        // The booking-model selector (absent ⇒ 0 ⇒ PRICING_MODEL_DEFAULT, so the
        // analytic path is unchanged for existing browser requests).
        pricing_model: enum_or_zero(o, "pricing_model"),
        product: Some(product_from_json(o)?),
    })
}

// ---------------------------------------------------------------------------
// RFQ lifecycle (decode requests, encode replies)
// ---------------------------------------------------------------------------

pub(super) fn quote_request_from_json(o: &Map<String, Value>) -> Result<QuoteRequest> {
    Ok(QuoteRequest {
        idempotency_key: string_field(o, "idempotency_key")?,
        instrument: Some(nested(o, "instrument", instrument_from_json)?),
        conventions: Some(nested(o, "conventions", conventions_from_json)?),
        correlation_id: opt_u64(o, "correlation_id"),
        surface_version: opt_u64(o, "surface_version"),
        attribution: opt_nested(o, "attribution", attribution_from_json)?,
    })
}

/// Decode an optional attribution record (book/seat identity) from JSON.
fn attribution_from_json(v: &Value) -> Result<celnet_proto::AttributionRecord> {
    let o = obj(v, "attribution")?;
    Ok(celnet_proto::AttributionRecord {
        quoted_by: opt_nested(o, "quotedBy", book_id_from_json)?,
        held_by: opt_nested(o, "heldBy", book_id_from_json)?,
        won: o.get("won").and_then(Value::as_bool),
        lp_count: o
            .get("lpCount")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
    })
}

/// Decode a book/owner identity from JSON.
fn book_id_from_json(v: &Value) -> Result<celnet_proto::BookId> {
    let o = obj(v, "bookId")?;
    let owner = o.get("owner").map(owner_from_json).transpose()?;
    Ok(celnet_proto::BookId {
        book: string_field(o, "book")?,
        owner,
    })
}

/// Decode an owner seat (human trader OR auto-pricer) from JSON.
fn owner_from_json(v: &Value) -> Result<celnet_proto::Owner> {
    let o = obj(v, "owner")?;
    let seat = if let Some(t) = o.get("trader").and_then(Value::as_str) {
        Some(celnet_proto::owner::Seat::Trader(t.to_owned()))
    } else {
        o.get("autoPricer")
            .and_then(Value::as_str)
            .map(|p| celnet_proto::owner::Seat::AutoPricer(p.to_owned()))
    };
    Ok(celnet_proto::Owner { seat })
}

/// Encode the who's-trading attribution chain to JSON, symmetric with
/// `attribution_from_json` (same camelCase keys the GUI/Excel decoders read).
/// Presence-tracked fields (`held_by`/`won`/`lp_count`) are omitted when absent so
/// the wire shape matches the proto `optional` semantics exactly — an unattributed
/// line carries no `attribution` key (see the call sites' `.map(...)`).
fn attribution_to_json(a: &celnet_proto::AttributionRecord) -> Value {
    let mut m = Map::new();
    if let Some(qb) = a.quoted_by.as_ref() {
        m.insert("quotedBy".to_owned(), book_id_to_json(qb));
    }
    if let Some(hb) = a.held_by.as_ref() {
        m.insert("heldBy".to_owned(), book_id_to_json(hb));
    }
    if let Some(won) = a.won {
        m.insert("won".to_owned(), Value::Bool(won));
    }
    if let Some(lp) = a.lp_count {
        m.insert("lpCount".to_owned(), Value::from(lp));
    }
    Value::Object(m)
}

/// Encode a book/owner identity to JSON, symmetric with `book_id_from_json`.
fn book_id_to_json(b: &celnet_proto::BookId) -> Value {
    let mut m = Map::new();
    m.insert("book".to_owned(), Value::String(b.book.clone()));
    if let Some(o) = b.owner.as_ref() {
        m.insert("owner".to_owned(), owner_to_json(o));
    }
    Value::Object(m)
}

/// Encode an owner seat (human trader OR auto-pricer) to JSON, symmetric with
/// `owner_from_json` (the `trader`/`autoPricer` oneof tag).
fn owner_to_json(o: &celnet_proto::Owner) -> Value {
    let mut m = Map::new();
    match o.seat.as_ref() {
        Some(celnet_proto::owner::Seat::Trader(t)) => {
            m.insert("trader".to_owned(), Value::String(t.clone()));
        }
        Some(celnet_proto::owner::Seat::AutoPricer(p)) => {
            m.insert("autoPricer".to_owned(), Value::String(p.clone()));
        }
        None => {}
    }
    Value::Object(m)
}

pub(super) fn quote_accept_from_json(o: &Map<String, Value>) -> Result<QuoteAccept> {
    Ok(QuoteAccept {
        quote_id: u64_field(o, "quote_id")?,
        idempotency_key: string_or_empty(o, "idempotency_key"),
        side: enum_or_zero(o, "side"),
    })
}

pub(super) fn quote_reject_from_json(o: &Map<String, Value>) -> Result<QuoteReject> {
    Ok(QuoteReject {
        quote_id: u64_field(o, "quote_id")?,
        reason: string_or_empty(o, "reason"),
    })
}

pub(super) fn quote_to_json(q: &Quote) -> Value {
    json!({
        "quote_id": q.quote_id,
        "idempotency_key": q.idempotency_key,
        "price": q.price.as_ref().map(two_way_to_json),
        "greeks": q.greeks.as_ref().map(greeks_to_json),
        "conventions": q.conventions.as_ref().map(conventions_to_json),
        "resolved_strike": q.resolved_strike,
        "epoch_nanos": q.epoch_nanos,
        "valid_until_nanos": q.valid_until_nanos,
        "correlation_id": q.correlation_id,
        "surface_version": q.surface_version,
        "attribution": q.attribution.as_ref().map(attribution_to_json),
        // Presence-tracked MC standard error (set only for MC-priced products);
        // the WS quote path must carry it so GUI/Excel disclose MC uncertainty.
        "price_std_error": q.price_std_error,
    })
}

pub(super) fn execution_to_json(e: &Execution) -> Value {
    json!({
        "execution_id": e.execution_id,
        "quote_id": e.quote_id,
        "side": e.side,
        "traded_premium": e.traded_premium,
        "epoch_nanos": e.epoch_nanos,
        "attribution": e.attribution.as_ref().map(attribution_to_json),
    })
}

pub(super) fn reject_ack_to_json(a: &RejectAck) -> Value {
    json!({ "quote_id": a.quote_id, "epoch_nanos": a.epoch_nanos })
}

// ---------------------------------------------------------------------------
// one-shot pricing
// ---------------------------------------------------------------------------

pub(super) fn price_request_from_json(o: &Map<String, Value>) -> Result<PriceRequest> {
    Ok(PriceRequest {
        request_id: u64_or_zero(o, "request_id"),
        instrument: Some(nested(o, "instrument", instrument_from_json)?),
        market: Some(nested(o, "market", market_context_from_json)?),
        conventions: Some(nested(o, "conventions", conventions_from_json)?),
        correlation_id: opt_u64(o, "correlation_id"),
        surface_version: opt_u64(o, "surface_version"),
    })
}

pub(super) fn price_response_to_json(r: &PriceResponse) -> Value {
    json!({
        "request_id": r.request_id,
        "greeks": r.greeks.as_ref().map(greeks_to_json),
        "resolved_strike": r.resolved_strike,
        "conventions": r.conventions.as_ref().map(conventions_to_json),
        "correlation_id": r.correlation_id,
        "surface_version": r.surface_version,
        // Presence-tracked MC standard error (set only for MC-priced products) so
        // the WS one-shot price path matches the gRPC PriceResponse disclosure.
        "price_std_error": r.price_std_error,
    })
}

// ---------------------------------------------------------------------------
// RFS stream — client control (decode) and server messages (encode)
// ---------------------------------------------------------------------------

fn subscription_id_from_json(v: &Value) -> Result<SubscriptionId> {
    let o = obj(v, "subscription")?;
    Ok(SubscriptionId {
        value: u64_field(o, "value")?,
    })
}

fn subscription_id_to_json(s: &SubscriptionId) -> Value {
    json!({ "value": s.value })
}

pub(super) fn subscribe_from_json(o: &Map<String, Value>) -> Result<Subscribe> {
    Ok(Subscribe {
        subscription: Some(nested(o, "subscription", subscription_id_from_json)?),
        instrument: Some(nested(o, "instrument", instrument_from_json)?),
        conventions: Some(nested(o, "conventions", conventions_from_json)?),
        throttle_nanos: u64_or_zero(o, "throttle_nanos"),
        correlation_id: opt_u64(o, "correlation_id"),
        surface_version: opt_u64(o, "surface_version"),
        attribution: opt_nested(o, "attribution", attribution_from_json)?,
    })
}

pub(super) fn modify_from_json(o: &Map<String, Value>) -> Result<Modify> {
    Ok(Modify {
        subscription: Some(nested(o, "subscription", subscription_id_from_json)?),
        instrument: Some(nested(o, "instrument", instrument_from_json)?),
        conventions: Some(nested(o, "conventions", conventions_from_json)?),
        throttle_nanos: u64_or_zero(o, "throttle_nanos"),
        surface_version: opt_u64(o, "surface_version"),
    })
}

pub(super) fn unsubscribe_from_json(o: &Map<String, Value>) -> Result<Unsubscribe> {
    Ok(Unsubscribe {
        subscription: Some(nested(o, "subscription", subscription_id_from_json)?),
    })
}

pub(super) fn resync_from_json(o: &Map<String, Value>) -> Result<Resync> {
    Ok(Resync {
        subscription: Some(nested(o, "subscription", subscription_id_from_json)?),
        last_sequence: u64_or_zero(o, "last_sequence"),
    })
}

pub(super) fn execute_from_json(o: &Map<String, Value>) -> Result<Execute> {
    Ok(Execute {
        subscription: Some(nested(o, "subscription", subscription_id_from_json)?),
        token: u64_field(o, "token")?,
        idempotency_key: string_or_empty(o, "idempotency_key"),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn market_series_subscribe_from_json(
    o: &Map<String, Value>,
) -> Result<celnet_proto::MarketSeriesSubscribe> {
    let observable = o
        .get("observable")
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| CodecError("market_series_subscribe missing `observable`".to_owned()))?;
    Ok(celnet_proto::MarketSeriesSubscribe {
        subscription: Some(nested(o, "subscription", subscription_id_from_json)?),
        pair: Some(nested(o, "pair", ccy_pair_from_json)?),
        observable,
        tenor: opt_nested(o, "tenor", tenor_from_json)?,
        delta: o.get("delta").and_then(Value::as_f64),
        throttle_nanos: u64_or_zero(o, "throttle_nanos"),
        history_limit: o
            .get("history_limit")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0),
    })
}

pub(super) fn market_series_unsubscribe_from_json(
    o: &Map<String, Value>,
) -> Result<celnet_proto::MarketSeriesUnsubscribe> {
    Ok(celnet_proto::MarketSeriesUnsubscribe {
        subscription: Some(nested(o, "subscription", subscription_id_from_json)?),
    })
}

fn tradable_vec(v: &[TradableToken]) -> Value {
    Value::Array(v.iter().map(tradable_to_json).collect())
}

fn snapshot_to_json(s: &Snapshot) -> Value {
    json!({
        "subscription": s.subscription.as_ref().map(subscription_id_to_json),
        "sequence": s.sequence,
        "price": s.price.as_ref().map(two_way_to_json),
        "greeks": s.greeks.as_ref().map(greeks_to_json),
        "vol": s.vol,
        "conventions": s.conventions.as_ref().map(conventions_to_json),
        "resolved_strike": s.resolved_strike,
        "tradable": tradable_vec(&s.tradable),
        "surface_version": s.surface_version,
        "correlation_id": s.correlation_id,
        "epoch_nanos": s.epoch_nanos,
        "attribution": s.attribution.as_ref().map(attribution_to_json),
    })
}

fn update_to_json(u: &Update) -> Value {
    json!({
        "subscription": u.subscription.as_ref().map(subscription_id_to_json),
        "sequence": u.sequence,
        "price": u.price.as_ref().map(two_way_to_json),
        "greeks": u.greeks.as_ref().map(greeks_to_json),
        "vol": u.vol,
        "tradable": tradable_vec(&u.tradable),
        "surface_version": u.surface_version,
        "epoch_nanos": u.epoch_nanos,
    })
}

fn executed_to_json(e: &Executed) -> Value {
    json!({
        "subscription": e.subscription.as_ref().map(subscription_id_to_json),
        "token": e.token,
        "execution_id": e.execution_id,
        "side": e.side,
        "traded_premium": e.traded_premium,
        "correlation_id": e.correlation_id,
        "epoch_nanos": e.epoch_nanos,
        "attribution": e.attribution.as_ref().map(attribution_to_json),
    })
}

fn stream_reject_to_json(r: &StreamReject) -> Value {
    json!({
        "subscription": r.subscription.as_ref().map(subscription_id_to_json),
        "token": r.token,
        "reason": r.reason,
        "correlation_id": r.correlation_id,
        "epoch_nanos": r.epoch_nanos,
    })
}

fn stream_end_to_json(e: &StreamEnd) -> Value {
    json!({
        "subscription": e.subscription.as_ref().map(subscription_id_to_json),
        "reason": e.reason,
    })
}

/// Encode a server stream message as a type-tagged JSON frame: `{"type": "...",
/// ...fields}`. The tag is the proto oneof variant name (snake_case) so a client
/// dispatches on `type` exactly as a gRPC client matches the `oneof`.
pub(super) fn server_stream_message_to_json(
    msg: &celnet_proto::ServerStreamMessage,
) -> Option<Value> {
    use celnet_proto::server_stream_message::Message;
    let inner = msg.message.as_ref()?;
    let (tag, body) = match inner {
        Message::Snapshot(s) => ("snapshot", snapshot_to_json(s)),
        Message::Update(u) => ("update", update_to_json(u)),
        Message::Heartbeat(h) => (
            "heartbeat",
            json!({
                "subscription": h.subscription.as_ref().map(subscription_id_to_json),
                "sequence": h.sequence,
                "epoch_nanos": h.epoch_nanos,
                // Server observability surfaced on the beat (additive): the exact
                // ring conflation-drop count, the drain-side price p50/p99/p99.9
                // (ns), and the surface-version / correlation provenance echo.
                "conflation_drops": h.conflation_drops,
                "server_price_p50_nanos": h.server_price_p50_nanos,
                "server_price_p99_nanos": h.server_price_p99_nanos,
                "server_price_p999_nanos": h.server_price_p999_nanos,
                "surface_version": h.surface_version,
                "correlation_id": h.correlation_id,
            }),
        ),
        Message::StreamEnd(e) => ("stream_end", stream_end_to_json(e)),
        Message::Executed(e) => ("executed", executed_to_json(e)),
        Message::StreamReject(r) => ("stream_reject", stream_reject_to_json(r)),
        Message::MarketSeriesSnapshot(s) => {
            ("market_series_snapshot", market_series_snapshot_to_json(s))
        }
        Message::MarketSeriesPoint(p) => ("market_series_point", market_series_point_to_json(p)),
    };
    Some(tagged(tag, body))
}

fn market_series_point_to_json(p: &celnet_proto::MarketSeriesPoint) -> Value {
    json!({
        "subscription": p.subscription.as_ref().map(subscription_id_to_json),
        "sequence": p.sequence,
        "value": p.value,
        "epoch_nanos": p.epoch_nanos,
    })
}

fn market_series_snapshot_to_json(s: &celnet_proto::MarketSeriesSnapshot) -> Value {
    json!({
        "subscription": s.subscription.as_ref().map(subscription_id_to_json),
        "sequence": s.sequence,
        "pair": s.pair.as_ref().map(ccy_pair_to_json),
        "observable": s.observable,
        "points": s.points.iter().map(market_series_point_to_json).collect::<Vec<_>>(),
        "epoch_nanos": s.epoch_nanos,
    })
}

// ---------------------------------------------------------------------------
// surface — read / mark / scenario
// ---------------------------------------------------------------------------

fn broker_quote_set_from_json(v: &Value) -> Result<BrokerQuoteSet> {
    let o = obj(v, "broker_quote")?;
    Ok(BrokerQuoteSet {
        tenor_years: f64_field(o, "tenor_years")?,
        atm_vol: f64_field(o, "atm_vol")?,
        rr_25: f64_or_zero(o, "rr_25"),
        bf_25: f64_or_zero(o, "bf_25"),
        rr_10: f64_or_zero(o, "rr_10"),
        bf_10: f64_or_zero(o, "bf_10"),
        has_ten_delta: bool_or_false(o, "has_ten_delta"),
    })
}

fn broker_quote_set_to_json(b: &BrokerQuoteSet) -> Value {
    json!({
        "tenor_years": b.tenor_years,
        "atm_vol": b.atm_vol,
        "rr_25": b.rr_25,
        "bf_25": b.bf_25,
        "rr_10": b.rr_10,
        "bf_10": b.bf_10,
        "has_ten_delta": b.has_ten_delta,
    })
}

pub(super) fn get_smile_request_from_json(o: &Map<String, Value>) -> Result<GetSmileRequest> {
    Ok(GetSmileRequest {
        pair: opt_nested(o, "pair", ccy_pair_from_json)?,
        tenor_years: f64_field(o, "tenor_years")?,
        conventions: Some(nested(o, "conventions", conventions_from_json)?),
    })
}

pub(super) fn mark_surface_request_from_json(o: &Map<String, Value>) -> Result<MarkSurfaceRequest> {
    let broker_quotes = o
        .get("broker_quotes")
        .and_then(Value::as_array)
        .ok_or_else(|| err("mark_surface needs a `broker_quotes` array"))?
        .iter()
        .map(broker_quote_set_from_json)
        .collect::<Result<Vec<_>>>()?;
    Ok(MarkSurfaceRequest {
        pair: opt_nested(o, "pair", ccy_pair_from_json)?,
        broker_quotes,
        conventions: Some(nested(o, "conventions", conventions_from_json)?),
        smile_model: opt_smile_model(o, "smile_model"),
    })
}

/// Decode an optional smile-model selector from JSON. Accepts either the proto3
/// enum integer or its `SMILE_MODEL_*` string name; absent ⇒ `None` (server
/// default calibration).
fn opt_smile_model(o: &Map<String, Value>, key: &str) -> Option<i32> {
    match o.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_i64().and_then(|v| i32::try_from(v).ok()),
        Some(Value::String(s)) => match s.as_str() {
            "SMILE_MODEL_MARKET_HEDGE" => Some(celnet_proto::SmileModel::MarketHedge as i32),
            "SMILE_MODEL_STOCHASTIC_VOL" => Some(celnet_proto::SmileModel::StochasticVol as i32),
            "SMILE_MODEL_PARAMETRIC" => Some(celnet_proto::SmileModel::Parametric as i32),
            "SMILE_MODEL_PARAMETRIC_SURFACE" => {
                Some(celnet_proto::SmileModel::ParametricSurface as i32)
            }
            _ => None,
        },
        Some(_) => None,
    }
}

fn smile_point_to_json(p: &SmilePoint) -> Value {
    json!({ "delta": p.delta, "tenor_years": p.tenor_years, "vol": p.vol })
}

fn arb_report_to_json(a: &ArbReport) -> Value {
    json!({
        "butterfly_arbitrage_free": a.butterfly_arbitrage_free,
        "calendar_arbitrage_free": a.calendar_arbitrage_free,
        "worst_density": a.worst_density,
        "note": a.note,
        // The TYPED, authoritative calibration-family provenance: the numeric
        // SmileModel tag plus a stable label. A consumer reads `smile_model` (or
        // `smile_model_label`) directly — never the `model=` token in `note`.
        "smile_model": a.smile_model,
        "smile_model_label": smile_model_label(a.smile_model),
    })
}

/// A stable, machine-friendly label for a wire [`celnet_proto::SmileModel`] tag,
/// surfaced alongside the numeric provenance so a JSON consumer can render the
/// calibration family without re-deriving the enum. Vendor-/method-neutral by name
/// (guardrail #8). An unknown tag is reported honestly as `unknown`.
fn smile_model_label(tag: i32) -> &'static str {
    match celnet_proto::SmileModel::try_from(tag) {
        Ok(celnet_proto::SmileModel::MarketHedge) => "market-hedge",
        Ok(celnet_proto::SmileModel::StochasticVol) => "stochastic-vol",
        Ok(celnet_proto::SmileModel::Parametric) => "parametric",
        Ok(celnet_proto::SmileModel::ParametricSurface) => "parametric-surface",
        Ok(celnet_proto::SmileModel::ExtendedSurface) => "extended-surface",
        Err(_) => "unknown",
    }
}

fn smile_to_json(s: &Smile) -> Value {
    json!({
        "pair": s.pair.as_ref().map(ccy_pair_to_json),
        "tenor_years": s.tenor_years,
        "broker_quotes": s.broker_quotes.as_ref().map(broker_quote_set_to_json),
        "points": Value::Array(s.points.iter().map(smile_point_to_json).collect()),
        "conventions": s.conventions.as_ref().map(conventions_to_json),
        "arbitrage": s.arbitrage.as_ref().map(arb_report_to_json),
        "epoch_nanos": s.epoch_nanos,
    })
}

pub(super) fn smile_reply_to_json(s: &Smile) -> Value {
    smile_to_json(s)
}

pub(super) fn mark_surface_response_to_json(r: &MarkSurfaceResponse) -> Value {
    json!({
        "pair": r.pair.as_ref().map(ccy_pair_to_json),
        "surface_version": r.surface_version,
        "smiles": Value::Array(r.smiles.iter().map(smile_to_json).collect()),
        "epoch_nanos": r.epoch_nanos,
    })
}

fn shock_axis_from_json(v: &Value) -> Result<ShockAxis> {
    let o = obj(v, "axis")?;
    Ok(ShockAxis {
        factor: enum_or_zero(o, "factor"),
        relative: bool_or_false(o, "relative"),
        steps: f64_vec(o, "steps"),
    })
}

fn vega_bucket_from_json(v: &Value) -> Result<celnet_proto::VegaBucket> {
    let o = obj(v, "vega_pillar")?;
    Ok(celnet_proto::VegaBucket {
        tenor_years: f64_field(o, "tenor_years")?,
        delta: f64_or_zero(o, "delta"),
        vega: 0.0,
    })
}

fn cross_gamma_pair_from_json(v: &Value) -> Result<CrossGamma> {
    let o = obj(v, "cross_gamma_pair")?;
    Ok(CrossGamma {
        factor_a: enum_or_zero(o, "factor_a"),
        factor_b: enum_or_zero(o, "factor_b"),
        value: 0.0,
    })
}

fn risk_bucket_request_from_json(v: &Value) -> Result<RiskBucketRequest> {
    let o = obj(v, "risk_buckets")?;
    let vega_pillars = o
        .get("vega_pillars")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(vega_bucket_from_json)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let cross_gamma_pairs = o
        .get("cross_gamma_pairs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(cross_gamma_pair_from_json)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(RiskBucketRequest {
        vega_pillars,
        cross_gamma_pairs,
        roll_horizons_years: f64_vec(o, "roll_horizons_years"),
    })
}

pub(super) fn scenario_request_from_json(o: &Map<String, Value>) -> Result<ScenarioRequest> {
    let axes = o
        .get("axes")
        .and_then(Value::as_array)
        .ok_or_else(|| err("scenario needs an `axes` array"))?
        .iter()
        .map(shock_axis_from_json)
        .collect::<Result<Vec<_>>>()?;
    Ok(ScenarioRequest {
        instrument: Some(nested(o, "instrument", instrument_from_json)?),
        base_market: Some(nested(o, "base_market", market_context_from_json)?),
        conventions: Some(nested(o, "conventions", conventions_from_json)?),
        axes,
        expiry_years: f64_or_zero(o, "expiry_years"),
        risk_buckets: opt_nested(o, "risk_buckets", risk_bucket_request_from_json)?,
        smile_model: opt_smile_model(o, "smile_model"),
    })
}

fn scenario_point_to_json(p: &ScenarioPoint) -> Value {
    json!({
        "applied_shocks": p.applied_shocks,
        "shocked_market": p.shocked_market.as_ref().map(market_context_to_json),
        "greeks": p.greeks.as_ref().map(greeks_to_json),
        "expiry_years": p.expiry_years,
    })
}

fn vega_bucket_to_json(b: &celnet_proto::VegaBucket) -> Value {
    json!({ "tenor_years": b.tenor_years, "delta": b.delta, "vega": b.vega })
}

fn cross_gamma_to_json(c: &CrossGamma) -> Value {
    json!({ "factor_a": c.factor_a, "factor_b": c.factor_b, "value": c.value })
}

fn bucketed_risk_to_json(r: &BucketedRisk) -> Value {
    json!({
        "vega_buckets": Value::Array(r.vega_buckets.iter().map(vega_bucket_to_json).collect()),
        "cross_gammas": Value::Array(r.cross_gammas.iter().map(cross_gamma_to_json).collect()),
        "theta_roll": r.theta_roll,
        "roll_horizons_years": r.roll_horizons_years,
    })
}

pub(super) fn scenario_response_to_json(r: &ScenarioResponse) -> Value {
    json!({
        "points": Value::Array(r.points.iter().map(scenario_point_to_json).collect()),
        "bucketed_risk": r.bucketed_risk.as_ref().map(bucketed_risk_to_json),
    })
}

// ---------------------------------------------------------------------------
// risk: server-side hierarchical risk (RiskService)
//
// The SAME contract, second encoding (rule 9): every key is the proto snake_case
// field name; every enum rides by its proto enum NUMBER; optional fields are
// `null`/absent when `None`. `attribution` reuses the existing camelCase chain.
// ---------------------------------------------------------------------------

fn vanilla_inputs_to_json(i: &VanillaInputs) -> Value {
    json!({
        "spot": i.spot, "strike": i.strike, "vol": i.vol,
        "t": i.t, "r_dom": i.r_dom, "r_for": i.r_for,
    })
}

fn org_key_to_json(k: &OrgKey) -> Value {
    json!({
        "trader": k.trader, "book": k.book, "desk": k.desk,
        "ccy_pair": k.ccy_pair.as_ref().map(ccy_pair_to_json),
        "location": k.location, "entity": k.entity,
    })
}

// `RiskPosition` is only ever ENCODED outbound by the server (built from the live
// book); the server never decodes a wire `RiskPosition` inbound (clients send
// scope/principal/numeraire requests, not positions), so there is no
// `risk_position_from_json` — adding an unused decoder would be dead code.
fn risk_position_to_json(p: &RiskPosition) -> Value {
    json!({
        "position_id": p.position_id,
        "org": p.org.as_ref().map(org_key_to_json),
        "option_type": p.option_type,
        "notional_base": p.notional_base,
        "inputs": p.inputs.as_ref().map(vanilla_inputs_to_json),
        "quoted_delta": p.quoted_delta,
        "premium_style": p.premium_style,
        "surface_version": p.surface_version,
        "attribution": p.attribution.as_ref().map(attribution_to_json),
    })
}

fn risk_scope_from_json(v: &Value) -> Result<RiskScope> {
    let o = obj(v, "scope")?;
    Ok(RiskScope {
        dimension: enum_or_zero(o, "dimension"),
        value: u64_or_zero(o, "value"),
    })
}

fn risk_scope_to_json(s: &RiskScope) -> Value {
    json!({ "dimension": s.dimension, "value": s.value })
}

fn entitlement_rule_from_json(v: &Value) -> Result<EntitlementRule> {
    let o = obj(v, "rule")?;
    let scopes = o
        .get("scopes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(risk_scope_from_json)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(EntitlementRule { scopes })
}

fn principal_from_json(v: &Value) -> Result<EntitlementPrincipal> {
    let o = obj(v, "principal")?;
    let rules = |key: &str| -> Result<Vec<EntitlementRule>> {
        o.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(entitlement_rule_from_json)
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()
            .map(Option::unwrap_or_default)
    };
    Ok(EntitlementPrincipal {
        grant_all: bool_or_false(o, "grant_all"),
        grants: rules("grants")?,
        denies: rules("denies")?,
    })
}

fn numeraire_from_json(v: &Value) -> Result<ReportingNumeraire> {
    let o = obj(v, "numeraire")?;
    let rates = o
        .get("rates")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|r| {
                    let ro = obj(r, "rate")?;
                    Ok(NumeraireRate {
                        ccy: string_field(ro, "ccy")?,
                        rate: f64_or_zero(ro, "rate"),
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(ReportingNumeraire {
        numeraire: string_field(o, "numeraire")?,
        rates,
    })
}

fn vega_pillar_from_json(v: &Value) -> Result<VegaPillar> {
    let o = obj(v, "pillar")?;
    Ok(VegaPillar {
        tenor_days: u32::try_from(u64_or_zero(o, "tenor_days")).unwrap_or(0),
        delta_bp: o
            .get("delta_bp")
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .unwrap_or(0),
    })
}

fn vega_pillar_to_json(p: &VegaPillar) -> Value {
    json!({ "tenor_days": p.tenor_days, "delta_bp": p.delta_bp })
}

fn vega_pillars_from_json(o: &Map<String, Value>) -> Result<Vec<VegaPillar>> {
    o.get("vega_pillars")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(vega_pillar_from_json)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn ccy_exposure_leg_to_json(l: &CcyExposureLeg) -> Value {
    json!({ "ccy": l.ccy, "amount": l.amount })
}

fn vega_ladder_bucket_to_json(b: &VegaLadderBucket) -> Value {
    json!({
        "pillar": b.pillar.as_ref().map(vega_pillar_to_json),
        "vega": b.vega,
    })
}

fn additive_risk_to_json(a: &AdditiveRisk) -> Value {
    json!({
        "delta_numeraire": a.delta_numeraire,
        "delta_vector": Value::Array(a.delta_vector.iter().map(ccy_exposure_leg_to_json).collect()),
        "gamma": a.gamma,
        "vega_numeraire": a.vega_numeraire,
        "theta": a.theta,
        "vanna": a.vanna,
        "volga": a.volga,
        "charm": a.charm,
        "speed": a.speed,
        "zomma": a.zomma,
        "color": a.color,
        "premium_numeraire": a.premium_numeraire,
        "vega_ladder": Value::Array(a.vega_ladder.iter().map(vega_ladder_bucket_to_json).collect()),
    })
}

fn nonadditive_risk_to_json(n: &NonAdditiveRisk) -> Value {
    json!({
        "var": n.var,
        "es": n.es,
        "var_alpha": n.var_alpha,
        "curvature_spot": n.curvature_spot,
    })
}

fn risk_node_to_json(n: &RiskNode) -> Value {
    json!({
        "dimension": n.dimension,
        "group": n.group,
        "additive": n.additive.as_ref().map(additive_risk_to_json),
        "nonadditive": n.nonadditive.as_ref().map(nonadditive_risk_to_json),
        "position_count": n.position_count,
    })
}

fn limit_utilization_to_json(u: &LimitUtilization) -> Value {
    json!({
        "metric": u.metric,
        "vega_pillar": u.vega_pillar.as_ref().map(vega_pillar_to_json),
        "tenor_days": u.tenor_days,
        "cap": u.cap,
        "exposure": u.exposure,
        "ratio": u.ratio,
        "status": u.status,
        "enforcement": u.enforcement,
        "headroom": u.headroom,
    })
}

pub(super) fn list_positions_request_from_json(
    o: &Map<String, Value>,
) -> Result<ListPositionsRequest> {
    Ok(ListPositionsRequest {
        scope: opt_nested(o, "scope", risk_scope_from_json)?,
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn list_positions_response_to_json(r: &ListPositionsResponse) -> Value {
    json!({
        "positions": Value::Array(r.positions.iter().map(risk_position_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn aggregate_risk_request_from_json(
    o: &Map<String, Value>,
) -> Result<AggregateRiskRequest> {
    Ok(AggregateRiskRequest {
        dimension: enum_or_zero(o, "dimension"),
        numeraire: opt_nested(o, "numeraire", numeraire_from_json)?,
        principal: opt_nested(o, "principal", principal_from_json)?,
        scope: opt_nested(o, "scope", risk_scope_from_json)?,
        vega_pillars: vega_pillars_from_json(o)?,
        var_spot_shocks: f64_vec(o, "var_spot_shocks"),
        var_alpha: f64_or_zero(o, "var_alpha"),
        curvature_risk_weight: f64_or_zero(o, "curvature_risk_weight"),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn aggregate_risk_response_to_json(r: &AggregateRiskResponse) -> Value {
    json!({
        "dimension": r.dimension,
        "numeraire": r.numeraire,
        "nodes": Value::Array(r.nodes.iter().map(risk_node_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn drill_risk_request_from_json(o: &Map<String, Value>) -> Result<DrillRiskRequest> {
    Ok(DrillRiskRequest {
        node: opt_nested(o, "node", risk_scope_from_json)?,
        child_dimension: enum_or_zero(o, "child_dimension"),
        numeraire: opt_nested(o, "numeraire", numeraire_from_json)?,
        principal: opt_nested(o, "principal", principal_from_json)?,
        vega_pillars: vega_pillars_from_json(o)?,
        include_children: bool_or_false(o, "include_children"),
        include_positions: bool_or_false(o, "include_positions"),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn drill_risk_response_to_json(r: &DrillRiskResponse) -> Value {
    json!({
        "node": r.node.as_ref().map(risk_scope_to_json),
        "children": Value::Array(r.children.iter().map(risk_node_to_json).collect()),
        "positions": Value::Array(r.positions.iter().map(risk_position_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn limit_status_request_from_json(o: &Map<String, Value>) -> Result<LimitStatusRequest> {
    Ok(LimitStatusRequest {
        scope: opt_nested(o, "scope", risk_scope_from_json)?,
        numeraire: opt_nested(o, "numeraire", numeraire_from_json)?,
        principal: opt_nested(o, "principal", principal_from_json)?,
        vega_pillars: vega_pillars_from_json(o)?,
        var_spot_shocks: f64_vec(o, "var_spot_shocks"),
        var_alpha: f64_or_zero(o, "var_alpha"),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn limit_status_response_to_json(r: &LimitStatusResponse) -> Value {
    json!({
        "scope": r.scope.as_ref().map(risk_scope_to_json),
        "limits": Value::Array(r.limits.iter().map(limit_utilization_to_json).collect()),
        "worst": r.worst,
        "hard_breach": r.hard_breach,
        "correlation_id": r.correlation_id,
    })
}

// ---------------------------------------------------------------------------
// frame helpers
// ---------------------------------------------------------------------------

/// Build a `{"type": tag, ...body}` frame: merge the type tag into the body object
/// so the client dispatches on the single `type` discriminator.
pub(super) fn tagged(tag: &str, body: Value) -> Value {
    let mut map = match body {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("value".to_owned(), other);
            m
        }
    };
    map.insert("type".to_owned(), Value::String(tag.to_owned()));
    Value::Object(map)
}

/// Build a typed `error` frame echoing a correlation id when one is present.
pub(super) fn error_frame(message: &str, correlation_id: Option<u64>) -> Value {
    json!({ "type": "error", "message": message, "correlation_id": correlation_id })
}

/// Silence unused-variant lints on the convenience enum aliases imported only to
/// document the vocabulary the codec speaks (they are referenced via their proto
/// `as i32` tags in client payloads, not by name here).
const _: fn() = || {
    let _ = (
        tenor::Unit::Years,
        shock_axis::Factor::Spot,
        StrategyKind::RiskReversal,
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    /// A round-trip of the instrument oneof: a vanilla call decodes from the JSON a
    /// browser client would send, with the strike/delta oneof and convention enums
    /// carried by their canonical proto numbers.
    #[test]
    fn vanilla_instrument_round_trips_from_json() {
        let v = json!({
            "pair": { "base": "EUR", "quote": "USD" },
            "tenor": { "unit": 3, "count": 1 },
            "expiry_years": 1.0,
            "quantity": { "notional": 1_000_000.0, "base_ccy": true },
            "side": 2,
            "vanilla": {
                "option_type": 0,
                "strike": { "strike": 1.12 }
            }
        });
        let instr = instrument_from_json(&v).expect("decode");
        assert_eq!(instr.expiry_years.to_bits(), 1.0_f64.to_bits());
        match instr.product {
            Some(instrument::Product::Vanilla(vn)) => match vn.strike.unwrap().spec {
                Some(strike_or_delta::Spec::Strike(s)) => {
                    assert_eq!(s.to_bits(), 1.12_f64.to_bits());
                }
                other => panic!("expected a strike spec, got {other:?}"),
            },
            other => panic!("expected a vanilla product, got {other:?}"),
        }
    }

    /// MC-honesty on the WS wire: a `Quote` carrying a Monte-Carlo standard error
    /// (set only for MC-priced products like a clamped cliquet) MUST serialize
    /// `price_std_error` onto the JSON the browser/Excel client reads; a closed-form
    /// quote MUST serialize it as JSON null. This gates the wire fix that the WS
    /// quote path no longer silently drops the MC uncertainty.
    #[test]
    fn quote_json_carries_mc_std_error_presence() {
        let mc = Quote {
            quote_id: 7,
            price_std_error: Some(1.7e-4),
            ..Default::default()
        };
        let j = quote_to_json(&mc);
        assert_eq!(j["price_std_error"].as_f64(), Some(1.7e-4));

        let closed = Quote {
            quote_id: 8,
            price_std_error: None,
            ..Default::default()
        };
        let j2 = quote_to_json(&closed);
        assert!(
            j2["price_std_error"].is_null(),
            "a closed-form quote must serialize price_std_error as null"
        );
    }

    /// The Wave-1 products decode from the JSON a browser client sends, into the
    /// correct `product` oneof arms with their fields carried by snake_case name
    /// and enums by their canonical proto numbers.
    #[test]
    fn wave1_products_decode_from_json() {
        let base = |product: Value| {
            let mut m = serde_json::Map::new();
            m.insert("expiry_years".to_owned(), json!(1.0));
            if let Value::Object(p) = product {
                for (k, v) in p {
                    m.insert(k, v);
                }
            }
            Value::Object(m)
        };

        let var_swap =
            instrument_from_json(&base(json!({ "variance_swap": { "strike_vol": 0.11 } })))
                .expect("decode var swap");
        match var_swap.product {
            Some(instrument::Product::VarianceSwap(vs)) => {
                assert_eq!(vs.strike_vol.to_bits(), 0.11_f64.to_bits());
            }
            other => panic!("expected variance_swap, got {other:?}"),
        }

        let vol_swap =
            instrument_from_json(&base(json!({ "volatility_swap": { "strike_vol": 0.0 } })))
                .expect("decode vol swap");
        assert!(matches!(
            vol_swap.product,
            Some(instrument::Product::VolatilitySwap(_))
        ));

        let asian = instrument_from_json(&base(json!({
            "asian_option": {
                "option_type": 0,
                "strike": 1.10,
                "averaging": 0,
                "observations": 12,
                "method": 0,
                "elapsed_avg": 1.095,
                "elapsed_weight": 0.25
            }
        })))
        .expect("decode asian");
        match asian.product {
            Some(instrument::Product::AsianOption(a)) => {
                assert_eq!(a.observations, 12);
                assert_eq!(a.strike.to_bits(), 1.10_f64.to_bits());
                assert_eq!(a.elapsed_weight.to_bits(), 0.25_f64.to_bits());
            }
            other => panic!("expected asian_option, got {other:?}"),
        }
    }

    /// The Wave-2 products decode from a browser client's JSON into the correct
    /// `product` oneof arms — including the presence-tracked optional clamps on
    /// the cliquet (absent ⇒ `None`, present ⇒ `Some`).
    #[test]
    fn wave2_products_decode_from_json() {
        let base = |product: Value| {
            let mut m = serde_json::Map::new();
            m.insert("expiry_years".to_owned(), json!(1.0));
            if let Value::Object(p) = product {
                for (k, v) in p {
                    m.insert(k, v);
                }
            }
            Value::Object(m)
        };

        let fs = instrument_from_json(&base(json!({
            "forward_start": { "option_type": 0, "moneyness": 1.0, "reset": 0.25 }
        })))
        .expect("decode forward_start");
        match fs.product {
            Some(instrument::Product::ForwardStart(f)) => {
                assert_eq!(f.moneyness.to_bits(), 1.0_f64.to_bits());
                assert_eq!(f.reset.to_bits(), 0.25_f64.to_bits());
            }
            other => panic!("expected forward_start, got {other:?}"),
        }

        // Plain ratchet: no clamp keys ⇒ every optional bound is None.
        let plain = instrument_from_json(&base(json!({
            "cliquet": { "option_type": 0, "moneyness": 1.0, "periods": 4 }
        })))
        .expect("decode plain cliquet");
        match plain.product {
            Some(instrument::Product::Cliquet(c)) => {
                assert_eq!(c.periods, 4);
                assert!(c.local_floor.is_none() && c.local_cap.is_none());
                assert!(c.global_floor.is_none() && c.global_cap.is_none());
            }
            other => panic!("expected cliquet, got {other:?}"),
        }

        // Clamped cliquet: presence-tracked floor/cap + MC knobs round-trip.
        let clamped = instrument_from_json(&base(json!({
            "cliquet": {
                "option_type": 0, "moneyness": 1.0, "periods": 6,
                "local_floor": 0.0, "local_cap": 0.03,
                "mc_pairs": 50000, "mc_seed": 12345
            }
        })))
        .expect("decode clamped cliquet");
        match clamped.product {
            Some(instrument::Product::Cliquet(c)) => {
                assert_eq!(c.local_floor, Some(0.0));
                assert_eq!(c.local_cap, Some(0.03));
                assert_eq!(c.mc_pairs, 50_000);
                assert_eq!(c.mc_seed, 12_345);
            }
            other => panic!("expected cliquet, got {other:?}"),
        }

        let quanto = instrument_from_json(&base(json!({
            "quanto": {
                "payoff": 1, "option_type": 1, "strike": 1.12,
                "conversion_vol": 0.09, "correlation": -0.3
            }
        })))
        .expect("decode quanto");
        match quanto.product {
            Some(instrument::Product::Quanto(q)) => {
                assert_eq!(q.payoff, celnet_proto::QuantoPayoff::Digital as i32);
                assert_eq!(q.strike.to_bits(), 1.12_f64.to_bits());
                assert_eq!(q.correlation.to_bits(), (-0.3_f64).to_bits());
            }
            other => panic!("expected quanto, got {other:?}"),
        }
    }

    /// The Wave-3 products (TARF / accumulator / lookback) decode from a browser
    /// client's JSON into the correct `product` oneof arms — including the nested
    /// `FixingSchedule` body on the TARF/accumulator and the MC knobs.
    #[test]
    fn wave3_products_decode_from_json() {
        let base = |product: Value| {
            let mut m = serde_json::Map::new();
            m.insert("expiry_years".to_owned(), json!(1.0));
            if let Value::Object(p) = product {
                for (k, v) in p {
                    m.insert(k, v);
                }
            }
            Value::Object(m)
        };

        let tarf = instrument_from_json(&base(json!({
            "tarf": {
                "option_type": 1, "strike": 1.10, "target": 0.30, "leverage": 2.0,
                "redemption": 1,
                "schedule": { "fixing_years": [0.25, 0.5, 0.75, 1.0], "fixing_notional": 1.0 },
                "mc_pairs": 50000, "mc_seed": 999
            }
        })))
        .expect("decode tarf");
        match tarf.product {
            Some(instrument::Product::Tarf(t)) => {
                assert_eq!(
                    t.redemption,
                    celnet_proto::TarfRedemption::CappedGain as i32
                );
                assert_eq!(t.target.to_bits(), 0.30_f64.to_bits());
                let s = t.schedule.expect("tarf carries a schedule");
                assert_eq!(s.fixing_years.len(), 4);
                assert_eq!(s.fixing_notional.to_bits(), 1.0_f64.to_bits());
                assert_eq!(t.mc_pairs, 50_000);
                assert_eq!(t.mc_seed, 999);
            }
            other => panic!("expected tarf, got {other:?}"),
        }

        let acc = instrument_from_json(&base(json!({
            "accumulator": {
                "pivot": 1.10, "barrier": 1.16, "leverage": 2.0, "monitoring": 1,
                "schedule": { "fixing_years": [0.5, 1.0], "fixing_notional": 1.0 },
                "mc_pairs": 40000, "mc_seed": 7
            }
        })))
        .expect("decode accumulator");
        match acc.product {
            Some(instrument::Product::Accumulator(a)) => {
                assert_eq!(
                    a.monitoring,
                    celnet_proto::AccumulatorMonitoring::Continuous as i32
                );
                assert_eq!(a.barrier.to_bits(), 1.16_f64.to_bits());
                assert_eq!(a.schedule.expect("schedule").fixing_years.len(), 2);
            }
            other => panic!("expected accumulator, got {other:?}"),
        }

        // Continuous lookback: no MC knobs needed.
        let lb = instrument_from_json(&base(json!({
            "lookback": {
                "style": 0, "option_type": 0, "monitoring": 0
            }
        })))
        .expect("decode lookback");
        match lb.product {
            Some(instrument::Product::Lookback(l)) => {
                assert_eq!(l.style, celnet_proto::LookbackStyle::Floating as i32);
                assert_eq!(
                    l.monitoring,
                    celnet_proto::LookbackMonitoring::Continuous as i32
                );
            }
            other => panic!("expected lookback, got {other:?}"),
        }

        // Discrete lookback: MC knobs round-trip.
        let lbd = instrument_from_json(&base(json!({
            "lookback": {
                "style": 1, "option_type": 0, "monitoring": 1,
                "strike": 1.05, "observations": 32, "mc_pairs": 20000, "mc_seed": 11
            }
        })))
        .expect("decode discrete lookback");
        match lbd.product {
            Some(instrument::Product::Lookback(l)) => {
                assert_eq!(l.style, celnet_proto::LookbackStyle::Fixed as i32);
                assert_eq!(
                    l.monitoring,
                    celnet_proto::LookbackMonitoring::Discrete as i32
                );
                assert_eq!(l.strike.to_bits(), 1.05_f64.to_bits());
                assert_eq!(l.observations, 32);
                assert_eq!(l.mc_pairs, 20_000);
                assert_eq!(l.mc_seed, 11);
            }
            other => panic!("expected lookback, got {other:?}"),
        }
    }

    /// The window-barrier product and the booking-model selector decode from the
    /// browser JSON path (the GUI/Excel transport). The exact wire shape: a
    /// `window_barrier` product key carrying a nested `vanilla`, `barrier`,
    /// `side`, `window_start`, `window_end`, optional `mc_pairs`/`mc_steps`/
    /// `mc_seed`; and a top-level `pricing_model` integer on the instrument
    /// (0 = DEFAULT, 1 = LOCAL_STOCH_VOL).
    #[test]
    fn window_barrier_and_pricing_model_decode_from_json() {
        let instr = instrument_from_json(&json!({
            "expiry_years": 1.0,
            "pricing_model": 1,
            "window_barrier": {
                "vanilla": { "option_type": 0, "strike": { "strike": 1.10 } },
                "barrier": 1.30,
                "side": 0,
                "window_start": 0.25,
                "window_end": 0.75,
                "mc_pairs": 8000,
                "mc_steps": 64,
                "mc_seed": 42
            }
        }))
        .expect("decode window barrier");
        assert_eq!(
            instr.pricing_model,
            celnet_proto::PricingModel::LocalStochVol as i32,
            "the top-level pricing_model selector decodes"
        );
        match instr.product {
            Some(instrument::Product::WindowBarrier(w)) => {
                let v = w.vanilla.expect("window barrier carries a vanilla");
                assert_eq!(v.option_type, celnet_proto::OptionType::Call as i32);
                assert_eq!(w.barrier.to_bits(), 1.30_f64.to_bits());
                assert_eq!(w.side, celnet_proto::BarrierSide::Up as i32);
                assert_eq!(w.window_start.to_bits(), 0.25_f64.to_bits());
                assert_eq!(w.window_end.to_bits(), 0.75_f64.to_bits());
                assert_eq!(w.mc_pairs, 8000);
                assert_eq!(w.mc_steps, 64);
                assert_eq!(w.mc_seed, 42);
            }
            other => panic!("expected window_barrier, got {other:?}"),
        }

        // An absent pricing_model decodes to DEFAULT (proto3 zero) — existing
        // browser requests are unchanged.
        let plain = instrument_from_json(&json!({
            "expiry_years": 1.0,
            "vanilla": { "option_type": 0, "strike": { "strike": 1.10 } }
        }))
        .expect("decode plain vanilla");
        assert_eq!(
            plain.pricing_model,
            celnet_proto::PricingModel::Default as i32
        );
    }

    /// The American / Bermudan early-exercise product decodes from the browser
    /// JSON path (GUI/Excel transport). The exact wire shape: an `american`
    /// product key carrying `option_type`, `strike`, `exercise_style`, an optional
    /// `bermudan_dates` array, and the LSM knobs `lsm_paths`/`lsm_exercise_dates`/
    /// `lsm_seed`.
    #[test]
    fn american_decodes_from_json() {
        // American (continuous), FD engine (lsm_paths absent ⇒ 0).
        let american = instrument_from_json(&json!({
            "expiry_years": 1.0,
            "american": {
                "option_type": 1,
                "strike": 1.10,
                "exercise_style": 0
            }
        }))
        .expect("decode american");
        match american.product {
            Some(instrument::Product::American(a)) => {
                assert_eq!(a.option_type, celnet_proto::OptionType::Put as i32);
                assert_eq!(a.strike.to_bits(), 1.10_f64.to_bits());
                assert_eq!(
                    a.exercise_style,
                    celnet_proto::ExerciseStyle::American as i32
                );
                assert!(a.bermudan_dates.is_empty());
                assert_eq!(a.lsm_paths, 0);
            }
            other => panic!("expected american, got {other:?}"),
        }

        // Bermudan with an explicit date set + LSM engine knobs.
        let bermudan = instrument_from_json(&json!({
            "expiry_years": 1.0,
            "american": {
                "option_type": 0,
                "strike": 1.05,
                "exercise_style": 1,
                "bermudan_dates": [0.25, 0.5, 0.75, 1.0],
                "lsm_paths": 100000,
                "lsm_exercise_dates": 50,
                "lsm_seed": 7
            }
        }))
        .expect("decode bermudan");
        match bermudan.product {
            Some(instrument::Product::American(a)) => {
                assert_eq!(
                    a.exercise_style,
                    celnet_proto::ExerciseStyle::Bermudan as i32
                );
                assert_eq!(a.bermudan_dates.len(), 4);
                assert_eq!(a.bermudan_dates[0].to_bits(), 0.25_f64.to_bits());
                assert_eq!(a.lsm_paths, 100_000);
                assert_eq!(a.lsm_exercise_dates, 50);
                assert_eq!(a.lsm_seed, 7);
            }
            other => panic!("expected american, got {other:?}"),
        }
    }

    /// A delta-specified strike decodes to the `delta` oneof arm.
    #[test]
    fn delta_strike_decodes_to_delta_arm() {
        let v = json!({ "delta": 0.25 });
        let sd = strike_or_delta_from_json(&v).expect("decode");
        assert!(matches!(sd.spec, Some(strike_or_delta::Spec::Delta(_))));
    }

    /// A presence-tracked optional u64 is `None` when absent or null, `Some` when set.
    #[test]
    fn optional_u64_presence() {
        let mut o = Map::new();
        assert_eq!(opt_u64(&o, "surface_version"), None);
        o.insert("surface_version".to_owned(), Value::Null);
        assert_eq!(opt_u64(&o, "surface_version"), None);
        o.insert("surface_version".to_owned(), json!(42));
        assert_eq!(opt_u64(&o, "surface_version"), Some(42));
    }

    /// The `tagged` helper merges the type discriminator into the body object.
    #[test]
    fn tagged_merges_type_into_body() {
        let f = tagged("quote", json!({ "quote_id": 7 }));
        assert_eq!(f["type"], json!("quote"));
        assert_eq!(f["quote_id"], json!(7));
    }

    /// An `aggregate_risk` WS frame decodes field-for-field by proto snake_case name,
    /// enums by number, and the optional principal/scope/correlation_id present.
    #[test]
    fn aggregate_risk_request_decodes_from_ws_json() {
        let frame = json!({
            "type": "aggregate_risk",
            "dimension": 3, // RiskDimension::DESK
            "numeraire": { "numeraire": "USD", "rates": [{ "ccy": "EUR", "rate": 1.10 }] },
            "principal": {
                "grant_all": false,
                "grants": [{ "scopes": [{ "dimension": 3, "value": 99 }] }],
                "denies": []
            },
            "scope": { "dimension": 3, "value": 99 },
            "vega_pillars": [{ "tenor_days": 365, "delta_bp": 2500 }],
            "var_spot_shocks": [-0.01, 0.0, 0.01],
            "var_alpha": 0.99,
            "curvature_risk_weight": 0.2,
            "correlation_id": 7
        });
        let o = frame.as_object().unwrap();
        let req = aggregate_risk_request_from_json(o).unwrap();
        assert_eq!(req.dimension, 3);
        assert_eq!(req.numeraire.as_ref().unwrap().numeraire, "USD");
        assert_eq!(req.numeraire.as_ref().unwrap().rates[0].rate, 1.10);
        let p = req.principal.as_ref().unwrap();
        assert!(!p.grant_all);
        assert_eq!(p.grants[0].scopes[0].value, 99);
        assert_eq!(req.scope.as_ref().unwrap().value, 99);
        assert_eq!(req.vega_pillars[0].tenor_days, 365);
        assert_eq!(req.vega_pillars[0].delta_bp, 2500);
        assert_eq!(req.var_spot_shocks, vec![-0.01, 0.0, 0.01]);
        assert_eq!(req.var_alpha, 0.99);
        assert_eq!(req.curvature_risk_weight, 0.2);
        assert_eq!(req.correlation_id, Some(7));
    }

    /// An `aggregate_risk_response` encodes the node tree by proto field name, enums
    /// by number, and the presence-tracked nonadditive measures (absent ⇒ `null`).
    #[test]
    fn aggregate_risk_response_encodes_to_ws_json() {
        let resp = AggregateRiskResponse {
            dimension: 0, // FIRM
            numeraire: "USD".to_owned(),
            nodes: vec![RiskNode {
                dimension: 0,
                group: 0,
                additive: Some(AdditiveRisk {
                    delta_numeraire: 1.5,
                    delta_vector: vec![CcyExposureLeg {
                        ccy: "EUR".to_owned(),
                        amount: 10.0,
                    }],
                    gamma: 2.0,
                    vega_numeraire: 3.0,
                    theta: 0.0,
                    vanna: 0.0,
                    volga: 0.0,
                    charm: 0.0,
                    speed: 0.0,
                    zomma: 0.0,
                    color: 0.0,
                    premium_numeraire: 4.0,
                    vega_ladder: vec![VegaLadderBucket {
                        pillar: Some(VegaPillar {
                            tenor_days: 365,
                            delta_bp: 5000,
                        }),
                        vega: 3.0,
                    }],
                }),
                nonadditive: Some(NonAdditiveRisk {
                    var: Some(5.0),
                    es: Some(6.0),
                    var_alpha: Some(0.99),
                    curvature_spot: None, // not evaluated ⇒ null, never spurious 0
                }),
                position_count: 2,
            }],
            correlation_id: Some(7),
        };
        let v = aggregate_risk_response_to_json(&resp);
        assert_eq!(v["dimension"], json!(0));
        assert_eq!(v["numeraire"], json!("USD"));
        let node = &v["nodes"][0];
        assert_eq!(node["position_count"], json!(2));
        assert_eq!(node["additive"]["delta_numeraire"], json!(1.5));
        assert_eq!(node["additive"]["delta_vector"][0]["ccy"], json!("EUR"));
        assert_eq!(
            node["additive"]["vega_ladder"][0]["pillar"]["tenor_days"],
            json!(365)
        );
        assert_eq!(node["nonadditive"]["var"], json!(5.0));
        // An unevaluated curvature is `null`, not a spurious zero.
        assert_eq!(node["nonadditive"]["curvature_spot"], Value::Null);
        assert_eq!(v["correlation_id"], json!(7));
    }
}
