/**
 * Auto-hedge wsCodec round-trip + wire-shape tests — the GUI end of the ONE
 * `celnet.wire` hedge contract (docs/AUTO-HEDGING). For every value with an encoder +
 * decoder, `fromWire(toWire(x))` deep-equals `x`, and the ON-THE-WIRE JSON matches the
 * server descriptor codec: snake_case names, NUMERIC enum tags, the `condition|action`
 * node oneof, the reused `RouteValueDesc` value oneof, `skew_bp`/`lp_won` OMIT-when-
 * absent, and `size`/`value`/`action` singular messages as JSON `null` when absent.
 */
import { describe, expect, it } from "vitest";

import {
  hedgeFieldToWire,
  hedgeFieldFromWire,
  execStyleToWire,
  hedgeMetricToWire,
  hedgeScopeToWire,
  hedgeSizeKindToWire,
  exitActionKindToWire,
  hedgeSizeToWire,
  hedgeSizeFromWire,
  exitActionToWire,
  exitActionFromWire,
  hedgeConditionToWire,
  hedgeConditionFromWire,
  hedgeNodeToWire,
  hedgeNodeFromWire,
  hedgeGraphToWire,
  hedgeGraphFromWire,
  warehouseThresholdToWire,
  warehouseThresholdFromWire,
  hedgeProvenanceToWire,
  hedgeProvenanceFromWire,
  hedgeIntentToWire,
  hedgeIntentFromWire,
  hedgeConfigToWire,
  hedgeConfigFromWire,
  hedgeLpPanelToWire,
  hedgeLpPanelFromWire,
} from "../src/data/wsCodec";
import type {
  ExitAction,
  HedgeConfig,
  HedgeGraph,
  HedgeIntent,
  HedgeLpPanel,
  HedgeProvenance,
  WarehouseThreshold,
} from "../src/data/contract";

const fullAction: ExitAction = {
  kind: "rfq_out",
  instrument: "AGG-OIS",
  size: { kind: "fixed", fixed: 12_500 },
  skewBp: 3.5,
  toEdge: false,
  style: "worked",
  lps: ["LP-1", "LP-3"],
  internalFirst: true,
  reason: "big residual",
};

describe("hedge enum ordinals (byte-parity with the proto)", () => {
  it("HedgeFieldEnum maps in ordinal order 0..18", () => {
    expect(hedgeFieldToWire("instrument_id")).toBe(0);
    expect(hedgeFieldToWire("breached")).toBe(13);
    expect(hedgeFieldToWire("hedge_cost_bp")).toBe(17);
    expect(hedgeFieldToWire("counterparty")).toBe(18); // string identity field, wire tag 18
    expect(hedgeFieldFromWire(13)).toBe("breached");
    expect(hedgeFieldFromWire(18)).toBe("counterparty");
    expect(hedgeFieldFromWire(99)).toBe("instrument_id"); // out-of-range ⇒ proto3 zero
  });
  it("the other enums map to their proto tags", () => {
    expect(execStyleToWire("worked")).toBe(1);
    expect(hedgeMetricToWire("net_delta")).toBe(2);
    expect(hedgeScopeToWire("instrument")).toBe(2);
    expect(hedgeSizeKindToWire("fixed")).toBe(2);
    expect(exitActionKindToWire("split")).toBe(5);
    expect(exitActionKindToWire("escalate")).toBe(6);
  });
});

describe("hedge size + exit action round-trip", () => {
  it("round-trips a fixed size", () => {
    const s = { kind: "fixed" as const, fixed: 42 };
    expect(hedgeSizeFromWire(hedgeSizeToWire(s))).toEqual(s);
  });
  it("round-trips every exit-action kind", () => {
    const kinds: ExitAction["kind"][] = [
      "warehouse",
      "cross_internal",
      "skew",
      "submit_market_order",
      "rfq_out",
      "split",
      "escalate",
    ];
    for (const kind of kinds) {
      const a: ExitAction = { ...fullAction, kind };
      expect(exitActionFromWire(exitActionToWire(a))).toEqual(a);
    }
  });
  it("OMITS skew_bp when null and emits size as an object", () => {
    const a: ExitAction = { ...fullAction, kind: "skew", skewBp: null, toEdge: true };
    const wire = exitActionToWire(a);
    expect("skew_bp" in wire).toBe(false);
    expect(wire["to_edge"]).toBe(true);
    expect(typeof wire["size"]).toBe("object");
    expect(exitActionFromWire(wire).skewBp).toBeNull();
  });
  it("emits enum tags numerically", () => {
    const wire = exitActionToWire(fullAction);
    expect(wire["kind"]).toBe(4); // rfq_out
    expect(wire["style"]).toBe(1); // worked
    expect(wire["lps"]).toEqual(["LP-1", "LP-3"]);
  });
});

describe("hedge condition + node oneof", () => {
  it("round-trips a condition and rides field/op as i32 tags", () => {
    const c = {
      field: "utilization" as const,
      op: "ge" as const,
      value: { kind: "num" as const, num: 1 },
      onTrue: 3,
      onFalse: 4,
    };
    const wire = hedgeConditionToWire(c);
    expect(wire["field"]).toBe(11);
    expect(wire["op"]).toBe(3);
    expect(wire["value"]).toEqual({ num: 1 });
    expect(hedgeConditionFromWire(wire)).toEqual(c);
  });
  it("carries a null value when unset", () => {
    const wire = hedgeConditionToWire({
      field: "breached",
      op: "eq",
      value: null,
      onTrue: 1,
      onFalse: 2,
    });
    expect(wire["value"]).toBeNull();
    expect(hedgeConditionFromWire(wire).value).toBeNull();
  });
  it("keys the node oneof by the live arm (condition vs action)", () => {
    const condNode = hedgeNodeToWire({
      kind: "condition",
      id: 0,
      condition: { field: "breached", op: "eq", value: { kind: "text", text: "false" }, onTrue: 1, onFalse: 2 },
    });
    expect("condition" in condNode).toBe(true);
    expect("action" in condNode).toBe(false);

    const actionNode = hedgeNodeToWire({ kind: "action", id: 1, action: fullAction });
    expect("action" in actionNode).toBe(true);
    expect("condition" in actionNode).toBe(false);
    expect(hedgeNodeFromWire(actionNode)).toEqual({ kind: "action", id: 1, action: fullAction });
  });
});

describe("hedge graph round-trip", () => {
  it("round-trips a mixed condition/action graph", () => {
    const g: HedgeGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: {
            field: "breached",
            op: "eq",
            value: { kind: "text", text: "false" },
            onTrue: 1,
            onFalse: 2,
          },
        },
        {
          kind: "action",
          id: 1,
          action: { ...fullAction, kind: "warehouse" },
        },
        { kind: "action", id: 2, action: fullAction },
      ],
    };
    expect(hedgeGraphFromWire(hedgeGraphToWire(g))).toEqual(g);
  });
});

describe("warehouse threshold round-trip", () => {
  it("round-trips + rides scope/metric as i32", () => {
    const t: WarehouseThreshold = {
      scopeKind: "book",
      scopeId: "fi-rates-emea",
      metric: "dv01",
      cap: 250_000,
      amber: 0.7,
      red: 0.9,
      targetFraction: 0.7,
      minClip: 1_000,
      maxClip: 100_000,
      ramped: true,
      rampK: 1.5,
    };
    const wire = warehouseThresholdToWire(t);
    expect(wire["scope_kind"]).toBe(1);
    expect(wire["metric"]).toBe(0);
    expect(wire["ramp_k"]).toBe(1.5);
    expect(warehouseThresholdFromWire(wire)).toEqual(t);
  });
});

describe("provenance + intent round-trip", () => {
  const prov: HedgeProvenance = {
    hedgeId: "H-0001",
    book: "fi-marex",
    instrument: "US10Y",
    firedAt: 1_700_000_000_000,
    metric: "dv01",
    threshold: 250_000,
    netRisk: 300_000,
    utilization: 1.2,
    band: "breach",
    policyPath: [0, 2, 3],
    action: fullAction,
    internalCrossed: 20_000,
    externalHedged: 30_000,
    residual: 5_000,
    hedgePrice: 100.26,
    midAtFire: 100.25,
    slippageBp: 1.2,
    lpWon: "LP-2",
    advisory: true,
    lps: ["LP-1", "LP-3"],
  };

  it("round-trips a provenance with lp_won present", () => {
    expect(hedgeProvenanceFromWire(hedgeProvenanceToWire(prov))).toEqual(prov);
  });
  it("carries the targeted lps array on the wire", () => {
    const wire = hedgeProvenanceToWire(prov);
    expect(wire["lps"]).toEqual(["LP-1", "LP-3"]);
    expect(hedgeProvenanceFromWire(wire).lps).toEqual(["LP-1", "LP-3"]);
  });
  it("round-trips an empty lps set (internal / no-trade)", () => {
    const wire = hedgeProvenanceToWire({ ...prov, lps: [] });
    expect(wire["lps"]).toEqual([]);
    expect(hedgeProvenanceFromWire(wire).lps).toEqual([]);
  });
  it("OMITS lp_won when null and nulls the action when absent", () => {
    const wire = hedgeProvenanceToWire({ ...prov, lpWon: null, action: null });
    expect("lp_won" in wire).toBe(false);
    expect(wire["action"]).toBeNull();
    const back = hedgeProvenanceFromWire(wire);
    expect(back.lpWon).toBeNull();
    expect(back.action).toBeNull();
  });
  it("round-trips an intent", () => {
    const intent: HedgeIntent = {
      book: "fi-rates-emea",
      instrument: "OIS-5Y",
      action: { ...fullAction, kind: "cross_internal" },
      band: "red",
      netRisk: -200_000,
      threshold: 250_000,
      utilization: 0.95,
      overflow: 12_000,
      size: 12_000,
      internalCrossed: 12_000,
      externalHedged: 0,
      advisory: true,
      firedAt: 1_700_000_000_123,
      policyPath: [0, 2, 4, 5],
      reason: "red · cross_internal",
      lps: [],
    };
    expect(hedgeIntentFromWire(hedgeIntentToWire(intent))).toEqual(intent);
  });
  it("carries the targeted lps array on an external intent", () => {
    const intent: HedgeIntent = {
      book: "fi-rates-emea",
      instrument: "US10Y",
      action: fullAction,
      band: "breach",
      netRisk: 300_000,
      threshold: 250_000,
      utilization: 1.2,
      overflow: 50_000,
      size: 50_000,
      internalCrossed: 0,
      externalHedged: 50_000,
      advisory: true,
      firedAt: 1_700_000_000_500,
      policyPath: [0, 3],
      reason: "breach · rfq_out",
      lps: ["LP-1", "LP-2", "LP-4"],
    };
    const wire = hedgeIntentToWire(intent);
    expect(wire["lps"]).toEqual(["LP-1", "LP-2", "LP-4"]);
    expect(hedgeIntentFromWire(wire)).toEqual(intent);
  });
});

describe("hedging LP panel round-trip", () => {
  it("round-trips a panel + rides scope_kind as i32 with include/exclude arrays", () => {
    const p: HedgeLpPanel = {
      scopeKind: "book",
      scopeId: "fi-rates-emea",
      include: ["LP-1", "LP-2", "LP-3"],
      exclude: ["LP-2"],
    };
    const wire = hedgeLpPanelToWire(p);
    expect(wire["scope_kind"]).toBe(1); // book
    expect(wire["scope_id"]).toBe("fi-rates-emea");
    expect(wire["include"]).toEqual(["LP-1", "LP-2", "LP-3"]);
    expect(wire["exclude"]).toEqual(["LP-2"]);
    expect(hedgeLpPanelFromWire(wire)).toEqual(p);
  });
  it("round-trips an unrestricted (empty include/exclude) panel", () => {
    const p: HedgeLpPanel = { scopeKind: "desk", scopeId: "emea", include: [], exclude: [] };
    expect(hedgeLpPanelFromWire(hedgeLpPanelToWire(p))).toEqual(p);
  });
});

describe("engine config round-trip", () => {
  it("round-trips config with per-desk toggles", () => {
    const c: HedgeConfig = {
      killSwitch: false,
      advisoryOnly: true,
      deskEnabled: [
        { desk: "emea", enabled: true },
        { desk: "marex", enabled: false },
      ],
      maxClip: 150_000_000,
      maxHedgesPerInterval: 20,
      dailyExternalNotionalCap: 2_000_000_000,
      lpPanels: [],
    };
    const wire = hedgeConfigToWire(c);
    expect(wire["kill_switch"]).toBe(false);
    expect(wire["max_hedges_per_interval"]).toBe(20);
    expect(hedgeConfigFromWire(wire)).toEqual(c);
  });
  it("round-trips config carrying standing lp_panels", () => {
    const c: HedgeConfig = {
      killSwitch: false,
      advisoryOnly: true,
      deskEnabled: [{ desk: "emea", enabled: true }],
      maxClip: 150_000_000,
      maxHedgesPerInterval: 20,
      dailyExternalNotionalCap: 2_000_000_000,
      lpPanels: [
        { scopeKind: "book", scopeId: "fi-rates-emea", include: ["LP-1", "LP-2", "LP-3"], exclude: ["LP-2"] },
        { scopeKind: "desk", scopeId: "emea", include: [], exclude: ["LP-4"] },
      ],
    };
    const wire = hedgeConfigToWire(c);
    expect(Array.isArray(wire["lp_panels"])).toBe(true);
    expect((wire["lp_panels"] as unknown[]).length).toBe(2);
    expect(hedgeConfigFromWire(wire)).toEqual(c);
  });
});
