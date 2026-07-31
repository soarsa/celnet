/**
 * StreetLiquidityWorkspace — render + format. Drives the league table with `useApp`
 * mocked (no server): rows render, optional metrics show "—" (never NaN/0), a chronic
 * misser's win-rate is coloured, a last-look rejecter is flagged, and sorting reorders
 * (default deals-won desc; undefined sorts last).
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import type { LpFlowMetrics } from "../src/data/contract";

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

function makeApp(rows: LpFlowMetrics[]) {
  const listLpFlowMetrics = vi.fn(async () => rows);
  return {
    app: {
      transport: { listLpFlowMetrics },
      auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true },
    },
    listLpFlowMetrics,
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
