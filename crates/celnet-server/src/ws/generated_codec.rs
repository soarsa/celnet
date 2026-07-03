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
    Accumulator, AmericanOption, ArbReport, AsianOption, BasketLeg, BasketOption, BondInstrument,
    BrokenDate, CcyPair, Cliquet, CommodityRef, Conventions, CryptoPair, CurveSet, Digital,
    DoubleBarrier, EquityRef, FixingSchedule, ForwardStart, FraInstrument, FxForward, FxSwap,
    Greeks, Instrument, Leg, ListedFutureOption, Lookback, MarketContext, MetalPair, Ndf,
    OisInstrument, OisPillar, PerpetualOption, PillarTenor, Pivot, PriceRequest, PriceResponse,
    PriceXvaRequest, PriceXvaResponse, Quantity, Quanto, RateSensitivities, RatesInstrument,
    RatesPriceRequest, RatesPriceResponse, RatesPricingResult, SingleBarrier, Solve, Strategy,
    StrikeOrDelta, Symbol, Tarf, Tenor, Touch, Underlying, Vanilla, VanillaIrsInstrument,
    VarianceSwap, VolatilitySwap, WindowBarrier, XvaResult as WireXvaResult, XvaSurvivalCurve,
    XvaTrade, instrument, pillar_tenor, rate_sensitivities, rates_instrument, strike_or_delta,
};
use serde_json::{Map, Value, json};

use super::codec::CodecError;
use super::codec_overrides::{self, FieldRule};

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
                        let absent_singular_message =
                            field.label == WireLabel::Singular && is_message_type(field.proto_type);
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
