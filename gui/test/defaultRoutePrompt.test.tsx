/**
 * DefaultRoutePrompt — the startup routing-guard popup, driven with `useApp` mocked
 * (no server). Covers: it FLASHES the alertdialog when the firm has no valid default
 * routed portfolio; it is SUPPRESSED when a valid default exists; it stays suppressed
 * after dismissal (the once-per-session flag); it RE-EVALUATES live and closes when a
 * valid default is set; the Guided-setup CTA opens the wizard; and without
 * `risk_manage` the CTA is disabled with an ask-an-admin note (the warning still shows).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { RiskBook, RiskRoutingGraph } from "../src/data/contract";
import { compileRulesToGraph, newRuleId, type RiskRule } from "../src/lib/riskRules";
import { RISK_ROUTING_CHANGED_EVENT } from "../src/lib/routingGuard";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { DefaultRoutePrompt } from "../src/components/DefaultRoutePrompt";

function book(id: string, enabled: boolean): RiskBook {
  return {
    id,
    name: id.toUpperCase(),
    parentId: null,
    deskId: null,
    description: "",
    limits: null,
    enabled,
    // asset-class-agnostic fixture; a book must name ONE franchise
    assetClass: "fx_options",
  };
}
function defaultRule(bookId: string): RiskRule {
  return { id: newRuleId(), conditions: [], bookId, enabled: true };
}
/** A graph whose catch-all routes to an enabled `catchall` book (a valid default). */
function validGraph(): RiskRoutingGraph {
  return compileRulesToGraph([defaultRule("catchall")]);
}

/**
 * A mocked app whose routing graph + book roster are read from mutable closures, so a
 * test can flip the firm to a valid state and re-fetch. `canRisk` gates the CTA.
 */
function makeApp(opts: { graph?: RiskRoutingGraph | null; books?: RiskBook[]; canRisk?: boolean } = {}) {
  const store: { graph: RiskRoutingGraph | null; books: RiskBook[] } = {
    graph: opts.graph ?? null,
    books: opts.books ?? [book("catchall", true)],
  };
  const getRiskRoutingGraph = vi.fn(async () => store.graph);
  const listRiskBooks = vi.fn(async () => store.books);
  const setWorkspace = vi.fn();
  return {
    store,
    getRiskRoutingGraph,
    listRiskBooks,
    setWorkspace,
    app: {
      transport: {
        getRiskRoutingGraph,
        listRiskBooks,
        listDesks: vi.fn(async () => []),
        listFixConnections: vi.fn(async () => []),
      },
      auth: {
        signedIn: true,
        user: { id: "u", email: "trader@celnet.com" },
        can: (action: string) => (action === "risk_manage" ? (opts.canRisk ?? true) : true),
      },
      setWorkspace,
    },
  };
}

beforeEach(() => {
  state.app = null;
  sessionStorage.clear();
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("DefaultRoutePrompt — visibility", () => {
  it("flashes the alertdialog when there is no valid default routed portfolio", async () => {
    state.app = makeApp({ graph: null }).app;
    render(<DefaultRoutePrompt />);

    const dialog = await screen.findByRole("alertdialog", { name: "No default risk portfolio" });
    expect(dialog).toBeInTheDocument();
    expect(screen.getByTestId("default-route-guided")).toBeEnabled();
  });

  it("is suppressed when a valid default exists", async () => {
    const h = makeApp({ graph: validGraph(), books: [book("catchall", true)] });
    state.app = h.app;
    render(<DefaultRoutePrompt />);

    await waitFor(() => expect(h.listRiskBooks).toHaveBeenCalled());
    expect(screen.queryByTestId("default-route-prompt")).toBeNull();
  });

  it("warns when the catch-all points at a DISABLED portfolio", async () => {
    state.app = makeApp({ graph: validGraph(), books: [book("catchall", false)] }).app;
    render(<DefaultRoutePrompt />);
    expect(await screen.findByTestId("default-route-prompt")).toBeInTheDocument();
  });
});

describe("DefaultRoutePrompt — don't nag", () => {
  it("stays dismissed for the session after Later (and sets the session flag)", async () => {
    state.app = makeApp({ graph: null }).app;
    const { rerender } = render(<DefaultRoutePrompt />);

    fireEvent.click(await screen.findByTestId("default-route-later"));
    expect(screen.queryByTestId("default-route-prompt")).toBeNull();
    expect(sessionStorage.getItem("celnet:default-route-warn-dismissed")).toBe("1");

    // A re-render (still no valid default) must NOT bring it back this session.
    rerender(<DefaultRoutePrompt />);
    await waitFor(() => expect(screen.queryByTestId("default-route-prompt")).toBeNull());
  });
});

describe("DefaultRoutePrompt — live re-evaluation", () => {
  it("closes when a valid default is set (config-changed broadcast)", async () => {
    const h = makeApp({ graph: null });
    state.app = h.app;
    render(<DefaultRoutePrompt />);
    expect(await screen.findByTestId("default-route-prompt")).toBeInTheDocument();

    // The firm gains a valid default; broadcasting the change re-evaluates the guard.
    h.store.graph = validGraph();
    h.store.books = [book("catchall", true)];
    act(() => {
      window.dispatchEvent(new Event(RISK_ROUTING_CHANGED_EVENT));
    });

    await waitFor(() => expect(screen.queryByTestId("default-route-prompt")).toBeNull());
  });
});

describe("DefaultRoutePrompt — actions & gating", () => {
  it("opens the guided-setup wizard from the primary CTA", async () => {
    state.app = makeApp({ graph: null, canRisk: true }).app;
    render(<DefaultRoutePrompt />);

    fireEvent.click(await screen.findByTestId("default-route-guided"));
    // The prompt is replaced by the guided-setup wizard.
    expect(await screen.findByRole("dialog", { name: "Guided setup" })).toBeInTheDocument();
    expect(screen.queryByTestId("default-route-prompt")).toBeNull();
  });

  it("navigates to Risk → Routing from the secondary link", async () => {
    const h = makeApp({ graph: null });
    state.app = h.app;
    render(<DefaultRoutePrompt />);

    fireEvent.click(await screen.findByTestId("default-route-routing"));
    expect(h.setWorkspace).toHaveBeenCalledWith("riskrouting");
    expect(screen.queryByTestId("default-route-prompt")).toBeNull();
  });

  it("disables the CTA (with an ask-an-admin note) without Manage-Risk, but still warns", async () => {
    state.app = makeApp({ graph: null, canRisk: false }).app;
    render(<DefaultRoutePrompt />);

    expect(await screen.findByTestId("default-route-prompt")).toBeInTheDocument();
    expect(screen.getByTestId("default-route-guided")).toBeDisabled();
    expect(screen.getByTestId("default-route-cap-note")).toBeInTheDocument();
  });
});
