//! Descriptor-driven WS JSON encoder (arch item G — `ws-codec-from-proto`).
//!
//! This is the **generated** side of the deferred unary codec swap: it encodes a
//! proto message body to the WS mirror's JSON purely from the descriptor-derived
//! field tables in [`celnet_proto::wire_contract`] plus the curated
//! [override table](super::codec_overrides) — it never hardcodes a JSON key or a
//! field list, and it is an **independent code path** that does **not** call the
//! hand codec ([`super::codec`]). The differential harness
//! (`tests/ws_codec_differential.rs`) proves, message-by-message, that this
//! encoder is **byte-identical** to the hand codec over the hard FX-legacy cases
//! and representative simple / nested / repeated / oneof messages, before
//! `handle_unary` is ever swapped onto it (the final increment).
//!
//! ## How it is descriptor-driven
//!
//! [`encode`] iterates [`celnet_proto::wire_contract::fields_for`] for the message
//! and, for each [`WireField`], decides the JSON key (the field table's `json_key`
//! as remapped by [`super::codec_overrides::field_rule`]), presence handling (from
//! the descriptor `label` / `oneof_group`) and value shape (scalar / nested
//! message / repeated) — all from the table. Nested message fields resolve their
//! own table by stripping the parent prefix off the descriptor `proto_type`
//! ([`simple_type_name`], quirk 1), and the whole loop recurses.
//!
//! ## The reflection bridge
//!
//! Rust has no runtime field reflection over `prost` structs, so the raw field
//! *values* are read through the [`WireAdapter`] trait, implemented per concrete
//! message. An adapter is a dumb value accessor keyed by the **proto** field name
//! (`get("price") → F64(self.price)`); it makes **no** JSON-shaping decisions —
//! the JSON key, oneof wrapping, `null`/omission and synthesized keys are all
//! decided by [`encode`] from the tables. This keeps the encoding logic genuinely
//! descriptor-driven while confining the unavoidable per-message code to
//! mechanical field access. (For the eventual full-surface swap this adapter layer
//! is the natural place to auto-generate from the descriptor in `build.rs`.)

use celnet_proto::wire_contract::{self, WireField, WireLabel};
use celnet_proto::{
    Accumulator, AmericanOption, ArbReport, AsianOption, AttributionRecord, BasketLeg,
    BasketOption, BondInstrument, BookId, BrokenDate, CcyPair, Cliquet, CombinedTailRiskRequest,
    CombinedTailRiskResponse, CommodityRef, Conventions, CreateFixConnectionRequest,
    CreateFixConnectionResponse, CryptoPair, CurveSet, DealerQuote, DeleteFixConnectionRequest,
    DeleteFixConnectionResponse, Digital, DoubleBarrier, EntitlementPrincipal, EntitlementRule,
    EquityRef, Execution, FixConnectionDesc, FixConnectionSpec, FixMessage, FixingSchedule,
    ForwardStart, FraInstrument, FxForward, FxSwap, Greeks, Instrument, JointTailScenario, Leg,
    ListFixConnectionsRequest, ListFixConnectionsResponse, ListFixMessagesRequest,
    ListFixMessagesResponse, ListedFutureOption, Lookback, MarketContext, MetalPair,
    MultiDealerQuote, Ndf, OisFixedPeriod, OisInstrument, OisPillar, OisSwapLeg, Owner,
    PerpetualOption, PillarTenor, Pivot, PriceRequest, PriceResponse, PriceXvaRequest,
    PriceXvaResponse, Quantity, Quanto, Quote, QuoteAccept, QuoteReject, QuoteRequest,
    RateSensitivities, RatesInstrument, RatesPriceRequest, RatesPriceResponse, RatesPricingResult,
    RatesQuote, RatesQuoteRequest, RejectAck, RiskScope, SetFixConnectionEnabledRequest,
    SetFixConnectionEnabledResponse, SingleBarrier, Solve, Strategy, StrikeOrDelta, Symbol,
    TailRiskCurvePillar, TailRiskFiPosition, TailRiskKeyRate, TailRiskOptionLeg, Tarf, Tenor,
    Touch, TwoWayPrice, Underlying, UpdateFixConnectionRequest, UpdateFixConnectionResponse,
    Vanilla, VanillaIrsInstrument, VarEs, VarianceSwap, VolatilitySwap, WindowBarrier,
    XvaResult as WireXvaResult, XvaSurvivalCurve, XvaTrade, instrument, pillar_tenor,
    rate_sensitivities, rates_instrument, strike_or_delta, tail_risk_fi_position,
};
// Wave-3 verb families (arch item G — `ws-codec-from-proto`): the surface
// (`GetSmile`/`MarkSurface`/`Scenario`), server-side risk (`ListPositions` /
// `AggregateRisk` / `AggregateRatesRisk` / `DrillRisk` / `LimitStatus` /
// `BookRatesPosition` / `ListRatesPositions`) and dealer-desk RFQ
// (`SubmitDeskRequest` / `RespondDeskRequest` / `AcceptDeskQuote` /
// `ListDeskRequests` / `ListDeals`) message trees.
use celnet_proto::{
    AcceptDeskQuoteRequest, AcceptDeskQuoteResponse, AdditiveRisk, AggregateRatesRiskRequest,
    AggregateRatesRiskResponse, AggregateRiskRequest, AggregateRiskResponse,
    BookRatesPositionRequest, BookRatesPositionResponse, BrokerQuoteSet, BucketedRisk,
    CcyExposureLeg, CrossGamma, Deal, DealScope, DeskQuote, DeskReject, DeskRequest,
    DeskRequestScope, DrillRiskRequest, DrillRiskResponse, GetSmileRequest, KeyRateDv01,
    LimitStatusRequest, LimitStatusResponse, LimitUtilization, ListDealsRequest, ListDealsResponse,
    ListDeskRequestsRequest, ListDeskRequestsResponse, ListPositionsRequest, ListPositionsResponse,
    ListRatesPositionsRequest, ListRatesPositionsResponse, MarkSurfaceRequest, MarkSurfaceResponse,
    NonAdditiveRisk, NumeraireRate, OrgKey, RatesPosition, RatesRiskNode, RatesRiskScope,
    ReportingNumeraire, RespondDeskRequestRequest, RespondDeskRequestResponse, RiskBucketRequest,
    RiskNode, RiskPosition, ScenarioPoint, ScenarioRequest, ScenarioResponse, ShockAxis, Smile,
    SmileModel, SmilePoint, SubmitDeskRequestRequest, SubmitDeskRequestResponse, VanillaInputs,
    VegaBucket, VegaLadderBucket, VegaPillar, respond_desk_request_request::Response as RespondArm,
};
// Curve-query verb family (SurfaceService `GetCurve` / `MarkCurve` / `CurveScenario`
// — the fixed-income market-data query surface, ADR-0021). Pure rates messages with
// no FX-legacy wire quirks, so they encode/decode straight from the field tables
// (round-trip proven in `tests/curve_query_ws.rs`).
use celnet_proto::{
    CurveParPillar, CurvePoint, CurveScenarioReprice, CurveScenarioRequest, CurveScenarioResponse,
    GetCurveRequest, GetCurveResponse, MarkCurveRequest, MarkCurveResponse,
};
// Wave-4 verb family (arch item G — `ws-codec-from-proto`): the AuthService surface
// — login/session, the user / desk / entity / book CRUD, capabilities + roles, the
// instrument registry (`InstrumentDefDesc` + its `definition` family oneof), and the
// `BuildCurve` curve-calibration verb. This is the final family; after it every WS
// unary verb runs on the descriptor-driven generated codec.
use celnet_proto::{
    AggregatedBookDesc, AggregatedBookSpec, AggregationParamsDesc, BondDef, BookDesc,
    BuildCurveRequest, CalibratedCurve, CalibratedCurvePoint, CapabilityDesc,
    CreateAggregatedBookRequest, CreateAggregatedBookResponse, CreateBookRequest,
    CreateBookResponse, CreateDeskRequest, CreateDeskResponse, CreateEntityRequest,
    CreateEntityResponse, CreateInstrumentRequest, CreateInstrumentResponse,
    CreatePricingGroupRequest, CreatePricingGroupResponse, CreateUserRequest, CreateUserResponse,
    DatePillar, DeleteAggregatedBookRequest, DeleteAggregatedBookResponse, DeleteBookRequest,
    DeleteBookResponse, DeleteDeskRequest, DeleteDeskResponse, DeleteEntityRequest,
    DeleteEntityResponse, DeleteInstrumentRequest, DeleteInstrumentResponse,
    DeletePricingGroupRequest, DeletePricingGroupResponse, DeleteUserRequest, DeleteUserResponse,
    DepositDef, DeskDesc, EntityDesc, ExternalId, FeaturePipelineDesc, FeatureSpecDesc, FraDef,
    GetInstrumentRequest, GetInstrumentResponse, GetRoleCapabilitiesRequest,
    GetRoleCapabilitiesResponse, GetUserCapabilitiesRequest, GetUserCapabilitiesResponse,
    InstrumentDefDesc, InstrumentQuote, ListAggregatedBooksRequest, ListAggregatedBooksResponse,
    ListBooksRequest, ListBooksResponse, ListDesksRequest, ListDesksResponse, ListEntitiesRequest,
    ListEntitiesResponse, ListInstrumentsRequest, ListInstrumentsResponse,
    ListPricingGroupsRequest, ListPricingGroupsResponse, ListUsersRequest, ListUsersResponse,
    LoginRequest, LoginResponse, LogoutRequest, LogoutResponse, OisDef, PricingGroupDesc,
    PricingGroupSpec, PricingProvenance, ResetPasswordRequest, ResetPasswordResponse,
    SetRoleCapabilitiesRequest, SetRoleCapabilitiesResponse, SetUserCapabilitiesRequest,
    SetUserCapabilitiesResponse, StirFutureDef, TieringConfigDesc, TieringGuardrailsDesc,
    TieringStrategyDesc, UpdateAggregatedBookRequest, UpdateAggregatedBookResponse,
    UpdateBookRequest, UpdateBookResponse, UpdateDeskRequest, UpdateDeskResponse,
    UpdateEntityRequest, UpdateEntityResponse, UpdateInstrumentRequest, UpdateInstrumentResponse,
    UpdatePricingGroupPipelineRequest, UpdatePricingGroupPipelineResponse,
    UpdatePricingGroupRequest, UpdatePricingGroupResponse, UpdateUserRequest, UpdateUserResponse,
    UserDesc, VanillaIrsDef, instrument_def_desc::Definition as InstrumentDefinition,
};
// Aggregated-book composite publish frames (D3): the descriptor-driven ENCODE side
// of the GUI-facing composite a subscriber reads over `StreamService.StreamSession`,
// proven byte-identical to the hand codec by `tests/ws_codec_differential.rs`.
use celnet_proto::{
    AggregatedBookSnapshot, AggregatedBookStreamSnapshot, AggregatedBookStreamUpdate,
    AggregatedInstrument, LpContribution, SubscriptionId,
};
use serde_json::{Map, Value, json};

use super::codec::CodecError;
use super::codec_overrides::{self, FieldRule};
use celnet_proto::Notification;

// ---------------------------------------------------------------------------
// value model + reflection bridge
// ---------------------------------------------------------------------------

/// The raw value of one proto field, read from a concrete message by its
/// [`WireAdapter`]. Carries only the value + its primitive kind; the JSON shaping
/// (key, oneof wrapping, recursion) is decided by [`encode`] from the field table.
enum WireVal<'a> {
    /// A `double` / `float` scalar.
    F64(f64),
    /// An unsigned integer scalar (`uint32` / `uint64` / `fixed*`).
    U64(u64),
    /// A signed integer scalar (`int32`/`int64`/`sint*`/`sfixed*`).
    I64(i64),
    /// A proto enum, carried by its canonical enum number (as the hand codec does).
    Enum(i32),
    /// A `bool` scalar.
    Bool(bool),
    /// A UTF-8 `string` scalar.
    Str(&'a str),
    /// A present nested message — encoded by recursing on its own field table.
    Msg(&'a dyn WireAdapter),
    /// A `repeated` message field.
    RepeatedMsg(Vec<&'a dyn WireAdapter>),
    /// A `repeated double` scalar field (e.g. a rates `key_rate_ladder`).
    RepeatedF64(&'a [f64]),
    /// A `repeated string` scalar field (e.g. an instrument family's `calendars`).
    RepeatedStr(&'a [String]),
    /// A `repeated` enum field, carried by canonical enum number (e.g. the
    /// `PricingProvenance.features` waterfall) — encoded as an array of ints, matching
    /// the hand codec's enum-as-int convention.
    RepeatedEnum(&'a [i32]),
}

/// The reflection bridge: a per-message accessor yielding a field's raw value by
/// its **proto** (snake_case) name. Implementations are mechanical field reads and
/// make no JSON-shaping decisions.
trait WireAdapter {
    /// The value of proto field `proto_name`, or `None` when it is absent — an
    /// unset `oneof` arm, an absent proto3 `optional`, or an absent singular
    /// message. [`encode`] turns `None` into omission or `null` per the descriptor
    /// label.
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>>;

    /// Message-level synthesized JSON keys computed from an accessor rather than a
    /// straight field projection (the curated FX-legacy synthesis, quirks b/c).
    /// Default: none.
    fn synthesized(&self) -> Vec<(&'static str, Value)> {
        Vec::new()
    }

    /// A bespoke whole-message projection the descriptor field table cannot
    /// express (quirk a — `Underlying` → the legacy `{base, quote}` body); when
    /// `Some`, it *replaces* the generic field-table encoding entirely. Default:
    /// none (use the generic path).
    fn message_projection(&self) -> Option<Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// the generic, table-driven encoder
// ---------------------------------------------------------------------------

/// The package-relative simple type name of a descriptor `proto_type`
/// (`RateSensitivities.FxRho` → `FxRho`, `Tenor.Unit` → `Unit`) — the key the
/// per-message field tables ([`celnet_proto::wire_contract::fields_for`]) are
/// keyed on (quirk 1: strip the parent prefix to resolve a nested field's table).
fn simple_type_name(proto_type: &str) -> &str {
    proto_type.rsplit('.').next().unwrap_or(proto_type)
}

/// Whether `proto_type` names a contract message (has its own field table) — as
/// opposed to a scalar (`double`, `string`, …) or an enum (which carries no field
/// table). Drives the "absent singular message ⇒ `null`" rule below.
fn is_message_type(proto_type: &str) -> bool {
    wire_contract::fields_for(simple_type_name(proto_type)).is_some()
}

/// Encode one already-fetched field value to its JSON form, recursing into nested
/// / repeated messages by resolving their field table off the descriptor
/// `proto_type`.
fn encode_value(field: &WireField, value: WireVal<'_>) -> Value {
    match value {
        WireVal::F64(x) => json!(x),
        WireVal::U64(x) => json!(x),
        WireVal::I64(x) => json!(x),
        WireVal::Enum(x) => json!(x),
        WireVal::Bool(b) => json!(b),
        WireVal::Str(s) => json!(s),
        WireVal::Msg(inner) => encode(simple_type_name(field.proto_type), inner),
        WireVal::RepeatedMsg(items) => Value::Array(
            items
                .into_iter()
                .map(|inner| encode(simple_type_name(field.proto_type), inner))
                .collect(),
        ),
        WireVal::RepeatedF64(items) => Value::Array(items.iter().map(|x| json!(x)).collect()),
        WireVal::RepeatedStr(items) => Value::Array(items.iter().map(|s| json!(s)).collect()),
        WireVal::RepeatedEnum(items) => Value::Array(items.iter().map(|x| json!(x)).collect()),
    }
}

/// Encode `message` (by its simple type name) from its descriptor field table +
/// the curated override rules, reading raw values through `adapter`. The single
/// descriptor-driven encode entry point; recurses for nested messages.
fn encode(message: &str, adapter: &dyn WireAdapter) -> Value {
    // (a) A whole-message FX-legacy projection short-circuits the field table.
    if let Some(projected) = adapter.message_projection() {
        return projected;
    }

    let mut map = Map::new();
    if let Some(fields) = wire_contract::fields_for(message) {
        for field in fields {
            let key = match codec_overrides::field_rule(message, field.proto_name) {
                FieldRule::Keep => field.json_key,
                FieldRule::Rename(k) => k,
                // Suppressed: its value reaches the wire through a synthesized key.
                FieldRule::Suppress => continue,
            };
            match adapter.get(field.proto_name) {
                Some(value) => {
                    map.insert(key.to_owned(), encode_value(field, value));
                }
                None => {
                    // An absent SINGULAR message field mirrors the hand codec's
                    // `.map(..)` → `null`. An unset oneof arm is always omitted. An
                    // absent proto3 `optional` is omitted by default (the request-side
                    // leaf messages, e.g. `Tenor.broken_date`) but emitted as `null`
                    // for the Price-family response messages whose hand encoders build
                    // via `json!({ .. })` (`correlation_id` / `surface_version` /
                    // `price_std_error`) — the per-message override, quirk-symmetric
                    // with the request side.
                    if field.oneof_group.is_none() {
                        // An absent SINGULAR message field renders as `null` for the
                        // `json!({ .. })`-style hand encoders, but is OMITTED for the
                        // manual-`Map`-building hand encoders (`attribution_to_json` /
                        // `book_id_to_json`, which insert only present fields) — the
                        // latter are flagged by `omit_absent_message`.
                        let absent_singular_message = field.label == WireLabel::Singular
                            && is_message_type(field.proto_type)
                            && !codec_overrides::omit_absent_message(message);
                        let null_absent_optional = field.label == WireLabel::Optional
                            && codec_overrides::null_absent_optional(message);
                        if absent_singular_message || null_absent_optional {
                            map.insert(key.to_owned(), Value::Null);
                        }
                    }
                }
            }
        }
    }

    // (b)/(c) message-level synthesized keys (computed from accessors).
    for (key, value) in adapter.synthesized() {
        map.insert(key.to_owned(), value);
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// public typed entry points (the message surface this increment proves)
// ---------------------------------------------------------------------------

/// Encode a [`CcyPair`] to its WS JSON `{base, quote}` — descriptor-driven.
#[must_use]
pub fn encode_ccy_pair(p: &CcyPair) -> Value {
    encode("CcyPair", p)
}

/// Encode a [`RateSensitivities`] to its carry-tagged WS JSON oneof
/// (`{"fx": {rho_dom, rho_for}}` / `{"carry": {discount_rho, carry_rho}}`).
#[must_use]
pub fn encode_rate_sensitivities(rs: &RateSensitivities) -> Value {
    encode("RateSensitivities", rs)
}

/// Encode a [`Greeks`] to its WS JSON — including the FX-legacy flat
/// `rho_dom`/`rho_for` synthesized beside the `rate_sensitivities` oneof (quirk c).
#[must_use]
pub fn encode_greeks(g: &Greeks) -> Value {
    encode("Greeks", g)
}

/// Encode a [`MarketContext`] to its FX-legacy WS JSON
/// `{spot, vol, r_dom, r_for}` (quirk b).
#[must_use]
pub fn encode_market_context(m: &MarketContext) -> Value {
    encode("MarketContext", m)
}

/// Encode an [`Underlying`] to its FX-legacy `{base, quote}` `pair` body, or
/// `null` for a non-FX underlying (quirk a).
#[must_use]
pub fn encode_underlying(u: &Underlying) -> Value {
    encode("Underlying", u)
}

/// Encode a [`Tenor`] to its WS JSON, with the camelCase `brokenDate` key
/// (quirk d).
#[must_use]
pub fn encode_tenor(t: &Tenor) -> Value {
    encode("Tenor", t)
}

/// Encode a [`Strategy`] to its WS JSON (a repeated `legs` array, each leg
/// carrying the `strike`/`delta` oneof body).
#[must_use]
pub fn encode_strategy(s: &Strategy) -> Value {
    encode("Strategy", s)
}

/// Encode a [`Conventions`] to its WS JSON (the six market-convention enum tags),
/// descriptor-driven (mirrors the hand `conventions_to_json`).
#[must_use]
pub fn encode_conventions(c: &Conventions) -> Value {
    encode("Conventions", c)
}

/// Encode a [`PriceResponse`] (the one-shot pricing reply) to its WS JSON — the
/// Greeks strip, resolved strike, echoed conventions and the presence-tracked
/// `correlation_id` / `surface_version` / `price_std_error` emitted as `null` when
/// absent (mirrors the hand `price_response_to_json`).
#[must_use]
pub fn encode_price_response(r: &PriceResponse) -> Value {
    encode("PriceResponse", r)
}

/// Encode a [`RatesPriceResponse`] to its WS JSON (mirrors the hand
/// `rates_price_response_to_json`).
#[must_use]
pub fn encode_rates_price_response(r: &RatesPriceResponse) -> Value {
    encode("RatesPriceResponse", r)
}

/// Encode a [`RatesQuote`] (the fixed-income taker RFQ reply of
/// `QuoteService.RequestRatesQuote`) to its WS JSON — the two-way bid/offer, the
/// full FI risk `result`, size, timestamps, and the presence-tracked
/// `correlation_id` emitted as `null` when absent (see
/// [`super::codec_overrides::null_absent_optional`]). Descriptor-driven, so a WS
/// FI RFQ reply is byte-identical to the gRPC `RatesQuote`.
#[must_use]
pub fn encode_rates_quote(q: &RatesQuote) -> Value {
    encode("RatesQuote", q)
}

/// Encode a [`PriceXvaResponse`] to its WS JSON (mirrors the hand
/// `price_xva_response_to_json`).
#[must_use]
pub fn encode_price_xva_response(r: &PriceXvaResponse) -> Value {
    encode("PriceXvaResponse", r)
}

/// Encode an [`ArbReport`] to its WS JSON — the two arbitrage flags, worst density,
/// note, the numeric `smile_model` provenance tag and the synthesized
/// `smile_model_label` (mirrors the hand `arb_report_to_json`; the label is
/// re-derived independently from the `SmileModel` enum in
/// [`super::codec_overrides::arb_report_synth`]).
#[must_use]
pub fn encode_arb_report(a: &ArbReport) -> Value {
    encode("ArbReport", a)
}

// --- curve-query entry points (SurfaceService GetCurve / MarkCurve / CurveScenario —
//     the fixed-income market-data query surface, ADR-0021). Descriptor-driven, so a
//     WS curve read is byte-identical to the gRPC reply. --------------------------

/// Decode a WS `get_curve` body into a [`GetCurveRequest`].
///
/// # Errors
/// Malformed `curve_set` / tenor body, as a [`CodecError`].
pub fn decode_get_curve_request(o: &Map<String, Value>) -> DResult<GetCurveRequest> {
    decode(GetCurveRequest::MESSAGE, o)
}

/// Encode a [`GetCurveResponse`] (the read curve: points, par pillars, version) to
/// its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_get_curve_response(r: &GetCurveResponse) -> Value {
    encode("GetCurveResponse", r)
}

/// Decode a WS `mark_curve` body into a [`MarkCurveRequest`].
///
/// # Errors
/// Malformed `curve_set` body, as a [`CodecError`].
pub fn decode_mark_curve_request(o: &Map<String, Value>) -> DResult<MarkCurveRequest> {
    decode(MarkCurveRequest::MESSAGE, o)
}

/// Encode a [`MarkCurveResponse`] (the marked version + bootstrapped points) to its
/// WS JSON — descriptor-driven.
#[must_use]
pub fn encode_mark_curve_response(r: &MarkCurveResponse) -> Value {
    encode("MarkCurveResponse", r)
}

/// Decode a WS `curve_scenario` body into a [`CurveScenarioRequest`].
///
/// # Errors
/// Malformed `curve_set` / `instrument` / shift body, as a [`CodecError`].
pub fn decode_curve_scenario_request(o: &Map<String, Value>) -> DResult<CurveScenarioRequest> {
    decode(CurveScenarioRequest::MESSAGE, o)
}

/// Encode a [`CurveScenarioResponse`] (the shifted curve + optional repriced leg) to
/// its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_curve_scenario_response(r: &CurveScenarioResponse) -> Value {
    encode("CurveScenarioResponse", r)
}

// ---------------------------------------------------------------------------
// per-message reflection adapters (mechanical field access only)
// ---------------------------------------------------------------------------

impl WireAdapter for CcyPair {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "base" => Some(WireVal::Str(&self.base)),
            "quote" => Some(WireVal::Str(&self.quote)),
            _ => None,
        }
    }
}

impl WireAdapter for Underlying {
    fn get(&self, _proto_name: &str) -> Option<WireVal<'_>> {
        // Never reached: the FX-legacy whole-message projection short-circuits the
        // field-table loop for `Underlying`.
        None
    }

    fn message_projection(&self) -> Option<Value> {
        Some(codec_overrides::underlying_fx_projection(self))
    }
}

impl WireAdapter for MarketContext {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "spot" => Some(WireVal::F64(self.spot)),
            "vol" => Some(WireVal::F64(self.vol)),
            // Renamed to `r_dom` by the override; `carry` is suppressed and reaches
            // the wire as the synthesized `r_for` instead.
            "discount_rate" => Some(WireVal::F64(self.discount_rate)),
            _ => None,
        }
    }

    fn synthesized(&self) -> Vec<(&'static str, Value)> {
        codec_overrides::market_context_synth(self)
    }
}

impl WireAdapter for Greeks {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "price" => Some(WireVal::F64(self.price)),
            "delta_spot" => Some(WireVal::F64(self.delta_spot)),
            "delta_forward" => Some(WireVal::F64(self.delta_forward)),
            "gamma" => Some(WireVal::F64(self.gamma)),
            "vega" => Some(WireVal::F64(self.vega)),
            "theta" => Some(WireVal::F64(self.theta)),
            "rate_sensitivities" => self
                .rate_sensitivities
                .as_ref()
                .map(|rs| WireVal::Msg(rs as &dyn WireAdapter)),
            "vanna" => Some(WireVal::F64(self.vanna)),
            "volga" => Some(WireVal::F64(self.volga)),
            "charm" => Some(WireVal::F64(self.charm)),
            "speed" => Some(WireVal::F64(self.speed)),
            "zomma" => Some(WireVal::F64(self.zomma)),
            "color" => Some(WireVal::F64(self.color)),
            _ => None,
        }
    }

    fn synthesized(&self) -> Vec<(&'static str, Value)> {
        codec_overrides::greeks_synth(self)
    }
}

impl WireAdapter for RateSensitivities {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        use rate_sensitivities::Sensitivities;
        match (proto_name, &self.sensitivities) {
            ("fx", Some(Sensitivities::Fx(fx))) => Some(WireVal::Msg(fx as &dyn WireAdapter)),
            ("carry", Some(Sensitivities::Carry(c))) => Some(WireVal::Msg(c as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for rate_sensitivities::FxRho {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "rho_dom" => Some(WireVal::F64(self.rho_dom)),
            "rho_for" => Some(WireVal::F64(self.rho_for)),
            _ => None,
        }
    }
}

impl WireAdapter for rate_sensitivities::CarryRho {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "discount_rho" => Some(WireVal::F64(self.discount_rho)),
            "carry_rho" => Some(WireVal::F64(self.carry_rho)),
            _ => None,
        }
    }
}

impl WireAdapter for Tenor {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "unit" => Some(WireVal::Enum(self.unit)),
            "count" => Some(WireVal::U64(u64::from(self.count))),
            "broken_date" => self
                .broken_date
                .as_ref()
                .map(|b| WireVal::Msg(b as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for BrokenDate {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "year" => Some(WireVal::I64(i64::from(self.year))),
            "month" => Some(WireVal::U64(u64::from(self.month))),
            "day" => Some(WireVal::U64(u64::from(self.day))),
            _ => None,
        }
    }
}

impl WireAdapter for Strategy {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "kind" => Some(WireVal::Enum(self.kind)),
            "legs" => Some(WireVal::RepeatedMsg(
                self.legs.iter().map(|l| l as &dyn WireAdapter).collect(),
            )),
            _ => None,
        }
    }
}

impl WireAdapter for Leg {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "option_type" => Some(WireVal::Enum(self.option_type)),
            "strike" => self
                .strike
                .as_ref()
                .map(|s| WireVal::Msg(s as &dyn WireAdapter)),
            "side" => Some(WireVal::Enum(self.side)),
            "ratio" => Some(WireVal::F64(self.ratio)),
            _ => None,
        }
    }
}

impl WireAdapter for StrikeOrDelta {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        use strike_or_delta::Spec;
        match (proto_name, &self.spec) {
            ("strike", Some(Spec::Strike(x))) => Some(WireVal::F64(*x)),
            ("delta", Some(Spec::Delta(x))) => Some(WireVal::F64(*x)),
            _ => None,
        }
    }
}

// --- Price-family RESPONSE adapters (arch item G — increment 4) --------------
// The one-shot pricing reply surface. Absent proto3-`optional` scalars return
// `None` here and are turned into JSON `null` by the generic encoder's
// [`codec_overrides::null_absent_optional`] policy for these messages.

impl WireAdapter for Conventions {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "delta_convention" => Some(WireVal::Enum(self.delta_convention)),
            "atm_convention" => Some(WireVal::Enum(self.atm_convention)),
            "premium_style" => Some(WireVal::Enum(self.premium_style)),
            "cut" => Some(WireVal::Enum(self.cut)),
            "day_count" => Some(WireVal::Enum(self.day_count)),
            "settlement" => Some(WireVal::Enum(self.settlement)),
            _ => None,
        }
    }
}

impl WireAdapter for PriceResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "request_id" => Some(WireVal::U64(self.request_id)),
            "greeks" => self
                .greeks
                .as_ref()
                .map(|g| WireVal::Msg(g as &dyn WireAdapter)),
            "resolved_strike" => Some(WireVal::F64(self.resolved_strike)),
            "conventions" => self
                .conventions
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            "surface_version" => self.surface_version.map(WireVal::U64),
            "price_std_error" => self.price_std_error.map(WireVal::F64),
            _ => None,
        }
    }
}

impl WireAdapter for RatesPricingResult {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "pv" => Some(WireVal::F64(self.pv)),
            "par_rate" => Some(WireVal::F64(self.par_rate)),
            "pv01" => Some(WireVal::F64(self.pv01)),
            "dv01" => Some(WireVal::F64(self.dv01)),
            "key_rate_ladder" => Some(WireVal::RepeatedF64(&self.key_rate_ladder)),
            "bid" => Some(WireVal::F64(self.bid)),
            "offer" => Some(WireVal::F64(self.offer)),
            "bid_size" => Some(WireVal::F64(self.bid_size)),
            "offer_size" => Some(WireVal::F64(self.offer_size)),
            _ => None,
        }
    }
}

impl WireAdapter for RatesPriceResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "request_id" => Some(WireVal::U64(self.request_id)),
            "result" => self
                .result
                .as_ref()
                .map(|r| WireVal::Msg(r as &dyn WireAdapter)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for TwoWayPrice {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "bid" => Some(WireVal::F64(self.bid)),
            "offer" => Some(WireVal::F64(self.offer)),
            _ => None,
        }
    }
}

impl WireAdapter for RatesQuote {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "quote_id" => Some(WireVal::U64(self.quote_id)),
            "idempotency_key" => Some(WireVal::Str(&self.idempotency_key)),
            "price" => self
                .price
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            "result" => self
                .result
                .as_ref()
                .map(|r| WireVal::Msg(r as &dyn WireAdapter)),
            "notional" => Some(WireVal::F64(self.notional)),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            "valid_until_nanos" => Some(WireVal::I64(self.valid_until_nanos)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for WireXvaResult {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "cva" => Some(WireVal::F64(self.cva)),
            "dva" => Some(WireVal::F64(self.dva)),
            "fva" => Some(WireVal::F64(self.fva)),
            "total_adjustment" => Some(WireVal::F64(self.total_adjustment)),
            _ => None,
        }
    }
}

impl WireAdapter for PriceXvaResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "request_id" => Some(WireVal::U64(self.request_id)),
            "result" => self
                .result
                .as_ref()
                .map(|r| WireVal::Msg(r as &dyn WireAdapter)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for ArbReport {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "butterfly_arbitrage_free" => Some(WireVal::Bool(self.butterfly_arbitrage_free)),
            "calendar_arbitrage_free" => Some(WireVal::Bool(self.calendar_arbitrage_free)),
            "worst_density" => Some(WireVal::F64(self.worst_density)),
            "note" => Some(WireVal::Str(&self.note)),
            "smile_model" => Some(WireVal::Enum(self.smile_model)),
            _ => None,
        }
    }

    fn synthesized(&self) -> Vec<(&'static str, Value)> {
        codec_overrides::arb_report_synth(self.smile_model)
    }
}

// ===========================================================================
// DECODE (arch item G — `ws-codec-from-proto`, increment 3)
// ===========================================================================
//
// The input-side mirror of the encoder above: decode a WS JSON body to the proto
// message, purely from the same descriptor-derived field tables
// ([`celnet_proto::wire_contract::fields_for`]) plus the curated
// [override table](super::codec_overrides). It is an **independent code path** that
// does **not** call the hand codec; the differential harness
// (`tests/ws_codec_differential.rs`) proves, message-by-message, that
// `generated_decode(json)` produces the **byte-identical** proto message the hand
// decoder ([`super::codec`]) produces over the client conformance corpus, before
// `handle_unary` is ever swapped onto it.
//
// ## How it is descriptor-driven (symmetric to `encode`)
//
// [`decode`] iterates [`fields_for`] for the message and, for each [`WireField`],
// decides the JSON key (the field table's `json_key` as remapped by
// [`super::codec_overrides::field_rule`] — the very same override the encoder
// reads, so encode and decode never disagree on a key) and the oneof-arm
// precedence (first present arm in descriptor declaration order, mirroring the
// hand codec's `if / else if` cascade) — all from the table. The per-message
// [`WireBuilder`] then does the mechanical conversion + placement of one field's
// value onto a `Default`-constructed message, recursing into nested messages via
// the typed `decode_*` entry points. Per-field **presence policy**
// (required-erroring vs proto3-default vs `Option`-`None`) — which the descriptor
// cannot express (proto3 has no `required`) — lives in that mechanical placement
// via the [presence helpers](self) (`req_*` error on absence, the rest default),
// exactly reproducing the hand codec's field-by-field choice. The three FX-legacy
// **message-level** decode projections the field table cannot express
// (`MarketContext` → the `fx` constructor; `Underlying` → the legacy `{base,quote}`
// pair / the richer `underlying` oneof) short-circuit the generic walk, symmetric
// to the encoder's [`WireAdapter::message_projection`].

/// The decode result alias — the hand codec's [`CodecError`] is the shared error.
type DResult<T> = std::result::Result<T, CodecError>;

/// The object body of a JSON value, or a codec error if it is not an object.
/// Mirrors the hand codec's private `obj` primitive (byte-identical result).
fn obj<'a>(v: &'a Value, what: &str) -> DResult<&'a Map<String, Value>> {
    v.as_object()
        .ok_or_else(|| CodecError(format!("{what} must be a JSON object")))
}

// --- presence-aware scalar conversions (the per-field decode policy) --------
// Each mirrors exactly one hand-codec primitive (`codec.rs` "scalar field
// accessors"): the `req_*` helpers reproduce the required accessors that error on
// absence (`f64_field` / `string_field`), the rest reproduce the proto3-default
// accessors (`*_or_zero` / `*_or_empty` / `enum_or_zero`). `value` is the field's
// JSON value if present-and-non-null (as resolved by [`decode`]), else `None`.

/// A required `f64` (mirrors `f64_field`): error on absence or a non-numeric value.
fn req_f64(value: Option<&Value>, field: &str) -> DResult<f64> {
    value
        .and_then(Value::as_f64)
        .ok_or_else(|| CodecError(format!("missing or non-numeric field `{field}`")))
}

/// An `f64` defaulting to `0.0` (mirrors `f64_or_zero`).
fn f64_or_zero(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

/// A required `String` (mirrors `string_field`): error on absence or a non-string.
fn req_string(value: Option<&Value>, field: &str) -> DResult<String> {
    value
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| CodecError(format!("missing or non-string field `{field}`")))
}

/// A `String` defaulting to empty (mirrors `string_or_empty`).
fn string_or_empty(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// A `bool` defaulting to `false` (mirrors `bool_or_false`).
fn bool_or_false(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

/// An i32 enum-tag defaulting to the proto3 zero value (mirrors `enum_or_zero`).
fn enum_or_zero(value: Option<&Value>) -> i32 {
    value
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

/// A `u32` count defaulting to `0` (mirrors `u32::try_from(u64_or_zero(..))`).
fn u32_or_zero(value: Option<&Value>) -> u32 {
    u32::try_from(value.and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0)
}

/// An i32 (signed year) defaulting to `0` (mirrors the `BrokenDate.year` decode).
fn i32_or_zero(value: Option<&Value>) -> i32 {
    value
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

/// A required nested message (mirrors `nested`): error on absence, else decode it
/// from its object body via `T`'s [`WireBuilder`].
fn req_msg<T: WireBuilder>(value: Option<&Value>, what: &str) -> DResult<T> {
    let v = value.ok_or_else(|| CodecError(format!("missing nested field `{what}`")))?;
    decode(T::MESSAGE, obj(v, what)?)
}

/// An optional nested message (mirrors `opt_nested`): `None` on absence, else
/// decode it via `T`'s [`WireBuilder`].
fn opt_msg<T: WireBuilder>(value: Option<&Value>, what: &str) -> DResult<Option<T>> {
    match value {
        None => Ok(None),
        Some(v) => decode(T::MESSAGE, obj(v, what)?).map(Some),
    }
}

/// A required repeated message (mirrors `strategy_from_json`'s `legs` decode): the
/// value must be a JSON array; each element is decoded via `T`'s [`WireBuilder`].
fn req_repeated<T: WireBuilder>(value: Option<&Value>, what: &str) -> DResult<Vec<T>> {
    value
        .and_then(Value::as_array)
        .ok_or_else(|| CodecError(format!("field `{what}` must be a JSON array")))?
        .iter()
        .map(|v| decode(T::MESSAGE, obj(v, what)?))
        .collect()
}

/// A `u64` defaulting to `0` (mirrors `u64_or_zero`).
fn u64_or_zero(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

/// An optional presence-tracked `u64` (mirrors `opt_u64`).
fn opt_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
}

/// An optional presence-tracked `f64` (mirrors `opt_f64`).
fn opt_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64)
}

/// A REQUIRED `u64` field carried as a JSON integer (mirrors the hand `u64_field`):
/// error on absence or a non-integer value.
fn req_u64(value: Option<&Value>, field: &str) -> DResult<u64> {
    value
        .and_then(Value::as_u64)
        .ok_or_else(|| CodecError(format!("missing or non-integer field `{field}`")))
}

/// An optional presence-tracked `bool` (mirrors `o.get(k).and_then(as_bool)`): `None`
/// on absence or a non-bool value.
fn opt_bool(value: Option<&Value>) -> Option<bool> {
    value.and_then(Value::as_bool)
}

/// An optional presence-tracked `u32` (mirrors `.and_then(as_u64).and_then(|n|
/// u32::try_from(n).ok())`): `None` on absence OR on a value outside `u32` range.
fn opt_u32(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

/// A `u32` count that DEFAULTS to `0` when absent but ERRORS on overflow — the
/// exotic-product `u32::try_from(u64_or_zero(..)).map_err(..)` policy (distinct from
/// [`u32_or_zero`], which silently clamps overflow to `0`; `Tenor.count` uses that).
fn u32_ranged(value: Option<&Value>, field: &str) -> DResult<u32> {
    u32::try_from(value.and_then(Value::as_u64).unwrap_or(0))
        .map_err(|_| CodecError(format!("field `{field}` out of u32 range")))
}

/// A REQUIRED `u32` field carried as a JSON integer, range-checked (mirrors the hand
/// `u32_field`): error on absence / a non-integer value / u32 overflow.
fn req_u32(value: Option<&Value>, field: &str) -> DResult<u32> {
    let n = value
        .and_then(Value::as_u64)
        .ok_or_else(|| CodecError(format!("missing or non-integer field `{field}`")))?;
    u32::try_from(n).map_err(|_| CodecError(format!("field `{field}` out of u32 range")))
}

/// A repeated `f64` defaulting to empty, silently dropping non-numeric elements
/// (mirrors `f64_vec` — the basket `correlations` / schedule `fixing_years` shape).
fn f64_vec(value: Option<&Value>) -> Vec<f64> {
    value
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

/// A repeated `String` defaulting to empty, silently dropping non-string elements
/// (mirrors the hand `string_array` — the instrument family `calendars` shape): a
/// non-array (or absent) value yields an empty vec rather than an error.
fn string_vec(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// A repeated `f64` (absent/null ⇒ empty) that ERRORS on a non-numeric element
/// (mirrors the strict `f64_array` — the XVA survival curve / American Bermudan
/// date-set shape).
fn f64_array(value: Option<&Value>, field: &str) -> DResult<Vec<f64>> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_f64()
                    .ok_or_else(|| CodecError(format!("`{field}[{i}]` must be a number")))
            })
            .collect(),
        Some(_) => Err(CodecError(format!("`{field}` must be an array of numbers"))),
    }
}

/// An OPTIONAL repeated message (absent/null ⇒ empty vec), the proto3-faithful
/// counterpart of the required [`req_repeated`]: a `repeated` field that is simply
/// omitted decodes to an empty list rather than erroring (e.g. the `CombinedTailRisk`
/// `fi_positions` on an options-only request). Each present element is decoded via
/// `T`'s [`WireBuilder`].
fn opt_repeated<T: WireBuilder>(value: Option<&Value>, what: &str) -> DResult<Vec<T>> {
    match value {
        None => Ok(Vec::new()),
        Some(v) => v
            .as_array()
            .ok_or_else(|| CodecError(format!("field `{what}` must be a JSON array")))?
            .iter()
            .map(|e| decode(T::MESSAGE, obj(e, what)?))
            .collect(),
    }
}

/// An optional presence-tracked `String`, **byte-identical to the hand codec's
/// lenient [`super::codec`] `opt_string`** (`o.get(key).and_then(Value::as_str)
/// .filter(|s| !s.is_empty())`). Three byte-identity-critical behaviours mirror the
/// hand codec exactly:
/// - absent / `null` ⇒ `None` (`decode` has already collapsed both to `None` here);
/// - a present **non-string** value ⇒ `None`, **not** a decode error. This is the
///   fix for the FX-legacy wire-key collision the transport creates: the WS framing
///   layer injects a NUMERIC `correlation_id` (the request↔reply sequence) onto every
///   frame, which lands on the SAME JSON key as a proto `string correlation_id`
///   (`SubmitDeskRequestRequest` / `BookRatesPositionRequest` / `ListRatesPositions`
///   / `ListDeskRequests` …). The hand codec's `Value::as_str` yields `None` for that
///   number, leaving the message field `None`; the generated codec MUST do the same or
///   the whole request fails to decode. (A strict "must be a string" error here was the
///   regression that broke the live rates-book booking + RFQ-desk injection e2e flows;
///   the conformance corpus missed it because its vectors set `correlation_id` to a
///   STRING, never the transport's numeric framing id.)
/// - an **empty** string ⇒ `None` (the hand `.filter(|s| !s.is_empty())`), so an admin
///   `session_token: ""` decodes to `None`, not `Some("")`.
///
/// The `field` name is retained for call-site symmetry with [`req_string`] (which
/// needs it for its required-field error); this lenient reader never errors.
fn opt_string(value: Option<&Value>, _field: &str) -> DResult<Option<String>> {
    Ok(value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned))
}

// ---------------------------------------------------------------------------
// the reflection bridge + the generic, table-driven decoder
// ---------------------------------------------------------------------------

/// The decode reflection bridge — the input-side mirror of [`WireAdapter`]. A
/// per-message builder that places one field's value onto a `Default`-constructed
/// message. [`decode`] (the generic, table-driven walk) has already resolved the
/// JSON key (via the shared override) and, for a oneof, selected the single live
/// arm; the builder only converts the raw JSON value to the field's concrete Rust
/// type and places it — recursing into nested messages via [`req_msg`] /
/// [`opt_msg`] / [`req_repeated`]. It carries the per-field **presence policy**
/// (which of the `req_*` vs defaulting helpers to use) — the one thing the proto3
/// descriptor cannot express — and nothing else; the JSON key, oneof precedence
/// and field set are all the descriptor's, decided in [`decode`].
trait WireBuilder: Default {
    /// The simple message type name — the key its field table is registered under
    /// in [`celnet_proto::wire_contract::MESSAGE_FIELDS`].
    const MESSAGE: &'static str;

    /// Place `field`'s value onto `self`. `value` is the field's JSON value when
    /// present-and-non-null, else `None`; the builder applies the required-vs-
    /// default policy (mirroring the hand codec's per-field accessor choice).
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()>;
}

/// Decode `message` (by its simple type name) from its already-unwrapped object
/// body, driven by the descriptor field table + the curated override rules,
/// building the concrete `T` through its [`WireBuilder`]. The single
/// descriptor-driven decode entry point; recurses (via the `req_*`/`opt_*` helpers
/// the builders call) for nested messages.
fn decode<T: WireBuilder>(message: &str, o: &Map<String, Value>) -> DResult<T> {
    let mut builder = T::default();
    let Some(fields) = wire_contract::fields_for(message) else {
        // A message with no field table (none in the current contract reach here)
        // decodes to its default — nothing to read.
        return Ok(builder);
    };

    // Real-oneof groups whose live arm has already been placed — later arms of the
    // same group are ignored (the hand codec's `if / else if` arm precedence).
    let mut oneof_placed: Vec<&str> = Vec::new();

    for field in fields {
        // Resolve the JSON key through the SAME curated override the encoder reads,
        // so encode and decode can never disagree on a key.
        let key = match codec_overrides::field_rule(message, field.proto_name) {
            FieldRule::Keep => field.json_key,
            FieldRule::Rename(k) => k,
            // Suppressed fields are encode-only synthesis (no JSON source to read).
            FieldRule::Suppress => continue,
        };
        let value = o.get(key).filter(|v| !v.is_null());

        match field.oneof_group {
            Some(group) => {
                // First present arm in declaration order wins; skip the rest.
                if oneof_placed.contains(&group) {
                    continue;
                }
                if value.is_some() {
                    builder.set(field, value)?;
                    oneof_placed.push(group);
                }
            }
            // A plain (or proto3-`optional`) field: the builder's presence policy
            // turns `None` into an error (required) or the proto3 default.
            None => builder.set(field, value)?,
        }
    }

    // A required oneof with no live arm errors (mirrors e.g. `strike_or_delta_from_json`
    // "must carry exactly one of `strike` or `delta`").
    for field in fields {
        if let Some(group) = field.oneof_group
            && codec_overrides::oneof_required(message, group)
            && !oneof_placed.contains(&group)
        {
            return Err(CodecError(format!(
                "`{message}` needs exactly one `{group}` arm"
            )));
        }
    }

    Ok(builder)
}

// ---------------------------------------------------------------------------
// public typed decode entry points (the request-side surface this increment
// proves byte-identical to the hand codec)
// ---------------------------------------------------------------------------

/// Decode a WS `{base, quote}` pair into a [`CcyPair`] — descriptor-driven
/// (mirrors the hand `ccy_pair_from_json`).
///
/// # Errors
/// Malformed body (missing `base`/`quote`), as a [`CodecError`].
pub fn decode_ccy_pair(v: &Value) -> DResult<CcyPair> {
    decode(CcyPair::MESSAGE, obj(v, "pair")?)
}

/// Decode a WS `{base, quote}` pair into an FX [`Underlying`] via the byte-identical
/// `fx` constructor (mirrors the hand `underlying_from_json`, quirk a).
///
/// # Errors
/// Malformed pair body, as a [`CodecError`].
pub fn decode_underlying_fx(v: &Value) -> DResult<Underlying> {
    Ok(Underlying::fx(decode_ccy_pair(v)?))
}

/// Decode the richer cross-asset `underlying` oneof object into an [`Underlying`]
/// — exactly one arm (`fx` / `metal` / `equity` / `commodity` / `digital_asset`),
/// mirroring the hand `underlying_object_from_json` arm precedence (quirk a).
///
/// # Errors
/// Malformed body or no/unknown arm, as a [`CodecError`].
pub fn decode_underlying_object(v: &Value) -> DResult<Underlying> {
    let o = obj(v, "underlying")?;
    if let Some(fx) = o.get("fx") {
        Ok(Underlying::fx(decode_ccy_pair(fx)?))
    } else if let Some(m) = o.get("metal") {
        Ok(Underlying::metal(decode(
            MetalPair::MESSAGE,
            obj(m, "metal")?,
        )?))
    } else if let Some(e) = o.get("equity") {
        Ok(Underlying::equity(decode(
            EquityRef::MESSAGE,
            obj(e, "equity")?,
        )?))
    } else if let Some(c) = o.get("commodity") {
        Ok(Underlying::commodity(decode(
            CommodityRef::MESSAGE,
            obj(c, "commodity")?,
        )?))
    } else if let Some(d) = o.get("digital_asset") {
        Ok(Underlying::digital_asset(decode(
            CryptoPair::MESSAGE,
            obj(d, "digital_asset")?,
        )?))
    } else {
        Err(CodecError(
            "underlying needs exactly one arm (fx / metal / equity / commodity / digital_asset)"
                .to_owned(),
        ))
    }
}

/// Decode a WS `{spot, vol, r_dom, r_for}` market body into a [`MarketContext`] via
/// the byte-identical `fx` constructor (mirrors the hand `market_context_from_json`,
/// quirk b: the FX `r_dom`/`r_for` keys, not the generalized `{discount_rate, carry}`).
///
/// # Errors
/// Missing/non-numeric `spot` or `vol`, as a [`CodecError`].
pub fn decode_market_context(v: &Value) -> DResult<MarketContext> {
    let o = obj(v, "market")?;
    Ok(MarketContext::fx(
        req_f64(o.get("spot"), "spot")?,
        req_f64(o.get("vol"), "vol")?,
        f64_or_zero(o.get("r_dom")),
        f64_or_zero(o.get("r_for")),
    ))
}

/// Decode a WS `tenor` body into a [`Tenor`] (mirrors `tenor_from_json`; the
/// camelCase `brokenDate` key is resolved by the shared override, quirk d).
///
/// # Errors
/// Malformed `tenor` / `brokenDate` body, as a [`CodecError`].
pub fn decode_tenor(v: &Value) -> DResult<Tenor> {
    decode(Tenor::MESSAGE, obj(v, "tenor")?)
}

/// Decode a WS `conventions` body into a [`Conventions`] (mirrors
/// `conventions_from_json`).
///
/// # Errors
/// Non-object `conventions` body, as a [`CodecError`].
pub fn decode_conventions(v: &Value) -> DResult<Conventions> {
    decode(Conventions::MESSAGE, obj(v, "conventions")?)
}

/// Decode a WS `quantity` body into a [`Quantity`] (mirrors `quantity_from_json`).
///
/// # Errors
/// Non-object `quantity` body, as a [`CodecError`].
pub fn decode_quantity(v: &Value) -> DResult<Quantity> {
    decode(Quantity::MESSAGE, obj(v, "quantity")?)
}

/// Decode a WS `solve` body into a [`Solve`] (mirrors `solve_from_json`).
///
/// # Errors
/// Non-object `solve` body, as a [`CodecError`].
pub fn decode_solve(v: &Value) -> DResult<Solve> {
    decode(Solve::MESSAGE, obj(v, "solve")?)
}

/// Decode a WS `strike` body into a [`StrikeOrDelta`] — the `strike`/`delta` oneof
/// (mirrors `strike_or_delta_from_json`).
///
/// # Errors
/// Neither `strike` nor `delta` present, as a [`CodecError`].
pub fn decode_strike_or_delta(v: &Value) -> DResult<StrikeOrDelta> {
    decode(StrikeOrDelta::MESSAGE, obj(v, "strike")?)
}

/// Decode a WS `vanilla` body into a [`Vanilla`] (mirrors `vanilla_from_json`).
///
/// # Errors
/// Missing `strike`, as a [`CodecError`].
pub fn decode_vanilla(v: &Value) -> DResult<Vanilla> {
    decode(Vanilla::MESSAGE, obj(v, "vanilla")?)
}

/// Decode a WS `strategy` body into a [`Strategy`] — the `kind` + the repeated
/// `legs` array (mirrors `strategy_from_json`).
///
/// # Errors
/// Missing `legs` array, as a [`CodecError`].
pub fn decode_strategy(v: &Value) -> DResult<Strategy> {
    decode(Strategy::MESSAGE, obj(v, "strategy")?)
}

// ---------------------------------------------------------------------------
// per-message reflection builders (mechanical convert + place only)
// ---------------------------------------------------------------------------

impl WireBuilder for CcyPair {
    const MESSAGE: &'static str = "CcyPair";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "base" => self.base = req_string(value, "base")?,
            "quote" => self.quote = req_string(value, "quote")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for MetalPair {
    const MESSAGE: &'static str = "MetalPair";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "metal" => self.metal = enum_or_zero(value),
            "quote" => self.quote = req_string(value, "quote")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Symbol {
    const MESSAGE: &'static str = "Symbol";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "ticker" => self.ticker = req_string(value, "ticker")?,
            "venue" => self.venue = string_or_empty(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for EquityRef {
    const MESSAGE: &'static str = "EquityRef";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "symbol" => self.symbol = Some(req_msg::<Symbol>(value, "symbol")?),
            "currency" => self.currency = req_string(value, "currency")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CommodityRef {
    const MESSAGE: &'static str = "CommodityRef";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "symbol" => self.symbol = Some(req_msg::<Symbol>(value, "symbol")?),
            "currency" => self.currency = req_string(value, "currency")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CryptoPair {
    const MESSAGE: &'static str = "CryptoPair";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "base" => self.base = req_string(value, "base")?,
            "quote" => self.quote = req_string(value, "quote")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Tenor {
    const MESSAGE: &'static str = "Tenor";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "unit" => self.unit = enum_or_zero(value),
            "count" => self.count = u32_or_zero(value),
            // The camelCase `brokenDate` key was resolved by the shared override
            // in `decode`; here it is a plain optional nested message.
            "broken_date" => self.broken_date = opt_msg::<BrokenDate>(value, "brokenDate")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for BrokenDate {
    const MESSAGE: &'static str = "BrokenDate";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "year" => self.year = i32_or_zero(value),
            "month" => self.month = u32_or_zero(value),
            "day" => self.day = u32_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Quantity {
    const MESSAGE: &'static str = "Quantity";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "notional" => self.notional = f64_or_zero(value),
            "base_ccy" => self.base_ccy = bool_or_false(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Solve {
    const MESSAGE: &'static str = "Solve";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "target" => self.target = enum_or_zero(value),
            "target_premium" => self.target_premium = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Conventions {
    const MESSAGE: &'static str = "Conventions";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "delta_convention" => self.delta_convention = enum_or_zero(value),
            "atm_convention" => self.atm_convention = enum_or_zero(value),
            "premium_style" => self.premium_style = enum_or_zero(value),
            "cut" => self.cut = enum_or_zero(value),
            "day_count" => self.day_count = enum_or_zero(value),
            "settlement" => self.settlement = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for StrikeOrDelta {
    const MESSAGE: &'static str = "StrikeOrDelta";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        use strike_or_delta::Spec;
        // `decode` calls `set` only for the single live oneof arm it selected.
        match field.proto_name {
            "strike" => self.spec = Some(Spec::Strike(req_f64(value, "strike")?)),
            "delta" => self.spec = Some(Spec::Delta(req_f64(value, "delta")?)),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Vanilla {
    const MESSAGE: &'static str = "Vanilla";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = Some(req_msg::<StrikeOrDelta>(value, "strike")?),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Leg {
    const MESSAGE: &'static str = "Leg";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = Some(req_msg::<StrikeOrDelta>(value, "strike")?),
            "side" => self.side = enum_or_zero(value),
            "ratio" => self.ratio = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Strategy {
    const MESSAGE: &'static str = "Strategy";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "kind" => self.kind = enum_or_zero(value),
            "legs" => self.legs = req_repeated::<Leg>(value, "leg")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

/// A field present in the descriptor table but not handled by a builder — a
/// contract change the codec has not caught up with. The `MESSAGE_FIELDS`
/// alignment tests + the differential harness fail loudly if this is ever hit, so
/// the generated decoder can never silently drop a newly-added field.
fn unhandled(message: &str, field: &str) -> CodecError {
    CodecError(format!(
        "generated decoder for `{message}` has no builder arm for field `{field}` \
         (contract drifted — update the WireBuilder)"
    ))
}

// ===========================================================================
// DECODE — the Instrument-consuming Price family (arch item G — increment 4)
// ===========================================================================
//
// The full request-side surface the unified one-shot `Price(Instrument)` RPC
// decodes: the `Instrument` message + its 24-arm `product` oneof, and the
// `PriceRequest` / `RatesPriceRequest` / `PriceXvaRequest` envelopes. Each product
// body (`SingleBarrier`, `AsianOption`, `BasketOption`, …) and the rates/xva
// sub-messages decode through the generic table-driven [`decode`] via their
// [`WireBuilder`] — the descriptor drives the field set and the JSON key; the
// builder only carries the per-field presence policy the proto3 descriptor cannot
// express (required-erroring vs proto3-default vs `Option`, and the exotic
// `u32`-range-checked casts). The two FX-legacy trees that the field table cannot
// express — the `Instrument.underlying` dual-key (legacy `pair` vs the richer
// `underlying` oneof) and the FX-legacy `MarketContext` inside `PriceRequest` —
// keep the message-level projection escape hatch established in increment 3
// ([`decode_instrument`] / [`decode_price_request`]); the pure rates & XVA trees
// carry no FX-legacy divergence and decode fully generically.

/// A required nested value by key (mirrors the hand `nested`): error on an absent
/// key, else hand the raw value (including `null`, which the nested decoder then
/// rejects as a non-object — byte-identical to the hand path).
fn req_value<'a>(value: Option<&'a Value>, what: &str) -> DResult<&'a Value> {
    value.ok_or_else(|| CodecError(format!("missing nested field `{what}`")))
}

/// Decode a nested product body `{ "<key>": { .. } }` into its concrete `T` through
/// the generic table-driven decoder — the input-side mirror of the encoder's nested
/// recursion, subsuming the 24 hand `*_from_json` product decoders.
fn body<T: WireBuilder>(v: &Value, key: &str) -> DResult<T> {
    decode(T::MESSAGE, obj(v, key)?)
}

/// Decode an [`Instrument`] from its WS JSON object (mirrors the hand
/// `instrument_from_json`) — a **message-level projection**: the `underlying`
/// dual-key (legacy FX `pair` vs the richer cross-asset `underlying` oneof, quirk a)
/// and the `product` 24-arm oneof are not expressible as a flat field-table walk, so
/// they are resolved here, delegating every leaf/product body to the generated
/// generic decoders. `v` is the instrument object itself.
///
/// # Errors
/// Malformed body, a missing required field (`expiry_years`), or an absent/unknown
/// `product` arm, as a [`CodecError`].
pub fn decode_instrument(v: &Value) -> DResult<Instrument> {
    let o = obj(v, "instrument")?;
    Ok(Instrument {
        underlying: instrument_underlying(o)?,
        tenor: opt_msg(o.get("tenor").filter(|v| !v.is_null()), "tenor")?,
        expiry_years: req_f64(o.get("expiry_years"), "expiry_years")?,
        quantity: opt_msg(o.get("quantity").filter(|v| !v.is_null()), "quantity")?,
        side: enum_or_zero(o.get("side")),
        solve: opt_msg(o.get("solve").filter(|v| !v.is_null()), "solve")?,
        pricing_model: enum_or_zero(o.get("pricing_model")),
        settlement_style: enum_or_zero(o.get("settlement_style")),
        product: Some(decode_product(o)?),
    })
}

/// The `Instrument.underlying` dual-key resolution (mirrors the hand
/// `instrument_underlying_from_json`): the richer cross-asset `underlying` oneof is
/// authoritative when present; otherwise the legacy FX `pair` projection; a frame
/// carrying neither yields `None`.
fn instrument_underlying(o: &Map<String, Value>) -> DResult<Option<Underlying>> {
    if let Some(v) = o.get("underlying").filter(|v| !v.is_null()) {
        return decode_underlying_object(v).map(Some);
    }
    match o.get("pair") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(Underlying::fx(decode_ccy_pair(v)?))),
    }
}

/// Decode the `product` oneof (mirrors the hand `product_from_json`): the JSON
/// carries exactly one product key, whose body decodes through the generic
/// table-driven [`decode`]. First present arm wins, in the hand codec's declaration
/// order.
fn decode_product(o: &Map<String, Value>) -> DResult<instrument::Product> {
    use instrument::Product;
    if let Some(v) = o.get("vanilla") {
        Ok(Product::Vanilla(body(v, "vanilla")?))
    } else if let Some(v) = o.get("strategy") {
        Ok(Product::Strategy(body(v, "strategy")?))
    } else if let Some(v) = o.get("single_barrier") {
        Ok(Product::SingleBarrier(body(v, "single_barrier")?))
    } else if let Some(v) = o.get("double_barrier") {
        Ok(Product::DoubleBarrier(body(v, "double_barrier")?))
    } else if let Some(v) = o.get("digital") {
        Ok(Product::Digital(body(v, "digital")?))
    } else if let Some(v) = o.get("touch") {
        Ok(Product::Touch(body(v, "touch")?))
    } else if let Some(v) = o.get("variance_swap") {
        Ok(Product::VarianceSwap(body(v, "variance_swap")?))
    } else if let Some(v) = o.get("volatility_swap") {
        Ok(Product::VolatilitySwap(body(v, "volatility_swap")?))
    } else if let Some(v) = o.get("asian_option") {
        Ok(Product::AsianOption(body(v, "asian_option")?))
    } else if let Some(v) = o.get("forward_start") {
        Ok(Product::ForwardStart(body(v, "forward_start")?))
    } else if let Some(v) = o.get("cliquet") {
        Ok(Product::Cliquet(body(v, "cliquet")?))
    } else if let Some(v) = o.get("quanto") {
        Ok(Product::Quanto(body(v, "quanto")?))
    } else if let Some(v) = o.get("tarf") {
        Ok(Product::Tarf(body(v, "tarf")?))
    } else if let Some(v) = o.get("pivot") {
        Ok(Product::Pivot(body(v, "pivot")?))
    } else if let Some(v) = o.get("accumulator") {
        Ok(Product::Accumulator(body(v, "accumulator")?))
    } else if let Some(v) = o.get("lookback") {
        Ok(Product::Lookback(body(v, "lookback")?))
    } else if let Some(v) = o.get("window_barrier") {
        Ok(Product::WindowBarrier(body(v, "window_barrier")?))
    } else if let Some(v) = o.get("american") {
        Ok(Product::American(body(v, "american")?))
    } else if let Some(v) = o.get("basket") {
        Ok(Product::Basket(body(v, "basket")?))
    } else if let Some(v) = o.get("fx_forward") {
        Ok(Product::FxForward(body(v, "fx_forward")?))
    } else if let Some(v) = o.get("fx_swap") {
        Ok(Product::FxSwap(body(v, "fx_swap")?))
    } else if let Some(v) = o.get("ndf") {
        Ok(Product::Ndf(body(v, "ndf")?))
    } else if let Some(v) = o.get("perpetual_option") {
        Ok(Product::PerpetualOption(body(v, "perpetual_option")?))
    } else if let Some(v) = o.get("listed_future_option") {
        Ok(Product::ListedFutureOption(body(
            v,
            "listed_future_option",
        )?))
    } else {
        Err(CodecError(
            "instrument needs exactly one product (vanilla / strategy / \
             single_barrier / double_barrier / digital / touch / variance_swap / \
             volatility_swap / asian_option / forward_start / cliquet / quanto / \
             tarf / pivot / accumulator / lookback / window_barrier / american / \
             basket / fx_forward / fx_swap / ndf / perpetual_option / \
             listed_future_option)"
                .to_owned(),
        ))
    }
}

/// Decode a [`PriceRequest`] envelope from its already-unwrapped WS JSON object
/// (mirrors the hand `price_request_from_json`) — a **message-level projection**
/// because it nests the FX-legacy [`Instrument`] and [`MarketContext`] projections.
///
/// # Errors
/// A missing required nested field (`instrument` / `market` / `conventions`) or a
/// malformed body, as a [`CodecError`].
pub fn decode_price_request(o: &Map<String, Value>) -> DResult<PriceRequest> {
    Ok(PriceRequest {
        request_id: u64_or_zero(o.get("request_id")),
        instrument: Some(decode_instrument(req_value(
            o.get("instrument"),
            "instrument",
        )?)?),
        market: Some(decode_market_context(req_value(
            o.get("market"),
            "market",
        )?)?),
        conventions: Some(decode_conventions(req_value(
            o.get("conventions"),
            "conventions",
        )?)?),
        correlation_id: opt_u64(o.get("correlation_id").filter(|v| !v.is_null())),
        surface_version: opt_u64(o.get("surface_version").filter(|v| !v.is_null())),
    })
}

/// Decode a [`RatesPriceRequest`] envelope (mirrors the hand
/// `rates_price_request_from_json`). The pure rates tree carries no FX-legacy
/// divergence, so it decodes fully generically through the descriptor.
///
/// # Errors
/// A missing required nested field (`curve_set` / `instrument`) or a malformed body.
pub fn decode_rates_price_request(o: &Map<String, Value>) -> DResult<RatesPriceRequest> {
    decode(RatesPriceRequest::MESSAGE, o)
}

/// Decode a [`RatesQuoteRequest`] envelope (the fixed-income taker RFQ of
/// `QuoteService.RequestRatesQuote`). The pure rates tree carries no FX-legacy
/// divergence, so it decodes fully generically through the descriptor — the
/// `curve_set` / `instrument` reuse the SAME nested `CurveSet` / `RatesInstrument`
/// builders the outright `PriceRates` request decode does.
///
/// # Errors
/// A missing required nested field (`curve_set` / `instrument`, or the required
/// `RatesInstrument` arm) or a malformed body.
pub fn decode_rates_quote_request(o: &Map<String, Value>) -> DResult<RatesQuoteRequest> {
    decode(RatesQuoteRequest::MESSAGE, o)
}

/// Decode a [`PriceXvaRequest`] envelope (mirrors the hand
/// `price_xva_request_from_json`). The pure XVA tree decodes fully generically.
///
/// # Errors
/// A missing required field (`trades` / the market scalars / `counterparty` / `own`)
/// or a malformed body.
pub fn decode_price_xva_request(o: &Map<String, Value>) -> DResult<PriceXvaRequest> {
    decode(PriceXvaRequest::MESSAGE, o)
}

// --- product WireBuilders (mechanical convert + place; the presence policy) ---

impl WireBuilder for SingleBarrier {
    const MESSAGE: &'static str = "SingleBarrier";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "vanilla" => self.vanilla = Some(req_msg::<Vanilla>(value, "vanilla")?),
            "kind" => self.kind = enum_or_zero(value),
            "side" => self.side = enum_or_zero(value),
            "barrier" => self.barrier = req_f64(value, "barrier")?,
            "rebate" => self.rebate = f64_or_zero(value),
            "monitoring" => self.monitoring = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DoubleBarrier {
    const MESSAGE: &'static str = "DoubleBarrier";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "vanilla" => self.vanilla = Some(req_msg::<Vanilla>(value, "vanilla")?),
            "kind" => self.kind = enum_or_zero(value),
            "lower_barrier" => self.lower_barrier = req_f64(value, "lower_barrier")?,
            "upper_barrier" => self.upper_barrier = req_f64(value, "upper_barrier")?,
            "rebate" => self.rebate = f64_or_zero(value),
            "monitoring" => self.monitoring = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Digital {
    const MESSAGE: &'static str = "Digital";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "style" => self.style = enum_or_zero(value),
            "payout" => self.payout = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Touch {
    const MESSAGE: &'static str = "Touch";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "kind" => self.kind = enum_or_zero(value),
            "lower_barrier" => self.lower_barrier = req_f64(value, "lower_barrier")?,
            "upper_barrier" => self.upper_barrier = f64_or_zero(value),
            "rebate" => self.rebate = f64_or_zero(value),
            "monitoring" => self.monitoring = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for VarianceSwap {
    const MESSAGE: &'static str = "VarianceSwap";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "strike_vol" => self.strike_vol = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for VolatilitySwap {
    const MESSAGE: &'static str = "VolatilitySwap";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "strike_vol" => self.strike_vol = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for AsianOption {
    const MESSAGE: &'static str = "AsianOption";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "averaging" => self.averaging = enum_or_zero(value),
            "observations" => self.observations = u32_ranged(value, "asian_option.observations")?,
            "method" => self.method = enum_or_zero(value),
            "elapsed_avg" => self.elapsed_avg = f64_or_zero(value),
            "elapsed_weight" => self.elapsed_weight = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ForwardStart {
    const MESSAGE: &'static str = "ForwardStart";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "moneyness" => self.moneyness = req_f64(value, "moneyness")?,
            "reset" => self.reset = req_f64(value, "reset")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Cliquet {
    const MESSAGE: &'static str = "Cliquet";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "moneyness" => self.moneyness = req_f64(value, "moneyness")?,
            "periods" => self.periods = u32_ranged(value, "cliquet.periods")?,
            "local_floor" => self.local_floor = opt_f64(value),
            "local_cap" => self.local_cap = opt_f64(value),
            "global_floor" => self.global_floor = opt_f64(value),
            "global_cap" => self.global_cap = opt_f64(value),
            "mc_pairs" => self.mc_pairs = u32_ranged(value, "cliquet.mc_pairs")?,
            "mc_seed" => self.mc_seed = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Quanto {
    const MESSAGE: &'static str = "Quanto";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "payoff" => self.payoff = enum_or_zero(value),
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "conversion_vol" => self.conversion_vol = f64_or_zero(value),
            "correlation" => self.correlation = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for FixingSchedule {
    const MESSAGE: &'static str = "FixingSchedule";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "fixing_years" => self.fixing_years = f64_vec(value),
            "fixing_notional" => self.fixing_notional = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Tarf {
    const MESSAGE: &'static str = "Tarf";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "target" => self.target = req_f64(value, "target")?,
            "leverage" => self.leverage = f64_or_zero(value),
            "redemption" => self.redemption = enum_or_zero(value),
            "schedule" => self.schedule = opt_msg::<FixingSchedule>(value, "schedule")?,
            "mc_pairs" => self.mc_pairs = u32_ranged(value, "tarf.mc_pairs")?,
            "mc_seed" => self.mc_seed = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Pivot {
    const MESSAGE: &'static str = "Pivot";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "pivot" => self.pivot = req_f64(value, "pivot")?,
            "target" => self.target = req_f64(value, "target")?,
            "leverage" => self.leverage = f64_or_zero(value),
            "redemption" => self.redemption = enum_or_zero(value),
            "schedule" => self.schedule = opt_msg::<FixingSchedule>(value, "schedule")?,
            "mc_pairs" => self.mc_pairs = u32_ranged(value, "pivot.mc_pairs")?,
            "mc_seed" => self.mc_seed = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Accumulator {
    const MESSAGE: &'static str = "Accumulator";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "pivot" => self.pivot = req_f64(value, "pivot")?,
            "barrier" => self.barrier = req_f64(value, "barrier")?,
            "leverage" => self.leverage = f64_or_zero(value),
            "monitoring" => self.monitoring = enum_or_zero(value),
            "schedule" => self.schedule = opt_msg::<FixingSchedule>(value, "schedule")?,
            "mc_pairs" => self.mc_pairs = u32_ranged(value, "accumulator.mc_pairs")?,
            "mc_seed" => self.mc_seed = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Lookback {
    const MESSAGE: &'static str = "Lookback";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "style" => self.style = enum_or_zero(value),
            "option_type" => self.option_type = enum_or_zero(value),
            "monitoring" => self.monitoring = enum_or_zero(value),
            "strike" => self.strike = f64_or_zero(value),
            "observations" => self.observations = u32_ranged(value, "lookback.observations")?,
            "mc_pairs" => self.mc_pairs = u32_ranged(value, "lookback.mc_pairs")?,
            "mc_seed" => self.mc_seed = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for WindowBarrier {
    const MESSAGE: &'static str = "WindowBarrier";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "vanilla" => self.vanilla = Some(req_msg::<Vanilla>(value, "vanilla")?),
            "barrier" => self.barrier = req_f64(value, "barrier")?,
            "side" => self.side = enum_or_zero(value),
            "window_start" => self.window_start = req_f64(value, "window_start")?,
            "window_end" => self.window_end = req_f64(value, "window_end")?,
            "mc_pairs" => self.mc_pairs = u32_ranged(value, "window_barrier.mc_pairs")?,
            "mc_steps" => self.mc_steps = u32_ranged(value, "window_barrier.mc_steps")?,
            "mc_seed" => self.mc_seed = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for BasketLeg {
    const MESSAGE: &'static str = "BasketLeg";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            // The leg underlying rides under the legacy `pair` key (resolved by the
            // override), decoded via the FX `{base, quote}` projection.
            "underlying" => {
                self.underlying = match value {
                    Some(v) => Some(Underlying::fx(decode_ccy_pair(v)?)),
                    None => None,
                };
            }
            "weight" => self.weight = req_f64(value, "weight")?,
            "spot" => self.spot = req_f64(value, "spot")?,
            "vol" => self.vol = req_f64(value, "vol")?,
            "r_for" => self.r_for = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for BasketOption {
    const MESSAGE: &'static str = "BasketOption";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "legs" => self.legs = req_repeated::<BasketLeg>(value, "basket leg")?,
            "correlations" => self.correlations = f64_vec(value),
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "kind" => self.kind = enum_or_zero(value),
            "mc_paths" => self.mc_paths = u32_ranged(value, "basket.mc_paths")?,
            "mc_replications" => {
                self.mc_replications = u32_ranged(value, "basket.mc_replications")?;
            }
            "mc_steps" => self.mc_steps = u32_ranged(value, "basket.mc_steps")?,
            "mc_seed" => self.mc_seed = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for AmericanOption {
    const MESSAGE: &'static str = "AmericanOption";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "exercise_style" => self.exercise_style = enum_or_zero(value),
            "bermudan_dates" => {
                self.bermudan_dates = f64_array(value, "american.bermudan_dates")?;
            }
            "lsm_paths" => self.lsm_paths = u32_ranged(value, "american.lsm_paths")?,
            "lsm_exercise_dates" => {
                self.lsm_exercise_dates = u32_ranged(value, "american.lsm_exercise_dates")?;
            }
            "lsm_seed" => self.lsm_seed = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for FxForward {
    const MESSAGE: &'static str = "FxForward";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "contract_rate" => self.contract_rate = req_f64(value, "contract_rate")?,
            "notional" => self.notional = req_f64(value, "notional")?,
            "side" => self.side = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for FxSwap {
    const MESSAGE: &'static str = "FxSwap";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "near" => self.near = Some(req_msg::<FxForward>(value, "near")?),
            "far" => self.far = Some(req_msg::<FxForward>(value, "far")?),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for Ndf {
    const MESSAGE: &'static str = "Ndf";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "contract_rate" => self.contract_rate = req_f64(value, "contract_rate")?,
            "notional" => self.notional = req_f64(value, "notional")?,
            "side" => self.side = enum_or_zero(value),
            "fixing" => self.fixing = enum_or_zero(value),
            "settlement_ccy" => self.settlement_ccy = string_or_empty(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for PerpetualOption {
    const MESSAGE: &'static str = "PerpetualOption";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "notional" => self.notional = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListedFutureOption {
    const MESSAGE: &'static str = "ListedFutureOption";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "future_symbol" => {
                self.future_symbol = Some(req_msg::<Symbol>(value, "future_symbol")?);
            }
            "future_expiry_years" => {
                self.future_expiry_years = req_f64(value, "future_expiry_years")?;
            }
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "notional" => self.notional = f64_or_zero(value),
            "margining" => self.margining = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- linear-rates WireBuilders (the pure rates tree — fully generic) ---------

impl WireBuilder for OisInstrument {
    const MESSAGE: &'static str = "OisInstrument";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor_years" => self.tenor_years = req_u32(value, "tenor_years")?,
            "fixed_rate" => self.fixed_rate = req_f64(value, "fixed_rate")?,
            "notional" => self.notional = req_f64(value, "notional")?,
            "side" => self.side = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for VanillaIrsInstrument {
    const MESSAGE: &'static str = "VanillaIrsInstrument";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor_years" => self.tenor_years = req_u32(value, "tenor_years")?,
            "fixed_rate" => self.fixed_rate = req_f64(value, "fixed_rate")?,
            "notional" => self.notional = req_f64(value, "notional")?,
            "side" => self.side = enum_or_zero(value),
            "fixed_frequency" => self.fixed_frequency = enum_or_zero(value),
            "fixed_day_count" => self.fixed_day_count = enum_or_zero(value),
            "float_frequency" => self.float_frequency = enum_or_zero(value),
            "float_day_count" => self.float_day_count = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for FraInstrument {
    const MESSAGE: &'static str = "FraInstrument";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "start_months" => self.start_months = req_u32(value, "start_months")?,
            "end_months" => self.end_months = req_u32(value, "end_months")?,
            "fixed_rate" => self.fixed_rate = req_f64(value, "fixed_rate")?,
            "notional" => self.notional = req_f64(value, "notional")?,
            "side" => self.side = enum_or_zero(value),
            "accrual_basis" => self.accrual_basis = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for BondInstrument {
    const MESSAGE: &'static str = "BondInstrument";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "coupon_rate" => self.coupon_rate = req_f64(value, "coupon_rate")?,
            "coupon_frequency" => self.coupon_frequency = enum_or_zero(value),
            "day_count" => self.day_count = enum_or_zero(value),
            "maturity_date" => {
                self.maturity_date = Some(req_msg::<BrokenDate>(value, "maturity_date")?);
            }
            "redemption" => self.redemption = req_f64(value, "redemption")?,
            "side" => self.side = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for RatesInstrument {
    const MESSAGE: &'static str = "RatesInstrument";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        use rates_instrument::Instrument;
        // `decode` calls `set` only for the single live oneof arm it selected.
        match field.proto_name {
            "ois" => {
                self.instrument = Some(Instrument::Ois(req_msg::<OisInstrument>(value, "ois")?))
            }
            "irs" => {
                self.instrument = Some(Instrument::Irs(req_msg::<VanillaIrsInstrument>(
                    value, "irs",
                )?))
            }
            "fra" => {
                self.instrument = Some(Instrument::Fra(req_msg::<FraInstrument>(value, "fra")?))
            }
            "bond" => {
                self.instrument = Some(Instrument::Bond(req_msg::<BondInstrument>(value, "bond")?))
            }
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for PillarTenor {
    const MESSAGE: &'static str = "PillarTenor";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        use pillar_tenor::Point;
        match field.proto_name {
            "years" => self.point = Some(Point::Years(req_u32(value, "years")?)),
            "months" => self.point = Some(Point::Months(req_u32(value, "months")?)),
            "maturity_date" => {
                self.point = Some(Point::MaturityDate(req_msg::<BrokenDate>(
                    value,
                    "maturity_date",
                )?));
            }
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for OisPillar {
    const MESSAGE: &'static str = "OisPillar";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor" => self.tenor = Some(req_msg::<PillarTenor>(value, "tenor")?),
            "par_rate" => self.par_rate = req_f64(value, "par_rate")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CurveSet {
    const MESSAGE: &'static str = "CurveSet";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "currency" => self.currency = req_string(value, "currency")?,
            "reference_date" => {
                self.reference_date = Some(req_msg::<BrokenDate>(value, "reference_date")?);
            }
            "ois_pillars" => self.ois_pillars = req_repeated::<OisPillar>(value, "ois_pillar")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- curve-query request WireBuilders (decode; SurfaceService GetCurve / MarkCurve /
//     CurveScenario). The curve source (`curve_set`) is decoded optionally — the
//     handler enforces the required-vs-pinned rule — and the shift/tenor axes are
//     proto3-default repeated doubles. ----------------------------------------------

impl WireBuilder for GetCurveRequest {
    const MESSAGE: &'static str = "GetCurveRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "curve_set" => self.curve_set = opt_msg::<CurveSet>(value, "curve_set")?,
            "query_tenor_years" => self.query_tenor_years = f64_vec(value),
            "curve_version" => self.curve_version = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for MarkCurveRequest {
    const MESSAGE: &'static str = "MarkCurveRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "curve_set" => self.curve_set = opt_msg::<CurveSet>(value, "curve_set")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CurveScenarioRequest {
    const MESSAGE: &'static str = "CurveScenarioRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "curve_set" => self.curve_set = opt_msg::<CurveSet>(value, "curve_set")?,
            "parallel_shift_bp" => self.parallel_shift_bp = f64_or_zero(value),
            "key_rate_shift_bp" => self.key_rate_shift_bp = f64_vec(value),
            "query_tenor_years" => self.query_tenor_years = f64_vec(value),
            "instrument" => self.instrument = opt_msg::<RatesInstrument>(value, "instrument")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for RatesPriceRequest {
    const MESSAGE: &'static str = "RatesPriceRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "request_id" => self.request_id = u64_or_zero(value),
            "curve_set" => self.curve_set = Some(req_msg::<CurveSet>(value, "curve_set")?),
            "instrument" => {
                self.instrument = Some(req_msg::<RatesInstrument>(value, "instrument")?);
            }
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for RatesQuoteRequest {
    const MESSAGE: &'static str = "RatesQuoteRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "idempotency_key" => self.idempotency_key = string_or_empty(value),
            "curve_set" => self.curve_set = Some(req_msg::<CurveSet>(value, "curve_set")?),
            "instrument" => {
                self.instrument = Some(req_msg::<RatesInstrument>(value, "instrument")?);
            }
            "notional" => self.notional = f64_or_zero(value),
            "side" => self.side = enum_or_zero(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- XVA WireBuilders (the pure valuation-adjustment tree — fully generic) ----

impl WireBuilder for XvaTrade {
    const MESSAGE: &'static str = "XvaTrade";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_type" => self.option_type = enum_or_zero(value),
            "strike" => self.strike = req_f64(value, "strike")?,
            "expiry_years" => self.expiry_years = req_f64(value, "expiry_years")?,
            "vol" => self.vol = req_f64(value, "vol")?,
            "notional" => self.notional = req_f64(value, "notional")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for XvaSurvivalCurve {
    const MESSAGE: &'static str = "XvaSurvivalCurve";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "pillar_times" => self.pillar_times = f64_array(value, "pillar_times")?,
            "hazard_rates" => self.hazard_rates = f64_array(value, "hazard_rates")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for PriceXvaRequest {
    const MESSAGE: &'static str = "PriceXvaRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "request_id" => self.request_id = u64_or_zero(value),
            "trades" => self.trades = req_repeated::<XvaTrade>(value, "trade")?,
            "r_dom" => self.r_dom = req_f64(value, "r_dom")?,
            "r_for" => self.r_for = req_f64(value, "r_for")?,
            "spot0" => self.spot0 = req_f64(value, "spot0")?,
            "sigma" => self.sigma = req_f64(value, "sigma")?,
            "paths" => self.paths = req_u32(value, "paths")?,
            "seed" => self.seed = u64_or_zero(value),
            "exposure_steps" => self.exposure_steps = req_u32(value, "exposure_steps")?,
            "counterparty" => {
                self.counterparty = Some(req_msg::<XvaSurvivalCurve>(value, "counterparty")?);
            }
            "own" => self.own = Some(req_msg::<XvaSurvivalCurve>(value, "own")?),
            "lgd_counterparty" => self.lgd_counterparty = req_f64(value, "lgd_counterparty")?,
            "lgd_own" => self.lgd_own = req_f64(value, "lgd_own")?,
            "funding_spread" => self.funding_spread = req_f64(value, "funding_spread")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// ===========================================================================
// CombinedTailRisk — the C2c unified options+FI joint tail (RiskService)
// ===========================================================================
//
// The RiskService.CombinedTailRisk request/response run fully on this
// descriptor-driven codec: the messages carry NO FX-legacy quirk (flat GK inputs,
// snake_case keys, one plain oneof), so both directions decode/encode purely from
// the field tables via the generic [`decode`] / [`encode`] walk — no message-level
// projection and only the one curated `oneof_required` entry
// (`TailRiskFiPosition.position`). Both traits are implemented for every message so
// the round-trip harness (`tests/ws_codec_differential.rs`) can prove
// `decode(encode(v)) == v` byte-stable in each direction.

/// Decode a [`CombinedTailRiskRequest`] envelope from its already-unwrapped WS JSON
/// object — fully generic (no FX-legacy divergence).
///
/// # Errors
/// A malformed leg / scenario / curve body, a missing required scalar, or a
/// `TailRiskFiPosition` carrying no `position` arm, as a [`CodecError`].
pub fn decode_combined_tail_risk_request(
    o: &Map<String, Value>,
) -> DResult<CombinedTailRiskRequest> {
    decode(CombinedTailRiskRequest::MESSAGE, o)
}

/// Encode a [`CombinedTailRiskResponse`] to its WS JSON — the joint VaR/ES, the FI
/// key-rate ladder, the signed parallel DV01 and the presence-tracked
/// `correlation_id`, descriptor-driven.
#[must_use]
pub fn encode_combined_tail_risk_response(r: &CombinedTailRiskResponse) -> Value {
    encode("CombinedTailRiskResponse", r)
}

/// Encode a [`CombinedTailRiskRequest`] to its WS JSON — the symmetric counterpart of
/// [`decode_combined_tail_risk_request`] (the WS mirror's second encoding of the one
/// contract in the request direction, e.g. for request logging / replay), and the
/// encode half the round-trip harness proves stable.
#[must_use]
pub fn encode_combined_tail_risk_request(r: &CombinedTailRiskRequest) -> Value {
    encode("CombinedTailRiskRequest", r)
}

/// Decode a [`CombinedTailRiskResponse`] from its WS JSON object — the symmetric
/// counterpart of [`encode_combined_tail_risk_response`] (the WS mirror's second
/// encoding of the one contract in the response direction, e.g. a client-side read),
/// and the decode half the round-trip harness proves stable.
///
/// # Errors
/// A malformed key-rate / VaR-ES body or a non-numeric scalar, as a [`CodecError`].
pub fn decode_combined_tail_risk_response(
    o: &Map<String, Value>,
) -> DResult<CombinedTailRiskResponse> {
    decode(CombinedTailRiskResponse::MESSAGE, o)
}

// --- request-side WireBuilders (decode: convert + place; the presence policy) ---

impl WireBuilder for CombinedTailRiskRequest {
    const MESSAGE: &'static str = "CombinedTailRiskRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "option_legs" => {
                self.option_legs = opt_repeated::<TailRiskOptionLeg>(value, "option_legs")?;
            }
            "fi_positions" => {
                self.fi_positions = opt_repeated::<TailRiskFiPosition>(value, "fi_positions")?;
            }
            "base_curve" => {
                self.base_curve = opt_repeated::<TailRiskCurvePillar>(value, "base_curve")?;
            }
            "scenarios" => {
                self.scenarios = opt_repeated::<JointTailScenario>(value, "scenarios")?;
            }
            "alpha" => self.alpha = f64_or_zero(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for TailRiskOptionLeg {
    const MESSAGE: &'static str = "TailRiskOptionLeg";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "pair" => self.pair = Some(req_msg::<CcyPair>(value, "pair")?),
            "option_type" => self.option_type = enum_or_zero(value),
            "notional_base" => self.notional_base = f64_or_zero(value),
            "spot" => self.spot = req_f64(value, "spot")?,
            "strike" => self.strike = req_f64(value, "strike")?,
            "vol" => self.vol = req_f64(value, "vol")?,
            "t" => self.t = req_f64(value, "t")?,
            "r_dom" => self.r_dom = f64_or_zero(value),
            "r_for" => self.r_for = f64_or_zero(value),
            "quoted_delta" => self.quoted_delta = enum_or_zero(value),
            "premium_style" => self.premium_style = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for TailRiskFiPosition {
    const MESSAGE: &'static str = "TailRiskFiPosition";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "ois_swap" => {
                self.position = Some(tail_risk_fi_position::Position::OisSwap(req_msg::<
                    OisSwapLeg,
                >(
                    value, "ois_swap",
                )?));
            }
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for OisSwapLeg {
    const MESSAGE: &'static str = "OisSwapLeg";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "start" => self.start = req_f64(value, "start")?,
            "periods" => self.periods = opt_repeated::<OisFixedPeriod>(value, "periods")?,
            "fixed_rate" => self.fixed_rate = req_f64(value, "fixed_rate")?,
            "notional" => self.notional = req_f64(value, "notional")?,
            "receive_fixed" => self.receive_fixed = bool_or_false(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for OisFixedPeriod {
    const MESSAGE: &'static str = "OisFixedPeriod";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "pay" => self.pay = req_f64(value, "pay")?,
            "accrual" => self.accrual = req_f64(value, "accrual")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for TailRiskCurvePillar {
    const MESSAGE: &'static str = "TailRiskCurvePillar";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "t" => self.t = req_f64(value, "t")?,
            "zero_rate" => self.zero_rate = req_f64(value, "zero_rate")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for JointTailScenario {
    const MESSAGE: &'static str = "JointTailScenario";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "spot_rel" => self.spot_rel = f64_or_zero(value),
            "vol_abs" => self.vol_abs = f64_or_zero(value),
            "discount_abs" => self.discount_abs = f64_or_zero(value),
            "carry_abs" => self.carry_abs = f64_or_zero(value),
            "rate_shifts" => self.rate_shifts = f64_array(value, "rate_shifts")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- response-side WireBuilders (decode; exercised by the round-trip harness) ---

impl WireBuilder for CombinedTailRiskResponse {
    const MESSAGE: &'static str = "CombinedTailRiskResponse";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "joint_var_es" => self.joint_var_es = opt_msg::<VarEs>(value, "joint_var_es")?,
            "key_rate" => self.key_rate = opt_repeated::<TailRiskKeyRate>(value, "key_rate")?,
            "fi_parallel_dv01" => self.fi_parallel_dv01 = f64_or_zero(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for VarEs {
    const MESSAGE: &'static str = "VarEs";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "var" => self.var = req_f64(value, "var")?,
            "es" => self.es = req_f64(value, "es")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for TailRiskKeyRate {
    const MESSAGE: &'static str = "TailRiskKeyRate";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor_years" => self.tenor_years = req_f64(value, "tenor_years")?,
            "dv01" => self.dv01 = req_f64(value, "dv01")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- reflection adapters (encode: mechanical field access only) ---

impl WireAdapter for CombinedTailRiskRequest {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "option_legs" => Some(WireVal::RepeatedMsg(
                self.option_legs
                    .iter()
                    .map(|x| x as &dyn WireAdapter)
                    .collect(),
            )),
            "fi_positions" => Some(WireVal::RepeatedMsg(
                self.fi_positions
                    .iter()
                    .map(|x| x as &dyn WireAdapter)
                    .collect(),
            )),
            "base_curve" => Some(WireVal::RepeatedMsg(
                self.base_curve
                    .iter()
                    .map(|x| x as &dyn WireAdapter)
                    .collect(),
            )),
            "scenarios" => Some(WireVal::RepeatedMsg(
                self.scenarios
                    .iter()
                    .map(|x| x as &dyn WireAdapter)
                    .collect(),
            )),
            "alpha" => Some(WireVal::F64(self.alpha)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            "session_token" => self.session_token.as_deref().map(WireVal::Str),
            _ => None,
        }
    }
}

impl WireAdapter for TailRiskOptionLeg {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "pair" => self
                .pair
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            "option_type" => Some(WireVal::Enum(self.option_type)),
            "notional_base" => Some(WireVal::F64(self.notional_base)),
            "spot" => Some(WireVal::F64(self.spot)),
            "strike" => Some(WireVal::F64(self.strike)),
            "vol" => Some(WireVal::F64(self.vol)),
            "t" => Some(WireVal::F64(self.t)),
            "r_dom" => Some(WireVal::F64(self.r_dom)),
            "r_for" => Some(WireVal::F64(self.r_for)),
            "quoted_delta" => Some(WireVal::Enum(self.quoted_delta)),
            "premium_style" => Some(WireVal::Enum(self.premium_style)),
            _ => None,
        }
    }
}

impl WireAdapter for TailRiskFiPosition {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match (proto_name, &self.position) {
            ("ois_swap", Some(tail_risk_fi_position::Position::OisSwap(s))) => {
                Some(WireVal::Msg(s as &dyn WireAdapter))
            }
            _ => None,
        }
    }
}

impl WireAdapter for OisSwapLeg {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "start" => Some(WireVal::F64(self.start)),
            "periods" => Some(WireVal::RepeatedMsg(
                self.periods.iter().map(|p| p as &dyn WireAdapter).collect(),
            )),
            "fixed_rate" => Some(WireVal::F64(self.fixed_rate)),
            "notional" => Some(WireVal::F64(self.notional)),
            "receive_fixed" => Some(WireVal::Bool(self.receive_fixed)),
            _ => None,
        }
    }
}

impl WireAdapter for OisFixedPeriod {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "pay" => Some(WireVal::F64(self.pay)),
            "accrual" => Some(WireVal::F64(self.accrual)),
            _ => None,
        }
    }
}

impl WireAdapter for TailRiskCurvePillar {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "t" => Some(WireVal::F64(self.t)),
            "zero_rate" => Some(WireVal::F64(self.zero_rate)),
            _ => None,
        }
    }
}

impl WireAdapter for JointTailScenario {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "spot_rel" => Some(WireVal::F64(self.spot_rel)),
            "vol_abs" => Some(WireVal::F64(self.vol_abs)),
            "discount_abs" => Some(WireVal::F64(self.discount_abs)),
            "carry_abs" => Some(WireVal::F64(self.carry_abs)),
            "rate_shifts" => Some(WireVal::RepeatedF64(&self.rate_shifts)),
            _ => None,
        }
    }
}

impl WireAdapter for CombinedTailRiskResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "joint_var_es" => self
                .joint_var_es
                .as_ref()
                .map(|v| WireVal::Msg(v as &dyn WireAdapter)),
            "key_rate" => Some(WireVal::RepeatedMsg(
                self.key_rate
                    .iter()
                    .map(|k| k as &dyn WireAdapter)
                    .collect(),
            )),
            "fi_parallel_dv01" => Some(WireVal::F64(self.fi_parallel_dv01)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for VarEs {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "var" => Some(WireVal::F64(self.var)),
            "es" => Some(WireVal::F64(self.es)),
            _ => None,
        }
    }
}

impl WireAdapter for TailRiskKeyRate {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_years" => Some(WireVal::F64(self.tenor_years)),
            "dv01" => Some(WireVal::F64(self.dv01)),
            _ => None,
        }
    }
}

// ===========================================================================
// FixAdminService — the inbound FIX-acceptor connection administration surface
// ===========================================================================
//
// Six unary verbs (list / create / update / delete / set-enabled connections +
// list captured messages) run fully on this descriptor-driven codec. The requests
// carry the shared admin envelope (`EntitlementPrincipal` principal + `session_token`
// + `correlation_id`) plus a `FixConnectionSpec`; the responses carry a
// `FixConnectionDesc` and a page of `FixMessage`. Every message decodes/encodes
// purely from the field tables via the generic [`decode`] / [`encode`] walk EXCEPT
// `ListFixMessagesRequest`, whose two hand-codec quirks the field table cannot
// express — a whitespace-trimmed `connection_id` and a `limit` that saturates to
// `u32::MAX` on overflow (rather than the generic clamp-to-zero) — so it keeps the
// message-level projection escape hatch ([`decode_list_fix_messages_request`]). The
// admin response messages emit their absent optional `correlation_id` as JSON `null`
// (they are in [`super::codec_overrides::null_absent_optional`]).

// --- shared entitlement-principal WireBuilders (decode; reused by risk + auth) ---

impl WireBuilder for RiskScope {
    const MESSAGE: &'static str = "RiskScope";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "dimension" => self.dimension = enum_or_zero(value),
            "value" => self.value = u64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for EntitlementRule {
    const MESSAGE: &'static str = "EntitlementRule";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "scopes" => self.scopes = opt_repeated::<RiskScope>(value, "scopes")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for EntitlementPrincipal {
    const MESSAGE: &'static str = "EntitlementPrincipal";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "grant_all" => self.grant_all = bool_or_false(value),
            "grants" => self.grants = opt_repeated::<EntitlementRule>(value, "grants")?,
            "denies" => self.denies = opt_repeated::<EntitlementRule>(value, "denies")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- fix-admin request WireBuilders (decode) --------------------------------

impl WireBuilder for FixConnectionSpec {
    const MESSAGE: &'static str = "FixConnectionSpec";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "id" => self.id = string_or_empty(value),
            "name" => self.name = req_string(value, "name")?,
            "kind" => self.kind = enum_or_zero(value),
            "bind_addr" => self.bind_addr = req_string(value, "bind_addr")?,
            "sender_comp_id" => self.sender_comp_id = req_string(value, "sender_comp_id")?,
            "target_comp_id" => self.target_comp_id = req_string(value, "target_comp_id")?,
            "enabled" => self.enabled = bool_or_false(value),
            "desk" => self.desk = string_or_empty(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListFixConnectionsRequest {
    const MESSAGE: &'static str = "ListFixConnectionsRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CreateFixConnectionRequest {
    const MESSAGE: &'static str = "CreateFixConnectionRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "spec" => self.spec = Some(req_msg::<FixConnectionSpec>(value, "spec")?),
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdateFixConnectionRequest {
    const MESSAGE: &'static str = "UpdateFixConnectionRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "id" => self.id = req_string(value, "id")?,
            "spec" => self.spec = Some(req_msg::<FixConnectionSpec>(value, "spec")?),
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeleteFixConnectionRequest {
    const MESSAGE: &'static str = "DeleteFixConnectionRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "id" => self.id = req_string(value, "id")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for SetFixConnectionEnabledRequest {
    const MESSAGE: &'static str = "SetFixConnectionEnabledRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "id" => self.id = req_string(value, "id")?,
            "enabled" => self.enabled = bool_or_false(value),
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

/// Decode a [`ListFixConnectionsRequest`] envelope — fully generic.
///
/// # Errors
/// A malformed `principal` body, as a [`CodecError`].
pub fn decode_list_fix_connections_request(
    o: &Map<String, Value>,
) -> DResult<ListFixConnectionsRequest> {
    decode(ListFixConnectionsRequest::MESSAGE, o)
}

/// Decode a [`CreateFixConnectionRequest`] envelope — fully generic.
///
/// # Errors
/// A missing/malformed `spec` (or a missing required `name`/`bind_addr`/comp-ids), as
/// a [`CodecError`].
pub fn decode_create_fix_connection_request(
    o: &Map<String, Value>,
) -> DResult<CreateFixConnectionRequest> {
    decode(CreateFixConnectionRequest::MESSAGE, o)
}

/// Decode an [`UpdateFixConnectionRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `id`, a missing/malformed `spec`, as a [`CodecError`].
pub fn decode_update_fix_connection_request(
    o: &Map<String, Value>,
) -> DResult<UpdateFixConnectionRequest> {
    decode(UpdateFixConnectionRequest::MESSAGE, o)
}

/// Decode a [`DeleteFixConnectionRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `id`, as a [`CodecError`].
pub fn decode_delete_fix_connection_request(
    o: &Map<String, Value>,
) -> DResult<DeleteFixConnectionRequest> {
    decode(DeleteFixConnectionRequest::MESSAGE, o)
}

/// Decode a [`SetFixConnectionEnabledRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `id`, as a [`CodecError`].
pub fn decode_set_fix_connection_enabled_request(
    o: &Map<String, Value>,
) -> DResult<SetFixConnectionEnabledRequest> {
    decode(SetFixConnectionEnabledRequest::MESSAGE, o)
}

/// Decode a [`ListFixMessagesRequest`] — a **message-level projection** mirroring the
/// hand `list_fix_messages_request_from_json`: two field-table-inexpressible quirks —
/// a `connection_id` that treats a whitespace-only string as absent
/// (`.filter(|s| !s.trim().is_empty())`), and a `limit` that saturates to `u32::MAX`
/// on overflow (the poll page cap) rather than the generic clamp-to-zero. The
/// remaining fields decode with the shared presence helpers.
///
/// # Errors
/// A malformed `principal` body, as a [`CodecError`].
pub fn decode_list_fix_messages_request(o: &Map<String, Value>) -> DResult<ListFixMessagesRequest> {
    let connection_id = o
        .get("connection_id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    Ok(ListFixMessagesRequest {
        connection_id,
        after_seq: u64_or_zero(o.get("after_seq")),
        limit: u32::try_from(u64_or_zero(o.get("limit"))).unwrap_or(u32::MAX),
        principal: opt_msg::<EntitlementPrincipal>(
            o.get("principal").filter(|v| !v.is_null()),
            "principal",
        )?,
        correlation_id: opt_u64(o.get("correlation_id").filter(|v| !v.is_null())),
        session_token: opt_string(o.get("session_token"), "session_token")?,
    })
}

// --- fix-admin encode adapters (encode) -------------------------------------

impl WireAdapter for FixConnectionDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "id" => Some(WireVal::Str(&self.id)),
            "name" => Some(WireVal::Str(&self.name)),
            "kind" => Some(WireVal::Enum(self.kind)),
            "bind_addr" => Some(WireVal::Str(&self.bind_addr)),
            "sender_comp_id" => Some(WireVal::Str(&self.sender_comp_id)),
            "target_comp_id" => Some(WireVal::Str(&self.target_comp_id)),
            "enabled" => Some(WireVal::Bool(self.enabled)),
            "running" => Some(WireVal::Bool(self.running)),
            "bound_addr" => Some(WireVal::Str(&self.bound_addr)),
            "desk" => Some(WireVal::Str(&self.desk)),
            _ => None,
        }
    }
}

impl WireAdapter for FixMessage {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "seq" => Some(WireVal::U64(self.seq)),
            "connection_id" => Some(WireVal::Str(&self.connection_id)),
            "direction" => Some(WireVal::Enum(self.direction)),
            "msg_type" => Some(WireVal::Str(&self.msg_type)),
            "summary" => Some(WireVal::Str(&self.summary)),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            "raw" => Some(WireVal::Str(&self.raw)),
            _ => None,
        }
    }
}

impl WireAdapter for ListFixConnectionsResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "connections" => Some(WireVal::RepeatedMsg(
                self.connections
                    .iter()
                    .map(|c| c as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for CreateFixConnectionResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "connection" => self
                .connection
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdateFixConnectionResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "connection" => self
                .connection
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DeleteFixConnectionResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for SetFixConnectionEnabledResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "connection" => self
                .connection
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for ListFixMessagesResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "messages" => Some(WireVal::RepeatedMsg(
                self.messages
                    .iter()
                    .map(|m| m as &dyn WireAdapter)
                    .collect(),
            )),
            "latest_seq" => Some(WireVal::U64(self.latest_seq)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

/// Encode a [`ListFixConnectionsResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_fix_connections_response(r: &ListFixConnectionsResponse) -> Value {
    encode("ListFixConnectionsResponse", r)
}

/// Encode a [`CreateFixConnectionResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_create_fix_connection_response(r: &CreateFixConnectionResponse) -> Value {
    encode("CreateFixConnectionResponse", r)
}

/// Encode an [`UpdateFixConnectionResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_fix_connection_response(r: &UpdateFixConnectionResponse) -> Value {
    encode("UpdateFixConnectionResponse", r)
}

/// Encode a [`DeleteFixConnectionResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_delete_fix_connection_response(r: &DeleteFixConnectionResponse) -> Value {
    encode("DeleteFixConnectionResponse", r)
}

/// Encode a [`SetFixConnectionEnabledResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_set_fix_connection_enabled_response(r: &SetFixConnectionEnabledResponse) -> Value {
    encode("SetFixConnectionEnabledResponse", r)
}

/// Encode a [`ListFixMessagesResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_fix_messages_response(r: &ListFixMessagesResponse) -> Value {
    encode("ListFixMessagesResponse", r)
}

// ===========================================================================
// QuoteService — the RFQ lifecycle (RequestQuote / RequestMultiDealerQuote /
// AcceptQuote / RejectQuote)
// ===========================================================================
//
// Four unary verbs. `QuoteRequest` keeps a message-level projection
// ([`decode_quote_request`]) because it nests the FX-legacy `Instrument` dual-key,
// exactly like `PriceRequest`; `QuoteAccept` / `QuoteReject` decode fully generically.
// The reply surface (`Quote` / `MultiDealerQuote` / `DealerQuote` / `Execution` /
// `RejectAck`) encodes from the field tables, with three curated overrides the
// descriptor cannot express: the who's-trading `AttributionRecord` / `BookId` / `Owner`
// tree rides under **camelCase** wire keys and its manual-`Map` hand encoders OMIT
// absent fields (`omit_absent_message`), the `json!({ .. })` reply encoders render
// absent presence-tracked fields as `null` (`null_absent_optional`), and the booked
// `Execution` never serializes its `instrument` (a `Suppress` rule).

// --- who's-trading attribution WireBuilders (decode; shared) ----------------

impl WireBuilder for Owner {
    const MESSAGE: &'static str = "Owner";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        use celnet_proto::owner::Seat;
        // `decode` calls `set` only for the single live oneof arm it selected.
        match field.proto_name {
            "trader" => self.seat = Some(Seat::Trader(req_string(value, "trader")?)),
            "auto_pricer" => self.seat = Some(Seat::AutoPricer(req_string(value, "autoPricer")?)),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for BookId {
    const MESSAGE: &'static str = "BookId";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "book" => self.book = req_string(value, "book")?,
            "owner" => self.owner = opt_msg::<Owner>(value, "owner")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for AttributionRecord {
    const MESSAGE: &'static str = "AttributionRecord";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        // The camelCase wire keys (`quotedBy` / `heldBy` / `lpCount`) were resolved by
        // the shared override in `decode`; here the fields are plain optionals.
        match field.proto_name {
            "quoted_by" => self.quoted_by = opt_msg::<BookId>(value, "quotedBy")?,
            "held_by" => self.held_by = opt_msg::<BookId>(value, "heldBy")?,
            "won" => self.won = opt_bool(value),
            "lp_count" => self.lp_count = opt_u32(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- quote request WireBuilders (decode) ------------------------------------

impl WireBuilder for QuoteAccept {
    const MESSAGE: &'static str = "QuoteAccept";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "quote_id" => self.quote_id = req_u64(value, "quote_id")?,
            "idempotency_key" => self.idempotency_key = string_or_empty(value),
            "side" => self.side = enum_or_zero(value),
            "lp_id" => self.lp_id = string_or_empty(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for QuoteReject {
    const MESSAGE: &'static str = "QuoteReject";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "quote_id" => self.quote_id = req_u64(value, "quote_id")?,
            "reason" => self.reason = string_or_empty(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

/// Decode a [`QuoteRequest`] — a **message-level projection** (mirrors the hand
/// `quote_request_from_json`): it nests the FX-legacy [`Instrument`] dual-key that the
/// flat field table cannot express, delegating the leaf bodies to the generic
/// decoders. `conventions` is required; the attribution / ids / caller identity are
/// presence-tracked.
///
/// # Errors
/// A missing required field (`idempotency_key` / `instrument` / `conventions`) or a
/// malformed body, as a [`CodecError`].
pub fn decode_quote_request(o: &Map<String, Value>) -> DResult<QuoteRequest> {
    Ok(QuoteRequest {
        idempotency_key: req_string(o.get("idempotency_key"), "idempotency_key")?,
        instrument: Some(decode_instrument(req_value(
            o.get("instrument"),
            "instrument",
        )?)?),
        conventions: Some(decode_conventions(req_value(
            o.get("conventions"),
            "conventions",
        )?)?),
        correlation_id: opt_u64(o.get("correlation_id").filter(|v| !v.is_null())),
        surface_version: opt_u64(o.get("surface_version").filter(|v| !v.is_null())),
        attribution: opt_msg::<AttributionRecord>(
            o.get("attribution").filter(|v| !v.is_null()),
            "attribution",
        )?,
        session_token: opt_string(o.get("session_token"), "session_token")?,
        principal: opt_msg::<EntitlementPrincipal>(
            o.get("principal").filter(|v| !v.is_null()),
            "principal",
        )?,
    })
}

/// Decode a [`QuoteAccept`] envelope — fully generic.
///
/// # Errors
/// A missing `quote_id`, as a [`CodecError`].
pub fn decode_quote_accept(o: &Map<String, Value>) -> DResult<QuoteAccept> {
    decode(QuoteAccept::MESSAGE, o)
}

/// Decode a [`QuoteReject`] envelope — fully generic.
///
/// # Errors
/// A missing `quote_id`, as a [`CodecError`].
pub fn decode_quote_reject(o: &Map<String, Value>) -> DResult<QuoteReject> {
    decode(QuoteReject::MESSAGE, o)
}

// --- who's-trading attribution adapters (encode; shared) --------------------

impl WireAdapter for Owner {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        use celnet_proto::owner::Seat;
        match (proto_name, &self.seat) {
            ("trader", Some(Seat::Trader(t))) => Some(WireVal::Str(t)),
            ("auto_pricer", Some(Seat::AutoPricer(p))) => Some(WireVal::Str(p)),
            _ => None,
        }
    }
}

impl WireAdapter for BookId {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "book" => Some(WireVal::Str(&self.book)),
            "owner" => self
                .owner
                .as_ref()
                .map(|o| WireVal::Msg(o as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for AttributionRecord {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "quoted_by" => self
                .quoted_by
                .as_ref()
                .map(|b| WireVal::Msg(b as &dyn WireAdapter)),
            "held_by" => self
                .held_by
                .as_ref()
                .map(|b| WireVal::Msg(b as &dyn WireAdapter)),
            "won" => self.won.map(WireVal::Bool),
            "lp_count" => self.lp_count.map(|n| WireVal::U64(u64::from(n))),
            _ => None,
        }
    }
}

// --- quote reply adapters (encode) ------------------------------------------

impl WireAdapter for Quote {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "quote_id" => Some(WireVal::U64(self.quote_id)),
            "idempotency_key" => Some(WireVal::Str(&self.idempotency_key)),
            "price" => self
                .price
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            "greeks" => self
                .greeks
                .as_ref()
                .map(|g| WireVal::Msg(g as &dyn WireAdapter)),
            "conventions" => self
                .conventions
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "resolved_strike" => Some(WireVal::F64(self.resolved_strike)),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            "valid_until_nanos" => Some(WireVal::I64(self.valid_until_nanos)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            "surface_version" => self.surface_version.map(WireVal::U64),
            "attribution" => self
                .attribution
                .as_ref()
                .map(|a| WireVal::Msg(a as &dyn WireAdapter)),
            "price_std_error" => self.price_std_error.map(WireVal::F64),
            "pricing_provenance" => self
                .pricing_provenance
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for PricingProvenance {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "pricing_group_id" => Some(WireVal::Str(&self.pricing_group_id)),
            "mode" => Some(WireVal::Enum(self.mode)),
            "raw_bid" => Some(WireVal::F64(self.raw_bid)),
            "raw_mid" => Some(WireVal::F64(self.raw_mid)),
            "raw_offer" => Some(WireVal::F64(self.raw_offer)),
            "constructed_bid" => Some(WireVal::F64(self.constructed_bid)),
            "constructed_offer" => Some(WireVal::F64(self.constructed_offer)),
            "tiered_bid" => Some(WireVal::F64(self.tiered_bid)),
            "tiered_offer" => Some(WireVal::F64(self.tiered_offer)),
            "outbound_bid" => Some(WireVal::F64(self.outbound_bid)),
            "outbound_offer" => Some(WireVal::F64(self.outbound_offer)),
            "applied_margin" => Some(WireVal::F64(self.applied_margin)),
            "applied_skew" => Some(WireVal::F64(self.applied_skew)),
            "features" => Some(WireVal::RepeatedEnum(&self.features)),
            _ => None,
        }
    }
}

impl WireAdapter for DealerQuote {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "lp_id" => Some(WireVal::Str(&self.lp_id)),
            "price" => self
                .price
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            "greeks" => self
                .greeks
                .as_ref()
                .map(|g| WireVal::Msg(g as &dyn WireAdapter)),
            "resolved_strike" => Some(WireVal::F64(self.resolved_strike)),
            "valid_until_nanos" => Some(WireVal::I64(self.valid_until_nanos)),
            "attribution" => self
                .attribution
                .as_ref()
                .map(|a| WireVal::Msg(a as &dyn WireAdapter)),
            "price_std_error" => self.price_std_error.map(WireVal::F64),
            _ => None,
        }
    }
}

impl WireAdapter for MultiDealerQuote {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "quote_id" => Some(WireVal::U64(self.quote_id)),
            "idempotency_key" => Some(WireVal::Str(&self.idempotency_key)),
            "dealers" => Some(WireVal::RepeatedMsg(
                self.dealers.iter().map(|d| d as &dyn WireAdapter).collect(),
            )),
            "best_bid_lp_id" => Some(WireVal::Str(&self.best_bid_lp_id)),
            "best_offer_lp_id" => Some(WireVal::Str(&self.best_offer_lp_id)),
            "conventions" => self
                .conventions
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            "surface_version" => self.surface_version.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for Execution {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        // `instrument` is suppressed by the override (never serialized by the hand
        // `execution_to_json`), so it is never requested here.
        match proto_name {
            "execution_id" => Some(WireVal::U64(self.execution_id)),
            "quote_id" => Some(WireVal::U64(self.quote_id)),
            "side" => Some(WireVal::Enum(self.side)),
            "traded_premium" => Some(WireVal::F64(self.traded_premium)),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            "attribution" => self
                .attribution
                .as_ref()
                .map(|a| WireVal::Msg(a as &dyn WireAdapter)),
            "pricing_provenance" => self
                .pricing_provenance
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for RejectAck {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "quote_id" => Some(WireVal::U64(self.quote_id)),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            _ => None,
        }
    }
}

/// Encode a [`Quote`] (the single-dealer RFQ reply) to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_quote(q: &Quote) -> Value {
    encode("Quote", q)
}

/// Encode a [`MultiDealerQuote`] (the ranked LP panel reply) to its WS JSON.
#[must_use]
pub fn encode_multi_dealer_quote(m: &MultiDealerQuote) -> Value {
    encode("MultiDealerQuote", m)
}

/// Encode an [`Execution`] (the booking confirmation) to its WS JSON.
#[must_use]
pub fn encode_execution(e: &Execution) -> Value {
    encode("Execution", e)
}

/// Encode a [`RejectAck`] (the quote-declined acknowledgement) to its WS JSON.
#[must_use]
pub fn encode_reject_ack(a: &RejectAck) -> Value {
    encode("RejectAck", a)
}

// ===========================================================================
// SurfaceService — smile read / broker mark / scenario risk (arch item G —
// `ws-codec-from-proto`, wave 3)
// ===========================================================================
//
// Three unary verbs (`GetSmile` / `MarkSurface` / `Scenario`). The request tree
// decodes generically off the descriptor EXCEPT `ScenarioRequest`, which nests
// the FX-legacy dual-key `Instrument` and so keeps the message-level projection
// escape hatch ([`decode_scenario_request`], delegating the instrument to
// [`decode_instrument`]). Two decode quirks the descriptor cannot express are
// carried in the mechanical builder-placement layer, not the field table:
// `smile_model` is a presence-tracked enum that also accepts its `SMILE_MODEL_*`
// string name ([`opt_smile_model`]), and a request-side `VegaBucket` / `CrossGamma`
// ignores its response-only `vega` / `value` field (hardcoded `0.0`), exactly as
// the hand codec does. The reply surface (`Smile` / `MarkSurfaceResponse` /
// `ScenarioResponse` and their sub-trees) encodes straight from the field tables —
// every absent singular message reaches the wire as JSON `null`, matching the hand
// `json!({ .. })` encoders, so no override-table entry is needed.

/// Decode an optional smile-model selector (mirrors the hand `opt_smile_model`):
/// accepts either the proto3 enum integer or its `SMILE_MODEL_*` string name;
/// absent/null ⇒ `None` (the server's default calibration). The string arm is
/// re-derived **independently** from the published [`SmileModel`] enum (guardrail:
/// re-derive constants from the source, never call the hand codec) and accepts
/// exactly the four selectable families the hand codec does — an unrecognized name
/// (including `SMILE_MODEL_EXTENDED_SURFACE`, a server-internal repair family, not a
/// client selector) decodes to `None`, byte-identically. `value` is the field's JSON
/// value if present-and-non-null (as resolved by [`decode`]), else `None`.
fn opt_smile_model(value: Option<&Value>) -> Option<i32> {
    match value {
        None => None,
        Some(Value::Number(n)) => n.as_i64().and_then(|v| i32::try_from(v).ok()),
        Some(Value::String(s)) => match s.as_str() {
            "SMILE_MODEL_MARKET_HEDGE" => Some(SmileModel::MarketHedge as i32),
            "SMILE_MODEL_STOCHASTIC_VOL" => Some(SmileModel::StochasticVol as i32),
            "SMILE_MODEL_PARAMETRIC" => Some(SmileModel::Parametric as i32),
            "SMILE_MODEL_PARAMETRIC_SURFACE" => Some(SmileModel::ParametricSurface as i32),
            _ => None,
        },
        Some(_) => None,
    }
}

// --- surface request WireBuilders (decode) ----------------------------------

impl WireBuilder for BrokerQuoteSet {
    const MESSAGE: &'static str = "BrokerQuoteSet";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor_years" => self.tenor_years = req_f64(value, "tenor_years")?,
            "atm_vol" => self.atm_vol = req_f64(value, "atm_vol")?,
            "rr_25" => self.rr_25 = f64_or_zero(value),
            "bf_25" => self.bf_25 = f64_or_zero(value),
            "rr_10" => self.rr_10 = f64_or_zero(value),
            "bf_10" => self.bf_10 = f64_or_zero(value),
            "has_ten_delta" => self.has_ten_delta = bool_or_false(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ShockAxis {
    const MESSAGE: &'static str = "ShockAxis";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "factor" => self.factor = enum_or_zero(value),
            "relative" => self.relative = bool_or_false(value),
            "steps" => self.steps = f64_vec(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CrossGamma {
    const MESSAGE: &'static str = "CrossGamma";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "factor_a" => self.factor_a = enum_or_zero(value),
            "factor_b" => self.factor_b = enum_or_zero(value),
            // `value` is a response-only field: a request-side cross-gamma pair only
            // names the two factors, so the hand codec hardcodes `0.0` regardless of
            // any JSON value (`cross_gamma_pair_from_json`).
            "value" => self.value = 0.0,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for VegaBucket {
    const MESSAGE: &'static str = "VegaBucket";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor_years" => self.tenor_years = req_f64(value, "tenor_years")?,
            "delta" => self.delta = f64_or_zero(value),
            // `vega` is a response-only measure: a request-side pillar only selects
            // the (tenor, delta) bucket, so the hand codec hardcodes `0.0`
            // (`vega_bucket_from_json`).
            "vega" => self.vega = 0.0,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for RiskBucketRequest {
    const MESSAGE: &'static str = "RiskBucketRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "vega_pillars" => {
                self.vega_pillars = opt_repeated::<VegaBucket>(value, "vega_pillars")?
            }
            "cross_gamma_pairs" => {
                self.cross_gamma_pairs = opt_repeated::<CrossGamma>(value, "cross_gamma_pairs")?;
            }
            "roll_horizons_years" => self.roll_horizons_years = f64_vec(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for GetSmileRequest {
    const MESSAGE: &'static str = "GetSmileRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "pair" => self.pair = opt_msg::<CcyPair>(value, "pair")?,
            "tenor_years" => self.tenor_years = req_f64(value, "tenor_years")?,
            "conventions" => self.conventions = Some(req_msg::<Conventions>(value, "conventions")?),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for MarkSurfaceRequest {
    const MESSAGE: &'static str = "MarkSurfaceRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "pair" => self.pair = opt_msg::<CcyPair>(value, "pair")?,
            "broker_quotes" => {
                self.broker_quotes = req_repeated::<BrokerQuoteSet>(value, "broker_quotes")?;
            }
            "conventions" => self.conventions = Some(req_msg::<Conventions>(value, "conventions")?),
            "smile_model" => self.smile_model = opt_smile_model(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

/// Decode a [`GetSmileRequest`] envelope — fully generic (a plain `CcyPair` +
/// `Conventions`; no FX-legacy dual-key).
///
/// # Errors
/// A missing required field (`tenor_years` / `conventions`) or a malformed nested
/// body, as a [`CodecError`].
pub fn decode_get_smile_request(o: &Map<String, Value>) -> DResult<GetSmileRequest> {
    decode(GetSmileRequest::MESSAGE, o)
}

/// Decode a [`MarkSurfaceRequest`] envelope — fully generic; the `smile_model`
/// selector accepts its enum integer or `SMILE_MODEL_*` string name via
/// [`opt_smile_model`].
///
/// # Errors
/// A missing required `broker_quotes` array / `conventions`, or a malformed nested
/// body, as a [`CodecError`].
pub fn decode_mark_surface_request(o: &Map<String, Value>) -> DResult<MarkSurfaceRequest> {
    decode(MarkSurfaceRequest::MESSAGE, o)
}

/// Decode a [`ScenarioRequest`] envelope (mirrors the hand `scenario_request_from_json`)
/// — a **message-level projection** because it nests the FX-legacy dual-key
/// [`Instrument`] and FX-legacy [`MarketContext`], delegating both to the shared
/// generated projections. The `axes` grid is required; `risk_buckets` and
/// `smile_model` are presence-tracked.
///
/// # Errors
/// A missing required nested field (`instrument` / `base_market` / `conventions` /
/// `axes`) or a malformed body, as a [`CodecError`].
pub fn decode_scenario_request(o: &Map<String, Value>) -> DResult<ScenarioRequest> {
    Ok(ScenarioRequest {
        instrument: Some(decode_instrument(req_value(
            o.get("instrument"),
            "instrument",
        )?)?),
        base_market: Some(decode_market_context(req_value(
            o.get("base_market"),
            "base_market",
        )?)?),
        conventions: Some(decode_conventions(req_value(
            o.get("conventions"),
            "conventions",
        )?)?),
        axes: req_repeated::<ShockAxis>(o.get("axes"), "axes")?,
        expiry_years: f64_or_zero(o.get("expiry_years").filter(|v| !v.is_null())),
        risk_buckets: opt_msg::<RiskBucketRequest>(
            o.get("risk_buckets").filter(|v| !v.is_null()),
            "risk_buckets",
        )?,
        smile_model: opt_smile_model(o.get("smile_model").filter(|v| !v.is_null())),
    })
}

// --- surface reply WireAdapters (encode) ------------------------------------

impl WireAdapter for SmilePoint {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "delta" => Some(WireVal::F64(self.delta)),
            "tenor_years" => Some(WireVal::F64(self.tenor_years)),
            "vol" => Some(WireVal::F64(self.vol)),
            _ => None,
        }
    }
}

impl WireAdapter for BrokerQuoteSet {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_years" => Some(WireVal::F64(self.tenor_years)),
            "atm_vol" => Some(WireVal::F64(self.atm_vol)),
            "rr_25" => Some(WireVal::F64(self.rr_25)),
            "bf_25" => Some(WireVal::F64(self.bf_25)),
            "rr_10" => Some(WireVal::F64(self.rr_10)),
            "bf_10" => Some(WireVal::F64(self.bf_10)),
            "has_ten_delta" => Some(WireVal::Bool(self.has_ten_delta)),
            _ => None,
        }
    }
}

impl WireAdapter for Smile {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "pair" => self
                .pair
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            "tenor_years" => Some(WireVal::F64(self.tenor_years)),
            "broker_quotes" => self
                .broker_quotes
                .as_ref()
                .map(|b| WireVal::Msg(b as &dyn WireAdapter)),
            "points" => Some(WireVal::RepeatedMsg(
                self.points.iter().map(|p| p as &dyn WireAdapter).collect(),
            )),
            "conventions" => self
                .conventions
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "arbitrage" => self
                .arbitrage
                .as_ref()
                .map(|a| WireVal::Msg(a as &dyn WireAdapter)),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            _ => None,
        }
    }
}

impl WireAdapter for MarkSurfaceResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "pair" => self
                .pair
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            "surface_version" => Some(WireVal::U64(self.surface_version)),
            "smiles" => Some(WireVal::RepeatedMsg(
                self.smiles.iter().map(|s| s as &dyn WireAdapter).collect(),
            )),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            _ => None,
        }
    }
}

impl WireAdapter for VegaBucket {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_years" => Some(WireVal::F64(self.tenor_years)),
            "delta" => Some(WireVal::F64(self.delta)),
            "vega" => Some(WireVal::F64(self.vega)),
            _ => None,
        }
    }
}

impl WireAdapter for CrossGamma {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "factor_a" => Some(WireVal::Enum(self.factor_a)),
            "factor_b" => Some(WireVal::Enum(self.factor_b)),
            "value" => Some(WireVal::F64(self.value)),
            _ => None,
        }
    }
}

impl WireAdapter for ScenarioPoint {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "applied_shocks" => Some(WireVal::RepeatedF64(&self.applied_shocks)),
            "shocked_market" => self
                .shocked_market
                .as_ref()
                .map(|m| WireVal::Msg(m as &dyn WireAdapter)),
            "greeks" => self
                .greeks
                .as_ref()
                .map(|g| WireVal::Msg(g as &dyn WireAdapter)),
            "expiry_years" => Some(WireVal::F64(self.expiry_years)),
            _ => None,
        }
    }
}

impl WireAdapter for BucketedRisk {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "vega_buckets" => Some(WireVal::RepeatedMsg(
                self.vega_buckets
                    .iter()
                    .map(|b| b as &dyn WireAdapter)
                    .collect(),
            )),
            "cross_gammas" => Some(WireVal::RepeatedMsg(
                self.cross_gammas
                    .iter()
                    .map(|c| c as &dyn WireAdapter)
                    .collect(),
            )),
            "theta_roll" => Some(WireVal::RepeatedF64(&self.theta_roll)),
            "roll_horizons_years" => Some(WireVal::RepeatedF64(&self.roll_horizons_years)),
            _ => None,
        }
    }
}

impl WireAdapter for ScenarioResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "points" => Some(WireVal::RepeatedMsg(
                self.points.iter().map(|p| p as &dyn WireAdapter).collect(),
            )),
            "bucketed_risk" => self
                .bucketed_risk
                .as_ref()
                .map(|b| WireVal::Msg(b as &dyn WireAdapter)),
            _ => None,
        }
    }
}

/// Encode a [`Smile`] (the `GetSmile` reply) to its WS JSON — descriptor-driven
/// (mirrors the hand `smile_reply_to_json` / `smile_to_json`). Absent singular
/// messages (`pair` / `broker_quotes` / `conventions` / `arbitrage`) reach the wire
/// as JSON `null`.
#[must_use]
pub fn encode_smile(s: &Smile) -> Value {
    encode("Smile", s)
}

/// Encode a [`MarkSurfaceResponse`] to its WS JSON (mirrors the hand
/// `mark_surface_response_to_json`).
#[must_use]
pub fn encode_mark_surface_response(r: &MarkSurfaceResponse) -> Value {
    encode("MarkSurfaceResponse", r)
}

/// Encode a [`ScenarioResponse`] to its WS JSON (mirrors the hand
/// `scenario_response_to_json`).
#[must_use]
pub fn encode_scenario_response(r: &ScenarioResponse) -> Value {
    encode("ScenarioResponse", r)
}

// ===========================================================================
// RiskService — server-side hierarchical risk + linear-rates book/list/rollup
// (arch item G — `ws-codec-from-proto`, wave 3)
// ===========================================================================
//
// Seven unary verbs (`ListPositions` / `AggregateRisk` / `AggregateRatesRisk` /
// `DrillRisk` / `LimitStatus` / `BookRatesPosition` / `ListRatesPositions`). Every
// request envelope decodes fully generically off the descriptor, reusing the shared
// `EntitlementPrincipal` tree (built for the fix-admin family) plus the
// `ReportingNumeraire` / `RiskScope` / `VegaPillar` / `RatesPosition` sub-trees. The
// rich `RiskNode` / `RiskPosition` / `RatesRiskNode` reply hierarchies encode
// straight from the field tables, with two curated FX-legacy encode divergences the
// descriptor cannot express: `VanillaInputs` carries the same `r_dom`/`r_for` carry
// seam as `MarketContext` (quirk b, [`codec_overrides::vanilla_inputs_synth`]), and
// `OrgKey.underlying` rides under the legacy `ccy_pair` key via the FX `Underlying`
// projection (quirk a). Absent presence-tracked reply fields (the `correlation_id`
// echoes, `RiskPosition.attribution`, the `NonAdditiveRisk` VaR/ES/curvature) reach
// the wire as JSON `null` (`codec_overrides::null_absent_optional`).

// --- risk request WireBuilders (decode) -------------------------------------

impl WireBuilder for NumeraireRate {
    const MESSAGE: &'static str = "NumeraireRate";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "ccy" => self.ccy = req_string(value, "ccy")?,
            "rate" => self.rate = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ReportingNumeraire {
    const MESSAGE: &'static str = "ReportingNumeraire";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "numeraire" => self.numeraire = req_string(value, "numeraire")?,
            "rates" => self.rates = opt_repeated::<NumeraireRate>(value, "rates")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for VegaPillar {
    const MESSAGE: &'static str = "VegaPillar";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor_days" => self.tenor_days = u32_or_zero(value),
            "delta_bp" => self.delta_bp = i32_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for RatesRiskScope {
    const MESSAGE: &'static str = "RatesRiskScope";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "entity" => self.entity = opt_u32(value),
            "book" => self.book = opt_u32(value),
            "ccy" => self.ccy = opt_string(value, "ccy")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for RatesPosition {
    const MESSAGE: &'static str = "RatesPosition";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "position_id" => self.position_id = u64_or_zero(value),
            "entity" => self.entity = u32_or_zero(value),
            "book" => self.book = u32_or_zero(value),
            "instrument" => {
                self.instrument = Some(req_msg::<RatesInstrument>(value, "instrument")?);
            }
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListPositionsRequest {
    const MESSAGE: &'static str = "ListPositionsRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "scope" => self.scope = opt_msg::<RiskScope>(value, "scope")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for AggregateRiskRequest {
    const MESSAGE: &'static str = "AggregateRiskRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "dimension" => self.dimension = enum_or_zero(value),
            "numeraire" => self.numeraire = opt_msg::<ReportingNumeraire>(value, "numeraire")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "scope" => self.scope = opt_msg::<RiskScope>(value, "scope")?,
            "vega_pillars" => {
                self.vega_pillars = opt_repeated::<VegaPillar>(value, "vega_pillars")?
            }
            "var_spot_shocks" => self.var_spot_shocks = f64_vec(value),
            "var_alpha" => self.var_alpha = f64_or_zero(value),
            "curvature_risk_weight" => self.curvature_risk_weight = f64_or_zero(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DrillRiskRequest {
    const MESSAGE: &'static str = "DrillRiskRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "node" => self.node = opt_msg::<RiskScope>(value, "node")?,
            "child_dimension" => self.child_dimension = enum_or_zero(value),
            "numeraire" => self.numeraire = opt_msg::<ReportingNumeraire>(value, "numeraire")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "vega_pillars" => {
                self.vega_pillars = opt_repeated::<VegaPillar>(value, "vega_pillars")?
            }
            "include_children" => self.include_children = bool_or_false(value),
            "include_positions" => self.include_positions = bool_or_false(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for LimitStatusRequest {
    const MESSAGE: &'static str = "LimitStatusRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "scope" => self.scope = opt_msg::<RiskScope>(value, "scope")?,
            "numeraire" => self.numeraire = opt_msg::<ReportingNumeraire>(value, "numeraire")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "vega_pillars" => {
                self.vega_pillars = opt_repeated::<VegaPillar>(value, "vega_pillars")?
            }
            "var_spot_shocks" => self.var_spot_shocks = f64_vec(value),
            "var_alpha" => self.var_alpha = f64_or_zero(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for AggregateRatesRiskRequest {
    const MESSAGE: &'static str = "AggregateRatesRiskRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "curve_set" => self.curve_set = Some(req_msg::<CurveSet>(value, "curve_set")?),
            "positions" => self.positions = opt_repeated::<RatesPosition>(value, "positions")?,
            "scope" => self.scope = opt_msg::<RatesRiskScope>(value, "scope")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for BookRatesPositionRequest {
    const MESSAGE: &'static str = "BookRatesPositionRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            "position" => self.position = Some(req_msg::<RatesPosition>(value, "position")?),
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_string(value, "correlation_id")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListRatesPositionsRequest {
    const MESSAGE: &'static str = "ListRatesPositionsRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            "scope" => self.scope = opt_msg::<RatesRiskScope>(value, "scope")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_string(value, "correlation_id")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

/// Decode a [`ListPositionsRequest`] envelope — fully generic (shared
/// `RiskScope` + `EntitlementPrincipal`).
///
/// # Errors
/// A malformed nested body, as a [`CodecError`].
pub fn decode_list_positions_request(o: &Map<String, Value>) -> DResult<ListPositionsRequest> {
    decode(ListPositionsRequest::MESSAGE, o)
}

/// Decode an [`AggregateRiskRequest`] envelope — fully generic.
///
/// # Errors
/// A malformed nested body, as a [`CodecError`].
pub fn decode_aggregate_risk_request(o: &Map<String, Value>) -> DResult<AggregateRiskRequest> {
    decode(AggregateRiskRequest::MESSAGE, o)
}

/// Decode a [`DrillRiskRequest`] envelope — fully generic.
///
/// # Errors
/// A malformed nested body, as a [`CodecError`].
pub fn decode_drill_risk_request(o: &Map<String, Value>) -> DResult<DrillRiskRequest> {
    decode(DrillRiskRequest::MESSAGE, o)
}

/// Decode a [`LimitStatusRequest`] envelope — fully generic.
///
/// # Errors
/// A malformed nested body, as a [`CodecError`].
pub fn decode_limit_status_request(o: &Map<String, Value>) -> DResult<LimitStatusRequest> {
    decode(LimitStatusRequest::MESSAGE, o)
}

/// Decode an [`AggregateRatesRiskRequest`] envelope — fully generic; the
/// `curve_set` / `positions` reuse the shared `CurveSet` / `RatesInstrument`
/// builders the `PriceRates` request decode does.
///
/// # Errors
/// A missing required `curve_set` (or `RatesInstrument` arm) or a malformed body.
pub fn decode_aggregate_rates_risk_request(
    o: &Map<String, Value>,
) -> DResult<AggregateRatesRiskRequest> {
    decode(AggregateRatesRiskRequest::MESSAGE, o)
}

/// Decode a [`BookRatesPositionRequest`] envelope — fully generic.
///
/// # Errors
/// A missing required `position` (or `RatesInstrument` arm) or a malformed body.
pub fn decode_book_rates_position_request(
    o: &Map<String, Value>,
) -> DResult<BookRatesPositionRequest> {
    decode(BookRatesPositionRequest::MESSAGE, o)
}

/// Decode a [`ListRatesPositionsRequest`] envelope — fully generic.
///
/// # Errors
/// A malformed nested body, as a [`CodecError`].
pub fn decode_list_rates_positions_request(
    o: &Map<String, Value>,
) -> DResult<ListRatesPositionsRequest> {
    decode(ListRatesPositionsRequest::MESSAGE, o)
}

// --- shared linear-rates instrument/curve encode adapters (used by the rates
// risk rollup AND the dealer-desk `DeskRequest`/`Deal` blotter). The pure rates
// tree carries no FX-legacy quirk; every absent singular message (a bond
// `maturity_date`, a curve `reference_date`, a pillar `tenor`) reaches the wire as
// JSON `null`, matching the hand `rates_instrument_to_json` / `curve_set_to_json`.

impl WireAdapter for OisInstrument {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_years" => Some(WireVal::U64(u64::from(self.tenor_years))),
            "fixed_rate" => Some(WireVal::F64(self.fixed_rate)),
            "notional" => Some(WireVal::F64(self.notional)),
            "side" => Some(WireVal::Enum(self.side)),
            _ => None,
        }
    }
}

impl WireAdapter for VanillaIrsInstrument {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_years" => Some(WireVal::U64(u64::from(self.tenor_years))),
            "fixed_rate" => Some(WireVal::F64(self.fixed_rate)),
            "notional" => Some(WireVal::F64(self.notional)),
            "side" => Some(WireVal::Enum(self.side)),
            "fixed_frequency" => Some(WireVal::Enum(self.fixed_frequency)),
            "fixed_day_count" => Some(WireVal::Enum(self.fixed_day_count)),
            "float_frequency" => Some(WireVal::Enum(self.float_frequency)),
            "float_day_count" => Some(WireVal::Enum(self.float_day_count)),
            _ => None,
        }
    }
}

impl WireAdapter for FraInstrument {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "start_months" => Some(WireVal::U64(u64::from(self.start_months))),
            "end_months" => Some(WireVal::U64(u64::from(self.end_months))),
            "fixed_rate" => Some(WireVal::F64(self.fixed_rate)),
            "notional" => Some(WireVal::F64(self.notional)),
            "side" => Some(WireVal::Enum(self.side)),
            "accrual_basis" => Some(WireVal::Enum(self.accrual_basis)),
            _ => None,
        }
    }
}

impl WireAdapter for BondInstrument {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "coupon_rate" => Some(WireVal::F64(self.coupon_rate)),
            "coupon_frequency" => Some(WireVal::Enum(self.coupon_frequency)),
            "day_count" => Some(WireVal::Enum(self.day_count)),
            "maturity_date" => self
                .maturity_date
                .as_ref()
                .map(|d| WireVal::Msg(d as &dyn WireAdapter)),
            "redemption" => Some(WireVal::F64(self.redemption)),
            "side" => Some(WireVal::Enum(self.side)),
            _ => None,
        }
    }
}

impl WireAdapter for RatesInstrument {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        use rates_instrument::Instrument;
        match (proto_name, &self.instrument) {
            ("ois", Some(Instrument::Ois(x))) => Some(WireVal::Msg(x as &dyn WireAdapter)),
            ("irs", Some(Instrument::Irs(x))) => Some(WireVal::Msg(x as &dyn WireAdapter)),
            ("fra", Some(Instrument::Fra(x))) => Some(WireVal::Msg(x as &dyn WireAdapter)),
            ("bond", Some(Instrument::Bond(x))) => Some(WireVal::Msg(x as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for PillarTenor {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        use pillar_tenor::Point;
        match (proto_name, &self.point) {
            ("years", Some(Point::Years(y))) => Some(WireVal::U64(u64::from(*y))),
            ("months", Some(Point::Months(m))) => Some(WireVal::U64(u64::from(*m))),
            ("maturity_date", Some(Point::MaturityDate(d))) => {
                Some(WireVal::Msg(d as &dyn WireAdapter))
            }
            _ => None,
        }
    }
}

impl WireAdapter for OisPillar {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor" => self
                .tenor
                .as_ref()
                .map(|t| WireVal::Msg(t as &dyn WireAdapter)),
            "par_rate" => Some(WireVal::F64(self.par_rate)),
            _ => None,
        }
    }
}

impl WireAdapter for CurveSet {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "currency" => Some(WireVal::Str(&self.currency)),
            "reference_date" => self
                .reference_date
                .as_ref()
                .map(|d| WireVal::Msg(d as &dyn WireAdapter)),
            "ois_pillars" => Some(WireVal::RepeatedMsg(
                self.ois_pillars
                    .iter()
                    .map(|p| p as &dyn WireAdapter)
                    .collect(),
            )),
            _ => None,
        }
    }
}

// --- curve-query reply WireAdapters (encode; SurfaceService GetCurve / MarkCurve /
//     CurveScenario — the fixed-income market-data query surface, ADR-0021). Pure
//     rates tree, no FX-legacy quirks. ---------------------------------------------

impl WireAdapter for CurvePoint {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_years" => Some(WireVal::F64(self.tenor_years)),
            "zero_rate" => Some(WireVal::F64(self.zero_rate)),
            "discount_factor" => Some(WireVal::F64(self.discount_factor)),
            _ => None,
        }
    }
}

impl WireAdapter for CurveParPillar {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_years" => Some(WireVal::F64(self.tenor_years)),
            "par_rate" => Some(WireVal::F64(self.par_rate)),
            _ => None,
        }
    }
}

impl WireAdapter for GetCurveResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "currency" => Some(WireVal::Str(&self.currency)),
            "reference_date" => self
                .reference_date
                .as_ref()
                .map(|d| WireVal::Msg(d as &dyn WireAdapter)),
            "points" => Some(WireVal::RepeatedMsg(
                self.points.iter().map(|p| p as &dyn WireAdapter).collect(),
            )),
            "par_pillars" => Some(WireVal::RepeatedMsg(
                self.par_pillars
                    .iter()
                    .map(|p| p as &dyn WireAdapter)
                    .collect(),
            )),
            "curve_version" => self.curve_version.map(WireVal::U64),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            _ => None,
        }
    }
}

impl WireAdapter for MarkCurveResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "currency" => Some(WireVal::Str(&self.currency)),
            "curve_version" => Some(WireVal::U64(self.curve_version)),
            "par_pillars" => Some(WireVal::RepeatedMsg(
                self.par_pillars
                    .iter()
                    .map(|p| p as &dyn WireAdapter)
                    .collect(),
            )),
            "points" => Some(WireVal::RepeatedMsg(
                self.points.iter().map(|p| p as &dyn WireAdapter).collect(),
            )),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            _ => None,
        }
    }
}

impl WireAdapter for CurveScenarioReprice {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "base_pv" => Some(WireVal::F64(self.base_pv)),
            "shifted_pv" => Some(WireVal::F64(self.shifted_pv)),
            "pv_change" => Some(WireVal::F64(self.pv_change)),
            "dv01" => Some(WireVal::F64(self.dv01)),
            _ => None,
        }
    }
}

impl WireAdapter for CurveScenarioResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "currency" => Some(WireVal::Str(&self.currency)),
            "points" => Some(WireVal::RepeatedMsg(
                self.points.iter().map(|p| p as &dyn WireAdapter).collect(),
            )),
            "reprice" => self
                .reprice
                .as_ref()
                .map(|r| WireVal::Msg(r as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for RatesPosition {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "position_id" => Some(WireVal::U64(self.position_id)),
            "entity" => Some(WireVal::U64(u64::from(self.entity))),
            "book" => Some(WireVal::U64(u64::from(self.book))),
            "instrument" => self
                .instrument
                .as_ref()
                .map(|i| WireVal::Msg(i as &dyn WireAdapter)),
            _ => None,
        }
    }
}

// --- risk reply WireAdapters (encode) ---------------------------------------

impl WireAdapter for RiskScope {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "dimension" => Some(WireVal::Enum(self.dimension)),
            "value" => Some(WireVal::U64(self.value)),
            _ => None,
        }
    }
}

impl WireAdapter for VanillaInputs {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "spot" => Some(WireVal::F64(self.spot)),
            "strike" => Some(WireVal::F64(self.strike)),
            "vol" => Some(WireVal::F64(self.vol)),
            "t" => Some(WireVal::F64(self.t)),
            // Renamed to `r_dom` by the override; `carry` is suppressed and reaches
            // the wire as the synthesized `r_for` instead (the FX carry seam, quirk b).
            "discount_rate" => Some(WireVal::F64(self.discount_rate)),
            _ => None,
        }
    }

    fn synthesized(&self) -> Vec<(&'static str, Value)> {
        codec_overrides::vanilla_inputs_synth(self)
    }
}

impl WireAdapter for OrgKey {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "trader" => Some(WireVal::U64(u64::from(self.trader))),
            "book" => Some(WireVal::U64(u64::from(self.book))),
            "desk" => Some(WireVal::U64(u64::from(self.desk))),
            // Renamed to the legacy `ccy_pair` key by the override; the value is the
            // FX `{base, quote}` projection produced by the `Underlying` adapter.
            "underlying" => self
                .underlying
                .as_ref()
                .map(|u| WireVal::Msg(u as &dyn WireAdapter)),
            "location" => Some(WireVal::U64(u64::from(self.location))),
            "entity" => Some(WireVal::U64(u64::from(self.entity))),
            _ => None,
        }
    }
}

impl WireAdapter for RiskPosition {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "position_id" => Some(WireVal::U64(self.position_id)),
            "org" => self
                .org
                .as_ref()
                .map(|o| WireVal::Msg(o as &dyn WireAdapter)),
            "option_type" => Some(WireVal::Enum(self.option_type)),
            "notional_base" => Some(WireVal::F64(self.notional_base)),
            "inputs" => self
                .inputs
                .as_ref()
                .map(|i| WireVal::Msg(i as &dyn WireAdapter)),
            "quoted_delta" => Some(WireVal::Enum(self.quoted_delta)),
            "premium_style" => Some(WireVal::Enum(self.premium_style)),
            "surface_version" => Some(WireVal::U64(self.surface_version)),
            "attribution" => self
                .attribution
                .as_ref()
                .map(|a| WireVal::Msg(a as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for VegaPillar {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_days" => Some(WireVal::U64(u64::from(self.tenor_days))),
            "delta_bp" => Some(WireVal::I64(i64::from(self.delta_bp))),
            _ => None,
        }
    }
}

impl WireAdapter for VegaLadderBucket {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "pillar" => self
                .pillar
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            "vega" => Some(WireVal::F64(self.vega)),
            _ => None,
        }
    }
}

impl WireAdapter for CcyExposureLeg {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "ccy" => Some(WireVal::Str(&self.ccy)),
            "amount" => Some(WireVal::F64(self.amount)),
            _ => None,
        }
    }
}

impl WireAdapter for AdditiveRisk {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "delta_numeraire" => Some(WireVal::F64(self.delta_numeraire)),
            "delta_vector" => Some(WireVal::RepeatedMsg(
                self.delta_vector
                    .iter()
                    .map(|l| l as &dyn WireAdapter)
                    .collect(),
            )),
            "gamma" => Some(WireVal::F64(self.gamma)),
            "vega_numeraire" => Some(WireVal::F64(self.vega_numeraire)),
            "theta" => Some(WireVal::F64(self.theta)),
            "vanna" => Some(WireVal::F64(self.vanna)),
            "volga" => Some(WireVal::F64(self.volga)),
            "charm" => Some(WireVal::F64(self.charm)),
            "speed" => Some(WireVal::F64(self.speed)),
            "zomma" => Some(WireVal::F64(self.zomma)),
            "color" => Some(WireVal::F64(self.color)),
            "premium_numeraire" => Some(WireVal::F64(self.premium_numeraire)),
            "vega_ladder" => Some(WireVal::RepeatedMsg(
                self.vega_ladder
                    .iter()
                    .map(|b| b as &dyn WireAdapter)
                    .collect(),
            )),
            _ => None,
        }
    }
}

impl WireAdapter for NonAdditiveRisk {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        // Every field is a presence-tracked optional scalar: absent ⇒ `None` here,
        // rendered as JSON `null` by the `null_absent_optional` policy for this
        // message (a not-evaluated measure is `null`, never a spurious zero).
        match proto_name {
            "var" => self.var.map(WireVal::F64),
            "es" => self.es.map(WireVal::F64),
            "var_alpha" => self.var_alpha.map(WireVal::F64),
            "curvature_spot" => self.curvature_spot.map(WireVal::F64),
            _ => None,
        }
    }
}

impl WireAdapter for RiskNode {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "dimension" => Some(WireVal::Enum(self.dimension)),
            "group" => Some(WireVal::U64(self.group)),
            "additive" => self
                .additive
                .as_ref()
                .map(|a| WireVal::Msg(a as &dyn WireAdapter)),
            "nonadditive" => self
                .nonadditive
                .as_ref()
                .map(|n| WireVal::Msg(n as &dyn WireAdapter)),
            "position_count" => Some(WireVal::U64(u64::from(self.position_count))),
            _ => None,
        }
    }
}

impl WireAdapter for LimitUtilization {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "metric" => Some(WireVal::Enum(self.metric)),
            "vega_pillar" => self
                .vega_pillar
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            "tenor_days" => Some(WireVal::U64(u64::from(self.tenor_days))),
            "cap" => Some(WireVal::F64(self.cap)),
            "exposure" => Some(WireVal::F64(self.exposure)),
            "ratio" => Some(WireVal::F64(self.ratio)),
            "status" => Some(WireVal::Enum(self.status)),
            "enforcement" => Some(WireVal::Enum(self.enforcement)),
            "headroom" => Some(WireVal::F64(self.headroom)),
            _ => None,
        }
    }
}

impl WireAdapter for KeyRateDv01 {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor_years" => Some(WireVal::U64(u64::from(self.tenor_years))),
            "dv01" => Some(WireVal::F64(self.dv01)),
            _ => None,
        }
    }
}

impl WireAdapter for RatesRiskNode {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "ccy" => Some(WireVal::Str(&self.ccy)),
            "net_pv" => Some(WireVal::F64(self.net_pv)),
            "net_pv01" => Some(WireVal::F64(self.net_pv01)),
            "net_dv01" => Some(WireVal::F64(self.net_dv01)),
            "key_rate_ladder" => Some(WireVal::RepeatedMsg(
                self.key_rate_ladder
                    .iter()
                    .map(|k| k as &dyn WireAdapter)
                    .collect(),
            )),
            _ => None,
        }
    }
}

impl WireAdapter for ListPositionsResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "positions" => Some(WireVal::RepeatedMsg(
                self.positions
                    .iter()
                    .map(|p| p as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for AggregateRiskResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "dimension" => Some(WireVal::Enum(self.dimension)),
            "numeraire" => Some(WireVal::Str(&self.numeraire)),
            "nodes" => Some(WireVal::RepeatedMsg(
                self.nodes.iter().map(|n| n as &dyn WireAdapter).collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DrillRiskResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "node" => self
                .node
                .as_ref()
                .map(|n| WireVal::Msg(n as &dyn WireAdapter)),
            "children" => Some(WireVal::RepeatedMsg(
                self.children
                    .iter()
                    .map(|n| n as &dyn WireAdapter)
                    .collect(),
            )),
            "positions" => Some(WireVal::RepeatedMsg(
                self.positions
                    .iter()
                    .map(|p| p as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for LimitStatusResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "scope" => self
                .scope
                .as_ref()
                .map(|s| WireVal::Msg(s as &dyn WireAdapter)),
            "limits" => Some(WireVal::RepeatedMsg(
                self.limits.iter().map(|l| l as &dyn WireAdapter).collect(),
            )),
            "worst" => Some(WireVal::Enum(self.worst)),
            "hard_breach" => Some(WireVal::Bool(self.hard_breach)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for AggregateRatesRiskResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "nodes" => Some(WireVal::RepeatedMsg(
                self.nodes.iter().map(|n| n as &dyn WireAdapter).collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for BookRatesPositionResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "position" => self
                .position
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for ListRatesPositionsResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "positions" => Some(WireVal::RepeatedMsg(
                self.positions
                    .iter()
                    .map(|p| p as &dyn WireAdapter)
                    .collect(),
            )),
            _ => None,
        }
    }
}

/// Encode a [`ListPositionsResponse`] to its WS JSON (mirrors the hand
/// `list_positions_response_to_json`).
#[must_use]
pub fn encode_list_positions_response(r: &ListPositionsResponse) -> Value {
    encode("ListPositionsResponse", r)
}

/// Encode an [`AggregateRiskResponse`] to its WS JSON (mirrors the hand
/// `aggregate_risk_response_to_json`).
#[must_use]
pub fn encode_aggregate_risk_response(r: &AggregateRiskResponse) -> Value {
    encode("AggregateRiskResponse", r)
}

/// Encode a [`DrillRiskResponse`] to its WS JSON (mirrors the hand
/// `drill_risk_response_to_json`).
#[must_use]
pub fn encode_drill_risk_response(r: &DrillRiskResponse) -> Value {
    encode("DrillRiskResponse", r)
}

/// Encode a [`LimitStatusResponse`] to its WS JSON (mirrors the hand
/// `limit_status_response_to_json`).
#[must_use]
pub fn encode_limit_status_response(r: &LimitStatusResponse) -> Value {
    encode("LimitStatusResponse", r)
}

/// Encode an [`AggregateRatesRiskResponse`] to its WS JSON (mirrors the hand
/// `aggregate_rates_risk_response_to_json`).
#[must_use]
pub fn encode_aggregate_rates_risk_response(r: &AggregateRatesRiskResponse) -> Value {
    encode("AggregateRatesRiskResponse", r)
}

/// Encode a [`BookRatesPositionResponse`] to its WS JSON (mirrors the hand
/// `book_rates_position_response_to_json`).
#[must_use]
pub fn encode_book_rates_position_response(r: &BookRatesPositionResponse) -> Value {
    encode("BookRatesPositionResponse", r)
}

/// Encode a [`ListRatesPositionsResponse`] to its WS JSON (mirrors the hand
/// `list_rates_positions_response_to_json`).
#[must_use]
pub fn encode_list_rates_positions_response(r: &ListRatesPositionsResponse) -> Value {
    encode("ListRatesPositionsResponse", r)
}

// ===========================================================================
// RfqDeskService — dealer-side RFQ/IOI desk inbox + deal blotter (arch item G —
// `ws-codec-from-proto`, wave 3)
// ===========================================================================
//
// Five unary verbs (`SubmitDeskRequest` / `RespondDeskRequest` / `AcceptDeskQuote` /
// `ListDeskRequests` / `ListDeals`). `SubmitDeskRequest` / `AcceptDeskQuote` /
// `ListDeskRequests` decode fully generically (the FI `RatesInstrument`/`CurveSet`
// reuse the shared rates builders). Two decode quirks the field table cannot express
// keep the message-level projection escape hatch: `RespondDeskRequest`'s `response`
// oneof errors when BOTH `quote` and `reject` are present (a mutual-exclusion the
// generic first-arm-wins walk cannot express, plus `contains_key` — not
// present-and-non-null — presence, [`decode_respond_desk_request`]); and `ListDeals`'
// `scope` is silently dropped when it is not a JSON object rather than erroring
// (`and_then(as_object)`, [`decode_list_deals`]). The reply surface (`DeskRequest` /
// `Deal`, reusing the rates encode tree) renders every absent presence-tracked field
// (`quote` / `position_id` / `correlation_id`) as JSON `null`
// (`codec_overrides::null_absent_optional`).

/// A repeated proto enum (`Vec<i32>`) defaulting to empty (mirrors the hand
/// `DeskRequestScope.states` decode): absent/non-array ⇒ empty, and each present
/// element is a truncating `as i32` cast of its JSON integer (byte-identical to the
/// hand `filter_map(as_i64).map(|n| n as i32)`), silently dropping non-integers.
fn enum_vec(value: Option<&Value>) -> Vec<i32> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_i64)
                .map(|n| n as i32)
                .collect()
        })
        .unwrap_or_default()
}

// --- desk request WireBuilders (decode) -------------------------------------

impl WireBuilder for DeskQuote {
    const MESSAGE: &'static str = "DeskQuote";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "price" => self.price = req_f64(value, "price")?,
            "notional" => self.notional = req_f64(value, "notional")?,
            "valid_for_ms" => self.valid_for_ms = u32_or_zero(value),
            "trader" => self.trader = string_or_empty(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeskReject {
    const MESSAGE: &'static str = "DeskReject";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "reason" => self.reason = string_or_empty(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DealScope {
    const MESSAGE: &'static str = "DealScope";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "desk" => self.desk = opt_string(value, "desk")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeskRequestScope {
    const MESSAGE: &'static str = "DeskRequestScope";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "states" => self.states = enum_vec(value),
            "desk" => self.desk = opt_string(value, "desk")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for SubmitDeskRequestRequest {
    const MESSAGE: &'static str = "SubmitDeskRequestRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            "kind" => self.kind = enum_or_zero(value),
            "counterparty" => self.counterparty = string_or_empty(value),
            "desk" => self.desk = string_or_empty(value),
            "instrument" => {
                self.instrument = Some(req_msg::<RatesInstrument>(value, "instrument")?);
            }
            "curve_set" => self.curve_set = Some(req_msg::<CurveSet>(value, "curve_set")?),
            "side" => self.side = enum_or_zero(value),
            "notional" => self.notional = f64_or_zero(value),
            "ttl_ms" => self.ttl_ms = u32_or_zero(value),
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_string(value, "correlation_id")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for AcceptDeskQuoteRequest {
    const MESSAGE: &'static str = "AcceptDeskQuoteRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            "request_id" => self.request_id = string_or_empty(value),
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_string(value, "correlation_id")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListDeskRequestsRequest {
    const MESSAGE: &'static str = "ListDeskRequestsRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = opt_string(value, "session_token")?,
            "scope" => self.scope = opt_msg::<DeskRequestScope>(value, "scope")?,
            "principal" => self.principal = opt_msg::<EntitlementPrincipal>(value, "principal")?,
            "correlation_id" => self.correlation_id = opt_string(value, "correlation_id")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

/// Decode a [`SubmitDeskRequestRequest`] envelope — fully generic (the FI
/// `RatesInstrument` / `CurveSet` reuse the shared rates builders).
///
/// # Errors
/// A missing required `instrument` / `curve_set` (or the `RatesInstrument` arm) or a
/// malformed body, as a [`CodecError`].
pub fn decode_submit_desk_request(o: &Map<String, Value>) -> DResult<SubmitDeskRequestRequest> {
    decode(SubmitDeskRequestRequest::MESSAGE, o)
}

/// Decode a [`RespondDeskRequestRequest`] envelope (mirrors the hand
/// `respond_desk_request_from_json`) — a **message-level projection** because its
/// `response` oneof is mutually exclusive: exactly one of `quote` / `reject` selects
/// the arm, BOTH present is an error, and presence is by key (`contains_key`, so a
/// `quote: null` reaches the `DeskQuote` decoder and is rejected as a non-object),
/// neither of which the generic first-arm-wins oneof walk expresses. The two arm
/// bodies decode through the shared generated `DeskQuote` / `DeskReject` builders.
///
/// # Errors
/// Both arms present, a malformed arm/principal body, as a [`CodecError`].
pub fn decode_respond_desk_request(o: &Map<String, Value>) -> DResult<RespondDeskRequestRequest> {
    let response = match (o.contains_key("quote"), o.contains_key("reject")) {
        (true, false) => Some(RespondArm::Quote(decode::<DeskQuote>(
            DeskQuote::MESSAGE,
            obj(o.get("quote").expect("present by contains_key"), "quote")?,
        )?)),
        (false, true) => Some(RespondArm::Reject(decode::<DeskReject>(
            DeskReject::MESSAGE,
            obj(o.get("reject").expect("present by contains_key"), "reject")?,
        )?)),
        (false, false) => None,
        (true, true) => {
            return Err(CodecError(
                "respond: set exactly one of `quote` / `reject`, not both".to_owned(),
            ));
        }
    };
    Ok(RespondDeskRequestRequest {
        session_token: opt_string(
            o.get("session_token").filter(|v| !v.is_null()),
            "session_token",
        )?,
        request_id: string_or_empty(o.get("request_id").filter(|v| !v.is_null())),
        principal: opt_msg::<EntitlementPrincipal>(
            o.get("principal").filter(|v| !v.is_null()),
            "principal",
        )?,
        correlation_id: opt_string(
            o.get("correlation_id").filter(|v| !v.is_null()),
            "correlation_id",
        )?,
        response,
    })
}

/// Decode an [`AcceptDeskQuoteRequest`] envelope — fully generic.
///
/// # Errors
/// A malformed nested body, as a [`CodecError`].
pub fn decode_accept_desk_quote(o: &Map<String, Value>) -> DResult<AcceptDeskQuoteRequest> {
    decode(AcceptDeskQuoteRequest::MESSAGE, o)
}

/// Decode a [`ListDeskRequestsRequest`] envelope — fully generic (the `scope`
/// `DeskRequestScope` decodes through its generic builder, erroring on a non-object
/// scope exactly as the hand `opt_nested` does).
///
/// # Errors
/// A malformed nested body, as a [`CodecError`].
pub fn decode_list_desk_requests(o: &Map<String, Value>) -> DResult<ListDeskRequestsRequest> {
    decode(ListDeskRequestsRequest::MESSAGE, o)
}

/// Decode a [`ListDealsRequest`] envelope (mirrors the hand `list_deals_from_json`)
/// — a **message-level projection** for the `scope` quirk: a `DealScope` present but
/// NOT a JSON object (or `null`) is silently dropped to `None` rather than erroring
/// (`and_then(as_object)`), unlike the erroring `opt_nested`/`opt_msg` the other
/// scoped verbs use. A present-object scope decodes through the generic `DealScope`
/// builder.
///
/// # Errors
/// A malformed `principal` body, as a [`CodecError`].
pub fn decode_list_deals(o: &Map<String, Value>) -> DResult<ListDealsRequest> {
    // The non-erroring scope: absent / null / non-object ⇒ `None`; a JSON object ⇒
    // the decoded `DealScope`.
    let scope = match o.get("scope") {
        Some(Value::Object(s)) => Some(decode::<DealScope>(DealScope::MESSAGE, s)?),
        _ => None,
    };
    Ok(ListDealsRequest {
        session_token: opt_string(
            o.get("session_token").filter(|v| !v.is_null()),
            "session_token",
        )?,
        scope,
        principal: opt_msg::<EntitlementPrincipal>(
            o.get("principal").filter(|v| !v.is_null()),
            "principal",
        )?,
        correlation_id: opt_string(
            o.get("correlation_id").filter(|v| !v.is_null()),
            "correlation_id",
        )?,
    })
}

// --- notification push WireAdapter (encode) ---------------------------------

impl WireAdapter for Notification {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "notification_id" => Some(WireVal::Str(&self.notification_id)),
            "kind" => Some(WireVal::Enum(self.kind)),
            "at_nanos" => Some(WireVal::I64(self.at_nanos)),
            "request_id" => self.request_id.as_deref().map(WireVal::Str),
            "desk" => Some(WireVal::Str(&self.desk)),
            "counterparty" => Some(WireVal::Str(&self.counterparty)),
            "request_kind" => Some(WireVal::Enum(self.request_kind)),
            "headline" => Some(WireVal::Str(&self.headline)),
            "detail" => self.detail.as_deref().map(WireVal::Str),
            "alert_worthy" => Some(WireVal::Bool(self.alert_worthy)),
            "reason" => self.reason.map(WireVal::Enum),
            _ => None,
        }
    }

    fn synthesized(&self) -> Vec<(&'static str, Value)> {
        // The push-frame discriminator the WS drain stamps ahead of the proto field
        // body (mirrors `codec::notification_to_json`'s inline `"type":"notification"`).
        vec![("type", json!("notification"))]
    }
}

/// Encode a `Notification` push frame — the descriptor-driven mirror of the hand
/// [`super::codec::notification_to_json`]. Proven byte-identical by the differential
/// harness (`ws_codec_differential`), including the `alert_worthy` flag and the
/// presence-tracked manual-intervention `reason`.
#[must_use]
pub fn encode_notification(n: &Notification) -> Value {
    encode("Notification", n)
}

// --- desk reply WireAdapters (encode) ---------------------------------------

impl WireAdapter for DeskQuote {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "price" => Some(WireVal::F64(self.price)),
            "notional" => Some(WireVal::F64(self.notional)),
            "valid_for_ms" => Some(WireVal::U64(u64::from(self.valid_for_ms))),
            "trader" => Some(WireVal::Str(&self.trader)),
            _ => None,
        }
    }
}

impl WireAdapter for DeskRequest {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "request_id" => Some(WireVal::Str(&self.request_id)),
            "kind" => Some(WireVal::Enum(self.kind)),
            "counterparty" => Some(WireVal::Str(&self.counterparty)),
            "desk" => Some(WireVal::Str(&self.desk)),
            "instrument" => self
                .instrument
                .as_ref()
                .map(|i| WireVal::Msg(i as &dyn WireAdapter)),
            "curve_set" => self
                .curve_set
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "side" => Some(WireVal::Enum(self.side)),
            "notional" => Some(WireVal::F64(self.notional)),
            "received_at_nanos" => Some(WireVal::I64(self.received_at_nanos)),
            "expires_at_nanos" => Some(WireVal::I64(self.expires_at_nanos)),
            "state" => Some(WireVal::Enum(self.state)),
            "quote" => self
                .quote
                .as_ref()
                .map(|q| WireVal::Msg(q as &dyn WireAdapter)),
            "correlation_id" => self.correlation_id.as_deref().map(WireVal::Str),
            _ => None,
        }
    }
}

impl WireAdapter for Deal {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "deal_id" => Some(WireVal::Str(&self.deal_id)),
            "request_id" => Some(WireVal::Str(&self.request_id)),
            "kind" => Some(WireVal::Enum(self.kind)),
            "counterparty" => Some(WireVal::Str(&self.counterparty)),
            "desk" => Some(WireVal::Str(&self.desk)),
            "instrument" => self
                .instrument
                .as_ref()
                .map(|i| WireVal::Msg(i as &dyn WireAdapter)),
            "curve_set" => self
                .curve_set
                .as_ref()
                .map(|c| WireVal::Msg(c as &dyn WireAdapter)),
            "side" => Some(WireVal::Enum(self.side)),
            "notional" => Some(WireVal::F64(self.notional)),
            "price" => Some(WireVal::F64(self.price)),
            "executed_at_nanos" => Some(WireVal::I64(self.executed_at_nanos)),
            "trader" => Some(WireVal::Str(&self.trader)),
            "position_id" => self.position_id.map(WireVal::U64),
            "correlation_id" => self.correlation_id.as_deref().map(WireVal::Str),
            "pricing_provenance" => self
                .pricing_provenance
                .as_ref()
                .map(|p| WireVal::Msg(p as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for SubmitDeskRequestResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "request" => self
                .request
                .as_ref()
                .map(|r| WireVal::Msg(r as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for RespondDeskRequestResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "request" => self
                .request
                .as_ref()
                .map(|r| WireVal::Msg(r as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for AcceptDeskQuoteResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "deal" => self
                .deal
                .as_ref()
                .map(|d| WireVal::Msg(d as &dyn WireAdapter)),
            "request" => self
                .request
                .as_ref()
                .map(|r| WireVal::Msg(r as &dyn WireAdapter)),
            _ => None,
        }
    }
}

impl WireAdapter for ListDeskRequestsResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "requests" => Some(WireVal::RepeatedMsg(
                self.requests
                    .iter()
                    .map(|r| r as &dyn WireAdapter)
                    .collect(),
            )),
            _ => None,
        }
    }
}

impl WireAdapter for ListDealsResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "deals" => Some(WireVal::RepeatedMsg(
                self.deals.iter().map(|d| d as &dyn WireAdapter).collect(),
            )),
            _ => None,
        }
    }
}

/// Encode a [`SubmitDeskRequestResponse`] to its WS JSON (mirrors the hand
/// `submit_desk_request_response_to_json`).
#[must_use]
pub fn encode_submit_desk_request_response(r: &SubmitDeskRequestResponse) -> Value {
    encode("SubmitDeskRequestResponse", r)
}

/// Encode a [`RespondDeskRequestResponse`] to its WS JSON (mirrors the hand
/// `respond_desk_request_response_to_json`).
#[must_use]
pub fn encode_respond_desk_request_response(r: &RespondDeskRequestResponse) -> Value {
    encode("RespondDeskRequestResponse", r)
}

/// Encode an [`AcceptDeskQuoteResponse`] to its WS JSON (mirrors the hand
/// `accept_desk_quote_response_to_json`).
#[must_use]
pub fn encode_accept_desk_quote_response(r: &AcceptDeskQuoteResponse) -> Value {
    encode("AcceptDeskQuoteResponse", r)
}

/// Encode a [`ListDeskRequestsResponse`] to its WS JSON (mirrors the hand
/// `list_desk_requests_response_to_json`).
#[must_use]
pub fn encode_list_desk_requests_response(r: &ListDeskRequestsResponse) -> Value {
    encode("ListDeskRequestsResponse", r)
}

/// Encode a [`ListDealsResponse`] to its WS JSON (mirrors the hand
/// `list_deals_response_to_json`).
#[must_use]
pub fn encode_list_deals_response(r: &ListDealsResponse) -> Value {
    encode("ListDealsResponse", r)
}

// ===========================================================================
// AuthService — the admin + session surface (wave 4, arch item G —
// `ws-codec-from-proto`, the FINAL family): login/session, user / desk / entity /
// book CRUD, capabilities + roles, the instrument registry (`InstrumentDefDesc` +
// its `definition` family oneof) and `BuildCurve`. The request decoders and reply
// encoders below run purely on the descriptor field tables + the shared presence
// helpers; the `ws_codec_differential` harness proves each is byte-identical to the
// hand codec (retained as the frozen oracle in `super::codec::diff_support`) over
// the auth conformance shapes + edge vectors (absent optionals, empty/whitespace
// strings, the `capabilities` repeated-message lists, the instrument family oneof
// with its `calendars` repeated-string + `BondDef` optional coupon dates).
// ===========================================================================

// --- shared decode helper: the lenient `external_ids` list --------------------

/// Decode the `InstrumentDefDesc.external_ids` list exactly as the hand
/// `instrument_def_from_json`: a non-array (or absent) value yields an empty vec
/// (NOT an error), and each present element is an object decoded through
/// [`ExternalId`]'s builder under the `external_id` error label.
///
/// # Errors
/// A non-object element, or an element missing `scheme`/`value`, as a [`CodecError`].
fn external_ids(value: Option<&Value>) -> DResult<Vec<ExternalId>> {
    match value.and_then(Value::as_array) {
        None => Ok(Vec::new()),
        Some(items) => items
            .iter()
            .map(|e| decode(ExternalId::MESSAGE, obj(e, "external_id")?))
            .collect(),
    }
}

// --- auth nested-message builders (decode) -----------------------------------

impl WireBuilder for CapabilityDesc {
    const MESSAGE: &'static str = "CapabilityDesc";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "action" => self.action = req_string(value, "action")?,
            "asset" => self.asset = req_string(value, "asset")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ExternalId {
    const MESSAGE: &'static str = "ExternalId";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "scheme" => self.scheme = req_string(value, "scheme")?,
            "value" => self.value = req_string(value, "value")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DepositDef {
    const MESSAGE: &'static str = "DepositDef";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "index" => self.index = req_string(value, "index")?,
            "tenor" => self.tenor = req_string(value, "tenor")?,
            "day_count" => self.day_count = req_string(value, "day_count")?,
            "business_day_convention" => {
                self.business_day_convention = req_string(value, "business_day_convention")?;
            }
            "calendars" => self.calendars = string_vec(value),
            "spot_lag_days" => self.spot_lag_days = u32_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for FraDef {
    const MESSAGE: &'static str = "FraDef";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "float_index" => self.float_index = req_string(value, "float_index")?,
            "start_tenor" => self.start_tenor = req_string(value, "start_tenor")?,
            "end_tenor" => self.end_tenor = req_string(value, "end_tenor")?,
            "accrual_day_count" => self.accrual_day_count = req_string(value, "accrual_day_count")?,
            "business_day_convention" => {
                self.business_day_convention = req_string(value, "business_day_convention")?;
            }
            "calendars" => self.calendars = string_vec(value),
            "spot_lag_days" => self.spot_lag_days = u32_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for StirFutureDef {
    const MESSAGE: &'static str = "StirFutureDef";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "contract_code" => self.contract_code = req_string(value, "contract_code")?,
            "reference_start" => self.reference_start = req_string(value, "reference_start")?,
            "reference_end" => self.reference_end = req_string(value, "reference_end")?,
            "day_count" => self.day_count = req_string(value, "day_count")?,
            "calendars" => self.calendars = string_vec(value),
            "convexity_vol" => self.convexity_vol = f64_or_zero(value),
            "contract_size" => self.contract_size = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for VanillaIrsDef {
    const MESSAGE: &'static str = "VanillaIrsDef";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor" => self.tenor = req_string(value, "tenor")?,
            "fixed_frequency" => self.fixed_frequency = req_string(value, "fixed_frequency")?,
            "fixed_day_count" => self.fixed_day_count = req_string(value, "fixed_day_count")?,
            "float_index" => self.float_index = req_string(value, "float_index")?,
            "float_frequency" => self.float_frequency = req_string(value, "float_frequency")?,
            "float_day_count" => self.float_day_count = req_string(value, "float_day_count")?,
            "business_day_convention" => {
                self.business_day_convention = req_string(value, "business_day_convention")?;
            }
            "calendars" => self.calendars = string_vec(value),
            "roll_convention" => self.roll_convention = string_or_empty(value),
            "spot_lag_days" => self.spot_lag_days = u32_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for OisDef {
    const MESSAGE: &'static str = "OisDef";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "tenor" => self.tenor = req_string(value, "tenor")?,
            "index" => self.index = req_string(value, "index")?,
            "fixed_frequency" => self.fixed_frequency = req_string(value, "fixed_frequency")?,
            "fixed_day_count" => self.fixed_day_count = req_string(value, "fixed_day_count")?,
            "float_day_count" => self.float_day_count = req_string(value, "float_day_count")?,
            "business_day_convention" => {
                self.business_day_convention = req_string(value, "business_day_convention")?;
            }
            "calendars" => self.calendars = string_vec(value),
            "spot_lag_days" => self.spot_lag_days = u32_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for BondDef {
    const MESSAGE: &'static str = "BondDef";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "issuer" => self.issuer = req_string(value, "issuer")?,
            "coupon_rate" => self.coupon_rate = f64_or_zero(value),
            "coupon_type" => self.coupon_type = req_string(value, "coupon_type")?,
            "coupon_frequency" => self.coupon_frequency = string_or_empty(value),
            "day_count" => self.day_count = req_string(value, "day_count")?,
            "issue_date" => self.issue_date = opt_msg::<BrokenDate>(value, "issue_date")?,
            "dated_date" => self.dated_date = opt_msg::<BrokenDate>(value, "dated_date")?,
            "first_coupon_date" => {
                self.first_coupon_date = opt_msg::<BrokenDate>(value, "first_coupon_date")?;
            }
            "maturity_date" => self.maturity_date = opt_msg::<BrokenDate>(value, "maturity_date")?,
            "redemption" => self.redemption = f64_or_zero(value),
            "calendars" => self.calendars = string_vec(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for InstrumentDefDesc {
    const MESSAGE: &'static str = "InstrumentDefDesc";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            // `instrument_id`/`description` mirror `opt_string(..).unwrap_or_default()`
            // (= `string_or_empty`): an absent OR empty value decodes to "".
            "instrument_id" => self.instrument_id = string_or_empty(value),
            "name" => self.name = req_string(value, "name")?,
            "description" => self.description = string_or_empty(value),
            "currency" => self.currency = req_string(value, "currency")?,
            "external_ids" => self.external_ids = external_ids(value)?,
            // The `definition` family oneof: the generic decoder has already selected
            // the single live arm (first present in declaration order — the same
            // precedence as the hand `family_from_json`).
            "deposit" => {
                self.definition = Some(InstrumentDefinition::Deposit(req_msg::<DepositDef>(
                    value, "deposit",
                )?));
            }
            "fra" => {
                self.definition = Some(InstrumentDefinition::Fra(req_msg::<FraDef>(value, "fra")?));
            }
            "stir_future" => {
                self.definition = Some(InstrumentDefinition::StirFuture(req_msg::<StirFutureDef>(
                    value,
                    "stir_future",
                )?));
            }
            "vanilla_irs" => {
                self.definition = Some(InstrumentDefinition::VanillaIrs(req_msg::<VanillaIrsDef>(
                    value,
                    "vanilla_irs",
                )?));
            }
            "ois" => {
                self.definition = Some(InstrumentDefinition::Ois(req_msg::<OisDef>(value, "ois")?));
            }
            "bond" => {
                self.definition = Some(InstrumentDefinition::Bond(req_msg::<BondDef>(
                    value, "bond",
                )?));
            }
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for InstrumentQuote {
    const MESSAGE: &'static str = "InstrumentQuote";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "instrument_id" => self.instrument_id = req_string(value, "instrument_id")?,
            "quote" => self.quote = req_f64(value, "quote")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DatePillar {
    const MESSAGE: &'static str = "DatePillar";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "maturity_date" => {
                self.maturity_date = Some(req_msg::<BrokenDate>(value, "maturity_date")?);
            }
            "quote" => self.quote = req_f64(value, "quote")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- auth request-envelope builders (decode) ---------------------------------

impl WireBuilder for LoginRequest {
    const MESSAGE: &'static str = "LoginRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "email" => self.email = req_string(value, "email")?,
            "password" => self.password = req_string(value, "password")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for LogoutRequest {
    const MESSAGE: &'static str = "LogoutRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListUsersRequest {
    const MESSAGE: &'static str = "ListUsersRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CreateUserRequest {
    const MESSAGE: &'static str = "CreateUserRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "email" => self.email = req_string(value, "email")?,
            "display_name" => self.display_name = req_string(value, "display_name")?,
            "role" => self.role = enum_or_zero(value),
            "desk_ids" => self.desk_ids = string_vec(value),
            "password" => self.password = req_string(value, "password")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            "all_desks" => self.all_desks = bool_or_false(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdateUserRequest {
    const MESSAGE: &'static str = "UpdateUserRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "display_name" => self.display_name = req_string(value, "display_name")?,
            "role" => self.role = enum_or_zero(value),
            "desk_ids" => self.desk_ids = string_vec(value),
            "disabled" => self.disabled = bool_or_false(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            "all_desks" => self.all_desks = bool_or_false(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeleteUserRequest {
    const MESSAGE: &'static str = "DeleteUserRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ResetPasswordRequest {
    const MESSAGE: &'static str = "ResetPasswordRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "new_password" => self.new_password = req_string(value, "new_password")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for GetUserCapabilitiesRequest {
    const MESSAGE: &'static str = "GetUserCapabilitiesRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for SetUserCapabilitiesRequest {
    const MESSAGE: &'static str = "SetUserCapabilitiesRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "grants" => self.grants = opt_repeated::<CapabilityDesc>(value, "grants")?,
            "denies" => self.denies = opt_repeated::<CapabilityDesc>(value, "denies")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for GetRoleCapabilitiesRequest {
    const MESSAGE: &'static str = "GetRoleCapabilitiesRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "role" => self.role = enum_or_zero(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for SetRoleCapabilitiesRequest {
    const MESSAGE: &'static str = "SetRoleCapabilitiesRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "role" => self.role = enum_or_zero(value),
            "capabilities" => {
                self.capabilities = opt_repeated::<CapabilityDesc>(value, "capabilities")?;
            }
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListDesksRequest {
    const MESSAGE: &'static str = "ListDesksRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CreateDeskRequest {
    const MESSAGE: &'static str = "CreateDeskRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "name" => self.name = req_string(value, "name")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdateDeskRequest {
    const MESSAGE: &'static str = "UpdateDeskRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "name" => self.name = req_string(value, "name")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeleteDeskRequest {
    const MESSAGE: &'static str = "DeleteDeskRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListEntitiesRequest {
    const MESSAGE: &'static str = "ListEntitiesRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CreateEntityRequest {
    const MESSAGE: &'static str = "CreateEntityRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "name" => self.name = req_string(value, "name")?,
            "code" => self.code = req_string(value, "code")?,
            // `opt_u32(..).unwrap_or(0)` = defaults-to-0, clamps overflow to 0.
            "key" => self.key = u32_or_zero(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdateEntityRequest {
    const MESSAGE: &'static str = "UpdateEntityRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "key" => self.key = req_u32(value, "key")?,
            "name" => self.name = req_string(value, "name")?,
            "code" => self.code = req_string(value, "code")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeleteEntityRequest {
    const MESSAGE: &'static str = "DeleteEntityRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "key" => self.key = req_u32(value, "key")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListBooksRequest {
    const MESSAGE: &'static str = "ListBooksRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CreateBookRequest {
    const MESSAGE: &'static str = "CreateBookRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "name" => self.name = req_string(value, "name")?,
            "entity_key" => self.entity_key = req_u32(value, "entity_key")?,
            "key" => self.key = u32_or_zero(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdateBookRequest {
    const MESSAGE: &'static str = "UpdateBookRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "key" => self.key = req_u32(value, "key")?,
            "name" => self.name = req_string(value, "name")?,
            "entity_key" => self.entity_key = req_u32(value, "entity_key")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeleteBookRequest {
    const MESSAGE: &'static str = "DeleteBookRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "key" => self.key = req_u32(value, "key")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for AggregationParamsDesc {
    const MESSAGE: &'static str = "AggregationParamsDesc";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "staleness_tau_ms" => self.staleness_tau_ms = req_u64(value, "staleness_tau_ms")?,
            "max_quote_age_ms" => self.max_quote_age_ms = req_u64(value, "max_quote_age_ms")?,
            "divergence_gating" => self.divergence_gating = bool_or_false(value),
            "min_contributors" => self.min_contributors = req_u32(value, "min_contributors")?,
            "depth_levels" => self.depth_levels = req_u32(value, "depth_levels")?,
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for AggregatedBookSpec {
    const MESSAGE: &'static str = "AggregatedBookSpec";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "id" => self.id = string_or_empty(value),
            "name" => self.name = req_string(value, "name")?,
            "member_connection_ids" => self.member_connection_ids = string_vec(value),
            "scope_mode" => self.scope_mode = enum_or_zero(value),
            "instrument_ids" => self.instrument_ids = string_vec(value),
            "params" => self.params = opt_msg::<AggregationParamsDesc>(value, "params")?,
            "enabled" => self.enabled = bool_or_false(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for TieringConfigDesc {
    const MESSAGE: &'static str = "TieringConfigDesc";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "unit" => self.unit = enum_or_zero(value),
            "strategies" => {
                self.strategies = opt_repeated::<TieringStrategyDesc>(value, "strategy")?;
            }
            "guardrails" => {
                self.guardrails = opt_msg::<TieringGuardrailsDesc>(value, "guardrails")?;
            }
            "stale_policy" => self.stale_policy = enum_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for TieringStrategyDesc {
    const MESSAGE: &'static str = "TieringStrategyDesc";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "kind" => self.kind = enum_or_zero(value),
            "half_spread" => self.half_spread = f64_or_zero(value),
            "kappa" => self.kappa = f64_or_zero(value),
            "s_max" => self.s_max = f64_or_zero(value),
            "smoothing_weight" => self.smoothing_weight = f64_or_zero(value),
            "expected_spread" => self.expected_spread = f64_or_zero(value),
            "max_divergence" => self.max_divergence = f64_or_zero(value),
            "core_spread" => self.core_spread = f64_or_zero(value),
            "max_output_spread" => self.max_output_spread = f64_or_zero(value),
            "spread_scale_factor" => self.spread_scale_factor = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for TieringGuardrailsDesc {
    const MESSAGE: &'static str = "TieringGuardrailsDesc";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "h_min" => self.h_min = f64_or_zero(value),
            "h_max" => self.h_max = f64_or_zero(value),
            "s_max" => self.s_max = f64_or_zero(value),
            "spread_floor" => self.spread_floor = f64_or_zero(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListAggregatedBooksRequest {
    const MESSAGE: &'static str = "ListAggregatedBooksRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CreateAggregatedBookRequest {
    const MESSAGE: &'static str = "CreateAggregatedBookRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "spec" => self.spec = Some(req_msg::<AggregatedBookSpec>(value, "spec")?),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdateAggregatedBookRequest {
    const MESSAGE: &'static str = "UpdateAggregatedBookRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "spec" => self.spec = Some(req_msg::<AggregatedBookSpec>(value, "spec")?),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeleteAggregatedBookRequest {
    const MESSAGE: &'static str = "DeleteAggregatedBookRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for FeatureSpecDesc {
    const MESSAGE: &'static str = "FeatureSpecDesc";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "kind" => self.kind = enum_or_zero(value),
            "unit" => self.unit = enum_or_zero(value),
            "shift" => self.shift = f64_or_zero(value),
            "reference" => self.reference = opt_f64(value),
            "tiering" => self.tiering = opt_msg::<TieringConfigDesc>(value, "tiering")?,
            "axe_side" => self.axe_side = enum_or_zero(value),
            "magnitude" => self.magnitude = f64_or_zero(value),
            "kappa" => self.kappa = f64_or_zero(value),
            "s_max" => self.s_max = f64_or_zero(value),
            "skew" => self.skew = f64_or_zero(value),
            "triggered" => self.triggered = bool_or_false(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for FeaturePipelineDesc {
    const MESSAGE: &'static str = "FeaturePipelineDesc";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "features" => self.features = opt_repeated::<FeatureSpecDesc>(value, "feature")?,
            "guardrails" => {
                self.guardrails = opt_msg::<TieringGuardrailsDesc>(value, "guardrails")?;
            }
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for PricingGroupSpec {
    const MESSAGE: &'static str = "PricingGroupSpec";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "id" => self.id = string_or_empty(value),
            "name" => self.name = req_string(value, "name")?,
            "description" => self.description = string_or_empty(value),
            "member_connection_ids" => self.member_connection_ids = string_vec(value),
            "member_user_ids" => self.member_user_ids = string_vec(value),
            "member_desks" => self.member_desks = string_vec(value),
            "esp_pipeline" => {
                self.esp_pipeline = opt_msg::<FeaturePipelineDesc>(value, "esp_pipeline")?;
            }
            "rfq_pipeline" => {
                self.rfq_pipeline = opt_msg::<FeaturePipelineDesc>(value, "rfq_pipeline")?;
            }
            "share_pipeline" => self.share_pipeline = bool_or_false(value),
            "enabled" => self.enabled = bool_or_false(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListPricingGroupsRequest {
    const MESSAGE: &'static str = "ListPricingGroupsRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CreatePricingGroupRequest {
    const MESSAGE: &'static str = "CreatePricingGroupRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "spec" => self.spec = Some(req_msg::<PricingGroupSpec>(value, "spec")?),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdatePricingGroupRequest {
    const MESSAGE: &'static str = "UpdatePricingGroupRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "spec" => self.spec = Some(req_msg::<PricingGroupSpec>(value, "spec")?),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeletePricingGroupRequest {
    const MESSAGE: &'static str = "DeletePricingGroupRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "id" => self.id = req_string(value, "id")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdatePricingGroupPipelineRequest {
    const MESSAGE: &'static str = "UpdatePricingGroupPipelineRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "group_id" => self.group_id = req_string(value, "group_id")?,
            "mode" => self.mode = enum_or_zero(value),
            "pipeline" => self.pipeline = opt_msg::<FeaturePipelineDesc>(value, "pipeline")?,
            "share_pipeline" => self.share_pipeline = bool_or_false(value),
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for ListInstrumentsRequest {
    const MESSAGE: &'static str = "ListInstrumentsRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for GetInstrumentRequest {
    const MESSAGE: &'static str = "GetInstrumentRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "instrument_id" => self.instrument_id = req_string(value, "instrument_id")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for CreateInstrumentRequest {
    const MESSAGE: &'static str = "CreateInstrumentRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "instrument" => {
                self.instrument = Some(req_msg::<InstrumentDefDesc>(value, "instrument")?);
            }
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for UpdateInstrumentRequest {
    const MESSAGE: &'static str = "UpdateInstrumentRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "instrument" => {
                self.instrument = Some(req_msg::<InstrumentDefDesc>(value, "instrument")?);
            }
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for DeleteInstrumentRequest {
    const MESSAGE: &'static str = "DeleteInstrumentRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "instrument_id" => self.instrument_id = req_string(value, "instrument_id")?,
            "correlation_id" => self.correlation_id = opt_u64(value),
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

impl WireBuilder for BuildCurveRequest {
    const MESSAGE: &'static str = "BuildCurveRequest";
    fn set(&mut self, field: &WireField, value: Option<&Value>) -> DResult<()> {
        match field.proto_name {
            "request_id" => self.request_id = req_string(value, "request_id")?,
            "currency" => self.currency = req_string(value, "currency")?,
            "reference_date" => {
                self.reference_date = Some(req_msg::<BrokenDate>(value, "reference_date")?);
            }
            "pillars" => self.pillars = opt_repeated::<InstrumentQuote>(value, "pillars")?,
            "session_token" => self.session_token = req_string(value, "session_token")?,
            "date_pillars" => {
                self.date_pillars = opt_repeated::<DatePillar>(value, "date_pillars")?
            }
            other => return Err(unhandled(Self::MESSAGE, other)),
        }
        Ok(())
    }
}

// --- auth request decode entry points ----------------------------------------

/// Decode a [`LoginRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `email`/`password`, as a [`CodecError`].
pub fn decode_login_request(o: &Map<String, Value>) -> DResult<LoginRequest> {
    decode(LoginRequest::MESSAGE, o)
}

/// Decode a [`LogoutRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_logout_request(o: &Map<String, Value>) -> DResult<LogoutRequest> {
    decode(LogoutRequest::MESSAGE, o)
}

/// Decode a [`ListUsersRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_list_users_request(o: &Map<String, Value>) -> DResult<ListUsersRequest> {
    decode(ListUsersRequest::MESSAGE, o)
}

/// Decode a [`CreateUserRequest`] envelope — fully generic.
///
/// # Errors
/// A missing required string (`session_token`/`email`/`display_name`/`password`), as
/// a [`CodecError`].
pub fn decode_create_user_request(o: &Map<String, Value>) -> DResult<CreateUserRequest> {
    decode(CreateUserRequest::MESSAGE, o)
}

/// Decode an [`UpdateUserRequest`] envelope — fully generic.
///
/// # Errors
/// A missing required string (`session_token`/`id`/`display_name`), as a [`CodecError`].
pub fn decode_update_user_request(o: &Map<String, Value>) -> DResult<UpdateUserRequest> {
    decode(UpdateUserRequest::MESSAGE, o)
}

/// Decode a [`DeleteUserRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`, as a [`CodecError`].
pub fn decode_delete_user_request(o: &Map<String, Value>) -> DResult<DeleteUserRequest> {
    decode(DeleteUserRequest::MESSAGE, o)
}

/// Decode a [`ResetPasswordRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`/`new_password`, as a [`CodecError`].
pub fn decode_reset_password_request(o: &Map<String, Value>) -> DResult<ResetPasswordRequest> {
    decode(ResetPasswordRequest::MESSAGE, o)
}

/// Decode a [`GetUserCapabilitiesRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`, as a [`CodecError`].
pub fn decode_get_user_capabilities_request(
    o: &Map<String, Value>,
) -> DResult<GetUserCapabilitiesRequest> {
    decode(GetUserCapabilitiesRequest::MESSAGE, o)
}

/// Decode a [`SetUserCapabilitiesRequest`] envelope — fully generic (the `grants` /
/// `denies` capability lists default to empty when absent).
///
/// # Errors
/// A missing `session_token`/`id`, a malformed capability, as a [`CodecError`].
pub fn decode_set_user_capabilities_request(
    o: &Map<String, Value>,
) -> DResult<SetUserCapabilitiesRequest> {
    decode(SetUserCapabilitiesRequest::MESSAGE, o)
}

/// Decode a [`GetRoleCapabilitiesRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_get_role_capabilities_request(
    o: &Map<String, Value>,
) -> DResult<GetRoleCapabilitiesRequest> {
    decode(GetRoleCapabilitiesRequest::MESSAGE, o)
}

/// Decode a [`SetRoleCapabilitiesRequest`] envelope — fully generic (the
/// `capabilities` list defaults to empty when absent).
///
/// # Errors
/// A missing `session_token`, a malformed capability, as a [`CodecError`].
pub fn decode_set_role_capabilities_request(
    o: &Map<String, Value>,
) -> DResult<SetRoleCapabilitiesRequest> {
    decode(SetRoleCapabilitiesRequest::MESSAGE, o)
}

/// Decode a [`ListDesksRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_list_desks_request(o: &Map<String, Value>) -> DResult<ListDesksRequest> {
    decode(ListDesksRequest::MESSAGE, o)
}

/// Decode a [`CreateDeskRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`name`, as a [`CodecError`].
pub fn decode_create_desk_request(o: &Map<String, Value>) -> DResult<CreateDeskRequest> {
    decode(CreateDeskRequest::MESSAGE, o)
}

/// Decode an [`UpdateDeskRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`/`name`, as a [`CodecError`].
pub fn decode_update_desk_request(o: &Map<String, Value>) -> DResult<UpdateDeskRequest> {
    decode(UpdateDeskRequest::MESSAGE, o)
}

/// Decode a [`DeleteDeskRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`, as a [`CodecError`].
pub fn decode_delete_desk_request(o: &Map<String, Value>) -> DResult<DeleteDeskRequest> {
    decode(DeleteDeskRequest::MESSAGE, o)
}

/// Decode a [`ListEntitiesRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_list_entities_request(o: &Map<String, Value>) -> DResult<ListEntitiesRequest> {
    decode(ListEntitiesRequest::MESSAGE, o)
}

/// Decode a [`CreateEntityRequest`] envelope — fully generic (`key` defaults to 0).
///
/// # Errors
/// A missing `session_token`/`name`/`code`, as a [`CodecError`].
pub fn decode_create_entity_request(o: &Map<String, Value>) -> DResult<CreateEntityRequest> {
    decode(CreateEntityRequest::MESSAGE, o)
}

/// Decode an [`UpdateEntityRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`name`/`code`, a missing/out-of-range `key`, as a
/// [`CodecError`].
pub fn decode_update_entity_request(o: &Map<String, Value>) -> DResult<UpdateEntityRequest> {
    decode(UpdateEntityRequest::MESSAGE, o)
}

/// Decode a [`DeleteEntityRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, a missing/out-of-range `key`, as a [`CodecError`].
pub fn decode_delete_entity_request(o: &Map<String, Value>) -> DResult<DeleteEntityRequest> {
    decode(DeleteEntityRequest::MESSAGE, o)
}

/// Decode a [`ListBooksRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_list_books_request(o: &Map<String, Value>) -> DResult<ListBooksRequest> {
    decode(ListBooksRequest::MESSAGE, o)
}

/// Decode a [`CreateBookRequest`] envelope — fully generic (`key` defaults to 0).
///
/// # Errors
/// A missing `session_token`/`name`, a missing/out-of-range `entity_key`, as a
/// [`CodecError`].
pub fn decode_create_book_request(o: &Map<String, Value>) -> DResult<CreateBookRequest> {
    decode(CreateBookRequest::MESSAGE, o)
}

/// Decode an [`UpdateBookRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`name`, a missing/out-of-range `key`/`entity_key`, as a
/// [`CodecError`].
pub fn decode_update_book_request(o: &Map<String, Value>) -> DResult<UpdateBookRequest> {
    decode(UpdateBookRequest::MESSAGE, o)
}

/// Decode a [`DeleteBookRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, a missing/out-of-range `key`, as a [`CodecError`].
pub fn decode_delete_book_request(o: &Map<String, Value>) -> DResult<DeleteBookRequest> {
    decode(DeleteBookRequest::MESSAGE, o)
}

/// Decode a [`ListAggregatedBooksRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_list_aggregated_books_request(
    o: &Map<String, Value>,
) -> DResult<ListAggregatedBooksRequest> {
    decode(ListAggregatedBooksRequest::MESSAGE, o)
}

/// Decode a [`CreateAggregatedBookRequest`] envelope — the required `spec` nests the
/// [`AggregatedBookSpec`] body (its repeated members/ids, the `scope_mode` enum and the
/// nested `params`).
///
/// # Errors
/// A missing `session_token`, a missing/malformed `spec`, as a [`CodecError`].
pub fn decode_create_aggregated_book_request(
    o: &Map<String, Value>,
) -> DResult<CreateAggregatedBookRequest> {
    decode(CreateAggregatedBookRequest::MESSAGE, o)
}

/// Decode an [`UpdateAggregatedBookRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`, a missing/malformed `spec`, as a [`CodecError`].
pub fn decode_update_aggregated_book_request(
    o: &Map<String, Value>,
) -> DResult<UpdateAggregatedBookRequest> {
    decode(UpdateAggregatedBookRequest::MESSAGE, o)
}

/// Decode a [`DeleteAggregatedBookRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`, as a [`CodecError`].
pub fn decode_delete_aggregated_book_request(
    o: &Map<String, Value>,
) -> DResult<DeleteAggregatedBookRequest> {
    decode(DeleteAggregatedBookRequest::MESSAGE, o)
}

/// Decode a [`ListPricingGroupsRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_list_pricing_groups_request(
    o: &Map<String, Value>,
) -> DResult<ListPricingGroupsRequest> {
    decode(ListPricingGroupsRequest::MESSAGE, o)
}

/// Decode a [`CreatePricingGroupRequest`] envelope — the required `spec` nests the
/// [`PricingGroupSpec`] body (its repeated membership lists and the two nested
/// [`FeaturePipelineDesc`] pipelines, each carrying the [`FeatureSpecDesc`] array).
///
/// # Errors
/// A missing `session_token`, a missing/malformed `spec`, as a [`CodecError`].
pub fn decode_create_pricing_group_request(
    o: &Map<String, Value>,
) -> DResult<CreatePricingGroupRequest> {
    decode(CreatePricingGroupRequest::MESSAGE, o)
}

/// Decode an [`UpdatePricingGroupRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`, a missing/malformed `spec`, as a [`CodecError`].
pub fn decode_update_pricing_group_request(
    o: &Map<String, Value>,
) -> DResult<UpdatePricingGroupRequest> {
    decode(UpdatePricingGroupRequest::MESSAGE, o)
}

/// Decode a [`DeletePricingGroupRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`id`, as a [`CodecError`].
pub fn decode_delete_pricing_group_request(
    o: &Map<String, Value>,
) -> DResult<DeletePricingGroupRequest> {
    decode(DeletePricingGroupRequest::MESSAGE, o)
}

/// Decode an [`UpdatePricingGroupPipelineRequest`] envelope — the optional `pipeline`
/// nests the [`FeaturePipelineDesc`] body (its feature array + guardrails), and `mode`
/// selects which mode's pipeline it replaces.
///
/// # Errors
/// A missing `session_token`/`group_id`, or a malformed `pipeline`, as a [`CodecError`].
pub fn decode_update_pricing_group_pipeline_request(
    o: &Map<String, Value>,
) -> DResult<UpdatePricingGroupPipelineRequest> {
    decode(UpdatePricingGroupPipelineRequest::MESSAGE, o)
}

/// Decode a [`ListInstrumentsRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`, as a [`CodecError`].
pub fn decode_list_instruments_request(o: &Map<String, Value>) -> DResult<ListInstrumentsRequest> {
    decode(ListInstrumentsRequest::MESSAGE, o)
}

/// Decode a [`GetInstrumentRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`instrument_id`, as a [`CodecError`].
pub fn decode_get_instrument_request(o: &Map<String, Value>) -> DResult<GetInstrumentRequest> {
    decode(GetInstrumentRequest::MESSAGE, o)
}

/// Decode a [`CreateInstrumentRequest`] envelope — the required `instrument` nests
/// the [`InstrumentDefDesc`] registry body (its lenient `external_ids` + the
/// `definition` family oneof).
///
/// # Errors
/// A missing `session_token`, a missing/malformed `instrument`, as a [`CodecError`].
pub fn decode_create_instrument_request(
    o: &Map<String, Value>,
) -> DResult<CreateInstrumentRequest> {
    decode(CreateInstrumentRequest::MESSAGE, o)
}

/// Decode an [`UpdateInstrumentRequest`] envelope — the required `instrument` nests
/// the [`InstrumentDefDesc`] registry body.
///
/// # Errors
/// A missing `session_token`, a missing/malformed `instrument`, as a [`CodecError`].
pub fn decode_update_instrument_request(
    o: &Map<String, Value>,
) -> DResult<UpdateInstrumentRequest> {
    decode(UpdateInstrumentRequest::MESSAGE, o)
}

/// Decode a [`DeleteInstrumentRequest`] envelope — fully generic.
///
/// # Errors
/// A missing `session_token`/`instrument_id`, as a [`CodecError`].
pub fn decode_delete_instrument_request(
    o: &Map<String, Value>,
) -> DResult<DeleteInstrumentRequest> {
    decode(DeleteInstrumentRequest::MESSAGE, o)
}

/// Decode a [`BuildCurveRequest`] envelope — the required `reference_date` nests a
/// [`BrokenDate`]; `pillars` / `date_pillars` are optional repeated pillar arrays
/// (absent ⇒ empty).
///
/// # Errors
/// A missing `request_id`/`currency`/`session_token`, a missing/malformed
/// `reference_date` or pillar, as a [`CodecError`].
pub fn decode_build_curve_request(o: &Map<String, Value>) -> DResult<BuildCurveRequest> {
    decode(BuildCurveRequest::MESSAGE, o)
}

// --- auth nested-message adapters (encode) -----------------------------------

impl WireAdapter for UserDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "id" => Some(WireVal::Str(&self.id)),
            "email" => Some(WireVal::Str(&self.email)),
            "display_name" => Some(WireVal::Str(&self.display_name)),
            "role" => Some(WireVal::Enum(self.role)),
            // `repeated string desk_ids`: always present (an empty set ⇒ `[]`).
            "desk_ids" => Some(WireVal::RepeatedStr(&self.desk_ids)),
            "disabled" => Some(WireVal::Bool(self.disabled)),
            "all_desks" => Some(WireVal::Bool(self.all_desks)),
            _ => None,
        }
    }
}

impl WireAdapter for DeskDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "id" => Some(WireVal::Str(&self.id)),
            "name" => Some(WireVal::Str(&self.name)),
            _ => None,
        }
    }
}

impl WireAdapter for EntityDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "key" => Some(WireVal::U64(u64::from(self.key))),
            "name" => Some(WireVal::Str(&self.name)),
            "code" => Some(WireVal::Str(&self.code)),
            _ => None,
        }
    }
}

impl WireAdapter for BookDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "key" => Some(WireVal::U64(u64::from(self.key))),
            "name" => Some(WireVal::Str(&self.name)),
            "entity_key" => Some(WireVal::U64(u64::from(self.entity_key))),
            _ => None,
        }
    }
}

impl WireAdapter for CapabilityDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "action" => Some(WireVal::Str(&self.action)),
            "asset" => Some(WireVal::Str(&self.asset)),
            _ => None,
        }
    }
}

impl WireAdapter for ExternalId {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "scheme" => Some(WireVal::Str(&self.scheme)),
            "value" => Some(WireVal::Str(&self.value)),
            _ => None,
        }
    }
}

impl WireAdapter for DepositDef {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "index" => Some(WireVal::Str(&self.index)),
            "tenor" => Some(WireVal::Str(&self.tenor)),
            "day_count" => Some(WireVal::Str(&self.day_count)),
            "business_day_convention" => Some(WireVal::Str(&self.business_day_convention)),
            "calendars" => Some(WireVal::RepeatedStr(&self.calendars)),
            "spot_lag_days" => Some(WireVal::U64(u64::from(self.spot_lag_days))),
            _ => None,
        }
    }
}

impl WireAdapter for FraDef {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "float_index" => Some(WireVal::Str(&self.float_index)),
            "start_tenor" => Some(WireVal::Str(&self.start_tenor)),
            "end_tenor" => Some(WireVal::Str(&self.end_tenor)),
            "accrual_day_count" => Some(WireVal::Str(&self.accrual_day_count)),
            "business_day_convention" => Some(WireVal::Str(&self.business_day_convention)),
            "calendars" => Some(WireVal::RepeatedStr(&self.calendars)),
            "spot_lag_days" => Some(WireVal::U64(u64::from(self.spot_lag_days))),
            _ => None,
        }
    }
}

impl WireAdapter for StirFutureDef {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "contract_code" => Some(WireVal::Str(&self.contract_code)),
            "reference_start" => Some(WireVal::Str(&self.reference_start)),
            "reference_end" => Some(WireVal::Str(&self.reference_end)),
            "day_count" => Some(WireVal::Str(&self.day_count)),
            "calendars" => Some(WireVal::RepeatedStr(&self.calendars)),
            "convexity_vol" => Some(WireVal::F64(self.convexity_vol)),
            "contract_size" => Some(WireVal::F64(self.contract_size)),
            _ => None,
        }
    }
}

impl WireAdapter for VanillaIrsDef {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor" => Some(WireVal::Str(&self.tenor)),
            "fixed_frequency" => Some(WireVal::Str(&self.fixed_frequency)),
            "fixed_day_count" => Some(WireVal::Str(&self.fixed_day_count)),
            "float_index" => Some(WireVal::Str(&self.float_index)),
            "float_frequency" => Some(WireVal::Str(&self.float_frequency)),
            "float_day_count" => Some(WireVal::Str(&self.float_day_count)),
            "business_day_convention" => Some(WireVal::Str(&self.business_day_convention)),
            "calendars" => Some(WireVal::RepeatedStr(&self.calendars)),
            "roll_convention" => Some(WireVal::Str(&self.roll_convention)),
            "spot_lag_days" => Some(WireVal::U64(u64::from(self.spot_lag_days))),
            _ => None,
        }
    }
}

impl WireAdapter for OisDef {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "tenor" => Some(WireVal::Str(&self.tenor)),
            "index" => Some(WireVal::Str(&self.index)),
            "fixed_frequency" => Some(WireVal::Str(&self.fixed_frequency)),
            "fixed_day_count" => Some(WireVal::Str(&self.fixed_day_count)),
            "float_day_count" => Some(WireVal::Str(&self.float_day_count)),
            "business_day_convention" => Some(WireVal::Str(&self.business_day_convention)),
            "calendars" => Some(WireVal::RepeatedStr(&self.calendars)),
            "spot_lag_days" => Some(WireVal::U64(u64::from(self.spot_lag_days))),
            _ => None,
        }
    }
}

impl WireAdapter for BondDef {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "issuer" => Some(WireVal::Str(&self.issuer)),
            "coupon_rate" => Some(WireVal::F64(self.coupon_rate)),
            "coupon_type" => Some(WireVal::Str(&self.coupon_type)),
            "coupon_frequency" => Some(WireVal::Str(&self.coupon_frequency)),
            "day_count" => Some(WireVal::Str(&self.day_count)),
            // `optional BrokenDate` coupon-schedule dates: absent ⇒ `null` (BondDef ∈
            // null-absent-optional); `maturity_date` (singular message) renders `null`
            // via the generic rule.
            "issue_date" => self.issue_date.as_ref().map(|d| WireVal::Msg(d)),
            "dated_date" => self.dated_date.as_ref().map(|d| WireVal::Msg(d)),
            "first_coupon_date" => self.first_coupon_date.as_ref().map(|d| WireVal::Msg(d)),
            "maturity_date" => self.maturity_date.as_ref().map(|d| WireVal::Msg(d)),
            "redemption" => Some(WireVal::F64(self.redemption)),
            "calendars" => Some(WireVal::RepeatedStr(&self.calendars)),
            _ => None,
        }
    }
}

impl WireAdapter for InstrumentDefDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "instrument_id" => Some(WireVal::Str(&self.instrument_id)),
            "name" => Some(WireVal::Str(&self.name)),
            "description" => Some(WireVal::Str(&self.description)),
            "currency" => Some(WireVal::Str(&self.currency)),
            "external_ids" => Some(WireVal::RepeatedMsg(
                self.external_ids
                    .iter()
                    .map(|x| x as &dyn WireAdapter)
                    .collect(),
            )),
            // The `definition` family oneof: only the live arm's key is emitted (the
            // generic encoder omits the absent arms), byte-identical to the hand
            // `family_to_json` single-key insertion.
            "deposit" => match &self.definition {
                Some(InstrumentDefinition::Deposit(d)) => Some(WireVal::Msg(d)),
                _ => None,
            },
            "fra" => match &self.definition {
                Some(InstrumentDefinition::Fra(f)) => Some(WireVal::Msg(f)),
                _ => None,
            },
            "stir_future" => match &self.definition {
                Some(InstrumentDefinition::StirFuture(s)) => Some(WireVal::Msg(s)),
                _ => None,
            },
            "vanilla_irs" => match &self.definition {
                Some(InstrumentDefinition::VanillaIrs(v)) => Some(WireVal::Msg(v)),
                _ => None,
            },
            "ois" => match &self.definition {
                Some(InstrumentDefinition::Ois(o)) => Some(WireVal::Msg(o)),
                _ => None,
            },
            "bond" => match &self.definition {
                Some(InstrumentDefinition::Bond(b)) => Some(WireVal::Msg(b)),
                _ => None,
            },
            _ => None,
        }
    }
}

impl WireAdapter for CalibratedCurvePoint {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "instrument_id" => Some(WireVal::Str(&self.instrument_id)),
            "time_years" => Some(WireVal::F64(self.time_years)),
            "discount_factor" => Some(WireVal::F64(self.discount_factor)),
            "zero_rate" => Some(WireVal::F64(self.zero_rate)),
            "label" => Some(WireVal::Str(&self.label)),
            _ => None,
        }
    }
}

impl WireAdapter for CalibratedCurve {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "request_id" => Some(WireVal::Str(&self.request_id)),
            "currency" => Some(WireVal::Str(&self.currency)),
            "reference_date" => self.reference_date.as_ref().map(|d| WireVal::Msg(d)),
            "points" => Some(WireVal::RepeatedMsg(
                self.points.iter().map(|p| p as &dyn WireAdapter).collect(),
            )),
            _ => None,
        }
    }
}

// --- auth reply-envelope adapters (encode) -----------------------------------

impl WireAdapter for LoginResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "session_token" => Some(WireVal::Str(&self.session_token)),
            "user" => self.user.as_ref().map(|u| WireVal::Msg(u)),
            "expires_nanos" => Some(WireVal::I64(self.expires_nanos)),
            "capabilities" => Some(WireVal::RepeatedMsg(
                self.capabilities
                    .iter()
                    .map(|c| c as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for LogoutResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "ended" => Some(WireVal::Bool(self.ended)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for ListUsersResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "users" => Some(WireVal::RepeatedMsg(
                self.users.iter().map(|u| u as &dyn WireAdapter).collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for CreateUserResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "user" => self.user.as_ref().map(|u| WireVal::Msg(u)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdateUserResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "user" => self.user.as_ref().map(|u| WireVal::Msg(u)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DeleteUserResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "removed" => Some(WireVal::Bool(self.removed)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for ResetPasswordResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for GetUserCapabilitiesResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "grants" => Some(WireVal::RepeatedMsg(
                self.grants.iter().map(|c| c as &dyn WireAdapter).collect(),
            )),
            "denies" => Some(WireVal::RepeatedMsg(
                self.denies.iter().map(|c| c as &dyn WireAdapter).collect(),
            )),
            "effective" => Some(WireVal::RepeatedMsg(
                self.effective
                    .iter()
                    .map(|c| c as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for SetUserCapabilitiesResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "grants" => Some(WireVal::RepeatedMsg(
                self.grants.iter().map(|c| c as &dyn WireAdapter).collect(),
            )),
            "denies" => Some(WireVal::RepeatedMsg(
                self.denies.iter().map(|c| c as &dyn WireAdapter).collect(),
            )),
            "effective" => Some(WireVal::RepeatedMsg(
                self.effective
                    .iter()
                    .map(|c| c as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for GetRoleCapabilitiesResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "capabilities" => Some(WireVal::RepeatedMsg(
                self.capabilities
                    .iter()
                    .map(|c| c as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for SetRoleCapabilitiesResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "capabilities" => Some(WireVal::RepeatedMsg(
                self.capabilities
                    .iter()
                    .map(|c| c as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for ListDesksResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "desks" => Some(WireVal::RepeatedMsg(
                self.desks.iter().map(|d| d as &dyn WireAdapter).collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for CreateDeskResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "desk" => self.desk.as_ref().map(|d| WireVal::Msg(d)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdateDeskResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "desk" => self.desk.as_ref().map(|d| WireVal::Msg(d)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DeleteDeskResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "removed" => Some(WireVal::Bool(self.removed)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for ListEntitiesResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "entities" => Some(WireVal::RepeatedMsg(
                self.entities
                    .iter()
                    .map(|e| e as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for CreateEntityResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "entity" => self.entity.as_ref().map(|e| WireVal::Msg(e)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdateEntityResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "entity" => self.entity.as_ref().map(|e| WireVal::Msg(e)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DeleteEntityResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "removed" => Some(WireVal::Bool(self.removed)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for ListBooksResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "books" => Some(WireVal::RepeatedMsg(
                self.books.iter().map(|b| b as &dyn WireAdapter).collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for CreateBookResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "book" => self.book.as_ref().map(|b| WireVal::Msg(b)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdateBookResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "book" => self.book.as_ref().map(|b| WireVal::Msg(b)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DeleteBookResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "removed" => Some(WireVal::Bool(self.removed)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for AggregationParamsDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "staleness_tau_ms" => Some(WireVal::U64(self.staleness_tau_ms)),
            "max_quote_age_ms" => Some(WireVal::U64(self.max_quote_age_ms)),
            "divergence_gating" => Some(WireVal::Bool(self.divergence_gating)),
            "min_contributors" => Some(WireVal::U64(u64::from(self.min_contributors))),
            "depth_levels" => Some(WireVal::U64(u64::from(self.depth_levels))),
            _ => None,
        }
    }
}

impl WireAdapter for AggregatedBookDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "id" => Some(WireVal::Str(&self.id)),
            "name" => Some(WireVal::Str(&self.name)),
            "member_connection_ids" => Some(WireVal::RepeatedStr(&self.member_connection_ids)),
            "scope_mode" => Some(WireVal::Enum(self.scope_mode)),
            "instrument_ids" => Some(WireVal::RepeatedStr(&self.instrument_ids)),
            "params" => self.params.as_ref().map(|p| WireVal::Msg(p)),
            "enabled" => Some(WireVal::Bool(self.enabled)),
            _ => None,
        }
    }
}

impl WireAdapter for TieringConfigDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "unit" => Some(WireVal::Enum(self.unit)),
            "strategies" => Some(WireVal::RepeatedMsg(
                self.strategies
                    .iter()
                    .map(|s| s as &dyn WireAdapter)
                    .collect(),
            )),
            "guardrails" => self.guardrails.as_ref().map(|g| WireVal::Msg(g)),
            "stale_policy" => Some(WireVal::Enum(self.stale_policy)),
            _ => None,
        }
    }
}

impl WireAdapter for TieringStrategyDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "kind" => Some(WireVal::Enum(self.kind)),
            "half_spread" => Some(WireVal::F64(self.half_spread)),
            "kappa" => Some(WireVal::F64(self.kappa)),
            "s_max" => Some(WireVal::F64(self.s_max)),
            "smoothing_weight" => Some(WireVal::F64(self.smoothing_weight)),
            "expected_spread" => Some(WireVal::F64(self.expected_spread)),
            "max_divergence" => Some(WireVal::F64(self.max_divergence)),
            "core_spread" => Some(WireVal::F64(self.core_spread)),
            "max_output_spread" => Some(WireVal::F64(self.max_output_spread)),
            "spread_scale_factor" => Some(WireVal::F64(self.spread_scale_factor)),
            _ => None,
        }
    }
}

impl WireAdapter for TieringGuardrailsDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "h_min" => Some(WireVal::F64(self.h_min)),
            "h_max" => Some(WireVal::F64(self.h_max)),
            "s_max" => Some(WireVal::F64(self.s_max)),
            "spread_floor" => Some(WireVal::F64(self.spread_floor)),
            _ => None,
        }
    }
}

impl WireAdapter for ListAggregatedBooksResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "books" => Some(WireVal::RepeatedMsg(
                self.books.iter().map(|b| b as &dyn WireAdapter).collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for CreateAggregatedBookResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "book" => self.book.as_ref().map(|b| WireVal::Msg(b)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdateAggregatedBookResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "book" => self.book.as_ref().map(|b| WireVal::Msg(b)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DeleteAggregatedBookResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "removed" => Some(WireVal::Bool(self.removed)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for FeatureSpecDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "kind" => Some(WireVal::Enum(self.kind)),
            "unit" => Some(WireVal::Enum(self.unit)),
            "shift" => Some(WireVal::F64(self.shift)),
            // proto3 `optional` scalar: absent ⇒ omitted (the hand codec omits it too).
            "reference" => self.reference.map(WireVal::F64),
            "tiering" => self.tiering.as_ref().map(|t| WireVal::Msg(t)),
            "axe_side" => Some(WireVal::Enum(self.axe_side)),
            "magnitude" => Some(WireVal::F64(self.magnitude)),
            "kappa" => Some(WireVal::F64(self.kappa)),
            "s_max" => Some(WireVal::F64(self.s_max)),
            "skew" => Some(WireVal::F64(self.skew)),
            "triggered" => Some(WireVal::Bool(self.triggered)),
            _ => None,
        }
    }
}

impl WireAdapter for FeaturePipelineDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "features" => Some(WireVal::RepeatedMsg(
                self.features
                    .iter()
                    .map(|f| f as &dyn WireAdapter)
                    .collect(),
            )),
            "guardrails" => self.guardrails.as_ref().map(|g| WireVal::Msg(g)),
            _ => None,
        }
    }
}

impl WireAdapter for PricingGroupDesc {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "id" => Some(WireVal::Str(&self.id)),
            "name" => Some(WireVal::Str(&self.name)),
            "description" => Some(WireVal::Str(&self.description)),
            "member_connection_ids" => Some(WireVal::RepeatedStr(&self.member_connection_ids)),
            "member_user_ids" => Some(WireVal::RepeatedStr(&self.member_user_ids)),
            "member_desks" => Some(WireVal::RepeatedStr(&self.member_desks)),
            "esp_pipeline" => self.esp_pipeline.as_ref().map(|p| WireVal::Msg(p)),
            "rfq_pipeline" => self.rfq_pipeline.as_ref().map(|p| WireVal::Msg(p)),
            "share_pipeline" => Some(WireVal::Bool(self.share_pipeline)),
            "enabled" => Some(WireVal::Bool(self.enabled)),
            _ => None,
        }
    }
}

impl WireAdapter for ListPricingGroupsResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "groups" => Some(WireVal::RepeatedMsg(
                self.groups.iter().map(|g| g as &dyn WireAdapter).collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for CreatePricingGroupResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "group" => self.group.as_ref().map(|g| WireVal::Msg(g)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdatePricingGroupResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "group" => self.group.as_ref().map(|g| WireVal::Msg(g)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DeletePricingGroupResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "removed" => Some(WireVal::Bool(self.removed)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdatePricingGroupPipelineResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "group" => self.group.as_ref().map(|g| WireVal::Msg(g)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for ListInstrumentsResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "instruments" => Some(WireVal::RepeatedMsg(
                self.instruments
                    .iter()
                    .map(|i| i as &dyn WireAdapter)
                    .collect(),
            )),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for GetInstrumentResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "instrument" => self.instrument.as_ref().map(|i| WireVal::Msg(i)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for CreateInstrumentResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "instrument" => self.instrument.as_ref().map(|i| WireVal::Msg(i)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for UpdateInstrumentResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "instrument" => self.instrument.as_ref().map(|i| WireVal::Msg(i)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

impl WireAdapter for DeleteInstrumentResponse {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "removed" => Some(WireVal::Bool(self.removed)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            _ => None,
        }
    }
}

// --- auth reply encode entry points ------------------------------------------

/// Encode a [`LoginResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_login_response(r: &LoginResponse) -> Value {
    encode("LoginResponse", r)
}

/// Encode a [`LogoutResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_logout_response(r: &LogoutResponse) -> Value {
    encode("LogoutResponse", r)
}

/// Encode a [`ListUsersResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_users_response(r: &ListUsersResponse) -> Value {
    encode("ListUsersResponse", r)
}

/// Encode a [`CreateUserResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_create_user_response(r: &CreateUserResponse) -> Value {
    encode("CreateUserResponse", r)
}

/// Encode an [`UpdateUserResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_user_response(r: &UpdateUserResponse) -> Value {
    encode("UpdateUserResponse", r)
}

/// Encode a [`DeleteUserResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_delete_user_response(r: &DeleteUserResponse) -> Value {
    encode("DeleteUserResponse", r)
}

/// Encode a [`ResetPasswordResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_reset_password_response(r: &ResetPasswordResponse) -> Value {
    encode("ResetPasswordResponse", r)
}

/// Encode a [`GetUserCapabilitiesResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_get_user_capabilities_response(r: &GetUserCapabilitiesResponse) -> Value {
    encode("GetUserCapabilitiesResponse", r)
}

/// Encode a [`SetUserCapabilitiesResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_set_user_capabilities_response(r: &SetUserCapabilitiesResponse) -> Value {
    encode("SetUserCapabilitiesResponse", r)
}

/// Encode a [`GetRoleCapabilitiesResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_get_role_capabilities_response(r: &GetRoleCapabilitiesResponse) -> Value {
    encode("GetRoleCapabilitiesResponse", r)
}

/// Encode a [`SetRoleCapabilitiesResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_set_role_capabilities_response(r: &SetRoleCapabilitiesResponse) -> Value {
    encode("SetRoleCapabilitiesResponse", r)
}

/// Encode a [`ListDesksResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_desks_response(r: &ListDesksResponse) -> Value {
    encode("ListDesksResponse", r)
}

/// Encode a [`CreateDeskResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_create_desk_response(r: &CreateDeskResponse) -> Value {
    encode("CreateDeskResponse", r)
}

/// Encode an [`UpdateDeskResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_desk_response(r: &UpdateDeskResponse) -> Value {
    encode("UpdateDeskResponse", r)
}

/// Encode a [`DeleteDeskResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_delete_desk_response(r: &DeleteDeskResponse) -> Value {
    encode("DeleteDeskResponse", r)
}

/// Encode a [`ListEntitiesResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_entities_response(r: &ListEntitiesResponse) -> Value {
    encode("ListEntitiesResponse", r)
}

/// Encode a [`CreateEntityResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_create_entity_response(r: &CreateEntityResponse) -> Value {
    encode("CreateEntityResponse", r)
}

/// Encode an [`UpdateEntityResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_entity_response(r: &UpdateEntityResponse) -> Value {
    encode("UpdateEntityResponse", r)
}

/// Encode a [`DeleteEntityResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_delete_entity_response(r: &DeleteEntityResponse) -> Value {
    encode("DeleteEntityResponse", r)
}

/// Encode a [`ListBooksResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_books_response(r: &ListBooksResponse) -> Value {
    encode("ListBooksResponse", r)
}

/// Encode a [`CreateBookResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_create_book_response(r: &CreateBookResponse) -> Value {
    encode("CreateBookResponse", r)
}

/// Encode an [`UpdateBookResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_book_response(r: &UpdateBookResponse) -> Value {
    encode("UpdateBookResponse", r)
}

/// Encode a [`DeleteBookResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_delete_book_response(r: &DeleteBookResponse) -> Value {
    encode("DeleteBookResponse", r)
}

/// Encode a [`ListAggregatedBooksResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_aggregated_books_response(r: &ListAggregatedBooksResponse) -> Value {
    encode("ListAggregatedBooksResponse", r)
}

/// Encode a [`CreateAggregatedBookResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_create_aggregated_book_response(r: &CreateAggregatedBookResponse) -> Value {
    encode("CreateAggregatedBookResponse", r)
}

/// Encode an [`UpdateAggregatedBookResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_aggregated_book_response(r: &UpdateAggregatedBookResponse) -> Value {
    encode("UpdateAggregatedBookResponse", r)
}

/// Encode a [`DeleteAggregatedBookResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_delete_aggregated_book_response(r: &DeleteAggregatedBookResponse) -> Value {
    encode("DeleteAggregatedBookResponse", r)
}

/// Encode a [`ListPricingGroupsResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_pricing_groups_response(r: &ListPricingGroupsResponse) -> Value {
    encode("ListPricingGroupsResponse", r)
}

/// Encode a [`CreatePricingGroupResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_create_pricing_group_response(r: &CreatePricingGroupResponse) -> Value {
    encode("CreatePricingGroupResponse", r)
}

/// Encode an [`UpdatePricingGroupResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_pricing_group_response(r: &UpdatePricingGroupResponse) -> Value {
    encode("UpdatePricingGroupResponse", r)
}

/// Encode a [`DeletePricingGroupResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_delete_pricing_group_response(r: &DeletePricingGroupResponse) -> Value {
    encode("DeletePricingGroupResponse", r)
}

/// Encode an [`UpdatePricingGroupPipelineResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_pricing_group_pipeline_response(
    r: &UpdatePricingGroupPipelineResponse,
) -> Value {
    encode("UpdatePricingGroupPipelineResponse", r)
}

/// Encode a [`ListInstrumentsResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_list_instruments_response(r: &ListInstrumentsResponse) -> Value {
    encode("ListInstrumentsResponse", r)
}

/// Encode a [`GetInstrumentResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_get_instrument_response(r: &GetInstrumentResponse) -> Value {
    encode("GetInstrumentResponse", r)
}

/// Encode a [`CreateInstrumentResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_create_instrument_response(r: &CreateInstrumentResponse) -> Value {
    encode("CreateInstrumentResponse", r)
}

/// Encode an [`UpdateInstrumentResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_update_instrument_response(r: &UpdateInstrumentResponse) -> Value {
    encode("UpdateInstrumentResponse", r)
}

/// Encode a [`DeleteInstrumentResponse`] to its WS JSON — descriptor-driven.
#[must_use]
pub fn encode_delete_instrument_response(r: &DeleteInstrumentResponse) -> Value {
    encode("DeleteInstrumentResponse", r)
}

/// Encode a [`CalibratedCurve`] to its WS JSON — descriptor-driven (mirrors the hand
/// `calibrated_curve_to_json`; `reference_date` renders `null`-when-absent).
#[must_use]
pub fn encode_calibrated_curve(c: &CalibratedCurve) -> Value {
    encode("CalibratedCurve", c)
}

// --- aggregated-book composite publish frames (D3) --------------------------

impl WireAdapter for SubscriptionId {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "value" => Some(WireVal::U64(self.value)),
            _ => None,
        }
    }
}

impl WireAdapter for LpContribution {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "lp_name" => Some(WireVal::Str(&self.lp_name)),
            "bid" => Some(WireVal::F64(self.bid)),
            "offer" => Some(WireVal::F64(self.offer)),
            "stale" => Some(WireVal::Bool(self.stale)),
            _ => None,
        }
    }
}

impl WireAdapter for AggregatedInstrument {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "instrument_id" => Some(WireVal::Str(&self.instrument_id)),
            "display_name" => Some(WireVal::Str(&self.display_name)),
            "isin" => Some(WireVal::Str(&self.isin)),
            "cusip" => Some(WireVal::Str(&self.cusip)),
            "best_bid" => Some(WireVal::F64(self.best_bid)),
            "best_offer" => Some(WireVal::F64(self.best_offer)),
            "bid_size" => Some(WireVal::F64(self.bid_size)),
            "offer_size" => Some(WireVal::F64(self.offer_size)),
            "confidence" => Some(WireVal::F64(self.confidence)),
            "contributions" => Some(WireVal::RepeatedMsg(
                self.contributions
                    .iter()
                    .map(|c| c as &dyn WireAdapter)
                    .collect(),
            )),
            _ => None,
        }
    }
}

impl WireAdapter for AggregatedBookSnapshot {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "book_id" => Some(WireVal::Str(&self.book_id)),
            "instruments" => Some(WireVal::RepeatedMsg(
                self.instruments
                    .iter()
                    .map(|i| i as &dyn WireAdapter)
                    .collect(),
            )),
            _ => None,
        }
    }
}

impl WireAdapter for AggregatedBookStreamSnapshot {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "subscription" => self.subscription.as_ref().map(|s| WireVal::Msg(s)),
            "sequence" => Some(WireVal::U64(self.sequence)),
            "book" => self.book.as_ref().map(|b| WireVal::Msg(b)),
            "correlation_id" => self.correlation_id.map(WireVal::U64),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            _ => None,
        }
    }
}

impl WireAdapter for AggregatedBookStreamUpdate {
    fn get(&self, proto_name: &str) -> Option<WireVal<'_>> {
        match proto_name {
            "subscription" => self.subscription.as_ref().map(|s| WireVal::Msg(s)),
            "sequence" => Some(WireVal::U64(self.sequence)),
            "book" => self.book.as_ref().map(|b| WireVal::Msg(b)),
            "epoch_nanos" => Some(WireVal::I64(self.epoch_nanos)),
            _ => None,
        }
    }
}

/// Encode an [`AggregatedBookStreamSnapshot`] (the baseline composite frame) to its
/// WS JSON — descriptor-driven.
#[must_use]
pub fn encode_aggregated_book_stream_snapshot(s: &AggregatedBookStreamSnapshot) -> Value {
    encode("AggregatedBookStreamSnapshot", s)
}

/// Encode an [`AggregatedBookStreamUpdate`] (a composite delta frame) to its WS JSON
/// — descriptor-driven.
#[must_use]
pub fn encode_aggregated_book_stream_update(u: &AggregatedBookStreamUpdate) -> Value {
    encode("AggregatedBookStreamUpdate", u)
}
