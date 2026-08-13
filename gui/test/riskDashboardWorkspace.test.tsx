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

function makeApp(opts: {
  risk: RiskBookRisk[];
  books: RiskBook[];
  auth?: { user: unknown; isAdmin: boolean; can: (a: string, s: string) => boolean };
}) {
  return {
    transport: {
      label: "mock",
      listRiskBookRisk: vi.fn(async () => opts.risk),
      listRiskBooks: vi.fn(async () => opts.books),
      // The other consolidated tabs load their own seams on mount — stub them so
      // switching to (or clamping onto) any tab (only the ACTIVE tab mounts) never
      // throws:
      //   • Portfolios (RiskBooksWorkspace) + Routing (RiskRoutingWorkspace) → listDesks
      //   • Routing → getRiskRoutingGraph, Acceptance → getAcceptanceGraph
      //   • Positions (RatesBookWorkspace) → listRatesPositions / listEntities / listBooks
      //   • Quotes (QuotesBlotterWorkspace) → listDeskRequests + streamNotifications
      //   • Client blotter (DealsBlotterWorkspace, client lens) → listDeals + listRiskBooks + streamNotifications
      //   • Hedge blotter (HedgeDealsView) → listHedgeProvenance + streamHedgeIntents
      //   • Hedge flows (HedgeMonitor) → getHedgeConfig + listHedgeProvenance +
      //     streamHedgeIntents + listHedgeSuggestions (the standing suggest-mode rows)
      listDesks: vi.fn(async () => []),
      listFixConnections: vi.fn(async () => []),
      getRiskRoutingGraph: vi.fn(async () => null),
      getAcceptanceGraph: vi.fn(async () => null),
      aggregateRatesRisk: vi.fn(async () => ({ nodes: [] })),
      listRatesPositions: vi.fn(async () => ({ positions: [] })),
      listEntities: vi.fn(async () => []),
      listBooks: vi.fn(async () => []),
      listDeskRequests: vi.fn(async () => ({ requests: [] })),
      listDeals: vi.fn(async () => ({ deals: [] })),
      streamNotifications: vi.fn(() => () => {}),
      listHedgeProvenance: vi.fn(async () => []),
      listHedgeSuggestions: vi.fn(async () => []),
      streamHedgeIntents: vi.fn(() => () => {}),
      getHedgeConfig: vi.fn(async () => ({
        killSwitch: false,
        execution: "advisory",
        deskEnabled: [],
        maxClip: 1_000_000,
        maxHedgesPerInterval: 20,
        dailyExternalNotionalCap: 1_000_000_000,
        lpPanels: [],
        compositeSpreadBp: 0.5,
        vehicles: [],
        exitModes: [],
      })),
    },
    conventions: {},
    scope: undefined,
    activeDomain: "fixed_income",
    auth: opts.auth ?? {
      user: { id: "u", email: "admin@celnet.com" },
      isAdmin: true,
      can: () => true,
    },
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

describe("RiskDashboardWorkspace — the consolidated 9-way Risk host", () => {
  const emptyBooks = { risk: [] as RiskBookRisk[], books: [] as RiskBook[] };

  it("splits management tabs (Risk) from ledger tabs (Fixed Income → Book)", async () => {
    state.app = makeApp(emptyBooks); // admin can() => true ⇒ every tab its host presents
    const MGMT = ["dashboard", "portfolios", "routing", "acceptance"] as const;
    const LEDGER = ["positions", "quotes", "clientblotter", "hedgeblotter", "hedgeflows"] as const;

    // The Risk host presents the four MANAGEMENT tabs and NONE of the ledgers — the
    // ledgers moved to the Fixed-Income "Book" host. An admin sees everything each host
    // presents, so a missing tab here is a routing fault, never a capability one.
    await act(async () => {
      render(<RiskDashboardWorkspace />);
    });
    for (const tab of MGMT) expect(screen.getByTestId(`risk-tab-${tab}`)).toBeInTheDocument();
    for (const tab of LEDGER) expect(screen.queryByTestId(`risk-tab-${tab}`)).toBeNull();
    // The retired sub-views stay retired.
    expect(screen.queryByTestId("risk-tab-scenario")).toBeNull();
    expect(screen.queryByTestId("risk-tab-deals")).toBeNull();
    // Default lands on Dashboard (the routed-risk roll-up).
    expect(screen.getByTestId("risk-tab-dashboard")).toHaveAttribute("aria-pressed", "true");

    cleanup();

    // The ledger host is the mirror image: the five blotters, none of the management
    // tabs, defaulting to Positions (its own first tab, not the other host's default).
    await act(async () => {
      render(<RiskDashboardWorkspace variant="ledgers" />);
    });
    for (const tab of LEDGER) expect(screen.getByTestId(`risk-tab-${tab}`)).toBeInTheDocument();
    for (const tab of MGMT) expect(screen.queryByTestId(`risk-tab-${tab}`)).toBeNull();
    expect(screen.getByTestId("risk-tab-positions")).toHaveAttribute("aria-pressed", "true");
  });

  it("mounts the split blotters + the self-fetching monitor on their tabs", async () => {
    state.app = makeApp(emptyBooks);
    await act(async () => {
      render(<RiskDashboardWorkspace variant="ledgers" />);
    });

    // Client blotter → the received (client) deals blotter, WITHOUT the client/hedge
    // lens toggle (it is a dedicated tab, so the toggle bar is hidden entirely).
    await act(async () => {
      fireEvent.click(screen.getByTestId("risk-tab-clientblotter"));
    });
    expect(await screen.findByText("Received deals")).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "deals lens" })).toBeNull();
    expect(screen.queryByTestId("deals-lens-hedge")).toBeNull();
    expect(screen.queryByTestId("deals-lens-client")).toBeNull();

    // Hedge blotter → the executed-hedge ledger.
    await act(async () => {
      fireEvent.click(screen.getByTestId("risk-tab-hedgeblotter"));
    });
    expect(await screen.findByText("Hedge deals")).toBeInTheDocument();

    // Hedge flows → the self-fetching live monitor (no props from a parent).
    await act(async () => {
      fireEvent.click(screen.getByTestId("risk-tab-hedgeflows"));
    });
    expect(await screen.findByTestId("hedge-monitor")).toBeInTheDocument();
  });

  it("switches to Routing then Acceptance — only the active tab's body mounts", async () => {
    state.app = makeApp(emptyBooks);
    await act(async () => {
      render(<RiskDashboardWorkspace />);
    });
    // The Dashboard heat table is mounted by default.
    expect(await screen.findByRole("table")).toBeInTheDocument();

    // Routing tab → the fill→portfolio rule builder mounts; the Dashboard unmounts.
    await act(async () => {
      fireEvent.click(screen.getByTestId("risk-tab-routing"));
    });
    expect(await screen.findByRole("heading", { name: "Risk Routing" })).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();

    // Acceptance tab → the accept/reject builder mounts; Routing unmounts.
    await act(async () => {
      fireEvent.click(screen.getByTestId("risk-tab-acceptance"));
    });
    expect(await screen.findByRole("heading", { name: "Acceptance" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Risk Routing" })).not.toBeInTheDocument();
  });

  it("hides the Acceptance tab from a risk_manage holder lacking manage_acceptance", async () => {
    // risk_manage·FI (reaches the host + Dashboard/Portfolios/Routing) but NOT
    // manage_acceptance ⇒ the Acceptance tab is hidden. The ledger tabs are no longer
    // on this host at all — they live on the Fixed-Income "Book" host.
    state.app = makeApp({
      ...emptyBooks,
      auth: {
        user: { id: "u", email: "riskmgr@celnet.com" },
        isAdmin: false,
        can: (a: string, s: string) =>
          s === "fixed_income" && (a === "view" || a === "risk_manage"),
      },
    });
    await act(async () => {
      render(<RiskDashboardWorkspace />);
    });
    for (const tab of ["dashboard", "portfolios", "routing"] as const) {
      expect(screen.getByTestId(`risk-tab-${tab}`)).toBeInTheDocument();
    }
    expect(screen.queryByTestId("risk-tab-acceptance")).not.toBeInTheDocument();
    // Default Dashboard still active (it is visible for this identity).
    expect(screen.getByTestId("risk-tab-dashboard")).toHaveAttribute("aria-pressed", "true");
  });

  it("clamps to the first visible tab when deep-linked to a forbidden tab", async () => {
    // Same risk_manage-only identity, deep-linked to the hidden Acceptance tab ⇒ the
    // active tab clamps to the first visible one (Dashboard), never an empty pane.
    state.app = makeApp({
      ...emptyBooks,
      auth: {
        user: { id: "u", email: "riskmgr@celnet.com" },
        isAdmin: false,
        can: (a: string, s: string) =>
          s === "fixed_income" && (a === "view" || a === "risk_manage"),
      },
    });
    await act(async () => {
      render(<RiskDashboardWorkspace initialTab="acceptance" />);
    });
    expect(screen.queryByTestId("risk-tab-acceptance")).not.toBeInTheDocument();
    expect(screen.getByTestId("risk-tab-dashboard")).toHaveAttribute("aria-pressed", "true");
  });

  it("shows a view-only FI trader ONLY the ledger tabs (Positions/Quotes/Client blotter/Hedge blotter — the view floor)", async () => {
    // A booking-only FI trader (view·FI, no risk_manage / manage_acceptance / hedge)
    // reaches the LEDGER host and sees the four view-floor tabs. This identity holds no
    // `risk_manage`, so under the old combined host the whole surface was out of reach —
    // splitting the ledgers onto the `view` floor is exactly what makes them reachable.
    state.app = makeApp({
      ...emptyBooks,
      auth: {
        user: { id: "u", email: "trader@celnet.com" },
        isAdmin: false,
        can: (a: string, s: string) => a === "view" && s === "fixed_income",
      },
    });
    await act(async () => {
      render(<RiskDashboardWorkspace variant="ledgers" />);
    });
    for (const tab of ["positions", "quotes", "clientblotter", "hedgeblotter"] as const) {
      expect(screen.getByTestId(`risk-tab-${tab}`)).toBeInTheDocument();
    }
    // Clamps onto the first visible tab (Positions), never an empty pane.
    expect(screen.getByTestId("risk-tab-positions")).toHaveAttribute("aria-pressed", "true");
    // Hedge flows needs `hedge` (hidden here); the management tabs are on the other host.
    for (const tab of ["dashboard", "portfolios", "routing", "acceptance", "hedgeflows"] as const) {
      expect(screen.queryByTestId(`risk-tab-${tab}`)).not.toBeInTheDocument();
    }
  });
});
