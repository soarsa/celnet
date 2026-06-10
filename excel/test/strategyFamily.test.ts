// Excel client parity for the multi-leg vol STRATEGY family (proto `Strategy`,
// product field 8) — the most-traded OTC FX structures (risk reversal /
// straddle / strangle / seagull), expressed as `CELNET.INSTRUMENT` repeated
// ("legs", callPut, strike, side, ratio?) terms rows exactly like BASKET's
// matrix keys. These tests prove the add-in shapes each template into the
// EXACT wire frame the server's `strategy_from_json` decodes (HAND-BUILT
// expected frames — strategy never had a per-product worksheet function, so
// the parity proof is spec-path == hand-built frame, the strongest form), that
// the ladders mirror the GUI's fixed convention-delta templates
// (`gui/src/data/seed.ts strategyLegs`) leg-for-leg, and that the grammar
// rejects malformed input with typed errors naming the offending key/cell. No
// pricing math lives in the add-in: the server prices the signed `side·ratio`
// leg sum (`crates/celnet-server/src/pricer.rs`), gated against the
// independent GK leg-sum oracle by the frozen `strategy.json` vectors.

import { describe, expect, it } from "vitest";

import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  parseStrategyKind,
  shapeStrategy,
  subscriptionKey,
} from "../src/functions/shaping";
import {
  decodeInstrumentToken,
  encodeInstrumentToken,
  shapeSpecInstrument,
} from "../src/functions/instrumentSpec";
import { instrumentToWire } from "../src/contract/wsCodec";
import { instrumentFromWire } from "../src/contract/instrumentCodec";
import * as e from "../src/contract/enums";

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

describe("strategy-kind parsing", () => {
  it("parses the canonical names separator-insensitively plus the RR shorthand", () => {
    expect(parseStrategyKind("RISK_REVERSAL")).toBe("RISK_REVERSAL");
    expect(parseStrategyKind("risk reversal")).toBe("RISK_REVERSAL");
    expect(parseStrategyKind("RiskReversal")).toBe("RISK_REVERSAL");
    expect(parseStrategyKind("rr")).toBe("RISK_REVERSAL");
    expect(parseStrategyKind("STRADDLE")).toBe("STRADDLE");
    expect(parseStrategyKind("strangle")).toBe("STRANGLE");
    expect(parseStrategyKind("Seagull")).toBe("SEAGULL");
  });

  it("rejects an unknown kind with a typed error listing the templates", () => {
    expect(() => parseStrategyKind("BUTTERFLY")).toThrow(ShapingError);
    expect(() => parseStrategyKind("BUTTERFLY")).toThrow(
      /RISK_REVERSAL, STRADDLE, STRANGLE, SEAGULL/,
    );
  });
});

// ---------------------------------------------------------------------------
// shaping → wire encoding (HAND-BUILT server frames)
// ---------------------------------------------------------------------------

describe("strategy shaping + wire encoding", () => {
  it("encodes the GUI risk-reversal template (long 25dC / short 25dP) as the HAND-BUILT frame", () => {
    const i = shapeStrategy({
      pair: "EURUSD",
      tenor: "1Y",
      notional: 1e7,
      kind: "RISK_REVERSAL",
      legs: [
        ["C", "25dC", "BUY", 1],
        ["P", "25dP", "SELL", 1],
      ],
    });
    // Field-for-field the frame the server's `strategy_from_json` decodes —
    // and leg-for-leg the GUI's `strategyLegs("RISK_REVERSAL")` template.
    expect(instrumentToWire(i)).toEqual({
      pair: { base: "EUR", quote: "USD" },
      tenor: { unit: e.tenorUnit.toWire("YEARS"), count: 1 },
      expiry_years: 1,
      quantity: { notional: 1e7, base_ccy: true },
      side: e.side.toWire("TWO_WAY"),
      strategy: {
        kind: e.strategyKind.toWire("RISK_REVERSAL"), // RISK_REVERSAL = 0
        legs: [
          { option_type: 0, strike: { delta: 0.25 }, side: 0, ratio: 1 },
          { option_type: 1, strike: { delta: -0.25 }, side: 1, ratio: 1 },
        ],
      },
    });
  });

  it("encodes the straddle with ATM/DNS delta-neutral legs and a defaulted ratio", () => {
    const i = shapeStrategy({
      pair: "USDJPY",
      tenor: "3M",
      notional: 5e6,
      kind: "STRADDLE",
      // 3-cell rows: the ratio defaults to the unit leg (proto's 1.0).
      legs: [
        ["C", "ATM", "BUY"],
        ["P", "DNS", "BUY"],
      ],
    });
    const wire = instrumentToWire(i);
    expect(wire["strategy"]).toEqual({
      kind: e.strategyKind.toWire("STRADDLE"), // STRADDLE = 2
      legs: [
        { option_type: 0, strike: { delta: 0 }, side: 0, ratio: 1 },
        { option_type: 1, strike: { delta: 0 }, side: 0, ratio: 1 },
      ],
    });
    // Exactly one product arm is present (oneof discipline).
    expect("vanilla" in wire).toBe(false);
    expect("basket" in wire).toBe(false);
  });

  it("encodes the 3-leg seagull leg-for-leg as the GUI template ladder", () => {
    const i = shapeStrategy({
      pair: "EURUSD",
      tenor: "6M",
      notional: 1.5e7,
      kind: "SEAGULL",
      legs: [
        ["C", "25dC", "BUY", 1],
        ["C", "10dC", "SELL", 1],
        ["P", "25dP", "SELL", 1],
      ],
    });
    const wire = instrumentToWire(i);
    expect(wire["strategy"]).toEqual({
      kind: e.strategyKind.toWire("SEAGULL"), // SEAGULL = 3
      legs: [
        { option_type: 0, strike: { delta: 0.25 }, side: 0, ratio: 1 },
        { option_type: 0, strike: { delta: 0.1 }, side: 1, ratio: 1 },
        { option_type: 1, strike: { delta: -0.25 }, side: 1, ratio: 1 },
      ],
    });
  });

  it("carries absolute strike levels and a non-unit (1x2) ratio onto the wire", () => {
    const i = shapeStrategy({
      pair: "GBPUSD",
      tenor: "1Y",
      notional: 1e6,
      kind: "RR",
      legs: [
        ["C", 1.35, "BUY", 1],
        ["P", 1.15, "SELL", 2],
      ],
    });
    const wire = instrumentToWire(i);
    expect(wire["strategy"]).toEqual({
      kind: 0,
      legs: [
        { option_type: 0, strike: { strike: 1.35 }, side: 0, ratio: 1 },
        { option_type: 1, strike: { strike: 1.15 }, side: 1, ratio: 2 },
      ],
    });
  });

  it("decodes the server's wire frame back to the typed instrument (codec inverse)", () => {
    const decoded = instrumentFromWire({
      pair: { base: "EUR", quote: "USD" },
      tenor: { unit: 3, count: 1 },
      expiry_years: 1,
      quantity: { notional: 1e7, base_ccy: true },
      side: 2,
      strategy: {
        kind: 0,
        legs: [
          { option_type: 0, strike: { delta: 0.25 }, side: 0, ratio: 1 },
          { option_type: 1, strike: { delta: -0.25 }, side: 1, ratio: 1 },
        ],
      },
    });
    expect(decoded.product).toEqual({
      kind: "strategy",
      strategy: {
        kind: "RISK_REVERSAL",
        legs: [
          { optionType: "CALL", strike: { kind: "delta", delta: 0.25 }, side: "BUY", ratio: 1 },
          { optionType: "PUT", strike: { kind: "delta", delta: -0.25 }, side: "SELL", ratio: 1 },
        ],
      },
    });
  });

  it("rejects a mislabeled ladder, malformed legs and bad cells with typed errors", () => {
    const rr: (string | number)[][] = [
      ["C", "25dC", "BUY", 1],
      ["P", "25dP", "SELL", 1],
    ];
    const base = { pair: "EURUSD", tenor: "1Y", notional: 1e6 } as const;
    // A straddle is a 2-leg template; 3 legs is a mislabeled structure.
    expect(() =>
      shapeStrategy({ ...base, kind: "STRADDLE", legs: [...rr, ["C", "10dC", "SELL", 1]] }),
    ).toThrow(/STRADDLE books exactly 2 legs/);
    // A seagull books 3 legs; 2 is a mislabeled structure.
    expect(() => shapeStrategy({ ...base, kind: "SEAGULL", legs: rr })).toThrow(
      /SEAGULL books exactly 3 legs/,
    );
    // A short leg row names the expected cells.
    expect(() =>
      shapeStrategy({ ...base, kind: "RR", legs: [["C", "25dC"], ["P", "25dP", "SELL", 1]] }),
    ).toThrow(/leg 1 must be \[callPut, strike, side, ratio\?\]/);
    // Bad cells: call/put, strike/delta, side, ratio — each a typed error.
    expect(() =>
      shapeStrategy({ ...base, kind: "RR", legs: [["X", "25dC", "BUY", 1], rr[1]!] }),
    ).toThrow(/invalid call\/put/);
    expect(() =>
      shapeStrategy({ ...base, kind: "RR", legs: [["C", "25xC", "BUY", 1], rr[1]!] }),
    ).toThrow(/invalid strike\/delta/);
    expect(() =>
      shapeStrategy({ ...base, kind: "RR", legs: [["C", "25dC", "HOLD", 1], rr[1]!] }),
    ).toThrow(/invalid side/);
    expect(() =>
      shapeStrategy({ ...base, kind: "RR", legs: [["C", "25dC", "BUY", -1], rr[1]!] }),
    ).toThrow(/ratio.*must be a positive number/);
    // A non-positive notional is rejected exactly like every other family.
    expect(() => shapeStrategy({ ...base, notional: 0, kind: "RR", legs: rr })).toThrow(
      ShapingError,
    );
  });
});

// ---------------------------------------------------------------------------
// the polymorphic CELNET.INSTRUMENT dispatch + the opaque token
// ---------------------------------------------------------------------------

/** The risk-reversal terms rows a trader would type (kind via a term). */
const RR_TERMS: (string | number)[][] = [
  ["kind", "RISK_REVERSAL"],
  ["legs", "C", "25dC", "BUY", 1],
  ["legs", "P", "25dP", "SELL", 1],
];

describe("INSTRUMENT family dispatch: STRATEGY + the template-named families", () => {
  it("shapes the generic STRATEGY family from a (\"kind\", …) term + repeated legs rows", () => {
    const spec = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "STRATEGY",
      terms: RR_TERMS,
      tenor: "1Y",
      notional: 1e7,
    });
    expect(spec).toEqual(
      shapeStrategy({
        pair: "EURUSD",
        tenor: "1Y",
        notional: 1e7,
        kind: "RISK_REVERSAL",
        legs: [
          ["C", "25dC", "BUY", 1],
          ["P", "25dP", "SELL", 1],
        ],
      }),
    );
  });

  it("answers to the proto-arm name `strategy` verbatim", () => {
    const byArm = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "strategy",
      terms: RR_TERMS,
      tenor: "1Y",
    });
    const byName = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "STRATEGY",
      terms: RR_TERMS,
      tenor: "1Y",
    });
    expect(encodeInstrumentToken(byArm)).toBe(encodeInstrumentToken(byName));
  });

  it("binds the kind from each template-named product (no kind term needed)", () => {
    const legsOnly = RR_TERMS.slice(1);
    const byTemplate = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "RISK_REVERSAL",
      terms: legsOnly,
      tenor: "1Y",
    });
    const byGeneric = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "STRATEGY",
      terms: RR_TERMS,
      tenor: "1Y",
    });
    expect(encodeInstrumentToken(byTemplate)).toBe(encodeInstrumentToken(byGeneric));
    // Every template name dispatches (with its own template's ladder).
    for (const [product, legs] of [
      ["STRADDLE", [["legs", "C", "ATM", "BUY"], ["legs", "P", "ATM", "BUY"]]],
      ["STRANGLE", [["legs", "C", "10dC", "BUY"], ["legs", "P", "10dP", "BUY"]]],
      [
        "SEAGULL",
        [
          ["legs", "C", "25dC", "BUY"],
          ["legs", "C", "10dC", "SELL"],
          ["legs", "P", "25dP", "SELL"],
        ],
      ],
    ] as const) {
      const spec = shapeSpecInstrument({
        underlier: "EURUSD",
        product,
        terms: legs,
        tenor: "1Y",
      });
      expect(spec.product.kind).toBe("strategy");
      if (spec.product.kind === "strategy") {
        expect(spec.product.strategy.kind).toBe(product);
      }
    }
  });

  it("accepts an AGREEING (\"kind\", …) term on a template name and rejects a contradictory one", () => {
    const legsOnly = RR_TERMS.slice(1);
    const restated = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "RISK_REVERSAL",
      terms: [["kind", "RR"], ...legsOnly],
      tenor: "1Y",
    });
    const bound = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "RISK_REVERSAL",
      terms: legsOnly,
      tenor: "1Y",
    });
    expect(encodeInstrumentToken(restated)).toBe(encodeInstrumentToken(bound));
    // A contradictory kind is a typed error — never a silent override.
    expect(() =>
      shapeSpecInstrument({
        underlier: "EURUSD",
        product: "STRADDLE",
        terms: [["kind", "SEAGULL"], ...legsOnly],
        tenor: "1Y",
      }),
    ).toThrow(/STRADDLE binds the strategy kind/);
  });

  it("is terms-order-free: shuffled rows produce the byte-identical token", () => {
    const shuffled = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "STRATEGY",
      // The legs rows keep their relative order (they ARE the ladder order);
      // the scalar kind/tenor/notional rows move freely.
      terms: [
        ["legs", "C", "25dC", "BUY", 1],
        ["notional", 1e7],
        ["legs", "P", "25dP", "SELL", 1],
        ["kind", "RISK_REVERSAL"],
        ["tenor", "1Y"],
      ],
    });
    const ordered = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "STRATEGY",
      terms: RR_TERMS,
      tenor: "1Y",
      notional: 1e7,
    });
    expect(encodeInstrumentToken(shuffled)).toBe(encodeInstrumentToken(ordered));
  });

  it("round-trips the token losslessly (decode → re-encode is byte-identical)", () => {
    const token = encodeInstrumentToken(
      shapeSpecInstrument({
        underlier: "EURUSD",
        product: "SEAGULL",
        terms: [
          ["legs", "C", "25dC", "BUY", 1],
          ["legs", "C", "10dC", "SELL", 1],
          ["legs", "P", "25dP", "SELL", 1],
        ],
        tenor: "6M",
      }),
    );
    expect(encodeInstrumentToken(decodeInstrumentToken(token))).toBe(token);
  });

  it("overlays a non-FX underlier exactly like every other family (metal arm)", () => {
    const spec = shapeSpecInstrument({
      underlier: "XAUUSD",
      product: "STRADDLE",
      terms: [
        ["legs", "C", "ATM", "BUY"],
        ["legs", "P", "ATM", "BUY"],
      ],
      tenor: "3M",
    });
    const wire = instrumentToWire(spec);
    expect(wire["underlying"]).toEqual({
      metal: { metal: e.metal.toWire("GOLD"), quote: "USD" },
      settlement_ccy: "USD",
    });
    const token = encodeInstrumentToken(spec);
    expect(encodeInstrumentToken(decodeInstrumentToken(token))).toBe(token);
  });

  it("requires its terms with typed errors NAMING the gap", () => {
    // No kind (generic family, no template name).
    expect(() =>
      shapeSpecInstrument({
        underlier: "EURUSD",
        product: "STRATEGY",
        terms: RR_TERMS.slice(1),
        tenor: "1Y",
      }),
    ).toThrow(/STRATEGY requires the term `kind`/);
    // No legs rows.
    expect(() =>
      shapeSpecInstrument({
        underlier: "EURUSD",
        product: "STRATEGY",
        terms: [["kind", "RR"]],
        tenor: "1Y",
      }),
    ).toThrow(/requires `legs` rows/);
    // An unknown term names the key and the family's key set.
    expect(() =>
      shapeSpecInstrument({
        underlier: "EURUSD",
        product: "RISK_REVERSAL",
        terms: [...RR_TERMS.slice(1), ["barrier", 1.2]],
        tenor: "1Y",
      }),
    ).toThrow(/`barrier`.*kind, legs/s);
  });

  it("keys same-kind strategies with different ladders onto DISTINCT subscriptions", () => {
    const at25 = shapeStrategy({
      pair: "EURUSD",
      tenor: "1Y",
      notional: 1,
      kind: "RR",
      legs: [
        ["C", "25dC", "BUY", 1],
        ["P", "25dP", "SELL", 1],
      ],
    });
    const at10 = shapeStrategy({
      pair: "EURUSD",
      tenor: "1Y",
      notional: 1,
      kind: "RR",
      legs: [
        ["C", "10dC", "BUY", 1],
        ["P", "10dP", "SELL", 1],
      ],
    });
    expect(subscriptionKey(at25, DEFAULT_CONVENTIONS)).not.toBe(
      subscriptionKey(at10, DEFAULT_CONVENTIONS),
    );
    // Identical ladders coalesce onto one subscription.
    const again = shapeStrategy({
      pair: "EURUSD",
      tenor: "1Y",
      notional: 1,
      kind: "RR",
      legs: [
        ["C", "25dC", "BUY", 1],
        ["P", "25dP", "SELL", 1],
      ],
    });
    expect(subscriptionKey(at25, DEFAULT_CONVENTIONS)).toBe(
      subscriptionKey(again, DEFAULT_CONVENTIONS),
    );
  });
});
