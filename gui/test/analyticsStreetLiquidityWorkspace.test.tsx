/**
 * StreetLiquidityWorkspace — render + format. Drives the league table with `useApp`
 * mocked (no server): rows render, optional metrics show "—" (never NaN/0), a chronic
 * misser's win-rate is coloured, a last-look rejecter is flagged, and sorting reorders
 * (default deals-won desc; undefined sorts last).
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import type {
  LpFlowMetrics,
  StreetBreakdownRow,
  StreetOrder,
  StreetOrdersView,
} from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { StreetLiquidityWorkspace } from "../src/workspaces/analytics/StreetLiquidityWorkspace";

function lp(overrides: Partial<LpFlowMetrics> = {}): LpFlowMetrics {
  return {
    lpId: "Citadel Securities",
    tickCount: 48_200,
    quoteCount: 980,
    dealsWon: 612,
    wonNotional: 4_100_000_000,
    missed: 360,
    lastLookRejects: 4,
    winRate: 0.624,
    meanCover: 0.8,
    ...overrides,
  };
}

const MISSER = lp({
  lpId: "DNB Markets",
  tickCount: 15_200,
  quoteCount: 720,
  dealsWon: 22,
  wonNotional: 90_000_000,
  missed: 690,
  lastLookRejects: 8,
  winRate: 22 / 720,
  meanCover: 3.3,
});

const REJECTER = lp({
  lpId: "Handelsbanken",
  quoteCount: 520,
  dealsWon: 40,
  lastLookRejects: 180,
  winRate: 40 / 520,
  meanCover: 3.1,
});

const DORMANT = lp({
  lpId: "Raiffeisen",
  tickCount: 800,
  quoteCount: 0,
  dealsWon: 0,
  wonNotional: 0,
  missed: 0,
  lastLookRejects: 0,
  winRate: undefined,
  meanCover: undefined,
});

/** The street-side EXECUTION half of the workspace queries this alongside the league
 *  table; the default stub returns an honest empty view so the league-table cases
 *  render unchanged. Pass `street` to exercise the execution panels. */
const EMPTY_STREET: StreetOrdersView = { orders: [], breakdown: [], totalMatching: 0 };

function makeApp(rows: LpFlowMetrics[], street: StreetOrdersView = EMPTY_STREET) {
  const listLpFlowMetrics = vi.fn(async () => rows);
  const listStreetOrders = vi.fn(async () => street);
  return {
    app: {
      transport: { listLpFlowMetrics, listStreetOrders },
      auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true },
    },
    listLpFlowMetrics,
    listStreetOrders,
  };
}

async function renderWorkspace(app: unknown): Promise<void> {
  state.app = app;
  await act(async () => {
    render(<StreetLiquidityWorkspace />);
  });
}

afterEach(() => {
  cleanup();
  state.app = null;
});

describe("StreetLiquidityWorkspace", () => {
  it("renders an LP row with a formatted tick rate, counts and compact notional", async () => {
    const { app } = makeApp([lp()]);
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /Citadel Securities/ });
    const cells = within(row).getAllByRole("cell");
    // Column order: tickRate, quotes, dealsWon, wonNotional, missed, lastLook, winRate, meanCover.
    expect(cells[0]).toHaveTextContent("48.2K"); // compact tick rate
    expect(cells[2]).toHaveTextContent("612"); // deals won
    expect(cells[3]).toHaveTextContent(/\$4\.1B/); // won notional, compact
    expect(within(row).getByText("62.4%")).toBeInTheDocument(); // win-rate
    expect(within(row).getByText("0.8 bps")).toBeInTheDocument(); // mean cover
  });

  it("shows '—' for absent win-rate and mean-cover on a dormant LP (never NaN or 0)", async () => {
    const { app } = makeApp([DORMANT]);
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /Raiffeisen/ });
    const dashes = within(row).getAllByText("—");
    expect(dashes.length).toBeGreaterThanOrEqual(2); // win-rate + mean-cover
    expect(within(row).queryByText(/NaN/)).toBeNull();
  });

  it("colours a chronic misser's win-rate (the low band carries the danger styling)", async () => {
    const { app } = makeApp([MISSER]);
    await renderWorkspace(app);
    expect(screen.getByTitle(/Chronic misser/)).toBeInTheDocument();
  });

  it("flags a last-look rejecter's reject count", async () => {
    const { app } = makeApp([REJECTER]);
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /Handelsbanken/ });
    expect(within(row).getByTitle(/Last-look rejecter/)).toHaveTextContent("180");
  });

  it("defaults to deals-won descending and toggles direction on re-click", async () => {
    const { app } = makeApp([
      lp({ lpId: "AAA", dealsWon: 10 }),
      lp({ lpId: "BBB", dealsWon: 200 }),
      lp({ lpId: "CCC", dealsWon: 50 }),
    ]);
    await renderWorkspace(app);
    const bodyRows = () =>
      screen
        .getAllByRole("row")
        .filter((r) => within(r).queryByRole("rowheader"))
        .map((r) => within(r).getByRole("rowheader").textContent);
    // Default: deals-won desc → BBB (200), CCC (50), AAA (10).
    expect(bodyRows()).toEqual(["BBB", "CCC", "AAA"]);

    // Click the "Deals won" header → toggles to ascending.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Deals won/ }));
    });
    expect(bodyRows()).toEqual(["AAA", "CCC", "BBB"]);
  });

  it("sorts an absent win-rate last regardless of direction", async () => {
    const { app } = makeApp([lp({ lpId: "HasVal", winRate: 0.3 }), DORMANT]);
    await renderWorkspace(app);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Win rate/ }));
    });
    const rowHeaders = screen.getAllByRole("rowheader").map((h) => h.textContent);
    // Win-rate desc: the defined value first, the absent (Raiffeisen) last.
    expect(rowHeaders[rowHeaders.length - 1]).toBe("Raiffeisen");
  });
});

// --- the street-side EXECUTION panels ---------------------------------------
//
// The property under test is the one the whole surface exists to protect: a metric
// the server reported as ABSENT must render as an explicit marker, and must never be
// substituted with a plausible zero. The complementary case matters just as much — a
// genuinely OBSERVED zero must still render as 0.

/** A breakdown row where some metrics are observed and others genuinely absent. */
function breakdownRow(over: Partial<StreetBreakdownRow> = {}): StreetBreakdownRow {
  return {
    dimension: "lp",
    key: "citigroup-sim",
    orders: 4,
    filled: 3,
    partiallyFilled: 1,
    rejected: 0,
    cancelled: 0,
    expired: 0,
    lastLookPulled: 0,
    noLiquidity: 0,
    compositeBackstop: 0,
    requestedQty: 40_000,
    filledQty: 32_500,
    fillRatio: 0.8125,
    winRate: 1,
    meanSlippageBp: 0.4,
    meanResponseLatencyNanos: undefined,
    lastLookRate: 0,
    meanCover: undefined,
    meanCompetitors: 2.5,
    ...over,
  };
}

/** A blotter order with the unobserved routing fields absent. */
function streetOrder(over: Partial<StreetOrder> = {}): StreetOrder {
  return {
    orderId: "SO-1",
    tsNanos: 1_700_000_000_000_000_000n,
    lpId: "citigroup-sim",
    venue: "named_lp",
    instrument: "USSW10",
    family: "ois",
    tenorYears: 10,
    side: "sell",
    requestedQty: 4000,
    filledQty: 4000,
    requestedPrice: 0.03,
    filledPrice: 0.03005,
    slippageBp: 0.5,
    outcome: "filled",
    reason: undefined,
    competitors: [
      { lpId: "citigroup-sim", price: 0.03005 },
      { lpId: "jpm-sim", price: 0.03007 },
    ],
    parentHedgeId: "HDG-1",
    parentPositionId: 42n,
    orderType: undefined,
    timeInForce: undefined,
    responseLatencyNanos: undefined,
    ...over,
  };
}

describe("StreetExecution (the order-level half of the workspace)", () => {
  it("renders an absent metric as the marker and a real zero as 0", async () => {
    const { app } = makeApp([], {
      orders: [streetOrder()],
      breakdown: [breakdownRow()],
      totalMatching: 1,
    });
    await renderWorkspace(app);

    // An OBSERVED zero stays a zero: 0 last-look pulls out of 4 orders is 0.0%.
    expect(screen.getByText("0.0%")).toBeTruthy();
    // An ABSENT metric (no order in the group had a cover, none carried a measured
    // round trip) renders the marker — and there is more than one such cell.
    const absent = screen
      .getAllByTitle("Genuinely absent — not observed, not zero")
      .map((n) => n.textContent);
    expect(absent.length).toBeGreaterThanOrEqual(2);
    expect(absent.every((t) => t === "—")).toBe(true);
    // Nothing anywhere turned an absence into a zero latency.
    expect(screen.queryByText("0 ns")).toBeNull();
  });

  it("calls an unroutable order a SETUP fault, not a market one, and says which LP", async () => {
    // `no_order_endpoint` means the order never left the building: the LP is quoting,
    // but no order route is configured, so nobody was asked. On the wire it arrives as
    // the same `no_liquidity` outcome a genuinely quiet market produces — and the two
    // demand opposite responses (fix a setting vs widen the panel / accept the risk),
    // so the UI must not render them identically.
    const { app } = makeApp([], {
      orders: [
        streetOrder({
          orderId: "SO-CFG",
          lpId: "citigroup-sim",
          outcome: "no_liquidity",
          reason: "no_order_endpoint",
          filledQty: 0,
          filledPrice: undefined,
          slippageBp: undefined,
        }),
      ],
      breakdown: [breakdownRow()],
      totalMatching: 1,
    });
    await renderWorkspace(app);

    // The chip does NOT borrow the market-shaped label. Asserted on the CHIP itself —
    // the outcome FILTER also lists "No liquidity" as an <option>, and matching that
    // would make this pass for the wrong reason.
    const chip = screen.getByTestId("street-outcome-chip");
    expect(chip.textContent).toBe("Not sent · setup");
    expect(chip.getAttribute("data-config-fault")).toBe("true");

    // A standing banner states it without needing a hover, names the provider, and
    // points at where the setting lives.
    const banner = screen.getByTestId("street-config-fault");
    expect(banner.textContent).toContain("could not");
    expect(banner.textContent).toContain("citigroup-sim");
    expect(banner.textContent).toContain("Administration");
  });

  it("leaves a GENUINE no-price alone — no setup banner, no borrowed label", async () => {
    // The counter-case that stops the banner becoming noise: `no_firm_lp_price` IS a
    // market fact (nobody showed an executable price), so it keeps the market label and
    // raises no configuration alarm.
    const { app } = makeApp([], {
      orders: [
        streetOrder({
          orderId: "SO-MKT",
          lpId: undefined,
          outcome: "no_liquidity",
          reason: "no_firm_lp_price",
          filledQty: 0,
          filledPrice: undefined,
          slippageBp: undefined,
          competitors: [],
        }),
      ],
      breakdown: [breakdownRow()],
      totalMatching: 1,
    });
    await renderWorkspace(app);

    const chip = screen.getByTestId("street-outcome-chip");
    expect(chip.textContent).toBe("No liquidity");
    expect(chip.getAttribute("data-config-fault")).toBeNull();
    expect(screen.queryByTestId("street-config-fault")).toBeNull();
  });

  it("PRINTS a venue reject reason on the row instead of hiding it in a hover", async () => {
    // The UAT report of 2026-08-20: five futures sheds came back "Rejected" with no
    // stated reason. The reason was on the wire the whole time (`NOT_A_WHOLE_LOT`) and
    // only ever reachable by hovering — which is the same mistake as the config-fault
    // tooltip, and nobody hovers a blotter.
    const { app } = makeApp([], {
      orders: [
        streetOrder({
          orderId: "SO-24",
          lpId: "cme-sim",
          instrument: "ZTU26",
          outcome: "rejected",
          reason: "NOT_A_WHOLE_LOT",
          filledQty: 0,
          filledPrice: undefined,
          slippageBp: undefined,
        }),
      ],
      breakdown: [breakdownRow()],
      totalMatching: 1,
    });
    await renderWorkspace(app);

    // The chip still says what happened…
    expect(screen.getByTestId("street-outcome-chip").textContent).toBe("Rejected");
    // …and the row now also says WHY, visibly.
    const reason = screen.getByTestId("street-outcome-reason");
    expect(reason.textContent).toBe("not a whole lot");
    // A venue refusal is the counterparty's answer, not our configuration — it must not
    // be relabelled as a setup fault or raise the setup banner.
    expect(
      screen.getByTestId("street-outcome-chip").getAttribute("data-config-fault"),
    ).toBeNull();
    expect(screen.queryByTestId("street-config-fault")).toBeNull();
  });

  it("prints an UNKNOWN venue code rather than swallowing it", async () => {
    // A code we have no gloss for is still the most useful text on the row; show it
    // de-underscored rather than dropping it because it is not in the vocabulary.
    const { app } = makeApp([], {
      orders: [
        streetOrder({
          orderId: "SO-X",
          lpId: "cme-sim",
          outcome: "rejected",
          reason: "SOME_NEW_VENUE_CODE",
          filledQty: 0,
          filledPrice: undefined,
          slippageBp: undefined,
        }),
      ],
      breakdown: [breakdownRow()],
      totalMatching: 1,
    });
    await renderWorkspace(app);
    expect(screen.getByTestId("street-outcome-reason").textContent).toBe(
      "SOME NEW VENUE CODE",
    );
  });

  it("labels a composite backstop explicitly and never as an LP", async () => {
    const { app } = makeApp([], {
      orders: [
        streetOrder({
          orderId: "SO-2",
          lpId: undefined,
          venue: "composite_backstop",
          outcome: "no_liquidity",
          reason: "no_firm_lp_price",
          filledQty: 0,
          filledPrice: undefined,
          slippageBp: undefined,
          competitors: [],
        }),
      ],
      breakdown: [
        breakdownRow({ key: "COMPOSITE", compositeBackstop: 1, orders: 1, filled: 0 }),
      ],
      totalMatching: 1,
    });
    await renderWorkspace(app);

    // The explicit bucket, spelled out — not merged into an LP and not hidden.
    expect(screen.getAllByText("Composite backstop").length).toBeGreaterThanOrEqual(1);
    // The outcome chip (the <option> in the filter select carries the same text).
    expect(screen.getAllByText("No liquidity").length).toBeGreaterThanOrEqual(2);
  });

  it("shows the pre-page total so the blotter never implies it has every row", async () => {
    const { app } = makeApp([], {
      orders: [streetOrder()],
      breakdown: [breakdownRow()],
      totalMatching: 5312,
    });
    await renderWorkspace(app);
    expect(screen.getByText(/Showing 1 of 5,312 matching orders/)).toBeTruthy();
  });

  it("renders an honest empty state rather than a fabricated row", async () => {
    const { app, listStreetOrders } = makeApp([]);
    await renderWorkspace(app);
    expect(listStreetOrders).toHaveBeenCalled();
    expect(screen.getByText(/No street orders recorded in this window/)).toBeTruthy();
  });
});
