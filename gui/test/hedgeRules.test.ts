/**
 * Hedge RULES model tests — the trader-facing rules-table abstraction over the wire
 * {@link HedgeGraph}. Verifies the compile → graph → decompile round-trip (conditions,
 * order, exit actions preserved), the plain-English description, the exit-action leaf
 * defaults, and the structural conflict detector (duplicate / shadow / exactly-one-
 * default) — the hedge analogue of `riskRules.test.ts`.
 */
import { describe, expect, it } from "vitest";

import type { ExitAction } from "../src/data/contract";
import { defaultExitAction } from "../src/lib/hedgeExit";
import {
  compileRulesToHedgeGraph,
  decompileHedgeGraphToRules,
  describeHedgeRule,
  detectHedgeRuleConflicts,
  newHedgeRuleId,
  type HedgeRule,
  type HedgeRuleCondition,
} from "../src/lib/hedgeRules";

function cond(
  field: HedgeRuleCondition["field"],
  op: HedgeRuleCondition["op"],
  value: HedgeRuleCondition["value"],
): HedgeRuleCondition {
  return { field, op, value };
}
function rule(conditions: HedgeRuleCondition[], action: ExitAction): HedgeRule {
  return { id: newHedgeRuleId(), conditions, action, enabled: true };
}

const marketOrder: ExitAction = { ...defaultExitAction("submit_market_order") };
const warehouse: ExitAction = { ...defaultExitAction("warehouse") };

describe("compile ↔ decompile round-trip", () => {
  it("preserves conditions, order and exit actions", () => {
    const rules: HedgeRule[] = [
      rule([cond("breached", "eq", { kind: "text", text: "true" }), cond("counterparty_toxicity", "gt", { kind: "num", num: 0.6 })], marketOrder),
      rule([], warehouse),
    ];
    const graph = compileRulesToHedgeGraph(rules);
    const back = decompileHedgeGraphToRules(graph);
    expect(back).toHaveLength(2);
    expect(back[0]?.conditions).toEqual(rules[0]?.conditions);
    expect(back[0]?.action.kind).toBe("submit_market_order");
    expect(back[1]?.conditions).toEqual([]);
    expect(back[1]?.action.kind).toBe("warehouse");
  });

  it("produces a strictly-acyclic right-spine graph (edges point forward)", () => {
    const graph = compileRulesToHedgeGraph([
      rule([cond("utilization", "ge", { kind: "num", num: 1 })], marketOrder),
      rule([], warehouse),
    ]);
    for (const n of graph.nodes) {
      if (n.kind === "condition") {
        expect(n.condition.onTrue).toBeGreaterThan(n.id);
        expect(n.condition.onFalse).toBeGreaterThan(n.id);
      }
    }
  });

  it("an empty rule list compiles to an empty graph", () => {
    expect(compileRulesToHedgeGraph([])).toEqual({ entry: 0, nodes: [] });
  });
});

describe("describeHedgeRule", () => {
  it("renders a guarded rule and the default rule", () => {
    const guarded = rule([cond("breached", "eq", { kind: "text", text: "true" })], marketOrder);
    expect(describeHedgeRule(guarded)).toContain("Breached");
    expect(describeHedgeRule(guarded)).toContain("Submit market order");
    expect(describeHedgeRule(rule([], warehouse))).toBe("Otherwise → Warehouse (hold)");
  });
});

describe("detectHedgeRuleConflicts", () => {
  it("passes a clean specific + default pair", () => {
    const conflicts = detectHedgeRuleConflicts([
      rule([cond("breached", "eq", { kind: "text", text: "true" })], marketOrder),
      rule([], warehouse),
    ]);
    expect(conflicts).toHaveLength(0);
  });

  it("flags an exact duplicate", () => {
    const c = cond("breached", "eq", { kind: "text", text: "true" });
    const conflicts = detectHedgeRuleConflicts([rule([c], marketOrder), rule([c], warehouse), rule([], warehouse)]);
    expect(conflicts.some((x) => x.message.includes("Duplicate"))).toBe(true);
  });

  it("flags a missing default", () => {
    const conflicts = detectHedgeRuleConflicts([
      rule([cond("breached", "eq", { kind: "text", text: "true" })], marketOrder),
    ]);
    expect(conflicts.some((x) => x.message.includes("No default"))).toBe(true);
  });

  it("flags a rule shadowed by an earlier default", () => {
    const conflicts = detectHedgeRuleConflicts([
      rule([], warehouse), // default first — shadows everything after
      rule([cond("breached", "eq", { kind: "text", text: "true" })], marketOrder),
    ]);
    expect(conflicts.some((x) => x.message.includes("Unreachable"))).toBe(true);
  });
});
