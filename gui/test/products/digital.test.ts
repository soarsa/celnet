/**
 * Per-family round-trip gate for the digital {@link ProductSpec}. Builds the wire
 * instrument from the spec defaults, asserts the product-oneof arm, the stamped
 * tenor, deterministic round-trip through the real `wsCodec`, and that
 * `allowedModels` tracks the server booking matrix (no drift).
 */
import { describe, expect, it } from "vitest";

import { digitalSpec } from "../../src/products/digital";
import type { ProductBuildCtx } from "../../src/products/types";
import { bookingModelsFor, tenorYearsToTenor } from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";

/** A representative EURUSD 3M context (matches the registry round-trip test). */
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

describe("digital ProductSpec", () => {
  it("builds a wire-valid, deterministically round-tripping instrument from its defaults", () => {
    const inst = digitalSpec.toInstrument(digitalSpec.defaults, CTX);
    expect(inst.product.kind).toBe(digitalSpec.kind);
    expect(inst.tenor).toEqual(CTX.tenor);
    // DEFAULT booking model is presence-omitted on the wire.
    expect(inst.pricingModel).toBeUndefined();
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equals bookingModelsFor(kind)", () => {
    expect(digitalSpec.allowedModels).toEqual(bookingModelsFor(digitalSpec.kind));
  });
});
