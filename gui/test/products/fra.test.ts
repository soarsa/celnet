/**
 * FRA product-spec contract test (fi-bond-ticket-gui). The FRA
 * {@link RatesProductSpec} is a `rates`-family entry: lawful defaults, a wire `fra`
 * arm built from a `[start, end]` month window, a par pin, and pricing through the
 * SAME offline engine the live `price_rates` mirror wraps.
 */
import { describe, expect, it } from "vitest";

import { fraSpec, type FraInputs } from "../../src/products/fra";
import { isRatesSpec, type ProductBuildCtx } from "../../src/products";
import { tenorYearsToTenor } from "../../src/data/seed";
import { DEFAULT_USD_SOFR_CURVE, priceRatesInstrumentOffline } from "../../src/data/ratesPricing";

const CTX: ProductBuildCtx = {
  pair: { base: "EUR", quote: "USD" },
  tenor: tenorYearsToTenor(0.5),
  tenorYears: 0.5,
  notionalMm: 100,
  pricingModel: "DEFAULT",
  atmForward: 1.1,
  spot: 1.1,
  pipDecimals: 4,
  today: { year: 2026, month: 6, day: 25 },
};

describe("fraSpec — the FRA rates family", () => {
  it("is a rates-family spec in the Fixed-income group", () => {
    expect(isRatesSpec(fraSpec)).toBe(true);
    expect(fraSpec.id).toBe("FRA");
    expect(fraSpec.group).toBe("Fixed income (rates)");
    expect(fraSpec.priceActionLabel).toBe("Price FRA");
  });

  it("has lawful defaults and builds the wire `fra` arm (a 3×6 window)", () => {
    expect(fraSpec.validate?.(fraSpec.defaults, CTX)).toEqual([]);
    const instrument = fraSpec.toRatesInstrument(fraSpec.defaults, CTX);
    expect(instrument.kind).toBe("fra");
    if (instrument.kind !== "fra") throw new Error("expected a FRA arm");
    expect(instrument.fra.startMonths).toBe(3);
    expect(instrument.fra.endMonths).toBe(6);
    expect(instrument.fra.notional).toBe(100_000_000);
    expect(instrument.fra.accrualBasis).toBe("ACT_360");
  });

  it("gates a non-increasing window and a non-positive notional", () => {
    const bad: FraInputs = { ...fraSpec.defaults, startMonths: 6, endMonths: 6, notionalMm: 0 };
    expect((fraSpec.validate?.(bad, CTX) ?? []).length).toBeGreaterThanOrEqual(2);
  });

  it("prices its default through the shared offline engine", () => {
    const res = priceRatesInstrumentOffline(
      DEFAULT_USD_SOFR_CURVE,
      fraSpec.toRatesInstrument(fraSpec.defaults, CTX),
    );
    expect(Number.isFinite(res.pv)).toBe(true);
    expect(res.parRate).toBeGreaterThan(0);
    expect(res.keyRateLadder.length).toBe(DEFAULT_USD_SOFR_CURVE.pillars.length);
  });

  it("pins the fixed rate to par so a re-price is ~zero", () => {
    const par = priceRatesInstrumentOffline(
      DEFAULT_USD_SOFR_CURVE,
      fraSpec.toRatesInstrument(fraSpec.defaults, CTX),
    ).parRate;
    const pinned = fraSpec.pinToPar!(fraSpec.defaults, par);
    expect(pinned.fixedRatePct).toBeCloseTo(par * 100, 4);
    const res = priceRatesInstrumentOffline(
      DEFAULT_USD_SOFR_CURVE,
      fraSpec.toRatesInstrument(pinned, CTX),
    );
    expect(Math.abs(res.pv)).toBeLessThan(1e3);
  });
});
