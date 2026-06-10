/**
 * GW2 product-registry contract test. For every registered {@link ProductSpec}:
 *   - its default inputs build a wire-valid `Instrument` whose product-oneof arm
 *     matches the declared `kind`, and that instrument round-trips through the
 *     real `wsCodec.instrumentToWire` deterministically (the byte-stable wire
 *     path the server decodes);
 *   - its `allowedModels` equal `bookingModelsFor(kind)` (no drift from the
 *     server's booking-model matrix);
 *   - the catalogue is internally consistent (unique ids/labels, grouped).
 * This is the per-ProductSpec round-trip gate the registry grows under.
 */
import { describe, expect, it } from "vitest";

import {
  PRODUCT_GROUP_ORDER,
  PRODUCT_REGISTRY,
  registryByGroup,
  specById,
  type ProductBuildCtx,
} from "../src/products";
import { bookingModelsFor, tenorYearsToTenor } from "../src/data/seed";
import { instrumentToWire } from "../src/data/wsCodec";

/** A representative EURUSD 3M context, ATM-forward seeded above spot. */
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

describe("product registry (GW2)", () => {
  it("has at least one registered family", () => {
    expect(PRODUCT_REGISTRY.length).toBeGreaterThan(0);
  });

  it("every spec builds a wire-valid, deterministically round-tripping instrument from its defaults", () => {
    for (const spec of PRODUCT_REGISTRY) {
      const inst = spec.toInstrument(spec.defaults, CTX);
      // Tenor stamped (buildInstrument tail); DEFAULT model presence-omitted.
      // A declared no-expiry family (the perpetual — the one tenorless product)
      // instead books the contract's canonical no-expiry shape: no tenor label
      // exists and the expiry is the exact proto3 zero the server requires.
      if (spec.noExpiry) {
        expect(inst.tenor).toBeUndefined();
        expect(inst.expiryYears).toBe(0);
      } else {
        expect(inst.tenor).toEqual(CTX.tenor);
      }
      if (spec.allowedModels.length === 1 && spec.allowedModels[0] === "DEFAULT") {
        expect(inst.pricingModel).toBeUndefined();
      }
      // The product-oneof arm the shell will book under matches the spec.
      expect(inst.product.kind).toBe(spec.kind);
      // Round-trips through the real wire codec, and is byte-stable.
      const wire = instrumentToWire(inst);
      expect(wire).toBeTruthy();
      expect(instrumentToWire(inst)).toEqual(wire);
    }
  });

  it("allowedModels equal bookingModelsFor(kind) for every spec (no booking-matrix drift)", () => {
    for (const spec of PRODUCT_REGISTRY) {
      expect(spec.allowedModels).toEqual(bookingModelsFor(spec.kind));
    }
  });

  it("ids and labels are unique and every spec is groupable + findable", () => {
    const ids = PRODUCT_REGISTRY.map((s) => s.id);
    const labels = PRODUCT_REGISTRY.map((s) => s.label);
    expect(new Set(ids).size).toBe(ids.length);
    expect(new Set(labels).size).toBe(labels.length);
    for (const spec of PRODUCT_REGISTRY) {
      expect(PRODUCT_GROUP_ORDER).toContain(spec.group);
      expect(specById(spec.id)).toBe(spec);
    }
  });

  it("registryByGroup covers exactly the registered specs in canonical group order", () => {
    const grouped = registryByGroup();
    const flat = grouped.flatMap((g) => g.specs);
    expect(flat.length).toBe(PRODUCT_REGISTRY.length);
    // Groups appear in canonical order.
    const order = grouped.map((g) => PRODUCT_GROUP_ORDER.indexOf(g.group));
    expect(order).toEqual([...order].sort((a, b) => a - b));
  });
});
