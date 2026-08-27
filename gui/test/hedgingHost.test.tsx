/**
 * The consolidated Hedging host + the lifecycle ribbon.
 *
 * Two things are worth locking down here, and neither is cosmetic:
 *
 *   1. **One ledger.** The hedge blotter used to render on three screens at once. The
 *      host presents it on exactly one tab, and the Flow board HANDS OFF to that tab
 *      rather than embedding a second copy.
 *   2. **The drop-off is visible.** Orders sent, nothing filled, and the board still
 *      reporting hedges firing is the state that let the books sit pinned at their
 *      limit. The ribbon has to say so out loud.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import type {
  HedgeProvenance,
  RiskBookRisk,
  StreetOrder,
  StreetOutcome,
} from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { HedgingHost } from "../src/workspaces/hedging/HedgingHost";

function riskRow(over: Partial<RiskBookRisk> = {}): RiskBookRisk {
  return {
    bookId: "rates-usd",
    name: "RATES USD",
    netNotional: 0,
    grossNotional: 0,
    positionCount: 3,
    delta: 0,
    gamma: 0,
    vega: 0,
    theta: 0,
    dv01: 4_997,
    pnl: null,
    limits: [{ band: "red", fraction: 0.89, used: 4_997, limit: 5_600 }],
    ...over,
  } as RiskBookRisk;
}

function fire(hedgeId: string, external: number, residual = 0): HedgeProvenance {
  return {
    hedgeId,
    book: "rates-usd",
    instrument: "ZTU26",
    internalCrossed: 308,
    externalHedged: external,
    residual,
    advisory: false,
    band: "red",
    utilization: 0.89,
    firedAt: 1,
    hedgePrice: 0,
    midAtFire: 0,
    slippageBp: 0,
    lpWon: null,
    lps: [],
    policyPath: [],
    action: null,
    metric: "dv01",
    threshold: 5_600,
    netRisk: 4_997,
    vehiclePlan: null,
  } as unknown as HedgeProvenance;
}

function order(id: string, outcome: StreetOutcome, filledQty: number, reason?: string): StreetOrder {
  return {
    orderId: id,
    parentHedgeId: "h1",
    outcome,
    filledQty,
    requestedQty: 1_800_000,
    reason,
    instrument: "ZTU26",
    competitors: [],
  } as unknown as StreetOrder;
}

function makeApp(orders: StreetOrder[], provenance: HedgeProvenance[] = [fire("h1", 932)]) {
  return {
    auth: { user: { id: "u", email: "t@celnet.com" }, isAdmin: true, can: () => true },
    transport: {
      label: "live-ws",
      listRiskBooks: vi.fn(async () => []),
      listRiskBookRisk: vi.fn(async () => [riskRow()]),
      listHedgeProvenance: vi.fn(async () => provenance),
      listStreetOrders: vi.fn(async () => ({
        orders,
        breakdown: [],
        totalMatching: orders.length,
      })),
      streamHedgeIntents: () => () => undefined,
      getHedgeConfig: vi.fn(async () => ({}) as never),
      listHedgeSuggestions: vi.fn(async () => []),
    },
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("HedgingHost — one row for the whole hedge lifecycle", () => {
  it("presents the four lifecycle tabs in order", async () => {
    state.app = makeApp([]);
    await act(async () => {
      render(<HedgingHost />);
    });
    for (const tab of ["flow", "monitor", "blotter", "rules"] as const) {
      expect(screen.getByTestId(`hedging-tab-${tab}`)).toBeInTheDocument();
    }
  });

  it("opens on Flow, and the board does NOT embed a second ledger", async () => {
    // The whole point of the consolidation: the ledger lives on ONE tab. A board that
    // embedded it is how the same table came to render on three screens.
    state.app = makeApp([]);
    await act(async () => {
      render(<HedgingHost />);
    });
    expect(screen.getByTestId("hedge-flow-workspace")).toBeInTheDocument();
    expect(screen.queryByTestId("hedge-flow-ledger")).toBeNull();
  });

  it("a stage drill on the board opens the Blotter tab", async () => {
    state.app = makeApp([order("o1", "filled", 1_800_000)]);
    await act(async () => {
      render(<HedgingHost />);
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("hedge-stage-street"));
    });
    expect(screen.getByTestId("hedging-tab-blotter")).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByTestId("hedge-flow-workspace")).toBeNull();
  });

  it("opens on the Rules tab when deep-linked there (the retired `hedging` id)", async () => {
    state.app = makeApp([]);
    await act(async () => {
      render(<HedgingHost initialTab="rules" />);
    });
    expect(screen.getByTestId("hedging-tab-rules")).toHaveAttribute("aria-pressed", "true");
  });

  it("explains itself to a deep-link from an identity without `hedge`", async () => {
    // Every tab needs `hedge` — including the Blotter, because `HedgeDealsView` gates
    // itself on it and `listHedgeProvenance` enforces it server-side. Showing a Blotter
    // tab on the `view` floor would offer a door to a locked room.
    const app = makeApp([]);
    state.app = { ...app, auth: { ...app.auth, can: (c: string) => c === "view" } };
    await act(async () => {
      render(<HedgingHost />);
    });
    expect(screen.getByTestId("hedging-denied")).toBeInTheDocument();
    for (const tab of ["flow", "monitor", "blotter", "rules"] as const) {
      expect(screen.queryByTestId(`hedging-tab-${tab}`)).toBeNull();
    }
  });
});

describe("the lifecycle ribbon — the drop-off nothing used to show", () => {
  it("shouts when every street order shed nothing", async () => {
    state.app = makeApp([
      order("o1", "rejected", 0, "NOT_A_WHOLE_LOT"),
      order("o2", "rejected", 0, "NOT_A_WHOLE_LOT"),
    ]);
    await act(async () => {
      render(<HedgingHost />);
    });
    const alert = await screen.findByTestId("hedge-street-stalled");
    expect(alert).toHaveTextContent(/nothing is filling/i);
    expect(alert).toHaveTextContent("NOT_A_WHOLE_LOT");
    // The street stage reads 0 of 2 — the subtraction the old board never did.
    expect(screen.getByTestId("hedge-stage-street")).toHaveTextContent("0/2");
  });

  it("stays quiet when the fills are landing", async () => {
    state.app = makeApp([order("o1", "filled", 1_800_000)]);
    await act(async () => {
      render(<HedgingHost />);
    });
    await screen.findByTestId("hedge-lifecycle-ribbon");
    expect(screen.queryByTestId("hedge-street-stalled")).toBeNull();
    expect(screen.getByTestId("hedge-stage-street")).toHaveTextContent("1/1");
  });

  it("reports street orders that carry no hedge id instead of dropping them", async () => {
    const orphan = { ...order("o9", "filled", 1_000), parentHedgeId: undefined } as StreetOrder;
    state.app = makeApp([order("o1", "filled", 1_000), orphan]);
    await act(async () => {
      render(<HedgingHost />);
    });
    expect(await screen.findByTestId("hedge-street-unlinked")).toHaveTextContent(
      /1 street order/i,
    );
  });
});
