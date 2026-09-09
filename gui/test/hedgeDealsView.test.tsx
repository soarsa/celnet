/**
 * HedgeDealsView — the executed-hedge ledger lens of the Deals blotter, driven with
 * `useApp` mocked (no server). Covers: a fired hedge renders the full picture — the LP
 * we hit (`lpWon`), the LP panel fanned to, the realised hedge price + mid + slippage,
 * and the internal/external/residual amounts; an empty trail shows the honest note;
 * and a non-`hedge` identity sees the capability note WITHOUT the read being attempted.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import type { HedgeProvenance, StreetOrder } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { HedgeDealsView } from "../src/workspaces/HedgeDealsView";
import { defaultExitAction } from "../src/lib/hedgeExit";

function provenance(over: Partial<HedgeProvenance> = {}): HedgeProvenance {
  return {
    hedgeId: "hedge-1",
    book: "EMEA Rates",
    instrument: "USD-OIS 10y",
    firedAt: Date.UTC(2026, 7, 7, 9, 30, 0),
    metric: "dv01",
    threshold: 100,
    netRisk: 145,
    utilization: 1.45,
    band: "red",
    policyPath: [0, 1],
    // An EXTERNAL exit action (submit_market_order) — the desk shows external hedges
    // by default; an internalised action (warehouse / cross_internal / …) is hidden.
    action: defaultExitAction("submit_market_order"),
    // `self` vehicle ⇒ no unit arithmetic to record (contract.ts:4669-4672)
    vehiclePlan: null,
    internalCrossed: 30_000_000,
    externalHedged: 70_000_000,
    residual: 5_000_000,
    hedgePrice: 100.262,
    midAtFire: 100.25,
    slippageBp: 1.2,
    lpWon: "JPM",
    advisory: false,
    lps: ["JPM", "Citi", "Barclays"],
    ...over,
  };
}

/** A street order as the analytics seam reports it, joined to its parent hedge. */
function order(over: Partial<StreetOrder> = {}): StreetOrder {
  return {
    orderId: "SO-1",
    tsNanos: 1_700_000_000_000_000_000n,
    lpId: "cme-sim",
    venue: "named_lp",
    instrument: "ZTU26",
    family: "bond_future",
    tenorYears: 2,
    side: "sell",
    requestedQty: 1_800_000,
    filledQty: 1_800_000,
    requestedPrice: 103.5,
    filledPrice: 103.5,
    slippageBp: 0,
    outcome: "filled",
    reason: undefined,
    competitors: [],
    parentHedgeId: "HDG-1",
    parentPositionId: 7n,
    orderType: undefined,
    timeInForce: undefined,
    responseLatencyNanos: undefined,
    ...over,
  };
}

function makeApp(
  opts: { can?: boolean; rows?: HedgeProvenance[]; orders?: StreetOrder[] } = {},
) {
  const listHedgeProvenance = vi.fn(async () => opts.rows ?? []);
  const listStreetOrders = vi.fn(async () => ({
    orders: opts.orders ?? [],
    breakdown: [],
    totalMatching: (opts.orders ?? []).length,
  }));
  return {
    app: {
      transport: {
        label: "in-app",
        listHedgeProvenance,
        listStreetOrders,
        streamHedgeIntents: vi.fn(() => () => {}),
      },
      auth: { can: () => opts.can ?? true },
    },
    listHedgeProvenance,
    listStreetOrders,
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("HedgeDealsView", () => {
  it("renders the full hedge picture — LP hit, price, mid, slippage, amounts", async () => {
    state.app = makeApp({ can: true, rows: [provenance()] }).app;
    render(<HedgeDealsView />);

    const row = await screen.findByTestId("hedge-deal-row-hedge-1");
    const scope = within(row);
    // Who we hedged with (the winner) + the panel we fanned to.
    expect(scope.getByTestId("hedge-lpwon-hedge-1")).toHaveTextContent("JPM");
    expect(scope.getByText("Citi")).toBeInTheDocument();
    expect(scope.getByText("Barclays")).toBeInTheDocument();
    // The realised price + mid + slippage.
    expect(scope.getByText("100.2620")).toBeInTheDocument();
    expect(scope.getByText("100.2500")).toBeInTheDocument();
    expect(scope.getByText("+1.2bp")).toBeInTheDocument();
    // The amounts (internal cross / external / residual), compact.
    expect(scope.getByText("30m")).toBeInTheDocument();
    expect(scope.getByText("70m")).toBeInTheDocument();
    expect(scope.getByText("5m")).toBeInTheDocument();
    // A live external fire.
    expect(scope.getByText("LIVE")).toBeInTheDocument();
  });

  it("reconciles a per-fill execution record to its parent deal via parent_position_id", async () => {
    state.app = makeApp({
      can: true,
      rows: [provenance({ hedgeId: "HDG-7", advisory: true, parentPositionId: 9001n })],
    }).app;
    render(<HedgeDealsView />);

    const row = await screen.findByTestId("hedge-deal-row-HDG-7");
    // The parent-deal link cell carries the `#<position_id>` reconciliation reference.
    expect(within(row).getByTestId("hedge-parent-HDG-7")).toHaveTextContent("#9001");
    // An advisory external-shed fill still renders (it is NOT hidden) — it reads ADVISORY.
    expect(within(row).getByText("ADVISORY")).toBeInTheDocument();
  });

  it("renders an em dash in the parent-deal cell for a book-level record (no parent)", async () => {
    state.app = makeApp({ can: true, rows: [provenance({ hedgeId: "HDG-8" })] }).app;
    render(<HedgeDealsView />);
    const cell = await screen.findByTestId("hedge-parent-HDG-8");
    expect(cell).toHaveTextContent("—");
  });

  it("hides internalised (warehouse) decisions by default, reveals them via the toggle", async () => {
    const warehouse = provenance({
      hedgeId: "WH-1",
      action: defaultExitAction("warehouse"),
      // `self` vehicle ⇒ no unit arithmetic to record (contract.ts:4669-4672)
      vehiclePlan: null,
      lpWon: null,
      externalHedged: 0,
    });
    state.app = makeApp({ can: true, rows: [provenance(), warehouse] }).app;
    render(<HedgeDealsView />);

    // The external hedge shows; the warehouse (internalised) row is hidden by default.
    await screen.findByTestId("hedge-deal-row-hedge-1");
    expect(screen.queryByTestId("hedge-deal-row-WH-1")).not.toBeInTheDocument();

    // Turn on "Show internalised" — the warehouse row now appears.
    fireEvent.click(screen.getByTestId("hedge-show-internalised"));
    expect(await screen.findByTestId("hedge-deal-row-WH-1")).toBeInTheDocument();
  });

  it("renders the fired time from epoch NANOS, not as an out-of-range date", async () => {
    // `HedgeProvenance.fired_at` is epoch nanoseconds. Passing it straight to `new Date`
    // is ~57,000 years out of range, so the whole column rendered "Invalid Date" — the
    // kind of break that survives because nobody reads a timestamp they already know.
    const built = makeApp({
      rows: [provenance({ hedgeId: "HDG-T", firedAt: 1_787_173_452_008_410_400 })],
    });
    state.app = built.app;
    render(<HedgeDealsView />);
    const row = await screen.findByTestId("hedge-deal-row-HDG-T");
    expect(row.textContent).not.toContain("Invalid Date");
    // Asserted as a SHAPE, not a literal clock: the rendered time is local, so pinning
    // "21:04:12" only passes in UTC and fails everywhere else — a test that breaks on the
    // reader's timezone tells you nothing about the bug it was written for.
    expect(row.textContent).toMatch(/^\d{2}:\d{2}:\d{2}/);
  });

  it("shows the ORDERS a hedge put on the wire — the half the ledger never carried", async () => {
    // The ledger records the DECISION. Until now nothing on this screen said which
    // provider was asked, for how much, or what came back — so a hedge that did not
    // reduce the book looked identical to one that did.
    const built = makeApp({
      rows: [provenance({ hedgeId: "HDG-1", externalHedged: 308.4 })],
      orders: [
        order({ orderId: "SO-4", filledQty: 1_800_000, outcome: "filled" }),
        order({
          orderId: "SO-3",
          filledQty: 0,
          filledPrice: undefined,
          outcome: "rejected",
          reason: "NOT_A_WHOLE_LOT",
        }),
      ],
    });
    state.app = built.app;
    render(<HedgeDealsView />);

    // The count is visible on the row without opening anything.
    const toggle = await screen.findByTestId("hedge-orders-toggle-HDG-1");
    expect(toggle.textContent).toContain("2 sent");
    expect(toggle.textContent).toContain("1 filled");
    expect(screen.queryByTestId("trade-details-modal")).toBeNull();

    fireEvent.click(toggle);
    // The SHARED modal — the same surface the client blotter opens for a client fill.
    const modal = screen.getByTestId("trade-details-modal");
    expect(within(modal).getByTestId("trade-details-kind").textContent).toBe("Hedge");
    expect(within(modal).getByTestId("trade-details-order-SO-4")).toBeTruthy();

    // …and the REASON is printed, not tooltipped: it is the whole diagnosis when a
    // hedge fires and the book does not move.
    expect(within(modal).getByTestId("trade-details-order-reason").textContent).toBe(
      "NOT A WHOLE LOT",
    );
  });

  it("distinguishes an INTERNALISED decision from one whose orders are missing", async () => {
    // An internalised hedge never asks the street, so having no orders is a fact about
    // what it did — not missing data. It stays OPENABLE, because the decision itself is
    // still worth reading; what changes is what the modal says about the absence.
    const built = makeApp({
      rows: [
        provenance({
          hedgeId: "HDG-9",
          action: defaultExitAction("warehouse"),
          vehiclePlan: null,
          lpWon: null,
          externalHedged: 0,
        }),
      ],
      orders: [],
    });
    state.app = built.app;
    render(<HedgeDealsView />);
    // Internalised rows are hidden by default — reveal them, then read the cell.
    fireEvent.click(await screen.findByTestId("hedge-show-internalised"));
    const toggle = await screen.findByTestId("hedge-orders-toggle-HDG-9");
    expect(toggle.textContent).toBe("—");

    fireEvent.click(toggle);
    expect(screen.getByTestId("trade-details-no-orders").textContent).toContain(
      "never asks the street",
    );
  });

  it("shows the honest empty note when no hedges have fired", async () => {
    state.app = makeApp({ can: true, rows: [] }).app;
    render(<HedgeDealsView />);
    expect(await screen.findByText(/No fired hedges yet/i)).toBeInTheDocument();
  });

  it("gates a non-hedge identity WITHOUT attempting the read", async () => {
    const built = makeApp({ can: false });
    state.app = built.app;
    render(<HedgeDealsView />);
    expect(screen.getByText(/requires the/i)).toBeInTheDocument();
    // The gated view never calls the server-gated read.
    expect(built.listHedgeProvenance).not.toHaveBeenCalled();
  });
});
