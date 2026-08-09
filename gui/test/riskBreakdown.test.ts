/**
 * riskBreakdown — the pure tenor/instrument fold behind the Risk Dashboard
 * drill-down. Covers: tenor bucketing at the boundaries; grouping deals by tenor
 * bucket + by instrument with summed gross notional and fill counts; the null-aware
 * DV01 sum (stays "—"/null with no source, sums when a source carries one); and the
 * short→long ordering.
 */
import { describe, expect, it } from "vitest";

import {
  bucketDealsByBook,
  dealToBreakdownItem,
  dealsForBook,
  groupByInstrument,
  groupByProduct,
  groupByTenor,
  normalizeBookKey,
  riskBreakdownFor,
  tenorBucketLabel,
  type BreakdownItem,
} from "../src/data/riskBreakdown";
import type { Deal, RatesProductKind } from "../src/data/contract";

function deal(
  tenorYears: number,
  notional: number,
  riskBookId = "bk-1",
  productKind: RatesProductKind = "OIS",
): Deal {
  return {
    dealId: `d-${tenorYears}-${notional}-${productKind}`,
    requestId: "r",
    kind: "RFQ",
    counterparty: "CP",
    desk: "rates",
    productKind,
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

/** A breakdown item literal (all fields) for the pure fold tests. */
function item(
  tenorYears: number,
  notional: number,
  dv01: number | null,
  productKind: RatesProductKind = "OIS",
): BreakdownItem {
  return { tenorYears, instrument: `${tenorYears}y ${productKind}`, productKind, notional, dv01 };
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
  it("maps a routed OIS deal to a normalized item (gross notional, OIS label, null DV01)", () => {
    expect(dealToBreakdownItem(deal(10, -50_000_000))).toEqual({
      tenorYears: 10,
      instrument: "10y OIS",
      productKind: "OIS",
      notional: 50_000_000, // gross (absolute)
      dv01: null, // never fabricated — the Deal wire carries no DV01
    });
  });

  it("labels a non-OIS deal by its decoded product family", () => {
    expect(dealToBreakdownItem(deal(10, 60_000_000, "bk-2", "BOND"))).toMatchObject({
      instrument: "10y BOND",
      productKind: "BOND",
    });
  });
});

describe("groupByTenor", () => {
  it("sums gross notional + fill count per tenor bucket, ordered short→long", () => {
    const items = [item(2, 10, null), item(1, 5, null), item(7, 30, null), item(8, 20, null)];
    expect(groupByTenor(items)).toEqual([
      { key: "≤ 2y", notional: 15, count: 2, dv01: null },
      { key: "5–10y", notional: 50, count: 2, dv01: null },
    ]);
  });

  it("sums DV01 within a bucket when the source carries one (null-aware)", () => {
    const items = [item(7, 30, 1200), item(8, 20, 800)];
    expect(groupByTenor(items)).toEqual([{ key: "5–10y", notional: 50, count: 2, dv01: 2000 }]);
  });

  it("keeps DV01 null for a bucket where no contribution carries one", () => {
    const items = [item(7, 30, null), item(8, 20, 500)];
    // Mixed: one null + one real ⇒ the real one sums (null contributes nothing).
    expect(groupByTenor(items)).toEqual([{ key: "5–10y", notional: 50, count: 2, dv01: 500 }]);
  });
});

describe("groupByInstrument", () => {
  it("groups distinct instruments and sums per instrument, ordered by tenor", () => {
    const items = [item(10, 40, null), item(5, 15, null), item(10, 60, null)];
    expect(groupByInstrument(items)).toEqual([
      { key: "5y OIS", notional: 15, count: 1, dv01: null },
      { key: "10y OIS", notional: 100, count: 2, dv01: null },
    ]);
  });
});

describe("groupByProduct", () => {
  it("groups by product family, ordered by gross notional descending", () => {
    const items = [
      item(5, 30, null, "OIS"),
      item(10, 60, null, "BOND"),
      item(2, 20, null, "IRS"),
      item(10, 40, null, "OIS"),
    ];
    // OIS = 70 (largest), BOND = 60, IRS = 20 — gross-desc ordered.
    expect(groupByProduct(items)).toEqual([
      { key: "OIS", notional: 70, count: 2, dv01: null },
      { key: "BOND", notional: 60, count: 1, dv01: null },
      { key: "IRS", notional: 20, count: 1, dv01: null },
    ]);
  });

  it("collapses a single-family book to one honest row", () => {
    const rows = groupByProduct([item(5, 30, null), item(10, 40, null)]);
    expect(rows).toEqual([{ key: "OIS", notional: 70, count: 2, dv01: null }]);
  });
});

describe("normalizeBookKey", () => {
  it("mirrors the server slugify: lowercase, non-alphanumeric runs → single dash, trimmed", () => {
    expect(normalizeBookKey("wash-book")).toBe("wash-book");
    expect(normalizeBookKey("WASH_BOOK")).toBe("wash-book");
    expect(normalizeBookKey("Wash Book")).toBe("wash-book");
    expect(normalizeBookKey("  Default Book  ")).toBe("default-book");
    expect(normalizeBookKey("FI::Rates//EMEA")).toBe("fi-rates-emea");
  });

  it("is idempotent on an already-minted slug (slug of a slug is itself)", () => {
    for (const slug of ["warehouse", "fi-marex", "fi-rates-emea", "book-2"]) {
      expect(normalizeBookKey(slug)).toBe(slug);
    }
  });

  it("collapses a pathological all-separator id to the empty key", () => {
    expect(normalizeBookKey("---")).toBe("");
    expect(normalizeBookKey("   ")).toBe("");
  });
});

describe("bucketDealsByBook / dealsForBook — the deal→portfolio join", () => {
  it("buckets routed deals under their normalized book key", () => {
    const byBook = bucketDealsByBook([
      deal(5, 30_000_000, "wash-book"),
      deal(10, 50_000_000, "wash-book"),
      deal(2, 20_000_000, "default-book"),
    ]);
    expect(byBook.get("wash-book")).toHaveLength(2);
    expect(byBook.get("default-book")).toHaveLength(1);
  });

  it("RECONCILES a case/format-drifted roster id to slug-stamped deals (the live failure mode)", () => {
    // Deals stamped with the server slug `wash-book`; the roster row id arrives in a
    // divergent form. The normalized join must still resolve — this is exactly what
    // made the breakdown blank on live before the fix.
    const byBook = bucketDealsByBook([
      deal(5, 30_000_000, "wash-book"),
      deal(10, 50_000_000, "wash-book"),
    ]);
    expect(dealsForBook(byBook, "WASH_BOOK")).toHaveLength(2);
    expect(dealsForBook(byBook, "Wash Book")).toHaveLength(2);
    // And the breakdown folded from the reconciled deals sums to the book total.
    expect(riskBreakdownFor(dealsForBook(byBook, "WASH_BOOK")).totalNotional).toBe(80_000_000);
  });

  it("skips unrouted fills (no riskBookId) and returns [] for a book with no mapped deals", () => {
    const unrouted = deal(5, 10_000_000, "wash-book");
    delete unrouted.riskBookId;
    const byBook = bucketDealsByBook([unrouted, deal(10, 50_000_000, "wash-book")]);
    expect(byBook.get("wash-book")).toHaveLength(1); // only the routed one
    expect(dealsForBook(byBook, "empty-book")).toEqual([]);
  });
});

describe("riskBreakdownFor", () => {
  it("folds routed deals into all three lenses with totals", () => {
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
    expect(bd.byProduct).toEqual([{ key: "OIS", notional: 100_000_000, count: 3, dv01: null }]);
  });

  it("RECONCILES: every lens sums to the same book total (mixed families)", () => {
    const deals = [
      deal(5, 30_000_000, "bk-1", "OIS"),
      deal(10, 60_000_000, "bk-1", "BOND"),
      deal(2, 20_000_000, "bk-1", "IRS"),
    ];
    const bd = riskBreakdownFor(deals);
    const sum = (rows: { notional: number }[]) => rows.reduce((a, r) => a + r.notional, 0);

    expect(bd.totalNotional).toBe(110_000_000);
    // The invariant: product-type gross reconciles to the book total, same as tenor.
    expect(sum(bd.byProduct)).toBe(bd.totalNotional);
    expect(sum(bd.byTenor)).toBe(bd.totalNotional);
    expect(sum(bd.byInstrument)).toBe(bd.totalNotional);
    // Three product buckets, gross-desc: BOND 60 > OIS 30 > IRS 20.
    expect(bd.byProduct.map((r) => r.key)).toEqual(["BOND", "OIS", "IRS"]);
  });

  it("is empty for a portfolio with no routed deals", () => {
    const bd = riskBreakdownFor([]);
    expect(bd).toEqual({
      byTenor: [],
      byInstrument: [],
      byProduct: [],
      totalNotional: 0,
      count: 0,
    });
  });
});
