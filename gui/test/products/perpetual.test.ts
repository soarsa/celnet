/**
 * GW2 per-family round-trip gate for the perpetual (no-expiry) American option —
 * the ONE tenorless, expiryless product (proto `perpetual_option`, field 30).
 * Builds from the spec defaults against a representative EURUSD context and pins
 * the contract's canonical no-expiry shape: `expiryYears = 0` exactly, NO tenor
 * (neither on the instrument nor on the wire frame), the exact `perpetual_option`
 * wire body (numeric enums, snake_case field names — what the server WS codec's
 * `perpetual_option_from_json` decodes), deterministic round-trip, booking-model
 * parity, the declared `noExpiry` honesty seam, and the registry grouping.
 */
import { describe, expect, it } from "vitest";

import { perpetualSpec, type PerpetualInputs } from "../../src/products/perpetual";
import type { ProductBuildCtx } from "../../src/products/types";
import { registryByGroup } from "../../src/products";
import { bookingModelsFor, perpetualInstrument, tenorYearsToTenor } from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";
import { priceInstrument } from "../../src/data/pricing";
import type { MarketContext } from "../../src/data/contract";
import * as e from "../../src/data/enums";

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

describe("perpetual product spec (GW2)", () => {
  it("builds the declared kind with NO tenor and exactly zero expiry from its defaults", () => {
    const inst = perpetualSpec.toInstrument(perpetualSpec.defaults, CTX);
    expect(inst.product.kind).toBe(perpetualSpec.kind);
    expect(inst.product.kind).toBe("perpetualOption");
    // The canonical no-expiry shape: the ctx tenor is NOT stamped (a perpetual
    // has no tenor label) and the expiry is the exact proto3 zero the server's
    // term validator requires for this arm.
    expect(inst.tenor).toBeUndefined();
    expect(inst.expiryYears).toBe(0);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("declares the no-expiry seam the ticket shell disables the expiry controls on", () => {
    expect(perpetualSpec.noExpiry).toBeDefined();
    expect(perpetualSpec.noExpiry!.reason.length).toBeGreaterThan(0);
  });

  it("encodes under `perpetual_option` (field 30) with no `tenor` key on the frame", () => {
    const inst = perpetualSpec.toInstrument(
      { optionType: "PUT", strike: 1.05 } satisfies PerpetualInputs,
      CTX,
    );
    const w = instrumentToWire(inst);
    expect(w["perpetual_option"]).toEqual({
      // OptionType CALL=0, PUT=1.
      option_type: e.optionType.toWire("PUT"),
      strike: 1.05,
      notional: 10e6,
    });
    expect(w["expiry_years"]).toBe(0);
    // The tenorless frame: no tenor key at all (matching the SDK's optional
    // tenor and the server round-trip test's no-tenor JSON shape).
    expect("tenor" in w).toBe(false);
    // A oneof carries exactly one body.
    expect(w["vanilla"]).toBeUndefined();
    expect(w["listed_future_option"]).toBeUndefined();
  });

  it("round-trips deterministically through the real wire codec", () => {
    const inst = perpetualSpec.toInstrument(perpetualSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("defaults a 0 strike to SPOT (not ATMF — no forward exists without an expiry)", () => {
    const inst = perpetualSpec.toInstrument(perpetualSpec.defaults, CTX);
    if (inst.product.kind !== "perpetualOption") throw new Error("expected perpetualOption");
    expect(inst.product.perpetualOption.strike).toBe(CTX.spot);
    expect(inst.product.perpetualOption.notional).toBe(CTX.notionalMm * 1e6);
  });

  it("matches the seed builder byte-for-byte (the spec delegates, never duplicates)", () => {
    const viaSpec = perpetualSpec.toInstrument(
      { optionType: "CALL", strike: 1.2 } satisfies PerpetualInputs,
      CTX,
    );
    const viaSeed = perpetualInstrument(CTX.pair, CTX.notionalMm, {
      optionType: "CALL",
      strike: 1.2,
    });
    expect(viaSpec).toEqual(viaSeed);
    expect(instrumentToWire(viaSpec)).toEqual(instrumentToWire(viaSeed));
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(perpetualSpec.allowedModels).toEqual(bookingModelsFor(perpetualSpec.kind));
  });

  // The offline price itself is gated against the frozen golden corpus (the
  // independent bisection-re-derived oracle) in `conformance.test.ts`; here the
  // ANALYTIC Greek strip is validated against an independent central finite
  // difference of that same closed form — never merely asserted plausible.
  it("offline analytic Greeks match a central finite difference of the closed form", () => {
    const fd = (f: (x: number) => number, x: number, h: number): number =>
      (f(x + h) - f(x - h)) / (2 * h);
    // |a−b| ≤ max(rel·max(|a|,|b|), abs) — the server-side assert_close law,
    // with the SAME per-Greek bands the server's perpetual FD test pins.
    const expectClose = (a: number, b: number, rel: number, abs: number): void => {
      expect(Math.abs(a - b)).toBeLessThanOrEqual(
        Math.max(rel * Math.max(Math.abs(a), Math.abs(b)), abs),
      );
    };
    // Continuation-region markets on both sides (call b < r; put r > 0).
    const cases: { optionType: "CALL" | "PUT"; strike: number; m: MarketContext }[] = [
      { optionType: "CALL", strike: 1.25, m: { spot: 1.3, vol: 0.1, rDom: 0.05, rFor: 0.01 } },
      { optionType: "PUT", strike: 1.25, m: { spot: 1.3, vol: 0.1, rDom: 0.05, rFor: 0.01 } },
    ];
    for (const c of cases) {
      const inst = perpetualSpec.toInstrument({ optionType: c.optionType, strike: c.strike }, CTX);
      const at = (m: MarketContext) => priceInstrument(inst, m);
      const g = at(c.m).greeks;
      const hs = 1e-4 * c.m.spot;
      expectClose(g.deltaSpot, fd((x) => at({ ...c.m, spot: x }).greeks.price, c.m.spot, hs), 1e-6, 1e-9);
      expectClose(g.gamma, fd((x) => at({ ...c.m, spot: x }).greeks.deltaSpot, c.m.spot, hs), 1e-6, 1e-9);
      expectClose(g.vega, fd((x) => at({ ...c.m, vol: x }).greeks.price, c.m.vol, 1e-6), 1e-6, 1e-8);
      expectClose(g.rhoDom, fd((x) => at({ ...c.m, rDom: x }).greeks.price, c.m.rDom, 1e-7), 1e-5, 1e-7);
      expectClose(g.rhoFor, fd((x) => at({ ...c.m, rFor: x }).greeks.price, c.m.rFor, 1e-7), 1e-5, 1e-7);
      // Time-homogeneity: theta is identically zero (a structural law, not ≈0).
      expect(g.theta).toBe(0);
    }
  });

  it("sits in the Path-dependent gallery group (beside the early-exercise American)", () => {
    expect(perpetualSpec.group).toBe("Path-dependent");
    const grouped = registryByGroup();
    const group = grouped.find((g) => g.group === "Path-dependent");
    expect(group).toBeDefined();
    const ids = group!.specs.map((s) => s.id);
    expect(ids).toContain("PERPETUAL");
    // Catalogue order: the perpetual follows the dated American family.
    expect(ids.indexOf("PERPETUAL")).toBe(ids.indexOf("AMERICAN") + 1);
  });
});
