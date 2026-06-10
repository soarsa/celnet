// Wave 6 — Excel client parity for the booking-model (pricing-model) selector and
// the window (partial-time) barrier that Track B-rust added to the ONE wire
// contract. These tests prove the add-in:
//   1. parses the trader-facing model selector (ANALYTIC/DEFAULT, LSV/
//      LOCAL_STOCH_VOL), case-insensitive, with a rejection message generated
//      from the canonical enum list (so it cannot drift from the supported set);
//   2. encodes the top-level `pricing_model` integer (proto `Instrument`
//      field 22) ONLY when non-DEFAULT — a DEFAULT/absent model keeps the wire
//      frame byte-identical to the contract before the field existed (proto3
//      zero value, omitted; the server reads an absent key as DEFAULT);
//   3. shapes a single-barrier under LSV (the model travels uniformly on the
//      instrument, so a BARRIER spec with model LSV reaches the LSV engine);
//   4. shapes a window barrier into the EXACT `window_barrier` product key (proto
//      field 23) the server's `crates/celnet-server/src/ws/codec.rs`
//      `window_barrier_from_json` decodes — a nested `vanilla`, `barrier`,
//      numeric `side`, `window_start`/`window_end`, and `mc_pairs`/`mc_steps`/
//      `mc_seed` — with LOCAL_STOCH_VOL pre-selected (a window barrier has no
//      closed form, so it is LSV-only);
//   5. renders the server's reply into the right spill geometry, honestly
//      surfacing the Monte-Carlo std-error row only when the MC engine returned
//      one (the exact ADI-PDE route omits it).
// No pricing math lives in the add-in — the numbers are the server's libm-core
// values (gated by celnet-server/tests/lsv_model.rs against an independent
// LsvModel reprice + the analytic oracle), so a cell is bit-identical to the
// SDK/CLI. The wire shape here is pinned to the SAME literal JSON the server's
// `window_barrier_and_pricing_model_decode_from_json` Rust test decodes.

import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  formatPremiumSpill,
  parsePricingModel,
  shapeBarrier,
  shapeWindowBarrier,
  subscriptionKey,
} from "../src/functions/shaping";
import { instrumentToWire } from "../src/contract/wsCodec";
import { pricingModel as pricingModelCodec, PRICING_MODEL_MEMBERS } from "../src/contract/enums";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { Greeks, Instrument } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// argument parsing — the trader-facing model selector → contract vocabulary
// ---------------------------------------------------------------------------

describe("pricing-model argument parsing", () => {
  it("defaults to DEFAULT (analytic) for empty/absent and the analytic aliases", () => {
    expect(parsePricingModel(undefined)).toBe("DEFAULT");
    expect(parsePricingModel("")).toBe("DEFAULT");
    expect(parsePricingModel("default")).toBe("DEFAULT");
    expect(parsePricingModel("ANALYTIC")).toBe("DEFAULT");
    expect(parsePricingModel("closed-form")).toBe("DEFAULT");
  });

  it("parses the LSV aliases case-insensitively", () => {
    expect(parsePricingModel("lsv")).toBe("LOCAL_STOCH_VOL");
    expect(parsePricingModel("LSV")).toBe("LOCAL_STOCH_VOL");
    expect(parsePricingModel("local_stoch_vol")).toBe("LOCAL_STOCH_VOL");
    expect(parsePricingModel("Local-Stoch-Vol")).toBe("LOCAL_STOCH_VOL");
    expect(parsePricingModel("  local_stochastic_volatility  ")).toBe("LOCAL_STOCH_VOL");
  });

  it("rejects garbage with a message generated from the canonical enum list", () => {
    expect(() => parsePricingModel("heston")).toThrow(ShapingError);
    try {
      parsePricingModel("heston");
    } catch (err) {
      const msg = (err as ShapingError).message;
      // The accepted-list is derived from the SAME members list the wire codec is
      // built from, so it can never drift from the supported set.
      for (const m of PRICING_MODEL_MEMBERS) expect(msg).toContain(m);
      expect(msg).toContain("LSV");
    }
  });
});

// ---------------------------------------------------------------------------
// enum codec — the string ↔ proto-number projection (the wire contract)
// ---------------------------------------------------------------------------

describe("pricing-model wire codec (proto enum numbers)", () => {
  it("maps DEFAULT↔0 and LOCAL_STOCH_VOL↔1, reversibly", () => {
    expect(pricingModelCodec.toWire("DEFAULT")).toBe(0);
    expect(pricingModelCodec.toWire("LOCAL_STOCH_VOL")).toBe(1);
    expect(pricingModelCodec.fromWire(0)).toBe("DEFAULT");
    expect(pricingModelCodec.fromWire(1)).toBe("LOCAL_STOCH_VOL");
    // An unknown number clamps to the proto3 zero value (DEFAULT), exactly as a
    // proto3 reader would for a member this build does not yet name.
    expect(pricingModelCodec.fromWire(7)).toBe("DEFAULT");
  });
  it("the members list is the index === proto enum number contract", () => {
    expect(PRICING_MODEL_MEMBERS).toEqual(["DEFAULT", "LOCAL_STOCH_VOL"]);
  });
});

// ---------------------------------------------------------------------------
// pricing-model on the instrument — encoded ONLY when non-DEFAULT (field 22)
// ---------------------------------------------------------------------------

describe("pricing-model encoding on the instrument (proto field 22)", () => {
  it("omits `pricing_model` entirely for the DEFAULT (analytic) model — byte-identical wire", () => {
    const analytic = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.25,
      kind: "KNOCK_OUT",
      side: "UP",
      // model omitted ⇒ DEFAULT
    });
    expect(analytic.pricingModel).toBe("DEFAULT");
    const wire = instrumentToWire(analytic);
    // The proto3 zero value is omitted on the wire — the frame is identical to the
    // contract before the field existed (the server's `enum_or_zero` reads it as 0).
    expect("pricing_model" in wire).toBe(false);
  });

  it("encodes a single-barrier under LSV with `pricing_model: 1` on the instrument", () => {
    const lsv = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
      kind: "KNOCK_OUT",
      side: "UP",
      model: "LSV",
    });
    expect(lsv.pricingModel).toBe("LOCAL_STOCH_VOL");
    const wire = instrumentToWire(lsv);
    expect(wire["pricing_model"]).toBe(1);
    // The product oneof is unchanged — LSV is a model selector, not a product.
    expect(wire["single_barrier"]).toMatchObject({
      vanilla: { option_type: 0, strike: { strike: 1.1 } },
      kind: 1, // KNOCK_OUT
      side: 0, // UP
      barrier: 1.3,
      monitoring: 0, // CONTINUOUS
    });
  });

  it("a double-barrier accepts the LSV directive (the SERVER rejects it, never the cell)", () => {
    // The add-in does not pre-judge product support — it shapes the request and lets
    // the server return INVALID_ARGUMENT (honest, never a silent client-side fallback).
    const dbar = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.0,
      upperBarrier: 1.3,
      kind: "KNOCK_OUT",
      model: "lsv",
    });
    const wire = instrumentToWire(dbar);
    expect(wire["pricing_model"]).toBe(1);
    expect("double_barrier" in wire).toBe(true);
  });

  it("distinguishes the same single barrier priced ANALYTIC vs LSV in the coalescing key", () => {
    const analytic = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
      kind: "KNOCK_OUT",
      side: "UP",
    });
    const lsv = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
      kind: "KNOCK_OUT",
      side: "UP",
      model: "LSV",
    });
    // Two cells differing only in booking model must NOT coalesce onto one
    // subscription (LSV ≠ analytic value for the supported products).
    expect(subscriptionKey(analytic, DEFAULT_CONVENTIONS)).not.toBe(
      subscriptionKey(lsv, DEFAULT_CONVENTIONS),
    );
  });
});

// ---------------------------------------------------------------------------
// window barrier shaping → wire encoding (the proto field-23 product contract)
// ---------------------------------------------------------------------------

describe("window-barrier shaping + wire encoding", () => {
  it("shapes a window barrier with LOCAL_STOCH_VOL pre-selected and a full-life default window", () => {
    const i = shapeWindowBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
    });
    expect(i.pricingModel).toBe("LOCAL_STOCH_VOL");
    expect(i.side).toBe("TWO_WAY");
    expect(i.product).toEqual({
      kind: "windowBarrier",
      windowBarrier: {
        vanilla: { optionType: "CALL", strike: { kind: "strike", strike: 1.1 } },
        barrier: 1.3,
        side: "UP",
        windowStart: 0,
        windowEnd: i.expiryYears, // full-life default
        mcPairs: 0,
        mcSteps: 0,
        mcSeed: 0n,
      },
    });
  });

  it("encodes the oneof under `window_barrier` (field 23) — the EXACT shape the server decodes", () => {
    // This literal matches `window_barrier_and_pricing_model_decode_from_json` in
    // crates/celnet-server/src/ws/codec.rs (the authoritative Rust decode test).
    const i = shapeWindowBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "P",
      notional: 1e6,
      barrier: 1.3,
      side: "UP",
      windowStart: 0.25,
      windowEnd: 0.75,
      mcPairs: 8000,
      mcSteps: 64,
      mcSeed: 42,
    });
    const wire = instrumentToWire(i);
    // The instrument MUST carry pricing_model = 1 (window barrier is LSV-only).
    expect(wire["pricing_model"]).toBe(1);
    expect(wire["window_barrier"]).toEqual({
      vanilla: { option_type: 1, strike: { strike: 1.1 } }, // PUT
      barrier: 1.3,
      side: 0, // BARRIER_SIDE_UP
      window_start: 0.25,
      window_end: 0.75,
      mc_pairs: 8000,
      mc_steps: 64,
      mc_seed: 42,
    });
    // Exactly one product arm is present (oneof discipline).
    expect("single_barrier" in wire).toBe(false);
    expect("double_barrier" in wire).toBe(false);
    expect("vanilla" in wire).toBe(false);
  });

  it("encodes a delta-quoted strike under the nested vanilla and a DOWN side", () => {
    const i = shapeWindowBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: "25dP",
      callPut: "P",
      notional: 1e6,
      barrier: 0.95,
      side: "DOWN",
      windowEnd: 0.5, // a "front" partial barrier
    });
    const wb = instrumentToWire(i)["window_barrier"] as Record<string, unknown>;
    expect(wb["vanilla"]).toEqual({ option_type: 1, strike: { delta: -0.25 } });
    expect(wb["side"]).toBe(1); // DOWN
    expect(wb["window_start"]).toBe(0);
    expect(wb["window_end"]).toBe(0.5);
  });

  it("rejects a non-positive barrier, a degenerate window, and a window past expiry", () => {
    const base = {
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
    } as const;
    expect(() => shapeWindowBarrier({ ...base, barrier: 0 })).toThrow(ShapingError);
    expect(() =>
      shapeWindowBarrier({ ...base, barrier: 1.3, windowStart: 0.6, windowEnd: 0.6 }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeWindowBarrier({ ...base, barrier: 1.3, windowStart: 0.6, windowEnd: 0.3 }),
    ).toThrow(ShapingError);
    // window end beyond the 1Y expiry (~1.0 year fraction) is rejected.
    expect(() =>
      shapeWindowBarrier({ ...base, barrier: 1.3, windowEnd: 2.0 }),
    ).toThrow(ShapingError);
  });

  it("distinguishes window barriers from single barriers and from MC-vs-PDE in the coalescing key", () => {
    const sbar = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
      kind: "KNOCK_OUT",
      side: "UP",
      model: "LSV",
    });
    const pde = shapeWindowBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
    });
    const mc = shapeWindowBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
      mcPairs: 8000,
      mcSeed: 42,
    });
    const keys = new Set([
      subscriptionKey(sbar, DEFAULT_CONVENTIONS),
      subscriptionKey(pde, DEFAULT_CONVENTIONS),
      subscriptionKey(mc, DEFAULT_CONVENTIONS),
    ]);
    expect(keys.size).toBe(3);
  });
});

// ---------------------------------------------------------------------------
// spill formatting — exact (ADI-PDE) carries no std-error; MC carries one
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

describe("window-barrier / LSV spill formatting", () => {
  it("the exact ADI-PDE route spills premium + 13 Greeks + footer (NO std-error row)", () => {
    const m = formatPremiumSpill({
      premium: SAMPLE_GREEKS.price,
      stdError: undefined,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 9n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.0123]);
    expect(m.length).toBe(15); // 1 premium + 13 Greeks + 1 footer
    expect(m.some((r) => r[0] === "std_error")).toBe(false);
  });

  it("the Monte-Carlo route inserts an honest std-error row after the premium", () => {
    const m = formatPremiumSpill({
      premium: SAMPLE_GREEKS.price,
      stdError: 1.5e-5,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 9n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 0.0123]);
    expect(m[1]).toEqual(["std_error", 1.5e-5]);
    expect(m.length).toBe(16); // + the std-error row
  });
});

// ---------------------------------------------------------------------------
// end-to-end through the function composition + the WS transport (FakeSocket)
// ---------------------------------------------------------------------------

/** A controllable in-memory socket (mirrors test/barrierDigitalTouch.test.ts). */
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

/** Reply to the outstanding request_quote with a server-shaped quote body. */
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

describe("single-barrier-under-LSV end-to-end over the WS mirror", () => {
  it("sends pricing_model=1 alongside the single_barrier oneof and renders the premium", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
      kind: "KNOCK_OUT",
      side: "UP",
      model: "lsv",
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "barrier-lsv");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["pricing_model"]).toBe(1);
    expect(wireInstr["single_barrier"]).toMatchObject({ kind: 1, side: 0, barrier: 1.3 });
    // The LSV PDE route is exact for a single barrier ⇒ no std-error in the reply.
    replyQuote(sock, { greeks: { price: 0.0071, delta_spot: 0.28 }, resolved_strike: 1.1 });
    const quote = await p;
    expect(quote.priceStdError).toBeUndefined();
    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0071]);
    expect(m.some((r) => r[0] === "std_error")).toBe(false);
  });
});

describe("window-barrier end-to-end over the WS mirror", () => {
  it("sends a window_barrier oneof (field 23) on the ADI-PDE route and renders premium + Greeks (no std-error)", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeWindowBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
      side: "UP",
      windowStart: 0.25,
      windowEnd: 0.75,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "window-barrier-pde");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["pricing_model"]).toBe(1);
    expect(wireInstr["window_barrier"]).toMatchObject({
      vanilla: { option_type: 0, strike: { strike: 1.1 } },
      barrier: 1.3,
      side: 0,
      window_start: 0.25,
      window_end: 0.75,
      mc_pairs: 0, // ADI-PDE
    });
    // An exact PDE reply carries no std-error.
    replyQuote(sock, { greeks: { price: 0.0042, delta_spot: 0.19 }, resolved_strike: 1.1 });
    const quote = await p;
    expect(quote.priceStdError).toBeUndefined();
    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0042]);
    expect(m.some((r) => r[0] === "std_error")).toBe(false);
  });

  it("sends mc_pairs>0 on the Monte-Carlo route and surfaces the server's std-error honestly", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeWindowBarrier({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.1,
      callPut: "C",
      notional: 1e6,
      barrier: 1.3,
      mcPairs: 8000,
      mcSteps: 64,
      mcSeed: 42,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "window-barrier-mc");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<string, unknown>;
    expect(wireInstr["window_barrier"]).toMatchObject({
      mc_pairs: 8000,
      mc_steps: 64,
      mc_seed: 42,
    });
    // The MC engine returns a price_std_error — surfaced as the std_error row.
    replyQuote(sock, { greeks: { price: 0.0039 }, resolved_strike: 1.1, price_std_error: 2.1e-5 });
    const quote = await p;
    expect(quote.priceStdError).toBe(2.1e-5);
    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 0.0039]);
    expect(m[1]).toEqual(["std_error", 2.1e-5]);
  });
});
