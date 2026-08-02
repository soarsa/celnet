/**
 * RiskDashboardWorkspace — the per-book risk dashboard. These tests drive it with
 * `useApp` mocked (no server): the heat overview lists every book, selecting one
 * shows its greeks + RAG limit strip, and a `null` dv01/pnl renders as "—" (never 0).
 */
import { act } from "react";
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
      // The Portfolios tab (RiskBooksWorkspace) loads the desk roster on mount for
      // admins — stub it so switching tabs in a test does not throw.
      listDesks: vi.fn(async () => []),
    },
    auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true },
    setSignInOpen: vi.fn(),
  };
}

/**
 * A streaming app whose transport implements `subscribeRiskBookRisk` by capturing
 * the callback, so a test can push snapshot / update frames on demand. The one-shot
 * `listRiskBookRisk` throws — proving the workspace renders purely from the stream.
 */
function makeStreamingApp(books: RiskBook[]) {
  let onSnapshot: ((rows: RiskBookRisk[], version: number) => void) | null = null;
  const teardown = vi.fn();
  return {
    push: (rows: RiskBookRisk[], version: number) => {
      if (onSnapshot) onSnapshot(rows, version);
    },
    teardown,
    app: {
      transport: {
        listRiskBooks: vi.fn(async () => books),
        listRiskBookRisk: vi.fn(async () => {
          throw new Error("poll must not be used when the stream is available");
        }),
        subscribeRiskBookRisk: (cb: (rows: RiskBookRisk[], version: number) => void) => {
          onSnapshot = cb;
          return teardown;
        },
      },
      auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true },
      setSignInOpen: vi.fn(),
    },
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

  it("shows the FI risk vector (DV01 / notional, NO option greeks) and renders null dv01/pnl as an em dash", async () => {
    state.app = makeApp({
      risk: [riskRow({ dv01: null, pnl: null })],
      books: [bookOf("fx-emea", "FX EMEA", "emea")],
    });
    render(<RiskDashboardWorkspace />);

    // The first book auto-selects; its detail panel renders the FI-relevant tiles.
    const detail = await screen.findByRole("region", { name: /risk detail for FX EMEA/i });
    expect(within(detail).getByText("Net notional")).toBeInTheDocument();
    expect(within(detail).getByText("DV01")).toBeInTheDocument();
    expect(within(detail).getByText("PnL")).toBeInTheDocument();
    // The FX-option greeks are meaningless for rates/bonds and MUST NOT render here.
    expect(within(detail).queryByText("Δ Delta")).not.toBeInTheDocument();
    expect(within(detail).queryByText("Vega")).not.toBeInTheDocument();
    expect(within(detail).queryByText("Γ Gamma")).not.toBeInTheDocument();
    expect(within(detail).queryByText("Θ Theta")).not.toBeInTheDocument();
    // dv01 + pnl are not-yet-evaluated ⇒ rendered as "—", never a fabricated 0.
    expect(within(detail).getAllByText("—").length).toBeGreaterThanOrEqual(2);
  });

  it("rolls up a firm-wide global exposure across all portfolios", async () => {
    state.app = makeApp({
      risk: [
        riskRow({ bookId: "a", name: "Book A", netNotional: 100, grossNotional: 200, positionCount: 3 }),
        riskRow({ bookId: "b", name: "Book B", netNotional: -40, grossNotional: 60, positionCount: 2 }),
      ],
      books: [bookOf("a", "Book A"), bookOf("b", "Book B")],
    });
    render(<RiskDashboardWorkspace />);

    const global = await screen.findByRole("region", {
      name: /global exposure across all risk portfolios/i,
    });
    expect(within(global).getByText("Global exposure")).toBeInTheDocument();
    // Positions fold: 3 + 2 = 5.
    expect(within(global).getByText("Positions")).toBeInTheDocument();
    expect(within(global).getByText("5")).toBeInTheDocument();
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

  it("renders the table from a pushed live snapshot and updates on a second push", async () => {
    const s = makeStreamingApp([bookOf("fx-emea", "FX EMEA"), bookOf("fx-apac", "FX APAC")]);
    state.app = s.app;
    render(<RiskDashboardWorkspace />);

    // Baseline push (version 1): the overview renders purely from the streamed frame.
    await act(async () => {
      s.push([riskRow(), riskRow({ bookId: "fx-apac", name: "FX APAC" })], 1);
    });
    const overview = await screen.findByRole("table");
    expect(within(overview).getByText("FX EMEA")).toBeInTheDocument();
    expect(within(overview).getByText("FX APAC")).toBeInTheDocument();
    // A subtle "live" indicator is shown while streaming.
    expect(screen.getByRole("status", { name: /live risk stream/i })).toBeInTheDocument();

    // A second push (version 2) replaces the set — a routed fill dropped FX APAC.
    await act(async () => {
      s.push([riskRow({ bookId: "fx-only", name: "FX ONLY" })], 2);
    });
    const updated = await screen.findByRole("table");
    expect(within(updated).getByText("FX ONLY")).toBeInTheDocument();
    expect(within(updated).queryByText("FX APAC")).not.toBeInTheDocument();
  });

  it("ignores a pushed frame whose version regressed (stale delivery)", async () => {
    const s = makeStreamingApp([bookOf("fx-emea", "FX EMEA")]);
    state.app = s.app;
    render(<RiskDashboardWorkspace />);

    await act(async () => {
      s.push([riskRow({ bookId: "fx-new", name: "FX NEW" })], 5);
    });
    // An older-version frame must NOT overwrite the newer applied state.
    await act(async () => {
      s.push([riskRow({ bookId: "fx-stale", name: "FX STALE" })], 3);
    });
    const overview = await screen.findByRole("table");
    expect(within(overview).getByText("FX NEW")).toBeInTheDocument();
    expect(within(overview).queryByText("FX STALE")).not.toBeInTheDocument();
  });

  it("falls back to the one-shot poll when the transport lacks the push", async () => {
    // The plain makeApp transport has no `subscribeRiskBookRisk` ⇒ the poll path runs.
    state.app = makeApp({
      risk: [riskRow()],
      books: [bookOf("fx-emea", "FX EMEA")],
    });
    render(<RiskDashboardWorkspace />);
    const overview = await screen.findByRole("table");
    expect(within(overview).getByText("FX EMEA")).toBeInTheDocument();
    // No live badge on the fallback poll path.
    expect(screen.queryByRole("status", { name: /live risk stream/i })).not.toBeInTheDocument();
  });

  it("exposes Dashboard + Portfolios tabs and the empty-state link lands on Portfolios", async () => {
    // Zero enabled portfolios ⇒ the Dashboard empty-state is shown.
    state.app = makeApp({ risk: [], books: [] });
    render(<RiskDashboardWorkspace />);

    // Both consolidated tabs are present; Dashboard is the default.
    const dashboardTab = await screen.findByTestId("risk-tab-dashboard");
    const portfoliosTab = screen.getByTestId("risk-tab-portfolios");
    expect(dashboardTab).toHaveAttribute("aria-pressed", "true");
    expect(portfoliosTab).toHaveAttribute("aria-pressed", "false");

    // The empty-state's inline "Portfolios" affordance switches to that tab — where
    // the create/enable editor now lives (no separate rail destination).
    fireEvent.click(await screen.findByTestId("empty-goto-portfolios"));

    expect(screen.getByTestId("risk-tab-portfolios")).toHaveAttribute("aria-pressed", "true");
    // The relocated risk-portfolio editor is now mounted as the tab panel.
    expect(
      await screen.findByRole("navigation", { name: /risk portfolio tree/i }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("new-risk-book")).toBeInTheDocument();
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
