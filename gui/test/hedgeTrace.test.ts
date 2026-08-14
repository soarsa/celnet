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
import {
  HEDGE_FIELD_REGISTRY,
  hedgeFieldUnprovidedReason,
} from "../src/lib/hedgeFields";
import { blankHedgeState, traceHedgeGraph, validateHedgeGraph } from "../src/lib/hedgeTrace";

/** A client whose id carries the desk's "treat as toxic" marker. */
const TOXIC_CP = "TOXIC-ALPHA";

/**
 * The doc §5.4 example policy compiled from rules.
 *
 * The "toxic flow" leg names the COUNTERPARTY rather than reading `counterparty_toxicity`:
 * nothing computes a toxicity score server-side, so that field is declared `Unprovided` and
 * a rule branching on it is refused by `HedgeGraph::validate` — and now, identically, by
 * `validateHedgeGraph`. The Rust reference graph in `tests/hedge_oracle.rs` was rewritten
 * the same way, so both sides describe one policy a desk could actually run.
 */
function examplePolicy(): HedgeGraph {
  const marketOrder = { ...defaultExitAction("submit_market_order") };
  const cross = { ...defaultExitAction("cross_internal"), instrument: "AGG-OIS" };
  const warehouse = { ...defaultExitAction("warehouse") };
  const rules: HedgeRule[] = [
    { id: newHedgeRuleId(), conditions: [{ field: "breached", op: "eq", value: { kind: "text", text: "false" } }], action: warehouse, enabled: true },
    { id: newHedgeRuleId(), conditions: [{ field: "counterparty", op: "contains", value: { kind: "text", text: "TOXIC" } }], action: marketOrder, enabled: true },
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
      counterparty: TOXIC_CP,
      overflow: 50_000,
    });
    expect(trace.landedAction?.kind).toBe("submit_market_order");
  });

  it("crosses internally when benign flow has an offset", () => {
    const trace = traceHedgeGraph(graph, {
      ...blankHedgeState(),
      breached: true,
      counterparty: "BENIGN-BETA",
      internalOffsetAvailable: 40_000,
    });
    expect(trace.landedAction?.kind).toBe("cross_internal");
  });

  it("splits when benign flow has no offset", () => {
    const trace = traceHedgeGraph(graph, {
      ...blankHedgeState(),
      breached: true,
      counterparty: "BENIGN-BETA",
      internalOffsetAvailable: 0,
    });
    expect(trace.landedAction?.kind).toBe("split");
  });
});

describe("validateHedgeGraph", () => {
  it("accepts the compiled example policy", () => {
    expect(validateHedgeGraph(examplePolicy())).toHaveLength(0);
  });

  it("refuses a condition on a field nothing populates, carrying the server's reason", () => {
    // The exact rule the divergence let through: it passed client validation and was then
    // refused by the server on save, with the trader given no way to see it coming.
    const graph: HedgeGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: {
            field: "counterparty_toxicity",
            op: "gt",
            value: { kind: "num", num: 0.6 },
            onTrue: 1,
            onFalse: 1,
          },
        },
        { kind: "action", id: 1, action: { ...defaultExitAction("warehouse") } },
      ],
    };
    const issues = validateHedgeGraph(graph);
    const dead = issues.filter((i) => i.code === "unprovided_field");
    expect(dead).toHaveLength(1);
    expect(dead[0].node).toBe(0);
    // The REASON must reach the trader, not merely the rejection — and it must be the
    // server's own words (parity with the Rust text is enforced separately).
    expect(dead[0].message).toContain(hedgeFieldUnprovidedReason("counterparty_toxicity") ?? "");
  });

  it("flags EVERY unprovided field, and no computed one", () => {
    for (const spec of HEDGE_FIELD_REGISTRY) {
      const graph: HedgeGraph = {
        entry: 0,
        nodes: [
          {
            kind: "condition",
            id: 0,
            condition: {
              field: spec.field,
              op: spec.validOps[0],
              value:
                spec.kind === "numeric" ? { kind: "num", num: 1 } : { kind: "text", text: "x" },
              onTrue: 1,
              onFalse: 1,
            },
          },
          { kind: "action", id: 1, action: { ...defaultExitAction("warehouse") } },
        ],
      };
      const dead = validateHedgeGraph(graph).some((i) => i.code === "unprovided_field");
      expect(dead, `${spec.field}`).toBe(spec.provider.state === "unprovided");
    }
  });

  it("reports an unprovided field ALONGSIDE an operator defect, not instead of it", () => {
    // The server accumulates both; a client that short-circuited would show one reason and
    // then surprise the trader with the other on the next save.
    const graph: HedgeGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          // `desk` is an ENUM: `gt` is not a legal operator for it, AND nothing populates it.
          condition: {
            field: "desk",
            op: "gt",
            value: { kind: "num", num: 1 },
            onTrue: 1,
            onFalse: 1,
          },
        },
        { kind: "action", id: 1, action: { ...defaultExitAction("warehouse") } },
      ],
    };
    const codes = validateHedgeGraph(graph).map((i) => i.code);
    expect(codes).toContain("op_not_valid_for_field");
    expect(codes).toContain("unprovided_field");
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
