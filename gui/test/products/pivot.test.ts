/**
 * GW2 per-family round-trip gate for the pivot-TRA {@link ProductSpec} (wire arm
 * 32). Builds the wire instrument from the spec defaults under a representative
 * EURUSD 3M context, asserts the product-oneof arm + stamped tenor, byte-stable
 * wire round-trip, the booking-model matrix, and the TARF degeneracy default
 * (`pivot` left 0 resolves to the strike — the exact plain-TARF slice).
 */
import { describe, expect, it } from "vitest";

import { pivotSpec, pivotTerms, DEFAULT_PIVOT } from "../../src/products/pivot";
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

describe("pivotSpec (GW2)", () => {
  it("builds a wire-valid instrument with the declared kind + stamped tenor from its defaults", () => {
    const inst = pivotSpec.toInstrument(pivotSpec.defaults, CTX);
    expect(inst.product.kind).toBe(pivotSpec.kind);
    expect(inst.product.kind).toBe("pivot");
    expect(inst.tenor).toEqual(CTX.tenor);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("round-trips deterministically through the real wire codec", () => {
    const inst = pivotSpec.toInstrument(pivotSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("encodes the pivot arm body field-for-field as the server codec decodes it", () => {
    const inst = pivotSpec.toInstrument(
      { ...DEFAULT_PIVOT, strike: 1.12, pivot: 1.08, target: 0.12, leverage: 2.5 },
      CTX,
    );
    const wire = instrumentToWire(inst) as Record<string, unknown>;
    const body = wire["pivot"] as Record<string, unknown>;
    expect(body).toBeTruthy();
    expect(body["strike"]).toBe(1.12);
    expect(body["pivot"]).toBe(1.08);
    expect(body["target"]).toBe(0.12);
    expect(body["leverage"]).toBe(2.5);
    const schedule = body["schedule"] as Record<string, unknown>;
    expect((schedule["fixing_years"] as number[]).length).toBe(DEFAULT_PIVOT.fixings);
  });

  it("defaults the pivot to the resolved strike — the exact plain-TARF slice", () => {
    // strike 0 ⇒ ATMF; pivot 0 ⇒ the resolved strike. The degeneracy default
    // keeps an untouched ticket on the validated TARF slice.
    const terms = pivotTerms(DEFAULT_PIVOT, CTX.atmForward, CTX.tenorYears);
    expect(terms.strike).toBe(CTX.atmForward);
    expect(terms.pivot).toBe(terms.strike);
    // An explicit pivot survives untouched (dead band / overlap geometries).
    const dead = pivotTerms(
      { ...DEFAULT_PIVOT, strike: 1.28, pivot: 1.33 },
      CTX.atmForward,
      CTX.tenorYears,
    );
    expect(dead.strike).toBe(1.28);
    expect(dead.pivot).toBe(1.33);
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(pivotSpec.allowedModels).toEqual(bookingModelsFor(pivotSpec.kind));
  });
});
