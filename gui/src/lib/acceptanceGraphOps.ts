/**
 * Small immutable helpers for editing acceptance-rule conditions — the acceptance
 * analogue of `lib/hedgeGraphOps.ts`, factored into `lib/` so the rule editor and the
 * tests share them. Every helper returns a NEW value (never mutates).
 */
import type { AcceptanceField, RouteOp, RouteValue } from "../data/contract";
import { acceptanceFieldSpec } from "./acceptanceFields";

/** The default operator for an acceptance field — the first legal op for its kind. */
export function defaultOpForAcceptanceField(field: AcceptanceField): RouteOp {
  return acceptanceFieldSpec(field).validOps[0] ?? "eq";
}

/** A fresh value literal whose variant matches `op` (for a field of `field`'s kind). */
export function defaultValueForOp(field: AcceptanceField, op: RouteOp): RouteValue {
  const kind = acceptanceFieldSpec(field).kind;
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
