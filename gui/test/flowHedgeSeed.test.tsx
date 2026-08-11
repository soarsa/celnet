/**
 * "Change hedging strategy" from live flow — a Deals-blotter deal row spawns a NEW hedge
 * exit-policy rule pre-scoped to that deal's flow, handed to the Hedging → Exit Policy
 * builder via the {@link HedgeSeedProvider} store (not a URL-encoded rule).
 *
 * Covered here (real modules, `useApp` mocked so there is no server):
 *   • the pure seed helpers — `hedgeSeedFromDeal` projects a Deal onto the seed;
 *     `hedgeRuleFromSeed` builds `ccy = <ccy> AND product = <productKind> AND
 *     desk = <desk> AND counterparty = <counterparty>` (the identity fields the hedge
 *     graph can test), defaulting the exit action to WAREHOUSE, and
 *     `HEDGE_SEED_UNREPRESENTED` names the fields the graph vocabulary still cannot
 *     express (tenor/notional/side);
 *   • right-clicking a DEAL row shows "Change hedging strategy"; invoking it captures the
 *     deal into the seed store AND navigates to the `hedging` workspace;
 *   • the item is HIDDEN for a viewer lacking the `hedge` capability;
 *   • the Hedging Exit Policy builder consumes the seed: it opens a NEW draft rule with the
 *     flow conditions pre-filled + the seed hint, and consumes the seed ONCE (no re-seed).
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { Deal } from "../src/data/contract";
import {
  HEDGE_SEED_UNREPRESENTED,
  hedgeRuleFromSeed,
  hedgeSeedFromDeal,
} from "../src/lib/hedgeSeed";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { DealsBlotterWorkspace } from "../src/workspaces/DealsBlotterWorkspace";
import { HedgingWorkspace } from "../src/workspaces/hedging/HedgingWorkspace";
import { HedgeSeedProvider, useHedgeSeed } from "../src/app/HedgeSeedContext";

// --- fixtures ---------------------------------------------------------------

/** A booked FI (OIS) deal for the Deals blotter, carrying the flow facts. */
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

type CanFn = (action: string, asset: string) => boolean;

function makeApp(opts: { deals?: Deal[]; can?: CanFn }) {
  const updateHedgePolicyGraph = vi.fn(async (g: unknown) => g);
  const setWorkspace = vi.fn();
  return {
    setWorkspace,
    updateHedgePolicyGraph,
    app: {
      setWorkspace,
      conventions: {},
      scope: undefined,
      activeDomain: "fixed_income",
      transport: {
        label: "mock",
        listRiskBooks: vi.fn(async () => []),
        listDeals: vi.fn(async () => ({ deals: opts.deals ?? [] })),
        streamNotifications: vi.fn(() => () => {}),
        getHedgePolicyGraph: vi.fn(async () => null),
        // The Exit Policy tab loads the engine config for the leaf editor's hedge-VEHICLE
        // picker (a named vehicle must be a registry row).
        getHedgeConfig: vi.fn(async () => ({ vehicles: [], exitModes: [] })),
        updateHedgePolicyGraph,
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

/** A probe that mirrors the seed store's `pending` into the DOM + can fire a seed. */
function SeedProbe({ deal }: { deal: Deal }): React.ReactElement {
  const { pending, requestHedgeSeed } = useHedgeSeed();
  const dealId = pending?.source.kind === "deal" ? pending.source.deal.dealId : "";
  return (
    <div>
      <span data-testid="hseed-deal">{dealId}</span>
      <button data-testid="hseed-fire" onClick={() => requestHedgeSeed(hedgeSeedFromDeal(deal))}>
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

describe("hedgeSeed — pure helpers", () => {
  it("projects a Deal onto the seed's flow facts", () => {
    const s = hedgeSeedFromDeal(fiDeal("Balyasny"));
    expect(s).toMatchObject({
      dealId: "deal-Balyasny",
      counterparty: "Balyasny",
      productKind: "OIS",
      currency: "USD",
      desk: "g10-rates",
      tenorYears: 5,
      notional: 1e7,
      side: "BUY",
    });
    expect(s.instrumentSymbol).toBeUndefined();
  });

  it("builds ccy/product/desk/counterparty equality conditions with a WAREHOUSE default action", () => {
    const rule = hedgeRuleFromSeed(hedgeSeedFromDeal(fiDeal("Balyasny")));
    expect(rule.conditions.map((c) => c.field)).toEqual(["ccy", "product", "desk", "counterparty"]);
    expect(rule.conditions.every((c) => c.op === "eq")).toBe(true);
    expect(rule.conditions.map((c) => c.value)).toEqual([
      { kind: "text", text: "USD" },
      { kind: "text", text: "OIS" },
      { kind: "text", text: "g10-rates" },
      { kind: "text", text: "Balyasny" },
    ]);
    // The action is left as the safe WAREHOUSE default — the trader picks the real one.
    expect(rule.action.kind).toBe("warehouse");
    expect(rule.enabled).toBe(true);
  });

  it("seeds the counterparty literal exactly as the deal carries it (no transform)", () => {
    const rule = hedgeRuleFromSeed(hedgeSeedFromDeal(fiDeal("CITADEL")));
    const cp = rule.conditions.find((c) => c.field === "counterparty");
    expect(cp).toBeDefined();
    expect(cp?.op).toBe("eq");
    expect(cp?.value).toEqual({ kind: "text", text: "CITADEL" });
  });

  it("seeds instrument_id only when the deal carries a symbol", () => {
    const withSym = hedgeRuleFromSeed({ ...hedgeSeedFromDeal(fiDeal("X")), instrumentSymbol: "US10Y" });
    expect(withSym.conditions.map((c) => c.field)).toContain("instrument_id");
    expect(withSym.conditions.find((c) => c.field === "instrument_id")?.value).toEqual({
      kind: "text",
      text: "US10Y",
    });
  });

  it("names the deal fields the hedge vocabulary cannot express (counterparty is now representable)", () => {
    expect(HEDGE_SEED_UNREPRESENTED).toEqual(["tenor", "notional", "side"]);
    expect(HEDGE_SEED_UNREPRESENTED).not.toContain("counterparty");
  });
});

// --- blotter row → seed + navigate ------------------------------------------

describe("Deals row → Change hedging strategy", () => {
  it("right-clicking a row shows the action; invoking it seeds the deal AND navigates to hedging", async () => {
    const deal = fiDeal("Balyasny");
    const { app, setWorkspace } = makeApp({ deals: [deal] });
    state.app = app;
    await act(async () => {
      render(
        <HedgeSeedProvider>
          <SeedProbe deal={deal} />
          <DealsBlotterWorkspace />
        </HedgeSeedProvider>,
      );
    });

    const row = (await screen.findByText("Balyasny")).closest("tr") as HTMLElement;
    await act(async () => {
      fireEvent.contextMenu(row);
    });
    const menuItem = await screen.findByTestId("flow-change-hedging-strategy");
    expect(menuItem).toHaveTextContent("OIS USD");

    await act(async () => {
      fireEvent.click(menuItem);
    });
    // The deal is captured into the seed store, and we navigate to the `hedging` workspace.
    expect(screen.getByTestId("hseed-deal")).toHaveTextContent("deal-Balyasny");
    expect(setWorkspace).toHaveBeenCalledWith("hedging");
  });

  it("hides the item for a viewer lacking the `hedge` capability", async () => {
    const deal = fiDeal("Citadel");
    // Everything allowed EXCEPT hedge.
    const can: CanFn = (a) => a !== "hedge";
    state.app = makeApp({ deals: [deal], can }).app;
    await act(async () => {
      render(
        <HedgeSeedProvider>
          <DealsBlotterWorkspace />
        </HedgeSeedProvider>,
      );
    });
    const row = (await screen.findByText("Citadel")).closest("tr") as HTMLElement;
    await act(async () => {
      fireEvent.contextMenu(row);
    });
    // The acceptance item is still offered; the hedging item is absent.
    expect(await screen.findByTestId("flow-create-acceptance-rule")).toBeInTheDocument();
    expect(screen.queryByTestId("flow-change-hedging-strategy")).toBeNull();
  });
});

// --- the Exit Policy builder consumes the seed ------------------------------

describe("Hedging Exit Policy builder consumes the flow seed", () => {
  it("opens a NEW draft rule pre-filled from the deal + shows the seed hint, consuming the seed once", async () => {
    const deal = fiDeal("Balyasny");
    state.app = makeApp({}).app;
    await act(async () => {
      render(
        <HedgeSeedProvider>
          <SeedProbe deal={deal} />
          <HedgingWorkspace />
        </HedgeSeedProvider>,
      );
    });
    // Default policy tab loads; the list view is shown (no editor yet).
    await screen.findByTestId("hedge-create-rule");
    expect(screen.queryByTestId("hedge-rule-editor")).toBeNull();

    // Fire a seed for the deal → the builder opens a pre-filled draft editor with the hint.
    await act(async () => {
      fireEvent.click(screen.getByTestId("hseed-fire"));
    });

    expect(await screen.findByTestId("hedge-rule-editor")).toBeInTheDocument();
    const hint = await screen.findByTestId("hedge-seed-hint");
    expect(hint).toHaveTextContent("New rule seeded from deal #deal-Balyasny");
    expect(hint).toHaveTextContent("Balyasny");
    expect(hint).toHaveTextContent("OIS 5y");

    // The draft conditions scope the flow: Currency = USD AND Product = OIS AND
    // Desk = g10-rates AND Counterparty = Balyasny.
    const preview = screen.getByTestId("hedge-rule-preview");
    expect(preview).toHaveTextContent("Currency");
    expect(preview).toHaveTextContent("USD");
    expect(preview).toHaveTextContent("Product");
    expect(preview).toHaveTextContent("OIS");
    expect(preview).toHaveTextContent("Desk");
    expect(preview).toHaveTextContent("Counterparty");
    expect(preview).toHaveTextContent("Balyasny");
    // Nothing auto-saved.
    expect((state.app as ReturnType<typeof makeApp>["app"]).transport.updateHedgePolicyGraph).not
      .toHaveBeenCalled();

    // The seed is consumed exactly once (the store is now empty) — re-entry does not re-seed.
    await waitFor(() => expect(screen.getByTestId("hseed-deal")).toHaveTextContent(""));
  });

  it("cancelling the seeded draft returns to the list and does NOT re-open (seed consumed)", async () => {
    const deal = fiDeal("Point72");
    state.app = makeApp({}).app;
    await act(async () => {
      render(
        <HedgeSeedProvider>
          <SeedProbe deal={deal} />
          <HedgingWorkspace />
        </HedgeSeedProvider>,
      );
    });
    await screen.findByTestId("hedge-create-rule");
    await act(async () => {
      fireEvent.click(screen.getByTestId("hseed-fire"));
    });
    await screen.findByTestId("hedge-rule-editor");

    // Cancel the editor → back to the list; the consumed seed does not re-open it.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    });
    await waitFor(() => expect(screen.queryByTestId("hedge-rule-editor")).toBeNull());
    expect(await screen.findByTestId("hedge-create-rule")).toBeInTheDocument();
  });
});
