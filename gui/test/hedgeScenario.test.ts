/**
 * The Exit-Policy guided-rule wizard model (`lib/hedgeScenario`): scenario presets
 * generate VALID first-match-wins graphs (each round-trips through the compiler and
 * passes `validateHedgeGraph` + the conflict checks), the DV01-vs-notional metric is
 * carried with its correct unit, and the flow analysis derives HONEST suggestions from
 * a book's risk-store row + client-blotter deals (and says so when there is no flow).
 */
import { describe, expect, it } from "vitest";

import {
  METRIC_DESCRIPTORS,
  SCENARIOS,
  buildScenarioRules,
  defaultScenarioParams,
  metricMagnitude,
  niceRoundUp,
  scenarioParamErrors,
  summariseBookFlow,
  suggestThresholds,
  type ScenarioParams,
} from "../src/lib/hedgeScenario";
import {
  compileRulesToHedgeGraph,
  decompileHedgeGraphToRules,
  describeHedgeRule,
  detectHedgeRuleConflicts,
} from "../src/lib/hedgeRules";
import { validateHedgeGraph } from "../src/lib/hedgeTrace";
import type { Deal, RiskBookRisk } from "../src/data/contract";

const params = (over: Partial<ScenarioParams> = {}): ScenarioParams => ({
  ...defaultScenarioParams(),
  metric: "net_notional",
  threshold: 250_000_000,
  escalateThreshold: 500_000_000,
  counterparty: "CITADEL",
  ...over,
});

describe("metric descriptors carry the honest unit", () => {
  it("net notional is a $ face metric; net DV01 is a $/bp risk metric", () => {
    expect(METRIC_DESCRIPTORS.net_notional.unit).toBe("$");
    expect(METRIC_DESCRIPTORS.net_dv01.unit).toBe("$/bp");
    // The two must never share a unit — that conflation is the historical bug.
    expect(METRIC_DESCRIPTORS.net_notional.unit).not.toBe(METRIC_DESCRIPTORS.net_dv01.unit);
  });
});

describe("every scenario generates a valid, conflict-free graph", () => {
  for (const spec of SCENARIOS) {
    it(`${spec.id} compiles to a graph the server accepts`, () => {
      const rules = buildScenarioRules(spec.id, params());
      // Exactly one trailing default (catch-all) rule.
      expect(rules.filter((r) => r.conditions.length === 0)).toHaveLength(1);
      expect(rules[rules.length - 1]?.conditions).toHaveLength(0);
      // Compiles to a structurally valid graph.
      const graph = compileRulesToHedgeGraph(rules.filter((r) => r.enabled));
      expect(validateHedgeGraph(graph)).toEqual([]);
      // No error-severity rule conflicts.
      expect(detectHedgeRuleConflicts(rules).filter((c) => c.severity === "error")).toEqual([]);
      // Round-trips through the compiler/decompiler (same rule count).
      expect(decompileHedgeGraphToRules(graph)).toHaveLength(rules.length);
    });
  }
});

describe("warehouse-then-flatten", () => {
  it("thresholds the chosen metric and flattens above it, holding otherwise", () => {
    const rules = buildScenarioRules("warehouse-then-flatten", params({ metric: "net_notional", threshold: 250_000_000 }));
    expect(rules).toHaveLength(2);
    const [flatten, dflt] = rules;
    expect(flatten?.conditions[0]?.field).toBe("net_notional");
    expect(flatten?.conditions[0]?.op).toBe("gt");
    expect(flatten?.action.kind).toBe("submit_market_order");
    expect(flatten?.action.size.kind).toBe("full"); // flatten, not just the overflow
    expect(dflt?.action.kind).toBe("warehouse");
  });

  it("honours a net_dv01 metric", () => {
    const rules = buildScenarioRules("warehouse-then-flatten", params({ metric: "net_dv01", threshold: 5000 }));
    expect(rules[0]?.conditions[0]?.field).toBe("net_dv01");
  });
});

describe("tiered-escalate orders the bigger threshold first (first-match-wins)", () => {
  it("escalation rule precedes the skew rule so neither is shadowed", () => {
    const rules = buildScenarioRules("tiered-escalate", params({ threshold: 250_000_000, escalateThreshold: 500_000_000 }));
    expect(rules).toHaveLength(3);
    expect(rules[0]?.action.kind).toBe("submit_market_order");
    expect(rules[1]?.action.kind).toBe("skew");
    expect(rules[2]?.action.kind).toBe("warehouse");
    // The generated graph is genuinely unshadowed.
    expect(detectHedgeRuleConflicts(rules).filter((c) => c.severity === "error")).toEqual([]);
  });
});

describe("b2b-counterparty", () => {
  it("matches the named counterparty and externalises it", () => {
    const rules = buildScenarioRules("b2b-counterparty", params({ counterparty: "CITADEL" }));
    expect(rules[0]?.conditions[0]?.field).toBe("counterparty");
    expect(describeHedgeRule(rules[0]!)).toContain("CITADEL");
    expect(rules[0]?.action.kind).toBe("submit_market_order");
  });
});

describe("pure-internalise is a single warehouse catch-all", () => {
  it("holds everything", () => {
    const rules = buildScenarioRules("pure-internalise", params());
    expect(rules).toHaveLength(1);
    expect(rules[0]?.conditions).toHaveLength(0);
    expect(rules[0]?.action.kind).toBe("warehouse");
  });
});

describe("scenarioParamErrors gates parameters honestly", () => {
  it("requires a positive threshold for the metric scenarios", () => {
    expect(scenarioParamErrors("warehouse-then-flatten", params({ threshold: 0 }))).toHaveLength(1);
    expect(scenarioParamErrors("warehouse-then-flatten", params({ threshold: 1 }))).toHaveLength(0);
  });
  it("requires the escalation threshold to exceed the first", () => {
    expect(scenarioParamErrors("tiered-escalate", params({ threshold: 500, escalateThreshold: 250 }))).not.toEqual([]);
    expect(scenarioParamErrors("tiered-escalate", params({ threshold: 250, escalateThreshold: 500 }))).toEqual([]);
  });
  it("requires a counterparty for the back-to-back scenario", () => {
    expect(scenarioParamErrors("b2b-counterparty", params({ counterparty: "  " }))).not.toEqual([]);
  });
  it("pure-internalise needs no parameters", () => {
    expect(scenarioParamErrors("pure-internalise", params({ threshold: 0, counterparty: "" }))).toEqual([]);
  });
});

describe("niceRoundUp rounds to a 1/2/5 × 10ⁿ figure", () => {
  it("rounds up sensibly", () => {
    expect(niceRoundUp(187_000_000)).toBe(200_000_000);
    expect(niceRoundUp(43_000)).toBe(50_000);
    expect(niceRoundUp(9_000_000)).toBe(10_000_000);
    expect(niceRoundUp(1_500_000)).toBe(2_000_000);
    expect(niceRoundUp(0)).toBe(0);
  });
});

// --- flow analysis ---------------------------------------------------------

const book = (over: Partial<RiskBookRisk> = {}): RiskBookRisk => ({
  bookId: "DEFAULT_BOOK",
  name: "Default book",
  netNotional: 0,
  grossNotional: 0,
  positionCount: 0,
  delta: 0,
  gamma: 0,
  vega: 0,
  theta: 0,
  dv01: null,
  pnl: null,
  limits: [],
  ...over,
});

const deal = (over: Partial<Deal>): Deal =>
  ({
    dealId: "d-1",
    requestId: "r-1",
    kind: "RFQ",
    counterparty: "CP",
    desk: "g10-rates",
    productKind: "OIS",
    instrument: {} as Deal["instrument"],
    curveSet: { currency: "USD" } as Deal["curveSet"],
    side: "BUY",
    notional: 100_000_000,
    price: 100,
    executedAtNanos: 0n,
    trader: "t",
    riskBookId: "DEFAULT_BOOK",
    ...over,
  }) as Deal;

describe("summariseBookFlow reads the risk store + client blotter", () => {
  it("derives net DV01, net notional, deal count and peak utilisation for the book", () => {
    const s = summariseBookFlow(
      book({
        netNotional: 250_000_000,
        grossNotional: 400_000_000,
        positionCount: 6,
        dv01: 5_000,
        limits: [
          { metric: "net_notional", used: 250_000_000, limit: 500_000_000, fraction: 0.5, band: "green" },
          { metric: "gross_notional", used: 400_000_000, limit: 500_000_000, fraction: 0.8, band: "amber" },
        ],
      }),
      [deal({ side: "BUY", notional: 150_000_000 }), deal({ dealId: "d-2", side: "SELL", notional: 50_000_000 })],
    );
    expect(s.netDv01).toBe(5_000);
    expect(s.netNotional).toBe(250_000_000);
    expect(s.dealCount).toBe(2);
    expect(s.dealSignedNotional).toBe(100_000_000); // +150m − 50m
    expect(s.peakUtilizationPct).toBe(80);
    expect(s.hasFlow).toBe(true);
  });

  it("filters deals to the book by riskBookId", () => {
    const s = summariseBookFlow(book({ positionCount: 1 }), [
      deal({ riskBookId: "OTHER" }),
      deal({ dealId: "d-2", riskBookId: "DEFAULT_BOOK" }),
    ]);
    expect(s.dealCount).toBe(1);
  });

  it("reports no flow honestly when the book is empty", () => {
    const s = summariseBookFlow(book(), []);
    expect(s.hasFlow).toBe(false);
  });
});

describe("suggestThresholds never fabricates a number", () => {
  it("returns null when there is no flow", () => {
    const s = summariseBookFlow(book(), []);
    expect(suggestThresholds(s, "net_notional")).toBeNull();
  });

  it("returns null for a metric that is not evaluable (DV01 null)", () => {
    const s = summariseBookFlow(book({ netNotional: 250_000_000, positionCount: 3, dv01: null }), []);
    expect(suggestThresholds(s, "net_dv01")).toBeNull();
    expect(suggestThresholds(s, "net_notional")).not.toBeNull();
  });

  it("suggests a rounded cap with headroom and a flatten trigger at 0.8× cap", () => {
    const s = summariseBookFlow(book({ netNotional: 250_000_000, positionCount: 3 }), []);
    const sug = suggestThresholds(s, "net_notional");
    expect(sug).not.toBeNull();
    // 250m × 1.25 = 312.5m → niceRoundUp → 500m; flatten = 0.8 × 500m = 400m.
    expect(sug?.cap).toBe(500_000_000);
    expect(sug?.flattenThreshold).toBe(400_000_000);
    expect(sug?.current).toBe(250_000_000);
  });

  it("uses the absolute DV01 magnitude for a short book", () => {
    const s = summariseBookFlow(book({ netNotional: -80_000_000, positionCount: 2, dv01: -4_000 }), []);
    expect(metricMagnitude(s, "net_dv01")).toBe(4_000);
    const sug = suggestThresholds(s, "net_dv01");
    // 4000 × 1.25 = 5000 → niceRoundUp → 5000; flatten 4000.
    expect(sug?.cap).toBe(5_000);
    expect(sug?.flattenThreshold).toBe(4_000);
  });
});
