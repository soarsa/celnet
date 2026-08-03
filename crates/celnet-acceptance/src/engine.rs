//! The resolver: a pure, bounded walk of an [`AcceptanceGraph`] that lands one
//! inbound lift on one acceptance decision.
//!
//! [`AcceptanceEngine::evaluate`] is deterministic and allocation-light — it holds
//! only a `NodeId` cursor and a step counter, follows one edge per
//! [`AcceptanceNode`], and returns the first [`AcceptanceDecision`] leaf plus the id
//! of the matched leaf. A validated graph is guaranteed acyclic and terminating, so
//! the step cap (`nodes.len() + 1`) is a belt-and-braces guard; a
//! structurally-broken graph (which [`AcceptanceGraph::validate`] rules out at the
//! server write) degrades to a **safe** manual-review
//! [`AcceptanceDecision::Hold`] — never a silent auto-accept and never a hang. This
//! is `RiskRouter::route`'s / `HedgeRouter::resolve`'s exact shape, made total in the
//! decision domain (no `Result`).

use crate::context::AcceptanceContext;
use crate::graph::{AcceptanceAction, AcceptanceDecision, AcceptanceGraph, AcceptanceNode, NodeId};

/// The stateless incoming-quote-acceptance resolver.
#[derive(Clone, Copy, Debug, Default)]
pub struct AcceptanceEngine;

/// The outcome of an evaluation: the resolved decision and the id of the leaf it
/// landed on (`None` when a structurally-broken graph forced the safe fallback).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptanceOutcome {
    /// The acceptance decision the lift resolves to.
    pub decision: AcceptanceDecision,
    /// The `Decision` leaf id the walk landed on, or `None` on the structural
    /// fallback.
    pub matched: Option<NodeId>,
}

impl AcceptanceEngine {
    /// Walk `graph` from its entry against `ctx`, returning the first
    /// [`AcceptanceDecision`] leaf and its node id. Pure and `O(depth)`.
    ///
    /// On a validated graph this always lands on a real `Decision` leaf. On a
    /// structurally-broken graph (a missing entry/edge target, or the step cap
    /// tripping on a residual cycle) it degrades to
    /// [`AcceptanceDecision::Hold`] with `matched: None` — the conservative choice
    /// that neither auto-fills nor silently rejects legitimate flow, routing the
    /// lift to a human instead.
    #[must_use]
    pub fn evaluate(&self, graph: &AcceptanceGraph, ctx: &AcceptanceContext) -> AcceptanceOutcome {
        let mut current = graph.entry;
        let cap = graph.nodes.len().saturating_add(1);

        for _ in 0..cap {
            let Some(node) = graph.nodes.get(&current) else {
                return Self::fallback();
            };
            match node {
                AcceptanceNode::Decision { action } => {
                    return AcceptanceOutcome {
                        decision: decision_of(action),
                        matched: Some(current),
                    };
                }
                AcceptanceNode::Condition {
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
        Self::fallback()
    }

    /// The safe outcome for a structurally-broken graph: hold for manual review.
    fn fallback() -> AcceptanceOutcome {
        AcceptanceOutcome {
            decision: AcceptanceDecision::Hold(
                "acceptance policy is structurally invalid — routed for manual review".to_owned(),
            ),
            matched: None,
        }
    }
}

/// Flatten a leaf [`AcceptanceAction`] into the resolved [`AcceptanceDecision`],
/// carrying the reason string inline.
fn decision_of(action: &AcceptanceAction) -> AcceptanceDecision {
    match action {
        AcceptanceAction::Accept => AcceptanceDecision::Accept,
        AcceptanceAction::Reject { reason } => AcceptanceDecision::Reject(reason.clone()),
        AcceptanceAction::HoldForReview { reason } => AcceptanceDecision::Hold(reason.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::AcceptanceField;
    use crate::graph::default_accept_all_graph;
    use celnet_risk_routing::{RouteOp, RouteValue};
    use std::collections::BTreeMap;

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

    #[test]
    fn accept_all_default_accepts() {
        let g = default_accept_all_graph();
        let out = AcceptanceEngine.evaluate(&g, &AcceptanceContext::default());
        assert_eq!(out.decision, AcceptanceDecision::Accept);
        assert_eq!(out.matched, Some(0));
    }

    #[test]
    fn rejects_by_counterparty() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                AcceptanceField::Counterparty,
                RouteOp::Eq,
                RouteValue::Text("BLOCKED".into()),
                1,
                2,
            ),
        );
        nodes.insert(
            1,
            decide(AcceptanceAction::Reject {
                reason: "counterparty blocked".into(),
            }),
        );
        nodes.insert(2, decide(AcceptanceAction::Accept));
        let g = AcceptanceGraph { entry: 0, nodes };

        let blocked = AcceptanceContext {
            counterparty: "BLOCKED".into(),
            ..Default::default()
        };
        let out = AcceptanceEngine.evaluate(&g, &blocked);
        assert_eq!(
            out.decision,
            AcceptanceDecision::Reject("counterparty blocked".into())
        );
        assert_eq!(out.matched, Some(1));

        let ok = AcceptanceContext {
            counterparty: "GOOD".into(),
            ..Default::default()
        };
        assert_eq!(
            AcceptanceEngine.evaluate(&g, &ok).decision,
            AcceptanceDecision::Accept
        );
    }

    #[test]
    fn rejects_when_notional_over_cap() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                AcceptanceField::NotionalUsd,
                RouteOp::Gt,
                RouteValue::Num(100_000_000.0),
                1,
                2,
            ),
        );
        nodes.insert(
            1,
            decide(AcceptanceAction::Reject {
                reason: "over notional cap".into(),
            }),
        );
        nodes.insert(2, decide(AcceptanceAction::Accept));
        let g = AcceptanceGraph { entry: 0, nodes };

        let big = AcceptanceContext {
            notional_usd: 250_000_000.0,
            ..Default::default()
        };
        assert_eq!(
            AcceptanceEngine.evaluate(&g, &big).decision,
            AcceptanceDecision::Reject("over notional cap".into())
        );
        let small = AcceptanceContext {
            notional_usd: 5_000_000.0,
            ..Default::default()
        };
        assert_eq!(
            AcceptanceEngine.evaluate(&g, &small).decision,
            AcceptanceDecision::Accept
        );
    }

    #[test]
    fn rejects_when_edge_below_floor() {
        // An unprofitable lift (edge_bps < 0.2) is rejected.
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                AcceptanceField::EdgeBps,
                RouteOp::Lt,
                RouteValue::Num(0.2),
                1,
                2,
            ),
        );
        nodes.insert(
            1,
            decide(AcceptanceAction::Reject {
                reason: "below edge floor".into(),
            }),
        );
        nodes.insert(2, decide(AcceptanceAction::Accept));
        let g = AcceptanceGraph { entry: 0, nodes };

        let thin = AcceptanceContext {
            edge_bps: -0.5,
            ..Default::default()
        };
        assert_eq!(
            AcceptanceEngine.evaluate(&g, &thin).decision,
            AcceptanceDecision::Reject("below edge floor".into())
        );
        let fat = AcceptanceContext {
            edge_bps: 1.5,
            ..Default::default()
        };
        assert_eq!(
            AcceptanceEngine.evaluate(&g, &fat).decision,
            AcceptanceDecision::Accept
        );
    }

    #[test]
    fn holds_by_condition() {
        // A large clip from a named counterparty is held for manual review.
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                AcceptanceField::QuoteAgeMs,
                RouteOp::Gt,
                RouteValue::Num(800.0),
                1,
                2,
            ),
        );
        nodes.insert(
            1,
            decide(AcceptanceAction::HoldForReview {
                reason: "stale quote — desk review".into(),
            }),
        );
        nodes.insert(2, decide(AcceptanceAction::Accept));
        let g = AcceptanceGraph { entry: 0, nodes };

        let stale = AcceptanceContext {
            quote_age_ms: 1_500.0,
            ..Default::default()
        };
        let out = AcceptanceEngine.evaluate(&g, &stale);
        assert_eq!(
            out.decision,
            AcceptanceDecision::Hold("stale quote — desk review".into())
        );
        assert_eq!(out.matched, Some(1));
    }

    #[test]
    fn entry_is_a_decision_returns_immediately() {
        let mut nodes = BTreeMap::new();
        nodes.insert(0, decide(AcceptanceAction::Accept));
        let g = AcceptanceGraph { entry: 0, nodes };
        let out = AcceptanceEngine.evaluate(&g, &AcceptanceContext::default());
        assert_eq!(out.decision, AcceptanceDecision::Accept);
        assert_eq!(out.matched, Some(0));
    }

    #[test]
    fn missing_entry_degrades_to_hold() {
        let mut nodes = BTreeMap::new();
        nodes.insert(0, decide(AcceptanceAction::Accept));
        let g = AcceptanceGraph { entry: 9, nodes };
        let out = AcceptanceEngine.evaluate(&g, &AcceptanceContext::default());
        assert!(matches!(out.decision, AcceptanceDecision::Hold(_)));
        assert_eq!(out.matched, None);
    }

    #[test]
    fn cycle_trips_step_cap_and_degrades_to_hold() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                AcceptanceField::Counterparty,
                RouteOp::Eq,
                RouteValue::Text("X".into()),
                0,
                0,
            ),
        );
        let g = AcceptanceGraph { entry: 0, nodes };
        let ctx = AcceptanceContext {
            counterparty: "X".into(),
            ..Default::default()
        };
        let out = AcceptanceEngine.evaluate(&g, &ctx);
        assert!(matches!(out.decision, AcceptanceDecision::Hold(_)));
        assert_eq!(out.matched, None);
    }
}
