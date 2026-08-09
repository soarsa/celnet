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

function deal(
  bookId: string,
  tenorYears: number,
  notional: number,
  productKind: Deal["productKind"] = "OIS",
): Deal {
  return {
    dealId: `d-${bookId}-${tenorYears}-${notional}-${productKind}`,
    requestId: "r",
    kind: "RFQ",
    counterparty: "CP",
    desk: "rates",
    productKind,
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
  /** The expanded drill-down row's panel (`<td id="risk-breakdown-<bookId>">`) — the
   * scope that disambiguates the row-level breakdown from the detail-card one, which
   * now shares the same aria-label. */
  function expandedPanel(bookId: string): HTMLElement {
    const panel = document.getElementById(`risk-breakdown-${bookId}`);
    if (panel === null) throw new Error(`expanded panel for ${bookId} not found`);
    return panel;
  }

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

    // Both lenses render, summed from the routed deals. Scope to the EXPANDED ROW's
    // panel (id `risk-breakdown-bk-1`) — the same breakdown also renders in the
    // selected-book detail card, so a global label query would match both.
    const grid = within(expandedPanel("bk-1")).getByLabelText("Risk breakdown for Alpha");
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
      expect(
        within(expandedPanel("bk-1")).getByText(/No routed fills to break down/i),
      ).toBeInTheDocument(),
    );
  });

  it("renders a By-product-type breakdown reconciling to the book total", async () => {
    state.app = makeApp({
      riskRows: [risk("bk-1", "Alpha")],
      deals: [
        deal("bk-1", 5, 30_000_000, "OIS"),
        deal("bk-1", 10, 60_000_000, "BOND"),
        deal("bk-1", 2, 20_000_000, "IRS"),
      ],
    });
    render(<DashboardPanel onGoToPortfolios={vi.fn()} />);
    fireEvent.click(await screen.findByTestId("risk-expand-bk-1"));

    const grid = within(expandedPanel("bk-1")).getByLabelText("Risk breakdown for Alpha");
    // The third lens renders, with a bucket per family (multiple product buckets).
    expect(within(grid).getByText("By product type")).toBeInTheDocument();
    expect(within(grid).getByText("BOND")).toBeInTheDocument();
    expect(within(grid).getByText("OIS")).toBeInTheDocument();
    expect(within(grid).getByText("IRS")).toBeInTheDocument();
    // Reconciliation: the note reports the 110m book gross the product rows sum to.
    expect(within(grid).getByText(/110m gross notional/)).toBeInTheDocument();
    expect(within(grid).getAllByText("60m").length).toBeGreaterThanOrEqual(1); // BOND bucket
  });
});

describe("DashboardPanel — selected-book detail breakdown", () => {
  it("renders the By-product/tenor/instrument breakdown in the selected-book detail card", async () => {
    state.app = makeApp({
      riskRows: [risk("bk-1", "Alpha")],
      deals: [
        deal("bk-1", 5, 30_000_000, "OIS"),
        deal("bk-1", 10, 50_000_000, "OIS"),
        deal("bk-1", 10, 60_000_000, "BOND"),
      ],
    });
    render(<DashboardPanel onGoToPortfolios={vi.fn()} />);

    // The detail card (not the expand-row) carries the breakdown for the selected book.
    const detail = await screen.findByLabelText("Risk detail for Alpha");
    const grid = await within(detail).findByLabelText("Risk breakdown for Alpha");
    expect(within(grid).getByText("By tenor")).toBeInTheDocument();
    expect(within(grid).getByText("By product type")).toBeInTheDocument();
    expect(within(grid).getByText("By instrument")).toBeInTheDocument();
    // Reconciliation: the three lenses fold the SAME 140m item set.
    expect(within(grid).getByText(/140m gross notional/)).toBeInTheDocument();
  });

  it("reconciles a slug-drifted deal stamp to the roster book id (the live failure mode)", async () => {
    // The roster row id is `WASH_BOOK`; the routed deals carry the server slug
    // `wash-book`. The normalized join must still populate the breakdown — this is the
    // exact case/format mismatch that left the live breakdown blank.
    state.app = makeApp({
      riskRows: [risk("WASH_BOOK", "Wash")],
      deals: [deal("wash-book", 5, 30_000_000, "OIS"), deal("wash-book", 10, 50_000_000, "OIS")],
    });
    render(<DashboardPanel onGoToPortfolios={vi.fn()} />);

    const detail = await screen.findByLabelText("Risk detail for Wash");
    const grid = await within(detail).findByLabelText("Risk breakdown for Wash");
    expect(within(grid).getByText(/2 routed fills/)).toBeInTheDocument();
    expect(within(grid).getByText(/80m gross notional/)).toBeInTheDocument();
  });

  it("shows an honest empty note in the detail card when no deals map to the book", async () => {
    state.app = makeApp({ riskRows: [risk("bk-1", "Alpha")], deals: [] });
    render(<DashboardPanel onGoToPortfolios={vi.fn()} />);

    const detail = await screen.findByLabelText("Risk detail for Alpha");
    await waitFor(() =>
      expect(
        within(detail).getByText(/No routed fills to break down for Alpha/i),
      ).toBeInTheDocument(),
    );
  });
});

describe("DashboardPanel — sortable roster (Change 2)", () => {
  // Three books with distinct gross so an order is observable: Beta 3m > Alpha 2m > Gamma 1m.
  function threeBooks() {
    return {
      riskRows: [
        risk("bk-a", "Alpha", { grossNotional: 2_000_000, netNotional: 900_000, positionCount: 5 }),
        risk("bk-b", "Beta", { grossNotional: 3_000_000, netNotional: 100_000, positionCount: 2 }),
        risk("bk-g", "Gamma", { grossNotional: 1_000_000, netNotional: 500_000, positionCount: 9 }),
      ],
    };
  }

  /** The portfolio names in DOM (roster) order — the sorted row sequence. */
  function rosterOrder(container: HTMLElement): string[] {
    return [...container.querySelectorAll('button[data-testid^="risk-expand-"]')].map(
      (b) => b.getAttribute("aria-label")?.match(/for (\w+)/)?.[1] ?? "",
    );
  }

  it("defaults to gross notional descending (highest exposure first)", async () => {
    state.app = makeApp(threeBooks());
    const { container } = render(<DashboardPanel onGoToPortfolios={vi.fn()} />);
    await screen.findAllByText("Beta");

    expect(rosterOrder(container)).toEqual(["Beta", "Alpha", "Gamma"]);
    // The Gross header advertises the active descending sort.
    const grossBtn = screen.getByTestId("risk-sort-gross");
    expect(grossBtn.closest("th")).toHaveAttribute("aria-sort", "descending");
  });

  it("toggles direction on the active column and re-sorts", async () => {
    state.app = makeApp(threeBooks());
    const { container } = render(<DashboardPanel onGoToPortfolios={vi.fn()} />);
    await screen.findAllByText("Beta");

    fireEvent.click(screen.getByTestId("risk-sort-gross")); // desc → asc
    expect(screen.getByTestId("risk-sort-gross").closest("th")).toHaveAttribute(
      "aria-sort",
      "ascending",
    );
    expect(rosterOrder(container)).toEqual(["Gamma", "Alpha", "Beta"]);
  });

  it("sorts by a different column (net desc) and stamps aria-sort there", async () => {
    state.app = makeApp(threeBooks());
    const { container } = render(<DashboardPanel onGoToPortfolios={vi.fn()} />);
    await screen.findAllByText("Beta");

    fireEvent.click(screen.getByTestId("risk-sort-net")); // net desc: Alpha 900k > Gamma 500k > Beta 100k
    expect(screen.getByTestId("risk-sort-net").closest("th")).toHaveAttribute(
      "aria-sort",
      "descending",
    );
    expect(screen.getByTestId("risk-sort-gross").closest("th")).toHaveAttribute("aria-sort", "none");
    expect(rosterOrder(container)).toEqual(["Alpha", "Gamma", "Beta"]);
  });
});
