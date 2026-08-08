/**
 * "Create acceptance rule" from live flow — the Deals / Quotes blotter rows spawn an
 * acceptance rule pre-populated with the row's counterparty, handed to the Acceptance
 * builder via the {@link AcceptanceSeedProvider} store (not a URL-encoded rule).
 *
 * The Shell keeps every workspace pane mounted (P0-11) and also mounts a hidden
 * `acceptance` alias pane (a RiskDashboardWorkspace with `initialTab="acceptance"`), so
 * several RiskDashboardWorkspace instances share the app-level seed. The row action
 * therefore (a) seeds the store and (b) NAVIGATES to the `acceptance` alias; ONLY the
 * Acceptance-host instance (`initialTab === "acceptance"`) reacts to + consumes the seed,
 * so no hidden pane races the one-shot.
 *
 * Covered here (real modules, `useApp` mocked so there is no server):
 *   • the pure seed helpers — `counterpartyAcceptanceRule` builds `Counterparty = <name>`
 *     (equality, text) with the default REJECT decision; `mergeSeedRule` PREPENDS above the
 *     catch-all without clobbering the existing graph;
 *   • right-clicking a DEAL row shows "Create acceptance rule"; invoking it captures the
 *     row's counterparty into the seed store AND navigates to the `acceptance` workspace;
 *     the quotes row ⋯ kebab does the same;
 *   • the Acceptance host merges the seed into the CURRENT policy (existing rules intact) as
 *     an unsaved, savable edit — the counterparty matches the row; the base Dashboard host
 *     does NOT react;
 *   • a risk_manage holder LACKING manage_acceptance still lands on Acceptance, but the
 *     seeded rule is read-only (no Save) with the ask-an-admin note.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { AcceptanceGraph, Deal, DeskRequest } from "../src/data/contract";
import {
  compileRulesToAcceptanceGraph,
  newAcceptanceRuleId,
  newDefaultAcceptanceRule,
  type AcceptanceRule,
} from "../src/lib/acceptanceRules";
import { counterpartyAcceptanceRule, mergeSeedRule } from "../src/lib/acceptanceSeed";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { RiskDashboardWorkspace, type RiskDashboardTab } from "../src/workspaces/RiskDashboardWorkspace";
import { DealsBlotterWorkspace } from "../src/workspaces/DealsBlotterWorkspace";
import { QuotesBlotterWorkspace } from "../src/workspaces/QuotesBlotterWorkspace";
import {
  AcceptanceSeedProvider,
  useAcceptanceSeed,
} from "../src/app/AcceptanceSeedContext";

// --- fixtures ---------------------------------------------------------------

/** A booked FI (OIS) deal for the Deals blotter, carrying `counterparty`. */
function fiDeal(counterparty: string): Deal {
  return {
    dealId: `deal-${counterparty}`,
    requestId: `req-${counterparty}`,
    kind: "RFQ",
    counterparty,
    desk: "g10-rates",
    productKind: "OIS",
    instrument: { tenorYears: 5, fixedRate: 0.04, notional: 1e7, direction: "PAY_FIXED" },
    curveSet: { currency: "USD", referenceDate: { year: 2026, month: 6, day: 26 }, pillars: [] },
    side: "BUY",
    notional: 1e7,
    price: 0.0405,
    executedAtNanos: 1_700_000_000_000_000_000n,
    trader: "Sam",
  };
}

/** A shown FI (OIS) quote for the Quotes blotter, carrying `counterparty`. */
function fiQuote(counterparty: string): DeskRequest {
  return {
    requestId: `q-${counterparty}`,
    kind: "RFQ",
    counterparty,
    desk: "g10-rates",
    instrument: { tenorYears: 5, fixedRate: 0.04, notional: 1e7, direction: "PAY_FIXED" },
    curveSet: { currency: "USD", referenceDate: { year: 2026, month: 6, day: 26 }, pillars: [] },
    side: "BUY",
    notional: 1e7,
    receivedAtNanos: 1_700_000_000_000_000_000n,
    state: "QUOTED",
    quote: { price: 0.0405, notional: 1e7, validForMs: 30_000, trader: "Sam" },
  };
}

/** A stored policy: one specific reject + the accept-all default (decompiles to 2 rows). */
function policyGraph(): AcceptanceGraph {
  const rules: AcceptanceRule[] = [
    {
      id: newAcceptanceRuleId(),
      conditions: [{ field: "edge_bps", op: "lt", value: { kind: "num", num: 0.5 } }],
      action: { kind: "reject", reason: "below edge floor" },
      enabled: true,
    },
    { id: newAcceptanceRuleId(), conditions: [], action: { kind: "accept", reason: "" }, enabled: true },
  ];
  return compileRulesToAcceptanceGraph(rules);
}

type CanFn = (action: string, asset: string) => boolean;

function makeApp(opts: {
  deals?: Deal[];
  quotes?: DeskRequest[];
  graph?: AcceptanceGraph | null;
  can?: CanFn;
}) {
  const updateAcceptanceGraph = vi.fn(async (g: AcceptanceGraph) => g);
  const setWorkspace = vi.fn();
  return {
    updateAcceptanceGraph,
    setWorkspace,
    app: {
      setWorkspace,
      conventions: {},
      scope: undefined,
      activeDomain: "fixed_income",
      transport: {
        label: "mock",
        listRiskBookRisk: vi.fn(async () => []),
        listRiskBooks: vi.fn(async () => []),
        listDesks: vi.fn(async () => []),
        listFixConnections: vi.fn(async () => []),
        getRiskRoutingGraph: vi.fn(async () => null),
        getAcceptanceGraph: vi.fn(async () => (opts.graph !== undefined ? opts.graph : policyGraph())),
        updateAcceptanceGraph,
        aggregateRatesRisk: vi.fn(async () => ({ nodes: [] })),
        listRatesPositions: vi.fn(async () => ({ positions: [] })),
        listEntities: vi.fn(async () => []),
        listBooks: vi.fn(async () => []),
        listDeskRequests: vi.fn(async () => ({ requests: opts.quotes ?? [] })),
        listDeals: vi.fn(async () => ({ deals: opts.deals ?? [] })),
        streamNotifications: vi.fn(() => () => {}),
      },
      auth: {
        user: { id: "u", email: "admin@celnet.com" },
        isAdmin: true,
        can: opts.can ?? (() => true),
      },
      setSignInOpen: vi.fn(),
    },
  };
}

/** A probe that mirrors the seed store's `pending` into the DOM for assertions. */
function SeedProbe(): React.ReactElement {
  const { pending, requestAcceptanceSeed } = useAcceptanceSeed();
  return (
    <div>
      <span data-testid="seed-cp">{pending?.counterparty ?? ""}</span>
      <button data-testid="seed-fire" onClick={() => requestAcceptanceSeed("Balyasny")}>
        seed
      </button>
    </div>
  );
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

// --- pure helpers -----------------------------------------------------------

describe("acceptanceSeed — pure helpers", () => {
  it("builds a Counterparty = <name> equality rule with the default reject decision", () => {
    const rule = counterpartyAcceptanceRule("Balyasny");
    expect(rule.conditions).toHaveLength(1);
    expect(rule.conditions[0]?.field).toBe("counterparty");
    expect(rule.conditions[0]?.op).toBe("eq");
    expect(rule.conditions[0]?.value).toEqual({ kind: "text", text: "Balyasny" });
    expect(rule.action.kind).toBe("reject");
    expect(rule.action.reason).toBe("");
    expect(rule.enabled).toBe(true);
  });

  it("merges the seed ABOVE the trailing catch-all without clobbering existing rules", () => {
    const existing: AcceptanceRule[] = [
      {
        id: "a",
        conditions: [{ field: "edge_bps", op: "lt", value: { kind: "num", num: 0.5 } }],
        action: { kind: "reject", reason: "floor" },
        enabled: true,
      },
      newDefaultAcceptanceRule(), // catch-all (accept), must stay LAST
    ];
    const seed = counterpartyAcceptanceRule("Point72");
    const merged = mergeSeedRule(existing, seed);

    expect(merged).toHaveLength(3);
    expect(merged[0]).toBe(existing[0]);
    expect(merged[1]).toBe(seed);
    expect(merged[2]?.conditions).toHaveLength(0);
    expect(existing).toHaveLength(2); // input not mutated
  });
});

// --- blotter row → seed + navigate ------------------------------------------

describe("Deals row → Create acceptance rule", () => {
  it("right-clicking a row shows the action; invoking it seeds the counterparty AND navigates to acceptance", async () => {
    const { app, setWorkspace } = makeApp({ deals: [fiDeal("Balyasny")] });
    state.app = app;
    await act(async () => {
      render(
        <AcceptanceSeedProvider>
          <SeedProbe />
          <DealsBlotterWorkspace />
        </AcceptanceSeedProvider>,
      );
    });

    const row = (await screen.findByText("Balyasny")).closest("tr") as HTMLElement;
    await act(async () => {
      fireEvent.contextMenu(row);
    });
    const menuItem = await screen.findByTestId("flow-create-acceptance-rule");
    expect(menuItem).toHaveTextContent("Counterparty = Balyasny");

    await act(async () => {
      fireEvent.click(menuItem);
    });
    // The row's counterparty is captured into the seed store, and we navigate to the
    // `acceptance` workspace (the consolidated Risk host's Acceptance tab).
    expect(screen.getByTestId("seed-cp")).toHaveTextContent("Balyasny");
    expect(setWorkspace).toHaveBeenCalledWith("acceptance");
  });

  it("opens the row menu from the keyboard context-menu key", async () => {
    state.app = makeApp({ deals: [fiDeal("Citadel")] }).app;
    await act(async () => {
      render(
        <AcceptanceSeedProvider>
          <DealsBlotterWorkspace />
        </AcceptanceSeedProvider>,
      );
    });
    const row = (await screen.findByText("Citadel")).closest("tr") as HTMLElement;
    await act(async () => {
      fireEvent.keyDown(row, { key: "ContextMenu" });
    });
    expect(await screen.findByTestId("flow-create-acceptance-rule")).toHaveTextContent(
      "Counterparty = Citadel",
    );
  });
});

describe("Quotes row → Create acceptance rule", () => {
  it("the ⋯ kebab seeds the counterparty AND navigates to acceptance", async () => {
    const { app, setWorkspace } = makeApp({ quotes: [fiQuote("Millennium")] });
    state.app = app;
    await act(async () => {
      render(
        <AcceptanceSeedProvider>
          <SeedProbe />
          <QuotesBlotterWorkspace />
        </AcceptanceSeedProvider>,
      );
    });

    const kebab = await screen.findByTestId("quote-row-kebab");
    await act(async () => {
      fireEvent.click(kebab);
    });
    await act(async () => {
      fireEvent.click(await screen.findByTestId("flow-create-acceptance-rule"));
    });
    expect(screen.getByTestId("seed-cp")).toHaveTextContent("Millennium");
    expect(setWorkspace).toHaveBeenCalledWith("acceptance");
  });
});

// --- the Acceptance host consumes the seed ----------------------------------

/** Render the Acceptance-host instance (the `acceptance` alias) + the seed trigger. */
async function renderAcceptanceHost(can?: CanFn): Promise<ReturnType<typeof vi.fn>> {
  const built = makeApp({ can });
  state.app = built.app;
  await act(async () => {
    render(
      <AcceptanceSeedProvider>
        <SeedProbe />
        <RiskDashboardWorkspace initialTab={"acceptance" as RiskDashboardTab} />
      </AcceptanceSeedProvider>,
    );
  });
  return built.updateAcceptanceGraph;
}

describe("Acceptance host consumes the flow seed", () => {
  it("merges a Counterparty=<name> rule into the existing policy as a savable unsaved edit", async () => {
    const updateAcceptanceGraph = await renderAcceptanceHost();
    // The stored policy loads: specific reject + accept-all default.
    const table = await screen.findByTestId("acceptance-rules-table");
    expect(within(table).getAllByTestId(/acceptance-rule-row-/)).toHaveLength(2);

    // Fire a seed for "Balyasny" → the host merges it as an unsaved edit.
    await act(async () => {
      fireEvent.click(screen.getByTestId("seed-fire"));
    });

    // The seeded row is highlighted + carries the counterparty; the existing rules survive.
    const seeded = await waitFor(() => {
      const el = screen.getByTestId("acceptance-rules-table").querySelector('[data-seeded="true"]');
      expect(el).not.toBeNull();
      return el as HTMLElement;
    });
    expect(seeded).toHaveTextContent("Counterparty = Balyasny");
    const rows = within(screen.getByTestId("acceptance-rules-table")).getAllByTestId(/acceptance-rule-row-/);
    expect(rows).toHaveLength(3); // existing reject + seed + default
    // An unsaved edit ⇒ Save is enabled; nothing auto-saved.
    await waitFor(() => expect(screen.getByTestId("acceptance-save-policy")).not.toBeDisabled());
    expect(updateAcceptanceGraph).not.toHaveBeenCalled();
  });

  it("the seeded counterparty exactly matches the requested row value", async () => {
    await renderAcceptanceHost();
    await screen.findByTestId("acceptance-rules-table");
    await act(async () => {
      fireEvent.click(screen.getByTestId("seed-fire"));
    });
    const seeded = await waitFor(() => {
      const el = screen.getByTestId("acceptance-rules-table").querySelector('[data-seeded="true"]');
      expect(el).not.toBeNull();
      return el as HTMLElement;
    });
    // The probe seeds "Balyasny"; the rule condition renders exactly that.
    expect(seeded).toHaveTextContent("Counterparty = Balyasny");
    expect(screen.getByTestId("seed-cp")).toHaveTextContent(""); // consumed (cleared)
  });

  it("a risk_manage holder without manage_acceptance lands read-only with the ask-an-admin note", async () => {
    const can: CanFn = (a, s) => s === "fixed_income" && (a === "view" || a === "risk_manage");
    const updateAcceptanceGraph = await renderAcceptanceHost(can);
    // Without manage_acceptance the Acceptance tab is clamped away initially.
    expect(screen.queryByTestId("risk-tab-acceptance")).toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByTestId("seed-fire"));
    });

    // The seed reveals the Acceptance tab read-only + shows the seeded rule + note; no Save.
    await waitFor(() =>
      expect(screen.getByTestId("risk-tab-acceptance")).toHaveAttribute("aria-pressed", "true"),
    );
    const seeded = screen.getByTestId("acceptance-rules-table").querySelector('[data-seeded="true"]');
    expect(seeded).toHaveTextContent("Counterparty = Balyasny");
    expect(screen.getByTestId("acceptance-seed-readonly-note")).toBeInTheDocument();
    expect(screen.queryByTestId("acceptance-save-policy")).toBeNull();
    expect(updateAcceptanceGraph).not.toHaveBeenCalled();
  });
});

describe("the base Dashboard host does NOT react to the seed", () => {
  it("stays on Dashboard when a seed fires (only the acceptance host reacts)", async () => {
    const built = makeApp({});
    state.app = built.app;
    await act(async () => {
      render(
        <AcceptanceSeedProvider>
          <SeedProbe />
          <RiskDashboardWorkspace />
        </AcceptanceSeedProvider>,
      );
    });
    expect(await screen.findByTestId("risk-tab-dashboard")).toHaveAttribute("aria-pressed", "true");

    await act(async () => {
      fireEvent.click(screen.getByTestId("seed-fire"));
    });
    // The base (initialTab=dashboard) instance ignores the seed — no spurious tab flip.
    expect(screen.getByTestId("risk-tab-dashboard")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("risk-tab-acceptance")).toHaveAttribute("aria-pressed", "false");
  });
});
