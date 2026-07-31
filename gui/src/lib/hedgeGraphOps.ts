/**
 * Small immutable helpers for editing hedge-rule conditions — the hedge analogue of
 * the risk-routing `workspaces/riskrouting/graphOps.ts` default-op / default-value
 * helpers, factored into `lib/` so the rule editor and the tests share them. Every
 * helper returns a NEW value (never mutates).
 */
import type { HedgeField, RouteOp, RouteValue } from "../data/contract";
import { hedgeFieldSpec } from "./hedgeFields";

/** The default operator for a hedge field — the first legal op for its kind. */
export function defaultOpForHedgeField(field: HedgeField): RouteOp {
  return hedgeFieldSpec(field).validOps[0] ?? "eq";
}

/** A fresh value literal whose variant matches `op` (for a field of `field`'s kind). */
export function defaultValueForOp(field: HedgeField, op: RouteOp): RouteValue {
  const kind = hedgeFieldSpec(field).kind;
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
