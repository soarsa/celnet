//! Domain ⇄ proto converters for the auto-hedge surface — the single place that maps
//! `celnet-hedge-routing` domain types + the persisted [`hedge_policy`](crate::config::hedge_policy)
//! config onto the wire [`celnet_proto`] descriptors and back. Shared by the
//! [`AutoHedgeEngine`](super::engine::AutoHedgeEngine) (which stamps proto provenance /
//! intents) and the `AuthService` hedge RPC handlers (which round-trip the policy graph,
//! thresholds and config).
//!
//! The condition vocabulary (`RouteOp` / `RouteValue`) is reused verbatim from risk
//! routing; the exit-action leaf is carried FLAT (a `kind` + the union of arm fields) so
//! the wire codec stays small (`ExitActionDesc`, `docs/…REQUIREMENTS.md` §5.3).

// `tonic::Status` is the house edge-error type carried by every `…_from_wire` `Result`
// (the same convention as `services::auth` / `services::risk_transfer::wire`): a large but
// standard error variant on a rare control-plane conversion, not a hot path.
#![allow(clippy::result_large_err)]

use celnet_hedge_routing::{
    ExecStyle, ExitAction, HedgeExitMode, HedgeField, HedgeGraph, HedgeLpPanel, HedgeNode,
    HedgeRatioPlan, HedgeSize, HedgeVehicle, HedgeVehicleRegistry, HedgeVehicleRule, HedgingModel,
    NodeId, RagStatus, RouteOp, RouteValue,
};
use celnet_proto::{
    ExecStyleEnum, ExitActionDesc, ExitActionKind, HedgeConditionDesc, HedgeConfigDesc,
    HedgeDeskToggle as HedgeDeskToggleDesc, HedgeExitModeBinding, HedgeExitModeEnum,
    HedgeFieldEnum, HedgeGraphDesc, HedgeLpPanelDesc, HedgeNodeDesc, HedgeSizeDesc, HedgeSizeKind,
    HedgeVehicleDesc, HedgeVehicleKindEnum, HedgeVehiclePlanDesc, HedgingModelBinding, RouteRange,
    RouteValueDesc, StringList, WarehouseThresholdDesc, hedge_node_desc, route_value_desc,
};
use tonic::Status;

use crate::config::hedge_policy::{
    HedgeConfigDef, HedgeDeskToggle, HedgeExecutionMode, HedgeMetric, HedgeScopeKind,
    HedgeThresholdDef, ScopedExitMode, ScopedHedgingModel, ScopedLpPanel, ScopedThreshold,
};

// --- RagStatus label --------------------------------------------------------

/// The stable lower-case band label carried on provenance / intents.
#[must_use]
pub fn band_label(status: RagStatus) -> &'static str {
    match status {
        RagStatus::Green => "green",
        RagStatus::Amber => "amber",
        RagStatus::Red => "red",
        RagStatus::Breach => "breach",
    }
}

// --- HedgeField ⇄ HedgeFieldEnum --------------------------------------------

/// The wire enum value for a domain [`HedgeField`] (exhaustive — no lossy default).
#[must_use]
pub fn hedge_field_to_wire(f: HedgeField) -> i32 {
    let e = match f {
        HedgeField::InstrumentId => HedgeFieldEnum::HedgeFieldInstrumentId,
        HedgeField::Ccy => HedgeFieldEnum::HedgeFieldCcy,
        HedgeField::Product => HedgeFieldEnum::HedgeFieldProduct,
        HedgeField::Book => HedgeFieldEnum::HedgeFieldBook,
        HedgeField::Desk => HedgeFieldEnum::HedgeFieldDesk,
        HedgeField::NetDv01 => HedgeFieldEnum::HedgeFieldNetDv01,
        HedgeField::NetNotional => HedgeFieldEnum::HedgeFieldNetNotional,
        HedgeField::NetVega => HedgeFieldEnum::HedgeFieldNetVega,
        HedgeField::NetGamma => HedgeFieldEnum::HedgeFieldNetGamma,
        HedgeField::InventorySign => HedgeFieldEnum::HedgeFieldInventorySign,
        HedgeField::Threshold => HedgeFieldEnum::HedgeFieldThreshold,
        HedgeField::Utilization => HedgeFieldEnum::HedgeFieldUtilization,
        HedgeField::Overflow => HedgeFieldEnum::HedgeFieldOverflow,
        HedgeField::Breached => HedgeFieldEnum::HedgeFieldBreached,
        HedgeField::CounterpartyToxicity => HedgeFieldEnum::HedgeFieldCounterpartyToxicity,
        HedgeField::InventoryAgeSecs => HedgeFieldEnum::HedgeFieldInventoryAgeSecs,
        HedgeField::InternalOffsetAvailable => HedgeFieldEnum::HedgeFieldInternalOffsetAvailable,
        HedgeField::HedgeCostBp => HedgeFieldEnum::HedgeFieldHedgeCostBp,
        HedgeField::Counterparty => HedgeFieldEnum::HedgeFieldCounterparty,
    };
    e as i32
}

/// Resolve a wire enum value onto a domain [`HedgeField`].
///
/// # Errors
/// An unknown ordinal, as an `invalid_argument` [`Status`].
pub fn hedge_field_from_wire(v: i32) -> Result<HedgeField, Status> {
    match HedgeFieldEnum::try_from(v) {
        Ok(HedgeFieldEnum::HedgeFieldInstrumentId) => Ok(HedgeField::InstrumentId),
        Ok(HedgeFieldEnum::HedgeFieldCcy) => Ok(HedgeField::Ccy),
        Ok(HedgeFieldEnum::HedgeFieldProduct) => Ok(HedgeField::Product),
        Ok(HedgeFieldEnum::HedgeFieldBook) => Ok(HedgeField::Book),
        Ok(HedgeFieldEnum::HedgeFieldDesk) => Ok(HedgeField::Desk),
        Ok(HedgeFieldEnum::HedgeFieldNetDv01) => Ok(HedgeField::NetDv01),
        Ok(HedgeFieldEnum::HedgeFieldNetNotional) => Ok(HedgeField::NetNotional),
        Ok(HedgeFieldEnum::HedgeFieldNetVega) => Ok(HedgeField::NetVega),
        Ok(HedgeFieldEnum::HedgeFieldNetGamma) => Ok(HedgeField::NetGamma),
        Ok(HedgeFieldEnum::HedgeFieldInventorySign) => Ok(HedgeField::InventorySign),
        Ok(HedgeFieldEnum::HedgeFieldThreshold) => Ok(HedgeField::Threshold),
        Ok(HedgeFieldEnum::HedgeFieldUtilization) => Ok(HedgeField::Utilization),
        Ok(HedgeFieldEnum::HedgeFieldOverflow) => Ok(HedgeField::Overflow),
        Ok(HedgeFieldEnum::HedgeFieldBreached) => Ok(HedgeField::Breached),
        Ok(HedgeFieldEnum::HedgeFieldCounterpartyToxicity) => Ok(HedgeField::CounterpartyToxicity),
        Ok(HedgeFieldEnum::HedgeFieldInventoryAgeSecs) => Ok(HedgeField::InventoryAgeSecs),
        Ok(HedgeFieldEnum::HedgeFieldInternalOffsetAvailable) => {
            Ok(HedgeField::InternalOffsetAvailable)
        }
        Ok(HedgeFieldEnum::HedgeFieldHedgeCostBp) => Ok(HedgeField::HedgeCostBp),
        Ok(HedgeFieldEnum::HedgeFieldCounterparty) => Ok(HedgeField::Counterparty),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown HedgeFieldEnum ordinal {v}"
        ))),
    }
}

// --- RouteOp ⇄ RouteOpEnum (reused from routing) ----------------------------

/// The wire enum value for a domain [`RouteOp`].
#[must_use]
pub fn route_op_to_wire(op: RouteOp) -> i32 {
    use celnet_proto::RouteOpEnum;
    let e = match op {
        RouteOp::Eq => RouteOpEnum::RouteOpEq,
        RouteOp::Ne => RouteOpEnum::RouteOpNe,
        RouteOp::Gt => RouteOpEnum::RouteOpGt,
        RouteOp::Ge => RouteOpEnum::RouteOpGe,
        RouteOp::Lt => RouteOpEnum::RouteOpLt,
        RouteOp::Le => RouteOpEnum::RouteOpLe,
        RouteOp::Contains => RouteOpEnum::RouteOpContains,
        RouteOp::In => RouteOpEnum::RouteOpIn,
        RouteOp::Between => RouteOpEnum::RouteOpBetween,
    };
    e as i32
}

/// Resolve a wire enum value onto a domain [`RouteOp`].
///
/// # Errors
/// An unknown ordinal, as an `invalid_argument` [`Status`].
pub fn route_op_from_wire(v: i32) -> Result<RouteOp, Status> {
    use celnet_proto::RouteOpEnum;
    match RouteOpEnum::try_from(v) {
        Ok(RouteOpEnum::RouteOpEq) => Ok(RouteOp::Eq),
        Ok(RouteOpEnum::RouteOpNe) => Ok(RouteOp::Ne),
        Ok(RouteOpEnum::RouteOpGt) => Ok(RouteOp::Gt),
        Ok(RouteOpEnum::RouteOpGe) => Ok(RouteOp::Ge),
        Ok(RouteOpEnum::RouteOpLt) => Ok(RouteOp::Lt),
        Ok(RouteOpEnum::RouteOpLe) => Ok(RouteOp::Le),
        Ok(RouteOpEnum::RouteOpContains) => Ok(RouteOp::Contains),
        Ok(RouteOpEnum::RouteOpIn) => Ok(RouteOp::In),
        Ok(RouteOpEnum::RouteOpBetween) => Ok(RouteOp::Between),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown RouteOpEnum ordinal {v}"
        ))),
    }
}

/// Map a stored [`RouteValue`] onto its wire [`RouteValueDesc`] oneof.
#[must_use]
pub fn route_value_to_wire(value: &RouteValue) -> RouteValueDesc {
    let v = match value {
        RouteValue::Num(x) => route_value_desc::V::Num(*x),
        RouteValue::Text(s) => route_value_desc::V::Text(s.clone()),
        RouteValue::List(items) => route_value_desc::V::List(StringList {
            values: items.clone(),
        }),
        RouteValue::Range { lo, hi } => route_value_desc::V::Range(RouteRange { lo: *lo, hi: *hi }),
    };
    RouteValueDesc { v: Some(v) }
}

/// Map a wire [`RouteValueDesc`] back onto a stored [`RouteValue`].
///
/// # Errors
/// An empty value oneof, as an `invalid_argument` [`Status`].
pub fn route_value_from_wire(d: RouteValueDesc) -> Result<RouteValue, Status> {
    match d.v {
        Some(route_value_desc::V::Num(x)) => Ok(RouteValue::Num(x)),
        Some(route_value_desc::V::Text(s)) => Ok(RouteValue::Text(s)),
        Some(route_value_desc::V::List(l)) => Ok(RouteValue::List(l.values)),
        Some(route_value_desc::V::Range(r)) => Ok(RouteValue::Range { lo: r.lo, hi: r.hi }),
        None => Err(Status::invalid_argument(
            "hedge condition value oneof is empty",
        )),
    }
}

// --- HedgeSize ⇄ HedgeSizeDesc ----------------------------------------------

/// The wire descriptor for a domain [`HedgeSize`].
#[must_use]
pub fn hedge_size_to_wire(size: HedgeSize) -> HedgeSizeDesc {
    let (kind, fixed) = match size {
        HedgeSize::Overflow => (HedgeSizeKind::HedgeSizeOverflow, 0.0),
        HedgeSize::Full => (HedgeSizeKind::HedgeSizeFull, 0.0),
        HedgeSize::Fixed(x) => (HedgeSizeKind::HedgeSizeFixed, x),
    };
    HedgeSizeDesc {
        kind: kind as i32,
        fixed,
    }
}

/// The domain [`HedgeSize`] for a wire descriptor (an unknown/absent kind ⇒ `Overflow`).
#[must_use]
pub fn hedge_size_from_wire(d: &HedgeSizeDesc) -> HedgeSize {
    match HedgeSizeKind::try_from(d.kind) {
        Ok(HedgeSizeKind::HedgeSizeFull) => HedgeSize::Full,
        Ok(HedgeSizeKind::HedgeSizeFixed) => HedgeSize::Fixed(d.fixed),
        _ => HedgeSize::Overflow,
    }
}

/// The wire enum value for a domain [`ExecStyle`].
#[must_use]
pub fn exec_style_to_wire(style: ExecStyle) -> i32 {
    let e = match style {
        ExecStyle::Immediate => ExecStyleEnum::ExecStyleImmediate,
        ExecStyle::Worked => ExecStyleEnum::ExecStyleWorked,
    };
    e as i32
}

/// The domain [`ExecStyle`] for a wire enum value (anything but `WORKED` ⇒ `Immediate`).
#[must_use]
pub fn exec_style_from_wire(v: i32) -> ExecStyle {
    match ExecStyleEnum::try_from(v) {
        Ok(ExecStyleEnum::ExecStyleWorked) => ExecStyle::Worked,
        _ => ExecStyle::Immediate,
    }
}

// --- ExitAction ⇄ ExitActionDesc (flat) -------------------------------------

/// Map a domain [`ExitAction`] onto its FLAT wire [`ExitActionDesc`] (only the fields
/// relevant to the action's kind are set; the rest carry their proto defaults).
#[must_use]
pub fn exit_action_to_wire(action: &ExitAction) -> ExitActionDesc {
    exit_action_with_vehicle_to_wire(action, &HedgeVehicle::SelfInstrument)
}

/// Map a domain [`ExitAction`] **and its hedge vehicle** onto the FLAT wire
/// [`ExitActionDesc`]. The vehicle lives on the graph NODE domain-side but rides FLAT on
/// the action descriptor here, which is what lets it apply uniformly to every action kind
/// (`ClearRisk` included) without a per-variant wire shape.
#[must_use]
pub fn exit_action_with_vehicle_to_wire(
    action: &ExitAction,
    vehicle: &HedgeVehicle,
) -> ExitActionDesc {
    let mut d = ExitActionDesc {
        kind: ExitActionKind::ExitActionWarehouse as i32,
        instrument: String::new(),
        size: None,
        skew_bp: None,
        to_edge: false,
        style: ExecStyleEnum::ExecStyleImmediate as i32,
        lps: Vec::new(),
        internal_first: false,
        reason: String::new(),
        vehicle_kind: hedge_vehicle_kind_to_wire(vehicle),
        vehicle_instrument: vehicle.named_instrument().unwrap_or_default().to_owned(),
    };
    match action {
        ExitAction::Warehouse => {}
        ExitAction::CrossInternal {
            instrument,
            max_size,
        } => {
            d.kind = ExitActionKind::ExitActionCrossInternal as i32;
            d.instrument = instrument.clone();
            d.size = Some(hedge_size_to_wire(*max_size));
        }
        ExitAction::Skew { bp, to_edge } => {
            d.kind = ExitActionKind::ExitActionSkew as i32;
            d.skew_bp = *bp;
            d.to_edge = *to_edge;
        }
        ExitAction::SubmitMarketOrder { size, style } => {
            d.kind = ExitActionKind::ExitActionSubmitMarketOrder as i32;
            d.size = Some(hedge_size_to_wire(*size));
            d.style = exec_style_to_wire(*style);
        }
        ExitAction::RfqOut { lps, size } => {
            d.kind = ExitActionKind::ExitActionRfqOut as i32;
            d.lps = lps.clone();
            d.size = Some(hedge_size_to_wire(*size));
        }
        ExitAction::Split {
            internal_first,
            style,
        } => {
            d.kind = ExitActionKind::ExitActionSplit as i32;
            d.internal_first = *internal_first;
            d.style = exec_style_to_wire(*style);
        }
        ExitAction::Escalate { reason } => {
            d.kind = ExitActionKind::ExitActionEscalate as i32;
            d.reason = reason.clone();
        }
        ExitAction::ClearRisk => {
            d.kind = ExitActionKind::ExitActionClearRisk as i32;
        }
    }
    d
}

/// Map a FLAT wire [`ExitActionDesc`] back onto a domain [`ExitAction`], reading only the
/// fields the `kind` selects.
///
/// # Errors
/// An unknown `kind` ordinal, as an `invalid_argument` [`Status`].
pub fn exit_action_from_wire(d: &ExitActionDesc) -> Result<ExitAction, Status> {
    let size = || {
        d.size
            .as_ref()
            .map_or(HedgeSize::Overflow, hedge_size_from_wire)
    };
    match ExitActionKind::try_from(d.kind) {
        Ok(ExitActionKind::ExitActionWarehouse) => Ok(ExitAction::Warehouse),
        Ok(ExitActionKind::ExitActionCrossInternal) => Ok(ExitAction::CrossInternal {
            instrument: d.instrument.clone(),
            max_size: size(),
        }),
        Ok(ExitActionKind::ExitActionSkew) => Ok(ExitAction::Skew {
            bp: d.skew_bp,
            to_edge: d.to_edge,
        }),
        Ok(ExitActionKind::ExitActionSubmitMarketOrder) => Ok(ExitAction::SubmitMarketOrder {
            size: size(),
            style: exec_style_from_wire(d.style),
        }),
        Ok(ExitActionKind::ExitActionRfqOut) => Ok(ExitAction::RfqOut {
            lps: d.lps.clone(),
            size: size(),
        }),
        Ok(ExitActionKind::ExitActionSplit) => Ok(ExitAction::Split {
            internal_first: d.internal_first,
            style: exec_style_from_wire(d.style),
        }),
        Ok(ExitActionKind::ExitActionEscalate) => Ok(ExitAction::Escalate {
            reason: d.reason.clone(),
        }),
        Ok(ExitActionKind::ExitActionClearRisk) => Ok(ExitAction::ClearRisk),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown ExitActionKind ordinal {}",
            d.kind
        ))),
    }
}

// --- HedgeVehicle ⇄ (vehicle_kind, vehicle_instrument) ----------------------

/// The wire enum ordinal for a domain [`HedgeVehicle`]'s kind.
#[must_use]
pub fn hedge_vehicle_kind_to_wire(vehicle: &HedgeVehicle) -> i32 {
    let e = match vehicle {
        HedgeVehicle::SelfInstrument => HedgeVehicleKindEnum::HedgeVehicleSelf,
        HedgeVehicle::Benchmark => HedgeVehicleKindEnum::HedgeVehicleBenchmark,
        HedgeVehicle::Instrument { .. } => HedgeVehicleKindEnum::HedgeVehicleInstrument,
        HedgeVehicle::Future { .. } => HedgeVehicleKindEnum::HedgeVehicleFuture,
    };
    e as i32
}

/// Rebuild a domain [`HedgeVehicle`] from an [`ExitActionDesc`]'s flat vehicle fields.
///
/// An **unknown or absent** `vehicle_kind` resolves to
/// [`HedgeVehicle::SelfInstrument`] — the proto default is `SELF` (ordinal 0), so a
/// descriptor written before vehicles existed decodes to the historical self-hedge and
/// nothing changes behaviour. This is deliberately lenient (never an error), matching
/// [`hedge_size_from_wire`]'s treatment of an unknown size kind.
#[must_use]
pub fn hedge_vehicle_from_wire(d: &ExitActionDesc) -> HedgeVehicle {
    match HedgeVehicleKindEnum::try_from(d.vehicle_kind) {
        Ok(HedgeVehicleKindEnum::HedgeVehicleBenchmark) => HedgeVehicle::Benchmark,
        Ok(HedgeVehicleKindEnum::HedgeVehicleInstrument) => HedgeVehicle::Instrument {
            instrument_id: d.vehicle_instrument.clone(),
        },
        Ok(HedgeVehicleKindEnum::HedgeVehicleFuture) => HedgeVehicle::Future {
            contract_id: d.vehicle_instrument.clone(),
        },
        _ => HedgeVehicle::SelfInstrument,
    }
}

// --- HedgeVehicleRule ⇄ HedgeVehicleDesc ------------------------------------

/// Map one persisted vehicle-registry row onto its wire descriptor.
#[must_use]
pub fn vehicle_rule_to_wire(r: &HedgeVehicleRule) -> HedgeVehicleDesc {
    HedgeVehicleDesc {
        id: r.id.clone(),
        instrument_id: r.instrument_id.clone(),
        product: r.product.clone(),
        ccy: r.ccy.clone(),
        min_maturity_years: r.min_maturity_years,
        max_maturity_years: r.max_maturity_years,
        hedge_instrument_id: r.hedge_instrument_id.clone(),
        is_future: r.is_future,
        dv01_per_unit: r.dv01_per_unit,
        unit_label: r.unit_label.clone(),
    }
}

/// Map a wire vehicle-registry descriptor back onto the persisted row.
#[must_use]
pub fn vehicle_rule_from_wire(d: &HedgeVehicleDesc) -> HedgeVehicleRule {
    HedgeVehicleRule {
        id: d.id.clone(),
        instrument_id: d.instrument_id.clone(),
        product: d.product.clone(),
        ccy: d.ccy.clone(),
        min_maturity_years: d.min_maturity_years,
        max_maturity_years: d.max_maturity_years,
        hedge_instrument_id: d.hedge_instrument_id.clone(),
        is_future: d.is_future,
        dv01_per_unit: d.dv01_per_unit,
        unit_label: d.unit_label.clone(),
    }
}

// --- HedgeExitMode ⇄ HedgeExitModeEnum / HedgeExitModeBinding ---------------

/// The wire enum ordinal for a domain [`HedgeExitMode`].
#[must_use]
pub fn exit_mode_to_wire(mode: HedgeExitMode) -> i32 {
    let e = match mode {
        HedgeExitMode::Auto => HedgeExitModeEnum::HedgeExitModeAuto,
        HedgeExitMode::Suggest => HedgeExitModeEnum::HedgeExitModeSuggest,
    };
    e as i32
}

/// The domain [`HedgeExitMode`] for a wire ordinal (anything but `SUGGEST` ⇒ `Auto`, so
/// an absent/unknown value keeps the historical fire-on-breach behaviour).
#[must_use]
pub fn exit_mode_from_wire(v: i32) -> HedgeExitMode {
    match HedgeExitModeEnum::try_from(v) {
        Ok(HedgeExitModeEnum::HedgeExitModeSuggest) => HedgeExitMode::Suggest,
        _ => HedgeExitMode::Auto,
    }
}

/// Map a persisted [`ScopedExitMode`] onto its wire binding.
#[must_use]
pub fn exit_mode_binding_to_wire(m: &ScopedExitMode) -> HedgeExitModeBinding {
    HedgeExitModeBinding {
        scope_kind: m.scope_kind.as_i32(),
        scope_id: m.scope_id.clone(),
        mode: exit_mode_to_wire(m.mode),
    }
}

/// Map a persisted [`ScopedHedgingModel`] onto its wire binding.
///
/// The posture and its DV01 budget travel together: a non-positive `dv01_budget` means
/// INHERIT the scope's configured threshold, and is carried across verbatim rather than
/// normalised here, so the wire never implies a cap the operator did not set.
#[must_use]
pub fn hedging_model_binding_to_wire(m: &ScopedHedgingModel) -> HedgingModelBinding {
    HedgingModelBinding {
        scope_kind: m.scope_kind.as_i32(),
        scope_id: m.scope_id.clone(),
        model: m.model.as_i32(),
        dv01_budget: m.dv01_budget,
    }
}

/// Map a wire binding onto the persisted [`ScopedHedgingModel`].
#[must_use]
pub fn hedging_model_binding_from_wire(d: &HedgingModelBinding) -> ScopedHedgingModel {
    ScopedHedgingModel {
        scope_kind: HedgeScopeKind::from_i32(d.scope_kind),
        scope_id: d.scope_id.clone(),
        // An unknown ordinal folds to `Custom` — the escape hatch that keeps the desk's
        // authored graph — so a wire value this build does not understand can never
        // silently re-post a book onto a derived posture.
        model: HedgingModel::from_i32(d.model),
        dv01_budget: d.dv01_budget,
    }
}

/// Map a wire binding onto the persisted [`ScopedExitMode`].
#[must_use]
pub fn exit_mode_binding_from_wire(d: &HedgeExitModeBinding) -> ScopedExitMode {
    ScopedExitMode {
        scope_kind: HedgeScopeKind::from_i32(d.scope_kind),
        scope_id: d.scope_id.clone(),
        mode: exit_mode_from_wire(d.mode),
    }
}

// --- HedgeRatioPlan → HedgeVehiclePlanDesc ----------------------------------

/// Project a sized vehicle hedge onto its wire descriptor, including the **DV01 basis**
/// and the `duration_correct` flag — so no surface can render the size as exact when it
/// was computed off the duration-blind exposure proxy.
#[must_use]
pub fn vehicle_plan_to_wire(plan: &HedgeRatioPlan) -> HedgeVehiclePlanDesc {
    HedgeVehiclePlanDesc {
        hedge_instrument_id: plan.hedge_instrument_id.clone(),
        unit_label: plan.unit_label.clone(),
        whole_units: plan.whole_units,
        dv01_basis: plan.basis.label().to_owned(),
        duration_correct: plan.basis.is_duration_correct(),
        target_dv01: plan.target_dv01,
        dv01_per_unit: plan.dv01_per_unit,
        exact_units: plan.exact_units,
        units: plan.units,
        hedged_dv01: plan.hedged_dv01,
        residual_dv01: plan.residual_dv01,
        summary: plan.summary(),
    }
}

// --- HedgeGraph ⇄ HedgeGraphDesc --------------------------------------------

/// Flatten a stored [`HedgeGraph`] into its wire [`HedgeGraphDesc`] (id-carrying node list).
#[must_use]
pub fn hedge_graph_to_wire(graph: &HedgeGraph) -> HedgeGraphDesc {
    let nodes = graph
        .nodes
        .iter()
        .map(|(&id, node)| hedge_node_to_wire(id, node))
        .collect();
    HedgeGraphDesc {
        entry: graph.entry,
        nodes,
    }
}

/// Map one `(id, HedgeNode)` onto a wire [`HedgeNodeDesc`].
#[must_use]
fn hedge_node_to_wire(id: NodeId, node: &HedgeNode) -> HedgeNodeDesc {
    let n = match node {
        HedgeNode::Condition {
            field,
            op,
            value,
            on_true,
            on_false,
        } => hedge_node_desc::Node::Condition(HedgeConditionDesc {
            field: hedge_field_to_wire(*field),
            op: route_op_to_wire(*op),
            value: Some(route_value_to_wire(value)),
            on_true: *on_true,
            on_false: *on_false,
        }),
        HedgeNode::Action { exit, vehicle } => {
            hedge_node_desc::Node::Action(exit_action_with_vehicle_to_wire(exit, vehicle))
        }
    };
    HedgeNodeDesc { id, node: Some(n) }
}

/// Rebuild a stored [`HedgeGraph`] from its wire [`HedgeGraphDesc`] (the id→node map).
///
/// # Errors
/// A node with no `node` oneof arm, a malformed condition, or an unknown enum ordinal.
pub fn hedge_graph_from_wire(d: HedgeGraphDesc) -> Result<HedgeGraph, Status> {
    let mut nodes = std::collections::BTreeMap::new();
    for nd in d.nodes {
        let id = nd.id;
        let node = match nd.node {
            Some(hedge_node_desc::Node::Condition(c)) => HedgeNode::Condition {
                field: hedge_field_from_wire(c.field)?,
                op: route_op_from_wire(c.op)?,
                value: route_value_from_wire(
                    c.value
                        .ok_or_else(|| Status::invalid_argument("hedge condition has no value"))?,
                )?,
                on_true: c.on_true,
                on_false: c.on_false,
            },
            Some(hedge_node_desc::Node::Action(a)) => HedgeNode::Action {
                exit: exit_action_from_wire(&a)?,
                vehicle: hedge_vehicle_from_wire(&a),
            },
            None => {
                return Err(Status::invalid_argument(format!(
                    "hedge node {id} carries neither a condition nor an action"
                )));
            }
        };
        nodes.insert(id, node);
    }
    Ok(HedgeGraph {
        entry: d.entry,
        nodes,
    })
}

// --- WarehouseThreshold (persisted) ⇄ WarehouseThresholdDesc ----------------

/// Map a persisted [`ScopedThreshold`] onto its wire [`WarehouseThresholdDesc`].
#[must_use]
pub fn threshold_to_wire(t: &ScopedThreshold) -> WarehouseThresholdDesc {
    WarehouseThresholdDesc {
        scope_kind: t.def.scope_kind.as_i32(),
        scope_id: t.scope_id.clone(),
        metric: t.def.metric.as_i32(),
        cap: t.def.cap,
        amber: t.def.amber,
        red: t.def.red,
        target_fraction: t.def.target_fraction,
        min_clip: t.def.min_clip,
        max_clip: t.def.max_clip,
        ramped: t.def.ramped,
        ramp_k: t.def.ramp_k,
    }
}

/// Map a wire [`WarehouseThresholdDesc`] onto a persisted [`ScopedThreshold`]. `max_clip`
/// of `0` (the proto default when unset) is treated as unbounded (`INFINITY`).
#[must_use]
pub fn threshold_from_wire(d: &WarehouseThresholdDesc) -> ScopedThreshold {
    let max_clip = if d.max_clip <= 0.0 {
        f64::INFINITY
    } else {
        d.max_clip
    };
    ScopedThreshold {
        scope_id: d.scope_id.clone(),
        def: HedgeThresholdDef {
            scope_kind: HedgeScopeKind::from_i32(d.scope_kind),
            metric: HedgeMetric::from_i32(d.metric),
            cap: d.cap,
            amber: d.amber,
            red: d.red,
            target_fraction: d.target_fraction,
            min_clip: d.min_clip,
            max_clip,
            ramped: d.ramped,
            ramp_k: d.ramp_k,
        },
    }
}

// --- HedgeLpPanel (scoped) ⇄ HedgeLpPanelDesc -------------------------------

/// Map a persisted [`ScopedLpPanel`] onto its wire [`HedgeLpPanelDesc`].
#[must_use]
pub fn lp_panel_to_wire(p: &ScopedLpPanel) -> HedgeLpPanelDesc {
    HedgeLpPanelDesc {
        scope_kind: p.scope_kind.as_i32(),
        scope_id: p.scope_id.clone(),
        include: p.panel.include.clone(),
        exclude: p.panel.exclude.clone(),
    }
}

/// Map a wire [`HedgeLpPanelDesc`] onto a persisted [`ScopedLpPanel`].
#[must_use]
pub fn lp_panel_from_wire(d: &HedgeLpPanelDesc) -> ScopedLpPanel {
    ScopedLpPanel {
        scope_kind: HedgeScopeKind::from_i32(d.scope_kind),
        scope_id: d.scope_id.clone(),
        panel: HedgeLpPanel {
            include: d.include.clone(),
            exclude: d.exclude.clone(),
        },
    }
}

// --- HedgeConfig ⇄ HedgeConfigDesc ------------------------------------------

/// Map the persisted [`HedgeConfigDef`] onto its wire [`HedgeConfigDesc`].
#[must_use]
pub fn config_to_wire(c: &HedgeConfigDef) -> HedgeConfigDesc {
    HedgeConfigDesc {
        kill_switch: c.kill_switch,
        execution: c.execution.as_i32(),
        composite_spread_bp: c.composite_spread_bp,
        desk_enabled: c
            .desk_enabled
            .iter()
            .map(|t| HedgeDeskToggleDesc {
                desk: t.desk.clone(),
                enabled: t.enabled,
            })
            .collect(),
        max_clip: c.max_clip,
        max_hedges_per_interval: c.max_hedges_per_interval,
        daily_external_notional_cap: c.daily_external_notional_cap,
        lp_panels: c.lp_panels.iter().map(lp_panel_to_wire).collect(),
        vehicles: c.vehicles.rules.iter().map(vehicle_rule_to_wire).collect(),
        exit_modes: c.exit_modes.iter().map(exit_mode_binding_to_wire).collect(),
        hedging_models: c
            .hedging_models
            .iter()
            .map(hedging_model_binding_to_wire)
            .collect(),
    }
}

/// Map a wire [`HedgeConfigDesc`] onto the persisted [`HedgeConfigDef`].
#[must_use]
pub fn config_from_wire(d: &HedgeConfigDesc) -> HedgeConfigDef {
    HedgeConfigDef {
        kill_switch: d.kill_switch,
        execution: HedgeExecutionMode::from_i32(d.execution),
        composite_spread_bp: d.composite_spread_bp,
        desk_enabled: d
            .desk_enabled
            .iter()
            .map(|t| HedgeDeskToggle {
                desk: t.desk.clone(),
                enabled: t.enabled,
            })
            .collect(),
        max_clip: d.max_clip,
        max_hedges_per_interval: d.max_hedges_per_interval,
        daily_external_notional_cap: d.daily_external_notional_cap,
        lp_panels: d.lp_panels.iter().map(lp_panel_from_wire).collect(),
        vehicles: HedgeVehicleRegistry::new(
            d.vehicles.iter().map(vehicle_rule_from_wire).collect(),
        ),
        exit_modes: d
            .exit_modes
            .iter()
            .map(exit_mode_binding_from_wire)
            .collect(),
        hedging_models: d
            .hedging_models
            .iter()
            .map(hedging_model_binding_from_wire)
            .collect(),
        // `min_edge_bps` is not carried on `HedgeConfigDesc` (a server-side price-tolerance
        // floor, not a GUI-edited engine control), so it defaults here; the `set_hedge_config`
        // handler PRESERVES the operator's stored value across a wire write.
        min_edge_bps: crate::services::internalise::DEFAULT_MIN_EDGE_BPS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hedge_field_round_trips_every_variant() {
        for f in [
            HedgeField::InstrumentId,
            HedgeField::Ccy,
            HedgeField::Product,
            HedgeField::Book,
            HedgeField::Desk,
            HedgeField::NetDv01,
            HedgeField::NetNotional,
            HedgeField::NetVega,
            HedgeField::NetGamma,
            HedgeField::InventorySign,
            HedgeField::Threshold,
            HedgeField::Utilization,
            HedgeField::Overflow,
            HedgeField::Breached,
            HedgeField::CounterpartyToxicity,
            HedgeField::InventoryAgeSecs,
            HedgeField::InternalOffsetAvailable,
            HedgeField::HedgeCostBp,
            HedgeField::Counterparty,
        ] {
            assert_eq!(hedge_field_from_wire(hedge_field_to_wire(f)).unwrap(), f);
        }
    }

    #[test]
    fn exit_action_round_trips_every_variant() {
        let actions = [
            ExitAction::Warehouse,
            ExitAction::CrossInternal {
                instrument: "EURUSD".into(),
                max_size: HedgeSize::Overflow,
            },
            ExitAction::Skew {
                bp: Some(2.5),
                to_edge: false,
            },
            ExitAction::Skew {
                bp: None,
                to_edge: true,
            },
            ExitAction::SubmitMarketOrder {
                size: HedgeSize::Full,
                style: ExecStyle::Worked,
            },
            ExitAction::RfqOut {
                lps: vec!["LP-1".into(), "LP-2".into()],
                size: HedgeSize::Fixed(5_000.0),
            },
            ExitAction::Split {
                internal_first: true,
                style: ExecStyle::Immediate,
            },
            ExitAction::Escalate {
                reason: "toxic".into(),
            },
        ];
        for a in actions {
            let round = exit_action_from_wire(&exit_action_to_wire(&a)).unwrap();
            assert_eq!(round, a, "exit action must round-trip through the wire");
        }
    }

    #[test]
    fn hedge_graph_round_trips() {
        use celnet_hedge_routing::HedgeGraph;
        use std::collections::BTreeMap;
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            HedgeNode::Condition {
                field: HedgeField::Breached,
                op: RouteOp::Eq,
                value: RouteValue::Text("false".into()),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(1, HedgeNode::action(ExitAction::Warehouse));
        nodes.insert(
            2,
            HedgeNode::action(ExitAction::CrossInternal {
                instrument: "EURUSD".into(),
                max_size: HedgeSize::Overflow,
            }),
        );
        let g = HedgeGraph { entry: 0, nodes };
        let round = hedge_graph_from_wire(hedge_graph_to_wire(&g)).unwrap();
        assert_eq!(round, g);
    }

    #[test]
    fn threshold_round_trips() {
        let t = ScopedThreshold {
            scope_id: "RATES-EUR".into(),
            def: HedgeThresholdDef {
                scope_kind: HedgeScopeKind::Book,
                metric: HedgeMetric::Dv01,
                cap: 100_000.0,
                amber: 0.8,
                red: 0.9,
                target_fraction: 0.8,
                min_clip: 1_000.0,
                max_clip: 50_000.0,
                ramped: true,
                ramp_k: 2.0,
            },
        };
        assert_eq!(threshold_from_wire(&threshold_to_wire(&t)), t);
    }

    #[test]
    fn config_round_trips() {
        let c = HedgeConfigDef {
            kill_switch: true,
            execution: HedgeExecutionMode::Composite,
            composite_spread_bp: 1.25,
            desk_enabled: vec![HedgeDeskToggle {
                desk: "FX".into(),
                enabled: false,
            }],
            max_clip: 25_000.0,
            max_hedges_per_interval: 10,
            daily_external_notional_cap: 1_000_000.0,
            lp_panels: vec![
                ScopedLpPanel {
                    scope_kind: HedgeScopeKind::Book,
                    scope_id: "RATES-EUR".into(),
                    panel: HedgeLpPanel {
                        include: vec!["LP-1".into(), "LP-2".into()],
                        exclude: vec!["LP-3".into()],
                    },
                },
                ScopedLpPanel {
                    scope_kind: HedgeScopeKind::Desk,
                    scope_id: "RATES".into(),
                    panel: HedgeLpPanel {
                        include: vec![],
                        exclude: vec!["LP-4".into()],
                    },
                },
            ],
            // `min_edge_bps` is a server-side price-tolerance floor NOT carried on the wire, so
            // `config_from_wire` recovers the default — the round-trip only holds when `c`
            // carries that same default here.
            min_edge_bps: crate::services::internalise::DEFAULT_MIN_EDGE_BPS,
            // The hedge-vehicle registry + the scoped exit modes round-trip through the SAME
            // config message (no separate CRUD RPC), so they must survive the wire hop.
            vehicles: HedgeVehicleRegistry::new(vec![HedgeVehicleRule {
                id: "US-BOND-10Y".into(),
                product: "BOND".into(),
                ccy: "USD".into(),
                min_maturity_years: 7.0,
                max_maturity_years: 12.0,
                hedge_instrument_id: "TY-DEC26".into(),
                is_future: true,
                dv01_per_unit: 78.0,
                unit_label: "contract".into(),
                ..HedgeVehicleRule::default()
            }]),
            exit_modes: vec![ScopedExitMode {
                scope_kind: HedgeScopeKind::Book,
                scope_id: "CREDIT".into(),
                mode: HedgeExitMode::Suggest,
            }],
        };
        assert_eq!(config_from_wire(&config_to_wire(&c)), c);
    }

    /// A hedge VEHICLE round-trips through the FLAT action descriptor, and — the
    /// back-compat contract — a descriptor with no vehicle set decodes to the self-hedge, so
    /// a policy authored before vehicles existed is byte-identical.
    #[test]
    fn hedge_vehicle_round_trips_and_defaults_to_self() {
        for v in [
            HedgeVehicle::SelfInstrument,
            HedgeVehicle::Benchmark,
            HedgeVehicle::Instrument {
                instrument_id: "US912810TM0".into(),
            },
            HedgeVehicle::Future {
                contract_id: "TY-DEC26".into(),
            },
        ] {
            let d = exit_action_with_vehicle_to_wire(&ExitAction::ClearRisk, &v);
            assert_eq!(hedge_vehicle_from_wire(&d), v, "vehicle must round-trip");
            assert_eq!(
                exit_action_from_wire(&d).unwrap(),
                ExitAction::ClearRisk,
                "the action is unaffected by its vehicle"
            );
        }
        // A descriptor written before vehicles existed carries kind 0 / no instrument.
        let legacy = exit_action_to_wire(&ExitAction::Warehouse);
        assert_eq!(legacy.vehicle_kind, 0);
        assert!(legacy.vehicle_instrument.is_empty());
        assert_eq!(
            hedge_vehicle_from_wire(&legacy),
            HedgeVehicle::SelfInstrument
        );
    }

    /// A whole GRAPH carrying a vehicle leaf survives the wire hop, which is what makes the
    /// choice persistable in `identity.json` and editable in the GUI.
    #[test]
    fn a_graph_with_a_vehicle_leaf_round_trips() {
        use std::collections::BTreeMap;
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            HedgeNode::action_with(
                ExitAction::SubmitMarketOrder {
                    size: HedgeSize::Overflow,
                    style: ExecStyle::Immediate,
                },
                HedgeVehicle::Future {
                    contract_id: "TY-DEC26".into(),
                },
            ),
        );
        let g = HedgeGraph { entry: 0, nodes };
        assert_eq!(hedge_graph_from_wire(hedge_graph_to_wire(&g)).unwrap(), g);
    }

    #[test]
    fn vehicle_rule_and_exit_mode_round_trip() {
        let r = HedgeVehicleRule {
            id: "R1".into(),
            instrument_id: "XS-CORP-9Y".into(),
            product: "BOND".into(),
            ccy: "USD".into(),
            min_maturity_years: 7.0,
            max_maturity_years: 12.0,
            hedge_instrument_id: "TY-DEC26".into(),
            is_future: true,
            dv01_per_unit: 78.0,
            unit_label: "contract".into(),
        };
        assert_eq!(vehicle_rule_from_wire(&vehicle_rule_to_wire(&r)), r);
        for m in [HedgeExitMode::Auto, HedgeExitMode::Suggest] {
            assert_eq!(exit_mode_from_wire(exit_mode_to_wire(m)), m);
        }
        // An unknown ordinal degrades to AUTO — the historical behaviour, never SUGGEST.
        assert_eq!(exit_mode_from_wire(99), HedgeExitMode::Auto);
    }

    #[test]
    fn lp_panel_round_trips() {
        let p = ScopedLpPanel {
            scope_kind: HedgeScopeKind::Instrument,
            scope_id: "EURUSD".into(),
            panel: HedgeLpPanel {
                include: vec!["LP-A".into()],
                exclude: vec!["LP-B".into(), "LP-C".into()],
            },
        };
        assert_eq!(lp_panel_from_wire(&lp_panel_to_wire(&p)), p);
    }
}
