/**
 * GetCurve codec parity (Excel add-in). The Excel client speaks the SAME
 * type-tagged snake_case wire as the GUI and the server's descriptor-driven
 * `generated_codec`: the `get_curve` request body and the `get_curve_response`
 * frame must encode / decode field-for-field identically across every client
 * (CLAUDE.md #9 — one clean current contract). The request reuses the shared
 * `CurveSet` encoder (`ratesCurveSetToWire`) verbatim. This asserts the codec
 * byte-shape, the input shaping, the spill layout and the connection round-trip.
 */
import { describe, expect, it } from "vitest";

import type {
  GetCurveRequest,
  GetCurveResponse,
} from "../src/contract/contract";
import {
  getCurveRequestToWire,
  getCurveResponseFromWire,
  ratesCurveSetToWire,
} from "../src/contract/wsCodec";
import type { WireObject } from "../src/contract/wsCodec";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import {
  ShapingError,
  formatGetCurveSpill,
  shapeGetCurveRequest,
} from "../src/functions/shaping";

const REQUEST: GetCurveRequest = {
  curveSet: {
    currency: "USD",
    referenceDate: { year: 2026, month: 6, day: 25 },
    pillars: [
      { tenorYears: 2, parRate: 0.0412 },
      { tenorYears: 5, parRate: 0.0418 },
      { tenorYears: 10, parRate: 0.0424 },
    ],
  },
  queryTenorYears: [2, 5, 10],
};

describe("Excel wsCodec — GetCurve", () => {
  it("encodes the snake_case get_curve body, reusing the shared CurveSet encoder", () => {
    const wire = getCurveRequestToWire(REQUEST);
    expect(wire).toEqual({
      curve_set: ratesCurveSetToWire(REQUEST.curveSet),
      query_tenor_years: [2, 5, 10],
    });
    // The shared encoder shapes the `tenor` oneof under `{ years }` (server-decoded).
    expect(wire["curve_set"]).toEqual({
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      ois_pillars: [
        { tenor: { years: 2 }, par_rate: 0.0412 },
        { tenor: { years: 5 }, par_rate: 0.0418 },
        { tenor: { years: 10 }, par_rate: 0.0424 },
      ],
    });
    // Unpinned ⇒ no `curve_version` field on the wire (the server bootstraps live).
    expect("curve_version" in wire).toBe(false);
  });

  it("emits `curve_version` ONLY when a version is pinned", () => {
    const pinned = getCurveRequestToWire({ ...REQUEST, curveVersion: 7 });
    expect(pinned["curve_version"]).toBe(7);
    expect(getCurveRequestToWire(REQUEST)["curve_version"]).toBeUndefined();
  });

  it("decodes a server-shaped get_curve_response frame losslessly (live, unpinned)", () => {
    const frame: WireObject = {
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [
        { tenor_years: 2, zero_rate: 0.04089, discount_factor: 0.92163 },
        { tenor_years: 5, zero_rate: 0.04154, discount_factor: 0.81236 },
        { tenor_years: 10, zero_rate: 0.04211, discount_factor: 0.65712 },
      ],
      par_pillars: [
        { tenor_years: 2.0027, par_rate: 0.0412 },
        { tenor_years: 5.0055, par_rate: 0.0418 },
        { tenor_years: 10.011, par_rate: 0.0424 },
      ],
      epoch_nanos: 1_750_000_000_000_000_000,
    };
    const reply: GetCurveResponse = getCurveResponseFromWire(frame);
    expect(reply).toEqual({
      currency: "USD",
      referenceDate: { year: 2026, month: 6, day: 25 },
      points: [
        { tenorYears: 2, zeroRate: 0.04089, discountFactor: 0.92163 },
        { tenorYears: 5, zeroRate: 0.04154, discountFactor: 0.81236 },
        { tenorYears: 10, zeroRate: 0.04211, discountFactor: 0.65712 },
      ],
      parPillars: [
        { tenorYears: 2.0027, parRate: 0.0412 },
        { tenorYears: 5.0055, parRate: 0.0418 },
        { tenorYears: 10.011, parRate: 0.0424 },
      ],
      epochNanos: 1_750_000_000_000_000_000n,
    });
    // Unpinned read ⇒ no `curveVersion` surfaced (presence-tracked).
    expect("curveVersion" in reply).toBe(false);
  });

  it("surfaces the marked `curveVersion` on a pinned read", () => {
    const reply = getCurveResponseFromWire({
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [{ tenor_years: 2, zero_rate: 0.041, discount_factor: 0.921 }],
      par_pillars: [{ tenor_years: 2.0027, par_rate: 0.0412 }],
      curve_version: 42,
      epoch_nanos: 1_750_000_000_000_000_000,
    });
    expect(reply.curveVersion).toBe(42n);
  });
});

// --- CELNET.GETCURVE input shaping (`shapeGetCurveRequest`) -------------------
describe("get-curve input shaping — a [tenorYears, parRate] range into a GetCurveRequest", () => {
  it("shapes the shared RatesCurveSet and queries at each pillar tenor", () => {
    const req = shapeGetCurveRequest({
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
    expect(req.queryTenorYears).toEqual([2, 5, 10]);
    expect("curveVersion" in req).toBe(false);
  });

  it("carries a pinned version when supplied and defaults the currency to USD", () => {
    const req = shapeGetCurveRequest({
      curve: [[2, 0.0412]],
      referenceDate: "2026-06-25",
      pinnedVersion: 7,
    });
    expect(req.curveVersion).toBe(7);
    expect(req.curveSet.currency).toBe("USD");
  });

  it("treats a blank pinned version as a live bootstrap (no version)", () => {
    const req = shapeGetCurveRequest({
      curve: [[2, 0.0412]],
      referenceDate: "2026-06-25",
      pinnedVersion: "",
    });
    expect("curveVersion" in req).toBe(false);
  });

  it("rejects a negative / non-integer pinned version and an empty curve", () => {
    expect(() =>
      shapeGetCurveRequest({ curve: [[2, 0.0412]], referenceDate: "2026-06-25", pinnedVersion: -1 }),
    ).toThrow(/non-negative integer/);
    expect(() =>
      shapeGetCurveRequest({ curve: [[2, 0.0412]], referenceDate: "2026-06-25", pinnedVersion: 1.5 }),
    ).toThrow(ShapingError);
    expect(() => shapeGetCurveRequest({ curve: [["", ""]], referenceDate: "2026-06-25" })).toThrow(
      /at least one OIS pillar/,
    );
  });
});

// --- CELNET.GETCURVE spill layout (`formatGetCurveSpill`) ---------------------
describe("get-curve spill layout — a labelled 4-column matrix", () => {
  const REPLY: GetCurveResponse = {
    currency: "USD",
    referenceDate: { year: 2026, month: 6, day: 25 },
    points: [
      { tenorYears: 2, zeroRate: 0.04089, discountFactor: 0.92163 },
      { tenorYears: 5, zeroRate: 0.04154, discountFactor: 0.81236 },
    ],
    parPillars: [
      { tenorYears: 2.0027, parRate: 0.0412 },
      { tenorYears: 5.0055, parRate: 0.0418 },
    ],
    epochNanos: 1_750_000_000_000_000_000n,
  };

  it("lays out the points block, the par-pillars block and a live footer", () => {
    const spill = formatGetCurveSpill(REPLY);
    expect(spill).toEqual([
      ["point", "tenor_years", "discount_factor", "zero_rate"],
      ["point[0]", 2, 0.92163, 0.04089],
      ["point[1]", 5, 0.81236, 0.04154],
      ["par_pillar", "tenor_years", "par_rate", ""],
      ["par[0]", 2.0027, 0.0412, ""],
      ["par[1]", 5.0055, 0.0418, ""],
      ["version", "live", "currency", "USD"],
    ]);
    // Office.js custom functions require a rectangular 2-D return.
    for (const row of spill) expect(row.length).toBe(4);
  });

  it("stamps the pinned version `v<n>` in the footer on a pinned read", () => {
    const spill = formatGetCurveSpill({ ...REPLY, curveVersion: 42n });
    expect(spill[spill.length - 1]).toEqual(["version", "v42", "currency", "USD"]);
  });
});

// --- Connection.getCurve round-trip (mirrors the buildCurve seam) -------------
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

describe("Connection.getCurve round-trip", () => {
  it("sends a `get_curve` frame with the wire body and decodes the get_curve_response reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.getCurve(REQUEST);
    const sent = sock.sentOfType("get_curve")[0]!;
    // The wire body is the codec output; the connection injects only correlation_id.
    for (const [k, v] of Object.entries(getCurveRequestToWire(REQUEST))) {
      expect(sent[k]).toEqual(v);
    }
    // `get_curve` is a pure calculation (like get_smile) — no session token rides it.
    expect("session_token" in sent).toBe(false);
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "get_curve_response",
      correlation_id: corr,
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [{ tenor_years: 2, zero_rate: 0.04089, discount_factor: 0.92163 }],
      par_pillars: [{ tenor_years: 2.0027, par_rate: 0.0412 }],
      epoch_nanos: 1_750_000_000_000_000_000,
    });
    const reply = await p;
    expect(reply.currency).toBe("USD");
    expect(reply.points).toEqual([{ tenorYears: 2, zeroRate: 0.04089, discountFactor: 0.92163 }]);
    expect(reply.parPillars).toEqual([{ tenorYears: 2.0027, parRate: 0.0412 }]);
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.getCurve({ ...REQUEST, curveVersion: 999 });
    const sent = sock.sentOfType("get_curve")[0]!;
    expect(sent["curve_version"]).toBe(999);
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "error",
      correlation_id: corr,
      message: "failed_precondition: unknown curve version",
    });
    await expect(p).rejects.toThrow(/failed_precondition/);
  });
});
