/**
 * Decision Audit — the sort/filter contract, the honesty rules, and the wire codec.
 *
 * These are the behaviours the surface's credibility rests on: a total sort so rows do
 * not reshuffle, a filter that narrows to exactly the cited evidence, a walked path that
 * flags a node the graph no longer has rather than relabelling it, a series built only
 * from recorded rows, a caveat whenever the window has rolled, and a codec that THROWS
 * on an unknown enum ordinal rather than mis-decoding an audit row.
 */
import { describe, expect, it } from "vitest";

import type { AcceptanceGraph, DecisionRecord, HedgeGraph } from "../src/data/contract";
import {
  decisionJournalResponseFromWire,
  decisionRecordFromWire,
  listDecisionJournalRequestToWire,
  ruleAdviceResponseFromWire,
} from "../src/data/wsCodec";
import {
  filterRows,
  resolveWalkedPath,
  sortRows,
  utilizationSeries,
  windowCaveat,
} from "../src/workspaces/audit/decisionAudit";

function row(over: Partial<DecisionRecord> = {}): DecisionRecord {
  return {
    seq: 1,
    decidedAtNanos: 1_000_000_000,
    engine: "hedge",
    outcome: "no_action",
    outcomeLabel: "WAREHOUSE",
    reason: "red · WAREHOUSE — the policy graph resolved a warehouse hold at node 1",
    policyPath: [0, 1],
    scope: "book:rates-usd",
    book: "rates-usd",
    instrument: "OIS",
    desk: "RATES",
    metric: "dv01",
    netRisk: -97_000,
    threshold: 100_000,
    utilization: 0.97,
    band: "red",
    notional: 0,
    advisory: false,
    ...over,
  };
}

describe("audit sorting", () => {
  it("is total — equal keys break on seq so rows never reshuffle", () => {
    const rows = [
      row({ seq: 3, band: "red" }),
      row({ seq: 1, band: "red" }),
      row({ seq: 2, band: "red" }),
    ];
    const once = sortRows(rows, { key: "band", dir: "asc" }).map((r) => r.seq);
    const twice = sortRows(sortRows(rows, { key: "band", dir: "asc" }), {
      key: "band",
      dir: "asc",
    }).map((r) => r.seq);
    expect(once).toEqual([1, 2, 3]);
    expect(twice).toEqual(once);
  });

  it("orders numerics numerically, not lexically", () => {
    const rows = [row({ seq: 9, utilization: 0.9 }), row({ seq: 10, utilization: 0.1 })];
    expect(sortRows(rows, { key: "utilization", dir: "desc" }).map((r) => r.seq)).toEqual([9, 10]);
    expect(sortRows(rows, { key: "seq", dir: "desc" }).map((r) => r.seq)).toEqual([10, 9]);
  });
});

describe("audit filtering", () => {
  it("matches the reason, the scope and the counterparty case-insensitively", () => {
    const rows = [
      row({ seq: 1, reason: "halted: kill-switch / desk disabled" }),
      row({ seq: 2, counterparty: "cp-bank" }),
      row({ seq: 3, scope: "bucket:emea" }),
    ];
    expect(filterRows(rows, { query: "KILL-SWITCH" }).map((r) => r.seq)).toEqual([1]);
    expect(filterRows(rows, { query: "cp-b" }).map((r) => r.seq)).toEqual([2]);
    expect(filterRows(rows, { query: "emea" }).map((r) => r.seq)).toEqual([3]);
    expect(filterRows(rows, { query: "   " })).toHaveLength(3);
  });

  it("narrows to exactly the cited evidence rows", () => {
    const rows = [row({ seq: 1 }), row({ seq: 2 }), row({ seq: 3 })];
    expect(filterRows(rows, { query: "", seqs: [1, 3] }).map((r) => r.seq)).toEqual([1, 3]);
    expect(filterRows(rows, { query: "", seqs: [] })).toHaveLength(0);
  });
});

describe("walked path resolution", () => {
  const hedgeGraph: HedgeGraph = {
    entry: 0,
    nodes: [
      {
        kind: "condition",
        id: 0,
        condition: {
          field: "breached",
          op: "eq",
          value: { kind: "text", text: "false" },
          onTrue: 1,
          onFalse: 2,
        },
      },
      { kind: "action", id: 1, action: { kind: "warehouse" } },
    ],
  };

  it("labels every node the current graph still contains, marking the leaf", () => {
    const steps = resolveWalkedPath([0, 1], hedgeGraph, "hedge");
    expect(steps).toHaveLength(2);
    expect(steps[0]?.stale).toBe(false);
    expect(steps[0]?.label).toContain("breached");
    expect(steps[1]?.leaf).toBe(true);
    expect(steps[1]?.label).toContain("warehouse");
  });

  /** The honesty rule: a node the graph no longer has is FLAGGED, never relabelled. */
  it("flags a node the current graph no longer contains rather than inventing a label", () => {
    const steps = resolveWalkedPath([0, 7], hedgeGraph, "hedge");
    expect(steps[1]).toMatchObject({ id: 7, label: null, stale: true, leaf: true });
  });

  it("resolves nothing (but drops nothing) when no graph is loaded", () => {
    const steps = resolveWalkedPath([0, 1], null, "hedge");
    expect(steps.map((s) => s.id)).toEqual([0, 1]);
    expect(steps.every((s) => s.stale)).toBe(true);
  });

  it("uses the acceptance vocabulary for an acceptance row", () => {
    const graph: AcceptanceGraph = {
      entry: 0,
      nodes: [{ kind: "decision", id: 0, action: { kind: "accept" } }],
    };
    expect(resolveWalkedPath([0], graph, "acceptance")[0]?.label).toContain("Accept");
  });
});

describe("utilisation series", () => {
  it("is built only from recorded hedge rows of the same cell, oldest first", () => {
    const rows = [
      row({ seq: 3, utilization: 0.9 }),
      row({ seq: 1, utilization: 0.4 }),
      row({ seq: 2, utilization: 0.7 }),
      row({ seq: 4, book: "other", utilization: 0.99 }),
      row({ seq: 5, engine: "acceptance", utilization: 0 }),
    ];
    const series = utilizationSeries(rows, "rates-usd", "OIS");
    expect(series.map((p) => p.seq)).toEqual([1, 2, 3]);
    expect(series.map((p) => p.utilization)).toEqual([0.4, 0.7, 0.9]);
  });

  it("marks which points actually acted", () => {
    const rows = [row({ seq: 1 }), row({ seq: 2, outcome: "fired", outcomeLabel: "RFQ_OUT" })];
    expect(utilizationSeries(rows, "rates-usd", "OIS").map((p) => p.acted)).toEqual([false, true]);
  });
});

describe("window honesty", () => {
  it("stays silent when nothing has been evicted", () => {
    expect(windowCaveat(120, 0)).toBeNull();
  });

  it("names the loss when the bounded ring has rolled", () => {
    const caveat = windowCaveat(20_000, 3_616);
    expect(caveat).toContain("INCOMPLETE");
    expect(caveat).toContain("3,616");
    expect(caveat).toContain("20,000");
  });
});

describe("wire codec", () => {
  const wire = {
    seq: 42,
    decided_at: 1_700_000_000_000_000_000,
    engine: 3,
    outcome: 2,
    outcome_label: "WAREHOUSE",
    reason: "red · WAREHOUSE",
    policy_path: [0, 1],
    scope: "book:rates-usd",
    book: "rates-usd",
    instrument: "OIS",
    desk: "RATES",
    metric: 0,
    net_risk: -97_000,
    threshold: 100_000,
    utilization: 0.97,
    band: "red",
    notional: 0,
    advisory: false,
  };

  it("decodes a full row and leaves absent optionals undefined", () => {
    const r = decisionRecordFromWire(wire);
    expect(r.seq).toBe(42);
    expect(r.engine).toBe("hedge");
    expect(r.outcome).toBe("no_action");
    expect(r.policyPath).toEqual([0, 1]);
    expect(r.counterparty).toBeUndefined();
    expect(r.hedgeId).toBeUndefined();
    expect(r.traceId).toBeUndefined();
  });

  it("carries the presence-tracked join keys when the server sends them", () => {
    const r = decisionRecordFromWire({
      ...wire,
      counterparty: "cp-bank",
      hedge_id: "HDG-7",
      trace_id: 4242,
      position_id: 7,
      request_id: "RFQ-1",
      symbol: "US10Y",
    });
    expect(r.counterparty).toBe("cp-bank");
    expect(r.hedgeId).toBe("HDG-7");
    expect(r.traceId).toBe(4242);
    expect(r.positionId).toBe(7);
    expect(r.requestId).toBe("RFQ-1");
    expect(r.symbol).toBe("US10Y");
  });

  /** A mis-decoded audit row is worse than a visible failure. */
  it("throws on an unknown engine or outcome ordinal instead of guessing", () => {
    expect(() => decisionRecordFromWire({ ...wire, engine: 0 })).toThrow(/unknown decision engine/);
    expect(() => decisionRecordFromWire({ ...wire, engine: 99 })).toThrow(/unknown decision engine/);
    expect(() => decisionRecordFromWire({ ...wire, outcome: 0 })).toThrow(/unknown decision outcome/);
  });

  it("carries the eviction accounting off the envelope", () => {
    const page = decisionJournalResponseFromWire({
      records: [wire],
      total_recorded: 900,
      evicted: 12,
    });
    expect(page.records).toHaveLength(1);
    expect(page.totalRecorded).toBe(900);
    expect(page.evicted).toBe(12);
  });

  it("omits absent request filters rather than sending nulls", () => {
    expect(listDecisionJournalRequestToWire()).toEqual({});
    expect(listDecisionJournalRequestToWire({ engine: "acceptance", outcome: "no_action" })).toEqual(
      { engine: 1, outcome: 2 },
    );
    expect(listDecisionJournalRequestToWire({ book: "rates-usd", limit: 50 })).toEqual({
      book: "rates-usd",
      limit: 50,
    });
  });

  it("decodes advice with its evidence citations and honest denominator", () => {
    const page = ruleAdviceResponseFromWire({
      advice: [
        {
          advice_id: "hedge_policy_missing:rates-usd:OIS:",
          kind: "hedge_policy_missing",
          engine: 3,
          title: "rates-usd · OIS has no hedge policy",
          rationale: "7 risk evaluations …",
          recommended_action: "Author a hedge rule …",
          scope_book: "rates-usd",
          scope_instrument: "OIS",
          occurrences: 7,
          first_seen: 100,
          last_seen: 700,
          evidence_seqs: [1, 2, 3],
          editor: "hedging",
        },
      ],
      rows_considered: 41,
    });
    expect(page.rowsConsidered).toBe(41);
    expect(page.advice[0]?.evidenceSeqs).toEqual([1, 2, 3]);
    expect(page.advice[0]?.scopeCounterparty).toBeUndefined();
    expect(page.advice[0]?.engine).toBe("hedge");
  });
});
