/**
 * EventTraceWorkspace — render + interaction. Drives the two-pane view with `useApp`
 * mocked (no server): the recent-traces list renders, selecting a row loads its
 * timeline (Δ latencies + the slowest-hop emphasis), a symbol filter refetches, and
 * empty / unknown-trace states render gracefully.
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { TraceEvent, TraceSummary } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { EventTraceWorkspace } from "../src/workspaces/analytics/EventTraceWorkspace";

function summary(overrides: Partial<TraceSummary> = {}): TraceSummary {
  return {
    traceId: 7n,
    firstStage: "price_computed",
    lastStage: "hedge_fired",
    firstTimestampNs: 100n,
    lastTimestampNs: 900n,
    totalLatencyNs: 800,
    eventCount: 9,
    symbol: "EURUSD",
    counterparty: "cp-alpha",
    outcome: "hedged",
    ...overrides,
  };
}

const REJECTED = summary({
  traceId: 8n,
  lastStage: "acceptance_decided",
  symbol: "GBPUSD",
  counterparty: "cp-gamma",
  outcome: "rejected",
  eventCount: 5,
  totalLatencyNs: 540_120_000,
});

function ev(seq: number, stage: TraceEvent["stage"], tsNs: bigint, extra: Partial<TraceEvent> = {}): TraceEvent {
  return { traceId: 7n, seq, stage, timestampNs: tsNs, symbol: "EURUSD", side: "buy", ...extra };
}

const TRACE_7: TraceEvent[] = [
  ev(0, "price_computed", 0n, { price: 1.08495, counterparty: "cp-alpha" }),
  ev(1, "quote_published", 2_400n, { quoteId: "Q-7001" }),
  ev(2, "order_received", 812_000_000n, { quoteId: "Q-7001" }),
  ev(3, "deal_booked", 816_400_000n, { dealId: "deal-7", positionId: 4242n }),
];

function makeApp(summaries: TraceSummary[], eventsById: Map<bigint, TraceEvent[]>) {
  const listTraces = vi.fn(async (filter?: { symbol?: string; counterparty?: string }) => {
    let rows = summaries;
    if (filter?.symbol) rows = rows.filter((r) => r.symbol === filter.symbol);
    if (filter?.counterparty) rows = rows.filter((r) => r.counterparty === filter.counterparty);
    return rows;
  });
  const getTrace = vi.fn(async (id: bigint) => eventsById.get(id) ?? []);
  let onNotification: (() => void) | undefined;
  const streamNotifications = vi.fn((_scope: unknown, cb: () => void) => {
    onNotification = cb;
    return () => {
      onNotification = undefined;
    };
  });
  return {
    app: {
      transport: { listTraces, getTrace, streamNotifications },
      auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true },
      traceFocus: null,
      openTrace: vi.fn(),
      clearTraceFocus: vi.fn(),
    },
    listTraces,
    getTrace,
    emit: (): void => onNotification?.(),
  };
}

async function renderWorkspace(app: unknown): Promise<void> {
  state.app = app;
  await act(async () => {
    render(<EventTraceWorkspace />);
  });
}

afterEach(() => {
  cleanup();
  state.app = null;
});

describe("EventTraceWorkspace", () => {
  it("searches loaded traces WITHOUT a round trip, reaching fields the server filters cannot", async () => {
    // The symbol / counterparty boxes above are SERVER filters that re-query. This one
    // narrows what is already on screen and reaches the trace id, the outcome and the
    // last stage — the trace id in particular being what you have when someone sends
    // you one.
    const { app } = makeApp([summary(), REJECTED], new Map());
    await renderWorkspace(app);
    expect(screen.getAllByTestId("trace-row")).toHaveLength(2);

    const before = app as { transport: { listTraces: { mock: { calls: unknown[] } } } };
    const callsBefore = before.transport.listTraces.mock.calls.length;

    fireEvent.change(screen.getByLabelText("Search traces"), { target: { value: "hedge_fired" } });
    expect(screen.getAllByTestId("trace-row")).toHaveLength(1);
    // No refetch: narrowing the loaded page must not go back to the server.
    expect(before.transport.listTraces.mock.calls.length).toBe(callsBefore);

    // …and by the trace id itself.
    fireEvent.change(screen.getByLabelText("Search traces"), { target: { value: "7" } });
    expect(screen.getAllByTestId("trace-row")).toHaveLength(1);
  });

  it("offers only outcomes that are actually present, and filters to one", async () => {
    const { app } = makeApp([summary(), REJECTED], new Map());
    await renderWorkspace(app);

    const select = screen.getByTestId("event-trace-outcome-filter");
    // A dead option is worse than no filter — the vocabulary comes from the rows.
    const values = within(select)
      .getAllByRole("option")
      .map((o) => (o as HTMLOptionElement).value);
    expect(values).toContain("hedged");
    expect(values).toContain("rejected");

    fireEvent.change(select, { target: { value: "rejected" } });
    const rows = screen.getAllByTestId("trace-row");
    expect(rows).toHaveLength(1);
    expect(rows[0]?.textContent).toContain("GBPUSD");
  });

  it("renders the recent-traces list with symbol, counterparty, outcome + total latency", async () => {
    const { app } = makeApp([summary(), REJECTED], new Map());
    await renderWorkspace(app);
    expect(await screen.findByTestId("event-trace-list")).toBeTruthy();
    const rows = screen.getAllByTestId("trace-row");
    expect(rows).toHaveLength(2);
    const hedged = screen.getByRole("row", { name: /EURUSD/ });
    expect(within(hedged).getByText("Hedged")).toBeTruthy();
    expect(within(hedged).getByText("cp-alpha")).toBeTruthy();
    expect(within(hedged).getByText("800 ns")).toBeTruthy();
    expect(screen.getByRole("row", { name: /GBPUSD/ })).toBeTruthy();
    expect(screen.getByText("Rejected")).toBeTruthy();
  });

  it("loads the timeline for a selected trace with per-hop Δ latency + slowest-hop mark", async () => {
    const events = new Map<bigint, TraceEvent[]>([[7n, TRACE_7]]);
    const { app, getTrace } = makeApp([summary(), REJECTED], events);
    await renderWorkspace(app);
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "#7" }));
    });
    await waitFor(() => expect(getTrace).toHaveBeenCalledWith(7n));
    const timeline = await screen.findByTestId("trace-timeline");
    // All four stages render, in order.
    expect(within(timeline).getByText("Price computed")).toBeTruthy();
    expect(within(timeline).getByText("Deal booked")).toBeTruthy();
    // The biggest hop (order_received Δ ≈ 812 ms) is emphasised as the slowest.
    expect(within(timeline).getByText(/slowest hop/i)).toBeTruthy();
    expect(timeline.textContent).toContain("812.0 ms");
    // The first stage shows "start", not a Δ.
    expect(timeline.textContent).toContain("start");
  });

  it("refetches when the symbol filter changes (debounced)", async () => {
    const { app, listTraces } = makeApp([summary(), REJECTED], new Map());
    await renderWorkspace(app);
    const input = screen.getByPlaceholderText("e.g. EURUSD");
    await act(async () => {
      fireEvent.change(input, { target: { value: "EURUSD" } });
    });
    await waitFor(() =>
      expect(listTraces).toHaveBeenCalledWith(expect.objectContaining({ symbol: "EURUSD" })),
    );
    // The filtered list drops the GBPUSD row.
    await waitFor(() => expect(screen.getAllByTestId("trace-row")).toHaveLength(1));
  });

  it("shows an empty state when no traces match", async () => {
    const { app } = makeApp([], new Map());
    await renderWorkspace(app);
    expect(await screen.findByText("No traces match.")).toBeTruthy();
  });

  it("handles an unknown / evicted trace (getTrace [] ) gracefully", async () => {
    const { app } = makeApp([summary()], new Map()); // getTrace(7n) ⇒ []
    await renderWorkspace(app);
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "#7" }));
    });
    expect(await screen.findByText(/no events/i)).toBeTruthy();
  });

  it("prompts to sign in when signed out", async () => {
    const { app } = makeApp([summary()], new Map());
    (app.auth as { user: unknown }).user = null;
    await renderWorkspace(app);
    expect(screen.getByText(/sign in to view event traces/i)).toBeTruthy();
  });
});
