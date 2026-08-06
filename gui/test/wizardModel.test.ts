/**
 * The pure guided-setup model: the dependency-ordered apply sequence (books →
 * routing → threshold → policy), the create-response id threading into the routing
 * graph + threshold scope, the stop-on-first-failure (never half-apply silently),
 * the capability-skip, and the "disabled/unknown routing target" guard the wizard
 * exists to prevent.
 */
import { describe, expect, it, vi } from "vitest";

import type { RiskBook, RiskRoutingGraph, WarehouseThreshold } from "../src/data/contract";
import { newDefaultHedgeRule } from "../src/lib/hedgeRules";
import { newRuleId, type RiskRule } from "../src/lib/riskRules";
import {
  applyWizard,
  defaultWizardThreshold,
  portfolioErrors,
  routingErrors,
  type WizardApplyTransport,
  type WizardBook,
  type WizardDraft,
} from "../src/workspaces/hedging/SetupWizard/wizardModel";

function draftWith(overrides: Partial<WizardDraft> = {}): WizardDraft {
  const book: WizardBook = {
    key: "k1",
    name: "EMEA",
    parentKey: null,
    deskId: null,
    enabled: true,
    limits: null,
  };
  const defaultRule: RiskRule = { id: newRuleId(), conditions: [], bookId: "k1", enabled: true };
  const threshold: WarehouseThreshold = { ...defaultWizardThreshold("k1") };
  return {
    books: [book],
    routingRules: [defaultRule],
    includeThreshold: true,
    threshold,
    hedgeRules: [newDefaultHedgeRule()],
    ...overrides,
  };
}

/** A transport double that records the exact call ORDER and slugs created books. */
function recordingTx(overrides: Partial<WizardApplyTransport> = {}): {
  tx: WizardApplyTransport;
  order: string[];
  updateRiskRoutingGraph: ReturnType<typeof vi.fn>;
  updateHedgeThreshold: ReturnType<typeof vi.fn>;
} {
  const order: string[] = [];
  const updateRiskRoutingGraph = vi.fn(async (g: RiskRoutingGraph) => {
    order.push("routing");
    return g;
  });
  const updateHedgeThreshold = vi.fn(async (t: WarehouseThreshold) => {
    order.push("threshold");
    return [t];
  });
  const tx: WizardApplyTransport = {
    createRiskBook: vi.fn(async (spec: RiskBook) => {
      order.push(`createRiskBook:${spec.name}`);
      return { ...spec, id: spec.id };
    }),
    updateRiskRoutingGraph,
    updateHedgeThreshold,
    updateHedgePolicyGraph: vi.fn(async (g) => {
      order.push("policy");
      return g;
    }),
    ...overrides,
  };
  return { tx, order, updateRiskRoutingGraph, updateHedgeThreshold };
}

describe("applyWizard — dependency-ordered apply", () => {
  it("commits books → routing → threshold → policy, threading the created ids", async () => {
    const { tx, order, updateRiskRoutingGraph, updateHedgeThreshold } = recordingTx();

    const result = await applyWizard(tx, draftWith(), { risk: true, hedge: true }, () => {});

    expect(result.ok).toBe(true);
    // Strict dependency order.
    expect(order).toEqual(["createRiskBook:EMEA", "routing", "threshold", "policy"]);
    // The book id the server returned (the slug, not the wizard key) is what got created.
    expect(result.createdBookIds).toEqual(["emea"]);

    // The routing graph references the REAL created id, never the local wizard key.
    const graph = updateRiskRoutingGraph.mock.calls[0]![0] as RiskRoutingGraph;
    const bookLeaf = graph.nodes.find((n) => n.kind === "book");
    expect(bookLeaf).toBeDefined();
    expect(bookLeaf && bookLeaf.kind === "book" ? bookLeaf.bookId : null).toBe("emea");
    // No dangling wizard-key reference survives into the persisted graph.
    expect(JSON.stringify(graph)).not.toContain("k1");

    // The book-scoped threshold is likewise re-pointed at the real id.
    const t = updateHedgeThreshold.mock.calls[0]![0] as WarehouseThreshold;
    expect(t.scopeId).toBe("emea");
  });

  it("stops on the first failure and never half-applies the later steps", async () => {
    const { tx, order } = recordingTx({
      updateRiskRoutingGraph: vi.fn(async () => {
        throw new Error("graph rejected: unknown target");
      }),
    });

    const result = await applyWizard(tx, draftWith(), { risk: true, hedge: true }, () => {});

    expect(result.ok).toBe(false);
    expect(result.failedStep).toBe("routing");
    // Books succeeded before the failure — surfaced, not lost.
    expect(result.createdBookIds).toEqual(["emea"]);
    // The threshold + policy RPCs were NEVER attempted.
    expect(order).toEqual(["createRiskBook:EMEA"]);
    expect(tx.updateHedgeThreshold).not.toHaveBeenCalled();
    expect(tx.updateHedgePolicyGraph).not.toHaveBeenCalled();
    const routingStep = result.steps.find((s) => s.id === "routing");
    expect(routingStep?.status).toBe("error");
  });

  it("skips the hedge half when the caller lacks the hedge capability", async () => {
    const { tx, order } = recordingTx();

    const result = await applyWizard(tx, draftWith(), { risk: true, hedge: false }, () => {});

    expect(result.ok).toBe(true);
    expect(order).toEqual(["createRiskBook:EMEA", "routing"]);
    expect(tx.updateHedgeThreshold).not.toHaveBeenCalled();
    expect(tx.updateHedgePolicyGraph).not.toHaveBeenCalled();
    expect(result.steps.find((s) => s.id === "threshold")?.status).toBe("skipped");
  });
});

describe("routingErrors — the disabled/unknown-target guard", () => {
  it("flags a rule that targets a portfolio not in the enabled step-1 set", () => {
    const rules: RiskRule[] = [
      {
        id: newRuleId(),
        conditions: [{ field: "ccy", op: "eq", value: { kind: "text", text: "EUR" } }],
        bookId: "ghost", // not an enabled key
        enabled: true,
      },
      { id: newRuleId(), conditions: [], bookId: "k1", enabled: true },
    ];
    const errs = routingErrors(rules, new Set(["k1"]));
    expect(errs.some((e) => /not one of your enabled/i.test(e))).toBe(true);
  });

  it("passes when every enabled rule targets an enabled portfolio", () => {
    const rules: RiskRule[] = [
      {
        id: newRuleId(),
        conditions: [{ field: "ccy", op: "eq", value: { kind: "text", text: "EUR" } }],
        bookId: "k1",
        enabled: true,
      },
      { id: newRuleId(), conditions: [], bookId: "k1", enabled: true },
    ];
    expect(routingErrors(rules, new Set(["k1"]))).toEqual([]);
  });
});

describe("portfolioErrors — step 1 gating", () => {
  it("requires at least one enabled, named portfolio", () => {
    expect(portfolioErrors([]).length).toBeGreaterThan(0);
    const disabled: WizardBook = {
      key: "k1",
      name: "EMEA",
      parentKey: null,
      deskId: null,
      enabled: false,
      limits: null,
    };
    expect(portfolioErrors([disabled]).some((e) => /enable at least one/i.test(e))).toBe(true);
    const ok: WizardBook = { ...disabled, enabled: true };
    expect(portfolioErrors([ok])).toEqual([]);
  });
});
