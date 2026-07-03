/**
 * MarkCurve codec parity (Excel add-in) — the fixed-income analogue of the
 * MarkSurface mark and the write-twin of GetCurve. The Excel client speaks the SAME
 * type-tagged snake_case wire as the GUI and the server's descriptor-driven
 * `generated_codec`: the `mark_curve` request body and the `mark_curve_response`
 * frame must encode / decode field-for-field identically across every client
 * (CLAUDE.md #9 — one clean current contract). The request reuses the shared
 * `CurveSet` encoder (`ratesCurveSetToWire`) verbatim — identical to GetCurve, so a
 * mark and a live read bootstrap byte-identically. This asserts the codec byte-shape,
 * the input shaping, the spill layout, the connection round-trip, AND the
 * MarkCurve → GetCurve(pinnedVersion) reproduction contract: the version a mark
 * stamps is exactly what a later pinned read reproduces the curve from.
 */
import { describe, expect, it } from "vitest";

import type {
  GetCurveResponse,
  MarkCurveRequest,
  MarkCurveResponse,
} from "../src/contract/contract";
import {
  markCurveRequestToWire,
  markCurveResponseFromWire,
  ratesCurveSetToWire,
} from "../src/contract/wsCodec";
import type { WireObject } from "../src/contract/wsCodec";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import {
  ShapingError,
  formatMarkCurveSpill,
  shapeGetCurveRequest,
  shapeMarkCurveRequest,
} from "../src/functions/shaping";

const REQUEST: MarkCurveRequest = {
  curveSet: {
    currency: "USD",
    referenceDate: { year: 2026, month: 6, day: 25 },
    pillars: [
      { tenorYears: 2, parRate: 0.0412 },
      { tenorYears: 5, parRate: 0.0418 },
      { tenorYears: 10, parRate: 0.0424 },
    ],
  },
};

describe("Excel wsCodec — MarkCurve", () => {
  it("encodes the snake_case mark_curve body, reusing the shared CurveSet encoder", () => {
    const wire = markCurveRequestToWire(REQUEST);
    expect(wire).toEqual({
      curve_set: ratesCurveSetToWire(REQUEST.curveSet),
    });
    // The shared encoder shapes the `tenor` oneof under `{ years }` (server-decoded) —
    // byte-identical to the GetCurve request's `curve_set` (one encoding, no divergence).
    expect(wire["curve_set"]).toEqual({
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      ois_pillars: [
        { tenor: { years: 2 }, par_rate: 0.0412 },
        { tenor: { years: 5 }, par_rate: 0.0418 },
        { tenor: { years: 10 }, par_rate: 0.0424 },
      ],
    });
    // A mark carries NO tenor axis and NO pinned version — it CREATES a version.
    expect("query_tenor_years" in wire).toBe(false);
    expect("curve_version" in wire).toBe(false);
  });

  it("decodes a server-shaped mark_curve_response frame losslessly (always a version)", () => {
    const frame: WireObject = {
      currency: "USD",
      curve_version: 7,
      points: [
        { tenor_years: 2.0027, zero_rate: 0.04089, discount_factor: 0.92163 },
        { tenor_years: 5.0055, zero_rate: 0.04154, discount_factor: 0.81236 },
        { tenor_years: 10.011, zero_rate: 0.04211, discount_factor: 0.65712 },
      ],
      par_pillars: [
        { tenor_years: 2.0027, par_rate: 0.0412 },
        { tenor_years: 5.0055, par_rate: 0.0418 },
        { tenor_years: 10.011, par_rate: 0.0424 },
      ],
      epoch_nanos: 1_750_000_000_000_000_000,
    };
    const reply: MarkCurveResponse = markCurveResponseFromWire(frame);
    expect(reply).toEqual({
      currency: "USD",
      curveVersion: 7n,
      points: [
        { tenorYears: 2.0027, zeroRate: 0.04089, discountFactor: 0.92163 },
        { tenorYears: 5.0055, zeroRate: 0.04154, discountFactor: 0.81236 },
        { tenorYears: 10.011, zeroRate: 0.04211, discountFactor: 0.65712 },
      ],
      parPillars: [
        { tenorYears: 2.0027, parRate: 0.0412 },
        { tenorYears: 5.0055, parRate: 0.0418 },
        { tenorYears: 10.011, parRate: 0.0424 },
      ],
      epochNanos: 1_750_000_000_000_000_000n,
    });
  });

  it("recovers a large (>2^53) version id without precision loss", () => {
    const reply = markCurveResponseFromWire({
      currency: "USD",
      curve_version: 9_007_199_254_740_993n,
      points: [{ tenor_years: 2, zero_rate: 0.041, discount_factor: 0.921 }],
      par_pillars: [{ tenor_years: 2.0027, par_rate: 0.0412 }],
      epoch_nanos: 1_750_000_000_000_000_000,
    });
    expect(reply.curveVersion).toBe(9_007_199_254_740_993n);
  });
});

// --- CELNET.MARKCURVE input shaping (`shapeMarkCurveRequest`) -----------------
describe("mark-curve input shaping — a [tenorYears, parRate] range into a MarkCurveRequest", () => {
  it("shapes the shared RatesCurveSet (identical to GetCurve) and carries no version", () => {
    const req = shapeMarkCurveRequest({
      curve: [
        [2, 0.0412],
        [5, 0.0418],
        [10, 0.0424],
      ],
      referenceDate: "2026-06-25",
    });
    expect(req.curveSet).toEqual({
      currency: "USD",
      referenceDate: { year: 2026, month: 6, day: 25 },
      pillars: [
        { tenorYears: 2, parRate: 0.0412 },
        { tenorYears: 5, parRate: 0.0418 },
        { tenorYears: 10, parRate: 0.0424 },
      ],
    });
    // A mark's shaped curveSet is IDENTICAL to a live GetCurve of the same pillars.
    const read = shapeGetCurveRequest({
      curve: [
        [2, 0.0412],
        [5, 0.0418],
        [10, 0.0424],
      ],
      referenceDate: "2026-06-25",
    });
    expect(req.curveSet).toEqual(read.curveSet);
  });

  it("defaults the currency to USD and rejects an empty curve", () => {
    expect(shapeMarkCurveRequest({ curve: [[2, 0.0412]], referenceDate: "2026-06-25" }).curveSet.currency).toBe("USD");
    expect(() => shapeMarkCurveRequest({ curve: [["", ""]], referenceDate: "2026-06-25" })).toThrow(
      ShapingError,
    );
  });
});

// --- CELNET.MARKCURVE spill layout (`formatMarkCurveSpill`) -------------------
describe("mark-curve spill layout — a labelled 4-column matrix with a concrete version", () => {
  const REPLY: MarkCurveResponse = {
    currency: "USD",
    curveVersion: 7n,
    points: [
      { tenorYears: 2.0027, zeroRate: 0.04089, discountFactor: 0.92163 },
      { tenorYears: 5.0055, zeroRate: 0.04154, discountFactor: 0.81236 },
    ],
    parPillars: [
      { tenorYears: 2.0027, parRate: 0.0412 },
      { tenorYears: 5.0055, parRate: 0.0418 },
    ],
    epochNanos: 1_750_000_000_000_000_000n,
  };

  it("lays out the points block, the par-pillars block and a `v<n>` version footer", () => {
    const spill = formatMarkCurveSpill(REPLY);
    expect(spill).toEqual([
      ["point", "tenor_years", "discount_factor", "zero_rate"],
      ["point[0]", 2.0027, 0.92163, 0.04089],
      ["point[1]", 5.0055, 0.81236, 0.04154],
      ["par_pillar", "tenor_years", "par_rate", ""],
      ["par[0]", 2.0027, 0.0412, ""],
      ["par[1]", 5.0055, 0.0418, ""],
      ["version", "v7", "currency", "USD"],
    ]);
    // Office.js custom functions require a rectangular 2-D return.
    for (const row of spill) expect(row.length).toBe(4);
  });
});

// --- Connection round-trip + MarkCurve → GetCurve(pin) reproduction -----------
class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  readonly sent: Record<string, unknown>[] = [];

  send(data: string): void {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }
  deliver(frame: Record<string, unknown>): void {
    this.onmessage?.(JSON.stringify(frame));
  }
  sentOfType(type: string): Record<string, unknown>[] {
    return this.sent.filter((f) => f["type"] === type);
  }
}

function makeConn(): { conn: Connection; sock: FakeSocket } {
  const sock = new FakeSocket();
  let seq = 1;
  const conn = new Connection({
    url: "ws://test",
    factory: () => sock,
    stalenessWindowMs: 1000,
    clock: () => 0,
    setTimer: () => seq++,
    clearTimer: () => {},
    requestTimeoutMs: 5000,
  });
  const events: StreamEvent[] = [];
  conn.onEvent((e) => events.push(e));
  return { conn, sock };
}

describe("Connection.markCurve round-trip", () => {
  it("sends a `mark_curve` frame with the wire body and decodes the mark_curve_response reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.markCurve(REQUEST);
    const sent = sock.sentOfType("mark_curve")[0]!;
    for (const [k, v] of Object.entries(markCurveRequestToWire(REQUEST))) {
      expect(sent[k]).toEqual(v);
    }
    // Like `mark_surface`, the mark carries NO session token (the proto has only
    // `curve_set`; the server admits + stamps the version).
    expect("session_token" in sent).toBe(false);
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "mark_curve_response",
      correlation_id: corr,
      currency: "USD",
      curve_version: 42,
      points: [{ tenor_years: 2.0027, zero_rate: 0.04089, discount_factor: 0.92163 }],
      par_pillars: [{ tenor_years: 2.0027, par_rate: 0.0412 }],
      epoch_nanos: 1_750_000_000_000_000_000,
    });
    const reply = await p;
    expect(reply.currency).toBe("USD");
    expect(reply.curveVersion).toBe(42n);
    expect(reply.points).toEqual([{ tenorYears: 2.0027, zeroRate: 0.04089, discountFactor: 0.92163 }]);
  });

  it("reproduces the marked curve: the version a mark stamps is the version a later GetCurve pins", async () => {
    const { conn, sock } = makeConn();
    sock.open();

    // 1) Mark the curve — the server stamps a fresh version.
    const markP = conn.markCurve(REQUEST);
    const markSent = sock.sentOfType("mark_curve")[0]!;
    sock.deliver({
      type: "mark_curve_response",
      correlation_id: markSent["correlation_id"] as number,
      currency: "USD",
      curve_version: 99,
      points: [{ tenor_years: 2.0027, zero_rate: 0.04089, discount_factor: 0.92163 }],
      par_pillars: [{ tenor_years: 2.0027, par_rate: 0.0412 }],
      epoch_nanos: 1_750_000_000_000_000_000,
    });
    const marked = await markP;
    expect(marked.curveVersion).toBe(99n);

    // 2) Pin a GetCurve to the stamped version — the request MUST carry that exact
    //    version (a `Number` is safe here since the id fits) so the server reads the
    //    marked curve back rather than bootstrapping live.
    const pinnedVersion = Number(marked.curveVersion);
    const getP = conn.getCurve(
      shapeGetCurveRequest({
        curve: [
          [2, 0.0412],
          [5, 0.0418],
          [10, 0.0424],
        ],
        referenceDate: "2026-06-25",
        pinnedVersion,
      }),
    );
    const getSent = sock.sentOfType("get_curve")[0]!;
    expect(getSent["curve_version"]).toBe(99);
    // The pinned read echoes the marked version back (presence-tracked on read).
    sock.deliver({
      type: "get_curve_response",
      correlation_id: getSent["correlation_id"] as number,
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [{ tenor_years: 2.0027, zero_rate: 0.04089, discount_factor: 0.92163 }],
      par_pillars: [{ tenor_years: 2.0027, par_rate: 0.0412 }],
      curve_version: 99,
      epoch_nanos: 1_750_000_000_000_000_100,
    });
    const read: GetCurveResponse = await getP;
    expect(read.curveVersion).toBe(99n);
    // The pinned read reports the same first point the mark bootstrapped.
    expect(read.points[0]).toEqual(marked.points[0]);
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.markCurve(REQUEST);
    const sent = sock.sentOfType("mark_curve")[0]!;
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "error",
      correlation_id: corr,
      message: "failed_precondition: curve set has no pillars",
    });
    await expect(p).rejects.toThrow(/failed_precondition/);
  });
});
