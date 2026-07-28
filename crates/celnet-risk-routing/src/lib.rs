//! Celnet **risk routing** — the pure, server-free decision-graph engine that
//! routes an accepted fill's *risk* to a target **risk book** (portfolio).
//!
//! When an order/RFQ is filled, the resulting risk must land in a trader-defined
//! book so that limits, greeks, and PnL are managed per book. Traders express
//! *which* book via a decision tree of `IF <field> <op> <value> THEN <book>`
//! rules. This crate is workstream §8.1 of `docs/FI-RISK-ROUTING-REQUIREMENTS.md`
//! — the pure foundation the server, proto/WS, and GUI layers build on. It has
//! **no** server, proto, or wire dependency; only `serde` for persisting a graph.
//!
//! # Model
//!
//! - [`RoutingContext`] — a flat snapshot of one fill's routable field values
//!   (the rule-evaluation input), decoupled from the server's booked-position
//!   type on purpose.
//! - [`RouteField`] / [`RouteOp`] / [`RouteValue`] — the typed rule vocabulary. A
//!   [`FieldKind`] classifies each field, and [`RouteOp::valid_for`] pins the
//!   legal operator/value matrix so a malformed rule (`side > 5`) is rejected.
//! - [`RiskRoutingGraph`] — a directed graph of [`RoutingNode`]s ([`RoutingNode::Condition`]
//!   internal nodes with `yes`/`no` edges; [`RoutingNode::Book`] terminal leaves),
//!   entered at a single node per fill.
//! - [`RiskRouter::route`] — a pure, bounded walk to the landing book.
//! - [`RiskRoutingGraph::validate`] — proves a graph is well-formed (acyclic,
//!   every path terminates at a known book, every condition type-consistent),
//!   collecting **all** defects at once. A validated graph makes routing total:
//!   every context resolves to `Ok(book)`, never panicking.
//!
//! # Purity
//!
//! Deterministic and allocation-light: no I/O, no locks, no server or wire
//! coupling. The independent oracle for its correctness is a hand-written truth
//! table of fills → books (see `tests/`), never the engine re-run against itself.

mod context;
mod field;
mod graph;
mod router;

pub use context::RoutingContext;
pub use field::{CtxValue, FieldKind, RouteField, RouteOp, RouteValue};
pub use graph::{NodeId, RiskRoutingGraph, RouteError, RoutingNode};
pub use router::RiskRouter;
