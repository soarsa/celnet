/**
 * Acceptance RULES model tests — the trader-facing rules-table abstraction over the
 * wire {@link AcceptanceGraph}. Verifies the compile → graph → decompile round-trip
 * (conditions, order, decisions preserved) is byte-stable, the plain-English
 * description, the decision-leaf defaults, and the structural conflict detector
 * (duplicate / shadow / exactly-one-default) — the acceptance analogue of
 * `hedgeRules.test.ts`.
 */
import { describe, expect, it } from "vitest";

import type { AcceptanceAction } from "../src/data/contract";
import { defaultAcceptanceAction } from "../src/lib/acceptanceAction";
import {
  compileRulesToAcceptanceGraph,
  decompileAcceptanceGraphToRules,
  describeAcceptanceRule,
  detectAcceptanceRuleConflicts,
  newAcceptanceRuleId,
  type AcceptanceRule,
  type AcceptanceRuleCondition,
} from "../src/lib/acceptanceRules";

function cond(
  field: AcceptanceRuleCondition["field"],
  op: AcceptanceRuleCondition["op"],
  value: AcceptanceRuleCondition["value"],
): AcceptanceRuleCondition {
  return { field, op, value };
}
function rule(conditions: AcceptanceRuleCondition[], action: AcceptanceAction): AcceptanceRule {
  return { id: newAcceptanceRuleId(), conditions, action, enabled: true };
}

const reject: AcceptanceAction = { kind: "reject", reason: "below edge floor" };
const hold: AcceptanceAction = { kind: "hold_for_review", reason: "stale quote" };
const accept: AcceptanceAction = defaultAcceptanceAction("accept");

describe("compile ↔ decompile round-trip", () => {
  it("preserves conditions, order and decisions (with reasons)", () => {
    const rules: AcceptanceRule[] = [
      rule([cond("edge_bps", "lt", { kind: "num", num: 0.5 })], reject),
      rule([cond("counterparty", "in", { kind: "list", values: ["Alpha", "Beta"] })], hold),
      rule([], accept),
    ];
    const graph = compileRulesToAcceptanceGraph(rules);
    const back = decompileAcceptanceGraphToRules(graph);
    expect(back).toHaveLength(3);
    expect(back[0]?.conditions).toEqual(rules[0]?.conditions);
    expect(back[0]?.action).toEqual(reject);
    expect(back[1]?.action).toEqual(hold);
    expect(back[2]?.conditions).toEqual([]);
    expect(back[2]?.action.kind).toBe("accept");
  });

  it("is byte-stable — recompiling the decompiled rules yields an identical graph", () => {
    const rules: AcceptanceRule[] = [
      rule(
        [
          cond("asset_class", "eq", { kind: "text", text: "fixed_income" }),
          cond("quote_age_ms", "gt", { kind: "num", num: 800 }),
        ],
        hold,
      ),
      rule([], accept),
    ];
    const graph = compileRulesToAcceptanceGraph(rules);
    const recompiled = compileRulesToAcceptanceGraph(decompileAcceptanceGraphToRules(graph));
    expect(recompiled).toEqual(graph);
  });

  it("produces a strictly-acyclic right-spine graph (edges point forward)", () => {
    const graph = compileRulesToAcceptanceGraph([
      rule([cond("notional_usd", "ge", { kind: "num", num: 1_000_000 })], reject),
      rule([], accept),
    ]);
    for (const n of graph.nodes) {
      if (n.kind === "condition") {
        expect(n.condition.onTrue).toBeGreaterThan(n.id);
        expect(n.condition.onFalse).toBeGreaterThan(n.id);
      }
    }
  });

  it("an empty rule list compiles to the empty graph", () => {
    expect(compileRulesToAcceptanceGraph([])).toEqual({ entry: 0, nodes: [] });
  });
});

describe("describeAcceptanceRule", () => {
  it("renders a specific rule as guard → decision", () => {
    const r = rule([cond("edge_bps", "lt", { kind: "num", num: 0.5 })], reject);
    expect(describeAcceptanceRule(r)).toBe('Edge (bps) < 0.5 → Reject · "below edge floor"');
  });
  it("renders the default rule as Otherwise → decision", () => {
    expect(describeAcceptanceRule(rule([], accept))).toBe("Otherwise → Accept");
  });
});

describe("detectAcceptanceRuleConflicts", () => {
  it("flags a missing default (no catch-all)", () => {
    const conflicts = detectAcceptanceRuleConflicts([
      rule([cond("edge_bps", "lt", { kind: "num", num: 0.5 })], reject),
    ]);
    expect(conflicts.some((c) => c.message.includes("No default rule"))).toBe(true);
  });

  it("flags an exact duplicate condition set", () => {
    const c = cond("side", "eq", { kind: "text", text: "buy" });
    const conflicts = detectAcceptanceRuleConflicts([
      rule([c], reject),
      rule([c], hold),
      rule([], accept),
    ]);
    expect(conflicts.some((x) => x.message.includes("Duplicate rule"))).toBe(true);
  });

  it("flags a rule shadowed by an earlier, more general rule", () => {
    const conflicts = detectAcceptanceRuleConflicts([
      rule([cond("side", "eq", { kind: "text", text: "buy" })], reject),
      rule(
        [
          cond("side", "eq", { kind: "text", text: "buy" }),
          cond("edge_bps", "lt", { kind: "num", num: 0 }),
        ],
        hold,
      ),
      rule([], accept),
    ]);
    expect(conflicts.some((x) => x.message.includes("Unreachable"))).toBe(true);
  });

  it("is clean for a well-formed policy with exactly one default", () => {
    const conflicts = detectAcceptanceRuleConflicts([
      rule([cond("edge_bps", "lt", { kind: "num", num: 0.5 })], reject),
      rule([], accept),
    ]);
    expect(conflicts).toHaveLength(0);
  });
});
