/**
 * IRS product-spec contract test (fi-bond-ticket-gui). The vanilla-IRS
 * {@link RatesProductSpec} is a `rates`-family entry in the shared registry: its
 * defaults are lawful, `toRatesInstrument` builds the wire `irs` arm, `pinToPar`
 * pins the fixed rate, and the built instrument prices through the SAME offline
 * engine the live `price_rates` mirror wraps.
 */
import { describe, expect, it } from "vitest";

import { irsSpec, type IrsInputs } from "../../src/products/irs";
import { isRatesSpec, type ProductBuildCtx } from "../../src/products";
import { tenorYearsToTenor } from "../../src/data/seed";
import { DEFAULT_USD_SOFR_CURVE, priceRatesInstrumentOffline } from "../../src/data/ratesPricing";

const CTX: ProductBuildCtx = {
  pair: { base: "EUR", quote: "USD" },
  tenor: tenorYearsToTenor(5),
  tenorYears: 5,
  notionalMm: 100,
  pricingModel: "DEFAULT",
  atmForward: 1.1,
  spot: 1.1,
  pipDecimals: 4,
  today: { year: 2026, month: 6, day: 25 },
};

describe("irsSpec — the vanilla-IRS rates family", () => {
  it("is a rates-family spec in the Fixed-income group", () => {
    expect(isRatesSpec(irsSpec)).toBe(true);
    expect(irsSpec.id).toBe("IRS");
    expect(irsSpec.group).toBe("Fixed income (rates)");
    expect(irsSpec.priceActionLabel).toBe("Price swap");
  });

  it("has lawful defaults and builds the wire `irs` arm from them", () => {
    expect(irsSpec.validate?.(irsSpec.defaults, CTX)).toEqual([]);
    const instrument = irsSpec.toRatesInstrument(irsSpec.defaults, CTX);
    expect(instrument.kind).toBe("irs");
    if (instrument.kind !== "irs") throw new Error("expected an IRS arm");
    expect(instrument.irs.tenorYears).toBe(5);
    expect(instrument.irs.fixedRate).toBeCloseTo(0.0405, 12);
    expect(instrument.irs.notional).toBe(100_000_000);
    expect(instrument.irs.fixedFrequency).toBe("SEMI_ANNUAL");
    expect(instrument.irs.floatFrequency).toBe("QUARTERLY");
  });

  it("gates a sub-1Y tenor and a non-positive notional", () => {
    const bad: IrsInputs = { ...irsSpec.defaults, tenorYears: 0, notionalMm: 0 };
    expect((irsSpec.validate?.(bad, CTX) ?? []).length).toBeGreaterThanOrEqual(2);
  });

  it("prices its default through the shared offline engine", () => {
    const res = priceRatesInstrumentOffline(
      DEFAULT_USD_SOFR_CURVE,
      irsSpec.toRatesInstrument(irsSpec.defaults, CTX),
    );
    expect(Number.isFinite(res.pv)).toBe(true);
    expect(res.parRate).toBeGreaterThan(0);
    expect(res.keyRateLadder.length).toBe(DEFAULT_USD_SOFR_CURVE.pillars.length);
  });

  it("pins the fixed rate to par so a re-price is ~flat", () => {
    const par = priceRatesInstrumentOffline(
      DEFAULT_USD_SOFR_CURVE,
      irsSpec.toRatesInstrument(irsSpec.defaults, CTX),
    ).parRate;
    const pinned = irsSpec.pinToPar!(irsSpec.defaults, par);
    expect(pinned.fixedRatePct).toBeCloseTo(par * 100, 4);
    const res = priceRatesInstrumentOffline(
      DEFAULT_USD_SOFR_CURVE,
      irsSpec.toRatesInstrument(pinned, CTX),
    );
    expect(Math.abs(res.pv)).toBeLessThan(1e3); // 4-dp rounding of a 100mm swap
  });
});
