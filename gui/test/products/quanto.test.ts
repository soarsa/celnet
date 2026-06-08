/**
 * Per-family round-trip gate (GW2) for the quanto {@link ProductSpec}: its
 * defaults build a wire-valid instrument of the declared kind, with the tenor
 * stamped, that round-trips deterministically through the real `instrumentToWire`
 * codec, and whose `allowedModels` match the server booking matrix. The default
 * strike (0) falls back to the ATM-forward at build (`quantoTerms`).
 */
import { describe, expect, it } from "vitest";

import { quantoSpec } from "../../src/products/quanto";
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

describe("quanto ProductSpec (GW2)", () => {
  it("builds the declared kind with the tenor stamped, DEFAULT model omitted", () => {
    const inst = quantoSpec.toInstrument(quantoSpec.defaults, CTX);
    expect(inst.product.kind).toBe(quantoSpec.kind);
    expect(inst.tenor).toEqual(CTX.tenor);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("defaults the strike to the ATM-forward when zero (quantoTerms fallback)", () => {
    const inst = quantoSpec.toInstrument(quantoSpec.defaults, CTX);
    if (inst.product.kind !== "quanto") throw new Error("expected a quanto product");
    expect(inst.product.quanto.strike).toBe(CTX.atmForward);
  });

  it("round-trips deterministically through the real wire codec", () => {
    const inst = quantoSpec.toInstrument(quantoSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(quantoSpec.allowedModels).toEqual(bookingModelsFor(quantoSpec.kind));
  });
});
