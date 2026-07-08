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
    ArbReport, AsianOption, BasketLeg, BasketOption, BrokerQuoteSet, BucketedRisk, CcyExposureLeg,
    CcyPair, Cliquet, Conventions, CrossGamma, DealerQuote, Digital, DoubleBarrier,
    DrillRiskRequest, DrillRiskResponse, EntitlementPrincipal, EntitlementRule, Execute, Executed,
    Execution, FixingSchedule, ForwardStart, FxForward, FxSwap, GetSmileRequest, Greeks,
    Instrument, Leg, LimitStatusRequest, LimitStatusResponse, LimitUtilization,
    ListPositionsRequest, ListPositionsResponse, ListedFutureOption, Lookback, MarkSurfaceRequest,
    MarkSurfaceResponse, MarketContext, Modify, MultiDealerQuote, Ndf, NonAdditiveRisk,
    NumeraireRate, OrgKey, PerpetualOption, Pivot, PriceRequest, PriceResponse, Quantity, Quanto,
    Quote, QuoteAccept, QuoteReject, QuoteRequest, RejectAck, ReportingNumeraire, Resync,
    RiskBucketRequest, RiskNode, RiskPosition, RiskScope, ScenarioPoint, ScenarioRequest,
    ScenarioResponse, ShockAxis, SingleBarrier, Smile, SmilePoint, Snapshot, Solve, Strategy,
    StrategyKind, StreamAuth, StreamEnd, StreamReject, StrikeOrDelta, Subscribe, SubscriptionId,
    Tarf, Tenor, Touch, TradableToken, TwoWayPrice, Unsubscribe, Update, Vanilla, VanillaInputs,
    VarianceSwap, VegaLadderBucket, VegaPillar, VolatilitySwap, WindowBarrier, instrument,
    shock_axis, strike_or_delta, tenor,
};
// The FIX-admin contract (manage the inbound FIX acceptor connections). Kept in a
// dedicated `use` so the long alphabetized list above stays undisturbed.
use celnet_proto::{
    CreateFixConnectionRequest, CreateFixConnectionResponse, DeleteFixConnectionRequest,
    DeleteFixConnectionResponse, FixConnectionDesc, FixConnectionSpec, FixMessage,
    ListFixConnectionsRequest, ListFixConnectionsResponse, ListFixMessagesRequest,
    ListFixMessagesResponse, SetFixConnectionEnabledRequest, SetFixConnectionEnabledResponse,
    UpdateFixConnectionRequest, UpdateFixConnectionResponse,
};
// AuthService — server-enforced sessions + user/desk/entity/book administration (WS mirror).
use celnet_proto::{
    BookDesc, CapabilityDesc, CreateBookRequest, CreateBookResponse, CreateDeskRequest,
    CreateDeskResponse, CreateEntityRequest, CreateEntityResponse, CreateUserRequest,
    CreateUserResponse, DeleteBookRequest, DeleteBookResponse, DeleteDeskRequest,
    DeleteDeskResponse, DeleteEntityRequest, DeleteEntityResponse, DeleteUserRequest,
    DeleteUserResponse, DeskDesc, EntityDesc, GetRoleCapabilitiesRequest,
    GetRoleCapabilitiesResponse, GetUserCapabilitiesRequest, GetUserCapabilitiesResponse,
    ListBooksRequest, ListBooksResponse, ListDesksRequest, ListDesksResponse, ListEntitiesRequest,
    ListEntitiesResponse, ListUsersRequest, ListUsersResponse, LoginRequest, LoginResponse,
    LogoutRequest, LogoutResponse, ResetPasswordRequest, ResetPasswordResponse,
    SetRoleCapabilitiesRequest, SetRoleCapabilitiesResponse, SetUserCapabilitiesRequest,
    SetUserCapabilitiesResponse, UpdateBookRequest, UpdateBookResponse, UpdateDeskRequest,
    UpdateDeskResponse, UpdateEntityRequest, UpdateEntityResponse, UpdateUserRequest,
    UpdateUserResponse, UserDesc,
};
// AuthService — instrument reference-data registry (WS mirror of the instrument RPCs).
use celnet_proto::{
    CreateInstrumentRequest, CreateInstrumentResponse, DeleteInstrumentRequest,
    DeleteInstrumentResponse, GetInstrumentRequest, GetInstrumentResponse, InstrumentDefDesc,
    ListInstrumentsRequest, ListInstrumentsResponse, UpdateInstrumentRequest,
    UpdateInstrumentResponse, instrument_def_desc::Definition as InstrumentDefinition,
};
// AuthService — discount-curve bootstrap from registry-referenced instruments
// (WS mirror of the `BuildCurve` RPC: resolve each pillar id against the
// reference-data registry, bootstrap, return per-instrument calibrated points).
use celnet_proto::{
    BuildCurveRequest, CalibratedCurve, CalibratedCurvePoint, DatePillar, InstrumentQuote,
};
// Linear-rates (fixed-income) contract — the WS mirror of PricingService::PriceRates.
use celnet_proto::{
    BondInstrument, CurveSet, FraInstrument, OisInstrument, OisPillar, RatesInstrument,
    RatesPriceRequest, RatesPriceResponse, RatesPricingResult, VanillaIrsInstrument,
    rates_instrument,
};
// XVA (valuation-adjustment) contract — the WS mirror of PricingService::PriceXva.
use celnet_proto::{
    PriceXvaRequest, PriceXvaResponse, XvaResult as WireXvaResult, XvaSurvivalCurve, XvaTrade,
};
// Linear-rates portfolio risk — the WS mirror of RiskService::AggregateRatesRisk.
use celnet_proto::{
    AcceptDeskQuoteRequest, BookRatesPositionRequest, BookRatesPositionResponse, Deal, DeskQuote,
    DeskReject, DeskRequest, DeskRequestScope, ListDealsRequest, ListDealsResponse,
    ListDeskRequestsRequest, ListDeskRequestsResponse, ListRatesPositionsRequest,
    ListRatesPositionsResponse, Notification, RespondDeskRequestRequest, SubmitDeskRequestRequest,
    respond_desk_request_request::Response as RespondArm,
};
use celnet_proto::{
    AggregateRatesRiskRequest, AggregateRatesRiskResponse, KeyRateDv01, RatesPosition,
    RatesRiskNode, RatesRiskScope,
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

/// An optional presence-tracked `u32` field (`null`/absent/out-of-range ⇒ `None`).
fn opt_u32(o: &Map<String, Value>, key: &str) -> Option<u32> {
    match o.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()),
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

/// An optional `String` field (`null`/absent/empty ⇒ `None`).
fn opt_string(o: &Map<String, Value>, key: &str) -> Option<String> {
    o.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
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

/// Decode a JSON `{base, quote}` pair object into an FX [`Underlying`]. The WS
/// JSON mirror keeps the `pair` key (the GUI/Excel contract is unchanged); the
/// generalized wire `underlying` is its FX projection.
fn underlying_from_json(v: &Value) -> Result<celnet_proto::Underlying> {
    Ok(celnet_proto::Underlying::fx(ccy_pair_from_json(v)?))
}

/// Encode an FX [`Underlying`] back to the JSON `{base, quote}` pair object (the
/// FX arm only; a non-FX underlying never reaches this FX-only WS surface).
fn underlying_to_json(u: &celnet_proto::Underlying) -> Value {
    u.as_fx().map_or_else(|| json!(null), ccy_pair_to_json)
}

/// Decode an instrument's underlying for the WS mirror, supporting every wire
/// arm — the cross-asset arms carried on a richer `underlying` object
/// (`{"fx": {base,quote}}`, `{"metal": {metal, quote}}`,
/// `{"equity": {symbol:{ticker,venue}, currency}}`, `{"commodity": {...}}`,
/// `{"digital_asset": {base, quote}}`) plus the legacy FX `pair` key.
///
/// **Precedence — `underlying` is authoritative.** The production GUI/Excel
/// encoders always emit the legacy FX `pair` projection BESIDE the richer
/// `underlying` oneof so the FX-keyed surfaces stay total (`instrumentToWire` in
/// both clients). The `pair` is therefore the legacy FX *projection* of the same
/// instrument, while `underlying` is the asset-class-discriminated, richer form:
/// when both are present the `underlying` oneof wins and the `pair` is ignored.
/// This is the only rule that routes a legitimate cross-asset frame (e.g. an
/// INVERSE_COIN digital-asset frame, which carries `digital_asset` +
/// `settlement_style` + the `{base,quote}` leg-string `pair`) to its correct arm
/// rather than mis-decoding it as FX and pricing the LINEAR (USD) value. A
/// pure-FX legacy frame carries `pair` only (no `underlying`), so it still
/// decodes to the FX arm byte-identically. A frame carrying neither yields `None`
/// (the same as the prior `opt_nested("pair")`).
fn instrument_underlying_from_json(
    o: &Map<String, Value>,
) -> Result<Option<celnet_proto::Underlying>> {
    // The richer cross-asset `underlying` oneof is authoritative when present — it
    // carries the asset-class discriminator a legacy `pair` projection cannot.
    if let Some(v) = o.get("underlying").filter(|v| !v.is_null()) {
        return underlying_object_from_json(v).map(Some);
    }
    // The legacy FX `pair` key is the FX projection, consulted only when no richer
    // `underlying` is present (the unchanged pure-FX GUI/Excel contract).
    match o.get("pair") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(celnet_proto::Underlying::fx(ccy_pair_from_json(v)?))),
    }
}

/// Decode the richer `underlying` object (the wire `Underlying` oneof) — exactly
/// one arm key set, mirroring the proto. The cross-asset arms lower onto the
/// generated `celnet_proto::Underlying` constructors.
fn underlying_object_from_json(v: &Value) -> Result<celnet_proto::Underlying> {
    let o = obj(v, "underlying")?;
    if let Some(fx) = o.get("fx") {
        Ok(celnet_proto::Underlying::fx(ccy_pair_from_json(fx)?))
    } else if let Some(m) = o.get("metal") {
        Ok(celnet_proto::Underlying::metal(metal_pair_from_json(m)?))
    } else if let Some(e) = o.get("equity") {
        Ok(celnet_proto::Underlying::equity(equity_ref_from_json(e)?))
    } else if let Some(c) = o.get("commodity") {
        Ok(celnet_proto::Underlying::commodity(
            commodity_ref_from_json(c)?,
        ))
    } else if let Some(d) = o.get("digital_asset") {
        Ok(celnet_proto::Underlying::digital_asset(
            crypto_pair_from_json(d)?,
        ))
    } else {
        Err(err(
            "underlying needs exactly one arm (fx / metal / equity / commodity / \
             digital_asset)",
        ))
    }
}

fn metal_pair_from_json(v: &Value) -> Result<celnet_proto::MetalPair> {
    let o = obj(v, "metal")?;
    Ok(celnet_proto::MetalPair {
        metal: enum_or_zero(o, "metal"),
        quote: string_field(o, "quote")?,
    })
}

fn symbol_from_json(v: &Value) -> Result<celnet_proto::Symbol> {
    let o = obj(v, "symbol")?;
    Ok(celnet_proto::Symbol {
        ticker: string_field(o, "ticker")?,
        venue: string_or_empty(o, "venue"),
    })
}

fn equity_ref_from_json(v: &Value) -> Result<celnet_proto::EquityRef> {
    let o = obj(v, "equity")?;
    Ok(celnet_proto::EquityRef {
        symbol: Some(nested(o, "symbol", symbol_from_json)?),
        currency: string_field(o, "currency")?,
    })
}

fn commodity_ref_from_json(v: &Value) -> Result<celnet_proto::CommodityRef> {
    let o = obj(v, "commodity")?;
    Ok(celnet_proto::CommodityRef {
        symbol: Some(nested(o, "symbol", symbol_from_json)?),
        currency: string_field(o, "currency")?,
    })
}

fn crypto_pair_from_json(v: &Value) -> Result<celnet_proto::CryptoPair> {
    let o = obj(v, "digital_asset")?;
    Ok(celnet_proto::CryptoPair {
        base: string_field(o, "base")?,
        quote: string_field(o, "quote")?,
    })
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
    // The WS JSON mirror keeps the FX `r_dom`/`r_for` keys (the GUI/Excel contract
    // is unchanged); the FX projection of the generalized `{discount_rate, carry}`
    // wire form is built via the byte-identical `fx` constructor.
    Ok(MarketContext::fx(
        f64_field(o, "spot")?,
        f64_field(o, "vol")?,
        f64_or_zero(o, "r_dom"),
        f64_or_zero(o, "r_for"),
    ))
}

fn market_context_to_json(m: &MarketContext) -> Value {
    json!({ "spot": m.spot, "vol": m.vol, "r_dom": m.r_dom(), "r_for": m.r_for() })
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
        // The flat two-rho projection (FX verbatim; cost-of-carry losslessly
        // projected) — kept for clients that read the flat shape directly.
        "rho_dom": g.rho_dom(),
        "rho_for": g.rho_for(),
        // The carry-tagged arm itself, so a client renders the asset-class-correct
        // rate Greeks (FX: rho_dom/rho_for; cross-asset: discount_rho/carry_rho).
        // `null` only for an absent strip; FX/metal lines carry the `fx` arm.
        "rate_sensitivities": g.rate_sensitivities.as_ref().map(rate_sensitivities_to_json),
        "vanna": g.vanna,
        "volga": g.volga,
        "charm": g.charm,
        "speed": g.speed,
        "zomma": g.zomma,
        "color": g.color,
    })
}

/// Serialize the carry-tagged rate-sensitivity arm onto the wire JSON, mirroring
/// the proto `RateSensitivities` oneof: `{"fx":{rho_dom,rho_for}}` for FX/metal,
/// `{"carry":{discount_rho,carry_rho}}` for a cross-asset cost-of-carry line. The
/// streamed edge of the carry seam reaching the JSON clients.
fn rate_sensitivities_to_json(rs: &celnet_proto::RateSensitivities) -> Value {
    use celnet_proto::rate_sensitivities::Sensitivities;
    match &rs.sensitivities {
        Some(Sensitivities::Fx(fx)) => json!({
            "fx": { "rho_dom": fx.rho_dom, "rho_for": fx.rho_for },
        }),
        Some(Sensitivities::Carry(c)) => json!({
            "carry": { "discount_rho": c.discount_rho, "carry_rho": c.carry_rho },
        }),
        None => Value::Null,
    }
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

/// Decode a pivot Target-Redemption Accumulator (`pivot`) — the TARF sibling
/// with the distinct pivot kink. Mirrors `tarf_from_json` field-for-field plus
/// the `pivot` level (`pivot == strike` is the exact TARF slice).
fn pivot_from_json(v: &Value) -> Result<Pivot> {
    let o = obj(v, "pivot")?;
    let mc_pairs = u32::try_from(u64_or_zero(o, "mc_pairs"))
        .map_err(|_| err("pivot.mc_pairs out of range"))?;
    let schedule = o
        .get("schedule")
        .map(fixing_schedule_from_json)
        .transpose()?;
    Ok(Pivot {
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        pivot: f64_field(o, "pivot")?,
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

fn basket_leg_from_json(v: &Value) -> Result<BasketLeg> {
    let o = obj(v, "basket leg")?;
    Ok(BasketLeg {
        underlying: opt_nested(o, "pair", underlying_from_json)?,
        weight: f64_field(o, "weight")?,
        spot: f64_field(o, "spot")?,
        vol: f64_field(o, "vol")?,
        r_for: f64_or_zero(o, "r_for"),
    })
}

fn basket_from_json(v: &Value) -> Result<BasketOption> {
    let o = obj(v, "basket")?;
    // The `legs` array: one BasketLeg object per underlying.
    let legs = o
        .get("legs")
        .and_then(Value::as_array)
        .ok_or_else(|| err("basket.legs must be an array of leg objects"))?
        .iter()
        .map(basket_leg_from_json)
        .collect::<Result<Vec<_>>>()?;
    // The `correlations` array: the row-major N×N correlation matrix (length N²).
    let correlations = f64_vec(o, "correlations");
    let mc_paths = u32::try_from(u64_or_zero(o, "mc_paths"))
        .map_err(|_| err("basket.mc_paths out of range"))?;
    let mc_replications = u32::try_from(u64_or_zero(o, "mc_replications"))
        .map_err(|_| err("basket.mc_replications out of range"))?;
    let mc_steps = u32::try_from(u64_or_zero(o, "mc_steps"))
        .map_err(|_| err("basket.mc_steps out of range"))?;
    Ok(BasketOption {
        legs,
        correlations,
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        kind: enum_or_zero(o, "kind"),
        mc_paths,
        mc_replications,
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

/// Decode an FX outright forward (`fx_forward`) — the W2 linear-book product.
/// `side` is the proto `Side` enum tag (BUY = 0). Closed-form, exact ⇒ no MC.
fn fx_forward_from_json(v: &Value) -> Result<FxForward> {
    let o = obj(v, "fx_forward")?;
    Ok(FxForward {
        contract_rate: f64_field(o, "contract_rate")?,
        notional: f64_field(o, "notional")?,
        side: enum_or_zero(o, "side"),
    })
}

/// Decode an FX swap (`fx_swap`) — a near leg + a far leg, each an `fx_forward`.
fn fx_swap_from_json(v: &Value) -> Result<FxSwap> {
    let o = obj(v, "fx_swap")?;
    Ok(FxSwap {
        near: Some(nested(o, "near", fx_forward_from_json)?),
        far: Some(nested(o, "far", fx_forward_from_json)?),
    })
}

/// Decode a non-deliverable forward (`ndf`) — cash-settled at a named fixing.
/// `fixing` is the proto `FixingSource` enum tag; `settlement_ccy` names the
/// convertible settlement currency. The fixing identity is metadata only.
fn ndf_from_json(v: &Value) -> Result<Ndf> {
    let o = obj(v, "ndf")?;
    Ok(Ndf {
        contract_rate: f64_field(o, "contract_rate")?,
        notional: f64_field(o, "notional")?,
        side: enum_or_zero(o, "side"),
        fixing: enum_or_zero(o, "fixing"),
        settlement_ccy: string_or_empty(o, "settlement_ccy"),
    })
}

/// Decode a perpetual (no-expiry) American option (`perpetual_option`). The
/// enclosing instrument's `expiry_years` MUST be 0 for this arm (a perpetual
/// has no expiry to encode) — enforced at the shared pricing/validity seam,
/// never silently ignored. `notional` is the booked trade size (proto3 scalar,
/// absent ⇒ 0), not a pricing input — the premium is per 1 unit of base.
fn perpetual_option_from_json(v: &Value) -> Result<PerpetualOption> {
    let o = obj(v, "perpetual_option")?;
    Ok(PerpetualOption {
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        notional: f64_or_zero(o, "notional"),
    })
}

/// Decode an option on a listed future (`listed_future_option`): the future's
/// contract identity (`future_symbol`: ticker + venue MIC), the future's own
/// expiry (which must outlive the option's — validity-checked at the shared
/// pricing seam) and the premium `margining` tag (the proto `Margining` enum
/// number; 0 = equity-style upfront, 1 = futures-style daily-margined).
fn listed_future_option_from_json(v: &Value) -> Result<ListedFutureOption> {
    let o = obj(v, "listed_future_option")?;
    Ok(ListedFutureOption {
        future_symbol: Some(nested(o, "future_symbol", symbol_from_json)?),
        future_expiry_years: f64_field(o, "future_expiry_years")?,
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        notional: f64_or_zero(o, "notional"),
        margining: enum_or_zero(o, "margining"),
    })
}

/// Decode the instrument `product` oneof. The JSON carries exactly one of the
/// product keys (`vanilla`, `strategy`, `single_barrier`, `double_barrier`,
/// `digital`, `touch`, `variance_swap`, `volatility_swap`, `asian_option`,
/// `forward_start`, `cliquet`, `quanto`, `tarf`, `pivot`, `accumulator`,
/// `lookback`, `window_barrier`, `american`, `basket`, `fx_forward`, `fx_swap`,
/// `ndf`, `perpetual_option`, `listed_future_option`) — the same shape as the
/// proto oneof.
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
    } else if let Some(v) = o.get("pivot") {
        Ok(instrument::Product::Pivot(pivot_from_json(v)?))
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
    } else if let Some(v) = o.get("basket") {
        Ok(instrument::Product::Basket(basket_from_json(v)?))
    } else if let Some(v) = o.get("fx_forward") {
        Ok(instrument::Product::FxForward(fx_forward_from_json(v)?))
    } else if let Some(v) = o.get("fx_swap") {
        Ok(instrument::Product::FxSwap(fx_swap_from_json(v)?))
    } else if let Some(v) = o.get("ndf") {
        Ok(instrument::Product::Ndf(ndf_from_json(v)?))
    } else if let Some(v) = o.get("perpetual_option") {
        Ok(instrument::Product::PerpetualOption(
            perpetual_option_from_json(v)?,
        ))
    } else if let Some(v) = o.get("listed_future_option") {
        Ok(instrument::Product::ListedFutureOption(
            listed_future_option_from_json(v)?,
        ))
    } else {
        Err(err(
            "instrument needs exactly one product (vanilla / strategy / \
             single_barrier / double_barrier / digital / touch / variance_swap / \
             volatility_swap / asian_option / forward_start / cliquet / quanto / \
             tarf / pivot / accumulator / lookback / window_barrier / american / \
             basket / fx_forward / fx_swap / ndf / perpetual_option / \
             listed_future_option)",
        ))
    }
}

pub(super) fn instrument_from_json(v: &Value) -> Result<Instrument> {
    let o = obj(v, "instrument")?;
    Ok(Instrument {
        // Every wire underlying arm: the legacy FX `pair` key (unchanged) or the
        // richer cross-asset `underlying` object (equity / commodity / digital
        // asset / metal), mirroring the gRPC contract field-for-field.
        underlying: instrument_underlying_from_json(o)?,
        tenor: opt_nested(o, "tenor", tenor_from_json)?,
        expiry_years: f64_field(o, "expiry_years")?,
        quantity: opt_nested(o, "quantity", quantity_from_json)?,
        side: enum_or_zero(o, "side"),
        solve: opt_nested(o, "solve", solve_from_json)?,
        // The booking-model selector (absent ⇒ 0 ⇒ PRICING_MODEL_DEFAULT, so the
        // analytic path is unchanged for existing browser requests).
        pricing_model: enum_or_zero(o, "pricing_model"),
        // The settlement style (absent ⇒ 0 ⇒ SETTLEMENT_STYLE_LINEAR, byte-identical
        // to the contract before this field existed). Selects the inverse/coin-
        // margined crypto convention when set; mirrors gRPC.
        settlement_style: enum_or_zero(o, "settlement_style"),
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
        // Caller identity rides in the unary body (mirrors the risk decoders): the
        // service resolves + gates the RFQ under the access posture and binds any
        // later accept to this principal.
        session_token: opt_string(o, "session_token"),
        principal: opt_nested(o, "principal", principal_from_json)?,
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
        // The multi-dealer line selector, mirroring the gRPC field exactly:
        // absent/empty selects the single-dealer quote (byte-identical to the
        // pre-panel contract); a `DealerQuote.lp_id` books that pinned panel row.
        lp_id: string_or_empty(o, "lp_id"),
        // Accepting caller identity (mirrors the risk decoders): resolved + gated +
        // bound to the requesting quote's recorded principal.
        session_token: opt_string(o, "session_token"),
        principal: opt_nested(o, "principal", principal_from_json)?,
    })
}

pub(super) fn quote_reject_from_json(o: &Map<String, Value>) -> Result<QuoteReject> {
    Ok(QuoteReject {
        quote_id: u64_field(o, "quote_id")?,
        reason: string_or_empty(o, "reason"),
        session_token: opt_string(o, "session_token"),
        principal: opt_nested(o, "principal", principal_from_json)?,
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

/// Encode one liquidity provider's line of a multi-dealer panel field-for-field
/// with the wire [`DealerQuote`] (snake_case keys; presence-tracked fields are
/// `null` when `None`) — the WS mirror of the gRPC message, never a fork.
fn dealer_quote_to_json(d: &DealerQuote) -> Value {
    json!({
        "lp_id": d.lp_id,
        "price": d.price.as_ref().map(two_way_to_json),
        "greeks": d.greeks.as_ref().map(greeks_to_json),
        "resolved_strike": d.resolved_strike,
        "valid_until_nanos": d.valid_until_nanos,
        "attribution": d.attribution.as_ref().map(attribution_to_json),
        "price_std_error": d.price_std_error,
    })
}

/// Encode the multi-dealer (RFQ-to-many) panel response field-for-field with the
/// wire [`MultiDealerQuote`]: the ranked dealer lines plus the touch winners, so
/// a WS client (GUI / Excel) can lift/hit a specific dealer's line by echoing its
/// `lp_id` on `accept_quote` — exactly the gRPC contract, second encoding.
pub(super) fn multi_dealer_quote_to_json(m: &MultiDealerQuote) -> Value {
    json!({
        "quote_id": m.quote_id,
        "idempotency_key": m.idempotency_key,
        "dealers": Value::Array(m.dealers.iter().map(dealer_quote_to_json).collect()),
        "best_bid_lp_id": m.best_bid_lp_id,
        "best_offer_lp_id": m.best_offer_lp_id,
        "conventions": m.conventions.as_ref().map(conventions_to_json),
        "epoch_nanos": m.epoch_nanos,
        "correlation_id": m.correlation_id,
        "surface_version": m.surface_version,
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
// Linear rates (fixed income) — the WS mirror of PricingService::PriceRates
// ---------------------------------------------------------------------------

/// Decode a `u32` proto field carried as a JSON integer, range-checked.
fn u32_field(o: &Map<String, Value>, key: &str) -> Result<u32> {
    u32::try_from(u64_field(o, key)?).map_err(|_| err(format!("field `{key}` out of u32 range")))
}

/// Decode a `PillarTenor` `{ years | months | maturity_date }` — exactly one arm.
/// `years`/`months` are whole counts; `maturity_date` is a `BrokenDate`.
fn pillar_tenor_from_json(v: &Value) -> Result<celnet_proto::PillarTenor> {
    use celnet_proto::pillar_tenor::Point;
    let o = obj(v, "tenor")?;
    let point = if o.contains_key("years") {
        Point::Years(u32_field(o, "years")?)
    } else if o.contains_key("months") {
        Point::Months(u32_field(o, "months")?)
    } else if o.contains_key("maturity_date") {
        Point::MaturityDate(nested(o, "maturity_date", broken_date_from_json)?)
    } else {
        return Err(err(
            "pillar `tenor` oneof: expected a `years`, `months`, or `maturity_date` arm",
        ));
    };
    Ok(celnet_proto::PillarTenor { point: Some(point) })
}

/// Decode one OIS curve pillar `{ tenor: {...}, par_rate }`.
fn ois_pillar_from_json(v: &Value) -> Result<OisPillar> {
    let o = obj(v, "ois_pillar")?;
    Ok(OisPillar {
        tenor: Some(nested(o, "tenor", pillar_tenor_from_json)?),
        par_rate: f64_field(o, "par_rate")?,
    })
}

/// Decode a `CurveSet` `{ currency, reference_date, ois_pillars[] }`.
fn curve_set_from_json(v: &Value) -> Result<CurveSet> {
    let o = obj(v, "curve_set")?;
    let pillars = o
        .get("ois_pillars")
        .and_then(Value::as_array)
        .ok_or_else(|| err("`curve_set.ois_pillars` must be an array"))?
        .iter()
        .map(ois_pillar_from_json)
        .collect::<Result<Vec<_>>>()?;
    Ok(CurveSet {
        currency: string_field(o, "currency")?,
        reference_date: Some(nested(o, "reference_date", broken_date_from_json)?),
        ois_pillars: pillars,
    })
}

/// Decode one `InstrumentQuote` `{ instrument_id, quote }` — a registry pillar id
/// paired with its observed calibrating quote.
fn instrument_quote_from_json(v: &Value) -> Result<InstrumentQuote> {
    let o = obj(v, "pillar")?;
    Ok(InstrumentQuote {
        instrument_id: string_field(o, "instrument_id")?,
        quote: f64_field(o, "quote")?,
    })
}

/// Decode one date-anchored `DatePillar` `{ maturity_date, quote }` — an explicit
/// maturity date paired with its observed simple ACT/360 rate.
fn date_pillar_from_json(v: &Value) -> Result<DatePillar> {
    let o = obj(v, "date_pillar")?;
    Ok(DatePillar {
        maturity_date: Some(nested(o, "maturity_date", broken_date_from_json)?),
        quote: f64_field(o, "quote")?,
    })
}

/// Decode an optional `{ key: [...] }` pillar array (absent ⇒ empty), mapping each
/// element through `decode`.
fn optional_pillar_array<T>(
    o: &Map<String, Value>,
    key: &str,
    decode: impl Fn(&Value) -> Result<T>,
) -> Result<Vec<T>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items.iter().map(decode).collect(),
        Some(_) => Err(err(format!("`build_curve.{key}` must be an array"))),
    }
}

/// Decode a `BuildCurveRequest` `{ request_id, currency, reference_date, pillars[],
/// date_pillars[], session_token }`. Both pillar arrays are optional (default empty);
/// the handler rejects a request carrying neither. The bearer `session_token` is
/// injected by the connection.
pub(super) fn build_curve_request_from_json(o: &Map<String, Value>) -> Result<BuildCurveRequest> {
    let pillars = optional_pillar_array(o, "pillars", instrument_quote_from_json)?;
    let date_pillars = optional_pillar_array(o, "date_pillars", date_pillar_from_json)?;
    Ok(BuildCurveRequest {
        request_id: string_field(o, "request_id")?,
        currency: string_field(o, "currency")?,
        reference_date: Some(nested(o, "reference_date", broken_date_from_json)?),
        pillars,
        session_token: string_field(o, "session_token")?,
        date_pillars,
    })
}

/// Encode one bootstrapped `CalibratedCurvePoint`.
fn calibrated_curve_point_to_json(p: &CalibratedCurvePoint) -> Value {
    json!({
        "instrument_id": p.instrument_id,
        "time_years": p.time_years,
        "discount_factor": p.discount_factor,
        "zero_rate": p.zero_rate,
        "label": p.label,
    })
}

/// Encode a `CalibratedCurve` `{ request_id, currency, reference_date, points[] }`.
pub(super) fn calibrated_curve_to_json(c: &CalibratedCurve) -> Value {
    json!({
        "request_id": c.request_id,
        "currency": c.currency,
        "reference_date": c.reference_date.as_ref().map(broken_date_to_json),
        "points": Value::Array(
            c.points.iter().map(calibrated_curve_point_to_json).collect(),
        ),
    })
}

/// Decode an `OisInstrument` `{ tenor_years, fixed_rate, notional, side }`.
fn ois_instrument_from_json(v: &Value) -> Result<OisInstrument> {
    let o = obj(v, "ois")?;
    Ok(OisInstrument {
        tenor_years: u32_field(o, "tenor_years")?,
        fixed_rate: f64_field(o, "fixed_rate")?,
        notional: f64_field(o, "notional")?,
        side: enum_or_zero(o, "side"),
    })
}

/// Decode a `VanillaIrsInstrument` `{ tenor_years, fixed_rate, notional, side,
/// fixed_frequency, fixed_day_count, float_frequency, float_day_count }`.
fn vanilla_irs_instrument_from_json(v: &Value) -> Result<VanillaIrsInstrument> {
    let o = obj(v, "irs")?;
    Ok(VanillaIrsInstrument {
        tenor_years: u32_field(o, "tenor_years")?,
        fixed_rate: f64_field(o, "fixed_rate")?,
        notional: f64_field(o, "notional")?,
        side: enum_or_zero(o, "side"),
        fixed_frequency: enum_or_zero(o, "fixed_frequency"),
        fixed_day_count: enum_or_zero(o, "fixed_day_count"),
        float_frequency: enum_or_zero(o, "float_frequency"),
        float_day_count: enum_or_zero(o, "float_day_count"),
    })
}

/// Decode a `FraInstrument` `{ start_months, end_months, fixed_rate, notional,
/// side, accrual_basis }`.
fn fra_instrument_from_json(v: &Value) -> Result<FraInstrument> {
    let o = obj(v, "fra")?;
    Ok(FraInstrument {
        start_months: u32_field(o, "start_months")?,
        end_months: u32_field(o, "end_months")?,
        fixed_rate: f64_field(o, "fixed_rate")?,
        notional: f64_field(o, "notional")?,
        side: enum_or_zero(o, "side"),
        accrual_basis: enum_or_zero(o, "accrual_basis"),
    })
}

/// Decode a `BondInstrument` `{ coupon_rate, coupon_frequency, day_count,
/// maturity_date, redemption, side }`.
fn bond_instrument_from_json(v: &Value) -> Result<BondInstrument> {
    let o = obj(v, "bond")?;
    Ok(BondInstrument {
        coupon_rate: f64_field(o, "coupon_rate")?,
        coupon_frequency: enum_or_zero(o, "coupon_frequency"),
        day_count: enum_or_zero(o, "day_count"),
        maturity_date: Some(nested(o, "maturity_date", broken_date_from_json)?),
        redemption: f64_field(o, "redemption")?,
        side: enum_or_zero(o, "side"),
    })
}

/// Decode a `RatesInstrument` oneof — the `ois` / `irs` / `fra` / `bond` arms. The
/// first present arm in declaration order wins (mirrors the generated codec).
fn rates_instrument_from_json(v: &Value) -> Result<RatesInstrument> {
    let o = obj(v, "instrument")?;
    let arm = if o.contains_key("ois") {
        rates_instrument::Instrument::Ois(ois_instrument_from_json(o.get("ois").unwrap())?)
    } else if o.contains_key("irs") {
        rates_instrument::Instrument::Irs(vanilla_irs_instrument_from_json(o.get("irs").unwrap())?)
    } else if o.contains_key("fra") {
        rates_instrument::Instrument::Fra(fra_instrument_from_json(o.get("fra").unwrap())?)
    } else if o.contains_key("bond") {
        rates_instrument::Instrument::Bond(bond_instrument_from_json(o.get("bond").unwrap())?)
    } else {
        return Err(err(
            "rates `instrument` oneof: expected an `ois`, `irs`, `fra`, or `bond` arm",
        ));
    };
    Ok(RatesInstrument {
        instrument: Some(arm),
    })
}

/// Decode a `RatesPriceRequest`.
pub(super) fn rates_price_request_from_json(o: &Map<String, Value>) -> Result<RatesPriceRequest> {
    Ok(RatesPriceRequest {
        request_id: u64_or_zero(o, "request_id"),
        curve_set: Some(nested(o, "curve_set", curve_set_from_json)?),
        instrument: Some(nested(o, "instrument", rates_instrument_from_json)?),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

/// Encode a `RatesPricingResult`.
fn rates_pricing_result_to_json(r: &RatesPricingResult) -> Value {
    json!({
        "pv": r.pv,
        "par_rate": r.par_rate,
        "pv01": r.pv01,
        "dv01": r.dv01,
        "key_rate_ladder": r.key_rate_ladder,
    })
}

/// Encode a `RatesPriceResponse`.
pub(super) fn rates_price_response_to_json(r: &RatesPriceResponse) -> Value {
    json!({
        "request_id": r.request_id,
        "result": r.result.as_ref().map(rates_pricing_result_to_json),
        "correlation_id": r.correlation_id,
    })
}

// ---------------------------------------------------------------------------
// XVA (valuation adjustments) — the WS mirror of PricingService::PriceXva
// ---------------------------------------------------------------------------

/// Decode a required `repeated double` field carried as a JSON array (absent/null
/// ⇒ empty). Each element must be a JSON number.
fn f64_array(o: &Map<String, Value>, key: &str) -> Result<Vec<f64>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_f64()
                    .ok_or_else(|| err(format!("`{key}[{i}]` must be a number")))
            })
            .collect(),
        Some(_) => Err(err(format!("`{key}` must be an array of numbers"))),
    }
}

/// Decode one `XvaTrade` `{ option_type, strike, expiry_years, vol, notional }`.
fn xva_trade_from_json(v: &Value) -> Result<XvaTrade> {
    let o = obj(v, "trade")?;
    Ok(XvaTrade {
        option_type: enum_or_zero(o, "option_type"),
        strike: f64_field(o, "strike")?,
        expiry_years: f64_field(o, "expiry_years")?,
        vol: f64_field(o, "vol")?,
        notional: f64_field(o, "notional")?,
    })
}

/// Decode an `XvaSurvivalCurve` `{ pillar_times[], hazard_rates[] }`. Empty
/// `pillar_times` with a single `hazard_rates` entry is the flat-curve form; equal
/// lengths otherwise (the piecewise form). Shape is enforced by the engine mapping.
fn xva_survival_curve_from_json(v: &Value) -> Result<XvaSurvivalCurve> {
    let o = obj(v, "survival_curve")?;
    Ok(XvaSurvivalCurve {
        pillar_times: f64_array(o, "pillar_times")?,
        hazard_rates: f64_array(o, "hazard_rates")?,
    })
}

/// Decode a `PriceXvaRequest` from the browser JSON shape.
pub(super) fn price_xva_request_from_json(o: &Map<String, Value>) -> Result<PriceXvaRequest> {
    let trades = o
        .get("trades")
        .and_then(Value::as_array)
        .ok_or_else(|| err("`trades` must be an array"))?
        .iter()
        .map(xva_trade_from_json)
        .collect::<Result<Vec<_>>>()?;
    Ok(PriceXvaRequest {
        request_id: u64_or_zero(o, "request_id"),
        trades,
        r_dom: f64_field(o, "r_dom")?,
        r_for: f64_field(o, "r_for")?,
        spot0: f64_field(o, "spot0")?,
        sigma: f64_field(o, "sigma")?,
        paths: u32_field(o, "paths")?,
        seed: u64_or_zero(o, "seed"),
        exposure_steps: u32_field(o, "exposure_steps")?,
        counterparty: Some(nested(o, "counterparty", xva_survival_curve_from_json)?),
        own: Some(nested(o, "own", xva_survival_curve_from_json)?),
        lgd_counterparty: f64_field(o, "lgd_counterparty")?,
        lgd_own: f64_field(o, "lgd_own")?,
        funding_spread: f64_field(o, "funding_spread")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

/// Encode an `XvaResult` `{ cva, dva, fva, total_adjustment }`.
fn xva_result_to_json(r: &WireXvaResult) -> Value {
    json!({
        "cva": r.cva,
        "dva": r.dva,
        "fva": r.fva,
        "total_adjustment": r.total_adjustment,
    })
}

/// Encode a `PriceXvaResponse`.
pub(super) fn price_xva_response_to_json(r: &PriceXvaResponse) -> Value {
    json!({
        "request_id": r.request_id,
        "result": r.result.as_ref().map(xva_result_to_json),
        "correlation_id": r.correlation_id,
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

/// Decode a `RatesSubscribe` (open a fixed-income streaming line) from the WS JSON
/// mirror — the FI counterpart of [`subscribe_from_json`]. Reuses the existing
/// hand codec's `rates_instrument_from_json` + `curve_set_from_json` (the same
/// decoders the rates unary edge validates against), so a WS client opens a rates
/// stream over the SAME multiplexed session it opens an FX stream on.
pub(super) fn rates_subscribe_from_json(
    o: &Map<String, Value>,
) -> Result<celnet_proto::RatesSubscribe> {
    Ok(celnet_proto::RatesSubscribe {
        subscription: Some(nested(o, "subscription", subscription_id_from_json)?),
        instrument: Some(nested(o, "instrument", rates_instrument_from_json)?),
        curve_set: Some(nested(o, "curve_set", curve_set_from_json)?),
        throttle_nanos: u64_or_zero(o, "throttle_nanos"),
        correlation_id: opt_u64(o, "correlation_id"),
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

/// Decode a `StreamAuth` authenticate frame from the WS JSON mirror, mirroring how
/// the gated risk requests carry auth in the body: an optional `session_token`
/// (validated against the live session registry) and an optional entitlement
/// `principal` (absent ⇒ the server's grant-all default under Permissive). This is
/// the WS-side counterpart of the `Authenticate` arm the gRPC stream already carries,
/// so a WS client (GUI/Excel) can pin its caller before any subscribe/execute.
pub(super) fn stream_auth_from_json(o: &Map<String, Value>) -> Result<StreamAuth> {
    Ok(StreamAuth {
        session_token: opt_string(o, "session_token"),
        principal: opt_nested(o, "principal", principal_from_json)?,
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
        underlying: Some(nested(o, "pair", underlying_from_json)?),
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
        Message::RatesStreamSnapshot(s) => {
            ("rates_stream_snapshot", rates_stream_snapshot_to_json(s))
        }
        Message::RatesStreamUpdate(u) => ("rates_stream_update", rates_stream_update_to_json(u)),
    };
    Some(tagged(tag, body))
}

fn rates_stream_snapshot_to_json(s: &celnet_proto::RatesStreamSnapshot) -> Value {
    json!({
        "subscription": s.subscription.as_ref().map(subscription_id_to_json),
        "sequence": s.sequence,
        "result": s.result.as_ref().map(rates_pricing_result_to_json),
        "curve_shift": s.curve_shift,
        "correlation_id": s.correlation_id,
        "epoch_nanos": s.epoch_nanos,
    })
}

fn rates_stream_update_to_json(u: &celnet_proto::RatesStreamUpdate) -> Value {
    json!({
        "subscription": u.subscription.as_ref().map(subscription_id_to_json),
        "sequence": u.sequence,
        "result": u.result.as_ref().map(rates_pricing_result_to_json),
        "curve_shift": u.curve_shift,
        "epoch_nanos": u.epoch_nanos,
    })
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
        "pair": s.underlying.as_ref().map(underlying_to_json),
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
        "t": i.t, "r_dom": i.r_dom(), "r_for": i.r_for(),
    })
}

fn org_key_to_json(k: &OrgKey) -> Value {
    json!({
        "trader": k.trader, "book": k.book, "desk": k.desk,
        "ccy_pair": k.underlying.as_ref().map(underlying_to_json),
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
        session_token: opt_string(o, "session_token"),
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
        session_token: opt_string(o, "session_token"),
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

// ---------------------------------------------------------------------------
// Linear-rates portfolio risk — WS mirror of RiskService::AggregateRatesRisk.
// Reuses the `price_rates` curve/instrument codecs (`curve_set_from_json`,
// `rates_instrument_from_json`) so the risk path decodes the *identical* market
// and economics the pricing edge does — no duplicate rates JSON shapes.
// ---------------------------------------------------------------------------

/// Decode one `RatesPosition` `{ position_id, entity, book, instrument }`. The
/// `instrument` arm reuses the shared `price_rates` oneof decoder; `entity`/`book`
/// are proto3 scalars (absent ⇒ the `0` default cell).
fn rates_position_from_json(v: &Value) -> Result<RatesPosition> {
    let o = obj(v, "position")?;
    Ok(RatesPosition {
        position_id: u64_or_zero(o, "position_id"),
        entity: opt_u32(o, "entity").unwrap_or(0),
        book: opt_u32(o, "book").unwrap_or(0),
        instrument: Some(nested(o, "instrument", rates_instrument_from_json)?),
    })
}

/// Decode the optional `RatesRiskScope` `{ entity?, book?, ccy? }` — each present
/// field narrows the rollup; an absent field does not constrain.
fn rates_risk_scope_from_json(v: &Value) -> Result<RatesRiskScope> {
    let o = obj(v, "scope")?;
    Ok(RatesRiskScope {
        entity: opt_u32(o, "entity"),
        book: opt_u32(o, "book"),
        ccy: opt_string(o, "ccy"),
    })
}

/// Decode an `AggregateRatesRiskRequest`. `session_token`/`principal` thread through
/// exactly as `aggregate_risk_request_from_json` does, so the gRPC handler's
/// deny-by-default entitlement check sees the same caller identity over WS as gRPC.
pub(super) fn aggregate_rates_risk_request_from_json(
    o: &Map<String, Value>,
) -> Result<AggregateRatesRiskRequest> {
    let positions = o
        .get("positions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(rates_position_from_json)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(AggregateRatesRiskRequest {
        curve_set: Some(nested(o, "curve_set", curve_set_from_json)?),
        positions,
        scope: opt_nested(o, "scope", rates_risk_scope_from_json)?,
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_u64(o, "correlation_id"),
        session_token: opt_string(o, "session_token"),
    })
}

/// Encode a `KeyRateDv01` `{ tenor_years, dv01 }` ladder bucket.
fn key_rate_dv01_to_json(k: &KeyRateDv01) -> Value {
    json!({
        "tenor_years": k.tenor_years,
        "dv01": k.dv01,
    })
}

/// Encode a per-currency `RatesRiskNode` with its netted scalars + tenor ladder.
fn rates_risk_node_to_json(n: &RatesRiskNode) -> Value {
    json!({
        "ccy": n.ccy,
        "net_pv": n.net_pv,
        "net_pv01": n.net_pv01,
        "net_dv01": n.net_dv01,
        "key_rate_ladder": Value::Array(n.key_rate_ladder.iter().map(key_rate_dv01_to_json).collect()),
    })
}

/// Encode an `AggregateRatesRiskResponse` `{ nodes[], correlation_id }`.
pub(super) fn aggregate_rates_risk_response_to_json(r: &AggregateRatesRiskResponse) -> Value {
    json!({
        "nodes": Value::Array(r.nodes.iter().map(rates_risk_node_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

// ---------------------------------------------------------------------------
// Dealer-quoting desk + linear-rates Book/List + notification push — the WS
// mirror of RfqDeskService / RiskService(BookRatesPosition,ListRatesPositions) /
// NotificationService. Reuses the shared `price_rates` curve/instrument codecs and
// the `RatesPosition` decoder so the desk path speaks the IDENTICAL market shape.
// ---------------------------------------------------------------------------

// ---- shared sub-type encoders (the rates instrument/curve, mirrored to JSON) ----

/// Encode a `BrokenDate` `{ year, month, day }`.
fn broken_date_to_json(d: &celnet_proto::BrokenDate) -> Value {
    json!({ "year": d.year, "month": d.month, "day": d.day })
}

/// Encode a `PillarTenor` to its single-arm object `{ years | months | maturity_date }`.
fn pillar_tenor_to_json(t: &celnet_proto::PillarTenor) -> Value {
    use celnet_proto::pillar_tenor::Point;
    match &t.point {
        Some(Point::Years(years)) => json!({ "years": years }),
        Some(Point::Months(months)) => json!({ "months": months }),
        Some(Point::MaturityDate(d)) => json!({ "maturity_date": broken_date_to_json(d) }),
        None => json!({}),
    }
}

/// Encode a `CurveSet` `{ currency, reference_date, ois_pillars[] }`.
fn curve_set_to_json(c: &CurveSet) -> Value {
    json!({
        "currency": c.currency,
        "reference_date": c.reference_date.as_ref().map(broken_date_to_json),
        "ois_pillars": Value::Array(
            c.ois_pillars
                .iter()
                .map(|p| json!({
                    "tenor": p.tenor.as_ref().map(pillar_tenor_to_json),
                    "par_rate": p.par_rate,
                }))
                .collect(),
        ),
    })
}

/// Encode a `RatesInstrument` oneof — the `ois` / `irs` / `fra` / `bond` arms.
fn rates_instrument_to_json(i: &RatesInstrument) -> Value {
    match i.instrument.as_ref() {
        Some(rates_instrument::Instrument::Ois(ois)) => json!({
            "ois": {
                "tenor_years": ois.tenor_years,
                "fixed_rate": ois.fixed_rate,
                "notional": ois.notional,
                "side": ois.side,
            }
        }),
        Some(rates_instrument::Instrument::Irs(irs)) => json!({
            "irs": {
                "tenor_years": irs.tenor_years,
                "fixed_rate": irs.fixed_rate,
                "notional": irs.notional,
                "side": irs.side,
                "fixed_frequency": irs.fixed_frequency,
                "fixed_day_count": irs.fixed_day_count,
                "float_frequency": irs.float_frequency,
                "float_day_count": irs.float_day_count,
            }
        }),
        Some(rates_instrument::Instrument::Fra(fra)) => json!({
            "fra": {
                "start_months": fra.start_months,
                "end_months": fra.end_months,
                "fixed_rate": fra.fixed_rate,
                "notional": fra.notional,
                "side": fra.side,
                "accrual_basis": fra.accrual_basis,
            }
        }),
        Some(rates_instrument::Instrument::Bond(bond)) => json!({
            "bond": {
                "coupon_rate": bond.coupon_rate,
                "coupon_frequency": bond.coupon_frequency,
                "day_count": bond.day_count,
                "maturity_date": bond.maturity_date.as_ref().map(broken_date_to_json),
                "redemption": bond.redemption,
                "side": bond.side,
            }
        }),
        None => json!({}),
    }
}

/// Encode a `RatesPosition` `{ position_id, entity, book, instrument }`.
fn rates_position_to_json(p: &RatesPosition) -> Value {
    json!({
        "position_id": p.position_id,
        "entity": p.entity,
        "book": p.book,
        "instrument": p.instrument.as_ref().map(rates_instrument_to_json),
    })
}

/// Decode a `DeskQuote` `{ price, notional, valid_for_ms, trader }`.
fn desk_quote_from_json(v: &Value) -> Result<DeskQuote> {
    let o = obj(v, "quote")?;
    Ok(DeskQuote {
        price: f64_field(o, "price")?,
        notional: f64_field(o, "notional")?,
        valid_for_ms: opt_u32(o, "valid_for_ms").unwrap_or(0),
        trader: string_or_empty(o, "trader"),
    })
}

/// Encode a `DeskQuote`.
fn desk_quote_to_json(q: &DeskQuote) -> Value {
    json!({
        "price": q.price,
        "notional": q.notional,
        "valid_for_ms": q.valid_for_ms,
        "trader": q.trader,
    })
}

/// Encode a `DeskRequest` (the desk's view of an inbound RFQ/IOI).
fn desk_request_to_json(r: &DeskRequest) -> Value {
    json!({
        "request_id": r.request_id,
        "kind": r.kind,
        "counterparty": r.counterparty,
        "desk": r.desk,
        "instrument": r.instrument.as_ref().map(rates_instrument_to_json),
        "curve_set": r.curve_set.as_ref().map(curve_set_to_json),
        "side": r.side,
        "notional": r.notional,
        "received_at_nanos": r.received_at_nanos,
        "expires_at_nanos": r.expires_at_nanos,
        "state": r.state,
        "quote": r.quote.as_ref().map(desk_quote_to_json),
        "correlation_id": r.correlation_id,
    })
}

/// Encode a `Deal` (a booked received deal).
fn deal_to_json(d: &Deal) -> Value {
    json!({
        "deal_id": d.deal_id,
        "request_id": d.request_id,
        "kind": d.kind,
        "counterparty": d.counterparty,
        "desk": d.desk,
        "instrument": d.instrument.as_ref().map(rates_instrument_to_json),
        "curve_set": d.curve_set.as_ref().map(curve_set_to_json),
        "side": d.side,
        "notional": d.notional,
        "price": d.price,
        "executed_at_nanos": d.executed_at_nanos,
        "trader": d.trader,
        "position_id": d.position_id,
        "correlation_id": d.correlation_id,
    })
}

/// Encode a `Notification` (a server→client push event). Public to the WS layer so
/// the notification drain task can frame it onto the connection's outbound sink.
pub(super) fn notification_to_json(n: &Notification) -> Value {
    json!({
        "type": "notification",
        "notification_id": n.notification_id,
        "kind": n.kind,
        "at_nanos": n.at_nanos,
        "request_id": n.request_id,
        "desk": n.desk,
        "counterparty": n.counterparty,
        "request_kind": n.request_kind,
        "headline": n.headline,
        "detail": n.detail,
    })
}

// ---- BookRatesPosition / ListRatesPositions (RiskService rates Book/List) ----

pub(super) fn book_rates_position_request_from_json(
    o: &Map<String, Value>,
) -> Result<BookRatesPositionRequest> {
    Ok(BookRatesPositionRequest {
        session_token: opt_string(o, "session_token"),
        position: Some(nested(o, "position", rates_position_from_json)?),
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_string(o, "correlation_id"),
    })
}

pub(super) fn book_rates_position_response_to_json(r: &BookRatesPositionResponse) -> Value {
    json!({ "position": r.position.as_ref().map(rates_position_to_json) })
}

pub(super) fn list_rates_positions_request_from_json(
    o: &Map<String, Value>,
) -> Result<ListRatesPositionsRequest> {
    Ok(ListRatesPositionsRequest {
        session_token: opt_string(o, "session_token"),
        scope: opt_nested(o, "scope", rates_risk_scope_from_json)?,
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_string(o, "correlation_id"),
    })
}

pub(super) fn list_rates_positions_response_to_json(r: &ListRatesPositionsResponse) -> Value {
    json!({
        "positions": Value::Array(r.positions.iter().map(rates_position_to_json).collect()),
    })
}

// ---- RfqDeskService unary requests / responses ----

pub(super) fn submit_desk_request_from_json(
    o: &Map<String, Value>,
) -> Result<SubmitDeskRequestRequest> {
    Ok(SubmitDeskRequestRequest {
        session_token: opt_string(o, "session_token"),
        kind: enum_or_zero(o, "kind"),
        counterparty: string_or_empty(o, "counterparty"),
        desk: string_or_empty(o, "desk"),
        instrument: Some(nested(o, "instrument", rates_instrument_from_json)?),
        curve_set: Some(nested(o, "curve_set", curve_set_from_json)?),
        side: enum_or_zero(o, "side"),
        notional: f64_or_zero(o, "notional"),
        ttl_ms: opt_u32(o, "ttl_ms").unwrap_or(0),
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_string(o, "correlation_id"),
    })
}

pub(super) fn submit_desk_request_response_to_json(
    r: &celnet_proto::SubmitDeskRequestResponse,
) -> Value {
    json!({ "request": r.request.as_ref().map(desk_request_to_json) })
}

pub(super) fn respond_desk_request_from_json(
    o: &Map<String, Value>,
) -> Result<RespondDeskRequestRequest> {
    // Exactly one of `quote` / `reject` selects the oneof arm.
    let response = match (o.contains_key("quote"), o.contains_key("reject")) {
        (true, false) => Some(RespondArm::Quote(desk_quote_from_json(
            o.get("quote").unwrap(),
        )?)),
        (false, true) => {
            let rj = obj(o.get("reject").unwrap(), "reject")?;
            Some(RespondArm::Reject(DeskReject {
                reason: string_or_empty(rj, "reason"),
            }))
        }
        (false, false) => None,
        (true, true) => {
            return Err(err(
                "respond: set exactly one of `quote` / `reject`, not both",
            ));
        }
    };
    Ok(RespondDeskRequestRequest {
        session_token: opt_string(o, "session_token"),
        request_id: string_or_empty(o, "request_id"),
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_string(o, "correlation_id"),
        response,
    })
}

pub(super) fn respond_desk_request_response_to_json(
    r: &celnet_proto::RespondDeskRequestResponse,
) -> Value {
    json!({ "request": r.request.as_ref().map(desk_request_to_json) })
}

pub(super) fn accept_desk_quote_from_json(
    o: &Map<String, Value>,
) -> Result<AcceptDeskQuoteRequest> {
    Ok(AcceptDeskQuoteRequest {
        session_token: opt_string(o, "session_token"),
        request_id: string_or_empty(o, "request_id"),
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_string(o, "correlation_id"),
    })
}

pub(super) fn accept_desk_quote_response_to_json(
    r: &celnet_proto::AcceptDeskQuoteResponse,
) -> Value {
    json!({
        "deal": r.deal.as_ref().map(deal_to_json),
        "request": r.request.as_ref().map(desk_request_to_json),
    })
}

/// Decode an optional `DeskRequestScope` `{ states[], desk? }`.
fn desk_request_scope_from_json(v: &Value) -> Result<DeskRequestScope> {
    let o = obj(v, "scope")?;
    let states = o
        .get("states")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_i64)
                .map(|n| n as i32)
                .collect()
        })
        .unwrap_or_default();
    Ok(DeskRequestScope {
        states,
        desk: opt_string(o, "desk"),
    })
}

pub(super) fn list_desk_requests_from_json(
    o: &Map<String, Value>,
) -> Result<ListDeskRequestsRequest> {
    Ok(ListDeskRequestsRequest {
        session_token: opt_string(o, "session_token"),
        scope: opt_nested(o, "scope", desk_request_scope_from_json)?,
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_string(o, "correlation_id"),
    })
}

pub(super) fn list_desk_requests_response_to_json(r: &ListDeskRequestsResponse) -> Value {
    json!({ "requests": Value::Array(r.requests.iter().map(desk_request_to_json).collect()) })
}

pub(super) fn list_deals_from_json(o: &Map<String, Value>) -> Result<ListDealsRequest> {
    let scope = o
        .get("scope")
        .and_then(Value::as_object)
        .map(|s| celnet_proto::DealScope {
            desk: opt_string(s, "desk"),
        });
    Ok(ListDealsRequest {
        session_token: opt_string(o, "session_token"),
        scope,
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_string(o, "correlation_id"),
    })
}

pub(super) fn list_deals_response_to_json(r: &ListDealsResponse) -> Value {
    json!({ "deals": Value::Array(r.deals.iter().map(deal_to_json).collect()) })
}

/// Decode a `StreamNotificationsRequest` (the WS `subscribe_notifications` frame):
/// `{ session_token?, scope:{ desks[] }?, principal?, correlation_id? }`.
pub(super) fn stream_notifications_request_from_json(
    o: &Map<String, Value>,
) -> Result<celnet_proto::StreamNotificationsRequest> {
    let scope =
        o.get("scope")
            .and_then(Value::as_object)
            .map(|s| celnet_proto::NotificationScope {
                desks: s
                    .get("desks")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            });
    Ok(celnet_proto::StreamNotificationsRequest {
        session_token: opt_string(o, "session_token"),
        scope,
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_string(o, "correlation_id"),
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
        session_token: opt_string(o, "session_token"),
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
        session_token: opt_string(o, "session_token"),
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
// fix-admin: manage the inbound FIX acceptor connections
// ---------------------------------------------------------------------------

/// A managed connection's runtime descriptor → JSON. `kind` is the proto enum tag
/// (an integer), mirroring how the risk frames encode their enums.
fn fix_connection_desc_to_json(d: &FixConnectionDesc) -> Value {
    json!({
        "id": d.id,
        "name": d.name,
        "kind": d.kind,
        "bind_addr": d.bind_addr,
        "sender_comp_id": d.sender_comp_id,
        "target_comp_id": d.target_comp_id,
        "enabled": d.enabled,
        "running": d.running,
        "bound_addr": d.bound_addr,
        "desk": d.desk,
    })
}

/// The editable connection fields (a nested `spec` object on create/update).
fn fix_connection_spec_from_json(v: &Value) -> Result<FixConnectionSpec> {
    let o = obj(v, "spec")?;
    Ok(FixConnectionSpec {
        id: string_or_empty(o, "id"),
        name: string_field(o, "name")?,
        kind: enum_or_zero(o, "kind"),
        bind_addr: string_field(o, "bind_addr")?,
        sender_comp_id: string_field(o, "sender_comp_id")?,
        target_comp_id: string_field(o, "target_comp_id")?,
        enabled: bool_or_false(o, "enabled"),
        desk: string_or_empty(o, "desk"),
    })
}

pub(super) fn list_fix_connections_request_from_json(
    o: &Map<String, Value>,
) -> Result<ListFixConnectionsRequest> {
    Ok(ListFixConnectionsRequest {
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_u64(o, "correlation_id"),
        session_token: opt_string(o, "session_token"),
    })
}

pub(super) fn list_fix_connections_response_to_json(r: &ListFixConnectionsResponse) -> Value {
    json!({
        "connections": Value::Array(r.connections.iter().map(fix_connection_desc_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn create_fix_connection_request_from_json(
    o: &Map<String, Value>,
) -> Result<CreateFixConnectionRequest> {
    Ok(CreateFixConnectionRequest {
        spec: Some(nested(o, "spec", fix_connection_spec_from_json)?),
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_u64(o, "correlation_id"),
        session_token: opt_string(o, "session_token"),
    })
}

pub(super) fn create_fix_connection_response_to_json(r: &CreateFixConnectionResponse) -> Value {
    json!({
        "connection": r.connection.as_ref().map(fix_connection_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn update_fix_connection_request_from_json(
    o: &Map<String, Value>,
) -> Result<UpdateFixConnectionRequest> {
    Ok(UpdateFixConnectionRequest {
        id: string_field(o, "id")?,
        spec: Some(nested(o, "spec", fix_connection_spec_from_json)?),
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_u64(o, "correlation_id"),
        session_token: opt_string(o, "session_token"),
    })
}

pub(super) fn update_fix_connection_response_to_json(r: &UpdateFixConnectionResponse) -> Value {
    json!({
        "connection": r.connection.as_ref().map(fix_connection_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn delete_fix_connection_request_from_json(
    o: &Map<String, Value>,
) -> Result<DeleteFixConnectionRequest> {
    Ok(DeleteFixConnectionRequest {
        id: string_field(o, "id")?,
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_u64(o, "correlation_id"),
        session_token: opt_string(o, "session_token"),
    })
}

pub(super) fn delete_fix_connection_response_to_json(r: &DeleteFixConnectionResponse) -> Value {
    json!({ "correlation_id": r.correlation_id })
}

pub(super) fn set_fix_connection_enabled_request_from_json(
    o: &Map<String, Value>,
) -> Result<SetFixConnectionEnabledRequest> {
    Ok(SetFixConnectionEnabledRequest {
        id: string_field(o, "id")?,
        enabled: bool_or_false(o, "enabled"),
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_u64(o, "correlation_id"),
        session_token: opt_string(o, "session_token"),
    })
}

pub(super) fn set_fix_connection_enabled_response_to_json(
    r: &SetFixConnectionEnabledResponse,
) -> Value {
    json!({
        "connection": r.connection.as_ref().map(fix_connection_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

/// One captured session frame → JSON. `direction` rides by its proto enum number;
/// `seq`/`epoch_nanos` are 64-bit (the client parses them losslessly as bigints).
fn fix_message_to_json(m: &FixMessage) -> Value {
    json!({
        "seq": m.seq,
        "connection_id": m.connection_id,
        "direction": m.direction,
        "msg_type": m.msg_type,
        "summary": m.summary,
        "epoch_nanos": m.epoch_nanos,
        "raw": m.raw,
    })
}

pub(super) fn list_fix_messages_request_from_json(
    o: &Map<String, Value>,
) -> Result<ListFixMessagesRequest> {
    let connection_id = o
        .get("connection_id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    Ok(ListFixMessagesRequest {
        connection_id,
        after_seq: opt_u64(o, "after_seq").unwrap_or(0),
        limit: u32::try_from(opt_u64(o, "limit").unwrap_or(0)).unwrap_or(u32::MAX),
        principal: opt_nested(o, "principal", principal_from_json)?,
        correlation_id: opt_u64(o, "correlation_id"),
        session_token: opt_string(o, "session_token"),
    })
}

pub(super) fn list_fix_messages_response_to_json(r: &ListFixMessagesResponse) -> Value {
    json!({
        "messages": Value::Array(r.messages.iter().map(fix_message_to_json).collect()),
        "latest_seq": r.latest_seq,
        "correlation_id": r.correlation_id,
    })
}

// ---------------------------------------------------------------------------
// auth: server-enforced sessions + user / desk administration
// ---------------------------------------------------------------------------

/// A user account → JSON. `role` rides by its proto enum number (mirroring the
/// other enum frames); the password hash is never present on the wire.
fn user_desc_to_json(u: &UserDesc) -> Value {
    json!({
        "id": u.id,
        "email": u.email,
        "display_name": u.display_name,
        "role": u.role,
        "desk_id": u.desk_id,
        "disabled": u.disabled,
    })
}

/// A desk → JSON.
fn desk_desc_to_json(d: &DeskDesc) -> Value {
    json!({ "id": d.id, "name": d.name })
}

pub(super) fn login_request_from_json(o: &Map<String, Value>) -> Result<LoginRequest> {
    Ok(LoginRequest {
        email: string_field(o, "email")?,
        password: string_field(o, "password")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn login_response_to_json(r: &LoginResponse) -> Value {
    json!({
        "session_token": r.session_token,
        "user": r.user.as_ref().map(user_desc_to_json),
        "expires_nanos": r.expires_nanos,
        // The caller's own effective set — a client gates its own affordances on it.
        "capabilities": capability_list_to_json(&r.capabilities),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn logout_request_from_json(o: &Map<String, Value>) -> Result<LogoutRequest> {
    Ok(LogoutRequest {
        session_token: string_field(o, "session_token")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn logout_response_to_json(r: &LogoutResponse) -> Value {
    json!({ "ended": r.ended, "correlation_id": r.correlation_id })
}

pub(super) fn list_users_request_from_json(o: &Map<String, Value>) -> Result<ListUsersRequest> {
    Ok(ListUsersRequest {
        session_token: string_field(o, "session_token")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn list_users_response_to_json(r: &ListUsersResponse) -> Value {
    json!({
        "users": Value::Array(r.users.iter().map(user_desc_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn create_user_request_from_json(o: &Map<String, Value>) -> Result<CreateUserRequest> {
    Ok(CreateUserRequest {
        session_token: string_field(o, "session_token")?,
        email: string_field(o, "email")?,
        display_name: string_field(o, "display_name")?,
        role: enum_or_zero(o, "role"),
        desk_id: opt_string(o, "desk_id"),
        password: string_field(o, "password")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn create_user_response_to_json(r: &CreateUserResponse) -> Value {
    json!({
        "user": r.user.as_ref().map(user_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn update_user_request_from_json(o: &Map<String, Value>) -> Result<UpdateUserRequest> {
    Ok(UpdateUserRequest {
        session_token: string_field(o, "session_token")?,
        id: string_field(o, "id")?,
        display_name: string_field(o, "display_name")?,
        role: enum_or_zero(o, "role"),
        desk_id: opt_string(o, "desk_id"),
        disabled: bool_or_false(o, "disabled"),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn update_user_response_to_json(r: &UpdateUserResponse) -> Value {
    json!({
        "user": r.user.as_ref().map(user_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn delete_user_request_from_json(o: &Map<String, Value>) -> Result<DeleteUserRequest> {
    Ok(DeleteUserRequest {
        session_token: string_field(o, "session_token")?,
        id: string_field(o, "id")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn delete_user_response_to_json(r: &DeleteUserResponse) -> Value {
    json!({ "removed": r.removed, "correlation_id": r.correlation_id })
}

pub(super) fn reset_password_request_from_json(
    o: &Map<String, Value>,
) -> Result<ResetPasswordRequest> {
    Ok(ResetPasswordRequest {
        session_token: string_field(o, "session_token")?,
        id: string_field(o, "id")?,
        new_password: string_field(o, "new_password")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn reset_password_response_to_json(r: &ResetPasswordResponse) -> Value {
    json!({ "correlation_id": r.correlation_id })
}

/// Encode one capability (`{action, asset}` labels) for the wire.
fn capability_desc_to_json(c: &CapabilityDesc) -> Value {
    json!({ "action": c.action, "asset": c.asset })
}

/// Encode a list of capabilities as a JSON array.
fn capability_list_to_json(caps: &[CapabilityDesc]) -> Value {
    Value::Array(caps.iter().map(capability_desc_to_json).collect())
}

/// Decode one capability object: both `action` and `asset` labels are required.
/// Label validity is the server's call (it rejects unknown labels); here we only
/// require the fields to be present strings.
fn capability_desc_from_json(v: &Value) -> Result<CapabilityDesc> {
    let o = v
        .as_object()
        .ok_or_else(|| err("each capability must be an object with action + asset"))?;
    Ok(CapabilityDesc {
        action: string_field(o, "action")?,
        asset: string_field(o, "asset")?,
    })
}

/// Decode an optional JSON array of capabilities under `key` (absent ⇒ empty).
fn capability_list_from_json(o: &Map<String, Value>, key: &str) -> Result<Vec<CapabilityDesc>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(arr) => arr
            .as_array()
            .ok_or_else(|| err(format!("`{key}` must be an array of capabilities")))?
            .iter()
            .map(capability_desc_from_json)
            .collect(),
    }
}

pub(super) fn get_user_capabilities_request_from_json(
    o: &Map<String, Value>,
) -> Result<GetUserCapabilitiesRequest> {
    Ok(GetUserCapabilitiesRequest {
        session_token: string_field(o, "session_token")?,
        id: string_field(o, "id")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn get_user_capabilities_response_to_json(r: &GetUserCapabilitiesResponse) -> Value {
    json!({
        "grants": capability_list_to_json(&r.grants),
        "denies": capability_list_to_json(&r.denies),
        "effective": capability_list_to_json(&r.effective),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn set_user_capabilities_request_from_json(
    o: &Map<String, Value>,
) -> Result<SetUserCapabilitiesRequest> {
    Ok(SetUserCapabilitiesRequest {
        session_token: string_field(o, "session_token")?,
        id: string_field(o, "id")?,
        grants: capability_list_from_json(o, "grants")?,
        denies: capability_list_from_json(o, "denies")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn set_user_capabilities_response_to_json(r: &SetUserCapabilitiesResponse) -> Value {
    json!({
        "grants": capability_list_to_json(&r.grants),
        "denies": capability_list_to_json(&r.denies),
        "effective": capability_list_to_json(&r.effective),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn get_role_capabilities_request_from_json(
    o: &Map<String, Value>,
) -> Result<GetRoleCapabilitiesRequest> {
    Ok(GetRoleCapabilitiesRequest {
        session_token: string_field(o, "session_token")?,
        role: enum_or_zero(o, "role"),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn get_role_capabilities_response_to_json(r: &GetRoleCapabilitiesResponse) -> Value {
    json!({
        "capabilities": capability_list_to_json(&r.capabilities),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn set_role_capabilities_request_from_json(
    o: &Map<String, Value>,
) -> Result<SetRoleCapabilitiesRequest> {
    Ok(SetRoleCapabilitiesRequest {
        session_token: string_field(o, "session_token")?,
        role: enum_or_zero(o, "role"),
        capabilities: capability_list_from_json(o, "capabilities")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn set_role_capabilities_response_to_json(r: &SetRoleCapabilitiesResponse) -> Value {
    json!({
        "capabilities": capability_list_to_json(&r.capabilities),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn list_desks_request_from_json(o: &Map<String, Value>) -> Result<ListDesksRequest> {
    Ok(ListDesksRequest {
        session_token: string_field(o, "session_token")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn list_desks_response_to_json(r: &ListDesksResponse) -> Value {
    json!({
        "desks": Value::Array(r.desks.iter().map(desk_desc_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn create_desk_request_from_json(o: &Map<String, Value>) -> Result<CreateDeskRequest> {
    Ok(CreateDeskRequest {
        session_token: string_field(o, "session_token")?,
        name: string_field(o, "name")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn create_desk_response_to_json(r: &CreateDeskResponse) -> Value {
    json!({
        "desk": r.desk.as_ref().map(desk_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn delete_desk_request_from_json(o: &Map<String, Value>) -> Result<DeleteDeskRequest> {
    Ok(DeleteDeskRequest {
        session_token: string_field(o, "session_token")?,
        id: string_field(o, "id")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn update_desk_request_from_json(o: &Map<String, Value>) -> Result<UpdateDeskRequest> {
    Ok(UpdateDeskRequest {
        session_token: string_field(o, "session_token")?,
        id: string_field(o, "id")?,
        name: string_field(o, "name")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn update_desk_response_to_json(r: &UpdateDeskResponse) -> Value {
    json!({
        "desk": r.desk.as_ref().map(desk_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn delete_desk_response_to_json(r: &DeleteDeskResponse) -> Value {
    json!({ "removed": r.removed, "correlation_id": r.correlation_id })
}

/// An entity → JSON.
fn entity_desc_to_json(e: &EntityDesc) -> Value {
    json!({ "key": e.key, "name": e.name, "code": e.code })
}

/// A book → JSON.
fn book_desc_to_json(b: &BookDesc) -> Value {
    json!({ "key": b.key, "name": b.name, "entity_key": b.entity_key })
}

pub(super) fn list_entities_request_from_json(
    o: &Map<String, Value>,
) -> Result<ListEntitiesRequest> {
    Ok(ListEntitiesRequest {
        session_token: string_field(o, "session_token")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn list_entities_response_to_json(r: &ListEntitiesResponse) -> Value {
    json!({
        "entities": Value::Array(r.entities.iter().map(entity_desc_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn create_entity_request_from_json(
    o: &Map<String, Value>,
) -> Result<CreateEntityRequest> {
    Ok(CreateEntityRequest {
        session_token: string_field(o, "session_token")?,
        name: string_field(o, "name")?,
        code: string_field(o, "code")?,
        key: opt_u32(o, "key").unwrap_or(0),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn create_entity_response_to_json(r: &CreateEntityResponse) -> Value {
    json!({
        "entity": r.entity.as_ref().map(entity_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn update_entity_request_from_json(
    o: &Map<String, Value>,
) -> Result<UpdateEntityRequest> {
    Ok(UpdateEntityRequest {
        session_token: string_field(o, "session_token")?,
        key: u32_field(o, "key")?,
        name: string_field(o, "name")?,
        code: string_field(o, "code")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn update_entity_response_to_json(r: &UpdateEntityResponse) -> Value {
    json!({
        "entity": r.entity.as_ref().map(entity_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn delete_entity_request_from_json(
    o: &Map<String, Value>,
) -> Result<DeleteEntityRequest> {
    Ok(DeleteEntityRequest {
        session_token: string_field(o, "session_token")?,
        key: u32_field(o, "key")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn delete_entity_response_to_json(r: &DeleteEntityResponse) -> Value {
    json!({ "removed": r.removed, "correlation_id": r.correlation_id })
}

pub(super) fn list_books_request_from_json(o: &Map<String, Value>) -> Result<ListBooksRequest> {
    Ok(ListBooksRequest {
        session_token: string_field(o, "session_token")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn list_books_response_to_json(r: &ListBooksResponse) -> Value {
    json!({
        "books": Value::Array(r.books.iter().map(book_desc_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn create_book_request_from_json(o: &Map<String, Value>) -> Result<CreateBookRequest> {
    Ok(CreateBookRequest {
        session_token: string_field(o, "session_token")?,
        name: string_field(o, "name")?,
        entity_key: u32_field(o, "entity_key")?,
        key: opt_u32(o, "key").unwrap_or(0),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn create_book_response_to_json(r: &CreateBookResponse) -> Value {
    json!({
        "book": r.book.as_ref().map(book_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn update_book_request_from_json(o: &Map<String, Value>) -> Result<UpdateBookRequest> {
    Ok(UpdateBookRequest {
        session_token: string_field(o, "session_token")?,
        key: u32_field(o, "key")?,
        name: string_field(o, "name")?,
        entity_key: u32_field(o, "entity_key")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn update_book_response_to_json(r: &UpdateBookResponse) -> Value {
    json!({
        "book": r.book.as_ref().map(book_desc_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn delete_book_request_from_json(o: &Map<String, Value>) -> Result<DeleteBookRequest> {
    Ok(DeleteBookRequest {
        session_token: string_field(o, "session_token")?,
        key: u32_field(o, "key")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn delete_book_response_to_json(r: &DeleteBookResponse) -> Value {
    json!({ "removed": r.removed, "correlation_id": r.correlation_id })
}

// --- instrument reference data (AuthService instrument RPCs) ----------------
//
// The WS mirror of the instrument registry. The `InstrumentDefDesc.definition`
// oneof is carried as a single family-keyed sub-object (`{ "ois": { … } }`); the
// keys are the proto oneof variant tokens. Every field is snake_case on the wire
// (the GUI codec maps camelCase ⇄ snake_case in lockstep with this file).

/// Read a `Vec<String>` array field, defaulting to empty when absent/non-array.
fn string_array(o: &Map<String, Value>, key: &str) -> Vec<String> {
    o.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn external_id_to_json(x: &celnet_proto::ExternalId) -> Value {
    json!({ "scheme": x.scheme, "value": x.value })
}

fn external_id_from_json(v: &Value) -> Result<celnet_proto::ExternalId> {
    let o = obj(v, "external_id")?;
    Ok(celnet_proto::ExternalId {
        scheme: string_field(o, "scheme")?,
        value: string_field(o, "value")?,
    })
}

/// Encode the family-specific block as `(variant_token, fields_object)`.
fn family_to_json(def: &InstrumentDefinition) -> (&'static str, Value) {
    match def {
        InstrumentDefinition::Deposit(d) => (
            "deposit",
            json!({
                "index": d.index, "tenor": d.tenor, "day_count": d.day_count,
                "business_day_convention": d.business_day_convention,
                "calendars": d.calendars, "spot_lag_days": d.spot_lag_days,
            }),
        ),
        InstrumentDefinition::Fra(f) => (
            "fra",
            json!({
                "float_index": f.float_index, "start_tenor": f.start_tenor,
                "end_tenor": f.end_tenor, "accrual_day_count": f.accrual_day_count,
                "business_day_convention": f.business_day_convention,
                "calendars": f.calendars, "spot_lag_days": f.spot_lag_days,
            }),
        ),
        InstrumentDefinition::StirFuture(s) => (
            "stir_future",
            json!({
                "contract_code": s.contract_code, "reference_start": s.reference_start,
                "reference_end": s.reference_end, "day_count": s.day_count,
                "calendars": s.calendars, "convexity_vol": s.convexity_vol,
                "contract_size": s.contract_size,
            }),
        ),
        InstrumentDefinition::VanillaIrs(v) => (
            "vanilla_irs",
            json!({
                "tenor": v.tenor, "fixed_frequency": v.fixed_frequency,
                "fixed_day_count": v.fixed_day_count, "float_index": v.float_index,
                "float_frequency": v.float_frequency, "float_day_count": v.float_day_count,
                "business_day_convention": v.business_day_convention,
                "calendars": v.calendars, "roll_convention": v.roll_convention,
                "spot_lag_days": v.spot_lag_days,
            }),
        ),
        InstrumentDefinition::Ois(o) => (
            "ois",
            json!({
                "tenor": o.tenor, "index": o.index, "fixed_frequency": o.fixed_frequency,
                "fixed_day_count": o.fixed_day_count, "float_day_count": o.float_day_count,
                "business_day_convention": o.business_day_convention,
                "calendars": o.calendars, "spot_lag_days": o.spot_lag_days,
            }),
        ),
        InstrumentDefinition::Bond(b) => (
            "bond",
            json!({
                "issuer": b.issuer, "coupon_rate": b.coupon_rate,
                "coupon_type": b.coupon_type, "coupon_frequency": b.coupon_frequency,
                "day_count": b.day_count,
                "issue_date": b.issue_date.as_ref().map(broken_date_to_json),
                "dated_date": b.dated_date.as_ref().map(broken_date_to_json),
                "first_coupon_date": b.first_coupon_date.as_ref().map(broken_date_to_json),
                "maturity_date": b.maturity_date.as_ref().map(broken_date_to_json),
                "redemption": b.redemption, "calendars": b.calendars,
            }),
        ),
    }
}

fn deposit_from_json(v: &Value) -> Result<celnet_proto::DepositDef> {
    let o = obj(v, "deposit")?;
    Ok(celnet_proto::DepositDef {
        index: string_field(o, "index")?,
        tenor: string_field(o, "tenor")?,
        day_count: string_field(o, "day_count")?,
        business_day_convention: string_field(o, "business_day_convention")?,
        calendars: string_array(o, "calendars"),
        spot_lag_days: opt_u32(o, "spot_lag_days").unwrap_or(0),
    })
}

fn fra_from_json(v: &Value) -> Result<celnet_proto::FraDef> {
    let o = obj(v, "fra")?;
    Ok(celnet_proto::FraDef {
        float_index: string_field(o, "float_index")?,
        start_tenor: string_field(o, "start_tenor")?,
        end_tenor: string_field(o, "end_tenor")?,
        accrual_day_count: string_field(o, "accrual_day_count")?,
        business_day_convention: string_field(o, "business_day_convention")?,
        calendars: string_array(o, "calendars"),
        spot_lag_days: opt_u32(o, "spot_lag_days").unwrap_or(0),
    })
}

fn stir_future_from_json(v: &Value) -> Result<celnet_proto::StirFutureDef> {
    let o = obj(v, "stir_future")?;
    Ok(celnet_proto::StirFutureDef {
        contract_code: string_field(o, "contract_code")?,
        reference_start: string_field(o, "reference_start")?,
        reference_end: string_field(o, "reference_end")?,
        day_count: string_field(o, "day_count")?,
        calendars: string_array(o, "calendars"),
        convexity_vol: opt_f64(o, "convexity_vol").unwrap_or(0.0),
        contract_size: opt_f64(o, "contract_size").unwrap_or(0.0),
    })
}

fn vanilla_irs_from_json(v: &Value) -> Result<celnet_proto::VanillaIrsDef> {
    let o = obj(v, "vanilla_irs")?;
    Ok(celnet_proto::VanillaIrsDef {
        tenor: string_field(o, "tenor")?,
        fixed_frequency: string_field(o, "fixed_frequency")?,
        fixed_day_count: string_field(o, "fixed_day_count")?,
        float_index: string_field(o, "float_index")?,
        float_frequency: string_field(o, "float_frequency")?,
        float_day_count: string_field(o, "float_day_count")?,
        business_day_convention: string_field(o, "business_day_convention")?,
        calendars: string_array(o, "calendars"),
        roll_convention: opt_string(o, "roll_convention").unwrap_or_default(),
        spot_lag_days: opt_u32(o, "spot_lag_days").unwrap_or(0),
    })
}

fn ois_def_from_json(v: &Value) -> Result<celnet_proto::OisDef> {
    let o = obj(v, "ois")?;
    Ok(celnet_proto::OisDef {
        tenor: string_field(o, "tenor")?,
        index: string_field(o, "index")?,
        fixed_frequency: string_field(o, "fixed_frequency")?,
        fixed_day_count: string_field(o, "fixed_day_count")?,
        float_day_count: string_field(o, "float_day_count")?,
        business_day_convention: string_field(o, "business_day_convention")?,
        calendars: string_array(o, "calendars"),
        spot_lag_days: opt_u32(o, "spot_lag_days").unwrap_or(0),
    })
}

fn opt_broken_date(o: &Map<String, Value>, key: &str) -> Result<Option<celnet_proto::BrokenDate>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(broken_date_from_json(v)?)),
    }
}

fn bond_from_json(v: &Value) -> Result<celnet_proto::BondDef> {
    let o = obj(v, "bond")?;
    Ok(celnet_proto::BondDef {
        issuer: string_field(o, "issuer")?,
        coupon_rate: opt_f64(o, "coupon_rate").unwrap_or(0.0),
        coupon_type: string_field(o, "coupon_type")?,
        coupon_frequency: opt_string(o, "coupon_frequency").unwrap_or_default(),
        day_count: string_field(o, "day_count")?,
        issue_date: opt_broken_date(o, "issue_date")?,
        dated_date: opt_broken_date(o, "dated_date")?,
        first_coupon_date: opt_broken_date(o, "first_coupon_date")?,
        maturity_date: opt_broken_date(o, "maturity_date")?,
        redemption: opt_f64(o, "redemption").unwrap_or(0.0),
        calendars: string_array(o, "calendars"),
    })
}

/// Detect and decode the family sub-object; `None` ⇒ no family set (the service
/// rejects it with `invalid_argument`).
fn family_from_json(o: &Map<String, Value>) -> Result<Option<InstrumentDefinition>> {
    if o.contains_key("deposit") {
        Ok(Some(InstrumentDefinition::Deposit(deposit_from_json(
            &o["deposit"],
        )?)))
    } else if o.contains_key("fra") {
        Ok(Some(InstrumentDefinition::Fra(fra_from_json(&o["fra"])?)))
    } else if o.contains_key("stir_future") {
        Ok(Some(InstrumentDefinition::StirFuture(
            stir_future_from_json(&o["stir_future"])?,
        )))
    } else if o.contains_key("vanilla_irs") {
        Ok(Some(InstrumentDefinition::VanillaIrs(
            vanilla_irs_from_json(&o["vanilla_irs"])?,
        )))
    } else if o.contains_key("ois") {
        Ok(Some(InstrumentDefinition::Ois(ois_def_from_json(
            &o["ois"],
        )?)))
    } else if o.contains_key("bond") {
        Ok(Some(InstrumentDefinition::Bond(bond_from_json(
            &o["bond"],
        )?)))
    } else {
        Ok(None)
    }
}

fn instrument_def_to_json(d: &InstrumentDefDesc) -> Value {
    let mut v = json!({
        "instrument_id": d.instrument_id,
        "name": d.name,
        "description": d.description,
        "currency": d.currency,
        "external_ids": Value::Array(d.external_ids.iter().map(external_id_to_json).collect()),
    });
    if let (Some(map), Some(def)) = (v.as_object_mut(), d.definition.as_ref()) {
        let (key, val) = family_to_json(def);
        map.insert(key.to_string(), val);
    }
    v
}

fn instrument_def_from_json(v: &Value) -> Result<InstrumentDefDesc> {
    let o = obj(v, "instrument")?;
    let external_ids = o
        .get("external_ids")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(external_id_from_json)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(InstrumentDefDesc {
        instrument_id: opt_string(o, "instrument_id").unwrap_or_default(),
        name: string_field(o, "name")?,
        description: opt_string(o, "description").unwrap_or_default(),
        currency: string_field(o, "currency")?,
        external_ids,
        definition: family_from_json(o)?,
    })
}

pub(super) fn list_instruments_request_from_json(
    o: &Map<String, Value>,
) -> Result<ListInstrumentsRequest> {
    Ok(ListInstrumentsRequest {
        session_token: string_field(o, "session_token")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn list_instruments_response_to_json(r: &ListInstrumentsResponse) -> Value {
    json!({
        "instruments": Value::Array(r.instruments.iter().map(instrument_def_to_json).collect()),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn get_instrument_request_from_json(
    o: &Map<String, Value>,
) -> Result<GetInstrumentRequest> {
    Ok(GetInstrumentRequest {
        session_token: string_field(o, "session_token")?,
        instrument_id: string_field(o, "instrument_id")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn get_instrument_response_to_json(r: &GetInstrumentResponse) -> Value {
    json!({
        "instrument": r.instrument.as_ref().map(instrument_def_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn create_instrument_request_from_json(
    o: &Map<String, Value>,
) -> Result<CreateInstrumentRequest> {
    Ok(CreateInstrumentRequest {
        session_token: string_field(o, "session_token")?,
        instrument: Some(nested(o, "instrument", instrument_def_from_json)?),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn create_instrument_response_to_json(r: &CreateInstrumentResponse) -> Value {
    json!({
        "instrument": r.instrument.as_ref().map(instrument_def_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn update_instrument_request_from_json(
    o: &Map<String, Value>,
) -> Result<UpdateInstrumentRequest> {
    Ok(UpdateInstrumentRequest {
        session_token: string_field(o, "session_token")?,
        instrument: Some(nested(o, "instrument", instrument_def_from_json)?),
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn update_instrument_response_to_json(r: &UpdateInstrumentResponse) -> Value {
    json!({
        "instrument": r.instrument.as_ref().map(instrument_def_to_json),
        "correlation_id": r.correlation_id,
    })
}

pub(super) fn delete_instrument_request_from_json(
    o: &Map<String, Value>,
) -> Result<DeleteInstrumentRequest> {
    Ok(DeleteInstrumentRequest {
        session_token: string_field(o, "session_token")?,
        instrument_id: string_field(o, "instrument_id")?,
        correlation_id: opt_u64(o, "correlation_id"),
    })
}

pub(super) fn delete_instrument_response_to_json(r: &DeleteInstrumentResponse) -> Value {
    json!({ "removed": r.removed, "correlation_id": r.correlation_id })
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

/// Test-support surface for the differential byte-identity harness
/// (`tests/ws_codec_differential.rs`, arch item G — `ws-codec-from-proto`).
///
/// The per-message hand encoders/decoders in this module are (intentionally)
/// module-private. This surface exposes thin `pub` wrappers around exactly the
/// ones the differential harness compares the [generated](super::generated_codec)
/// descriptor-driven encoder against — the hand codec is the byte-identity
/// reference. `#[doc(hidden)]`: this is not part of the client contract, only a
/// verification seam.
#[doc(hidden)]
pub mod diff_support {
    use celnet_proto::{
        ArbReport, CcyPair, CommodityRef, Conventions, CreateFixConnectionRequest,
        CreateFixConnectionResponse, CryptoPair, DeleteFixConnectionRequest,
        DeleteFixConnectionResponse, EquityRef, Execution, Greeks, Instrument,
        ListFixConnectionsRequest, ListFixConnectionsResponse, ListFixMessagesRequest,
        ListFixMessagesResponse, MarketContext, MetalPair, MultiDealerQuote, PriceRequest,
        PriceResponse, PriceXvaRequest, PriceXvaResponse, Quantity, Quote, QuoteAccept,
        QuoteReject, QuoteRequest, RateSensitivities, RatesPriceRequest, RatesPriceResponse,
        RejectAck, SetFixConnectionEnabledRequest, SetFixConnectionEnabledResponse, Solve,
        Strategy, StrikeOrDelta, Symbol, Tenor, Underlying, UpdateFixConnectionRequest,
        UpdateFixConnectionResponse, Vanilla,
    };
    // Wave-3 verb families (arch item G): the surface / server-side-risk / dealer-desk
    // hand-codec references the generated codec is proven byte-identical to.
    use celnet_proto::{
        AcceptDeskQuoteRequest, AcceptDeskQuoteResponse, AggregateRatesRiskRequest,
        AggregateRatesRiskResponse, AggregateRiskRequest, AggregateRiskResponse,
        BookRatesPositionRequest, BookRatesPositionResponse, DrillRiskRequest, DrillRiskResponse,
        GetSmileRequest, LimitStatusRequest, LimitStatusResponse, ListDealsRequest,
        ListDealsResponse, ListDeskRequestsRequest, ListDeskRequestsResponse, ListPositionsRequest,
        ListPositionsResponse, ListRatesPositionsRequest, ListRatesPositionsResponse,
        MarkSurfaceRequest, MarkSurfaceResponse, RespondDeskRequestRequest,
        RespondDeskRequestResponse, ScenarioRequest, ScenarioResponse, Smile,
        SubmitDeskRequestRequest, SubmitDeskRequestResponse,
    };
    // Wave-4 verb family (arch item G): the AuthService admin + session surface the
    // generated codec is proven byte-identical to (login/session, user/desk/entity/book
    // CRUD, capabilities + roles, the instrument registry, and `BuildCurve`).
    use celnet_proto::{
        BuildCurveRequest, CalibratedCurve, CreateBookRequest, CreateBookResponse,
        CreateDeskRequest, CreateDeskResponse, CreateEntityRequest, CreateEntityResponse,
        CreateInstrumentRequest, CreateInstrumentResponse, CreateUserRequest, CreateUserResponse,
        DeleteBookRequest, DeleteBookResponse, DeleteDeskRequest, DeleteDeskResponse,
        DeleteEntityRequest, DeleteEntityResponse, DeleteInstrumentRequest,
        DeleteInstrumentResponse, DeleteUserRequest, DeleteUserResponse, GetInstrumentRequest,
        GetInstrumentResponse, GetRoleCapabilitiesRequest, GetRoleCapabilitiesResponse,
        GetUserCapabilitiesRequest, GetUserCapabilitiesResponse, ListBooksRequest,
        ListBooksResponse, ListDesksRequest, ListDesksResponse, ListEntitiesRequest,
        ListEntitiesResponse, ListInstrumentsRequest, ListInstrumentsResponse, ListUsersRequest,
        ListUsersResponse, LoginRequest, LoginResponse, LogoutRequest, LogoutResponse,
        ResetPasswordRequest, ResetPasswordResponse, SetRoleCapabilitiesRequest,
        SetRoleCapabilitiesResponse, SetUserCapabilitiesRequest, SetUserCapabilitiesResponse,
        UpdateBookRequest, UpdateBookResponse, UpdateDeskRequest, UpdateDeskResponse,
        UpdateEntityRequest, UpdateEntityResponse, UpdateInstrumentRequest,
        UpdateInstrumentResponse, UpdateUserRequest, UpdateUserResponse,
    };
    use serde_json::{Map, Value};

    use super::CodecError;

    /// Hand-codec reference for `CcyPair` (byte-identity target).
    #[must_use]
    pub fn hand_ccy_pair(p: &CcyPair) -> Value {
        super::ccy_pair_to_json(p)
    }

    /// Hand-codec reference for the FX-legacy `Underlying` projection.
    #[must_use]
    pub fn hand_underlying(u: &Underlying) -> Value {
        super::underlying_to_json(u)
    }

    /// Hand-codec reference for the FX-legacy `MarketContext` projection.
    #[must_use]
    pub fn hand_market_context(m: &MarketContext) -> Value {
        super::market_context_to_json(m)
    }

    /// Hand-codec reference for `Greeks` (incl. the flat `rho_dom`/`rho_for`).
    #[must_use]
    pub fn hand_greeks(g: &Greeks) -> Value {
        super::greeks_to_json(g)
    }

    /// Hand-codec reference for the carry-tagged `RateSensitivities` oneof.
    #[must_use]
    pub fn hand_rate_sensitivities(rs: &RateSensitivities) -> Value {
        super::rate_sensitivities_to_json(rs)
    }

    /// Hand-codec `Tenor` decoder — the round-trip reference for the decode-only
    /// `Tenor` (no hand encoder exists; the generated encoder's output must
    /// round-trip through this decoder to the identical proto, exercising the
    /// camelCase `brokenDate` key).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_tenor_from_json(v: &Value) -> Result<Tenor, CodecError> {
        super::tenor_from_json(v)
    }

    /// Hand-codec `Strategy` decoder — the round-trip reference for the decode-only
    /// `Strategy` (repeated legs + the `strike`/`delta` oneof body).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_strategy_from_json(v: &Value) -> Result<Strategy, CodecError> {
        super::strategy_from_json(v)
    }

    // --- decode references (increment 3): the request-side leaf bodies the
    // generated decoder must produce byte-identically. Each is a thin `pub`
    // wrapper around the module-private hand decoder the generated path is
    // compared against. Comparing the DECODED proto (via `PartialEq`) is the
    // byte-identity contract: two encoders/decoders agree iff the message they
    // produce is the same, i.e. re-encodes to the identical protobuf bytes.

    /// Hand-codec `CcyPair` decoder (byte-identity target).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_ccy_pair_from_json(v: &Value) -> Result<CcyPair, CodecError> {
        super::ccy_pair_from_json(v)
    }

    /// Hand-codec FX-legacy `Underlying` decoder (the `{base, quote}` pair → FX arm).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_underlying_from_json(v: &Value) -> Result<Underlying, CodecError> {
        super::underlying_from_json(v)
    }

    /// Hand-codec richer cross-asset `underlying` oneof decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_underlying_object_from_json(v: &Value) -> Result<Underlying, CodecError> {
        super::underlying_object_from_json(v)
    }

    /// Hand-codec FX-legacy `MarketContext` decoder (`{spot, vol, r_dom, r_for}`).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_market_context_from_json(v: &Value) -> Result<MarketContext, CodecError> {
        super::market_context_from_json(v)
    }

    /// Hand-codec `Conventions` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_conventions_from_json(v: &Value) -> Result<Conventions, CodecError> {
        super::conventions_from_json(v)
    }

    /// Hand-codec `Quantity` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_quantity_from_json(v: &Value) -> Result<Quantity, CodecError> {
        super::quantity_from_json(v)
    }

    /// Hand-codec `Solve` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_solve_from_json(v: &Value) -> Result<Solve, CodecError> {
        super::solve_from_json(v)
    }

    /// Hand-codec `StrikeOrDelta` decoder (the `strike`/`delta` oneof).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_strike_or_delta_from_json(v: &Value) -> Result<StrikeOrDelta, CodecError> {
        super::strike_or_delta_from_json(v)
    }

    /// Hand-codec `Vanilla` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_vanilla_from_json(v: &Value) -> Result<Vanilla, CodecError> {
        super::vanilla_from_json(v)
    }

    /// Hand-codec `MetalPair` decoder (the `metal` cross-asset arm).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_metal_pair_from_json(v: &Value) -> Result<MetalPair, CodecError> {
        super::metal_pair_from_json(v)
    }

    /// Hand-codec `Symbol` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_symbol_from_json(v: &Value) -> Result<Symbol, CodecError> {
        super::symbol_from_json(v)
    }

    /// Hand-codec `EquityRef` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_equity_ref_from_json(v: &Value) -> Result<EquityRef, CodecError> {
        super::equity_ref_from_json(v)
    }

    /// Hand-codec `CommodityRef` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_commodity_ref_from_json(v: &Value) -> Result<CommodityRef, CodecError> {
        super::commodity_ref_from_json(v)
    }

    /// Hand-codec `CryptoPair` decoder (the `digital_asset` cross-asset arm).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_crypto_pair_from_json(v: &Value) -> Result<CryptoPair, CodecError> {
        super::crypto_pair_from_json(v)
    }

    // --- decode references (increment 4): the Instrument-consuming Price family.
    // The generated descriptor-driven decoder must produce byte-identically the
    // proto message these hand decoders produce, over the full price / rates / xva
    // conformance corpus (the browser/Excel wire shapes) — every one of the 24
    // product arms and the three request envelopes.

    /// Hand-codec `Instrument` decoder (the 24-arm `product` oneof + the FX-legacy
    /// `underlying` dual-key). The byte-identity reference for the whole
    /// Instrument-consuming request surface.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_instrument_from_json(v: &Value) -> Result<Instrument, CodecError> {
        super::instrument_from_json(v)
    }

    /// Hand-codec `PriceRequest` envelope decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_price_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<PriceRequest, CodecError> {
        super::price_request_from_json(o)
    }

    /// Hand-codec `RatesPriceRequest` envelope decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_rates_price_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<RatesPriceRequest, CodecError> {
        super::rates_price_request_from_json(o)
    }

    /// Hand-codec `PriceXvaRequest` envelope decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_price_xva_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<PriceXvaRequest, CodecError> {
        super::price_xva_request_from_json(o)
    }

    // --- encode references (increment 4): the Price-family RESPONSE surface. ----

    /// Hand-codec reference for `Conventions` encode (the echoed convention block).
    #[must_use]
    pub fn hand_conventions_to_json(c: &Conventions) -> Value {
        super::conventions_to_json(c)
    }

    /// Hand-codec reference for the `PriceResponse` one-shot pricing reply.
    #[must_use]
    pub fn hand_price_response_to_json(r: &PriceResponse) -> Value {
        super::price_response_to_json(r)
    }

    /// Hand-codec reference for the `RatesPriceResponse` reply.
    #[must_use]
    pub fn hand_rates_price_response_to_json(r: &RatesPriceResponse) -> Value {
        super::rates_price_response_to_json(r)
    }

    /// Hand-codec reference for the `PriceXvaResponse` reply.
    #[must_use]
    pub fn hand_price_xva_response_to_json(r: &PriceXvaResponse) -> Value {
        super::price_xva_response_to_json(r)
    }

    /// Hand-codec reference for the `ArbReport` encode (incl. the synthesized
    /// `smile_model_label`).
    #[must_use]
    pub fn hand_arb_report_to_json(a: &ArbReport) -> Value {
        super::arb_report_to_json(a)
    }

    // --- FixAdminService references: the request decoders + response encoders the
    // generated fix-admin codec is proven byte-identical to (the six connection /
    // message admin verbs, including the nested `EntitlementPrincipal` and the
    // quirked `ListFixMessagesRequest` projection).

    /// Hand-codec `ListFixConnectionsRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_fix_connections_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListFixConnectionsRequest, CodecError> {
        super::list_fix_connections_request_from_json(o)
    }

    /// Hand-codec `CreateFixConnectionRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_create_fix_connection_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<CreateFixConnectionRequest, CodecError> {
        super::create_fix_connection_request_from_json(o)
    }

    /// Hand-codec `UpdateFixConnectionRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_update_fix_connection_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<UpdateFixConnectionRequest, CodecError> {
        super::update_fix_connection_request_from_json(o)
    }

    /// Hand-codec `DeleteFixConnectionRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_delete_fix_connection_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<DeleteFixConnectionRequest, CodecError> {
        super::delete_fix_connection_request_from_json(o)
    }

    /// Hand-codec `SetFixConnectionEnabledRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_set_fix_connection_enabled_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<SetFixConnectionEnabledRequest, CodecError> {
        super::set_fix_connection_enabled_request_from_json(o)
    }

    /// Hand-codec `ListFixMessagesRequest` decoder (the quirked
    /// whitespace-`connection_id` / saturating-`limit` projection).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_fix_messages_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListFixMessagesRequest, CodecError> {
        super::list_fix_messages_request_from_json(o)
    }

    /// Hand-codec reference for the `ListFixConnectionsResponse` encode.
    #[must_use]
    pub fn hand_list_fix_connections_response_to_json(r: &ListFixConnectionsResponse) -> Value {
        super::list_fix_connections_response_to_json(r)
    }

    /// Hand-codec reference for the `CreateFixConnectionResponse` encode.
    #[must_use]
    pub fn hand_create_fix_connection_response_to_json(r: &CreateFixConnectionResponse) -> Value {
        super::create_fix_connection_response_to_json(r)
    }

    /// Hand-codec reference for the `UpdateFixConnectionResponse` encode.
    #[must_use]
    pub fn hand_update_fix_connection_response_to_json(r: &UpdateFixConnectionResponse) -> Value {
        super::update_fix_connection_response_to_json(r)
    }

    /// Hand-codec reference for the `DeleteFixConnectionResponse` encode.
    #[must_use]
    pub fn hand_delete_fix_connection_response_to_json(r: &DeleteFixConnectionResponse) -> Value {
        super::delete_fix_connection_response_to_json(r)
    }

    /// Hand-codec reference for the `SetFixConnectionEnabledResponse` encode.
    #[must_use]
    pub fn hand_set_fix_connection_enabled_response_to_json(
        r: &SetFixConnectionEnabledResponse,
    ) -> Value {
        super::set_fix_connection_enabled_response_to_json(r)
    }

    /// Hand-codec reference for the `ListFixMessagesResponse` encode.
    #[must_use]
    pub fn hand_list_fix_messages_response_to_json(r: &ListFixMessagesResponse) -> Value {
        super::list_fix_messages_response_to_json(r)
    }

    // --- QuoteService references: the RFQ-lifecycle request decoders + reply encoders
    // the generated quote codec is proven byte-identical to (incl. the who's-trading
    // `AttributionRecord` camelCase tree + the suppressed `Execution.instrument`).

    /// Hand-codec `QuoteRequest` decoder (the FX-legacy `Instrument` projection +
    /// the attribution tree).
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_quote_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<QuoteRequest, CodecError> {
        super::quote_request_from_json(o)
    }

    /// Hand-codec `QuoteAccept` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_quote_accept_from_json(o: &Map<String, Value>) -> Result<QuoteAccept, CodecError> {
        super::quote_accept_from_json(o)
    }

    /// Hand-codec `QuoteReject` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_quote_reject_from_json(o: &Map<String, Value>) -> Result<QuoteReject, CodecError> {
        super::quote_reject_from_json(o)
    }

    /// Hand-codec reference for the single-dealer `Quote` encode.
    #[must_use]
    pub fn hand_quote_to_json(q: &Quote) -> Value {
        super::quote_to_json(q)
    }

    /// Hand-codec reference for the ranked-panel `MultiDealerQuote` encode.
    #[must_use]
    pub fn hand_multi_dealer_quote_to_json(m: &MultiDealerQuote) -> Value {
        super::multi_dealer_quote_to_json(m)
    }

    /// Hand-codec reference for the `Execution` booking-confirmation encode.
    #[must_use]
    pub fn hand_execution_to_json(e: &Execution) -> Value {
        super::execution_to_json(e)
    }

    /// Hand-codec reference for the `RejectAck` encode.
    #[must_use]
    pub fn hand_reject_ack_to_json(a: &RejectAck) -> Value {
        super::reject_ack_to_json(a)
    }

    // --- SurfaceService references (wave 3): the smile-read / broker-mark / scenario
    // request decoders + reply encoders the generated surface codec is proven
    // byte-identical to (incl. the `smile_model` dual int/string enum, the
    // `VegaBucket`/`CrossGamma` hardcoded-field decode, and the `Smile` sub-tree).

    /// Hand-codec `GetSmileRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_get_smile_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<GetSmileRequest, CodecError> {
        super::get_smile_request_from_json(o)
    }

    /// Hand-codec `MarkSurfaceRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_mark_surface_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<MarkSurfaceRequest, CodecError> {
        super::mark_surface_request_from_json(o)
    }

    /// Hand-codec `ScenarioRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_scenario_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ScenarioRequest, CodecError> {
        super::scenario_request_from_json(o)
    }

    /// Hand-codec reference for the `GetSmile` reply (`Smile`).
    #[must_use]
    pub fn hand_smile_reply_to_json(s: &Smile) -> Value {
        super::smile_reply_to_json(s)
    }

    /// Hand-codec reference for the `MarkSurfaceResponse` reply.
    #[must_use]
    pub fn hand_mark_surface_response_to_json(r: &MarkSurfaceResponse) -> Value {
        super::mark_surface_response_to_json(r)
    }

    /// Hand-codec reference for the `ScenarioResponse` reply.
    #[must_use]
    pub fn hand_scenario_response_to_json(r: &ScenarioResponse) -> Value {
        super::scenario_response_to_json(r)
    }

    // --- RiskService references (wave 3): the position / aggregate / drill / limit
    // request decoders + response encoders (incl. the FX-legacy `VanillaInputs`
    // `r_dom`/`r_for` and `OrgKey.underlying`→`ccy_pair` projections, the shared
    // `EntitlementPrincipal`/`ReportingNumeraire` trees, and the linear-rates
    // book/list + firm rollup).

    /// Hand-codec `ListPositionsRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_positions_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListPositionsRequest, CodecError> {
        super::list_positions_request_from_json(o)
    }

    /// Hand-codec `AggregateRiskRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_aggregate_risk_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<AggregateRiskRequest, CodecError> {
        super::aggregate_risk_request_from_json(o)
    }

    /// Hand-codec `AggregateRatesRiskRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_aggregate_rates_risk_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<AggregateRatesRiskRequest, CodecError> {
        super::aggregate_rates_risk_request_from_json(o)
    }

    /// Hand-codec `DrillRiskRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_drill_risk_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<DrillRiskRequest, CodecError> {
        super::drill_risk_request_from_json(o)
    }

    /// Hand-codec `LimitStatusRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_limit_status_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<LimitStatusRequest, CodecError> {
        super::limit_status_request_from_json(o)
    }

    /// Hand-codec `BookRatesPositionRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_book_rates_position_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<BookRatesPositionRequest, CodecError> {
        super::book_rates_position_request_from_json(o)
    }

    /// Hand-codec `ListRatesPositionsRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_rates_positions_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListRatesPositionsRequest, CodecError> {
        super::list_rates_positions_request_from_json(o)
    }

    /// Hand-codec reference for the `ListPositionsResponse` reply.
    #[must_use]
    pub fn hand_list_positions_response_to_json(r: &ListPositionsResponse) -> Value {
        super::list_positions_response_to_json(r)
    }

    /// Hand-codec reference for the `AggregateRiskResponse` reply.
    #[must_use]
    pub fn hand_aggregate_risk_response_to_json(r: &AggregateRiskResponse) -> Value {
        super::aggregate_risk_response_to_json(r)
    }

    /// Hand-codec reference for the `AggregateRatesRiskResponse` reply.
    #[must_use]
    pub fn hand_aggregate_rates_risk_response_to_json(r: &AggregateRatesRiskResponse) -> Value {
        super::aggregate_rates_risk_response_to_json(r)
    }

    /// Hand-codec reference for the `DrillRiskResponse` reply.
    #[must_use]
    pub fn hand_drill_risk_response_to_json(r: &DrillRiskResponse) -> Value {
        super::drill_risk_response_to_json(r)
    }

    /// Hand-codec reference for the `LimitStatusResponse` reply.
    #[must_use]
    pub fn hand_limit_status_response_to_json(r: &LimitStatusResponse) -> Value {
        super::limit_status_response_to_json(r)
    }

    /// Hand-codec reference for the `BookRatesPositionResponse` reply.
    #[must_use]
    pub fn hand_book_rates_position_response_to_json(r: &BookRatesPositionResponse) -> Value {
        super::book_rates_position_response_to_json(r)
    }

    /// Hand-codec reference for the `ListRatesPositionsResponse` reply.
    #[must_use]
    pub fn hand_list_rates_positions_response_to_json(r: &ListRatesPositionsResponse) -> Value {
        super::list_rates_positions_response_to_json(r)
    }

    // --- RfqDeskService references (wave 3): the desk submit/respond/accept/list
    // request decoders + response encoders (incl. the `respond` oneof
    // error-on-both quirk, the `list_deals` non-erroring scope, and the
    // `RatesInstrument`/`CurveSet` encode tree in `DeskRequest`/`Deal`).

    /// Hand-codec `SubmitDeskRequestRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_submit_desk_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<SubmitDeskRequestRequest, CodecError> {
        super::submit_desk_request_from_json(o)
    }

    /// Hand-codec `RespondDeskRequestRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_respond_desk_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<RespondDeskRequestRequest, CodecError> {
        super::respond_desk_request_from_json(o)
    }

    /// Hand-codec `AcceptDeskQuoteRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_accept_desk_quote_from_json(
        o: &Map<String, Value>,
    ) -> Result<AcceptDeskQuoteRequest, CodecError> {
        super::accept_desk_quote_from_json(o)
    }

    /// Hand-codec `ListDeskRequestsRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_desk_requests_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListDeskRequestsRequest, CodecError> {
        super::list_desk_requests_from_json(o)
    }

    /// Hand-codec `ListDealsRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_deals_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListDealsRequest, CodecError> {
        super::list_deals_from_json(o)
    }

    /// Hand-codec reference for the `SubmitDeskRequest` reply.
    #[must_use]
    pub fn hand_submit_desk_request_response_to_json(r: &SubmitDeskRequestResponse) -> Value {
        super::submit_desk_request_response_to_json(r)
    }

    /// Hand-codec reference for the `RespondDeskRequest` reply.
    #[must_use]
    pub fn hand_respond_desk_request_response_to_json(r: &RespondDeskRequestResponse) -> Value {
        super::respond_desk_request_response_to_json(r)
    }

    /// Hand-codec reference for the `AcceptDeskQuote` reply.
    #[must_use]
    pub fn hand_accept_desk_quote_response_to_json(r: &AcceptDeskQuoteResponse) -> Value {
        super::accept_desk_quote_response_to_json(r)
    }

    /// Hand-codec reference for the `ListDeskRequests` reply.
    #[must_use]
    pub fn hand_list_desk_requests_response_to_json(r: &ListDeskRequestsResponse) -> Value {
        super::list_desk_requests_response_to_json(r)
    }

    /// Hand-codec reference for the `ListDeals` reply.
    #[must_use]
    pub fn hand_list_deals_response_to_json(r: &ListDealsResponse) -> Value {
        super::list_deals_response_to_json(r)
    }

    // --- wave 4: AuthService references (the frozen byte-identity oracle) --------
    //
    // Thin `pub` wrappers around the module-private hand codec for the auth family,
    // the final unary family the generated codec is proven byte-identical to. Request
    // decoders (compared via decoded-proto `PartialEq` + re-encoded bytes) then reply
    // encoders (compared via exact JSON text).

    /// Hand-codec `LoginRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_login_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<LoginRequest, CodecError> {
        super::login_request_from_json(o)
    }

    /// Hand-codec `LoginResponse` encoder.
    #[must_use]
    pub fn hand_login_response_to_json(r: &LoginResponse) -> Value {
        super::login_response_to_json(r)
    }

    /// Hand-codec `LogoutRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_logout_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<LogoutRequest, CodecError> {
        super::logout_request_from_json(o)
    }

    /// Hand-codec `LogoutResponse` encoder.
    #[must_use]
    pub fn hand_logout_response_to_json(r: &LogoutResponse) -> Value {
        super::logout_response_to_json(r)
    }

    /// Hand-codec `ListUsersRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_users_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListUsersRequest, CodecError> {
        super::list_users_request_from_json(o)
    }

    /// Hand-codec `ListUsersResponse` encoder.
    #[must_use]
    pub fn hand_list_users_response_to_json(r: &ListUsersResponse) -> Value {
        super::list_users_response_to_json(r)
    }

    /// Hand-codec `CreateUserRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_create_user_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<CreateUserRequest, CodecError> {
        super::create_user_request_from_json(o)
    }

    /// Hand-codec `CreateUserResponse` encoder.
    #[must_use]
    pub fn hand_create_user_response_to_json(r: &CreateUserResponse) -> Value {
        super::create_user_response_to_json(r)
    }

    /// Hand-codec `UpdateUserRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_update_user_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<UpdateUserRequest, CodecError> {
        super::update_user_request_from_json(o)
    }

    /// Hand-codec `UpdateUserResponse` encoder.
    #[must_use]
    pub fn hand_update_user_response_to_json(r: &UpdateUserResponse) -> Value {
        super::update_user_response_to_json(r)
    }

    /// Hand-codec `DeleteUserRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_delete_user_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<DeleteUserRequest, CodecError> {
        super::delete_user_request_from_json(o)
    }

    /// Hand-codec `DeleteUserResponse` encoder.
    #[must_use]
    pub fn hand_delete_user_response_to_json(r: &DeleteUserResponse) -> Value {
        super::delete_user_response_to_json(r)
    }

    /// Hand-codec `ResetPasswordRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_reset_password_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ResetPasswordRequest, CodecError> {
        super::reset_password_request_from_json(o)
    }

    /// Hand-codec `ResetPasswordResponse` encoder.
    #[must_use]
    pub fn hand_reset_password_response_to_json(r: &ResetPasswordResponse) -> Value {
        super::reset_password_response_to_json(r)
    }

    /// Hand-codec `GetUserCapabilitiesRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_get_user_capabilities_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<GetUserCapabilitiesRequest, CodecError> {
        super::get_user_capabilities_request_from_json(o)
    }

    /// Hand-codec `GetUserCapabilitiesResponse` encoder.
    #[must_use]
    pub fn hand_get_user_capabilities_response_to_json(r: &GetUserCapabilitiesResponse) -> Value {
        super::get_user_capabilities_response_to_json(r)
    }

    /// Hand-codec `SetUserCapabilitiesRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_set_user_capabilities_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<SetUserCapabilitiesRequest, CodecError> {
        super::set_user_capabilities_request_from_json(o)
    }

    /// Hand-codec `SetUserCapabilitiesResponse` encoder.
    #[must_use]
    pub fn hand_set_user_capabilities_response_to_json(r: &SetUserCapabilitiesResponse) -> Value {
        super::set_user_capabilities_response_to_json(r)
    }

    /// Hand-codec `GetRoleCapabilitiesRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_get_role_capabilities_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<GetRoleCapabilitiesRequest, CodecError> {
        super::get_role_capabilities_request_from_json(o)
    }

    /// Hand-codec `GetRoleCapabilitiesResponse` encoder.
    #[must_use]
    pub fn hand_get_role_capabilities_response_to_json(r: &GetRoleCapabilitiesResponse) -> Value {
        super::get_role_capabilities_response_to_json(r)
    }

    /// Hand-codec `SetRoleCapabilitiesRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_set_role_capabilities_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<SetRoleCapabilitiesRequest, CodecError> {
        super::set_role_capabilities_request_from_json(o)
    }

    /// Hand-codec `SetRoleCapabilitiesResponse` encoder.
    #[must_use]
    pub fn hand_set_role_capabilities_response_to_json(r: &SetRoleCapabilitiesResponse) -> Value {
        super::set_role_capabilities_response_to_json(r)
    }

    /// Hand-codec `ListDesksRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_desks_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListDesksRequest, CodecError> {
        super::list_desks_request_from_json(o)
    }

    /// Hand-codec `ListDesksResponse` encoder.
    #[must_use]
    pub fn hand_list_desks_response_to_json(r: &ListDesksResponse) -> Value {
        super::list_desks_response_to_json(r)
    }

    /// Hand-codec `CreateDeskRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_create_desk_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<CreateDeskRequest, CodecError> {
        super::create_desk_request_from_json(o)
    }

    /// Hand-codec `CreateDeskResponse` encoder.
    #[must_use]
    pub fn hand_create_desk_response_to_json(r: &CreateDeskResponse) -> Value {
        super::create_desk_response_to_json(r)
    }

    /// Hand-codec `UpdateDeskRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_update_desk_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<UpdateDeskRequest, CodecError> {
        super::update_desk_request_from_json(o)
    }

    /// Hand-codec `UpdateDeskResponse` encoder.
    #[must_use]
    pub fn hand_update_desk_response_to_json(r: &UpdateDeskResponse) -> Value {
        super::update_desk_response_to_json(r)
    }

    /// Hand-codec `DeleteDeskRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_delete_desk_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<DeleteDeskRequest, CodecError> {
        super::delete_desk_request_from_json(o)
    }

    /// Hand-codec `DeleteDeskResponse` encoder.
    #[must_use]
    pub fn hand_delete_desk_response_to_json(r: &DeleteDeskResponse) -> Value {
        super::delete_desk_response_to_json(r)
    }

    /// Hand-codec `ListEntitiesRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_entities_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListEntitiesRequest, CodecError> {
        super::list_entities_request_from_json(o)
    }

    /// Hand-codec `ListEntitiesResponse` encoder.
    #[must_use]
    pub fn hand_list_entities_response_to_json(r: &ListEntitiesResponse) -> Value {
        super::list_entities_response_to_json(r)
    }

    /// Hand-codec `CreateEntityRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_create_entity_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<CreateEntityRequest, CodecError> {
        super::create_entity_request_from_json(o)
    }

    /// Hand-codec `CreateEntityResponse` encoder.
    #[must_use]
    pub fn hand_create_entity_response_to_json(r: &CreateEntityResponse) -> Value {
        super::create_entity_response_to_json(r)
    }

    /// Hand-codec `UpdateEntityRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_update_entity_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<UpdateEntityRequest, CodecError> {
        super::update_entity_request_from_json(o)
    }

    /// Hand-codec `UpdateEntityResponse` encoder.
    #[must_use]
    pub fn hand_update_entity_response_to_json(r: &UpdateEntityResponse) -> Value {
        super::update_entity_response_to_json(r)
    }

    /// Hand-codec `DeleteEntityRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_delete_entity_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<DeleteEntityRequest, CodecError> {
        super::delete_entity_request_from_json(o)
    }

    /// Hand-codec `DeleteEntityResponse` encoder.
    #[must_use]
    pub fn hand_delete_entity_response_to_json(r: &DeleteEntityResponse) -> Value {
        super::delete_entity_response_to_json(r)
    }

    /// Hand-codec `ListBooksRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_books_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListBooksRequest, CodecError> {
        super::list_books_request_from_json(o)
    }

    /// Hand-codec `ListBooksResponse` encoder.
    #[must_use]
    pub fn hand_list_books_response_to_json(r: &ListBooksResponse) -> Value {
        super::list_books_response_to_json(r)
    }

    /// Hand-codec `CreateBookRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_create_book_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<CreateBookRequest, CodecError> {
        super::create_book_request_from_json(o)
    }

    /// Hand-codec `CreateBookResponse` encoder.
    #[must_use]
    pub fn hand_create_book_response_to_json(r: &CreateBookResponse) -> Value {
        super::create_book_response_to_json(r)
    }

    /// Hand-codec `UpdateBookRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_update_book_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<UpdateBookRequest, CodecError> {
        super::update_book_request_from_json(o)
    }

    /// Hand-codec `UpdateBookResponse` encoder.
    #[must_use]
    pub fn hand_update_book_response_to_json(r: &UpdateBookResponse) -> Value {
        super::update_book_response_to_json(r)
    }

    /// Hand-codec `DeleteBookRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_delete_book_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<DeleteBookRequest, CodecError> {
        super::delete_book_request_from_json(o)
    }

    /// Hand-codec `DeleteBookResponse` encoder.
    #[must_use]
    pub fn hand_delete_book_response_to_json(r: &DeleteBookResponse) -> Value {
        super::delete_book_response_to_json(r)
    }

    /// Hand-codec `ListInstrumentsRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_list_instruments_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<ListInstrumentsRequest, CodecError> {
        super::list_instruments_request_from_json(o)
    }

    /// Hand-codec `ListInstrumentsResponse` encoder.
    #[must_use]
    pub fn hand_list_instruments_response_to_json(r: &ListInstrumentsResponse) -> Value {
        super::list_instruments_response_to_json(r)
    }

    /// Hand-codec `GetInstrumentRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_get_instrument_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<GetInstrumentRequest, CodecError> {
        super::get_instrument_request_from_json(o)
    }

    /// Hand-codec `GetInstrumentResponse` encoder.
    #[must_use]
    pub fn hand_get_instrument_response_to_json(r: &GetInstrumentResponse) -> Value {
        super::get_instrument_response_to_json(r)
    }

    /// Hand-codec `CreateInstrumentRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_create_instrument_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<CreateInstrumentRequest, CodecError> {
        super::create_instrument_request_from_json(o)
    }

    /// Hand-codec `CreateInstrumentResponse` encoder.
    #[must_use]
    pub fn hand_create_instrument_response_to_json(r: &CreateInstrumentResponse) -> Value {
        super::create_instrument_response_to_json(r)
    }

    /// Hand-codec `UpdateInstrumentRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_update_instrument_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<UpdateInstrumentRequest, CodecError> {
        super::update_instrument_request_from_json(o)
    }

    /// Hand-codec `UpdateInstrumentResponse` encoder.
    #[must_use]
    pub fn hand_update_instrument_response_to_json(r: &UpdateInstrumentResponse) -> Value {
        super::update_instrument_response_to_json(r)
    }

    /// Hand-codec `DeleteInstrumentRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_delete_instrument_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<DeleteInstrumentRequest, CodecError> {
        super::delete_instrument_request_from_json(o)
    }

    /// Hand-codec `DeleteInstrumentResponse` encoder.
    #[must_use]
    pub fn hand_delete_instrument_response_to_json(r: &DeleteInstrumentResponse) -> Value {
        super::delete_instrument_response_to_json(r)
    }

    /// Hand-codec `BuildCurveRequest` decoder.
    ///
    /// # Errors
    /// Propagates the hand codec's [`CodecError`] on a malformed body.
    pub fn hand_build_curve_request_from_json(
        o: &Map<String, Value>,
    ) -> Result<BuildCurveRequest, CodecError> {
        super::build_curve_request_from_json(o)
    }

    /// Hand-codec `CalibratedCurve` encoder.
    #[must_use]
    pub fn hand_calibrated_curve_to_json(c: &CalibratedCurve) -> Value {
        super::calibrated_curve_to_json(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WS Greeks JSON carries the carry-tagged rate-sensitivity arm: an FX line
    /// emits the `fx` arm (and the flat `rho_dom`/`rho_for` stay byte-identical), a
    /// cross-asset line emits the `carry` arm `{discount_rho, carry_rho}` while the
    /// flat projection a legacy client reads still equals the FX-shaped rhos.
    #[test]
    fn greeks_json_carries_the_rate_sensitivity_arm() {
        // FX arm.
        let fx = celnet_proto::Greeks {
            rate_sensitivities: Some(celnet_proto::RateSensitivities::fx(0.5, -0.2)),
            ..Default::default()
        };
        let jf = greeks_to_json(&fx);
        assert_eq!(jf["rho_dom"].as_f64().unwrap().to_bits(), 0.5_f64.to_bits());
        assert_eq!(
            jf["rho_for"].as_f64().unwrap().to_bits(),
            (-0.2_f64).to_bits()
        );
        assert_eq!(
            jf["rate_sensitivities"]["fx"]["rho_dom"].as_f64().unwrap(),
            0.5
        );
        assert_eq!(
            jf["rate_sensitivities"]["fx"]["rho_for"].as_f64().unwrap(),
            -0.2
        );
        assert!(jf["rate_sensitivities"].get("carry").is_none());

        // Carry arm: discount_rho/carry_rho named, flat projection consistent.
        let carry = celnet_proto::Greeks {
            rate_sensitivities: Some(celnet_proto::RateSensitivities {
                sensitivities: Some(celnet_proto::rate_sensitivities::Sensitivities::Carry(
                    celnet_proto::rate_sensitivities::CarryRho {
                        discount_rho: 0.3,
                        carry_rho: 0.2,
                    },
                )),
            }),
            ..Default::default()
        };
        let jc = greeks_to_json(&carry);
        assert_eq!(
            jc["rate_sensitivities"]["carry"]["discount_rho"]
                .as_f64()
                .unwrap(),
            0.3
        );
        assert_eq!(
            jc["rate_sensitivities"]["carry"]["carry_rho"]
                .as_f64()
                .unwrap(),
            0.2
        );
        // Flat projection: rho_dom = discount + carry = 0.5; rho_for = −carry = −0.2.
        assert_eq!(jc["rho_dom"].as_f64().unwrap(), 0.5);
        assert_eq!(jc["rho_for"].as_f64().unwrap(), -0.2);
    }

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

    /// The correlated multi-asset basket decodes from the JSON a browser/Excel
    /// client sends, including the `legs` array (each leg's own market data) and
    /// the row-major `correlations` array — the WS half of the five-surface
    /// api-first parity.
    #[test]
    fn basket_instrument_round_trips_from_json() {
        let v = json!({
            "pair": { "base": "EUR", "quote": "USD" },
            "expiry_years": 1.0,
            "side": 0,
            "basket": {
                "legs": [
                    { "pair": { "base": "EUR", "quote": "USD" },
                      "weight": 0.5, "spot": 1.10, "vol": 0.11, "r_for": 0.015 },
                    { "pair": { "base": "GBP", "quote": "USD" },
                      "weight": 0.5, "spot": 1.27, "vol": 0.13, "r_for": 0.02 }
                ],
                "correlations": [1.0, 0.4, 0.4, 1.0],
                "option_type": 0,
                "strike": 1.18,
                "kind": 2,
                "mc_paths": 8192,
                "mc_replications": 16,
                "mc_steps": 1,
                "mc_seed": 12648430
            }
        });
        let instr = instrument_from_json(&v).expect("decode");
        match instr.product {
            Some(instrument::Product::Basket(b)) => {
                assert_eq!(b.legs.len(), 2);
                assert_eq!(b.legs[0].spot.to_bits(), 1.10_f64.to_bits());
                assert_eq!(b.legs[1].vol.to_bits(), 0.13_f64.to_bits());
                assert_eq!(
                    b.legs[0].underlying.as_ref().unwrap().as_fx().unwrap().base,
                    "EUR"
                );
                assert_eq!(b.correlations, vec![1.0, 0.4, 0.4, 1.0]);
                assert_eq!(b.kind, celnet_proto::BasketKind::WorstOf as i32);
                assert_eq!(b.strike.to_bits(), 1.18_f64.to_bits());
                assert_eq!(b.mc_paths, 8192);
                assert_eq!(b.mc_replications, 16);
            }
            other => panic!("expected a basket product, got {other:?}"),
        }
    }

    /// The W2 linear-book product arms decode from the WS JSON mirror field-for-
    /// field (the same shape as the proto oneof), so the WS transport reaches the
    /// same shared `price_instrument` routing as gRPC.
    #[test]
    fn fx_forward_instrument_round_trips_from_json() {
        let v = json!({
            "pair": { "base": "EUR", "quote": "USD" },
            "expiry_years": 1.0,
            "side": 0,
            "fx_forward": { "contract_rate": 1.25, "notional": 1000000.0, "side": 0 }
        });
        let instr = instrument_from_json(&v).expect("decode");
        match instr.product {
            Some(instrument::Product::FxForward(f)) => {
                assert_eq!(f.contract_rate.to_bits(), 1.25_f64.to_bits());
                assert_eq!(f.notional.to_bits(), 1_000_000.0_f64.to_bits());
                assert_eq!(f.side, celnet_proto::Side::Buy as i32);
            }
            other => panic!("expected an fx_forward product, got {other:?}"),
        }
    }

    #[test]
    fn fx_swap_instrument_round_trips_from_json() {
        let v = json!({
            "pair": { "base": "EUR", "quote": "USD" },
            "expiry_years": 1.0,
            "side": 0,
            "fx_swap": {
                "near": { "contract_rate": 1.25, "notional": 2000000.0, "side": 0 },
                "far": { "contract_rate": 1.25, "notional": 2000000.0, "side": 1 }
            }
        });
        let instr = instrument_from_json(&v).expect("decode");
        match instr.product {
            Some(instrument::Product::FxSwap(s)) => {
                let near = s.near.expect("near leg");
                let far = s.far.expect("far leg");
                assert_eq!(near.side, celnet_proto::Side::Buy as i32);
                assert_eq!(far.side, celnet_proto::Side::Sell as i32);
                assert_eq!(near.notional.to_bits(), 2_000_000.0_f64.to_bits());
            }
            other => panic!("expected an fx_swap product, got {other:?}"),
        }
    }

    #[test]
    fn ndf_instrument_round_trips_from_json() {
        let v = json!({
            "pair": { "base": "USD", "quote": "BRL" },
            "expiry_years": 0.5,
            "side": 0,
            "ndf": {
                "contract_rate": 5.1,
                "notional": 1000000.0,
                "side": 0,
                "fixing": 3,
                "settlement_ccy": "USD"
            }
        });
        let instr = instrument_from_json(&v).expect("decode");
        match instr.product {
            Some(instrument::Product::Ndf(n)) => {
                assert_eq!(n.contract_rate.to_bits(), 5.1_f64.to_bits());
                assert_eq!(n.fixing, celnet_proto::FixingSource::BrlPtax as i32);
                assert_eq!(n.settlement_ccy, "USD");
            }
            other => panic!("expected an ndf product, got {other:?}"),
        }
    }

    /// The perpetual-option arm (proto field 30) decodes from the WS JSON
    /// mirror field-for-field. A perpetual has no expiry, so the instrument
    /// carries the exact proto3 zero `expiry_years: 0.0` (the shape the shared
    /// validity seam enforces).
    #[test]
    fn perpetual_option_instrument_round_trips_from_json() {
        let v = json!({
            "pair": { "base": "EUR", "quote": "USD" },
            "expiry_years": 0.0,
            "side": 0,
            "perpetual_option": {
                "option_type": 1,
                "strike": 1.05,
                "notional": 10000000.0
            }
        });
        let instr = instrument_from_json(&v).expect("decode");
        assert_eq!(instr.expiry_years.to_bits(), 0.0_f64.to_bits());
        match instr.product {
            Some(instrument::Product::PerpetualOption(p)) => {
                assert_eq!(p.option_type, celnet_proto::OptionType::Put as i32);
                assert_eq!(p.strike.to_bits(), 1.05_f64.to_bits());
                assert_eq!(p.notional.to_bits(), 10_000_000.0_f64.to_bits());
            }
            other => panic!("expected a perpetual_option product, got {other:?}"),
        }
    }

    /// The listed-future-option arm (proto field 31) decodes from the WS JSON
    /// mirror field-for-field, including the nested `future_symbol` contract
    /// identity and the `margining` enum number.
    #[test]
    fn listed_future_option_instrument_round_trips_from_json() {
        let v = json!({
            "underlying": { "commodity": { "symbol": { "ticker": "BRENT" }, "currency": "USD" } },
            "expiry_years": 0.5,
            "side": 0,
            "listed_future_option": {
                "future_symbol": { "ticker": "BRN-DEC26", "venue": "IFEU" },
                "future_expiry_years": 0.55,
                "option_type": 0,
                "strike": 85.0,
                "notional": 1000.0,
                "margining": 1
            }
        });
        let instr = instrument_from_json(&v).expect("decode");
        match instr.product {
            Some(instrument::Product::ListedFutureOption(o)) => {
                let symbol = o.future_symbol.expect("future_symbol");
                assert_eq!(symbol.ticker, "BRN-DEC26");
                assert_eq!(symbol.venue, "IFEU");
                assert_eq!(o.future_expiry_years.to_bits(), 0.55_f64.to_bits());
                assert_eq!(o.option_type, celnet_proto::OptionType::Call as i32);
                assert_eq!(o.strike.to_bits(), 85.0_f64.to_bits());
                assert_eq!(o.notional.to_bits(), 1_000.0_f64.to_bits());
                assert_eq!(o.margining, celnet_proto::Margining::FuturesStyle as i32);
            }
            other => panic!("expected a listed_future_option product, got {other:?}"),
        }
    }

    /// The instrument-underlying decode treats the richer cross-asset `underlying`
    /// oneof as AUTHORITATIVE over the legacy FX `pair` projection when BOTH are
    /// present — the exact shape the production GUI/Excel encoders emit (the FX
    /// `pair` projection rides beside `underlying` so the FX-keyed surfaces stay
    /// total). A digital-asset frame must decode to the `DigitalAsset` arm, not be
    /// mis-decoded as FX from its `{base,quote}` projection — the root-cause fix for
    /// the cross-asset WS routing defect (without it, an INVERSE_COIN frame would
    /// price the LINEAR value four orders of magnitude away). A pure-FX legacy frame
    /// (`pair` only) still decodes to the FX arm, byte-identically.
    #[test]
    fn underlying_oneof_takes_precedence_over_legacy_pair_projection() {
        // Both keys present (the production cross-asset frame): `underlying` wins.
        let both = json!({
            "pair": { "base": "BTC", "quote": "USD" },
            "underlying": { "digital_asset": { "base": "BTC", "quote": "USD" } },
            "expiry_years": 0.5,
            "side": 0,
            "settlement_style": celnet_proto::SettlementStyle::InverseCoin as i32,
            "vanilla": { "option_type": 0, "strike": { "strike": 31000.0 } }
        });
        let instr = instrument_from_json(&both).expect("decode");
        let underlying = instr.underlying.expect("an underlying is decoded");
        assert!(
            underlying.as_digital_asset().is_some(),
            "the richer `underlying` oneof must win over the legacy `pair` projection, \
             got {underlying:?}"
        );
        assert!(
            underlying.as_fx().is_none(),
            "a cross-asset frame must NOT be mis-decoded as FX from its `pair` projection"
        );
        // The INVERSE_COIN settlement style rides through to select the coin payoff.
        assert_eq!(
            instr.settlement_style,
            celnet_proto::SettlementStyle::InverseCoin as i32
        );

        // Pure-FX legacy frame (`pair` only): still the FX arm, byte-identically.
        let fx_only = json!({
            "pair": { "base": "EUR", "quote": "USD" },
            "expiry_years": 1.0,
            "side": 0,
            "vanilla": { "option_type": 0, "strike": { "strike": 1.1 } }
        });
        let fx_instr = instrument_from_json(&fx_only).expect("decode");
        let fx_underlying = fx_instr.underlying.expect("an underlying is decoded");
        let pair = fx_underlying
            .as_fx()
            .expect("a pure-`pair` frame decodes to FX");
        assert_eq!(pair.base, "EUR");
        assert_eq!(pair.quote, "USD");
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

    /// The Wave-3 products (TARF / pivot TRA / accumulator / lookback) decode
    /// from a browser client's JSON into the correct `product` oneof arms —
    /// including the nested `FixingSchedule` body on the TARF/pivot/accumulator
    /// and the MC knobs.
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

        // The pivot TRA (arm 32): the TARF body plus the distinct `pivot` level.
        let piv = instrument_from_json(&base(json!({
            "pivot": {
                "option_type": 0, "strike": 1.08, "pivot": 1.13, "target": 0.20,
                "leverage": 2.5, "redemption": 1,
                "schedule": { "fixing_years": [0.25, 0.5, 0.75, 1.0], "fixing_notional": 1.0 },
                "mc_pairs": 30000, "mc_seed": 1707
            }
        })))
        .expect("decode pivot");
        match piv.product {
            Some(instrument::Product::Pivot(p)) => {
                assert_eq!(p.option_type, celnet_proto::OptionType::Call as i32);
                assert_eq!(p.strike.to_bits(), 1.08_f64.to_bits());
                assert_eq!(p.pivot.to_bits(), 1.13_f64.to_bits());
                assert_eq!(p.target.to_bits(), 0.20_f64.to_bits());
                assert_eq!(p.leverage.to_bits(), 2.5_f64.to_bits());
                assert_eq!(
                    p.redemption,
                    celnet_proto::TarfRedemption::CappedGain as i32
                );
                let s = p.schedule.expect("pivot carries a schedule");
                assert_eq!(s.fixing_years.len(), 4);
                assert_eq!(s.fixing_notional.to_bits(), 1.0_f64.to_bits());
                assert_eq!(p.mc_pairs, 30_000);
                assert_eq!(p.mc_seed, 1707);
            }
            other => panic!("expected pivot, got {other:?}"),
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

    /// A `price_rates` request decodes from the browser JSON shape into the proto
    /// `RatesPriceRequest`, and the decoded curve + instrument price through the
    /// engine — proving the WS mirror reaches the SAME rates path as gRPC.
    #[test]
    fn rates_price_request_decodes_and_prices() {
        let body = json!({
            "request_id": 9,
            "curve_set": {
                "currency": "USD",
                "reference_date": { "year": 2026, "month": 6, "day": 25 },
                "ois_pillars": [
                    { "tenor": { "years": 1 }, "par_rate": 0.0432 },
                    { "tenor": { "years": 2 }, "par_rate": 0.0418 },
                    { "tenor": { "years": 5 }, "par_rate": 0.0405 }
                ]
            },
            "instrument": {
                "ois": { "tenor_years": 5, "fixed_rate": 0.0405, "notional": 100000000.0, "side": 1 }
            },
            "correlation_id": 7
        });
        let o = body.as_object().unwrap();
        let req = rates_price_request_from_json(o).expect("decodes");
        assert_eq!(req.request_id, 9);
        assert_eq!(req.correlation_id, Some(7));
        let curve = req.curve_set.as_ref().unwrap();
        assert_eq!(curve.currency, "USD");
        assert_eq!(curve.ois_pillars.len(), 3);
        // Each whole-year pillar decoded into the `Years` arm.
        use celnet_proto::pillar_tenor::Point;
        assert_eq!(
            curve.ois_pillars[0].tenor.as_ref().unwrap().point,
            Some(Point::Years(1))
        );
        assert_eq!(
            curve.ois_pillars[2].tenor.as_ref().unwrap().point,
            Some(Point::Years(5))
        );

        // The decoded request prices through the engine: a 5y receive-fixed swap
        // (side=1=SELL) at the 5y par rate is ~par, so PV ~ 0.
        let result = crate::rates_pricing::price_rates(&req).expect("prices");
        assert!(
            (result.par_rate - 0.0405).abs() < 1e-6,
            "par {}",
            result.par_rate
        );
        assert!(result.pv.abs() < 1.0, "near-par PV {}", result.pv);
        assert_eq!(result.key_rate_ladder.len(), 3);

        // The response re-encodes to the browser shape.
        let resp = celnet_proto::RatesPriceResponse {
            request_id: req.request_id,
            result: Some(result),
            correlation_id: req.correlation_id,
        };
        let v = rates_price_response_to_json(&resp);
        assert_eq!(v["request_id"], json!(9));
        assert_eq!(v["correlation_id"], json!(7));
        assert!(v["result"]["par_rate"].as_f64().unwrap() > 0.0);
        assert_eq!(v["result"]["key_rate_ladder"].as_array().unwrap().len(), 3);
    }

    /// A `price_xva` request decodes from the browser JSON shape into the proto
    /// `PriceXvaRequest`, prices through the SAME `celnet-xva` engine the gRPC edge
    /// uses, and the `PriceXvaResponse` re-encodes to the browser shape — the WS
    /// mirror is a second encoding of the one contract, round-tripping field-for-field.
    #[test]
    fn xva_price_request_decodes_prices_and_response_round_trips() {
        let body = json!({
            "request_id": 11,
            "trades": [
                { "option_type": 0, "strike": 1.10, "expiry_years": 1.0, "vol": 0.12, "notional": 1.0 },
                { "option_type": 1, "strike": 1.05, "expiry_years": 1.5, "vol": 0.14, "notional": -1.0 }
            ],
            "r_dom": 0.03,
            "r_for": 0.01,
            "spot0": 1.10,
            "sigma": 0.13,
            "paths": 2048,
            "seed": 11259375,
            "exposure_steps": 8,
            "counterparty": { "pillar_times": [0.5, 2.0], "hazard_rates": [0.02, 0.05] },
            "own": { "pillar_times": [], "hazard_rates": [0.015] },
            "lgd_counterparty": 0.6,
            "lgd_own": 0.55,
            "funding_spread": 0.008,
            "correlation_id": 5
        });
        let o = body.as_object().unwrap();
        let req = price_xva_request_from_json(o).expect("decodes");

        // Header + netting set decoded field-for-field.
        assert_eq!(req.request_id, 11);
        assert_eq!(req.correlation_id, Some(5));
        assert_eq!(req.trades.len(), 2);
        assert_eq!(
            req.trades[0].option_type,
            celnet_proto::OptionType::Call as i32
        );
        assert_eq!(
            req.trades[1].option_type,
            celnet_proto::OptionType::Put as i32
        );
        assert_eq!(req.trades[1].notional, -1.0);
        assert_eq!(req.paths, 2048);
        assert_eq!(req.exposure_steps, 8);
        // Piecewise counterparty curve vs flat own curve decoded distinctly.
        let cpty = req.counterparty.as_ref().unwrap();
        assert_eq!(cpty.pillar_times, vec![0.5, 2.0]);
        assert_eq!(cpty.hazard_rates, vec![0.02, 0.05]);
        let own = req.own.as_ref().unwrap();
        assert!(own.pillar_times.is_empty());
        assert_eq!(own.hazard_rates, vec![0.015]);

        // The decoded request prices through the engine (same path as gRPC).
        let result = crate::xva_pricing::price_xva(&req).expect("prices");
        assert!(result.cva > 0.0, "CVA {}", result.cva);
        assert!(result.dva > 0.0, "DVA {}", result.dva);
        assert!(
            (result.total_adjustment - (result.cva - result.dva + result.fva)).abs() < 1e-15,
            "total identity"
        );

        // The response re-encodes to the browser shape, field-for-field.
        let resp = celnet_proto::PriceXvaResponse {
            request_id: req.request_id,
            result: Some(result),
            correlation_id: req.correlation_id,
        };
        let v = price_xva_response_to_json(&resp);
        assert_eq!(v["request_id"], json!(11));
        assert_eq!(v["correlation_id"], json!(5));
        assert_eq!(v["result"]["cva"].as_f64().unwrap(), result.cva);
        assert_eq!(v["result"]["dva"].as_f64().unwrap(), result.dva);
        assert_eq!(v["result"]["fva"].as_f64().unwrap(), result.fva);
        assert_eq!(
            v["result"]["total_adjustment"].as_f64().unwrap(),
            result.total_adjustment
        );
    }

    /// The `build_curve` frame decodes the registry-referenced pillar set the GUI /
    /// Excel client sends, and a `CalibratedCurve` re-encodes to the browser shape —
    /// the WS mirror of `AuthService::BuildCurve`, field-for-field with the GUI codec.
    #[test]
    fn build_curve_request_decodes_and_calibrated_curve_encodes() {
        let body = json!({
            "request_id": "curve-001",
            "currency": "USD",
            "reference_date": { "year": 2026, "month": 6, "day": 25 },
            "pillars": [
                { "instrument_id": "usd-depo-3m", "quote": 0.0431 },
                { "instrument_id": "usd-irs-10y", "quote": 0.0418 }
            ],
            "date_pillars": [
                { "maturity_date": { "year": 2027, "month": 12, "day": 31 }, "quote": 0.0415 }
            ],
            "session_token": "tok-123"
        });
        let req = build_curve_request_from_json(body.as_object().unwrap()).expect("decodes");
        assert_eq!(req.request_id, "curve-001");
        assert_eq!(req.currency, "USD");
        assert_eq!(req.session_token, "tok-123");
        let ref_date = req.reference_date.as_ref().unwrap();
        assert_eq!((ref_date.year, ref_date.month, ref_date.day), (2026, 6, 25));
        assert_eq!(req.pillars.len(), 2);
        assert_eq!(req.pillars[0].instrument_id, "usd-depo-3m");
        assert!((req.pillars[1].quote - 0.0418).abs() < 1e-12);
        // The date-anchored pillar decodes alongside the instrument pillars.
        assert_eq!(req.date_pillars.len(), 1);
        let dp_date = req.date_pillars[0].maturity_date.as_ref().unwrap();
        assert_eq!((dp_date.year, dp_date.month, dp_date.day), (2027, 12, 31));
        assert!((req.date_pillars[0].quote - 0.0415).abs() < 1e-12);

        // A bootstrapped result re-encodes to the snake_case `calibrated_curve` frame.
        let curve = CalibratedCurve {
            request_id: req.request_id.clone(),
            currency: req.currency.clone(),
            reference_date: req.reference_date,
            points: vec![
                CalibratedCurvePoint {
                    instrument_id: "usd-depo-3m".to_owned(),
                    time_years: 0.2521,
                    discount_factor: 0.98912,
                    zero_rate: 0.04318,
                    label: String::new(),
                },
                CalibratedCurvePoint {
                    instrument_id: String::new(),
                    time_years: 1.5151,
                    discount_factor: 0.93827,
                    zero_rate: 0.04150,
                    label: "Date 2027-12-31".to_owned(),
                },
                CalibratedCurvePoint {
                    instrument_id: "usd-irs-10y".to_owned(),
                    time_years: 10.0,
                    discount_factor: 0.6612,
                    zero_rate: 0.04134,
                    label: String::new(),
                },
            ],
        };
        let v = calibrated_curve_to_json(&curve);
        assert_eq!(v["request_id"], json!("curve-001"));
        assert_eq!(v["currency"], json!("USD"));
        assert_eq!(v["reference_date"]["year"], json!(2026));
        assert_eq!(v["points"].as_array().unwrap().len(), 3);
        assert_eq!(v["points"][0]["instrument_id"], json!("usd-depo-3m"));
        // The date pillar carries its display label and an empty instrument id.
        assert_eq!(v["points"][1]["label"], json!("Date 2027-12-31"));
        assert_eq!(v["points"][1]["instrument_id"], json!(""));
        assert!((v["points"][2]["discount_factor"].as_f64().unwrap() - 0.6612).abs() < 1e-12);
        assert!((v["points"][2]["time_years"].as_f64().unwrap() - 10.0).abs() < 1e-12);
    }

    /// A `build_curve` body with neither pillar array decodes to two empty ladders
    /// (the handler enforces non-empty); a non-array `pillars` is a contract error.
    #[test]
    fn build_curve_request_pillar_arrays_are_optional_but_typed() {
        let empty = json!({
            "request_id": "curve-002",
            "currency": "USD",
            "reference_date": { "year": 2026, "month": 6, "day": 25 },
            "session_token": "tok-123"
        });
        let req = build_curve_request_from_json(empty.as_object().unwrap())
            .expect("missing pillar arrays default to empty");
        assert!(req.pillars.is_empty());
        assert!(req.date_pillars.is_empty());

        let malformed = json!({
            "request_id": "curve-003",
            "currency": "USD",
            "reference_date": { "year": 2026, "month": 6, "day": 25 },
            "pillars": "not-an-array",
            "session_token": "tok-123"
        });
        let err = build_curve_request_from_json(malformed.as_object().unwrap())
            .expect_err("a non-array pillars must error");
        assert!(err.to_string().contains("pillars"));
    }

    /// A curve mixing all three `PillarTenor` arms — a whole-year `years`, a
    /// `months` tenor, and an explicit `maturity_date` broken date — decodes into
    /// the matching oneof variants, then re-encodes to the same JSON shape
    /// (round-trip of the new wire form), and prices without error.
    #[test]
    fn rates_price_request_decodes_mixed_pillar_arms() {
        use celnet_proto::pillar_tenor::Point;
        let body = json!({
            "request_id": 11,
            "curve_set": {
                "currency": "USD",
                "reference_date": { "year": 2026, "month": 6, "day": 25 },
                "ois_pillars": [
                    { "tenor": { "years": 1 }, "par_rate": 0.0432 },
                    { "tenor": { "months": 18 }, "par_rate": 0.0418 },
                    { "tenor": { "maturity_date": { "year": 2031, "month": 6, "day": 30 } }, "par_rate": 0.0405 }
                ]
            },
            "instrument": {
                "ois": { "tenor_years": 2, "fixed_rate": 0.041, "notional": 100000000.0, "side": 1 }
            }
        });
        let req = rates_price_request_from_json(body.as_object().unwrap()).expect("decodes");
        let pillars = &req.curve_set.as_ref().unwrap().ois_pillars;
        assert_eq!(
            pillars[0].tenor.as_ref().unwrap().point,
            Some(Point::Years(1))
        );
        assert_eq!(
            pillars[1].tenor.as_ref().unwrap().point,
            Some(Point::Months(18))
        );
        assert_eq!(
            pillars[2].tenor.as_ref().unwrap().point,
            Some(Point::MaturityDate(celnet_proto::BrokenDate {
                year: 2031,
                month: 6,
                day: 30,
            }))
        );

        // The curve re-encodes to the same arm shapes (encode/decode symmetry).
        let back = curve_set_to_json(req.curve_set.as_ref().unwrap());
        assert_eq!(back["ois_pillars"][1]["tenor"]["months"], json!(18));
        assert_eq!(
            back["ois_pillars"][2]["tenor"]["maturity_date"]["year"],
            json!(2031)
        );

        // And it prices to a finite par rate end to end.
        let priced = crate::rates_pricing::price_rates(&req).expect("prices");
        assert!(priced.par_rate.is_finite() && priced.par_rate > 0.0);
    }

    /// An `aggregate_rates_risk` request decodes from the browser JSON shape into
    /// the proto `AggregateRatesRiskRequest`: the shared curve/instrument codecs
    /// rebuild the market + economics, positions carry their `(entity, book)` cell,
    /// the optional scope threads through, and `session_token`/`principal` survive
    /// for the gRPC handler's entitlement check.
    #[test]
    fn aggregate_rates_risk_request_decodes() {
        let body = json!({
            "curve_set": {
                "currency": "USD",
                "reference_date": { "year": 2026, "month": 6, "day": 25 },
                "ois_pillars": [
                    { "tenor": { "years": 1 }, "par_rate": 0.0432 },
                    { "tenor": { "years": 5 }, "par_rate": 0.0405 }
                ]
            },
            "positions": [
                {
                    "position_id": 11,
                    "entity": 1,
                    "book": 100,
                    "instrument": {
                        "ois": { "tenor_years": 5, "fixed_rate": 0.0405, "notional": 1.0e8, "side": 1 }
                    }
                }
            ],
            "scope": { "entity": 1, "book": 100 },
            "session_token": "sess-xyz",
            "correlation_id": 42
        });
        let o = body.as_object().unwrap();
        let req = aggregate_rates_risk_request_from_json(o).expect("decodes");

        assert_eq!(req.correlation_id, Some(42));
        assert_eq!(req.session_token.as_deref(), Some("sess-xyz"));
        let curve = req.curve_set.as_ref().unwrap();
        assert_eq!(curve.currency, "USD");
        assert_eq!(curve.ois_pillars.len(), 2);
        assert_eq!(req.positions.len(), 1);
        let p = &req.positions[0];
        assert_eq!(p.position_id, 11);
        assert_eq!(p.entity, 1);
        assert_eq!(p.book, 100);
        assert!(p.instrument.is_some());
        let scope = req.scope.as_ref().unwrap();
        assert_eq!(scope.entity, Some(1));
        assert_eq!(scope.book, Some(100));
        assert_eq!(scope.ccy, None);

        // The decoded request rolls up through the SAME edge the gRPC handler calls,
        // proving the WS mirror reaches the identical rates-risk path.
        let resp = crate::services::rates_risk::aggregate::single_node_aggregate(&req)
            .expect("aggregates");
        assert_eq!(resp.nodes.len(), 1);
        assert_eq!(resp.nodes[0].ccy, "USD");

        // The response re-encodes to the browser shape: per-ccy node + tenor ladder.
        let v = aggregate_rates_risk_response_to_json(&resp);
        assert_eq!(v["correlation_id"], json!(42));
        let node = &v["nodes"][0];
        assert_eq!(node["ccy"], json!("USD"));
        assert!(node["net_dv01"].is_number());
        let ladder = node["key_rate_ladder"].as_array().unwrap();
        assert_eq!(ladder.len(), 2);
        assert_eq!(ladder[0]["tenor_years"], json!(1));
        assert!(ladder[0]["dv01"].is_number());
    }

    // ---- dealer-quoting desk + rates Book/List + notification codecs ----------

    fn ois_instrument_json() -> Value {
        json!({ "ois": { "tenor_years": 5, "fixed_rate": 0.0405, "notional": 25000000.0, "side": 0 } })
    }

    fn curve_json() -> Value {
        json!({
            "currency": "USD",
            "reference_date": { "year": 2026, "month": 6, "day": 25 },
            "ois_pillars": [ { "tenor": { "years": 1 }, "par_rate": 0.0432 }, { "tenor": { "years": 5 }, "par_rate": 0.0405 } ]
        })
    }

    /// `book_rates_position` decodes its position + principal + correlation; the
    /// response re-encodes the stored position.
    #[test]
    fn book_rates_position_round_trip() {
        let o = json!({
            "type": "book_rates_position",
            "session_token": "tok",
            "position": { "position_id": 0, "entity": 1, "book": 10, "instrument": ois_instrument_json() },
            "principal": { "grant_all": true },
            "correlation_id": "corr-7"
        });
        let req = book_rates_position_request_from_json(o.as_object().unwrap()).expect("decodes");
        assert_eq!(req.session_token.as_deref(), Some("tok"));
        assert_eq!(req.correlation_id.as_deref(), Some("corr-7"));
        let pos = req.position.unwrap();
        assert_eq!(pos.entity, 1);
        assert_eq!(pos.book, 10);
        let v = book_rates_position_response_to_json(&BookRatesPositionResponse {
            position: Some(pos),
        });
        assert_eq!(v["position"]["book"], json!(10));
        assert!(v["position"]["instrument"]["ois"]["tenor_years"].is_number());
    }

    /// `list_rates_positions` decodes its scope/principal; the response encodes the
    /// positions array.
    #[test]
    fn list_rates_positions_round_trip() {
        let o = json!({
            "type": "list_rates_positions",
            "scope": { "book": 10 },
            "principal": { "grant_all": true },
            "correlation_id": "c"
        });
        let req = list_rates_positions_request_from_json(o.as_object().unwrap()).expect("decodes");
        assert_eq!(req.scope.unwrap().book, Some(10));
        let v = list_rates_positions_response_to_json(&ListRatesPositionsResponse {
            positions: vec![RatesPosition {
                position_id: 3,
                entity: 1,
                book: 10,
                instrument: None,
            }],
        });
        assert_eq!(v["positions"].as_array().unwrap().len(), 1);
        assert_eq!(v["positions"][0]["position_id"], json!(3));
    }

    /// `submit_desk_request` decodes the full RFQ shape including the shared rates
    /// instrument + curve and the optional principal.
    #[test]
    fn submit_desk_request_decodes() {
        let o = json!({
            "type": "submit_desk_request",
            "kind": 1,
            "counterparty": "cp-bank",
            "desk": "g10",
            "instrument": ois_instrument_json(),
            "curve_set": curve_json(),
            "side": 0,
            "notional": 25000000.0,
            "ttl_ms": 30000,
            "principal": { "grant_all": true },
            "correlation_id": "c-1"
        });
        let req = submit_desk_request_from_json(o.as_object().unwrap()).expect("decodes");
        assert_eq!(req.kind, 1);
        assert_eq!(req.desk, "g10");
        assert_eq!(req.ttl_ms, 30000);
        assert_eq!(req.correlation_id.as_deref(), Some("c-1"));
        assert!(req.instrument.is_some() && req.curve_set.is_some());
    }

    /// `respond_desk_request` decodes the `quote` and `reject` oneof arms, and
    /// rejects setting both.
    #[test]
    fn respond_desk_request_oneof_decodes() {
        let q = json!({
            "type": "respond_desk_request",
            "request_id": "desk-req-1",
            "quote": { "price": 0.0411, "notional": 25000000.0, "valid_for_ms": 30000, "trader": "alice" }
        });
        let req = respond_desk_request_from_json(q.as_object().unwrap()).expect("decodes quote");
        assert!(matches!(req.response, Some(RespondArm::Quote(_))));

        let r = json!({
            "type": "respond_desk_request",
            "request_id": "desk-req-1",
            "reject": { "reason": "off-market" }
        });
        let req = respond_desk_request_from_json(r.as_object().unwrap()).expect("decodes reject");
        assert!(matches!(req.response, Some(RespondArm::Reject(_))));

        let both = json!({
            "type": "respond_desk_request", "request_id": "x",
            "quote": { "price": 0.0, "notional": 1.0, "valid_for_ms": 0, "trader": "" },
            "reject": { "reason": "no" }
        });
        assert!(respond_desk_request_from_json(both.as_object().unwrap()).is_err());
    }

    /// `accept_desk_quote` / `list_desk_requests` / `list_deals` decode their reads.
    #[test]
    fn desk_reads_decode() {
        let a = json!({ "type": "accept_desk_quote", "request_id": "desk-req-1", "principal": { "grant_all": true } });
        let req = accept_desk_quote_from_json(a.as_object().unwrap()).expect("decodes");
        assert_eq!(req.request_id, "desk-req-1");

        let l =
            json!({ "type": "list_desk_requests", "scope": { "states": [1, 2], "desk": "g10" } });
        let req = list_desk_requests_from_json(l.as_object().unwrap()).expect("decodes");
        let scope = req.scope.unwrap();
        assert_eq!(scope.states, vec![1, 2]);
        assert_eq!(scope.desk.as_deref(), Some("g10"));

        let d = json!({ "type": "list_deals", "scope": { "desk": "g10" } });
        let req = list_deals_from_json(d.as_object().unwrap()).expect("decodes");
        assert_eq!(req.scope.unwrap().desk.as_deref(), Some("g10"));
    }

    /// A `Notification` encodes as a `{"type":"notification", …}` push frame.
    #[test]
    fn notification_encodes_as_push_frame() {
        let n = Notification {
            notification_id: "notif-1".to_owned(),
            kind: celnet_proto::NotificationKind::RfqReceived as i32,
            at_nanos: 42,
            request_id: Some("desk-req-1".to_owned()),
            desk: "g10".to_owned(),
            counterparty: "cp".to_owned(),
            request_kind: celnet_proto::DeskRequestKind::Rfq as i32,
            headline: "New RFQ".to_owned(),
            detail: Some("needs pricing".to_owned()),
        };
        let v = notification_to_json(&n);
        assert_eq!(v["type"], json!("notification"));
        assert_eq!(v["notification_id"], json!("notif-1"));
        assert_eq!(v["desk"], json!("g10"));
        assert_eq!(v["request_id"], json!("desk-req-1"));
        assert_eq!(v["detail"], json!("needs pricing"));
    }

    /// The `subscribe_notifications` frame decodes its desk scope + principal.
    #[test]
    fn stream_notifications_request_decodes() {
        let o = json!({
            "type": "subscribe_notifications",
            "session_token": "tok",
            "scope": { "desks": ["g10", "em"] },
            "principal": { "grant_all": true },
            "correlation_id": "c"
        });
        let req = stream_notifications_request_from_json(o.as_object().unwrap()).expect("decodes");
        assert_eq!(req.session_token.as_deref(), Some("tok"));
        assert_eq!(req.scope.unwrap().desks, vec!["g10", "em"]);
        assert_eq!(req.correlation_id.as_deref(), Some("c"));
    }

    /// `get_user_capabilities` decodes its target id; the response encodes the
    /// overlay + effective set as three capability arrays.
    #[test]
    fn get_user_capabilities_round_trip() {
        let o = json!({
            "type": "get_user_capabilities",
            "session_token": "tok",
            "id": "u-1",
            "correlation_id": 9
        });
        let req = get_user_capabilities_request_from_json(o.as_object().unwrap()).expect("decodes");
        assert_eq!(req.session_token, "tok");
        assert_eq!(req.id, "u-1");
        assert_eq!(req.correlation_id, Some(9));
        let v = get_user_capabilities_response_to_json(&GetUserCapabilitiesResponse {
            grants: vec![CapabilityDesc {
                action: "administer".into(),
                asset: "fx_options".into(),
            }],
            denies: vec![CapabilityDesc {
                action: "execute".into(),
                asset: "fixed_income".into(),
            }],
            effective: vec![CapabilityDesc {
                action: "view".into(),
                asset: "fx_options".into(),
            }],
            correlation_id: Some(9),
        });
        assert_eq!(v["grants"][0]["action"], json!("administer"));
        assert_eq!(v["denies"][0]["asset"], json!("fixed_income"));
        assert_eq!(v["effective"][0]["action"], json!("view"));
        assert_eq!(v["correlation_id"], json!(9));
    }

    /// `set_user_capabilities` decodes both overlay arrays (absent ⇒ empty); the
    /// response mirrors the get shape. This keeps the decoder + router in lockstep
    /// for the new admin verb.
    #[test]
    fn set_user_capabilities_round_trip() {
        let o = json!({
            "type": "set_user_capabilities",
            "session_token": "tok",
            "id": "u-2",
            "grants": [{ "action": "book", "asset": "fixed_income" }],
            "denies": [],
            "correlation_id": 4
        });
        let req = set_user_capabilities_request_from_json(o.as_object().unwrap()).expect("decodes");
        assert_eq!(req.id, "u-2");
        assert_eq!(req.grants.len(), 1);
        assert_eq!(req.grants[0].action, "book");
        assert!(req.denies.is_empty());
        assert_eq!(req.correlation_id, Some(4));

        // A missing overlay array decodes to empty, never an error.
        let bare = json!({ "type": "set_user_capabilities", "session_token": "t", "id": "u" });
        let req2 =
            set_user_capabilities_request_from_json(bare.as_object().unwrap()).expect("decodes");
        assert!(req2.grants.is_empty() && req2.denies.is_empty());

        // A malformed capability entry (missing `asset`) is rejected at decode.
        let bad = json!({
            "type": "set_user_capabilities", "session_token": "t", "id": "u",
            "grants": [{ "action": "book" }]
        });
        assert!(set_user_capabilities_request_from_json(bad.as_object().unwrap()).is_err());

        let v = set_user_capabilities_response_to_json(&SetUserCapabilitiesResponse {
            grants: req.grants.clone(),
            denies: vec![],
            effective: req.grants,
            correlation_id: Some(4),
        });
        assert_eq!(v["grants"][0]["asset"], json!("fixed_income"));
        assert_eq!(v["effective"][0]["action"], json!("book"));
    }

    /// The fixed-income acceptor kinds round-trip through the FIX-connection codec:
    /// the spec decoder reads the integer enum tag (1 = FI-quote, 2 = FI-stream) and
    /// the descriptor encoder emits it, so the decoder/router stay in lockstep with
    /// the proto enum's new variants.
    #[test]
    fn fix_acceptor_fi_kinds_round_trip_through_the_codec() {
        for tag in [
            celnet_proto::FixAcceptorKind::Options as i32,
            celnet_proto::FixAcceptorKind::FixedIncomeQuote as i32,
            celnet_proto::FixAcceptorKind::FixedIncomeStream as i32,
        ] {
            let spec_json = json!({
                "name": "Bank A — FI",
                "kind": tag,
                "bind_addr": "127.0.0.1:9099",
                "sender_comp_id": "CELNET",
                "target_comp_id": "CELNET-CPTY",
                "enabled": true,
                "desk": "g10",
            });
            let spec = fix_connection_spec_from_json(&spec_json).expect("spec decodes");
            assert_eq!(spec.kind, tag, "the wire enum tag survives the decode");

            let desc = FixConnectionDesc {
                id: "fi-1".into(),
                name: spec.name.clone(),
                kind: tag,
                bind_addr: spec.bind_addr.clone(),
                sender_comp_id: spec.sender_comp_id.clone(),
                target_comp_id: spec.target_comp_id.clone(),
                enabled: true,
                running: false,
                bound_addr: String::new(),
                desk: spec.desk.clone(),
            };
            let v = fix_connection_desc_to_json(&desc);
            assert_eq!(v["kind"], json!(tag), "the descriptor re-emits the tag");
        }
    }

    /// Each instrument family round-trips byte-identically through the WS codec
    /// (`InstrumentDefDesc` → JSON → `InstrumentDefDesc`), and the family oneof is
    /// carried under its variant-token key.
    #[test]
    fn instrument_families_round_trip_through_codec() {
        let families = vec![
            InstrumentDefinition::Ois(celnet_proto::OisDef {
                tenor: "2Y".into(),
                index: "sofr".into(),
                fixed_frequency: "annual".into(),
                fixed_day_count: "act_360".into(),
                float_day_count: "act_360".into(),
                business_day_convention: "modified_following".into(),
                calendars: vec!["united_states".into()],
                spot_lag_days: 2,
            }),
            InstrumentDefinition::Bond(celnet_proto::BondDef {
                issuer: "US Treasury".into(),
                coupon_rate: 0.045,
                coupon_type: "fixed".into(),
                coupon_frequency: "semi_annual".into(),
                day_count: "act_act".into(),
                issue_date: Some(celnet_proto::BrokenDate {
                    year: 2026,
                    month: 1,
                    day: 31,
                }),
                dated_date: None,
                first_coupon_date: None,
                maturity_date: Some(celnet_proto::BrokenDate {
                    year: 2028,
                    month: 1,
                    day: 31,
                }),
                redemption: 100.0,
                calendars: vec!["united_states".into()],
            }),
        ];
        for (i, fam) in families.into_iter().enumerate() {
            let desc = InstrumentDefDesc {
                instrument_id: format!("x-{i}"),
                name: format!("X {i}"),
                description: "round-trip".into(),
                currency: "USD".into(),
                external_ids: vec![celnet_proto::ExternalId {
                    scheme: "ticker".into(),
                    value: format!("X{i}"),
                }],
                definition: Some(fam),
            };
            let v = instrument_def_to_json(&desc);
            let back = instrument_def_from_json(&v).expect("decodes");
            assert_eq!(desc, back, "family at index {i}");
        }
    }

    /// A `create_instrument` request with no family sub-object decodes with an
    /// unset `definition` (the service then rejects it with `invalid_argument`).
    #[test]
    fn instrument_without_family_decodes_unset() {
        let req = json!({
            "session_token": "t",
            "instrument": { "name": "X", "currency": "USD" },
        });
        let o = req.as_object().unwrap();
        let decoded = create_instrument_request_from_json(o).expect("decodes");
        assert!(decoded.instrument.unwrap().definition.is_none());
    }
}
