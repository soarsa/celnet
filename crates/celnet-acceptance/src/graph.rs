//! The acceptance decision graph — the acceptance-action vocabulary, the nodes, the
//! graph container, the error type, and well-formedness validation.
//!
//! An [`AcceptanceGraph`] is a directed graph of [`AcceptanceNode`]s entered at a
//! single node per evaluation. Internal nodes are [`AcceptanceNode::Condition`]s (a
//! typed test with `yes`/`no` edges, reusing `celnet-risk-routing`'s
//! [`RouteOp`]/[`RouteValue`]); leaves are [`AcceptanceNode::Decision`]s carrying one
//! [`AcceptanceAction`]. The one structural difference from a risk-routing / hedge
//! graph is that leaves are **acceptance decisions** (accept / reject / hold), not
//! book targets or exit actions — and, because a decision carries no external
//! registry target, [`AcceptanceGraph::validate`] needs no registries.
//! [`AcceptanceGraph::validate`] proves a graph is safe to resolve against —
//! collecting **every** defect (never stopping at the first) so a GUI can surface
//! them all at once, mirroring `RiskRoutingGraph::validate` / `HedgeGraph::validate`.

use crate::field::AcceptanceField;
use celnet_risk_routing::{FieldKind, RouteOp, RouteValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// A node identifier, unique within one graph.
pub type NodeId = u32;

/// A terminal acceptance action — the leaf a resolved acceptance path lands on.
///
/// The default/green leaf is [`AcceptanceAction::Accept`] (fill exactly as today);
/// [`AcceptanceAction::Reject`] refuses the lift with a wire-surfaced reason; and
/// [`AcceptanceAction::HoldForReview`] leaves the desk request pending so a human
/// accepts it via the existing desk-accept path instead of auto-filling. This is
/// the future home for credit checks + quote validations at the acceptance point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcceptanceAction {
    /// Accept the lift — proceed to book exactly as today.
    Accept,
    /// Reject the lift — do not book; surface `reason` to the counterparty.
    Reject {
        /// The rejection rationale, surfaced on the FIX `Text(58)`.
        reason: String,
    },
    /// Hold the lift for manual review — do not auto-book; leave the desk request
    /// pending so a human accepts it via the existing desk-accept path.
    HoldForReview {
        /// The manual-review rationale, surfaced on the FIX `Text(58)`.
        reason: String,
    },
}

impl AcceptanceAction {
    /// A short, stable kind label for provenance / UI / logging.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            AcceptanceAction::Accept => "ACCEPT",
            AcceptanceAction::Reject { .. } => "REJECT",
            AcceptanceAction::HoldForReview { .. } => "HOLD_FOR_REVIEW",
        }
    }
}

/// The resolved decision an acceptance walk lands on — the flattened result the
/// server acts on (the reason strings are carried inline so the caller need not
/// re-inspect the leaf action).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcceptanceDecision {
    /// Accept the lift and book it.
    Accept,
    /// Reject the lift with this reason.
    Reject(String),
    /// Hold the lift for manual review with this reason.
    Hold(String),
}

impl AcceptanceDecision {
    /// Whether the decision permits an automatic booking (only [`Self::Accept`]).
    #[must_use]
    pub fn is_accept(&self) -> bool {
        matches!(self, AcceptanceDecision::Accept)
    }
}

/// A node in an [`AcceptanceGraph`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AcceptanceNode {
    /// A decision node: evaluate `field op value` on the lift; on `true` follow
    /// `on_true`, else `on_false`.
    Condition {
        /// The lift field to test.
        field: AcceptanceField,
        /// The comparison operator (reused verbatim from risk routing).
        op: RouteOp,
        /// The literal compared against.
        value: RouteValue,
        /// Successor when the condition holds.
        on_true: NodeId,
        /// Successor when the condition does not hold.
        on_false: NodeId,
    },
    /// A terminal leaf: the lift resolves to this acceptance action.
    Decision {
        /// The acceptance action to take.
        action: AcceptanceAction,
    },
}

/// A validation or resolution defect. [`AcceptanceGraph::validate`] returns a `Vec`
/// of these (all defects at once); [`crate::AcceptanceEngine::evaluate`] never
/// surfaces one — it degrades a structurally-broken graph (which validation rules
/// out) to a safe manual-review [`AcceptanceDecision::Hold`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcceptanceError {
    /// The graph's `entry` id is not present in `nodes`.
    MissingEntry {
        /// The dangling entry id.
        entry: NodeId,
    },
    /// A condition edge points at a node id that does not exist.
    DanglingEdge {
        /// The condition node holding the edge.
        from: NodeId,
        /// The missing target id.
        to: NodeId,
    },
    /// A cycle is reachable from `entry` — some path would never terminate.
    Cycle,
    /// A condition's operator is not valid for its field's [`FieldKind`].
    OpNotValidForField {
        /// The offending node.
        node: NodeId,
        /// The field.
        field: AcceptanceField,
        /// The invalid operator.
        op: RouteOp,
    },
    /// A condition's value variant does not match its operator.
    ValueTypeMismatch {
        /// The offending node.
        node: NodeId,
        /// The field.
        field: AcceptanceField,
        /// The operator.
        op: RouteOp,
    },
}

impl fmt::Display for AcceptanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcceptanceError::MissingEntry { entry } => {
                write!(f, "entry node {entry} is not present in the graph")
            }
            AcceptanceError::DanglingEdge { from, to } => {
                write!(f, "node {from} has an edge to non-existent node {to}")
            }
            AcceptanceError::Cycle => write!(f, "a cycle is reachable from the entry node"),
            AcceptanceError::OpNotValidForField { node, field, op } => write!(
                f,
                "node {node}: operator {op:?} is not valid for field {field:?} ({:?})",
                field.kind()
            ),
            AcceptanceError::ValueTypeMismatch { node, field, op } => write!(
                f,
                "node {node}: value type is inconsistent with operator {op:?} on field {field:?}"
            ),
        }
    }
}

impl std::error::Error for AcceptanceError {}

/// A directed decision graph resolving an inbound lift to an acceptance decision,
/// entered at [`AcceptanceGraph::entry`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceGraph {
    /// The node at which every evaluation's walk begins.
    pub entry: NodeId,
    /// All nodes, keyed by id. A `BTreeMap` keeps serialization deterministic.
    pub nodes: BTreeMap<NodeId, AcceptanceNode>,
}

/// The default **accept-all** graph: a single [`AcceptanceAction::Accept`] leaf
/// entered directly. Seeded on a pristine store so existing behaviour is UNCHANGED
/// until a trader writes rules — every lift resolves to `Accept` and books exactly
/// as before.
#[must_use]
pub fn default_accept_all_graph() -> AcceptanceGraph {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        0u32,
        AcceptanceNode::Decision {
            action: AcceptanceAction::Accept,
        },
    );
    AcceptanceGraph { entry: 0, nodes }
}

/// DFS colours for reachable-cycle detection.
#[derive(Clone, Copy, PartialEq)]
enum Color {
    /// On the current DFS stack.
    Gray,
    /// Fully explored.
    Black,
}

impl AcceptanceGraph {
    /// The edge targets of a node: both successors of a `Condition`, none of a
    /// `Decision`.
    fn successors(&self, id: NodeId) -> Vec<NodeId> {
        match self.nodes.get(&id) {
            Some(AcceptanceNode::Condition {
                on_true, on_false, ..
            }) => vec![*on_true, *on_false],
            _ => Vec::new(),
        }
    }

    /// Whether the value variant of a condition is consistent with its operator.
    /// Assumes `op.valid_for(field.kind())` already held. Identical matrix to
    /// `RiskRoutingGraph`'s / `HedgeGraph`'s, over [`AcceptanceField`] kinds.
    fn value_matches_op(field: AcceptanceField, op: RouteOp, value: &RouteValue) -> bool {
        match op {
            RouteOp::Eq | RouteOp::Ne => match field.kind() {
                FieldKind::Numeric => matches!(value, RouteValue::Num(_)),
                FieldKind::String | FieldKind::Enum => matches!(value, RouteValue::Text(_)),
            },
            RouteOp::Gt | RouteOp::Ge | RouteOp::Lt | RouteOp::Le => {
                matches!(value, RouteValue::Num(_))
            }
            RouteOp::Contains => matches!(value, RouteValue::Text(_)),
            RouteOp::In => matches!(value, RouteValue::List(_)),
            RouteOp::Between => matches!(value, RouteValue::Range { .. }),
        }
    }

    /// Detect a cycle reachable from `entry` via iterative three-colour DFS.
    fn has_reachable_cycle(&self) -> bool {
        if !self.nodes.contains_key(&self.entry) {
            return false;
        }
        let mut color: BTreeMap<NodeId, Color> = BTreeMap::new();
        let mut stack: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
        color.insert(self.entry, Color::Gray);
        stack.push((self.entry, self.successors(self.entry)));

        while let Some(frame) = stack.last_mut() {
            if let Some(next) = frame.1.pop() {
                if !self.nodes.contains_key(&next) {
                    continue; // dangling — reported elsewhere
                }
                match color.get(&next) {
                    Some(Color::Gray) => return true, // back edge → cycle
                    Some(Color::Black) => {}          // already fully explored
                    None => {
                        color.insert(next, Color::Gray);
                        let succ = self.successors(next);
                        stack.push((next, succ));
                    }
                }
            } else {
                let done = frame.0;
                color.insert(done, Color::Black);
                stack.pop();
            }
        }
        false
    }

    /// Validate well-formedness, collecting **all** defects. `Ok(())` guarantees
    /// [`crate::AcceptanceEngine::evaluate`] is total: any
    /// [`AcceptanceContext`](crate::AcceptanceContext) walks a bounded, acyclic path
    /// to a `Decision` leaf.
    ///
    /// Rejects: a missing `entry`; any condition edge to a non-existent node; any
    /// cycle reachable from `entry`; any condition whose operator is invalid for its
    /// field kind or whose value variant is inconsistent with the operator.
    ///
    /// Unlike the hedge graph, a `Decision` leaf carries no external instrument / LP
    /// target, so there is no registry to validate against.
    ///
    /// # Errors
    /// Returns every collected [`AcceptanceError`] when the graph is malformed.
    pub fn validate(&self) -> Result<(), Vec<AcceptanceError>> {
        let mut errors = Vec::new();

        if !self.nodes.contains_key(&self.entry) {
            errors.push(AcceptanceError::MissingEntry { entry: self.entry });
        }

        for (&id, node) in &self.nodes {
            if let AcceptanceNode::Condition {
                field,
                op,
                value,
                on_true,
                on_false,
            } = node
            {
                for &to in &[*on_true, *on_false] {
                    if !self.nodes.contains_key(&to) {
                        errors.push(AcceptanceError::DanglingEdge { from: id, to });
                    }
                }
                if !op.valid_for(field.kind()) {
                    errors.push(AcceptanceError::OpNotValidForField {
                        node: id,
                        field: *field,
                        op: *op,
                    });
                } else if !Self::value_matches_op(*field, *op, value) {
                    errors.push(AcceptanceError::ValueTypeMismatch {
                        node: id,
                        field: *field,
                        op: *op,
                    });
                }
            }
        }

        if self.has_reachable_cycle() {
            errors.push(AcceptanceError::Cycle);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(
        field: AcceptanceField,
        op: RouteOp,
        value: RouteValue,
        t: NodeId,
        f: NodeId,
    ) -> AcceptanceNode {
        AcceptanceNode::Condition {
            field,
            op,
            value,
            on_true: t,
            on_false: f,
        }
    }

    fn decide(action: AcceptanceAction) -> AcceptanceNode {
        AcceptanceNode::Decision { action }
    }

    /// A minimal valid graph: `counterparty == "BAD" ? REJECT : ACCEPT`.
    fn valid_graph() -> AcceptanceGraph {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                AcceptanceField::Counterparty,
                RouteOp::Eq,
                RouteValue::Text("BAD".into()),
                1,
                2,
            ),
        );
        nodes.insert(
            1,
            decide(AcceptanceAction::Reject {
                reason: "blocked counterparty".into(),
            }),
        );
        nodes.insert(2, decide(AcceptanceAction::Accept));
        AcceptanceGraph { entry: 0, nodes }
    }

    #[test]
    fn default_accept_all_validates() {
        assert!(default_accept_all_graph().validate().is_ok());
    }

    #[test]
    fn valid_graph_passes() {
        assert!(valid_graph().validate().is_ok());
    }

    #[test]
    fn missing_entry_rejected() {
        let mut g = valid_graph();
        g.entry = 99;
        let errs = g.validate().unwrap_err();
        assert!(errs.contains(&AcceptanceError::MissingEntry { entry: 99 }));
    }

    #[test]
    fn dangling_edge_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(
            0,
            cond(
                AcceptanceField::Counterparty,
                RouteOp::Eq,
                RouteValue::Text("BAD".into()),
                1,
                77,
            ),
        );
        let errs = g.validate().unwrap_err();
        assert!(errs.contains(&AcceptanceError::DanglingEdge { from: 0, to: 77 }));
    }

    #[test]
    fn op_not_valid_for_field_rejected() {
        let mut g = valid_graph();
        // Counterparty (Enum) with Gt is invalid.
        g.nodes.insert(
            0,
            cond(
                AcceptanceField::Counterparty,
                RouteOp::Gt,
                RouteValue::Num(5.0),
                1,
                2,
            ),
        );
        let errs = g.validate().unwrap_err();
        assert!(errs.contains(&AcceptanceError::OpNotValidForField {
            node: 0,
            field: AcceptanceField::Counterparty,
            op: RouteOp::Gt,
        }));
    }

    #[test]
    fn value_type_mismatch_rejected() {
        let mut g = valid_graph();
        // NotionalUsd (Numeric) Eq but value is Text → mismatch.
        g.nodes.insert(
            0,
            cond(
                AcceptanceField::NotionalUsd,
                RouteOp::Eq,
                RouteValue::Text("x".into()),
                1,
                2,
            ),
        );
        let errs = g.validate().unwrap_err();
        assert!(errs.contains(&AcceptanceError::ValueTypeMismatch {
            node: 0,
            field: AcceptanceField::NotionalUsd,
            op: RouteOp::Eq,
        }));
    }

    #[test]
    fn direct_cycle_rejected() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                AcceptanceField::Counterparty,
                RouteOp::Eq,
                RouteValue::Text("A".into()),
                1,
                1,
            ),
        );
        nodes.insert(
            1,
            cond(
                AcceptanceField::Counterparty,
                RouteOp::Eq,
                RouteValue::Text("B".into()),
                0,
                2,
            ),
        );
        nodes.insert(2, decide(AcceptanceAction::Accept));
        let g = AcceptanceGraph { entry: 0, nodes };
        let errs = g.validate().unwrap_err();
        assert!(errs.contains(&AcceptanceError::Cycle));
    }

    #[test]
    fn collects_all_errors_at_once() {
        let mut nodes = BTreeMap::new();
        // Bad op AND dangling edge in one graph.
        nodes.insert(
            0,
            cond(
                AcceptanceField::Counterparty,
                RouteOp::Gt,
                RouteValue::Num(5.0),
                1,
                88,
            ),
        );
        nodes.insert(1, decide(AcceptanceAction::Accept));
        let g = AcceptanceGraph { entry: 0, nodes };
        let errs = g.validate().unwrap_err();
        assert!(errs.len() >= 2, "expected multiple errors, got {errs:?}");
    }

    #[test]
    fn action_kind_labels() {
        assert_eq!(AcceptanceAction::Accept.kind(), "ACCEPT");
        assert_eq!(
            AcceptanceAction::Reject { reason: "x".into() }.kind(),
            "REJECT"
        );
        assert_eq!(
            AcceptanceAction::HoldForReview { reason: "x".into() }.kind(),
            "HOLD_FOR_REVIEW"
        );
    }

    #[test]
    fn error_display_is_nonempty() {
        assert!(!AcceptanceError::Cycle.to_string().is_empty());
        assert!(
            !AcceptanceError::MissingEntry { entry: 1 }
                .to_string()
                .is_empty()
        );
    }
}
