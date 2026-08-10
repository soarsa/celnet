/**
 * DealsBlotterWorkspace — the Client/Hedge lens split, against the REAL AppProvider +
 * offline MockTransport (like dealInternalise.test). Covers: the blotter shows a
 * [Client deals | Hedge deals] toggle with Client active by default; switching to
 * Hedge deals renders the executed-hedge ledger (a seeded fired hedge with its LP +
 * price), and switching back returns to the client fills.
 */
import { act } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { DealsBlotterWorkspace } from "../src/workspaces/DealsBlotterWorkspace";
import { createMockTransport } from "../src/data/mockSource";
import type { CelnetTransport } from "../src/data/transport";
import type { Deal, HedgeProvenance, ListDealsResponse } from "../src/data/contract";

function fiDeal(over: Partial<Deal>): Deal {
  return {
    dealId: "deal-x",
    requestId: "req-x",
    kind: "RFQ",
    counterparty: "Point72",
    desk: "g10-rates",
    productKind: "OIS",
    instrument: { tenorYears: 5, fixedRate: 0.04, notional: 1e7, direction: "PAY_FIXED" },
    curveSet: { currency: "USD", referenceDate: { year: 2026, month: 6, day: 26 }, pillars: [] },
    side: "BUY",
    notional: 1e7,
    price: 0.0405,
    executedAtNanos: 1_700_000_000_000_000_000n,
    trader: "Sam",
    ...over,
  };
}

function hedge(): HedgeProvenance {
  return {
    hedgeId: "hedge-42",
    book: "EMEA Rates",
    instrument: "USD-OIS 10y",
    firedAt: Date.UTC(2026, 7, 7, 9, 30, 0),
    metric: "dv01",
    threshold: 100,
    netRisk: 145,
    utilization: 1.45,
    band: "red",
    policyPath: [0],
    // An EXTERNAL hedge (submit_market_order) — the hedge desk shows external hedges by default.
    action: {
      kind: "submit_market_order",
      instrument: "",
      size: { kind: "overflow", fixed: 0 },
      skewBp: null,
      toEdge: false,
      style: "immediate",
      lps: [],
      internalFirst: false,
      reason: "",
    },
    internalCrossed: 30_000_000,
    externalHedged: 70_000_000,
    residual: 0,
    hedgePrice: 100.262,
    midAtFire: 100.25,
    slippageBp: 1.2,
    lpWon: "Goldman",
    advisory: false,
    lps: ["Goldman", "Citi"],
  };
}

function transport(): CelnetTransport {
  const t = createMockTransport();
  const deals: Deal[] = [fiDeal({ dealId: "deal-client", counterparty: "Point72" })];
  t.listDeals = async (): Promise<ListDealsResponse> => ({ deals });
  t.listHedgeProvenance = async (): Promise<HedgeProvenance[]> => [hedge()];
  return t;
}

async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

afterEach(() => {
  window.history.replaceState(null, "", "/");
  document.body.innerHTML = "";
});

describe("DealsBlotterWorkspace — Client/Hedge lens", () => {
  it("defaults to Client deals, switches to the Hedge ledger, and back", async () => {
    window.history.replaceState(null, "", "/?mock&dom=fixed_income");
    await act(async () => {
      render(
        <AppProvider transport={transport()}>
          <DealsBlotterWorkspace />
        </AppProvider>,
      );
    });
    await settle();

    const clientTab = screen.getByTestId("deals-lens-client");
    const hedgeTab = screen.getByTestId("deals-lens-hedge");
    expect(clientTab).toHaveAttribute("aria-pressed", "true");
    expect(hedgeTab).toHaveAttribute("aria-pressed", "false");
    // Client fill is visible in the default lens.
    expect(screen.getByText("Point72")).toBeInTheDocument();

    // Switch to Hedge deals → the executed-hedge ledger with its LP + realised price.
    await act(async () => {
      fireEvent.click(hedgeTab);
    });
    await settle();
    expect(hedgeTab).toHaveAttribute("aria-pressed", "true");
    const hedgeRow = await screen.findByTestId("hedge-deal-row-hedge-42");
    expect(within(hedgeRow).getByTestId("hedge-lpwon-hedge-42")).toHaveTextContent("Goldman");
    expect(within(hedgeRow).getByText("100.2620")).toBeInTheDocument();
    // The client fill is no longer mounted in this lens.
    expect(screen.queryByText("Point72")).toBeNull();

    // Switch back to Client deals.
    await act(async () => {
      fireEvent.click(clientTab);
    });
    await settle();
    expect(screen.getByText("Point72")).toBeInTheDocument();
  });
});
