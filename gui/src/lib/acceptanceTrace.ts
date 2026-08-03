/**
 * A pure, dependency-free TypeScript port of the acceptance-graph ENGINE
 * (`crates/celnet-acceptance`: the `RouteOp::eval` reuse, the lift projection, the
 * bounded acyclic walk, and `AcceptanceGraph::validate`). It powers two client
 * affordances with SERVER PARITY:
 *
 *   1. {@link traceAcceptanceGraph} — walk a graph against a sample LIFT and return the
 *      exact node path + landing decision (the "what would fire" panel).
 *   2. {@link validateAcceptanceGraph} — collect every well-formedness defect the
 *      server would, so Save is disabled until the graph is one the server accepts.
 *
 * The operator evaluation reuses {@link evalOp} verbatim from `lib/routeTrace.ts` (the
 * shared `RouteOp::eval` port) — an acceptance condition tests exactly like a routing
 * one; only the fields it projects and the leaf it lands on differ.
 */
import type {
  AcceptanceAction,
  AcceptanceCondition,
  AcceptanceField,
  AcceptanceGraph,
  AcceptanceNode,
  RouteOp,
  RouteValue,
} from "../data/contract";
import { acceptanceFieldKind, opValidForAcceptanceField } from "./acceptanceFields";
import { type CtxValue, evalOp } from "./routeTrace";

/**
 * A composed sample LIFT — the client mirror of `celnet_acceptance`'s lift context.
 * Numeric fields carry their natural units; string/enum fields carry their literal.
 */
export interface AcceptanceSampleLift {
  counterparty: string;
  side: string;
  assetClass: string;
  desk: string;
  instrumentSymbol: string;
  notionalUsd: number;
  tenorYears: number;
  edgeBps: number;
  quoteAgeMs: number;
}

/** A blank lift (all-empty / all-zero) — the lift-context default mirror. */
export function blankAcceptanceLift(): AcceptanceSampleLift {
  return {
    counterparty: "",
    side: "",
    assetClass: "",
    desk: "",
    instrumentSymbol: "",
    notionalUsd: 0,
    tenorYears: 0,
    edgeBps: 0,
    quoteAgeMs: 0,
  };
}

/** Project a lift onto one field (the lift-context `get` mirror). */
export function acceptanceFieldValue(lift: AcceptanceSampleLift, field: AcceptanceField): CtxValue {
  switch (field) {
    case "counterparty":
      return { kind: "text", text: lift.counterparty };
    case "side":
      return { kind: "text", text: lift.side };
    case "asset_class":
      return { kind: "text", text: lift.assetClass };
    case "desk":
      return { kind: "text", text: lift.desk };
    case "instrument_symbol":
      return { kind: "text", text: lift.instrumentSymbol };
    case "notional_usd":
      return { kind: "num", num: lift.notionalUsd };
    case "tenor_years":
      return { kind: "num", num: lift.tenorYears };
    case "edge_bps":
      return { kind: "num", num: lift.edgeBps };
    case "quote_age_ms":
      return { kind: "num", num: lift.quoteAgeMs };
  }
}

/** Evaluate one acceptance condition against a lift; a `null` value ⇒ the false branch. */
export function evalAcceptanceCondition(
  cond: AcceptanceCondition,
  lift: AcceptanceSampleLift,
): boolean {
  if (cond.value === null) return false;
  return evalOp(cond.op, acceptanceFieldValue(lift, cond.field), cond.value);
}

/** How a graph walk ended. */
export type AcceptanceTraceOutcome = "decision" | "cycle" | "node_not_found";

/** The result of walking an acceptance graph against a sample lift. */
export interface AcceptanceTraceResult {
  /** The node ids visited, in order (the highlighted path). */
  path: number[];
  /** The landing decision when the walk reached a decision leaf, else `null`. */
  landedAction: AcceptanceAction | null;
  /** How the walk terminated (`decision` on success). */
  outcome: AcceptanceTraceOutcome;
}

/**
 * Walk `graph` from its entry against `lift`, returning the visited node path and the
 * landing decision. Bounded by `nodes.length + 1` steps so a (still-being-edited) cycle
 * ends as `cycle`.
 */
export function traceAcceptanceGraph(
  graph: AcceptanceGraph,
  lift: AcceptanceSampleLift,
): AcceptanceTraceResult {
  const byId = new Map<number, AcceptanceNode>(graph.nodes.map((n) => [n.id, n]));
  const path: number[] = [];
  const cap = graph.nodes.length + 1;
  let current = graph.entry;

  for (let step = 0; step < cap; step += 1) {
    const node = byId.get(current);
    if (!node) return { path, landedAction: null, outcome: "node_not_found" };
    path.push(current);
    if (node.kind === "decision") {
      return { path, landedAction: node.action, outcome: "decision" };
    }
    current = evalAcceptanceCondition(node.condition, lift)
      ? node.condition.onTrue
      : node.condition.onFalse;
  }
  return { path, landedAction: null, outcome: "cycle" };
}

/** A single well-formedness defect (mirrors one server validate variant). */
export interface AcceptanceValidationIssue {
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
    | "empty_graph";
  /** A trader-readable description. */
  message: string;
}

/** Whether a condition's value VARIANT is consistent with its op. */
function valueMatchesOp(field: AcceptanceField, op: RouteOp, value: RouteValue): boolean {
  const kind = acceptanceFieldKind(field);
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
function hasReachableCycle(graph: AcceptanceGraph): boolean {
  const byId = new Map<number, AcceptanceNode>(graph.nodes.map((n) => [n.id, n]));
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

/**
 * Collect EVERY well-formedness defect of `graph` — the client mirror of the server's
 * acceptance-graph validate. An empty result means the server will accept the graph.
 * Decision leaves carry no target constraint (an empty `reason` is accepted server-side).
 */
export function validateAcceptanceGraph(graph: AcceptanceGraph): AcceptanceValidationIssue[] {
  const issues: AcceptanceValidationIssue[] = [];
  const byId = new Map<number, AcceptanceNode>(graph.nodes.map((n) => [n.id, n]));

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
    if (node.kind === "decision") continue;
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
    } else if (!opValidForAcceptanceField(c.field, c.op)) {
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
  }

  if (hasReachableCycle(graph)) {
    issues.push({ node: null, code: "cycle", message: "A cycle is reachable from the entry." });
  }

  return issues;
}
