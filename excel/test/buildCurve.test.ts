/**
 * BuildCurve codec parity (Excel add-in). The Excel client speaks the SAME
 * type-tagged snake_case wire as the GUI and the server's `ws/codec.rs`: the
 * `build_curve` request body and the `calibrated_curve` response frame must encode
 * / decode field-for-field identically across every client (GUIDE.md #9 — one
 * clean current contract). This asserts the round-trip on the Excel codec.
 */
import { describe, expect, it } from "vitest";

import type {
  BuildCurveRequest,
  CalibratedCurve,
} from "../src/contract/contract";
import {
  buildCurveRequestToWire,
  calibratedCurveFromWire,
} from "../src/contract/wsCodec";
import type { WireObject } from "../src/contract/wsCodec";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import {
  ShapingError,
  formatCalibratedCurveSpill,
  shapeBuildCurveRequest,
} from "../src/functions/shaping";

const REQUEST: BuildCurveRequest = {
  requestId: "curve-001",
  currency: "USD",
  referenceDate: { year: 2026, month: 6, day: 25 },
  pillars: [
    { instrumentId: "usd-sofr-depo-3m", quote: 0.0431 },
    { instrumentId: "usd-sofr-irs-10y", quote: 0.0418 },
  ],
  datePillars: [
    { maturityDate: { year: 2027, month: 12, day: 31 }, quote: 0.0415 },
  ],
};

describe("Excel wsCodec — BuildCurve", () => {
  it("encodes the snake_case build_curve body (connection injects auth)", () => {
    const wire = buildCurveRequestToWire(REQUEST);
    expect(wire).toEqual({
      request_id: "curve-001",
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      pillars: [
        { instrument_id: "usd-sofr-depo-3m", quote: 0.0431 },
        { instrument_id: "usd-sofr-irs-10y", quote: 0.0418 },
      ],
      date_pillars: [
        { maturity_date: { year: 2027, month: 12, day: 31 }, quote: 0.0415 },
      ],
    });
    expect("session_token" in wire).toBe(false);
  });

  it("decodes a server-shaped calibrated_curve frame losslessly", () => {
    const frame: WireObject = {
      request_id: "curve-001",
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [
        {
          instrument_id: "usd-sofr-depo-3m",
          time_years: 0.2521,
          discount_factor: 0.98912,
          zero_rate: 0.04318,
          label: "",
        },
        {
          instrument_id: "",
          time_years: 1.5151,
          discount_factor: 0.93827,
          zero_rate: 0.0415,
          label: "Date 2027-12-31",
        },
        {
          instrument_id: "usd-sofr-irs-10y",
          time_years: 10.0,
          discount_factor: 0.6612,
          zero_rate: 0.04134,
          label: "",
        },
      ],
    };
    const curve: CalibratedCurve = calibratedCurveFromWire(frame);
    expect(curve).toEqual({
      requestId: "curve-001",
      currency: "USD",
      referenceDate: { year: 2026, month: 6, day: 25 },
      points: [
        {
          instrumentId: "usd-sofr-depo-3m",
          timeYears: 0.2521,
          discountFactor: 0.98912,
          zeroRate: 0.04318,
          label: "",
        },
        {
          instrumentId: "",
          timeYears: 1.5151,
          discountFactor: 0.93827,
          zeroRate: 0.0415,
          label: "Date 2027-12-31",
        },
        {
          instrumentId: "usd-sofr-irs-10y",
          timeYears: 10.0,
          discountFactor: 0.6612,
          zeroRate: 0.04134,
          label: "",
        },
      ],
    });
  });

  it("round-trips the request header through the wire (server echoes it back)", () => {
    const wire = buildCurveRequestToWire(REQUEST);
    const echoed: WireObject = {
      request_id: wire.request_id,
      currency: wire.currency,
      reference_date: wire.reference_date,
      points: [],
    };
    const curve = calibratedCurveFromWire(echoed);
    expect(curve.requestId).toBe(REQUEST.requestId);
    expect(curve.currency).toBe(REQUEST.currency);
    expect(curve.referenceDate).toEqual(REQUEST.referenceDate);
  });
});

// --- CELNET.CURVE input shaping (`shapeBuildCurveRequest`) --------------------
describe("curve input shaping — a [pillar, quote] range into a BuildCurveRequest", () => {
  it("classifies non-date strings as registry-instrument pillars", () => {
    const req = shapeBuildCurveRequest({
      pillars: [
        ["usd-sofr-depo-3m", 0.0431],
        ["usd-sofr-irs-10y", 0.0418],
      ],
      referenceDate: "2026-06-25",
    });
    expect(req.pillars).toEqual([
      { instrumentId: "usd-sofr-depo-3m", quote: 0.0431 },
      { instrumentId: "usd-sofr-irs-10y", quote: 0.0418 },
    ]);
    expect(req.datePillars).toEqual([]);
    expect(req.currency).toBe("USD");
    expect(req.referenceDate).toEqual({ year: 2026, month: 6, day: 25 });
    expect(req.requestId).toMatch(/^curve-\d+$/);
  });

  it("classifies a `YYYY-MM-DD` string and an Excel date serial as date-anchored pillars", () => {
    const req = shapeBuildCurveRequest({
      pillars: [
        ["2027-12-31", 0.0415],
        [44927, 0.0402], // Excel serial 44927 = 2023-01-01
      ],
      referenceDate: "2026-06-25",
    });
    expect(req.pillars).toEqual([]);
    expect(req.datePillars).toEqual([
      { maturityDate: { year: 2027, month: 12, day: 31 }, quote: 0.0415 },
      { maturityDate: { year: 2023, month: 1, day: 1 }, quote: 0.0402 },
    ]);
  });

  it("mixes registry + date pillars in one range and ignores blank trailing rows", () => {
    const req = shapeBuildCurveRequest({
      pillars: [
        ["usd-sofr-depo-3m", 0.0431],
        ["", ""],
        ["2027-12-31", 0.0415],
        ["usd-sofr-irs-10y", 0.0418],
        ["", ""],
      ],
      referenceDate: "2026-06-25",
      currency: "usd",
    });
    expect(req.pillars).toEqual([
      { instrumentId: "usd-sofr-depo-3m", quote: 0.0431 },
      { instrumentId: "usd-sofr-irs-10y", quote: 0.0418 },
    ]);
    expect(req.datePillars).toEqual([
      { maturityDate: { year: 2027, month: 12, day: 31 }, quote: 0.0415 },
    ]);
    expect(req.currency).toBe("USD");
  });

  it("rejects an empty curve, a non-numeric quote, a boolean pillar and a bad currency", () => {
    expect(() => shapeBuildCurveRequest({ pillars: [["", ""]], referenceDate: "2026-06-25" })).toThrow(
      /at least one pillar/,
    );
    expect(() =>
      shapeBuildCurveRequest({ pillars: [["usd-sofr-depo-3m", "x"]], referenceDate: "2026-06-25" }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeBuildCurveRequest({ pillars: [[true, 0.04]], referenceDate: "2026-06-25" }),
    ).toThrow(/instrument id or a maturity date/);
    expect(() =>
      shapeBuildCurveRequest({ pillars: [["usd-sofr-depo-3m", 0.04]], referenceDate: "2026-06-25", currency: "US" }),
    ).toThrow(/3-letter ISO/);
  });
});

// --- CELNET.CURVE spill layout (`formatCalibratedCurveSpill`) -----------------
describe("curve spill layout — a labelled (1 + points)x4 matrix", () => {
  const CURVE: CalibratedCurve = {
    requestId: "curve-001",
    currency: "USD",
    referenceDate: { year: 2026, month: 6, day: 25 },
    points: [
      { instrumentId: "usd-sofr-depo-3m", timeYears: 0.2521, discountFactor: 0.98912, zeroRate: 0.04318, label: "" },
      { instrumentId: "", timeYears: 1.5151, discountFactor: 0.93827, zeroRate: 0.0415, label: "Date 2027-12-31" },
      { instrumentId: "usd-sofr-irs-10y", timeYears: 10.0, discountFactor: 0.6612, zeroRate: 0.04134, label: "" },
    ],
  };

  it("lays out a header then one row per pillar, labelling by label > instrumentId > positional", () => {
    const spill = formatCalibratedCurveSpill(CURVE);
    expect(spill).toEqual([
      ["pillar", "time_years", "discount_factor", "zero_rate"],
      ["usd-sofr-depo-3m", 0.2521, 0.98912, 0.04318],
      ["Date 2027-12-31", 1.5151, 0.93827, 0.0415],
      ["usd-sofr-irs-10y", 10.0, 0.6612, 0.04134],
    ]);
    // Office.js custom functions require a rectangular 2-D return.
    for (const row of spill) expect(row.length).toBe(4);
  });

  it("falls back to a positional pillar[i] label when both label and instrumentId are blank", () => {
    const spill = formatCalibratedCurveSpill({
      ...CURVE,
      points: [{ instrumentId: "", timeYears: 0.5, discountFactor: 0.98, zeroRate: 0.04, label: "" }],
    });
    expect(spill[1]).toEqual(["pillar[0]", 0.5, 0.98, 0.04]);
  });
});

// --- Connection.buildCurve round-trip (mirrors the priceRates seam) -----------
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

describe("Connection.buildCurve round-trip", () => {
  it("sends a `build_curve` frame with the wire body and decodes the calibrated_curve reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.buildCurve(REQUEST);
    const sent = sock.sentOfType("build_curve")[0]!;
    // The wire body is the codec output; the connection injects only correlation_id.
    for (const [k, v] of Object.entries(buildCurveRequestToWire(REQUEST))) {
      expect(sent[k]).toEqual(v);
    }
    expect("session_token" in sent).toBe(false);
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "calibrated_curve",
      correlation_id: corr,
      request_id: REQUEST.requestId,
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [
        { instrument_id: "usd-sofr-depo-3m", time_years: 0.2521, discount_factor: 0.98912, zero_rate: 0.04318, label: "" },
      ],
    });
    const curve = await p;
    expect(curve.requestId).toBe(REQUEST.requestId);
    expect(curve.points).toEqual([
      { instrumentId: "usd-sofr-depo-3m", timeYears: 0.2521, discountFactor: 0.98912, zeroRate: 0.04318, label: "" },
    ]);
  });

  it("rides the held session token when authenticated (BuildCurve REQUIRES it server-side)", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    conn.setSessionToken("tok-abc");
    const p = conn.buildCurve(REQUEST);
    const sent = sock.sentOfType("build_curve")[0]!;
    expect(sent["session_token"]).toBe("tok-abc");
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "calibrated_curve",
      correlation_id: corr,
      request_id: REQUEST.requestId,
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [],
    });
    await p;
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.buildCurve(REQUEST);
    const corr = sock.sentOfType("build_curve")[0]!["correlation_id"] as number;
    sock.deliver({ type: "error", correlation_id: corr, message: "not_found: unknown instrument id" });
    await expect(p).rejects.toThrow(/not_found/);
  });
});
