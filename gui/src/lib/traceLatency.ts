/**
 * traceLatency — pure derivations for the Event Trace timeline. Kept out of the
 * React workspace so the inter-stage Δ arithmetic and the adaptive formatter are
 * unit-testable in isolation (no DOM, no CSS).
 *
 * A trace's events carry strictly-increasing `timestampNs` (monotonic ns from a
 * fixed epoch), so the latency of a hop is simply `later.timestampNs −
 * earlier.timestampNs`. The first event has no predecessor, so its Δ is `undefined`.
 */
import type { TraceEvent } from "../data/contract";

const NS_PER_US = 1_000;
const NS_PER_MS = 1_000_000;

/**
 * A nanosecond latency in adaptive units — ns below 1 µs, µs below 1 ms, ms above.
 * `undefined` (the first stage's absent Δ) renders "—"; a real 0 ns renders "0 ns".
 * Mirrors the Latency/Ops table's scale so a sub-µs pinned-core hop and a multi-ms
 * venue round-trip are both legible.
 */
export function fmtTraceLatency(ns: number | undefined): string {
  if (ns === undefined || ns < 0) return "—";
  if (ns < NS_PER_US) return `${Math.round(ns)} ns`;
  if (ns < NS_PER_MS) {
    const us = ns / NS_PER_US;
    return `${us.toFixed(us < 10 ? 2 : 1)} µs`;
  }
  const ms = ns / NS_PER_MS;
  return `${ms.toFixed(ms < 10 ? 2 : 1)} ms`;
}

/** The inter-stage latency of one event: the Δ from its predecessor, + max-hop flag. */
export interface InterStageLatency {
  /** ns since the previous event (`later.ts − earlier.ts`); `undefined` for the first. */
  deltaNs: number | undefined;
  /** True for the single largest Δ hop in the trace — the timeline's slowest step. */
  isMax: boolean;
}

/**
 * The per-event inter-stage latencies for a trace's ORDERED events (seq order). The
 * result is parallel to `events`: index 0 has `deltaNs === undefined` (no
 * predecessor), each later index the ns since the prior event. Exactly one entry is
 * flagged `isMax` — the largest Δ (the first, on a tie) — so the timeline can
 * emphasise the slowest hop. An empty/degenerate input yields an empty array.
 */
export function interStageLatencies(events: readonly TraceEvent[]): InterStageLatency[] {
  const deltas: (number | undefined)[] = events.map((e, i) =>
    i === 0 ? undefined : Number(e.timestampNs - events[i - 1]!.timestampNs),
  );
  let maxIdx = -1;
  let maxVal = -1;
  deltas.forEach((d, i) => {
    if (d !== undefined && d > maxVal) {
      maxVal = d;
      maxIdx = i;
    }
  });
  return deltas.map((d, i) => ({ deltaNs: d, isMax: maxIdx >= 0 && i === maxIdx }));
}
