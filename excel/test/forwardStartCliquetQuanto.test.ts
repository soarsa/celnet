// Wave 2 — Excel client parity for the forward-start vanilla, cliquet (ratchet)
// and quanto option the Rust slice put on the ONE wire contract. These tests
// prove the add-in shapes each product into the EXACT wire shape the server's
// `crates/celnet-server/src/ws/codec.rs` decodes (the product oneof keys
// `forward_start` / `cliquet` / `quanto` carrying field numbers 16 / 17 / 18),
// that the presence-tracked cliquet clamps + `mc_pairs` / `mc_seed` encode exactly
// like the proto `optional double` / the server's `opt_f64`, and that the
// CELNET.FORWARDSTART / CLIQUET / QUANTO functions decode the server's reply into
// the right spill geometry — including the HONEST Monte-Carlo std-error row that
// appears ONLY for a clamped (MC-priced) cliquet. No pricing math lives in the
// add-in; the numbers are the server's libm-core values, so a cell is
// bit-identical to the SDK/CLI (whose gate tests reconcile the server pricer to
// the celnet-exotics closed forms + celnet-golden as the independent oracle).

import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  cliquetIsMonteCarlo,
  formatCliquetSpill,
  formatForwardStartSpill,
  formatQuantoSpill,
  parseQuantoPayoff,
  shapeCliquet,
  shapeForwardStart,
  shapeQuanto,
  subscriptionKey,
} from "../src/functions/shaping";
import { instrumentToWire, quoteFromWire } from "../src/contract/wsCodec";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { Cliquet, Greeks, Instrument } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

describe("quanto payoff parsing", () => {
  it("parses the payoff selector with defaults and rejects garbage", () => {
    expect(parseQuantoPayoff(undefined)).toBe("VANILLA");
    expect(parseQuantoPayoff("")).toBe("VANILLA");
    expect(parseQuantoPayoff("vanilla")).toBe("VANILLA");
    expect(parseQuantoPayoff("V")).toBe("VANILLA");
    expect(parseQuantoPayoff("digital")).toBe("DIGITAL");
    expect(parseQuantoPayoff("DIG")).toBe("DIGITAL");
    expect(() => parseQuantoPayoff("barrier")).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// forward-start: shaping → wire encoding (the contract field-number contract)
// ---------------------------------------------------------------------------

describe("forward-start shaping + wire encoding", () => {
  it("shapes a TWO_WAY forward-start call with a proportional strike + reset", () => {
    const i = shapeForwardStart({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      reset: 0.25,
      notional: 1e6,
    });
    expect(i.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(i.side).toBe("TWO_WAY");
    expect(i.quantity).toEqual({ notional: 1e6, baseCcy: true });
    expect(i.product).toEqual({
      kind: "forwardStart",
      forwardStart: { optionType: "CALL", moneyness: 1.0, reset: 0.25 },
    });
  });

  it("encodes the oneof under `forward_start` (proto field 16) with numeric enum + snake_case", () => {
    const i = shapeForwardStart({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "P",
      moneyness: 1.05,
      reset: 0.5,
      notional: 1e6,
    });
    const wire = instrumentToWire(i);
    expect(wire["forward_start"]).toEqual({ option_type: 1, moneyness: 1.05, reset: 0.5 });
    // Exactly one product arm is present (oneof discipline).
    expect("vanilla" in wire).toBe(false);
    expect("cliquet" in wire).toBe(false);
    expect("quanto" in wire).toBe(false);
  });

  it("rejects a reset outside [0, expiry] and a non-positive moneyness", () => {
    expect(() =>
      shapeForwardStart({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        moneyness: 1.0,
        reset: 1.5, // beyond the 1Y expiry
        notional: 1e6,
      }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeForwardStart({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        moneyness: 0,
        reset: 0.25,
        notional: 1e6,
      }),
    ).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// cliquet: shaping → wire encoding (presence-tracked clamps + MC knobs)
// ---------------------------------------------------------------------------

describe("cliquet shaping + wire encoding", () => {
  it("shapes a PLAIN ratchet (no clamps) and encodes no clamp keys (closed form)", () => {
    const i = shapeCliquet({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      periods: 4,
      notional: 1e6,
    });
    expect(i.product.kind).toBe("cliquet");
    const c = (i.product as { kind: "cliquet"; cliquet: Cliquet }).cliquet;
    expect(c.optionType).toBe("CALL");
    expect(c.periods).toBe(4);
    expect(c.localFloor).toBeUndefined();
    expect(c.localCap).toBeUndefined();
    expect(c.mcSeed).toBe(0n);
    expect(cliquetIsMonteCarlo(c)).toBe(false);

    const wire = instrumentToWire(i);
    const body = wire["cliquet"] as Record<string, unknown>;
    expect(body).toEqual({
      option_type: 0,
      moneyness: 1.0,
      periods: 4,
      mc_pairs: 0,
      mc_seed: 0, // 64-bit field carried as a plain JSON number (like token/ids/nanos)
    });
    // Presence-tracked: an absent clamp is OMITTED, never sent as a 0.
    expect("local_floor" in body).toBe(false);
    expect("local_cap" in body).toBe(false);
    expect("global_floor" in body).toBe(false);
    expect("global_cap" in body).toBe(false);
  });

  it("shapes a CLAMPED cliquet (MC-priced) and encodes the supplied clamps + MC knobs", () => {
    const i = shapeCliquet({
      pair: "USDJPY",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      periods: 6,
      notional: 1e6,
      localFloor: 0.0,
      localCap: 0.03,
      mcPairs: 50000,
      mcSeed: 12345,
    });
    const c = (i.product as { kind: "cliquet"; cliquet: Cliquet }).cliquet;
    expect(cliquetIsMonteCarlo(c)).toBe(true);
    expect(c.localFloor).toBe(0.0);
    expect(c.localCap).toBe(0.03);
    expect(c.mcSeed).toBe(12345n);

    const wire = instrumentToWire(i);
    const body = wire["cliquet"] as Record<string, unknown>;
    expect(body).toEqual({
      option_type: 0,
      moneyness: 1.0,
      periods: 6,
      local_floor: 0.0, // present even though 0 (a real floor, distinct from absent)
      local_cap: 0.03,
      mc_pairs: 50000,
      mc_seed: 12345, // 64-bit field carried as a plain JSON number
    });
    expect("global_floor" in body).toBe(false);
    expect("global_cap" in body).toBe(false);
  });

  it("rejects periods < 1, an inverted clamp pair, and a negative MC seed", () => {
    expect(() =>
      shapeCliquet({ pair: "EURUSD", tenor: "1Y", callPut: "C", moneyness: 1.0, periods: 0, notional: 1e6 }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeCliquet({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        moneyness: 1.0,
        periods: 4,
        notional: 1e6,
        localFloor: 0.05,
        localCap: 0.02, // floor above cap
      }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeCliquet({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        moneyness: 1.0,
        periods: 4,
        notional: 1e6,
        localCap: 0.03,
        mcSeed: -1,
      }),
    ).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// quanto: shaping → wire encoding
// ---------------------------------------------------------------------------

describe("quanto shaping + wire encoding", () => {
  it("shapes a vanilla quanto with an absolute strike + drift inputs", () => {
    const i = shapeQuanto({
      pair: "USDJPY",
      tenor: "1Y",
      callPut: "C",
      strike: 152.5,
      notional: 1e6,
      conversionVol: 0.09,
      correlation: -0.3,
    });
    expect(i.product).toEqual({
      kind: "quanto",
      quanto: {
        payoff: "VANILLA",
        optionType: "CALL",
        strike: 152.5,
        conversionVol: 0.09,
        correlation: -0.3,
      },
    });
  });

  it("encodes the oneof under `quanto` (proto field 18) with numeric enums + snake_case", () => {
    const i = shapeQuanto({
      pair: "USDJPY",
      tenor: "1Y",
      callPut: "P",
      strike: 150,
      notional: 1e6,
      conversionVol: 0.09,
      correlation: -0.3,
      payoff: "DIGITAL",
    });
    const wire = instrumentToWire(i);
    expect(wire["quanto"]).toEqual({
      payoff: 1, // QUANTO_PAYOFF_DIGITAL
      option_type: 1, // PUT
      strike: 150,
      conversion_vol: 0.09,
      correlation: -0.3,
    });
    expect("forward_start" in wire).toBe(false);
  });

  it("rejects a delta-quoted strike, an out-of-range correlation and a negative conversion vol", () => {
    expect(() =>
      shapeQuanto({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        strike: "25dC",
        notional: 1e6,
        conversionVol: 0.09,
        correlation: 0.1,
      }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeQuanto({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        strike: 1.1,
        notional: 1e6,
        conversionVol: 0.09,
        correlation: 1.5,
      }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeQuanto({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        strike: 1.1,
        notional: 1e6,
        conversionVol: -0.01,
        correlation: 0.1,
      }),
    ).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// subscription coalescing key over the new products
// ---------------------------------------------------------------------------

describe("subscription coalescing key over the structured products", () => {
  it("distinguishes a forward-start from a cliquet from a quanto", () => {
    const fwd = shapeForwardStart({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      reset: 0.25,
      notional: 1e6,
    });
    const clq = shapeCliquet({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      periods: 4,
      notional: 1e6,
    });
    const qto = shapeQuanto({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      strike: 1.1,
      notional: 1e6,
      conversionVol: 0.09,
      correlation: -0.3,
    });
    const keys = new Set([
      subscriptionKey(fwd, DEFAULT_CONVENTIONS),
      subscriptionKey(clq, DEFAULT_CONVENTIONS),
      subscriptionKey(qto, DEFAULT_CONVENTIONS),
    ]);
    expect(keys.size).toBe(3);
  });

  it("distinguishes a plain ratchet from a clamped cliquet that differs only in a clamp", () => {
    const plain = shapeCliquet({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      periods: 4,
      notional: 1e6,
    });
    const clamped = shapeCliquet({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      periods: 4,
      notional: 1e6,
      localCap: 0.03,
    });
    expect(subscriptionKey(plain, DEFAULT_CONVENTIONS)).not.toBe(
      subscriptionKey(clamped, DEFAULT_CONVENTIONS),
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

describe("structured-product spill formatting", () => {
  it("FORWARDSTART spill leads with the premium, then the 13 Greeks, then a footer (no std-error)", () => {
    const m = formatForwardStartSpill({
      premium: SAMPLE_GREEKS.price,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 7n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.0421]);
    // 1 premium + 13 Greeks + 1 footer = 15 rows (closed form ⇒ no std-error).
    expect(m.length).toBe(15);
    expect(m[1]).toEqual(["delta_spot", 0.51]);
    expect(m[13]).toEqual(["color", -0.0002]);
    expect(String(m[14]?.[0])).toContain("surface v7");
  });

  it("CLIQUET spill OMITS the std-error row for a plain ratchet (exact closed form)", () => {
    const m = formatCliquetSpill({
      premium: 0.031,
      stdError: undefined, // plain ratchet ⇒ no MC noise
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 3n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.031]);
    // No std-error row ⇒ premium + 13 Greeks + footer = 15 rows.
    expect(m.length).toBe(15);
    expect(m[1]).toEqual(["delta_spot", 0.51]);
    expect(m.some((row) => row[0] === "std_error")).toBe(false);
  });

  it("CLIQUET spill SURFACES the MC std-error row for a clamped cliquet (honest precision)", () => {
    const m = formatCliquetSpill({
      premium: 0.0285,
      stdError: 1.7e-4, // MC standard error reported by the server
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 3n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.0285]);
    expect(m[1]).toEqual(["std_error", 1.7e-4]);
    // With the std-error row: premium + std_error + 13 Greeks + footer = 16 rows.
    expect(m.length).toBe(16);
    expect(m[2]).toEqual(["delta_spot", 0.51]);
    expect(String(m[15]?.[0])).toContain("surface v3");
  });

  it("QUANTO spill leads with the premium, then the 13 Greeks, then a footer", () => {
    const m = formatQuantoSpill({
      premium: SAMPLE_GREEKS.price,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: undefined,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.0421]);
    expect(m.length).toBe(15);
    expect(String(m[14]?.[0])).toContain("surface live");
  });
});

// ---------------------------------------------------------------------------
// quoteFromWire decodes the presence-tracked MC std-error (proto field 7)
// ---------------------------------------------------------------------------

describe("quote decode surfaces the presence-tracked MC std-error", () => {
  it("reads price_std_error when present and leaves it undefined when absent", () => {
    const withErr = quoteFromWire({
      quote_id: 1,
      idempotency_key: "k",
      price: { bid: 0, offer: 0 },
      greeks: { price: 0.0285 },
      conventions: {},
      resolved_strike: 0,
      epoch_nanos: 0,
      valid_until_nanos: 0,
      price_std_error: 1.7e-4,
    });
    expect(withErr.priceStdError).toBe(1.7e-4);

    const noErr = quoteFromWire({
      quote_id: 1,
      idempotency_key: "k",
      price: { bid: 0, offer: 0 },
      greeks: { price: 0.031 },
      conventions: {},
      resolved_strike: 0,
      epoch_nanos: 0,
      valid_until_nanos: 0,
    });
    expect(noErr.priceStdError).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// end-to-end through the function + the WS transport (FakeSocket harness)
// ---------------------------------------------------------------------------

/** A controllable in-memory socket (mirrors test/swapsAndAsian.test.ts). */
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

describe("forward-start end-to-end over the WS mirror", () => {
  it("sends a forward_start oneof (field 16) and renders the discounted premium + Greeks", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeForwardStart({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      reset: 0.25,
      notional: 1e6,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "fwdstart");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["forward_start"]).toEqual({ option_type: 0, moneyness: 1.0, reset: 0.25 });
    replyQuote(sock, { greeks: { price: 0.0188, delta_spot: 0.49, vega: 0.0031 }, resolved_strike: 1.1 });
    const quote = await p;
    const m = formatForwardStartSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0188]);
    expect(m[1]).toEqual(["delta_spot", 0.49]);
  });
});

describe("cliquet end-to-end over the WS mirror", () => {
  it("PLAIN ratchet: sends cliquet (field 17) with no clamps and renders no std-error", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeCliquet({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      periods: 4,
      notional: 1e6,
    });
    const isMc = instr.product.kind === "cliquet" && cliquetIsMonteCarlo(instr.product.cliquet);
    expect(isMc).toBe(false);
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "cliquet-plain");
    const body = (sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>)[
      "cliquet"
    ] as Record<string, unknown>;
    expect("local_floor" in body).toBe(false);
    expect("local_cap" in body).toBe(false);
    // A plain ratchet is exact; even if a stray std-error arrived, the function
    // suppresses it because the product carries no clamp.
    replyQuote(sock, { greeks: { price: 0.031, delta_spot: 0.5 }, resolved_strike: 1.1 });
    const quote = await p;
    const m = formatCliquetSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.031]);
    expect(m.some((row) => row[0] === "std_error")).toBe(false);
  });

  it("CLAMPED cliquet: sends the clamps + MC knobs and surfaces the MC std-error row", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeCliquet({
      pair: "USDJPY",
      tenor: "1Y",
      callPut: "C",
      moneyness: 1.0,
      periods: 6,
      notional: 1e6,
      localFloor: 0.0,
      localCap: 0.03,
      mcPairs: 50000,
      mcSeed: 12345,
    });
    const isMc = instr.product.kind === "cliquet" && cliquetIsMonteCarlo(instr.product.cliquet);
    expect(isMc).toBe(true);
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "cliquet-clamped");
    const body = (sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>)[
      "cliquet"
    ] as Record<string, unknown>;
    expect(body).toMatchObject({
      option_type: 0,
      periods: 6,
      local_floor: 0.0,
      local_cap: 0.03,
      mc_pairs: 50000,
    });
    // The server returns the MC premium AND its standard error (proto field 7).
    replyQuote(sock, {
      greeks: { price: 0.0285, delta_spot: 0.47 },
      resolved_strike: 1.1,
      price_std_error: 1.7e-4,
    });
    const quote = await p;
    expect(quote.priceStdError).toBe(1.7e-4);
    const m = formatCliquetSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0285]);
    expect(m[1]).toEqual(["std_error", 1.7e-4]);
  });
});

describe("quanto end-to-end over the WS mirror", () => {
  it("sends a quanto oneof (field 18) and renders the discounted premium + Greeks", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeQuanto({
      pair: "USDJPY",
      tenor: "1Y",
      callPut: "C",
      strike: 152.5,
      notional: 1e6,
      conversionVol: 0.09,
      correlation: -0.3,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "quanto");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["quanto"]).toMatchObject({
      payoff: 0,
      option_type: 0,
      strike: 152.5,
      conversion_vol: 0.09,
      correlation: -0.3,
    });
    replyQuote(sock, { greeks: { price: 0.052, delta_spot: 0.52 }, resolved_strike: 152.5 });
    const quote = await p;
    const m = formatQuantoSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.052]);
    expect(m[1]).toEqual(["delta_spot", 0.52]);
  });
});
