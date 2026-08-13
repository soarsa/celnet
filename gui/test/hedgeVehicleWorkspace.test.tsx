/**
 * The two new Hedging Rules tabs — the hedge-VEHICLE registry and the AUTO/SUGGEST exit
 * mode — plus the vehicle picker on the exit-action leaf, driven with `useApp` mocked.
 *
 * The picker assertions are about a real failure boundary, not cosmetics: a NAMED vehicle
 * must be a registry row, because that row is the only source of its DV01-per-unit and the
 * server rejects one it cannot price. So the control offers exactly the registered
 * instruments, and it says so when the registry is empty rather than letting a rule be
 * authored that will bounce.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

import type { HedgeConfig, HedgeGraph, HedgeVehicleRule } from "../src/data/contract";
import { compileRulesToHedgeGraph, newHedgeRuleId, type HedgeRule } from "../src/lib/hedgeRules";
import { defaultExitAction } from "../src/lib/hedgeExit";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { HedgingWorkspace } from "../src/workspaces/hedging/HedgingWorkspace";

const vehicle: HedgeVehicleRule = {
  id: "us-corp-7-10y",
  instrumentId: "",
  product: "BOND",
  ccy: "USD",
  minMaturityYears: 7,
  maxMaturityYears: 10,
  hedgeInstrumentId: "TY-DEC26",
  isFuture: true,
  dv01PerUnit: 78,
  unitLabel: "contract",
};

function config(overrides: Partial<HedgeConfig> = {}): HedgeConfig {
  return {
    killSwitch: false,
    execution: "lp_panel_then_composite",
    deskEnabled: [{ desk: "emea", enabled: true }],
    maxClip: 1_000_000,
    maxHedgesPerInterval: 20,
    dailyExternalNotionalCap: 1_000_000_000,
    lpPanels: [],
    compositeSpreadBp: 0.5,
    vehicles: [vehicle],
    exitModes: [{ scopeKind: "book", scopeId: "fi-credit-emea", mode: "suggest" }],
    ...overrides,
  };
}

function policyGraph(): HedgeGraph {
  const rules: HedgeRule[] = [
    { id: newHedgeRuleId(), conditions: [], action: { ...defaultExitAction("warehouse") }, enabled: true },
  ];
  return compileRulesToHedgeGraph(rules);
}

function makeApp(opts: { cfg?: HedgeConfig; canEdit?: boolean } = {}) {
  const setHedgeConfig = vi.fn(async (c: HedgeConfig) => c);
  return {
    setHedgeConfig,
    app: {
      transport: {
        getHedgePolicyGraph: vi.fn(async () => policyGraph()),
        updateHedgePolicyGraph: vi.fn(async (g: HedgeGraph) => g),
        listRiskBooks: vi.fn(async () => []),
        listHedgeThresholds: vi.fn(async () => []),
        updateHedgeThreshold: vi.fn(async () => []),
        listHedgeProvenance: vi.fn(async () => []),
        getHedgeConfig: vi.fn(async () => opts.cfg ?? config()),
        setHedgeConfig,
        streamHedgeIntents: vi.fn(() => () => undefined),
      },
      auth: {
        user: { id: "u", email: "hedge@celnet.com" },
        isAdmin: true,
        can: () => opts.canEdit ?? true,
      },
      setSignInOpen: vi.fn(),
    },
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("the hedge-vehicle registry tab", () => {
  it("renders the roster on the shared DataTable with the match, bucket and DV01 columns", async () => {
    state.app = makeApp().app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-vehicles"));

    const row = await screen.findByTestId("vehicle-row-us-corp-7-10y");
    expect(row).toHaveTextContent("any instrument · BOND · USD");
    expect(row).toHaveTextContent("7–10y");
    expect(row).toHaveTextContent("TY-DEC26");
    expect(row).toHaveTextContent("78");
    expect(row).toHaveTextContent("contract");
    // The shared grid binding, not a hand-rolled table.
    expect(screen.getByRole("table", { name: "Hedge vehicle registry" })).toBeInTheDocument();
  });

  it("blocks a save on a zero DV01 and surfaces EVERY problem at once", async () => {
    const built = makeApp();
    state.app = built.app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-vehicles"));
    fireEvent.click(await screen.findByTestId("vehicle-add"));

    // A fresh draft is already invalid (no hedge instrument, no DV01) — the errors show
    // together rather than one at a time.
    const errors = await screen.findByTestId("vehicle-errors");
    expect(errors).toHaveTextContent(/Hedge instrument is required/);
    expect(errors).toHaveTextContent(/DV01 per unit must be greater than 0/);
    expect(screen.getByTestId("vehicle-save")).toBeDisabled();

    // Inverting the maturity bucket adds its own error alongside the others.
    fireEvent.change(screen.getByTestId("vehicle-min-maturity"), { target: { value: "10" } });
    fireEvent.change(screen.getByTestId("vehicle-max-maturity"), { target: { value: "2" } });
    expect(screen.getByTestId("vehicle-errors")).toHaveTextContent(/Max maturity must be greater/);
    expect(built.setHedgeConfig).not.toHaveBeenCalled();
  });

  it("rejects a duplicate id against the live roster", async () => {
    state.app = makeApp().app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-vehicles"));
    fireEvent.click(await screen.findByTestId("vehicle-add"));
    fireEvent.change(screen.getByTestId("vehicle-id"), { target: { value: "us-corp-7-10y" } });
    expect(screen.getByTestId("vehicle-errors")).toHaveTextContent(/already used/);
  });

  it("commits a well-formed row through set_hedge_config (no separate CRUD verb)", async () => {
    const built = makeApp();
    state.app = built.app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-vehicles"));
    fireEvent.click(await screen.findByTestId("vehicle-add"));

    fireEvent.change(screen.getByTestId("vehicle-id"), { target: { value: "uk-gilt-3-7y" } });
    // The hedge instrument is a PICKER now. Reference data is US-only today, so a gilt
    // future is named through the picker's raw-commit escape hatch (typed, then blurred)
    // rather than chosen from the list — the path a desk uses for any instrument the
    // committed universe does not carry.
    const hedgeInstrument = screen.getByTestId("vehicle-hedge-instrument");
    fireEvent.change(hedgeInstrument, { target: { value: "G-MAR27" } });
    fireEvent.blur(hedgeInstrument);
    fireEvent.change(screen.getByTestId("vehicle-dv01"), { target: { value: "64" } });
    fireEvent.change(screen.getByTestId("vehicle-min-maturity"), { target: { value: "3" } });
    fireEvent.change(screen.getByTestId("vehicle-max-maturity"), { target: { value: "7" } });
    await act(async () => {
      fireEvent.click(screen.getByTestId("vehicle-save"));
    });

    expect(built.setHedgeConfig).toHaveBeenCalledTimes(1);
    const committed = built.setHedgeConfig.mock.calls[0]?.[0] as HedgeConfig;
    expect(committed.vehicles.map((v) => v.id)).toEqual(["us-corp-7-10y", "uk-gilt-3-7y"]);
    expect(committed.vehicles[1]?.dv01PerUnit).toBe(64);
  });

  it("flipping “is a future” re-defaults the unit label (a contract is not a face amount)", async () => {
    state.app = makeApp().app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-vehicles"));
    fireEvent.click(await screen.findByTestId("vehicle-add"));

    expect(screen.getByTestId("vehicle-unit-label")).toHaveValue("1mm face");
    fireEvent.click(screen.getByTestId("vehicle-is-future"));
    expect(screen.getByTestId("vehicle-unit-label")).toHaveValue("contract");
  });

  it("is read-only without the hedge capability", async () => {
    state.app = makeApp({ canEdit: false }).app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-vehicles"));
    await screen.findByTestId("vehicle-row-us-corp-7-10y");
    expect(screen.queryByTestId("vehicle-add")).toBeNull();
    expect(screen.queryByTestId("vehicle-edit-us-corp-7-10y")).toBeNull();
  });
});

describe("the exit-mode tab", () => {
  it("lists the bound scopes and says explicitly that SUGGEST is a standing row, not a popup", async () => {
    state.app = makeApp().app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-exit-mode"));

    const row = await screen.findByTestId("exit-mode-row-book-fi-credit-emea");
    expect(row).toHaveTextContent("SUGGEST");
    expect(row).toHaveTextContent(/standing row/i);
    expect(screen.getByTestId("exit-modes")).toHaveTextContent(/not a confirmation dialog/i);
  });

  it("commits a new binding through set_hedge_config", async () => {
    const built = makeApp();
    state.app = built.app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-exit-mode"));
    fireEvent.click(await screen.findByTestId("exit-mode-add"));

    fireEvent.change(screen.getByTestId("exit-mode-scope-id"), { target: { value: "fi-rates-emea" } });
    fireEvent.click(screen.getByTestId("exit-mode-option-auto"));
    await act(async () => {
      fireEvent.click(screen.getByTestId("exit-mode-save"));
    });

    const committed = built.setHedgeConfig.mock.calls[0]?.[0] as HedgeConfig;
    expect(committed.exitModes).toHaveLength(2);
    expect(committed.exitModes[1]).toEqual({
      scopeKind: "book",
      scopeId: "fi-rates-emea",
      mode: "auto",
    });
  });

  it("rejects a duplicate scope binding", async () => {
    state.app = makeApp().app;
    render(<HedgingWorkspace />);
    fireEvent.click(screen.getByTestId("tab-exit-mode"));
    fireEvent.click(await screen.findByTestId("exit-mode-add"));
    fireEvent.change(screen.getByTestId("exit-mode-scope-id"), { target: { value: "fi-credit-emea" } });
    expect(screen.getByTestId("exit-mode-errors")).toHaveTextContent(/already exists/);
    expect(screen.getByTestId("exit-mode-save")).toBeDisabled();
  });
});

describe("the vehicle picker on an exit-action leaf", () => {
  /** Open the rule editor on a fresh draft and switch its leaf to `kind`. */
  async function openLeaf(kind: string): Promise<void> {
    render(<HedgingWorkspace />);
    fireEvent.click(await screen.findByTestId("hedge-create-rule"));
    await screen.findByTestId("exit-action-kind");
    fireEvent.change(screen.getByTestId("exit-action-kind"), { target: { value: kind } });
  }

  it("shows on every SIZE-BEARING leaf", async () => {
    for (const kind of ["submit_market_order", "rfq_out", "split", "clear_risk", "cross_internal"]) {
      state.app = makeApp().app;
      await openLeaf(kind);
      expect(screen.getByTestId("exit-vehicle")).toBeInTheDocument();
      cleanup();
    }
  });

  it("shows on NONE of the leaves that place no order", async () => {
    for (const kind of ["warehouse", "skew", "escalate"]) {
      state.app = makeApp().app;
      await openLeaf(kind);
      expect(screen.queryByTestId("exit-vehicle")).toBeNull();
      cleanup();
    }
  });

  it("defaults to SELF and explains that it is the same security sold back at ratio 1", async () => {
    state.app = makeApp().app;
    await openLeaf("submit_market_order");
    expect(screen.getByTestId("exit-vehicle-kind")).toHaveValue("self");
    expect(screen.getByTestId("exit-vehicle-hint")).toHaveTextContent(/same security/i);
    expect(screen.getByTestId("exit-vehicle-hint")).toHaveTextContent(/ratio is exactly 1/i);
    // SELF names nothing, so there is no instrument control at all.
    expect(screen.queryByTestId("exit-vehicle-instrument")).toBeNull();
  });

  it("explains that BENCHMARK resolves from the registry by maturity bucket", async () => {
    state.app = makeApp().app;
    await openLeaf("submit_market_order");
    fireEvent.change(screen.getByTestId("exit-vehicle-kind"), { target: { value: "benchmark" } });
    expect(screen.getByTestId("exit-vehicle-hint")).toHaveTextContent(/maturity bucket/i);
    expect(screen.queryByTestId("exit-vehicle-instrument")).toBeNull();
  });

  it("offers the instrument control ONLY for the named kinds, populated from the registry", async () => {
    state.app = makeApp().app;
    await openLeaf("submit_market_order");
    fireEvent.change(screen.getByTestId("exit-vehicle-kind"), { target: { value: "future" } });

    const picker = screen.getByTestId("exit-vehicle-instrument");
    expect(picker).toBeInTheDocument();
    // Exactly the registry's hedge instruments — free text would author a rule the server
    // rejects, because an unregistered vehicle has no DV01 per unit.
    expect(screen.getByRole("option", { name: "TY-DEC26" })).toBeInTheDocument();
    fireEvent.change(picker, { target: { value: "TY-DEC26" } });
    expect(picker).toHaveValue("TY-DEC26");
  });

  it("says so when the registry is EMPTY instead of allowing an unfillable choice", async () => {
    state.app = makeApp({ cfg: config({ vehicles: [] }) }).app;
    await openLeaf("rfq_out");
    fireEvent.change(screen.getByTestId("exit-vehicle-kind"), { target: { value: "instrument" } });
    expect(screen.getByTestId("exit-vehicle-empty-registry")).toHaveTextContent(/Hedging Rules → Vehicles/);
    expect(screen.getByTestId("exit-vehicle-instrument")).toBeDisabled();
  });
});
