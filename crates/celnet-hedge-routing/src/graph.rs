//! The exit-policy decision graph — the exit-action vocabulary, the nodes, the
//! graph container, the error type, and well-formedness validation.
//!
//! A [`HedgeGraph`] is a directed graph of [`HedgeNode`]s entered at a single node
//! per evaluation. Internal nodes are [`HedgeNode::Condition`]s (a typed test with
//! `yes`/`no` edges, reusing `celnet-risk-routing`'s [`RouteOp`]/[`RouteValue`]);
//! leaves are [`HedgeNode::Action`]s carrying one [`ExitAction`]. The one
//! structural difference from a risk-routing graph is that leaves are **exit
//! actions**, not book targets. [`HedgeGraph::validate`] proves a graph is safe to
//! resolve against — collecting **every** defect (never stopping at the first) so
//! a GUI can surface them all at once, mirroring `RiskRoutingGraph::validate`.

use crate::field::HedgeField;
use celnet_risk_routing::{FieldKind, RouteOp, RouteValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A node identifier, unique within one graph.
pub type NodeId = u32;

/// How much of the position an exit action targets.
///
/// `Overflow` (the default) hedges to the band edge — the transaction-cost-optimal
/// amount (`§4.3`, Whalley–Wilmott 1997); `Full` flattens the whole position;
/// `Fixed` names an explicit clip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum HedgeSize {
    /// Hedge the overflow beyond the band edge (default).
    Overflow,
    /// Flatten the whole net position.
    Full,
    /// An explicit magnitude in the budget metric's native units.
    Fixed(f64),
}

/// The execution schedule of an external hedge (`§3.4`).
///
/// `Immediate` places one clip onto the RFQ panel (true back-to-back);
/// `Worked` slices the clip on an Almgren–Chriss schedule to trade off market
/// impact against timing risk when the overflow is large relative to liquidity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecStyle {
    /// One clip, now (back-to-back).
    Immediate,
    /// An Almgren–Chriss-scheduled series of slices.
    Worked,
}

/// A terminal exit action — the leaf a resolved policy path lands on
/// (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §5.3).
///
/// The engine always internalises before it externalises (the internalisation-ratio
/// literature, §3.1): `Warehouse`/`Skew` cost nothing, `CrossInternal` nets at the
/// consolidated mid, and only `SubmitMarketOrder`/`RfqOut` pay the street.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ExitAction {
    /// Hold — do nothing, keep the risk. The default/green-band leaf; explicit so
    /// the graph is total.
    Warehouse,
    /// Offset against opposing internal flow / other desks in the Agg Book
    /// (internal cross at the consolidated mid).
    CrossInternal {
        /// The aggregation instrument to cross against (must exist).
        instrument: String,
        /// How much to cross.
        max_size: HedgeSize,
    },
    /// Lean the two-way to **attract** the offsetting side (passive
    /// internalisation) — the amber-band lever; no trade, just a quote lean.
    Skew {
        /// Explicit skew in bp, or `None` to lean to the band edge (`to_edge`).
        bp: Option<f64>,
        /// Whether to size the lean off the band-edge overflow rather than `bp`.
        to_edge: bool,
    },
    /// **Back-to-back**: place an offsetting external order onto the RFQ/FIX panel.
    SubmitMarketOrder {
        /// How much to hedge.
        size: HedgeSize,
        /// Immediate (one clip) or worked (Almgren–Chriss slices).
        style: ExecStyle,
    },
    /// Request a two-way from named external LPs and lift the best to hedge (the
    /// FI / large-clip externalisation path).
    RfqOut {
        /// The LPs to fan the request to (each must be a known, enabled LP).
        lps: Vec<String>,
        /// How much to hedge.
        size: HedgeSize,
    },
    /// Net internally up to `internal_offset_available`, then externalise the
    /// residual — the composite "internalise then hedge overflow" primitive.
    Split {
        /// Net internally first (vs. externalise first).
        internal_first: bool,
        /// Execution style for the externalised residual.
        style: ExecStyle,
    },
    /// Fire an alert / route to a human desk instead of auto-acting — for
    /// toxic/large/illiquid overflow the desk wants to hand-manage.
    Escalate {
        /// The escalation rationale, surfaced on the notification.
        reason: String,
    },
}

impl ExitAction {
    /// A short, stable kind label for provenance / UI / logging.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            ExitAction::Warehouse => "WAREHOUSE",
            ExitAction::CrossInternal { .. } => "CROSS_INTERNAL",
            ExitAction::Skew { .. } => "SKEW",
            ExitAction::SubmitMarketOrder { .. } => "SUBMIT_MARKET_ORDER",
            ExitAction::RfqOut { .. } => "RFQ_OUT",
            ExitAction::Split { .. } => "SPLIT",
            ExitAction::Escalate { .. } => "ESCALATE",
        }
    }

    /// Whether firing this action places a **real external order** on the street
    /// (the ops surface a desk arms behind the live-hedging flag). `CrossInternal`
    /// and `Skew` never leave the firm; `Warehouse`/`Escalate` place no order.
    #[must_use]
    pub fn is_external(&self) -> bool {
        matches!(
            self,
            ExitAction::SubmitMarketOrder { .. }
                | ExitAction::RfqOut { .. }
                | ExitAction::Split { .. }
        )
    }
}

/// A node in a [`HedgeGraph`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HedgeNode {
    /// A decision node: evaluate `field op value` on the risk state; on `true`
    /// follow `on_true`, else `on_false`.
    Condition {
        /// The risk-state field to test.
        field: HedgeField,
        /// The comparison operator (reused verbatim from risk routing).
        op: RouteOp,
        /// The literal compared against.
        value: RouteValue,
        /// Successor when the condition holds.
        on_true: NodeId,
        /// Successor when the condition does not hold.
        on_false: NodeId,
    },
    /// A terminal leaf: the risk state resolves to this exit action.
    Action {
        /// The exit action to fire.
        exit: ExitAction,
    },
}

/// A validation or resolution defect. [`HedgeGraph::validate`] returns a `Vec` of
/// these (all defects at once); [`crate::HedgeRouter::resolve`] returns a single
/// one on the rare structural failure a validated graph rules out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HedgeError {
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
    /// A reachable node id was not found during a resolve walk (a validated graph
    /// never yields this; it guards the resolver's own walk).
    NodeNotFound {
        /// The missing id.
        node: NodeId,
    },
    /// A condition's operator is not valid for its field's [`FieldKind`].
    OpNotValidForField {
        /// The offending node.
        node: NodeId,
        /// The field.
        field: HedgeField,
        /// The invalid operator.
        op: RouteOp,
    },
    /// A condition's value variant does not match its operator.
    ValueTypeMismatch {
        /// The offending node.
        node: NodeId,
        /// The field.
        field: HedgeField,
        /// The operator.
        op: RouteOp,
    },
    /// A `CrossInternal` action names an aggregation instrument that is not known.
    UnknownInstrument {
        /// The node holding the bad target.
        node: NodeId,
        /// The unknown instrument id.
        instrument: String,
    },
    /// An `RfqOut` action names an LP that is not a known, enabled liquidity
    /// provider.
    UnknownLp {
        /// The node holding the bad target.
        node: NodeId,
        /// The unknown LP id.
        lp: String,
    },
    /// An `RfqOut` action names no LPs at all — it could never route.
    EmptyRfqPanel {
        /// The node holding the empty panel.
        node: NodeId,
    },
}

impl fmt::Display for HedgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HedgeError::MissingEntry { entry } => {
                write!(f, "entry node {entry} is not present in the graph")
            }
            HedgeError::DanglingEdge { from, to } => {
                write!(f, "node {from} has an edge to non-existent node {to}")
            }
            HedgeError::Cycle => write!(f, "a cycle is reachable from the entry node"),
            HedgeError::NodeNotFound { node } => {
                write!(f, "node {node} was not found during resolution")
            }
            HedgeError::OpNotValidForField { node, field, op } => write!(
                f,
                "node {node}: operator {op:?} is not valid for field {field:?} ({:?})",
                field.kind()
            ),
            HedgeError::ValueTypeMismatch { node, field, op } => write!(
                f,
                "node {node}: value type is inconsistent with operator {op:?} on field {field:?}"
            ),
            HedgeError::UnknownInstrument { node, instrument } => write!(
                f,
                "node {node}: CROSS_INTERNAL targets unknown instrument {instrument:?}"
            ),
            HedgeError::UnknownLp { node, lp } => {
                write!(f, "node {node}: RFQ_OUT targets unknown LP {lp:?}")
            }
            HedgeError::EmptyRfqPanel { node } => {
                write!(f, "node {node}: RFQ_OUT names no LPs")
            }
        }
    }
}

impl std::error::Error for HedgeError {}

/// A directed decision graph resolving a book's risk state to an exit action,
/// entered at [`HedgeGraph::entry`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HedgeGraph {
    /// The node at which every evaluation's walk begins.
    pub entry: NodeId,
    /// All nodes, keyed by id. A `BTreeMap` keeps serialization deterministic.
    pub nodes: BTreeMap<NodeId, HedgeNode>,
}

/// DFS colours for reachable-cycle detection.
#[derive(Clone, Copy, PartialEq)]
enum Color {
    /// On the current DFS stack.
    Gray,
    /// Fully explored.
    Black,
}

impl HedgeGraph {
    /// The edge targets of a node: both successors of a `Condition`, none of an
    /// `Action`.
    fn successors(&self, id: NodeId) -> Vec<NodeId> {
        match self.nodes.get(&id) {
            Some(HedgeNode::Condition {
                on_true, on_false, ..
            }) => vec![*on_true, *on_false],
            _ => Vec::new(),
        }
    }

    /// Whether the value variant of a condition is consistent with its operator.
    /// Assumes `op.valid_for(field.kind())` already held. Identical matrix to
    /// `RiskRoutingGraph`'s, over [`HedgeField`] kinds.
    fn value_matches_op(field: HedgeField, op: RouteOp, value: &RouteValue) -> bool {
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

    /// Validate well-formedness against the live registries, collecting **all**
    /// defects. `Ok(())` guarantees [`crate::HedgeRouter::resolve`] is total: any
    /// [`HedgeContext`](crate::HedgeContext) walks a bounded, acyclic path to an
    /// `Action` leaf whose venue/LP/instrument targets exist.
    ///
    /// Rejects: a missing `entry`; any condition edge to a non-existent node; any
    /// cycle reachable from `entry`; any condition whose operator is invalid for
    /// its field kind or whose value variant is inconsistent with the operator;
    /// any `CrossInternal` naming an unknown aggregation instrument; any `RfqOut`
    /// with an empty panel or an unknown/disabled LP.
    pub fn validate(
        &self,
        known_instruments: &BTreeSet<String>,
        known_lps: &BTreeSet<String>,
    ) -> Result<(), Vec<HedgeError>> {
        let mut errors = Vec::new();

        if !self.nodes.contains_key(&self.entry) {
            errors.push(HedgeError::MissingEntry { entry: self.entry });
        }

        for (&id, node) in &self.nodes {
            match node {
                HedgeNode::Condition {
                    field,
                    op,
                    value,
                    on_true,
                    on_false,
                } => {
                    for &to in &[*on_true, *on_false] {
                        if !self.nodes.contains_key(&to) {
                            errors.push(HedgeError::DanglingEdge { from: id, to });
                        }
                    }
                    if !op.valid_for(field.kind()) {
                        errors.push(HedgeError::OpNotValidForField {
                            node: id,
                            field: *field,
                            op: *op,
                        });
                    } else if !Self::value_matches_op(*field, *op, value) {
                        errors.push(HedgeError::ValueTypeMismatch {
                            node: id,
                            field: *field,
                            op: *op,
                        });
                    }
                }
                HedgeNode::Action { exit } => {
                    Self::validate_action(id, exit, known_instruments, known_lps, &mut errors);
                }
            }
        }

        if self.has_reachable_cycle() {
            errors.push(HedgeError::Cycle);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate one action leaf's venue/LP/instrument targets.
    fn validate_action(
        id: NodeId,
        exit: &ExitAction,
        known_instruments: &BTreeSet<String>,
        known_lps: &BTreeSet<String>,
        errors: &mut Vec<HedgeError>,
    ) {
        match exit {
            ExitAction::CrossInternal { instrument, .. } => {
                if !known_instruments.contains(instrument) {
                    errors.push(HedgeError::UnknownInstrument {
                        node: id,
                        instrument: instrument.clone(),
                    });
                }
            }
            ExitAction::RfqOut { lps, .. } => {
                if lps.is_empty() {
                    errors.push(HedgeError::EmptyRfqPanel { node: id });
                }
                for lp in lps {
                    if !known_lps.contains(lp) {
                        errors.push(HedgeError::UnknownLp {
                            node: id,
                            lp: lp.clone(),
                        });
                    }
                }
            }
            // Warehouse / Skew / SubmitMarketOrder (default panel) / Split /
            // Escalate carry no external registry target to check.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruments() -> BTreeSet<String> {
        ["EURUSD", "GBPUSD"].iter().map(|s| s.to_string()).collect()
    }
    fn lps() -> BTreeSet<String> {
        ["LP-1", "LP-2", "LP-3"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn cond(field: HedgeField, op: RouteOp, value: RouteValue, t: NodeId, f: NodeId) -> HedgeNode {
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

    /// A minimal valid graph: `breached == false ? WAREHOUSE : SUBMIT_MARKET_ORDER`.
    fn valid_graph() -> HedgeGraph {
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
    fn valid_graph_passes() {
        assert!(valid_graph().validate(&instruments(), &lps()).is_ok());
    }

    #[test]
    fn missing_entry_rejected() {
        let mut g = valid_graph();
        g.entry = 99;
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.contains(&HedgeError::MissingEntry { entry: 99 }));
    }

    #[test]
    fn dangling_edge_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(
            0,
            cond(
                HedgeField::Breached,
                RouteOp::Eq,
                RouteValue::Text("false".into()),
                1,
                77,
            ),
        );
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.contains(&HedgeError::DanglingEdge { from: 0, to: 77 }));
    }

    #[test]
    fn unknown_cross_instrument_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(
            2,
            act(ExitAction::CrossInternal {
                instrument: "GHOST".into(),
                max_size: HedgeSize::Overflow,
            }),
        );
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.contains(&HedgeError::UnknownInstrument {
            node: 2,
            instrument: "GHOST".into(),
        }));
    }

    #[test]
    fn unknown_lp_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(
            2,
            act(ExitAction::RfqOut {
                lps: vec!["LP-1".into(), "LP-9".into()],
                size: HedgeSize::Overflow,
            }),
        );
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.contains(&HedgeError::UnknownLp {
            node: 2,
            lp: "LP-9".into(),
        }));
    }

    #[test]
    fn empty_rfq_panel_rejected() {
        let mut g = valid_graph();
        g.nodes.insert(
            2,
            act(ExitAction::RfqOut {
                lps: vec![],
                size: HedgeSize::Overflow,
            }),
        );
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.contains(&HedgeError::EmptyRfqPanel { node: 2 }));
    }

    #[test]
    fn op_not_valid_for_field_rejected() {
        let mut g = valid_graph();
        // Breached (Enum) with Gt is invalid.
        g.nodes.insert(
            0,
            cond(
                HedgeField::Breached,
                RouteOp::Gt,
                RouteValue::Num(5.0),
                1,
                2,
            ),
        );
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.contains(&HedgeError::OpNotValidForField {
            node: 0,
            field: HedgeField::Breached,
            op: RouteOp::Gt,
        }));
    }

    #[test]
    fn value_type_mismatch_rejected() {
        let mut g = valid_graph();
        // Overflow (Numeric) Eq but value is Text → mismatch.
        g.nodes.insert(
            0,
            cond(
                HedgeField::Overflow,
                RouteOp::Eq,
                RouteValue::Text("x".into()),
                1,
                2,
            ),
        );
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.contains(&HedgeError::ValueTypeMismatch {
            node: 0,
            field: HedgeField::Overflow,
            op: RouteOp::Eq,
        }));
    }

    #[test]
    fn direct_cycle_rejected() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0,
            cond(
                HedgeField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("EUR".into()),
                1,
                1,
            ),
        );
        nodes.insert(
            1,
            cond(
                HedgeField::Ccy,
                RouteOp::Eq,
                RouteValue::Text("USD".into()),
                0,
                2,
            ),
        );
        nodes.insert(2, act(ExitAction::Warehouse));
        let g = HedgeGraph { entry: 0, nodes };
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.contains(&HedgeError::Cycle));
    }

    #[test]
    fn collects_all_errors_at_once() {
        let mut nodes = BTreeMap::new();
        // Bad op AND dangling edge AND unknown LP in one graph.
        nodes.insert(
            0,
            cond(
                HedgeField::Breached,
                RouteOp::Gt,
                RouteValue::Num(5.0),
                1,
                88,
            ),
        );
        nodes.insert(
            1,
            act(ExitAction::RfqOut {
                lps: vec!["GHOST".into()],
                size: HedgeSize::Full,
            }),
        );
        let g = HedgeGraph { entry: 0, nodes };
        let errs = g.validate(&instruments(), &lps()).unwrap_err();
        assert!(errs.len() >= 3, "expected multiple errors, got {errs:?}");
    }

    #[test]
    fn action_kind_and_is_external() {
        assert_eq!(ExitAction::Warehouse.kind(), "WAREHOUSE");
        assert!(!ExitAction::Warehouse.is_external());
        assert!(
            !ExitAction::Skew {
                bp: None,
                to_edge: true
            }
            .is_external()
        );
        assert!(
            !ExitAction::CrossInternal {
                instrument: "EURUSD".into(),
                max_size: HedgeSize::Overflow
            }
            .is_external()
        );
        assert!(
            ExitAction::SubmitMarketOrder {
                size: HedgeSize::Overflow,
                style: ExecStyle::Immediate
            }
            .is_external()
        );
        assert!(
            ExitAction::RfqOut {
                lps: vec!["LP-1".into()],
                size: HedgeSize::Full
            }
            .is_external()
        );
    }

    #[test]
    fn error_display_is_nonempty() {
        assert!(!HedgeError::Cycle.to_string().is_empty());
        assert!(!HedgeError::EmptyRfqPanel { node: 1 }.to_string().is_empty());
    }
}
