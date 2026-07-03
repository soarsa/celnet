// Fixed-income RFQ slice — the `request_rates_quote` add-in path (=CELNET.RATESRFQ,
// the FI twin of =CELNET.RFQ): shape a curve range + instrument terms + taker side
// into the typed contract, encode to the wire EXACTLY as the server's descriptor-
// driven codec decodes it (`crates/celnet-server/src/ws/generated_codec.rs`
// `decode_rates_quote_request`, projecting the pure snake_case proto field names of
// `celnet.proto` message `RatesQuoteRequest`), decode a `rates_quote`
// (`encode_rates_quote`), and lay out the two-way + risk as a spill. The add-in
// holds no FI math: it is a thin client of the live `celnet-rates` engine over the
// one unversioned contract (a SINGLE two-way maker quote — no multi-dealer panel).
import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { RatesCurveSet, RatesInstrument } from "../src/contract/contract";
import {
  parseFrame,
  ratesCurveSetToWire,
  ratesInstrumentUnionToWire,
  ratesQuoteFromWire,
  ratesQuoteRequestToWire,
} from "../src/contract/wsCodec";
import {
  ShapingError,
  formatRatesRfqSpill,
  parseRatesRfqInstrumentKind,
  parseRatesRfqSide,
  shapeRatesRfqInstrument,
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

// A 3-pillar self-discounting USD-SOFR curve and a 5Y OIS to quote.
const CURVE: RatesCurveSet = {
  currency: "USD",
  referenceDate: { year: 2025, month: 6, day: 16 },
  pillars: [
    { tenorYears: 2, parRate: 0.0405 },
    { tenorYears: 5, parRate: 0.041 },
    { tenorYears: 10, parRate: 0.0418 },
  ],
};
const OIS_INSTRUMENT: RatesInstrument = {
  kind: "ois",
  ois: { tenorYears: 5, fixedRate: 0.041, notional: 10_000_000, direction: "PAY_FIXED" },
};

describe("rates RFQ request codec — exact field names the engine decodes", () => {
  it("encodes RatesQuoteRequest as { idempotency_key, curve_set, instrument, notional, side }", () => {
    // BYTE-IDENTICAL to the server's `decode_rates_quote_request` (the pure
    // snake_case proto projection): the curve_set + instrument reuse the byte-verified
    // unary rates encoders, `side` is the numeric wire `Side`, and `correlation_id`
    // is OMITTED (the transport routing id owns it under the unified correlation).
    const wire = ratesQuoteRequestToWire({
      idempotencyKey: "ratesrfq:USD:TWO_WAY:1e7",
      curveSet: CURVE,
      instrument: OIS_INSTRUMENT,
      notional: 10_000_000,
      side: "TWO_WAY",
    });
    expect(wire).toEqual({
      idempotency_key: "ratesrfq:USD:TWO_WAY:1e7",
      curve_set: ratesCurveSetToWire(CURVE),
      instrument: ratesInstrumentUnionToWire(OIS_INSTRUMENT),
      notional: 10_000_000,
      side: 2, // SIDE_TWO_WAY (BUY=0, SELL=1, TWO_WAY=2)
    });
    expect(wire).not.toHaveProperty("correlation_id");
  });

  it("maps the taker side to the numeric wire Side (BUY=0, SELL=1, TWO_WAY=2)", () => {
    const at = (side: "BUY" | "SELL" | "TWO_WAY") =>
      ratesQuoteRequestToWire({
        idempotencyKey: "k",
        curveSet: CURVE,
        instrument: OIS_INSTRUMENT,
        notional: 1e7,
        side,
      })["side"];
    expect(at("BUY")).toBe(0);
    expect(at("SELL")).toBe(1);
    expect(at("TWO_WAY")).toBe(2);
  });
});

describe("rates RFQ reply codec — decodes the two-way RatesQuote", () => {
  it("decodes a rates_quote frame into the typed RatesQuote (price + shared result)", () => {
    // A RAW JSON literal (never a JS object — a JS number would pre-round the
    // oversized id) parsed through the transport's exact 64-bit-preserving
    // `parseFrame`, so the `splitmix64` `quote_id` (> MAX_SAFE_INTEGER) and the
    // wall-clock nanos are recovered as EXACT bigints, exactly as the live socket
    // delivers them.
    const raw =
      '{"type":"rates_quote",' +
      '"quote_id":12345678901234567890,' +
      '"idempotency_key":"ratesrfq:USD:TWO_WAY:1e7",' +
      '"price":{"bid":0.0409,"offer":0.0411},' +
      '"result":{"pv":0.0,"par_rate":0.041,"pv01":4500.0,"dv01":4520.0,"key_rate_ladder":[900.0,2100.0,1520.0]},' +
      '"notional":10000000,' +
      '"epoch_nanos":1700000000000000000,' +
      '"valid_until_nanos":1700000030000000000,' +
      '"correlation_id":7}';
    const quote = ratesQuoteFromWire(parseFrame(raw) as Record<string, unknown>);
    expect(quote.quoteId).toBe(12_345_678_901_234_567_890n);
    expect(quote.idempotencyKey).toBe("ratesrfq:USD:TWO_WAY:1e7");
    expect(quote.price).toEqual({ bid: 0.0409, offer: 0.0411 });
    expect(quote.result).toEqual({
      pv: 0.0,
      parRate: 0.041,
      pv01: 4_500.0,
      dv01: 4_520.0,
      keyRateLadder: [900.0, 2100.0, 1520.0],
    });
    expect(quote.notional).toBe(10_000_000);
    expect(quote.epochNanos).toBe(1_700_000_000_000_000_000n);
    expect(quote.validUntilNanos).toBe(1_700_000_030_000_000_000n);
    expect(quote.correlationId).toBe(7n);
  });

  it("treats an absent/null correlation_id as undefined (presence-tracked)", () => {
    const base = {
      quote_id: 1,
      idempotency_key: "k",
      price: { bid: 0.0409, offer: 0.0411 },
      result: { pv: 0, par_rate: 0.041, pv01: 1, dv01: 1, key_rate_ladder: [] },
      notional: 1e7,
      epoch_nanos: 0,
      valid_until_nanos: 0,
    };
    expect(ratesQuoteFromWire(base).correlationId).toBeUndefined();
    expect(ratesQuoteFromWire({ ...base, correlation_id: null }).correlationId).toBeUndefined();
  });
});

describe("rates RFQ side + instrument shaping", () => {
  it("parses the taker side with desk aliases, defaulting an absent side to TWO_WAY", () => {
    expect(parseRatesRfqSide(undefined)).toBe("TWO_WAY");
    expect(parseRatesRfqSide("")).toBe("TWO_WAY");
    expect(parseRatesRfqSide("BUY")).toBe("BUY");
    expect(parseRatesRfqSide("pay")).toBe("BUY");
    expect(parseRatesRfqSide("PAY_FIXED")).toBe("BUY");
    expect(parseRatesRfqSide("long")).toBe("BUY");
    expect(parseRatesRfqSide("SELL")).toBe("SELL");
    expect(parseRatesRfqSide("receive")).toBe("SELL");
    expect(parseRatesRfqSide("RECEIVE_FIXED")).toBe("SELL");
    expect(parseRatesRfqSide("short")).toBe("SELL");
    expect(parseRatesRfqSide("TWO_WAY")).toBe("TWO_WAY");
    expect(parseRatesRfqSide("both")).toBe("TWO_WAY");
    expect(() => parseRatesRfqSide("mumble")).toThrow(ShapingError);
  });

  it("parses the instrument selector (OIS default / IRS), rejecting FRA/BOND + junk", () => {
    expect(parseRatesRfqInstrumentKind(undefined)).toBe("OIS");
    expect(parseRatesRfqInstrumentKind("")).toBe("OIS");
    expect(parseRatesRfqInstrumentKind("ois")).toBe("OIS");
    expect(parseRatesRfqInstrumentKind("IRS")).toBe("IRS");
    expect(parseRatesRfqInstrumentKind("swap")).toBe("IRS");
    expect(() => parseRatesRfqInstrumentKind("FRA")).toThrow(/CELNET\.FRA/);
    expect(() => parseRatesRfqInstrumentKind("BOND")).toThrow(/CELNET\.BOND/);
    expect(() => parseRatesRfqInstrumentKind("xyz")).toThrow(/expected OIS or IRS/);
  });

  it("shapes the OIS arm (default), deriving the arm direction from the taker side", () => {
    // BUY (pay fixed) → PAY_FIXED; SELL (receive fixed) → RECEIVE_FIXED; TWO_WAY →
    // PAY_FIXED (neutral — the server overrides the arm side with the RFQ side).
    const buy = shapeRatesRfqInstrument({ tenor: "5Y", fixedRate: 0.041, notional: 1e7, side: "BUY" });
    expect(buy).toEqual({
      kind: "ois",
      ois: { tenorYears: 5, fixedRate: 0.041, notional: 1e7, direction: "PAY_FIXED" },
    });
    const sell = shapeRatesRfqInstrument({ tenor: 5, fixedRate: 0.041, notional: 1e7, side: "SELL" });
    expect((sell as { ois: { direction: string } }).ois.direction).toBe("RECEIVE_FIXED");
    const twoWay = shapeRatesRfqInstrument({ tenor: 5, fixedRate: 0.041, notional: 1e7, side: "TWO_WAY" });
    expect((twoWay as { ois: { direction: string } }).ois.direction).toBe("PAY_FIXED");
  });

  it("shapes the IRS arm with market-default legs when instrument=IRS", () => {
    const irs = shapeRatesRfqInstrument({
      tenor: 5,
      fixedRate: 0.041,
      notional: 1e7,
      side: "BUY",
      instrument: "IRS",
    });
    expect(irs).toEqual({
      kind: "irs",
      irs: {
        tenorYears: 5,
        fixedRate: 0.041,
        notional: 1e7,
        direction: "PAY_FIXED",
        fixedFrequency: "SEMI_ANNUAL",
        fixedDayCount: "ACT_360",
        floatFrequency: "QUARTERLY",
        floatDayCount: "ACT_360",
      },
    });
  });

  it("rejects a non-positive notional and a non-finite fixed rate (via the arm shaper)", () => {
    expect(() =>
      shapeRatesRfqInstrument({ tenor: 5, fixedRate: 0.041, notional: -1, side: "BUY" }),
    ).toThrow(/positive/);
    expect(() =>
      shapeRatesRfqInstrument({ tenor: 5, fixedRate: Number.NaN, notional: 1e7, side: "BUY" }),
    ).toThrow(/finite/);
  });
});

describe("rates RFQ spill layout", () => {
  it("lays out bid/offer/mid, the RFQ metadata, then the taker-side risk block", () => {
    const bid = 0.0409;
    const offer = 0.0411;
    const mid = (bid + offer) / 2; // recompute with the SAME formula (float-exact)
    const spill = formatRatesRfqSpill({
      quoteId: 42n,
      idempotencyKey: "k",
      price: { bid, offer },
      result: { pv: 0, parRate: 0.041, pv01: 4_500, dv01: 4_520, keyRateLadder: [900, 2100, 1520] },
      notional: 10_000_000,
      epochNanos: 0n,
      // 2025-06-16T00:00:30Z in epoch nanos.
      validUntilNanos: 1_750_032_030_000_000_000n,
    });
    expect(spill.slice(0, 5)).toEqual([
      ["bid", bid],
      ["offer", offer],
      ["mid", mid], // (bid + offer) / 2 — the side-independent fair level
      ["notional", 10_000_000],
      ["quote_id", "42"], // 64-bit-exact as a string
    ]);
    // The fair-level cross-check: for a swap the two-way mid IS the par rate.
    expect(spill[2]![1] as number).toBeCloseTo(spill[7]![1] as number, 12);
    expect(spill.slice(6)).toEqual([
      ["pv", 0],
      ["par_rate", 0.041],
      ["pv01", 4_500],
      ["dv01", 4_520],
    ]);
    // The valid_until cell is an ISO-8601 instant (last-look deadline).
    expect(spill[5]![0]).toBe("valid_until");
    expect(String(spill[5]![1])).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    // Office.js custom functions require a rectangular 2-D return.
    expect(spill.length).toBe(10);
    for (const row of spill) expect(row.length).toBe(2);
  });
});

describe("Connection.requestRatesQuote round-trip", () => {
  it("sends a `request_rates_quote` frame with the exact wire body and decodes the two-way", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.requestRatesQuote(CURVE, OIS_INSTRUMENT, "TWO_WAY", 10_000_000, "ratesrfq:USD:TWO_WAY:1e7");
    const sent = sock.sentOfType("request_rates_quote")[0]!;
    // The body is byte-identical to the server-decoded RatesQuoteRequest (the
    // transport adds only `type` + the routing `correlation_id`).
    expect(sent["idempotency_key"]).toBe("ratesrfq:USD:TWO_WAY:1e7");
    expect(sent["curve_set"]).toEqual(ratesCurveSetToWire(CURVE));
    expect(sent["instrument"]).toEqual(ratesInstrumentUnionToWire(OIS_INSTRUMENT));
    expect(sent["notional"]).toBe(10_000_000);
    expect(sent["side"]).toBe(2);
    const corr = sent["correlation_id"] as number;
    expect(typeof corr).toBe("number");
    sock.deliver({
      type: "rates_quote",
      correlation_id: corr,
      quote_id: 999,
      idempotency_key: "ratesrfq:USD:TWO_WAY:1e7",
      price: { bid: 0.0409, offer: 0.0411 },
      result: {
        pv: 0,
        par_rate: 0.041,
        pv01: 4_500,
        dv01: 4_520,
        key_rate_ladder: [900, 2100, 1520],
      },
      notional: 10_000_000,
      epoch_nanos: 1_700_000_000_000_000_000,
      valid_until_nanos: 1_700_000_030_000_000_000,
    });
    const quote = await p;
    expect(quote.quoteId).toBe(999n);
    expect(quote.price).toEqual({ bid: 0.0409, offer: 0.0411 });
    expect(quote.result.parRate).toBe(0.041);
    expect(quote.notional).toBe(10_000_000);
  });

  it("rejects on a typed error frame correlated to the request", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.requestRatesQuote(CURVE, OIS_INSTRUMENT, "BUY", 10_000_000, "k");
    const corr = sock.sentOfType("request_rates_quote")[0]!["correlation_id"] as number;
    sock.deliver({
      type: "error",
      correlation_id: corr,
      message: "invalid_argument: `notional` must be a positive, finite RFQ size",
    });
    await expect(p).rejects.toThrow(/positive, finite RFQ size/);
  });
});
