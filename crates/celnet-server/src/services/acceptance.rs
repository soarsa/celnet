//! Domain ⇄ proto converters for the incoming-quote-acceptance surface — the single
//! place that maps `celnet-acceptance` domain types onto the wire [`celnet_proto`]
//! descriptors and back. Used by the `AuthService` acceptance RPC handlers (which
//! round-trip the acceptance graph get/update).
//!
//! The condition vocabulary (`RouteOp` / `RouteValue`) is reused verbatim from risk
//! routing — the same converters the auto-hedge wire uses
//! ([`crate::services::auto_hedge::wire`]) — so there is one source of truth for the
//! decision-graph condition semantics. The acceptance-action leaf is carried FLAT (a
//! `kind` + a `reason`) so the wire codec stays small (`AcceptanceActionDesc`).

// `tonic::Status` is the house edge-error type carried by every `…_from_wire` `Result`
// (the same convention as `services::auth` / `services::auto_hedge::wire`): a large but
// standard error variant on a rare control-plane conversion, not a hot path.
#![allow(clippy::result_large_err)]

use celnet_acceptance::{
    AcceptanceAction, AcceptanceField, AcceptanceGraph, AcceptanceNode, NodeId,
};
use celnet_proto::{
    AcceptanceActionDesc, AcceptanceActionKind, AcceptanceConditionDesc, AcceptanceFieldEnum,
    AcceptanceGraphDesc, AcceptanceNodeDesc, acceptance_node_desc,
};
use tonic::Status;

use crate::services::auto_hedge::wire::{
    route_op_from_wire, route_op_to_wire, route_value_from_wire, route_value_to_wire,
};

// --- AcceptanceField ⇄ AcceptanceFieldEnum ----------------------------------

/// The wire enum value for a domain [`AcceptanceField`] (exhaustive — no lossy default).
#[must_use]
pub fn acceptance_field_to_wire(f: AcceptanceField) -> i32 {
    let e = match f {
        AcceptanceField::Counterparty => AcceptanceFieldEnum::AcceptanceFieldCounterparty,
        AcceptanceField::NotionalUsd => AcceptanceFieldEnum::AcceptanceFieldNotionalUsd,
        AcceptanceField::TenorYears => AcceptanceFieldEnum::AcceptanceFieldTenorYears,
        AcceptanceField::InstrumentSymbol => AcceptanceFieldEnum::AcceptanceFieldInstrumentSymbol,
        AcceptanceField::Side => AcceptanceFieldEnum::AcceptanceFieldSide,
        AcceptanceField::EdgeBps => AcceptanceFieldEnum::AcceptanceFieldEdgeBps,
        AcceptanceField::QuoteAgeMs => AcceptanceFieldEnum::AcceptanceFieldQuoteAgeMs,
        AcceptanceField::AssetClass => AcceptanceFieldEnum::AcceptanceFieldAssetClass,
        AcceptanceField::Desk => AcceptanceFieldEnum::AcceptanceFieldDesk,
    };
    e as i32
}

/// Resolve a wire enum value onto a domain [`AcceptanceField`].
///
/// # Errors
/// An unknown ordinal, as an `invalid_argument` [`Status`].
pub fn acceptance_field_from_wire(v: i32) -> Result<AcceptanceField, Status> {
    match AcceptanceFieldEnum::try_from(v) {
        Ok(AcceptanceFieldEnum::AcceptanceFieldCounterparty) => Ok(AcceptanceField::Counterparty),
        Ok(AcceptanceFieldEnum::AcceptanceFieldNotionalUsd) => Ok(AcceptanceField::NotionalUsd),
        Ok(AcceptanceFieldEnum::AcceptanceFieldTenorYears) => Ok(AcceptanceField::TenorYears),
        Ok(AcceptanceFieldEnum::AcceptanceFieldInstrumentSymbol) => {
            Ok(AcceptanceField::InstrumentSymbol)
        }
        Ok(AcceptanceFieldEnum::AcceptanceFieldSide) => Ok(AcceptanceField::Side),
        Ok(AcceptanceFieldEnum::AcceptanceFieldEdgeBps) => Ok(AcceptanceField::EdgeBps),
        Ok(AcceptanceFieldEnum::AcceptanceFieldQuoteAgeMs) => Ok(AcceptanceField::QuoteAgeMs),
        Ok(AcceptanceFieldEnum::AcceptanceFieldAssetClass) => Ok(AcceptanceField::AssetClass),
        Ok(AcceptanceFieldEnum::AcceptanceFieldDesk) => Ok(AcceptanceField::Desk),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown AcceptanceFieldEnum ordinal {v}"
        ))),
    }
}

// --- AcceptanceAction ⇄ AcceptanceActionDesc (flat) -------------------------

/// Map a domain [`AcceptanceAction`] onto its FLAT wire [`AcceptanceActionDesc`] (the
/// `reason` is empty for `ACCEPT`).
#[must_use]
pub fn acceptance_action_to_wire(action: &AcceptanceAction) -> AcceptanceActionDesc {
    let (kind, reason) = match action {
        AcceptanceAction::Accept => (AcceptanceActionKind::AcceptanceActionAccept, String::new()),
        AcceptanceAction::Reject { reason } => {
            (AcceptanceActionKind::AcceptanceActionReject, reason.clone())
        }
        AcceptanceAction::HoldForReview { reason } => (
            AcceptanceActionKind::AcceptanceActionHoldForReview,
            reason.clone(),
        ),
    };
    AcceptanceActionDesc {
        kind: kind as i32,
        reason,
    }
}

/// Map a FLAT wire [`AcceptanceActionDesc`] back onto a domain [`AcceptanceAction`],
/// reading `reason` only for the kinds that carry one.
///
/// # Errors
/// An unknown `kind` ordinal, as an `invalid_argument` [`Status`].
pub fn acceptance_action_from_wire(d: &AcceptanceActionDesc) -> Result<AcceptanceAction, Status> {
    match AcceptanceActionKind::try_from(d.kind) {
        Ok(AcceptanceActionKind::AcceptanceActionAccept) => Ok(AcceptanceAction::Accept),
        Ok(AcceptanceActionKind::AcceptanceActionReject) => Ok(AcceptanceAction::Reject {
            reason: d.reason.clone(),
        }),
        Ok(AcceptanceActionKind::AcceptanceActionHoldForReview) => {
            Ok(AcceptanceAction::HoldForReview {
                reason: d.reason.clone(),
            })
        }
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown AcceptanceActionKind ordinal {}",
            d.kind
        ))),
    }
}

// --- AcceptanceGraph ⇄ AcceptanceGraphDesc ----------------------------------

/// Flatten a stored [`AcceptanceGraph`] into its wire [`AcceptanceGraphDesc`]
/// (id-carrying node list; the `BTreeMap` keys make it deterministically id-ordered).
#[must_use]
pub fn acceptance_graph_to_wire(graph: &AcceptanceGraph) -> AcceptanceGraphDesc {
    let nodes = graph
        .nodes
        .iter()
        .map(|(&id, node)| acceptance_node_to_wire(id, node))
        .collect();
    AcceptanceGraphDesc {
        entry: graph.entry,
        nodes,
    }
}

/// Map one `(id, AcceptanceNode)` onto a wire [`AcceptanceNodeDesc`].
#[must_use]
fn acceptance_node_to_wire(id: NodeId, node: &AcceptanceNode) -> AcceptanceNodeDesc {
    let n = match node {
        AcceptanceNode::Condition {
            field,
            op,
            value,
            on_true,
            on_false,
        } => acceptance_node_desc::Node::Condition(AcceptanceConditionDesc {
            field: acceptance_field_to_wire(*field),
            op: route_op_to_wire(*op),
            value: Some(route_value_to_wire(value)),
            on_true: *on_true,
            on_false: *on_false,
        }),
        AcceptanceNode::Decision { action } => {
            acceptance_node_desc::Node::Decision(acceptance_action_to_wire(action))
        }
    };
    AcceptanceNodeDesc { id, node: Some(n) }
}

/// Rebuild a stored [`AcceptanceGraph`] from its wire [`AcceptanceGraphDesc`] (the
/// id→node map). Structural well-formedness (acyclic, type-consistent) is left to the
/// store's `set_acceptance_graph` validation on write.
///
/// # Errors
/// A node with no `node` oneof arm, a duplicate id, a malformed condition, or an unknown
/// enum ordinal.
pub fn acceptance_graph_from_wire(d: AcceptanceGraphDesc) -> Result<AcceptanceGraph, Status> {
    let mut nodes = std::collections::BTreeMap::new();
    for nd in d.nodes {
        let id = nd.id;
        let node = match nd.node {
            Some(acceptance_node_desc::Node::Condition(c)) => AcceptanceNode::Condition {
                field: acceptance_field_from_wire(c.field)?,
                op: route_op_from_wire(c.op)?,
                value: route_value_from_wire(c.value.ok_or_else(|| {
                    Status::invalid_argument("acceptance condition has no value")
                })?)?,
                on_true: c.on_true,
                on_false: c.on_false,
            },
            Some(acceptance_node_desc::Node::Decision(a)) => AcceptanceNode::Decision {
                action: acceptance_action_from_wire(&a)?,
            },
            None => {
                return Err(Status::invalid_argument(format!(
                    "acceptance node {id} carries neither a condition nor a decision"
                )));
            }
        };
        if nodes.insert(id, node).is_some() {
            return Err(Status::invalid_argument(format!(
                "duplicate acceptance node id {id}"
            )));
        }
    }
    Ok(AcceptanceGraph {
        entry: d.entry,
        nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_acceptance::{RouteOp, RouteValue};
    use std::collections::BTreeMap;

    #[test]
    fn acceptance_field_round_trips_every_variant() {
        for f in [
            AcceptanceField::Counterparty,
            AcceptanceField::NotionalUsd,
            AcceptanceField::TenorYears,
            AcceptanceField::InstrumentSymbol,
            AcceptanceField::Side,
            AcceptanceField::EdgeBps,
            AcceptanceField::QuoteAgeMs,
            AcceptanceField::AssetClass,
            AcceptanceField::Desk,
        ] {
            assert_eq!(
                acceptance_field_from_wire(acceptance_field_to_wire(f)).unwrap(),
                f
            );
        }
    }

    #[test]
    fn acceptance_action_round_trips_every_variant() {
        for a in [
            AcceptanceAction::Accept,
            AcceptanceAction::Reject {
                reason: "blocked".into(),
            },
            AcceptanceAction::HoldForReview {
                reason: "desk review".into(),
            },
        ] {
            let round = acceptance_action_from_wire(&acceptance_action_to_wire(&a)).unwrap();
            assert_eq!(round, a);
        }
    }

    #[test]
    fn acceptance_graph_round_trips() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            AcceptanceNode::Condition {
                field: AcceptanceField::Counterparty,
                op: RouteOp::Eq,
                value: RouteValue::Text("BLOCKED".into()),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1,
            AcceptanceNode::Decision {
                action: AcceptanceAction::Reject {
                    reason: "blocked counterparty".into(),
                },
            },
        );
        nodes.insert(
            2,
            AcceptanceNode::Decision {
                action: AcceptanceAction::Accept,
            },
        );
        let g = AcceptanceGraph { entry: 0, nodes };
        let round = acceptance_graph_from_wire(acceptance_graph_to_wire(&g)).unwrap();
        assert_eq!(round, g);
    }
}
