/**
 * riskRules — the trader RULES model and its exact bridge to the wire
 * {@link RiskRoutingGraph}. These tests pin: (1) compile↔decompile round-trips a
 * rule list exactly; (2) the compiled graph is genuinely first-match-wins (earlier
 * rule + ANDed conditions win) when traced by the server-parity walker; (3)
 * describeRule's plain-English one-liner; and (4) each of the four conflict checks.
 */
import { describe, expect, it } from "vitest";

import type { RouteValue } from "../src/data/contract";
import {
  compileRulesToGraph,
  decompileGraphToRules,
  describeRule,
  detectRuleConflicts,
  newRuleId,
  type RiskRule,
  type RuleCondition,
} from "../src/lib/riskRules";
import { blankFill, traceGraph, type SampleFill } from "../src/lib/routeTrace";

function cond(field: RuleCondition["field"], op: RuleCondition["op"], value: RouteValue): RuleCondition {
  return { field, op, value };
}
function rule(conditions: RuleCondition[], bookId: string | null): RiskRule {
  return { id: newRuleId(), conditions, bookId, enabled: true };
}
/** Compare rules ignoring the (non-wire) id. */
function core(rs: RiskRule[]) {
  return rs.map((r) => ({ conditions: r.conditions, bookId: r.bookId, enabled: r.enabled }));
}
function fill(overrides: Partial<SampleFill> = {}): SampleFill {
  return { ...blankFill(), ...overrides };
}

const BOOKS = new Set(["BOOK-A", "BOOK-B", "DEFAULT"]);

describe("compile ↔ decompile round-trip", () => {
  it("recovers a multi-rule list exactly (conditions, order, destinations)", () => {
    const rules = [
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" }), cond("notional", "gt", { kind: "num", num: 5e7 })], "BOOK-A"),
      rule([cond("product", "eq", { kind: "text", text: "bond" })], "BOOK-B"),
      rule([], "DEFAULT"), // catch-all
    ];
    const round = decompileGraphToRules(compileRulesToGraph(rules));
    expect(core(round)).toEqual(core(rules));
  });

  it("round-trips two rules that target the SAME book (distinct leaves)", () => {
    const rules = [
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-A"),
      rule([cond("ccy", "eq", { kind: "text", text: "GBP" })], "BOOK-A"),
      rule([], "DEFAULT"),
    ];
    const round = decompileGraphToRules(compileRulesToGraph(rules));
    expect(core(round)).toEqual(core(rules));
  });

  it("an empty rule list compiles to an empty graph", () => {
    expect(compileRulesToGraph([])).toEqual({ entry: 0, nodes: [] });
    expect(decompileGraphToRules({ entry: 0, nodes: [] })).toEqual([]);
  });
});

describe("first-match-wins ordering (server-parity trace of the compiled graph)", () => {
  const rules = [
    rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-A"),
    rule([cond("product", "eq", { kind: "text", text: "bond" })], "BOOK-B"),
    rule([], "DEFAULT"),
  ];
  const graph = compileRulesToGraph(rules);

  it("the EARLIER rule wins when a fill matches two rules", () => {
    // Matches rule 1 (ccy=EUR) AND rule 2 (product=bond) → rule 1 wins.
    expect(traceGraph(graph, fill({ ccy: "EUR", product: "bond" })).landedBook).toBe("BOOK-A");
  });

  it("a later rule fires when the earlier does not match", () => {
    expect(traceGraph(graph, fill({ product: "bond" })).landedBook).toBe("BOOK-B");
  });

  it("the default catch-all fires when nothing matches", () => {
    expect(traceGraph(graph, fill({ ccy: "JPY" })).landedBook).toBe("DEFAULT");
  });

  it("ALL of a rule's ANDed conditions must hold (any failed leg falls through)", () => {
    const anded = compileRulesToGraph([
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" }), cond("notional", "gt", { kind: "num", num: 5e7 })], "BOOK-A"),
      rule([], "DEFAULT"),
    ]);
    expect(traceGraph(anded, fill({ ccy: "EUR", notional: 6e7 })).landedBook).toBe("BOOK-A");
    // ccy matches but notional does not → falls through to DEFAULT.
    expect(traceGraph(anded, fill({ ccy: "EUR", notional: 1e7 })).landedBook).toBe("DEFAULT");
  });
});

describe("describeRule", () => {
  const label = (id: string): string => (id === "BOOK-A" ? "RATES / A" : id);

  it("renders ANDed legs and the destination", () => {
    const r = rule(
      [cond("product", "eq", { kind: "text", text: "bond" }), cond("desk", "eq", { kind: "text", text: "marex" })],
      "BOOK-A",
    );
    const s = describeRule(r, label);
    expect(s).toContain("Product = bond");
    expect(s).toContain("AND");
    expect(s).toContain("Desk = marex");
    expect(s).toContain("→ RATES / A");
  });

  it("renders the default rule as “Otherwise → …”", () => {
    expect(describeRule(rule([], "BOOK-A"), label)).toBe("Otherwise → RATES / A");
  });
});

describe("detectRuleConflicts", () => {
  it("flags an exact-duplicate rule as an error", () => {
    const rules = [
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-A"),
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-B"),
      rule([], "DEFAULT"),
    ];
    const cs = detectRuleConflicts(rules, BOOKS);
    const dup = cs.find((c) => c.ruleId === rules[1].id && /duplicate/i.test(c.message));
    expect(dup?.severity).toBe("error");
  });

  it("flags a shadowed (subset-of-earlier) rule as unreachable", () => {
    const rules = [
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-A"),
      // more specific: superset of rule 0's conditions ⇒ rule 0 always wins first.
      rule(
        [cond("ccy", "eq", { kind: "text", text: "EUR" }), cond("product", "eq", { kind: "text", text: "bond" })],
        "BOOK-B",
      ),
      rule([], "DEFAULT"),
    ];
    const cs = detectRuleConflicts(rules, BOOKS);
    const shadow = cs.find((c) => c.ruleId === rules[1].id && /unreachable/i.test(c.message));
    expect(shadow?.severity).toBe("error");
  });

  it("flags a non-last default as shadowing everything after it", () => {
    const rules = [
      rule([], "DEFAULT"), // default not last
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-A"),
    ];
    const cs = detectRuleConflicts(rules, BOOKS);
    expect(cs.some((c) => c.ruleId === rules[1].id && c.severity === "error")).toBe(true);
  });

  it("requires exactly one default: errors when there is none", () => {
    const rules = [
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-A"),
      rule([cond("product", "eq", { kind: "text", text: "bond" })], "BOOK-B"),
    ];
    const cs = detectRuleConflicts(rules, BOOKS);
    expect(cs.some((c) => c.severity === "error" && /no default rule/i.test(c.message))).toBe(true);
  });

  it("requires exactly one default: errors when there is more than one", () => {
    const rules = [rule([], "BOOK-A"), rule([], "DEFAULT")];
    const cs = detectRuleConflicts(rules, BOOKS);
    const multi = cs.filter((c) => /multiple default/i.test(c.message));
    expect(multi.length).toBeGreaterThanOrEqual(2);
    expect(multi.every((c) => c.severity === "error")).toBe(true);
  });

  it("warns (not errors) when two rules target the same enabled book", () => {
    const rules = [
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-A"),
      rule([cond("ccy", "eq", { kind: "text", text: "GBP" })], "BOOK-A"),
      rule([], "DEFAULT"),
    ];
    const cs = detectRuleConflicts(rules, BOOKS);
    const warns = cs.filter((c) => /already routes/i.test(c.message));
    expect(warns.length).toBe(2);
    expect(warns.every((c) => c.severity === "warn")).toBe(true);
  });

  it("ignores disabled rules (they cannot conflict)", () => {
    const disabledDup: RiskRule = {
      id: newRuleId(),
      conditions: [cond("ccy", "eq", { kind: "text", text: "EUR" })],
      bookId: "BOOK-B",
      enabled: false,
    };
    const rules = [
      rule([cond("ccy", "eq", { kind: "text", text: "EUR" })], "BOOK-A"),
      disabledDup,
      rule([], "DEFAULT"),
    ];
    const cs = detectRuleConflicts(rules, BOOKS);
    expect(cs.some((c) => c.ruleId === disabledDup.id)).toBe(false);
  });
});
