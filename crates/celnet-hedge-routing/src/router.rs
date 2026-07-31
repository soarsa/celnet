//! The resolver: a pure, bounded walk of a [`HedgeGraph`] that lands one risk
//! state on one exit action.
//!
//! [`HedgeRouter::resolve`] is deterministic and allocation-light — it holds only
//! a `NodeId` cursor and a step counter, follows one edge per [`HedgeNode`], and
//! returns the first [`ExitAction`] leaf, plus the exact node path it walked (the
//! provenance trail). A validated graph is guaranteed acyclic and terminating, so
//! the step cap (`nodes.len() + 1`) is a belt-and-braces guard that turns any
//! residual cycle into a [`HedgeError::Cycle`] rather than a hang. This is
//! `RiskRouter::route`'s exact shape, with action leaves and a recorded path.

use crate::context::HedgeContext;
use crate::graph::{ExitAction, HedgeError, HedgeGraph, HedgeNode, NodeId};

/// The stateless auto-hedge exit-policy resolver.
#[derive(Clone, Copy, Debug, Default)]
pub struct HedgeRouter;

/// The outcome of a resolve: the fired exit action and the node path walked to
/// reach it (the provenance trail surfaced on `HedgeProvenance.policy_path`).
#[derive(Clone, Debug, PartialEq)]
pub struct Resolution<'g> {
    /// The first (and only) exit action for this risk state.
    pub action: &'g ExitAction,
    /// The exact ids visited, in order, ending at the action leaf.
    pub path: Vec<NodeId>,
}

impl HedgeRouter {
    /// Walk `graph` from its entry against `ctx`, returning the exit action and
    /// the node path walked. Pure and `O(depth)`.
    ///
    /// Errors only on a structurally broken graph that [`HedgeGraph::validate`]
    /// would already reject: [`HedgeError::NodeNotFound`] (an edge/entry into a
    /// missing node) or [`HedgeError::Cycle`] (the step cap tripped). On a
    /// validated graph this is always `Ok`.
    pub fn resolve<'g>(
        graph: &'g HedgeGraph,
        ctx: &HedgeContext,
    ) -> Result<Resolution<'g>, HedgeError> {
        let mut current = graph.entry;
        let cap = graph.nodes.len().saturating_add(1);
        let mut path = Vec::with_capacity(cap.min(16));

        for _ in 0..cap {
            path.push(current);
            let node = graph
                .nodes
                .get(&current)
                .ok_or(HedgeError::NodeNotFound { node: current })?;
            match node {
                HedgeNode::Action { exit } => return Ok(Resolution { action: exit, path }),
                HedgeNode::Condition {
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
        Err(HedgeError::Cycle)
    }

    /// Convenience: resolve to just the action (dropping the path) — the common
    /// case when the caller does not need the provenance trail.
    pub fn action<'g>(
        graph: &'g HedgeGraph,
        ctx: &HedgeContext,
    ) -> Result<&'g ExitAction, HedgeError> {
        Self::resolve(graph, ctx).map(|r| r.action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{ExecStyle, HedgeNode, HedgeSize};
    use celnet_risk_routing::{RouteOp, RouteValue};
    use std::collections::BTreeMap;

    fn cond(
        field: crate::field::HedgeField,
        op: RouteOp,
        value: RouteValue,
        t: NodeId,
        f: NodeId,
    ) -> HedgeNode {
        HedgeNode::Condition {
            field,
            op,
            value,
            on_true: t,
            on_false: f,
        }
    }
    fn act(exit: ExitAction) -> HedgeNode {
        HedgeNode::Action { exit }
    }

    use crate::field::HedgeField;

    fn one_condition_graph() -> HedgeGraph {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                HedgeField::Breached,
                RouteOp::Eq,
                RouteValue::Text("false".into()),
                1,
                2,
            ),
        );
        nodes.insert(1, act(ExitAction::Warehouse));
        nodes.insert(
            2,
            act(ExitAction::SubmitMarketOrder {
                size: HedgeSize::Overflow,
                style: ExecStyle::Immediate,
            }),
        );
        HedgeGraph { entry: 0, nodes }
    }

    #[test]
    fn resolves_true_branch_warehouse() {
        let g = one_condition_graph();
        let ctx = HedgeContext {
            breached: false,
            ..Default::default()
        };
        let r = HedgeRouter::resolve(&g, &ctx).unwrap();
        assert_eq!(r.action, &ExitAction::Warehouse);
        assert_eq!(r.path, vec![0, 1]);
    }

    #[test]
    fn resolves_false_branch_hedge() {
        let g = one_condition_graph();
        let ctx = HedgeContext {
            breached: true,
            ..Default::default()
        };
        let r = HedgeRouter::resolve(&g, &ctx).unwrap();
        assert_eq!(
            r.action,
            &ExitAction::SubmitMarketOrder {
                size: HedgeSize::Overflow,
                style: ExecStyle::Immediate
            }
        );
        assert_eq!(r.path, vec![0, 2]);
    }

    #[test]
    fn entry_is_an_action_returns_immediately() {
        let mut nodes = BTreeMap::new();
        nodes.insert(0, act(ExitAction::Warehouse));
        let g = HedgeGraph { entry: 0, nodes };
        let r = HedgeRouter::resolve(&g, &HedgeContext::default()).unwrap();
        assert_eq!(r.action, &ExitAction::Warehouse);
        assert_eq!(r.path, vec![0]);
    }

    #[test]
    fn missing_entry_is_node_not_found() {
        let mut nodes = BTreeMap::new();
        nodes.insert(0, act(ExitAction::Warehouse));
        let g = HedgeGraph { entry: 9, nodes };
        assert_eq!(
            HedgeRouter::resolve(&g, &HedgeContext::default()),
            Err(HedgeError::NodeNotFound { node: 9 })
        );
    }

    #[test]
    fn cycle_trips_step_cap() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                HedgeField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("EUR".into()),
                0,
                0,
            ),
        );
        let g = HedgeGraph { entry: 0, nodes };
        let ctx = HedgeContext {
            ccy: "EUR".into(),
            ..Default::default()
        };
        assert_eq!(HedgeRouter::resolve(&g, &ctx), Err(HedgeError::Cycle));
    }
}
