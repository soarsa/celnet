// Wave 1 — Excel client parity for the variance swap, volatility swap and
// arithmetic Asian option the Rust slice put on the ONE wire contract. These
// tests prove the add-in shapes each product into the EXACT wire shape the
// server's `crates/celnet-server/src/ws/codec.rs` decodes (the product oneof keys
// `variance_swap` / `volatility_swap` / `asian_option` carrying field numbers
// 13 / 14 / 15), and that the VARSWAP / VOLSWAP / ASIAN families (the polymorphic
// CELNET.INSTRUMENT + verb path) decode
// the server's reply into the right spill geometry. No pricing math lives in the
// add-in — the numbers are the server's libm-core values, so a cell is
// bit-identical to the SDK/CLI (whose own gate tests reconcile the server pricer
// to the celnet-exotics closed forms as the independent oracle).

import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  formatPremiumSpill,
  formatVarSwapSpill,
  formatVolSwapSpill,
  parseAsianMethod,
  parseAveragingStyle,
  shapeAsianOption,
  shapeVarianceSwap,
  shapeVolatilitySwap,
  subscriptionKey,
} from "../src/functions/shaping";
import { instrumentToWire } from "../src/contract/wsCodec";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { Greeks, Instrument } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

describe("swap / Asian argument parsing", () => {
  it("parses averaging style with defaults and rejects garbage", () => {
    expect(parseAveragingStyle(undefined)).toBe("DISCRETE");
    expect(parseAveragingStyle("")).toBe("DISCRETE");
    expect(parseAveragingStyle("discrete")).toBe("DISCRETE");
    expect(parseAveragingStyle("D")).toBe("DISCRETE");
    expect(parseAveragingStyle("continuous")).toBe("CONTINUOUS");
    expect(parseAveragingStyle("cont")).toBe("CONTINUOUS");
    expect(() => parseAveragingStyle("weekly")).toThrow(ShapingError);
  });

  it("parses Asian method with defaults and rejects garbage", () => {
    expect(parseAsianMethod(undefined)).toBe("CURRAN");
    expect(parseAsianMethod("curran")).toBe("CURRAN");
    expect(parseAsianMethod("TW")).toBe("TURNBULL_WAKEMAN");
    expect(parseAsianMethod("turnbull_wakeman")).toBe("TURNBULL_WAKEMAN");
    expect(() => parseAsianMethod("monte-carlo")).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// instrument shaping → wire encoding (the contract field-number contract)
// ---------------------------------------------------------------------------

describe("variance swap shaping + wire encoding", () => {
  it("shapes a TWO_WAY variance swap with a fresh (zero) fair-strike request", () => {
    const i = shapeVarianceSwap({ pair: "EURUSD", tenor: "1Y", notional: 1e6 });
    expect(i.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(i.side).toBe("TWO_WAY");
    expect(i.quantity).toEqual({ notional: 1e6, baseCcy: true });
    expect(i.product).toEqual({ kind: "varianceSwap", varianceSwap: { strikeVol: 0 } });
  });

  it("pins a fixed strike vol when supplied", () => {
    const i = shapeVarianceSwap({ pair: "EURUSD", tenor: "1Y", notional: 1e6, strikeVol: 0.11 });
    expect(i.product).toEqual({ kind: "varianceSwap", varianceSwap: { strikeVol: 0.11 } });
  });

  it("encodes the oneof under `variance_swap` (proto field 13) with snake_case body", () => {
    const i = shapeVarianceSwap({ pair: "EURUSD", tenor: "1Y", notional: 1e6, strikeVol: 0.11 });
    const wire = instrumentToWire(i);
    expect(wire["variance_swap"]).toEqual({ strike_vol: 0.11 });
    // Exactly one product arm is present (oneof discipline).
    expect("vanilla" in wire).toBe(false);
    expect("volatility_swap" in wire).toBe(false);
    expect("asian_option" in wire).toBe(false);
  });

  it("rejects a non-positive notional and a too-large strike vol", () => {
    expect(() => shapeVarianceSwap({ pair: "EURUSD", tenor: "1Y", notional: 0 })).toThrow(ShapingError);
    expect(() => shapeVarianceSwap({ pair: "EURUSD", tenor: "1Y", notional: 1e6, strikeVol: 9 })).toThrow(
      ShapingError,
    );
  });
});

describe("volatility swap shaping + wire encoding", () => {
  it("encodes the oneof under `volatility_swap` (proto field 14)", () => {
    const i = shapeVolatilitySwap({ pair: "GBPUSD", tenor: "6M", notional: 2.5e6 });
    expect(i.product).toEqual({ kind: "volatilitySwap", volatilitySwap: { strikeVol: 0 } });
    const wire = instrumentToWire(i);
    expect(wire["volatility_swap"]).toEqual({ strike_vol: 0 });
    expect("variance_swap" in wire).toBe(false);
  });
});

describe("Asian option shaping + wire encoding", () => {
  it("shapes a DISCRETE Curran Asian with the full payoff spec", () => {
    const i = shapeAsianOption({
      pair: "USDJPY",
      tenor: "1Y",
      strike: 152.5,
      callPut: "C",
      notional: 1e6,
      averaging: "DISCRETE",
      observations: 12,
      method: "CURRAN",
    });
    expect(i.product).toEqual({
      kind: "asianOption",
      asianOption: {
        optionType: "CALL",
        strike: 152.5,
        averaging: "DISCRETE",
        observations: 12,
        method: "CURRAN",
        elapsedAvg: 0,
        elapsedWeight: 0,
      },
    });
  });

  it("encodes the oneof under `asian_option` (proto field 15) with numeric enums + snake_case", () => {
    const i = shapeAsianOption({
      pair: "USDJPY",
      tenor: "1Y",
      strike: 152.5,
      callPut: "P",
      notional: 1e6,
      averaging: "CONTINUOUS",
      method: "TW",
      elapsedAvg: 1.095,
      elapsedWeight: 0.25,
    });
    const wire = instrumentToWire(i);
    expect(wire["asian_option"]).toEqual({
      option_type: 1, // PUT
      strike: 152.5,
      averaging: 1, // AVERAGING_STYLE_CONTINUOUS
      observations: 0, // ignored for the continuous style
      method: 1, // ASIAN_METHOD_TURNBULL_WAKEMAN
      elapsed_avg: 1.095,
      elapsed_weight: 0.25,
    });
  });

  it("requires observations >= 1 for DISCRETE averaging", () => {
    expect(() =>
      shapeAsianOption({ pair: "EURUSD", tenor: "1Y", strike: 1.1, callPut: "C", notional: 1e6 }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeAsianOption({
        pair: "EURUSD",
        tenor: "1Y",
        strike: 1.1,
        callPut: "C",
        notional: 1e6,
        observations: 0,
      }),
    ).toThrow(ShapingError);
  });

  it("rejects a delta-quoted strike (an Asian has no delta strike convention)", () => {
    expect(() =>
      shapeAsianOption({
        pair: "EURUSD",
        tenor: "1Y",
        strike: "25dC",
        callPut: "C",
        notional: 1e6,
        observations: 4,
      }),
    ).toThrow(ShapingError);
  });

  it("rejects an out-of-range elapsed weight", () => {
    expect(() =>
      shapeAsianOption({
        pair: "EURUSD",
        tenor: "1Y",
        strike: 1.1,
        callPut: "C",
        notional: 1e6,
        observations: 4,
        elapsedWeight: 1,
      }),
    ).toThrow(ShapingError);
  });
});

describe("subscription coalescing key over the new products", () => {
  it("distinguishes a variance swap from a vol swap from an Asian", () => {
    const v = shapeVarianceSwap({ pair: "EURUSD", tenor: "1Y", notional: 1e6 });
    const vol = shapeVolatilitySwap({ pair: "EURUSD", tenor: "1Y", notional: 1e6 });
    const asn = shapeAsianOption({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "C",
      notional: 1e6,
      observations: 12,
    });
    const keys = new Set([
      subscriptionKey(v, DEFAULT_CONVENTIONS),
      subscriptionKey(vol, DEFAULT_CONVENTIONS),
      subscriptionKey(asn, DEFAULT_CONVENTIONS),
    ]);
    expect(keys.size).toBe(3);
  });

  it("distinguishes two Asians that differ only in seasoning", () => {
    const fresh = shapeAsianOption({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "C",
      notional: 1e6,
      observations: 12,
    });
    const seasoned = shapeAsianOption({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "C",
      notional: 1e6,
      observations: 12,
      elapsedAvg: 1.1,
      elapsedWeight: 0.25,
    });
    expect(subscriptionKey(fresh, DEFAULT_CONVENTIONS)).not.toBe(
      subscriptionKey(seasoned, DEFAULT_CONVENTIONS),
    );
  });
});

// ---------------------------------------------------------------------------
// spill formatting
// ---------------------------------------------------------------------------

const SAMPLE_GREEKS: Greeks = {
  price: 0.0421,
  deltaSpot: 0.51,
  deltaForward: 0.5,
  gamma: 2.1,
  vega: 0.0039,
  theta: -0.00012,
  rhoDom: 0.004,
  rhoFor: -0.005,
  vanna: 0.001,
  volga: 0.002,
  charm: 0.0001,
  speed: -0.3,
  zomma: 0.05,
  color: -0.0002,
};

describe("swap / Asian spill formatting", () => {
  it("VARSWAP spill shows fair variance and its sqrt, plus a convention footer", () => {
    const m = formatVarSwapSpill({
      fairVariance: 0.0121, // K_var = 0.11^2
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 42n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["fair_variance", 0.0121]);
    expect(m[1]?.[0]).toBe("fair_vol");
    expect(m[1]?.[1]).toBeCloseTo(0.11, 12); // sqrt(0.0121)
    expect(String(m[2]?.[0])).toContain("surface v42");
    expect(String(m[2]?.[0])).toContain("SPOT_UNADJUSTED");
  });

  it("VOLSWAP spill shows the convexity-adjusted fair vol + footer", () => {
    const m = formatVolSwapSpill({
      fairVol: 0.1085,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: undefined,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["fair_vol", 0.1085]);
    expect(String(m[1]?.[0])).toContain("surface live");
  });

  it("ASIAN spill leads with the premium, then the 13 risk Greeks, then a footer", () => {
    const m = formatPremiumSpill({
      premium: SAMPLE_GREEKS.price,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 7n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.0421]);
    // 1 premium + 13 Greeks + 1 footer = 15 rows.
    expect(m.length).toBe(15);
    expect(m[1]).toEqual(["delta_spot", 0.51]);
    expect(m[13]).toEqual(["color", -0.0002]);
    expect(String(m[14]?.[0])).toContain("surface v7");
  });
});

// ---------------------------------------------------------------------------
// end-to-end through the function + the WS transport (FakeSocket harness)
// ---------------------------------------------------------------------------

/** A controllable in-memory socket (mirrors test/connection.test.ts). */
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

function makeWiredConnection(): { conn: Connection; sock: FakeSocket } {
  const sock = new FakeSocket();
  const conn = new Connection({ url: "ws://test", factory: () => sock, requestTimeoutMs: 5000 });
  sock.open();
  return { conn, sock };
}

/** Reply to the (single) outstanding request_quote with a server-shaped quote. */
function replyQuote(sock: FakeSocket, body: Record<string, unknown>): void {
  const sent = sock.sentOfType("request_quote").at(-1)!;
  sock.deliver({
    type: "quote",
    correlation_id: sent["correlation_id"],
    quote_id: 1,
    idempotency_key: "k",
    price: { bid: 0, offer: 0 },
    greeks: {},
    conventions: {},
    resolved_strike: 0,
    epoch_nanos: 0,
    valid_until_nanos: 0,
    ...body,
  });
}

// These drive the EXACT request_quote → quote round-trip the VARSWAP /
// VOLSWAP / ASIAN bodies compose (shape → conn.requestQuote → format), over the
// real Connection + WS codec against a FakeSocket. This proves the new oneof
// reaches the wire with the right field key/body and the server reply decodes
// into the right spill — without the workbook-singleton browser-WebSocket path
// (the same composition the function bodies run, mirroring the SDK's e2e shape).

describe("variance-swap end-to-end over the WS mirror", () => {
  it("sends a variance_swap oneof (field 13) and renders the fair variance + fair vol", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeVarianceSwap({ pair: "EURUSD", tenor: "1Y", notional: 1e6 });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "varswap");
    const sent = sock.sentOfType("request_quote").at(-1)!;
    const wireInstr = sent["instrument"] as Record<string, unknown>;
    expect(wireInstr["variance_swap"]).toEqual({ strike_vol: 0 });
    // Server returns K_var = 0.0121 on resolved_strike (and greeks.price).
    replyQuote(sock, { resolved_strike: 0.0121, greeks: { price: 0.0121 } });
    const quote = await p;
    const m = formatVarSwapSpill({
      fairVariance: quote.resolvedStrike,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["fair_variance", 0.0121]);
    expect(m[1]?.[1]).toBeCloseTo(0.11, 12);
  });
});

describe("volatility-swap end-to-end over the WS mirror", () => {
  it("sends a volatility_swap oneof (field 14) and renders the fair vol", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeVolatilitySwap({ pair: "GBPUSD", tenor: "6M", notional: 2e6 });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "volswap");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["volatility_swap"]).toEqual({ strike_vol: 0 });
    replyQuote(sock, { resolved_strike: 0.1085, greeks: { price: 0.1085 } });
    const quote = await p;
    const m = formatVolSwapSpill({
      fairVol: quote.resolvedStrike,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["fair_vol", 0.1085]);
  });
});

describe("Asian end-to-end over the WS mirror", () => {
  it("sends an asian_option oneof (field 15) and renders the discounted premium + Greeks", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeAsianOption({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "C",
      notional: 1e6,
      averaging: "DISCRETE",
      observations: 12,
      method: "CURRAN",
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "asian");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["asian_option"]).toMatchObject({
      option_type: 0,
      strike: 1.1,
      averaging: 0,
      observations: 12,
      method: 0,
    });
    replyQuote(sock, {
      greeks: { price: 0.0307, delta_spot: 0.48, vega: 0.0035 },
      resolved_strike: 1.1,
    });
    const quote = await p;
    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0307]);
    expect(m[1]).toEqual(["delta_spot", 0.48]);
  });
});
