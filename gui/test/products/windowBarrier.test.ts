/**
 * GW2 window-barrier {@link windowBarrierSpec} round-trip test: the spec's
 * defaults build a wire-valid `windowBarrier` instrument LOCKED to
 * LOCAL_STOCH_VOL (no closed form), with the stamped tenor, that round-trips
 * deterministically through `wsCodec.instrumentToWire`, and whose `allowedModels`
 * match the server booking matrix.
 */
import { describe, expect, it } from "vitest";

import { windowBarrierSpec } from "../../src/products/windowBarrier";
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

describe("window barrier spec (GW2)", () => {
  it("builds a wire-valid windowBarrier instrument from its defaults", () => {
    const inst = windowBarrierSpec.toInstrument(windowBarrierSpec.defaults, CTX);
    expect(inst.product.kind).toBe(windowBarrierSpec.kind);
    expect(inst.tenor).toEqual(CTX.tenor);
    // No closed form ⇒ the booking model is LOCKED to LOCAL_STOCH_VOL, overriding
    // the trader's DEFAULT selection in CTX.
    expect(inst.pricingModel).toBe("LOCAL_STOCH_VOL");
  });

  it("round-trips deterministically through instrumentToWire", () => {
    const inst = windowBarrierSpec.toInstrument(windowBarrierSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equal bookingModelsFor(kind) (LOCAL_STOCH_VOL only)", () => {
    expect(windowBarrierSpec.allowedModels).toEqual(bookingModelsFor(windowBarrierSpec.kind));
  });
});
