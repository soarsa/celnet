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

// --- rule enumeration ------------------------------------------------------

/**
 * One condition met while walking a root-to-leaf rule, with the branch SENSE that
 * was taken: `onTrue` ⇒ the condition held; `onFalse` ⇒ it did NOT (the human rule
 * reads that leg negated).
 */
export interface PathCondition {
  field: RouteField;
  op: RouteOp;
  value: RouteValue | null;
  branch: "onTrue" | "onFalse";
}

/**
 * One enumerated root-to-leaf rule of a {@link RiskRoutingGraph}: the ANDed
 * conditions along the walk and the book it lands in. `valid` is false when the
 * path cannot terminate at a known enabled book (dangling edge, cycle, or an
 * unknown / unset destination), with `issue` naming the reason.
 */
export interface EnumeratedPath {
  /** The node ids visited on this path, in order (for the canvas highlight). */
  nodes: number[];
  /** The conditions ANDed along the path, in evaluation order. */
  conditions: PathCondition[];
  /** The landing book id, or `null` when the path does not reach a book leaf. */
  bookId: string | null;
  /** Whether this rule terminates at a known enabled book. */
  valid: boolean;
  /** When `valid` is false, a trader-readable reason. */
  issue?: string;
}

/** A hard cap on enumerated paths so a wide (still-being-edited) graph never blows up. */
const MAX_ENUMERATED_PATHS = 256;

/**
 * Enumerate every root-to-leaf path of `graph` that TERMINATES AT A BOOK LEAF, in
 * the exact order they are EVALUATED — a DFS from {@link RiskRoutingGraph.entry}
 * following `on_true` before `on_false`, so the first path is the first the router
 * tests. A branch that dangles off a missing node or loops back on itself never
 * reaches a book and is therefore NOT a path — it is a graph DEFECT surfaced by
 * {@link validateGraph}, not something rendered as a rule. A book leaf is still
 * returned when its destination book is unset or unknown/disabled (flagged
 * `valid:false` with an `issue`) — it IS a route, just one to a bad destination.
 *
 * The walk is cycle-safe (a per-path visited set caps depth at the node count) and
 * globally bounded by {@link MAX_ENUMERATED_PATHS}. Prefer {@link enumerateRules},
 * which groups these paths by destination book into one rule per book.
 */
export function enumeratePaths(
  graph: RiskRoutingGraph,
  knownBookIds?: ReadonlySet<string>,
): EnumeratedPath[] {
  const byId = new Map<number, RoutingNode>(graph.nodes.map((n) => [n.id, n]));
  const out: EnumeratedPath[] = [];
  if (graph.nodes.length === 0 || !byId.has(graph.entry)) return out;

  const walk = (
    current: number,
    visited: ReadonlySet<number>,
    conditions: PathCondition[],
    nodes: number[],
  ): void => {
    if (out.length >= MAX_ENUMERATED_PATHS) return;
    const node = byId.get(current);
    // A dangling edge (missing node) or a loop never terminates at a book, so it is
    // not a rule — validateGraph reports it as a defect instead.
    if (!node || visited.has(current)) return;
    const nextNodes = [...nodes, current];
    if (node.kind === "book") {
      const bookId = node.bookId.length === 0 ? null : node.bookId;
      const unknown =
        knownBookIds !== undefined && (bookId === null || !knownBookIds.has(bookId));
      const leaf: EnumeratedPath = { nodes: nextNodes, conditions, bookId, valid: !unknown };
      if (unknown) {
        leaf.issue =
          bookId === null
            ? "No destination book is selected."
            : `Routes to unknown / disabled book “${bookId}”.`;
      }
      out.push(leaf);
      return;
    }
    const nextVisited = new Set(visited);
    nextVisited.add(current);
    const c = node.condition;
    walk(
      c.onTrue,
      nextVisited,
      [...conditions, { field: c.field, op: c.op, value: c.value, branch: "onTrue" }],
      nextNodes,
    );
    walk(
      c.onFalse,
      nextVisited,
      [...conditions, { field: c.field, op: c.op, value: c.value, branch: "onFalse" }],
      nextNodes,
    );
  };

  walk(graph.entry, new Set(), [], []);
  return out;
}

/**
 * One trader-facing routing RULE: a single destination BOOK and every guard that
 * reaches it. The book-terminating {@link enumeratePaths} are grouped by their
 * destination `bookId`, so a book reached by several branches is ONE rule with
 * several alternative `guards` (read as `guard_a` OR `guard_b`), not several rules.
 * Rules keep evaluation order — the book the router reaches first is rule #1.
 */
export interface EnumeratedRule {
  /** The destination book id (the group key), or `null` when the leaf has no book set. */
  bookId: string | null;
  /**
   * The alternative guard condition-lists — one per distinct path reaching this
   * book. A single-element array is one route; multiple elements are OR-alternatives.
   * An empty inner array is an unconditional route (the entry is itself this book).
   */
  guards: PathCondition[][];
  /** The node-id paths (parallel to {@link guards}) for the canvas highlight. */
  paths: number[][];
  /** Whether the destination book is known + enabled (false ⇒ unknown/disabled/unset). */
  valid: boolean;
  /** When `valid` is false, a trader-readable reason. */
  issue?: string;
}

/**
 * Group the book-terminating paths of `graph` into one {@link EnumeratedRule} per
 * distinct destination book, preserving evaluation order (the book reached first is
 * rule #1). Paths that loop or dangle are excluded upstream by {@link enumeratePaths}
 * — an all-looping / bookless graph therefore yields `[]` (no rules), while
 * {@link validateGraph} still surfaces the cycle/dangle as a defect. When
 * `knownBookIds` is supplied, a rule whose destination is unset or unknown/disabled
 * carries `valid:false` + an `issue`.
 */
export function enumerateRules(
  graph: RiskRoutingGraph,
  knownBookIds?: ReadonlySet<string>,
): EnumeratedRule[] {
  const paths = enumeratePaths(graph, knownBookIds);
  const UNSET_KEY = " __unset__";
  const keyOf = (bookId: string | null): string => bookId ?? UNSET_KEY;
  const byKey = new Map<string, EnumeratedRule>();
  const order: string[] = [];

  for (const p of paths) {
    const key = keyOf(p.bookId);
    let rule = byKey.get(key);
    if (rule === undefined) {
      rule = { bookId: p.bookId, guards: [], paths: [], valid: true };
      byKey.set(key, rule);
      order.push(key);
    }
    rule.guards.push(p.conditions);
    rule.paths.push(p.nodes);
    // A rule is only as valid as its worst path to the destination.
    if (!p.valid) {
      rule.valid = false;
      if (rule.issue === undefined && p.issue !== undefined) rule.issue = p.issue;
    }
  }

  return order.map((key) => byKey.get(key) as EnumeratedRule);
}
