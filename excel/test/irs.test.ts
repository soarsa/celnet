// Fixed-income (rates) vanilla-swap slice — the `price_rates` add-in path carrying the
// `RatesInstrument.irs` oneof arm: shape the scalar IRS terms into the typed contract,
// encode to the wire EXACTLY as the server decodes it
// (`crates/celnet-server/src/ws/codec.rs` `vanilla_irs_instrument_from_json` +
// `crates/celnet-proto/proto/celnet.proto` `VanillaIrsInstrument` / `RatesPricingResult`),
// decode a `rates_price_response`, and lay out the priced swap result as a spill. The
// add-in holds no swap math: it is a thin client of the live `celnet-rates` engine over
// the one unversioned contract, byte-identical to the GUI's `ratesInstrumentUnionToWire`.
//
// Wire-shape oracle (verified against codec.rs): the `irs` body is ALL FLAT scalars +
// numeric enum tags (NO nested date — the swap is spot-starting, its schedule rebuilt
// server-side from the curve reference date + `tenor_years`; a nested effective/maturity
// date would be rejected). Enums are raw integers: side (SIDE_BUY=0 = PAY_FIXED, SIDE_SELL=1
// = RECEIVE_FIXED); fixed_frequency/float_frequency = PaymentFrequency (ANNUAL=0,
// SEMI_ANNUAL=1, QUARTERLY=2); fixed_day_count/float_day_count = DayCount (ACT_365_FIXED=0,
// ACT_360=1 — a leg has no 30/360 arm). A swap result is the SHARED `RatesPricingResult`
// (pv/par_rate/pv01/dv01 + a pillar-shaped key-rate ladder).
import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { RatesCurveSet, VanillaIrsInstrument } from "../src/contract/contract";
import {
  irsInstrumentToWire,
  ratesCurveSetToWire,
  ratesPricingResultFromWire,
} from "../src/contract/wsCodec";
import {
  ShapingError,
  formatRatesSpill,
  parsePaymentFrequency,
  parseRatesLegDayCount,
  parseRatesSide,
  shapeIrsInstrument,
} from "../src/functions/shaping";

// --- in-memory socket (mirrors test/bond.test.ts / test/rates.test.ts) -------
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

// A 3-pillar self-discounting USD-SOFR curve and the canonical 5Y USD-market swap
// (receive-fixed, SEMI_ANNUAL/ACT_360 fixed vs QUARTERLY/ACT_360 float).
const CURVE: RatesCurveSet = {
  currency: "USD",
  referenceDate: { year: 2025, month: 6, day: 16 },
  pillars: [
    { tenorYears: 2, parRate: 0.0405 },
    { tenorYears: 5, parRate: 0.041 },
    { tenorYears: 10, parRate: 0.0418 },
  ],
};
const IRS: VanillaIrsInstrument = {
  tenorYears: 5,
  fixedRate: 0.04,
  notional: 10_000_000,
  direction: "RECEIVE_FIXED",
  fixedFrequency: "SEMI_ANNUAL",
  fixedDayCount: "ACT_360",
  floatFrequency: "QUARTERLY",
  floatDayCount: "ACT_360",
};

describe("IRS wire codec — exact field names the engine decodes", () => {
  it("encodes the irs arm as { irs: { tenor_years, fixed_rate, notional, side, fixed_frequency, fixed_day_count, float_frequency, float_day_count } }", () => {
    // Byte-identical to the server encoder + round-trip oracle in codec.rs and the
    // GUI's `ratesInstrumentUnionToWire` `irs` arm. Every field is FLAT (no nested
    // date): the swap is spot-starting, so there is no effective/maturity date on the
    // wire — the schedule is rebuilt from the curve reference date + tenor_years.
    expect(irsInstrumentToWire(IRS)).toEqual({
      irs: {
        tenor_years: 5,
        fixed_rate: 0.04,
        notional: 10_000_000,
        side: 1,
        fixed_frequency: 1,
        fixed_day_count: 1,
        float_frequency: 2,
        float_day_count: 1,
      },
    });
  });

  it("keeps the irs body entirely flat — no nested date object (the spot-starting trap)", () => {
    const body = irsInstrumentToWire(IRS).irs as Record<string, unknown>;
    for (const v of Object.values(body)) {
      expect(typeof v).toBe("number");
    }
  });

  it("maps fixed/float frequency to the PaymentFrequency proto tag (ANNUAL=0, SEMI_ANNUAL=1, QUARTERLY=2)", () => {
    const fixedTag = (f: VanillaIrsInstrument["fixedFrequency"]): unknown =>
      (irsInstrumentToWire({ ...IRS, fixedFrequency: f }).irs as Record<string, unknown>)[
        "fixed_frequency"
      ];
    const floatTag = (f: VanillaIrsInstrument["floatFrequency"]): unknown =>
      (irsInstrumentToWire({ ...IRS, floatFrequency: f }).irs as Record<string, unknown>)[
        "float_frequency"
      ];
    expect(fixedTag("ANNUAL")).toBe(0);
    expect(fixedTag("SEMI_ANNUAL")).toBe(1);
    expect(fixedTag("QUARTERLY")).toBe(2);
    expect(floatTag("ANNUAL")).toBe(0);
    expect(floatTag("SEMI_ANNUAL")).toBe(1);
    expect(floatTag("QUARTERLY")).toBe(2);
  });

  it("maps fixed/float day-count to the DayCount proto tag (ACT_365_FIXED=0, ACT_360=1 — no 30/360 leg arm)", () => {
    const fixedTag = (d: VanillaIrsInstrument["fixedDayCount"]): unknown =>
      (irsInstrumentToWire({ ...IRS, fixedDayCount: d }).irs as Record<string, unknown>)[
        "fixed_day_count"
      ];
    const floatTag = (d: VanillaIrsInstrument["floatDayCount"]): unknown =>
      (irsInstrumentToWire({ ...IRS, floatDayCount: d }).irs as Record<string, unknown>)[
        "float_day_count"
      ];
    expect(fixedTag("ACT_365_FIXED")).toBe(0);
    expect(fixedTag("ACT_360")).toBe(1);
    expect(floatTag("ACT_365_FIXED")).toBe(0);
    expect(floatTag("ACT_360")).toBe(1);
  });

  it("maps PAY_FIXED → side 0 (BUY) and RECEIVE_FIXED → side 1 (SELL)", () => {
    const paySide = (irsInstrumentToWire({ ...IRS, direction: "PAY_FIXED" }).irs as Record<string, unknown>)["side"];
    const recvSide = (irsInstrumentToWire({ ...IRS, direction: "RECEIVE_FIXED" }).irs as Record<string, unknown>)["side"];
    expect(paySide).toBe(0);
    expect(recvSide).toBe(1);
  });

  it("decodes a swap rates_price_response.result (pillar-shaped key-rate ladder)", () => {
    const frame = {
      result: {
        pv: -12345.6,
        par_rate: 0.0412,
        pv01: 4321.0,
        dv01: 4300.5,
        key_rate_ladder: [1200.1, 2100.2, 1000.2],
      },
    };
    expect(ratesPricingResultFromWire(frame)).toEqual({
      pv: -12345.6,
      parRate: 0.0412,
      pv01: 4321.0,
      dv01: 4300.5,
      keyRateLadder: [1200.1, 2100.2, 1000.2],
    });
  });
});

describe("IRS input shaping", () => {
  it("parses payment frequency with the canonical names, desk aliases, and a per-leg fallback", () => {
    expect(parsePaymentFrequency("ANNUAL", "SEMI_ANNUAL")).toBe("ANNUAL");
    expect(parsePaymentFrequency("q", "SEMI_ANNUAL")).toBe("QUARTERLY");
    expect(parsePaymentFrequency("SA", "ANNUAL")).toBe("SEMI_ANNUAL");
    // Empty/absent ⇒ the supplied leg fallback.
    expect(parsePaymentFrequency(undefined, "SEMI_ANNUAL")).toBe("SEMI_ANNUAL");
    expect(parsePaymentFrequency("", "QUARTERLY")).toBe("QUARTERLY");
    expect(() => parsePaymentFrequency("MONTHLY", "ANNUAL")).toThrow(ShapingError);
  });

  it("parses an IRS leg day-count (ACT_360 / ACT_365_FIXED) and rejects 30/360 with a reason", () => {
    expect(parseRatesLegDayCount("ACT_360", "ACT_360")).toBe("ACT_360");
    expect(parseRatesLegDayCount("act365f", "ACT_360")).toBe("ACT_365_FIXED");
    expect(parseRatesLegDayCount(undefined, "ACT_360")).toBe("ACT_360");
    expect(parseRatesLegDayCount("", "ACT_365_FIXED")).toBe("ACT_365_FIXED");
    // 30/360 is not a leg basis — rejected with an explicit message, not coerced.
    expect(() => parseRatesLegDayCount("30/360", "ACT_360")).toThrow(/not a leg basis/);
    expect(() => parseRatesLegDayCount("ACT_ACT", "ACT_360")).toThrow(ShapingError);
  });

  it("parses side with a PAY_FIXED default and the PAY/RECEIVE aliases", () => {
    expect(parseRatesSide(undefined)).toBe("PAY_FIXED");
    expect(parseRatesSide("")).toBe("PAY_FIXED");
    expect(parseRatesSide("payer")).toBe("PAY_FIXED");
    expect(parseRatesSide("RECEIVE_FIXED")).toBe("RECEIVE_FIXED");
    expect(parseRatesSide("rec")).toBe("RECEIVE_FIXED");
    expect(() => parseRatesSide("LONG")).toThrow(ShapingError);
  });

  it("shapes an IRS with the USD-market defaults (PAY_FIXED / SEMI_ANNUAL·ACT_360 fixed / QUARTERLY·ACT_360 float)", () => {
    const irs = shapeIrsInstrument({ tenor: 5, fixedRate: 0.04, notional: 10_000_000 });
    expect(irs).toEqual({
      tenorYears: 5,
      fixedRate: 0.04,
      notional: 10_000_000,
      direction: "PAY_FIXED",
      fixedFrequency: "SEMI_ANNUAL",
      fixedDayCount: "ACT_360",
      floatFrequency: "QUARTERLY",
      floatDayCount: "ACT_360",
    });
  });

  it("shapes explicit overrides (a receive-fixed annual ACT/365F fixed vs annual ACT/365F float, `5Y` string tenor)", () => {
    const irs = shapeIrsInstrument({
      tenor: "5Y",
      fixedRate: 0.045,
      notional: 25_000_000,
      side: "RECEIVE_FIXED",
      fixedFrequency: "ANNUAL",
      fixedDayCount: "ACT_365_FIXED",
      floatFrequency: "ANNUAL",
      floatDayCount: "ACT_365_FIXED",
    });
    expect(irs).toEqual({
      tenorYears: 5,
      fixedRate: 0.045,
      notional: 25_000_000,
      direction: "RECEIVE_FIXED",
      fixedFrequency: "ANNUAL",
      fixedDayCount: "ACT_365_FIXED",
      floatFrequency: "ANNUAL",
      floatDayCount: "ACT_365_FIXED",
    });
  });

  it("rejects a sub-1Y tenor, a non-finite fixed rate, and a non-positive notional", () => {
    expect(() => shapeIrsInstrument({ tenor: 0, fixedRate: 0.04, notional: 1e6 })).toThrow(ShapingError);
    expect(() => shapeIrsInstrument({ tenor: 5, fixedRate: Number.NaN, notional: 1e6 })).toThrow(/finite/);
    expect(() => shapeIrsInstrument({ tenor: 5, fixedRate: 0.04, notional: 0 })).toThrow(/positive/);
  });
});

describe("IRS spill layout", () => {
  it("lays out pv / par_rate / pv01 / dv01 then the tenor-labelled key-rate ladder (reusing formatRatesSpill)", () => {
    const spill = formatRatesSpill(
      {
        pv: -12345.6,
        parRate: 0.0412,
        pv01: 4321.0,
        dv01: 4300.5,
        keyRateLadder: [1200.1, 2100.2, 1000.2],
      },
      CURVE.pillars,
    );
    expect(spill).toEqual([
      ["pv", -12345.6],
      ["par_rate", 0.0412],
      ["pv01", 4321.0],
      ["dv01", 4300.5],
      ["kr_dv01[2Y]", 1200.1],
      ["kr_dv01[5Y]", 2100.2],
      ["kr_dv01[10Y]", 1000.2],
    ]);
    for (const row of spill) expect(row.length).toBe(2);
  });
});

describe("Connection.priceRatesIrs round-trip", () => {
  it("sends a `price_rates` frame carrying the irs arm and decodes the reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceRatesIrs(CURVE, IRS);
    const sent = sock.sentOfType("price_rates")[0]!;
    expect(sent["curve_set"]).toEqual(ratesCurveSetToWire(CURVE));
    expect(sent["instrument"]).toEqual(irsInstrumentToWire(IRS));
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "rates_price_response",
      correlation_id: corr,
      result: {
        pv: -12345.6,
        par_rate: 0.0412,
        pv01: 4321.0,
        dv01: 4300.5,
        key_rate_ladder: [1200.1, 2100.2, 1000.2],
      },
    });
    const result = await p;
    expect(result).toEqual({
      pv: -12345.6,
      parRate: 0.0412,
      pv01: 4321.0,
      dv01: 4300.5,
      keyRateLadder: [1200.1, 2100.2, 1000.2],
    });
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceRatesIrs(CURVE, IRS);
    const corr = sock.sentOfType("price_rates")[0]!["correlation_id"] as number;
    sock.deliver({
      type: "error",
      correlation_id: corr,
      message: "invalid_argument: IRS tenor must be a whole number of years",
    });
    await expect(p).rejects.toThrow(/invalid_argument/);
  });
});
