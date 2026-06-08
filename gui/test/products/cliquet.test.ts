/**
 * Per-family round-trip gate (GW2) for the cliquet {@link ProductSpec}: its
 * defaults build a wire-valid instrument of the declared kind, with the tenor
 * stamped, that round-trips deterministically through the real `instrumentToWire`
 * codec, and whose `allowedModels` match the server booking matrix.
 */
import { describe, expect, it } from "vitest";

import { cliquetSpec } from "../../src/products/cliquet";
import { bookingModelsFor, tenorYearsToTenor } from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";
import type { ProductBuildCtx } from "../../src/products";

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

describe("cliquet ProductSpec (GW2)", () => {
  it("builds the declared kind with the tenor stamped, DEFAULT model omitted", () => {
    const inst = cliquetSpec.toInstrument(cliquetSpec.defaults, CTX);
    expect(inst.product.kind).toBe(cliquetSpec.kind);
    expect(inst.tenor).toEqual(CTX.tenor);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("round-trips deterministically through the real wire codec", () => {
    const inst = cliquetSpec.toInstrument(cliquetSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(cliquetSpec.allowedModels).toEqual(bookingModelsFor(cliquetSpec.kind));
  });
});
