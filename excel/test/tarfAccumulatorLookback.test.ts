// Wave 3 — Excel client parity for the TARF, accumulator and lookback the Rust
// slice put on the ONE wire contract. These tests prove the add-in shapes each
// product into the EXACT wire shape the server's `crates/celnet-server/src/ws/
// codec.rs` decodes (the product oneof keys `tarf` / `accumulator` / `lookback`
// carrying field numbers 19 / 20 / 21), that the count-based fixing schedule
// encodes the SAME `fixing_years` array the SDK's `equal_fixing_years` produces
// (so an Excel TARF/accumulator is bit-identical to the SDK/CLI), and that the
// TARF / ACCUMULATOR / LOOKBACK families (the polymorphic CELNET.INSTRUMENT +
// verb path) decode the SERVER's reply into
// the right spill geometry — including the HONEST Monte-Carlo std-error row.
//
// MC-honesty is the heart of this wave: TARF and accumulator are ALWAYS Monte-
// Carlo priced (the server genuinely emits `price_std_error` on the quote), so
// the spill always carries a `std_error` row; a lookback carries it for the
// DISCRETE variant only (CONTINUOUS is the exact Conze-Viswanathan / Goldman-
// Sosin-Gatto closed form — no std-error). The end-to-end tests assert against
// SERVER-SHAPED quote replies — the same `quote` frame the server's WS mirror
// emits, with `price_std_error` present exactly when the product is MC-priced —
// never a field the server cannot produce. No pricing math lives in the add-in;
// the numbers are the server's libm-core values, so a cell is bit-identical to
// the SDK/CLI (whose gate tests reconcile the server pricer to the celnet-exotics
// closed/MC forms + celnet-golden as the independent oracle).

import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  formatPremiumSpill,
  lookbackIsMonteCarlo,
  parseAccumulatorMonitoring,
  parseLookbackMonitoring,
  parseLookbackStyle,
  parseTarfRedemption,
  shapeAccumulator,
  shapeLookback,
  shapeTarf,
  subscriptionKey,
} from "../src/functions/shaping";
import { instrumentToWire } from "../src/contract/wsCodec";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { Accumulator, Greeks, Instrument, Lookback, Tarf } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// enum-selector parsing
// ---------------------------------------------------------------------------

describe("path-dependent enum selector parsing", () => {
  it("parses the TARF redemption with defaults and aliases and rejects garbage", () => {
    expect(parseTarfRedemption(undefined)).toBe("FULL_GAIN");
    expect(parseTarfRedemption("")).toBe("FULL_GAIN");
    expect(parseTarfRedemption("full")).toBe("FULL_GAIN");
    expect(parseTarfRedemption("CAPPED_GAIN")).toBe("CAPPED_GAIN");
    expect(parseTarfRedemption("capped")).toBe("CAPPED_GAIN");
    expect(() => parseTarfRedemption("knockout")).toThrow(ShapingError);
  });

  it("parses the accumulator monitoring with defaults and aliases and rejects garbage", () => {
    expect(parseAccumulatorMonitoring(undefined)).toBe("DISCRETE");
    expect(parseAccumulatorMonitoring("disc")).toBe("DISCRETE");
    expect(parseAccumulatorMonitoring("CONTINUOUS")).toBe("CONTINUOUS");
    expect(parseAccumulatorMonitoring("cont")).toBe("CONTINUOUS");
    expect(() => parseAccumulatorMonitoring("hourly")).toThrow(ShapingError);
  });

  it("parses the lookback style/monitoring with defaults and aliases and rejects garbage", () => {
    expect(parseLookbackStyle(undefined)).toBe("FLOATING");
    expect(parseLookbackStyle("fix")).toBe("FIXED");
    expect(() => parseLookbackStyle("asian")).toThrow(ShapingError);
    expect(parseLookbackMonitoring(undefined)).toBe("CONTINUOUS");
    expect(parseLookbackMonitoring("disc")).toBe("DISCRETE");
    expect(() => parseLookbackMonitoring("weekly")).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// TARF: shaping → wire encoding (the contract field-number + schedule contract)
// ---------------------------------------------------------------------------

describe("TARF shaping + wire encoding", () => {
  it("shapes a TWO_WAY exporter TARF with an equally-spaced fixing schedule", () => {
    const i = shapeTarf({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "P",
      strike: 1.1,
      target: 0.1,
      leverage: 2,
      fixings: 4,
      notional: 1e6,
    });
    expect(i.side).toBe("TWO_WAY");
    expect(i.product.kind).toBe("tarf");
    const t = (i.product as { kind: "tarf"; tarf: Tarf }).tarf;
    expect(t.optionType).toBe("PUT");
    expect(t.strike).toBe(1.1);
    expect(t.target).toBe(0.1);
    expect(t.leverage).toBe(2);
    expect(t.redemption).toBe("FULL_GAIN");
    // The count-based schedule is k/n for k=1..=n — bit-identical to the SDK's
    // `equal_fixing_years` so an Excel TARF encodes the same wire as the SDK/CLI.
    expect(t.schedule.fixingYears).toEqual([0.25, 0.5, 0.75, 1.0]);
    expect(t.schedule.fixingNotional).toBe(1.0);
    expect(t.mcSeed).toBe(0n);
  });

  it("encodes the oneof under `tarf` (proto field 19) with a nested schedule + numeric enums", () => {
    const i = shapeTarf({
      pair: "USDJPY",
      tenor: "1Y",
      callPut: "P",
      strike: 150,
      target: 0.08,
      leverage: 1.5,
      fixings: 2,
      notional: 1e6,
      redemption: "CAPPED_GAIN",
      fixingNotional: 0.5,
      mcPairs: 50000,
      mcSeed: 12345,
    });
    const wire = instrumentToWire(i);
    expect(wire["tarf"]).toEqual({
      option_type: 1, // PUT
      strike: 150,
      target: 0.08,
      leverage: 1.5,
      redemption: 1, // TARF_REDEMPTION_CAPPED_GAIN
      schedule: { fixing_years: [0.5, 1.0], fixing_notional: 0.5 },
      mc_pairs: 50000,
      mc_seed: 12345, // 64-bit field carried as a plain JSON number
    });
    // Exactly one product arm is present (oneof discipline).
    expect("accumulator" in wire).toBe(false);
    expect("lookback" in wire).toBe(false);
    expect("vanilla" in wire).toBe(false);
  });

  it("rejects a delta strike, a non-positive target, a negative leverage and < 1 fixings", () => {
    const base = {
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "P",
      target: 0.1,
      leverage: 2,
      fixings: 4,
      notional: 1e6,
    };
    expect(() => shapeTarf({ ...base, strike: "25dP" })).toThrow(ShapingError);
    expect(() => shapeTarf({ ...base, strike: 1.1, target: 0 })).toThrow(ShapingError);
    expect(() => shapeTarf({ ...base, strike: 1.1, leverage: -1 })).toThrow(ShapingError);
    expect(() => shapeTarf({ ...base, strike: 1.1, fixings: 0 })).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// accumulator: shaping → wire encoding (barrier > pivot, schedule)
// ---------------------------------------------------------------------------

describe("accumulator shaping + wire encoding", () => {
  it("shapes a TWO_WAY accumulator with pivot/barrier and an equally-spaced schedule", () => {
    const i = shapeAccumulator({
      pair: "EURUSD",
      tenor: "1Y",
      pivot: 1.1,
      barrier: 1.15,
      leverage: 2,
      fixings: 3,
      notional: 1e6,
    });
    const a = (i.product as { kind: "accumulator"; accumulator: Accumulator }).accumulator;
    expect(a.pivot).toBe(1.1);
    expect(a.barrier).toBe(1.15);
    expect(a.monitoring).toBe("DISCRETE");
    expect(a.schedule.fixingYears).toEqual([1 / 3, 2 / 3, 1.0]);
    expect(a.schedule.fixingNotional).toBe(1.0);
  });

  it("encodes the oneof under `accumulator` (proto field 20) with continuous monitoring", () => {
    const i = shapeAccumulator({
      pair: "USDJPY",
      tenor: "1Y",
      pivot: 150,
      barrier: 158,
      leverage: 1,
      fixings: 2,
      notional: 1e6,
      monitoring: "CONTINUOUS",
      mcPairs: 40000,
      mcSeed: 777,
    });
    const wire = instrumentToWire(i);
    expect(wire["accumulator"]).toEqual({
      pivot: 150,
      barrier: 158,
      leverage: 1,
      monitoring: 1, // ACCUMULATOR_MONITORING_CONTINUOUS
      schedule: { fixing_years: [0.5, 1.0], fixing_notional: 1.0 },
      mc_pairs: 40000,
      mc_seed: 777,
    });
    expect("tarf" in wire).toBe(false);
    expect("lookback" in wire).toBe(false);
  });

  it("rejects a barrier at/below the pivot, a delta pivot, a negative leverage and < 1 fixings", () => {
    const base = { pair: "EURUSD", tenor: "1Y", leverage: 1, fixings: 3, notional: 1e6 };
    expect(() => shapeAccumulator({ ...base, pivot: 1.1, barrier: 1.1 })).toThrow(ShapingError);
    expect(() => shapeAccumulator({ ...base, pivot: 1.1, barrier: 1.0 })).toThrow(ShapingError);
    expect(() => shapeAccumulator({ ...base, pivot: "25dC", barrier: 1.2 })).toThrow(ShapingError);
    expect(() => shapeAccumulator({ ...base, pivot: 1.1, barrier: 1.2, leverage: -1 })).toThrow(
      ShapingError,
    );
    expect(() => shapeAccumulator({ ...base, pivot: 1.1, barrier: 1.2, fixings: 0 })).toThrow(
      ShapingError,
    );
  });
});

// ---------------------------------------------------------------------------
// lookback: shaping → wire encoding (floating vs fixed, continuous vs discrete)
// ---------------------------------------------------------------------------

describe("lookback shaping + wire encoding", () => {
  it("shapes a FLOATING CONTINUOUS lookback (exact closed form, not MC)", () => {
    const i = shapeLookback({ pair: "EURUSD", tenor: "1Y", callPut: "C", notional: 1e6 });
    const l = (i.product as { kind: "lookback"; lookback: Lookback }).lookback;
    expect(l.style).toBe("FLOATING");
    expect(l.monitoring).toBe("CONTINUOUS");
    expect(l.strike).toBe(0);
    expect(l.observations).toBe(0);
    expect(l.mcSeed).toBe(0n);
    expect(lookbackIsMonteCarlo(l)).toBe(false);

    const wire = instrumentToWire(i);
    expect(wire["lookback"]).toEqual({
      style: 0, // LOOKBACK_STYLE_FLOATING
      option_type: 0, // CALL
      monitoring: 0, // LOOKBACK_MONITORING_CONTINUOUS
      strike: 0,
      observations: 0,
      mc_pairs: 0,
      mc_seed: 0,
    });
  });

  it("shapes a FIXED DISCRETE lookback (MC-priced) carrying its strike + observations", () => {
    const i = shapeLookback({
      pair: "USDJPY",
      tenor: "1Y",
      callPut: "P",
      notional: 1e6,
      style: "FIXED",
      monitoring: "DISCRETE",
      strike: 150,
      observations: 252,
      mcPairs: 60000,
      mcSeed: 99,
    });
    const l = (i.product as { kind: "lookback"; lookback: Lookback }).lookback;
    expect(lookbackIsMonteCarlo(l)).toBe(true);
    expect(l.strike).toBe(150);
    expect(l.observations).toBe(252);
    expect(l.mcSeed).toBe(99n);

    const wire = instrumentToWire(i);
    expect(wire["lookback"]).toEqual({
      style: 1, // LOOKBACK_STYLE_FIXED
      option_type: 1, // PUT
      monitoring: 1, // LOOKBACK_MONITORING_DISCRETE
      strike: 150,
      observations: 252,
      mc_pairs: 60000,
      mc_seed: 99,
    });
  });

  it("rejects a strike on a FLOATING lookback and requires one on a FIXED lookback", () => {
    // FLOATING settles vs the path extremum — a supplied strike is a user error.
    expect(() =>
      shapeLookback({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        notional: 1e6,
        style: "FLOATING",
        strike: 1.1,
      }),
    ).toThrow(ShapingError);
    // FIXED needs an absolute strike.
    expect(() =>
      shapeLookback({ pair: "EURUSD", tenor: "1Y", callPut: "C", notional: 1e6, style: "FIXED" }),
    ).toThrow(ShapingError);
    // A delta strike is never valid for a lookback.
    expect(() =>
      shapeLookback({
        pair: "EURUSD",
        tenor: "1Y",
        callPut: "C",
        notional: 1e6,
        style: "FIXED",
        strike: "25dC",
      }),
    ).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// subscription coalescing key over the new products
// ---------------------------------------------------------------------------

describe("subscription coalescing key over the path-dependent products", () => {
  it("distinguishes a TARF from an accumulator from a lookback", () => {
    const tarf = shapeTarf({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "P",
      strike: 1.1,
      target: 0.1,
      leverage: 2,
      fixings: 4,
      notional: 1e6,
    });
    const acc = shapeAccumulator({
      pair: "EURUSD",
      tenor: "1Y",
      pivot: 1.1,
      barrier: 1.15,
      leverage: 2,
      fixings: 4,
      notional: 1e6,
    });
    const lbk = shapeLookback({ pair: "EURUSD", tenor: "1Y", callPut: "C", notional: 1e6 });
    const keys = new Set([
      subscriptionKey(tarf, DEFAULT_CONVENTIONS),
      subscriptionKey(acc, DEFAULT_CONVENTIONS),
      subscriptionKey(lbk, DEFAULT_CONVENTIONS),
    ]);
    expect(keys.size).toBe(3);
  });

  it("distinguishes a continuous from a discrete lookback that differ only in monitoring", () => {
    const cont = shapeLookback({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      notional: 1e6,
      monitoring: "CONTINUOUS",
    });
    const disc = shapeLookback({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      notional: 1e6,
      monitoring: "DISCRETE",
      observations: 100,
    });
    expect(subscriptionKey(cont, DEFAULT_CONVENTIONS)).not.toBe(
      subscriptionKey(disc, DEFAULT_CONVENTIONS),
    );
  });
});

// ---------------------------------------------------------------------------
// spill formatting (MC honesty)
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

describe("path-dependent spill formatting", () => {
  it("surfaces the MC std-error row when the server reports one (TARF/accumulator/discrete-lookback)", () => {
    const m = formatPremiumSpill({
      premium: 0.0285,
      stdError: 1.7e-4,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 3n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.0285]);
    expect(m[1]).toEqual(["std_error", 1.7e-4]);
    // premium + std_error + 13 Greeks + footer = 16 rows.
    expect(m.length).toBe(16);
    expect(m[2]).toEqual(["delta_spot", 0.51]);
    expect(String(m[15]?.[0])).toContain("surface v3");
  });

  it("OMITS the std-error row for an exact closed form (continuous lookback)", () => {
    const m = formatPremiumSpill({
      premium: 0.031,
      stdError: undefined,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: undefined,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.031]);
    // No std-error row ⇒ premium + 13 Greeks + footer = 15 rows.
    expect(m.length).toBe(15);
    expect(m[1]).toEqual(["delta_spot", 0.51]);
    expect(m.some((row) => row[0] === "std_error")).toBe(false);
    expect(String(m[14]?.[0])).toContain("surface live");
  });
});

// ---------------------------------------------------------------------------
// end-to-end through the function + the WS transport (FakeSocket harness)
// asserting against SERVER-SHAPED quote replies
// ---------------------------------------------------------------------------

/** A controllable in-memory socket (mirrors test/forwardStartCliquetQuanto.test.ts). */
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
 * Reply to the (single) outstanding request_quote with a SERVER-SHAPED quote — the
 * exact `quote` frame the server's WS mirror emits. The server stamps
 * `price_std_error` on `body` only for an MC-priced product; this harness passes
 * through whatever the test supplies so we never fabricate a field the server
 * cannot produce.
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

function wireProduct(sock: FakeSocket, key: string): Record<string, unknown> {
  const instr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
  return instr[key] as Record<string, unknown>;
}

describe("TARF end-to-end over the WS mirror — server emits the MC std-error", () => {
  it("sends a tarf oneof (field 19) and renders premium + MC std-error + Greeks", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeTarf({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "P",
      strike: 1.1,
      target: 0.1,
      leverage: 2,
      fixings: 4,
      notional: 1e6,
      mcPairs: 50000,
      mcSeed: 7,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "tarf");
    const body = wireProduct(sock, "tarf");
    expect(body["schedule"]).toEqual({ fixing_years: [0.25, 0.5, 0.75, 1.0], fixing_notional: 1.0 });
    expect(body["mc_pairs"]).toBe(50000);
    // The server returns the MC premium AND its standard error (proto field 7) —
    // a TARF is always MC-priced, so the server genuinely stamps it.
    replyQuote(sock, {
      greeks: { price: 0.0123, delta_spot: -0.4, vega: 0.0021 },
      resolved_strike: 1.1,
      price_std_error: 2.4e-4,
    });
    const quote = await p;
    expect(quote.priceStdError).toBe(2.4e-4);
    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0123]);
    expect(m[1]).toEqual(["std_error", 2.4e-4]);
    expect(m[2]).toEqual(["delta_spot", -0.4]);
  });
});

describe("accumulator end-to-end over the WS mirror — server emits the MC std-error", () => {
  it("sends an accumulator oneof (field 20) and renders premium + MC std-error + Greeks", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeAccumulator({
      pair: "EURUSD",
      tenor: "1Y",
      pivot: 1.1,
      barrier: 1.15,
      leverage: 2,
      fixings: 3,
      notional: 1e6,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "accumulator");
    const body = wireProduct(sock, "accumulator");
    expect(body).toMatchObject({
      pivot: 1.1,
      barrier: 1.15,
      monitoring: 0, // DISCRETE
      schedule: { fixing_years: [1 / 3, 2 / 3, 1.0], fixing_notional: 1.0 },
    });
    replyQuote(sock, {
      greeks: { price: 0.0451, delta_spot: 0.62 },
      resolved_strike: 1.1,
      price_std_error: 3.1e-4,
    });
    const quote = await p;
    expect(quote.priceStdError).toBe(3.1e-4);
    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0451]);
    expect(m[1]).toEqual(["std_error", 3.1e-4]);
  });
});

describe("lookback end-to-end over the WS mirror — MC iff DISCRETE", () => {
  it("CONTINUOUS: sends lookback (field 21), server omits std-error ⇒ no std-error row", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeLookback({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "C",
      notional: 1e6,
      monitoring: "CONTINUOUS",
    });
    const isMc = instr.product.kind === "lookback" && lookbackIsMonteCarlo(instr.product.lookback);
    expect(isMc).toBe(false);
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "lookback-cont");
    const body = wireProduct(sock, "lookback");
    expect(body["monitoring"]).toBe(0); // CONTINUOUS
    // The server prices a continuous lookback by exact closed form ⇒ it does NOT
    // stamp price_std_error. The function suppresses any std-error for a non-MC
    // product anyway, so the spill omits the row.
    replyQuote(sock, { greeks: { price: 0.072, delta_spot: 0.55 }, resolved_strike: 0 });
    const quote = await p;
    expect(quote.priceStdError).toBeUndefined();
    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.072]);
    expect(m.some((row) => row[0] === "std_error")).toBe(false);
  });

  it("DISCRETE: sends the strike+observations+MC knobs and surfaces the server's std-error", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeLookback({
      pair: "USDJPY",
      tenor: "1Y",
      callPut: "P",
      notional: 1e6,
      style: "FIXED",
      monitoring: "DISCRETE",
      strike: 150,
      observations: 252,
      mcPairs: 60000,
      mcSeed: 99,
    });
    const isMc = instr.product.kind === "lookback" && lookbackIsMonteCarlo(instr.product.lookback);
    expect(isMc).toBe(true);
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "lookback-disc");
    const body = wireProduct(sock, "lookback");
    expect(body).toMatchObject({
      style: 1, // FIXED
      option_type: 1, // PUT
      monitoring: 1, // DISCRETE
      strike: 150,
      observations: 252,
      mc_pairs: 60000,
    });
    replyQuote(sock, {
      greeks: { price: 0.0612, delta_spot: -0.48 },
      resolved_strike: 150,
      price_std_error: 1.2e-4,
    });
    const quote = await p;
    expect(quote.priceStdError).toBe(1.2e-4);
    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0612]);
    expect(m[1]).toEqual(["std_error", 1.2e-4]);
  });
});
