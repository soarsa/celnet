/**
 * hedgeLifecycle — the five-stage ribbon: exposure → decided → sent → filled →
 * residual, and the drop-off between the stages that no single screen showed.
 *
 * The case that matters most here is the one the board used to hide: orders sent,
 * every one refused, and the desk still reading "hedges firing".
 */

import { describe, expect, it } from "vitest";

import type { HedgeProvenance, StreetOrder, StreetOutcome } from "../src/data/contract";
import type { HedgeBucket } from "../src/lib/hedgeBuckets";
import { hedgeLifecycle, streetIsStalled, streetTally } from "../src/lib/hedgeLifecycle";

/** A drawable bucket. Only `netRisk` feeds the exposure stage. */
function bucket(book: string, netRisk: number): HedgeBucket {
  return {
    book,
    band: "red",
    utilization: 0.9,
    netRisk,
    threshold: 5_000,
    fill: 0.9,
    overflow: 0,
    needsHedge: false,
  };
}

/** A fired hedge. The three disposition fields are the risk-metric stages. */
function fire(
  hedgeId: string,
  crossed: number,
  external: number,
  residual: number,
  advisory = false,
): HedgeProvenance {
  return {
    hedgeId,
    book: "rates-usd",
    instrument: "ZTU26",
    internalCrossed: crossed,
    externalHedged: external,
    residual,
    advisory,
    band: "red",
    utilization: 0.9,
  } as HedgeProvenance;
}

/** A street order. `parentHedgeId` is the breach → street-order walk. */
function order(
  orderId: string,
  parentHedgeId: string | undefined,
  outcome: StreetOutcome,
  filledQty: number,
  reason?: string,
): StreetOrder {
  return {
    orderId,
    parentHedgeId,
    outcome,
    filledQty,
    requestedQty: 1_800_000,
    reason,
    instrument: "ZTU26",
  } as StreetOrder;
}

describe("streetTally — what happened on the street, in COUNTS", () => {
  it("separates clean fills, partials and the ones that shed nothing", () => {
    const t = streetTally([
      order("o1", "h1", "filled", 1_800_000),
      order("o2", "h1", "partially_filled", 600_000),
      order("o3", "h1", "rejected", 0, "NOT_A_WHOLE_LOT"),
      order("o4", "h1", "no_liquidity", 0, "no_firm_lp_price"),
    ]);
    expect(t.sent).toBe(4);
    expect(t.filled).toBe(1);
    expect(t.partial).toBe(1);
    expect(t.unfilled).toBe(2);
  });

  it("ranks the unfilled reasons worst-first so the dominant failure leads", () => {
    const t = streetTally([
      order("o1", "h1", "rejected", 0, "NOT_A_WHOLE_LOT"),
      order("o2", "h1", "rejected", 0, "NOT_A_WHOLE_LOT"),
      order("o3", "h1", "rejected", 0, "NOT_A_WHOLE_LOT"),
      order("o4", "h1", "cancelled", 0, "IOC_DEPTH_EXHAUSTED"),
    ]);
    expect(t.reasons).toEqual([
      { reason: "NOT_A_WHOLE_LOT", count: 3 },
      { reason: "IOC_DEPTH_EXHAUSTED", count: 1 },
    ]);
  });

  it("names the OUTCOME when the venue gave no reason — never drops the row", () => {
    const t = streetTally([order("o1", "h1", "expired", 0)]);
    expect(t.unfilled).toBe(1);
    expect(t.reasons).toEqual([{ reason: "expired", count: 1 }]);
  });

  it("an outcome claiming a fill of ZERO quantity shed nothing", () => {
    // Trusting the label alone would count this as risk removed when none moved.
    const t = streetTally([order("o1", "h1", "filled", 0)]);
    expect(t.filled).toBe(0);
    expect(t.unfilled).toBe(1);
  });

  it("counts unattributable orders separately instead of silently dropping them", () => {
    const t = streetTally([
      order("o1", undefined, "filled", 1_000),
      order("o2", "", "filled", 1_000),
      order("o3", "h1", "filled", 1_000),
    ]);
    expect(t.sent).toBe(1);
    expect(t.unlinked).toBe(2);
  });

  it("narrows to the given fires, ignoring orders from other decisions", () => {
    const t = streetTally(
      [
        order("o1", "h1", "filled", 1_000),
        order("o2", "h2", "rejected", 0, "NOT_A_WHOLE_LOT"),
      ],
      new Set(["h1"]),
    );
    expect(t.sent).toBe(1);
    expect(t.unfilled).toBe(0);
  });
});

describe("hedgeLifecycle — the ribbon", () => {
  it("sums exposure as ABSOLUTE net risk, so a short book adds rather than cancels", () => {
    // Signed netting across books would report a hedged-looking zero for a desk that
    // is in fact long one book and short another — two exposures, not none.
    const lc = hedgeLifecycle([bucket("a", -3_000), bucket("b", 2_000)], [], []);
    expect(lc.exposure).toBe(5_000);
  });

  it("splits the decision into crossed / external / warehoused and totals them", () => {
    const lc = hedgeLifecycle([], [fire("h1", 308, 932, 120)], []);
    expect(lc.crossed).toBe(308);
    expect(lc.external).toBe(932);
    expect(lc.warehoused).toBe(120);
    expect(lc.decided).toBe(1_360);
    expect(lc.fires).toBe(1);
  });

  it("EXCLUDES advisory dry-runs from every risk stage", () => {
    const lc = hedgeLifecycle(
      [],
      [fire("h1", 100, 200, 0), fire("h2", 9_000, 9_000, 9_000, true)],
      [],
    );
    expect(lc.decided).toBe(300);
    expect(lc.fires).toBe(1);
  });

  it("does not let an advisory fire's street orders into the tally either", () => {
    const lc = hedgeLifecycle(
      [],
      [fire("live", 0, 500, 0), fire("dry", 0, 500, 0, true)],
      [order("o1", "live", "filled", 1_000), order("o2", "dry", "filled", 1_000)],
    );
    expect(lc.street.sent).toBe(1);
  });

  it("survives a non-finite figure instead of poisoning the whole ribbon with NaN", () => {
    const lc = hedgeLifecycle(
      [bucket("a", Number.NaN)],
      [fire("h1", Number.POSITIVE_INFINITY, 500, 0)],
      [],
    );
    expect(lc.exposure).toBe(0);
    expect(lc.crossed).toBe(0);
    expect(lc.external).toBe(500);
  });
});

describe("streetIsStalled — the signal the old board could not give", () => {
  it("is TRUE when every sent order bounced: hedges 'firing', nothing shed", () => {
    // The exact shape of the DV01-denominated futures bug: the engine fires, orders
    // go out, the venue refuses all of them, and the books never drain.
    const lc = hedgeLifecycle(
      [bucket("wash", 4_997)],
      [fire("h1", 0, 932, 0)],
      [
        order("o1", "h1", "rejected", 0, "NOT_A_WHOLE_LOT"),
        order("o2", "h1", "rejected", 0, "NOT_A_WHOLE_LOT"),
      ],
    );
    expect(streetIsStalled(lc.street)).toBe(true);
    expect(lc.street.reasons[0]).toEqual({ reason: "NOT_A_WHOLE_LOT", count: 2 });
  });

  it("is FALSE when a partial filled — degraded is not stalled", () => {
    const t = streetTally([
      order("o1", "h1", "partially_filled", 600_000),
      order("o2", "h1", "rejected", 0, "NOT_A_WHOLE_LOT"),
    ]);
    expect(streetIsStalled(t)).toBe(false);
  });

  it("is FALSE on a quiet desk that sent nothing — silence is not failure", () => {
    expect(streetIsStalled(streetTally([]))).toBe(false);
  });
});
