/**
 * ClientFlowWorkspace — render + format. Drives the table with `useApp` mocked (no
 * server): rows render, optional metrics show "—" (never NaN/0), net-negative $/mm
 * is coloured, a high fisher is flagged, sorting reorders, the group-by selector
 * re-queries, and the product filter re-filters the Asset grouping.
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import type { ClientFlowMetrics, FlowGroupBy } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { ClientFlowWorkspace } from "../src/workspaces/analytics/ClientFlowWorkspace";

function metric(overrides: Partial<ClientFlowMetrics> = {}): ClientFlowMetrics {
  return {
    label: "Millennium Capital",
    quoteCount: 120,
    tradedCount: 84,
    tradedNotional: 640_000_000,
    grossPnl: 82_000,
    totalMarkout: 9_000,
    totalHedgeCost: 6_500,
    netPnl: 66_500,
    dpmGross: 128,
    dpmNet: 104,
    capturedVsOffered: 0.73,
    meanCoverDistance: 0.6,
    breakevenSpread: 24,
    quoteToTradeRatio: 1.43,
    hitRate: 0.7,
    fishingScore: 0.02,
    ...overrides,
  };
}

const FISHER = metric({
  label: "IMC",
  quoteCount: 420,
  tradedCount: 0,
  tradedNotional: 0,
  grossPnl: 0,
  totalMarkout: 0,
  totalHedgeCost: 0,
  netPnl: 0,
  dpmGross: undefined,
  dpmNet: undefined,
  capturedVsOffered: undefined,
  meanCoverDistance: undefined,
  breakevenSpread: undefined,
  quoteToTradeRatio: undefined,
  hitRate: 0,
  fishingScore: 0.98,
});

const LOSER = metric({ label: "Balyasny", dpmNet: -83, netPnl: -1_000, fishingScore: 0.5 });

/** A mock app whose transport returns `byGroup[groupBy]` (defaulting to `rows`). */
function makeApp(
  rows: ClientFlowMetrics[],
  byGroup: Partial<Record<FlowGroupBy, ClientFlowMetrics[]>> = {},
) {
  const listClientFlowMetrics = vi.fn(async (gb: FlowGroupBy) => byGroup[gb] ?? rows);
  // A capturing notification seam: the workspace subscribes and refetches (debounced)
  // on every frame; `emit()` drives a frame so the test can assert the live revalidate.
  let onNotification: (() => void) | undefined;
  const streamNotifications = vi.fn(
    (_scope: unknown, cb: () => void) => {
      onNotification = cb;
      return () => {
        onNotification = undefined;
      };
    },
  );
  return {
    app: {
      transport: { listClientFlowMetrics, streamNotifications },
      auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true },
    },
    listClientFlowMetrics,
    streamNotifications,
    emit: (): void => onNotification?.(),
  };
}

async function renderWorkspace(app: unknown): Promise<void> {
  state.app = app;
  await act(async () => {
    render(<ClientFlowWorkspace />);
  });
}

afterEach(() => {
  cleanup();
  state.app = null;
});

describe("ClientFlowWorkspace", () => {
  it("searches by the row LABEL, which follows the grouping axis", async () => {
    // The label is a client here, a desk or a product under another grouping — so the
    // search deliberately matches the label rather than a fixed field, and stays correct
    // as the axis changes underneath it.
    const { app } = makeApp([metric(), FISHER, LOSER]);
    await renderWorkspace(app);
    expect(screen.getAllByRole("row").length).toBeGreaterThan(3);

    fireEvent.change(screen.getByLabelText(/Search by/), { target: { value: "balyasny" } });
    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(2); // header + Balyasny
    expect(rows[1]?.textContent).toContain("Balyasny");
  });

  it("filters to a FISHING band, which no text filter could reach", async () => {
    // The band is derived from the score, so it is not a printed value: IMC is 0.98
    // (high), the default metric 0.02 (low).
    const { app } = makeApp([metric(), FISHER, LOSER]);
    await renderWorkspace(app);

    fireEvent.change(screen.getByTestId("client-flow-band-filter"), {
      target: { value: "high" },
    });
    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(2);
    expect(rows[1]?.textContent).toContain("IMC");
  });

  it("renders a client-flow row with formatted $/mm and counts", async () => {
    const { app } = makeApp([metric()]);
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /Millennium Capital/ });
    const cells = within(row).getAllByRole("cell");
    // Column order: dpmGross, dpmNet, capt/offered, cover, breakeven, q/t, hit, fishing, …
    expect(cells[0]).toHaveTextContent("$128");
    expect(cells[1]).toHaveTextContent("$104");
    expect(within(row).getByText("70.0%")).toBeInTheDocument(); // hit-rate
  });

  it("shows '—' for every absent optional metric (never NaN or 0)", async () => {
    const { app } = makeApp([FISHER]);
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /IMC/ });
    // Five dashes: dpmGross, dpmNet, capt/offered, cover, breakeven, q/t (6 absent).
    const dashes = within(row).getAllByText("—");
    expect(dashes.length).toBeGreaterThanOrEqual(5);
    expect(within(row).queryByText(/NaN/)).toBeNull();
  });

  it("flags a high fisher (fishing score in the danger band)", async () => {
    const { app } = makeApp([FISHER]);
    await renderWorkspace(app);
    // The high-fisher meter carries the explanatory title.
    expect(screen.getByTitle(/High fisher/)).toBeInTheDocument();
    expect(screen.getByText("0.98")).toBeInTheDocument();
  });

  it("colours a net-negative $/mm row (the loser's dpmNet cell carries the negative class)", async () => {
    const { app } = makeApp([LOSER]);
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /Balyasny/ });
    const cells = within(row).getAllByRole("cell");
    expect(cells[1]).toHaveTextContent("-$83");
    expect(cells[1]?.className).toMatch(/negative/);
  });

  it("re-queries the server when the group-by selector changes", async () => {
    const byGroup: Partial<Record<FlowGroupBy, ClientFlowMetrics[]>> = {
      client: [metric()],
      asset: [metric({ label: "Fixed Income" }), metric({ label: "FX Options" })],
    };
    const { app, listClientFlowMetrics } = makeApp([metric()], byGroup);
    await renderWorkspace(app);
    expect(listClientFlowMetrics).toHaveBeenLastCalledWith("client");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Asset" }));
    });
    expect(listClientFlowMetrics).toHaveBeenLastCalledWith("asset");
    expect(screen.getByRole("row", { name: /Fixed Income/ })).toBeInTheDocument();
    expect(screen.getByRole("row", { name: /FX Options/ })).toBeInTheDocument();
  });

  it("filters to one product row under the Asset grouping (product filter re-filter)", async () => {
    const byGroup: Partial<Record<FlowGroupBy, ClientFlowMetrics[]>> = {
      asset: [metric({ label: "Fixed Income" }), metric({ label: "FX Options" })],
    };
    const { app } = makeApp([], byGroup);
    await renderWorkspace(app);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Asset" }));
    });
    // Product filter is enabled only on the Asset grouping.
    const fiBtn = screen.getByRole("button", { name: "Fixed Income" });
    expect(fiBtn).toBeEnabled();
    await act(async () => {
      fireEvent.click(fiBtn);
    });
    expect(screen.getByRole("row", { name: /Fixed Income/ })).toBeInTheDocument();
    expect(screen.queryByRole("row", { name: /FX Options/ })).toBeNull();
  });

  it("disables the product filter for non-asset groupings", async () => {
    const { app } = makeApp([metric()]);
    await renderWorkspace(app);
    // Default group-by is client → the FI/FXO product buttons are disabled.
    expect(screen.getByRole("button", { name: "Fixed Income" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "FX Options" })).toBeDisabled();
  });

  it("sorts by a numeric column and toggles direction on re-click", async () => {
    const { app } = makeApp([
      metric({ label: "AAA", dpmNet: 10 }),
      metric({ label: "BBB", dpmNet: 200 }),
      metric({ label: "CCC", dpmNet: 50 }),
    ]);
    await renderWorkspace(app);
    // Default sort is dpmNet desc → BBB (200), CCC (50), AAA (10).
    const bodyRows = () =>
      screen
        .getAllByRole("row")
        .filter((r) => within(r).queryByRole("rowheader"))
        .map((r) => within(r).getByRole("rowheader").textContent);
    expect(bodyRows()).toEqual(["BBB", "CCC", "AAA"]);

    // Click the "$/mm net" header → toggles to ascending.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /\$\/mm net/ }));
    });
    expect(bodyRows()).toEqual(["AAA", "CCC", "BBB"]);
  });

  it("refetches the current group-by on a notification frame (debounced, coalescing a storm)", async () => {
    vi.useFakeTimers();
    try {
      const { app, listClientFlowMetrics, emit } = makeApp([metric()]);
      state.app = app;
      await act(async () => {
        render(<ClientFlowWorkspace />);
      });
      // Mount did the first fetch (group-by "client"), and the workspace subscribed.
      expect(listClientFlowMetrics).toHaveBeenCalledTimes(1);
      expect(listClientFlowMetrics).toHaveBeenLastCalledWith("client");

      // A rapid burst of frames must coalesce into a single trailing-debounced refetch.
      await act(async () => {
        emit();
        emit();
        emit();
        vi.advanceTimersByTime(200);
        emit();
      });
      // Still within the 750ms window from the last frame → no refetch yet.
      expect(listClientFlowMetrics).toHaveBeenCalledTimes(1);

      await act(async () => {
        vi.advanceTimersByTime(750);
      });
      // Exactly one refetch for the whole burst, using the active group-by.
      expect(listClientFlowMetrics).toHaveBeenCalledTimes(2);
      expect(listClientFlowMetrics).toHaveBeenLastCalledWith("client");
    } finally {
      vi.useRealTimers();
    }
  });

  it("tears down the notification subscription and pending debounce on unmount", async () => {
    vi.useFakeTimers();
    try {
      const { app, listClientFlowMetrics, streamNotifications, emit } = makeApp([metric()]);
      state.app = app;
      let unmount: () => void = () => {};
      await act(async () => {
        ({ unmount } = render(<ClientFlowWorkspace />));
      });
      expect(streamNotifications).toHaveBeenCalledTimes(1);

      // A frame arrives, then we unmount before the debounce fires → no late refetch.
      await act(async () => {
        emit();
        vi.advanceTimersByTime(200);
        unmount();
      });
      await act(async () => {
        vi.advanceTimersByTime(1_000);
      });
      // Only the mount fetch ran; the pending debounce was cleared on unmount.
      expect(listClientFlowMetrics).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("sorts undefined metrics last regardless of direction", async () => {
    const { app } = makeApp([
      metric({ label: "HasVal", dpmNet: 5 }),
      metric({ label: "NoVal", dpmNet: undefined, tradedNotional: 0 }),
    ]);
    await renderWorkspace(app);
    // dpmNet desc: the defined value first, the undefined last.
    const rowHeaders = screen
      .getAllByRole("rowheader")
      .map((h) => h.textContent);
    expect(rowHeaders[rowHeaders.length - 1]).toBe("NoVal");
  });
});
