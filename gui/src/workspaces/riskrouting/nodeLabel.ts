/**
 * Presentational helpers shared by the canvas cards, the branch selects, and the
 * trace panel — a compact human summary of a node and of a route value.
 */
import type { RouteValue, RoutingNode } from "../../data/contract";
import { fieldSpec, opGlyph } from "../../lib/routeFields";

/** A compact number format for card / value display (60m, 1.5k, 10). */
export function compactNum(n: number): string {
  if (!Number.isFinite(n)) return "∞";
  return new Intl.NumberFormat("en-US", {
    notation: Math.abs(n) >= 1000 ? "compact" : "standard",
    maximumFractionDigits: 2,
  }).format(n);
}

/** Render a route value literal for a card / summary. */
export function valueLabel(value: RouteValue | null): string {
  if (value === null) return "—";
  switch (value.kind) {
    case "num":
      return compactNum(value.num);
    case "text":
      return value.text.length === 0 ? "“”" : `“${value.text}”`;
    case "list":
      return value.values.length === 0 ? "[ ]" : `[${value.values.join(", ")}]`;
    case "range":
      return `${compactNum(value.lo)}…${compactNum(value.hi)}`;
  }
}

/**
 * A one-line summary of a node for the branch selects + trace path (e.g.
 * `Currency = “EUR”` or `Book: FX EMEA`). Books resolve their id to a name via
 * `bookName`.
 */
export function nodeSummary(node: RoutingNode, bookName: (id: string) => string): string {
  if (node.kind === "book") {
    return node.bookId.length === 0 ? "Book: (unset)" : `Book: ${bookName(node.bookId)}`;
  }
  const c = node.condition;
  return `${fieldSpec(c.field).label} ${opGlyph(c.op)} ${valueLabel(c.value)}`;
}
