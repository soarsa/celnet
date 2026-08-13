/**
 * instrumentPicker — the option model behind the hedge-vehicle picker.
 *
 * These pin the three things that made the free-text field wrong, and that the picker
 * exists to fix:
 *
 *  (a) a futures contract's DV01 per contract is ALREADY on the wire, so it is derived
 *      rather than retyped from memory — while a cash bond's is NOT (it is a function of
 *      the live curve), so nothing is invented for one;
 *  (b) a hedge can name the PRODUCT (`ZF`), which resolves to the front month and keeps
 *      working across the quarterly roll, and that option is offered ABOVE the pinned
 *      delivery months because a pinned one is what silently breaks;
 *  (c) an instrument that can actually FILL outranks one that merely exists.
 */
import { describe, expect, it } from "vitest";

import type { InstrumentDef } from "../src/data/contract";
import {
  filterInstrumentGroups,
  groupInstrumentOptions,
  hedgeVehicleOptions,
  liquidIdentifierSet,
} from "../src/lib/instrumentPicker";

/** A seeded Treasury future, as `reference_data::treasury_future_defs` produces one. */
function future(
  id: string,
  symbol: string,
  deliveryYear: number,
  deliveryMonth: number,
  dv01: number,
): InstrumentDef {
  return {
    instrumentId: id,
    name: `US 5Y T-NOTE FUT ${deliveryMonth}-${deliveryYear}`,
    description: "",
    currency: "USD",
    externalIds: [{ scheme: "ticker", value: id }],
    family: "bond_future",
    bondFuture: {
      contractCode: id,
      contractSymbol: symbol,
      underlyingIssuer: "US Treasury",
      contractFaceValue: 100_000,
      tickSizePoints: 0.0078125,
      tickValue: 7.8125,
      notionalCouponRate: 0.06,
      deliverableMinMonths: 50,
      deliverableMaxMonths: 63,
      deliveryMonthStart: { year: deliveryYear, month: deliveryMonth, day: 1 },
      firstDeliveryDate: { year: deliveryYear, month: deliveryMonth, day: 1 },
      lastTradingDate: { year: deliveryYear, month: deliveryMonth, day: 30 },
      lastDeliveryDate: { year: deliveryYear, month: deliveryMonth, day: 30 },
      dv01PerContractAtNotionalYield: dv01,
      calendars: ["united_states"],
    },
  };
}

function bond(id: string, isin: string): InstrumentDef {
  return {
    instrumentId: id,
    name: "UST 10Y 4.375%",
    description: "",
    currency: "USD",
    externalIds: [{ scheme: "isin", value: isin }],
    family: "bond",
    bond: {
      issuer: "US Treasury",
      couponRate: 4.375,
      couponType: "fixed",
      couponFrequency: "semi_annual",
      dayCount: "act_act",
      maturityDate: { year: 2036, month: 5, day: 15 },
      redemption: 100,
      calendars: ["united_states"],
    },
  };
}

const DEFS: InstrumentDef[] = [
  future("ZFZ26", "ZF", 2026, 12, 47.2),
  future("ZFU26", "ZF", 2026, 9, 47.0),
  bond("US912810TM0", "US912810TM0"),
];

describe("hedgeVehicleOptions", () => {
  it("derives a future's DV01 per contract from its own terms, and never invents a bond's", () => {
    const opts = hedgeVehicleOptions({ defs: DEFS });

    const zfu = opts.find((o) => o.value === "ZFU26");
    expect(zfu?.dv01PerUnit).toBe(47.0);
    expect(zfu?.isFuture).toBe(true);
    expect(zfu?.unitLabel).toBe("contract");

    // A cash bond's DV01 depends on the LIVE CURVE, not on any published static term —
    // so the picker supplies none and the trader must. Guessing here would silently
    // mis-size every hedge routed at it.
    const cash = opts.find((o) => o.value === "US912810TM0");
    expect(cash?.dv01PerUnit).toBeNull();
    expect(cash?.isFuture).toBe(false);
  });

  it("offers the rolling PRODUCT, carrying the FRONT contract's terms", () => {
    const opts = hedgeVehicleOptions({ defs: DEFS });
    const product = opts.find((o) => o.value === "ZF");

    expect(
      product,
      "the ZF product must be offered, not only its delivery months",
    ).toBeDefined();
    expect(product?.isRollingProduct).toBe(true);
    // Sep-26 is the nearest delivery month, so the product's defaults are ITS defaults —
    // the contract the vehicle actually resolves to today.
    expect(product?.dv01PerUnit).toBe(47.0);
    expect(product?.sublabel).toContain("ZFU26");
    expect(product?.sublabel).toContain("auto-rolls");
  });

  it("ranks the rolling product ABOVE the pinned delivery months", () => {
    const opts = hedgeVehicleOptions({ defs: DEFS });
    const productAt = opts.findIndex((o) => o.isRollingProduct);
    const pinnedAt = opts.findIndex((o) => o.isFuture && !o.isRollingProduct);

    // The pinned option is the one that stops trading four times a year, so the option
    // that survives the roll is the one a trader should reach first.
    expect(productAt).toBeGreaterThanOrEqual(0);
    expect(productAt).toBeLessThan(pinnedAt);
  });

  it("marks what a live composite covers, matching on ISIN as well as the id", () => {
    const liquidIds = liquidIdentifierSet([
      { instrumentId: "ZFU26", isin: "", cusip: "" },
      { instrumentId: "", isin: "US912810TM0", cusip: "" },
    ]);
    const opts = hedgeVehicleOptions({ defs: DEFS, liquidIds });

    expect(opts.find((o) => o.value === "ZFU26")?.hasLiveLiquidity).toBe(true);
    expect(opts.find((o) => o.value === "US912810TM0")?.hasLiveLiquidity).toBe(
      true,
    );
    // Dec-26 is listed but nobody is quoting it — a hedge routed there backstops.
    expect(opts.find((o) => o.value === "ZFZ26")?.hasLiveLiquidity).toBe(false);
  });

  it("sorts a fillable instrument above one that merely exists, within its group", () => {
    const liquidIds = liquidIdentifierSet([
      { instrumentId: "ZFZ26", isin: "", cusip: "" },
    ]);
    const groups = groupInstrumentOptions(
      hedgeVehicleOptions({ defs: DEFS, liquidIds }),
    );
    const months = groups.find((g) => g.group.includes("delivery month"));

    // ZFZ26 sorts after ZFU26 alphabetically, but it is the one with live liquidity —
    // "can fill" outranks "exists".
    expect(months?.options.map((o) => o.value)).toEqual(["ZFZ26", "ZFU26"]);
  });

  it("searches across the code, the name and the product symbol", () => {
    const groups = groupInstrumentOptions(hedgeVehicleOptions({ defs: DEFS }));

    // Searching a CONTRACT CODE surfaces both the pinned contract and the rolling
    // product that currently resolves to it — a trader who knows they want "the ZFU26
    // hedge" is shown the vehicle that will still be right after the roll, not only the
    // one that expires.
    expect(
      filterInstrumentGroups(groups, "zfu").flatMap((g) =>
        g.options.map((o) => o.value),
      ),
    ).toEqual(["ZF", "ZFU26"]);
    // The product symbol alone still matches its own contracts.
    expect(
      filterInstrumentGroups(groups, "zfz").flatMap((g) =>
        g.options.map((o) => o.value),
      ),
    ).toEqual(["ZFZ26"]);
    // A query matching nothing yields no groups at all, so the popup can say so rather
    // than render empty headings.
    expect(filterInstrumentGroups(groups, "gilt")).toEqual([]);
    // An empty query is not a filter.
    expect(filterInstrumentGroups(groups, "  ").length).toBe(groups.length);
  });
});
