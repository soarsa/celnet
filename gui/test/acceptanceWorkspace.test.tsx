/**
 * AcceptanceWorkspace — the incoming-quote-acceptance rule builder driven with `useApp`
 * mocked (no server). Covers: the policy loads into the rules table and opens the
 * per-rule editor with the decision leaf; Save compiles the rules and calls
 * `updateAcceptanceGraph`; the whole surface is read-only without the `manage_acceptance`
 * capability; and the live trace panel resolves a decision.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { AcceptanceGraph } from "../src/data/contract";
import {
  compileRulesToAcceptanceGraph,
  decompileAcceptanceGraphToRules,
  newAcceptanceRuleId,
  type AcceptanceRule,
} from "../src/lib/acceptanceRules";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { AcceptanceWorkspace } from "../src/workspaces/acceptance/AcceptanceWorkspace";
import { at } from "./support";

function policyGraph(): AcceptanceGraph {
  const rules: AcceptanceRule[] = [
    {
      id: newAcceptanceRuleId(),
      conditions: [{ field: "edge_bps", op: "lt", value: { kind: "num", num: 0.5 } }],
      action: { kind: "reject", reason: "below edge floor" },
      enabled: true,
    },
    { id: newAcceptanceRuleId(), conditions: [], action: { kind: "accept", reason: "" }, enabled: true },
  ];
  return compileRulesToAcceptanceGraph(rules);
}

function makeApp(opts: { canEdit?: boolean; graph?: AcceptanceGraph | null } = {}) {
  const updateAcceptanceGraph = vi.fn(async (g: AcceptanceGraph) => g);
  return {
    app: {
      transport: {
        getAcceptanceGraph: vi.fn(async () => (opts.graph !== undefined ? opts.graph : policyGraph())),
        updateAcceptanceGraph,
      },
      auth: {
        user: { id: "u", email: "acc@celnet.com" },
        isAdmin: true,
        can: () => opts.canEdit ?? true,
      },
      setSignInOpen: vi.fn(),
    },
    updateAcceptanceGraph,
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("AcceptanceWorkspace — rules", () => {
  it("loads the policy into the rules table and opens the decision editor", async () => {
    state.app = makeApp().app;
    render(<AcceptanceWorkspace />);

    // The seeded policy decompiles to two rows (specific reject + default accept).
    expect(await screen.findByTestId("acceptance-rules-table")).toBeInTheDocument();
    expect(await screen.findByTestId("acceptance-rule-row-0")).toBeInTheDocument();
    expect(screen.getByTestId("acceptance-rule-decision-0")).toHaveTextContent("Reject");

    fireEvent.click(screen.getByTestId("acceptance-create-rule"));
    expect(await screen.findByTestId("acceptance-rule-editor")).toBeInTheDocument();
    // The leaf editor is the acceptance DECISION picker (accept/reject/hold).
    expect(screen.getByTestId("acceptance-action-kind")).toBeInTheDocument();
  });

  it("renders the live trace panel with a resolved decision", async () => {
    state.app = makeApp().app;
    render(<AcceptanceWorkspace />);
    expect(await screen.findByTestId("acceptance-trace")).toBeInTheDocument();
    expect(screen.getByTestId("trace-decision")).toBeInTheDocument();
    expect(screen.getByTestId("trace-action")).toBeInTheDocument();
  });

  it("seeds a single accept-all rule when no policy exists yet", async () => {
    state.app = makeApp({ graph: null }).app;
    render(<AcceptanceWorkspace />);
    expect(await screen.findByTestId("acceptance-rule-row-0")).toBeInTheDocument();
    expect(screen.getByTestId("acceptance-rule-decision-0")).toHaveTextContent("Accept");
    // Nothing to save on a pristine seed.
    expect(screen.getByTestId("acceptance-save-policy")).toBeDisabled();
  });

  it("compiles + saves via updateAcceptanceGraph after a mutation", async () => {
    const { app, updateAcceptanceGraph } = makeApp();
    state.app = app;
    render(<AcceptanceWorkspace />);

    // Disable the first (specific) rule → the policy is now dirty and savable.
    fireEvent.click(await screen.findByTestId("acceptance-rule-toggle-0"));
    const save = screen.getByTestId("acceptance-save-policy");
    await waitFor(() => expect(save).not.toBeDisabled());
    fireEvent.click(save);

    await waitFor(() => expect(updateAcceptanceGraph).toHaveBeenCalledTimes(1));
    const committed = at(at(updateAcceptanceGraph.mock.calls, 0), 0) as AcceptanceGraph;
    // With the specific rule disabled, the compiled graph is the single accept default.
    const back = decompileAcceptanceGraphToRules(committed);
    expect(back).toHaveLength(1);
    expect(back[0]?.action.kind).toBe("accept");
  });
});

describe("AcceptanceWorkspace — capability gating", () => {
  it("is read-only without the manage_acceptance capability", async () => {
    state.app = makeApp({ canEdit: false }).app;
    render(<AcceptanceWorkspace />);
    // The rules table still renders, but no create/save affordance exists.
    expect(await screen.findByTestId("acceptance-rules-table")).toBeInTheDocument();
    expect(screen.queryByTestId("acceptance-create-rule")).toBeNull();
    expect(screen.queryByTestId("acceptance-save-policy")).toBeNull();
  });
});
