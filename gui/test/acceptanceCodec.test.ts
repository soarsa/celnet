/**
 * Acceptance wsCodec round-trip + wire-shape tests — the GUI end of the ONE
 * `celnet.wire` acceptance contract (`celnet-acceptance`). For every value with an
 * encoder + decoder, `fromWire(toWire(x))` deep-equals `x`, and the ON-THE-WIRE JSON
 * matches the server descriptor codec: snake_case names, NUMERIC enum tags, the
 * `condition|decision` node oneof, the reused `RouteValueDesc` value oneof, and the
 * `{ graph: {...} | null }` request/response framing.
 */
import { describe, expect, it } from "vitest";

import {
  acceptanceFieldToWire,
  acceptanceFieldFromWire,
  acceptanceActionKindToWire,
  acceptanceActionKindFromWire,
  acceptanceActionToWire,
  acceptanceActionFromWire,
  acceptanceConditionToWire,
  acceptanceConditionFromWire,
  acceptanceNodeToWire,
  acceptanceNodeFromWire,
  acceptanceGraphToWire,
  acceptanceGraphFromWire,
  acceptanceGraphResponseFromWire,
  updateAcceptanceGraphRequestToWire,
  updateAcceptanceGraphResponseFromWire,
} from "../src/data/wsCodec";
import type { AcceptanceGraph } from "../src/data/contract";

describe("acceptance enum ordinals (byte-parity with the proto)", () => {
  it("AcceptanceFieldEnum maps in ordinal order 0..8", () => {
    expect(acceptanceFieldToWire("counterparty")).toBe(0);
    expect(acceptanceFieldToWire("notional_usd")).toBe(1);
    expect(acceptanceFieldToWire("tenor_years")).toBe(2);
    expect(acceptanceFieldToWire("instrument_symbol")).toBe(3);
    expect(acceptanceFieldToWire("side")).toBe(4);
    expect(acceptanceFieldToWire("edge_bps")).toBe(5);
    expect(acceptanceFieldToWire("quote_age_ms")).toBe(6);
    expect(acceptanceFieldToWire("asset_class")).toBe(7);
    expect(acceptanceFieldToWire("desk")).toBe(8);
    expect(acceptanceFieldFromWire(5)).toBe("edge_bps");
    expect(acceptanceFieldFromWire(99)).toBe("counterparty"); // out-of-range ⇒ proto3 zero
  });
  it("AcceptanceActionKind maps accept=0 / reject=1 / hold_for_review=2", () => {
    expect(acceptanceActionKindToWire("accept")).toBe(0);
    expect(acceptanceActionKindToWire("reject")).toBe(1);
    expect(acceptanceActionKindToWire("hold_for_review")).toBe(2);
    expect(acceptanceActionKindFromWire(2)).toBe("hold_for_review");
    expect(acceptanceActionKindFromWire(99)).toBe("accept");
  });
});

describe("acceptance decision + condition + node oneof", () => {
  it("encodes a decision as a kind tag + reason string", () => {
    const wire = acceptanceActionToWire({ kind: "reject", reason: "below edge floor" });
    expect(wire).toEqual({ kind: 1, reason: "below edge floor" });
    expect(acceptanceActionFromWire(wire)).toEqual({ kind: "reject", reason: "below edge floor" });
  });
  it("round-trips a condition and rides field/op as i32 tags", () => {
    const c = {
      field: "edge_bps" as const,
      op: "lt" as const,
      value: { kind: "num" as const, num: 0.5 },
      onTrue: 3,
      onFalse: 4,
    };
    const wire = acceptanceConditionToWire(c);
    expect(wire["field"]).toBe(5);
    expect(wire["op"]).toBe(4); // lt
    expect(wire["value"]).toEqual({ num: 0.5 });
    expect(acceptanceConditionFromWire(wire)).toEqual(c);
  });
  it("carries a null value when unset", () => {
    const wire = acceptanceConditionToWire({
      field: "counterparty",
      op: "eq",
      value: null,
      onTrue: 1,
      onFalse: 2,
    });
    expect(wire["value"]).toBeNull();
    expect(acceptanceConditionFromWire(wire).value).toBeNull();
  });
  it("keys the node oneof by the live arm (condition vs decision)", () => {
    const condNode = acceptanceNodeToWire({
      kind: "condition",
      id: 0,
      condition: { field: "side", op: "eq", value: { kind: "text", text: "buy" }, onTrue: 1, onFalse: 2 },
    });
    expect("condition" in condNode).toBe(true);
    expect("decision" in condNode).toBe(false);

    const decisionNode = acceptanceNodeToWire({
      kind: "decision",
      id: 1,
      action: { kind: "accept", reason: "" },
    });
    expect("decision" in decisionNode).toBe(true);
    expect("condition" in decisionNode).toBe(false);
    expect(acceptanceNodeFromWire(decisionNode)).toEqual({
      kind: "decision",
      id: 1,
      action: { kind: "accept", reason: "" },
    });
  });
});

const mixedGraph: AcceptanceGraph = {
  entry: 0,
  nodes: [
    {
      kind: "condition",
      id: 0,
      condition: {
        field: "edge_bps",
        op: "lt",
        value: { kind: "num", num: 0.5 },
        onTrue: 1,
        onFalse: 2,
      },
    },
    { kind: "decision", id: 1, action: { kind: "reject", reason: "below edge floor" } },
    { kind: "decision", id: 2, action: { kind: "accept", reason: "" } },
  ],
};

describe("acceptance graph + request/response framing", () => {
  it("round-trips a mixed condition/decision graph", () => {
    expect(acceptanceGraphFromWire(acceptanceGraphToWire(mixedGraph))).toEqual(mixedGraph);
  });

  it("decodes a present graph from the get response", () => {
    const resp = { graph: acceptanceGraphToWire(mixedGraph) };
    expect(acceptanceGraphResponseFromWire(resp)).toEqual(mixedGraph);
  });

  it("decodes a null / absent graph as null (policy not yet defined)", () => {
    expect(acceptanceGraphResponseFromWire({ graph: null })).toBeNull();
    expect(acceptanceGraphResponseFromWire({})).toBeNull();
  });

  it("frames the update request + decodes the always-present update reply", () => {
    const req = updateAcceptanceGraphRequestToWire(mixedGraph);
    expect(acceptanceGraphFromWire(req["graph"] as Record<string, unknown>)).toEqual(mixedGraph);
    const reply = { graph: acceptanceGraphToWire(mixedGraph) };
    expect(updateAcceptanceGraphResponseFromWire(reply)).toEqual(mixedGraph);
  });
});
