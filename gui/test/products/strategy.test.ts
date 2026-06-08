/**
 * Per-family round-trip gate (GW2) for the vanilla + strategy {@link ProductSpec}s
 * (the leg-ladder family). Each spec's defaults build a wire-valid instrument of
 * the declared kind with the tenor stamped (DEFAULT model presence-omitted), that
 * round-trips deterministically through the real `instrumentToWire` codec, and
 * whose `allowedModels` match the server booking matrix. The legs are
 * template-fixed: we additionally pin each spec's instrument to its monolith
 * `vanillaInstrument` / `strategyInstrument` byte-for-byte.
 */
import { describe, expect, it } from "vitest";

import {
  riskReversalSpec,
  seagullSpec,
  straddleSpec,
  strangleSpec,
  vanillaSpec,
} from "../../src/products/strategy";
import {
  bookingModelsFor,
  strategyInstrument,
  tenorYearsToTenor,
  vanillaInstrument,
} from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";
import type { Instrument, StrategyKind } from "../../src/data/contract";
import type { AnyProductSpec, ProductBuildCtx } from "../../src/products";

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

const SPECS: readonly AnyProductSpec[] = [
  vanillaSpec,
  riskReversalSpec,
  strangleSpec,
  straddleSpec,
  seagullSpec,
];

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
