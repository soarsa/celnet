/**
 * HedgingWorkspace — the auto-hedge surface driven with `useApp` mocked (no server).
 * Covers: the Exit-Policy tab loads a policy into the rules table and opens the
 * per-rule editor with the exit-action leaf; the Thresholds tab renders the roster +
 * form; the Execution mode tab renders the engine kill-switch + execution-mode config
 * (the live flow monitor moved to Risk → Hedge flows, asserted absent here); and the
 * whole surface is read-only without the `hedge` capability.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import type {
  HedgeConfig,
  HedgeGraph,
  HedgeIntent,
  HedgeProvenance,
  WarehouseThreshold,
} from "../src/data/contract";
import { compileRulesToHedgeGraph, newHedgeRuleId, type HedgeRule } from "../src/lib/hedgeRules";
import { defaultExitAction } from "../src/lib/hedgeExit";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { HedgingWorkspace } from "../src/workspaces/hedging/HedgingWorkspace";
import { at } from "./support";

function policyGraph(): HedgeGraph {
  const rules: HedgeRule[] = [
    {
      id: newHedgeRuleId(),
      conditions: [{ field: "breached", op: "eq", value: { kind: "text", text: "true" } }],
      action: { ...defaultExitAction("submit_market_order") },
      enabled: true,
    },
    { id: newHedgeRuleId(), conditions: [], action: { ...defaultExitAction("warehouse") }, enabled: true },
  ];
  return compileRulesToHedgeGraph(rules);
}

const threshold: WarehouseThreshold = {
  scopeKind: "book",
  scopeId: "fi-rates-emea",
  metric: "dv01",
  cap: 250_000,
  amber: 0.7,
  red: 0.9,
  targetFraction: 0.7,
  minClip: 1_000,
  maxClip: 100_000,
  ramped: false,
  rampK: 0,
};

const config: HedgeConfig = {
  killSwitch: false,
  execution: "advisory",
  deskEnabled: [{ desk: "emea", enabled: true }],
  maxClip: 1_000_000,
  maxHedgesPerInterval: 20,
  dailyExternalNotionalCap: 1_000_000_000,
  lpPanels: [
    { scopeKind: "book", scopeId: "fi-rates-emea", include: ["LP-1", "LP-2", "LP-3"], exclude: ["LP-2"] },
  ],
  // no per-scope bindings: every scope takes its documented default —
  // exit mode `auto` (contract.ts:4848-4850), model CUSTOM (:4853-4856).
  hedgingModels: [],
  exitModes: [],
  vehicles: [],
  compositeSpreadBp: 0.5,
};

const advisoryIntent: HedgeIntent = {
  book: "fi-rates-emea",
  instrument: "US10Y",
  action: { ...defaultExitAction("submit_market_order") },
  // `self` vehicle ⇒ no unit arithmetic to record (contract.ts:4669-4672)
  vehiclePlan: null,
  // this fixture is `advisory: true` — nothing traded, a suggestion stands (:4725-4728)
  exitMode: "suggest",
  band: "breach",
  netRisk: 300_000,
  threshold: 250_000,
  utilization: 1.2,
  overflow: 75_000,
  size: 75_000,
  internalCrossed: 0,
  externalHedged: 75_000,
  advisory: true,
  firedAt: Date.now(),
  policyPath: [0, 1],
  reason: "breach · submit_market_order",
  lps: ["LP-1", "LP-3"],
};

const provenanceRow: HedgeProvenance = {
  // `self` vehicle ⇒ no unit arithmetic to record (contract.ts:4669-4672)
  vehiclePlan: null,
  hedgeId: "H-0001",
  book: "fi-rates-emea",
  instrument: "US10Y",
  firedAt: Date.now(),
  metric: "dv01",
  threshold: 250_000,
  netRisk: 300_000,
  utilization: 1.2,
  band: "breach",
  policyPath: [0, 1],
  action: { ...defaultExitAction("submit_market_order") },
  internalCrossed: 0,
  externalHedged: 75_000,
  residual: 0,
  hedgePrice: 100.26,
  midAtFire: 100.25,
  slippageBp: 1.2,
  lpWon: "LP-1",
  advisory: true,
  lps: ["LP-1", "LP-3"],
};

function makeApp(opts: { canEdit?: boolean } = {}) {
  return {
    transport: {
      getHedgePolicyGraph: vi.fn(async () => policyGraph()),
      updateHedgePolicyGraph: vi.fn(async (g: HedgeGraph) => g),
      listRiskBooks: vi.fn(async () => [
        { id: "fi-rates-emea", name: "EMEA Rates", parentId: null, deskId: "emea", description: "", limits: null, enabled: true },
      ]),
      listHedgeThresholds: vi.fn(async () => [threshold]),
      updateHedgeThreshold: vi.fn(async () => [threshold]),
      listHedgeProvenance: vi.fn(async () => [provenanceRow]),
      getHedgeConfig: vi.fn(async () => config),
      setHedgeConfig: vi.fn(async (c: HedgeConfig) => c),
      streamHedgeIntents: (onIntent: (i: HedgeIntent) => void) => {
        onIntent(advisoryIntent);
        return () => undefined;
      },
    },
    auth: {
      user: { id: "u", email: "hedge@celnet.com" },
      isAdmin: true,
      can: () => opts.canEdit ?? true,
    },
    setSignInOpen: vi.fn(),
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("HedgingWorkspace — exit policy", () => {
  it("loads the policy into the rules table and opens the exit-action editor", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);

    // The seeded policy decompiles to two rows (specific + default).
    expect(await screen.findByTestId("hedge-rules-table")).toBeInTheDocument();
    expect(await screen.findByTestId("hedge-rule-row-0")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("hedge-create-rule"));
    expect(await screen.findByTestId("hedge-rule-editor")).toBeInTheDocument();
    // The leaf editor is the ExitAction picker (not a book target).
    expect(screen.getByTestId("exit-action-kind")).toBeInTheDocument();
  });

  it("renders the live trace panel with a resolved action", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);
    expect(await screen.findByTestId("hedge-trace")).toBeInTheDocument();
    expect(screen.getByTestId("trace-band")).toBeInTheDocument();
    expect(screen.getByTestId("trace-action")).toBeInTheDocument();
  });
});

describe("HedgingWorkspace — thresholds", () => {
  it("renders the threshold roster + form", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-thresholds"));
    expect(await screen.findByTestId("threshold-table")).toBeInTheDocument();
    expect(screen.getByTestId("threshold-row-fi-rates-emea")).toBeInTheDocument();
    expect(screen.getByTestId("threshold-form")).toBeInTheDocument();
  });

  it("pre-fills the bands at the engine default (amber 0.80 / red 0.90, target at the amber edge)", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-thresholds"));
    expect(await screen.findByTestId("threshold-form")).toBeInTheDocument();
    // The blank-draft pre-fill must match celnet-hedge-routing's WarehouseThreshold
    // default (DEFAULT_AMBER 0.80 / DEFAULT_RED 0.90, target_fraction = amber) — not
    // the old 0.70 amber, which diverged from the engine.
    expect(screen.getByLabelText("Amber (0–1)")).toHaveValue(0.8);
    expect(screen.getByLabelText("Red (0–1)")).toHaveValue(0.9);
    expect(screen.getByLabelText("Target fraction")).toHaveValue(0.8);
  });
});

describe("HedgingWorkspace — execution mode", () => {
  it("renders the engine execution-mode config (kill-switch + mode) on the Execution mode tab", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-execution"));

    // The kill-switch / execution-mode controls (HedgeConfigControl) live here now.
    expect(await screen.findByTestId("kill-switch")).toBeInTheDocument();
    expect(screen.getByTestId("hedge-execution-mode")).toBeInTheDocument();
    expect(screen.getByTestId("exec-mode-advisory")).toBeInTheDocument();
  });

  it("does NOT mount the live monitor on the Hedging Rules surface (it moved to Risk → Hedge flows)", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-execution"));
    // Wait for the config to settle so a late-mounting monitor would have appeared.
    expect(await screen.findByTestId("kill-switch")).toBeInTheDocument();

    // The flow-monitor sections are gone from this surface entirely.
    expect(screen.queryByTestId("hedge-monitor")).toBeNull();
    expect(screen.queryByTestId("engine-strip")).toBeNull();
    expect(screen.queryByTestId("live-hedges")).toBeNull();
    // And the old Monitor tab no longer exists.
    expect(screen.queryByTestId("tab-monitor")).toBeNull();
  });
});

describe("HedgingWorkspace — LP panels", () => {
  it("renders the standing panels with their resolved effective LP set", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-lp-panels"));

    expect(await screen.findByTestId("lp-panels-table")).toBeInTheDocument();
    // The seeded book panel (include LP-1/2/3, exclude LP-2) resolves to {LP-1, LP-3}.
    const resolved = screen.getByTestId("lp-panel-resolved-book-fi-rates-emea");
    expect(resolved).toHaveTextContent("LP-1");
    expect(resolved).toHaveTextContent("LP-3");
    expect(resolved).not.toHaveTextContent("LP-2");
  });

  it("adds a panel with an include + an exclude and shows the resolved set live, then saves", async () => {
    const app = makeApp();
    state.app = app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-lp-panels"));

    fireEvent.click(await screen.findByTestId("lp-panel-add"));
    expect(await screen.findByTestId("lp-panel-editor")).toBeInTheDocument();

    fireEvent.change(screen.getByTestId("lp-panel-scope-id"), { target: { value: "fi-marex" } });
    fireEvent.click(screen.getByTestId("lp-panel-include-LP-1"));
    fireEvent.click(screen.getByTestId("lp-panel-include-LP-2"));
    fireEvent.click(screen.getByTestId("lp-panel-include-LP-3"));
    fireEvent.click(screen.getByTestId("lp-panel-exclude-LP-2"));

    // The live resolved-set display reflects include-minus-exclude = {LP-1, LP-3}.
    const resolved = screen.getByTestId("lp-panel-resolved");
    expect(resolved).toHaveTextContent("LP-1");
    expect(resolved).toHaveTextContent("LP-3");
    expect(resolved).not.toHaveTextContent("LP-2");

    fireEvent.click(screen.getByTestId("lp-panel-save"));
    // The new row appears once the (optimistic) commit settles — flushes the async set.
    expect(await screen.findByTestId("lp-panel-row-book-fi-marex")).toBeInTheDocument();
    // The committed config carries the new panel bound to lpPanels.
    expect(app.transport.setHedgeConfig).toHaveBeenCalledTimes(1);
    const committed = at(at(app.transport.setHedgeConfig.mock.calls, 0), 0) as HedgeConfig;
    const added = committed.lpPanels.find((p) => p.scopeId === "fi-marex");
    expect(added).toEqual({ scopeKind: "book", scopeId: "fi-marex", include: ["LP-1", "LP-2", "LP-3"], exclude: ["LP-2"] });
  });

  it("blocks saving a panel whose effective set is empty (client mirror of the server check)", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-lp-panels"));

    fireEvent.click(await screen.findByTestId("lp-panel-add"));
    fireEvent.change(screen.getByTestId("lp-panel-scope-id"), { target: { value: "fi-x" } });
    // Include LP-1 then exclude it ⇒ empty effective set ⇒ error + save disabled.
    fireEvent.click(screen.getByTestId("lp-panel-include-LP-1"));
    fireEvent.click(screen.getByTestId("lp-panel-exclude-LP-1"));

    expect(await screen.findByTestId("lp-panel-errors")).toHaveTextContent(/empty/i);
    expect(screen.getByTestId("lp-panel-save")).toBeDisabled();
  });

  it("is read-only without the hedge capability (no add affordance)", async () => {
    state.app = makeApp({ canEdit: false });
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-lp-panels"));
    expect(await screen.findByTestId("lp-panels-table")).toBeInTheDocument();
    expect(screen.queryByTestId("lp-panel-add")).toBeNull();
    expect(screen.queryByTestId("lp-panel-edit-book-fi-rates-emea")).toBeNull();
  });
});

describe("HedgingWorkspace — capability gating", () => {
  it("is read-only without the hedge capability", async () => {
    state.app = makeApp({ canEdit: false });
    render(<HedgingWorkspace />);
    // The rules table still renders, but no create/save affordance exists.
    expect(await screen.findByTestId("hedge-rules-table")).toBeInTheDocument();
    expect(screen.queryByTestId("hedge-create-rule")).toBeNull();
    expect(screen.queryByTestId("hedge-save-policy")).toBeNull();
  });
});
