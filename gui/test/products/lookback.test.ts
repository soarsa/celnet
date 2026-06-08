/**
 * GW2 per-family round-trip gate for the lookback. Builds from the spec defaults
 * against a representative EURUSD 3M context, asserts the product-oneof arm +
 * stamped tenor, deterministic wire round-trip, and booking-model parity. The
 * FIXED-strike ATMF default is exercised by a second build that flips style.
 */
import { describe, expect, it } from "vitest";

import { lookbackSpec, type LookbackInputs } from "../../src/products/lookback";
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

describe("lookback product spec (GW2)", () => {
  it("builds the declared kind with the stamped tenor from its defaults", () => {
    const inst = lookbackSpec.toInstrument(lookbackSpec.defaults, CTX);
    expect(inst.product.kind).toBe(lookbackSpec.kind);
    expect(inst.product.kind).toBe("lookback");
    expect(inst.tenor).toEqual(CTX.tenor);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("round-trips deterministically through the real wire codec", () => {
    const inst = lookbackSpec.toInstrument(lookbackSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("defaults the FIXED-strike lookback to the ATM-forward when strike is 0", () => {
    const fixed: LookbackInputs = { ...lookbackSpec.defaults, style: "FIXED", strike: 0 };
    const inst = lookbackSpec.toInstrument(fixed, CTX);
    expect(inst.product.kind).toBe("lookback");
    if (inst.product.kind !== "lookback") throw new Error("expected lookback product");
    expect(inst.product.lookback.strike).toBe(CTX.atmForward);
    const wire = instrumentToWire(inst);
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(lookbackSpec.allowedModels).toEqual(bookingModelsFor(lookbackSpec.kind));
  });
});
