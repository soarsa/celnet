/**
 * LatencyOpsWorkspace — render + format. Drives the table with `useApp` mocked (no
 * server): rows render in pipeline order, latency formats in adaptive units (ns/µs/ms),
 * an unsampled (zero) field shows "—", the health strip renders, and clicking a column
 * header sorts + toggles direction.
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import type { LatencyMetrics, LatencyStage } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { LatencyOpsWorkspace, latencyTone } from "../src/workspaces/analytics/LatencyOpsWorkspace";

function stage(overrides: Partial<LatencyStage> = {}): LatencyStage {
  return {
    op: "vanilla_price",
    stageLabel: "Price (pinned core)",
    count: 12_345,
    p50Ns: 820,
    p99Ns: 2_400,
    p999Ns: 5_200,
    p9999Ns: 9_100,
    minNs: 240,
    maxNs: 14_000,
    meanNs: 910.4,
    ...overrides,
  };
}

const SURFACE = stage({
  op: "surface_vol",
  stageLabel: "Surface / curve rebuild",
  count: 38_400,
  p50Ns: 42_000,
  p99Ns: 88_000,
  p999Ns: 0, // unsampled percentile ⇒ "—"
  p9999Ns: 210_000,
  minNs: 18_000,
  maxNs: 262_000,
  meanNs: 51_320.5,
});

const BOOK = stage({
  op: "book",
  stageLabel: "Ack→fill→book",
  count: 41_000,
  p50Ns: 2_600_000,
  p99Ns: 6_100_000,
  p999Ns: 9_400_000,
  p9999Ns: 13_000_000,
  minNs: 1_100_000,
  maxNs: 17_000_000,
  meanNs: 2_980_000,
});

function metrics(stages: LatencyStage[]): LatencyMetrics {
  return {
    stages,
    health: { drainedTotal: 100_000, droppedTotal: 12, observedGaps: 0, tickHz: 24_000_000 },
  };
}

function makeApp(m: LatencyMetrics) {
  const listLatencyMetrics = vi.fn(async () => m);
  // A capturing notification seam: the workspace subscribes and refetches (debounced)
  // on every frame; `emit()` drives a frame so the test can assert the live revalidate.
  let onNotification: (() => void) | undefined;
  const streamNotifications = vi.fn((_scope: unknown, cb: () => void) => {
    onNotification = cb;
    return () => {
      onNotification = undefined;
    };
  });
  return {
    app: {
      transport: { listLatencyMetrics, streamNotifications },
      auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true },
    },
    listLatencyMetrics,
    streamNotifications,
    emit: (): void => onNotification?.(),
  };
}

async function renderWorkspace(app: unknown): Promise<void> {
  state.app = app;
  await act(async () => {
    render(<LatencyOpsWorkspace />);
  });
}

/** Body-row rowheaders (the Stage column), in DOM order. */
function stageOrder(): string[] {
  return screen
    .getAllByRole("rowheader")
    .map((h) => h.textContent ?? "");
}

afterEach(() => {
  cleanup();
  state.app = null;
});

describe("LatencyOpsWorkspace", () => {
  it("searches stages by label AND by raw op tag", async () => {
    // Two names for one row: the label is what the eye reads on screen, the op is what a
    // trace or a log line carries. Someone arriving from either must find the stage.
    const { app } = makeApp(metrics([stage(), BOOK]));
    await renderWorkspace(app);
    expect(screen.getAllByRole("row")).toHaveLength(3); // header + 2 stages

    const search = screen.getByLabelText("Search stages");
    fireEvent.change(search, { target: { value: "Ack→fill" } });
    expect(screen.getAllByRole("row")).toHaveLength(2);

    // …and the same row by its op tag, which never appears in the label.
    fireEvent.change(search, { target: { value: "vanilla_price" } });
    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(2);
    expect(rows[1]?.textContent).toContain("Price (pinned core)");
  });

  it("filters to a latency BAND — the question this table exists to answer", async () => {
    // The band is derived from p99, so it is not a value any text filter could reach:
    // `stage()` is well inside budget, BOOK is milliseconds and therefore red.
    const { app } = makeApp(metrics([stage(), BOOK]));
    await renderWorkspace(app);

    fireEvent.change(screen.getByTestId("latency-tone-filter"), { target: { value: "red" } });
    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(2);
    expect(rows[1]?.textContent).toContain("Ack→fill→book");
  });

  it("says so honestly when nothing matches, rather than showing an empty grid", async () => {
    const { app } = makeApp(metrics([stage()]));
    await renderWorkspace(app);
    fireEvent.change(screen.getByLabelText("Search stages"), {
      target: { value: "no-such-stage" },
    });
    expect(screen.getByText(/No stage matches this search or band/)).toBeTruthy();
  });

  it("renders a stage row with adaptive-unit latencies (ns and µs)", async () => {
    const { app } = makeApp(metrics([stage()]));
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /Price \(pinned core\)/ });
    const cells = within(row).getAllByRole("cell");
    // Column order: count, p50, p99, p99.9, max, mean, spectrum.
    expect(cells[1]).toHaveTextContent("820 ns");
    expect(cells[2]).toHaveTextContent("2.40 µs");
  });

  it("formats a millisecond stage in ms", async () => {
    const { app } = makeApp(metrics([BOOK]));
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /Ack→fill→book/ });
    const cells = within(row).getAllByRole("cell");
    expect(cells[1]).toHaveTextContent("2.60 ms");
    expect(cells[2]).toHaveTextContent("6.10 ms");
  });

  it("shows '—' for an unsampled (zero) percentile, never '0 ns'", async () => {
    const { app } = makeApp(metrics([SURFACE]));
    await renderWorkspace(app);
    const row = screen.getByRole("row", { name: /Surface \/ curve rebuild/ });
    // p99.9 is the 4th metric cell (index 3) and is zero ⇒ "—".
    const cells = within(row).getAllByRole("cell");
    expect(cells[3]).toHaveTextContent("—");
    expect(within(row).queryByText(/0 ns/)).toBeNull();
    expect(within(row).queryByText(/NaN/)).toBeNull();
  });

  it("renders the telemetry-health strip (drained / dropped / gaps / tick freq)", async () => {
    const { app } = makeApp(metrics([stage()]));
    await renderWorkspace(app);
    expect(screen.getByText("Drained")).toBeInTheDocument();
    expect(screen.getByText("Dropped")).toBeInTheDocument();
    expect(screen.getByText("Gaps")).toBeInTheDocument();
    expect(screen.getByText("Tick freq")).toBeInTheDocument();
    expect(screen.getByText("100,000")).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument();
    expect(screen.getByText("24 MHz")).toBeInTheDocument();
  });

  it("preserves the server pipeline order until a header is clicked", async () => {
    const { app } = makeApp(metrics([stage(), SURFACE, BOOK]));
    await renderWorkspace(app);
    const order = stageOrder();
    expect(order[0]).toContain("Price (pinned core)");
    expect(order[1]).toContain("Surface / curve rebuild");
    expect(order[2]).toContain("Ack→fill→book");
  });

  it("sorts by a numeric column (p99 desc) and toggles to ascending on re-click", async () => {
    const { app } = makeApp(metrics([stage(), SURFACE, BOOK]));
    await renderWorkspace(app);

    // Click "p99" → descending by p99: book (6.1ms) > surface (88µs) > price (2.4µs).
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /p99$/ }));
    });
    let order = stageOrder();
    expect(order[0]).toContain("Ack→fill→book");
    expect(order[2]).toContain("Price (pinned core)");

    // Re-click → ascending: price, surface, book.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /p99$/ }));
    });
    order = stageOrder();
    expect(order[0]).toContain("Price (pinned core)");
    expect(order[2]).toContain("Ack→fill→book");
  });

  it("refetches on a notification frame (debounced) and on the idle interval", async () => {
    vi.useFakeTimers();
    try {
      const { app, listLatencyMetrics, emit } = makeApp(metrics([stage()]));
      state.app = app;
      await act(async () => {
        render(<LatencyOpsWorkspace />);
      });
      // Mount did the first fetch and the workspace subscribed.
      expect(listLatencyMetrics).toHaveBeenCalledTimes(1);

      // A burst of frames coalesces into a single trailing-debounced refetch.
      await act(async () => {
        emit();
        emit();
        vi.advanceTimersByTime(200);
        emit();
      });
      expect(listLatencyMetrics).toHaveBeenCalledTimes(1); // still within the 750ms window
      await act(async () => {
        vi.advanceTimersByTime(750);
      });
      expect(listLatencyMetrics).toHaveBeenCalledTimes(2);

      // The idle interval keeps refreshing even with no notification traffic (5s cadence).
      await act(async () => {
        vi.advanceTimersByTime(5_000);
      });
      expect(listLatencyMetrics).toHaveBeenCalledTimes(3);
      await act(async () => {
        vi.advanceTimersByTime(5_000);
      });
      expect(listLatencyMetrics).toHaveBeenCalledTimes(4);
    } finally {
      vi.useRealTimers();
    }
  });

  it("tears down the subscription, debounce and interval on unmount", async () => {
    vi.useFakeTimers();
    try {
      const { app, listLatencyMetrics, streamNotifications, emit } = makeApp(metrics([stage()]));
      state.app = app;
      let unmount: () => void = () => {};
      await act(async () => {
        ({ unmount } = render(<LatencyOpsWorkspace />));
      });
      expect(streamNotifications).toHaveBeenCalledTimes(1);
      await act(async () => {
        emit();
        vi.advanceTimersByTime(200);
        unmount();
      });
      // No late debounced refetch and no further interval ticks after unmount.
      await act(async () => {
        vi.advanceTimersByTime(20_000);
      });
      expect(listLatencyMetrics).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  // --- p99 traffic-light row tint ------------------------------------------

  describe("latencyTone (p99 → green/amber/red)", () => {
    const MS = 1_000_000; // ns per ms

    it("is green below 1 ms", () => {
      expect(latencyTone(820)).toBe("green"); // 0.82 µs pinned core
      expect(latencyTone(0.5 * MS)).toBe("green");
      expect(latencyTone(1 * MS - 1)).toBe("green"); // just under 1 ms
    });

    it("is amber on the inclusive 1–2 ms band, including both boundaries", () => {
      expect(latencyTone(1 * MS)).toBe("amber"); // exactly 1 ms → amber, not green
      expect(latencyTone(1.3 * MS)).toBe("amber"); // ~1.3 ms quote_accept
      expect(latencyTone(2 * MS)).toBe("amber"); // exactly 2 ms → amber, not red
    });

    it("is red above 2 ms", () => {
      expect(latencyTone(2 * MS + 1)).toBe("red"); // just over 2 ms
      expect(latencyTone(2.96 * MS)).toBe("red");
      expect(latencyTone(6_100_000)).toBe("red"); // 6.1 ms book round-trip
    });

    it("returns undefined for an unsampled p99 (never a false green)", () => {
      expect(latencyTone(0)).toBeUndefined();
      expect(latencyTone(-5)).toBeUndefined();
    });
  });

  it("tints each stage row by its p99 band (data-latency-tone) and renders the legend", async () => {
    // p99: price 2.4µs → green, surface 88µs → green, book 6.1ms → red.
    const AMBER = stage({
      op: "quote_accept",
      stageLabel: "Quote accept",
      p99Ns: 1_300_000, // 1.3 ms → amber
    });
    const { app } = makeApp(metrics([stage(), AMBER, BOOK]));
    await renderWorkspace(app);

    const green = screen.getByRole("row", { name: /Price \(pinned core\)/ });
    const amber = screen.getByRole("row", { name: /Quote accept/ });
    const red = screen.getByRole("row", { name: /Ack→fill→book/ });
    expect(green).toHaveAttribute("data-latency-tone", "green");
    expect(amber).toHaveAttribute("data-latency-tone", "amber");
    expect(red).toHaveAttribute("data-latency-tone", "red");

    // The legend states the coding in real (screen-reader-legible) text.
    expect(screen.getByText(/Row tint/)).toBeInTheDocument();
    expect(screen.getByText(/<1/)).toBeInTheDocument();
    expect(screen.getByText(/1–2/)).toBeInTheDocument();
    expect(screen.getByText(/>2/)).toBeInTheDocument();
  });

  it("shows an empty state before sign-in", async () => {
    const { app } = makeApp(metrics([stage()]));
    // Signed-out: no user.
    (app.auth as { user: unknown }).user = null;
    await renderWorkspace(app);
    expect(screen.getByText(/Sign in to view latency/)).toBeInTheDocument();
  });
});
