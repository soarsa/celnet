/**
 * The Risk guided-setup model: the dependency-ordered apply sequence (books → routing →
 * acceptance), the create-response id threading into the routing graph, the
 * stop-on-first-failure (never half-apply silently), the capability-skip of the
 * acceptance half, and the acceptance-step validation. Exercises the SHARED apply
 * engine (`applyWizardCore`) through the risk wrapper, so the books+routing spine and
 * the acceptance branch are proven together with consistent ids.
 */
import { describe, expect, it, vi } from "vitest";

import type { AcceptanceGraph, RiskBook, RiskRoutingGraph } from "../src/data/contract";
import { newRuleId, type RiskRule } from "../src/lib/riskRules";
import {
  acceptanceErrors,
  applyRiskWizard,
  defaultAcceptanceRules,
  type RiskWizardApplyTransport,
  type RiskWizardDraft,
} from "../src/workspaces/risksetup/riskWizardModel";
import type { WizardBook } from "../src/workspaces/setupWizard/wizardModel";

function draftWith(overrides: Partial<RiskWizardDraft> = {}): RiskWizardDraft {
  const book: WizardBook = {
    key: "k1",
    name: "EMEA",
    parentKey: null,
    deskId: null,
    enabled: true,
    limits: null,
  };
  const defaultRule: RiskRule = { id: newRuleId(), conditions: [], bookId: "k1", enabled: true };
  return {
    books: [book],
    routingRules: [defaultRule],
    acceptanceRules: defaultAcceptanceRules(),
    ...overrides,
  };
}

/** A transport double that records the exact call ORDER and slugs created books. */
function recordingTx(overrides: Partial<RiskWizardApplyTransport> = {}): {
  tx: RiskWizardApplyTransport;
  order: string[];
  updateRiskRoutingGraph: ReturnType<typeof vi.fn>;
  updateAcceptanceGraph: ReturnType<typeof vi.fn>;
} {
  const order: string[] = [];
  const updateRiskRoutingGraph = vi.fn(async (g: RiskRoutingGraph) => {
    order.push("routing");
    return g;
  });
  const updateAcceptanceGraph = vi.fn(async (g: AcceptanceGraph) => {
    order.push("acceptance");
    return g;
  });
  const tx: RiskWizardApplyTransport = {
    createRiskBook: vi.fn(async (spec: RiskBook) => {
      order.push(`createRiskBook:${spec.name}`);
      return { ...spec, id: spec.id };
    }),
    updateRiskRoutingGraph,
    updateAcceptanceGraph,
    ...overrides,
  };
  return { tx, order, updateRiskRoutingGraph, updateAcceptanceGraph };
}

describe("applyRiskWizard — dependency-ordered apply", () => {
  it("commits books → routing → acceptance, threading the created ids consistently", async () => {
    const { tx, order, updateRiskRoutingGraph, updateAcceptanceGraph } = recordingTx();

    const result = await applyRiskWizard(tx, draftWith(), { risk: true, acceptance: true }, () => {});

    expect(result.ok).toBe(true);
    // Strict dependency order: books first (mint ids), then routing, then acceptance.
    expect(order).toEqual(["createRiskBook:EMEA", "routing", "acceptance"]);
    expect(result.createdBookIds).toEqual(["emea"]);

    // The routing graph references the REAL created id, never the local wizard key.
    const graph = updateRiskRoutingGraph.mock.calls[0]![0] as RiskRoutingGraph;
    const bookLeaf = graph.nodes.find((n) => n.kind === "book");
    expect(bookLeaf && bookLeaf.kind === "book" ? bookLeaf.bookId : null).toBe("emea");
    expect(JSON.stringify(graph)).not.toContain("k1");

    // The acceptance graph is the compiled decision tree — a condition node (edge_bps)
    // plus reject/accept decision leaves.
    const acc = updateAcceptanceGraph.mock.calls[0]![0] as AcceptanceGraph;
    expect(acc.nodes.some((n) => n.kind === "condition")).toBe(true);
    expect(acc.nodes.filter((n) => n.kind === "decision").length).toBeGreaterThanOrEqual(2);
  });

  it("stops on the first failure and never writes the acceptance policy", async () => {
    const { tx, order } = recordingTx({
      updateRiskRoutingGraph: vi.fn(async () => {
        throw new Error("graph rejected: unknown target");
      }),
    });

    const result = await applyRiskWizard(tx, draftWith(), { risk: true, acceptance: true }, () => {});

    expect(result.ok).toBe(false);
    expect(result.failedStep).toBe("routing");
    expect(result.createdBookIds).toEqual(["emea"]);
    expect(order).toEqual(["createRiskBook:EMEA"]);
    expect(tx.updateAcceptanceGraph).not.toHaveBeenCalled();
    // Acceptance was entitled + would-run, but the sequence stopped at routing before
    // reaching it — so it is left untouched at `pending`, never run.
    expect(result.steps.find((s) => s.id === "acceptance")?.status).toBe("pending");
  });

  it("skips the acceptance half when the caller lacks manage_acceptance", async () => {
    const { tx, order } = recordingTx();

    const result = await applyRiskWizard(tx, draftWith(), { risk: true, acceptance: false }, () => {});

    expect(result.ok).toBe(true);
    expect(order).toEqual(["createRiskBook:EMEA", "routing"]);
    expect(tx.updateAcceptanceGraph).not.toHaveBeenCalled();
    expect(result.steps.find((s) => s.id === "acceptance")?.status).toBe("skipped");
  });
});

describe("acceptanceErrors — step-3 gating", () => {
  it("passes the seeded default policy (reject-below-edge + accept-all)", () => {
    expect(acceptanceErrors(defaultAcceptanceRules())).toEqual([]);
  });

  it("flags a policy with no default (catch-all) rule", () => {
    const specificOnly = defaultAcceptanceRules().filter((r) => r.conditions.length > 0);
    expect(acceptanceErrors(specificOnly).length).toBeGreaterThan(0);
  });
});
