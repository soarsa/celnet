// Fixed-income (rates) FRA slice — the `price_rates` add-in path carrying the
// `RatesInstrument.fra` oneof arm: shape the scalar FRA terms into the typed contract,
// encode to the wire EXACTLY as the server decodes it
// (`crates/celnet-server/src/ws/codec.rs` `fra_instrument_from_json` +
// `crates/celnet-proto/proto/celnet.proto` `FraInstrument` / `RatesPricingResult`),
// decode a `rates_price_response`, and lay out the priced FRA result as a spill. The
// add-in holds no FRA math: it is a thin client of the live `celnet-rates` engine over
// the one unversioned contract, byte-identical to the GUI's `ratesInstrumentUnionToWire`.
//
// Wire-shape oracle (verified against codec.rs): the `fra` body is ALL FLAT scalars +
// numeric enum tags (NO nested date — the accrual window is quoted as whole `start_months`
// / `end_months` from spot, the "3x6 FRA" convention, and the roll-adjusted dates are
// rebuilt server-side). Enums are raw integers: side (SIDE_BUY=0 = PAY_FIXED, SIDE_SELL=1
// = RECEIVE_FIXED); accrual_basis = AccrualBasis (ACT_360=0, ACT_365_FIXED=1,
// THIRTY_360_BOND_BASIS=2). A FRA result is the SHARED `RatesPricingResult`
// (pv/par_rate/pv01/dv01 + a pillar-shaped key-rate ladder).
import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { FraInstrument, RatesCurveSet } from "../src/contract/contract";
import {
  fraInstrumentToWire,
  ratesCurveSetToWire,
  ratesPricingResultFromWire,
} from "../src/contract/wsCodec";
import {
  ShapingError,
  formatRatesSpill,
  parseFraMonths,
  parseRatesAccrualBasis,
  shapeFraInstrument,
} from "../src/functions/shaping";

// --- in-memory socket (mirrors test/bond.test.ts / test/irs.test.ts) ---------
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

// A 3-pillar self-discounting USD-SOFR curve and the canonical 3x6 FRA (receive-fixed,
// ACT/360 accrual).
const CURVE: RatesCurveSet = {
  currency: "USD",
  referenceDate: { year: 2025, month: 6, day: 16 },
  pillars: [
    { tenorYears: 2, parRate: 0.0405 },
    { tenorYears: 5, parRate: 0.041 },
    { tenorYears: 10, parRate: 0.0418 },
  ],
};
const FRA: FraInstrument = {
  startMonths: 3,
  endMonths: 6,
  fixedRate: 0.043,
  notional: 10_000_000,
  direction: "RECEIVE_FIXED",
  accrualBasis: "ACT_360",
};

describe("FRA wire codec — exact field names the engine decodes", () => {
  it("encodes the fra arm as { fra: { start_months, end_months, fixed_rate, notional, side, accrual_basis } }", () => {
    // Byte-identical to the server encoder + round-trip oracle in codec.rs and the
    // GUI's `ratesInstrumentUnionToWire` `fra` arm. The window is FLAT whole-month
    // tenors (no nested date): the roll-adjusted dates are rebuilt server-side.
    expect(fraInstrumentToWire(FRA)).toEqual({
      fra: {
        start_months: 3,
        end_months: 6,
        fixed_rate: 0.043,
        notional: 10_000_000,
        side: 1,
        accrual_basis: 0,
      },
    });
  });

  it("keeps the fra body entirely flat — no nested date object (the month-window trap)", () => {
    const body = fraInstrumentToWire(FRA).fra as Record<string, unknown>;
    for (const v of Object.values(body)) {
      expect(typeof v).toBe("number");
    }
  });

  it("maps accrual_basis to the AccrualBasis proto tag (ACT_360=0, ACT_365_FIXED=1, THIRTY_360_BOND_BASIS=2)", () => {
    const tag = (b: FraInstrument["accrualBasis"]): unknown =>
      (fraInstrumentToWire({ ...FRA, accrualBasis: b }).fra as Record<string, unknown>)[
        "accrual_basis"
      ];
    expect(tag("ACT_360")).toBe(0);
    expect(tag("ACT_365_FIXED")).toBe(1);
    expect(tag("THIRTY_360_BOND_BASIS")).toBe(2);
  });

  it("maps PAY_FIXED → side 0 (BUY) and RECEIVE_FIXED → side 1 (SELL)", () => {
    const paySide = (fraInstrumentToWire({ ...FRA, direction: "PAY_FIXED" }).fra as Record<string, unknown>)["side"];
    const recvSide = (fraInstrumentToWire({ ...FRA, direction: "RECEIVE_FIXED" }).fra as Record<string, unknown>)["side"];
    expect(paySide).toBe(0);
    expect(recvSide).toBe(1);
  });

  it("carries the window as start_months / end_months (a 3x6 FRA is {3, 6})", () => {
    const nine = fraInstrumentToWire({ ...FRA, startMonths: 6, endMonths: 9 }).fra as Record<string, unknown>;
    expect(nine["start_months"]).toBe(6);
    expect(nine["end_months"]).toBe(9);
  });

  it("decodes a FRA rates_price_response.result (pillar-shaped key-rate ladder)", () => {
    const frame = {
      result: {
        pv: 5123.4,
        par_rate: 0.0428,
        pv01: 61.2,
        dv01: 60.9,
        key_rate_ladder: [40.1, 20.8, 0.0],
      },
    };
    expect(ratesPricingResultFromWire(frame)).toEqual({
      pv: 5123.4,
      parRate: 0.0428,
      pv01: 61.2,
      dv01: 60.9,
      keyRateLadder: [40.1, 20.8, 0.0],
    });
  });
});

describe("FRA input shaping", () => {
  it("parses window months from a number or an `NM` string; rejects garbage and negatives", () => {
    expect(parseFraMonths(3, "start")).toBe(3);
    expect(parseFraMonths("6M", "end")).toBe(6);
    expect(parseFraMonths("9", "end")).toBe(9);
    expect(() => parseFraMonths("3Q", "start")).toThrow(ShapingError);
    expect(() => parseFraMonths(-1, "start")).toThrow(ShapingError);
    expect(() => parseFraMonths(3.5, "start")).toThrow(ShapingError);
  });

  it("parses the accrual basis with a fallback; defaults to ACT_360 for a FRA", () => {
    expect(parseRatesAccrualBasis("ACT_360", "ACT_360")).toBe("ACT_360");
    expect(parseRatesAccrualBasis("act365f", "ACT_360")).toBe("ACT_365_FIXED");
    expect(parseRatesAccrualBasis("30/360", "ACT_360")).toBe("THIRTY_360_BOND_BASIS");
    expect(parseRatesAccrualBasis(undefined, "ACT_360")).toBe("ACT_360");
    expect(parseRatesAccrualBasis("", "ACT_360")).toBe("ACT_360");
    expect(() => parseRatesAccrualBasis("ACT_ACT", "ACT_360")).toThrow(ShapingError);
  });

  it("shapes a FRA with the market defaults (PAY_FIXED / ACT_360)", () => {
    const fra = shapeFraInstrument({
      startMonths: 3,
      endMonths: 6,
      fixedRate: 0.043,
      notional: 10_000_000,
    });
    expect(fra).toEqual({
      startMonths: 3,
      endMonths: 6,
      fixedRate: 0.043,
      notional: 10_000_000,
      direction: "PAY_FIXED",
      accrualBasis: "ACT_360",
    });
  });

  it("shapes explicit overrides (a receive-fixed 6x12 ACT/365F FRA, `NM` string window)", () => {
    const fra = shapeFraInstrument({
      startMonths: "6M",
      endMonths: "12M",
      fixedRate: 0.0455,
      notional: 50_000_000,
      side: "RECEIVE_FIXED",
      accrualBasis: "ACT_365_FIXED",
    });
    expect(fra).toEqual({
      startMonths: 6,
      endMonths: 12,
      fixedRate: 0.0455,
      notional: 50_000_000,
      direction: "RECEIVE_FIXED",
      accrualBasis: "ACT_365_FIXED",
    });
  });

  it("rejects an inverted/degenerate window, a non-finite rate, and a non-positive notional", () => {
    expect(() =>
      shapeFraInstrument({ startMonths: 6, endMonths: 3, fixedRate: 0.043, notional: 1e6 }),
    ).toThrow(/strictly after/);
    expect(() =>
      shapeFraInstrument({ startMonths: 6, endMonths: 6, fixedRate: 0.043, notional: 1e6 }),
    ).toThrow(/strictly after/);
    expect(() =>
      shapeFraInstrument({ startMonths: 3, endMonths: 6, fixedRate: Number.NaN, notional: 1e6 }),
    ).toThrow(/finite/);
    expect(() =>
      shapeFraInstrument({ startMonths: 3, endMonths: 6, fixedRate: 0.043, notional: 0 }),
    ).toThrow(/positive/);
  });
});

describe("FRA spill layout", () => {
  it("lays out pv / par_rate / pv01 / dv01 then the tenor-labelled key-rate ladder (reusing formatRatesSpill)", () => {
    const spill = formatRatesSpill(
      {
        pv: 5123.4,
        parRate: 0.0428,
        pv01: 61.2,
        dv01: 60.9,
        keyRateLadder: [40.1, 20.8, 0.0],
      },
      CURVE.pillars,
    );
    expect(spill).toEqual([
      ["pv", 5123.4],
      ["par_rate", 0.0428],
      ["pv01", 61.2],
      ["dv01", 60.9],
      ["kr_dv01[2Y]", 40.1],
      ["kr_dv01[5Y]", 20.8],
      ["kr_dv01[10Y]", 0.0],
    ]);
    for (const row of spill) expect(row.length).toBe(2);
  });
});

describe("Connection.priceRatesFra round-trip", () => {
  it("sends a `price_rates` frame carrying the fra arm and decodes the reply", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceRatesFra(CURVE, FRA);
    const sent = sock.sentOfType("price_rates")[0]!;
    expect(sent["curve_set"]).toEqual(ratesCurveSetToWire(CURVE));
    expect(sent["instrument"]).toEqual(fraInstrumentToWire(FRA));
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "rates_price_response",
      correlation_id: corr,
      result: {
        pv: 5123.4,
        par_rate: 0.0428,
        pv01: 61.2,
        dv01: 60.9,
        key_rate_ladder: [40.1, 20.8, 0.0],
      },
    });
    const result = await p;
    expect(result).toEqual({
      pv: 5123.4,
      parRate: 0.0428,
      pv01: 61.2,
      dv01: 60.9,
      keyRateLadder: [40.1, 20.8, 0.0],
    });
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.priceRatesFra(CURVE, FRA);
    const corr = sock.sentOfType("price_rates")[0]!["correlation_id"] as number;
    sock.deliver({
      type: "error",
      correlation_id: corr,
      message: "invalid_argument: FRA window end must be after start",
    });
    await expect(p).rejects.toThrow(/invalid_argument/);
  });
});
