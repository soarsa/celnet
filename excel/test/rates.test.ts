// Fixed-income (rates) slice — the `price_rates` add-in path: shape a curve range
// + OIS terms into the typed contract, encode to the wire EXACTLY as the server
// decodes it (`crates/celnet-proto/proto/celnet.proto` messages `CurveSet`,
// `OisInstrument`, `RatesPricingResult`), decode a `rates_price_response`, and lay
// out the priced result as a spill. The add-in holds no rates math: it is a thin
// client of the live `celnet-rates` engine over the one unversioned contract.
import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { OisInstrument, RatesCurveSet } from "../src/contract/contract";
import {
  ratesCurveSetToWire,
  ratesInstrumentToWire,
  ratesPricingResultFromWire,
} from "../src/contract/wsCodec";
import {
  ShapingError,
  formatRatesSpill,
  parseBrokenDate,
  parseOisDirection,
  parseOisTenorYears,
  shapeOisInstrument,
  shapeRatesCurve,
} from "../src/functions/shaping";

// --- in-memory socket (mirrors test/connection.test.ts) ---------------------
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

// A 3-pillar self-discounting USD-SOFR curve and a 5Y payer OIS.
const CURVE: RatesCurveSet = {
  currency: "USD",
  referenceDate: { year: 2025, month: 6, day: 16 },
  pillars: [
    { tenorYears: 2, parRate: 0.0405 },
    { tenorYears: 5, parRate: 0.041 },
    { tenorYears: 10, parRate: 0.0418 },
  ],
};
const OIS: OisInstrument = {
  tenorYears: 5,
  fixedRate: 0.041,
  notional: 10_000_000,
  direction: "PAY_FIXED",
};

describe("rates wire codec — exact field names the engine decodes", () => {
  it("encodes the curve set as { currency, reference_date{y,m,d}, ois_pillars[{tenor_years,par_rate}] }", () => {
    const wire = ratesCurveSetToWire(CURVE);
    expect(wire).toEqual({
      currency: "USD",
      reference_date: { year: 2025, month: 6, day: 16 },
      ois_pillars: [
        { tenor_years: 2, par_rate: 0.0405 },
        { tenor_years: 5, par_rate: 0.041 },
        { tenor_years: 10, par_rate: 0.0418 },
      ],
    });
  });

  it("encodes the OIS arm as { ois: { tenor_years, fixed_rate, notional, side } }", () => {
    expect(ratesInstrumentToWire(OIS)).toEqual({
      ois: { tenor_years: 5, fixed_rate: 0.041, notional: 10_000_000, side: 0 },
    });
  });

  it("maps PAY_FIXED → side 0 (BUY) and RECEIVE_FIXED → side 1 (SELL)", () => {
    const payer = ratesInstrumentToWire({ ...OIS, direction: "PAY_FIXED" }).ois as Record<string, unknown>;
    const receiver = ratesInstrumentToWire({ ...OIS, direction: "RECEIVE_FIXED" }).ois as Record<string, unknown>;
    expect(payer["side"]).toBe(0);
    expect(receiver["side"]).toBe(1);
  });

  it("decodes a rates_price_response.result into the typed RatesPricingResult", () => {
    const frame = {
      result: {
        pv: 12_345.67,
        par_rate: 0.0411,
        pv01: 4_500.0,
        dv01: 4_520.0,
        key_rate_ladder: [900.0, 2100.0, 1520.0],
      },
    };
    expect(ratesPricingResultFromWire(frame)).toEqual({
      pv: 12_345.67,
      parRate: 0.0411,
      pv01: 4_500.0,
      dv01: 4_520.0,
      keyRateLadder: [900.0, 2100.0, 1520.0],
    });
  });

  it("rejects a malformed key_rate_ladder (non-array / non-finite entries)", () => {
    expect(() => ratesPricingResultFromWire({ result: { key_rate_ladder: 7 } })).toThrow(/must be an array/);
    expect(() =>
      ratesPricingResultFromWire({ result: { key_rate_ladder: [1, "x"] } }),
    ).toThrow(/finite number/);
  });
});

describe("rates input shaping", () => {
  it("parses an Excel date serial and an ISO string to the same BrokenDate", () => {
    // Excel serial 44927 is 2023-01-01 (epoch 1899-12-30).
    expect(parseBrokenDate(44927)).toEqual({ year: 2023, month: 1, day: 1 });
    expect(parseBrokenDate("2025-06-16")).toEqual({ year: 2025, month: 6, day: 16 });
  });

  it("rejects a non-date reference value", () => {
    expect(() => parseBrokenDate("16/06/2025")).toThrow(ShapingError);
    expect(() => parseBrokenDate(0)).toThrow(ShapingError);
  });

  it("parses OIS tenor as whole years from a number or a `5Y` string", () => {
    expect(parseOisTenorYears(5)).toBe(5);
    expect(parseOisTenorYears("10Y")).toBe(10);
    expect(() => parseOisTenorYears(0)).toThrow(/>= 1/);
    expect(() => parseOisTenorYears(2.5)).toThrow(/whole number/);
    expect(() => parseOisTenorYears("3M")).toThrow(ShapingError);
  });

  it("parses direction with the desk aliases and rejects anything else", () => {
    expect(parseOisDirection("PAY_FIXED")).toBe("PAY_FIXED");
    expect(parseOisDirection("payer")).toBe("PAY_FIXED");
    expect(parseOisDirection("Pay")).toBe("PAY_FIXED");
    expect(parseOisDirection("RECEIVE_FIXED")).toBe("RECEIVE_FIXED");
    expect(parseOisDirection("receiver")).toBe("RECEIVE_FIXED");
    expect(parseOisDirection("rec")).toBe("RECEIVE_FIXED");
    expect(() => parseOisDirection("SELL")).toThrow(ShapingError);
  });

  it("shapes a curve range, defaulting the currency to USD and ignoring blank rows", () => {
    const curve = shapeRatesCurve({
      curve: [
        [2, 0.0405],
        [5, 0.041],
        ["", ""],
        [10, 0.0418],
        ["", ""],
      ],
      referenceDate: "2025-06-16",
    });
    expect(curve.currency).toBe("USD");
    expect(curve.referenceDate).toEqual({ year: 2025, month: 6, day: 16 });
    expect(curve.pillars).toEqual([
      { tenorYears: 2, parRate: 0.0405 },
      { tenorYears: 5, parRate: 0.041 },
      { tenorYears: 10, parRate: 0.0418 },
    ]);
  });

  it("rejects out-of-order pillars, an empty curve, and a non-USD currency code shape", () => {
    expect(() =>
      shapeRatesCurve({
        curve: [
          [5, 0.041],
          [2, 0.0405],
        ],
        referenceDate: "2025-06-16",
      }),
    ).toThrow(/strictly increasing/);
    expect(() => shapeRatesCurve({ curve: [["", ""]], referenceDate: "2025-06-16" })).toThrow(
      /at least one OIS pillar/,
    );
    expect(() =>
      shapeRatesCurve({ curve: [[2, 0.04]], referenceDate: "2025-06-16", currency: "US" }),
    ).toThrow(/3-letter ISO/);
  });

  it("shapes an OIS instrument, requiring a positive notional and finite fixed rate", () => {
    expect(shapeOisInstrument({ tenor: "5Y", fixedRate: 0.041, direction: "PAY_FIXED", notional: 1e7 })).toEqual(OIS);
    expect(() =>
      shapeOisInstrument({ tenor: 5, fixedRate: 0.041, direction: "PAY_FIXED", notional: -1 }),
    ).toThrow(/positive/);
    expect(() =>
      shapeOisInstrument({ tenor: 5, fixedRate: Number.NaN, direction: "PAY_FIXED", notional: 1e7 }),
    ).toThrow(/finite/);
  });
});

describe("rates spill layout", () => {
  it("lays out pv/par_rate/pv01/dv01 then a tenor-labelled key-rate ladder", () => {
    const spill = formatRatesSpill(
      { pv: 12_345.67, parRate: 0.0411, pv01: 4_500, dv01: 4_520, keyRateLadder: [900, 2100, 1520] },
      CURVE.pillars,
    );
    expect(spill).toEqual([
      ["pv", 12_345.67],
      ["par_rate", 0.0411],
      ["pv01", 4_500],
      ["dv01", 4_520],
      ["kr_dv01[2Y]", 900],
      ["kr_dv01[5Y]", 2100],
      ["kr_dv01[10Y]", 1520],
    ]);
    // Office.js custom functions require a rectangular 2-D return.
    for (const row of spill) expect(row.length).toBe(2);
  });

  it("falls back to positional ladder labels when the engine count != pillar count", () => {
    const spill = formatRatesSpill(
      { pv: 1, parRate: 0.04, pv01: 2, dv01: 3, keyRateLadder: [10, 20] },
      CURVE.pillars,
    );
    expect(spill.slice(4)).toEqual([
      ["kr_dv01[0]", 10],
      ["kr_dv01[1]", 20],
    ]);
  });
});

describe("Connection.priceRates round-trip", () => {
  it("sends a `price_rates` frame with the wire bodies and decodes the reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceRates(CURVE, OIS);
    const sent = sock.sentOfType("price_rates")[0]!;
    expect(sent["curve_set"]).toEqual(ratesCurveSetToWire(CURVE));
    expect(sent["instrument"]).toEqual(ratesInstrumentToWire(OIS));
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "rates_price_response",
      correlation_id: corr,
      result: {
        pv: 12_345.67,
        par_rate: 0.0411,
        pv01: 4_500,
        dv01: 4_520,
        key_rate_ladder: [900, 2100, 1520],
      },
    });
    const result = await p;
    expect(result).toEqual({
      pv: 12_345.67,
      parRate: 0.0411,
      pv01: 4_500,
      dv01: 4_520,
      keyRateLadder: [900, 2100, 1520],
    });
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceRates(CURVE, OIS);
    const corr = sock.sentOfType("price_rates")[0]!["correlation_id"] as number;
    sock.deliver({ type: "error", correlation_id: corr, message: "failed_precondition: non-USD curve" });
    await expect(p).rejects.toThrow(/failed_precondition/);
  });
});
