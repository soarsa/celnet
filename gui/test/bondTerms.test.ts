/**
 * bondTerms — the client-side join between an aggregated-book composite line and
 * the instrument reference-data registry, plus the bond-term formatting the
 * Aggregated Book price tile renders. Pure functions, so this is a direct seam:
 * the join resolution (id → ISIN → CUSIP, canonical-id-wins, non-bond skip,
 * graceful-null) and the display formatting (coupon %, maturity date, the term
 * rows incl. zero-coupon handling) are all exercised without a server.
 */

import { describe, expect, it } from "vitest";

import type {
  AggregatedInstrument,
  BondDef,
  InstrumentDef,
} from "../src/data/contract";
import {
  bondTermRows,
  formatCouponPct,
  formatMaturity,
  indexBondDefs,
  resolveBondDef,
} from "../src/lib/bondTerms";

/** A minimal fixed-coupon bond definition for the join tests. */
function bondDef(over: Partial<InstrumentDef & { bond: BondDef }> = {}): InstrumentDef {
  const bond: BondDef = {
    issuer: "US Treasury",
    couponRate: 4.125,
    couponType: "fixed",
    couponFrequency: "semi_annual",
    dayCount: "act_act",
    maturityDate: { year: 2035, month: 2, day: 15 },
    redemption: 100,
    calendars: ["united_states"],
    ...over.bond,
  };
  return {
    instrumentId: "91282CJM4",
    name: "US Treasury 4.125% 2035",
    description: "",
    currency: "USD",
    externalIds: [
      { scheme: "isin", value: "US91282CJM47" },
      { scheme: "cusip", value: "91282CJM4" },
    ],
    family: "bond",
    bond,
    ...over,
  } as InstrumentDef;
}

/** A composite line carrying only the identity a subscriber joins on. */
function line(over: Partial<AggregatedInstrument> = {}): AggregatedInstrument {
  return {
    instrumentId: "91282CJM4",
    displayName: "UST 5Y 4.125%",
    isin: "US91282CJM47",
    cusip: "91282CJM4",
    bestBid: 98.8,
    bestOffer: 98.9,
    bidSize: 1_000_000,
    offerSize: 1_000_000,
    confidence: 0.9,
    contributions: [],
    ...over,
  };
}

describe("indexBondDefs / resolveBondDef", () => {
  it("resolves a composite line by its canonical instrumentId", () => {
    const index = indexBondDefs([bondDef()]);
    const resolved = resolveBondDef(index, line());
    expect(resolved?.couponRate).toBe(4.125);
  });

  it("falls back to the ISIN when the instrumentId does not match", () => {
    const index = indexBondDefs([bondDef({ instrumentId: "internal-xyz" })]);
    const resolved = resolveBondDef(index, line({ instrumentId: "does-not-match" }));
    expect(resolved?.issuer).toBe("US Treasury");
  });

  it("falls back to the CUSIP when neither id nor ISIN match", () => {
    const index = indexBondDefs([
      bondDef({
        instrumentId: "internal-xyz",
        externalIds: [{ scheme: "cusip", value: "91282CJM4" }],
      }),
    ]);
    const resolved = resolveBondDef(index, line({ instrumentId: "x", isin: "" }));
    expect(resolved).not.toBeNull();
  });

  it("returns null for an unseeded instrument (graceful degradation)", () => {
    const index = indexBondDefs([bondDef()]);
    const resolved = resolveBondDef(
      index,
      line({ instrumentId: "other", isin: "US000", cusip: "000" }),
    );
    expect(resolved).toBeNull();
  });

  it("skips non-bond families entirely", () => {
    const nonBond: InstrumentDef = {
      instrumentId: "usd-sofr-ois-5y",
      name: "OIS 5Y",
      description: "",
      currency: "USD",
      externalIds: [],
      family: "ois",
      ois: {
        index: "SOFR",
        tenor: "5Y",
        fixedFrequency: "annual",
        fixedDayCount: "act_360",
        floatDayCount: "act_360",
        businessDayConvention: "modified_following",
        calendars: ["united_states"],
        spotLagDays: 2,
      },
    };
    const index = indexBondDefs([nonBond]);
    expect(index.size).toBe(0);
  });

  it("lets the canonical instrumentId win over an external alias collision", () => {
    const bondA = bondDef({
      instrumentId: "A",
      externalIds: [{ scheme: "isin", value: "SHARED" }],
    });
    const bondB = bondDef({
      instrumentId: "SHARED",
      bond: { ...(bondDef().bond as BondDef), couponRate: 9.99 },
      externalIds: [],
    });
    const index = indexBondDefs([bondA, bondB]);
    // "SHARED" must resolve to bond B (canonical id), not bond A's alias.
    expect(index.get("SHARED")?.couponRate).toBe(9.99);
  });
});

describe("formatCouponPct", () => {
  it("trims trailing zeros", () => {
    expect(formatCouponPct(4.25)).toBe("4.25%");
    expect(formatCouponPct(4.125)).toBe("4.125%");
    expect(formatCouponPct(4)).toBe("4%");
    expect(formatCouponPct(4.625)).toBe("4.625%");
  });

  it("degrades a non-finite rate to a dash", () => {
    expect(formatCouponPct(Number.NaN)).toBe("—");
  });
});

describe("formatMaturity", () => {
  it("renders a compact human date", () => {
    expect(formatMaturity({ year: 2035, month: 2, day: 15 })).toBe("15 Feb 2035");
    expect(formatMaturity({ year: 2026, month: 10, day: 5 })).toBe("05 Oct 2026");
  });
});

describe("bondTermRows", () => {
  it("emits issuer, coupon, frequency, day-count and maturity for a fixed coupon", () => {
    const rows = bondTermRows(bondDef().bond as BondDef);
    expect(rows.map((r) => r.label)).toEqual([
      "Issuer",
      "Coupon",
      "Frequency",
      "Day count",
      "Maturity",
    ]);
    expect(rows.find((r) => r.key === "coupon")?.value).toBe("4.125%");
    expect(rows.find((r) => r.key === "frequency")?.value).toBe("Semi-annual");
    expect(rows.find((r) => r.key === "dayCount")?.value).toBe("ACT/ACT");
    expect(rows.find((r) => r.key === "maturity")?.value).toBe("15 Feb 2035");
  });

  it("omits the frequency row and labels a zero-coupon bill", () => {
    const bill: BondDef = {
      issuer: "US Treasury",
      couponRate: 0,
      couponType: "zero",
      couponFrequency: "",
      dayCount: "act_360",
      maturityDate: { year: 2026, month: 10, day: 22 },
      redemption: 100,
      calendars: ["united_states"],
    };
    const rows = bondTermRows(bill);
    expect(rows.some((r) => r.key === "frequency")).toBe(false);
    expect(rows.find((r) => r.key === "coupon")?.value).toBe("Zero coupon");
    expect(rows.find((r) => r.key === "dayCount")?.value).toBe("ACT/360");
  });

  it("omits the issuer row when blank", () => {
    const noIssuer = { ...(bondDef().bond as BondDef), issuer: "" };
    const rows = bondTermRows(noIssuer);
    expect(rows.some((r) => r.key === "issuer")).toBe(false);
  });
});
