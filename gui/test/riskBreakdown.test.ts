/**
 * riskBreakdown — the pure tenor/instrument fold behind the Risk Dashboard
 * drill-down. Covers: tenor bucketing at the boundaries; grouping deals by tenor
 * bucket + by instrument with summed gross notional and fill counts; the null-aware
 * DV01 sum (stays "—"/null with no source, sums when a source carries one); and the
 * short→long ordering.
 */
import { describe, expect, it } from "vitest";

import {
  dealToBreakdownItem,
  groupByInstrument,
  groupByTenor,
  riskBreakdownFor,
  tenorBucketLabel,
  type BreakdownItem,
} from "../src/data/riskBreakdown";
import type { Deal } from "../src/data/contract";

function deal(tenorYears: number, notional: number, riskBookId = "bk-1"): Deal {
  return {
    dealId: `d-${tenorYears}-${notional}`,
    requestId: "r",
    kind: "RFQ",
    counterparty: "CP",
    desk: "rates",
    instrument: { tenorYears, fixedRate: 0.04, notional, direction: "RECEIVE_FIXED" },
    curveSet: {
      currency: "USD",
      referenceDate: { year: 2026, month: 1, day: 1 },
      pillars: [],
    },
    side: "BUY",
    notional,
    price: 0.04,
    executedAtNanos: 0n,
    trader: "t",
    riskBookId,
  };
}

describe("tenorBucketLabel", () => {
  it("buckets whole-year tenors at the inclusive upper boundary", () => {
    expect(tenorBucketLabel(1)).toBe("≤ 2y");
    expect(tenorBucketLabel(2)).toBe("≤ 2y");
    expect(tenorBucketLabel(3)).toBe("2–5y");
    expect(tenorBucketLabel(5)).toBe("2–5y");
    expect(tenorBucketLabel(7)).toBe("5–10y");
    expect(tenorBucketLabel(10)).toBe("5–10y");
    expect(tenorBucketLabel(15)).toBe("10–20y");
    expect(tenorBucketLabel(20)).toBe("10–20y");
    expect(tenorBucketLabel(30)).toBe("> 20y");
  });
});

describe("dealToBreakdownItem", () => {
  it("maps a routed deal to a normalized item (gross notional, OIS label, null DV01)", () => {
    const item = dealToBreakdownItem(deal(10, -50_000_000));
    expect(item).toEqual({
      tenorYears: 10,
      instrument: "10y OIS",
      notional: 50_000_000, // gross (absolute)
      dv01: null, // never fabricated — the Deal wire carries no DV01
    });
  });
});

describe("groupByTenor", () => {
  it("sums gross notional + fill count per tenor bucket, ordered short→long", () => {
    const items: BreakdownItem[] = [
      { tenorYears: 2, instrument: "2y OIS", notional: 10, dv01: null },
      { tenorYears: 1, instrument: "1y OIS", notional: 5, dv01: null },
      { tenorYears: 7, instrument: "7y OIS", notional: 30, dv01: null },
      { tenorYears: 8, instrument: "8y OIS", notional: 20, dv01: null },
    ];
    const rows = groupByTenor(items);
    expect(rows).toEqual([
      { key: "≤ 2y", notional: 15, count: 2, dv01: null },
      { key: "5–10y", notional: 50, count: 2, dv01: null },
    ]);
  });

  it("sums DV01 within a bucket when the source carries one (null-aware)", () => {
    const items: BreakdownItem[] = [
      { tenorYears: 7, instrument: "7y OIS", notional: 30, dv01: 1200 },
      { tenorYears: 8, instrument: "8y OIS", notional: 20, dv01: 800 },
    ];
    const rows = groupByTenor(items);
    expect(rows).toEqual([{ key: "5–10y", notional: 50, count: 2, dv01: 2000 }]);
  });

  it("keeps DV01 null for a bucket where no contribution carries one", () => {
    const items: BreakdownItem[] = [
      { tenorYears: 7, instrument: "7y OIS", notional: 30, dv01: null },
      { tenorYears: 8, instrument: "8y OIS", notional: 20, dv01: 500 },
    ];
    // Mixed: one null + one real ⇒ the real one sums (null contributes nothing).
    expect(groupByTenor(items)).toEqual([{ key: "5–10y", notional: 50, count: 2, dv01: 500 }]);
  });
});

describe("groupByInstrument", () => {
  it("groups distinct instruments and sums per instrument, ordered by tenor", () => {
    const items: BreakdownItem[] = [
      { tenorYears: 10, instrument: "10y OIS", notional: 40, dv01: null },
      { tenorYears: 5, instrument: "5y OIS", notional: 15, dv01: null },
      { tenorYears: 10, instrument: "10y OIS", notional: 60, dv01: null },
    ];
    expect(groupByInstrument(items)).toEqual([
      { key: "5y OIS", notional: 15, count: 1, dv01: null },
      { key: "10y OIS", notional: 100, count: 2, dv01: null },
    ]);
  });
});

describe("riskBreakdownFor", () => {
  it("folds routed deals into both lenses with totals", () => {
    const deals = [deal(3, 25_000_000), deal(10, 50_000_000), deal(10, 25_000_000)];
    const bd = riskBreakdownFor(deals);

    expect(bd.count).toBe(3);
    expect(bd.totalNotional).toBe(100_000_000);
    expect(bd.byTenor).toEqual([
      { key: "2–5y", notional: 25_000_000, count: 1, dv01: null },
      { key: "5–10y", notional: 75_000_000, count: 2, dv01: null },
    ]);
    expect(bd.byInstrument).toEqual([
      { key: "3y OIS", notional: 25_000_000, count: 1, dv01: null },
      { key: "10y OIS", notional: 75_000_000, count: 2, dv01: null },
    ]);
  });

  it("is empty for a portfolio with no routed deals", () => {
    const bd = riskBreakdownFor([]);
    expect(bd).toEqual({ byTenor: [], byInstrument: [], totalNotional: 0, count: 0 });
  });
});
