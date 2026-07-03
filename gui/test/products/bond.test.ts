/**
 * Bond product-spec contract test (fi-bond-ticket-gui). The cash-bond
 * {@link RatesProductSpec} is a `rates`-family entry: lawful defaults, a wire `bond`
 * arm built from coupon/maturity/redemption, the bond-specific result view (dirty PV
 * + yield, no PV01 row, no ladder, no par pin), and pricing through the SAME offline
 * engine the live `price_rates` mirror wraps.
 */
import { describe, expect, it } from "vitest";

import { bondSpec, type BondInputs } from "../../src/products/bond";
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

describe("bondSpec — the cash-bond rates family", () => {
  it("is a rates-family spec in the Fixed-income group", () => {
    expect(isRatesSpec(bondSpec)).toBe(true);
    expect(bondSpec.id).toBe("BOND");
    expect(bondSpec.group).toBe("Fixed income (rates)");
    expect(bondSpec.priceActionLabel).toBe("Price bond");
  });

  it("carries the bond result view (dirty PV + yield, no PV01 row / ladder) and no par pin", () => {
    expect(bondSpec.resultView).toEqual({
      pvLabel: "Dirty PV",
      pvHasCurrencyUnit: false,
      parLabel: "Yield to maturity",
      showPv01: false,
      showLadder: false,
    });
    // The bond's par metric is a yield with no fixed-rate input, so it offers no pin.
    expect(bondSpec.pinToPar).toBeUndefined();
  });

  it("has lawful defaults and builds the wire `bond` arm from them", () => {
    expect(bondSpec.validate?.(bondSpec.defaults, CTX)).toEqual([]);
    const instrument = bondSpec.toRatesInstrument(bondSpec.defaults, CTX);
    expect(instrument.kind).toBe("bond");
    if (instrument.kind !== "bond") throw new Error("expected a bond arm");
    expect(instrument.bond.couponRate).toBeCloseTo(0.04, 12);
    expect(instrument.bond.redemption).toBe(100);
    expect(instrument.bond.position).toBe("LONG");
    expect(instrument.bond.maturityDate.year).toBe(DEFAULT_USD_SOFR_CURVE.referenceDate.year + 5);
  });

  it("gates a non-positive redemption and a maturity on/before the curve spot", () => {
    const badRedemption: BondInputs = { ...bondSpec.defaults, redemption: 0 };
    expect((bondSpec.validate?.(badRedemption, CTX) ?? []).length).toBeGreaterThanOrEqual(1);
    const badMaturity: BondInputs = {
      ...bondSpec.defaults,
      maturityDate: { year: 2020, month: 1, day: 1 },
    };
    expect((bondSpec.validate?.(badMaturity, CTX) ?? []).length).toBeGreaterThanOrEqual(1);
  });

  it("prices its default to a dirty PV + yield through the shared offline engine", () => {
    const res = priceRatesInstrumentOffline(
      DEFAULT_USD_SOFR_CURVE,
      bondSpec.toRatesInstrument(bondSpec.defaults, CTX),
    );
    expect(res.pv).toBeGreaterThan(0); // long dirty PV
    expect(res.parRate).toBeGreaterThan(0); // yield to maturity
    expect(res.pv01).toBe(res.dv01); // bond PV01 ≡ yield DV01
    expect(res.keyRateLadder).toEqual([]); // no curve-space ladder on the wire
  });
});
