/**
 * StatusRibbon — the trading-latency readout in the bottom ribbon. The pure stage
 * selectors + adaptive formatter, then the rendered ribbon driven by `useApp`
 * mocked (no server): the headline **avg quote** + **order p99** show real values
 * from a stubbed `listLatencyMetrics` (incl. the mock fixture — parity), degrade to
 * "—" on absent/denied, and are gated on `view_analytics` (hidden — and NOT polled —
 * for a caller without it). The transport (mock/replay) and client render-p99 segments
 * were removed from the ribbon; the build stamp (version + date) stays for everyone.
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";

import type { LatencyMetrics, LatencyStage } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { MockTransport } from "../src/data/mockSource";
import {
  StatusRibbon,
  selectAvgQuoteNs,
  selectOrderP99Ns,
} from "../src/app/StatusRibbon";
import { fmtLatencyShort } from "../src/lib/format";

// --- fixtures ---------------------------------------------------------------

function stage(op: string, overrides: Partial<LatencyStage> = {}): LatencyStage {
  return {
    op,
    stageLabel: op,
    count: 1_000,
    p50Ns: 1_000,
    p99Ns: 4_000,
    p999Ns: 8_000,
    p9999Ns: 12_000,
    minNs: 500,
    maxNs: 16_000,
    meanNs: 1_500,
    ...overrides,
  };
}

const HEALTH: LatencyMetrics["health"] = {
  drainedTotal: 10,
  droppedTotal: 0,
  observedGaps: 0,
  tickHz: 24_000_000,
};

function metrics(stages: LatencyStage[]): LatencyMetrics {
  return { stages, health: { ...HEALTH } };
}

/** A mock app whose `listLatencyMetrics` resolves `result` (or rejects if it is an Error). */
function makeApp(opts: { can?: boolean; result?: LatencyMetrics | Error } = {}) {
  const can = opts.can ?? true;
  const listLatencyMetrics = vi.fn(async (): Promise<LatencyMetrics> => {
    if (opts.result instanceof Error) throw opts.result;
    return opts.result ?? metrics([]);
  });
  return {
    app: {
      transport: { label: "mock/replay", listLatencyMetrics },
      auth: { user: null, can: () => can },
      stream: {
        rows: [{ health: "HEALTHY", gaps: 0 }],
        totalSeq: 0n,
        lpCount: 3,
        observability: { received: false, serverPriceP99Nanos: 0n, conflationDrops: 0n },
      },
    },
    listLatencyMetrics,
  };
}

async function renderRibbon(app: unknown): Promise<void> {
  state.app = app;
  await act(async () => {
    render(<StatusRibbon />);
  });
}

afterEach(() => {
  cleanup();
  state.app = null;
});

// --- pure selectors ---------------------------------------------------------

describe("StatusRibbon latency selectors", () => {
  it("avg quote prefers the ESP tick→quote publish stage (mean)", () => {
    const m = metrics([stage("stream_publish", { meanNs: 1_980 }), stage("rfq_respond", { meanNs: 286_400 })]);
    expect(selectAvgQuoteNs(m)).toBe(1_980);
  });

  it("avg quote falls back to RFQ respond when the publish stage is absent or empty", () => {
    const absent = metrics([stage("rfq_respond", { meanNs: 286_400 })]);
    expect(selectAvgQuoteNs(absent)).toBe(286_400);
    const empty = metrics([stage("stream_publish", { count: 0, meanNs: 1_980 }), stage("rfq_respond", { meanNs: 286_400 })]);
    expect(selectAvgQuoteNs(empty)).toBe(286_400);
  });

  it("order p99 prefers the ack→fill→book stage (p99), falling back to quote accept", () => {
    const m = metrics([stage("book", { p99Ns: 6_100_000 }), stage("quote_accept", { p99Ns: 3_400_000 })]);
    expect(selectOrderP99Ns(m)).toBe(6_100_000);
    const fallback = metrics([stage("quote_accept", { p99Ns: 3_400_000 })]);
    expect(selectOrderP99Ns(fallback)).toBe(3_400_000);
  });

  it("returns null for no metrics or no matching stage (rendered '—', never a fabricated 0)", () => {
    expect(selectAvgQuoteNs(null)).toBeNull();
    expect(selectOrderP99Ns(null)).toBeNull();
    expect(selectAvgQuoteNs(metrics([stage("book")]))).toBeNull();
  });
});

describe("fmtLatencyShort", () => {
  it("renders sub-millisecond in µs and a millisecond or more in ms", () => {
    expect(fmtLatencyShort(1_980)).toBe("2.0µs");
    expect(fmtLatencyShort(6_100_000)).toBe("6.10ms");
  });
  it("renders a gap for non-finite / negative input", () => {
    expect(fmtLatencyShort(Number.NaN)).toBe("—");
    expect(fmtLatencyShort(-1)).toBe("—");
  });
});

// --- rendered ribbon --------------------------------------------------------

describe("StatusRibbon trading-latency readout", () => {
  it("renders avg-quote (µs) + order-p99 (ms) from a stubbed listLatencyMetrics", async () => {
    const { app } = makeApp({
      result: metrics([
        stage("stream_publish", { meanNs: 1_980 }),
        stage("book", { p99Ns: 6_100_000 }),
      ]),
    });
    await renderRibbon(app);
    const avg = await screen.findByTestId("avg-quote");
    await waitFor(() => expect(avg).toHaveTextContent("avg quote 2.0µs"));
    expect(screen.getByTestId("order-p99")).toHaveTextContent("order p99 6.10ms");
  });

  it("shows realistic values from the offline MOCK fixture (parity)", async () => {
    const mock = new MockTransport();
    const app = {
      transport: { label: "mock/replay", listLatencyMetrics: () => mock.listLatencyMetrics() },
      auth: { user: null, can: () => true },
      stream: {
        rows: [{ health: "HEALTHY", gaps: 0 }],
        totalSeq: 0n,
        lpCount: 3,
        observability: { received: false, serverPriceP99Nanos: 0n, conflationDrops: 0n },
      },
    };
    await renderRibbon(app);
    // stream_publish mean 1_980.9ns → 2.0µs ; book p99 6_100_000ns → 6.10ms
    await waitFor(() => expect(screen.getByTestId("avg-quote")).toHaveTextContent("avg quote 2.0µs"));
    expect(screen.getByTestId("order-p99")).toHaveTextContent("order p99 6.10ms");
  });

  it("degrades to '—' when the metrics carry no matching stage", async () => {
    const { app } = makeApp({ result: metrics([stage("surface_vol")]) });
    await renderRibbon(app);
    const avg = await screen.findByTestId("avg-quote");
    await waitFor(() => expect(avg).toHaveTextContent("avg quote —"));
    expect(screen.getByTestId("order-p99")).toHaveTextContent("order p99 —");
  });

  it("swallows an RPC denial / disconnect to '—' (never throws)", async () => {
    const { app } = makeApp({ result: new Error("denied: view_analytics required") });
    await renderRibbon(app);
    const avg = await screen.findByTestId("avg-quote");
    await waitFor(() => expect(avg).toHaveTextContent("avg quote —"));
    expect(screen.getByTestId("order-p99")).toHaveTextContent("order p99 —");
  });

  it("hides the readout AND does not poll for a caller without view_analytics", async () => {
    const { app, listLatencyMetrics } = makeApp({ can: false });
    await renderRibbon(app);
    expect(screen.queryByTestId("avg-quote")).toBeNull();
    expect(screen.queryByTestId("order-p99")).toBeNull();
    expect(listLatencyMetrics).not.toHaveBeenCalled();
  });

  it("removes the render-p99 and transport (mock/replay) segments, keeps version + date", async () => {
    const { app } = makeApp({ result: metrics([stage("stream_publish"), stage("book")]) });
    await renderRibbon(app);
    const footer = screen.getByRole("contentinfo");
    // the two removed segments are gone
    expect(within(footer).queryByText(/render p99/)).toBeNull();
    expect(within(footer).queryByText(/mock\/replay/)).toBeNull();
    // the build stamp (version + date) remains
    expect(within(footer).getByText(/celnet test/)).toBeInTheDocument();
    expect(within(footer).getByText(/1970-01-01/)).toBeInTheDocument();
    // the headline trading latencies remain
    expect(within(footer).getByTestId("avg-quote")).toBeInTheDocument();
    expect(within(footer).getByTestId("order-p99")).toBeInTheDocument();
  });
});
