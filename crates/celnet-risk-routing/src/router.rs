//! The resolver: a pure, bounded walk of a [`RiskRoutingGraph`] that lands one
//! fill on one risk book.
//!
//! [`RiskRouter::route`] is deterministic and allocation-light — it holds only a
//! `NodeId` cursor and a step counter, follows one edge per [`RoutingNode`], and
//! returns the first `Book` leaf's id. A validated graph is guaranteed acyclic
//! and terminating, so the step cap (`nodes.len() + 1`) is a belt-and-braces
//! guard that turns any residual cycle into a [`RouteError::Cycle`] rather than a
//! hang.

use crate::context::RoutingContext;
use crate::graph::{RiskRoutingGraph, RouteError, RoutingNode};

/// The stateless risk-routing resolver.
#[derive(Clone, Copy, Debug, Default)]
pub struct RiskRouter;

impl RiskRouter {
    /// Walk `graph` from its entry against `ctx`, returning the target risk-book
    /// id. Pure and `O(depth)`.
    ///
    /// Errors only on a structurally broken graph that
    /// [`RiskRoutingGraph::validate`] would already reject:
    /// [`RouteError::NodeNotFound`] (an edge/entry into a missing node) or
    /// [`RouteError::Cycle`] (the step cap tripped). On a validated graph this is
    /// always `Ok`.
    pub fn route<'g>(
        graph: &'g RiskRoutingGraph,
        ctx: &RoutingContext,
    ) -> Result<&'g str, RouteError> {
        let mut current = graph.entry;
        // A validated graph visits each node at most once; +1 lets a legitimate
        // terminal Book at the last step resolve before we declare a cycle.
        let cap = graph.nodes.len().saturating_add(1);

        for _ in 0..cap {
            let node = graph
                .nodes
                .get(&current)
                .ok_or(RouteError::NodeNotFound { node: current })?;
            match node {
                RoutingNode::Book { risk_book_id } => return Ok(risk_book_id.as_str()),
                RoutingNode::Condition {
                    field,
                    op,
                    value,
                    on_true,
                    on_false,
                } => {
                    let ctx_value = ctx.get(*field);
                    current = if op.eval(ctx_value, value) {
                        *on_true
                    } else {
                        *on_false
                    };
                }
            }
        }
        Err(RouteError::Cycle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{RouteField, RouteOp, RouteValue};
    use crate::graph::{NodeId, RoutingNode};
    use std::collections::BTreeMap;

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

    fn one_condition_graph() -> RiskRoutingGraph {
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
    fn routes_true_branch() {
        let g = one_condition_graph();
        let ctx = RoutingContext {
            ccy: "EUR".into(),
            ..Default::default()
        };
        assert_eq!(RiskRouter::route(&g, &ctx).unwrap(), "BOOK-A");
    }

    #[test]
    fn routes_false_branch() {
        let g = one_condition_graph();
        let ctx = RoutingContext {
            ccy: "USD".into(),
            ..Default::default()
        };
        assert_eq!(RiskRouter::route(&g, &ctx).unwrap(), "DEFAULT");
    }

    #[test]
    fn entry_is_a_book_returns_immediately() {
        let mut nodes = BTreeMap::new();
        nodes.insert(0, book("ONLY"));
        let g = RiskRoutingGraph { entry: 0, nodes };
        assert_eq!(
            RiskRouter::route(&g, &RoutingContext::default()).unwrap(),
            "ONLY"
        );
    }

    #[test]
    fn missing_entry_is_node_not_found() {
        let mut nodes = BTreeMap::new();
        nodes.insert(0, book("X"));
        let g = RiskRoutingGraph { entry: 9, nodes };
        assert_eq!(
            RiskRouter::route(&g, &RoutingContext::default()),
            Err(RouteError::NodeNotFound { node: 9 })
        );
    }

    #[test]
    fn cycle_trips_step_cap() {
        // An (invalid) self-referential condition: route must not hang.
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                RouteField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("EUR".into()),
                0,
                0,
            ),
        );
        let g = RiskRoutingGraph { entry: 0, nodes };
        let ctx = RoutingContext {
            ccy: "EUR".into(),
            ..Default::default()
        };
        assert_eq!(RiskRouter::route(&g, &ctx), Err(RouteError::Cycle));
    }
}
