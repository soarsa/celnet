//! The decision graph — nodes, the graph container, the error type, and
//! well-formedness validation.
//!
//! A [`RiskRoutingGraph`] is a directed graph of [`RoutingNode`]s entered at a
//! single node per fill. Internal nodes are [`RoutingNode::Condition`]s (a typed
//! test with `yes`/`no` edges); leaves are [`RoutingNode::Book`]s (a terminal
//! risk-book target). [`RiskRoutingGraph::validate`] proves a graph is safe to
//! route against — collecting **every** defect (never stopping at the first) so a
//! GUI can surface them all at once, mirroring the server's `check_pricing_group`.

use crate::field::{RouteField, RouteOp, RouteValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A node identifier, unique within one graph.
pub type NodeId = u32;

/// A node in a [`RiskRoutingGraph`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RoutingNode {
    /// A decision node: evaluate `field op value` on the fill; on `true` follow
    /// `on_true`, else `on_false`.
    Condition {
        /// The field to test.
        field: RouteField,
        /// The comparison operator.
        op: RouteOp,
        /// The literal compared against.
        value: RouteValue,
        /// Successor when the condition holds.
        on_true: NodeId,
        /// Successor when the condition does not hold.
        on_false: NodeId,
    },
    /// A terminal leaf: the fill's risk routes to this risk book.
    Book {
        /// The target risk-book slug (must exist in the known-books set).
        risk_book_id: String,
    },
}

/// A validation or routing defect. [`RiskRoutingGraph::validate`] returns a
/// `Vec` of these (all defects at once); [`crate::RiskRouter::route`] returns a
/// single one on the rare structural failure a validated graph rules out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
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
    /// A reachable node id was not found during a route walk (a validated graph
    /// never yields this; it guards the router's own walk).
    NodeNotFound {
        /// The missing id.
        node: NodeId,
    },
    /// A `Book` leaf targets a risk-book id absent from the known-books set.
    UnknownBook {
        /// The node holding the bad target.
        node: NodeId,
        /// The unknown book id.
        book: String,
    },
    /// A condition's operator is not valid for its field's [`crate::FieldKind`]
    /// (e.g. `side > 5`).
    OpNotValidForField {
        /// The offending node.
        node: NodeId,
        /// The field.
        field: RouteField,
        /// The invalid operator.
        op: RouteOp,
    },
    /// A condition's value variant does not match its operator (e.g. `In`
    /// without a `List`, `Between` without a `Range`, a numeric field compared
    /// to `Text`).
    ValueTypeMismatch {
        /// The offending node.
        node: NodeId,
        /// The field.
        field: RouteField,
        /// The operator.
        op: RouteOp,
    },
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteError::MissingEntry { entry } => {
                write!(f, "entry node {entry} is not present in the graph")
            }
            RouteError::DanglingEdge { from, to } => {
                write!(f, "node {from} has an edge to non-existent node {to}")
            }
            RouteError::Cycle => write!(f, "a cycle is reachable from the entry node"),
            RouteError::NodeNotFound { node } => {
                write!(f, "node {node} was not found during routing")
            }
            RouteError::UnknownBook { node, book } => {
                write!(f, "node {node} targets unknown risk book {book:?}")
            }
            RouteError::OpNotValidForField { node, field, op } => write!(
                f,
                "node {node}: operator {op:?} is not valid for field {field:?} ({:?})",
                field.kind()
            ),
            RouteError::ValueTypeMismatch { node, field, op } => write!(
                f,
                "node {node}: value type is inconsistent with operator {op:?} on field {field:?}"
            ),
        }
    }
}

impl std::error::Error for RouteError {}

/// A directed decision graph routing an accepted fill to a risk book, entered at
/// [`RiskRoutingGraph::entry`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiskRoutingGraph {
    /// The node at which every fill's walk begins.
    pub entry: NodeId,
    /// All nodes, keyed by id. A `BTreeMap` keeps serialization deterministic.
    pub nodes: BTreeMap<NodeId, RoutingNode>,
}

/// DFS colours for reachable-cycle detection.
#[derive(Clone, Copy, PartialEq)]
enum Color {
    /// On the current DFS stack.
    Gray,
    /// Fully explored.
    Black,
}

impl RiskRoutingGraph {
    /// The edge targets of a node: both successors of a `Condition`, none of a
    /// `Book`.
    fn successors(&self, id: NodeId) -> Vec<NodeId> {
        match self.nodes.get(&id) {
            Some(RoutingNode::Condition {
                on_true, on_false, ..
            }) => vec![*on_true, *on_false],
            _ => Vec::new(),
        }
    }

    /// Whether the value variant of a condition is consistent with its operator.
    /// Assumes `op.valid_for(field.kind())` already held.
    fn value_matches_op(field: RouteField, op: RouteOp, value: &RouteValue) -> bool {
        use crate::field::FieldKind;
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
    /// Missing entry / dangling edges are handled separately; here they simply
    /// terminate a branch.
    fn has_reachable_cycle(&self) -> bool {
        if !self.nodes.contains_key(&self.entry) {
            return false;
        }
        let mut color: BTreeMap<NodeId, Color> = BTreeMap::new();
        // Each stack frame carries a node and its not-yet-visited successors.
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

    /// Validate well-formedness against the set of `known_books`, collecting
    /// **all** defects. `Ok(())` guarantees [`crate::RiskRouter::route`] is total:
    /// any [`RoutingContext`](crate::RoutingContext) walks a bounded, acyclic path
    /// to a `Book` leaf whose id is known.
    ///
    /// Rejects: a missing `entry`; any condition edge to a non-existent node; any
    /// cycle reachable from `entry`; any `Book` targeting an unknown book; any
    /// condition whose operator is invalid for its field kind or whose value
    /// variant is inconsistent with the operator. (Acyclic + no reachable
    /// dangling edges + finite nodes ⇒ every reachable path terminates at a
    /// `Book`, so termination needs no separate check.)
    pub fn validate(&self, known_books: &BTreeSet<String>) -> Result<(), Vec<RouteError>> {
        let mut errors = Vec::new();

        if !self.nodes.contains_key(&self.entry) {
            errors.push(RouteError::MissingEntry { entry: self.entry });
        }

        for (&id, node) in &self.nodes {
            match node {
                RoutingNode::Condition {
                    field,
                    op,
                    value,
                    on_true,
                    on_false,
                } => {
                    for &to in &[*on_true, *on_false] {
                        if !self.nodes.contains_key(&to) {
                            errors.push(RouteError::DanglingEdge { from: id, to });
                        }
                    }
                    if !op.valid_for(field.kind()) {
                        errors.push(RouteError::OpNotValidForField {
                            node: id,
                            field: *field,
                            op: *op,
                        });
                    } else if !Self::value_matches_op(*field, *op, value) {
                        errors.push(RouteError::ValueTypeMismatch {
                            node: id,
                            field: *field,
                            op: *op,
                        });
                    }
                }
                RoutingNode::Book { risk_book_id } => {
                    if !known_books.contains(risk_book_id) {
                        errors.push(RouteError::UnknownBook {
                            node: id,
                            book: risk_book_id.clone(),
                        });
                    }
                }
            }
        }

        if self.has_reachable_cycle() {
            errors.push(RouteError::Cycle);
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

    fn books() -> BTreeSet<String> {
        ["BOOK-A", "DEFAULT"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn cond(
        field: RouteField,
        op: RouteOp,
        value: RouteValue,
        t: NodeId,
        f: NodeId,
    ) -> RoutingNode {
        RoutingNode::Condition {
            field,
            op,
            value,
            on_true: t,
            on_false: f,
        }
    }

    fn book(id: &str) -> RoutingNode {
        RoutingNode::Book {
            risk_book_id: id.to_string(),
        }
    }

    /// A minimal valid graph: one condition to two book leaves.
    fn valid_graph() -> RiskRoutingGraph {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                RouteField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("EUR".into()),
                1,
                2,
            ),
        );
        nodes.insert(1, book("BOOK-A"));
        nodes.insert(2, book("DEFAULT"));
        RiskRoutingGraph { entry: 0, nodes }
    }

    #[test]
    fn valid_graph_passes() {
        assert!(valid_graph().validate(&books()).is_ok());
    }

    #[test]
    fn missing_entry_rejected() {
        let mut g = valid_graph();
        g.entry = 99;
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.contains(&RouteError::MissingEntry { entry: 99 }));
    }

    #[test]
    fn dangling_edge_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(
            0,
            cond(
                RouteField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("EUR".into()),
                1,
                77,
            ),
        );
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.contains(&RouteError::DanglingEdge { from: 0, to: 77 }));
    }

    #[test]
    fn unknown_book_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(1, book("GHOST"));
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.contains(&RouteError::UnknownBook {
            node: 1,
            book: "GHOST".into(),
        }));
    }

    #[test]
    fn op_not_valid_for_field_rejected() {
        let mut g = valid_graph();
        // side (Enum) with Gt is invalid.
        g.nodes.insert(
            0,
            cond(RouteField::Side, RouteOp::Gt, RouteValue::Num(5.0), 1, 2),
        );
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.contains(&RouteError::OpNotValidForField {
            node: 0,
            field: RouteField::Side,
            op: RouteOp::Gt,
        }));
    }

    #[test]
    fn value_type_mismatch_rejected() {
        let mut g = valid_graph();
        // notional (Numeric) Eq but value is Text → mismatch.
        g.nodes.insert(
            0,
            cond(
                RouteField::Notional,
                RouteOp::Eq,
                RouteValue::Text("x".into()),
                1,
                2,
            ),
        );
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.contains(&RouteError::ValueTypeMismatch {
            node: 0,
            field: RouteField::Notional,
            op: RouteOp::Eq,
        }));
    }

    #[test]
    fn between_without_range_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(
            0,
            cond(
                RouteField::Notional,
                RouteOp::Between,
                RouteValue::Num(5.0),
                1,
                2,
            ),
        );
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.contains(&RouteError::ValueTypeMismatch {
            node: 0,
            field: RouteField::Notional,
            op: RouteOp::Between,
        }));
    }

    #[test]
    fn in_without_list_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(
            0,
            cond(
                RouteField::Counterparty,
                RouteOp::In,
                RouteValue::Text("HF-1".into()),
                1,
                2,
            ),
        );
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.contains(&RouteError::ValueTypeMismatch {
            node: 0,
            field: RouteField::Counterparty,
            op: RouteOp::In,
        }));
    }

    #[test]
    fn direct_cycle_rejected() {
        let mut nodes = BTreeMap::new();
        // 0 -> (1 | 1); 1 -> (0 | 2); 2 book. 0..1..0 is a cycle.
        nodes.insert(
            0,
            cond(
                RouteField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("EUR".into()),
                1,
                1,
            ),
        );
        nodes.insert(
            1,
            cond(
                RouteField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("USD".into()),
                0,
                2,
            ),
        );
        nodes.insert(2, book("DEFAULT"));
        let g = RiskRoutingGraph { entry: 0, nodes };
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.contains(&RouteError::Cycle));
    }

    #[test]
    fn self_loop_rejected() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                RouteField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("EUR".into()),
                0,
                1,
            ),
        );
        nodes.insert(1, book("DEFAULT"));
        let g = RiskRoutingGraph { entry: 0, nodes };
        assert!(
            g.validate(&books())
                .unwrap_err()
                .contains(&RouteError::Cycle)
        );
    }

    #[test]
    fn collects_all_errors_at_once() {
        let mut nodes = BTreeMap::new();
        // Bad op AND dangling edge AND unknown book in one graph.
        nodes.insert(
            0,
            cond(RouteField::Side, RouteOp::Gt, RouteValue::Num(5.0), 1, 88),
        );
        nodes.insert(1, book("GHOST"));
        let g = RiskRoutingGraph { entry: 0, nodes };
        let errs = g.validate(&books()).unwrap_err();
        assert!(errs.len() >= 3, "expected multiple errors, got {errs:?}");
    }

    #[test]
    fn error_display_is_nonempty() {
        let e = RouteError::Cycle;
        assert!(!e.to_string().is_empty());
    }
}
