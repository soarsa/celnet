//! Celnet **incoming-quote acceptance** — the pure, server-free decision-graph
//! engine that gates whether an inbound counterparty *lift* is **accepted**,
//! **rejected**, or **held for manual review**.
//!
//! When a counterparty lifts a live quote, the desk faces a gate before it commits:
//! *should we fill this at all?* Credit head-room, a stale quote, an unprofitable
//! edge, an over-cap clip, a blocked name — any of these should turn an otherwise
//! valid lift away, or route it to a human. Traders express that policy exactly like
//! a risk-routing / hedge graph: an `IF <lift-field> <op> <value> THEN
//! <acceptance-action>` decision tree built in the same editor. This crate is the
//! third trader-configurable rule engine, architecturally identical to
//! [`celnet-risk-routing`](celnet_risk_routing) (which routes a *booked* fill to a
//! risk book) and `celnet-hedge-routing` (which resolves a book's warehoused risk to
//! an exit action). It is the future home for credit checks + quote validations at
//! the acceptance point. It has **no** server, proto, or wire dependency; only
//! `serde` for persisting a graph.
//!
//! # Model
//!
//! - [`AcceptanceContext`] — a flat snapshot of one inbound *lift* at the acceptance
//!   point (counterparty, notional, tenor, instrument, side, captured edge, quote
//!   age, asset, desk), the rule-evaluation input. The acceptance analogue of
//!   `celnet-risk-routing`'s per-fill `RoutingContext`.
//! - [`AcceptanceField`] — the lift fields a rule can branch on. Its condition
//!   vocabulary — the operator ([`RouteOp`]) and literal ([`RouteValue`]) — is
//!   **reused verbatim from `celnet-risk-routing`**: one source of truth for the
//!   total, never-panicking `field op value` semantics.
//! - [`AcceptanceAction`] — the terminal leaves: the acceptance vocabulary the graph
//!   resolves to (`ACCEPT`, `REJECT`, `HOLD_FOR_REVIEW`).
//! - [`AcceptanceGraph`] — a directed graph of [`AcceptanceNode`]s
//!   ([`AcceptanceNode::Condition`] internal nodes with `yes`/`no` edges;
//!   [`AcceptanceNode::Decision`] terminal leaves), entered at a single node per
//!   evaluation.
//! - [`AcceptanceEngine::evaluate`] — a pure, bounded walk to the first acceptance
//!   decision, yielding an [`AcceptanceOutcome`] (the [`AcceptanceDecision`] + the
//!   matched leaf id).
//! - [`AcceptanceGraph::validate`] — proves a graph is well-formed (acyclic, every
//!   path terminates at a decision, every condition type-consistent), collecting
//!   **all** defects at once. A `Decision` leaf carries no external target, so — unlike
//!   the hedge graph — validation needs no registries.
//! - [`default_accept_all_graph`] — the identity policy (a single `ACCEPT` leaf) so a
//!   pristine store leaves existing behaviour UNCHANGED until a trader writes rules.
//!
//! # Purity
//!
//! Deterministic and allocation-light: no I/O, no locks, no server or wire coupling.
//! The independent oracle for its correctness is a hand-written truth table of
//! `(lift, policy) → decision` (see the unit tests), never the engine re-run against
//! itself.

mod context;
mod engine;
mod field;
mod graph;

pub use context::AcceptanceContext;
pub use engine::{AcceptanceEngine, AcceptanceOutcome};
pub use field::AcceptanceField;
pub use graph::{
    AcceptanceAction, AcceptanceDecision, AcceptanceError, AcceptanceGraph, AcceptanceNode, NodeId,
    default_accept_all_graph,
};

// Re-export the condition vocabulary reused verbatim from risk routing, so a
// consumer builds an acceptance condition from one import surface without also
// depending on `celnet-risk-routing` directly.
pub use celnet_risk_routing::{CtxValue, FieldKind, RouteOp, RouteValue};
