//! Celnet **auto-hedging / risk-internalisation** — the pure, server-free
//! decision-graph engine that resolves a book's *warehoused-risk state* to an
//! **exit action** (warehouse it, cross it internally, skew to attract offset,
//! back-to-back it to the street, RFQ it out, split, or escalate).
//!
//! A market-maker who captures client flow holds inventory and faces one
//! continuous choice on every unit of that risk: **warehouse it** (hold, capture
//! the spread, let opposing flow / mean-reversion flatten it) or **externalise
//! it** (pay the street to shed it now). The desk's policy — *"internalise up to
//! a threshold, then back-to-back"* — is expressed exactly like a risk-routing
//! graph: an `IF <risk-field> <op> <value> THEN <exit-action>` decision tree the
//! trader builds in the same editor. This crate is workstream §10.1 of
//! `docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` — the pure foundation
//! the server control loop, proto/WS, and GUI layers build on. It has **no**
//! server, proto, or wire dependency.
//!
//! # Model
//!
//! - [`HedgeContext`] — a flat snapshot of one `(book × instrument)`
//!   **risk state** (net DV01 / greeks, inventory sign, threshold band,
//!   flow toxicity, internal-offset availability), the rule-evaluation input.
//!   The auto-hedge analogue of `celnet-risk-routing`'s per-fill `RoutingContext`.
//! - [`HedgeField`] — the risk-state fields a rule can branch on. Its condition
//!   vocabulary — the operator ([`RouteOp`]) and literal ([`RouteValue`]) — is
//!   **reused verbatim from `celnet-risk-routing`**: one source of truth for the
//!   total, never-panicking `field op value` semantics.
//! - [`ExitAction`] — the terminal leaves: the exit vocabulary the graph resolves
//!   to (`WAREHOUSE`, `CROSS_INTERNAL`, `SKEW`, `SUBMIT_MARKET_ORDER`, `RFQ_OUT`,
//!   `SPLIT`, `ESCALATE`).
//! - [`HedgeLpPanel`] — the standing include/exclude liquidity-provider selection an
//!   external exit action fans to, and its pure resolver
//!   ([`HedgeLpPanel::effective_lps`]) that turns include/exclude into the **effective
//!   LP set** against the live known-LP registry (generalising the per-rule `RfqOut`
//!   include list with exclude semantics — the LP-panel config the tutorial needs).
//! - [`HedgeGraph`] — a directed graph of [`HedgeNode`]s ([`HedgeNode::Condition`]
//!   internal nodes with `yes`/`no` edges; [`HedgeNode::Action`] terminal leaves),
//!   entered at a single node per evaluation.
//! - [`HedgeRouter::resolve`] — a pure, bounded walk to the first exit action.
//! - [`HedgeGraph::validate`] — proves a graph is well-formed (acyclic, every
//!   path terminates at a valid action whose venue/LP/instrument targets exist,
//!   every condition type-consistent), collecting **all** defects at once.
//!
//! # The threshold / warehouse band ([`band`])
//!
//! The configurable "100" is a **soft, banded risk budget** ([`WarehouseThreshold`])
//! reusing the `celnet-limits` vocabulary (`LimitMetric` / `RagStatus` /
//! `LimitSpec` soft bands): green = warehouse, amber = skew to attract offset,
//! red = hedge the overflow. The default hedge size is the **overflow to the band
//! edge** (a transaction-cost-optimal band policy, Whalley–Wilmott 1997), clipped
//! to `[min_clip, max_clip]` (Zakamouline 2006 minimum-ticket), with an optional
//! utilisation-ramped fraction (Barzykin–Bergault–Guéant 2021). The
//! internalise-then-hedge netting decomposition ([`netting_split`]) nets the
//! overflow internally first, externalising only the residual (Butz–Oomen 2019).
//!
//! # Purity
//!
//! Deterministic and allocation-light: no I/O, no locks, no server or wire
//! coupling. The independent oracle for its correctness is a hand-written truth
//! table of `(risk-state, policy) → action` and `(risk, threshold) → sizing`
//! (see `tests/`), never the engine re-run against itself.

mod band;
mod context;
mod field;
mod graph;
mod lp_panel;
mod router;

pub use band::{HedgeSizing, NettingSplit, WarehouseThreshold, netting_split};
pub use context::HedgeContext;
pub use field::{HedgeField, HedgeFieldKind};
pub use graph::{ExecStyle, ExitAction, HedgeError, HedgeGraph, HedgeNode, HedgeSize, NodeId};
pub use lp_panel::{HedgeLpPanel, LpPanelError};
pub use router::HedgeRouter;

// Re-export the condition vocabulary reused verbatim from risk routing, so a
// consumer builds a hedge condition from one import surface without also
// depending on `celnet-risk-routing` directly.
pub use celnet_risk_routing::{CtxValue, FieldKind, RouteOp, RouteValue};

// Re-export the limits band vocabulary the warehouse threshold is built on.
pub use celnet_limits::{Enforcement, LimitMetric, LimitSpec, RagStatus};
