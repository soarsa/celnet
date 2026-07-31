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

use celnet_proto::{Greeks, MarketContext, Underlying, VanillaInputs};
use serde_json::{Value, json};

/// The Price-family response messages whose absent proto3-`optional` scalar fields
/// are emitted as JSON `null` (present-with-null), NOT omitted — the hand codec's
/// one-shot pricing encoders (`price_response_to_json` / `rates_price_response_to_json`
/// / `price_xva_response_to_json`) build these via `json!({ .. })`, where an
/// `Option::None` scalar serializes to `null` under its key. This is the mirror image
/// of the *request*-side leaf messages (e.g. `Tenor.broken_date`) whose absent
/// proto3-`optional` fields are **omitted** by the hand `.map(..)` / conditional-insert
/// encoders; the divergence is genuinely per-message, so it is recorded here rather
/// than baked into the generic encoder. Keyed on the simple message type name.
const NULL_ABSENT_OPTIONAL_MESSAGES: &[&str] = &[
    "PriceResponse",
    "RatesPriceResponse",
    "PriceXvaResponse",
    // The fixed-income taker RFQ reply (`QuoteService.RequestRatesQuote`): its
    // presence-tracked `correlation_id` reaches the wire as `null` when absent,
    // consistent with the other one-shot reply messages above.
    "RatesQuote",
    // The FixAdminService reply envelopes: their `json!({ .. })` hand encoders emit
    // the `Option<u64>` `correlation_id` as `null` when absent (present-with-null),
    // so the generated encoder must too. (The `connection` singular-message field is
    // already rendered `null`-when-absent by the generic encoder's singular-message
    // rule and needs no entry here.)
    "ListFixConnectionsResponse",
    "CreateFixConnectionResponse",
    "UpdateFixConnectionResponse",
    "DeleteFixConnectionResponse",
    "SetFixConnectionEnabledResponse",
    "ListFixMessagesResponse",
    // The QuoteService reply messages: their `json!({ .. })` hand encoders emit every
    // absent presence-tracked field (`correlation_id` / `surface_version` /
    // `price_std_error` / the `attribution` optional message) as JSON `null`.
    "Quote",
    "DealerQuote",
    "MultiDealerQuote",
    "Execution",
    // The RiskService reply messages: their `json!({ .. })` hand encoders emit every
    // absent presence-tracked field as JSON `null` — the optional `correlation_id`
    // echoes (u64), the `RiskPosition.attribution` optional message, and the
    // `NonAdditiveRisk` VaR/ES/curvature optional scalars (absent ⇒ not-evaluated,
    // never a spurious zero).
    "ListPositionsResponse",
    "AggregateRiskResponse",
    "AggregateRatesRiskResponse",
    "DrillRiskResponse",
    "LimitStatusResponse",
    "RiskPosition",
    "NonAdditiveRisk",
    // The RfqDeskService `DeskRequest` / `Deal` blotter messages: their `json!({ .. })`
    // hand encoders emit the optional `DeskRequest.quote` message, the
    // `Deal.position_id` (u64) and both messages' `correlation_id` (string) as JSON
    // `null` when absent (present-with-null through the RFQ lifecycle).
    "DeskRequest",
    "Deal",
    // The NotificationService push frame: its `json!({ .. })` hand encoder emits the
    // optional `request_id` / `detail` strings and the presence-tracked
    // manual-intervention `reason` enum as JSON `null` when absent.
    "Notification",
    // The AuthService reply envelopes (wave 4 — login/session, user/desk/entity/book
    // CRUD, capabilities, roles, instrument registry): every one is built by a
    // `json!({ .. })` hand encoder that emits the `Option<u64>` `correlation_id` as
    // JSON `null` when absent (present-with-null). Their singular-message payload
    // fields (`user` / `desk` / `entity` / `book` / `instrument`) already render
    // `null`-when-absent via the generic singular-message rule and need no entry.
    "LoginResponse",
    "LogoutResponse",
    "ListUsersResponse",
    "CreateUserResponse",
    "UpdateUserResponse",
    "DeleteUserResponse",
    "ResetPasswordResponse",
    "GetUserCapabilitiesResponse",
    "SetUserCapabilitiesResponse",
    "GetRoleCapabilitiesResponse",
    "SetRoleCapabilitiesResponse",
    "ListDesksResponse",
    "CreateDeskResponse",
    "DeleteDeskResponse",
    "ListEntitiesResponse",
    "CreateEntityResponse",
    "UpdateEntityResponse",
    "DeleteEntityResponse",
    "ListBooksResponse",
    "CreateBookResponse",
    "UpdateBookResponse",
    "DeleteBookResponse",
    // The AuthService aggregated-book reply envelopes (ADR-0022): each `json!({ .. })`
    // hand encoder emits the `Option<u64>` `correlation_id` as JSON `null` when absent;
    // the `book` singular-message payload already renders `null`-when-absent via the
    // generic singular-message rule and needs no entry.
    "ListAggregatedBooksResponse",
    "CreateAggregatedBookResponse",
    "UpdateAggregatedBookResponse",
    "DeleteAggregatedBookResponse",
    // The AuthService pricing-group reply envelopes (FI client-tiering): each
    // `json!({ .. })` hand encoder emits the `Option<u64>` `correlation_id` as JSON `null`
    // when absent; the `group` singular-message payload already renders `null`-when-absent
    // via the generic singular-message rule and needs no entry.
    "ListPricingGroupsResponse",
    "CreatePricingGroupResponse",
    "UpdatePricingGroupResponse",
    "DeletePricingGroupResponse",
    "UpdatePricingGroupPipelineResponse",
    // The AuthService risk-routing reply envelopes (FI risk routing, phase 4): each
    // `json!({ .. })` hand encoder emits the `Option<u64>` `correlation_id` as JSON `null`
    // when absent; the `book` / `graph` singular-message payloads already render
    // `null`-when-absent via the generic singular-message rule and need no entry.
    "ListRiskBooksResponse",
    "CreateRiskBookResponse",
    "UpdateRiskBookResponse",
    "DeleteRiskBookResponse",
    "GetRiskRoutingGraphResponse",
    "UpdateRiskRoutingGraphResponse",
    // The AuthService auto-hedging reply envelopes (Phase B): each `json!({ .. })` hand
    // encoder emits the `Option<u64>` `correlation_id` as JSON `null` when absent; the
    // `graph` / `config` singular-message payloads already render `null`-when-absent via
    // the generic singular-message rule and need no entry. The nested `ExitActionDesc` /
    // `HedgeProvenance` records are NOT listed: their presence-tracked scalars (`skew_bp`
    // / `lp_won`) OMIT-when-absent, exactly like `RiskTransferProvenance`. (The threshold
    // list envelopes carry only the repeated `thresholds` + the null-absent `correlation_id`.)
    "GetHedgePolicyGraphResponse",
    "UpdateHedgePolicyGraphResponse",
    "ListHedgeThresholdsResponse",
    "UpdateHedgeThresholdResponse",
    "ListHedgeProvenanceResponse",
    "GetHedgeConfigResponse",
    "SetHedgeConfigResponse",
    // The AuthService risk-transfer reply envelopes (RiskTransfer RPCs): each
    // `json!({ .. })` hand encoder emits the `Option<u64>` `correlation_id` as JSON
    // `null` when absent; the `transfer` singular-message payload already renders
    // `null`-when-absent via the generic singular-message rule and needs no entry.
    // The nested `RiskTransfer` / `RiskTransferProvenance` records are NOT listed:
    // their presence-tracked scalars (`partial_notional` / `agreed_price` / `approver`
    // / `decided_at` / `transfer_price`) and `optional provenance` OMIT-when-absent,
    // exactly like `RiskBookDesc`. (`RiskTransferInbox` carries no optional field.)
    "InitiateRiskTransferResponse",
    "AcceptRiskTransferResponse",
    "RejectRiskTransferResponse",
    "CancelRiskTransferResponse",
    "ListRiskTransfersResponse",
    // The phase-5 per-book risk aggregation reply (`ListRiskBookRisk`): the reply
    // envelope's `json!({ .. })` hand encoder emits the `Option<u64>` `correlation_id`
    // as JSON `null` when absent, and each nested `RiskBookRiskDesc` row emits its
    // optional `dv01`/`pnl` (not-yet-evaluated rates DV01 / mark PnL — §5.3/§5.4) as
    // JSON `null` when absent, never a fabricated zero. Both message names are listed so
    // the generic encoder applies the null-absent policy at the envelope AND the nested
    // row. `LimitUtilizationDesc` carries no optional field, so it is not listed.
    "ListRiskBookRiskResponse",
    "RiskBookRiskDesc",
    // The client-flow analytics roster (Analytics phase 2): the `ListClientFlowMetricsResponse`
    // envelope emits its `Option<u64>` `correlation_id` as JSON `null` when absent, and each
    // nested `ClientFlowMetricsDesc` row emits every optional `$/mm` / spread / ratio metric
    // (`dpm_gross`, `dpm_net`, `captured_vs_offered`, `mean_cover_distance`, `breakeven_spread`,
    // `quote_to_trade_ratio`, `hit_rate`) as JSON `null` when its denominator is zero — the
    // divide-by-zero guard, never a fabricated zero. Both names listed (envelope + nested row).
    "ListClientFlowMetricsResponse",
    "ClientFlowMetricsDesc",
    // The street-side / LP liquidity roster (§2.4): the `ListLpFlowMetricsResponse`
    // envelope emits its `Option<u64>` `correlation_id` as JSON `null` when absent, and
    // each nested `LpFlowMetricsDesc` row emits its optional `win_rate` / `mean_cover`
    // ratios as JSON `null` when their denominator is zero — the divide-by-zero guard.
    "ListLpFlowMetricsResponse",
    "LpFlowMetricsDesc",
    // The Latency/Ops analytics snapshot (Analytics pillar B): the
    // `ListLatencyMetricsResponse` envelope emits its `Option<u64>` `correlation_id`
    // as JSON `null` when absent (and its absent singular `health` message renders
    // as `null` by the descriptor default) — quirk-symmetric with the client-flow
    // envelope so the hand + generated encoders stay byte-identical.
    "ListLatencyMetricsResponse",
    "ListInstrumentsResponse",
    "GetInstrumentResponse",
    "CreateInstrumentResponse",
    "UpdateInstrumentResponse",
    "DeleteInstrumentResponse",
    // The `BondDef` instrument family block: its `optional BrokenDate` coupon-schedule
    // dates (`issue_date` / `dated_date` / `first_coupon_date`) are emitted as JSON
    // `null` when absent by `family_to_json`'s `.as_ref().map(..)` (the `maturity_date`
    // singular message already renders `null`-when-absent via the generic rule).
    "BondDef",
    // The aggregated-book composite BASELINE stream frame (D3): its `json!({ .. })`
    // hand encoder emits the `Option<u64>` `correlation_id` as JSON `null` when
    // absent; the `subscription` / `book` singular-message payloads already render
    // `null`-when-absent via the generic singular-message rule and need no entry.
    // (`AggregatedBookStreamUpdate` carries no optional field, so it is not listed.)
    "AggregatedBookStreamSnapshot",
    // The live per-book risk stream BASELINE frame: its `json!({ .. })` hand encoder emits
    // the `Option<u64>` `correlation_id` as JSON `null` when absent; the `subscription`
    // singular-message payload already renders `null`-when-absent via the generic rule, and
    // each nested `RiskBookRiskDesc` row applies its own null-absent policy (listed above).
    // (`RiskBookRiskUpdate` carries no optional field, so it is not listed.)
    "RiskBookRiskSnapshot",
];

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
        // (b) VanillaInputs carries the SAME FX-legacy carry seam as MarketContext:
        //     the risk cube's per-position pricing inputs expose the flat FX
        //     `r_dom`/`r_for` accessors beside the generalized `{discount_rate, carry}`.
        //     `discount_rate` IS `r_dom`; `carry` is suppressed in favour of the
        //     synthesized flat `r_for` ([`vanilla_inputs_synth`]).
        ("VanillaInputs", "discount_rate") => FieldRule::Rename("r_dom"),
        ("VanillaInputs", "carry") => FieldRule::Suppress,
        // The risk `OrgKey` carries the position's (FX-only) underlying under the
        // legacy `ccy_pair` key, encoded via the same FX `{base, quote}` projection
        // as `Underlying` (quirk a). The generated `Underlying` message projection
        // resolves the value; this rule only renames the field key.
        ("OrgKey", "underlying") => FieldRule::Rename("ccy_pair"),
        // (d) camelCase `brokenDate` on the wire (snake_case `broken_date` in the
        //     descriptor field table).
        ("Tenor", "broken_date") => FieldRule::Rename("brokenDate"),
        // A basket leg carries its (FX-only) underlying under the legacy `pair` key,
        // decoded via the FX `{base, quote}` projection — the same FX-legacy pair
        // surface as `Underlying` (quirk a), one level down. The generated
        // `BasketLeg` builder resolves the `pair` value into the FX arm.
        ("BasketLeg", "underlying") => FieldRule::Rename("pair"),
        // (quote family) the who's-trading `AttributionRecord` rides under camelCase
        // wire keys the GUI/Excel decoders read (`attribution_to_json`).
        ("AttributionRecord", "quoted_by") => FieldRule::Rename("quotedBy"),
        ("AttributionRecord", "held_by") => FieldRule::Rename("heldBy"),
        ("AttributionRecord", "lp_count") => FieldRule::Rename("lpCount"),
        // (quote family) the `Owner` auto-pricer oneof arm rides under camelCase.
        ("Owner", "auto_pricer") => FieldRule::Rename("autoPricer"),
        // (quote family) the booked `Execution` carries the traded `instrument` in the
        // proto, but the WS `execution_to_json` never serializes it — suppress it so
        // the generated encoder omits the key exactly as the hand codec does. (Encode
        // only: `Execution` is a response, never decoded through the generic walk.)
        ("Execution", "instrument") => FieldRule::Suppress,
        _ => FieldRule::Keep,
    }
}

/// Whether `message`'s absent SINGULAR message fields are OMITTED (rather than
/// rendered as JSON `null`) by the hand encoder — the messages whose hand encoder
/// builds a `serde_json::Map` inserting only present fields (`attribution_to_json` /
/// `book_id_to_json`), as opposed to the `json!({ .. })` encoders that emit every
/// absent field as `null`. Keyed on the simple message type name; the default is
/// `false` (the `json!`-style null-everything policy the Price/quote replies use).
pub(crate) fn omit_absent_message(message: &str) -> bool {
    matches!(message, "AttributionRecord" | "BookId")
}

/// Whether `message`'s absent proto3-`optional` scalar fields are emitted as JSON
/// `null` (rather than omitted) by the hand encoder — the Price-family one-shot
/// response messages (see [`NULL_ABSENT_OPTIONAL_MESSAGES`]). The generated encoder
/// consults this so `correlation_id`/`surface_version`/`price_std_error` reach the
/// wire as `null` when `None`, byte-identical to the hand `json!({ .. })` encoders,
/// while request-side leaf messages keep the omit-when-absent default.
pub(crate) fn null_absent_optional(message: &str) -> bool {
    NULL_ABSENT_OPTIONAL_MESSAGES.contains(&message)
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
    matches!(
        (message, group),
        // The strike/delta specification (`strike_or_delta_from_json`).
        ("StrikeOrDelta", "spec")
        // A rates curve pillar must name a tenor point (`pillar_tenor_from_json`
        // errors when neither `years`, `months`, nor `maturity_date` is present).
        | ("PillarTenor", "point")
        // A rates instrument must name its family (`rates_instrument_from_json`
        // errors when the `ois` arm is absent).
        | ("RatesInstrument", "instrument")
        // A combined-tail-risk FI leg must name its instrument arm (only `ois_swap`
        // today); a leg carrying no arm cannot build a `celnet_rates_risk::FiPosition`.
        | ("TailRiskFiPosition", "position")
        // A routing condition value must carry exactly one arm (num / text / list /
        // range) — the hand `route_value_desc_from_json` errors on an empty value.
        | ("RouteValueDesc", "v")
        // A routing node must be either a condition or a book leaf — the hand
        // `routing_node_desc_from_json` errors on a node carrying neither arm.
        | ("RoutingNodeDesc", "node")
        // A hedge node must be either a condition or a terminal exit action — the hand
        // `hedge_node_desc_from_json` errors on a node carrying neither arm.
        | ("HedgeNodeDesc", "node")
    )
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

/// (b) `VanillaInputs` synthesizes the flat foreign rate `r_for` from the FX carry
/// arm ([`VanillaInputs::r_for`]) — the risk-cube per-position analogue of
/// [`market_context_synth`]; `carry` is suppressed (see [`field_rule`]) and
/// `discount_rate` is renamed to `r_dom`. Byte-identical to the hand codec's
/// `vanilla_inputs_to_json`, which emits the same two FX accessors.
pub(crate) fn vanilla_inputs_synth(i: &VanillaInputs) -> Vec<(&'static str, Value)> {
    vec![("r_for", json!(i.r_for()))]
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

/// The `ArbReport` synthesized `smile_model_label`: a stable, machine-friendly label
/// for the numeric `smile_model` tag, surfaced beside the numeric provenance (the
/// hand `arb_report_to_json` emits both). Re-derived here **independently** from the
/// published `celnet_proto::SmileModel` enum (guardrail: re-derive constants from the
/// source, never call the hand codec); the differential harness proves this mapping
/// stays byte-identical to the hand `smile_model_label`. Vendor-/method-neutral by
/// name; an unrecognized tag is reported honestly as `unknown`.
pub(crate) fn arb_report_synth(smile_model: i32) -> Vec<(&'static str, Value)> {
    use celnet_proto::SmileModel;
    let label = match SmileModel::try_from(smile_model) {
        Ok(SmileModel::MarketHedge) => "market-hedge",
        Ok(SmileModel::StochasticVol) => "stochastic-vol",
        Ok(SmileModel::Parametric) => "parametric",
        Ok(SmileModel::ParametricSurface) => "parametric-surface",
        Ok(SmileModel::ExtendedSurface) => "extended-surface",
        Err(_) => "unknown",
    };
    vec![("smile_model_label", json!(label))]
}
