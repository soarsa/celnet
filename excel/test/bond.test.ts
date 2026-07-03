// Fixed-income (rates) cash-bond slice — the `price_rates` add-in path carrying the
// `RatesInstrument.bond` oneof arm: shape the scalar bond terms into the typed
// contract, encode to the wire EXACTLY as the server decodes it
// (`crates/celnet-server/src/ws/codec.rs` `bond_instrument_from_json` +
// `crates/celnet-proto/proto/celnet.proto` `BondInstrument` / `RatesPricingResult`),
// decode a `rates_price_response`, and lay out the priced bond result as a spill.
// The add-in holds no bond math: it is a thin client of the live `celnet-rates`
// bond engine over the one unversioned contract.
//
// Wire-shape oracle (verified against codec.rs): the `bond` body is FLAT scalars +
// numeric enum tags EXCEPT `maturity_date`, which nests a `BrokenDate`
// {year, month, day}. Enums are raw integers: coupon_frequency (ANNUAL=0,
// SEMI_ANNUAL=1, QUARTERLY=2), day_count/AccrualBasis (ACT_360=0, ACT_365_FIXED=1,
// THIRTY_360_BOND_BASIS=2), side (SIDE_BUY=0 long, SIDE_SELL=1 short). The bond
// price response is the SHARED `RatesPricingResult` with bond semantics —
// pv = dirty price, par_rate = yield to maturity, pv01 = dv01 = yield DV01, and an
// EMPTY key_rate_ladder.
import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { BondInstrument, RatesCurveSet } from "../src/contract/contract";
import {
  bondInstrumentToWire,
  ratesCurveSetToWire,
  ratesPricingResultFromWire,
} from "../src/contract/wsCodec";
import {
  ShapingError,
  formatBondSpill,
  parseBondCouponFrequency,
  parseBondDayCount,
  parseBondSide,
  shapeBondInstrument,
} from "../src/functions/shaping";

// --- in-memory socket (mirrors test/rates.test.ts) --------------------------
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

// A 3-pillar self-discounting USD-SOFR discount curve and a 6Y 6% semi-annual bond.
const CURVE: RatesCurveSet = {
  currency: "USD",
  referenceDate: { year: 2025, month: 6, day: 16 },
  pillars: [
    { tenorYears: 2, parRate: 0.0405 },
    { tenorYears: 5, parRate: 0.041 },
    { tenorYears: 10, parRate: 0.0418 },
  ],
};
const BOND: BondInstrument = {
  couponRate: 0.06,
  couponFrequency: "SEMI_ANNUAL",
  dayCount: "THIRTY_360_BOND_BASIS",
  maturityDate: { year: 2031, month: 6, day: 16 },
  redemption: 100,
  side: "LONG",
};

describe("bond wire codec — exact field names the engine decodes", () => {
  it("encodes the bond arm as { bond: { coupon_rate, coupon_frequency, day_count, maturity_date{y,m,d}, redemption, side } }", () => {
    // Byte-identical to the server encoder `rates_instrument_to_json` (bond arm) and
    // the round-trip oracle in `crates/celnet-server/src/ws/codec.rs`. The body is
    // FLAT except `maturity_date`, which nests a BrokenDate — the same nested-date
    // discipline as the OIS pillar tenor (a flat maturity would be rejected).
    expect(bondInstrumentToWire(BOND)).toEqual({
      bond: {
        coupon_rate: 0.06,
        coupon_frequency: 1,
        day_count: 2,
        maturity_date: { year: 2031, month: 6, day: 16 },
        redemption: 100,
        side: 0,
      },
    });
  });

  it("maps coupon_frequency to the PaymentFrequency proto tag (ANNUAL=0, SEMI_ANNUAL=1, QUARTERLY=2)", () => {
    const tag = (f: BondInstrument["couponFrequency"]): unknown =>
      (bondInstrumentToWire({ ...BOND, couponFrequency: f }).bond as Record<string, unknown>)[
        "coupon_frequency"
      ];
    expect(tag("ANNUAL")).toBe(0);
    expect(tag("SEMI_ANNUAL")).toBe(1);
    expect(tag("QUARTERLY")).toBe(2);
  });

  it("maps day_count to the AccrualBasis proto tag (ACT_360=0, ACT_365_FIXED=1, THIRTY_360_BOND_BASIS=2)", () => {
    const tag = (d: BondInstrument["dayCount"]): unknown =>
      (bondInstrumentToWire({ ...BOND, dayCount: d }).bond as Record<string, unknown>)["day_count"];
    expect(tag("ACT_360")).toBe(0);
    expect(tag("ACT_365_FIXED")).toBe(1);
    expect(tag("THIRTY_360_BOND_BASIS")).toBe(2);
  });

  it("maps LONG → side 0 (BUY, +PV) and SHORT → side 1 (SELL, −PV)", () => {
    const longSide = (bondInstrumentToWire({ ...BOND, side: "LONG" }).bond as Record<string, unknown>)["side"];
    const shortSide = (bondInstrumentToWire({ ...BOND, side: "SHORT" }).bond as Record<string, unknown>)["side"];
    expect(longSide).toBe(0);
    expect(shortSide).toBe(1);
  });

  it("carries a zero-coupon bond as coupon_rate 0 (a valid arm, not omitted)", () => {
    const zero = bondInstrumentToWire({ ...BOND, couponRate: 0 }).bond as Record<string, unknown>;
    expect(zero["coupon_rate"]).toBe(0);
  });

  it("decodes a bond rates_price_response.result (bond semantics) with an empty ladder", () => {
    // For a bond: pv = dirty price, par_rate = yield to maturity, pv01 = dv01 =
    // yield DV01, key_rate_ladder = [] (a closed-form yield-space risk). The shared
    // `ratesPricingResultFromWire` decoder accepts the empty ladder verbatim.
    const frame = {
      result: {
        pv: 108.75,
        par_rate: 0.0455,
        pv01: 0.0492,
        dv01: 0.0492,
        key_rate_ladder: [],
      },
    };
    expect(ratesPricingResultFromWire(frame)).toEqual({
      pv: 108.75,
      parRate: 0.0455,
      pv01: 0.0492,
      dv01: 0.0492,
      keyRateLadder: [],
    });
  });
});

describe("bond input shaping", () => {
  it("parses coupon frequency with the canonical names and desk aliases", () => {
    expect(parseBondCouponFrequency("ANNUAL")).toBe("ANNUAL");
    expect(parseBondCouponFrequency("ann")).toBe("ANNUAL");
    expect(parseBondCouponFrequency("SEMI_ANNUAL")).toBe("SEMI_ANNUAL");
    expect(parseBondCouponFrequency("semi")).toBe("SEMI_ANNUAL");
    expect(parseBondCouponFrequency("QUARTERLY")).toBe("QUARTERLY");
    expect(parseBondCouponFrequency("Q")).toBe("QUARTERLY");
    // Empty/absent ⇒ SEMI_ANNUAL (the standard cash-bond convention).
    expect(parseBondCouponFrequency(undefined)).toBe("SEMI_ANNUAL");
    expect(parseBondCouponFrequency("")).toBe("SEMI_ANNUAL");
    expect(() => parseBondCouponFrequency("MONTHLY")).toThrow(ShapingError);
  });

  it("parses day count with the canonical names and desk aliases; defaults to 30/360 Bond Basis", () => {
    expect(parseBondDayCount("ACT_360")).toBe("ACT_360");
    expect(parseBondDayCount("act365f")).toBe("ACT_365_FIXED");
    expect(parseBondDayCount("THIRTY_360_BOND_BASIS")).toBe("THIRTY_360_BOND_BASIS");
    expect(parseBondDayCount("30/360")).toBe("THIRTY_360_BOND_BASIS");
    expect(parseBondDayCount("bond")).toBe("THIRTY_360_BOND_BASIS");
    expect(parseBondDayCount(undefined)).toBe("THIRTY_360_BOND_BASIS");
    expect(() => parseBondDayCount("ACT_ACT")).toThrow(ShapingError);
  });

  it("parses side with the desk aliases; defaults to LONG and rejects TWO_WAY", () => {
    expect(parseBondSide("LONG")).toBe("LONG");
    expect(parseBondSide("buy")).toBe("LONG");
    expect(parseBondSide(undefined)).toBe("LONG");
    expect(parseBondSide("SHORT")).toBe("SHORT");
    expect(parseBondSide("sell")).toBe("SHORT");
    expect(() => parseBondSide("TWO_WAY")).toThrow(ShapingError);
  });

  it("shapes a bond instrument with the market defaults (SEMI_ANNUAL / 30-360 / face 100 / LONG)", () => {
    const bond = shapeBondInstrument({
      maturity: "2031-06-16",
      referenceDate: CURVE.referenceDate,
      couponRate: 0.06,
    });
    expect(bond).toEqual(BOND);
  });

  it("shapes explicit overrides (a short zero-coupon ACT/365F bond with a non-par face)", () => {
    const bond = shapeBondInstrument({
      maturity: "2028-12-31",
      referenceDate: CURVE.referenceDate,
      couponRate: 0,
      redemption: 1000,
      frequency: "ANNUAL",
      dayCount: "ACT_365_FIXED",
      side: "SHORT",
    });
    expect(bond).toEqual({
      couponRate: 0,
      couponFrequency: "ANNUAL",
      dayCount: "ACT_365_FIXED",
      maturityDate: { year: 2028, month: 12, day: 31 },
      redemption: 1000,
      side: "SHORT",
    });
  });

  it("accepts an Excel date serial for the maturity", () => {
    // A future Excel serial (epoch 1899-12-30): 48717 is 2033-05-18.
    const bond = shapeBondInstrument({
      maturity: 48717,
      referenceDate: CURVE.referenceDate,
      couponRate: 0.05,
    });
    expect(bond.maturityDate).toEqual({ year: 2033, month: 5, day: 18 });
  });

  it("rejects a maturity at or before the curve reference date", () => {
    expect(() =>
      shapeBondInstrument({
        maturity: "2025-06-16",
        referenceDate: CURVE.referenceDate,
        couponRate: 0.06,
      }),
    ).toThrow(/strictly after the curve reference date/);
    expect(() =>
      shapeBondInstrument({
        maturity: "2024-01-01",
        referenceDate: CURVE.referenceDate,
        couponRate: 0.06,
      }),
    ).toThrow(ShapingError);
  });

  it("rejects a non-finite / negative coupon rate and a non-positive redemption", () => {
    expect(() =>
      shapeBondInstrument({
        maturity: "2031-06-16",
        referenceDate: CURVE.referenceDate,
        couponRate: Number.NaN,
      }),
    ).toThrow(/finite/);
    expect(() =>
      shapeBondInstrument({
        maturity: "2031-06-16",
        referenceDate: CURVE.referenceDate,
        couponRate: -0.01,
      }),
    ).toThrow(/non-negative/);
    expect(() =>
      shapeBondInstrument({
        maturity: "2031-06-16",
        referenceDate: CURVE.referenceDate,
        couponRate: 0.06,
        redemption: 0,
      }),
    ).toThrow(/positive/);
  });
});

describe("bond spill layout", () => {
  it("lays out dirty_price / ytm / dv01 (the fields the wire carries for a bond)", () => {
    const spill = formatBondSpill({
      pv: 108.75,
      parRate: 0.0455,
      pv01: 0.0492,
      dv01: 0.0492,
      keyRateLadder: [],
    });
    expect(spill).toEqual([
      ["dirty_price", 108.75],
      ["ytm", 0.0455],
      ["dv01", 0.0492],
    ]);
    // Office.js custom functions require a rectangular 2-D return.
    for (const row of spill) expect(row.length).toBe(2);
  });
});

describe("Connection.priceRatesBond round-trip", () => {
  it("sends a `price_rates` frame carrying the bond arm and decodes the reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceRatesBond(CURVE, BOND);
    const sent = sock.sentOfType("price_rates")[0]!;
    expect(sent["curve_set"]).toEqual(ratesCurveSetToWire(CURVE));
    expect(sent["instrument"]).toEqual(bondInstrumentToWire(BOND));
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "rates_price_response",
      correlation_id: corr,
      result: {
        pv: 108.75,
        par_rate: 0.0455,
        pv01: 0.0492,
        dv01: 0.0492,
        key_rate_ladder: [],
      },
    });
    const result = await p;
    expect(result).toEqual({
      pv: 108.75,
      parRate: 0.0455,
      pv01: 0.0492,
      dv01: 0.0492,
      keyRateLadder: [],
    });
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceRatesBond(CURVE, BOND);
    const corr = sock.sentOfType("price_rates")[0]!["correlation_id"] as number;
    sock.deliver({
      type: "error",
      correlation_id: corr,
      message: "failed_precondition: bond maturity must be after settlement",
    });
    await expect(p).rejects.toThrow(/failed_precondition/);
  });
});
