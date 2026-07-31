/**
 * HedgingWorkspace — the auto-hedge surface driven with `useApp` mocked (no server).
 * Covers: the Exit-Policy tab loads a policy into the rules table and opens the
 * per-rule editor with the exit-action leaf; the Thresholds tab renders the roster +
 * form; the Monitor tab renders a streamed advisory intent (with its ADVISORY badge +
 * RAG chip) and a fired provenance row; and the whole surface is read-only without the
 * `hedge` capability.
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
  advisoryOnly: true,
  deskEnabled: [{ desk: "emea", enabled: true }],
  maxClip: 1_000_000,
  maxHedgesPerInterval: 20,
  dailyExternalNotionalCap: 1_000_000_000,
};

const advisoryIntent: HedgeIntent = {
  book: "fi-rates-emea",
  instrument: "US10Y",
  action: { ...defaultExitAction("submit_market_order") },
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
};

const provenanceRow: HedgeProvenance = {
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
  lpWon: "LP-2",
  advisory: true,
};

function makeApp(opts: { canEdit?: boolean } = {}) {
  return {
    transport: {
      getHedgePolicyGraph: vi.fn(async () => policyGraph()),
      updateHedgePolicyGraph: vi.fn(async (g: HedgeGraph) => g),
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

describe("HedgingWorkspace — monitor", () => {
  it("shows the streamed advisory intent with its badge + RAG, and a provenance row", async () => {
    state.app = makeApp();
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-monitor"));

    // The kill-switch / advisory-only controls.
    expect(await screen.findByTestId("kill-switch")).toBeInTheDocument();
    expect(screen.getByTestId("advisory-only")).toBeInTheDocument();

    // The streamed advisory intent renders with its ADVISORY badge + a RAG chip.
    expect(await screen.findByTestId("intent-row")).toBeInTheDocument();
    expect(screen.getAllByTestId("advisory-badge").length).toBeGreaterThan(0);
    expect(screen.getByTestId("rag-book-fi-rates-emea")).toBeInTheDocument();

    // The fired provenance appears in the audit table.
    expect(await screen.findByTestId("provenance-row-H-0001")).toBeInTheDocument();
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
