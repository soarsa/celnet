/**
 * Hedge TRACE + VALIDATE tests — the client mirror of the Rust hedge engine. Verifies
 * the walk lands the exit action the policy resolves to for a given risk state (the
 * "what would fire" panel), that `breached` compares by "true"/"false" text, and that
 * the validator collects the well-formedness defects that gate Save (dangling edge,
 * cycle, unset/typed value, action-target missing).
 */
import { describe, expect, it } from "vitest";

import type { HedgeGraph } from "../src/data/contract";
import { compileRulesToHedgeGraph, newHedgeRuleId, type HedgeRule } from "../src/lib/hedgeRules";
import { defaultExitAction } from "../src/lib/hedgeExit";
import { blankHedgeState, traceHedgeGraph, validateHedgeGraph } from "../src/lib/hedgeTrace";

/** The doc §5.4 example policy compiled from rules. */
function examplePolicy(): HedgeGraph {
  const marketOrder = { ...defaultExitAction("submit_market_order") };
  const cross = { ...defaultExitAction("cross_internal"), instrument: "AGG-OIS" };
  const warehouse = { ...defaultExitAction("warehouse") };
  const rules: HedgeRule[] = [
    { id: newHedgeRuleId(), conditions: [{ field: "breached", op: "eq", value: { kind: "text", text: "false" } }], action: warehouse, enabled: true },
    { id: newHedgeRuleId(), conditions: [{ field: "counterparty_toxicity", op: "gt", value: { kind: "num", num: 0.6 } }], action: marketOrder, enabled: true },
    { id: newHedgeRuleId(), conditions: [{ field: "internal_offset_available", op: "gt", value: { kind: "num", num: 0 } }], action: cross, enabled: true },
    { id: newHedgeRuleId(), conditions: [], action: { ...defaultExitAction("split"), internalFirst: true }, enabled: true },
  ];
  return compileRulesToHedgeGraph(rules);
}

describe("traceHedgeGraph", () => {
  const graph = examplePolicy();

  it("warehouses a green (un-breached) state", () => {
    const trace = traceHedgeGraph(graph, { ...blankHedgeState(), breached: false });
    expect(trace.outcome).toBe("action");
    expect(trace.landedAction?.kind).toBe("warehouse");
  });

  it("fires a market order for toxic breached flow", () => {
    const trace = traceHedgeGraph(graph, {
      ...blankHedgeState(),
      breached: true,
      counterpartyToxicity: 0.75,
      overflow: 50_000,
    });
    expect(trace.landedAction?.kind).toBe("submit_market_order");
  });

  it("crosses internally when benign flow has an offset", () => {
    const trace = traceHedgeGraph(graph, {
      ...blankHedgeState(),
      breached: true,
      counterpartyToxicity: 0.1,
      internalOffsetAvailable: 40_000,
    });
    expect(trace.landedAction?.kind).toBe("cross_internal");
  });

  it("splits when benign flow has no offset", () => {
    const trace = traceHedgeGraph(graph, {
      ...blankHedgeState(),
      breached: true,
      counterpartyToxicity: 0.1,
      internalOffsetAvailable: 0,
    });
    expect(trace.landedAction?.kind).toBe("split");
  });
});

describe("validateHedgeGraph", () => {
  it("accepts the compiled example policy", () => {
    expect(validateHedgeGraph(examplePolicy())).toHaveLength(0);
  });

  it("flags a dangling edge", () => {
    const graph: HedgeGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: { field: "breached", op: "eq", value: { kind: "text", text: "true" }, onTrue: 99, onFalse: 1 },
        },
        { kind: "action", id: 1, action: { ...defaultExitAction("warehouse") } },
      ],
    };
    expect(validateHedgeGraph(graph).some((i) => i.code === "dangling_edge")).toBe(true);
  });

  it("flags a cross-internal action with no instrument, and an rfq with no LPs", () => {
    const graph: HedgeGraph = {
      entry: 0,
      nodes: [
        { kind: "action", id: 0, action: { ...defaultExitAction("cross_internal") } },
        { kind: "action", id: 1, action: { ...defaultExitAction("rfq_out") } },
      ],
    };
    const issues = validateHedgeGraph(graph);
    expect(issues.filter((i) => i.code === "action_target_missing")).toHaveLength(2);
  });

  it("flags an unset value and an op invalid for the field kind", () => {
    const graph: HedgeGraph = {
      entry: 0,
      nodes: [
        { kind: "condition", id: 0, condition: { field: "utilization", op: "gt", value: null, onTrue: 1, onFalse: 1 } },
        // `contains` is a string-only op, invalid for the numeric `overflow` field.
        { kind: "condition", id: 1, condition: { field: "overflow", op: "contains", value: { kind: "text", text: "x" }, onTrue: 2, onFalse: 2 } },
        { kind: "action", id: 2, action: { ...defaultExitAction("warehouse") } },
      ],
    };
    const codes = validateHedgeGraph(graph).map((i) => i.code);
    expect(codes).toContain("value_unset");
    expect(codes).toContain("op_not_valid_for_field");
  });
});
