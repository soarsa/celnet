/**
 * GW2 touch {@link touchSpec} round-trip test: the spec's defaults build a
 * wire-valid `touch` instrument with the stamped tenor, that round-trips
 * deterministically through `wsCodec.instrumentToWire`, and whose `allowedModels`
 * match the server booking matrix.
 */
import { describe, expect, it } from "vitest";

import { touchSpec } from "../../src/products/touch";
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

describe("touch spec (GW2)", () => {
  it("builds a wire-valid touch instrument from its defaults", () => {
    const inst = touchSpec.toInstrument(touchSpec.defaults, CTX);
    expect(inst.product.kind).toBe(touchSpec.kind);
    expect(inst.tenor).toEqual(CTX.tenor);
    // DEFAULT booking model ⇒ presence-omitted on the wire frame.
    expect(inst.pricingModel).toBeUndefined();
  });

  it("round-trips deterministically through instrumentToWire", () => {
    const inst = touchSpec.toInstrument(touchSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(touchSpec.allowedModels).toEqual(bookingModelsFor(touchSpec.kind));
  });
});
