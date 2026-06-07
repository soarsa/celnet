// PC-AMERICAN — Excel client parity for the American / Bermudan early-exercise
// vanilla (proto `AmericanOption`, product field 24) that the exotics/proto/server
// slice added to the ONE wire contract. These tests prove the add-in:
//   1. parses the trader-facing exercise-style selector (AMERICAN/AMER/A,
//      BERMUDAN/BERM/B), case-insensitive, rejecting garbage;
//   2. shapes a continuous AMERICAN (empty bermudan_dates) and a BERMUDAN with the
//      `n` equally-spaced exercise dates `k/n·T` over (0, T] — BIT-IDENTICAL to the
//      CLI's `american_spec` (crates/celnet-cli/src/exotic.rs), so an Excel cell
//      encodes the SAME `bermudan_dates` the CLI does;
//   3. encodes the oneof under the EXACT `american` product key (proto field 24)
//      the server's crates/celnet-server/src/ws/codec.rs `american_from_json`
//      decodes — `option_type`, `strike`, numeric `exercise_style`, a
//      `bermudan_dates` array, and the `lsm_paths`/`lsm_exercise_dates`/`lsm_seed`
//      knobs (64-bit seed as a JSON number, the codec's convention);
//   4. selects the exact projected-SOR FD engine for `lsmPaths == 0` (no std-error)
//      and the Longstaff-Schwartz Monte-Carlo engine for `lsmPaths > 0` (which
//      carries a `price_std_error`), surfacing the std-error row honestly;
//   5. renders the server's reply into the right spill geometry end-to-end over the
//      WS mirror.
// No pricing math lives in the add-in — the numbers are the server's libm-core
// values (gated by celnet-server's american_*_on_the_wire gates against an
// independent celnet_vanilla European GK + celnet_exotics::american_lsm oracle and
// the hand-pinned Longstaff & Schwartz (2001) Table 1 reference), so a cell is
// bit-identical to the SDK/CLI. The end-to-end FD reply value below is pinned to
// that published reference (2.314; the server's PSOR FD gives 2.3195) so the Excel
// parity test documents the SAME oracle the Rust gate uses.

import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  americanIsMonteCarlo,
  formatPathDependentSpill,
  parseExerciseStyle,
  shapeAmerican,
  shapeLookback,
  subscriptionKey,
} from "../src/functions/shaping";
import { instrumentToWire } from "../src/contract/wsCodec";
import { exerciseStyle as exerciseStyleCodec } from "../src/contract/enums";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { AmericanOption, Greeks, Instrument } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// argument parsing — the trader-facing exercise-style selector → vocabulary
// ---------------------------------------------------------------------------

describe("exercise-style argument parsing", () => {
  it("defaults to AMERICAN for empty/absent and the american aliases", () => {
    expect(parseExerciseStyle(undefined)).toBe("AMERICAN");
    expect(parseExerciseStyle("")).toBe("AMERICAN");
    expect(parseExerciseStyle("american")).toBe("AMERICAN");
    expect(parseExerciseStyle("Amer")).toBe("AMERICAN");
    expect(parseExerciseStyle("a")).toBe("AMERICAN");
  });

  it("parses the bermudan aliases case-insensitively", () => {
    expect(parseExerciseStyle("bermudan")).toBe("BERMUDAN");
    expect(parseExerciseStyle("BERM")).toBe("BERMUDAN");
    expect(parseExerciseStyle("  B  ")).toBe("BERMUDAN");
  });

  it("rejects garbage with a ShapingError", () => {
    expect(() => parseExerciseStyle("euro")).toThrow(ShapingError);
    expect(() => parseExerciseStyle("knockout")).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// enum codec — the string ↔ proto-number projection (the wire contract)
// ---------------------------------------------------------------------------

describe("exercise-style wire codec (proto enum numbers)", () => {
  it("maps AMERICAN↔0 and BERMUDAN↔1, reversibly", () => {
    expect(exerciseStyleCodec.toWire("AMERICAN")).toBe(0);
    expect(exerciseStyleCodec.toWire("BERMUDAN")).toBe(1);
    expect(exerciseStyleCodec.fromWire(0)).toBe("AMERICAN");
    expect(exerciseStyleCodec.fromWire(1)).toBe("BERMUDAN");
    // An unknown number clamps to the proto3 zero value (AMERICAN).
    expect(exerciseStyleCodec.fromWire(9)).toBe("AMERICAN");
  });
});

// ---------------------------------------------------------------------------
// American shaping → wire encoding (the proto field-24 product contract)
// ---------------------------------------------------------------------------

describe("american shaping + wire encoding", () => {
  it("shapes a continuous AMERICAN with an empty bermudan_dates set and the FD engine", () => {
    const i = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "P",
      notional: 1e6,
    });
    expect(i.side).toBe("TWO_WAY");
    // Early exercise is the product's native engine — the booking model stays
    // DEFAULT (not a pricing directive), so the wire omits pricing_model entirely.
    expect(i.pricingModel).toBeUndefined();
    expect(i.product).toEqual({
      kind: "american",
      american: {
        optionType: "PUT",
        strike: 1.1,
        exerciseStyle: "AMERICAN",
        bermudanDates: [],
        lsmPaths: 0,
        lsmExerciseDates: 0,
        lsmSeed: 0n,
      },
    });
  });

  it("encodes the oneof under `american` (field 24) — the EXACT shape the server decodes", () => {
    const i = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "P",
      notional: 1e6,
    });
    const wire = instrumentToWire(i);
    expect(wire["american"]).toEqual({
      option_type: 1, // PUT
      strike: 1.1,
      exercise_style: 0, // AMERICAN
      bermudan_dates: [],
      lsm_paths: 0,
      lsm_exercise_dates: 0,
      lsm_seed: 0,
    });
    // Exactly one product arm is present (oneof discipline) — no stray vanilla.
    expect("vanilla" in wire).toBe(false);
    expect("single_barrier" in wire).toBe(false);
    // Early exercise is the native engine: no pricing_model on the wire.
    expect("pricing_model" in wire).toBe(false);
  });

  it("shapes a BERMUDAN from a step count into the k/n·T dates (bit-identical to the CLI)", () => {
    const i = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.05,
      callPut: "C",
      notional: 1e6,
      style: "BERMUDAN",
      bermudanSteps: 4,
    });
    // 4 quarterly dates k/4·T over (0, T]; T ≈ 1.0 year fraction for a 1Y tenor.
    const T = i.expiryYears;
    const product = i.product;
    if (product.kind !== "american") throw new Error("expected american product");
    expect(product.american.exerciseStyle).toBe("BERMUDAN");
    expect(product.american.bermudanDates).toEqual([
      (T * 1) / 4,
      (T * 2) / 4,
      (T * 3) / 4,
      (T * 4) / 4,
    ]);
    // The final date is exactly expiry (always exercisable).
    expect(product.american.bermudanDates.at(-1)).toBe(T);
    const wire = instrumentToWire(i);
    expect(wire["american"]).toMatchObject({
      option_type: 0, // CALL
      strike: 1.05,
      exercise_style: 1, // BERMUDAN
      bermudan_dates: [(T * 1) / 4, (T * 2) / 4, (T * 3) / 4, T],
    });
  });

  it("a positive step count selects BERMUDAN even when the style is omitted (CLI parity)", () => {
    const i = shapeAmerican({
      pair: "EURUSD",
      tenor: "6M",
      strike: 1.1,
      callPut: "P",
      notional: 1e6,
      bermudanSteps: 2,
    });
    const product = i.product;
    if (product.kind !== "american") throw new Error("expected american product");
    expect(product.american.exerciseStyle).toBe("BERMUDAN");
    expect(product.american.bermudanDates).toHaveLength(2);
  });

  it("selects the LSM Monte-Carlo engine for lsmPaths > 0 and carries its knobs", () => {
    const i = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "P",
      notional: 1e6,
      lsmPaths: 50000,
      lsmExerciseDates: 50,
      lsmSeed: 7,
    });
    const product = i.product;
    if (product.kind !== "american") throw new Error("expected american product");
    expect(americanIsMonteCarlo(product.american)).toBe(true);
    const wire = instrumentToWire(i);
    expect(wire["american"]).toMatchObject({
      lsm_paths: 50000,
      lsm_exercise_dates: 50,
      lsm_seed: 7,
    });
  });

  it("the exact FD engine (lsmPaths == 0) is not Monte-Carlo", () => {
    const i = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "P",
      notional: 1e6,
    });
    const product = i.product;
    if (product.kind !== "american") throw new Error("expected american product");
    expect(americanIsMonteCarlo(product.american)).toBe(false);
  });

  it("rejects a delta strike, a BERMUDAN without dates, a contradictory style+steps, and a bad step count", () => {
    const base = {
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "P",
      notional: 1e6,
    } as const;
    // A delta strike is rejected — an American is struck at an absolute level.
    expect(() => shapeAmerican({ ...base, strike: "25dP" })).toThrow(ShapingError);
    // BERMUDAN with no exercise dates is a domain error (mirrors the server).
    expect(() => shapeAmerican({ ...base, strike: 1.1, style: "BERMUDAN" })).toThrow(ShapingError);
    expect(() =>
      shapeAmerican({ ...base, strike: 1.1, style: "BERMUDAN", bermudanSteps: 0 }),
    ).toThrow(ShapingError);
    // An explicit AMERICAN with a positive step count is contradictory.
    expect(() =>
      shapeAmerican({ ...base, strike: 1.1, style: "AMERICAN", bermudanSteps: 4 }),
    ).toThrow(ShapingError);
    // A non-integer / negative step count is rejected.
    expect(() => shapeAmerican({ ...base, strike: 1.1, bermudanSteps: 2.5 })).toThrow(ShapingError);
    expect(() => shapeAmerican({ ...base, strike: 1.1, bermudanSteps: -1 })).toThrow(ShapingError);
  });

  it("distinguishes AMERICAN vs BERMUDAN vs FD-vs-LSM in the coalescing key", () => {
    const amerFd = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "P",
      notional: 1e6,
    });
    const berm = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "P",
      notional: 1e6,
      bermudanSteps: 4,
    });
    const amerLsm = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.1,
      callPut: "P",
      notional: 1e6,
      lsmPaths: 50000,
      lsmSeed: 7,
    });
    const keys = new Set([
      subscriptionKey(amerFd, DEFAULT_CONVENTIONS),
      subscriptionKey(berm, DEFAULT_CONVENTIONS),
      subscriptionKey(amerLsm, DEFAULT_CONVENTIONS),
    ]);
    expect(keys.size).toBe(3);
    // An American is not a (FLOATING) lookback — different product key entirely.
    const lb = shapeLookback({
      pair: "EURUSD",
      tenor: "1Y",
      callPut: "P",
      notional: 1e6,
    });
    expect(subscriptionKey(amerFd, DEFAULT_CONVENTIONS)).not.toBe(
      subscriptionKey(lb, DEFAULT_CONVENTIONS),
    );
  });
});

// ---------------------------------------------------------------------------
// spill formatting — exact FD carries no std-error; LSM carries one
// ---------------------------------------------------------------------------

const SAMPLE_GREEKS: Greeks = {
  price: 2.3195,
  deltaSpot: -0.43,
  deltaForward: -0.42,
  gamma: 0.061,
  vega: 15.8,
  theta: -2.1,
  rhoDom: -18.0,
  rhoFor: 17.5,
  vanna: 0.21,
  volga: 0.07,
  charm: 0.004,
  speed: -0.0009,
  zomma: 0.0012,
  color: -0.00003,
};

describe("american spill formatting", () => {
  it("the exact FD route spills premium + 13 Greeks + footer (NO std-error row)", () => {
    const m = formatPathDependentSpill({
      premium: SAMPLE_GREEKS.price,
      stdError: undefined,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 3n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 2.3195]);
    expect(m.length).toBe(15); // 1 premium + 13 Greeks + 1 footer
    expect(m.some((r) => r[0] === "std_error")).toBe(false);
  });

  it("the Longstaff-Schwartz route inserts an honest std-error row after the premium", () => {
    const m = formatPathDependentSpill({
      premium: SAMPLE_GREEKS.price,
      stdError: 9e-3,
      greeks: SAMPLE_GREEKS,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 3n,
      epochNanos: 0n,
    });
    expect(m[0]).toEqual(["premium", 2.3195]);
    expect(m[1]).toEqual(["std_error", 9e-3]);
    expect(m.length).toBe(16);
  });
});

// ---------------------------------------------------------------------------
// end-to-end through the WS transport (FakeSocket) — request shape + render
// ---------------------------------------------------------------------------

/** A controllable in-memory socket (mirrors test/pricingModelWindowBarrier.test.ts). */
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

describe("american end-to-end over the WS mirror", () => {
  it("sends an `american` oneof (field 24) on the exact FD route and renders the pinned reference (no std-error)", async () => {
    // Longstaff & Schwartz (2001) Table 1, row 1: American PUT, S0=K=40, r=0.06,
    // sigma=0.20, T=1, no dividend — their FD reference 2.314 (the server's PSOR FD
    // gives 2.3195). The Excel cell renders whatever the server prices; pinning the
    // reply to the published reference documents the SAME oracle the Rust gate uses.
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 40,
      callPut: "P",
      notional: 1e6,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "american-fd");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<
      string,
      unknown
    >;
    expect(wireInstr["american"]).toMatchObject({
      option_type: 1, // PUT
      strike: 40,
      exercise_style: 0, // AMERICAN
      bermudan_dates: [],
      lsm_paths: 0, // exact FD
    });
    // The exact FD engine reports no std-error.
    replyQuote(sock, { greeks: { price: 2.3195, delta_spot: -0.43 }, resolved_strike: 40 });
    const quote = await p;
    expect(quote.priceStdError).toBeUndefined();
    const isMc =
      instr.product.kind === "american" &&
      americanIsMonteCarlo((instr.product as { american: AmericanOption }).american);
    const m = formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 2.3195]);
    // 2.3195 is within 1e-2 of the published 2.314 FD reference (the Rust gate's bound).
    expect(Math.abs(quote.greeks.price - 2.314)).toBeLessThan(1e-2);
    expect(m.some((r) => r[0] === "std_error")).toBe(false);
  });

  it("sends lsm_paths>0 on the Longstaff-Schwartz route and surfaces the server's std-error honestly", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 40,
      callPut: "P",
      notional: 1e6,
      lsmPaths: 100000,
      lsmExerciseDates: 50,
      lsmSeed: 42,
    });
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "american-lsm");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<
      string,
      unknown
    >;
    expect(wireInstr["american"]).toMatchObject({
      lsm_paths: 100000,
      lsm_exercise_dates: 50,
      lsm_seed: 42,
    });
    // The LSM engine returns a price_std_error — surfaced as the std_error row.
    replyQuote(sock, { greeks: { price: 2.313 }, resolved_strike: 40, price_std_error: 9e-3 });
    const quote = await p;
    expect(quote.priceStdError).toBe(9e-3);
    const isMc =
      instr.product.kind === "american" &&
      americanIsMonteCarlo((instr.product as { american: AmericanOption }).american);
    const m = formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    expect(m[0]).toEqual(["premium", 2.313]);
    expect(m[1]).toEqual(["std_error", 9e-3]);
  });

  it("sends a BERMUDAN `american` oneof with the k/n·T dates over the wire", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeAmerican({
      pair: "EURUSD",
      tenor: "1Y",
      strike: 1.05,
      callPut: "C",
      notional: 1e6,
      bermudanSteps: 4,
    });
    const T = instr.expiryYears;
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "bermudan");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<
      string,
      unknown
    >;
    expect(wireInstr["american"]).toMatchObject({
      exercise_style: 1, // BERMUDAN
      bermudan_dates: [(T * 1) / 4, (T * 2) / 4, (T * 3) / 4, T],
    });
    replyQuote(sock, { greeks: { price: 0.031 }, resolved_strike: 1.05 });
    const quote = await p;
    expect(quote.greeks.price).toBe(0.031);
  });
});
