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
    BrokenDate, CcyPair, CommodityRef, Conventions, CryptoPair, EquityRef, Greeks, Leg,
    MarketContext, MetalPair, Quantity, RateSensitivities, Solve, Strategy, StrikeOrDelta, Symbol,
    Tenor, Underlying, Vanilla, rate_sensitivities, strike_or_delta,
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
    /// A UTF-8 `string` scalar.
    Str(&'a str),
    /// A present nested message — encoded by recursing on its own field table.
    Msg(&'a dyn WireAdapter),
    /// A `repeated` message field.
    RepeatedMsg(Vec<&'a dyn WireAdapter>),
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
        WireVal::Str(s) => json!(s),
        WireVal::Msg(inner) => encode(simple_type_name(field.proto_type), inner),
        WireVal::RepeatedMsg(items) => Value::Array(
            items
                .into_iter()
                .map(|inner| encode(simple_type_name(field.proto_type), inner))
                .collect(),
        ),
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
                    // `.map(..)` → `null`. An unset oneof arm or an absent proto3
                    // `optional` is simply omitted.
                    if field.oneof_group.is_none()
                        && field.label == WireLabel::Singular
                        && is_message_type(field.proto_type)
                    {
                        map.insert(key.to_owned(), Value::Null);
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
