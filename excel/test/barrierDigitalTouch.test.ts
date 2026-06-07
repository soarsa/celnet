// Wave 4 — Excel client parity for the single-barrier, double-barrier, digital and
// touch products that were ALREADY on the ONE wire contract + priced server-side,
// but unreachable from any Excel cell. These tests prove the add-in shapes each
// product into the EXACT wire shape the server's
// `crates/celnet-server/src/ws/codec.rs` decodes — the product oneof keys
// `single_barrier` / `double_barrier` / `digital` / `touch` (field numbers
// 9 / 10 / 11 / 12) with the same nested vanilla body, numeric enums and snake_case
// fields the Rust decoder reads (`single_barrier_from_json` /
// `double_barrier_from_json` / `digital_from_json` / `touch_from_json`) — and that
// the CELNET.BARRIER / DIGITAL / TOUCH functions decode the server's reply into the
// right spill geometry. No pricing math lives in the add-in — the numbers are the
// server's libm-core values (the SDK/CLI + celnet-golden/celnet-exotics gate the
// server pricer against the independent oracle), so a cell is bit-identical to the
// SDK/CLI. The four products are priced exactly (closed-form / PDE) — the server
// stamps `std_error: None` — so no spill carries a Monte-Carlo std-error row.

import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  formatExoticPremiumSpill,
  parseBarrierKind,
  parseBarrierSide,
  parseDigitalStyle,
  parseMonitoringStyle,
  parseTouchKind,
  shapeBarrier,
  shapeDigital,
  shapeTouch,
  subscriptionKey,
} from "../src/functions/shaping";
import { instrumentToWire } from "../src/contract/wsCodec";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { Greeks, Instrument } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// argument parsing — the trader-facing selectors → contract vocabulary
// ---------------------------------------------------------------------------

describe("barrier / digital / touch argument parsing", () => {
  it("parses barrier kind with defaults and rejects garbage", () => {
    expect(parseBarrierKind(undefined)).toBe("KNOCK_IN");
    expect(parseBarrierKind("")).toBe("KNOCK_IN");
    expect(parseBarrierKind("ki")).toBe("KNOCK_IN");
    expect(parseBarrierKind("knock_in")).toBe("KNOCK_IN");
    expect(parseBarrierKind("KO")).toBe("KNOCK_OUT");
    expect(parseBarrierKind("knock-out")).toBe("KNOCK_OUT");
    expect(() => parseBarrierKind("rebate")).toThrow(ShapingError);
  });

  it("parses barrier side with defaults and rejects garbage", () => {
    expect(parseBarrierSide(undefined)).toBe("UP");
    expect(parseBarrierSide("up")).toBe("UP");
    expect(parseBarrierSide("DOWN")).toBe("DOWN");
    expect(parseBarrierSide("dn")).toBe("DOWN");
    expect(() => parseBarrierSide("sideways")).toThrow(ShapingError);
  });

  it("parses monitoring style with the continuous default (OTC standard)", () => {
    expect(parseMonitoringStyle(undefined)).toBe("CONTINUOUS");
    expect(parseMonitoringStyle("cont")).toBe("CONTINUOUS");
    expect(parseMonitoringStyle("DISCRETE")).toBe("DISCRETE");
    expect(parseMonitoringStyle("disc")).toBe("DISCRETE");
    expect(() => parseMonitoringStyle("weekly")).toThrow(ShapingError);
  });

  it("parses the touch family with defaults and rejects garbage", () => {
    expect(parseTouchKind(undefined)).toBe("ONE_TOUCH");
    expect(parseTouchKind("ot")).toBe("ONE_TOUCH");
    expect(parseTouchKind("NT")).toBe("NO_TOUCH");
    expect(parseTouchKind("dnt")).toBe("DOUBLE_NO_TOUCH");
    expect(parseTouchKind("DOT")).toBe("DOUBLE_ONE_TOUCH");
    expect(parseTouchKind("double_one_touch")).toBe("DOUBLE_ONE_TOUCH");
    expect(() => parseTouchKind("triple")).toThrow(ShapingError);
  });

  it("parses digital style with defaults and rejects garbage", () => {
    expect(parseDigitalStyle(undefined)).toBe("CASH_OR_NOTHING");
    expect(parseDigitalStyle("cash")).toBe("CASH_OR_NOTHING");
    expect(parseDigitalStyle("ASSET")).toBe("ASSET_OR_NOTHING");
    expect(() => parseDigitalStyle("digital")).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// instrument shaping → wire encoding (the contract field-number/key contract)
// ---------------------------------------------------------------------------

describe("single-barrier shaping + wire encoding", () => {
  it("shapes a TWO_WAY single barrier with the vanilla payoff and defaults", () => {
    const i = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.25,
    });
    expect(i.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(i.side).toBe("TWO_WAY");
    expect(i.quantity).toEqual({ notional: 1e6, baseCcy: true });
    expect(i.product).toEqual({
      kind: "singleBarrier",
      singleBarrier: {
        vanilla: { optionType: "CALL", strike: { kind: "strike", strike: 1.1 } },
        kind: "KNOCK_IN",
        side: "UP",
        barrier: 1.25,
        rebate: 0,
        monitoring: "CONTINUOUS",
      },
    });
  });

  it("encodes the oneof under `single_barrier` (proto field 9) with a nested vanilla, numeric enums + snake_case", () => {
    const i = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "P",
      notional: 1e6,
      barrier: 1.0,
      kind: "KNOCK_OUT",
      side: "DOWN",
      rebate: 0.002,
      monitoring: "DISCRETE",
    });
    const wire = instrumentToWire(i);
    expect(wire["single_barrier"]).toEqual({
      vanilla: { option_type: 1, strike: { strike: 1.1 } }, // PUT
      kind: 1, // BARRIER_KIND_KNOCK_OUT
      side: 1, // BARRIER_SIDE_DOWN
      barrier: 1.0,
      rebate: 0.002,
      monitoring: 1, // MONITORING_STYLE_DISCRETE
    });
    // Exactly one product arm is present (oneof discipline).
    expect("vanilla" in wire).toBe(false);
    expect("double_barrier" in wire).toBe(false);
    expect("digital" in wire).toBe(false);
    expect("touch" in wire).toBe(false);
  });

  it("encodes a delta-quoted barrier strike under the nested vanilla", () => {
    const i = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: "25dP",
      callPut: "P",
      notional: 1e6,
      barrier: 1.0,
      side: "DOWN",
    });
    const sb = instrumentToWire(i)["single_barrier"] as Record<string, unknown>;
    expect(sb["vanilla"]).toEqual({ option_type: 1, strike: { delta: -0.25 } });
  });

  it("rejects a non-positive notional and a non-positive barrier", () => {
    expect(() =>
      shapeBarrier({ pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.1, callPut: "C", notional: 0, barrier: 1.2 }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeBarrier({ pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.1, callPut: "C", notional: 1e6, barrier: 0 }),
    ).toThrow(ShapingError);
  });
});

describe("double-barrier shaping + wire encoding", () => {
  it("selects the DOUBLE product when an upper barrier is supplied (barrier = lower)", () => {
    const i = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.0, // lower
      upperBarrier: 1.25,
      kind: "KNOCK_OUT",
    });
    expect(i.product).toEqual({
      kind: "doubleBarrier",
      doubleBarrier: {
        vanilla: { optionType: "CALL", strike: { kind: "strike", strike: 1.1 } },
        kind: "KNOCK_OUT",
        lowerBarrier: 1.0,
        upperBarrier: 1.25,
        rebate: 0,
        monitoring: "CONTINUOUS",
      },
    });
    const wire = instrumentToWire(i);
    expect(wire["double_barrier"]).toEqual({
      vanilla: { option_type: 0, strike: { strike: 1.1 } }, // CALL
      kind: 1, // KNOCK_OUT
      lower_barrier: 1.0,
      upper_barrier: 1.25,
      rebate: 0,
      monitoring: 0, // CONTINUOUS
    });
    expect("single_barrier" in wire).toBe(false);
  });

  it("rejects a side on a double barrier (it brackets spot, no single side)", () => {
    expect(() =>
      shapeBarrier({
        pair: "EURUSD",
        tenor: "1Y",
        strikeOrDelta: 1.1,
        callPut: "C",
        notional: 1e6,
        barrier: 1.0,
        upperBarrier: 1.25,
        side: "UP",
      }),
    ).toThrow(ShapingError);
  });

  it("rejects an upper barrier at or below the lower", () => {
    expect(() =>
      shapeBarrier({
        pair: "EURUSD",
        tenor: "1Y",
        strikeOrDelta: 1.1,
        callPut: "C",
        notional: 1e6,
        barrier: 1.25,
        upperBarrier: 1.0,
      }),
    ).toThrow(ShapingError);
  });
});

describe("digital shaping + wire encoding", () => {
  it("encodes the oneof under `digital` (proto field 11) with numeric enums + snake_case", () => {
    const i = shapeDigital({
      pair: "EURUSD",
      tenor: "6M",
      strike: 1.15,
      callPut: "C",
      notional: 1e6,
      style: "ASSET",
      payout: 1000,
    });
    expect(i.product).toEqual({
      kind: "digital",
      digital: { optionType: "CALL", strike: 1.15, style: "ASSET_OR_NOTHING", payout: 1000 },
    });
    const wire = instrumentToWire(i);
    expect(wire["digital"]).toEqual({
      option_type: 0, // CALL
      strike: 1.15,
      style: 1, // ASSET_OR_NOTHING
      payout: 1000,
    });
    expect("single_barrier" in wire).toBe(false);
    expect("touch" in wire).toBe(false);
  });

  it("defaults to cash-or-nothing with a zero payout when omitted", () => {
    const i = shapeDigital({ pair: "EURUSD", tenor: "1Y", strike: 1.1, callPut: "P", notional: 1e6 });
    expect(instrumentToWire(i)["digital"]).toEqual({
      option_type: 1, // PUT
      strike: 1.1,
      style: 0, // CASH_OR_NOTHING
      payout: 0,
    });
  });

  it("rejects a delta-quoted strike (a digital strikes at a level)", () => {
    expect(() =>
      shapeDigital({ pair: "EURUSD", tenor: "1Y", strike: "25dC", callPut: "C", notional: 1e6 }),
    ).toThrow(ShapingError);
  });
});

describe("touch shaping + wire encoding", () => {
  it("encodes a one-touch under `touch` (proto field 12) with the sole lower barrier", () => {
    const i = shapeTouch({
      pair: "EURUSD",
      tenor: "3M",
      kind: "OT",
      barrier: 1.2,
      notional: 1e6,
      rebate: 1.0,
    });
    expect(i.product).toEqual({
      kind: "touch",
      touch: {
        kind: "ONE_TOUCH",
        lowerBarrier: 1.2,
        upperBarrier: 0,
        rebate: 1.0,
        monitoring: "CONTINUOUS",
      },
    });
    const wire = instrumentToWire(i);
    expect(wire["touch"]).toEqual({
      kind: 0, // ONE_TOUCH
      lower_barrier: 1.2,
      upper_barrier: 0,
      rebate: 1.0,
      monitoring: 0, // CONTINUOUS
    });
    expect("digital" in wire).toBe(false);
  });

  it("encodes a double-no-touch with both barriers + numeric kind", () => {
    const i = shapeTouch({
      pair: "EURUSD",
      tenor: "1Y",
      kind: "DNT",
      barrier: 1.0,
      upperBarrier: 1.25,
      notional: 1e6,
      rebate: 1.0,
      monitoring: "DISCRETE",
    });
    expect(instrumentToWire(i)["touch"]).toEqual({
      kind: 2, // DOUBLE_NO_TOUCH
      lower_barrier: 1.0,
      upper_barrier: 1.25,
      rebate: 1.0,
      monitoring: 1, // DISCRETE
    });
  });

  it("requires an upper barrier for a double structure (DNT/DOT)", () => {
    expect(() =>
      shapeTouch({ pair: "EURUSD", tenor: "1Y", kind: "DNT", barrier: 1.0, notional: 1e6 }),
    ).toThrow(ShapingError);
  });

  it("rejects an upper barrier on a single touch (OT/NT)", () => {
    expect(() =>
      shapeTouch({
        pair: "EURUSD",
        tenor: "1Y",
        kind: "NT",
        barrier: 1.0,
        upperBarrier: 1.25,
        notional: 1e6,
      }),
    ).toThrow(ShapingError);
  });
});

describe("subscription coalescing key over the new products", () => {
  it("distinguishes single barrier / double barrier / digital / touch", () => {
    const sb = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.25,
    });
    const db = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.0,
      upperBarrier: 1.25,
    });
    const dig = shapeDigital({ pair: "EURUSD", tenor: "1Y", strike: 1.1, callPut: "C", notional: 1e6 });
    const tch = shapeTouch({ pair: "EURUSD", tenor: "1Y", kind: "OT", barrier: 1.2, notional: 1e6 });
    const keys = new Set([
      subscriptionKey(sb, DEFAULT_CONVENTIONS),
      subscriptionKey(db, DEFAULT_CONVENTIONS),
      subscriptionKey(dig, DEFAULT_CONVENTIONS),
      subscriptionKey(tch, DEFAULT_CONVENTIONS),
    ]);
    expect(keys.size).toBe(4);
  });

  it("distinguishes two single barriers that differ only in knock direction", () => {
    const ki = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.25,
      kind: "KNOCK_IN",
    });
    const ko = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.25,
      kind: "KNOCK_OUT",
    });
    expect(subscriptionKey(ki, DEFAULT_CONVENTIONS)).not.toBe(subscriptionKey(ko, DEFAULT_CONVENTIONS));
  });
});

// ---------------------------------------------------------------------------
// spill formatting — premium + 13 Greeks (no MC std-error: exact pricing)
// ---------------------------------------------------------------------------

const SAMPLE_GREEKS: Greeks = {
  price: 0.0123,
  deltaSpot: 0.41,
  deltaForward: 0.4,
  gamma: 1.7,
  vega: 0.0031,
  theta: -0.00009,
  rhoDom: 0.003,
  rhoFor: -0.004,
  vanna: 0.0008,
  volga: 0.0015,
  charm: 0.00007,
  speed: -0.2,
  zomma: 0.04,
  color: -0.00015,
};

describe("barrier / digital / touch spill formatting", () => {
  it("leads with the premium, then the 13 risk Greeks, then a footer (no std-error row)", () => {
    const m = formatExoticPremiumSpill({
      premium: SAMPLE_GREEKS.price,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 9n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.0123]);
    // 1 premium + 13 Greeks + 1 footer = 15 rows; NO "std_error" row (exact pricing).
    expect(m.length).toBe(15);
    expect(m[1]).toEqual(["delta_spot", 0.41]);
    expect(m[13]).toEqual(["color", -0.00015]);
    expect(m.some((r) => r[0] === "std_error")).toBe(false);
    expect(String(m[14]?.[0])).toContain("surface v9");
    expect(String(m[14]?.[0])).toContain("SPOT_UNADJUSTED");
  });
});

// ---------------------------------------------------------------------------
// end-to-end through the function composition + the WS transport (FakeSocket)
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

/**
 * Reply to the (single) outstanding request_quote with a server-shaped quote. The
 * shape mirrors `crates/celnet-server/src/ws/codec.rs` `quote_to_json` — a closed-
 * form barrier/digital/touch reply carries NO `price_std_error` (the pricer stamps
 * `std_error: None`), so we never fabricate that field here.
 */
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

describe("single-barrier end-to-end over the WS mirror", () => {
  it("sends a single_barrier oneof (field 9) and renders the premium + Greeks", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.25,
      kind: "KNOCK_OUT",
      side: "UP",
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "barrier");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["single_barrier"]).toMatchObject({
      vanilla: { option_type: 0, strike: { strike: 1.1 } },
      kind: 1, // KNOCK_OUT
      side: 0, // UP
      barrier: 1.25,
      monitoring: 0, // CONTINUOUS
    });
    replyQuote(sock, {
      greeks: { price: 0.0089, delta_spot: 0.32, vega: 0.0021 },
      resolved_strike: 1.1,
    });
    const quote = await p;
    // A closed-form barrier carries no MC std-error.
    expect(quote.priceStdError).toBeUndefined();
    const m = formatExoticPremiumSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0089]);
    expect(m[1]).toEqual(["delta_spot", 0.32]);
    expect(m.some((r) => r[0] === "std_error")).toBe(false);
  });
});

describe("double-barrier end-to-end over the WS mirror", () => {
  it("sends a double_barrier oneof (field 10) with lower/upper barriers", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.0,
      upperBarrier: 1.25,
      kind: "KNOCK_OUT",
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "double-barrier");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["double_barrier"]).toMatchObject({
      vanilla: { option_type: 0, strike: { strike: 1.1 } },
      kind: 1,
      lower_barrier: 1.0,
      upper_barrier: 1.25,
    });
    expect("single_barrier" in wireInstr).toBe(false);
    replyQuote(sock, { greeks: { price: 0.0042 }, resolved_strike: 1.1 });
    const quote = await p;
    expect(quote.greeks.price).toBe(0.0042);
  });
});

describe("digital end-to-end over the WS mirror", () => {
  it("sends a digital oneof (field 11) and renders the premium", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeDigital({
      pair: "EURUSD",
      tenor: "6M",
      strike: 1.15,
      callPut: "C",
      notional: 1e6,
      style: "CASH",
      payout: 1,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "digital");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["digital"]).toEqual({ option_type: 0, strike: 1.15, style: 0, payout: 1 });
    replyQuote(sock, { greeks: { price: 0.41, delta_spot: 1.2 }, resolved_strike: 1.15 });
    const quote = await p;
    const m = formatExoticPremiumSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.41]);
    expect(m[1]).toEqual(["delta_spot", 1.2]);
  });
});

describe("touch end-to-end over the WS mirror", () => {
  it("sends a touch oneof (field 12) for a double-no-touch and renders the premium", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeTouch({
      pair: "EURUSD",
      tenor: "1Y",
      kind: "DNT",
      barrier: 1.0,
      upperBarrier: 1.25,
      notional: 1e6,
      rebate: 1.0,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "touch");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["touch"]).toEqual({
      kind: 2, // DOUBLE_NO_TOUCH
      lower_barrier: 1.0,
      upper_barrier: 1.25,
      rebate: 1.0,
      monitoring: 0,
    });
    replyQuote(sock, { greeks: { price: 0.55 }, resolved_strike: 0 });
    const quote = await p;
    expect(quote.priceStdError).toBeUndefined();
    const m = formatExoticPremiumSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.55]);
    expect(m.some((r) => r[0] === "std_error")).toBe(false);
  });
});
