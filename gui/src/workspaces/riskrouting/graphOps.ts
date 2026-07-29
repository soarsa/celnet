/**
 * Immutable edit operations on a {@link RiskRoutingGraph}. Every helper returns a
 * NEW graph (never mutates) so React state updates stay predictable and the
 * undo-free editor never aliases the loaded object. Kept out of the components so
 * the transforms are unit-testable in isolation.
 */
import type {
  RiskRoutingGraph,
  RouteField,
  RouteOp,
  RouteValue,
  RoutingNode,
} from "../../data/contract";
import { fieldSpec } from "../../lib/routeFields";

/** The next free node id (max existing + 1, or 0 for an empty graph). */
export function nextNodeId(graph: RiskRoutingGraph): number {
  return graph.nodes.reduce((mx, n) => Math.max(mx, n.id), -1) + 1;
}

/** The default operator for a field — the first legal op for its kind. */
export function defaultOpForField(field: RouteField): RouteOp {
  return fieldSpec(field).validOps[0] ?? "eq";
}

/** A fresh value literal whose variant matches `op` (for a field of `field`'s kind). */
export function defaultValueForOp(field: RouteField, op: RouteOp): RouteValue {
  const kind = fieldSpec(field).kind;
  switch (op) {
    case "eq":
    case "ne":
      return kind === "numeric" ? { kind: "num", num: 0 } : { kind: "text", text: "" };
    case "gt":
    case "ge":
    case "lt":
    case "le":
      return { kind: "num", num: 0 };
    case "contains":
      return { kind: "text", text: "" };
    case "in":
      return { kind: "list", values: [] };
    case "between":
      return { kind: "range", lo: 0, hi: 0 };
  }
}

/** Replace one node by id with `next` (identity by id). */
export function replaceNode(graph: RiskRoutingGraph, next: RoutingNode): RiskRoutingGraph {
  return { ...graph, nodes: graph.nodes.map((n) => (n.id === next.id ? next : n)) };
}

/** Add a fresh Condition node pre-bound to `field`, returning `{ graph, id }`. */
export function addCondition(
  graph: RiskRoutingGraph,
  field: RouteField,
): { graph: RiskRoutingGraph; id: number } {
  const id = nextNodeId(graph);
  const op = defaultOpForField(field);
  const node: RoutingNode = {
    kind: "condition",
    id,
    condition: {
      field,
      op,
      value: defaultValueForOp(field, op),
      // Both branches self-loop as a visible "unwired" placeholder the validator
      // flags via the reachable-cycle / dangling check until the trader wires them.
      onTrue: id,
      onFalse: id,
    },
  };
  const nodes = [...graph.nodes, node];
  return { graph: { entry: graph.nodes.length === 0 ? id : graph.entry, nodes }, id };
}

/** Add a fresh Book leaf (no target yet), returning `{ graph, id }`. */
export function addBook(graph: RiskRoutingGraph): { graph: RiskRoutingGraph; id: number } {
  const id = nextNodeId(graph);
  const node: RoutingNode = { kind: "book", id, bookId: "" };
  const nodes = [...graph.nodes, node];
  return { graph: { entry: graph.nodes.length === 0 ? id : graph.entry, nodes }, id };
}

/**
 * Rebind a Condition node to a new field, resetting its op + value to the new
 * field's defaults (a field swap invalidates the prior op/value). Branches are
 * preserved. A no-op on a Book node.
 */
export function replaceField(
  graph: RiskRoutingGraph,
  nodeId: number,
  field: RouteField,
): RiskRoutingGraph {
  const node = graph.nodes.find((n) => n.id === nodeId);
  if (!node || node.kind !== "condition") return graph;
  const op = defaultOpForField(field);
  return replaceNode(graph, {
    ...node,
    condition: {
      ...node.condition,
      field,
      op,
      value: defaultValueForOp(field, op),
    },
  });
}

/** Set a Condition's operator, coercing its value to the new op's default variant. */
export function setOperator(
  graph: RiskRoutingGraph,
  nodeId: number,
  op: RouteOp,
): RiskRoutingGraph {
  const node = graph.nodes.find((n) => n.id === nodeId);
  if (!node || node.kind !== "condition") return graph;
  return replaceNode(graph, {
    ...node,
    condition: { ...node.condition, op, value: defaultValueForOp(node.condition.field, op) },
  });
}

/** Set a Condition's value literal. */
export function setValue(
  graph: RiskRoutingGraph,
  nodeId: number,
  value: RouteValue,
): RiskRoutingGraph {
  const node = graph.nodes.find((n) => n.id === nodeId);
  if (!node || node.kind !== "condition") return graph;
  return replaceNode(graph, { ...node, condition: { ...node.condition, value } });
}

/** Wire a Condition's `on_true`/`on_false` branch to a target node id. */
export function setBranch(
  graph: RiskRoutingGraph,
  nodeId: number,
  branch: "onTrue" | "onFalse",
  target: number,
): RiskRoutingGraph {
  const node = graph.nodes.find((n) => n.id === nodeId);
  if (!node || node.kind !== "condition") return graph;
  return replaceNode(graph, { ...node, condition: { ...node.condition, [branch]: target } });
}

/** Set a Book leaf's target risk-book id. */
export function setBookTarget(
  graph: RiskRoutingGraph,
  nodeId: number,
  bookId: string,
): RiskRoutingGraph {
  const node = graph.nodes.find((n) => n.id === nodeId);
  if (!node || node.kind !== "book") return graph;
  return replaceNode(graph, { ...node, bookId });
}

/** Mark a node as the entry (the walk's start). */
export function setEntry(graph: RiskRoutingGraph, nodeId: number): RiskRoutingGraph {
  return { ...graph, entry: nodeId };
}

/**
 * Delete a node and heal every edge that pointed at it: any branch (or the entry)
 * that referenced the removed node is re-pointed at a surviving fallback so the
 * graph stays free of dangling edges where possible.
 */
export function deleteNode(graph: RiskRoutingGraph, nodeId: number): RiskRoutingGraph {
  const remaining = graph.nodes.filter((n) => n.id !== nodeId);
  if (remaining.length === graph.nodes.length) return graph;
  const fallback = remaining[0]?.id ?? 0;
  const heal = (target: number): number => (target === nodeId ? fallback : target);
  const nodes: RoutingNode[] = remaining.map((n) =>
    n.kind === "condition"
      ? {
          ...n,
          condition: {
            ...n.condition,
            onTrue: heal(n.condition.onTrue),
            onFalse: heal(n.condition.onFalse),
          },
        }
      : n,
  );
  const entry = graph.entry === nodeId ? fallback : graph.entry;
  return { entry, nodes };
}
