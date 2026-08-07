/**
 * RiskSetupWizard — the Risk guided-setup flow, driven with `useApp` mocked (no server).
 * Covers: the four steps render and advance; the acceptance step renders the seeded
 * accept/reject/hold builder; an end-to-end Apply firing the three existing RPCs
 * (createRiskBook → updateRiskRoutingGraph → updateAcceptanceGraph) in dependency order
 * with the step-1 book id threaded into the routing graph; landing on the Acceptance
 * tab; and per-step capability gating.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { AcceptanceGraph, RiskBook, RiskRoutingGraph } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { RiskSetupWizard } from "../src/workspaces/risksetup/RiskSetupWizard";

function makeApp(opts: { canRisk?: boolean; canAcceptance?: boolean } = {}) {
  const order: string[] = [];
  const createRiskBook = vi.fn(async (spec: RiskBook) => {
    order.push(`createRiskBook:${spec.name}`);
    return { ...spec, id: spec.id || "srv" };
  });
  const updateRiskRoutingGraph = vi.fn(async (g: RiskRoutingGraph) => {
    order.push("routing");
    return g;
  });
  const updateAcceptanceGraph = vi.fn(async (g: AcceptanceGraph) => {
    order.push("acceptance");
    return g;
  });
  const setWorkspace = vi.fn();
  return {
    order,
    createRiskBook,
    updateRiskRoutingGraph,
    updateAcceptanceGraph,
    setWorkspace,
    app: {
      transport: {
        listDesks: vi.fn(async () => []),
        listFixConnections: vi.fn(async () => []),
        createRiskBook,
        updateRiskRoutingGraph,
        updateAcceptanceGraph,
      },
      auth: {
        user: { id: "u", email: "risk@celnet.com" },
        can: (action: string) =>
          action === "risk_manage"
            ? (opts.canRisk ?? true)
            : action === "manage_acceptance"
              ? (opts.canAcceptance ?? true)
              : true,
      },
      setWorkspace,
    },
  };
}

/** Set a <select> to the option whose visible text is `label`. */
function selectByLabel(selectEl: HTMLElement, label: string): void {
  const opt = within(selectEl).getByRole("option", { name: label }) as HTMLOptionElement;
  fireEvent.change(selectEl, { target: { value: opt.value } });
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("RiskSetupWizard — flow", () => {
  it("renders all four steps, including the acceptance builder, and advances", async () => {
    state.app = makeApp().app;
    render(<RiskSetupWizard onClose={vi.fn()} />);

    // Step 1 — portfolios (shared).
    expect(await screen.findByTestId("wiz-portfolio-0")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("wiz-portfolio-name-0"), { target: { value: "EMEA" } });
    fireEvent.click(screen.getByTestId("wiz-next"));

    // Step 2 — routing (shared).
    expect(await screen.findByTestId("wiz-routing-list")).toBeInTheDocument();
    selectByLabel(screen.getByTestId("wiz-default-target"), "EMEA");
    fireEvent.click(screen.getByTestId("wiz-next"));

    // Step 3 — acceptance (risk-specific): the seeded reject rule + the accept-all default.
    expect(await screen.findByTestId("risk-wiz-acceptance")).toBeInTheDocument();
    expect(screen.getByTestId("risk-wiz-acc-rule-0")).toBeInTheDocument();
    expect(screen.getByTestId("risk-wiz-acc-default")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("wiz-next"));

    // Step 4 — review.
    expect(await screen.findByTestId("risk-wiz-review-acceptance")).toBeInTheDocument();
    expect(screen.getByTestId("wiz-apply")).toBeInTheDocument();
  });

  it("applies by calling the three RPCs in order with the created id threaded into routing", async () => {
    const h = makeApp();
    state.app = h.app;
    render(<RiskSetupWizard onClose={vi.fn()} />);

    fireEvent.change(await screen.findByTestId("wiz-portfolio-name-0"), { target: { value: "EMEA" } });
    fireEvent.click(screen.getByTestId("wiz-next"));
    selectByLabel(await screen.findByTestId("wiz-default-target"), "EMEA");
    fireEvent.click(screen.getByTestId("wiz-next"));
    // Acceptance step — keep the seeded valid policy, advance.
    await screen.findByTestId("risk-wiz-acceptance");
    fireEvent.click(screen.getByTestId("wiz-next"));

    fireEvent.click(await screen.findByTestId("wiz-apply"));

    await waitFor(() => expect(h.updateAcceptanceGraph).toHaveBeenCalled());
    // Dependency order: books first, then routing, then acceptance.
    expect(h.order).toEqual(["createRiskBook:EMEA", "routing", "acceptance"]);

    // The routing graph references the server-minted id ("emea"), not a wizard key.
    const graph = h.updateRiskRoutingGraph.mock.calls[0]![0] as RiskRoutingGraph;
    const leaf = graph.nodes.find((n) => n.kind === "book");
    expect(leaf && leaf.kind === "book" ? leaf.bookId : null).toBe("emea");

    // The acceptance graph is the compiled decision tree (a condition + decision leaves).
    const acc = h.updateAcceptanceGraph.mock.calls[0]![0] as AcceptanceGraph;
    expect(acc.nodes.some((n) => n.kind === "condition")).toBe(true);

    // On success the trader lands on the Risk → Acceptance tab.
    expect(h.setWorkspace).toHaveBeenCalledWith("acceptance");
  });

  it("adds an acceptance rule via the builder", async () => {
    state.app = makeApp().app;
    render(<RiskSetupWizard onClose={vi.fn()} />);
    // Jump to the acceptance step via the stepper.
    fireEvent.click(await screen.findByTestId("wiz-step-2"));
    expect(await screen.findByTestId("risk-wiz-acc-rule-0")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("risk-wiz-add-acc-rule"));
    // A second specific rule now precedes the catch-all.
    expect(await screen.findByTestId("risk-wiz-acc-rule-1")).toBeInTheDocument();
  });
});

describe("RiskSetupWizard — capability gating", () => {
  it("makes portfolios + routing read-only without risk_manage", async () => {
    state.app = makeApp({ canRisk: false }).app;
    render(<RiskSetupWizard onClose={vi.fn()} />);

    expect(await screen.findByTestId("wiz-cap-banner")).toBeInTheDocument();
    expect(screen.queryByTestId("wiz-add-portfolio")).toBeNull();
    expect(within(screen.getByTestId("wiz-step-0")).getByText("read-only")).toBeInTheDocument();
  });

  it("makes the acceptance step read-only without manage_acceptance", async () => {
    state.app = makeApp({ canAcceptance: false }).app;
    render(<RiskSetupWizard onClose={vi.fn()} />);

    fireEvent.click(await screen.findByTestId("wiz-step-2"));
    await screen.findByTestId("risk-wiz-acceptance");
    // No editing affordance on the read-only acceptance step.
    expect(screen.queryByTestId("risk-wiz-add-acc-rule")).toBeNull();
    expect(within(screen.getByTestId("wiz-step-2")).getByText("read-only")).toBeInTheDocument();
  });
});
