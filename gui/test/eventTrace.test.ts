/**
 * Event Trace — the codec round-trip (TraceEvent / TraceSummary decode incl. the
 * stage int↔label map + null-absent optionals, get_trace / list_traces framing +
 * decode), the pure inter-stage-latency derivation (Δ = later.ts − earlier.ts + the
 * max-hop flag) and the offline mock store (list / filter / unknown-id). Pure (no
 * React); the workspace render lives in eventTraceWorkspace.test.tsx.
 */
import { describe, expect, it } from "vitest";

import {
  getTraceRequestToWire,
  getTraceResponseFromWire,
  listTracesRequestToWire,
  listTracesResponseFromWire,
  traceEventFromWire,
  traceStageFromWire,
  traceSummaryFromWire,
} from "../src/data/wsCodec";
import type { WireObject } from "../src/data/wsCodec";
import { fmtTraceLatency, interStageLatencies } from "../src/lib/traceLatency";
import { MockTransport } from "../src/data/mockSource";
import type { TraceEvent } from "../src/data/contract";
import { ANALYTICS_WORKSPACES, WORKSPACE_CAPABILITY } from "../src/lib/commands";

// A fully-populated QUOTE_PUBLISHED event (stage int 2), mirroring the server oracle.
const QUOTE_WIRE: WireObject = {
  trace_id: 7,
  seq: 0,
  stage: 2,
  timestamp_ns: 1_000,
  symbol: "EURUSD",
  side: "buy",
  price: 1.085,
  notional: 5_000_000,
  quote_id: "Q-1",
  deal_id: null,
  book_id: null,
  counterparty: "cp-a",
  decision: null,
  hedge_id: null,
  detail: "bid=1.0849; offer=1.0851",
  position_id: null,
};

// A DEAL_BOOKED event (stage int 7): position_id present, display deal_id absent.
const BOOKED_WIRE: WireObject = {
  trace_id: 7,
  seq: 1,
  stage: 7,
  timestamp_ns: 2_500,
  symbol: "EURUSD",
  side: "",
  price: 1.0851,
  notional: null,
  quote_id: null,
  deal_id: null,
  book_id: "BOOK-G10",
  counterparty: "cp-a",
  decision: null,
  hedge_id: null,
  detail: null,
  position_id: 42,
};

describe("traceStageFromWire", () => {
  it("maps each wire int (0–9) to its canonical stage", () => {
    expect(traceStageFromWire(0)).toBe("unspecified");
    expect(traceStageFromWire(1)).toBe("price_computed");
    expect(traceStageFromWire(2)).toBe("quote_published");
    expect(traceStageFromWire(3)).toBe("order_received");
    expect(traceStageFromWire(4)).toBe("last_look");
    expect(traceStageFromWire(5)).toBe("acceptance_decided");
    expect(traceStageFromWire(6)).toBe("risk_routed");
    expect(traceStageFromWire(7)).toBe("deal_booked");
    expect(traceStageFromWire(8)).toBe("hedge_decided");
    expect(traceStageFromWire(9)).toBe("hedge_fired");
  });

  it("decodes an out-of-range int to unspecified, never a throw", () => {
    expect(traceStageFromWire(99)).toBe("unspecified");
    expect(traceStageFromWire(-1)).toBe("unspecified");
  });
});

describe("traceEventFromWire", () => {
  it("decodes a fully-populated event: bigint ids, enum stage, present optionals", () => {
    const e = traceEventFromWire(QUOTE_WIRE);
    expect(e.traceId).toBe(7n);
    expect(e.seq).toBe(0);
    expect(e.stage).toBe("quote_published");
    expect(e.timestampNs).toBe(1_000n);
    expect(e.symbol).toBe("EURUSD");
    expect(e.side).toBe("buy");
    expect(e.price).toBe(1.085);
    expect(e.notional).toBe(5_000_000);
    expect(e.quoteId).toBe("Q-1");
    expect(e.counterparty).toBe("cp-a");
    expect(e.detail).toBe("bid=1.0849; offer=1.0851");
  });

  it("renders null-absent optionals as undefined (present ⇒ value, null ⇒ undefined)", () => {
    const e = traceEventFromWire(QUOTE_WIRE);
    // Present on the wire ⇒ the value; null on the wire ⇒ undefined.
    expect(e.dealId).toBeUndefined();
    expect(e.bookId).toBeUndefined();
    expect(e.decision).toBeUndefined();
    expect(e.hedgeId).toBeUndefined();
    expect(e.positionId).toBeUndefined();
  });

  it("recovers a u64 position_id as a bigint on the booking event", () => {
    const e = traceEventFromWire(BOOKED_WIRE);
    expect(e.stage).toBe("deal_booked");
    expect(e.positionId).toBe(42n);
    expect(e.bookId).toBe("BOOK-G10");
    expect(e.dealId).toBeUndefined();
    expect(e.notional).toBeUndefined(); // null ⇒ undefined, never 0
  });

  it("treats a missing (absent-key) optional the same as null ⇒ undefined", () => {
    const minimal: WireObject = { trace_id: 3, seq: 0, stage: 1, timestamp_ns: 10, symbol: "X", side: "" };
    const e = traceEventFromWire(minimal);
    expect(e.price).toBeUndefined();
    expect(e.quoteId).toBeUndefined();
    expect(e.positionId).toBeUndefined();
  });
});

describe("traceSummaryFromWire", () => {
  it("decodes a summary row, coercing outcome + null-absent counterparty", () => {
    const wire: WireObject = {
      trace_id: 7,
      first_stage: 1,
      last_stage: 9,
      first_timestamp_ns: 100,
      last_timestamp_ns: 900,
      total_latency_ns: 800,
      event_count: 9,
      symbol: "EURUSD",
      counterparty: "cp-a",
      outcome: "hedged",
    };
    const s = traceSummaryFromWire(wire);
    expect(s.traceId).toBe(7n);
    expect(s.firstStage).toBe("price_computed");
    expect(s.lastStage).toBe("hedge_fired");
    expect(s.totalLatencyNs).toBe(800);
    expect(s.eventCount).toBe(9);
    expect(s.outcome).toBe("hedged");
    expect(s.counterparty).toBe("cp-a");
  });

  it("renders an absent counterparty as undefined and an unknown outcome as in_flight", () => {
    const wire: WireObject = {
      trace_id: 1,
      first_stage: 3,
      last_stage: 5,
      first_timestamp_ns: 10,
      last_timestamp_ns: 20,
      total_latency_ns: 10,
      event_count: 2,
      symbol: "GBPUSD",
      counterparty: null,
      outcome: "weird",
    };
    const s = traceSummaryFromWire(wire);
    expect(s.counterparty).toBeUndefined();
    expect(s.outcome).toBe("in_flight");
  });
});

describe("get_trace / list_traces framing", () => {
  it("frames get_trace with just the trace_id (token auto-injected)", () => {
    expect(getTraceRequestToWire(42n)).toEqual({ trace_id: 42 });
  });

  it("decodes the { events: [...] } get_trace reply in order", () => {
    const evs = getTraceResponseFromWire({ events: [QUOTE_WIRE, BOOKED_WIRE] });
    expect(evs).toHaveLength(2);
    expect(evs[0]!.stage).toBe("quote_published");
    expect(evs[1]!.stage).toBe("deal_booked");
    expect(evs[1]!.positionId).toBe(42n);
  });

  it("decodes an empty / unknown-trace reply to []", () => {
    expect(getTraceResponseFromWire({ events: [] })).toEqual([]);
    expect(getTraceResponseFromWire({})).toEqual([]);
  });

  it("frames list_traces filters, omitting absent / empty ones", () => {
    expect(listTracesRequestToWire()).toEqual({});
    expect(listTracesRequestToWire({})).toEqual({});
    expect(listTracesRequestToWire({ limit: 25 })).toEqual({ limit: 25 });
    expect(listTracesRequestToWire({ symbol: "EURUSD", counterparty: "cp-a", limit: 10 })).toEqual({
      symbol: "EURUSD",
      counterparty: "cp-a",
      limit: 10,
    });
    // An empty-string filter is omitted (not sent as an exact-match on "").
    expect(listTracesRequestToWire({ symbol: "", counterparty: "" })).toEqual({});
  });

  it("decodes the { traces: [...] } list_traces reply", () => {
    const rows = listTracesResponseFromWire({
      traces: [
        {
          trace_id: 7,
          first_stage: 1,
          last_stage: 9,
          first_timestamp_ns: 100,
          last_timestamp_ns: 900,
          total_latency_ns: 800,
          event_count: 9,
          symbol: "EURUSD",
          counterparty: "cp-a",
          outcome: "hedged",
        },
      ],
    });
    expect(rows).toHaveLength(1);
    expect(rows[0]!.traceId).toBe(7n);
    expect(rows[0]!.outcome).toBe("hedged");
  });
});

// --- interStageLatencies (pure Δ derivation) --------------------------------

function ev(seq: number, tsNs: bigint): TraceEvent {
  return { traceId: 1n, seq, stage: "price_computed", timestampNs: tsNs, symbol: "X", side: "" };
}

describe("interStageLatencies", () => {
  it("computes Δ = later.ts − earlier.ts, first stage undefined", () => {
    const out = interStageLatencies([ev(0, 100n), ev(1, 2_500n), ev(2, 2_540n)]);
    expect(out[0]!.deltaNs).toBeUndefined();
    expect(out[1]!.deltaNs).toBe(2_400);
    expect(out[2]!.deltaNs).toBe(40);
  });

  it("flags the single largest Δ hop as the max", () => {
    const out = interStageLatencies([ev(0, 0n), ev(1, 10n), ev(2, 1_010n), ev(3, 1_020n)]);
    const maxIdx = out.findIndex((l) => l.isMax);
    expect(maxIdx).toBe(2); // the 1_000 ns hop dominates
    expect(out.filter((l) => l.isMax)).toHaveLength(1);
    expect(out[0]!.isMax).toBe(false); // the first (no Δ) is never the max
  });

  it("returns [] for an empty trace", () => {
    expect(interStageLatencies([])).toEqual([]);
  });
});

describe("fmtTraceLatency", () => {
  it("renders adaptive ns/µs/ms and — for an absent Δ", () => {
    expect(fmtTraceLatency(undefined)).toBe("—");
    expect(fmtTraceLatency(820)).toBe("820 ns");
    expect(fmtTraceLatency(2_400)).toBe("2.40 µs");
    expect(fmtTraceLatency(2_600_000)).toBe("2.60 ms");
  });
});

// --- offline mock store -----------------------------------------------------

describe("MockTransport event traces", () => {
  it("lists the seeded fixtures (booked+hedged, rejected), newest first", async () => {
    const t = new MockTransport();
    const rows = await t.listTraces();
    expect(rows.length).toBeGreaterThanOrEqual(2);
    const outcomes = new Set(rows.map((r) => r.outcome));
    expect(outcomes.has("hedged")).toBe(true);
    expect(outcomes.has("rejected")).toBe(true);
  });

  it("getTrace returns the full 9-stage booked+hedged fixture in seq order", async () => {
    const t = new MockTransport();
    const evs = await t.getTrace(7n);
    expect(evs).toHaveLength(9);
    expect(evs.map((e) => e.seq)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8]);
    expect(evs[0]!.stage).toBe("price_computed");
    expect(evs[8]!.stage).toBe("hedge_fired");
    // timestamps strictly increasing ⇒ every Δ is positive.
    for (let i = 1; i < evs.length; i++) {
      expect(evs[i]!.timestampNs > evs[i - 1]!.timestampNs).toBe(true);
    }
  });

  it("getTrace on an unknown / evicted id resolves to []", async () => {
    const t = new MockTransport();
    expect(await t.getTrace(999_999n)).toEqual([]);
  });

  it("filters the list by exact-match symbol and counterparty", async () => {
    const t = new MockTransport();
    const bySymbol = await t.listTraces({ symbol: "EURUSD" });
    expect(bySymbol.length).toBeGreaterThanOrEqual(1);
    expect(bySymbol.every((r) => r.symbol === "EURUSD")).toBe(true);

    const byCp = await t.listTraces({ counterparty: "cp-gamma" });
    expect(byCp.length).toBeGreaterThanOrEqual(1);
    expect(byCp.every((r) => r.counterparty === "cp-gamma")).toBe(true);
    expect(byCp.every((r) => r.outcome === "rejected")).toBe(true);

    const none = await t.listTraces({ symbol: "NOPE" });
    expect(none).toEqual([]);
  });
});

describe("Event Trace nav gating", () => {
  it("belongs to the analytics domain and gates on view_analytics", () => {
    expect(ANALYTICS_WORKSPACES.has("eventtrace")).toBe(true);
    expect(WORKSPACE_CAPABILITY["eventtrace"]).toBe("view_analytics");
  });
});
