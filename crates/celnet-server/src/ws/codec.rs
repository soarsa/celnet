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
    ArbReport, BrokerQuoteSet, BucketedRisk, CcyPair, Conventions, CrossGamma, Digital,
    DoubleBarrier, Execute, Executed, Execution, GetSmileRequest, Greeks, Instrument, Leg,
    MarkSurfaceRequest, MarkSurfaceResponse, MarketContext, Modify, PriceRequest, PriceResponse,
    Quantity, Quote, QuoteAccept, QuoteReject, QuoteRequest, RejectAck, Resync, RiskBucketRequest,
    ScenarioPoint, ScenarioRequest, ScenarioResponse, ShockAxis, SingleBarrier, Smile, SmilePoint,
    Snapshot, Solve, Strategy, StrategyKind, StreamEnd, StreamReject, StrikeOrDelta, Subscribe,
    SubscriptionId, Tenor, Touch, TradableToken, TwoWayPrice, Unsubscribe, Update, Vanilla,
    instrument, shock_axis, strike_or_delta, tenor,
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

/// Decode the instrument `product` oneof. The JSON carries exactly one of the
/// product keys (`vanilla`, `strategy`, `single_barrier`, `double_barrier`,
/// `digital`, `touch`) — the same shape as the proto oneof.
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
    } else {
        Err(err(
            "instrument needs exactly one product (vanilla / strategy / \
             single_barrier / double_barrier / digital / touch)",
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
    })
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
    })
}

pub(super) fn execution_to_json(e: &Execution) -> Value {
    json!({
        "execution_id": e.execution_id,
        "quote_id": e.quote_id,
        "side": e.side,
        "traded_premium": e.traded_premium,
        "epoch_nanos": e.epoch_nanos,
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
            }),
        ),
        Message::StreamEnd(e) => ("stream_end", stream_end_to_json(e)),
        Message::Executed(e) => ("executed", executed_to_json(e)),
        Message::StreamReject(r) => ("stream_reject", stream_reject_to_json(r)),
    };
    Some(tagged(tag, body))
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
    })
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
    })
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
}
