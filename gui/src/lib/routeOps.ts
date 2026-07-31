/**
 * The decision-graph OPERATOR primitives shared by the risk-routing and auto-hedge
 * field registries (`lib/routeFields.ts`, `lib/hedgeFields.ts`). Both graphs reuse
 * `celnet_risk_routing::RouteOp` verbatim, so the value-kind classification, the
 * per-kind legal-operator matrix (mirrors `RouteOp::valid_for`), and the operator
 * label/glyph live here ONCE rather than being copied per registry (DRY —
 * docs/AUTO-HEDGING §5.1 reuses the routing op vocabulary unchanged).
 */
import type { RouteOp } from "../data/contract";

/** The value shape of a field — pins which operators and value editor apply. */
export type FieldKind = "enum" | "numeric" | "string";

/** Numeric fields accept ordering + equality + range (no substring / membership-by-list). */
export const NUMERIC_OPS: RouteOp[] = ["eq", "ne", "gt", "ge", "lt", "le", "between"];
/** Free strings accept equality + substring + list membership (no ordering). */
export const STRING_OPS: RouteOp[] = ["eq", "ne", "contains", "in"];
/** Enums accept equality + list membership only (no substring — an enum has no infix). */
export const ENUM_LIKE_OPS: RouteOp[] = ["eq", "ne", "in"];

/** Human label for an operator (the editor dropdowns share it). */
export function opLabel(op: RouteOp): string {
  switch (op) {
    case "eq":
      return "equals";
    case "ne":
      return "not equal";
    case "gt":
      return "greater than";
    case "ge":
      return "≥";
    case "lt":
      return "less than";
    case "le":
      return "≤";
    case "contains":
      return "contains";
    case "in":
      return "in list";
    case "between":
      return "between";
  }
}

/** A compact operator glyph for node cards / rule one-liners (legible at small sizes). */
export function opGlyph(op: RouteOp): string {
  switch (op) {
    case "eq":
      return "=";
    case "ne":
      return "≠";
    case "gt":
      return ">";
    case "ge":
      return "≥";
    case "lt":
      return "<";
    case "le":
      return "≤";
    case "contains":
      return "⊃";
    case "in":
      return "∈";
    case "between":
      return "⇔";
  }
}
