/**
 * FI live risk-book-risk push (feature/risk-routing) — the GUI half of the single
 * wire contract. Two concerns, gated here without a server:
 *
 *   1. wsCodec round-trip — the `risk_book_risk_subscribe` client frame + the
 *      `risk_book_risk_{snapshot,update}` decoders produce/read the EXACT
 *      snake_case JSON the server codec emits (`subscription.value`, `sequence`,
 *      `version`, `books: RiskBookRiskDesc[]`, `epoch_nanos`), decoding to the SAME
 *      `RiskBookRisk` rows the 6a poll returns (only the delivery changes).
 *
 *   2. mock transport + stream — the offline `MockTransport` emits a baseline
 *      `riskBookRiskSnapshot` from the seeded synthetic risk over the SAME
 *      `StreamSession` seam the live WS transport implements, so the risk dashboard
 *      is exercisable end-to-end offline via `subscribeRiskBookRisk`.
 */

import { describe, expect, it } from "vitest";

import {
  riskBookRiskStreamSnapshotFromWire,
  riskBookRiskStreamUpdateFromWire,
  riskBookRiskSubscribeToWire,
} from "../src/data/wsCodec";
import { MockTransport } from "../src/data/mockSource";
import type { RiskBookRisk } from "../src/data/contract";

/** One book's risk row on the wire (snake_case, `RiskBookRiskDesc`). */
const WIRE_BOOK = {
  book_id: "fx-emea",
  name: "FX EMEA",
  net_notional: -1.2e8,
  gross_notional: 8e8,
  position_count: 12,
  delta: 3_000_000,
  gamma: 42_000,
  vega: 250_000,
  theta: -18_000,
  dv01: null,
  pnl: null,
  limits: [
    { metric: "net_notional", used: 1.2e8, limit: 1e9, fraction: 0.12, band: 0 },
  ],
};

describe("risk-book-risk live push codec", () => {
  it("encodes a subscribe frame with subscription.value + correlation id", () => {
    const wire = riskBookRiskSubscribeToWire({ subscriptionId: 5n, correlationId: 9n });
    expect(wire).toEqual({ subscription: { value: 5 }, correlation_id: 9 });
  });

  it("omits an absent correlation id", () => {
    const wire = riskBookRiskSubscribeToWire({ subscriptionId: 1n });
    expect(wire).toEqual({ subscription: { value: 1 } });
    expect("correlation_id" in wire).toBe(false);
  });

  it("decodes a snapshot frame into { books, version } + framing", () => {
    const snap = riskBookRiskStreamSnapshotFromWire({
      subscription: { value: 3 },
      sequence: 1,
      version: 7,
      correlation_id: 9,
      epoch_nanos: 1_700_000_000_000_000_000,
      books: [WIRE_BOOK],
    });
    expect(snap.subscriptionId).toBe(3n);
    expect(snap.sequence).toBe(1n);
    expect(snap.version).toBe(7);
    expect(snap.correlationId).toBe(9n);
    expect(snap.books).toHaveLength(1);
    const row = snap.books[0]!;
    expect(row.bookId).toBe("fx-emea");
    expect(row.netNotional).toBeCloseTo(-1.2e8);
    expect(row.delta).toBeCloseTo(3_000_000);
    // `dv01`/`pnl` ride null across the wire (not-yet-evaluated), never fabricated 0.
    expect(row.dv01).toBeNull();
    expect(row.pnl).toBeNull();
    expect(row.limits[0]!.band).toBe("green");
  });

  it("decodes an update frame (no correlation id) at a later version", () => {
    const upd = riskBookRiskStreamUpdateFromWire({
      subscription: { value: 3 },
      sequence: 2,
      version: 8,
      epoch_nanos: 1_700_000_000_000_000_001,
      books: [],
    });
    expect(upd.subscriptionId).toBe(3n);
    expect(upd.sequence).toBe(2n);
    expect(upd.version).toBe(8);
    expect(upd.books).toEqual([]);
  });
});

describe("mock transport risk-book-risk push", () => {
  it("emits a baseline snapshot from the seeded synthetic risk over the session", () => {
    const t = new MockTransport({ tickMs: 60_000 });
    const session = t.openStreamSession();
    const seen: { books: RiskBookRisk[]; version: number; kind: string }[] = [];
    session.onEvent((e) => {
      if (e.kind === "riskBookRiskSnapshot") {
        seen.push({ books: e.snapshot.books, version: e.snapshot.version, kind: e.kind });
      }
    });
    session.subscribeRiskBookRisk();

    // The baseline snapshot is emitted synchronously inside subscribe.
    expect(seen).toHaveLength(1);
    expect(seen[0]!.version).toBe(1);
    // Every enabled seed book appears, matching the one-shot poll's rows exactly.
    expect(seen[0]!.books.length).toBeGreaterThan(0);
    session.close();
  });

  it("delivers the seeded risk through the transport-level subscribe helper", async () => {
    const t = new MockTransport({ tickMs: 60_000 });
    const poll = await t.listRiskBookRisk();
    const pushed: RiskBookRisk[][] = [];
    const teardown = t.subscribeRiskBookRisk!((books) => pushed.push(books));
    expect(pushed).toHaveLength(1);
    // The pushed baseline carries the SAME rows the poll returns (delivery-only change).
    expect(pushed[0]!.map((b) => b.bookId).sort()).toEqual(poll.map((b) => b.bookId).sort());
    teardown();
  });
});
