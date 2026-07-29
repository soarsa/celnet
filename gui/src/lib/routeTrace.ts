/**
 * A pure, dependency-free TypeScript port of the Rust risk-routing ENGINE
 * (`crates/celnet-risk-routing`: `field.rs::RouteOp::eval`, `context.rs`,
 * `router.rs::RiskRouter::route`, `graph.rs::validate`). It powers two client-side
 * affordances with SERVER PARITY:
 *
 *   1. {@link traceGraph} — walk a graph against a sample fill and return the exact
 *      node path + landing book, so the canvas can light up the traversed branch.
 *      Same operator semantics + same bounded acyclic walk as `RiskRouter::route`.
 *   2. {@link validateGraph} — collect every well-formedness defect the server's
 *      `RiskRoutingGraph::validate` would, so Save is disabled until the graph is
 *      one the server will accept (no dangling edge, no cycle, every path ends at a
 *      known book, every condition op/value consistent with its field).
 *
 * Parity is not asserted — it is TESTED against the same truth table the Rust
 * oracle uses (see `gui/test/routeTrace.test.ts`). Every evaluation is TOTAL: a
 * type-mismatched or unset condition yields `false` (the false branch) and never
 * throws, exactly like the hot path in `RouteOp::eval`.
 */
import type {
  RiskRoutingGraph,
  RouteCondition,
  RouteField,
  RouteOp,
  RouteValue,
  RoutingNode,
} from "../data/contract";
import { fieldKind, opValidForField } from "./routeFields";

/**
 * A composed sample fill — the client mirror of `celnet_risk_routing
 * ::RoutingContext`. Numeric fields carry their natural units (notional the
 * absolute base amount, tenor in years); string/enum fields carry their literal.
 */
export interface SampleFill {
  instrumentId: string;
  ccy: string;
  product: string;
  side: string;
  notional: number;
  tenor: number;
  strike: number;
  counterparty: string;
  user: string;
  desk: string;
  price: number;
}

/** A blank fill (all-empty / all-zero) — the `RoutingContext::default` mirror. */
export function blankFill(): SampleFill {
  return {
    instrumentId: "",
    ccy: "",
    product: "",
    side: "",
    notional: 0,
    tenor: 0,
    strike: 0,
    counterparty: "",
    user: "",
    desk: "",
    price: 0,
  };
}

/** The left-hand value a field projects — numeric fields a number, else text. */
export type CtxValue = { kind: "num"; num: number } | { kind: "text"; text: string };

/** Project a fill onto one field (the `RoutingContext::get` mirror). */
export function fieldValue(fill: SampleFill, field: RouteField): CtxValue {
  switch (field) {
    case "notional":
      return { kind: "num", num: fill.notional };
    case "tenor":
      return { kind: "num", num: fill.tenor };
    case "strike":
      return { kind: "num", num: fill.strike };
    case "price":
      return { kind: "num", num: fill.price };
    case "instrument_id":
      return { kind: "text", text: fill.instrumentId };
    case "ccy":
      return { kind: "text", text: fill.ccy };
    case "product":
      return { kind: "text", text: fill.product };
    case "side":
      return { kind: "text", text: fill.side };
    case "counterparty":
      return { kind: "text", text: fill.counterparty };
    case "user":
      return { kind: "text", text: fill.user };
    case "desk":
      return { kind: "text", text: fill.desk };
  }
}

/** Parse a list entry to a finite number the Rust `str::parse::<f64>` way (empty / NaN ⇒ null). */
function parseNumeric(entry: string): number | null {
  if (entry.length === 0) return null;
  const n = Number(entry);
  return Number.isFinite(n) ? n : null;
}

/**
 * Evaluate `ctx <op> value`, TOTALLY — every type-mismatched combination returns
 * `false` and never throws (the byte-faithful port of `RouteOp::eval`).
 */
export function evalOp(op: RouteOp, ctx: CtxValue, value: RouteValue): boolean {
  switch (op) {
    case "eq":
      if (ctx.kind === "num" && value.kind === "num") return ctx.num === value.num;
      if (ctx.kind === "text" && value.kind === "text") return ctx.text === value.text;
      return false;
    case "ne":
      if (ctx.kind === "num" && value.kind === "num") return ctx.num !== value.num;
      if (ctx.kind === "text" && value.kind === "text") return ctx.text !== value.text;
      return false;
    case "gt":
      return ctx.kind === "num" && value.kind === "num" ? ctx.num > value.num : false;
    case "ge":
      return ctx.kind === "num" && value.kind === "num" ? ctx.num >= value.num : false;
    case "lt":
      return ctx.kind === "num" && value.kind === "num" ? ctx.num < value.num : false;
    case "le":
      return ctx.kind === "num" && value.kind === "num" ? ctx.num <= value.num : false;
    case "contains":
      return ctx.kind === "text" && value.kind === "text" ? ctx.text.includes(value.text) : false;
    case "in":
      if (ctx.kind === "text" && value.kind === "list") {
        return value.values.some((it) => it === ctx.text);
      }
      if (ctx.kind === "num" && value.kind === "list") {
        return value.values.some((it) => {
          const n = parseNumeric(it);
          return n !== null && n === ctx.num;
        });
      }
      return false;
    case "between":
      return ctx.kind === "num" && value.kind === "range"
        ? value.lo <= ctx.num && ctx.num <= value.hi
        : false;
  }
}

/** Evaluate one condition against a fill; a `null` value is unset ⇒ the false branch. */
export function evalCondition(cond: RouteCondition, fill: SampleFill): boolean {
  if (cond.value === null) return false;
  return evalOp(cond.op, fieldValue(fill, cond.field), cond.value);
}

/** How a graph walk ended. */
export type TraceOutcome = "book" | "cycle" | "node_not_found";

/** The result of walking a graph against a sample fill. */
export interface TraceResult {
  /** The node ids visited, in order (the highlighted path on the canvas). */
  path: number[];
  /** The landing risk-book id when the walk reached a book leaf, else `null`. */
  landedBook: string | null;
  /** How the walk terminated (`book` on success). */
  outcome: TraceOutcome;
}

/**
 * Walk `graph` from its entry against `fill`, returning the visited node path and
 * the landing book — the `RiskRouter::route` mirror, with the path collected for
 * the UI. Bounded by `nodes.length + 1` steps so a (still-being-edited) cycle
 * ends as `cycle`, never a hang.
 */
export function traceGraph(graph: RiskRoutingGraph, fill: SampleFill): TraceResult {
  const byId = new Map<number, RoutingNode>(graph.nodes.map((n) => [n.id, n]));
  const path: number[] = [];
  const cap = graph.nodes.length + 1;
  let current = graph.entry;

  for (let step = 0; step < cap; step += 1) {
    const node = byId.get(current);
    if (!node) return { path, landedBook: null, outcome: "node_not_found" };
    path.push(current);
    if (node.kind === "book") {
      return { path, landedBook: node.bookId, outcome: "book" };
    }
    current = evalCondition(node.condition, fill) ? node.condition.onTrue : node.condition.onFalse;
  }
  return { path, landedBook: null, outcome: "cycle" };
}

/** A single well-formedness defect (mirrors one `RouteError` variant). */
export interface ValidationIssue {
  /** The offending node id, or `null` for a whole-graph defect (missing entry / cycle). */
  node: number | null;
  /** A stable defect code (mirrors the `RouteError` variant name, snake-cased). */
  code:
    | "missing_entry"
    | "dangling_edge"
    | "cycle"
    | "unknown_book"
    | "op_not_valid_for_field"
    | "value_type_mismatch"
    | "value_unset"
    | "empty_graph";
  /** A trader-readable description. */
  message: string;
}

/** Whether a condition's value VARIANT is consistent with its op (mirrors `value_matches_op`). */
function valueMatchesOp(field: RouteField, op: RouteOp, value: RouteValue): boolean {
  const kind = fieldKind(field);
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

/** Three-colour DFS: is a cycle reachable from `entry`? (mirror of `has_reachable_cycle`). */
function hasReachableCycle(graph: RiskRoutingGraph): boolean {
  const byId = new Map<number, RoutingNode>(graph.nodes.map((n) => [n.id, n]));
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
    if (!byId.has(next)) continue; // dangling — reported elsewhere
    const c = color.get(next);
    if (c === GRAY) return true; // back edge → cycle
    if (c === BLACK) continue;
    color.set(next, GRAY);
    stack.push({ node: next, rest: successors(next) });
  }
  return false;
}

/**
 * Collect EVERY well-formedness defect of `graph` against the set of enabled book
 * ids — the client mirror of `RiskRoutingGraph::validate`. An empty result means
 * the server will accept the graph. Save gates on this being empty.
 */
export function validateGraph(
  graph: RiskRoutingGraph,
  knownBookIds: ReadonlySet<string>,
): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  const byId = new Map<number, RoutingNode>(graph.nodes.map((n) => [n.id, n]));

  if (graph.nodes.length === 0) {
    issues.push({ node: null, code: "empty_graph", message: "The graph has no nodes." });
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
    if (node.kind === "book") {
      if (node.bookId.length === 0 || !knownBookIds.has(node.bookId)) {
        issues.push({
          node: node.id,
          code: "unknown_book",
          message:
            node.bookId.length === 0
              ? "A book leaf has no target book selected."
              : `Book leaf targets unknown / disabled book “${node.bookId}”.`,
        });
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
      issues.push({
        node: node.id,
        code: "value_unset",
        message: "This condition has no value set.",
      });
    } else if (!opValidForField(c.field, c.op)) {
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
