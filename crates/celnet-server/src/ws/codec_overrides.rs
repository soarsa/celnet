//! Curated WS-codec override table (arch item G — `ws-codec-from-proto`).
//!
//! The descriptor-derived field tables in [`celnet_proto::wire_contract`] project
//! every proto field onto `json_key == proto_name` (snake_case). The hand-curated
//! WS *client* contract intentionally diverges from that naive projection in a
//! handful of FX-legacy ways the descriptor alone cannot encode
//! (`docs/INTERFACES.md` → "WS codec from proto descriptor (item G)"). This module
//! is the single, data-first home for those divergences: a small
//! `(message, field) → rule` table plus a few typed synthesis functions for the
//! cases that *compute* a JSON key from an accessor rather than merely renaming a
//! field. The [generated encoder](super::generated_codec) reads this table to stay
//! **byte-identical** to the hand codec ([`super::codec`]) while remaining a fully
//! descriptor-driven, independent code path.
//!
//! The four FX-legacy divergences captured here (see the module-level activation
//! plan in `docs/INTERFACES.md`):
//! - **(a)** `Underlying` → the legacy `{base, quote}` `pair` body (its FX arm),
//!   NOT the `ref` oneof / `settlement_ccy` — a whole-message projection
//!   ([`underlying_fx_projection`]).
//! - **(b)** `MarketContext` → the FX `r_dom`/`r_for` accessors, NOT the
//!   generalized `{discount_rate, carry}` — `discount_rate` is renamed to `r_dom`
//!   and `carry` is suppressed in favour of the synthesized `r_for`
//!   ([`market_context_synth`]).
//! - **(c)** `Greeks` → the flat `rho_dom`/`rho_for` emitted beside the
//!   `rate_sensitivities` oneof. Proto fields 7/8 are `reserved`, so they are NOT
//!   in the field table; they are *synthesized* from the carry-tagged arm
//!   ([`greeks_synth`]).
//! - **(d)** `Tenor.broken_date` → camelCase `brokenDate` on the wire (snake_case
//!   in the table) — a plain key rename.

use celnet_proto::{Greeks, MarketContext, Underlying};
use serde_json::{Value, json};

/// How the generated encoder should treat one descriptor field's JSON key.
///
/// The default for every field is [`FieldRule::Keep`] (emit under the field
/// table's `json_key`, i.e. the proto snake_case name). Only the curated
/// exceptions in [`field_rule`] deviate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldRule {
    /// Emit under the field table's `json_key` verbatim (the common case).
    Keep,
    /// Emit under this JSON key instead of the field table's `json_key`.
    Rename(&'static str),
    /// Do not emit this field at all — its value reaches the wire through a
    /// synthesized key instead (e.g. `MarketContext.carry` → the `r_for` synth).
    Suppress,
}

/// The curated JSON-key rule for `(message, proto_name)`.
///
/// Keyed on the message's **simple** type name (matching
/// [`celnet_proto::wire_contract::MESSAGE_FIELDS`]) and the proto (snake_case)
/// field name. Everything not listed is [`FieldRule::Keep`], so this table stays
/// tiny and only records genuine divergences from the descriptor projection.
pub(crate) fn field_rule(message: &str, proto_name: &str) -> FieldRule {
    match (message, proto_name) {
        // (b) MarketContext FX-legacy: `discount_rate` IS `r_dom`; the generalized
        //     `carry` message is replaced by the synthesized flat `r_for`.
        ("MarketContext", "discount_rate") => FieldRule::Rename("r_dom"),
        ("MarketContext", "carry") => FieldRule::Suppress,
        // (d) camelCase `brokenDate` on the wire (snake_case `broken_date` in the
        //     descriptor field table).
        ("Tenor", "broken_date") => FieldRule::Rename("brokenDate"),
        _ => FieldRule::Keep,
    }
}

/// Whether a real-oneof `group` on `message` is **required** — i.e. the hand
/// decoder errors if the JSON carries no live arm (rather than leaving the oneof
/// `None`). The decode override the descriptor cannot express: proto3 says nothing
/// about whether a oneof must be set. Keyed on the message's **simple** type name
/// and the oneof group name; the default is `false` (an absent oneof decodes to
/// `None`), so only the genuinely-mandatory oneofs are listed.
///
/// `StrikeOrDelta.spec` is the archetype: `strike_or_delta_from_json` errors with
/// "strike must carry exactly one of `strike` or `delta`" when neither arm is
/// present, so the generated decoder must reject the same body identically.
pub(crate) fn oneof_required(message: &str, group: &str) -> bool {
    matches!((message, group), ("StrikeOrDelta", "spec"))
}

// Message-level synthesized keys — the cases where a JSON key is *derived* from an
// accessor rather than being a straight field projection (quirks b and c). There is
// no free-function dispatch on `message` here: synthesis needs the concrete typed
// value, so each generated adapter's `WireAdapter::synthesized` delegates directly
// to the typed helper below, keeping the arithmetic in one auditable place.

/// (c) `Greeks` synthesizes the flat `rho_dom`/`rho_for` scalars (retired proto
/// fields 7/8, `reserved`) beside the `rate_sensitivities` oneof, from the
/// carry-tagged arm's lossless FX-shaped projection
/// ([`Greeks::rho_dom`]/[`Greeks::rho_for`]). Byte-identical to the hand codec's
/// `greeks_to_json`, which emits the very same two accessors.
pub(crate) fn greeks_synth(g: &Greeks) -> Vec<(&'static str, Value)> {
    vec![
        ("rho_dom", json!(g.rho_dom())),
        ("rho_for", json!(g.rho_for())),
    ]
}

/// (b) `MarketContext` synthesizes the flat foreign rate `r_for` from the FX carry
/// arm ([`MarketContext::r_for`]); the generalized `carry` message is suppressed
/// (see [`field_rule`]) and `discount_rate` is renamed to `r_dom`. Byte-identical
/// to the hand codec's `market_context_to_json`.
pub(crate) fn market_context_synth(m: &MarketContext) -> Vec<(&'static str, Value)> {
    vec![("r_for", json!(m.r_for()))]
}

/// (a) `Underlying` → the legacy `{base, quote}` `pair` body of its FX arm, or
/// JSON `null` for a non-FX underlying (which never reaches this FX-only WS
/// surface — the hand codec's `underlying_to_json` behaves identically). The pair
/// body is produced by the **generated** `CcyPair` encoder so it stays
/// descriptor-driven rather than re-hand-rolled here.
pub(crate) fn underlying_fx_projection(u: &Underlying) -> Value {
    u.as_fx()
        .map_or(Value::Null, super::generated_codec::encode_ccy_pair)
}
