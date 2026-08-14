/**
 * A pure, dependency-free TypeScript port of the hedge-routing ENGINE
 * (`crates/celnet-hedge-routing`: the `RouteOp::eval` reuse, the `HedgeContext`
 * projection, the bounded acyclic `HedgeRouter::resolve` walk, and `HedgeGraph::
 * validate`). It powers two client affordances with SERVER PARITY:
 *
 *   1. {@link traceHedgeGraph} — walk a graph against a sample RISK STATE and return
 *      the exact node path + landing exit action (the "what would fire" panel).
 *   2. {@link validateHedgeGraph} — collect every well-formedness defect the server's
 *      `HedgeGraph::validate` would, so Save is disabled until the graph is one the
 *      server will accept.
 *
 * The operator evaluation reuses {@link evalOp} verbatim from `lib/routeTrace.ts`
 * (the shared `RouteOp::eval` port) — a hedge condition tests exactly like a routing
 * one; only the fields it projects and the leaf it lands on differ.
 */
import type {
  ExitAction,
  HedgeCondition,
  HedgeField,
  HedgeGraph,
  HedgeNode,
  RouteOp,
  RouteValue,
} from "../data/contract";
import {
  hedgeFieldKind,
  hedgeFieldSpec,
  hedgeFieldUnprovidedReason,
  opValidForHedgeField,
} from "./hedgeFields";
import { type CtxValue, evalOp } from "./routeTrace";

/**
 * A composed sample RISK STATE — the client mirror of `celnet_hedge_routing
 * ::HedgeContext`. Numeric fields carry their natural units; string/enum fields
 * carry their literal (`breached` projects to the text "true"/"false").
 */
export interface HedgeSampleState {
  instrumentId: string;
  ccy: string;
  product: string;
  book: string;
  desk: string;
  counterparty: string;
  netDv01: number;
  netNotional: number;
  netVega: number;
  netGamma: number;
  inventorySign: number;
  threshold: number;
  utilization: number;
  overflow: number;
  breached: boolean;
  counterpartyToxicity: number;
  inventoryAgeSecs: number;
  internalOffsetAvailable: number;
  hedgeCostBp: number;
}

/** A blank risk state (all-empty / all-zero) — the `HedgeContext::default` mirror. */
export function blankHedgeState(): HedgeSampleState {
  return {
    instrumentId: "",
    ccy: "",
    product: "",
    book: "",
    desk: "",
    counterparty: "",
    netDv01: 0,
    netNotional: 0,
    netVega: 0,
    netGamma: 0,
    inventorySign: 0,
    threshold: 0,
    utilization: 0,
    overflow: 0,
    breached: false,
    counterpartyToxicity: 0,
    inventoryAgeSecs: 0,
    internalOffsetAvailable: 0,
    hedgeCostBp: 0,
  };
}

/** Project a risk state onto one field (the `HedgeContext::get` mirror). */
export function hedgeFieldValue(state: HedgeSampleState, field: HedgeField): CtxValue {
  switch (field) {
    case "instrument_id":
      return { kind: "text", text: state.instrumentId };
    case "ccy":
      return { kind: "text", text: state.ccy };
    case "product":
      return { kind: "text", text: state.product };
    case "book":
      return { kind: "text", text: state.book };
    case "desk":
      return { kind: "text", text: state.desk };
    case "counterparty":
      return { kind: "text", text: state.counterparty };
    case "breached":
      return { kind: "text", text: state.breached ? "true" : "false" };
    case "net_dv01":
      return { kind: "num", num: state.netDv01 };
    case "net_notional":
      return { kind: "num", num: state.netNotional };
    case "net_vega":
      return { kind: "num", num: state.netVega };
    case "net_gamma":
      return { kind: "num", num: state.netGamma };
    case "inventory_sign":
      return { kind: "num", num: state.inventorySign };
    case "threshold":
      return { kind: "num", num: state.threshold };
    case "utilization":
      return { kind: "num", num: state.utilization };
    case "overflow":
      return { kind: "num", num: state.overflow };
    case "counterparty_toxicity":
      return { kind: "num", num: state.counterpartyToxicity };
    case "inventory_age_secs":
      return { kind: "num", num: state.inventoryAgeSecs };
    case "internal_offset_available":
      return { kind: "num", num: state.internalOffsetAvailable };
    case "hedge_cost_bp":
      return { kind: "num", num: state.hedgeCostBp };
  }
}

/** Evaluate one hedge condition against a risk state; a `null` value ⇒ the false branch. */
export function evalHedgeCondition(cond: HedgeCondition, state: HedgeSampleState): boolean {
  if (cond.value === null) return false;
  return evalOp(cond.op, hedgeFieldValue(state, cond.field), cond.value);
}

/** How a graph walk ended. */
export type HedgeTraceOutcome = "action" | "cycle" | "node_not_found";

/** The result of walking a hedge graph against a sample risk state. */
export interface HedgeTraceResult {
  /** The node ids visited, in order (the highlighted path). */
  path: number[];
  /** The landing exit action when the walk reached an action leaf, else `null`. */
  landedAction: ExitAction | null;
  /** How the walk terminated (`action` on success). */
  outcome: HedgeTraceOutcome;
}

/**
 * Walk `graph` from its entry against `state`, returning the visited node path and
 * the landing exit action — the `HedgeRouter::resolve` mirror. Bounded by
 * `nodes.length + 1` steps so a (still-being-edited) cycle ends as `cycle`.
 */
export function traceHedgeGraph(graph: HedgeGraph, state: HedgeSampleState): HedgeTraceResult {
  const byId = new Map<number, HedgeNode>(graph.nodes.map((n) => [n.id, n]));
  const path: number[] = [];
  const cap = graph.nodes.length + 1;
  let current = graph.entry;

  for (let step = 0; step < cap; step += 1) {
    const node = byId.get(current);
    if (!node) return { path, landedAction: null, outcome: "node_not_found" };
    path.push(current);
    if (node.kind === "action") {
      return { path, landedAction: node.action, outcome: "action" };
    }
    current = evalHedgeCondition(node.condition, state) ? node.condition.onTrue : node.condition.onFalse;
  }
  return { path, landedAction: null, outcome: "cycle" };
}

/** A single well-formedness defect (mirrors one `HedgeError` variant). */
export interface HedgeValidationIssue {
  /** The offending node id, or `null` for a whole-graph defect. */
  node: number | null;
  /** A stable defect code. */
  code:
    | "missing_entry"
    | "dangling_edge"
    | "cycle"
    | "op_not_valid_for_field"
    | "value_type_mismatch"
    | "value_unset"
    | "action_target_missing"
    | "unprovided_field"
    | "empty_graph";
  /** A trader-readable description. */
  message: string;
}

/** Whether a condition's value VARIANT is consistent with its op. */
function valueMatchesOp(field: HedgeField, op: RouteOp, value: RouteValue): boolean {
  const kind = hedgeFieldKind(field);
  switch (op) {
    case "eq":
    case "ne":
      return kind === "numeric" ? value.kind === "num" : value.kind === "text";
    case "gt":
    case "ge":
    case "lt":
    case "le":
      return value.kind === "num";
    case "contains":
      return value.kind === "text";
    case "in":
      return value.kind === "list";
    case "between":
      return value.kind === "range";
  }
}

/** Three-colour DFS: is a cycle reachable from `entry`? */
function hasReachableCycle(graph: HedgeGraph): boolean {
  const byId = new Map<number, HedgeNode>(graph.nodes.map((n) => [n.id, n]));
  if (!byId.has(graph.entry)) return false;
  const successors = (id: number): number[] => {
    const n = byId.get(id);
    return n && n.kind === "condition" ? [n.condition.onTrue, n.condition.onFalse] : [];
  };
  const GRAY = 1;
  const BLACK = 2;
  const color = new Map<number, number>();
  const stack: { node: number; rest: number[] }[] = [];
  color.set(graph.entry, GRAY);
  stack.push({ node: graph.entry, rest: successors(graph.entry) });

  while (stack.length > 0) {
    const frame = stack[stack.length - 1];
    if (!frame) break;
    const next = frame.rest.pop();
    if (next === undefined) {
      color.set(frame.node, BLACK);
      stack.pop();
      continue;
    }
    if (!byId.has(next)) continue;
    const c = color.get(next);
    if (c === GRAY) return true;
    if (c === BLACK) continue;
    color.set(next, GRAY);
    stack.push({ node: next, rest: successors(next) });
  }
  return false;
}

/** Whether an action leaf names every target it needs (mirrors the server action check). */
function actionTargetIssue(action: ExitAction): string | null {
  switch (action.kind) {
    case "cross_internal":
      return action.instrument.trim().length === 0
        ? "Cross-internal action has no aggregation instrument selected."
        : null;
    case "rfq_out":
      return action.lps.length === 0 ? "RFQ-out action names no LPs to fan the request to." : null;
    default:
      if (action.size.kind === "fixed" && !(action.size.fixed > 0)) {
        return "A fixed hedge size must be greater than zero.";
      }
      return null;
  }
}

/**
 * Collect EVERY well-formedness defect of `graph` — the client mirror of
 * `HedgeGraph::validate`. An empty result means the server will accept the graph.
 *
 * "The server will accept it" is a claim this function has to keep EXACTLY, not
 * approximately: every check the server makes must have a counterpart here, or a trader
 * builds a graph that passes locally and is refused on save with no way to see why. The
 * `unprovided_field` check is one such counterpart — the server refuses a condition on a
 * field nothing populates (`HedgeError::UnprovidedField`), so this must too, carrying the
 * SAME reason text (mirrored in `hedgeFields.ts`, drift-guarded against the Rust source by
 * `test/hedgeFieldProviderParity.test.ts`).
 */
export function validateHedgeGraph(graph: HedgeGraph): HedgeValidationIssue[] {
  const issues: HedgeValidationIssue[] = [];
  const byId = new Map<number, HedgeNode>(graph.nodes.map((n) => [n.id, n]));

  if (graph.nodes.length === 0) {
    issues.push({ node: null, code: "empty_graph", message: "The policy has no nodes." });
    return issues;
  }

  if (!byId.has(graph.entry)) {
    issues.push({
      node: null,
      code: "missing_entry",
      message: `The entry node #${graph.entry} does not exist.`,
    });
  }

  for (const node of graph.nodes) {
    if (node.kind === "action") {
      const issue = actionTargetIssue(node.action);
      if (issue !== null) {
        issues.push({ node: node.id, code: "action_target_missing", message: issue });
      }
      continue;
    }
    const c = node.condition;
    for (const to of [c.onTrue, c.onFalse]) {
      if (!byId.has(to)) {
        issues.push({
          node: node.id,
          code: "dangling_edge",
          message: `A branch points to node #${to}, which does not exist.`,
        });
      }
    }
    if (c.value === null) {
      issues.push({ node: node.id, code: "value_unset", message: "This condition has no value set." });
    } else if (!opValidForHedgeField(c.field, c.op)) {
      issues.push({
        node: node.id,
        code: "op_not_valid_for_field",
        message: `Operator is not valid for the ${c.field} field.`,
      });
    } else if (!valueMatchesOp(c.field, c.op, c.value)) {
      issues.push({
        node: node.id,
        code: "value_type_mismatch",
        message: "The value type does not match the operator.",
      });
    }
    // A well-typed condition on a field nothing populates is still dead: its operand is the
    // context default forever. Checked INDEPENDENTLY of the operator/value defects above
    // (not chained onto the `else if`) so the author sees every reason at once — exactly as
    // the server's `HedgeGraph::validate` accumulates it.
    const unprovided = hedgeFieldUnprovidedReason(c.field);
    if (unprovided !== null) {
      issues.push({
        node: node.id,
        code: "unprovided_field",
        message:
          `${hedgeFieldSpec(c.field).label} has no production source, so this rule could ` +
          `never fire (${unprovided}).`,
      });
    }
  }

  if (hasReachableCycle(graph)) {
    issues.push({ node: null, code: "cycle", message: "A cycle is reachable from the entry." });
  }

  return issues;
}
