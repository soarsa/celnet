/**
 * SetupWizard — the Hedging guided-setup flow, driven with `useApp` mocked (no
 * server). Covers: the four steps render and advance; per-step capability gating
 * (portfolio/routing read-only without risk_manage, hedge step read-only without
 * hedge); and an end-to-end Apply firing the four existing RPCs in dependency order
 * with the step-1 book id threaded into the routing graph.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { RiskBook, RiskRoutingGraph } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { SetupWizard } from "../src/workspaces/hedging/SetupWizard/SetupWizard";

function makeApp(opts: { canRisk?: boolean; canHedge?: boolean } = {}) {
  const order: string[] = [];
  const createRiskBook = vi.fn(async (spec: RiskBook) => {
    order.push(`createRiskBook:${spec.name}`);
    return { ...spec, id: spec.id || "srv" };
  });
  const updateRiskRoutingGraph = vi.fn(async (g: RiskRoutingGraph) => {
    order.push("routing");
    return g;
  });
  const updateHedgeThreshold = vi.fn(async (t) => {
    order.push("threshold");
    return [t];
  });
  const updateHedgePolicyGraph = vi.fn(async (g) => {
    order.push("policy");
    return g;
  });
  const setWorkspace = vi.fn();
  return {
    order,
    createRiskBook,
    updateRiskRoutingGraph,
    updateHedgeThreshold,
    updateHedgePolicyGraph,
    setWorkspace,
    app: {
      transport: {
        listDesks: vi.fn(async () => []),
        listFixConnections: vi.fn(async () => []),
        createRiskBook,
        updateRiskRoutingGraph,
        updateHedgeThreshold,
        updateHedgePolicyGraph,
      },
      auth: {
        user: { id: "u", email: "trader@celnet.com" },
        can: (action: string) =>
          action === "risk_manage" ? (opts.canRisk ?? true) : (opts.canHedge ?? true),
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

describe("SetupWizard — flow", () => {
  it("renders all four steps and advances through them", async () => {
    state.app = makeApp().app;
    render(<SetupWizard onClose={vi.fn()} />);

    // Step 1 — portfolios.
    expect(await screen.findByTestId("wiz-portfolio-0")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("wiz-portfolio-name-0"), { target: { value: "EMEA" } });
    fireEvent.click(screen.getByTestId("wiz-next"));

    // Step 2 — routing.
    expect(await screen.findByTestId("wiz-routing-list")).toBeInTheDocument();
    selectByLabel(screen.getByTestId("wiz-default-target"), "EMEA");
    fireEvent.click(screen.getByTestId("wiz-next"));

    // Step 3 — internalise & hedge.
    expect(await screen.findByTestId("wiz-threshold-form")).toBeInTheDocument();
    selectByLabel(screen.getByTestId("wiz-threshold-scope"), "EMEA");
    fireEvent.click(screen.getByTestId("wiz-next"));

    // Step 4 — review.
    expect(await screen.findByTestId("wiz-review-portfolios")).toBeInTheDocument();
    expect(screen.getByTestId("wiz-apply")).toBeInTheDocument();
  });

  it("applies by calling the four RPCs in order with the created id threaded into routing", async () => {
    const h = makeApp();
    state.app = h.app;
    render(<SetupWizard onClose={vi.fn()} />);

    fireEvent.change(await screen.findByTestId("wiz-portfolio-name-0"), { target: { value: "EMEA" } });
    fireEvent.click(screen.getByTestId("wiz-next"));
    selectByLabel(await screen.findByTestId("wiz-default-target"), "EMEA");
    fireEvent.click(screen.getByTestId("wiz-next"));
    selectByLabel(await screen.findByTestId("wiz-threshold-scope"), "EMEA");
    fireEvent.click(screen.getByTestId("wiz-next"));

    fireEvent.click(await screen.findByTestId("wiz-apply"));

    await waitFor(() => expect(h.updateHedgePolicyGraph).toHaveBeenCalled());
    // Dependency order: books first, then routing, threshold, policy.
    expect(h.order).toEqual(["createRiskBook:EMEA", "routing", "threshold", "policy"]);

    // The routing graph references the server-minted id ("emea"), not a wizard key.
    const graph = h.updateRiskRoutingGraph.mock.calls[0]![0] as RiskRoutingGraph;
    const leaf = graph.nodes.find((n) => n.kind === "book");
    expect(leaf && leaf.kind === "book" ? leaf.bookId : null).toBe("emea");

    // On success the trader lands on the Risk Dashboard.
    expect(h.setWorkspace).toHaveBeenCalledWith("riskdashboard");
  });
});

describe("SetupWizard — capability gating", () => {
  it("makes portfolios + routing read-only without risk_manage", async () => {
    state.app = makeApp({ canRisk: false }).app;
    render(<SetupWizard onClose={vi.fn()} />);

    expect(await screen.findByTestId("wiz-cap-banner")).toBeInTheDocument();
    // No editing affordance on the read-only portfolio step.
    expect(screen.queryByTestId("wiz-add-portfolio")).toBeNull();
    // The stepper marks the gated steps read-only.
    expect(within(screen.getByTestId("wiz-step-0")).getByText("read-only")).toBeInTheDocument();
  });

  it("makes the internalise & hedge step read-only without hedge", async () => {
    state.app = makeApp({ canHedge: false }).app;
    render(<SetupWizard onClose={vi.fn()} />);

    // Jump straight to the hedge step via the stepper.
    fireEvent.click(await screen.findByTestId("wiz-step-2"));
    expect(await screen.findByTestId("wiz-include-threshold")).toBeDisabled();
    expect(screen.queryByTestId("wiz-add-hedge-rule")).toBeNull();
  });
});
