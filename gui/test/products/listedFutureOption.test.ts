/**
 * GW2 per-family round-trip gate for the option on a listed future (proto
 * `listed_future_option`, field 31). Builds from the spec defaults against a
 * representative EURUSD 3M context and pins: the product-oneof arm + stamped
 * tenor, the exact wire body (nested `future_symbol`, `future_expiry_years`,
 * numeric `option_type`/`margining` enums — what the server WS codec's
 * `listed_future_option_from_json` decodes), the future-outlives-option validity
 * BY CONSTRUCTION (`futureExpiryYears = tenorYears + lag`, lag clamped ≥ 0, at
 * every tenor), the margining toggle's enum numbers, deterministic round-trip,
 * booking-model parity and the gallery grouping.
 */
import { describe, expect, it } from "vitest";

import {
  listedFutureOptionSpec,
  type ListedFutureOptionInputs,
} from "../../src/products/listedFutureOption";
import type { ProductBuildCtx } from "../../src/products/types";
import { registryByGroup } from "../../src/products";
import {
  bookingModelsFor,
  listedFutureOptionInstrument,
  tenorYearsToTenor,
} from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";
import { priceInstrument } from "../../src/data/pricing";
import type { Margining, MarketContext } from "../../src/data/contract";
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

describe("listed-future-option product spec (GW2)", () => {
  it("builds the declared kind with the stamped tenor from its defaults", () => {
    const inst = listedFutureOptionSpec.toInstrument(listedFutureOptionSpec.defaults, CTX);
    expect(inst.product.kind).toBe(listedFutureOptionSpec.kind);
    expect(inst.product.kind).toBe("listedFutureOption");
    expect(inst.tenor).toEqual(CTX.tenor);
    expect(inst.expiryYears).toBe(CTX.tenorYears);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("encodes under `listed_future_option` (field 31) with the nested future_symbol + margining tag", () => {
    const inputs: ListedFutureOptionInputs = {
      optionType: "PUT",
      strike: 1.12,
      futureTicker: " 6e ",
      futureVenue: " xcme ",
      futureLagYears: 0.05,
      margining: "FUTURES_STYLE",
    };
    const w = instrumentToWire(listedFutureOptionSpec.toInstrument(inputs, CTX));
    expect(w["listed_future_option"]).toEqual({
      // The symbol is trimmed/uppercased (the cross-asset symbol hygiene).
      future_symbol: { ticker: "6E", venue: "XCME" },
      future_expiry_years: CTX.tenorYears + 0.05,
      // OptionType CALL=0, PUT=1; Margining EQUITY_STYLE=0, FUTURES_STYLE=1.
      option_type: e.optionType.toWire("PUT"),
      strike: 1.12,
      notional: 10e6,
      margining: e.margining.toWire("FUTURES_STYLE"),
    });
    // A oneof carries exactly one body.
    expect(w["vanilla"]).toBeUndefined();
    expect(w["perpetual_option"]).toBeUndefined();
  });

  it("pins the canonical Margining enum numbers the server decodes by", () => {
    expect(e.margining.toWire("EQUITY_STYLE")).toBe(0);
    expect(e.margining.toWire("FUTURES_STYLE")).toBe(1);
    expect(e.margining.fromWire(0)).toBe("EQUITY_STYLE");
    expect(e.margining.fromWire(1)).toBe("FUTURES_STYLE");
  });

  it("the future outlives the option BY CONSTRUCTION at every tenor (lag clamped ≥ 0)", () => {
    for (const tenorYears of [1 / 365, 0.25, 1, 3]) {
      const ctx: ProductBuildCtx = { ...CTX, tenor: tenorYearsToTenor(tenorYears), tenorYears };
      // Even an (invalid) negative lag input cannot undercut the option expiry.
      for (const lag of [-1, 0, 0.5]) {
        const inst = listedFutureOptionSpec.toInstrument(
          { ...listedFutureOptionSpec.defaults, futureLagYears: lag },
          ctx,
        );
        if (inst.product.kind !== "listedFutureOption") throw new Error("expected the arm");
        const o = inst.product.listedFutureOption;
        expect(o.futureExpiryYears).toBeGreaterThanOrEqual(inst.expiryYears);
        expect(inst.expiryYears).toBeGreaterThan(0);
        expect(o.futureExpiryYears).toBe(tenorYears + Math.max(0, lag));
      }
    }
  });

  it("defaults the strike to the ATM-forward (the futures level at the option expiry)", () => {
    const inst = listedFutureOptionSpec.toInstrument(listedFutureOptionSpec.defaults, CTX);
    if (inst.product.kind !== "listedFutureOption") throw new Error("expected the arm");
    expect(inst.product.listedFutureOption.strike).toBe(CTX.atmForward);
    expect(inst.product.listedFutureOption.margining).toBe("EQUITY_STYLE");
  });

  it("matches the seed builder byte-for-byte (the spec delegates, never duplicates)", () => {
    const viaSpec = listedFutureOptionSpec.toInstrument(listedFutureOptionSpec.defaults, CTX);
    const viaSeed = listedFutureOptionInstrument(CTX.pair, CTX.tenorYears, CTX.notionalMm, {
      futureSymbol: { ticker: "6E", venue: "XCME" },
      futureExpiryYears: CTX.tenorYears,
      optionType: "CALL",
      strike: CTX.atmForward,
      margining: "EQUITY_STYLE",
    });
    expect(viaSpec).toEqual(viaSeed);
    expect(instrumentToWire(viaSpec)).toEqual(instrumentToWire(viaSeed));
  });

  it("round-trips deterministically through the real wire codec", () => {
    const inst = listedFutureOptionSpec.toInstrument(listedFutureOptionSpec.defaults, CTX);
    const wire = instrumentToWire(inst);
    expect(wire).toBeTruthy();
    expect(instrumentToWire(inst)).toEqual(wire);
  });

  it("allowedModels equal bookingModelsFor(kind)", () => {
    expect(listedFutureOptionSpec.allowedModels).toEqual(
      bookingModelsFor(listedFutureOptionSpec.kind),
    );
  });

  // The offline price itself is gated against the frozen golden corpus (the
  // published Haug §1.2.2 worked market) in `conformance.test.ts`; here the
  // ANALYTIC Greek strip is validated against an independent central finite
  // difference of that same closed form, on BOTH margining conventions.
  it("offline analytic Greeks match a central finite difference of the closed form", () => {
    const fd = (f: (x: number) => number, x: number, h: number): number =>
      (f(x + h) - f(x - h)) / (2 * h);
    // |a−b| ≤ max(rel·max(|a|,|b|), abs) — the server-side assert_close law. The
    // rel band is 1e-5 (looser than the perpetual's 1e-6) because this closed
    // form goes through the GUI's rational normal-CDF (~1e-7 absolute), whose
    // own local error derivative the finite difference legitimately picks up.
    const expectClose = (a: number, b: number, rel: number, abs: number): void => {
      expect(Math.abs(a - b)).toBeLessThanOrEqual(
        Math.max(rel * Math.max(Math.abs(a), Math.abs(b)), abs),
      );
    };
    const m: MarketContext = { spot: 19.0, vol: 0.28, rDom: 0.1, rFor: 0.1 };
    const t = 0.75;
    const ctx: ProductBuildCtx = { ...CTX, tenor: tenorYearsToTenor(t), tenorYears: t };
    for (const margining of ["EQUITY_STYLE", "FUTURES_STYLE"] as Margining[]) {
      for (const optionType of ["CALL", "PUT"] as const) {
        const build = (expiry: number) =>
          listedFutureOptionSpec.toInstrument(
            { ...listedFutureOptionSpec.defaults, optionType, strike: 19.0, margining },
            { ...ctx, tenor: tenorYearsToTenor(expiry), tenorYears: expiry },
          );
        const inst = build(t);
        const at = (mk: MarketContext) => priceInstrument(inst, mk);
        const g = at(m).greeks;
        const hs = 1e-4 * m.spot;
        expectClose(g.deltaSpot, fd((x) => at({ ...m, spot: x }).greeks.price, m.spot, hs), 1e-5, 1e-8);
        // The underlying IS the futures level: forward delta equals spot delta.
        expect(g.deltaForward).toBe(g.deltaSpot);
        expectClose(g.gamma, fd((x) => at({ ...m, spot: x }).greeks.deltaSpot, m.spot, hs), 1e-5, 1e-8);
        expectClose(g.vega, fd((x) => at({ ...m, vol: x }).greeks.price, m.vol, 1e-6), 1e-5, 1e-8);
        expectClose(g.rhoDom, fd((x) => at({ ...m, rDom: x }).greeks.price, m.rDom, 1e-7), 1e-5, 1e-8);
        // Calendar theta (per-day): −∂V/∂T of the closed form, FD'd over the
        // option expiry (the future's own expiry never enters the price).
        const dVdT = fd((expiry) => priceInstrument(build(expiry), m).greeks.price, t, 1e-5);
        expectClose(g.theta, -dVdT / 365, 1e-5, 1e-9);
      }
    }
  });

  it("sits in the Vanilla & strategies gallery group (a vanilla on a listed future)", () => {
    expect(listedFutureOptionSpec.group).toBe("Vanilla & strategies");
    const grouped = registryByGroup();
    const group = grouped.find((g) => g.group === "Vanilla & strategies");
    expect(group).toBeDefined();
    expect(group!.specs.map((s) => s.id)).toContain("LISTED_FUTURE_OPTION");
  });
});
