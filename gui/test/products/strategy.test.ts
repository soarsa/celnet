/**
 * Per-family gate (GW2 + round 2) for the vanilla + strategy {@link ProductSpec}s
 * (the leg-ladder family). Each spec's defaults build a wire-valid instrument of
 * the declared kind with the tenor stamped (DEFAULT model presence-omitted), that
 * round-trips deterministically through the real `instrumentToWire` codec, and
 * whose `allowedModels` match the server booking matrix. The template DEFAULTS
 * are pinned to the monolith `vanillaInstrument` / `strategyInstrument`
 * byte-for-byte; round 2 adds the EDITABLE ladder — the strike-entry grammar
 * (level / 25dC / 25dP / ATM, typed errors), the template structure laws (typed
 * violations + honest messages, surfaced through the spec `validate` seam), and
 * the custom builds (custom-strike vanilla call/put, edited strategy legs riding
 * the wire's arbitrary-leg `Strategy`).
 */
import { describe, expect, it } from "vitest";

import {
  riskReversalSpec,
  seagullSpec,
  straddleSpec,
  strangleSpec,
  vanillaSpec,
  type StrategyInputs,
} from "../../src/products/strategy";
import {
  formatStrikeEntry,
  legLawMessage,
  legLawViolations,
  parseStrikeEntry,
  strikeEntryMessage,
  strikeEntryViolations,
  templateLegCount,
  type StrategyLegInputs,
} from "../../src/products/strategyLegEditor";
import {
  bookingModelsFor,
  strategyInstrument,
  tenorYearsToTenor,
  vanillaInstrument,
} from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";
import type { Instrument, StrategyKind } from "../../src/data/contract";
import type { ProductBuildCtx, ProductSpec } from "../../src/products";

const CTX: ProductBuildCtx = {
  pair: { base: "EUR", quote: "USD" },
  tenor: tenorYearsToTenor(0.25),
  tenorYears: 0.25,
  notionalMm: 10,
  pricingModel: "DEFAULT",
  atmForward: 1.105,
  spot: 1.1,
  pipDecimals: 4,
  today: { year: 2026, month: 6, day: 8 },
};

const SPECS: readonly ProductSpec<StrategyInputs>[] = [
  vanillaSpec,
  riskReversalSpec,
  strangleSpec,
  straddleSpec,
  seagullSpec,
];

/** A wire-valid editable leg (shorthand for the law/build cases below). */
function leg(
  optionType: "CALL" | "PUT",
  strike: StrategyLegInputs["strike"],
  side: "BUY" | "SELL",
  ratio = 1,
): StrategyLegInputs {
  return { optionType, strike, side, ratio };
}

describe("vanilla + strategy ProductSpecs (GW2)", () => {
  for (const spec of SPECS) {
    describe(`${spec.id}`, () => {
      it("builds the declared kind with the tenor stamped, DEFAULT model omitted", () => {
        const inst = spec.toInstrument(spec.defaults, CTX);
        expect(inst.product.kind).toBe(spec.kind);
        expect(inst.tenor).toEqual(CTX.tenor);
        expect(inst.pricingModel).toBeUndefined();
      });

      it("round-trips deterministically through the real wire codec", () => {
        const inst = spec.toInstrument(spec.defaults, CTX);
        const wire = instrumentToWire(inst);
        expect(wire).toBeTruthy();
        expect(instrumentToWire(inst)).toEqual(wire);
      });

      it("allowedModels equal bookingModelsFor(kind)", () => {
        expect(spec.allowedModels).toEqual(bookingModelsFor(spec.kind));
      });

      it("sits in the Vanilla & strategies gallery group", () => {
        expect(spec.group).toBe("Vanilla & strategies");
      });

      it("declares the structure law and its defaults are lawful", () => {
        expect(spec.validate).toBeDefined();
        expect(spec.validate!(spec.defaults, CTX)).toEqual([]);
      });
    });
  }

  it("reproduces the monolith buildInstrument output byte-for-byte", () => {
    const expected: Record<string, Instrument> = {
      VANILLA: vanillaInstrument(CTX.pair, CTX.tenorYears, "CALL", 0.25, CTX.notionalMm),
      RISK_REVERSAL: strategyInstrument(CTX.pair, CTX.tenorYears, "RISK_REVERSAL", CTX.notionalMm),
      STRANGLE: strategyInstrument(CTX.pair, CTX.tenorYears, "STRANGLE", CTX.notionalMm),
      STRADDLE: strategyInstrument(CTX.pair, CTX.tenorYears, "STRADDLE", CTX.notionalMm),
      SEAGULL: strategyInstrument(CTX.pair, CTX.tenorYears, "SEAGULL", CTX.notionalMm),
    };
    for (const spec of SPECS) {
      // The legacy tail stamps the trader tenor; DEFAULT is presence-omitted so
      // the analytic frame is byte-identical to the bare builder + tenor stamp.
      const monolith: Instrument = { ...expected[spec.id]!, tenor: CTX.tenor };
      expect(spec.toInstrument(spec.defaults, CTX)).toEqual(monolith);
    }
  });

  it("VANILLA books a vanilla; the four strategies book a strategy of the matching kind", () => {
    expect(vanillaSpec.kind).toBe("vanilla");
    const strategyKinds: Record<string, StrategyKind> = {
      RISK_REVERSAL: "RISK_REVERSAL",
      STRANGLE: "STRANGLE",
      STRADDLE: "STRADDLE",
      SEAGULL: "SEAGULL",
    };
    for (const spec of [riskReversalSpec, strangleSpec, straddleSpec, seagullSpec]) {
      expect(spec.kind).toBe("strategy");
      const inst = spec.toInstrument(spec.defaults, CTX);
      if (inst.product.kind !== "strategy") throw new Error("expected a strategy product");
      expect(inst.product.strategy.kind).toBe(strategyKinds[spec.id]);
    }
  });
});

describe("strike-entry grammar (the inline strike solve vocabulary)", () => {
  it("parses an absolute level", () => {
    expect(parseStrikeEntry("1.0850", "CALL")).toEqual({
      ok: true,
      strike: { kind: "strike", strike: 1.085 },
    });
  });

  it("parses convention deltas with the platform sign discipline (call +, put −)", () => {
    expect(parseStrikeEntry("25dC", "CALL")).toEqual({
      ok: true,
      strike: { kind: "delta", delta: 0.25 },
    });
    expect(parseStrikeEntry("25dP", "PUT")).toEqual({
      ok: true,
      strike: { kind: "delta", delta: -0.25 },
    });
    // Case-insensitive, like the rest of the platform vocabulary.
    expect(parseStrikeEntry("10dp", "PUT")).toEqual({
      ok: true,
      strike: { kind: "delta", delta: -0.1 },
    });
  });

  it("parses ATM as the signed 50Δ pillar of the leg's type", () => {
    expect(parseStrikeEntry("ATM", "CALL")).toEqual({
      ok: true,
      strike: { kind: "delta", delta: 0.5 },
    });
    expect(parseStrikeEntry("atm", "PUT")).toEqual({
      ok: true,
      strike: { kind: "delta", delta: -0.5 },
    });
  });

  it("returns typed, actionable errors for every bad entry", () => {
    const cases: { raw: string; optionType: "CALL" | "PUT"; kind: string }[] = [
      { raw: "", optionType: "CALL", kind: "EMPTY" },
      { raw: "banana", optionType: "CALL", kind: "UNRECOGNIZED" },
      { raw: "0", optionType: "CALL", kind: "NON_POSITIVE_LEVEL" },
      { raw: "-1.1", optionType: "CALL", kind: "NON_POSITIVE_LEVEL" },
      { raw: "0dC", optionType: "CALL", kind: "DELTA_OUT_OF_RANGE" },
      { raw: "100dC", optionType: "CALL", kind: "DELTA_OUT_OF_RANGE" },
      { raw: "25dP", optionType: "CALL", kind: "DELTA_LETTER_MISMATCH" },
      { raw: "25dC", optionType: "PUT", kind: "DELTA_LETTER_MISMATCH" },
    ];
    for (const c of cases) {
      const parsed = parseStrikeEntry(c.raw, c.optionType);
      expect(parsed.ok, `\`${c.raw}\` must not parse`).toBe(false);
      if (parsed.ok) continue;
      expect(parsed.error.kind).toBe(c.kind);
      // Every error formats to a non-empty, honest trader message.
      expect(strikeEntryMessage(parsed.error).length).toBeGreaterThan(0);
    }
  });

  it("formats the committed strike canonically and round-trips through the parser", () => {
    expect(formatStrikeEntry({ kind: "strike", strike: 1.085 })).toBe("1.085");
    expect(formatStrikeEntry({ kind: "delta", delta: 0.25 })).toBe("25dC");
    expect(formatStrikeEntry({ kind: "delta", delta: -0.1 })).toBe("10dP");
    expect(formatStrikeEntry({ kind: "delta", delta: 0.5 })).toBe("ATM");
    expect(formatStrikeEntry({ kind: "delta", delta: -0.5 })).toBe("ATM");
    for (const [spec, ot] of [
      [{ kind: "strike", strike: 1.085 }, "CALL"],
      [{ kind: "delta", delta: 0.25 }, "CALL"],
      [{ kind: "delta", delta: -0.1 }, "PUT"],
      [{ kind: "delta", delta: -0.5 }, "PUT"],
    ] as const) {
      expect(parseStrikeEntry(formatStrikeEntry(spec), ot)).toEqual({ ok: true, strike: spec });
    }
  });

  it("flags an unparseable in-progress draft (and only an unparseable one)", () => {
    const clean = [leg("CALL", { kind: "delta", delta: 0.25 }, "BUY")];
    expect(strikeEntryViolations(clean)).toEqual([]);
    const drafting: StrategyLegInputs[] = [{ ...clean[0]!, strikeDraft: "1.08" }];
    expect(strikeEntryViolations(drafting)).toEqual([]);
    const broken: StrategyLegInputs[] = [{ ...clean[0]!, strikeDraft: "banana" }];
    const violations = strikeEntryViolations(broken);
    expect(violations).toHaveLength(1);
    expect(violations[0]!.legIndex).toBe(0);
    expect(violations[0]!.error.kind).toBe("UNRECOGNIZED");
  });
});

describe("template structure laws (typed violations, honest messages)", () => {
  it("each template's required leg count", () => {
    expect(templateLegCount("VANILLA")).toBe(1);
    expect(templateLegCount("RISK_REVERSAL")).toBe(2);
    expect(templateLegCount("STRANGLE")).toBe(2);
    expect(templateLegCount("STRADDLE")).toBe(2);
    expect(templateLegCount("SEAGULL")).toBe(3);
  });

  it("risk reversal: two legs of opposite type, one bought one sold", () => {
    const sameType = [
      leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"),
      leg("CALL", { kind: "delta", delta: 0.1 }, "SELL"),
    ];
    expect(legLawViolations("RISK_REVERSAL", sameType).map((v) => v.law)).toContain(
      "OPPOSITE_TYPES",
    );
    const sameSide = [
      leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"),
      leg("PUT", { kind: "delta", delta: -0.25 }, "BUY"),
    ];
    expect(legLawViolations("RISK_REVERSAL", sameSide).map((v) => v.law)).toContain(
      "OPPOSITE_SIDES",
    );
    const lawful = [
      leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"),
      leg("PUT", { kind: "delta", delta: -0.25 }, "SELL"),
    ];
    expect(legLawViolations("RISK_REVERSAL", lawful)).toEqual([]);
  });

  it("straddle: legs share one strike (same level, or both ATM)", () => {
    const atm = [
      leg("CALL", { kind: "delta", delta: 0.5 }, "BUY"),
      leg("PUT", { kind: "delta", delta: -0.5 }, "BUY"),
    ];
    expect(legLawViolations("STRADDLE", atm)).toEqual([]);
    const sameLevel = [
      leg("CALL", { kind: "strike", strike: 1.1 }, "BUY"),
      leg("PUT", { kind: "strike", strike: 1.1 }, "BUY"),
    ];
    expect(legLawViolations("STRADDLE", sameLevel)).toEqual([]);
    // A 25Δ call and a 25Δ put resolve to two DIFFERENT levels — not a straddle.
    const wings = [
      leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"),
      leg("PUT", { kind: "delta", delta: -0.25 }, "BUY"),
    ];
    expect(legLawViolations("STRADDLE", wings).map((v) => v.law)).toContain("SAME_STRIKE");
  });

  it("strangle: distinct strikes on the same side (one shared strike is a straddle)", () => {
    const shared = [
      leg("CALL", { kind: "strike", strike: 1.1 }, "BUY"),
      leg("PUT", { kind: "strike", strike: 1.1 }, "BUY"),
    ];
    expect(legLawViolations("STRANGLE", shared).map((v) => v.law)).toContain("DISTINCT_STRIKES");
    const mixedSides = [
      leg("CALL", { kind: "delta", delta: 0.1 }, "BUY"),
      leg("PUT", { kind: "delta", delta: -0.1 }, "SELL"),
    ];
    expect(legLawViolations("STRANGLE", mixedSides).map((v) => v.law)).toContain("SAME_SIDE");
  });

  it("seagull: three legs mixing call/put and buy/sell", () => {
    const twoLegs = [
      leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"),
      leg("PUT", { kind: "delta", delta: -0.25 }, "SELL"),
    ];
    const count = legLawViolations("SEAGULL", twoLegs);
    expect(count.map((v) => v.law)).toContain("LEG_COUNT");
    const allCalls = [
      leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"),
      leg("CALL", { kind: "delta", delta: 0.1 }, "SELL"),
      leg("CALL", { kind: "delta", delta: 0.05 }, "SELL"),
    ];
    expect(legLawViolations("SEAGULL", allCalls).map((v) => v.law)).toContain("BOTH_TYPES");
    const allBuys = [
      leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"),
      leg("CALL", { kind: "delta", delta: 0.1 }, "BUY"),
      leg("PUT", { kind: "delta", delta: -0.25 }, "BUY"),
    ];
    expect(legLawViolations("SEAGULL", allBuys).map((v) => v.law)).toContain("BOTH_SIDES");
  });

  it("a non-positive ratio (notional weight) is flagged per leg", () => {
    const legs = [
      leg("CALL", { kind: "delta", delta: 0.25 }, "BUY", 0),
      leg("PUT", { kind: "delta", delta: -0.25 }, "SELL"),
    ];
    const violations = legLawViolations("RISK_REVERSAL", legs);
    expect(violations).toContainEqual({ law: "POSITIVE_RATIO", legIndex: 0 });
  });

  it("every violation formats to a non-empty actionable message", () => {
    const messages = [
      legLawMessage({ law: "LEG_COUNT", template: "SEAGULL", required: 3, actual: 2 }),
      legLawMessage({ law: "OPPOSITE_TYPES", template: "RISK_REVERSAL" }),
      legLawMessage({ law: "OPPOSITE_SIDES", template: "RISK_REVERSAL" }),
      legLawMessage({ law: "SAME_SIDE", template: "STRANGLE" }),
      legLawMessage({ law: "SAME_STRIKE", template: "STRADDLE" }),
      legLawMessage({ law: "DISTINCT_STRIKES", template: "STRANGLE" }),
      legLawMessage({ law: "BOTH_TYPES", template: "SEAGULL" }),
      legLawMessage({ law: "BOTH_SIDES", template: "SEAGULL" }),
      legLawMessage({ law: "POSITIVE_RATIO", legIndex: 1 }),
    ];
    for (const m of messages) expect(m.length).toBeGreaterThan(10);
    expect(messages[0]).toContain("add 1 leg");
    expect(messages[8]).toContain("leg 2");
  });

  it("the spec validate seam surfaces both entry errors and law violations", () => {
    const inputs: StrategyInputs = {
      template: "RISK_REVERSAL",
      legs: [
        { ...leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"), strikeDraft: "nope" },
        leg("PUT", { kind: "delta", delta: -0.25 }, "BUY"), // same side: law violation
      ],
    };
    const messages = riskReversalSpec.validate!(inputs, CTX);
    expect(messages.length).toBe(2);
    expect(messages[0]).toContain("leg 1 strike");
    expect(messages[1]).toContain("buys one leg and sells the other");
  });
});

describe("custom structures ride the wire (the round-2 leg builder)", () => {
  it("builds a custom-strike vanilla PUT at an absolute level", () => {
    const inputs: StrategyInputs = {
      template: "VANILLA",
      legs: [leg("PUT", { kind: "strike", strike: 1.085 }, "BUY")],
    };
    const inst = vanillaSpec.toInstrument(inputs, CTX);
    expect(inst.product).toEqual({
      kind: "vanilla",
      vanilla: { optionType: "PUT", strike: { kind: "strike", strike: 1.085 } },
    });
    // The frame is the existing builder's (only the payoff differs from default).
    const frame = { ...vanillaInstrument(CTX.pair, CTX.tenorYears, "PUT", 0.25, CTX.notionalMm) };
    expect(inst.quantity).toEqual(frame.quantity);
    expect(inst.side).toBe(frame.side);
    // The wire arm carries the absolute strike (the server prices the level as-is).
    const wire = instrumentToWire(inst) as { vanilla?: { strike?: unknown } };
    expect(wire.vanilla?.strike).toEqual({ strike: 1.085 });
  });

  it("an edited strategy's legs ride the wire Strategy (arbitrary legs, kind kept)", () => {
    const inputs: StrategyInputs = {
      template: "RISK_REVERSAL",
      legs: [
        leg("CALL", { kind: "strike", strike: 1.12 }, "BUY", 2),
        leg("PUT", { kind: "delta", delta: -0.1 }, "SELL"),
      ],
    };
    expect(riskReversalSpec.validate!(inputs, CTX)).toEqual([]);
    const inst = riskReversalSpec.toInstrument(inputs, CTX);
    if (inst.product.kind !== "strategy") throw new Error("expected a strategy product");
    expect(inst.product.strategy.kind).toBe("RISK_REVERSAL");
    expect(inst.product.strategy.legs).toEqual([
      { optionType: "CALL", strike: { kind: "strike", strike: 1.12 }, side: "BUY", ratio: 2 },
      { optionType: "PUT", strike: { kind: "delta", delta: -0.1 }, side: "SELL", ratio: 1 },
    ]);
    const wire = instrumentToWire(inst) as {
      strategy?: { legs?: { strike?: unknown; ratio?: number }[] };
    };
    expect(wire.strategy?.legs?.[0]?.strike).toEqual({ strike: 1.12 });
    expect(wire.strategy?.legs?.[0]?.ratio).toBe(2);
    expect(wire.strategy?.legs?.[1]?.strike).toEqual({ delta: -0.1 });
  });

  it("an in-progress strike draft never reaches the contract object", () => {
    const inputs: StrategyInputs = {
      template: "RISK_REVERSAL",
      legs: [
        { ...leg("CALL", { kind: "delta", delta: 0.25 }, "BUY"), strikeDraft: "1.1" },
        leg("PUT", { kind: "delta", delta: -0.25 }, "SELL"),
      ],
    };
    const inst = riskReversalSpec.toInstrument(inputs, CTX);
    if (inst.product.kind !== "strategy") throw new Error("expected a strategy product");
    for (const l of inst.product.strategy.legs) {
      expect(Object.keys(l).sort()).toEqual(["optionType", "ratio", "side", "strike"]);
    }
  });
});
