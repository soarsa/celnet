/**
 * RiskDashboardWorkspace — the per-book risk dashboard. These tests drive it with
 * `useApp` mocked (no server): the heat overview lists every book, selecting one
 * shows its greeks + RAG limit strip, and a `null` dv01/pnl renders as "—" (never 0).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import type { RiskBook, RiskBookRisk } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { RiskDashboardWorkspace } from "../src/workspaces/RiskDashboardWorkspace";

function riskRow(overrides: Partial<RiskBookRisk> = {}): RiskBookRisk {
  return {
    bookId: "fx-emea",
    name: "FX EMEA",
    netNotional: -1.2e8,
    grossNotional: 8e8,
    positionCount: 12,
    delta: 3_000_000,
    gamma: 42_000,
    vega: 250_000,
    theta: -18_000,
    dv01: null,
    pnl: null,
    limits: [{ metric: "net_notional", used: 1.2e8, limit: 1e9, fraction: 0.12, band: "green" }],
    ...overrides,
  };
}

function bookOf(id: string, name: string, deskId: string | null = null): RiskBook {
  return { id, name, parentId: null, deskId, description: "", limits: null, enabled: true };
}

function makeApp(opts: { risk: RiskBookRisk[]; books: RiskBook[] }) {
  return {
    transport: {
      listRiskBookRisk: vi.fn(async () => opts.risk),
      listRiskBooks: vi.fn(async () => opts.books),
    },
    auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true },
    setSignInOpen: vi.fn(),
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("RiskDashboardWorkspace", () => {
  it("renders a heat overview row per book", async () => {
    state.app = makeApp({
      risk: [riskRow(), riskRow({ bookId: "fx-apac", name: "FX APAC" })],
      books: [bookOf("fx-emea", "FX EMEA"), bookOf("fx-apac", "FX APAC")],
    });
    render(<RiskDashboardWorkspace />);

    const overview = await screen.findByRole("table");
    expect(within(overview).getByText("FX EMEA")).toBeInTheDocument();
    expect(within(overview).getByText("FX APAC")).toBeInTheDocument();
  });

  it("shows the selected book's greeks and renders null dv01/pnl as an em dash", async () => {
    state.app = makeApp({
      risk: [riskRow({ dv01: null, pnl: null })],
      books: [bookOf("fx-emea", "FX EMEA", "emea")],
    });
    render(<RiskDashboardWorkspace />);

    // The first book auto-selects; its detail panel renders the greek tiles.
    const detail = await screen.findByRole("region", { name: /risk detail for FX EMEA/i });
    expect(within(detail).getByText("Δ Delta")).toBeInTheDocument();
    expect(within(detail).getByText("Vega")).toBeInTheDocument();
    // dv01 + pnl are not-yet-evaluated ⇒ rendered as "—", never a fabricated 0.
    expect(within(detail).getAllByText("—").length).toBeGreaterThanOrEqual(2);
  });

  it("selects a different book when its overview row is clicked", async () => {
    state.app = makeApp({
      risk: [riskRow(), riskRow({ bookId: "fx-apac", name: "FX APAC" })],
      books: [bookOf("fx-emea", "FX EMEA"), bookOf("fx-apac", "FX APAC")],
    });
    render(<RiskDashboardWorkspace />);

    const overview = await screen.findByRole("table");
    fireEvent.click(within(overview).getByText("FX APAC"));
    expect(await screen.findByRole("region", { name: /risk detail for FX APAC/i })).toBeInTheDocument();
  });

  it("shows the limit-utilization strip with a RAG meter for the selected book", async () => {
    state.app = makeApp({
      risk: [
        riskRow({
          limits: [{ metric: "net_notional", used: 9.5e8, limit: 1e9, fraction: 0.95, band: "amber" }],
        }),
      ],
      books: [bookOf("fx-emea", "FX EMEA")],
    });
    render(<RiskDashboardWorkspace />);

    const detail = await screen.findByRole("region", { name: /risk detail for FX EMEA/i });
    expect(within(detail).getByRole("meter", { name: /net notional utilization/i })).toBeInTheDocument();
  });
});
