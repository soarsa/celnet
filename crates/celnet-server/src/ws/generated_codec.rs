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
    BrokenDate, CcyPair, Greeks, Leg, MarketContext, RateSensitivities, Strategy, StrikeOrDelta,
    Tenor, Underlying, rate_sensitivities, strike_or_delta,
};
use serde_json::{Map, Value, json};

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
