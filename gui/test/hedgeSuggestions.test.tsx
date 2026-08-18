/**
 * The suggest-then-exit surface, driven with `useApp` mocked (no server).
 *
 * Three things are load-bearing enough to pin:
 *
 *  1. The row reads as an INSTRUCTION — the headline, the RAG context, and the vehicle
 *     plan's exact-vs-traded units with the residual SIGN spelled out in words.
 *  2. The duration-proxy warning appears IF AND ONLY IF `durationCorrect === false`. A
 *     proxy-based size presented as exact is the failure mode this whole surface exists to
 *     prevent, and an over-eager warning on an exact plan is just as corrosive.
 *  3. Acting raises NO dialog. That is the entire point of the feature — the standing row
 *     IS the deliberation step — so both buttons are asserted to call the transport
 *     directly, with the right `dismiss` flag, and to leave no `role="dialog"` behind.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { HedgeConfig, HedgeSuggestion, HedgeVehiclePlan } from "../src/data/contract";
import { defaultExitAction } from "../src/lib/hedgeExit";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { HedgeMonitor } from "../src/workspaces/hedging/HedgeMonitor";

const config: HedgeConfig = {
  killSwitch: false,
  execution: "lp_panel_then_composite",
  deskEnabled: [{ desk: "emea", enabled: true }],
  maxClip: 1_000_000,
  maxHedgesPerInterval: 20,
  dailyExternalNotionalCap: 1_000_000_000,
  lpPanels: [],
  compositeSpreadBp: 0.5,
  vehicles: [],
  exitModes: [{ scopeKind: "book", scopeId: "fi-credit-emea", mode: "suggest" }],
  // no per-scope model bindings: every scope stays CUSTOM (contract.ts:4853-4856)
  hedgingModels: [],
};

/** A whole-lot plan rounded DOWN off the duration-blind proxy — 36 DV01 left behind. */
const proxyPlan: HedgeVehiclePlan = {
  hedgeInstrumentId: "TY-DEC26",
  unitLabel: "contract",
  wholeUnits: true,
  dv01Basis: "exposure-proxy",
  durationCorrect: false,
  targetDv01: 24_840,
  dv01PerUnit: 78,
  exactUnits: 24_840 / 78,
  units: 318,
  hedgedDv01: 24_804,
  residualDv01: 36,
  summary: "Sell 318 contracts of TY-DEC26",
};

/** A duration-correct plan rounded UP — over-hedged by 31 DV01. */
const exactPlan: HedgeVehiclePlan = {
  hedgeInstrumentId: "G-MAR27",
  unitLabel: "contract",
  wholeUnits: true,
  dv01Basis: "analytic",
  durationCorrect: true,
  targetDv01: 8_993,
  dv01PerUnit: 64,
  exactUnits: 8_993 / 64,
  units: 141,
  hedgedDv01: 9_024,
  residualDv01: -31,
  summary: "Sell 141 contracts of G-MAR27",
};

function suggestion(overrides: Partial<HedgeSuggestion> = {}): HedgeSuggestion {
  return {
    suggestionId: "SUG-1",
    book: "fi-credit-emea",
    instrument: "XS2034-ACME-4H",
    desk: "emea",
    raisedAt: Date.now() - 45_000,
    band: "breach",
    netRisk: 24_840,
    threshold: 18_000,
    utilization: 1.38,
    action: { ...defaultExitAction("submit_market_order"), vehicleKind: "future", vehicleInstrument: "TY-DEC26" },
    policyPath: [0, 2, 3],
    externalSize: 24_840,
    vehiclePlan: proxyPlan,
    headline: "Sell 318 contracts of TY-DEC26",
    rationale: "Corporate 9y is 138% of its DV01 budget.",
    parentPositionId: 8_814n,
    lps: ["LP-1", "LP-3"],
    midAtRaise: 111.42,
    ...overrides,
  };
}

function makeApp(opts: {
  suggestions?: HedgeSuggestion[];
  executeImpl?: (id: string, dismiss: boolean) => Promise<{
    provenance: null;
    suggestions: HedgeSuggestion[];
  }>;
  canAct?: boolean;
} = {}) {
  const rows = opts.suggestions ?? [suggestion()];
  const executeHedgeSuggestion = vi.fn(
    opts.executeImpl ??
      (async (id: string) => ({ provenance: null, suggestions: rows.filter((s) => s.suggestionId !== id) })),
  );
  const listHedgeSuggestions = vi.fn(async () => rows);
  return {
    executeHedgeSuggestion,
    listHedgeSuggestions,
    app: {
      transport: {
        getHedgeConfig: vi.fn(async () => config),
        listHedgeProvenance: vi.fn(async () => []),
        listHedgeSuggestions,
        executeHedgeSuggestion,
        streamHedgeIntents: vi.fn(() => () => undefined),
      },
      auth: {
        user: { id: "u", email: "hedge@celnet.com" },
        isAdmin: true,
        can: () => opts.canAct ?? true,
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

describe("the standing-suggestion row reads as an instruction", () => {
  it("leads with the headline and carries the risk context beside it", async () => {
    const built = makeApp();
    state.app = built.app;
    render(<HedgeMonitor />);

    const row = await screen.findByTestId("suggestion-SUG-1");
    expect(row).toHaveTextContent("Sell 318 contracts of TY-DEC26");
    expect(row).toHaveTextContent("fi-credit-emea · XS2034-ACME-4H");
    expect(row).toHaveTextContent("BREACH");
    expect(row).toHaveTextContent("138% of budget");
    expect(row).toHaveTextContent("Corporate 9y is 138% of its DV01 budget.");
  });

  it("shows the exact units beside the traded units, so the rounding is visible", async () => {
    state.app = makeApp().app;
    render(<HedgeMonitor />);
    const plan = await screen.findByTestId("suggestion-plan");
    expect(plan).toHaveTextContent("318 contracts"); // traded (whole lots)
    expect(plan).toHaveTextContent("318.46"); // exact, before rounding
  });

  it("spells the residual SIGN out — a positive residual is risk STILL ON the book", async () => {
    state.app = makeApp().app;
    render(<HedgeMonitor />);
    expect(await screen.findByTestId("suggestion-residual")).toHaveTextContent(
      "rounded down, 36 DV01 still on the book",
    );
  });

  it("a NEGATIVE residual reads as over-hedged, not as a smaller shortfall", async () => {
    state.app = makeApp({
      suggestions: [
        suggestion({
          suggestionId: "SUG-2",
          vehiclePlan: exactPlan,
          headline: "Sell 141 contracts of G-MAR27",
        }),
      ],
    }).app;
    render(<HedgeMonitor />);
    expect(await screen.findByTestId("suggestion-residual")).toHaveTextContent(
      "rounded up, over-hedged by 31 DV01",
    );
  });

  it("announces new rows: the list is an aria-live region", async () => {
    state.app = makeApp().app;
    render(<HedgeMonitor />);
    expect(await screen.findByTestId("suggestion-list")).toHaveAttribute("aria-live", "polite");
  });

  it("says so honestly when nothing stands", async () => {
    state.app = makeApp({ suggestions: [] }).app;
    render(<HedgeMonitor />);
    expect(await screen.findByTestId("suggestions-empty")).toBeInTheDocument();
  });
});

describe("the duration-proxy warning appears IFF the size is proxy-derived", () => {
  it("warns when durationCorrect === false", async () => {
    state.app = makeApp().app;
    render(<HedgeMonitor />);
    const warning = await screen.findByTestId("suggestion-proxy-warning");
    expect(warning).toHaveTextContent(/approximate/i);
    expect(warning).toHaveTextContent(/duration-blind/i);
  });

  it("does NOT warn when the plan is duration-correct", async () => {
    state.app = makeApp({
      suggestions: [suggestion({ suggestionId: "SUG-2", vehiclePlan: exactPlan })],
    }).app;
    render(<HedgeMonitor />);
    await screen.findByTestId("suggestion-SUG-2");
    expect(screen.queryByTestId("suggestion-proxy-warning")).toBeNull();
  });

  it("renders no plan block at all for a self-vehicle suggestion (no unit arithmetic exists)", async () => {
    state.app = makeApp({
      suggestions: [suggestion({ suggestionId: "SUG-3", vehiclePlan: null })],
    }).app;
    render(<HedgeMonitor />);
    await screen.findByTestId("suggestion-SUG-3");
    expect(screen.queryByTestId("suggestion-plan")).toBeNull();
  });
});

describe("acting on a suggestion NEVER opens a dialog", () => {
  it("“Hedge now” calls the transport directly with dismiss=false and raises no dialog", async () => {
    const built = makeApp();
    state.app = built.app;
    render(<HedgeMonitor />);

    const btn = await screen.findByTestId("suggestion-hedge-SUG-1");
    await act(async () => {
      fireEvent.click(btn);
    });

    expect(built.executeHedgeSuggestion).toHaveBeenCalledWith("SUG-1", false);
    // The whole premise: no modal, no confirm step, ever.
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await waitFor(() => expect(screen.queryByTestId("suggestion-SUG-1")).toBeNull());
  });

  it("“Dismiss” calls the transport with dismiss=true and raises no dialog", async () => {
    const built = makeApp();
    state.app = built.app;
    render(<HedgeMonitor />);

    const btn = await screen.findByTestId("suggestion-dismiss-SUG-1");
    await act(async () => {
      fireEvent.click(btn);
    });

    expect(built.executeHedgeSuggestion).toHaveBeenCalledWith("SUG-1", true);
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => expect(screen.queryByTestId("suggestion-SUG-1")).toBeNull());
  });

  it("restores the row and shows an inline error when the act FAILS", async () => {
    const built = makeApp({
      executeImpl: async () => {
        throw new Error("hedge rejected: LP panel empty");
      },
    });
    state.app = built.app;
    render(<HedgeMonitor />);

    const btn = await screen.findByTestId("suggestion-hedge-SUG-1");
    await act(async () => {
      fireEvent.click(btn);
    });

    // Optimistically removed, then RESTORED — a rejected hedge never loses the suggestion.
    await waitFor(() => expect(screen.getByTestId("suggestion-SUG-1")).toBeInTheDocument());
    expect(await screen.findByTestId("suggestion-error")).toHaveTextContent("hedge rejected");
  });

  it("offers no act buttons to a viewer without the hedge capability", async () => {
    state.app = makeApp({ canAct: false }).app;
    render(<HedgeMonitor />);
    // The row is still SHOWN — it is risk information — but it cannot be acted on.
    await screen.findByTestId("suggestion-SUG-1");
    expect(screen.queryByTestId("suggestion-hedge-SUG-1")).toBeNull();
    expect(screen.queryByTestId("suggestion-dismiss-SUG-1")).toBeNull();
  });
});
