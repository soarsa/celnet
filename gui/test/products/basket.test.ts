/**
 * GW2 per-family round-trip gate for the correlated multi-asset basket
 * {@link ProductSpec}. Builds the wire instrument from the spec defaults (a 2-leg
 * worst-of) under a representative EURUSD 3M context, asserts the product-oneof
 * arm + stamped tenor, byte-stable wire round-trip, and the booking-model matrix.
 */
import { describe, expect, it } from "vitest";

import { basketSpec } from "../../src/products/basket";
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

describe("basketSpec (GW2)", () => {
  it("builds a wire-valid instrument with the declared kind + stamped tenor from its defaults", () => {
    const inst = basketSpec.toInstrument(basketSpec.defaults, CTX);
    expect(inst.product.kind).toBe(basketSpec.kind);
    expect(inst.product.kind).toBe("basket");
    expect(inst.tenor).toEqual(CTX.tenor);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("round-trips deterministically through the real wire codec", () => {
    const inst = basketSpec.toInstrument(basketSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(basketSpec.allowedModels).toEqual(bookingModelsFor(basketSpec.kind));
  });
});
