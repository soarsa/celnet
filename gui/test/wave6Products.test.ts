/**
 * Wave-6 booking-model selector + window-barrier parity for the GUI end of the ONE
 * `celnet.wire` contract. Track B-rust added a pricing/booking-model selector on the
 * `Instrument` (proto field 22, `pricing_model`) ALONGSIDE the existing `solve`
 * directive — so one field reaches every flow (price/quote/stream/scenario) — plus a
 * new `window_barrier` product arm (proto field 23) that is LOCAL_STOCH_VOL-only (it
 * has no closed form). Everything is APPENDED additively: no `schema_version`, no
 * renumber, the DEFAULT (analytic) path stays byte-identical (CLAUDE.md rule 9).
 *
 * These exercise the REAL `src/data/wsCodec.ts`, `src/data/enums.ts`,
 * `src/data/seed.ts` and `src/data/pricing.ts` through their public surface with NO
 * server and NO mocks. The numerics claim is deliberately NARROW and HONEST: the GUI
 * does NOT price the LSV/window-barrier products offline (the LSV engine is
 * server-side), so there is nothing to fake — the offline pricer FAILS LOUDLY for a
 * window barrier rather than inventing a value, and the wire encoding is validated
 * against the authoritative field names/numbers + enum tags the server WS codec
 * decodes by (the INDEPENDENT oracle here is the proto contract's pinned tags).
 */
import { describe, expect, it } from "vitest";

import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import {
  bookingModelsFor,
  singleBarrierInstrument,
  vanillaInstrument,
  windowBarrierInstrument,
  type WindowBarrierTerms,
} from "../src/data/seed";
import { priceInstrument } from "../src/data/pricing";
import type { CcyPair, Instrument, MarketContext, StrikeOrDelta } from "../src/data/contract";

const PAIR: CcyPair = { base: "EUR", quote: "USD" };
const MKT: MarketContext = { spot: 1.1, vol: 0.2, rDom: 0.03, rFor: 0.01 };
const EXPIRY = 1;

function strikeLevel(strike: number): StrikeOrDelta {
  return { kind: "strike", strike };
}

function windowBarrier(over: Partial<WindowBarrierTerms> = {}): WindowBarrierTerms {
  return {
    optionType: "CALL",
    strike: strikeLevel(1.1),
    barrier: 1.3,
    side: "UP",
    windowStart: 0.25,
    windowEnd: 0.75,
    mcPairs: 8000,
    mcSteps: 64,
    mcSeed: 42n,
    ...over,
  };
}

// ---------------------------------------------------------------------------
// canonical proto tags — the INDEPENDENT oracle for the wire encoding
// ---------------------------------------------------------------------------

describe("wave-6 PricingModel enum — pinned to the canonical proto numbers", () => {
  it("maps DEFAULT=0 / LOCAL_STOCH_VOL=1 reversibly", () => {
    // The handoff's authoritative tags: PRICING_MODEL_DEFAULT=0,
    // PRICING_MODEL_LOCAL_STOCH_VOL=1. Hand-stated literals (not derived from the
    // codec) so a mis-stated enum cannot pass against itself (Lesson c).
    expect(e.pricingModel.toWire("DEFAULT")).toBe(0);
    expect(e.pricingModel.toWire("LOCAL_STOCH_VOL")).toBe(1);
    expect(e.pricingModel.fromWire(0)).toBe("DEFAULT");
    expect(e.pricingModel.fromWire(1)).toBe("LOCAL_STOCH_VOL");
    // An unknown tag clamps to the proto3 zero value (DEFAULT), like a proto reader.
    expect(e.pricingModel.fromWire(99)).toBe("DEFAULT");
  });
});

// ---------------------------------------------------------------------------
// pricing_model on the Instrument (proto field 22) — additive, DEFAULT byte-identical
// ---------------------------------------------------------------------------

describe("wave-6 pricing_model — encoded on the Instrument, DEFAULT omitted", () => {
  it("omits pricing_model entirely for a DEFAULT (analytic) vanilla — byte-identical", () => {
    const w = instrumentToWire(vanillaInstrument(PAIR, EXPIRY, "CALL", 0.25, 10));
    // Proto3 default-0 is presence-omitted ⇒ an analytic frame carries NO
    // pricing_model key, exactly like the legacy frame before Wave 6.
    expect(w["pricing_model"]).toBeUndefined();
    expect("pricing_model" in w).toBe(false);
    // The product arm is unchanged.
    expect(w["vanilla"]).toBeDefined();
  });

  it("emits pricing_model=1 when LOCAL_STOCH_VOL is selected on a supported product", () => {
    const inst: Instrument = {
      ...singleBarrierInstrument(PAIR, EXPIRY, 10, {
        optionType: "CALL",
        strike: strikeLevel(1.1),
        kind: "KNOCK_OUT",
        side: "UP",
        barrier: 1.2,
        rebate: 0,
        monitoring: "CONTINUOUS",
      }),
      pricingModel: "LOCAL_STOCH_VOL",
    };
    const w = instrumentToWire(inst);
    expect(w["pricing_model"]).toBe(e.pricingModel.toWire("LOCAL_STOCH_VOL"));
    expect(w["pricing_model"]).toBe(1);
    // It lives at the TOP LEVEL of the instrument (alongside solve), NOT inside the
    // product body — so it reaches every flow uniformly, exactly like the proto.
    const body = w["single_barrier"] as Record<string, unknown>;
    expect(body["pricing_model"]).toBeUndefined();
  });

  it("an explicit DEFAULT pricing_model is still omitted (no spurious zero)", () => {
    const inst: Instrument = {
      ...vanillaInstrument(PAIR, EXPIRY, "CALL", 0.25, 10),
      pricingModel: "DEFAULT",
    };
    expect(instrumentToWire(inst)["pricing_model"]).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// window_barrier product arm (proto field 23) — LSV-only, exact wire shape
// ---------------------------------------------------------------------------

describe("wave-6 window_barrier — instrumentToWire oneof arm (field 23)", () => {
  it("the seed builder pre-selects LOCAL_STOCH_VOL (no closed form)", () => {
    const inst = windowBarrierInstrument(PAIR, EXPIRY, 10, windowBarrier());
    expect(inst.pricingModel).toBe("LOCAL_STOCH_VOL");
    expect(inst.product.kind).toBe("windowBarrier");
  });

  it("encodes a window barrier under `window_barrier` with the nested vanilla + window", () => {
    const w = instrumentToWire(
      windowBarrierInstrument(
        PAIR,
        EXPIRY,
        10,
        windowBarrier({ side: "UP", barrier: 1.3, windowStart: 0.25, windowEnd: 0.75 }),
      ),
    );
    const body = w["window_barrier"] as Record<string, unknown>;
    const vanilla = body["vanilla"] as Record<string, unknown>;
    // The window barrier REUSES the SAME nested vanilla message as the single/double
    // barrier (option_type + strike-or-delta).
    expect(vanilla["option_type"]).toBe(e.optionType.toWire("CALL"));
    expect(vanilla["strike"]).toEqual({ strike: 1.1 });
    // The exact server WS codec field NAMES (window_barrier_from_json:
    // vanilla=1, barrier=2, side=3, window_start=4, window_end=5, mc_pairs=6,
    // mc_steps=7, mc_seed=8).
    expect(body["barrier"]).toBe(1.3);
    expect(body["side"]).toBe(e.barrierSide.toWire("UP"));
    expect(body["window_start"]).toBe(0.25);
    expect(body["window_end"]).toBe(0.75);
    expect(body["mc_pairs"]).toBe(8000);
    expect(body["mc_steps"]).toBe(64);
    expect(body["mc_seed"]).toBe(42n);
    // A LSV window barrier carries pricing_model=1 at the top level.
    expect(w["pricing_model"]).toBe(e.pricingModel.toWire("LOCAL_STOCH_VOL"));
    // A oneof carries exactly one body (no other product arm leaks in).
    expect(w["single_barrier"]).toBeUndefined();
    expect(w["vanilla"]).toBeUndefined();
  });

  it("mc_pairs = 0 selects the exact PDE (still emits the field; the server reads 0 as PDE)", () => {
    const w = instrumentToWire(
      windowBarrierInstrument(PAIR, EXPIRY, 10, windowBarrier({ mcPairs: 0 })),
    );
    const body = w["window_barrier"] as Record<string, unknown>;
    expect(body["mc_pairs"]).toBe(0);
  });

  it("a window barrier round-trips a delta-strike in the nested vanilla", () => {
    const w = instrumentToWire(
      windowBarrierInstrument(
        PAIR,
        EXPIRY,
        10,
        windowBarrier({ strike: { kind: "delta", delta: 0.25 } }),
      ),
    );
    const body = w["window_barrier"] as Record<string, unknown>;
    expect((body["vanilla"] as Record<string, unknown>)["strike"]).toEqual({ delta: 0.25 });
  });
});

// ---------------------------------------------------------------------------
// booking-model gating — which products support which models
// ---------------------------------------------------------------------------

describe("wave-6 bookingModelsFor — the supported booking models per product", () => {
  it("vanilla + single barrier support DEFAULT and LOCAL_STOCH_VOL", () => {
    expect(bookingModelsFor("vanilla")).toEqual(["DEFAULT", "LOCAL_STOCH_VOL"]);
    expect(bookingModelsFor("singleBarrier")).toEqual(["DEFAULT", "LOCAL_STOCH_VOL"]);
  });

  it("the window barrier supports ONLY LOCAL_STOCH_VOL (no closed form)", () => {
    expect(bookingModelsFor("windowBarrier")).toEqual(["LOCAL_STOCH_VOL"]);
  });

  it("the LSV-unsupported products support ONLY the DEFAULT model", () => {
    // Selecting LSV for these would be rejected server-side with UnsupportedModel —
    // the GUI never offers it (the selector only shows DEFAULT, so it is never sent).
    for (const kind of [
      "strategy",
      "doubleBarrier",
      "digital",
      "touch",
      "varianceSwap",
      "volatilitySwap",
      "asianOption",
      "forwardStart",
      "cliquet",
      "quanto",
      "tarf",
      "accumulator",
      "lookback",
    ] as const) {
      expect(bookingModelsFor(kind)).toEqual(["DEFAULT"]);
    }
  });
});

// ---------------------------------------------------------------------------
// offline honesty — the window barrier is NOT priced offline (no faked LSV)
// ---------------------------------------------------------------------------

describe("wave-6 offline pricer — never fabricates an LSV window-barrier price", () => {
  it("priceInstrument fails LOUDLY for a window barrier (server-side only)", () => {
    const inst = windowBarrierInstrument(PAIR, EXPIRY, 10, windowBarrier());
    // The offline mock has no LSV engine; rather than invent a value (which would
    // be a fake — CLAUDE.md: no mocks/placeholders/overclaim), the offline pricer
    // throws. The TicketWorkspace gates the window barrier to the live transport so
    // this is never reached in the UI, but the contract here is "no fabrication".
    expect(() => priceInstrument(inst, MKT)).toThrow(/server-side only|LOCAL_STOCH_VOL/);
  });

  it("a DEFAULT vanilla still prices offline unchanged (the analytic path is intact)", () => {
    // Selecting/leaving DEFAULT does not change the offline analytic price — the
    // pricing_model is a transport-level routing field, not a pricing input here.
    const plain = priceInstrument(vanillaInstrument(PAIR, EXPIRY, "CALL", 0.25, 10), MKT);
    const tagged = priceInstrument(
      { ...vanillaInstrument(PAIR, EXPIRY, "CALL", 0.25, 10), pricingModel: "DEFAULT" },
      MKT,
    );
    expect(tagged.greeks.price).toBe(plain.greeks.price);
  });
});
