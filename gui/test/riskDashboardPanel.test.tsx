/**
 * DashboardPanel — the Risk Dashboard's per-portfolio rolled-up risk view, driven
 * with `useApp` mocked (no server). Covers: numeric roster cells + global-exposure
 * tiles carry the right-aligned tabular numeric class (Change 1 alignment); the
 * shared compact formatter renders `1.5m` (not a divergent `1.5M`); and a
 * portfolio's tenor/instrument drill-down is keyboard-expandable (aria-expanded)
 * and, once open, shows the By-tenor + By-instrument breakdown summed from the
 * routed deals (Change 2).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { Deal, RiskBook, RiskBookRisk } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { DashboardPanel } from "../src/workspaces/RiskDashboardWorkspace";

function risk(bookId: string, name: string, over: Partial<RiskBookRisk> = {}): RiskBookRisk {
  return {
    bookId,
    name,
    netNotional: 1_500_000,
    grossNotional: 2_500_000,
    positionCount: 3,
    delta: 0,
    gamma: 0,
    vega: 0,
    theta: 0,
    dv01: null,
    pnl: null,
    limits: [],
    ...over,
  };
}

function deal(bookId: string, tenorYears: number, notional: number): Deal {
  return {
    dealId: `d-${bookId}-${tenorYears}-${notional}`,
    requestId: "r",
    kind: "RFQ",
    counterparty: "CP",
    desk: "rates",
    instrument: { tenorYears, fixedRate: 0.04, notional, direction: "RECEIVE_FIXED" },
    curveSet: { currency: "USD", referenceDate: { year: 2026, month: 1, day: 1 }, pillars: [] },
    side: "BUY",
    notional,
    price: 0.04,
    executedAtNanos: 0n,
    trader: "t",
    riskBookId: bookId,
  };
}

function makeApp(opts: { books?: RiskBook[]; riskRows?: RiskBookRisk[]; deals?: Deal[] } = {}) {
  const riskRows = opts.riskRows ?? [risk("bk-1", "Alpha")];
  return {
    transport: {
      label: "in-app",
      listRiskBooks: vi.fn(async () => opts.books ?? []),
      // No subscribe seam ⇒ the panel falls through to the one-shot poll.
      listRiskBookRisk: vi.fn(async () => riskRows),
      listDeals: vi.fn(async () => ({ deals: opts.deals ?? [] })),
      streamNotifications: vi.fn(() => () => {}),
    },
    auth: { user: { id: "u", email: "risk@celnet.com" }, isAdmin: true, can: () => true },
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("DashboardPanel — numeric alignment (Change 1)", () => {
  it("renders roster numeric cells with the right-aligned tabular class", async () => {
    state.app = makeApp({ riskRows: [risk("bk-1", "Alpha")] });
    const { container } = render(<DashboardPanel onGoToPortfolios={vi.fn()} />);

    // The row renders once the poll resolves (name appears in the roster + detail).
    expect((await screen.findAllByText("Alpha")).length).toBeGreaterThanOrEqual(1);

    // Every numeric roster cell carries a `.num`-family class (CSS modules hash the
    // name, so match the stable prefix). Net + Gross + Positions + DV01 = 4 cells.
    const numCells = container.querySelectorAll('td[class*="num"]');
    expect(numCells.length).toBeGreaterThanOrEqual(4);
  });

  it("uses the shared compact formatter (`1.5m`, not `1.5M`)", async () => {
    state.app = makeApp({ riskRows: [risk("bk-1", "Alpha", { netNotional: 1_500_000 })] });
    render(<DashboardPanel onGoToPortfolios={vi.fn()} />);
    // The global-exposure tile + the roster both fold to 1.5m.
    expect(await screen.findAllByText("1.5m")).not.toHaveLength(0);
    expect(screen.queryByText("1.5M")).toBeNull();
  });
});

describe("DashboardPanel — tenor/instrument drill-down (Change 2)", () => {
  it("expands a portfolio to reveal the By-tenor + By-instrument breakdown", async () => {
    state.app = makeApp({
      riskRows: [risk("bk-1", "Alpha")],
      deals: [deal("bk-1", 5, 30_000_000), deal("bk-1", 10, 50_000_000), deal("bk-1", 10, 20_000_000)],
    });
    render(<DashboardPanel onGoToPortfolios={vi.fn()} />);

    const toggle = await screen.findByTestId("risk-expand-bk-1");
    // Collapsed by default.
    expect(toggle).toHaveAttribute("aria-expanded", "false");

    // Keyboard/click activation expands it.
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");

    // Both lenses render, summed from the routed deals.
    const grid = await screen.findByLabelText("Risk breakdown for Alpha");
    expect(within(grid).getByText("By tenor")).toBeInTheDocument();
    expect(within(grid).getByText("By instrument")).toBeInTheDocument();
    // 2–5y bucket (30m) + 5–10y bucket (70m); 10y OIS instrument sums to 70m.
    expect(within(grid).getByText("5–10y")).toBeInTheDocument();
    expect(within(grid).getByText("10y OIS")).toBeInTheDocument();
    expect(within(grid).getAllByText("70m").length).toBeGreaterThanOrEqual(1);
  });

  it("shows an honest empty note for a portfolio with no routed deals", async () => {
    state.app = makeApp({ riskRows: [risk("bk-1", "Alpha")], deals: [] });
    render(<DashboardPanel onGoToPortfolios={vi.fn()} />);

    const toggle = await screen.findByTestId("risk-expand-bk-1");
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(screen.getByText(/No routed fills to break down/i)).toBeInTheDocument(),
    );
  });
});
