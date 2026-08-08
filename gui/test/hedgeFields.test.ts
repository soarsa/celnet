/**
 * hedgeFields — the `counterparty` identity field (wire tag 18) is a first-class hedge
 * rule-builder condition: it renders in the field palette (Identity group), it is a free
 * STRING (so the ValueEditor gives a text/list input), and it carries exactly the
 * string operator set (`eq`/`ne`/`in`/`contains`) — the same shape as `instrument_id`.
 */
import { describe, expect, it } from "vitest";

import {
  HEDGE_FIELD_REGISTRY,
  hedgeFieldKind,
  hedgeFieldSpec,
  opValidForHedgeField,
} from "../src/lib/hedgeFields";
import { hedgeFieldToWire } from "../src/data/wsCodec";
import type { RouteOp } from "../src/data/contract";

describe("hedgeFields — counterparty condition field", () => {
  it("is registered in the palette under the Identity group", () => {
    const spec = HEDGE_FIELD_REGISTRY.find((s) => s.field === "counterparty");
    expect(spec).toBeDefined();
    expect(spec?.group).toBe("Identity");
    expect(spec?.label).toBe("Counterparty");
    expect(spec?.hint.length).toBeGreaterThan(0);
  });

  it("is a free STRING field (drives the text/list value editor)", () => {
    expect(hedgeFieldKind("counterparty")).toBe("string");
    // mirrors the existing string identity field exactly
    expect(hedgeFieldKind("counterparty")).toBe(hedgeFieldKind("instrument_id"));
  });

  it("allows exactly the string operators eq/ne/in/contains", () => {
    const ops = [...hedgeFieldSpec("counterparty").validOps].sort();
    expect(ops).toEqual((["contains", "eq", "in", "ne"] as RouteOp[]).sort());
    for (const op of ["eq", "ne", "in", "contains"] as RouteOp[]) {
      expect(opValidForHedgeField("counterparty", op)).toBe(true);
    }
    // numeric-only operators are rejected (the server would reject them too)
    for (const op of ["gt", "ge", "lt", "le", "between"] as RouteOp[]) {
      expect(opValidForHedgeField("counterparty", op)).toBe(false);
    }
  });

  it("serializes to wire tag 18 (appended after hedge_cost_bp=17)", () => {
    expect(hedgeFieldToWire("counterparty")).toBe(18);
  });

  it("keeps the registry array index equal to the wire tag (ordinal parity)", () => {
    HEDGE_FIELD_REGISTRY.forEach((spec, idx) => {
      expect(hedgeFieldToWire(spec.field)).toBe(idx);
    });
  });
});
