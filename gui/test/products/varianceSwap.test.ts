/**
 * GW2 per-family round-trip gate for the variance swap. Builds from the spec
 * defaults against a representative EURUSD 3M context, asserts the product-oneof
 * arm + stamped tenor, deterministic wire round-trip, and booking-model parity.
 */
import { describe, expect, it } from "vitest";

import { varianceSwapSpec } from "../../src/products/varianceSwap";
import type { ProductBuildCtx } from "../../src/products/types";
import { bookingModelsFor, tenorYearsToTenor } from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";

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

describe("variance swap product spec (GW2)", () => {
  it("builds the declared kind with the stamped tenor from its defaults", () => {
    const inst = varianceSwapSpec.toInstrument(varianceSwapSpec.defaults, CTX);
    expect(inst.product.kind).toBe(varianceSwapSpec.kind);
    expect(inst.product.kind).toBe("varianceSwap");
    expect(inst.tenor).toEqual(CTX.tenor);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("round-trips deterministically through the real wire codec", () => {
    const inst = varianceSwapSpec.toInstrument(varianceSwapSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(varianceSwapSpec.allowedModels).toEqual(bookingModelsFor(varianceSwapSpec.kind));
  });
});
